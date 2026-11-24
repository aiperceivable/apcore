import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import { BindingLoader } from '../../src/bindings.js';
import { Registry } from '../../src/registry/registry.js';
import { Executor } from '../../src/executor.js';
import { FunctionModule } from '../../src/decorator.js';
import { Context, createIdentity } from '../../src/context.js';
import { ContextBindingError } from '../../src/errors.js';
import { InMemoryExporter, TracingMiddleware } from '../../src/observability/tracing.js';
import { MetricsCollector, MetricsMiddleware } from '../../src/observability/metrics.js';

let tmpDir: string;
let registry: Registry;
let loader: BindingLoader;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'binding-executor-test-'));
  registry = new Registry();
  loader = new BindingLoader();
});

afterEach(() => {
  rmSync(tmpDir, { recursive: true, force: true });
});

function writeTempModule(filename: string, content: string): string {
  const filePath = join(tmpDir, filename);
  writeFileSync(filePath, `${content}\nexport const inputSchema = {type:'object'};\nexport const outputSchema = {type:'object'};\n`, 'utf-8');
  return filePath;
}

function writeTempYaml(filename: string, content: string): string {
  const filePath = join(tmpDir, filename);
  writeFileSync(filePath, content, 'utf-8');
  return filePath;
}

describe('Binding + Registry + Executor', () => {
  it('loads binding and executes through executor', async () => {
    const modPath = writeTempModule(
      'greet.mjs',
      'export function greet(inputs) { return { greeting: "Hello, " + inputs.name }; }\n',
    );
    const yamlPath = writeTempYaml(
      'greet.binding.yaml',
      `bindings:\n  - module_id: "test.greet"\n    target: "${modPath}:greet"\n    description: "Greet module"\n`,
    );

    await loader.loadBindings(yamlPath, registry);
    expect(registry.has('test.greet')).toBe(true);

    const executor = new Executor({ registry });
    const result = await executor.call('test.greet', { name: 'World' });
    expect(result['greeting']).toBe('Hello, World');
  });

  it('binding with inline schemas validates inputs', async () => {
    const modPath = writeTempModule(
      'validated.mjs',
      'export function handle(inputs) { return { result: inputs.name }; }\n',
    );
    const yamlPath = writeTempYaml(
      'validated.binding.yaml',
      `bindings:\n  - module_id: "test.validated"\n    target: "${modPath}:handle"\n    input_schema:\n      type: object\n      properties:\n        name:\n          type: string\n      required:\n        - name\n    output_schema:\n      type: object\n`,
    );

    await loader.loadBindings(yamlPath, registry);
    const executor = new Executor({ registry });

    // Valid input succeeds
    const result = await executor.call('test.validated', { name: 'Alice' });
    expect(result['result']).toBe('Alice');

    // Invalid input fails (number instead of string)
    await expect(executor.call('test.validated', { name: 123 })).rejects.toThrow();
  });

  it('loads multiple bindings from directory and executes all', async () => {
    const modPath = writeTempModule(
      'multi.mjs',
      'export function alpha() { return { id: "alpha" }; }\nexport function beta() { return { id: "beta" }; }\n',
    );

    writeTempYaml(
      'alpha.binding.yaml',
      `bindings:\n  - module_id: "dir.alpha"\n    target: "${modPath}:alpha"\n`,
    );
    writeTempYaml(
      'beta.binding.yaml',
      `bindings:\n  - module_id: "dir.beta"\n    target: "${modPath}:beta"\n`,
    );

    await loader.loadBindingDir(tmpDir, registry);
    expect(registry.has('dir.alpha')).toBe(true);
    expect(registry.has('dir.beta')).toBe(true);

    const executor = new Executor({ registry });

    const r1 = await executor.call('dir.alpha', {});
    expect(r1['id']).toBe('alpha');

    const r2 = await executor.call('dir.beta', {});
    expect(r2['id']).toBe('beta');
  });

  it('binding module with tracing and metrics', async () => {
    const modPath = writeTempModule(
      'traced.mjs',
      'export function handler(inputs) { return { ok: true }; }\n',
    );
    const yamlPath = writeTempYaml(
      'traced.binding.yaml',
      `bindings:\n  - module_id: "test.traced"\n    target: "${modPath}:handler"\n`,
    );

    await loader.loadBindings(yamlPath, registry);

    const exporter = new InMemoryExporter();
    const metrics = new MetricsCollector();
    const executor = new Executor({
      registry,
      middlewares: [new TracingMiddleware(exporter), new MetricsMiddleware(metrics)],
    });

    const result = await executor.call('test.traced', {});
    expect(result['ok']).toBe(true);

    // Verify span exported
    const spans = exporter.getSpans();
    expect(spans).toHaveLength(1);
    expect(spans[0].status).toBe('ok');
    expect(spans[0].attributes['module_id']).toBe('test.traced');

    // Verify metrics recorded
    const snap = metrics.snapshot();
    const counters = snap['counters'] as Record<string, number>;
    expect(counters['apcore_module_calls_total|module_id=test.traced,status=success']).toBe(1);
  });

  it('binding module receives context with identity', async () => {
    const modPath = writeTempModule(
      'ctx_aware.mjs',
      'export function handle(inputs, context) { return { callerId: context?.callerId ?? "none", hasIdentity: context?.identity != null }; }\n',
    );
    const yamlPath = writeTempYaml(
      'ctx.binding.yaml',
      `bindings:\n  - module_id: "test.ctx"\n    target: "${modPath}:handle"\n`,
    );

    await loader.loadBindings(yamlPath, registry);

    const executor = new Executor({ registry });
    const identity = createIdentity('user123', 'user', ['admin']);
    // Issue #66: executor is NOT a Context.create() parameter. The Executor
    // auto-binds itself on the first call().
    const ctx = Context.create(identity);
    expect(ctx.executor).toBeNull();

    const result = await executor.call('test.ctx', {}, ctx);
    expect(result['hasIdentity']).toBe(true);
    // After the call returns, the Context's *original* instance still has
    // executor === null (Context is immutable); the bound copy lived inside
    // the pipeline. This is the expected v0.22.0 behavior.
    expect(ctx.executor).toBeNull();
  });

  // -------------------------------------------------------------------------
  // Executor binding contract (apcore Issue #66 / v0.22.0)
  // -------------------------------------------------------------------------

  it('auto-binds executor on first call() (Issue #66)', async () => {
    const registry = new Registry();
    let observed: unknown = null;
    registry.register('test.observe', new FunctionModule({
      execute: (_inputs, context) => {
        observed = context.executor;
        return { ok: true };
      },
      moduleId: 'test.observe',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Captures executor from context',
    }));

    const executor = new Executor({ registry });
    const ctx = Context.create();
    expect(ctx.executor).toBeNull();

    await executor.call('test.observe', {}, ctx);
    // Module saw the auto-bound executor on its child context.
    expect(observed).toBe(executor);
  });

  it('rebind on the same executor is idempotent (Issue #66)', async () => {
    const registry = new Registry();
    registry.register('test.noop', new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'test.noop',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'noop',
    }));

    const executor = new Executor({ registry });
    const ctx = Context.create();

    // Three calls on the same Context + Executor MUST NOT raise.
    await executor.call('test.noop', {}, ctx);
    await executor.call('test.noop', {}, ctx);
    await executor.call('test.noop', {}, ctx);
  });

  it('cross-executor rebind raises ContextBindingError (Issue #66)', () => {
    const executorA = new Executor({ registry: new Registry() });
    const executorB = new Executor({ registry: new Registry() });

    // Manually bind to A using the internal helper, then try B.
    const ctx = Context.create()._withExecutor(executorA);
    expect(() => ctx._withExecutor(executorB)).toThrow(ContextBindingError);
  });

  it('mixed bindings and FunctionModule in same registry', async () => {
    // Register FunctionModule directly
    registry.register('direct.add', new FunctionModule({
      execute: (inputs) => ({ sum: (inputs['a'] as number) + (inputs['b'] as number) }),
      moduleId: 'direct.add',
      inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
      outputSchema: Type.Object({ sum: Type.Number() }),
      description: 'Direct add',
    }));

    // Load binding
    const modPath = writeTempModule(
      'multiply.mjs',
      'export function multiply(inputs) { return { product: inputs.a * inputs.b }; }\n',
    );
    const yamlPath = writeTempYaml(
      'multiply.binding.yaml',
      `bindings:\n  - module_id: "binding.multiply"\n    target: "${modPath}:multiply"\n`,
    );
    await loader.loadBindings(yamlPath, registry);

    expect(registry.has('direct.add')).toBe(true);
    expect(registry.has('binding.multiply')).toBe(true);

    const executor = new Executor({ registry });

    const addResult = await executor.call('direct.add', { a: 5, b: 3 });
    expect(addResult['sum']).toBe(8);

    const mulResult = await executor.call('binding.multiply', { a: 5, b: 3 });
    expect(mulResult['product']).toBe(15);
  });
});
