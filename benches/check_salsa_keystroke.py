"""Compare two complete Criterion runs of the salsa_keystroke benchmark."""

import argparse
import json
import math
from pathlib import Path

# Controls must stay within the drift budget in either direction. A faster
# end-to-end keystroke passes as long as the controls stay stable.
CASES = {
    "upsert_unchanged/130k": (0.95, 1.05),
    "upsert_unchanged/1m": (0.95, 1.05),
    "write_phase/130k": (0.95, 1.05),
    "write_phase/1m": (0.95, 1.05),
    "keystroke/130k": (0.0, 1.05),
    "keystroke/1m": (0.0, 1.05),
}


def median(root, case):
    path = root / case / "new" / "estimates.json"
    try:
        value = json.loads(path.read_text())["median"]["point_estimate"]
        if type(value) not in (float, int) or not math.isfinite(value) or value <= 0:
            raise ValueError("expected a positive finite median")
    except (OSError, ValueError, KeyError, TypeError, OverflowError) as error:
        raise ValueError(f"{path}: {error}") from error
    return value


def check(baseline, candidate, emit=print):
    passed = True
    for case, (minimum, maximum) in CASES.items():
        try:
            ratio = median(candidate, case) / median(baseline, case)
        except ValueError as error:
            emit(f"FAIL {case}: {error}")
            passed = False
            continue
        in_budget = minimum <= ratio <= maximum
        verdict = "PASS" if in_budget else "FAIL"
        budget = f"{minimum:.2f}x–{maximum:.2f}x" if minimum else f"≤ {maximum:.2f}x"
        emit(f"{verdict} {case}: {ratio:.4f}x baseline (expected {budget})")
        passed &= in_budget
    return passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path, help="Criterion output directory for the baseline")
    parser.add_argument("candidate", type=Path, help="Criterion output directory for the candidate")
    args = parser.parse_args()
    if args.baseline.resolve() == args.candidate.resolve():
        parser.error("baseline and candidate must be separate Criterion output directories")
    return 0 if check(args.baseline, args.candidate) else 1


if __name__ == "__main__":
    raise SystemExit(main())
