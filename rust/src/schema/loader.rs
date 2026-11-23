// APCore Protocol — Schema loader
// Spec reference: Loading schemas from files and inline definitions

use serde::{Deserialize, Serialize};
use serde_yaml_ng as serde_yaml;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::errors::{ErrorCode, ModuleError};
use crate::schema::ref_resolver::{RefResolver, DEFAULT_MAX_REF_DEPTH};
use crate::schema::SchemaDefinition;

/// Strategy for loading schemas when both YAML and native definitions exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaStrategy {
    /// Prefer YAML schema files over native code definitions.
    YamlFirst,
    /// Prefer native code definitions over YAML files.
    NativeFirst,
    /// Use only YAML files; ignore native definitions.
    YamlOnly,
}

/// Loads JSON schemas from various sources.
#[derive(Debug)]
pub struct SchemaLoader {
    schemas: HashMap<String, serde_json::Value>,
    pub strategy: SchemaStrategy,
    /// Optional base directory used by the spec-compatible `load()` method.
    schemas_dir: Option<PathBuf>,
    /// `schema.max_ref_depth` — the `$ref` recursion cap handed to the
    /// [`RefResolver`] built by [`Self::resolve`].
    max_ref_depth: usize,
}

impl SchemaLoader {
    /// Create a new schema loader with the default strategy.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            strategy: SchemaStrategy::YamlFirst,
            schemas_dir: None,
            max_ref_depth: DEFAULT_MAX_REF_DEPTH,
        }
    }

    /// Create a schema loader with a specific strategy.
    #[must_use]
    pub fn with_strategy(strategy: SchemaStrategy) -> Self {
        Self {
            strategy,
            ..Self::new()
        }
    }

    /// Spec-compatible constructor: create a loader from a `Config` and optional schemas directory.
    ///
    /// `schemas_dir` overrides the directory used by [`Self::load`] to resolve schema files.
    /// When `schemas_dir` is `None` the loader falls back to `config.schema.root`, then
    /// `config.modules_path`, and finally to the current working directory.
    pub fn with_config(config: &Config, schemas_dir: Option<&Path>) -> Self {
        let resolved_dir = schemas_dir
            .map(std::path::Path::to_path_buf)
            .or_else(|| {
                config
                    .get("schema.root")
                    .and_then(|v| v.as_str().map(PathBuf::from))
            })
            .or_else(|| config.modules_path.clone());
        let max_ref_depth = config
            .get("schema.max_ref_depth")
            .and_then(|v| v.as_u64())
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(DEFAULT_MAX_REF_DEPTH);
        // A-C-003: apcore-python (loader.py:1037) and apcore-typescript
        // (loader.ts:138) both read `schema.strategy` here; this loader used
        // to hardcode `YamlFirst` regardless of the config value. An absent
        // or unparseable value falls back to `YamlFirst`, matching the
        // declared default at config.rs:452 and TypeScript's fallback
        // (Python's constructor raises on an unparseable value instead —
        // a pre-existing Python/TypeScript divergence this fix does not
        // change).
        let strategy = config
            .get("schema.strategy")
            .and_then(|v| v.as_str().map(str::to_string))
            .and_then(|s| match s.as_str() {
                "yaml_first" => Some(SchemaStrategy::YamlFirst),
                "native_first" => Some(SchemaStrategy::NativeFirst),
                "yaml_only" => Some(SchemaStrategy::YamlOnly),
                _ => None,
            })
            .unwrap_or(SchemaStrategy::YamlFirst);
        Self {
            schemas: HashMap::new(),
            strategy,
            schemas_dir: resolved_dir,
            max_ref_depth,
        }
    }

    /// The `$ref` recursion cap this loader hands to [`RefResolver`].
    #[must_use]
    pub fn max_ref_depth(&self) -> usize {
        self.max_ref_depth
    }

    /// Resolve every `$ref` in `schema`, anchored at this loader's schemas
    /// directory and (optionally) the file `schema` was read from.
    ///
    /// Until this existed, nothing on the Rust load path ever invoked
    /// [`RefResolver`]: `schema.max_ref_depth` was silently inert and
    /// `SCHEMA_CIRCULAR_REF` / `SCHEMA_MAX_DEPTH_EXCEEDED` could not be raised
    /// at load time. apcore-python (`loader.py`) and apcore-typescript
    /// (`loader.ts`) both wire the resolver into their loaders.
    ///
    /// # Errors
    ///
    /// `SCHEMA_CIRCULAR_REF` for a `$ref` → `$ref` chain,
    /// `SCHEMA_MAX_DEPTH_EXCEEDED` when the chain exceeds `max_ref_depth`, and
    /// `SCHEMA_NOT_FOUND` for an unresolvable reference.
    pub fn resolve(
        &self,
        schema: &serde_json::Value,
        current_file: Option<&Path>,
    ) -> Result<serde_json::Value, ModuleError> {
        match self.build_resolver(current_file).resolve(schema) {
            Ok(resolved) => Ok(resolved),
            // D-104: the file root is the primary base for a local `#/…`
            // pointer (A05 step 4a) and stays so. When it does not resolve
            // there, the schema NODE being resolved is the fallback — a `$defs`
            // block nested inside `input_schema`, which apcore-python and
            // apcore-typescript accept and which used to raise
            // SCHEMA_NOT_FOUND here. Each SDK rejected what the other accepted,
            // so no schema file carrying a local `$ref` loaded in all three.
            //
            // The retry is per node rather than per document so a pointer
            // inside `output_schema` can never resolve into `input_schema`'s
            // `$defs`: each node is resolved with itself as its own fallback,
            // which is exactly what the two peers do.
            Err(err) if err.code == ErrorCode::SchemaNotFound => {
                let Some(obj) = schema.as_object() else {
                    return Err(err);
                };
                if !obj.contains_key("input_schema") && !obj.contains_key("output_schema") {
                    return Err(err);
                }
                let mut out = serde_json::Map::with_capacity(obj.len());
                for (key, node) in obj {
                    let resolver = match key.as_str() {
                        "input_schema" | "output_schema" | "error_schema" => self
                            .build_resolver(current_file)
                            .with_node_fallback(node.clone()),
                        _ => self.build_resolver(current_file),
                    };
                    out.insert(key.clone(), resolver.resolve_in_document(node, schema)?);
                }
                Ok(serde_json::Value::Object(out))
            }
            Err(err) => Err(err),
        }
    }

    /// Build a [`RefResolver`] carrying this loader's depth cap, schemas root,
    /// current file and in-memory registrations.
    fn build_resolver(&self, current_file: Option<&Path>) -> RefResolver {
        let mut resolver = RefResolver::with_max_depth(self.max_ref_depth);
        if let Some(dir) = self.schemas_dir.as_deref() {
            resolver = resolver.with_schemas_dir(dir);
        }
        if let Some(file) = current_file {
            resolver = resolver.with_current_file(file);
        }
        // In-memory schemas registered on this loader are referenceable by
        // their exact key, matching the peers' registry-first lookup.
        for (name, value) in &self.schemas {
            resolver.register(name, value.clone());
        }
        resolver
    }

    /// Spec-compatible load: resolve a schema for `module_id` and return a [`SchemaDefinition`].
    ///
    /// Resolution order:
    /// 1. If the schema was previously loaded in-memory via [`Self::load_from_value`] or
    ///    [`Self::load_from_file`], return it wrapped in a `SchemaDefinition`.
    /// 2. Otherwise, attempt to load
    ///    `<schemas_dir>/<module_id_path>.schema.yaml` (then `.schema.yml`,
    ///    then `.schema.json`), where `module_id_path` is `module_id.replace('.', '/')`.
    ///    Aligned with `apcore-python.SchemaLoader.load` (loader.py:62) and
    ///    `apcore-typescript.SchemaLoader.load` (loader.ts:53). Sync SCHEMA-002.
    ///
    /// `schemas_dir` is the directory supplied to [`Self::with_config`], or the current working
    /// directory when none was provided.
    pub fn load(&mut self, module_id: &str) -> Result<SchemaDefinition, ModuleError> {
        // 1. Already in memory?
        if let Some(value) = self.get(module_id) {
            return Self::value_to_schema_def(module_id, value.clone());
        }

        // 2. Try to find a file on disk.
        let base = self
            .schemas_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("."));

        // Convert the dot-separated module ID to a directory path and append
        // the canonical `.schema.yaml` (or `.schema.yml` / `.schema.json`)
        // suffix per spec — matches Python and TypeScript loaders.
        let module_path = module_id.replace('.', "/");
        let candidates = [
            base.join(format!("{module_path}.schema.yaml")),
            base.join(format!("{module_path}.schema.yml")),
            base.join(format!("{module_path}.schema.json")),
        ];

        let mut last_err: Option<ModuleError> = None;
        for path in &candidates {
            if path.exists() {
                self.load_from_file(module_id, path)?;
                // INVARIANT: load_from_file inserts into self.schemas on Ok; the key is present.
                let value = self.get(module_id).expect("just loaded").clone();
                // Resolve `$ref`s against this file's directory before shaping
                // the definition, so circular / too-deep / unresolvable
                // references fail at load time as they do in Python and TS.
                let value = self.resolve(&value, Some(path))?;
                return Self::value_to_schema_def(module_id, value);
            }
            last_err = Some(ModuleError::new(
                ErrorCode::SchemaNotFound,
                format!(
                    "Schema file not found for module '{}' (tried {})",
                    module_id,
                    path.display()
                ),
            ));
        }

        Err(last_err.unwrap_or_else(|| {
            ModuleError::new(
                ErrorCode::SchemaNotFound,
                format!("Schema not found for module '{module_id}'"),
            )
        }))
    }

    /// Resolve `module_id`'s schema using `self.strategy` (`schema.strategy`,
    /// set via [`Self::with_config`] or [`Self::with_strategy`]).
    ///
    /// `native_input_schema` / `native_output_schema` are the schemas
    /// generated from the module's Rust types (e.g. via `schemars`), when
    /// available. Under [`SchemaStrategy::YamlFirst`] (the default) a YAML
    /// file is tried first and the native schemas are the fallback when no
    /// file is found; [`SchemaStrategy::NativeFirst`] reverses that;
    /// [`SchemaStrategy::YamlOnly`] never falls back to the native schemas.
    ///
    /// Mirrors apcore-python's `SchemaLoader.get_schema`
    /// (`schema/loader.py:1027`) and apcore-typescript's
    /// `SchemaLoader.getSchema` (`schema/loader.ts:133`) — schema-system.md
    /// "Contract: SchemaLoader.get_schema" (A-C-003). Returns a single
    /// [`SchemaDefinition`] rather than the peers' `(ResolvedSchema,
    /// ResolvedSchema)` pair: this loader has no analogous
    /// content-addressable cache to key a second type off of, and neither
    /// peer's cache is reached by any caller outside their own tests.
    ///
    /// # Errors
    ///
    /// Propagates [`Self::load`]'s errors when no native fallback applies
    /// for the active strategy.
    pub fn get_schema(
        &mut self,
        module_id: &str,
        native_input_schema: Option<&serde_json::Value>,
        native_output_schema: Option<&serde_json::Value>,
    ) -> Result<SchemaDefinition, ModuleError> {
        match self.strategy {
            SchemaStrategy::YamlFirst => match self.load(module_id) {
                Ok(sd) => Ok(sd),
                Err(err) if err.code == ErrorCode::SchemaNotFound => {
                    match (native_input_schema, native_output_schema) {
                        (Some(input), Some(output)) => {
                            Ok(Self::wrap_native(module_id, input.clone(), output.clone()))
                        }
                        _ => Err(err),
                    }
                }
                Err(err) => Err(err),
            },
            SchemaStrategy::NativeFirst => match (native_input_schema, native_output_schema) {
                (Some(input), Some(output)) => {
                    Ok(Self::wrap_native(module_id, input.clone(), output.clone()))
                }
                _ => self.load(module_id),
            },
            SchemaStrategy::YamlOnly => self.load(module_id),
        }
    }

    /// Build a [`SchemaDefinition`] directly from already-resolved native
    /// schemas, without touching disk or the `$ref` resolver.
    fn wrap_native(
        module_id: &str,
        input_schema: serde_json::Value,
        output_schema: serde_json::Value,
    ) -> SchemaDefinition {
        SchemaDefinition {
            module_id: module_id.to_string(),
            description: String::new(),
            input_schema,
            output_schema,
            error_schema: None,
            definitions: None,
            version: None,
        }
    }

    /// Convert a raw JSON `Value` into a [`SchemaDefinition`] for `module_id`.
    fn value_to_schema_def(
        module_id: &str,
        value: serde_json::Value,
    ) -> Result<SchemaDefinition, ModuleError> {
        // If the stored value already has the SchemaDefinition shape, deserialize it.
        if value.get("input_schema").is_some() && value.get("output_schema").is_some() {
            return serde_json::from_value::<SchemaDefinition>(value).map_err(|e| {
                ModuleError::new(
                    ErrorCode::SchemaParseError,
                    format!("Failed to deserialize SchemaDefinition for '{module_id}': {e}"),
                )
            });
        }

        // Otherwise, treat the value as the input_schema itself and build a minimal definition.
        Ok(SchemaDefinition {
            module_id: module_id.to_string(),
            description: value
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            input_schema: value
                .get("input_schema")
                .or_else(|| value.get("inputSchema"))
                .cloned()
                .unwrap_or_else(|| value.clone()),
            output_schema: value
                .get("output_schema")
                .or_else(|| value.get("outputSchema"))
                .cloned()
                .unwrap_or(serde_json::json!({})),
            error_schema: None,
            definitions: None,
            version: None,
        })
    }

    /// Load a schema from a JSON/YAML file.
    pub fn load_from_file(&mut self, name: &str, path: &Path) -> Result<(), ModuleError> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            ModuleError::new(
                ErrorCode::SchemaNotFound,
                format!("Failed to read schema file '{}': {}", path.display(), e),
            )
        })?;

        // Determine format from extension; default to YAML (which also handles JSON).
        let value: serde_json::Value = if path.extension().is_some_and(|ext| ext == "json") {
            serde_json::from_str(&contents).map_err(|e| {
                ModuleError::new(
                    ErrorCode::SchemaParseError,
                    format!("Failed to parse JSON schema '{}': {}", path.display(), e),
                )
            })?
        } else {
            // YAML parser handles both .yaml/.yml (and is a superset of JSON)
            serde_yaml::from_str(&contents).map_err(|e| {
                ModuleError::new(
                    ErrorCode::SchemaParseError,
                    format!("Failed to parse YAML schema '{}': {}", path.display(), e),
                )
            })?
        };

        self.schemas.insert(name.to_string(), value);
        Ok(())
    }

    /// Load a schema from a JSON value.
    pub fn load_from_value(
        &mut self,
        name: &str,
        schema: serde_json::Value,
    ) -> Result<(), ModuleError> {
        self.schemas.insert(name.to_string(), schema);
        Ok(())
    }

    /// Get a loaded schema by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.schemas.get(name)
    }

    /// List all loaded schema names.
    pub fn list(&self) -> Vec<&str> {
        self.schemas
            .keys()
            .map(std::string::String::as_str)
            .collect()
    }
}

impl Default for SchemaLoader {
    fn default() -> Self {
        Self::new()
    }
}
