/**
 * Regression for A-D-EXEC-004 / spec D-19 — `callWithTrace` MUST share
 * `call()`'s error semantics. In particular it MUST run the `on_error`
 * middleware chain so a recovery dict returned from middleware reaches
 * the caller paired with the trace, instead of being shadowed by an
 * unconditional rethrow.
 */

import { describe, expect, it } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { Context } from '../src/context.js';
import { Middleware } from '../src/middleware/base.js';

class RecoveringMiddleware extends Middleware {
  override onError(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _error: Error,
    _context: Context,
  ): Record<string, unknown> | null {
    return { recovered: true, fallback: 'value' };
  }
}

describe('callWithTrace D-19 — runs on_error middleware (A-D-EXEC-004)', () => {
  it('returns the middleware recovery result paired with the trace', async () => {
    const reg = new Registry();
    reg.register('test.failing', {
      id: 'test.failing',
      description: 'always fails',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: () => {
        throw new Error('boom');
      },
    });

    const exec = new Executor({ registry: reg });
    exec.use(new RecoveringMiddleware());

    const [output, trace] = await exec.callWithTrace('test.failing', {});
    expect(output).toEqual({ recovered: true, fallback: 'value' });
    expect(trace).toBeDefined();
  });

  it('rethrows when no middleware recovers', async () => {
    const reg = new Registry();
    reg.register('test.failing2', {
      id: 'test.failing2',
      description: 'always fails',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: () => {
        throw new Error('still broken');
      },
    });

    const exec = new Executor({ registry: reg });
    await expect(exec.callWithTrace('test.failing2', {})).rejects.toThrow('still broken');
  });
});
