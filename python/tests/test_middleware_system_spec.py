"""Spec-traced contract tests for the Middleware System.

Source spec: apcore/docs/features/middleware-system.md
Contracts:
  - Middleware.before
  - Middleware.after
  - Middleware.on_error
  - Middleware.detect_async   (no standalone symbol in this SDK -> skips)

Each test carries a verbatim clause id of the form
`middleware_system.<method>.<kind>.<detail>` so cross-language diffs line up
row by row. These clause ids are intentionally identical across the Python,
TypeScript, and Rust suites. Tests only -- production source is never modified.

Onion-model facts confirmed against the source under test
(`apcore.middleware.manager.MiddlewareManager`):
  - `execute_before` runs in registration/snapshot order and returns
    `(final_inputs, executed_middlewares)`; a failing `before()` is wrapped in
    `MiddlewareChainError` carrying `executed_middlewares`.
  - `execute_after` runs in REVERSE order and propagates the first exception
    (fail-fast).
  - `execute_on_error` runs in reverse over the executed middlewares;
    first dict (or RetrySignal) wins; exceptions inside a handler are logged
    and iteration continues.
"""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.context import Context
from apcore.errors import ModuleError
from apcore.middleware import (
    AfterMiddleware,
    BeforeMiddleware,
    Middleware,
    MiddlewareChainError,
    MiddlewareManager,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _ctx() -> Context:
    """A fresh execution context for a single pipeline pass."""
    return Context.create()


def _manager(*middlewares: Middleware) -> MiddlewareManager:
    """A MiddlewareManager pre-loaded with the given middlewares in order.

    `add()` is priority-sorted but stable for equal priorities, so passing
    middlewares with the default priority preserves registration order.
    """
    mgr = MiddlewareManager()
    for mw in middlewares:
        mgr.add(mw)
    return mgr


class _Recording(Middleware):
    """Middleware that records ordered observations into a shared sink."""

    def __init__(self, label: str, sink: list[str], *, priority: int = 100) -> None:
        super().__init__(priority=priority)
        self.label = label
        self.sink = sink

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
        self.sink.append(f"before:{self.label}")
        return None

    def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
        self.sink.append(f"after:{self.label}")
        return None

    def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context) -> None:
        self.sink.append(f"on_error:{self.label}")
        return None


# ===========================================================================
# Contract: Middleware.before
# ===========================================================================


class TestBeforeInputs:
    def test_before_module_id_required(self) -> None:
        """middleware_system.before.input.module_id.required

        `module_id` is a required positional input. Calling before() without
        it must raise TypeError (the missing-argument failure path).
        """
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.before(inputs={}, context=_ctx())  # type: ignore[call-arg]

    def test_before_inputs_required(self) -> None:
        """middleware_system.before.input.inputs.required

        `inputs` is a required positional input. Omitting it must raise
        TypeError.
        """
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.before("mod.id", context=_ctx())  # type: ignore[call-arg]

    def test_before_context_required(self) -> None:
        """middleware_system.before.input.context.required

        `context` is a required positional input. Omitting it must raise
        TypeError.
        """
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.before("mod.id", {})  # type: ignore[call-arg]


class TestBeforeReturns:
    def test_before_none_passes_through(self) -> None:
        """middleware_system.before.returns.none_passthrough

        Returning None from before() leaves the inputs unchanged through the
        manager's execute_before pass.
        """
        mgr = _manager(BeforeMiddleware(lambda m, i, c: None))
        inputs = {"a": 1}
        final, executed = mgr.execute_before("mod.id", inputs, _ctx())
        assert final == {"a": 1}
        assert final is inputs  # untouched identity confirms passthrough
        assert len(executed) == 1

    def test_before_dict_replaces_inputs(self) -> None:
        """middleware_system.before.returns.dict_replaces_inputs

        Returning a dict from before() replaces the inputs seen by downstream
        middleware and by the module body.
        """
        seen_downstream: dict[str, Any] = {}

        def replace(m: str, i: dict[str, Any], c: Context) -> dict[str, Any]:
            return {"replaced": True}

        def observe(m: str, i: dict[str, Any], c: Context) -> None:
            seen_downstream.update(i)
            return None

        mgr = _manager(BeforeMiddleware(replace), BeforeMiddleware(observe))
        final, _ = mgr.execute_before("mod.id", {"orig": 1}, _ctx())
        assert final == {"replaced": True}
        # downstream middleware observed the replacement, not the original.
        assert seen_downstream == {"replaced": True}


class TestBeforeErrors:
    def test_before_raise_wrapped_in_chain_error(self) -> None:
        """middleware_system.before.error.MIDDLEWARE_CHAIN_ERROR

        A before() that raises is wrapped by the manager in
        MiddlewareChainError with code MIDDLEWARE_CHAIN_ERROR, carrying the
        original cause.
        """
        boom = RuntimeError("before exploded")

        def raiser(m: str, i: dict[str, Any], c: Context) -> dict[str, Any]:
            raise boom

        mgr = _manager(BeforeMiddleware(raiser))
        with pytest.raises(MiddlewareChainError) as exc_info:
            mgr.execute_before("mod.id", {}, _ctx())
        err = exc_info.value
        assert isinstance(err, ModuleError)
        assert err.code == "MIDDLEWARE_CHAIN_ERROR"
        assert err.original is boom

    def test_before_failure_skips_downstream_tracks_executed(self) -> None:
        """middleware_system.before.error.aborts_pipeline_tracks_executed

        When a middleware's before() raises, downstream before() hooks are
        skipped and the raised MiddlewareChainError carries exactly the
        middlewares whose before() had been entered (executed_middlewares).
        """
        sink: list[str] = []
        first = _Recording("first", sink)

        def raiser(m: str, i: dict[str, Any], c: Context) -> dict[str, Any]:
            sink.append("before:raiser")
            raise ValueError("stop")

        raising_mw = BeforeMiddleware(raiser)
        downstream = _Recording("downstream", sink)
        mgr = _manager(first, raising_mw, downstream)

        with pytest.raises(MiddlewareChainError) as exc_info:
            mgr.execute_before("mod.id", {}, _ctx())

        # downstream before() never ran.
        assert "before:downstream" not in sink
        assert sink == ["before:first", "before:raiser"]
        # executed_middlewares = the ones entered, up to and including the raiser.
        assert exc_info.value.executed_middlewares == [first, raising_mw]


class TestBeforeProperties:
    async def test_before_async_supported(self) -> None:
        """middleware_system.before.property.async

        Python permits async before(): the async-aware manager path awaits a
        coroutine return and applies its result.
        """

        class AsyncBefore(Middleware):
            async def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                await asyncio.sleep(0)
                return {"awaited": True}

        mgr = _manager(AsyncBefore())
        final, executed = await mgr.execute_before_async("mod.id", {"orig": 1}, _ctx())
        assert final == {"awaited": True}
        assert len(executed) == 1

    async def test_before_thread_safe_concurrent(self) -> None:
        """middleware_system.before.property.thread_safe

        N concurrent execute_before_async passes with distinct inputs complete
        without exception, and each pass returns its own distinct result with no
        cross-talk. Concurrent add() during the passes does not corrupt state.
        """
        n = 10

        def tag_input(m: str, i: dict[str, Any], c: Context) -> dict[str, Any]:
            return {"echo": i["idx"]}

        mgr = _manager(BeforeMiddleware(tag_input))

        async def one(idx: int) -> dict[str, Any]:
            await asyncio.sleep(0)
            final, _ = await mgr.execute_before_async("mod.id", {"idx": idx}, _ctx())
            return final

        async def churn() -> None:
            for _ in range(n):
                await asyncio.sleep(0)
                mgr.add(BeforeMiddleware(lambda m, i, c: None))

        results, _ = await asyncio.gather(
            asyncio.gather(*(one(i) for i in range(n))),
            churn(),
        )
        assert [r["echo"] for r in results] == list(range(n))
        # The snapshot pattern means the concurrent adds never raised.

    def test_before_pure_false_mutates_context(self) -> None:
        """middleware_system.before.property.pure

        Contract declares pure: false. A before() may mutate context.data; the
        mutation is observable via the public context after execute_before.
        """

        def mutate(m: str, i: dict[str, Any], c: Context) -> None:
            c.data["ext.spec.before_ran"] = True
            return None

        ctx = _ctx()
        assert "ext.spec.before_ran" not in ctx.data
        mgr = _manager(BeforeMiddleware(mutate))
        mgr.execute_before("mod.id", {}, ctx)
        assert ctx.data["ext.spec.before_ran"] is True


class TestBeforeSideEffects:
    def test_before_runs_in_registration_order(self) -> None:
        """middleware_system.before.side_effect.1.registration_order

        before() hooks run in registration order (MW1 then MW2 then MW3).
        """
        sink: list[str] = []
        mgr = _manager(
            _Recording("1", sink),
            _Recording("2", sink),
            _Recording("3", sink),
        )
        mgr.execute_before("mod.id", {}, _ctx())
        assert sink == ["before:1", "before:2", "before:3"]


# ===========================================================================
# Contract: Middleware.after
# ===========================================================================


class TestAfterInputs:
    def test_after_module_id_required(self) -> None:
        """middleware_system.after.input.module_id.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.after(inputs={}, output={}, context=_ctx())  # type: ignore[call-arg]

    def test_after_inputs_required(self) -> None:
        """middleware_system.after.input.inputs.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.after("mod.id", output={}, context=_ctx())  # type: ignore[call-arg]

    def test_after_output_required(self) -> None:
        """middleware_system.after.input.output.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.after("mod.id", {}, context=_ctx())  # type: ignore[call-arg]

    def test_after_context_required(self) -> None:
        """middleware_system.after.input.context.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.after("mod.id", {}, {})  # type: ignore[call-arg]


class TestAfterReturns:
    def test_after_none_passes_through(self) -> None:
        """middleware_system.after.returns.none_passthrough

        Returning None from after() leaves the output unchanged.
        """
        mgr = _manager(AfterMiddleware(lambda m, i, o, c: None))
        out = {"v": 1}
        final = mgr.execute_after("mod.id", {}, out, _ctx())
        assert final == {"v": 1}
        assert final is out

    def test_after_dict_replaces_output(self) -> None:
        """middleware_system.after.returns.dict_replaces_output

        Returning a dict from after() replaces the output passed onward.
        """
        mgr = _manager(AfterMiddleware(lambda m, i, o, c: {"wrapped": o}))
        final = mgr.execute_after("mod.id", {}, {"v": 1}, _ctx())
        assert final == {"wrapped": {"v": 1}}


class TestAfterErrors:
    def test_after_raise_propagates_fail_fast(self) -> None:
        """middleware_system.after.error.fail_fast_propagates

        An after() that raises propagates immediately (fail-fast): the chain
        stops at the first error and remaining hooks do not run.
        """
        sink: list[str] = []

        def raiser(m: str, i: dict[str, Any], o: dict[str, Any], c: Context) -> dict[str, Any]:
            raise RuntimeError("after exploded")

        # after() runs in REVERSE order: the LAST-registered runs first.
        # Register raiser last so it runs first and the recorder never runs.
        recorder = _Recording("never", sink)
        mgr = _manager(recorder, AfterMiddleware(raiser))
        with pytest.raises(RuntimeError, match="after exploded"):
            mgr.execute_after("mod.id", {}, {"v": 1}, _ctx())
        assert "after:never" not in sink


class TestAfterProperties:
    async def test_after_async_supported(self) -> None:
        """middleware_system.after.property.async"""

        class AsyncAfter(Middleware):
            async def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any]:
                await asyncio.sleep(0)
                return {"awaited": output}

        mgr = _manager(AsyncAfter())
        final = await mgr.execute_after_async("mod.id", {}, {"v": 1}, _ctx())
        assert final == {"awaited": {"v": 1}}

    async def test_after_thread_safe_concurrent(self) -> None:
        """middleware_system.after.property.thread_safe

        N concurrent execute_after_async passes with distinct outputs each
        return their own result without cross-talk or exception.
        """
        n = 10
        mgr = _manager(AfterMiddleware(lambda m, i, o, c: {"echo": o["idx"]}))

        async def one(idx: int) -> dict[str, Any]:
            await asyncio.sleep(0)
            return await mgr.execute_after_async("mod.id", {}, {"idx": idx}, _ctx())

        results = await asyncio.gather(*(one(i) for i in range(n)))
        assert [r["echo"] for r in results] == list(range(n))


class TestAfterSideEffects:
    def test_after_runs_in_reverse_order(self) -> None:
        """middleware_system.after.side_effect.1.reverse_order

        after() hooks run in REVERSE registration order (MW3 then MW2 then MW1).
        """
        sink: list[str] = []
        mgr = _manager(
            _Recording("1", sink),
            _Recording("2", sink),
            _Recording("3", sink),
        )
        mgr.execute_after("mod.id", {}, {"v": 1}, _ctx())
        assert sink == ["after:3", "after:2", "after:1"]


# ===========================================================================
# Contract: Middleware.on_error
# ===========================================================================


class TestOnErrorInputs:
    def test_on_error_module_id_required(self) -> None:
        """middleware_system.on_error.input.module_id.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.on_error(inputs={}, error=ModuleError(code="X", message="x"), context=_ctx())  # type: ignore[call-arg]

    def test_on_error_inputs_required(self) -> None:
        """middleware_system.on_error.input.inputs.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.on_error("mod.id", error=ModuleError(code="X", message="x"), context=_ctx())  # type: ignore[call-arg]

    def test_on_error_error_required(self) -> None:
        """middleware_system.on_error.input.error.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.on_error("mod.id", {}, context=_ctx())  # type: ignore[call-arg]

    def test_on_error_context_required(self) -> None:
        """middleware_system.on_error.input.context.required"""
        mw = Middleware()
        with pytest.raises(TypeError):
            mw.on_error("mod.id", {}, ModuleError(code="X", message="x"))  # type: ignore[call-arg]


class TestOnErrorReturns:
    def test_on_error_first_dict_wins(self) -> None:
        """middleware_system.on_error.returns.first_recovery_wins

        on_error() runs in reverse over executed middlewares; the first handler
        to return a non-None dict provides recovery and short-circuits the rest.
        """
        sink: list[str] = []

        class Recover(Middleware):
            def __init__(self, label: str, value: dict[str, Any] | None) -> None:
                super().__init__()
                self.label = label
                self.value = value

            def on_error(
                self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
            ) -> dict[str, Any] | None:
                sink.append(self.label)
                return self.value

        # Registration order: A, B, C. Reverse run order: C, B, A.
        a = Recover("A", {"by": "A"})
        b = Recover("B", {"by": "B"})
        c = Recover("C", None)
        mgr = _manager(a, b, c)
        executed = [a, b, c]
        result = mgr.execute_on_error("mod.id", {}, RuntimeError("x"), _ctx(), executed)
        # C ran (returned None), B recovered, A was short-circuited.
        assert result == {"by": "B"}
        assert sink == ["C", "B"]
        assert "A" not in sink

    def test_on_error_none_passthrough_returns_none(self) -> None:
        """middleware_system.on_error.returns.none_passthrough

        When every on_error returns None, the manager returns None (the original
        error keeps propagating).
        """
        mgr = _manager(Middleware(), Middleware())
        executed = mgr.snapshot()
        result = mgr.execute_on_error("mod.id", {}, RuntimeError("x"), _ctx(), executed)
        assert result is None


class TestOnErrorErrors:
    def test_on_error_handler_exception_is_swallowed(self) -> None:
        """middleware_system.on_error.error.handler_must_not_raise

        on_error MUST NOT raise out of the chain: an exception inside one
        handler is logged and iteration continues with the next handler, which
        can still recover.
        """
        sink: list[str] = []

        class Boom(Middleware):
            def on_error(
                self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
            ) -> dict[str, Any] | None:
                sink.append("boom")
                raise RuntimeError("handler blew up")

        class Recover(Middleware):
            def on_error(
                self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
            ) -> dict[str, Any] | None:
                sink.append("recover")
                return {"recovered": True}

        recover = Recover()
        boom = Boom()
        # Reverse run order: boom first (raises, swallowed), then recover.
        mgr = _manager(recover, boom)
        executed = [recover, boom]
        result = mgr.execute_on_error("mod.id", {}, RuntimeError("x"), _ctx(), executed)
        assert result == {"recovered": True}
        assert sink == ["boom", "recover"]


class TestOnErrorProperties:
    async def test_on_error_async_supported(self) -> None:
        """middleware_system.on_error.property.async"""

        class AsyncRecover(Middleware):
            async def on_error(
                self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
            ) -> dict[str, Any]:
                await asyncio.sleep(0)
                return {"recovered_async": True}

        mw = AsyncRecover()
        mgr = _manager(mw)
        result = await mgr.execute_on_error_async("mod.id", {}, RuntimeError("x"), _ctx(), [mw])
        assert result == {"recovered_async": True}

    async def test_on_error_thread_safe_concurrent(self) -> None:
        """middleware_system.on_error.property.thread_safe

        N concurrent on_error recovery passes with distinct errors each yield
        their own recovery output without cross-talk or exception.
        """
        n = 8

        class EchoRecover(Middleware):
            async def on_error(
                self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
            ) -> dict[str, Any]:
                await asyncio.sleep(0)
                return {"echo": inputs["idx"]}

        mw = EchoRecover()
        mgr = _manager(mw)

        async def one(idx: int) -> dict[str, Any] | None:
            return await mgr.execute_on_error_async("mod.id", {"idx": idx}, RuntimeError(str(idx)), _ctx(), [mw])

        results = await asyncio.gather(*(one(i) for i in range(n)))
        assert [r["echo"] for r in results if r] == list(range(n))


class TestOnErrorSideEffects:
    def test_on_error_runs_in_reverse_over_executed(self) -> None:
        """middleware_system.on_error.side_effect.1.reverse_over_executed

        on_error() runs in reverse order over ONLY the executed middlewares.
        If only MW1 and MW2 executed before the failure, on_error runs
        MW2 then MW1 and MW3 is never touched.
        """
        sink: list[str] = []
        mw1 = _Recording("1", sink)
        mw2 = _Recording("2", sink)
        mw3 = _Recording("3", sink)
        mgr = _manager(mw1, mw2, mw3)
        # Simulate failure after mw1, mw2 ran their before() (mw3 did not).
        executed = [mw1, mw2]
        mgr.execute_on_error("mod.id", {}, RuntimeError("x"), _ctx(), executed)
        assert sink == ["on_error:2", "on_error:1"]
        assert "on_error:3" not in sink


# ===========================================================================
# Contract: Middleware.detect_async
# ---------------------------------------------------------------------------
# The spec declares `## Contract: Middleware.detect_async` (a pure, idempotent,
# thread-safe sync predicate that returns True for async handlers). This SDK
# does NOT expose a `Middleware.detect_async` method/function symbol; async
# detection is performed inline in MiddlewareManager via
# `inspect.isawaitable(return_value)` (authoritative) supplemented by
# `inspect.iscoroutinefunction` (fast path). Because the contract references a
# symbol that does not exist here, each clause is recorded as a missing-symbol
# contract gap (skip) so the cross-language row lines up rather than producing
# a coarse import/compile failure.
# ===========================================================================


_DETECT_ASYNC_MISSING = (
    "missing symbol: Middleware.detect_async (contract gap) -- apcore-python has "
    "no standalone detect_async method/function; async detection is inlined in "
    "MiddlewareManager via inspect.isawaitable / inspect.iscoroutinefunction."
)


class TestDetectAsync:
    @pytest.mark.skip(reason=_DETECT_ASYNC_MISSING)
    def test_detect_async_handler_input_required(self) -> None:
        """middleware_system.detect_async.input.handler.required"""
        raise AssertionError("unreachable: skipped contract gap")

    @pytest.mark.skip(reason=_DETECT_ASYNC_MISSING)
    def test_detect_async_returns_bool(self) -> None:
        """middleware_system.detect_async.returns.bool"""
        raise AssertionError("unreachable: skipped contract gap")

    @pytest.mark.skip(reason=_DETECT_ASYNC_MISSING)
    def test_detect_async_property_pure(self) -> None:
        """middleware_system.detect_async.property.pure"""
        raise AssertionError("unreachable: skipped contract gap")

    @pytest.mark.skip(reason=_DETECT_ASYNC_MISSING)
    def test_detect_async_property_idempotent(self) -> None:
        """middleware_system.detect_async.property.idempotent"""
        raise AssertionError("unreachable: skipped contract gap")
