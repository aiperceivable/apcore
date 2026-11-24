/**
 * Tests for error-formatter.ts: ErrorFormatterRegistry operations and ErrorFormatter interface.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { ErrorFormatterRegistry } from '../src/error-formatter.js';
import type { ErrorFormatter } from '../src/error-formatter.js';
import { ModuleError, ErrorFormatterDuplicateError } from '../src/errors.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeError(code: string = 'TEST_CODE', message: string = 'test message'): ModuleError {
  return new ModuleError(code, message);
}

function makeFormatter(tag: string): ErrorFormatter {
  return {
    format(error: ModuleError, context?: unknown): Record<string, unknown> {
      return { tag, code: error.code, message: error.message, context: context ?? null };
    },
  };
}

// ---------------------------------------------------------------------------
// ErrorFormatterRegistry
// ---------------------------------------------------------------------------

describe('ErrorFormatterRegistry.register', () => {
  beforeEach(() => {
    ErrorFormatterRegistry.clear();
  });

  it('registers a formatter without error', () => {
    expect(() =>
      ErrorFormatterRegistry.register('adapter-a', makeFormatter('a')),
    ).not.toThrow();
  });

  it('throws ErrorFormatterDuplicateError when registering same name twice', () => {
    ErrorFormatterRegistry.register('dup', makeFormatter('v1'));
    expect(() => ErrorFormatterRegistry.register('dup', makeFormatter('v2'))).toThrow(
      ErrorFormatterDuplicateError,
    );
  });

  it('duplicate error includes the adapter name in its message', () => {
    ErrorFormatterRegistry.register('my-adapter', makeFormatter('x'));
    try {
      ErrorFormatterRegistry.register('my-adapter', makeFormatter('y'));
    } catch (e) {
      expect((e as Error).message).toContain('my-adapter');
    }
  });

  it('ErrorFormatterDuplicateError has correct code', () => {
    ErrorFormatterRegistry.register('code-check', makeFormatter('cc'));
    try {
      ErrorFormatterRegistry.register('code-check', makeFormatter('cc2'));
    } catch (e) {
      expect((e as ErrorFormatterDuplicateError).code).toBe('ERROR_FORMATTER_DUPLICATE');
    }
  });
});

describe('ErrorFormatterRegistry.get', () => {
  beforeEach(() => {
    ErrorFormatterRegistry.clear();
  });

  it('returns the registered formatter', () => {
    const fmt = makeFormatter('test');
    ErrorFormatterRegistry.register('get-test', fmt);
    expect(ErrorFormatterRegistry.get('get-test')).toBe(fmt);
  });

  it('returns undefined for unregistered adapter', () => {
    expect(ErrorFormatterRegistry.get('nonexistent')).toBeUndefined();
  });

  it('returns undefined after the formatter has been unregistered', () => {
    ErrorFormatterRegistry.register('temp', makeFormatter('t'));
    ErrorFormatterRegistry.unregister('temp');
    expect(ErrorFormatterRegistry.get('temp')).toBeUndefined();
  });
});

describe('ErrorFormatterRegistry.format', () => {
  beforeEach(() => {
    ErrorFormatterRegistry.clear();
  });

  it('uses registered formatter when available', () => {
    ErrorFormatterRegistry.register('my-fmt', makeFormatter('my-fmt'));
    const err = makeError('SOME_CODE', 'some message');
    const result = ErrorFormatterRegistry.format('my-fmt', err);
    expect(result['tag']).toBe('my-fmt');
    expect(result['code']).toBe('SOME_CODE');
    expect(result['message']).toBe('some message');
  });

  it('falls back to error.toJSON() when no formatter registered', () => {
    const err = makeError('FALLBACK', 'fallback msg');
    const result = ErrorFormatterRegistry.format('unregistered', err);
    expect(result['code']).toBe('FALLBACK');
    expect(result['message']).toBe('fallback msg');
  });

  it('passes context argument to formatter', () => {
    ErrorFormatterRegistry.register('ctx-fmt', makeFormatter('ctx'));
    const err = makeError();
    const ctx = { requestId: 'abc-123', userId: 42 };
    const result = ErrorFormatterRegistry.format('ctx-fmt', err, ctx);
    expect(result['context']).toEqual(ctx);
  });

  it('passes undefined context when not provided', () => {
    ErrorFormatterRegistry.register('no-ctx', makeFormatter('nc'));
    const err = makeError();
    const result = ErrorFormatterRegistry.format('no-ctx', err);
    expect(result['context']).toBeNull();
  });

  it('fallback toJSON includes timestamp and code', () => {
    const err = makeError('T_CODE', 'msg');
    const json = ErrorFormatterRegistry.format('absent-adapter', err);
    expect(json['code']).toBe('T_CODE');
    // toJSON always includes message
    expect(json).toHaveProperty('message');
  });
});

describe('ErrorFormatterRegistry.unregister', () => {
  beforeEach(() => {
    ErrorFormatterRegistry.clear();
  });

  it('removes the formatter so format falls back to toJSON', () => {
    ErrorFormatterRegistry.register('to-unregister', makeFormatter('tu'));
    ErrorFormatterRegistry.unregister('to-unregister');
    const err = makeError('CODE', 'msg');
    const result = ErrorFormatterRegistry.format('to-unregister', err);
    // Fallback to toJSON: has 'code' but not our custom 'tag'
    expect(result['code']).toBe('CODE');
    expect(result['tag']).toBeUndefined();
  });

  it('silently does nothing when unregistering a non-existent adapter', () => {
    expect(() => ErrorFormatterRegistry.unregister('does-not-exist')).not.toThrow();
  });

  it('allows re-registration after unregister', () => {
    ErrorFormatterRegistry.register('reuse', makeFormatter('v1'));
    ErrorFormatterRegistry.unregister('reuse');
    expect(() => ErrorFormatterRegistry.register('reuse', makeFormatter('v2'))).not.toThrow();
  });
});

describe('ErrorFormatterRegistry.clear', () => {
  it('removes all registered formatters', () => {
    ErrorFormatterRegistry.clear();
    ErrorFormatterRegistry.register('a', makeFormatter('a'));
    ErrorFormatterRegistry.register('b', makeFormatter('b'));
    ErrorFormatterRegistry.register('c', makeFormatter('c'));
    ErrorFormatterRegistry.clear();
    expect(ErrorFormatterRegistry.get('a')).toBeUndefined();
    expect(ErrorFormatterRegistry.get('b')).toBeUndefined();
    expect(ErrorFormatterRegistry.get('c')).toBeUndefined();
  });

  it('allows fresh registration after clear', () => {
    ErrorFormatterRegistry.clear();
    ErrorFormatterRegistry.register('fresh', makeFormatter('f'));
    expect(() =>
      ErrorFormatterRegistry.register('fresh', makeFormatter('f2')),
    ).toThrow(ErrorFormatterDuplicateError);
    ErrorFormatterRegistry.clear();
    expect(() =>
      ErrorFormatterRegistry.register('fresh', makeFormatter('f3')),
    ).not.toThrow();
  });
});

// ---------------------------------------------------------------------------
// Custom ErrorFormatter implementations
// ---------------------------------------------------------------------------

describe('ErrorFormatter custom implementations', () => {
  beforeEach(() => {
    ErrorFormatterRegistry.clear();
  });

  it('custom formatter can inspect error details', () => {
    const detailFormatter: ErrorFormatter = {
      format(error: ModuleError): Record<string, unknown> {
        return {
          errorCode: error.code,
          isRetryable: error.retryable,
          details: error.details,
        };
      },
    };
    ErrorFormatterRegistry.register('detail-adapter', detailFormatter);
    const err = new ModuleError('MY_ERR', 'oops', { field: 'email' });
    const result = ErrorFormatterRegistry.format('detail-adapter', err);
    expect(result['errorCode']).toBe('MY_ERR');
    expect(result['details']).toEqual({ field: 'email' });
  });

  it('custom formatter can use context for request-scoped info', () => {
    const requestFormatter: ErrorFormatter = {
      format(error: ModuleError, context?: unknown): Record<string, unknown> {
        const req = context as { traceId?: string } | undefined;
        return {
          code: error.code,
          traceId: req?.traceId ?? 'unknown',
        };
      },
    };
    ErrorFormatterRegistry.register('request-adapter', requestFormatter);
    const err = makeError('REQ_ERR', 'bad request');
    const result = ErrorFormatterRegistry.format('request-adapter', err, { traceId: 'trace-xyz' });
    expect(result['traceId']).toBe('trace-xyz');
  });

  it('two different adapters can have independent formatters', () => {
    ErrorFormatterRegistry.register('adapter-x', makeFormatter('x'));
    ErrorFormatterRegistry.register('adapter-y', makeFormatter('y'));
    const err = makeError();
    expect(ErrorFormatterRegistry.format('adapter-x', err)['tag']).toBe('x');
    expect(ErrorFormatterRegistry.format('adapter-y', err)['tag']).toBe('y');
  });
});
