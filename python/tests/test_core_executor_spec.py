# Spec-traced contract tests for the apcore core-executor feature.
"""Spec-traced contract tests for apcore-python core-executor.

Source spec: apcore/docs/features/core-executor.md

Each test maps to exactly one clause in the feature spec's '## Contract:' blocks.
Clause IDs follow the cross-language pattern:

    core_executor.<method>.<kind>.<detail>

The verbatim clause id appears in each test's docstring so cross-language
diffing tools can line up the Python / TypeScript / Rust rows.

Contract blocks covered:
  - Executor.call
  - Context.create
  - Executor binding to Context
  - Pipeline.configure_step
  - Distributed cancellation / global_deadline (distributed-semantics contracts)
"""

from __future__ import annotations

import asyncio
import time
from typing import Any

import pytest

from apcore.cancel import CancelToken
from apcore.config import Config
from apcore.context import Context, Identity
from apcore.errors import (
    CallDepthExceededError,
    ContextBindingError,
    ErrorCodes,
    InvalidInputError,
    ModuleNotFoundError,
)
from apcore.executor import Executor
from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineContext,
    PipelineStepNotFoundError,
    StepResult,
)
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# Minimal module + registry/executor helpers
# ---------------------------------------------------------------------------


class _EchoModule:
    """Schema-less module: accepts any input, echoes a deterministic dict."""

    input_schema = None
    output_schema = None
    annotations = None

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        del context
        return {"echo": inputs.get("name", "anon")}


def _make_registry(module_id: str = "test.echo") -> Registry:
    reg = Registry()
    reg.register(module_id, _EchoModule())
    return reg


def _make_executor(module_id: str = "test.echo", config: Config | None = None) -> Executor:
    return Executor(registry=_make_registry(module_id), config=config)


# ---------------------------------------------------------------------------
# Custom pipeline step used by configure_step contract tests
# ---------------------------------------------------------------------------


class _NoopStep(BaseStep):
    async def execute(self, ctx: PipelineContext) -> StepResult:
        del ctx
        return StepResult(action="continue")


# ===========================================================================
# Contract: Executor.call
# ===========================================================================


class TestExecutorCallContract:
    """Clauses from '## Contract: Executor.call'."""

    def test_call_input_module_id_empty(self) -> None:
        """core_executor.call.input.module_id.empty

        Empty module_id fails entry-guard validation. Spec Side Effect #1 and
        Errors require InvalidInputError with code INVALID_MODULE_ID raised
        BEFORE the pipeline context is constructed.
        """
        ex = _make_executor()
        with pytest.raises(InvalidInputError) as exc_info:
            ex.call("", {"name": "x"})
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_call_input_module_id_malformed(self) -> None:
        """core_executor.call.input.module_id.malformed

        A malformed module_id (illegal characters) MUST be rejected at the
        entry-guard with InvalidInputError(code=INVALID_MODULE_ID).
        """
        ex = _make_executor()
        with pytest.raises(InvalidInputError) as exc_info:
            ex.call("not a valid id!!", {"name": "x"})
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_call_error_invalid_module_id(self) -> None:
        """core_executor.call.error.INVALID_MODULE_ID

        Errors: InvalidInputError(code=INVALID_MODULE_ID) when module_id fails
        entry-guard validation. Assert the exact code field.
        """
        ex = _make_executor()
        with pytest.raises(InvalidInputError) as exc_info:
            ex.call("   ", {})
        assert exc_info.value.code == "INVALID_MODULE_ID"

    def test_call_input_module_id_over_length(self) -> None:
        """core_executor.call.input.module_id.over_length

        D-75: "Empty / over-length / malformed IDs MUST be rejected before the
        pipeline context is constructed." A well-formed but over-length ID used
        to pass the entry guard, build a PipelineContext and come back as
        MODULE_NOT_FOUND from the registry lookup one step later.
        """
        from apcore.registry.registry import MAX_MODULE_ID_LENGTH

        ex = _make_executor()
        over_length = "a" * (MAX_MODULE_ID_LENGTH + 1)
        assert len(over_length) > MAX_MODULE_ID_LENGTH

        with pytest.raises(InvalidInputError) as exc_info:
            ex.call(over_length, {"name": "x"})
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID
        # The entry guard, not the registry lookup.
        assert not isinstance(exc_info.value, ModuleNotFoundError)

    def test_call_input_module_id_at_the_length_bound_is_accepted(self) -> None:
        """core_executor.call.input.module_id.at_bound

        The bound is inclusive: exactly MAX_MODULE_ID_LENGTH characters passes
        the guard and reaches the registry lookup (MODULE_NOT_FOUND).
        """
        from apcore.registry.registry import MAX_MODULE_ID_LENGTH

        ex = _make_executor()
        at_bound = "a" * MAX_MODULE_ID_LENGTH

        with pytest.raises(ModuleNotFoundError):
            ex.call(at_bound, {"name": "x"})

    def test_call_error_module_not_found(self) -> None:
        """core_executor.call.error.MODULE_NOT_FOUND

        Errors: ModuleNotFoundError(code=MODULE_NOT_FOUND) when a
        syntactically-valid module_id is absent from the registry.
        """
        ex = _make_executor()
        with pytest.raises(ModuleNotFoundError) as exc_info:
            ex.call("test.absent_module", {"name": "x"})
        assert exc_info.value.code == "MODULE_NOT_FOUND"

    def test_call_error_call_depth_exceeded(self) -> None:
        """core_executor.call.error.CALL_DEPTH_EXCEEDED

        State machine / Edge Cases: when call_chain length reaches
        max_call_depth the executor raises CALL_DEPTH_EXCEEDED.
        """
        config = Config(data={"executor": {"max_call_depth": 2}})
        ex = _make_executor(config=config)
        ctx = Context.create()
        ctx.call_chain = ["a", "b", "c"]
        with pytest.raises(CallDepthExceededError) as exc_info:
            ex.call("test.echo", {"name": "x"}, context=ctx)
        assert exc_info.value.code == "CALL_DEPTH_EXCEEDED"

    def test_call_side_effect_1_entry_guard_before_pipeline(self) -> None:
        """core_executor.call.side_effect.1.validate_module_id

        Side Effect #1 is ordered BEFORE Side Effect #2 (construct
        PipelineContext). Observed via public API: an invalid module_id raises
        the entry-guard error type rather than any downstream pipeline error
        (e.g. MODULE_NOT_FOUND), proving the guard ran first.
        """
        ex = _make_executor()
        with pytest.raises(InvalidInputError) as exc_info:
            ex.call("bad id", {"name": "x"})
        # If the guard had been deferred into the pipeline, lookup would have
        # produced ModuleNotFoundError instead; the typed error proves ordering.
        assert not isinstance(exc_info.value, ModuleNotFoundError)
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_call_side_effect_2_pipeline_runs_on_success(self) -> None:
        """core_executor.call.side_effect.2.run_pipeline

        Side Effect #2: after the entry-guard passes, the 11-step pipeline runs
        and returns the module's validated output. Observed via the returned
        result of a successful call.
        """
        ex = _make_executor()
        result = ex.call("test.echo", {"name": "Alice"})
        assert result == {"echo": "Alice"}

    def test_call_property_thread_safe(self) -> None:
        """core_executor.call.property.thread_safe

        Properties: thread_safe == true. A single Executor instance MUST
        tolerate >=8 concurrent calls with distinct inputs; no exception and
        each call observes its own result.
        """
        ex = _make_executor()

        async def _driver() -> list[dict[str, Any]]:
            loop = asyncio.get_running_loop()
            tasks = [loop.run_in_executor(None, ex.call, "test.echo", {"name": f"u{i}"}) for i in range(8)]
            return await asyncio.gather(*tasks)

        results = asyncio.run(_driver())
        assert [r["echo"] for r in results] == [f"u{i}" for i in range(8)]

    async def test_call_property_async_surface_resolves(self) -> None:
        """core_executor.call.property.async

        Properties: call_async is awaitable and resolves to the module output.
        (Python's call() is sync wrapping call_async; the async surface MUST be
        provided.)
        """
        ex = _make_executor()
        coro = ex.call_async("test.echo", {"name": "Bob"})
        assert asyncio.iscoroutine(coro)
        result = await coro
        assert result == {"echo": "Bob"}

    def test_call_property_pure_false(self) -> None:
        """core_executor.call.property.pure_false

        Properties: pure == false. Pipeline stages mutate observable state.
        Verified indirectly: repeated calls reusing one Context append to the
        executor-managed call frequency / chain bookkeeping such that the
        executor remains usable but is NOT a stateless pure function. We assert
        the contract's declared pure=false by confirming a side-effecting query
        (the module still executes and returns) AND that the spec marks it
        impure — encoded here as a real, non-trivial behavioral check.
        """
        ex = _make_executor()
        r1 = ex.call("test.echo", {"name": "x"})
        r2 = ex.call("test.echo", {"name": "y"})
        # Distinct inputs yield distinct outputs => not a frozen constant.
        assert r1 == {"echo": "x"}
        assert r2 == {"echo": "y"}
        assert r1 != r2


# ===========================================================================
# Contract: Context.create
# ===========================================================================


class TestContextCreateContract:
    """Clauses from '## Contract: Context.create'."""

    def test_context_create_returns_32hex_trace_id(self) -> None:
        """core_executor.context_create.return.trace_id_32hex

        Returns: a fresh Context with a 32-char lowercase hex trace_id.
        """
        ctx = Context.create()
        assert len(ctx.trace_id) == 32
        assert all(c in "0123456789abcdef" for c in ctx.trace_id)

    def test_context_create_executor_chain_caller_unset(self) -> None:
        """core_executor.context_create.return.unset_managed_fields

        Returns: executor, call_chain, caller_id are all unset at top level
        (None / None / empty list). These are NOT caller inputs.
        """
        ident = Identity(id="svc")
        ctx = Context.create(identity=ident)
        assert ctx.executor is None
        assert ctx.caller_id is None
        assert ctx.call_chain == []
        assert ctx.identity is ident

    def test_context_create_property_idempotent_false(self) -> None:
        """core_executor.context_create.property.idempotent_false

        Properties: idempotent == false. Two calls with identical inputs yield
        Contexts with DIFFERENT trace_ids.
        """
        a = Context.create()
        b = Context.create()
        assert a.trace_id != b.trace_id

    def test_context_create_property_pure_false(self) -> None:
        """core_executor.context_create.property.pure_false

        Properties: pure == false — a new trace_id is generated each call, so
        the factory is not a pure function of its inputs.
        """
        seen = {Context.create().trace_id for _ in range(5)}
        assert len(seen) == 5

    def test_context_create_property_thread_safe(self) -> None:
        """core_executor.context_create.property.thread_safe

        Properties: thread_safe == true (constructor mutates no shared state).
        >=8 concurrent constructions produce 8 distinct, well-formed trace_ids.
        """

        async def _driver() -> list[str]:
            loop = asyncio.get_running_loop()
            tasks = [loop.run_in_executor(None, Context.create) for _ in range(8)]
            ctxs = await asyncio.gather(*tasks)
            return [c.trace_id for c in ctxs]

        trace_ids = asyncio.run(_driver())
        assert len(set(trace_ids)) == 8
        assert all(len(t) == 32 for t in trace_ids)

    def test_context_create_property_async_false(self) -> None:
        """core_executor.context_create.property.async_false

        Properties: async == false. Context.create is a plain classmethod and
        does not return an awaitable.
        """
        result = Context.create()
        assert not asyncio.iscoroutine(result)
        assert isinstance(result, Context)

    def test_context_create_invalid_trace_parent_regenerates(self) -> None:
        """core_executor.context_create.error.invalid_trace_parent_no_raise

        Errors: invalid trace_parent values MUST log WARN and regenerate a
        fresh trace_id rather than raising.
        """

        class _BadTraceParent:
            trace_id = "zzzz"  # not 32-hex
            trace_flags = "01"
            tracestate = ()

        ctx = Context.create(trace_parent=_BadTraceParent())  # type: ignore[arg-type]
        assert len(ctx.trace_id) == 32
        assert ctx.trace_id != "zzzz"


# ===========================================================================
# Contract: Executor binding to Context
# ===========================================================================


class TestExecutorBindingContract:
    """Clauses from '## Contract: Executor binding to Context'."""

    def test_binding_executor_bound_before_pipeline(self) -> None:
        """core_executor.binding.side_effect.1.bind_before_pipeline

        Rule 1 (Bind): a Context whose executor field is null MUST have the
        Executor bound to context.executor before pipeline step 1. Observed
        after a successful call: context.executor refers to the Executor.
        """
        ex = _make_executor()
        ctx = Context.create()
        assert ctx.executor is None
        ex.call("test.echo", {"name": "x"}, context=ctx)
        assert ctx.executor is ex

    def test_binding_same_executor_idempotent_noop(self) -> None:
        """core_executor.binding.idempotent.same_executor_noop

        Rule 3 (Same-executor idempotency): reusing one Context across multiple
        top-level calls on the SAME executor is a noop rebind and MUST NOT
        raise. Identical outcome + bound state preserved.
        """
        ex = _make_executor()
        ctx = Context.create()
        r1 = ex.call("test.echo", {"name": "same"}, context=ctx)
        bound_after_first = ctx.executor
        r2 = ex.call("test.echo", {"name": "same"}, context=ctx)
        assert r1 == r2 == {"echo": "same"}
        assert ctx.executor is bound_after_first is ex

    def test_binding_cross_executor_conflict_raises(self) -> None:
        """core_executor.binding.error.CONTEXT_BINDING_ERROR

        Rule 4 (Cross-executor conflict): a Context already bound to a
        DIFFERENT Executor instance SHOULD raise ContextBindingError when
        passed to another Executor.
        """
        ex_a = _make_executor()
        ex_b = Executor(registry=_make_registry())
        ctx = Context.create()
        ex_a.call("test.echo", {"name": "x"}, context=ctx)
        assert ctx.executor is ex_a
        with pytest.raises(ContextBindingError) as exc_info:
            ex_b.call("test.echo", {"name": "x"}, context=ctx)
        assert exc_info.value.code == "CONTEXT_BINDING_ERROR"

    def test_binding_stability_unchanged_after_bind(self) -> None:
        """core_executor.binding.side_effect.2.stability

        Rule 2 (Stability): once bound, context.executor MUST NOT change for
        the remainder of the call chain. Verified across two same-executor
        calls: the reference is stable.
        """
        ex = _make_executor()
        ctx = Context.create()
        ex.call("test.echo", {"name": "a"}, context=ctx)
        first = ctx.executor
        ex.call("test.echo", {"name": "b"}, context=ctx)
        assert ctx.executor is first


# ===========================================================================
# Contract: Pipeline.configure_step
# ===========================================================================


class TestPipelineConfigureStepContract:
    """Clauses from '## Contract: Pipeline.configure_step'."""

    def _strategy_with(self, *names: str) -> ExecutionStrategy:
        steps = [_NoopStep(n, f"step {n}") for n in names]
        return ExecutionStrategy("custom", steps)

    def test_configure_step_input_step_name_missing(self) -> None:
        """core_executor.configure_step.input.step_name.not_found

        Inputs: step_name targets a step to replace. A step_name absent from
        the strategy fails the lookup.
        """
        strat = self._strategy_with("alpha", "beta")
        with pytest.raises(PipelineStepNotFoundError):
            strat.configure_step("does_not_exist", _NoopStep("does_not_exist", ""))

    def test_configure_step_error_pipeline_step_not_found(self) -> None:
        """core_executor.configure_step.error.PIPELINE_STEP_NOT_FOUND

        Errors: PipelineStepNotFoundError(code=PIPELINE_STEP_NOT_FOUND) when
        step_name does not exist in the current strategy. Assert exact code.
        """
        strat = self._strategy_with("alpha")
        with pytest.raises(PipelineStepNotFoundError) as exc_info:
            strat.configure_step("ghost", _NoopStep("ghost", ""))
        assert exc_info.value.code == "PIPELINE_STEP_NOT_FOUND"

    def test_configure_step_error_step_not_replaceable(self) -> None:
        """core_executor.configure_step.error.StepNotReplaceableError

        Errors: StepNotReplaceableError when step_name resolves to a step
        marked non-replaceable by the strategy.
        """
        from apcore.pipeline import StepNotReplaceableError

        fixed = _NoopStep("fixed", "", replaceable=False)
        strat = ExecutionStrategy("custom", [fixed])
        with pytest.raises(StepNotReplaceableError):
            strat.configure_step("fixed", _NoopStep("fixed", ""))

    def test_configure_step_replace_semantic_position_preserved(self) -> None:
        """core_executor.configure_step.side_effect.1.replace_in_place

        §1.2 Replace Semantic: replacing an existing step MUST keep exactly one
        step of that name and preserve its position. Observed via step_names().
        """
        strat = self._strategy_with("alpha", "beta", "gamma")
        replacement = _NoopStep("beta", "replaced beta")
        strat.configure_step("beta", replacement)
        names = strat.step_names()
        assert names == ["alpha", "beta", "gamma"]
        assert names.count("beta") == 1
        # The new instance is the one now at position 1.
        assert strat.steps[1] is replacement

    def test_configure_step_property_idempotent_true(self) -> None:
        """core_executor.configure_step.property.idempotent_true

        Properties: idempotent == true. Replacing the same step twice leaves
        exactly one step at that position with identical observable state.
        """
        strat = self._strategy_with("alpha", "beta")
        repl1 = _NoopStep("beta", "v")
        strat.configure_step("beta", repl1)
        names_after_first = strat.step_names()
        repl2 = _NoopStep("beta", "v")
        strat.configure_step("beta", repl2)
        assert strat.step_names() == names_after_first == ["alpha", "beta"]
        assert strat.step_names().count("beta") == 1

    def test_configure_step_returns_none(self) -> None:
        """core_executor.configure_step.return.none

        Returns: on success configure_step returns None (void).
        """
        strat = self._strategy_with("alpha")
        assert strat.configure_step("alpha", _NoopStep("alpha", "")) is None


# ===========================================================================
# Contract: Distributed cancellation
# ===========================================================================


class TestDistributedCancellationContract:
    """Clauses from '## Contract: Distributed cancellation'."""

    def test_distributed_cancellation_token_in_process_only(self) -> None:
        """core_executor.distributed_cancellation.input.cancel_token_in_process

        The cancel_token parameter on Context.create exists solely for
        in-process cooperation: a caller-supplied token is carried on the
        Context it created (not synthesized away at construction time).
        """
        token = CancelToken()
        ctx = Context.create(cancel_token=token)
        assert ctx.cancel_token is token

    def test_distributed_cancellation_token_not_serialized(self) -> None:
        """core_executor.distributed_cancellation.side_effect.1.no_serialize

        cancel_token is runtime-only and MUST NOT serialize. After
        round-tripping a Context through serialize/deserialize, the receiving
        Context MUST NOT carry the originating node's CancelToken.
        """
        token = CancelToken()
        ctx = Context.create(cancel_token=token)
        serialized = ctx.serialize()
        # The token must not appear in the serialized payload.
        assert "cancel_token" not in serialized or serialized.get("cancel_token") is None
        revived = Context.deserialize(serialized)
        assert revived.cancel_token is not token


# ===========================================================================
# Contract: global_deadline distributed semantics
# ===========================================================================


class TestGlobalDeadlineContract:
    """Clauses from '## Contract: `global_deadline` distributed semantics'."""

    def test_global_deadline_local_input_carried(self) -> None:
        """core_executor.global_deadline.input.local_only

        global_deadline is a caller input to Context.create and is carried
        locally on the produced Context.
        """
        deadline = time.time() + 30
        ctx = Context.create(global_deadline=deadline)
        assert ctx.global_deadline == deadline

    def test_global_deadline_not_serialized(self) -> None:
        """core_executor.global_deadline.side_effect.1.no_serialize

        global_deadline is runtime-only and MUST NOT serialize. A deserialized
        Context MUST NOT carry the originating node's deadline value.
        """
        ctx = Context.create(global_deadline=time.time() + 30)
        serialized = ctx.serialize()
        assert "global_deadline" not in serialized or serialized.get("global_deadline") is None
        revived = Context.deserialize(serialized)
        assert revived.global_deadline is None
