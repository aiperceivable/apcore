import { describe, it, expect } from 'vitest';
import { propagateError } from '../../src/utils/error-propagation.js';
import { ModuleError, ModuleExecuteError, ModuleTimeoutError } from '../../src/errors.js';
import { Context } from '../../src/context.js';

describe('propagateError', () => {
  const ctx = new Context('trace-123', 'caller', ['caller', 'target']);

  it('wraps raw Error as ModuleExecuteError', () => {
    const raw = new TypeError('something broke');
    const result = propagateError(raw, 'my.module', ctx);
    expect(result).toBeInstanceOf(ModuleExecuteError);
    expect(result.message).toContain('TypeError');
    expect(result.message).toContain('something broke');
    expect(result.details['call_chain']).toEqual(['caller', 'target']);
  });

  it('enriches existing ModuleError with trace context', () => {
    const err = new ModuleTimeoutError('my.module', 5000);
    const result = propagateError(err, 'my.module', ctx);
    expect(result).toBe(err); // Same object, enriched
    expect(result.traceId).toBe('trace-123');
    expect(result.details['module_id']).toBe('my.module');
    expect(result.details['call_chain']).toEqual(['caller', 'target']);
  });

  it('does not overwrite existing traceId on ModuleError', () => {
    const err = new ModuleError('TEST', 'test', {}, undefined, 'existing-trace');
    const result = propagateError(err, 'my.module', ctx);
    expect(result.traceId).toBe('existing-trace');
  });

  it('does not overwrite existing module_id in details', () => {
    const err = new ModuleError('TEST', 'test', { module_id: 'original' });
    const result = propagateError(err, 'my.module', ctx);
    expect(result.details['module_id']).toBe('original');
  });
});
