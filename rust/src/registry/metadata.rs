// APCore Protocol — Metadata and ID map loading for the registry system
// Spec reference: §4.13 metadata merge, companion YAML loading

use std::collections::HashMap;
use std::path::Path;

use crate::errors::{ErrorCode, ModuleError};
use crate::registry::types::DepInfo;

/// Load a `*_meta.yaml` companion metadata file.
///
/// Returns an empty map if the file does not exist (metadata is optional).
///
/// Aligned with `apcore-python.load_metadata` and
/// `apcore-typescript.loadMetadata`.
pub fn load_metadata(meta_path: &Path) -> Result<HashMap<String, serde_json::Value>, ModuleError> {
    if !meta_path.exists() {
        return Ok(HashMap::new());
    }

    let content = std::fs::read_to_string(meta_path).map_err(|e| {
        ModuleError::new(
            ErrorCode::ConfigInvalid,
            format!("Cannot read metadata file {}: {}", meta_path.display(), e),
        )
    })?;

    let parsed: serde_json::Value = serde_yaml_ng::from_str(&content).map_err(|e| {
        ModuleError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "Invalid YAML in metadata file {}: {}",
                meta_path.display(),
                e
            ),
        )
    })?;

    match parsed {
        serde_json::Value::Null => Ok(HashMap::new()),
        serde_json::Value::Object(map) => Ok(map.into_iter().collect()),
        _ => Err(ModuleError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "Metadata file must be a YAML mapping: {}",
                meta_path.display()
            ),
        )),
    }
}

/// Convert raw dependency dicts from YAML to typed `DepInfo` objects.
///
/// Aligned with `apcore-python.parse_dependencies` and
/// `apcore-typescript.parseDependencies`.
pub fn parse_dependencies(deps_raw: &[serde_json::Value]) -> Vec<DepInfo> {
    let mut result = Vec::new();
    for dep in deps_raw {
        let module_id = dep.get("module_id").and_then(|v| v.as_str());
        match module_id {
            Some(id) if !id.is_empty() => {
                result.push(DepInfo {
                    module_id: id.to_string(),
                    version: dep
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    optional: dep
                        .get("optional")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                });
            }
            _ => {
                tracing::warn!("Dependency entry missing 'module_id', skipping: {:?}", dep);
            }
        }
    }
    result
}

/// Merge YAML metadata over code-level attributes per spec §4.13.
///
/// Scalar fields follow "YAML wins, code is the fallback".
///
/// Aligned with `apcore-python.merge_module_metadata` and
/// `apcore-typescript.mergeModuleMetadata`.
pub fn merge_module_metadata<S: std::hash::BuildHasher>(
    code: &HashMap<String, serde_json::Value, S>,
    yaml: &HashMap<String, serde_json::Value, S>,
) -> HashMap<String, serde_json::Value> {
    let mut merged = HashMap::new();

    // description: YAML wins
    let description = yaml
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| code.get("description").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    merged.insert(
        "description".to_string(),
        serde_json::Value::String(description),
    );

    // name: YAML wins
    let name = yaml
        .get("name")
        .cloned()
        .filter(|v| !v.is_null())
        .or_else(|| code.get("name").cloned());
    if let Some(n) = name {
        merged.insert("name".to_string(), n);
    }

    // tags: YAML wins when present
    let tags = yaml
        .get("tags")
        .filter(|v| !v.is_null())
        .or_else(|| code.get("tags"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    merged.insert("tags".to_string(), tags);

    // version: YAML wins
    let version = yaml
        .get("version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| code.get("version").and_then(|v| v.as_str()))
        .unwrap_or("1.0.0")
        .to_string();
    merged.insert("version".to_string(), serde_json::Value::String(version));

    // documentation: YAML wins
    let docs = yaml
        .get("documentation")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| code.get("documentation").and_then(|v| v.as_str()));
    if let Some(d) = docs {
        merged.insert(
            "documentation".to_string(),
            serde_json::Value::String(d.to_string()),
        );
    }

    // metadata: shallow merge (code base, YAML overlay)
    let mut meta_map: serde_json::Map<String, serde_json::Value> = code
        .get("metadata")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(yaml_meta) = yaml.get("metadata").and_then(|v| v.as_object()) {
        for (k, v) in yaml_meta {
            meta_map.insert(k.clone(), v.clone());
        }
    }
    merged.insert("metadata".to_string(), serde_json::Value::Object(meta_map));

    // annotations: FIELD-LEVEL merge, YAML > code > defaults
    // (PROTOCOL_SPEC §4.13). This used to take the YAML object WHOLE whenever
    // one was present, so a module whose code declared
    // `{destructive: true, readonly: false}` and whose YAML declared only
    // `{readonly: true}` lost `destructive` entirely. Both peers merge per
    // field (`merge_annotations` / `mergeAnnotations`); the keys neither side
    // sets fall through to the `ModuleAnnotations` serde defaults, which is the
    // "> defaults" leg of the rule.
    let yaml_annotations = yaml.get("annotations").filter(|v| !v.is_null());
    let code_annotations = code.get("annotations").filter(|v| !v.is_null());
    match (code_annotations, yaml_annotations) {
        (Some(base), Some(overlay)) => {
            let mut fields = base.as_object().cloned().unwrap_or_default();
            if let Some(overlay) = overlay.as_object() {
                for (key, value) in overlay {
                    fields.insert(key.clone(), value.clone());
                }
                merged.insert("annotations".to_string(), serde_json::Value::Object(fields));
            } else {
                // A non-object YAML annotations value cannot be merged per
                // field; keep the existing "YAML wins" behaviour for it.
                merged.insert("annotations".to_string(), overlay.clone());
            }
        }
        (None, Some(only)) | (Some(only), None) => {
            merged.insert("annotations".to_string(), only.clone());
        }
        (None, None) => {}
    }

    // dependencies: YAML wins when present (an explicit empty list overrides
    // code-declared dependencies), code is the fallback.
    //
    // The key was absent from the returned map altogether, so it survived
    // discovery — the registry reads it off the raw YAML before merging — and
    // was then lost before storage, leaving the post-registration metadata
    // accessor unable to see it. LOAD-order topological sorting therefore
    // worked while RELOAD-order sorting had nothing to sort by. Same defect and
    // same fix as apcore-python and apcore-typescript (aiperceivable/apcore-typescript#35).
    let dependencies = yaml
        .get("dependencies")
        .filter(|v| !v.is_null())
        .or_else(|| code.get("dependencies").filter(|v| !v.is_null()))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    merged.insert("dependencies".to_string(), dependencies);

    // examples: YAML wins fully when present
    let examples = yaml
        .get("examples")
        .filter(|v| !v.is_null())
        .or_else(|| code.get("examples"));
    if let Some(e) = examples {
        merged.insert("examples".to_string(), e.clone());
    }

    merged
}

/// Load an ID Map YAML file for canonical ID overrides.
///
/// The file must contain a top-level `mappings` list. Each entry has `file`
/// (the module file's path relative to the extension root) and `id`
/// (PROTOCOL_SPEC §2.2, D-138).
///
/// Aligned with `apcore-python.load_id_map` and
/// `apcore-typescript.loadIdMap`.
pub fn load_id_map(
    id_map_path: &Path,
) -> Result<HashMap<String, HashMap<String, serde_json::Value>>, ModuleError> {
    if !id_map_path.exists() {
        return Err(ModuleError::new(
            ErrorCode::ConfigNotFound,
            format!("ID map file not found: {}", id_map_path.display()),
        ));
    }

    let content = std::fs::read_to_string(id_map_path).map_err(|e| {
        ModuleError::new(
            ErrorCode::ConfigInvalid,
            format!("Cannot read ID map file {}: {}", id_map_path.display(), e),
        )
    })?;

    let parsed: serde_json::Value = serde_yaml_ng::from_str(&content).map_err(|e| {
        ModuleError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "Invalid YAML in ID map file {}: {}",
                id_map_path.display(),
                e
            ),
        )
    })?;

    let mappings = parsed
        .get("mappings")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            ModuleError::new(
                ErrorCode::ConfigInvalid,
                "ID map must contain a 'mappings' list",
            )
        })?;

    let mut result: HashMap<String, HashMap<String, serde_json::Value>> = HashMap::new();
    for entry in mappings {
        let file_path = match entry.get("file").and_then(|v| v.as_str()) {
            Some(f) if !f.is_empty() => f.to_string(),
            _ => {
                tracing::warn!("ID map entry missing 'file' field, skipping");
                continue;
            }
        };

        let mut info = HashMap::new();
        info.insert(
            "id".to_string(),
            entry
                .get("id")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::String(file_path.clone())),
        );
        // Keyed by the path relative to the extension root, spelled with `/`
        // and without a leading `./`, the form the discoverer matches against.
        let key = file_path.trim_start_matches("./").replace('\\', "/");
        result.insert(key, info);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dependencies_empty() {
        assert!(parse_dependencies(&[]).is_empty());
    }

    #[test]
    fn test_parse_dependencies_valid() {
        let raw = vec![
            serde_json::json!({"module_id": "foo.bar", "version": "1.0.0", "optional": false}),
            serde_json::json!({"module_id": "baz", "optional": true}),
        ];
        let deps = parse_dependencies(&raw);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].module_id, "foo.bar");
        assert_eq!(deps[0].version, Some("1.0.0".to_string()));
        assert!(!deps[0].optional);
        assert_eq!(deps[1].module_id, "baz");
        assert!(deps[1].optional);
    }

    #[test]
    fn test_parse_dependencies_missing_module_id() {
        let raw = vec![serde_json::json!({"version": "1.0.0"})];
        let deps = parse_dependencies(&raw);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_merge_module_metadata_annotations_are_field_level_merged() {
        // PROTOCOL_SPEC §4.13: YAML > code > defaults, per FIELD. Taking the
        // YAML object whole silently dropped every code-set flag the YAML did
        // not restate — here, `destructive`.
        let mut code = HashMap::new();
        code.insert(
            "annotations".to_string(),
            serde_json::json!({"destructive": true, "readonly": false}),
        );

        let mut yaml = HashMap::new();
        yaml.insert(
            "annotations".to_string(),
            serde_json::json!({"readonly": true}),
        );

        let merged = merge_module_metadata(&code, &yaml);
        let annotations = merged.get("annotations").expect("annotations key");
        assert_eq!(annotations["destructive"], serde_json::json!(true));
        assert_eq!(annotations["readonly"], serde_json::json!(true));
    }

    #[test]
    fn test_merge_module_metadata_annotations_from_one_side_only() {
        let mut code = HashMap::new();
        code.insert(
            "annotations".to_string(),
            serde_json::json!({"destructive": true}),
        );
        let yaml: HashMap<String, serde_json::Value> = HashMap::new();
        let merged = merge_module_metadata(&code, &yaml);
        assert_eq!(
            merged.get("annotations").expect("annotations key")["destructive"],
            serde_json::json!(true)
        );

        // Absent on both sides stays absent, so the descriptor keeps `None`.
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(!merge_module_metadata(&empty, &empty).contains_key("annotations"));
    }

    #[test]
    fn test_merge_module_metadata_emits_dependencies() {
        // The key was missing from the returned map entirely, so a declared
        // dependency was lost between discovery and storage.
        let mut code = HashMap::new();
        code.insert(
            "dependencies".to_string(),
            serde_json::json!([{"module_id": "common.db"}]),
        );
        let yaml: HashMap<String, serde_json::Value> = HashMap::new();
        let merged = merge_module_metadata(&code, &yaml);
        assert_eq!(
            merged.get("dependencies").expect("dependencies key"),
            &serde_json::json!([{"module_id": "common.db"}])
        );

        // YAML wins, and an explicit empty list is a real override.
        let mut yaml = HashMap::new();
        yaml.insert("dependencies".to_string(), serde_json::json!([]));
        let merged = merge_module_metadata(&code, &yaml);
        assert_eq!(
            merged.get("dependencies").expect("dependencies key"),
            &serde_json::json!([])
        );

        // Declared nowhere: the key is still present, as an empty list.
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        assert_eq!(
            merge_module_metadata(&empty, &empty)
                .get("dependencies")
                .expect("dependencies key"),
            &serde_json::json!([])
        );
    }

    #[test]
    fn test_merge_module_metadata() {
        let mut code = HashMap::new();
        code.insert(
            "description".to_string(),
            serde_json::json!("Code description"),
        );
        code.insert("version".to_string(), serde_json::json!("1.0.0"));

        let mut yaml = HashMap::new();
        yaml.insert(
            "description".to_string(),
            serde_json::json!("YAML description"),
        );

        let merged = merge_module_metadata(&code, &yaml);
        assert_eq!(
            merged.get("description").unwrap().as_str().unwrap(),
            "YAML description"
        );
        assert_eq!(merged.get("version").unwrap().as_str().unwrap(), "1.0.0");
    }
}
