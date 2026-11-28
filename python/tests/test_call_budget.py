"""Budget accounting through SDK call, preflight, and stream entry points."""

import asyncio
from collections.abc import AsyncIterator
from typing import Any

import pytest

from apcore import APCore, CallBudget, CancelToken, Context, Executor, Middleware, Registry
from apcore.approval import ApprovalRequest, ApprovalResult, CallbackApprovalHandler
from apcore.errors import ApprovalPendingError, CallBudgetExceededError, InvalidInputError, ModuleError
from apcore.middleware.base import RetrySignal
from apcore.pipeline import ConfigurationError
from conformance.canonical_fixtures import load_fixture


@pytest.mark.parametrize("case", load_fixture("call_budget.json")["test_cases"], ids=lambda case: case["id"])
async def test_canonical_call_budget(case: dict[str, Any]) -> None:
    client = APCore()
    module_id = case["module_id"]

    @client.module(module_id)
    def account(actual_cost: int | None, ctx: Context[Any]) -> bool:
        """Report the fixture's cost when provided."""
        if actual_cost is not None:
            ctx.record_cost(actual_cost)
        return True

    budget = CallBudget(
        max_calls=case["max_calls"],
        max_cost=case["max_cost"],
        estimated_costs={module_id: case["estimated_cost"]},
    )
    root: Context[Any] = Context.create().with_budget(budget)
    for invocation, expected in zip(case["calls"], case["expected"], strict=True):
        inputs = {"actual_cost": invocation["actual_cost"]}
        if expected["error_code"] is None:
            assert await client.call_async(module_id, inputs, root) == {"result": True}
        else:
            with pytest.raises(CallBudgetExceededError) as denial:
                await client.call_async(module_id, inputs, root)
            assert denial.value.code == expected["error_code"]
        snapshot = budget.snapshot()
        assert snapshot.calls_used == expected["calls_used"]
        assert snapshot.calls_reserved == expected["calls_reserved"]
        assert snapshot.cost_used == expected["cost_used"]


async def test_large_aggregate_snapshot_stays_in_the_portable_integer_range() -> None:
    client = APCore()
    maximum = (1 << 53) - 1
    budget = CallBudget()
    root = Context.create().with_budget(budget)

    @client.module("budget.maximum")
    def charge(ctx: Context[Any]) -> None:
        """Record one invocation's largest portable cost."""
        ctx.record_cost(maximum)

    await client.call_async("budget.maximum", {}, root)
    await client.call_async("budget.maximum", {}, root)
    assert budget.snapshot().calls_used == 2
    assert budget.snapshot().cost_used == maximum


async def test_budget_preflight_is_free_and_actual_cost_replaces_the_estimate() -> None:
    client = APCore()

    @client.module("budget.calculate")
    def calculate(actual: int, ctx: Context[Any]) -> int:
        ctx.record_cost(actual)
        return actual

    estimates = {"budget.calculate": 2}
    budget = CallBudget(max_calls=2, max_cost=5, estimated_costs=estimates)
    estimates["budget.calculate"] = 20
    base: Context[Any] = Context.create()
    ctx = base.with_budget(budget)
    assert ctx.serialize() == base.serialize()
    initial = budget.snapshot()
    assert (initial.calls_used, initial.calls_reserved, initial.cost_used) == (0, 0, 0)
    assert (await client.validate_async("budget.calculate", {"actual": 0}, ctx)).valid
    assert budget.snapshot() == initial

    assert await client.call_async("budget.calculate", {"actual": 0}, ctx) == {"result": 0}
    assert budget.snapshot().cost_used == 0
    with pytest.raises(ModuleError) as denied:
        await client.call_async("budget.calculate", {"actual": 6}, ctx)
    assert denied.value.code == "CALL_BUDGET_EXCEEDED"
    assert denied.value.details["dimension"] == "cost"
    assert denied.value.retryable is False
    spent = budget.snapshot()
    assert (spent.calls_used, spent.calls_reserved, spent.cost_used) == (2, 0, 6)


async def test_budget_holds_a_concurrent_slot_then_releases_on_pending() -> None:
    executor = Executor(Registry())
    client = APCore(executor=executor)
    entered, unblock = asyncio.Event(), asyncio.Event()

    async def approve(request: ApprovalRequest) -> ApprovalResult:
        entered.set()
        await unblock.wait()
        return ApprovalResult("pending", approval_id="review-1")

    executor.set_approval_handler(CallbackApprovalHandler(approve))

    @client.module("budget.pending", requires_approval=True)
    def pending() -> int:
        raise AssertionError("Pending approval must never execute")

    @client.module("budget.direct")
    def direct() -> int:
        return 1

    budget = CallBudget(max_calls=1)
    ctx: Context[Any] = Context.create().with_budget(budget)
    held = asyncio.create_task(client.call_async("budget.pending", {}, ctx))
    await entered.wait()
    assert budget.snapshot().calls_reserved == 1
    with pytest.raises(ModuleError) as denied:
        await client.call_async("budget.direct", {}, ctx)
    assert denied.value.details["dimension"] == "calls"
    unblock.set()
    with pytest.raises(ApprovalPendingError):
        await held
    assert budget.snapshot().calls_reserved == budget.snapshot().calls_used == 0
    assert await client.call_async("budget.direct", {}, ctx) == {"result": 1}


async def test_budget_charges_started_retries_and_counts_streams_once() -> None:
    executor = Executor(Registry())
    client = APCore(executor=executor)

    @client.module("budget.retry")
    def fail(retry: bool) -> bool:
        if not retry:
            raise ValueError("once")
        return retry

    class RetryOnce(Middleware):
        def on_error(
            self, module_id: str, inputs: dict[str, Any], error: Exception, context: Any
        ) -> RetrySignal | None:
            return RetrySignal({"retry": True}) if not inputs.get("retry") else None

    client.use(RetryOnce())
    budget = CallBudget(max_calls=3, estimated_costs={"budget.retry": 2, "budget.stream": 1})
    ctx: Context[Any] = Context.create().with_budget(budget)
    assert (await executor.call_async_with_trace("budget.retry", {"retry": False}, ctx))[0] == {"result": True}
    assert budget.snapshot().calls_used == 2
    assert budget.snapshot().cost_used == 4

    class Stream:
        description = "Return streaming results"
        input_schema = output_schema = {"type": "object"}
        annotations = {"streaming": True}

        def execute(self, inputs: dict[str, Any], context: Context[Any]) -> dict[str, Any]:
            raise AssertionError("Streaming must not invoke execute")

        async def stream(self, inputs: dict[str, Any], context: Context[Any]) -> AsyncIterator[dict[str, Any]]:
            context.record_cost(4)
            yield {"first": True}
            yield {"second": True}

    client.register("budget.stream", Stream())
    stream = client.stream("budget.stream", {}, ctx)
    assert budget.snapshot().calls_used == 2
    assert [chunk async for chunk in stream] == [{"first": True}, {"second": True}]
    spent = budget.snapshot()
    assert (spent.calls_used, spent.calls_reserved, spent.cost_used) == (3, 0, 8)


async def test_budget_releases_a_reservation_on_pre_dispatch_cancellation() -> None:
    from apcore.cancel import ExecutionCancelledError

    client = APCore()
    token = CancelToken()

    @client.module("budget.cancel")
    def never() -> int:
        raise AssertionError("Cancelled execution must not dispatch")

    class Cancel(Middleware):
        def before(self, module_id: str, inputs: dict[str, Any], context: Context[Any]) -> None:
            token.cancel()

    client.use(Cancel())
    budget = CallBudget(max_calls=1)
    base: Context[Any] = Context.create(cancel_token=token)
    with pytest.raises(ExecutionCancelledError):
        await client.call_async("budget.cancel", {}, base.with_budget(budget))
    assert budget.snapshot().calls_reserved == budget.snapshot().calls_used == 0


async def test_budget_rejects_missing_guard_and_is_shared_by_nested_calls() -> None:
    executor = Executor(Registry())
    client = APCore(executor=executor)
    entered: list[str] = []

    @client.module("budget.parent")
    async def parent(ctx: Context[Any]) -> bool:
        entered.append("parent")
        await client.call_async("budget.child", {}, ctx)
        return True

    @client.module("budget.child")
    async def child() -> bool:
        entered.append("child")
        return True

    budget = CallBudget(max_calls=1)
    ctx: Context[Any] = Context.create().with_budget(budget)
    with pytest.raises(ConfigurationError):
        await executor.call_async_with_trace("budget.parent", {}, ctx, strategy="minimal")
    assert entered == []
    with pytest.raises(CallBudgetExceededError):
        await client.call_async("budget.parent", {}, ctx)
    assert entered == ["parent"]
    assert budget.snapshot().calls_used == 1


def test_call_cost_must_use_nonnegative_safe_integers() -> None:
    for value in (-1, True, 1 << 53):
        with pytest.raises(InvalidInputError):
            CallBudget(max_calls=value)
        with pytest.raises(InvalidInputError):
            CallBudget(estimated_costs={"budget.action": value})
