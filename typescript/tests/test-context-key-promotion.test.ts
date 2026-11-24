import { describe, it, expect } from 'vitest';
import { ContextKey } from '../src/context-key.js';
import { Context } from '../src/context.js';
import {
  TRACING_SPANS, TRACING_SAMPLED, METRICS_STARTS,
  LOGGING_START, REDACTED_OUTPUT, RETRY_COUNT_BASE,
} from '../src/context-keys.js';

describe('ContextKey v0.22.0 promotion (#63)', () => {
  it('builtin identifiers match spec §1.5', () => {
    expect(TRACING_SPANS.name).toBe('_apcore.mw.tracing.spans');
    expect(TRACING_SAMPLED.name).toBe('_apcore.mw.tracing.sampled');
    expect(METRICS_STARTS.name).toBe('_apcore.mw.metrics.starts');
    expect(LOGGING_START.name).toBe('_apcore.mw.logging.start_time');
    expect(REDACTED_OUTPUT.name).toBe('_apcore.executor.redacted_output');
    expect(RETRY_COUNT_BASE.name).toBe('_apcore.mw.retry.count');
  });

  it('key-anchored API round-trips', () => {
    const KEY = new ContextKey<number>('ext.test.retry.count');
    // Issue #66: Context.create() takes (identity, ...) — passing nothing
    // yields a fresh top-level Context with no identity, which is all this
    // round-trip test needs.
    const ctx = Context.create();
    expect(KEY.exists(ctx)).toBe(false);
    KEY.set(ctx, 3);
    expect(KEY.exists(ctx)).toBe(true);
    expect(KEY.get(ctx)).toBe(3);
    KEY.delete(ctx);
    expect(KEY.exists(ctx)).toBe(false);
  });

  it('scoped key works for ext.* namespace', () => {
    const KEY = new ContextKey<string>('ext.myapp.trace');
    const scoped = KEY.scoped('request-1');
    expect(scoped.name).toBe('ext.myapp.trace.request-1');
  });

  it('ContextKey is exported from src/index.ts', async () => {
    const { ContextKey: CK } = await import('../src/index.js');
    expect(CK).toBeDefined();
    expect(new CK('test')).toBeInstanceOf(ContextKey);
  });
});
