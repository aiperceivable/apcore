// APCore Protocol — Error types
// Spec reference: Error handling and ErrorCode enumeration
// Aligned to apcore-python reference implementation

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Framework error code prefixes reserved by the protocol.
///
/// Canonical set of exactly 14 prefixes (see decision A-D-006). Codes that
/// belong to the framework but fall outside these prefixes (e.g.
/// `CIRCUIT_BREAKER_OPEN`, `PIPELINE_STEP_ERROR`, `STEP_NOT_FOUND`,
/// `STRATEGY_NOT_FOUND`, `RELOAD_FAILED`) are protected by the exact-code
/// collision check in [`ErrorCodeRegistry::register`] rather than by a prefix.
pub const FRAMEWORK_ERROR_CODE_PREFIXES: &[&str] = &[
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
];

/// All error codes defined by the `APCore` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    ConfigNotFound,
    ConfigInvalid,
    #[serde(rename = "ACL_RULE_ERROR")]
    ACLRuleError,
    #[serde(rename = "ACL_DENIED")]
    ACLDenied,
    ModuleNotFound,
    ModuleDisabled,
    ModuleTimeout,
    ModuleLoadError,
    ModuleExecuteError,
    ReloadFailed,
    ExecutionCancelled,
    SchemaValidationError,
    SchemaNotFound,
    SchemaParseError,
    SchemaCircularRef,
    /// Issue #44 (PROTOCOL_SPEC §4.15): a `oneOf` or `anyOf` schema rejected the
    /// input because no branch matched. Cross-language: Python/TS `SCHEMA_UNION_NO_MATCH`.
    SchemaUnionNoMatch,
    /// Issue #44 (PROTOCOL_SPEC §4.15): a `oneOf` schema rejected the input because
    /// more than one branch matched. Cross-language: Python/TS `SCHEMA_UNION_AMBIGUOUS`.
    SchemaUnionAmbiguous,
    /// Issue #44 (PROTOCOL_SPEC §4.15): recursive `$ref` resolution exceeded `max_depth`.
    /// Cross-language: Python/TS `SCHEMA_MAX_DEPTH_EXCEEDED`.
    SchemaMaxDepthExceeded,
    CallDepthExceeded,
    CircularCall,
    CallFrequencyExceeded,
    GeneralInvalidInput,
    /// Raised when a `module_id` is empty or fails the canonical ID grammar
    /// (empty / pattern / length / reserved-word checks in
    /// `validate_module_id`). Distinct from [`Self::GeneralInvalidInput`] so
    /// consumers can match module-id format failures separately from generic
    /// input errors. Normative MUST per error-system.md §560 (core-executor.md
    /// §261/267, registry-system.md §136, async-tasks.md §259). Cross-language:
    /// Python/TS `INVALID_MODULE_ID`.
    InvalidModuleId,
    GeneralInternalError,
    GeneralNotImplemented,
    FuncMissingTypeHint,
    FuncMissingReturnType,
    BindingInvalidTarget,
    BindingModuleNotFound,
    BindingCallableNotFound,
    BindingNotCallable,
    /// Deprecated since 0.19.0; superseded by `BindingSchemaInferenceFailed`.
    /// Kept for backward-compatibility deserialization. See protocol-spec §5.12.8.
    /// The alias below makes the backward-compat contract explicit even if the variant is ever
    /// renamed: old serialized payloads containing `"BINDING_SCHEMA_MISSING"` remain decodable.
    #[serde(alias = "BINDING_SCHEMA_MISSING")]
    BindingSchemaMissing,
    BindingSchemaInferenceFailed,
    BindingSchemaModeConflict,
    BindingStrictSchemaIncompatible,
    BindingFileInvalid,
    CircularDependency,
    MiddlewareChainError,
    ApprovalDenied,
    ApprovalTimeout,
    ApprovalPending,
    VersionIncompatible,
    ErrorCodeCollision,
    DependencyNotFound,
    DependencyVersionMismatch,
    ConfigNamespaceDuplicate,
    ConfigNamespaceReserved,
    ConfigEnvPrefixConflict,
    ConfigMountError,
    ConfigBindError,
    ConfigEnvMapConflict,
    ErrorFormatterDuplicate,
    PipelineAbort,
    PipelineConfigInvalid,
    /// Sync alignment (W-7): pipeline-configuration errors that are NOT
    /// dependency-graph violations (`requires`/`provides`) — for example
    /// removing a non-existent step, configuring a non-existent step, or
    /// declaring a custom step without a valid `after`/`before` anchor.
    /// Distinct from [`Self::PipelineDependencyError`] so callers can match
    /// the structural-config case independently of dependency-graph failures.
    ///
    /// Serializes to `PIPELINE_CONFIGURATION_ERROR`, the code registered in
    /// `features/error-system.md` §"Pipeline & Step Configuration Errors" and
    /// mandated for Rust by decision D-52. It was previously spelled
    /// `ConfigurationError`, which serialized to `CONFIGURATION_ERROR` — a
    /// wire string that appears in no canonical registry and in no other SDK.
    /// Cross-language: the wire code `PIPELINE_CONFIGURATION_ERROR` in all
    /// three SDKs. The CLASS names differ — Python `ConfigurationError`
    /// (pipeline.py), TypeScript `ConfigurationError` (pipeline-config.ts) —
    /// and that is exactly why the conformance fixture asserts the code and
    /// not the name.
    PipelineConfigurationError,
    PipelineHandlerNotSupported,
    PipelineStepInsertionAmbiguous,
    /// `execution-pipeline.md` §The engine loop: a pipeline step's
    /// handler raised an error and `ignore_errors` is `false`. The step name and
    /// the original error are stored in `details["step_name"]` / `details["cause"]`.
    /// Cross-language: Python/TS `PIPELINE_STEP_ERROR`.
    PipelineStepError,
    /// `execution-pipeline.md` §Modifying a strategy: `configure_step`
    /// targeted a step name that does not exist in the strategy.
    /// Cross-language: Python/TS `PIPELINE_STEP_NOT_FOUND`.
    PipelineStepNotFound,
    /// Issue #33 §2.1: a strategy was constructed in which a step's
    /// `requires` list referenced a field not produced by any preceding
    /// step's `provides`. Strategies MUST fail fast at construction rather
    /// than warn at runtime. Details carry `step_name` and `requires`.
    /// Cross-language: Python/TS `PIPELINE_DEPENDENCY_ERROR`.
    PipelineDependencyError,
    StepNotFound,
    StepNotRemovable,
    StepNotReplaceable,
    StepNameDuplicate,
    StrategyNotFound,
    EntryPointNotFound,
    EntryPointAmbiguous,
    /// Reserved for future opt-in runtime entry-point loading APIs (e.g.,
    /// `libloading`-based plugin discovery). No current API path raises this
    /// error. See protocol-spec §5.2.
    EntryPointRuntimeUnsupported,
    /// `Registry::discover_internal()` was called but no custom discoverer
    /// has been configured via `Registry::set_discoverer()`. Rust-specific:
    /// `apcore-python` has a default filesystem discoverer so this state is
    /// unreachable there.
    NoDiscovererConfigured,
    /// Raised when `AsyncTaskManager::submit` is called at the task-slot limit.
    /// Cross-language: Python `TASK_LIMIT_EXCEEDED`, TypeScript `TASK_LIMIT_EXCEEDED`.
    TaskLimitExceeded,
    /// Raised when a `TaskStore` backend is unreachable or refuses an operation
    /// (spec v1.50.0, D-92).
    ///
    /// `async-tasks.md` declares `TaskStoreError(code=TASK_STORE_UNAVAILABLE)`
    /// on eight surfaces — every `TaskStore` method and every
    /// `AsyncTaskManager` method that touches the store — and no SDK defined
    /// it, so no caller could ever catch it. A declared error type no
    /// implementation can raise is the "declared surface reaches no mechanism"
    /// shape PROTOCOL_SPEC §9.1.3 forbids for configuration keys, applied to an
    /// error contract.
    ///
    /// The bundled `InMemoryTaskStore` cannot fail and never raises it; that is
    /// exactly why the code is **public** rather than raised internally. The
    /// hosts who need it write the network-backed stores the contract was
    /// written for, and `AsyncTaskManager` propagates it (D-81) rather than
    /// mapping a store outage onto "task not found".
    ///
    /// Landed late: D-92 was implemented in apcore-python in the v1.50.0 wave
    /// and missed here and in apcore-typescript. Nothing noticed until
    /// `error_codes.json` gained a case asserting the code is
    /// framework-reserved — an SDK that never defined it does not collide, so
    /// the case is red for exactly the state the decision corrects.
    TaskStoreUnavailable,
    /// Raised when `AsyncTaskManager::start_reaper` is called while another
    /// reaper is already running (async-tasks.md "Contract:
    /// AsyncTaskManager.start_reaper", A-C-004/A-C-005). This doc comment
    /// previously cited a decision "A-D-019" claiming the divergence from
    /// apcore-python's `RuntimeError` and apcore-typescript's plain `Error`
    /// was intentional and Rust-specific; no such decision exists in
    /// `docs/spec/*.md` in the apcore repo, and both peers now raise
    /// `ModuleError` with this same code.
    ReaperAlreadyRunning,
    /// Raised when a version constraint string is malformed (e.g., `">="`
    /// without a digit operand, `"v1.0"` prefix, or a non-semver operand).
    /// Cross-language: Python `VERSION_CONSTRAINT_INVALID`, TypeScript `VERSION_CONSTRAINT_INVALID`.
    VersionConstraintInvalid,
    /// Issue #35 / decision D-51 (`features/observability.md` §"Optional
    /// `parent_id` Override on `inject()`"): the `parent_id` passed to
    /// `TraceContext::inject_checked` does not match `^[0-9a-f]{16}$`.
    /// Serializes to `INVALID_PARENT_ID`, the exact code required by the
    /// `trace_context.json` fixture. Cross-language: Python raises
    /// `InvalidParentIdError` (a `ValueError`) and TypeScript throws
    /// `InvalidParentIdError`, both carrying this code.
    InvalidParentId,
    /// Issue #32 (PROTOCOL_SPEC §2.1.1, multi-module-discovery.md): two or more
    /// classes in the same file produce the same `class_segment` after
    /// `snake_case` conversion. The registry rejects the entire file — no
    /// partial registration is permitted. Details carry `file_path`,
    /// `class_names`, and `conflicting_segment`. Cross-language:
    /// Python/TS `MODULE_ID_CONFLICT`.
    ModuleIdConflict,
    /// Issue #32 (PROTOCOL_SPEC §2.1.1): a derived `class_segment` does not
    /// conform to the canonical ID grammar (e.g., starts with a digit after
    /// snake_case conversion). Details carry `file_path`, `class_name`, and
    /// `segment`. Cross-language: Python/TS `INVALID_SEGMENT`.
    InvalidSegment,
    /// Issue #32 (PROTOCOL_SPEC §2.1.1, §2.7): the full derived `module_id`
    /// exceeds `MAX_MODULE_ID_LENGTH` (192 characters). Details carry
    /// `file_path` and `module_id`. Cross-language: Python/TS `ID_TOO_LONG`.
    IdTooLong,
    /// Issue #42 (middleware-system.md §1.2): the `CircuitBreakerMiddleware`
    /// short-circuited a call because the circuit for the (`module_id`,
    /// `caller_id`) pair is `OPEN`. Details carry `module_id` and `caller_id`.
    /// Cross-language: Python/TS `CIRCUIT_BREAKER_OPEN`.
    #[serde(rename = "CIRCUIT_BREAKER_OPEN")]
    CircuitBreakerOpen,
    /// Issue #45 (system-modules.md §1.4): `system.control.reload_module` was
    /// called with both `module_id` and `path_filter` set. Cross-language:
    /// Python/TS `MODULE_RELOAD_CONFLICT`.
    ModuleReloadConflict,
    /// Issue #45 (system-modules.md §1.5): a system module failed to register
    /// during `register_sys_modules` and the caller requested strict failure.
    /// Cross-language: Python/TS `SYS_MODULE_REGISTRATION_FAILED`.
    SysModuleRegistrationFailed,
    /// `APCore::disable()` / `APCore::enable()` was called but `sys_modules` is
    /// not enabled in the current config. Cross-language: Python `RuntimeError`,
    /// TypeScript `Error` (sync finding A-007). The Rust SDK uses a typed
    /// `ModuleError` with this code rather than panicking, matching idiomatic
    /// Rust error handling.
    SysModulesDisabled,
    /// Sync finding D-25: `system.control.update_config` was called with a key
    /// listed in `RESTRICTED_KEYS`. Cross-language: Python/TS
    /// `CONFIG_KEY_RESTRICTED`. Distinct from `ConfigInvalid` so consumers
    /// can match the policy-deny case separately from value-shape errors.
    ConfigKeyRestricted,
    /// Issue #62 (streaming.md §"Streaming Module Interface"): a module declares
    /// `annotations.streaming = true` but does not implement `StreamingModule`
    /// (i.e. `as_streaming()` returns `None`). The registry rejects the registration.
    /// Cross-language: Python/TS `STREAMING_INTERFACE_MISMATCH`.
    StreamingInterfaceMismatch,
    /// Issue #65 (registry-system.md §"Registration Ordering Invariants"): two
    /// concurrent register() calls for the same module_id — the second is rejected
    /// because the first is still in the in-flight loading set.
    /// Cross-language: Python/TS `DUPLICATE_MODULE_ID`.
    DuplicateModuleId,
    /// Issue #66 (core-executor.md §"Contract: Executor binding to Context"): a
    /// Context whose `executor` field is already bound to a different Executor
    /// instance was passed to `Executor::bind_executor` (or the equivalent
    /// pipeline-entry binding). Cross-executor rebind is rejected by default;
    /// SDKs that accept silently MUST document the deviation prominently.
    /// Cross-language: Python/TS `CONTEXT_BINDING_ERROR`.
    #[serde(rename = "CONTEXT_BINDING_ERROR")]
    ContextBindingError,
}

impl ErrorCode {
    /// All `ErrorCode` variants defined by the protocol.
    ///
    /// This list MUST contain every enum variant; it is the source from which
    /// the framework reserved-code set is derived (mirrors Python
    /// `_collect_framework_codes` and TypeScript `collectFrameworkCodes`).
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::ConfigNotFound,
        ErrorCode::ConfigInvalid,
        ErrorCode::ACLRuleError,
        ErrorCode::ACLDenied,
        ErrorCode::ModuleNotFound,
        ErrorCode::ModuleDisabled,
        ErrorCode::ModuleTimeout,
        ErrorCode::ModuleLoadError,
        ErrorCode::ModuleExecuteError,
        ErrorCode::ReloadFailed,
        ErrorCode::ExecutionCancelled,
        ErrorCode::SchemaValidationError,
        ErrorCode::SchemaNotFound,
        ErrorCode::SchemaParseError,
        ErrorCode::SchemaCircularRef,
        ErrorCode::SchemaUnionNoMatch,
        ErrorCode::SchemaUnionAmbiguous,
        ErrorCode::SchemaMaxDepthExceeded,
        ErrorCode::CallDepthExceeded,
        ErrorCode::CircularCall,
        ErrorCode::CallFrequencyExceeded,
        ErrorCode::GeneralInvalidInput,
        ErrorCode::InvalidModuleId,
        ErrorCode::GeneralInternalError,
        ErrorCode::GeneralNotImplemented,
        ErrorCode::FuncMissingTypeHint,
        ErrorCode::FuncMissingReturnType,
        ErrorCode::BindingInvalidTarget,
        ErrorCode::BindingModuleNotFound,
        ErrorCode::BindingCallableNotFound,
        ErrorCode::BindingNotCallable,
        ErrorCode::BindingSchemaMissing,
        ErrorCode::BindingSchemaInferenceFailed,
        ErrorCode::BindingSchemaModeConflict,
        ErrorCode::BindingStrictSchemaIncompatible,
        ErrorCode::BindingFileInvalid,
        ErrorCode::CircularDependency,
        ErrorCode::MiddlewareChainError,
        ErrorCode::ApprovalDenied,
        ErrorCode::ApprovalTimeout,
        ErrorCode::ApprovalPending,
        ErrorCode::VersionIncompatible,
        ErrorCode::ErrorCodeCollision,
        ErrorCode::DependencyNotFound,
        ErrorCode::DependencyVersionMismatch,
        ErrorCode::ConfigNamespaceDuplicate,
        ErrorCode::ConfigNamespaceReserved,
        ErrorCode::ConfigEnvPrefixConflict,
        ErrorCode::ConfigMountError,
        ErrorCode::ConfigBindError,
        ErrorCode::ConfigEnvMapConflict,
        ErrorCode::ErrorFormatterDuplicate,
        ErrorCode::PipelineAbort,
        ErrorCode::PipelineConfigInvalid,
        ErrorCode::PipelineConfigurationError,
        ErrorCode::PipelineHandlerNotSupported,
        ErrorCode::PipelineStepInsertionAmbiguous,
        ErrorCode::PipelineStepError,
        ErrorCode::PipelineStepNotFound,
        ErrorCode::PipelineDependencyError,
        ErrorCode::StepNotFound,
        ErrorCode::StepNotRemovable,
        ErrorCode::StepNotReplaceable,
        ErrorCode::StepNameDuplicate,
        ErrorCode::StrategyNotFound,
        ErrorCode::EntryPointNotFound,
        ErrorCode::EntryPointAmbiguous,
        ErrorCode::EntryPointRuntimeUnsupported,
        ErrorCode::NoDiscovererConfigured,
        ErrorCode::TaskLimitExceeded,
        ErrorCode::TaskStoreUnavailable,
        ErrorCode::ReaperAlreadyRunning,
        ErrorCode::VersionConstraintInvalid,
        ErrorCode::InvalidParentId,
        ErrorCode::ModuleIdConflict,
        ErrorCode::InvalidSegment,
        ErrorCode::IdTooLong,
        ErrorCode::CircuitBreakerOpen,
        ErrorCode::ModuleReloadConflict,
        ErrorCode::SysModuleRegistrationFailed,
        ErrorCode::SysModulesDisabled,
        ErrorCode::ConfigKeyRestricted,
        ErrorCode::StreamingInterfaceMismatch,
        ErrorCode::DuplicateModuleId,
        ErrorCode::ContextBindingError,
    ];

    /// Canonical wire string for this code (SCREAMING_SNAKE_CASE via serde,
    /// e.g. `MODULE_EXECUTE_ERROR`). This is the cross-language-stable
    /// representation used in error fingerprints and observability metric
    /// labels — never `Debug` formatting, which yields PascalCase and breaks
    /// parity with Python/TS (sync finding A-D-14). Falls back to `Debug` only
    /// if serde serialization unexpectedly fails (should never occur).
    #[must_use]
    pub fn wire_str(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{self:?}"))
    }
}

/// Lazily-built set of every framework error-code wire string.
///
/// Mirrors Python `_FRAMEWORK_CODES` / TypeScript `FRAMEWORK_CODES`. Used by
/// [`ErrorCodeRegistry::register`] for the exact-code collision check and to
/// seed the registry's `all_codes` set.
pub static FRAMEWORK_CODES: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(|| {
    ErrorCode::ALL
        .iter()
        .filter_map(|c| match serde_json::to_value(c) {
            Ok(serde_json::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect()
});

/// Declarative `retryable` policy, keyed by error code (single source of
/// truth). The authority is PROTOCOL_SPEC §8.6's classification table, of which
/// `apcore/conformance/fixtures/error_recovery_metadata.json` pins an
/// executable subset; the fixture is what CI compares, the table is what the
/// map is transcribed from.
///
/// - `Some(true)`  — retrying the same call later may succeed, the precondition
///   being time rather than a corrected input (`APPROVAL_TIMEOUT`: nobody
///   answered; `MODULE_TIMEOUT`; `GENERAL_INTERNAL_ERROR`).
/// - `Some(false)` — governance / structural; retrying without changing the
///   precondition is not useful (`APPROVAL_DENIED`: a human said no, which is
///   terminal — the distinction from `APPROVAL_TIMEOUT` is the one an
///   autonomous caller most needs, apcore#36).
/// - `None`        — left for the module author to supply (business errors,
///   e.g. `MODULE_EXECUTE_ERROR`); any code not listed here stays unset.
///
/// **Structural difference from the peers, deliberate.** apcore-python resolves
/// this per error *class* (`_default_retryable`, via the `_UNSET` sentinel in
/// `ModuleError.__init__`) and apcore-typescript per class as well
/// (`static DEFAULT_RETRYABLE`, read off `this.constructor`). This binding has
/// no error-class hierarchy — one `ModuleError` keyed by [`ErrorCode`] — so the
/// policy is keyed by code instead, which is also why the fixture is the only
/// place the two shapes can be compared and why
/// `conformance_error_recovery_metadata_defaults` asserts every case rather
/// than deferring to a class-level test that has nowhere to live here.
///
/// **Scope.** Every code PROTOCOL_SPEC §8.6's classification table pins, plus
/// the `CONFIG_*` family §9.12.4 covers with a categorical **MUST** ("All
/// config errors ... are non-retryable"), plus the extras
/// [`user_fixable_for_code`] already carries so the two fields never disagree
/// about a code one of them knows. Codes outside all three stay `None`: the
/// peers default a further tail to `false` per class, but a value invented on
/// one side of a three-SDK contract is the divergence this function exists to
/// prevent, and §8.6 is the authority that makes the rest transcription rather
/// than invention.
///
/// `EXECUTION_CANCELLED` is retryable (D-135) in the sense §8.6 gives it: the
/// same call sent again WITH A FRESH `CancelToken` may succeed. A retry inside
/// the pipeline reuses the call's own token, which stays cancelled, so
/// [`crate::middleware::RetryMiddleware`] re-raising it cannot resurrect a call
/// the caller cancelled.
///
/// `MODULE_EXECUTE_ERROR` (§8.6 "Depends" — on the module's
/// `annotations.idempotent`) is deliberately not carried, which is exactly
/// what `None` means here.
///
/// Not part of the public crate API: `pub` only so integration tests in `tests/`
/// can assert fixture↔source parity; hidden from rustdoc.
#[doc(hidden)]
#[must_use]
pub fn retryable_for_code(code: ErrorCode) -> Option<bool> {
    // One arm per value rather than one per semantic group: `clippy::match_same_arms`
    // rejects the grouped form, so the grouping lives in the comments inside the
    // chain, which is where `INVALID_PARENT_ID` already carried its rationale.
    match code {
        // §8.6 "Yes" — the missing precondition is time, so the same call sent
        // later may succeed without the caller changing anything.
        // `RELOAD_FAILED` is raised by `Config::reload`, never by module
        // execution, so it carries the annotation without reaching the retry
        // gate.
        ErrorCode::ApprovalTimeout
        | ErrorCode::ModuleTimeout
        | ErrorCode::GeneralInternalError
        // A full task pool is a capacity condition, not a caller fault: the
        // slot the submission wants is freed by a task finishing. apcore-python
        // (`TaskLimitExceededError._default_retryable = True`) and
        // apcore-typescript (`DEFAULT_RETRYABLE = true`) both resolve it this
        // way, and `RetryMiddleware` in all three gates on
        // `retryable == Some(true)` — so omitting it here meant the peers
        // auto-retried a submission this SDK surfaced to the caller (ERR-003).
        | ErrorCode::TaskLimitExceeded
        | ErrorCode::TaskStoreUnavailable
        | ErrorCode::ReloadFailed
        // D-135: retried with a fresh token, and the circuit closes again after
        // its recovery window — the code alone carries the default, not only
        // the middleware's builder.
        | ErrorCode::ExecutionCancelled
        | ErrorCode::CircuitBreakerOpen => Some(true),

        // §8.6 "No", grouped by why retrying cannot help.
        //
        // Input faults: the caller fixes what it sends, not when it sends.
        ErrorCode::SchemaValidationError
        | ErrorCode::GeneralInvalidInput
        | ErrorCode::SchemaUnionNoMatch
        | ErrorCode::SchemaUnionAmbiguous
        // Not in §8.6; carried because `user_fixable_for_code` carries it, and
        // apcore-python pins the same value on the class
        // (`InvalidParentIdError._default_retryable = False`). A caller that
        // sent a malformed `parent_id` fixes it by sending a valid one, never
        // by retrying the same call.
        | ErrorCode::InvalidParentId
        // Governance: a decision, not a transient condition. The distinction
        // between `APPROVAL_DENIED` (a human said no — terminal) and
        // `APPROVAL_TIMEOUT` (nobody answered — retryable) is the one an
        // autonomous caller most needs (apcore#36). `APPROVAL_PENDING` is
        // non-retryable because the caller should poll, not re-submit.
        | ErrorCode::ACLDenied
        | ErrorCode::ACLRuleError
        | ErrorCode::ApprovalDenied
        | ErrorCode::ApprovalPending
        | ErrorCode::ModuleDisabled
        // Call-chain structure: retrying reproduces the same chain.
        | ErrorCode::CallDepthExceeded
        | ErrorCode::CircularCall
        | ErrorCode::CallFrequencyExceeded
        // Resolution and wiring: needs a deployment or code change.
        | ErrorCode::ModuleNotFound
        | ErrorCode::ModuleLoadError
        | ErrorCode::VersionConstraintInvalid
        | ErrorCode::VersionIncompatible
        | ErrorCode::DependencyNotFound
        | ErrorCode::DependencyVersionMismatch
        | ErrorCode::CircularDependency
        | ErrorCode::ErrorCodeCollision
        | ErrorCode::MiddlewareChainError
        // Schema definition faults: the schema is wrong, not the moment.
        | ErrorCode::SchemaNotFound
        | ErrorCode::SchemaParseError
        | ErrorCode::SchemaCircularRef
        | ErrorCode::SchemaMaxDepthExceeded
        // Binding faults: the binding declaration needs fixing.
        | ErrorCode::BindingInvalidTarget
        | ErrorCode::BindingModuleNotFound
        | ErrorCode::BindingCallableNotFound
        | ErrorCode::BindingNotCallable
        | ErrorCode::BindingFileInvalid
        | ErrorCode::BindingSchemaMissing
        | ErrorCode::BindingSchemaInferenceFailed
        | ErrorCode::BindingSchemaModeConflict
        | ErrorCode::BindingStrictSchemaIncompatible
        // Authoring faults: the module's own signature needs a developer.
        | ErrorCode::FuncMissingTypeHint
        | ErrorCode::FuncMissingReturnType
        // §9.12.4: every config error is non-retryable. `CONFIG_KEY_RESTRICTED`
        // is not a table row but is a config error, so the categorical MUST
        // reaches it too.
        | ErrorCode::ConfigNotFound
        | ErrorCode::ConfigInvalid
        | ErrorCode::ConfigNamespaceDuplicate
        | ErrorCode::ConfigNamespaceReserved
        | ErrorCode::ConfigEnvPrefixConflict
        | ErrorCode::ConfigMountError
        | ErrorCode::ConfigBindError
        | ErrorCode::ConfigEnvMapConflict
        | ErrorCode::ConfigKeyRestricted
        // D-135: a configuration error does not go away on retry; set
        // explicitly rather than left unset.
        | ErrorCode::PipelineConfigurationError => Some(false),

        // Unlisted codes (e.g. MODULE_EXECUTE_ERROR) stay unset — see the
        // scope note above.
        _ => None,
    }
}

/// Declarative `user_fixable` policy, keyed by error code (single source of
/// truth; kept in lock-step with the same fixture as [`retryable_for_code`]).
///
/// - `Some(true)`  — the caller can resolve it by changing the input or
///   configuration they sent.
/// - `Some(false)` — governance / system / structural / transient; not
///   resolvable by changing input.
/// - `None`        — left for the module author to supply (business errors,
///   e.g. `MODULE_EXECUTE_ERROR`); any code not listed here stays unset.
///
/// Mirrors apcore-python's private `_USER_FIXABLE_BY_CODE` and TypeScript's
/// `@internal` `USER_FIXABLE_BY_CODE` — both of which are keyed by code, unlike
/// `retryable`; see [`retryable_for_code`] for that asymmetry and for the
/// visibility note that applies to this function too.
#[doc(hidden)]
#[must_use]
pub fn user_fixable_for_code(code: ErrorCode) -> Option<bool> {
    match code {
        // Caller can fix by changing input/config:
        ErrorCode::SchemaValidationError
        | ErrorCode::GeneralInvalidInput
        | ErrorCode::ModuleNotFound
        | ErrorCode::VersionConstraintInvalid
        // A malformed `parent_id` override is supplied by the caller, so the
        // caller can fix it by passing a valid 16-hex span id.
        | ErrorCode::InvalidParentId
        | ErrorCode::BindingSchemaInferenceFailed
        | ErrorCode::BindingSchemaModeConflict
        | ErrorCode::BindingStrictSchemaIncompatible
        | ErrorCode::DependencyNotFound
        | ErrorCode::DependencyVersionMismatch => Some(true),
        // Governance / system / structural / transient — not caller-fixable by input:
        ErrorCode::ACLDenied
        | ErrorCode::ApprovalDenied
        | ErrorCode::ApprovalTimeout
        | ErrorCode::ModuleTimeout
        | ErrorCode::ModuleDisabled
        | ErrorCode::CallDepthExceeded
        | ErrorCode::CircularCall
        | ErrorCode::CallFrequencyExceeded
        | ErrorCode::GeneralInternalError => Some(false),
        // Unlisted codes (e.g. MODULE_EXECUTE_ERROR) stay unset.
        _ => None,
    }
}

/// Structured error returned by module execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub details: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_fixable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl ModuleError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: HashMap::new(),
            cause: None,
            trace_id: None,
            timestamp: Utc::now(),
            // Both recovery fields resolve their framework-deterministic
            // default from the error code (single source of truth:
            // `retryable_for_code` / `user_fixable_for_code`), and an explicit
            // value still overrides via `with_retryable` / `with_user_fixable`.
            // Mirrors apcore-python `ModuleError.__init__`, which resolves
            // `user_fixable` from `code` and `retryable` from the error class.
            retryable: retryable_for_code(code),
            ai_guidance: None,
            user_fixable: user_fixable_for_code(code),
            suggestion: None,
        }
    }

    // Builder methods

    #[must_use]
    pub fn with_details(mut self, details: HashMap<String, serde_json::Value>) -> Self {
        self.details = details;
        self
    }

    #[must_use]
    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.cause = Some(cause.into());
        self
    }

    #[must_use]
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    #[must_use]
    pub fn with_ai_guidance(mut self, ai_guidance: impl Into<String>) -> Self {
        self.ai_guidance = Some(ai_guidance.into());
        self
    }

    #[must_use]
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = Some(retryable);
        self
    }

    /// Explicitly set `user_fixable`, overriding the code-derived default
    /// resolved in [`ModuleError::new`]. Mirrors apcore-python's explicit
    /// `user_fixable=` override of `_USER_FIXABLE_BY_CODE`.
    #[must_use]
    pub fn with_user_fixable(mut self, user_fixable: bool) -> Self {
        self.user_fixable = Some(user_fixable);
        self
    }

    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Convert to a sparse JSON dictionary (omitting None fields).
    #[must_use]
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }

    /// Builder for `GENERAL_INVALID_INPUT` carrying the default AI recovery
    /// guidance. Mirrors apcore-python `InvalidInputError`, whose `__init__`
    /// supplies a default `ai_guidance` (still overridable via
    /// [`Self::with_ai_guidance`]). `user_fixable` resolves to `true` from the
    /// code via [`user_fixable_for_code`].
    #[must_use]
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::GeneralInvalidInput, message).with_ai_guidance(
            "The input was malformed or missing required fields. Check the values against the \
             module's input_schema and retry with corrected input.",
        )
    }

    /// A `TaskStore` backend is unreachable or refused an operation (D-92).
    ///
    /// The counterpart of apcore-python's `TaskStoreError` and
    /// apcore-typescript's `TaskStoreError`. Rust carries the type as an
    /// `ErrorCode` variant plus this constructor rather than a per-error
    /// struct, which is how every other error in this SDK is shaped.
    ///
    /// The bundled `InMemoryTaskStore` cannot fail and never calls this; it is
    /// public because the hosts who need it are the ones writing the
    /// network-backed stores `async-tasks.md` was written for. Details carry
    /// `operation` and `reason` when supplied, matching both peers.
    #[must_use]
    pub fn task_store_unavailable(operation: &str, reason: &str) -> Self {
        let mut detail = if operation.is_empty() {
            "TaskStore is unavailable".to_string()
        } else {
            format!("TaskStore operation '{operation}' failed")
        };
        if !reason.is_empty() {
            detail = format!("{detail}: {reason}");
        }
        let mut err = Self::new(ErrorCode::TaskStoreUnavailable, detail);
        let mut details: HashMap<String, serde_json::Value> = HashMap::new();
        if !operation.is_empty() {
            details.insert("operation".to_string(), serde_json::json!(operation));
        }
        if !reason.is_empty() {
            details.insert("reason".to_string(), serde_json::json!(reason));
        }
        if !details.is_empty() {
            err = err.with_details(details);
        }
        err
    }

    /// Builder for `ACL_DENIED` carrying the default AI recovery guidance and
    /// the structured caller/target detail (apcore#37).
    ///
    /// Mirrors apcore-python `ACLDeniedError` and apcore-typescript
    /// `ACLDeniedError`, which supply the same guidance text word for word (via
    /// `setdefault` and `??` respectively) and the same
    /// `Access denied: {caller} -> {target}` message shape. The guidance
    /// remains overridable through [`Self::with_ai_guidance`], which is what
    /// those two defaulting forms buy in a language with keyword arguments.
    /// Both reach an external caller: `apcore-a2a` appends `ai_guidance` to the
    /// failure text when `disclose_refusal_reason` is on, and discloses the
    /// message itself.
    ///
    /// The two surfaces carry the absent caller differently, on purpose.
    /// `details.caller_id` is the **raw wire value** — `null` for an
    /// unauthenticated caller, which is what both peers put there and what the
    /// cross-language contract pins. The message and guidance name the
    /// **resolved sentinel** `@external`, which is the identity the ACL itself
    /// matched: [`ACL::check_access`](crate::acl::ACL::check_access) resolves a
    /// `None` caller to `@external` before evaluating any rule, and the
    /// `AuditEntry` for this very denial records `@external` too.
    ///
    /// Naming it anything else makes the guidance actively wrong rather than
    /// merely vague. `@external` is matched as an exact literal — no wildcard
    /// reaches it — so an operator or agent acting on "verify the caller has
    /// the required role" needs that exact spelling to write a rule that fires.
    /// A prose-only word like "anonymous" would send them to a `callers:`
    /// pattern the matcher can never produce.
    #[must_use]
    pub fn acl_denied(caller_id: Option<&str>, target_id: &str) -> Self {
        // INVARIANT: the same sentinel `ACL::check_inner` resolves `None` to
        // before matching, and the one `build_audit_entry` records. Keep the
        // three surfaces (audit, message, guidance) naming one identity.
        let caller = caller_id.unwrap_or(crate::acl::EXTERNAL_CALLER);
        let mut details = HashMap::new();
        details.insert("caller_id".to_string(), serde_json::json!(caller_id));
        details.insert("target_id".to_string(), serde_json::json!(target_id));
        Self::new(
            ErrorCode::ACLDenied,
            format!("Access denied: {caller} -> {target_id}"),
        )
        .with_details(details)
        .with_ai_guidance(format!(
            "Access denied for '{caller}' calling '{target_id}'. Verify the caller has the \
             required role or permission, or try an alternative module with similar functionality."
        ))
    }

    #[must_use]
    pub fn config_namespace_duplicate(name: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("name".to_string(), serde_json::json!(name));
        Self::new(
            ErrorCode::ConfigNamespaceDuplicate,
            format!("Namespace '{name}' is already registered"),
        )
        .with_details(details)
    }

    #[must_use]
    pub fn config_namespace_reserved(name: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("name".to_string(), serde_json::json!(name));
        Self::new(
            ErrorCode::ConfigNamespaceReserved,
            format!("Namespace '{name}' is reserved"),
        )
        .with_details(details)
    }

    #[must_use]
    pub fn config_env_prefix_conflict(prefix: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("env_prefix".to_string(), serde_json::json!(prefix));
        Self::new(
            ErrorCode::ConfigEnvPrefixConflict,
            format!("env_prefix '{prefix}' conflicts with reserved pattern"),
        )
        .with_details(details)
    }

    #[must_use]
    pub fn config_env_map_conflict(env_var: &str, owner: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("env_var".to_string(), serde_json::json!(env_var));
        details.insert("owner".to_string(), serde_json::json!(owner));
        Self::new(
            ErrorCode::ConfigEnvMapConflict,
            format!("Environment variable '{env_var}' is already mapped by '{owner}'"),
        )
        .with_details(details)
    }

    #[must_use]
    pub fn config_mount_error(namespace: &str, reason: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("namespace".to_string(), serde_json::json!(namespace));
        Self::new(
            ErrorCode::ConfigMountError,
            format!("Mount failed for '{namespace}': {reason}"),
        )
        .with_details(details)
    }

    #[must_use]
    pub fn config_bind_error(namespace: &str, reason: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("namespace".to_string(), serde_json::json!(namespace));
        Self::new(
            ErrorCode::ConfigBindError,
            format!("Bind failed for '{namespace}': {reason}"),
        )
        .with_details(details)
    }

    #[must_use]
    pub fn error_formatter_duplicate(adapter_name: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("adapter_name".to_string(), serde_json::json!(adapter_name));
        Self::new(
            ErrorCode::ErrorFormatterDuplicate,
            format!("ErrorFormatter for adapter '{adapter_name}' is already registered"),
        )
        .with_details(details)
    }

    #[must_use]
    pub fn pipeline_abort(step: &str, explanation: Option<&str>) -> Self {
        let mut details = HashMap::new();
        details.insert("step".to_string(), serde_json::json!(step));
        Self::new(
            ErrorCode::PipelineAbort,
            format!(
                "Pipeline aborted at step '{}': {}",
                step,
                explanation.unwrap_or("no explanation")
            ),
        )
        .with_details(details)
    }

    pub fn step_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::StepNotFound, message)
    }

    pub fn step_not_removable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::StepNotRemovable, message)
    }

    pub fn step_not_replaceable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::StepNotReplaceable, message)
    }

    pub fn step_name_duplicate(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::StepNameDuplicate, message)
    }

    pub fn strategy_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::StrategyNotFound, message)
    }

    /// Wrap a step's underlying error in a `PipelineStepError` per §1.1.
    ///
    /// The original error is preserved in `details["cause"]` (full JSON form) and
    /// the step name in `details["step_name"]`. Use [`Self::is_pipeline_step_error`]
    /// + [`Self::unwrap_pipeline_step_error`] to recover the original error.
    #[must_use]
    pub fn pipeline_step_error(step_name: &str, cause: &ModuleError) -> Self {
        let mut details = HashMap::new();
        details.insert("step_name".to_string(), serde_json::json!(step_name));
        if let Ok(cause_json) = serde_json::to_value(cause) {
            details.insert("cause".to_string(), cause_json);
        }
        let cause_message = cause.message.clone();
        Self::new(
            ErrorCode::PipelineStepError,
            format!("Pipeline step '{step_name}' failed: {cause_message}"),
        )
        .with_details(details)
        .with_cause(cause_message)
    }

    /// Whether this error is a `PipelineStepError` wrapper.
    #[must_use]
    pub fn is_pipeline_step_error(&self) -> bool {
        self.code == ErrorCode::PipelineStepError
    }

    /// The step name carried by a `PipelineStepError`, if any.
    #[must_use]
    pub fn step_name(&self) -> Option<&str> {
        self.details.get("step_name").and_then(|v| v.as_str())
    }

    /// Recover the original error wrapped by `pipeline_step_error()`. Returns
    /// `None` if this error is not a `PipelineStepError` or the cause was not
    /// preserved in a structured form.
    #[must_use]
    pub fn unwrap_pipeline_step_error(&self) -> Option<ModuleError> {
        if !self.is_pipeline_step_error() {
            return None;
        }
        self.details
            .get("cause")
            .and_then(|v| serde_json::from_value::<ModuleError>(v.clone()).ok())
    }

    /// Returns true if this error is a `MiddlewareChainError`.
    #[must_use]
    pub fn is_middleware_chain_error(&self) -> bool {
        matches!(self.code, ErrorCode::MiddlewareChainError)
    }

    /// Recover the original error wrapped by `MiddlewareManager.execute_before()`.
    ///
    /// Returns `None` if this error is not a `MiddlewareChainError` or the
    /// inner error was not preserved in a structured form. Cross-language
    /// parity with apcore-python `MiddlewareChainError.original` and
    /// apcore-typescript `MiddlewareChainError.original` (sync finding A-D-015).
    #[must_use]
    pub fn unwrap_middleware_chain_error(&self) -> Option<ModuleError> {
        if !self.is_middleware_chain_error() {
            return None;
        }
        self.details
            .get("inner_error")
            .and_then(|v| serde_json::from_value::<ModuleError>(v.clone()).ok())
    }

    /// Builder for `MODULE_ID_CONFLICT` (Issue #32, PROTOCOL_SPEC §2.1.1).
    ///
    /// Two or more classes in the same file produced the same `class_segment`
    /// after snake_case conversion. The whole file is rejected.
    #[must_use]
    pub fn module_id_conflict(
        file_path: &str,
        class_names: &[String],
        conflicting_segment: &str,
    ) -> Self {
        let mut details = HashMap::new();
        details.insert("file_path".to_string(), serde_json::json!(file_path));
        details.insert("class_names".to_string(), serde_json::json!(class_names));
        details.insert(
            "conflicting_segment".to_string(),
            serde_json::json!(conflicting_segment),
        );
        Self::new(
            ErrorCode::ModuleIdConflict,
            format!(
                "Module ID conflict in '{file_path}': classes {class_names:?} both produce segment '{conflicting_segment}'"
            ),
        )
        .with_details(details)
    }

    /// Builder for `INVALID_SEGMENT` (Issue #32, PROTOCOL_SPEC §2.1.1).
    #[must_use]
    pub fn invalid_segment(file_path: &str, class_name: &str, segment: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("file_path".to_string(), serde_json::json!(file_path));
        details.insert("class_name".to_string(), serde_json::json!(class_name));
        details.insert("segment".to_string(), serde_json::json!(segment));
        Self::new(
            ErrorCode::InvalidSegment,
            format!(
                "Invalid class segment '{segment}' derived from '{class_name}' in '{file_path}': must match \
                 ^[a-z][a-z0-9_]*$"
            ),
        )
        .with_details(details)
    }

    /// Builder for `CIRCUIT_OPEN` (Issue #42, middleware-system.md §1.2).
    ///
    /// Returned by `CircuitBreakerMiddleware::before` when the circuit for the
    /// given `(module_id, caller_id)` pair is `OPEN`. The error carries
    /// `details["module_id"]` and `details["caller_id"]`.
    #[must_use]
    pub fn circuit_breaker_open(module_id: &str, caller_id: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        details.insert("caller_id".to_string(), serde_json::json!(caller_id));
        Self::new(
            ErrorCode::CircuitBreakerOpen,
            format!("Circuit open for module '{module_id}' (caller '{caller_id}') — call rejected"),
        )
        .with_details(details)
        .with_retryable(true)
        .with_ai_guidance(
            "The downstream module is temporarily unavailable. The circuit will probe \
             after the recovery window elapses; retry the request after a short delay.",
        )
    }

    /// Builder for `STREAMING_INTERFACE_MISMATCH` (Issue #62).
    #[must_use]
    pub fn streaming_interface_mismatch(
        module_id: impl Into<String>,
        mismatch_reason: impl Into<String>,
    ) -> Self {
        let module_id = module_id.into();
        let mismatch_reason = mismatch_reason.into();
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        details.insert(
            "mismatch_reason".to_string(),
            serde_json::json!(mismatch_reason),
        );
        Self::new(
            ErrorCode::StreamingInterfaceMismatch,
            format!("Module '{module_id}' streaming interface mismatch: {mismatch_reason}"),
        )
        .with_details(details)
    }

    /// Builder for `DUPLICATE_MODULE_ID` (Issue #65).
    #[must_use]
    pub fn duplicate_module_id(module_id: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        Self::new(
            ErrorCode::DuplicateModuleId,
            format!("Module '{module_id}' is already registered or registration is in progress"),
        )
        .with_details(details)
    }

    /// Builder for `MODULE_NOT_FOUND` carrying the default AI recovery
    /// guidance (ERR-004).
    ///
    /// The five codes below (`MODULE_NOT_FOUND`, `MODULE_DISABLED`,
    /// `MODULE_TIMEOUT`, `DEPENDENCY_NOT_FOUND`,
    /// `DEPENDENCY_VERSION_MISMATCH`) were the only ones apcore-python and
    /// apcore-typescript attach a default `ai_guidance` to and this SDK did
    /// not — every raise site built them with a bare [`Self::new`], while eight
    /// other codes already had guidance here. The wording is the peers',
    /// verbatim, so an agent reading `ai_guidance` gets the same instruction on
    /// all three.
    #[must_use]
    pub fn module_not_found(module_id: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        Self::new(
            ErrorCode::ModuleNotFound,
            format!("Module not found: {module_id}"),
        )
        .with_details(details)
        .with_ai_guidance(format!(
            "Module '{module_id}' does not exist in the registry. Verify the module ID \
             spelling. Use system.manifest.full to list available modules."
        ))
    }

    /// Builder for `MODULE_DISABLED` carrying the default AI recovery guidance
    /// (ERR-004).
    #[must_use]
    pub fn module_disabled(module_id: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        Self::new(
            ErrorCode::ModuleDisabled,
            format!("Module '{module_id}' is disabled"),
        )
        .with_details(details)
        .with_ai_guidance(format!(
            "Module '{module_id}' is currently disabled. Use system.control.toggle_feature \
             to re-enable it, or find an alternative module."
        ))
    }

    /// Builder for `MODULE_TIMEOUT` carrying the default AI recovery guidance
    /// (ERR-004).
    #[must_use]
    pub fn module_timeout(module_id: &str, timeout_ms: u64) -> Self {
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        details.insert("timeout_ms".to_string(), serde_json::json!(timeout_ms));
        Self::new(
            ErrorCode::ModuleTimeout,
            format!("Module {module_id} timed out after {timeout_ms}ms"),
        )
        .with_details(details)
        .with_ai_guidance(format!(
            "Module '{module_id}' timed out after {timeout_ms}ms. Consider: 1) Breaking \
             the operation into smaller steps. 2) Reducing the input data size. 3) Asking \
             the user if a longer timeout is acceptable."
        ))
    }

    /// Builder for `DEPENDENCY_NOT_FOUND` carrying the default AI recovery
    /// guidance (ERR-004).
    #[must_use]
    pub fn dependency_not_found(module_id: &str, dependency_id: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        details.insert(
            "dependency_id".to_string(),
            serde_json::json!(dependency_id),
        );
        Self::new(
            ErrorCode::DependencyNotFound,
            format!("Module '{module_id}' has unsatisfied required dependency '{dependency_id}'"),
        )
        .with_details(details)
        .with_ai_guidance(format!(
            "Module '{module_id}' declares a required dependency on '{dependency_id}', but \
             no such module is registered. Either register '{dependency_id}' before loading \
             '{module_id}', mark the dependency as optional, or remove it."
        ))
    }

    /// Builder for `DEPENDENCY_VERSION_MISMATCH` carrying the default AI
    /// recovery guidance (ERR-004).
    #[must_use]
    pub fn dependency_version_mismatch(
        module_id: &str,
        dependency_id: &str,
        required: &str,
        actual: &str,
    ) -> Self {
        let mut details = HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        details.insert(
            "dependency_id".to_string(),
            serde_json::json!(dependency_id),
        );
        details.insert("required".to_string(), serde_json::json!(required));
        details.insert("actual".to_string(), serde_json::json!(actual));
        Self::new(
            ErrorCode::DependencyVersionMismatch,
            format!(
                "Module '{module_id}' requires dependency '{dependency_id}' version \
                 '{required}', but registered version is '{actual}'"
            ),
        )
        .with_details(details)
        .with_ai_guidance(format!(
            "Module '{module_id}' declares dependency '{dependency_id}' with version \
             constraint '{required}', but the registered version is '{actual}'. Either \
             upgrade the dependency, relax the constraint, or register a compatible version."
        ))
    }

    /// Builder for `ID_TOO_LONG` (Issue #32, PROTOCOL_SPEC §2.1.1, §2.7).
    #[must_use]
    pub fn id_too_long(file_path: &str, module_id: &str) -> Self {
        let mut details = HashMap::new();
        details.insert("file_path".to_string(), serde_json::json!(file_path));
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        Self::new(
            ErrorCode::IdTooLong,
            format!(
                "Derived module_id '{module_id}' exceeds maximum length of {} characters",
                crate::registry::multi_class::MAX_MODULE_ID_LEN
            ),
        )
        .with_details(details)
    }
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}", self.code, self.message)
    }
}

impl std::error::Error for ModuleError {}

impl From<serde_json::Error> for ModuleError {
    fn from(err: serde_json::Error) -> Self {
        ModuleError::new(ErrorCode::GeneralInvalidInput, err.to_string())
    }
}

// ---------------------------------------------------------------------------
// Named error types using thiserror
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
#[error("Schema validation error: {message}")]
pub struct SchemaValidationError {
    pub message: String,
    pub errors: Vec<HashMap<String, String>>,
}

impl SchemaValidationError {
    pub fn new(message: impl Into<String>, errors: Vec<HashMap<String, String>>) -> Self {
        Self {
            message: message.into(),
            errors,
        }
    }

    #[must_use]
    pub fn to_module_error(&self) -> ModuleError {
        let mut details = HashMap::new();
        let errors_json: Vec<serde_json::Value> = self
            .errors
            .iter()
            .map(|error| {
                let location = error.get("path").or_else(|| error.get("field")).map_or("", String::as_str);
                let path = if location.is_empty() || location.starts_with('/') { location.to_string() }
                    else { format!("/{}", location.replace('[', ".").replace(']', "").split('.')
                        .filter(|segment| !segment.is_empty()).map(|segment| segment.replace('~', "~0").replace('/', "~1"))
                        .collect::<Vec<_>>().join("/")) };
                serde_json::json!({"path":path,
                    "keyword":error.get("keyword").or_else(|| error.get("constraint")).map_or("schema", String::as_str),
                    "message":error.get("message").filter(|message| !message.is_empty()).map_or("Schema validation failed", String::as_str),
                })
            })
            .collect();
        details.insert("errors".to_string(), serde_json::Value::Array(errors_json));
        ModuleError::new(ErrorCode::SchemaValidationError, &self.message)
            .with_details(details)
            .with_ai_guidance(
                "Input failed schema validation. \
                 Check the 'errors' field in details for specific validation failures.",
            )
    }
}

#[derive(Debug, Error)]
#[error("Schema circular ref: {message}")]
pub struct SchemaCircularRefError {
    pub message: String,
    pub ref_path: String,
}

impl SchemaCircularRefError {
    pub fn new(message: impl Into<String>, ref_path: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ref_path: ref_path.into(),
        }
    }

    #[must_use]
    pub fn to_module_error(&self) -> ModuleError {
        let mut details = HashMap::new();
        details.insert(
            "ref_path".to_string(),
            serde_json::Value::String(self.ref_path.clone()),
        );
        ModuleError::new(ErrorCode::SchemaCircularRef, &self.message).with_details(details)
    }
}

#[derive(Debug, Error)]
#[error("Version incompatible: {message}")]
pub struct VersionIncompatibleError {
    pub message: String,
}

impl VersionIncompatibleError {
    #[must_use]
    pub fn to_module_error(&self) -> ModuleError {
        ModuleError::new(ErrorCode::VersionIncompatible, &self.message)
    }
}

#[derive(Debug, Error)]
#[error("Error code collision: {message}")]
pub struct ErrorCodeCollisionError {
    pub message: String,
    pub code: String,
    pub module_id: String,
    pub conflict_source: String,
}

impl ErrorCodeCollisionError {
    pub fn new(
        message: impl Into<String>,
        code: impl Into<String>,
        module_id: impl Into<String>,
        conflict_source: impl Into<String>,
    ) -> Self {
        Self {
            message: message.into(),
            code: code.into(),
            module_id: module_id.into(),
            conflict_source: conflict_source.into(),
        }
    }

    #[must_use]
    pub fn to_module_error(&self) -> ModuleError {
        let mut details = HashMap::new();
        details.insert(
            "code".to_string(),
            serde_json::Value::String(self.code.clone()),
        );
        details.insert(
            "module_id".to_string(),
            serde_json::Value::String(self.module_id.clone()),
        );
        details.insert(
            "conflict_source".to_string(),
            serde_json::Value::String(self.conflict_source.clone()),
        );
        ModuleError::new(ErrorCode::ErrorCodeCollision, &self.message).with_details(details)
    }
}

// ---------------------------------------------------------------------------
// ErrorCodeRegistry — tracks module error codes and detects collisions
// ---------------------------------------------------------------------------

/// Registry that tracks which error codes belong to which modules,
/// enforcing framework-prefix reservation and cross-module uniqueness.
///
/// Matches the Python `ErrorCodeRegistry` for conformance testing.
pub struct ErrorCodeRegistry {
    module_codes: HashMap<String, HashSet<String>>,
    all_codes: HashSet<String>,
}

impl ErrorCodeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            module_codes: HashMap::new(),
            // Seed with the framework code set so `all_codes()` returns
            // framework + module codes (mirrors Python/TS `_all_codes`).
            all_codes: FRAMEWORK_CODES.clone(),
        }
    }

    /// Register a set of error codes for the given module.
    ///
    /// Returns `Err(ModuleError)` with `ErrorCode::ErrorCodeCollision` if any
    /// code uses a reserved framework prefix or is already owned by a different
    /// module. Re-registering the same code for the same module is idempotent.
    pub fn register(
        &mut self,
        module_id: &str,
        codes: &HashSet<String>,
    ) -> Result<(), ModuleError> {
        for code in codes {
            // Check exact-code collision with a framework code. This protects
            // framework codes whose names fall outside the 14 reserved prefixes
            // (e.g. CIRCUIT_BREAKER_OPEN, PIPELINE_STEP_ERROR, STEP_NOT_FOUND,
            // STRATEGY_NOT_FOUND, RELOAD_FAILED). Mirrors Python `_FRAMEWORK_CODES`
            // and TypeScript `FRAMEWORK_CODES`.
            if FRAMEWORK_CODES.contains(code) {
                return Err(ErrorCodeCollisionError::new(
                    format!("Error code '{code}' is a reserved framework code"),
                    code,
                    module_id,
                    "framework",
                )
                .to_module_error());
            }

            // Check framework prefix collision
            for prefix in FRAMEWORK_ERROR_CODE_PREFIXES {
                if code.starts_with(prefix) {
                    return Err(ErrorCodeCollisionError::new(
                        format!("Error code '{code}' uses reserved framework prefix '{prefix}'"),
                        code,
                        module_id,
                        "framework",
                    )
                    .to_module_error());
                }
            }

            // Check cross-module collision
            if let Some(owner) = self.find_owner(code) {
                if owner != module_id {
                    return Err(ErrorCodeCollisionError::new(
                        format!("Error code '{code}' already registered by module '{owner}'"),
                        code,
                        module_id,
                        &owner,
                    )
                    .to_module_error());
                }
            }
        }

        let existing = self.module_codes.entry(module_id.to_string()).or_default();
        existing.extend(codes.iter().cloned());

        self.rebuild_all_codes();
        Ok(())
    }

    /// Remove all error codes registered for the given module.
    pub fn unregister(&mut self, module_id: &str) {
        self.module_codes.remove(module_id);
        self.rebuild_all_codes();
    }

    /// Find the module that owns the given error code, if any.
    fn find_owner(&self, code: &str) -> Option<String> {
        for (mid, codes) in &self.module_codes {
            if codes.contains(code) {
                return Some(mid.clone());
            }
        }
        None
    }

    /// Rebuild the `all_codes` set from the framework codes plus the current
    /// `module_codes`, so `all_codes()` always returns framework + module codes.
    fn rebuild_all_codes(&mut self) {
        let mut all = FRAMEWORK_CODES.clone();
        all.extend(
            self.module_codes
                .values()
                .flat_map(|codes| codes.iter().cloned()),
        );
        self.all_codes = all;
    }

    /// Returns a reference to the set of all currently registered codes.
    #[must_use]
    pub fn all_codes(&self) -> &HashSet<String> {
        &self.all_codes
    }

    /// Returns the codes registered for a specific module, if any.
    #[must_use]
    pub fn codes_for_module(&self, module_id: &str) -> Option<&HashSet<String>> {
        self.module_codes.get(module_id)
    }
}

impl Default for ErrorCodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
