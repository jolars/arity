use std::io::BufReader;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use super::*;

/// A subprocess keeps each server's environment isolated from parallel tests.
struct EnvServer {
    child: Child,
    messages: Receiver<Message>,
    next_id: i32,
}

impl EnvServer {
    fn start(cwd: &Path, config: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arity"))
            .arg("lsp")
            .current_dir(cwd)
            .env("ARITY_CONFIG", config)
            .env_remove("ARITY_REMOTE_URL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn arity lsp");
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (sender, messages) = channel();
        std::thread::spawn(move || {
            while let Ok(Some(message)) = Message::read(&mut stdout) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            messages,
            next_id: 1,
        };
        server.request(
            "initialize",
            json!({
                "processId": null,
                "capabilities": {"textDocument": {"diagnostic": {}}},
            }),
        );
        server.notify("initialized", json!({}));
        server
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        Message::Request(Request::new(id.clone(), method.to_string(), params))
            .write(self.child.stdin.as_mut().unwrap())
            .expect("write request");
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("server response before timeout");
            if let Message::Response(response) = message
                && response.id == id
            {
                return response.response_result.expect("response result");
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        Message::Notification(Notification::new(method.to_string(), params))
            .write(self.child.stdin.as_mut().unwrap())
            .expect("write notification");
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {"uri": uri, "languageId": "r", "version": 1, "text": text},
            }),
        );
    }

    fn format(&mut self, uri: &str) -> Value {
        self.request(
            "textDocument/formatting",
            json!({
                "textDocument": {"uri": uri},
                "options": {"tabSize": 2, "insertSpaces": true},
            }),
        )
    }
}

impl Drop for EnvServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn env_config_formats_and_reloads_without_a_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let config = dir.path().join("user.toml");
    std::fs::write(&config, "[format]\nline-width = 30\n").unwrap();
    let uri = path_uri(&project.join("main.R"));
    let source = "x<-foo(alpha,beta,gamma,delta,epsilon,zeta,eta,theta)\n";
    let mut server = EnvServer::start(&project, &config);
    server.open(&uri, source);
    let edits = server.format(&uri);
    let narrow = apply_edits(source, edits.as_array().unwrap());
    assert!(narrow.contains("foo(\n"), "{narrow}");

    // A different length invalidates the stamp even on coarse-mtime filesystems.
    std::fs::write(&config, "[format]\nline-width = 100\n").unwrap();
    let edits = server.format(&uri);
    assert_eq!(
        apply_edits(source, edits.as_array().unwrap()),
        "x <- foo(alpha, beta, gamma, delta, epsilon, zeta, eta, theta)\n"
    );

    std::fs::write(project.join("arity.toml"), "[format]\nline-width = 30\n").unwrap();
    let edits = server.format(&uri);
    assert_eq!(apply_edits(source, edits.as_array().unwrap()), narrow);

    std::fs::remove_file(project.join("arity.toml")).unwrap();
    let edits = server.format(&uri);
    assert!(!apply_edits(source, edits.as_array().unwrap()).contains("foo(\n"));
    std::fs::remove_file(&config).unwrap();
    assert!(
        server.format(&uri).is_null(),
        "missing fallback must refuse formatting"
    );
    std::fs::write(&config, "[format]\nline-width = 30\n").unwrap();
    let edits = server.format(&uri);
    assert_eq!(apply_edits(source, edits.as_array().unwrap()), narrow);
}

#[test]
fn env_config_errors_refuse_formatting_and_recover() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let config = dir.path().join("user.toml");
    let uri = path_uri(&dir.path().join("main.R"));
    let mut server = EnvServer::start(dir.path(), &config);
    server.open(&uri, "x<-1\n");
    assert!(server.format(&uri).is_null());
    std::fs::write(&config, "[format]\nline-widht = 80\n").unwrap();
    assert!(server.format(&uri).is_null());
    std::fs::write(&config, "").unwrap();
    assert!(server.format(&uri).is_array());
}

#[test]
fn env_config_controls_lint_selection() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let config = dir.path().join("user.toml");
    std::fs::write(&config, "[lint]\nselect = [\"equals-na\"]\n").unwrap();
    let uri = path_uri(&dir.path().join("main.R"));
    let mut server = EnvServer::start(dir.path(), &config);
    server.open(&uri, "x == NA\n");
    let report = server.request(
        "textDocument/diagnostic",
        json!({"textDocument": {"uri": uri}}),
    );
    let items = report["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{report}");
    assert_eq!(items[0]["code"], "equals-na");
}
