"""Observable contracts for scoped and delegated callers."""

from __future__ import annotations

from collections.abc import AsyncIterator
from typing import Any

import pytest

from apcore.acl import ACL, ACLRule
from apcore.approval import ApprovalRequest, ApprovalResult
from apcore.builtin_steps import build_standard_strategy
from apcore.context import Context, Identity
from apcore.errors import ACLDeniedError, ApprovalDeniedError, ApprovalPendingError, InvalidInputError
from apcore.executor import Executor
from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineContext,
    PipelineState,
    StepResult,
    _ScopedFrame,
    _ScopedSteps,
)
from apcore.policy import PolicyEvaluationRequest, PolicyVerdict
from apcore.registry import Registry
from apcore.sys_modules.identity import _audit_payload_extras


class _Module:
    description = "Delegation test"
    input_schema = {"type": "object"}
    output_schema = {"type": "object"}

    async def execute(self, inputs, context):
        return {"actor": context.identity.id}


def _executor(*rules: ACLRule) -> Executor:
    registry = Registry()
    registry.register("test.run", _Module())
    return Executor(registry, acl=ACL(list(rules), default_effect="allow") if rules else None)


def _allow(conditions: dict | None = None) -> ACLRule:
    return ACLRule(["*"], ["test.run"], "allow", conditions=conditions)


def test_identity_delegation_round_trips_and_preserves_legacy_wire() -> None:
    legacy = Identity("root", "user", ("owner",), {"note": "ordinary"})
    assert "scopes" not in Context.create(legacy).serialize()["identity"]
    assert legacy.authority_key() is None
    assert legacy.owner_key() is None
    scopes = ["write", "read", "read"]
    root = Identity("root", "user", ("owner",), {}, scopes=scopes, issuer="https://issuer.example")
    scopes.clear()
    actor = Identity("agent", "ai", ("worker",))
    delegated = root.delegate(actor, [])
    assert root.scopes == ("read", "write")
    assert delegated.scopes == ()
    assert delegated.root_subject == root
    assert delegated.delegation_depth == 1
    assert Context.deserialize(Context.create(delegated).serialize()).identity == delegated
    assert delegated.owner_key() == root.delegate(actor, ["read"]).owner_key()
    assert delegated.authority_key() != root.delegate(actor, ["read"]).authority_key()


def test_identity_delegation_rejects_malformed_authority() -> None:
    root = Identity("same", scopes=["read"])
    delegated = root
    for _ in range(8):
        delegated = delegated.delegate(Identity("same"), ["read"])
    assert delegated.delegation_depth == 8
    for bad in (
        lambda: delegated.delegate(Identity("same"), []),
        lambda: root.delegate(Identity("same"), ["write"]),
        lambda: root.delegate(Identity("same", scopes=[]), ["read"]),
        lambda: Identity("same").delegate(Identity("same"), []),
        lambda: Identity("same", on_behalf_of=root),
        lambda: Identity("", scopes=["read"]),
        lambda: Identity("same", "", scopes=["read"]),
        lambda: Identity("same", roles=("",), scopes=["read"]),
    ):
        with pytest.raises(InvalidInputError):
            bad()
    encoded = Context.create(root).serialize()
    encoded["identity"]["on_behalf_of"] = encoded["identity"]
    with pytest.raises(InvalidInputError):
        Context.deserialize(encoded)


@pytest.mark.asyncio
async def test_scoped_executor_and_discovery_require_first_stock_scope_grant() -> None:
    root = Identity("root", "user", ("owner",), scopes=["Read", "write"])
    caller = root.delegate(Identity("agent", "ai", ("worker",)), ["Read"])
    grant = _allow(
        {
            "scopes": ["Read"],
            "roles": ["worker"],
            "subject_roles": ["owner"],
            "subject_types": ["user"],
            "max_delegation_depth": 1,
        }
    )
    with _executor(grant) as executor:
        assert await executor.call_async("test.run", {}, Context.create(caller)) == {"actor": "agent"}
        assert executor.get_callable_descriptor("test.run", Context.create(caller)) is not None
    for rules in (
        (),
        (_allow(), grant),
        (_allow({"scopes": ["Read", "write"]}),),
        (_allow({"$or": [{"scopes": ["Read"]}]}),),
    ):
        with _executor(*rules) as executor:
            with pytest.raises(ACLDeniedError):
                await executor.call_async("test.run", {}, Context.create(caller))
            assert not (await executor.validate_async("test.run", {}, Context.create(caller))).valid
            assert executor.get_callable_descriptor("test.run", Context.create(caller)) is None
            with pytest.raises(ACLDeniedError):
                async for _ in executor.stream("test.run", {}, Context.create(caller)):
                    pytest.fail("Scoped principal streamed without a scope grant")


def test_scoped_discovery_requires_an_acl_gate_before_dispatch() -> None:
    registry = Registry()
    registry.register("test.run", _Module())
    acl = ACL([_allow({"scopes": ["read"]}), _allow()])
    steps = build_standard_strategy(registry=registry).steps
    acl_index = next(index for index, step in enumerate(steps) if step.name == "acl_check")
    acl_gate = steps.pop(acl_index)
    dispatch_index = next(index for index, step in enumerate(steps) if step.name == "execute")
    steps.insert(dispatch_index + 1, acl_gate)
    strategy = ExecutionStrategy("late-acl", steps)

    with Executor(registry, acl=acl, strategy=strategy) as executor:
        scoped = Context.create(Identity("actor", scopes=["read"]))
        assert executor.get_callable_descriptor("test.run", scoped) is None
        assert not executor.list_callable(scoped).items
        assert executor.get_callable_descriptor("test.run", Context.create(Identity("ordinary"))) is not None


@pytest.mark.asyncio
@pytest.mark.parametrize("replace_dispatch", [False, True])
async def test_scoped_strategy_requires_stock_dispatch(replace_dispatch: bool) -> None:
    class HostDispatch(BaseStep):
        async def execute(self, ctx: PipelineContext) -> StepResult:
            return StepResult("continue")

    registry = Registry()
    registry.register("test.run", _Module())
    steps = build_standard_strategy(registry=registry).steps
    dispatch = next(index for index, step in enumerate(steps) if step.name == "execute")
    steps = steps[:dispatch]
    if replace_dispatch:
        steps.append(HostDispatch("execute"))
    strategy = ExecutionStrategy("host-dispatch", steps)

    with Executor(registry, acl=ACL([_allow({"scopes": ["read"]})]), strategy=strategy) as executor:
        scoped = Context.create(Identity("actor", scopes=["read"]))
        with pytest.raises(ACLDeniedError):
            await executor.call_async("test.run", {}, scoped)
        assert not (await executor.validate_async("test.run", {}, scoped)).valid
        assert executor.get_callable_descriptor("test.run", scoped) is None


@pytest.mark.asyncio
async def test_scoped_strategy_requires_dispatch_before_returning_output() -> None:
    class FinishEarly(BaseStep):
        async def execute(self, ctx: PipelineContext) -> StepResult:
            return StepResult("skip_to", skip_to="return_result")

    with _executor(_allow({"scopes": ["read"]})) as executor:
        executor.current_strategy.insert_before("execute", FinishEarly("finish_early"))
        context = Context.create(Identity("actor", scopes=["read"]))
        with pytest.raises(ACLDeniedError):
            await executor.call_async("test.run", {}, context)


@pytest.mark.asyncio
async def test_scoped_lookup_binds_module_before_step_observers() -> None:
    class ClearLookupOutput:
        def after_step(self, step_name: str, state: PipelineState, result: StepResult) -> None:
            if step_name == "module_lookup":
                state.context.module = None

    with _executor(_allow({"scopes": ["read"]})) as executor:
        executor.current_strategy.add_step_middleware(ClearLookupOutput())
        context = Context.create(Identity("actor", scopes=["read"]))
        with pytest.raises(ACLDeniedError):
            await executor.call_async("test.run", {}, context)
        with pytest.raises(ACLDeniedError):
            async for _ in executor.stream("test.run", {}, context):
                pytest.fail("A scoped stream must retain its selected module")


@pytest.mark.parametrize("field", ["caller_id", "call_chain"])
def test_scoped_guard_requires_the_original_child_frame(field: str) -> None:
    identity = Identity("actor", scopes=["read"])
    parent = Context.create(identity).child("test.parent")
    pipeline = PipelineContext("test.run", {}, parent)
    frame = _ScopedFrame.capture(pipeline, identity, _ScopedSteps(0, 1, 2))
    pipeline.context = parent.child("test.run")
    frame.require_match(pipeline)

    other_child = Context.create(identity).child("test.other").child("test.run")
    setattr(pipeline.context, field, getattr(other_child, field))
    with pytest.raises(ACLDeniedError):
        frame.require_match(pipeline)


@pytest.mark.asyncio
async def test_scoped_nested_call_and_stream_keep_the_child_frame() -> None:
    class StreamingModule(_Module):
        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
            yield {"caller": context.caller_id, "chain": context.call_chain}

    registry = Registry()
    registry.register("test.run", StreamingModule())
    grant = ACLRule(["test.parent"], ["test.run"], "allow", conditions={"scopes": ["read"]})
    caller = Context.create(Identity("actor", scopes=["read"])).child("test.parent")

    with Executor(registry, acl=ACL([grant])) as executor:
        assert await executor.call_async("test.run", {}, caller) == {"actor": "actor"}
        chunks = [chunk async for chunk in executor.stream("test.run", {}, caller)]
        assert chunks == [{"caller": "test.parent", "chain": ["test.parent", "test.run"]}]


def test_audit_recursively_redacts_identity_attributes() -> None:
    root = Identity("root", attrs={"profile": {"Authorization": "signed", "name": "root"}}, scopes=["read"])
    caller = root.delegate(Identity("agent", attrs={"outer": [{"session_token": "signed"}]}), ["read"])
    audit = _audit_payload_extras(Context.create(caller))
    assert (audit["actor_id"], audit["subject_id"], audit["delegation_depth"]) == ("agent", "root", 1)
    nested = audit["identity"]
    assert nested["outer"] == [{"session_token": "<redacted>"}]
    assert nested["on_behalf_of"]["profile"] == {"Authorization": "<redacted>", "name": "root"}


@pytest.mark.asyncio
async def test_scoped_approvals_bind_detached_authority_before_consuming() -> None:
    class Handler:
        supports_final_scope = True
        supports_authority_scope = True
        binding: str | None = None

        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            self.binding = request.authority_key
            assert request.context.identity is not None
            assert request.context.identity.attrs["key"] == "private"
            return ApprovalResult("pending", approval_id="pending-credential")

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            pytest.fail("Approval must bind the current request")

        async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
            if approval_id != "pending-credential" or self.binding is None or request.authority_key != self.binding:
                return ApprovalResult("rejected")
            self.binding = None
            return ApprovalResult("approved")

    class Evaluate:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            assert request.identity is not None
            assert request.identity.root_subject.attrs.get("secret") in ("private", None)
            assert request.authority_key == request.identity.authority_key()
            request.identity.attrs["detached"] = True
            return PolicyVerdict("require_approval")

    registry = Registry()
    registry.register("test.run", _Module())
    handler = Handler()
    actor = Identity("actor", attrs={"key": "private"})
    root = Identity("root", attrs={"secret": "private"}, scopes=["read", "write"])
    original = root.delegate(actor, ["read", "write"])
    with Executor(
        registry,
        acl=ACL([_allow({"scopes": ["read"]})]),
        approval_handler=handler,
        policy_evaluator=Evaluate(),
    ) as executor:
        with pytest.raises(ApprovalPendingError):
            await executor.call_async("test.run", {}, Context.create(original))
        for changed in (root.delegate(actor, ["read"]), Identity("other", scopes=["read"]).delegate(actor, ["read"])):
            with pytest.raises(ApprovalDeniedError):
                await executor.call_async(
                    "test.run", {"_approval_token": "pending-credential"}, Context.create(changed)
                )
            assert handler.binding == original.authority_key()
        assert await executor.call_async(
            "test.run", {"_approval_token": "pending-credential"}, Context.create(original)
        ) == {"actor": "actor"}
        assert handler.binding is None
