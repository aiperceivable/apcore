// APCore Protocol — Schema exporter
// Spec reference: Exporting schemas in various formats

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::strict::{apply_llm_descriptions, strip_extension_keywords, to_strict_schema};
use super::SchemaDefinition;
use crate::errors::{ErrorCode, ModuleError};
use crate::module::{ModuleAnnotations, ModuleExample};

/// Optional parameters for the spec-compatible [`SchemaExporter::export`] method.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Optional annotations to merge into the exported schema (e.g. `x-*` fields).
    pub annotations: Option<serde_json::Value>,
    /// Optional examples array to include in the exported schema.
    pub examples: Option<serde_json::Value>,
    /// Override the schema name in the exported output.
    pub name: Option<String>,
}

/// Profile controlling how schemas are exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportProfile {
    Mcp,
    #[serde(rename = "openai")]
    OpenAi,
    Anthropic,
    Generic,
}

/// Exports schemas to various formats.
#[derive(Debug)]
pub struct SchemaExporter;

impl SchemaExporter {
    /// Create a new schema exporter.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Export a schema using the given profile.
    ///
    /// Returns a [`serde_json::Value`] (spec-compatible with Python/TypeScript `-> dict`).
    /// Optional `options` allows overriding the name and merging annotations/examples.
    pub fn export(
        &self,
        schema: &serde_json::Value,
        profile: ExportProfile,
        options: Option<&ExportOptions>,
    ) -> Result<serde_json::Value, ModuleError> {
        let mut exported = match profile {
            ExportProfile::Mcp => self.export_mcp(schema)?,
            ExportProfile::OpenAi => self.export_openai(schema)?,
            ExportProfile::Anthropic => self.export_anthropic(schema)?,
            ExportProfile::Generic => self.export_generic(schema)?,
        };

        // Apply optional overrides from ExportOptions.
        if let Some(opts) = options {
            if let (Some(obj), Some(name)) = (exported.as_object_mut(), &opts.name) {
                obj.insert("name".to_string(), serde_json::Value::String(name.clone()));
            }
            if let (Some(obj), Some(annotations)) = (exported.as_object_mut(), &opts.annotations) {
                if let Some(ann_obj) = annotations.as_object() {
                    for (k, v) in ann_obj {
                        obj.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
            if let (Some(obj), Some(examples)) = (exported.as_object_mut(), &opts.examples) {
                obj.insert("examples".to_string(), examples.clone());
            }
        }

        Ok(exported)
    }

    /// Export a schema to a pretty-printed JSON string (preserves legacy behaviour).
    ///
    /// Equivalent to calling [`Self::export`] and serializing the result.
    pub fn export_serialized(
        &self,
        schema: &serde_json::Value,
        profile: ExportProfile,
        options: Option<&ExportOptions>,
    ) -> Result<String, ModuleError> {
        let value = self.export(schema, profile, options)?;
        serde_json::to_string_pretty(&value).map_err(|e| {
            ModuleError::new(
                ErrorCode::SchemaParseError,
                format!("Failed to serialize exported schema: {e}"),
            )
        })
    }

    /// Sync SCHEMA-004: Spec-aligned export taking a [`SchemaDefinition`] plus
    /// optional `ModuleAnnotations` and `ModuleExample` collections, so MCP
    /// and Anthropic envelopes carry the full annotation + meta + examples
    /// payload (matching apcore-python `SchemaExporter.export` and
    /// apcore-typescript `SchemaExporter.export`).
    ///
    /// - `MCP` → emits `{name, description, inputSchema, annotations: {...},
    ///   _meta: {cacheable, cacheTtl, paginated, paginationStyle, ...}}`
    /// - `OpenAI` → emits the strict-mode envelope (Algorithm A23)
    /// - `Anthropic` → emits `{name, description, input_schema}` plus
    ///   `input_examples` if any examples are supplied
    /// - `Generic` → returns the full schema definition
    ///
    /// `name` overrides the envelope's `name` field; when `None`, falls back
    /// to `schema_def.module_id` (with `.` → `_` for OpenAI/Anthropic).
    pub fn export_def(
        &self,
        schema_def: &SchemaDefinition,
        profile: ExportProfile,
        annotations: Option<&ModuleAnnotations>,
        examples: Option<&[ModuleExample]>,
        name: Option<&str>,
    ) -> Result<serde_json::Value, ModuleError> {
        match profile {
            ExportProfile::Mcp => Ok(Self::build_mcp_envelope(schema_def, annotations, name)),
            ExportProfile::OpenAi => Ok(Self::build_openai_envelope(schema_def, name)),
            ExportProfile::Anthropic => {
                Ok(Self::build_anthropic_envelope(schema_def, examples, name))
            }
            ExportProfile::Generic => Ok(Self::build_generic_envelope(schema_def)),
        }
    }

    fn build_mcp_envelope(
        schema_def: &SchemaDefinition,
        annotations: Option<&ModuleAnnotations>,
        name: Option<&str>,
    ) -> serde_json::Value {
        let resolved_name = name.map_or_else(|| schema_def.module_id.clone(), str::to_string);
        let ann = annotations.cloned().unwrap_or_default();
        let mut envelope = serde_json::json!({
            "name": resolved_name,
            "description": schema_def.description,
            "inputSchema": schema_def.input_schema,
            "annotations": {
                "readOnlyHint": ann.readonly,
                "destructiveHint": ann.destructive,
                "idempotentHint": ann.idempotent,
                "openWorldHint": ann.open_world,
            },
            "_meta": {
                "cacheable": ann.cacheable,
                "cacheTtl": ann.cache_ttl,
                "cacheKeyFields": ann.cache_key_fields,
                "paginated": ann.paginated,
                "paginationStyle": ann.pagination_style,
            },
        });
        if ann.requires_approval {
            envelope["_meta"]["requiresApproval"] = serde_json::json!(true);
        }
        if ann.streaming {
            envelope["_meta"]["streaming"] = serde_json::json!(true);
        }
        envelope
    }

    fn build_openai_envelope(
        schema_def: &SchemaDefinition,
        name: Option<&str>,
    ) -> serde_json::Value {
        let resolved_name =
            name.map_or_else(|| schema_def.module_id.replace('.', "_"), str::to_string);
        // Sync finding A-D-030: substitute `description` with
        // `x-llm-description` at every node BEFORE strict-mode strips x-*
        // keys, so the LLM-tuned descriptions survive the export. Mirrors
        // apcore-python's `_apply_llm_descriptions` and apcore-typescript's
        // `applyLlmDescriptions`.
        let with_llm = apply_llm_descriptions(&schema_def.input_schema);
        let strict_parameters = to_strict_schema(&with_llm);
        serde_json::json!({
            "type": "function",
            "function": {
                "name": resolved_name,
                "description": schema_def.description,
                "parameters": strict_parameters,
                "strict": true,
            }
        })
    }

    fn build_anthropic_envelope(
        schema_def: &SchemaDefinition,
        examples: Option<&[ModuleExample]>,
        name: Option<&str>,
    ) -> serde_json::Value {
        let resolved_name =
            name.map_or_else(|| schema_def.module_id.replace('.', "_"), str::to_string);
        // A-D-030: substitute description with x-llm-description before
        // emitting the input_schema so Anthropic sees the LLM-tuned text.
        let input_schema_with_llm =
            strip_extension_keywords(&apply_llm_descriptions(&schema_def.input_schema));
        let mut envelope = serde_json::json!({
            "name": resolved_name,
            "description": schema_def.description,
            "input_schema": input_schema_with_llm,
        });
        if let Some(exs) = examples {
            if !exs.is_empty() {
                let inputs: Vec<serde_json::Value> = exs.iter().map(|e| e.inputs.clone()).collect();
                envelope["input_examples"] = serde_json::Value::Array(inputs);
            }
        }
        envelope
    }

    fn build_generic_envelope(schema_def: &SchemaDefinition) -> serde_json::Value {
        serde_json::json!({
            "module_id": schema_def.module_id,
            "description": schema_def.description,
            "input_schema": schema_def.input_schema,
            "output_schema": schema_def.output_schema,
            "definitions": schema_def.definitions.clone().unwrap_or(serde_json::Value::Null),
        })
    }

    /// Export all schemas from a loader using the given profile.
    ///
    /// Returns a map of schema name → [`serde_json::Value`].
    pub fn export_all(
        &self,
        loader: &super::loader::SchemaLoader,
        profile: ExportProfile,
    ) -> Result<HashMap<String, serde_json::Value>, ModuleError> {
        let mut result = HashMap::new();
        for name in loader.list() {
            if let Some(schema) = loader.get(name) {
                let exported = self.export(schema, profile, None)?;
                result.insert(name.to_string(), exported);
            }
        }
        Ok(result)
    }

    /// Export all schemas from a loader to serialized JSON strings (legacy helper).
    pub fn export_all_serialized(
        &self,
        loader: &super::loader::SchemaLoader,
        profile: ExportProfile,
    ) -> Result<HashMap<String, String>, ModuleError> {
        let mut result = HashMap::new();
        for name in loader.list() {
            if let Some(schema) = loader.get(name) {
                let exported = self.export_serialized(schema, profile, None)?;
                result.insert(name.to_string(), exported);
            }
        }
        Ok(result)
    }

    /// MCP format: { name, description, inputSchema, annotations, _meta }
    ///
    /// Normalize native annotations and MCP hints through the same envelope
    /// builder as typed exports. Approval and streaming are true-only metadata.
    #[allow(clippy::unused_self)] // consistent method signature for dispatch through export()
    fn export_mcp(&self, schema: &serde_json::Value) -> Result<serde_json::Value, ModuleError> {
        let name = schema
            .get("name")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let description = schema
            .get("description")
            .cloned()
            .unwrap_or(serde_json::Value::String(String::new()));
        let input_schema = schema
            .get("input_schema")
            .or_else(|| schema.get("inputSchema"))
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let mut annotations = schema
            .get("annotations")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        for (hint, native) in [
            ("readOnlyHint", "readonly"),
            ("destructiveHint", "destructive"),
            ("idempotentHint", "idempotent"),
            ("openWorldHint", "open_world"),
            ("requiresApproval", "requires_approval"),
        ] {
            if annotations.get(native).is_none() {
                if let Some(value) = annotations.get(hint).cloned() {
                    annotations[native] = value;
                }
            }
        }
        for (meta, native) in [
            ("requiresApproval", "requires_approval"),
            ("streaming", "streaming"),
        ] {
            if annotations.get(native).is_none() {
                if let Some(value) = schema
                    .get("_meta")
                    .and_then(|value| value.get(meta))
                    .cloned()
                {
                    annotations[native] = value;
                }
            }
        }
        let annotations: ModuleAnnotations =
            serde_json::from_value(annotations).map_err(|error| {
                ModuleError::new(
                    ErrorCode::SchemaParseError,
                    format!("Invalid export annotations: {error}"),
                )
            })?;
        let definition = SchemaDefinition {
            module_id: String::new(),
            description: String::new(),
            input_schema,
            output_schema: serde_json::json!({}),
            error_schema: None,
            definitions: None,
            version: None,
        };
        let mut envelope = Self::build_mcp_envelope(&definition, Some(&annotations), None);
        envelope["name"] = name;
        envelope["description"] = description;
        if let Some(meta) = schema.get("_meta").and_then(serde_json::Value::as_object) {
            for (key, value) in meta {
                if key != "requiresApproval" && key != "streaming" {
                    envelope["_meta"][key] = value.clone();
                }
            }
        }
        Ok(envelope)
    }

    /// `OpenAI` format: { type: "function", function: { name, description, parameters, strict } }
    ///
    /// Sync SCHEMA-003: parameters MUST be transformed via Algorithm A23
    /// (`to_strict_schema`) so OpenAI strict-mode requirements hold —
    /// `additionalProperties: false` on every object schema and all properties
    /// listed in `required`. Mirrors apcore-python `exporter.py:70` and
    /// apcore-typescript `exporter.ts:59`.
    #[allow(clippy::unused_self)] // consistent method signature for dispatch through export()
    #[allow(clippy::unnecessary_wraps)] // consistent Result return for dispatch through export()
    fn export_openai(&self, schema: &serde_json::Value) -> Result<serde_json::Value, ModuleError> {
        let name = schema
            .get("name")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let description = schema
            .get("description")
            .cloned()
            .unwrap_or(serde_json::Value::String(String::new()));
        let parameters = schema
            .get("input_schema")
            .or_else(|| schema.get("inputSchema"))
            .or_else(|| schema.get("parameters"))
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Apply Algorithm A23 strict transform so the envelope satisfies
        // OpenAI's strict-mode contract.
        let strict_parameters = to_strict_schema(&apply_llm_descriptions(&parameters));

        Ok(serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": strict_parameters,
                "strict": true,
            }
        }))
    }

    /// Anthropic format: { name, description, `input_schema` }
    #[allow(clippy::unused_self)] // consistent method signature for dispatch through export()
    #[allow(clippy::unnecessary_wraps)] // consistent Result return for dispatch through export()
    fn export_anthropic(
        &self,
        schema: &serde_json::Value,
    ) -> Result<serde_json::Value, ModuleError> {
        let name = schema
            .get("name")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let description = schema
            .get("description")
            .cloned()
            .unwrap_or(serde_json::Value::String(String::new()));
        let input_schema = schema
            .get("input_schema")
            .or_else(|| schema.get("inputSchema"))
            .cloned()
            .unwrap_or(serde_json::json!({}));

        Ok(serde_json::json!({
            "name": name,
            "description": description,
            "input_schema": strip_extension_keywords(&apply_llm_descriptions(&input_schema)),
        }))
    }

    /// Generic format: return schema as-is.
    #[allow(clippy::unused_self)] // consistent method signature for dispatch through export()
    #[allow(clippy::unnecessary_wraps)] // consistent Result return for dispatch through export()
    fn export_generic(&self, schema: &serde_json::Value) -> Result<serde_json::Value, ModuleError> {
        Ok(schema.clone())
    }
}

impl Default for SchemaExporter {
    fn default() -> Self {
        Self::new()
    }
}
