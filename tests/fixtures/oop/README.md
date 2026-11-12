# OOP fixture matrix

These fixtures preserve the starting point for the OOP roadmap in `TODO.md`.
They cover S3, S4, Reference Classes, R6, and released S7 0.2.2 syntax. Each
system has definitions, use sites, incomplete editor input, shadowed APIs, and
dynamic declarations. Additional cases reproduce issue #183, receiver completion
leakage, generator aliases, named class arguments, and R6 and Reference Class
method environments.

The hand-maintained `fixture_names()` in `tests/lsp_protocol/oop.rs` registers
every input. `matrix.json` declares its parse status, cursor probes, and known
gaps. Adding an input also requires entries in both places. The registration
test checks that the matrix is complete, the cursor probes are unambiguous, and
parsing preserves every byte, including incomplete input.

The protocol tests run the built CLI against each input and compare its lint
rules, severities, and spans with diagnostics published by the real LSP server.
They also request document symbols, definitions, references, completion,
signatures, and type hierarchy, including parents and children.
Method-environment probes also check highlights and rename safety for injected
bindings. Each system uses one open document that changes between cases, so the
tests exercise live edits without mixing declarations from unrelated fixtures.
Configuration and index caches live in temporary directories; package indexing
is disabled.

The five snapshots under `tests/snapshots/lsp_protocol__oop__*` record current
behavior, including the explicitly labeled gaps. They are a baseline, not a
specification of correct OOP semantics. When implementing a capability, remove
or narrow the relevant gap annotation and review the changed snapshot. Keep the
input as a regression case. Dynamic declarations whose identity cannot be
established statically should continue to return unknown results.

Run the matrix with:

```sh
compute run -- cargo test --test lsp_protocol oop::
compute run -- cargo insta review
```

The tests do not require R, R6, or S7. During the initial audit, the complete
fixtures were also executed with R 4.6.1, R6 2.6.1, and S7 0.2.2. The separate
[runtime oracle](../oop_oracle/README.md) pins observations from those versions
and checks parser, formatter, and incremental invariants on executable examples.
