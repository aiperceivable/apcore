"""Execution-time governance policy: external overrides for governance annotations.

RFC pilot for apcore#76 ("first-class external governance / policy API").
An :class:`ExecutionPolicy` lets a platform operator override the governance
annotations (``requires_approval``, ``destructive``) of already-registered
modules at execution time, independent of how the modules were registered.
It attaches to the :class:`~apcore.executor.Executor` (``policy=`` parameter)
and is consulted by the approval gate (Step 5).

Precedence: a matched policy rule overrides the module's own declared or
scanned annotations — external governance is the platform's word against the
module author's. Matching uses the same wildcard semantics as the ACL system
(Algorithm A08) and the same specificity scoring (Algorithm A10); on a
specificity tie the more restrictive rule wins.

Governance principle (apcore#76): a misconfigured or unreachable governance
control must warn or error, never silently allow. ``from_dict`` therefore
rejects unknown keys (a typo in a governance file must not silently no-op),
and ``strict=True`` makes the approval gate fail closed when approval is
required but no ApprovalHandler is configured.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

from apcore.context import _APPROVAL_TOKEN_KEY
from apcore.utils.pattern import calculate_specificity, match_pattern

if TYPE_CHECKING:  # pragma: no cover - import cycle guard, types only
    from apcore.context import Context

__all__ = ["PolicyRule", "PolicyDecision", "ExecutionPolicy", "strip_approval_token"]

_POLICY_KEYS = {"rules", "gate_destructive", "strict"}
_RULE_KEYS = {"pattern", "requires_approval", "destructive", "reason"}


@dataclass(frozen=True)
class PolicyRule:
    """A single pattern-based governance override.

    Attributes:
        pattern: Module ID wildcard pattern (Algorithm A08 semantics, same as
            ACL rules). ``*`` matches any character sequence including dots.
        requires_approval: Override for the module's ``requires_approval``
            annotation. None leaves the module's own value in effect.
        destructive: Override for the module's ``destructive`` annotation.
            None leaves the module's own value in effect.
        reason: Human-readable rationale, carried into the audit trail.
    """

    pattern: str
    requires_approval: bool | None = None
    destructive: bool | None = None
    reason: str | None = None

    def __post_init__(self) -> None:
        """Validate field types eagerly — a malformed governance rule must fail loud."""
        if not isinstance(self.pattern, str) or not self.pattern:
            raise ValueError("PolicyRule.pattern must be a non-empty string")
        for name in ("requires_approval", "destructive"):
            value = getattr(self, name)
            if value is not None and not isinstance(value, bool):
                raise ValueError(f"PolicyRule.{name} must be a bool or None, got {type(value).__name__}")
        if self.reason is not None and not isinstance(self.reason, str):
            raise ValueError(f"PolicyRule.reason must be a string or None, got {type(self.reason).__name__}")


@dataclass(frozen=True)
class PolicyDecision:
    """The effective governance verdict for one module under a policy.

    Attributes:
        module_id: The module the decision applies to.
        requires_approval: Effective ``requires_approval`` after overrides.
        destructive: Effective ``destructive`` after overrides.
        needs_approval: Final approval-gate verdict — true when the effective
            ``requires_approval`` is true, or when the policy gates destructive
            modules and the effective ``destructive`` is true.
        rule: The matched :class:`PolicyRule`, or None when no rule matched.
        overridden: True when the matched rule changed a base annotation value.
    """

    module_id: str
    requires_approval: bool
    destructive: bool
    needs_approval: bool
    rule: PolicyRule | None = None
    overridden: bool = False


def strip_approval_token(arguments: dict[str, Any] | None) -> dict[str, Any] | None:
    """Return *arguments* without the framework-owned ``_approval_token`` key.

    PROTOCOL_SPEC §7.9.6 rule 5: the approval token **MUST** be stripped from
    ``arguments`` *before* policy resolution. §7.4 already requires it to be
    removed "before passing to subsequent steps", and that does not reach this
    case — policy resolution happens *inside* Step 5, ahead of any subsequent
    step, so an implementation can satisfy §7.4 literally and still hand the
    token to the policy. It is a protocol-level key, not caller input; leaving
    it in place puts a token into the audit trail and the
    ``apcore.policy.override`` payload.

    The caller's mapping is never mutated: the token is the one difference
    between a call and its ``_approval_token`` resume, and the pipeline's own
    copy is stripped separately by the gate.
    """
    if arguments is None or _APPROVAL_TOKEN_KEY not in arguments:
        return arguments
    return {k: v for k, v in arguments.items() if k != _APPROVAL_TOKEN_KEY}


def _read_annotation_bool(annotations: Any, name: str) -> bool:
    """Read a boolean annotation from a dataclass-like object or a dict."""
    if annotations is None:
        return False
    if isinstance(annotations, dict):
        return bool(annotations.get(name, False))
    return bool(getattr(annotations, name, False))


def _require_bool(value: Any, key: str) -> bool:
    """Return *value* as a bool, or raise when it is anything else.

    Governance switches must not be set by type coercion. ``bool(value)`` read
    ``gate_destructive: "false"`` as **True** — a non-empty string is truthy —
    and ``gate_destructive: []`` as False, while apcore-rust's serde-typed
    ``bool`` field and apcore-typescript's ``_requireBoolean`` reject either
    outright. The same governance document produced a different effective policy
    per runtime.

    That matters more than a normal type nit: ``gate_destructive`` is the switch
    that turns a ``destructive`` annotation into an approval gate
    (PROTOCOL_SPEC §7.9.2), so a silent coercion is a governance decision made by
    accident. §7.9.4 already makes ``from_dict`` fail loud on unknown keys; this
    extends the same discipline to the values.

    ``None`` (and an absent key) mean "unset" and take the documented default of
    ``False`` rather than raising — parity with apcore-typescript.

    Note ``bool`` is checked before ``int``: ``True`` is an ``int`` in Python, so
    an ``isinstance(value, int)`` test would accept ``1`` and ``0`` as flags.

    Raises:
        ValueError: When *value* is neither None nor a bool.
    """
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError(
            f"Policy key '{key}' must be a boolean, got {type(value).__name__} ({value!r}); "
            "a governance switch must not be set by type coercion"
        )
    return value


def _restrictiveness(rule: PolicyRule) -> tuple[bool, bool]:
    """Tie-break key: rules that force gating rank above rules that relax it."""
    return (rule.requires_approval is True, rule.destructive is True)


class ExecutionPolicy:
    """Declarative execution-time governance overrides (apcore#76 RFC pilot).

    Args:
        rules: Ordered pattern-based overrides. The most specific matching
            rule wins (Algorithm A10 specificity); on a tie the more
            restrictive rule wins, then the earlier-defined rule.
        gate_destructive: When true, modules whose effective ``destructive``
            annotation is true require approval even if ``requires_approval``
            is false — the opt-in resolution of the destructive↔approval
            gap described in apcore#76.
        strict: When true, the approval gate fails closed (denies) when a
            module needs approval but no ApprovalHandler is configured.
            When false (default), the gate keeps the PROTOCOL_SPEC §7.4
            skip behavior but logs a warning.

    Example::

        policy = ExecutionPolicy(
            [PolicyRule("orders.delete_*", requires_approval=True, reason="human sign-off")],
            gate_destructive=True,
            strict=True,
        )
        executor = Executor(registry, approval_handler=handler, policy=policy)
    """

    def __init__(
        self,
        rules: list[PolicyRule] | None = None,
        *,
        gate_destructive: bool = False,
        strict: bool = False,
    ) -> None:
        self._rules: tuple[PolicyRule, ...] = tuple(rules or ())
        for rule in self._rules:
            if not isinstance(rule, PolicyRule):
                raise ValueError(f"ExecutionPolicy rules must be PolicyRule instances, got {type(rule).__name__}")
        self.gate_destructive = bool(gate_destructive)
        self.strict = bool(strict)

    @property
    def rules(self) -> tuple[PolicyRule, ...]:
        """The policy's rules as an immutable tuple."""
        return self._rules

    def resolve(
        self,
        module_id: str,
        annotations: Any = None,
        *,
        arguments: dict[str, Any] | None = None,
        context: Context | None = None,
    ) -> PolicyDecision:
        """Compute the effective governance decision for a module.

        PROTOCOL_SPEC §7.9.6 requires policy resolution to *receive* the call
        site — the invocation arguments and the :class:`~apcore.context.Context`
        — alongside the module ID and annotations. Governance keyed on the
        module alone forces an operator to gate every call to a module in order
        to gate some of them, which weakens ``requires_approval`` from "this
        needs approval" to "this might".

        The built-in pattern rules of §7.9.1 deliberately **do not consult**
        ``arguments`` or ``context``: a rule set's verdict stays a function of
        the module ID and the annotations alone, so it remains statically
        auditable and reproducible from the policy document, and adding the call
        site cannot change the verdict any existing policy produces (rule 6).
        They are here so that an implementation can carry the call site into the
        audit trail and the ``apcore.policy.override`` event — and what it
        records there **MUST** be the call site's *shape*, which keys were
        present and their types, never argument values, since rule 4 places this
        before schema validation and an argument may hold a secret.

        Keyword-only optional parameters are one of the shapes rule 7 names as
        conforming; what it requires is the **capability**, not one API form.

        .. note:: Rule 3(b) was withdrawn in spec v1.25.0. It promised that "a
            host-supplied policy implementation can decide on arguments", which
            no SDK can satisfy: ``ExecutionPolicy`` is a concrete class here and
            in apcore-typescript and a concrete ``struct`` in apcore-rust, and
            ``set_policy`` takes that concrete type in all three, so there is no
            policy interface for a host to implement. Subclassing this class
            happens to work in Python, but it is not a specified extension point
            and nothing here builds toward one.

        Args:
            module_id: Canonical module ID being invoked.
            annotations: The module's annotations (ModuleAnnotations, dict,
                or None). Only ``requires_approval`` and ``destructive``
                are consulted.
            arguments: The invocation's arguments, when available. These have
                **NOT** been schema-validated — the approval gate is pipeline
                Step 5 and input validation is Step 7 (PROTOCOL_SPEC §12.8) —
                so an override MUST NOT assume them well-formed, present, or of
                the declared type. Keyword-only, and optional, so every existing
                caller keeps working unchanged.
            context: The execution context of the call, when available. Same
                keyword-only, optional treatment.

        Returns:
            A :class:`PolicyDecision` carrying the effective values, the
            final ``needs_approval`` verdict, and audit metadata.
        """
        # §7.9.6 rule 5 — the framework-owned approval token never reaches a
        # governance decision. Stripping here rather than at each call site
        # covers every door into resolution: `Executor.validate` (§7.9.5
        # preflight) and the approval gate both arrive through this method,
        # and a future third caller is covered without being remembered.
        arguments = strip_approval_token(arguments)

        # Read, deliberately unused by the built-in rules (§7.9.6 rule 2).
        # A host-supplied policy overriding resolve() receives them and may.
        _ = (arguments, context)

        base_requires_approval = _read_annotation_bool(annotations, "requires_approval")
        base_destructive = _read_annotation_bool(annotations, "destructive")

        rule = self._match(module_id)
        effective_requires_approval = (
            base_requires_approval if rule is None or rule.requires_approval is None else rule.requires_approval
        )
        effective_destructive = base_destructive if rule is None or rule.destructive is None else rule.destructive

        return PolicyDecision(
            module_id=module_id,
            requires_approval=effective_requires_approval,
            destructive=effective_destructive,
            needs_approval=effective_requires_approval or (self.gate_destructive and effective_destructive),
            rule=rule,
            overridden=(
                effective_requires_approval != base_requires_approval or effective_destructive != base_destructive
            ),
        )

    def _match(self, module_id: str) -> PolicyRule | None:
        """Return the winning rule for a module ID, or None."""
        best: PolicyRule | None = None
        best_score = -1
        for rule in self._rules:
            if not match_pattern(rule.pattern, module_id):
                continue
            score = calculate_specificity(rule.pattern)
            if score > best_score:
                best, best_score = rule, score
            elif score == best_score and best is not None and _restrictiveness(rule) > _restrictiveness(best):
                best = rule
        return best

    @classmethod
    def from_dict(cls, data: Any) -> ExecutionPolicy:
        """Build a policy from a plain mapping (YAML/JSON governance file).

        Parsing is strict in both directions: unknown keys raise ``ValueError``
        so a typo in a governance file fails loud instead of silently disabling
        a control, and ``gate_destructive`` / ``strict`` must be real booleans
        rather than anything ``bool()`` happens to accept. ``gate_destructive:
        "false"`` used to enable the gate (a non-empty string is truthy) while
        apcore-rust and apcore-typescript both rejected the document — see
        :func:`_require_bool`.

        Expected shape::

            gate_destructive: true
            strict: true
            rules:
              - pattern: "orders.delete_*"
                requires_approval: true
                reason: "destructive order operations need human sign-off"

        Args:
            data: Parsed mapping, e.g. ``yaml.safe_load`` output.

        Returns:
            A validated :class:`ExecutionPolicy`.

        Raises:
            ValueError: On unknown keys, missing ``pattern``, or wrong types.
        """
        if not isinstance(data, dict):
            raise ValueError(f"Policy document must be a mapping, got {type(data).__name__}")
        unknown = set(data) - _POLICY_KEYS
        if unknown:
            raise ValueError(f"Unknown policy keys {sorted(unknown)}; a governance file must not silently no-op")

        rules_raw = data.get("rules", [])
        if not isinstance(rules_raw, list):
            raise ValueError(f"Policy 'rules' must be a list, got {type(rules_raw).__name__}")

        rules: list[PolicyRule] = []
        for index, item in enumerate(rules_raw):
            if not isinstance(item, dict):
                raise ValueError(f"Policy rule #{index} must be a mapping, got {type(item).__name__}")
            unknown = set(item) - _RULE_KEYS
            if unknown:
                raise ValueError(
                    f"Policy rule #{index} has unknown keys {sorted(unknown)}; "
                    "a governance rule must not silently no-op"
                )
            if "pattern" not in item:
                raise ValueError(f"Policy rule #{index} is missing required key 'pattern'")
            rules.append(PolicyRule(**item))

        return cls(
            rules,
            gate_destructive=_require_bool(data.get("gate_destructive"), "gate_destructive"),
            strict=_require_bool(data.get("strict"), "strict"),
        )
