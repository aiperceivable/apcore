import { describe, it, expect } from 'vitest';
import { TraceContext } from '../src/trace-context.js';
import type { TraceParent } from '../src/trace-context.js';
import { Context } from '../src/context.js';
import type { Span } from '../src/observability/tracing.js';

const TRACEPARENT_RE = /^[0-9a-f]{2}-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$/;

describe('TraceContext.inject()', () => {
  it('produces a valid traceparent format', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    expect(headers).toHaveProperty('traceparent');
    expect(headers.traceparent).toMatch(TRACEPARENT_RE);
  });

  it('uses context traceId (already 32-hex)', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    const parts = headers.traceparent.split('-');
    // traceId is already 32-hex format, no dash stripping needed
    // Format: 00-<32hex>-<16hex>-<2hex>
    // Splitting by '-' gives: ["00", <32hex>, <16hex>, <2hex>]
    expect(parts[1]).toBe(ctx.traceId);
  });

  it('starts with version 00', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    expect(headers.traceparent.startsWith('00-')).toBe(true);
  });

  it('ends with trace flags 01', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    expect(headers.traceparent.endsWith('-01')).toBe(true);
  });

  it('uses spanId from tracing stack when available', () => {
    const ctx = Context.create();
    const fakeSpan: Span = {
      traceId: ctx.traceId,
      name: 'test',
      startTime: 0,
      spanId: 'abcdef0123456789',
      parentSpanId: null,
      attributes: {},
      endTime: null,
      status: 'ok',
      events: [],
    };
    ctx.data['_apcore.mw.tracing.spans'] = [fakeSpan];

    const headers = TraceContext.inject(ctx);
    const parts = headers.traceparent.split('-');
    expect(parts[2]).toBe('abcdef0123456789');
  });

  it('generates random parentId when no spans exist', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    const parts = headers.traceparent.split('-');
    const parentId = parts[2];
    expect(parentId).toHaveLength(16);
    expect(parentId).toMatch(/^[0-9a-f]{16}$/);
  });
});

describe('TraceContext.extract()', () => {
  it('parses a valid traceparent header', () => {
    const headers = { traceparent: '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' };
    const result = TraceContext.extract(headers);
    expect(result).not.toBeNull();
    expect(result!.version).toBe('00');
    expect(result!.traceId).toBe('4bf92f3577b34da6a3ce929d0e0e4736');
    expect(result!.parentId).toBe('00f067aa0ba902b7');
    expect(result!.traceFlags).toBe('01');
  });

  it('returns null for missing header', () => {
    const result = TraceContext.extract({});
    expect(result).toBeNull();
  });

  it('returns null for unrelated headers', () => {
    const result = TraceContext.extract({ 'other-header': 'value' });
    expect(result).toBeNull();
  });

  it('returns null for malformed traceparent', () => {
    const result = TraceContext.extract({ traceparent: 'invalid-format' });
    expect(result).toBeNull();
  });

  it('returns null for short trace_id', () => {
    const result = TraceContext.extract({ traceparent: '00-abc-00f067aa0ba902b7-01' });
    expect(result).toBeNull();
  });

  it('normalizes uppercase to lowercase', () => {
    const headers = { traceparent: '00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01' };
    const result = TraceContext.extract(headers);
    expect(result).not.toBeNull();
    expect(result!.traceId).toBe('4bf92f3577b34da6a3ce929d0e0e4736');
  });

  it('parses unsampled trace flags', () => {
    const headers = { traceparent: '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00' };
    const result = TraceContext.extract(headers);
    expect(result).not.toBeNull();
    expect(result!.traceFlags).toBe('00');
  });

  it('returns null for all-zero trace_id', () => {
    const result = TraceContext.extract({ traceparent: '00-00000000000000000000000000000000-00f067aa0ba902b7-01' });
    expect(result).toBeNull();
  });

  it('returns null for all-zero parent_id', () => {
    const result = TraceContext.extract({ traceparent: '00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01' });
    expect(result).toBeNull();
  });

  it('returns null for version ff', () => {
    const result = TraceContext.extract({ traceparent: 'ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' });
    expect(result).toBeNull();
  });

  it('returns a frozen object', () => {
    const headers = { traceparent: '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' };
    const result = TraceContext.extract(headers);
    expect(result).not.toBeNull();
    expect(Object.isFrozen(result)).toBe(true);
  });
});

describe('TraceContext.fromTraceparent()', () => {
  it('parses a valid traceparent string', () => {
    const tp = TraceContext.fromTraceparent('00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01');
    expect(tp.version).toBe('00');
    expect(tp.traceId).toBe('4bf92f3577b34da6a3ce929d0e0e4736');
    expect(tp.parentId).toBe('00f067aa0ba902b7');
    expect(tp.traceFlags).toBe('01');
  });

  it('throws on invalid traceparent', () => {
    expect(() => TraceContext.fromTraceparent('not-a-valid-traceparent'))
      .toThrow('Malformed traceparent');
  });

  it('throws on empty string', () => {
    expect(() => TraceContext.fromTraceparent(''))
      .toThrow('Malformed traceparent');
  });

  it('throws on missing parts', () => {
    expect(() => TraceContext.fromTraceparent('00-4bf92f3577b34da6a3ce929d0e0e4736'))
      .toThrow('Malformed traceparent');
  });

  it('throws on all-zero trace_id', () => {
    expect(() => TraceContext.fromTraceparent('00-00000000000000000000000000000000-00f067aa0ba902b7-01'))
      .toThrow('all-zero trace_id or parent_id');
  });

  it('throws on all-zero parent_id', () => {
    expect(() => TraceContext.fromTraceparent('00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01'))
      .toThrow('all-zero trace_id or parent_id');
  });

  it('throws on version ff', () => {
    expect(() => TraceContext.fromTraceparent('ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01'))
      .toThrow('version ff is not allowed');
  });

  it('returns a frozen object', () => {
    const tp = TraceContext.fromTraceparent('00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01');
    expect(Object.isFrozen(tp)).toBe(true);
  });
});

describe('Round-trip: inject -> extract', () => {
  it('preserves trace_id through inject then extract', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    const parsed = TraceContext.extract(headers);

    expect(parsed).not.toBeNull();
    // traceId is already 32-hex format
    expect(parsed!.traceId).toBe(ctx.traceId);
  });

  it('preserves parent_id through inject then extract', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    const parsed = TraceContext.extract(headers);

    expect(parsed).not.toBeNull();
    const parts = headers.traceparent.split('-');
    expect(parsed!.parentId).toBe(parts[2]);
  });
});

describe('TraceContext.parseTracestate()', () => {
  it('parses a single key=value pair', () => {
    expect(TraceContext.parseTracestate('vendor=value1')).toEqual([['vendor', 'value1']]);
  });

  it('parses multiple comma-separated entries', () => {
    expect(TraceContext.parseTracestate('a=1,b=2,c=3')).toEqual([
      ['a', '1'],
      ['b', '2'],
      ['c', '3'],
    ]);
  });

  it('trims surrounding whitespace on each entry', () => {
    expect(TraceContext.parseTracestate(' a=1 , b=2 ')).toEqual([
      ['a', '1'],
      ['b', '2'],
    ]);
  });

  it('drops malformed entries silently', () => {
    expect(TraceContext.parseTracestate('a=1,nokey,b=2,=novalue,c=3')).toEqual([
      ['a', '1'],
      ['b', '2'],
      ['c', '3'],
    ]);
  });

  it('returns empty array for empty string', () => {
    expect(TraceContext.parseTracestate('')).toEqual([]);
  });

  it('caps at 32 entries per W3C', () => {
    const raw = Array.from({ length: 40 }, (_, i) => `k${i}=v${i}`).join(',');
    const parsed = TraceContext.parseTracestate(raw);
    expect(parsed).toHaveLength(32);
    expect(parsed[0]).toEqual(['k0', 'v0']);
    expect(parsed[31]).toEqual(['k31', 'v31']);
  });

  it('preserves equals signs inside the value', () => {
    expect(TraceContext.parseTracestate('a=foo=bar')).toEqual([['a', 'foo=bar']]);
  });
});

describe('TraceContext.formatTracestate()', () => {
  it('joins entries with commas', () => {
    expect(TraceContext.formatTracestate([['a', '1'], ['b', '2']])).toBe('a=1,b=2');
  });

  it('returns empty string for empty input', () => {
    expect(TraceContext.formatTracestate([])).toBe('');
  });

  it('round-trips with parseTracestate', () => {
    const entries: Array<[string, string]> = [['vendor1', 'opaque1'], ['vendor2', 'opaque2']];
    const formatted = TraceContext.formatTracestate(entries);
    const parsed = TraceContext.parseTracestate(formatted);
    expect(parsed).toEqual(entries);
  });
});

describe('TraceContext.extract() — case-insensitive header lookup', () => {
  const TP = '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01';

  it('finds header with mixed-case key (Traceparent)', () => {
    const result = TraceContext.extract({ Traceparent: TP });
    expect(result).not.toBeNull();
    expect(result!.traceId).toBe('4bf92f3577b34da6a3ce929d0e0e4736');
  });

  it('finds header with all-uppercase key (TRACEPARENT)', () => {
    const result = TraceContext.extract({ TRACEPARENT: TP });
    expect(result).not.toBeNull();
  });

  it('finds tracestate with mixed-case key (TraceState)', () => {
    const result = TraceContext.extract({ traceparent: TP, TraceState: 'a=1,b=2' });
    expect(result).not.toBeNull();
    expect(result!.tracestate).toEqual([['a', '1'], ['b', '2']]);
  });

  it('supports a Headers instance', () => {
    const h = new Headers();
    h.set('traceparent', TP);
    h.set('tracestate', 'a=1');
    const result = TraceContext.extract(h);
    expect(result).not.toBeNull();
    expect(result!.tracestate).toEqual([['a', '1']]);
  });

  it('supports a Map', () => {
    const m = new Map<string, string>();
    m.set('Traceparent', TP);
    m.set('TraceState', 'x=y');
    const result = TraceContext.extract(m);
    expect(result).not.toBeNull();
    expect(result!.tracestate).toEqual([['x', 'y']]);
  });
});

describe('TraceContext.extract() — tracestate population', () => {
  const TP = '00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01';

  it('populates empty tracestate when header absent', () => {
    const result = TraceContext.extract({ traceparent: TP });
    expect(result).not.toBeNull();
    expect(result!.tracestate).toEqual([]);
  });

  it('parses tracestate when header present', () => {
    const result = TraceContext.extract({ traceparent: TP, tracestate: 'congo=t61rcWkgMzE,rojo=00f067aa0ba902b7' });
    expect(result).not.toBeNull();
    expect(result!.tracestate).toEqual([
      ['congo', 't61rcWkgMzE'],
      ['rojo', '00f067aa0ba902b7'],
    ]);
  });
});

describe('TraceContext.inject() — dynamic traceFlags', () => {
  it('propagates inbound traceFlags=00 (unsampled) through inject', () => {
    const ctx = Context.create();
    // Stash an inbound trace parent that came in with flags=00.
    const inbound: TraceParent = {
      version: '00',
      traceId: ctx.traceId,
      parentId: '00f067aa0ba902b7',
      traceFlags: '00',
      tracestate: [],
    };
    ctx.data['_apcore.trace.inbound'] = inbound;

    const headers = TraceContext.inject(ctx);
    expect(headers.traceparent.endsWith('-00')).toBe(true);
  });

  it('defaults to traceFlags=01 for new roots (no inbound)', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    expect(headers.traceparent.endsWith('-01')).toBe(true);
  });

  it('returns tracestate header when inbound has tracestate', () => {
    const ctx = Context.create();
    const inbound: TraceParent = {
      version: '00',
      traceId: ctx.traceId,
      parentId: '00f067aa0ba902b7',
      traceFlags: '01',
      tracestate: [['vendor', 'opaque']],
    };
    ctx.data['_apcore.trace.inbound'] = inbound;

    const headers = TraceContext.inject(ctx);
    expect(headers.tracestate).toBe('vendor=opaque');
  });

  it('omits tracestate header when inbound tracestate is empty', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx);
    expect(headers.tracestate).toBeUndefined();
  });

  // D11-002a: Context.create must propagate the inbound TraceParent into
  // data so downstream TraceContext.inject() can honour the inbound W3C
  // sampling decision and vendor state. Previously TS Context.create only
  // seeded traceId, dropping traceFlags + tracestate, so inbound `00`
  // (unsampled) was silently upgraded to `01` (sampled) downstream.
  it('Context.create propagates inbound traceFlags=00 to inject() without manual stash', () => {
    const inbound: TraceParent = {
      version: '00',
      traceId: '0af7651916cd43dd8448eb211c80319c',
      parentId: '00f067aa0ba902b7',
      traceFlags: '00',
      tracestate: [],
    };
    const ctx = Context.create(null, inbound);
    const headers = TraceContext.inject(ctx);
    expect(headers.traceparent.endsWith('-00')).toBe(true);
  });

  it('Context.create propagates inbound tracestate to inject() without manual stash', () => {
    const inbound: TraceParent = {
      version: '00',
      traceId: '0af7651916cd43dd8448eb211c80319c',
      parentId: '00f067aa0ba902b7',
      traceFlags: '01',
      tracestate: [['vendor', 'opaque']],
    };
    const ctx = Context.create(null, inbound);
    const headers = TraceContext.inject(ctx);
    expect(headers.tracestate).toBe('vendor=opaque');
  });

  it('stores inbound trace state under two scalar keys (cross-language parity)', () => {
    const inbound: TraceParent = {
      version: '00',
      traceId: '0af7651916cd43dd8448eb211c80319c',
      parentId: '00f067aa0ba902b7',
      traceFlags: '00',
      tracestate: [['vendor', 'opaque']],
    };
    const ctx = Context.create(null, inbound);
    // Two scalar keys hold the parsed state; the whole-object key is not used.
    expect(ctx.data['_apcore.trace.flags']).toBe('00');
    expect(ctx.data['_apcore.trace.state']).toEqual([['vendor', 'opaque']]);
    expect(ctx.data['_apcore.trace.inbound']).toBeUndefined();
    // Round-trips through inject via the two-key form.
    const headers = TraceContext.inject(ctx);
    expect(headers.traceparent.endsWith('-00')).toBe(true);
    expect(headers.tracestate).toBe('vendor=opaque');
  });
});

describe('TraceContext.inject() — parentId override', () => {
  it('uses the provided parentId verbatim when valid', () => {
    const ctx = Context.create();
    const headers = TraceContext.inject(ctx, '0123456789abcdef');
    const parts = headers.traceparent.split('-');
    expect(parts[2]).toBe('0123456789abcdef');
  });

  it('throws on malformed parentId (wrong length)', () => {
    const ctx = Context.create();
    expect(() => TraceContext.inject(ctx, 'tooShort')).toThrow();
  });

  it('throws on malformed parentId (non-hex characters)', () => {
    const ctx = Context.create();
    expect(() => TraceContext.inject(ctx, 'XXXXXXXXXXXXXXXX')).toThrow();
  });

  it('throws on uppercase parentId (must be lowercase hex)', () => {
    const ctx = Context.create();
    expect(() => TraceContext.inject(ctx, '0123456789ABCDEF')).toThrow();
  });

  it('still derives parentId from spans when override not provided', () => {
    const ctx = Context.create();
    const fakeSpan: Span = {
      traceId: ctx.traceId,
      name: 'test',
      startTime: 0,
      spanId: 'abcdef0123456789',
      parentSpanId: null,
      attributes: {},
      endTime: null,
      status: 'ok',
      events: [],
    };
    ctx.data['_apcore.mw.tracing.spans'] = [fakeSpan];

    const headers = TraceContext.inject(ctx);
    const parts = headers.traceparent.split('-');
    expect(parts[2]).toBe('abcdef0123456789');
  });
});

describe('Context.create() with traceParent', () => {
  it('uses traceParent traceId as 32-hex format', () => {
    const tp: TraceParent = {
      version: '00',
      traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
      parentId: '00f067aa0ba902b7',
      traceFlags: '01',
      tracestate: [],
    };
    const ctx = Context.create(null, tp);
    expect(ctx.traceId).toBe('4bf92f3577b34da6a3ce929d0e0e4736');
  });

  it('produces a valid 32-hex string from traceParent', () => {
    const tp: TraceParent = {
      version: '00',
      traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
      parentId: '00f067aa0ba902b7',
      traceFlags: '01',
      tracestate: [],
    };
    const ctx = Context.create(null, tp);
    // Should match 32-hex format
    expect(ctx.traceId).toMatch(/^[0-9a-f]{32}$/);
  });

  it('still works without traceParent', () => {
    const ctx = Context.create();
    expect(ctx.traceId).toBeDefined();
    expect(ctx.traceId).toMatch(/^[0-9a-f]{32}$/);
  });

  it('generates unique traceIds without traceParent', () => {
    const ctx1 = Context.create();
    const ctx2 = Context.create();
    expect(ctx1.traceId).not.toBe(ctx2.traceId);
  });

  it('full round-trip: context -> inject -> extract -> create', () => {
    const original = Context.create();
    const headers = TraceContext.inject(original);
    const parsed = TraceContext.extract(headers);
    expect(parsed).not.toBeNull();

    const restored = Context.create(null, parsed);
    expect(restored.traceId).toBe(original.traceId);
  });
});
