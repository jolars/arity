# Hotspot classification

Read this only after the phase split identifies a hot subsystem. Re-measure all
shares on the current machine and input.

## Match the cost to the change

- Whole-tree passes that ask only token questions may use one allocation-free
  green-tree traversal. Preserve document order and all error/directive gates.
- Repeated rowan cursor walks or repeated `children_with_tokens().collect()`
  calls suggest eliminating duplicate traversal or materialization.
- `RawVec::grow_one` or `finish_grow` suggests a collection whose size can be
  bounded and reserved. Follow its callers; generic symbol type parameters can
  be merged and are not reliable attribution.
- Allocator symbols identify traffic, not a fix site. Follow callers to the
  allocation that can be removed or reduced.
- Parser tree construction is invasive because fewer nodes change CST shape.
  Treat it as a last resort, and never pool rowan's `NodeCache` across parses;
  it retains green nodes and creates a misleading warm benchmark.
- Formatter profiles on real files have historically put more time in IR
  lowering and rendering than printing. Look for repeated child collection and
  document construction along `ir_statements`, `ir_line`, binary sides, and
  assignments.
- A linter's shared walk fans out to many rules. Attribute cost to a named rule
  before changing dispatch; check `TODO.md` for recorded materializations.
- High salsa or project shares on directory lint point to query granularity and
  durability. Preserve the invalidation expectations in
  `tests/salsa_incremental.rs`.
- Roxygen shares depend heavily on documentation density. Confirm them on the
  affected package before treating them as general parser cost.
- CLI reports may be dominated by discovery, cache behavior, index setup,
  rayon, or rendering. These require the actual CLI, not merely a directory
  harness mode.

## Known traps

- The fixture corpus contains many small, roxygen-heavy files. It exaggerates
  per-file fixed costs and can make parsing dominate a formatter profile.
- A missing sub-phase often means inlining, not absence. Cross-check a different
  call site or use `INLINE=1` for drill-down, then return to normal phase output.
- Do not benchmark only a rayon directory path; spare cores can hide a
  single-file regression. Measure one file first, then the package.
- `format --check` without `--no-cache` measures fixed-point cache hits after
  warmup.
- One-shot `lint` is not the language server's persistent lint database.
- Do not replace mimalloc or optimize allocator internals before locating the
  upstream allocation.
- Do not repeat the measured no-win prefilter of `directive::parse` on an
  `arity` prefix; previous results were within timer resolution.

Record a confirmed no-win in the "Performance" section of `TODO.md` only when
that history will prevent a plausible repeated experiment.
