#!/usr/bin/env bash
#
# Sample one phase of arity's work with `perf` and print where the time goes.
#
# The target is `benches/profile.rs`, a plain binary that runs one phase in a
# loop, so the profile is of the library rather than of process startup. The
# script builds it under the `profiling` cargo profile (release codegen plus
# debug info, no strip) with frame pointers forced on, records it, and prints
# three views of the result:
#
#   1. a PHASE SPLIT -- inclusive share of a curated set of subsystem roots.
#      Read this first: it says which phase to open.
#   2. TOP INCLUSIVE -- every symbol by inclusive share, so a hot phase can be
#      followed down to the function that owns it.
#   3. TOP SELF -- leaf self time. Read this last; it flatters the allocator
#      and rowan internals, which are symptoms, not fix sites.
#
# Like `scripts/bench.sh`, this is a *visibility* tool: opt-in, local, never a
# gate. Unlike it, nothing here is tracked -- a profile is a local observation
# of one machine on one day, so the artifacts land in target/profile/ and stay
# out of git.
#
# Usage:
#   ./scripts/profile.sh                                # format, 300 iters
#   ./scripts/profile.sh --mode parse                   # a different phase
#   ./scripts/profile.sh --mode lint --path pkg/R/x.R   # a file you care about
#   ./scripts/profile.sh --mode lint-dir --path pkg/R   # the rayon CLI path
#   ITERATIONS=2000 ./scripts/profile.sh                # more samples
#   FREQ=4999 ./scripts/profile.sh                      # denser sampling
#   TOP=40 ./scripts/profile.sh                         # longer symbol lists
#   REPORT_ONLY=1 ./scripts/profile.sh                  # re-read the last data
#   INLINE=1 ./scripts/profile.sh                       # expand inline frames
#                                                       # (blanks the phase table)
#
# `--mode` and `--path` are passed through to the target; see its --help for
# the mode list. Everything after `--` is passed through verbatim.
#
# Two settings are load-bearing, and hand-rolling a perf invocation will lose
# them:
#
#   * `-Cforce-frame-pointers=yes` plus `--call-graph fp`. Release codegen
#     omits frame pointers, so without this the callchains are truncated and
#     every inclusive view silently collapses into self time. `-g` after
#     `--call-graph dwarf` is not a fix: `-g` *is* `--call-graph fp` and
#     quietly overrides what precedes it.
#   * the `[profile.profiling]` section in Cargo.toml, which adds the debug
#     info release does not carry. Profiling a plain --release build resolves
#     fewer symbols and no inline frames at all.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${PROFILE_OUT:-target/profile}"
DATA="$OUT_DIR/profile.data"
FOLDED="$OUT_DIR/profile.folded"
SVG="$OUT_DIR/profile.svg"

ITERATIONS="${ITERATIONS:-}"
FREQ="${FREQ:-1999}"
TOP="${TOP:-25}"
# Symbols below this inclusive/self share are not worth printing.
MIN_PCT="${MIN_PCT:-0.5}"

log() { echo -e "$@" >&2; }

command -v perf >/dev/null 2>&1 || {
    echo "error: perf is not on PATH (devenv provides it: pkgs.perf)" >&2
    exit 1
}

# Everything after `--`, and every unrecognized flag, goes to the target.
declare -a TARGET_ARGS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help) sed -n '2,36p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;  # through the usage block
        --) shift; TARGET_ARGS+=("$@"); break ;;
        *) TARGET_ARGS+=("$1"); shift ;;
    esac
done
[ -n "$ITERATIONS" ] && TARGET_ARGS+=(--iterations "$ITERATIONS")

# --- Build and record --------------------------------------------------------

# REPORT_ONLY=1 re-reads the last recording: the analysis below is cheap to
# change, and re-recording would resample a different run.
if [ "${REPORT_ONLY:-0}" = "1" ]; then
    [ -f "$DATA" ] || { echo "error: no recording at $DATA" >&2; exit 1; }
    log ">> Re-reading $DATA (REPORT_ONLY=1)"
else
    log ">> Building the profiling target..."
    # RUSTFLAGS changes every crate's fingerprint, so the first run rebuilds the
    # world -- into target/profiling/, leaving the release cache alone.
    RUSTFLAGS="${RUSTFLAGS:-} -Cforce-frame-pointers=yes" \
        cargo build --profile profiling --bench profile --quiet

    BIN=$(find target/profiling/deps -maxdepth 1 -type f -name 'profile-*' ! -name '*.d' \
        -printf '%T@\t%p\n' | sort -rn | head -1 | cut -f2)
    [ -n "$BIN" ] || { echo "error: could not find the built profile binary" >&2; exit 1; }

    mkdir -p "$OUT_DIR"
    log ">> Recording (perf -F $FREQ, call-graph fp)..."
    perf record --quiet --call-graph fp -F "$FREQ" -o "$DATA" -- "$BIN" "${TARGET_ARGS[@]}"
    log ">> Recorded to $DATA"
fi

# --- Analyze -----------------------------------------------------------------

# One pass over the raw stacks computes all three views. Each sample is weighted
# by its period (the cycles it stands for), which is what makes a frequency-mode
# recording proportional to time. A symbol is counted *once* per sample for the
# inclusive view, so recursion does not inflate it.
#
# The phase table is the curated part: label, then a regex matched against every
# frame of a sample. Extend it as subsystems get interesting -- it is a reading
# aid, not a contract.
PHASES="\
parse|arity_parser::parser::core::parse
 lex|arity_parser::parser::lexer
 parse_expr|arity_parser::parser::expr
 structural|arity_parser::parser::structural
 roxygen|arity_parser::parser::roxygen
 build_tree|arity_parser::parser::tree_builder
 reparse|arity_parser::parser::reparse
format|arity_formatter::formatter
 lower|arity_formatter::formatter::rules
 print|arity_formatter::formatter::printer
 render|arity_formatter::formatter::render
 trivia|arity_formatter::formatter::trivia
semantic|arity::semantic
lint_rules|arity::linter::rules
project|arity::project
salsa|salsa::
rowan|rowan::
allocator|^(mi_|_int_malloc|_int_free|malloc|free|cfree|realloc|calloc)"

# `--no-inline` by default, and it is not a detail: with inline expansion on,
# perf renames *every* frame to its short source name, so `format_with_options`
# loses its module path and the phase table (which matches on module paths)
# silently reports nothing. `INLINE=1` turns expansion back on when the question
# is which inlined helper inside a function is hot.
INLINE_FLAG="--no-inline"
[ "${INLINE:-0}" = "1" ] && INLINE_FLAG=""

# shellcheck disable=SC2086 # deliberate: empty means "no flag"
perf script -i "$DATA" $INLINE_FLAG -F comm,period,ip,sym,dso 2>/dev/null |
    awk -v phases="$PHASES" -v top="$TOP" -v min_pct="$MIN_PCT" '
    function flush_sample(   sym, label) {
        if (n_frames == 0) return
        total += period; n_samples++
        self[frames[1]] += period
        for (sym in seen) incl[sym] += period
        for (label in phase_re) {
            for (sym in seen) {
                if (sym ~ phase_re[label]) { phase[label] += period; break }
            }
        }
        delete seen; n_frames = 0
    }
    BEGIN {
        # Split on the *first* "|" only: a phase regex may contain more.
        n = split(phases, lines, "\n")
        for (i = 1; i <= n; i++) {
            bar = index(lines[i], "|")
            raw = substr(lines[i], 1, bar - 1)
            label = raw; sub(/^ +/, "", label)
            depth[label] = (raw ~ /^ /) ? 1 : 0
            phase_re[label] = substr(lines[i], bar + 1)
            order[i] = label
        }
        n_phases = n
    }
    # Header line of a sample: "comm  <period> <event>: ".
    /^[^\t ]/ {
        flush_sample()
        period = $2 + 0
        if (period <= 0) period = 1
        next
    }
    # A frame: "\t<addr> <symbol> (<dso>)". The symbol may contain spaces
    # (generic parameters), so strip from both ends rather than taking a field.
    /^\t/ {
        line = $0
        sub(/^\t[ ]*[0-9a-fA-F]+ /, "", line)
        sub(/ \([^)]*\)$/, "", line)
        if (line == "[unknown]" || line == "") next
        n_frames++
        frames[n_frames] = line
        seen[line] = 1
        next
    }
    END {
        flush_sample()
        if (total == 0) { print "no samples"; exit 1 }

        printf "\n%d samples, %.2f Gcycles\n", n_samples, total / 1e9
        print ""
        print "=== PHASE SPLIT (inclusive share of total) ==="
        for (i = 1; i <= n_phases; i++) {
            label = order[i]
            if (!(label in phase)) continue
            pct = 100 * phase[label] / total
            if (pct < min_pct) continue
            printf "%s%-22s %6.1f%%\n", (depth[label] ? "  " : ""), label, pct
        }

        print ""
        printf "=== TOP %d INCLUSIVE (share of total, deduped per sample) ===\n", top
        for (sym in incl) {
            # Every sample sits under the runtime prologue; listing it at 100%
            # says nothing about arity.
            if (sym ~ /^(_start|main|__libc_|std::rt::|std::sys::backtrace::|core::hint::black_box|profile::main)/) continue
            inc_pct[sym] = 100 * incl[sym] / total
        }
        show(inc_pct, top, min_pct)

        print ""
        printf "=== TOP %d SELF (leaf time) ===\n", top
        for (sym in self) self_pct[sym] = 100 * self[sym] / total
        show(self_pct, top, min_pct)
    }
    function show(tbl, limit, floor,   sym, i, k, best, best_sym) {
        for (sym in tbl) k++
        for (i = 1; i <= limit && i <= k; i++) {
            best = -1; best_sym = ""
            for (sym in tbl) if (tbl[sym] > best) { best = tbl[sym]; best_sym = sym }
            if (best < floor) break
            printf "%6.1f%%  %s\n", best, best_sym
            delete tbl[best_sym]
        }
    }'

# --- Flamegraph (best effort) ------------------------------------------------

if command -v flamegraph >/dev/null 2>&1; then
    if flamegraph --perfdata "$DATA" -o "$SVG" >/dev/null 2>&1; then
        log "\n>> Flamegraph: $SVG"
    else
        log "\n>> (flamegraph failed; the raw data is still at $DATA)"
    fi
elif command -v inferno-collapse-perf >/dev/null 2>&1; then
    perf script -i "$DATA" | inferno-collapse-perf >"$FOLDED"
    inferno-flamegraph "$FOLDED" >"$SVG" && log "\n>> Flamegraph: $SVG"
fi

log ">> Raw data: $DATA  (perf report -i $DATA -g graph,caller)"
