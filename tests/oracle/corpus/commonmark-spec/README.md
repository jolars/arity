# Vendored CommonMark spec

`spec.txt` is the upstream [CommonMark](https://spec.commonmark.org/) spec test
set, vendored verbatim as a **broad input corpus** for the roxygen projector-
parity gate. **Do not edit it by hand** --- refresh it from upstream.

- Source:
  <https://raw.githubusercontent.com/commonmark/commonmark-spec/master/spec.txt>
- Version: 0.31.2

We use only the spec's markdown **inputs**, never its `expected_html`: arity's
target is the Rd that **roxygen2** renders, so roxygen2 is the oracle (it parses
markdown via `cmark`/`cmark-gfm`, then translates a subset to Rd and validates).
`scripts/build-commonmark-corpus.R` extracts a section's examples, wraps each
into an `@md` roxygen block, and emits `{slug, input}` JSONL keyed by the spec's
canonical global example number (`cm-<NNN>`). The Rd pins are minted separately
by the `projector-pins` op (same as the harvested corpus). See
`docs/design/roxygen-inline-pass.md` §10.
