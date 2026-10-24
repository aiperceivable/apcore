"""Regression tests for batch mutation and failure attribution."""

import json
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


if __name__ == "__main__":
    unittest.main()
