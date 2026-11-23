"""Tests for the execution-time governance policy (apcore#76 RFC pilot).

Covers PolicyRule validation, ExecutionPolicy.from_dict strict parsing,
resolve() precedence/specificity semantics, and the approval-gate
integration: policy overrides, gate_destructive, strict fail-closed,
and the fail-loud warnings for ungated governance gaps.
"""

from __future__ import annotations

import logging
from typing import Any

import pytest
from pydantic import BaseModel, ConfigDict

from apcore import ExecutionPolicy, PolicyDecision, PolicyRule
from apcore.approval import AlwaysDenyHandler, ApprovalRequest, ApprovalResult, AutoApproveHandler
from apcore.context import Context
from apcore.errors import ApprovalDeniedError
from apcore.executor import Executor
from apcore.module import ModuleAnnotations
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# Test module implementations
# ---------------------------------------------------------------------------


class PermissiveInput(BaseModel):
    model_config = ConfigDict(extra="allow")


class PermissiveOutput(BaseModel):
    model_config = ConfigDict(extra="allow")


class PlainModule:
    """Module with no governance annotations."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations()

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


class ApprovalRequiredModule:
    """Module declaring requires_approval=True."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations(requires_approval=True)
    description = "Gated operation"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


class DestructiveNoApprovalModule:
    """Module declaring destructive=True but requires_approval=False — the #76 footgun."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations(destructive=True)
    description = "Destructive but ungated"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


class RecordingHandler:
    """Approval handler that records requests and returns a fixed decision."""

    def __init__(self, status: str = "approved") -> None:
        self.requests: list[ApprovalRequest] = []
        self._status = status

    async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
        self.requests.append(request)
        return ApprovalResult(status=self._status)  # type: ignore[arg-type]

    async def check_approval(self, approval_id: str) -> ApprovalResult:
        return ApprovalResult(status="rejected", reason="not supported")


@pytest.fixture
def registry() -> Registry:
    reg = Registry()
    reg.register("orders.list_orders", PlainModule())
    reg.register("orders.delete_order", DestructiveNoApprovalModule())
    reg.register("admin.reset", ApprovalRequiredModule())
    return reg


# ---------------------------------------------------------------------------
# PolicyRule validation
# ---------------------------------------------------------------------------


class TestPolicyRule:
    def test_valid_rule(self) -> None:
        rule = PolicyRule("orders.*", requires_approval=True, reason="ops sign-off")
        assert rule.pattern == "orders.*"
        assert rule.requires_approval is True
        assert rule.destructive is None

    def test_empty_pattern_rejected(self) -> None:
        with pytest.raises(ValueError, match="pattern"):
            PolicyRule("")

    def test_non_string_pattern_rejected(self) -> None:
        with pytest.raises(ValueError, match="pattern"):
            PolicyRule(None)  # type: ignore[arg-type]

    def test_non_bool_override_rejected(self) -> None:
        with pytest.raises(ValueError, match="requires_approval"):
            PolicyRule("a.b", requires_approval="yes")  # type: ignore[arg-type]
        with pytest.raises(ValueError, match="destructive"):
            PolicyRule("a.b", destructive=1)  # type: ignore[arg-type]

    def test_non_string_reason_rejected(self) -> None:
        with pytest.raises(ValueError, match="reason"):
            PolicyRule("a.b", reason=42)  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# ExecutionPolicy.from_dict — strict parsing (fail loud on typos)
# ---------------------------------------------------------------------------


class TestFromDict:
    def test_full_document(self) -> None:
        policy = ExecutionPolicy.from_dict(
            {
                "gate_destructive": True,
                "strict": True,
                "rules": [
                    {"pattern": "orders.delete_*", "requires_approval": True, "reason": "sign-off"},
                    {"pattern": "reports.*", "destructive": False},
                ],
            }
        )
        assert policy.gate_destructive is True
        assert policy.strict is True
        assert len(policy.rules) == 2
        assert policy.rules[0].pattern == "orders.delete_*"

    def test_empty_document(self) -> None:
        policy = ExecutionPolicy.from_dict({})
        assert policy.rules == ()
        assert policy.gate_destructive is False
        assert policy.strict is False

    def test_non_mapping_rejected(self) -> None:
        with pytest.raises(ValueError, match="mapping"):
            ExecutionPolicy.from_dict(["not", "a", "dict"])

    def test_unknown_policy_key_rejected(self) -> None:
        with pytest.raises(ValueError, match="gate_destructiv"):
            ExecutionPolicy.from_dict({"gate_destructiv": True})

    def test_rules_not_a_list_rejected(self) -> None:
        with pytest.raises(ValueError, match="list"):
            ExecutionPolicy.from_dict({"rules": {"pattern": "a.b"}})

    def test_rule_not_a_mapping_rejected(self) -> None:
        with pytest.raises(ValueError, match="rule #0"):
            ExecutionPolicy.from_dict({"rules": ["a.b"]})

    def test_rule_unknown_key_rejected(self) -> None:
        with pytest.raises(ValueError, match="require_approval"):
            ExecutionPolicy.from_dict({"rules": [{"pattern": "a.b", "require_approval": True}]})

    def test_rule_missing_pattern_rejected(self) -> None:
        with pytest.raises(ValueError, match="pattern"):
            ExecutionPolicy.from_dict({"rules": [{"requires_approval": True}]})

    def test_non_policyrule_in_constructor_rejected(self) -> None:
        with pytest.raises(ValueError, match="PolicyRule"):
            ExecutionPolicy([{"pattern": "a.b"}])  # type: ignore[list-item]


# ---------------------------------------------------------------------------
# ExecutionPolicy.resolve — precedence and specificity
# ---------------------------------------------------------------------------


class TestResolve:
    def test_no_rules_passes_annotations_through(self) -> None:
        policy = ExecutionPolicy()
        decision = policy.resolve("a.b", ModuleAnnotations(requires_approval=True, destructive=True))
        assert decision == PolicyDecision(
            module_id="a.b",
            requires_approval=True,
            destructive=True,
            needs_approval=True,
            rule=None,
            overridden=False,
        )

    def test_dict_annotations_supported(self) -> None:
        policy = ExecutionPolicy()
        decision = policy.resolve("a.b", {"requires_approval": True})
        assert decision.requires_approval is True
        assert decision.needs_approval is True

    def test_none_annotations_supported(self) -> None:
        decision = ExecutionPolicy().resolve("a.b", None)
        assert decision.needs_approval is False

    def test_rule_forces_approval(self) -> None:
        policy = ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True)])
        decision = policy.resolve("orders.delete_order", ModuleAnnotations())
        assert decision.requires_approval is True
        assert decision.needs_approval is True
        assert decision.overridden is True
        assert decision.rule is not None and decision.rule.pattern == "orders.delete_*"

    def test_rule_exempts_approval(self) -> None:
        policy = ExecutionPolicy([PolicyRule("batch.*", requires_approval=False)])
        decision = policy.resolve("batch.cleanup", ModuleAnnotations(requires_approval=True))
        assert decision.requires_approval is False
        assert decision.needs_approval is False
        assert decision.overridden is True

    def test_none_fields_do_not_override(self) -> None:
        policy = ExecutionPolicy([PolicyRule("a.*", destructive=True)])
        decision = policy.resolve("a.b", ModuleAnnotations(requires_approval=True))
        assert decision.requires_approval is True  # untouched
        assert decision.destructive is True  # overridden
        assert decision.overridden is True

    def test_unmatched_rule_is_ignored(self) -> None:
        policy = ExecutionPolicy([PolicyRule("other.*", requires_approval=True)])
        decision = policy.resolve("a.b", ModuleAnnotations())
        assert decision.rule is None
        assert decision.needs_approval is False
        assert decision.overridden is False

    def test_most_specific_rule_wins(self) -> None:
        policy = ExecutionPolicy(
            [
                PolicyRule("*", requires_approval=True),
                PolicyRule("orders.list_orders", requires_approval=False),
            ]
        )
        decision = policy.resolve("orders.list_orders", ModuleAnnotations())
        assert decision.needs_approval is False
        decision = policy.resolve("orders.delete_order", ModuleAnnotations())
        assert decision.needs_approval is True

    def test_specificity_tie_more_restrictive_wins(self) -> None:
        policy = ExecutionPolicy(
            [
                PolicyRule("orders.*", requires_approval=False),
                PolicyRule("orders.*", requires_approval=True),
            ]
        )
        decision = policy.resolve("orders.delete_order", ModuleAnnotations())
        assert decision.needs_approval is True

    def test_gate_destructive_requires_approval(self) -> None:
        policy = ExecutionPolicy(gate_destructive=True)
        decision = policy.resolve("orders.delete_order", ModuleAnnotations(destructive=True))
        assert decision.requires_approval is False
        assert decision.needs_approval is True

    def test_gate_destructive_with_override(self) -> None:
        policy = ExecutionPolicy(
            [PolicyRule("orders.delete_*", destructive=True)],
            gate_destructive=True,
        )
        decision = policy.resolve("orders.delete_order", ModuleAnnotations())
        assert decision.destructive is True
        assert decision.needs_approval is True


# ---------------------------------------------------------------------------
# Approval gate integration (Executor end-to-end)
# ---------------------------------------------------------------------------


class TestApprovalGateWithPolicy:
    def test_policy_forces_approval_handler_consulted(self, registry: Registry) -> None:
        handler = RecordingHandler(status="approved")
        policy = ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True, reason="sign-off")])
        executor = Executor(registry=registry, approval_handler=handler, policy=policy)
        result = executor.call("orders.delete_order")
        assert result["status"] == "executed"
        assert len(handler.requests) == 1
        assert handler.requests[0].module_id == "orders.delete_order"

    def test_policy_forces_approval_deny_blocks(self, registry: Registry) -> None:
        policy = ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True)])
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler(), policy=policy)
        with pytest.raises(ApprovalDeniedError):
            executor.call("orders.delete_order")

    def test_policy_exempts_module_handler_not_consulted(self, registry: Registry) -> None:
        handler = RecordingHandler(status="rejected")
        policy = ExecutionPolicy([PolicyRule("admin.reset", requires_approval=False)])
        executor = Executor(registry=registry, approval_handler=handler, policy=policy)
        result = executor.call("admin.reset")
        assert result["status"] == "executed"
        assert handler.requests == []

    def test_strict_policy_fails_closed_without_handler(self, registry: Registry) -> None:
        policy = ExecutionPolicy(strict=True)
        executor = Executor(registry=registry, policy=policy)
        with pytest.raises(ApprovalDeniedError, match="fails closed"):
            executor.call("admin.reset")

    def test_strict_policy_without_gated_modules_executes(self, registry: Registry) -> None:
        policy = ExecutionPolicy(strict=True)
        executor = Executor(registry=registry, policy=policy)
        result = executor.call("orders.list_orders")
        assert result["status"] == "executed"

    def test_no_handler_warns_once_and_skips(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        """Fail-loud default: requires_approval without a handler skips per spec but warns once."""
        executor = Executor(registry=registry)
        with caplog.at_level(logging.WARNING, logger="apcore.builtin_steps"):
            result = executor.call("admin.reset")
            assert result["status"] == "executed"
            result = executor.call("admin.reset")
            assert result["status"] == "executed"

        warnings = [r for r in caplog.records if "no ApprovalHandler is configured" in r.message]
        assert len(warnings) == 1
        assert "admin.reset" in warnings[0].getMessage()

    def test_destructive_ungated_warns_once(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        executor = Executor(registry=registry)
        with caplog.at_level(logging.WARNING, logger="apcore.builtin_steps"):
            executor.call("orders.delete_order")
            executor.call("orders.delete_order")

        warnings = [r for r in caplog.records if "destructive=True" in r.message]
        assert len(warnings) == 1
        assert "orders.delete_order" in warnings[0].getMessage()

    def test_non_destructive_module_no_warning(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        executor = Executor(registry=registry)
        with caplog.at_level(logging.WARNING, logger="apcore.builtin_steps"):
            executor.call("orders.list_orders")
        assert [r for r in caplog.records if r.levelno >= logging.WARNING] == []

    def test_gate_destructive_covers_destructive_module(self, registry: Registry) -> None:
        handler = RecordingHandler(status="approved")
        policy = ExecutionPolicy(gate_destructive=True)
        executor = Executor(registry=registry, approval_handler=handler, policy=policy)
        result = executor.call("orders.delete_order")
        assert result["status"] == "executed"
        assert len(handler.requests) == 1

    def test_policy_override_audit_logged(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        policy = ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True, reason="sign-off")])
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler(), policy=policy)
        with caplog.at_level(logging.INFO, logger="apcore.builtin_steps"):
            executor.call("orders.delete_order")

        audits = [r for r in caplog.records if "Policy override" in r.message]
        assert len(audits) == 1
        message = audits[0].getMessage()
        assert "orders.delete_order" in message
        assert "sign-off" in message

    def test_no_audit_when_policy_does_not_override(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        policy = ExecutionPolicy([PolicyRule("other.*", requires_approval=True)])
        executor = Executor(registry=registry, policy=policy)
        with caplog.at_level(logging.INFO, logger="apcore.builtin_steps"):
            executor.call("orders.list_orders")
        assert [r for r in caplog.records if "Policy override" in r.message] == []

    def test_set_policy_at_runtime(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        assert executor.call("orders.delete_order")["status"] == "executed"

        executor.set_policy(ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True)]))
        with pytest.raises(ApprovalDeniedError):
            executor.call("orders.delete_order")

        executor.set_policy(None)
        assert executor.call("orders.delete_order")["status"] == "executed"

    def test_policy_gated_request_upholds_annotations_contract(self, registry: Registry) -> None:
        """ApprovalRequest.annotations must keep the 'requires_approval is guaranteed
        true' contract (PROTOCOL_SPEC §7) even when the gate was policy-forced on a
        module that declares requires_approval=False."""
        handler = RecordingHandler(status="approved")
        policy = ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True)])
        executor = Executor(registry=registry, approval_handler=handler, policy=policy)
        executor.call("orders.delete_order")

        assert len(handler.requests) == 1
        annotations = handler.requests[0].annotations
        assert annotations.requires_approval is True  # effective, not the raw False
        assert annotations.destructive is True  # module's own value untouched

    def test_gate_destructive_request_upholds_annotations_contract(self, registry: Registry) -> None:
        """gate_destructive-driven approval also presents effective annotations."""
        handler = RecordingHandler(status="approved")
        executor = Executor(registry=registry, approval_handler=handler, policy=ExecutionPolicy(gate_destructive=True))
        executor.call("orders.delete_order")

        assert len(handler.requests) == 1
        annotations = handler.requests[0].annotations
        assert annotations.requires_approval is True
        assert annotations.destructive is True

    def test_policy_destructive_override_visible_to_handler(self, registry: Registry) -> None:
        """A policy that marks a module destructive shows that to the handler."""
        handler = RecordingHandler(status="approved")
        policy = ExecutionPolicy(
            [PolicyRule("admin.reset", destructive=True)],
        )
        executor = Executor(registry=registry, approval_handler=handler, policy=policy)
        executor.call("admin.reset")

        assert len(handler.requests) == 1
        annotations = handler.requests[0].annotations
        assert annotations.requires_approval is True
        assert annotations.destructive is True  # policy-effective, module declared False

    def test_from_registry_accepts_policy(self, registry: Registry) -> None:
        policy = ExecutionPolicy([PolicyRule("admin.*", requires_approval=False)])
        handler = RecordingHandler(status="rejected")
        executor = Executor.from_registry(registry, approval_handler=handler, policy=policy)
        assert executor.call("admin.reset")["status"] == "executed"
        assert handler.requests == []


# ---------------------------------------------------------------------------
# Governance events on the event bus (apcore#77)
# ---------------------------------------------------------------------------


class CollectingEmitter:
    """Duck-typed EventEmitter that records events synchronously."""

    def __init__(self) -> None:
        self.events: list[Any] = []

    def emit(self, event: Any) -> None:
        self.events.append(event)

    def of_type(self, event_type: str) -> list[Any]:
        return [e for e in self.events if e.event_type == event_type]


class TestGovernanceEvents:
    def test_approved_decision_event(self, registry: Registry) -> None:
        emitter = CollectingEmitter()
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler(), event_emitter=emitter)
        executor.call("admin.reset")

        events = emitter.of_type("apcore.approval.decision")
        assert len(events) == 1
        event = events[0]
        assert event.module_id == "admin.reset"
        assert event.severity == "info"
        assert event.data["status"] == "approved"
        assert event.data["approved_by"] == "auto"
        assert event.data["trace_id"]

    def test_rejected_decision_event_severity_warn(self, registry: Registry) -> None:
        emitter = CollectingEmitter()
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler(), event_emitter=emitter)
        with pytest.raises(ApprovalDeniedError):
            executor.call("admin.reset")

        events = emitter.of_type("apcore.approval.decision")
        assert len(events) == 1
        assert events[0].severity == "warn"
        assert events[0].data["status"] == "rejected"

    def test_strict_fail_closed_emits_decision_event(self, registry: Registry) -> None:
        emitter = CollectingEmitter()
        executor = Executor(registry=registry, policy=ExecutionPolicy(strict=True), event_emitter=emitter)
        with pytest.raises(ApprovalDeniedError):
            executor.call("admin.reset")

        events = emitter.of_type("apcore.approval.decision")
        assert len(events) == 1
        assert events[0].severity == "warn"
        assert events[0].data["status"] == "rejected"

    def test_policy_override_event(self, registry: Registry) -> None:
        emitter = CollectingEmitter()
        policy = ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True, reason="sign-off")])
        executor = Executor(
            registry=registry, approval_handler=AutoApproveHandler(), policy=policy, event_emitter=emitter
        )
        executor.call("orders.delete_order")

        overrides = emitter.of_type("apcore.policy.override")
        assert len(overrides) == 1
        data = overrides[0].data
        assert data["module_id"] == "orders.delete_order"
        assert data["pattern"] == "orders.delete_*"
        assert data["requires_approval"] is True
        assert data["needs_approval"] is True
        assert data["reason"] == "sign-off"
        # The final adjudication is a separate event.
        assert len(emitter.of_type("apcore.approval.decision")) == 1

    def test_no_events_when_gate_not_involved(self, registry: Registry) -> None:
        emitter = CollectingEmitter()
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler(), event_emitter=emitter)
        executor.call("orders.list_orders")
        assert emitter.of_type("apcore.approval.decision") == []
        assert emitter.of_type("apcore.policy.override") == []

    def test_no_decision_event_when_gate_skipped_without_handler(self, registry: Registry) -> None:
        """Non-strict skip (spec §7.4) emits no decision event — parity with the
        'no audit log when gate skipped' contract."""
        emitter = CollectingEmitter()
        executor = Executor(registry=registry, event_emitter=emitter)
        executor.call("admin.reset")
        assert emitter.of_type("apcore.approval.decision") == []

    def test_acl_denied_event(self, registry: Registry) -> None:
        from apcore.acl import ACL
        from apcore.errors import ACLDeniedError

        emitter = CollectingEmitter()
        executor = Executor(registry=registry, acl=ACL(rules=[], default_effect="deny"), event_emitter=emitter)
        with pytest.raises(ACLDeniedError):
            executor.call("orders.list_orders")

        events = emitter.of_type("apcore.acl.denied")
        assert len(events) == 1
        assert events[0].severity == "warn"
        assert events[0].data["module_id"] == "orders.list_orders"
        # caller_id mirrors what the ACL used to decide — None at a top-level
        # entry (no upstream caller in the call chain); the field carries the
        # calling module on inter-module calls, which is the ACL's real subject.
        assert "caller_id" in events[0].data
        assert events[0].data["trace_id"]

    def test_acl_denied_event_not_emitted_in_preflight(self, registry: Registry) -> None:
        """validate() runs the ACL step in dry_run — it must NOT emit a denial event."""
        from apcore.acl import ACL

        emitter = CollectingEmitter()
        executor = Executor(registry=registry, acl=ACL(rules=[], default_effect="deny"), event_emitter=emitter)
        executor.validate("orders.list_orders", {})
        assert emitter.of_type("apcore.acl.denied") == []


# ---------------------------------------------------------------------------
# validate() preflight reflects policy
# ---------------------------------------------------------------------------


class TestValidateWithPolicy:
    def test_validate_reports_policy_forced_approval(self, registry: Registry) -> None:
        policy = ExecutionPolicy([PolicyRule("orders.delete_*", requires_approval=True)])
        executor = Executor(registry=registry, policy=policy)
        preflight = executor.validate("orders.delete_order", {})
        assert preflight.requires_approval is True

    def test_validate_reports_gate_destructive(self, registry: Registry) -> None:
        executor = Executor(registry=registry, policy=ExecutionPolicy(gate_destructive=True))
        preflight = executor.validate("orders.delete_order", {})
        assert preflight.requires_approval is True

    def test_validate_reports_policy_exemption(self, registry: Registry) -> None:
        policy = ExecutionPolicy([PolicyRule("admin.reset", requires_approval=False)])
        executor = Executor(registry=registry, policy=policy)
        preflight = executor.validate("admin.reset", {})
        assert preflight.requires_approval is False

    def test_validate_without_policy_unchanged(self, registry: Registry) -> None:
        executor = Executor(registry=registry)
        assert executor.validate("admin.reset", {}).requires_approval is True
        assert executor.validate("orders.list_orders", {}).requires_approval is False


class TestGovernanceFlagsAreNotCoerced:
    """`gate_destructive` / `strict` must be real booleans in a governance file.

    ``from_dict`` used to run the raw value through ``bool()``, so
    ``gate_destructive: "false"`` became **True** (a non-empty string is truthy)
    and ``gate_destructive: []`` became False — while apcore-rust's serde-typed
    ``bool`` field and apcore-typescript's ``_requireBoolean`` both reject either
    outright. The same governance document therefore produced a different
    effective policy per runtime.

    ``gate_destructive`` is the switch that turns a ``destructive`` annotation
    into an approval gate (PROTOCOL_SPEC §7.9.2), so a silent coercion is a
    governance decision made by accident. §7.9.4 already requires ``from_dict``
    to fail loud on unknown keys; this is the same discipline applied to values.
    """

    @pytest.mark.parametrize("key", ["gate_destructive", "strict"])
    @pytest.mark.parametrize(
        "value",
        ["false", "true", "", [], {}, 0, 1, 1.0, None.__class__],
        ids=["str_false", "str_true", "empty_str", "list", "dict", "int_0", "int_1", "float", "type"],
    )
    def test_non_boolean_flag_is_rejected(self, key: str, value: Any) -> None:
        with pytest.raises(ValueError, match=key):
            ExecutionPolicy.from_dict({key: value})

    def test_the_truthy_string_trap_is_closed(self) -> None:
        # The headline case: `"false"` silently enabled the gate.
        with pytest.raises(ValueError, match="gate_destructive"):
            ExecutionPolicy.from_dict({"gate_destructive": "false"})

    def test_the_falsy_container_trap_is_closed(self) -> None:
        with pytest.raises(ValueError, match="gate_destructive"):
            ExecutionPolicy.from_dict({"gate_destructive": []})

    def test_error_says_a_boolean_was_required(self) -> None:
        with pytest.raises(ValueError) as exc_info:
            ExecutionPolicy.from_dict({"strict": "yes"})
        message = str(exc_info.value)
        assert "boolean" in message
        assert "str" in message

    @pytest.mark.parametrize("key", ["gate_destructive", "strict"])
    def test_real_booleans_still_pass(self, key: str) -> None:
        assert getattr(ExecutionPolicy.from_dict({key: True}), key) is True
        assert getattr(ExecutionPolicy.from_dict({key: False}), key) is False

    @pytest.mark.parametrize("key", ["gate_destructive", "strict"])
    def test_absent_and_null_mean_the_documented_default(self, key: str) -> None:
        # Parity with apcore-typescript `_requireBoolean`: undefined/null are
        # "unset" and take the documented default of False, not an error.
        assert getattr(ExecutionPolicy.from_dict({}), key) is False
        assert getattr(ExecutionPolicy.from_dict({key: None}), key) is False


# ---------------------------------------------------------------------------
# §7.9.6 — call-site inputs to policy resolution (apcore#102)
# ---------------------------------------------------------------------------


class _CallSiteRecordingPolicy(ExecutionPolicy):
    """Host-supplied policy that records the call site it was handed."""

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self.seen: list[tuple[str, dict[str, Any] | None, Context | None]] = []

    def resolve(
        self,
        module_id: str,
        annotations: Any = None,
        *,
        arguments: dict[str, Any] | None = None,
        context: Context | None = None,
    ) -> PolicyDecision:
        self.seen.append((module_id, arguments, context))
        return super().resolve(module_id, annotations, arguments=arguments, context=context)


class TestPolicyCallSite:
    """PROTOCOL_SPEC §7.9.6: resolution receives the call site but rules ignore it."""

    def test_resolve_accepts_the_call_site(self) -> None:
        decision = ExecutionPolicy().resolve(
            "a.b",
            ModuleAnnotations(requires_approval=True),
            arguments={"amount": 1000},
            context=Context.create(),
        )
        assert decision.needs_approval is True

    def test_existing_positional_callers_keep_working(self) -> None:
        """§7.9.6 rule 5 in Python idiom: the new inputs are keyword-only and optional."""
        assert ExecutionPolicy().resolve("a.b", ModuleAnnotations()).needs_approval is False
        assert ExecutionPolicy().resolve("a.b").needs_approval is False

    @pytest.mark.parametrize(
        "policy",
        [
            ExecutionPolicy(),
            ExecutionPolicy([PolicyRule("orders.*", requires_approval=True)]),
            ExecutionPolicy([PolicyRule("orders.delete_*", destructive=True)], gate_destructive=True),
            ExecutionPolicy([PolicyRule("*", requires_approval=False)]),
        ],
        ids=["empty", "forces", "gate_destructive", "exempts"],
    )
    @pytest.mark.parametrize(
        "annotations",
        [None, ModuleAnnotations(), ModuleAnnotations(requires_approval=True), ModuleAnnotations(destructive=True)],
        ids=["none", "plain", "requires_approval", "destructive"],
    )
    def test_built_in_rules_do_not_consult_the_call_site(self, policy: ExecutionPolicy, annotations: Any) -> None:
        """§7.9.6 rule 2 + the compatibility guarantee: verdicts are bit-for-bit unchanged.

        A rule set's verdict MUST stay a function of the module ID and the
        annotations alone, so it remains reproducible from the policy document.
        """
        baseline = policy.resolve("orders.delete_order", annotations)
        with_call_site = policy.resolve(
            "orders.delete_order",
            annotations,
            arguments={"order_id": "x", "force": True},
            context=Context.create(identity=None),
        )
        assert with_call_site == baseline

    def test_malformed_arguments_do_not_disturb_resolution(self) -> None:
        """§7.9.6 rule 4: the arguments have NOT been schema-validated at Step 5."""
        policy = ExecutionPolicy([PolicyRule("a.b", requires_approval=True)])
        baseline = policy.resolve("a.b", ModuleAnnotations())
        for bogus in ({"amount": object()}, {}, {"": None}):
            assert policy.resolve("a.b", ModuleAnnotations(), arguments=bogus) == baseline

    def test_the_approval_gate_passes_the_call_site(self, registry: Registry) -> None:
        policy = _CallSiteRecordingPolicy([PolicyRule("admin.reset", requires_approval=True)])
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler(), policy=policy)
        context = Context.create()
        executor.call("admin.reset", {"scope": "all"}, context=context)

        gate_calls = [c for c in policy.seen if c[0] == "admin.reset"]
        assert gate_calls, "the approval gate must consult the policy"
        _, arguments, seen_context = gate_calls[-1]
        assert arguments == {"scope": "all"}
        # The pipeline hands the step a derived Context (call_chain extended),
        # so identity is not the assertion — provenance is.
        assert seen_context is not None
        assert seen_context.trace_id == context.trace_id
        assert seen_context.call_chain == ["admin.reset"]

    def test_the_gate_never_shows_the_policy_a_protocol_level_key(self, registry: Registry) -> None:
        """§7.9.6 rule 5 (normative at spec v1.25.0): strip ``_approval_token`` first.

        §7.4's existing "remove before passing to subsequent steps" does not
        reach this: policy resolution happens *inside* Step 5, ahead of any
        subsequent step, so an implementation can satisfy §7.4 literally and
        still hand the token to the policy — putting it into the audit trail and
        the ``apcore.policy.override`` payload.
        """
        policy = _CallSiteRecordingPolicy([PolicyRule("admin.reset", requires_approval=True)])
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler(), policy=policy)
        executor.call("admin.reset", {"scope": "all", "_approval_token": "tok"})
        assert all("_approval_token" not in (args or {}) for _, args, _ in policy.seen)

    def test_preflight_passes_the_call_site_too(self, registry: Registry) -> None:
        """§7.9.5 reports the verdict the gate will enforce, from the same inputs."""
        policy = _CallSiteRecordingPolicy([PolicyRule("admin.reset", requires_approval=True)])
        executor = Executor(registry=registry, policy=policy)
        executor.validate("admin.reset", {"scope": "all"})
        assert any(args == {"scope": "all"} for _, args, _ in policy.seen)

    def test_a_subclass_can_decide_on_arguments(self, registry: Registry) -> None:
        """The point of §7.9.6: gate *some* calls to a module rather than all of them.

        Note this is NOT a specified extension point — §7.9.6 rule 3(b) was
        withdrawn in spec v1.25.0 because ``ExecutionPolicy`` is a concrete type
        in all three SDKs and ``set_policy`` takes that concrete type, so no host
        can supply an implementation. Subclassing works in Python; the test
        exists to show the call site genuinely arrives, not to bless the shape.
        """

        class AmountPolicy(ExecutionPolicy):
            def resolve(
                self,
                module_id: str,
                annotations: Any = None,
                *,
                arguments: dict[str, Any] | None = None,
                context: Context | None = None,
            ) -> PolicyDecision:
                base = super().resolve(module_id, annotations, arguments=arguments, context=context)
                # Arguments are NOT schema-validated here — treat them defensively.
                amount = (arguments or {}).get("amount")
                if isinstance(amount, (int, float)) and not isinstance(amount, bool) and amount > 100:
                    return PolicyDecision(
                        module_id=base.module_id,
                        requires_approval=True,
                        destructive=base.destructive,
                        needs_approval=True,
                        rule=base.rule,
                        overridden=True,
                    )
                return base

        handler = RecordingHandler(status="approved")
        executor = Executor(registry=registry, approval_handler=handler, policy=AmountPolicy())
        executor.call("orders.list_orders", {"amount": 5})
        assert handler.requests == [], "a small call is not gated"
        executor.call("orders.list_orders", {"amount": 5000})
        assert len(handler.requests) == 1, "a large call is gated"
