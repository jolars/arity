#!/usr/bin/env bash
#
# Benchmark arity's formatter speed against other R formatters (air, styler) on
# a synthetic corpus, using hyperfine. Mirrors `task air-compat` in spirit: an
# opt-in, local-only measurement that regenerates a tracked, machine-readable
# artifact (benches/benchmark_results.json).
#
# The JSON artifact feeds the docs benchmark page (book/src/reference/benchmarks.md):
# `cargo run --example docgen` renders it into the generated partials at doc-gen
# time, and `mdbook build book` builds the site. The benchmark itself is never
# re-run at site-build time or in CI -- only this script rewrites the numbers.
#
# This is a *visibility* tool, not a quality gate and not an air/styler-parity
# target. It measures wall-clock formatting speed only, never output equivalence
# (that is what `task air-compat` covers). Tools do different work and pay
# different startup floors (styler is an R process), so treat the *ratios*, not
# the absolute milliseconds, as the takeaway.
#
# Usage:
#   ./scripts/bench-format.sh              # synthetic corpus (two size tiers)
#   ./scripts/bench-format.sh --out PATH   # write the JSON artifact elsewhere
#   ARITY_BENCH_INPUT=path/to/file.R ./scripts/bench-format.sh
#                                          # benchmark one real file instead
#   ARITY_BENCH_STYLER=1 ./scripts/bench-format.sh
#                                          # also measure styler (opt-in; slow)
#
# styler is an R package and pays an interpreter startup floor plus a steep
# per-line cost (seconds even on the small tier, minutes on the large one), so
# it is *not* run by default: set `ARITY_BENCH_STYLER=1` to opt in. Even then it
# is skipped on tiers larger than STYLER_MAX_LINES to keep a run tractable.
#
# Timing backend: prefers `hyperfine` (warmup + stddev/min/max) with `jq` to
# read its JSON; falls back to a plain shell timing loop (mean only) when either
# is missing. Comparison tools absent from PATH are skipped silently. Every tool
# runs stdin -> stdout so the comparison is free of file-mutation noise.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ARITY="$REPO_ROOT/target/release/arity"
JSON_OUT="benches/benchmark_results.json"
HYPERFINE_MIN_RUNS=3

# Repetition counts that build the two synthetic size tiers from the base block.
SMALL_REPS=2
LARGE_REPS=24

# styler is opt-in and, even then, skipped on tiers above this line count (it is
# an R process, orders of magnitude slower than the native tools).
STYLER_MAX_LINES=20000

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) JSON_OUT="$2"; shift 2 ;;
        -h|--help) sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; exit 2 ;;
    esac
done

# Progress goes to stderr so the JSON artifact path is the only thing on stdout.
log() { echo -e "$@" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }

# --- Tool detection ----------------------------------------------------------

HAVE_AIR=$(have air && echo yes || echo no)
HAVE_HYPERFINE=$(have hyperfine && echo yes || echo no)
HAVE_JQ=$(have jq && echo yes || echo no)

# styler is an R package: present only when Rscript can load it. Detect once,
# but only enable it when the caller opts in (it is slow; see the header).
HAVE_STYLER=no
STYLER_VER=""
if [ "${ARITY_BENCH_STYLER:-0}" != "0" ] && have Rscript; then
    STYLER_VER=$(Rscript -e 'cat(as.character(packageVersion("styler")))' 2>/dev/null || true)
    [ -n "$STYLER_VER" ] && HAVE_STYLER=yes
fi

BACKEND="shell-loop"
if [ "$HAVE_HYPERFINE" = "yes" ] && [ "$HAVE_JQ" = "yes" ]; then
    BACKEND="hyperfine"
fi

# --- Build the release binary ------------------------------------------------

log ">> Building release binary..."
cargo build --release --quiet

ARITY_VER=$("$ARITY" --version | awk '{print $2}')
AIR_VER=""
[ "$HAVE_AIR" = "yes" ] && AIR_VER=$(air --version 2>/dev/null | awk '{print $2}')

HOST_OS=$(uname -s | tr '[:upper:]' '[:lower:]')
HOST_ARCH=$(uname -m)
HOST_CPU=""
[ -f /proc/cpuinfo ] && HOST_CPU=$(grep -m1 "model name" /proc/cpuinfo | sed 's/.*: //')

log "Formatters:"
log "  arity: $ARITY_VER"
if [ "$HAVE_AIR" = "yes" ]; then log "  air: $AIR_VER"; else log "  air: (not on PATH -- skipped)"; fi
if [ "$HAVE_STYLER" = "yes" ]; then log "  styler: $STYLER_VER (opt-in)"; else log "  styler: (off -- set ARITY_BENCH_STYLER=1 to enable)"; fi
log "  backend: $BACKEND"
[ "$BACKEND" = "shell-loop" ] && log "  (hint: install hyperfine + jq for stddev/min/max stats)"
log

# --- Active tool list (arity is always first: the baseline) ------------------

declare -a TOOLS=("arity")
[ "$HAVE_AIR" = "yes" ]    && TOOLS+=("air")
[ "$HAVE_STYLER" = "yes" ] && TOOLS+=("styler")

# Command template per tool, with FILE substituted at call time (stdin -> stdout).
cmd_for() {
    local tool="$1" file="$2"
    case "$tool" in
        arity)  echo "$ARITY format < '$file' > /dev/null" ;;
        air)    echo "air format --stdin-file-path bench.R < '$file' > /dev/null" ;;
        styler) echo "Rscript -e 'invisible(styler::style_text(readLines(file(\"stdin\"))))' < '$file' > /dev/null" ;;
    esac
}

# --- JSON helpers ------------------------------------------------------------

json_escape() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

# Run one command; echo "mean stddev min max runs" in milliseconds. For the
# shell-loop backend stddev/min/max are the literal "null".
run_one() {
    local iterations="$1" cmd="$2"
    if [ "$BACKEND" = "hyperfine" ]; then
        local tmp; tmp=$(mktemp)
        hyperfine --warmup 2 --min-runs "$HYPERFINE_MIN_RUNS" \
            --export-json "$tmp" --style=none "$cmd" >/dev/null 2>&1
        local mean stddev min max runs
        mean=$(jq -r '.results[0].mean' "$tmp")
        stddev=$(jq -r '.results[0].stddev' "$tmp")
        min=$(jq -r '.results[0].min' "$tmp")
        max=$(jq -r '.results[0].max' "$tmp")
        runs=$(jq -r '.results[0].times | length' "$tmp")
        rm -f "$tmp"
        awk -v m="$mean" -v s="$stddev" -v lo="$min" -v hi="$max" -v r="$runs" \
            'BEGIN { printf "%.4f %.4f %.4f %.4f %d\n", m*1000, s*1000, lo*1000, hi*1000, r }'
    else
        local start end i
        start=$(date +%s%N)
        for ((i=1; i<=iterations; i++)); do eval "$cmd" >/dev/null 2>&1 || true; done
        end=$(date +%s%N)
        awk -v t="$((end - start))" -v n="$iterations" \
            'BEGIN { printf "%.4f null null null %d\n", (t/n)/1e6, n }'
    fi
}

# --- Build the corpus tiers --------------------------------------------------

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Each entry: "id|name|file|iterations".
declare -a CORPUS=()

if [[ -n "${ARITY_BENCH_INPUT:-}" ]]; then
    [ -f "$ARITY_BENCH_INPUT" ] || { echo "error: ARITY_BENCH_INPUT='$ARITY_BENCH_INPUT' is not a file" >&2; exit 1; }
    log ">> Using override input: $ARITY_BENCH_INPUT"
    CORPUS+=("override|$(basename "$ARITY_BENCH_INPUT")|$ARITY_BENCH_INPUT|10")
else
    log ">> Generating synthetic corpus from formatter fixtures..."
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
    [ "$found" -eq 1 ] || { echo "error: no tests/fixtures/formatter/*/expected.R files found" >&2; exit 1; }

    # NOTE: tiers repeat the same base block, so content is cache-friendly and
    # not fully representative of real code. They exist to amortize process
    # startup and show rough scaling, not to model a real workload.
    for tier in "small:$SMALL_REPS:50" "large:$LARGE_REPS:5"; do
        IFS=':' read -r label reps iters <<< "$tier"
        corpus="$TMP/corpus_${label}.R"
        : >"$corpus"
        for ((i = 0; i < reps; i++)); do cat "$BASE" >>"$corpus"; done
        CORPUS+=("$label|$label|$corpus|$iters")
    done
fi

# --- Run ---------------------------------------------------------------------

declare -a DOC_ID=() DOC_NAME=() DOC_FILE=() DOC_SIZE=() DOC_LINES=() DOC_ITERS=()
declare -a RES_DOC=() RES_TOOL=() RES_MEAN=() RES_STDDEV=() RES_MIN=() RES_MAX=() RES_RUNS=()

for entry in "${CORPUS[@]}"; do
    IFS='|' read -r id name file iters <<< "$entry"

    # Sanity gate: arity must format the doc without error (parse diagnostics).
    if ! "$ARITY" format < "$file" >/dev/null 2>&1; then
        log "!! skip $name -- arity cannot format it (parse diagnostics)"
        continue
    fi

    size=$(wc -c < "$file"); lines=$(wc -l < "$file")
    DOC_ID+=("$id"); DOC_NAME+=("$name"); DOC_FILE+=("$file")
    DOC_SIZE+=("$size"); DOC_LINES+=("$lines"); DOC_ITERS+=("$iters")

    log "== $name ($size bytes, $lines lines) =="
    for tool in "${TOOLS[@]}"; do
        # styler is orders of magnitude slower; skip it on oversized tiers so an
        # opt-in run stays tractable. It simply gets no row for that tier.
        if [ "$tool" = "styler" ] && [ "$lines" -gt "$STYLER_MAX_LINES" ]; then
            log "  styler... (skipped: $lines > $STYLER_MAX_LINES lines)"
            continue
        fi
        cmd="$(cmd_for "$tool" "$file")"
        log "  $tool..."
        read -r mean stddev min max runs < <(run_one "$iters" "$cmd")
        RES_DOC+=("$id"); RES_TOOL+=("$tool"); RES_MEAN+=("$mean")
        RES_STDDEV+=("$stddev"); RES_MIN+=("$min"); RES_MAX+=("$max"); RES_RUNS+=("$runs")
    done
    log
done

[ "${#DOC_ID[@]}" -gt 0 ] || { echo "error: no documents benchmarked (corpus missing or all gated out)" >&2; exit 1; }

# --- Render JSON -------------------------------------------------------------

mkdir -p "$(dirname "$JSON_OUT")"
{
    printf '{\n'
    printf '  "schema_version": 1,\n'
    printf '  "meta": {\n'
    printf '    "generated_at": "%s",\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '    "host": {"os": "%s", "arch": "%s", "cpu": "%s"},\n' \
        "$(json_escape "$HOST_OS")" "$(json_escape "$HOST_ARCH")" "$(json_escape "$HOST_CPU")"
    printf '    "backend": "%s",\n' "$BACKEND"
    printf '    "min_runs": %d,\n' "$HYPERFINE_MIN_RUNS"
    printf '    "tools": {\n'
    printf '      "arity": {"version": "%s"}' "$(json_escape "$ARITY_VER")"
    [ "$HAVE_AIR" = "yes" ]    && printf ',\n      "air": {"version": "%s"}' "$(json_escape "$AIR_VER")"
    [ "$HAVE_STYLER" = "yes" ] && printf ',\n      "styler": {"version": "%s"}' "$(json_escape "$STYLER_VER")"
    printf '\n    }\n'
    printf '  },\n'

    printf '  "documents": [\n'
    for i in "${!DOC_ID[@]}"; do
        printf '    {"id":"%s","name":"%s","file":"%s","size_bytes":%d,"lines":%d,"iterations":%d}' \
            "${DOC_ID[$i]}" "$(json_escape "${DOC_NAME[$i]}")" "$(basename "${DOC_FILE[$i]}")" \
            "${DOC_SIZE[$i]}" "${DOC_LINES[$i]}" "${DOC_ITERS[$i]}"
        [ "$i" -lt $((${#DOC_ID[@]} - 1)) ] && printf ','
        printf '\n'
    done
    printf '  ],\n'

    printf '  "results": [\n'
    for i in "${!RES_DOC[@]}"; do
        printf '    {"document":"%s","formatter":"%s","mean_ms":%s,"stddev_ms":%s,"min_ms":%s,"max_ms":%s,"runs":%d}' \
            "${RES_DOC[$i]}" "${RES_TOOL[$i]}" "${RES_MEAN[$i]}" \
            "${RES_STDDEV[$i]}" "${RES_MIN[$i]}" "${RES_MAX[$i]}" "${RES_RUNS[$i]}"
        [ "$i" -lt $((${#RES_DOC[@]} - 1)) ] && printf ','
        printf '\n'
    done
    printf '  ]\n'
    printf '}\n'
} > "$JSON_OUT"

log ">> Wrote $JSON_OUT"
echo "$JSON_OUT"
