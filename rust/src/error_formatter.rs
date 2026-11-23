// APCore Protocol — ErrorFormatterRegistry (§8.8)
// Allows adapter-specific error serialization while retaining a default fallback.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::errors::ModuleError;

/// Trait for adapter-specific error formatters.
///
/// Implementors receive a `ModuleError` and an optional opaque context value and
/// return a `serde_json::Value` suitable for the target transport / adapter.
pub trait ErrorFormatter: Send + Sync {
    fn format(&self, error: &ModuleError, context: Option<&dyn std::any::Any>)
        -> serde_json::Value;
}

static FORMATTER_REGISTRY: OnceLock<RwLock<HashMap<String, Arc<dyn ErrorFormatter>>>> =
    OnceLock::new();

fn global_formatters() -> &'static RwLock<HashMap<String, Arc<dyn ErrorFormatter>>> {
    FORMATTER_REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Registry for per-adapter `ErrorFormatter` implementations.
pub struct ErrorFormatterRegistry;

impl ErrorFormatterRegistry {
    /// Register a formatter for the given adapter name.
    ///
    /// Returns `Err(ModuleError::error_formatter_duplicate)` if a formatter for
    /// `adapter_name` is already registered.
    pub fn register(
        adapter_name: &str,
        formatter: Box<dyn ErrorFormatter>,
    ) -> Result<(), ModuleError> {
        let mut map = global_formatters().write();

        if map.contains_key(adapter_name) {
            return Err(ModuleError::error_formatter_duplicate(adapter_name));
        }
        map.insert(adapter_name.to_string(), Arc::from(formatter));
        Ok(())
    }

    /// Return the formatter registered for `adapter_name`, if any.
    #[must_use]
    pub fn get(adapter_name: &str) -> Option<Arc<dyn ErrorFormatter>> {
        global_formatters().read().get(adapter_name).cloned()
    }

    /// Format an error using the formatter registered for `adapter_name`.
    ///
    /// Falls back to `error.to_dict()` if no formatter is registered for the adapter.
    #[must_use]
    pub fn format(
        adapter_name: &str,
        error: &ModuleError,
        context: Option<&dyn std::any::Any>,
    ) -> serde_json::Value {
        let map = global_formatters().read();
        match map.get(adapter_name) {
            Some(formatter) => formatter.format(error, context),
            None => error.to_dict(),
        }
    }

    /// Returns true if a formatter is registered for `adapter_name`.
    #[must_use]
    pub fn is_registered(adapter_name: &str) -> bool {
        global_formatters().read().contains_key(adapter_name)
    }

    /// Returns the list of registered adapter names.
    #[must_use]
    pub fn registered_adapters() -> Vec<String> {
        global_formatters().read().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PrefixFormatter {
        prefix: String,
    }

    impl ErrorFormatter for PrefixFormatter {
        fn format(
            &self,
            error: &ModuleError,
            _context: Option<&dyn std::any::Any>,
        ) -> serde_json::Value {
            serde_json::json!({
                "adapter_prefix": self.prefix,
                "message": error.message,
            })
        }
    }

    #[test]
    fn test_error_formatter_registry_format_fallback() {
        // An unregistered adapter falls back to to_dict().
        let error = ModuleError::new(crate::errors::ErrorCode::GeneralInternalError, "oops");
        let result = ErrorFormatterRegistry::format("nonexistent_adapter_xyz", &error, None);
        // The fallback must include the "message" key from to_dict().
        assert_eq!(result["message"], serde_json::json!("oops"));
    }

    #[test]
    fn test_error_formatter_registry_register_and_format() {
        let adapter = "test_adapter_unique_abc123";
        // Ensure not already registered (fresh state in test).
        if !ErrorFormatterRegistry::is_registered(adapter) {
            let formatter = Box::new(PrefixFormatter {
                prefix: "TEST".to_string(),
            });
            ErrorFormatterRegistry::register(adapter, formatter).unwrap();
        }

        let error = ModuleError::new(crate::errors::ErrorCode::GeneralInternalError, "fail");
        let result = ErrorFormatterRegistry::format(adapter, &error, None);
        assert_eq!(result["adapter_prefix"], serde_json::json!("TEST"));
        assert_eq!(result["message"], serde_json::json!("fail"));
    }

    #[test]
    fn test_error_formatter_registry_duplicate_returns_error() {
        let adapter = "test_adapter_dup_xyz987";
        let formatter1 = Box::new(PrefixFormatter {
            prefix: "A".to_string(),
        });
        let formatter2 = Box::new(PrefixFormatter {
            prefix: "B".to_string(),
        });
        // Only register if not already registered.
        if !ErrorFormatterRegistry::is_registered(adapter) {
            ErrorFormatterRegistry::register(adapter, formatter1).unwrap();
        }
        let result = ErrorFormatterRegistry::register(adapter, formatter2);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, crate::errors::ErrorCode::ErrorFormatterDuplicate);
    }
}
