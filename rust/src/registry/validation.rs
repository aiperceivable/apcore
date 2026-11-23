// APCore Protocol — Registry validation utilities
// Spec reference: Module interface validation

/// Validate that a module descriptor contains the required fields.
///
/// Returns a list of validation error strings. Empty list means valid.
///
/// This is the Rust equivalent of the Python/TypeScript `validate_module`
/// function. Since Rust modules are statically typed, the primary use case
/// is validating dynamically loaded descriptors or JSON-based module
/// definitions rather than runtime type inspection.
///
/// Aligned with `apcore-python.validate_module` and
/// `apcore-typescript.validateModule`.
#[must_use]
pub fn validate_descriptor(descriptor: &serde_json::Value) -> Vec<String> {
    let mut errors = Vec::new();

    // Check input_schema
    match descriptor
        .get("input_schema")
        .or_else(|| descriptor.get("inputSchema"))
    {
        None => {
            errors
                .push("Missing or invalid input_schema: must be a JSON Schema object".to_string());
        }
        Some(v) if !v.is_object() => {
            errors
                .push("Missing or invalid input_schema: must be a JSON Schema object".to_string());
        }
        _ => {}
    }

    // Check output_schema
    match descriptor
        .get("output_schema")
        .or_else(|| descriptor.get("outputSchema"))
    {
        None => errors
            .push("Missing or invalid output_schema: must be a JSON Schema object".to_string()),
        Some(v) if !v.is_object() => errors
            .push("Missing or invalid output_schema: must be a JSON Schema object".to_string()),
        _ => {}
    }

    // Check description
    match descriptor.get("description") {
        None => errors.push("Missing or empty description".to_string()),
        Some(v) => {
            if let Some(s) = v.as_str() {
                if s.is_empty() {
                    errors.push("Missing or empty description".to_string());
                }
            } else {
                errors.push("Missing or empty description".to_string());
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_descriptor() {
        let desc = serde_json::json!({
            "input_schema": {"type": "object"},
            "output_schema": {"type": "object"},
            "description": "A test module"
        });
        assert_eq!(validate_descriptor(&desc), Vec::<String>::new());
    }

    #[test]
    fn test_missing_input_schema() {
        let desc = serde_json::json!({
            "output_schema": {"type": "object"},
            "description": "A test module"
        });
        let errors = validate_descriptor(&desc);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("input_schema"));
    }

    #[test]
    fn test_missing_description() {
        let desc = serde_json::json!({
            "input_schema": {"type": "object"},
            "output_schema": {"type": "object"}
        });
        let errors = validate_descriptor(&desc);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("description"));
    }

    #[test]
    fn test_empty_description() {
        let desc = serde_json::json!({
            "input_schema": {"type": "object"},
            "output_schema": {"type": "object"},
            "description": ""
        });
        let errors = validate_descriptor(&desc);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("description"));
    }

    #[test]
    fn test_all_missing() {
        let desc = serde_json::json!({});
        let errors = validate_descriptor(&desc);
        assert_eq!(errors.len(), 3);
    }
}
