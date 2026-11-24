"""Cross-language driver for ``acl_argument_scoped_approval.json``.

PROTOCOL_SPEC §6.1.1 / §6.1.6 / §6.1.7 / §6.1.8 / §6.8.1 (spec v1.28.0
apcore#108, extended v1.29.0 apcore#109).

An ACL rule answers two independent questions — may this caller reach this
target at all, and must *this particular call* be put to a human first. The
orthogonal ``approval`` field carries the second, and the built-in
structure-only ``arguments`` condition decides whether a rule matches this call.

**Every case runs twice**, because the ``arguments`` condition can only be
answered when a governance projection is available and §6.1.8 case 1 makes
``check()`` a public entry point that may be called without one: the same rules
and the same call therefore have two well-defined answers, and both are
contracts. Run 1 supplies a projection and asserts the unsuffixed expectations;
run 2 supplies none and asserts the ``*_no_projection`` ones. Python can hand a
projection to the legacy boolean as well as to the structured accessor — it
rides on the :class:`Context` — so this SDK asserts every key in both runs.

The two cases worth reading before the rest are
``no_projection_must_not_grant_via_an_empty_stand_in`` and
``no_projection_makes_a_deny_rule_take_effect``: they bracket the same
fail-open bug from both directions. Substituting an empty key set for an absent
projection makes ``has_none_of`` vacuously satisfied, so an ``allow`` rule
grants for a call whose arguments were never seen — and leaves ``has_key``
unsatisfied, so a ``deny`` rule fails to take effect. Only the UNEVALUABLE
reading (§6.1.8 rule 1) refuses in both.

Then read ``an_out_of_scope_approval_rule_raises_nothing``: it is the guard on
§6.1.1 rule 5, and an implementation that records a pending requirement before
matching the rule's patterns passes every other case here and fails that one.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.acl import ACL, ACLRule, AuditEntry
from apcore.context import Context, GovernanceProjection, Identity

from .canonical_fixtures import fixtures_dir, load_fixture

FIXTURE = "acl_argument_scoped_approval.json"


def _present() -> bool:
    return (fixtures_dir() / FIXTURE).is_file()


# The fixture lands in the spec repo one push after this driver, so that
# `check_driver_coverage.py --strict` has a driver to find for it. Until then
# the module skips and names the unexercised fixture — "not verified", never
# "passed".
pytestmark = pytest.mark.skipif(not _present(), reason=f"{FIXTURE} not in the spec repo yet (spec v1.28.0, #108)")


def _cases() -> list[dict[str, Any]]:
    return load_fixture(FIXTURE)["test_cases"] if _present() else []


def _build(case: dict[str, Any]) -> tuple[ACL, list[AuditEntry]]:
    """Build the case's ACL, honouring ``callers_raw``.

    ``callers_raw`` carries a deliberately malformed pattern field, and the
    fixture marks those cases ``skip_if_unrepresentable`` for SDKs whose type
    system cannot hold one. Python can: :class:`ACLRule` is a plain dataclass,
    so the value reaches §6.1.4.1's precheck exactly as a YAML file's would —
    which is the whole point of the case, so it is built rather than skipped.
    """
    entries: list[AuditEntry] = []
    rules = [
        ACLRule(
            callers=r["callers_raw"] if "callers_raw" in r else list(r["callers"]),
            targets=r["targets_raw"] if "targets_raw" in r else list(r["targets"]),
            effect=r["effect"],
            conditions=r.get("conditions"),
            **({"approval": r["approval"]} if "approval" in r else {}),
        )
        for r in case["rules"]
    ]
    acl = ACL(rules=rules, default_effect=case["default_effect"], audit_logger=entries.append)
    return acl, entries


def _context(case: dict[str, Any], *, with_projection: bool) -> Context:
    """A context carrying the run's projection, or none at all.

    ``arguments: null`` means the case has NO projection to supply in either
    run (§6.1.8 rule 1), which is a different thing from an empty projection —
    so the field stays None rather than being set to
    ``GovernanceProjection.of({})``, and the two runs coincide. The context
    itself is always present: §6.5's "conditions but no context" non-match is a
    separate rule this fixture is not about.
    """
    ctx = Context(trace_id="t-109", caller_id=case["caller_id"], identity=Identity(id="u", roles=("dev",)))
    if with_projection and case["arguments"] is not None:
        ctx.governance_projection = GovernanceProjection.of(case["arguments"])
    return ctx


def _expected(case: dict[str, Any], name: str, *, with_projection: bool) -> Any:
    """Read one expectation from the run's column of the fixture."""
    return case[f"expected_{name}" if with_projection else f"expected_{name}_no_projection"]


def _assert_run(case: dict[str, Any], *, with_projection: bool) -> None:
    """Assert one column: the structured decision, the audit entry, the boolean."""
    acl, entries = _build(case)
    column = "with a projection" if with_projection else "with NO projection"
    note = f"{case['note']}\n  [{column}]"

    decision = acl.check_access(case["caller_id"], case["target_id"], _context(case, with_projection=with_projection))
    assert decision.access == _expected(case, "access", with_projection=with_projection), note
    assert decision.approval_required is _expected(case, "approval_required", with_projection=with_projection), note
    assert decision.matched_rule_index == _expected(case, "matched_rule_index", with_projection=with_projection), note

    # §6.3.1: handler_error is non-null IF AND ONLY IF a condition was
    # unevaluable. Read before the legacy call below, which emits its own entry.
    assert len(entries) == 1, f"{note}\n  check_access must emit exactly one audit entry"
    entry = entries[0]

    # §6.8.1: the legacy boolean fails closed on an approval requirement — as a
    # property of the DECISION since v1.29.0, so a pending requirement raised by
    # a rule that did not itself match closes it too.
    legacy = acl.check(case["caller_id"], case["target_id"], _context(case, with_projection=with_projection))
    assert legacy is _expected(case, "legacy_check", with_projection=with_projection), note

    handler_error_expected = _expected(case, "audit_handler_error_present", with_projection=with_projection)
    assert (
        entry.handler_error is not None
    ) is handler_error_expected, f"{note}\n  handler_error was {entry.handler_error!r}"
    # §6.1.1 rule 5: a pending requirement neither suppresses nor substitutes
    # for handler_error, and the entry carries the FINAL approval value.
    assert entry.approval_required is _expected(case, "approval_required", with_projection=with_projection), note


async def _assert_async_run(case: dict[str, Any], *, with_projection: bool) -> None:
    """The async twins answer identically — §6.1.1 rule 5 must not drift by path."""
    acl, entries = _build(case)
    column = "with a projection" if with_projection else "with NO projection"
    note = f"{case['note']}\n  [async, {column}]"

    ctx = _context(case, with_projection=with_projection)
    decision = await acl.async_check_access(case["caller_id"], case["target_id"], ctx)
    assert decision.access == _expected(case, "access", with_projection=with_projection), note
    assert decision.approval_required is _expected(case, "approval_required", with_projection=with_projection), note
    assert decision.matched_rule_index == _expected(case, "matched_rule_index", with_projection=with_projection), note

    handler_error_expected = _expected(case, "audit_handler_error_present", with_projection=with_projection)
    assert (entries[0].handler_error is not None) is handler_error_expected, note

    legacy = await acl.async_check(
        case["caller_id"], case["target_id"], _context(case, with_projection=with_projection)
    )
    assert legacy is _expected(case, "legacy_check", with_projection=with_projection), note


@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["id"])
def test_argument_scoped_approval(case: dict[str, Any]) -> None:
    _assert_run(case, with_projection=True)
    _assert_run(case, with_projection=False)


@pytest.mark.asyncio
@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["id"])
async def test_argument_scoped_approval_async(case: dict[str, Any]) -> None:
    await _assert_async_run(case, with_projection=True)
    await _assert_async_run(case, with_projection=False)


@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["id"])
def test_validate_rules(case: dict[str, Any]) -> None:
    """§6.1.8 closing paragraph — validation is context-free, so it has ONE column.

    Cases 2-4 are decidable with no context and no handler, so
    ``validate_rules()`` must surface them at deploy time.
    """
    acl, _ = _build(case)
    note = case["note"]

    all_findings = acl.validate_rules()
    expected_path = case["expected_validation_finding_path"]
    # §6.1.8 rule 3: every faulty predicate is reported, so a case may pin the
    # exact finding set rather than the presence of one.
    expected_paths = case.get("expected_validation_finding_paths")
    if expected_paths is not None:
        assert [f.condition_path for f in all_findings] == expected_paths, note
        for finding in all_findings:
            assert finding.sync_resolvable is False, note
            assert finding.async_resolvable is False, note
    elif expected_path:
        findings = [f for f in all_findings if f.condition_path == expected_path]
        assert findings, f"{note}\n  validate_rules() reported no finding at '{expected_path}': {all_findings}"
        assert findings[0].sync_resolvable is False, note
        assert findings[0].async_resolvable is False, note
    else:
        assert all_findings == (), f"{note}\n  unexpected findings: {all_findings}"


def test_deny_plus_approval_is_rejected_at_every_entry_point() -> None:
    """§6.1.6 rule 3 — the meaningless combination cannot get in by any door."""
    from apcore.errors import ACLRuleError

    with pytest.raises(ACLRuleError):
        ACLRule(callers=["*"], targets=["x.y"], effect="deny", approval="required")
