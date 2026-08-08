---
paths:
  - "src/roxygen.rs"
  - "src/roxygen/**/*.rs"
  - "crates/arity-parser/src/parser/roxygen/**/*.rs"
  - "tests/roxygen_projector.rs"
  - "tests/roxygen_oracle.rs"
  - "tests/roxygen_lint_oracle.rs"
  - "tests/oracle/**/*"
  - "examples/rdproj.rs"
---

# Roxygen rules

Work here is split three ways, and the split is load-bearing:

- **Parsing** roxygen is the parser's job (`crates/arity-parser/src/parser/roxygen/`).
- **Formatting** roxygen is the formatter's job (`crates/arity-formatter/src/formatter/roxygen.rs`).
- **`src/roxygen/`** hosts the **CST → Rd-tree projector** (`project_rd.rs` plus
  submodules), which is *test-only*.

The full workflow for closing a parity gap is the `roxygen-parity` skill.

## The projector is a diagnostic, never a fix

- `src/roxygen/project_rd.rs` is a **test-only faithful diagnostic** behind the
  projector-parity gate. It is **not** a roxygen2 reimplementation.
- **Never patch the projector to make a case pass.** A divergence means the CST
  (or the encoding translation) is wrong — fix the parser.
- It emits only the parser-owned Rd section subtrees. Anything roxygen2 does
  that is *evaluation* rather than parsing is out of scope by construction.

## Gates

- `tests/roxygen_projector.rs` is the **CI-safe** conformance gate: pure Rust, no
  R, diffing the projector against pinned `<stem>.rdtree` files, allowlist-gated
  (`tests/oracle/roxygen-projector-allowlist.txt`). A newly-passing case is
  **ratcheted into the allowlist** so it cannot regress.
- `tests/roxygen_oracle.rs` and `tests/roxygen_lint_oracle.rs` are `#[ignore]`d
  and need R plus `roxygen2` — run via `task roxygen-oracle` /
  `task roxygen-lint-oracle`.
- The reference implementation is a local `roxygen2-ref/` checkout, pinned to
  the version in `tests/oracle/.roxygen2-source`. A fresh clone does not have
  it; clone it when you need it.

## Parser-side constraints

The sub-tokens of a `#'` line must **tile the line's bytes exactly** —
losslessness depends on it. Roxygen structure (marker, tags, arguments, prose,
Rd macros, markdown blocks) lives in the CST; it is never re-lexed downstream.

## Formatter-side constraints

Layout is chosen by a tag's `TagClass`, **never** by its written form: a body
written inline after `@details` and one written on the next line canonicalize to
the same output (Tenet 1). `tests/roxygen_format_stability.rs` pins that.
