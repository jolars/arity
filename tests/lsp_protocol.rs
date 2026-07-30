//! In-memory LSP protocol / integration tests.
//!
//! Unlike `tests/lsp.rs` (which exercises the request handlers as pure
//! functions), these drive the *real* server loop over
//! [`lsp_server::Connection::memory`]: the initialize handshake, message
//! dispatch, request coalescing/supersession on the lint thread, and the
//! shutdown/exit lifecycle. This is the regression net for the planned
//! request-cancellation work (see `TODO.md`, "request cancellation +
//! stale-read protocol").
//!
//! Everything is timeout-guarded: a wedged server surfaces as a panic (test
//! failure), never a hang. Because linting runs asynchronously on a dedicated
//! thread, the harness waits for a *specific* message (e.g. a
//! `publishDiagnostics` at an exact version) and drains anything in between,
//! rather than asserting on message counts or ordering.

use std::thread::JoinHandle;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

/// Generous so slow CI never trips it; short enough that a genuinely wedged
/// server fails the test instead of hanging the suite.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A file URI for the single test document. Absolute on the host platform so
/// URI<->path conversion behaves (mirrors `src/lsp/test_support.rs`).
fn doc_uri() -> &'static str {
    if cfg!(windows) {
        "file:///C:/tmp/lsp_protocol_test.R"
    } else {
        "file:///tmp/lsp_protocol_test.R"
    }
}

/// A file URI for an `arity.toml` in the same directory as [`doc_uri`], for
/// exercising the `didChangeWatchedFiles` config-change path.
fn arity_toml_uri() -> &'static str {
    if cfg!(windows) {
        "file:///C:/tmp/arity.toml"
    } else {
        "file:///tmp/arity.toml"
    }
}

/// A file URI whose *parent directory does not exist* on disk. Config
/// discovery anchors on that directory, so this is the shape that used to make
/// `textDocument/formatting` answer `null` (an editor buffer in a directory
/// that was never created, or was deleted while open). On the Windows CI
/// runners `C:\tmp` is itself missing, which is how this first surfaced.
fn missing_dir_uri() -> &'static str {
    if cfg!(windows) {
        "file:///C:/arity-no-such-dir-9f3a/lsp_protocol_test.R"
    } else {
        "file:///arity-no-such-dir-9f3a/lsp_protocol_test.R"
    }
}

/// Drives the client end of an in-memory connection against a real `serve`
/// loop running on a background thread.
struct Harness {
    client: Connection,
    server: Option<JoinHandle<()>>,
    next_id: i32,
    /// The `capabilities` object from the initialize response, for the
    /// capabilities test.
    capabilities: Value,
}

impl Harness {
    /// Start the server and complete the initialize handshake in **push**
    /// diagnostic mode: the advertised client capabilities omit
    /// `textDocument.diagnostic`, so the server publishes diagnostics as
    /// `textDocument/publishDiagnostics` notifications (observable here). No
    /// `workspaceFolders`/`rootUri` are sent, so the server skips its workspace
    /// seed walk (hermetic, fast).
    fn start_push() -> Self {
        Self::start_with_capabilities(json!({ "textDocument": { "hover": {} } }))
    }

    /// Like [`start_push`] but with caller-chosen client `capabilities`, so a test
    /// can opt into features gated on them (e.g. dynamic watched-file registration).
    fn start_with_capabilities(capabilities: Value) -> Self {
        let (server_conn, client_conn) = Connection::memory();
        let server = std::thread::spawn(move || {
            let _ = arity::lsp::serve(server_conn);
        });

        let mut harness = Harness {
            client: client_conn,
            server: Some(server),
            next_id: 1,
            capabilities: Value::Null,
        };

        let init_id = harness.request(
            "initialize",
            json!({
                "processId": null,
                "clientInfo": { "name": "arity-protocol-test" },
                "capabilities": capabilities,
            }),
        );
        let resp = harness.recv_response(&init_id);
        let result = resp
            .response_result
            .expect("initialize succeeded with a result");
        harness.capabilities = result
            .get("capabilities")
            .cloned()
            .expect("initialize result carries capabilities");

        harness.notify("initialized", json!({}));
        harness
    }

    fn request(&mut self, method: &str, params: Value) -> RequestId {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        let req = Request {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        self.client
            .sender
            .send(Message::Request(req))
            .expect("send request");
        id
    }

    fn notify(&self, method: &str, params: Value) {
        let not = Notification {
            method: method.to_string(),
            params,
        };
        self.client
            .sender
            .send(Message::Notification(not))
            .expect("send notification");
    }

    fn did_open(&self, uri: &str, text: &str, version: i32) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "r",
                    "version": version,
                    "text": text,
                }
            }),
        );
    }

    fn did_change(&self, uri: &str, text: &str, version: i32) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [ { "text": text } ]
            }),
        );
    }

    /// Receive messages until `pred` matches, draining (ignoring) the rest. On
    /// timeout, panic with everything drained so far.
    fn recv_until(&self, what: &str, mut pred: impl FnMut(&Message) -> bool) -> Message {
        let mut drained: Vec<Message> = Vec::new();
        loop {
            match self.client.receiver.recv_timeout(TIMEOUT) {
                Ok(msg) => {
                    if pred(&msg) {
                        return msg;
                    }
                    drained.push(msg);
                }
                Err(err) => {
                    panic!("timed out waiting for {what} ({err}); drained so far: {drained:#?}")
                }
            }
        }
    }

    fn recv_response(&self, id: &RequestId) -> Response {
        let msg = self.recv_until(
            &format!("response to {id:?}"),
            |m| matches!(m, Message::Response(r) if &r.id == id),
        );
        match msg {
            Message::Response(r) => r,
            _ => unreachable!(),
        }
    }

    /// Wait for a `publishDiagnostics` notification for `uri` at exactly
    /// `version`, skipping any stale/earlier generations. Returns the
    /// `diagnostics` array.
    fn recv_publish_for(&self, uri: &str, version: i32) -> Vec<Value> {
        let msg = self.recv_until(
            &format!("publishDiagnostics for {uri} @ v{version}"),
            |m| match m {
                Message::Notification(n) if n.method == "textDocument/publishDiagnostics" => {
                    n.params.get("uri").and_then(Value::as_str) == Some(uri)
                        && n.params.get("version").and_then(Value::as_i64) == Some(version as i64)
                }
                _ => false,
            },
        );
        match msg {
            Message::Notification(n) => n
                .params
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .expect("publishDiagnostics carries a diagnostics array"),
            _ => unreachable!(),
        }
    }

    /// Graceful shutdown: `shutdown` request -> read its response -> `exit`
    /// notification -> join the server thread. A clean join asserts the loop
    /// broke and the lint thread was joined.
    fn shutdown(&mut self) {
        let id = self.request("shutdown", Value::Null);
        let resp = self.recv_response(&id);
        assert!(resp.response_result.is_ok(), "shutdown errored: {resp:?}");
        // `handle_shutdown` sends the response above, then blocks receiving the
        // exit notification; send it to release the loop.
        self.notify("exit", Value::Null);
        self.server
            .take()
            .expect("server handle")
            .join()
            .expect("server thread joined cleanly");
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // If a test panicked before `shutdown()`, dropping the client
        // disconnects the server's receiver, so its `select!` recv returns
        // `Err` and the main loop breaks on its own. Don't `join()` here:
        // joining (and asserting) inside `drop` during an unwind risks a
        // double panic. Just let the thread wind down.
        if let Some(handle) = self.server.take() {
            drop(std::mem::replace(&mut self.client, {
                // Replace the client with a throwaway disconnected connection so
                // the real one drops now, signaling the server to exit.
                let (a, _b) = Connection::memory();
                a
            }));
            let _ = handle.join();
        }
    }
}

// A default-on `unused-binding` finding: `x` is assigned but never read.
const BUGGY: &str = "x <- 1\nprint(2)\n";
// The same binding, now read: no finding.
const CLEAN: &str = "x <- 1\nprint(x)\n";

#[test]
fn initialize_advertises_core_capabilities() {
    let mut h = Harness::start_push();
    let caps = &h.capabilities;
    assert!(
        caps.get("documentFormattingProvider").is_some(),
        "formatting advertised: {caps:#?}"
    );
    assert!(caps.get("hoverProvider").is_some(), "hover advertised");
    assert!(
        caps.get("diagnosticProvider").is_some(),
        "diagnostic provider advertised"
    );
    // Injected via raw JSON (no typed field in lsp-types 0.97).
    assert_eq!(
        caps.get("typeHierarchyProvider"),
        Some(&json!(true)),
        "type hierarchy advertised"
    );
    h.shutdown();
}

#[test]
fn did_open_then_formatting_request_responds() {
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-1\n", 1);

    let id = h.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    let resp = h.recv_response(&id);
    let result = resp
        .response_result
        .expect("formatting succeeded with a result");
    let edits = result
        .as_array()
        .expect("formatting returns an array of edits");
    assert_eq!(edits.len(), 1, "one whole-document edit: {edits:#?}");
    assert_eq!(
        edits[0].get("newText").and_then(Value::as_str),
        Some("x <- 1\n"),
        "reformatted text"
    );
    h.shutdown();
}

#[test]
fn formatting_works_when_the_documents_directory_is_missing() {
    // Regression: config discovery canonicalized its anchor directory, so a
    // buffer whose parent directory doesn't exist made `resolve_settings` fail
    // and `textDocument/formatting` silently answer `null`. Discovery must
    // degrade to "no config file found" (default settings) instead.
    let mut h = Harness::start_push();
    let uri = missing_dir_uri();
    h.did_open(uri, "x<-1\n", 1);

    let id = h.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    let resp = h.recv_response(&id);
    let result = resp
        .response_result
        .expect("formatting succeeded with a result");
    let edits = result
        .as_array()
        .expect("formatting returns an array of edits, not null");
    assert_eq!(
        edits[0].get("newText").and_then(Value::as_str),
        Some("x <- 1\n"),
        "reformatted text: {edits:#?}"
    );
    h.shutdown();
}

#[test]
fn did_open_buggy_publishes_diagnostics() {
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, BUGGY, 1);

    let diags = h.recv_publish_for(uri, 1);
    assert!(
        !diags.is_empty(),
        "buggy buffer should publish at least one diagnostic"
    );
    h.shutdown();
}

#[test]
fn rapid_did_change_coalesces_and_supersedes() {
    let mut h = Harness::start_push();
    let uri = doc_uri();

    // Fire a burst without waiting for intermediate lints. Coalescing keeps the
    // latest version per URI; supersession cancels an in-flight analyze of the
    // same URI; the main loop's version gate drops any stale publish. The final
    // buffer is CLEAN, so the v4 publish must carry no diagnostics — proving the
    // earlier buggy generations never stuck.
    h.did_open(uri, BUGGY, 1);
    h.did_change(uri, BUGGY, 2);
    h.did_change(uri, BUGGY, 3);
    h.did_change(uri, CLEAN, 4);

    let diags = h.recv_publish_for(uri, 4);
    assert!(
        diags.is_empty(),
        "final (clean) version should publish no diagnostics, got: {diags:#?}"
    );
    h.shutdown();
}

#[test]
fn shutdown_exit_joins_cleanly() {
    let mut h = Harness::start_push();
    h.did_open(doc_uri(), CLEAN, 1);
    // The successful join inside `shutdown` is the assertion: the loop broke via
    // `handle_shutdown`, state dropped, and the lint thread joined.
    h.shutdown();
}

#[test]
fn cancel_request_returns_request_cancelled() {
    // A `$/cancelRequest` for an in-flight read short-circuits with the spec's
    // `RequestCancelled` (-32800). Ordering is practically deterministic: the
    // cancel notification is already queued on the client->server channel when
    // the main loop finishes dispatching the read, whereas the read's reply must
    // still traverse the lint thread + read pool before it lands on `out_rx`. So
    // the loop processes the cancel first, drops the request from its live set,
    // and the later reply is discarded. (The ironclad, timing-free guarantee
    // lives in the `state.rs` unit tests.)
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-1\n", 1);

    let id = h.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    h.notify("$/cancelRequest", json!({ "id": h.next_id - 1 }));

    let resp = h.recv_response(&id);
    let err = resp
        .response_result
        .expect_err("canceled request should error, got a result");
    assert_eq!(err.code, -32800, "RequestCancelled: {err:?}");
    h.shutdown();
}

#[test]
fn stale_read_returns_content_modified() {
    // A read computed against v1 that is superseded by a v2 edit before it
    // replies must not deliver a stale result: the main loop returns
    // `ContentModified` (-32801) so the client re-requests. Same ordering
    // argument as the cancel test: the `didChange` (which bumps the tracked
    // version to 2) is already queued when the read is dispatched, so the loop
    // sees v2 by the time the v1 reply arrives.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-1\n", 1);

    let id = h.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    // Supersede the buffer the read was dispatched against.
    h.did_change(uri, "y<-2\n", 2);

    let resp = h.recv_response(&id);
    let err = resp
        .response_result
        .expect_err("superseded read should error, got a result");
    assert_eq!(err.code, -32801, "ContentModified: {err:?}");
    h.shutdown();
}

#[test]
fn registers_file_watchers_when_client_supports_dynamic_registration() {
    // A client that supports dynamic registration for watched files gets a
    // `client/registerCapability` at startup covering R sources, config, and
    // package metadata (there is no static server capability for this).
    let mut h = Harness::start_with_capabilities(json!({
        "textDocument": { "hover": {} },
        "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } },
    }));

    let msg = h.recv_until(
        "client/registerCapability",
        |m| matches!(m, Message::Request(r) if r.method == "client/registerCapability"),
    );
    let Message::Request(req) = msg else {
        unreachable!()
    };
    let reg = req
        .params
        .get("registrations")
        .and_then(Value::as_array)
        .expect("registrations array")
        .iter()
        .find(|r| {
            r.get("method").and_then(Value::as_str) == Some("workspace/didChangeWatchedFiles")
        })
        .expect("a watched-files registration");
    let globs: Vec<&str> = reg
        .pointer("/registerOptions/watchers")
        .and_then(Value::as_array)
        .expect("watchers array")
        .iter()
        .filter_map(|w| w.get("globPattern").and_then(Value::as_str))
        .collect();
    for want in [
        "**/*.{R,r}",
        "**/arity.toml",
        "**/DESCRIPTION",
        "**/NAMESPACE",
    ] {
        assert!(globs.contains(&want), "watches {want}: {globs:?}");
    }
    h.shutdown();
}

#[test]
fn watched_arity_toml_change_relints_open_documents() {
    // An `arity.toml` change reaches the server as `didChangeWatchedFiles` and
    // re-lints open buffers without any edit — the config cache is dropped and the
    // finding is republished at the same document version.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, BUGGY, 1);
    let first = h.recv_publish_for(uri, 1);
    assert!(!first.is_empty(), "buggy doc has a finding");

    h.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [ { "uri": arity_toml_uri(), "type": 2 } ] }),
    );

    // The re-lint republishes for the same (unchanged) version.
    let again = h.recv_publish_for(uri, 1);
    assert_eq!(
        again.len(),
        first.len(),
        "re-lint republishes the finding after the config change"
    );
    h.shutdown();
}
