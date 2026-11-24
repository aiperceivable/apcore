/**
 * OBS-003 — a span's correlation attributes are a wire payload, so they are
 * snake_case like every other attribute on the same span.
 *
 * `TracingMiddleware.before` wrote `moduleId` / `callerId` in camelCase while
 * apcore-python and apcore-rust write `module_id` / `caller_id` — and all
 * three write the LATER attributes (`duration_ms`, `success`, `error_code`) in
 * snake_case, so a single TypeScript span carried a mixed convention.
 * observability.md names these correlation fields in snake_case.
 */

import { describe, it, expect } from 'vitest';
import { TracingMiddleware } from '../../src/observability/tracing.js';
import type { Span } from '../../src/observability/tracing.js';
import { Context } from '../../src/context.js';

function spansOf(context: Context): Span[] {
  return (context.data['_apcore.mw.tracing.spans'] as Span[]) ?? [];
}

/** Collects exported spans; `after()` exports through it when sampled. */
function makeExporter(): { export: (span: Span) => void; spans: Span[] } {
  const spans: Span[] = [];
  return { export: (span: Span) => { spans.push(span); }, spans };
}

describe('span correlation attributes are snake_case (OBS-003)', () => {
  it('before() writes module_id and caller_id', () => {
    const mw = new TracingMiddleware(makeExporter());
    const context = Context.create().child('executor.email.send');
    mw.before('executor.email.send', {}, context);

    const attrs = spansOf(context)[0].attributes;
    expect(attrs['module_id']).toBe('executor.email.send');
    expect('caller_id' in attrs).toBe(true);
    expect(attrs['moduleId']).toBeUndefined();
    expect(attrs['callerId']).toBeUndefined();
  });

  it('every attribute on one span uses the same convention', () => {
    const mw = new TracingMiddleware(makeExporter());
    const context = Context.create().child('executor.email.send');
    mw.before('executor.email.send', {}, context);
    const span = spansOf(context)[0];
    mw.after('executor.email.send', {}, {}, context);

    for (const key of Object.keys(span.attributes)) {
      expect(key).toBe(key.toLowerCase());
    }
    expect(span.attributes['duration_ms']).toBeTypeOf('number');
    expect(span.attributes['success']).toBe(true);
  });

  it('caller_id carries the context callerId on a nested call', () => {
    const mw = new TracingMiddleware(makeExporter());
    const root = Context.create().child('executor.a');
    const nested = root.child('executor.b');
    mw.before('executor.b', {}, nested);

    const attrs = spansOf(nested)[0].attributes;
    expect(attrs['caller_id']).toBe('executor.a');
  });
});
