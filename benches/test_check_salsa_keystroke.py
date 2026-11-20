import json
from pathlib import Path
import tempfile
import unittest

from check_salsa_keystroke import CASES, check


class KeystrokeGateTests(unittest.TestCase):
    def setUp(self):
        directory = self.enterContext(tempfile.TemporaryDirectory())
        self.baseline = Path(directory) / "baseline"
        self.candidate = Path(directory) / "candidate"
        for case in CASES:
            self.write(self.baseline, case, 100)
            self.write(self.candidate, case, 100)

    def write(self, root, case, median):
        path = root / case / "new" / "estimates.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"median": {"point_estimate": median}}))
        return path

    def test_improvement_with_stable_controls_passes(self):
        self.write(self.candidate, "write_phase/130k", 103)
        self.write(self.candidate, "keystroke/130k", 91)
        self.assertTrue(check(self.baseline, self.candidate, emit=lambda _: None))

    def test_regression_and_control_drift_fail(self):
        self.write(self.candidate, "keystroke/130k", 109)
        self.write(self.candidate, "upsert_unchanged/1m", 91)
        messages = []
        self.assertFalse(check(self.baseline, self.candidate, emit=messages.append))
        for case in ("keystroke/130k", "upsert_unchanged/1m"):
            self.assertTrue(any(message.startswith(f"FAIL {case}:") for message in messages))

    def test_missing_measurement_in_either_run_fails(self):
        for root in (self.baseline, self.candidate):
            case = "keystroke/1m"
            path = self.write(root, case, 100)
            path.unlink()
            with self.subTest(root=root):
                self.assertFalse(check(self.baseline, self.candidate, emit=lambda _: None))
            self.write(root, case, 100)

    def test_invalid_measurement_fails(self):
        for median in (0, -1, float("nan"), float("inf"), "100"):
            with self.subTest(median=median):
                self.write(self.candidate, "keystroke/1m", median)
                self.assertFalse(check(self.baseline, self.candidate, emit=lambda _: None))


if __name__ == "__main__":
    unittest.main()
