"""Cross-language driver for ``acl_effect_value_closure.json``.

PROTOCOL_SPEC §6.1.5 (spec v1.30.0, apcore#111): an ACL rule's ``effect`` value
is a closed set — ``allow`` and ``deny``, nothing else — and a rule carrying
anything else fails with ``ACLRuleError`` at **every** entry point that accepts
a rule. ``default_effect`` is closed on the same terms.

**The entry point is the substance of this fixture, not a detail.** The defect
it pins is not a missing check: the check existed, in this SDK's ``ACL.load``,
and was reachable from one of its three doors. ``effect: "Allow"`` — the
capitalisation an operator writes by hand — failed from a YAML file and was
*accepted* through ``ACLRule(...)`` and ``add_rule()``, then read as ``deny`` at
check time. So each case drives every door in its ``entry_points`` list, and the
fixture states no per-door expectation on purpose: a value legal through one
door and illegal through another is precisely the defect, so the fixture has no
shape that could express it.

``add_rule`` is exercised through its **kwargs** path as well as with a
pre-built rule, because they are two different constructions of the rule and
only one of them is ``add_rule``'s own. Both must reject.

Where a case is rejected there is no access decision to assert, which is the
point — §6.1.5 forbids resolving an unrecognised ``effect`` to a decision at
all. ``test_a_mutated_effect_is_not_resolved_to_a_decision`` in
``tests/test_acl.py`` covers the one route the closed doors cannot: assigning
``rule.effect`` on an already-constructed dataclass.
"""

from __future__ import annotations

import tempfile
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL, ACLRule
from apcore.errors import ACLRuleError

from .canonical_fixtures import fixtures_dir, load_fixture, reject_unknown_expectations

FIXTURE = "acl_effect_value_closure.json"


def _present() -> bool:
    return (fixtures_dir() / FIXTURE).is_file()


# The fixture lands in the spec repo one push after this driver, so that
# `check_driver_coverage.py --strict` has a driver to find for it. Until then
# the module skips and names the unexercised fixture — "not verified", never
# "passed".
pytestmark = pytest.mark.skipif(not _present(), reason=f"{FIXTURE} not in the spec repo yet (spec v1.30.0, #111)")


def _cases() -> list[dict[str, Any]]:
    return load_fixture(FIXTURE)["test_cases"] if _present() else []


# ---------------------------------------------------------------------------
# The three doors, each as the single expression a user would write
# ---------------------------------------------------------------------------


def _door_load(case: dict[str, Any]) -> ACL:
    """File loading: the rule reaches the ACL through ``ACL.load``."""
    doc = {"default_effect": case["default_effect"], "rules": [case["rule"]]}
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump(doc))
        return ACL.load(str(path))


def _door_construct(case: dict[str, Any]) -> ACL:
    """Direct construction: ``ACLRule(...)`` inside ``ACL(...)``.

    One door rather than two because a case may put the offending value on
    either field — ``ACLRule`` rejects a bad ``effect`` and the ``ACL``
    constructor rejects a bad ``default_effect``, and the fixture lists
    ``construct`` for both.
    """
    return ACL(rules=[ACLRule(**case["rule"])], default_effect=case["default_effect"])


def _door_add_rule(case: dict[str, Any]) -> ACL:
    """Runtime insertion with a pre-built rule."""
    acl = ACL(default_effect=case["default_effect"])
    acl.add_rule(ACLRule(**case["rule"]))
    return acl


def _door_add_rule_kwargs(case: dict[str, Any]) -> ACL:
    """Runtime insertion through ``add_rule``'s own kwargs path.

    ``add_rule`` returns ``None``, which §6.1.6 rule 3 explicitly does not treat
    as an exemption from rejecting a bad rule: it raises, the way Python signals
    a value that cannot be constructed.
    """
    acl = ACL(default_effect=case["default_effect"])
    acl.add_rule(
        callers=list(case["rule"]["callers"]),
        targets=list(case["rule"]["targets"]),
        effect=case["rule"]["effect"],
    )
    return acl


#: Fixture door name -> the driver callables that exercise it. ``add_rule`` has
#: two because the pre-built and kwargs constructions are different code paths
#: through the same public method.
_DOORS: dict[str, tuple[Any, ...]] = {
    "load": (_door_load,),
    "construct": (_door_construct,),
    "add_rule": (_door_add_rule, _door_add_rule_kwargs),
}


def _entry_points(case: dict[str, Any]) -> list[str]:
    """The doors this case lists, failing on one this driver cannot drive.

    A door silently skipped is the shape this whole fixture exists to catch, so
    an unknown name fails rather than being ignored.
    """
    declared = case["entry_points"]
    unknown = sorted(set(declared) - set(_DOORS))
    if unknown:
        pytest.fail(
            f"[{FIXTURE} :: {case['id']}] lists entry point(s) {unknown} this driver "
            f"does not drive. Teach the driver, do not skip it. Known: {sorted(_DOORS)}"
        )
    return list(declared)


def _offending_value(case: dict[str, Any]) -> str:
    """The out-of-enum value a rejecting case carries, on whichever field holds it.

    A case puts it on the rule's ``effect`` or on ``default_effect``, never on
    both, so the message must name that one — "invalid effect" with no value in
    it would leave the operator to guess which of two fields they mistyped.
    """
    if case["rule"]["effect"] not in ("allow", "deny"):
        return str(case["rule"]["effect"])
    return str(case["default_effect"])


@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["id"])
def test_acl_effect_value_closure(case: dict[str, Any]) -> None:
    reject_unknown_expectations(FIXTURE, case, {"expected_load"})
    expected = case["expected_load"]
    assert expected in ("ok", "reject"), f"[{FIXTURE} :: {case['id']}] unknown expected_load {expected!r}"

    for door in _entry_points(case):
        for build in _DOORS[door]:
            where = f"{case['note']}\n  door={door} ({build.__name__})"
            if expected == "ok":
                acl = build(case)
                # A positive case needs an observable post-condition: "did not
                # raise" is also what an implementation that drops the rule does.
                assert len(acl.rules) == 1, where
                assert acl.rules[0].effect == case["rule"]["effect"], where
                assert acl.default_effect == case["default_effect"], where
            else:
                with pytest.raises(ACLRuleError) as excinfo:
                    build(case)
                message = str(excinfo.value)
                quoted = f"'{_offending_value(case)}'"
                assert quoted in message, f"{where}\n  message did not name {quoted}: {message}"


def test_every_case_names_at_least_one_door() -> None:
    """A case with no ``entry_points`` would assert nothing while reading as covered."""
    for case in _cases():
        assert case["entry_points"], case["id"]


def test_the_loader_message_names_the_rule_index() -> None:
    """§6.1.5: the error names the rule index *and* the offending value.

    The index is what the loader can say and the dataclass cannot, so it is
    asserted on the one door that knows it — and on rule 1 rather than rule 0,
    where an off-by-one would read as correct.
    """
    doc = {
        "default_effect": "deny",
        "rules": [
            {"callers": ["agent.*"], "targets": ["orders.*"], "effect": "allow"},
            {"callers": ["agent.*"], "targets": ["orders.*"], "effect": "Allow"},
        ],
    }
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump(doc))
        with pytest.raises(ACLRuleError, match=r"Rule 1 has invalid effect 'Allow'"):
            ACL.load(str(path))
