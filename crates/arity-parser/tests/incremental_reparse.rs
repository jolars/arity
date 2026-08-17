//! Oracle property test for incremental reparse: every successful
//! `parser::reparse` must produce a tree and diagnostics byte-identical to a
//! full `parse` of the edited text. This is the correctness net behind Tenet 4.

use std::path::PathBuf;

use arity_parser::parser::{Edit, ParseDiagnostic, ReparseKind, parse, reparse, reparse_edits};
use arity_parser::syntax::SyntaxNode;

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

/// Apply an edit sequence left-to-right (the [`reparse_edits`] contract).
fn apply_all(edits: &[Edit], src: &str) -> String {
    edits.iter().fold(src.to_string(), |acc, e| e.apply(&acc))
}

/// Disjoint multi-edit sequences over `src`. Sampled insertion points are grouped
/// into windows and ordered **right-to-left** so each edit's `src`-relative
/// coordinates stay valid as its predecessors apply (a predecessor always sits to
/// its right, leaving leftward offsets untouched).
fn multi_edit_seqs(src: &str) -> Vec<Vec<Edit>> {
    let stride = (src.len() / 60).max(1);
    let inserts = ["z", " ", "\n", "x1"];
    let offsets: Vec<usize> = (0..=src.len())
        .filter(|&i| src.is_char_boundary(i))
        .enumerate()
        .filter(|(n, _)| n % stride == 0)
        .map(|(_, o)| o)
        .collect();

    let mut seqs = Vec::new();
    for win in [2usize, 3] {
        for start in (0..offsets.len().saturating_sub(win - 1)).step_by(win) {
            let group = &offsets[start..start + win];
            // Order right-to-left; pick a deterministic insert per offset.
            let seq: Vec<Edit> = group
                .iter()
                .rev()
                .enumerate()
                .map(|(k, &o)| Edit {
                    range: o..o,
                    insert: inserts[k % inserts.len()].to_string(),
                })
                .collect();
            seqs.push(seq);
        }
    }
    seqs
}

fn check_multi(src: &str) {
    let old = parse(src);
    let old_root = old.cst.clone();
    for edits in multi_edit_seqs(src) {
        let target = apply_all(&edits, src);
        let full = parse(&target);
        let Some(reparsed) = reparse_edits(&old_root, src, &old.diagnostics, &edits, &target)
        else {
            continue; // any step fell back — the caller does a full parse
        };
        assert_eq!(reparsed.kind, ReparseKind::Multi);

        let got = fingerprint(&SyntaxNode::new_root(reparsed.green.clone()));
        let want = fingerprint(&full.cst);
        assert_eq!(
            got, want,
            "reparse_edits tree mismatch for edits {edits:?} on:\n{src}",
        );
        assert_eq!(
            sorted_diags(reparsed.diagnostics),
            sorted_diags(full.diagnostics),
            "reparse_edits diagnostics mismatch for edits {edits:?} on:\n{src}",
        );
    }
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

/// The single-edit sweep driven through [`reparse_edits`] as a one-element
/// chain — the shape a keystroke actually takes once the language server stages
/// its `didChange` range. `check_source` covers the same edits through
/// `reparse`, so a divergence between the two is a bug in the chain's verify.
fn check_single_via_edits(src: &str) {
    let old = parse(src);
    let old_root = old.cst.clone();
    for edit in edits_for(src) {
        let target = edit.apply(src);
        let full = parse(&target);
        let edits = [edit.clone()];
        let Some(reparsed) = reparse_edits(&old_root, src, &old.diagnostics, &edits, &target)
        else {
            continue; // fell back to full parse — always correct
        };
        assert_eq!(reparsed.kind, ReparseKind::Multi);

        assert_eq!(
            fingerprint(&SyntaxNode::new_root(reparsed.green.clone())),
            fingerprint(&full.cst),
            "reparse_edits tree mismatch for single edit {edit:?} on:\n{src}",
        );
        assert_eq!(
            sorted_diags(reparsed.diagnostics),
            sorted_diags(full.diagnostics),
            "reparse_edits diagnostics mismatch for single edit {edit:?} on:\n{src}",
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
    "#' Title\n#' @param x A number.\n#' @examples\n#' f(1)\nf <- function(x) x\n",
    "g <- function() {\n  #' inner doc\n  #' @param y z\n  h <- 1\n  h\n}\n",
    "#' Use `x + y` and \\code{f} per [docs](u).\nf <- function(x) x\n",
    "#' See \\link[base]{sum} or [mean()] now.\nf <- function(x) x\n",
    // Flat, non-braced top-level statements: the `reparse_toplevel` path.
    "library(dplyr)\nx <- read.csv(\"f\")\nresult <- foo(a, b)\n",
    "a <- 1\nb <- a + 2\nc <- b |> sqrt()\n",
    "f <- function(a, b = 1) a + b\ng <- function(x) x\n",
    // Multi-line top-level statement continued by a trailing operator, so edits
    // near the line break exercise the merge/split boundary.
    "total <- first +\n  second\nn <- 10\n",
    // Multibyte identifiers (issue #108): the sweep edits at every char
    // boundary, so a token relex that mis-measured a name's width would show up
    // as a tree mismatch here.
    "日本語 <- 1\ncafé <- 日本語 + 2\n",
    "f <- function(café, 日本語 = 1) {\n  café + 日本語\n}\n",
];

#[test]
fn reparse_matches_full_parse_on_snippets() {
    for src in SOURCES {
        check_source(src);
    }
}

#[test]
fn reparse_edits_matches_full_parse_on_snippets() {
    for src in SOURCES {
        check_multi(src);
    }
}

#[test]
fn reparse_edits_single_edit_matches_full_parse_on_snippets() {
    for src in SOURCES {
        check_single_via_edits(src);
    }
}

#[test]
fn reparse_rejects_an_edit_the_text_cannot_take() {
    // `reparse` takes an `Edit` from its caller, so it answers "no strategy
    // applies" for a range the text cannot hold rather than panicking deep in a
    // tree lookup or a slice.
    let src = "café <- 1\n";
    let old = parse(src);
    // End past the text, start past the text, inverted, and splitting the 'é'.
    for (start, end) in [(0, 900), (900, 900), (5, 2), (3, 4)] {
        let edit = Edit {
            range: start..end,
            insert: "x".to_string(),
        };
        assert!(
            reparse(&old.cst, src, &old.diagnostics, &edit).is_none(),
            "expected no strategy for {edit:?}",
        );
    }
}

#[test]
fn reparse_edits_rejects_an_out_of_bounds_edit() {
    // A staged sequence is caller data: a coalescing gap or a misordered batch
    // can hand us a range the text cannot hold. That must be a `None` the caller
    // recovers from via `diff_edit`, never a panic in the language server's lint
    // thread.
    let src = "x <- 1\n";
    let old = parse(src);
    let edits = vec![Edit {
        range: 900..900,
        insert: "y".to_string(),
    }];
    assert!(reparse_edits(&old.cst, src, &old.diagnostics, &edits, src).is_none());
}

#[test]
fn reparse_edits_rejects_a_mid_character_range() {
    // Same contract for a range that splits a multibyte char: no such edit can
    // produce any `&str`, so the answer is `None` rather than a slice panic.
    let src = "日本語 <- 1\n";
    let old = parse(src);
    let edits = vec![Edit {
        range: 1..2,
        insert: "q".to_string(),
    }];
    assert!(reparse_edits(&old.cst, src, &old.diagnostics, &edits, src).is_none());
}

#[test]
fn reparse_edits_two_disjoint_edits_match_full_parse() {
    // Two independent identifier edits (multi-cursor rename) far apart: the
    // single-edit `diff_edit` would span from the first to the second, but the
    // precise path reparses each region.
    let src = "alpha <- 1\nbeta <- alpha + 2\ngamma <- beta\n";
    let a = src.find("alpha <- 1").unwrap();
    let b = src.rfind("beta").unwrap();
    // Right-to-left order so `src` coordinates stay valid across application.
    let edits = vec![
        Edit {
            range: b..b,
            insert: "X".to_string(),
        },
        Edit {
            range: a..a,
            insert: "Y".to_string(),
        },
    ];
    let target = apply_all(&edits, src);
    let old = parse(src);
    let full = parse(&target);
    let r = reparse_edits(&old.cst, src, &old.diagnostics, &edits, &target).expect("multi reparse");
    assert_eq!(r.kind, ReparseKind::Multi);
    assert_eq!(
        fingerprint(&SyntaxNode::new_root(r.green.clone())),
        fingerprint(&full.cst),
    );
}

#[test]
fn reparse_edits_rejects_mismatched_target() {
    // The verify-guard: correct edits but a `target` that is *not* what they
    // produce must yield `None` (caller falls back to `diff_edit`).
    let src = "x <- 1\ny <- 2\n";
    let at = src.find('1').unwrap();
    let edits = vec![Edit {
        range: at..at + 1,
        insert: "9".to_string(),
    }];
    let old = parse(src);
    assert!(reparse_edits(&old.cst, src, &old.diagnostics, &edits, "totally different").is_none());
}

#[test]
fn reparse_edits_empty_slice_is_none() {
    let src = "x <- 1\n";
    let old = parse(src);
    assert!(reparse_edits(&old.cst, src, &old.diagnostics, &[], src).is_none());
}

#[test]
fn reparse_matches_full_parse_on_large_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../arity-formatter/tests/fixtures/formatter/air_call/expected.R");
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

#[test]
fn toplevel_edit_uses_toplevel_strategy() {
    // Edit inside a bare, non-braced top-level call argument — not a single
    // token (spans a call boundary) and not inside any `{ }` block.
    let src = "library(dplyr)\nresult <- foo(a, b)\nn <- 10\n";
    let at = src.find("foo(a, b)").unwrap() + "foo(a".len();
    let edit = Edit {
        range: at..at,
        insert: ", z".to_string(),
    };
    let old = parse(src);
    let r = reparse(&old.cst, src, &old.diagnostics, &edit).expect("toplevel reparse");
    assert_eq!(r.kind, ReparseKind::TopLevel);
}

#[test]
fn toplevel_signature_edit_uses_toplevel_strategy() {
    // Edit inside a top-level function *signature* (outside the body block).
    let src = "f <- function(a, b) a + b\ng <- function(x) x\n";
    let at = src.find("function(a").unwrap() + "function(a".len();
    let edit = Edit {
        range: at..at,
        insert: ", c".to_string(),
    };
    let old = parse(src);
    let r = reparse(&old.cst, src, &old.diagnostics, &edit).expect("toplevel reparse");
    assert_eq!(r.kind, ReparseKind::TopLevel);
}

/// A trailing-operator merge: appending `+` at the end of a top-level statement
/// makes it continue onto the next line. Whatever strategy is chosen, the result
/// must match a full parse (the oracle here is an explicit spot check).
#[test]
fn toplevel_trailing_operator_merge_matches_full_parse() {
    let src = "x <- 1\ny <- 2\n";
    let at = src.find("x <- 1").unwrap() + "x <- 1".len();
    let edit = Edit {
        range: at..at,
        insert: " +".to_string(),
    };
    let old = parse(src);
    let new_text = edit.apply(src);
    let full = parse(&new_text);
    if let Some(r) = reparse(&old.cst, src, &old.diagnostics, &edit) {
        assert_eq!(
            fingerprint(&SyntaxNode::new_root(r.green.clone())),
            fingerprint(&full.cst),
            "trailing-operator merge diverged from full parse",
        );
    }
}

/// A trailing-operator split: deleting the `+` that continued a multi-line
/// statement splits it into two. Must match a full parse if reparse fires.
#[test]
fn toplevel_trailing_operator_split_matches_full_parse() {
    let src = "total <- first +\n  second\n";
    // delete the `+` (and keep the surrounding spaces intact otherwise)
    let plus = src.find('+').unwrap();
    let edit = Edit {
        range: plus..plus + 1,
        insert: String::new(),
    };
    let old = parse(src);
    let new_text = edit.apply(src);
    let full = parse(&new_text);
    if let Some(r) = reparse(&old.cst, src, &old.diagnostics, &edit) {
        assert_eq!(
            fingerprint(&SyntaxNode::new_root(r.green.clone())),
            fingerprint(&full.cst),
            "trailing-operator split diverged from full parse",
        );
    }
}
