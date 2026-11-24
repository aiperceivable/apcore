"""Regression tests for batch mutation and failure attribution."""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from conformance import check_case_pinning as pinning


class BatchProbeTests(unittest.TestCase):
    def test_second_strategy_mutates_only_remaining_cases(self):
        cases = [
            {"id": "already_pinned", "expected": "ok"},
            {"id": "remaining", "expected": "reject"},
        ]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "fixture.json"
            backup = root / "backup.json"
            original = json.dumps({"test_cases": cases}) + "\n"
            path.write_text(original)
            backup.write_text(original)

            def aggregate_driver(*_):
                # Like a Rust aggregate test, the driver stops at its first
                # failing assertion. Re-mutating a pinned case hides the next.
                for case, pristine in zip(json.loads(path.read_text())["test_cases"], cases):
                    if case["expected"] != pristine["expected"]:
                        return True, f"failures:\nFAIL [{case['id']}]"
                return False, "test result: ok. 1 passed"

            with patch.object(pinning, "run_drivers_verbose", side_effect=aggregate_driver):
                found = pinning.batch_probe(
                    "rust", root, [], path, backup, cases[1:], "", "append"
                )

            self.assertEqual(found, {"remaining"})
            self.assertEqual(path.read_text(), original)

    def test_batch_restores_fixture_when_driver_raises(self):
        cases = [{"id": "case", "expected": True}]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "fixture.json"
            backup = root / "backup.json"
            original = json.dumps({"test_cases": cases}) + "\n"
            path.write_text(original)
            backup.write_text(original)
            with patch.object(pinning, "run_drivers_verbose", side_effect=RuntimeError("driver")):
                with self.assertRaisesRegex(RuntimeError, "driver"):
                    pinning.batch_probe("rust", root, [], path, backup, cases, "", "append")
            self.assertEqual(path.read_text(), original)


class IndeterminateRunTests(unittest.TestCase):
    def run_cli(self, driver_results, *flags):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "sample.json"
            original = json.dumps({"test_cases": [{"id": "sample", "expected": True}]})
            path.write_text(original)
            drivers = {"sample": {"python": [root / "test_sample.py"]}}
            argv = ["check_case_pinning.py", "--sdk", "python", *flags]
            with (
                patch.object(pinning, "REPO", root),
                patch.object(pinning, "FIXTURES", root),
                patch.object(pinning, "BASELINE", root / "baseline.json"),
                patch.object(pinning, "driver_files", return_value=drivers),
                patch.object(pinning, "run_drivers_verbose", side_effect=driver_results),
                patch.object(pinning.signal, "signal"),
                patch.object(sys, "argv", argv),
            ):
                result = pinning.main()
            self.assertEqual(path.read_text(), original)
            self.assertFalse((root / "baseline.json").exists())
            return result

    def test_red_baseline_fails_even_when_reporting_for_the_ci_ratchet(self):
        self.assertNotEqual(self.run_cli([(True, "failed before mutation")]), 0)

    def test_strict_run_fails_when_no_baseline_tests_execute(self):
        self.assertNotEqual(self.run_cli([pinning.NoTestsRan("empty selection")], "--strict"), 0)

    def test_lost_driver_during_mutation_fails_and_does_not_write_a_baseline(self):
        results = [(False, "one passing test"), pinning.NoTestsRan("lost driver")]
        self.assertNotEqual(self.run_cli(results, "--per-case", "--write-baseline"), 0)


if __name__ == "__main__":
    unittest.main()
