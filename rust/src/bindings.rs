// APCore Protocol — Binding loader
// Spec reference: protocol-spec §5.12 (Bindings YAML)
//
// Cross-language note: Rust cannot dynamically import compiled modules at
// runtime, so the canonical `target: "module:callable"` string is used as
// an opaque key into a user-supplied handler map. The YAML syntax itself
// is byte-identical across Python, TypeScript, and Rust SDKs.

use serde::{Deserialize, Serialize};
#[cfg(feature = "config")]
use serde_yaml_ng as serde_yaml;
use std::collections::HashMap;
#[cfg(feature = "config")]
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::context::Context;
#[cfg(feature = "config")]
use crate::decorator::FunctionModule;
use crate::errors::{ErrorCode, ModuleError};
#[cfg(feature = "config")]
use crate::module::ModuleAnnotations;
#[cfg(feature = "config")]
use crate::registry::registry::Registry;
#[cfg(feature = "config")]
use crate::schema::openai_strict::assert_openai_strict_compatible;
#[cfg(feature = "config")]
use crate::utils::helpers::match_glob;

#[cfg(feature = "config")]
const CURRENT_SPEC_VERSION: &str = "1.0";

/// Config key naming the directory scanned for binding files (§9.1.1).
#[cfg(feature = "config")]
const CONFIG_KEY_BINDINGS_DIR: &str = "bindings.dir";

/// Config key naming the glob binding files must match within that directory.
#[cfg(feature = "config")]
const CONFIG_KEY_BINDINGS_PATTERN: &str = "bindings.pattern";

/// Canonical default for `bindings.dir`, from `schemas/defaults.schema.json`
/// (spec v1.36.0) and `$defs/BindingsConfig` in
/// `schemas/apcore-config.schema.json`.
///
/// KEPT after v1.36.0 gave the key a canonical default, rather than deleted.
/// Since that release `Config::get("bindings.dir")` resolves `./bindings`
/// through `CONFIG_DEFAULTS`, so every call that supplies a [`Config`] reaches
/// the default through the one precedence chain §5.12.6 clause 2 requires and
/// never touches this constant. What it still serves is the call that supplies
/// **no** `Config` at all — `load_binding_dir_with_config(None, None, None)`,
/// a supported public invocation with nothing for `Config::get` to answer
/// from. The alternative, synthesising a `Config::from_defaults()` inside the
/// loader, would make the no-config path apply `APCORE_*` overrides that its
/// caller never asked for.
///
/// The duplication that a second copy of a canonical value always risks is
/// pinned rather than trusted: `the_loader_default_matches_the_canonical_
/// default_table` asserts these two constants equal `Config::default_for` for
/// their keys, so the pair cannot drift.
#[cfg(feature = "config")]
const DEFAULT_BINDING_DIR: &str = "./bindings";

/// Canonical default for `bindings.pattern`, from the same schemas, kept on the
/// same terms as [`DEFAULT_BINDING_DIR`].
#[cfg(feature = "config")]
const DEFAULT_BINDING_PATTERN: &str = "*.binding.yaml";

#[cfg(feature = "config")]
const SUPPORTED_SPEC_VERSIONS: &[&str] = &["1.0"];

/// Boxed async handler function type.
///
/// The handler takes the module inputs plus a reference to the execution
/// context and returns a JSON result. Handlers are stored as `Arc` so they
/// can be cheaply cloned when materializing multiple modules.
pub type BindingHandlerFn = Arc<
    dyn for<'a> Fn(
            serde_json::Value,
            &'a Context<serde_json::Value>,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, ModuleError>>
                    + Send
                    + 'a,
            >,
        > + Send
        + Sync,
>;

/// Backward-compatible alias.
pub type BindingHandler = BindingHandlerFn;

/// A binding handler bundled with optional auto-derived schemas.
///
/// When `auto_schema: true` is specified in a binding entry, the loader
/// reads schemas from this struct and rejects missing inference sources.
/// Use [`typed_handler`] to create instances with
/// auto-generated schemas from `schemars::JsonSchema` types.
pub struct TypedBindingHandler {
    pub handler: BindingHandlerFn,
    pub input_schema: Option<serde_json::Value>,
    pub output_schema: Option<serde_json::Value>,
}

/// Create a [`TypedBindingHandler`] with auto-derived JSON Schemas.
///
/// The input and output types must implement both `schemars::JsonSchema`
/// (for auto-schema derivation) and the standard serde traits (for
/// runtime de/serialization).
///
/// # Example
///
/// ```ignore
/// use apcore::bindings::typed_handler;
/// use schemars::JsonSchema;
///
/// #[derive(serde::Deserialize, JsonSchema)]
/// struct Input { name: String }
///
/// #[derive(serde::Serialize, JsonSchema)]
/// struct Output { greeting: String }
///
/// let handler = typed_handler::<Input, Output>(|input| {
///     Ok(Output { greeting: format!("Hello, {}!", input.name) })
/// });
/// ```
pub fn typed_handler<I, O>(
    f: impl Fn(I) -> Result<O, ModuleError> + Send + Sync + 'static,
) -> TypedBindingHandler
where
    I: schemars::JsonSchema + serde::de::DeserializeOwned + Send + 'static,
    O: schemars::JsonSchema + serde::Serialize + Send + 'static,
{
    let (handler, input_schema, output_schema) =
        typed_async_handler(move |input, _ctx| Box::pin(std::future::ready(f(input))));
    TypedBindingHandler {
        handler,
        input_schema: Some(input_schema),
        output_schema: Some(output_schema),
    }
}

pub(crate) fn typed_async_handler<I, O>(
    f: impl for<'a> Fn(
            I,
            &'a Context<serde_json::Value>,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<O, ModuleError>> + Send + 'a>>
        + Send
        + Sync
        + 'static,
) -> (BindingHandlerFn, serde_json::Value, serde_json::Value)
where
    I: schemars::JsonSchema + serde::de::DeserializeOwned + Send + 'static,
    O: schemars::JsonSchema + serde::Serialize + Send + 'static,
{
    let f = Arc::new(f);
    let handler: BindingHandlerFn = Arc::new(move |input: serde_json::Value, ctx| {
        let f = Arc::clone(&f);
        Box::pin(async move {
            let typed: I = serde_json::from_value(input).map_err(|e| {
                ModuleError::new(
                    ErrorCode::GeneralInvalidInput,
                    format!("Failed to deserialize input: {e}"),
                )
            })?;
            let result = f(typed, ctx).await?;
            serde_json::to_value(result).map_err(|e| {
                ModuleError::new(
                    ErrorCode::GeneralInternalError,
                    format!("Failed to serialize output: {e}"),
                )
            })
        })
    });

    (
        handler,
        serde_json::to_value(schemars::schema_for!(I)).unwrap_or_default(),
        serde_json::to_value(schemars::schema_for!(O)).unwrap_or_default(),
    )
}

/// `auto_schema` field accepts either a boolean or a mode string.
///
/// `true` is a synonym for `"permissive"`. See protocol-spec §5.12.5.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AutoSchemaValue {
    Bool(bool),
    Mode(String),
}

impl AutoSchemaValue {
    /// Normalize to canonical mode string: `"permissive"` or `"strict"`.
    /// Returns `None` when explicitly disabled (`false`).
    pub fn normalize(&self) -> Result<Option<&str>, String> {
        match self {
            Self::Bool(true) => Ok(Some("permissive")),
            Self::Bool(false) => Ok(None),
            Self::Mode(s) => match s.as_str() {
                "true" | "permissive" => Ok(Some("permissive")),
                "strict" => Ok(Some("strict")),
                other => Err(format!(
                    "auto_schema must be a boolean or one of [\"true\", \"permissive\", \"strict\"]; got {other:?}"
                )),
            },
        }
    }
}

/// The `validation.binding.*` limits of PROTOCOL_SPEC §9.1.2, resolved once.
///
/// **Every limit is unconstrained by default.** apcore does not impose limits
/// on the content its users author; it offers them, and an operator opts in.
/// [`ValidationLimits::default`] checks nothing, which is what a loader built
/// without a configuration uses — so a binding file that loaded before this
/// type existed still loads after it.
#[derive(Debug, Default, Clone)]
#[cfg(feature = "config")]
pub struct ValidationLimits {
    description_max: Option<usize>,
    documentation_max: Option<usize>,
    tags_pattern: Option<regex::Regex>,
    require_semver: bool,
}

#[cfg(feature = "config")]
impl ValidationLimits {
    /// Read the four keys from a [`Config`]. Unset keys stay unconstrained.
    ///
    /// A `tags_pattern` that does not compile is an error rather than a silent
    /// skip (§9.2.3 requirement 6d): a constraint that quietly checks nothing
    /// is the failure mode §10.6.1 exists to forbid.
    pub fn from_config(config: &crate::config::Config) -> Result<Self, ModuleError> {
        let usize_key = |key: &str| -> Option<usize> {
            config
                .get(key)
                .and_then(|v| v.as_u64())
                .and_then(|n| usize::try_from(n).ok())
        };
        let tags_pattern = match config
            .get("validation.binding.tags_pattern")
            .and_then(|v| v.as_str().map(std::string::ToString::to_string))
            .filter(|s: &String| !s.is_empty())
        {
            None => None,
            Some(src) => Some(regex::Regex::new(&src).map_err(|e| {
                ModuleError::new(
                    ErrorCode::BindingFileInvalid,
                    format!(
                        "validation.binding.tags_pattern '{src}' does not compile ({e}), \
                         so no tag could be checked against it"
                    ),
                )
            })?),
        };
        Ok(Self {
            description_max: usize_key("validation.binding.description_max_length"),
            documentation_max: usize_key("validation.binding.documentation_max_length"),
            tags_pattern,
            require_semver: config
                .get("validation.binding.version_require_semver")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    /// Apply the configured limits to one entry. Unset limits check nothing.
    #[cfg(feature = "config")]
    fn check(&self, entry: &BindingEntry, path: &Path) -> Result<(), ModuleError> {
        let invalid = |reason: String| {
            ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!("{}: {reason}", path.display()),
            )
        };
        for (field, value, max) in [
            (
                "description",
                entry.description.as_deref(),
                self.description_max,
            ),
            (
                "documentation",
                entry.documentation.as_deref(),
                self.documentation_max,
            ),
        ] {
            if let (Some(value), Some(max)) = (value, max) {
                // §9.1.2 requirement 4: characters, not bytes.
                let len = value.chars().count();
                if len > max {
                    return Err(invalid(format!(
                        "binding '{}' field '{field}' is {len} characters, over the {max} \
                         configured by validation.binding.{field}_max_length",
                        entry.module_id
                    )));
                }
            }
        }
        if let Some(pattern) = &self.tags_pattern {
            for tag in &entry.tags {
                if !pattern.is_match(tag) {
                    return Err(invalid(format!(
                        "binding '{}' tag '{tag}' does not match \
                         validation.binding.tags_pattern '{}'",
                        entry.module_id,
                        pattern.as_str()
                    )));
                }
            }
        }
        if self.require_semver && !semver_re().is_match(&entry.version) {
            return Err(invalid(format!(
                "binding '{}' version '{}' is not SemVer, required by \
                 validation.binding.version_require_semver",
                entry.module_id, entry.version
            )));
        }
        Ok(())
    }
}

/// PROTOCOL_SPEC §9.1.2 requirement 6 — the semver.org grammar, unmodified and
/// written into the specification as a literal so three implementations cannot
/// invent three. `1.0` does NOT match: the patch component is required.
///
/// Assembled with `concat!` rather than a line-continued raw string. A `\`
/// before a newline does NOT continue a RAW string in Rust, so the first
/// version of this literal kept the backslash, the newline and the indentation
/// — and `cargo fmt` then joined the lines, leaving three runs of literal
/// spaces inside the pattern. It compiled, and it rejected EVERY version
/// including valid SemVer. There is no `(?x)` flag here: whitespace in this
/// pattern is significant, so it must contain none.
#[cfg(feature = "config")]
fn semver_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(concat!(
            r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)",
            r"(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)",
            r"(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?",
            r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$",
        ))
        .expect("the §9.1.2 semver grammar is a compile-time constant")
    })
}

/// A single binding entry. Mirrors the canonical YAML structure defined in
/// protocol-spec §5.12.2.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingEntry {
    pub module_id: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_schema: Option<AutoSchemaValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
    /// Declared execution budget (protocol-spec §5.2 / D-11). `timeout` is in
    /// milliseconds and overrides `executor.default_timeout` for this module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Path of the `*.binding.yaml` / `*.json` file this entry was ingested
    /// from. Populated by the loader, never part of the wire form.
    ///
    /// Threaded into `BINDING_STRICT_SCHEMA_INCOMPATIBLE` and
    /// `BINDING_SCHEMA_INFERENCE_FAILED` diagnostics so the `{file_path}: `
    /// message prefix and the `file_path` details key required by
    /// protocol-spec §5.12.8 carry a real value. apcore-python and
    /// apcore-typescript both pass the binding file path at the same sites.
    #[serde(skip)]
    pub source_file: Option<String>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

/// Top-level binding file structure: `spec_version` + `bindings:` list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingsFile {
    #[serde(default)]
    pub spec_version: Option<String>,
    pub bindings: Vec<BindingEntry>,
}

/// In-memory resolved schema pair for a binding.
#[derive(Debug, Clone)]
#[cfg(feature = "config")]
struct ResolvedSchemas {
    input: serde_json::Value,
    output: serde_json::Value,
}

/// Loads and resolves module bindings from `*.binding.yaml` files.
#[derive(Debug)]
#[cfg(feature = "config")]
pub struct BindingLoader {
    /// Registered binding entries keyed by `module_id`.
    bindings: HashMap<String, BindingEntry>,
    /// Resolved schemas (after `schema_ref` loading) keyed by `module_id`.
    schemas: HashMap<String, ResolvedSchemas>,
    /// PROTOCOL_SPEC §9.1.2 limits, resolved from the configuration when the
    /// loader is invoked with one. Default is unconstrained, which is what a
    /// loader built without a configuration keeps.
    limits: ValidationLimits,
}

#[cfg(feature = "config")]
fn read_schema_reference(
    binding_dir: &Path,
    source_path: &Path,
    schema_root: Option<&Path>,
    reference: &Path,
) -> Result<ResolvedSchemas, ModuleError> {
    let invalid = |reason| {
        ModuleError::new(
            ErrorCode::BindingFileInvalid,
            format!("{}: schema_ref {reason}", source_path.display()),
        )
    };
    if reference.as_os_str().is_empty()
        || reference.has_root()
        || matches!(
            reference.components().next(),
            Some(std::path::Component::Prefix(_))
        )
    {
        return Err(invalid("must be a non-empty relative path"));
    }
    let binding_root = if binding_dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        binding_dir
    }
    .canonicalize()
    .map_err(|_| invalid("binding directory is not readable"))?;
    let resolved = binding_root
        .join(reference)
        .canonicalize()
        .map_err(|_| invalid("file is not readable"))?;
    if !resolved.starts_with(&binding_root)
        && !reference_in_schema_root(&resolved, &binding_root, schema_root)
    {
        return Err(invalid(
            "resolves outside the binding directory and schema root",
        ));
    }

    let content =
        std::fs::read_to_string(&resolved).map_err(|_| invalid("file is not readable"))?;
    let doc: serde_json::Value =
        serde_yaml::from_str(&content).map_err(|_| invalid("file has invalid YAML"))?;
    let doc = doc
        .as_object()
        .ok_or_else(|| invalid("file must be a mapping"))?;
    let schema = |name| match doc.get(name) {
        Some(value @ serde_json::Value::Object(_)) => Ok(value.clone()),
        Some(_) => Err(invalid("input_schema and output_schema must be mappings")),
        None => Ok(serde_json::Value::Object(serde_json::Map::new())),
    };
    Ok(ResolvedSchemas {
        input: schema("input_schema")?,
        output: schema("output_schema")?,
    })
}

#[cfg(feature = "config")]
fn reference_in_schema_root(
    reference: &Path,
    binding_root: &Path,
    schema_root: Option<&Path>,
) -> bool {
    if let Some(root) = schema_root {
        return root
            .canonicalize()
            .is_ok_and(|root| reference.starts_with(root));
    }
    // The conventional sibling is permitted, but it cannot redirect access
    // elsewhere. An explicitly configured root may itself be a symlink.
    binding_root
        .parent()
        .is_some_and(|parent| reference.starts_with(parent.join("schemas")))
}

#[cfg(feature = "config")]
impl BindingLoader {
    /// Create a new empty binding loader.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            schemas: HashMap::new(),
            limits: ValidationLimits::default(),
        }
    }

    /// Load bindings from a JSON file. Same canonical structure as YAML.
    pub fn load_from_file(&mut self, path: &Path) -> Result<(), ModuleError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!("Failed to read binding file '{}': {}", path.display(), e),
            )
        })?;
        let doc: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
            ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!("Failed to parse binding JSON '{}': {}", path.display(), e),
            )
        })?;
        require_bindings_key(path, doc.get("bindings"))?;
        let file: BindingsFile = serde_json::from_value(doc).map_err(|e| {
            ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!("Failed to parse binding JSON '{}': {}", path.display(), e),
            )
        })?;
        self.ingest(file, path, None)
    }

    /// Load bindings from a YAML file.
    pub fn load_from_yaml(&mut self, path: &Path) -> Result<(), ModuleError> {
        self.load_yaml(path, None)
    }

    fn load_yaml(&mut self, path: &Path, schema_root: Option<&Path>) -> Result<(), ModuleError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!("Failed to read binding YAML '{}': {}", path.display(), e),
            )
        })?;
        let doc: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
            ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!("Failed to parse binding YAML '{}': {}", path.display(), e),
            )
        })?;
        require_bindings_key(path, doc.get("bindings"))?;
        let file: BindingsFile = serde_yaml::from_value(doc).map_err(|e| {
            ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!("Failed to parse binding YAML '{}': {}", path.display(), e),
            )
        })?;
        self.ingest(file, path, schema_root)
    }

    /// Common ingestion path after parsing into [`BindingsFile`].
    fn ingest(
        &mut self,
        file: BindingsFile,
        source_path: &Path,
        schema_root: Option<&Path>,
    ) -> Result<(), ModuleError> {
        match file.spec_version.as_deref() {
            None => {
                tracing::warn!(
                    path = %source_path.display(),
                    default_version = CURRENT_SPEC_VERSION,
                    "spec_version missing in bindings file; defaulting. \
                     See protocol-spec §5.12.2"
                );
            }
            Some(v) if !SUPPORTED_SPEC_VERSIONS.contains(&v) => {
                tracing::warn!(
                    path = %source_path.display(),
                    spec_version = v,
                    supported = ?SUPPORTED_SPEC_VERSIONS,
                    "bindings spec_version is newer than supported; proceeding best-effort"
                );
            }
            _ => {}
        }

        let dir = source_path.parent().unwrap_or_else(|| Path::new("."));
        for mut entry in file.bindings {
            // §2.2 target syntax, at parse time — see `validate_target`.
            validate_target(&entry.target)?;
            // §9.1.2 — unconstrained unless the operator configured a limit.
            self.limits.check(&entry, source_path)?;
            // Record the originating file so downstream diagnostics can emit
            // the `{file_path}: ` prefix mandated by protocol-spec §5.12.8
            // (parity with apcore-python / apcore-typescript).
            entry.source_file = Some(source_path.display().to_string());
            let module_id = entry.module_id.clone();
            let schemas = self.resolve_schemas(&entry, dir, source_path, schema_root)?;
            self.schemas.insert(module_id.clone(), schemas);
            self.bindings.insert(module_id, entry);
        }
        Ok(())
    }

    /// Resolve input/output schemas per protocol-spec §5.12.2.
    ///
    /// Detects mode conflicts (multiple schema fields specified together)
    /// and loads `schema_ref` external files. For Rust, `auto_schema` is
    /// deferred until registration supplies the target's typed handler schemas.
    #[allow(clippy::unused_self)] // Kept with the loader's schema-resolution pipeline.
    #[allow(clippy::too_many_lines)] // one linear mode-resolution ladder (§3.4); each arm returns, so splitting would fragment the decision order
    fn resolve_schemas(
        &self,
        entry: &BindingEntry,
        binding_dir: &Path,
        source_path: &Path,
        schema_root: Option<&Path>,
    ) -> Result<ResolvedSchemas, ModuleError> {
        let modes = detect_modes(entry);
        if modes.len() > 1 {
            return Err(ModuleError::new(
                ErrorCode::BindingSchemaModeConflict,
                format!(
                    "{}: binding '{}' specifies multiple schema modes ({}). Choose one. See protocol-spec §5.12.2",
                    source_path.display(),
                    entry.module_id,
                    modes.join(", "),
                ),
            ));
        }

        // Mode 1: explicit input/output_schema
        if entry.input_schema.is_some() || entry.output_schema.is_some() {
            let input = entry.input_schema.clone();
            let output = entry.output_schema.clone();
            if input.is_none() || output.is_none() {
                return Err(ModuleError::new(
                    ErrorCode::BindingFileInvalid,
                    format!(
                        "{}: binding '{}': explicit schema mode requires both 'input_schema' and 'output_schema'",
                        source_path.display(),
                        entry.module_id,
                    ),
                ));
            }
            return Ok(ResolvedSchemas {
                input: input.unwrap(),
                output: output.unwrap(),
            });
        }

        // Mode 2: external file reference
        if let Some(ref_str) = &entry.schema_ref {
            return read_schema_reference(
                binding_dir,
                source_path,
                schema_root,
                Path::new(ref_str),
            );
        }

        // Mode 3: explicit auto_schema (any value) OR implicit default
        let auto_mode = match &entry.auto_schema {
            Some(v) => v.normalize().map_err(|reason| {
                ModuleError::new(
                    ErrorCode::BindingFileInvalid,
                    format!(
                        "{}: binding '{}': {}",
                        source_path.display(),
                        entry.module_id,
                        reason
                    ),
                )
            })?,
            None => None,
        };

        // auto_schema explicitly false → no mode left, error
        if entry.auto_schema.is_some() && auto_mode.is_none() {
            return Err(ModuleError::new(
                ErrorCode::BindingSchemaInferenceFailed,
                format!(
                    "{}: binding '{}': auto_schema is explicitly false; provide input_schema/output_schema or schema_ref instead. See protocol-spec §5.12.2",
                    source_path.display(),
                    entry.module_id,
                ),
            ));
        }

        // The target is resolved at registration. Null marks deferred inference,
        // never a permissive fallback; every mode rejects an uninferable target.
        Ok(ResolvedSchemas {
            input: serde_json::Value::Null,
            output: serde_json::Value::Null,
        })
    }

    /// Load all YAML binding files matching `pattern` in `dir`.
    ///
    /// The explicit-argument tier of [`Self::load_binding_dir_with_config`];
    /// equivalent to calling it with `Some(dir)` and no [`Config`]. Kept so the
    /// existing call sites, which always know their directory, stay unchanged.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BindingFileInvalid`] when `dir` is not a directory, or when
    /// any binding file inside it fails to load.
    pub fn load_binding_dir(
        &mut self,
        dir: &Path,
        pattern: Option<&str>,
    ) -> Result<usize, ModuleError> {
        self.load_binding_dir_with_config(Some(dir), pattern, None)
    }

    /// Load all YAML binding files matching `pattern` in the binding directory,
    /// resolving both from `config` when they are not passed explicitly
    /// (PROTOCOL_SPEC §5.12.6).
    ///
    /// Each of the two settings resolves through the same three tiers —
    /// **explicit argument > config > canonical default**:
    ///
    /// | | argument | config key | default |
    /// |---|---|---|---|
    /// | directory | `dir` | `bindings.dir` | `./bindings` |
    /// | pattern | `pattern` | `bindings.pattern` | `*.binding.yaml` |
    ///
    /// The env tier arrives for free: `APCORE_BINDINGS_DIR` and
    /// `APCORE_BINDINGS_PATTERN` are applied to the `Config` by
    /// `apply_env_overrides` like every other `APCORE_*` variable, so reading
    /// through `Config` implements §9.2's precedence chain and resolves a
    /// relative configured directory against the project root (§9.2.2).
    ///
    /// The default tier is `Config`'s as of spec v1.36.0: that release added the
    /// `bindings` section to `schemas/defaults.schema.json`, which
    /// `CONFIG_DEFAULTS` transcribes, so [`Config::get`] answers `./bindings`
    /// and `*.binding.yaml` for an undeclared key exactly as it does for
    /// `extensions.root` and `schema.root`. The `DEFAULT_BINDING_*` constants
    /// below are reached only when the caller supplies no `Config` at all,
    /// where there is nothing to resolve through; they are pinned equal to
    /// [`Config::default_for`] by an in-crate test.
    ///
    /// Scanning stays **user-invoked**. Nothing in this SDK calls this at client
    /// initialisation, and §5.12.6's MUST does not ask for that: it binds a
    /// loader that was invoked, so no filesystem I/O is added to a startup that
    /// never asked for bindings.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BindingFileInvalid`] when the resolved directory is not a
    /// directory, or when any binding file inside it fails to load.
    pub fn load_binding_dir_with_config(
        &mut self,
        dir: Option<&Path>,
        pattern: Option<&str>,
        config: Option<&crate::config::Config>,
    ) -> Result<usize, ModuleError> {
        let from_config = |key: &str| -> Option<String> {
            config.and_then(|c| c.get(key)).and_then(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty())
                    .map(std::string::ToString::to_string)
            })
        };

        // §9.1.2 — resolve the validation limits from the same configuration
        // that supplies `bindings.dir`. Every one is unconstrained by default,
        // so this changes nothing for a configuration that sets none of them.
        if let Some(cfg) = config {
            self.limits = ValidationLimits::from_config(cfg)?;
        }

        let dir = dir
            .map(Path::to_path_buf)
            .or_else(|| config.and_then(|c| c.get_path(CONFIG_KEY_BINDINGS_DIR)))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_BINDING_DIR));
        let pattern: String = match pattern {
            Some(explicit) => explicit.to_string(),
            None => from_config(CONFIG_KEY_BINDINGS_PATTERN)
                .unwrap_or_else(|| DEFAULT_BINDING_PATTERN.to_string()),
        };

        let dir = dir.as_path();
        let pattern = pattern.as_str();

        if !dir.is_dir() {
            return Err(ModuleError::new(
                ErrorCode::BindingFileInvalid,
                format!(
                    "Binding directory '{}' does not exist or is not a directory",
                    dir.display()
                ),
            ));
        }

        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| {
                ModuleError::new(
                    ErrorCode::BindingFileInvalid,
                    format!("Failed to read directory '{}': {}", dir.display(), e),
                )
            })?
            .filter_map(std::result::Result::ok)
            // PROTOCOL_SPEC 5.12.6 clause 1 / 9.2.3: the pattern is matched
            // with Algorithm A25 against each entry's FILENAME. The suffix
            // comparison this replaces stripped a LEADING `*` and compared with
            // `ends_with`, so `data*.yaml` — an ordinary glob — matched nothing
            // while apcore-python matched one file (#116).
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| match_glob(pattern, name))
            })
            .collect();

        entries.sort_by_key(std::fs::DirEntry::file_name);

        let schema_root = config.map(|cfg| {
            cfg.get_path("schema.root")
                .unwrap_or_else(|| cfg.resolve_path("./schemas"))
        });
        let before = self.bindings.len();
        for entry in entries {
            self.load_yaml(&entry.path(), schema_root.as_deref())?;
        }
        Ok(self.bindings.len() - before)
    }

    /// Resolve a binding by `module_id`.
    pub fn resolve(&self, module_id: &str) -> Result<&BindingEntry, ModuleError> {
        self.bindings.get(module_id).ok_or_else(|| {
            ModuleError::new(
                ErrorCode::BindingModuleNotFound,
                format!("Binding '{module_id}' not found"),
            )
        })
    }

    /// List all loaded binding `module_id`s.
    pub fn list_bindings(&self) -> Vec<&str> {
        self.bindings
            .keys()
            .map(std::string::String::as_str)
            .collect()
    }

    /// Register every loaded binding as a [`FunctionModule`] in `registry`,
    /// using `handlers` keyed by the binding's full `target` string.
    ///
    /// Per protocol-spec §5.12.3, Rust treats the `target` string
    /// as an opaque handler-map key. The user is responsible for providing a
    /// closure for every `target` referenced by the loaded YAML.
    ///
    /// Returns the number of modules registered, or an error if any binding
    /// is missing a handler.
    ///
    /// # Errors
    ///
    /// A binding without an explicit schema pair or schema reference is rejected
    /// with [`ErrorCode::BindingSchemaInferenceFailed`]: this API supplies
    /// untyped handlers, so no schema can be inferred in any automatic mode.
    /// Use [`Self::register_into_with_typed_handlers`] with
    /// [`typed_handler`] for `auto_schema: strict` bindings.
    #[allow(clippy::needless_pass_by_value)] // Preserve the public API that accepts owned handlers.
    pub fn register_into_with_handlers(
        &self,
        registry: &Registry,
        handlers: HashMap<String, BindingHandler>,
    ) -> Result<usize, ModuleError> {
        let mut count = 0usize;
        for (module_id, entry) in &self.bindings {
            let handler = handlers.get(&entry.target).cloned().ok_or_else(|| {
                ModuleError::new(
                    ErrorCode::BindingModuleNotFound,
                    format!(
                        "No handler provided for binding '{}' (target '{}')",
                        module_id, entry.target
                    ),
                )
            })?;

            if entry.input_schema.is_none() && entry.schema_ref.is_none() {
                return Err(strict_inference_failed(
                    entry, module_id,
                    "this registration path supplies untyped handlers, so no schema can be inferred",
                ));
            }

            let schemas = self
                .schemas
                .get(module_id)
                .cloned()
                .unwrap_or(ResolvedSchemas {
                    input: serde_json::json!({"type": "object"}),
                    output: serde_json::json!({"type": "object"}),
                });

            let annotations = annotations_with_resources(entry, module_id)?;

            let mut metadata = entry.metadata.clone();
            apply_display_metadata(&mut metadata, entry.display.as_ref());

            let description = entry.description.clone().unwrap_or_default();
            let documentation = entry.documentation.clone();

            let module = FunctionModule::with_description(
                annotations,
                schemas.input,
                schemas.output,
                description,
                documentation,
                entry.tags.clone(),
                entry.version.as_str(),
                metadata,
                Vec::new(),
                move |inputs, ctx| {
                    let handler = Arc::clone(&handler);
                    Box::pin(async move { (handler)(inputs, ctx).await })
                },
            );

            registry.register_module(module_id.as_str(), Box::new(module))?;
            count += 1;
        }
        Ok(count)
    }

    /// Register bindings using [`TypedBindingHandler`]s that carry auto-derived schemas.
    ///
    /// When a binding entry uses `auto_schema` (explicit or implicit default) AND the
    /// corresponding handler carries schemas (`TypedBindingHandler::input_schema` /
    /// `output_schema` are `Some`), the handler's schemas are used instead of the
    /// deferred inference placeholder. Missing schemas fail loudly in every mode.
    /// This is the primary mechanism for
    /// Rust `auto_schema` support per protocol-spec §5.12.5.
    ///
    /// For bindings with explicit `input_schema`/`output_schema` or `schema_ref`,
    /// the YAML-specified schemas take precedence (handler schemas are ignored).
    #[allow(clippy::needless_pass_by_value)] // Preserve the public API that accepts owned handlers.
    pub fn register_into_with_typed_handlers(
        &self,
        registry: &Registry,
        handlers: HashMap<String, TypedBindingHandler>,
    ) -> Result<usize, ModuleError> {
        let mut count = 0usize;
        for (module_id, entry) in &self.bindings {
            let typed = handlers.get(&entry.target).ok_or_else(|| {
                ModuleError::new(
                    ErrorCode::BindingModuleNotFound,
                    format!(
                        "No handler provided for binding '{}' (target '{}')",
                        module_id, entry.target
                    ),
                )
            })?;

            // Determine final schemas: YAML-resolved vs handler-provided.
            let yaml_schemas = self.schemas.get(module_id);
            let has_explicit_yaml = entry.input_schema.is_some() || entry.schema_ref.is_some();
            // `auto_schema: strict` promises an OpenAI/Anthropic strict-compatible
            // schema (protocol-spec §5.12.5).
            let strict = !has_explicit_yaml && normalized_auto_mode(entry) == Some("strict");

            let (input_schema, output_schema) = if has_explicit_yaml {
                // YAML-specified schemas take precedence.
                let s = yaml_schemas.cloned().unwrap_or(ResolvedSchemas {
                    input: serde_json::json!({"type": "object"}),
                    output: serde_json::json!({"type": "object"}),
                });
                (s.input, s.output)
            } else if let (Some(is), Some(os)) = (&typed.input_schema, &typed.output_schema) {
                // Handler provides auto-derived schemas (schemars).
                (is.clone(), os.clone())
            } else {
                // No schema to check means the strict promise cannot be kept.
                // Falling back to the permissive `{"type":"object"}` pair here
                // would make `assert_openai_strict_compatible` succeed
                // vacuously — the exact defect apcore-python
                // (`BindingSchemaInferenceFailedError`, bindings.py) and
                // apcore-typescript (bindings.ts) raise on.
                return Err(strict_inference_failed(
                    entry,
                    module_id,
                    "the supplied TypedBindingHandler carries no input_schema/output_schema",
                ));
            };

            if strict {
                let file_path = entry.source_file.as_deref();
                assert_openai_strict_compatible(
                    &input_schema,
                    module_id,
                    Some("input"),
                    file_path,
                )?;
                assert_openai_strict_compatible(
                    &output_schema,
                    module_id,
                    Some("output"),
                    file_path,
                )?;
            }

            let annotations = annotations_with_resources(entry, module_id)?;
            let mut metadata = entry.metadata.clone();
            apply_display_metadata(&mut metadata, entry.display.as_ref());

            let description = entry.description.clone().unwrap_or_default();
            let documentation = entry.documentation.clone();
            let handler = Arc::clone(&typed.handler);

            let module = FunctionModule::with_description(
                annotations,
                input_schema,
                output_schema,
                description,
                documentation,
                entry.tags.clone(),
                entry.version.as_str(),
                metadata,
                Vec::new(),
                move |inputs, ctx| {
                    let handler = Arc::clone(&handler);
                    Box::pin(async move { (handler)(inputs, ctx).await })
                },
            );

            registry.register_module(module_id.as_str(), Box::new(module))?;
            count += 1;
        }
        Ok(count)
    }
}

#[cfg(feature = "config")]
impl Default for BindingLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalized `auto_schema` mode for an entry: `Some("permissive")`,
/// `Some("strict")`, or `None` (absent, explicitly `false`, or invalid — the
/// latter two are already rejected by `resolve_schemas` at ingest).
#[cfg(feature = "config")]
fn normalized_auto_mode(entry: &BindingEntry) -> Option<&'static str> {
    match entry.auto_schema.as_ref()?.normalize() {
        Ok(Some("strict")) => Some("strict"),
        Ok(Some(_)) => Some("permissive"),
        _ => None,
    }
}

/// Build the `BINDING_SCHEMA_INFERENCE_FAILED` error raised when a binding
/// has no typed schema available in any automatic inference mode.
///
/// Mirrors apcore-python `BindingSchemaInferenceFailedError` and
/// apcore-typescript `BindingSchemaInferenceFailedError`: message carries the
/// `{file_path}: ` prefix from protocol-spec §5.12.8 and the details
/// map carries `module_id`, `target` and `file_path`.
#[cfg(feature = "config")]
fn strict_inference_failed(entry: &BindingEntry, module_id: &str, reason: &str) -> ModuleError {
    let mode = normalized_auto_mode(entry).unwrap_or("implicit");
    let loc = entry
        .source_file
        .as_deref()
        .map_or_else(String::new, |p| format!("{p}: "));
    let mut details = HashMap::new();
    details.insert(
        "module_id".to_string(),
        serde_json::Value::String(module_id.to_string()),
    );
    details.insert(
        "target".to_string(),
        serde_json::Value::String(entry.target.clone()),
    );
    if let Some(path) = entry.source_file.as_deref() {
        details.insert(
            "file_path".to_string(),
            serde_json::Value::String(path.to_string()),
        );
    }
    ModuleError::new(
        ErrorCode::BindingSchemaInferenceFailed,
        format!(
            "{loc}binding '{module_id}' (target '{}') declares auto_schema: {mode} but no schema could be inferred: {reason}. \
             Register it via BindingLoader::register_into_with_typed_handlers with a `typed_handler`, or declare input_schema/output_schema explicitly. \
             See protocol-spec §5.12.5",
            entry.target,
        ),
    )
    .with_details(details)
}

/// Reject a bindings document that omits the required top-level `bindings`
/// key, with the canonical `BindingFileInvalidError` message.
///
/// protocol-spec §5.12.8 fixes the template as
/// `"Invalid binding file '{file_path}': {reason}"` and
/// `conformance/fixtures/binding_errors.json` pins the exact string for this
/// condition. apcore-rust used to surface serde's own
/// `missing field \`bindings\`` text instead, which shares no wording with what
/// apcore-python and apcore-typescript emit for the same file — a message-parity
/// divergence that no test could see, because the driver for that fixture case
/// asserted nothing (apcore#93).
#[cfg(feature = "config")]
fn require_bindings_key<T>(path: &Path, bindings: Option<&T>) -> Result<(), ModuleError> {
    if bindings.is_some() {
        return Ok(());
    }
    Err(ModuleError::new(
        ErrorCode::BindingFileInvalid,
        format!(
            "Invalid binding file '{}': missing required top-level key 'bindings'",
            path.display()
        ),
    ))
}

/// Validate a binding `target` against the §2.2 string-target syntax.
///
/// The canonical regex is
/// `^[@./a-zA-Z_][-@./a-zA-Z0-9_]*:[a-zA-Z_][a-zA-Z0-9_.]*$`; the part that is
/// meaningful for every SDK — and the only part apcore-rust can act on — is the
/// `<module_path>:<symbol>` split, since Rust resolves a target through an
/// opaque handler-map key and never touches the filesystem (§3.7 "Rust
/// caveat"), so the traversal-rejection §2.2 asks of TypeScript has no analogue
/// here.
///
/// §2.2: "Such validation produces `BindingInvalidTargetError` at parse time."
/// Before apcore#93 apcore-rust performed none: `ErrorCode::BindingInvalidTarget`
/// was declared, categorised, and raised by nothing, so a `target` with no
/// separator loaded silently and failed much later as an unrelated
/// handler-lookup miss. The message matches apcore-python's
/// `BindingInvalidTargetError` byte for byte.
#[cfg(feature = "config")]
fn validate_target(target: &str) -> Result<(), ModuleError> {
    let well_formed = target
        .split_once(':')
        .is_some_and(|(module_path, symbol)| !module_path.is_empty() && !symbol.is_empty());
    if well_formed {
        return Ok(());
    }
    Err(ModuleError::new(
        ErrorCode::BindingInvalidTarget,
        format!("Invalid binding target '{target}'. Expected format: 'module.path:callable_name'."),
    ))
}

/// Detect which schema-mode fields a binding entry sets.
#[cfg(feature = "config")]
fn detect_modes(entry: &BindingEntry) -> Vec<String> {
    let mut modes = Vec::new();
    if entry.auto_schema.is_some() {
        modes.push("auto_schema".to_string());
    }
    if entry.input_schema.is_some() || entry.output_schema.is_some() {
        modes.push("input_schema/output_schema".to_string());
    }
    if entry.schema_ref.is_some() {
        modes.push("schema_ref".to_string());
    }
    modes
}

/// Translate an `annotations` JSON object into [`ModuleAnnotations`].
///
/// Deserialization is delegated to [`ModuleAnnotations`]'s own `Deserialize`
/// impl, which is the single definition of the §4.4.1 wire format: all twelve
/// declared keys plus `discoverable`, the nested `extra` object, the legacy
/// flattened form, and the nested-wins precedence rule.
///
/// A hand-rolled parser lived here and matched five keys —
/// `readonly` / `destructive` / `idempotent` / `requires_approval` /
/// `open_world`. Everything else, `streaming`, `cacheable`, `cache_ttl`,
/// `cache_key_fields`, `paginated`, `pagination_style` and the literal key
/// `extra` itself, fell into the catch-all and was nested under its own name,
/// so `{"extra": {...}}` deserialized to `extra.extra` and
/// `annotations: {streaming: true}` never reached the streaming-interface check
/// that apcore-python and apcore-typescript fail registration on.
///
/// Missing or `null` input yields `ModuleAnnotations::default()`.
///
/// # Errors
///
/// `BINDING_FILE_INVALID` when the block is not an object or a declared key
/// carries the wrong type (e.g. `cache_ttl: "soon"`).
/// Parse a binding's annotations and merge its declared `resources` block into
/// `annotations.extra`, where the executor reads it (D-11). Keeping the budget
/// in `extra` matches the canonical wire form (§4.4.1): extension keys are not
/// flattened onto the annotations root.
#[cfg(feature = "config")]
fn annotations_with_resources(
    entry: &BindingEntry,
    module_id: &str,
) -> Result<ModuleAnnotations, ModuleError> {
    let mut annotations = annotations_from_value(module_id, entry.annotations.as_ref())?;
    if let Some(resources) = entry.resources.clone() {
        annotations.extra.insert("resources".to_string(), resources);
    }
    Ok(annotations)
}

#[cfg(feature = "config")]
fn annotations_from_value(
    module_id: &str,
    value: Option<&serde_json::Value>,
) -> Result<ModuleAnnotations, ModuleError> {
    let Some(value) = value else {
        return Ok(ModuleAnnotations::default());
    };
    if value.is_null() {
        return Ok(ModuleAnnotations::default());
    }
    serde_json::from_value::<ModuleAnnotations>(value.clone()).map_err(|e| {
        ModuleError::new(
            ErrorCode::BindingFileInvalid,
            format!("Invalid `annotations` block for binding '{module_id}': {e}"),
        )
    })
}

/// Publish the canonical overlay and retain the Rust compatibility key.
#[cfg(feature = "config")]
fn apply_display_metadata(
    metadata: &mut HashMap<String, serde_json::Value>,
    display: Option<&serde_json::Value>,
) {
    if let Some(value) = display {
        metadata.insert("display".to_string(), value.clone());
        metadata
            .entry("apcore.display".to_string())
            .or_insert_with(|| value.clone());
    }
}

#[cfg(all(test, feature = "config"))]
mod tests {

    /// PROTOCOL_SPEC §9.1.2 requirement 6 — BOTH halves.
    ///
    /// The rejection half alone is not coverage, and this test exists because
    /// that is exactly how the defect shipped: the only case anyone wrote was
    /// `version: "1.0"` expecting a rejection, and it passed against a pattern
    /// that rejected *everything*. A constraint that refuses valid input is as
    /// broken as one that accepts invalid input, and only the acceptance half
    /// can tell them apart.
    #[test]
    fn semver_grammar_accepts_valid_versions_and_rejects_invalid_ones() {
        for accepted in [
            "0.0.0",
            "1.0.0",
            "0.1.0",
            "10.20.30",
            "1.2.3-rc.1",
            "1.2.3-0.3.7",
            "1.2.3+build.5",
            "1.2.3-beta.1+exp.sha.5114f85",
        ] {
            assert!(
                semver_re().is_match(accepted),
                "§9.1.2's grammar must accept the valid SemVer {accepted:?}"
            );
        }
        for rejected in ["1.0", "1", "v1.0.0", "1.0.0.0", "01.0.0", "", "1.0.0 "] {
            assert!(
                !semver_re().is_match(rejected),
                "§9.1.2's grammar must reject {rejected:?}"
            );
        }
    }

    /// The pattern must contain no whitespace: there is no `(?x)` flag, so a
    /// stray space is a mandatory character in the subject string. This is the
    /// mechanism of the defect above, pinned separately so a future reformat
    /// cannot reintroduce it silently.
    #[test]
    fn semver_pattern_contains_no_whitespace() {
        let pattern = semver_re().as_str();
        assert!(
            !pattern.chars().any(char::is_whitespace),
            "the §9.1.2 semver pattern must contain no whitespace, found: {pattern:?}"
        );
    }
    use super::*;
    use serde_json::json;

    fn write_yaml(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn test_binding_loader_new_is_empty() {
        let loader = BindingLoader::new();
        assert_eq!(loader.list_bindings(), Vec::<&str>::new());
    }

    #[test]
    fn test_binding_loader_default() {
        let loader = BindingLoader::default();
        assert_eq!(loader.list_bindings(), Vec::<&str>::new());
    }

    #[test]
    fn test_resolve_missing_binding() {
        let loader = BindingLoader::new();
        let result = loader.resolve("nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, ErrorCode::BindingModuleNotFound);
        assert!(err.message.contains("nonexistent"));
    }

    #[test]
    fn test_load_from_yaml_canonical_format() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: utils.greet
    target: "greet:greet_fn"
    description: "Greet someone"
    tags: ["util"]
    auto_schema: true
"#;
        let p = write_yaml(&dir, "greet.binding.yaml", yaml);

        let mut loader = BindingLoader::new();
        loader.load_from_yaml(&p).unwrap();
        let entry = loader.resolve("utils.greet").unwrap();
        assert_eq!(entry.target, "greet:greet_fn");
        assert_eq!(entry.description.as_deref(), Some("Greet someone"));
        assert_eq!(entry.tags, vec!["util"]);
    }

    #[test]
    fn test_load_from_json_canonical_format() {
        let dir = tempfile::tempdir().unwrap();
        let body = json!({
            "spec_version": "1.0",
            "bindings": [
                {"module_id": "a.b", "target": "mod:fn", "input_schema": {"type": "object"}, "output_schema": {"type": "object"}}
            ]
        });
        let p = dir.path().join("b.json");
        std::fs::write(&p, serde_json::to_string(&body).unwrap()).unwrap();

        let mut loader = BindingLoader::new();
        loader.load_from_file(&p).unwrap();
        assert_eq!(loader.list_bindings().len(), 1);
    }

    #[test]
    fn test_mode_conflict_auto_schema_plus_input_schema() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    auto_schema: true
    input_schema: {type: object}
    output_schema: {type: object}
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        let err = loader.load_from_yaml(&p).unwrap_err();
        assert_eq!(err.code, ErrorCode::BindingSchemaModeConflict);
        assert!(err.message.contains("multiple schema modes"));
    }

    #[test]
    fn test_mode_conflict_schema_ref_plus_auto_schema() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    auto_schema: strict
    schema_ref: "./schema.yaml"
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        let err = loader.load_from_yaml(&p).unwrap_err();
        assert_eq!(err.code, ErrorCode::BindingSchemaModeConflict);
    }

    #[test]
    fn test_explicit_input_only_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    input_schema: {type: object}
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        let err = loader.load_from_yaml(&p).unwrap_err();
        assert_eq!(err.code, ErrorCode::BindingFileInvalid);
        assert!(err.message.contains("requires both"));
    }

    #[test]
    fn test_implicit_auto_schema_default() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        loader.load_from_yaml(&p).unwrap();
        // Implicit auto: should resolve without error.
        assert_eq!(loader.list_bindings().len(), 1);
    }

    #[test]
    fn test_auto_schema_false_explicit_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    auto_schema: false
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        let err = loader.load_from_yaml(&p).unwrap_err();
        assert_eq!(err.code, ErrorCode::BindingSchemaInferenceFailed);
    }

    #[test]
    fn test_auto_schema_strict_value() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    auto_schema: strict
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        loader.load_from_yaml(&p).unwrap();
        let entry = loader.resolve("x").unwrap();
        match &entry.auto_schema {
            Some(AutoSchemaValue::Mode(m)) => assert_eq!(m, "strict"),
            other => panic!("expected Mode(strict), got {other:?}"),
        }
    }

    #[test]
    fn test_auto_schema_invalid_string_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    auto_schema: "not-a-mode"
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        let err = loader.load_from_yaml(&p).unwrap_err();
        assert_eq!(err.code, ErrorCode::BindingFileInvalid);
        assert!(err.message.contains("not-a-mode"));
    }

    #[test]
    fn test_schema_ref_loads_external_yaml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("schema.yaml"),
            r"
input_schema:
  type: object
  properties:
    name: {type: string}
output_schema:
  type: object
  properties:
    greeting: {type: string}
",
        )
        .unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    schema_ref: "./schema.yaml"
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        loader.load_from_yaml(&p).unwrap();
        let schemas = loader.schemas.get("x").unwrap();
        assert_eq!(schemas.input["properties"]["name"]["type"], "string");
        assert_eq!(schemas.output["properties"]["greeting"]["type"], "string");
    }

    #[test]
    fn test_schema_ref_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    schema_ref: "./does-not-exist.yaml"
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        let err = loader.load_from_yaml(&p).unwrap_err();
        assert_eq!(err.code, ErrorCode::BindingFileInvalid);
        assert!(err.message.contains("schema_ref"));
    }

    #[test]
    fn test_load_binding_dir_default_pattern() {
        let dir = tempfile::tempdir().unwrap();
        write_yaml(
            &dir,
            "a.binding.yaml",
            r#"
spec_version: "1.0"
bindings:
  - module_id: alpha
    target: "m.alpha:fn"
"#,
        );
        write_yaml(
            &dir,
            "b.binding.yaml",
            r#"
spec_version: "1.0"
bindings:
  - module_id: beta
    target: "m.beta:fn"
"#,
        );
        write_yaml(
            &dir,
            "ignored.yaml",
            r#"
spec_version: "1.0"
bindings:
  - module_id: ignored
    target: "m.ignored:fn"
"#,
        );

        let mut loader = BindingLoader::new();
        let count = loader.load_binding_dir(dir.path(), None).unwrap();
        assert_eq!(count, 2);
        assert!(loader.resolve("alpha").is_ok());
        assert!(loader.resolve("beta").is_ok());
        assert!(loader.resolve("ignored").is_err());
    }

    #[test]
    fn test_auto_schema_value_normalize() {
        assert_eq!(
            AutoSchemaValue::Bool(true).normalize().unwrap(),
            Some("permissive")
        );
        assert_eq!(AutoSchemaValue::Bool(false).normalize().unwrap(), None);
        assert_eq!(
            AutoSchemaValue::Mode("true".to_string())
                .normalize()
                .unwrap(),
            Some("permissive")
        );
        assert_eq!(
            AutoSchemaValue::Mode("permissive".to_string())
                .normalize()
                .unwrap(),
            Some("permissive")
        );
        assert_eq!(
            AutoSchemaValue::Mode("strict".to_string())
                .normalize()
                .unwrap(),
            Some("strict")
        );
        assert!(AutoSchemaValue::Mode("invalid".to_string())
            .normalize()
            .is_err());
    }

    #[test]
    fn binding_resources_are_folded_into_annotations_extra() {
        // A binding can declare the per-module budget (D-11). It must survive
        // `deny_unknown_fields` and land where BuiltinExecute reads it.
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: slow.mod
    target: "m:f"
    resources:
      timeout: 1234
"#;
        let file: BindingsFile = serde_yaml_ng::from_str(yaml).unwrap();
        let entry = &file.bindings[0];
        let annotations = annotations_with_resources(entry, &entry.module_id).unwrap();
        assert_eq!(
            annotations.extra.get("resources"),
            Some(&serde_json::json!({"timeout": 1234}))
        );
    }

    #[test]
    fn test_annotations_round_trip_through_loader() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    annotations:
      readonly: true
      idempotent: true
      destructive: false
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        loader.load_from_yaml(&p).unwrap();
        let entry = loader.resolve("x").unwrap();
        let ann = entry.annotations.as_ref().unwrap();
        assert_eq!(ann["readonly"], true);
        assert_eq!(ann["idempotent"], true);
        assert_eq!(ann["destructive"], false);
    }

    #[test]
    fn test_display_round_trip_through_loader() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: x
    target: "m:f"
    display:
      alias: "x_short"
      cli:
        alias: "x"
"#;
        let p = write_yaml(&dir, "x.binding.yaml", yaml);
        let mut loader = BindingLoader::new();
        loader.load_from_yaml(&p).unwrap();
        let entry = loader.resolve("x").unwrap();
        let display = entry.display.as_ref().unwrap();
        assert_eq!(display["alias"], "x_short");
        assert_eq!(display["cli"]["alias"], "x");
    }
}

#[cfg(all(test, feature = "config"))]
mod bindings_dir_from_config_tests {
    //! `bindings.dir` / `bindings.pattern` reaching the loader from a
    //! **config file** (aiperceivable/apcore#114, PROTOCOL_SPEC §5.12.6).
    //!
    //! The discriminating shape #114 asks for is a config *file* that declares
    //! the key, with no explicit directory argument. Every pre-existing
    //! `load_binding_dir` test passes the directory explicitly, and that is
    //! precisely the one path which behaved identically before and after the
    //! fix, so none of them can tell the two apart.

    use super::{BindingLoader, DEFAULT_BINDING_DIR, DEFAULT_BINDING_PATTERN};
    use crate::config::Config;
    use std::path::Path;

    /// Write a §9.1-valid config file that declares `bindings` as given.
    ///
    /// The required fields are the ones legacy-mode `validate()` enforces
    /// (A-D-03): version, project.name, extensions.root, schema.root,
    /// acl.root, acl.default_effect.
    fn write_config(dir: &Path, bindings_yaml: &str) -> std::path::PathBuf {
        let path = dir.join("apcore.yaml");
        std::fs::write(
            &path,
            format!(
                "version: '1.0.0'\n\
                 project:\n  name: demo\n\
                 extensions:\n  root: ./extensions\n\
                 schema:\n  root: ./schemas\n\
                 acl:\n  root: ./acl\n  default_effect: deny\n\
                 {bindings_yaml}"
            ),
        )
        .unwrap();
        path
    }

    fn write_binding(dir: &Path, name: &str, module_id: &str) {
        std::fs::write(
            dir.join(name),
            format!(
                "spec_version: \"1.0\"\nbindings:\n  - module_id: {module_id}\n    target: \"m.{module_id}:fn\"\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn bindings_dir_declared_in_a_config_file_drives_the_scan() {
        // The #114 case. Before this wiring existed the key was registered in
        // the §9.1.1 surface, validated by §9.3, and read by nothing: an
        // operator who set `bindings.dir` in apcore.yaml got no scan at all.
        let workspace = tempfile::tempdir().unwrap();
        let bindings_dir = workspace.path().join("my-bindings");
        std::fs::create_dir(&bindings_dir).unwrap();
        write_binding(&bindings_dir, "a.binding.yaml", "alpha");
        write_binding(&bindings_dir, "b.binding.yaml", "beta");

        let config_path = write_config(workspace.path(), "bindings:\n  dir: ./my-bindings\n");
        let config = Config::load(&config_path).unwrap();
        assert_eq!(
            config
                .get("bindings.dir")
                .and_then(|v| v.as_str().map(str::to_string)),
            Some("./my-bindings".to_string()),
            "precondition: the key must survive the load"
        );

        let mut loader = BindingLoader::new();
        let count = loader
            .load_binding_dir_with_config(None, None, Some(&config))
            .unwrap();

        assert_eq!(
            count, 2,
            "the configured directory must actually be scanned"
        );
        assert!(loader.resolve("alpha").is_ok());
        assert!(loader.resolve("beta").is_ok());
    }

    #[test]
    fn bindings_pattern_declared_in_a_config_file_drives_the_match() {
        // `bindings.pattern` was in the same position as `dir`: its default
        // lived in the loader signature rather than being read from config.
        let workspace = tempfile::tempdir().unwrap();
        let bindings_dir = workspace.path().join("bindings");
        std::fs::create_dir(&bindings_dir).unwrap();
        write_binding(&bindings_dir, "a.bind.yaml", "alpha");
        write_binding(&bindings_dir, "b.binding.yaml", "beta");

        let config_path = write_config(
            workspace.path(),
            &format!(
                "bindings:\n  dir: {}\n  pattern: '*.bind.yaml'\n",
                bindings_dir.to_str().unwrap()
            ),
        );
        let config = Config::load(&config_path).unwrap();

        let mut loader = BindingLoader::new();
        let count = loader
            .load_binding_dir_with_config(None, None, Some(&config))
            .unwrap();

        assert_eq!(count, 1, "only the configured pattern must match");
        assert!(loader.resolve("alpha").is_ok());
        assert!(
            loader.resolve("beta").is_err(),
            "the default *.binding.yaml pattern must not survive a configured one"
        );
    }

    #[test]
    fn explicit_arguments_outrank_the_config_file() {
        // Tier 1 of explicit > config > default. An embedder who names a
        // directory must never have it redirected by a config file.
        let workspace = tempfile::tempdir().unwrap();
        let configured = workspace.path().join("configured");
        let explicit = workspace.path().join("explicit");
        std::fs::create_dir(&configured).unwrap();
        std::fs::create_dir(&explicit).unwrap();
        write_binding(&configured, "c.binding.yaml", "from_config");
        write_binding(&explicit, "e.binding.yaml", "from_argument");

        let config_path = write_config(
            workspace.path(),
            &format!("bindings:\n  dir: {}\n", configured.to_str().unwrap()),
        );
        let config = Config::load(&config_path).unwrap();

        let mut loader = BindingLoader::new();
        loader
            .load_binding_dir_with_config(Some(&explicit), None, Some(&config))
            .unwrap();

        assert!(loader.resolve("from_argument").is_ok());
        assert!(loader.resolve("from_config").is_err());
    }

    #[test]
    fn default_applies_when_the_config_declares_no_bindings_section() {
        // Tier 3. Spec v1.36.0 moved this default INTO `defaults.schema.json`,
        // so `CONFIG_DEFAULTS` now carries it and `Config::get` is what answers
        // `./bindings` here — the loader's own constant is not consulted on
        // this path any more.
        //
        // This assertion was `Config::default_for("bindings.dir").is_none()`
        // and existed to stop the loader's hardcoded default from being quietly
        // deleted while nothing else supplied one. v1.36.0 supplies one, so the
        // guard is inverted rather than dropped: the key MUST now resolve
        // through the canonical table, and to the same value the loader would
        // otherwise have used.
        assert_eq!(
            Config::default_for("bindings.dir")
                .and_then(|v| v.as_str().map(str::to_string))
                .as_deref(),
            Some(DEFAULT_BINDING_DIR),
            "precondition: since spec v1.36.0 the canonical default table owns \
             this default, and it must agree with the loader's constant"
        );

        let workspace = tempfile::tempdir().unwrap();
        let config_path = write_config(workspace.path(), "");
        let config = Config::load(&config_path).unwrap();

        let mut loader = BindingLoader::new();
        let err = loader
            .load_binding_dir_with_config(None, None, Some(&config))
            .unwrap_err();

        assert!(
            err.message.contains(DEFAULT_BINDING_DIR),
            "the canonical default directory must be the one attempted: {}",
            err.message
        );
    }

    #[test]
    fn default_applies_when_no_config_is_supplied_at_all() {
        let mut loader = BindingLoader::new();
        let err = loader
            .load_binding_dir_with_config(None, None, None)
            .unwrap_err();
        assert!(err.message.contains(DEFAULT_BINDING_DIR), "{}", err.message);
    }

    #[test]
    fn the_legacy_two_argument_entry_point_is_unchanged() {
        // Source compatibility: `load_binding_dir(dir, None)` still means
        // "this directory, default pattern", consulting no config.
        let dir = tempfile::tempdir().unwrap();
        write_binding(dir.path(), "a.binding.yaml", "alpha");
        write_binding(dir.path(), "ignored.yaml", "ignored");

        let mut loader = BindingLoader::new();
        let count = loader.load_binding_dir(dir.path(), None).unwrap();

        assert_eq!(count, 1);
        assert!(loader.resolve("alpha").is_ok());
        assert_eq!(DEFAULT_BINDING_PATTERN, "*.binding.yaml");
    }

    #[test]
    fn the_loader_default_matches_the_canonical_default_table() {
        // The constants survive v1.36.0 only for the no-`Config` invocation
        // (`load_binding_dir_with_config(None, None, None)`), where there is
        // nothing for `Config::get` to answer from. A second copy of a
        // canonical value is a drift risk, so it is pinned instead of trusted:
        // if `defaults.schema.json` ever changes either value,
        // `CONFIG_DEFAULTS` follows it (the governance fixture forces that) and
        // this test then reports the loader constant left behind.
        for (key, loader_default) in [
            ("bindings.dir", DEFAULT_BINDING_DIR),
            ("bindings.pattern", DEFAULT_BINDING_PATTERN),
        ] {
            assert_eq!(
                Config::default_for(key)
                    .and_then(|v| v.as_str().map(str::to_string))
                    .as_deref(),
                Some(loader_default),
                "`{key}`: the loader constant and the canonical default table \
                 must not drift"
            );
        }
    }
}
