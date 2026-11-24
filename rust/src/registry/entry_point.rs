// APCore Protocol — Entry point resolution for discovered module files
// Spec reference: Module entry point discovery and naming conventions
//
// NOTE: In Rust, modules are compiled at build time — there is no runtime
// file import like Python's `importlib` or TypeScript's `require()`. This
// module provides the naming-convention utilities (snake_to_pascal conversion)
// and entry-point metadata parsing that are needed by registry tooling and
// build-time code generators. Actual dynamic loading (e.g. via `libloading`
// for `.so`/`.dylib` plugins) is left to specific Discoverer implementations.

use std::collections::HashMap;
use std::path::Path;

use crate::errors::{ErrorCode, ModuleError};

/// Convert a `snake_case` string to `PascalCase`.
///
/// Aligned with `apcore-python.snake_to_pascal` and
/// `apcore-typescript.snakeToPascal`.
#[must_use]
pub fn snake_to_pascal(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect()
}

/// Resolve an entry-point class/struct name from metadata.
///
/// If `meta` contains an `"entry_point"` key in the format `"filename:ClassName"`,
/// returns the class name portion. Otherwise returns `None` to signal that
/// auto-inference should be used.
///
/// Aligned with `apcore-python.resolve_entry_point` metadata parsing and
/// `apcore-typescript.resolveEntryPoint`.
#[allow(clippy::implicit_hasher)] // public API: callers always use the default hasher
pub fn resolve_entry_point_name(
    file_path: &Path,
    meta: Option<&HashMap<String, serde_json::Value>>,
) -> Result<Option<String>, ModuleError> {
    if let Some(m) = meta {
        if let Some(ep) = m.get("entry_point") {
            if let Some(ep_str) = ep.as_str() {
                let class_name = ep_str.split(':').next_back().unwrap_or(ep_str);
                if class_name.is_empty() {
                    return Err(ModuleError::new(
                        ErrorCode::ModuleLoadError,
                        format!(
                            "Empty entry point class name in metadata for {}",
                            file_path.display()
                        ),
                    ));
                }
                return Ok(Some(class_name.to_string()));
            }
        }
    }
    Ok(None)
}

/// Derive a `PascalCase` struct name from a file stem.
///
/// E.g., `"send_email"` -> `"SendEmail"`, `"my_module"` -> `"MyModule"`.
/// Useful for code generators that need a conventional struct name from a
/// discovered file path.
#[must_use]
pub fn infer_struct_name(file_path: &Path) -> String {
    let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    snake_to_pascal(stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_snake_to_pascal() {
        assert_eq!(snake_to_pascal("send_email"), "SendEmail");
        assert_eq!(snake_to_pascal("my_module"), "MyModule");
        assert_eq!(snake_to_pascal("hello"), "Hello");
        assert_eq!(snake_to_pascal(""), "");
        assert_eq!(snake_to_pascal("a_b_c"), "ABC");
    }

    #[test]
    fn test_resolve_entry_point_with_meta() {
        let mut meta = HashMap::new();
        meta.insert(
            "entry_point".to_string(),
            serde_json::json!("my_file:MyClass"),
        );
        let path = PathBuf::from("my_file.rs");
        let result = resolve_entry_point_name(&path, Some(&meta)).unwrap();
        assert_eq!(result, Some("MyClass".to_string()));
    }

    #[test]
    fn test_resolve_entry_point_no_meta() {
        let path = PathBuf::from("my_file.rs");
        let result = resolve_entry_point_name(&path, None).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_infer_struct_name() {
        assert_eq!(
            infer_struct_name(&PathBuf::from("send_email.rs")),
            "SendEmail"
        );
        assert_eq!(
            infer_struct_name(&PathBuf::from("/foo/bar/my_module.rs")),
            "MyModule"
        );
    }
}
