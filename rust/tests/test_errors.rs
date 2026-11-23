//! Tests for ModuleError and ErrorCode.

use apcore::errors::{retryable_for_code, user_fixable_for_code, ErrorCode, ModuleError};

// ---------------------------------------------------------------------------
// ErrorCode
// ---------------------------------------------------------------------------

#[test]
fn test_error_code_equality() {
    assert_eq!(ErrorCode::ModuleNotFound, ErrorCode::ModuleNotFound);
    assert_ne!(ErrorCode::ModuleNotFound, ErrorCode::ACLDenied);
}

#[test]
fn test_error_code_serialization() {
    let code = ErrorCode::SchemaValidationError;
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, "\"SCHEMA_VALIDATION_ERROR\"");
}

#[test]
fn test_error_code_deserialization() {
    let code: ErrorCode = serde_json::from_str("\"ACL_DENIED\"").unwrap();
    assert_eq!(code, ErrorCode::ACLDenied);
}

/// Regression for sync ERR-001: CircuitBreakerOpen must serialize as
/// `CIRCUIT_BREAKER_OPEN` to align with apcore-python and apcore-typescript.
#[test]
fn test_circuit_breaker_open_serde_rename_aligns_cross_language() {
    let json = serde_json::to_value(ErrorCode::CircuitBreakerOpen).unwrap();
    assert_eq!(json, serde_json::json!("CIRCUIT_BREAKER_OPEN"));
    let parsed: ErrorCode = serde_json::from_str("\"CIRCUIT_BREAKER_OPEN\"").unwrap();
    assert_eq!(parsed, ErrorCode::CircuitBreakerOpen);
}

#[test]
fn test_all_error_codes_defined() {
    // Verify the full set matches the protocol spec (37 codes).
    let codes = [
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
        ErrorCode::CallDepthExceeded,
        ErrorCode::CircularCall,
        ErrorCode::CallFrequencyExceeded,
        ErrorCode::GeneralInvalidInput,
        ErrorCode::GeneralInternalError,
        ErrorCode::GeneralNotImplemented,
        ErrorCode::FuncMissingTypeHint,
        ErrorCode::FuncMissingReturnType,
        ErrorCode::BindingInvalidTarget,
        ErrorCode::BindingModuleNotFound,
        ErrorCode::BindingCallableNotFound,
        ErrorCode::BindingNotCallable,
        ErrorCode::BindingSchemaMissing,
        ErrorCode::BindingFileInvalid,
        ErrorCode::CircularDependency,
        ErrorCode::MiddlewareChainError,
        ErrorCode::ApprovalDenied,
        ErrorCode::ApprovalTimeout,
        ErrorCode::ApprovalPending,
        ErrorCode::VersionIncompatible,
        ErrorCode::ErrorCodeCollision,
        ErrorCode::DependencyNotFound,
    ];
    assert_eq!(codes.len(), 37, "Protocol defines exactly 37 error codes");
}

// ---------------------------------------------------------------------------
// ModuleError
// ---------------------------------------------------------------------------

#[test]
fn test_module_error_basic_fields() {
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "module 'foo' not found");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
    assert_eq!(err.message, "module 'foo' not found");
}

#[test]
fn test_module_error_retryable_resolved_from_code() {
    let err = ModuleError::new(ErrorCode::GeneralInternalError, "oops");
    assert_eq!(err.retryable, Some(true));

    let acl_denied = ModuleError::new(ErrorCode::ACLDenied, "no");
    assert_eq!(acl_denied.retryable, Some(false));

    let unset = ModuleError::new(ErrorCode::ModuleExecuteError, "boom");
    assert_eq!(unset.retryable, None);
}

#[test]
fn test_retryable_for_code_matches_governance_timeout_semantics() {
    // The distinction an autonomous caller most needs: a human said no
    // (terminal) versus nobody answered (a later attempt may succeed).
    assert_eq!(retryable_for_code(ErrorCode::ApprovalDenied), Some(false));
    assert_eq!(retryable_for_code(ErrorCode::ApprovalTimeout), Some(true));
}

#[test]
fn test_retryable_for_code_covers_the_same_extras_as_user_fixable() {
    // `INVALID_PARENT_ID` is the one code carried beyond PROTOCOL_SPEC §8.6 and
    // the §9.12.4 config family. The two policies must not disagree about a
    // code either of them knows, and apcore-python pins the same value on the
    // class.
    assert_eq!(retryable_for_code(ErrorCode::InvalidParentId), Some(false));
    assert_eq!(
        user_fixable_for_code(ErrorCode::InvalidParentId),
        Some(true)
    );
}

/// PROTOCOL_SPEC §8.6's classification table, transcribed row for row.
///
/// The table is prose in the spec repo, so nothing mechanical keeps this map
/// honest — this test is that mechanism. A row here that stops matching means
/// either the spec moved (update both) or the map drifted (fix the map).
#[test]
fn test_retryable_for_code_transcribes_the_spec_8_6_table() {
    // §8.6 "Yes" — the missing precondition is time.
    for code in [
        ErrorCode::ModuleTimeout,
        ErrorCode::GeneralInternalError,
        ErrorCode::ApprovalTimeout,
        ErrorCode::ReloadFailed,
    ] {
        assert_eq!(
            retryable_for_code(code),
            Some(true),
            "{code:?} is §8.6 \"Yes\""
        );
    }

    // §8.6 "No" — retrying reproduces the same failure.
    for code in [
        ErrorCode::ConfigNotFound,
        ErrorCode::ConfigInvalid,
        ErrorCode::ACLRuleError,
        ErrorCode::ACLDenied,
        ErrorCode::ApprovalDenied,
        ErrorCode::ApprovalPending,
        ErrorCode::ModuleNotFound,
        ErrorCode::ModuleDisabled,
        ErrorCode::ModuleLoadError,
        ErrorCode::SchemaValidationError,
        ErrorCode::SchemaNotFound,
        ErrorCode::SchemaParseError,
        ErrorCode::SchemaCircularRef,
        ErrorCode::SchemaMaxDepthExceeded,
        ErrorCode::SchemaUnionNoMatch,
        ErrorCode::SchemaUnionAmbiguous,
        ErrorCode::CallDepthExceeded,
        ErrorCode::CircularCall,
        ErrorCode::CallFrequencyExceeded,
        ErrorCode::GeneralInvalidInput,
        ErrorCode::FuncMissingTypeHint,
        ErrorCode::FuncMissingReturnType,
        ErrorCode::BindingInvalidTarget,
        ErrorCode::BindingModuleNotFound,
        ErrorCode::BindingCallableNotFound,
        ErrorCode::BindingNotCallable,
        ErrorCode::BindingSchemaMissing,
        ErrorCode::BindingFileInvalid,
        ErrorCode::CircularDependency,
        ErrorCode::MiddlewareChainError,
        ErrorCode::VersionIncompatible,
        ErrorCode::ErrorCodeCollision,
    ] {
        assert_eq!(
            retryable_for_code(code),
            Some(false),
            "{code:?} is §8.6 \"No\""
        );
    }

    // §8.6 "Depends" (on `annotations.idempotent`) is exactly what `None` means
    // here: the framework declines to answer and the module author supplies it.
    assert_eq!(retryable_for_code(ErrorCode::ModuleExecuteError), None);
}

/// PROTOCOL_SPEC §9.12.4: "All config errors ... are non-retryable
/// (`retryable = false`)".
///
/// Swept over [`ErrorCode::ALL`] rather than listed, so a config code added
/// later fails here until its `retryable` is pinned — the categorical MUST is
/// what makes that the right default, and `CONFIG_KEY_RESTRICTED` (a config
/// error the §9.12.4 table does not list) is why a hand-kept list would have
/// already been short by one.
#[test]
fn test_every_config_error_code_is_non_retryable() {
    let mut checked = 0_usize;
    for &code in ErrorCode::ALL {
        if !code.wire_str().starts_with("CONFIG_") {
            continue;
        }
        checked += 1;
        assert_eq!(
            retryable_for_code(code),
            Some(false),
            "{} is a config error: §9.12.4 requires retryable = false",
            code.wire_str()
        );
    }
    assert!(
        checked >= 9,
        "expected the whole CONFIG_* family, saw {checked}"
    );
}

/// D-135: `EXECUTION_CANCELLED` is retryable from its code — a cancelled call
/// may be sent again with a fresh `CancelToken`.
#[test]
fn test_execution_cancelled_is_retryable_by_code() {
    assert_eq!(
        retryable_for_code(ErrorCode::ExecutionCancelled),
        Some(true)
    );
    assert_eq!(
        ModuleError::new(ErrorCode::ExecutionCancelled, "cancelled").retryable,
        Some(true)
    );
}

#[test]
fn test_acl_denied_default_guidance_and_plain_caller_message() {
    let err = ModuleError::acl_denied(Some("api.gateway"), "cli.rm");
    assert_eq!(err.message, "Access denied: api.gateway -> cli.rm");
    assert_eq!(
        err.ai_guidance.as_deref(),
        Some(
            "Access denied for 'api.gateway' calling 'cli.rm'. Verify the caller has the required role or permission, or try an alternative module with similar functionality."
        )
    );
    // Structured detail, as apcore-python and apcore-typescript both attach.
    assert_eq!(err.details["caller_id"], serde_json::json!("api.gateway"));
    assert_eq!(err.details["target_id"], serde_json::json!("cli.rm"));
    assert_eq!(err.retryable, Some(false));
    assert_eq!(err.user_fixable, Some(false));
}

#[test]
fn test_acl_denied_absent_caller_names_the_sentinel_the_acl_matched() {
    // An unauthenticated caller has no id, but it is not unidentified to the
    // ACL: `check_access` resolves `None` to `@external` before matching, and
    // the AuditEntry for this denial records `@external`. The prose names that
    // same identity so the guidance points at a rule an operator can actually
    // write (`@external` is matched as an exact literal — no wildcard reaches
    // it). `details.caller_id` stays null: that is the raw wire value, and it
    // is what both peers put there.
    let err = ModuleError::acl_denied(None, "cli.rm");
    assert_eq!(err.message, "Access denied: @external -> cli.rm");
    assert_eq!(
        err.ai_guidance.as_deref(),
        Some(
            "Access denied for '@external' calling 'cli.rm'. Verify the caller has the required role or permission, or try an alternative module with similar functionality."
        )
    );
    assert_eq!(err.details["caller_id"], serde_json::Value::Null);
    assert_eq!(err.details["target_id"], serde_json::json!("cli.rm"));

    // The spelling is the crate's one sentinel, not a literal typed twice.
    assert_eq!(apcore::EXTERNAL_CALLER, "@external");
    assert!(err.message.contains(apcore::EXTERNAL_CALLER));
}

#[test]
fn test_acl_denied_guidance_can_be_overridden() {
    let err = ModuleError::acl_denied(Some("api.gateway"), "cli.rm")
        .with_ai_guidance("use a safer module");
    assert_eq!(err.ai_guidance.as_deref(), Some("use a safer module"));
}

#[test]
fn test_module_error_user_fixable_resolved_from_code() {
    // GENERAL_INTERNAL_ERROR is governance/system — not caller-fixable by input.
    // Resolved from the code at construction (parity with apcore-python
    // `_USER_FIXABLE_BY_CODE`).
    let err = ModuleError::new(ErrorCode::GeneralInternalError, "oops");
    assert_eq!(err.user_fixable, Some(false));

    // A code absent from the policy leaves user_fixable unset for the module
    // author to supply.
    let unset = ModuleError::new(ErrorCode::ModuleExecuteError, "boom");
    assert_eq!(unset.user_fixable, None);
}

#[test]
fn test_module_error_no_cause_by_default() {
    let err = ModuleError::new(ErrorCode::GeneralInternalError, "oops");
    assert!(err.cause.is_none());
}

#[test]
fn test_module_error_no_trace_id_by_default() {
    let err = ModuleError::new(ErrorCode::GeneralInternalError, "oops");
    assert!(err.trace_id.is_none());
}

#[test]
fn test_module_error_display() {
    let err = ModuleError::new(ErrorCode::ACLDenied, "access denied");
    let s = format!("{err}");
    assert!(s.contains("ACLDenied"));
    assert!(s.contains("access denied"));
}

#[test]
fn test_module_error_details_empty_by_default() {
    let err = ModuleError::new(ErrorCode::GeneralInvalidInput, "bad input");
    assert!(err.details.is_empty());
}

#[test]
fn test_module_error_with_details() {
    let mut err = ModuleError::new(ErrorCode::SchemaValidationError, "field missing");
    err.details
        .insert("field".to_string(), serde_json::json!("user_id"));
    assert_eq!(err.details["field"], "user_id");
}

#[test]
fn test_module_error_serialization_round_trip() {
    let err = ModuleError::new(ErrorCode::ModuleTimeout, "timed out after 30s");
    let json = serde_json::to_string(&err).unwrap();
    let restored: ModuleError = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.code, ErrorCode::ModuleTimeout);
    assert_eq!(restored.message, "timed out after 30s");
}

#[test]
fn test_module_error_is_std_error() {
    let err = ModuleError::new(ErrorCode::GeneralInternalError, "oops");
    // Verify it satisfies std::error::Error
    let _: &dyn std::error::Error = &err;
}

// ---------------------------------------------------------------------------
// A-D-015: MiddlewareChainError unwrap recovers the original typed error
// ---------------------------------------------------------------------------

#[test]
fn unwrap_middleware_chain_error_recovers_inner() {
    let inner = ModuleError::new(ErrorCode::ApprovalDenied, "approval rejected");
    let mut details = std::collections::HashMap::new();
    details.insert(
        "inner_error".to_string(),
        serde_json::to_value(&inner).unwrap(),
    );
    let wrapped = ModuleError::new(ErrorCode::MiddlewareChainError, inner.message.clone())
        .with_details(details);

    let recovered = wrapped.unwrap_middleware_chain_error().expect("unwrap");
    assert_eq!(recovered.code, ErrorCode::ApprovalDenied);
    assert_eq!(recovered.message, "approval rejected");
}

#[test]
fn unwrap_middleware_chain_error_returns_none_for_other_codes() {
    let err = ModuleError::new(ErrorCode::ModuleTimeout, "timeout");
    assert!(err.unwrap_middleware_chain_error().is_none());
}

#[test]
fn unwrap_middleware_chain_error_returns_none_when_inner_missing() {
    let err = ModuleError::new(ErrorCode::MiddlewareChainError, "no inner");
    assert!(err.unwrap_middleware_chain_error().is_none());
}

// ---------------------------------------------------------------------------
// A-D-006 + A-D-007: 14 canonical reserved prefixes + exact framework-code
// collision check.
// ---------------------------------------------------------------------------

use apcore::errors::{ErrorCodeRegistry, FRAMEWORK_ERROR_CODE_PREFIXES};
use std::collections::HashSet;

fn code_set(codes: &[&str]) -> HashSet<String> {
    codes.iter().map(|s| (*s).to_string()).collect()
}

/// A-D-006: the reserved-prefix set is exactly the canonical 14, with the
/// four non-canonical prefixes (CIRCUIT_, PIPELINE_, STEP_, STRATEGY_) dropped.
#[test]
fn test_reserved_prefixes_are_canonical_14() {
    let expected: HashSet<&str> = [
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
    ]
    .into_iter()
    .collect();
    let actual: HashSet<&str> = FRAMEWORK_ERROR_CODE_PREFIXES.iter().copied().collect();
    assert_eq!(actual, expected);
    assert_eq!(FRAMEWORK_ERROR_CODE_PREFIXES.len(), 14);
    for dropped in ["CIRCUIT_", "PIPELINE_", "STEP_", "STRATEGY_"] {
        assert!(!actual.contains(dropped), "{dropped} must not be reserved");
    }
}

/// A-D-006: a custom code with a `STEP_` prefix that is NOT a framework code
/// now registers successfully (prefix no longer reserved).
#[test]
fn test_register_step_custom_succeeds_after_prefix_narrowing() {
    let mut reg = ErrorCodeRegistry::new();
    let result = reg.register("m", &code_set(&["STEP_CUSTOM"]));
    assert!(result.is_ok(), "STEP_CUSTOM should register: {result:?}");
}

/// A-D-007: an exact framework code that no longer matches any prefix
/// (`STEP_NOT_FOUND`) is still rejected via the exact-code check.
#[test]
fn test_register_step_not_found_rejected_as_framework_code() {
    let mut reg = ErrorCodeRegistry::new();
    let err = reg
        .register("m", &code_set(&["STEP_NOT_FOUND"]))
        .expect_err("STEP_NOT_FOUND is a framework code");
    assert_eq!(err.code, ErrorCode::ErrorCodeCollision);
    assert_eq!(
        err.details.get("conflict_source").and_then(|v| v.as_str()),
        Some("framework")
    );
}

/// A-D-007: a framework code with no reserved prefix at all (`RELOAD_FAILED`)
/// is rejected by the exact-code check.
#[test]
fn test_register_reload_failed_rejected_as_framework_code() {
    let mut reg = ErrorCodeRegistry::new();
    let err = reg
        .register("m", &code_set(&["RELOAD_FAILED"]))
        .expect_err("RELOAD_FAILED is a framework code");
    assert_eq!(err.code, ErrorCode::ErrorCodeCollision);
    assert_eq!(
        err.details.get("conflict_source").and_then(|v| v.as_str()),
        Some("framework")
    );
}

/// A-D-007: CIRCUIT_BREAKER_OPEN / PIPELINE_STEP_ERROR / STRATEGY_NOT_FOUND
/// remain protected by the exact-code check after prefix narrowing.
#[test]
fn test_register_non_prefix_framework_codes_rejected() {
    for code in [
        "CIRCUIT_BREAKER_OPEN",
        "PIPELINE_STEP_ERROR",
        "STRATEGY_NOT_FOUND",
    ] {
        let mut reg = ErrorCodeRegistry::new();
        assert!(
            reg.register("m", &code_set(&[code])).is_err(),
            "{code} must be rejected as framework code"
        );
    }
}

/// A-D-021: a fresh registry (no modules) seeds `all_codes` with the framework
/// code set, so it is non-empty and contains framework codes.
#[test]
fn test_fresh_registry_all_codes_contains_framework_codes() {
    let reg = ErrorCodeRegistry::new();
    assert!(!reg.all_codes().is_empty());
    assert!(reg.all_codes().contains("SCHEMA_VALIDATION_ERROR"));
    assert!(reg.all_codes().contains("CIRCUIT_BREAKER_OPEN"));
    assert!(reg.all_codes().contains("RELOAD_FAILED"));
}

/// A-D-021: after registering module codes, `all_codes` still contains the
/// framework codes (rebuild must not drop them).
#[test]
fn test_all_codes_retains_framework_after_module_register() {
    let mut reg = ErrorCodeRegistry::new();
    reg.register("m", &code_set(&["MY_CUSTOM_CODE"])).unwrap();
    assert!(reg.all_codes().contains("MY_CUSTOM_CODE"));
    assert!(reg.all_codes().contains("SCHEMA_VALIDATION_ERROR"));
}

/// D-135: `CIRCUIT_BREAKER_OPEN` is retryable from its code, not only through
/// the middleware's builder — the circuit re-probes after its recovery window.
#[test]
fn test_circuit_breaker_open_is_retryable_by_code_and_builder() {
    assert_eq!(
        retryable_for_code(ErrorCode::CircuitBreakerOpen),
        Some(true)
    );
    assert_eq!(
        ModuleError::new(ErrorCode::CircuitBreakerOpen, "open").retryable,
        Some(true)
    );

    let err = ModuleError::circuit_breaker_open("billing.charge", "api.gateway");
    assert_eq!(err.code, ErrorCode::CircuitBreakerOpen);
    assert_eq!(
        err.retryable,
        Some(true),
        "the circuit re-probes after the recovery window, so the call may succeed later"
    );
    assert_eq!(
        err.details["module_id"],
        serde_json::json!("billing.charge")
    );
    assert_eq!(err.details["caller_id"], serde_json::json!("api.gateway"));
}
