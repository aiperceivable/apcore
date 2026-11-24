"""Tests for built-in context key constants."""

from apcore.context_key import ContextKey
from apcore.context_keys import (
    LOGGING_START,
    METRICS_STARTS,
    REDACTED_OUTPUT,
    RETRY_COUNT_BASE,
    TRACING_SAMPLED,
    TRACING_SPANS,
)


class TestBuiltinKeys:
    def test_tracing_spans_name(self) -> None:
        assert TRACING_SPANS.name == "_apcore.mw.tracing.spans"

    def test_tracing_sampled_name(self) -> None:
        assert TRACING_SAMPLED.name == "_apcore.mw.tracing.sampled"

    def test_metrics_starts_name(self) -> None:
        assert METRICS_STARTS.name == "_apcore.mw.metrics.starts"

    def test_logging_start_name(self) -> None:
        assert LOGGING_START.name == "_apcore.mw.logging.start_time"

    def test_redacted_output_name(self) -> None:
        assert REDACTED_OUTPUT.name == "_apcore.executor.redacted_output"

    def test_retry_count_base_name(self) -> None:
        assert RETRY_COUNT_BASE.name == "_apcore.mw.retry.count"

    def test_retry_count_base_scoped(self) -> None:
        scoped = RETRY_COUNT_BASE.scoped("my_module")
        assert scoped.name == "_apcore.mw.retry.count.my_module"

    def test_all_keys_are_context_key_instances(self) -> None:
        for key in [
            TRACING_SPANS,
            TRACING_SAMPLED,
            METRICS_STARTS,
            LOGGING_START,
            REDACTED_OUTPUT,
            RETRY_COUNT_BASE,
        ]:
            assert isinstance(key, ContextKey)
