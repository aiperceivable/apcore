// APCore Protocol — Span and SpanExporter
// Spec reference: Distributed tracing spans

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::errors::ModuleError;

/// A tracing span representing a unit of work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start_time: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<f64>,
    #[serde(default)]
    pub attributes: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub(crate) events: Vec<SpanEvent>,
    pub status: SpanStatus,
}

/// An event within a span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpanEvent {
    pub name: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub attributes: HashMap<String, serde_json::Value>,
}

/// Status of a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanStatus {
    Unset,
    Ok,
    Error,
}

impl Span {
    /// Create a new span with a random hex `span_id`.
    pub fn new(name: impl Into<String>, trace_id: impl Into<String>) -> Self {
        let span_id = format!(
            "{:016x}",
            uuid::Uuid::new_v4().as_u128() & 0xFFFF_FFFF_FFFF_FFFF
        );
        #[allow(clippy::cast_precision_loss)]
        // intentional: millisecond timestamps fit in f64 for practical purposes
        let now = Utc::now().timestamp_millis() as f64 / 1000.0;
        Self {
            trace_id: trace_id.into(),
            span_id,
            parent_span_id: None,
            name: name.into(),
            start_time: now,
            end_time: None,
            attributes: HashMap::new(),
            events: vec![],
            status: SpanStatus::Unset,
        }
    }

    /// End the span, recording the end time as epoch seconds.
    pub fn end(&mut self) {
        #[allow(clippy::cast_precision_loss)]
        // intentional: millisecond timestamps fit in f64 for practical purposes
        let end = Utc::now().timestamp_millis() as f64 / 1000.0;
        self.end_time = Some(end);
    }

    /// Add an attribute to the span.
    pub fn set_attribute(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.attributes.insert(key.into(), value);
    }

    /// Add an event to the span.
    pub fn add_event(&mut self, name: impl Into<String>) {
        self.events.push(SpanEvent {
            name: name.into(),
            timestamp: Utc::now(),
            attributes: HashMap::new(),
        });
    }

    /// Add an event with attributes to the span.
    pub fn add_event_with_attributes(
        &mut self,
        name: impl Into<String>,
        attributes: HashMap<String, serde_json::Value>,
    ) {
        self.events.push(SpanEvent {
            name: name.into(),
            timestamp: Utc::now(),
            attributes,
        });
    }
}

/// Trait for exporting completed spans.
#[async_trait]
pub trait SpanExporter: Send + Sync + std::fmt::Debug {
    /// Export a single completed span.
    async fn export(&self, span: &Span) -> Result<(), ModuleError>;

    /// Shut down the exporter.
    async fn shutdown(&self) -> Result<(), ModuleError>;
}
