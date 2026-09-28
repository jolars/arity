"""Process ownership and transports for the opt-in LSP benchmark."""

import os
import queue
import signal
import socket
import subprocess
import threading
import time
import uuid
from pathlib import Path


def ark_dependencies():
    # Keep the two-server comparison and its CI tests dependency-free.
    try:
        from jupyter_client import BlockingKernelClient
        from jupyter_client.connect import write_connection_file
    except ImportError as error:
        raise RuntimeError(
            "Ark benchmarks require jupyter_client (including pyzmq); "
            "install it in the benchmark's Python environment or use --no-ark"
        ) from error
    return BlockingKernelClient, write_connection_file


def r_runtime(rscript):
    """Resolve the same R installation and libraries as the selected Rscript."""
    lines = subprocess.check_output(
        [
            rscript,
            "--vanilla",
            "-e",
            'cat(R.home(), R.version.string, .libPaths(), sep="\\n")',
        ],
        text=True,
        timeout=30,
    ).splitlines()
    if len(lines) < 3:
        raise RuntimeError("Rscript did not report its R home, version, and libraries")
    home, version, *libraries = lines
    return {
        "version": version,
        "home": home,
        "library_paths": libraries,
        "env": {
            "R_HOME": home,
            "R_LIBS_SITE": os.pathsep.join(libraries),
            "R_LIBS": "",
            "R_LIBS_USER": "",
        },
    }


class TcpReader:
    def __init__(self, sock):
        self.sock = sock
        self.reader = sock.makefile("rb")

    def _ack(self, value):
        # Ark can send the header before the body. A delayed ACK for that header
        # stalls Nagle's algorithm on the server, inflating every small response.
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_QUICKACK, 1)
        return value

    def readline(self):
        return self._ack(self.reader.readline())

    def read(self, length):
        return self._ack(self.reader.read(length))

    def close(self):
        self.reader.close()


class StdioServer:
    transport = "stdio"

    def __init__(self, command, cwd, env, stderr_path, *, stdio=True):
        self.closed = False
        self.stderr_file = open(stderr_path, "wb")  # noqa: SIM115
        try:
            self.proc = subprocess.Popen(
                command,
                cwd=cwd,
                env=env,
                stdin=subprocess.PIPE if stdio else subprocess.DEVNULL,
                stdout=subprocess.PIPE if stdio else self.stderr_file,
                stderr=self.stderr_file,
                start_new_session=True,
            )
        except BaseException:
            self.stderr_file.close()
            raise

    def connect(self, timeout):
        return self.proc.stdin, self.proc.stdout

    def check(self):
        pass

    def shutdown(self):
        try:
            self.proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass

    def kill(self):
        if self.closed:
            return
        self.closed = True
        # Workers may still hold pipes open after the session leader exits.
        try:
            os.killpg(self.proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.proc.wait(timeout=5)
        for stream in (self.proc.stdin, self.proc.stdout):
            if stream is not None:
                stream.close()
        self.stderr_file.close()


def ark_port(message, comm_id):
    """Read Ark's legacy or newer server-start envelope for our comm only."""
    content = message.get("content") or {}
    if content.get("comm_id") != comm_id:
        return None
    if message.get("msg_type") == "comm_close":
        raise RuntimeError("Ark closed the LSP comm before starting the server")
    if message.get("msg_type") != "comm_msg":
        return None
    data = content.get("data") or {}
    if data.get("msg_type", data.get("method")) != "server_started":
        return None
    params = data.get("content", data.get("params")) or {}
    port = params.get("port")
    if type(port) is not int or not 0 < port < 65536:
        raise RuntimeError(f"Ark returned an invalid LSP port: {port!r}")
    return port


class ArkServer(StdioServer):
    transport = "tcp (TCP_NODELAY, TCP_QUICKACK)"

    def __init__(self, command, cwd, env, stderr_path, state_dir):
        kernel_client, write_connection = ark_dependencies()
        connection, _ = write_connection(
            str(Path(state_dir) / "connection.json"),
            ip="127.0.0.1",
            key=uuid.uuid4().hex.encode(),
        )
        self.kernel = kernel_client(connection_file=connection)
        self.kernel.load_connection_file()
        self.sock = None
        self.streams = ()
        self.drain_stop = threading.Event()
        self.drain_thread = None
        self.drain_error = None
        command = [arg.replace("{connection_file}", connection) for arg in command]
        super().__init__(command, cwd, env, stderr_path, stdio=False)

    def connect(self, timeout):
        deadline = time.monotonic() + timeout

        def remaining():
            if self.proc.poll() is not None:
                raise RuntimeError(
                    f"Ark exited during startup (rc={self.proc.returncode})"
                )
            seconds = deadline - time.monotonic()
            if seconds <= 0:
                raise RuntimeError("Ark kernel/LSP startup timed out")
            return seconds

        self.kernel.start_channels()
        self.kernel.wait_for_ready(timeout=remaining())
        comm_id = uuid.uuid4().hex
        self.kernel.session.send(
            self.kernel.shell_channel.socket,
            "comm_open",
            {
                "comm_id": comm_id,
                "target_name": "positron.lsp",
                "data": {"ip_address": "127.0.0.1"},
            },
        )
        while True:
            try:
                message = self.kernel.get_iopub_msg(timeout=min(remaining(), 0.5))
            except queue.Empty:
                continue
            port = ark_port(message, comm_id)
            if port is not None:
                break

        self.sock = socket.create_connection(("127.0.0.1", port), timeout=remaining())
        self.sock.settimeout(None)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.streams = (self.sock.makefile("wb"), TcpReader(self.sock))
        self.drain_thread = threading.Thread(target=self._drain, daemon=True)
        self.drain_thread.start()
        return self.streams

    def _drain(self):
        # An idle kernel can still publish R output. Consume it so the Jupyter
        # channel cannot accumulate messages or exert backpressure on the kernel.
        try:
            while not self.drain_stop.is_set():
                try:
                    self.kernel.get_iopub_msg(timeout=0.2)
                except queue.Empty:
                    pass
        except Exception as error:  # noqa: BLE001 - Report failures from the reader thread.
            if not self.drain_stop.is_set():
                self.drain_error = error

    def check(self):
        if self.drain_error is not None:
            raise RuntimeError(
                "Ark's Jupyter output channel failed"
            ) from self.drain_error

    def shutdown(self):
        self.kernel.shutdown(restart=False)
        super().shutdown()

    def kill(self):
        if self.closed:
            return
        self.drain_stop.set()
        try:
            super().kill()
        finally:
            if self.drain_thread is not None:
                self.drain_thread.join(timeout=2)
            self.kernel.stop_channels()
            if self.sock is not None:
                try:
                    self.sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                for stream in self.streams:
                    stream.close()
                self.sock.close()
