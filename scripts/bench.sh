#!/usr/bin/env bash
#
# Benchmark arity's formatter and linter speed against other R tools, using
# hyperfine. Mirrors `task air-compat` in spirit: an opt-in, local-only
# measurement that regenerates a tracked, machine-readable artifact
# (benches/benchmark_results.json).
#
# Two operations, each measured at two scopes:
#
#   * formatter (vs air; styler opt-in)   single files + a real R package
#   * linter    (vs jarl)                 single files + a real R package
#
# "Single files" are synthetic corpus tiers built from the formatter fixtures;
# "projects" is the R/ source tree of a real package (tidyr by default), cloned
# once into a cache. arity is the baseline in every chart; every other tool's
# time is reported relative to it.
#
# The JSON artifact feeds the docs benchmark page (docs/src/reference/benchmarks.md):
# `cargo run --example docgen` renders it into the generated partials at doc-gen
# time, and `mdbook build docs` builds the site. The benchmark itself is never
# re-run at site-build time or in CI -- only this script rewrites the numbers.
#
# This is a *visibility* tool, not a quality gate and not a parity target. It
# measures wall-clock speed only, never output equivalence (that is what
# `task air-compat` covers). Tools do different work and pay different startup
# floors (styler is an R process), so treat the *ratios*, not the absolute
# milliseconds, as the takeaway.
#
# Usage:
#   ./scripts/bench.sh                     # all charts (formatter + linter)
#   ./scripts/bench.sh --out PATH          # write the JSON artifact elsewhere
#   ARITY_BENCH_STYLER=1 ./scripts/bench.sh
#                                          # also measure styler on formatter
#                                          # single files (opt-in; slow)
#   ARITY_BENCH_PROJECT=/path/to/pkg ./scripts/bench.sh
#                                          # use a local package checkout instead
#                                          # of cloning tidyr
#
# styler is an R package and pays an interpreter startup floor plus a steep
# per-line cost, so it is *not* run by default and only ever on the formatter
# single-file tiers (never on projects, where style_dir would mutate the
# checkout): set `ARITY_BENCH_STYLER=1` to opt in. Even then it is skipped on
# tiers larger than STYLER_MAX_LINES to keep a run tractable.
#
# Timing backend: prefers `hyperfine` (warmup + stddev/min/max) with `jq` to
# read its JSON; falls back to a plain shell timing loop (mean only) when either
# is missing. Comparison tools absent from PATH are skipped silently.
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

# The real package benchmarked for the "projects" charts. Cloned once (shallow,
# pinned tag) into a cache unless ARITY_BENCH_PROJECT points at a local checkout.
PROJECT_NAME="tidyr"
PROJECT_REPO="https://github.com/tidyverse/tidyr"
PROJECT_TAG="v1.3.2"
BENCH_CACHE="${ARITY_BENCH_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/arity-bench}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) JSON_OUT="$2"; shift 2 ;;
        -h|--help) sed -n '2,49p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; exit 2 ;;
    esac
done

# Progress goes to stderr so the JSON artifact path is the only thing on stdout.
log() { echo -e "$@" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }

# --- Tool detection ----------------------------------------------------------

HAVE_AIR=$(have air && echo yes || echo no)
HAVE_JARL=$(have jarl && echo yes || echo no)
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
JARL_VER=""
[ "$HAVE_JARL" = "yes" ] && JARL_VER=$(jarl --version 2>/dev/null | awk '{print $2}')

HOST_OS=$(uname -s | tr '[:upper:]' '[:lower:]')
HOST_ARCH=$(uname -m)
HOST_CPU=""
[ -f /proc/cpuinfo ] && HOST_CPU=$(grep -m1 "model name" /proc/cpuinfo | sed 's/.*: //')

log "Tools:"
log "  arity: $ARITY_VER (baseline)"
if [ "$HAVE_AIR" = "yes" ]; then log "  air: $AIR_VER (formatter)"; else log "  air: (not on PATH -- skipped)"; fi
if [ "$HAVE_JARL" = "yes" ]; then log "  jarl: $JARL_VER (linter)"; else log "  jarl: (not on PATH -- skipped)"; fi
if [ "$HAVE_STYLER" = "yes" ]; then log "  styler: $STYLER_VER (formatter, opt-in)"; else log "  styler: (off -- set ARITY_BENCH_STYLER=1 to enable)"; fi
log "  backend: $BACKEND"
[ "$BACKEND" = "shell-loop" ] && log "  (hint: install hyperfine + jq for stddev/min/max stats)"
log

# --- Command templates (stdin -> stdout, or a path/dir) ----------------------

# Formatter command for TOOL over PATH in MODE (stdin|path). Path mode uses
# --check so a directory run never mutates the checkout while still doing the
# full formatting work.
fmt_cmd() {
    case "$1:$2" in
        arity:stdin)  echo "$ARITY format < '$3' > /dev/null 2>&1" ;;
        arity:path)   echo "$ARITY format --check '$3' > /dev/null 2>&1" ;;
        air:stdin)    echo "air format --stdin-file-path bench.R < '$3' > /dev/null 2>&1" ;;
        air:path)     echo "air format --check '$3' > /dev/null 2>&1" ;;
        styler:stdin) echo "Rscript -e 'invisible(styler::style_text(readLines(file(\"stdin\"))))' < '$3' > /dev/null 2>&1" ;;
    esac
}

# Linter command for TOOL over PATH in MODE. jarl has no stdin mode, so the
# linter charts always pass a file or directory path (arity matched for a
# like-for-like comparison).
lint_cmd() {
    case "$1:$2" in
        arity:stdin) echo "$ARITY lint < '$3' > /dev/null 2>&1" ;;
        arity:path)  echo "$ARITY lint '$3' > /dev/null 2>&1" ;;
        jarl:path)   echo "jarl check '$3' > /dev/null 2>&1" ;;
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
        # -i: tools that find issues exit non-zero (linters, format --check);
        # we time the work regardless of the verdict.
        hyperfine --warmup 2 --min-runs "$HYPERFINE_MIN_RUNS" -i \
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

# --- Corpus + project inputs -------------------------------------------------

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

log ">> Generating synthetic corpus from formatter fixtures..."
# One deterministic base block: every formatter fixture's expected.R, in sorted
# order, blank-line separated. These are guaranteed arity-parseable and
# arity-clean, so `arity format` never errors on them.
BASE="$TMP/base.R"
: >"$BASE"
found=0
while IFS= read -r f; do
    cat "$f" >>"$BASE"
    printf '\n\n' >>"$BASE"
    found=1
done < <(find crates/arity-formatter/tests/fixtures/formatter -name expected.R | sort)
[ "$found" -eq 1 ] || { echo "error: no crates/arity-formatter/tests/fixtures/formatter/*/expected.R files found" >&2; exit 1; }

# NOTE: tiers repeat the same base block, so content is cache-friendly and not
# fully representative of real code. They exist to amortize process startup and
# show rough scaling, not to model a real workload.
CORPUS_SMALL="$TMP/corpus_small.R"; : >"$CORPUS_SMALL"
CORPUS_LARGE="$TMP/corpus_large.R"; : >"$CORPUS_LARGE"
for ((i = 0; i < SMALL_REPS; i++)); do cat "$BASE" >>"$CORPUS_SMALL"; done
for ((i = 0; i < LARGE_REPS; i++)); do cat "$BASE" >>"$CORPUS_LARGE"; done

# The project: a real package's R/ source tree. Cloned once (shallow, pinned) or
# taken from a local checkout via ARITY_BENCH_PROJECT. Best-effort: if the clone
# fails (e.g. offline) the project charts are simply omitted.
PROJECT_TARGET=""
PKG_DIR=""
if [ -n "${ARITY_BENCH_PROJECT:-}" ]; then
    PKG_DIR="$ARITY_BENCH_PROJECT"
else
    PKG_DIR="$BENCH_CACHE/$PROJECT_NAME"
    if [ ! -d "$PKG_DIR/.git" ] && [ ! -d "$PKG_DIR/R" ]; then
        log ">> Cloning $PROJECT_NAME ($PROJECT_TAG) into $PKG_DIR..."
        mkdir -p "$BENCH_CACHE"
        if ! git clone --depth 1 --branch "$PROJECT_TAG" "$PROJECT_REPO" "$PKG_DIR" >/dev/null 2>&1; then
            log "!! clone failed -- project charts will be omitted (set ARITY_BENCH_PROJECT to a local checkout)"
            rm -rf "$PKG_DIR"
            PKG_DIR=""
        fi
    fi
fi
if [ -n "$PKG_DIR" ] && [ -d "$PKG_DIR" ]; then
    if [ -d "$PKG_DIR/R" ]; then PROJECT_TARGET="$PKG_DIR/R"; else PROJECT_TARGET="$PKG_DIR"; fi
fi

# --- Result accumulation -----------------------------------------------------

declare -a DOC_CHART=() DOC_ID=() DOC_NAME=() DOC_SIZE=() DOC_LINES=()
declare -a RES_CHART=() RES_DOC=() RES_TOOL=() RES_MEAN=() RES_STDDEV=() RES_MIN=() RES_MAX=()

# Total bytes/lines of the .R files under a directory (or of a single file).
input_size() {
    if [ -d "$1" ]; then
        find "$1" -type f \( -name '*.R' -o -name '*.r' \) -exec cat {} + | wc -c
    else
        wc -c < "$1"
    fi
}
input_lines() {
    if [ -d "$1" ]; then
        find "$1" -type f \( -name '*.R' -o -name '*.r' \) -exec cat {} + | wc -l
    else
        wc -l < "$1"
    fi
}

# Benchmark one document across a tool list and record the rows.
#   bench_doc CHART OP MODE DOC_ID NAME PATH ITERS TOOL...
bench_doc() {
    local chart="$1" op="$2" mode="$3" doc_id="$4" name="$5" path="$6" iters="$7"; shift 7
    local tools=("$@")

    local size lines
    size=$(input_size "$path"); lines=$(input_lines "$path")
    DOC_CHART+=("$chart"); DOC_ID+=("$doc_id"); DOC_NAME+=("$name")
    DOC_SIZE+=("$size"); DOC_LINES+=("$lines")

    log "== [$chart] $name ($size bytes, $lines lines) =="
    local tool cmd mean stddev min max runs
    for tool in "${tools[@]}"; do
        if [ "$op" = format ]; then cmd="$(fmt_cmd "$tool" "$mode" "$path")"; else cmd="$(lint_cmd "$tool" "$mode" "$path")"; fi
        [ -z "$cmd" ] && continue
        # styler is orders of magnitude slower; skip it on oversized tiers.
        if [ "$tool" = "styler" ] && [ "$lines" -gt "$STYLER_MAX_LINES" ]; then
            log "  styler... (skipped: $lines > $STYLER_MAX_LINES lines)"
            continue
        fi
        log "  $tool..."
        read -r mean stddev min max runs < <(run_one "$iters" "$cmd")
        RES_CHART+=("$chart"); RES_DOC+=("$doc_id"); RES_TOOL+=("$tool"); RES_MEAN+=("$mean")
        RES_STDDEV+=("$stddev"); RES_MIN+=("$min"); RES_MAX+=("$max")
    done
    log
}

# --- Tool lists per chart ----------------------------------------------------

declare -a FMT_FILE_TOOLS=("arity")
[ "$HAVE_AIR" = "yes" ]    && FMT_FILE_TOOLS+=("air")
[ "$HAVE_STYLER" = "yes" ] && FMT_FILE_TOOLS+=("styler")

declare -a FMT_PROJ_TOOLS=("arity")
[ "$HAVE_AIR" = "yes" ] && FMT_PROJ_TOOLS+=("air")

declare -a LINT_TOOLS=("arity")
[ "$HAVE_JARL" = "yes" ] && LINT_TOOLS+=("jarl")

# --- Run the charts ----------------------------------------------------------

# Formatter, single files (stdin -> stdout).
bench_doc formatter-files format stdin small small "$CORPUS_SMALL" 50 "${FMT_FILE_TOOLS[@]}"
bench_doc formatter-files format stdin large large "$CORPUS_LARGE" 5  "${FMT_FILE_TOOLS[@]}"

# Linter, single files (path input; jarl has no stdin mode).
bench_doc linter-files lint path small small "$CORPUS_SMALL" 50 "${LINT_TOOLS[@]}"
bench_doc linter-files lint path large large "$CORPUS_LARGE" 5  "${LINT_TOOLS[@]}"

# Projects (a real package's R/ tree), if available.
if [ -n "$PROJECT_TARGET" ]; then
    bench_doc formatter-projects format path "$PROJECT_NAME" "$PROJECT_NAME" "$PROJECT_TARGET" 10 "${FMT_PROJ_TOOLS[@]}"
    bench_doc linter-projects    lint   path "$PROJECT_NAME" "$PROJECT_NAME" "$PROJECT_TARGET" 10 "${LINT_TOOLS[@]}"
else
    log "!! no project input -- omitting the project charts"
fi

[ "${#DOC_ID[@]}" -gt 0 ] || { echo "error: no documents benchmarked" >&2; exit 1; }

# --- Render JSON -------------------------------------------------------------

# Emit the documents[] array body for one chart (filtered by chart key).
emit_documents() {
    local chart="$1" first=1 i
    for i in "${!DOC_ID[@]}"; do
        [ "${DOC_CHART[$i]}" = "$chart" ] || continue
        [ "$first" -eq 1 ] || printf ',\n'
        first=0
        printf '            {"id":"%s","name":"%s","size_bytes":%d,"lines":%d}' \
            "${DOC_ID[$i]}" "$(json_escape "${DOC_NAME[$i]}")" "${DOC_SIZE[$i]}" "${DOC_LINES[$i]}"
    done
    printf '\n'
}

# Emit the results[] array body for one chart (filtered by chart key).
emit_results() {
    local chart="$1" first=1 i
    for i in "${!RES_DOC[@]}"; do
        [ "${RES_CHART[$i]}" = "$chart" ] || continue
        [ "$first" -eq 1 ] || printf ',\n'
        first=0
        printf '            {"document":"%s","tool":"%s","mean_ms":%s,"stddev_ms":%s,"min_ms":%s,"max_ms":%s}' \
            "${RES_DOC[$i]}" "${RES_TOOL[$i]}" "${RES_MEAN[$i]}" \
            "${RES_STDDEV[$i]}" "${RES_MIN[$i]}" "${RES_MAX[$i]}"
    done
    printf '\n'
}

# True if any result rows were recorded for a chart.
chart_has_rows() {
    local chart="$1" i
    for i in "${!RES_CHART[@]}"; do [ "${RES_CHART[$i]}" = "$chart" ] && return 0; done
    return 1
}

# Emit one chart object (title, caption, documents[], results[]), followed by
# TRAILING (a comma or empty). Skips silently if the chart recorded no rows.
emit_chart() {
    local chart="$1" title="$2" caption="$3" trailing="$4"
    chart_has_rows "$chart" || return 0
    printf '        {\n'
    printf '          "id":"%s",\n' "$chart"
    printf '          "title":"%s",\n' "$(json_escape "$title")"
    printf '          "caption":"%s",\n' "$(json_escape "$caption")"
    printf '          "documents":[\n'; emit_documents "$chart"; printf '          ],\n'
    printf '          "results":[\n'; emit_results "$chart"; printf '          ]\n'
    printf '        }%s\n' "$trailing"
}

mkdir -p "$(dirname "$JSON_OUT")"
{
    printf '{\n'
    printf '  "schema_version": 2,\n'
    printf '  "meta": {\n'
    printf '    "generated_at": "%s",\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '    "host": {"os": "%s", "arch": "%s", "cpu": "%s"},\n' \
        "$(json_escape "$HOST_OS")" "$(json_escape "$HOST_ARCH")" "$(json_escape "$HOST_CPU")"
    printf '    "backend": "%s",\n' "$BACKEND"
    printf '    "min_runs": %d,\n' "$HYPERFINE_MIN_RUNS"
    printf '    "tools": {\n'
    printf '      "arity": {"version": "%s"}' "$(json_escape "$ARITY_VER")"
    [ "$HAVE_AIR" = "yes" ]    && printf ',\n      "air": {"version": "%s"}' "$(json_escape "$AIR_VER")"
    [ "$HAVE_JARL" = "yes" ]   && printf ',\n      "jarl": {"version": "%s"}' "$(json_escape "$JARL_VER")"
    [ "$HAVE_STYLER" = "yes" ] && printf ',\n      "styler": {"version": "%s"}' "$(json_escape "$STYLER_VER")"
    printf '\n    }\n'
    printf '  },\n'

    printf '  "sections": [\n'

    # --- Formatter section ---
    printf '    {\n      "id":"formatter",\n      "title":"Formatter",\n      "charts":[\n'
    # The projects chart follows the files chart only when it has rows; pick the
    # comma between them from what actually ran.
    if chart_has_rows formatter-projects; then f_files_trail=","; else f_files_trail=""; fi
    emit_chart formatter-files "Single files" \
        "Formatting speed on single files relative to arity, one dot per synthetic corpus tier. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures." \
        "$f_files_trail"
    emit_chart formatter-projects "Projects" \
        "Formatting speed on a real R package (the $PROJECT_NAME source tree) relative to arity, on the same log-ratio axis." \
        ""
    printf '      ]\n    },\n'

    # --- Linter section ---
    printf '    {\n      "id":"linter",\n      "title":"Linter",\n      "charts":[\n'
    if chart_has_rows linter-projects; then l_files_trail=","; else l_files_trail=""; fi
    emit_chart linter-files "Single files" \
        "Linting speed on single files relative to arity, one dot per synthetic corpus tier, on the same log-ratio axis as the formatter charts." \
        "$l_files_trail"
    emit_chart linter-projects "Projects" \
        "Linting speed on a real R package (the $PROJECT_NAME source tree) relative to arity, on the same log-ratio axis." \
        ""
    printf '      ]\n    }\n'

    printf '  ]\n'
    printf '}\n'
} > "$JSON_OUT"

log ">> Wrote $JSON_OUT"
echo "$JSON_OUT"
