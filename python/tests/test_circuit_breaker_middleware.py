"""Tests for CircuitBreakerMiddleware (Issue #42) — rolling-window model [D11-001]."""

from __future__ import annotations

import threading
from datetime import datetime, timedelta, timezone
from typing import Any

import pytest

from apcore.errors import CircuitBreakerOpenError, CircuitOpenError
from apcore.middleware.circuit_breaker import (
    CTX_CIRCUIT_STATE,
    CircuitBreakerMiddleware,
    CircuitBreakerState,
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


class _Ctx:
    """Minimal Context stand-in exposing ``caller_id`` and a ``data`` dict."""

    def __init__(self, caller_id: str | None = None) -> None:
        self.caller_id = caller_id
        self.data: dict[str, Any] = {}


class _RecordingEmitter:
    """Captures every emitted event for assertions."""

    def __init__(self) -> None:
        self.events: list[Any] = []

    def emit(self, event: Any) -> None:
        self.events.append(event)


class _MutableClock:
    """Controllable monotonic-ish clock for deterministic recovery tests."""

    def __init__(self) -> None:
        self.now = datetime(2026, 1, 1, tzinfo=timezone.utc)

    def __call__(self) -> datetime:
        return self.now

    def advance_ms(self, ms: int) -> None:
        self.now += timedelta(milliseconds=ms)


def _drive_failures(
    cb: CircuitBreakerMiddleware,
    n: int,
    module_id: str = "m1",
    ctx: _Ctx | None = None,
) -> None:
    ctx = ctx or _Ctx()
    err = RuntimeError("boom")
    for _ in range(n):
        cb.on_error(module_id, {}, err, ctx)


def _drive_successes(
    cb: CircuitBreakerMiddleware,
    n: int,
    module_id: str = "m1",
    ctx: _Ctx | None = None,
) -> None:
    ctx = ctx or _Ctx()
    for _ in range(n):
        cb.after(module_id, {}, {}, ctx)


# ---------------------------------------------------------------------------
# Public API naming (audit D2-001 — cross-SDK parity, matches apcore-rust)
# ---------------------------------------------------------------------------


def test_circuit_breaker_state_exported_from_top_level() -> None:
    import apcore
    from apcore.middleware.circuit_breaker import (
        CircuitBreakerState as MiddlewareState,
    )

    assert apcore.CircuitBreakerState is MiddlewareState
    assert "CircuitBreakerState" in apcore.__all__


def test_circuit_breaker_state_has_expected_members() -> None:
    from apcore import CircuitBreakerState

    assert {m.name for m in CircuitBreakerState} == {"CLOSED", "OPEN", "HALF_OPEN"}


def test_old_middleware_circuit_state_name_is_gone() -> None:
    import apcore
    import apcore.middleware.circuit_breaker as cb_module

    assert "CircuitState" not in apcore.__all__
    assert not hasattr(cb_module, "CircuitState")


# ---------------------------------------------------------------------------
# Construction / validation
# ---------------------------------------------------------------------------


def test_default_state_is_closed() -> None:
    cb = CircuitBreakerMiddleware()
    assert cb.get_state("m1") == CircuitBreakerState.CLOSED


def test_custom_priority() -> None:
    cb = CircuitBreakerMiddleware(priority=200)
    assert cb.priority == 200


def test_invalid_open_threshold_raises() -> None:
    with pytest.raises(ValueError, match="open_threshold"):
        CircuitBreakerMiddleware(open_threshold=1.5)


def test_invalid_recovery_window_raises() -> None:
    with pytest.raises(ValueError, match="recovery_window_ms"):
        CircuitBreakerMiddleware(recovery_window_ms=-1)


def test_invalid_window_size_raises() -> None:
    with pytest.raises(ValueError, match="window_size"):
        CircuitBreakerMiddleware(window_size=0)


def test_invalid_min_samples_raises() -> None:
    with pytest.raises(ValueError, match="min_samples"):
        CircuitBreakerMiddleware(min_samples=0)


def test_min_samples_clamped_to_window_size(caplog: pytest.LogCaptureFixture) -> None:
    import logging

    with caplog.at_level(logging.WARNING):
        cb = CircuitBreakerMiddleware(window_size=5, min_samples=10)
    assert cb._min_samples == 5
    assert any("min_samples" in r.message for r in caplog.records)


# ---------------------------------------------------------------------------
# CLOSED → OPEN at error-rate threshold (with min_samples gate)
# ---------------------------------------------------------------------------


def test_opens_at_error_rate_threshold() -> None:
    """6 errors + 4 successes in a window of 10 → rate 0.6 ≥ 0.5 → OPEN."""
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=10, min_samples=5)
    ctx = _Ctx(caller_id="orchestrator.billing")
    module = "executor.payment.charge"

    _drive_failures(cb, 6, module, ctx)
    _drive_successes(cb, 4, module, ctx)

    assert cb.get_state(module, "orchestrator.billing") == CircuitBreakerState.OPEN


def test_does_not_open_below_min_samples() -> None:
    """Below min_samples, even a 100% error rate must NOT open the circuit."""
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=20, min_samples=5)
    ctx = _Ctx()
    _drive_failures(cb, 4, "m1", ctx)  # 4 < min_samples
    assert cb.get_state("m1") == CircuitBreakerState.CLOSED
    # 5th failure crosses the gate → opens.
    _drive_failures(cb, 1, "m1", ctx)
    assert cb.get_state("m1") == CircuitBreakerState.OPEN


def test_does_not_open_below_threshold() -> None:
    """Enough samples but error rate under threshold stays CLOSED."""
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=10, min_samples=5)
    ctx = _Ctx()
    _drive_failures(cb, 4, "m1", ctx)
    _drive_successes(cb, 6, "m1", ctx)  # rate 0.4 < 0.5
    assert cb.get_state("m1") == CircuitBreakerState.CLOSED


# ---------------------------------------------------------------------------
# OPEN — short-circuits before()
# ---------------------------------------------------------------------------


def test_open_circuit_short_circuits_before() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5)
    ctx = _Ctx(caller_id="caller.x")
    _drive_failures(cb, 5, "m1", ctx)
    assert cb.get_state("m1", "caller.x") == CircuitBreakerState.OPEN

    with pytest.raises(CircuitBreakerOpenError) as exc_info:
        cb.before("m1", {}, ctx)
    assert exc_info.value.module_id == "m1"
    assert exc_info.value.caller_id == "caller.x"


def test_open_circuit_does_not_affect_other_keys() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5)
    ctx_a = _Ctx(caller_id="caller.a")
    _drive_failures(cb, 5, "m1", ctx_a)
    assert cb.get_state("m1", "caller.a") == CircuitBreakerState.OPEN

    # Different module — unaffected.
    assert cb.get_state("m2", "caller.a") == CircuitBreakerState.CLOSED
    cb.before("m2", {}, ctx_a)  # must not raise

    # Same module, different caller — independent circuit.
    ctx_b = _Ctx(caller_id="caller.b")
    assert cb.get_state("m1", "caller.b") == CircuitBreakerState.CLOSED
    cb.before("m1", {}, ctx_b)  # must not raise


# ---------------------------------------------------------------------------
# OPEN → HALF_OPEN → CLOSED / OPEN (clock-driven recovery)
# ---------------------------------------------------------------------------


def test_half_open_after_recovery_window_admits_single_probe() -> None:
    clock = _MutableClock()
    cb = CircuitBreakerMiddleware(
        open_threshold=0.5, window_size=5, min_samples=5, recovery_window_ms=30_000, clock=clock
    )
    ctx = _Ctx(caller_id="c1")
    _drive_failures(cb, 5, "m1", ctx)
    assert cb.get_state("m1", "c1") == CircuitBreakerState.OPEN

    # Before recovery window: still OPEN, before() rejects.
    clock.advance_ms(5_000)
    with pytest.raises(CircuitBreakerOpenError):
        cb.before("m1", {}, ctx)

    # After recovery window: transitions to HALF_OPEN, first probe admitted.
    clock.advance_ms(30_000)
    cb.before("m1", {}, ctx)  # probe 1 admitted, no raise
    assert ctx.data[CTX_CIRCUIT_STATE] == CircuitBreakerState.HALF_OPEN.value

    # Second concurrent probe rejected (probe_in_flight).
    ctx2 = _Ctx(caller_id="c1")
    with pytest.raises(CircuitBreakerOpenError):
        cb.before("m1", {}, ctx2)


def test_probe_success_closes_circuit() -> None:
    clock = _MutableClock()
    emitter = _RecordingEmitter()
    cb = CircuitBreakerMiddleware(
        open_threshold=0.5,
        window_size=5,
        min_samples=5,
        recovery_window_ms=30_000,
        emitter=emitter,
        clock=clock,
    )
    ctx = _Ctx(caller_id="c1")
    _drive_failures(cb, 5, "m1", ctx)
    clock.advance_ms(30_000)
    cb.before("m1", {}, ctx)  # admit probe
    cb.after("m1", {}, {}, ctx)  # probe succeeds
    assert cb.get_state("m1", "c1") == CircuitBreakerState.CLOSED

    closed = [e for e in emitter.events if e.event_type == "apcore.circuit.closed"]
    assert len(closed) == 1
    assert closed[0].data["module_id"] == "m1"
    assert closed[0].data["caller_id"] == "c1"


def test_probe_failure_reopens_circuit() -> None:
    clock = _MutableClock()
    cb = CircuitBreakerMiddleware(
        open_threshold=0.5, window_size=5, min_samples=5, recovery_window_ms=30_000, clock=clock
    )
    ctx = _Ctx(caller_id="c1")
    _drive_failures(cb, 5, "m1", ctx)
    clock.advance_ms(30_000)
    cb.before("m1", {}, ctx)  # admit probe
    cb.on_error("m1", {}, RuntimeError("boom"), ctx)  # probe fails
    assert cb.get_state("m1", "c1") == CircuitBreakerState.OPEN


# ---------------------------------------------------------------------------
# context.data circuit.state written on every call
# ---------------------------------------------------------------------------


def test_before_writes_circuit_state_to_context_closed() -> None:
    cb = CircuitBreakerMiddleware()
    ctx = _Ctx(caller_id="c1")
    cb.before("m1", {}, ctx)
    assert ctx.data[CTX_CIRCUIT_STATE] == CircuitBreakerState.CLOSED.value


def test_before_writes_open_state_on_short_circuit() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5)
    ctx = _Ctx(caller_id="c1")
    _drive_failures(cb, 5, "m1", ctx)
    with pytest.raises(CircuitBreakerOpenError):
        cb.before("m1", {}, ctx)
    assert ctx.data[CTX_CIRCUIT_STATE] == CircuitBreakerState.OPEN.value


# ---------------------------------------------------------------------------
# Events emitted on transitions
# ---------------------------------------------------------------------------


def test_open_transition_emits_event_with_payload() -> None:
    emitter = _RecordingEmitter()
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5, emitter=emitter)
    ctx = _Ctx(caller_id="c1")
    _drive_failures(cb, 5, "m1", ctx)

    opened = [e for e in emitter.events if e.event_type == "apcore.circuit.opened"]
    assert len(opened) == 1
    payload = opened[0].data
    assert payload["module_id"] == "m1"
    assert payload["caller_id"] == "c1"
    assert payload["error_rate"] >= 0.5


def test_no_emitter_does_not_raise() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5, emitter=None)
    ctx = _Ctx()
    _drive_failures(cb, 5, "m1", ctx)  # must not raise despite opening
    assert cb.get_state("m1") == CircuitBreakerState.OPEN


# ---------------------------------------------------------------------------
# on_error ignores CircuitBreakerOpenError
# ---------------------------------------------------------------------------


def test_on_error_ignores_circuit_open_error() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=1)
    ctx = _Ctx()
    result = cb.on_error("m1", {}, CircuitOpenError(module_id="m1"), ctx)
    assert result is None
    assert cb.get_state("m1") == CircuitBreakerState.CLOSED


# ---------------------------------------------------------------------------
# Manual reset
# ---------------------------------------------------------------------------


def test_reset_closes_open_circuit() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5)
    ctx = _Ctx(caller_id="c1")
    _drive_failures(cb, 5, "m1", ctx)
    assert cb.get_state("m1", "c1") == CircuitBreakerState.OPEN
    cb.reset("m1", "c1")
    assert cb.get_state("m1", "c1") == CircuitBreakerState.CLOSED


# ---------------------------------------------------------------------------
# Per-(module_id, caller_id) independence
# ---------------------------------------------------------------------------


def test_per_module_caller_independence() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5)
    ctx_a = _Ctx(caller_id="a")
    ctx_b = _Ctx(caller_id="b")
    _drive_failures(cb, 5, "m1", ctx_a)
    assert cb.get_state("m1", "a") == CircuitBreakerState.OPEN
    assert cb.get_state("m1", "b") == CircuitBreakerState.CLOSED
    # caller 'b' circuit is independent and still admits calls.
    cb.before("m1", {}, ctx_b)


def test_missing_caller_id_buckets_under_empty_string() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=5, min_samples=5)
    ctx = _Ctx(caller_id=None)
    _drive_failures(cb, 5, "m1", ctx)
    assert cb.get_state("m1") == CircuitBreakerState.OPEN
    assert cb.get_state("m1", None) == CircuitBreakerState.OPEN


# ---------------------------------------------------------------------------
# Priority and base-class integration
# ---------------------------------------------------------------------------


def test_circuit_breaker_middleware_priority_ordering() -> None:
    from apcore.middleware.manager import MiddlewareManager

    cb_high = CircuitBreakerMiddleware(priority=900)
    cb_low = CircuitBreakerMiddleware(priority=10)

    mgr = MiddlewareManager()
    mgr.add(cb_low)
    mgr.add(cb_high)
    snap = mgr.snapshot()
    assert snap[0] is cb_high
    assert snap[1] is cb_low


# ---------------------------------------------------------------------------
# CircuitBreakerOpenError properties
# ---------------------------------------------------------------------------


def test_circuit_open_error_properties() -> None:
    err = CircuitBreakerOpenError(module_id="my.module", caller_id="orchestrator")
    assert err.code == "CIRCUIT_BREAKER_OPEN"
    assert err.module_id == "my.module"
    assert err.caller_id == "orchestrator"
    assert err.retryable is True


def test_circuit_open_error_caller_id_optional() -> None:
    err = CircuitBreakerOpenError(module_id="my.module")
    assert err.module_id == "my.module"
    assert err.caller_id is None


def test_error_codes_circuit_open_constant() -> None:
    from apcore.errors import ErrorCodes

    assert ErrorCodes.CIRCUIT_OPEN == "CIRCUIT_OPEN"
    assert ErrorCodes.CIRCUIT_BREAKER_OPEN == "CIRCUIT_BREAKER_OPEN"


# ---------------------------------------------------------------------------
# Concurrency
# ---------------------------------------------------------------------------


def test_concurrent_access_does_not_corrupt_state() -> None:
    cb = CircuitBreakerMiddleware(open_threshold=0.5, window_size=20, min_samples=5)
    errors: list[Exception] = []

    def worker() -> None:
        ctx = _Ctx(caller_id="shared-caller")
        try:
            for _ in range(100):
                try:
                    cb.before("shared", {}, ctx)
                    cb.after("shared", {}, {}, ctx)
                except CircuitBreakerOpenError:
                    pass
                cb.on_error("shared", {}, RuntimeError("boom"), ctx)
        except Exception as exc:  # noqa: BLE001
            errors.append(exc)

    threads = [threading.Thread(target=worker) for _ in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    assert not errors, f"Concurrency errors: {errors}"
    assert cb.get_state("shared", "shared-caller") in CircuitBreakerState.__members__.values()
