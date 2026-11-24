// APCore Protocol — Pluggable observability storage backends
// Spec reference: observability.md §1.1 Pluggable Observability Storage

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use super::error_history::ErrorEntry;

/// A single recorded metric observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    pub name: String,
    pub value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_id: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
    pub timestamp: DateTime<Utc>,
}

impl MetricPoint {
    #[must_use]
    pub fn new(name: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            value,
            module_id: None,
            labels: HashMap::new(),
            timestamp: Utc::now(),
        }
    }

    #[must_use]
    pub fn with_module_id(mut self, module_id: impl Into<String>) -> Self {
        self.module_id = Some(module_id.into());
        self
    }

    #[must_use]
    pub fn with_labels(mut self, labels: HashMap<String, String>) -> Self {
        self.labels = labels;
        self
    }
}

/// Pluggable observability storage backend.
///
/// Implementations persist `ErrorEntry` and `MetricPoint` records produced by
/// `ErrorHistory` and `MetricsCollector`. The default in-memory implementation
/// is `InMemoryObservabilityStore`. Production deployments may swap in
/// Redis-, SQL-, or file-backed stores.
#[async_trait]
pub trait ObservabilityStore: Send + Sync + std::fmt::Debug {
    /// Record an error entry.
    async fn record_error(&self, entry: ErrorEntry);

    /// Get error entries, optionally filtered by module and limited.
    async fn get_errors(&self, module_id: Option<&str>, limit: Option<usize>) -> Vec<ErrorEntry>;

    /// Record a metric point.
    async fn record_metric(&self, metric: MetricPoint);

    /// Get metric points, optionally filtered by module and metric name.
    async fn get_metrics(
        &self,
        module_id: Option<&str>,
        metric_name: Option<&str>,
    ) -> Vec<MetricPoint>;

    /// Flush any pending writes (no-op for in-memory).
    async fn flush(&self);

    /// Clear all stored records.
    async fn clear(&self);

    /// Return the implementation type name (used for diagnostics and conformance checks).
    fn type_name(&self) -> &'static str;
}

/// Default in-memory observability store.
#[derive(Debug, Clone, Default)]
pub struct InMemoryObservabilityStore {
    inner: Arc<Mutex<InMemoryState>>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    errors: Vec<ErrorEntry>,
    metrics: Vec<MetricPoint>,
}

impl InMemoryObservabilityStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ObservabilityStore for InMemoryObservabilityStore {
    async fn record_error(&self, entry: ErrorEntry) {
        self.inner.lock().errors.push(entry);
    }

    async fn get_errors(&self, module_id: Option<&str>, limit: Option<usize>) -> Vec<ErrorEntry> {
        let state = self.inner.lock();
        let filtered: Vec<ErrorEntry> = match module_id {
            Some(id) => state
                .errors
                .iter()
                .filter(|e| e.module_id == id)
                .cloned()
                .collect(),
            None => state.errors.clone(),
        };
        match limit {
            Some(n) => filtered.into_iter().take(n).collect(),
            None => filtered,
        }
    }

    async fn record_metric(&self, metric: MetricPoint) {
        self.inner.lock().metrics.push(metric);
    }

    async fn get_metrics(
        &self,
        module_id: Option<&str>,
        metric_name: Option<&str>,
    ) -> Vec<MetricPoint> {
        let state = self.inner.lock();
        state
            .metrics
            .iter()
            .filter(|m| match module_id {
                Some(id) => m.module_id.as_deref() == Some(id),
                None => true,
            })
            .filter(|m| match metric_name {
                Some(name) => m.name == name,
                None => true,
            })
            .cloned()
            .collect()
    }

    async fn flush(&self) {}

    async fn clear(&self) {
        let mut state = self.inner.lock();
        state.errors.clear();
        state.metrics.clear();
    }

    fn type_name(&self) -> &'static str {
        "InMemoryObservabilityStore"
    }
}
