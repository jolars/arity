#!/usr/bin/env bash
#
# Benchmark arity's formatter speed against `air` (posit-dev/air) on large
# inputs, using hyperfine. Mirrors `task air-compat` in spirit: an opt-in,
# local-only measurement that regenerates a tracked report (BENCH.md).
#
# This is a *visibility* tool, not a quality gate and not an air-parity target.
# It measures wall-clock formatting speed only, never output equivalence (that
# is what `task air-compat` covers).
#
# Usage:
#   ./scripts/bench-format.sh           # synthetic corpus (two size tiers)
#   ARITY_BENCH_INPUT=path/to/file.R ./scripts/bench-format.sh
#                                       # benchmark a real file instead
#
set -euo pipefail

# Resolve repo root from this script's location so it works from any cwd.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ARITY="$REPO_ROOT/target/release/arity"
OUT="$REPO_ROOT/BENCH.md"

# Repetition counts that build the two synthetic size tiers from the base block.
SMALL_REPS=2
LARGE_REPS=24

require() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: '$1' not found on PATH" >&2
    exit 1
  }
}
require hyperfine
require air

echo ">> Building release binary..."
cargo build --release

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --- Build the corpus tiers -------------------------------------------------

declare -a TIERS=()        # human label per tier
declare -a TIER_FILES=()   # corpus file per tier
declare -a TIER_LINES=()   # line count per tier

if [[ -n "${ARITY_BENCH_INPUT:-}" ]]; then
  if [[ ! -f "$ARITY_BENCH_INPUT" ]]; then
    echo "error: ARITY_BENCH_INPUT='$ARITY_BENCH_INPUT' is not a file" >&2
    exit 1
  fi
  echo ">> Using override input: $ARITY_BENCH_INPUT"
  TIERS+=("override")
  TIER_FILES+=("$ARITY_BENCH_INPUT")
  TIER_LINES+=("$(wc -l <"$ARITY_BENCH_INPUT")")
else
  echo ">> Generating synthetic corpus from formatter fixtures..."
  # One deterministic base block: every formatter fixture's expected.R, in
  # sorted order, blank-line separated. These are guaranteed arity-parseable
  # and arity-clean, so `arity format` never errors on them.
  BASE="$TMP/base.R"
  : >"$BASE"
  found=0
  while IFS= read -r f; do
    cat "$f" >>"$BASE"
    printf '\n\n' >>"$BASE"
    found=1
  done < <(find tests/fixtures/formatter -name expected.R | sort)
  if [[ "$found" -eq 0 ]]; then
    echo "error: no tests/fixtures/formatter/*/expected.R files found" >&2
    exit 1
  fi

  # NOTE: tiers repeat the same base block, so content is cache-friendly and
  # not fully representative of real-world code. They exist to amortize process
  # startup and show rough scaling, not to model a real workload.
  for tier in "small:$SMALL_REPS" "large:$LARGE_REPS"; do
    label="${tier%%:*}"
    reps="${tier##*:}"
    corpus="$TMP/corpus_${label}.R"
    : >"$corpus"
    for ((i = 0; i < reps; i++)); do cat "$BASE" >>"$corpus"; done
    TIERS+=("$label")
    TIER_FILES+=("$corpus")
    TIER_LINES+=("$(wc -l <"$corpus")")
  done
fi

# --- Sanity gate: arity must format the largest corpus without error --------

LAST_IDX=$((${#TIER_FILES[@]} - 1))
echo ">> Sanity check: arity format on '${TIERS[$LAST_IDX]}' corpus..."
if ! "$ARITY" format <"${TIER_FILES[$LAST_IDX]}" >/dev/null; then
  echo "error: arity failed to format the corpus (parse diagnostics?)" >&2
  exit 1
fi

# --- Run hyperfine per tier -------------------------------------------------

declare -a RESULT_MD=()
for idx in "${!TIERS[@]}"; do
  label="${TIERS[$idx]}"
  corpus="${TIER_FILES[$idx]}"
  md="$TMP/result_${label}.md"
  echo
  echo ">> Benchmarking '$label' tier (${TIER_LINES[$idx]} lines)..."
  hyperfine --warmup 3 \
    --command-name arity "$ARITY format < '$corpus' > /dev/null" \
    --command-name air "air format --stdin-file-path bench.R < '$corpus' > /dev/null" \
    --export-markdown "$md"
  RESULT_MD+=("$md")
done

# --- Assemble BENCH.md ------------------------------------------------------

{
  echo "# Formatter benchmark: arity vs. air"
  echo
  echo "Wall-clock formatting speed of \`arity\` against \`air\`"
  echo "(posit-dev/air), measured with [hyperfine]. Both tools format"
  echo "stdin → stdout (exit 0 regardless of changes), so the comparison is"
  echo "free of file-mutation and exit-code noise."
  echo
  echo "**This is not a CI gate and not an air-parity target.** Timings are"
  echo "machine- and run-dependent; this file measures speed only, never"
  echo "output equivalence (see \`AIR_COMPAT.md\` / \`task air-compat\` for"
  echo "that). Regenerate with \`task bench\`."
  echo
  echo "[hyperfine]: https://github.com/sharkdp/hyperfine"
  echo
  if [[ -n "${ARITY_BENCH_INPUT:-}" ]]; then
    echo "## Input"
    echo
    echo "- Override file \`$ARITY_BENCH_INPUT\` (${TIER_LINES[0]} lines)."
  else
    echo "## Corpus"
    echo
    echo "Synthetic: every \`tests/fixtures/formatter/*/expected.R\` concatenated"
    echo "(sorted, blank-line separated) into a base block, repeated to two tiers."
    echo "Content repeats, so it is cache-friendly and not fully representative of"
    echo "real code; it exists to amortize startup and show rough scaling."
    echo
    for idx in "${!TIERS[@]}"; do
      echo "- **${TIERS[$idx]}**: ${TIER_LINES[$idx]} lines"
    done
  fi
  echo
  echo "## Results"
  for idx in "${!TIERS[@]}"; do
    echo
    echo "### ${TIERS[$idx]} (${TIER_LINES[$idx]} lines)"
    echo
    cat "${RESULT_MD[$idx]}"
  done
} >"$OUT"

echo
echo ">> Wrote $OUT"
