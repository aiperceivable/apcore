"""Observable contracts for scoped and delegated callers."""

from __future__ import annotations

import pytest

from apcore.acl import ACL, ACLRule
from apcore.approval import ApprovalRequest, ApprovalResult
from apcore.context import Context, Identity
from apcore.errors import ACLDeniedError, ApprovalDeniedError, ApprovalPendingError, InvalidInputError
from apcore.executor import Executor
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
