"""Cross-language driver for ``acl_pattern_arity.json``.

PROTOCOL_SPEC §6.2.1 (spec v1.31.0, apcore#112): a ``callers`` / ``targets``
pattern array's **arity** is a closed set. The array MUST carry at least one
operand, every element MUST be a non-empty string, ``$or`` at index 0 MUST be
followed by at least one pattern, ``$not`` at index 0 by exactly one, and
``$or`` / ``$not`` MUST NOT appear at any index other than 0 — a pattern array
is FLAT, the operators do not nest, and there is no precedence. Anything else is
rejected with ``ACLRuleError`` at **every** entry point that accepts a rule.

**Why a rejection and not a non-match.** Through v1.30.0 all three SDKs returned
``False`` from the matcher for ``[]``, ``["$or"]`` and ``["$not"]``, reading an
arity fault as a *scope* decision. The rule was then inert: the outcome tracked
``default_effect`` exactly and ``validate_rules()`` reported nothing. On an
``allow`` rule that is merely useless. On a ``deny`` rule under
``default_effect: allow`` the call the operator wrote the rule to block is
PERMITTED — reachable from a plain YAML file, because ``ACL.load`` rejects an
*omitted* ``callers`` / ``targets`` and permitted an *empty* one.

**Two case shapes, discriminated by ``kind``.**

``closure`` offers the rule at every door in ``entry_points`` and asserts
``expected_load``. There is deliberately no per-door expectation: a shape legal
through one door and illegal through another is precisely the defect, so the
fixture has no shape that could express it. A ``closure`` case that also carries
``expected_validation_finding_paths`` is a **tier-2** case — well-formed under
every structural rule and still matching no legal module ID, of which
``["$not", "*"]`` is the sharpest example. It MUST load, MUST be reported by
``validate_rules()``, and MUST NOT change any decision.

**A case carries either ``rule`` (one) or ``rules`` (an ordered list).** The
list form pins §6.2.1 point 2's **cross-rule** half — the index chooses the
rule, then the axis order chooses the fault inside it — and is offered at
``load`` and ``construct`` only, since ``add_rule`` takes one rule at a time and
cannot express it. ``expected_load`` sees neither half of the ordering, so
``expected_refused_axis`` and ``expected_refused_rule_index`` carry it: an
implementation with the wrong order still raises ``ACLRuleError`` for every case
here, which is exactly why the plain reject/accept expectation is not enough.
An implementation that sweeps one axis across every rule before looking at the
next passes every single-rule case and fails the discriminating pair — that is
the defect measured in apcore-typescript, whose loader validated rule by rule
while its constructor swept axis by axis, so one file produced two different
errors through two doors of the same SDK.

**"Axis" is every per-rule check a door performs**, not only ``effect`` /
``approval`` / patterns. A loader has axes the other doors cannot have — #107's
rule-key closure, the missing-field check, the value-type checks — and the sweep
prohibition binds those too: apcore-rust closed the key set across the whole
file before any rule's ``effect`` was read, and refused for rule 1 a file that
was already bad at rule 0. ``default_effect`` sits outside the scheme entirely:
it is not a rule, has no index, and is judged **first, before any rule, at every
door**, so ``expected_refused_axis: "default_effect"`` also asserts that no rule
axis and no rule index were named.

``backstop`` is the one route no door covers: assigning the field on an
**already-installed** rule, which no constructor can intercept and which —
unlike an unrecognised ``effect``, never read again once the doors are shut —
the matcher WILL consult on the next ``check()``. §6.1.4.1 then classifies it
exactly as it classifies a malformed type: the scope is unreadable, the rule is
UNEVALUABLE, and §6.1.1's effect table decides. **Two decision surfaces, and
they diverge**: ``expected_access`` is the string on the structured accessor,
``expected_legacy_check`` the legacy boolean, and §6.8.1 makes the boolean
fail-closed for allow-with-approval-required — which is
``mutated_empty_targets_on_approval_rule_raises_pending_requirement``, the case
a driver reading the boolean as if it were ``access`` fails alone.

**The doors are the substance.** ``add_rule`` is driven three ways, because they
are three different constructions of the rule and only one of them is
``ACLRule``'s: pre-built, ``add_rule``'s own kwargs path, and — since spec
v1.31.0 resolved it normatively — a rule that was **well-formed when
constructed and mutated afterwards**. That third one is the door this SDK did
not close: the check was threaded through ``ACLRule.__post_init__``, which
reaches ``add_rule`` only *through construction*, so ``r.targets = []`` followed
by ``acl.add_rule(r)`` inserted without raising. The fixture cannot express the
difference — ``entry_points`` carries no per-door expectation — so the third
callable below is what pins it.
"""

from __future__ import annotations

import re
import tempfile
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL, ACLRule, AuditEntry
from apcore.errors import ACLRuleError

from .canonical_fixtures import (
    dispatch_or_fail,
    fixtures_dir,
    load_fixture,
    reject_unknown_expectations,
)

FIXTURE = "acl_pattern_arity.json"


def _present() -> bool:
    return (fixtures_dir() / FIXTURE).is_file()


# The fixture lands in the spec repo alongside the three SDK drivers. Until the
# checkout carries it the module skips and names the unexercised fixture —
# "not verified", never "passed".
pytestmark = pytest.mark.skipif(not _present(), reason=f"{FIXTURE} not in the spec repo yet (spec v1.31.0, #112)")


def _cases() -> list[dict[str, Any]]:
    return load_fixture(FIXTURE)["test_cases"] if _present() else []


#: Every key a fixture rule may carry, and which :class:`ACLRule` accepts.
#: A case carrying anything else — `priority`, say — is exercising the loader's
#: rule-key closure (#107), which no typed rule can express, and the fixture
#: lists only the `load` door for it.
_RULE_KWARG_KEYS = ("callers", "targets", "effect", "description", "approval")


def _rule_kwargs(raw: dict[str, Any]) -> dict[str, Any]:
    """The case's rule as :class:`ACLRule` kwargs, carrying only the keys it states.

    Fails on a key :class:`ACLRule` cannot take rather than dropping it. A
    silently dropped key would hand the constructing doors a rule the fixture
    did not describe — and for a case whose whole point is that key, the door
    would report itself exercised having never seen the fault.
    """
    unknown = sorted(set(raw) - set(_RULE_KWARG_KEYS))
    if unknown:
        pytest.fail(
            f"[{FIXTURE}] a rule carries key(s) {unknown} that ACLRule cannot take. "
            f"Such a case belongs at the 'load' door alone; driving it here would drop them silently."
        )
    kwargs: dict[str, Any] = {
        "callers": list(raw["callers"]),
        "targets": list(raw["targets"]),
        "effect": raw["effect"],
    }
    if "description" in raw:
        kwargs["description"] = raw["description"]
    if "approval" in raw:
        kwargs["approval"] = raw["approval"]
    return kwargs


def _raw_rules(case: dict[str, Any]) -> list[dict[str, Any]]:
    """The case's rules, in order.

    A case carries either ``rule`` (one) or ``rules`` (an ordered list); the
    list form exists for §6.2.1's cross-rule half. Exactly one of the two, so a
    case that grew a second shape by accident fails here rather than having one
    of them silently ignored.
    """
    has_one, has_many = "rule" in case, "rules" in case
    if has_one == has_many:
        pytest.fail(
            f"[{FIXTURE} :: {case['id']}] must carry exactly one of 'rule' / 'rules', "
            f"got rule={has_one} rules={has_many}"
        )
    return list(case["rules"]) if has_many else [case["rule"]]


def _only_rule(case: dict[str, Any]) -> dict[str, Any]:
    """The case's single rule, for a door that cannot take a list.

    ``add_rule`` takes one rule at a time, so a multi-rule case cannot be
    offered at it — and the fixture does not list it for one. A case that did
    fails loudly here: silently driving only the head of the list would report
    a door as exercised that never saw the fault.
    """
    rules = _raw_rules(case)
    if len(rules) != 1:
        pytest.fail(
            f"[{FIXTURE} :: {case['id']}] lists an 'add_rule' entry point for a "
            f"{len(rules)}-rule case; that door takes one rule at a time and cannot express it"
        )
    return rules[0]


# ---------------------------------------------------------------------------
# `kind: closure` — the three doors, each as the expression a user would write
# ---------------------------------------------------------------------------


def _door_load(case: dict[str, Any]) -> ACL:
    """File loading: the rules reach the ACL through ``ACL.load``.

    Not a theoretical door. ``ACL.load`` rejects an *omitted* ``callers`` /
    ``targets`` and permitted an *empty* one, so ``targets: []`` under
    ``effect: deny`` was a rule an operator could write in YAML, that loaded
    clean, and that blocked nothing. It is also the only door that can name a
    rule **index**, which is what makes it the one that can see the cross-rule
    half of §6.2.1 point 2 directly.
    """
    doc = {"default_effect": case["default_effect"], "rules": _raw_rules(case)}
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump(doc))
        return ACL.load(str(path))


def _door_construct(case: dict[str, Any]) -> ACL:
    """Direct construction: ``ACLRule(...)`` inside ``ACL(...)``.

    The list is built in index order, so the first bad rule raises — which is
    the cross-rule half at this door. It cannot *name* the index: a rule under
    construction has no position yet and §6.1.5 forbids inventing one, so which
    rule was refused is observable only through the axis its fault sits on.
    That is exactly what the fixture's discriminating pair is built to expose.
    """
    # §6.2.1 puts `default_effect` FIRST, before any rule, at every door. In
    # Python the rule expression is evaluated before the ACL constructor is
    # entered at all, and `ACLRule(...)` is a door in its own right — so the
    # constructor is offered the `default_effect` before the rules are built,
    # which is the order the door is required to judge in. This asserts nothing
    # on its own: an ACL constructor that did not judge `default_effect` first
    # would return here happily and the case would fail on the line below.
    ACL(default_effect=case["default_effect"])
    return ACL(
        rules=[ACLRule(**_rule_kwargs(raw)) for raw in _raw_rules(case)],
        default_effect=case["default_effect"],
    )


def _door_construct_mutated(case: dict[str, Any]) -> ACL:
    """Direct construction of a rule that was well-formed **when constructed**.

    Mirrors :func:`_door_add_rule_mutated` at the other door §6.1.6 rule 3
    names: ``ACL``'s own constructor accepts a list of already-built
    ``ACLRule`` objects exactly as ``add_rule`` accepts one, so a rule mutated
    after its own construction and handed to ``ACL(rules=[...])`` for the
    first time is being offered to a door for the first time — construction
    history is not legible to the door and cannot exempt a rule from the check
    every other rule reaching it gets (spec v1.33.0, §6.1.4.1/§6.2.1
    disambiguation of "assigned onto an already-constructed rule").

    :func:`_door_construct` alone cannot see this gap: it builds each
    ``ACLRule`` from the case's bad value *directly*, which raises inside
    ``ACLRule.__post_init__`` before ``ACL(rules=[...])`` is ever reached — so
    the ACL constructor's own validation, as opposed to ``ACLRule``'s, is
    never actually exercised by that driver alone. This one mutates a
    well-formed rule per the case's `rule`/`rules` shape and hands the whole
    list to the constructor in one call, same as `_door_construct` does for
    the cross-rule ordering it asserts.
    """
    rules = []
    for raw in _raw_rules(case):
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        for name, value in _rule_kwargs(raw).items():
            setattr(rule, name, value)
        rules.append(rule)
    return ACL(rules=rules, default_effect=case["default_effect"])


def _door_add_rule(case: dict[str, Any]) -> ACL:
    """Runtime insertion with a pre-built rule."""
    acl = ACL(default_effect=case["default_effect"])
    acl.add_rule(ACLRule(**_rule_kwargs(_only_rule(case))))
    return acl


def _door_add_rule_kwargs(case: dict[str, Any]) -> ACL:
    """Runtime insertion through ``add_rule``'s own kwargs path.

    ``add_rule`` returns ``None``; §6.1.6 rule 3 explicitly does not treat that
    as an exemption from rejecting a bad rule.
    """
    acl = ACL(default_effect=case["default_effect"])
    acl.add_rule(**_rule_kwargs(_only_rule(case)))
    return acl


def _door_add_rule_mutated(case: dict[str, Any]) -> ACL:
    """Runtime insertion of a rule that was well-formed **when constructed**.

    §6.2.1's first point of order: a rule offered to runtime insertion MUST be
    validated at that moment, whatever its history, and an implementation MUST
    NOT rely on the rule type's own construction-time check to cover this door.
    ``ACLRule`` is a non-frozen dataclass, so this route passes through no
    constructor at all — and it is the route on which two of three
    implementations re-validated and one did not.

    **Every** field the case states is assigned, not only the pattern arrays:
    the baseline rule is well-formed on all three axes, so a case whose fault
    sits on ``effect`` or ``approval`` reaches ``add_rule`` through this door
    too, and the axis order §6.2.1 fixes is asserted at runtime insertion as
    well as at the two constructing doors.
    """
    acl = ACL(default_effect=case["default_effect"])
    rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
    for name, value in _rule_kwargs(_only_rule(case)).items():
        setattr(rule, name, value)
    acl.add_rule(rule)
    return acl


#: Fixture door name -> the driver callables that exercise it. ``add_rule`` has
#: three because a pre-built rule, the kwargs path and a rule mutated after
#: construction are three different constructions through one public method,
#: and only the first two go through :class:`ACLRule`. ``construct`` has two
#: for the same reason ``add_rule`` has more than one: a rule built with the
#: bad value directly (raising inside ``ACLRule.__post_init__``, never
#: reaching ``ACL.__init__``'s own check) and a rule mutated after its own
#: construction (which does reach it) are different paths through one door,
#: and only the direct-construction fault is representable in apcore-rust's
#: type system — the mutated path is what closes the ambiguity spec v1.33.0
#: resolved between "already-constructed rule" meaning "already installed in a
#: live ACL" versus "any rule object currently holding a bad value".
_DOORS: dict[str, tuple[Any, ...]] = {
    "load": (_door_load,),
    "construct": (_door_construct, _door_construct_mutated),
    "add_rule": (_door_add_rule, _door_add_rule_kwargs, _door_add_rule_mutated),
}

#: The doors whose error message carries a rule **index**. Only the loader has
#: one: §6.1.5 says a rule under construction has no position yet and an
#: implementation MUST NOT invent one, so every other door names the rule
#: ``ACLRule`` and nothing more. ``expected_refused_rule_index`` is therefore
#: asserted positively at the loader and, at the other doors, as the negative
#: it implies — no invented index — rather than being skipped there.
_DOORS_THAT_NAME_THE_RULE_INDEX = frozenset({_door_load})

_NAMED_RULE_INDEX = re.compile(r"\bRule (\d+)\b")


def _entry_points(case: dict[str, Any]) -> list[str]:
    """The doors this case lists, failing on one this driver cannot drive.

    A door silently skipped is the shape this fixture exists to catch, so an
    unknown name fails rather than being ignored.
    """
    declared = case["entry_points"]
    unknown = sorted(set(declared) - set(_DOORS))
    if unknown:
        pytest.fail(
            f"[{FIXTURE} :: {case['id']}] lists entry point(s) {unknown} this driver "
            f"does not drive. Teach the driver, do not skip it. Known: {sorted(_DOORS)}"
        )
    return list(declared)


#: The value the fixture gives the field a case is *not* testing. Used only to
#: check that a rejection names the field that is actually faulty: the message
#: must name a field whose declared value is something other than this, and
#: ``["*"]`` is legal under every clause of §6.2.1, so the assertion can only
#: catch a wrong field — never fail on a right one.
_UNTESTED_FIELD_VALUE = ["*"]

_NAMED_FIELD = re.compile(r"has an invalid '(callers|targets)'")
_NAMED_EFFECT = re.compile(r"has invalid effect '(.*?)'")
_NAMED_APPROVAL = re.compile(r"carries approval: \w+ on a '\w+' rule|has invalid approval")
_NAMED_DEFAULT_EFFECT = re.compile(r"Invalid default_effect '(.*?)'")


def _refused_rule(case: dict[str, Any]) -> dict[str, Any]:
    """The rule the case says should be refused — rule 0 unless it says otherwise."""
    return _raw_rules(case)[case.get("expected_refused_rule_index", 0)]


def _assert_pattern_axis(case: dict[str, Any], message: str, where: str, field: str | None) -> None:
    """The rejection is a §6.2.1 shape fault, on *field* when the case names one."""
    assert "PROTOCOL_SPEC §6.2.1" in message, f"{where}\n  message did not cite §6.2.1: {message}"
    named = _NAMED_FIELD.search(message)
    assert named is not None, f"{where}\n  message named neither 'callers' nor 'targets': {message}"
    if field is not None:
        assert named.group(1) == field, (
            f"{where}\n  §6.2.1 orders the pattern axis 'callers' before 'targets'; the message "
            f"names {named.group(1)!r} where {field!r} was expected: {message}"
        )
        return
    # No declared axis: the case puts its fault on exactly one field and gives
    # the other the legal `["*"]`, so naming that one is naming the wrong field.
    assert _refused_rule(case)[named.group(1)] != _UNTESTED_FIELD_VALUE, (
        f"{where}\n  message names {named.group(1)!r}, whose declared value is {_UNTESTED_FIELD_VALUE} — "
        f"legal under every clause of §6.2.1. The fault is on the other field: {message}"
    )


def _assert_refused_axis(case: dict[str, Any], message: str, where: str, axis: str) -> None:
    """§6.2.1 point 2: the rule is bad on more than one axis and *this* one is named.

    ``expected_load`` cannot see which fault a rejection names, so a case that
    is faulty on two axes at once needs this second assertion or the ordering it
    exists to pin is not tested at all. The negative half matters as much as the
    positive one: an implementation running ``effect`` -> patterns ->
    ``approval`` still raises ``ACLRuleError`` for every case here.

    ``effect`` and ``approval`` name **axes**; ``callers`` and ``targets`` name a
    **field within the single pattern axis**, so asserting one of those also
    asserts that the pattern axis is the one that fired.
    """
    if axis == "default_effect":
        # Not a rule axis at all — `default_effect` is not a rule and has no
        # index, so the rule ordering never reaches it and it is judged first.
        named = _NAMED_DEFAULT_EFFECT.search(message)
        assert named is not None, f"{where}\n  expected the refusal to name 'default_effect': {message}"
        assert named.group(1) == case["default_effect"], f"{where}\n  wrong default_effect value named: {message}"
        assert (
            _NAMED_FIELD.search(message) is None and _NAMED_EFFECT.search(message) is None
        ), f"{where}\n  the refusal named a rule axis; 'default_effect' precedes every rule: {message}"
        assert (
            _NAMED_RULE_INDEX.search(message) is None
        ), f"{where}\n  'default_effect' is not a rule and has no index, yet one was named: {message}"
    elif axis == "effect":
        named = _NAMED_EFFECT.search(message)
        assert named is not None, f"{where}\n  expected the refusal to name the 'effect' axis: {message}"
        assert named.group(1) == _refused_rule(case)["effect"], f"{where}\n  wrong effect value named: {message}"
        assert (
            _NAMED_FIELD.search(message) is None
        ), f"{where}\n  the refusal named a pattern field; 'effect' precedes the pattern axis: {message}"
    elif axis == "approval":
        assert (
            _NAMED_APPROVAL.search(message) is not None
        ), f"{where}\n  expected the refusal to name the 'approval' axis: {message}"
        assert (
            _NAMED_FIELD.search(message) is None
        ), f"{where}\n  the refusal named a pattern field; 'approval' precedes the pattern axis: {message}"
    else:
        _assert_pattern_axis(case, message, where, axis)


def _assert_refused_rule_index(case: dict[str, Any], message: str, where: str, *, door_names_index: bool) -> None:
    """§6.2.1 point 2, cross-rule half: the LOWEST-INDEXED bad rule is the one refused.

    An implementation that sweeps one axis across every rule before looking at
    the next passes every single-rule case in this fixture and fails the
    discriminating pair, where rule 0's fault sits on a *later* axis than
    rule 1's.

    Only the loader can *name* the index. A rule under construction has no
    position yet and §6.1.5 forbids inventing one, so at every other door the
    expectation is asserted as the negative it implies — that no index was
    invented — rather than skipped. Which rule was refused stays observable
    there through ``expected_refused_axis``, which is why the fixture's pair
    puts the two rules' faults on different axes.
    """
    expected = case.get("expected_refused_rule_index")
    if expected is None:
        return
    named = _NAMED_RULE_INDEX.search(message)
    if door_names_index:
        assert named is not None, f"{where}\n  expected the refusal to name rule {expected}: {message}"
        assert int(named.group(1)) == expected, (
            f"{where}\n  §6.2.1: the refusal names rule {named.group(1)} where the lowest-indexed "
            f"bad rule is {expected}: {message}"
        )
    else:
        assert named is None, (
            f"{where}\n  this door names no rule index — a rule under construction has no position "
            f"yet and §6.1.5 forbids inventing one — yet the message carries one: {message}"
        )


def _assert_rejected(case: dict[str, Any], build: Any, where: str) -> None:
    """A rejecting case: ``ACLRuleError``, on the axis and the rule the case declares."""
    with pytest.raises(ACLRuleError) as excinfo:
        build(case)
    message = str(excinfo.value)

    _assert_refused_rule_index(
        case,
        message,
        where,
        door_names_index=build in _DOORS_THAT_NAME_THE_RULE_INDEX,
    )

    axis = case.get("expected_refused_axis")
    if axis is None:
        _assert_pattern_axis(case, message, where, None)
        return
    axis = dispatch_or_fail(
        FIXTURE,
        case["id"],
        axis,
        {a: a for a in ("default_effect", "effect", "approval", "callers", "targets")},
        "expected_refused_axis",
    )
    _assert_refused_axis(case, message, where, axis)


def _assert_loaded(case: dict[str, Any], acl: ACL, where: str) -> None:
    """An accepting case: every rule is installed, unchanged and in order.

    "Did not raise" is also what an implementation that drops the rule does, so
    the post-condition is that each array survived the door verbatim — and the
    ORDER survived too, which a multi-rule case is the only thing that can see.
    """
    declared = _raw_rules(case)
    assert len(acl.rules) == len(declared), where
    for index, (installed, raw) in enumerate(zip(acl.rules, declared)):
        assert list(installed.callers) == list(raw["callers"]), f"{where}\n  rule {index} callers"
        assert list(installed.targets) == list(raw["targets"]), f"{where}\n  rule {index} targets"
        assert installed.effect == raw["effect"], f"{where}\n  rule {index} effect"
    assert acl.default_effect == case["default_effect"], where

    expected_findings = case.get("expected_validation_finding_paths")
    if expected_findings is None:
        return
    # Tier 2 (§6.2.1): a well-formed array that still matches no legal module ID
    # is REPORTED, with the same shape as a structural fault — path
    # `callers` / `targets`, a null key, both resolvability flags false.
    findings = acl.validate_rules()
    assert [f.condition_path for f in findings] == expected_findings, (
        f"{where}\n  validate_rules() reported {[f.condition_path for f in findings]}, "
        f"expected exactly {expected_findings}"
    )
    # `expected_validation_finding_paths` states paths without rule indices,
    # which only reads unambiguously while the case carries one rule — every
    # tier-2 case does, and this pins that it stays true.
    assert len(declared) == 1, f"{where}\n  a multi-rule case cannot state finding paths without indices"
    for finding in findings:
        assert finding.rule_index == 0, where
        assert finding.condition_key is None, where
        assert finding.sync_resolvable is False, where
        assert finding.async_resolvable is False, where


def _run_closure(case: dict[str, Any]) -> None:
    reject_unknown_expectations(
        FIXTURE,
        case,
        {
            "expected_load",
            "expected_validation_finding_paths",
            "expected_refused_axis",
            "expected_refused_rule_index",
        },
    )
    expected = dispatch_or_fail(
        FIXTURE,
        case["id"],
        case["expected_load"],
        {"ok": "ok", "reject": "reject"},
        "expected_load",
    )
    if expected == "reject":
        # A rejected rule never reaches `validate_rules()`, so a finding
        # expectation on one could not be asserted and must not read as covered.
        assert (
            "expected_validation_finding_paths" not in case
        ), f"[{FIXTURE} :: {case['id']}] states a validation finding on a rule rejected at every door"

    for door in _entry_points(case):
        for build in _DOORS[door]:
            where = f"{case['note']}\n  door={door} ({build.__name__})"
            if expected == "ok":
                _assert_loaded(case, build(case), where)
            else:
                _assert_rejected(case, build, where)


# ---------------------------------------------------------------------------
# `kind: backstop` — the route no door covers
# ---------------------------------------------------------------------------

_BACKSTOP_EXPECTATIONS = {
    "expected_access",
    "expected_legacy_check",
    "expected_approval_required",
    "expected_matched_rule_index",
    "expected_audit_handler_error_present",
    "expected_handler_error_paths",
    "expected_validation_finding_paths",
}


def _mutations(case: dict[str, Any]) -> list[dict[str, Any]]:
    """``mutate`` as a list; the fixture states one entry or several."""
    mutate = case.get("mutate")
    if mutate is None:
        return []
    return list(mutate) if isinstance(mutate, list) else [mutate]


def _reported_paths(handler_error: str) -> list[str]:
    """Split a ``handler_error`` into the paths it names, in order.

    Each part is ``"<path>: <reason>"`` and a reason may itself contain
    ``": "``, so only the first separator delimits the path. §6.1.1 rule 2 makes
    ``"; "`` the separator between parts, which is why no reason phrase may
    contain one.
    """
    return [part.split(": ", 1)[0] for part in handler_error.split("; ")]


def _run_backstop(case: dict[str, Any]) -> None:
    reject_unknown_expectations(FIXTURE, case, _BACKSTOP_EXPECTATIONS)
    dispatch_or_fail(
        FIXTURE,
        case["id"],
        case["mutation_route"],
        {"installed_rule": True},
        "mutation_route",
    )

    captured: list[AuditEntry] = []
    acl = ACL(
        rules=[ACLRule(**_rule_kwargs(case["rule"]))],
        default_effect=case["default_effect"],
        audit_logger=captured.append,
    )

    # Reached through the public accessor on purpose: `mutation_route:
    # installed_rule` requires mutating a rule that is ALREADY IN the ACL, and
    # `ACL.rules` handing back the live dataclass is exactly the route
    # apcore-rust does not have (`&[ACLRule]`, no `rules_mut`).
    installed = acl.rules[0]
    for mutation in _mutations(case):
        setattr(installed, mutation["field"], mutation["value"])

    note = case["note"]

    decision = acl.check_access(case["caller_id"], case["target_id"])
    assert (
        decision.access == case["expected_access"]
    ), f"{note}\n  structured access was {decision.access!r}, expected {case['expected_access']!r}"
    if "expected_approval_required" in case:
        assert decision.approval_required is case["expected_approval_required"], note
    if "expected_matched_rule_index" in case:
        assert decision.matched_rule_index == case["expected_matched_rule_index"], note

    # §6.3.1: exactly one audit entry per check, read before the legacy call
    # below emits its own.
    assert len(captured) == 1, f"{note}\n  expected exactly one audit entry, got {len(captured)}"
    entry = captured[0]
    if case["expected_audit_handler_error_present"]:
        assert entry.handler_error is not None, f"{note}\n  expected a non-null handler_error"
        assert _reported_paths(entry.handler_error) == case["expected_handler_error_paths"], (
            f"{note}\n  handler_error {entry.handler_error!r} does not name exactly "
            f"{case['expected_handler_error_paths']} in order"
        )
    else:
        assert entry.handler_error is None, f"{note}\n  handler_error was {entry.handler_error!r}, expected null"
        assert case["expected_handler_error_paths"] == [], note

    # The SECOND decision surface. §6.8.1 makes the boolean fail-closed for
    # allow-with-approval-required, so it is not `access == "allow"` and a
    # driver that treats it as such fails exactly one case here.
    legacy = acl.check(case["caller_id"], case["target_id"])
    assert (
        legacy is case["expected_legacy_check"]
    ), f"{note}\n  legacy check() returned {legacy!r}, expected {case['expected_legacy_check']!r}"

    findings = acl.validate_rules()
    assert [f.condition_path for f in findings] == case["expected_validation_finding_paths"], (
        f"{note}\n  validate_rules() reported {[f.condition_path for f in findings]}, "
        f"expected exactly {case['expected_validation_finding_paths']}"
    )
    for finding in findings:
        # §6.1.3 rule 3's keyless structural fault, tier 1 and tier 2 alike.
        assert finding.condition_key is None, note
        assert finding.sync_resolvable is False, note
        assert finding.async_resolvable is False, note


_KINDS = {"closure": _run_closure, "backstop": _run_backstop}


@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["id"])
def test_acl_pattern_arity(case: dict[str, Any]) -> None:
    run = dispatch_or_fail(FIXTURE, case["id"], case["kind"], _KINDS, "case kind")
    run(case)


def test_every_case_names_at_least_one_door_or_a_mutation_route() -> None:
    """A case that states neither would assert nothing while reading as covered."""
    for case in _cases():
        if case["kind"] == "closure":
            assert case["entry_points"], case["id"]
        else:
            assert case["mutation_route"], case["id"]


def test_both_case_kinds_are_present() -> None:
    """The two tiers are different mechanisms; a fixture reduced to one is a regression."""
    kinds = {case["kind"] for case in _cases()}
    assert kinds == set(_KINDS), kinds


#: Every loader refusal that carries a rule index, as ``(axis, rule body)``.
#: The rule below is placed at index **1**, so a driver regex that only matches
#: rule 0, or that misses a spelling entirely, fails here rather than reading
#: the refusal as naming no rule at all.
_LOADER_AXES_THAT_NAME_AN_INDEX = [
    ("unknown_key", {"callers": ["*"], "targets": ["*"], "effect": "allow", "priority": 3}),
    ("missing_field", {"targets": ["*"], "effect": "allow"}),
    ("not_a_mapping", "not a rule"),
    ("effect", {"callers": ["*"], "targets": ["*"], "effect": "Allow"}),
    ("approval", {"callers": ["*"], "targets": ["*"], "effect": "deny", "approval": "required"}),
    ("pattern_type", {"callers": "admin.*", "targets": ["*"], "effect": "allow"}),
    ("pattern_shape", {"callers": [], "targets": ["*"], "effect": "allow"}),
    ("conditions_type", {"callers": ["*"], "targets": ["*"], "effect": "allow", "conditions": "roles"}),
]


@pytest.mark.parametrize(
    ("axis", "bad_rule"),
    _LOADER_AXES_THAT_NAME_AN_INDEX,
    ids=[a for a, _ in _LOADER_AXES_THAT_NAME_AN_INDEX],
)
def test_every_loader_refusal_spells_the_rule_index_the_same_way(axis: str, bad_rule: Any) -> None:
    """The trap :data:`_NAMED_RULE_INDEX` can walk into, pinned rather than assumed.

    ``expected_refused_rule_index`` is asserted by reading the index back out of
    the message, so the regex has to accept **every** spelling this SDK's doors
    use. The two axis families are raised from different code paths — the
    per-rule validators go through ``where=f"Rule {i}"`` while the rule-key
    closure builds its own message — and in apcore-rust those two paths worded
    the index differently, so a refusal on the loader-only key axis matched
    nothing and read as naming *no rule at all*: a silent false pass on the one
    case built to catch a real bug.

    apcore-python spells it ``Rule <n>`` everywhere, which is what this asserts
    — on rule **1**, where a regex anchored to rule 0 or an off-by-one would
    read as correct.
    """
    doc = {
        "default_effect": "deny",
        "rules": [{"callers": ["api.*"], "targets": ["executor.*"], "effect": "allow"}, bad_rule],
    }
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump(doc))
        with pytest.raises(ACLRuleError) as excinfo:
            ACL.load(str(path))

    message = str(excinfo.value)
    named = _NAMED_RULE_INDEX.search(message)
    assert named is not None, (
        f"the {axis} axis names no rule index this driver can read, so a case asserting "
        f"expected_refused_rule_index would pass on it whatever index it meant: {message}"
    )
    assert int(named.group(1)) == 1, f"expected the refusal to name rule 1: {message}"


def test_a_default_effect_refusal_names_no_rule_index() -> None:
    """`default_effect` is not a rule and has no index; none may be invented.

    Asserted as its own test as well as inside the axis branch, because "no
    index present" is the half a driver silently defaults to 0 instead of
    checking.
    """
    doc = {"default_effect": "Allow", "rules": [{"callers": [], "targets": ["*"], "effect": "allow"}]}
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump(doc))
        with pytest.raises(ACLRuleError) as excinfo:
            ACL.load(str(path))

    message = str(excinfo.value)
    assert _NAMED_DEFAULT_EFFECT.search(message) is not None, message
    assert _NAMED_RULE_INDEX.search(message) is None, f"a default_effect refusal named a rule index: {message}"


def test_the_loader_message_names_the_rule_index() -> None:
    """§6.2.1: the rejection names the field **and** the rule index where the door has one.

    A rule under construction has no position yet and an implementation MUST NOT
    invent one, so the index is asserted on the one door that knows it — and on
    rule 1 rather than rule 0, where an off-by-one would read as correct.
    """
    doc = {
        "default_effect": "deny",
        "rules": [
            {"callers": ["agent.*"], "targets": ["orders.*"], "effect": "allow"},
            {"callers": ["agent.*"], "targets": [], "effect": "allow"},
        ],
    }
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump(doc))
        with pytest.raises(ACLRuleError, match=r"Rule 1 has an invalid 'targets'"):
            ACL.load(str(path))
