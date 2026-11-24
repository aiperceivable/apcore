"""Argument-scoped approval — PROTOCOL_SPEC v1.28.0 §6.1.6-§6.1.8, §6.8.1, §6.9 (#108).

The gap this closes: every decision point that could read a call's arguments was
unable to escalate it to a human, and the one point that decides whether to ask
a human was forbidden from consulting them. The ACL could **refuse** on
arguments and an ``ApprovalHandler`` could **wave through** on arguments;
nothing could **ask**, and a refusal is not a question.

Five sections, one per normative surface:

* ``TestApprovalRuleField`` — §6.1.6, the orthogonal ``approval`` field.
* ``TestArgumentsCondition`` — §6.1.7, the built-in structure-only predicates.
* ``TestGovernanceProjection`` — §6.1.8, keys and types, never a value.
* ``TestAccessDecision`` — §6.8.1, the structured result and the fail-closed
  legacy boolean.
* ``TestGovernancePrecedence`` — §6.9 / §7.4 / §7.9.5, the union at Step 5 and
  the privilege-escalation guard in row 4.
"""

from __future__ import annotations

import tempfile
from pathlib import Path
from typing import Any

import pytest
import yaml
from pydantic import BaseModel, ConfigDict

from apcore.acl import ACL, AccessDecision, ACLRule, AuditEntry
from apcore.approval import ApprovalRequest, ApprovalResult, CallbackApprovalHandler
from apcore.builtin_steps import BuiltinACLCheck, BuiltinModuleLookup, build_standard_strategy
from apcore.context import Context, GovernanceProjection, Identity
from apcore.errors import ACLDeniedError, ACLRuleError, ApprovalDeniedError
from apcore.executor import Executor
from apcore.module import ModuleAnnotations
from apcore.pipeline import BaseStep, PipelineContext, StepResult
from apcore.policy import ExecutionPolicy, PolicyRule
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


class Permissive(BaseModel):
    model_config = ConfigDict(extra="allow")


class GitPush:
    """The driving example: ``git push`` is fine, ``git push --force`` is not."""

    input_schema = Permissive
    output_schema = Permissive
    annotations = ModuleAnnotations(requires_approval=False, destructive=False)
    description = "Push commits to a remote"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"pushed": True}


def _registry() -> Registry:
    reg = Registry()
    reg.register("cli.git_push", GitPush())
    return reg


def _force_rule(**overrides: Any) -> ACLRule:
    """``allow`` + ``approval: required`` when the call carries ``force``."""
    fields: dict[str, Any] = {
        "callers": ["*"],
        "targets": ["cli.git_push"],
        "effect": "allow",
        "approval": "required",
        "description": "a forced push is put to a human",
        "conditions": {"arguments": {"has_key": ["force"]}},
    }
    fields.update(overrides)
    return ACLRule(**fields)


def _acl(*rules: ACLRule, default_effect: str = "allow", audit: list[AuditEntry] | None = None) -> ACL:
    return ACL(
        rules=list(rules),
        default_effect=default_effect,
        audit_logger=(audit.append if audit is not None else None),
    )


def _ctx(projection_of: dict[str, Any] | None = None, **kwargs: Any) -> Context:
    ctx = Context(trace_id="t-1", identity=Identity(id="u-1", type="user"), **kwargs)
    if projection_of is not None:
        ctx.governance_projection = GovernanceProjection.of(projection_of)
    return ctx


def _load_yaml_acl(rule: dict[str, Any], default_effect: str = "deny") -> ACL:
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump({"default_effect": default_effect, "rules": [rule]}))
        return ACL.load(str(path))


# ---------------------------------------------------------------------------
# §6.1.6 — the `approval` rule field
# ---------------------------------------------------------------------------


class TestApprovalRuleField:
    """Authorization and approval are two results, not one (§6.1.6)."""

    def test_absent_approval_means_not_required(self) -> None:
        """Every rule written before v1.28.0 keeps its meaning exactly (rule 1)."""
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        assert rule.approval == "not_required"
        assert _acl(rule).check("api.a", "executor.b", _ctx()) is True

    def test_loader_accepts_approval_required_on_an_allow_rule(self) -> None:
        acl = _load_yaml_acl(
            {
                "callers": ["*"],
                "targets": ["cli.git_push"],
                "effect": "allow",
                "approval": "required",
            }
        )
        assert acl.rules[0].approval == "required"

    def test_loader_accepts_explicit_not_required(self) -> None:
        acl = _load_yaml_acl({"callers": ["*"], "targets": ["*"], "effect": "deny", "approval": "not_required"})
        assert acl.rules[0].approval == "not_required"

    def test_approval_is_in_the_closed_rule_key_set(self) -> None:
        """§6.1.6 rule 3: adding the field was only safe once §6.1.5 closed the set."""
        from apcore.acl import _RULE_KEYS

        assert "approval" in _RULE_KEYS

    def test_approval_required_on_a_deny_rule_is_rejected_at_load(self) -> None:
        """§6.1.6 rule 2. The combination has no meaning; half-applying it is the

        failure mode §6.1.5 was written to end.
        """
        with pytest.raises(ACLRuleError) as excinfo:
            _load_yaml_acl({"callers": ["agent.*"], "targets": ["orders.*"], "effect": "deny", "approval": "required"})
        message = str(excinfo.value)
        assert "Rule 0" in message, message
        assert "approval" in message and "deny" in message, message

    def test_approval_required_on_a_deny_rule_is_rejected_on_direct_construction(self) -> None:
        """``ACL(rules=[...])`` never reaches the loader's parser — the same door

        §6.1.1 case 5 and §6.1.4.1 exist for.
        """
        with pytest.raises(ACLRuleError, match="deny"):
            ACLRule(callers=["*"], targets=["*"], effect="deny", approval="required")

    def test_approval_required_on_a_deny_rule_is_rejected_by_add_rule(self) -> None:
        acl = _acl()
        with pytest.raises(ACLRuleError, match="deny"):
            acl.add_rule(callers=["*"], targets=["*"], effect="deny", approval="required")
        assert acl.rules == ()

    def test_an_unknown_approval_value_is_rejected(self) -> None:
        with pytest.raises(ACLRuleError, match="invalid approval"):
            _load_yaml_acl({"callers": ["*"], "targets": ["*"], "effect": "allow", "approval": "maybe"})
        with pytest.raises(ACLRuleError, match="invalid approval"):
            ACLRule(callers=["*"], targets=["*"], effect="allow", approval=True)


# ---------------------------------------------------------------------------
# §6.1.7 — the `arguments` condition
# ---------------------------------------------------------------------------


class TestArgumentsCondition:
    """One built-in, structure-only condition key (§6.1.7)."""

    @pytest.mark.parametrize(
        ("predicate", "arguments", "expected"),
        [
            # has_key — ANY of the named keys is present
            ({"has_key": ["force"]}, {"force": True}, True),
            ({"has_key": ["force"]}, {"remote": "origin"}, False),
            ({"has_key": ["force", "mirror"]}, {"mirror": True}, True),
            ({"has_key": []}, {"force": True}, False),
            # has_all_keys — EVERY named key is present
            ({"has_all_keys": ["force", "remote"]}, {"force": True, "remote": "o"}, True),
            ({"has_all_keys": ["force", "remote"]}, {"force": True}, False),
            ({"has_all_keys": []}, {}, True),
            # has_none_of — NONE of the named keys is present
            ({"has_none_of": ["force"]}, {"remote": "origin"}, True),
            ({"has_none_of": ["force"]}, {"force": False}, False),
            ({"has_none_of": []}, {"force": True}, True),
            # several predicates in one block are AND-ed like any condition object
            ({"has_key": ["force"], "has_none_of": ["dry_run"]}, {"force": True}, True),
            ({"has_key": ["force"], "has_none_of": ["dry_run"]}, {"force": True, "dry_run": 1}, False),
        ],
    )
    def test_predicates(self, predicate: dict[str, Any], arguments: dict[str, Any], expected: bool) -> None:
        acl = _acl(_force_rule(conditions={"arguments": predicate}), default_effect="deny")
        # An `allow`-with-approval rule makes check() fail closed either way, so
        # the predicate's verdict is read off the structured result.
        decision = acl.check_access("agent.a", "cli.git_push", _ctx(projection_of=arguments))
        assert (decision.access == "allow") is expected

    def test_no_predicate_reads_a_value(self) -> None:
        """`has_key` answers presence, never the value bound to the key."""
        acl = _acl(_force_rule(), default_effect="deny")
        for value in (True, False, None, "", 0, {"nested": "secret"}):
            decision = acl.check_access("agent.a", "cli.git_push", _ctx(projection_of={"force": value}))
            assert decision.access == "allow", f"presence must not depend on the value {value!r}"

    def test_it_is_registered_as_an_ordinary_builtin(self) -> None:
        """§6.1.7: registered the way the other built-ins are, with no new

        registration point — which is also what gets it §6.1.4's precheck free.
        """
        assert "arguments" in ACL._condition_handlers

    def test_a_misspelled_condition_key_is_unevaluable_not_inert(self) -> None:
        """`argument:` written for `arguments:` — the §6.1.4 precheck catches it."""
        deny = ACLRule(
            callers=["*"],
            targets=["cli.git_push"],
            effect="deny",
            conditions={"argument": {"has_key": ["force"]}},
        )
        audit: list[AuditEntry] = []
        acl = _acl(deny, default_effect="allow", audit=audit)
        assert acl.check("agent.a", "cli.git_push", _ctx(projection_of={"force": True})) is False
        assert audit[-1].handler_error is not None
        assert "argument" in audit[-1].handler_error

    @pytest.mark.parametrize(
        "malformed",
        [
            {"has_key": "force"},  # bare string: iterable, so read char by char
            {"has_key": [1, 2]},
            {"has_key": None},
            {"has_keys": ["force"]},  # not in the closed predicate vocabulary
            {"equals": {"force": True}},  # a value predicate, deliberately unspecified
            "force",  # the whole block is not a mapping
            ["force"],
            {},  # constrains nothing — fail closed, as §6.1's empty `$not` does
        ],
    )
    def test_a_malformed_predicate_is_unevaluable_not_false(self, malformed: Any) -> None:
        """§6.1.1's principle, not UNSATISFIED: a `deny` rule takes effect and an

        `allow` rule does not grant. Answering "no" would put an `allow` rule's
        `has_none_of` typo back into the silently-inert state §6.1.1 exists to end.
        """
        audit: list[AuditEntry] = []
        deny = ACLRule(
            callers=["*"],
            targets=["cli.git_push"],
            effect="deny",
            conditions={"arguments": malformed},
        )
        acl = _acl(deny, default_effect="allow", audit=audit)
        assert acl.check("agent.a", "cli.git_push", _ctx(projection_of={"force": True})) is False
        assert audit[-1].handler_error is not None
        assert "arguments" in audit[-1].handler_error

        audit.clear()
        allow = ACLRule(
            callers=["*"],
            targets=["cli.git_push"],
            effect="allow",
            conditions={"arguments": malformed},
        )
        acl = _acl(allow, default_effect="deny", audit=audit)
        assert acl.check("agent.a", "cli.git_push", _ctx(projection_of={"force": True})) is False
        assert audit[-1].handler_error is not None

    @pytest.mark.parametrize(
        "malformed",
        [
            ({"has_key": "force"}, "arguments.has_key"),  # a malformed predicate value
            ({"has_keys": ["force"]}, "arguments.has_keys"),  # an unrecognised predicate name
            ({}, "arguments"),  # an empty block — no predicate to name
        ],
    )
    def test_the_precheck_covers_the_predicate_structure(self, malformed: Any) -> None:
        """§6.1.4: all three are decidable with no context and no handler, so they

        are precheck faults and `validate_rules()` reports them (§6.1.2 rule 3) —
        not merely the `arguments` key's registry status.

        §6.1.8 fixes the path: it descends to the offending predicate where one
        can be named, exactly as §6.1.4 descends into ``$or[1].k``, and stops at
        ``arguments`` where none can be.
        """
        value, expected_path = malformed
        acl = _acl(_force_rule(conditions={"arguments": value}))
        findings = acl.validate_rules()
        assert len(findings) == 1
        assert findings[0].condition_path == expected_path

    def test_a_malformed_predicate_is_reported_by_validate_rules(self) -> None:
        """The precheck is context-free, so `validate_rules()` sees it too (§6.1.2)."""
        acl = _acl(_force_rule(conditions={"arguments": {"has_key": "force"}}))
        findings = acl.validate_rules()
        assert len(findings) == 1
        # §6.1.8: the path descends to the predicate; the KEY stays `arguments`,
        # for a reader who wants the condition rather than its position.
        assert findings[0].condition_path == "arguments.has_key"
        assert findings[0].condition_key == "arguments"
        # A structural fault resolves on neither evaluation path, however well
        # registered the key is (§6.1.3 rule 3).
        assert findings[0].sync_resolvable is False
        assert findings[0].async_resolvable is False

    def test_a_wellformed_arguments_condition_is_not_a_finding(self) -> None:
        assert _acl(_force_rule()).validate_rules() == ()

    def test_an_absent_projection_is_unevaluable_not_an_empty_key_set(self) -> None:
        """§6.1.8: reading no projection as "no arguments" would make

        `has_none_of` vacuously true, and an `allow` rule gated on it would then
        grant on every call from an entry point that populates no projection.
        """
        allow = ACLRule(
            callers=["*"],
            targets=["cli.git_push"],
            effect="allow",
            conditions={"arguments": {"has_none_of": ["force"]}},
        )
        acl = _acl(allow, default_effect="deny")
        assert acl.check("agent.a", "cli.git_push", _ctx()) is False

    @pytest.mark.asyncio
    async def test_the_async_path_resolves_it_too(self) -> None:
        acl = _acl(_force_rule(), default_effect="deny")
        decision = await acl.async_check_access("agent.a", "cli.git_push", _ctx(projection_of={"force": True}))
        assert decision.access == "allow"
        assert decision.approval_required is True

    def test_it_composes_under_the_compound_operators(self) -> None:
        rule = _force_rule(
            conditions={"$not": {"arguments": {"has_key": ["force"]}}},
            approval="not_required",
        )
        acl = _acl(rule, default_effect="deny")
        assert acl.check("agent.a", "cli.git_push", _ctx(projection_of={"remote": "o"})) is True
        assert acl.check("agent.a", "cli.git_push", _ctx(projection_of={"force": True})) is False


# ---------------------------------------------------------------------------
# §6.1.8 — the governance projection
# ---------------------------------------------------------------------------


class TestGovernanceProjection:
    """Keys and types, never a value (§6.1.8)."""

    def test_it_carries_keys_and_types_and_no_value(self) -> None:
        projection = GovernanceProjection.of(
            {"force": True, "remote": "origin", "depth": 3, "ratio": 1.5, "tags": [], "meta": {}, "none": None}
        )
        assert projection.keys == {"force", "remote", "depth", "ratio", "tags", "meta", "none"}
        assert projection.types == {
            "force": "boolean",
            "remote": "string",
            "depth": "integer",
            "ratio": "number",
            "tags": "array",
            "meta": "object",
            "none": "null",
        }

    def test_no_value_survives_the_projection(self) -> None:
        """Structural, not filtered: there is no field a value could live in.

        A projection that structurally cannot hold a value cannot leak one,
        whatever a future predicate does with it.
        """
        secret = "hunter2-do-not-leak"
        projection = GovernanceProjection.of({"password": secret, "token": {"inner": secret}})
        assert secret not in repr(projection)
        assert secret not in str(projection.types)
        assert set(projection.types.values()) == {"string", "object"}
        assert not any(isinstance(v, (dict, list)) for v in projection.types.values())

    def test_bool_is_not_reported_as_an_integer(self) -> None:
        """Python's ``bool`` subclasses ``int``; the other test order loses every flag."""
        assert GovernanceProjection.of({"force": True}).types["force"] == "boolean"

    def test_a_value_with_no_json_counterpart_is_unknown(self) -> None:
        assert GovernanceProjection.of({"obj": object()}).types["obj"] == "unknown"

    def test_the_approval_token_is_not_projected(self) -> None:
        """§7.4's resume re-enters from Step 1, so a projection carrying the

        framework-owned token would let the ACL reach a different Step 4 verdict
        on the resume than on the call a human just approved.
        """
        projection = GovernanceProjection.of({"force": True, "_approval_token": "tok-1"})
        assert projection.keys == {"force"}

    def test_it_is_not_redacted_inputs(self) -> None:
        """§6.1.8 rule 3 forbids substituting ``redacted_inputs``, whose contract

        is safe *logging* and which is a raw copy when there is no input schema.
        """
        seen: list[Context] = []
        projections: list[Any] = []
        reg = Registry()

        class Recording(GitPush):
            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                seen.append(context)
                return {"pushed": True}

        class Probe(BaseStep):
            def __init__(self) -> None:
                super().__init__(name="probe", description="record the projection", pure=True)

            async def execute(self, ctx: PipelineContext) -> StepResult:
                projections.append(ctx.governance_projection)
                return StepResult(action="continue")

        reg.register("cli.git_push", Recording())
        strategy = build_standard_strategy(registry=reg)
        strategy.insert_before("acl_check", Probe())
        Executor(registry=reg, strategy=strategy).call("cli.git_push", {"force": True, "remote": "origin"})

        ctx = seen[0]
        # The module declares a permissive schema with no x-sensitive markers,
        # so redacted_inputs holds the values verbatim...
        assert ctx.redacted_inputs == {"force": True, "remote": "origin"}
        # ...while the projection carried beside it holds none of them.
        #
        # CTX-2: the projection lives on the per-call PipelineContext, not on
        # the execution Context — that object is handed to module code and
        # outlives the ACL check it was computed for.
        assert ctx.governance_projection is None
        projection = projections[0]
        assert projection is not None
        assert projection.keys == {"force", "remote"}
        assert projection.types == {"force": "boolean", "remote": "string"}

    def test_it_is_populated_before_step_4(self) -> None:
        """§6.1.8 rule 1: computed at Step 3 and available at Step 4. The

        ordering is normative here, not an implementation detail that happens to
        hold — the `arguments` condition has nothing to read otherwise.
        """
        seen: list[Any] = []

        class Probe(BaseStep):
            def __init__(self) -> None:
                super().__init__(name="probe", description="record the projection", pure=True)

            async def execute(self, ctx: PipelineContext) -> StepResult:
                seen.append(ctx.governance_projection)
                return StepResult(action="continue")

        reg = _registry()
        strategy = build_standard_strategy(registry=reg)
        strategy.insert_before("acl_check", Probe())
        # The probe sits between module_lookup (Step 3) and acl_check (Step 4).
        names = strategy.step_names()
        assert names.index("module_lookup") < names.index("probe") < names.index("acl_check")

        Executor(registry=reg, strategy=strategy).call("cli.git_push", {"force": True})
        assert len(seen) == 1
        assert seen[0] is not None, "the projection was not populated before Step 4"
        assert seen[0].keys == {"force"}

    @pytest.mark.asyncio
    async def test_step_3_populates_it_from_the_call_arguments(self) -> None:
        reg = _registry()
        lookup = BuiltinModuleLookup(registry=reg)
        ctx = PipelineContext(module_id="cli.git_push", inputs={"force": True}, context=Context.create())
        await lookup.execute(ctx)
        # CTX-2: Step 3 writes it onto the PipelineContext, not onto the
        # caller-visible execution Context.
        assert ctx.governance_projection == GovernanceProjection.of({"force": True})
        assert ctx.context.governance_projection is None

    def test_it_does_not_reach_the_serialized_context(self) -> None:
        """Transient, like ``executor`` and ``services``."""
        ctx = _ctx(projection_of={"force": True})
        assert "governance_projection" not in ctx.serialize()
        assert Context.deserialize(ctx.serialize()).governance_projection is None


# ---------------------------------------------------------------------------
# §6.8.1 — AccessDecision and the fail-closed boolean
# ---------------------------------------------------------------------------


class TestAccessDecision:
    """The structured result, and the boolean that must fail closed (§6.8.1)."""

    def test_it_carries_all_four_members(self) -> None:
        acl = _acl(_force_rule(), default_effect="deny")
        decision = acl.check_access("agent.a", "cli.git_push", _ctx(projection_of={"force": True}))
        assert isinstance(decision, AccessDecision)
        assert decision.access == "allow"
        assert decision.approval_required is True
        assert decision.matched_rule_index == 0
        assert decision.reason == "rule_match"

    def test_a_plain_allow_requires_no_approval(self) -> None:
        acl = _acl(ACLRule(callers=["*"], targets=["*"], effect="allow"), default_effect="deny")
        decision = acl.check_access("agent.a", "cli.git_push", _ctx(projection_of={}))
        assert (decision.access, decision.approval_required) == ("allow", False)

    def test_a_deny_carries_no_approval_requirement(self) -> None:
        acl = _acl(ACLRule(callers=["*"], targets=["*"], effect="deny"), default_effect="allow")
        decision = acl.check_access("agent.a", "cli.git_push", _ctx(projection_of={"force": True}))
        assert (decision.access, decision.approval_required) == ("deny", False)

    def test_no_match_means_no_approval_requirement(self) -> None:
        """§6.9 row 2: there is no default approval requirement."""
        acl = _acl(_force_rule(callers=["nobody.*"]), default_effect="allow")
        decision = acl.check_access("agent.a", "cli.git_push", _ctx(projection_of={"force": True}))
        assert decision.approval_required is False
        assert decision.matched_rule_index is None
        assert decision.reason == "default_effect"

    def test_no_rules_reports_no_rules(self) -> None:
        decision = _acl(default_effect="allow").check_access("agent.a", "cli.git_push", _ctx())
        assert decision.reason == "no_rules"

    def test_the_legacy_boolean_fails_closed_on_an_approval_requirement(self) -> None:
        """§6.8.1. A non-Executor caller can only read a boolean as "let it

        through", and returning True would run a call the ACL said needed a
        human. False is wrong in the benign direction.
        """
        acl = _acl(_force_rule(), default_effect="deny")
        ctx = _ctx(projection_of={"force": True})
        assert acl.check_access("agent.a", "cli.git_push", ctx).access == "allow"
        assert acl.check("agent.a", "cli.git_push", ctx) is False

    def test_the_legacy_boolean_is_unchanged_without_an_approval_requirement(self) -> None:
        acl = _acl(_force_rule(), default_effect="deny")
        # No `force` in the arguments: the rule does not match at all.
        assert acl.check("agent.a", "cli.git_push", _ctx(projection_of={"remote": "o"})) is False
        plain = _acl(ACLRule(callers=["*"], targets=["*"], effect="allow"), default_effect="deny")
        assert plain.check("agent.a", "cli.git_push", _ctx(projection_of={"force": True})) is True

    @pytest.mark.asyncio
    async def test_the_async_legacy_boolean_fails_closed_too(self) -> None:
        acl = _acl(_force_rule(), default_effect="deny")
        ctx = _ctx(projection_of={"force": True})
        assert (await acl.async_check_access("agent.a", "cli.git_push", ctx)).access == "allow"
        assert await acl.async_check("agent.a", "cli.git_push", ctx) is False

    def test_one_audit_entry_per_check_whichever_accessor_is_used(self) -> None:
        """§6.3.1: exactly one entry per check, and the two accessors are the

        same decision at different widths — not two decisions.
        """
        audit: list[AuditEntry] = []
        acl = _acl(_force_rule(), default_effect="deny", audit=audit)
        ctx = _ctx(projection_of={"force": True})
        acl.check_access("agent.a", "cli.git_push", ctx)
        assert len(audit) == 1
        acl.check("agent.a", "cli.git_push", ctx)
        assert len(audit) == 2


# ---------------------------------------------------------------------------
# §6.3.1 — AuditEntry.approval_required
# ---------------------------------------------------------------------------


class TestAuditEntryApprovalRequired:
    """A new field beside ``decision``, not a third ``decision`` value (§6.3.1)."""

    def test_decision_stays_a_two_valued_string(self) -> None:
        """§6.9 row 7: ``decision`` is a string downstream consumers parse, and a

        third value would break every existing parser.
        """
        audit: list[AuditEntry] = []
        acl = _acl(_force_rule(), default_effect="deny", audit=audit)
        acl.check_access("agent.a", "cli.git_push", _ctx(projection_of={"force": True}))
        assert audit[0].decision == "allow"
        assert audit[0].approval_required is True

    def test_it_defaults_to_false(self) -> None:
        entry = AuditEntry(
            timestamp="2026-01-01T00:00:00+00:00",
            caller_id="a",
            target_id="b",
            decision="allow",
            reason="rule_match",
        )
        assert entry.approval_required is False

    def test_false_when_no_rule_matched(self) -> None:
        audit: list[AuditEntry] = []
        _acl(default_effect="allow", audit=audit).check("agent.a", "cli.git_push", _ctx())
        assert audit[0].approval_required is False

    def test_false_when_the_matched_rule_required_none(self) -> None:
        audit: list[AuditEntry] = []
        acl = _acl(ACLRule(callers=["*"], targets=["*"], effect="allow"), default_effect="deny", audit=audit)
        acl.check("agent.a", "cli.git_push", _ctx())
        assert audit[0].approval_required is False


# ---------------------------------------------------------------------------
# §6.9 / §7.4 / §7.9.5 — governance precedence
# ---------------------------------------------------------------------------


def _recording_handler(seen: list[ApprovalRequest], status: str = "approved") -> CallbackApprovalHandler:
    async def handler(request: ApprovalRequest) -> ApprovalResult:
        seen.append(request)
        return ApprovalResult(status=status, approved_by="tester")

    return CallbackApprovalHandler(handler)


class TestGovernancePrecedence:
    """The union at Step 5 and the row-4 privilege-escalation guard (§6.9)."""

    def test_an_acl_sourced_requirement_reaches_step_5(self) -> None:
        """§7.4: an implementation that reads only the annotation silently

        ignores every rule carrying `approval` — the rule loads, matches, and
        does nothing.
        """
        seen: list[ApprovalRequest] = []
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            approval_handler=_recording_handler(seen),
        )
        executor.call("cli.git_push", {"force": True})
        assert len(seen) == 1, "the ACL's approval requirement never reached the gate"
        assert seen[0].module_id == "cli.git_push"

    def test_the_gate_does_not_fire_when_the_rule_does_not_match(self) -> None:
        """Argument-scoped, not module-scoped: that is the whole point. Gating

        every call to a module in order to gate some of them is what weakens
        `requires_approval` from "this needs approval" to "this might".
        """
        seen: list[ApprovalRequest] = []
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            approval_handler=_recording_handler(seen),
        )
        executor.call("cli.git_push", {"remote": "origin"})
        assert seen == []

    def test_step_4_does_not_deny_a_call_that_merely_needs_approval(self) -> None:
        """The gate, not the ACL step, is what stops it — otherwise "ask a human"

        silently becomes a flat refusal and Step 5 never sees the requirement.
        """
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            approval_handler=_recording_handler([], status="rejected"),
        )
        with pytest.raises(ApprovalDeniedError):
            executor.call("cli.git_push", {"force": True})

    def test_a_pending_requirement_reaches_step_5_too(self) -> None:
        """§6.1.1 rule 5 on the ordinary Executor pipeline (spec v1.29.0, #109).

        `has_keys` is not a predicate name, so the gate is unevaluable **with the
        projection present** — §6.1.2 makes an unregistered key a warning rather
        than a load failure, so `validate_rules()` at deploy time is not a
        mitigation that can be assumed. Before v1.29.0 the gate stepped aside,
        the broad rule granted, and one typo turned "ask a human" into "do not":
        the forced push ran with `seen == []`.
        """
        seen: list[ApprovalRequest] = []
        executor = Executor(
            registry=_registry(),
            acl=_acl(
                _force_rule(conditions={"arguments": {"has_keys": ["force"]}}),
                ACLRule(callers=["*"], targets=["cli.git_push"], effect="allow"),
                default_effect="deny",
            ),
            approval_handler=_recording_handler(seen),
        )
        executor.call("cli.git_push", {"force": True})
        assert len(seen) == 1, "the pending approval requirement never reached the gate"

    def test_an_out_of_scope_gate_does_not_reach_step_5(self) -> None:
        """Rule 5's containment at the pipeline level: the gate is written about

        another module, so an ordinary `git push` must not acquire a human.
        """
        seen: list[ApprovalRequest] = []
        executor = Executor(
            registry=_registry(),
            acl=_acl(
                _force_rule(targets=["cli.deploy"], conditions={"arguments": {"has_keys": ["force"]}}),
                ACLRule(callers=["*"], targets=["cli.git_push"], effect="allow"),
                default_effect="deny",
            ),
            approval_handler=_recording_handler(seen),
        )
        executor.call("cli.git_push", {"force": True})
        assert seen == []

    def test_an_acl_denial_still_denies(self) -> None:
        deny = ACLRule(callers=["*"], targets=["cli.git_push"], effect="deny")
        executor = Executor(registry=_registry(), acl=_acl(deny, default_effect="allow"))
        with pytest.raises(ACLDeniedError):
            executor.call("cli.git_push", {"force": True})

    def test_the_handler_sees_requires_approval_true(self) -> None:
        """§7.4 rule 3 / §7.9.3: the ApprovalRequest carries the EFFECTIVE

        annotations, so §7.3's "requires_approval is guaranteed true" holds for
        an ACL-sourced requirement as it does for a policy-sourced one.
        """
        seen: list[ApprovalRequest] = []
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            approval_handler=_recording_handler(seen),
        )
        executor.call("cli.git_push", {"force": True})
        assert seen[0].annotations.requires_approval is True
        # The module itself declares False — the gate must not hand that on.
        assert GitPush.annotations.requires_approval is False

    def test_row_3_union_with_the_module_annotation(self) -> None:
        """Either source may require a human; neither may cancel the other."""
        seen: list[ApprovalRequest] = []
        reg = Registry()

        class Annotated(GitPush):
            annotations = ModuleAnnotations(requires_approval=True)

        reg.register("cli.git_push", Annotated())
        executor = Executor(
            registry=reg,
            # A matching rule that requires nothing must not clear the annotation.
            acl=_acl(ACLRule(callers=["*"], targets=["*"], effect="allow"), default_effect="deny"),
            approval_handler=_recording_handler(seen),
        )
        executor.call("cli.git_push", {"remote": "origin"})
        assert len(seen) == 1

    def test_row_4_a_policy_must_not_clear_the_acl_requirement(self) -> None:
        """The privilege-escalation guard. The ACL is caller-scoped; an

        ExecutionPolicy is module-scoped, and letting a module-scoped override
        cancel a caller-scoped decision means a policy rule written for
        `cli.*` silently strips a requirement an ACL author attached to one
        untrusted caller.
        """
        seen: list[ApprovalRequest] = []
        policy = ExecutionPolicy([PolicyRule("cli.*", requires_approval=False, reason="platform says no gate")])
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            approval_handler=_recording_handler(seen),
            policy=policy,
        )
        executor.call("cli.git_push", {"force": True})
        assert len(seen) == 1, "an ExecutionPolicy cleared a requirement the ACL set"

    def test_row_4_a_policy_may_still_add_a_requirement(self) -> None:
        """The guard is one-directional: a policy may ADD, never remove."""
        seen: list[ApprovalRequest] = []
        policy = ExecutionPolicy([PolicyRule("cli.*", requires_approval=True)])
        executor = Executor(
            registry=_registry(),
            acl=_acl(ACLRule(callers=["*"], targets=["*"], effect="allow"), default_effect="deny"),
            approval_handler=_recording_handler(seen),
            policy=policy,
        )
        executor.call("cli.git_push", {"remote": "origin"})
        assert len(seen) == 1

    def test_row_4_a_policy_clearing_the_module_annotation_still_works(self) -> None:
        """ "A policy `requires_approval: false` overrides the module's

        *annotation*, never the ACL's decision" — the annotation half is
        unchanged.
        """
        seen: list[ApprovalRequest] = []
        reg = Registry()

        class Annotated(GitPush):
            annotations = ModuleAnnotations(requires_approval=True)

        reg.register("cli.git_push", Annotated())
        executor = Executor(
            registry=reg,
            acl=_acl(ACLRule(callers=["*"], targets=["*"], effect="allow"), default_effect="deny"),
            approval_handler=_recording_handler(seen),
            policy=ExecutionPolicy([PolicyRule("cli.*", requires_approval=False)]),
        )
        executor.call("cli.git_push", {"remote": "origin"})
        assert seen == []

    def test_row_5_gate_destructive_still_contributes(self) -> None:
        seen: list[ApprovalRequest] = []
        reg = Registry()

        class Destructive(GitPush):
            annotations = ModuleAnnotations(requires_approval=False, destructive=True)

        reg.register("cli.git_push", Destructive())
        executor = Executor(
            registry=reg,
            acl=_acl(ACLRule(callers=["*"], targets=["*"], effect="allow"), default_effect="deny"),
            approval_handler=_recording_handler(seen),
            policy=ExecutionPolicy(gate_destructive=True),
        )
        executor.call("cli.git_push", {"remote": "origin"})
        assert len(seen) == 1

    def test_strict_fails_closed_on_an_acl_sourced_requirement(self) -> None:
        """§7.9.4 rule 1 does not care where the requirement came from."""
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            policy=ExecutionPolicy(strict=True),
        )
        with pytest.raises(ApprovalDeniedError):
            executor.call("cli.git_push", {"force": True})
        # ...and a call the rule does not match is untouched.
        assert executor.call("cli.git_push", {"remote": "origin"}) == {"pushed": True}

    def test_row_6_preflight_reports_the_union(self) -> None:
        """§7.9.5: `validate()` reports the GOVERNANCE-effective requirement.

        Reporting only the policy-effective value would tell a caller no
        approval is needed for a call the gate will stop.
        """
        executor = Executor(registry=_registry(), acl=_acl(_force_rule(), default_effect="allow"))
        assert executor.validate("cli.git_push", {"force": True}).requires_approval is True
        assert executor.validate("cli.git_push", {"remote": "origin"}).requires_approval is False

    def test_row_6_preflight_union_survives_a_clearing_policy(self) -> None:
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            policy=ExecutionPolicy([PolicyRule("cli.*", requires_approval=False)]),
        )
        assert executor.validate("cli.git_push", {"force": True}).requires_approval is True

    def test_preflight_stays_non_destructive(self) -> None:
        """A dry run reports; it must not put anything to a human."""
        seen: list[ApprovalRequest] = []
        executor = Executor(
            registry=_registry(),
            acl=_acl(_force_rule(), default_effect="allow"),
            approval_handler=_recording_handler(seen),
        )
        result = executor.validate("cli.git_push", {"force": True})
        assert result.requires_approval is True
        assert seen == []

    @pytest.mark.asyncio
    async def test_step_4_records_the_requirement_on_the_pipeline_context(self) -> None:
        """The plumbing §7.4 rule 1 needs: Step 4's second result reaches Step 5."""
        step = BuiltinACLCheck(acl=_acl(_force_rule(), default_effect="allow"))
        ctx = PipelineContext(
            module_id="cli.git_push",
            inputs={"force": True},
            context=_ctx(projection_of={"force": True}, caller_id="agent.a"),
        )
        await step.execute(ctx)
        assert ctx.acl_approval_required is True

    @pytest.mark.asyncio
    async def test_a_legacy_acl_object_contributes_no_requirement(self) -> None:
        """A custom ACL with only the boolean cannot carry one, and inferring one

        would be inventing governance.
        """

        class LegacyACL:
            def check(self, caller_id: Any, target_id: str, context: Any = None) -> bool:
                return True

        ctx = PipelineContext(module_id="cli.git_push", inputs={}, context=_ctx())
        await BuiltinACLCheck(acl=LegacyACL()).execute(ctx)
        assert ctx.acl_approval_required is False
