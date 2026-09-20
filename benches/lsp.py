#!/usr/bin/env python3
"""Measure latency, runtime, and resident memory in an R LSP session.

The harness launches each server over stdio, samples its complete process tree
from ``/proc``, and records four milestones: initialized baseline, files-opened
settled state, post-edit settled state, and the sampled peak. It intentionally
uses no third-party Python packages so an Arity development shell can run it
without additional setup. Adapted from Panache's ``benches/lsp_memory.py``.
"""

import argparse
import itertools
import json
import math
import os
import platform
import shutil
import signal
import statistics
import subprocess
import tempfile
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

CLK_TCK = os.sysconf("SC_CLK_TCK")
IDLE_CPU_FRACTION = 0.05
MILESTONES = ("baseline", "settled", "edited", "peak")
SCHEMA_VERSION = 1
SAMPLE_INTERVAL_SECONDS = 0.15
SERVER_META = {
    "arity": ("Arity", "static R analysis and native package indexing"),
    "languageserver": ("languageserver", "R-backed analysis and lintr diagnostics"),
}
ROOT = Path(__file__).resolve().parents[1]
CORPUS_REPO = "https://github.com/tidyverse/tidyr.git"
CORPUS_REVISION = "bfdfc359b90b5f1432eb9de21cd03d9c11893137"  # v1.3.2.


# --- /proc sampling ---------------------------------------------------------


def parse_rss_kb(status):
    """Return VmRSS from one ``/proc/<pid>/status`` payload."""
    for line in status.splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    return 0


def parse_cpu_ticks(stat):
    """Return user + system ticks from one ``/proc/<pid>/stat`` payload."""
    fields = stat[stat.rindex(")") + 2 :].split()
    return int(fields[11]) + int(fields[12])


def process_tree(root_pid):
    """Return every live PID in the process tree rooted at ``root_pid``."""
    pids = {root_pid}
    frontier = [root_pid]
    while frontier:
        pid = frontier.pop()
        try:
            tasks = list(Path(f"/proc/{pid}/task").iterdir())
        except OSError:
            continue
        for task in tasks:
            try:
                children = (task / "children").read_text().split()
            except OSError:
                continue
            for child in map(int, children):
                if child not in pids:
                    pids.add(child)
                    frontier.append(child)
    return pids


def read_process(pid):
    """Return RSS, PSS, and CPU ticks for one process, or ``None`` if gone."""
    try:
        status = Path(f"/proc/{pid}/status").read_text()
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return None

    rss = parse_rss_kb(status)
    try:
        pss = 0
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
            if line.startswith("Pss:"):
                pss = int(line.split()[1])
                break
    except (FileNotFoundError, ProcessLookupError):
        return None

    return rss, pss, parse_cpu_ticks(stat)


def sample_tree(root_pid):
    """Sum RSS, PSS, CPU ticks, and process count over a live process tree."""
    rss = pss = cpu = count = 0
    for pid in process_tree(root_pid):
        reading = read_process(pid)
        if reading is None:
            continue
        rss += reading[0]
        pss += reading[1]
        cpu += reading[2]
        count += 1
    return rss, pss, cpu, count


class Sampler(threading.Thread):
    def __init__(self, pid, interval=SAMPLE_INTERVAL_SECONDS):
        super().__init__(daemon=True)
        self.pid = pid
        self.interval = interval
        self.stop_flag = threading.Event()
        self.samples = []
        self.peak_rss = 0
        self.peak_pss = 0
        self.peak_processes = 0
        self.started_at = time.monotonic()
        self.error = None

    def run(self):
        try:
            while not self.stop_flag.is_set():
                rss, pss, cpu, count = sample_tree(self.pid)
                if count:
                    elapsed = time.monotonic() - self.started_at
                    self.samples.append((elapsed, rss, pss, cpu, count))
                    self.peak_rss = max(self.peak_rss, rss)
                    self.peak_pss = max(self.peak_pss, pss)
                    self.peak_processes = max(self.peak_processes, count)
                self.stop_flag.wait(self.interval)
        except (OSError, ValueError) as error:
            self.error = error

    def milestone(self):
        if self.error is not None:
            raise RuntimeError("process sampling failed") from self.error
        rss, pss, _, count = sample_tree(self.pid)
        if not count:
            raise RuntimeError("server process tree exited before a memory milestone")
        self.peak_rss = max(self.peak_rss, rss)
        self.peak_pss = max(self.peak_pss, pss)
        self.peak_processes = max(self.peak_processes, count)
        return {
            "rss_mb": round(rss / 1024, 1),
            "pss_mb": round(pss / 1024, 1),
            "processes": count,
        }

    def peak(self):
        return {
            "rss_mb": round(self.peak_rss / 1024, 1),
            "pss_mb": round(self.peak_pss / 1024, 1),
            "processes": self.peak_processes,
        }

    def quiet_since(self, seconds, not_before=0.0):
        """Return the quiet window's start, excluding samples from earlier phases."""
        if not self.samples:
            return None
        cutoff = max(self.samples[-1][0] - seconds, not_before)
        window = [sample for sample in self.samples if sample[0] >= cutoff]
        span = window[-1][0] - window[0][0] if len(window) >= 3 else 0.0
        # Permit sampling slop without shortening the requested quiet period.
        if span <= 0 or span < seconds - self.interval * 1.5:
            return None
        # Exited workers remove their lifetime ticks from the process-tree sum.
        # Restart the quiet window instead of interpreting that drop as idle CPU.
        if any(b[3] < a[3] or b[4] != a[4] for a, b in itertools.pairwise(window)):
            return None
        cpu_seconds = (window[-1][3] - window[0][3]) / CLK_TCK
        if max(0, cpu_seconds) / span >= IDLE_CPU_FRACTION:
            return None
        return window[0][0]


# --- minimal LSP client -----------------------------------------------------


class Client:
    def __init__(self, command, cwd, env, stderr_path):
        self.stderr_file = open(stderr_path, "wb")  # noqa: SIM115
        self.proc = subprocess.Popen(
            command,
            cwd=cwd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr_file,
            start_new_session=True,
        )
        if self.proc.stdin is None or self.proc.stdout is None:
            raise RuntimeError("failed to open language-server stdio")
        self.stdin = self.proc.stdin
        self.stdout = self.proc.stdout
        self.next_id = 1
        self.write_lock = threading.Lock()
        self.responses = {}
        self.notifications = []
        self.state = threading.Condition()
        self.alive = True
        self.reader = threading.Thread(target=self._read_loop, daemon=True)
        self.reader.start()

    def _read_loop(self):
        while True:
            length = None
            while True:
                line = self.stdout.readline()
                if not line:
                    with self.state:
                        self.alive = False
                        self.state.notify_all()
                    return
                line = line.strip()
                if not line:
                    break
                if line.lower().startswith(b"content-length:"):
                    length = int(line.split(b":", 1)[1])
            if length is None:
                continue
            try:
                message = json.loads(self.stdout.read(length))
            except (json.JSONDecodeError, UnicodeDecodeError):
                continue
            with self.state:
                if "id" in message and ("result" in message or "error" in message):
                    self.responses[message["id"]] = message
                elif "method" in message:
                    self.notifications.append(message)
                    if "id" in message:
                        self._answer(message)
                self.state.notify_all()

    def _answer(self, request):
        if request["method"] == "workspace/configuration":
            items = (request.get("params") or {}).get("items") or []
            result = [None] * max(1, len(items))
        else:
            result = None
        self._send({"jsonrpc": "2.0", "id": request["id"], "result": result})

    def _send(self, message):
        payload = json.dumps(message).encode()
        with self.write_lock:
            try:
                self.stdin.write(b"Content-Length: %d\r\n\r\n" % len(payload))
                self.stdin.write(payload)
                self.stdin.flush()
            except (BrokenPipeError, ValueError, OSError):
                pass

    def notify(self, method, params):
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def request(self, method, params, timeout):
        with self.state:
            request_id = self.next_id
            self.next_id += 1
        self._send(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        )
        deadline = time.monotonic() + timeout
        with self.state:
            while request_id not in self.responses:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not self.alive:
                    return None
                self.state.wait(min(1.0, remaining))
            return self.responses.pop(request_id)

    def count_published_diagnostics(self):
        with self.state:
            return sum(
                notification.get("method") == "textDocument/publishDiagnostics"
                for notification in self.notifications
            )

    def wait_diagnostics(self, versions, timeout, after=0):
        deadline = time.monotonic() + timeout
        with self.state:
            while True:
                pending = set(versions)
                for message in self.notifications[after:]:
                    if message.get("method") != "textDocument/publishDiagnostics":
                        continue
                    params = message.get("params", {})
                    uri = params.get("uri")
                    version = params.get("version")
                    if uri in pending and (version is None or version >= versions[uri]):
                        pending.remove(uri)
                if not pending:
                    return
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not self.alive:
                    raise RuntimeError(f"diagnostics timed out for {sorted(pending)}")
                self.state.wait(min(1.0, remaining))

    def shutdown(self):
        if self.proc.poll() is None:
            response = self.request("shutdown", None, timeout=15)
            if response is not None and "error" not in response:
                self.notify("exit", None)
                try:
                    self.proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
        self.kill()

    def kill(self):
        # The session leader may have exited while workers still hold its pipes open.
        try:
            os.killpg(self.proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.proc.wait(timeout=5)
        self.stdin.close()
        self.reader.join(timeout=5)
        self.stdout.close()
        self.stderr_file.close()


CAPABILITIES = {
    "general": {"positionEncodings": ["utf-16"]},
    "workspace": {
        "workspaceFolders": True,
        "configuration": True,
        "didChangeConfiguration": {"dynamicRegistration": True},
        "diagnostics": {"refreshSupport": True},
        "symbol": {"dynamicRegistration": True},
    },
    "textDocument": {
        "synchronization": {"dynamicRegistration": True, "didSave": True},
        "publishDiagnostics": {"relatedInformation": True, "versionSupport": True},
        "diagnostic": {"dynamicRegistration": True, "relatedDocumentSupport": True},
        "hover": {"contentFormat": ["markdown", "plaintext"]},
        "definition": {"dynamicRegistration": True},
        "documentSymbol": {"hierarchicalDocumentSymbolSupport": True},
    },
}


def require_response(response, method, require_result=False):
    if response is None:
        raise RuntimeError(f"{method} timed out or the server exited")
    if "error" in response:
        raise RuntimeError(f"{method} failed: {response['error']}")
    if require_result and not response.get("result"):
        raise RuntimeError(f"{method} returned no result")
    return response


def wait_until_quiet(client, sampler, quiet_seconds, timeout, phase, not_before):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if sampler.error is not None:
            raise RuntimeError("process sampling failed") from sampler.error
        if client.proc.poll() is not None:
            raise RuntimeError(
                f"server exited during {phase} (rc={client.proc.returncode})"
            )
        quiet_since = sampler.quiet_since(quiet_seconds, not_before)
        if quiet_since is not None:
            return sampler.started_at + quiet_since
        time.sleep(sampler.interval)
    raise RuntimeError(f"server did not become quiet during {phase} within {timeout}s")


def utf16_length(text):
    return len(text.encode("utf-16-le")) // 2


def position_for_offset(text, offset):
    line = text.count("\n", 0, offset)
    line_start = text.rfind("\n", 0, offset) + 1
    return {"line": line, "character": utf16_length(text[line_start:offset])}


def prepare_documents(files):
    documents = []
    target = None
    names = ["arity_lsp_left", "arity_lsp_rite"]
    for index, path in enumerate(files):
        # Preserve CRLF and reject invalid UTF-8 rather than measuring a repaired file.
        text = path.read_bytes().decode("utf-8")
        if index == 0:
            if any(name in text for name in names):
                raise ValueError("benchmark binding collides with a corpus symbol")
            text += "\n\n# LSP benchmark bindings (buffer only).\n"
            definitions = []
            for name in names:
                definitions.append(position_for_offset(text, len(text)))
                text += f"{name} <- function(value) value + 1L\n"
            use_offset = len(text)
            use = position_for_offset(text, use_offset)
            text += f"{names[0]}(1L)\n"
            hover = position_for_offset(text, len(text))
            text += "mean(1:3)\n"
            target = {
                "uri": path.as_uri(),
                "names": names,
                "hover": hover,
                "definitions": definitions,
                "use": use,
                "use_offset": use_offset,
            }
        documents.append({"path": str(path), "uri": path.as_uri(), "text": text})
    if target is None:
        raise ValueError("at least one document is required")
    return documents, target


def pull_diagnostics(client, uris, timeout):
    for uri in uris:
        require_response(
            client.request(
                "textDocument/diagnostic",
                {"textDocument": {"uri": uri}},
                timeout=timeout,
            ),
            "textDocument/diagnostic",
        )


def exercise_shared_requests(client, uris):
    for uri in uris[:3]:
        require_response(
            client.request(
                "textDocument/documentSymbol",
                {"textDocument": {"uri": uri}},
                timeout=60,
            ),
            "textDocument/documentSymbol",
        )
        require_response(
            client.request(
                "textDocument/hover",
                {"textDocument": {"uri": uri}, "position": {"line": 0, "character": 0}},
                timeout=60,
            ),
            "textDocument/hover",
        )


def result_summary(method, result):
    """Count returned work so an empty response is visible in comparisons."""
    if result in (None, [], {}):
        return 0, None
    if method == "textDocument/documentSymbol":

        def count(symbol):
            return 1 + sum(count(child) for child in symbol.get("children", []))

        return sum(count(symbol) for symbol in result), None
    if method in ("textDocument/definition", "textDocument/references"):
        locations = result if isinstance(result, list) else [result]
        uris = {
            location.get("uri") or location.get("targetUri") for location in locations
        }
        return len(locations), len(uris - {None})
    if method == "textDocument/rename":
        changes = result.get("changes") or {}
        count = sum(len(edits) for edits in changes.values())
        uris = set(changes)
        for change in result.get("documentChanges") or []:
            if document := change.get("textDocument"):
                count += len(change.get("edits", []))
                uris.add(document["uri"])
        return count, len(uris)
    return 1, None


def request_record(key, label, method, targets):
    return {
        "key": key,
        "label": label,
        "method": method,
        "targets": targets,
        "empty_results": 0,
        "_latencies_ms": [],
        "_result_counts": [],
        "_result_files": [],
        "_payload_bytes": [],
    }


def record_response(record, response, elapsed_ms):
    result = require_response(response, record["method"]).get("result")
    count, files = result_summary(record["method"], result)
    record["_latencies_ms"].append(elapsed_ms)
    record["_result_counts"].append(count)
    record["empty_results"] += count == 0
    if files is not None:
        record["_result_files"].append(files)
    record["_payload_bytes"].append(
        len(json.dumps(result, separators=(",", ":"), ensure_ascii=False).encode())
    )


def finalize_request_record(record):
    latencies = record["_latencies_ms"]
    record["samples"] = len(latencies)
    record["median_ms"] = round(statistics.median(latencies), 3)
    record["p95_ms"] = round(sorted(latencies)[math.ceil(len(latencies) * 0.95) - 1], 3)
    for field in ("result_counts", "result_files", "payload_bytes"):
        values = record[f"_{field}"]
        if values:
            prefix = "result_count" if field == "result_counts" else field
            record[f"{prefix}_min"] = min(values)
            record[f"{prefix}_median"] = statistics.median(values)
            record[f"{prefix}_max"] = max(values)
    return record


def benchmark_requests(client, key, label, method, params, runs, warmups, timeout):
    """Measure serial stdio round trips, excluding warmups and result summaries."""
    for _ in range(warmups):
        for target in params:
            require_response(client.request(method, target, timeout=timeout), method)
    record = request_record(key, label, method, len(params))
    for _ in range(runs):
        for target in params:
            started = time.perf_counter_ns()
            response = client.request(method, target, timeout=timeout)
            elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
            record_response(record, response, elapsed_ms)
    return finalize_request_record(record)


def benchmark_shared_requests(client, uris, target, runs, warmups, timeout):
    navigation = {
        "textDocument": {"uri": target["uri"]},
        "position": {**target["use"], "character": target["use"]["character"] + 1},
    }
    requests = [
        (
            "document_symbol",
            "Document symbols",
            "textDocument/documentSymbol",
            [{"textDocument": {"uri": uri}} for uri in uris[:3]],
        ),
        (
            "hover",
            "Hover",
            "textDocument/hover",
            [{"textDocument": {"uri": target["uri"]}, "position": target["hover"]}],
        ),
        ("definition", "Go to definition", "textDocument/definition", [navigation]),
        (
            "references",
            "Find references",
            "textDocument/references",
            [{**navigation, "context": {"includeDeclaration": True}}],
        ),
        (
            "rename",
            "Rename",
            "textDocument/rename",
            [{**navigation, "newName": "arity_lsp_renamed"}],
        ),
    ]
    return [
        benchmark_requests(client, key, label, method, params, runs, warmups, timeout)
        for key, label, method, params in requests
    ]


def churn_definitions(client, target, edits, timeout, text="", sync_kind=2):
    record = request_record(
        "edit_definition", "Edit to definition", "textDocument/definition", 1
    )
    record["stale_responses"] = 0
    start = target["use"]
    for edit in range(1, edits + 1):
        selected = edit % 2
        name = target["names"][selected]
        if sync_kind == 1:
            offset = target["use_offset"]
            text = text[:offset] + name + text[offset + len(name) :]
            changes = [{"text": text}]
        else:
            changes = [
                {
                    "range": {
                        "start": start,
                        "end": {
                            "line": start["line"],
                            "character": start["character"] + utf16_length(name),
                        },
                    },
                    "text": name,
                }
            ]
        started = time.perf_counter_ns()
        deadline = time.monotonic() + timeout
        client.notify(
            "textDocument/didChange",
            {
                "textDocument": {"uri": target["uri"], "version": edit + 1},
                "contentChanges": changes,
            },
        )
        while True:
            response = client.request(
                "textDocument/definition",
                {
                    "textDocument": {"uri": target["uri"]},
                    "position": {**start, "character": start["character"] + 1},
                },
                timeout=max(0, deadline - time.monotonic()),
            )
            result = require_response(response, "textDocument/definition").get("result")
            locations = (
                result if isinstance(result, list) else [result] if result else []
            )
            location = locations[0] if len(locations) == 1 else {}
            uri = location.get("uri") or location.get("targetUri")
            span = location.get("range") or location.get("targetSelectionRange") or {}
            # Some servers reply from a stale parse during their edit debounce.
            # Count that work and include it in time to the first correct answer.
            if (
                uri == target["uri"]
                and span.get("start") == target["definitions"][selected]
            ):
                break
            record["stale_responses"] += 1
            if time.monotonic() >= deadline:
                raise RuntimeError(
                    f"stale or incorrect definition after edit {edit}: {result}"
                )
            time.sleep(0.01)
        elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
        record_response(record, response, elapsed_ms)
    return finalize_request_record(record)


def isolated_environment(directory):
    env = os.environ.copy()
    config_home = Path(directory) / "config"
    cache_home = Path(directory) / "cache"
    config_home.mkdir()
    cache_home.mkdir()
    env["XDG_CONFIG_HOME"] = str(config_home)
    env["XDG_CACHE_HOME"] = str(cache_home)
    env["NO_COLOR"] = "1"
    env.pop("ARITY_REMOTE_URL", None)
    return env


def run_session(
    spec,
    run_number,
    project,
    files,
    edits,
    settle_timeout,
    quiet_seconds,
    stderr_dir,
    latency_runs,
    latency_warmups,
):
    key, command = spec
    print(f"==> speed and memory: {key} (run {run_number})", flush=True)
    stderr_path = Path(stderr_dir) / f"{key}-run-{run_number}.stderr.log"
    client = None
    sampler = None
    with tempfile.TemporaryDirectory(prefix=f"arity-lsp-{key}-") as state_dir:
        try:
            env = isolated_environment(state_dir)
            started_at = time.monotonic()
            client = Client(
                command,
                cwd=str(project),
                env=env,
                stderr_path=str(stderr_path),
            )
            sampler = Sampler(client.proc.pid)
            sampler.start()

            initialized = require_response(
                client.request(
                    "initialize",
                    {
                        "processId": os.getpid(),
                        "clientInfo": {"name": "arity-lsp-bench", "version": "1"},
                        "rootUri": project.as_uri(),
                        "rootPath": str(project),
                        "capabilities": CAPABILITIES,
                        "workspaceFolders": [
                            {"uri": project.as_uri(), "name": project.name}
                        ],
                        "initializationOptions": {},
                        "trace": "off",
                    },
                    timeout=settle_timeout,
                ),
                "initialize",
                require_result=True,
            )
            init_seconds = round(time.monotonic() - started_at, 6)
            capabilities = (initialized.get("result") or {}).get("capabilities", {})
            pull = bool(capabilities.get("diagnosticProvider"))
            sync = capabilities.get("textDocumentSync", 0)
            sync_kind = sync.get("change", 0) if isinstance(sync, dict) else sync
            if sync_kind not in (1, 2):
                raise RuntimeError(f"unsupported document synchronization: {sync}")
            client.notify("initialized", {})

            baseline_ready = wait_until_quiet(
                client,
                sampler,
                quiet_seconds,
                settle_timeout,
                "initialization",
                time.monotonic() - sampler.started_at,
            )
            workspace_ready_seconds = round(baseline_ready - started_at, 6)
            milestones = {"baseline": sampler.milestone()}
            baseline_seconds = round(time.monotonic() - started_at, 2)
            print(
                f"    baseline {milestones['baseline']['rss_mb']} MiB RSS", flush=True
            )

            documents, edit_target = prepare_documents(files)
            uris = [document["uri"] for document in documents]
            documents_started_at = time.monotonic()
            for document in documents:
                client.notify(
                    "textDocument/didOpen",
                    {
                        "textDocument": {
                            "uri": document["uri"],
                            "languageId": "r",
                            "version": 1,
                            "text": document["text"],
                        }
                    },
                )

            if pull:
                pull_diagnostics(client, uris, settle_timeout)
            else:
                client.wait_diagnostics(dict.fromkeys(uris, 1), settle_timeout)
            exercise_shared_requests(client, uris)
            documents_ready = wait_until_quiet(
                client,
                sampler,
                quiet_seconds,
                settle_timeout,
                "files-opened settle",
                time.monotonic() - sampler.started_at,
            )
            documents_ready_seconds = round(documents_ready - documents_started_at, 6)
            milestones["settled"] = sampler.milestone()
            settled_seconds = round(time.monotonic() - started_at, 2)
            print(f"    settled  {milestones['settled']['rss_mb']} MiB RSS", flush=True)

            request_latencies = benchmark_shared_requests(
                client, uris, edit_target, latency_runs, latency_warmups, settle_timeout
            )
            with client.state:
                before_edits = len(client.notifications)
            edit_started_at = time.monotonic()
            request_latencies.append(
                churn_definitions(
                    client,
                    edit_target,
                    edits,
                    timeout=settle_timeout,
                    text=documents[0]["text"],
                    sync_kind=sync_kind,
                )
            )
            edit_work_seconds = round(time.monotonic() - edit_started_at, 6)
            if pull:
                pull_diagnostics(client, uris, settle_timeout)
            else:
                client.wait_diagnostics(
                    {uris[0]: edits + 1}, settle_timeout, after=before_edits
                )
            wait_until_quiet(
                client,
                sampler,
                quiet_seconds,
                settle_timeout,
                "post-edit settle",
                time.monotonic() - sampler.started_at,
            )
            milestones["edited"] = sampler.milestone()
            edit_seconds = round(time.monotonic() - edit_started_at, 2)
            total_seconds = round(time.monotonic() - started_at, 2)

            sampler.stop_flag.set()
            sampler.join(timeout=2)
            milestones["peak"] = sampler.peak()
            result = {
                "run": run_number,
                "milestones": milestones,
                "init_seconds": init_seconds,
                "workspace_ready_seconds": workspace_ready_seconds,
                "documents_ready_seconds": documents_ready_seconds,
                "edit_work_seconds": edit_work_seconds,
                "baseline_seconds": baseline_seconds,
                "settled_seconds": settled_seconds,
                "edit_seconds": edit_seconds,
                "total_seconds": total_seconds,
                "diagnostic_mode": "pull" if pull else "push",
                "text_sync": "full" if sync_kind == 1 else "incremental",
                "diagnostics_published": client.count_published_diagnostics(),
                "diagnostic_requests": len(uris) * 2 if pull else 0,
                "definition_requests": edits + request_latencies[-1]["stale_responses"],
                "samples": len(sampler.samples),
                "request_latencies": request_latencies,
            }
            for record in request_latencies:
                print(
                    f"    {record['label']}: {record['median_ms']:.3f} ms median, "
                    f"{record['p95_ms']:.3f} ms p95 ({record['empty_results']} empty)",
                    flush=True,
                )
            print(
                f"    edited   {milestones['edited']['rss_mb']} MiB RSS"
                f"  (peak {milestones['peak']['rss_mb']} MiB, {total_seconds}s)",
                flush=True,
            )
            client.shutdown()
            client = None
            return result
        finally:
            if sampler is not None and sampler.is_alive():
                sampler.stop_flag.set()
                sampler.join(timeout=2)
            if client is not None:
                client.kill()


# --- aggregation and output ------------------------------------------------


def rounded_median(values, digits):
    return round(statistics.median(values), digits)


def public_record(value):
    """Keep per-run summaries while omitting internal raw sample arrays."""
    if isinstance(value, dict):
        return {
            key: public_record(item)
            for key, item in value.items()
            if not key.startswith("_")
        }
    if isinstance(value, list):
        return [public_record(item) for item in value]
    return value


def aggregate_latencies(runs):
    records = []
    for index, first in enumerate(runs[0]["request_latencies"]):
        combined = request_record(
            first["key"], first["label"], first["method"], first["targets"]
        )
        for run in runs:
            record = run["request_latencies"][index]
            combined["empty_results"] += record["empty_results"]
            combined["stale_responses"] = combined.get(
                "stale_responses", 0
            ) + record.get("stale_responses", 0)
            for field in (
                "_latencies_ms",
                "_result_counts",
                "_result_files",
                "_payload_bytes",
            ):
                combined[field].extend(record[field])
        records.append(public_record(finalize_request_record(combined)))
    return records


def aggregate_runs(runs):
    milestones = {}
    for milestone in MILESTONES:
        milestones[milestone] = {
            "rss_mb": rounded_median(
                [run["milestones"][milestone]["rss_mb"] for run in runs], 1
            ),
            "pss_mb": rounded_median(
                [run["milestones"][milestone]["pss_mb"] for run in runs], 1
            ),
            "processes": int(
                statistics.median(
                    [run["milestones"][milestone]["processes"] for run in runs]
                )
            ),
        }
    timing_keys = (
        "init_seconds",
        "workspace_ready_seconds",
        "documents_ready_seconds",
        "edit_work_seconds",
        "baseline_seconds",
        "settled_seconds",
        "edit_seconds",
        "total_seconds",
    )
    return {
        "milestones": milestones,
        "timings": {
            key: rounded_median([run[key] for run in runs], 6) for key in timing_keys
        },
        "request_latencies": aggregate_latencies(runs),
        "samples": int(statistics.median([run["samples"] for run in runs])),
    }


def host_metadata():
    cpu = "unknown"
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    memory_gb = 0
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                memory_gb = int(line.split()[1]) // 1024 // 1024
                break
    except OSError:
        pass
    return {
        "os": platform.system(),
        "arch": platform.machine(),
        "cpu": cpu,
        "memory_gb": memory_gb,
    }


def corpus_checkout(cache):
    project = cache / "tidyr"
    fresh = not project.exists()
    if fresh:
        cache.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                "git",
                "clone",
                "--no-checkout",
                "--filter=blob:none",
                CORPUS_REPO,
                str(project),
            ],
            check=True,
        )

    def git(*args):
        return subprocess.check_output(
            ["git", "-C", str(project), *args], text=True
        ).strip()

    if git("remote", "get-url", "origin") != CORPUS_REPO:
        raise ValueError(f"unexpected corpus origin: {project}")
    # Check before checkout so a benchmark never overwrites local work.
    if fresh or git("rev-parse", "--verify", "HEAD") != CORPUS_REVISION:
        if not fresh and git("status", "--porcelain", "--untracked-files=all"):
            raise ValueError(f"dirty corpus checkout: {project}")
        subprocess.run(
            [
                "git",
                "-C",
                str(project),
                "fetch",
                "--depth=1",
                "origin",
                CORPUS_REVISION,
            ],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(project), "checkout", "--detach", CORPUS_REVISION],
            check=True,
        )
    if git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError(f"dirty corpus checkout: {project}")
    return project


def write_artifact(output, payload):
    # CLI and LSP measurements share one artifact but can be refreshed separately.
    artifact = (
        json.loads(output.read_text()) if output.exists() else {"schema_version": 2}
    )
    artifact["lsp"] = payload
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=output.parent, delete=False) as tmp:
        json.dump(artifact, tmp, indent=2)
        tmp.write("\n")
    Path(tmp.name).replace(output)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--out", type=Path, default=ROOT / "benches/benchmark_results.json"
    )
    parser.add_argument(
        "--arity", type=Path, help="use an existing release binary instead of building"
    )
    parser.add_argument("--rscript", default="Rscript")
    parser.add_argument("--cache", type=Path, default=ROOT / "target/bench-lsp/corpus")
    parser.add_argument("--open-files", type=int, default=5)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--edits", type=int, default=1000)
    parser.add_argument("--latency-runs", type=int, default=20)
    parser.add_argument("--latency-warmups", type=int, default=2)
    parser.add_argument("--settle-timeout", type=float, default=120)
    parser.add_argument("--quiet-seconds", type=float, default=5)
    parser.add_argument(
        "--stderr-dir", type=Path, default=ROOT / "target/bench-lsp/logs"
    )
    args = parser.parse_args()

    if platform.system() != "Linux" or not Path("/proc/self/smaps_rollup").is_file():
        parser.error("the memory benchmark requires Linux with /proc/smaps_rollup")
    if args.runs < 1 or args.edits < 1 or args.open_files < 1:
        parser.error("--runs, --edits, and --open-files must be positive")
    if args.latency_runs < 1 or args.latency_warmups < 0:
        parser.error(
            "--latency-runs must be positive and --latency-warmups nonnegative"
        )
    if (
        args.quiet_seconds <= SAMPLE_INTERVAL_SECONDS * 2
        or args.settle_timeout <= args.quiet_seconds
    ):
        parser.error(
            "--quiet-seconds must exceed two sample intervals and be below --settle-timeout"
        )

    rscript = shutil.which(args.rscript)
    if rscript is None:
        parser.error(f"Rscript executable not found: {args.rscript}")
    args.rscript = str(Path(rscript).resolve())
    r_version = subprocess.check_output(
        [
            args.rscript,
            "--vanilla",
            "-e",
            'cat(paste0("languageserver ", packageVersion("languageserver"), "; R ", getRversion(), "; lintr ", packageVersion("lintr")))',
        ],
        text=True,
    ).strip()
    if args.arity is None:
        subprocess.run(
            ["cargo", "build", "--release", "--bin", "arity"], cwd=ROOT, check=True
        )
    arity = (args.arity or ROOT / "target/release/arity").resolve()
    versions = {
        "arity": subprocess.check_output([str(arity), "--version"], text=True).strip(),
        "languageserver": r_version,
    }
    project = corpus_checkout(args.cache.resolve())
    tracked = subprocess.check_output(
        ["git", "-C", str(project), "ls-files", "-z", "R/"], text=True
    ).split("\0")
    files = sorted(
        (project / path for path in tracked if path.lower().endswith(".r")),
        key=lambda path: (-path.stat().st_size, str(path)),
    )[: args.open_files]
    if len(files) != args.open_files:
        parser.error("corpus has fewer R files than --open-files")
    stderr_dir = args.stderr_dir.resolve()
    stderr_dir.mkdir(parents=True, exist_ok=True)
    specs = [
        ("arity", [str(arity), "--no-config", "lsp"]),
        ("languageserver", [args.rscript, "--vanilla", "-e", "languageserver::run()"]),
    ]
    keys = [key for key, _ in specs]

    runs_by_server = {key: [] for key in keys}
    session_index = 0
    for repetition in range(args.runs):
        order = specs if repetition % 2 == 0 else list(reversed(specs))
        for spec in order:
            session_index += 1
            key = spec[0]
            runs_by_server[key].append(
                run_session(
                    spec,
                    repetition + 1,
                    project,
                    files,
                    args.edits,
                    args.settle_timeout,
                    args.quiet_seconds,
                    stderr_dir,
                    args.latency_runs,
                    args.latency_warmups,
                )
            )
            if session_index < args.runs * len(specs):
                time.sleep(2)

    servers = []
    for key, command in specs:
        label, doing = SERVER_META.get(key, (key, ""))
        runs = runs_by_server[key]
        servers.append(
            {
                "key": key,
                "label": label,
                "doing": doing,
                "version": versions.get(key, "unknown"),
                "command": [Path(command[0]).name, *command[1:]],
                "runs": public_record(runs),
                "aggregate": aggregate_runs(runs),
            }
        )

    documents, _ = prepare_documents(files)
    payload = {
        "schema_version": SCHEMA_VERSION,
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "host": host_metadata(),
            "runs": args.runs,
            "quiet_seconds": args.quiet_seconds,
            "settle_timeout_seconds": args.settle_timeout,
            "edit_count": args.edits,
            "latency_runs": args.latency_runs,
            "latency_warmups": args.latency_warmups,
            "sample_interval_seconds": SAMPLE_INTERVAL_SECONDS,
            "edit_poll_seconds": 0.01,
        },
        "corpus": {
            "name": "tidyr v1.3.2",
            "repo": CORPUS_REPO,
            "revision": CORPUS_REVISION,
        },
        "session": {
            "project": project.name,
            "files": [str(path.relative_to(project)) for path in files],
            "file_count": len(files),
            "disk_bytes": sum(path.stat().st_size for path in files),
            "opened_bytes": sum(
                len(document["text"].encode()) for document in documents
            ),
            "edited_file": str(files[0].relative_to(project)),
        },
        "servers": servers,
    }
    output = Path(args.out)
    write_artifact(output, payload)
    print(f"==> wrote {output}")


if __name__ == "__main__":
    main()
