"""Tests for the observability package __init__.py re-exports."""


class TestObservabilityPackageExports:
    """Verify all expected names are importable from apcore.observability."""

    def test_span_identity(self):
        from apcore.observability import Span
        from apcore.observability.tracing import Span as _Span

        assert Span is _Span

    def test_span_exporter_identity(self):
        from apcore.observability import SpanExporter
        from apcore.observability.tracing import SpanExporter as _SpanExporter

        assert SpanExporter is _SpanExporter

    def test_stdout_exporter_identity(self):
        from apcore.observability import StdoutExporter
        from apcore.observability.tracing import StdoutExporter as _StdoutExporter

        assert StdoutExporter is _StdoutExporter

    def test_in_memory_exporter_identity(self):
        from apcore.observability import InMemoryExporter
        from apcore.observability.tracing import InMemoryExporter as _InMemoryExporter

        assert InMemoryExporter is _InMemoryExporter

    def test_tracing_middleware_identity(self):
        from apcore.observability import TracingMiddleware
        from apcore.observability.tracing import TracingMiddleware as _TracingMiddleware

        assert TracingMiddleware is _TracingMiddleware

    def test_context_logger_identity(self):
        from apcore.observability import ContextLogger
        from apcore.observability.context_logger import ContextLogger as _ContextLogger

        assert ContextLogger is _ContextLogger

    def test_obs_logging_middleware_identity(self):
        from apcore.observability import ObsLoggingMiddleware
        from apcore.observability.context_logger import (
            ObsLoggingMiddleware as _ObsLoggingMiddleware,
        )

        assert ObsLoggingMiddleware is _ObsLoggingMiddleware

    def test_metrics_collector_identity(self):
        from apcore.observability import MetricsCollector
        from apcore.observability.metrics import MetricsCollector as _MetricsCollector

        assert MetricsCollector is _MetricsCollector

    def test_metrics_middleware_identity(self):
        from apcore.observability import MetricsMiddleware
        from apcore.observability.metrics import MetricsMiddleware as _MetricsMiddleware

        assert MetricsMiddleware is _MetricsMiddleware

    def test_otlp_exporter_identity(self):
        from apcore.observability import OTLPExporter
        from apcore.observability.tracing import OTLPExporter as _OTLPExporter

        assert OTLPExporter is _OTLPExporter

    def test_all_contains_all_public_names(self):
        import apcore.observability as obs

        # Original exports must still be present (non-breaking superset)
        original = {
            "ContextLogger",
            "ErrorEntry",
            "ErrorHistory",
            "InMemoryExporter",
            "MetricsCollector",
            "MetricsMiddleware",
            "ObsLoggingMiddleware",
            "OTLPExporter",
            "Span",
            "SpanExporter",
            "StdoutExporter",
            "TracingMiddleware",
            "UsageCollector",
            "UsageMiddleware",
            "create_span",
            "estimate_p99_latency_ms",
        }
        # v0.20 hardening additions
        hardening_additions = {
            "BatchSpanProcessor",
            "SimpleSpanProcessor",
            "SpanProcessor",
            "InMemoryObservabilityStore",
            "ObservabilityStore",
            "MetricPoint",
            "PrometheusExporter",
            "RedactionConfig",
            "normalize_message",
            "compute_fingerprint",
        }
        # Issue #43 §1: generic pluggable StorageBackend.
        storage_additions = {
            "StorageBackend",
            "InMemoryStorageBackend",
        }
        # Issue #45 §3: push-style UsageExporter interface.
        usage_exporter_additions = {
            "UsageExporter",
            "NoopUsageExporter",
            "PeriodicUsageExporter",
        }
        expected = original | hardening_additions | storage_additions | usage_exporter_additions
        assert set(obs.__all__) == expected

    def test_all_entries_are_attributes(self):
        import apcore.observability as obs

        for name in obs.__all__:
            assert hasattr(obs, name), f"{name} in __all__ but not an attribute"

    def test_otlp_exporter_importable_from_top_level(self):
        from apcore import OTLPExporter
        from apcore.observability.tracing import OTLPExporter as _OTLPExporter

        assert OTLPExporter is _OTLPExporter
