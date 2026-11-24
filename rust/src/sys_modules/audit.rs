// APCore Protocol — Audit trail for system control modules
// Spec reference: system-modules.md §1.2 Contextual Audit Trail (Issue #45)

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::context::Context;
use crate::errors::ModuleError;

/// Action recorded by an audit entry.
///
/// Cross-language parity with `apcore-python.AuditEntry.action` and the
/// TypeScript SDK enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    UpdateConfig,
    ReloadModule,
    ToggleFeature,
}

impl AuditAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UpdateConfig => "update_config",
            Self::ReloadModule => "reload_module",
            Self::ToggleFeature => "toggle_feature",
        }
    }
}

/// Structured before/after change record. `before` and `after` are arbitrary
/// JSON values to fit any of: config values, version strings, or booleans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditChange {
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}

/// A single audit record for a system control operation.
///
/// Spec: system-modules.md §1.2 normative rules. Each entry carries the
/// caller identity (extracted from `context.identity`), the target module,
/// the trace_id, and the change applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub action: AuditAction,
    pub target_module_id: String,
    pub actor_id: String,
    pub actor_type: String,
    pub trace_id: String,
    pub change: AuditChange,
    /// Groups the entries a single multi-module operation produced (D-111).
    ///
    /// A bulk reload writes one entry PER MODULE, because
    /// `AuditStore::query(module_id)` filters on a concrete id and cannot find
    /// an entry keyed on the glob. Per-module entries alone lose the fact that
    /// they were one deploy, so every entry from one bulk reload carries the
    /// same correlation id and "what did this deploy touch" stays a single
    /// query.
    ///
    /// Empty for single-target operations, which need no grouping. `#[serde(default)]`
    /// so a stored entry written before this field existed still deserializes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub correlation_id: String,
}

/// Storage backend for audit entries.
///
/// Implementations may persist entries to a database, a file, or hold them
/// in memory. `append` is fallible to allow remote backends to surface
/// transport errors.
#[async_trait]
pub trait AuditStore: Send + Sync {
    async fn append(&self, entry: AuditEntry) -> Result<(), ModuleError>;

    /// Optional query method. Default returns `Ok(vec![])` so simple
    /// write-only backends do not need to implement it.
    async fn query(
        &self,
        _module_id: Option<&str>,
        _actor_id: Option<&str>,
        _since: Option<DateTime<Utc>>,
    ) -> Result<Vec<AuditEntry>, ModuleError> {
        Ok(Vec::new())
    }
}

/// Thread-safe in-memory implementation of `AuditStore`. Useful for tests
/// and small single-process deployments.
#[derive(Debug, Clone, Default)]
pub struct InMemoryAuditStore {
    entries: Arc<Mutex<Vec<AuditEntry>>>,
}

impl InMemoryAuditStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Synchronous read of all stored entries (clones the inner vec).
    #[must_use]
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().clone()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }
}

#[async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(&self, entry: AuditEntry) -> Result<(), ModuleError> {
        self.entries.lock().push(entry);
        Ok(())
    }

    async fn query(
        &self,
        module_id: Option<&str>,
        actor_id: Option<&str>,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<AuditEntry>, ModuleError> {
        let entries = self.entries.lock().clone();
        Ok(entries
            .into_iter()
            .filter(|e| module_id.is_none_or(|m| e.target_module_id == m))
            .filter(|e| actor_id.is_none_or(|a| e.actor_id == a))
            .filter(|e| since.is_none_or(|t| e.timestamp >= t))
            .collect())
    }
}

/// Build an `AuditEntry` from `Context.identity` and the call's trace_id.
///
/// Falls back to `"unknown"` when the context has no identity attached, so
/// audit records are always emitted even for anonymous callers.
pub(crate) fn build_audit_entry(
    action: AuditAction,
    target_module_id: &str,
    ctx: &Context<serde_json::Value>,
    change: AuditChange,
) -> AuditEntry {
    build_correlated_audit_entry(action, target_module_id, ctx, change, String::new())
}

/// [`build_audit_entry`] with an explicit correlation id (D-111).
pub(crate) fn build_correlated_audit_entry(
    action: AuditAction,
    target_module_id: &str,
    ctx: &Context<serde_json::Value>,
    change: AuditChange,
    correlation_id: String,
) -> AuditEntry {
    let (actor_id, actor_type) = ctx.identity.as_ref().map_or_else(
        || ("unknown".to_string(), "unknown".to_string()),
        |id| (id.id().to_string(), id.identity_type().to_string()),
    );
    AuditEntry {
        timestamp: Utc::now(),
        action,
        target_module_id: target_module_id.to_string(),
        actor_id,
        actor_type,
        trace_id: ctx.trace_id.clone(),
        change,
        correlation_id,
    }
}

/// Append an `AuditEntry` to the store if configured; otherwise log at
/// INFO level and discard. The `Arc<dyn AuditStore>` is cheap to clone, so
/// callers can hold a single instance and pass references freely.
pub(crate) async fn record_audit(store: Option<&Arc<dyn AuditStore>>, entry: AuditEntry) {
    if let Some(store) = store {
        if let Err(e) = store.append(entry.clone()).await {
            tracing::warn!(error = %e, target_module = %entry.target_module_id, "AuditStore append failed");
        }
    } else {
        tracing::info!(
            action = %entry.action.as_str(),
            target_module_id = %entry.target_module_id,
            actor_id = %entry.actor_id,
            actor_type = %entry.actor_type,
            trace_id = %entry.trace_id,
            "audit (no store configured)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Identity;
    use std::collections::HashMap;

    fn ctx_with_identity(id: &str, type_: &str) -> Context<serde_json::Value> {
        let identity = Identity::new(id.to_string(), type_.to_string(), vec![], HashMap::new());
        Context {
            trace_id: "trace-1".to_string(),
            identity: Some(identity),
            services: serde_json::Value::Null,
            caller_id: None,
            data: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            call_chain: vec![],
            redacted_inputs: None,
            redacted_output: None,
            cancel_token: None,
            global_deadline: None,
            executor: None,
        }
    }

    #[test]
    fn build_entry_extracts_identity() {
        let ctx = ctx_with_identity("user-abc", "user");
        let entry = build_audit_entry(
            AuditAction::UpdateConfig,
            "system.control.update_config",
            &ctx,
            AuditChange {
                before: serde_json::json!(1),
                after: serde_json::json!(2),
            },
        );
        assert_eq!(entry.actor_id, "user-abc");
        assert_eq!(entry.actor_type, "user");
        assert_eq!(entry.trace_id, "trace-1");
        assert_eq!(entry.action, AuditAction::UpdateConfig);
    }

    #[tokio::test]
    async fn in_memory_store_round_trip() {
        let store = InMemoryAuditStore::new();
        let ctx = ctx_with_identity("svc-1", "service");
        let entry = build_audit_entry(
            AuditAction::ToggleFeature,
            "risky.module",
            &ctx,
            AuditChange {
                before: serde_json::json!(true),
                after: serde_json::json!(false),
            },
        );
        store.append(entry).await.unwrap();
        assert_eq!(store.len(), 1);

        let by_module = store.query(Some("risky.module"), None, None).await.unwrap();
        assert_eq!(by_module.len(), 1);
        assert_eq!(by_module[0].actor_id, "svc-1");

        let by_actor = store.query(None, Some("nope"), None).await.unwrap();
        assert!(by_actor.is_empty());
    }
}
