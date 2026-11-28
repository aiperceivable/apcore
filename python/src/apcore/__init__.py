# ruff: noqa: F401
"""apcore - Schema-driven module standard."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncGenerator
from contextlib import aclosing
from importlib import import_module
from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as _get_version
from typing import Any

# Core
from apcore.approval import (
    AlwaysDenyHandler,
    ApprovalHandler,
    ApprovalRequest,
    ApprovalResult,
    AutoApproveHandler,
    CallbackApprovalHandler,
    ScopedApprovalHandler,
)
from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.capabilities import AccessHint, CallableDefinition, CapabilityPage, CapabilitySearchProvider, ModuleSummary
from apcore.context import Context, ContextFactory, GovernanceProjection, Identity
from apcore.context_key import ContextKey
from apcore.context_keys import (
    LOGGING_START,
    METRICS_STARTS,
    REDACTED_OUTPUT,
    RETRY_COUNT_BASE,
    TRACING_SAMPLED,
    TRACING_SPANS,
)
from apcore.registry import (
    DEFAULT_MODULE_VERSION,
    EPHEMERAL_NAMESPACE_PREFIX,
    MAX_MODULE_ID_LENGTH,
    MODULE_ID_PATTERN,
    REGISTRY_EVENTS,
    RESERVED_WORDS,
    Registry,
    class_name_to_segment,
    discover_multi_class,
)
from apcore.registry.conflicts import (
    ConflictResult as ConflictResult,
    ConflictSeverity as ConflictSeverity,
    ConflictType as ConflictType,
    detect_id_conflicts as detect_id_conflicts,
)
from apcore.registry.registry import Discoverer, ModuleValidator
from apcore.client import APCore
from apcore.bound_module import AsyncBoundModule, BoundModule
from apcore.lifecycle import (
    DrainAbortedContext,
    DrainContext,
    LifecycleError,
    LifecycleModuleReport,
    LifecycleOperation,
    LifecycleReport,
    ReloadOptions,
    ShutdownOptions,
    UnregisterOptions,
)
from apcore.registry.types import DependencyInfo, DiscoveredModule, ModuleDescriptor
from apcore.events.emitter import EventSubscriber
from apcore.executor import Executor
from apcore.utils.redaction import REDACTED_VALUE, redact_sensitive

# Module types
from apcore.module import (
    Change,
    DEFAULT_ANNOTATIONS,
    Module,
    ModuleAnnotations,
    ModuleExample,
    PreflightCheckResult,
    PreflightResult,
    PreviewResult,
    ValidationResult,
)

# Error Formatter
from apcore.error_formatter import ErrorFormatter, ErrorFormatterRegistry
from apcore.streaming import (
    StreamCompletion,
    StreamCompletionReason,
    StreamExecution,
    StreamingModule,
    StreamStepOutcome,
)

# Errors
from apcore.errors import (
    ACLDeniedError,
    StreamingInterfaceError,
    ACLRuleError,
    ApprovalDeniedError,
    ApprovalError,
    ApprovalPendingError,
    ApprovalTimeoutError,
    BindingCallableNotFoundError,
    BindingFileInvalidError,
    BindingInvalidTargetError,
    BindingModuleNotFoundError,
    BindingNotCallableError,
    BindingSchemaInferenceFailedError,
    BindingSchemaMissingError,
    BindingSchemaModeConflictError,
    BindingStrictSchemaIncompatibleError,
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    CircularDependencyError,
    CircuitBreakerOpenError,
    CircuitOpenError,
    ConfigBindError,
    ConfigEnvMapConflictError,
    ConfigEnvPrefixConflictError,
    ConfigError,
    ConfigMountError,
    ConfigNamespaceDuplicateError,
    ConfigNamespaceReservedError,
    ConfigNotFoundError,
    ContextBindingError,
    DependencyNotFoundError,
    DependencyVersionMismatchError,
    ErrorCodeCollisionError,
    ErrorCodeRegistry,
    ErrorCodes,
    ErrorFormatterDuplicateError,
    FRAMEWORK_ERROR_CODE_PREFIXES,
    FuncMissingReturnTypeError,
    FuncMissingTypeHintError,
    IdTooLongError,
    InternalError,
    InvalidInputError,
    InvalidParentIdError,
    InvalidSegmentError,
    ModuleDisabledError,
    ModuleIdConflictError,
    ModuleError,
    ModuleExecuteError,
    ModuleLoadError,
    ModuleNotFoundError,
    ModuleReloadConflictError,
    ModuleTimeoutError,
    PolicyDeniedError,
    PolicyEvaluationError,
    ReloadFailedError,
    SysModuleRegistrationError,
    SysModulesDisabledError,
    SchemaCircularRefError,
    SchemaMaxDepthExceededError,
    SchemaNotFoundError,
    SchemaParseError,
    SchemaValidationError,
    TaskLimitExceededError,
    TaskStoreError,
    VersionConstraintError,
)

# ACL
from apcore.acl import (
    ACL,
    AccessDecision,
    ACLRule,
    AuditEntry,
    ConditionOutcome,
    RuleValidationFinding,
    VisibilityDecision,
)

# Execution-time governance policy (apcore#76 RFC pilot)
from apcore.policy import (
    ExecutionPolicy,
    PolicyDecision,
    PolicyEvaluationRequest,
    PolicyEvaluator,
    PolicyRule,
    PolicyVerdict,
)

# Middleware
from apcore.middleware import (
    APCORE_KEY_PREFIX,
    EXT_KEY_PREFIX,
    AfterMiddleware,
    BeforeMiddleware,
    CircuitBreakerMiddleware,
    CircuitBreakerState,
    ErrorHistoryMiddleware,
    LoggingMiddleware,
    Middleware,
    MiddlewareChainError,
    MiddlewareManager,
    NamespaceCheck,
    PlatformNotifyMiddleware,
    RetryConfig,
    RetryMiddleware,
    RetrySignal,
    validate_context_key,
)

# Decorators
from apcore.decorator import FunctionModule
from apcore._docstrings import parse_docstring as parse_docstring

# Extensions
from apcore.extensions import ExtensionManager, ExtensionPoint

# Async tasks
from apcore.async_task import (
    AsyncTaskManager,
    BackoffStrategy,
    InMemoryTaskStore,
    RetryConfig as AsyncRetryConfig,
    RetryPolicy,
    TaskInfo,
    TaskStatus,
    TaskStore,
)

# Schema
from apcore.schema import (
    SchemaLoader as SchemaLoader,
    SchemaStrategy as SchemaStrategy,
    SchemaValidator as SchemaValidator,
    SchemaExporter as SchemaExporter,
    RefResolver as RefResolver,
    to_strict_schema as to_strict_schema,
)

# Version negotiation (A14)
from apcore.version import VersionIncompatibleError as VersionIncompatibleError
from apcore.version import negotiate_version as negotiate_version

# Utilities
from apcore.utils import match_pattern as match_pattern
from apcore.utils.call_chain import guard_call_chain as guard_call_chain
from apcore.utils.call_chain import DEFAULT_MAX_CALL_DEPTH as DEFAULT_MAX_CALL_DEPTH
from apcore.utils.call_chain import (
    DEFAULT_MAX_MODULE_REPEAT as DEFAULT_MAX_MODULE_REPEAT,
)
from apcore.utils.error_propagation import propagate_error as propagate_error
from apcore.utils.normalize import (
    CanonicalNameError as CanonicalNameError,
    CanonicalNameResult as CanonicalNameResult,
    canonicalize_name as canonicalize_name,
    normalize_to_canonical_id as normalize_to_canonical_id,
)
from apcore.utils.pattern import calculate_specificity as calculate_specificity

# NOTE (audit D1-003, 2026-05-28): the events-layer circuit-breaker types
# `CircuitBreakerWrapper` / `CircuitState` are intentionally NOT re-exported at
# the top level. TypeScript and Rust export them at the package/crate root, but
# Python deliberately keeps them under `apcore.events.circuit_breaker` to avoid
# resurrecting a top-level `CircuitState` name (the middleware enum of that name
# was renamed to `CircuitBreakerState`; a top-level `CircuitState` would be
# ambiguous). This divergence is enforced by
# tests/test_circuit_breaker_middleware.py::test_old_middleware_circuit_state_name_is_gone.

# Pipeline
from apcore.pipeline import (
    BaseStep,
    ConfigurationError,
    ExecutionStrategy,
    PipelineAbortError,
    PipelineContext,
    PipelineDependencyError,
    PipelineEngine,
    PipelineState,
    PipelineStepError,
    PipelineStepNotFoundError,
    PipelineTrace,
    Step,
    StepMiddleware,
    StepNameDuplicateError,
    StepNotFoundError,
    StepNotRemovableError,
    StepNotReplaceableError,
    StepResult,
    StepTrace,
    GovernanceState,
    StrategyInfo,
    StrategyNotFoundError,
)

# Pipeline Configuration
from apcore.pipeline_config import (
    build_strategy_from_config,
    register_step_type,
    registered_step_types,
    unregister_step_type,
)

# Pipeline Preset Builders (parity with apcore-typescript and apcore-rust)
from apcore.builtin_steps import (
    BuiltinACLCheck,
    BuiltinApprovalGate,
    BuiltinCallChainGuard,
    BuiltinContextCreation,
    BuiltinExecute,
    BuiltinInputValidation,
    BuiltinMiddlewareAfter,
    BuiltinMiddlewareBefore,
    BuiltinModuleLookup,
    BuiltinOutputValidation,
    BuiltinReturnResult,
    build_internal_strategy,
    build_minimal_strategy,
    build_performance_strategy,
    build_standard_strategy,
    build_testing_strategy,
)

# Trace Context
from apcore.trace_context import TraceContext, TraceParent

# ---------------------------------------------------------------------------
# Default client for simplified global access
# ---------------------------------------------------------------------------

# Module-level convenience wrappers — **Python-SDK-only**.
#
# These forward to a lazily-created default :class:`APCore` singleton. The
# TypeScript and Rust SDKs have no counterpart; users in those languages
# must construct a client explicitly. This is intentional — the Python
# convenience is optional syntactic sugar, not part of the cross-language
# apcore contract documented in the protocol spec.
#
# The client is allocated on first use (not at import) so
# ``import apcore`` stays cheap (no Registry/Executor instantiation, no
# cached event loop). Callers that want deterministic teardown can call
# :func:`apcore.close` to release the Executor's sync loop; a subsequent
# call reconstructs a fresh client.

# Backing cache for the lazy default client. The public name
# ``apcore._default_client`` is resolved via :func:`__getattr__` (PEP 562)
# so the attribute is *absent* until someone looks at it — preserving the
# "no work at import" guarantee.
_CACHED_DEFAULT_CLIENT: APCore | None = None


def _get_default_client() -> APCore:
    """Return the process-default APCore, creating it lazily on first use."""
    global _CACHED_DEFAULT_CLIENT
    if _CACHED_DEFAULT_CLIENT is None:
        _CACHED_DEFAULT_CLIENT = APCore()
    return _CACHED_DEFAULT_CLIENT


def close() -> None:
    """Release the default APCore client, if one was created.

    Unloads registered modules and releases the cached sync event loop.
    Inside asyncio, unloading modules with active calls is deferred until
    those calls finish. Subsequent convenience calls create a fresh client.
    """
    global _CACHED_DEFAULT_CLIENT
    client = _CACHED_DEFAULT_CLIENT
    if client is None:
        return
    _CACHED_DEFAULT_CLIENT = None
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        client._shutdown()
    else:
        client.registry._stop_admission()
        for module_id in client.registry.list(visibility=["public", "hidden"]):
            client.registry.unregister(module_id)
        client._close_owned_resources(terminal=True)


# Explicit imports of extension names remain available during migration.
_EXTENSION_EXPORTS = {
    "ApCoreEvent": "apcore.events",
    "BindingLoader": "apcore.bindings",
    "Config": "apcore.config",
    "ContextLogger": "apcore.observability",
    "ErrorEntry": "apcore.observability",
    "ErrorHistory": "apcore.observability",
    "EventEmitter": "apcore.events",
    "EventRetryConfig": "apcore.events",
    "FileOverridesStore": "apcore.sys_modules.overrides",
    "FileSubscriber": "apcore.events",
    "FilterSubscriber": "apcore.events",
    "HealthModule": "apcore.sys_modules.health",
    "HealthSummaryModule": "apcore.sys_modules.health",
    "InMemoryExporter": "apcore.observability",
    "InMemoryOverridesStore": "apcore.sys_modules.overrides",
    "InMemoryStorageBackend": "apcore.observability",
    "METRIC_CALLS_TOTAL": "apcore.observability.metrics",
    "METRIC_DURATION_SECONDS": "apcore.observability.metrics",
    "ManifestFullModule": "apcore.sys_modules.manifest",
    "ManifestModule": "apcore.sys_modules.manifest",
    "MetricsCollector": "apcore.observability",
    "MetricsMiddleware": "apcore.observability",
    "NoopUsageExporter": "apcore.observability",
    "OTLPExporter": "apcore.observability",
    "ObsLoggingMiddleware": "apcore.observability",
    "OverridesStore": "apcore.sys_modules.overrides",
    "PeriodicUsageExporter": "apcore.observability",
    "RESERVED_NAMESPACES": "apcore.config",
    "ReloadModule": "apcore.sys_modules.control",
    "Span": "apcore.observability",
    "SpanExporter": "apcore.observability",
    "StdoutExporter": "apcore.observability",
    "StdoutSubscriber": "apcore.events",
    "StorageBackend": "apcore.observability",
    "SysModulesContext": "apcore.sys_modules.registration",
    "ToggleFeatureModule": "apcore.sys_modules.control",
    "ToggleState": "apcore.sys_modules.control",
    "TracingMiddleware": "apcore.observability",
    "UpdateConfigModule": "apcore.sys_modules.control",
    "UsageCollector": "apcore.observability",
    "UsageExporter": "apcore.observability",
    "UsageMiddleware": "apcore.observability",
    "UsageModule": "apcore.sys_modules.usage",
    "UsageSummaryModule": "apcore.sys_modules.usage",
    "WebhookSubscriber": "apcore.events",
    "classify_health_status": "apcore.sys_modules.health",
    "create_span": "apcore.observability",
    "create_subscriber_from_config": "apcore.events",
    "discover_config_file": "apcore.config",
    "register_sys_modules": "apcore.sys_modules.registration",
}


def __getattr__(name: str) -> Any:
    """Lazily resolve the default client and explicit extension imports."""
    if name == "_default_client":
        return _get_default_client()
    extension = _EXTENSION_EXPORTS.get(name)
    if extension is not None:
        value = getattr(import_module(extension), name)
        globals()[name] = value
        return value
    raise AttributeError(f"module 'apcore' has no attribute {name!r}")


def call(
    module_id: str,
    inputs: dict[str, Any] | None = None,
    context: Context | None = None,
    version_hint: str | None = None,
) -> dict[str, Any]:
    """Global convenience for _default_client.call()."""
    return _get_default_client().call(module_id, inputs, context, version_hint=version_hint)


async def call_async(
    module_id: str,
    inputs: dict[str, Any] | None = None,
    context: Context | None = None,
    version_hint: str | None = None,
) -> dict[str, Any]:
    """Global convenience for _default_client.call_async()."""
    return await _get_default_client().call_async(module_id, inputs, context, version_hint=version_hint)


def module(
    id: str | None = None,  # noqa: A002
    description: str | None = None,
    documentation: str | None = None,
    annotations: dict[str, Any] | None = None,
    tags: list[str] | None = None,
    version: str = "1.0.0",
    metadata: dict[str, Any] | None = None,
    display: dict[str, Any] | None = None,
    examples: list[Any] | None = None,
    registry: Any = None,
    *,
    resources: dict[str, Any] | None = None,
) -> Any:
    """Register a module on the default APCore instance.

    This wrapper accepts an extra ``registry`` keyword argument not present on
    APCore.module() — it is an intentional escape hatch that lets global-decorator
    usage target a specific Registry instance rather than the default one.
    """
    from apcore.decorator import module as original_module

    return original_module(
        id=id,
        description=description,
        documentation=documentation,
        annotations=annotations,
        tags=tags,
        version=version,
        metadata=metadata,
        display=display,
        examples=examples,
        registry=_get_default_client().registry if registry is None else registry,
        resources=resources,
    )


async def stream(
    module_id: str,
    inputs: dict[str, Any] | None = None,
    context: Context | None = None,
    version_hint: str | None = None,
) -> AsyncGenerator[dict[str, Any], None]:
    """Global convenience for _default_client.stream()."""
    async with aclosing(_get_default_client().stream(module_id, inputs, context, version_hint=version_hint)) as output:
        async for chunk in output:
            yield chunk


def validate(module_id: str, inputs: dict[str, Any] | None = None, context: Context | None = None) -> PreflightResult:
    """Global convenience for _default_client.validate()."""
    return _get_default_client().validate(module_id, inputs, context)


def register(module_id: str, module_obj: Any) -> None:
    """Global convenience for _default_client.register()."""
    _get_default_client().register(module_id, module_obj)


def describe(module_id: str) -> str:
    """Global convenience for _default_client.describe()."""
    return _get_default_client().describe(module_id)


def use(middleware: Any) -> APCore:
    """Global convenience for _default_client.use()."""
    return _get_default_client().use(middleware)


def use_before(callback: Any) -> APCore:
    """Global convenience for _default_client.use_before()."""
    return _get_default_client().use_before(callback)


def use_after(callback: Any) -> APCore:
    """Global convenience for _default_client.use_after()."""
    return _get_default_client().use_after(callback)


def remove(middleware: Any) -> bool:
    """Global convenience for _default_client.remove()."""
    return _get_default_client().remove(middleware)


def discover() -> int:
    """Global convenience for _default_client.discover()."""
    return _get_default_client().discover()


def list_modules(tags: list[str] | None = None, prefix: str | None = None) -> list[str]:
    """Global convenience for _default_client.list_modules()."""
    return _get_default_client().list_modules(tags=tags, prefix=prefix)


def on(event_type: str, handler: Any) -> EventSubscriber:
    """Global convenience for _default_client.on()."""
    return _get_default_client().on(event_type, handler)


def off(subscriber: EventSubscriber) -> None:
    """Global convenience for _default_client.off()."""
    _get_default_client().off(subscriber)


def disable(module_id: str, reason: str = "Disabled via APCore client") -> dict[str, Any]:
    """Global convenience for _default_client.disable()."""
    return _get_default_client().disable(module_id, reason)


def enable(module_id: str, reason: str = "Enabled via APCore client") -> dict[str, Any]:
    """Global convenience for _default_client.enable()."""
    return _get_default_client().enable(module_id, reason)


try:
    __version__ = _get_version("apcore")
except PackageNotFoundError:
    __version__ = "unknown"

__all__ = [
    "ACL",
    "ACLRule",
    "APCore",
    "AsyncBoundModule",
    "ApprovalDeniedError",
    "ApprovalHandler",
    "ApprovalPendingError",
    "ApprovalRequest",
    "ApprovalResult",
    "AutoApproveHandler",
    "BoundModule",
    "CancelToken",
    "AccessHint",
    "CallableDefinition",
    "CapabilityPage",
    "CapabilitySearchProvider",
    "ModuleSummary",
    "VisibilityDecision",
    "Context",
    "ErrorCodes",
    "ExecutionPolicy",
    "PolicyDeniedError",
    "PolicyEvaluationError",
    "PolicyEvaluationRequest",
    "PolicyEvaluator",
    "PolicyVerdict",
    "ExecutionStrategy",
    "Executor",
    "Identity",
    "Middleware",
    "Module",
    "ModuleAnnotations",
    "ModuleError",
    "PipelineEngine",
    "PreflightResult",
    "PreviewResult",
    "Registry",
    "ReloadOptions",
    "SchemaExporter",
    "SchemaValidator",
    "ScopedApprovalHandler",
    "ShutdownOptions",
    "StreamCompletion",
    "StreamExecution",
    "StreamingModule",
    "TraceContext",
    "UnregisterOptions",
    "call",
    "call_async",
    "close",
    "module",
    "redact_sensitive",
    "register",
    "stream",
    "to_strict_schema",
    "validate",
]
