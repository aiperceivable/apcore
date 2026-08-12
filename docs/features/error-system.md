---
description: "ModuleError hierarchy with code/message/timestamp and AI-guidance fields (retryable, ai_guidance, user_fixable, suggestion); reserved-prefix protection, code/formatter registry."
---

# Error System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §8 Error Handling Specification.


## Overview

The Error System provides a structured, hierarchical error model for human developers and automated callers. Every error carries a unique code, a human-readable message, and optional guidance fields that a caller may use to diagnose a failure or choose a recovery path. The system also includes an extensible error code registry and a formatter registry for surface-specific error rendering.

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

### Default `user_fixable` Policy (Resolved by Error Code)

Framework-deterministic errors **MUST** carry a default `user_fixable` resolved from the
error `code` at construction time, so the recovery contract is defined once at the source and
projected to every surface (MCP / CLI / A2A) without per-adapter backfilling. The value is part
of the cross-language contract and **MUST** be identical across SDKs — it is locked by the
conformance fixture `conformance/fixtures/error_recovery_metadata.json`.

- **`true`** — the caller can resolve it by changing the input or configuration they sent:
  `SCHEMA_VALIDATION_ERROR`, `GENERAL_INVALID_INPUT`, `MODULE_NOT_FOUND`,
  `VERSION_CONSTRAINT_INVALID`, `BINDING_SCHEMA_INFERENCE_FAILED`,
  `BINDING_SCHEMA_MODE_CONFLICT`, `BINDING_STRICT_SCHEMA_INCOMPATIBLE`,
  `DEPENDENCY_NOT_FOUND`, `DEPENDENCY_VERSION_MISMATCH`.
- **`false`** — governance / system / structural / transient, not resolvable by changing input:
  `ACL_DENIED`, `APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `MODULE_TIMEOUT`, `MODULE_DISABLED`,
  `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL`, `CALL_FREQUENCY_EXCEEDED`, `GENERAL_INTERNAL_ERROR`.
- **unset (`null`)** — any code not listed above, including `MODULE_EXECUTE_ERROR`
  (business-logic failures): the module author supplies `user_fixable` when raising.

An explicitly supplied `user_fixable` (including an explicit `null`) **MUST** override the
per-code default. `suggestion` is intentionally left to the module author (it overlaps with
`ai_guidance`); `x-*` metadata is likewise author-owned and not defaulted by the framework.

The policy table is the single source of truth for the value. Each SDK stores it as an
**internal** symbol (not part of the public API): Python's module-private `_USER_FIXABLE_BY_CODE`,
TypeScript's `@internal` `USER_FIXABLE_BY_CODE`, and Rust's `#[doc(hidden)]`
`user_fixable_for_code`.

=== "Python"
    ```python
    from apcore.errors import ModuleError

    err = ModuleError(code="SCHEMA_VALIDATION_ERROR", message="invalid email")
    assert err.user_fixable is True                 # default resolved from the code

    override = ModuleError(code="SCHEMA_VALIDATION_ERROR", message="...", user_fixable=None)
    assert override.user_fixable is None            # explicit value (incl. None) wins
    ```
=== "TypeScript"
    ```typescript
    import { ModuleError } from "apcore-js";

    const err = new ModuleError("SCHEMA_VALIDATION_ERROR", "invalid email");
    console.assert(err.userFixable === true);        // default resolved from the code

    // An explicit value via a subclass option (incl. null) wins over the per-code default.
    ```
=== "Rust"
    ```rust
    use apcore::errors::{ErrorCode, ModuleError};

    let err = ModuleError::new(ErrorCode::SchemaValidationError, "invalid email");
    assert_eq!(err.user_fixable, Some(true));        // default resolved from the code

    let override_ = err.with_user_fixable(false);    // explicit override
    assert_eq!(override_.user_fixable, Some(false));
    ```

> **Note (spec alignment, pending).** `docs/spec/protocol-spec.md` §8 currently states that
> `user_fixable` "defaults to `null` (omitted)". That remains true for unlisted codes, but is
> now incomplete for the framework-deterministic codes above — the protocol spec needs the
> matching qualification (tracked separately; protocol-spec edits require a linked issue + dual
> review).

### Error Code Registry
- Provide an `ErrorCodeRegistry` for registering custom module-specific error codes at runtime.
- Framework error code prefixes are reserved and **MUST NOT** be used by user modules. Reserved prefixes: `ACL_`, `APPROVAL_`, `BINDING_`, `CALL_`, `CIRCULAR_`, `CONFIG_`, `DEPENDENCY_`, `ERROR_CODE_`, `FUNC_`, `GENERAL_`, `MIDDLEWARE_`, `MODULE_`, `SCHEMA_`, `VERSION_`.
- Individual framework codes that do not fall under a reserved prefix (for example `CIRCUIT_BREAKER_OPEN`, `CONTEXT_BINDING_ERROR`, `STREAMING_INTERFACE_MISMATCH`, `STRATEGY_NOT_FOUND`, `PIPELINE_STEP_ERROR`, `STEP_NOT_FOUND`, `RELOAD_FAILED`, `EXECUTION_CANCELLED`, `TASK_LIMIT_EXCEEDED`) are protected by exact-code collision detection — `register()` **MUST** reject a custom code equal to any registered framework code, regardless of prefix.
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
| `SchemaCircularRefError` | `SCHEMA_CIRCULAR_REF` | — | A `$ref` → `$ref` chain that re-enters itself without reaching a schema body. A `$ref` re-entered *through* a body is a legal self-reference and does **not** raise (PROTOCOL_SPEC §4.15) |
| `InvalidInputError` | `GENERAL_INVALID_INPUT` | — | Invalid input data |

#### Call Chain Safety Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `CallDepthExceededError` | `CALL_DEPTH_EXCEEDED` | — | Exceeds max nesting depth (carries `current_depth`, `max_depth`) |
| `CircularCallError` | `CIRCULAR_CALL` | — | Circular call detected (carries `module_id`, `call_chain`) |
| `CallFrequencyExceededError` | `CALL_FREQUENCY_EXCEEDED` | — | Module called too many times (carries `module_id`, `count`, `max_repeat`) |

> **Cross-language note (idiomatic).** The "carries …" fields above always travel in the serialized `details` map (the normative wire contract). Python and TypeScript additionally expose them as typed convenience accessors on the error subclass (e.g. `err.current_depth` / `err.currentDepth`); Rust keeps the error struct flat and reads the same values from `err.details` (e.g. `err.details.get("current_depth")`). The data and wire form are identical — only the ergonomic accessor layer differs by language.

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

#### Streaming Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `StreamingInterfaceError` | `STREAMING_INTERFACE_MISMATCH` | — | A module declared streaming support but its `stream()` method does not satisfy the [`StreamingModule` interface](./streaming.md#streaming-module-interface-issue-62). Carries `module_id`, `expected_signature`, `actual_signature`, and `mismatch_reason` (enum: `wrong_arity` / `not_async` / `wrong_return_type` / `missing_marker`). Raised at module-load time, not at first call. |

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

#### Pipeline & Step Configuration Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `PipelineConfigurationError` | `PIPELINE_CONFIGURATION_ERROR` | — | The `pipeline:` config section names a step or anchor that does not exist, in `remove`, `configure`, or a `steps[].after` / `steps[].before` anchor. Raised at parse time (see `pipeline_failfast_config` fixture) |
| `PipelineConfigInvalidError` | `PIPELINE_CONFIG_INVALID` | — | A pipeline config **field** failed validation — a malformed value, not a missing step |
| `PipelineDependencyError` | `PIPELINE_DEPENDENCY_ERROR` | — | Step `requires` not satisfied by preceding `provides` declarations |
| `PipelineStepError` | `PIPELINE_STEP_ERROR` | — | Generic step-level execution failure raised by `StepMiddleware.on_step_error` |
| `PipelineStepNotFoundError` | `PIPELINE_STEP_NOT_FOUND` | — | `configure_step()` called directly on a strategy with an unknown step name |
| `StepNotFoundError` | `STEP_NOT_FOUND` | — | Any other direct strategy-API lookup miss: `remove()`, `insert_after()` / `insert_before()` anchor resolution |
| `StepNameDuplicateError` | `STEP_NAME_DUPLICATE` | — | Two steps registered under the same name |
| `StrategyNotFoundError` | `STRATEGY_NOT_FOUND` | — | Pipeline strategy preset name (e.g. `standard`, `minimal`) does not exist |

!!! note "Config layer vs strategy API"
    The same mistake — naming a step that does not exist — produces a **different** code depending on which layer you are in, and the distinction is deliberate:

    - Calling the strategy API directly (`strategy.remove("x")`, `configure_step("x")`, an `insert_after` anchor) raises `STEP_NOT_FOUND` / `PIPELINE_STEP_NOT_FOUND`. The caller is code, and the step name is a program value.
    - Going through the `pipeline:` config section re-classifies that miss as `PIPELINE_CONFIGURATION_ERROR`. The caller is a YAML file, and `STEP_NOT_FOUND` is too low-level to tell an operator which config key is wrong — the re-classified message names the section (`pipeline.configure:`, `pipeline.steps:`).

    All three SDKs implement the re-classification (apcore-rust `pipeline_config.rs:344`/`:358`, and the equivalent guards in apcore-python `pipeline_config.py` and apcore-typescript `pipeline-config.ts`). `PIPELINE_CONFIG_INVALID` is **not** part of this pair — it is a malformed-value code and MUST NOT be emitted for a missing step.

#### Schema Edge-Case Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `SchemaValidationFailedError` | `SCHEMA_VALIDATION_FAILED` | — | **Deprecated, retired alias — no longer emitted by any SDK.** All schema validation failures (raised errors and validate-result `error_code`) now use `SCHEMA_VALIDATION_ERROR`. Retained only for backward-compatible imports. |
| `SchemaMaxDepthExceededError` | `SCHEMA_MAX_DEPTH_EXCEEDED` | — | `$ref` resolution exceeded `schema.max_ref_depth` (default 32). Distinct from `SCHEMA_CIRCULAR_REF`: the chain is well-formed, just too deep. Depth is consumed by `$ref` hops only, not by structural descent |
| `SchemaValidationError` | `SCHEMA_UNION_NO_MATCH` | — | Value matched no branch of a `oneOf` / `anyOf` union. Carried as the error's `error_code`; pinned by `conformance/fixtures/schema_hardening_union.json` |
| `SchemaValidationError` | `SCHEMA_UNION_AMBIGUOUS` | — | Value matched more than one branch of a `oneOf` union, which **MUST** be exclusive. Carried as the error's `error_code`; pinned by `conformance/fixtures/schema_hardening_union.json` |

#### Module Registration Conflict Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `ModuleIdConflictError` | `MODULE_ID_CONFLICT` | — | Two modules resolved to the same Canonical ID during discovery |
| `ModuleReloadConflictError` | `MODULE_RELOAD_CONFLICT` | Yes | Hot-reload skipped because the module is in flight; safe to retry |
| `SystemModuleRegistrationFailedError` | `SYS_MODULE_REGISTRATION_FAILED` | — | A `sys.*` module failed to register at framework startup |

> **Cross-language ergonomic note (D1-002 — intentional, language-idiomatic).**
> Re-registering an already-registered module ID raises the error **code**
> `DUPLICATE_MODULE_ID` (with `details.module_id` populated, `retryable=false`)
> in all three SDKs — that wire-observable contract is identical. Only the
> *typed wrapper* differs by language idiom, and a dedicated class is **not**
> required by the protocol (`protocol-spec.md` §8.7 does not list one; this
> section mandates only the code):
>
> - **TypeScript** ships a dedicated `DuplicateModuleIdError extends ModuleError`.
>   TS's idiom is one subclass per code, so a class here is consistent with its
>   own surface.
> - **Rust** raises `ModuleError::duplicate_module_id()` — a named builder on the
>   single `ModuleError` struct keyed by `ErrorCode::DuplicateModuleId`. Rust has
>   no per-code structs; the builder is its idiomatic equivalent.
> - **Python** raises the generic `InvalidInputError(code=DUPLICATE_MODULE_ID)`.
>   A dedicated subclass MAY be added for symmetry with Python's other typed
>   classes, but is not required.
>
> Because every SDK exposes the same `code` + `details`, cross-language fixtures
> and the MCP/A2A error mappers behave identically. This is a **language-surface
> divergence**, not a parity bug.

#### Binding Inference Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `BindingSchemaInferenceFailedError` | `BINDING_SCHEMA_INFERENCE_FAILED` | — | Auto-schema inference from type hints failed (missing/unsupported types) |
| `BindingSchemaModeConflictError` | `BINDING_SCHEMA_MODE_CONFLICT` | — | Binding declares both an inline schema and an external schema reference |

#### Async Task Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `TaskLimitExceededError` | `TASK_LIMIT_EXCEEDED` | Yes | `AsyncTaskManager` concurrent-task ceiling reached |

#### Versioning Errors

| Error Class | Code | Retryable | Description |
|---|---|---|---|
| `VersionConstraintInvalidError` | `VERSION_CONSTRAINT_INVALID` | — | Dependency version constraint string is malformed (semver/range parse failure) |

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

`ACL_`, `APPROVAL_`, `BINDING_`, `CALL_`, `CIRCULAR_`, `CONFIG_`, `DEPENDENCY_`, `ERROR_CODE_`, `FUNC_`, `GENERAL_`, `MIDDLEWARE_`, `MODULE_`, `SCHEMA_`, `VERSION_`

This is the single canonical set (identical to the reserved-prefix list in the Error Code Registry requirements above). One-off framework codes outside these prefixes (e.g. `CIRCUIT_BREAKER_OPEN`, `CONTEXT_BINDING_ERROR`, `STREAMING_INTERFACE_MISMATCH`, `STRATEGY_NOT_FOUND`, `PIPELINE_*`, `STEP_*`, `RELOAD_FAILED`, `EXECUTION_CANCELLED`, `TASK_LIMIT_EXCEEDED`) are not prefix-reserved; they are protected by exact-code collision detection in `register()`.

### ErrorFormatterRegistry

The `ErrorFormatterRegistry` enables adapters (MCP, OpenAI, etc.) to register custom error formatters that transform `ModuleError` instances into adapter-specific error responses.

=== "Python"
    ```python
    from apcore import ErrorFormatterRegistry, ModuleError

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

## Contract: ModuleError.to_dict

### Inputs
- No inputs

### Errors
- No errors raised

### Returns
- On success: `dict`/`Record<string, unknown>`/`serde_json::Value` — serialized error with guaranteed keys: `code` (string), `message` (string), `ai_guidance` (string), plus any optional keys (`context`, `details`, `source`, `timestamp`)

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

## Invariants: ModuleError

The following invariants MUST hold for every `ModuleError` instance across all language implementations:
- `code` MUST be a non-empty string from the registered error-code registry
- `message` MUST be a non-empty human-readable string
- `ai_guidance` MUST be a non-empty string with actionable recovery guidance for AI agents
- Error code constants `INVALID_MODULE_ID` and `DUPLICATE_MODULE_ID` MUST be defined and used where the spec declares them — see `docs/features/core-executor.md` §Contract and `docs/features/registry-system.md` §Contract
