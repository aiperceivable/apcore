// APCore Protocol — System manifest modules
// Spec reference: system.manifest.module, system.manifest.full

use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::context::Context;
use crate::errors::ModuleError;
use crate::module::Module;
use crate::registry::registry::Registry;

/// Canonical `source_path` for a module, or `None` when `project.source_root`
/// is unset.
///
/// SYS-7: a relative path was synthesized when no source root was configured,
/// which states as fact something the configuration does not say. Both peers
/// return null there (apcore-python `_compute_source_path` returns `None`), and
/// `sys-manifest-module.schema.json` types the field `["string", "null"]`.
fn compute_source_path(source_root: &str, module_id: &str) -> serde_json::Value {
    if source_root.is_empty() {
        return serde_json::Value::Null;
    }
    json!(format!(
        "{}/{}.rs",
        source_root,
        module_id.replace('.', "/")
    ))
}

/// The annotation projection both manifest modules publish.
///
/// The two governance flags come from the D-96 union of the live module
/// instance and the registry descriptor — the same source
/// `BuiltinApprovalGate` reads. `readonly` / `idempotent` describe behaviour
/// rather than governance and stay descriptor-sourced.
///
/// The manifest is what an agent reads to decide whether to call a module, so
/// advertising a governance value the gate does not enforce is worse than
/// advertising none. Reading the descriptor alone said `requires_approval:
/// false` for a module whose instance declares it — and this SDK accepts a
/// caller-supplied `ModuleDescriptor`, so the two genuinely disagree.
fn annotations_view(
    registry: &Registry,
    descriptor: &crate::registry::registry::ModuleDescriptor,
) -> serde_json::Value {
    let governance = crate::module::ModuleAnnotations::governance_union(
        registry
            .get(&descriptor.module_id)
            .ok()
            .flatten()
            .map(|m| m.annotations())
            .as_ref(),
        descriptor.annotations.as_ref(),
    );
    json!({
        "readonly": descriptor.annotations.as_ref().is_some_and(|a| a.readonly),
        "idempotent": descriptor.annotations.as_ref().is_some_and(|a| a.idempotent),
        "requires_approval": governance.as_ref().is_some_and(|a| a.requires_approval),
        "destructive": governance.as_ref().is_some_and(|a| a.destructive),
    })
}

/// Build one manifest entry from the registry descriptor.
///
/// This is the single definition of a manifest entry, shared by
/// `system.manifest.module` and `system.manifest.full` (SYS-8): apcore-python's
/// `manifest.full` entry is field-for-field its `manifest.module` output, which
/// is what `sys-manifest-full.schema.json` asserts by `$ref`-ing
/// `sys-manifest-module.schema.json`. Two independent builders is how the two
/// drifted — `documentation` and `metadata` reached one and not the other.
///
/// `include_schemas` / `include_source_paths` emit the key as `null` rather
/// than omitting it (SYS-9); the canonical schemas type those fields
/// `["object", "null"]` / `["string", "null"]`, and an absent key and a null
/// one are different answers to "does this module have a source path".
fn manifest_entry(
    registry: &Registry,
    descriptor: &crate::registry::registry::ModuleDescriptor,
    source_root: &str,
    include_schemas: bool,
    include_source_paths: bool,
) -> serde_json::Value {
    let module_id = descriptor.module_id.as_str();
    // Prefer the descriptor's description (canonical registered metadata,
    // including YAML overrides); fall back to the live instance only when the
    // descriptor lacks one. Matches apcore-python / apcore-typescript.
    let description = if descriptor.description.is_empty() {
        registry
            .get(module_id)
            .ok()
            .flatten()
            .map(|m| m.description().to_string())
            .unwrap_or_default()
    } else {
        descriptor.description.clone()
    };

    json!({
        "module_id": module_id,
        "description": description,
        // SYS-6: both come off the DESCRIPTOR, which carries them —
        // `register_versioned` populates `metadata`, and a `*.binding.yaml`
        // populates `documentation`. They were hardcoded to null / {} under a
        // comment claiming the `Module` trait does not expose them, which is
        // true and irrelevant: this reads the descriptor, not the trait.
        "documentation": descriptor
            .documentation
            .as_ref()
            .map_or(serde_json::Value::Null, |d| json!(d)),
        "metadata": json!(descriptor.metadata),
        "source_path": if include_source_paths {
            compute_source_path(source_root, module_id)
        } else {
            serde_json::Value::Null
        },
        "input_schema": if include_schemas {
            descriptor.input_schema.clone()
        } else {
            serde_json::Value::Null
        },
        "output_schema": if include_schemas {
            descriptor.output_schema.clone()
        } else {
            serde_json::Value::Null
        },
        "annotations": annotations_view(registry, descriptor),
        "tags": descriptor.tags,
        "dependencies": descriptor.dependencies,
    })
}

/// system.manifest.module — Full manifest for a single registered module.
pub struct ManifestModule {
    registry: Arc<Registry>,
    config: Arc<Mutex<Config>>,
}

impl ManifestModule {
    pub fn new(registry: Arc<Registry>, config: Arc<Mutex<Config>>) -> Self {
        Self { registry, config }
    }
}

#[async_trait]
impl Module for ManifestModule {
    fn description(&self) -> &'static str {
        "Full manifest for a single registered module"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["module_id"],
            "properties": {
                "module_id": {"type": "string"}
            }
        })
    }

    // PROTOCOL_SPEC §6.7.1.6 (SYS-24): `output_schema` MUST declare the full
    // field contract. A bare `{"type": "object"}` satisfies "equivalent output
    // schemas" only in the sense that any two such declarations are equivalent
    // to each other. Canonical shape: apcore/schemas/sys-manifest-module.schema.json.
    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["module_id", "description"],
            "properties": {
                "module_id": {"type": "string", "description": "Canonical module ID"},
                "description": {"type": "string", "description": "Module description (max 200 characters, plain text)"},
                "documentation": {"type": ["string", "null"], "description": "Extended documentation (max 5000 characters, Markdown allowed)"},
                "source_path": {"type": ["string", "null"], "description": "Source file path, or null when project.source_root is unset"},
                "input_schema": {"type": ["object", "null"], "description": "Module input JSON Schema"},
                "output_schema": {"type": ["object", "null"], "description": "Module output JSON Schema"},
                "annotations": {"type": ["object", "null"], "description": "Module annotation metadata"},
                "tags": {"type": "array", "items": {"type": "string"}, "description": "Module tags"},
                "dependencies": {"type": "array", "description": "Declared module dependencies"},
                "metadata": {"type": ["object", "null"], "description": "Additional module metadata"}
            }
        })
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        // Reject an empty module_id with InvalidInput (GENERAL_INVALID_INPUT)
        // rather than letting it fall through to ModuleNotFound, matching
        // apcore-python / apcore-typescript.
        let module_id = super::require_string(&inputs, "module_id")?;
        let module_id = module_id.as_str();

        let descriptor = self
            .registry
            .get_definition(module_id)?
            .ok_or_else(|| ModuleError::module_not_found(module_id))?;

        let source_root = {
            let cfg = self.config.lock().await;
            cfg.get("project.source_root")
                .and_then(|v| v.as_str().map(std::string::ToString::to_string))
                .unwrap_or_default()
        };

        Ok(manifest_entry(
            &self.registry,
            &descriptor,
            &source_root,
            true,
            true,
        ))
    }
}

/// system.manifest.full — Complete system manifest with filtering.
pub struct ManifestFullModule {
    registry: Arc<Registry>,
    config: Arc<Mutex<Config>>,
}

impl ManifestFullModule {
    pub fn new(registry: Arc<Registry>, config: Arc<Mutex<Config>>) -> Self {
        Self { registry, config }
    }
}

#[async_trait]
impl Module for ManifestFullModule {
    fn description(&self) -> &'static str {
        "Complete system manifest with filtering"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "include_schemas": {"type": "boolean", "default": true},
                "include_source_paths": {"type": "boolean", "default": true},
                "prefix": {"type": "string"},
                "tags": {"type": "array", "items": {"type": "string"}}
            }
        })
    }

    // PROTOCOL_SPEC §6.7.1.6 (SYS-24). Canonical shape:
    // apcore/schemas/sys-manifest-full.schema.json, whose `modules` items
    // `$ref` sys-manifest-module.schema.json — the entries are the same shape
    // `system.manifest.module` returns.
    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["project_name", "module_count", "modules"],
            "properties": {
                "project_name": {"type": "string", "description": "Project name from configuration"},
                "module_count": {"type": "integer", "description": "Number of modules returned"},
                "modules": {
                    "type": "array",
                    "description": "Module manifest entries, each the shape system.manifest.module returns",
                    "items": ManifestModule::new(
                        std::sync::Arc::clone(&self.registry),
                        std::sync::Arc::clone(&self.config),
                    )
                    .output_schema()
                }
            }
        })
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        let include_schemas = inputs
            .get("include_schemas")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let include_source_paths = inputs
            .get("include_source_paths")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let prefix = inputs.get("prefix").and_then(|v| v.as_str());
        let filter_tags: Option<Vec<&str>> = inputs
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

        let (project_name, source_root) = {
            let cfg = self.config.lock().await;
            let name = cfg
                .get("project.name")
                .and_then(|v| v.as_str().map(std::string::ToString::to_string))
                .unwrap_or_else(|| "apcore".to_string());
            let root = cfg
                .get("project.source_root")
                .and_then(|v| v.as_str().map(std::string::ToString::to_string))
                .unwrap_or_default();
            (name, root)
        };

        // SYS-11: filtering is delegated to `Registry::list`, which is where
        // both filters are defined. Re-implementing the tag match against
        // `descriptor.tags` here bypassed that method's union with the live
        // instance's `tags()` (D11-003), so a module declaring
        // `fn tags(&self)` and registered via `register_module` — which builds
        // an empty `descriptor.tags` — was invisible to a tag query.
        let module_ids = self.registry.list(filter_tags.as_deref(), prefix, None);

        let mut modules = Vec::new();
        for mid in &module_ids {
            let Some(descriptor) = self.registry.get_definition(mid).ok().flatten() else {
                continue;
            };
            modules.push(manifest_entry(
                &self.registry,
                &descriptor,
                &source_root,
                include_schemas,
                include_source_paths,
            ));
        }

        Ok(json!({
            "project_name": project_name,
            "module_count": modules.len(),
            "modules": modules,
        }))
    }
}
