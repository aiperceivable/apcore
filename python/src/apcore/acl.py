"""ACL (Access Control List) types and implementation for apcore.

This module defines the ACLRule dataclass and the ACL class that enforces
pattern-based access control between modules.
"""

from __future__ import annotations

import contextvars
import inspect
import logging
import os
import threading
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, ClassVar

import yaml

from apcore.acl_handlers import (
    ACLConditionHandler,
    ConditionOutcome,
    ROOT_CONDITION_PATH,
    _as_outcome,
    _ArgumentsHandler,
    _IdentityTypesHandler,
    _MaxCallDepthHandler,
    _NotHandler,
    _NotHandlerAsync,
    _OrHandler,
    _OrHandlerAsync,
    _projection_var,
    _RolesHandler,
    join_condition_path,
    validate_arguments_condition,
)
from apcore.config import Config
from apcore.context import Context
from apcore.errors import ACLRuleError, ConfigError, ConfigNotFoundError
from apcore.utils.pattern import match_pattern

__all__ = [
    "AUDIT_EVENT_NAME",
    "AccessDecision",
    "ACLRule",
    "AuditEntry",
    "ACL",
    "ConditionOutcome",
    "RuleValidationFinding",
]

_logger = logging.getLogger(__name__)

# Surfaces unevaluable conditions (PROTOCOL_SPEC §6.1.1) into the AuditEntry
# built for the current check() / async_check() invocation, keyed by condition
# **path** (§6.1.4) so the audit message can be ordered lexicographically as
# §6.1.1 rule 2 requires. By path and not by key, because a key may occur at
# several positions in a nested $or / $not tree, which leaves ordering by key
# undefined. Installed fresh at the start of each public check so nested calls
# do not leak diagnostics across audit entries.
#
# The dict is mutated in place rather than re-``set``: a ContextVar assignment
# made inside a coroutine does not necessarily propagate back to the caller's
# context, while a mutation of the shared mapping always does.
_handler_error_var: contextvars.ContextVar[dict[str, str] | None] = contextvars.ContextVar(
    "_apcore_acl_handler_error", default=None
)

# `_projection_var` and `current_governance_projection` live in apcore.acl_handlers
# now — next to `_ArgumentsHandler`, their only reader — so that
# apcore.acl -> apcore.acl_handlers stays a one-way dependency (imported above).

# Condition paths for the rule's pattern fields (PROTOCOL_SPEC §6.1.4).
_CALLERS_PATH = "callers"
_TARGETS_PATH = "targets"


def _record_handler_error(path: str, reason: str) -> None:
    """Record an unevaluable condition for the in-flight check(), if any.

    Keyed by §6.1.4 condition path. The first reason recorded for a path wins:
    the same path can be reached by several rules in one check, and a stable
    diagnostic beats a last-writer one. A no-op when no check is in flight, so
    the evaluator stays usable directly.
    """
    errors = _handler_error_var.get()
    if errors is not None and path not in errors:
        errors[path] = f"{path}: {reason}"


def _handler_error_message() -> str | None:
    """Render the recorded diagnostics for the AuditEntry, or None.

    PROTOCOL_SPEC §6.1.1 rule 2: every unevaluable condition MUST be reported,
    ordered **lexicographically by condition path** and separated by ``"; "``.
    By path rather than evaluation order because the two differ across languages
    (``serde_json``'s map is ordered; Python dicts and JS objects preserve
    insertion order), and by path rather than key because a nested ``$or`` may
    carry one key at several positions.
    """
    errors = _handler_error_var.get()
    if not errors:
        return None
    return "; ".join(errors[path] for path in sorted(errors))


#: The two values PROTOCOL_SPEC §6.1.6 gives the ``approval`` rule field.
APPROVAL_REQUIRED = "required"
APPROVAL_NOT_REQUIRED = "not_required"
_APPROVAL_VALUES: frozenset[str] = frozenset({APPROVAL_REQUIRED, APPROVAL_NOT_REQUIRED})

#: The complete set of values PROTOCOL_SPEC §6.1's field table gives ``effect``
#: — and ``default_effect``, which §6.1.5 closes on the same terms. One set for
#: both fields on purpose: they are the same two values one level apart, and two
#: copies of a value set are two things that drift.
_EFFECT_VALUES: frozenset[str] = frozenset({"allow", "deny"})


def _validate_effect(effect: Any, *, where: str) -> None:
    """Reject an ``effect`` outside :data:`_EFFECT_VALUES` (§6.1.5, spec v1.30.0).

    One function for all three doors §6.1.6 rule 3 names — file loading, direct
    construction, runtime insertion. Before spec v1.30.0 this check lived inline
    in :meth:`ACL.load` and nowhere else, so ``effect: "Allow"`` (the
    capitalisation an operator writes by hand) failed from a YAML file and was
    *accepted* through ``ACLRule(...)`` and :meth:`ACL.add_rule` — then read as
    ``deny`` at check time, which under ``default_effect: allow`` turns a rule
    written to permit into one that denies everything it matches, silently.

    Args:
        effect: The rule's ``effect`` value.
        where: How to name the rule in the message — ``"Rule 3"`` from the
            loader, ``"ACLRule"`` from direct construction. Same convention as
            :func:`_validate_approval`; the message is byte-identical to the
            loader's historical one, which apcore-typescript and apcore-rust
            also emit.

    Raises:
        ACLRuleError: When *effect* is not ``"allow"`` or ``"deny"``.
    """
    if effect not in _EFFECT_VALUES:
        raise ACLRuleError(f"{where} has invalid effect '{effect}', must be 'allow' or 'deny'")


def _validate_default_effect(default_effect: Any) -> None:
    """Reject a ``default_effect`` outside :data:`_EFFECT_VALUES` (§6.1.5).

    **Judged FIRST, before any rule, at every door** (§6.2.1, spec v1.31.0).
    ``default_effect`` is not a rule and has no index, so the rule ordering
    never reaches it — and a configuration wrong in both ``default_effect`` and
    a rule is exactly the one-file-one-error case the ordering exists for. Left
    unstated before v1.31.0, and implementations differed.

    "First" is ahead of the **file-level checks on the ``rules`` collection
    itself**, not merely ahead of the individual rules: a document both missing
    ``rules`` and carrying an unrecognised ``default_effect`` is refused for the
    ``default_effect``. No conformance case covers that combination, so
    ``TestDefaultEffectIsJudgedBeforeAnyRule`` in ``tests/test_acl.py`` is the
    only guard on it.

    One function for both doors that accept one — :meth:`ACL.load` and
    :meth:`ACL.__init__` — for the same reason :func:`_validate_effect` is one
    function: the loader used to leave this entirely to the constructor, which
    runs *after* the whole rule list has been parsed and validated, so a file
    carrying a bad ``default_effect`` and a bad rule 0 named the rule.

    Args:
        default_effect: The configuration's ``default_effect`` value.

    Raises:
        ACLRuleError: When it is not ``"allow"`` or ``"deny"``.
    """
    if default_effect not in _EFFECT_VALUES:
        raise ACLRuleError(
            f"Invalid default_effect '{default_effect}': must be 'allow' or 'deny'. "
            "Cross-language parity with apcore-typescript constructor validation (sync A-D-025)."
        )


#: The compound-operator tokens PROTOCOL_SPEC §6.2.1 reserves for **index 0**
#: of a ``callers`` / ``targets`` array. Detection is by **equality** and never
#: by a ``$`` prefix or a substring: ``$orders.*`` is an ordinary pattern that
#: merely begins with the same character, and MUST load.
_PATTERN_OPERATORS: frozenset[str] = frozenset({"$or", "$not"})

#: The remedy every operand-less array needs, appended to the three arity
#: messages. Both readings of an empty array are plausible and the failure is
#: now at boot, where a good message is the whole remedy (§6.2.1).
#:
#: No ``"; "`` anywhere in this string, or in any reason phrase
#: :func:`_pattern_array_fault` returns: §6.1.1 rule 2 makes ``"; "`` the
#: separator between the diagnostics an ``AuditEntry``'s ``handler_error``
#: carries, so a reason containing one splits into two unparseable entries.
_EMPTY_ARITY_REMEDY = (
    'Write ["*"] if "everything" was meant. A rule that was meant to match nothing is not a rule — delete it.'
)


def _pattern_array_fault(patterns: Any) -> str | None:
    """Return why *patterns* is outside §6.2.1's closed shape, or None (spec v1.31.0).

    The single structural predicate behind every door — :meth:`ACL.load`,
    :meth:`ACLRule.__post_init__` and therefore :meth:`ACL.add_rule` — and
    behind :meth:`ACL._precheck_patterns`, which is the backstop for the one
    route no door covers. One predicate on purpose, for the reason
    :func:`_validate_effect` is one function: two copies of a value set are two
    things that drift, which is how the loader came to be the only door that
    checked ``effect``.

    A pattern array is **FLAT** (§6.2.1). The operators do not nest, there is no
    precedence, an operand is always a plain pattern string, and there is
    exactly one operator position — index 0. So the shape is finite, decidable
    with no context and no registry, and identical in all three languages:

    1. the array MUST NOT be empty;
    2. every element MUST be a non-empty string;
    3. ``$or`` at index 0 MUST be followed by at least one pattern;
    4. ``$not`` at index 0 MUST be followed by **exactly** one;
    5. ``$or`` / ``$not`` MUST NOT appear at any index other than 0.

    Through spec v1.30.0 none of this was enforced anywhere, while
    ``schemas/acl-config.schema.json`` had declared ``minItems: 1`` and
    ``minLength: 1`` since the file existed — the third instance of #107's and
    #111's shape, in which the normative artefact declared the constraint and no
    entry point enforced it. The matcher read the arity fault as a *scope*
    decision and returned False, which makes an ``allow`` rule merely useless
    and a ``deny`` rule a **fail-open**: under ``default_effect: allow`` the call
    the operator wrote the rule to block was permitted, by a rule that loaded
    without error and a validator that called it clean.

    Args:
        patterns: The ``callers`` or ``targets`` value to inspect.

    Returns:
        A reason phrase naming the fault, or None when the shape is legal. The
        phrase carries no field name and no rule index, so the door can prefix
        both and the precheck can hand it to ``handler_error`` under the
        field's own §6.1.4 path.
    """
    if not isinstance(patterns, list) or not all(isinstance(pattern, str) for pattern in patterns):
        # The §6.1.4.1 *type* fault, which this predicate deliberately does not
        # own. It is reported by the precheck rather than rejected at the door
        # (``ACLRule(callers="a")`` still constructs), it is unrepresentable in
        # apcore-rust's ``Vec<String>``, and an array whose elements are not
        # strings has no meaningful arity reading anyway.
        return None

    if not patterns:
        return f"a pattern array MUST carry at least one operand and this one is empty. {_EMPTY_ARITY_REMEDY}"

    for index, pattern in enumerate(patterns):
        if not pattern:
            return (
                f"the pattern at index {index} is the empty string, which matches no legal module ID. "
                "The schema has always declared minLength: 1 on both fields' items."
            )

    for index, pattern in enumerate(patterns[1:], start=1):
        if pattern in _PATTERN_OPERATORS:
            return (
                f"the reserved token {pattern!r} appears at index {index}. A pattern array is FLAT — the "
                "operators do not nest and there is no precedence — so '$or' / '$not' are operators at "
                "index 0 and nowhere else, and every element after it is a plain pattern. "
                "['$or', '$not', 'a'] is not \"or-of-not\" and ['api.*', '$not', 'cli.*'] is not "
                '"api.* but not cli.*" — neither form exists. Through spec v1.30.0 the token was read as '
                "a literal pattern, which this section's own reserved-token MUST NOT guarantees can "
                "never match — dead weight in a security policy."
            )

    operands = len(patterns) - 1
    if patterns[0] == "$or" and operands < 1:
        return f"'$or' at index 0 MUST be followed by at least one pattern, and none was given. {_EMPTY_ARITY_REMEDY}"
    if patterns[0] == "$not" and operands != 1:
        if operands == 0:
            return (
                "'$not' at index 0 MUST be followed by exactly one pattern, and none was given. "
                f"{_EMPTY_ARITY_REMEDY}"
            )
        return (
            f"'$not' at index 0 MUST be followed by exactly one pattern, got {operands}. Through spec "
            "v1.30.0 the form was implementation-defined and every SDK consulted the first operand and "
            "dropped the rest, so ['$not', p1, p2] silently meant ['$not', p1] and an 'allow' rule "
            "GRANTED p2 — the second target the operator excluded. There is no mechanical rewrite: "
            "['$not', p1] preserves what the rule has actually been doing, while \"neither p1 nor p2\" "
            "needs a leading 'deny' rule, which ENDS the scan where a non-matching rule lets it continue "
            "to later rules. Rewrite by hand, against the rule's position."
        )
    return None


def _validate_pattern_array(patterns: Any, field: str, *, where: str) -> None:
    """Reject a ``callers`` / ``targets`` array outside §6.2.1's shape (spec v1.31.0, #112).

    One function for all three doors §6.1.6 rule 3 names — file loading, direct
    construction, runtime insertion — threaded exactly as
    :func:`_validate_effect` is, and for the same reason: the closure has to be
    per *entry point* rather than per code path, or a shape rejected from a YAML
    file is accepted through ``ACLRule(...)``.

    Args:
        patterns: The field's value.
        field: ``"callers"`` or ``"targets"`` — §6.2.1 constrains both
            identically, and an implementation that validates one and infers the
            other is the defect the conformance fixture's ``*_in_callers_*``
            mirrors exist to catch.
        where: How to name the rule — ``"Rule 3"`` from the loader, ``"ACLRule"``
            from direct construction. A rule under construction has no position
            yet and §6.2.1 forbids inventing one, which is why the caller
            supplies this rather than the function guessing.

    Raises:
        ACLRuleError: When the array's shape is outside §6.2.1's closure.
    """
    reason = _pattern_array_fault(patterns)
    if reason is not None:
        raise ACLRuleError(f"{where} has an invalid '{field}' (PROTOCOL_SPEC §6.2.1): {reason}")


def _validate_approval(approval: Any, effect: Any, *, where: str) -> None:
    """Enforce §6.1.6 on one rule's ``approval`` / ``effect`` pair.

    Authorization and the approval requirement are two independent results, so
    a rule carries both — but ``approval: required`` on a ``deny`` rule names a
    state that means nothing ("denied *and* put it to a human"), and acting on
    half of a governance rule is the failure mode §6.1.5 was written to end.

    Args:
        approval: The rule's ``approval`` value.
        effect: The rule's ``effect`` value.
        where: How to name the rule in the message — ``"Rule 3"`` from the
            loader, ``"ACLRule"`` from direct construction.

    Raises:
        ACLRuleError: On an unknown value or on ``required`` with ``deny``.
    """
    if approval not in _APPROVAL_VALUES:
        raise ACLRuleError(
            f"{where} has invalid approval {approval!r}, must be " f"{APPROVAL_REQUIRED!r} or {APPROVAL_NOT_REQUIRED!r}"
        )
    if approval == APPROVAL_REQUIRED and effect == "deny":
        raise ACLRuleError(
            f"{where} carries approval: {APPROVAL_REQUIRED} on a 'deny' rule. The combination has "
            "no meaning — a refusal is not a question — and PROTOCOL_SPEC §6.1.6 requires it to be "
            "rejected rather than half-applied. Use effect: allow with approval: required to ask a "
            "human, or effect: deny to refuse outright."
        )


def _validate_rule(rule: ACLRule, *, where: str) -> None:
    """Validate one whole rule, in the order §6.2.1 fixes (spec v1.31.0, #112).

    **The order is normative, and §6.2.1 states it for the first time.** A rule
    that is bad on more than one axis MUST be refused for the first of
    ``effect`` -> ``approval`` -> ``callers`` / ``targets`` that it fails, so
    the same rule produces the same error in every implementation. §6.1.6 rule 2
    *implies* that ``effect`` is read before ``approval`` — judging "``deny``
    plus ``approval: required``" requires knowing the effect — but it states no
    order, which is why three implementations had three: ``{callers: [],
    targets: [], effect: "Allow"}`` was refused for its ``effect`` in one and
    for its patterns in another, and a third ran ``effect`` -> patterns ->
    ``approval``. All were conformant, because nothing said otherwise.

    ``effect`` comes first because the pairing rule below it is read *off* the
    effect: ``effect: "DENY"`` with ``approval: required`` is not a ``deny``
    rule, so reporting the pairing would name a rule the specification does not
    recognise.

    **The pattern fields are ONE axis**, covering §6.1.4.1's *type* fault and
    §6.2.1's *shape* closure together, with the type fault first — a value must
    be a list of strings before its arity means anything — and ``callers``
    before ``targets``. Only the shape half is a door rejection here; the type
    fault is unrepresentable in apcore-rust, so it is reported by
    :meth:`ACL._precheck_patterns`, which gives it the same precedence within
    the same field order.

    **Rule index dominates all three axes** (§6.2.1). A rule *set* with more
    than one bad rule MUST be refused for the LOWEST-INDEXED bad rule, and an
    implementation MUST NOT sweep one axis across every rule before looking at
    the next. That is a property of the callers rather than of this function:
    :meth:`ACL.load` validates rule by rule in file order, and ``ACL(rules=[...])``
    validates each :class:`ACLRule` as it is constructed, so both refuse the
    first bad rule rather than the first bad axis. Sweeping is how one
    implementation came to report a lower-indexed rule's pattern fault from its
    loader and a higher-indexed rule's ``effect`` fault from its constructor,
    for the same file.

    **One function, called at every door**, which is §6.2.1's first point of
    order: a rule offered to :meth:`ACL.add_rule` MUST be validated *at that
    moment*, whatever its history — including a rule that was
    well-formed when constructed and has since had ``callers`` or ``targets``
    assigned — and an implementation MUST NOT rely on the rule type's own
    construction-time check to cover that door. Threading the check through
    :meth:`ACLRule.__post_init__` alone reaches ``add_rule`` only *through
    construction*, which the mutated rule walks straight past.

    Args:
        rule: The rule to validate. Read only; nothing is mutated or normalised.
        where: How to name the rule in the message — ``"Rule 3"`` from the
            loader, ``"ACLRule"`` from direct construction and from runtime
            insertion. A rule under construction has no position yet and §6.1.5
            forbids inventing one.

    Raises:
        ACLRuleError: On the first axis the rule fails.
    """
    _validate_effect(rule.effect, where=where)
    _validate_approval(rule.approval, rule.effect, where=where)
    _validate_pattern_array(rule.callers, _CALLERS_PATH, where=where)
    _validate_pattern_array(rule.targets, _TARGETS_PATH, where=where)


@dataclass
class ACLRule:
    """A single access control rule.

    Rules are evaluated in order by the ACL system. Each rule specifies
    caller patterns, target patterns, and an effect (allow/deny).

    ``effect`` and ``approval`` answer two **independent** questions
    (PROTOCOL_SPEC §6.1.6): may this caller reach this target at all, and must
    this particular call be put to a human before it runs. ``approval`` defaults
    to ``"not_required"``, so every rule written before spec v1.28.0 keeps its
    meaning exactly.

    ``effect`` is ``"allow"`` or ``"deny"`` and nothing else (§6.1.5). The
    annotation says ``str`` because the field is a string, but the set is closed
    and enforced here, at every door that accepts a rule.
    """

    callers: list[str]
    targets: list[str]
    effect: str
    description: str = ""
    conditions: dict[str, Any] | None = None
    approval: str = APPROVAL_NOT_REQUIRED

    def __post_init__(self) -> None:
        """Reject an out-of-enum ``effect`` (§6.1.5), a meaningless ``deny`` + ``approval`` pair (§6.1.6), and a pattern array outside §6.2.1's closed shape.

        Validated on the dataclass rather than only in :meth:`ACL.load` because
        ``ACL(rules=[...])`` and :meth:`ACL.add_rule` never reach the loader's
        parser — the same door §6.1.1 case 5 and §6.1.4.1 exist for. The loader
        still checks first, so a file fault names the rule's index.

        The order — ``effect`` -> ``approval`` -> ``callers`` / ``targets`` —
        is normative (§6.2.1) and lives in :func:`_validate_rule`, which
        :meth:`ACL.add_rule` calls on the rule it is *handed* rather than only
        on the one it builds. Construction is one door of three, and it is the
        only one this hook covers.

        The pattern check is deliberately silent on the §6.1.4.1 *type* fault:
        ``ACLRule(callers="admin.*")`` still constructs and is caught at
        evaluation by the precheck, because that fault is unrepresentable in
        apcore-rust and rejecting it here would split the SDKs on which
        configurations exist.
        """
        _validate_rule(self, where="ACLRule")


#: The complete set of keys an ACL rule may carry (PROTOCOL_SPEC §6.1).
#: Closed on purpose: a key nothing evaluates is otherwise dropped in silence,
#: which widens an ``allow`` rule with no warning (#107). ``approval`` joined
#: the set in spec v1.28.0 (§6.1.6) — adding it was only safe *because* the set
#: was closed first, since an SDK that still dropped unknown keys would read a
#: ``deny``-with-``approval`` rule as a bare rule and act on half of it.
_RULE_KEYS: frozenset[str] = frozenset({"callers", "targets", "effect", "description", "conditions", "approval"})

#: Reserved in earlier revisions of §6.1 and evaluated by no implementation.
#: Rejected like any other unknown key, but named as reserved in the message:
#: an operator who wrote ``actions: ["describe"]`` meant to restrict the rule and
#: is better served by "not implemented" than by "unknown key".
_RESERVED_RULE_KEYS: frozenset[str] = frozenset({"id", "actions", "priority"})


def _reject_unknown_rule_keys(index: int, raw_rule: dict) -> None:
    """Raise ``ACLRuleError`` for any key outside :data:`_RULE_KEYS`."""
    unknown = sorted(set(raw_rule) - _RULE_KEYS)
    if not unknown:
        return
    reserved = [k for k in unknown if k in _RESERVED_RULE_KEYS]
    other = [k for k in unknown if k not in _RESERVED_RULE_KEYS]
    parts = []
    if reserved:
        names = ", ".join(repr(k) for k in reserved)
        parts.append(f"{names} reserved for a future specification version and evaluated " f"by no implementation")
    if other:
        parts.append(f"{', '.join(repr(k) for k in other)} unrecognised")
    raise ACLRuleError(
        f"Rule {index} carries {'; '.join(parts)}. The rule key set is closed "
        f"({', '.join(sorted(_RULE_KEYS))}); a key nothing evaluates would be "
        f"dropped silently and leave the rule wider than written."
    )


@dataclass(frozen=True)
class AuditEntry:
    """Structured record of an ACL check decision."""

    timestamp: str  # ISO 8601
    caller_id: str
    target_id: str
    decision: str  # "allow" or "deny"
    reason: str  # "rule_match", "default_effect", "no_rules"
    matched_rule: str | None = None  # Rule description (immutable snapshot)
    matched_rule_index: int | None = None
    identity_type: str | None = None
    roles: tuple[str, ...] = field(default_factory=tuple)
    call_depth: int | None = None
    trace_id: str | None = None
    # PROTOCOL_SPEC §6.3.1: non-null IF AND ONLY IF a condition was unevaluable
    # (§6.1.1) — no registered handler, the handler raised, or an async handler
    # could not be resolved on the sync check() path. It MUST stay null for an
    # ordinary UNSATISFIED condition: that distinction is what makes "the
    # handler said no" and "no answer was obtainable" tellable apart after the
    # fact. Several unevaluable conditions in one check are joined with "; " in
    # lexicographic order of condition key.
    handler_error: str | None = None
    # PROTOCOL_SPEC §6.3.1: whether the matched rule required this call to be
    # put to a human (§6.1.6). False when no rule matched or the matched rule
    # required none. Added **beside** ``decision`` rather than widening it:
    # ``decision`` is a string downstream consumers parse, and a third value
    # would break every existing parser.
    approval_required: bool = False


@dataclass(frozen=True)
class AccessDecision:
    """The structured result of an ACL check (PROTOCOL_SPEC §6.8.1).

    A boolean can carry authorization but not the second axis of §6.1.6, so the
    structured accessors :meth:`ACL.check_access` and
    :meth:`ACL.async_check_access` return this instead. The boolean entry points
    are kept and unchanged in name.

    Attributes:
        access: ``"allow"`` or ``"deny"`` — the authorization verdict, with the
            same semantics the boolean always had.
        approval_required: Whether **this call** must be put to a human before
            it runs. Only ever true alongside ``access == "allow"``: §6.1.6
            rejects the other combination at load.
        matched_rule_index: Index of the deciding rule in definition order, or
            None when the decision came from ``default_effect``.
        reason: ``"rule_match"``, ``"default_effect"`` or ``"no_rules"`` — which
            branch of §6.3 produced the decision.

    Note:
        ``access == "allow"`` is **not** the legacy boolean. A call that is
        allowed but needs approval makes :meth:`ACL.check` return ``False``
        (§6.8.1), because a non-Executor caller can only read a boolean as "let
        it through". Read :attr:`access` and :attr:`approval_required`
        separately, or call ``check()`` and accept its fail-closed answer.
    """

    access: str
    approval_required: bool = False
    matched_rule_index: int | None = None
    reason: str = "default_effect"


@dataclass(frozen=True)
class RuleValidationFinding:
    """One structural or registry fault found by the §6.1.4 precheck.

    Returned by :meth:`ACL.validate_rules` (PROTOCOL_SPEC §6.1.2 rule 3).

    Attributes:
        rule_index: Index of the offending rule in definition order.
        condition_path: Where the fault sits (§6.1.4) — ``roles``,
            ``$or[1].mispelled``, ``$`` for a non-mapping ``conditions``, or
            ``callers`` / ``targets`` for a malformed pattern field.
        condition_key: The key itself, for readers who do not need the path, or
            None for a fault that has no key — a malformed pattern field, a
            non-mapping ``conditions``, or a malformed ``$or`` element.
        effect: The rule's effect. A finding on a ``deny`` rule is the
            consequential one — that rule now denies every call it matches.
        sync_resolvable: Whether the condition resolves for :meth:`ACL.check`.
            False for a keyless structural fault.
        async_resolvable: Whether it resolves for :meth:`ACL.async_check`.
            False for a keyless structural fault.

    The two flags are reported separately and MUST NOT be collapsed into one
    boolean (§6.1.3). They mean **resolvable on that evaluation path**, not
    "present in that registry": ``async_check()`` falls back to the sync
    registry, so ``async_resolvable`` is the union of both and every built-in
    leaf handler is resolvable on both paths. A finding with
    ``sync_resolvable=False, async_resolvable=True`` is an async-only handler —
    usable under ``async_check()``, unevaluable under ``check()``.
    """

    rule_index: int
    condition_path: str
    condition_key: str | None
    effect: str
    sync_resolvable: bool
    async_resolvable: bool


@dataclass(frozen=True)
class _Fault:
    """An internal precheck finding, before it is bound to a rule index.

    ``key`` is None for a fault that is not attached to a condition key — a
    malformed pattern field, a non-mapping ``conditions``, a malformed ``$or``
    element. The two flags mean "can this fault be resolved by evaluating on
    that path", not "is the key present in that registry": a structural fault
    resolves on neither, which is why they default to False. Reading them as a
    registry lookup would report a malformed ``$or`` *value* as resolvable on
    both paths, since ``$or`` itself has a handler.
    """

    path: str
    key: str | None
    reason: str
    sync_resolvable: bool = False
    async_resolvable: bool = False


# ---------------------------------------------------------------------------
# Audit delivery (PROTOCOL_SPEC §6.3.2, apcore#118 decision D-66)
# ---------------------------------------------------------------------------

#: The three settings of an ACL file's `audit:` block, and their defaults.
_AUDIT_FIELDS = ("enabled", "include_denied", "log_level")
_AUDIT_DEFAULTS: dict[str, Any] = {"enabled": True, "include_denied": True, "log_level": "info"}
_AUDIT_LEVELS = {
    "trace": logging.DEBUG,
    "debug": logging.DEBUG,
    "info": logging.INFO,
    "warn": logging.WARNING,
    "error": logging.ERROR,
}

#: The stable name the default sink emits under (§10.3's event table).
AUDIT_EVENT_NAME = "apcore.acl.audit"


class _AuditSink:
    """The ONE effective sink for an ACL, per §6.3.2 requirement 1.

    Never two. A callback supplied to the constructor receives every entry and
    is not narrowed, levelled or silenced by the `audit:` block: an API argument
    beats configuration, and the alternative lets a file silently truncate a
    compliance sink a developer installed deliberately. The block configures the
    **default sink** and nothing else.

    A sink also owns its failure state. Requirement 5 suppresses diagnostics
    after the first, scoped to one ACL instance **and one effective sink
    configuration** — replacing the sink or reloading a configuration that
    changes it builds a new `_AuditSink`, so a new failure is never hidden
    behind an old one.
    """

    __slots__ = ("_callback", "_config", "_failed", "_rejected_awaitable")

    def __init__(
        self,
        callback: Callable[[AuditEntry], None] | None,
        config: dict[str, Any] | None,
    ) -> None:
        self._callback = callback
        #: `None` means the document declared no `audit:` block. Requirement 2:
        #: DECLARATION activates the default sink, never the default value —
        #: `enabled` defaults to true, so reading the merged view would switch a
        #: log record per check on for every ACL file in existence.
        self._config = config
        self._failed = False
        self._rejected_awaitable = False

    @property
    def is_active(self) -> bool:
        if self._callback is not None:
            return True
        return bool(self._config) and bool(self._config.get("enabled", True))

    def deliver(self, entry: AuditEntry) -> None:
        """Deliver one entry. NEVER raises (§6.3.2 requirement 3)."""
        if self._callback is not None:
            self._deliver_to_callback(entry)
            return
        if self._config is None or not self._config.get("enabled", True):
            return
        if entry.decision == "deny" and not self._config.get("include_denied", True):
            # Requirement 6 — the load-time notice for this is emitted once, at
            # load; withholding here is silent by design.
            return
        self._emit_default(entry)

    def _deliver_to_callback(self, entry: AuditEntry) -> None:
        callback = self._callback
        assert callback is not None
        try:
            result = callback(entry)
        except Exception as exc:  # noqa: BLE001 — containment IS the requirement
            self._report_failure(exc)
            return
        if inspect.isawaitable(result):
            # Requirement 4. An async callback's failure surfaces after the
            # decision has already been returned, outside the containment
            # requirement 3 promises. Treated as an invalid delivery, reported
            # once, and the coroutine is closed so it does not warn on GC.
            if not self._rejected_awaitable:
                self._rejected_awaitable = True
                _logger.warning(
                    "The ACL audit_logger returned an awaitable. PROTOCOL_SPEC §6.3.2 "
                    "requirement 4 requires a SYNCHRONOUS callback: an async one fails after "
                    "the access decision has been returned, where it can no longer be "
                    "contained. This entry was NOT delivered. Enqueue the entry inside the "
                    "callback and return instead."
                )
            close = getattr(result, "close", None)
            if callable(close):
                close()

    def _emit_default(self, entry: AuditEntry) -> None:
        """§6.3.2 requirement 2 — the default sink.

        All thirteen §6.3.1 fields go out as STRUCTURED data under their
        `snake_case` wire names, not interpolated into the message. Without
        that, one specification yields three different "structured records"
        across the SDKs and nothing downstream consumes all three.
        """
        level = _AUDIT_LEVELS.get(str(self._config.get("log_level", "info")), logging.INFO)
        try:
            _audit_logger_default.log(level, AUDIT_EVENT_NAME, extra={"apcore_audit": asdict(entry)})
        except Exception as exc:  # noqa: BLE001 — a logging handler can fail too
            self._report_failure(exc)

    def _report_failure(self, exc: BaseException) -> None:
        if self._failed:
            return
        self._failed = True
        _logger.warning(
            "ACL audit delivery failed (%s: %s). The access decision is unaffected "
            "(PROTOCOL_SPEC §6.3.2 requirement 3) — auditing is a side channel and does not "
            "hold a veto over access. Further failures from this sink are not reported; "
            "replacing the sink or reloading the ACL starts a new report.",
            type(exc).__name__,
            exc,
        )


def _parse_audit_block(data: dict[str, Any], yaml_path: str) -> dict[str, Any] | None:
    """Validate an ACL file's ``audit:`` block (§6.3.2 requirement 8).

    Returns ``None`` only when the document declares no ``audit`` key at all —
    the distinction requirement 2 turns on, because `enabled` defaults to
    ``True`` and reading the merged view would switch a log record per check on
    for every ACL file in existence.

    **Presence, not truthiness.** ``audit:`` with nothing under it parses to
    ``None``, and the operator still wrote the block: that is a declaration with
    every setting at its default, not an absence.

    Validates the SUBTREE only: types and unknown keys inside the block. Every
    other unrecognised root key in an ACL file keeps being ignored.
    """
    if "audit" not in data:
        return None
    raw = data["audit"]
    if raw is None:
        return dict(_AUDIT_DEFAULTS)
    if not isinstance(raw, dict):
        raise ConfigError(
            f"{yaml_path}: 'audit' must be a mapping (PROTOCOL_SPEC §6.3.2), got " f"{type(raw).__name__}"
        )

    unknown = sorted(set(raw) - set(_AUDIT_FIELDS))
    if unknown:
        raise ConfigError(
            f"{yaml_path}: unknown key(s) in the 'audit' block: {', '.join(unknown)}. "
            f"The block accepts exactly {', '.join(_AUDIT_FIELDS)} "
            f"($defs/AuditConfig in schemas/acl-config.schema.json)."
        )

    config = dict(_AUDIT_DEFAULTS)
    for key in ("enabled", "include_denied"):
        if key in raw:
            if not isinstance(raw[key], bool):
                raise ConfigError(f"{yaml_path}: 'audit.{key}' must be a boolean, got {raw[key]!r}")
            config[key] = raw[key]
    if "log_level" in raw:
        if raw["log_level"] not in _AUDIT_LEVELS:
            raise ConfigError(
                f"{yaml_path}: 'audit.log_level' must be one of "
                f"{', '.join(_AUDIT_LEVELS)}, got {raw['log_level']!r}"
            )
        config["log_level"] = raw["log_level"]

    if not config["include_denied"]:
        # §6.3.2 requirement 6 — a notice, not a refusal. It withholds the
        # security-relevant half of the record, so an operator who wrote it
        # deliberately gets told once per load rather than stopped.
        _logger.warning(
            "%s sets audit.include_denied: false, so DENIED access attempts will not be "
            "recorded by the default audit sink (PROTOCOL_SPEC §6.3.2 requirement 6). "
            "Allowed calls are still recorded. Remove the entry to restore denials.",
            yaml_path,
        )
    return config


def _warn_audit_block_overridden(
    callback: Callable[[AuditEntry], None] | None,
    config: dict[str, Any] | None,
) -> None:
    """§6.3.2 requirement 1 — name EVERY field the callback overrides.

    Not only the most visible one: an operator who set `include_denied` and
    `log_level` and hears about one of them has been told the smaller half of
    what happened.
    """
    if callback is None or not config:
        return
    _logger.warning(
        "An audit_logger was supplied to this ACL, so it is the effective sink and the ACL "
        "file's 'audit:' block does not apply. The callback receives EVERY entry, allow and "
        "deny alike. These declared settings have no effect: %s "
        "(PROTOCOL_SPEC §6.3.2 requirement 1).",
        ", ".join(f"audit.{f}" for f in _AUDIT_FIELDS if f in config),
    )


#: The default sink's own logger, separate from this module's so an operator can
#: route `apcore.acl.audit` records without touching ACL diagnostics.
_audit_logger_default = logging.getLogger("apcore.acl.audit")


class ACL:
    """Access Control List with pattern-based rules and first-match-wins evaluation.

    Implements PROTOCOL_SPEC section 6 for module access control.

    Surface:
        :meth:`check` / :meth:`async_check` — the decision.
        :meth:`add_rule`, :meth:`remove_rule`, :meth:`reload` — mutation.
        :attr:`default_effect`, :attr:`rules` — read-only introspection (§6.8).
        :meth:`validate_rules` — diagnostics for malformed rules and unregistered
        condition keys, to run once handler registration is complete (§6.1.2).

    Thread safety:
        Internally synchronized. All public methods (check, add_rule,
        remove_rule, reload) and the read-only accessors are safe to call
        concurrently. The accessors take the same snapshot the check path does.
    """

    _condition_handlers: ClassVar[dict[str, ACLConditionHandler]] = {
        "identity_types": _IdentityTypesHandler(),
        "roles": _RolesHandler(),
        "max_call_depth": _MaxCallDepthHandler(),
        # PROTOCOL_SPEC §6.1.7: `arguments` is built-in, and registered here
        # beside the other built-ins rather than through a new registration
        # point — `register_condition` writes runtime code into a process-wide
        # registry, and a deployment-registered argument handler is exactly the
        # unauditable host code §7.9.6 rule 2 keeps out of a governance verdict.
        # Being an ordinary registry entry also means §6.1.4's precheck covers
        # it for free: `argument:` written for `arguments:` is an unregistered
        # key, so the rule is unevaluable rather than silently inert.
        "arguments": _ArgumentsHandler(),
    }

    # Async condition handlers: used by _evaluate_conditions_async for compound
    # operators ($or, $not) that need to await sub-condition evaluations.
    # Falls back to _condition_handlers for keys not present here.
    _async_condition_handlers: ClassVar[dict[str, ACLConditionHandler]] = {}

    @classmethod
    def register_condition(cls, key: str, handler: ACLConditionHandler) -> None:
        """Register a condition handler. Replaces existing handler for same key."""
        cls._condition_handlers[key] = handler

    @classmethod
    def register_async_condition(cls, key: str, handler: ACLConditionHandler) -> None:
        """Register an async-aware condition handler used by async_check().

        Async handlers are used instead of the sync handler when the async
        evaluation path is active. This allows compound operators like $or/$not
        to properly await sub-condition handlers.

        Mirrors ``apcore-typescript.ACL.registerAsyncCondition``.
        """
        cls._async_condition_handlers[key] = handler

    # -- Evaluation (PROTOCOL_SPEC §6.1.1) ---------------------------------

    @classmethod
    def _evaluate_conditions(
        cls,
        conditions: dict[str, Any],
        context: Context,
        path_prefix: str = "",
    ) -> ConditionOutcome:
        """Evaluate a ``conditions`` object on the sync path (PROTOCOL_SPEC §6.1.1).

        Returns one of three outcomes, not a boolean. A key whose handler
        answered "no" is ``UNSATISFIED``; a key the implementation cannot answer
        **as written** is ``UNEVALUABLE``. Collapsing the two is the defect
        §6.1.1 exists to prevent: it made a ``deny`` rule with a misspelled
        condition key fail **open**.

        A ``conditions`` object ANDs its keys, so §6.1.1's composition table
        applies: an outright ``UNSATISFIED`` wins even if a sibling was
        unevaluable, and ``UNEVALUABLE`` otherwise propagates. Short-circuiting
        on the decisive ``UNSATISFIED`` child is permitted (and a child skipped
        that way was never evaluated, so it records no diagnostic); short-
        circuiting on an ``UNEVALUABLE`` child is NOT, because a later sibling
        may still produce the decisive answer.

        Structural and registry faults are normally caught by :meth:`_precheck_conditions`
        before this runs (§6.1.4), which is what makes those diagnostics
        deterministic across implementations. The guards here are the same rules
        applied where the evaluator is entered directly.

        Args:
            conditions: The condition object to evaluate.
            context: The execution context.
            path_prefix: §6.1.4 path of the enclosing node, so a nested fault is
                reported as ``$or[1].k`` rather than as a bare key.
        """
        if not isinstance(conditions, dict):
            return cls._malformed_conditions(conditions, path_prefix)

        saw_unevaluable = False
        for key, value in conditions.items():
            outcome = cls._evaluate_condition(key, value, context, join_condition_path(path_prefix, key))
            if outcome is ConditionOutcome.UNSATISFIED:
                # Decisive: an outright "no" wins the AND. Remaining keys are
                # not evaluated, and therefore record nothing.
                return ConditionOutcome.UNSATISFIED
            if outcome is ConditionOutcome.UNEVALUABLE:
                saw_unevaluable = True
        return ConditionOutcome.UNEVALUABLE if saw_unevaluable else ConditionOutcome.SATISFIED

    @classmethod
    def _malformed_conditions(cls, conditions: Any, path_prefix: str = "") -> ConditionOutcome:
        """Classify a non-mapping ``conditions`` value as UNEVALUABLE (§6.1.1 case 5).

        ``ACLRule.conditions`` is annotated ``dict[str, Any] | None``, but the
        annotation binds nobody: ``ACL(rules=[...])`` and ``add_rule()`` build
        rules programmatically and never reach ``ACL.load``'s parser, so a scalar
        or a list arrives here intact. Iterating it raised ``AttributeError``
        straight out of ``check()``, which the ``ACL.check`` contract forbids.

        It is UNEVALUABLE rather than UNSATISFIED because a malformed block is a
        misconfiguration, not a handler answering "no": calling it UNSATISFIED
        would let a ``deny`` rule fall through to ``default_effect``, which is
        the bypass §6.1.1 exists to close.
        """
        path = path_prefix or ROOT_CONDITION_PATH
        type_name = type(conditions).__name__
        _logger.warning(
            "ACL conditions at %s must be a mapping, got %s — unevaluable (PROTOCOL_SPEC §6.1.1): "
            "a 'deny' rule takes effect, an 'allow' rule does not grant",
            path,
            type_name,
        )
        _record_handler_error(path, f"ACL conditions must be a mapping, got {type_name}")
        return ConditionOutcome.UNEVALUABLE

    @classmethod
    def _evaluate_condition(
        cls,
        key: str,
        value: Any,
        context: Context,
        path: str,
    ) -> ConditionOutcome:
        """Evaluate one condition key on the sync path, at *path* (§6.1.4)."""
        handler = cls._condition_handlers.get(key)
        if handler is None:
            # A typo'd key (`role:` for `roles:`) is not "condition not met" —
            # no answer is obtainable, so the rule resolves toward refusing
            # access and `AuditEntry.handler_error` says why.
            cls._record_unresolvable_key(key, path)
            return ConditionOutcome.UNEVALUABLE

        # Snapshot before invoking so a compound operator that merely *propagates*
        # a child's UNEVALUABLE does not also record a generic entry of its own —
        # the child already named the precise path.
        before = cls._recorded_condition_paths()
        try:
            result = cls._invoke_handler(handler, value, context, path)
        except Exception as exc:
            _logger.exception("Handler for condition %s raised — unevaluable (PROTOCOL_SPEC §6.1.1)", path)
            _record_handler_error(path, f"{type(exc).__name__}: {exc}")
            return ConditionOutcome.UNEVALUABLE

        if inspect.isawaitable(result):
            # Try to advance the coroutine one synchronous step. If the
            # coroutine returns without hitting an ``await`` (sync-only
            # body wrapped in async fn), StopIteration carries the value.
            # Otherwise it suspends — unevaluable on this path.
            try:
                result.send(None)  # type: ignore[attr-defined]
            except StopIteration as stop:
                return cls._coerce(stop.value, path, before)
            except Exception as exc:
                _logger.exception(
                    "Handler for condition %s raised during sync resolution — unevaluable (PROTOCOL_SPEC §6.1.1)",
                    path,
                )
                _record_handler_error(path, f"{type(exc).__name__}: {exc}")
                return ConditionOutcome.UNEVALUABLE
            else:
                # Coroutine suspended — genuinely async, can't run in sync path.
                # This is a *configuration* fault, not an unmet condition, so it
                # is one of §6.1.1's unevaluable situations (parity with
                # apcore-typescript's "Async condition … in sync context" and
                # apcore-rust's ``Poll::Pending`` arm).
                result.close()  # type: ignore[attr-defined]
                _logger.warning(
                    "Async condition %s suspended in sync context — unevaluable "
                    "(PROTOCOL_SPEC §6.1.1). Use async_check() for handlers needing await.",
                    path,
                )
                _record_handler_error(path, "async condition suspended in sync context — use async_check()")
                return ConditionOutcome.UNEVALUABLE

        return cls._coerce(result, path, before)

    @classmethod
    async def _evaluate_conditions_async(
        cls,
        conditions: dict[str, Any],
        context: Context,
        path_prefix: str = "",
    ) -> ConditionOutcome:
        """Async variant. Uses async handler if registered, falls back to sync.

        Same three-valued contract and same composition rules as
        :meth:`_evaluate_conditions`.
        """
        if not isinstance(conditions, dict):
            return cls._malformed_conditions(conditions, path_prefix)

        saw_unevaluable = False
        for key, value in conditions.items():
            outcome = await cls._evaluate_condition_async(key, value, context, join_condition_path(path_prefix, key))
            if outcome is ConditionOutcome.UNSATISFIED:
                return ConditionOutcome.UNSATISFIED
            if outcome is ConditionOutcome.UNEVALUABLE:
                saw_unevaluable = True
        return ConditionOutcome.UNEVALUABLE if saw_unevaluable else ConditionOutcome.SATISFIED

    @classmethod
    async def _evaluate_condition_async(
        cls,
        key: str,
        value: Any,
        context: Context,
        path: str,
    ) -> ConditionOutcome:
        """Evaluate one condition key on the async path, at *path* (§6.1.4)."""
        # Prefer async-specific handler (e.g., _OrHandlerAsync) so compound
        # operators recurse through the async path and properly await. Falls
        # back to the sync registry per PROTOCOL_SPEC §6.1.3.
        handler = cls._async_condition_handlers.get(key) or cls._condition_handlers.get(key)
        if handler is None:
            cls._record_unresolvable_key(key, path)
            return ConditionOutcome.UNEVALUABLE
        before = cls._recorded_condition_paths()
        try:
            result = cls._invoke_handler(handler, value, context, path)
            if inspect.isawaitable(result):
                result = await result
        except Exception as exc:
            _logger.exception("Handler for condition %s raised — unevaluable (PROTOCOL_SPEC §6.1.1)", path)
            _record_handler_error(path, f"{type(exc).__name__}: {exc}")
            return ConditionOutcome.UNEVALUABLE
        return cls._coerce(result, path, before)

    @staticmethod
    def _invoke_handler(handler: Any, value: Any, context: Context, path: str) -> Any:
        """Call a handler, handing the built-in compound operators their path.

        ``$or`` / ``$not`` recurse, so a fault beneath them must be reported at
        its position in the tree. They advertise the capability with
        ``path_aware``; every other handler keeps the two-argument protocol.
        """
        if getattr(handler, "path_aware", False):
            return handler.evaluate_at(value, context, path)
        return handler.evaluate(value, context)

    @classmethod
    def _coerce(cls, result: Any, path: str, before: frozenset[str]) -> ConditionOutcome:
        """Coerce a handler result, recording a diagnostic if it is UNEVALUABLE.

        A handler may report UNEVALUABLE itself, and that has to reach
        ``handler_error`` or §6.1.1 rule 2 is not met. But ``$or`` / ``$not``
        also return UNEVALUABLE when *propagating* a child's, and the child has
        already recorded the precise path — so a generic entry at the operator's
        own path would be a duplicate naming a less useful location. ``before``
        distinguishes the two: anything recorded during the call means the
        subtree spoke for itself.
        """
        outcome = _as_outcome(result)
        if outcome is ConditionOutcome.UNEVALUABLE and not (cls._recorded_condition_paths() - before):
            _record_handler_error(path, "handler could not evaluate the condition as written")
        return outcome

    @staticmethod
    def _record_unresolvable_key(key: str, path: str) -> None:
        """Warn and record an unresolvable condition key at *path*."""
        _logger.warning(
            "Unknown ACL condition %s (key %r) — unevaluable (PROTOCOL_SPEC §6.1.1): "
            "a 'deny' rule takes effect, an 'allow' rule does not grant",
            path,
            key,
        )
        _record_handler_error(path, "unknown ACL condition")

    # -- Structural and registry precheck (PROTOCOL_SPEC §6.1.4) -----------

    @classmethod
    def _precheck_patterns(cls, rule: ACLRule) -> list[_Fault]:
        """Check that ``callers`` and ``targets`` are each a list of strings **of legal shape** (§6.1.4.1).

        A value that is not a list of strings is a malformed rule, not a pattern
        set. A bare string is iterable, so ``callers: "admin.*"`` written where
        ``callers: ["admin.*"]`` was meant is read character by character — and
        ``*`` is a valid pattern matching everything, so an ``allow`` rule
        carrying that typo granted access to **every caller**. A non-subscriptable
        scalar raised ``TypeError`` out of ``check()``, which its contract forbids.

        Since spec v1.31.0 (§6.2.1, #112) the **arity** is checked here too. That
        half is the backstop for the one route no door covers: :class:`ACLRule`
        is a non-frozen dataclass, so ``rule.targets = []`` reaches the evaluator
        whatever the constructors check. §6.1.5's v1.30.0 reasoning for why the
        ``effect`` closure needs no backstop — the value is never read again once
        the doors are shut — does **not** transfer, because a mutated pattern
        array *is* read: the matcher consults it on the next ``check()``. An
        array outside §6.2.1's closure is therefore a precheck fault on exactly
        the same terms as a malformed type: the rule's scope is unreadable, the
        rule is UNEVALUABLE, and §6.1.1's effect table decides — a ``deny`` rule
        takes effect and denies, an ``allow`` rule does not match and MUST NOT
        grant. §6.1.4.1 has no partially-readable tier: ``targets: []`` is
        legible as an empty scope in a way ``targets: 3`` is not, and acting on
        that difference is the per-implementation judgement call that produced
        three different answers in #100.

        Tier 2 — an array that is well-formed under every clause above and still
        matches nothing — is deliberately **not** here. It is
        :meth:`_never_matches`, and it feeds :meth:`validate_rules` alone.

        Both fields are examined; the check does not stop at the first fault, so
        the finding set is a pure function of the rule (§6.1.4 determinism). At
        most one fault per field: the type fault keeps precedence, because an
        array whose elements are not strings has no meaningful arity reading.
        """
        faults: list[_Fault] = []
        for path, value in ((_CALLERS_PATH, rule.callers), (_TARGETS_PATH, rule.targets)):
            if not (isinstance(value, list) and all(isinstance(pattern, str) for pattern in value)):
                faults.append(
                    _Fault(
                        path=path,
                        key=None,
                        reason=f"{path} must be a list of strings, got {type(value).__name__}",
                    )
                )
                continue
            reason = _pattern_array_fault(value)
            if reason is not None:
                faults.append(_Fault(path=path, key=None, reason=reason))
        return faults

    @staticmethod
    def _never_matches(field: str, patterns: list[str]) -> str | None:
        """Tier 2 (§6.2.1, spec v1.31.0): a well-formed array that matches no module ID.

        A **separate predicate** from :meth:`_precheck_patterns` on purpose. It
        is consulted by :meth:`validate_rules` after the precheck reports the
        field clean, and it MUST NOT reach ``handler_error`` or any access
        decision: such a rule loads, is reported, and decides exactly as it did
        before — ``["$not", "*"]`` on a ``deny`` rule under
        ``default_effect: allow`` still lets an unrelated call through.

        Why a finding and not a rejection. Closing §6.2.1's arities does not
        exhaust the inert class — ``["$not", "*"]`` has perfectly legal arity,
        exactly one operand, and matches nothing, producing the identical
        fail-open — but detecting it means reasoning about the **match
        relation** rather than the array's shape, and that predicate cannot be
        closed without freezing the pattern language. §6.1.4's determinism
        guarantee binds precheck-origin diagnostics because they feed
        ``handler_error`` and the decision; tier-2 findings feed neither, so a
        divergence between SDKs is a missed diagnostic rather than an ACL file
        that loads in one language and fails in another. §6.1.3 governs: this is
        diagnostics, not enforcement.

        The criterion is normative and this list is §6.2.1's MUST-detect
        **minimum**, not a closed set — the mistake §6.1.1 corrected in v1.25.0
        was enumerating where it should have stated a principle.

        Args:
            field: ``"callers"`` or ``"targets"``. Unlike tier 1 this is not
                field-symmetric: ``@external`` is the caller-side sentinel §6.5
                substitutes for a null ``caller_id``, so it is exactly what
                ``callers`` is for and matches no module ID as a ``targets``
                pattern.
            patterns: A pattern array the precheck has already called clean.

        Returns:
            A reason phrase, or None when the array can match something.
        """
        if not patterns:  # pragma: no cover — the precheck rejected it already
            return None

        if patterns[0] == "$not":
            operand = patterns[1]
            # "any pattern consisting only of wildcards" — `*`, `**`, `***`.
            # Not a string comparison against "*": `**` is universal too, and an
            # implementation that compares literals reports one and not the other.
            if set(operand) == {"*"}:
                return (
                    f"['$not', {operand!r}] negates a pattern that matches every module ID, so the rule "
                    "fires for nothing and protects nothing. It is well-formed, so it loads and changes "
                    "no decision — but a rule that can never match is not the rule the operator wrote."
                )
            return None

        operands = patterns[1:] if patterns[0] == "$or" else patterns

        # PROTOCOL_SPEC §6.2.2 (v1.37.0): A08 has `*` as its ONLY metacharacter,
        # and §2.7 forbids `?` in a module ID, so a pattern carrying one matches
        # nothing and can never match anything. The author meant a wildcard —
        # `?` IS one in every other pattern surface the spec defines (A25,
        # §9.2.3) — and got a rule that is silently inert: a `deny` guarding
        # nothing, or an `allow` that never fires and sends its author looking
        # elsewhere. Reported, never rejected, and the meaning is unchanged:
        # promoting `?` in A08 would widen `allow` rules that are inert in every
        # deployed policy today, which is the one direction an authorization
        # matcher must not move without an operator's consent.
        with_question = [operand for operand in operands if "?" in operand]
        if with_question and len(with_question) == len(operands):
            return (
                f"every pattern here contains '?', which is a LITERAL in ACL matching (Algorithm A08 — "
                f"'*' is the only metacharacter) and which §2.7 forbids in a module ID, so {with_question!r} "
                "matches nothing and can never match anything. '?' is a wildcard in every other pattern "
                "surface (A25, §9.2.3), which is where the expectation comes from. The rule loads and "
                "changes no decision — it simply never fires."
            )

        if field == _TARGETS_PATH and all(operand == "@external" for operand in operands):
            return (
                "'@external' is the caller-side sentinel §6.5 substitutes for a null caller_id. No module "
                "ID is '@external', so as a 'targets' pattern it matches nothing. It remains entirely "
                "legal in 'callers', which is what it is for."
            )
        return None

    @classmethod
    def _warn_dead_question_patterns(cls, rules: list[ACLRule], *, base_index: int = 0) -> None:
        """Warn — never fail — for an ACL pattern carrying ``?`` (§6.2.2).

        PROTOCOL_SPEC §6.2.2 clause 1: loading MUST NOT fail on such a pattern,
        but MUST warn, naming the rule index, the field and the pattern as
        written. Clause 3: the meaning is unchanged — ``?`` stays a literal and
        the rule keeps matching nothing. This adds a diagnostic and changes no
        decision, deliberately, because promoting ``?`` to a wildcard in A08
        would widen ``allow`` rules that are inert in every deployed policy
        today.

        :meth:`validate_rules` reports the same condition at deploy time; this
        is the load-time half, on the §6.1.2 precedent.
        """
        for offset, rule in enumerate(rules):
            for pattern_field, patterns in ((_CALLERS_PATH, rule.callers), (_TARGETS_PATH, rule.targets)):
                if not isinstance(patterns, list):
                    continue
                for pattern in patterns:
                    if isinstance(pattern, str) and "?" in pattern:
                        _logger.warning(
                            "ACL rule %d (effect=%s) %s pattern %r contains '?', which is a LITERAL in "
                            "ACL matching (Algorithm A08 — '*' is the only metacharacter) and which "
                            "PROTOCOL_SPEC §2.7 forbids in a module ID. The pattern therefore matches "
                            "nothing and can never match anything: a 'deny' rule guards nothing, an "
                            "'allow' rule never fires. '?' IS a wildcard in every other pattern surface "
                            "(Algorithm A25, §9.2.3), which is where the expectation comes from. Nothing "
                            "about this rule's meaning has changed — see §6.2.2.",
                            base_index + offset,
                            rule.effect,
                            pattern_field,
                            pattern,
                        )

    @classmethod
    def _precheck_conditions(cls, conditions: Any, *, async_path: bool, path_prefix: str = "") -> list[_Fault]:
        """Walk the whole ``conditions`` tree for structural and registry faults (§6.1.4).

        Context-independent, invokes no handler, and **does not short-circuit** —
        its completeness is what makes §6.1.1 rule 2's deterministic
        ``handler_error`` achievable across implementations. It determines:

        - whether ``conditions`` is a mapping (§6.1.1 case 5);
        - for each key, whether a handler is resolvable on the evaluation path in
          use (§6.1.1 case 1, §6.1.3);
        - for each compound operator, whether its value has the required shape
          (§6.1.1 case 4).

        Args:
            conditions: The condition object (or malformed value) to examine.
            async_path: True when the enclosing call is :meth:`async_check`, which
                resolves against the async registry and falls back to the sync one.
            path_prefix: §6.1.4 path of the enclosing node.
        """
        faults: list[_Fault] = []
        if not isinstance(conditions, dict):
            path = path_prefix or ROOT_CONDITION_PATH
            return [
                _Fault(
                    path=path,
                    key=None,
                    reason=f"ACL conditions must be a mapping, got {type(conditions).__name__}",
                )
            ]

        for key, value in conditions.items():
            path = join_condition_path(path_prefix, key)
            if key == "$or":
                if not isinstance(value, list):
                    faults.append(
                        _Fault(path=path, key=key, reason=f"$or value must be a list, got {type(value).__name__}")
                    )
                    continue
                for index, sub in enumerate(value):
                    branch = f"{path}[{index}]"
                    if not isinstance(sub, dict):
                        faults.append(
                            _Fault(
                                path=branch,
                                key=None,
                                reason=f"$or element must be a mapping, got {type(sub).__name__}",
                            )
                        )
                        continue
                    faults.extend(cls._precheck_conditions(sub, async_path=async_path, path_prefix=branch))
            elif key == "$not":
                if not isinstance(value, dict):
                    faults.append(
                        _Fault(path=path, key=key, reason=f"$not value must be a mapping, got {type(value).__name__}")
                    )
                    continue
                faults.extend(cls._precheck_conditions(value, async_path=async_path, path_prefix=path))
            elif key == "arguments":
                # §6.1.7's vocabulary is closed and its predicate values are
                # lists of strings, both of which are decidable without a
                # context and without running the handler — so they belong in
                # the precheck, where the finding is a pure function of the rule
                # and identical across implementations (§6.1.4 determinism).
                # Attached to the `arguments` key, like a malformed `$or` value:
                # the flags stay False because a structural fault resolves on
                # neither evaluation path, however well registered the key is.
                # §6.1.8: descend to the offending predicate where one can be
                # named, exactly as §6.1.4 descends into `$or[1].k`, and report
                # EVERY faulty predicate rather than stopping at the first.
                for suffix, reason in validate_arguments_condition(value):
                    faults.append(_Fault(path=f"{path}{suffix}", key=key, reason=reason))
            else:
                sync_resolvable = key in cls._condition_handlers
                async_resolvable = sync_resolvable or key in cls._async_condition_handlers
                resolvable = async_resolvable if async_path else sync_resolvable
                if not resolvable:
                    faults.append(
                        _Fault(
                            path=path,
                            key=key,
                            reason="unknown ACL condition",
                            sync_resolvable=sync_resolvable,
                            async_resolvable=async_resolvable,
                        )
                    )
        return faults

    @classmethod
    def _precheck_rule(cls, rule: ACLRule, *, async_path: bool) -> list[_Fault]:
        """Full §6.1.4 precheck for one rule: pattern fields, then conditions."""
        faults = cls._precheck_patterns(rule)
        if rule.conditions is not None:
            faults.extend(cls._precheck_conditions(rule.conditions, async_path=async_path))
        return faults

    @classmethod
    def _warn_unregistered_condition_keys(cls, rules: list[ACLRule], *, base_index: int = 0) -> None:
        """Warn — never fail — for rules that fail the §6.1.4 precheck.

        PROTOCOL_SPEC §6.1.2: ``register_condition`` writes to a runtime,
        process-wide registry and ``acl.root`` discovery commonly runs during
        framework bootstrap, ahead of application code. Loading MUST NOT fail on
        an unregistered key, but MUST warn, naming the rule index, the key and
        the rule's ``effect`` — the ``effect`` because a misconfigured ``deny``
        rule is the consequential case. :meth:`validate_rules` is the
        deterministic check to run once registration is complete.
        """
        for offset, rule in enumerate(rules):
            for fault in cls._precheck_rule(rule, async_path=False):
                _logger.warning(
                    "ACL rule %d (effect=%s) fails the structural/registry precheck at %s: %s. "
                    "On the sync check() path the rule is unevaluable (PROTOCOL_SPEC §6.1.1), so a "
                    "'deny' rule takes effect and an 'allow' rule does not grant. Register a handler "
                    "with ACL.register_condition(), or call ACL.validate_rules() after bootstrap to "
                    "assert on this.",
                    base_index + offset,
                    rule.effect,
                    fault.path,
                    fault.reason,
                )

    def validate_rules(self) -> tuple[RuleValidationFinding, ...]:
        """Report every rule that fails the §6.1.4 structural and registry precheck.

        The explicit validation entry point PROTOCOL_SPEC §6.1.2 rule 3
        requires. It is named ``validate_rules`` rather than
        ``validate_conditions`` because it reports structural faults in
        ``callers`` and ``targets`` as well (§6.1.4.1), not only condition keys.

        Since spec v1.31.0 (§6.2.1, #112) it is also the **only** reader of
        :meth:`_never_matches` — tier 2, an array that is well-formed under
        every structural clause and still matches no legal module ID. Such a
        rule protects nothing and MUST be reported, but it MUST NOT be rejected
        and MUST NOT change any access decision; the finding carries the same
        shape as a structural fault (path ``callers`` / ``targets``, a **null**
        key, both resolvability flags False), so a reader cannot tell the tiers
        apart and does not need to.

        Loading an ACL only warns, because handler registration is a runtime,
        process-wide act that legitimately happens after discovery; this method
        is what a deployment calls once registration is complete, so it can turn
        a broken rule into a startup error of its own choosing.

        A finding is emitted whenever ``sync_resolvable`` is false — **including**
        when ``async_resolvable`` is true (§6.1.3 rule 3). An async-only handler
        is a working condition under :meth:`async_check` and an unevaluable one
        under :meth:`check`; an application that only ever calls ``async_check``
        may ignore such a finding, but that judgement belongs to the caller, not
        to the validator.

        Pure read: it does not mutate the ACL, register handlers, or emit an
        audit event.

        Returns:
            A possibly-empty tuple of :class:`RuleValidationFinding`, ordered by
            ``rule_index`` and then lexicographically by ``condition_path`` — by
            path and not by key, because a nested ``$or`` may carry the same key
            at several positions, which leaves ordering by key undefined. Empty
            means every rule currently passes the precheck on the sync path; it
            is not a guarantee about the future, since a later :meth:`add_rule`
            can introduce a new fault.
        """
        with self._lock:
            rules = list(self._rules)

        cls = type(self)
        findings: list[RuleValidationFinding] = []
        for index, rule in enumerate(rules):
            faults = cls._precheck_rule(rule, async_path=False)
            # Tier 2 is consulted only where the precheck reported the field
            # clean: an array whose shape is already faulty has no readable
            # match relation, and two findings on one path would say the same
            # thing twice.
            faulty_fields = {fault.path for fault in faults}
            for pattern_field, patterns in ((_CALLERS_PATH, rule.callers), (_TARGETS_PATH, rule.targets)):
                if pattern_field in faulty_fields:
                    continue
                reason = cls._never_matches(pattern_field, patterns)
                if reason is not None:
                    faults.append(_Fault(path=pattern_field, key=None, reason=reason))
            for fault in sorted(faults, key=lambda f: f.path):
                findings.append(
                    RuleValidationFinding(
                        rule_index=index,
                        condition_path=fault.path,
                        condition_key=fault.key,
                        effect=rule.effect,
                        sync_resolvable=fault.sync_resolvable,
                        async_resolvable=fault.async_resolvable,
                    )
                )
        return tuple(findings)

    def __init__(
        self,
        rules: list[ACLRule] | None = None,
        default_effect: str = "deny",
        *,
        audit_logger: Callable[[AuditEntry], None] | None = None,
        audit_config: dict[str, Any] | None = None,
    ) -> None:
        """Initialize ACL with ordered rules and a default effect.

        Args:
            rules: Ordered list of ACL rules (first match wins). Defaults to [].
            default_effect: Effect when no rule matches — ``"allow"`` or
                ``"deny"``, a closed set (PROTOCOL_SPEC §6.1.5).
            audit_logger: Optional callback invoked with an AuditEntry for
                every check() call. Useful for structured audit trails. It
                receives EVERY entry and is never narrowed, levelled or
                silenced by *audit_config* (PROTOCOL_SPEC §6.3.2 requirement 1).
            audit_config: The ACL file's ``audit:`` block, as declared. ``None``
                means the document declared none, which keeps the pre-v1.45.0
                behaviour of no audit output without a callback. Supplied by
                :meth:`load`; callers constructing an ACL directly normally pass
                *audit_logger* instead.

        Raises:
            ACLRuleError: When *default_effect* is outside ``allow`` / ``deny``,
                or when a rule in *rules* is invalid **at this moment** —
                whatever its history. Each rule validated its own ``effect``,
                ``approval`` and pattern-array shape when it was constructed
                (``ACLRule.__post_init__``), but a rule mutated *after* its own
                construction and handed straight to this constructor — never
                having been installed in any ACL — is being offered to this
                door for the first time, and this constructor does not get to
                treat construction history as an exemption from the check
                every other rule in *rules* gets (PROTOCOL_SPEC §6.1.4.1 /
                §6.2.1, spec v1.33.0). This is narrower than it sounds: a rule
                already installed inside a *previously constructed* ``ACL`` and
                mutated afterward through a reference the caller already holds
                is a different, uninterceptable route — see §6.1.1's
                UNEVALUABLE backstop, which this constructor does not affect.
        """
        # PROTOCOL_SPEC §6.1.5 closes ``default_effect`` on the same terms as a
        # rule's ``effect``, and this constructor is the only writer —
        # ``load()`` and ``reload()`` both funnel through here — so the value is
        # in-enum for the lifetime of the object. Reads _EFFECT_VALUES rather
        # than an inline literal so the two fields cannot drift on which values
        # are legal; the message stays as it was for cross-language parity.
        # §6.2.1 puts this FIRST, before any rule, at every door — and this
        # constructor takes its rules already built, so "first" is the first
        # thing it does.
        _validate_default_effect(default_effect)

        self._rules = list(rules) if rules is not None else []

        # PROTOCOL_SPEC §6.1.6 rule 3: direct construction is one of the three
        # entry points a malformed rule MUST be rejected at, and §6.1.4.1 /
        # §6.2.1 (spec v1.33.0) settle what "already-constructed rule" means
        # for the backstop that exempts *only* a rule already installed in a
        # live ACL and mutated afterward through a caller's own reference —
        # not a rule mutated before it ever reaches this constructor.
        # ``ACLRule.__post_init__`` already caught this at true construction
        # time; what it cannot catch is `rule.targets = []` run *after*
        # construction and *before* being handed to ``ACL(rules=[rule])`` for
        # the first time, since `__post_init__` runs exactly once. Same
        # function, same ``where=`` convention `add_rule` uses, so both doors
        # raise an identical message for the same fault, in list order (lowest
        # index first — a `for` loop raising on first failure already gives us
        # this for free).
        for rule in self._rules:
            _validate_rule(rule, where="ACLRule")

        self._default_effect: str = default_effect
        self._yaml_path: str | None = None
        self._audit_logger: Callable[[AuditEntry], None] | None = audit_logger
        self._audit_config: dict[str, Any] | None = dict(audit_config) if audit_config is not None else None
        # §6.3.2 requirement 1: one effective sink, never two. Built here and
        # rebuilt by `reload()`, which is what scopes requirement 5's
        # once-per-failure suppression to a sink CONFIGURATION rather than to
        # the ACL object's lifetime.
        self._audit_sink = _AuditSink(audit_logger, self._audit_config)
        _warn_audit_block_overridden(audit_logger, self._audit_config)
        self._logger: logging.Logger = logging.getLogger(__name__)
        self._lock = threading.Lock()
        self.debug: bool = False
        # PROTOCOL_SPEC §6.5: warn the *first* time a conditional rule is
        # skipped for want of a context. Keyed by (rule index, effect) so a
        # hot path does not flood the log.
        self._warned_missing_context: set[tuple[int | None, str]] = set()

        # PROTOCOL_SPEC §6.1.2 rule 4: every entry point that accepts rules is
        # covered, direct construction included — ACL.load() and reload() both
        # funnel through here.
        type(self)._warn_unregistered_condition_keys(self._rules)
        # PROTOCOL_SPEC §6.2.2 clause 1 — same entry point, same never-fail contract.
        type(self)._warn_dead_question_patterns(self._rules)

    @property
    def default_effect(self) -> str:
        """The effect applied when no rule matches — ``"allow"`` or ``"deny"``.

        Read-only accessor required by PROTOCOL_SPEC §6.8. A pure read: it
        emits no audit event and mutates nothing, and it reflects the reloaded
        file after :meth:`reload`.
        """
        with self._lock:
            return self._default_effect

    @property
    def rules(self) -> tuple[ACLRule, ...]:
        """The current rule list, in definition order.

        Read-only accessor required by PROTOCOL_SPEC §6.8. Returns an immutable
        tuple taken under the same snapshot discipline :meth:`check` uses, so a
        caller cannot reach through it to mutate the ACL's own list. Reflects
        the reloaded file after :meth:`reload`.
        """
        with self._lock:
            return tuple(self._rules)

    @classmethod
    def load(
        cls,
        yaml_path: str,
        *,
        audit_logger: Callable[[AuditEntry], None] | None = None,
    ) -> ACL:
        """Load ACL configuration from a YAML file.

        Args:
            yaml_path: Path to the YAML configuration file.
            audit_logger: Optional audit callback. Supplying it here makes it
                the effective sink for the loaded ACL, ahead of the file's
                ``audit:`` block (PROTOCOL_SPEC §6.3.2 requirement 1), and
                :meth:`reload` preserves it (requirement 7). Without this
                parameter the two halves of that contract — load from a file,
                audit through a callback — had no supported path between them.

        A rule referencing a condition key with no handler registered at load
        time is **not** an error: ``register_condition`` writes to a runtime,
        process-wide registry, and ``acl.root`` discovery commonly runs during
        framework bootstrap ahead of application code, so failing here would
        reject valid configurations on ordering alone (PROTOCOL_SPEC §6.1.2).
        A warning naming the rule index, the key and the rule's ``effect`` is
        emitted instead; :meth:`validate_rules` is the deterministic check
        to run once registration is complete.

        Returns:
            A new ACL instance configured from the YAML file.

        Raises:
            ConfigNotFoundError: If the file does not exist.
            ACLRuleError: If the YAML is invalid or has structural errors —
                including a ``conditions`` value that is not a mapping, which a
                file has no legitimate reason to carry and which would otherwise
                surface only as a §6.1.1 case-5 denial at the first ``check()``,
                and a ``callers`` / ``targets`` array outside §6.2.1's closed
                shape (spec v1.31.0, #112), which loaded clean and left the rule
                inert.
        """
        if not os.path.isfile(yaml_path):
            raise ConfigNotFoundError(config_path=yaml_path)

        with open(yaml_path, encoding="utf-8") as f:
            try:
                data = yaml.safe_load(f)
            except yaml.YAMLError as e:
                raise ACLRuleError(f"Invalid YAML in {yaml_path}: {e}") from e

        if not isinstance(data, dict):
            raise ACLRuleError(f"ACL config must be a mapping, got {type(data).__name__}")

        # PROTOCOL_SPEC §9.2.4.1 (apcore#118): an `audit:` block in an ACL file
        # has never been read. Deleting it from `acl-config.schema.json` would
        # produce no signal at all — no implementation validates an ACL file
        # against that schema, and this loader parses into an open mapping and
        # takes the fields it wants, so any unknown root key is dropped in
        # silence. The diagnostic therefore has to live here.
        #
        # Scoped to `audit` deliberately: §6.3.2 requirement 8 validates this
        # SUBTREE and nothing else, so every other unrecognised root key in an
        # ACL file keeps being ignored exactly as before. This was never
        # unknown-key closure for ACL files, and wiring the block does not make
        # it one.
        #
        # The §9.2.4.1 deprecation notice that used to stand here is gone: spec
        # v1.45.0 gave the block a delivery contract, and a key that has gained
        # a consumer must stop being announced as going away.
        audit_config = _parse_audit_block(data, yaml_path)

        # PROTOCOL_SPEC §6.2.1: `default_effect` is judged FIRST, before any
        # rule, at every door. It is not a rule and has no index, so the rule
        # ordering below never reaches it — and this used to be left entirely to
        # the `ACL` constructor at the bottom of this method, which runs after
        # every rule has been parsed and validated, so a file carrying a bad
        # `default_effect` AND a bad rule 0 was refused for the RULE here and
        # for `default_effect` through direct construction: one configuration,
        # two answers, from two doors of the same SDK.
        #
        # Placed above the `rules` checks and not merely above the loop:
        # "first" is ahead of the file-level checks on the `rules` COLLECTION
        # too, so a document both missing `rules` and carrying a bad
        # `default_effect` names the `default_effect`.
        default_effect: str = data.get("default_effect", "deny")
        _validate_default_effect(default_effect)

        if "rules" not in data:
            raise ACLRuleError("ACL config missing required 'rules' key")

        raw_rules = data["rules"]
        if not isinstance(raw_rules, list):
            raise ACLRuleError(f"'rules' must be a list, got {type(raw_rules).__name__}")

        rules: list[ACLRule] = []

        for i, raw_rule in enumerate(raw_rules):
            if not isinstance(raw_rule, dict):
                raise ACLRuleError(f"Rule {i} must be a mapping, got {type(raw_rule).__name__}")

            for key in ("callers", "targets", "effect"):
                if key not in raw_rule:
                    raise ACLRuleError(f"Rule {i} missing required key '{key}'")

            # A missing key was already rejected above so an omission cannot
            # render a rule inert; an unknown key is the same hazard pointing
            # the other way, and was dropped in silence until #107.
            _reject_unknown_rule_keys(i, raw_rule)

            # PROTOCOL_SPEC §6.1.5. Checked here as well as in
            # ACLRule.__post_init__ so a file fault names the offending rule's
            # index, which the dataclass cannot know — the same split
            # _validate_approval already uses.
            effect = raw_rule["effect"]
            _validate_effect(effect, where=f"Rule {i}")

            # PROTOCOL_SPEC §6.1.6. Checked here as well as in ACLRule.__post_init__
            # so a file fault names the offending rule's index, which the
            # dataclass cannot know.
            approval = raw_rule.get("approval", APPROVAL_NOT_REQUIRED)
            _validate_approval(approval, effect, where=f"Rule {i}")

            # PROTOCOL_SPEC §6.2.1 (spec v1.31.0, #112). Checked here as well as
            # in ACLRule.__post_init__ so a file fault names the offending rule's
            # index, which the dataclass cannot know — the same split
            # _validate_effect and _validate_approval already use. `ACL.load`
            # rejected an OMITTED callers / targets and permitted an EMPTY one,
            # so a plain YAML file reached the fail-open; the arity closure is
            # what shuts that route.
            callers = raw_rule["callers"]
            if not isinstance(callers, list):
                raise ACLRuleError(f"Rule {i} 'callers' must be a list, got {type(callers).__name__}")
            _validate_pattern_array(callers, "callers", where=f"Rule {i}")

            targets = raw_rule["targets"]
            if not isinstance(targets, list):
                raise ACLRuleError(f"Rule {i} 'targets' must be a list, got {type(targets).__name__}")
            _validate_pattern_array(targets, "targets", where=f"Rule {i}")

            raw_conditions = raw_rule.get("conditions")
            if raw_conditions is not None and not isinstance(raw_conditions, dict):
                # A YAML file cannot reach §6.1.1 case 5 — the parser rejects it
                # here, as apcore-typescript's `_parseAclRule` does. Direct
                # construction and add_rule() still can, which is what the
                # runtime precheck (§6.1.4) is for.
                raise ACLRuleError(f"Rule {i} 'conditions' must be a mapping, got {type(raw_conditions).__name__}")

            rules.append(
                ACLRule(
                    callers=callers,
                    targets=targets,
                    effect=effect,
                    description=raw_rule.get("description", ""),
                    conditions=raw_conditions,
                    approval=approval,
                )
            )

        acl = cls(
            rules=rules,
            default_effect=default_effect,
            audit_logger=audit_logger,
            audit_config=audit_config,
        )
        acl._yaml_path = yaml_path
        return acl

    @classmethod
    def discover(cls, config: Config) -> ACL | None:
        """Activate ``acl.root`` config-driven ACL discovery (D-64, Recommendation A).

        Resolves the ``acl.root`` config key (default ``"./acl"``) and loads an
        ACL from it when the path exists. The path is resolved relative to the
        directory of the config's source file when known (``config.source_path``),
        otherwise relative to the current working directory.

        ``acl.root`` is a directory by spec convention (``acl/{scope}_acl.yaml``,
        PROTOCOL_SPEC §3.1). When the resolved path is a directory, the
        conventional ``global_acl.yaml`` within it is loaded; if that file is
        absent the result is ``None`` (the missing-path no-op still holds). When
        the resolved path is itself a file, it is loaded directly. Either way the
        actual load goes through :meth:`load`.

        Critical invariant: a missing path returns ``None`` and attaches NOTHING.
        It MUST NOT synthesize an empty default-deny ACL — doing so would silently
        deny every inter-module call in every project that lacks an ``acl`` dir.
        Missing path means "no enforcement", identical to pre-D-64 behavior. The
        ``acl.default_effect`` config key only takes effect once a real ACL file
        is loaded (it is read by :meth:`load` from the ACL file itself); it never
        feeds a synthesized ACL here.

        Args:
            config: The loaded configuration to read ``acl.root`` from.

        Returns:
            A loaded :class:`ACL` when the resolved path exists, otherwise
            ``None``.

        Raises:
            ConfigNotFoundError: Never raised for a missing root (that path
                returns ``None``); only propagated from :meth:`load` if the
                path disappears between the existence check and the load.
            ACLRuleError: If the ACL file exists but is structurally invalid.
        """
        root = config.get("acl.root", Config.get_default("acl.root"))
        if root is None:
            return None

        root_path = Path(str(root))
        if not root_path.is_absolute():
            source_path = config.source_path
            if source_path is not None:
                base = Path(source_path).resolve().parent
            else:
                base = Path.cwd()
            root_path = base / root_path

        if not root_path.exists():
            # Missing path -> no enforcement. Do NOT synthesize an ACL.
            return None

        if root_path.is_dir():
            # Directory convention: acl/{scope}_acl.yaml (PROTOCOL_SPEC §3.1).
            acl_file = root_path / "global_acl.yaml"
            if not acl_file.is_file():
                # Directory present but no conventional ACL file -> no-op.
                return None
            return cls.load(str(acl_file))

        return cls.load(str(root_path))

    def check(
        self,
        caller_id: str | None,
        target_id: str,
        context: Context | None = None,
        *,
        projection: Any | None = None,
    ) -> bool:
        """Check if a call from caller_id to target_id is allowed.

        A rule whose conditions cannot be **evaluated** at all — no registered
        handler, a handler that raised, or an async handler on this sync path —
        resolves toward refusing access per PROTOCOL_SPEC §6.1.1: a ``deny``
        rule takes effect and the call is denied, an ``allow`` rule does not
        match and does not grant — but an ``allow`` rule that carried
        ``approval: required`` does not take the requirement with it, which
        becomes **pending** and composes with whatever grants later (§6.1.1
        rule 5). The emitted :class:`AuditEntry` carries a non-null
        ``handler_error`` naming the key and the reason. An unevaluable
        condition never raises out of this method.

        **This boolean fails closed on an approval requirement** (§6.8.1): a
        *decision* resolving to ``allow`` with an approval requirement returns
        ``False`` here, because a boolean can only be read as "let it through /
        do not" and letting it through would run a call the ACL said needed a
        human. Callers that need the two axes apart use :meth:`check_access`.

        Args:
            caller_id: The calling module ID, or None for external calls.
            target_id: The target module ID being called.
            context: Optional execution context for conditional rules.

        Returns:
            True if the call is allowed, False if denied **or if it is allowed
            but requires approval** — see :meth:`check_access`.
        """
        return self._as_legacy_boolean(self.check_access(caller_id, target_id, context, projection=projection))

    def check_access(
        self,
        caller_id: str | None,
        target_id: str,
        context: Context | None = None,
        *,
        projection: Any | None = None,
    ) -> AccessDecision:
        """Resolve a call to a structured :class:`AccessDecision` (PROTOCOL_SPEC §6.8.1).

        An ACL rule answers two independent questions (§6.1.6) — may this caller
        reach this target at all, and must this particular call be put to a
        human before it runs. :meth:`check` can carry only the first, so this is
        the accessor the Executor uses. Emits exactly one audit entry, like
        :meth:`check`; the two are the same decision reported at different
        widths, not two decisions.

        ``approval_required`` is the matched rule's own union any requirement
        left **pending** by an unevaluable ``allow`` rule (§6.1.1 rule 5), so it
        may originate in a rule that did not match and may accompany
        ``matched_rule_index: None`` when ``default_effect: allow`` granted.

        Args:
            caller_id: The calling module ID, or None for external calls.
            target_id: The target module ID being called.
            context: Optional execution context for conditional rules.

        Returns:
            An :class:`AccessDecision` carrying ``access``,
            ``approval_required``, ``matched_rule_index`` and ``reason``.
        """
        effective_caller, rules, default_effect, audit_sink = self._snapshot(caller_id)

        token = _handler_error_var.set({})
        projection_token = _projection_var.set(projection)
        try:
            matched: tuple[int, ACLRule] | None = None
            pending_approval = False
            for idx, rule in enumerate(rules):
                outcome = self._matches_rule(rule, effective_caller, target_id, context, rule_index=idx)
                if outcome is ConditionOutcome.UNEVALUABLE:
                    if self._unevaluable_rule_takes_effect(rule):
                        matched = (idx, rule)
                        break
                    # The rule steps aside (§6.1.1 rule 1) but its approval
                    # requirement does not go with it (rule 5).
                    pending_approval = pending_approval or self._raises_pending_approval(rule)
                    continue
                if outcome is ConditionOutcome.SATISFIED:
                    matched = (idx, rule)
                    break

            return self._finalize_check(
                log_method="check",
                caller_id=caller_id,
                effective_caller=effective_caller,
                target_id=target_id,
                rules_present=bool(rules),
                default_effect=default_effect,
                matched=matched,
                pending_approval=pending_approval,
                audit_sink=audit_sink,
                context=context,
            )
        finally:
            _projection_var.reset(projection_token)
            _handler_error_var.reset(token)

    @staticmethod
    def _as_legacy_boolean(decision: AccessDecision) -> bool:
        """Collapse an :class:`AccessDecision` to the legacy boolean, failing closed.

        PROTOCOL_SPEC §6.8.1: a **decision** resolving to ``allow`` with
        ``approval_required: true`` MUST make :meth:`check` return ``False``.
        The decision and not the matched rule, since spec v1.29.0: the
        requirement may be a pending one raised by an unevaluable rule that did
        not itself match, or carried through ``default_effect: allow`` (§6.1.1
        rule 5), and the boolean fails closed on those identically. Reading
        :attr:`AccessDecision.approval_required` rather than the rule is what
        makes that automatic.

        ``check()`` is public API consumed by callers that are not the Executor
        — tooling, preflight helpers, third-party integrations — and such a
        caller can only read a boolean as "let it through / do not". Returning
        True would run a call the ACL said needed a human. False is wrong in the
        benign direction: the caller sees a refusal where the truth was "ask
        first". The Executor reads :meth:`check_access` and is unaffected, and a
        legacy caller only meets this at all once an operator has authored a
        rule carrying ``approval``.
        """
        return decision.access == "allow" and not decision.approval_required

    async def async_check(
        self,
        caller_id: str | None,
        target_id: str,
        context: Context | None = None,
        *,
        projection: Any | None = None,
    ) -> bool:
        """Async ACL check. Supports both sync and async condition handlers.

        Same §6.1.1 three-outcome contract as :meth:`check`. One situation
        cannot arise here — an async handler unresolvable on the sync path —
        since this path awaits genuine async handlers; the async registry is
        also consulted first, so an async-only key resolves here and not there.

        Args:
            caller_id: The calling module ID, or None for external calls.
            target_id: The target module ID being called.
            context: Optional execution context for conditional rules.

        Returns:
            True if the call is allowed, False if denied **or if it is allowed
            but requires approval** — see :meth:`async_check_access`.
        """
        return self._as_legacy_boolean(
            await self.async_check_access(caller_id, target_id, context, projection=projection)
        )

    async def async_check_access(
        self,
        caller_id: str | None,
        target_id: str,
        context: Context | None = None,
        *,
        projection: Any | None = None,
    ) -> AccessDecision:
        """Async :meth:`check_access` (PROTOCOL_SPEC §6.8.1).

        Same structured result and the same single audit entry; the async
        condition registry is consulted first, exactly as in
        :meth:`async_check`.
        """
        effective_caller, rules, default_effect, audit_sink = self._snapshot(caller_id)

        token = _handler_error_var.set({})
        projection_token = _projection_var.set(projection)
        try:
            matched: tuple[int, ACLRule] | None = None
            pending_approval = False
            for idx, rule in enumerate(rules):
                outcome = await self._matches_rule_async(rule, effective_caller, target_id, context, rule_index=idx)
                if outcome is ConditionOutcome.UNEVALUABLE:
                    if self._unevaluable_rule_takes_effect(rule):
                        matched = (idx, rule)
                        break
                    # Same §6.1.1 rule 5 bookkeeping as the sync twin; the two
                    # entry points MUST NOT drift on a governance result.
                    pending_approval = pending_approval or self._raises_pending_approval(rule)
                    continue
                if outcome is ConditionOutcome.SATISFIED:
                    matched = (idx, rule)
                    break

            return self._finalize_check(
                log_method="async_check",
                caller_id=caller_id,
                effective_caller=effective_caller,
                target_id=target_id,
                rules_present=bool(rules),
                default_effect=default_effect,
                matched=matched,
                pending_approval=pending_approval,
                audit_sink=audit_sink,
                context=context,
            )
        finally:
            _projection_var.reset(projection_token)
            _handler_error_var.reset(token)

    def _snapshot(self, caller_id: str | None) -> tuple[str, list[ACLRule], str, "_AuditSink"]:
        """Atomically snapshot mutable state for a check call.

        Hands out the SINK rather than the raw callback: delivery has to be
        contained (§6.3.2 requirement 3) and the failure state that scopes
        requirement 5's suppression lives on the sink, so a call site holding a
        bare callable could not honour either.
        """
        effective_caller = "@external" if caller_id is None else caller_id
        with self._lock:
            return effective_caller, list(self._rules), self._default_effect, self._audit_sink

    def _finalize_check(
        self,
        *,
        log_method: str,
        caller_id: str | None,
        effective_caller: str,
        target_id: str,
        rules_present: bool,
        default_effect: str,
        matched: tuple[int, ACLRule] | None,
        pending_approval: bool,
        audit_sink: "_AuditSink",
        context: Context | None,
    ) -> AccessDecision:
        """Log the decision, emit an audit entry, and return the structured result.

        Shared by both check_access() and async_check_access() so that audit +
        logging logic lives in exactly one place.

        The approval requirement is the matched rule's own **union any pending
        requirement** raised during the scan (PROTOCOL_SPEC §6.9 rows 1-2 as
        amended in spec v1.29.0): there is no default approval *source*, so an
        unremarkable no-match still means ``False``, but a requirement raised by
        an unevaluable ``allow`` rule composes with whatever granted — including
        ``default_effect: allow``, which is the one route with no rule to carry
        it. A ``deny`` rule can never carry one of its own; §6.1.6 rejects that
        pair at construction.
        """
        if matched is not None:
            matched_idx, matched_rule = matched
            # PROTOCOL_SPEC §6.1.5: read the effect, do not *resolve* it. This
            # was ``"allow" if effect == "allow" else "deny"``, which resolved
            # any other string to a decision — the reading §6.1.5 forbids,
            # because under ``default_effect: allow`` it flips a rule written to
            # permit into one that denies everything it matches, with no error.
            # The value set is closed at every door that accepts a rule, so the
            # only way to arrive here out of enum is to assign ``rule.effect``
            # on an already-constructed dataclass. That raises rather than
            # deciding: an unrecognised effect has no decision, and a raised
            # ACLRuleError is how this SDK already says "not a rule I can act
            # on". It is not the §6.1.1 "MUST NOT raise" case, which is about a
            # condition that could not be *evaluated* — this value could not be
            # *read*.
            _validate_effect(matched_rule.effect, where=f"Rule {matched_idx}")
            access = matched_rule.effect
            approval_required = matched_rule.approval == APPROVAL_REQUIRED
            rule_label: str = matched_rule.description or "(no description)"
            reason = "rule_match"
        else:
            matched_idx = None
            matched_rule = None
            # Same rule one field up (§6.1.5). No guard is needed here: this
            # value comes from :meth:`__init__`, which is its only writer and
            # validates it against the same set.
            access = default_effect
            approval_required = False
            rule_label = "default"
            reason = "default_effect" if rules_present else "no_rules"

        # §6.1.1 rule 5 (spec v1.29.0, #109). Composed here rather than inside
        # either branch above because a pending requirement outlives the rule
        # that carried it: it rides through a later ``allow`` rule *and* through
        # ``default_effect: allow``, which is what makes ``approval_required:
        # True`` with ``matched_rule_index: None`` a legal combination. A denial
        # clears it — "denied *and* put it to a human" is the same meaningless
        # state §6.1.6 rejects on a rule — while ``matched_rule_index`` keeps
        # naming the rule that actually decided, not the unevaluable one.
        approval_required = access == "allow" and (approval_required or pending_approval)

        self._logger.debug(
            "ACL %s: caller_id=%s target_id=%s decision=%s approval_required=%s rule=%s",
            log_method,
            caller_id,
            target_id,
            access,
            approval_required,
            rule_label,
        )

        if audit_sink.is_active:
            audit_sink.deliver(
                self._build_audit_entry(
                    caller_id=effective_caller,
                    target_id=target_id,
                    decision=access,
                    reason=reason,
                    matched_rule=matched_rule,
                    matched_rule_index=matched_idx,
                    context=context,
                    approval_required=approval_required,
                )
            )

        return AccessDecision(
            access=access,
            approval_required=approval_required,
            matched_rule_index=matched_idx,
            reason=reason,
        )

    def _match_patterns(self, patterns: list[str], value: str, context: Context | None = None) -> bool:
        """Match a list of patterns against a value.

        Implements the compound operators ``$or`` / ``$not`` at index 0 of a
        pattern array (PROTOCOL_SPEC §6.2.1). The array is FLAT: index 0 is the
        only operator position and every later element is a plain pattern.

        The ``False`` returns below for an operand-less array are kept as
        **defence in depth only**. Since spec v1.31.0 every entry point rejects
        that shape and :meth:`_precheck_patterns` classifies whatever reaches
        evaluation around them as unevaluable, so no such array should arrive
        here — and reading an arity fault as a scope decision, which is what
        these returns used to be, is #112 itself.
        """
        if not patterns:
            return False

        # Check for compound operators
        first = patterns[0]
        if first == "$or":
            return any(self._match_pattern(p, value, context) for p in patterns[1:])
        if first == "$not":
            # $not expects exactly one subsequent pattern
            if len(patterns) < 2:
                return False
            return not self._match_pattern(patterns[1], value, context)

        # Standard OR behavior for flat list
        return any(self._match_pattern(p, value, context) for p in patterns)

    def _build_audit_entry(
        self,
        *,
        caller_id: str,
        target_id: str,
        decision: str,
        reason: str,
        matched_rule: ACLRule | None,
        matched_rule_index: int | None,
        context: Context | None,
        approval_required: bool = False,
    ) -> AuditEntry:
        """Build an AuditEntry, extracting optional fields from context."""
        identity_type: str | None = None
        roles: tuple[str, ...] = ()
        call_depth: int | None = None
        trace_id: str | None = None

        if context is not None:
            trace_id = context.trace_id
            call_depth = len(context.call_chain)
            if context.identity is not None:
                identity_type = context.identity.type
                roles = tuple(context.identity.roles)

        return AuditEntry(
            timestamp=datetime.now(timezone.utc).isoformat(),
            caller_id=caller_id,
            target_id=target_id,
            decision=decision,
            reason=reason,
            matched_rule=matched_rule.description if matched_rule is not None else None,
            matched_rule_index=matched_rule_index,
            identity_type=identity_type,
            roles=roles,
            call_depth=call_depth,
            trace_id=trace_id,
            handler_error=_handler_error_message(),
            approval_required=approval_required,
        )

    def _match_pattern(self, pattern: str, value: str, context: Context | None = None) -> bool:
        """Match a single pattern against a value, with special pattern handling.

        Handles @external and @system patterns locally, delegates all
        other patterns to the foundation match_pattern() utility.
        """
        if pattern == "@external":
            return value == "@external"
        if pattern == "@system":
            return context is not None and context.identity is not None and context.identity.type == "system"
        return match_pattern(pattern, value)

    def _matches_rule(
        self,
        rule: ACLRule,
        caller: str,
        target: str,
        context: Context | None,
        *,
        rule_index: int | None = None,
    ) -> ConditionOutcome:
        """Resolve a single rule against the caller and target (three-valued).

        Returns ``SATISFIED`` when the rule matches, ``UNSATISFIED`` when it does
        not, and ``UNEVALUABLE`` when the rule is malformed or its conditions
        could not be evaluated (PROTOCOL_SPEC §6.1.1) — which the caller resolves
        toward refusing access.

        Order is normative (§6.1.4 rule 4):

        a. The **structure** of ``callers`` / ``targets`` is prechecked before any
           pattern is read, because reading a malformed one is the §6.1.4.1
           fail-open.
        b. A malformed pattern field makes the rule unevaluable — its scope is
           unknowable, so it resolves per §6.1.1's effect table.
        c. Both fields well-formed and either failing to match means the rule
           **does not apply to this call**. Its conditions are not consulted, its
           faults do not reach ``handler_error``, and it does not change this
           decision. Otherwise one misspelled key in a narrowly scoped rule would
           decide calls it was never written about — ``callers: ["api.*"]`` with
           a typo'd condition denying a ``worker.*`` caller — which breaks
           first-match-wins. The fault is still real; :meth:`validate_rules`
           looks at every rule and no call, and is where it surfaces.
        d. Only then is the conditions tree prechecked, in full and context-free,
           **before** §6.5's no-context check. That ordering closes the bypass
           where ``conditions: {mispelled: true}`` on a ``deny`` rule passed
           traffic simply because the caller carried no identity.

        A rule that *passes* the precheck and then finds no context takes §6.5's
        path and does not match. ``roles`` is answerable in principle; this
        caller merely supplied no input for it.
        """
        pattern_faults = self._precheck_patterns(rule)
        if pattern_faults:
            self._report_faults(pattern_faults, rule_index, rule)
            return ConditionOutcome.UNEVALUABLE

        if not self._match_patterns(rule.callers, caller, context):
            return ConditionOutcome.UNSATISFIED

        if not self._match_patterns(rule.targets, target, context):
            return ConditionOutcome.UNSATISFIED

        if rule.conditions is None:
            return ConditionOutcome.SATISFIED

        return self._resolve_conditions(rule, context, rule_index, async_path=False)

    async def _matches_rule_async(
        self,
        rule: ACLRule,
        caller: str,
        target: str,
        context: Context | None,
        *,
        rule_index: int | None = None,
    ) -> ConditionOutcome:
        """Async version of :meth:`_matches_rule`, using the async evaluator.

        The precheck resolves against the async registry with a fallback to the
        sync one (§6.1.3), so an async-only handler is a fault on the sync path
        and not on this one.
        """
        pattern_faults = self._precheck_patterns(rule)
        if pattern_faults:
            self._report_faults(pattern_faults, rule_index, rule)
            return ConditionOutcome.UNEVALUABLE

        if not self._match_patterns(rule.callers, caller, context):
            return ConditionOutcome.UNSATISFIED

        if not self._match_patterns(rule.targets, target, context):
            return ConditionOutcome.UNSATISFIED

        if rule.conditions is None:
            return ConditionOutcome.SATISFIED

        faults = self._precheck_conditions(rule.conditions, async_path=True)
        if faults:
            self._report_faults(faults, rule_index, rule)
            return ConditionOutcome.UNEVALUABLE

        if context is None:
            self._warn_conditional_rule_without_context(rule_index, rule.effect)
            return ConditionOutcome.UNSATISFIED

        before = self._recorded_condition_paths()
        outcome = await self._evaluate_conditions_async(rule.conditions, context)
        if outcome is ConditionOutcome.UNEVALUABLE:
            self._warn_unevaluable_conditions(rule_index, rule, before)
        return outcome

    def _resolve_conditions(
        self,
        rule: ACLRule,
        context: Context | None,
        rule_index: int | None,
        *,
        async_path: bool,
    ) -> ConditionOutcome:
        """Precheck, then the §6.5 context check, then evaluate — in that order.

        A missing context is deliberately NOT one of §6.1.1's unevaluable
        situations: calling with no context is a legitimate shape for external
        entry points, not a misconfiguration, and treating it as a failure would
        flip the decision for every ``@external`` call meeting a conditional
        ``deny`` rule (PROTOCOL_SPEC §6.5). It stays a plain non-match, with a
        warning so the consequence is visible. The precheck runs first precisely
        so that a *malformed* rule does not reach that lenient path.
        """
        assert rule.conditions is not None  # guarded by both callers
        faults = self._precheck_conditions(rule.conditions, async_path=async_path)
        if faults:
            self._report_faults(faults, rule_index, rule)
            return ConditionOutcome.UNEVALUABLE

        if context is None:
            self._warn_conditional_rule_without_context(rule_index, rule.effect)
            return ConditionOutcome.UNSATISFIED

        before = self._recorded_condition_paths()
        outcome = self._evaluate_conditions(rule.conditions, context)
        if outcome is ConditionOutcome.UNEVALUABLE:
            self._warn_unevaluable_conditions(rule_index, rule, before)
        return outcome

    def _report_faults(self, faults: list[_Fault], rule_index: int | None, rule: ACLRule) -> None:
        """Record precheck faults into ``handler_error`` and warn (§6.1.1 rules 2-3)."""
        for fault in faults:
            _record_handler_error(fault.path, fault.reason)
        self._logger.warning(
            "ACL rule %s (effect=%s) is unevaluable — PROTOCOL_SPEC §6.1.4 precheck failed at %s. "
            "A 'deny' rule takes effect and the call is denied; an 'allow' rule does not grant.%s",
            "?" if rule_index is None else rule_index,
            rule.effect,
            "; ".join(f"{f.path}: {f.reason}" for f in sorted(faults, key=lambda f: f.path)),
            self._pending_approval_clause(rule),
        )

    @staticmethod
    def _recorded_condition_paths() -> frozenset[str]:
        """Snapshot the condition paths already diagnosed in this check()."""
        errors = _handler_error_var.get()
        return frozenset(errors) if errors else frozenset()

    def _warn_unevaluable_conditions(self, rule_index: int | None, rule: ACLRule, before: frozenset[str]) -> None:
        """Warn that a rule's conditions were unevaluable during execution (§6.1.1 rule 3).

        The message names the condition path(s), the rule's index and the rule's
        ``effect``; the ``effect`` is required because a misconfigured ``deny``
        rule is the consequential case. Only paths diagnosed *by this rule* are
        listed — ``before`` filters out ones an earlier rule already reported.
        """
        paths = sorted(self._recorded_condition_paths() - before)
        self._logger.warning(
            "ACL rule %s (effect=%s) has unevaluable condition(s) %s — PROTOCOL_SPEC §6.1.1: "
            "a 'deny' rule takes effect and the call is denied, an 'allow' rule does not grant.%s",
            "?" if rule_index is None else rule_index,
            rule.effect,
            ", ".join(repr(path) for path in paths) if paths else "(unreported)",
            self._pending_approval_clause(rule),
        )

    @classmethod
    def _pending_approval_clause(cls, rule: ACLRule) -> str:
        """The tail both §6.1.1 warnings carry when the rule gated a human.

        Without it the message reads as though the rule had no further effect,
        which is exactly the reading §6.1.1 rule 5 corrects: the requirement it
        carried is still live, and an operator debugging why a forced push was
        approved — or why an ordinary one now asks — needs to be told which.
        """
        if not cls._raises_pending_approval(rule):
            return ""
        return (
            " The rule carried approval: required, so per §6.1.1 rule 5 that requirement is PENDING "
            "and composes with whatever grants this call, including default_effect: allow."
        )

    def _warn_conditional_rule_without_context(self, rule_index: int | None, effect: str) -> None:
        """Warn once that a conditional rule was skipped for want of a context (§6.5)."""
        marker = (rule_index, effect)
        if marker in self._warned_missing_context:
            return
        self._warned_missing_context.add(marker)
        self._logger.warning(
            "ACL rule %s (effect=%s) has conditions but the check supplied no context, so the "
            "rule does not match (PROTOCOL_SPEC §6.5). A conditional 'deny' rule is therefore "
            "not a backstop for context-less callers — express a backstop as an unconditional "
            "'deny' rule or as default_effect: deny.",
            "?" if rule_index is None else rule_index,
            effect,
        )

    @staticmethod
    def _unevaluable_rule_takes_effect(rule: ACLRule) -> bool:
        """Apply §6.1.1's effect rule to a rule whose conditions were unevaluable.

        Returns True when the rule takes effect (a ``deny`` rule matches and the
        call is denied), False when evaluation continues to the next rule (an
        ``allow`` rule MUST NOT grant). Either way the audit entry already
        carries ``handler_error`` and a warning has been emitted.
        """
        return rule.effect == "deny"

    @staticmethod
    def _raises_pending_approval(rule: ACLRule) -> bool:
        """Does this unevaluable ``allow`` rule leave a **pending** approval requirement?

        PROTOCOL_SPEC §6.1.1 rule 5 (spec v1.29.0, #109).
        :meth:`_unevaluable_rule_takes_effect` answers the authorization axis;
        this answers the second one §6.1.6 added. "MUST NOT grant" was a
        complete instruction while a rule carried a single axis — the rule steps
        aside, and stepping aside was harmless because whatever granted next
        also said ``allow``. It is not complete for a rule that also carried
        ``approval: required``: the narrow rule gating ``git push --force``
        steps aside, a broader ``allow`` grants, and the call the operator gated
        runs with no human asked. So the requirement is recorded and composed by
        disjunction with whatever grants later, rather than discarded.

        **Scope is a precondition, and the caller has already enforced it.** A
        rule whose ``callers`` / ``targets`` do not match resolves to
        UNSATISFIED in :meth:`_matches_rule` and never reaches here (§6.1.4
        rule 4), so a rule written about one caller cannot attach a human to
        calls it was never written about. A rule whose pattern field is itself
        **malformed** does reach here, from the §6.1.4.1 precheck that runs
        before any pattern is read — deliberately, per rule 5's last clause: its
        scope cannot be read, so it cannot be shown not to apply here, which is
        the posture that field already produces under ``deny``, where an
        unreadable scope denies every call.

        The ``effect == "allow"`` test is the exact complement of the caller's
        ``deny`` branch, and only because §6.1.5 closed the value set at every
        door in spec v1.30.0: there is no third value for a rule to carry, so
        "not deny" and "allow" are now the same question.
        """
        return rule.effect == "allow" and rule.approval == APPROVAL_REQUIRED

    def add_rule(
        self,
        rule: ACLRule | None = None,
        *,
        callers: list[str] | str | None = None,
        targets: list[str] | str | None = None,
        effect: str = "deny",
        description: str = "",
        conditions: dict[str, Any] | None = None,
        approval: str = APPROVAL_NOT_REQUIRED,
    ) -> None:
        """Add a rule at position 0 (highest priority).

        Args:
            rule: Optional pre-built ACLRule.
            callers: Caller pattern(s) if *rule* is None.
            targets: Target pattern(s) if *rule* is None.
            effect: ``"allow"`` or ``"deny"`` if *rule* is None — the set is
                closed (PROTOCOL_SPEC §6.1.5).
            description: Rule description if *rule* is None.
            conditions: Rule conditions if *rule* is None.
            approval: ``"required"`` or ``"not_required"`` (default) if *rule*
                is None — whether a matching call must be put to a human
                (PROTOCOL_SPEC §6.1.6).

        Raises:
            ACLRuleError: When ``effect`` is outside ``allow`` / ``deny``
                (§6.1.5), when ``approval="required"`` is combined with
                ``effect="deny"``, which §6.1.6 makes meaningless, or when
                ``callers`` / ``targets`` is outside §6.2.1's closed shape —
                empty, ``$or`` with no operand, ``$not`` with none or more than
                one, an empty pattern string, or a reserved token anywhere but
                index 0 (spec v1.31.0). Runtime insertion is one of the three
                entry points §6.1.6 rule 3 requires all of these checks at; this
                method returning ``None`` is not an exemption, so it signals the
                way Python signals a value that cannot be constructed.

                The rule is validated **here**, on the object this method is
                handed, and not only by :class:`ACLRule`'s own constructor
                (§6.2.1). A pre-built rule may have been well-formed when it was
                constructed and had ``callers`` or ``targets`` assigned since —
                :class:`ACLRule` is a non-frozen dataclass — and such a rule
                reaches this door without passing through any constructor.

        Note:
            A condition key with no registered handler warns and does not
            raise, exactly as on :meth:`load` (PROTOCOL_SPEC §6.1.2 rule 4).
            The warning names index ``0``, where the rule lands.
        """
        if rule is None:
            if callers is None or targets is None:
                raise ValueError("Must provide either 'rule' or both 'callers' and 'targets'")

            def _to_list(v: list[str] | str) -> list[str]:
                return [v] if isinstance(v, str) else list(v)

            rule = ACLRule(
                callers=_to_list(callers),
                targets=_to_list(targets),
                effect=effect,
                description=description,
                conditions=conditions,
                approval=approval,
            )

        # PROTOCOL_SPEC §6.2.1: runtime insertion re-validates the rule it is
        # HANDED, whatever that rule's history. Threading the check through
        # :meth:`ACLRule.__post_init__` alone reaches this door only *through
        # construction*, and :class:`ACLRule` is a non-frozen dataclass — so
        #
        #     r = ACLRule(callers=["*"], targets=["*"], effect="deny")
        #     r.targets = []
        #     acl.add_rule(r)
        #
        # walked straight past it and installed a rule §6.2.1 forbids. The
        # backstop in :meth:`_precheck_patterns` then made that rule UNEVALUABLE
        # at every subsequent check rather than a rule, which is the outcome the
        # door exists to prevent. An implementation MUST NOT rely on the rule
        # type's own construction-time check to cover this door; the call is
        # therefore unconditional and covers the kwargs path too, where it is
        # merely redundant — the predicate is pure, so a second evaluation costs
        # a comparison and cannot disagree with the first.
        _validate_rule(rule, where="ACLRule")

        with self._lock:
            self._rules.insert(0, rule)
            # The §6.5 once-per-rule warning is keyed by rule INDEX, and this
            # insertion at index 0 shifts every existing rule down by one — so
            # a stale marker would suppress the warning for a different rule
            # than the one it was recorded for. Start fresh, as
            # :meth:`reload` does and as apcore-typescript's ``addRule`` does.
            self._warned_missing_context.clear()

        # PROTOCOL_SPEC §6.1.2 rule 4: runtime insertion is an entry point that
        # MUST be covered, not just file loading. The rule lands at index 0, so
        # that is the index the warning names. Warn-never-fail, as on load.
        type(self)._warn_unregistered_condition_keys([rule])
        type(self)._warn_dead_question_patterns([rule])

    def remove_rule(
        self,
        callers: list[str],
        targets: list[str],
        conditions: dict | None = None,
    ) -> bool:
        """Remove the first rule matching the given callers, targets, and (optional) conditions.

        Args:
            callers: The caller patterns to match.
            targets: The target patterns to match.
            conditions: When provided, also disambiguate by ACLRule.conditions
                via deep equality. Two rules with identical callers+targets but
                different conditions can be selectively removed by passing the
                conditions to match. Cross-language parity with apcore-typescript
                removeRule (sync finding A-D-026).

        Returns:
            True if a rule was found and removed, False otherwise.
        """
        with self._lock:
            for i, rule in enumerate(self._rules):
                if rule.callers != callers or rule.targets != targets:
                    continue
                if conditions is not None and rule.conditions != conditions:
                    continue
                self._rules.pop(i)
                # D-88: the §6.5 once-per-rule warning is keyed by rule INDEX,
                # and removing rule `i` shifts every rule after it up by one —
                # so a stale marker would suppress the warning for a different
                # rule than the one it was recorded for. Any operation that
                # inserts, removes or reorders rules clears the set, as
                # :meth:`add_rule` and :meth:`reload` do.
                self._warned_missing_context.clear()
                return True
            return False

    def reload(self) -> None:
        """Re-read the ACL from the original YAML file.

        Only works if the ACL was created via ACL.load().
        Raises ACLRuleError if no YAML path was stored.
        """
        with self._lock:
            yaml_path = self._yaml_path
        if yaml_path is None:
            raise ACLRuleError("Cannot reload: ACL was not loaded from a YAML file")
        # The callback is NOT passed here: `reload` preserves the one this
        # instance already has (requirement 7), and handing it to the
        # temporary would emit requirement 1's override notice a second time
        # for a configuration the caller has already been told about.
        reloaded = ACL.load(yaml_path)
        with self._lock:
            self._rules = reloaded._rules
            self._default_effect = reloaded._default_effect
            # §6.3.2 requirement 7: a reload refreshes the `audit:` block and
            # PRESERVES a programmatic callback, which was never read from the
            # file. Before v1.45.0 this method refreshed only the rules and the
            # default effect, so the audit block was the one part of the
            # document a reload did not pick up.
            #
            # Building a NEW sink is also what scopes requirement 5's
            # once-per-failure suppression: a reload that changes the sink's
            # configuration starts a fresh report rather than hiding a new
            # failure behind an old one.
            self._audit_config = reloaded._audit_config
            self._audit_sink = _AuditSink(self._audit_logger, self._audit_config)
            _warn_audit_block_overridden(self._audit_logger, self._audit_config)
            # The §6.5 once-per-rule warning is keyed by rule index, which the
            # reload may have repointed at a different rule. Start fresh.
            self._warned_missing_context.clear()


# ---------------------------------------------------------------------------
# Auto-register compound operators at module load time
# ---------------------------------------------------------------------------
ACL.register_condition("$or", _OrHandler(ACL._evaluate_conditions))
ACL.register_condition("$not", _NotHandler(ACL._evaluate_conditions))

# Async-aware variants: used by async_check() so $or/$not properly await
# async sub-condition handlers (mirrors TypeScript registerAsyncCondition).
ACL.register_async_condition("$or", _OrHandlerAsync(ACL._evaluate_conditions_async))
ACL.register_async_condition("$not", _NotHandlerAsync(ACL._evaluate_conditions_async))
