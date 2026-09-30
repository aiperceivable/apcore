---
description: "ModuleError model with code/message/timestamp, optional AI-guidance fields (retryable, ai_guidance, user_fixable, suggestion), per-code defaults, error code and formatter registries."
---

# Error System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §8 Error Handling Specification.


## Overview

The Error System provides a structured error model for human developers and automated callers. Every error carries a code, a human-readable message and a timestamp, plus optional guidance fields that a caller may use to diagnose a failure or choose a recovery path. The system also includes an extensible error code registry and a formatter registry for surface-specific error rendering.

## Requirements

### Structured Errors
- Provide a base `ModuleError` that all framework errors extend (Python, TypeScript) or are instances of (Rust).
- Every error **MUST** carry a `code` (string), `message` (string), and `timestamp` (ISO 8601 UTC).
- Errors **MUST** support optional fields: `details` (map), `cause`, `trace_id` (32-character lowercase hex, W3C), `retryable` (bool or null), `ai_guidance` (string), `user_fixable` (bool or null), and `suggestion` (string).
- Serialization via `to_dict()` (Python, Rust) / `toJSON()` (TypeScript) **MUST** produce sparse output — null fields are omitted.

### AI Guidance Fields
- `retryable`: Whether the same call may succeed if retried (null means unknown). Defaults per code — see [Default `retryable`](#default-retryable).
- `ai_guidance`: Free-text instructions for an AI agent attempting recovery. Optional; many framework errors supply a default text, and it is omitted from the serialized form when unset.
- `user_fixable`: Whether the caller can resolve the error without developer intervention. Defaults per code — see below.
- `suggestion`: A concrete next-step recommendation. Always left to the error's author.

### Default `user_fixable` Policy (Resolved by Error Code)

Framework-deterministic errors carry a default `user_fixable` resolved from the error `code` at
construction time, so the recovery contract is defined once at the source and projected to every
surface (MCP / CLI / A2A) without per-adapter backfilling. The value is identical across SDKs and is
locked by the conformance fixture `conformance/fixtures/error_recovery_metadata.json`.

- **`true`** — the caller can resolve it by changing the input or configuration they sent:
  `SCHEMA_VALIDATION_ERROR`, `GENERAL_INVALID_INPUT`, `MODULE_NOT_FOUND`,
  `VERSION_CONSTRAINT_INVALID`, `BINDING_SCHEMA_INFERENCE_FAILED`,
  `BINDING_SCHEMA_MODE_CONFLICT`, `BINDING_STRICT_SCHEMA_INCOMPATIBLE`,
  `DEPENDENCY_NOT_FOUND`, `DEPENDENCY_VERSION_MISMATCH`, `INVALID_PARENT_ID`.
- **`false`** — governance / system / structural / transient, not resolvable by changing input:
  `ACL_DENIED`, `APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `MODULE_TIMEOUT`, `MODULE_DISABLED`,
  `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL`, `CALL_FREQUENCY_EXCEEDED`, `GENERAL_INTERNAL_ERROR`.
- **unset (`null`)** — any code not listed above, including `MODULE_EXECUTE_ERROR`
  (business-logic failures): the module author supplies `user_fixable` when raising.

`INVALID_PARENT_ID` gets its `true` from Rust's code table and from Python's `InvalidParentIdError`;
TypeScript raises it as a plain `Error` with `code` set, which carries no `user_fixable`.
protocol-spec §8.1.1 still gives `null` as the default for every code; the per-code defaults above
are what all three SDKs implement.

An explicitly supplied `user_fixable` (including an explicit `null` in Python and TypeScript)
overrides the per-code default. `suggestion` is intentionally left to the module author (it overlaps
with `ai_guidance`); `x-*` metadata is likewise author-owned and not defaulted by the framework.
Each SDK stores the table as an internal symbol, not part of the public API.

=== "Python"
    ```python
    from apcore import ModuleError

    err = ModuleError(code="SCHEMA_VALIDATION_ERROR", message="invalid email")
    assert err.user_fixable is True                 # default resolved from the code

    override = ModuleError(code="SCHEMA_VALIDATION_ERROR", message="...", user_fixable=None)
    assert override.user_fixable is None            # explicit value (incl. None) wins
    ```
=== "TypeScript"
    ```typescript
    import { ModuleError } from "apcore-js";

    const err = new ModuleError("SCHEMA_VALIDATION_ERROR", "invalid email");
    console.assert(err.userFixable === true); // default resolved from the code

    // Positional: code, message, details, cause, traceId, retryable, aiGuidance, userFixable
    const override = new ModuleError(
        "SCHEMA_VALIDATION_ERROR", "...", undefined, undefined, undefined, undefined, undefined, null,
    );
    console.assert(override.userFixable === null); // explicit value (incl. null) wins
    ```
=== "Rust"
    ```rust
    use apcore::{ErrorCode, ModuleError};

    fn main() {
        let err = ModuleError::new(ErrorCode::SchemaValidationError, "invalid email");
        assert_eq!(err.user_fixable, Some(true)); // default resolved from the code

        let override_ = err.with_user_fixable(false); // explicit override
        assert_eq!(override_.user_fixable, Some(false));
    }
    ```

### Error Code Registry
- Provide an `ErrorCodeRegistry` for registering custom module-specific error codes at runtime.
- Framework error code prefixes are reserved and **MUST NOT** be used by user modules. Reserved prefixes: `ACL_`, `APPROVAL_`, `BINDING_`, `CALL_`, `CIRCULAR_`, `CONFIG_`, `DEPENDENCY_`, `ERROR_CODE_`, `FUNC_`, `GENERAL_`, `MIDDLEWARE_`, `MODULE_`, `SCHEMA_`, `VERSION_`.
- Individual framework codes that do not fall under a reserved prefix (for example `CIRCUIT_BREAKER_OPEN`, `CONTEXT_BINDING_ERROR`, `STREAMING_INTERFACE_MISMATCH`, `STRATEGY_NOT_FOUND`, `PIPELINE_STEP_ERROR`, `STEP_NOT_FOUND`, `RELOAD_FAILED`, `EXECUTION_CANCELLED`, `TASK_LIMIT_EXCEEDED`) are protected by exact-code collision detection — `register()` **MUST** reject a custom code equal to any framework code, regardless of prefix.
- Duplicate code registration **MUST** raise `ErrorCodeCollisionError`.

### Error Formatters
- Support a formatter registry for surface-specific error rendering (e.g., MCP, A2A, CLI).
- Duplicate formatter registration **MUST** raise `ErrorFormatterDuplicateError`.

## Technical Design

### ModuleError

=== "Python"
    ```python
    from typing import Any


    class ModuleError(Exception):
        code: str
        message: str
        details: dict[str, Any]          # {} when not supplied
        cause: Exception | None
        trace_id: str | None
        timestamp: str                   # ISO 8601 UTC, set at construction
        retryable: bool | None           # default from the subclass (see below)
        ai_guidance: str | None
        user_fixable: bool | None        # default from the code (see above)
        suggestion: str | None

        def __init__(
            self,
            code: str,
            message: str,
            details: dict[str, Any] | None = None,
            cause: Exception | None = None,
            trace_id: str | None = None,
            retryable: bool | None = ...,     # omitted -> subclass default
            ai_guidance: str | None = None,
            user_fixable: bool | None = ...,  # omitted -> per-code default
            suggestion: str | None = None,
        ) -> None: ...

        def to_dict(self) -> dict[str, Any]:
            """Sparse serialization (null fields omitted)."""
    ```
=== "TypeScript"
    ```typescript
    class ModuleError extends Error {
        readonly code: string;
        readonly details: Record<string, unknown>; // {} when not supplied
        readonly cause?: Error;
        readonly traceId?: string;
        readonly timestamp: string;                // ISO 8601 UTC
        readonly retryable: boolean | null;        // default from the subclass
        readonly aiGuidance: string | null;
        readonly userFixable: boolean | null;      // default from the code
        readonly suggestion: string | null;

        constructor(
            code: string,
            message: string,
            details?: Record<string, unknown>,
            cause?: Error,
            traceId?: string,
            retryable?: boolean | null,
            aiGuidance?: string | null,
            userFixable?: boolean | null,
            suggestion?: string | null,
        );

        toJSON(): Record<string, unknown>; // sparse, snake_case keys
    }
    ```
=== "Rust"
    ```rust
    use std::collections::HashMap;

    use apcore::ErrorCode;
    use chrono::{DateTime, Utc};

    pub struct ModuleError {
        pub code: ErrorCode,                               // serialized as "SCREAMING_SNAKE_CASE"
        pub message: String,
        pub details: HashMap<String, serde_json::Value>,   // omitted when empty
        pub cause: Option<String>,
        pub trace_id: Option<String>,
        pub timestamp: DateTime<Utc>,
        pub retryable: Option<bool>,                       // default from the code
        pub ai_guidance: Option<String>,
        pub user_fixable: Option<bool>,                    // default from the code
        pub suggestion: Option<String>,
    }
    // ModuleError::new(code, message), then with_details / with_cause / with_trace_id /
    // with_retryable / with_ai_guidance / with_user_fixable / with_suggestion; to_dict().
    ```

Python and TypeScript define one subclass per error (tables below). Rust has a single `ModuleError` struct: the code is an `ErrorCode` variant (for example `ErrorCode::ModuleNotFound`), and common errors have named builders (`ModuleError::invalid_input`, `ModuleError::module_timeout`, `ModuleError::circuit_breaker_open`, `ModuleError::task_store_unavailable`, …). The Rust exceptions are `ExecutionCancelledError` (converts into `ModuleError` via `From`), `VersionIncompatibleError` and `ErrorCodeCollisionError` (convert via `to_module_error()`).

Structured data such as `caller_id` or `call_chain` always travels in `details` (the wire contract). Python and TypeScript subclasses additionally expose some of it as typed accessors (e.g. `err.caller_id` / `err.callerId`); in Rust read `err.details`.

### Error Catalogue

Class names are the Python / TypeScript names; where the two differ, both are given.

#### Configuration Errors

| Code | Class | Description |
|---|---|---|
| `CONFIG_NOT_FOUND` | `ConfigNotFoundError` | Config file not found |
| `CONFIG_INVALID` | `ConfigError` | Invalid configuration |
| `CONFIG_NAMESPACE_DUPLICATE` | `ConfigNamespaceDuplicateError` | Duplicate namespace registration |
| `CONFIG_NAMESPACE_RESERVED` | `ConfigNamespaceReservedError` | Reserved namespace name |
| `CONFIG_ENV_PREFIX_CONFLICT` | `ConfigEnvPrefixConflictError` | Environment variable prefix collision |
| `CONFIG_ENV_MAP_CONFLICT` | `ConfigEnvMapConflictError` | Environment variable already mapped |
| `CONFIG_MOUNT_ERROR` | `ConfigMountError` | Invalid mount operation |
| `CONFIG_BIND_ERROR` | `ConfigBindError` | Binding to model class fails |
| `CONFIG_KEY_RESTRICTED` | `ModuleError` | `system.control.update_config` was asked to change a restricted key |

#### Module Lifecycle Errors

| Code | Class | Description |
|---|---|---|
| `MODULE_NOT_FOUND` | `ModuleNotFoundError` | Module does not exist in registry |
| `MODULE_DISABLED` | `ModuleDisabledError` | Module is registered but disabled |
| `MODULE_TIMEOUT` | `ModuleTimeoutError` | Execution exceeded timeout |
| `MODULE_LOAD_ERROR` | `ModuleLoadError` | Module file cannot be loaded |
| `MODULE_EXECUTE_ERROR` | `ModuleExecuteError` | Unhandled failure during execution |
| `RELOAD_FAILED` | `ReloadFailedError` | Hot-reload failed |
| `EXECUTION_CANCELLED` | `ExecutionCancelledError` | The call's `CancelToken` was cancelled — see [Cancellation](./cancellation.md) |
| `GENERAL_NOT_IMPLEMENTED` | — | Defined in all three SDKs; raised by Rust `Executor::stream()` when the module does not implement `stream()` |

#### Module ID & Registration Errors

| Code | Class | Description |
|---|---|---|
| `INVALID_MODULE_ID` | `InvalidInputError` (code set) | Module ID is empty or does not match the canonical ID grammar (registry `register()`, executor `call()`) |
| `DUPLICATE_MODULE_ID` | Python `InvalidInputError` (code set) / TS `DuplicateModuleIdError` | An already-registered module ID was registered again; `details.module_id` is populated |
| `MODULE_ID_CONFLICT` | `ModuleIdConflictError` | Multi-class discovery derived the same ID for two classes in one file |
| `INVALID_SEGMENT` | `InvalidSegmentError` | A derived class segment does not match the canonical ID grammar |
| `ID_TOO_LONG` | `IdTooLongError` | A derived module ID exceeds 192 characters |
| `MODULE_RELOAD_CONFLICT` | `ModuleReloadConflictError` | Hot-reload skipped because the module is in flight |
| `SYS_MODULE_REGISTRATION_FAILED` | `SysModuleRegistrationError` | A `system.*` module failed to register at startup |
| `SYS_MODULES_DISABLED` | `SysModulesDisabledError` | An `APCore` method that needs system modules was called while `sys_modules.enabled` is false |

#### Access Control Errors

| Code | Class | Description |
|---|---|---|
| `ACL_RULE_ERROR` | `ACLRuleError` | Invalid ACL rule definition |
| `ACL_DENIED` | `ACLDeniedError` | Access denied (`details`: `caller_id`, `target_id`) |

#### Approval Errors

| Code | Class | Description |
|---|---|---|
| — | `ApprovalError` | Base class for the three below |
| `APPROVAL_DENIED` | `ApprovalDeniedError` | Handler rejected the request |
| `APPROVAL_TIMEOUT` | `ApprovalTimeoutError` | Handler returned a timeout status |
| `APPROVAL_PENDING` | `ApprovalPendingError` | Awaiting async resolution (carries `approval_id`) |

#### Schema & Validation Errors

| Code | Class | Description |
|---|---|---|
| `SCHEMA_VALIDATION_ERROR` | `SchemaValidationError` | Input/output validation failed (carries `errors` list) |
| `SCHEMA_UNION_NO_MATCH` | `SchemaValidationError` (per-field `error_code`) | Value matched no branch of a `oneOf` / `anyOf` union |
| `SCHEMA_UNION_AMBIGUOUS` | `SchemaValidationError` (per-field `error_code`) | Value matched more than one branch of a `oneOf` union, which **MUST** be exclusive |
| `SCHEMA_NOT_FOUND` | `SchemaNotFoundError` | Schema file not found |
| `SCHEMA_PARSE_ERROR` | `SchemaParseError` | Invalid schema syntax |
| `SCHEMA_CIRCULAR_REF` | `SchemaCircularRefError` | A `$ref` → `$ref` chain that re-enters itself without reaching a schema body. A `$ref` re-entered *through* a body is a legal self-reference and does **not** raise (PROTOCOL_SPEC §4.15) |
| `SCHEMA_MAX_DEPTH_EXCEEDED` | `SchemaMaxDepthExceededError` | `$ref` resolution exceeded `schema.max_ref_depth` (default 32). Depth is consumed by `$ref` hops only, not by structural descent |
| `GENERAL_INVALID_INPUT` | `InvalidInputError` | Invalid input data |
| `INVALID_PARENT_ID` | Python `InvalidParentIdError` / TS `Error` with `code` | `parent_id` passed to `TraceContext.inject()` is not 16 lowercase hex characters |

#### Call Chain Safety Errors

| Code | Class | Description |
|---|---|---|
| `CALL_DEPTH_EXCEEDED` | `CallDepthExceededError` | Exceeds max nesting depth (`details`: `depth`, `max_depth`, `call_chain`) |
| `CIRCULAR_CALL` | `CircularCallError` | Circular call detected (`details`: `module_id`, `call_chain`) |
| `CALL_FREQUENCY_EXCEEDED` | `CallFrequencyExceededError` | Module called too many times (`details`: `module_id`, `count`, `max_repeat`, `call_chain`) |

See [Call Chain Guard](./call-chain-guard.md) for when each is raised.

#### Binding Errors

| Code | Class | Description |
|---|---|---|
| `BINDING_INVALID_TARGET` | `BindingInvalidTargetError` | Target format is invalid |
| `BINDING_MODULE_NOT_FOUND` | `BindingModuleNotFoundError` | Cannot import target module |
| `BINDING_CALLABLE_NOT_FOUND` | `BindingCallableNotFoundError` | Callable not found in target module |
| `BINDING_NOT_CALLABLE` | `BindingNotCallableError` | Resolved target is not callable |
| `BINDING_FILE_INVALID` | `BindingFileInvalidError` | Binding file has parse/format errors |
| `BINDING_SCHEMA_INFERENCE_FAILED` | `BindingSchemaInferenceFailedError` | Auto-schema inference from type hints failed (missing/unsupported types) |
| `BINDING_SCHEMA_MODE_CONFLICT` | `BindingSchemaModeConflictError` | Binding declares both an inline schema and an external schema reference |
| `BINDING_STRICT_SCHEMA_INCOMPATIBLE` | `BindingStrictSchemaIncompatibleError` | `auto_schema: strict` was requested but the inferred schema has incompatible features |

#### Type Annotation Errors

| Code | Class | Description |
|---|---|---|
| `FUNC_MISSING_TYPE_HINT` | `FuncMissingTypeHintError` | Function parameter lacks type annotation |
| `FUNC_MISSING_RETURN_TYPE` | `FuncMissingReturnTypeError` | Function lacks return type annotation |

#### Middleware, Streaming & Context Errors

| Code | Class | Description |
|---|---|---|
| `MIDDLEWARE_CHAIN_ERROR` | `MiddlewareChainError` | Middleware chain failure (carries original exception and middleware list) |
| `CIRCUIT_BREAKER_OPEN` | `CircuitBreakerOpenError` | `CircuitBreakerMiddleware` rejected the call because the circuit is open (`details`: `module_id`, `caller_id`) |
| `STREAMING_INTERFACE_MISMATCH` | `StreamingInterfaceError` | A module declared streaming support but its `stream()` method does not satisfy the [`StreamingModule` interface](./streaming.md). Carries `module_id`, `expected_signature`, `actual_signature`, and `mismatch_reason` (`wrong_arity` / `not_async` / `wrong_return_type` / `missing_marker`). Raised at module-load time |
| `CONTEXT_BINDING_ERROR` | `ContextBindingError` | An Executor tried to bind to a Context already bound to a different Executor |

#### Dependency, Versioning & Internal Errors

| Code | Class | Description |
|---|---|---|
| `DEPENDENCY_NOT_FOUND` | `DependencyNotFoundError` | Required module dependency not found |
| `DEPENDENCY_VERSION_MISMATCH` | `DependencyVersionMismatchError` | A declared dependency's version constraint is not satisfied |
| `CIRCULAR_DEPENDENCY` | `CircularDependencyError` | Circular module dependencies (carries `cycle_path`) |
| `VERSION_CONSTRAINT_INVALID` | `VersionConstraintError` | Dependency version constraint string is malformed |
| `VERSION_INCOMPATIBLE` | `VersionIncompatibleError` | Module declared version incompatible with SDK version |
| `ERROR_CODE_COLLISION` | `ErrorCodeCollisionError` | Error code collides with an existing registration |
| `ERROR_FORMATTER_DUPLICATE` | `ErrorFormatterDuplicateError` | Error formatter already registered for adapter name |
| `GENERAL_INTERNAL_ERROR` | `InternalError` | Unexpected framework error |

#### Async Task Errors

| Code | Class | Description |
|---|---|---|
| `TASK_LIMIT_EXCEEDED` | `TaskLimitExceededError` | `AsyncTaskManager` task ceiling reached |
| `TASK_STORE_UNAVAILABLE` | `TaskStoreError` | A `TaskStore` backend is unreachable or refused an operation (`details`: `operation`, `reason`) |
| `REAPER_ALREADY_RUNNING` | `ModuleError` | `start_reaper()` called while a reaper is already running |

See [Async Tasks](./async-tasks.md).

#### Pipeline & Step Configuration Errors

| Code | Class | Description |
|---|---|---|
| `PIPELINE_CONFIGURATION_ERROR` | `ConfigurationError` | The `pipeline:` config section names a step or anchor that does not exist, in `remove`, `configure`, or a `steps[].after` / `steps[].before` anchor. Raised at parse time |
| `PIPELINE_DEPENDENCY_ERROR` | `PipelineDependencyError` | Step `requires` not satisfied by preceding `provides` declarations |
| `PIPELINE_STEP_ERROR` | `PipelineStepError` | Generic step-level execution failure raised by `StepMiddleware.on_step_error` |
| `PIPELINE_ABORT` | `PipelineAbortError` | A step aborted the pipeline |
| `PIPELINE_STEP_NOT_FOUND` | `PipelineStepNotFoundError` | `configure_step()` called directly on a strategy with an unknown step name |
| `STEP_NOT_FOUND` | `StepNotFoundError` | Any other direct strategy-API lookup miss: `remove()`, `insert_after()` / `insert_before()` anchor resolution |
| `STEP_NOT_REMOVABLE` | `StepNotRemovableError` | `remove()` on a step marked non-removable |
| `STEP_NOT_REPLACEABLE` | `StepNotReplaceableError` | `replace()` on a step marked non-replaceable |
| `STEP_NAME_DUPLICATE` | `StepNameDuplicateError` | Two steps registered under the same name |
| `STRATEGY_NOT_FOUND` | `StrategyNotFoundError` | Pipeline strategy preset name (e.g. `standard`, `minimal`) does not exist |

!!! note "Config layer vs strategy API"
    The same mistake — naming a step that does not exist — produces a **different** code depending on which layer you are in, and the distinction is deliberate:

    - Calling the strategy API directly (`strategy.remove("x")`, `configure_step("x")`, an `insert_after` anchor) raises `STEP_NOT_FOUND` / `PIPELINE_STEP_NOT_FOUND`. The caller is code, and the step name is a program value.
    - Going through the `pipeline:` config section re-classifies that miss as `PIPELINE_CONFIGURATION_ERROR`. The caller is a YAML file, and `STEP_NOT_FOUND` is too low-level to tell an operator which config key is wrong — the re-classified message names the section (`pipeline.configure:`, `pipeline.steps:`).

    Rust additionally defines `PIPELINE_CONFIG_INVALID` for a malformed config **value** (not a missing step); Python and TypeScript report those as `PIPELINE_CONFIGURATION_ERROR`.

### Default `retryable`

`retryable` defaults from the error class (Python, TypeScript) or code (Rust):

- **`true`** — `APPROVAL_TIMEOUT`, `MODULE_TIMEOUT`, `GENERAL_INTERNAL_ERROR`, `RELOAD_FAILED`, `TASK_LIMIT_EXCEEDED`, `TASK_STORE_UNAVAILABLE`, and `CIRCUIT_BREAKER_OPEN` (Python and Rust; TypeScript's `CircuitBreakerOpenError` defaults to `false`).
- **unset (`null`)** — `MODULE_EXECUTE_ERROR` and `EXECUTION_CANCELLED` in all three SDKs, so the raiser decides. Python also leaves the pipeline/step codes unset, and Rust leaves unset every code it does not list in its code table (pipeline, registration and ID-derivation codes among them).
- **`false`** — every other framework error class.

The normative per-code table is protocol-spec §8.6. An explicit `retryable` on the instance always wins.

### ErrorCodes Constants

All framework codes are available as constants: `ErrorCodes.MODULE_NOT_FOUND` in Python and TypeScript, `ErrorCode::ModuleNotFound` in Rust.

=== "Python"
    ```python
    from apcore import ErrorCodes

    assert ErrorCodes.MODULE_NOT_FOUND == "MODULE_NOT_FOUND"
    assert ErrorCodes.APPROVAL_PENDING == "APPROVAL_PENDING"
    ```
=== "TypeScript"
    ```typescript
    import { ErrorCodes } from "apcore-js";

    console.assert(ErrorCodes.MODULE_NOT_FOUND === "MODULE_NOT_FOUND");
    console.assert(ErrorCodes.APPROVAL_PENDING === "APPROVAL_PENDING");
    ```
=== "Rust"
    ```rust
    use apcore::ErrorCode;

    fn main() {
        let code = ErrorCode::ModuleNotFound;
        assert_eq!(serde_json::to_value(code).unwrap(), "MODULE_NOT_FOUND");
    }
    ```

### ErrorCodeRegistry

The `ErrorCodeRegistry` enables modules to register custom error codes at runtime, with collision detection against framework codes, reserved prefixes, and other modules' codes.

=== "Python"
    ```python
    from apcore import ErrorCodeCollisionError, ErrorCodeRegistry

    registry = ErrorCodeRegistry()

    # Register custom codes for a module
    registry.register("payments.stripe", {"STRIPE_CARD_DECLINED", "STRIPE_RATE_LIMITED"})

    # Collision with another module's code
    try:
        registry.register("other.module", {"STRIPE_CARD_DECLINED"})
    except ErrorCodeCollisionError:
        pass

    # Framework prefixes are reserved
    try:
        registry.register("my.module", {"MODULE_CUSTOM"})
    except ErrorCodeCollisionError:
        pass

    # Unregister
    registry.unregister("payments.stripe")

    # All registered codes (framework + custom)
    all_codes = registry.all_codes  # frozenset[str]
    ```
=== "TypeScript"
    ```typescript
    import { ErrorCodeCollisionError, ErrorCodeRegistry } from "apcore-js";

    const registry = new ErrorCodeRegistry();

    // Register custom codes for a module
    registry.register("payments.stripe", new Set(["STRIPE_CARD_DECLINED", "STRIPE_RATE_LIMITED"]));

    // Collision with another module's code
    try {
        registry.register("other.module", new Set(["STRIPE_CARD_DECLINED"]));
    } catch (e) {
        console.assert(e instanceof ErrorCodeCollisionError);
    }

    // Framework prefixes are reserved
    try {
        registry.register("my.module", new Set(["MODULE_CUSTOM"]));
    } catch (e) {
        console.assert(e instanceof ErrorCodeCollisionError);
    }

    // Unregister
    registry.unregister("payments.stripe");

    // All registered codes (framework + custom)
    const allCodes = registry.allCodes; // ReadonlySet<string>
    ```
=== "Rust"
    ```rust
    use std::collections::HashSet;

    use apcore::{ErrorCode, ErrorCodeRegistry, ModuleError};

    fn main() -> Result<(), ModuleError> {
        let mut registry = ErrorCodeRegistry::new();

        // Register custom codes for a module
        let codes: HashSet<String> = ["STRIPE_CARD_DECLINED", "STRIPE_RATE_LIMITED"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        registry.register("payments.stripe", &codes)?;

        // Framework prefixes are reserved
        let reserved: HashSet<String> = ["MODULE_CUSTOM".to_string()].into_iter().collect();
        let err = registry.register("my.module", &reserved).unwrap_err();
        assert_eq!(err.code, ErrorCode::ErrorCodeCollision);

        // Unregister
        registry.unregister("payments.stripe");

        // All registered codes (framework + custom)
        let _all_codes: &HashSet<String> = registry.all_codes();
        Ok(())
    }
    ```

**Reserved framework error code prefixes:**

`ACL_`, `APPROVAL_`, `BINDING_`, `CALL_`, `CIRCULAR_`, `CONFIG_`, `DEPENDENCY_`, `ERROR_CODE_`, `FUNC_`, `GENERAL_`, `MIDDLEWARE_`, `MODULE_`, `SCHEMA_`, `VERSION_`

This is the single canonical set (identical to the list in the requirements above, and pinned by `conformance/fixtures/error_codes.json`). One-off framework codes outside these prefixes are protected by exact-code collision detection in `register()`.

### ErrorFormatterRegistry

The `ErrorFormatterRegistry` lets adapters (MCP, A2A, CLI, …) register a formatter that turns a `ModuleError` into the adapter's error shape. A formatter is an object with a `format(error, context)` method. The registry is process-global; `format()` falls back to the error's `to_dict()` / `toJSON()` when no formatter is registered for the adapter.

=== "Python"
    ```python
    from typing import Any

    from apcore import ErrorFormatterRegistry, ModuleError


    class McpFormatter:
        def format(self, error: ModuleError, context: object = None) -> dict[str, Any]:
            return {"code": error.code, "message": error.message}


    # Register a formatter for an adapter (raises ErrorFormatterDuplicateError if taken)
    ErrorFormatterRegistry.register("mcp", McpFormatter())

    # Format an error for a specific adapter
    error = ModuleError(code="MODULE_NOT_FOUND", message="no such module")
    formatted = ErrorFormatterRegistry.format("mcp", error)

    # Get a registered formatter
    formatter = ErrorFormatterRegistry.get("mcp")
    ```
=== "TypeScript"
    ```typescript
    import { ErrorFormatterRegistry, ModuleError } from "apcore-js";
    import type { ErrorFormatter } from "apcore-js";

    const mcpFormatter: ErrorFormatter = {
        format: (error: ModuleError) => ({ code: error.code, message: error.message }),
    };

    // Register a formatter for an adapter (throws ErrorFormatterDuplicateError if taken)
    ErrorFormatterRegistry.register("mcp", mcpFormatter);

    // Format an error for a specific adapter
    const error = new ModuleError("MODULE_NOT_FOUND", "no such module");
    const formatted = ErrorFormatterRegistry.format("mcp", error);

    // Get a registered formatter
    const formatter = ErrorFormatterRegistry.get("mcp");
    ```
=== "Rust"
    ```rust
    use apcore::{ErrorCode, ErrorFormatter, ErrorFormatterRegistry, ModuleError};
    use serde_json::{json, Value};

    struct McpFormatter;

    impl ErrorFormatter for McpFormatter {
        fn format(&self, error: &ModuleError, _context: Option<&dyn std::any::Any>) -> Value {
            json!({"code": error.code, "message": error.message})
        }
    }

    fn main() -> Result<(), ModuleError> {
        // Register a formatter for an adapter (Err if the name is taken)
        ErrorFormatterRegistry::register("mcp", Box::new(McpFormatter))?;

        // Format an error for a specific adapter
        let error = ModuleError::new(ErrorCode::ModuleNotFound, "no such module");
        let _formatted = ErrorFormatterRegistry::format("mcp", &error, None);

        // Check if a formatter is registered
        assert!(ErrorFormatterRegistry::is_registered("mcp"));
        Ok(())
    }
    ```

### Serialization

Errors serialize to sparse JSON with snake_case keys — null fields and empty `details` are omitted. `cause` is serialized as the cause's message string.

```json
{
  "code": "ACL_DENIED",
  "message": "Access denied: api.user -> executor.admin.reset",
  "details": {
    "caller_id": "api.user",
    "target_id": "executor.admin.reset"
  },
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "timestamp": "2026-03-10T12:00:00.000Z",
  "retryable": false,
  "ai_guidance": "Access denied for 'api.user' calling 'executor.admin.reset'. Verify the caller has the required role or permission, or try an alternative module with similar functionality.",
  "user_fixable": false
}
```

## Dependencies

- The **Executor** raises errors from this catalogue at each pipeline step.
- The **ACL System** uses `ACLDeniedError` and `ACLRuleError`.
- The **Approval System** uses `ApprovalDeniedError`, `ApprovalTimeoutError`, and `ApprovalPendingError`.
- The **Schema System** uses `SchemaValidationError`, `SchemaNotFoundError`, `SchemaParseError`, `SchemaCircularRefError`, and `SchemaMaxDepthExceededError`.
- The **Call Chain Guard** uses `CallDepthExceededError`, `CircularCallError`, and `CallFrequencyExceededError`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/errors.py` | Error classes, `ErrorCodes`, `ErrorCodeRegistry` |
    | `src/apcore/error_formatter.py` | `ErrorFormatter`, `ErrorFormatterRegistry` |
    | `src/apcore/cancel.py` | `ExecutionCancelledError` |
    | `src/apcore/pipeline.py` | Pipeline and step errors |

## Testing Strategy

- **Hierarchy tests** verify that all error subclasses inherit from `ModuleError` and carry the correct default `code`.
- **Serialization tests** confirm that `to_dict()` / `toJSON()` produces sparse output and includes all non-null fields.
- **Default tests** verify the per-code `retryable` and `user_fixable` defaults (`conformance/fixtures/error_recovery_metadata.json`) and that explicit values override them.
- **ErrorCodeRegistry tests** exercise registration, collision detection (cross-module, framework-prefix and exact framework code), unregistration, and the `all_codes` aggregation (`conformance/fixtures/error_codes.json`).
- **Error-specific property tests** confirm that domain-specific data (e.g., `ACLDeniedError.caller_id`, `CallDepthExceededError.max_depth`, `ApprovalPendingError.approval_id`) is accessible.

## Contract: ModuleError.to_dict

### Inputs
- No inputs

### Errors
- No errors raised

### Returns
- On success: `dict` / `Record<string, unknown>` / `serde_json::Value` — always contains `code`, `message` and `timestamp`; contains `details`, `cause`, `trace_id`, `retryable`, `ai_guidance`, `user_fixable` and `suggestion` only when set (non-null; `details` only when non-empty)

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

## Invariants: ModuleError

The following invariants hold for every `ModuleError` instance across all language implementations:
- `code` is a non-empty string — a framework code or a module's custom code
- `message` is a human-readable string
- `timestamp` is set at construction (ISO 8601 UTC)
- `trace_id`, when set, is the 32-character lowercase hex trace ID of the call that raised it
