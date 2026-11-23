"""Regression test for OBS-004: Prometheus label value escaping.

Prometheus exposition format requires label values to escape backslash
(``\\``), double-quote (``"``), and line-feed (``\\n``).  Without escaping,
a label value containing any of these characters produces malformed output
that breaks scrapers.  This test pins the escape behaviour to match the
TypeScript and Rust SDKs.
"""

from __future__ import annotations

from apcore.observability.metrics import MetricsCollector


class TestPrometheusLabelEscaping:
    """Label values containing special characters must be escaped."""

    def test_double_quote_escaped(self) -> None:
        collector = MetricsCollector()
        collector.increment(
            "apcore_module_calls_total",
            {"module_id": 'mod"x', "status": "success"},
        )
        output = collector.export_prometheus()
        # Must contain escaped backslash-quote in the value (mod\"x)
        assert 'module_id="mod\\"x"' in output
        # Must NOT contain the raw unescaped form that would break a parser
        assert 'module_id="mod"x"' not in output

    def test_backslash_escaped(self) -> None:
        collector = MetricsCollector()
        collector.increment(
            "apcore_module_calls_total",
            {"module_id": "mod\\path", "status": "ok"},
        )
        output = collector.export_prometheus()
        # Single backslash in source must be doubled in output
        assert 'module_id="mod\\\\path"' in output

    def test_newline_escaped(self) -> None:
        collector = MetricsCollector()
        collector.increment(
            "apcore_module_calls_total",
            {"module_id": "line1\nline2", "status": "ok"},
        )
        output = collector.export_prometheus()
        # Newline must be replaced with literal \n (2 chars)
        assert 'module_id="line1\\nline2"' in output
        # Sanity: no metric line contains a raw newline inside its label block
        for line in output.splitlines():
            if line.startswith("apcore_") and "{" in line:
                inside = line[line.index("{") + 1 : line.rindex("}")]
                assert "\n" not in inside

    def test_combined_special_chars(self) -> None:
        collector = MetricsCollector()
        collector.increment(
            "apcore_module_calls_total",
            {"module_id": 'a"b\\c\nd', "status": "ok"},
        )
        output = collector.export_prometheus()
        # Order of escapes: \ -> \\, then " -> \", then \n -> \n
        # So a"b\c\nd becomes a\"b\\c\nd in the output.
        assert 'module_id="a\\"b\\\\c\\nd"' in output
