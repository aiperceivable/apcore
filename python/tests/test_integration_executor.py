"""Integration tests for the complete executor pipeline."""

from __future__ import annotations

from typing import Any

import pytest

from apcore.acl import ACL, ACLRule
from apcore.config import Config
from apcore.context import Context, Identity
from apcore.errors import (
    ACLDeniedError,
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    ModuleExecuteError,
    ModuleTimeoutError,
)
from apcore.executor import Executor
from apcore.middleware import LoggingMiddleware, Middleware
from apcore.registry import Registry


# === Full Pipeline Tests ===


class TestFullPipeline:
    """End-to-end tests for the complete call() pipeline."""

    def test_full_pipeline_call(self, executor: Executor) -> None:
        """Full 10-step pipeline completes and returns correct output."""
        result = executor.call("test.sync_module", {"name": "World"})
        assert result == {"greeting": "Hello, World!"}

    def test_full_pipeline_with_middleware(self, mock_registry: Registry, sample_middleware: Any) -> None:
        """Middleware before/after called in correct order."""
        ex = Executor(registry=mock_registry, middlewares=[sample_middleware])
        result = ex.call("test.sync_module", {"name": "Alice"})
        assert result == {"greeting": "Hello, Alice!"}
        assert len(sample_middleware.before_calls) == 1
        assert len(sample_middleware.after_calls) == 1
        assert sample_middleware.before_calls[0][0] == "test.sync_module"

    def test_full_pipeline_with_acl_allow(self, mock_registry: Registry) -> None:
        """ACL allow rule permits the call."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")])
        ex = Executor(registry=mock_registry, acl=acl)
        result = ex.call("test.sync_module", {"name": "Bob"})
        assert result == {"greeting": "Hello, Bob!"}

    def test_full_pipeline_with_logging_middleware(self, mock_registry: Registry) -> None:
        """LoggingMiddleware does not interfere with the pipeline."""
        ex = Executor(registry=mock_registry, middlewares=[LoggingMiddleware()])
        result = ex.call("test.sync_module", {"name": "Charlie"})
        assert result == {"greeting": "Hello, Charlie!"}


# === Call Chain Tests ===


class TestCallChain:
    """Tests for module-to-module call chain propagation."""

    def test_call_chain_propagation(self, mock_registry: Registry, chain_module_factory: Any) -> None:
        """Module A calls module B via context.executor, chain propagates."""
        chain_module_factory("mod.a", calls="test.sync_module")
        ex = Executor(registry=mock_registry)
        result = ex.call("mod.a", {"name": "Chain"})
        assert result == {"greeting": "Hello, Chain!"}

    def test_circular_detection(self, mock_registry: Registry, chain_module_factory: Any) -> None:
        """A->B->A raises CircularCallError."""
        chain_module_factory("mod.a", calls="mod.b")
        chain_module_factory("mod.b", calls="mod.a")
        ex = Executor(registry=mock_registry)
        with pytest.raises(CircularCallError) as exc_info:
            ex.call("mod.a", {"name": "test"})
        assert exc_info.value.module_id in ("mod.a", "mod.b")

    def test_depth_exceeded(self, mock_registry: Registry, chain_module_factory: Any) -> None:
        """Self-recursive call chain exceeds max_call_depth."""
        chain_module_factory("mod.recursive", calls="mod.recursive")
        config = Config(data={"executor": {"max_call_depth": 5, "max_module_repeat": 100}})
        ex = Executor(registry=mock_registry, config=config)
        with pytest.raises(CallDepthExceededError):
            ex.call("mod.recursive", {"name": "deep"})


# === ACL Tests ===


class TestACLIntegration:
    """Integration tests for ACL enforcement in the executor pipeline."""

    def test_acl_denial(self, mock_registry: Registry) -> None:
        """ACL deny-all rule raises ACLDeniedError."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny")])
        ex = Executor(registry=mock_registry, acl=acl)
        with pytest.raises(ACLDeniedError) as exc_info:
            ex.call("test.sync_module", {"name": "test"})
        assert exc_info.value.target_id == "test.sync_module"

    def test_acl_context_conditions(self, mock_registry: Registry) -> None:
        """ACL with identity_types condition enforces correctly."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["test.*"],
                    effect="allow",
                    conditions={"identity_types": ["service"]},
                ),
            ],
            default_effect="deny",
        )
        ex = Executor(registry=mock_registry, acl=acl)

        # No identity -> denied
        with pytest.raises(ACLDeniedError):
            ex.call("test.sync_module", {"name": "test"})

        # Service identity -> allowed
        ctx = Context.create()
        ctx.identity = Identity(id="svc-1", type="service")
        result = ex.call("test.sync_module", {"name": "test"}, context=ctx)
        assert result == {"greeting": "Hello, test!"}


# === Middleware Error Recovery Tests ===


class TestMiddlewareErrorRecovery:
    """Tests for middleware on_error recovery in the executor pipeline."""

    def test_on_error_recovery(self, mock_registry: Registry) -> None:
        """Middleware on_error returns fallback dict instead of raising."""

        class RecoveryMiddleware(Middleware):
            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> dict[str, Any]:
                return {"fallback": True, "greeting": "recovered"}

        ex = Executor(registry=mock_registry, middlewares=[RecoveryMiddleware()])
        result = ex.call("test.failing_module", {})
        assert result == {"fallback": True, "greeting": "recovered"}

    def test_on_error_no_recovery_reraises(self, mock_registry: Registry) -> None:
        """Middleware on_error returning None re-raises original exception."""

        class NoRecoveryMiddleware(Middleware):
            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> None:
                return None

        ex = Executor(registry=mock_registry, middlewares=[NoRecoveryMiddleware()])
        with pytest.raises(ModuleExecuteError, match="module execution failed"):
            ex.call("test.failing_module", {})


# === Async Pipeline Tests ===


class TestAsyncPipeline:
    """Integration tests for async execution pipeline."""

    @pytest.mark.asyncio
    async def test_async_full_pipeline(self, executor: Executor) -> None:
        """call_async() with async module completes full pipeline."""
        result = await executor.call_async("test.async_module", {"name": "Async"})
        assert result == {"greeting": "Hello, Async!"}

    @pytest.mark.asyncio
    async def test_async_with_sync_module(self, executor: Executor) -> None:
        """call_async() bridges sync module via to_thread."""
        result = await executor.call_async("test.sync_module", {"name": "Bridged"})
        assert result == {"greeting": "Hello, Bridged!"}

    def test_sync_call_with_async_module(self, executor: Executor) -> None:
        """Sync call() bridges async module."""
        result = executor.call("test.async_module", {"name": "SyncBridge"})
        assert result == {"greeting": "Hello, SyncBridge!"}


# === Timeout Tests ===


class TestTimeoutIntegration:
    """Integration tests for timeout enforcement."""

    def test_timeout_with_slow_module(self, mock_registry: Registry) -> None:
        """Short timeout with slow module raises ModuleTimeoutError."""
        config = Config(data={"executor": {"default_timeout": 100}})
        ex = Executor(registry=mock_registry, config=config)
        with pytest.raises(ModuleTimeoutError) as exc_info:
            ex.call("test.slow_module", {})
        assert exc_info.value.module_id == "test.slow_module"
        assert exc_info.value.timeout_ms == 100

    @pytest.mark.asyncio
    async def test_async_timeout_with_slow_module(self, mock_registry: Registry) -> None:
        """Async timeout with slow module raises ModuleTimeoutError."""
        config = Config(data={"executor": {"default_timeout": 100}})
        ex = Executor(registry=mock_registry, config=config)
        with pytest.raises(ModuleTimeoutError):
            await ex.call_async("test.slow_module", {})


# === Public API Export Tests ===


class TestPublicAPIExports:
    """Tests verifying all executor/middleware symbols are importable from apcore."""

    def test_executor_importable(self) -> None:
        from apcore import Executor

        assert Executor is not None

    def test_acl_importable(self) -> None:
        from apcore import ACL, ACLRule

        assert ACL is not None
        assert ACLRule is not None

    def test_middleware_importable(self) -> None:
        from apcore import Middleware, MiddlewareManager

        assert Middleware is not None
        assert MiddlewareManager is not None

    def test_adapters_importable(self) -> None:
        from apcore import AfterMiddleware, BeforeMiddleware

        assert BeforeMiddleware is not None
        assert AfterMiddleware is not None

    def test_logging_middleware_importable(self) -> None:
        from apcore import LoggingMiddleware

        assert LoggingMiddleware is not None

    def test_redact_sensitive_importable(self) -> None:
        from apcore import redact_sensitive

        assert redact_sensitive is not None

    def test_validation_result_importable(self) -> None:
        from apcore import ValidationResult

        assert ValidationResult is not None


# === Error Convenience Property Tests ===


class TestErrorConvenienceProperties:
    """Tests for ergonomic property accessors on error subclasses."""

    def test_call_depth_exceeded_current_depth(self) -> None:
        e = CallDepthExceededError(depth=10, max_depth=32, call_chain=["a", "b"])
        assert e.current_depth == 10

    def test_call_depth_exceeded_max_depth(self) -> None:
        e = CallDepthExceededError(depth=10, max_depth=32, call_chain=["a", "b"])
        assert e.max_depth == 32

    def test_circular_call_error_module_id(self) -> None:
        e = CircularCallError(module_id="A", call_chain=["A", "B"])
        assert e.module_id == "A"

    def test_acl_denied_error_caller_id(self) -> None:
        e = ACLDeniedError(caller_id="api.handler", target_id="db.write")
        assert e.caller_id == "api.handler"

    def test_acl_denied_error_target_id(self) -> None:
        e = ACLDeniedError(caller_id="api.handler", target_id="db.write")
        assert e.target_id == "db.write"

    def test_call_frequency_exceeded_module_id(self) -> None:
        e = CallFrequencyExceededError(module_id="X", count=5, max_repeat=3, call_chain=["X"])
        assert e.module_id == "X"

    def test_call_frequency_exceeded_count(self) -> None:
        e = CallFrequencyExceededError(module_id="X", count=5, max_repeat=3, call_chain=["X"])
        assert e.count == 5

    def test_call_frequency_exceeded_max_repeat(self) -> None:
        e = CallFrequencyExceededError(module_id="X", count=5, max_repeat=3, call_chain=["X"])
        assert e.max_repeat == 3

    def test_module_timeout_error_module_id(self) -> None:
        e = ModuleTimeoutError(module_id="slow.mod", timeout_ms=5000)
        assert e.module_id == "slow.mod"

    def test_module_timeout_error_timeout_ms(self) -> None:
        e = ModuleTimeoutError(module_id="slow.mod", timeout_ms=5000)
        assert e.timeout_ms == 5000
