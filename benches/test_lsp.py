import importlib.util
import io
import json
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

MODULE_PATH = Path(__file__).with_name("lsp.py")


def load_harness():
    spec = importlib.util.spec_from_file_location("lsp", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load lsp.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ProcParsingTests(unittest.TestCase):
    def test_rss_parser_reads_kibibytes(self):
        harness = load_harness()
        status = "Name:\tlanguageserver\nVmRSS:\t  12345 kB\nThreads:\t8\n"
        self.assertEqual(harness.parse_rss_kb(status), 12345)

    def test_cpu_parser_handles_spaces_and_parentheses_in_comm(self):
        harness = load_harness()
        prefix = "812 (server worker (one)) "
        fields = ["S"] + ["0"] * 10 + ["17", "19"] + ["0"] * 20
        self.assertEqual(harness.parse_cpu_ticks(prefix + " ".join(fields)), 36)

    def test_quiet_window_excludes_earlier_phases_and_short_waits(self):
        harness = load_harness()
        sampler = harness.Sampler(1)
        sampler.samples = [(t, 1, 1, 0, 1) for t in (0.0, 2.5, 4.0)]
        self.assertIsNone(sampler.quiet_since(5.0))
        sampler.samples.append((5.0, 1, 1, 0, 1))
        self.assertEqual(sampler.quiet_since(5.0), 0.0)
        self.assertIsNone(sampler.quiet_since(5.0, not_before=1.0))

    def test_busy_window_is_not_ready(self):
        harness = load_harness()
        sampler = harness.Sampler(1)
        sampler.samples = [
            (t, 1, 1, int(t * harness.CLK_TCK), 1) for t in (0.0, 2.5, 5.0)
        ]
        self.assertIsNone(sampler.quiet_since(5.0))

    def test_exiting_worker_cannot_make_a_busy_window_look_quiet(self):
        harness = load_harness()
        sampler = harness.Sampler(1)
        sampler.samples = [(0, 1, 1, 100, 2), (2.5, 1, 1, 300, 2), (5, 1, 1, 50, 1)]
        self.assertIsNone(sampler.quiet_since(5.0))

    def test_worker_exit_between_status_and_pss_is_not_a_sampling_failure(self):
        harness = load_harness()
        fields = ["S"] + ["0"] * 10 + ["17", "19"] + ["0"] * 20
        with (
            patch.object(
                harness.Path,
                "read_text",
                side_effect=[
                    "VmRSS: 20 kB",
                    "1 (worker) " + " ".join(fields),
                    ProcessLookupError(3, "No such process"),
                ],
            ),
            patch.object(harness.Path, "exists", return_value=True),
        ):
            self.assertIsNone(harness.read_process(1))


class LatencyTests(unittest.TestCase):
    def edit_target(self):
        return {
            "uri": "file:///a.R",
            "names": ["arity_lsp_left", "arity_lsp_rite"],
            "use": {"line": 4, "character": 0},
            "use_offset": 0,
            "definitions": [
                {"line": 2, "character": 0},
                {"line": 3, "character": 0},
            ],
        }

    def test_edit_latency_includes_the_change_and_checks_the_new_definition(self):
        harness = load_harness()
        clock_ns = [0]
        target = self.edit_target()
        current = [0]

        def notify(method, params):
            clock_ns[0] += 2_000_000
            current[0] = target["names"].index(params["contentChanges"][0]["text"])

        def request(*args, **kwargs):
            clock_ns[0] += 5_000_000
            return {
                "result": [
                    {
                        "uri": target["uri"],
                        "range": {
                            "start": target["definitions"][current[0]],
                            "end": {"line": 3, "character": 13},
                        },
                    }
                ]
            }

        client = Mock()
        client.notify.side_effect = notify
        client.request.side_effect = request
        with patch.object(
            harness.time, "perf_counter_ns", side_effect=lambda: clock_ns[0]
        ):
            record = harness.churn_definitions(client, target, edits=2, timeout=30)
        self.assertEqual(record["median_ms"], 7.0)
        self.assertEqual(record["samples"], 2)

    def test_persistently_stale_definition_aborts_the_measurement(self):
        harness = load_harness()
        target = self.edit_target()
        client = Mock()
        client.request.return_value = {
            "result": [
                {
                    "uri": target["uri"],
                    "range": {
                        "start": target["definitions"][0],
                        "end": {"line": 2, "character": 13},
                    },
                }
            ]
        }
        with self.assertRaisesRegex(RuntimeError, "stale or incorrect"):
            harness.churn_definitions(client, target, edits=1, timeout=0.001)

    def test_full_sync_sends_the_updated_document_and_waits_out_stale_results(self):
        harness = load_harness()
        target = self.edit_target()
        client = Mock()
        client.request.side_effect = [
            {
                "result": {
                    "uri": target["uri"],
                    "range": {"start": target["definitions"][index]},
                }
            }
            for index in (0, 1)
        ]
        record = harness.churn_definitions(
            client, target, edits=1, timeout=1, text="arity_lsp_left(1L)\n", sync_kind=1
        )
        change = client.notify.call_args.args[1]["contentChanges"]
        self.assertEqual(change, [{"text": "arity_lsp_rite(1L)\n"}])
        self.assertEqual(record["stale_responses"], 1)
        self.assertEqual(client.request.call_count, 2)

    def test_warmups_are_excluded_and_results_are_counted(self):
        harness = load_harness()
        client = Mock()
        client.request.return_value = {
            "result": [{"name": "Chapter", "children": [{"name": "Section"}]}]
        }
        with patch.object(
            harness.time,
            "perf_counter_ns",
            side_effect=[0, 1_000_000, 2_000_000, 5_000_000],
        ):
            record = harness.benchmark_requests(
                client,
                "document_symbol",
                "Document symbols",
                "textDocument/documentSymbol",
                [{"textDocument": {"uri": "file:///a.R"}}],
                runs=2,
                warmups=1,
                timeout=30,
            )
        self.assertEqual(client.request.call_count, 3)
        self.assertEqual(record["samples"], 2)
        self.assertEqual(record["median_ms"], 2.0)
        self.assertEqual(record["p95_ms"], 3.0)
        self.assertEqual(record["result_count_min"], 2)
        self.assertEqual(record["empty_results"], 0)

    def test_timeouts_do_not_become_fast_samples(self):
        harness = load_harness()
        client = Mock()
        client.request.return_value = None
        with self.assertRaisesRegex(RuntimeError, "timed out"):
            harness.benchmark_requests(
                client,
                "hover",
                "Hover",
                "textDocument/hover",
                [{}],
                runs=1,
                warmups=0,
                timeout=30,
            )

    def test_empty_responses_are_visible_in_result_counts(self):
        harness = load_harness()
        client = Mock()
        client.request.return_value = {"result": None}
        record = harness.benchmark_requests(
            client,
            "hover",
            "Hover",
            "textDocument/hover",
            [{}],
            runs=2,
            warmups=1,
            timeout=30,
        )
        self.assertEqual(record["empty_results"], 2)
        self.assertEqual(record["result_count_min"], 0)
        self.assertEqual(record["samples"], 2)

    def test_protocol_errors_abort_measurement(self):
        harness = load_harness()
        client = Mock()
        client.request.return_value = {
            "error": {"code": -32601, "message": "Unknown method"}
        }
        with self.assertRaisesRegex(RuntimeError, "failed"):
            harness.benchmark_requests(
                client,
                "hover",
                "Hover",
                "textDocument/hover",
                [{}],
                runs=1,
                warmups=0,
                timeout=30,
            )

    def test_pooling_uses_all_samples_instead_of_run_percentiles(self):
        harness = load_harness()
        records = [
            {
                "key": "hover",
                "label": "Hover",
                "method": "textDocument/hover",
                "targets": 1,
                "empty_results": 0,
                "_latencies_ms": [1.0, 2.0, 3.0],
                "_result_counts": [1, 1, 1],
                "_result_files": [],
                "_payload_bytes": [10, 10, 10],
            },
            {
                "key": "hover",
                "label": "Hover",
                "method": "textDocument/hover",
                "targets": 1,
                "empty_results": 1,
                "_latencies_ms": [100.0],
                "_result_counts": [0],
                "_result_files": [],
                "_payload_bytes": [4],
            },
        ]
        pooled = harness.aggregate_latencies(
            [{"request_latencies": [record]} for record in records]
        )[0]
        self.assertEqual(pooled["samples"], 4)
        self.assertEqual(pooled["median_ms"], 2.5)
        self.assertEqual(pooled["p95_ms"], 100.0)
        self.assertEqual(pooled["empty_results"], 1)
        self.assertEqual(pooled["result_count_min"], 0)
        self.assertFalse(any(key.startswith("_") for key in pooled))

    def test_navigation_counts_work_across_files(self):
        harness = load_harness()
        self.assertEqual(
            harness.result_summary(
                "textDocument/definition",
                [
                    {"uri": "file:///a.R"},
                    {"targetUri": "file:///b.R"},
                ],
            ),
            (2, 2),
        )
        self.assertEqual(
            harness.result_summary(
                "textDocument/rename",
                {
                    "changes": {"file:///a.R": [{}, {}]},
                    "documentChanges": [
                        {"textDocument": {"uri": "file:///b.R"}, "edits": [{}]}
                    ],
                },
            ),
            (3, 2),
        )


class AggregationTests(unittest.TestCase):
    def test_aggregate_runs_uses_medians(self):
        harness = load_harness()
        runs = [
            {
                "milestones": {
                    "baseline": {"rss_mb": 10.0, "pss_mb": 9.0, "processes": 1},
                    "settled": {"rss_mb": 20.0, "pss_mb": 18.0, "processes": 1},
                    "edited": {"rss_mb": 21.0, "pss_mb": 19.0, "processes": 1},
                    "peak": {"rss_mb": 22.0, "pss_mb": 20.0, "processes": 1},
                },
                "init_seconds": 1.0,
                "baseline_seconds": 2.0,
                "settled_seconds": 3.0,
                "edit_seconds": 4.0,
                "total_seconds": 5.0,
                "samples": 50,
            },
            {
                "milestones": {
                    "baseline": {"rss_mb": 12.0, "pss_mb": 11.0, "processes": 1},
                    "settled": {"rss_mb": 24.0, "pss_mb": 22.0, "processes": 1},
                    "edited": {"rss_mb": 25.0, "pss_mb": 23.0, "processes": 1},
                    "peak": {"rss_mb": 27.0, "pss_mb": 25.0, "processes": 1},
                },
                "init_seconds": 1.2,
                "baseline_seconds": 2.2,
                "settled_seconds": 3.2,
                "edit_seconds": 4.2,
                "total_seconds": 5.2,
                "samples": 52,
            },
            {
                "milestones": {
                    "baseline": {"rss_mb": 11.0, "pss_mb": 10.0, "processes": 1},
                    "settled": {"rss_mb": 22.0, "pss_mb": 20.0, "processes": 1},
                    "edited": {"rss_mb": 23.0, "pss_mb": 21.0, "processes": 1},
                    "peak": {"rss_mb": 25.0, "pss_mb": 23.0, "processes": 1},
                },
                "init_seconds": 1.1,
                "baseline_seconds": 2.1,
                "settled_seconds": 3.1,
                "edit_seconds": 4.1,
                "total_seconds": 5.1,
                "samples": 51,
            },
        ]

        for run in runs:
            run.update(
                {
                    "workspace_ready_seconds": 0.123456,
                    "documents_ready_seconds": 0.234567,
                    "edit_work_seconds": 0.345678,
                    "request_latencies": [],
                }
            )
        aggregate = harness.aggregate_runs(runs)

        self.assertEqual(aggregate["milestones"]["settled"]["rss_mb"], 22.0)
        self.assertEqual(aggregate["milestones"]["edited"]["pss_mb"], 21.0)
        self.assertEqual(aggregate["timings"]["edit_seconds"], 4.1)
        self.assertEqual(aggregate["timings"]["documents_ready_seconds"], 0.234567)
        self.assertEqual(aggregate["samples"], 51)


class TransportTests(unittest.TestCase):
    def test_tcp_reader_acknowledges_fragmented_headers_and_reads_exact_body(self):
        from lsp_servers import TcpReader

        sock = Mock()
        sock.makefile.return_value = io.BytesIO(
            b'Content-Length: 15\r\n\r\n{"result":"\xce\xbb"}'
        )
        reader = TcpReader(sock)
        self.assertEqual(reader.readline(), b"Content-Length: 15\r\n")
        self.assertEqual(reader.readline(), b"\r\n")
        self.assertEqual(reader.read(15), '{"result":"λ"}'.encode())
        self.assertEqual(sock.setsockopt.call_count, 3)
        sock.setsockopt.assert_called_with(socket.IPPROTO_TCP, socket.TCP_QUICKACK, 1)
        reader.close()

    def test_client_handles_fragmented_tcp_responses_and_disconnect(self):
        harness = load_harness()
        from lsp_servers import TcpReader

        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        sock = socket.create_connection(listener.getsockname())
        peer, _ = listener.accept()
        self.addCleanup(listener.close)
        self.addCleanup(sock.close)
        self.addCleanup(peer.close)
        server = Mock()
        writer = sock.makefile("wb")
        reader = TcpReader(sock)
        self.addCleanup(writer.close)
        self.addCleanup(reader.close)
        server.connect.return_value = writer, reader
        client = harness.Client(server, timeout=1)
        # Split a UTF-8 code point across writes to exercise byte framing.
        payload = '{"id":1,"result":"λ"}'.encode()
        peer.sendall(b"Content-Length: %d\r\n\r\n" % len(payload) + payload[:-2])
        peer.sendall(payload[-2:])
        response = client.request("test", {}, timeout=2)
        self.assertEqual(response["result"], "λ")
        peer.shutdown(socket.SHUT_RDWR)
        client.reader.join(timeout=2)
        self.assertFalse(client.reader.is_alive())
        self.assertIsNone(client.request("test", {}, timeout=0))

    def test_stdio_spawn_failure_closes_log(self):
        from lsp_servers import StdioServer

        log = Mock()
        with (
            patch("lsp_servers.open", return_value=log),
            patch("lsp_servers.subprocess.Popen", side_effect=OSError("missing")),
            self.assertRaisesRegex(OSError, "missing"),
        ):
            StdioServer(["missing"], ".", {}, "log")
        log.close.assert_called_once()

    def test_stopping_stdio_unblocks_the_reader_and_closes_the_log(self):
        harness = load_harness()
        from lsp_servers import StdioServer

        with tempfile.TemporaryDirectory() as directory:
            server = StdioServer(
                [sys.executable, "-c", "import time; time.sleep(60)"],
                directory,
                None,
                str(Path(directory) / "stderr.log"),
            )
            client = harness.Client(server, timeout=1)
            try:
                self.assertIsNone(client.request("initialize", {}, timeout=0.01))
            finally:
                client.kill()
            self.assertIsNotNone(server.proc.poll())
            self.assertFalse(client.reader.is_alive())
            self.assertTrue(server.stderr_file.closed)
            client.kill()

    def test_ark_handshake_connects_only_to_its_announced_port(self):
        from lsp_servers import ArkServer

        kernel = Mock()
        kernel.get_iopub_msg.side_effect = [
            {"msg_type": "status"},
            {
                "msg_type": "comm_msg",
                "content": {
                    "comm_id": "other",
                    "data": {"msg_type": "server_started", "content": {"port": 2345}},
                },
            },
            {
                "msg_type": "comm_msg",
                "content": {
                    "comm_id": "mine",
                    "data": {"msg_type": "server_started", "content": {"port": 3456}},
                },
            },
        ]
        proc = Mock(stdin=None, stdout=None)
        proc.poll.return_value = None
        sock = Mock()
        with (
            tempfile.TemporaryDirectory() as directory,
            patch(
                "lsp_servers.ark_dependencies",
                return_value=(
                    Mock(return_value=kernel),
                    Mock(return_value=("/connection.json", {})),
                ),
            ),
            patch("lsp_servers.subprocess.Popen", return_value=proc) as spawn,
            patch("lsp_servers.socket.create_connection", return_value=sock) as connect,
            patch("lsp_servers.threading.Thread"),
            patch("lsp_servers.uuid.uuid4", return_value=Mock(hex="mine")),
            patch("lsp_servers.os.killpg"),
        ):
            server = ArkServer(
                ["ark", "--connection_file", "{connection_file}"],
                directory,
                {},
                str(Path(directory) / "log"),
                directory,
            )
            kernel.start_channels.assert_not_called()
            spawn.assert_called_once()
            self.assertEqual(
                spawn.call_args.args[0],
                ["ark", "--connection_file", "/connection.json"],
            )
            self.assertEqual(spawn.call_args.kwargs["stdin"], subprocess.DEVNULL)
            try:
                server.connect(timeout=1)
                self.assertEqual(connect.call_args.args[0], ("127.0.0.1", 3456))
                kernel.session.send.assert_called_once_with(
                    kernel.shell_channel.socket,
                    "comm_open",
                    {
                        "comm_id": "mine",
                        "target_name": "positron.lsp",
                        "data": {"ip_address": "127.0.0.1"},
                    },
                )
                sock.setsockopt.assert_called_with(
                    socket.IPPROTO_TCP, socket.TCP_NODELAY, 1
                )
            finally:
                server.kill()
            kernel.stop_channels.assert_called_once()

    def test_ark_startup_timeout_is_bounded(self):
        from lsp_servers import ArkServer

        server = ArkServer.__new__(ArkServer)
        server.proc = Mock()
        server.proc.poll.return_value = None
        server.kernel = Mock()
        with (
            patch("lsp_servers.time.monotonic", side_effect=[10, 11]),
            self.assertRaisesRegex(RuntimeError, "startup timed out"),
        ):
            server.connect(timeout=0.5)
        server.kernel.wait_for_ready.assert_not_called()

    def test_ark_accepts_both_startup_envelopes_and_ignores_other_comms(self):
        from lsp_servers import ark_port

        def message(data, comm_id="mine"):
            return {
                "msg_type": "comm_msg",
                "content": {"comm_id": comm_id, "data": data},
            }

        for envelope in [
            {"msg_type": "server_started", "content": {"port": 1234}},
            {"method": "server_started", "params": {"port": 1234}},
        ]:
            self.assertEqual(ark_port(message(envelope), "mine"), 1234)
            self.assertIsNone(ark_port(message(envelope, "other"), "mine"))
        self.assertIsNone(ark_port({"msg_type": "status"}, "mine"))
        for port in [None, True, 0, 65536, "1234"]:
            with self.assertRaisesRegex(RuntimeError, "port"):
                ark_port(
                    message({"msg_type": "server_started", "content": {"port": port}}),
                    "mine",
                )
        with self.assertRaisesRegex(RuntimeError, "closed"):
            ark_port({"msg_type": "comm_close", "content": {"comm_id": "mine"}}, "mine")

    def test_ark_cleanup_closes_channels_and_socket_even_after_kernel_exit(self):
        from lsp_servers import ArkServer

        server = ArkServer.__new__(ArkServer)
        server.closed = False
        server.streams = ()
        server.proc = Mock(pid=123, stdin=None, stdout=None)
        server.proc.poll.return_value = 0
        server.stderr_file = Mock()
        server.kernel = Mock()
        server.sock = Mock()
        server.drain_stop = threading.Event()
        server.drain_thread = Mock()
        with patch("lsp_servers.os.killpg") as killpg:
            server.kill()
        killpg.assert_called_once()
        server.kernel.stop_channels.assert_called_once()
        server.sock.shutdown.assert_called_once_with(socket.SHUT_RDWR)
        server.sock.close.assert_called_once()
        server.stderr_file.close.assert_called_once()
        self.assertTrue(server.drain_stop.is_set())

    def test_ark_startup_failure_is_cleaned_up_before_client_exists(self):
        harness = load_harness()
        server = Mock()
        server.connect.side_effect = RuntimeError("kernel timed out")
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(harness, "ArkServer", return_value=server),
            patch.object(harness, "Sampler") as sampler,
            self.assertRaisesRegex(RuntimeError, "kernel timed out"),
        ):
            harness.run_session(
                ("ark", ["ark"]),
                1,
                Path(directory),
                [],
                1,
                1,
                0.5,
                directory,
                1,
                0,
            )
        sampler.return_value.start.assert_called_once()
        server.kill.assert_called_once()

    def test_quiet_wait_rejects_a_disconnected_lsp_in_a_live_kernel(self):
        harness = load_harness()
        client = Mock(alive=False)
        client.proc.poll.return_value = None
        sampler = Mock(error=None, started_at=0)
        sampler.quiet_since.return_value = 1
        with self.assertRaisesRegex(RuntimeError, "disconnected"):
            harness.wait_until_quiet(client, sampler, 0.5, 1, "startup", 0)

    def test_ark_dependency_is_optional_and_error_is_actionable(self):
        from lsp_servers import ark_dependencies

        with (
            patch.dict(sys.modules, {"jupyter_client": None}),
            self.assertRaisesRegex(RuntimeError, "jupyter_client"),
        ):
            ark_dependencies()


class ReadinessTests(unittest.TestCase):
    def test_ark_checks_every_buffer_without_waiting_for_unchanged_diagnostics(self):
        harness = load_harness()
        client = Mock()
        client.request.return_value = {"result": []}
        uris = [f"file:///{n}.R" for n in range(5)]
        harness.await_documents(client, "ark", False, dict.fromkeys(uris, 1), 2)
        client.wait_diagnostics.assert_not_called()
        symbols = [
            args
            for args, _ in client.request.call_args_list
            if args[0] == "textDocument/documentSymbol"
        ]
        self.assertEqual([args[1]["textDocument"]["uri"] for args in symbols], uris)
        client.request.return_value = {"error": {"message": "failed"}}
        with self.assertRaisesRegex(RuntimeError, "failed"):
            harness.await_documents(client, "ark", False, dict.fromkeys(uris, 1), 2)

    def test_existing_servers_still_require_diagnostics_and_propagate_timeouts(self):
        harness = load_harness()
        client = Mock()
        client.wait_diagnostics.side_effect = RuntimeError("diagnostics timed out")
        with self.assertRaisesRegex(RuntimeError, "diagnostics timed out"):
            harness.await_documents(
                client, "languageserver", False, {"a": 2}, 2, after=5
            )
        client.wait_diagnostics.assert_called_once_with({"a": 2}, 2, after=5)
        client.request.assert_not_called()

    def test_ark_uses_rscript_resolved_library_paths(self):
        from lsp_servers import r_runtime

        with patch(
            "lsp_servers.subprocess.check_output",
            return_value="/r/home\nR version 4.6.1\n/lib/one\n/lib/two\n",
        ):
            runtime = r_runtime("/wrapped/Rscript")
        self.assertEqual(
            runtime["env"],
            {
                "R_HOME": "/r/home",
                "R_LIBS_SITE": "/lib/one:/lib/two",
                "R_LIBS": "",
                "R_LIBS_USER": "",
            },
        )
        self.assertEqual(runtime["library_paths"], ["/lib/one", "/lib/two"])


class OutputTests(unittest.TestCase):
    def test_required_result_rejects_an_empty_definition(self):
        harness = load_harness()

        with self.assertRaisesRegex(RuntimeError, "returned no result"):
            harness.require_response(
                {"jsonrpc": "2.0", "id": 1, "result": []},
                "textDocument/definition",
                require_result=True,
            )

    def test_artifact_update_preserves_cli_measurements(self):
        harness = load_harness()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "results.json"
            original = {"schema_version": 2, "meta": {"date": "old"}, "sections": [1]}
            path.write_text(json.dumps(original))
            harness.write_artifact(path, {"servers": ["new"]})
            self.assertEqual(
                json.loads(path.read_text()), {**original, "lsp": {"servers": ["new"]}}
            )
            path.write_text("invalid JSON")
            with self.assertRaises(json.JSONDecodeError):
                harness.write_artifact(path, {})
            self.assertEqual(path.read_text(), "invalid JSON")

    def test_utf16_positions_and_full_sync_offsets_preserve_original_bytes(self):
        harness = load_harness()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "unicode.R"
            original = '# 😀\r\nx <- "λ"\r\n'.encode()
            path.write_bytes(original)
            documents, target = harness.prepare_documents([path])
            text = documents[0]["text"]
            self.assertTrue(text.encode().startswith(original))
            self.assertEqual(
                text[target["use_offset"] :].splitlines()[0], "arity_lsp_left(1L)"
            )
            self.assertEqual(
                harness.position_for_offset("😀x", 1), {"line": 0, "character": 2}
            )
            self.assertEqual(path.read_bytes(), original)

    def test_diagnostics_must_cover_every_uri_and_latest_version(self):
        harness = load_harness()
        client = harness.Client.__new__(harness.Client)
        client.state = threading.Condition()
        client.alive = True

        def diagnostic(uri, version):
            return {
                "method": "textDocument/publishDiagnostics",
                "params": {"uri": uri, "version": version},
            }

        client.notifications = [diagnostic("a", 1), diagnostic("b", None)]
        with self.assertRaisesRegex(RuntimeError, "diagnostics timed out"):
            client.wait_diagnostics({"a": 2, "b": 1}, timeout=0)
        client.notifications.append(diagnostic("a", 2))
        client.wait_diagnostics({"a": 2, "b": 1}, timeout=0)


if __name__ == "__main__":
    unittest.main()
