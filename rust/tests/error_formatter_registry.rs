// Integration tests for ErrorFormatterRegistry (§8.8, v0.15.0)

use apcore::error_formatter::{ErrorFormatter, ErrorFormatterRegistry};
use apcore::errors::{ErrorCode, ModuleError};

struct JsonWrapFormatter;

impl ErrorFormatter for JsonWrapFormatter {
    fn format(
        &self,
        error: &ModuleError,
        _context: Option<&dyn std::any::Any>,
    ) -> serde_json::Value {
        serde_json::json!({
            "wrapped": true,
            "code": format!("{:?}", error.code),
            "message": error.message,
        })
    }
}

/// Generate a deterministic-enough unique adapter name for each test to avoid
/// cross-test interference with the global registry.
fn unique_adapter(base: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{base}_{nanos}")
}

#[test]
fn test_error_formatter_registry_fallback_to_dict() {
    let error = ModuleError::new(ErrorCode::GeneralInternalError, "fallback test");
    let result = ErrorFormatterRegistry::format("adapter_not_registered_xyz", &error, None);
    // Fallback: to_dict() should include "message" key.
    assert_eq!(result["message"], serde_json::json!("fallback test"));
}

#[test]
fn test_error_formatter_registry_register_and_use() {
    let adapter = unique_adapter("wrap_fmt");
    ErrorFormatterRegistry::register(adapter.as_str(), Box::new(JsonWrapFormatter)).unwrap();

    let error = ModuleError::new(ErrorCode::ModuleNotFound, "module gone");
    let result = ErrorFormatterRegistry::format(adapter.as_str(), &error, None);

    assert_eq!(result["wrapped"], serde_json::json!(true));
    assert_eq!(result["message"], serde_json::json!("module gone"));
}

#[test]
fn test_error_formatter_registry_duplicate_registration_fails() {
    let adapter = unique_adapter("dup_fmt");
    ErrorFormatterRegistry::register(adapter.as_str(), Box::new(JsonWrapFormatter)).unwrap();

    let result = ErrorFormatterRegistry::register(adapter.as_str(), Box::new(JsonWrapFormatter));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::ErrorFormatterDuplicate);
    // Details must include the adapter name.
    assert_eq!(
        err.details.get("adapter_name"),
        Some(&serde_json::Value::String(adapter))
    );
}

#[test]
fn test_error_formatter_registry_is_registered() {
    let adapter = unique_adapter("is_reg");
    assert!(!ErrorFormatterRegistry::is_registered(adapter.as_str()));
    ErrorFormatterRegistry::register(adapter.as_str(), Box::new(JsonWrapFormatter)).unwrap();
    assert!(ErrorFormatterRegistry::is_registered(adapter.as_str()));
}

#[test]
fn test_error_formatter_registry_get_returns_none_for_unknown() {
    let result = ErrorFormatterRegistry::get("totally_unknown_adapter_xyz");
    assert!(
        result.is_none(),
        "get() should return None for unregistered adapter"
    );
}

#[test]
fn test_error_formatter_registry_get_returns_formatter() {
    let adapter = unique_adapter("get_fmt");
    ErrorFormatterRegistry::register(adapter.as_str(), Box::new(JsonWrapFormatter)).unwrap();

    let formatter = ErrorFormatterRegistry::get(adapter.as_str());
    assert!(
        formatter.is_some(),
        "get() should return Some for registered adapter"
    );

    let error = ModuleError::new(ErrorCode::GeneralInternalError, "get test");
    let result = formatter.unwrap().format(&error, None);
    assert_eq!(result["wrapped"], serde_json::json!(true));
    assert_eq!(result["message"], serde_json::json!("get test"));
}
