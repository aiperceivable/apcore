import { describe, it, expect } from 'vitest';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { Config } from '../src/config.js';
import { Context } from '../src/context.js';
import { ModuleTimeoutError } from '../src/errors.js';

function makeRegistry(): Registry {
  const reg = new Registry();
  return reg;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

describe('Executor global timeout', () => {
  // Spec v1.50.0 D-100: the budget lives in the first-class
  // `Context.globalDeadline` field, not in a private `context.data` key, and
  // D-99 puts it on the epoch-SECONDS clock. These assertions used to read
  // `ctx.data['_apcore.executor.global_deadline']` in milliseconds.
  it('sets globalDeadline on root call', async () => {
    const reg = makeRegistry();
    let captured: number | null = null;
    reg.register('test.mod', {
      id: 'test.mod',
      execute: (_inputs: Record<string, unknown>, ctx: Context) => {
        captured = ctx.globalDeadline;
        return { ok: true };
      },
    });
    const config = new Config({ executor: { default_timeout: 30000, global_timeout: 60000, max_call_depth: 32, max_module_repeat: 3 } });
    const exec = new Executor({ registry: reg, config });

    const before = Date.now() / 1000;
    await exec.call('test.mod');
    expect(typeof captured).toBe('number');
    expect(captured as unknown as number).toBeGreaterThanOrEqual(before + 60);
  });

  it('inherits globalDeadline in nested calls', async () => {
    const reg = makeRegistry();
    let outerDeadline: number | undefined;
    let innerDeadline: number | undefined;

    reg.register('outer', {
      id: 'outer',
      execute: async (_inputs: Record<string, unknown>, ctx: Context) => {
        outerDeadline = ctx.globalDeadline as number;
        // Simulate a nested call by calling inner via executor
        const executor = ctx.executor as Executor;
        return executor.call('inner', {}, ctx);
      },
    });
    reg.register('inner', {
      id: 'inner',
      execute: (_inputs: Record<string, unknown>, ctx: Context) => {
        innerDeadline = ctx.globalDeadline as number;
        return { ok: true };
      },
    });

    const config = new Config({ executor: { default_timeout: 30000, global_timeout: 5000, max_call_depth: 32, max_module_repeat: 3 } });
    const exec = new Executor({ registry: reg, config });

    await exec.call('outer');
    expect(outerDeadline).toBeDefined();
    expect(innerDeadline).toBe(outerDeadline); // Same deadline inherited
  });

  it('throws ModuleTimeoutError when global deadline exceeded', async () => {
    const reg = makeRegistry();
    reg.register('slow', {
      id: 'slow',
      execute: async () => {
        await sleep(200);
        return { ok: true };
      },
    });

    // globalTimeout=50ms, module takes 200ms
    const config = new Config({ executor: { default_timeout: 30000, global_timeout: 50, max_call_depth: 32, max_module_repeat: 3 } });
    const exec = new Executor({ registry: reg, config });

    await expect(exec.call('slow')).rejects.toThrow(ModuleTimeoutError);
  });

  it('does not set deadline when globalTimeout is 0', async () => {
    const reg = makeRegistry();
    let captured: number | null = -1;
    reg.register('test.mod', {
      id: 'test.mod',
      execute: (_inputs: Record<string, unknown>, ctx: Context) => {
        captured = ctx.globalDeadline;
        return { ok: true };
      },
    });

    const config = new Config({ executor: { default_timeout: 30000, global_timeout: 0, max_call_depth: 32, max_module_repeat: 3 } });
    const exec = new Executor({ registry: reg, config });

    await exec.call('test.mod');
    expect(captured).toBeNull();
  });

  it('stream() also sets globalDeadline', async () => {
    const reg = makeRegistry();
    let captured: number | null = null;
    reg.register('test.stream', {
      id: 'test.stream',
      execute: (_inputs: Record<string, unknown>, ctx: Context) => {
        captured = ctx.globalDeadline;
        return { ok: true };
      },
    });

    const config = new Config({ executor: { default_timeout: 30000, global_timeout: 60000, max_call_depth: 32, max_module_repeat: 3 } });
    const exec = new Executor({ registry: reg, config });

    const chunks: Record<string, unknown>[] = [];
    for await (const chunk of exec.stream('test.stream')) {
      chunks.push(chunk);
    }
    expect(typeof captured).toBe('number');
  });
});
