import { describe, it, expect, vi, afterEach } from 'vitest';
import { Context } from '../../src/context.js';
import {
  createSpan,
  InMemoryExporter,
  OTLPExporter,
  StdoutExporter,
  TracingMiddleware,
} from '../../src/observability/tracing.js';

describe('Span', () => {
  it('createSpan creates span with defaults', () => {
    const span = createSpan({
      traceId: 'trace-1',
      name: 'test.span',
      startTime: 100,
    });
    expect(span.traceId).toBe('trace-1');
    expect(span.name).toBe('test.span');
    expect(span.startTime).toBe(100);
    expect(span.spanId).toBeDefined();
    expect(span.parentSpanId).toBeNull();
    expect(span.status).toBe('ok');
    expect(span.events).toEqual([]);
  });
});

describe('InMemoryExporter', () => {
  it('collects and retrieves spans', () => {
    const exporter = new InMemoryExporter();
    const span = createSpan({ traceId: 't1', name: 'test', startTime: 0 });
    exporter.export(span);
    expect(exporter.getSpans()).toHaveLength(1);
    expect(exporter.getSpans()[0].traceId).toBe('t1');
  });

  it('respects max_spans limit', () => {
    const exporter = new InMemoryExporter(3);
    for (let i = 0; i < 5; i++) {
      exporter.export(createSpan({ traceId: `t${i}`, name: 'test', startTime: i }));
    }
    const spans = exporter.getSpans();
    expect(spans).toHaveLength(3);
    expect(spans[0].traceId).toBe('t2');
  });

  it('clear removes all spans', () => {
    const exporter = new InMemoryExporter();
    exporter.export(createSpan({ traceId: 't1', name: 'test', startTime: 0 }));
    exporter.clear();
    expect(exporter.getSpans()).toHaveLength(0);
  });
});

describe('TracingMiddleware', () => {
  it('creates and exports spans on success', () => {
    const exporter = new InMemoryExporter();
    const mw = new TracingMiddleware(exporter);
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.after('mod.a', {}, { result: 'ok' }, ctx);

    const spans = exporter.getSpans();
    expect(spans).toHaveLength(1);
    expect(spans[0].name).toBe('apcore.module.execute');
    expect(spans[0].status).toBe('ok');
    expect(spans[0].attributes['module_id']).toBe('mod.a');
  });

  it('creates error spans', () => {
    const exporter = new InMemoryExporter();
    const mw = new TracingMiddleware(exporter);
    const ctx = Context.create();

    mw.before('mod.err', {}, ctx);
    mw.onError('mod.err', {}, new Error('fail'), ctx);

    const spans = exporter.getSpans();
    expect(spans).toHaveLength(1);
    expect(spans[0].status).toBe('error');
  });

  it('supports nested spans with parent chain', () => {
    const exporter = new InMemoryExporter();
    const mw = new TracingMiddleware(exporter);
    const ctx = Context.create();

    mw.before('mod.outer', {}, ctx);
    mw.before('mod.inner', {}, ctx);
    mw.after('mod.inner', {}, {}, ctx);
    mw.after('mod.outer', {}, {}, ctx);

    const spans = exporter.getSpans();
    expect(spans).toHaveLength(2);
    // inner span has outer as parent
    expect(spans[0].parentSpanId).toBe(spans[1].spanId);
  });

  it('off strategy does not export', () => {
    const exporter = new InMemoryExporter();
    const mw = new TracingMiddleware(exporter, 1.0, 'off');
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.after('mod.a', {}, {}, ctx);

    expect(exporter.getSpans()).toHaveLength(0);
  });

  it('error_first exports errors even when not sampled', () => {
    const exporter = new InMemoryExporter();
    const mw = new TracingMiddleware(exporter, 0.0, 'error_first');
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.onError('mod.a', {}, new Error('fail'), ctx);

    expect(exporter.getSpans()).toHaveLength(1);
  });

  it('throws on invalid sampling rate', () => {
    const exporter = new InMemoryExporter();
    expect(() => new TracingMiddleware(exporter, -0.1)).toThrow();
    expect(() => new TracingMiddleware(exporter, 1.5)).toThrow();
  });

  it('throws on invalid sampling strategy', () => {
    const exporter = new InMemoryExporter();
    expect(() => new TracingMiddleware(exporter, 1.0, 'invalid')).toThrow();
  });

  describe('setExporter', () => {
    it('rejects null exporter', () => {
      const mw = new TracingMiddleware(new InMemoryExporter());
      expect(() => mw.setExporter(null as any)).toThrow('exporter must implement SpanExporter interface');
    });

    it('rejects object without export method', () => {
      const mw = new TracingMiddleware(new InMemoryExporter());
      expect(() => mw.setExporter({} as any)).toThrow('exporter must implement SpanExporter interface');
    });

    it('accepts valid exporter', () => {
      const mw = new TracingMiddleware(new InMemoryExporter());
      const newExporter = { export: () => {} };
      expect(() => mw.setExporter(newExporter)).not.toThrow();
    });
  });
});

describe('OTLPExporter', () => {
  const originalFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  function makeTestSpan() {
    return createSpan({
      traceId: 'trace-abc',
      name: 'test.op',
      startTime: 1000,
      spanId: 'span-123',
      parentSpanId: 'parent-456',
      attributes: { foo: 'bar' },
    });
  }

  it('calls fetch with correct URL and payload shape', async () => {
    const mockFetch = vi.fn().mockResolvedValue(new Response('ok'));
    globalThis.fetch = mockFetch;

    const exporter = new OTLPExporter();
    const span = makeTestSpan();
    span.endTime = 2000;
    span.status = 'ok';
    exporter.export(span);

    // Wait for the fire-and-forget promise
    await vi.waitFor(() => expect(mockFetch).toHaveBeenCalledTimes(1));

    const [url, options] = mockFetch.mock.calls[0];
    expect(url).toBe('http://localhost:4318/v1/traces');
    expect(options.method).toBe('POST');
    expect(options.headers['Content-Type']).toBe('application/json');

    const body = JSON.parse(options.body);
    expect(body.resourceSpans).toHaveLength(1);
    expect(body.resourceSpans[0].resource.attributes[0].key).toBe('service.name');
    expect(body.resourceSpans[0].resource.attributes[0].value.stringValue).toBe('apcore');

    const exportedSpan = body.resourceSpans[0].scopeSpans[0].spans[0];
    expect(exportedSpan.traceId).toBe('trace-abc');
    expect(exportedSpan.spanId).toBe('span-123');
    expect(exportedSpan.parentSpanId).toBe('parent-456');
    expect(exportedSpan.name).toBe('test.op');
    expect(exportedSpan.status.code).toBe(1);
    expect(exportedSpan.attributes).toEqual([
      { key: 'foo', value: { stringValue: 'bar' } },
    ]);
  });

  it('uses default endpoint when none provided', () => {
    const mockFetch = vi.fn().mockResolvedValue(new Response('ok'));
    globalThis.fetch = mockFetch;

    const exporter = new OTLPExporter();
    exporter.export(makeTestSpan());

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(mockFetch.mock.calls[0][0]).toBe('http://localhost:4318/v1/traces');
  });

  it('uses custom endpoint when provided', () => {
    const mockFetch = vi.fn().mockResolvedValue(new Response('ok'));
    globalThis.fetch = mockFetch;

    const exporter = new OTLPExporter({ endpoint: 'http://custom:9999/traces' });
    exporter.export(makeTestSpan());

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(mockFetch.mock.calls[0][0]).toBe('http://custom:9999/traces');
  });

  it('includes custom headers in fetch call', () => {
    const mockFetch = vi.fn().mockResolvedValue(new Response('ok'));
    globalThis.fetch = mockFetch;

    const exporter = new OTLPExporter({
      headers: { 'X-Api-Key': 'secret-key', 'X-Custom': 'value' },
    });
    exporter.export(makeTestSpan());

    const headers = mockFetch.mock.calls[0][1].headers;
    expect(headers['X-Api-Key']).toBe('secret-key');
    expect(headers['X-Custom']).toBe('value');
    expect(headers['Content-Type']).toBe('application/json');
  });

  it('silently catches network errors', async () => {
    const mockFetch = vi.fn().mockRejectedValue(new Error('network down'));
    globalThis.fetch = mockFetch;

    const exporter = new OTLPExporter();
    // Should not throw
    expect(() => exporter.export(makeTestSpan())).not.toThrow();

    // Wait for the rejected promise to be caught
    await vi.waitFor(() => expect(mockFetch).toHaveBeenCalledTimes(1));
  });

  it('converts timestamps to nanoseconds in OTLP payload', async () => {
    const mockFetch = vi.fn().mockResolvedValue(new Response('ok'));
    globalThis.fetch = mockFetch;

    const exporter = new OTLPExporter();
    const span = createSpan({
      traceId: 'trace-ns',
      name: 'ns.test',
      startTime: 1700000000.123,
      spanId: 'span-ns',
    });
    span.endTime = 1700000002.456;
    exporter.export(span);

    await vi.waitFor(() => expect(mockFetch).toHaveBeenCalledTimes(1));

    const body = JSON.parse(mockFetch.mock.calls[0][1].body);
    const exportedSpan = body.resourceSpans[0].scopeSpans[0].spans[0];

    // startTime 1700000000.123 * 1_000_000_000 = 1700000000123000000
    const expectedStartNano = String(Math.round(1700000000.123 * 1_000_000_000));
    expect(exportedSpan.startTimeUnixNano).toBe(expectedStartNano);

    // endTime 1700000002.456 * 1_000_000_000 = 1700000002456000000
    const expectedEndNano = String(Math.round(1700000002.456 * 1_000_000_000));
    expect(exportedSpan.endTimeUnixNano).toBe(expectedEndNano);
  });

  it('omits endTimeUnixNano when endTime is null', async () => {
    const mockFetch = vi.fn().mockResolvedValue(new Response('ok'));
    globalThis.fetch = mockFetch;

    const exporter = new OTLPExporter();
    const span = createSpan({
      traceId: 'trace-no-end',
      name: 'no.end',
      startTime: 1700000000,
      spanId: 'span-no-end',
    });
    // endTime is null by default
    exporter.export(span);

    await vi.waitFor(() => expect(mockFetch).toHaveBeenCalledTimes(1));

    const body = JSON.parse(mockFetch.mock.calls[0][1].body);
    const exportedSpan = body.resourceSpans[0].scopeSpans[0].spans[0];

    expect(exportedSpan.startTimeUnixNano).toBe(String(Math.round(1700000000 * 1_000_000_000)));
    expect(exportedSpan.endTimeUnixNano).toBeUndefined();
  });
});
