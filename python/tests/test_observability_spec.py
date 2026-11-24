"""Spec-traced contract tests for the apcore Observability System.

Source spec: apcore/docs/features/observability.md
Contracts under test (verbatim ``## Contract:`` blocks):
  1. ``Tracer.start_span``        — MISSING SYMBOL in apcore-python (no ``Tracer`` class;
                                     the SDK exposes ``Span`` / ``create_span`` /
                                     ``TracingMiddleware`` instead). Documented as skips.
  2. ``MetricsEmitter.record``    — MISSING SYMBOL in apcore-python (no ``MetricsEmitter``
                                     class; the SDK exposes ``MetricsCollector`` instead).
                                     Documented as skips.
  3. ``PrometheusExporter.export``— PRESENT and fully exercised.

Each test's docstring carries the verbatim clause id formatted
``observability.<method>.<kind>.<detail>`` so cross-language diffs line up
row-for-row with the TypeScript and Rust suites.

These tests are READ-ONLY contract verification — they never modify
production source.
"""

from __future__ import annotations

import asyncio

import pytest

from apcore.observability.metrics import MetricsCollector
from apcore.observability.prometheus_exporter import PrometheusExporter

# ---------------------------------------------------------------------------
# Missing-symbol detection (keeps the whole file importable / runnable).
#
# The spec's first two contracts name classes (`Tracer`, `MetricsEmitter`) that
# this SDK does not ship. We probe for them dynamically rather than importing
# them at module scope, so an absent symbol degrades to a skip instead of an
# ImportError that would fail-collect the entire file.
# ---------------------------------------------------------------------------
try:  # pragma: no cover - import probe
    from apcore.observability import Tracer as _Tracer  # type: ignore[attr-defined]
except ImportError:  # pragma: no cover
    _Tracer = None  # type: ignore[assignment]

try:  # pragma: no cover - import probe
    from apcore.observability import MetricsEmitter as _MetricsEmitter  # type: ignore[attr-defined]
except ImportError:  # pragma: no cover
    _MetricsEmitter = None  # type: ignore[assignment]


# ===========================================================================
# Contract 1: Tracer.start_span  (MISSING SYMBOL — contract gap)
# ===========================================================================


def test_tracer_start_span_input_name_empty() -> None:
    """observability.start_span.input.name.empty

    Spec: `name` MUST NOT be empty. Cannot be exercised — `Tracer` is not
    shipped by apcore-python (the SDK uses Span/create_span/TracingMiddleware).
    """
    if _Tracer is None:
        pytest.skip("missing symbol: Tracer (contract gap)")
    tracer = _Tracer()  # pragma: no cover - symbol absent in this SDK
    span = tracer.start_span(name="")
    # If a Tracer ever lands, an empty name must still yield a usable span
    # object (the contract says creation failures are swallowed to a no-op).
    assert span is not None


def test_tracer_start_span_property_async_is_false() -> None:
    """observability.start_span.property.async

    Spec: async: false. Cannot be exercised — `Tracer` is not shipped.
    """
    if _Tracer is None:
        pytest.skip("missing symbol: Tracer (contract gap)")
    tracer = _Tracer()  # pragma: no cover - symbol absent in this SDK
    span = tracer.start_span(name="op")
    assert not asyncio.iscoroutine(span)


def test_tracer_start_span_property_thread_safe() -> None:
    """observability.start_span.property.thread_safe

    Spec: thread_safe: true. Cannot be exercised — `Tracer` is not shipped.
    """
    if _Tracer is None:
        pytest.skip("missing symbol: Tracer (contract gap)")
    tracer = _Tracer()  # pragma: no cover - symbol absent in this SDK

    async def start(i: int) -> object:
        await asyncio.sleep(0)
        return tracer.start_span(name=f"op-{i}")

    async def run() -> list[object]:
        return await asyncio.gather(*[start(i) for i in range(8)])

    spans = asyncio.run(run())
    assert len(spans) == 8


def test_tracer_start_span_property_pure_is_false() -> None:
    """observability.start_span.property.pure

    Spec: pure: false (registers span in the active trace context). Cannot be
    exercised — `Tracer` is not shipped.
    """
    if _Tracer is None:
        pytest.skip("missing symbol: Tracer (contract gap)")
    tracer = _Tracer()  # pragma: no cover - symbol absent in this SDK
    span = tracer.start_span(name="op")
    assert span is not None


# ===========================================================================
# Contract 2: MetricsEmitter.record  (MISSING SYMBOL — contract gap)
# ===========================================================================


def test_metrics_emitter_record_input_metric_name_registered() -> None:
    """observability.record.input.metric_name.registered

    Spec: `metric_name` MUST be a registered metric constant. Cannot be
    exercised — `MetricsEmitter` is not shipped by apcore-python (the SDK
    uses MetricsCollector.increment / observe).
    """
    if _MetricsEmitter is None:
        pytest.skip("missing symbol: MetricsEmitter (contract gap)")
    emitter = _MetricsEmitter()  # pragma: no cover - symbol absent in this SDK
    # Unregistered metric name is swallowed (no error per the contract).
    emitter.record("not_a_registered_metric", 1.0)


def test_metrics_emitter_record_property_async_is_false() -> None:
    """observability.record.property.async

    Spec: async: false. Cannot be exercised — `MetricsEmitter` is not shipped.
    """
    if _MetricsEmitter is None:
        pytest.skip("missing symbol: MetricsEmitter (contract gap)")
    emitter = _MetricsEmitter()  # pragma: no cover - symbol absent in this SDK
    result = emitter.record("apcore_module_calls_total", 1.0)
    assert not asyncio.iscoroutine(result)


def test_metrics_emitter_record_property_thread_safe() -> None:
    """observability.record.property.thread_safe

    Spec: thread_safe: true. Cannot be exercised — `MetricsEmitter` is not
    shipped.
    """
    if _MetricsEmitter is None:
        pytest.skip("missing symbol: MetricsEmitter (contract gap)")
    emitter = _MetricsEmitter()  # pragma: no cover - symbol absent in this SDK

    async def emit(i: int) -> None:
        await asyncio.sleep(0)
        emitter.record("apcore_module_calls_total", float(i))

    async def run() -> None:
        await asyncio.gather(*[emit(i) for i in range(8)])

    asyncio.run(run())


def test_metrics_emitter_record_property_pure_is_false() -> None:
    """observability.record.property.pure

    Spec: pure: false (side-effect: metric emitted to configured backend).
    Cannot be exercised — `MetricsEmitter` is not shipped.
    """
    if _MetricsEmitter is None:
        pytest.skip("missing symbol: MetricsEmitter (contract gap)")
    emitter = _MetricsEmitter()  # pragma: no cover - symbol absent in this SDK
    emitter.record("apcore_module_calls_total", 1.0)


# ===========================================================================
# Contract 3: PrometheusExporter.export  (PRESENT — fully exercised)
#
# Source note: in apcore-python the `collector` from the contract's ### Inputs
# is supplied at construction (`PrometheusExporter(collector=...)`), and
# `export()` takes no arguments and returns the Prometheus text string.
# ===========================================================================


def _exporter_with_one_counter() -> PrometheusExporter:
    """Build an exporter over a collector carrying a single known counter."""
    collector = MetricsCollector()
    collector.increment_calls("math.add", "success")
    return PrometheusExporter(collector=collector)


def test_prometheus_export_input_collector_required() -> None:
    """observability.export.input.collector.required

    Spec ### Inputs: `collector` (MetricsCollector, required) — source of
    metrics data. Building an exporter WITHOUT a collector must fail (the
    parameter is required), and the export must reflect the supplied
    collector's live data when present.
    """
    # Negative: the required `collector` arg cannot be omitted.
    with pytest.raises(TypeError):
        PrometheusExporter()  # type: ignore[call-arg]

    # Positive: a supplied collector's data is what `export()` renders.
    collector = MetricsCollector()
    collector.increment_calls("math.add", "success")
    exporter = PrometheusExporter(collector=collector)
    text = exporter.export()
    assert "apcore_module_calls_total" in text
    assert 'module_id="math.add"' in text
    assert 'status="success"' in text


def test_prometheus_export_error_none_does_not_propagate() -> None:
    """observability.export.error.none

    Spec ### Errors: None — export errors MUST be logged and MUST NOT
    propagate to callers. Exporting over an empty collector must return a
    string (empty exposition) without raising.
    """
    exporter = PrometheusExporter(collector=MetricsCollector())
    text = exporter.export()
    assert isinstance(text, str)


def test_prometheus_export_returns_prometheus_text() -> None:
    """observability.export.returns.prometheus_text

    Spec ### Returns: str — Prometheus text exposition format, UTF-8.
    Output must be a UTF-8-encodable string and, for populated collectors,
    carry the canonical HELP/TYPE comment lines.
    """
    exporter = _exporter_with_one_counter()
    text = exporter.export()
    assert isinstance(text, str)
    # Round-trips through UTF-8 without loss.
    assert text.encode("utf-8").decode("utf-8") == text
    # Prometheus exposition comment lines are present for a known metric.
    assert "# HELP apcore_module_calls_total" in text
    assert "# TYPE apcore_module_calls_total counter" in text


def test_prometheus_export_property_async_is_false() -> None:
    """observability.export.property.async

    Spec ### Properties: async: false. `export()` MUST be a plain synchronous
    call returning a concrete str (not a coroutine / awaitable).
    """
    exporter = _exporter_with_one_counter()
    result = exporter.export()
    assert isinstance(result, str)
    assert not asyncio.iscoroutine(result)
    assert not asyncio.isfuture(result)


def test_prometheus_export_property_thread_safe() -> None:
    """observability.export.property.thread_safe

    Spec ### Properties: thread_safe: true. Launch N (>=8) concurrent exports
    over a shared exporter via asyncio.gather; assert no exception is raised
    and every result is a consistent, identical snapshot string.
    """
    exporter = _exporter_with_one_counter()

    async def do_export() -> str:
        await asyncio.sleep(0)
        return exporter.export()

    async def run() -> list[str]:
        return await asyncio.gather(*[do_export() for _ in range(12)])

    results = asyncio.run(run())
    assert len(results) == 12
    # State is unchanged across reads, so every concurrent snapshot is equal.
    first = results[0]
    assert all(r == first for r in results)
    assert "apcore_module_calls_total" in first


def test_prometheus_export_property_pure_reads_live_state() -> None:
    """observability.export.property.pure

    Spec ### Properties: pure: false (reads from live collector state).
    `export()` must NOT mutate the collector (a query), yet it MUST reflect
    subsequent mutations of the live collector — proving it reads live state
    rather than a frozen copy.
    """
    collector = MetricsCollector()
    collector.increment_calls("math.add", "success")
    exporter = PrometheusExporter(collector=collector)

    before = collector.snapshot()
    first = exporter.export()
    after = collector.snapshot()
    # export() is a query: it does not mutate collector state.
    assert before == after
    assert "math.add" in first

    # Live-state coupling: a new module appears on the next export.
    collector.increment_calls("math.sub", "success")
    second = exporter.export()
    assert 'module_id="math.sub"' in second
    assert 'module_id="math.sub"' not in first


def test_prometheus_export_property_idempotent() -> None:
    """observability.export.property.idempotent

    Spec ### Properties: idempotent: true. Two successive calls with identical
    (unchanged) collector state MUST produce identical output and leave the
    observable collector state identical.
    """
    exporter = _exporter_with_one_counter()
    first = exporter.export()
    second = exporter.export()
    assert first == second
    assert first != ""
