"""PlatformNotifyMiddleware -- threshold sensor with hysteresis (PRD F8)."""

from __future__ import annotations

import threading
from collections import defaultdict
from datetime import datetime, timezone
from typing import Any

from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.middleware.base import Context, Middleware
from apcore.observability.metrics import MetricsCollector, module_error_rate, module_latency_ms


class PlatformNotifyMiddleware(Middleware):
    """Monitors error rates and latency, emits threshold events with hysteresis.

    Emits ``error_threshold_exceeded`` when a module's error rate crosses the
    configured threshold, ``latency_threshold_exceeded`` when p99 latency
    exceeds the limit, and ``apcore.health.recovered`` when a previously alerted
    module recovers below ``threshold * 0.5``.

    Hysteresis prevents repeated alerts until recovery is observed.
    """

    def __init__(
        self,
        event_emitter: EventEmitter,
        metrics_collector: MetricsCollector | None = None,
        error_rate_threshold: float = 0.1,
        latency_p99_threshold_ms: float = 5000.0,
    ) -> None:
        self._emitter = event_emitter
        self._metrics_collector = metrics_collector
        self._error_rate_threshold = error_rate_threshold
        self._latency_p99_threshold_ms = latency_p99_threshold_ms
        self._alert_lock = threading.Lock()
        self._alerted: dict[str, set[str]] = defaultdict(set)

    # ------------------------------------------------------------------
    # Middleware hooks
    # ------------------------------------------------------------------

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
        """No-op before hook."""
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> dict[str, Any] | None:
        """Check latency threshold and error-rate recovery after execution."""
        self._check_latency_threshold(module_id)
        self._check_error_recovery(module_id)
        return None

    def on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context,
    ) -> dict[str, Any] | None:
        """Check error rate threshold on error."""
        self._check_error_rate_threshold(module_id)
        return None

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _compute_error_rate(self, module_id: str) -> float:
        """Compute the error rate for a module from MetricsCollector counters.

        Delegates to the canonical snapshot extractor so this sensor and the
        ``system.health`` sys module cannot drift apart on which counters they
        read — the same split in apcore-rust had the second copy matching the
        wrong label key, pinning its error rate at 0 forever.
        """
        if self._metrics_collector is None:
            return 0.0
        return module_error_rate(self._metrics_collector, module_id)

    def _estimate_p99_ms(self, module_id: str) -> float:
        """Estimate the p99 latency in milliseconds from histogram buckets."""
        if self._metrics_collector is None:
            return 0.0
        return module_latency_ms(self._metrics_collector, module_id)[1]

    def _check_error_rate_threshold(self, module_id: str) -> None:
        """Emit ``apcore.health.error_threshold_exceeded`` if the rate is above threshold."""
        error_rate = self._compute_error_rate(module_id)
        with self._alert_lock:
            if error_rate >= self._error_rate_threshold and "error_rate" not in self._alerted[module_id]:
                ts = datetime.now(timezone.utc).isoformat()
                payload = {"error_rate": error_rate, "threshold": self._error_rate_threshold}
                self._emitter.emit(
                    ApCoreEvent(
                        event_type="apcore.health.error_threshold_exceeded",
                        module_id=module_id,
                        timestamp=ts,
                        severity="error",
                        data=dict(payload),
                    )
                )
                self._alerted[module_id].add("error_rate")

    def _check_latency_threshold(self, module_id: str) -> None:
        """Emit ``apcore.health.latency_threshold_exceeded`` if p99 is above threshold."""
        p99_ms = self._estimate_p99_ms(module_id)
        with self._alert_lock:
            if p99_ms >= self._latency_p99_threshold_ms and "latency" not in self._alerted[module_id]:
                ts = datetime.now(timezone.utc).isoformat()
                payload = {"p99_latency_ms": p99_ms, "threshold": self._latency_p99_threshold_ms}
                self._emitter.emit(
                    ApCoreEvent(
                        event_type="apcore.health.latency_threshold_exceeded",
                        module_id=module_id,
                        timestamp=ts,
                        severity="warn",
                        data=dict(payload),
                    )
                )
                self._alerted[module_id].add("latency")

    def _check_error_recovery(self, module_id: str) -> None:
        """Emit ``apcore.health.recovered`` (canonical event) when a previously alerted module recovers."""
        error_rate = self._compute_error_rate(module_id)
        with self._alert_lock:
            if "error_rate" not in self._alerted.get(module_id, set()):
                return
            if error_rate < self._error_rate_threshold * 0.5:
                self._emitter.emit(
                    ApCoreEvent(
                        event_type="apcore.health.recovered",
                        module_id=module_id,
                        timestamp=datetime.now(timezone.utc).isoformat(),
                        severity="info",
                        data={"status": "recovered", "error_rate": error_rate},
                    )
                )
                self._alerted[module_id].discard("error_rate")


__all__ = ["PlatformNotifyMiddleware"]
