// APCore Protocol — Observability module
// Spec reference: Tracing, metrics, logging, error history, and usage

pub mod error_history;
pub mod exporters;
pub mod logging;
pub mod metrics;
pub mod processor;
pub mod prometheus_exporter;
pub mod redaction;
pub mod span;
pub mod storage;
pub mod store;
pub mod tracing_config;
pub mod tracing_middleware;
pub mod usage;
pub mod usage_exporter;

pub use error_history::{
    compute_fingerprint, normalize_message, ErrorEntry, ErrorHistory, ErrorHistoryMiddleware,
};
pub use exporters::{CompositeExporter, InMemoryExporter, OTLPExporter, StdoutExporter};
pub use logging::{ContextLogger, ObsLoggingMiddleware};
pub use metrics::{MetricsCollector, MetricsMiddleware};
pub use processor::{
    BatchSpanProcessor, BatchSpanProcessorBuilder, BatchSpanProcessorConfig, SimpleSpanProcessor,
    SpanProcessor,
};
pub use prometheus_exporter::PrometheusExporter;
pub use redaction::{RedactionConfig, RedactionConfigBuilder, RedactionConfigError};
pub use span::{Span, SpanExporter};
pub use storage::{default_storage_backend, InMemoryStorageBackend, StorageBackend, StorageError};
pub use store::{InMemoryObservabilityStore, MetricPoint, ObservabilityStore};
pub use tracing_middleware::{SamplingStrategy, TracingMiddleware};
pub use usage::{UsageCollector, UsageMiddleware, UsageStats};
pub use usage_exporter::{NoopUsageExporter, PeriodicUsageExporter, UsageExporter};
