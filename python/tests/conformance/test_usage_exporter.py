"""Drive `usage_exporter.json` — the push-style UsageExporter contract (#45 §3, D-55).

The canonical fixture states three obligations for every SDK:

1. ``NoopUsageExporter`` is the default and drops summaries without side effects.
2. ``PeriodicUsageExporter.start()`` polls ``UsageCollector`` at the configured
   interval and calls ``exporter.export(summary)`` once per tick.
3. ``PeriodicUsageExporter.stop()`` halts the loop, calls ``exporter.shutdown()``
   exactly once, and is idempotent.

``tests/observability/test_usage_exporter.py`` already asserts most of this by
hand.  That copy cannot notice when the canonical fixture gains a case, which is
why this driver reads the fixture itself and fails on an unknown case id.

Timing note: case 2 declares ``interval_seconds: 0.1`` and ``ticks: 3``.  Wall
clock is not reproducible, so this driver polls until the exporter has produced
the fixture's tick count and fails if that has not happened inside a generous
budget.  That still asserts the fixture's claim — a one-shot or never-firing
loop cannot reach three exports at any deadline — without making the suite
flaky on a loaded machine.
"""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.observability import (
    NoopUsageExporter,
    PeriodicUsageExporter,
    UsageCollector,
    UsageExporter,
)

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("usage_exporter.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}

# Name given to the background task by PeriodicUsageExporter.start().  Used to
# assert the loop is really gone after stop() rather than merely dereferenced.
_TASK_NAME = "apcore-periodic-usage-exporter"


class _RecordingExporter:
    """UsageExporter double that records every export/shutdown call."""

    def __init__(self) -> None:
        self.exports: list[dict[str, Any]] = []
        self.shutdown_count = 0

    def export(self, summary: dict[str, Any]) -> None:
        self.exports.append(summary)

    def shutdown(self) -> None:
        self.shutdown_count += 1


def _live_exporter_tasks() -> list[asyncio.Task[Any]]:
    return [t for t in asyncio.all_tasks() if t.get_name() == _TASK_NAME and not t.done()]


async def _wait_for_exports(recorder: _RecordingExporter, count: int, budget_s: float) -> None:
    """Poll until *recorder* has seen *count* exports or *budget_s* elapses."""
    deadline = asyncio.get_running_loop().time() + budget_s
    while len(recorder.exports) < count and asyncio.get_running_loop().time() < deadline:
        await asyncio.sleep(0.01)


class TestUsageExporterFixture:
    """usage_exporter.json — Noop default, periodic push, idempotent stop."""

    def test_every_fixture_case_has_a_driver(self) -> None:
        """A case added to the canonical fixture must fail here, not pass silently."""
        driven = {
            "noop_usage_exporter_drops_summary",
            "periodic_usage_exporter_pushes_at_interval",
            "periodic_usage_exporter_stop_is_idempotent_and_drains",
        }
        assert set(CASES) == driven, (
            f"usage_exporter.json cases without a driver here: {sorted(set(CASES) - driven)}; "
            f"drivers with no matching case: {sorted(driven - set(CASES))}"
        )

    def test_noop_usage_exporter_drops_summary(self) -> None:
        case = CASES["noop_usage_exporter_drops_summary"]
        expected = case["expected"]

        exporter = NoopUsageExporter()
        assert isinstance(
            exporter, UsageExporter
        ), f"[{case['id']}] NoopUsageExporter must satisfy the UsageExporter protocol"

        observed: list[Any] = []
        errors: list[str] = []
        shutdown_completed = False
        for op in case["operations"]:
            try:
                if op["op"] == "export":
                    result = exporter.export({"modules": op["summary"]})
                    assert result is None, f"[{case['id']}] export() must return None, got {result!r}"
                elif op["op"] == "shutdown":
                    exporter.shutdown()
                    shutdown_completed = True
                else:  # pragma: no cover - guards against a fixture op this driver cannot express
                    pytest.fail(f"[{case['id']}] unhandled fixture op {op['op']!r}")
            except Exception as exc:  # noqa: BLE001 - the fixture asserts no errors escape
                errors.append(f"{type(exc).__name__}: {exc}")

        assert (
            observed == expected["calls_observed"]
        ), f"[{case['id']}] NoopUsageExporter must have no observable side effects"
        assert shutdown_completed is expected["shutdown_completed"], f"[{case['id']}] shutdown_completed mismatch"
        assert errors == expected["errors"], f"[{case['id']}] unexpected errors: {errors}"

    async def test_periodic_usage_exporter_pushes_at_interval(self) -> None:
        case = CASES["periodic_usage_exporter_pushes_at_interval"]
        interval = float(case["config"]["interval_seconds"])
        ticks = int(case["config"]["ticks"])
        expected = case["expected"]

        collector = UsageCollector()
        for rec in case["usage_records"]:
            collector.record(
                module_id=rec["module_id"],
                caller_id=rec["caller_id"],
                latency_ms=rec["latency_ms"],
                success=rec["success"],
            )

        recorder = _RecordingExporter()
        periodic = PeriodicUsageExporter(collector, recorder, interval_seconds=interval)
        await periodic.start()
        try:
            # Budget: 10x the nominal window so scheduler jitter cannot fail the run,
            # while a loop that fires once (or never) still cannot reach `ticks`.
            await _wait_for_exports(recorder, expected["export_call_count"], budget_s=interval * ticks * 10)
        finally:
            await periodic.stop()

        assert len(recorder.exports) >= expected["export_call_count"], (
            f"[{case['id']}] expected at least {expected['export_call_count']} export() calls at a "
            f"{interval}s interval, got {len(recorder.exports)} — the periodic loop is not ticking"
        )

        needle = expected["each_export_summary_includes"]
        for i, payload in enumerate(recorder.exports):
            assert needle in str(payload), (
                f"[{case['id']}] export #{i} payload does not include the recorded module " f"{needle!r}: {payload!r}"
            )

        # Bound to the OBSERVATION: `assert expected[...] is True` restated the
        # fixture and could not fail on SDK behaviour (apcore-python#32 /
        # aiperceivable/apcore#81).
        shutdown_completed = recorder.shutdown_count == 1
        assert shutdown_completed is expected["shutdown_completed_after_stop"], (
            f"[{case['id']}] shutdown_completed_after_stop: stop() must call exporter.shutdown() "
            f"exactly once, got {recorder.shutdown_count}"
        )

    async def test_periodic_usage_exporter_stop_is_idempotent_and_drains(self) -> None:
        case = CASES["periodic_usage_exporter_stop_is_idempotent_and_drains"]
        interval = float(case["config"]["interval_seconds"])
        expected = case["expected"]

        collector = UsageCollector()
        recorder = _RecordingExporter()
        periodic = PeriodicUsageExporter(collector, recorder, interval_seconds=interval)

        errors: list[str] = []
        for op in case["operations"]:
            try:
                if op["op"] == "start":
                    await periodic.start()
                elif op["op"] == "wait_ms":
                    await asyncio.sleep(op["duration_ms"] / 1000.0)
                elif op["op"] == "stop":
                    await periodic.stop()
                else:  # pragma: no cover - guards against a fixture op this driver cannot express
                    pytest.fail(f"[{case['id']}] unhandled fixture op {op['op']!r}")
            except Exception as exc:  # noqa: BLE001 - the fixture asserts no errors escape
                errors.append(f"{type(exc).__name__}: {exc}")

        assert (
            errors == expected["errors"]
        ), f"[{case['id']}] stop_idempotent: a repeated stop() must not raise, got {errors}"
        assert recorder.shutdown_count == expected["shutdown_call_count"], (
            f"[{case['id']}] shutdown must run exactly {expected['shutdown_call_count']} time(s) "
            f"across two stop() calls, got {recorder.shutdown_count}"
        )
        # Bound to the OBSERVATION: stop() is idempotent iff the repeated call
        # raised nothing and did not re-run shutdown.
        stop_idempotent = not errors and recorder.shutdown_count == expected["shutdown_call_count"]
        assert stop_idempotent is expected["stop_idempotent"], (
            f"[{case['id']}] stop_idempotent: errors={errors}, " f"shutdown_count={recorder.shutdown_count}"
        )

        # background_task_terminated: the loop task must actually be gone, not
        # merely unreferenced — a leaked task would keep exporting.
        assert (
            _live_exporter_tasks() == []
        ), f"[{case['id']}] background_task_terminated: {_TASK_NAME} still running after stop()"
        before = len(recorder.exports)
        await asyncio.sleep(interval * 3)
        assert (
            len(recorder.exports) == before
        ), f"[{case['id']}] export() fired after stop(): {before} → {len(recorder.exports)}"
