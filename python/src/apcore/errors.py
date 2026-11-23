"""Error hierarchy for apcore.

Defines ModuleError (the base for all apcore errors), standard ErrorCodes,
and specialized subclasses (ACLDeniedError, SchemaValidationError, etc.).
Each error carries optional AI guidance fields (retryable, ai_guidance,
user_fixable, suggestion) to enable Self-Healing agents.
"""

from __future__ import annotations

import threading
from datetime import datetime, timezone
from typing import Any

__all__ = [
    "ModuleError",
    "StreamingInterfaceError",
    "ConfigNotFoundError",
    "ConfigError",
    "ConfigNamespaceDuplicateError",
    "ConfigNamespaceReservedError",
    "ConfigEnvPrefixConflictError",
    "ConfigMountError",
    "ConfigBindError",
    "ErrorFormatterDuplicateError",
    "ACLRuleError",
    "ACLDeniedError",
    "ApprovalError",
    "ApprovalDeniedError",
    "ApprovalTimeoutError",
    "ApprovalPendingError",
    "ModuleNotFoundError",
    "ModuleDisabledError",
    "ModuleTimeoutError",
    "SchemaValidationError",
    "SchemaNotFoundError",
    "SchemaParseError",
    "SchemaCircularRefError",
    "SchemaMaxDepthExceededError",
    "CallDepthExceededError",
    "CircularCallError",
    "CallFrequencyExceededError",
    "InvalidInputError",
    "InvalidParentIdError",
    "ContextBindingError",
    "FuncMissingTypeHintError",
    "FuncMissingReturnTypeError",
    "BindingInvalidTargetError",
    "BindingModuleNotFoundError",
    "BindingCallableNotFoundError",
    "BindingNotCallableError",
    "BindingSchemaMissingError",
    "BindingSchemaInferenceFailedError",
    "BindingSchemaModeConflictError",
    "BindingStrictSchemaIncompatibleError",
    "BindingFileInvalidError",
    "CircularDependencyError",
    "ModuleLoadError",
    "ModuleExecuteError",
    "ReloadFailedError",
    "ModuleReloadConflictError",
    "SysModuleRegistrationError",
    "SysModulesDisabledError",
    "DependencyNotFoundError",
    "DependencyVersionMismatchError",
    "TaskLimitExceededError",
    "TaskStoreError",
    "VersionConstraintError",
    "InternalError",
    "ModuleIdConflictError",
    "InvalidSegmentError",
    "IdTooLongError",
    "CircuitBreakerOpenError",
    "CircuitOpenError",
    "ErrorCodes",
    "ErrorCodeCollisionError",
    "ErrorCodeRegistry",
    "FRAMEWORK_ERROR_CODE_PREFIXES",
]

_UNSET: Any = object()


# Declarative user_fixable policy, keyed by error code (single source of truth;
# kept in lock-step with conformance/fixtures/error_recovery_metadata.json so the
# language SDKs agree). True = the caller can resolve it by changing the input or
# configuration they sent; False = governance / system / structural / transient,
# not resolvable by changing input. Codes absent here leave user_fixable unset
# (e.g. MODULE_EXECUTE_ERROR — the module author supplies the recovery guidance).
_USER_FIXABLE_BY_CODE: dict[str, bool] = {
    # Caller can fix by changing input/config:
    "SCHEMA_VALIDATION_ERROR": True,
    "GENERAL_INVALID_INPUT": True,
    "MODULE_NOT_FOUND": True,
    "VERSION_CONSTRAINT_INVALID": True,
    "BINDING_SCHEMA_INFERENCE_FAILED": True,
    "BINDING_SCHEMA_MODE_CONFLICT": True,
    "BINDING_STRICT_SCHEMA_INCOMPATIBLE": True,
    "DEPENDENCY_NOT_FOUND": True,
    "DEPENDENCY_VERSION_MISMATCH": True,
    # Governance / system / structural / transient — not caller-fixable by input:
    "ACL_DENIED": False,
    "APPROVAL_DENIED": False,
    "APPROVAL_TIMEOUT": False,
    "MODULE_TIMEOUT": False,
    "MODULE_DISABLED": False,
    "CALL_DEPTH_EXCEEDED": False,
    "CIRCULAR_CALL": False,
    "CALL_FREQUENCY_EXCEEDED": False,
    "GENERAL_INTERNAL_ERROR": False,
}


class ModuleError(Exception):
    """Base error for all apcore errors."""

    _default_retryable: bool | None = None

    def __init__(
        self,
        code: str,
        message: str,
        details: dict[str, Any] | None = None,
        cause: Exception | None = None,
        trace_id: str | None = None,
        retryable: Any = _UNSET,
        ai_guidance: str | None = None,
        user_fixable: Any = _UNSET,
        suggestion: str | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.details: dict[str, Any] = details or {}
        self.cause = cause
        self.trace_id = trace_id
        self.timestamp = datetime.now(timezone.utc).isoformat()
        self.retryable = self._default_retryable if retryable is _UNSET else retryable
        self.ai_guidance = ai_guidance
        self.user_fixable = _USER_FIXABLE_BY_CODE.get(code) if user_fixable is _UNSET else user_fixable
        self.suggestion = suggestion

    def __str__(self) -> str:
        return f"[{self.code}] {self.message}"

    def to_dict(self) -> dict[str, Any]:
        """Serialize to dict with sparse output (null fields omitted)."""
        d: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
        }
        if self.details:
            d["details"] = self.details
        if self.cause is not None:
            d["cause"] = str(self.cause)
        if self.trace_id is not None:
            d["trace_id"] = self.trace_id
        d["timestamp"] = self.timestamp
        if self.retryable is not None:
            d["retryable"] = self.retryable
        if self.ai_guidance is not None:
            d["ai_guidance"] = self.ai_guidance
        if self.user_fixable is not None:
            d["user_fixable"] = self.user_fixable
        if self.suggestion is not None:
            d["suggestion"] = self.suggestion
        return d


class ConfigNotFoundError(ModuleError):
    """Raised when a configuration file cannot be found."""

    _default_retryable: bool | None = False

    def __init__(self, config_path: str, **kwargs: Any) -> None:
        super().__init__(
            code="CONFIG_NOT_FOUND",
            message=f"Configuration file not found: {config_path}",
            details={"config_path": config_path},
            **kwargs,
        )


class ConfigError(ModuleError):
    """Raised when configuration is invalid."""

    _default_retryable: bool | None = False

    def __init__(self, message: str, **kwargs: Any) -> None:
        super().__init__(code="CONFIG_INVALID", message=message, **kwargs)


class ConfigNamespaceDuplicateError(ModuleError):
    """Raised when a namespace name is already registered."""

    _default_retryable: bool | None = False

    def __init__(self, name: str, **kwargs: Any) -> None:
        super().__init__(
            code="CONFIG_NAMESPACE_DUPLICATE",
            message=f"Namespace already registered: {name!r}",
            details={"name": name},
            **kwargs,
        )


class ConfigNamespaceReservedError(ModuleError):
    """Raised when a namespace name is reserved by the framework."""

    _default_retryable: bool | None = False

    def __init__(self, name: str, message: str | None = None, **kwargs: Any) -> None:
        super().__init__(
            code="CONFIG_NAMESPACE_RESERVED",
            message=message or f"Namespace name is reserved: {name!r}",
            details={"name": name},
            **kwargs,
        )


class ConfigEnvPrefixConflictError(ModuleError):
    """Raised when a namespace env_prefix conflicts with an existing one."""

    _default_retryable: bool | None = False

    def __init__(self, env_prefix: str, **kwargs: Any) -> None:
        super().__init__(
            code="CONFIG_ENV_PREFIX_CONFLICT",
            message=f"Environment prefix conflicts with existing registration: {env_prefix!r}",
            details={"env_prefix": env_prefix},
            **kwargs,
        )


class ConfigEnvMapConflictError(ModuleError):
    """Raised when an env_map key is already claimed by another mapping."""

    _default_retryable: bool | None = False

    def __init__(self, env_var: str, owner: str, **kwargs: Any) -> None:
        super().__init__(
            code="CONFIG_ENV_MAP_CONFLICT",
            message=f"Environment variable {env_var!r} is already mapped by {owner!r}",
            details={"env_var": env_var, "owner": owner},
            **kwargs,
        )


class ConfigMountError(ModuleError):
    """Raised when a namespace mount operation is invalid."""

    _default_retryable: bool | None = False

    def __init__(self, message: str, **kwargs: Any) -> None:
        super().__init__(code="CONFIG_MOUNT_ERROR", message=message, **kwargs)


class ConfigBindError(ModuleError):
    """Raised when binding a namespace to a model class fails."""

    _default_retryable: bool | None = False

    def __init__(self, message: str, **kwargs: Any) -> None:
        super().__init__(code="CONFIG_BIND_ERROR", message=message, **kwargs)


class ErrorFormatterDuplicateError(ModuleError):
    """Raised when an error formatter is registered for an already-registered adapter."""

    _default_retryable: bool | None = False

    def __init__(self, adapter_name: str, **kwargs: Any) -> None:
        super().__init__(
            code="ERROR_FORMATTER_DUPLICATE",
            message=f"Error formatter already registered for adapter: {adapter_name!r}",
            details={"adapter_name": adapter_name},
            **kwargs,
        )


class ACLRuleError(ModuleError):
    """Raised when an ACL rule is invalid."""

    _default_retryable: bool | None = False

    def __init__(self, message: str, **kwargs: Any) -> None:
        super().__init__(code="ACL_RULE_ERROR", message=message, **kwargs)


class ACLDeniedError(ModuleError):
    """Raised when ACL denies access."""

    _default_retryable: bool | None = False

    def __init__(self, caller_id: str | None, target_id: str, **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Access denied for '{caller_id}' calling '{target_id}'. "
            "Verify the caller has the required role or permission, "
            "or try an alternative module with similar functionality.",
        )
        super().__init__(
            code="ACL_DENIED",
            message=f"Access denied: {caller_id} -> {target_id}",
            details={"caller_id": caller_id, "target_id": target_id},
            **kwargs,
        )

    @property
    def caller_id(self) -> str | None:
        """The caller ID that was denied."""
        return self.details["caller_id"]

    @property
    def target_id(self) -> str:
        """The target module ID that was denied access to."""
        return self.details["target_id"]


class ApprovalError(ModuleError):
    """Base error for all approval-related errors.

    Carries the full ApprovalResult for inspection by callers.
    Note: ``result`` is typed as ``Any`` to avoid a circular import with
    ``apcore.approval`` where ``ApprovalResult`` is defined.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        code: str,
        message: str,
        result: Any,
        **kwargs: Any,
    ) -> None:
        super().__init__(
            code=code,
            message=message,
            details={"module_id": kwargs.pop("module_id", None)},
            **kwargs,
        )
        self.result = result

    @property
    def module_id(self) -> str | None:
        """The module ID that required approval."""
        return self.details.get("module_id")

    @property
    def reason(self) -> str | None:
        """Human-readable reason from the approval handler's decision."""
        return getattr(self.result, "reason", None)


class ApprovalDeniedError(ApprovalError):
    """Raised when an approval handler rejects the request."""

    _default_retryable: bool | None = False

    def __init__(self, result: Any, module_id: str = "", **kwargs: Any) -> None:
        reason = getattr(result, "reason", None) or ""
        msg = f"Approval denied for module '{module_id}'"
        if reason:
            msg += f": {reason}"
        super().__init__(
            code="APPROVAL_DENIED",
            message=msg,
            result=result,
            module_id=module_id,
            **kwargs,
        )


class ApprovalTimeoutError(ApprovalError):
    """Raised when an approval request times out."""

    _default_retryable: bool | None = True

    def __init__(self, result: Any, module_id: str = "", **kwargs: Any) -> None:
        super().__init__(
            code="APPROVAL_TIMEOUT",
            message=f"Approval timed out for module '{module_id}'",
            result=result,
            module_id=module_id,
            **kwargs,
        )


class ApprovalPendingError(ApprovalError):
    """Raised when an approval is pending async resolution (Phase B)."""

    _default_retryable: bool | None = False

    def __init__(self, result: Any, module_id: str = "", **kwargs: Any) -> None:
        approval_id = getattr(result, "approval_id", None)
        super().__init__(
            code="APPROVAL_PENDING",
            message=f"Approval pending for module '{module_id}'",
            result=result,
            module_id=module_id,
            **kwargs,
        )
        self.details["approval_id"] = approval_id

    @property
    def approval_id(self) -> str | None:
        """The approval ID for async resume."""
        return self.details.get("approval_id")


class ModuleNotFoundError(ModuleError):
    """Raised when a module cannot be found."""

    _default_retryable: bool | None = False

    def __init__(self, module_id: str, message: str | None = None, **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Module '{module_id}' does not exist in the registry. "
            "Verify the module ID spelling. "
            "Use system.manifest.full to list available modules.",
        )
        super().__init__(
            code="MODULE_NOT_FOUND",
            message=message if message is not None else f"Module not found: {module_id}",
            details={"module_id": module_id},
            **kwargs,
        )


class ModuleDisabledError(ModuleError):
    """Raised when a disabled module is called."""

    _default_retryable: bool | None = False

    def __init__(self, module_id: str, **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Module '{module_id}' is currently disabled. "
            "Use system.control.toggle_feature to re-enable it, "
            "or find an alternative module.",
        )
        super().__init__(
            code="MODULE_DISABLED",
            message=f"Module '{module_id}' is disabled",
            details={"module_id": module_id},
            **kwargs,
        )


class ModuleTimeoutError(ModuleError):
    """Raised when module execution exceeds timeout."""

    _default_retryable: bool | None = True

    def __init__(self, module_id: str, timeout_ms: int, **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Module '{module_id}' timed out after {timeout_ms}ms. "
            "Consider: 1) Breaking the operation into smaller steps. "
            "2) Reducing the input data size. "
            "3) Asking the user if a longer timeout is acceptable.",
        )
        super().__init__(
            code="MODULE_TIMEOUT",
            message=f"Module {module_id} timed out after {timeout_ms}ms",
            details={"module_id": module_id, "timeout_ms": timeout_ms},
            **kwargs,
        )

    @property
    def module_id(self) -> str:
        """The module ID that timed out."""
        return self.details["module_id"]

    @property
    def timeout_ms(self) -> int:
        """The timeout value in milliseconds."""
        return self.details["timeout_ms"]


class CircuitBreakerOpenError(ModuleError):
    """Raised when CircuitBreakerMiddleware rejects a call because the circuit is open.

    Canonical name and code per cross-language spec (sync A-001):
    Python/TypeScript/Rust converge on ``CircuitBreakerOpenError`` /
    ``CIRCUIT_BREAKER_OPEN``. The legacy :class:`CircuitOpenError` is
    retained as a backwards-compatible subclass alias.
    """

    _default_retryable: bool | None = True

    def __init__(self, module_id: str, caller_id: str | None = None, **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Module '{module_id}' is temporarily unavailable (circuit open). "
            "The circuit will enter HALF_OPEN after the recovery window elapses. "
            "Retry the request after a short delay.",
        )
        # Allow subclasses (e.g. CircuitOpenError legacy alias) to override
        # the wire code via kwargs without breaking signature compatibility.
        code = kwargs.pop("code", "CIRCUIT_BREAKER_OPEN")
        super().__init__(
            code=code,
            message=f"Circuit open for module '{module_id}' — call rejected",
            details={"module_id": module_id, "caller_id": caller_id},
            **kwargs,
        )

    @property
    def module_id(self) -> str:
        """The module whose circuit is open."""
        return self.details["module_id"]

    @property
    def caller_id(self) -> str | None:
        """The caller whose (module_id, caller_id) circuit is open, if known."""
        return self.details.get("caller_id")


class CircuitOpenError(CircuitBreakerOpenError):
    """Deprecated alias for :class:`CircuitBreakerOpenError` (sync A-001).

    Kept as a subclass so existing ``except CircuitOpenError`` blocks continue
    to work. New code SHOULD raise and catch :class:`CircuitBreakerOpenError`
    directly. Will be removed in a future major release.

    Note: instances of this legacy class still emit the canonical
    ``CIRCUIT_BREAKER_OPEN`` error code on the wire — only the Python class
    name is retained for backwards compatibility.
    """


class SchemaValidationError(ModuleError):
    """Raised when schema validation fails."""

    _default_retryable: bool | None = False

    def __init__(
        self,
        message: str = "Schema validation failed",
        errors: list[dict[str, Any]] | None = None,
        **kwargs: Any,
    ) -> None:
        kwargs.setdefault(
            "ai_guidance",
            "Input validation failed. Review the error details to identify "
            "which fields have invalid values, then correct them or "
            "ask the user for valid input.",
        )
        super().__init__(
            code="SCHEMA_VALIDATION_ERROR",
            message=message,
            details={"errors": errors or []},
            **kwargs,
        )


class SchemaNotFoundError(ModuleError):
    """Raised when a schema file or reference target cannot be found."""

    _default_retryable: bool | None = False

    def __init__(self, schema_id: str, **kwargs: Any) -> None:
        super().__init__(
            code="SCHEMA_NOT_FOUND",
            message=f"Schema not found: {schema_id}",
            details={"schema_id": schema_id},
            **kwargs,
        )


class SchemaParseError(ModuleError):
    """Raised when a schema file has invalid syntax."""

    _default_retryable: bool | None = False

    def __init__(self, message: str, **kwargs: Any) -> None:
        super().__init__(code="SCHEMA_PARSE_ERROR", message=message, **kwargs)


class SchemaCircularRefError(ModuleError):
    """Raised when circular $ref references are detected."""

    _default_retryable: bool | None = False

    def __init__(self, ref_path: str, **kwargs: Any) -> None:
        super().__init__(
            code="SCHEMA_CIRCULAR_REF",
            message=f"Circular reference detected: {ref_path}",
            details={"ref_path": ref_path},
            **kwargs,
        )


class SchemaMaxDepthExceededError(ModuleError):
    """Raised when $ref resolution exceeds the maximum reference depth cap."""

    _default_retryable: bool | None = False

    def __init__(self, ref_path: str, **kwargs: Any) -> None:
        super().__init__(
            code="SCHEMA_MAX_DEPTH_EXCEEDED",
            message=f"Maximum reference depth exceeded: {ref_path}",
            details={"ref_path": ref_path},
            **kwargs,
        )


class CallDepthExceededError(ModuleError):
    """Raised when call chain exceeds maximum depth."""

    _default_retryable: bool | None = False

    def __init__(self, depth: int, max_depth: int, call_chain: list[str], **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Call depth {depth} exceeds maximum {max_depth}. "
            "Simplify the module call chain or restructure "
            "to reduce nesting depth.",
        )
        super().__init__(
            code="CALL_DEPTH_EXCEEDED",
            message=f"Call depth {depth} exceeds maximum {max_depth}",
            details={"depth": depth, "max_depth": max_depth, "call_chain": call_chain},
            **kwargs,
        )

    @property
    def current_depth(self) -> int:
        """The current call chain depth that exceeded the limit."""
        return self.details["depth"]

    @property
    def max_depth(self) -> int:
        """The configured maximum call depth."""
        return self.details["max_depth"]


class CircularCallError(ModuleError):
    """Raised when a circular call is detected."""

    _default_retryable: bool | None = False

    def __init__(self, module_id: str, call_chain: list[str], **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            "A circular call was detected in the module call chain. "
            "Review the call_chain in error details and restructure "
            "to eliminate the cycle.",
        )
        super().__init__(
            code="CIRCULAR_CALL",
            message=f"Circular call detected for module {module_id}",
            details={"module_id": module_id, "call_chain": call_chain},
            **kwargs,
        )

    @property
    def module_id(self) -> str:
        """The module ID that formed the circular call."""
        return self.details["module_id"]


class CallFrequencyExceededError(ModuleError):
    """Raised when a module is called too many times."""

    _default_retryable: bool | None = False

    def __init__(
        self,
        module_id: str,
        count: int,
        max_repeat: int,
        call_chain: list[str],
        **kwargs: Any,
    ) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Module '{module_id}' was called {count} times in this chain "
            f"(limit {max_repeat}), tripping the frequency guard. Reduce "
            "repeated calls or batch the work before retrying.",
        )
        super().__init__(
            code="CALL_FREQUENCY_EXCEEDED",
            message=f"Module {module_id} called {count} times, max is {max_repeat}",
            details={
                "module_id": module_id,
                "count": count,
                "max_repeat": max_repeat,
                "call_chain": call_chain,
            },
            **kwargs,
        )

    @property
    def module_id(self) -> str:
        """The module ID that exceeded the frequency limit."""
        return self.details["module_id"]

    @property
    def count(self) -> int:
        """The current invocation count."""
        return self.details["count"]

    @property
    def max_repeat(self) -> int:
        """The configured maximum repeat count."""
        return self.details["max_repeat"]


class InvalidInputError(ModuleError):
    """Raised for invalid input."""

    _default_retryable: bool | None = False

    def __init__(
        self,
        message: str = "Invalid input",
        code: str = "GENERAL_INVALID_INPUT",
        **kwargs: Any,
    ) -> None:
        kwargs.setdefault(
            "ai_guidance",
            "The input was malformed or missing required fields. Check the "
            "values against the module's input_schema and retry with corrected "
            "input.",
        )
        super().__init__(code=code, message=message, **kwargs)


class InvalidParentIdError(ModuleError, ValueError):
    """Raised when ``TraceContext.inject`` gets a ``parent_id`` override that is
    not 16 lowercase hex characters (``^[0-9a-f]{16}$``).

    Decision D-51 (``docs/features/observability.md`` §"Optional ``parent_id``
    Override on ``inject()``") pins the wire code ``INVALID_PARENT_ID`` for this
    rejection in every SDK: apcore-typescript throws an ``Error`` carrying
    ``code = 'INVALID_PARENT_ID'`` and apcore-rust returns
    ``ErrorCode::InvalidParentId``.

    Why it also inherits :class:`ValueError`: ``TraceContext.inject`` is public
    API that raised a bare ``ValueError`` from 0.13.0 through 0.26.x, and the
    spec's own Python example documents ``except ValueError``. Inheriting both
    gives existing ``except ValueError`` callers an unbroken contract while
    supplying the ``.code`` a polyglot caller matches on. The MRO is
    ``InvalidParentIdError -> ModuleError -> ValueError -> Exception``, so
    ``ModuleError.__str__`` wins and ``str(exc)`` renders
    ``"[INVALID_PARENT_ID] parent_id must be 16 lowercase hex chars, got 'ZZZZ'"``.

    ``user_fixable`` is set on the class rather than in
    :data:`_USER_FIXABLE_BY_CODE` because that map is asserted equal to
    ``conformance/fixtures/error_recovery_metadata.json``, which does not
    enumerate this code. ``True`` matches apcore-rust's ``user_fixable_for_code``
    — the caller supplies the bad ``parent_id`` and can fix it by passing a
    valid 16-hex span id.
    """

    _default_retryable: bool | None = False

    def __init__(self, parent_id: Any, **kwargs: Any) -> None:
        kwargs.setdefault("user_fixable", True)
        kwargs.setdefault(
            "ai_guidance",
            "The parent_id override passed to TraceContext.inject() must be 16 "
            "lowercase hex characters (^[0-9a-f]{16}$). Pass a valid span id or "
            "omit the argument to let apcore derive one.",
        )
        super().__init__(
            code="INVALID_PARENT_ID",
            message=f"parent_id must be 16 lowercase hex chars, got {parent_id!r}",
            details={"parent_id": parent_id},
            **kwargs,
        )


class ContextBindingError(ModuleError):
    """Raised when an Executor attempts to bind itself to a Context that is
    already bound to a *different* Executor instance.

    Per apcore PROTOCOL_SPEC §"Contract: Executor binding to Context", a
    Context whose ``executor`` field is non-null and refers to a different
    Executor instance is a cross-executor conflict; rebinding the same
    instance is a noop. SDKs SHOULD raise this error; SDKs that choose to
    accept silently MUST document the deviation prominently.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        message: str = "Context is already bound to a different Executor instance",
        code: str = "CONTEXT_BINDING_ERROR",
        **kwargs: Any,
    ) -> None:
        super().__init__(code=code, message=message, **kwargs)


class FuncMissingTypeHintError(ModuleError):
    """Raised when a function parameter has no type annotation or a forward reference cannot be resolved."""

    _default_retryable: bool | None = False

    def __init__(self, *, function_name: str, parameter_name: str, **kwargs: Any) -> None:
        super().__init__(
            code="FUNC_MISSING_TYPE_HINT",
            message=(
                f"Parameter '{parameter_name}' in function '{function_name}' has no type annotation. "
                f"Add a type hint like '{parameter_name}: str'."
            ),
            details={"function_name": function_name, "parameter_name": parameter_name},
            **kwargs,
        )


class FuncMissingReturnTypeError(ModuleError):
    """Raised when a function has no return type annotation."""

    _default_retryable: bool | None = False

    def __init__(self, *, function_name: str, **kwargs: Any) -> None:
        super().__init__(
            code="FUNC_MISSING_RETURN_TYPE",
            message=f"Function '{function_name}' has no return type annotation. Add a return type like '-> dict'.",
            details={"function_name": function_name},
            **kwargs,
        )


class BindingInvalidTargetError(ModuleError):
    """Raised when a binding target string does not contain a ':' separator."""

    _default_retryable: bool | None = False

    def __init__(self, *, target: str, **kwargs: Any) -> None:
        super().__init__(
            code="BINDING_INVALID_TARGET",
            message=f"Invalid binding target '{target}'. Expected format: 'module.path:callable_name'.",
            details={"target": target},
            **kwargs,
        )


class BindingModuleNotFoundError(ModuleError):
    """Raised when a binding target module cannot be imported."""

    _default_retryable: bool | None = False

    def __init__(self, *, module_path: str, **kwargs: Any) -> None:
        super().__init__(
            code="BINDING_MODULE_NOT_FOUND",
            message=f"Cannot import module '{module_path}'.",
            details={"module_path": module_path},
            **kwargs,
        )


class BindingCallableNotFoundError(ModuleError):
    """Raised when a callable cannot be found in the target module."""

    _default_retryable: bool | None = False

    def __init__(self, *, callable_name: str, module_path: str, **kwargs: Any) -> None:
        super().__init__(
            code="BINDING_CALLABLE_NOT_FOUND",
            message=f"Cannot find callable '{callable_name}' in module '{module_path}'.",
            details={"callable_name": callable_name, "module_path": module_path},
            **kwargs,
        )


class BindingNotCallableError(ModuleError):
    """Raised when a resolved binding target is not callable."""

    _default_retryable: bool | None = False

    def __init__(self, *, target: str, **kwargs: Any) -> None:
        super().__init__(
            code="BINDING_NOT_CALLABLE",
            message=f"Resolved target '{target}' is not callable.",
            details={"target": target},
            **kwargs,
        )


class BindingSchemaInferenceFailedError(ModuleError):
    """Raised when auto-schema mode (explicit or implicit) cannot infer a schema from the target.

    See protocol-spec §5.12.2 and §5.12.5.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        *,
        target: str,
        module_id: str | None = None,
        file_path: str | None = None,
        line: int | None = None,
        remediation: str | None = None,
        **kwargs: Any,
    ) -> None:
        loc = ""
        if file_path is not None:
            loc = f"{file_path}"
            if line is not None:
                loc += f":{line}"
            loc += ": "
        mod_part = f"binding '{module_id}' " if module_id else ""
        rem = (
            remediation
            or "target function lacks complete type hints. Add type annotations to all parameters and the return type, or specify input_schema/output_schema explicitly."
        )
        super().__init__(
            code="BINDING_SCHEMA_INFERENCE_FAILED",
            message=(
                f"{loc}{mod_part}auto schema inference failed for target '{target}'. "
                f"{rem} See protocol-spec §5.12.5"
            ),
            details={
                "target": target,
                "module_id": module_id,
                "file_path": file_path,
                "line": line,
            },
            **kwargs,
        )


# Deprecated alias kept for backward compatibility in 0.19.x; canonical name is
# BindingSchemaInferenceFailedError per protocol-spec §5.12.8.
BindingSchemaMissingError = BindingSchemaInferenceFailedError


class BindingSchemaModeConflictError(ModuleError):
    """Raised when a binding entry specifies multiple schema modes simultaneously.

    See protocol-spec §5.12.2.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        *,
        module_id: str,
        modes_listed: list[str],
        file_path: str | None = None,
        line: int | None = None,
        **kwargs: Any,
    ) -> None:
        loc = ""
        if file_path is not None:
            loc = f"{file_path}"
            if line is not None:
                loc += f":{line}"
            loc += ": "
        modes_str = ", ".join(modes_listed)
        super().__init__(
            code="BINDING_SCHEMA_MODE_CONFLICT",
            message=(
                f"{loc}binding '{module_id}' specifies multiple schema modes ({modes_str}). "
                "Choose one. See protocol-spec §5.12.2"
            ),
            details={
                "module_id": module_id,
                "modes_listed": modes_listed,
                "file_path": file_path,
                "line": line,
            },
            **kwargs,
        )


class BindingStrictSchemaIncompatibleError(ModuleError):
    """Raised when auto_schema: strict is requested but inferred schema contains incompatible features.

    See protocol-spec §5.12.5.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        *,
        module_id: str,
        features_listed: list[str],
        file_path: str | None = None,
        line: int | None = None,
        **kwargs: Any,
    ) -> None:
        loc = ""
        if file_path is not None:
            loc = f"{file_path}"
            if line is not None:
                loc += f":{line}"
            loc += ": "
        features_str = ", ".join(features_listed)
        super().__init__(
            code="BINDING_STRICT_SCHEMA_INCOMPATIBLE",
            message=(
                f"{loc}binding '{module_id}' uses auto_schema: strict but inferred schema "
                f"contains incompatible features: {features_str}. "
                "See protocol-spec §5.12.5"
            ),
            details={
                "module_id": module_id,
                "features_listed": features_listed,
                "file_path": file_path,
                "line": line,
            },
            **kwargs,
        )


class BindingFileInvalidError(ModuleError):
    """Raised when a binding file has parse errors, missing required fields, or is empty."""

    _default_retryable: bool | None = False

    def __init__(self, *, file_path: str, reason: str, **kwargs: Any) -> None:
        super().__init__(
            code="BINDING_FILE_INVALID",
            message=f"Invalid binding file '{file_path}': {reason}",
            details={"file_path": file_path, "reason": reason},
            **kwargs,
        )


class CircularDependencyError(ModuleError):
    """Raised when circular dependencies are detected among modules."""

    _default_retryable: bool | None = False

    def __init__(self, cycle_path: list[str], **kwargs: Any) -> None:
        super().__init__(
            code="CIRCULAR_DEPENDENCY",
            message=f"Circular dependency detected: {' -> '.join(cycle_path)}",
            details={"cycle_path": cycle_path},
            **kwargs,
        )


class ModuleLoadError(ModuleError):
    """Raised when a module file cannot be loaded or resolved."""

    _default_retryable: bool | None = False

    def __init__(self, module_id: str, reason: str, **kwargs: Any) -> None:
        super().__init__(
            code="MODULE_LOAD_ERROR",
            message=f"Failed to load module '{module_id}': {reason}",
            details={"module_id": module_id, "reason": reason},
            **kwargs,
        )


class DependencyNotFoundError(ModuleError):
    """Raised when a module's required dependency is not registered.

    Corresponds to error code ``DEPENDENCY_NOT_FOUND`` per PROTOCOL_SPEC §5.15.2.
    Replaces the previous practice of raising ``ModuleLoadError`` for missing
    dependencies — callers that caught ``ModuleLoadError`` for this scenario
    should either catch ``DependencyNotFoundError`` specifically or catch the
    common ``ModuleError`` base.
    """

    _default_retryable: bool | None = False

    def __init__(self, module_id: str, dependency_id: str, **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Module '{module_id}' declares a required dependency on "
            f"'{dependency_id}', but no such module is registered. Either "
            f"register '{dependency_id}' before loading '{module_id}', mark "
            "the dependency as optional, or remove it.",
        )
        super().__init__(
            code="DEPENDENCY_NOT_FOUND",
            message=(f"Module '{module_id}' has unsatisfied required dependency '{dependency_id}'"),
            details={"module_id": module_id, "dependency_id": dependency_id},
            **kwargs,
        )


class DependencyVersionMismatchError(ModuleError):
    """Raised when a declared dependency's version constraint is not satisfied."""

    _default_retryable: bool | None = False

    def __init__(
        self,
        module_id: str,
        dependency_id: str,
        required: str,
        actual: str,
        **kwargs: Any,
    ) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Module '{module_id}' declares dependency '{dependency_id}' with "
            f"version constraint '{required}', but the registered version is "
            f"'{actual}'. Either upgrade the dependency, relax the constraint, "
            "or register a compatible version.",
        )
        super().__init__(
            code="DEPENDENCY_VERSION_MISMATCH",
            message=(
                f"Module '{module_id}' requires dependency '{dependency_id}' "
                f"version '{required}', but registered version is '{actual}'"
            ),
            details={
                "module_id": module_id,
                "dependency_id": dependency_id,
                "required": required,
                "actual": actual,
            },
            **kwargs,
        )


class ReloadFailedError(ModuleError):
    """Raised when module hot-reload fails during re-discover or re-register."""

    _default_retryable: bool | None = True

    def __init__(self, module_id: str, reason: str, **kwargs: Any) -> None:
        super().__init__(
            code="RELOAD_FAILED",
            message=f"Failed to reload module '{module_id}': {reason}",
            details={"module_id": module_id, "reason": reason},
            **kwargs,
        )


class ModuleReloadConflictError(ModuleError):
    """Raised when both module_id and path_filter are provided to reload_module."""

    _default_retryable: bool | None = False

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(
            code="MODULE_RELOAD_CONFLICT",
            message="'module_id' and 'path_filter' are mutually exclusive",
            **kwargs,
        )


class SysModuleRegistrationError(ModuleError):
    """Raised when a system module fails to register and fail_on_error=True."""

    _default_retryable: bool | None = False

    def __init__(self, module_id: str, reason: str, **kwargs: Any) -> None:
        super().__init__(
            code="SYS_MODULE_REGISTRATION_FAILED",
            message=f"System module '{module_id}' failed to register: {reason}",
            details={"module_id": module_id, "reason": reason},
            **kwargs,
        )


class SysModulesDisabledError(ModuleError):
    """Raised when an APCore client method requires sys_modules to be enabled.

    Cross-language equivalent of Rust's ``ErrorCode::SysModulesDisabled``.
    Replaces bare ``RuntimeError`` in ``APCore.on``/``off``/``disable``/``enable``
    so callers can dispatch on ``error.code == "SYS_MODULES_DISABLED"`` instead
    of catching the generic ``Exception`` base.
    """

    _default_retryable: bool | None = False

    def __init__(self, message: str, **kwargs: Any) -> None:
        super().__init__(
            code="SYS_MODULES_DISABLED",
            message=message,
            **kwargs,
        )


class TaskLimitExceededError(ModuleError):
    """Raised when ``AsyncTaskManager.submit`` is called at the task-slot limit.

    Callers that caught the prior ``RuntimeError("Task limit reached ...")``
    should either catch ``TaskLimitExceededError`` specifically or catch the
    ``ModuleError`` base. The typed form makes the failure dispatchable via
    ``error.code == ErrorCodes.TASK_LIMIT_EXCEEDED`` across language SDKs.
    """

    _default_retryable: bool | None = True

    def __init__(self, max_tasks: int, **kwargs: Any) -> None:
        super().__init__(
            code="TASK_LIMIT_EXCEEDED",
            message=f"Task limit reached ({max_tasks})",
            details={"max_tasks": max_tasks},
            **kwargs,
        )


class TaskStoreError(ModuleError):
    """Raised when a ``TaskStore`` backend is unreachable or refuses an operation.

    Spec v1.50.0, D-92. ``async-tasks.md`` declares
    ``TaskStoreError(code=TASK_STORE_UNAVAILABLE)`` on eight surfaces — every
    ``TaskStore`` method and every ``AsyncTaskManager`` method that touches the
    store — and no SDK defined it, so no caller could ever catch it. A declared
    error type that no implementation can raise is the "declared surface
    reaches no mechanism" shape PROTOCOL_SPEC §9.1.3 forbids for configuration
    keys, here applied to an error contract.

    The bundled :class:`~apcore.async_task.InMemoryTaskStore` cannot fail and
    therefore never raises this; that is exactly why the type is **exported**
    rather than merely raised internally. The hosts who need it are the ones
    writing the network-backed stores the contract was written for: they raise
    it, and ``AsyncTaskManager`` propagates it (D-81) rather than mapping a
    store outage onto "task not found" / an empty list / a ``cancel`` that
    returns ``True`` for a save that never landed.
    """

    _default_retryable: bool | None = True

    def __init__(self, operation: str = "", reason: str = "", **kwargs: Any) -> None:
        detail = f"TaskStore operation '{operation}' failed" if operation else "TaskStore is unavailable"
        if reason:
            detail = f"{detail}: {reason}"
        details: dict[str, Any] = {}
        if operation:
            details["operation"] = operation
        if reason:
            details["reason"] = reason
        super().__init__(
            code="TASK_STORE_UNAVAILABLE",
            message=detail,
            details=details or None,
            **kwargs,
        )


class VersionConstraintError(ModuleError):
    """Raised when a declared version constraint string is malformed.

    Examples: a leading operator without a digit operand (``">="``), a
    ``"v1.0"`` prefix (unsupported), or a non-semver operand such as
    ``"not_a_version"`` that would silently degrade to ``(0,0,0)``.
    Surfaced at parse time by ``matches_version_hint`` / ``VersionedStore``
    callers to prevent YAML typos from permanently disabling constraint
    enforcement.
    """

    _default_retryable: bool | None = False

    def __init__(self, constraint: str, reason: str, **kwargs: Any) -> None:
        kwargs.setdefault(
            "ai_guidance",
            f"Constraint '{constraint}' is not a valid semver expression. "
            f"Use forms like '1.2.3', '>=1.2.0,<2.0.0', '^1.2.3', or '~1.2'. "
            f"{reason}",
        )
        super().__init__(
            code="VERSION_CONSTRAINT_INVALID",
            message=f"Invalid version constraint '{constraint}': {reason}",
            details={"constraint": constraint, "reason": reason},
            **kwargs,
        )


class ModuleExecuteError(ModuleError):
    """Raised when module execution fails with an unhandled error."""

    _default_retryable: bool | None = None

    def __init__(
        self,
        module_id: str = "",
        message: str = "Module execution failed",
        **kwargs: Any,
    ) -> None:
        super().__init__(
            code="MODULE_EXECUTE_ERROR",
            message=message,
            details={"module_id": module_id},
            **kwargs,
        )


class InternalError(ModuleError):
    """Raised for unexpected internal framework errors."""

    _default_retryable: bool | None = True

    def __init__(self, message: str = "Internal error", **kwargs: Any) -> None:
        super().__init__(
            code="GENERAL_INTERNAL_ERROR",
            message=message,
            **kwargs,
        )


class ModuleIdConflictError(ModuleError):
    """Raised when two classes in a file produce the same snake_case segment.

    Corresponds to error code ``MODULE_ID_CONFLICT`` per PROTOCOL_SPEC §2.1.1.
    No modules from the conflicting file are registered.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        file_path: str,
        class_names: list[str],
        conflicting_segment: str,
        **kwargs: Any,
    ) -> None:
        super().__init__(
            code="MODULE_ID_CONFLICT",
            message=(
                f"Module ID conflict in '{file_path}': "
                f"classes {class_names} produce the same segment '{conflicting_segment}'"
            ),
            details={
                "file_path": file_path,
                "class_names": class_names,
                "conflicting_segment": conflicting_segment,
            },
            **kwargs,
        )


class InvalidSegmentError(ModuleError):
    """Raised when a derived class_segment does not conform to the canonical ID grammar.

    Corresponds to error code ``INVALID_SEGMENT`` per PROTOCOL_SPEC §2.1.1.
    A valid segment must match ``^[a-z][a-z0-9_]*$``.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        file_path: str,
        class_name: str,
        segment: str,
        **kwargs: Any,
    ) -> None:
        super().__init__(
            code="INVALID_SEGMENT",
            message=(
                f"Invalid segment '{segment}' derived from class '{class_name}' "
                f"in '{file_path}': must match ^[a-z][a-z0-9_]*$"
            ),
            details={
                "file_path": file_path,
                "class_name": class_name,
                "segment": segment,
            },
            **kwargs,
        )


class IdTooLongError(ModuleError):
    """Raised when a derived module_id exceeds 192 characters.

    Corresponds to error code ``ID_TOO_LONG`` per PROTOCOL_SPEC §2.1.1.
    """

    _default_retryable: bool | None = False

    def __init__(
        self,
        file_path: str,
        module_id: str,
        **kwargs: Any,
    ) -> None:
        super().__init__(
            code="ID_TOO_LONG",
            message=(f"Derived module ID in '{file_path}' exceeds 192 characters (length: {len(module_id)})"),
            details={
                "file_path": file_path,
                "module_id": module_id,
                "length": len(module_id),
            },
            **kwargs,
        )


class ErrorCodes:
    """All framework error codes as constants.

    Use these instead of hardcoding error code strings.

    Example:
        if error.code == ErrorCodes.MODULE_NOT_FOUND:
            handle_not_found()
    """

    CONFIG_NOT_FOUND = "CONFIG_NOT_FOUND"
    CONFIG_INVALID = "CONFIG_INVALID"
    CONFIG_NAMESPACE_DUPLICATE = "CONFIG_NAMESPACE_DUPLICATE"
    CONFIG_NAMESPACE_RESERVED = "CONFIG_NAMESPACE_RESERVED"
    CONFIG_ENV_PREFIX_CONFLICT = "CONFIG_ENV_PREFIX_CONFLICT"
    CONFIG_MOUNT_ERROR = "CONFIG_MOUNT_ERROR"
    CONFIG_BIND_ERROR = "CONFIG_BIND_ERROR"
    CONFIG_ENV_MAP_CONFLICT = "CONFIG_ENV_MAP_CONFLICT"
    ERROR_FORMATTER_DUPLICATE = "ERROR_FORMATTER_DUPLICATE"
    ACL_RULE_ERROR = "ACL_RULE_ERROR"
    ACL_DENIED = "ACL_DENIED"
    MODULE_NOT_FOUND = "MODULE_NOT_FOUND"
    MODULE_DISABLED = "MODULE_DISABLED"
    MODULE_TIMEOUT = "MODULE_TIMEOUT"
    MODULE_LOAD_ERROR = "MODULE_LOAD_ERROR"
    MODULE_EXECUTE_ERROR = "MODULE_EXECUTE_ERROR"
    RELOAD_FAILED = "RELOAD_FAILED"
    EXECUTION_CANCELLED = "EXECUTION_CANCELLED"
    SCHEMA_VALIDATION_ERROR = "SCHEMA_VALIDATION_ERROR"
    # Deprecated, retired alias — no longer emitted by any code path. All schema
    # validation failures use SCHEMA_VALIDATION_ERROR (PROTOCOL_SPEC §8.2 registers
    # only ERROR). Retained for backward-compatible imports only.
    SCHEMA_VALIDATION_FAILED = "SCHEMA_VALIDATION_FAILED"
    SCHEMA_UNION_NO_MATCH = "SCHEMA_UNION_NO_MATCH"
    SCHEMA_UNION_AMBIGUOUS = "SCHEMA_UNION_AMBIGUOUS"
    SCHEMA_NOT_FOUND = "SCHEMA_NOT_FOUND"
    SCHEMA_PARSE_ERROR = "SCHEMA_PARSE_ERROR"
    SCHEMA_CIRCULAR_REF = "SCHEMA_CIRCULAR_REF"
    SCHEMA_MAX_DEPTH_EXCEEDED = "SCHEMA_MAX_DEPTH_EXCEEDED"
    CALL_DEPTH_EXCEEDED = "CALL_DEPTH_EXCEEDED"
    CIRCULAR_CALL = "CIRCULAR_CALL"
    CALL_FREQUENCY_EXCEEDED = "CALL_FREQUENCY_EXCEEDED"
    GENERAL_INVALID_INPUT = "GENERAL_INVALID_INPUT"
    GENERAL_INTERNAL_ERROR = "GENERAL_INTERNAL_ERROR"
    INVALID_MODULE_ID = "INVALID_MODULE_ID"
    DUPLICATE_MODULE_ID = "DUPLICATE_MODULE_ID"
    FUNC_MISSING_TYPE_HINT = "FUNC_MISSING_TYPE_HINT"
    FUNC_MISSING_RETURN_TYPE = "FUNC_MISSING_RETURN_TYPE"
    BINDING_INVALID_TARGET = "BINDING_INVALID_TARGET"
    BINDING_MODULE_NOT_FOUND = "BINDING_MODULE_NOT_FOUND"
    BINDING_CALLABLE_NOT_FOUND = "BINDING_CALLABLE_NOT_FOUND"
    BINDING_NOT_CALLABLE = "BINDING_NOT_CALLABLE"
    BINDING_SCHEMA_MISSING = "BINDING_SCHEMA_MISSING"
    BINDING_SCHEMA_INFERENCE_FAILED = "BINDING_SCHEMA_INFERENCE_FAILED"
    BINDING_SCHEMA_MODE_CONFLICT = "BINDING_SCHEMA_MODE_CONFLICT"
    BINDING_STRICT_SCHEMA_INCOMPATIBLE = "BINDING_STRICT_SCHEMA_INCOMPATIBLE"
    BINDING_FILE_INVALID = "BINDING_FILE_INVALID"
    CIRCULAR_DEPENDENCY = "CIRCULAR_DEPENDENCY"
    MIDDLEWARE_CHAIN_ERROR = "MIDDLEWARE_CHAIN_ERROR"
    CIRCUIT_OPEN = "CIRCUIT_OPEN"  # Deprecated: use CIRCUIT_BREAKER_OPEN (sync A-001)
    CIRCUIT_BREAKER_OPEN = "CIRCUIT_BREAKER_OPEN"
    APPROVAL_DENIED = "APPROVAL_DENIED"
    APPROVAL_TIMEOUT = "APPROVAL_TIMEOUT"
    APPROVAL_PENDING = "APPROVAL_PENDING"
    VERSION_INCOMPATIBLE = "VERSION_INCOMPATIBLE"
    ERROR_CODE_COLLISION = "ERROR_CODE_COLLISION"
    GENERAL_NOT_IMPLEMENTED = "GENERAL_NOT_IMPLEMENTED"
    DEPENDENCY_NOT_FOUND = "DEPENDENCY_NOT_FOUND"
    DEPENDENCY_VERSION_MISMATCH = "DEPENDENCY_VERSION_MISMATCH"
    VERSION_CONSTRAINT_INVALID = "VERSION_CONSTRAINT_INVALID"
    TASK_LIMIT_EXCEEDED = "TASK_LIMIT_EXCEEDED"
    TASK_STORE_UNAVAILABLE = "TASK_STORE_UNAVAILABLE"
    REAPER_ALREADY_RUNNING = "REAPER_ALREADY_RUNNING"
    PIPELINE_STEP_ERROR = "PIPELINE_STEP_ERROR"
    PIPELINE_STEP_NOT_FOUND = "PIPELINE_STEP_NOT_FOUND"
    # The pipeline/step/strategy codes below are raised from `apcore.pipeline`,
    # not from this module, which is why they were absent here. `ErrorCodes` is
    # the sole input to `_FRAMEWORK_CODES` — the exact-code half of the A17
    # collision guard — so a user module could register them as its own. See the
    # note above FRAMEWORK_ERROR_CODE_PREFIXES: the reserved-prefix set was
    # narrowed to 14 precisely because this exact-code check was believed to
    # cover the specific codes under CIRCUIT_/PIPELINE_/STREAMING_/CONTEXT_.
    # apcore-rust derives its set from the exhaustive `ErrorCode` enum and
    # rejects all eight; this map must be equally complete.
    PIPELINE_ABORT = "PIPELINE_ABORT"
    PIPELINE_CONFIGURATION_ERROR = "PIPELINE_CONFIGURATION_ERROR"
    PIPELINE_DEPENDENCY_ERROR = "PIPELINE_DEPENDENCY_ERROR"
    STEP_NAME_DUPLICATE = "STEP_NAME_DUPLICATE"
    STEP_NOT_FOUND = "STEP_NOT_FOUND"
    STEP_NOT_REMOVABLE = "STEP_NOT_REMOVABLE"
    STEP_NOT_REPLACEABLE = "STEP_NOT_REPLACEABLE"
    STRATEGY_NOT_FOUND = "STRATEGY_NOT_FOUND"
    MODULE_ID_CONFLICT = "MODULE_ID_CONFLICT"
    INVALID_SEGMENT = "INVALID_SEGMENT"
    ID_TOO_LONG = "ID_TOO_LONG"
    MODULE_RELOAD_CONFLICT = "MODULE_RELOAD_CONFLICT"
    SYS_MODULE_REGISTRATION_FAILED = "SYS_MODULE_REGISTRATION_FAILED"
    SYS_MODULES_DISABLED = "SYS_MODULES_DISABLED"
    STREAMING_INTERFACE_MISMATCH = "STREAMING_INTERFACE_MISMATCH"
    CONTEXT_BINDING_ERROR = "CONTEXT_BINDING_ERROR"
    # W3C Trace Context (decision D-51): a `parent_id` override passed to
    # `TraceContext.inject()` that is not `^[0-9a-f]{16}$`. Cross-language:
    # TypeScript `code = 'INVALID_PARENT_ID'`, Rust `ErrorCode::InvalidParentId`.
    INVALID_PARENT_ID = "INVALID_PARENT_ID"

    # Note: this class is intentionally NOT instantiated. All callers access the
    # constants as class attributes (`ErrorCodes.MODULE_NOT_FOUND`). A previous
    # version defined `__setattr__` / `__delattr__` traps, but those only fire
    # on instance attribute mutation (`ErrorCodes().X = ...`) — never on class
    # attribute mutation (`ErrorCodes.X = ...`) — so the traps were cargo-cult
    # code that gave a false sense of immutability without actually enforcing
    # it. Removed in favor of simple class attributes; if real immutability is
    # ever needed, use `typing.Final[str]` annotations or a metaclass.


# =============================================================================
# Framework reserved error code prefixes (Algorithm A17)
# =============================================================================

# The canonical 14 reserved prefixes (A-D-006). CIRCUIT_, PIPELINE_,
# STREAMING_, and CONTEXT_ are intentionally NOT reserved as prefixes: the
# framework only owns the specific codes under them (e.g. CIRCUIT_BREAKER_OPEN,
# PIPELINE_STEP_ERROR, STREAMING_INTERFACE_MISMATCH, CONTEXT_BINDING_ERROR),
# which remain protected by the exact-code collision check against
# _FRAMEWORK_CODES. Reserving these as prefixes over-claimed the namespace and
# diverged from the TypeScript/Rust SDKs, blocking legitimate module codes such
# as STREAMING_CUSTOM.
FRAMEWORK_ERROR_CODE_PREFIXES: frozenset[str] = frozenset(
    {
        "ACL_",
        "APPROVAL_",
        "BINDING_",
        "CALL_",
        "CIRCULAR_",
        "CONFIG_",
        "DEPENDENCY_",
        "ERROR_CODE_",
        "FUNC_",
        "GENERAL_",
        "MIDDLEWARE_",
        "MODULE_",
        "SCHEMA_",
        "VERSION_",
    }
)


def _collect_framework_codes() -> frozenset[str]:
    """Collect all error codes defined on ``ErrorCodes``."""
    return frozenset(
        value for name, value in vars(ErrorCodes).items() if not name.startswith("_") and isinstance(value, str)
    )


_FRAMEWORK_CODES: frozenset[str] = _collect_framework_codes()


class ErrorCodeRegistry:
    """Registry for custom module error codes with collision detection (Algorithm A17).

    Detects conflicts between module custom error codes and framework reserved
    codes, as well as between modules.

    Thread-safe: all public methods are internally synchronized.
    """

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._module_codes: dict[str, frozenset[str]] = {}
        self._all_codes: set[str] = set(_FRAMEWORK_CODES)

    @property
    def all_codes(self) -> frozenset[str]:
        """Return all registered error codes (framework + module)."""
        with self._lock:
            return frozenset(self._all_codes)

    def register(self, module_id: str, codes: set[str]) -> None:
        """Register custom error codes for a module.

        Args:
            module_id: The module registering the codes.
            codes: Set of error code strings to register.

        Raises:
            ErrorCodeCollisionError: If any code collides with a framework
                code or a code already registered by another module.
        """
        if not codes:
            return

        with self._lock:
            for code in codes:
                # Check collision with framework reserved codes
                if code in _FRAMEWORK_CODES:
                    raise ErrorCodeCollisionError(
                        code=code,
                        module_id=module_id,
                        conflict_source="framework",
                    )
                # Check collision with other modules
                if code in self._all_codes:
                    owner = self._find_owner(code)
                    if owner != module_id:
                        raise ErrorCodeCollisionError(
                            code=code,
                            module_id=module_id,
                            conflict_source=owner or "unknown",
                        )

            # Also check prefix reservation
            for code in codes:
                for prefix in FRAMEWORK_ERROR_CODE_PREFIXES:
                    if code.startswith(prefix):
                        raise ErrorCodeCollisionError(
                            code=code,
                            module_id=module_id,
                            conflict_source=f"reserved prefix '{prefix}'",
                        )

            self._module_codes[module_id] = frozenset(codes)
            self._all_codes.update(codes)

    def unregister(self, module_id: str) -> None:
        """Remove all error codes registered by a module."""
        with self._lock:
            codes = self._module_codes.pop(module_id, frozenset())
            self._all_codes -= codes

    def _find_owner(self, code: str) -> str | None:
        """Find which module owns a given code."""
        for mid, codes in self._module_codes.items():
            if code in codes:
                return mid
        return None


class StreamingInterfaceError(ModuleError):
    """Raised when a module declares streaming=True but its stream() signature is wrong."""

    _default_retryable: bool | None = False

    def __init__(
        self,
        module_id: str,
        expected_signature: str,
        actual_signature: str,
        mismatch_reason: str,
    ) -> None:
        super().__init__(
            code="STREAMING_INTERFACE_MISMATCH",
            message=(
                f"Module {module_id!r} declared streaming but stream() does not "
                f"match the StreamingModule Protocol "
                f"(reason={mismatch_reason}; expected {expected_signature}, "
                f"got {actual_signature})"
            ),
        )
        self.module_id = module_id
        self.expected_signature = expected_signature
        self.actual_signature = actual_signature
        self.mismatch_reason = mismatch_reason


class ErrorCodeCollisionError(ModuleError):
    """Raised when a module error code collides with an existing code."""

    _default_retryable: bool | None = False

    def __init__(self, code: str, module_id: str, conflict_source: str, **kwargs: Any) -> None:
        super().__init__(
            code="ERROR_CODE_COLLISION",
            message=(f"Error code '{code}' from module '{module_id}' collides with {conflict_source}"),
            details={
                "error_code": code,
                "module_id": module_id,
                "conflict_source": conflict_source,
            },
            **kwargs,
        )
