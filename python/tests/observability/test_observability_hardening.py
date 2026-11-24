"""Conformance tests for Observability Hardening (Issue #43).

Covers all 10 test cases defined in the canonical conformance fixture at:
  apcore/conformance/fixtures/observability_hardening.json

  1.  pluggable_store_default_inmemory
  2.  batch_processor_buffers_spans
  3.  batch_processor_drops_on_full_queue
  4.  error_history_evicts_oldest_first
  5.  error_fingerprint_dedup_same_error
  6.  error_fingerprint_normalization
  7.  fingerprint_different_errors_no_collision
  8.  redaction_field_pattern_match
  9.  redaction_value_pattern_match
  10. prometheus_format_includes_required_metrics
"""

from __future__ import annotations

import sys

import io
import json
import time
from typing import Any


from apcore.context import Context
from apcore.trace_context import TraceParent
from apcore.errors import ModuleError
from apcore.observability.context_logger import ObsLoggingMiddleware, RedactionConfig
from apcore.observability.error_history import (
    ErrorHistory,
    compute_fingerprint,
    normalize_message,
)
from apcore.observability.metrics import MetricsCollector
from apcore.observability.store import InMemoryObservabilityStore
from apcore.observability.tracing import BatchSpanProcessor, InMemoryExporter, Span
from conformance.canonical_fixtures import (
    case_ids,
    dispatch_or_fail,
    load_fixture,
    reject_unknown_expectations,
)

FIXTURE = "observability_hardening.json"

#: canonical case id -> case body. Hand-written assertions, fixture-sourced
#: values: an `expected` key no assertion reads is not a contract.
_CASES: dict[str, Any] = {case["id"]: case for case in load_fixture(FIXTURE)["test_cases"]}


def _context_for(log_entry: dict[str, Any]) -> Context:
    """Build a Context carrying the log entry's declared correlation fields.

    ``trace_id`` is seeded through a W3C traceparent (the only supported entry
    point — ``Context.create`` takes no ``trace_id``), and ``caller_id`` comes
    from ``Context.child``, which is the only thing that sets it.
    """
    root = Context.create(
        trace_parent=TraceParent(
            version="00",
            trace_id=log_entry["trace_id"],
            parent_id="0123456789abcdef",
            trace_flags="01",
        )
    )
    caller = root.child(log_entry["caller_id"])
    return caller.child(log_entry["module_id"])


def _case(case_id: str, known: set[str] | None = None) -> dict[str, Any]:
    """Return the canonical case *case_id*, rejecting expectations nobody reads.

    apcore#93: nine of this fixture's ten cases used to be transcribed into the
    driver as literals rather than read from it, so mutating any declared value
    left the suite green. Every case body below now takes its inputs AND its
    expectations from here.
    """
    case = _CASES[case_id]
    reject_unknown_expectations(FIXTURE, case, known if known is not None else {"expected"})
    return case


# ---------------------------------------------------------------------------
# 1. pluggable_store_default_inmemory
# ---------------------------------------------------------------------------


class TestPluggableStoreDefaultInMemory:
    """Case: pluggable_store_default_inmemory"""

    def test_error_history_default_store_is_inmemory(self) -> None:
        """ErrorHistory() uses the store type the fixture declares."""
        case = _case("pluggable_store_default_inmemory")
        assert (
            case["input"]["constructor_args"] == {}
        ), "this driver models the no-argument constructor the fixture describes"
        history = ErrorHistory()
        assert type(history.store).__name__ == case["expected"]["store_type"], (
            f"[{case['id']}] fixture declares store_type "
            f"{case['expected']['store_type']!r}, got {type(history.store).__name__!r}"
        )

    def test_metrics_collector_default_store_is_inmemory(self) -> None:
        """MetricsCollector() uses the same declared default store type."""
        case = _case("pluggable_store_default_inmemory")
        collector = MetricsCollector()
        assert type(collector._store).__name__ == case["expected"]["store_type"], (
            f"[{case['id']}] fixture declares store_type "
            f"{case['expected']['store_type']!r}, got {type(collector._store).__name__!r}"
        )

    def test_error_history_accepts_injected_store(self) -> None:
        """ErrorHistory accepts an explicit store at construction time."""
        store = InMemoryObservabilityStore()
        history = ErrorHistory(store=store)
        assert history.store is store

    def test_store_not_settable_after_construction(self) -> None:
        """The store is immutable after construction (no setter)."""
        history = ErrorHistory()
        assert not hasattr(type(history), "store") or isinstance(getattr(type(history), "store", None), property)


# ---------------------------------------------------------------------------
# 2. batch_processor_buffers_spans
# ---------------------------------------------------------------------------


class TestBatchProcessorBuffersSpans:
    """Case: batch_processor_buffers_spans"""

    def test_spans_enqueued_not_exported_immediately(self) -> None:
        """Spans submitted to BatchSpanProcessor are enqueued; not exported immediately."""
        case = _case("batch_processor_buffers_spans")
        params, expected = case["input"], case["expected"]
        exporter = InMemoryExporter()
        processor = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=2048,
            # At least the fixture's delay, and long enough that the background
            # flush cannot fire mid-test — so "exported immediately" means it.
            schedule_delay_ms=max(params["schedule_delay_ms"], 60_000),
        )
        try:
            for _ in range(params["spans_submitted"]):
                span = Span(trace_id="t1", name="test", start_time=time.time())
                processor.on_span_end(span)

            exported_immediately = len(exporter.get_spans())
            assert exported_immediately == expected["spans_exported_immediately"], (
                f"BatchSpanProcessor exported {exported_immediately} span(s) before any flush; "
                f"the fixture requires {expected['spans_exported_immediately']}"
            )
            assert processor.queue_size == expected["queue_size"]
            assert processor.spans_dropped == expected["spans_dropped"]
        finally:
            processor.shutdown()

    def test_spans_exported_after_flush(self) -> None:
        """Spans are exported when the background thread flushes."""
        exporter = InMemoryExporter()
        processor = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=2048,
            schedule_delay_ms=50,  # flush quickly
        )
        try:
            for _ in range(3):
                span = Span(trace_id="t1", name="test", start_time=time.time())
                processor.on_span_end(span)

            # Wait for background flush
            deadline = time.time() + 2.0
            while len(exporter.get_spans()) < 3 and time.time() < deadline:
                time.sleep(0.01)

            assert len(exporter.get_spans()) == 3
        finally:
            processor.shutdown()


# ---------------------------------------------------------------------------
# 3. batch_processor_drops_on_full_queue
# ---------------------------------------------------------------------------


class TestBatchProcessorDropsOnFullQueue:
    """Case: batch_processor_drops_on_full_queue"""

    def test_drops_spans_when_queue_full(self) -> None:
        """When queue is at max_queue_size, additional spans are dropped."""
        case = _case("batch_processor_drops_on_full_queue")
        params, expected = case["input"], case["expected"]
        exporter = InMemoryExporter()
        processor = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=params["max_queue_size"],
            schedule_delay_ms=60_000,
        )
        try:
            # Fill queue to the fixture's starting depth.
            for _ in range(params["queue_size_before"]):
                span = Span(trace_id="t1", name="test", start_time=time.time())
                processor.on_span_end(span)

            assert processor.queue_size == params["queue_size_before"]
            assert processor.spans_dropped == 0

            for _ in range(params["new_spans_submitted"]):
                span = Span(trace_id="t1", name="overflow", start_time=time.time())
                processor.on_span_end(span)

            assert processor.queue_size == expected["queue_size_after"], (
                f"[{case['id']}] fixture declares queue_size_after="
                f"{expected['queue_size_after']}, got {processor.queue_size}"
            )
            assert processor.spans_dropped == expected["spans_dropped"], (
                f"[{case['id']}] fixture declares spans_dropped="
                f"{expected['spans_dropped']}, got {processor.spans_dropped}"
            )
        finally:
            processor.shutdown()

    def test_spans_dropped_counter_accumulates(self) -> None:
        """spans_dropped accumulates across multiple overflow batches."""
        exporter = InMemoryExporter()
        processor = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=2,
            schedule_delay_ms=60_000,
        )
        try:
            for _ in range(2):
                processor.on_span_end(Span(trace_id="t1", name="ok", start_time=time.time()))

            for _ in range(5):
                processor.on_span_end(Span(trace_id="t1", name="over", start_time=time.time()))

            assert processor.spans_dropped == 5
        finally:
            processor.shutdown()


# ---------------------------------------------------------------------------
# 4. error_history_evicts_oldest_first
# ---------------------------------------------------------------------------


class TestErrorHistoryEvictsOldestFirst:
    """Case: error_history_evicts_oldest_first"""

    def test_evicts_oldest_last_seen_at_on_overflow(self) -> None:
        """When at capacity, the entry with the oldest last_seen_at is evicted."""
        case = _case("error_history_evicts_oldest_first")
        params, expected = case["input"], case["expected"]
        history = ErrorHistory(max_total_entries=params["max_total_entries"])

        # Recorded in the fixture's declared order, which is ascending
        # last_seen_at — so the first entry is the eviction candidate.
        for entry in params["existing_entries"]:
            history.record(
                entry["module_id"],
                ModuleError(code=entry["code"], message=f"err {entry['code']}"),
            )
        assert len(history.get_all()) == len(params["existing_entries"])

        new_entry = params["new_entry"]
        history.record(
            new_entry["module_id"],
            ModuleError(code=new_entry["code"], message=f"err {new_entry['code']}"),
        )

        remaining = history.get_all()
        codes = {e.code for e in remaining}
        assert len(remaining) == expected["total_entries"], (
            f"[{case['id']}] fixture declares total_entries={expected['total_entries']}, " f"got {len(remaining)}"
        )
        assert expected["evicted_entry_code"] not in codes, (
            f"[{case['id']}] fixture declares {expected['evicted_entry_code']!r} evicted, "
            f"but it survived: {sorted(codes)}"
        )
        assert codes == set(expected["remaining_entry_codes"]), (
            f"[{case['id']}] fixture declares remaining "
            f"{sorted(expected['remaining_entry_codes'])}, got {sorted(codes)}"
        )

    def test_total_entries_stays_at_limit(self) -> None:
        """Total entries never exceed max_total_entries after overflow."""
        history = ErrorHistory(max_total_entries=5)
        for i in range(10):
            history.record(f"mod.{i}", ModuleError(code=f"E{i}", message="err"))
        assert len(history.get_all()) == 5


# ---------------------------------------------------------------------------
# 5. error_fingerprint_dedup_same_error
# ---------------------------------------------------------------------------


class TestErrorFingerprintDedupSameError:
    """Case: error_fingerprint_dedup_same_error"""

    def test_identical_error_increments_count(self) -> None:
        """Recording the fixture's identical records dedups into one entry."""
        case = _case("error_fingerprint_dedup_same_error")
        params, expected = case["input"], case["expected"]
        history = ErrorHistory()
        for record in params["records"]:
            history.record(
                record["module_id"],
                ModuleError(code=record["code"], message=record["message"]),
            )

        assert len(history.get_all()) == expected["total_entries"], (
            f"[{case['id']}] fixture declares total_entries={expected['total_entries']}, "
            f"got {len(history.get_all())}"
        )
        entries = history.get(params["records"][0]["module_id"])
        assert len(entries) == expected["total_entries"]
        assert entries[0].count == expected["entry_count"], (
            f"[{case['id']}] fixture declares entry_count={expected['entry_count']}, " f"got {entries[0].count}"
        )

    def test_fingerprint_stored_on_entry(self) -> None:
        """ErrorEntry stores a non-empty fingerprint field."""
        history = ErrorHistory()
        history.record("mod.a", ModuleError(code="ERR", message="msg"))
        entry = history.get("mod.a")[0]
        assert len(entry.fingerprint) == 64
        assert all(c in "0123456789abcdef" for c in entry.fingerprint)


# ---------------------------------------------------------------------------
# 6. error_fingerprint_normalization
# ---------------------------------------------------------------------------


class TestErrorFingerprintNormalization:
    """Case: error_fingerprint_normalization"""

    def test_uuid_normalized_to_placeholder(self) -> None:
        """Each declared message normalizes to the declared normalized form."""
        case = _case("error_fingerprint_normalization")
        params, expected = case["input"], case["expected"]
        normalized = [normalize_message(m) for m in params["messages"]]
        assert normalized == expected["normalized_messages"], (
            f"[{case['id']}] fixture declares normalized_messages="
            f"{expected['normalized_messages']}, got {normalized}"
        )

    def test_same_normalized_message_produces_equal_fingerprints(self) -> None:
        """Fingerprint equality across the declared messages matches the fixture."""
        case = _case("error_fingerprint_normalization")
        params, expected = case["input"], case["expected"]
        fingerprints = [compute_fingerprint(params["code"], params["module_id"], m) for m in params["messages"]]
        equal = len(set(fingerprints)) == 1
        assert equal is expected["fingerprints_equal"], (
            f"[{case['id']}] fixture declares fingerprints_equal="
            f"{expected['fingerprints_equal']}, got {equal} for {fingerprints}"
        )

    def test_large_integers_normalized(self) -> None:
        """Integers with 4+ digits (with word boundaries) are replaced with <ID>."""
        assert normalize_message("failed 30000 times") == "failed <id> times"

    def test_iso_timestamp_normalized(self) -> None:
        """ISO 8601 date strings are replaced with <TIMESTAMP> (applied before integers)."""
        msg = "event at 2026-01-01T10:00:00Z"
        assert normalize_message(msg) == "event at <timestamp>"

    def test_uuid_dedup_via_record(self) -> None:
        """Errors with different UUIDs in the message are deduplicated into one entry."""
        history = ErrorHistory()
        history.record(
            "executor.auth",
            ModuleError(
                code="TOKEN_INVALID",
                message="token a1b2c3d4-e5f6-7890-abcd-ef1234567890 is invalid",
            ),
        )
        history.record(
            "executor.auth",
            ModuleError(
                code="TOKEN_INVALID",
                message="token 00000000-0000-0000-0000-000000000001 is invalid",
            ),
        )
        entries = history.get("executor.auth")
        assert len(entries) == 1
        assert entries[0].count == 2


# ---------------------------------------------------------------------------
# 7. fingerprint_different_errors_no_collision
# ---------------------------------------------------------------------------


class TestFingerprintDifferentErrorsNoCollision:
    """Case: fingerprint_different_errors_no_collision"""

    def test_different_codes_different_fingerprints(self) -> None:
        """Fingerprint equality across the declared entries matches the fixture."""
        case = _case("fingerprint_different_errors_no_collision")
        params, expected = case["input"], case["expected"]
        fingerprints = [compute_fingerprint(e["code"], e["module_id"], e["message"]) for e in params["entries"]]
        equal = len(set(fingerprints)) == 1
        assert equal is expected["fingerprints_equal"], (
            f"[{case['id']}] fixture declares fingerprints_equal="
            f"{expected['fingerprints_equal']}, got {equal} for {fingerprints}"
        )

    def test_different_module_ids_different_fingerprints(self) -> None:
        """Same code and message in different modules produce distinct fingerprints."""
        fp1 = compute_fingerprint("ERR", "mod.a", "msg")
        fp2 = compute_fingerprint("ERR", "mod.b", "msg")
        assert fp1 != fp2

    def test_fingerprint_is_64_char_hex(self) -> None:
        """compute_fingerprint always returns a 64-character lowercase hex string."""
        fp = compute_fingerprint("ERR", "mod", "message")
        assert len(fp) == 64
        assert fp == fp.lower()
        assert all(c in "0123456789abcdef" for c in fp)


# ---------------------------------------------------------------------------
# 8/9. redaction_field_pattern_match, redaction_value_pattern_match
# ---------------------------------------------------------------------------
#
# apcore#93. Both cases used to be transcribed: the patterns, the replacement,
# the log entry and the expected output were driver literals, and the three
# ``*_present`` booleans reached no assertion at all — the field case checked
# only that ``module_id``/``caller_id`` were KEYS of ``extra`` ("value may be
# None"), which an implementation that drops correlation entirely still
# satisfies. One driver now runs both cases straight off the fixture, and the
# correlation fields are asserted by VALUE against the log entry that declared
# them.


def _assert_redaction_case(case_id: str) -> None:
    from apcore.observability.context_logger import ContextLogger

    case = _case(case_id)
    params, expected = case["input"], case["expected"]
    cfg = params["redaction_config"]
    entry = params["log_entry"]

    buf = io.StringIO()
    config = RedactionConfig(
        field_patterns=list(cfg["field_patterns"]),
        value_patterns=list(cfg["value_patterns"]),
        replacement=cfg["replacement"],
    )
    ctx = _context_for(entry)
    logger = ContextLogger.from_context(ctx, name="test", output=buf, output_format="json")
    mw = ObsLoggingMiddleware(logger=logger, log_inputs=True, redaction_config=config)

    # D-131: the middleware logs the value the capture point wrote, never the
    # raw inputs it is handed — so the log entry's inputs go where the pipeline
    # puts them (no schema here, so the captured value equals the input).
    ctx.redacted_inputs = dict(entry["inputs"])
    mw.before(entry["module_id"], dict(entry["inputs"]), ctx)

    record = json.loads(buf.getvalue().strip())
    assert record["extra"]["inputs"] == expected["logged_inputs"], (
        f"[{case_id}] fixture declares logged_inputs={expected['logged_inputs']}, " f"got {record['extra']['inputs']}"
    )

    # Correlation fields must SURVIVE redaction — present and carrying the
    # value the log entry declared, not merely present as a null key.
    for field in ("trace_id", "caller_id", "module_id"):
        present = record.get(field) == entry[field]
        assert present is expected[f"{field}_present"], (
            f"[{case_id}] fixture declares {field}_present="
            f"{expected[f'{field}_present']}; the record carries {record.get(field)!r} "
            f"for a declared {entry[field]!r}"
        )


# ---------------------------------------------------------------------------
# 8. redaction_field_pattern_match
# ---------------------------------------------------------------------------


class TestRedactionFieldPatternMatch:
    """Case: redaction_field_pattern_match"""

    def test_field_matching_glob_is_redacted(self) -> None:
        """Field name matching the declared glob is replaced with the declared string."""
        _assert_redaction_case("redaction_field_pattern_match")

    def test_non_matching_field_not_redacted(self) -> None:
        """Fields not matching any pattern are logged unchanged."""
        config = RedactionConfig(field_patterns=["*secret*"], value_patterns=[])
        buf = io.StringIO()
        from apcore.observability.context_logger import ContextLogger

        logger = ContextLogger(name="test", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_inputs=True, redaction_config=config)

        ctx = Context.create()
        ctx.redacted_inputs = {
            "name": "alice",
            "age": "30",
        }  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        mw.before("mod.a", {"name": "alice", "age": "30"}, ctx)

        log_entry = json.loads(buf.getvalue().strip())
        assert log_entry["extra"]["inputs"]["name"] == "alice"
        assert log_entry["extra"]["inputs"]["age"] == "30"


# ---------------------------------------------------------------------------
# 9. redaction_value_pattern_match
# ---------------------------------------------------------------------------


class TestRedactionValuePatternMatch:
    """Case: redaction_value_pattern_match"""

    def test_value_matching_regex_is_redacted(self) -> None:
        """Field value matching the declared regex is replaced with the declared string."""
        _assert_redaction_case("redaction_value_pattern_match")

    def test_non_matching_value_not_redacted(self) -> None:
        """Values not matching any pattern are logged unchanged."""
        config = RedactionConfig(field_patterns=[], value_patterns=[r"^sk-[A-Za-z0-9]+"])
        buf = io.StringIO()
        from apcore.observability.context_logger import ContextLogger

        logger = ContextLogger(name="test", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_inputs=True, redaction_config=config)

        ctx = Context.create()
        ctx.redacted_inputs = {
            "token": "not-an-sk-token"
        }  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        mw.before("mod.a", {"token": "not-an-sk-token"}, ctx)

        log_entry = json.loads(buf.getvalue().strip())
        assert log_entry["extra"]["inputs"]["token"] == "not-an-sk-token"


# ---------------------------------------------------------------------------
# 10. prometheus_format_includes_required_metrics
# ---------------------------------------------------------------------------

#: The fixture's ``collector_state`` keys name metric FAMILIES; the label sets
#: are wiring, not expectation, so they live here rather than in the fixture.
_METRIC_LABELS: dict[str, dict[str, str]] = {
    "apcore_module_calls_total": {"module_id": "mod.a", "status": "success"},
    "apcore_module_errors_total": {"module_id": "mod.a", "error_code": "ERR"},
    "apcore_module_duration_seconds": {"module_id": "mod.a"},
}

#: ``<family>_observations`` in ``collector_state`` means "observe each of
#: these values on <family>" rather than "increment <family> by N".
_OBSERVATIONS_SUFFIX = "_observations"


def _assert_prometheus_text(case_id: str, output: str, required: list[str]) -> None:
    """``format: "prometheus_text"`` made observable.

    Containment alone is satisfied by an exporter that prints the metric names
    in a comment and nothing else, so the Prometheus text-format framing is
    asserted too: every required family carries its ``# HELP`` and ``# TYPE``
    lines, and at least one sample line.
    """
    for name in required:
        assert f"# HELP {name}" in output, f"[{case_id}] prometheus_text output must carry a HELP line for {name!r}"
        assert f"# TYPE {name} " in output, f"[{case_id}] prometheus_text output must carry a TYPE line for {name!r}"
    samples = [
        line
        for line in output.splitlines()
        if line and not line.startswith("#") and any(line.startswith(n) for n in required)
    ]
    assert samples, f"[{case_id}] prometheus_text output carried no sample line for any of {required}"


#: fixture ``expected.format`` -> the check that makes it observable. A
#: dispatch with no ``else`` would let an unrecognised format skip the
#: assertion entirely (apcore#93).
_EXPORT_FORMAT_CHECKS: dict[str, Any] = {"prometheus_text": _assert_prometheus_text}


class TestPrometheusFormatIncludesRequiredMetrics:
    """Case: prometheus_format_includes_required_metrics"""

    def test_output_contains_all_three_required_metrics(self) -> None:
        """The exported text contains every metric name the fixture requires.

        apcore#93: the three names and the collector state were driver
        literals, and ``expected.format`` was read by nothing at all. Both now
        come from the fixture; an unrecognised ``format`` is a hard failure
        rather than a skipped branch.
        """
        case = _case("prometheus_format_includes_required_metrics")
        params, expected = case["input"], case["expected"]

        collector = MetricsCollector()
        for metric, value in params["collector_state"].items():
            if metric.endswith(_OBSERVATIONS_SUFFIX):
                name = metric[: -len(_OBSERVATIONS_SUFFIX)]
                for observation in value:
                    collector.observe(name, _METRIC_LABELS[name], observation)
            else:
                collector.increment(metric, _METRIC_LABELS[metric], amount=value)

        output = collector.export_prometheus()

        missing = [name for name in expected["output_contains"] if name not in output]
        assert missing == [], (
            f"[{case['id']}] fixture requires output_contains="
            f"{expected['output_contains']}; missing from the export: {missing}"
        )
        check_format = dispatch_or_fail(
            FIXTURE,
            case["id"],
            expected["format"],
            _EXPORT_FORMAT_CHECKS,
            "export format",
        )
        check_format(case["id"], output, expected["output_contains"])

    def test_prometheus_text_format_conventions(self) -> None:
        """Output follows Prometheus text format: HELP, TYPE lines, then metric lines."""
        collector = MetricsCollector()
        collector.increment_calls("m", "success")
        output = collector.export_prometheus()
        assert "# HELP apcore_module_calls_total" in output
        assert "# TYPE apcore_module_calls_total counter" in output

    def test_prometheus_exporter_export_method(self) -> None:
        """PrometheusExporter.export() delegates to MetricsCollector.export_prometheus()."""
        from apcore.observability.prometheus_exporter import PrometheusExporter

        collector = MetricsCollector()
        collector.increment_calls("mod.a", "success")
        exporter = PrometheusExporter(collector=collector)
        output = exporter.export()
        assert "apcore_module_calls_total" in output


# ---------------------------------------------------------------------------
# Fixture coverage guard
# ---------------------------------------------------------------------------


class TestFixtureCoverage:
    """Every case in the canonical fixture has a driver class in this file.

    The assertions above are hand-written rather than generated from the
    fixture. That is fine, but the fixture used to be named only in the module
    docstring, so a case added on the spec side left no trace here. This guard
    closes that gap: a new canonical case fails until someone writes the class.
    """

    FIXTURE = "observability_hardening.json"

    #: canonical case id -> the class in this module that asserts it.
    COVERED: dict[str, str] = {
        "pluggable_store_default_inmemory": "TestPluggableStoreDefaultInMemory",
        "batch_processor_buffers_spans": "TestBatchProcessorBuffersSpans",
        "batch_processor_drops_on_full_queue": "TestBatchProcessorDropsOnFullQueue",
        "error_history_evicts_oldest_first": "TestErrorHistoryEvictsOldestFirst",
        "error_fingerprint_dedup_same_error": "TestErrorFingerprintDedupSameError",
        "error_fingerprint_normalization": "TestErrorFingerprintNormalization",
        "fingerprint_different_errors_no_collision": "TestFingerprintDifferentErrorsNoCollision",
        "redaction_field_pattern_match": "TestRedactionFieldPatternMatch",
        "redaction_value_pattern_match": "TestRedactionValuePatternMatch",
        "prometheus_format_includes_required_metrics": "TestPrometheusFormatIncludesRequiredMetrics",
    }

    def test_every_canonical_case_is_claimed(self) -> None:
        canonical = set(case_ids(self.FIXTURE))
        claimed = set(self.COVERED)
        assert canonical - claimed == set(), f"canonical fixture {self.FIXTURE} gained case(s) with no driver here"
        assert claimed - canonical == set(), f"this file claims case(s) {self.FIXTURE} no longer defines"

    def test_every_claimed_class_exists(self) -> None:
        module = sys.modules[__name__]
        missing = [cls for cls in self.COVERED.values() if not hasattr(module, cls)]
        assert missing == [], f"claimed driver class(es) not defined: {missing}"
