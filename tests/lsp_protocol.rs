//! In-memory LSP protocol / integration tests.
//!
//! Unlike `tests/lsp.rs` (which exercises the request handlers as pure
//! functions), these drive the *real* server loop over
//! [`lsp_server::Connection::memory`]: the initialize handshake, message
//! dispatch, request coalescing/supersession on the lint thread, and the
//! shutdown/exit lifecycle. This is the regression net for request cancellation
//! and stale-read gating as well as the surrounding protocol machinery.
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

/// A file URI for a `DESCRIPTION` in the same directory as [`doc_uri`]. The
/// directory is not a package root, so this exercises routing without pulling
/// in the diagnostics gate.
fn description_uri() -> &'static str {
    if cfg!(windows) {
        "file:///C:/tmp/DESCRIPTION"
    } else {
        "file:///tmp/DESCRIPTION"
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

/// A `file:` URI for an on-disk path, matching `src/lsp/uri.rs`'s convention of
/// a leading `/` before a Windows drive letter.
fn path_uri(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

/// A `DESCRIPTION` with every field `R CMD check` requires, so a test varies
/// only the one thing it is about (mirrors `tests/lint_description.rs`).
const COMPLETE_DESCRIPTION: &str = "\
Package: testpkg
Type: Package
Title: A Test Package
Version: 0.1.0
Authors@R: person(\"A\", \"B\", email = \"a@b.co\", role = c(\"aut\", \"cre\"))
Description: One sentence: it has a colon.
License: MIT + file LICENSE
Encoding: UTF-8
";

/// A real on-disk package: `DESCRIPTION`, `R/a.R`, `NAMESPACE`, and an **empty
/// `arity.toml`**. The last is load-bearing — without it, config discovery walks
/// out of the temp dir and a stray ancestor `arity.toml` could change the rule
/// set under the test.
///
/// Returns the dir (the caller must keep it alive) plus the two URIs.
fn package_fixture(description: &str, r_source: &str) -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("R")).expect("create R/");
    std::fs::write(root.join("arity.toml"), "").expect("write arity.toml");
    std::fs::write(root.join("DESCRIPTION"), description).expect("write DESCRIPTION");
    std::fs::write(root.join("NAMESPACE"), "").expect("write NAMESPACE");
    std::fs::write(root.join("R").join("a.R"), r_source).expect("write R/a.R");
    let desc_uri = path_uri(&root.join("DESCRIPTION"));
    let r_uri = path_uri(&root.join("R").join("a.R"));
    (dir, desc_uri, r_uri)
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
        self.did_open_as(uri, text, version, "r");
    }

    /// `didOpen` with a caller-chosen `languageId`. The server routes on the
    /// URI's file name, not this field, so a test can send a deliberately wrong
    /// one to pin that precedence.
    fn did_open_as(&self, uri: &str, text: &str, version: i32, language_id: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
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

    /// Send a `didChange` carrying raw `contentChanges` (ranged incremental
    /// edits and/or full-document replacements), applied by the server in order.
    fn did_change_raw(&self, uri: &str, version: i32, content_changes: Value) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": content_changes,
            }),
        );
    }

    /// Format the current buffer and return the document its edits produce,
    /// applied to `buffer` — the text the caller believes the server stores.
    ///
    /// Formatting answers with edits scoped to the lines that change, so the
    /// reply is no longer the whole document on its own. The callers keep every
    /// line of their fixture un-canonical (the `x<-` prefix), so every line
    /// comes back as a hunk and the read-back is still exact: a wrong splice
    /// shows up as a hunk that rewrites text the caller never sent.
    fn formatted_buffer(&mut self, uri: &str, buffer: &str) -> String {
        let id = self.request(
            "textDocument/formatting",
            json!({
                "textDocument": { "uri": uri },
                "options": { "tabSize": 2, "insertSpaces": true }
            }),
        );
        let resp = self.recv_response(&id);
        let edits = resp
            .response_result
            .expect("formatting result")
            .as_array()
            .expect("array of edits")
            .clone();
        apply_edits(buffer, &edits)
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

    /// Wait for a `publishDiagnostics` for `uri` whose diagnostics satisfy
    /// `pred`. Needed where a re-lint republishes at the *same* version (a
    /// cross-file `RelintAll` does not bump the buffer's version), so
    /// [`recv_publish_for`](Self::recv_publish_for) would return the stale
    /// generation.
    fn recv_publish_matching(
        &self,
        uri: &str,
        what: &str,
        mut pred: impl FnMut(&[Value]) -> bool,
    ) -> Vec<Value> {
        let msg = self.recv_until(
            &format!("publishDiagnostics for {uri} {what}"),
            |m| match m {
                Message::Notification(n) if n.method == "textDocument/publishDiagnostics" => {
                    n.params.get("uri").and_then(Value::as_str) == Some(uri)
                        && n.params
                            .get("diagnostics")
                            .and_then(Value::as_array)
                            .is_some_and(|d| pred(d))
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

    /// Drain for `window` and count the `publishDiagnostics` notifications for
    /// `uri`. Used where the *number* of generations is the property under test.
    fn count_publishes_for(&self, uri: &str, window: Duration) -> usize {
        let deadline = std::time::Instant::now() + window;
        let mut seen = 0;
        while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
            match self.client.receiver.recv_timeout(remaining) {
                Ok(Message::Notification(n))
                    if n.method == "textDocument/publishDiagnostics"
                        && n.params.get("uri").and_then(Value::as_str) == Some(uri) =>
                {
                    seen += 1;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        seen
    }

    /// Assert that nothing is published for `uri` within `window`. The absence
    /// counterpart to [`recv_until`](Self::recv_until), for the negative
    /// controls: a *shorter* wait than [`TIMEOUT`], since it is spent on every
    /// run of a passing test.
    fn expect_no_publish_for(&self, uri: &str, window: Duration) {
        let deadline = std::time::Instant::now() + window;
        while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
            match self.client.receiver.recv_timeout(remaining) {
                Ok(Message::Notification(n))
                    if n.method == "textDocument/publishDiagnostics"
                        && n.params.get("uri").and_then(Value::as_str) == Some(uri) =>
                {
                    panic!("expected no publish for {uri}, got {:#?}", n.params);
                }
                Ok(_) => {}
                Err(_) => return,
            }
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

/// Apply formatting `edits` (wire JSON) to `text` the way a client does: every
/// range indexes the *original* document, so splicing from the end keeps the
/// earlier offsets valid. Checks the LSP requirement that they do not overlap
/// along the way.
///
/// Positions are resolved here rather than through the server's own
/// `LineIndex`, so a bug in it cannot cancel itself out. UTF-16 throughout,
/// which is what these tests negotiate.
fn apply_edits(text: &str, edits: &[Value]) -> String {
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|edit| {
            let at = |which: &str| {
                let line = edit
                    .pointer(&format!("/range/{which}/line"))
                    .and_then(Value::as_u64)
                    .expect("edit position line");
                let character = edit
                    .pointer(&format!("/range/{which}/character"))
                    .and_then(Value::as_u64)
                    .expect("edit position character");
                utf16_position_to_byte(text, line as usize, character as usize)
            };
            (
                at("start"),
                at("end"),
                edit.get("newText")
                    .and_then(Value::as_str)
                    .expect("edit newText"),
            )
        })
        .collect();
    spans.sort_by_key(|&(start, ..)| start);
    for pair in spans.windows(2) {
        assert!(pair[0].1 <= pair[1].0, "edits must not overlap: {spans:?}");
    }
    let mut out = text.to_string();
    for &(start, end, new_text) in spans.iter().rev() {
        out.replace_range(start..end, new_text);
    }
    out
}

/// The byte offset of UTF-16 `character` on `line` of `text`, clamped to the
/// end of the text (a range end may sit one line past the last one).
fn utf16_position_to_byte(text: &str, line: usize, character: usize) -> usize {
    let mut at = 0;
    for _ in 0..line {
        match text[at..].find('\n') {
            Some(offset) => at += offset + 1,
            None => return text.len(),
        }
    }
    let mut units = 0;
    for ch in text[at..].chars() {
        if units >= character || ch == '\n' {
            break;
        }
        units += ch.len_utf16();
        at += ch.len_utf8();
    }
    at
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
    // `TextDocumentSyncKind::INCREMENTAL` serializes as the integer `2`.
    assert_eq!(
        caps.get("textDocumentSync"),
        Some(&json!(2)),
        "incremental text sync advertised: {caps:#?}"
    );
    assert!(
        caps.get("diagnosticProvider").is_some(),
        "diagnostic provider advertised"
    );
    // An explicit `false`: the label and its tooltip both come from the initial
    // response, so there is nothing for `inlayHint/resolve` to fetch.
    assert_eq!(
        caps.get("inlayHintProvider"),
        Some(&json!({ "resolveProvider": false })),
        "inlay hints advertised: {caps:#?}"
    );
    // Injected via raw JSON (no typed field in lsp-types 0.97).
    assert_eq!(
        caps.get("typeHierarchyProvider"),
        Some(&json!(true)),
        "type hierarchy advertised"
    );
    // `=` starts a named argument, so it both triggers and re-triggers
    // signature help to refresh the active parameter.
    let signature = caps
        .get("signatureHelpProvider")
        .expect("signature help advertised");
    assert_eq!(
        signature.get("triggerCharacters"),
        Some(&json!(["(", ",", "="])),
        "signature help trigger characters: {signature:#?}"
    );
    assert_eq!(
        signature.get("retriggerCharacters"),
        Some(&json!([")", "="])),
        "signature help retrigger characters: {signature:#?}"
    );
    h.shutdown();
}

#[test]
fn initialize_advertises_file_and_folder_rename_operations() {
    // `FileOperationPatternKind` serializes lowercase, and the folder filter is
    // what makes a client send `willRenameFiles` for a directory move at all —
    // both are wire-level facts a typed assertion would not catch.
    let mut h = Harness::start_push();
    let file_ops = h
        .capabilities
        .get("workspace")
        .and_then(|w| w.get("fileOperations"))
        .expect("file operations advertised");

    for op in ["willRename", "didRename"] {
        let filters = file_ops
            .get(op)
            .and_then(|r| r.get("filters"))
            .unwrap_or_else(|| panic!("{op} registered: {file_ops:#?}"));
        assert_eq!(
            filters,
            &json!([
                { "scheme": "file", "pattern": { "glob": "**/*.{R,r}", "matches": "file" } },
                { "scheme": "file", "pattern": { "glob": "**", "matches": "folder" } },
            ]),
            "{op} covers .R files and any folder"
        );
    }
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
    // Edits are line-scoped; this document is one line, so it is one edit.
    assert_eq!(edits.len(), 1, "one changed line is one edit: {edits:#?}");
    assert_eq!(
        edits[0].get("newText").and_then(Value::as_str),
        Some("x <- 1\n"),
        "reformatted text"
    );
    h.shutdown();
}

#[test]
fn formatting_reloads_changed_config_without_watcher_support() {
    // Neovim deliberately does not advertise dynamic watched-file registration
    // on Linux. A config edit must therefore become visible on the next format
    // request without a `workspace/didChangeWatchedFiles` notification.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join(".git")).expect("create git boundary");
    let config_path = dir.path().join("arity.toml");
    std::fs::write(&config_path, "[format]\nline-width = 40\n").expect("write config");
    let uri = path_uri(&dir.path().join("main.R"));
    let source = "x<-foo(alpha,beta,gamma,delta,epsilon,zeta,eta,theta)\n";

    let mut h = Harness::start_push();
    h.did_open(&uri, source, 1);
    let narrow = h.formatted_buffer(&uri, source);

    // Keep the byte length unchanged and advance the mtime explicitly so the
    // assertion remains reliable on filesystems with coarse timestamps.
    std::fs::write(&config_path, "[format]\nline-width = 80\n").expect("rewrite config");
    let modified = std::fs::metadata(&config_path)
        .expect("config metadata")
        .modified()
        .expect("config mtime");
    std::fs::File::options()
        .append(true)
        .open(&config_path)
        .expect("open config")
        .set_modified(modified + Duration::from_secs(2))
        .expect("advance config mtime");

    let wide = h.formatted_buffer(&uri, source);
    assert_ne!(
        wide, narrow,
        "the updated line width must change formatting"
    );
    assert_eq!(
        wide,
        "x <- foo(alpha, beta, gamma, delta, epsilon, zeta, eta, theta)\n"
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

/// A perfectly valid `DESCRIPTION`. As *R* it is a pile of syntax errors (eight,
/// at the time of writing): `Title: A Test Package` is three juxtaposed symbols,
/// `Authors@R:` is a slot access, and so on.
const DESCRIPTION: &str = "\
Package: testpkg
Type: Package
Title: A Test Package
Version: 0.1.0
Authors@R: person(\"A\", \"B\", email = \"a@b.co\", role = c(\"aut\", \"cre\"))
Description: One sentence: it has a colon.
License: MIT + file LICENSE
Encoding: UTF-8
";

#[test]
fn did_open_description_publishes_no_r_syntax_errors() {
    let mut h = Harness::start_push();
    let uri = description_uri();
    // Deliberately mislabeled: some clients register `DESCRIPTION` under the R
    // language (as `editors/code` already does for `NAMESPACE`). The URI's file
    // name must win, or we parse DCF with the R grammar.
    h.did_open_as(uri, DESCRIPTION, 1, "r");

    let diags = h.recv_publish_for(uri, 1);
    assert!(
        diags.is_empty(),
        "a DESCRIPTION must not be linted as R, got: {diags:#?}"
    );
    h.shutdown();
}

/// A `DESCRIPTION` that also happens to be *valid R* — each line is a `:` call
/// between two symbols — so nothing upstream bails on a parse error and the R
/// formatter really would rewrite it (to `Package:testpkg`, stripping the space
/// `read.dcf` needs). The guard, not a parse failure, has to be what stops it.
const DESCRIPTION_VALID_AS_R: &str = "Package: testpkg\nDepends: R\n";

#[test]
fn formatting_a_description_uses_the_dcf_grammar_over_the_wire() {
    let mut h = Harness::start_push();
    let uri = description_uri();
    h.did_open_as(uri, DESCRIPTION_VALID_AS_R, 1, "r-description");

    let id = h.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    let resp = h.recv_response(&id);
    let edits = resp
        .response_result
        .expect("formatting result")
        .as_array()
        .expect("array of edits")
        .clone();
    let formatted = apply_edits(DESCRIPTION_VALID_AS_R, &edits);
    // The single assertion that guards the whole feature: `Package: testpkg`
    // keeps the space `read.dcf` needs. The R formatter would have written
    // `Package:testpkg`, which is a different file to R.
    assert!(
        formatted.starts_with("Package: testpkg\n"),
        "formatted as R, not DCF: {formatted:?}"
    );
    assert!(formatted.contains("Depends:\n    R\n"), "{formatted:?}");
    h.shutdown();
}

#[test]
fn range_formatting_a_description_returns_null_over_the_wire() {
    // Canonical field order is a whole-document property, so "format these three
    // lines" has no coherent answer. Format-on-save uses the document provider,
    // which does work, so this is not a silently broken formatter.
    let mut h = Harness::start_push();
    let uri = description_uri();
    h.did_open_as(uri, DESCRIPTION_VALID_AS_R, 1, "r-description");

    let id = h.request(
        "textDocument/rangeFormatting",
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    let resp = h.recv_response(&id);
    assert_eq!(
        resp.response_result.expect("range formatting result"),
        Value::Null
    );
    h.shutdown();
}

#[test]
fn formatting_a_description_is_declined_when_the_config_says_so() {
    let (dir, desc_uri, _) = package_fixture(DESCRIPTION_VALID_AS_R, "f <- function() 1\n");
    std::fs::write(
        dir.path().join("arity.toml"),
        "[format]\ndescription = false\n",
    )
    .expect("arity.toml");

    let mut h = Harness::start_push();
    h.did_open_as(&desc_uri, DESCRIPTION_VALID_AS_R, 1, "r-description");

    let id = h.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": desc_uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    let resp = h.recv_response(&id);
    assert_eq!(
        resp.response_result.expect("formatting result"),
        Value::Null
    );
    h.shutdown();
}

/// Pull the `code` (the rule id) out of each published diagnostic.
fn rules_in(diags: &[Value]) -> Vec<String> {
    diags
        .iter()
        .filter_map(|d| d.get("code").and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[test]
fn did_open_description_publishes_packaging_diagnostics() {
    // Everything `R CMD check` wants except `License`.
    let description = COMPLETE_DESCRIPTION.replace("License: MIT + file LICENSE\n", "");
    let (_dir, desc_uri, _) = package_fixture(&description, "f <- function() 1\n");

    let mut h = Harness::start_push();
    h.did_open_as(&desc_uri, &description, 1, "r-description");

    let diags = h.recv_publish_for(&desc_uri, 1);
    assert_eq!(
        rules_in(&diags),
        vec!["description-missing-field"],
        "expected the missing `License` to be reported: {diags:#?}"
    );
    h.shutdown();
}

#[test]
fn description_outside_a_package_publishes_nothing() {
    // A `DESCRIPTION` with no `R/` beside it is not a package root, so the
    // editor gets silence rather than a screenful of missing-field findings.
    // The editor opens files for reasons a CLI never has.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("arity.toml"), "").expect("write arity.toml");
    let path = dir.path().join("DESCRIPTION");
    std::fs::write(&path, "Package: notapackage\n").expect("write DESCRIPTION");
    let uri = path_uri(&path);

    let mut h = Harness::start_push();
    h.did_open_as(&uri, "Package: notapackage\n", 1, "r-description");

    let diags = h.recv_publish_for(&uri, 1);
    assert!(diags.is_empty(), "expected silence, got: {diags:#?}");
    h.shutdown();
}

#[test]
fn a_malformed_description_publishes_dcf_syntax_errors() {
    let (_dir, desc_uri, _) = package_fixture(COMPLETE_DESCRIPTION, "f <- function() 1\n");
    // A continuation line with no field to continue: what `read.dcf` rejects.
    let text = "  orphan continuation\n";

    let mut h = Harness::start_push();
    h.did_open_as(&desc_uri, text, 1, "r-description");

    let diags = h.recv_publish_for(&desc_uri, 1);
    assert_eq!(rules_in(&diags), vec!["syntax-error"], "{diags:#?}");
    h.shutdown();
}

/// The headline: a DESCRIPTION edit the user has not saved must already count
/// for the R files around it. `dplyr::filter` is undeclared until `Imports`
/// names it, and the editor should agree the moment it is typed.
#[test]
fn editing_a_description_dependency_relints_an_open_r_file() {
    let (_dir, desc_uri, r_uri) = package_fixture(
        COMPLETE_DESCRIPTION,
        "f <- function(x) dplyr::filter(x, TRUE)\n",
    );

    let mut h = Harness::start_push();
    h.did_open(&r_uri, "f <- function(x) dplyr::filter(x, TRUE)\n", 1);
    let diags = h.recv_publish_matching(&r_uri, "reporting the undeclared dplyr", |d| {
        rules_in(d).contains(&"undeclared-dependency".to_string())
    });
    assert!(!diags.is_empty());

    h.did_open_as(&desc_uri, COMPLETE_DESCRIPTION, 1, "r-description");
    let _ = h.recv_publish_for(&desc_uri, 1);

    // Declare it — in the buffer only, nothing written to disk.
    let declared = COMPLETE_DESCRIPTION.replace("Encoding: UTF-8\n", "Imports:\n    dplyr\n");
    h.did_change(&desc_uri, &declared, 2);

    h.recv_publish_matching(&r_uri, "cleared once dplyr is declared", |d| {
        !rules_in(d).contains(&"undeclared-dependency".to_string())
    });
    h.shutdown();
}

/// The `description_facts` firewall's negative control. `Description:` is prose:
/// no R file's diagnostics can turn on it, so typing there must not re-lint the
/// package. Without the facts comparison every keystroke would fan out.
#[test]
fn editing_description_prose_does_not_relint_other_documents() {
    let (_dir, desc_uri, r_uri) = package_fixture(COMPLETE_DESCRIPTION, "f <- function() 1\n");

    let mut h = Harness::start_push();
    h.did_open(&r_uri, "f <- function() 1\n", 1);
    let _ = h.recv_publish_for(&r_uri, 1);
    h.did_open_as(&desc_uri, COMPLETE_DESCRIPTION, 1, "r-description");
    let _ = h.recv_publish_for(&desc_uri, 1);

    let edited = COMPLETE_DESCRIPTION.replace(
        "Description: One sentence: it has a colon.",
        "Description: One sentence: it has a colon and more words.",
    );
    h.did_change(&desc_uri, &edited, 2);
    // The DESCRIPTION itself still re-lints; `a.R` must not.
    let _ = h.recv_publish_for(&desc_uri, 2);
    h.expect_no_publish_for(&r_uri, Duration::from_millis(400));
    h.shutdown();
}

/// A facts change fans out with `RelintAll`, which re-lints *every* open
/// document — including the DESCRIPTION that triggered it. That converges only
/// because `upsert_description` short-circuits on unchanged text, so the second
/// pass sees the facts compare equal and stops. Lose that guard and the server
/// spins, publishing forever. Nothing else in the suite would notice.
#[test]
fn a_facts_change_relints_once_and_settles() {
    let (_dir, desc_uri, r_uri) = package_fixture(COMPLETE_DESCRIPTION, "f <- function() 1\n");

    let mut h = Harness::start_push();
    h.did_open(&r_uri, "f <- function() 1\n", 1);
    let _ = h.recv_publish_for(&r_uri, 1);
    h.did_open_as(&desc_uri, COMPLETE_DESCRIPTION, 1, "r-description");
    let _ = h.recv_publish_for(&desc_uri, 1);

    // `Imports` is a fact the project graph consumes, so this does fan out.
    let declared = COMPLETE_DESCRIPTION.replace("Encoding: UTF-8\n", "Imports:\n    dplyr\n");
    h.did_change(&desc_uri, &declared, 2);

    // Exactly two: the edit's own lint, plus the one re-lint the fan-out costs.
    // A third would mean the loop is feeding itself.
    let publishes = h.count_publishes_for(&desc_uri, Duration::from_millis(600));
    assert_eq!(
        publishes, 2,
        "expected one lint plus one fan-out re-lint, then silence"
    );
    h.shutdown();
}

/// Closing a dirty buffer must put the on-disk facts back. Otherwise the
/// unsaved dependency list outlives the editor session and silently gates every
/// R diagnostic in the package.
#[test]
fn closing_a_dirty_description_restores_the_on_disk_facts() {
    let (_dir, desc_uri, r_uri) = package_fixture(
        COMPLETE_DESCRIPTION,
        "f <- function(x) dplyr::filter(x, TRUE)\n",
    );

    let mut h = Harness::start_push();
    h.did_open(&r_uri, "f <- function(x) dplyr::filter(x, TRUE)\n", 1);
    let _ = h.recv_publish_matching(&r_uri, "reporting the undeclared dplyr", |d| {
        rules_in(d).contains(&"undeclared-dependency".to_string())
    });

    h.did_open_as(&desc_uri, COMPLETE_DESCRIPTION, 1, "r-description");
    let _ = h.recv_publish_for(&desc_uri, 1);
    let declared = COMPLETE_DESCRIPTION.replace("Encoding: UTF-8\n", "Imports:\n    dplyr\n");
    h.did_change(&desc_uri, &declared, 2);
    let _ = h.recv_publish_matching(&r_uri, "cleared once dplyr is declared", |d| {
        !rules_in(d).contains(&"undeclared-dependency".to_string())
    });

    // Close without saving: disk never gained the `Imports`, so the finding
    // must come back.
    h.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": desc_uri } }),
    );
    h.recv_publish_matching(&r_uri, "reporting dplyr again after the close", |d| {
        rules_in(d).contains(&"undeclared-dependency".to_string())
    });
    h.shutdown();
}

/// The same dispatch claim as the completion test below, for inlay hints: the
/// DESCRIPTION has to reach the DCF resolver. An array — even an empty one —
/// proves it routed; `null` is what a declined document looks like. The versions
/// themselves come from the real machine's library via an async harvest, so
/// asserting on them here would not be hermetic (`src/lsp/description.rs` owns
/// that).
#[test]
fn inlay_hints_in_a_dependency_field_reach_the_client() {
    let description = "Package: testpkg\nImports: dplyr (>= 1.0)\n";
    let (_dir, desc_uri, _) = package_fixture(description, "f <- function() 1\n");

    let mut h = Harness::start_push();
    h.did_open_as(&desc_uri, description, 1, "r-description");

    let id = h.request(
        "textDocument/inlayHint",
        json!({
            "textDocument": { "uri": desc_uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 2, "character": 0 }
            }
        }),
    );
    let resp = h.recv_response(&id);
    let result = resp.response_result.expect("inlay hint result");
    assert!(result.is_array(), "the DESCRIPTION routed: {result:?}");
    h.shutdown();
}

/// R has no hints yet, and the main loop declines before spending a read slot —
/// this is the test that changes the day argument-name hints land.
#[test]
fn inlay_hints_for_an_r_document_are_declined() {
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, CLEAN, 1);

    let id = h.request(
        "textDocument/inlayHint",
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            }
        }),
    );
    let resp = h.recv_response(&id);
    assert_eq!(
        resp.response_result.expect("an inlay hint result"),
        Value::Null
    );
    h.shutdown();
}

/// Inlay hints are pull-only: a change in cross-file context that arrives without
/// a document edit reaches them solely through `workspace/inlayHint/refresh`. A
/// push-mode harness, so this also pins that the refresh is not gated on the pull
/// diagnostic model.
#[test]
fn a_watched_config_change_asks_the_client_to_refresh_inlay_hints() {
    let mut h = Harness::start_with_capabilities(json!({
        "textDocument": { "hover": {} },
        "workspace": { "inlayHint": { "refreshSupport": true } }
    }));
    let uri = doc_uri();
    h.did_open(uri, CLEAN, 1);

    h.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [ { "uri": arity_toml_uri(), "type": 2 } ] }),
    );

    h.recv_until(
        "an inlay hint refresh",
        |msg| matches!(msg, Message::Request(req) if req.method == "workspace/inlayHint/refresh"),
    );
    h.shutdown();
}

/// The read path end to end: `on_completion` has to hand a DESCRIPTION to the
/// DCF resolver rather than declining it as non-R. Nothing in the unit tests
/// covers the dispatch.
#[test]
fn completion_in_a_dependency_field_reaches_the_client() {
    let description = "Package: testpkg\nImports: dp\n";
    let (_dir, desc_uri, _) = package_fixture(description, "f <- function() 1\n");

    let mut h = Harness::start_push();
    h.did_open_as(&desc_uri, description, 1, "r-description");

    let id = h.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": desc_uri },
            // End of `Imports: dp`.
            "position": { "line": 1, "character": 11 }
        }),
    );
    let resp = h.recv_response(&id);
    let result = resp.response_result.expect("completion result");
    let items = result
        .get("items")
        .and_then(Value::as_array)
        .expect("a completion list");
    let labels: Vec<&str> = items
        .iter()
        .filter_map(|i| i.get("label").and_then(Value::as_str))
        .collect();
    assert!(labels.contains(&"dplyr"), "{labels:?}");
    assert!(
        labels.iter().all(|l| l.starts_with("dp")),
        "the typed prefix must filter: {labels:?}"
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
fn incremental_did_change_applies_ranged_edit() {
    // A single ranged edit replaces `1` with `42` in `x<-1\n`. The server must
    // convert the LSP range to byte offsets and splice its buffer; formatting the
    // result proves the stored buffer became `x<-42\n`.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-1\n", 1);
    // `x<-1`: x(0) '<'(1) '-'(2) '1'(3). Replace char 3..4 with `42`.
    h.did_change_raw(
        uri,
        2,
        json!([{
            "range": { "start": { "line": 0, "character": 3 },
                       "end": { "line": 0, "character": 4 } },
            "text": "42"
        }]),
    );
    assert_eq!(h.formatted_buffer(uri, "x<-42\n"), "x <- 42\n");
    h.shutdown();
}

#[test]
fn incremental_did_change_applies_ordered_changes() {
    // Two ranged edits in one notification, where the second targets a line that
    // only exists after the first applied — so the server must re-index between
    // changes (positions are against the buffer left by the prior change).
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "foo\nbar\n", 1);
    h.did_change_raw(
        uri,
        2,
        json!([
            // Insert a new first line: buffer becomes `x<-1\nfoo\nbar\n`.
            { "range": { "start": { "line": 0, "character": 0 },
                         "end": { "line": 0, "character": 0 } },
              "text": "x<-1\n" },
            // `bar` is now on line 2 (it only moved there because of the change
            // above); replace it with `y<-2`.
            { "range": { "start": { "line": 2, "character": 0 },
                         "end": { "line": 2, "character": 3 } },
              "text": "y<-2" }
        ]),
    );
    // Formatting the spliced buffer proves both edits landed in order.
    assert_eq!(
        h.formatted_buffer(uri, "x<-1\nfoo\ny<-2\n"),
        "x <- 1\nfoo\ny <- 2\n"
    );
    h.shutdown();
}

#[test]
fn incremental_multi_cursor_change_reparses_and_diagnoses() {
    // A multi-cursor edit: two disjoint ranged changes in *separate* top-level
    // statements land in one `didChange`. The server records them as a precise
    // edit sequence (Stage B) and threads it to the reparse. A whole-text
    // `diff_edit` would collapse them into one span crossing the statement
    // boundary; the point here is that the end-to-end result is still correct.
    // Both edits stop `a`/`b` from being read, so the reparsed tree must yield
    // two `unused` diagnostics.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    let base = "a <- 1\nb <- 2\nprint(a)\nprint(b)\n";
    h.did_open(uri, base, 1);
    // v1 is clean (both bindings are read).
    let clean = h.recv_publish_for(uri, 1);
    assert!(clean.is_empty(), "base buffer is clean, got: {clean:#?}");

    // Replace the `a` in `print(a)` (line 2) and the `b` in `print(b)` (line 3),
    // each an equal-length splice so neither shifts the other's line offsets.
    h.did_change_raw(
        uri,
        2,
        json!([
            { "range": { "start": { "line": 2, "character": 6 },
                         "end": { "line": 2, "character": 7 } },
              "text": "9" },
            { "range": { "start": { "line": 3, "character": 6 },
                         "end": { "line": 3, "character": 7 } },
              "text": "9" }
        ]),
    );

    // Both `a` and `b` are now unused: the reparse must surface both findings.
    // (Receive the publish before any request — `recv_until` drains skipped
    // messages, so a formatting round-trip would swallow this notification.)
    // Two findings prove the reparsed tree reads `print(9)` in both statements
    // (not the original `print(a)`/`print(b)`), i.e. both edits landed correctly.
    let diags = h.recv_publish_for(uri, 2);
    assert!(
        diags.len() >= 2,
        "both unused bindings should be diagnosed after the multi-cursor edit, got: {diags:#?}"
    );
    h.shutdown();
}

#[test]
fn incremental_did_change_applies_utf16_ranged_edit() {
    // The ranged-edit tests above are pure ASCII, where a UTF-16 `character`
    // happens to equal a byte offset. Put a 2-byte and a 4-byte char before the
    // edit site so the two disagree: `x<-"é😀"` is 11 bytes but 8 UTF-16 units.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-\"\u{00e1}\u{1F600}\"\ny<-2\n", 1);
    // Line 0 in UTF-16 units: x(0) <(1) -(2) "(3) á(4) 😀(5..7) "(7).
    // Replacing 4..7 covers both wide chars, i.e. bytes 4..10.
    h.did_change_raw(
        uri,
        2,
        json!([{
            "range": { "start": { "line": 0, "character": 4 },
                       "end": { "line": 0, "character": 7 } },
            "text": "z"
        }]),
    );
    assert_eq!(
        h.formatted_buffer(uri, "x<-\"z\"\ny<-2\n"),
        "x <- \"z\"\ny <- 2\n"
    );
    h.shutdown();
}

#[test]
fn incremental_did_change_across_a_wide_char_boundary() {
    // A range starting *inside* a surrogate pair must clamp to the char's start
    // rather than splitting it — splitting would panic the buffer splice.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-\"\u{00e1}\u{1F600}\"\n", 1);
    // 😀 occupies UTF-16 units 5 and 6, so 6 is its trailing surrogate.
    h.did_change_raw(
        uri,
        2,
        json!([{
            "range": { "start": { "line": 0, "character": 6 },
                       "end": { "line": 0, "character": 7 } },
            "text": "Q"
        }]),
    );
    assert_eq!(
        h.formatted_buffer(uri, "x<-\"\u{00e1}Q\"\n"),
        "x <- \"\u{00e1}Q\"\n"
    );
    h.shutdown();
}

#[test]
fn incremental_did_change_inserting_a_wide_char_shifts_later_positions() {
    // Two changes in one batch where the first *introduces* wide chars and the
    // second addresses a UTF-16 column after them on the same line. The second
    // change resolves correctly only if the wide chars the first inserted were
    // recorded — a buffer that tracked line starts alone would land on the
    // wrong byte here.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    // The `x<-` prefix keeps the buffer un-canonical, so formatting always
    // yields an edit to read the stored text back from.
    h.did_open(uri, "x<-c(\"a\", \"bb\")\n", 1);
    h.did_change_raw(
        uri,
        2,
        json!([
            // `a` (UTF-16 6..7) becomes `á😀`: the line grows by 5 bytes but
            // only 2 UTF-16 units.
            { "range": { "start": { "line": 0, "character": 6 },
                         "end": { "line": 0, "character": 7 } },
              "text": "\u{00e1}\u{1F600}" },
            // `bb` now sits at UTF-16 13..15, which is bytes 16..18 — the two
            // differ only because the wide chars above were recorded.
            { "range": { "start": { "line": 0, "character": 13 },
                         "end": { "line": 0, "character": 15 } },
              "text": "z" }
        ]),
    );
    assert_eq!(
        h.formatted_buffer(uri, "x<-c(\"\u{00e1}\u{1F600}\", \"z\")\n"),
        "x <- c(\"\u{00e1}\u{1F600}\", \"z\")\n"
    );
    h.shutdown();
}

#[test]
fn incremental_did_change_coerces_an_inverted_range() {
    // An inverted client range used to panic the buffer splice. The main loop's
    // per-message `catch_unwind` kept the server alive, so the failure was
    // invisible — but the whole notification was abandoned, leaving the buffer
    // *and its version* behind what the client believes it sent.
    //
    // The range is coerced instead: the end collapses onto the start, making
    // this an insertion at the start offset, and the change is applied.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-1\ny<-2\n", 1);
    // Start is line 1 char 3 (byte 8, at the `2`); end is line 0 char 1 (byte 1).
    h.did_change_raw(
        uri,
        2,
        json!([{
            "range": { "start": { "line": 1, "character": 3 },
                       "end": { "line": 0, "character": 1 } },
            "text": "9"
        }]),
    );
    assert_eq!(
        h.formatted_buffer(uri, "x<-1\ny<-92\n"),
        "x <- 1\ny <- 92\n"
    );
    h.shutdown();
}

#[test]
fn full_replacement_after_ranged_edits_reseeds_the_buffer() {
    // A `range: None` change following ranged ones takes the `edits.clear()` /
    // `precise = false` path and reseeds the whole buffer.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(uri, "x<-1\n", 1);
    h.did_change_raw(
        uri,
        2,
        json!([
            { "range": { "start": { "line": 0, "character": 3 },
                         "end": { "line": 0, "character": 4 } },
              "text": "42" },
            { "text": "z<-\"\u{1F600}\"\n" }
        ]),
    );
    assert_eq!(
        h.formatted_buffer(uri, "z<-\"\u{1F600}\"\n"),
        "z <- \"\u{1F600}\"\n"
    );
    // A ranged edit against the reseeded buffer still resolves correctly.
    h.did_change_raw(
        uri,
        3,
        json!([{
            "range": { "start": { "line": 0, "character": 0 },
                       "end": { "line": 0, "character": 1 } },
            "text": "w"
        }]),
    );
    assert_eq!(
        h.formatted_buffer(uri, "w<-\"\u{1F600}\"\n"),
        "w <- \"\u{1F600}\"\n"
    );
    h.shutdown();
}

#[test]
fn rename_anchor_reanchors_across_disjoint_edits_via_precise_slice() {
    // prepareRename stashes an anchor on the function-local `value` (kept local so
    // the rename stays intra-file — a file-scope binding would need a seeded
    // workspace this hermetic harness has none of). Two *disjoint* ranged edits
    // then straddle its assignment: a comment line above and a statement below.
    // The server threads each batch into the anchor (Stage B), so `rename` folds
    // them precisely and re-anchors `value` where a coalesced whole-text
    // `diff_edit` would straddle the node interior and give up.
    //
    // To prove the *anchor* (not the request position) drove it, the rename
    // request keeps the original prepare position (line 1, char 2). The prepend
    // shifted `value` down a line, so that position now lands on the function
    // signature — a non-renameable spot, so the position fallback alone would
    // answer null. A non-null result means the accumulated slice re-anchored.
    let mut h = Harness::start_push();
    let uri = doc_uri();
    h.did_open(
        uri,
        "f <- function() {\n  value <- 1\n  print(value)\n}\n",
        1,
    );
    let _ = h.recv_publish_for(uri, 1);

    let prep_id = h.request(
        "textDocument/prepareRename",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 2 }
        }),
    );
    assert!(
        h.recv_response(&prep_id).response_result.is_ok(),
        "prepareRename offers a rename on `value`"
    );

    // Edit 1: prepend a comment line above the function.
    h.did_change_raw(
        uri,
        2,
        json!([
            { "range": { "start": { "line": 0, "character": 0 },
                         "end": { "line": 0, "character": 0 } },
              "text": "# note\n" }
        ]),
    );
    // Edit 2: append a statement at the new tail (line 5, char 0).
    h.did_change_raw(
        uri,
        3,
        json!([
            { "range": { "start": { "line": 5, "character": 0 },
                         "end": { "line": 5, "character": 0 } },
              "text": "z <- 2\n" }
        ]),
    );

    let ren_id = h.request(
        "textDocument/rename",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 2 },
            "newName": "v2"
        }),
    );
    let result = h
        .recv_response(&ren_id)
        .response_result
        .expect("rename responds");
    let changes = result
        .get("changes")
        .and_then(|c| c.get(uri))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            panic!("precise anchor should re-resolve `value` and rename it, got: {result:#?}")
        });
    assert_eq!(
        changes.len(),
        2,
        "definition + its read renamed: {changes:#?}"
    );
    for e in &changes {
        assert_eq!(e.get("newText").and_then(Value::as_str), Some("v2"));
    }
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

/// Format `text` and return the whole-document edit's end position `character`.
/// The edit range spans the *original* text, so with a multibyte char on the
/// last line the end column exposes the negotiated encoding: UTF-8 byte offsets
/// vs. UTF-16 code units.
fn format_end_character(h: &mut Harness, uri: &str, text: &str) -> i64 {
    h.did_open(uri, text, 1);
    let id = h.request(
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    let resp = h.recv_response(&id);
    let edits = resp
        .response_result
        .expect("formatting result")
        .as_array()
        .expect("array of edits")
        .clone();
    // Edits are line-scoped; these fixtures are one line, so the single edit's
    // end is the end of the document, which is what the encoding is read off.
    assert_eq!(edits.len(), 1, "one changed line is one edit: {edits:#?}");
    edits[0]
        .pointer("/range/end/character")
        .and_then(Value::as_i64)
        .expect("edit end character")
}

#[test]
fn negotiates_utf8_and_reports_byte_offsets() {
    // Client advertises UTF-8 → the server echoes it and reports byte offsets.
    let mut h = Harness::start_with_capabilities(json!({
        "textDocument": {},
        "general": { "positionEncodings": ["utf-8", "utf-16"] },
    }));
    assert_eq!(
        h.capabilities
            .pointer("/positionEncoding")
            .and_then(Value::as_str),
        Some("utf-8"),
        "server echoes the negotiated UTF-8 encoding"
    );
    // `x<-"é"` is 7 bytes (é is 2), 6 UTF-16 units. The whole-document format edit
    // ends past the last char, so its column is 7 under UTF-8.
    assert_eq!(format_end_character(&mut h, doc_uri(), "x<-\"é\""), 7);
    h.shutdown();
}

#[test]
fn defaults_to_utf16_and_reports_code_units() {
    // No `general.positionEncodings` → the server uses UTF-16, the LSP default.
    let mut h = Harness::start_push();
    assert_eq!(
        h.capabilities
            .pointer("/positionEncoding")
            .and_then(Value::as_str),
        Some("utf-16"),
        "server defaults to UTF-16 when the client offers no encodings"
    );
    // Same buffer, now measured in UTF-16 units: é is one unit, so the end
    // column is 6 rather than 7.
    assert_eq!(format_end_character(&mut h, doc_uri(), "x<-\"é\""), 6);
    h.shutdown();
}
