# Design: a real CommonMark inline pass for roxygen markdown

Status: **proposal** (for review before any code). Scope: the `@md` inline
grammar inside roxygen blocks --- emphasis/strong first, then links, code spans,
and the rest of the inline recognizers. Author/driver: `roxygen-parity` skill.

## 1. Goal and non-goals

**Goal.** A *complete, correct* CommonMark inline parser for roxygen `@md`
content --- not a pragmatic subset. "Correct" means: for every input, arity's
projected Rd structure matches what roxygen2 renders --- including nesting
(`**foo *bar* baz**`), the rule of 3 (`***`, `**foo*`, ...), full flanking
(whitespace **and** punctuation), intraword `_` rules, and
overlapping/mismatched delimiter runs.

**The oracle is roxygen2, not the CommonMark spec.** roxygen2 parses markdown
via `cmark`/`cmark-gfm` (so its *parsing* is faithful CommonMark), but the
content is always processed *through roxygen2*, which adds behaviors `cmark`
alone does not: a markdown-escaping pre-pass, the `rdComplete` brace/quote
**validation** (`tag-parser.R` →
`warn_roxy_tag("has mismatched braces or quotes")`), and a *subset* translation
to Rd (only constructs with an Rd analog). So **roxygen2's behavior is truth
wherever it diverges from raw `cmark`** --- both what it renders *and* what it
rejects. We never reason "CommonMark says X, so arity does X"; only "roxygen2
does Y, so arity does Y." The CommonMark spec is used **only as an input
corpus** (a broad supply of emphasis shapes); the expected output and the
pass/fail verdict always come from running roxygen2.

**Two oracle surfaces, not one.** (1) *Render parity* --- the projected Rd
matches (the existing projector gate). (2) *Diagnostic parity* --- roxygen2 also
**validates** and emits source-located warnings, then drops the offending
content (e.g. `\*not emphasis\*` →
`✖ <text>:3: @description has mismatched braces or quotes` + an empty
`\description{}`). arity should detect the *same condition* and emit a
**side-channel parse/lint diagnostic** (the CST stays lossless), which is
exactly the lint + LSP signal we want. An oracle-*error* input is therefore a
**diagnostic-parity fixture**, never a silently-skipped `blocked` case. Building
the diagnostics is aligned with the (deferred) linter/LSP phases; this slice at
minimum *records* each oracle-error condition so it is not mistaken for a render
gap.

**Non-goals (this design).** - Block structure changes. The existing block model
(`ROXYGEN_SECTION` / `ROXYGEN_PARAGRAPH` / lists / fenced code / HTML blocks)
stays. This is an *inline* pass that runs *within* a paragraph's text. -
Re-deriving `@md` mode anywhere new. Mode resolution stays where it is (lexer
`resolve_roxygen_block`, baked into leaf kinds; projector/formatter each
re-derive once, as today). - R evaluation (`` `r ...` ``, ```` ```{r} ````
knitr) --- out of scope, static parser.

## 2. Why the current shape cannot do this

Today emphasis is recognized **in the lexer, per line, as a local forward scan**
(`scan_md_emphasis` in `src/parser/roxygen/lex.rs`). It emits the *entire*
`*...*`/`**...**` span as a **single atomic token** (`ROXYGEN_MD_EMPH` /
`ROXYGEN_MD_STRONG`). The projector then just strips the outer delimiters and
treats the inside as flat `TEXT` (`push_inline` → `strip_delim`).

Three structural consequences, none fixable by tweaking the scanner's
heuristics:

1. **Nesting is unrepresentable.** The span's content is never re-tokenized, so
   `**foo *bar* baz**` cannot carry an inner `\emph`. The doc comment's own
   "faithful under-recognition" framing is a symptom: the function *bails to
   literal text* whenever it can't model a shape, because it has no way to model
   nesting, the rule of 3, or overlap.
2. **Matching is a forward "find the next closer" scan**, not CommonMark's
   delimiter-stack with backward matching. The rule of 3 and overlapping runs
   (`*foo **bar* baz**`) need the stack; a forward scan gets them wrong.
3. **Flanking ignores punctuation** (only whitespace / alphanumeric for `_`),
   and it is line-scoped, so emphasis can't span a soft line break.

The function's *shape* --- local, atomic, forward-scan, in the lexer --- is
wrong. CommonMark inline parsing is **non-local** (whole-block) and **two-pass
within a block** (delimiter scan → `process_emphasis`). We need that shape.

## 3. The algorithm we must implement (CommonMark §6)

Over a block's joined inline text:

1. **Tokenize inlines left→right** into a node list, pushing each
   `*`/`_`/`[`/`]` **delimiter run** onto a **delimiter stack** entry recording:
   delimiter char, run length, and the `can_open` / `can_close` flags.
2. **Flanking** (the flags' basis), per spec --- a *Unicode*
   whitespace/punctuation classification of the char immediately before and
   after the run (start/end of block count as whitespace):
   - **left-flanking**: not followed by whitespace, and (not followed by
     punctuation, or preceded by whitespace/punctuation).
   - **right-flanking**: not preceded by whitespace, and (not preceded by
     punctuation, or followed by whitespace/punctuation).
   - `*` can_open = left-flanking; can_close = right-flanking.
   - `_` can_open = left-flanking and (not right-flanking **or** preceded by
     punctuation); can_close = right-flanking and (not left-flanking **or**
     followed by punctuation). (This is the intraword-`_` rule, done right.)
3. **`process_emphasis`**: walk the stack; for each closer find the nearest
   matching opener below it; apply the **rule of 3** (if either run can both
   open and close, a match whose `open_len + close_len` is a multiple of 3 is
   forbidden unless *both* lengths are multiples of 3); consume **2** delimiters
   for strong, else **1** for emphasis; wrap the enclosed nodes; **remove**
   delimiters between the matched pair; leftover delimiters become literal text.

For arity we only need the `*`/`_` half first; `[`/`]` (links) is the *same
stack* and lands in the migration's second step.

## 4. Architecture: where the inline pass lives

The pass is part of `parse()` (so salsa's text→tokens→events→CST pipeline and
the incremental reparse path are unaffected --- incremental works off the
green-tree diff regardless of how events are produced). Concretely it sits in
the **grouper / builder** phase (`src/parser/roxygen/group.rs` + `build.rs`), at
**paragraph granularity**, not line granularity.

Today `emit_roxygen_block` opens a `ROXYGEN_PARAGRAPH` and streams each prose
line through `emit_prose_line` (one `Event::Tok` per line-body token). That
per-line streaming is the line-scoping. The new shape:

- While a paragraph is open, **collect its inline content** across lines into a
  logical stream: the content tokens plus the inter-line **trivia** (newline +
  next `#'` marker + leading whitespace) recorded at their positions. A soft
  line break is a single whitespace for flanking purposes (faithful: roxygen2
  joins lines, and with `hardbreaks = TRUE` a *hard* break is a trailing
  ``  ``/`\\`, handled separately and already modeled as a distinct node).
- At paragraph close, **run the inline pass** over that stream and **emit
  events**: text leaves, raw-delimiter leaves (unmatched), and `ROXYGEN_MD_EMPH`
  / `ROXYGEN_MD_STRONG` **nodes** wrapping their resolved children. Interleaved
  trivia is emitted at its original byte position --- *inside* a node when the
  span crosses a line break, at paragraph level otherwise.

This naturally subsumes **cross-line emphasis** (`**foo\nbar**`) and, once links
move into the pass, **cross-line links** --- the two "architecturally invasive"
items collapse into this one pass.

## 5. Token model change (lexer)

Under `@md`, the lexer **stops carving emphasis spans**. At a `*`/`_` it emits a
**raw delimiter-run token** (the maximal run, e.g. `*`, `**`, `***`) plus normal
text --- neutral leaves, no open/close decision. New `TokKind`: `RoxygenMdDelim`
(one kind; the run length and char are in the token text).

- Losslessness is trivially preserved (the delimiters are still text bytes; we
  just don't pre-group them).
- `RoxygenMdEmph` / `RoxygenMdStrong` **token** kinds are retired from the
  lexer. The *node* kinds `ROXYGEN_MD_EMPH` / `ROXYGEN_MD_STRONG` (already
  SyntaxKinds 90/91) are **reused as node kinds** --- a clean reuse, since
  nothing else holds them and the projector already classifies by `el.kind()`.
- Mode gating: `RoxygenMdDelim` is emitted only under `@md` (as today). Without
  `@md`, `*`/`_` stay literal prose text.

Every classifier that must learn the new kind (the trap's checklist):
`TokKind::roxygen_role`, `expr.rs` atom fallthrough,
`tree_builder::syntax_kind_for`, `syntax.rs` `is_roxygen_token` +
`is_roxygen_prose_content`, `kind_from_raw` + `COUNT`. A new leaf kind for the
raw delimiter (`ROXYGEN_MD_DELIM`) is appended after `ROXYGEN_MD_HTML_BLOCK`,
`COUNT` bumped.

## 6. Node shapes and losslessness

A resolved emphasis is a **node**:

```
ROXYGEN_MD_EMPH
  ROXYGEN_MD_DELIM "*"        (opener; the matched portion)
  …children…                  (text leaves, nested EMPH/STRONG nodes, trivia)
  ROXYGEN_MD_DELIM "*"        (closer)
```

- **Matched** delimiters become the node's opener/closer delimiter leaves. If a
  run is only **partially** consumed (e.g. `***foo**` → strong consumes 2 of 3,
  leaving 1 as literal `*`), the leftover stays a sibling `ROXYGEN_MD_DELIM`
  *outside* the node --- same byte, different parent. The emitted leaf texts
  must tile the original run exactly (`Event::Leaf` splits the run; this is
  exactly the mechanism the multi-line `\itemize` builder already uses).
- **Unmatched** delimiters and all text stay as leaves at paragraph level.
- **Cross-line** spans: the inter-line trivia (newline / `#'` marker /
  whitespace) lands *inside* the node as child trivia leaves. `reconstruct`
  concatenates all leaves in order ⇒ losslessness holds by construction. (This
  is the same marker-inside-an-inline-node situation flagged for cross-line
  links; the design accepts it deliberately and §8 handles the formatter
  consequence.)

## 7. Projector changes

`push_inline` / `paragraph_inlines` already walk `NodeOrToken` and classify by
kind. Changes:

- `ROXYGEN_MD_EMPH` / `ROXYGEN_MD_STRONG` are now **nodes**: recurse into their
  children (minus the delimiter leaves and trivia) to build the inner inline
  run, then wrap in `(\emph ...)` / `(\strong ...)`. This is where nesting
  finally projects: the inner `*bar*` is a child node, not flattened text.
- `Inline::Md(MdInline::Emph/Strong, String)` becomes a recursive variant
  carrying a child `Vec<Inline>` (or the node, walked lazily) instead of a flat
  `String`. `serialize_md_inline` recurses.
- `ROXYGEN_MD_DELIM` leaves (unmatched) project as literal text of their bytes.
- Whitespace normalization (`text_atom` / `norm_ws`) is unchanged and still
  applies to the inner text runs.

## 8. Formatter changes and idempotence

The formatter reflows prose, keeping certain nodes atomic (`physical_lines`
keeps `ROXYGEN_TAG` / `ROXYGEN_RD_MACRO` atomic). Plan:

- Treat `ROXYGEN_MD_EMPH` / `ROXYGEN_MD_STRONG` nodes as **atomic reflow
  chunks** (their full source text emitted as one unit), matching today's
  behavior where the whole `*...*` token is already atomic. No wrapping inside a
  span --- same as now.
- **Cross-line emphasis** (a node containing `#'` markers): like the cross-line
  list/HTML-block cases, emit as **marker-preserving atomic passthrough** rather
  than reflowing across the buried markers. Reuse the existing
  atomic-passthrough path (the one `physical_lines` already has for block macros
  / lists / fenced code), keyed on "node contains a `ROXYGEN_MARKER`".

**Idempotence argument.** Reflow normalizes runs of whitespace to single spaces;
CommonMark flanking depends only on whitespace-vs-not and punctuation-vs-not,
not on whitespace *count*. So single-space normalization preserves every run's
flanking class, and reparse re-resolves the *same* emphasis structure ⇒
`format(format(x)) == format(x)`. The one place to **verify** (not assumed): a
span pushed to a line start after `#'` --- the preceding char is still
whitespace (the marker space), so flanking is unchanged. Covered by an
idempotence fixture per tricky case.

## 9. Migration order

1. **Emphasis/strong only** (this design's first PR): lexer emits
   `RoxygenMdDelim`; new inline pass resolves `*`/`_` via the delimiter stack
   into `ROXYGEN_MD_EMPH` / `ROXYGEN_MD_STRONG` nodes; projector recurses;
   formatter treats nodes atomic. Links/code/images **stay as today's local span
   tokens**, resolved *before* the delimiter pass sees them (a code span / link
   is opaque to emphasis flanking, exactly as CommonMark's precedence: code
   spans, autolinks, and raw HTML bind tighter than emphasis). The pass simply
   treats an existing `ROXYGEN_MD_CODE` / `ROXYGEN_MD_LINK` / ... leaf as a
   single opaque inline.
2. **Links into the pass**: move `[`/`]` onto the same stack (CommonMark
   `look_for_link_or_image`), which also yields **cross-line links** for free.
3. **Code spans, autolinks, raw HTML, images** folded in as the pass matures, so
   the lexer's local recognizers retire one by one.

Each step is independently shippable, TDD'd, and ratcheted in the projector
allowlist.

## 10. Testing plan

The **oracle is roxygen2** (§1). The CommonMark spec test set is used **only as
a broad input corpus** --- never as the expected output. panache compares
parser→HTML against the spec's `expected_html`; we do **not** --- we throw the
spec's markdown **inputs** at roxygen2 and pin *whatever roxygen2 does* (render
or diagnostic). The spec examples become a third **corpus source** for the
existing projector-parity gate (`tests/roxygen_projector.rs`), alongside the
curated dir corpus and the harvested corpus:

- **Vendor** the CommonMark `spec.txt` (e.g.
  `tests/oracle/corpus/commonmark-spec/`, refresh via a script; mirror panache's
  "do not edit directly"). A small loader parses it into
  `{number, section, markdown}` (we ignore `expected_html`).
- **Scope per slice.** Slice 1 = the **"Emphasis and strong emphasis"** section
  (\~132 examples) --- the authoritative, exhaustive emphasis driver. Later
  slices pull in their sections (links, code spans, ...) as the pass grows.
- **Mint Rd pins from roxygen2 once.** Wrap each example's markdown into a valid
  `@md` roxygen block on an object, run `block-to-sections`, pin the result (no
  R at test time --- same as the harvested pins). The pure-Rust gate diffs
  `project_to_rd` against the pin.
- **Allowlist / backlog / diagnostic-parity** (three outcomes, not a vague
  "blocked"):
  - *Render parity* --- roxygen2 renders Rd; arity's projection matches →
    allowlist (or is backlog until the parser closes it). This is the bulk of
    the emphasis section (verified: roxygen2 renders nesting, the rule of 3,
    intraword, and flanking all faithfully).
  - *Diagnostic parity* --- roxygen2 **errors/warns and drops content**
    (`rdComplete` → `warn_roxy_tag`, e.g. `\*not emphasis\*`). This is **not**
    `blocked`: it is a diagnostic-parity case. Until arity emits the matching
    side-channel diagnostic (linter/LSP phase), record it with its exact oracle
    message so it is never mistaken for a render gap.
  - *Genuinely out of scope* --- an input pulling in a construct with no Rd
    analog or no arity model yet → `blocked` *with a reason* (never to silence a
    regression). For the emphasis section this bucket is expected to be small.

Complementary layers (unchanged in spirit):

- **Curated fixtures** (`tests/fixtures/parser/roxygen_md_emphasis_*`): the
  user's "Complex Cases" list --- rule-of-3, nesting, overlap, intraword,
  flanking, escapes --- as readable, reviewed CST snapshots asserting
  losslessness. (The spec corpus is the *breadth* net; these are the *legible*
  pins.)
- **Idempotence**: a formatter fixture per cross-line and per
  punctuation-adjacent case (the §8 verification).
- Guardrails: `cargo test` (incl. projector gate, no R), clippy, fmt;
  `task   roxygen-oracle` (R fixed-point) where available.

**Why this beats panache's exact model here:** the oracle *is* the tool we
conform to (roxygen2), so there is no separate HTML renderer to build or
maintain, and we capture roxygen2's *actual* behavior --- including its escaping
quirks and its `rdComplete` diagnostics --- rather than an idealized
`cmark`→HTML the user will never see. The spec is a generous source of emphasis
*inputs*; roxygen2 supplies every *answer*.

## 11. Risks and open questions

- **Scope of the rewrite.** The paragraph-level buffering changes how
  `emit_roxygen_block` emits prose. It must stay byte-identical for *non-`@md`*
  blocks and for `@md` blocks with no delimiters (regression-guarded by the full
  parser snapshot suite).
- **Trivia-inside-node** for cross-line spans is the part most likely to
  surprise the formatter; §8's atomic-passthrough keying is the mitigation, but
  cross-line emphasis fixtures must prove idempotence explicitly.
- **Unicode punctuation/whitespace classification.** CommonMark uses Unicode
  categories. roxygen content is ASCII-dominant; decide whether to ship
  ASCII-class first (and document the gap) or pull in a Unicode property table.
  Proposal: ASCII classification first, with a noted backlog item, since
  divergence requires non-ASCII punctuation *adjacent to a delimiter* --- rare
  in roxygen.
- **`RoxygenMdEmph`/`RoxygenMdStrong` token retirement** touches the format-
  stability baseline (an atomic token becomes a node). Expect to re-bless the
  affected baseline rows *with review* (the trap's discipline).

## 12. Bottom line

This is a foundational refactor (introduce the inline pass; emphasis first), not
a patch. It is the correct architecture, it makes nesting and the rule of 3
*representable* (today they are not), and it is the same machinery that robust
links need --- so it pays off twice. Reviewer's call: approve the emphasis-first
slice (§9.1), or adjust scope (e.g. ASCII-vs-Unicode flanking, or do links in
the same first slice).
