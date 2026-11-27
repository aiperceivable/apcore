"""Built-in pipeline steps for the v0.17 Pipeline v2 execution model.

Each class wraps one step of the executor pipeline, receiving its
dependencies via constructor injection.  Steps read from and write to
PipelineContext fields, returning StepResult to control pipeline flow.

Pipeline v2 steps raise domain errors directly instead of returning
abort results, enabling the executor to propagate typed exceptions.
"""

from __future__ import annotations

import asyncio
import dataclasses
import inspect
import json
import logging
import time
from collections.abc import Awaitable, Callable
from copy import deepcopy
from typing import TYPE_CHECKING, Any, cast

import pydantic

from apcore.approval import ApprovalRequest, ApprovalResult, ScopedApprovalHandler
from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.context import Context, GovernanceProjection
from apcore.context_keys import _current_trace_span
from apcore.decorator import FunctionModule
from apcore.errors import (
    ACLDeniedError,
    ApprovalDeniedError,
    ApprovalPendingError,
    ApprovalTimeoutError,
    InvalidInputError,
    ModuleDisabledError,
    ModuleExecuteError,
    ModuleNotFoundError,
    ModuleTimeoutError,
    SchemaValidationError,
    StreamingInterfaceError,
)
from apcore.module import ModuleAnnotations
from apcore.observability.context_logger import RedactionConfig
from apcore.pipeline import (
    BaseStep,
    ConfigurationError,
    ExecutionStrategy,
    PipelineContext,
    StepResult,
)
from apcore.policy import ExecutionPolicy, PolicyDecision, strip_approval_token
from apcore.schema.annotations import governance_union
from apcore.schema.hardening import warn_format_violations
from apcore.schema.types import _validation_failure_message
from apcore.schema.validator import _validate_strict_model
from apcore.config import Config
from apcore.utils.call_chain import guard_call_chain
from apcore.utils.module_output import normalize_module_output

if TYPE_CHECKING:
    from apcore.sys_modules.control import ToggleState

__all__ = [
    "BuiltinContextCreation",
    "BuiltinCallChainGuard",
    "BuiltinModuleLookup",
    "BuiltinACLCheck",
    "BuiltinApprovalGate",
    "BuiltinInputValidation",
    "BuiltinMiddlewareBefore",
    "BuiltinExecute",
    "BuiltinOutputValidation",
    "BuiltinMiddlewareAfter",
    "BuiltinReturnResult",
    "build_standard_strategy",
    "build_internal_strategy",
    "build_testing_strategy",
    "build_performance_strategy",
    "build_minimal_strategy",
]

_logger = logging.getLogger(__name__)


def _validate_module_payload(schema: type[pydantic.BaseModel], data: dict[str, Any], direction: str) -> Any:
    """Apply strict validation to a JSON value or an existing native value."""
    try:
        validate_json = getattr(schema, "model_validate_json", None)
        if callable(validate_json):
            try:
                encoded = json.dumps(data)
            except (TypeError, ValueError):
                # Native values without a JSON representation use strict native validation.
                pass
            else:
                return _validate_strict_model(schema, encoded, json_mode=True)
        return _validate_strict_model(schema, data, json_mode=False)
    except pydantic.ValidationError as error:
        errors = _convert_validation_errors(error)
    except SchemaValidationError as error:
        errors = error.details["errors"]
    # Native exception strings and tracebacks can render private instance values.
    raise SchemaValidationError(message=f"{direction} validation failed", errors=errors)


def _convert_validation_errors(error: pydantic.ValidationError) -> list[dict[str, Any]]:
    """Convert native validator failures to canonical JSON Schema details."""
    keywords = {
        "missing": "required",
        "literal_error": "enum",
        "enum": "enum",
        "greater_than_equal": "minimum",
        "greater_than": "exclusiveMinimum",
        "less_than_equal": "maximum",
        "less_than": "exclusiveMaximum",
        "string_too_short": "minLength",
        "string_too_long": "maxLength",
        "too_short": "minItems",
        "too_long": "maxItems",
        "string_pattern_mismatch": "pattern",
        "multiple_of": "multipleOf",
        "extra_forbidden": "additionalProperties",
    }
    results = []
    for item in error.errors(include_input=False, include_context=False):
        kind = item["type"]
        location = item["loc"][:-1] if kind in {"missing", "extra_forbidden"} else item["loc"]
        path = "".join("/" + str(part).replace("~", "~0").replace("/", "~1") for part in location)
        keyword = keywords.get(
            kind, "type" if kind.endswith(("_type", "_parsing")) or kind == "is_instance_of" else "format"
        )
        results.append({"path": path, "keyword": keyword, "message": _validation_failure_message(keyword, kind)})
    return results


def _ensure_middleware_manager(manager: Any | None, middlewares: list[Any] | None) -> Any:
    """Return *manager* if provided, otherwise build a fresh one from *middlewares*.

    Lets ``BuiltinMiddlewareBefore`` and ``BuiltinMiddlewareAfter`` keep a
    single execution path: production passes the executor's shared
    ``MiddlewareManager`` directly, while tests/standalone callers can
    supply a raw ``middlewares`` list and have a one-off manager built for
    them on the fly.
    """
    if manager is not None:
        return manager
    from apcore.middleware.manager import MiddlewareManager

    built = MiddlewareManager()
    for mw in middlewares or []:
        built.add(mw)
    return built


# ---------------------------------------------------------------------------
# Governance gate fields (PROTOCOL_SPEC §5.16.1, D-130)
# ---------------------------------------------------------------------------

_GATE_FIELD_REASONS: dict[str, str] = {
    "ignore_errors": "ignore_errors turns a denial into a warning and lets the call run",
    "match_modules": "match_modules exempts every module it does not match",
    "pure": "pure makes validate() run the gate and consult the ApprovalHandler during a dry run",
}


def gate_field_violations(step: Any, overrides: dict[str, Any]) -> list[str]:
    """Return the keys of *overrides* that would weaken *step* if it is a built-in gate.

    PROTOCOL_SPEC §5.16.1 (D-130): on the built-in ``acl_check`` and
    ``approval_gate`` steps, ``ignore_errors: true``, any ``match_modules`` and
    ``pure: true`` are rejected. Writing a field's default value is accepted, so
    ``pure: true`` on ``acl_check`` — whose built-in default is already
    ``pure=True`` (it is side-effect free and ``validate()`` runs it) — is not a
    violation. ``timeout_ms`` is always accepted. A step that is not a built-in
    gate (recognised by TYPE, §6.6.5.2) has no violations.
    """
    if not isinstance(step, (BuiltinACLCheck, BuiltinApprovalGate)):
        return []
    offending: list[str] = []
    for key, value in overrides.items():
        if key == "ignore_errors" and bool(value):
            offending.append(key)
        elif key == "match_modules" and value is not None:
            offending.append(key)
        elif key == "pure" and bool(value) and isinstance(step, BuiltinApprovalGate):
            offending.append(key)
    return offending


def gate_field_violation_message(step_name: str, keys: list[str]) -> str:
    """Build the ``PIPELINE_CONFIGURATION_ERROR`` message naming the gate step and every key."""
    noun = "field" if len(keys) == 1 else "fields"
    parts = "; ".join(f"'{k}' ({_GATE_FIELD_REASONS[k]})" for k in keys)
    return (
        f"Cannot configure step '{step_name}': {len(keys)} {noun} would weaken a governance gate: "
        f"{parts}. On the built-in gate steps acl_check and approval_gate only timeout_ms is "
        f"configurable; remove the step instead if it is not wanted (PROTOCOL_SPEC §5.16.1, D-130)."
    )


class _GovernanceGateFields(BaseStep):
    """Refuse weakening field values on a built-in governance gate at every door.

    ``pipeline.configure`` is one way to set ``ignore_errors`` / ``match_modules`` /
    ``pure`` on a step; plain attribute assignment is the programmatic one. Both
    land on these property setters, so the D-130 rule cannot be bypassed by
    configuring the step object directly instead of through YAML.
    """

    _fail_closed_on_error = True

    def _check_gate_field(self, key: str, value: Any) -> None:
        if gate_field_violations(self, {key: value}):
            raise ConfigurationError(gate_field_violation_message(self.name, [key]))

    @property
    def ignore_errors(self) -> bool:
        return bool(self.__dict__.get("_gate_ignore_errors", False))

    @ignore_errors.setter
    def ignore_errors(self, value: bool) -> None:
        self._check_gate_field("ignore_errors", value)
        self._gate_ignore_errors = value

    @property
    def match_modules(self) -> tuple[str, ...] | None:
        return self.__dict__.get("_gate_match_modules")

    @match_modules.setter
    def match_modules(self, value: tuple[str, ...] | None) -> None:
        self._check_gate_field("match_modules", value)
        self._gate_match_modules = value

    @property
    def pure(self) -> bool:
        return bool(self.__dict__.get("_gate_pure", False))

    @pure.setter
    def pure(self, value: bool) -> None:
        self._check_gate_field("pure", value)
        self._gate_pure = value


# ---------------------------------------------------------------------------
# Step 1: Context Creation
# ---------------------------------------------------------------------------


class BuiltinContextCreation(BaseStep):
    """Create or inherit execution context and set global deadline."""

    def __init__(
        self,
        *,
        config: Any | None = None,
        executor: Any | None = None,
    ) -> None:
        super().__init__(
            name="context_creation",
            description="Create execution context and set global deadline",
            removable=False,
            replaceable=False,
            pure=True,
            provides=("context",),
        )
        self._config = config
        self._executor = executor
        if config is not None:
            val = config.get("executor.global_timeout")
            self._global_timeout: int = val if val is not None else Config.get_default("executor.global_timeout")
        else:
            self._global_timeout = Config.get_default("executor.global_timeout")

    @property
    def global_timeout_ms(self) -> int:
        """Return the budget used when the caller supplies no deadline."""
        return self._global_timeout

    async def execute(self, ctx: PipelineContext) -> StepResult:
        if ctx.context is None:
            # Fallback path: a Context was not bound at the Executor entry
            # point (e.g. PipelineContext constructed directly in tests).
            # Per PROTOCOL_SPEC §"Contract: Context.create", executor is not
            # a public constructor input; bind via Context.bind_executor.
            base_ctx = Context.create()
            base_ctx.bind_executor(self._executor)
        else:
            base_ctx = ctx.context

        # Derive child context to add module_id to call chain. `child()`
        # carries the parent's `global_deadline` forward, so a nested call
        # inherits the budget of the tree it belongs to.
        ctx.context = base_ctx.child(ctx.module_id)
        if isinstance(ctx.context, Context):
            ctx.context._execution_scope = ctx.execution_scope
        # Every invocation owns its cancellation signal. Parent cancellation
        # propagates downward, while a timeout leaves ancestors reusable.
        parent_token = getattr(base_ctx, "cancel_token", None)
        ctx.context.cancel_token = parent_token.child() if parent_token is not None else CancelToken()

        # D-101 — the deadline belongs to the CALL TREE, not to the Context.
        # It is computed onto the context derived for THIS call, never onto
        # the caller-supplied Context, which `bind_executor` mutates in place
        # and which the caller is explicitly allowed to reuse across
        # successive top-level `Executor.call()` invocations. Written onto
        # that object, the second call inherits the first call's remaining
        # time — or fails immediately, having already expired.
        #
        # D-102 — recomputation is UNCONDITIONAL. The old guard also required
        # an empty `call_chain`, but a Context arriving from another process
        # carries a non-empty chain BY DEFINITION, so that conjunct inverted
        # core-executor.md's "The receiving Executor MUST recompute" exactly
        # where it applies and every cross-process sub-tree ran with no
        # global budget. The `global_deadline is None` clause alone is the
        # correct guard: it already preserves an in-process caller's explicit
        # value (and, via `child()`, the enclosing tree's).
        #
        # D-99 — the clock is EPOCH SECONDS (`time.time()`), as
        # docs/features/context-object.md has stated since the field was
        # introduced. `global_deadline` is a public `Context.create`
        # parameter, so a spec-following caller writes `time.time() + budget`;
        # compared against a monotonic basis that is roughly 1.8e9 against
        # roughly 1e5, so the deadline never fires and the call runs with no
        # budget at all — silent, and fail-open.
        if ctx.context.global_deadline is None:
            ctx.context.global_deadline = ctx.invocation_deadline
            if ctx.context.global_deadline is None and self._global_timeout > 0:
                ctx.context.global_deadline = time.time() + self._global_timeout / 1000.0
        ctx.invocation_deadline = ctx.context.global_deadline
        if ctx._stream_observation is not None:
            ctx._stream_observation.monitor(ctx.context)

        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Step 2: Call Chain Guard
# ---------------------------------------------------------------------------


class BuiltinCallChainGuard(BaseStep):
    """Call chain guard: depth, repeat limits, cancel token."""

    def __init__(self, *, config: Any | None = None) -> None:
        super().__init__(
            name="call_chain_guard",
            description="Validate call chain depth and repeat limits",
            removable=True,
            replaceable=True,
            pure=True,
            requires=("context",),
        )
        self._config = config
        if config is not None:
            val = config.get("executor.max_call_depth")
            self._max_call_depth: int = val if val is not None else Config.get_default("executor.max_call_depth")
            val = config.get("executor.max_module_repeat")
            self._max_module_repeat: int = val if val is not None else Config.get_default("executor.max_module_repeat")
        else:
            self._max_call_depth = Config.get_default("executor.max_call_depth")
            self._max_module_repeat = Config.get_default("executor.max_module_repeat")

    async def execute(self, ctx: PipelineContext) -> StepResult:
        # D-21 / A-D-EXEC-002: short-circuit before any expensive validation
        # or middleware work if the caller already cancelled. The pipeline
        # also re-checks at the Execute step (defensive backstop for tokens
        # cancelled while later steps run), but observing the token here is
        # what makes the two-point invariant hold — single-check
        # implementations leak compute through ACL/middleware/validation.
        cancel_token = getattr(ctx.context, "cancel_token", None)
        if cancel_token is not None and getattr(cancel_token, "is_cancelled", False):
            raise ExecutionCancelledError()

        call_chain = getattr(ctx.context, "call_chain", [])
        guard_call_chain(
            ctx.module_id,
            call_chain,
            max_call_depth=self._max_call_depth,
            max_module_repeat=self._max_module_repeat,
        )
        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Step 3: Module Lookup
# ---------------------------------------------------------------------------


def _resolve_redaction(config: Any | None) -> RedactionConfig:
    """The `obs.redaction.*` rules the capture point applies.

    PROTOCOL_SPEC §10.6.1 "Where the rules apply": the union of `x-sensitive`,
    `sensitive_keys` and `regex_patterns` MUST hold at log emission **and** at
    the executor's input/output capture point, with the SAME rules at each.
    Until this existed the capture point applied `x-sensitive` and the default
    key list only, so a configured `regex_patterns` entry redacted a bearer
    token in the log line an operator was watching and stored it in the audit
    record they were not (aiperceivable/apcore#120).

    Requirement 3: with no configuration the DEFAULTS apply, which is what
    `redact_sensitive(data, schema)` already did — so a caller who configures
    nothing sees no change.

    Resolved once per strategy, not per execution: §10.6.1 requirement 5 puts
    compilation at the configuration read, and `RedactionConfig` compiles
    `regex_patterns` in its constructor.
    """
    if config is None:
        return RedactionConfig.default()
    return RedactionConfig.from_config(config)


def _redact_payload(data: dict[str, Any] | None, schema_model: Any, rules: RedactionConfig | None) -> dict[str, Any]:
    from apcore.utils.redaction import redact_sensitive

    schema: Any = getattr(schema_model, "__apcore_source_schema__", None)
    if not isinstance(schema, dict):
        schema_json = getattr(schema_model, "model_json_schema", None)
        schema = schema_json() if callable(schema_json) else {}
    rules = rules or _resolve_redaction(None)
    return redact_sensitive(
        data or {},
        schema,
        sensitive_keys=rules.sensitive_keys,
        regex_patterns=rules.compiled_regex_patterns,
        replacement=rules.replacement,
    )


class BuiltinModuleLookup(BaseStep):
    """Resolve module from registry by ID with optional version hint.

    After a successful registry lookup, checks the toggle state and raises
    ``ModuleDisabledError`` if the module has been disabled via
    ``ToggleFeatureModule`` / ``ToggleState.disable()``.  This mirrors the
    MODULE_DISABLED (HTTP 403) behaviour in the TypeScript and Rust SDKs.
    """

    def __init__(
        self,
        *,
        registry: Any,
        toggle_state: Any | None = None,
        redaction: Any | None = None,
    ) -> None:
        super().__init__(
            name="module_lookup",
            description="Look up module in registry",
            removable=False,
            replaceable=False,
            pure=True,
            provides=("module",),
        )
        self._registry = registry
        self._redaction = redaction
        # Use the injected toggle_state when provided (e.g. for testing);
        # fall back to the package-level default singleton.
        if toggle_state is not None:
            self._toggle_state = toggle_state
        else:
            from apcore.sys_modules.control import _default_toggle_state

            self._toggle_state = _default_toggle_state

    @property
    def toggle_state(self) -> ToggleState:
        """State this step consults before executing a module."""
        return self._toggle_state

    def _set_toggle_state(self, toggle_state: ToggleState) -> None:
        self._toggle_state = toggle_state

    async def execute(self, ctx: PipelineContext) -> StepResult:
        version_hint = getattr(ctx, "version_hint", None)
        resolve = getattr(type(self._registry), "_resolve_registration", None)
        if resolve is not None:
            if ctx.execution_scope is not None:
                token = (
                    ctx._stream_observation.cancel_token
                    if ctx._stream_observation is not None
                    else getattr(ctx.context, "cancel_token", None)
                )
                lease = self._registry._acquire_registration(ctx.module_id, version_hint, token)
                registration = lease.__enter__()

                def release_lease() -> None:
                    task = ctx.module_task
                    if task is None or task.done():
                        lease.__exit__(None, None, None)
                    else:
                        # A cooperative timeout can return while work still
                        # runs. Retain its lease until the work itself stops.
                        task.add_done_callback(lambda _: lease.__exit__(None, None, None))

                ctx.execution_scope.callback(release_lease)
            else:
                registration = resolve(self._registry, ctx.module_id, version_hint)
            module = registration.module if registration is not None else None
            declared_annotations = registration.metadata.get("annotations") if registration is not None else None
        else:
            if version_hint is not None:
                module = self._registry.get(ctx.module_id, version_hint=version_hint)
            else:
                module = self._registry.get(ctx.module_id)
            get_declared = getattr(self._registry, "get_declared_annotations", None)
            declared_annotations = (
                get_declared(ctx.module_id) if module is not None and callable(get_declared) else None
            )
        if module is None:
            raise ModuleNotFoundError(module_id=ctx.module_id)

        # Spec §"Toggle" — disabled modules must not be executed.
        # Check AFTER registry lookup so we still return ModuleNotFoundError
        # for unregistered IDs (not ModuleDisabledError).
        if self._toggle_state.is_disabled(ctx.module_id):
            raise ModuleDisabledError(module_id=ctx.module_id)

        ctx.module = module
        # PROTOCOL_SPEC §7.4 (D-96): resolve the registry's DECLARED annotations
        # here and hand them to the gate, which has no registry of its own —
        # the same shape as `acl_approval_required`, which Step 4 computes and
        # Step 5 reads. Without this the gate could only ever see the live
        # instance, and a `*.binding.yaml` / `metadata=` source declaring
        # `requires_approval: true` reached `get_definition()` and the manifest
        # while the module executed UNGATED.
        ctx.declared_annotations = declared_annotations

        # Early input redaction: set context.redacted_inputs BEFORE any
        # middleware runs (step 6: BuiltinMiddlewareBefore). This ensures
        # logging middleware's before() hook sees redacted inputs instead
        # of None. Input validation (step 7) still validates; this step
        # only redacts for observability.
        if ctx.context is not None and hasattr(ctx.context, "redacted_inputs"):
            ctx.context.redacted_inputs = _redact_payload(
                strip_approval_token(ctx.inputs), getattr(module, "input_schema", None), self._redaction
            )

        # PROTOCOL_SPEC §6.1.8: the governance projection is computed HERE, at
        # Step 3, and made available to the ACL check at Step 4. The ordering is
        # normative rather than an implementation detail that happens to hold —
        # the `arguments` condition (§6.1.7) has nothing to read otherwise.
        #
        # It is deliberately NOT `redacted_inputs`, three lines above. That
        # field's contract is safe *logging*, and the branch just taken shows
        # exactly why §6.1.8 rule 3 forbids substituting it: with no input
        # schema it is a raw copy of the arguments, values and all. The
        # projection carries the key set and each key's JSON type and has no
        # field a value could live in.
        # CTX-2: onto the per-call PipelineContext, not onto `ctx.context`.
        # The execution Context is handed to module code and is explicitly
        # reusable across successive top-level calls, so a projection written
        # there survives the check it was computed for: a later direct
        # `acl.check_access(..., ctx)` then evaluated an `arguments` condition
        # against the PREVIOUS call's argument key set. Step 4 passes this
        # value to the ACL explicitly, which is the other delivery shape
        # §6.1.8 rule 4 blesses and the one apcore-typescript and apcore-rust
        # use.
        ctx.governance_projection = GovernanceProjection.of(ctx.inputs)

        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Step 4: ACL Check
# ---------------------------------------------------------------------------


class BuiltinACLCheck(_GovernanceGateFields):
    """Access control list enforcement."""

    def __init__(self, *, acl: Any | None = None, event_emitter: Any | None = None) -> None:
        super().__init__(
            name="acl_check",
            description="Enforce access control policies",
            removable=True,
            replaceable=True,
            pure=True,
            requires=("context", "module"),
        )
        self._acl = acl
        self._event_emitter = event_emitter

    def set_acl(self, acl: Any | None) -> None:
        """Replace the ACL provider used by this step at runtime."""
        self._acl = acl

    def set_event_emitter(self, event_emitter: Any | None) -> None:
        """Set the event emitter for ACL denials."""
        self._event_emitter = event_emitter

    @property
    def acl(self) -> Any | None:
        """The ACL provider this gate enforces (read by ``governance_state()``, §6.6.5.5)."""
        return self._acl

    async def execute(self, ctx: PipelineContext) -> StepResult:
        if self._acl is None:
            return StepResult(action="continue")

        caller_id = getattr(ctx.context, "caller_id", "anonymous")

        # PROTOCOL_SPEC §6.8.1: Step 4 produces TWO results, so this step reads
        # the structured accessor and not the boolean. The boolean deliberately
        # fails closed on an approval requirement — right for a tooling caller,
        # wrong here, because it would turn "ask a human" into a flat denial and
        # the approval gate at Step 5 would never see the requirement at all.
        #
        # The fallbacks keep a custom ACL object working: one that predates the
        # structured accessor simply contributes no approval requirement.
        allowed, approval_required = await self._decide(caller_id, ctx)
        ctx.acl_approval_required = approval_required

        if not allowed:
            # Publish a governance event on denial (canonical name proposed in
            # apcore#77). Guarded by dry_run so a validate() preflight probe
            # never emits a spurious denial event. Fires only on deny — allows
            # are high-volume and already covered by the apcore.acl.check span.
            self._emit_denied(caller_id, ctx.module_id, ctx.context, ctx.dry_run)
            raise ACLDeniedError(caller_id=caller_id, target_id=ctx.module_id)
        ctx._acl_allowed = True
        return StepResult(action="continue")

    async def _decide(self, caller_id: str, ctx: PipelineContext) -> tuple[bool, bool]:
        """Resolve the ACL to ``(allowed, approval_required)``.

        Prefers the structured accessors of PROTOCOL_SPEC §6.8.1, async first,
        and falls back to the legacy booleans for an ACL object that has none.
        On the legacy path ``approval_required`` is False: the boolean cannot
        carry it, and inferring one would be inventing governance.
        """
        # CTX-2: the projection is passed per check rather than read off the
        # Context. `_call_access` degrades to the old positional call for a
        # custom ACL object whose accessor does not accept the keyword, so a
        # host implementation written against the previous surface keeps
        # working.
        acl = self._acl
        if acl is None:
            return True, False
        access = getattr(acl, "async_check_access", None)
        if callable(access):
            decision = await self._call_access(access, caller_id, ctx, awaitable=True)
            return decision.access == "allow", bool(decision.approval_required)

        access = getattr(acl, "check_access", None)
        if callable(access):
            decision = self._call_access(access, caller_id, ctx, awaitable=False)
            return decision.access == "allow", bool(decision.approval_required)

        if hasattr(acl, "async_check"):
            return bool(await acl.async_check(caller_id, ctx.module_id, ctx.context)), False
        return bool(acl.check(caller_id, ctx.module_id, ctx.context)), False

    @staticmethod
    def _call_access(access: Any, caller_id: str, ctx: PipelineContext, *, awaitable: bool) -> Any:
        """Invoke an ACL accessor, passing the call's governance projection.

        A custom ACL predating the keyword still works: the TypeError is
        narrowed to the keyword itself, so a genuine error inside the accessor
        is never swallowed and retried.
        """
        try:
            return access(caller_id, ctx.module_id, ctx.context, projection=ctx.governance_projection)
        except TypeError as exc:
            if "projection" not in str(exc):
                raise
            return access(caller_id, ctx.module_id, ctx.context)

    def _emit_denied(self, caller_id: str, module_id: str, ctx: Context, dry_run: bool) -> None:
        """Emit ``apcore.acl.denied`` on the event bus (apcore#77) when live."""
        if self._event_emitter is None or dry_run:
            return
        from datetime import datetime, timezone

        from apcore.events.emitter import ApCoreEvent

        self._event_emitter.emit(
            ApCoreEvent(
                event_type="apcore.acl.denied",
                module_id=module_id,
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="warn",
                data={
                    "module_id": module_id,
                    "caller_id": caller_id,
                    "reason": "ACL denied",
                    "trace_id": getattr(ctx, "trace_id", None),
                },
            )
        )


# ---------------------------------------------------------------------------
# Step 5: Approval Gate
# ---------------------------------------------------------------------------


class BuiltinApprovalGate(_GovernanceGateFields):
    """Approval handler flow for modules requiring approval.

    For modules whose annotations declare ``requires_approval=True`` (or that
    an :class:`~apcore.policy.ExecutionPolicy` gates), calls the configured
    ApprovalHandler, emits an audit event (logging + tracing span event), and
    translates the result into either continued execution or the appropriate
    ``ApprovalError`` subclass.

    Fail-loud governance (apcore#76): when a module needs approval but no
    handler is configured, the gate keeps the PROTOCOL_SPEC §7.4 skip
    behavior but logs a warning (once per module). With
    ``ExecutionPolicy(strict=True)`` it fails closed instead. A module whose
    effective ``destructive`` annotation is true but that no approval gate
    covers is also warned about once per module.
    """

    def __init__(
        self,
        *,
        handler: Any | None = None,
        policy: ExecutionPolicy | None = None,
        event_emitter: Any | None = None,
    ) -> None:
        super().__init__(
            name="approval_gate",
            description="Request and verify module approval",
            removable=True,
            replaceable=True,
            pure=False,
            requires=("context", "module"),
        )
        self._handler = handler
        self._policy = policy
        self._event_emitter = event_emitter
        self._warned: set[str] = set()

    def set_handler(self, handler: Any | None) -> None:
        """Replace the approval handler used by this step at runtime."""
        self._handler = handler

    def set_policy(self, policy: ExecutionPolicy | None) -> None:
        """Replace the execution policy used by this step at runtime."""
        self._policy = policy

    def set_event_emitter(self, event_emitter: Any | None) -> None:
        """Set the event emitter for approval decisions and policy overrides."""
        self._event_emitter = event_emitter

    @property
    def handler(self) -> Any | None:
        """The ApprovalHandler this gate consults (read by ``governance_state()``, §6.6.5.5)."""
        return self._handler

    @property
    def policy(self) -> ExecutionPolicy | None:
        """The ExecutionPolicy this gate applies (read by ``governance_state()``, §6.6.5.5)."""
        return self._policy

    async def execute(self, ctx: PipelineContext) -> StepResult:
        module = ctx.module
        approval_token = ctx._take_approval_token()

        # PROTOCOL_SPEC §6.1.6 / §7.4: Step 4's second result. The gate fires on
        # the UNION of the module annotation, this, and gate_destructive (§6.9
        # rows 3-5) — an implementation that reads only the annotation silently
        # ignores every ACL rule carrying `approval`: the rule loads, matches,
        # and does nothing.
        acl_requires_approval = bool(getattr(ctx, "acl_approval_required", False))

        # PROTOCOL_SPEC §7.4 (D-96): governance is the UNION of the live module
        # instance and the registry's declared annotations. Reading the instance
        # alone ignored what an operator declared in a `*.binding.yaml` /
        # `metadata=` source — a `requires_approval: true` that reached
        # `get_definition()` and the manifest and gated nothing. Reading the
        # declared slot alone would be worse: it carries YAML > code precedence,
        # so a metadata `false` would cancel a module that asks to be gated.
        # Both single-source readings are fail-OPEN.
        governance = governance_union(
            getattr(module, "annotations", None),
            getattr(ctx, "declared_annotations", None),
        )

        decision: PolicyDecision | None = None
        if self._policy is not None:
            # PROTOCOL_SPEC §7.9.6: policy resolution receives the call site.
            # The arguments have NOT been schema-validated at this point — this
            # gate is Step 5 and input validation is Step 7 (§12.8) — and the
            # built-in pattern rules do not consult them, so the verdict of an
            # existing policy is unchanged. A host-supplied policy can decide
            # on them, and an implementation can carry them into the audit
            # trail. The pipeline separates the resume credential at entry.
            decision = self._policy.resolve(
                ctx.module_id,
                governance,
                arguments=ctx.inputs,
                context=ctx.context,
            )
            # §6.9 row 4: a policy may ADD an approval requirement and MUST NOT
            # remove one the ACL set. The ACL is a caller-scoped authorization
            # layer; ExecutionPolicy is a module-scoped platform override, and
            # letting the module-scoped one cancel the caller-scoped one is a
            # privilege escalation — a policy rule written for `orders.*` would
            # silently strip a requirement an ACL author attached to one
            # untrusted caller. Union is the only safe composition, so the OR
            # here is load-bearing and not a convenience.
            needs_approval = decision.needs_approval or acl_requires_approval
            effective_destructive = decision.destructive
            if decision.overridden:
                self._emit_policy_audit(decision, ctx.context)
        else:
            needs_approval = _requires_approval(governance) or acl_requires_approval
            effective_destructive = _is_destructive(governance)

        if not needs_approval:
            if effective_destructive:
                self._warn_once(
                    f"destructive_ungated:{ctx.module_id}",
                    "Module '%s' is annotated destructive=True but is not covered by the "
                    "approval gate (requires_approval is false and no policy gates destructive "
                    "modules). Consider requires_approval=True or ExecutionPolicy("
                    "gate_destructive=True). (apcore#76)",
                    ctx.module_id,
                )
            return StepResult(action="continue")

        if self._handler is None:
            if self._policy is not None and self._policy.strict:
                result = ApprovalResult(
                    status="rejected",
                    reason=(
                        "Approval required but no ApprovalHandler is configured; "
                        "ExecutionPolicy(strict=True) fails closed"
                    ),
                )
                self._emit_audit(result, ctx.module_id, ctx.context)
                raise ApprovalDeniedError(result=result, module_id=ctx.module_id)
            self._warn_once(
                f"no_handler:{ctx.module_id}",
                "Module '%s' requires approval but no ApprovalHandler is configured; "
                "the approval gate is skipped per PROTOCOL_SPEC §7.4. Configure an "
                "approval handler, or ExecutionPolicy(strict=True) to fail closed. (apcore#76)",
                ctx.module_id,
            )
            return StepResult(action="continue")

        annotations = dataclasses.replace(
            _coerce_annotations(governance), requires_approval=True, destructive=effective_destructive
        )
        request = ApprovalRequest(
            module_id=ctx.module_id,
            arguments=deepcopy(ctx.inputs),
            context=ctx.context,
            annotations=annotations,
            description=getattr(module, "description", None),
            tags=list(getattr(module, "tags", None) or []),
            caller_id=ctx.context.caller_id,
            action=ctx.module_id,
        )
        if approval_token is None:
            result = await self._handler.request_approval(request)
        elif isinstance(self._handler, ScopedApprovalHandler):
            result = await self._handler.check_approval_for_call(approval_token, request)
        else:
            result = ApprovalResult(status="rejected", reason="The approval handler does not support scoped resumption")
        self._emit_audit(result, ctx.module_id, ctx.context, approval_token)

        if result.status == "approved":
            return StepResult(action="continue")
        if result.status == "rejected":
            raise ApprovalDeniedError(result=result, module_id=ctx.module_id)
        if result.status == "timeout":
            raise ApprovalTimeoutError(result=result, module_id=ctx.module_id)
        if result.status == "pending":
            raise ApprovalPendingError(result=result, module_id=ctx.module_id)
        _logger.warning(
            "Unknown approval status %r for module %s, treating as denied",
            result.status,
            ctx.module_id,
        )
        raise ApprovalDeniedError(result=result, module_id=ctx.module_id)

    def _warn_once(self, key: str, message: str, *args: Any) -> None:
        """Log a governance warning once per dedup key (module/kind pair)."""
        if key in self._warned:
            return
        self._warned.add(key)
        _logger.warning(message, *args)

    def _emit_policy_audit(self, decision: PolicyDecision, ctx: Context) -> None:
        """Audit a policy-driven override: log, span event, and bus event.

        Publishes ``apcore.policy.override`` (canonical name proposed in
        apcore#77, pending PROTOCOL_SPEC §9.16.2 amendment) when an event
        emitter is configured.
        """
        rule = decision.rule
        _logger.info(
            "Policy override: module=%s pattern=%s requires_approval=%s destructive=%s needs_approval=%s reason=%s",
            decision.module_id,
            rule.pattern if rule else "",
            decision.requires_approval,
            decision.destructive,
            decision.needs_approval,
            rule.reason if rule else "",
        )
        span = _current_trace_span(ctx, include_parent=True)
        if span is not None:
            span.events.append(
                {
                    "name": "policy_override",
                    "module_id": decision.module_id,
                    "pattern": rule.pattern if rule else "",
                    "requires_approval": decision.requires_approval,
                    "destructive": decision.destructive,
                    "needs_approval": decision.needs_approval,
                    "reason": (rule.reason if rule else None) or "",
                }
            )
        if self._event_emitter is not None:
            from datetime import datetime, timezone

            from apcore.events.emitter import ApCoreEvent

            self._event_emitter.emit(
                ApCoreEvent(
                    event_type="apcore.policy.override",
                    module_id=decision.module_id,
                    timestamp=datetime.now(timezone.utc).isoformat(),
                    severity="info",
                    data={
                        "module_id": decision.module_id,
                        "pattern": rule.pattern if rule else "",
                        "requires_approval": decision.requires_approval,
                        "destructive": decision.destructive,
                        "needs_approval": decision.needs_approval,
                        "reason": (rule.reason if rule else None) or "",
                        "trace_id": getattr(ctx, "trace_id", None),
                    },
                )
            )

    def _emit_audit(
        self, result: ApprovalResult, module_id: str, ctx: Context, resume_credential: str | None = None
    ) -> None:
        """Audit an approval decision: log, span event, and bus event.

        Publishes ``apcore.approval.decision`` (canonical name proposed in
        apcore#77, pending PROTOCOL_SPEC §9.16.2 amendment) when an event
        emitter is configured. Severity mirrors the outcome: approved/pending
        are ``info``; rejected/timeout (governance interventions) are ``warn``.
        """
        audit_id = result.audit_id if result.audit_id not in (result.approval_id, resume_credential) else None
        _logger.info(
            "Approval decision: module=%s status=%s approved_by=%s reason=%s",
            module_id,
            result.status,
            result.approved_by,
            result.reason,
        )
        span = _current_trace_span(ctx, include_parent=True)
        if span is not None:
            span.events.append(
                {
                    "name": "approval_decision",
                    "module_id": module_id,
                    "status": result.status,
                    "approved_by": result.approved_by or "",
                    "reason": result.reason or "",
                    "approval_id": audit_id or "",
                }
            )
        if self._event_emitter is not None:
            from datetime import datetime, timezone

            from apcore.events.emitter import ApCoreEvent

            self._event_emitter.emit(
                ApCoreEvent(
                    event_type="apcore.approval.decision",
                    module_id=module_id,
                    timestamp=datetime.now(timezone.utc).isoformat(),
                    severity="info" if result.status in ("approved", "pending") else "warn",
                    data={
                        "module_id": module_id,
                        "status": result.status,
                        "approved_by": result.approved_by,
                        "reason": result.reason,
                        "approval_id": audit_id,
                        "trace_id": getattr(ctx, "trace_id", None),
                    },
                )
            )


def _requires_approval(annotations: Any) -> bool:
    """Read ``requires_approval`` off an annotations value (dataclass or dict)."""
    if annotations is None:
        return False
    if isinstance(annotations, ModuleAnnotations):
        return annotations.requires_approval
    if isinstance(annotations, dict):
        return bool(annotations.get("requires_approval", False))
    return False


def _module_requires_approval(module: Any) -> bool:
    """Read ``requires_approval`` from a MODULE's own annotations.

    Kept for callers that genuinely mean the instance. Anything deciding
    governance wants the D-96 union instead — see
    :func:`apcore.schema.governance_union`.
    """
    return _requires_approval(getattr(module, "annotations", None))


def _is_destructive(annotations: Any) -> bool:
    """Read ``destructive`` off an annotations value (dataclass or dict)."""
    if annotations is None:
        return False
    if isinstance(annotations, ModuleAnnotations):
        return annotations.destructive
    if isinstance(annotations, dict):
        return bool(annotations.get("destructive", False))
    return False


def _module_is_destructive(module: Any) -> bool:
    """Read ``destructive`` from a MODULE's own annotations.

    Kept for callers that genuinely mean the instance; governance decisions
    want the D-96 union — see :func:`apcore.schema.governance_union`.
    """
    return _is_destructive(getattr(module, "annotations", None))


def _is_idempotent(module: Any) -> bool:
    """Whether a module declares itself safe to retry (``annotations.idempotent``).

    A timed-out call may already have applied side effects. Marking
    ``MODULE_TIMEOUT`` retryable is only safe when the module opts in, so the
    executor passes this to :class:`ModuleTimeoutError` instead of leaving the
    class default in place.
    """
    annotations = getattr(module, "annotations", None)
    if annotations is None:
        return False
    if isinstance(annotations, dict):
        return annotations.get("idempotent") is True
    return getattr(annotations, "idempotent", False) is True


def _read_module_timeout_ms(module: Any, module_id: str) -> int | None:
    """Read a module's declared per-module timeout in milliseconds, or None.

    The declaration lives at ``resources.timeout`` (PROTOCOL_SPEC §5.2,
    ``docs/features/core-executor.md`` §Timeout Specification). Three spellings
    are accepted, matching how each SDK's descriptor stores it:

    * ``module.resources["timeout"]`` — the direct attribute, as
      apcore-typescript's ``mod['resources']`` reads it;
    * ``module.annotations.extra["resources"]["timeout"]`` — where
      ``ModuleAnnotations.from_dict`` files a non-canonical top-level key, and
      where apcore-rust reads it (``annotations.extra["resources"]``);
    * ``module.annotations["resources"]["timeout"]`` — a raw dict annotations
      block, as apcore-typescript's ``mod.annotations.resources`` reads it.

    Prior revisions read ``module.timeout_ms``, an attribute no apcore-python
    module ever defines. The per-module half of the dual-timeout model was
    therefore dead code: every call fell back to ``executor.default_timeout``
    and the "negative timeout MUST raise ``GENERAL_INVALID_INPUT``" rule was
    unreachable.

    Returns None when nothing is declared or the value is not a real number, so
    the caller falls back to the default. ``bool`` is excluded on purpose: it
    subclasses ``int`` in Python, and ``True`` must not become a 1 ms timeout.

    Raises:
        InvalidInputError: When a declared timeout is negative.
    """

    def read_number(value: Any) -> int | float | None:
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            return None
        if value != value or value in (float("inf"), float("-inf")):  # NaN / infinity
            return None
        if value < 0:
            raise InvalidInputError(
                message=f"Module '{module_id}' declares a negative timeout: {value}",
                code="GENERAL_INVALID_INPUT",
            )
        return value

    def read_resources(resources: Any) -> int | float | None:
        if isinstance(resources, dict):
            return read_number(resources.get("timeout"))
        if resources is not None:
            return read_number(getattr(resources, "timeout", None))
        return None

    declared = read_resources(getattr(module, "resources", None))

    if declared is None:
        annotations = getattr(module, "annotations", None)
        if isinstance(annotations, ModuleAnnotations):
            declared = read_resources(annotations.extra.get("resources"))
        elif isinstance(annotations, dict):
            declared = read_resources(annotations.get("resources"))
        elif annotations is not None:
            extra = getattr(annotations, "extra", None)
            if isinstance(extra, dict):
                declared = read_resources(extra.get("resources"))

    return None if declared is None else int(declared)


def _coerce_annotations(annotations: Any) -> ModuleAnnotations:
    """Coerce raw module annotations into a ``ModuleAnnotations`` instance."""
    if isinstance(annotations, ModuleAnnotations):
        return annotations
    if isinstance(annotations, dict):
        valid_fields = {f.name for f in dataclasses.fields(ModuleAnnotations)}
        return ModuleAnnotations(**{k: v for k, v in annotations.items() if k in valid_fields})
    return ModuleAnnotations()


# ---------------------------------------------------------------------------
# Step 6: Middleware Before
# ---------------------------------------------------------------------------


class BuiltinMiddlewareBefore(BaseStep):
    """Execute middleware before-chain.

    Always operates through a ``MiddlewareManager`` — when the caller does
    not supply one, an internal manager is built from the ``middlewares``
    list. This collapses what used to be two parallel execute() paths
    (manager + raw-list fallback) into one.
    """

    def __init__(
        self,
        *,
        middlewares: list[Any] | None = None,
        middleware_manager: Any | None = None,
    ) -> None:
        super().__init__(
            name="middleware_before",
            description="Run before-middleware chain",
            removable=True,
            replaceable=False,
            pure=False,
        )
        self._manager = _ensure_middleware_manager(middleware_manager, middlewares)

    async def execute(self, ctx: PipelineContext) -> StepResult:
        from apcore.middleware.manager import MiddlewareChainError

        try:
            if hasattr(self._manager, "execute_before_async"):
                inputs, executed = await self._manager.execute_before_async(
                    ctx.module_id,
                    ctx.inputs,
                    ctx.context,
                )
            else:
                inputs, executed = self._manager.execute_before(
                    ctx.module_id,
                    ctx.inputs,
                    ctx.context,
                )
            ctx.inputs = inputs
            ctx.executed_middlewares = list(executed)
        except MiddlewareChainError as exc:
            # Store executed middlewares for the executor's on_error recovery
            ctx.executed_middlewares = list(exc.executed_middlewares)
            raise
        except Exception as exc:
            # on_error recovery for non-chain errors
            if hasattr(self._manager, "execute_on_error_async"):
                recovery = await self._manager.execute_on_error_async(
                    ctx.module_id,
                    ctx.inputs,
                    exc,
                    ctx.context,
                    ctx.executed_middlewares,
                )
            elif hasattr(self._manager, "execute_on_error"):
                recovery = self._manager.execute_on_error(
                    ctx.module_id,
                    ctx.inputs,
                    exc,
                    ctx.context,
                    ctx.executed_middlewares,
                )
            else:
                recovery = None
            # Clear executed_middlewares after on_error to prevent double invocation
            ctx.executed_middlewares = []
            if recovery is not None:
                if ctx._stream_observation is not None and ctx._stream_observation.captures_completion:
                    ctx._stream_observation.record_execution_error(exc)
                ctx.output = recovery
                return StepResult(action="skip_to", skip_to="return_result")
            raise
        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Step 7: Input Validation
# ---------------------------------------------------------------------------


class BuiltinInputValidation(BaseStep):
    """Validate inputs against module schema and redact sensitive fields."""

    def __init__(self, *, redaction: Any | None = None) -> None:
        super().__init__(
            name="input_validation",
            description="Validate inputs against schema and redact sensitive fields",
            removable=True,
            replaceable=True,
            pure=True,
            requires=("module",),
            provides=("validated_inputs",),
        )
        self._redaction = redaction

    async def execute(self, ctx: PipelineContext) -> StepResult:
        module = ctx.module
        if module is None:
            return StepResult(action="abort", explanation="No module set on context")

        input_schema = getattr(module, "input_schema", None)
        if input_schema is None:
            ctx.validated_inputs = ctx.inputs
            if ctx.context is not None and hasattr(ctx.context, "redacted_inputs"):
                ctx.context.redacted_inputs = _redact_payload(strip_approval_token(ctx.inputs), None, self._redaction)
            return StepResult(action="continue")

        # The module-invocation boundary always validates without type coercion.
        validated = _validate_module_payload(input_schema, ctx.inputs, "Input")
        warn_format_violations(ctx.inputs, input_schema)

        ctx.validated_inputs = ctx.inputs
        if isinstance(validated, pydantic.BaseModel) and not isinstance(
            getattr(input_schema, "__apcore_source_schema__", None), dict
        ):
            if isinstance(module, FunctionModule):
                ctx.validated_inputs = module._validated_call_inputs(ctx.inputs, validated)
            elif getattr(module, "native_inputs", False) is True:
                fields = type(validated).model_fields.keys() & validated.model_fields_set
                ctx.validated_inputs = {
                    **(validated.model_extra or {}),
                    **{key: getattr(validated, key) for key in fields},
                }

        # Redact sensitive fields after successful validation
        if ctx.context is not None and hasattr(ctx.context, "redacted_inputs"):
            ctx.context.redacted_inputs = _redact_payload(
                strip_approval_token(ctx.inputs), input_schema, self._redaction
            )

        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Step 8: Execute
# ---------------------------------------------------------------------------


class BuiltinExecute(BaseStep):
    """Execute the module with timeout enforcement."""

    def __init__(self, *, config: Any | None = None) -> None:
        super().__init__(
            name="execute",
            description="Execute module with timeout",
            removable=False,
            replaceable=True,
            pure=False,
            requires=("module",),
            provides=("output",),
        )
        self._config = config

    @staticmethod
    def _end_for_timeout(ctx: PipelineContext, timeout_ms: int) -> ModuleTimeoutError:
        token = getattr(ctx.context, "cancel_token", None)
        observation = ctx._stream_observation
        error = ModuleTimeoutError(module_id=ctx.module_id, timeout_ms=timeout_ms, retryable=_is_idempotent(ctx.module))
        if observation is not None:
            if token is not None:
                token.check()
            observation.report_timeout(error)
        if token is not None:
            token.cancel()
        return error

    async def execute(self, ctx: PipelineContext) -> StepResult:
        module = ctx.module
        if module is None:
            return StepResult(action="abort", explanation="No module set on context")

        # Check cancel token
        cancel_token = getattr(ctx.context, "cancel_token", None)
        if cancel_token is not None and cancel_token.is_cancelled:
            raise ExecutionCancelledError()

        configured_timeout = self._config.get("executor.default_timeout") if self._config is not None else None
        default_timeout = (
            Config.get_default("executor.default_timeout") if configured_timeout is None else configured_timeout
        )
        # Check global deadline
        # D-99: `global_deadline` is epoch seconds, so it is compared against
        # `time.time()` — never a monotonic reading.
        global_deadline = getattr(ctx.context, "global_deadline", None)
        if global_deadline is not None and time.time() > global_deadline:
            raise self._end_for_timeout(ctx, int(default_timeout))

        inputs = ctx.validated_inputs if ctx.validated_inputs is not None else ctx.inputs

        if ctx.stream:
            from apcore.registry.registry import _validate_streaming_signature

            try:
                _validate_streaming_signature(ctx.module_id, module)
            except StreamingInterfaceError:
                pass  # No compatible streaming method. Execute once as a fallback.
            else:
                stream = module.stream(inputs, ctx.context)
                if inspect.isawaitable(stream):
                    stream = await stream
                ctx.output_stream = aiter(stream)
                close_stream = getattr(ctx.output_stream, "aclose", None)
                if ctx.execution_scope is not None and callable(close_stream):
                    ctx.execution_scope.push_async_callback(
                        _close_stream_source, ctx, cast(Callable[[], Awaitable[None]], close_stream)
                    )
                return StepResult(action="skip_to", skip_to="return_result")

        # Determine per-module timeout. `0` means "no per-module limit" on both
        # sides of the fallback — the same reading apcore-typescript
        # (`if (timeoutMs === 0)`) and apcore-rust (`if timeout_ms > 0`) use.
        # The global deadline below still applies.
        module_timeout_ms = _read_module_timeout_ms(module, ctx.module_id)
        effective_timeout_ms = default_timeout if module_timeout_ms is None else module_timeout_ms
        timeout_s = effective_timeout_ms / 1000.0 if effective_timeout_ms > 0 else None

        # Clamp to global deadline if set
        if global_deadline is not None:
            remaining = global_deadline - time.time()
            if remaining <= 0:
                raise self._end_for_timeout(ctx, int(default_timeout))
            if timeout_s is None or remaining < timeout_s:
                timeout_s = remaining

        try:
            execute = module.execute
            managed_reload = False
            if ctx.module_id == "system.control.reload_module" and "lifecycle" in inputs:
                from apcore.sys_modules.control import ReloadModule

                if isinstance(module, ReloadModule):
                    execute = module._execute_managed
                    managed_reload = True
            is_async = inspect.iscoroutinefunction(execute)
            if is_async:
                coro = execute(inputs, ctx.context)
            else:
                loop = asyncio.get_event_loop()
                coro = loop.run_in_executor(None, execute, inputs, ctx.context)

            task = ctx.module_task = asyncio.ensure_future(coro)
            # Consume late failures even when the call was cancelled while
            # waiting and no request remains to observe the result.
            task.add_done_callback(_discard_late_execution)
            if timeout_s is not None:
                try:
                    completed, _ = await asyncio.wait({task}, timeout=timeout_s)
                except asyncio.CancelledError:
                    if managed_reload:
                        task.cancel()
                    raise
                if not completed:
                    # Cancellation is cooperative: do not wait for cleanup or
                    # forcibly cancel the module's coroutine. Consume a late
                    # exception so a discarded result cannot become a warning.
                    raise self._end_for_timeout(ctx, int(timeout_s * 1000))
                output = task.result()
            else:
                # Cancelling the asyncio wrapper does not stop a worker thread.
                output = await task if is_async else await asyncio.shield(task)
            output = normalize_module_output(output)
            if output is None:
                raise ModuleExecuteError(module_id=ctx.module_id, message="Return value cannot be None")
            if not isinstance(output, dict):
                raise ModuleExecuteError(module_id=ctx.module_id, message="Return value must be Map")
            ctx.output = output
        except asyncio.TimeoutError:
            timeout_ms = int((timeout_s or 0) * 1000)
            raise ModuleTimeoutError(
                module_id=ctx.module_id,
                timeout_ms=timeout_ms,
                retryable=_is_idempotent(module),
            ) from None

        return StepResult(action="continue")


def _discard_late_execution(task: asyncio.Future[Any]) -> None:
    """Consume a timed-out invocation's eventual exception or output."""
    if not task.cancelled():
        task.exception()


async def _close_stream_source(ctx: PipelineContext, close: Callable[[], Awaitable[None]]) -> None:
    """Close the source once its pending read has stopped, then release its lease."""
    task = ctx.module_task
    if task is None or task.done():
        await close()
        return

    async def finish(pending: asyncio.Future[Any]) -> None:
        try:
            await pending
        except (Exception, asyncio.CancelledError):
            pass
        await close()

    cleanup = ctx.module_task = asyncio.create_task(finish(task))
    cleanup.add_done_callback(_discard_late_execution)


# ---------------------------------------------------------------------------
# Step 9: Output Validation
# ---------------------------------------------------------------------------


class BuiltinOutputValidation(BaseStep):
    """Validate output against module schema and redact sensitive fields."""

    def __init__(self, *, redaction: Any | None = None) -> None:
        super().__init__(
            name="output_validation",
            description="Validate output against schema and redact sensitive fields",
            removable=True,
            replaceable=True,
            pure=True,
            requires=("module", "output"),
            provides=("validated_output",),
        )
        self._redaction = redaction

    async def execute(self, ctx: PipelineContext) -> StepResult:
        module = ctx.module
        if module is None:
            return StepResult(action="abort", explanation="No module set on context")

        output_schema = getattr(module, "output_schema", None)
        if output_schema is None:
            ctx.validated_output = ctx.output
            if ctx.context is not None and hasattr(ctx.context, "redacted_output"):
                ctx.context.redacted_output = (
                    _redact_payload(ctx.output, None, self._redaction) if isinstance(ctx.output, dict) else None
                )
            return StepResult(action="continue")

        if ctx.output is None:
            ctx.validated_output = None
            if ctx.context is not None and hasattr(ctx.context, "redacted_output"):
                ctx.context.redacted_output = None
            return StepResult(action="continue")

        _validate_module_payload(output_schema, ctx.output, "Output")
        warn_format_violations(ctx.output, output_schema)

        ctx.validated_output = ctx.output

        # Store redacted output as first-class Context field (symmetric with
        # redacted_inputs). Previously stored under ctx.context.data["_apcore.
        # executor.redacted_output"] which was filtered out by serialize().
        if ctx.context is not None and hasattr(ctx.context, "redacted_output"):
            ctx.context.redacted_output = _redact_payload(ctx.output, output_schema, self._redaction)

        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Step 10: Middleware After
# ---------------------------------------------------------------------------


class BuiltinMiddlewareAfter(BaseStep):
    """Execute middleware after-chain.

    Always operates through a ``MiddlewareManager`` — when the caller does
    not supply one, an internal manager is built from the ``middlewares``
    list. Mirrors :class:`BuiltinMiddlewareBefore` so the two steps share
    the same execution model.
    """

    def __init__(
        self,
        *,
        middlewares: list[Any] | None = None,
        middleware_manager: Any | None = None,
    ) -> None:
        super().__init__(
            name="middleware_after",
            description="Run after-middleware chain",
            removable=True,
            replaceable=False,
            pure=False,
        )
        self._manager = _ensure_middleware_manager(middleware_manager, middlewares)

    async def execute(self, ctx: PipelineContext) -> StepResult:
        from apcore.middleware.manager import MiddlewareManager

        if isinstance(self._manager, MiddlewareManager):
            output = await self._manager.execute_after_async(
                ctx.module_id,
                ctx.inputs,
                ctx.output or {},
                ctx.context,
                executed_middlewares=ctx.executed_middlewares,
            )
        elif hasattr(self._manager, "execute_after_async"):
            output = await self._manager.execute_after_async(
                ctx.module_id,
                ctx.inputs,
                ctx.output or {},
                ctx.context,
            )
        else:
            output = self._manager.execute_after(
                ctx.module_id,
                ctx.inputs,
                ctx.output or {},
                ctx.context,
            )
        ctx.output = output
        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Step 11: Return Result
# ---------------------------------------------------------------------------


class BuiltinReturnResult(BaseStep):
    """Finalize pipeline output. Output is already on ctx."""

    def __init__(self) -> None:
        super().__init__(
            name="return_result",
            description="Finalize pipeline result",
            removable=False,
            replaceable=False,
            pure=True,
            requires=("output",),
        )

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Factory
# ---------------------------------------------------------------------------


def build_standard_strategy(
    *,
    registry: Any,
    config: Any | None = None,
    acl: Any | None = None,
    approval_handler: Any | None = None,
    policy: ExecutionPolicy | None = None,
    event_emitter: Any | None = None,
    middlewares: list[Any] | None = None,
    middleware_manager: Any | None = None,
    executor: Any | None = None,
    toggle_state: Any | None = None,
) -> ExecutionStrategy:
    """Build the standard 11-step execution strategy.

    Args:
        registry: Module registry for looking up modules by ID.
        config: Optional configuration for timeout/depth settings.
        acl: Optional ACL for access control enforcement.
        approval_handler: Optional approval handler for the approval gate.
        policy: Optional ExecutionPolicy with governance overrides for the
            approval gate (apcore#76 RFC pilot).
        event_emitter: Optional EventEmitter. When provided, the approval gate
            publishes apcore.approval.decision / apcore.policy.override
            governance events (apcore#77).
        middlewares: Optional list of middleware instances.
        middleware_manager: Optional MiddlewareManager for production parity.
        executor: Optional executor reference for context creation and approval.
        toggle_state: Optional per-instance ToggleState for the module-lookup
            read path (#71). When None, BuiltinModuleLookup falls back to the
            process-global ``_default_toggle_state`` for back-compat.

    Returns:
        An ExecutionStrategy containing the 11 built-in steps.
    """
    # PROTOCOL_SPEC §10.6.1 requirement 5: compiled once, here, rather than at
    # every execution — and the SAME object reaches all three capture points, so
    # the two `redacted_inputs` writers and the `redacted_output` writer cannot
    # drift apart the way the two log-emission passes did.
    redaction = _resolve_redaction(config)
    return ExecutionStrategy(
        "standard",
        [
            BuiltinContextCreation(config=config, executor=executor),
            BuiltinCallChainGuard(config=config),
            BuiltinModuleLookup(registry=registry, toggle_state=toggle_state, redaction=redaction),
            BuiltinACLCheck(acl=acl, event_emitter=event_emitter),
            BuiltinApprovalGate(handler=approval_handler, policy=policy, event_emitter=event_emitter),
            BuiltinMiddlewareBefore(
                middlewares=middlewares or [],
                middleware_manager=middleware_manager,
            ),
            BuiltinInputValidation(redaction=redaction),
            BuiltinExecute(config=config),
            BuiltinOutputValidation(redaction=redaction),
            BuiltinMiddlewareAfter(
                middlewares=middlewares or [],
                middleware_manager=middleware_manager,
            ),
            BuiltinReturnResult(),
        ],
    )


def build_internal_strategy(**kwargs: Any) -> ExecutionStrategy:
    """Build an internal strategy: standard minus acl_check and approval_gate.

    Suitable for trusted internal calls that skip access control.

    Args:
        **kwargs: Forwarded to build_standard_strategy().

    Returns:
        An ExecutionStrategy named "internal".
    """
    s = build_standard_strategy(**kwargs)
    s.remove("acl_check")
    s.remove("approval_gate")
    s.name = "internal"
    return s


def build_testing_strategy(**kwargs: Any) -> ExecutionStrategy:
    """Build a testing strategy: standard minus acl, approval, and call chain guard.

    Suitable for unit/integration tests that need minimal overhead.

    Args:
        **kwargs: Forwarded to build_standard_strategy().

    Returns:
        An ExecutionStrategy named "testing".
    """
    s = build_standard_strategy(**kwargs)
    s.remove("acl_check")
    s.remove("approval_gate")
    s.remove("call_chain_guard")
    s.name = "testing"
    return s


def build_performance_strategy(**kwargs: Any) -> ExecutionStrategy:
    """Build a performance strategy: standard minus middleware steps.

    Suitable for latency-sensitive calls that skip middleware overhead.

    Args:
        **kwargs: Forwarded to build_standard_strategy().

    Returns:
        An ExecutionStrategy named "performance".
    """
    s = build_standard_strategy(**kwargs)
    s.remove("middleware_before")
    s.remove("middleware_after")
    s.name = "performance"
    return s


def build_minimal_strategy(**kwargs: Any) -> ExecutionStrategy:
    """Build a minimal strategy: context → lookup → execute → return only.

    Suitable for pre-validated internal hot paths where ACL, approval,
    middleware, and schema validation are unnecessary. Use with caution —
    no safety checks, no input/output validation, no middleware.

    Args:
        **kwargs: Forwarded to build_standard_strategy().

    Returns:
        An ExecutionStrategy named "minimal" with 4 steps.
    """
    s = build_standard_strategy(**kwargs)
    s.remove("call_chain_guard")
    s.remove("acl_check")
    s.remove("approval_gate")
    s.remove("middleware_before")
    s.remove("input_validation")
    s.remove("output_validation")
    s.remove("middleware_after")
    s.name = "minimal"
    return s
