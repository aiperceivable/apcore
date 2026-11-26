"""Public invocation coverage for independent timers and trace spans."""

from __future__ import annotations

import asyncio
import io
import json
from collections.abc import AsyncIterator
from types import SimpleNamespace
from typing import Any

import pytest

from apcore import APCore, Context, ModuleAnnotations, ModuleError, TraceContext
from apcore.observability import context_logger, metrics, tracing, usage


@pytest.mark.asyncio
@pytest.mark.parametrize("finish_order", [(0, 1), (1, 0)], ids=["first-in-first-out", "last-in-first-out"])
async def test_siblings_keep_their_own_timers_spans_and_outbound_headers(
    monkeypatch: pytest.MonkeyPatch, finish_order: tuple[int, int]
) -> None:
    now = 100.0
    clock = SimpleNamespace(time=lambda: now)
    for implementation in (context_logger, metrics, usage, tracing):
        monkeypatch.setattr(implementation, "time", clock)

    client = APCore()
    exporter = tracing.InMemoryExporter()
    metric_store = metrics.MetricsCollector()
    usage_store = usage.UsageCollector()
    logs = io.StringIO()
    client.use(tracing.TracingMiddleware(exporter))
    client.use(metrics.MetricsMiddleware(metric_store))
    client.use(usage.UsageMiddleware(usage_store))
    client.use(context_logger.ObsLoggingMiddleware(context_logger.ContextLogger("probe", output=logs)))

    entered = [asyncio.Event(), asyncio.Event()]
    release = [asyncio.Event(), asyncio.Event()]
    started_at = [110.0, 120.0]
    finished_at: dict[int, float] = {}
    headers: dict[str, str] = {}
    parent = TraceContext.extract({"traceparent": "00-" + "1" * 32 + "-" + "2" * 16 + "-01"})
    shared_data: dict[str, Any] = {"shared": []}
    inbound = Context.create(trace_parent=parent, data=shared_data)

    async def run_child(index: int, context: Context) -> bool:
        assert context.data is shared_data
        entered[index].set()
        await release[index].wait()
        module_id = context.call_chain[-1]
        headers[module_id] = TraceContext.inject(context)["traceparent"]
        if index == 0:
            raise ValueError("child failed")
        return True

    @client.module(id="probe.first")
    async def first(context: Context) -> bool:
        """Run the first child."""
        return await run_child(0, context)

    @client.module(id="probe.second")
    async def second(context: Context) -> bool:
        """Run the second child."""
        return await run_child(1, context)

    @client.module(id="probe.parent")
    async def run_parent(context: Context) -> bool:
        """Run both child calls with controlled, overlapping lifetimes."""
        nonlocal now
        assert context.data is shared_data
        tasks = []
        for index, module_id in enumerate(("probe.first", "probe.second")):
            now = started_at[index]
            tasks.append(asyncio.create_task(client.call_async(module_id, {}, context)))
            await entered[index].wait()
        for index in finish_order:
            now += 5
            finished_at[index] = now
            release[index].set()
            if index == 0:
                with pytest.raises(ModuleError):
                    await tasks[index]
            else:
                await tasks[index]
        headers["probe.parent"] = TraceContext.inject(context)["traceparent"]
        return True

    await asyncio.wait_for(client.call_async("probe.parent", {}, inbound), timeout=2)

    spans = {span.attributes["module_id"]: span for span in exporter.get_spans()}
    root = spans["probe.parent"]
    assert root.trace_id == "1" * 32
    assert headers["probe.parent"].split("-")[2] == root.span_id
    timings = {}
    for index, module_id in enumerate(("probe.first", "probe.second")):
        span = spans[module_id]
        duration_seconds = finished_at[index] - started_at[index]
        timings[module_id] = duration_seconds * 1000
        assert span.parent_span_id == root.span_id
        assert span.attributes["duration_ms"] == timings[module_id]
        assert span.status == ("error" if index == 0 else "ok")
        assert headers[module_id].split("-")[2] == span.span_id
        assert metrics.module_latency_ms(metric_store, module_id)[0] == timings[module_id]
        assert usage_store.get_latencies(module_id) == [timings[module_id]]

    completed = {
        record["extra"]["module_id"]: record["extra"]["duration_ms"]
        for line in logs.getvalue().splitlines()
        if "duration_ms" in (record := json.loads(line))["extra"]
    }
    assert {module_id: completed[module_id] for module_id in timings} == timings
    assert root.parent_span_id == "2" * 16


@pytest.mark.asyncio
async def test_stream_uses_its_local_span_and_remote_parent() -> None:
    class Probe:
        description = "Expose the current trace header"
        input_schema = {"type": "object"}
        output_schema = {"type": "object", "properties": {"parent": {"type": "string"}}}
        annotations = ModuleAnnotations(streaming=True)

        async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            return {"parent": TraceContext.inject(context)["traceparent"].split("-")[2]}

        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
            yield await self.execute(inputs, context)

    exporter = tracing.InMemoryExporter()
    client = APCore()
    client.use(tracing.TracingMiddleware(exporter))
    client.register("probe.native_stream", Probe())
    parent = TraceContext.from_traceparent("00-" + "1" * 32 + "-" + "2" * 16 + "-01")

    chunks = [chunk async for chunk in client.stream("probe.native_stream", {}, Context.create(trace_parent=parent))]

    (span,) = exporter.get_spans()
    assert chunks == [{"parent": span.span_id}]
    assert span.parent_span_id == parent.parent_id


def test_otlp_exports_real_trace_span_and_parent_ids(monkeypatch: pytest.MonkeyPatch) -> None:
    pytest.importorskip("opentelemetry.exporter.otlp.proto.http.trace_exporter")
    proto = pytest.importorskip("opentelemetry.proto.collector.trace.v1.trace_service_pb2")
    requests = pytest.importorskip("requests")
    payloads = []

    def capture(session: Any, request: Any, **kwargs: Any) -> Any:
        assert request.url == "http://collector.example/v1/traces"
        payloads.append(proto.ExportTraceServiceRequest.FromString(request.body))
        response = requests.Response()
        response.status_code = 200
        return response

    monkeypatch.setattr(requests.Session, "send", capture)
    exporter = tracing.OTLPExporter(endpoint="http://collector.example/v1/traces")
    try:
        exporter.export(
            tracing.Span(
                trace_id="1" * 32,
                span_id="2" * 16,
                parent_span_id="3" * 16,
                name="apcore.module.execute",
                start_time=1000.0,
                end_time=1001.0,
            )
        )
    finally:
        exporter.shutdown()

    span = payloads[0].resource_spans[0].scope_spans[0].spans[0]
    assert span.trace_id.hex() == "1" * 32
    assert span.span_id.hex() == "2" * 16
    assert span.parent_span_id.hex() == "3" * 16
