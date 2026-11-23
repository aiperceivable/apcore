"""Conformance driver for the ACL unevaluable-condition fixture (spec §6.1.1/§6.1.4, #100, #106).

SECURITY golden test. A condition that CANNOT BE EVALUATED is not the same as a
condition that is FALSE, and the rule's ``effect`` decides what the difference
means: an unevaluable condition MUST resolve the rule toward refusing access —
a ``deny`` rule takes effect and the call is denied, an ``allow`` rule does not
match and MUST NOT grant. "Unevaluable" is a **principle**, not a closed list.

Fixture transition (apcore#100), complete: the corrected fixture has landed in
``conformance/fixtures/acl_handler_error.json``. The case it replaced,
``throwing_handler_does_not_flip_default_allow_to_deny_unsafely``, expected a
``deny`` rule with a crashing handler to let the call through — exactly what
spec v1.22.0 §6.1.1 reverses. ``_SUPERSEDED_CASE_IDS`` below still names it so
that a checkout pointed at a pre-v1.22.0 spec repo reports "superseded" rather
than a bare failure; it goes inert against the current fixture.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.acl import ACL, ACLRule, AuditEntry
from apcore.context import Context, Identity
from conformance.canonical_fixtures import load_fixture

# The roles the driver's context carries. Two cases —
# `structural_fault_gates_even_when_an_or_sibling_is_satisfied` and
# `execution_fault_does_not_gate_when_an_or_sibling_is_satisfied` — turn on the
# caller HOLDING `dev`, so that one `$or` branch is genuinely SATISFIED and the
# gating-vs-composition question actually gets asked. The fixture states that
# only in the cases' prose notes; no field carries it. Reported upstream.
_CONTEXT_ROLES = ("dev",)

# Superseded by spec v1.22.0 §6.1.1. Present only in the pre-#100 fixture; the
# corrected fixture replaces it with ``throwing_handler_on_deny_rule_denies``.
_SUPERSEDED_CASE_IDS = {"throwing_handler_does_not_flip_default_allow_to_deny_unsafely"}


def _load_fixture() -> dict:
    """Load the canonical fixture from the apcore spec repo."""
    return load_fixture("acl_handler_error.json")


_FIXTURE = _load_fixture()
_THROWING_KEY: str = _FIXTURE["throwing_condition_key"]
# Only the corrected fixture names an unregistered key. Absent from the old one.
_UNKNOWN_KEY: str | None = _FIXTURE.get("unknown_condition_key")


class _ThrowingConditionHandler:
    """Condition handler whose evaluate() always raises (unevaluable driver)."""

    def evaluate(self, value, context: Context) -> bool:  # noqa: ANN001
        raise RuntimeError("conformance throwing condition handler")


@pytest.fixture(autouse=True)
def _register_throwing_handler():
    """Register the throwing handler for the fixture key, then clean up.

    Nothing is registered for ``unknown_condition_key`` — that key is
    unevaluable *because* no handler resolves it. The setup also removes it
    defensively, so an unrelated test that registered it cannot make this driver
    silently assert the wrong thing.
    """
    previous = ACL._condition_handlers.get(_THROWING_KEY)
    ACL.register_condition(_THROWING_KEY, _ThrowingConditionHandler())
    unknown_previous = ACL._condition_handlers.pop(_UNKNOWN_KEY, None) if _UNKNOWN_KEY else None
    try:
        yield
    finally:
        if previous is not None:
            ACL._condition_handlers[_THROWING_KEY] = previous
        else:
            ACL._condition_handlers.pop(_THROWING_KEY, None)
        if _UNKNOWN_KEY and unknown_previous is not None:
            ACL._condition_handlers[_UNKNOWN_KEY] = unknown_previous


def _build_rules(raw_rules: list[dict]) -> list[ACLRule]:
    """Build ACLRules, honouring the §6.1.4.1 ``*_raw`` escape hatches.

    ``callers_raw`` / ``targets_raw`` carry a value that is deliberately NOT a
    list of strings, to exercise the malformed-pattern-field case. Python can
    represent all of them, so no case is skipped here; ``skip_if_unrepresentable``
    exists for apcore-rust, whose ``Vec<String>`` satisfies §6.1.4.1 by
    construction.
    """
    rules: list[ACLRule] = []
    for raw in raw_rules:
        callers: Any = raw["callers_raw"] if "callers_raw" in raw else raw["callers"]
        targets: Any = raw["targets_raw"] if "targets_raw" in raw else raw["targets"]
        rules.append(
            ACLRule(
                callers=callers,
                targets=targets,
                effect=raw["effect"],
                description=raw.get("description", ""),
                conditions=raw.get("conditions"),
            )
        )
    return rules


def _reported_paths(handler_error: str) -> list[str]:
    """Split a ``handler_error`` into the condition paths it names, in order.

    Each part is ``"<path>: <reason>"`` and a reason may itself contain ``": "``
    (``RuntimeError: boom``), so only the first separator delimits the path.
    """
    return [part.split(": ", 1)[0] for part in handler_error.split("; ")]


def _case_params() -> list:
    params = []
    for case in _FIXTURE["test_cases"]:
        marks = []
        if case["id"] in _SUPERSEDED_CASE_IDS:
            marks.append(
                pytest.mark.xfail(
                    strict=True,
                    reason=(
                        "Fixture pins pre-v1.22.0 behaviour: a deny rule whose condition handler "
                        "raises used to fall through to default_effect: allow. PROTOCOL_SPEC "
                        "§6.1.1 (#100) now makes the rule take effect. The corrected fixture is "
                        "staged in the spec repo and lands once all three SDK drivers do."
                    ),
                )
            )
        params.append(pytest.param(case, id=case["id"], marks=marks))
    return params


@pytest.mark.parametrize("case", _case_params())
def test_acl_handler_error(case: dict) -> None:
    captured: list[AuditEntry] = []

    acl = ACL(
        rules=_build_rules(case["rules"]),
        default_effect=case["default_effect"],
        audit_logger=captured.append,
    )

    # §6.1.4: the precheck is context-independent and runs before §6.5's
    # no-context check, so some cases deliberately supply no context at all.
    # The pre-#100 fixture has no `with_context` key and always wants one.
    context = (
        Context.create(identity=Identity(id="conformance-caller", type="user", roles=_CONTEXT_ROLES))
        if case.get("with_context", True)
        else None
    )
    decision = acl.check(case["caller_id"], case["target_id"], context)

    assert decision is case["expected"], f"expected decision {case['expected']}, got {decision}"

    assert len(captured) == 1, f"expected exactly one audit entry, got {len(captured)}"
    entry = captured[0]

    if not case["expected_audit_handler_error_present"]:
        assert entry.handler_error is None, (
            f"expected AuditEntry.handler_error to be null, got {entry.handler_error!r} — "
            "a well-formed rule skipped for want of a context is NOT unevaluable (§6.1.4 rule 2)"
        )
        return

    assert entry.handler_error is not None, "expected AuditEntry.handler_error to be non-null"

    expected_paths = case.get("expected_handler_error_paths")
    if expected_paths is not None:
        # §6.1.1 rule 2 + §6.1.4 determinism: exactly these paths, in this order.
        assert (
            _reported_paths(entry.handler_error) == expected_paths
        ), f"handler_error {entry.handler_error!r} does not name exactly {expected_paths} in order"
