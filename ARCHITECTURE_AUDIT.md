# Architecture audit: LSP / linter / formatter ↔ salsa ↔ rowan

*Audit date: 2026-06-09. Compares arity's incremental + language-server
architecture against rust-analyzer's and salsa's documented patterns, and
records which patterns are worth adopting.*

Arity is modeled after rust-analyzer. The headline finding is that the core is
**already closely aligned**: a single-writer salsa database owned by a dedicated
thread, snapshot-per-read on a worker pool, cancel-on-edit via salsa
cancellation, firewall queries that backdate on body edits, a lossless rowan CST
with on-demand typed AST, and a "parse never fails --- returns tree +
diagnostics" contract. This document names the genuine *gaps* relative to
rust-analyzer and recommends which to adopt. It is a reference, not a mandate;
the four gaps are tracked in `TODO.md`.

## Reference sources

- salsa book --- overview, durability, IR/structs tutorial:
  <https://salsa-rs.netlify.app/overview.html>,
  <https://salsa-rs.netlify.app/reference/durability.html>,
  <https://salsa-rs.github.io/salsa/tutorial/ir.html>
- salsa `Cancelled`: <https://docs.rs/salsa/latest/salsa/enum.Cancelled.html>
- rust-analyzer architecture & guide:
  <https://rust-analyzer.github.io/book/contributing/architecture.html>,
  <https://rust-analyzer.github.io/book/contributing/guide.html>
- rust-analyzer `main_loop.rs` / `global_state.rs`:
  <https://github.com/rust-lang/rust-analyzer/blob/master/crates/rust-analyzer/src/main_loop.rs>
- rowan incremental reparse (`reparse_token`/`reparse_block`):
  <https://rust-lang.github.io/rust-analyzer/src/syntax/parsing/reparsing.rs.html>
- durable incrementality (durability/version-vector firewall):
  <https://rust-analyzer.github.io/blog/2023/07/24/durable-incrementality.html>

--------------------------------------------------------------------------------

## 1. Current architecture (verified)

### Language server --- `src/lsp.rs`

`lsp-server` 0.7 + a `crossbeam_channel::select!` event loop (`main_loop`,
`src/lsp.rs:129`). The main loop owns **no** salsa database. A dedicated **lint
thread** (`spawn_lint_thread`, `src/lsp.rs:657`) owns the single
`IncrementalDatabase` and is its sole writer; its mutable state lives in
`LintWorker` (`src/lsp.rs:732`). Main-loop state is `GlobalState`
(`src/lsp.rs:303`) --- document buffers, cached findings keyed by
`(uri, version)`, config cache, the symbol provider `Arc`, and channel senders.

Each lint splits into:

- a cheap **write-phase** --- `prepare_document_in_project` (`&mut db`,
  `src/linter/check.rs:289`) upserts the live buffer + project siblings and
  returns owned data; runs on the lint thread.
- an expensive **read-phase** --- `analyze_prepared` (`&db` only,
  `src/linter/check.rs:351`) runs on a `rayon::spawn` worker (`src/lsp.rs:908`)
  holding a short-lived db clone, wrapped in `salsa::Cancelled::catch`.

The lint thread returns to its `select!` right after the write-phase, so a long
analyze never blocks queued reads. Read-only requests (format/hover) are
`ReadJob`s dispatched to rayon via `run_read` (`src/lsp.rs:793`, `:1040`), which
formats/hovers off the cached parse tree when the tracked buffer still matches
the live text and otherwise falls back to a fresh parse. Code actions are served
from the most recent lint's cached findings with no re-lint on a version match
(`src/lsp.rs:439`).

Scheduling: requests are **coalesced** (latest version per URI; stale edits
dropped). A `decide` scheduler (`src/lsp.rs:716`) keeps **≤1 analyze in
flight**: a strictly-newer edit of the *same* URI cancels the running analyze
via `db.trigger_cancellation()` (`src/lsp.rs:838`); a *different* pending URI
waits its turn (never cross-cancelled, so a multi-URI relint still publishes
every file). A version gate on publish backstops the finish-during-cancel race.

### Incremental layer --- `src/incremental.rs`, `src/project/graph.rs`

salsa **0.26** (`Cargo.toml:33`). Input `SourceFile { path, text }`
(`src/incremental.rs:22`); `path` is set once and never mutated, so path-keyed
queries don't re-run on a text edit. Per-file tracked queries:

- `parsed_document` (`src/incremental.rs:80`) ---
  `no_eq,   unsafe(non_update_types)`; caches `rowan::GreenNode` (`Send + Sync`,
  Arc-backed, not `Eq`); invalidation rides purely on input-text change.
- `parsed_tree_root` (`src/incremental.rs:109`) --- materializes a fresh cursor
  from the cached green (cheap atomic clone).
- `semantic_model` (`src/incremental.rs:117`).
- **Firewall queries** `file_exports` (`:130`), `file_free_reads` (`:141`),
  `source_edges` (`:156`) --- return `Eq` values (`BTreeSet<String>` /
  range-free `SourceEdgeKey`) that **backdate** when a body edit leaves them
  unchanged.

Project-level: interned `Project<'db>` (membership + namespace snapshot,
`src/project/graph.rs:44`) keys `project_graph` (`:77`, `no_eq`), which feeds
`visible_symbols(project, file)` (`:103`) returning an `Eq`/`Update`
`Visibility` (`:55`). Net effect (guarded by `tests/salsa_incremental.rs`): a
function-body edit re-runs only that file's `semantic_model`;
`file_exports`/`source_edges`/ `project_graph`/`visible_symbols` memos are all
reused.

Inputs are set via `upsert_file` (`src/incremental.rs:228`), which reuses the
existing `SourceFile` and skips the write when text is unchanged. In-memory
buffers get a synthetic path `<mem>/{uuid}.R` (`src/incremental.rs:216`).

### rowan usage

`build_tree` (`src/parser/tree_builder.rs:7`) drives a `GreenNodeBuilder` from
parser events. Typed AST is newtype `AstNode` wrappers over `SyntaxNode`
(`src/ast/nodes.rs`). **Whole-file reparse only**; no `SyntaxNodePtr`/`AstPtr`,
no block/token-level reparse. Consumers always re-resolve from a fresh cursor.

### Symbol index --- `src/rindex/`

The package symbol index (`CompositeProvider`) lives **outside** salsa entirely:
built on rayon, passed around as `Arc`, hot-swapped into the lint thread via a
channel when a background build completes.

--------------------------------------------------------------------------------

## 2. Already aligned with rust-analyzer (no action)

  | Pattern                            | rust-analyzer                                                                                   | arity                                                                                     |
  | ---------------------------------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
  | Single-writer db + snapshot reads  | `AnalysisHost::apply_change` writer; many `Analysis` snapshots                                  | lint thread sole writer; short-lived db clones for reads                                  |
  | Cancel-on-edit                     | new edit = `set_*` write → in-flight reads unwind `Cancelled::PendingWrite`; dispatcher retries | newer same-URI edit → `trigger_cancellation` → `Cancelled::catch` drops the stale analyze |
  | Debounce alternative               | latency intents + cancel/retry                                                                  | coalescing (latest version per URI) "in lieu of a debounce"                               |
  | Firewall + backdating queries      | thin derived queries whose output is stable under churn                                         | `file_exports`/`file_free_reads`/`source_edges` → `project_graph`/`visible_symbols`       |
  | Lossless CST + on-demand typed AST | rowan green nodes + zero-cost `AstNode`                                                         | identical model                                                                           |
  | Parse never fails                  | returns `(tree, Vec<SyntaxError>)`                                                              | `ParsedDocument { green, diagnostics }`                                                   |

These are correct and should **not** be churned.

--------------------------------------------------------------------------------

## 3. Gaps to adopt (tracked in `TODO.md`)

### 3.1 Type-level read/write split --- *adopted*

- **rust-analyzer:** `AnalysisHost` is the only writer (`apply_change`);
  `Analysis` is an immutable snapshot exposing only `&self` reads. The
  single-writer rule is a *compile-time* guarantee, and read handlers take
  `FilePosition`-style params, not the db.
- **arity (was):** "the lint thread is the sole writer" was only a *convention*.
  `run_read` and the analyze worker both received a full `IncrementalDatabase`
  clone and *could* have called `upsert_file` / salsa setters --- nothing in the
  types prevented it.
- **Done:** `IncrementalDatabase::snapshot()` mints a read-only newtype
  `Analysis(IncrementalDatabase)` (`src/incremental.rs`) exposing only the read
  queries (`lookup_file`, `file_text`, `file_path`, `parse_diagnostics`,
  `parsed_tree`, `semantic_model`) plus a crate-private `as_db()` for the
  read-phase salsa free functions (`intern_project`, `visible_symbols`). The
  read jobs (`run_read`) and the cross-file read-phase (`analyze_prepared`,
  `src/linter/check.rs`) now take `&Analysis`; the `&mut`-capable handle stays
  private to the lint worker. The single-writer invariant the module doc
  (`src/lsp.rs:5-14`) relies on is now a compile-time guarantee.

### 3.2 Durability + pulling the index into salsa --- *adopted*

- **rust-analyzer:** library/std `SourceRoot`s get `Durability::HIGH`, user code
  `LOW`. salsa keeps a version *vector*, so a keystroke (LOW write) skips
  revalidating the HIGH-durability library subgraph in a single integer compare.
- **arity (was):** no durability was set; all inputs defaulted to LOW. The rindex
  lived entirely outside salsa, rebuilt on rayon and hot-swapped via `Arc` through
  a channel; name resolution called the provider directly, bypassing salsa.
- **Done:** the harvested package index is a HIGH-durability salsa **singleton
  input** `LibraryIndex(Arc<IndexedProvider>)` (`src/incremental.rs`). A tracked
  `external_resolution(manifest, project, file)` query (`src/project/graph.rs`)
  resolves a file's free reads against it, returning an `Eq` `BTreeSet<String>`
  of undefined-symbol candidates. It depends only on `Eq` firewall projections
  (`file_free_reads`, the new `loaded_names`, `visible_symbols`) plus the HIGH
  manifest, so a body edit that leaves the free-read / loaded / visibility sets
  unchanged re-runs neither it nor any masking work — salsa skips the library
  subgraph via the version vector (`tests/salsa_incremental.rs`). The masking
  algorithm was extracted into free `resolve_origin`/`package_indexed` functions
  over the static base/bundled layers + the indexed layer, shared by the query,
  the `undefined-symbol` rule, and hover. The external `Arc<CompositeProvider>`
  swap pipeline is removed: the lint thread is the sole writer
  (`set_library_index`, HIGH durability), and hover reads the index from the read
  snapshot (`Analysis::library_data`). The rule re-attaches diagnostic spans (and
  re-applies the per-occurrence local-binding check) from the fresh
  `semantic_model`, so the range-free resolved set stays correct. R's
  default-package and bundled-CRAN lists stay `&'static` (compile-time constants,
  never in salsa). Single-file paths keep the `&dyn SymbolProvider` fallback.

### 3.3 FileId / VFS abstraction --- *lower leverage, removes a wart*

- **rust-analyzer:** opaque `FileId` + a `vfs` providing consistent file-system
  snapshots + `SourceRoot` grouping. Handlers convert URI → `FileId` at the
  boundary, so paths/cwd never leak into the analysis, and `SourceRoot` is the
  unit durability is assigned to.
- **arity:** keys directly on `PathBuf`; in-memory buffers use the synthetic
  `<mem>/{uuid}.R` path hack (`src/incremental.rs:216`).
- **Recommend:** a thin `FileId` (newtype over the salsa `SourceFile` id, or an
  interned path) plus a small file-source map, retiring the synthetic-path hack
  and giving project queries a clean key. Lower leverage for a single-crate R
  tool, but it removes a wart and is the right foundation for multi-root
  workspaces / `SourceRoot`-scoped durability (3.2). Files:
  `src/incremental.rs`, `src/project/graph.rs`, `src/lsp.rs` (URI↔FileId
  boundary).

### 3.4 Incremental reparse (token/block) + stable node pointers --- *highest effort, gate on benchmarks*

- **rust-analyzer:** `incremental_reparse` tries `reparse_token` (edit inside
  one token → re-lex that token), then `reparse_block` (edit inside a `{…}`
  subtree → re-parse just that block, splice the new `GreenNode`), then full
  reparse. Green nodes are immutable and structurally shared, so splicing reuses
  untouched subtrees. `SyntaxNodePtr`/`AstPtr` (offset+kind) survive reparses
  for source maps (never on *mutable* trees).
- **arity:** whole-file reparse under `parsed_document`; no stable pointers.
- **Recommend:** (a) add `reparse_token`/`reparse_block` beneath
  `parsed_document` as a perf optimization --- directly serves **Tenet 2**
  (incremental parsing is first-class) and the open "parse performance and
  incremental-reparse benchmarks" TODO. **Benchmark first**; this is where that
  data lands. (b) Add `SyntaxNodePtr`/`AstPtr` only when a feature needs a
  stable cross-edit reference --- none does today, so adopt-when-needed, don't
  build speculatively. Files: new reparse logic in `src/parser/`, hooked into
  `src/incremental.rs`.

--------------------------------------------------------------------------------

## 4. Secondary observations (no separate TODO)

- **rayon global-pool sharing.** *Resolved.* Lint analyze, reads, code actions,
  *and* heavy background package indexing used to share rayon's global pool,
  which has no priority concept --- a heavy index build could starve a
  latency-sensitive read. The LSP now uses two purpose-built `TaskPool`s
  (`src/lsp/task_pool.rs`): a read pool sized to the machine's parallelism for
  latency-sensitive work, and a single-thread index pool that isolates the one
  unbounded-duration job (background harvesting). rayon is reserved for future
  CLI data parallelism.
- **Diagnostics channel.** rust-analyzer surfaces diagnostics via
  `#[salsa::accumulator]`; arity threads them through query return values and
  the owned `PreparedProject`. The current approach is fine and explicit;
  accumulators are the idiomatic alternative if diagnostic plumbing grows.
- **Generalized cancel-and-retry.** rust-analyzer's dispatcher *retries* any
  read on cancellation against the new revision; arity's reads instead fall back
  to a fresh parse. The fallback is simpler and equally correct --- noted as a
  difference, not a gap.
