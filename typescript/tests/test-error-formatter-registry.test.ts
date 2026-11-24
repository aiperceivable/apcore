/**
 * Tests for ErrorFormatterRegistry (§8.8).
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { ErrorFormatterRegistry } from '../src/error-formatter.js';
import type { ErrorFormatter } from '../src/error-formatter.js';
import { ModuleError, ErrorFormatterDuplicateError } from '../src/errors.js';

// Helper to create a simple test error
function makeError(code: string, message: string): ModuleError {
  return new ModuleError(code, message);
}

// Helper to build a formatter
function makeFormatter(tag: string): ErrorFormatter {
  return {
    format(error: ModuleError, context?: unknown): Record<string, unknown> {
      return { tag, code: error.code, message: error.message, context: context ?? null };
    },
  };
}

describe('ErrorFormatterRegistry', () => {
  beforeEach(() => {
    // Clear registry state between tests
    ErrorFormatterRegistry.clear();
  });

  it('registers a formatter successfully', () => {
    ErrorFormatterRegistry.register('my-adapter', makeFormatter('my-adapter'));
    expect(ErrorFormatterRegistry.get('my-adapter')).toBeDefined();
  });

  it('get returns undefined for unregistered adapter', () => {
    expect(ErrorFormatterRegistry.get('nonexistent')).toBeUndefined();
  });

  it('throws ErrorFormatterDuplicateError on duplicate registration', () => {
    ErrorFormatterRegistry.register('dup-adapter', makeFormatter('dup'));
    expect(() => ErrorFormatterRegistry.register('dup-adapter', makeFormatter('dup2')))
      .toThrow(ErrorFormatterDuplicateError);
  });

  it('format uses registered formatter when available', () => {
    ErrorFormatterRegistry.register('fmt-adapter', makeFormatter('fmt'));
    const err = makeError('TEST_CODE', 'test message');
    const result = ErrorFormatterRegistry.format('fmt-adapter', err);
    expect(result['tag']).toBe('fmt');
    expect(result['code']).toBe('TEST_CODE');
    expect(result['message']).toBe('test message');
  });

  it('format falls back to error.toJSON() when no formatter registered', () => {
    const err = makeError('FALLBACK_CODE', 'fallback message');
    const result = ErrorFormatterRegistry.format('unregistered-adapter', err);
    // toJSON() always includes code and message
    expect(result['code']).toBe('FALLBACK_CODE');
    expect(result['message']).toBe('fallback message');
  });

  it('format passes context to the formatter', () => {
    ErrorFormatterRegistry.register('ctx-adapter', makeFormatter('ctx'));
    const err = makeError('CODE', 'msg');
    const ctx = { requestId: 'abc-123' };
    const result = ErrorFormatterRegistry.format('ctx-adapter', err, ctx);
    expect(result['context']).toEqual(ctx);
  });

  it('unregister removes a formatter', () => {
    ErrorFormatterRegistry.register('to-remove', makeFormatter('tr'));
    ErrorFormatterRegistry.unregister('to-remove');
    expect(ErrorFormatterRegistry.get('to-remove')).toBeUndefined();
  });

  it('allows re-registration after unregister', () => {
    ErrorFormatterRegistry.register('reuse', makeFormatter('v1'));
    ErrorFormatterRegistry.unregister('reuse');
    expect(() => ErrorFormatterRegistry.register('reuse', makeFormatter('v2'))).not.toThrow();
  });

  it('clear removes all formatters', () => {
    ErrorFormatterRegistry.register('a', makeFormatter('a'));
    ErrorFormatterRegistry.register('b', makeFormatter('b'));
    ErrorFormatterRegistry.clear();
    expect(ErrorFormatterRegistry.get('a')).toBeUndefined();
    expect(ErrorFormatterRegistry.get('b')).toBeUndefined();
  });

  it('ErrorFormatterDuplicateError has correct error code', () => {
    ErrorFormatterRegistry.register('code-check', makeFormatter('cc'));
    try {
      ErrorFormatterRegistry.register('code-check', makeFormatter('cc2'));
      expect.fail('should have thrown');
    } catch (e) {
      expect(e).toBeInstanceOf(ErrorFormatterDuplicateError);
      expect((e as ErrorFormatterDuplicateError).code).toBe('ERROR_FORMATTER_DUPLICATE');
    }
  });
});
