"""CircuitBreakerMiddleware — rolling-window per-(module_id, caller_id) breaker (Issue #42).

Spec reference: ``features/middleware-system.md`` § "CircuitBreakerMiddleware".

Tracks per-(``module_id``, ``caller_id``) outcomes over a bounded rolling window.
When the error rate in the window meets or exceeds ``open_threshold`` (and at
least ``min_samples`` outcomes are recorded), the circuit transitions to
``OPEN`` and short-circuits subsequent calls with
:class:`~apcore.errors.CircuitBreakerOpenError`. After ``recovery_window_ms``
elapses the circuit moves to ``HALF_OPEN`` and admits exactly one probe call;
a successful probe closes the circuit, a failed probe re-opens it.

State machine per (``module_id``, ``caller_id``)::

    CLOSED    → (error_rate >= open_threshold and samples >= min_samples) → OPEN
    OPEN      → (recovery_window_ms elapsed)                              → HALF_OPEN
    HALF_OPEN → (probe success)  → CLOSED  (emits apcore.circuit.closed)
    HALF_OPEN → (probe failure)  → OPEN    (emits apcore.circuit.opened)
"""

from __future__ import annotations

import logging
import threading
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from enum import Enum
from typing import Any, Callable

from apcore.errors import CircuitBreakerOpenError
from apcore.middleware.base import Context, Middleware, RetrySignal

__all__ = ["CircuitBreakerState", "CircuitBreakerMiddleware", "CTX_CIRCUIT_STATE"]

_logger = logging.getLogger(__name__)

#: Context-data key (framework namespace) holding the current circuit state.
#: Identical literal across the Python/TypeScript/Rust SDKs.
CTX_CIRCUIT_STATE = "_apcore.mw.circuit.state"


class CircuitBreakerState(Enum):
    CLOSED = "CLOSED"
    OPEN = "OPEN"
    HALF_OPEN = "HALF_OPEN"


@dataclass
class _CircuitRecord:
    """Per-(module_id, caller_id) circuit state."""

    state: CircuitBreakerState = CircuitBreakerState.CLOSED
    #: Rolling window of recent outcomes — ``True`` = success, ``False`` = error.
    window: deque[bool] = field(default_factory=deque)
    opened_at: datetime | None = None
    probe_in_flight: bool = False
    lock: threading.Lock = field(default_factory=threading.Lock, compare=False, repr=False)

    def error_rate(self) -> float:
        if not self.window:
            return 0.0
        errors = sum(1 for ok in self.window if not ok)
        return errors / len(self.window)


class CircuitBreakerMiddleware(Middleware):
    """Rolling-window circuit breaker keyed by ``(module_id, caller_id)``.

    When the circuit is OPEN the :meth:`before` hook raises
    :class:`~apcore.errors.CircuitBreakerOpenError`, short-circuiting module
    execution entirely. State transitions emit ``apcore.circuit.opened`` and
    ``apcore.circuit.closed`` via the injected ``emitter`` (when provided), and
    the current state string is written to
    ``context.data["_apcore.mw.circuit.state"]`` on every call.
    """

    def __init__(
        self,
        *,
        open_threshold: float = 0.5,
        recovery_window_ms: int = 30_000,
        window_size: int = 20,
        min_samples: int = 5,
        emitter: Any | None = None,
        priority: int = 100,
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        super().__init__(priority=priority)
        if not (0.0 <= open_threshold <= 1.0):
            raise ValueError(f"open_threshold must be in [0.0, 1.0], got {open_threshold}")
        if recovery_window_ms < 0:
            raise ValueError(f"recovery_window_ms must be >= 0, got {recovery_window_ms}")
        if window_size < 1:
            raise ValueError(f"window_size must be >= 1, got {window_size}")
        if min_samples < 1:
            raise ValueError(f"min_samples must be >= 1, got {min_samples}")
        if min_samples > window_size:
            _logger.warning(
                "min_samples (%d) exceeds window_size (%d); clamping min_samples to window_size. "
                "Otherwise the breaker could never open.",
                min_samples,
                window_size,
            )
            min_samples = window_size

        self._open_threshold = open_threshold
        self._recovery_window_ms = recovery_window_ms
        self._window_size = window_size
        self._min_samples = min_samples
        self._emitter = emitter
        self._clock: Callable[[], datetime] = clock or (lambda: datetime.now(timezone.utc))
        self._circuits: dict[tuple[str, str], _CircuitRecord] = {}
        self._registry_lock = threading.Lock()

    # ------------------------------------------------------------------
    # Public introspection
    # ------------------------------------------------------------------

    def get_state(self, module_id: str, caller_id: str | None = None) -> CircuitBreakerState:
        """Return the current circuit state for ``(module_id, caller_id)``.

        Applies a pending OPEN → HALF_OPEN recovery transition if the recovery
        window has elapsed, so the returned state reflects the live machine.
        """
        record = self._get_record((module_id, caller_id or ""))
        with record.lock:
            self._maybe_transition_to_half_open(record, module_id)
            return record.state

    def reset(self, module_id: str, caller_id: str | None = None) -> None:
        """Manually close the circuit for ``(module_id, caller_id)``."""
        record = self._get_record((module_id, caller_id or ""))
        with record.lock:
            record.state = CircuitBreakerState.CLOSED
            record.window.clear()
            record.opened_at = None
            record.probe_in_flight = False

    # ------------------------------------------------------------------
    # Middleware hooks
    # ------------------------------------------------------------------

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
        key = self._key_of(module_id, context)
        record = self._get_record(key)
        with record.lock:
            self._maybe_transition_to_half_open(record, module_id)

            if record.state == CircuitBreakerState.OPEN:
                self._write_state(context, CircuitBreakerState.OPEN)
                raise CircuitBreakerOpenError(module_id=module_id, caller_id=key[1] or None)

            if record.state == CircuitBreakerState.HALF_OPEN:
                if record.probe_in_flight:
                    self._write_state(context, CircuitBreakerState.OPEN)
                    raise CircuitBreakerOpenError(module_id=module_id, caller_id=key[1] or None)
                record.probe_in_flight = True

            self._write_state(context, record.state)
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> dict[str, Any] | None:
        key = self._key_of(module_id, context)
        record = self._get_record(key)
        emit_closed = False
        with record.lock:
            record.window.append(True)
            self._trim(record)
            if record.state == CircuitBreakerState.HALF_OPEN:
                record.state = CircuitBreakerState.CLOSED
                record.window.clear()
                record.opened_at = None
                record.probe_in_flight = False
                emit_closed = True
                _logger.info("Circuit CLOSED for module '%s' (caller '%s')", key[0], key[1])
        if emit_closed:
            self._emit("apcore.circuit.closed", key, 0.0, "info")
        return None

    def on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context,
    ) -> dict[str, Any] | RetrySignal | None:
        # Our own short-circuit rejections must not count as outcomes.
        if isinstance(error, CircuitBreakerOpenError):
            return None

        key = self._key_of(module_id, context)
        record = self._get_record(key)
        emit_opened = False
        rate = 0.0
        with record.lock:
            record.window.append(False)
            self._trim(record)
            rate = record.error_rate()

            if record.state == CircuitBreakerState.HALF_OPEN:
                opens = True
            elif record.state == CircuitBreakerState.CLOSED:
                opens = len(record.window) >= self._min_samples and rate >= self._open_threshold
            else:
                opens = False

            if opens:
                record.state = CircuitBreakerState.OPEN
                record.opened_at = self._clock()
                record.probe_in_flight = False
                emit_opened = True
                _logger.warning(
                    "Circuit OPEN for module '%s' (caller '%s') at error_rate %.3f",
                    key[0],
                    key[1],
                    rate,
                )
        if emit_opened:
            self._emit("apcore.circuit.opened", key, rate, "warn")
        return None

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _key_of(self, module_id: str, context: Context) -> tuple[str, str]:
        caller_id = getattr(context, "caller_id", None)
        return (module_id, caller_id or "")

    def _get_record(self, key: tuple[str, str]) -> _CircuitRecord:
        with self._registry_lock:
            record = self._circuits.get(key)
            if record is None:
                record = _CircuitRecord()
                self._circuits[key] = record
            return record

    def _trim(self, record: _CircuitRecord) -> None:
        """Evict oldest outcomes beyond ``window_size``. Caller holds record.lock."""
        while len(record.window) > self._window_size:
            record.window.popleft()

    def _maybe_transition_to_half_open(self, record: _CircuitRecord, module_id: str) -> None:
        """Transition OPEN → HALF_OPEN if recovery window has elapsed. Caller holds record.lock."""
        if record.state != CircuitBreakerState.OPEN or record.opened_at is None:
            return
        elapsed_ms = (self._clock() - record.opened_at).total_seconds() * 1000
        if elapsed_ms >= self._recovery_window_ms:
            record.state = CircuitBreakerState.HALF_OPEN
            record.probe_in_flight = False
            _logger.info("Circuit HALF_OPEN for module '%s' after recovery window", module_id)

    def _write_state(self, context: Context, state: CircuitBreakerState) -> None:
        data = getattr(context, "data", None)
        if isinstance(data, dict):
            data[CTX_CIRCUIT_STATE] = state.value

    def _emit(self, event_type: str, key: tuple[str, str], error_rate: float, severity: str) -> None:
        if self._emitter is None:
            return
        # Local import avoids a module-load-time cycle with apcore.events.
        from apcore.events.emitter import ApCoreEvent

        event = ApCoreEvent(
            event_type=event_type,
            module_id=key[0],
            timestamp=datetime.now(timezone.utc).isoformat(),
            severity=severity,
            data={
                "module_id": key[0],
                "caller_id": key[1] or None,
                "error_rate": error_rate,
            },
        )
        try:
            self._emitter.emit(event)
        except Exception:  # noqa: BLE001 — event emission must not break the pipeline.
            _logger.warning("Circuit event emission failed for '%s'", event_type, exc_info=True)
