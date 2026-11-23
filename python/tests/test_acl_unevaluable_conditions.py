"""Unevaluable ACL conditions, load-time validation, and ACL introspection.

Covers PROTOCOL_SPEC §6.1.1 / §6.1.2 / §6.1.3 / §6.1.4 / §6.1.4.1 / §6.3 /
§6.3.1 / §6.5 (apcore#100, #106) and §6.8 (apcore#101).

The defect these lock down: ``evaluate_conditions`` returned a plain boolean, so
"a handler answered no" and "no answer was obtainable" arrived at the rule loop
identically, and both meant *this rule does not match*. That is safe in one
direction only — an ``allow`` rule that cannot evaluate its condition does not
grant, but a ``deny`` rule that cannot evaluate its condition did not block.
A single misspelled key (``role:`` for ``roles:``) turned a rule its author
believed was blocking into decoration.
"""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path
from typing import Any

import pytest

from apcore.acl import ACL, ACLRule, AuditEntry, ConditionOutcome, RuleValidationFinding
from apcore.context import Context, Identity
from apcore.errors import ACLRuleError


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _ctx(roles: list[str] | None = None, identity_type: str = "user") -> Context:
    return Context.create(identity=Identity(id="u", type=identity_type, roles=tuple(roles or ())))


def _acl(
    conditions: dict[str, Any] | None,
    *,
    effect: str = "deny",
    default_effect: str = "allow",
) -> tuple[ACL, list[AuditEntry]]:
    captured: list[AuditEntry] = []
    acl = ACL(
        rules=[ACLRule(callers=["*"], targets=["*"], effect=effect, conditions=conditions)],
        default_effect=default_effect,
        audit_logger=captured.append,
    )
    return acl, captured


class _Raising:
    def evaluate(self, value: Any, context: Context) -> bool:
        raise RuntimeError("boom")


class _Suspending:
    async def evaluate(self, value: Any, context: Context) -> bool:
        await asyncio.sleep(0)
        return True


class _Answers:
    def __init__(self, answer: bool) -> None:
        self._answer = answer

    def evaluate(self, value: Any, context: Context) -> bool:
        return self._answer


@pytest.fixture
def registered():
    """Register condition handlers for the duration of one test."""
    keys: list[str] = []
    async_keys: list[str] = []

    def _register(key: str, handler: Any, *, asynchronous: bool = False) -> str:
        if asynchronous:
            ACL.register_async_condition(key, handler)
            async_keys.append(key)
        else:
            ACL.register_condition(key, handler)
            keys.append(key)
        return key

    try:
        yield _register
    finally:
        for key in keys:
            ACL._condition_handlers.pop(key, None)
        for key in async_keys:
            ACL._async_condition_handlers.pop(key, None)


# ---------------------------------------------------------------------------
# §6.1.1 — the effect rule
# ---------------------------------------------------------------------------


class TestUnevaluableResolvesTowardRefusingAccess:
    """A `deny` rule with an unevaluable condition MATCHES and DENIES."""

    def test_unknown_key_on_deny_rule_denies_over_default_allow(self) -> None:
        """The misspelled-key case #100 was opened for."""
        acl, captured = _acl({"role": ["admin"]}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx()) is False
        assert captured[0].decision == "deny"
        assert captured[0].reason == "rule_match"
        assert captured[0].matched_rule_index == 0

    def test_raising_handler_on_deny_rule_denies_over_default_allow(self, registered) -> None:
        key = registered("_t_raises", _Raising())
        acl, captured = _acl({key: True}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx()) is False
        assert captured[0].decision == "deny"

    def test_suspending_handler_on_deny_rule_denies_over_default_allow(self, registered) -> None:
        key = registered("_t_suspends", _Suspending())
        acl, captured = _acl({key: True}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx()) is False
        assert captured[0].decision == "deny"

    def test_unknown_key_on_allow_rule_still_does_not_grant(self) -> None:
        acl, captured = _acl({"role": ["admin"]}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx()) is False
        assert captured[0].reason == "default_effect"
        assert captured[0].matched_rule_index is None

    def test_unevaluable_allow_rule_falls_through_to_a_later_rule(self, registered) -> None:
        """An `allow` rule that cannot evaluate MUST NOT match — evaluation continues."""
        key = registered("_t_raises2", _Raising())
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="allow", conditions={key: True}),
                ACLRule(callers=["*"], targets=["*"], effect="allow", description="fallback"),
            ],
            default_effect="deny",
            audit_logger=captured.append,
        )
        assert acl.check("caller", "target", _ctx()) is True
        assert captured[0].matched_rule_index == 1
        assert captured[0].matched_rule == "fallback"

    def test_an_unevaluable_condition_never_raises_out_of_check(self, registered) -> None:
        """§6.1.1 rule 4: check() still returns a boolean."""
        key = registered("_t_raises3", _Raising())
        acl, _ = _acl({key: True}, effect="deny")
        assert isinstance(acl.check("caller", "target", _ctx()), bool)

    async def test_the_effect_rule_holds_on_the_async_path(self, registered) -> None:
        key = registered("_t_raises_async", _Raising())
        acl, captured = _acl({key: True}, effect="deny", default_effect="allow")
        assert await acl.async_check("caller", "target", _ctx()) is False
        assert captured[0].handler_error is not None

    def test_an_ordinary_false_condition_on_a_deny_rule_still_does_not_match(self, registered) -> None:
        """The counterpart guarantee: UNSATISFIED must NOT be promoted to a deny."""
        key = registered("_t_false", _Answers(False))
        acl, captured = _acl({key: True}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx()) is True
        assert captured[0].reason == "default_effect"
        assert captured[0].handler_error is None


# ---------------------------------------------------------------------------
# §6.1.1 — three-valued composition through AND, $or and $not
# ---------------------------------------------------------------------------


class TestCompoundComposition:
    def test_and_an_outright_no_wins_over_an_unevaluable_sibling(self, registered) -> None:
        """AND: any child UNSATISFIED -> UNSATISFIED, even beside an unevaluable one."""
        key = registered("_t_raises4", _Raising())
        # ``roles`` is evaluated first and answers "no", short-circuiting before
        # the raising handler is ever reached.
        acl, captured = _acl({"roles": ["admin"], key: True}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx(roles=["viewer"])) is True
        # Short-circuited child was never evaluated, so it records no diagnostic.
        assert captured[0].handler_error is None

    def test_and_unevaluable_first_then_no_is_still_unsatisfied(self, registered) -> None:
        """The decision is identical whichever order the keys are reached in."""
        key = registered("_t_raises5", _Raising())
        acl, captured = _acl({key: True, "roles": ["admin"]}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx(roles=["viewer"])) is True
        # Only the diagnostic differs: this key WAS reached, so it is reported.
        assert captured[0].handler_error is not None

    def test_and_unevaluable_with_all_others_satisfied_is_unevaluable(self, registered) -> None:
        key = registered("_t_raises6", _Raising())
        acl, _ = _acl({"roles": ["admin"], key: True}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx(roles=["admin"])) is False

    def test_or_an_outright_yes_wins_over_an_unevaluable_sibling(self, registered) -> None:
        key = registered("_t_raises7", _Raising())
        acl, _ = _acl(
            {"$or": [{"roles": ["admin"]}, {key: True}]},
            effect="allow",
            default_effect="deny",
        )
        assert acl.check("caller", "target", _ctx(roles=["admin"])) is True

    def test_or_no_yes_and_an_unevaluable_child_is_unevaluable(self, registered) -> None:
        key = registered("_t_raises8", _Raising())
        acl, captured = _acl(
            {"$or": [{"roles": ["admin"]}, {key: True}]},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("caller", "target", _ctx(roles=["viewer"])) is False
        assert captured[0].handler_error is not None

    def test_or_all_children_unsatisfied_is_unsatisfied(self) -> None:
        acl, captured = _acl(
            {"$or": [{"roles": ["admin"]}, {"identity_types": ["service"]}]},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("caller", "target", _ctx(roles=["viewer"])) is True
        assert captured[0].handler_error is None

    def test_not_of_unevaluable_is_unevaluable_not_satisfied(self, registered) -> None:
        """The bypass §6.1.1 closes one nesting level down.

        If ``$not`` negated "no answer" into "yes", a misspelled key inside a
        ``$not`` would SATISFY the very rule it was meant to gate.
        """
        acl, captured = _acl({"$not": {"mispelled": True}}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx()) is False, "an unevaluable $not must not grant"
        assert captured[0].handler_error is not None

    def test_not_of_unevaluable_on_a_deny_rule_denies(self) -> None:
        acl, _ = _acl({"$not": {"mispelled": True}}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx()) is False

    def test_not_of_satisfied_is_unsatisfied(self) -> None:
        acl, captured = _acl({"$not": {"roles": ["admin"]}}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx(roles=["admin"])) is False
        assert captured[0].handler_error is None

    def test_not_of_unsatisfied_is_satisfied(self) -> None:
        acl, _ = _acl({"$not": {"roles": ["admin"]}}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx(roles=["viewer"])) is True

    def test_not_of_empty_object_is_unsatisfied_not_unevaluable(self) -> None:
        """§6.1: `$not: {}` MUST evaluate to false, and it is a real answer."""
        acl, captured = _acl({"$not": {}}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx()) is False
        assert captured[0].handler_error is None

    def test_nested_or_inside_not_propagates_unevaluable(self, registered) -> None:
        key = registered("_t_raises9", _Raising())
        acl, _ = _acl(
            {"$not": {"$or": [{key: True}]}},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("caller", "target", _ctx()) is False

    async def test_async_not_of_unevaluable_is_unevaluable(self) -> None:
        acl, captured = _acl({"$not": {"mispelled": True}}, effect="allow", default_effect="deny")
        assert await acl.async_check("caller", "target", _ctx()) is False
        assert captured[0].handler_error is not None

    async def test_async_or_no_yes_and_an_unevaluable_child_is_unevaluable(self, registered) -> None:
        key = registered("_t_raises10", _Raising())
        acl, _ = _acl(
            {"$or": [{"roles": ["admin"]}, {key: True}]},
            effect="deny",
            default_effect="allow",
        )
        assert await acl.async_check("caller", "target", _ctx(roles=["viewer"])) is False

    def test_or_short_circuits_before_an_unevaluable_sibling_records_nothing(self, registered) -> None:
        """A child skipped by a legitimate short-circuit MUST NOT set handler_error."""
        key = registered("_t_raises11", _Raising())
        acl, captured = _acl(
            {"$or": [{"roles": ["admin"]}, {key: True}]},
            effect="allow",
            default_effect="deny",
        )
        assert acl.check("caller", "target", _ctx(roles=["admin"])) is True
        assert captured[0].handler_error is None


# ---------------------------------------------------------------------------
# §6.3.1 — handler_error aggregation
# ---------------------------------------------------------------------------


class TestHandlerErrorAggregation:
    def test_multiple_unevaluable_keys_are_listed_lexicographically(self) -> None:
        """§6.1.1 rule 2: every unevaluable condition, ordered by KEY, joined by '; '.

        Lexicographic rather than evaluation order because Python dicts keep
        insertion order while serde_json's map is sorted — "the first one
        encountered" would put a different key in the audit log per SDK.
        """
        acl, captured = _acl({"zeta": True, "alpha": True, "mu": True}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx()) is False
        message = captured[0].handler_error
        assert message is not None
        parts = message.split("; ")
        assert [p.split(":")[0] for p in parts] == ["alpha", "mu", "zeta"]

    def test_each_part_names_the_key_and_the_reason(self, registered) -> None:
        key = registered("_t_raises12", _Raising())
        acl, captured = _acl({key: True}, effect="allow", default_effect="deny")
        acl.check("caller", "target", _ctx())
        assert captured[0].handler_error == f"{key}: RuntimeError: boom"

    def test_unevaluable_keys_across_several_rules_are_all_reported(self) -> None:
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="allow", conditions={"zeta": True}),
                ACLRule(callers=["*"], targets=["*"], effect="allow", conditions={"alpha": True}),
            ],
            default_effect="deny",
            audit_logger=captured.append,
        )
        assert acl.check("caller", "target", _ctx()) is False
        assert captured[0].handler_error is not None
        assert captured[0].handler_error.startswith("alpha:")
        assert "zeta:" in captured[0].handler_error

    def test_handler_error_does_not_leak_between_checks(self) -> None:
        acl, captured = _acl({"mispelled": True}, effect="allow", default_effect="deny")
        acl.check("caller", "target", _ctx())
        acl.rules  # noqa: B018 - accessor is a pure read, must not disturb state
        acl2, captured2 = _acl(None, effect="allow", default_effect="deny")
        acl2.check("caller", "target", _ctx())
        assert captured[0].handler_error is not None
        assert captured2[0].handler_error is None

    def test_the_suspending_diagnostic_points_at_async_check(self, registered) -> None:
        key = registered("_t_suspends2", _Suspending())
        acl, captured = _acl({key: True}, effect="allow", default_effect="deny")
        acl.check("caller", "target", _ctx())
        assert captured[0].handler_error is not None
        assert "async_check()" in captured[0].handler_error


# ---------------------------------------------------------------------------
# §6.1.1 rule 3 / §6.1.2 rule 2 — warnings
# ---------------------------------------------------------------------------


class TestWarnings:
    def test_evaluation_warns_naming_key_index_and_effect(self, caplog: pytest.LogCaptureFixture) -> None:
        acl, _ = _acl({"mispelled": True}, effect="deny", default_effect="allow")
        caplog.clear()
        with caplog.at_level(logging.WARNING):
            acl.check("caller", "target", _ctx())
        joined = "\n".join(r.message for r in caplog.records)
        assert "mispelled" in joined
        assert "effect=deny" in joined
        assert "rule 0" in joined

    def test_construction_warns_naming_index_key_and_effect(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING):
            ACL(
                rules=[
                    ACLRule(callers=["*"], targets=["*"], effect="allow"),
                    ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"mispelled": True}),
                ],
                default_effect="deny",
            )
        joined = "\n".join(r.message for r in caplog.records)
        assert "ACL rule 1" in joined
        assert "effect=deny" in joined
        assert "mispelled" in joined

    def test_construction_does_not_raise_for_an_unregistered_key(self) -> None:
        """§6.1.2 rule 1: loading MUST NOT fail — handler registration is runtime."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"nope": 1})])
        assert len(acl.rules) == 1

    def test_load_warns_but_succeeds(self, tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
        path = tmp_path / "acl.yaml"
        path.write_text(
            "default_effect: allow\n"
            "rules:\n"
            "  - callers: ['*']\n"
            "    targets: ['*']\n"
            "    effect: deny\n"
            "    conditions:\n"
            "      mispelled: true\n",
            encoding="utf-8",
        )
        with caplog.at_level(logging.WARNING):
            acl = ACL.load(str(path))
        assert len(acl.rules) == 1
        joined = "\n".join(r.message for r in caplog.records)
        assert "ACL rule 0" in joined and "mispelled" in joined

    def test_add_rule_warns_naming_index_zero(self, caplog: pytest.LogCaptureFixture) -> None:
        """§6.1.2 rule 4: runtime insertion is an entry point that MUST be covered."""
        acl = ACL(default_effect="allow")
        caplog.clear()
        with caplog.at_level(logging.WARNING):
            acl.add_rule(callers=["*"], targets=["*"], effect="deny", conditions={"mispelled": True})
        joined = "\n".join(r.message for r in caplog.records)
        assert "ACL rule 0" in joined
        assert "effect=deny" in joined
        assert "mispelled" in joined

    def test_add_rule_does_not_warn_for_a_registered_key(self, caplog: pytest.LogCaptureFixture) -> None:
        acl = ACL(default_effect="allow")
        caplog.clear()
        with caplog.at_level(logging.WARNING):
            acl.add_rule(callers=["*"], targets=["*"], effect="deny", conditions={"roles": ["admin"]})
        assert not [r for r in caplog.records if "precheck" in r.message]

    def test_nested_keys_are_warned_about(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING):
            ACL(
                rules=[
                    ACLRule(
                        callers=["*"],
                        targets=["*"],
                        effect="deny",
                        conditions={"$or": [{"roles": ["a"]}, {"$not": {"deeply_mispelled": True}}]},
                    )
                ]
            )
        joined = "\n".join(r.message for r in caplog.records)
        # §6.1.4: the warning names the nested path, not a bare key.
        assert "$or[1].$not.deeply_mispelled" in joined

    def test_a_conditional_rule_skipped_for_want_of_context_warns(self, caplog: pytest.LogCaptureFixture) -> None:
        """§6.5: not an unevaluable condition, but the consequence must be visible."""
        acl, captured = _acl({"roles": ["admin"]}, effect="deny", default_effect="allow")
        caplog.clear()
        with caplog.at_level(logging.WARNING):
            assert acl.check("caller", "target", None) is True
        joined = "\n".join(r.message for r in caplog.records)
        assert "no context" in joined
        assert "effect=deny" in joined
        # A missing context is NOT unevaluable, so no handler_error is recorded.
        assert captured[0].handler_error is None


# ---------------------------------------------------------------------------
# §6.1.2 rule 3 / §6.1.3 — validate_rules()
# ---------------------------------------------------------------------------


class TestValidateRules:
    def test_empty_when_every_key_resolves(self) -> None:
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"roles": ["a"]}),
                ACLRule(callers=["*"], targets=["*"], effect="allow"),
            ]
        )
        assert acl.validate_rules() == ()

    def test_reports_rule_index_key_and_effect(self) -> None:
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="allow"),
                ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"mispelled": True}),
            ]
        )
        assert acl.validate_rules() == (
            RuleValidationFinding(
                rule_index=1,
                condition_path="mispelled",
                condition_key="mispelled",
                effect="deny",
                sync_resolvable=False,
                async_resolvable=False,
            ),
        )

    def test_reports_keys_nested_in_compound_operators(self) -> None:
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="deny",
                    conditions={"$or": [{"roles": ["a"]}, {"$not": {"nested_typo": True}}]},
                )
            ]
        )
        findings = acl.validate_rules()
        assert [f.condition_key for f in findings] == ["nested_typo"]
        # §6.1.4: the path locates the fault, since a key can occur at several
        # positions in one tree.
        assert [f.condition_path for f in findings] == ["$or[1].$not.nested_typo"]

    def test_the_builtin_compound_operators_are_never_findings(self) -> None:
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"$not": {"roles": ["a"]}})])
        assert acl.validate_rules() == ()

    def test_an_async_only_key_is_reported_with_both_flags(self, registered) -> None:
        """§6.1.3 rule 2: a finding is emitted whenever sync_resolvable is false.

        check() consults only the sync registry, so an async-only key is a
        working condition under async_check() and an UNEVALUABLE one under
        check(). One collapsed boolean cannot express that.
        """
        key = registered("_t_async_only", _Answers(True), asynchronous=True)
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={key: True})])
        findings = acl.validate_rules()
        assert len(findings) == 1
        assert findings[0].sync_resolvable is False
        assert findings[0].async_resolvable is True

    def test_a_sync_only_key_resolves_on_both_paths(self, registered) -> None:
        """The built-ins are sync-only and resolve on async_check via fallback."""
        key = registered("_t_sync_only", _Answers(True))
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={key: True})])
        assert acl.validate_rules() == ()

    def test_an_async_only_key_really_is_unevaluable_on_the_sync_path(self, registered) -> None:
        """The finding is not cosmetic: it predicts a real behavioural difference."""
        key = registered("_t_async_only2", _Answers(True), asynchronous=True)
        acl, _ = _acl({key: True}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx()) is False, "sync check(): unevaluable -> deny rule fires"
        assert asyncio.run(acl.async_check("caller", "target", _ctx())) is False, "async_check(): condition satisfied"

    def test_rules_without_conditions_are_never_reported(self) -> None:
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny")])
        assert acl.validate_rules() == ()

    def test_is_a_pure_read(self) -> None:
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"mispelled": True})],
            audit_logger=captured.append,
        )
        before = acl.rules
        acl.validate_rules()
        assert acl.rules == before
        assert acl.default_effect == "deny"
        assert captured == [], "validate_rules() MUST NOT emit an audit event"

    def test_reflects_a_key_registered_after_construction(self, registered) -> None:
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"_t_late": True})])
        assert len(acl.validate_rules()) == 1
        registered("_t_late", _Answers(True))
        assert acl.validate_rules() == ()

    def test_findings_are_ordered_by_rule_then_lexicographically_by_path(self) -> None:
        """§6.1.2 rule 3 — by path, not by insertion order and not by key."""
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"b_typo": 1, "a_typo": 1}),
                ACLRule(callers=["*"], targets=["*"], effect="allow", conditions={"c_typo": 1}),
            ]
        )
        assert [(f.rule_index, f.condition_path) for f in acl.validate_rules()] == [
            (0, "a_typo"),
            (0, "b_typo"),
            (1, "c_typo"),
        ]

    def test_reports_a_malformed_conditions_block(self) -> None:
        """The gap the narrower name hid: `$` is a fault the validator must see."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions="oops")])  # type: ignore[arg-type]
        findings = acl.validate_rules()
        assert [(f.condition_path, f.effect) for f in findings] == [("$", "deny")]

    def test_reports_a_malformed_compound_operand(self) -> None:
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"$or": "not-a-list"}),
                ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"$not": 3}),
            ]
        )
        assert [(f.rule_index, f.condition_path) for f in acl.validate_rules()] == [(0, "$or"), (1, "$not")]

    def test_reports_malformed_callers_and_targets(self) -> None:
        """§6.1.4.1 — the reason the method could not stay named validate_conditions."""
        acl = ACL(rules=[ACLRule(callers="admin.*", targets=5, effect="allow")])  # type: ignore[arg-type]
        findings = acl.validate_rules()
        assert [f.condition_path for f in findings] == ["callers", "targets"]
        assert all(f.sync_resolvable is False and f.async_resolvable is False for f in findings)

    def test_a_well_formed_rule_with_no_conditions_is_never_reported(self) -> None:
        acl = ACL(rules=[ACLRule(callers=["a"], targets=["b"], effect="allow")])
        assert acl.validate_rules() == ()


# ---------------------------------------------------------------------------
# §6.8 — ACL introspection
# ---------------------------------------------------------------------------


class TestIntrospection:
    def test_default_effect_is_readable(self) -> None:
        assert ACL(default_effect="allow").default_effect == "allow"
        assert ACL(default_effect="deny").default_effect == "deny"

    def test_default_effect_matches_what_check_applies(self) -> None:
        for effect, expected in (("allow", True), ("deny", False)):
            acl = ACL(default_effect=effect)
            assert acl.default_effect == effect
            assert acl.check("caller", "target") is expected

    def test_rules_are_returned_in_definition_order(self) -> None:
        first = ACLRule(callers=["a"], targets=["*"], effect="allow", description="first")
        second = ACLRule(callers=["b"], targets=["*"], effect="deny", description="second")
        acl = ACL(rules=[first, second])
        assert [r.description for r in acl.rules] == ["first", "second"]

    def test_rules_does_not_hand_out_a_mutable_reference(self) -> None:
        """§6.8 rule 3: the caller must not be able to mutate the ACL's own list."""
        acl = ACL(rules=[ACLRule(callers=["a"], targets=["*"], effect="allow")])
        snapshot = acl.rules
        assert isinstance(snapshot, tuple)
        acl.add_rule(callers=["b"], targets=["*"], effect="deny")
        assert len(snapshot) == 1, "an earlier snapshot must not observe later mutations"
        assert len(acl.rules) == 2

    def test_accessors_reflect_a_reload(self, tmp_path: Path) -> None:
        """§6.8 rule 4: both read the live object, never a cached parse."""
        path = tmp_path / "acl.yaml"
        path.write_text(
            "default_effect: deny\nrules:\n  - callers: ['a']\n    targets: ['*']\n    effect: allow\n",
            encoding="utf-8",
        )
        acl = ACL.load(str(path))
        assert acl.default_effect == "deny"
        assert len(acl.rules) == 1

        path.write_text(
            "default_effect: allow\n"
            "rules:\n"
            "  - callers: ['a']\n"
            "    targets: ['*']\n"
            "    effect: allow\n"
            "  - callers: ['b']\n"
            "    targets: ['*']\n"
            "    effect: deny\n",
            encoding="utf-8",
        )
        acl.reload()
        assert acl.default_effect == "allow"
        assert len(acl.rules) == 2

    def test_accessors_are_pure_reads(self) -> None:
        captured: list[AuditEntry] = []
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")], audit_logger=captured.append)
        acl.default_effect  # noqa: B018
        acl.rules  # noqa: B018
        assert captured == [], "an accessor MUST NOT emit an audit event"

    def test_accessors_are_not_underscore_prefixed(self) -> None:
        """§6.8 rule 1 + api-surface-conventions §3: a public path, not a private name."""
        assert isinstance(type(ACL).__getattribute__(ACL, "default_effect"), property)
        assert isinstance(type(ACL).__getattribute__(ACL, "rules"), property)


# ---------------------------------------------------------------------------
# ConditionOutcome as a handler return value
# ---------------------------------------------------------------------------


class TestConditionOutcomeReturnValue:
    def test_a_handler_may_report_unevaluable_itself(self, registered) -> None:
        class _SelfReporting:
            def evaluate(self, value: Any, context: Context) -> ConditionOutcome:
                return ConditionOutcome.UNEVALUABLE

        key = registered("_t_self_unevaluable", _SelfReporting())
        acl, _ = _acl({key: True}, effect="deny", default_effect="allow")
        assert acl.check("caller", "target", _ctx()) is False

    def test_a_handler_returning_the_satisfied_member_is_satisfied(self, registered) -> None:
        class _SelfReporting:
            def evaluate(self, value: Any, context: Context) -> ConditionOutcome:
                return ConditionOutcome.SATISFIED

        key = registered("_t_self_satisfied", _SelfReporting())
        acl, _ = _acl({key: True}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx()) is True

    def test_a_plain_bool_keeps_its_historical_meaning(self, registered) -> None:
        key = registered("_t_plain_bool", _Answers(True))
        acl, _ = _acl({key: True}, effect="allow", default_effect="deny")
        assert acl.check("caller", "target", _ctx()) is True


# ---------------------------------------------------------------------------
# A non-mapping `conditions` value
# ---------------------------------------------------------------------------


_NON_MAPPING_CONDITIONS = [
    pytest.param("oops", "str", id="str"),
    pytest.param(3, "int", id="int"),
    pytest.param(0.5, "float", id="float"),
    pytest.param(True, "bool", id="bool"),
    pytest.param(["roles"], "list", id="list"),
    pytest.param([], "list", id="empty-list"),
    pytest.param((), "tuple", id="tuple"),
    pytest.param({"a"}, "set", id="set"),
]


class TestMalformedConditionsValue:
    """A `conditions` value that is not a mapping is UNEVALUABLE, not an exception.

    ``ACLRule.conditions`` is annotated ``dict[str, Any] | None``, but the
    annotation binds nobody: ``ACL(rules=[...])`` and ``add_rule()`` build rules
    programmatically and never reach ``ACL.load``'s parser, so a scalar or a list
    arrives at the evaluator intact. Iterating it raised ``AttributeError``
    straight out of ``check()`` — which the ``ACL.check`` contract forbids:
    "check MUST NOT raise to indicate a deny; it MUST return false", with raising
    reserved for unrecoverable internal failures. A malformed rule supplied by
    the host is not one of those.

    UNSATISFIED would not do either: it would let a ``deny`` rule fall through to
    the next rule and then to ``default_effect``, which is the bypass §6.1.1
    exists to close.
    """

    @pytest.mark.parametrize(("conditions", "type_name"), _NON_MAPPING_CONDITIONS)
    def test_deny_rule_takes_effect_instead_of_raising(self, conditions: Any, type_name: str) -> None:
        acl, captured = _acl(conditions, effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].decision == "deny"
        assert captured[0].reason == "rule_match"
        assert captured[0].handler_error is not None
        assert type_name in captured[0].handler_error

    @pytest.mark.parametrize(("conditions", "type_name"), _NON_MAPPING_CONDITIONS)
    async def test_deny_rule_takes_effect_on_the_async_path_too(self, conditions: Any, type_name: str) -> None:
        acl, captured = _acl(conditions, effect="deny", default_effect="allow")
        assert await acl.async_check("user", "service.op", _ctx()) is False
        assert captured[0].decision == "deny"
        assert captured[0].handler_error is not None
        assert type_name in captured[0].handler_error

    @pytest.mark.parametrize(("conditions", "type_name"), _NON_MAPPING_CONDITIONS)
    def test_allow_rule_does_not_grant(self, conditions: Any, type_name: str) -> None:
        acl, captured = _acl(conditions, effect="allow", default_effect="deny")
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].reason == "default_effect"
        assert captured[0].handler_error is not None

    def test_the_coordinator_reproduction(self) -> None:
        """The exact probe that escaped as ``AttributeError: 'str' object has no attribute 'items'``."""
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions="oops")],  # type: ignore[arg-type]
            default_effect="allow",
        )
        assert acl.check("user", "service.op", _ctx(roles=["dev"])) is False

    def test_the_diagnostic_uses_the_jsonpath_root(self) -> None:
        """No real condition key exists to name, so the root path `$` stands in.

        §6.1.4 settles on ``$`` — the JSONPath root — which keeps the notation
        consistent with ``$or[1].$not.k``, where no root token otherwise appears.
        """
        acl, captured = _acl("oops", effect="deny", default_effect="allow")
        acl.check("user", "service.op", _ctx())
        assert captured[0].handler_error == "$: ACL conditions must be a mapping, got str"

    def test_it_warns(self, caplog: pytest.LogCaptureFixture) -> None:
        acl, _ = _acl(["roles"], effect="deny", default_effect="allow")
        caplog.clear()
        with caplog.at_level(logging.WARNING):
            acl.check("user", "service.op", _ctx())
        joined = "\n".join(r.message for r in caplog.records)
        assert "must be a mapping" in joined
        assert "effect=deny" in joined

    def test_add_rule_is_covered_too(self) -> None:
        """The other entry point that bypasses the parser."""
        acl = ACL(default_effect="allow")
        acl.add_rule(callers=["*"], targets=["*"], effect="deny", conditions="oops")  # type: ignore[arg-type]
        assert acl.check("user", "service.op", _ctx()) is False

    def test_none_and_empty_mapping_are_unaffected(self) -> None:
        """``None`` means "no conditions"; ``{}`` means "vacuously satisfied"."""
        for conditions in (None, {}):
            acl, captured = _acl(conditions, effect="deny", default_effect="allow")
            assert acl.check("user", "service.op", _ctx()) is False, "the rule matches on patterns alone"
            assert captured[0].handler_error is None

    @pytest.mark.parametrize(
        ("conditions", "path"),
        [
            ({"$or": "not-a-list"}, "$or"),
            ({"$or": 5}, "$or"),
            ({"$not": 3}, "$not"),
            ({"$not": "x"}, "$not"),
        ],
        ids=["or-str", "or-int", "not-int", "not-str"],
    )
    def test_a_malformed_compound_operand_is_unevaluable(self, conditions: dict[str, Any], path: str) -> None:
        """§6.1.1 case 4, INVERTED at spec v1.25.0.

        Through v1.24.0 all three SDKs classified this UNSATISFIED, because
        §6.1.1 enumerated exactly three unevaluable situations and a handler
        handed a malformed value does run to completion. The consequence was
        that a ``deny`` rule carrying ``$or: "typo"`` stayed inert — the v1.22.0
        defect reached through a second door. "Unevaluable" is now a principle:
        the implementation cannot answer the condition **as written**.
        """
        acl, captured = _acl(conditions, effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", _ctx()) is False, "the deny rule must take effect"
        assert captured[0].handler_error is not None
        assert captured[0].handler_error.startswith(f"{path}: ")

        acl2, captured2 = _acl(conditions, effect="deny", default_effect="allow")
        assert asyncio.run(acl2.async_check("user", "service.op", _ctx())) is False
        assert captured2[0].handler_error is not None

    @pytest.mark.parametrize(
        ("conditions", "path"),
        [
            ({"$or": "not-a-list"}, "$or"),
            ({"$not": 3}, "$not"),
        ],
        ids=["or", "not"],
    )
    def test_a_malformed_compound_operand_on_an_allow_rule_does_not_grant(
        self, conditions: dict[str, Any], path: str
    ) -> None:
        acl, captured = _acl(conditions, effect="allow", default_effect="deny")
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].reason == "default_effect"
        assert captured[0].handler_error is not None

    def test_a_non_mapping_or_element_is_unevaluable(self) -> None:
        """Not a fixture case, and not one the spec names explicitly.

        §6.1.1 case 4 lists "`$or` whose value is not a list" and "`$not` whose
        value is not an object", but the section is explicit that the list is
        non-exhaustive and that an implementation meeting an unlisted case MUST
        classify it by the principle rather than defaulting it to UNSATISFIED.
        An `$or` element that is not a condition object asks no question, so it
        is unevaluable — otherwise `$or: ["typo"]` on a `deny` rule is inert,
        which is the same failure one level down. Flagged for a fixture case so
        all three SDKs agree.
        """
        acl, captured = _acl({"$or": ["not-a-dict"]}, effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None
        assert captured[0].handler_error.startswith("$or[0]: ")

    def test_a_well_formed_or_still_composes_normally(self) -> None:
        """The guard must not swallow a legitimately unsatisfied `$or`."""
        acl, captured = _acl(
            {"$or": [{"roles": ["admin"]}, {"identity_types": ["service"]}]},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("user", "service.op", _ctx(roles=["viewer"])) is True
        assert captured[0].handler_error is None


# ---------------------------------------------------------------------------
# §6.1.4 — the structural and registry precheck
# ---------------------------------------------------------------------------


class TestPrecheckOrdering:
    """The precheck is context-free and runs BEFORE §6.5's no-context check.

    That ordering is the whole design. It closes the bypass where
    ``conditions: {mispelled: true}`` on a `deny` rule passed traffic simply
    because the caller carried no identity — measured in all three SDKs — while
    leaving §6.5 itself untouched for rules that are merely unanswerable *by
    this caller*.
    """

    def test_a_misspelled_key_denies_even_without_a_context(self) -> None:
        acl, captured = _acl({"mispelled": True}, effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", None) is False
        assert captured[0].decision == "deny"
        assert captured[0].handler_error is not None
        assert captured[0].handler_error.startswith("mispelled: ")

    async def test_a_misspelled_key_denies_without_a_context_on_the_async_path(self) -> None:
        acl, captured = _acl({"mispelled": True}, effect="deny", default_effect="allow")
        assert await acl.async_check("user", "service.op", None) is False
        assert captured[0].handler_error is not None

    def test_a_malformed_conditions_block_denies_even_without_a_context(self) -> None:
        acl, captured = _acl("oops", effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", None) is False
        assert captured[0].handler_error == "$: ACL conditions must be a mapping, got str"

    def test_a_well_formed_conditional_rule_still_takes_the_no_context_path(self) -> None:
        """§6.1.4 rule 2 — the control, and the line the design turns on.

        ``roles`` is registered and well formed, so the rule PASSES the precheck
        and then finds no context: it does not match, the call is allowed, and
        no ``handler_error`` is recorded. A registered, context-dependent
        condition is NOT unevaluable merely because this caller sent nothing.
        """
        acl, captured = _acl({"roles": ["admin"]}, effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", None) is True
        assert captured[0].reason == "default_effect"
        assert captured[0].handler_error is None, "a question this caller did not answer is not a fault"

    async def test_the_control_holds_on_the_async_path(self) -> None:
        acl, captured = _acl({"roles": ["admin"]}, effect="deny", default_effect="allow")
        assert await acl.async_check("user", "service.op", None) is True
        assert captured[0].handler_error is None

    def test_a_non_matching_rule_is_never_prechecked_for_conditions(self) -> None:
        """A rule whose patterns do not match does not apply, so its typo is not this call's problem.

        Prechecking conditions on every rule regardless of pattern match would
        make one misspelled key in one unrelated `deny` rule deny the entire
        registry. §6.3's loop only evaluates conditions inside the
        caller-and-target-matched branch, and §6.1.4 does not restructure it.
        """
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers=["someone.else"], targets=["*"], effect="deny", conditions={"mispelled": True})],
            default_effect="allow",
            audit_logger=captured.append,
        )
        assert acl.check("user", "service.op", _ctx()) is True
        assert captured[0].handler_error is None

    def test_the_precheck_does_not_short_circuit(self) -> None:
        """§6.1.4 rule 3 — completeness is what makes handler_error deterministic."""
        acl, captured = _acl(
            {"zeta_typo": True, "alpha_typo": True, "mu_typo": True},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None
        assert _reported_paths(captured[0].handler_error) == ["alpha_typo", "mu_typo", "zeta_typo"]

    def test_two_faults_in_one_or_report_both_ordered_by_path(self) -> None:
        """The fixture case that pins diagnostics across languages."""
        acl, captured = _acl(
            {"$or": [{"b_typo": True}, {"a_typo": True}]},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None
        assert _reported_paths(captured[0].handler_error) == ["$or[0].b_typo", "$or[1].a_typo"]

    def test_a_repeated_key_at_two_positions_gets_two_paths(self) -> None:
        """Why ordering is by path and not by key: one key, two positions."""
        acl, captured = _acl(
            {"$or": [{"dup_typo": 1}, {"$not": {"dup_typo": 2}}]},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None
        assert _reported_paths(captured[0].handler_error) == ["$or[0].dup_typo", "$or[1].$not.dup_typo"]

    def test_an_execution_origin_fault_still_carries_its_nested_path(self, registered) -> None:
        key = registered("_t_nested_raise", _Raising())
        acl, captured = _acl({"$or": [{key: True}]}, effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None
        assert captured[0].handler_error.startswith(f"$or[0].{key}: ")


def _reported_paths(handler_error: str) -> list[str]:
    """Split a ``handler_error`` into the condition paths it names, in order."""
    return [part.split(": ", 1)[0] for part in handler_error.split("; ")]


# ---------------------------------------------------------------------------
# §6.1.4.1 — malformed `callers` / `targets` (apcore#106)
# ---------------------------------------------------------------------------


class TestMalformedPatternFields:
    """A `callers`/`targets` that is not a list of strings is unevaluable.

    The failure this closes fails **open**. A bare string is iterable, so
    ``callers: "admin.*"`` written where ``callers: ["admin.*"]`` was meant is
    read character by character, and ``*`` is a valid pattern matching
    everything — so an `allow` rule carrying that typo granted access to every
    caller. Whether a given typo was dangerous depended only on whether the
    mistyped string happened to contain a `*`.
    """

    def test_string_callers_on_an_allow_rule_does_not_grant(self) -> None:
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers="admin.*", targets=["*"], effect="allow")],  # type: ignore[arg-type]
            default_effect="deny",
            audit_logger=captured.append,
        )
        assert acl.check("attacker", "service.op", _ctx()) is False, "the pre-#106 fail-open"
        assert captured[0].handler_error is not None
        assert _reported_paths(captured[0].handler_error) == ["callers"]

    def test_string_callers_on_a_deny_rule_takes_effect(self) -> None:
        acl = ACL(
            rules=[ACLRule(callers="admin.*", targets=["*"], effect="deny")],  # type: ignore[arg-type]
            default_effect="allow",
        )
        assert acl.check("attacker", "service.op", _ctx()) is False

    @pytest.mark.parametrize(
        "value",
        [5, None, {"a": 1}, "admin.*", ["ok", 5], (("a",)), 0.5, True],
        ids=["int", "none", "dict", "str", "list-with-int", "tuple", "float", "bool"],
    )
    def test_no_shape_escapes_check_as_an_exception(self, value: Any) -> None:
        """`check` MUST NOT raise; `callers=5` used to raise TypeError, `{"a":1}` KeyError."""
        for field in ("callers", "targets"):
            kwargs: dict[str, Any] = {"callers": ["*"], "targets": ["*"], "effect": "deny"}
            kwargs[field] = value
            acl = ACL(rules=[ACLRule(**kwargs)], default_effect="allow")
            assert acl.check("attacker", "service.op", _ctx()) is False
            assert asyncio.run(acl.async_check("attacker", "service.op", _ctx())) is False

    def test_both_fields_are_reported_without_short_circuiting(self) -> None:
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers="a", targets=5, effect="deny")],  # type: ignore[arg-type]
            default_effect="allow",
            audit_logger=captured.append,
        )
        assert acl.check("attacker", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None
        assert _reported_paths(captured[0].handler_error) == ["callers", "targets"]

    def test_an_empty_list_cannot_be_constructed(self) -> None:
        """REVERSED in spec v1.31.0 (§6.2.1, apcore#112) — replaces one test with two.

        Through spec v1.30.0 this file carried
        ``test_an_empty_list_is_well_formed_and_simply_never_matches``, whose
        docstring read *"§6.5 keeps this a plain non-match — it is a valid list
        of strings"* and which asserted ``check(...) is True`` with
        ``handler_error is None`` on a ``deny`` rule under
        ``default_effect: allow``. That reading was decided during apcore#106's
        implementation and enshrined here — and it is precisely the fail-open
        apcore#112 was filed about: the ``deny`` rule an operator wrote loaded
        without error, ``validate_rules()`` called it clean, and the call it
        named was permitted. §6.5's edge-case table required it, and that row is
        **replaced rather than reinterpreted**. §6.2.1 now closes the arity at
        every door, so the rule cannot be built at all.
        """
        with pytest.raises(ACLRuleError, match=r"ACLRule has an invalid 'callers'"):
            ACLRule(callers=[], targets=["*"], effect="deny")

    def test_an_empty_list_assigned_after_construction_denies(self) -> None:
        """The other half of the replacement: the one route no door covers.

        ``ACLRule`` is a non-frozen dataclass, so ``rule.callers = []`` reaches
        the evaluator whatever the constructors check — and unlike an
        unrecognised ``effect``, which §6.1.5 disposes of by observing the value
        is never read again, a mutated pattern array *is* read by the matcher.
        §6.1.4.1 therefore classifies it: unevaluable, so the ``deny`` rule
        takes effect, ``handler_error`` names the field and a warning is
        emitted. Same rule, same probe, opposite answer to the test this
        replaces.
        """
        captured: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        acl = ACL(rules=[rule], default_effect="allow", audit_logger=captured.append)
        rule.callers = []
        assert acl.check("attacker", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None
        assert _reported_paths(captured[0].handler_error) == ["callers"]

    def test_a_malformed_pattern_field_is_caught_before_the_patterns_are_read(self) -> None:
        """`callers: "*"` matched everything by iterating characters; now it is a fault."""
        acl = ACL(
            rules=[ACLRule(callers="*", targets=["*"], effect="allow")],  # type: ignore[arg-type]
            default_effect="deny",
        )
        assert acl.check("attacker", "service.op", _ctx()) is False


# ---------------------------------------------------------------------------
# ACL.load rejects a non-mapping `conditions` (parity with apcore-typescript)
# ---------------------------------------------------------------------------


class TestLoadRejectsMalformedConditions:
    def test_load_raises_for_a_non_mapping_conditions(self, tmp_path: Path) -> None:
        path = tmp_path / "acl.yaml"
        path.write_text(
            "default_effect: allow\n"
            "rules:\n"
            "  - callers: ['*']\n"
            "    targets: ['*']\n"
            "    effect: deny\n"
            "    conditions: oops\n",
            encoding="utf-8",
        )
        with pytest.raises(ACLRuleError, match="'conditions' must be a mapping"):
            ACL.load(str(path))

    def test_load_still_accepts_an_absent_conditions(self, tmp_path: Path) -> None:
        path = tmp_path / "acl.yaml"
        path.write_text(
            "default_effect: allow\nrules:\n  - callers: ['*']\n    targets: ['*']\n    effect: deny\n",
            encoding="utf-8",
        )
        assert ACL.load(str(path)).rules[0].conditions is None

    def test_load_still_accepts_a_mapping_conditions(self, tmp_path: Path) -> None:
        path = tmp_path / "acl.yaml"
        path.write_text(
            "default_effect: allow\n"
            "rules:\n"
            "  - callers: ['*']\n"
            "    targets: ['*']\n"
            "    effect: deny\n"
            "    conditions:\n"
            "      roles: [admin]\n",
            encoding="utf-8",
        )
        assert ACL.load(str(path)).rules[0].conditions == {"roles": ["admin"]}


# ---------------------------------------------------------------------------
# §6.1.4 rule 4 — the precheck must not widen a rule's reach
# ---------------------------------------------------------------------------


class TestPrecheckDoesNotWidenReach:
    """A rule a well-formed pattern field excludes takes no part in the decision.

    Without this, one misspelled key in a narrowly scoped rule would decide
    calls that rule was never written about — ``callers: ["api.*"]`` with
    ``conditions: {mispelled: true}`` and ``effect: deny`` denying a ``worker.*``
    caller — which breaks first-match-wins. The fault is still real;
    :meth:`ACL.validate_rules` looks at every rule and no call, and is where a
    scoped rule's typo is meant to surface.
    """

    @staticmethod
    def _scoped_deny() -> tuple[ACL, list[AuditEntry]]:
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers=["api.*"], targets=["*"], effect="deny", conditions={"mispelled": True})],
            default_effect="allow",
            audit_logger=captured.append,
        )
        return acl, captured

    def test_a_caller_the_rule_excludes_is_unaffected(self) -> None:
        acl, captured = self._scoped_deny()
        assert acl.check("worker.job", "service.op", _ctx()) is True
        assert captured[0].handler_error is None, "the excluded rule's fault must not be consulted"
        assert captured[0].reason == "default_effect"

    def test_a_target_the_rule_excludes_is_unaffected(self) -> None:
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["billing.*"], effect="deny", conditions={"mispelled": True})],
            default_effect="allow",
            audit_logger=captured.append,
        )
        assert acl.check("api.gateway", "service.op", _ctx()) is True
        assert captured[0].handler_error is None

    def test_a_caller_the_rule_covers_is_denied(self) -> None:
        """The other half: in scope, the fault decides."""
        acl, captured = self._scoped_deny()
        assert acl.check("api.gateway", "service.op", _ctx()) is False
        assert captured[0].handler_error is not None

    async def test_the_ruling_holds_on_the_async_path(self) -> None:
        acl, captured = self._scoped_deny()
        assert await acl.async_check("worker.job", "service.op", _ctx()) is True
        assert captured[0].handler_error is None

    def test_first_match_wins_is_preserved(self) -> None:
        """A scoped, faulty rule must not pre-empt a later rule that does apply."""
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[
                ACLRule(callers=["api.*"], targets=["*"], effect="deny", conditions={"mispelled": True}),
                ACLRule(callers=["worker.*"], targets=["*"], effect="allow", description="worker lane"),
            ],
            default_effect="deny",
            audit_logger=captured.append,
        )
        assert acl.check("worker.job", "service.op", _ctx()) is True
        assert captured[0].matched_rule == "worker lane"
        assert captured[0].handler_error is None

    def test_validate_rules_still_reports_the_scoped_fault(self) -> None:
        """The fault does not vanish — it surfaces where it belongs."""
        acl, _ = self._scoped_deny()
        assert [(f.rule_index, f.condition_path) for f in acl.validate_rules()] == [(0, "mispelled")]

    def test_a_malformed_pattern_field_still_makes_the_rule_unevaluable(self) -> None:
        """Rule 4(b): a malformed field is not an exclusion — the scope is unknowable."""
        acl = ACL(
            rules=[ACLRule(callers="api.*", targets=["*"], effect="deny")],  # type: ignore[arg-type]
            default_effect="allow",
        )
        assert acl.check("worker.job", "service.op", _ctx()) is False


class TestKeylessFaultsReportNoKey:
    """§6.1.4: a fault not attached to a condition key reports ``condition_key`` as None.

    The two flags mean "can this fault be resolved by evaluating on that path",
    not "is the key present in that registry" — a structural fault resolves on
    neither. Read as a registry lookup they would report a malformed ``$or``
    *value* as resolvable on both paths, because ``$or`` itself has a handler.
    """

    @pytest.mark.parametrize(
        ("rule", "path"),
        [
            (ACLRule(callers="a", targets=["*"], effect="deny"), "callers"),  # type: ignore[arg-type]
            (ACLRule(callers=["*"], targets=5, effect="deny"), "targets"),  # type: ignore[arg-type]
            (ACLRule(callers=["*"], targets=["*"], effect="deny", conditions="oops"), "$"),  # type: ignore[arg-type]
            (ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"$or": ["nope"]}), "$or[0]"),
        ],
        ids=["callers", "targets", "non-mapping-conditions", "or-element"],
    )
    def test_a_keyless_fault_reports_a_null_key_and_both_flags_false(self, rule: ACLRule, path: str) -> None:
        finding = ACL(rules=[rule]).validate_rules()[0]
        assert finding.condition_path == path
        assert finding.condition_key is None
        assert finding.sync_resolvable is False
        assert finding.async_resolvable is False

    @pytest.mark.parametrize(
        ("conditions", "key"),
        [({"$or": "not-a-list"}, "$or"), ({"$not": 3}, "$not")],
        ids=["or-value", "not-value"],
    )
    def test_a_malformed_operator_value_keeps_its_key_but_resolves_on_neither_path(
        self, conditions: dict[str, Any], key: str
    ) -> None:
        """The fault IS attached to `$or` / `$not`, so the key is reported.

        Both flags stay false all the same: `$or` has a registered handler, so a
        registry lookup would wrongly call this resolvable on both paths.
        """
        finding = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions=conditions)])
        found = finding.validate_rules()[0]
        assert found.condition_key == key
        assert found.sync_resolvable is False
        assert found.async_resolvable is False

    def test_an_unresolvable_key_still_reports_its_key(self) -> None:
        finding = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={"typo": 1})])
        found = finding.validate_rules()[0]
        assert found.condition_path == "typo"
        assert found.condition_key == "typo"

    def test_an_async_only_key_reports_the_asymmetry_not_a_registry_lookup(self, registered) -> None:
        key = registered("_t_flags_async_only", _Answers(True), asynchronous=True)
        found = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny", conditions={key: 1})]).validate_rules()
        assert (found[0].sync_resolvable, found[0].async_resolvable) == (False, True)


# ---------------------------------------------------------------------------
# §6.1.4 rule 5 — the precheck GATES; §6.1.1's table governs what was evaluated
# ---------------------------------------------------------------------------


class TestGatingVersusComposition:
    """The split is precheck-vs-execution, not structural-vs-everything.

    §6.1.4 rule 1 says a rule that fails the precheck is unevaluable; §6.1.1's
    `$or` table says an outright "yes" wins even beside an unevaluable sibling.
    They only appear to conflict: the table governs conditions that were
    actually **evaluated**, and a precheck fault means the rule never got that
    far. An implementation that gates on execution-origin faults too would deny
    where the table says grant — the second test below is the one that catches it.
    """

    def test_a_structural_fault_gates_even_when_an_or_sibling_is_satisfied(self) -> None:
        """The caller HAS `dev`, so composition alone would say SATISFIED."""
        acl, captured = _acl(
            {"$or": [{"unregistered_key": True}, {"roles": ["dev"]}]},
            effect="deny",
            default_effect="allow",
        )
        assert acl.check("user", "service.op", _ctx(roles=["dev"])) is False, "the precheck gates the rule"
        assert _reported_paths(captured[0].handler_error or "") == ["$or[0].unregistered_key"]

    def test_an_execution_fault_does_not_gate_when_an_or_sibling_is_satisfied(self, registered) -> None:
        """The mirror image, and the one that catches over-gating.

        The key is REGISTERED, so the precheck passes; it only throws when run,
        which is execution-origin. §6.1.1's `$or` table therefore applies and the
        satisfied `roles` branch wins: the `$or` is SATISFIED, the `allow` rule
        matches, and the call is granted — with a `handler_error` recording the
        branch that did throw.
        """
        key = registered("_t_gate_split_raise", _Raising())
        acl, captured = _acl(
            {"$or": [{key: True}, {"roles": ["dev"]}]},
            effect="allow",
            default_effect="deny",
        )
        assert acl.check("user", "service.op", _ctx(roles=["dev"])) is True, "an execution fault must NOT gate"
        assert captured[0].handler_error is not None
        assert captured[0].handler_error.startswith(f"$or[0].{key}: ")

    async def test_the_split_holds_on_the_async_path(self, registered) -> None:
        key = registered("_t_gate_split_raise_async", _Raising())
        acl, _ = _acl({"$or": [{key: True}, {"roles": ["dev"]}]}, effect="allow", default_effect="deny")
        assert await acl.async_check("user", "service.op", _ctx(roles=["dev"])) is True

        acl2, _ = _acl({"$or": [{"nope": 1}, {"roles": ["dev"]}]}, effect="allow", default_effect="deny")
        assert await acl2.async_check("user", "service.op", _ctx(roles=["dev"])) is False

    def test_an_execution_fault_still_propagates_when_no_sibling_is_satisfied(self, registered) -> None:
        """Not gating is not the same as ignoring — the table still returns UNEVALUABLE."""
        key = registered("_t_gate_split_raise2", _Raising())
        acl, _ = _acl({"$or": [{key: True}, {"roles": ["admin"]}]}, effect="deny", default_effect="allow")
        assert acl.check("user", "service.op", _ctx(roles=["dev"])) is False


# ---------------------------------------------------------------------------
# §6.1.1 rule 5 — a pending approval requirement (spec v1.29.0, apcore#109)
# ---------------------------------------------------------------------------


def _approval_acl(
    *rules: ACLRule,
    default_effect: str = "deny",
) -> tuple[ACL, list[AuditEntry]]:
    captured: list[AuditEntry] = []
    return ACL(rules=list(rules), default_effect=default_effect, audit_logger=captured.append), captured


def _gate(**overrides: Any) -> ACLRule:
    """The gate: ``allow`` + ``approval: required``, unevaluable as written.

    ``mispelled_key`` is registered nowhere, so the rule reaches §6.1.1 by the
    §6.1.4 precheck — the same door a misspelled ``arguments`` predicate, an
    unregistered condition key and a raising handler all arrive through.
    """
    fields: dict[str, Any] = {
        "callers": ["*"],
        "targets": ["cli.git_push"],
        "effect": "allow",
        "approval": "required",
        "conditions": {"mispelled_key": True},
    }
    fields.update(overrides)
    return ACLRule(**fields)


def _broad_allow(target: str = "cli.git_push") -> ACLRule:
    return ACLRule(callers=["*"], targets=[target], effect="allow")


class TestPendingApprovalRequirement:
    """An unevaluable ``allow`` rule does not take its approval with it.

    §6.1.1 was written when a rule carried one axis, and "MUST NOT grant" was
    then complete: the rule steps aside, and stepping aside was harmless because
    whatever granted next also said ``allow``. §6.1.6 gave rules a second axis,
    and stepping aside began discarding it — on exactly the shape §6.1.7 exists
    for, a narrow approval rule ahead of a broad allow. ``git push --force`` was
    authorized with ``approval_required: False`` and ``matched_rule_index``
    naming a rule that never mentioned approval.
    """

    def test_a_later_allow_rule_carries_the_requirement(self) -> None:
        """The driving shape: the gate is unevaluable, rule 1 grants, the human stays."""
        acl, captured = _approval_acl(_gate(), _broad_allow())
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert decision.access == "allow"
        assert decision.approval_required is True, "the requirement was discarded with the rule"
        # The index names the rule that actually DECIDED, not the one that
        # raised the requirement — they are different rules, deliberately.
        assert decision.matched_rule_index == 1
        assert captured[0].approval_required is True, "the audit entry must carry the FINAL value"
        assert captured[0].handler_error is not None, "rule 2 is untouched by rule 5"

    def test_default_effect_allow_carries_it_with_no_second_rule(self) -> None:
        """The boundary a "a later rule grants" reading misses: there is no later rule.

        Pins the combination §6.9 row 2 newly makes legal — ``approval_required:
        True`` with ``matched_rule_index: None``.
        """
        acl, captured = _approval_acl(_gate(), default_effect="allow")
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required) == ("allow", True)
        assert decision.matched_rule_index is None
        assert decision.reason == "default_effect"
        assert captured[0].approval_required is True

    def test_a_denial_clears_it(self) -> None:
        """A denial clears it — "denied *and* put it to a human" is the state §6.1.6

        rejects on a rule.

        The pending requirement must not reconstruct it from two rules.
        """
        deny = ACLRule(callers=["*"], targets=["cli.git_push"], effect="deny")
        acl, captured = _approval_acl(_gate(), deny, default_effect="allow")
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required) == ("deny", False)
        assert decision.matched_rule_index == 1, "the index names the rule that decided"
        assert captured[0].approval_required is False

    def test_the_default_deny_clears_it_too(self) -> None:
        acl, _ = _approval_acl(_gate(), default_effect="deny")
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required) == ("deny", False)

    def test_an_out_of_scope_rule_raises_nothing(self) -> None:
        """Rule 5's containment, and the assertion that keeps the fix from over-reaching.

        The gate is written about ``cli.deploy``. Its conditions are never
        consulted (§6.1.4 rule 4) and it must not attach a human to a call it was
        never written about. An implementation that records the requirement
        before matching patterns passes every other test here and fails this one.
        """
        acl, captured = _approval_acl(_gate(targets=["cli.deploy"]), _broad_allow())
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required) == ("allow", False)
        assert decision.matched_rule_index == 1
        assert captured[0].handler_error is None, "an out-of-scope rule's faults are not this call's"

    def test_an_out_of_scope_caller_raises_nothing(self) -> None:
        """The same containment on the other pattern field."""
        acl, _ = _approval_acl(_gate(callers=["worker.*"]), _broad_allow())
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required) == ("allow", False)

    def test_a_malformed_pattern_field_does_raise_it(self) -> None:
        """The one point where a requirement attaches without a demonstrated match.

        ``callers: "*"`` where ``callers: ["*"]`` was meant is unevaluable before
        any pattern is read (§6.1.4.1), so the rule's SCOPE cannot be read and it
        cannot be shown not to apply here — the same posture the field already
        produces under ``deny``, where an unreadable scope denies every call.
        """
        acl, captured = _approval_acl(_gate(callers="*", conditions=None), _broad_allow())
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required) == ("allow", True)
        assert decision.matched_rule_index == 1
        assert captured[0].handler_error is not None

    def test_an_unevaluable_rule_without_approval_raises_nothing(self) -> None:
        """Rule 5 is about the second axis only; rule 1 is unchanged for the first."""
        acl, _ = _approval_acl(_gate(approval="not_required"), _broad_allow())
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required) == ("allow", False)

    def test_a_satisfied_gate_is_not_the_pending_path(self) -> None:
        """The ordinary v1.28.0 route still reports the rule that matched."""
        acl, _ = _approval_acl(_gate(conditions={"roles": ["dev"]}), _broad_allow())
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx(roles=["dev"]))
        assert (decision.access, decision.approval_required) == ("allow", True)
        assert decision.matched_rule_index == 0

    def test_the_legacy_boolean_fails_closed_on_it(self) -> None:
        """§6.8.1 as a property of the DECISION, not of the matched rule.

        Before v1.29.0 this returned True: the boolean read a matched rule that
        carried no approval, and a caller that can only read "let it through /
        do not" let a gated call through.
        """
        acl, _ = _approval_acl(_gate(), _broad_allow())
        assert acl.check("agent.planner", "cli.git_push", _ctx()) is False

    async def test_the_async_twins_answer_identically(self) -> None:
        """A governance result MUST NOT depend on which entry point was called."""
        acl, captured = _approval_acl(_gate(), _broad_allow())
        decision = await acl.async_check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required, decision.matched_rule_index) == ("allow", True, 1)
        assert captured[0].approval_required is True
        assert await acl.async_check("agent.planner", "cli.git_push", _ctx()) is False

    async def test_the_async_twin_contains_it_too(self) -> None:
        acl, _ = _approval_acl(_gate(targets=["cli.deploy"]), _broad_allow())
        decision = await acl.async_check_access("agent.planner", "cli.git_push", _ctx())
        assert decision.approval_required is False

    def test_one_pending_rule_is_enough_and_several_do_not_compound(self) -> None:
        """Disjunction, so the flag is idempotent — asserted because a naive

        counter or a last-writer assignment would drop it on the second rule.
        """
        acl, _ = _approval_acl(_gate(), _gate(approval="not_required"), _gate(), _broad_allow())
        decision = acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert (decision.access, decision.approval_required, decision.matched_rule_index) == ("allow", True, 3)

    def test_the_warning_says_the_requirement_is_pending(self, caplog) -> None:
        """§6.1.1 rule 3's warning would otherwise read as "no further effect",

        which is the exact misreading rule 5 corrects.
        """
        acl, _ = _approval_acl(_gate(), _broad_allow())
        with caplog.at_level(logging.WARNING):
            acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert "PENDING" in caplog.text
        assert "§6.1.1 rule 5" in caplog.text

    def test_no_such_clause_when_the_rule_carried_no_approval(self, caplog) -> None:
        acl, _ = _approval_acl(_gate(approval="not_required"), _broad_allow())
        with caplog.at_level(logging.WARNING):
            acl.check_access("agent.planner", "cli.git_push", _ctx())
        assert "PENDING" not in caplog.text


# ---------------------------------------------------------------------------
# D-88 (spec v1.49.0) — the §6.5 warning dedupe is keyed by rule INDEX, so any
# operation that inserts, removes or reorders rules must clear it.
# ---------------------------------------------------------------------------


class TestSixFiveWarningDedupeClearedOnIndexShift:
    """A stale marker suppresses the warning for a DIFFERENT rule.

    ``_warned_missing_context`` is keyed by ``(rule_index, effect)``. Both
    ``add_rule`` (inserts at index 0) and ``remove_rule`` (pops at index ``i``)
    shift the rules after the mutation point, so a marker recorded for the old
    occupant of an index would silence the new one — and specifically the rule
    the operator just touched.
    """

    @staticmethod
    def _rule(marker_role: str) -> ACLRule:
        return ACLRule(
            callers=["*"],
            targets=["*"],
            effect="deny",
            conditions={"roles": [marker_role]},
        )

    @staticmethod
    def _no_context_warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
        return [r.message for r in caplog.records if "no context" in r.message]

    def test_the_warning_is_deduped_without_a_mutation(self, caplog: pytest.LogCaptureFixture) -> None:
        acl = ACL(rules=[self._rule("admin")], default_effect="allow")
        with caplog.at_level(logging.WARNING):
            acl.check("caller", "target", None)
            acl.check("caller", "target", None)
        assert len(self._no_context_warnings(caplog)) == 1

    def test_add_rule_clears_the_dedupe(self, caplog: pytest.LogCaptureFixture) -> None:
        acl = ACL(rules=[self._rule("admin")], default_effect="allow")
        with caplog.at_level(logging.WARNING):
            acl.check("caller", "target", None)
            assert len(self._no_context_warnings(caplog)) == 1

            caplog.clear()
            acl.add_rule(self._rule("operator"))
            acl.check("caller", "target", None)

        # Two rules, two indices, and neither is suppressed by the marker the
        # pre-insertion rule left at index 0.
        assert len(self._no_context_warnings(caplog)) == 2

    def test_remove_rule_clears_the_dedupe(self, caplog: pytest.LogCaptureFixture) -> None:
        acl = ACL(
            rules=[self._rule("admin"), self._rule("operator")],
            default_effect="allow",
        )
        with caplog.at_level(logging.WARNING):
            acl.check("caller", "target", None)
            assert len(self._no_context_warnings(caplog)) == 2

            caplog.clear()
            removed = acl.remove_rule(["*"], ["*"], conditions={"roles": ["admin"]})
            assert removed is True
            acl.check("caller", "target", None)

        # The surviving rule moved from index 1 to index 0. Without the clear,
        # the marker left by the REMOVED rule at index 0 would silence it.
        assert len(self._no_context_warnings(caplog)) == 1

    def test_a_failed_remove_does_not_have_to_clear(self, caplog: pytest.LogCaptureFixture) -> None:
        """No rule removed means no index shifted, so the dedupe still holds."""
        acl = ACL(rules=[self._rule("admin")], default_effect="allow")
        with caplog.at_level(logging.WARNING):
            acl.check("caller", "target", None)
            caplog.clear()
            assert acl.remove_rule(["nobody"], ["*"]) is False
            acl.check("caller", "target", None)
        assert self._no_context_warnings(caplog) == []
