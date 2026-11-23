// APCore Protocol — Schema module
// Spec reference: Schema loading, validation, export, and reference resolution

pub mod exporter;
pub mod hardening;
pub mod loader;
pub mod openai_strict;
pub mod ref_resolver;
pub mod strict;
pub mod validator;

pub use exporter::{ExportOptions, ExportProfile, SchemaExporter};
pub use hardening::{content_hash, format_warnings, FormatWarning};
pub use loader::{SchemaLoader, SchemaStrategy};
pub use openai_strict::{assert_openai_strict_compatible, detect_openai_strict_incompatibilities};
pub use ref_resolver::RefResolver;
pub use strict::to_strict_schema;
pub use validator::{DetailedValidationResult, SchemaValidator};

use serde::{Deserialize, Serialize};

/// A structured schema definition for a module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDefinition {
    /// The module this schema belongs to.
    pub module_id: String,
    /// Human-readable description of the schema.
    pub description: String,
    /// JSON Schema for the module's input.
    pub input_schema: serde_json::Value,
    /// JSON Schema for the module's output.
    pub output_schema: serde_json::Value,
    /// Optional JSON Schema for the module's error output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_schema: Option<serde_json::Value>,
    /// Optional reusable schema definitions (e.g. $defs / definitions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definitions: Option<serde_json::Value>,
    /// Optional schema version string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}
