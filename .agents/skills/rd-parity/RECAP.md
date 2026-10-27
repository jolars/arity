# Rd parity recap

Keep the measured status and next investigation here; keep durable workflow
instructions in `SKILL.md`.

## Initial baseline: 2026-10-27

- Reference: R 4.6.1 (2026-06-24), recorded in the oracle pins.
- All 13 registered help documents match the pinned and live R trees. The corpus
  includes generated help pages from cli and rlang, with provenance in the
  fixture README.
- All 18 bundled system-macro argument shapes match R's definitions without
  expansion.
- Recovery, losslessness, typed navigation, custom macro preservation, and
  file/stdin CLI selection have regression tests. Workspace checks passed at the
  initial implementation.

These counts describe the curated corpus. Broader R and CRAN coverage has not
been measured, and no confirmed open grammar gap is recorded yet.

## Next investigation

Prefer a user-reported discrepancy. Otherwise inspect a small set of real help
files beyond the two existing package examples, looking for mode transitions
around quoted R-like text, comments, nested conditionals, and macro options.
Confirm any suspected gap with R and reduce it to a fixture before expanding the
parser. Record actual discoveries here rather than treating those candidate
areas as known bugs.

Unexpanded custom macros, unevaluated `\Sexpr`, and absent formatting or editor
integration are the agreed scope, not an oracle backlog.
