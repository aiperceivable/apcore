"""Tests for PlatformNotifyMiddleware threshold sensor with hysteresis."""

from __future__ import annotations

from unittest.mock import MagicMock

from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.middleware.base import Middleware
from apcore.middleware.platform_notify import PlatformNotifyMiddleware
from apcore.observability.metrics import MetricsCollector


def _make_metrics_with_error_rate(module_id: str, total_calls: int, error_calls: int) -> MetricsCollector:
    """Create a MetricsCollector pre-loaded with call/error counts for a module."""
    mc = MetricsCollector()
    labels_ok = {"module_id": module_id, "status": "success"}
    labels_err = {"module_id": module_id, "status": "error"}
    error_labels = {"module_id": module_id, "error_code": "SOME_ERROR"}
    success_count = total_calls - error_calls
    for _ in range(success_count):
        mc.increment("apcore_module_calls_total", labels_ok)
    for _ in range(error_calls):
        mc.increment("apcore_module_calls_total", labels_err)
        mc.increment("apcore_module_errors_total", error_labels)
    return mc


def _add_latency_observations(mc: MetricsCollector, module_id: str, values: list[float]) -> None:
    """Add duration observations in seconds to a MetricsCollector."""
    for v in values:
        mc.observe("apcore_module_duration_seconds", {"module_id": module_id}, v)


class TestPlatformNotifyMiddleware:
    def test_constructor_accepts_dependencies(self) -> None:
        emitter = EventEmitter()
        mc = MetricsCollector()
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.2,
            latency_p99_threshold_ms=3000.0,
        )
        assert isinstance(mw, Middleware)

    def test_on_error_emits_error_threshold_exceeded(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        # 15% error rate (above 0.1 threshold)
        mc = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=15)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.1,
        )
        mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())
        # Canonical-only emission per the v0.22.0 spec MUST (apcore#78).
        assert emitter.emit.call_count == 1
        canonical = [
            c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.health.error_threshold_exceeded"
        ]
        assert len(canonical) == 1
        event: ApCoreEvent = canonical[0][0][0]
        assert event.module_id == "mod.a"
        assert event.severity == "error"
        assert "error_rate" in event.data
        assert "threshold" in event.data

    def test_on_error_no_emit_below_threshold(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        # 5% error rate (below 0.1 threshold)
        mc = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=5)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.1,
        )
        mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())
        emitter.emit.assert_not_called()

    def test_on_error_hysteresis_no_re_alert(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        mc = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=15)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.1,
        )
        mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())
        mw.on_error("mod.a", {}, RuntimeError("boom2"), MagicMock())
        # Hysteresis: only one threshold event despite two on_error calls (apcore#78).
        assert emitter.emit.call_count == 1

    def test_on_error_returns_none(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        mc = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=15)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.1,
        )
        result = mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())
        assert result is None

    def test_after_emits_latency_threshold_exceeded(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        mc = MetricsCollector()
        # Add observations where p99 > 5000ms (5s)
        # 95 fast + 5 slow (>5s) so the p99 bucket boundary exceeds 5s
        _add_latency_observations(mc, "mod.b", [0.1] * 95 + [6.0] * 5)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            latency_p99_threshold_ms=5000.0,
        )
        mw.after("mod.b", {}, {}, MagicMock())
        # Should emit the canonical apcore.health.latency_threshold_exceeded (apcore#78)
        calls = [
            c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.health.latency_threshold_exceeded"
        ]
        assert len(calls) == 1
        event: ApCoreEvent = calls[0][0][0]
        assert event.module_id == "mod.b"
        assert event.severity == "warn"
        assert "p99_latency_ms" in event.data
        assert "threshold" in event.data

    def test_after_no_emit_below_latency_threshold(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        mc = MetricsCollector()
        # All fast observations - p99 well below 5s
        _add_latency_observations(mc, "mod.b", [0.1] * 100)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            latency_p99_threshold_ms=5000.0,
        )
        mw.after("mod.b", {}, {}, MagicMock())
        emitter.emit.assert_not_called()

    def test_after_emits_recovery_event(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        # Start with high error rate to trigger alert
        mc = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=15)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.1,
        )
        # Trigger the error alert (canonical-only, apcore#78)
        mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())
        assert emitter.emit.call_count == 1

        # Now replace metrics with low error rate (< threshold * 0.5 = 0.05)
        mc2 = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=3)
        mw._metrics_collector = mc2

        emitter.reset_mock()
        mw.after("mod.a", {}, {}, MagicMock())
        # Should emit apcore.health.recovered with status=recovered.
        # Legacy 'module_health_changed' alias removed in v0.18.0.
        recovery_calls = [c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.health.recovered"]
        assert len(recovery_calls) == 1
        event: ApCoreEvent = recovery_calls[0][0][0]
        assert event.data["status"] == "recovered"
        assert event.module_id == "mod.a"

    def test_after_recovery_clears_alert_flag(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        # High error rate
        mc = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=15)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.1,
        )
        # Trigger alert (canonical-only, apcore#78)
        mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())
        assert emitter.emit.call_count == 1

        # Recover
        mc2 = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=3)
        mw._metrics_collector = mc2
        mw.after("mod.a", {}, {}, MagicMock())

        # Now trigger again with high error rate - should emit again since flag was cleared
        mc3 = _make_metrics_with_error_rate("mod.a", total_calls=100, error_calls=20)
        mw._metrics_collector = mc3
        emitter.reset_mock()
        mw.on_error("mod.a", {}, RuntimeError("boom3"), MagicMock())
        # Canonical-only emission (apcore#78)
        assert emitter.emit.call_count == 1
        canonical = [
            c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.health.error_threshold_exceeded"
        ]
        assert len(canonical) == 1

    def test_after_returns_none(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        mc = MetricsCollector()
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
        )
        result = mw.after("mod.a", {}, {}, MagicMock())
        assert result is None

    def test_before_returns_none(self) -> None:
        emitter = MagicMock(spec=EventEmitter)
        mc = MetricsCollector()
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
        )
        result = mw.before("mod.a", {}, MagicMock())
        assert result is None

    def test_after_no_crash_without_metrics_collector(self) -> None:
        """after() should be a no-op when metrics_collector is None, not crash."""
        emitter = MagicMock(spec=EventEmitter)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=None,  # type: ignore[arg-type]
        )
        result = mw.after("mod.a", {}, {}, MagicMock())
        assert result is None
        emitter.emit.assert_not_called()

    def test_on_error_no_crash_without_metrics_collector(self) -> None:
        """on_error() should be a no-op when metrics_collector is None, not crash."""
        emitter = MagicMock(spec=EventEmitter)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=None,  # type: ignore[arg-type]
        )
        result = mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())
        assert result is None
        emitter.emit.assert_not_called()
