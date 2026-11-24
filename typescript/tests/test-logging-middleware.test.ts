/**
 * Tests for middleware/logging.ts — LoggingMiddleware.
 */

import { describe, it, expect, vi } from 'vitest';
import { LoggingMiddleware } from '../src/middleware/logging.js';
import type { Logger } from '../src/middleware/logging.js';
import { Context } from '../src/context.js';

function makeContext(moduleId: string = 'test.mod'): Context {
  const ctx = Context.create();
  return ctx.child(moduleId);
}

function makeLogger(): Logger & { infoCalls: Array<[string, unknown]>; errorCalls: Array<[string, unknown]> } {
  const logger = {
    infoCalls: [] as Array<[string, unknown]>,
    errorCalls: [] as Array<[string, unknown]>,
    info(message: string, extra?: Record<string, unknown>) {
      logger.infoCalls.push([message, extra]);
    },
    error(message: string, extra?: Record<string, unknown>) {
      logger.errorCalls.push([message, extra]);
    },
  };
  return logger;
}

describe('LoggingMiddleware', () => {
  describe('before', () => {
    it('logs module start with inputs when logInputs is true', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logInputs: true });
      const ctx = makeContext('my.module');
      const result = mw.before('my.module', { key: 'val' }, ctx);
      expect(result).toBeNull();
      expect(logger.infoCalls).toHaveLength(1);
      expect(logger.infoCalls[0][0]).toContain('START my.module');
    });

    it('does not log when logInputs is false', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logInputs: false });
      const ctx = makeContext();
      mw.before('mod', { key: 'val' }, ctx);
      expect(logger.infoCalls).toHaveLength(0);
    });

    it('stores start time in context data', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger });
      const ctx = makeContext();
      mw.before('mod', {}, ctx);
      expect(typeof ctx.data['_apcore.mw.logging.start_time']).toBe('number');
    });

    it('uses redacted inputs when available', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logInputs: true });
      const ctx = makeContext();
      ctx.redactedInputs = { key: '***REDACTED***' };
      mw.before('mod', { key: 'secret' }, ctx);
      const extra = logger.infoCalls[0][1] as Record<string, unknown>;
      expect(extra['inputs']).toEqual({ key: '***REDACTED***' });
    });
  });

  describe('after', () => {
    it('logs module end with duration when logOutputs is true', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logOutputs: true });
      const ctx = makeContext('my.module');
      ctx.data['_apcore.mw.logging.start_time'] = performance.now() - 100;
      const result = mw.after('my.module', {}, { result: 'ok' }, ctx);
      expect(result).toBeNull();
      expect(logger.infoCalls).toHaveLength(1);
      expect(logger.infoCalls[0][0]).toContain('END my.module');
      expect(logger.infoCalls[0][0]).toMatch(/\d+\.\d+ms/);
    });

    it('does not log when logOutputs is false', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logOutputs: false });
      const ctx = makeContext();
      mw.after('mod', {}, { result: 'ok' }, ctx);
      expect(logger.infoCalls).toHaveLength(0);
    });

    it('handles missing start time gracefully', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logOutputs: true });
      const ctx = makeContext();
      mw.after('mod', {}, { result: 'ok' }, ctx);
      expect(logger.infoCalls).toHaveLength(1);
    });
  });

  describe('onError', () => {
    it('logs error with redacted inputs when logErrors is true', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logErrors: true });
      const ctx = makeContext('my.module');
      ctx.redactedInputs = { safe: 'data' };
      const error = new Error('something broke');
      const result = mw.onError('my.module', { secret: 'val' }, error, ctx);
      expect(result).toBeNull();
      expect(logger.errorCalls).toHaveLength(1);
      expect(logger.errorCalls[0][0]).toContain('ERROR my.module');
      const extra = logger.errorCalls[0][1] as Record<string, unknown>;
      expect(extra['inputs']).toEqual({ safe: 'data' });
    });

    it('does not log when logErrors is false', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logErrors: false });
      const ctx = makeContext();
      mw.onError('mod', {}, new Error('fail'), ctx);
      expect(logger.errorCalls).toHaveLength(0);
    });

    it('omits inputs rather than logging the raw ones when redactedInputs is null', () => {
      // PROTOCOL_SPEC 10.6.1 requirement 5 (D-131): never the raw inputs.
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger, logErrors: true });
      const ctx = makeContext();
      mw.onError('mod', { raw: 'data' }, new Error('fail'), ctx);
      const extra = logger.errorCalls[0][1] as Record<string, unknown>;
      expect('inputs' in extra).toBe(false);
    });
  });

  describe('priority', () => {
    it('has priority 700 (logging range)', () => {
      const mw = new LoggingMiddleware();
      expect(mw.priority).toBe(700);
    });
  });

  describe('defaults', () => {
    it('uses default logger when none provided', () => {
      const mw = new LoggingMiddleware();
      const ctx = makeContext();
      // Should not throw
      expect(() => mw.before('mod', {}, ctx)).not.toThrow();
    });

    it('enables all logging by default', () => {
      const logger = makeLogger();
      const mw = new LoggingMiddleware({ logger });
      const ctx = makeContext();
      mw.before('mod', {}, ctx);
      mw.after('mod', {}, { r: 1 }, ctx);
      mw.onError('mod', {}, new Error('e'), ctx);
      expect(logger.infoCalls).toHaveLength(2);
      expect(logger.errorCalls).toHaveLength(1);
    });
  });
});
