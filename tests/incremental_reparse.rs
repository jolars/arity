//! Oracle property test for incremental reparse: every successful
//! `parser::reparse` must produce a tree and diagnostics byte-identical to a
//! full `parse` of the edited text. This is the correctness net behind Tenet 4.

use std::path::PathBuf;

use ravel::parser::{Edit, ParseDiagnostic, ReparseKind, parse, reparse};
use ravel::syntax::SyntaxNode;

/// A complete structural fingerprint of a tree: every node/token with its kind,
/// range, and (for tokens) text. Two trees with equal fingerprints are
/// identical.
fn fingerprint(node: &SyntaxNode) -> String {
    let mut out = String::new();
    for el in node.descendants_with_tokens() {
        let text = el
            .as_token()
            .map(|t| t.text().to_string())
            .unwrap_or_default();
        out.push_str(&format!(
            "{:?}@{:?} {:?}\n",
            el.kind(),
            el.text_range(),
            text
        ));
    }
    out
}

fn sorted_diags(mut diags: Vec<ParseDiagnostic>) -> Vec<(usize, usize, String)> {
    diags.sort_by(|a, b| (a.start, a.end, &a.message).cmp(&(b.start, b.end, &b.message)));
    diags
        .into_iter()
        .map(|d| (d.start, d.end, d.message))
        .collect()
}

/// Sampled single-character edits over `src`. For small inputs every char
/// boundary and every edit kind; for large inputs a stride keeps it bounded.
fn edits_for(src: &str) -> Vec<Edit> {
    // Keep ~200 sample offsets regardless of size so the large-fixture sweep
    // stays fast; small snippets hit every boundary.
    let stride = (src.len() / 200).max(1);
    let inserts = ["z", " ", "\n", "#", "1", "\""];
    let mut edits = Vec::new();
    let boundaries: Vec<usize> = (0..=src.len())
        .filter(|&i| src.is_char_boundary(i))
        .collect();
    for (n, &o) in boundaries.iter().enumerate() {
        if n % stride != 0 {
            continue;
        }
        for ins in inserts {
            edits.push(Edit {
                range: o..o,
                insert: ins.to_string(),
            });
        }
        // delete / replace the char starting at o
        if o < src.len() {
            let next = (o + 1..=src.len())
                .find(|&i| src.is_char_boundary(i))
                .unwrap();
            edits.push(Edit {
                range: o..next,
                insert: String::new(),
            });
            edits.push(Edit {
                range: o..next,
                insert: "q".to_string(),
            });
        }
    }
    edits
}

fn check_source(src: &str) {
    let old = parse(src);
    let old_root = old.cst.clone();
    for edit in edits_for(src) {
        let new_text = edit.apply(src);
        let full = parse(&new_text);
        let Some(reparsed) = reparse(&old_root, src, &old.diagnostics, &edit) else {
            continue; // fell back to full parse — always correct
        };

        let got = fingerprint(&SyntaxNode::new_root(reparsed.green.clone()));
        let want = fingerprint(&full.cst);
        assert_eq!(
            got, want,
            "reparse ({:?}) tree mismatch for edit {:?} on:\n{}",
            reparsed.kind, edit, src
        );
        assert_eq!(
            sorted_diags(reparsed.diagnostics),
            sorted_diags(full.diagnostics),
            "reparse ({:?}) diagnostics mismatch for edit {:?} on:\n{}",
            reparsed.kind,
            edit,
            src
        );
    }
}

const SOURCES: &[&str] = &[
    "x <- 1 + 2\n",
    "foo <- function(a, b) {\n  a + b\n}\n",
    "if (x > 0) {\n  print(\"hello world\")\n} else {\n  y\n}\n",
    "# a comment\nresult <- compute(data, n = 10) # trailing\n",
    "s <- \"a string with spaces\"\nv <- c(1, 2, 3)\n",
    "g <- function() {\n  for (i in 1:10) {\n    cat(i)\n  }\n}\n",
    "nested <- {\n  a <- {\n    b + c\n  }\n  a\n}\n",
    "df[[\"col\"]] <- xs[idx]\n",
    "pkg::fn(arg) |> transform()\n",
];

#[test]
fn reparse_matches_full_parse_on_snippets() {
    for src in SOURCES {
        check_source(src);
    }
}

#[test]
fn reparse_matches_full_parse_on_large_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/formatter/air_call/expected.R");
    let Ok(src) = std::fs::read_to_string(&path) else {
        return; // fixture optional
    };
    check_source(&src);
}

#[test]
fn token_edit_uses_token_strategy() {
    let src = "alpha <- beta + gamma\n";
    // insert inside the identifier `beta` (byte offset of the 'e' in beta)
    let at = src.find("beta").unwrap() + 1;
    let edit = Edit {
        range: at..at,
        insert: "X".to_string(),
    };
    let old = parse(src);
    let r = reparse(&old.cst, src, &old.diagnostics, &edit).expect("token reparse");
    assert_eq!(r.kind, ReparseKind::Token);
}

#[test]
fn body_edit_uses_block_strategy() {
    let src = "f <- function() {\n  a + b\n}\n";
    // insert a new char between `a` and ` + b` inside the function body block
    let at = src.find("a + b").unwrap() + 1;
    let edit = Edit {
        range: at..at,
        insert: "c".to_string(),
    };
    let old = parse(src);
    let r = reparse(&old.cst, src, &old.diagnostics, &edit).expect("block reparse");
    assert_eq!(r.kind, ReparseKind::Block);
}
