"""Built-in ACL condition handlers and handler protocols.

Defines the ACLConditionHandler protocol (sync and async variants), the
three-valued :class:`ConditionOutcome` of PROTOCOL_SPEC §6.1.1, three basic
handlers (identity_types, roles, max_call_depth), and two compound operators
($or, $not) with both sync and async variants.

The compound operators are **path-aware** (§6.1.4): they receive the condition
path they sit at so a fault nested inside them can be reported as
``$or[1].$not.k`` rather than as a bare key. A key can occur at several
positions in one tree, which is why diagnostics are ordered by path and not by
key.
"""

from __future__ import annotations

import contextvars
from enum import Enum
from typing import Any, Awaitable, Callable, ClassVar, Mapping, Protocol, Union, runtime_checkable

from apcore.context import Context

__all__ = [
    "ConditionOutcome",
    "SyncACLConditionHandler",
    "AsyncACLConditionHandler",
    "ACLConditionHandler",
]


# CTX-2 — the governance projection of the call CURRENTLY being checked.
#
# §6.1.8 rule 4 leaves the delivery mechanism idiomatic, but rules 1 and 3 make
# the projection a property of the call, not of an object that outlives it. It
# used to be written onto the execution Context at Step 3 and left there, so a
# module or middleware holding that Context and calling `check_access` again
# later evaluated an `arguments` condition against the PREVIOUS call's argument
# key set. apcore-typescript and apcore-rust scope it to one ACL evaluation.
#
# Set for the duration of one check_access / async_check_access call and reset
# in `finally` by apcore.acl.ACL (the only writer); this module only reads it.
# Lives here, next to `_ArgumentsHandler` which is its only reader, so that
# apcore.acl -> apcore.acl_handlers stays a one-way dependency.
_projection_var: contextvars.ContextVar[Any | None] = contextvars.ContextVar(
    "_apcore_acl_governance_projection", default=None
)


def current_governance_projection(context: Any = None) -> Any | None:
    """The projection in force for the ACL evaluation now running, or None.

    Falls back to ``context.governance_projection`` so that an ACL constructed
    by a host that carries the projection on its own Context — the other shape
    §6.1.8 rule 4 blesses — keeps working.
    """
    projection = _projection_var.get()
    if projection is not None:
        return projection
    return getattr(context, "governance_projection", None)


class ConditionOutcome(Enum):
    """Three-valued result of evaluating an ACL condition (PROTOCOL_SPEC §6.1.1).

    A condition that **is false** and a condition that **cannot be evaluated**
    are different outcomes and MUST NOT be represented the same way: on a
    ``deny`` rule the first means "this rule does not apply" while the second
    means "deny". Collapsing them into a boolean is the defect §6.1.1 exists to
    prevent — a misspelled key (``role:`` for ``roles:``) turned a ``deny`` rule
    its author believed was blocking into decoration.

    Values:
        SATISFIED: A registered handler ran to completion and answered "yes".
        UNSATISFIED: A registered handler ran to completion and answered "no".
        UNEVALUABLE: The condition cannot be answered **as written**. This is a
            **principle, not a closed list** (§6.1.1). The situations every
            implementation meets: the key has no handler resolvable on the path
            in use; the handler raised; the handler was asynchronous and could
            not be resolved on the synchronous ``check()`` path; the value is
            malformed for its key; ``conditions`` itself is not a mapping. An
            implementation meeting an unlisted case classifies it by the
            principle, never by defaulting it to UNSATISFIED.

    A custom condition handler MAY return a :class:`ConditionOutcome` directly
    to report ``UNEVALUABLE`` for its own reasons; returning a plain ``bool``
    keeps the ordinary satisfied/unsatisfied meaning.
    """

    SATISFIED = "satisfied"
    UNSATISFIED = "unsatisfied"
    UNEVALUABLE = "unevaluable"


#: Path of the ``conditions`` object itself (PROTOCOL_SPEC §6.1.4). The JSONPath
#: root, used when the fault is the whole block rather than a key inside it —
#: it is the only path with no key component, and it keeps the notation
#: consistent with ``$or[1].$not.k``.
ROOT_CONDITION_PATH = "$"


def join_condition_path(prefix: str, key: str) -> str:
    """Compose a §6.1.4 condition path from a prefix and a key.

    ``("", "roles")`` -> ``roles``; ``("$or[1]", "k")`` -> ``$or[1].k``;
    ``("$not", "k")`` -> ``$not.k``. Paths nest, so a key inside ``$not`` inside
    the second ``$or`` branch is ``$or[1].$not.k``.
    """
    return f"{prefix}.{key}" if prefix else key


def _as_outcome(value: Any) -> ConditionOutcome:
    """Coerce a handler return value to a :class:`ConditionOutcome`.

    A handler that already speaks the three-valued vocabulary is passed through
    untouched; anything else keeps the historical boolean meaning.
    """
    if isinstance(value, ConditionOutcome):
        return value
    return ConditionOutcome.SATISFIED if value else ConditionOutcome.UNSATISFIED


@runtime_checkable
class SyncACLConditionHandler(Protocol):
    """Sync condition handler protocol."""

    def evaluate(self, value: Any, context: Context) -> bool | ConditionOutcome: ...


@runtime_checkable
class AsyncACLConditionHandler(Protocol):
    """Async condition handler protocol."""

    async def evaluate(self, value: Any, context: Context) -> bool | ConditionOutcome: ...


ACLConditionHandler = Union[SyncACLConditionHandler, AsyncACLConditionHandler]

# Type alias for the recursive evaluation function used by compound handlers.
# Compound operators recurse through the three-valued evaluator so that an
# unevaluable sub-condition propagates per §6.1.1's composition table rather
# than being flattened into "false" one nesting level down.
_EvalFn = Callable[[dict[str, Any], Context, str], ConditionOutcome]
_AsyncEvalFn = Callable[[dict[str, Any], Context, str], Awaitable[ConditionOutcome]]


# ---------------------------------------------------------------------------
# Basic handlers
# ---------------------------------------------------------------------------


class _IdentityTypesHandler:
    """Check context.identity.type matches allowed value(s).

    Per spec, identity_types condition value MUST be a list.
    """

    def evaluate(self, value: Any, context: Context) -> bool:
        if context.identity is None:
            return False
        if not isinstance(value, list):
            return False
        return context.identity.type in value


class _RolesHandler:
    """Check role overlap between identity and required roles.

    Per spec, roles condition value MUST be a list.
    """

    def evaluate(self, value: Any, context: Context) -> bool:
        if context.identity is None:
            return False
        if not isinstance(value, list):
            return False
        return bool(set(context.identity.roles) & set(value))


class _MaxCallDepthHandler:
    """Check call chain length does not exceed threshold."""

    def evaluate(self, value: Any, context: Context) -> bool:
        threshold = value.get("lte") if isinstance(value, dict) else value
        # Reject bool explicitly: Python ``bool`` is a subclass of ``int``, so a
        # YAML ``max_call_depth: true`` would coerce to threshold=1 and could
        # ALLOW a shallow call where TS/Rust fail closed. Fail closed here too.
        if isinstance(threshold, bool):
            return False
        # A-D-004: accept an integral numeric threshold consistently across SDKs.
        # YAML/JSON may surface an integer depth as a float (``5.0``); TS has no
        # int/float distinction and treats ``5.0`` as 5, so Python must too —
        # otherwise the rule silently fails to match and falls through to deny.
        # A non-integral float (``5.5``) is a meaningless depth and fails closed,
        # mirroring TypeScript's ``Number.isInteger`` guard.
        if isinstance(threshold, float):
            if not threshold.is_integer():
                return False
            threshold = int(threshold)
        elif not isinstance(threshold, int):
            return False
        return len(context.call_chain) <= threshold


#: The complete predicate vocabulary of the ``arguments`` condition
#: (PROTOCOL_SPEC §6.1.7). Closed for the same reason §6.1.5 closes the rule key
#: set: a predicate nothing evaluates would be dropped, and an ``arguments``
#: block whose only predicate was dropped is vacuously true — which widens an
#: ``allow`` rule in silence.
ARGUMENT_PREDICATES: frozenset[str] = frozenset({"has_key", "has_all_keys", "has_none_of"})


def validate_arguments_condition(value: Any) -> list[tuple[str, str]]:
    """Structurally validate an ``arguments`` condition value (§6.1.7).

    Context-independent and value-free, so §6.1.4's precheck can run it before
    any handler and produce the same findings in every implementation.

    Returns:
        An empty list when the value is well-formed, otherwise one
        ``(path_suffix, reason)`` pair per fault. ``path_suffix`` is
        ``".<predicate>"`` when a predicate is at fault and ``""`` when none can
        be named, which is §6.1.8's condition-path rule: the finding descends to
        the offending predicate exactly as §6.1.4 descends into ``$or[1].k``,
        because ``arguments`` alone does not say which of several predicates is
        wrong. The predicate is named **as written**, so a misspelling is
        searchable.

        **Every** faulty predicate is reported and the walk does not stop at the
        first (§6.1.8 rule 3). Reporting one would make the *choice* of which
        one observable, and no reading of §6.1.4 makes "the first" a pure
        function of the rule: ``{"has_key": "force", "zzz": [...]}`` carries both
        a malformed value and an unrecognised name, and checking names before
        values reports a different path from walking predicate by predicate.

        A fault makes the condition UNEVALUABLE (§6.1.1), never false:
        ``has_key: "force"`` written for ``has_key: ["force"]`` asks no question
        the implementation can answer, and answering "no" would put an
        ``allow`` rule's ``has_none_of`` typo back into the silently-inert state
        §6.1.1 exists to end.
    """
    if not isinstance(value, Mapping):
        # Terminal: a value that is not a mapping has no predicates to walk.
        return [("", f"arguments value must be a mapping, got {type(value).__name__}")]
    if not value:
        # Fail-closed for the reason §6.1 gives an empty `$not`: a predicate
        # block that constrains nothing asks no question, and reading it as
        # "satisfied" would make an `allow` rule carrying it grant on every
        # call — which is precisely what an author who deleted the last
        # predicate by accident did not mean.
        return [("", "arguments block is empty; it constrains nothing and asks no question")]
    faults: list[tuple[str, str]] = []
    # Sorted, so the finding ORDER is a pure function of the rule rather than of
    # dict insertion order (§6.1.4 determinism).
    for predicate in sorted(value):
        names = value[predicate]
        if predicate not in ARGUMENT_PREDICATES:
            faults.append(
                (
                    f".{predicate}",
                    f"unknown arguments predicate {predicate!r}; "
                    f"the vocabulary is closed ({', '.join(sorted(ARGUMENT_PREDICATES))})",
                )
            )
            continue
        # A bare string is iterable, so `has_key: "force"` would otherwise be
        # read character by character — the §6.1.4.1 failure one level down.
        if not isinstance(names, list) or not all(isinstance(name, str) for name in names):
            faults.append(
                (
                    f".{predicate}",
                    f"arguments.{predicate} must be a list of strings, got {type(names).__name__}",
                )
            )
    return faults


class _ArgumentsHandler:
    """The built-in ``arguments`` condition (PROTOCOL_SPEC §6.1.7).

    Structure-only. ``has_key`` passes when **any** named key is present,
    ``has_all_keys`` when **every** one is, ``has_none_of`` when **none** is;
    several predicates in one block are AND-ed like any other condition object.
    No predicate reads a value, and that is a design constraint rather than a
    first cut: the argument view here is not reliably redacted, and the
    arguments have not been schema-validated (the ACL is Step 4, validation is
    Step 7), so key presence is the one question well-defined on what is
    available.

    It is **built-in with no registration point**: a deployment-registered
    argument handler would be exactly the unauditable host code §7.9.6 rule 2
    keeps out of a governance verdict, and a fixed vocabulary keeps the decision
    reproducible from the ACL document.

    It reads :attr:`~apcore.context.Context.governance_projection` (§6.1.8), not
    the raw arguments and not ``redacted_inputs``. An absent projection is
    UNEVALUABLE, never "no arguments": treating it as an empty key set would
    make ``has_none_of`` vacuously true, and an ``allow`` rule gated on
    ``has_none_of`` would then grant on every call made through an entry point
    that populates no projection.
    """

    def evaluate(self, value: Any, context: Context) -> ConditionOutcome:
        if validate_arguments_condition(value):
            return ConditionOutcome.UNEVALUABLE

        # CTX-2 — the projection in force for THIS evaluation, not whatever a
        # previous call happened to leave on the Context. §6.1.8 rule 1 makes it
        # a property of the call being checked; `current_governance_projection`
        # still falls back to the Context field for a host that carries it
        # there, the other shape rule 4 blesses.
        projection = current_governance_projection(context)
        if projection is None:
            return ConditionOutcome.UNEVALUABLE
        present = projection.keys

        for predicate, names in value.items():
            if predicate == "has_key":
                satisfied = any(name in present for name in names)
            elif predicate == "has_all_keys":
                satisfied = all(name in present for name in names)
            else:  # has_none_of — the vocabulary is closed and validated above
                satisfied = not any(name in present for name in names)
            if not satisfied:
                return ConditionOutcome.UNSATISFIED
        return ConditionOutcome.SATISFIED


# ---------------------------------------------------------------------------
# Compound handlers
# ---------------------------------------------------------------------------
#
# PROTOCOL_SPEC §6.1.1 composition table (three-valued / Kleene logic):
#
#   $or  — any child SATISFIED                 -> SATISFIED
#          no child SATISFIED, >=1 UNEVALUABLE -> UNEVALUABLE
#          every child UNSATISFIED             -> UNSATISFIED
#   $not — child SATISFIED                     -> UNSATISFIED
#          child UNSATISFIED                   -> SATISFIED
#          child UNEVALUABLE                   -> UNEVALUABLE
#
# Short-circuiting is permitted on the decisive child (SATISFIED for $or) but
# MUST NOT happen on an UNEVALUABLE one: a later sibling may still decide it.
# Short-circuiting applies to handler *execution* only — §6.1.4's precheck walks
# the whole tree first, so a malformed operator or a misspelled key is always
# found and always reported, whatever the execution order turns out to be.
#
# A malformed operand — `$or` that is not a list, `$not` that is not an object,
# an `$or` element that is not an object — is UNEVALUABLE, not UNSATISFIED
# (§6.1.1 case 4). A handler handed `$or: "not-a-list"` can return false and
# look exactly like one that answered "no"; recording that as UNSATISFIED puts a
# `deny` rule carrying `$or: "typo"` back into the inert state §6.1.1 exists to
# end. In practice §6.1.4's precheck catches these before any handler runs;
# the guards below are the same rule applied where the evaluator is entered
# directly.


class _CompoundHandler:
    """Base for the built-in compound operators, which are **path-aware**.

    An ordinary handler answers about one key and needs no path. ``$or`` and
    ``$not`` recurse, so a fault beneath them has to be reported at its position
    in the tree (``$or[1].k``) rather than as a bare key. The evaluator detects
    the capability through :attr:`path_aware` and calls :meth:`evaluate_at`.
    """

    path_aware: ClassVar[bool] = True

    def evaluate_at(self, value: Any, context: Context, path: str) -> ConditionOutcome:
        raise NotImplementedError

    def evaluate(self, value: Any, context: Context) -> ConditionOutcome:
        """Path-less entry point, for a caller invoking the handler directly."""
        return self.evaluate_at(value, context, "")


class _AsyncCompoundHandler:
    """Async counterpart of :class:`_CompoundHandler`."""

    path_aware: ClassVar[bool] = True

    async def evaluate_at(self, value: Any, context: Context, path: str) -> ConditionOutcome:
        raise NotImplementedError

    async def evaluate(self, value: Any, context: Context) -> ConditionOutcome:
        return await self.evaluate_at(value, context, "")


class _OrHandler(_CompoundHandler):
    """$or: list of condition dicts. SATISFIED if ANY sub-set is satisfied."""

    def __init__(self, evaluate_fn: _EvalFn) -> None:
        self._evaluate = evaluate_fn

    def evaluate_at(self, value: Any, context: Context, path: str) -> ConditionOutcome:
        if not isinstance(value, list):
            return ConditionOutcome.UNEVALUABLE
        saw_unevaluable = False
        for index, sub in enumerate(value):
            if not isinstance(sub, dict):
                # An element that is not a condition object asks no question.
                saw_unevaluable = True
                continue
            outcome = self._evaluate(sub, context, f"{path}[{index}]")
            if outcome is ConditionOutcome.SATISFIED:
                return ConditionOutcome.SATISFIED
            if outcome is ConditionOutcome.UNEVALUABLE:
                saw_unevaluable = True
        return ConditionOutcome.UNEVALUABLE if saw_unevaluable else ConditionOutcome.UNSATISFIED


class _NotHandler(_CompoundHandler):
    """$not: single condition dict. SATISFIED if the sub-set is UNSATISFIED.

    ``$not`` of an UNEVALUABLE child is UNEVALUABLE, never SATISFIED: negating
    "no answer" into "yes" would let a misspelled key inside a ``$not`` satisfy
    the very rule it was meant to gate.
    """

    def __init__(self, evaluate_fn: _EvalFn) -> None:
        self._evaluate = evaluate_fn

    def evaluate_at(self, value: Any, context: Context, path: str) -> ConditionOutcome:
        if not isinstance(value, dict):
            return ConditionOutcome.UNEVALUABLE
        outcome = self._evaluate(value, context, path)
        if outcome is ConditionOutcome.UNEVALUABLE:
            return ConditionOutcome.UNEVALUABLE
        # An empty object evaluates to SATISFIED, so ``$not: {}`` is UNSATISFIED
        # (fail-closed), as PROTOCOL_SPEC §6.1 requires.
        return ConditionOutcome.UNSATISFIED if outcome is ConditionOutcome.SATISFIED else ConditionOutcome.SATISFIED


# ---------------------------------------------------------------------------
# Async compound handlers (for use with async_check / _evaluate_conditions_async)
# ---------------------------------------------------------------------------


class _OrHandlerAsync(_AsyncCompoundHandler):
    """Async $or: list of condition dicts. SATISFIED if ANY sub-set is satisfied.

    Mirrors TypeScript's OrHandlerAsync — uses the async evaluation path so
    async sub-condition handlers are properly awaited.
    """

    def __init__(self, evaluate_fn: _AsyncEvalFn) -> None:
        self._evaluate = evaluate_fn

    async def evaluate_at(self, value: Any, context: Context, path: str) -> ConditionOutcome:
        if not isinstance(value, list):
            return ConditionOutcome.UNEVALUABLE
        saw_unevaluable = False
        for index, sub in enumerate(value):
            if not isinstance(sub, dict):
                saw_unevaluable = True
                continue
            outcome = await self._evaluate(sub, context, f"{path}[{index}]")
            if outcome is ConditionOutcome.SATISFIED:
                return ConditionOutcome.SATISFIED
            if outcome is ConditionOutcome.UNEVALUABLE:
                saw_unevaluable = True
        return ConditionOutcome.UNEVALUABLE if saw_unevaluable else ConditionOutcome.UNSATISFIED


class _NotHandlerAsync(_AsyncCompoundHandler):
    """Async $not: single condition dict. SATISFIED if the sub-set is UNSATISFIED.

    Mirrors TypeScript's NotHandlerAsync — uses the async evaluation path so
    async sub-condition handlers are properly awaited. As on the sync path,
    ``$not`` of an UNEVALUABLE child stays UNEVALUABLE.
    """

    def __init__(self, evaluate_fn: _AsyncEvalFn) -> None:
        self._evaluate = evaluate_fn

    async def evaluate_at(self, value: Any, context: Context, path: str) -> ConditionOutcome:
        if not isinstance(value, dict):
            return ConditionOutcome.UNEVALUABLE
        outcome = await self._evaluate(value, context, path)
        if outcome is ConditionOutcome.UNEVALUABLE:
            return ConditionOutcome.UNEVALUABLE
        return ConditionOutcome.UNSATISFIED if outcome is ConditionOutcome.SATISFIED else ConditionOutcome.SATISFIED
