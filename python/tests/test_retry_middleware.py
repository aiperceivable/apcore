"""Tests for RetryMiddleware."""

from __future__ import annotations

from unittest.mock import patch

import pytest

from apcore.context import Context
from apcore.errors import ModuleError, ModuleTimeoutError
from apcore.middleware.manager import RetrySignal
from apcore.middleware.retry import RetryConfig, RetryMiddleware


def _make_context() -> Context:
    """Create a minimal Context for testing."""
    return Context(trace_id="test-trace", data={})


class TestRetryMiddlewareDefaults:
    def test_default_config_values(self) -> None:
        """Default RetryConfig has expected values."""
        cfg = RetryConfig()
        assert cfg.max_retries == 3
        assert cfg.strategy == "exponential"
        assert cfg.base_delay_ms == 100
        assert cfg.max_delay_ms == 5000
        assert cfg.jitter is True

    def test_default_config_used_when_none(self) -> None:
        """RetryMiddleware creates a default config when None is passed."""
        mw = RetryMiddleware(config=None)
        assert mw._config.max_retries == 3


class TestRetryMiddlewareRetryable:
    def test_retryable_error_returns_retry_signal(self) -> None:
        """A retryable error triggers retry by returning a RetrySignal."""
        mw = RetryMiddleware(config=RetryConfig(jitter=False, base_delay_ms=0))
        ctx = _make_context()
        inputs = {"x": 1}
        error = ModuleTimeoutError(module_id="mod.a", timeout_ms=1000)

        result = mw.on_error("mod.a", inputs, error, ctx)

        assert isinstance(result, RetrySignal)
        assert result.inputs == {"x": 1}
        assert ctx.data["_apcore.mw.retry.count.mod.a"] == 1

    def test_retry_signal_carries_copy_of_inputs(self) -> None:
        """The RetrySignal.inputs is a copy, not the same object as inputs."""
        mw = RetryMiddleware(config=RetryConfig(jitter=False, base_delay_ms=0))
        ctx = _make_context()
        inputs = {"x": 1}
        error = ModuleTimeoutError(module_id="mod.a", timeout_ms=1000)

        result = mw.on_error("mod.a", inputs, error, ctx)

        assert isinstance(result, RetrySignal)
        assert result.inputs is not inputs
        assert result.inputs == inputs


class TestRetryMiddlewareNonRetryable:
    def test_non_retryable_error_returns_none(self) -> None:
        """A non-retryable ModuleError returns None (no retry)."""
        mw = RetryMiddleware(config=RetryConfig(jitter=False, base_delay_ms=0))
        ctx = _make_context()
        error = ModuleError(code="TEST", message="fail", retryable=False)

        result = mw.on_error("mod.a", {"x": 1}, error, ctx)

        assert result is None

    def test_plain_exception_returns_none(self) -> None:
        """A plain Exception (no retryable attr) is not retried."""
        mw = RetryMiddleware(config=RetryConfig(jitter=False, base_delay_ms=0))
        ctx = _make_context()
        error = ValueError("boom")

        result = mw.on_error("mod.a", {"x": 1}, error, ctx)

        assert result is None

    def test_retryable_none_returns_none(self) -> None:
        """An error with retryable=None is not retried (only True triggers retry)."""
        mw = RetryMiddleware(config=RetryConfig(jitter=False, base_delay_ms=0))
        ctx = _make_context()
        error = ModuleError(code="TEST", message="fail", retryable=None)

        result = mw.on_error("mod.a", {"x": 1}, error, ctx)

        assert result is None


class TestRetryMiddlewareMaxRetries:
    def test_max_retries_exceeded_returns_none(self) -> None:
        """After max_retries attempts, on_error returns None."""
        mw = RetryMiddleware(config=RetryConfig(max_retries=2, jitter=False, base_delay_ms=0))
        ctx = _make_context()
        error = ModuleTimeoutError(module_id="mod.a", timeout_ms=1000)

        # First retry succeeds
        assert mw.on_error("mod.a", {"x": 1}, error, ctx) is not None
        assert ctx.data["_apcore.mw.retry.count.mod.a"] == 1

        # Second retry succeeds
        assert mw.on_error("mod.a", {"x": 1}, error, ctx) is not None
        assert ctx.data["_apcore.mw.retry.count.mod.a"] == 2

        # Third attempt: max exceeded
        assert mw.on_error("mod.a", {"x": 1}, error, ctx) is None
        # Count should not increment past max
        assert ctx.data["_apcore.mw.retry.count.mod.a"] == 2


class TestRetryMiddlewareDelayCalculation:
    def test_exponential_backoff_delay(self) -> None:
        """Exponential strategy: base * 2^attempt, capped at max_delay_ms."""
        mw = RetryMiddleware(
            config=RetryConfig(
                strategy="exponential",
                base_delay_ms=100,
                max_delay_ms=5000,
                jitter=False,
            )
        )

        assert mw._calculate_delay(0) == 100.0  # 100 * 2^0
        assert mw._calculate_delay(1) == 200.0  # 100 * 2^1
        assert mw._calculate_delay(2) == 400.0  # 100 * 2^2
        assert mw._calculate_delay(3) == 800.0  # 100 * 2^3
        assert mw._calculate_delay(10) == 5000.0  # capped at max

    def test_fixed_delay(self) -> None:
        """Fixed strategy: always returns base_delay_ms."""
        mw = RetryMiddleware(
            config=RetryConfig(
                strategy="fixed",
                base_delay_ms=250,
                jitter=False,
            )
        )

        assert mw._calculate_delay(0) == 250.0
        assert mw._calculate_delay(1) == 250.0
        assert mw._calculate_delay(5) == 250.0

    def test_jitter_adds_randomness(self) -> None:
        """Jitter multiplies delay by a random factor between 0.5 and 1.5."""
        mw = RetryMiddleware(
            config=RetryConfig(
                strategy="fixed",
                base_delay_ms=1000,
                jitter=True,
            )
        )

        # Patch random.uniform to return a known value
        with patch("apcore.middleware.retry.random.uniform", return_value=0.75):
            delay = mw._calculate_delay(0)
            assert delay == 750.0

        with patch("apcore.middleware.retry.random.uniform", return_value=1.5):
            delay = mw._calculate_delay(0)
            assert delay == 1500.0

        with patch("apcore.middleware.retry.random.uniform", return_value=0.5):
            delay = mw._calculate_delay(0)
            assert delay == 500.0

    def test_jitter_range_is_plausible(self) -> None:
        """Without mocking, jitter delays fall within [base*0.5, base*1.5]."""
        mw = RetryMiddleware(
            config=RetryConfig(
                strategy="fixed",
                base_delay_ms=1000,
                jitter=True,
            )
        )

        delays = [mw._calculate_delay(0) for _ in range(100)]
        assert all(500.0 <= d <= 1500.0 for d in delays)


class TestRetryMiddlewareContextTracking:
    def test_retry_count_tracked_per_module(self) -> None:
        """Each module_id gets its own retry counter in context.data."""
        mw = RetryMiddleware(config=RetryConfig(max_retries=3, jitter=False, base_delay_ms=0))
        ctx = _make_context()
        error = ModuleTimeoutError(module_id="mod.a", timeout_ms=1000)

        mw.on_error("mod.a", {"x": 1}, error, ctx)
        mw.on_error("mod.b", {"y": 2}, error, ctx)

        assert ctx.data["_apcore.mw.retry.count.mod.a"] == 1
        assert ctx.data["_apcore.mw.retry.count.mod.b"] == 1

    def test_retry_count_increments(self) -> None:
        """Retry count increments on each retry attempt."""
        mw = RetryMiddleware(config=RetryConfig(max_retries=5, jitter=False, base_delay_ms=0))
        ctx = _make_context()
        error = ModuleTimeoutError(module_id="mod.a", timeout_ms=1000)

        for i in range(3):
            mw.on_error("mod.a", {"x": 1}, error, ctx)

        assert ctx.data["_apcore.mw.retry.count.mod.a"] == 3

    @patch("apcore.middleware.retry.time.sleep")
    def test_sleep_called_with_correct_delay(self, mock_sleep) -> None:  # type: ignore[no-untyped-def]
        """time.sleep is called with the calculated delay in seconds."""
        mw = RetryMiddleware(
            config=RetryConfig(
                strategy="fixed",
                base_delay_ms=200,
                jitter=False,
            )
        )
        ctx = _make_context()
        error = ModuleTimeoutError(module_id="mod.a", timeout_ms=1000)

        mw.on_error("mod.a", {"x": 1}, error, ctx)

        mock_sleep.assert_called_once_with(0.2)  # 200ms -> 0.2s


class TestRetryMiddlewareIntegration:
    """End-to-end test through MiddlewareManager + Executor (W11).

    The unit-level tests above call mw.on_error() directly; this test
    verifies the wiring — MiddlewareManager recognises RetrySignal and the
    Executor actually re-runs the module.
    """

    @pytest.mark.asyncio
    async def test_retry_re_runs_module_and_returns_success(self) -> None:
        """A module that fails retryable twice then succeeds is re-run via the pipeline."""
        from apcore.errors import ModuleError
        from apcore.executor import Executor
        from apcore.registry import Registry
        from pydantic import BaseModel

        attempts: list[int] = []

        class _InSchema(BaseModel):
            x: int

        class _OutSchema(BaseModel):
            attempt: int

        class FlakyModule:
            input_schema = _InSchema
            output_schema = _OutSchema
            description = "Fails the first two times, succeeds on the third."

            async def execute(self, inputs: dict, context) -> dict:  # type: ignore[no-untyped-def]
                attempts.append(1)
                if len(attempts) < 3:
                    raise ModuleError(code="TRANSIENT", message="flaky", retryable=True)
                return {"attempt": len(attempts)}

        reg = Registry()
        reg.register("flaky", FlakyModule())

        mw = RetryMiddleware(config=RetryConfig(max_retries=5, jitter=False, base_delay_ms=0))
        ex = Executor(registry=reg, middlewares=[mw])

        result = await ex.call_async("flaky", {"x": 1})
        assert result == {"attempt": 3}
        assert len(attempts) == 3

    @pytest.mark.asyncio
    async def test_retry_gives_up_after_max_and_propagates_error(self) -> None:
        """After max_retries with persistent failure, the original error propagates."""
        from apcore.errors import ModuleError
        from apcore.executor import Executor
        from apcore.registry import Registry
        from pydantic import BaseModel

        class _InSchema(BaseModel):
            x: int

        class _OutSchema(BaseModel):
            ok: bool

        class AlwaysFailsModule:
            input_schema = _InSchema
            output_schema = _OutSchema
            description = "Always fails with retryable=True."

            async def execute(self, inputs: dict, context) -> dict:  # type: ignore[no-untyped-def]
                raise ModuleError(code="TRANSIENT", message="gone", retryable=True)

        reg = Registry()
        reg.register("never", AlwaysFailsModule())

        mw = RetryMiddleware(config=RetryConfig(max_retries=2, jitter=False, base_delay_ms=0))
        ex = Executor(registry=reg, middlewares=[mw])

        with pytest.raises(ModuleError):
            await ex.call_async("never", {"x": 1})
