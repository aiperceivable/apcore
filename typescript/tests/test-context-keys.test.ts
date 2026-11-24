/**
 * Tests for built-in context key constants.
 */

import { describe, it, expect } from 'vitest';
import {
  TRACING_SPANS,
  TRACING_SAMPLED,
  METRICS_STARTS,
  LOGGING_START,
  REDACTED_OUTPUT,
  RETRY_COUNT_BASE,
  ContextKey as SubpathContextKey,
} from '../src/context-keys.js';
import { ContextKey } from '../src/context-key.js';

describe('Built-in context keys', () => {
  it('TRACING_SPANS has correct name', () => {
    expect(TRACING_SPANS.name).toBe('_apcore.mw.tracing.spans');
  });

  it('TRACING_SAMPLED has correct name', () => {
    expect(TRACING_SAMPLED.name).toBe('_apcore.mw.tracing.sampled');
  });

  it('METRICS_STARTS has correct name', () => {
    expect(METRICS_STARTS.name).toBe('_apcore.mw.metrics.starts');
  });

  it('LOGGING_START has correct name', () => {
    expect(LOGGING_START.name).toBe('_apcore.mw.logging.start_time');
  });

  it('REDACTED_OUTPUT has correct name', () => {
    expect(REDACTED_OUTPUT.name).toBe('_apcore.executor.redacted_output');
  });

  it('RETRY_COUNT_BASE has correct name', () => {
    expect(RETRY_COUNT_BASE.name).toBe('_apcore.mw.retry.count');
  });

  it('RETRY_COUNT_BASE scoped produces correct name', () => {
    expect(RETRY_COUNT_BASE.scoped('my_module').name).toBe('_apcore.mw.retry.count.my_module');
  });

  it('all keys are ContextKey instances', () => {
    for (const key of [
      TRACING_SPANS,
      TRACING_SAMPLED,
      METRICS_STARTS,
      LOGGING_START,
      REDACTED_OUTPUT,
      RETRY_COUNT_BASE,
    ]) {
      expect(key).toBeInstanceOf(ContextKey);
    }
  });

  // C5 (cross-language sync): the README advertises the `apcore-js/context-keys`
  // subpath — mapped to this module — as an entry point for `ContextKey<T>`
  // itself, not only for the pre-built constants.
  it('re-exports the ContextKey class from the context-keys subpath module', () => {
    expect(SubpathContextKey).toBeDefined();
    expect(SubpathContextKey).toBe(ContextKey);
    const custom = new SubpathContextKey<number>('app.custom');
    expect(custom.name).toBe('app.custom');
  });
});
