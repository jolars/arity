#!/usr/bin/env bash
#
# Benchmark arity's formatter and linter speed against other R tools, using
# hyperfine. Mirrors `task air-compat` in spirit: an opt-in, local-only
# measurement that regenerates a tracked, machine-readable artifact
# (benches/benchmark_results.json).
#
# Two operations, each measured at two scopes:
#
#   * formatter (vs air, styler)          single files + real R packages
#   * linter    (vs jarl, lintr)          single files + real R packages
#
# "Single files" mixes two synthetic corpus tiers built from the formatter
# fixtures with the largest real source file of each benchmarked package;
# "projects" is the R/ source tree of each package, cloned once into a cache.
# arity is the baseline in every chart; every other tool's time is reported
# relative to it.
#
# The JSON artifact feeds the docs benchmark page (docs/src/guide/performance.md):
# `cargo run --example docgen` renders it into the generated partials at doc-gen
# time, and `mdbook build docs` builds the site. The benchmark itself is never
# re-run at site-build time or in CI -- only this script rewrites the numbers.
#
# This is a *visibility* tool, not a quality gate and not a parity target. It
# measures wall-clock speed only, never output equivalence (that is what
# `task air-compat` covers). Tools do different work and pay different startup
# floors (styler and lintr are R processes), so treat the *ratios*, not
# the absolute milliseconds, as the takeaway.
#
# Usage:
#   ./scripts/bench.sh                     # all charts (formatter + linter)
#   ./scripts/bench.sh --out PATH          # write the JSON artifact elsewhere
#   ARITY_BENCH_NO_R=1 ./scripts/bench.sh
#                                          # skip the R-backed tools (styler,
#                                          # lintr) for a fast run
#   ARITY_BENCH_PROJECT=/path/to/pkg ./scripts/bench.sh
#                                          # use a single local package checkout
#                                          # instead of the pinned clones
#
# styler and lintr are R packages that pay an interpreter startup floor plus a
# steep per-line cost, so they are skipped on documents above R_SLOW_MAX_LINES
# to keep a run tractable; styler additionally never runs on projects, where
# style_dir would mutate the checkout.
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

# The slow R-backed tools (styler, lintr) are skipped on documents above this
# line count; they are orders of magnitude slower than the native tools.
R_SLOW_MAX_LINES=20000

# The real packages benchmarked for the "projects" charts, and the source of the
# real single-file documents. Cloned once (shallow, pinned tag) into a cache
# unless ARITY_BENCH_PROJECT points at a local checkout, which replaces the list.
# Format: NAME|REPO|TAG
PROJECT_SPECS=(
    "tidyr|https://github.com/tidyverse/tidyr|v1.3.2"
    "MASS|https://github.com/cran/MASS|7.3-66"
)
BENCH_CACHE="${ARITY_BENCH_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/arity-bench}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) JSON_OUT="$2"; shift 2 ;;
        -h|--help) sed -n '2,48p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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

# Version of an installed R package, or empty when Rscript cannot load it. The
# whole R-backed set can be turned off with ARITY_BENCH_NO_R=1.
r_pkg_version() {
    [ "${ARITY_BENCH_NO_R:-0}" = "0" ] || return 0
    have Rscript || return 0
    Rscript -e "cat(as.character(packageVersion('$1')))" 2>/dev/null || true
}

STYLER_VER=$(r_pkg_version styler)
LINTR_VER=$(r_pkg_version lintr)
HAVE_STYLER=$([ -n "$STYLER_VER" ] && echo yes || echo no)
HAVE_LINTR=$([ -n "$LINTR_VER" ] && echo yes || echo no)

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

# Every tool that will appear in the artifact's meta.tools map, arity first.
declare -a TOOL_NAMES=("arity") TOOL_VERS=("$ARITY_VER")
add_tool() { [ -n "$2" ] && { TOOL_NAMES+=("$1"); TOOL_VERS+=("$2"); }; return 0; }
add_tool air "$AIR_VER"
add_tool jarl "$JARL_VER"
add_tool styler "$STYLER_VER"
add_tool lintr "$LINTR_VER"

HOST_OS=$(uname -s | tr '[:upper:]' '[:lower:]')
HOST_ARCH=$(uname -m)
HOST_CPU=""
[ -f /proc/cpuinfo ] && HOST_CPU=$(grep -m1 "model name" /proc/cpuinfo | sed 's/.*: //')

log "Tools:"
log "  arity: $ARITY_VER (baseline)"
if [ "$HAVE_AIR" = "yes" ]; then log "  air: $AIR_VER (formatter)"; else log "  air: (not on PATH -- skipped)"; fi
if [ "$HAVE_JARL" = "yes" ]; then log "  jarl: $JARL_VER (linter)"; else log "  jarl: (not on PATH -- skipped)"; fi
if [ "$HAVE_STYLER" = "yes" ]; then log "  styler: $STYLER_VER (formatter, R)"; else log "  styler: (unavailable -- skipped)"; fi
if [ "$HAVE_LINTR" = "yes" ]; then log "  lintr: $LINTR_VER (linter, R)"; else log "  lintr: (unavailable -- skipped)"; fi
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
        # styler keeps a persistent on-disk cache of already-styled expressions,
        # which would make its timings depend on what earlier runs happened to
        # style. Deactivate it (session-scoped) so every run does the full work,
        # like every other tool here.
        styler:stdin) echo "Rscript -e 'styler::cache_deactivate(); invisible(styler::style_text(readLines(file(\"stdin\"))))' < '$3' > /dev/null 2>&1" ;;
    esac
}

# Linter command for TOOL over PATH in MODE. jarl has no stdin mode, so the
# linter charts always pass a file or directory path (arity matched for a
# like-for-like comparison). lintr takes one path either way and picks its
# file/directory entry point from it.
lint_cmd() {
    case "$1:$2" in
        arity:stdin) echo "$ARITY lint < '$3' > /dev/null 2>&1" ;;
        arity:path)  echo "$ARITY lint '$3' > /dev/null 2>&1" ;;
        jarl:path)   echo "jarl check '$3' > /dev/null 2>&1" ;;
        lintr:path)  echo "Rscript -e 'p <- \"$3\"; invisible(if (dir.exists(p)) lintr::lint_dir(p) else lintr::lint(p))' > /dev/null 2>&1" ;;
    esac
}

# The R-backed tools that are too slow to run on the large documents.
is_slow_tool() { [ "$1" = "styler" ] || [ "$1" = "lintr" ]; }

# --- JSON helpers ------------------------------------------------------------

json_escape() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

# Run one command; echo "mean stddev min max runs" in milliseconds. For the
# shell-loop backend stddev/min/max are the literal "null".
run_one() {
    local iterations="$1" cmd="$2" warmup="$3"
    if [ "$BACKEND" = "hyperfine" ]; then
        local tmp; tmp=$(mktemp)
        # -i: tools that find issues exit non-zero (linters, format --check);
        # we time the work regardless of the verdict.
        hyperfine --warmup "$warmup" --min-runs "$HYPERFINE_MIN_RUNS" -i \
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
# show rough scaling; the real package files below cover representativeness.
CORPUS_SMALL="$TMP/corpus_small.R"; : >"$CORPUS_SMALL"
CORPUS_LARGE="$TMP/corpus_large.R"; : >"$CORPUS_LARGE"
for ((i = 0; i < SMALL_REPS; i++)); do cat "$BASE" >>"$CORPUS_SMALL"; done
for ((i = 0; i < LARGE_REPS; i++)); do cat "$BASE" >>"$CORPUS_LARGE"; done

# The projects: each package's R/ source tree. Cloned once (shallow, pinned) or
# taken from a single local checkout via ARITY_BENCH_PROJECT. Best-effort: a
# project whose clone fails (e.g. offline) is simply dropped.
declare -a PROJ_NAMES=() PROJ_TARGETS=()

add_project() {
    local name="$1" dir="$2"
    [ -d "$dir" ] || return 0
    if [ -d "$dir/R" ]; then PROJ_NAMES+=("$name"); PROJ_TARGETS+=("$dir/R")
    else PROJ_NAMES+=("$name"); PROJ_TARGETS+=("$dir"); fi
}

if [ -n "${ARITY_BENCH_PROJECT:-}" ]; then
    add_project "$(basename "${ARITY_BENCH_PROJECT%/}")" "$ARITY_BENCH_PROJECT"
else
    for spec in "${PROJECT_SPECS[@]}"; do
        IFS='|' read -r p_name p_repo p_tag <<<"$spec"
        p_dir="$BENCH_CACHE/$p_name"
        if [ ! -d "$p_dir/.git" ] && [ ! -d "$p_dir/R" ]; then
            log ">> Cloning $p_name ($p_tag) into $p_dir..."
            mkdir -p "$BENCH_CACHE"
            if ! git clone --depth 1 --branch "$p_tag" "$p_repo" "$p_dir" >/dev/null 2>&1; then
                log "!! clone failed -- $p_name will be omitted (set ARITY_BENCH_PROJECT to a local checkout)"
                rm -rf "$p_dir"
                continue
            fi
        fi
        add_project "$p_name" "$p_dir"
    done
fi
[ "${#PROJ_NAMES[@]}" -gt 0 ] || log "!! no project inputs -- omitting the project charts and the real single-file documents"

# The real single-file documents: the largest .R file of each project, so the
# file charts are not purely synthetic. Ordered by size so the charts read
# small-to-large across real files and then tiers.
declare -a FILE_IDS=() FILE_NAMES=() FILE_PATHS=()
if [ "${#PROJ_NAMES[@]}" -gt 0 ]; then
    biggest_list="$TMP/biggest"; : >"$biggest_list"
    for i in "${!PROJ_NAMES[@]}"; do
        biggest=$(find "${PROJ_TARGETS[$i]}" -type f \( -name '*.R' -o -name '*.r' \) -printf '%s\t%p\n' \
            | sort -t$'\t' -k1,1nr -k2,2 | head -1 | cut -f2)
        [ -n "$biggest" ] || continue
        printf '%s\t%s\t%s\n' "$(wc -c <"$biggest")" "${PROJ_NAMES[$i]}" "$biggest" >>"$biggest_list"
    done
    while IFS=$'\t' read -r _size p_name p_path; do
        [ -n "$p_path" ] || continue
        FILE_IDS+=("$p_name-file")
        FILE_NAMES+=("$p_name/$(basename "$p_path")")
        FILE_PATHS+=("$p_path")
    done < <(sort -t$'\t' -k1,1n -k2,2 "$biggest_list")
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
    local tool cmd mean stddev min max runs warmup
    for tool in "${tools[@]}"; do
        if [ "$op" = format ]; then cmd="$(fmt_cmd "$tool" "$mode" "$path")"; else cmd="$(lint_cmd "$tool" "$mode" "$path")"; fi
        [ -z "$cmd" ] && continue
        warmup=2
        # styler and lintr are orders of magnitude slower; skip them on oversized
        # documents, and do not pay for a second warmup run on the rest.
        if is_slow_tool "$tool"; then
            warmup=1
            if [ "$lines" -gt "$R_SLOW_MAX_LINES" ]; then
                log "  $tool... (skipped: $lines > $R_SLOW_MAX_LINES lines)"
                continue
            fi
        fi
        log "  $tool..."
        read -r mean stddev min max runs < <(run_one "$iters" "$cmd" "$warmup")
        RES_CHART+=("$chart"); RES_DOC+=("$doc_id"); RES_TOOL+=("$tool"); RES_MEAN+=("$mean")
        RES_STDDEV+=("$stddev"); RES_MIN+=("$min"); RES_MAX+=("$max")
    done
    log
}

# --- Tool lists per chart ----------------------------------------------------

declare -a FMT_FILE_TOOLS=("arity")
[ "$HAVE_AIR" = "yes" ]    && FMT_FILE_TOOLS+=("air")
[ "$HAVE_STYLER" = "yes" ] && FMT_FILE_TOOLS+=("styler")

# styler is absent from the project charts on purpose: style_dir would rewrite
# the checkout, and it has no check-only directory mode.
declare -a FMT_PROJ_TOOLS=("arity")
[ "$HAVE_AIR" = "yes" ] && FMT_PROJ_TOOLS+=("air")

declare -a LINT_TOOLS=("arity")
[ "$HAVE_JARL" = "yes" ]  && LINT_TOOLS+=("jarl")
[ "$HAVE_LINTR" = "yes" ] && LINT_TOOLS+=("lintr")

# --- Run the charts ----------------------------------------------------------

# Formatter, single files (stdin -> stdout): real package files, then the tiers.
for i in "${!FILE_IDS[@]}"; do
    bench_doc formatter-files format stdin "${FILE_IDS[$i]}" "${FILE_NAMES[$i]}" "${FILE_PATHS[$i]}" 20 "${FMT_FILE_TOOLS[@]}"
done
bench_doc formatter-files format stdin small small "$CORPUS_SMALL" 50 "${FMT_FILE_TOOLS[@]}"
bench_doc formatter-files format stdin large large "$CORPUS_LARGE" 5  "${FMT_FILE_TOOLS[@]}"

# Linter, single files (path input; jarl has no stdin mode).
for i in "${!FILE_IDS[@]}"; do
    bench_doc linter-files lint path "${FILE_IDS[$i]}" "${FILE_NAMES[$i]}" "${FILE_PATHS[$i]}" 20 "${LINT_TOOLS[@]}"
done
bench_doc linter-files lint path small small "$CORPUS_SMALL" 50 "${LINT_TOOLS[@]}"
bench_doc linter-files lint path large large "$CORPUS_LARGE" 5  "${LINT_TOOLS[@]}"

# Projects (each package's R/ tree), if available.
for i in "${!PROJ_NAMES[@]}"; do
    bench_doc formatter-projects format path "${PROJ_NAMES[$i]}" "${PROJ_NAMES[$i]}" "${PROJ_TARGETS[$i]}" 10 "${FMT_PROJ_TOOLS[@]}"
done
for i in "${!PROJ_NAMES[@]}"; do
    bench_doc linter-projects lint path "${PROJ_NAMES[$i]}" "${PROJ_NAMES[$i]}" "${PROJ_TARGETS[$i]}" 10 "${LINT_TOOLS[@]}"
done

[ "${#DOC_ID[@]}" -gt 0 ] || { echo "error: no documents benchmarked" >&2; exit 1; }

# A human list of the benchmarked packages for the project captions.
PROJECT_LIST=""
for i in "${!PROJ_NAMES[@]}"; do
    if [ -z "$PROJECT_LIST" ]; then PROJECT_LIST="${PROJ_NAMES[$i]}"
    elif [ "$i" -eq $((${#PROJ_NAMES[@]} - 1)) ]; then PROJECT_LIST="$PROJECT_LIST and ${PROJ_NAMES[$i]}"
    else PROJECT_LIST="$PROJECT_LIST, ${PROJ_NAMES[$i]}"; fi
done

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
    for i in "${!TOOL_NAMES[@]}"; do
        [ "$i" -eq 0 ] || printf ',\n'
        printf '      "%s": {"version": "%s"}' \
            "${TOOL_NAMES[$i]}" "$(json_escape "${TOOL_VERS[$i]}")"
    done
    printf '\n    }\n'
    printf '  },\n'

    printf '  "sections": [\n'

    # --- Formatter section ---
    printf '    {\n      "id":"formatter",\n      "title":"Formatter",\n      "charts":[\n'
    # The projects chart follows the files chart only when it has rows; pick the
    # comma between them from what actually ran.
    if chart_has_rows formatter-projects; then f_files_trail=","; else f_files_trail=""; fi
    emit_chart formatter-files "Single files" \
        "Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures." \
        "$f_files_trail"
    emit_chart formatter-projects "Projects" \
        "Formatting speed on real R packages (the $PROJECT_LIST source trees) relative to arity, on the same log-ratio axis." \
        ""
    printf '      ]\n    },\n'

    # --- Linter section ---
    printf '    {\n      "id":"linter",\n      "title":"Linter",\n      "charts":[\n'
    if chart_has_rows linter-projects; then l_files_trail=","; else l_files_trail=""; fi
    emit_chart linter-files "Single files" \
        "Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts." \
        "$l_files_trail"
    emit_chart linter-projects "Projects" \
        "Linting speed on real R packages (the $PROJECT_LIST source trees) relative to arity, on the same log-ratio axis." \
        ""
    printf '      ]\n    }\n'

    printf '  ]\n'
    printf '}\n'
} > "$JSON_OUT"

log ">> Wrote $JSON_OUT"
echo "$JSON_OUT"
