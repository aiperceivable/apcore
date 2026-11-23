/**
 * Unit tests for APCore client class.
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, it, expect, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { FunctionModule } from '../src/decorator.js';
import { Executor } from '../src/executor.js';
import { ModuleNotFoundError } from '../src/errors.js';
import { Middleware } from '../src/middleware/index.js';
import { Registry } from '../src/registry/registry.js';
import type { Context } from '../src/context.js';
import type { PipelineState, StepMiddleware } from '../src/pipeline.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const AddInputSchema = Type.Object({ a: Type.Number(), b: Type.Number() });
const AddOutputSchema = Type.Object({ result: Type.Number() });

function registerAdd(client: APCore, id = 'math.add'): FunctionModule {
  return client.module({
    id,
    inputSchema: AddInputSchema,
    outputSchema: AddOutputSchema,
    description: 'Add two numbers',
    execute: (inputs) => ({ result: (inputs.a as number) + (inputs.b as number) }),
  });
}

class TrackingMiddleware extends Middleware {
  beforeCalled = false;
  afterCalled = false;

  override before(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _context: Context,
  ): null {
    this.beforeCalled = true;
    return null;
  }

  override after(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    _context: Context,
  ): null {
    this.afterCalled = true;
    return null;
  }
}

// ---------------------------------------------------------------------------
// Construction tests
// ---------------------------------------------------------------------------

describe('APCore construction', () => {
  it('creates default Registry and Executor', () => {
    const client = new APCore();
    expect(client.registry).toBeInstanceOf(Registry);
    expect(client.executor).toBeInstanceOf(Executor);
    expect(client.config).toBeNull();
  });

  it('accepts a custom Registry', () => {
    const registry = new Registry();
    const client = new APCore({ registry });
    expect(client.registry).toBe(registry);
  });

  it('accepts a custom Executor', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const client = new APCore({ registry, executor });
    expect(client.executor).toBe(executor);
  });

  it('passes config through to auto-created Executor', () => {
    const config = new Config({ extensions: { root: '/tmp' } });
    const client = new APCore({ config });
    expect(client.config).toBe(config);
  });

  it('loads config from file path via Config.load()', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-test-'));
    const configPath = path.join(tempDir, 'apcore.yaml');
    // `version` and `project.name` are the only two keys a config file must
    // declare (§9.1) — Config.load validates by default.
    fs.writeFileSync(
      configPath,
      'version: "1.0.0"\nproject:\n  name: client-test\nextensions:\n  root: /custom/path\n',
    );

    try {
      const config = Config.load(configPath);
      const client = new APCore({ config });
      expect(client.config).toBeDefined();
      expect(client.config?.get('extensions.root')).toBe('/custom/path');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });
});

// ---------------------------------------------------------------------------
// Module registration tests
// ---------------------------------------------------------------------------

describe('APCore.module()', () => {
  it('returns a FunctionModule', () => {
    const client = new APCore();
    const fm = registerAdd(client);
    expect(fm).toBeInstanceOf(FunctionModule);
  });

  it('registers the module in the registry', () => {
    const client = new APCore();
    registerAdd(client);
    expect(client.registry.has('math.add')).toBe(true);
  });

  it('sets description and metadata', () => {
    const client = new APCore();
    const fm = client.module({
      id: 'math.add',
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      tags: ['math'],
      version: '2.0.0',
      execute: (inputs) => ({ result: (inputs.a as number) + (inputs.b as number) }),
    });
    expect(fm.description).toBe('Add two numbers');
    expect(fm.tags).toEqual(['math']);
    expect(fm.version).toBe('2.0.0');
  });
});

describe('APCore.register()', () => {
  it('registers a class-based module', async () => {
    const client = new APCore();

    const addModule = {
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      async execute(inputs: Record<string, unknown>) {
        return { result: (inputs.a as number) + (inputs.b as number) };
      },
    };

    client.register('math.add', addModule);
    expect(client.registry.has('math.add')).toBe(true);

    const result = await client.call('math.add', { a: 10, b: 5 });
    expect(result).toEqual({ result: 15 });
  });

  it('returns a promise that settles only after an async onLoad has completed', async () => {
    const client = new APCore();
    let loaded = false;
    const mod = {
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      async onLoad() {
        await new Promise((r) => setTimeout(r, 5));
        loaded = true;
      },
      execute(inputs: Record<string, unknown>) {
        return { result: (inputs.a as number) + (inputs.b as number) };
      },
    };

    const pending = client.register('math.add', mod);
    expect(pending).toBeInstanceOf(Promise);
    await pending;

    expect(loaded).toBe(true);
    expect(client.registry.has('math.add')).toBe(true);
    expect(await client.call('math.add', { a: 1, b: 2 })).toEqual({ result: 3 });
  });

  it('surfaces an async onLoad rejection to the caller', async () => {
    const client = new APCore();
    const mod = {
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      async onLoad() {
        throw new Error('load boom');
      },
      execute: () => ({ result: 0 }),
    };

    await expect(client.register('math.add', mod)).rejects.toThrow('load boom');
    expect(client.registry.has('math.add')).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Call tests
// ---------------------------------------------------------------------------

describe('APCore.call()', () => {
  it('executes a module and returns the result', async () => {
    const client = new APCore();
    registerAdd(client);

    const result = await client.call('math.add', { a: 10, b: 5 });
    expect(result).toEqual({ result: 15 });
  });

  it('throws ModuleNotFoundError for unknown module', async () => {
    const client = new APCore();
    await expect(client.call('nonexistent.module', { a: 1 })).rejects.toThrow(ModuleNotFoundError);
  });
});

// ---------------------------------------------------------------------------
// Middleware tests
// ---------------------------------------------------------------------------

describe('APCore.use()', () => {
  it('returns self for chaining', () => {
    const client = new APCore();
    const mw = new TrackingMiddleware();
    const result = client.use(mw);
    expect(result).toBe(client);
  });

  it('supports chaining multiple middlewares', () => {
    const client = new APCore();
    const mw1 = new TrackingMiddleware();
    const mw2 = new TrackingMiddleware();
    const result = client.use(mw1).use(mw2);
    expect(result).toBe(client);
  });

  it('fires middleware during call', async () => {
    const client = new APCore();
    const mw = new TrackingMiddleware();
    client.use(mw);
    registerAdd(client);

    await client.call('math.add', { a: 1, b: 2 });
    expect(mw.beforeCalled).toBe(true);
    expect(mw.afterCalled).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Discover tests
// ---------------------------------------------------------------------------

describe('APCore.discover()', () => {
  it('delegates to registry.discover()', async () => {
    const client = new APCore();
    // Without a valid extensions config, discover() should throw ConfigNotFoundError
    await expect(client.discover()).rejects.toThrow('extensions');
  });
});

// ---------------------------------------------------------------------------
// ListModules tests
// ---------------------------------------------------------------------------

describe('APCore.listModules()', () => {
  it('returns empty array when no modules registered', () => {
    const client = new APCore();
    expect(client.listModules()).toEqual([]);
  });

  it('returns sorted module IDs', () => {
    const client = new APCore();
    registerAdd(client, 'math.add');

    client.module({
      id: 'greet.hello',
      inputSchema: Type.Object({ name: Type.String() }),
      outputSchema: Type.Object({ message: Type.String() }),
      execute: (inputs) => ({ message: `Hello, ${inputs.name}` }),
    });

    expect(client.listModules()).toEqual(['greet.hello', 'math.add']);
  });

  it('filters by prefix', () => {
    const client = new APCore();
    registerAdd(client, 'math.add');

    client.module({
      id: 'greet.hello',
      inputSchema: Type.Object({ name: Type.String() }),
      outputSchema: Type.Object({ message: Type.String() }),
      execute: (inputs) => ({ message: `Hello, ${inputs.name}` }),
    });

    expect(client.listModules({ prefix: 'math' })).toEqual(['math.add']);
  });

  it('filters by tags', () => {
    const client = new APCore();

    client.module({
      id: 'math.add',
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      tags: ['math', 'core'],
      execute: (inputs) => ({ result: (inputs.a as number) + (inputs.b as number) }),
    });

    client.module({
      id: 'greet.hello',
      inputSchema: Type.Object({ name: Type.String() }),
      outputSchema: Type.Object({ message: Type.String() }),
      tags: ['greet'],
      execute: (inputs) => ({ message: `Hello, ${inputs.name}` }),
    });

    expect(client.listModules({ tags: ['math'] })).toEqual(['math.add']);
  });
});

// ---------------------------------------------------------------------------
// Stream tests
// ---------------------------------------------------------------------------

describe('APCore.stream()', () => {
  it('yields chunks from a streaming module', async () => {
    const client = new APCore();
    registerAdd(client);

    const chunks: Record<string, unknown>[] = [];
    for await (const chunk of client.stream('math.add', { a: 3, b: 4 })) {
      chunks.push(chunk);
    }
    // FunctionModule does not implement stream(), so executor falls back to single-chunk
    expect(chunks).toHaveLength(1);
    expect(chunks[0]).toEqual({ result: 7 });
  });

  it('throws ModuleNotFoundError for unknown module', async () => {
    const client = new APCore();
    const gen = client.stream('nonexistent.module', { a: 1 });
    await expect(gen.next()).rejects.toThrow(ModuleNotFoundError);
  });
});

// ---------------------------------------------------------------------------
// Validate tests
// ---------------------------------------------------------------------------

describe('APCore.validate()', () => {
  it('returns valid PreflightResult for correct inputs', async () => {
    const client = new APCore();
    registerAdd(client);

    const result = await client.validate('math.add', { a: 1, b: 2 });
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
    expect(result.requiresApproval).toBe(false);
    expect(result.checks.length).toBeGreaterThan(0);
  });

  it('returns invalid PreflightResult for bad inputs', async () => {
    const client = new APCore();
    registerAdd(client);

    const result = await client.validate('math.add', { a: 'not_a_number' });
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it('returns invalid PreflightResult for unknown module', async () => {
    const client = new APCore();
    const result = await client.validate('nonexistent.module', {});
    expect(result.valid).toBe(false);
    expect(result.errors.some((e: Record<string, unknown>) => e['code'] === 'MODULE_NOT_FOUND')).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Describe tests
// ---------------------------------------------------------------------------

describe('APCore.describe()', () => {
  it('returns description string for a registered module', () => {
    const client = new APCore();
    registerAdd(client);

    const desc = client.describe('math.add');
    expect(typeof desc).toBe('string');
    expect(desc).toContain('math.add');
  });

  it('throws ModuleNotFoundError for unknown module', () => {
    const client = new APCore();
    expect(() => client.describe('nonexistent.module')).toThrow(ModuleNotFoundError);
  });
});

// ---------------------------------------------------------------------------
// useBefore / useAfter / remove tests
// ---------------------------------------------------------------------------

describe('APCore.useBefore()', () => {
  it('returns self for chaining', () => {
    const client = new APCore();
    const result = client.useBefore(() => null);
    expect(result).toBe(client);
  });

  it('fires before callback during call', async () => {
    const client = new APCore();
    registerAdd(client);

    let called = false;
    client.useBefore((_moduleId, _inputs, _ctx) => {
      called = true;
      return null;
    });

    await client.call('math.add', { a: 1, b: 2 });
    expect(called).toBe(true);
  });

  it('registering a second callback does not trigger the duplicate-middleware warning', async () => {
    const client = new APCore();
    registerAdd(client);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const seen: string[] = [];
    try {
      client.useBefore(() => {
        seen.push('first');
        return null;
      });
      client.useBefore(() => {
        seen.push('second');
        return null;
      });
      client.useAfter(() => null);
      client.useAfter(() => null);

      const duplicateWarnings = warnSpy.mock.calls.filter((c) =>
        String(c[0]).includes('Duplicate middleware registration'),
      );
      expect(duplicateWarnings).toEqual([]);
    } finally {
      warnSpy.mockRestore();
    }

    await client.call('math.add', { a: 1, b: 2 });
    expect(seen).toEqual(['first', 'second']);
  });

  it('registering the SAME callback twice still warns', () => {
    const client = new APCore();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const cb = () => null;
      client.useBefore(cb);
      client.useBefore(cb);
      const duplicateWarnings = warnSpy.mock.calls.filter((c) =>
        String(c[0]).includes('Duplicate middleware registration'),
      );
      expect(duplicateWarnings).toHaveLength(1);
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('a registered callback wrapper can still be removed', () => {
    const client = new APCore();
    client.useBefore(() => null);
    const wrapper = client.executor.middlewares[0];
    expect(client.remove(wrapper)).toBe(true);
    expect(client.executor.middlewares).toHaveLength(0);
  });
});

describe('APCore.useAfter()', () => {
  it('returns self for chaining', () => {
    const client = new APCore();
    const result = client.useAfter(() => null);
    expect(result).toBe(client);
  });

  it('fires after callback during call', async () => {
    const client = new APCore();
    registerAdd(client);

    let capturedOutput: Record<string, unknown> | null = null;
    client.useAfter((_moduleId, _inputs, output, _ctx) => {
      capturedOutput = output;
      return null;
    });

    await client.call('math.add', { a: 5, b: 3 });
    expect(capturedOutput).toEqual({ result: 8 });
  });
});

describe('APCore.remove()', () => {
  it('returns true when middleware is found and removed', () => {
    const client = new APCore();
    const mw = new TrackingMiddleware();
    client.use(mw);
    expect(client.remove(mw)).toBe(true);
  });

  it('returns false when middleware was not registered', () => {
    const client = new APCore();
    const mw = new TrackingMiddleware();
    expect(client.remove(mw)).toBe(false);
  });

  it('middleware no longer fires after removal', async () => {
    const client = new APCore();
    const mw = new TrackingMiddleware();
    client.use(mw);
    registerAdd(client);

    client.remove(mw);
    await client.call('math.add', { a: 1, b: 2 });
    expect(mw.beforeCalled).toBe(false);
    expect(mw.afterCalled).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// callAsync tests
// ---------------------------------------------------------------------------

describe('APCore.callAsync()', () => {
  it('executes a module and returns the result (alias for call)', async () => {
    const client = new APCore();
    registerAdd(client);

    const result = await client.callAsync('math.add', { a: 3, b: 7 });
    expect(result).toEqual({ result: 10 });
  });
});

// ---------------------------------------------------------------------------
// Events on/off tests
// ---------------------------------------------------------------------------

describe('APCore.on() / off()', () => {
  it('throws when events are not enabled', () => {
    const client = new APCore();
    expect(() => client.on('test', () => {})).toThrow('Events are not enabled');
  });

  it('throws off() when events are not enabled', () => {
    const client = new APCore();
    expect(() => client.off({ onEvent: () => {} })).toThrow('Events are not enabled');
  });

  it('subscribes and receives events when enabled', async () => {
    const config = new Config({
      sys_modules: { enabled: true, events: { enabled: true } },
    });
    const client = new APCore({ config });
    registerAdd(client);

    const received: string[] = [];
    client.on('apcore.registry.module_registered', (event) => {
      received.push(event.moduleId!);
    });

    // Register another module to trigger event
    client.module({
      id: 'math.sub',
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      execute: (inputs) => ({ result: (inputs.a as number) - (inputs.b as number) }),
    });

    expect(received).toContain('math.sub');
  });

  it('unsubscribes via off()', () => {
    const config = new Config({
      sys_modules: { enabled: true, events: { enabled: true } },
    });
    const client = new APCore({ config });

    let count = 0;
    const sub = client.on('apcore.registry.module_registered', () => { count++; });
    client.off(sub);

    // Register a module - subscriber should not be called
    client.module({
      id: 'math.add',
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      execute: (inputs) => ({ result: (inputs.a as number) + (inputs.b as number) }),
    });
    expect(count).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// Enable/disable tests
// ---------------------------------------------------------------------------

describe('APCore.enable() / disable()', () => {
  it('throws when sys_modules with events not enabled', async () => {
    const client = new APCore();
    await expect(client.disable('mod.a')).rejects.toThrow('sys_modules');
    await expect(client.enable('mod.a')).rejects.toThrow('sys_modules');
  });

  it('disables and enables a module', async () => {
    const config = new Config({
      sys_modules: { enabled: true, events: { enabled: true } },
    });
    const client = new APCore({ config });
    registerAdd(client);

    const disableResult = await client.disable('math.add', 'testing');
    expect(disableResult['success']).toBe(true);
    expect(disableResult['enabled']).toBe(false);

    const enableResult = await client.enable('math.add', 'testing');
    expect(enableResult['success']).toBe(true);
    expect(enableResult['enabled']).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Export tests
// ---------------------------------------------------------------------------

describe('APCore exports', () => {
  it('is exported from index', async () => {
    const mod = await import('../src/index.js');
    expect(mod.APCore).toBeDefined();
    expect(typeof mod.APCore).toBe('function');
  });
});

// ---------------------------------------------------------------------------
// Step middleware (protocol-spec §5.16 requirement 5)
// ---------------------------------------------------------------------------

describe('APCore.addStepMiddleware()', () => {
  class RecordingStepMiddleware implements StepMiddleware {
    readonly before: string[] = [];
    readonly after: string[] = [];

    beforeStep(stepName: string, _state: PipelineState): void {
      this.before.push(stepName);
    }

    afterStep(stepName: string, _state: PipelineState, _result: unknown): void {
      this.after.push(stepName);
    }
  }

  it('returns self for chaining', () => {
    const client = new APCore();
    expect(client.addStepMiddleware(new RecordingStepMiddleware())).toBe(client);
  });

  it('runs around every pipeline step of a call', async () => {
    const client = new APCore();
    registerAdd(client);
    const mw = new RecordingStepMiddleware();
    client.addStepMiddleware(mw);

    await client.call('math.add', { a: 1, b: 2 });

    const steps = client.executor.currentStrategy.stepNames();
    expect(mw.before).toEqual(steps);
    expect(mw.after).toEqual(steps);
  });

  it('is exposed on the Executor as well', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const mw = new RecordingStepMiddleware();
    expect(executor.addStepMiddleware(mw)).toBe(executor);
    expect(executor.stepMiddlewares).toEqual([mw]);
  });
});
