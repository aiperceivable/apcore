import { describe, it, expect } from 'vitest';
import { Context, createIdentity } from '../../src/context.js';
import { ContextLogger, ObsLoggingMiddleware } from '../../src/observability/context-logger.js';

function createBufferOutput() {
  const lines: string[] = [];
  return {
    output: { write: (s: string) => lines.push(s) },
    lines,
  };
}

describe('ContextLogger', () => {
  it('creates with defaults', () => {
    const logger = new ContextLogger();
    // Should not throw
    expect(logger).toBeDefined();
  });

  it('logs JSON format by default', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    logger.info('test message');
    expect(lines).toHaveLength(1);
    const parsed = JSON.parse(lines[0]);
    expect(parsed.level).toBe('info');
    expect(parsed.message).toBe('test message');
  });

  it('logs text format', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ format: 'text', output });
    logger.info('test message');
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain('[INFO]');
    expect(lines[0]).toContain('test message');
  });

  it('respects log level filtering', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ level: 'warn', output });
    logger.debug('should not appear');
    logger.info('should not appear');
    logger.warn('should appear');
    logger.error('should appear');
    expect(lines).toHaveLength(2);
  });

  it('redacts _secret_ prefix keys', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    logger.info('test', { _secret_token: 'abc123', name: 'Bob' });
    const parsed = JSON.parse(lines[0]);
    expect(parsed.extra._secret_token).toBe('***REDACTED***');
    expect(parsed.extra.name).toBe('Bob');
  });

  it('does not redact when disabled', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output, redactSensitive: false });
    logger.info('test', { _secret_token: 'abc123' });
    const parsed = JSON.parse(lines[0]);
    expect(parsed.extra._secret_token).toBe('abc123');
  });

  it('redacts a configured sensitive key (e.g. password) in extra', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    logger.info('login', { password: 'x', user: 'bob' });
    const parsed = JSON.parse(lines[0]);
    expect(parsed.extra.password).toBe('***REDACTED***');
    expect(parsed.extra.user).toBe('bob');
  });

  it('redacts a configured sensitive key nested inside extra', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    logger.info('login', { creds: { password: 'x', user: 'bob' } });
    const parsed = JSON.parse(lines[0]);
    expect(parsed.extra.creds.password).toBe('***REDACTED***');
    expect(parsed.extra.creds.user).toBe('bob');
  });

  it('redacts _secret_ keys nested inside objects', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    logger.info('test', { user: { name: 'Bob', _secret_password: 'hunter2' } });
    const parsed = JSON.parse(lines[0]);
    expect(parsed.extra.user.name).toBe('Bob');
    expect(parsed.extra.user._secret_password).toBe('***REDACTED***');
  });

  it('redacts _secret_ keys inside array elements', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    logger.info('test', { items: [{ _secret_key: 'shh', label: 'ok' }] });
    const parsed = JSON.parse(lines[0]);
    expect(parsed.extra.items[0]._secret_key).toBe('***REDACTED***');
    expect(parsed.extra.items[0].label).toBe('ok');
  });

  it('logs text format with extra fields included in output', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ format: 'text', output });
    logger.info('test message', { key1: 'val1', key2: 'val2' });
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain('key1=val1');
    expect(lines[0]).toContain('key2=val2');
  });

  it('fromContext sets trace/module/caller', () => {
    const { output, lines } = createBufferOutput();
    const ctx = Context.create(createIdentity('user1'));
    const childCtx = ctx.child('mod.test');
    const logger = ContextLogger.fromContext(childCtx, 'test-logger', { output });
    logger.info('context log');
    const parsed = JSON.parse(lines[0]);
    expect(parsed.trace_id).toBe(ctx.traceId);
    expect(parsed.module_id).toBe('mod.test');
    expect(parsed.logger).toBe('test-logger');
  });

  it('all log levels work', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ level: 'trace', output });
    logger.trace('t');
    logger.debug('d');
    logger.info('i');
    logger.warn('w');
    logger.error('e');
    logger.fatal('f');
    expect(lines).toHaveLength(6);
  });
});

describe('ObsLoggingMiddleware', () => {
  it('logs before and after', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    const mw = new ObsLoggingMiddleware({ logger });
    const ctx = Context.create();

    mw.before('mod.a', { name: 'Alice' }, ctx);
    mw.after('mod.a', { name: 'Alice' }, { result: 'ok' }, ctx);

    expect(lines).toHaveLength(2);
    const before = JSON.parse(lines[0]);
    const after = JSON.parse(lines[1]);
    expect(before.message).toBe('Module call started');
    expect(after.message).toBe('Module call completed');
    expect(after.extra.duration_ms).toBeDefined();
  });

  it('logs onError', () => {
    const { output, lines } = createBufferOutput();
    const logger = new ContextLogger({ output });
    const mw = new ObsLoggingMiddleware({ logger });
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.onError('mod.a', {}, new Error('boom'), ctx);

    expect(lines).toHaveLength(2);
    const errorLog = JSON.parse(lines[1]);
    expect(errorLog.message).toBe('Module call failed');
    expect(errorLog.extra.error_type).toBe('Error');
  });
});
