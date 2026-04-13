# Error System

## Overview

The Error System provides a structured, hierarchical error model designed for both human developers and AI agents. Every error carries a unique code, a human-readable message, and optional AI guidance fields that enable self-healing agents to diagnose and recover from failures without human intervention. The system also includes an extensible error code registry and a formatter registry for surface-specific error rendering.

## Requirements

### Structured Error Hierarchy
- Provide a base `ModuleError` class that all framework errors extend.
- Every error **MUST** carry a `code` (string), `message` (string), and `timestamp` (ISO 8601 UTC).
- Errors **MUST** support optional fields: `details` (dict), `cause` (parent exception), `trace_id` (UUID), `retryable` (bool or null), `ai_guidance` (string), `user_fixable` (bool), and `suggestion` (string).
- Serialization via `to_dict()` / `toJSON()` **MUST** produce sparse output — null fields are omitted.

### AI Guidance Fields
- `retryable`: Whether the operation can be retried (null means unknown). Each error class **MAY** set a class-level default.
- `ai_guidance`: Free-text instructions for an AI agent attempting recovery.
- `user_fixable`: Whether the error can be resolved by the end-user without developer intervention.
- `suggestion`: A concrete next-step recommendation.

### Error Code Registry
- Provide an `ErrorCodeRegistry` for registering custom module-specific error codes at runtime.
- Framework error code prefixes (`MODULE_`, `SCHEMA_`, `ACL_`, `CONFIG_`, `GENERAL_`, `CIRCULAR_`, `APPROVAL_`, etc.) are reserved and **MUST NOT** be used by user modules.
- Duplicate code registration **MUST** raise `ErrorCodeCollisionError`.

### Error Formatters
- Support a formatter registry for surface-specific error rendering (e.g., MCP, A2A, CLI).
- Duplicate formatter registration **MUST** raise `ErrorFormatterDuplicateError`.

## Technical Design

### ModuleError (Base Class)

=== "Python"
    ```python
    from apcore.errors import ModuleError

    class ModuleError(Exception):
        def __init__(
            self,
            code: str,
            message: str,
            details: dict | None = None,
            cause: Exception | None = None,
            trace_id: str | None = None,
            retryable: bool | None = None,
            ai_guidance: str | None = None,
            user_fixable: bool | None = None,
            suggestion: str | None = None,
        ) -> None: ...

        def to_dict(self) -> dict:
            """Serialize to dict with sparse output (null fields omitted)"""
            ...
    ```
=== "TypeScript"
    ```typescript
    import { ModuleError } from "apcore-js";

    interface ErrorOptions {
        cause?: Error;
        traceId?: string;
        retryable?: boolean | null;
        aiGuidance?: string;
        userFixable?: boolean;
        suggestion?: string;
    }

    class ModuleError extends Error {
        readonly code: string;
        readonly details: Record<string, unknown> | null;
        readonly cause: Error | null;
        readonly traceId: string | null;
        readonly timestamp: string;
        readonly retryable: boolean | null;
        readonly aiGuidance: string | null;
        readonly userFixable: boolean | null;
        readonly suggestion: string | null;

        toJSON(): Record<string, unknown>;
    }
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;

    pub struct ModuleError {
        pub code: String,
        pub message: String,
        pub details: Option<serde_json::Value>,
        pub cause: Option<Box<dyn std::error::Error>>,
        pub trace_id: Option<String>,
        pub timestamp: String,
        pub retryable: Option<bool>,
        pub ai_guidance: Option<String>,
        pub user_fixable: Option<bool>,
        pub suggestion: Option<String>,
    }
    ```

### Error Hierarchy

The framework defines error subclasses grouped by domain. Each subclass sets an appropriate `code` and may override `_default_retryable`.

#### Configuration Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `ConfigNotFoundError` | `CONFIG_NOT_FOUND` | — | Config file not found |
| `ConfigError` | `CONFIG_INVALID` | — | Invalid configuration |
| `ConfigNamespaceDuplicateError` | `CONFIG_NAMESPACE_DUPLICATE` | — | Duplicate namespace registration |
| `ConfigNamespaceReservedError` | `CONFIG_NAMESPACE_RESERVED` | — | Reserved namespace name |
| `ConfigEnvPrefixConflictError` | `CONFIG_ENV_PREFIX_CONFLICT` | — | Environment variable prefix collision |
| `ConfigEnvMapConflictError` | `CONFIG_ENV_MAP_CONFLICT` | — | Environment variable already mapped |
| `ConfigMountError` | `CONFIG_MOUNT_ERROR` | — | Invalid mount operation |
| `ConfigBindError` | `CONFIG_BIND_ERROR` | — | Binding to model class fails |

#### Module Lifecycle Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `ModuleNotFoundError` | `MODULE_NOT_FOUND` | — | Module does not exist in registry |
| `ModuleDisabledError` | `MODULE_DISABLED` | — | Module is registered but disabled |
| `ModuleTimeoutError` | `MODULE_TIMEOUT` | — | Execution exceeded timeout |
| `ModuleLoadError` | `MODULE_LOAD_ERROR` | — | Module file cannot be loaded |
| `ModuleExecuteError` | `MODULE_EXECUTE_ERROR` | — | Unhandled failure during execution |
| `ReloadFailedError` | `RELOAD_FAILED` | Yes | Hot-reload failed (transient) |

#### Access Control Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `ACLRuleError` | `ACL_RULE_ERROR` | — | Invalid ACL rule definition |
| `ACLDeniedError` | `ACL_DENIED` | — | Access denied (carries `caller_id`, `target_id`) |

#### Approval Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `ApprovalError` | (base) | — | Base class for approval failures |
| `ApprovalDeniedError` | `APPROVAL_DENIED` | — | Handler rejected the request |
| `ApprovalTimeoutError` | `APPROVAL_TIMEOUT` | Yes | Approval timed out |
| `ApprovalPendingError` | `APPROVAL_PENDING` | — | Awaiting async resolution (carries `approval_id`) |

#### Schema & Validation Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `SchemaValidationError` | `SCHEMA_VALIDATION_ERROR` | — | Input/output validation failed (carries `errors` list) |
| `SchemaNotFoundError` | `SCHEMA_NOT_FOUND` | — | Schema file not found |
| `SchemaParseError` | `SCHEMA_PARSE_ERROR` | — | Invalid schema syntax |
| `SchemaCircularRefError` | `SCHEMA_CIRCULAR_REF` | — | Circular `$ref` detected |
| `InvalidInputError` | `GENERAL_INVALID_INPUT` | — | Invalid input data |

#### Call Chain Safety Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `CallDepthExceededError` | `CALL_DEPTH_EXCEEDED` | — | Exceeds max nesting depth (carries `current_depth`, `max_depth`) |
| `CircularCallError` | `CIRCULAR_CALL` | — | Circular call detected (carries `module_id`, `call_chain`) |
| `CallFrequencyExceededError` | `CALL_FREQUENCY_EXCEEDED` | — | Module called too many times (carries `module_id`, `count`, `max_repeat`) |

#### Binding Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `BindingInvalidTargetError` | `BINDING_INVALID_TARGET` | — | Target format is invalid |
| `BindingModuleNotFoundError` | `BINDING_MODULE_NOT_FOUND` | — | Cannot import target module |
| `BindingCallableNotFoundError` | `BINDING_CALLABLE_NOT_FOUND` | — | Callable not found in target module |
| `BindingNotCallableError` | `BINDING_NOT_CALLABLE` | — | Resolved target is not callable |
| `BindingSchemaMissingError` | `BINDING_SCHEMA_MISSING` | — | No schema and type hints fail |
| `BindingFileInvalidError` | `BINDING_FILE_INVALID` | — | Binding file has parse/format errors |

#### Middleware Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `MiddlewareChainError` | `MIDDLEWARE_CHAIN_ERROR` | — | Middleware chain failure (carries original exception and middleware list) |

#### Dependency & Internal Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `CircularDependencyError` | `CIRCULAR_DEPENDENCY` | — | Circular module dependencies (carries `cycle_path`) |
| `InternalError` | `GENERAL_INTERNAL_ERROR` | Yes | Unexpected framework error |
| `ExecutionCancelledError` | `EXECUTION_CANCELLED` | — | Cooperative cancellation triggered (defined in `cancel` module) |

#### Versioning & Registration Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `ErrorCodeCollisionError` | `ERROR_CODE_COLLISION` | — | Error code collides with existing registration |
| `VersionIncompatibleError` | `VERSION_INCOMPATIBLE` | — | Module declared version incompatible with SDK version |
| `ErrorFormatterDuplicateError` | `ERROR_FORMATTER_DUPLICATE` | — | Error formatter already registered for adapter name |
| `DependencyNotFoundError` | `DEPENDENCY_NOT_FOUND` | — | Required module dependency not found |

#### Type Annotation Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `FuncMissingTypeHintError` | `FUNC_MISSING_TYPE_HINT` | — | Function parameter lacks type annotation |
| `FuncMissingReturnTypeError` | `FUNC_MISSING_RETURN_TYPE` | — | Function lacks return type annotation |

### ErrorCodes Constants

All error codes are available as class-level constants on the `ErrorCodes` class:

```python
from apcore.errors import ErrorCodes

ErrorCodes.MODULE_NOT_FOUND      # "MODULE_NOT_FOUND"
ErrorCodes.ACL_DENIED            # "ACL_DENIED"
ErrorCodes.APPROVAL_PENDING      # "APPROVAL_PENDING"
ErrorCodes.SCHEMA_VALIDATION_ERROR  # "SCHEMA_VALIDATION_ERROR"
```

### ErrorCodeRegistry

The `ErrorCodeRegistry` enables modules to register custom error codes at runtime, with collision detection against both framework-reserved prefixes and other modules' codes.

=== "Python"
    ```python
    from apcore.errors import ErrorCodeRegistry

    registry = ErrorCodeRegistry()

    # Register custom codes for a module
    registry.register("payments.stripe", {"STRIPE_CARD_DECLINED", "STRIPE_RATE_LIMITED"})

    # Collision detection
    registry.register("other.module", {"STRIPE_CARD_DECLINED"})
    # Raises ErrorCodeCollisionError

    # Framework prefixes are reserved
    registry.register("my.module", {"MODULE_CUSTOM"})
    # Raises ErrorCodeCollisionError (MODULE_ is reserved)

    # Unregister
    registry.unregister("payments.stripe")

    # List all registered codes
    all_codes = registry.all_codes  # frozenset
    ```
=== "TypeScript"
    ```typescript
    import { ErrorCodeRegistry } from "apcore-js";

    const registry = new ErrorCodeRegistry();

    // Register custom codes for a module
    registry.register("payments.stripe", new Set(["STRIPE_CARD_DECLINED", "STRIPE_RATE_LIMITED"]));

    // Collision detection
    registry.register("other.module", new Set(["STRIPE_CARD_DECLINED"]));
    // Throws ErrorCodeCollisionError

    // Framework prefixes are reserved
    registry.register("my.module", new Set(["MODULE_CUSTOM"]));
    // Throws ErrorCodeCollisionError (MODULE_ is reserved)

    // Unregister
    registry.unregister("payments.stripe");

    // List all registered codes
    const allCodes = registry.allCodes; // ReadonlySet
    ```
=== "Rust"
    ```rust
    use apcore::errors::ErrorCodeRegistry;
    use std::collections::HashSet;

    let mut registry = ErrorCodeRegistry::new();

    // Register custom codes for a module
    let codes: HashSet<String> = ["STRIPE_CARD_DECLINED", "STRIPE_RATE_LIMITED"]
        .iter().map(|s| s.to_string()).collect();
    registry.register("payments.stripe", codes)?;

    // Unregister
    registry.unregister("payments.stripe");

    // List all registered codes
    let all_codes = registry.all_codes();
    ```

**Reserved framework error code prefixes:**

`MODULE_`, `SCHEMA_`, `ACL_`, `GENERAL_`, `CONFIG_`, `CIRCULAR_`, `DEPENDENCY_`, `CALL_`, `FUNC_`, `BINDING_`, `MIDDLEWARE_`, `APPROVAL_`, `VERSION_`, `ERROR_CODE_`

### ErrorFormatterRegistry

The `ErrorFormatterRegistry` enables adapters (MCP, OpenAI, etc.) to register custom error formatters that transform `ModuleError` instances into adapter-specific error responses.

=== "Python"
    ```python
    from apcore.errors import ErrorFormatterRegistry, ModuleError

    # Register a formatter for an adapter
    ErrorFormatterRegistry.register("mcp", lambda error, ctx: {
        "code": error.code,
        "message": error.message,
    })

    # Format an error for a specific adapter
    formatted = ErrorFormatterRegistry.format("mcp", some_error)

    # Get a registered formatter
    formatter = ErrorFormatterRegistry.get("mcp")
    ```
=== "TypeScript"
    ```typescript
    import { ErrorFormatterRegistry, ModuleError } from "apcore-js";

    // Register a formatter for an adapter
    ErrorFormatterRegistry.register("mcp", (error, ctx) => ({
        code: error.code,
        message: error.message,
    }));

    // Format an error for a specific adapter
    const formatted = ErrorFormatterRegistry.format("mcp", someError);

    // Get a registered formatter
    const formatter = ErrorFormatterRegistry.get("mcp");
    ```
=== "Rust"
    ```rust
    use apcore::errors::{ErrorFormatterRegistry, ModuleError};

    // Register a formatter for an adapter
    ErrorFormatterRegistry::register("mcp", Box::new(McpFormatter))?;

    // Format an error for a specific adapter
    let formatted = ErrorFormatterRegistry::format("mcp", &some_error, None);

    // Check if a formatter is registered
    let exists = ErrorFormatterRegistry::is_registered("mcp");
    ```

Registering a duplicate adapter name raises `ErrorFormatterDuplicateError`.

### Serialization

Errors serialize to sparse JSON — null/None fields are omitted:

```json
{
  "code": "ACL_DENIED",
  "message": "Access denied: api.user → executor.admin.reset",
  "timestamp": "2026-03-10T12:00:00Z",
  "trace_id": "a1b2c3d4-...",
  "details": {
    "caller_id": "api.user",
    "target_id": "executor.admin.reset"
  },
  "retryable": false,
  "ai_guidance": "The caller does not have permission. Add an ACL allow rule for this caller-target pair.",
  "suggestion": "Add an ACL rule: caller=api.user, target=executor.admin.reset, effect=allow"
}
```

## Dependencies

- The **Executor** raises errors from this hierarchy at each pipeline step.
- The **ACL System** uses `ACLDeniedError` and `ACLRuleError`.
- The **Approval System** uses `ApprovalDeniedError`, `ApprovalTimeoutError`, and `ApprovalPendingError`.
- The **Schema System** uses `SchemaValidationError`, `SchemaNotFoundError`, `SchemaParseError`, and `SchemaCircularRefError`.
- The **Call Chain Guard** uses `CallDepthExceededError`, `CircularCallError`, and `CallFrequencyExceededError`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/errors.py` | Full error hierarchy, ErrorCodes, ErrorCodeRegistry |

## Testing Strategy

- **Hierarchy tests** verify that all error subclasses inherit from `ModuleError` and carry the correct default `code`.
- **Serialization tests** confirm that `to_dict()` / `toJSON()` produces sparse output and includes all non-null fields.
- **AI guidance tests** verify that `retryable`, `ai_guidance`, `user_fixable`, and `suggestion` are preserved through serialization.
- **ErrorCodeRegistry tests** exercise registration, collision detection (both cross-module and framework-prefix), unregistration, and the `all_codes` aggregation.
- **Error-specific property tests** confirm that domain-specific properties (e.g., `ACLDeniedError.caller_id`, `CallDepthExceededError.max_depth`, `ApprovalPendingError.approval_id`) are accessible.
