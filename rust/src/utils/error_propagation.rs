// APCore Protocol — Error propagation (Algorithm A11)
// Spec reference: Error handling and propagation across module boundaries

use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};

/// Wrap a raw error into a standardized `ModuleError` (Algorithm A11).
///
/// If the error is already a `ModuleError`, enriches it with trace context
/// (`trace_id`, `module_id`, `call_chain`) where missing. Otherwise wraps it as a
/// `ModuleExecuteError`.
///
/// # Arguments
///
/// * `error` - The raw error to propagate.
/// * `module_id` - Module ID where the error occurred.
/// * `context` - Current execution context.
///
/// # Returns
///
/// A `ModuleError` with `trace_id`, `module_id`, and `call_chain` attached.
#[allow(clippy::needless_pass_by_value)] // public API: Box<dyn Error> ownership transfer is conventional
pub fn propagate_error<T>(
    error: Box<dyn std::error::Error>,
    module_id: &str,
    context: &Context<T>,
) -> ModuleError {
    // If the error is already a ModuleError, enrich with context if missing
    if let Some(module_error) = error.downcast_ref::<ModuleError>() {
        let mut enriched = module_error.clone();

        if enriched.trace_id.is_none() {
            enriched.trace_id = Some(context.trace_id.clone());
        }

        if !enriched.details.contains_key("module_id") {
            enriched
                .details
                .insert("module_id".to_string(), serde_json::json!(module_id));
        }

        if !enriched.details.contains_key("call_chain") {
            enriched.details.insert(
                "call_chain".to_string(),
                serde_json::json!(context.call_chain),
            );
        }

        return enriched;
    }

    // Wrap raw error as ModuleExecuteError
    let mut wrapped = ModuleError::new(
        ErrorCode::ModuleExecuteError,
        format!("Module '{module_id}' raised: {error}"),
    );
    wrapped.trace_id = Some(context.trace_id.clone());
    wrapped.cause = Some(error.to_string());
    wrapped
        .details
        .insert("module_id".to_string(), serde_json::json!(module_id));
    wrapped.details.insert(
        "call_chain".to_string(),
        serde_json::json!(context.call_chain),
    );

    wrapped
}

/// Convenience overload that accepts a `ModuleError` directly and enriches it
/// with trace context. This avoids the boxing overhead when the caller already
/// has a `ModuleError`.
pub fn propagate_module_error<T>(
    mut error: ModuleError,
    module_id: &str,
    context: &Context<T>,
) -> ModuleError {
    if error.trace_id.is_none() {
        error.trace_id = Some(context.trace_id.clone());
    }

    if !error.details.contains_key("module_id") {
        error
            .details
            .insert("module_id".to_string(), serde_json::json!(module_id));
    }

    if !error.details.contains_key("call_chain") {
        error.details.insert(
            "call_chain".to_string(),
            serde_json::json!(context.call_chain),
        );
    }

    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::errors::{ErrorCode, ModuleError};
    use std::io;

    #[test]
    fn test_propagate_raw_error() {
        let ctx: Context<()> = Context::anonymous();
        let raw_err: Box<dyn std::error::Error> =
            Box::new(io::Error::new(io::ErrorKind::NotFound, "file missing"));

        let result = propagate_error(raw_err, "executor.files.read", &ctx);

        assert_eq!(result.code, ErrorCode::ModuleExecuteError);
        assert!(result.message.contains("executor.files.read"));
        assert!(result.message.contains("file missing"));
        assert_eq!(result.trace_id, Some(ctx.trace_id.clone()));
        assert_eq!(
            result.details.get("module_id"),
            Some(&serde_json::json!("executor.files.read"))
        );
        assert!(result.details.contains_key("call_chain"));
        assert!(result.cause.is_some());
    }

    #[test]
    fn test_propagate_existing_module_error() {
        let ctx: Context<()> = Context::anonymous();
        let original = ModuleError::new(ErrorCode::ConfigNotFound, "config missing");
        let boxed: Box<dyn std::error::Error> = Box::new(original);

        let result = propagate_error(boxed, "executor.config.load", &ctx);

        assert_eq!(result.code, ErrorCode::ConfigNotFound);
        assert_eq!(result.message, "config missing");
        assert_eq!(result.trace_id, Some(ctx.trace_id.clone()));
        assert_eq!(
            result.details.get("module_id"),
            Some(&serde_json::json!("executor.config.load"))
        );
    }

    #[test]
    fn test_propagate_module_error_preserves_existing_trace_id() {
        let ctx: Context<()> = Context::anonymous();
        let original = ModuleError::new(ErrorCode::ModuleTimeout, "timed out")
            .with_trace_id("existing-trace-id");
        let boxed: Box<dyn std::error::Error> = Box::new(original);

        let result = propagate_error(boxed, "executor.slow.task", &ctx);

        // Should preserve the original trace_id, not overwrite
        assert_eq!(result.trace_id, Some("existing-trace-id".to_string()));
    }

    #[test]
    fn test_propagate_module_error_convenience() {
        let ctx: Context<()> = Context::anonymous();
        let original = ModuleError::new(ErrorCode::SchemaValidationError, "bad input");

        let result = propagate_module_error(original, "executor.validate.input", &ctx);

        assert_eq!(result.code, ErrorCode::SchemaValidationError);
        assert_eq!(result.trace_id, Some(ctx.trace_id.clone()));
        assert_eq!(
            result.details.get("module_id"),
            Some(&serde_json::json!("executor.validate.input"))
        );
        assert!(result.details.contains_key("call_chain"));
    }
}
