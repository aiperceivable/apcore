// APCore SDK — Integration tests
// Basic smoke tests to verify the skeleton compiles and types are accessible.

use apcore::cancel::CancelToken;
use apcore::context::Identity;
use apcore::errors::ErrorCode;

#[test]
fn test_error_code_variants_exist() {
    // Verify all critical ErrorCode variants are defined.
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
    assert_eq!(codes.len(), 37);
}

#[test]
fn test_cancel_token() {
    let token = CancelToken::new();
    assert!(!token.is_cancelled());
    token.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn test_apcore_from_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("apcore.yaml");
    std::fs::write(
        &config_path,
        "apcore:\n  executor:\n    max_call_depth: 42\n",
    )
    .unwrap();

    let client = apcore::client::APCore::from_path(config_path).unwrap();
    assert_eq!(client.config.executor.max_call_depth, 42);
}

#[test]
fn test_identity_creation() {
    let identity = Identity::new(
        "user-1".to_string(),
        "Test User".to_string(),
        vec!["admin".to_string()],
        std::collections::HashMap::new(),
    );
    assert_eq!(identity.id(), "user-1");
    assert_eq!(identity.roles().len(), 1);
}

#[test]
fn test_module_error_creation() {
    let error =
        apcore::errors::ModuleError::new(ErrorCode::GeneralInternalError, "something went wrong");
    assert_eq!(error.code, ErrorCode::GeneralInternalError);
    assert_eq!(error.retryable, Some(true));
    // user_fixable is resolved from the code: GENERAL_INTERNAL_ERROR is
    // governance/system, so not caller-fixable by input.
    assert_eq!(error.user_fixable, Some(false));
}
