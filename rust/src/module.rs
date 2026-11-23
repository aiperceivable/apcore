// APCore Protocol — Module trait and related types
// Spec reference: Module definition, annotations, preflight checks

use async_trait::async_trait;
use futures_core::Stream;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::LazyLock;

use crate::context::Context;
use crate::errors::ModuleError;

/// A stream of output chunks from a streaming module.
///
/// Each item is a `Result<Value, ModuleError>` so a module can fail mid-stream.
/// The stream is `Send + 'static` (no borrows from the producing module) so the
/// executor can drive it across `.await` points without lifetime issues.
pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<serde_json::Value, ModuleError>> + Send>>;

/// Core trait that all `APCore` modules must implement.
#[async_trait]
pub trait Module: Send + Sync {
    /// Returns the JSON Schema describing this module's input.
    fn input_schema(&self) -> serde_json::Value;

    /// Returns the JSON Schema describing this module's output.
    fn output_schema(&self) -> serde_json::Value;

    /// Returns a human-readable description of this module.
    fn description(&self) -> &str;

    /// Execute the module with the given inputs and context.
    async fn execute(
        &self,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError>;

    /// Stream execution — returns an async `Stream` of output chunks.
    ///
    /// Returns `None` if the module does not support streaming, signaling
    /// the executor to fall back to `execute()`. Modules that support
    /// streaming should override this to yield chunks incrementally — each
    /// `yield` is delivered to the caller as soon as it is produced (true
    /// streaming, no buffering).
    ///
    /// Note: this method is *not* `async` even though it returns a stream.
    /// The returned `ChunkStream` is itself an async iterator; constructing
    /// it must be cheap and synchronous so the executor can wire it into
    /// its own pipeline before the first chunk is awaited.
    ///
    /// **Validation contract:** `Executor::stream` validates the module's
    /// *merged* output (all chunks deep-merged) against `output_schema` only
    /// *after* the stream is exhausted (Phase 3), when the chunks are already
    /// delivered: a failure is published as
    /// `apcore.stream.post_validation_failed`, not yielded. Individual chunks
    /// are **not** validated as they are yielded. Callers performing incremental chunk
    /// processing must tolerate receiving chunks that may not independently
    /// satisfy `output_schema`. If per-chunk schema guarantees are required,
    /// validate each chunk inside this method before yielding it.
    fn stream(
        &self,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Option<ChunkStream> {
        None
    }

    /// Return a structured description of this module for AI/LLM consumption (spec §5.6).
    /// Default: builds description from `input_schema`, `output_schema`,
    /// `description`, and `annotations` (the spec-default shape; mirrors
    /// apcore-python / apcore-typescript).
    fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "description": self.description(),
            "input_schema": self.input_schema(),
            "output_schema": self.output_schema(),
            "annotations": self.annotations(),
        })
    }

    /// Run preflight checks before execution.
    ///
    /// Cross-language alignment (D11-009): mirrors apcore-python
    /// `Module.preflight(inputs, context) -> list[str]` and apcore-typescript
    /// `preflight(inputs, context): string[]`. Returns a list of advisory
    /// warning strings; an empty list means "no concerns". Modules that need
    /// to gate execution should return errors from `execute()` directly
    /// instead — preflight is non-fatal.
    ///
    /// `ctx` is `Option<&Context>` because `Executor::validate(module_id,
    /// inputs, ctx)` accepts a `None` context for call-chain-free preflight
    /// (matching Python's `executor.validate(..., context=None)`); modules
    /// that need a context for their checks must handle the `None` case.
    ///
    /// The default implementation returns an empty warning list. Modules
    /// override this to inspect inputs (e.g., flag oversize payloads, warn
    /// on deprecated argument shapes) without rejecting the call.
    fn preflight(
        &self,
        _inputs: &serde_json::Value,
        _ctx: Option<&Context<serde_json::Value>>,
    ) -> Vec<String> {
        Vec::new()
    }

    /// Module-instance tags (D11-003).
    ///
    /// Cross-language alignment with apcore-python (`registry.py:1027`,
    /// reads `getattr(mod, 'tags', [])`) and apcore-typescript
    /// (`registry.ts:689`, reads `mod['tags']`). Modules MAY override
    /// this to participate in `Registry::list(tags=...)` filtering even
    /// when registered without an explicit `ModuleDescriptor` (e.g. via
    /// `register_module(name, module)`). The Rust `Registry::list`
    /// unions these instance tags with `descriptor.tags`.
    ///
    /// The default returns an empty Vec.
    fn tags(&self) -> Vec<String> {
        Vec::new()
    }

    /// Module-instance behavior annotations (D10-003).
    ///
    /// Cross-language alignment with apcore-python (`builtin_steps.py`, reads
    /// `getattr(module, "annotations", None)`) and apcore-typescript
    /// (`builtin-steps.ts`, reads `mod['annotations']`). `BuiltinApprovalGate`
    /// sources the [`ApprovalRequest`](crate::approval::ApprovalRequest)
    /// annotations from the resolved live module instance via this accessor
    /// (PROTOCOL_SPEC §7.4 Step 5), not from the registry descriptor.
    ///
    /// The default implementation returns [`ModuleAnnotations::default`], so
    /// existing `impl Module` blocks remain source-compatible.
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations::default()
    }

    /// Long-form module documentation (Markdown), when the module carries any.
    ///
    /// Read by `Registry::register_module` so a declaration made on the module
    /// instance — e.g. a `*.binding.yaml` `documentation:` block, which the
    /// binding loader folds into the `FunctionModule` it builds — reaches the
    /// registered `ModuleDescriptor` instead of being hardcoded to `None`.
    /// Cross-language alignment with apcore-python (`registry/metadata.py`
    /// merges the module's `documentation`) and apcore-typescript
    /// (`registry/metadata-pure.ts`).
    ///
    /// The default returns `None`, so existing `impl Module` blocks are
    /// unaffected.
    fn documentation(&self) -> Option<&str> {
        None
    }

    /// Free-form module metadata, when the module carries any.
    ///
    /// Read by `Registry::register_module` for the same reason as
    /// [`documentation`](Self::documentation): the binding loader computes the
    /// `display:` block into `metadata["apcore.display"]` on the module it
    /// builds, and that map used to be overwritten with an empty one at
    /// registration.
    ///
    /// The default returns an empty map.
    fn metadata(&self) -> HashMap<String, serde_json::Value> {
        HashMap::new()
    }

    /// The module's own declared version, when it carries one.
    ///
    /// Read by `Registry::register_module`, which used to hardcode
    /// [`DEFAULT_MODULE_VERSION`](crate::registry::registry::DEFAULT_MODULE_VERSION)
    /// because there was nothing to read. That completes D-97: `FunctionModule`
    /// stored `version` and `examples` as fields with no corresponding trait
    /// accessor, exactly as it stored `annotations` / `tags` / `documentation` /
    /// `metadata` before D-97 added theirs — so a `*.binding.yaml` declaring
    /// `version:` was **validated** against
    /// `validation.binding.version_require_semver` and then discarded, and the
    /// module registered as `1.0.0`.
    ///
    /// apcore-python (`merge_module_metadata`: `getattr(module, "version",
    /// "1.0.0")`) and apcore-typescript (`mergeModuleMetadata`) both carry it.
    ///
    /// The default returns `None`, which the registry reads as "use the
    /// default version" — the same fallback the peers apply.
    fn version(&self) -> Option<&str> {
        None
    }

    /// The module's own declared examples, when it carries any.
    ///
    /// The second half of the same gap as [`version`](Self::version): stored as
    /// a `FunctionModule` field, reachable through no accessor, and replaced
    /// with an empty `Vec` at registration. Both peers merge the module's
    /// examples into the descriptor.
    ///
    /// The default returns an empty Vec.
    fn examples(&self) -> Vec<ModuleExample> {
        Vec::new()
    }

    /// The module's own declared dependencies, when it carries any.
    ///
    /// The last field of the same family as [`version`](Self::version) and
    /// [`examples`](Self::examples). `register_versioned` parsed dependencies
    /// out of its `metadata` ARGUMENT only, so a module that declares them on
    /// itself — which is how apcore-python (`getattr(module, "dependencies",
    /// [])`) and apcore-typescript read them — registered with an empty graph.
    /// `ReloadModule::topo_sort_modules` reads that accessor, so a
    /// `path_filter` reload of such modules sorted nothing and degenerated to
    /// alphabetical order.
    ///
    /// The default returns an empty Vec.
    fn dependencies(&self) -> Vec<crate::registry::registry::DependencyInfo> {
        Vec::new()
    }

    /// Optional preview hook — return a structured prediction of changes.
    ///
    /// Per protocol-spec §5.6, called by `Executor::validate()` *after* the standard
    /// validation pipeline has been processed. The default implementation
    /// returns `None` (= the module declines to predict).
    ///
    /// **Contract (RFC §"Optional preview() method"):**
    /// - MUST NOT have side effects.
    /// - Returning `None` (or omitting the method) means "no predicted changes".
    /// - Panicking is treated as advisory and does NOT fail validation; the
    ///   error is surfaced as a warning via the `module_preview` check.
    ///
    /// Cross-language alignment: matches apcore-python `Module.preview` and
    /// apcore-typescript `Module.preview?`.
    fn preview(
        &self,
        _inputs: &serde_json::Value,
        _ctx: Option<&Context<serde_json::Value>>,
    ) -> Option<PreviewResult> {
        None
    }

    /// Called after the module is registered.
    ///
    /// Returns `Err` to signal that the module failed to initialise; the
    /// registry rolls back the insertion so no half-initialised module remains
    /// registered. Aligns with `apcore-python Registry._invoke_on_load`.
    ///
    /// Default: no-op (`Ok(())`).
    fn on_load(&self) -> Result<(), ModuleError> {
        Ok(())
    }

    /// Called before the module is unregistered. Default: no-op.
    fn on_unload(&self) {}

    /// Called before hot-reload to capture state. Returns state dict for `on_resume()`.
    /// Default: returns None (no state to preserve).
    fn on_suspend(&self) -> Option<serde_json::Value> {
        None
    }

    /// Called after hot-reload to restore state from `on_suspend()`.
    /// Default: no-op.
    fn on_resume(&self, _state: serde_json::Value) {}

    /// Return a typed streaming handle for adapter/bridge code.
    ///
    /// Returns `None` for non-streaming modules (default). Streaming modules
    /// override this to return `Some(self)`.
    ///
    /// **Consistency invariant:** `Some(_)` here IFF `stream()` returns `Some(_)`.
    /// `Registry::register` enforces this by returning
    /// `Err(StreamingInterfaceMismatch)` when `annotations.streaming = true`
    /// but `as_streaming()` returns `None`.
    fn as_streaming(&self) -> Option<&dyn StreamingModule> {
        None
    }
}

/// Typed streaming interface for modules that support incremental output.
///
/// **Consistency invariant:** a module implementing `StreamingModule` MUST also
/// return `Some(_)` from `Module::stream()`. `Registry::register` enforces this
/// by returning `Err(StreamingInterfaceMismatch)` when `annotations.streaming =
/// true` but `as_streaming()` returns `None`.
pub trait StreamingModule: Module {
    fn stream_typed(
        &self,
        inputs: serde_json::Value,
        context: &crate::context::Context<serde_json::Value>,
    ) -> ChunkStream;
}

/// Metadata annotations attached to a module.
/// Describes behavioral characteristics of the module.
///
/// **Wire format (`PROTOCOL_SPEC` §4.4.1):** the `extra` field is serialized as a
/// nested JSON object under the key `"extra"`. Extension keys MUST NOT be
/// flattened to the annotations root. The custom `Deserialize` impl below
/// accepts both the canonical nested form and the legacy flattened form
/// (apcore-rust ≤ 0.17.1) for one MINOR backward-compat cycle.
#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)] // spec-defined annotation flags; consolidating into bitflags would break the public API
pub struct ModuleAnnotations {
    pub readonly: bool,
    pub destructive: bool,
    pub idempotent: bool,
    /// Whether **this module** requires human approval before execution.
    ///
    /// A module-level declaration, **not** the governance-effective value.
    /// `false` means this module asks for none; it does **not** mean none will
    /// be required. An ACL rule (PROTOCOL_SPEC §6.1.6), an `ExecutionPolicy`
    /// override or `gate_destructive` may each require one for a particular
    /// call (§6.9 rows 3-5), and §6.9 rule 3 unions them — neither source can
    /// cancel the other.
    ///
    /// The effective value is call-site dependent, because the ACL condition
    /// that decides it reads what the call carries (§6.1.7). Read it from
    /// [`Executor::validate`](crate::executor::Executor::validate) (§7.9.5),
    /// which reports the same verdict the approval gate will enforce. This
    /// annotation describes the **module**; the preflight describes the
    /// **call**.
    pub requires_approval: bool,
    pub open_world: bool,
    pub streaming: bool,
    pub cacheable: bool,
    pub cache_ttl: u64,
    pub cache_key_fields: Option<Vec<String>>,
    pub paginated: bool,
    pub pagination_style: String, // "cursor" | "offset" | "page"
    /// Whether this module is enumerated by `Registry::list()` /
    /// `Registry::iter()` / `Registry::module_ids()` by default.
    ///
    /// Per protocol-spec §2.5.1: defaults to `true`. Setting `discoverable: false`
    /// hides the module from default enumeration without unregistering it;
    /// callers that legitimately need to see every module ID can pass
    /// `include_hidden=true` to the relevant Registry method.
    ///
    /// Cross-language alignment: matches `ModuleAnnotations.discoverable`
    /// in apcore-python and apcore-typescript.
    pub discoverable: bool,
    /// Extension map for ecosystem package metadata.
    /// Serialized as a nested `"extra"` object per spec §4.4.1.
    pub extra: HashMap<String, serde_json::Value>,
    // Legacy fields moved to ModuleDescriptor:
    // name, version, author, description, tags, category, deprecated,
    // deprecated_message, since, hidden, examples, dependencies, metadata
}

impl<'de> Deserialize<'de> for ModuleAnnotations {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AnnotationsVisitor;

        impl<'de> Visitor<'de> for AnnotationsVisitor {
            type Value = ModuleAnnotations;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a ModuleAnnotations JSON object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<ModuleAnnotations, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut ann = ModuleAnnotations::default();
                let mut explicit_extra: Option<HashMap<String, serde_json::Value>> = None;
                let mut overflow: HashMap<String, serde_json::Value> = HashMap::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "readonly" => ann.readonly = map.next_value()?,
                        "destructive" => ann.destructive = map.next_value()?,
                        "idempotent" => ann.idempotent = map.next_value()?,
                        "requires_approval" => ann.requires_approval = map.next_value()?,
                        "open_world" => ann.open_world = map.next_value()?,
                        "streaming" => ann.streaming = map.next_value()?,
                        "cacheable" => ann.cacheable = map.next_value()?,
                        // D-115: a malformed value is TOLERATED and dropped, the
                        // rest survives, and it warns. Rejecting removed an
                        // ENTIRE module over one out-of-range integer, which is
                        // worse than strict — the declaration is recoverable and
                        // the module is not.
                        "cache_ttl" => {
                            let v: serde_json::Value = map.next_value()?;
                            ann.cache_ttl = v.as_u64().unwrap_or_else(|| {
                                tracing::warn!(
                                    value = %v,
                                    "cache_ttl must be a non-negative integer; clamping to 0 (D-115)"
                                );
                                0
                            });
                        }
                        "cache_key_fields" => ann.cache_key_fields = map.next_value()?,
                        "paginated" => ann.paginated = map.next_value()?,
                        "pagination_style" => ann.pagination_style = map.next_value()?,
                        "discoverable" => ann.discoverable = map.next_value()?,
                        "extra" => {
                            // Tolerate `null` extra → empty map.
                            let v: serde_json::Value = map.next_value()?;
                            explicit_extra = Some(match v {
                                serde_json::Value::Null => HashMap::new(),
                                serde_json::Value::Object(obj) => obj.into_iter().collect(),
                                // D-115: `extra` is declared an object; a
                                // scalar there is neither an object nor a reason
                                // to discard the module. Dropped with a warning
                                // rather than failing the whole
                                // ModuleDescriptor.
                                other => {
                                    tracing::warn!(
                                        value = %other,
                                        "ModuleAnnotations.extra must be an object; dropping it (D-115)"
                                    );
                                    HashMap::new()
                                }
                            });
                        }
                        _ => {
                            // Legacy flattened form (apcore-rust ≤ 0.17.1):
                            // unknown keys at the root are captured into overflow
                            // and later normalized into `extra`.
                            let v: serde_json::Value = map.next_value()?;
                            overflow.insert(key, v);
                        }
                    }
                }

                // §4.4.1 rule 7: nested explicit `extra` wins over legacy
                // top-level overflow. Build the merged map by writing overflow
                // first and then explicit_extra so the latter overwrites on
                // collision.
                let mut merged = overflow;
                if let Some(ex) = explicit_extra {
                    for (k, v) in ex {
                        merged.insert(k, v);
                    }
                }
                ann.extra = merged;
                Ok(ann)
            }
        }

        deserializer.deserialize_map(AnnotationsVisitor)
    }
}

impl Default for ModuleAnnotations {
    fn default() -> Self {
        Self {
            readonly: false,
            destructive: false,
            idempotent: false,
            requires_approval: false,
            open_world: true,
            streaming: false,
            cacheable: false,
            cache_ttl: 0,
            cache_key_fields: None,
            paginated: false,
            pagination_style: "cursor".to_string(),
            discoverable: true,
            extra: HashMap::new(),
        }
    }
}

impl ModuleAnnotations {
    /// Union the two governance sources a module's requirement can come from
    /// (D-96): the live instance's `annotations()` and the registry
    /// descriptor's.
    ///
    /// This SDK — unlike its peers, whose descriptors are DERIVED from the
    /// module — accepts a caller-supplied [`ModuleDescriptor`], documented on
    /// `Registry::register` as "loaded from a config file or discovered from an
    /// external source". Both single-source readings are therefore fail-OPEN:
    /// reading only the descriptor ignores what the module declares, reading
    /// only the module ignores what an operator declared in configuration. On
    /// an approval gate the failure direction is the whole argument —
    /// requiring an approval that was not strictly needed costs a prompt,
    /// skipping one that WAS needed is a bypass.
    ///
    /// Only the two governance flags are unioned. The rest of the annotations
    /// (caching, pagination, streaming, `extra`) describe the module's
    /// behaviour rather than its governance, and the live instance is
    /// authoritative for those; they are taken from `module` when it is
    /// present. Returns `None` only when neither source exists.
    ///
    /// **Every governance read MUST go through here.** It exists as one
    /// function precisely because the first fix for D-96 unioned the *gate* and
    /// left two other sites reading `module.annotations()` directly — the
    /// `ApprovalRequest` handed to the handler, and `Executor::validate`'s
    /// preflight — so the gate fired on one source and both the handler and the
    /// preflight described the call from another.
    ///
    /// [`ModuleDescriptor`]: crate::registry::registry::ModuleDescriptor
    #[must_use]
    pub fn governance_union(module: Option<&Self>, descriptor: Option<&Self>) -> Option<Self> {
        match (module, descriptor) {
            (Some(m), Some(d)) => {
                let mut merged = m.clone();
                merged.requires_approval = m.requires_approval || d.requires_approval;
                merged.destructive = m.destructive || d.destructive;
                Some(merged)
            }
            (Some(m), None) => Some(m.clone()),
            (None, Some(d)) => Some(d.clone()),
            (None, None) => None,
        }
    }
}

/// Default annotations instance — all fields at their spec defaults.
pub static DEFAULT_ANNOTATIONS: LazyLock<ModuleAnnotations> =
    LazyLock::new(ModuleAnnotations::default);

/// An example input/output pair for documentation.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModuleExample {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub inputs: serde_json::Value,
    pub output: serde_json::Value,
}

/// A single validation failure in the PROTOCOL_SPEC §4.14 detail format.
///
/// Cross-language parity with apcore-python `SchemaValidationErrorDetail`
/// (`schema/types.py`) and apcore-typescript `SchemaValidationErrorDetail`
/// (`schema/types.ts`): `path` and `message` are always present; `constraint`,
/// `expected`, and `actual` are optional and omitted from serialization when
/// absent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationErrorDetail {
    /// Dot/bracket path to the offending value (`""` / `<root>` for the root).
    pub path: String,
    /// Human-readable, one-line description of the failure.
    pub message: String,
    /// The JSON Schema keyword that was violated (e.g. `type`, `required`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint: Option<String>,
    /// The value the schema expected, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<serde_json::Value>,
    /// The value actually encountered, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<serde_json::Value>,
}

impl ValidationErrorDetail {
    /// Build a message-only detail (no path/constraint metadata). Used by
    /// custom `ModuleValidator` implementations and other call sites that only
    /// have a flat error string.
    #[must_use]
    pub fn message_only(message: impl Into<String>) -> Self {
        Self {
            path: String::new(),
            message: message.into(),
            constraint: None,
            expected: None,
            actual: None,
        }
    }
}

/// Result of validating a single aspect (used by `SchemaValidator` and `ModuleValidator`).
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ValidationResult {
    pub valid: bool,
    /// Per-failure detail objects (PROTOCOL_SPEC §4.14). Cross-language parity
    /// with apcore-python/typescript `SchemaValidationResult.errors`.
    #[serde(default)]
    pub errors: Vec<ValidationErrorDetail>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Result of a single preflight check (spec §12.8.4).
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PreflightCheckResult {
    /// Check name (e.g., "`module_id`", "`module_lookup`", "`call_chain`", "acl", "schema", "`module_preflight`").
    pub check: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Error details when `passed` is false; None when passed is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
    /// Non-fatal advisory messages (default: empty).
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Aggregated preflight results returned by `Executor::validate()` (spec §12.8.3).
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PreflightResult {
    /// True only if ALL checks passed.
    pub valid: bool,
    /// Ordered list of check results.
    pub checks: Vec<PreflightCheckResult>,
    /// The **governance-effective** approval requirement for this call
    /// (PROTOCOL_SPEC §7.9.5) — the union of the module annotation, an ACL rule
    /// carrying `approval`, an `ExecutionPolicy` override and
    /// `gate_destructive` (§6.9 rows 3-5).
    ///
    /// **Not the annotation alone.** Since spec v1.28.0 a module declaring
    /// `requires_approval: false` still reports `true` here when the ACL
    /// requires a human for the arguments *this* call carries. Reporting only
    /// the module's own declaration would tell a caller no approval is needed
    /// for a call the gate will stop; this union is by construction the same
    /// verdict the gate enforces.
    #[serde(default)]
    pub requires_approval: bool,
    /// Optional structured prediction of changes from the module's
    /// `preview()` hook (protocol-spec §5.6 / §12.8.5.1).
    ///
    /// - Empty when the module does not implement `preview()` or when
    ///   `preview()` returned `None`.
    /// - When `preview()` panics, this field is left empty and a
    ///   `module_preview` advisory check carries the warning, mirroring
    ///   `preflight()` semantics.
    ///
    /// Cross-language parity: matches `predicted_changes` /
    /// `predictedChanges` in apcore-python and apcore-typescript.
    #[serde(default)]
    pub predicted_changes: Vec<Change>,
}

/// Structured prediction of a single side-effect produced by executing a
/// module call.
///
/// Per protocol-spec §12.8.5.1. `action`, `target`, and `summary` are required; modules define
/// their own free-form taxonomy for `action` (e.g. "write", "delete", "send",
/// "charge", "publish") and `target` (e.g. "users.42", "smtp:user@example.com").
///
/// `before` / `after` are optional snapshots of prior / predicted state.
///
/// Extension fields with the `x-` prefix are permitted and round-trip via the
/// `extra` map. Any unknown key on the wire that does NOT start with `x-` is
/// rejected at deserialization time, matching the apcore-typescript TypeBox
/// schema (which sets `additionalProperties: false`).
///
/// Marked `#[non_exhaustive]` so a future spec revision can add a field without
/// a major version bump. That works by **removing struct-literal construction
/// from every crate but this one** — `..Default::default()` included, since it
/// is itself a struct expression (`error[E0639]`). Build one with
/// [`Change::new`], which sets the three required fields; assign `before`,
/// `after` and `extra` afterwards. See `api-surface-conventions.md` §9.1/§9.2.
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct Change {
    /// Free-form verb describing the kind of change (e.g. "write", "delete").
    pub action: String,
    /// Free-form identifier of what is changed (e.g. "users.42").
    pub target: String,
    /// Required, human-readable single-line summary of the change.
    pub summary: String,
    /// Optional. Snapshot of the prior state, when observable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    /// Optional. Predicted new state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    /// `x-*` extension fields (PROTOCOL_SPEC §4.4 / §4.6). Keys MUST start
    /// with `x-`; deserialization rejects any other unknown keys.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Change {
    /// Build a `Change` from its three required fields
    /// (`api-surface-conventions.md` §9.2 rule 4).
    ///
    /// `#[non_exhaustive]` removes struct-literal construction for downstream
    /// crates, and `Default::default()` alone would produce a value that is
    /// structurally valid and semantically empty — three empty strings where
    /// protocol-spec §12.8.5.1 requires an action, a target and a summary. Set `before`,
    /// `after` and `extra` on the returned value as needed.
    ///
    /// ```
    /// use apcore::Change;
    ///
    /// let mut change = Change::new("delete", "users.42", "Delete user 42");
    /// change.before = Some(serde_json::json!({ "active": true }));
    /// assert_eq!(change.action, "delete");
    /// ```
    #[must_use]
    pub fn new(
        action: impl Into<String>,
        target: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            action: action.into(),
            target: target.into(),
            summary: summary.into(),
            before: None,
            after: None,
            extra: HashMap::new(),
        }
    }
}

impl<'de> Deserialize<'de> for Change {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ChangeVisitor;

        impl<'de> Visitor<'de> for ChangeVisitor {
            type Value = Change;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a Change JSON object with action/target/summary and optional x-* extras",
                )
            }

            fn visit_map<M>(self, mut map: M) -> Result<Change, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut action: Option<String> = None;
                let mut target: Option<String> = None;
                let mut summary: Option<String> = None;
                let mut before: Option<serde_json::Value> = None;
                let mut after: Option<serde_json::Value> = None;
                let mut extra: HashMap<String, serde_json::Value> = HashMap::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "action" => action = Some(map.next_value()?),
                        "target" => target = Some(map.next_value()?),
                        "summary" => summary = Some(map.next_value()?),
                        "before" => before = Some(map.next_value()?),
                        "after" => after = Some(map.next_value()?),
                        other => {
                            // Per protocol-spec §12.8.5.1 any unknown key MUST
                            // start with `x-`. Reject anything else.
                            if !other.starts_with("x-") {
                                return Err(serde::de::Error::custom(format!(
                                    "Change has unknown key '{other}'; extension keys must start with 'x-'"
                                )));
                            }
                            let v: serde_json::Value = map.next_value()?;
                            extra.insert(other.to_string(), v);
                        }
                    }
                }

                Ok(Change {
                    action: action.ok_or_else(|| serde::de::Error::missing_field("action"))?,
                    target: target.ok_or_else(|| serde::de::Error::missing_field("target"))?,
                    summary: summary.ok_or_else(|| serde::de::Error::missing_field("summary"))?,
                    before,
                    after,
                    extra,
                })
            }
        }

        deserializer.deserialize_map(ChangeVisitor)
    }
}

/// Module's structured prediction of the changes that calling `execute()`
/// with the given inputs would produce. Returned by the optional
/// [`Module::preview`] hook; folded into [`PreflightResult::predicted_changes`]
/// by `Executor::validate()`.
///
/// Marked `#[non_exhaustive]` so a future spec revision can add a field without
/// a major version bump. That works by **removing struct-literal construction
/// from every crate but this one** — `..Default::default()` included, since it
/// is itself a struct expression (`error[E0639]`). Build one with
/// [`PreviewResult::new`]. See `api-surface-conventions.md` §9.1/§9.2.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PreviewResult {
    /// Ordered list of predicted side-effects.
    #[serde(default)]
    pub changes: Vec<Change>,
}

impl PreviewResult {
    /// Build a `PreviewResult` from its predicted changes
    /// (`api-surface-conventions.md` §9.2 rule 2 — a `#[non_exhaustive]` type
    /// must offer a construction path usable from a foreign crate).
    ///
    /// ```
    /// use apcore::{Change, PreviewResult};
    ///
    /// let preview = PreviewResult::new(vec![Change::new(
    ///     "send",
    ///     "smtp:user@example.com",
    ///     "Send the welcome email",
    /// )]);
    /// assert_eq!(preview.changes.len(), 1);
    /// ```
    #[must_use]
    pub fn new(changes: Vec<Change>) -> Self {
        Self { changes }
    }
}

/// Canonical name for the `acl` preflight check (PROTOCOL_SPEC §12.8.4 enum).
/// `Executor::validate` emits it for a denied call, and §12.8.5.1 makes it the
/// switch that withholds module-level introspection. Use this instead of the
/// literal string to avoid drift with other SDKs.
pub const ACL_CHECK_NAME: &str = "acl";

/// Canonical name for the `module_preflight` preflight check (PROTOCOL_SPEC
/// §12.8.4 enum). Use this instead of the literal string to avoid drift with
/// other SDKs.
pub const MODULE_PREFLIGHT_CHECK_NAME: &str = "module_preflight";

/// Canonical name for the `module_preview` preflight check (PROTOCOL_SPEC
/// §12.8.4 enum). Use this instead of the literal string to avoid drift with
/// other SDKs.
pub const MODULE_PREVIEW_CHECK_NAME: &str = "module_preview";

impl PreflightResult {
    /// Computed view: failed checks as typed refs (idiomatic Rust accessor).
    #[must_use]
    pub fn errors(&self) -> Vec<&PreflightCheckResult> {
        self.checks.iter().filter(|c| !c.passed).collect()
    }

    /// Computed view: failed checks serialized to JSON Value maps.
    ///
    /// Cross-language parity with apcore-python PreflightResult.errors
    /// (returns list[dict]) and apcore-typescript PreflightResult.errors
    /// (returns array of objects) — sync finding A-014.
    #[must_use]
    pub fn errors_as_json(&self) -> Vec<serde_json::Value> {
        self.checks
            .iter()
            .filter(|c| !c.passed)
            .filter_map(|c| serde_json::to_value(c).ok())
            .collect()
    }
}
