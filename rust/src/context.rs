// APCore Protocol — Identity, Context, and ContextFactory
// Spec reference: Execution context and identity model

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use parking_lot::RwLock;
use std::sync::Arc;

use crate::cancel::CancelToken;
use crate::observability::logging::ContextLogger;
use crate::trace_context::TraceParent;

const TRACE_ID_ZEROS: &str = "00000000000000000000000000000000";
const TRACE_ID_FFFF: &str = "ffffffffffffffffffffffffffffffff";

/// Generate a fresh 32-char lowercase hex trace_id aligned with W3C Trace Context.
#[must_use]
fn generate_trace_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Accept a trace_parent's trace_id if it is well-formed (32 lowercase hex,
/// not W3C-invalid); otherwise log WARN and return a fresh trace_id.
///
/// PROTOCOL_SPEC §10.5 `external_trace_parent_handling`: no dashed-UUID
/// stripping, no case folding — such normalization is the caller's
/// responsibility, not Context::create's.
fn accept_or_regenerate_trace_id(incoming: &str) -> String {
    let is_valid_hex = incoming.len() == 32
        && incoming
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let is_w3c_valid = incoming != TRACE_ID_ZEROS && incoming != TRACE_ID_FFFF;
    if is_valid_hex && is_w3c_valid {
        incoming.to_string()
    } else {
        tracing::warn!(
            "Invalid trace_id format in trace_parent: {:?}. Restarting trace.",
            incoming
        );
        generate_trace_id()
    }
}

/// Raw intermediate struct for deserializing Identity with private fields.
#[derive(Deserialize)]
struct IdentityRaw {
    id: String,
    #[serde(rename = "type")]
    identity_type: String,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    attrs: HashMap<String, serde_json::Value>,
}

impl From<IdentityRaw> for Identity {
    fn from(raw: IdentityRaw) -> Self {
        Identity {
            id: raw.id,
            identity_type: raw.identity_type,
            roles: raw.roles,
            attrs: raw.attrs,
        }
    }
}

/// Frozen/immutable identity representing the caller.
/// Fields are private to enforce immutability after construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "IdentityRaw")]
#[allow(clippy::struct_field_names)] // `identity_type` intentionally prefixed for clarity with renamed JSON field
pub struct Identity {
    id: String,
    #[serde(rename = "type")]
    identity_type: String,
    roles: Vec<String>,
    attrs: HashMap<String, serde_json::Value>,
}

impl Identity {
    /// Create a new identity with the given fields.
    #[must_use]
    pub fn new(
        id: String,
        identity_type: String,
        roles: Vec<String>,
        attrs: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            id,
            identity_type,
            roles,
            attrs,
        }
    }

    /// The unique identifier of the caller.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The type of the identity (e.g. "user", "service", "agent").
    #[must_use]
    pub fn identity_type(&self) -> &str {
        &self.identity_type
    }

    /// The roles assigned to this identity.
    #[must_use]
    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    /// Additional attributes associated with this identity.
    #[must_use]
    pub fn attrs(&self) -> &HashMap<String, serde_json::Value> {
        &self.attrs
    }

    /// Get an attribute value by key.
    ///
    /// Aligned with apcore D-03. May trigger a fetch if the identity
    /// is lazy-loaded (future extension).
    #[must_use]
    pub fn get_attr(&self, key: &str) -> Option<&serde_json::Value> {
        self.attrs.get(key)
    }
}

// Manual Hash impl: serde_json::Value doesn't implement Hash, so we sort
// attrs by key and hash each value's canonical JSON string representation.
// This is consistent with the derived PartialEq/Eq (order-independent HashMap
// equality), because sorting by key always produces the same ordered sequence
// for any two maps that compare equal.
impl std::hash::Hash for Identity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.identity_type.hash(state);
        self.roles.hash(state);
        let mut pairs: Vec<_> = self.attrs.iter().collect();
        pairs.sort_by_key(|(k, _)| k.as_str());
        for (k, v) in pairs {
            k.hash(state);
            v.to_string().hash(state);
        }
    }
}

/// Shared mutable data map that is readable/writable along the call chain.
///
/// Parent and child contexts share the same underlying `HashMap` through
/// an `Arc<parking_lot::RwLock<...>>`. Use `data.read()` / `data.write()`
/// to access entries — the guards are infallible (not `Result`), so no
/// `.unwrap()` is required.
///
/// IMPORTANT: `parking_lot::RwLock` guards are synchronous. Never hold a
/// guard across an `.await` point — copy or clone the data you need and
/// drop the guard first.
///
/// Note: deserialization creates a new `Arc` (not shared with the original).
pub type SharedData = Arc<RwLock<HashMap<String, serde_json::Value>>>;

/// Generic execution context carrying identity, services, and data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context<T> {
    pub trace_id: String,
    /// The caller identity. `None` represents an anonymous/unauthenticated caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    pub services: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_id: Option<String>,
    /// Shared data map — parent and child contexts share the same instance.
    /// Serializes as a plain `HashMap`; deserializes into a new `Arc<RwLock<...>>`.
    #[serde(
        serialize_with = "serialize_shared_data",
        deserialize_with = "deserialize_shared_data",
        default = "default_shared_data"
    )]
    pub data: SharedData,
    #[serde(default)]
    pub call_chain: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_inputs: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_output: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip)]
    pub cancel_token: Option<CancelToken>,
    /// Global deadline for dual-timeout enforcement (not serialized).
    /// Stored as absolute epoch seconds (f64) for cross-language alignment.
    #[serde(skip)]
    pub global_deadline: Option<f64>,
    /// Runtime reference to the executor for nested calls (not serialized).
    /// Opaque; use [`Context::executor`] to call through it.
    #[serde(skip)]
    pub executor: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

fn default_shared_data() -> SharedData {
    Arc::new(RwLock::new(HashMap::new()))
}

fn serialize_shared_data<S>(data: &SharedData, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let map = data.read();
    map.serialize(serializer)
}

fn deserialize_shared_data<'de, D>(deserializer: D) -> Result<SharedData, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let map = HashMap::<String, serde_json::Value>::deserialize(deserializer)?;
    Ok(Arc::new(RwLock::new(map)))
}

impl<T: Default> Context<T> {
    #[must_use]
    pub fn new(identity: Identity) -> Self {
        Self {
            trace_id: generate_trace_id(),
            identity: Some(identity),
            services: T::default(),
            caller_id: None,
            data: default_shared_data(),
            call_chain: vec![],
            redacted_inputs: None,
            redacted_output: None,
            cancel_token: None,
            global_deadline: None,
            executor: None,
        }
    }

    /// Create a context with no identity (anonymous/unauthenticated).
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            trace_id: generate_trace_id(),
            identity: None,
            services: T::default(),
            caller_id: None,
            data: default_shared_data(),
            call_chain: vec![],
            redacted_inputs: None,
            redacted_output: None,
            cancel_token: None,
            global_deadline: None,
            executor: None,
        }
    }

    /// Create a child context for nested calls.
    ///
    /// The child shares the same `data` map (via `Arc`) so writes in either
    /// parent or child are visible to both.
    #[must_use]
    pub fn child(&self, target_module_id: &str) -> Context<T>
    where
        T: Clone,
    {
        let caller_id = self.call_chain.last().cloned();
        let mut call_chain = self.call_chain.clone();
        call_chain.push(target_module_id.to_string());

        Context {
            trace_id: self.trace_id.clone(),
            identity: self.identity.clone(),
            services: self.services.clone(),
            caller_id,
            // Share the same underlying data map (Arc clone, not HashMap clone).
            data: Arc::clone(&self.data),
            call_chain,
            redacted_inputs: None,
            redacted_output: None,
            cancel_token: self.cancel_token.clone(),
            global_deadline: self.global_deadline,
            executor: self.executor.clone(),
        }
    }

    /// Serialize context to the canonical cross-language JSON wire shape.
    ///
    /// A-D-005 / A-D-020: this delegates to [`serialize`](Self::serialize) so the
    /// public `to_json` path produces the SAME wire shape as `serialize()` —
    /// `_context_version: 1` present, `services` excluded, `caller_id`/`identity`
    /// always emitted (null when absent), and `_`-prefixed data keys filtered.
    /// The previous implementation went through the derived `Serialize`, which
    /// emitted `services`, omitted `_context_version`, and dropped `caller_id`
    /// when `None` — a shape that could not round-trip through Python/TS.
    pub fn to_json(&self) -> serde_json::Value {
        self.serialize()
    }

    /// Serialize context to the canonical JSON wire shape, fallibly.
    ///
    /// A-D-005 / A-D-020: like [`to_json`](Self::to_json), delegates to
    /// [`serialize`](Self::serialize) to guarantee the canonical wire shape.
    /// `serialize()` is infallible, so this never returns `Err`; the `Result`
    /// signature is retained for API compatibility.
    pub fn to_json_checked(&self) -> Result<serde_json::Value, crate::errors::ModuleError> {
        Ok(self.serialize())
    }

    /// Deserialize context from JSON.
    pub fn from_json(
        data: serde_json::Value,
    ) -> Result<Context<serde_json::Value>, crate::errors::ModuleError> {
        let ctx: Context<serde_json::Value> = serde_json::from_value(data)?;
        Ok(ctx)
    }

    /// Serialize context to a cross-language JSON representation.
    ///
    /// Includes `_context_version: 1` at top level.
    /// Excludes: executor, services, `cancel_token`, `global_deadline`.
    /// Filters `_`-prefixed keys from data.
    pub fn serialize(&self) -> serde_json::Value {
        let mut result = serde_json::json!({
            "_context_version": 1,
            "trace_id": self.trace_id,
            "caller_id": self.caller_id,
            "call_chain": self.call_chain,
        });

        if let Some(ref identity) = self.identity {
            result["identity"] = serde_json::json!({
                "id": identity.id(),
                "type": identity.identity_type(),
                "roles": identity.roles(),
                "attrs": identity.attrs(),
            });
        } else {
            result["identity"] = serde_json::Value::Null;
        }

        if let Some(ref redacted) = self.redacted_inputs {
            result["redacted_inputs"] = serde_json::to_value(redacted).unwrap_or_else(|e| {
                tracing::error!(
                    trace_id = %self.trace_id,
                    error = %e,
                    "Context::serialize failed to serialize redacted_inputs"
                );
                serde_json::Value::Null
            });
        }
        if let Some(ref redacted) = self.redacted_output {
            result["redacted_output"] = serde_json::to_value(redacted).unwrap_or_else(|e| {
                tracing::error!(
                    trace_id = %self.trace_id,
                    error = %e,
                    "Context::serialize failed to serialize redacted_output"
                );
                serde_json::Value::Null
            });
        }

        // Filter _-prefixed keys from data
        let filtered: HashMap<String, serde_json::Value> = self
            .data
            .read()
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        result["data"] = serde_json::to_value(filtered).unwrap_or_else(|e| {
            tracing::error!(
                trace_id = %self.trace_id,
                error = %e,
                "Context::serialize failed to serialize data; returning empty object"
            );
            serde_json::json!({})
        });
        result
    }

    /// Deserialize a cross-language JSON representation into a Context.
    ///
    /// Non-serializable fields (executor, services, `cancel_token`,
    /// `global_deadline`) are set to `None`/default after deserialization.
    /// If `_context_version` is greater than 1, a warning is logged
    /// but deserialization proceeds (forward compatibility).
    #[allow(clippy::needless_pass_by_value)] // public API: callers pass owned Value; changing to &Value would be breaking
    pub fn deserialize(value: serde_json::Value) -> Result<Self, serde_json::Error>
    where
        T: Default,
    {
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("expected JSON object"))?;

        let version = obj
            .get("_context_version")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(1);

        if version > 1 {
            tracing::warn!(
                version = version,
                "Unknown _context_version (expected 1). \
                 Proceeding with best-effort deserialization."
            );
        }

        let identity: Option<Identity> = obj.get("identity").and_then(|v| {
            if v.is_null() {
                None
            } else {
                serde_json::from_value(v.clone()).ok()
            }
        });

        let data_map: HashMap<String, serde_json::Value> = obj
            .get("data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let call_chain: Vec<String> = obj
            .get("call_chain")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let redacted_inputs: Option<HashMap<String, serde_json::Value>> = obj
            .get("redacted_inputs")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let redacted_output: Option<HashMap<String, serde_json::Value>> = obj
            .get("redacted_output")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        Ok(Context {
            trace_id: obj
                .get("trace_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            caller_id: obj
                .get("caller_id")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            call_chain,
            identity,
            redacted_inputs,
            redacted_output,
            data: Arc::new(RwLock::new(data_map)),
            services: T::default(),
            cancel_token: None,
            global_deadline: None,
            executor: None,
        })
    }

    /// Get a context-scoped logger.
    ///
    /// # Deprecated
    ///
    /// Use the host application's logger (apcore#121). Removed at v2.0.
    ///
    /// Application and module code SHOULD log through the logger the host
    /// application has already configured — `tracing`, `log`, or whatever the
    /// binary installed. To emit automatic apcore execution events instead,
    /// install `ObsLoggingMiddleware` explicitly: that is a different facility,
    /// not a drop-in replacement for ad-hoc logging.
    ///
    /// This method has no configuration door and cannot acquire one without
    /// paying for it somewhere the framework should not. Its logger is built
    /// per call from a [`Context`], which carries no `Config` in any SDK, and
    /// the only route that avoids threading one through `Context`'s pinned
    /// six-parameter contract is a process-global logger — which would end the
    /// multi-instance isolation apcore currently gets for free. So the output
    /// is fixed at stderr / `info` / JSON, bypassing whatever the host has
    /// configured, and notably NOT going through `tracing`. Per PROTOCOL_SPEC
    /// §9.2.4's D-67 boundary, apcore does not own the host's logging policy.
    #[deprecated(
        since = "0.31.0",
        note = "use the host application's logger; its output is under the host's control and \
                this one's is not (always stderr, info, JSON, bypassing `tracing`). To emit \
                automatic apcore execution events, install ObsLoggingMiddleware explicitly. \
                See apcore#121. Removed at v2.0."
    )]
    pub fn logger(&self) -> ContextLogger {
        let module_id = self.call_chain.last().cloned();
        let caller_id = self.caller_id.clone();
        let mut logger = ContextLogger::new("apcore");
        logger.trace_id = Some(self.trace_id.clone());
        logger.module_id = module_id;
        logger.caller_id = caller_id;
        logger
    }

    /// Create a context from the canonical caller-supplied inputs.
    ///
    /// Unified factory signature per apcore Issue #66 / `core-executor.md`
    /// §"Contract: Context.create". The six inputs are positional and ordered
    /// to match the cross-language contract:
    ///
    /// 1. `identity` — optional caller identity. `None` represents an
    ///    anonymous/unauthenticated caller. Downstream consumers
    ///    (`ACL::check`, `sys_modules`) map `None` to the `@external`
    ///    sentinel at evaluation time.
    /// 2. `trace_parent` — optional W3C Trace Context entry. The trace_id is
    ///    inherited when well-formed (`^[0-9a-f]{32}$`, not all-zero,
    ///    not all-f); invalid values log WARN and a fresh trace_id is
    ///    generated. Per the v0.22.0 contract, `tracestate` is a field of
    ///    `TraceParent` rather than a separate parameter — see
    ///    [`crate::trace_context::TraceParent::tracestate`].
    /// 3. `cancel_token` — optional external cooperative-cancellation source.
    ///    Adopting this as a first-class parameter eliminates the post-hoc
    ///    `ctx.cancel_token = Some(token)` anti-pattern.
    /// 4. `data` — optional initial entries for the shared data map. The
    ///    inbound `trace_parent` (when present) seeds the canonical
    ///    `_apcore.trace.flags` / `_apcore.trace.state` keys so
    ///    `TraceContext::inject` propagates the inbound sampling decision and
    ///    vendor state.
    /// 5. `services` — caller-supplied DI container (positional `T`, not
    ///    `Option<T>`). The `cancel_token` parameter above is now first-class
    ///    and MUST NOT be smuggled inside `services`.
    /// 6. `global_deadline` — absolute epoch-seconds timestamp bounding the
    ///    total execution time of the call tree. Local-only.
    ///
    /// The following Context fields are NOT caller inputs and are intentionally
    /// excluded from the signature:
    ///
    /// - `trace_id` — derived from `trace_parent` or freshly generated.
    /// - `caller_id` — top-level Contexts always have `caller_id = None`;
    ///   managed exclusively by [`Context::child`].
    /// - `call_chain` — empty `[]` at top-level; managed by the Executor.
    /// - `executor` — bound by the Executor at pipeline entry; see
    ///   [`Context::bind_executor`].
    pub fn create(
        identity: Option<Identity>,
        trace_parent: Option<TraceParent>,
        cancel_token: Option<CancelToken>,
        data: Option<HashMap<String, serde_json::Value>>,
        services: T,
        global_deadline: Option<f64>,
    ) -> Self {
        let mut initial_data = data.unwrap_or_default();
        let trace_id = if let Some(tp) = trace_parent {
            let tid = accept_or_regenerate_trace_id(&tp.trace_id);
            // Stash inbound trace-flags as a 2-char hex string for symmetry
            // with the W3C wire format; readers parse via `u8::from_str_radix`.
            initial_data
                .entry(crate::trace_context::TRACE_FLAGS_KEY.to_string())
                .or_insert_with(|| serde_json::Value::String(format!("{:02x}", tp.trace_flags)));
            // Stash inbound tracestate as a JSON array of 2-element arrays so
            // `TraceContext::inject(&context)` can read it back without
            // requiring the caller to pass it as an explicit argument.
            // Mirrors apcore-python `_apcore.trace.state` (context.py).
            if !tp.tracestate.is_empty() {
                initial_data
                    .entry(crate::trace_context::TRACE_STATE_KEY.to_string())
                    .or_insert_with(|| {
                        serde_json::Value::Array(
                            tp.tracestate
                                .into_iter()
                                .map(|(k, v)| {
                                    serde_json::Value::Array(vec![
                                        serde_json::Value::String(k),
                                        serde_json::Value::String(v),
                                    ])
                                })
                                .collect(),
                        )
                    });
            }
            tid
        } else {
            generate_trace_id()
        };
        Self {
            trace_id,
            identity,
            services,
            caller_id: None,
            data: Arc::new(RwLock::new(initial_data)),
            call_chain: vec![],
            redacted_inputs: None,
            redacted_output: None,
            cancel_token,
            global_deadline,
            executor: None,
        }
    }

    /// Start building a context with optional W3C trace_parent inheritance.
    ///
    /// Use this when integrating with web frameworks that parse incoming
    /// `traceparent` headers — the builder accepts an `Option<TraceParent>`
    /// and inherits its trace_id when well-formed, or regenerates on invalid
    /// input. See PROTOCOL_SPEC §10.5 `external_trace_parent_handling`.
    #[must_use]
    pub fn builder() -> ContextBuilder<T> {
        ContextBuilder::new()
    }
}

/// Builder for [`Context`] that supports W3C Trace Context propagation.
///
/// Created via [`Context::builder`]. Accepts an optional `TraceParent` whose
/// trace_id is inherited when it matches `^[0-9a-f]{32}$` and is not the
/// W3C-reserved all-zero or all-f value. Otherwise the builder generates a
/// fresh trace_id and emits a `tracing::warn!` log.
pub struct ContextBuilder<T> {
    trace_parent: Option<TraceParent>,
    identity: Option<Identity>,
    services: Option<T>,
    caller_id: Option<String>,
    data: Option<HashMap<String, serde_json::Value>>,
    global_deadline: Option<f64>,
    cancel_token: Option<CancelToken>,
}

impl<T> Default for ContextBuilder<T> {
    fn default() -> Self {
        Self {
            trace_parent: None,
            identity: None,
            services: None,
            caller_id: None,
            data: None,
            global_deadline: None,
            cancel_token: None,
        }
    }
}

impl<T> ContextBuilder<T> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the W3C trace_parent to inherit from (when well-formed).
    #[must_use]
    pub fn trace_parent(mut self, trace_parent: Option<TraceParent>) -> Self {
        self.trace_parent = trace_parent;
        self
    }

    /// Set the caller identity. `None` is the anonymous/unauthenticated case.
    #[must_use]
    pub fn identity(mut self, identity: Option<Identity>) -> Self {
        self.identity = identity;
        self
    }

    /// Set the services container for dependency injection.
    #[must_use]
    pub fn services(mut self, services: T) -> Self {
        self.services = Some(services);
        self
    }

    /// Set the caller_id. Top-level calls leave this as `None`.
    #[must_use]
    pub fn caller_id(mut self, caller_id: Option<String>) -> Self {
        self.caller_id = caller_id;
        self
    }

    /// Seed the shared data map with initial entries.
    #[must_use]
    pub fn data(mut self, data: HashMap<String, serde_json::Value>) -> Self {
        self.data = Some(data);
        self
    }

    /// Set an absolute deadline (epoch seconds, f64) that bounds the total
    /// execution time for the call tree rooted at this context.
    #[must_use]
    pub fn global_deadline(mut self, deadline: Option<f64>) -> Self {
        self.global_deadline = deadline;
        self
    }

    /// Set the external cooperative-cancellation token for this Context.
    ///
    /// Mirrors the first-class `cancel_token` parameter of [`Context::create`]
    /// per the unified factory contract (apcore Issue #66 /
    /// `core-executor.md` §"Contract: Context.create"). Eliminates the
    /// post-hoc `ctx.cancel_token = Some(token)` anti-pattern.
    #[must_use]
    pub fn cancel_token(mut self, token: Option<CancelToken>) -> Self {
        self.cancel_token = token;
        self
    }
}

impl<T: Default> ContextBuilder<T> {
    /// Finalize the builder into a [`Context`].
    ///
    /// If a `trace_parent` was set, its trace_id is validated per
    /// PROTOCOL_SPEC §10.5 and either accepted verbatim or replaced with a
    /// freshly generated 32-char hex trace_id (with a WARN log on regen).
    /// When the inbound `trace_parent` carries a `trace_flags` byte, it is
    /// seeded into `data` under [`crate::trace_context::TRACE_FLAGS_KEY`] so
    /// that `TraceContext::inject` propagates the inbound sampling decision
    /// to outbound calls (cross-language parity with `apcore-python`).
    #[must_use]
    pub fn build(self) -> Context<T> {
        let trace_id = match self.trace_parent.as_ref() {
            Some(tp) => accept_or_regenerate_trace_id(&tp.trace_id),
            None => generate_trace_id(),
        };
        let mut initial_data = self.data.unwrap_or_default();
        if let Some(tp) = self.trace_parent.as_ref() {
            // Stash inbound trace-flags as a 2-char hex string for symmetry
            // with the W3C wire format; readers parse via `u8::from_str_radix`.
            initial_data
                .entry(crate::trace_context::TRACE_FLAGS_KEY.to_string())
                .or_insert_with(|| serde_json::Value::String(format!("{:02x}", tp.trace_flags)));
        }
        // D11-002b: Stash inbound tracestate as a JSON array of 2-element
        // arrays so `TraceContext::inject(&context)` can read it back without
        // requiring the caller to pass it as an explicit argument. Per the
        // v0.22.0 contract the inbound tracestate now travels on the
        // `TraceParent` itself (Issue #66) rather than via a separate builder
        // setter.
        if let Some(tp) = self.trace_parent.as_ref() {
            if !tp.tracestate.is_empty() {
                initial_data
                    .entry(crate::trace_context::TRACE_STATE_KEY.to_string())
                    .or_insert_with(|| {
                        serde_json::Value::Array(
                            tp.tracestate
                                .iter()
                                .map(|(k, v)| {
                                    serde_json::Value::Array(vec![
                                        serde_json::Value::String(k.clone()),
                                        serde_json::Value::String(v.clone()),
                                    ])
                                })
                                .collect(),
                        )
                    });
            }
        }
        Context {
            trace_id,
            identity: self.identity,
            services: self.services.unwrap_or_default(),
            caller_id: self.caller_id,
            data: Arc::new(RwLock::new(initial_data)),
            call_chain: vec![],
            redacted_inputs: None,
            redacted_output: None,
            cancel_token: self.cancel_token,
            global_deadline: self.global_deadline,
            executor: None,
        }
    }
}

impl<T> Context<T> {
    /// The Executor this context is bound to, for a nested call.
    ///
    /// A module calls another module with the Executor that is running it and
    /// the context it was handed (PROTOCOL_SPEC §5.7):
    ///
    /// ```ignore
    /// let executor = ctx.executor().expect("bound to a shared executor");
    /// let out = executor.call("common.util.parse", inputs, Some(ctx), None).await?;
    /// ```
    ///
    /// `None` when the context is not bound yet, or its Executor is not shared
    /// (see [`Executor::into_shared`](crate::executor::Executor::into_shared);
    /// an [`APCore`](crate::APCore) client's Executor always is).
    #[must_use]
    pub fn executor(&self) -> Option<Arc<crate::executor::Executor>> {
        self.executor
            .as_deref()?
            .downcast_ref::<crate::executor::ExecutorHandle>()?
            .executor()
    }

    /// SDK-internal: bind the receiving Executor to this Context per apcore
    /// Issue #66 / `core-executor.md` §"Contract: Executor binding to Context".
    ///
    /// Rules (identity comparison uses `Arc::ptr_eq`):
    ///
    /// 1. If `context.executor` is `None`, the Executor handle is stored and
    ///    `Ok(())` is returned.
    /// 2. If `context.executor` is already bound to **the same** Executor
    ///    instance (`Arc::ptr_eq`), the rebind is a noop and `Ok(())` is
    ///    returned. This covers the common pattern of reusing one Context
    ///    across multiple top-level calls on the same Executor.
    /// 3. If `context.executor` is already bound to a **different** Executor
    ///    instance, this method returns
    ///    [`crate::errors::ErrorCode::ContextBindingError`].
    ///
    /// Hidden from rustdoc because the binding is an Executor-pipeline
    /// invariant, not a caller-facing API. Application code should never call
    /// it directly.
    #[doc(hidden)]
    pub fn bind_executor(
        &mut self,
        executor: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<(), crate::errors::ModuleError> {
        match &self.executor {
            None => {
                self.executor = Some(executor);
                Ok(())
            }
            Some(existing) if Arc::ptr_eq(existing, &executor) => Ok(()),
            Some(_) => Err(crate::errors::ModuleError::new(
                crate::errors::ErrorCode::ContextBindingError,
                "Context already bound to a different Executor instance",
            )),
        }
    }
}

/// Factory trait for creating execution contexts from runtime requests.
///
/// # Why the request is an associated type (spec v1.49.0, D-76)
///
/// `Contract: ContextFactory.create_context` declares a single opaque
/// `request` input: the interface exists so a web-framework integration can
/// turn a *runtime-specific* request into a [`Context`], extracting the caller
/// identity along the way. A signature that already receives an [`Identity`]
/// has had that work done for it and has nothing left to do.
///
/// Rust needs a concrete type where Python and TypeScript pass `Any`. The
/// choices are a trait type parameter (`ContextFactory<R>`) or an associated
/// type; this trait uses an **associated type**, because a factory maps
/// exactly one request type to a `Context` — the same reason `Iterator` has
/// `Item` rather than a parameter. It keeps call sites inference-free
/// (`factory.create_context(req)` needs no turbofish) and it still names an
/// object-safe `dyn` form, `&dyn ContextFactory<Request = MyRequest>`.
///
/// ```no_run
/// use apcore::context::{Context, ContextFactory, Identity};
/// use apcore::errors::ModuleError;
/// use async_trait::async_trait;
/// use std::collections::HashMap;
///
/// struct AxumRequest {
///     user_id: Option<String>,
/// }
///
/// struct AxumContextFactory;
///
/// #[async_trait]
/// impl ContextFactory for AxumContextFactory {
///     type Request = AxumRequest;
///
///     async fn create_context(
///         &self,
///         request: AxumRequest,
///     ) -> Result<Context<serde_json::Value>, ModuleError> {
///         // An unauthenticated request still yields a usable, unprivileged
///         // Context; the ACL's default-deny governs what it may reach.
///         let identity = request.user_id.map(|id| {
///             Identity::new(id, "user".to_string(), vec![], HashMap::new())
///         });
///         Ok(Context::create(identity, None, None, None, serde_json::Value::Null, None))
///     }
/// }
/// ```
#[async_trait]
pub trait ContextFactory: Send + Sync {
    /// The host's runtime request type — an Axum extractor, an
    /// `http::Request<B>`, a framework-specific wrapper. Opaque to apcore:
    /// the implementation, not the protocol, knows how to read it.
    type Request: Send + 'static;

    /// Create a context from a runtime request.
    ///
    /// The canonical spec surface. Extraction failures that mean "not
    /// authenticated" MUST NOT use the error channel — sanitize to the
    /// anonymous `@external` identity (`identity: None`) instead and let the
    /// ACL decide. The error channel is for extraction that genuinely fails
    /// (I/O, for example).
    async fn create_context(
        &self,
        request: Self::Request,
    ) -> Result<Context<serde_json::Value>, crate::errors::ModuleError>;

    /// Create a new context for the given identity and services.
    /// Pass `None` for anonymous/unauthenticated contexts.
    ///
    /// Rust-only additive member: not part of
    /// `Contract: ContextFactory.create_context`, and unconstrained by it.
    async fn create(
        &self,
        identity: Option<Identity>,
        services: serde_json::Value,
    ) -> Result<Context<serde_json::Value>, crate::errors::ModuleError> {
        Ok(Context::create(identity, None, None, None, services, None))
    }

    /// Create a child context from an existing parent context.
    ///
    /// Rust-only additive member, as [`ContextFactory::create`].
    async fn create_child(
        &self,
        parent: &Context<serde_json::Value>,
        module_name: &str,
    ) -> Result<Context<serde_json::Value>, crate::errors::ModuleError> {
        Ok(parent.child(module_name))
    }
}
