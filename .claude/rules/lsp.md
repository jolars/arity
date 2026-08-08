---
paths:
  - "src/lsp.rs"
  - "src/lsp/**/*.rs"
---

# Language server rules

`src/lsp.rs` is a facade over `src/lsp/*`; the module doc carries the full
threading rationale — **read it before touching the main loop**.

## Transport

`lsp-server` (rust-analyzer's transport), stdio JSON-RPC, synchronous main loop.
Not tower-lsp: salsa cancellation is a synchronous unwind (`salsa::Cancelled`)
that composes with a sync loop plus thread pools and fights an async `&self`
model.

## Threading

- **The main loop owns no salsa database.** A dedicated **lint thread** owns the
  persistent `IncrementalDatabase` and is the sole *writer*. This is forced:
  salsa is strictly single-writer (a `set_*` setter blocks until every other db
  handle drops), and cross-file lint *writes* sibling files into the db — so
  lint cannot run on a shared read snapshot.
- Every lint splits into a cheap **write-phase** (`prepare_document_in_project`,
  `&mut db`, on the lint thread) and an expensive **read-phase**
  (`analyze_prepared`, `&db`, on the read pool). Keep that split: a long analyze
  must never block queued reads.
- Two purpose-built `TaskPool`s, not rayon's global pool (which has no priority
  concept): a **read pool** for latency-sensitive work, and a **single-thread
  index pool** isolating the one unbounded-duration job (background package
  indexing) so a long harvest can never slot-block a read. **Never put unbounded
  work on the read pool.**
- Requests are **coalesced** (latest version per URI wins) in lieu of a debounce;
  `decide` keeps at most one analyze in flight. A newer edit of the *same* URI
  cancels the running analyze; a *different* pending URI waits its turn and is
  never cross-canceled, so a multi-URI `RelintAll` still publishes every file.
- A read job holding a db clone when the lint thread writes trips
  `salsa::Cancelled`; that and a cache miss both fall back to a fresh parse.
  **Reads are always correct, only sometimes warm** — never trade that away.

## Paths

Convert URIs only through `src/lsp/uri.rs` (`to_path`/`from_path`), which strips
the `/` before a Windows drive letter and keeps the Unix root. Tests and
snapshots must not assume `/` versus `\`.

## Workspace awareness

- On-disk change detection is via dynamically-registered
  `workspace/didChangeWatchedFiles` watchers (`arity.toml`, `DESCRIPTION`,
  `NAMESPACE`, `.R` — see `watched_files.rs`) plus
  `workspace/didChangeWorkspaceFolders`.
- The LSP resolves `arity.toml` the same way the CLI does, so both walks honor
  the same excludes. A new setting that is a fact about the *machine* belongs in
  editor settings (`settings.rs`), not `arity.toml`; project facts belong in the
  config file.
- Rename covers symbols **and** files and folders (`file_rename.rs`); a folder
  rename fans out over workspace membership, and renames leaving the workspace
  scope are dropped.

## Testing

`tests/lsp.rs` (behavior) and `tests/lsp_protocol.rs` (wire level).
`tests/salsa_incremental.rs` guards that a body edit does not invalidate the
project graph — a regression there shows up here as latency, so keep it green.
