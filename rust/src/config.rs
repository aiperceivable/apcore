// APCore Protocol — Configuration
// Spec reference: Configuration loading, validation, and environment variable overrides (Algorithm A12)

use parking_lot::RwLock;
use serde::de::{DeserializeOwned, Error as DeError};
use serde::{Deserialize, Deserializer, Serialize};
use serde_yaml_ng as serde_yaml;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::errors::{ErrorCode, ModuleError};

/// Configuration mode detected from YAML content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfigMode {
    #[default]
    Legacy,
    Namespace,
}

/// Source for a `config.mount()` operation.
pub enum MountSource {
    Dict(serde_json::Value),
    File(PathBuf),
}

/// Default maximum nesting depth for env var key conversion.
pub const DEFAULT_MAX_DEPTH: usize = 5;

/// Environment variable naming the configuration file to load (§9.14 discovery).
///
/// apcore#88: this variable is an *argument to* [`Config::load`] — it selects
/// which document is read — and only happens to share the `APCORE_` prefix that
/// §9.2 turns into configuration overrides. Left in the override map its suffix
/// becomes the dot-path `config.file`, a key no schema declares (checked
/// against `conformance/fixtures/config_key_governance.json`), which then sits
/// inside the **declared** document that [`Config::validate`]'s §9.1
/// required-field check reads through [`Config::get_declared`].
/// `discover_config_file` consumes it; both env-override passes drop it.
const ENV_CONFIG_FILE: &str = "APCORE_CONFIG_FILE";

/// Environment variable key conversion strategy for a namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvStyle {
    /// Single `_` → `.` (section separator), double `__` → literal `_`.
    Nested,
    /// Suffix is lowercased as-is; no separator conversion.
    Flat,
    /// Match against defaults tree structure; fall back to Nested.
    #[default]
    Auto,
}

/// Registration info for a Config Bus namespace.
#[derive(Debug, Clone)]
pub struct NamespaceRegistration {
    pub name: String,
    /// Env var prefix. `None` = auto-derive from name (uppercase, `-` → `_`).
    pub env_prefix: Option<String>,
    pub defaults: Option<serde_json::Value>,
    pub schema: Option<serde_json::Value>,
    pub env_style: EnvStyle,
    pub max_depth: usize,
    /// Explicit bare env var → config key mapping (e.g. `"REDIS_URL" → "cache_url"`).
    pub env_map: Option<HashMap<String, String>>,
}

/// Summary of a registered namespace (returned by `registered_namespaces()`).
#[derive(Debug, Clone)]
pub struct NamespaceInfo {
    pub name: String,
    pub env_prefix: Option<String>,
    pub has_schema: bool,
}

static GLOBAL_NS_REGISTRY: OnceLock<RwLock<HashMap<String, NamespaceRegistration>>> =
    OnceLock::new();
/// Global bare env var → top-level config key mapping.
static GLOBAL_ENV_MAP: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
/// Tracks all claimed env var names (for conflict detection).
static ENV_MAP_CLAIMED: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();

fn global_ns_registry() -> &'static RwLock<HashMap<String, NamespaceRegistration>> {
    GLOBAL_NS_REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn global_env_map() -> &'static RwLock<HashMap<String, String>> {
    GLOBAL_ENV_MAP.get_or_init(|| RwLock::new(HashMap::new()))
}

fn env_map_claimed() -> &'static RwLock<HashMap<String, String>> {
    ENV_MAP_CLAIMED.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Top-level namespace names reserved by the apcore framework.
///
/// External callers **MUST NOT** register a namespace whose name appears in
/// this slice. Attempts to do so via [`Config::register_namespace`] fail with
/// `CONFIG_NAMESPACE_RESERVED`.
///
/// This is the single source of truth referenced by both
/// [`Config::register_namespace`] (enforcement) and
/// [`Config::reserved_namespaces`] (public query API). See
/// `PROTOCOL_SPEC` §9.5.1 (rules 3 and 4) and §9.9.5 for the normative
/// definition.
///
/// The slice is `'static` and inherently immutable from the caller's
/// perspective, satisfying §9.9.5 requirement (3).
pub const RESERVED_NAMESPACES: &[&str] = &["apcore", "_config"];

/// The canonical `observability` namespace name (`PROTOCOL_SPEC` §9.15.2).
///
/// Named rather than spelled inline because issue #33 turned it into a
/// *coupling*: it is simultaneously a typed [`Config`] field and a
/// `user_namespaces` key, and the four sites that reconcile the two
/// ([`Config::deserialize`], [`Config::get`], [`Config::namespace`],
/// `Serialize`) must agree on the spelling or the subkeys go missing again.
const OBSERVABILITY_NS: &str = "observability";

/// The canonical `executor` namespace name (`PROTOCOL_SPEC` §9.1).
///
/// Same coupling as [`OBSERVABILITY_NS`], reached from the other direction.
/// `executor` is a typed [`Config`] field, so `Config::deserialize` never
/// leaves a `user_namespaces` entry for it — but `set("executor.<key>", …)`
/// and `mount("executor", …)` both create one, because neither routes through
/// the typed struct for a key `ExecutorConfig` does not model. From that point
/// the namespace has two stores again, and the four sites that read it
/// ([`Config::get`], [`Config::namespace`], [`Config::bind`], `Serialize`)
/// must agree on the spelling or they disagree on the value.
const EXECUTOR_NS: &str = "executor";

/// The framework sections `Config` models as a typed field OUTSIDE the
/// `#[serde(flatten)] user_namespaces` bag, and whose raw object
/// [`Config::deserialize`] therefore has to retain by hand.
///
/// `PROTOCOL_SPEC` §9.14 (`reject_unknown_framework_keys`): with
/// `_config.strict` absent or false, a key inside a framework section that
/// `schemas/apcore-config.schema.json` does not declare **MUST** be retained
/// and readable through `get()`. Serde drops what the typed struct does not
/// model, silently, at parse time — so for every name in this list the raw
/// object is inserted into `user_namespaces` as well and the two stores are
/// reconciled by [`Config::typed_namespace_view`] (typed struct overlaid last).
///
/// The third typed field, `modules_path`, is deliberately absent: it is a
/// scalar, so it has no subkeys to lose and nothing to reconcile.
/// `test_config_unknown_framework_keys.rs` derives the `ConfigHelper` field
/// list from this source file and fails if a new typed field appears without
/// being classified into one of those two groups.
const TYPED_SECTIONS: &[&str] = &[EXECUTOR_NS, OBSERVABILITY_NS];

/// Every framework section `schemas/apcore-config.schema.json` declares,
/// paired with the immediate keys that schema declares for it.
///
/// Every section in that schema is `additionalProperties: false` (`extensions`
/// spells it `unevaluatedProperties: false` over a `oneOf`, which is the same
/// closedness reached through a branch), and `PROTOCOL_SPEC` §9.14 enforces
/// that closedness under `_config.strict: true`: a key here that the schema
/// does not declare **MUST** raise `CONFIG_INVALID`, and the error **MUST**
/// enumerate every offending key rather than failing on the first.
///
/// **This table is a projection of the canonical schemas, not a second source
/// of truth.** The SDK cannot read `apcore/schemas/` at runtime, so the
/// projection is transcribed here and
/// `framework_section_keys_match_the_canonical_schema` in
/// `tests/test_config_unknown_framework_keys.rs` re-derives it from
/// `schemas/apcore-config.schema.json` on every run and fails on any drift —
/// including a section added to the schema and not added here, which is the
/// drift a hand-maintained list would otherwise absorb silently.
///
/// `sys_modules` is the one section whose key list is a union of two canonical
/// files: `apcore-config.schema.json` declares only `enabled`, and the
/// remaining families (`health`, `usage`, `events`, …) are declared by
/// `schemas/sys-modules.schema.json`. That union is not this table's
/// invention — it is exactly how `conformance/fixtures/config_key_governance.json`
/// projects the canonical key surface ("the last namespaced under
/// `sys_modules.`"), and without it strict mode would reject
/// `sys_modules.health.enabled`, a key every SDK documents.
///
/// The top-level scalars `$schema` and `version` are absent because they are
/// not sections — §9.14 iterates the keys *inside* a section.
///
/// `#[doc(hidden)]`: public only so the conformance driver can diff it against
/// the canonical schema. Not part of the supported API surface.
#[doc(hidden)]
/// Every key the canonical schemas declare, as full dot-paths.
///
/// Generated from `schemas/*.schema.json` and pinned by
/// `conformance/fixtures/config_key_governance.json`.
///
/// `_config` is declared by the schema (`$defs/ConfigBusMeta`,
/// `additionalProperties: false`) and so is governed like any other section.
/// §9.10 skips it as a *namespace*, not as a section — and a typo in the strict
/// switch itself (`strcit: true`) is the single worst key to let through
/// silently, since it disables every other check the operator asked for.
///
/// This replaced a `section -> direct child names` table. Those schemas are
/// `additionalProperties: false` at EVERY level, not only at the section root,
/// so a one-level check left strict mode blind exactly where a typo is hardest
/// to spot: `observability.tracing.sampling_rat` passed it (its parent
/// `tracing` IS declared) while the canonical schema rejects it (sync finding
/// A-D-020).
pub const FRAMEWORK_CONFIG_KEYS: &[&str] = &[
    "$schema",
    "_config.allow_unknown",
    "_config.strict",
    "acl.audit.enabled",
    "acl.audit.include_denied",
    "acl.audit.log_level",
    "acl.default_effect",
    "acl.root",
    "bindings.dir",
    "bindings.pattern",
    "executor.default_timeout",
    "executor.global_timeout",
    "executor.max_call_depth",
    "executor.max_module_repeat",
    "extensions.auto_discover",
    "extensions.follow_symlinks",
    "extensions.ignore_patterns",
    "extensions.lazy_load",
    "extensions.max_depth",
    "extensions.namespace",
    "extensions.root",
    "extensions.roots",
    "id_map.auto_detect",
    "id_map.overrides",
    "logging.format",
    "logging.level",
    "middleware.disabled",
    "obs.redaction.regex_patterns",
    "obs.redaction.replacement",
    "obs.redaction.sensitive_keys",
    "observability.metrics.enabled",
    "observability.metrics.exporter",
    "observability.tracing.enabled",
    "observability.tracing.exporter",
    "observability.tracing.otlp_endpoint",
    "observability.tracing.sampling_rate",
    "observability.tracing.strategy",
    "pipeline.configure",
    "pipeline.remove",
    "pipeline.steps",
    "project.name",
    "project.version",
    "schema.max_ref_depth",
    "schema.root",
    "schema.strategy",
    "stream.max_merge_depth",
    "sys_modules.control.enabled",
    "sys_modules.control.overrides_path",
    "sys_modules.enabled",
    "sys_modules.error_history.max_entries_per_module",
    "sys_modules.error_history.max_total_entries",
    "sys_modules.events.enabled",
    "sys_modules.events.subscribers",
    "sys_modules.events.thresholds.error_rate",
    "sys_modules.events.thresholds.latency_p99_ms",
    "sys_modules.health.enabled",
    "sys_modules.manifest.enabled",
    "sys_modules.usage.bucketing_strategy",
    "sys_modules.usage.enabled",
    "sys_modules.usage.retention_hours",
    "validation.binding.description_max_length",
    "validation.binding.documentation_max_length",
    "validation.binding.tags_pattern",
    "validation.binding.version_require_semver",
    "validation.pipeline.step_name_max_length",
    "validation.pipeline.timeout_ms_max",
    "version",
];

/// Is `name` a framework section rather than a Config Bus namespace?
///
/// Used by the §9.10 strict unknown-namespace check. Rust flattens the
/// `apcore:` block's members to the top level of `user_namespaces` (so that
/// `get("acl.root")` works in both modes), which leaves framework sections
/// sitting alongside genuine namespaces in the same map. Without this filter,
/// `_config.strict: true` reported `unknown namespace 'acl'` for a document
/// whose only sin was declaring an `acl:` block — and `unknown namespace
/// 'project'` for the §9.1 required field.
/// True when `path` is itself a declared key, or a declared prefix of one.
fn is_declared_prefix(path: &str) -> bool {
    FRAMEWORK_CONFIG_KEYS
        .iter()
        .any(|k| *k == path || k.starts_with(&format!("{path}.")))
}

/// True when `path` names a declared CONTAINER (something is declared beneath
/// it), as opposed to a declared leaf.
fn is_declared_container(path: &str) -> bool {
    let with_dot = format!("{path}.");
    FRAMEWORK_CONFIG_KEYS
        .iter()
        .any(|k| k.starts_with(&with_dot))
}

fn is_framework_section(name: &str) -> bool {
    FRAMEWORK_CONFIG_KEYS
        .iter()
        .any(|path| path.split('.').next() == Some(name) && path.contains('.'))
}

/// A canonical default value. Const-constructible so [`CONFIG_DEFAULTS`] can
/// stay a plain `const` table rather than a lazily-built map.
#[derive(Debug, Clone, Copy)]
enum DefaultValue {
    Str(&'static str),
    Bool(bool),
    Int(i64),
}

impl DefaultValue {
    fn to_json(self) -> serde_json::Value {
        match self {
            Self::Str(s) => serde_json::Value::String(s.to_string()),
            Self::Bool(b) => serde_json::Value::Bool(b),
            Self::Int(i) => serde_json::Value::Number(i.into()),
        }
    }
}

/// Canonical default values for config keys that resolve to a default when
/// omitted, rather than being hard-required.
///
/// This is a verbatim transcription of `apcore/schemas/defaults.schema.json`
/// (the cross-SDK single source of truth), plus the two keys apcore-python
/// carries in `_DEFAULTS` that the schema does not model (`version` and
/// `project.name`). It mirrors apcore-python's `_DEFAULTS` (config.py) and
/// apcore-typescript's `DEFAULTS` (config-defaults.ts).
///
/// Consulted by [`Config::get`] **after** the typed-field and
/// `user_namespaces` lookups, so a value present in the loaded YAML always
/// wins. Before this table existed, a legacy YAML omitting these keys made
/// `Config::get` return `None` where both peers returned the canonical
/// default.
///
/// `executor.*` and `observability.*` are absent by design: they are typed
/// struct fields whose `Default` impls already carry the same values, and
/// `get_typed_field` resolves them first.
///
/// NOTE on `sys_modules.enabled`: `defaults.schema.json` and apcore-python
/// `_DEFAULTS` both declare `false`. The `sys_modules` *namespace*
/// registration (PROTOCOL_SPEC §9.15.3) declares `enabled: true` — that is the
/// namespace-bus default surfaced by [`Config::namespace`], a different
/// lookup. Both peers have the same split; keep it.
/// Every configuration key `validate_key_constraint` carries a constraint for.
///
/// `#[doc(hidden)]`: this exists so `config_key_governance.json` can assert the
/// constraint table stays inside the key set the canonical schemas declare. It
/// is not part of the public API. The in-crate test
/// `constrained_config_keys_matches_the_match_arms` keeps it honest — a key
/// added to the match without being added here fails there.
#[doc(hidden)]
pub const CONSTRAINED_CONFIG_KEYS: &[&str] = &[
    "acl.default_effect",
    "observability.tracing.sampling_rate",
    "observability.tracing.strategy",
    "observability.tracing.exporter",
    "observability.tracing.otlp_endpoint",
    "sys_modules.events.thresholds.error_rate",
    "sys_modules.events.thresholds.latency_p99_ms",
    "extensions.max_depth",
    "executor.default_timeout",
    "executor.global_timeout",
    "executor.max_call_depth",
    "executor.max_module_repeat",
    "sys_modules.error_history.max_entries_per_module",
    "sys_modules.error_history.max_total_entries",
];

/// Every configuration key the canonical default table declares.
///
/// `#[doc(hidden)]`: exposed only so `config_key_governance.json` can compare
/// this SDK's default table against `schemas/defaults.schema.json`. Use
/// [`Config::default_for`] to resolve a value.
#[doc(hidden)]
#[must_use]
pub fn config_default_keys() -> Vec<&'static str> {
    CONFIG_DEFAULTS.iter().map(|(k, _)| *k).collect()
}

/// The closed set of **path-typed** configuration keys — those whose value is a
/// filesystem path (PROTOCOL_SPEC §9.2.1).
///
/// Declared canonically by `"x-apcore-path": true` in
/// `schemas/apcore-config.schema.json` and the namespace schema it references,
/// `schemas/sys-modules.schema.json`; this slice is that projection.
///
/// It exists for consumers outside this SDK. Anything forwarding apcore
/// configuration across a process boundary — a CLI spawning a worker, a
/// supervisor building a container environment — has to know which `APCORE_*`
/// variables carry paths, because a relative value silently re-roots wherever
/// the working directory differs. Without a published set, each such consumer
/// builds its own and drifts from the others.
///
/// One exclusion is deliberate, being the mistake an implementer would
/// otherwise make: `bindings.pattern` is a glob matched against filenames
/// *within* `bindings.dir`, never resolved as a path itself. `id_map.overrides`
/// is the path of an ID map file, and `sys_modules.control.overrides_path` the
/// path of the runtime overrides file (D-138).
///
/// `extensions.roots` is list-valued and every element carries a path, in both
/// the bare-string and the `{ root, namespace }` form — hence the element key
/// `extensions.roots[]`.
///
/// This set says *which* keys carry paths. It says nothing about what a relative
/// value resolves against; that base is unspecified as of spec v1.34.0.
const PATH_TYPED_CONFIG_KEYS: &[&str] = &[
    "acl.root",
    "bindings.dir",
    "extensions.root",
    "extensions.roots[]",
    "id_map.overrides",
    "schema.root",
    "sys_modules.control.overrides_path",
];

/// PROTOCOL_SPEC §9.2.4 — the declared configuration keys that reach no
/// consumer in any implementation (aiperceivable/apcore#118). Order is the
/// order they are reported in, so two SDKs name them the same way.
///
/// Ten when the window opened in spec v1.39.0; seven since v1.44.0, which gave
/// `observability.tracing.enabled` / `.sampling_rate` / `.exporter` consumers
/// (§10.1.1) and cancelled their withdrawal. A key that has left the table MUST
/// NOT warn — §9.2.4 requirement 1.
///
/// `acl.default_effect` joined in v1.47.0 as §9.1.3's first application: it is
/// read from the ACL FILE, and the `apcore.yaml` twin reaches nothing. Ordered
/// first because §9.2.4's table lists it first.
const DEPRECATED_INERT_KEYS: &[&str] = &[
    "acl.default_effect",
    "observability.metrics.enabled",
    "observability.metrics.exporter",
    "logging.level",
    "logging.format",
    "acl.audit.enabled",
    "acl.audit.include_denied",
    "acl.audit.log_level",
    // D-137: names built-in middleware that does not exist; use `remove()` or
    // `pipeline.remove`.
    "middleware.disabled",
    // D-150: discovery runs when `discover()` is called.
    "extensions.auto_discover",
];

const CONFIG_DEFAULTS: &[(&str, DefaultValue)] = &[
    // No `version` entry, and no `project.name`: `defaults.schema.json`
    // declares neither, which is exactly what makes them §9.1's two required
    // fields. This table used to carry `version: "0.16.0"` — half of the
    // invented `version`/`project.name` pair every SDK once shipped — so
    // `get("version")` answered a stale frozen spec version here while
    // apcore-python and apcore-typescript answered nothing (A-D-021). The
    // required-field check was never affected: it reads `get_declared`.
    ("extensions.root", DefaultValue::Str("./extensions")),
    ("extensions.auto_discover", DefaultValue::Bool(true)),
    ("extensions.max_depth", DefaultValue::Int(8)),
    ("extensions.follow_symlinks", DefaultValue::Bool(false)),
    ("schema.root", DefaultValue::Str("./schemas")),
    ("schema.strategy", DefaultValue::Str("yaml_first")),
    ("schema.max_ref_depth", DefaultValue::Int(32)),
    // D-64 (Recommendation A): `acl.root` defaults rather than being required,
    // so a config that omits it stays VALID and `ACL::discover` anchors at the
    // conventional directory.
    ("acl.root", DefaultValue::Str("./acl")),
    ("acl.default_effect", DefaultValue::Str("deny")),
    ("sys_modules.enabled", DefaultValue::Bool(false)),
    ("stream.max_merge_depth", DefaultValue::Int(32)),
    // Spec v1.36.0 added the `bindings` section to `defaults.schema.json`.
    // Until then the section was declared in §9.1.1 and in
    // `apcore-config.schema.json` but absent from the file that IS the
    // canonical default tier, so no SDK could carry the default and all three
    // hardcoded `./bindings` at their binding loader instead. With these two
    // entries `Config::get("bindings.dir")` answers the documented value, which
    // is what §5.12.6 clause 1's "default `./bindings`" tier resolves through.
    ("bindings.dir", DefaultValue::Str("./bindings")),
    // PROTOCOL_SPEC §9.1.2 (spec v1.38.0, apcore#118). The six `validation.*`
    // keys are UNCONSTRAINED by default — apcore does not impose limits on the
    // content its users author, it offers them. Five of the six are
    // unconstrained AS null, and null is the absence of a default, so only this
    // one carries a canonical value: `defaults.schema.json` declares exactly
    // this entry and `config_key_governance.json` pins this table to that file.
    (
        "validation.binding.version_require_semver",
        DefaultValue::Bool(false),
    ),
    ("bindings.pattern", DefaultValue::Str("*.binding.yaml")),
];

/// Executor namespace configuration (`PROTOCOL_SPEC` §9.1).
///
/// All timeouts are in milliseconds.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ExecutorConfig {
    /// Per-module execution timeout (milliseconds). 0 means no per-module timeout.
    pub default_timeout: u64,
    /// Whole-call-chain deadline (milliseconds). 0 means no global deadline.
    pub global_timeout: u64,
    /// Maximum call chain depth before `MODULE_CALL_DEPTH_EXCEEDED` is raised.
    pub max_call_depth: u32,
    /// Maximum repeat count for the same module within a single call chain.
    pub max_module_repeat: u32,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            default_timeout: 30_000,
            global_timeout: 60_000,
            max_call_depth: 32,
            max_module_repeat: 3,
        }
    }
}

/// Observability namespace configuration (`PROTOCOL_SPEC` §9.1).
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ObservabilityConfig {
    pub tracing: TracingConfig,
    pub metrics: MetricsConfig,
}

/// Tracing sub-config of `ObservabilityConfig` (`PROTOCOL_SPEC` §9.1).
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct TracingConfig {
    pub enabled: bool,
    /// Trace sampling rate in `[0.0, 1.0]`. Default `1.0`.
    ///
    /// Modelled as a real field (rather than being swallowed by the untyped
    /// `user_namespaces` bag) so the `observability.tracing.sampling_rate`
    /// constraint in [`Config::validate_key_constraint`] is reachable. While
    /// `observability` was a typed struct with only `enabled`, an
    /// out-of-range `sampling_rate: 5.0` was silently dropped at
    /// deserialization — accepted by Rust where apcore-python and
    /// apcore-typescript both reject it with `CONFIG_INVALID` — and a
    /// legitimate `0.1` never survived a `data()` round-trip.
    pub sampling_rate: f64,
    /// Trace exporter: `"stdout" | "otlp" | "jaeger"`. Default `"stdout"`.
    ///
    /// PROTOCOL_SPEC §10.1.1 requirement 2. §9.15.2 used to document
    /// `in_memory` here instead of `jaeger`, disagreeing with
    /// `schemas/apcore-config.schema.json`; v1.44.0 makes the schema canonical
    /// and forbids the in-memory exporter as a configuration value, since a
    /// caller selecting it by name has no standardised way to read the spans
    /// it holds.
    pub exporter: String,
    /// How the sampling decision is made: `"full" | "proportional" |
    /// "error_first" | "off"`. Default `"full"` (PROTOCOL_SPEC §10.7).
    ///
    /// Declared by §9.15.2's namespace registration since that section was
    /// written and absent from the schema until v1.44.0, so `_config.strict`
    /// rejected it as an unknown key while the specification documented its
    /// default. Modelled as a real field for the same reason `sampling_rate`
    /// is: otherwise the constraint in [`Config::validate_key_constraint`] is
    /// unreachable and the value does not survive a `data()` round-trip.
    pub strategy: String,
    /// OTLP collector endpoint, or `None` for the implementation default.
    ///
    /// PROTOCOL_SPEC §10.1.1 requirement 3. Read only when `exporter` is
    /// `"otlp"`; setting it alongside any other exporter is a load-time
    /// `CONFIG_INVALID` error rather than a silent no-op.
    pub otlp_endpoint: Option<String>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sampling_rate: 1.0,
            strategy: "full".to_string(),
            otlp_endpoint: None,
            exporter: "stdout".to_string(),
        }
    }
}

/// Metrics sub-config of `ObservabilityConfig` (`PROTOCOL_SPEC` §9.1).
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct MetricsConfig {
    pub enabled: bool,
}

/// Top-level apcore configuration (`PROTOCOL_SPEC` §9.1).
///
/// Canonical wire format is a nested JSON/YAML object with `executor`,
/// `observability`, and any user-defined namespaces as siblings:
///
/// ```yaml
/// modules_path: ./modules
/// executor:
///   max_call_depth: 32
///   default_timeout: 30000
/// observability:
///   tracing:
///     enabled: true
/// my_vendor:
///   custom_setting: foo
/// ```
///
/// **v0.18.0 BREAKING CHANGE.** Prior versions accepted root-level
/// `max_call_depth`, `default_timeout_ms`, etc. The custom `Deserialize` impl
/// now rejects these with a hard error naming each field's canonical location.
/// **Note (sync finding A-D-016).** Apcore-python and apcore-typescript
/// register the built-in `observability`, `sys_modules` and `obs` namespaces at
/// module-load time, so every code path observes them. Rust has no cheap
/// equivalent (no implicit module-init hook without the `ctor` crate), so
/// the SDK uses an idempotent `OnceLock`-guarded `init_builtin_namespaces()`
/// that runs from the user-facing entry points: `Config::from_yaml_file`,
/// `Config::from_json_file`, `Config::from_defaults`, and
/// `Config::load_or_discover`.
///
/// `Config::default()` (`#[derive(Default)]`) is the low-level constructor
/// and intentionally does NOT trigger initialization — it is meant for
/// internal/test code that wants a bare struct without touching the
/// process-global namespace registry. **User code should call
/// `Config::from_defaults()` for canonical defaults**, which mirrors Python
/// and TypeScript behavior. Calling either `from_yaml_file`/`from_json_file`
/// also initializes the built-ins.
///
/// This is a documented Rust-specific divergence rather than a behavioral
/// bug; cross-language conformance fixtures rely on `from_yaml_file` /
/// `from_defaults` and therefore see consistent behavior.
/// ## `observability` is stored twice, on purpose (issue #33)
///
/// The four leaves modelled by [`ObservabilityConfig`]
/// (`tracing.enabled`, `tracing.sampling_rate`, `tracing.exporter`,
/// `metrics.enabled`) live in the typed struct. **The whole raw
/// `observability` object from the file also lives in `user_namespaces`**, so
/// the subkeys the typed struct does not model — `redaction.*`, `logging.*`,
/// `error_history.*`, `platform_notify.*`, `tracing.strategy`,
/// `tracing.otlp_endpoint`, `metrics.exporter`, all of them declared
/// configurable by the §9.15.2 namespace registration — survive the load
/// instead of being discarded by [`Config::deserialize`].
///
/// One rule resolves the overlap everywhere: **the typed struct wins for its
/// four leaves, the `user_namespaces` tree owns everything else.** It is
/// applied in exactly one place, [`Config::observability_view`], which
/// [`Config::get`], [`Config::namespace`], [`Config::bind`] and
/// `Serialize`/[`Config::data`] all read through — so `set()`, an env
/// override, and the file can never disagree about which value is live.
///
/// ## `executor` has the same two stores, reached differently (issue #34)
///
/// [`ExecutorConfig`] models *every* key `$defs/ExecutorConfig` in
/// `schemas/apcore-config.schema.json` declares, and that schema is
/// `additionalProperties: false`, so — unlike `observability` — no
/// *spec-declared* `executor` subkey is lost at load.
///
/// An **un**declared one was, until `PROTOCOL_SPEC` §9.14 made retention
/// normative: `executor: {zz_vendor_knob: …}` reached `ExecutorConfig`, which
/// does not model it, and serde discarded it before any accessor could see it.
/// §9.14 requires the opposite under the default (`_config.strict` absent or
/// false) — the key is kept and readable through `get()` — so
/// `Config::deserialize` now retains the raw `executor` object in
/// `user_namespaces` exactly as it does for `observability`. See
/// [`TYPED_SECTIONS`].
///
/// A second store also appears at runtime: `set("executor.<unmodelled>", …)`
/// falls past `set_typed_field` into `user_namespaces`, and
/// `mount("executor", …)` writes there directly. Once it exists, every reader
/// that consulted only one of the two stores was wrong — `Serialize` wrote the
/// typed struct and then let the flattened bag overwrite it, so a single
/// `set("executor.vendor_knob", …)` erased all four typed leaves from the §9.1
/// wire form. [`Config::executor_view`] applies the same rule as
/// `observability_view` (raw tree as base, typed struct overlaid last) and the
/// same four readers go through it.
///
/// The one deliberate asymmetry: `observability_view` is guarded on the raw
/// entry in [`Config::get`] so an undeclared block still reports `None`;
/// `executor_view` is not. `ExecutorConfig` has no optional leaf — every field
/// always carries a value — so `get("executor.max_call_depth")` already
/// answers `Some(32)` for a document that declares nothing. A container fetch
/// answering `None` while its own leaf answers `Some` is the contradiction
/// this issue is about, one level up.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub modules_path: Option<PathBuf>,
    pub executor: ExecutorConfig,
    pub observability: ObservabilityConfig,
    /// User-defined and vendor namespaces. Captures any top-level key not
    /// matching a canonical namespace above, plus the raw object of every
    /// [`TYPED_SECTIONS`] entry — `observability` and `executor` — so the
    /// subkeys their typed structs do not model survive the load
    /// (`PROTOCOL_SPEC` §9.14; see the struct-level note). `set`/`mount` can
    /// create the same entries at runtime. Per spec §9.1, custom namespace
    /// names should follow `[a-z][a-z0-9-]*`.
    pub user_namespaces: HashMap<String, serde_json::Value>,
    pub yaml_path: Option<PathBuf>,
    pub mode: ConfigMode,
    /// Atomic-style generation counter to detect concurrent modifications.
    /// Incremented on every mutation (set, mount, reload). Aligned with D-20.
    pub generation: u64,
    /// Namespaces attached via [`Config::mount`], in mount order, retained so
    /// [`Config::reload`] can replay them onto the freshly-read file.
    ///
    /// PROTOCOL_SPEC §9.11 requires mounts to survive a reload. Without this
    /// record `mount("my-plugin", …)` followed by `reload()` dropped the
    /// mounted subtree entirely (`my-plugin.timeout` → `None`), where
    /// apcore-python and apcore-typescript still resolved it.
    ///
    /// Public only because `Config` is constructed with struct-update syntax
    /// (`..Config::default()`) across the workspace; treat it as internal and
    /// mutate it through [`Config::mount`].
    pub mounts: Vec<(String, serde_json::Value)>,
}

/// Hand-written to keep the `observability` wire object consistent with
/// [`Config::get`] (issue #33).
///
/// `#[derive(Serialize)]` emitted the typed `observability` field and then the
/// `#[serde(flatten)] user_namespaces` bag into the same map. Once
/// `user_namespaces` carries an `observability` entry the second write wins,
/// so the typed leaves vanished from [`Config::data`] entirely: a config with
/// `tracing.sampling_rate: 0.1` and `logging.enabled: false` serialized as
/// `observability: {logging: {enabled: false}}` — the sampling rate, and the
/// canonical defaults for every other typed leaf, silently gone from the §9.1
/// wire form. Emitting [`Config::observability_view`] once, and skipping the
/// bag's own `observability` entry, makes `data()` report exactly what `get()`
/// resolves.
///
/// Issue #34: `executor` had the identical clobber, one `set()` away. Nothing
/// puts an `executor` entry in the bag at load time, so the two writes never
/// collided for a file-loaded config — but `set("executor.vendor_knob", "x")`
/// creates one, and from then on `data()["executor"]` was
/// `{"vendor_knob": "x"}`: `max_call_depth`, `max_module_repeat`,
/// `default_timeout` and `global_timeout` all gone from the §9.1 wire form,
/// including values the operator's file had set. Emitting
/// [`Config::executor_view`] and skipping the bag's `executor` entry fixes it
/// the same way.
impl Serialize for Config {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut len = 2 + self.user_namespaces.len();
        if self.modules_path.is_some() {
            len += 1;
        }
        if self.user_namespaces.contains_key(OBSERVABILITY_NS) {
            len -= 1;
        }
        if self.user_namespaces.contains_key(EXECUTOR_NS) {
            len -= 1;
        }

        let mut map = serializer.serialize_map(Some(len))?;
        if let Some(path) = self.modules_path.as_ref() {
            map.serialize_entry("modules_path", path)?;
        }
        map.serialize_entry(EXECUTOR_NS, &self.executor_view())?;
        map.serialize_entry(OBSERVABILITY_NS, &self.observability_view())?;
        for (key, value) in &self.user_namespaces {
            if key == OBSERVABILITY_NS || key == EXECUTOR_NS {
                continue;
            }
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Split `key` at a canonical namespace prefix, returning the remainder.
///
/// `Some("")` for the bare namespace name (a container fetch), `Some("rest")`
/// for `<ns>.rest`, `None` when `ns` is merely a string prefix of a different
/// top-level key (`observability_extra` must not route here).
fn strip_namespace<'a>(key: &'a str, ns: &str) -> Option<&'a str> {
    key.strip_prefix(ns)
        .filter(|rest| rest.is_empty() || rest.starts_with('.'))
        .map(|rest| rest.trim_start_matches('.'))
}

/// Walk a dot-path remainder through an already-reconciled namespace view.
///
/// An empty remainder yields the whole view, which is what makes a container
/// fetch (`get("executor")`) return the namespace object rather than `None`.
fn walk_view(view: serde_json::Value, rest: &str) -> Option<serde_json::Value> {
    let mut current = view;
    for part in rest.split('.').filter(|part| !part.is_empty()) {
        current = current.get(part)?.clone();
    }
    Some(current)
}

/// Legacy v0.17.x root-level field names that are no longer accepted in v0.18.0.
const LEGACY_ROOT_FIELDS: &[(&str, &str)] = &[
    ("max_call_depth", "executor.max_call_depth"),
    ("max_module_repeat", "executor.max_module_repeat"),
    ("default_timeout_ms", "executor.default_timeout"),
    ("global_timeout_ms", "executor.global_timeout"),
    ("enable_tracing", "observability.tracing.enabled"),
    ("enable_metrics", "observability.metrics.enabled"),
];

// Helper struct for two-pass deserialization of Config.
// Defined outside the fn body to satisfy items_after_statements lint.
#[derive(Deserialize)]
struct ConfigHelper {
    #[serde(default)]
    modules_path: Option<PathBuf>,
    #[serde(default)]
    executor: ExecutorConfig,
    #[serde(default)]
    observability: ObservabilityConfig,
    #[serde(flatten, default)]
    user_namespaces: HashMap<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Two-pass: first parse the wire form into a generic JSON object,
        // detect any v0.17.x legacy root-level fields, then materialize the
        // canonical struct via a helper that mirrors the serialized shape.
        let raw = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;

        let mut violations: Vec<String> = Vec::new();
        for (legacy, canonical) in LEGACY_ROOT_FIELDS {
            if raw.contains_key(*legacy) {
                violations.push(format!("'{legacy}' → '{canonical}'"));
            }
        }
        if !violations.is_empty() {
            return Err(D::Error::custom(format!(
                "apcore v0.18.0 changed Config layout: root-level fields {} are no longer accepted. \
                 Move them to their canonical nested namespace. \
                 See the apcore CHANGELOG entry for 0.18.0.",
                violations.join(", ")
            )));
        }

        let mut core_data = raw.clone();
        let mut mode = ConfigMode::Legacy;

        // §9.6: If "apcore" key is present, it's namespace mode.
        if let Some(apcore_val) = raw.get("apcore") {
            if let Some(apcore_obj) = apcore_val.as_object() {
                mode = ConfigMode::Namespace;
                // Merge apcore-namespace fields into the top-level core_data
                // so ConfigHelper can find them.
                for (k, v) in apcore_obj {
                    core_data.insert(k.clone(), v.clone());
                }
            }
        }

        // Issue #33: `observability` is a TYPED field on `ConfigHelper`, so
        // serde hands the whole object to `ObservabilityConfig` — which models
        // only `tracing.{enabled,sampling_rate,exporter}` and `metrics.enabled`
        // — and drops every other subkey on the floor. `redaction.*`,
        // `logging.*`, `error_history.*`, `platform_notify.*`,
        // `tracing.strategy`, `tracing.otlp_endpoint` and `metrics.exporter`
        // are all declared configurable by the §9.15.2 namespace registration
        // and were all discarded here, before any accessor could see them:
        // an operator's `logging.enabled: false` did not merely fail to apply,
        // `namespace("observability")` confidently reported the registered
        // default `true` back at them.
        //
        // Keeping the raw object in `user_namespaces` as well is what makes
        // those subkeys resolvable. It does NOT change the four typed leaves:
        // `get_direct` consults `get_typed_field` first, and every other
        // observability reader goes through `observability_view`, which
        // overlays the typed struct last. Read from `core_data` rather than
        // `raw` so namespace-mode files that nest the block under `apcore:`
        // are covered too.
        //
        // §9.14 generalizes #33's fix to EVERY typed section (see
        // [`TYPED_SECTIONS`]): with `_config.strict` absent or false, a key
        // inside a framework section that `apcore-config.schema.json` does not
        // declare MUST survive the load and be readable through `get()`.
        // `ExecutorConfig` models every key that schema declares, so no
        // *declared* `executor` subkey was ever lost — but an undeclared one
        // was, silently, at parse time, which is the defect §9.14 names: "the
        // operator wrote it and it vanished" is indistinguishable from "the
        // operator never wrote it".
        let raw_typed_sections: Vec<(&str, serde_json::Map<String, serde_json::Value>)> =
            TYPED_SECTIONS
                .iter()
                .filter_map(|name| {
                    core_data
                        .get(*name)
                        .and_then(serde_json::Value::as_object)
                        .map(|obj| (*name, obj.clone()))
                })
                .collect();

        let raw_modules_path = core_data.get("modules_path").cloned();

        let helper: ConfigHelper = serde_json::from_value(serde_json::Value::Object(core_data))
            .map_err(D::Error::custom)?;

        let mut user_namespaces = helper.user_namespaces;
        for (name, raw) in raw_typed_sections {
            user_namespaces.insert(name.to_string(), serde_json::Value::Object(raw));
        }
        // `modules_path` is the one typed field that is a SCALAR at the top
        // level rather than a section, so `TYPED_SECTIONS` above cannot carry
        // it: `core_data.get("modules_path")` is a string, not an object. Serde
        // moves it into `helper.modules_path` and leaves no as-written record,
        // which made `get_declared("modules_path")` answer `None` for a legacy
        // file that declares it — while a namespace-mode file answered `Some`,
        // because the `apcore:` members are lifted into `user_namespaces`
        // wholesale. Retaining it here is what makes the two modes agree
        // (aiperceivable/apcore#119).
        if let Some(raw_modules_path) = raw_modules_path {
            user_namespaces.insert("modules_path".to_string(), raw_modules_path);
        }

        // §9.6.3's `allow_unknown` row, both halves of which were inert
        // (apcore#118, decision D-69).
        if mode == ConfigMode::Namespace {
            // From `raw`, not from `user_namespaces`: the latter is a flattened
            // bag that in namespace mode also holds the `apcore:` members
            // lifted into `core_data` (`version`, `project`, …). Those are
            // framework keys, not namespaces, and a first draft that filtered
            // the bag reported `version` as an unregistered namespace.
            apply_allow_unknown(&raw, &mut user_namespaces);
        }

        Ok(Config {
            modules_path: helper.modules_path,
            executor: helper.executor,
            observability: helper.observability,
            user_namespaces,
            yaml_path: None,
            mode,
            generation: 0,
            mounts: Vec::new(),
        })
    }
}

/// PROTOCOL_SPEC §9.6.3's `allow_unknown` row (apcore#118, decision D-69).
///
/// Both halves of that row were inert. `allow_unknown: false` is documented as
/// "silently ignored (not stored)" and the namespace was stored anyway, so
/// `get()` answered for it; `allow_unknown: true` is documented as "stored,
/// accessible, **WARN logged**" and no implementation logged anything. Fixing
/// one without the other would leave the row half true, and both live here.
///
/// **Namespace mode only, by construction.** §9.6.3 is about *namespaces*, and
/// a legacy document has none — its root IS the `apcore` namespace, so an
/// unrecognised top-level key there is a framework key governed by §9.14's walk
/// under `strict`, not by this field. `strict`'s own clause (b) says it
/// "applies in legacy mode too", which is the specification saying clause (a)
/// does not.
///
/// Only a deployment that explicitly writes `allow_unknown: false` changes
/// behaviour, and what changes is that it finally gets the published contract
/// instead of a no-op — but the change is real: a `get()` that returned a value
/// now returns `None`.
fn apply_allow_unknown(
    raw: &serde_json::Map<String, serde_json::Value>,
    user_namespaces: &mut HashMap<String, serde_json::Value>,
) {
    // An ABSENT `_config` is the default pair `strict: false, allow_unknown:
    // true`, not an exemption: §9.6.3's matrix describes the defaults, so a
    // document that declares an unregistered namespace and no `_config` at all
    // is the row that warns. This is not the blanket warning §9.2.2 rejects —
    // it fires on a condition specific to the document (there IS an
    // unregistered namespace), never on every configuration ever loaded.
    let meta = raw.get("_config").and_then(|v| v.as_object());
    let strict = meta
        .and_then(|m| m.get("strict"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if strict {
        // `strict` already rejects an unknown namespace outright (clause a), so
        // this field is "only relevant when strict: false" per §9.6.3's own
        // comment.
        return;
    }
    let allow = meta
        .and_then(|m| m.get("allow_unknown"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    let registered = global_ns_registry().read();
    let mut unknown: Vec<String> = raw
        .keys()
        .filter(|k| {
            k.as_str() != "apcore"
                && k.as_str() != "_config"
                && !registered.contains_key(k.as_str())
        })
        .cloned()
        .collect();
    drop(registered);
    unknown.sort_unstable();
    if unknown.is_empty() {
        return;
    }

    if allow {
        tracing::warn!(
            namespaces = %unknown.join(", "),
            "Configuration declares namespace(s) that no package has registered. They are \
             stored and readable through get(), and NOT validated against any schema \
             (PROTOCOL_SPEC §9.6.3). Set _config.allow_unknown: false to have them dropped \
             instead, or _config.strict: true to reject them."
        );
        return;
    }

    tracing::info!(
        namespaces = %unknown.join(", "),
        "Dropping unregistered namespace(s) per _config.allow_unknown: false (PROTOCOL_SPEC \
         §9.6.3). Their keys are not stored and get() will not answer for them."
    );
    for name in unknown {
        user_namespaces.remove(&name);
    }
}

impl Config {
    /// Load config from a JSON file, apply env overrides, and validate.
    pub fn from_json_file(path: &std::path::Path) -> Result<Self, ModuleError> {
        let file = std::fs::File::open(path).map_err(|e| {
            ModuleError::new(
                ErrorCode::ConfigNotFound,
                format!("Config file not found: {}: {}", path.display(), e),
            )
        })?;
        let reader = std::io::BufReader::new(file);
        let mut config: Config = serde_json::from_reader(reader).map_err(|e| {
            ModuleError::new(
                ErrorCode::ConfigInvalid,
                format!("Failed to parse JSON config: {}: {}", path.display(), e),
            )
        })?;
        config.yaml_path = Some(path.to_path_buf());
        config.detect_mode();
        init_builtin_namespaces();
        config.apply_env_overrides();
        config.validate()?;
        config.warn_if_path_resolution_will_change();
        config.warn_deprecated_inert_keys();
        Ok(config)
    }

    /// Load config from a YAML file, apply env overrides, and validate.
    pub fn from_yaml_file(path: &std::path::Path) -> Result<Self, ModuleError> {
        let file = std::fs::File::open(path).map_err(|e| {
            ModuleError::new(
                ErrorCode::ConfigNotFound,
                format!("Config file not found: {}: {}", path.display(), e),
            )
        })?;
        let reader = std::io::BufReader::new(file);
        let mut config: Config = serde_yaml::from_reader(reader).map_err(|e| {
            ModuleError::new(
                ErrorCode::ConfigInvalid,
                format!("Failed to parse YAML config: {}: {}", path.display(), e),
            )
        })?;
        config.yaml_path = Some(path.to_path_buf());
        config.detect_mode();
        init_builtin_namespaces();
        config.apply_env_overrides();
        config.validate()?;
        config.warn_if_path_resolution_will_change();
        config.warn_deprecated_inert_keys();
        Ok(config)
    }

    /// Auto-detect format by file extension and load.
    pub fn load(path: &std::path::Path) -> Result<Self, ModuleError> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => Self::from_json_file(path),
            Some("yaml" | "yml") => Self::from_yaml_file(path),
            _ => {
                // Default to YAML
                Self::from_yaml_file(path)
            }
        }
    }

    /// No-arg load: discover the config file via the canonical search order
    /// and load it, falling back to `Config::from_defaults()` if none is
    /// found. Equivalent to apcore-python's `Config.load(path=None)` and
    /// apcore-typescript's `Config.discover()`.
    ///
    /// Sync finding A-D-013: spec contract is `Config.load(path?)`. Rust
    /// previously required a path on `load()` and exposed `discover()` as a
    /// separate method. This helper restores no-arg load parity for portable
    /// cross-language code without changing the strict-typed `load(&Path)`
    /// signature for callers that already know the path.
    pub fn load_or_discover() -> Result<Self, ModuleError> {
        match discover_config_file() {
            Some(path) => Self::load(&path),
            None => Ok(Self::from_defaults()),
        }
    }

    /// Validate a single config key against its registered constraint, if any.
    ///
    /// Mirrors apcore-python `_CONSTRAINTS` (config.py): a per-key
    /// `(check_fn, err_msg)` table. Returns:
    ///   - `None` — the key has no registered constraint (nothing to check).
    ///   - `Some(Ok(()))` — the value satisfies the constraint.
    ///   - `Some(Err(msg))` — the value violates the constraint; `msg` is the
    ///     human-readable reason (e.g. `"must be 'allow' or 'deny'"`).
    ///
    /// Used by `system.control.update_config` to validate-after-set and roll
    /// back on violation (system-modules.md §311-348).
    #[must_use]
    pub fn validate_key_constraint(
        key: &str,
        value: &serde_json::Value,
    ) -> Option<Result<(), String>> {
        // A JSON number that is integral (no fractional part) and within i64.
        fn is_integer(v: &serde_json::Value) -> bool {
            v.is_i64() || v.is_u64()
        }
        fn as_int(v: &serde_json::Value) -> Option<i64> {
            v.as_i64()
                .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
        }
        // A JSON number (int or float), excluding booleans (serde_json never
        // treats `true`/`false` as numbers, so `as_f64` already excludes them).
        fn as_number(v: &serde_json::Value) -> Option<f64> {
            v.as_f64()
        }

        let (ok, err_msg): (bool, &str) = match key {
            "acl.default_effect" => (
                value.as_str() == Some("allow") || value.as_str() == Some("deny"),
                "must be 'allow' or 'deny'",
            ),
            "observability.tracing.sampling_rate" | "sys_modules.events.thresholds.error_rate" => (
                as_number(value).is_some_and(|n| (0.0..=1.0).contains(&n)),
                "must be a number in [0.0, 1.0]",
            ),
            "observability.tracing.strategy" => (
                matches!(
                    value.as_str(),
                    Some("full" | "proportional" | "error_first" | "off")
                ),
                "must be 'full', 'proportional', 'error_first' or 'off'",
            ),
            "observability.tracing.exporter" => (
                // `in_memory` is deliberately absent: PROTOCOL_SPEC §10.1.1
                // requirement 2.
                matches!(value.as_str(), Some("stdout" | "otlp" | "jaeger")),
                "must be 'stdout', 'otlp' or 'jaeger'",
            ),
            "observability.tracing.otlp_endpoint" => (
                value.is_null() || value.as_str().is_some_and(|s| !s.trim().is_empty()),
                "must be a non-empty URL string, or null",
            ),
            "sys_modules.events.thresholds.latency_p99_ms" => (
                as_number(value).is_some_and(|n| n > 0.0),
                "must be a positive number",
            ),
            "extensions.max_depth" => (
                as_int(value).is_some_and(|n| (1..=16).contains(&n)),
                "must be an integer in [1, 16]",
            ),
            "executor.default_timeout" | "executor.global_timeout" => (
                is_integer(value) && as_int(value).is_some_and(|n| n >= 0),
                "must be a non-negative integer (milliseconds)",
            ),
            "executor.max_call_depth"
            | "executor.max_module_repeat"
            | "sys_modules.error_history.max_entries_per_module"
            | "sys_modules.error_history.max_total_entries" => (
                is_integer(value) && as_int(value).is_some_and(|n| n >= 1),
                "must be a positive integer",
            ),
            _ => return None,
        };

        if ok {
            Some(Ok(()))
        } else {
            Some(Err(err_msg.to_string()))
        }
    }

    /// Validate config constraints. Returns an error listing all violations.
    ///
    /// Sync CB-001: validates the spec-mandated field set beyond
    /// executor-only knobs — mirrors apcore-python `_REQUIRED_FIELDS` and
    /// `_CONSTRAINTS` (config.py). Constraints checked include:
    ///   - `acl.default_effect` ∈ {`allow`, `deny`}
    ///   - `observability.tracing.sampling_rate` ∈ [0.0, 1.0]
    ///   - executor numeric ranges (`max_call_depth`, `max_module_repeat`,
    ///     `default_timeout`, `global_timeout`)
    pub fn validate(&self) -> Result<(), ModuleError> {
        let mut errors: Vec<String> = Vec::new();

        // --- 1. Required fields (legacy mode only) -------------------------
        //
        // Per the canonical contract (config-bus.md "Contract: Config.validate")
        // and the reference SDK (apcore-python), required-field enforcement runs
        // only in legacy mode. In namespace mode the `apcore:` block is metadata,
        // not a standalone config, so a minimal namespace-mode YAML is accepted
        // (apcore-python `_validate_namespace_mode` runs constraints only).
        if self.mode != ConfigMode::Namespace {
            // PROTOCOL_SPEC §9.1: a key is required **only when it has no
            // canonical default**. Exactly two qualify. Everything else in the
            // section — `extensions.*`, `schema.*`, `acl.*`, `executor.*`,
            // `sys_modules.*`, `observability.*`, `stream.*` — carries a
            // default in `schemas/defaults.schema.json`, so requiring it would
            // reject a document the framework resolves perfectly well.
            // `schemas/apcore-config.schema.json` declares exactly these two in
            // its `required` array.
            const REQUIRED_FIELDS: &[&str] = &["version", "project.name"];
            // Deliberately uses `get_declared` rather than `get`: per §9.3
            // step 1, requiredness is evaluated against the DECLARED document,
            // before defaults are merged. `get()` falls back to
            // `CONFIG_DEFAULTS`; routing through it would make the check
            // vacuous (decision A-D-03, now spec-backed but narrower).
            //
            // Cross-language note: apcore-python and apcore-typescript
            // deep-merge their default table into the parsed document before
            // this loop runs, so their equivalent check is a no-op. Because
            // neither declares a default for `version` or `project.name`, all
            // three SDKs still accept and reject the same documents.
            for field in REQUIRED_FIELDS {
                if self.get_declared(field).is_none() {
                    errors.push(format!("missing required field '{field}'"));
                }
            }
        }

        // --- 2. Value constraints (both modes) -----------------------------
        //
        // Mirrors apcore-python `_CONSTRAINTS` and the canonical contract table.
        // A constraint applies only when the field is present; absence is a
        // required-field concern (handled above) or simply unset.
        self.collect_constraint_errors(&mut errors);

        // NOTE: there is deliberately NO `global_timeout >= default_timeout`
        // cross-field check. `builtin_steps.rs` clamps the per-module timeout
        // to the remaining global deadline
        // (`if timeout_ms == 0 || remaining_ms < timeout_ms { timeout_ms = remaining_ms }`),
        // so `global_timeout: 10000` with `default_timeout: 30000` is a valid
        // configuration meaning "no single module over 30s, whole chain under
        // 10s". Neither apcore-python, apcore-typescript, nor the PROTOCOL_SPEC
        // §9.3 constraint table rejects it.

        // --- 2b. `observability.tracing` cross-key consistency (§10.1.1) ---
        //
        // Requirement 3: an OTLP endpoint set against an exporter that does not
        // read it is a rejected configuration, not a silent no-op. A value an
        // operator wrote down and nothing reads is the shape of every defect
        // apcore#118 found. Runs in both modes — the disagreement is between
        // two keys, and which file layout declared them changes nothing.
        if self.observability.tracing.otlp_endpoint.is_some() {
            let exporter = &self.observability.tracing.exporter;
            if exporter != "otlp" {
                errors.push(format!(
                    "observability.tracing.otlp_endpoint is set but \
                     observability.tracing.exporter is '{exporter}', which does not read it. \
                     Set exporter to 'otlp', or remove the endpoint."
                ));
            }
        }

        // --- 3. Unknown framework keys (§9.14) -----------------------------
        //
        // `reject_unknown_framework_keys`. Runs in BOTH modes: §9.10 step 1
        // invokes it for legacy documents, where the whole file *is* the
        // `apcore` namespace, and step 2 for the `apcore` namespace of a
        // namespace-mode document.
        let strict = self.strict_mode();
        if strict {
            self.reject_unknown_framework_keys(&mut errors);
        }

        // Namespace-mode validation (A12-NS, §9.10). Mirrors apcore-python
        // `_validate_namespace_mode` (config.py:1106) and the TS equivalent
        // (sync finding A-D-02). In namespace mode we additionally:
        //   1. Validate each registered namespace that declares a schema
        //      against its loaded subtree.
        //   2. In strict mode (`_config.strict == true`) reject any top-level
        //      namespace that is not registered (other than `apcore`/`_config`
        //      and the framework sections — see `is_framework_section`).
        if self.mode == ConfigMode::Namespace {
            // Snapshot the global namespace registry once so we can look up
            // schemas without re-acquiring the lock per namespace.
            let registry_snapshot: HashMap<String, NamespaceRegistration> =
                global_ns_registry().read().clone();

            for (key, value) in &self.user_namespaces {
                if key == "apcore" || key == "_config" {
                    continue;
                }
                // Only object-valued top-level keys are namespaces.
                if !value.is_object() {
                    continue;
                }
                match registry_snapshot.get(key) {
                    None => {
                        // A framework section is not a namespace. Rust merges
                        // the `apcore:` block's members up to the top level of
                        // `user_namespaces`, so `acl`, `extensions`, `project`
                        // … sit here beside genuine namespaces; §9.10 step 3
                        // iterates namespaces only. Their keys are governed by
                        // `reject_unknown_framework_keys` above instead.
                        if strict && !is_framework_section(key) {
                            errors.push(format!("unknown namespace '{key}' in strict mode"));
                        }
                    }
                    Some(reg) => {
                        if let Some(schema) = reg.schema.as_ref() {
                            if let Err(e) =
                                crate::executor::validate_against_schema(value, schema, "Config")
                            {
                                errors.push(format!(
                                    "namespace '{key}' failed schema validation: {}",
                                    e.message
                                ));
                            }
                        }
                    }
                }
            }
        }

        // PROTOCOL_SPEC §9.2.3 requirement 6d / §10.6.1: an
        // `obs.redaction.regex_patterns` entry the engine cannot compile MUST
        // be reported here, not skipped at the first log record. A redaction
        // rule that redacts nothing is indistinguishable, from the outside,
        // from one that works — and on this surface the difference is
        // credentials in plaintext. The `regex` crate is the engine that
        // refuses lookaround and backreferences by design (to keep matching
        // linear in the input), so this fires on patterns that compile
        // perfectly well in Python and JavaScript: exactly the configuration
        // that redacts elsewhere and silently does nothing here.
        if let Some(serde_json::Value::Array(patterns)) = self.get("obs.redaction.regex_patterns") {
            for (index, entry) in patterns.iter().enumerate() {
                let Some(pattern) = entry.as_str() else {
                    continue;
                };
                if pattern.is_empty() {
                    continue;
                }
                if let Err(e) = regex::Regex::new(pattern) {
                    errors.push(format!(
                        "obs.redaction.regex_patterns[{index}] does not compile and would redact \
                         nothing: '{pattern}' ({e}). Patterns should stay inside the portable \
                         subset — no lookaround, no backreferences, no inline (?i) flags \
                         (PROTOCOL_SPEC 9.2.3 requirement 6)."
                    ));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            let message = format!("Config validation failed: {}", errors.join("; "));
            Err(ModuleError::new(ErrorCode::ConfigInvalid, message))
        }
    }

    /// Is `_config.strict` set on the loaded document (§9.6.3)?
    ///
    /// Read from `user_namespaces` rather than `get()` because `_config` is a
    /// reserved top-level key in both modes and must not pick up the
    /// implicit-`apcore` fallback or the [`CONFIG_DEFAULTS`] table. Absent or
    /// non-boolean means `false`, the documented default.
    fn strict_mode(&self) -> bool {
        self.user_namespaces
            .get("_config")
            .and_then(|v| v.get("strict"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    /// `PROTOCOL_SPEC` §9.14 `reject_unknown_framework_keys`: under
    /// `_config.strict: true`, every key inside a framework section MUST be
    /// declared by `schemas/apcore-config.schema.json`.
    ///
    /// **Collects, never short-circuits.** The spec is explicit that the error
    /// "MUST enumerate every offending key rather than failing on the first, so
    /// one restart is enough to see the whole problem" — an operator with two
    /// typos in different sections should not have to restart twice to find
    /// the second one. `validate` joins everything in `errors` into a single
    /// `CONFIG_INVALID`, so appending here is what produces that enumeration.
    ///
    /// Reads the raw `user_namespaces` tree, which after [`TYPED_SECTIONS`]
    /// retention carries the file's object for every framework section — in
    /// namespace mode including the ones nested under `apcore:`, which
    /// `Config::deserialize` merges up to the top level. Only sections the
    /// document actually declares are visited; an absent section has no keys
    /// to reject.
    ///
    /// Deliberately one level deep, matching the sub-algorithm ("for each key
    /// present in `apcore_data[section]`"). Nested closedness (e.g.
    /// `acl.audit.*`) is the canonical schema's business at validation time,
    /// not this check's.
    fn reject_unknown_framework_keys(&self, errors: &mut Vec<String>) {
        // Recursive: the canonical schemas are `additionalProperties: false` at
        // every level, so a nested typo such as
        // `observability.tracing.sampling_rat` is rejected too. This checked one
        // level only, which let exactly that case through — its parent
        // `tracing` IS declared (sync finding A-D-020).
        fn walk(
            data: &serde_json::Map<String, serde_json::Value>,
            prefix: &str,
            errors: &mut Vec<String>,
        ) {
            let mut keys: Vec<&String> = data.keys().collect();
            keys.sort();
            for key in keys {
                let path = format!("{prefix}.{key}");
                if !is_declared_prefix(&path) {
                    errors.push(format!("unknown key '{path}' (strict mode enabled)"));
                    continue; // do not descend into an undeclared subtree
                }
                // A declared leaf ends the walk whatever its value is; a
                // declared container holding a non-object is a type error the
                // A12 constraint table owns, not an undeclared key.
                if let Some(child) = data[key].as_object() {
                    if is_declared_container(&path) {
                        walk(child, &path, errors);
                    }
                }
            }
        }

        let mut sections: Vec<&str> = FRAMEWORK_CONFIG_KEYS
            .iter()
            .filter_map(|p| p.split_once('.').map(|(head, _)| head))
            .collect();
        sections.sort_unstable();
        sections.dedup();

        for section in sections {
            let Some(present) = self
                .user_namespaces
                .get(section)
                .and_then(serde_json::Value::as_object)
            else {
                continue;
            };
            walk(present, section, errors);
        }
    }

    /// Append a validation error for every present-but-out-of-range field in
    /// the canonical constraint table (config-bus.md "Contract: Config.validate";
    /// mirrors apcore-python `_CONSTRAINTS`). A field is checked only when
    /// present — absence is governed by the required-field set, not here.
    fn collect_constraint_errors(&self, errors: &mut Vec<String>) {
        // A JSON number (int or float); serde_json never treats booleans as
        // numbers, so `as_f64` already excludes `true`/`false`.
        fn as_number(v: &serde_json::Value) -> Option<f64> {
            v.as_f64()
        }
        // An integral JSON number within i64 range (excludes floats/booleans).
        fn as_int(v: &serde_json::Value) -> Option<i64> {
            if v.is_i64() || v.is_u64() {
                v.as_i64()
                    .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
            } else {
                None
            }
        }

        // The `observability.tracing.*` constraints DELEGATE to
        // `validate_key_constraint` rather than being restated here.
        //
        // This routine and `validate_key_constraint` are two hand-written
        // implementations of one table, and the pair has already drifted once:
        // the match arms added for spec v1.44.0 were reachable from
        // `validate_key_constraint` and invisible to `validate()`, so
        // `exporter: in_memory` — a value §10.1.1 requirement 2 forbids — loaded
        // without a word. Delegating means a key added to the match is enforced
        // by `validate()` for free.
        for key in [
            "observability.tracing.strategy",
            "observability.tracing.exporter",
            "observability.tracing.otlp_endpoint",
        ] {
            if let Some(value) = self.get(key) {
                if let Some(Err(msg)) = Self::validate_key_constraint(key, &value) {
                    errors.push(format!("{key} {msg} (got {value})"));
                }
            }
        }

        // acl.default_effect ∈ {allow, deny}
        if let Some(de) = self.get("acl.default_effect") {
            let ok = matches!(de.as_str(), Some("allow" | "deny"));
            if !ok {
                errors.push(format!(
                    "acl.default_effect must be 'allow' or 'deny' (got {de})"
                ));
            }
        }

        // Numbers in [0.0, 1.0].
        for key in [
            "observability.tracing.sampling_rate",
            "sys_modules.events.thresholds.error_rate",
        ] {
            if let Some(v) = self.get(key) {
                if as_number(&v).is_none_or(|n| !(0.0..=1.0).contains(&n)) {
                    errors.push(format!("{key} must be a number in [0.0, 1.0] (got {v})"));
                }
            }
        }

        // Positive numbers (> 0).
        if let Some(v) = self.get("sys_modules.events.thresholds.latency_p99_ms") {
            if as_number(&v).is_none_or(|n| n <= 0.0) {
                errors.push(format!(
                    "sys_modules.events.thresholds.latency_p99_ms must be a positive number (got {v})"
                ));
            }
        }

        // Non-negative integers (>= 0, milliseconds).
        for key in ["executor.default_timeout", "executor.global_timeout"] {
            if let Some(v) = self.get(key) {
                if as_int(&v).is_none_or(|n| n < 0) {
                    errors.push(format!("{key} must be a non-negative integer (got {v})"));
                }
            }
        }

        // Integers >= 1.
        for key in [
            "executor.max_call_depth",
            "executor.max_module_repeat",
            "sys_modules.error_history.max_entries_per_module",
            "sys_modules.error_history.max_total_entries",
        ] {
            if let Some(v) = self.get(key) {
                if as_int(&v).is_none_or(|n| n < 1) {
                    errors.push(format!("{key} must be an integer >= 1 (got {v})"));
                }
            }
        }

        // Integers in [1, 16]. Mirrors `validate_key_constraint` and the spec so
        // both Rust validation paths (validate / update_config) agree.
        if let Some(v) = self.get("extensions.max_depth") {
            if as_int(&v).is_none_or(|n| !(1..=16).contains(&n)) {
                errors.push(format!(
                    "extensions.max_depth must be an integer in [1, 16] (got {v})"
                ));
            }
        }
    }

    /// Build config from defaults, applying env var overrides.
    #[must_use]
    pub fn from_defaults() -> Self {
        let mut config = Self::default();
        config.detect_mode();
        init_builtin_namespaces();
        config.apply_env_overrides();
        config
    }

    /// Return the filesystem path the config was loaded from, if any.
    ///
    /// Returns the path passed to (or discovered by) [`Config::load`] /
    /// [`Config::from_yaml_file`] / [`Config::from_json_file`], or `None` for
    /// in-memory configs (deserialized directly) and those produced by
    /// [`Config::from_defaults`]. Consumers that resolve relative paths (e.g.
    /// `acl.root` via [`crate::acl::ACL::discover`]) use this to anchor
    /// resolution at the config file's directory rather than the current
    /// working directory. Mirrors apcore-python's `Config.source_path`.
    #[must_use]
    pub fn source_path(&self) -> Option<&std::path::Path> {
        self.yaml_path.as_deref()
    }

    /// The directory relative path-typed configuration values are *about*
    /// (aiperceivable/apcore#113, spec §9.2.2):
    ///
    /// ```text
    /// project_root =
    ///     directory of the config file   when it came from §9.14 tier 1-5
    ///                                    (explicitly pointed at, or project-local)
    ///     CWD                            when it came from tier 6-7 (user-level),
    ///                                    or when no config file was found
    /// ```
    ///
    /// Tiers 6-7 anchor at CWD because a per-user config's relative paths are
    /// per-*project* by intent: `extensions.root: ./extensions` in
    /// `~/.config/apcore/config.yaml` cannot sensibly mean
    /// `~/.config/apcore/extensions`. Tiers 2-5 — the overwhelmingly common
    /// case — are the ones where the config file's directory already *is* CWD,
    /// so the two candidate bases are indistinguishable there.
    ///
    /// The returned path is absolute, and canonicalized when the config file
    /// exists. `Config`s with no source path — [`Self::from_defaults`] and
    /// those deserialized in memory — report CWD.
    ///
    /// **This accessor changes nothing.** As of this release
    /// [`crate::acl::ACL::discover`] still resolves `acl.root` against the
    /// config file's directory for every tier including 6-7, and
    /// [`crate::schema::loader::SchemaLoader::with_config`] still resolves
    /// `schema.root` against CWD. Publishing the base is the §13.2 deprecation
    /// phase of unifying them; see
    /// [`Self::path_typed_keys`] for which keys it will apply to.
    #[must_use]
    pub fn project_root(&self) -> std::path::PathBuf {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        match self.source_path() {
            Some(source) if !is_user_level_config_path(source) => source
                .canonicalize()
                .unwrap_or_else(|_| cwd.join(source))
                .parent()
                .map_or_else(|| cwd.clone(), std::path::Path::to_path_buf),
            _ => cwd,
        }
    }

    /// Path-typed keys (§9.2.1) whose currently-resolved value is a **relative**
    /// path, reported using the [`Self::path_typed_keys`] spelling.
    ///
    /// Read through [`Self::get`], so a key left undeclared but carrying a
    /// canonical default counts — `schema.root`'s `./schemas` re-roots under a
    /// unified base exactly as a hand-written `./schemas` would. A config that
    /// spells every path-typed key absolutely yields an empty list, which is
    /// what makes it a usable guard rather than a restatement of
    /// `project_root != cwd`.
    ///
    /// `extensions.roots` is list-valued; it counts when *any* element is
    /// relative, in either the bare-string or the `{ root, namespace }` form.
    fn relative_path_typed_keys(&self) -> Vec<&'static str> {
        fn is_relative(value: Option<&str>) -> bool {
            value.is_some_and(|s| !s.is_empty() && std::path::Path::new(s).is_relative())
        }

        let mut relative = Vec::new();
        for key in Self::path_typed_keys() {
            let hit = match key.strip_suffix("[]") {
                Some(list_key) => matches!(
                    self.get(list_key),
                    Some(serde_json::Value::Array(ref items))
                        if items.iter().any(|item| is_relative(match item {
                            serde_json::Value::String(s) => Some(s.as_str()),
                            serde_json::Value::Object(o) =>
                                o.get("root").and_then(serde_json::Value::as_str),
                            _ => None,
                        }))
                ),
                None => is_relative(self.get(key).as_ref().and_then(serde_json::Value::as_str)),
            };
            if hit {
                relative.push(*key);
            }
        }
        relative
    }

    /// Warn that this config's relative path-typed values will re-root when
    /// §9.2.2's single base lands (aiperceivable/apcore#113).
    ///
    /// Deliberately **narrow**: it fires only when both halves of the condition
    /// hold — [`Self::project_root`] differs from CWD *and*
    /// [`Self::relative_path_typed_keys`] is non-empty. A blanket warning on
    /// every load would train operators to ignore it, and would be wrong for
    /// the tier 2-5 majority, whose project root already is CWD and for whom
    /// nothing changes at all.
    ///
    /// **Cadence: once per configuration load, never once per process.** This is
    /// normative — §9.2.2 requirement 2 states it outright and forbids
    /// suppressing the warning with process-global state — so it is not a local
    /// judgement call: an implementation holding a once-per-process flag is
    /// non-conforming as of spec v1.36.0.
    ///
    /// v1.35.0 required the warning and its two narrowing conditions but said
    /// nothing about cadence, and the three SDKs immediately invented three
    /// (Python deduplicating through the `warnings` filter, TypeScript holding
    /// a module-global once-flag, Rust warning per load). v1.36.0 adopts this
    /// SDK's reading and gives the two reasons for it. The warning is a
    /// property of *the document being loaded*, so every load meeting both
    /// conditions emits and a load meeting neither is silent. A once-per-process
    /// flag makes emission **order-dependent** — whichever configuration loads
    /// first consumes the warning, so a later affected document is silent and
    /// the operator cannot tell which one triggered it — and that same global is
    /// a test-isolation hazard, one test consuming the warning another needed.
    /// De-duplication for log volume belongs to the host logging layer.
    ///
    /// It follows that [`Self::reload`], which genuinely re-reads the file,
    /// warns again: §9.2.2 says a reload SHOULD re-evaluate, and an operator who
    /// has just edited the document should see the notice against the version
    /// they just wrote. This is the opposite cadence to the
    /// `observability.redaction.*` legacy-key warning, which guards a key read
    /// on a hot path rather than a document.
    fn warn_if_path_resolution_will_change(&self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let root = self.project_root();
        if root == cwd {
            return;
        }
        let relative = self.relative_path_typed_keys();
        if relative.is_empty() {
            return;
        }
        tracing::warn!(
            project_root = %root.display(),
            cwd = %cwd.display(),
            keys = %relative.join(", "),
            "[apcore] DEPRECATION (spec §13.2): this config file's directory is not the \
             process working directory, and the path-typed keys listed above hold relative \
             values. Today they resolve against different bases per key — acl.root against \
             the config file's directory, schema.root and extensions.root against the \
             working directory. A future major version resolves all of them against the \
             single project root shown above. Nothing changes in this release; write \
             absolute paths to pin today's behaviour. See aiperceivable/apcore#113"
        );
    }

    /// PROTOCOL_SPEC §9.2.4 — warn for declared keys that reach no consumer.
    ///
    /// Driven by the **declared** document, never the merged view (requirement
    /// 2). Every one of these keys has a default, so a merged-view check would
    /// fire for every configuration ever loaded — the blanket warning §9.2.2
    /// rejects, which trains operators to ignore the one that matters.
    ///
    /// Behaviour is unchanged (requirement 3): the keys still parse, still
    /// validate, still answer `get()`, and are still accepted under
    /// `_config.strict`. This adds the one thing they have never had — a way for
    /// an operator to find out that setting them does nothing.
    fn warn_deprecated_inert_keys(&self) {
        // Read the AS-WRITTEN document, not `get_declared` and not `data()`.
        // `observability` is a typed struct field, so both of those always carry
        // every leaf: `get_declared("observability.tracing.enabled")` answers
        // `Some(false)` for a document that never mentions it, and this warning
        // would then fire for every configuration ever loaded — the blanket
        // warning §9.2.2 rejects and §9.2.4 requirement 2 forbids.
        //
        // `user_namespaces` is the right source because it retains the RAW
        // object of every typed section exactly as the file wrote it (see the
        // field's own documentation), which is also what makes the three SDKs
        // report the same key set for the same file. That Rust's `get_declared`
        // cannot answer this question is a separate divergence, recorded rather
        // than repaired here.
        let declared: Vec<&str> = DEPRECATED_INERT_KEYS
            .iter()
            .copied()
            .filter(|key| {
                let Some((section, rest)) = key.split_once('.') else {
                    return false;
                };
                let Some(mut node) = self.user_namespaces.get(section).or_else(|| {
                    self.user_namespaces
                        .get("apcore")
                        .and_then(|framework| framework.get(section))
                }) else {
                    return false;
                };
                for part in rest.split('.') {
                    match node.get(part) {
                        Some(next) => node = next,
                        None => return false,
                    }
                }
                true
            })
            .collect();

        // No separate environment scan: an `APCORE_*` override goes through
        // `apply_env_overrides` -> `set()`, and `set()` now leaves the same
        // as-written record in `user_namespaces` that a file load leaves, for
        // typed leaves as well (#119). Until that fix `set()` short-circuited
        // into `set_typed_field` and touched nothing, so this function carried
        // a second traversal that re-derived the tier from `std::env::vars()`
        // — two spellings of one rule, in one file. The `set()` mirror deletes
        // the need for it; the environment-tier cases in
        // `tests/test_deprecated_inert_keys.rs` are what hold that claim.
        let mut declared: Vec<&str> = declared;
        // Report in the constant's order, so two SDKs name them the same way
        // whichever tier supplied each one.
        declared.sort_by_key(|k| DEPRECATED_INERT_KEYS.iter().position(|d| d == k));

        if declared.is_empty() {
            return;
        }
        tracing::warn!(
            count = declared.len(),
            keys = %declared.join(", "),
            "[apcore] DEPRECATION (spec §9.2.4): this configuration declares keys that \
             reach no consumer in any apcore SDK and have no effect. They keep parsing and \
             validating for the whole 1.x line and are removed no earlier than v2.0 \
             (§13.2 / §13.4). Nothing has changed in this release — the keys did nothing \
             before this warning existed. See aiperceivable/apcore#118"
        );
    }

    /// Discover and load config using the §9.14 search order.
    ///
    /// If no file is found, returns `Config::from_defaults()`.
    pub fn discover() -> Result<Self, ModuleError> {
        match discover_config_file() {
            Some(path) => Self::load(&path),
            None => Ok(Self::from_defaults()),
        }
    }

    /// The closed set of path-typed configuration keys (PROTOCOL_SPEC §9.2.1).
    ///
    /// A path-typed key is one whose value is a filesystem path. The set is
    /// declared canonically by `"x-apcore-path": true` in
    /// `schemas/apcore-config.schema.json`; this returns that projection,
    /// sorted.
    ///
    /// `extensions.roots` is reported as `extensions.roots[]` because it is
    /// list-valued and every element carries a path.
    ///
    /// Note what this does NOT tell you: what a *relative* value in one of these
    /// keys resolves against. That base is unspecified as of spec v1.34.0 and
    /// currently differs between keys — `acl.root` resolves against the config
    /// file's directory, `schema.root` against the process CWD.
    #[must_use]
    pub fn path_typed_keys() -> &'static [&'static str] {
        PATH_TYPED_CONFIG_KEYS
    }

    /// Get a config value by dot-path key.
    ///
    /// Walks the canonical nested namespace tree (`executor.*`,
    /// `observability.*`, `modules_path`) and falls back to user-defined
    /// namespaces. Per spec §9.1, all keys MUST use the canonical
    /// `<namespace>.<field>` form. Legacy v0.17.x short-form aliases
    /// (e.g. bare `max_call_depth`) are NOT accepted.
    ///
    /// Sync finding A-D-017: namespace resolution uses longest-prefix match
    /// against registered names, mirroring apcore-python's
    /// `_split_namespace_key` and apcore-typescript's `resolveNamespacePath`.
    /// Hyphenated namespace names (e.g. `apcore-mcp.transport.endpoint`)
    /// route correctly even though `.split('.')` would otherwise strand the
    /// hyphenated prefix on the first segment.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        if let Some(val) = self.get_direct(key) {
            return Some(val);
        }

        // A-D-009: §9.9.1 implicit-apcore fallback. In Namespace mode, when the
        // key's first segment does not resolve to a top-level namespace, retry
        // under the implicit `apcore` namespace so a user key stored under
        // `apcore.<key>` is reachable by its bare name. Mirrors apcore-python
        // (`_get_namespace_mode`, config.py:858/866) and apcore-typescript
        // (config.ts:774). Guarded against recursion: never re-prefix a key that
        // already targets `apcore`.
        if self.mode == ConfigMode::Namespace
            && key != "apcore"
            && !key.starts_with("apcore.")
            && self.user_namespaces.contains_key("apcore")
        {
            if let Some(val) = self.get_direct(&format!("apcore.{key}")) {
                return Some(val);
            }
        }

        // Then the registered namespace's own defaults (§9.15). apcore-python
        // and apcore-typescript seed these into the data tree at load in
        // namespace mode (`_apply_namespace_defaults`, config.py; the
        // `_globalNsRegistry` loop, config.ts), so their `get` answers from
        // them. Rust consulted them only inside `namespace()`, which left the
        // two readers disagreeing on any subkey the file omitted —
        // `namespace("sys_modules")["usage"]["enabled"]` was `true` while
        // `get("sys_modules.usage.enabled")` was `None`.
        //
        // Deliberately placed in `get` and NOT in `get_direct`: `get_declared`
        // delegates to `get_direct`, and legacy-mode required-field validation
        // depends on it distinguishing "declared" from "defaulted". A default
        // leaking there would let §9.3 step 1 pass on an undeclared key.
        // D-117: only in NAMESPACE mode. A legacy (namespace-less) document has
        // no namespaces, so a declaration ABOUT a namespace has nothing to say
        // about one — §9.6.3's reading, and what apcore-python and
        // apcore-typescript already did by seeding these defaults into the data
        // tree at load time, which only happens in namespace mode.
        //
        // Consulted unconditionally, this SDK answered a registered namespace's
        // default for a document that never opted into namespaces at all: a key
        // no reader of that file could see in it, and one the peers reported as
        // absent.
        if self.mode == ConfigMode::Namespace {
            if let Some(val) = Self::registered_namespace_default(key) {
                return Some(val);
            }
        }

        // Fall back to the canonical default table. apcore-python deep-merges
        // `_DEFAULTS` into its data tree at load and apcore-typescript merges
        // `DEFAULTS`, so both return the canonical value for a key a legacy
        // YAML omits. Consulting the table here gives Rust the same answer
        // without mutating the loaded tree (so `data()` still round-trips the
        // file as written).
        Self::default_for(key)
    }

    /// Resolve `key` against the `defaults` of its registered namespace (§9.15).
    ///
    /// Returns `None` when the key names no registered namespace, when that
    /// namespace declared no defaults, or when the path is absent from them.
    ///
    /// Ordered AFTER the loaded config and BEFORE [`CONFIG_DEFAULTS`]: a value
    /// the operator wrote always wins, and a namespace that declares its own
    /// default for a key is more specific than the flat canonical table. The
    /// two tables no longer disagree on `sys_modules.enabled` — both say
    /// `false` as of spec v1.17.0 — which is what made this ordering safe to
    /// state at all.
    fn registered_namespace_default(key: &str) -> Option<serde_json::Value> {
        let (ns_name, rest) = Self::match_registered_namespace(key)?;
        let registry = global_ns_registry().read();
        let defaults = registry.get(&ns_name)?.defaults.as_ref()?;
        if rest.is_empty() {
            return Some(defaults.clone());
        }
        let mut current = defaults;
        for part in rest.split('.') {
            current = current.get(part)?;
        }
        Some(current.clone())
    }

    /// Like [`Self::get`] but WITHOUT the [`CONFIG_DEFAULTS`] fallback: returns
    /// `Some` only when the key was actually declared by the loaded config
    /// (file, env override, `set()`, or a typed struct field).
    ///
    /// Used by [`Self::validate`]'s required-field check, which must
    /// distinguish "declared" from "defaulted".
    #[must_use]
    pub fn get_declared(&self, key: &str) -> Option<serde_json::Value> {
        if let Some(val) = self.get_as_written(key) {
            return Some(val);
        }
        if self.mode == ConfigMode::Namespace
            && key != "apcore"
            && !key.starts_with("apcore.")
            && self.user_namespaces.contains_key("apcore")
        {
            return self.get_as_written(&format!("apcore.{key}"));
        }
        None
    }

    /// Resolve the canonical default value for a config key, if one exists.
    ///
    /// Mirrors apcore-python's `Config.get_default` and apcore-typescript's
    /// `getDefault`. Returns `None` for keys without a defined default.
    #[must_use]
    pub fn default_for(key: &str) -> Option<serde_json::Value> {
        CONFIG_DEFAULTS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_json())
    }

    /// The single reconciled view of the `observability` namespace: the raw
    /// object as loaded (file, `set()`, `mount()`, env override), with the
    /// typed [`ObservabilityConfig`] leaves overlaid last.
    ///
    /// Overlay order encodes the precedence rule stated on [`Config`]: the
    /// typed struct is authoritative for the four leaves it models, the raw
    /// tree owns every other subkey. Because `set("observability.tracing.
    /// enabled", …)` routes into the typed struct (`set_typed_field` matches
    /// first) while the file also left a copy in the raw tree, overlaying the
    /// typed struct last is what stops a stale file value from resurfacing in
    /// `data()` / `namespace()` after a runtime `set`.
    ///
    /// Overlaying unconditionally is safe when nothing was configured: the
    /// typed defaults (`enabled: false`, `sampling_rate: 1.0`,
    /// `exporter: "stdout"`, `metrics.enabled: false`) are the same values the
    /// §9.15.2 registration declares.
    fn observability_view(&self) -> serde_json::Value {
        self.typed_namespace_view(OBSERVABILITY_NS, &self.observability)
    }

    /// The single reconciled view of the `executor` namespace: the raw object
    /// if one exists, with the typed [`ExecutorConfig`] overlaid last.
    ///
    /// Same precedence rule and same overlay order as
    /// [`Self::observability_view`], for the same reason. Since §9.14 the two
    /// stores also arise the same way: `Config::deserialize` retains the raw
    /// `executor:` block alongside the typed struct (see [`TYPED_SECTIONS`]),
    /// so the base layer carries whatever the file wrote — including the keys
    /// `ExecutorConfig` does not model, which is the whole point. `set(
    /// "executor.<key>", …)` for a key `set_typed_field` does not match, and
    /// `mount("executor", …)`, add to the same base layer at runtime.
    ///
    /// Overlaying the typed struct last is what stops a stale copy from
    /// resurfacing: a `set("executor.max_call_depth", …)` lands in the typed
    /// struct while the file's original value is still sitting in the raw
    /// tree, and the caller must read back what they just set. A `mount` that
    /// tries to override `max_call_depth` loses to the typed struct for the
    /// same reason — the documented rule ("the typed struct wins for the leaves
    /// it models"), matching how a mounted `observability.tracing.enabled`
    /// behaves.
    fn executor_view(&self) -> serde_json::Value {
        self.typed_namespace_view(EXECUTOR_NS, &self.executor)
    }

    /// Reconcile a namespace that is simultaneously a typed [`Config`] field
    /// and a possible `user_namespaces` entry: raw tree as the base, typed
    /// struct deep-merged over it.
    ///
    /// Extracted so [`Self::observability_view`] and [`Self::executor_view`]
    /// cannot drift apart — the precedence rule they encode is the same rule,
    /// and issues #33 and #34 were both "two stores, readers disagreed".
    fn typed_namespace_view<T: Serialize>(&self, name: &str, typed: &T) -> serde_json::Value {
        let mut view = self
            .user_namespaces
            .get(name)
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Ok(typed) = serde_json::to_value(typed) {
            deep_merge_value(&mut view, &typed);
        }
        view
    }

    /// Direct lookup with no implicit-apcore fallback (the pre-A-D-009 path).
    fn get_direct(&self, key: &str) -> Option<serde_json::Value> {
        // Check canonical typed fields first.
        if let Some(val) = self.get_typed_field(key) {
            return Some(val);
        }

        // Issue #33: everything else under `observability` resolves against
        // the reconciled view, not the raw bag. Leaf keys would resolve the
        // same either way, but CONTAINER fetches (`observability`,
        // `observability.tracing`, `observability.metrics`) must not hand back
        // a subtree whose typed leaves are stale — `get("observability.
        // tracing")["enabled"]` disagreeing with
        // `get("observability.tracing.enabled")` is the same wrong-value
        // failure this issue is about, one level down.
        if let Some(rest) = strip_namespace(key, OBSERVABILITY_NS) {
            // Guard on the raw entry, not on the view: without it a config
            // that never declared an `observability:` block would start
            // answering `Some` for the namespace key itself, and
            // `get_declared` — which decides required-field validation on
            // "did the document declare this" — would lose the distinction.
            if !self.user_namespaces.contains_key(OBSERVABILITY_NS) {
                return None;
            }
            return walk_view(self.observability_view(), rest);
        }

        // Issue #34: the same routing for `executor`, whose CONTAINER fetch
        // resolved to `None` for every file-loaded config — `executor` is a
        // typed field, so `user_namespaces` held no entry to traverse and the
        // dot-split fallback below found nothing. A config whose file said
        // `max_call_depth: 7` answered `Some(7)` for
        // `get("executor.max_call_depth")` and `None` for `get("executor")`.
        // apcore-python and apcore-typescript both return the object.
        //
        // Deliberately NOT guarded on the raw entry the way `observability` is:
        // that guard exists to preserve a "declared vs. defaulted" distinction,
        // and `executor` has none to preserve. Every `ExecutorConfig` leaf is
        // non-optional, so `get("executor.max_call_depth")` answers `Some(32)`
        // for a document that declares no `executor:` block at all; the
        // container has to answer `Some` there too or it contradicts its own
        // leaf. An unmodelled key still resolves to `None` unless something put
        // it in the raw tree — `walk_view` fails on the missing member.
        if let Some(rest) = strip_namespace(key, EXECUTOR_NS) {
            return walk_view(self.executor_view(), rest);
        }

        self.get_as_written(key)
    }

    /// Resolve `key` against the AS-WRITTEN store only — `user_namespaces`,
    /// never a typed struct and never a reconciled view.
    ///
    /// This is the half of `get_direct` that answers "is it in the document",
    /// as opposed to "what is its effective value", and [`Self::get_declared`]
    /// needs exactly that half. Extracted rather than copied: two spellings of
    /// one traversal is how `observability` came to have two stores that
    /// disagreed (see [`OBSERVABILITY_NS`]).
    fn get_as_written(&self, key: &str) -> Option<serde_json::Value> {
        // Longest-prefix match against the registered namespaces, then fall
        // back to dot-split on the first segment. Hyphenated names like
        // `apcore-mcp` cannot be reached by naive `split('.')`.
        if let Some((ns_name, rest)) = Self::match_registered_namespace(key) {
            let top = self.user_namespaces.get(&ns_name)?;
            if rest.is_empty() {
                return Some(top.clone());
            }
            let mut current = top;
            for part in rest.split('.') {
                current = current.get(part)?;
            }
            return Some(current.clone());
        }

        // Fall back to user namespaces with dot-path traversal on the first
        // segment (covers namespaces that were not explicitly registered).
        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() {
            return None;
        }
        let top = self.user_namespaces.get(parts[0])?;
        if parts.len() == 1 {
            return Some(top.clone());
        }
        let mut current = top;
        for part in &parts[1..] {
            current = current.get(*part)?;
        }
        Some(current.clone())
    }

    /// Match `key` against the longest registered namespace name that is a
    /// prefix-with-`.` (or exact-match) of the key. Returns `(namespace_name,
    /// remainder_after_namespace_dot)`. Used by `get()` to support
    /// hyphenated namespaces (sync finding A-D-017).
    fn match_registered_namespace(key: &str) -> Option<(String, String)> {
        let registry = global_ns_registry().read();
        // Sort registered names by length descending so longer matches win.
        let mut names: Vec<&String> = registry.keys().collect();
        names.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for name in names {
            if key == name.as_str() {
                return Some((name.clone(), String::new()));
            }
            let dotted = format!("{name}.");
            if key.starts_with(&dotted) {
                return Some((name.clone(), key[dotted.len()..].to_string()));
            }
        }
        None
    }

    /// Set a config value by dot-path key.
    ///
    /// Attempts to set canonical typed fields first, then falls back to
    /// user namespaces. Returns silently on type mismatch.
    ///
    /// NOTE (sync finding A-D-050, deferred): unlike `get()`, this does NOT
    /// route through `match_registered_namespace`. Doing so would re-acquire
    /// the global namespace-registry read lock, which deadlocks because
    /// `apply_env_overrides` already calls `set()` while holding that read
    /// guard (parking_lot RwLock is not reentrant under a queued writer). The
    /// naive dot-split below matches `get()` for every realistic namespace
    /// name (the two only diverge for a namespace whose *name* contains a dot,
    /// which `register_namespace` does not permit). See decision log.
    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        self.generation += 1;
        if let Some(framework_key) = key.strip_prefix("apcore.") {
            // Qualified framework writes also update fields read by runtime consumers.
            self.set_typed_field(framework_key, &value);
        }
        // Try canonical typed fields.
        if self.set_typed_field(key, &value) {
            // A typed write is still a DECLARATION, so it leaves the same
            // as-written record a file load leaves (#119).
            self.record_typed_as_written(key, &value);
            return;
        }

        // Fall back to user namespaces.
        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() {
            return;
        }
        if parts.len() == 1 {
            self.user_namespaces.insert(key.to_string(), value);
            return;
        }
        let root = self
            .user_namespaces
            .entry(parts[0].to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let mut current = root;
        for part in &parts[1..parts.len() - 1] {
            if !current.is_object() {
                *current = serde_json::Value::Object(serde_json::Map::new());
            }
            // INVARIANT: the preceding `if !current.is_object()` branch guarantees object shape.
            current = current
                .as_object_mut()
                .unwrap()
                .entry(part.to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        }
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        // INVARIANT: the preceding `if !current.is_object()` branch guarantees object shape.
        current
            .as_object_mut()
            .unwrap()
            .insert(parts[parts.len() - 1].to_string(), value);
    }

    /// Reload config from the stored `yaml_path`. Returns error if no path stored.
    pub fn reload(&mut self) -> Result<(), ModuleError> {
        let start_gen = self.generation;
        let path = self.yaml_path.clone().ok_or_else(|| {
            ModuleError::new(
                ErrorCode::ReloadFailed,
                "Cannot reload: no yaml_path stored (config was not loaded from a file)",
            )
        })?;
        let mut reloaded = Self::load(&path)?;

        if self.generation != start_gen {
            return Err(ModuleError::new(
                ErrorCode::ModuleReloadConflict,
                "Config modified during reload",
            ));
        }

        // Preserve the yaml_path through reload
        let yaml_path = self.yaml_path.take();
        // PROTOCOL_SPEC §9.11: mounted namespaces survive reload. Carry the
        // recorded mount payloads across and replay them in mount order on top
        // of the freshly-read file, so file content still wins for keys the
        // file declares and mounted-only keys remain resolvable. Matches
        // apcore-python and apcore-typescript, which re-apply their mount
        // registry after re-reading.
        let mounts = std::mem::take(&mut self.mounts);
        reloaded.generation = self.generation + 1;
        *self = reloaded;
        self.yaml_path = yaml_path;
        for (namespace, data) in &mounts {
            self.apply_mount(namespace, data);
        }
        self.mounts = mounts;
        // PROTOCOL_SPEC §9.11 step 5: re-validate. `Self::load` above already
        // validated the freshly-read file, but that ran BEFORE the mount replay
        // — so without this the post-mount tree is never checked and a mount
        // carrying an out-of-range value survives a reload silently.
        //
        // Rust's loaders validate unconditionally (there is no `validate=false`
        // opt-out to carry forward, unlike apcore-python and apcore-typescript),
        // so the second clause of §9.11 step 5 applies: always re-validate.
        self.validate()?;
        Ok(())
    }

    /// Re-read the original config file from disk, discarding any in-memory
    /// `set()` mutations made since the last load. Issue #45.5.
    ///
    /// Rust cannot dynamically reload compiled module code (no `.so`/`.rlib`
    /// hot-swap in the SDK), so the `system.control.reload_module` module
    /// uses this method when invoked with `reload_config: true` to refresh
    /// static configuration without a binary restart. The method requires
    /// the Config to have been loaded from a file via
    /// [`Config::from_yaml_file`] / [`Config::from_json_file`] / [`Config::load`];
    /// configs built via [`Config::from_defaults`] return `ReloadFailed`.
    ///
    /// Equivalent to [`Config::reload`] — kept as a separate method so the
    /// reload-module call site reads naturally and the spec terminology
    /// (`reload_from_disk`) stays first-class in the public API.
    pub fn reload_from_disk(&mut self) -> Result<(), ModuleError> {
        self.reload()
    }

    /// Return a `serde_json::Value` representing the full config as the
    /// canonical nested JSON object (`PROTOCOL_SPEC` §9.1 wire format).
    #[must_use]
    pub fn data(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    // --- Namespace registration (class methods) ---

    pub fn register_namespace(mut reg: NamespaceRegistration) -> Result<(), ModuleError> {
        if RESERVED_NAMESPACES.contains(&reg.name.as_str()) {
            return Err(ModuleError::config_namespace_reserved(&reg.name));
        }
        // Auto-derive env_prefix from name if not provided.
        if reg.env_prefix.is_none() {
            reg.env_prefix = Some(reg.name.to_uppercase().replace('-', "_"));
        }
        if reg.env_prefix.as_deref() == Some("APCORE") && reg.name != "apcore" {
            return Err(ModuleError::config_namespace_reserved(&reg.name));
        }
        let mut map = global_ns_registry().write();
        if map.contains_key(&reg.name) {
            return Err(ModuleError::config_namespace_duplicate(&reg.name));
        }
        // Check for duplicate env_prefix.
        let prefix = reg.env_prefix.as_deref().unwrap_or("");
        for existing in map.values() {
            if existing.env_prefix.as_deref() == Some(prefix) {
                return Err(ModuleError::config_env_prefix_conflict(prefix));
            }
        }
        // Validate env_map: no env var can be claimed twice.
        if let Some(ref em) = reg.env_map {
            let claimed = env_map_claimed().read();
            for env_var in em.keys() {
                if let Some(owner) = claimed.get(env_var) {
                    return Err(ModuleError::config_env_map_conflict(env_var, owner));
                }
            }
            drop(claimed);
            let mut claimed = env_map_claimed().write();
            for env_var in em.keys() {
                claimed.insert(env_var.clone(), reg.name.clone());
            }
        }
        map.insert(reg.name.clone(), reg);
        Ok(())
    }

    /// Register global bare env var → top-level config key mappings.
    pub fn env_map(mapping: HashMap<String, String>) -> Result<(), ModuleError> {
        let claimed_lock = env_map_claimed();
        let claimed = claimed_lock.read();
        for env_var in mapping.keys() {
            if let Some(owner) = claimed.get(env_var) {
                return Err(ModuleError::config_env_map_conflict(env_var, owner));
            }
        }
        drop(claimed);
        let mut claimed = claimed_lock.write();
        let mut gmap = global_env_map().write();
        for (env_var, config_key) in mapping {
            claimed.insert(env_var.clone(), "__global__".to_string());
            gmap.insert(env_var, config_key);
        }
        Ok(())
    }

    #[must_use]
    pub fn registered_namespaces() -> Vec<NamespaceInfo> {
        global_ns_registry()
            .read()
            .values()
            .map(|r| NamespaceInfo {
                name: r.name.clone(),
                env_prefix: r.env_prefix.clone(),
                has_schema: r.schema.is_some(),
            })
            .collect()
    }

    /// Return the set of top-level namespace names reserved by the apcore
    /// framework (`PROTOCOL_SPEC` §9.9.5).
    ///
    /// The returned slice is the single source of truth referenced by
    /// [`Config::register_namespace`] to enforce `CONFIG_NAMESPACE_RESERVED`
    /// (§9.5.1 rules 3 and 4). It is callable without instantiating a
    /// `Config`, so third-party consumers (custom CLIs, framework
    /// integrations) can fail-fast on user-supplied namespace names before
    /// invoking [`Config::register_namespace`].
    #[must_use]
    pub fn reserved_namespaces() -> &'static [&'static str] {
        RESERVED_NAMESPACES
    }

    // --- Namespace instance methods ---

    /// Return the merged value map for a namespace (config-bus.md §914/917/920).
    ///
    /// The returned map is "merged from defaults + YAML + env overrides": the
    /// registered namespace `defaults` form the base, overlaid (deep-merged) by
    /// the loaded `user_namespaces` subtree (which already carries YAML + env
    /// overrides). An unregistered namespace with no loaded values returns an
    /// EMPTY map (never `None`). Mirrors apcore-python `Config.namespace`, which
    /// returns `self._data.get(name, {})` over its pre-merged data tree.
    #[must_use]
    pub fn namespace(&self, name: &str) -> HashMap<String, serde_json::Value> {
        // Base: registered defaults (if the namespace is registered and its
        // defaults are an object). Non-object defaults contribute nothing.
        let mut merged = serde_json::Value::Object(serde_json::Map::new());
        if let Some(reg) = global_ns_registry().read().get(name) {
            if let Some(defaults @ serde_json::Value::Object(_)) = reg.defaults.as_ref() {
                deep_merge_value(&mut merged, defaults);
            }
        }

        // Overlay: loaded YAML + env values for this namespace.
        //
        // Issue #33: `observability` overlays the reconciled view instead of
        // the raw bag. Before the fix this method was the sharpest edge of the
        // defect — the raw bag held nothing at all for a file-loaded config, so
        // `namespace("observability")` returned the REGISTERED DEFAULT for a
        // key the operator had explicitly set: a file saying
        // `logging.enabled: false` read back `true`. That is worse than a
        // missing value, because nothing distinguishes it from a real choice.
        // Going through the view also keeps the four typed leaves in step with
        // `get()`, which the raw bag alone would not (a `set()` on
        // `tracing.enabled` lands in the typed struct, not the bag).
        // Issue #34: `executor` overlays its reconciled view for the same
        // reason. It is a typed field with no `user_namespaces` entry, so this
        // method returned an EMPTY map for every file-loaded config — while
        // `get("executor.max_call_depth")` on the same config returned the
        // file's value. A caller reading the namespace as a unit (which is what
        // `namespace()` is for, and what `bind` is built on) saw nothing at all
        // where apcore-python and apcore-typescript both return the object.
        // Unlike `observability` there is no registered §9.15 default layer
        // underneath, so the failure was a missing value rather than a
        // confidently wrong one — but `{}` still contradicts `get()`.
        if name == OBSERVABILITY_NS {
            deep_merge_value(&mut merged, &self.observability_view());
        } else if name == EXECUTOR_NS {
            deep_merge_value(&mut merged, &self.executor_view());
        } else if let Some(loaded @ serde_json::Value::Object(_)) = self.user_namespaces.get(name) {
            deep_merge_value(&mut merged, loaded);
        }

        match merged {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            // Unreachable: `merged` is initialized as an object and
            // `deep_merge_value` only overlays object members onto it.
            _ => HashMap::new(),
        }
    }

    pub fn mount(&mut self, namespace: &str, source: MountSource) -> Result<(), ModuleError> {
        self.generation += 1;
        // W-2: Reject reserved namespace per §9.7 spec.
        if namespace == "_config" {
            return Err(ModuleError::config_mount_error(
                namespace,
                "cannot mount to reserved namespace '_config'",
            ));
        }
        let data = match source {
            MountSource::Dict(v) => v,
            MountSource::File(path) => {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| ModuleError::config_mount_error(namespace, &e.to_string()))?;
                serde_yaml::from_str(&content)
                    .map_err(|e| ModuleError::config_mount_error(namespace, &e.to_string()))?
            }
        };
        if !data.is_object() {
            return Err(ModuleError::config_mount_error(
                namespace,
                "mount source must be a JSON object",
            ));
        }
        self.apply_mount(namespace, &data);
        // §9.11: mounts survive reload. Record the *resolved* data (not the
        // `MountSource`) so a replay after `reload()` does not re-read a file
        // that may have changed or disappeared.
        self.mounts.push((namespace.to_string(), data));
        Ok(())
    }

    /// Deep-merge an already-resolved mount payload into `user_namespaces`.
    ///
    /// Sync CB-002: deep-merge so peer keys in nested objects are preserved
    /// rather than overwritten. Mirrors apcore-python's `_deep_merge_dicts`
    /// (config.py) and apcore-typescript's `deepMerge`. Without this,
    /// `mount({db:{host:'a'}})` over `{db:{port:5432}}` would discard `port`.
    fn apply_mount(&mut self, namespace: &str, data: &serde_json::Value) {
        let entry = self
            .user_namespaces
            .entry(namespace.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let (Some(target), Some(source_map)) = (entry.as_object_mut(), data.as_object()) {
            for (k, v) in source_map {
                match target.get_mut(k) {
                    Some(existing) => {
                        deep_merge_value(existing, v);
                    }
                    None => {
                        target.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    pub fn bind<T: DeserializeOwned>(&self, namespace: &str) -> Result<T, ModuleError> {
        // Special-case canonical namespaces so `bind::<ExecutorConfig>("executor")`
        // returns the typed struct directly.
        match namespace {
            // Issue #34: bind the reconciled VIEW rather than the typed struct
            // alone, so `bind` agrees with `get("executor")` and
            // `namespace("executor")`. `bind::<ExecutorConfig>` is unchanged
            // (the struct ignores extra members); a caller binding their own
            // type over a `mount("executor", …)` payload previously could not
            // see it.
            EXECUTOR_NS => {
                return serde_json::from_value(self.executor_view())
                    .map_err(|e| ModuleError::config_bind_error(namespace, &e.to_string()))
            }
            // Issue #33: bind the reconciled VIEW, not the typed struct alone.
            // `bind::<ObservabilityConfig>` is unaffected (the struct ignores
            // the extra members), but a caller binding their own type — the
            // whole point of `bind` — previously received a payload from which
            // `redaction`, `logging`, `error_history` and `platform_notify` had
            // been stripped, and could not tell that from "unconfigured".
            OBSERVABILITY_NS => {
                return serde_json::from_value(self.observability_view())
                    .map_err(|e| ModuleError::config_bind_error(namespace, &e.to_string()))
            }
            _ => {}
        }

        // Bind from the merged namespace view (registered defaults + loaded
        // YAML/env), not the raw `user_namespaces` entry. `namespace()` overlays
        // loaded values onto registered defaults, so a registered default absent
        // from YAML is still present in the bound struct. Mirrors apcore-python
        // (`_instantiate_model(model, self.namespace(name), name)`) and
        // apcore-typescript (`bind` over the merged namespace).
        //
        // Sync finding A-D-018: an unregistered namespace with no data yields an
        // empty object, so `T`'s serde defaults take effect (matching
        // `_instantiate_model(model, {}, namespace)` / `new schema({})`) rather
        // than the old ConfigBindError("namespace not found").
        let merged: serde_json::Map<String, serde_json::Value> =
            self.namespace(namespace).into_iter().collect();
        serde_json::from_value(serde_json::Value::Object(merged))
            .map_err(|e| ModuleError::config_bind_error(namespace, &e.to_string()))
    }

    pub fn get_typed<T: DeserializeOwned>(&self, key: &str) -> Result<T, ModuleError> {
        let value = self
            .get(key)
            .ok_or_else(|| ModuleError::config_bind_error(key, "key not found"))?;
        serde_json::from_value(value)
            .map_err(|e| ModuleError::config_bind_error(key, &e.to_string()))
    }

    // --- Private helpers ---

    fn detect_mode(&mut self) {
        // W-3: Only activate namespace mode when "apcore" key is a mapping.
        // A null or scalar value is not a valid namespace indicator.
        self.mode = match self.user_namespaces.get("apcore") {
            Some(serde_json::Value::Object(_)) => ConfigMode::Namespace,
            _ => ConfigMode::Legacy,
        };
    }

    /// Apply APCORE_* environment variable overrides to both typed fields and settings.
    ///
    /// In legacy mode, all `APCORE_*` vars are mapped via `env_key_to_dot_path`.
    /// In namespace mode, registered `env_prefix` values are dispatched via
    /// longest-prefix-match (§9.10).
    fn apply_env_overrides(&mut self) {
        if self.mode == ConfigMode::Namespace {
            self.apply_namespace_env_overrides();
            return;
        }
        // Legacy mode: global env_map (bare env var → dot-path) takes
        // precedence, then flat APCORE_ prefix stripping. A-D-007: previously
        // the legacy branch ignored the global env_map entirely; apcore-python
        // (`_apply_env_overrides`, config.py:266) and apcore-typescript
        // (config.ts:240) consult it in every mode, so a `Config::env_map`
        // registration was silently dropped in legacy mode.
        let gmap = global_env_map().read();
        for (key, value) in std::env::vars() {
            let parsed = Self::coerce_env_value(&value);

            // 1. Global env_map (bare env var → top-level/dot-path key).
            if let Some(config_key) = gmap.get(&key) {
                tracing::debug!(env = %key, path = %config_key, "Applying legacy env override (global env_map)");
                self.apply_one_env_override(&key, config_key, parsed);
                continue;
            }

            // 2. Standard APCORE_ prefix stripping.
            //
            // apcore#88: the file selector is consumed by
            // `discover_config_file`; it is an argument to load(), not a value
            // the document declares. Kept here it would inject `config.file`.
            if key == ENV_CONFIG_FILE {
                continue;
            }
            if let Some(suffix) = key.strip_prefix("APCORE_") {
                let dot_path = Self::env_key_to_dot_path(suffix);
                tracing::debug!(env = %key, path = %dot_path, "Applying legacy env override");
                self.apply_one_env_override(&key, &dot_path, parsed);
            }
        }
    }

    /// §9.10: Namespace-aware env routing via longest-prefix-match.
    fn apply_namespace_env_overrides(&mut self) {
        let registry = global_ns_registry().read();
        let gmap = global_env_map().read();

        // Build namespace env_map lookup.
        let mut ns_env_maps: HashMap<&str, (&str, &str)> = HashMap::new();
        for reg in registry.values() {
            if let Some(ref em) = reg.env_map {
                for (env_var, config_key) in em {
                    ns_env_maps.insert(env_var.as_str(), (reg.name.as_str(), config_key.as_str()));
                }
            }
        }

        // Prefix table: sorted by length descending for longest-prefix-match.
        let mut prefixed: Vec<&NamespaceRegistration> = registry
            .values()
            .filter(|r| r.env_prefix.is_some())
            .collect();
        prefixed.sort_by(|a, b| {
            b.env_prefix
                .as_ref()
                .map_or(0, std::string::String::len)
                .cmp(&a.env_prefix.as_ref().map_or(0, std::string::String::len))
        });

        for (env_key, env_value) in std::env::vars() {
            let parsed = Self::coerce_env_value(&env_value);

            // 1. Global env_map (bare env var → top-level key).
            if let Some(config_key) = gmap.get(&env_key) {
                self.apply_one_env_override(&env_key, config_key, parsed);
                continue;
            }

            // 2. Namespace env_map (bare env var → namespace key).
            if let Some(&(ns_name, config_key)) = ns_env_maps.get(env_key.as_str()) {
                let full_path = format!("{ns_name}.{config_key}");
                self.apply_one_env_override(&env_key, &full_path, parsed);
                continue;
            }

            // 3. Prefix-based dispatch.
            let mut matched = false;
            for reg in &prefixed {
                let prefix = reg.env_prefix.as_deref().unwrap_or("");
                if let Some(suffix) = env_key.strip_prefix(prefix) {
                    let suffix = suffix.strip_prefix('_').unwrap_or(suffix);
                    if suffix.is_empty() {
                        continue;
                    }
                    let key = Self::resolve_env_suffix(suffix, reg);
                    let full_path = format!("{}.{key}", reg.name);
                    tracing::debug!(env = %env_key, path = %full_path, "Applying namespace env override");
                    self.apply_one_env_override(&env_key, &full_path, parsed.clone());
                    matched = true;
                    break;
                }
            }
            // Fallback: APCORE_ prefix with no matching namespace → treat as
            // top-level key (same as legacy mode). Per spec §9.8, un-matched
            // env vars resolve to their natural dot-path without namespace prefix.
            if !matched && env_key != ENV_CONFIG_FILE {
                // apcore#88: same exemption as the legacy branch — the file
                // selector is an argument to load(), not configuration.
                if let Some(suffix) = env_key.strip_prefix("APCORE_") {
                    let dot_path = format!("apcore.{}", Self::env_key_to_dot_path(suffix));
                    tracing::debug!(env = %env_key, path = %dot_path, "Applying fallback env override (no namespace match)");
                    self.apply_one_env_override(&env_key, &dot_path, parsed);
                }
            }
        }
    }

    /// Whether `key` names one of the path-typed configuration keys (§9.2.1).
    ///
    /// Compared against [`Self::path_typed_keys`] with the list marker
    /// stripped, so `extensions.roots` matches the `extensions.roots[]` entry
    /// the set publishes.
    fn is_path_typed_key(key: &str) -> bool {
        Self::path_typed_keys()
            .iter()
            .any(|declared| declared.trim_end_matches("[]") == key)
    }

    /// Apply one resolved environment override, discarding an **empty**
    /// path-typed value (PROTOCOL_SPEC §9.2.1 requirement 5).
    ///
    /// **An empty string is not a path.** §9.2 counts a *set but empty*
    /// `APCORE_*` variable as an override, so `export APCORE_ACL_ROOT=` — and a
    /// variable inherited empty from a container spec, which is the same thing
    /// to the tooling — used to blank out a directory the configuration file
    /// correctly declared. The `""` that replaced it is a legal relative path
    /// to every filesystem API and resolves to the working directory, so the
    /// failure was silent: the config said `./acl`, the process looked in `.`,
    /// and nothing reported a difference. The same shape is on record for
    /// `APCORE_CONFIG_FILE` (apcore#88), where an empty value injected a
    /// phantom `config.file` key.
    ///
    /// The value is therefore dropped and resolution falls through to the next
    /// tier — configuration file, then canonical default — exactly as if the
    /// variable had never been set. §9.2.1 permits a warning naming the key,
    /// which is emitted here rather than swallowed: an operator who exported
    /// the variable meant something by it.
    ///
    /// **Applied here, once, rather than at each consumer.** This is the single
    /// point where an environment tier becomes a configuration value, so every
    /// present and future reader of a path-typed key inherits the guard without
    /// having to know it exists. A per-consumer check is the arrangement that
    /// produced the defect: `ACL::discover` and `SchemaLoader` each decide what
    /// to do with an empty root, and a consumer added tomorrow decides again.
    ///
    /// Non-path-typed keys are untouched. An empty `APCORE_LOGGING_LEVEL` is a
    /// value the operator can mean, and §9.2.1's requirement is scoped to the
    /// closed path-typed set precisely because `""` is meaningless only there.
    fn apply_one_env_override(&mut self, env_key: &str, dot_path: &str, value: serde_json::Value) {
        let empty_path =
            Self::is_path_typed_key(dot_path.strip_prefix("apcore.").unwrap_or(dot_path))
                && matches!(&value, serde_json::Value::String(s) if s.is_empty());
        if empty_path {
            tracing::warn!(
                env = %env_key,
                key = %dot_path,
                "[apcore] `{env_key}` is set but empty, and `{dot_path}` is a path-typed \
                 configuration key (spec §9.2.1). An empty string is not a path, so the \
                 override is DISCARDED and `{dot_path}` falls through to the configuration \
                 file and then to its canonical default. Unset the variable to silence this, \
                 or give it the directory you meant."
            );
            return;
        }
        self.set(dot_path, value);
    }

    /// Map a canonical dot-path key to a typed field value.
    ///
    /// Recognizes only the canonical `<namespace>.<field>` form per spec §9.1.
    /// Legacy bare-name aliases are NOT accepted.
    fn get_typed_field(&self, key: &str) -> Option<serde_json::Value> {
        match key {
            "executor.max_call_depth" => Some(serde_json::Value::Number(
                self.executor.max_call_depth.into(),
            )),
            "executor.max_module_repeat" => Some(serde_json::Value::Number(
                self.executor.max_module_repeat.into(),
            )),
            "executor.default_timeout" => Some(serde_json::Value::Number(
                self.executor.default_timeout.into(),
            )),
            "executor.global_timeout" => Some(serde_json::Value::Number(
                self.executor.global_timeout.into(),
            )),
            "observability.tracing.enabled" => {
                Some(serde_json::Value::Bool(self.observability.tracing.enabled))
            }
            "observability.tracing.sampling_rate" => {
                serde_json::Number::from_f64(self.observability.tracing.sampling_rate)
                    .map(serde_json::Value::Number)
            }
            "observability.tracing.exporter" => Some(serde_json::Value::String(
                self.observability.tracing.exporter.clone(),
            )),
            "observability.tracing.strategy" => Some(serde_json::Value::String(
                self.observability.tracing.strategy.clone(),
            )),
            "observability.tracing.otlp_endpoint" => Some(
                self.observability
                    .tracing
                    .otlp_endpoint
                    .clone()
                    .map_or(serde_json::Value::Null, serde_json::Value::String),
            ),
            "observability.metrics.enabled" => {
                Some(serde_json::Value::Bool(self.observability.metrics.enabled))
            }
            "modules_path" => self
                .modules_path
                .as_ref()
                .map(|p| serde_json::Value::String(p.to_string_lossy().into_owned())),
            _ => None,
        }
    }

    /// Try to set a canonical typed field. Returns true if matched.
    /// Mirror a typed-field write into `user_namespaces`, the as-written store.
    ///
    /// `Config::deserialize` already does this for every name in
    /// [`TYPED_FRAMEWORK_SECTIONS`] — the raw object is retained beside the
    /// typed struct and the two are reconciled with the typed struct overlaid
    /// last. The `set()` path did not, so the same framework section had an
    /// as-written record when it came from a file and none when it came from
    /// `set()` or an `APCORE_*` override, and [`Self::get_declared`] could not
    /// tell "the document declared this" from "the struct has a default"
    /// (aiperceivable/apcore#119).
    ///
    /// Writing the raw copy is idempotent against the reconciled view: the
    /// overlay puts the typed value on top of a raw value it just equalled.
    fn record_typed_as_written(&mut self, key: &str, value: &serde_json::Value) {
        let parts: Vec<&str> = key.split('.').collect();
        if parts.len() == 1 {
            self.user_namespaces.insert(key.to_string(), value.clone());
            return;
        }
        let mut current = self
            .user_namespaces
            .entry(parts[0].to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        for part in &parts[1..parts.len() - 1] {
            if !current.is_object() {
                *current = serde_json::Value::Object(serde_json::Map::new());
            }
            // INVARIANT: the branch above guarantees object shape.
            current = current
                .as_object_mut()
                .unwrap()
                .entry((*part).to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        }
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        // INVARIANT: as above.
        current
            .as_object_mut()
            .unwrap()
            .insert(parts[parts.len() - 1].to_string(), value.clone());
    }

    fn set_typed_field(&mut self, key: &str, value: &serde_json::Value) -> bool {
        match key {
            "executor.max_call_depth" => {
                if let Some(n) = value.as_u64() {
                    #[allow(clippy::cast_possible_truncation)]
                    // config values are small and won't exceed u32::MAX
                    {
                        self.executor.max_call_depth = n as u32;
                    }
                    return true;
                }
            }
            "executor.max_module_repeat" => {
                if let Some(n) = value.as_u64() {
                    #[allow(clippy::cast_possible_truncation)]
                    // config values are small and won't exceed u32::MAX
                    {
                        self.executor.max_module_repeat = n as u32;
                    }
                    return true;
                }
            }
            "executor.default_timeout" => {
                if let Some(n) = value.as_u64() {
                    self.executor.default_timeout = n;
                    return true;
                }
            }
            "executor.global_timeout" => {
                if let Some(n) = value.as_u64() {
                    self.executor.global_timeout = n;
                    return true;
                }
            }
            "observability.tracing.enabled" => {
                if let Some(b) = value.as_bool() {
                    self.observability.tracing.enabled = b;
                    return true;
                }
            }
            "observability.tracing.sampling_rate" => {
                if let Some(n) = value.as_f64() {
                    self.observability.tracing.sampling_rate = n;
                    return true;
                }
            }
            "observability.tracing.exporter" => {
                if let Some(s) = value.as_str() {
                    self.observability.tracing.exporter = s.to_string();
                    return true;
                }
            }
            "observability.tracing.strategy" => {
                if let Some(s) = value.as_str() {
                    self.observability.tracing.strategy = s.to_string();
                    return true;
                }
            }
            "observability.tracing.otlp_endpoint" => {
                if value.is_null() {
                    self.observability.tracing.otlp_endpoint = None;
                    return true;
                }
                if let Some(s) = value.as_str() {
                    self.observability.tracing.otlp_endpoint = Some(s.to_string());
                    return true;
                }
            }
            "observability.metrics.enabled" => {
                if let Some(b) = value.as_bool() {
                    self.observability.metrics.enabled = b;
                    return true;
                }
            }
            "modules_path" => {
                if let Some(s) = value.as_str() {
                    self.modules_path = Some(PathBuf::from(s));
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// Convert an env-var suffix to a dot-path config key.
    ///
    /// Convention (matches Python reference):
    ///   - Single `_` → `.` (section separator)
    ///   - Double `__` → literal `_` (underscore within a field name)
    ///
    /// Example: `EXECUTOR_MAX__CALL__DEPTH` → `executor.max_call_depth`
    ///
    /// So to set `max_call_depth` via env, use `APCORE_EXECUTOR_MAX__CALL__DEPTH`.
    fn env_key_to_dot_path(raw: &str) -> String {
        Self::env_key_to_dot_path_with_depth(raw, usize::MAX)
    }

    /// Convert env var suffix to dot-path, stopping at `max_depth` segments.
    fn env_key_to_dot_path_with_depth(raw: &str, max_depth: usize) -> String {
        let lower = raw.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();
        let mut result = String::with_capacity(chars.len());
        let mut dot_count: usize = 0;
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '_' {
                if i + 1 < chars.len() && chars[i + 1] == '_' {
                    result.push('_'); // double __ → literal _
                    i += 2;
                } else if dot_count < max_depth.saturating_sub(1) {
                    result.push('.');
                    dot_count += 1;
                    i += 1;
                } else {
                    result.push('_'); // depth limit reached
                    i += 1;
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    /// Try to match suffix against keys in a JSON object tree (recursive).
    fn match_suffix_to_tree(
        suffix: &str,
        tree: &serde_json::Map<String, serde_json::Value>,
        depth: usize,
        max_depth: usize,
    ) -> Option<String> {
        // 1. Try full suffix as a flat key.
        if tree.contains_key(suffix) {
            return Some(suffix.to_string());
        }
        // 2. Depth limit.
        if depth >= max_depth.saturating_sub(1) {
            return None;
        }
        // 3. Try splitting at each underscore.
        for (i, ch) in suffix.char_indices() {
            if ch != '_' || i == 0 || i == suffix.len() - 1 {
                continue;
            }
            let prefix_part = &suffix[..i];
            let remainder = &suffix[i + 1..];
            if let Some(serde_json::Value::Object(subtree)) = tree.get(prefix_part) {
                if let Some(sub) =
                    Self::match_suffix_to_tree(remainder, subtree, depth + 1, max_depth)
                {
                    return Some(format!("{prefix_part}.{sub}"));
                }
            }
        }
        None
    }

    /// Resolve an env var suffix based on the registration's `env_style`.
    fn resolve_env_suffix(suffix: &str, reg: &NamespaceRegistration) -> String {
        match reg.env_style {
            EnvStyle::Flat => suffix.to_lowercase(),
            EnvStyle::Auto => {
                let lower = suffix.to_lowercase();
                if let Some(serde_json::Value::Object(tree)) = reg.defaults.as_ref() {
                    if let Some(resolved) =
                        Self::match_suffix_to_tree(&lower, tree, 0, reg.max_depth)
                    {
                        return resolved;
                    }
                }
                // Fall back to nested with depth.
                Self::env_key_to_dot_path_with_depth(suffix, reg.max_depth)
            }
            EnvStyle::Nested => Self::env_key_to_dot_path_with_depth(suffix, reg.max_depth),
        }
    }

    fn coerce_env_value(value: &str) -> serde_json::Value {
        if value.eq_ignore_ascii_case("true") {
            return serde_json::Value::Bool(true);
        }
        if value.eq_ignore_ascii_case("false") {
            return serde_json::Value::Bool(false);
        }
        if let Ok(n) = value.parse::<i64>() {
            return serde_json::Value::Number(n.into());
        }
        if let Ok(f) = value.parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return serde_json::Value::Number(n);
            }
        }
        serde_json::Value::String(value.to_string())
    }
}

// ---------------------------------------------------------------------------
// Built-in namespace initialization (§9.15)
// ---------------------------------------------------------------------------

fn init_builtin_namespaces() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let namespaces = vec![
            NamespaceRegistration {
                name: "observability".to_string(),
                env_prefix: Some("APCORE_OBSERVABILITY".to_string()),
                // PROTOCOL_SPEC §9.15.2: `tracing` and `metrics`, the two blocks
                // `$defs/ObservabilityConfig` declares, and nothing else (D-144).
                // The live equivalents of blocks other SDKs once registered here
                // are `sys_modules.error_history.*`,
                // `sys_modules.events.thresholds.*` and `obs.redaction.*`.
                defaults: Some(serde_json::json!({
                    "tracing": {
                        "enabled": false,
                        "strategy": "full",
                        "sampling_rate": 1.0,
                        "exporter": "stdout",
                        "otlp_endpoint": null
                    },
                    "metrics": {
                        "enabled": false,
                        "exporter": "stdout"
                    }
                })),
                schema: None,
                env_style: EnvStyle::Auto,
                max_depth: DEFAULT_MAX_DEPTH,
                env_map: None,
            },
            NamespaceRegistration {
                name: "sys_modules".to_string(),
                env_prefix: Some("APCORE_SYS".to_string()),
                // Activation is off by default: PROTOCOL_SPEC §6.6.3 states
                // `sys_modules.enabled = false (default)` -> 0 modules
                // registered, and schemas/sys-modules.schema.json declares
                // `default: false`. The per-module sub-flags stay true: they
                // select WHICH modules register once activation has happened.
                defaults: Some(serde_json::json!({
                    "enabled": false,
                    "health": { "enabled": true },
                    "manifest": { "enabled": true },
                    "usage": { "enabled": true, "retention_hours": 168, "bucketing_strategy": "hourly" },
                    "control": { "enabled": true },
                    // `error_history` is declared with defaults by
                    // schemas/sys-modules.schema.json and was missing here.
                    // `control.overrides_path` is the one deliberate omission:
                    // its declared default is null, which a namespace default
                    // cannot express distinctly from absence (A-D-021).
                    "error_history": {
                        "max_entries_per_module": 50,
                        "max_total_entries": 1000
                    },
                    "events": {
                        "enabled": false,
                        "subscribers": [],
                        "thresholds": { "error_rate": 0.1, "latency_p99_ms": 5000.0 }
                    }
                })),
                schema: None,
                env_style: EnvStyle::Auto,
                max_depth: DEFAULT_MAX_DEPTH,
                env_map: None,
            },
            // PROTOCOL_SPEC §9.15.4 (D-144): the namespace carrying the
            // configured redaction rules of §10.6.1, declared by
            // `$defs/ObsConfig`.
            NamespaceRegistration {
                name: "obs".to_string(),
                env_prefix: Some("APCORE_OBS".to_string()),
                defaults: Some(serde_json::json!({
                    "redaction": {
                        "regex_patterns": [],
                        "sensitive_keys": crate::observability::redaction::DEFAULT_SENSITIVE_KEYS,
                        "replacement": crate::observability::redaction::DEFAULT_REPLACEMENT
                    }
                })),
                schema: None,
                env_style: EnvStyle::Auto,
                max_depth: DEFAULT_MAX_DEPTH,
                env_map: None,
            },
        ];
        for ns in namespaces {
            // Ignore duplicate errors on re-init
            let _ = Config::register_namespace(ns);
        }
    });
}

// ---------------------------------------------------------------------------
// Config discovery (§9.14)
// ---------------------------------------------------------------------------

/// `$APCORE_CONFIG_FILE` is *consumed* here: both env-override passes skip it
/// so it never becomes the `config.file` override (apcore#88).
fn discover_config_file() -> Option<std::path::PathBuf> {
    if let Ok(env_path) = std::env::var(ENV_CONFIG_FILE) {
        if !env_path.is_empty() {
            return Some(std::path::PathBuf::from(env_path));
        }
    }

    let cwd_candidates = ["project.yaml", "project.yml", "apcore.yaml", "apcore.yml"];
    for name in &cwd_candidates {
        let p = std::path::Path::new(name);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    user_level_config_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
}

/// The §9.14 **tier 6-7** candidates, in search order: the user-level config
/// locations.
///
/// Tier 6 is the XDG-style path — `~/Library/Application Support/apcore/` on
/// macOS, `~/.config/apcore/` elsewhere — and tier 7 the legacy `~/.apcore/`.
///
/// Split out of [`discover_config_file`] so [`is_user_level_config_path`] can
/// answer "did this config come from a user-level tier?" against exactly the
/// set discovery searches, rather than a second hand-maintained copy that could
/// drift from it.
fn user_level_config_candidates() -> Vec<std::path::PathBuf> {
    let Some(home) = dirs_home() else {
        return Vec::new();
    };

    #[cfg(target_os = "macos")]
    let xdg = home
        .join("Library")
        .join("Application Support")
        .join("apcore")
        .join("config.yaml");
    #[cfg(not(target_os = "macos"))]
    let xdg = home.join(".config").join("apcore").join("config.yaml");

    vec![xdg, home.join(".apcore").join("config.yaml")]
}

/// Whether `path` names one of the §9.14 tier 6-7 user-level config locations.
///
/// This is the tier test [`Config::project_root`] needs, and it is answered by
/// **location** rather than by recording which discovery branch fired. The two
/// differ in exactly one case: `$APCORE_CONFIG_FILE` (tier 1) pointing straight
/// at the user-level file, which this reports as user-level. That is the
/// answer #113 wants either way — the reason tiers 6-7 anchor at CWD is that a
/// per-user config's relative paths are per-*project* by intent, and that is a
/// property of where the document lives, not of how it was found.
///
/// Paths are compared after canonicalization where possible, so `~/.apcore/`
/// reached through a symlink still matches.
fn is_user_level_config_path(path: &std::path::Path) -> bool {
    let canonical = path.canonicalize();
    let target = canonical.as_deref().unwrap_or(path);
    user_level_config_candidates().iter().any(|candidate| {
        candidate
            .canonicalize()
            .map_or_else(|_| candidate == path, |c| c == target)
    })
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}

/// Recursively merge `overlay` into `base`, preserving peer keys in nested
/// objects. Used by `Config::mount` (sync CB-002) to mirror Python's
/// `_deep_merge_dicts` semantics.
fn deep_merge_value(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                match base_map.get_mut(k) {
                    Some(existing) => deep_merge_value(existing, v),
                    None => {
                        base_map.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, value) => {
            *slot = value.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Config::default and ExecutorConfig defaults
    // -------------------------------------------------------------------------

    #[test]
    fn default_config_has_expected_executor_values() {
        let cfg = Config::default();
        assert_eq!(cfg.executor.max_call_depth, 32);
        assert_eq!(cfg.executor.max_module_repeat, 3);
        assert_eq!(cfg.executor.default_timeout, 30_000);
        assert_eq!(cfg.executor.global_timeout, 60_000);
    }

    #[test]
    fn bare_default_config_fails_required_field_check() {
        // Canonical contract (A-D-03): a bare struct-`default()` Config is in
        // legacy mode but carries none of the spec-mandated required fields
        // (version, project.name, extensions.root, schema.root,
        // acl.default_effect). It MUST now be rejected with CONFIG_INVALID.
        // (Previously this asserted the old lax behavior where it passed.)
        let cfg = Config::default();
        let result = cfg.validate();
        assert!(
            result.is_err(),
            "bare default config lacks required fields and must fail validation"
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    // -------------------------------------------------------------------------
    // Config::get / set for canonical typed fields
    // -------------------------------------------------------------------------

    #[test]
    fn get_canonical_executor_key() {
        let cfg = Config::default();
        let depth = cfg
            .get("executor.max_call_depth")
            .expect("key should exist");
        assert_eq!(depth, serde_json::json!(32u64));
    }

    #[test]
    fn set_then_get_canonical_executor_key() {
        let mut cfg = Config::default();
        cfg.set("executor.max_call_depth", serde_json::json!(10u64));
        let val = cfg.get("executor.max_call_depth").unwrap();
        assert_eq!(val.as_u64().unwrap(), 10);
    }

    #[test]
    fn get_observability_tracing_enabled() {
        let cfg = Config::default();
        let enabled = cfg.get("observability.tracing.enabled").unwrap();
        // Default is false
        assert_eq!(enabled, serde_json::json!(false));
    }

    #[test]
    fn set_observability_tracing_enabled() {
        let mut cfg = Config::default();
        cfg.set("observability.tracing.enabled", serde_json::json!(true));
        assert!(cfg.observability.tracing.enabled);
    }

    // -------------------------------------------------------------------------
    // Config::get / set for user namespaces (dot-path traversal)
    // -------------------------------------------------------------------------

    #[test]
    fn set_and_get_user_namespace_key() {
        let mut cfg = Config::default();
        cfg.set(
            "myapp.db.url",
            serde_json::json!("postgres://localhost/test"),
        );
        let val = cfg.get("myapp.db.url").expect("should exist");
        assert_eq!(val.as_str().unwrap(), "postgres://localhost/test");
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        let cfg = Config::default();
        assert!(cfg.get("nonexistent.key").is_none());
    }

    #[test]
    fn set_top_level_user_namespace_key() {
        let mut cfg = Config::default();
        cfg.set("myns", serde_json::json!("value"));
        assert_eq!(cfg.get("myns").unwrap(), serde_json::json!("value"));
    }

    // -------------------------------------------------------------------------
    // Config::validate
    // -------------------------------------------------------------------------

    #[test]
    fn validate_rejects_zero_max_call_depth() {
        let mut cfg = Config::default();
        cfg.executor.max_call_depth = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_max_module_repeat() {
        let mut cfg = Config::default();
        cfg.executor.max_module_repeat = 0;
        assert!(cfg.validate().is_err());
    }

    /// `global_timeout` below `default_timeout` is a VALID configuration:
    /// `builtin_steps.rs` clamps the per-module timeout to the remaining global
    /// deadline, so it reads as "no single module over 30s, whole chain under
    /// 10s". Neither peer nor the PROTOCOL_SPEC §9.3 constraint table rejects
    /// it, so neither does Rust.
    #[test]
    fn validate_allows_global_timeout_less_than_default_timeout() {
        let mut json = valid_legacy_config_json();
        json["executor"]["global_timeout"] = serde_json::json!(10_000);
        json["executor"]["default_timeout"] = serde_json::json!(30_000);
        let cfg = config_from_json(&json);
        assert!(
            cfg.validate().is_ok(),
            "the runtime clamp makes this configuration meaningful: {:?}",
            cfg.validate()
        );
    }

    #[test]
    fn validate_allows_zero_global_timeout_meaning_no_deadline() {
        // Build a complete, valid legacy config, then set global_timeout = 0
        // (0 = no global deadline). This must still pass. Uses a populated
        // config because required-field enforcement (A-D-03) now rejects bare
        // defaults that lack version/project.
        let mut json = valid_legacy_config_json();
        json["executor"]["global_timeout"] = serde_json::json!(0);
        let cfg = config_from_json(&json);
        assert!(
            cfg.validate().is_ok(),
            "zero global_timeout (no deadline) must be allowed: {:?}",
            cfg.validate()
        );
    }

    // -------------------------------------------------------------------------
    // Config deserialization — legacy field rejection
    // -------------------------------------------------------------------------

    #[test]
    fn deserialize_rejects_legacy_root_fields() {
        let json_str = r#"{"max_call_depth": 10}"#;
        let result: Result<Config, _> = serde_json::from_str(json_str);
        assert!(result.is_err(), "legacy root field should be rejected");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("v0.18.0") || err_msg.contains("max_call_depth"),
            "error should mention legacy key"
        );
    }

    #[test]
    fn deserialize_canonical_format_succeeds() {
        let json_str = r#"{"executor": {"max_call_depth": 16}}"#;
        let cfg: Config = serde_json::from_str(json_str).expect("canonical format should work");
        assert_eq!(cfg.executor.max_call_depth, 16);
    }

    // -------------------------------------------------------------------------
    // Config::data
    // -------------------------------------------------------------------------

    #[test]
    fn data_returns_json_object() {
        let cfg = Config::default();
        let data = cfg.data();
        assert!(data.is_object(), "data() should return a JSON object");
        assert!(data.get("executor").is_some());
    }

    // -------------------------------------------------------------------------
    // Config::reload without path
    // -------------------------------------------------------------------------

    #[test]
    fn reload_without_path_returns_error() {
        let mut cfg = Config::default();
        assert!(
            cfg.reload().is_err(),
            "reload without yaml_path should fail"
        );
    }

    // -------------------------------------------------------------------------
    // Config::mount
    // -------------------------------------------------------------------------

    #[test]
    fn mount_dict_into_user_namespace() {
        let mut cfg = Config::default();
        let data = serde_json::json!({"host": "localhost", "port": 5432});
        cfg.mount("database", MountSource::Dict(data)).unwrap();
        let host = cfg.get("database.host").unwrap();
        assert_eq!(host.as_str().unwrap(), "localhost");
    }

    #[test]
    fn mount_rejects_reserved_namespace() {
        let mut cfg = Config::default();
        let data = serde_json::json!({"key": "value"});
        let result = cfg.mount("_config", MountSource::Dict(data));
        assert!(
            result.is_err(),
            "should reject reserved namespace '_config'"
        );
    }

    #[test]
    fn mount_rejects_non_object_source() {
        let mut cfg = Config::default();
        let result = cfg.mount("ns", MountSource::Dict(serde_json::json!([1, 2, 3])));
        assert!(result.is_err(), "non-object source should be rejected");
    }

    // -------------------------------------------------------------------------
    // ConfigMode detection
    // -------------------------------------------------------------------------

    #[test]
    fn namespace_mode_detected_when_apcore_key_present() {
        let json_str = r#"{"apcore": {"executor": {"max_call_depth": 8}}}"#;
        let cfg: Config = serde_json::from_str(json_str).expect("should parse");
        // detect_mode() is called in from_yaml_file / from_json_file / from_defaults;
        // when deserializing raw, mode stays Legacy; we call detect_mode via from_defaults
        // which relies on from_defaults path. Test via from_defaults behavior:
        // Just verify the config parsed correctly.
        assert_eq!(cfg.executor.max_call_depth, 8);
    }

    // -------------------------------------------------------------------------
    // A-D-02: namespace-mode validation (strict unknown-namespace rejection
    // and registered-namespace schema validation).
    // -------------------------------------------------------------------------

    #[test]
    fn strict_mode_rejects_unknown_namespace() {
        // Namespace mode (apcore key present) + _config.strict=true +
        // an unregistered top-level namespace MUST fail with CONFIG_INVALID.
        let json_str = r#"{
            "apcore": {},
            "_config": {"strict": true},
            "totally-unregistered-ns-ad02": {"foo": "bar"}
        }"#;
        let cfg: Config = serde_json::from_str(json_str).expect("should parse");
        assert_eq!(cfg.mode, ConfigMode::Namespace);
        let result = cfg.validate();
        assert!(
            result.is_err(),
            "strict mode must reject unknown namespace, got {result:?}"
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    // -------------------------------------------------------------------------
    // A-D-03: required-field enforcement + full constraint set (legacy mode).
    // Canonical contract: docs/features/config-bus.md "Contract: Config.validate".
    // -------------------------------------------------------------------------

    /// A complete, spec-valid legacy-mode config. Mutate a clone to test that a
    /// single violation is rejected while the baseline passes.
    fn valid_legacy_config_json() -> serde_json::Value {
        serde_json::json!({
            "version": "0.23.0",
            "project": { "name": "demo" },
            "extensions": { "root": "./extensions", "max_depth": 8 },
            "schema": { "root": "./schemas" },
            "acl": { "root": "./acl", "default_effect": "deny" },
            "executor": {
                "default_timeout": 30000,
                "global_timeout": 60000,
                "max_call_depth": 32,
                "max_module_repeat": 3
            },
            "observability": { "tracing": { "enabled": false, "sampling_rate": 1.0 } },
            "middleware": {
                "circuit_breaker": {
                    "open_threshold": 0.5,
                    "recovery_window_ms": 30000,
                    "window_size": 20,
                    "min_samples": 5
                }
            },
            "sys_modules": {
                "events": { "thresholds": { "error_rate": 0.1, "latency_p99_ms": 5000.0 } }
            }
        })
    }

    fn config_from_json(value: &serde_json::Value) -> Config {
        serde_json::from_value(value.clone()).expect("fixture should deserialize")
    }

    #[test]
    fn validate_accepts_fully_valid_legacy_config() {
        let cfg = config_from_json(&valid_legacy_config_json());
        assert!(
            cfg.validate().is_ok(),
            "fully-valid config must pass: {:?}",
            cfg.validate()
        );
    }

    #[test]
    fn validate_accepts_legacy_config_missing_acl_root_with_default() {
        // D-64 (Recommendation A): `acl.root` is no longer hard-required. A
        // config omitting it is VALID, and `get("acl.root")` resolves to the
        // canonical default `"./acl"` (matching apcore-python/-typescript).
        let mut json = valid_legacy_config_json();
        json["acl"].as_object_mut().unwrap().remove("root");
        let cfg = config_from_json(&json);
        assert!(
            cfg.validate().is_ok(),
            "config omitting acl.root must now be valid: {:?}",
            cfg.validate()
        );
        assert_eq!(
            cfg.get("acl.root"),
            Some(serde_json::json!("./acl")),
            "omitted acl.root must resolve to the default \"./acl\""
        );
    }

    /// PROTOCOL_SPEC §9.1: every key that carries a canonical default in
    /// `defaults.schema.json` is optional. Rust used to hard-require
    /// `extensions.root`, `schema.root` and `acl.default_effect`, rejecting
    /// documents that apcore-python and apcore-typescript accept.
    #[test]
    fn validate_accepts_legacy_config_missing_defaulted_keys() {
        let mut json = valid_legacy_config_json();
        json.as_object_mut().unwrap().remove("extensions");
        json.as_object_mut().unwrap().remove("schema");
        json.as_object_mut().unwrap().remove("acl");
        let cfg = config_from_json(&json);
        assert!(
            cfg.validate().is_ok(),
            "keys with canonical defaults must not be required: {:?}",
            cfg.validate()
        );
        assert_eq!(
            cfg.get("extensions.root"),
            Some(serde_json::json!("./extensions"))
        );
        assert_eq!(cfg.get("schema.root"), Some(serde_json::json!("./schemas")));
        assert_eq!(
            cfg.get("acl.default_effect"),
            Some(serde_json::json!("deny"))
        );
    }

    #[test]
    fn validate_rejects_each_missing_required_field() {
        // PROTOCOL_SPEC §9.1: a key is required only when it has no canonical
        // default. Exactly two qualify — `version` and `project.name`.
        // `extensions.root`, `schema.root`, `acl.root` and `acl.default_effect`
        // all carry defaults in `defaults.schema.json`, so their absence is
        // VALID (see `validate_accepts_legacy_config_missing_defaulted_keys`).
        let removals: &[(&str, &str)] = &[("version", ""), ("project", "name")];
        for (top, nested) in removals {
            let mut json = valid_legacy_config_json();
            if nested.is_empty() {
                json.as_object_mut().unwrap().remove(*top);
            } else {
                json[*top].as_object_mut().unwrap().remove(*nested);
            }
            let cfg = config_from_json(&json);
            let result = cfg.validate();
            let field = if nested.is_empty() {
                (*top).to_string()
            } else {
                format!("{top}.{nested}")
            };
            assert!(
                result.is_err(),
                "missing required field '{field}' must be rejected"
            );
            assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
        }
    }

    #[test]
    fn validate_accepts_circuit_breaker_open_threshold_default_rate() {
        // [review-followup] open_threshold is an ERROR RATE in [0.0, 1.0]
        // (default 0.5), not an integer count. The valid default must pass.
        let mut json = valid_legacy_config_json();
        json["middleware"]["circuit_breaker"]["open_threshold"] = serde_json::json!(0.5);
        let cfg = config_from_json(&json);
        assert!(
            cfg.validate().is_ok(),
            "open_threshold = 0.5 must be accepted (rate in [0,1]): {:?}",
            cfg.validate()
        );
    }

    #[test]
    fn validate_accepts_circuit_breaker_open_threshold_zero_rate() {
        // 0.0 is a valid error rate (matches apcore-python: open_threshold 0
        // accepted).
        let mut json = valid_legacy_config_json();
        json["middleware"]["circuit_breaker"]["open_threshold"] = serde_json::json!(0);
        let cfg = config_from_json(&json);
        assert!(
            cfg.validate().is_ok(),
            "open_threshold = 0 must be accepted (rate in [0,1]): {:?}",
            cfg.validate()
        );
    }

    #[test]
    fn validate_rejects_error_history_max_entries_per_module_zero() {
        let mut json = valid_legacy_config_json();
        json["sys_modules"]["error_history"] = serde_json::json!({ "max_entries_per_module": 0 });
        let cfg = config_from_json(&json);
        let result = cfg.validate();
        assert!(
            result.is_err(),
            "max_entries_per_module = 0 must be rejected"
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn validate_rejects_error_history_max_total_entries_zero() {
        let mut json = valid_legacy_config_json();
        json["sys_modules"]["error_history"] = serde_json::json!({ "max_total_entries": 0 });
        let cfg = config_from_json(&json);
        let result = cfg.validate();
        assert!(result.is_err(), "max_total_entries = 0 must be rejected");
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn validate_rejects_events_latency_p99_zero() {
        // latency_p99_ms must be a positive number (> 0), not merely >= 0.
        let mut json = valid_legacy_config_json();
        json["sys_modules"]["events"]["thresholds"]["latency_p99_ms"] = serde_json::json!(0);
        let cfg = config_from_json(&json);
        let result = cfg.validate();
        assert!(result.is_err(), "latency_p99_ms = 0 must be rejected (> 0)");
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn constrained_config_keys_matches_the_match_arms() {
        // CONSTRAINED_CONFIG_KEYS is hand-maintained beside a `match` that
        // cannot be enumerated. This keeps the two honest: every listed key must
        // actually carry a constraint.
        for key in super::CONSTRAINED_CONFIG_KEYS {
            assert!(
                Config::validate_key_constraint(key, &serde_json::json!(null)).is_some(),
                "{key} is listed in CONSTRAINED_CONFIG_KEYS but carries no constraint"
            );
        }
        // And a key that carries none must not be listed.
        assert_eq!(
            Config::validate_key_constraint("project.name", &serde_json::json!(null)),
            None
        );
        assert!(!super::CONSTRAINED_CONFIG_KEYS.contains(&"project.name"));
    }

    #[test]
    fn middleware_circuit_breaker_is_not_a_config_key() {
        // `apcore-config.schema.json` declares MiddlewareConfig as `{ disabled }`
        // with additionalProperties:false, so `middleware.circuit_breaker.*` was
        // rejected by the canonical config schema while all three SDKs validated
        // it — and no SDK ever read it. The breaker is configured through its
        // constructor options and the declarative middleware-chain config.
        for key in [
            "middleware.circuit_breaker.open_threshold",
            "middleware.circuit_breaker.recovery_window_ms",
            "middleware.circuit_breaker.window_size",
            "middleware.circuit_breaker.min_samples",
        ] {
            assert_eq!(
                Config::validate_key_constraint(key, &serde_json::json!(-1)),
                None,
                "{key} must carry no constraint — it is not a config key"
            );
        }
    }

    #[test]
    fn validate_rejects_events_error_rate_above_one() {
        let mut json = valid_legacy_config_json();
        json["sys_modules"]["events"]["thresholds"]["error_rate"] = serde_json::json!(1.5);
        let cfg = config_from_json(&json);
        let result = cfg.validate();
        assert!(result.is_err(), "error_rate = 1.5 must be rejected ([0,1])");
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn validate_rejects_bad_default_effect_value() {
        let mut json = valid_legacy_config_json();
        json["acl"]["default_effect"] = serde_json::json!("maybe");
        let cfg = config_from_json(&json);
        assert_eq!(cfg.validate().unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn validate_rejects_extensions_max_depth_zero() {
        let mut json = valid_legacy_config_json();
        json["extensions"]["max_depth"] = serde_json::json!(0);
        let cfg = config_from_json(&json);
        assert_eq!(cfg.validate().unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn validate_rejects_extensions_max_depth_above_range() {
        // [config-maxdepth-residual] validate()'s collect_constraint_errors must
        // use the [1, 16] range (matching validate_key_constraint and the spec),
        // not an unbounded >= 1 check.
        let mut json = valid_legacy_config_json();
        json["extensions"]["max_depth"] = serde_json::json!(17);
        let cfg = config_from_json(&json);
        assert_eq!(cfg.validate().unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn validate_namespace_mode_skips_required_fields() {
        // Per the anchor (apcore-python `_validate_namespace_mode`), namespace
        // mode runs constraints only — NOT required fields. A minimal
        // namespace-mode config (no version/project/etc.) must still pass.
        let json_str = r#"{ "apcore": {} }"#;
        let cfg: Config = serde_json::from_str(json_str).expect("should parse");
        assert_eq!(cfg.mode, ConfigMode::Namespace);
        assert!(
            cfg.validate().is_ok(),
            "namespace mode must not require legacy required fields: {:?}",
            cfg.validate()
        );
    }

    #[test]
    fn registered_namespace_with_schema_rejects_invalid_data() {
        // Register a namespace with a schema requiring `count` to be an integer.
        let ns_name = "schema-ns-ad02";
        let reg = NamespaceRegistration {
            name: ns_name.to_string(),
            env_prefix: None,
            defaults: None,
            schema: Some(serde_json::json!({
                "type": "object",
                "properties": {"count": {"type": "integer"}},
                "required": ["count"]
            })),
            env_style: EnvStyle::Auto,
            max_depth: DEFAULT_MAX_DEPTH,
            env_map: None,
        };
        // Ignore duplicate-registration error if a prior test registered it.
        let _ = Config::register_namespace(reg);

        // Invalid: `count` is a string, not an integer.
        let json_str = r#"{
            "apcore": {},
            "schema-ns-ad02": {"count": "not-an-integer"}
        }"#;
        let cfg: Config = serde_json::from_str(json_str).expect("should parse");
        assert_eq!(cfg.mode, ConfigMode::Namespace);
        let result = cfg.validate();
        assert!(
            result.is_err(),
            "registered namespace with invalid data must fail, got {result:?}"
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);

        // Sanity: valid data passes the same schema.
        let valid_str = r#"{
            "apcore": {},
            "schema-ns-ad02": {"count": 5}
        }"#;
        let cfg_ok: Config = serde_json::from_str(valid_str).expect("should parse");
        assert!(cfg_ok.validate().is_ok());
    }
    #[test]
    fn path_typed_keys_matches_the_declared_set_in_both_directions() {
        use std::collections::BTreeSet;
        let actual: BTreeSet<&str> = Config::path_typed_keys().iter().copied().collect();
        let expected: BTreeSet<&str> = [
            "acl.root",
            "bindings.dir",
            "extensions.root",
            "extensions.roots[]",
            "id_map.overrides",
            "schema.root",
            "sys_modules.control.overrides_path",
        ]
        .into_iter()
        .collect();
        let invented: Vec<_> = actual.difference(&expected).collect();
        let missing: Vec<_> = expected.difference(&actual).collect();
        assert!(invented.is_empty(), "keys the SDK invented: {invented:?}");
        assert!(missing.is_empty(), "keys the SDK is missing: {missing:?}");
    }

    #[test]
    fn bindings_pattern_is_not_path_typed() {
        // Discriminating case. It sits in the same section as `bindings.dir` and
        // its default (`*.binding.yaml`) looks like a filename, so an
        // implementation that classifies by section sweeps it in. It is a glob
        // matched WITHIN `bindings.dir`, never resolved as a path itself.
        assert!(!Config::path_typed_keys().contains(&"bindings.pattern"));
    }

    #[test]
    fn non_path_string_keys_are_not_path_typed() {
        // An implementation that marks every string key as path-typed passes any
        // presence-only assertion and fails here.
        for key in [
            "acl.default_effect",
            "schema.strategy",
            "logging.level",
            "observability.tracing.exporter",
            "project.name",
        ] {
            assert!(!Config::path_typed_keys().contains(&key), "{key}");
        }
    }
}

#[cfg(test)]
mod path_base_deprecation_tests {
    //! The §13.2 deprecation notice for §9.2.2's single path base
    //! (aiperceivable/apcore#113, Option B-prime).
    //!
    //! Only the *warning* is under test here; the tier matrix behind
    //! [`Config::project_root`] needs the process working directory and `$HOME`
    //! and therefore lives in `tests/config_discovery.rs`, which serialises
    //! those. Nothing in this file changes CWD: every case puts the config file
    //! in a temp directory, which is already not the CWD the test binary runs
    //! under.

    use super::Config;
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl tracing_subscriber::fmt::MakeWriter<'_> for CaptureWriter {
        type Writer = Self;
        fn make_writer(&self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture_logs(f: impl FnOnce()) -> String {
        let buf = CaptureWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = buf.0.lock().unwrap().clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// A §9.1-valid config whose scalar path-typed keys take the given values.
    ///
    /// `bindings.dir` is written like its three siblings rather than left out:
    /// since spec v1.36.0 it carries the canonical default `./bindings`, so an
    /// omitted key is a RELATIVE path-typed value in the merged configuration,
    /// not an absent one.
    fn write_config(
        dir: &Path,
        extensions: &str,
        schema: &str,
        acl: &str,
        bindings: &str,
    ) -> std::path::PathBuf {
        let path = dir.join("apcore.yaml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "version: '0.15.0'").unwrap();
        writeln!(f, "project:").unwrap();
        writeln!(f, "  name: demo").unwrap();
        writeln!(f, "extensions:").unwrap();
        writeln!(f, "  root: {extensions}").unwrap();
        writeln!(f, "schema:").unwrap();
        writeln!(f, "  root: {schema}").unwrap();
        writeln!(f, "acl:").unwrap();
        writeln!(f, "  root: {acl}").unwrap();
        // `acl.default_effect` deliberately NOT written. It joined §9.2.4's
        // deprecation table in spec v1.47.0 (§9.1.3's first application — the
        // `apcore.yaml` twin reaches nothing, the ACL file's own key does the
        // work), so declaring it here would make every case in this module
        // carry a §9.2.4 notice. These cases are about §9.2.2's path-base
        // notice, and the "stays silent" ones assert on the absence of a
        // DEPRECATION line: a test about one notice must not be made to pass or
        // fail by another.
        writeln!(f, "bindings:").unwrap();
        writeln!(f, "  dir: {bindings}").unwrap();
        path
    }

    #[test]
    fn warns_when_the_project_root_differs_from_cwd_and_a_relative_path_key_is_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            dir.path(),
            "./extensions",
            "./schemas",
            "./acl",
            "./bindings",
        );

        let logs = capture_logs(|| {
            let config = Config::load(&path).unwrap();
            assert_ne!(
                config.project_root(),
                std::env::current_dir().unwrap(),
                "precondition: a temp-dir config must not sit in the test CWD"
            );
        });

        assert!(
            logs.contains("DEPRECATION"),
            "a config whose relative path keys will re-root must say so: {logs}"
        );
        assert!(
            logs.contains("aiperceivable/apcore#113"),
            "the warning must name the issue that explains the change: {logs}"
        );
        for key in ["extensions.root", "schema.root", "acl.root", "bindings.dir"] {
            assert!(
                logs.contains(key),
                "the warning must name the affected key {key}: {logs}"
            );
        }
    }

    #[test]
    fn stays_silent_when_every_path_typed_value_is_absolute() {
        // The half of the condition that stops this being a restatement of
        // `project_root != cwd`. An operator who already writes absolute paths
        // is unaffected by §9.2.2 and must not be warned.
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_str().unwrap().to_string();
        let path = write_config(
            dir.path(),
            &format!("{abs}/extensions"),
            &format!("{abs}/schemas"),
            &format!("{abs}/acl"),
            &format!("{abs}/bindings"),
        );

        let logs = capture_logs(|| {
            let config = Config::load(&path).unwrap();
            assert_ne!(config.project_root(), std::env::current_dir().unwrap());
            assert!(
                config.relative_path_typed_keys().is_empty(),
                "precondition: no relative path-typed value should remain"
            );
        });

        assert!(
            !logs.contains("DEPRECATION"),
            "absolute paths pin today's behaviour, so there is nothing to warn about: {logs}"
        );
    }

    #[test]
    fn two_affected_configurations_in_one_process_both_warn() {
        // §9.2.2 requirement 2's cadence clause, which spec v1.36.0 states
        // normatively: ONCE PER CONFIGURATION LOAD, never once per process.
        // Implementations MUST NOT suppress the warning with process-global
        // state.
        //
        // The failure the clause forbids is invisible to a one-config test. A
        // once-per-process flag passes every single-load assertion in this
        // module: the first load warns, which is all any of them observe. It is
        // only the SECOND affected document that goes silent, and by then the
        // operator has two candidate configurations and one warning naming
        // neither of them in particular.
        //
        // The two documents differ, and each warning must name its OWN project
        // root, so a second warning that were somehow a replay of the first
        // fails here too.
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first = write_config(
            first_dir.path(),
            "./extensions",
            "./schemas",
            "./acl",
            "./bindings",
        );
        let second = write_config(
            second_dir.path(),
            "./ext2",
            "./schemas2",
            "./acl2",
            "./bindings2",
        );

        let first_logs = capture_logs(|| {
            Config::load(&first).unwrap();
        });
        let second_logs = capture_logs(|| {
            Config::load(&second).unwrap();
        });

        for (which, logs, dir) in [
            ("first", &first_logs, first_dir.path()),
            ("second", &second_logs, second_dir.path()),
        ] {
            assert!(
                logs.contains("DEPRECATION"),
                "the {which} affected configuration must warn — a warning \
                 consumed by an earlier load is the process-global suppression \
                 §9.2.2 forbids: {logs}"
            );
            let root = dir.canonicalize().unwrap();
            assert!(
                logs.contains(&root.display().to_string()),
                "the {which} warning must name that document's own project \
                 root {}: {logs}",
                root.display()
            );
        }
    }

    #[test]
    fn an_unaffected_configuration_between_two_affected_ones_stays_silent() {
        // The other half of "a property of the document being loaded": a load
        // that meets neither condition emits nothing, and does not disturb the
        // loads on either side of it. Together with the test above this pins
        // the cadence to the DOCUMENT rather than to the process in both
        // directions.
        let affected_dir = tempfile::tempdir().unwrap();
        let clean_dir = tempfile::tempdir().unwrap();
        let affected = write_config(
            affected_dir.path(),
            "./extensions",
            "./schemas",
            "./acl",
            "./bindings",
        );
        let abs = clean_dir.path().to_str().unwrap().to_string();
        let clean = write_config(
            clean_dir.path(),
            &format!("{abs}/extensions"),
            &format!("{abs}/schemas"),
            &format!("{abs}/acl"),
            &format!("{abs}/bindings"),
        );

        let before = capture_logs(|| {
            Config::load(&affected).unwrap();
        });
        let middle = capture_logs(|| {
            Config::load(&clean).unwrap();
        });
        let after = capture_logs(|| {
            Config::load(&affected).unwrap();
        });

        assert!(before.contains("DEPRECATION"), "{before}");
        assert!(
            !middle.contains("DEPRECATION"),
            "every path-typed value is absolute, so this document cannot \
             re-root and must be silent: {middle}"
        );
        assert!(
            after.contains("DEPRECATION"),
            "re-reading the same affected document warns again — §9.2.2: a \
             reload SHOULD re-evaluate, and an operator who has just edited the \
             file should see the notice against what they wrote: {after}"
        );
    }

    #[test]
    fn relative_path_typed_keys_reports_the_list_form() {
        // `extensions.roots` is list-valued and both element shapes carry a
        // path; an implementation modelling only the bare-string form would
        // under-report and silently skip the warning.
        let config: Config = serde_json::from_value(serde_json::json!({
            "version": "1.0.0",
            "project": { "name": "demo" },
            "extensions": { "root": "/abs/extensions", "roots": [
                { "root": "./plugins", "namespace": "plugins" }
            ]},
            "schema": { "root": "/abs/schemas" },
            "acl": { "root": "/abs/acl", "default_effect": "deny" },
            "bindings": { "dir": "/abs/bindings" }
        }))
        .unwrap();

        assert_eq!(
            config.relative_path_typed_keys(),
            vec!["extensions.roots[]"]
        );
    }

    #[test]
    fn a_config_with_no_source_path_reports_cwd_as_its_project_root() {
        // Tier "no config file found": `from_defaults` has no source path, so
        // there is no directory to prefer over CWD.
        let config = Config::from_defaults();
        assert_eq!(config.source_path(), None);
        assert_eq!(config.project_root(), std::env::current_dir().unwrap());
    }
}
