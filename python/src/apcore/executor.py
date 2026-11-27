"""Executor — the module execution engine for apcore.

Resolves a module by ID, validates inputs against its schema, enforces ACL
and approval policies, runs the middleware chain, and returns the result.
Supports sync, async, and streaming execution modes.
"""

from __future__ import annotations

import inspect
import logging
import threading
import time
from collections.abc import AsyncGenerator, AsyncIterator
from contextlib import AsyncExitStack, aclosing
from copy import copy, deepcopy
from typing import TYPE_CHECKING, Any, Callable

from apcore._sync_bridge import _SyncBridge
from apcore.acl import ACL
from apcore.approval import ApprovalHandler
from apcore.builtin_steps import (
    BuiltinContextCreation,
    BuiltinModuleLookup,
    _bind_post_validation_policy,
    _is_idempotent,
)
from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.config import Config
from apcore.context import Context
from apcore.context_keys import _current_trace_span
from apcore.errors import (
    ACLDeniedError,
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    ContextBindingError,
    ErrorCodes,
    InvalidInputError,
    ModuleError,
    ModuleNotFoundError,
    ModuleTimeoutError,
    PolicyEvaluationError,
    SchemaValidationError,
)
from apcore.utils.error_propagation import propagate_error
from apcore.utils.module_output import normalize_module_output
from apcore.middleware import AfterMiddleware, BeforeMiddleware, Middleware
from apcore.middleware.manager import (
    MiddlewareChainError,
    MiddlewareManager,
    RetrySignal,
)
from apcore.schema.annotations import governance_union
from apcore.module import (
    Change,
    ModuleAnnotations,
    PreflightCheckResult,
    PreflightResult,
    PreviewResult,
)

from apcore.policy import ExecutionPolicy, PolicyEvaluator, strip_approval_token
from apcore.pipeline import (
    AbortReason,
    ExecutionStrategy,
    GovernanceState,
    PipelineAbortError,
    PipelineContext,
    PipelineEngine,
    PipelineStepError,
    PipelineTrace,
    StrategyInfo,
    StrategyNotFoundError,
    _any_match,
)
from apcore.registry import MAX_MODULE_ID_LENGTH, MODULE_ID_PATTERN, Registry
from apcore.streaming import StreamExecution, _StreamObservation

if TYPE_CHECKING:
    from apcore.sys_modules.control import ToggleState


__all__ = ["Executor"]
# NOTE: redact_sensitive and REDACTED_VALUE are kept importable from this module
# for backward compatibility but their canonical path is apcore.utils.redaction.

_logger = logging.getLogger(__name__)

# Canonical PreflightResult check names (PROTOCOL_SPEC §12.8.4 enum). Named
# rather than spelled inline because §12.8.5.1 makes `acl` the switch that
# withholds the other two — a rename that silently stopped matching would turn
# the disclosure gate off. apcore-rust exports the same three as public
# constants (`apcore::ACL_CHECK_NAME` and friends).
_ACL_CHECK = "acl"
_MODULE_PREFLIGHT_CHECK = "module_preflight"
_MODULE_PREVIEW_CHECK = "module_preview"

# Map pipeline step names to PreflightResult check names
_STEP_TO_CHECK: dict[str, str] = {
    "context_creation": "context",
    "call_chain_guard": "call_chain",
    "module_lookup": "module_lookup",
    "acl_check": _ACL_CHECK,
    "approval_gate": "approval",
    "middleware_before": "middleware",
    "input_validation": "schema",
}


def _trace_to_checks(
    trace: PipelineTrace, errors: dict[str, dict[str, Any]] | None = None
) -> list[PreflightCheckResult]:
    """Convert PipelineTrace steps to PreflightCheckResult list."""
    checks: list[PreflightCheckResult] = []
    for st in trace.steps:
        if st.skipped:
            continue
        check_name = _STEP_TO_CHECK.get(st.name, st.name)
        passed = st.result.action != "abort"
        error = None
        if not passed and st.result.explanation:
            error = (errors or {}).get(st.name) or {
                "code": f"STEP_{st.name.upper()}_FAILED",
                "message": st.result.explanation,
            }
        checks.append(PreflightCheckResult(check=check_name, passed=passed, error=error))
    return checks


_PREFLIGHT_CHECK_BY_TYPE: list[tuple[type[BaseException], str]] = [
    # Ordered: more specific first. isinstance() matches subclasses, so
    # walking in declaration order is enough to give the narrowest label.
    (ModuleNotFoundError, "module_lookup"),
    (ACLDeniedError, _ACL_CHECK),
    (SchemaValidationError, "schema"),
    (InvalidInputError, "schema"),
    (CallDepthExceededError, "call_chain"),
    (CircularCallError, "call_chain"),
    (CallFrequencyExceededError, "call_chain"),
]


def _preflight_check_for(exc: BaseException) -> str:
    """Map an exception raised mid-pipeline to a preflight check name.

    Drives off the error class hierarchy (``isinstance``) rather than error
    code strings so adding a new ``Approval*Error`` or ``Config*Error``
    subclass inherits a sensible check label via its base class instead of
    silently falling through to ``"unknown"``.
    """
    for error_cls, check_name in _PREFLIGHT_CHECK_BY_TYPE:
        if isinstance(exc, error_cls):
            return check_name
    return "unknown"


_MAX_MERGE_DEPTH = 32
_SYNC_BRIDGE_GRACE_SECONDS = 1.0


def _validate_bridge_timeout(timeout_ms: int | None) -> None:
    if timeout_ms is not None and (type(timeout_ms) is not int or not 0 <= timeout_ms <= threading.TIMEOUT_MAX * 1000):
        raise InvalidInputError(message="Sync bridge timeout must be a non-negative integer in milliseconds")


def _json_type_name(value: Any) -> str:
    """Return the JSON type name for *value*, matching Rust's ``json_type_name``.

    Used to label invalid streaming chunks identically across SDKs. ``bool``
    MUST be checked before ``int`` because ``bool`` is a subclass of ``int``.
    """
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__


def _resolve_merge_depth(config: Any) -> int:
    """Resolve ``stream.max_merge_depth`` (PROTOCOL_SPEC §5, canonical default 32).

    A non-positive or non-integer value falls back to the canonical default
    rather than disabling the cap: the cap exists to prevent stack exhaustion
    from adversarial chunk shapes, so a misconfiguration must not remove it.
    """
    if config is None:
        return _MAX_MERGE_DEPTH
    try:
        value = config.get("stream.max_merge_depth", _MAX_MERGE_DEPTH)
    except Exception:  # noqa: BLE001 — a Config that cannot answer is not fatal here
        return _MAX_MERGE_DEPTH
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        return _MAX_MERGE_DEPTH
    return value


def _deep_merge(
    base: dict[str, Any],
    override: dict[str, Any],
    *,
    _depth: int = 0,
    max_depth: int | None = None,
) -> None:
    """Recursively merge *override* into *base* in-place.

    Nested dicts are merged recursively; all other values (including lists)
    are replaced by the override value. Recursion is capped to guard against
    malicious or extremely nested streaming chunks.

    ``max_depth`` is the cap, defaulting to :data:`_MAX_MERGE_DEPTH` (32).
    PROTOCOL_SPEC §5 calls 32 the *canonical default*, which implies an
    override — and until now there was none: ``stream.max_merge_depth`` was a
    declared, schema-documented key that no code path read (apcore#118), so
    the constant WAS the contract. :meth:`Executor.stream` now resolves the key
    and passes it here.
    """
    cap = _MAX_MERGE_DEPTH if max_depth is None else max_depth
    if _depth >= cap:
        # At the depth cap, replace rather than recurse: the right value wins
        # at this level without further merging (mirrors apcore-rust).
        for key, value in override.items():
            base[key] = value
        return
    for key, value in override.items():
        if key in base and isinstance(base[key], dict) and isinstance(value, dict):
            nested = dict(base[key])
            _deep_merge(nested, value, _depth=_depth + 1, max_depth=cap)
            base[key] = nested
        else:
            base[key] = value


#: Steps whose removal withdraws a protection rather than a convenience
#: (PROTOCOL_SPEC §5.16 requirement 7).
_SECURITY_STEPS = ("acl_check", "approval_gate")


def _pipeline_section(config: Any | None) -> dict[str, Any]:
    """The `pipeline:` block of a loaded configuration, or `{}`."""
    if config is None:
        return {}
    section = config.get("pipeline")
    return section if isinstance(section, dict) and section else {}


def _warn_removed_security_steps(pipeline_config: dict[str, Any]) -> None:
    """§5.16 requirement 7 — removing `acl_check` or `approval_gate` warns.

    The reason is the TRANSITION, not the steady state. Requirement 6 makes a
    previously ignored section take effect, so a configuration that has been
    carrying `remove: [acl_check]` while ACL was enforced anyway starts having
    ACL genuinely removed. That is the operator getting what they asked for, and
    equally the one direction in which honouring configuration can withdraw a
    protection that was in place a moment earlier.

    A notice, never a refusal: the configuration is valid and was written
    deliberately, and rejecting it would break projects whose `pipeline:` block
    is harmless.
    """
    removed = [name for name in pipeline_config.get("remove", []) if name in _SECURITY_STEPS]
    if not removed:
        return
    _logger.warning(
        "pipeline.remove takes %s out of the execution pipeline. Until spec v1.43.0 a "
        "`pipeline:` section was accepted and then ignored, so this removal may not have "
        "been in effect before this release even though it was configured. "
        "%s. See PROTOCOL_SPEC 5.16 requirement 7.",
        " and ".join(repr(name) for name in removed),
        "Inter-module calls are no longer access-checked"
        if "acl_check" in removed
        else "Calls requiring approval are no longer gated",
    )


class Executor:
    """Central execution engine that orchestrates the module call pipeline.

    The Executor implements a robust execution flow: context creation, safety checks,
    module lookup, ACL enforcement, approval gate, input validation with
    redaction, middleware before chain, module execution, output validation,
    middleware after chain, and result return.
    """

    _registered_strategies: dict[str, ExecutionStrategy] = {}

    def __init__(
        self,
        registry: Registry,
        *,
        strategy: ExecutionStrategy | str | None = None,
        middlewares: list[Middleware] | None = None,
        acl: ACL | None = None,
        config: Config | None = None,
        approval_handler: ApprovalHandler | None = None,
        policy: ExecutionPolicy | None = None,
        policy_evaluator: PolicyEvaluator | None = None,
        event_emitter: Any = None,
        toggle_state: Any = None,
        sync_bridge_timeout_ms: int | None = None,
    ) -> None:
        """Initialize the Executor.

        Args:
            registry: Module registry for looking up modules by ID.
            strategy: Optional execution strategy. Can be an ExecutionStrategy
                instance, a preset name string ("standard", "internal",
                "testing", "performance", "minimal"), or None (defaults to
                standard).
            middlewares: Optional list of middleware instances to register.
            acl: Optional ACL for access control enforcement.
            config: Optional configuration for timeout/depth settings.
            approval_handler: Optional approval handler for Step 5 gate.
            policy: Optional ExecutionPolicy with execution-time governance
                overrides for the Step 5 gate (apcore#76 RFC pilot).
            policy_evaluator: Optional evaluator for the validated invocation.
            event_emitter: Optional EventEmitter. When provided, the executor
                emits apcore.stream.post_validation_failed events so
                post-stream failures (which cannot un-send already-yielded
                chunks) are still visible to subscribers.
            toggle_state: Optional per-instance ToggleState (#71). Injected into
                the module-lookup read path so each owning APCore instance sees
                only its own toggles. When None, the built-in strategy falls
                back to the process-global ``_default_toggle_state`` for
                back-compat with callers that construct an Executor directly.
            sync_bridge_timeout_ms: Default wait for the sync thread bridge.
                None uses the Python SDK configuration or derives the wait
                from the call deadline. Zero disables the wait bound.
        """
        if sync_bridge_timeout_ms is None and config is not None:
            sync_bridge_timeout_ms = config.get("apcore-python.executor.sync_bridge_timeout")
        _validate_bridge_timeout(sync_bridge_timeout_ms)
        self._sync_bridge_timeout_ms = sync_bridge_timeout_ms
        self._registry = registry
        self._middleware_manager = MiddlewareManager()
        self._acl = acl
        self._config = config
        self._approval_handler = approval_handler
        self._policy = policy
        self._event_emitter = event_emitter
        self._event_emitter_configured = event_emitter is not None
        self._toggle_state = toggle_state
        self._toggle_state_lock = threading.Lock()

        if middlewares:
            for mw in middlewares:
                self._middleware_manager.add(mw)

        # Resolve strategy (pass middleware_manager and executor for production parity)
        strategy_kwargs: dict[str, Any] = {
            "registry": registry,
            "config": config,
            "acl": acl,
            "approval_handler": approval_handler,
            "policy": policy,
            "event_emitter": event_emitter,
            "middlewares": middlewares,
            "middleware_manager": self._middleware_manager,
            "executor": self,
            "toggle_state": toggle_state,
        }
        if strategy is None:
            from apcore.builtin_steps import build_standard_strategy

            # PROTOCOL_SPEC §5.16 requirement 6: a configured `pipeline:` section
            # MUST be applied. The builder has always existed — it takes the
            # section as a dict — but nothing extracted that dict from a loaded
            # Config, so `pipeline: remove: [acl_check]` in apcore.yaml left all
            # eleven steps in place and a declared custom step silently never
            # ran (apcore#118, decision D-72).
            #
            # Only when the caller supplied no explicit strategy: an explicit
            # one is an API argument and wins over configuration, per D-73.
            pipeline_config = _pipeline_section(config)
            if pipeline_config:
                from apcore.pipeline_config import build_strategy_from_config

                _warn_removed_security_steps(pipeline_config)
                self._strategy = build_strategy_from_config(pipeline_config, **strategy_kwargs)
            else:
                self._strategy = build_standard_strategy(**strategy_kwargs)
        elif isinstance(strategy, str):
            self._strategy = self._resolve_strategy_name(strategy, **strategy_kwargs)
        else:
            self._strategy = strategy

        # PROTOCOL_SPEC §6.6.5.5 (D-129): however the strategy was supplied, the
        # executor's providers reach its built-in gate steps. A pre-built
        # instance never received them, so a deny-all ACL let every call through
        # while governance_state() reported the gate as configured and wired.
        self._strategy = self._bind_providers(self._strategy)
        self.set_policy_evaluator(policy_evaluator)

        self._pipeline_engine = PipelineEngine()

        if config is not None:
            val = config.get("executor.default_timeout")
            self._default_timeout: int = val if val is not None else Config.get_default("executor.default_timeout")
            val = config.get("executor.global_timeout")
            self._global_timeout: int = val if val is not None else Config.get_default("executor.global_timeout")
            val = config.get("executor.max_call_depth")
            self._max_call_depth: int = val if val is not None else Config.get_default("executor.max_call_depth")
            val = config.get("executor.max_module_repeat")
            self._max_module_repeat: int = val if val is not None else Config.get_default("executor.max_module_repeat")
        else:
            self._default_timeout = Config.get_default("executor.default_timeout")
            self._global_timeout = Config.get_default("executor.global_timeout")
            self._max_call_depth = Config.get_default("executor.max_call_depth")
            self._max_module_repeat = Config.get_default("executor.max_module_repeat")

        if self._default_timeout < 0:
            raise InvalidInputError(
                message=f"Negative default_timeout: {self._default_timeout}",
            )

        self._sync_bridge = _SyncBridge()

    @classmethod
    def from_registry(
        cls,
        registry: Registry,
        *,
        strategy: ExecutionStrategy | str | None = None,
        middlewares: list[Middleware] | None = None,
        acl: ACL | None = None,
        config: Config | None = None,
        approval_handler: ApprovalHandler | None = None,
        policy: ExecutionPolicy | None = None,
        policy_evaluator: PolicyEvaluator | None = None,
        sync_bridge_timeout_ms: int | None = None,
    ) -> Executor:
        """Convenience factory for creating an Executor from a Registry.

        Args:
            registry: The module registry.
            strategy: Optional execution strategy or preset name.
            middlewares: Optional middleware list.
            acl: Optional access control list.
            config: Optional configuration.
            approval_handler: Optional approval handler.
            policy: Optional execution-time governance policy (apcore#76).
            policy_evaluator: Optional evaluator for the validated invocation.
            sync_bridge_timeout_ms: Optional wait limit for the sync thread bridge.

        Returns:
            A configured Executor instance.
        """
        return cls(
            registry=registry,
            strategy=strategy,
            middlewares=middlewares,
            acl=acl,
            config=config,
            approval_handler=approval_handler,
            policy=policy,
            policy_evaluator=policy_evaluator,
            sync_bridge_timeout_ms=sync_bridge_timeout_ms,
        )

    @property
    def registry(self) -> Registry:
        """Return the Registry instance."""
        return self._registry

    @property
    def toggle_state(self) -> ToggleState:
        """Return the toggle state used by this executor's module lookup."""
        from apcore.sys_modules.control import _default_toggle_state

        for step in self._strategy.steps:
            if isinstance(step, BuiltinModuleLookup):
                return step.toggle_state
        return self._toggle_state if self._toggle_state is not None else _default_toggle_state

    def _adopt_client_toggle_state(self) -> ToggleState:
        """Reuse explicit state or isolate the standalone executor's legacy fallback."""
        from apcore.sys_modules.control import _default_toggle_state

        with self._toggle_state_lock:
            current = self.toggle_state
            if current is not _default_toggle_state:
                return current
            isolated = current._copy()
            steps = list(self._strategy.steps)
            for index, step in enumerate(steps):
                if isinstance(step, BuiltinModuleLookup):
                    lookup = copy(step)
                    lookup._set_toggle_state(isolated)
                    steps[index] = lookup
            self._toggle_state = isolated
            self._strategy.steps = steps
            return isolated

    @property
    def middlewares(self) -> list[Middleware]:
        """Return a copy of the current middleware list."""
        return self._middleware_manager.snapshot()

    def _bind_providers(self, strategy: ExecutionStrategy) -> ExecutionStrategy:
        """Bind this executor's providers into a private strategy and gate steps.

        PROTOCOL_SPEC §6.6.5.5 (D-129). Gates are located by TYPE, never by step
        name (§6.6.5.2), exactly as :meth:`governance_state` recognises them. A
        provider the executor holds replaces the one the step holds; a provider
        the executor was not given leaves the step's own provider untouched, so a
        pre-built strategy that carries its own ACL keeps enforcing it.

        Shared strategy templates keep their own providers and custom hook state.
        """
        from apcore.builtin_steps import BuiltinACLCheck, BuiltinApprovalGate

        bound = copy(strategy)
        bound.steps = [
            copy(step) if isinstance(step, (BuiltinACLCheck, BuiltinApprovalGate)) else step for step in strategy.steps
        ]
        bound.step_middlewares = list(strategy.step_middlewares)
        bound._rebuild_index()
        for step in bound.steps:
            if self._event_emitter_configured and isinstance(step, (BuiltinACLCheck, BuiltinApprovalGate)):
                step.set_event_emitter(self._event_emitter)
            if isinstance(step, BuiltinACLCheck):
                if self._acl is not None:
                    step.set_acl(self._acl)
            elif isinstance(step, BuiltinApprovalGate):
                if self._approval_handler is not None:
                    step.set_handler(self._approval_handler)
                if self._policy is not None:
                    step.set_policy(self._policy)
        return bound

    def set_acl(self, acl: ACL) -> None:
        """Set the access control provider.

        Updates both the executor field and every built-in ACL gate step
        (:class:`BuiltinACLCheck`, located by type — PROTOCOL_SPEC §6.6.5.5) of
        the running strategy. A custom step named ``acl_check`` that is not the
        built-in gate is not touched.

        Args:
            acl: The ACL instance to use for access control enforcement.
        """
        from apcore.builtin_steps import BuiltinACLCheck

        self._acl = acl
        found = False
        for step in self._strategy.steps:
            if isinstance(step, BuiltinACLCheck):
                step.set_acl(acl)
                found = True
        if not found:
            _logger.warning(
                "set_acl() called but current strategy has no BuiltinACLCheck step — ACL will not be enforced"
            )

    def set_approval_handler(self, handler: ApprovalHandler) -> None:
        """Set the approval handler for Step 5 gate.

        Updates both the executor field and every built-in approval gate step
        (:class:`BuiltinApprovalGate`, located by type — PROTOCOL_SPEC §6.6.5.5)
        of the running strategy.

        Args:
            handler: The ApprovalHandler instance to use for approval enforcement.
        """
        from apcore.builtin_steps import BuiltinApprovalGate

        self._approval_handler = handler
        for step in self._strategy.steps:
            if isinstance(step, BuiltinApprovalGate):
                step.set_handler(handler)

    def set_event_emitter(self, event_emitter: Any | None) -> None:
        """Set the emitter for governance and post-stream validation events."""
        from apcore.builtin_steps import BuiltinACLCheck, BuiltinApprovalGate

        self._event_emitter = event_emitter
        self._event_emitter_configured = True
        for step in self._strategy.steps:
            if isinstance(step, (BuiltinACLCheck, BuiltinApprovalGate)):
                step.set_event_emitter(event_emitter)

    def set_policy(self, policy: ExecutionPolicy | None) -> None:
        """Set the execution-time governance policy for the Step 5 gate.

        Updates both the executor field and every built-in approval gate step
        (:class:`BuiltinApprovalGate`, located by type — PROTOCOL_SPEC §6.6.5.5)
        of the running strategy.

        Args:
            policy: The ExecutionPolicy to apply, or None to restore the defaults.
        """
        from apcore.builtin_steps import BuiltinApprovalGate

        self._policy = policy
        for step in self._strategy.steps:
            if isinstance(step, BuiltinApprovalGate):
                step.set_policy(policy)

    def set_policy_evaluator(self, evaluator: PolicyEvaluator | None) -> None:
        """Install the evaluator for future invocations, or remove it with None."""
        if evaluator is not None:
            if not isinstance(evaluator, PolicyEvaluator) or not callable(evaluator.evaluate):
                raise InvalidInputError("Policy evaluator must implement evaluate()")
            _bind_post_validation_policy(self._strategy, evaluator)
        self._policy_evaluator = evaluator

    def use(self, middleware: Middleware) -> Executor:
        """Add class-based middleware with duplicate detection and return self for chaining."""
        self._middleware_manager.use(middleware)
        return self

    def use_before(self, callback: Callable[..., Any]) -> Executor:
        """Wrap callback in BeforeMiddleware adapter and add it."""
        wrapped = BeforeMiddleware(callback)
        self._middleware_manager.use(wrapped, identity_key=f"apcore.before.{id(callback)}")
        return self

    def use_after(self, callback: Callable[..., Any]) -> Executor:
        """Wrap callback in AfterMiddleware adapter and add it."""
        wrapped = AfterMiddleware(callback)
        self._middleware_manager.use(wrapped, identity_key=f"apcore.after.{id(callback)}")
        return self

    def remove(self, middleware: Middleware) -> bool:
        """Remove middleware by identity. Returns True if found and removed."""
        return self._middleware_manager.remove(middleware)

    def close(self) -> None:
        """Release synchronous call resources.

        Safe to call multiple times. Later calls create new loops and workers
        as needed. In-flight calls release their resources when they finish.
        """
        self._sync_bridge.close()

    def __enter__(self) -> Executor:
        return self

    def __exit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.close()

    async def __aenter__(self) -> Executor:
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.close()

    def call(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
        *,
        timeout_ms: int | None = None,
    ) -> dict[str, Any]:
        """Execute a module through the execution pipeline.

        Sync wrapper around :meth:`call_async`. Each thread caches its own
        loop for synchronous calls. A call made inside a running loop uses a
        background worker, with a bound on concurrent workers.

        Args:
            module_id: The module to execute.
            inputs: Input data dict. None is treated as {}.
            context: Optional execution context. Auto-created if None.
            version_hint: Optional semver hint for version negotiation.
            timeout_ms: Optional per-call thread-bridge wait in milliseconds.
                A value of 0 disables the bridge wait limit.

        Returns:
            The module output dict, possibly modified by middleware.
        """
        bridge_wait = self._sync_bridge_wait(context, timeout_ms)
        return self._sync_bridge.run(
            self.call_async(module_id, inputs, context, version_hint),
            module_id,
            bridge_wait,
        )

    def _sync_bridge_wait(
        self,
        context: Context | None,
        timeout_ms: int | None,
        strategy: ExecutionStrategy | str | None = None,
    ) -> float | None:
        _validate_bridge_timeout(timeout_ms)
        configured = self._sync_bridge_timeout_ms if timeout_ms is None else timeout_ms
        if configured is not None:
            return configured / 1000.0 if configured else None

        deadline = context.global_deadline if context is not None else None
        if deadline is not None:
            remaining = max(0.0, deadline - time.time())
        else:
            selected = self._strategy if strategy is None else strategy
            # Infer a bound only from the actual context step when its budget
            # is known before dispatch. Custom or unresolved strategies may
            # start their deadline later or use a different budget entirely.
            if not isinstance(selected, ExecutionStrategy) or not selected.steps:
                return None
            first = selected.steps[0]
            if (
                type(first) is not BuiltinContextCreation
                or first.match_modules is not None
                or not first.pure
                or selected.step_middlewares
                or first.global_timeout_ms <= 0
            ):
                return None
            remaining = first.global_timeout_ms / 1000.0
        wait = remaining + _SYNC_BRIDGE_GRACE_SECONDS
        return wait if wait <= threading.TIMEOUT_MAX else None

    def validate(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        *,
        timeout_ms: int | None = None,
        version_hint: str | None = None,
    ) -> PreflightResult:
        """Non-destructive preflight check using pipeline dry_run mode.

        Runs all pure steps (context creation, call chain guard, module lookup,
        ACL, input validation). Steps with pure=False (approval, middleware,
        execute) are automatically skipped. User-added pure steps are included.

        Args:
            module_id: The module to validate against.
            inputs: Input data to validate. None is treated as {}.
            context: Optional context for call-chain checks.
            timeout_ms: Optional per-call thread-bridge wait in milliseconds.
            version_hint: Optional semver hint, resolved as in ``call()``.

        Returns:
            PreflightResult with per-check status.
        """
        bridge_wait = self._sync_bridge_wait(context, timeout_ms)
        return self._sync_bridge.run(
            self.validate_async(module_id, inputs, context, version_hint=version_hint),
            module_id,
            bridge_wait,
        )

    async def validate_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        *,
        version_hint: str | None = None,
    ) -> PreflightResult:
        """Run preflight checks and optional module hooks on the current event loop."""
        business_inputs = strip_approval_token(inputs)
        inputs = business_inputs if business_inputs is not None else {}

        checks: list[PreflightCheckResult] = []
        evaluator = self._policy_evaluator

        # Check 0: module_id format (before pipeline)
        try:
            self._validate_module_id(module_id)
            checks.append(PreflightCheckResult(check="module_id", passed=True))
        except InvalidInputError as e:
            checks.append(PreflightCheckResult(check="module_id", passed=False, error=e.to_dict()))

        # PROTOCOL_SPEC §"Contract: Executor binding to Context": bind self
        # to the Context before pipeline step 1 (also applies to dry-run
        # validation, since the pipeline expects a bound Context).
        # A-D-008: validate() MUST be non-throwing. A Context already bound to
        # a DIFFERENT executor raises ContextBindingError on bind; surface that
        # as a failed check rather than letting it escape (mirrors Rust).
        if context is None:
            context = Context.create()
        try:
            context.bind_executor(self)
        except ContextBindingError as e:
            checks.append(
                PreflightCheckResult(
                    check="executor_binding",
                    passed=False,
                    error=e.to_dict(),
                )
            )
            return PreflightResult(valid=False, checks=checks)

        # Run pipeline in dry_run mode — pure=False steps are skipped
        pipe_ctx = PipelineContext(
            module_id=module_id,
            inputs=inputs,
            context=context,
            dry_run=True,
            version_hint=version_hint,
            _policy_enabled=evaluator is not None,
        )

        trace = None
        try:
            strategy = self._strategy
            if evaluator is not None:
                strategy = _bind_post_validation_policy(strategy, evaluator)
            _, trace = await self._pipeline_engine.run(strategy, pipe_ctx)
        except PipelineAbortError as e:
            trace = e.pipeline_trace
        except PipelineStepError as e:
            # Unwrap to the original typed error for preflight reporting.
            # trace is intentionally not set here to avoid _trace_to_checks
            # adding a second, redundant failure entry for the same step.
            underlying_e: Exception = e.cause if isinstance(e.cause, Exception) else e
            error_dict: dict[str, Any] = {
                "code": type(underlying_e).__name__,
                "message": str(underlying_e),
            }
            to_dict_fn = getattr(underlying_e, "to_dict", None)
            if callable(to_dict_fn):
                produced = to_dict_fn()
                if isinstance(produced, dict):
                    error_dict = produced
            checks.append(
                PreflightCheckResult(
                    check=_preflight_check_for(underlying_e),
                    passed=False,
                    error=error_dict,
                )
            )
        except Exception as e:
            # Step raised an error (e.g., ModuleNotFoundError, ACLDeniedError)
            # Convert to a failed check using the error's own code/dict
            error_dict = {"code": type(e).__name__, "message": str(e)}
            to_dict_fn = getattr(e, "to_dict", None)
            if callable(to_dict_fn):
                produced = to_dict_fn()
                if isinstance(produced, dict):
                    error_dict = produced

            checks.append(
                PreflightCheckResult(
                    check=_preflight_check_for(e),
                    passed=False,
                    error=error_dict,
                )
            )

        # Convert pipeline trace to PreflightResult checks
        if trace is not None:
            checks.extend(_trace_to_checks(trace, pipe_ctx.preflight_errors))

        # Detect requires_approval (preflight reports, does not enforce).
        #
        # §7.9.5 binds this to the verdict the Step-5 gate will enforce, so it
        # MUST read the same governance source: the D-96 union of the live
        # instance and the registry's DECLARED annotations. Reading the instance
        # alone reported "no approval needed" for a requirement an operator
        # declared in a `*.binding.yaml` / `metadata=` source — and the gate,
        # now reading the union, would stop the call the preflight waved
        # through. That disagreement is the one thing this method exists to
        # prevent.
        requires_approval = bool(pipe_ctx._policy_requires_approval)
        if evaluator is None and pipe_ctx.module is not None:
            annotations = governance_union(
                getattr(pipe_ctx.module, "annotations", None),
                pipe_ctx.declared_annotations,
            )
            if isinstance(annotations, ModuleAnnotations):
                requires_approval = annotations.requires_approval
            elif isinstance(annotations, dict):
                requires_approval = bool(annotations.get("requires_approval", False))
            from apcore.builtin_steps import BuiltinApprovalGate

            policies = [
                step.policy
                for step in self._strategy.steps
                if isinstance(step, BuiltinApprovalGate)
                and (step.match_modules is None or _any_match(step.match_modules, module_id))
            ]
            if not policies and self._policy is not None:
                policies = [self._policy]
            if policies:
                # Evaluate the same policy providers as the invocation gate,
                # including policies attached to a prebuilt strategy. A host's
                # custom resolver must never see the resume credential.
                arguments = strip_approval_token(pipe_ctx.inputs)
                requires_approval = any(
                    policy.resolve(module_id, annotations, arguments=arguments, context=pipe_ctx.context).needs_approval
                    for policy in policies
                )

        # PROTOCOL_SPEC §7.9.5: report the GOVERNANCE-effective requirement —
        # the union of §6.9 rows 3-5 — and not merely the policy-effective one.
        # An ACL rule carrying `approval` (§6.1.6) is a third source, and the
        # union is also what §6.9 row 4 demands: a policy that clears the
        # module's annotation MUST NOT clear the ACL's requirement, so this OR
        # sits deliberately after the policy resolution above and not inside it.
        # Reporting only the policy-effective value would tell a caller no
        # approval is needed for a call the gate will stop.
        requires_approval = requires_approval or pipe_ctx.acl_approval_required

        # Module introspection requires an explicit ACL verdict when a provider
        # is wired. A skipped or interrupted gate produces no authorization.
        from apcore.builtin_steps import BuiltinACLCheck

        acl_enforced = any(isinstance(step, BuiltinACLCheck) and step.acl is not None for step in self._strategy.steps)
        acl_denied = (acl_enforced and not pipe_ctx._acl_allowed) or any(
            c.check == _ACL_CHECK and not c.passed for c in checks
        )
        can_disclose = not acl_denied and (evaluator is None or pipe_ctx._policy_requires_approval is not None)
        module = pipe_ctx.module if can_disclose else None
        disclosure_inputs, disclosure_context = inputs, pipe_ctx.context
        if evaluator is not None and module is not None:
            try:
                invocation = pipe_ctx._policy_invocation
                if invocation is None:
                    raise PolicyEvaluationError()
                disclosure_inputs, disclosure_context = invocation.preflight_snapshot(pipe_ctx)
            except PolicyEvaluationError as exc:
                checks.append(PreflightCheckResult(check=_preflight_check_for(exc), passed=False, error=exc.to_dict()))
                module = None

        def callback_inputs() -> tuple[dict[str, Any], Context[Any]]:
            if evaluator is None:
                return disclosure_inputs, disclosure_context
            callback_context = copy(disclosure_context)
            callback_context.identity = deepcopy(disclosure_context.identity)
            callback_context.call_chain = list(disclosure_context.call_chain)
            return deepcopy(disclosure_inputs), callback_context

        # Module-level preflight (optional)
        if module is not None and callable(getattr(module, "preflight", None)):
            try:
                preflight_warnings = module.preflight(*callback_inputs())
                if isinstance(preflight_warnings, list) and preflight_warnings:
                    checks.append(
                        PreflightCheckResult(
                            check=_MODULE_PREFLIGHT_CHECK,
                            passed=True,
                            warnings=preflight_warnings,
                        )
                    )
                else:
                    checks.append(PreflightCheckResult(check=_MODULE_PREFLIGHT_CHECK, passed=True))
            except Exception as exc:
                checks.append(
                    PreflightCheckResult(
                        check=_MODULE_PREFLIGHT_CHECK,
                        passed=True,
                        warnings=[f"preflight() raised {type(exc).__name__}: {exc}"],
                    )
                )

        # Module-level preview (optional, protocol-spec §5.6 / §12.8.5.1).
        # Predicted changes are an advisory signal — if preview() raises, we
        # surface the failure as a warning on the module_preview check rather
        # than failing validation. This mirrors preflight() exception semantics.
        predicted_changes: list[Change] = []
        if module is not None and callable(getattr(module, "preview", None)):
            try:
                raw = module.preview(*callback_inputs())
                # Support both sync and async preview() implementations.
                if inspect.isawaitable(raw):
                    raw = await raw
                if isinstance(raw, PreviewResult):
                    predicted_changes = list(raw.changes)
                    checks.append(PreflightCheckResult(check=_MODULE_PREVIEW_CHECK, passed=True))
            except Exception as exc:
                checks.append(
                    PreflightCheckResult(
                        check=_MODULE_PREVIEW_CHECK,
                        passed=True,
                        warnings=[f"preview() raised {type(exc).__name__}: {exc}"],
                    )
                )

        valid = all(c.passed for c in checks)
        return PreflightResult(
            valid=valid,
            checks=checks,
            requires_approval=requires_approval,
            predicted_changes=predicted_changes,
        )

    @staticmethod
    def _validate_module_id(module_id: str) -> None:
        """Validate module_id format at public entry points.

        The length bound is checked here and not only in the registry (D-75):
        ``Contract: Executor.call`` requires empty, over-length and malformed
        IDs alike to be rejected *before* the ``PipelineContext`` is built. A
        well-formed but over-length ID otherwise reached the registry lookup
        and came back as ``MODULE_NOT_FOUND`` instead of ``INVALID_MODULE_ID``.
        """
        if not module_id or not MODULE_ID_PATTERN.match(module_id):
            raise InvalidInputError(
                message=f"Invalid module ID: '{module_id}'. Must match pattern: {MODULE_ID_PATTERN.pattern}",
                code=ErrorCodes.INVALID_MODULE_ID,
            )
        if len(module_id) > MAX_MODULE_ID_LENGTH:
            raise InvalidInputError(
                message=f"Module ID exceeds maximum length of {MAX_MODULE_ID_LENGTH}: {len(module_id)}",
                code=ErrorCodes.INVALID_MODULE_ID,
            )

    def _translate_abort(self, abort: PipelineAbortError) -> ModuleError:
        """Translate PipelineAbortError into the appropriate ModuleError subclass.

        Dispatches on stable signals (``abort.step`` name and
        ``abort.abort_reason``) rather than free-form explanation text so a
        change in step-level wording does not silently break error
        translation.
        """
        explanation = abort.explanation or ""
        step = abort.step
        reason = abort.abort_reason

        # Honour typed abort_reason first (new in v0.20). Older call paths
        # still default to AbortReason.OTHER and fall through to step-based
        # dispatch below.
        if reason is AbortReason.MODULE_TIMEOUT:
            return ModuleTimeoutError(module_id="", timeout_ms=0)
        if reason is AbortReason.MODULE_CANCELLED:
            from apcore.cancel import ExecutionCancelledError

            return ExecutionCancelledError()

        if step == "module_lookup":
            # Explanation format: "Module 'id' not found" — extract the id.
            return ModuleNotFoundError(module_id=explanation.split(": ")[-1] if ": " in explanation else "")
        if step == "acl_check":
            # Explanation format: "Access denied: {caller} -> {target}"
            caller_id = ""
            target_id = ""
            if " -> " in explanation:
                parts = explanation.split(": ", 1)[-1]
                pair = parts.split(" -> ", 1)
                if len(pair) == 2:
                    caller_id, target_id = pair[0].strip(), pair[1].strip()
            return ACLDeniedError(caller_id=caller_id, target_id=target_id)
        # Note: there is no `approval_gate` branch here. BuiltinApprovalGate
        # raises typed Approval{Denied,Timeout,Pending}Error subclasses
        # *directly* (see builtin_steps.BuiltinApprovalGate.execute), so the
        # error never reaches this translation path. Custom user-supplied
        # approval steps MUST follow the same contract: raise the typed
        # Approval*Error subclass with a real ApprovalResult — do NOT abort
        # via StepResult(action="abort", explanation="...rejected...").
        if step in ("input_validation", "output_validation"):
            return SchemaValidationError(message=explanation)

        # Fallback: return as ModuleError
        return ModuleError(code="PIPELINE_ABORT", message=explanation)

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        """Async module execution — delegates to PipelineEngine.

        Args:
            module_id: The module to execute.
            inputs: Input data dict. None is treated as {}.
            context: Optional execution context. Auto-created if None.
            version_hint: Optional semver hint for version negotiation.

        Returns:
            The module output dict, possibly modified by middleware.
        """
        output, _ = await self._run_call(module_id, inputs, context, version_hint)
        return output

    async def _run_call(
        self,
        module_id: str,
        inputs: dict[str, Any] | None,
        context: Any | None,
        version_hint: str | None,
        strategy: ExecutionStrategy | str | None = None,
    ) -> tuple[dict[str, Any], PipelineTrace]:
        self._validate_module_id(module_id)
        effective_strategy = self._effective_strategy(strategy)
        evaluator = self._policy_evaluator
        if evaluator is not None:
            effective_strategy = _bind_post_validation_policy(effective_strategy, evaluator)
        if context is None:
            context = Context.create()
        context.bind_executor(self)

        invocation_deadline: float | None = None
        attempted_steps = []
        started = time.monotonic()
        while True:
            async with AsyncExitStack() as execution_scope:
                # Every retry derives from the original caller. Context data is
                # shared across attempts, while caller identity and depth stay fixed.
                pipe_ctx = PipelineContext(
                    module_id=module_id,
                    inputs=inputs or {},
                    context=context,
                    strategy=effective_strategy,
                    version_hint=version_hint,
                    invocation_deadline=invocation_deadline,
                    execution_scope=execution_scope,
                    _policy_enabled=evaluator is not None,
                )
                try:
                    output, trace = await self._pipeline_engine.run(effective_strategy, pipe_ctx)
                except PipelineAbortError as exc:
                    raise self._translate_abort(exc) from exc
                except Exception as exc:
                    recovery = await self._recover_from_call_error(exc, pipe_ctx, module_id)
                    if isinstance(recovery, RetrySignal):
                        inputs = recovery.inputs
                        invocation_deadline = pipe_ctx.invocation_deadline
                        if pipe_ctx.trace is not None:
                            attempted_steps.extend(pipe_ctx.trace.steps)
                        continue
                    output = recovery
                    trace = pipe_ctx.trace or PipelineTrace(module_id=module_id, strategy_name=effective_strategy.name)
            if attempted_steps:
                trace.steps[:0] = attempted_steps
                trace.total_duration_ms = (time.monotonic() - started) * 1000
            return output, trace

    @staticmethod
    def _unwrap_cancellation(exc: Exception) -> "ExecutionCancelledError | None":
        """Return the ExecutionCancelledError ``exc`` is/wraps, else None.

        D-20: cancellation must propagate ahead of on_error recovery. The
        typed cancellation may be wrapped twice on the way out of the engine:
        ``PipelineEngine.run`` wraps the step's failure in a
        ``PipelineStepError`` (cause), and middleware machinery may wrap it in
        a ``MiddlewareChainError`` (original). Peel both layers before
        checking so every entry point (call_async, stream, call_with_trace)
        detects the cancellation regardless of how deeply it is wrapped.
        """
        unwrapped = exc
        if isinstance(unwrapped, PipelineStepError) and isinstance(unwrapped.cause, Exception):
            unwrapped = unwrapped.cause
        if isinstance(unwrapped, MiddlewareChainError):
            unwrapped = unwrapped.original
        return unwrapped if isinstance(unwrapped, ExecutionCancelledError) else None

    async def _recover_from_call_error(
        self,
        exc: Exception,
        pipe_ctx: PipelineContext,
        module_id: str,
    ) -> "dict[str, Any] | RetrySignal":
        """Run A11 error propagation + middleware on_error recovery.

        Returns:
            - ``dict`` — a recovery output from the first handler that provided one.
            - :class:`RetrySignal` — a handler asked for a retry; the caller
              must re-run the pipeline.
            - Never returns ``None``: if no handler recovered, the unwrapped
              original error is raised (D-22 / A-D-EXEC-005).

        D-22 — Error Unwrap Rule: when middleware machinery wraps a
        domain-typed error (e.g. ``ApprovalDeniedError``) in a
        ``MiddlewareChainError`` for diagnostics, the executor MUST unwrap
        the wrapper and propagate ``MiddlewareChainError.original`` to
        ``propagate_error`` and to the caller. Replacing the cause with a
        generic ``ModuleExecuteError`` collapses callers' ability to
        dispatch on the typed cause (e.g. MCP/A2A bridges keying on
        ``APPROVAL_DENIED`` vs ``MODULE_EXECUTE_ERROR``). This mirrors the
        TypeScript and Rust SDKs.

        D-20 — Cancellation short-circuit: an ExecutionCancelledError raised
        mid-pipeline MUST bypass on_error recovery entirely. The engine wraps
        it in a PipelineStepError (and machinery may further wrap it in a
        MiddlewareChainError), so re-check the unwrapped cause here as a
        backstop. This guarantees ALL entry points — call_async, stream, and
        call_with_trace — honour the bypass even if their caller did not
        unwrap first. Idempotent for callers (call_async/stream) that already
        short-circuited: they pass in the typed cause, not the wrapper.
        """
        cancelled = self._unwrap_cancellation(exc)
        if cancelled is not None:
            raise cancelled
        recoverable = not pipe_ctx._policy_pending
        if isinstance(exc, PipelineStepError):
            recoverable = recoverable and exc.recoverable
            if isinstance(exc.cause, Exception):
                exc = exc.cause
        ctx_obj = pipe_ctx.context
        # Unwrap MiddlewareChainError BEFORE propagation so the wrapped
        # typed cause is what middleware on_error handlers and the final
        # caller observe — not the chain-machinery wrapper.
        original = exc.original if isinstance(exc, MiddlewareChainError) else exc
        wrapped = propagate_error(original, module_id, ctx_obj) if ctx_obj else original
        executed_mw = pipe_ctx.executed_middlewares
        if recoverable and executed_mw:
            recovery = await self._middleware_manager.execute_on_error_async(
                module_id, pipe_ctx.inputs, wrapped, ctx_obj, executed_mw
            )
            if recovery is not None:
                return recovery
        raise wrapped from original

    def _merge_depth_cap(self) -> int:
        """The streaming deep-merge depth cap for this executor (PROTOCOL_SPEC §5)."""
        return _resolve_merge_depth(self._config)

    async def stream(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> AsyncGenerator[dict[str, Any], None]:
        """Async generator that streams module output chunks.

        Phase 1: Pipeline runs steps 1-7 (context, guard, lookup, ACL, approval,
        middleware_before, input_validation). Step 8 (execute) sets up the stream
        or falls back to single-chunk mode.
        Phase 2: After streaming, runs output_validation + middleware_after on
        accumulated output.

        Args:
            module_id: The module to execute.
            inputs: Input data dict. None is treated as {}.
            context: Optional execution context. Auto-created if None.
            version_hint: Optional semver hint for version negotiation.

        Yields:
            Dict chunks from the module's stream() or a single call_async() result.
        """
        execution = self._open_stream(module_id, inputs, context, version_hint, capture=False)
        try:
            async with aclosing(execution):
                async for chunk in execution.chunks:
                    yield chunk
        except GeneratorExit:
            # Keep the ordinary stream's explicit close contract. Timeout
            # and cancellation still report before detached cleanup finishes.
            await execution._wait_for_cleanup()
            raise

    def stream_with_completion(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> StreamExecution:
        """Open one lazy stream with a separately observable completion."""
        return self._open_stream(module_id, inputs, context, version_hint, capture=True)

    def _open_stream(
        self,
        module_id: str,
        inputs: dict[str, Any] | None,
        context: Context | None,
        version_hint: str | None,
        *,
        capture: bool,
    ) -> StreamExecution:

        def source(observation: _StreamObservation, scope: AsyncExitStack) -> AsyncGenerator[dict[str, Any], None]:
            return self._stream(module_id, inputs, context, version_hint, scope, observation)

        def timeout_error(pipeline: PipelineContext | None) -> ModuleError:
            if pipeline is not None and pipeline._policy_evaluation_active:
                token = pipeline.context.cancel_token
                if token is not None and token.is_cancelled:
                    return ExecutionCancelledError()
                return PolicyEvaluationError()
            return self._stream_timeout_error(module_id, pipeline)

        return StreamExecution(
            source, self._strategy.step_names(), timeout_error=timeout_error, captures_completion=capture
        )

    async def _stream(
        self,
        module_id: str,
        inputs: dict[str, Any] | None,
        context: Context | None,
        version_hint: str | None,
        execution_scope: AsyncExitStack,
        observation: _StreamObservation,
    ) -> AsyncGenerator[dict[str, Any], None]:
        self._validate_module_id(module_id)
        evaluator = self._policy_evaluator
        strategy = self._strategy
        if evaluator is not None:
            strategy = _bind_post_validation_policy(strategy, evaluator)

        # PROTOCOL_SPEC §"Contract: Executor binding to Context": bind self
        # to the Context before pipeline step 1.
        if context is None:
            context = Context.create()
        context.bind_executor(self)
        context = copy(context)
        observation.cancel_token = CancelToken(parent=context.cancel_token)
        context.cancel_token = observation.cancel_token

        pipe_ctx = PipelineContext(
            module_id=module_id,
            inputs=inputs or {},
            context=context,
            version_hint=version_hint,
            stream=True,
            execution_scope=execution_scope,
            _stream_observation=observation,
            _policy_enabled=evaluator is not None,
        )
        observation.context = pipe_ctx
        observation.steps = strategy.step_names()
        observation.monitor(context)
        self._check_stream_context(pipe_ctx)

        # Phase 1: Run pipeline up to execute step.
        # BuiltinExecute detects ctx.stream=True and checks for module.stream().
        try:
            await self._pipeline_engine.run(strategy, pipe_ctx)
        except PipelineAbortError as e:
            raise self._translate_abort(e) from e
        except PipelineStepError as e:
            underlying = e.cause if isinstance(e.cause, Exception) else e
            # D-20: cancellation short-circuits BEFORE on_error (mirror call_async).
            cancelled = self._unwrap_cancellation(underlying)
            if cancelled is not None:
                raise cancelled
            recovery = await self._recover_from_call_error(e, pipe_ctx, module_id)
            if isinstance(recovery, RetrySignal):
                _logger.warning(
                    "Retry requested during stream for '%s' — ignored; re-raising",
                    module_id,
                )
                raise underlying
            self._check_stream_context(pipe_ctx)
            if observation.captures_completion:
                observation.record_execution_error(underlying)
            yield recovery
            return
        except ExecutionCancelledError:
            raise
        except Exception as exc:
            recovery = await self._recover_from_call_error(exc, pipe_ctx, module_id)
            if isinstance(recovery, RetrySignal):
                # Retry is not meaningful once a stream has been entered: the
                # first failure-aware caller of stream() could only be mid-
                # stream, so we translate a retry request back into the
                # original error rather than silently re-running.
                _logger.warning(
                    "Retry requested during stream for '%s' — ignored; re-raising",
                    module_id,
                )
                raise exc
            self._check_stream_context(pipe_ctx)
            if observation.captures_completion:
                observation.record_execution_error(exc)
            yield recovery
            return

        # If module has no stream(), pipeline already executed and set ctx.output
        self._check_stream_context(pipe_ctx)
        if pipe_ctx.output_stream is None:
            observation.capture_fallback_output(pipe_ctx.output)
            yield pipe_ctx.output or {}
            return

        # Phase 2: Iterate stream, accumulate chunks
        accumulated: dict[str, Any] = {}
        # PROTOCOL_SPEC §5: resolved ONCE per stream, not per chunk. A
        # configuration mutated mid-stream — `system.control.update_config`
        # can do exactly that — must not change the cap of a stream already
        # in flight. This used to be called inside the chunk loop, which
        # contradicted the guarantee its own commit message stated.
        merge_depth_cap = self._merge_depth_cap()
        try:
            async for idx, chunk in self._stream_chunks(pipe_ctx):
                chunk = normalize_module_output(chunk)
                # Cross-SDK contract (Rust ``deep_merge_chunks_checked``):
                # a stream chunk is valid iff it is a JSON object (dict).
                # Reject the first non-object chunk BEFORE merge and BEFORE
                # yield so the invalid chunk is never delivered to the
                # consumer; deep_merge can only accumulate objects.
                if not isinstance(chunk, dict):
                    actual_type = _json_type_name(chunk)
                    raise InvalidInputError(
                        message=(
                            f"Streaming chunk at index {idx} is not a JSON object "
                            f"(got {actual_type}); chunks must be objects so "
                            f"deep_merge can accumulate them."
                        ),
                        code="GENERAL_INVALID_INPUT",
                        details={
                            "code": "STREAM_CHUNK_NOT_OBJECT",
                            "chunk_index": idx,
                            "actual_type": actual_type,
                        },
                    )
                _deep_merge(
                    accumulated,
                    deepcopy(chunk) if observation.captures_completion else chunk,
                    max_depth=merge_depth_cap,
                )
                yield chunk
        except ExecutionCancelledError:
            raise
        except Exception as exc:
            recovery = await self._recover_from_call_error(exc, pipe_ctx, module_id)
            if isinstance(recovery, RetrySignal):
                # Retry is not meaningful once a stream has been entered: the
                # first failure-aware caller of stream() could only be mid-
                # stream, so we translate a retry request back into the
                # original error rather than silently re-running.
                _logger.warning(
                    "Retry requested during stream for '%s' — ignored; re-raising",
                    module_id,
                )
                raise exc
            self._check_stream_context(pipe_ctx)
            if observation.captures_completion:
                observation.record_execution_error(exc)
            yield recovery
            return

        # Phase 3: Output validation + middleware_after on accumulated result
        self._check_stream_context(pipe_ctx)
        pipe_ctx.output = accumulated
        observation.start_post_processing()
        post_steps = [s for s in strategy.steps if s.name in ("output_validation", "middleware_after", "return_result")]
        if post_steps:
            # The post-stream sub-strategy is a slice of an already-validated
            # parent strategy; its ``requires`` keys (e.g. ``module``,
            # ``output``) are seeded by the streaming pre-flight, not by
            # earlier steps in this sub-strategy. Skip the strict §2.1
            # dependency check that would otherwise reject the slice.
            post_strategy = ExecutionStrategy("post_stream", post_steps, validate_dependencies=False)
            if observation.captures_completion:
                post_strategy.step_middlewares.extend(strategy.step_middlewares)
            try:
                await self._pipeline_engine.run(post_strategy, pipe_ctx)
            except Exception as post_exc:
                cancelled = self._unwrap_cancellation(post_exc)
                if cancelled is not None:
                    raise cancelled
                if observation.captures_completion:
                    observation.record_post_error(post_exc)
                # Non-fatal for the caller: chunks have already been yielded,
                # so a failed post-stream validation/middleware run cannot
                # retroactively change the observable output. Still emit an
                # observability event + WARNING log so the failure is visible
                # — unvalidated output that reached a consumer is worth
                # investigating even if it can't be un-sent.
                _logger.warning(
                    "Post-stream validation/middleware failed for module %s",
                    module_id,
                    exc_info=True,
                )
                self._emit_post_stream_failure(module_id, pipe_ctx.context, post_exc)
                return
        self._check_stream_context(pipe_ctx)
        observation.capture_fallback_output(accumulated)

    async def _stream_chunks(self, ctx: PipelineContext) -> AsyncIterator[tuple[int, Any]]:
        """Read each chunk within the call deadline and check the linked token."""
        source = ctx.output_stream
        if source is None:
            return
        index = 0
        while True:
            self._check_stream_context(ctx)
            try:
                chunk = await anext(source)
            except StopAsyncIteration:
                self._check_stream_context(ctx)
                return
            self._check_stream_context(ctx)
            yield index, chunk
            index += 1

    def _check_stream_context(self, ctx: PipelineContext) -> None:
        observation = ctx._stream_observation
        if observation is not None and observation.timeout_error is not None:
            raise observation.timeout_error
        token = getattr(ctx.context, "cancel_token", None)
        if token is not None:
            token.check()
        deadline = getattr(ctx.context, "global_deadline", None)
        if deadline is not None and time.time() >= deadline:
            raise self._stream_timeout_error(ctx.module_id, ctx)

    def _stream_timeout_error(self, module_id: str, ctx: PipelineContext | None) -> ModuleTimeoutError:
        return ModuleTimeoutError(
            module_id=module_id, timeout_ms=self._global_timeout, retryable=_is_idempotent(ctx.module if ctx else None)
        )

    def _emit_post_stream_failure(
        self,
        module_id: str,
        context: Context | None,
        exc: BaseException,
    ) -> None:
        """Emit an ApCoreEvent for post-stream validation/middleware failure.

        Also annotates the active tracing span (if any) with status='error'
        and an exception attribute so trace exporters surface the failure.
        """
        if self._event_emitter is not None:
            from datetime import datetime, timezone

            from apcore.events.emitter import ApCoreEvent

            event = ApCoreEvent(
                event_type="apcore.stream.post_validation_failed",
                module_id=module_id,
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="error",
                data={
                    "error_type": type(exc).__name__,
                    "message": str(exc),
                    "trace_id": context.trace_id if context is not None else None,
                },
            )
            self._event_emitter.emit(event)

        if context is not None:
            active_span = _current_trace_span(context)
            if active_span is not None:
                setattr(active_span, "status", "error")
                attrs = getattr(active_span, "attributes", None)
                if isinstance(attrs, dict):
                    attrs["exception"] = f"{type(exc).__name__}: {exc}"

    # _execute_async removed in v0.17 (replaced by BuiltinExecute pipeline step)

    # -------------------------------------------------------------------------
    # Strategy resolution
    # -------------------------------------------------------------------------

    @staticmethod
    def _resolve_strategy_name(name: str, **kwargs: Any) -> ExecutionStrategy:
        """Resolve a strategy name to an ExecutionStrategy instance.

        Checks preset names first, then the class-level registered strategies.

        Args:
            name: Strategy name ("standard", "internal", "testing", "performance",
                "minimal", or a previously registered name).
            **kwargs: Forwarded to preset builder functions.

        Returns:
            The resolved ExecutionStrategy.

        Raises:
            StrategyNotFoundError: If the name is not recognized.
        """
        from apcore.builtin_steps import (
            build_internal_strategy,
            build_minimal_strategy,
            build_performance_strategy,
            build_standard_strategy,
            build_testing_strategy,
        )

        preset_builders: dict[str, Any] = {
            "standard": build_standard_strategy,
            "internal": build_internal_strategy,
            "testing": build_testing_strategy,
            "performance": build_performance_strategy,
            "minimal": build_minimal_strategy,
        }

        if name in preset_builders:
            return preset_builders[name](**kwargs)

        if name in Executor._registered_strategies:
            return Executor._registered_strategies[name]

        raise StrategyNotFoundError(
            message=f"Strategy '{name}' not found. "
            f"Available: {sorted(set(list(preset_builders) + list(Executor._registered_strategies)))}"
        )

    # -------------------------------------------------------------------------
    # call_with_trace / call_async_with_trace
    # -------------------------------------------------------------------------

    def call_with_trace(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any | None = None,
        *,
        version_hint: str | None = None,
        strategy: ExecutionStrategy | str | None = None,
        timeout_ms: int | None = None,
    ) -> tuple[dict[str, Any], PipelineTrace]:
        """Sync call that returns (result, trace).

        Sync wrapper around :meth:`call_async_with_trace`. Runs the module
        through the pipeline engine and returns both the output and the full
        pipeline trace for introspection.

        Args:
            module_id: The module to execute.
            inputs: Input data dict. None is treated as {}.
            context: Optional execution context.
            version_hint: Optional semver hint for version negotiation.
            strategy: Override strategy for this call.
            timeout_ms: Optional per-call thread-bridge wait in milliseconds.

        Returns:
            A tuple of (result dict, PipelineTrace).
        """
        bridge_wait = self._sync_bridge_wait(context, timeout_ms, strategy)
        return self._sync_bridge.run(
            self.call_async_with_trace(module_id, inputs, context, version_hint=version_hint, strategy=strategy),
            module_id,
            bridge_wait,
        )

    async def call_async_with_trace(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any | None = None,
        *,
        version_hint: str | None = None,
        strategy: ExecutionStrategy | str | None = None,
    ) -> tuple[dict[str, Any], PipelineTrace]:
        """Async call that returns (result, trace).

        Runs the module through the pipeline engine and returns both the
        output and the full pipeline trace for introspection. Errors flow
        through the same A11 propagation + middleware ``on_error`` recovery
        path as :meth:`call_async`; if a middleware recovers, the recovery
        dict is returned alongside the trace captured up to the failure.

        Args:
            module_id: The module to execute.
            inputs: Input data dict. None is treated as {}.
            context: Optional execution context.
            strategy: Override strategy for this call.

        Returns:
            A tuple of (result dict, PipelineTrace).
        """
        return await self._run_call(module_id, inputs, context, version_hint, strategy)

    def _effective_strategy(
        self,
        strategy: ExecutionStrategy | str | None,
    ) -> ExecutionStrategy:
        """Return the strategy to use for a call, resolving strings."""
        if strategy is None:
            return self._strategy
        if isinstance(strategy, str):
            resolved = self._resolve_strategy_name(
                strategy,
                registry=self._registry,
                config=self._config,
                acl=self._acl,
                approval_handler=self._approval_handler,
                policy=self._policy,
                event_emitter=self._event_emitter,
                middlewares=self._middleware_manager.snapshot(),
                toggle_state=self.toggle_state,
            )
        else:
            resolved = strategy
        # §6.6.5.5 (D-129): a per-call strategy is still a way this executor gets
        # a strategy, so its gates receive the executor's providers too.
        return self._bind_providers(resolved)

    # -------------------------------------------------------------------------
    # Introspection
    # -------------------------------------------------------------------------

    @classmethod
    def register_strategy(cls, name: str, strategy: ExecutionStrategy) -> None:
        """Register a named strategy for resolution by string name.

        Args:
            name: The name to register under.
            strategy: The ExecutionStrategy instance.
        """
        cls._registered_strategies[name] = strategy

    def list_strategies(self) -> list[StrategyInfo]:
        """Return StrategyInfo for the current strategy and all registered strategies.

        Returns:
            A list of StrategyInfo, starting with the current strategy.
        """
        seen: set[str] = set()
        result: list[StrategyInfo] = []

        # Current strategy first
        info = self._strategy.info()
        result.append(info)
        seen.add(info.name)

        # Registered strategies
        for name, strat in sorted(self._registered_strategies.items()):
            if name not in seen:
                result.append(strat.info())
                seen.add(name)

        return result

    @property
    def current_strategy(self) -> ExecutionStrategy:
        """Return the current execution strategy."""
        return self._strategy

    def _module_governance_annotations(self, module_id: str) -> ModuleAnnotations | None:
        return governance_union(
            getattr(self._registry.get(module_id), "annotations", None),
            self._registry.get_declared_annotations(module_id),
        )

    def ungated_approval_modules(self) -> list[str]:
        """List registered module IDs with an approval declaration the built-in gate would skip.

        Use after registration to report the default strategy's approval posture.
        The report includes hidden modules and evaluates the policy without a call site.
        Caller-specific ACL requirements are evaluated during each invocation.
        """
        from apcore.builtin_steps import BuiltinApprovalGate

        enforcing_gates = [
            step
            for step in self._strategy.steps
            if isinstance(step, BuiltinApprovalGate)
            and (step.handler is not None or step.policy.strict)
        ]
        ungated: list[str] = []
        for module_id in self._registry.list(visibility=["public", "hidden"]):
            annotations = self._module_governance_annotations(module_id)
            if annotations is None or not annotations.requires_approval:
                continue
            if not any(
                gate.policy.resolve(module_id, annotations).needs_approval
                for gate in enforcing_gates
            ):
                ungated.append(module_id)
        return ungated

    def governance_state(self) -> GovernanceState:
        """Return what is actually gating this executor (PROTOCOL_SPEC 6.6.5).

        A **pure read**: it never enforces, warns, throws or mutates. What to do
        about an unprotected control surface belongs to the caller -- a
        serve-time adapter may warn or refuse, a test may assert, a health
        endpoint may report. Putting the reaction here would make it
        unavoidable and untestable.

        The value is computed fresh on every call, so attaching an ACL or
        swapping the strategy is visible in the next one.

        Returns:
            A :class:`~apcore.pipeline.GovernanceState` of eight observations
            plus one derived flag.
        """
        from apcore.builtin_steps import (
            BuiltinACLCheck,
            BuiltinApprovalGate,
            _requires_approval,
        )

        steps = self._strategy.steps
        # Gate detection is by TYPE, never by step name (PROTOCOL_SPEC 6.6.5.2).
        # `StrategyInfo` carries names only, and a custom step named
        # `acl_check` that never consults an ACL would satisfy a name test --
        # producing `builtin_acl_gate_wired=True` for a gate that is not there.
        # That is the one direction this accessor must never fail in.
        acl_gate_wired = any(isinstance(step, BuiltinACLCheck) for step in steps)
        approval_gate_wired = any(isinstance(step, BuiltinApprovalGate) for step in steps)

        # `visibility` must include hidden: the accessor reports what is
        # REGISTERED, and `list()` defaults to public-only. A control module
        # registered with `discoverable: false` is still callable by ID, so
        # omitting it would under-report the write surface.
        all_ids = self._registry.list(visibility=["public", "hidden"])
        control_ids = [mid for mid in all_ids if mid.startswith("system.control.")]
        read_modules_registered = any(
            mid.startswith(("system.health.", "system.usage.", "system.manifest.")) for mid in all_ids
        )

        # Read governance through the SAME union the approval gate uses (D-96).
        # A second, narrower reading here would be a second way for the accessor
        # to disagree with the pipeline it describes: a control module whose
        # requirement is declared in a metadata source, not on the instance, is
        # gated by the pipeline and was reported here as ungated — which a
        # serve-time adapter may act on by refusing to start.
        all_control_require_approval = bool(control_ids) and all(
            _requires_approval(self._module_governance_annotations(mid)) for mid in control_ids
        )

        # PROTOCOL_SPEC §6.6.5.5 requirement 3 (D-129): report what the RUNNING
        # gate step holds when that gate is wired — a gate may enforce a provider
        # the executor never saw — and fall back to the executor's own provider
        # only when no built-in gate is wired ("configured and not wired").
        acl_gates = [step for step in steps if isinstance(step, BuiltinACLCheck)]
        approval_gates = [step for step in steps if isinstance(step, BuiltinApprovalGate)]
        if acl_gates:
            acl_configured = any(gate.acl is not None for gate in acl_gates)
        else:
            acl_configured = self._acl is not None
        if approval_gates:
            handler_configured = any(gate.handler is not None for gate in approval_gates)
            policy_strict = any(gate.policy.strict for gate in approval_gates)
        else:
            handler_configured = self._approval_handler is not None
            policy_strict = self._policy is not None and bool(getattr(self._policy, "strict", False))

        # PROTOCOL_SPEC 6.6.5.1. The approval conjunct carries
        # `all_control_modules_require_approval` because `approval_gate` is
        # per-module conditional while `acl_check` is not (6.6.5.1.1).
        unprotected = (
            bool(control_ids)
            and not (acl_configured and acl_gate_wired)
            and not (approval_gate_wired and all_control_require_approval and (handler_configured or policy_strict))
        )

        return GovernanceState(
            control_modules_registered=bool(control_ids),
            read_modules_registered=read_modules_registered,
            acl_configured=acl_configured,
            builtin_acl_gate_wired=acl_gate_wired,
            approval_handler_configured=handler_configured,
            builtin_approval_gate_wired=approval_gate_wired,
            policy_strict=policy_strict,
            all_control_modules_require_approval=all_control_require_approval,
            unprotected_control_surface=unprotected,
        )

    def describe_pipeline(self) -> StrategyInfo:
        """Return an AI-introspectable description of the current pipeline.

        Returns:
            A StrategyInfo dataclass with name, step_count, step_names, and
            a human-readable description string.
        """
        return self._strategy.info()
