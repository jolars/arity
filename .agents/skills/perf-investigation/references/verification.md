# Verification and handoff

Read this before handing off or committing a performance change.

## Required checks

```sh
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Run the focused behavioral suites relevant to the changed area:

```sh
cargo test -p arity-formatter --test formatter
cargo test -p arity-parser --test parser_snapshots
cargo test -p arity-parser --test incremental_reparse
cargo test --test salsa_incremental
cargo test --test lint
cargo test --test roxygen_projector
```

For parser or formatter changes, round-trip representative real code with
`task corpus`, `format --verify`, and `parse --verify --quiet` as appropriate.
For formatter changes, compare baseline and candidate output trees byte for byte.

Never accept an unread insta snapshot. A changed snapshot is a correctness
failure by default in performance work; explain any intentional exception
before accepting it.

## Measurement record

State the production path, input, core pinning, warmups, run count, median,
minimum, and percentage change. Use at least 20 timed runs. Include measurements
for experiments that were removed because they did not beat noise.

Keep one measured optimization per commit, and follow the atomic path areas in
the `AGENTS.md` release section. Do not commit unless the user requested it.

If the improvement materially moves the published performance page and updating
it is in scope, run `task bench` and commit its tracked JSON artifact. Do not edit
generated benchmark documentation directly.
