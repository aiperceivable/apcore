"""Public typed declaration tests through the stock client and executor."""

from __future__ import annotations

from inspect import iscoroutinefunction, signature
from typing import Annotated, Any, Literal, assert_type
from uuid import UUID

import pytest
from pydantic import BaseModel, ConfigDict, Field

from apcore import (
    ACL,
    ACLRule,
    APCore,
    ApprovalRequest,
    ApprovalResult,
    AutoApproveHandler,
    Context,
    Executor,
    Identity,
    PolicyEvaluationRequest,
    PolicyVerdict,
    PreflightResult,
    PreviewResult,
    Registry,
)
from apcore.errors import (
    ACLDeniedError,
    ApprovalDeniedError,
    ApprovalPendingError,
    PolicyDeniedError,
    PolicyEvaluationError,
    SchemaValidationError,
)
from apcore.module import Change


class Order(BaseModel):
    model_config = ConfigDict(strict=True)
    id: UUID = Field(alias="order_id")


class Receipt(BaseModel):
    order: Order
    quantity: int


class NativePayload(BaseModel):
    value: Annotated[str | UUID, Field(union_mode="left_to_right")]


def test_bound_module_preserves_native_results_and_callable_metadata() -> None:
    client = APCore()

    def save(order: Order, /, quantity: int = 1, *, multiplier: int = 2) -> Receipt:
        """Save an order."""
        assert isinstance(order, Order)
        return Receipt(order=order, quantity=quantity * multiplier)

    bound = client.module("orders.save")(save)
    order = Order(order_id=UUID("23416db0-3d95-44b0-8072-578bed43391a"))
    expected = Receipt(order=order, quantity=3)

    assert assert_type(bound.invoke(order, multiplier=3), Receipt) == expected
    assert assert_type(bound.raw(order, multiplier=3), Receipt) == expected
    assert bound(order, multiplier=3) == expected
    assert bound.__name__ == save.__name__
    assert bound.__doc__ == save.__doc__
    assert signature(bound) == signature(save)
    assert bound.apcore_module is client.registry.get("orders.save")
    assert client.call("orders.save", {"order": {"order_id": str(order.id)}, "multiplier": 3}) == {
        "order": {"order_id": str(order.id)},
        "quantity": 3,
    }

    with pytest.raises(TypeError):
        bound.invoke(order=order)  # type: ignore[call-arg]
    with pytest.raises(SchemaValidationError):
        bound.invoke(order, quantity="2")  # type: ignore[arg-type]


def test_bound_module_keeps_the_native_union_branch() -> None:
    client = APCore()

    @client.module("payload.classify")
    def classify(payload: NativePayload) -> str:
        return type(payload.value).__name__

    value = UUID("23416db0-3d95-44b0-8072-578bed43391a")
    payload = NativePayload(value=value)
    assert classify.raw(payload) == "UUID"
    assert client.call("payload.classify", {"payload": payload}) == {"result": "UUID"}
    assert classify.invoke(payload) == "UUID"
    assert classify.invoke(NativePayload(value=str(value))) == "str"


def test_final_policy_rejects_a_native_union_branch_with_a_colliding_json_projection() -> None:
    calls: list[dict[str, Any]] = []

    class Evaluator:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            calls.append(request.arguments)
            return PolicyVerdict("allow")

    client = APCore(executor=Executor(Registry(), policy_evaluator=Evaluator()))

    @client.module("payload.classify")
    def classify(payload: NativePayload) -> str:
        return type(payload.value).__name__

    value = UUID("23416db0-3d95-44b0-8072-578bed43391a")
    assert classify.invoke(NativePayload(value=str(value))) == "str"
    calls.clear()
    payload = NativePayload(value=value)
    preflight = classify.validate(payload)
    assert [error["code"] for error in preflight.errors] == ["POLICY_EVALUATION_ERROR"]
    with pytest.raises(PolicyEvaluationError):
        client.call("payload.classify", {"payload": payload})
    with pytest.raises(PolicyEvaluationError):
        classify.invoke(payload)
    assert calls == []


def test_bound_module_routes_acl_approval_and_preview_through_executor() -> None:
    executor = Executor(Registry())
    client = APCore(executor=executor)
    executions: list[int] = []
    previews: list[int] = []

    def preview(inputs: dict[str, Any], context: Context[Any]) -> PreviewResult:
        assert context.identity is not None and context.identity.type == "system"
        previews.append(inputs["quantity"])
        return PreviewResult(changes=[Change(action="update", target="inventory", summary="Update the quantity")])

    @client.module("inventory.update", destructive=True, requires_approval=True, preview=preview)
    def update(quantity: int) -> int:
        executions.append(quantity)
        return quantity

    executor.set_acl(ACL([ACLRule(callers=["@system"], targets=["inventory.update"], effect="allow")]))
    assert not update.validate(3).valid
    assert update.preview(3).changes == []
    with pytest.raises(ACLDeniedError):
        update.invoke(3)
    assert previews == executions == []

    context: Context[Any] = Context.create(identity=Identity(id="stock-system", type="system"))
    authorized = update.with_context(context)
    validation = assert_type(authorized.validate(quantity=3), PreflightResult)
    assert validation.valid and validation.requires_approval
    assert [change.summary for change in authorized.preview(3).changes] == ["Update the quantity"]
    assert previews == [3, 3]
    with pytest.raises(ApprovalDeniedError):
        authorized.invoke(3)
    assert executions == []

    executor.set_approval_handler(AutoApproveHandler())
    assert authorized.invoke(3) == 3
    assert update.raw(4) == 4
    assert executions == [3, 4]


async def test_async_bound_method_preserves_raw_awaitability_and_governed_result() -> None:
    client = APCore()

    class Greeter:
        prefix = "hello"

        async def greet(self, name: str, **labels: str) -> str:
            return f"{self.prefix}, {labels['title']} {name}"

    greet = client.module("greet.person")(Greeter().greet)

    assert iscoroutinefunction(greet)
    assert await greet("Ada", title="Dr") == "hello, Dr Ada"
    assert await greet.raw("Ada", title="Dr") == "hello, Dr Ada"
    assert assert_type(await greet.invoke_async("Ada", title="Dr"), str) == "hello, Dr Ada"
    assert (await greet.validate_async("Ada", title="Dr")).valid
    assert (await greet.preview_async("Ada", title="Dr")).changes == []


def test_multiple_bindings_of_same_function_keep_independent_clients() -> None:
    first = APCore(executor=Executor(Registry(), acl=ACL()))
    second = APCore()

    def get() -> int:
        return 1

    blocked = first.module("test.get")(get)
    allowed = second.module("test.get")(get)

    with pytest.raises(ACLDeniedError):
        blocked.invoke()
    assert allowed.invoke() == 1
    assert blocked() == 1


def test_bound_module_injects_typed_context_outside_the_input_schema() -> None:
    client = APCore()

    @client.module("test.identify")
    def identify(context: Context[Any], /, fallback: str = "anonymous") -> str:
        return context.identity.id if context.identity is not None else fallback

    context: Context[Any] = Context.create(identity=Identity(id="operator"))
    assert identify.invoke(context) == "operator"
    assert identify.invoke(fallback="guest") == "guest"  # type: ignore[call-arg]
    assert identify.with_context(context).invoke() == "operator"  # type: ignore[call-arg]
    assert identify.raw(context) == "operator"
    assert client.call("test.identify", {}) == {"result": "anonymous"}


@pytest.mark.asyncio
async def test_bound_module_reuses_the_final_policy_and_approval_snapshot() -> None:
    order = Order(order_id=UUID("23416db0-3d95-44b0-8072-578bed43391a"))
    expected = {"order": {"order_id": str(order.id)}, "quantity": 3}
    decisions: list[str] = []
    invocations: list[Receipt] = []

    class Evaluator:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            decision: Literal["require_approval", "deny"] = (
                "require_approval" if request.arguments == expected else "deny"
            )
            decisions.append(decision)
            return PolicyVerdict(decision)

    class Handler:
        supports_final_scope = True
        digest: str | None = None

        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            assert request.arguments == expected
            self.digest = request.canonical_args_digest
            assert self.digest is not None
            return ApprovalResult("pending", approval_id="pending-1")

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            raise AssertionError("Final approval requires a scoped check")

        async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
            assert request.arguments == expected
            assert request.canonical_args_digest == self.digest
            return ApprovalResult("approved" if approval_id == "pending-1" else "rejected")

    client = APCore(executor=Executor(Registry(), policy_evaluator=Evaluator(), approval_handler=Handler()))

    @client.module("orders.process", destructive=True)
    def process(order: Order, quantity: int) -> Receipt:
        result = Receipt(order=order, quantity=quantity)
        invocations.append(result)
        return result

    assert (await process.validate_async(order, 3)).requires_approval
    with pytest.raises(PolicyDeniedError):
        await process.invoke_async(order, 4)
    with pytest.raises(ApprovalPendingError) as pending:
        await process.invoke_async(order, 3)
    assert invocations == []
    assert pending.value.approval_id == "pending-1"
    receipt = await process.with_approval_token(pending.value.approval_id).invoke_async(order, 3)
    assert invocations == [receipt] == [Receipt(order=order, quantity=3)]
    assert decisions == ["require_approval", "deny", "require_approval", "require_approval"]
