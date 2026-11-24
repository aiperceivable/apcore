/**
 * Regression for A-D-EXEC-001 / spec D-11 (per-module timeout).
 *
 * BuiltinExecute MUST honour each module's declared `resources.timeout`
 * (milliseconds) ahead of the executor-wide default. The global deadline,
 * when present, further clamps the effective timeout — it never extends it.
 */

import { describe, expect, it, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { Config } from '../src/config.js';
import { Context } from '../src/context.js';
import { ModuleTimeoutError, InvalidInputError } from '../src/errors.js';
import { DEFAULT_ANNOTATIONS } from '../src/module.js';

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

describe('BuiltinExecute per-module timeout (D-11)', () => {
  it('uses the module-declared resources.timeout when present', async () => {
    const reg = new Registry();
    reg.register('slow.mod', {
      id: 'slow.mod',
      description: 'always slow',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      resources: { timeout: 30 },
      execute: async () => {
        await sleep(200);
        return { ok: true };
      },
    });

    const config = new Config({ executor: { default_timeout: 5000 } });
    const exec = new Executor({ registry: reg, config });
    await expect(exec.call('slow.mod', {})).rejects.toBeInstanceOf(ModuleTimeoutError);
  });

  it('reads resources.timeout from annotations.resources as well', async () => {
    const reg = new Registry();
    reg.register('slow.ann', {
      id: 'slow.ann',
      description: 'slow via annotations',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      annotations: { resources: { timeout: 40 } },
      execute: async () => {
        await sleep(250);
        return { ok: true };
      },
    });

    const config = new Config({ executor: { default_timeout: 5000 } });
    const exec = new Executor({ registry: reg, config });
    await expect(exec.call('slow.ann', {})).rejects.toBeInstanceOf(ModuleTimeoutError);
  });

  it('falls back to executor.default_timeout when no resources.timeout is declared', async () => {
    const reg = new Registry();
    reg.register('fast.mod', {
      id: 'fast.mod',
      description: 'no module timeout',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      execute: async () => {
        await sleep(10);
        return { ok: true };
      },
    });

    const config = new Config({ executor: { default_timeout: 5000 } });
    const exec = new Executor({ registry: reg, config });
    const result = await exec.call('fast.mod', {});
    expect(result).toEqual({ ok: true });
  });

  it('global deadline clamps the module timeout further when shorter', async () => {
    const reg = new Registry();
    reg.register('slow.global', {
      id: 'slow.global',
      description: 'module says 10s but global deadline says 50ms',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      resources: { timeout: 10_000 },
      execute: async () => {
        await sleep(2_000);
        return { ok: true };
      },
    });

    // global_timeout 50ms — much tighter than module timeout 10s
    const config = new Config({
      executor: { default_timeout: 5000, global_timeout: 50 },
    });
    const exec = new Executor({ registry: reg, config });
    await expect(exec.call('slow.global', {})).rejects.toBeInstanceOf(ModuleTimeoutError);
  });

  it.each([Number.POSITIVE_INFINITY, '20'])(
    'falls back to the annotation budget for a non-finite or non-numeric timeout (%s)',
    async (timeout) => {
      const registry = new Registry();
      registry.register('slow.invalid_budget', {
        description: 'uses the annotation budget',
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        resources: { timeout },
        annotations: { extra: { resources: { timeout: 15 } } },
        execute: async () => {
          await sleep(100);
          return {};
        },
      });
      const executor = new Executor({ registry });
      await expect(executor.call('slow.invalid_budget', {})).rejects.toMatchObject({
        code: 'MODULE_TIMEOUT',
        details: { timeoutMs: 15 },
      });
    },
  );

  it('lets a zero module timeout disable the executor default', async () => {
    const registry = new Registry();
    const execute = vi.fn(async () => {
      await sleep(20);
      return {};
    });
    registry.register('slow.unlimited', {
      description: 'has no per-module time limit',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      resources: { timeout: 0 },
      execute,
    });
    const executor = new Executor({
      registry,
      config: new Config({ executor: { default_timeout: 5 } }),
    });
    await expect(executor.call('slow.unlimited', {})).resolves.toEqual({});
    expect(execute).toHaveBeenCalledOnce();
  });

  it('raises GENERAL_INVALID_INPUT for a negative declared timeout (A-D-W1)', async () => {
    const reg = new Registry();
    reg.register('neg.mod', {
      id: 'neg.mod',
      description: 'declares an invalid negative timeout',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      resources: { timeout: -1 },
      execute: async () => ({ ok: true }),
    });

    const config = new Config({ executor: { default_timeout: 5000 } });
    const exec = new Executor({ registry: reg, config });

    const error = await exec.call('neg.mod', {}).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(InvalidInputError);
    expect((error as InvalidInputError).code).toBe('GENERAL_INVALID_INPUT');
  });
});

describe('module() resources budget (D-11 / high-level API)', () => {
  it('reads the budget from annotations.extra.resources', async () => {
    const { APCore } = await import('../src/client.js');
    const client = new APCore({ config: new Config({ executor: { default_timeout: 5000 } }) });
    client.module({
      id: 'slow.annotated',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      annotations: { ...DEFAULT_ANNOTATIONS, extra: { resources: { timeout: 20 } } },
      execute: async () => {
        await sleep(100);
        return {};
      },
    });
    await expect(client.call('slow.annotated', {})).rejects.toMatchObject({
      code: 'MODULE_TIMEOUT',
      details: { timeoutMs: 20, moduleId: 'slow.annotated' },
    });
  });

  it('carries resources.timeout through the APCore.module() factory', async () => {
    const { APCore } = await import('../src/client.js');
    const config = new Config({ executor: { default_timeout: 5000 } });
    const client = new APCore({ config });
    client.module({
      id: 'slow.high',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      resources: { timeout: 30 },
      execute: async () => {
        await sleep(200);
        return { ok: true };
      },
    });
    await expect(client.call('slow.high', {})).rejects.toBeInstanceOf(ModuleTimeoutError);
  });

  it('warns when a host declares timeoutSec instead of resources.timeout', async () => {
    const { APCore } = await import('../src/client.js');
    const { vi } = await import('vitest');
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const client = new APCore();
    client.module({
      id: 'slow.misdeclared',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      // Deliberately unsupported: the old silent-drop path.
      // @ts-expect-error unknown option
      timeoutSec: 300,
      execute: () => ({}),
    });
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("'timeoutSec'"));
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('resources'));
    warn.mockRestore();
  });
});

describe('MODULE_TIMEOUT retry safety', () => {
  it('is not retryable by default (non-idempotent module)', async () => {
    const reg = new Registry();
    reg.register('slow.side_effect', {
      id: 'slow.side_effect',
      description: 'writes then hangs',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      resources: { timeout: 30 },
      execute: async () => {
        await sleep(200);
        return { ok: true };
      },
    });
    const exec = new Executor({ registry: reg, config: new Config({ executor: { default_timeout: 5000 } }) });
    const failure: unknown = await exec.call('slow.side_effect', {}).catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(ModuleTimeoutError);
    expect((failure as ModuleTimeoutError).retryable).toBe(false);
  });

  it('is retryable when the module declares annotations.idempotent', async () => {
    const reg = new Registry();
    reg.register('slow.idempotent', {
      id: 'slow.idempotent',
      description: 'safe to repeat',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      resources: { timeout: 30 },
      annotations: { idempotent: true },
      execute: async () => {
        await sleep(200);
        return { ok: true };
      },
    });
    const exec = new Executor({ registry: reg, config: new Config({ executor: { default_timeout: 5000 } }) });
    const failure: unknown = await exec.call('slow.idempotent', {}).catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(ModuleTimeoutError);
    expect((failure as ModuleTimeoutError).retryable).toBe(true);
  });
});
