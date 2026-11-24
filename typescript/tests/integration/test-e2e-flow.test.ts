import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Context, createIdentity } from '../../src/context.js';
import { Executor } from '../../src/executor.js';
import { FunctionModule } from '../../src/decorator.js';
import { Registry } from '../../src/registry/registry.js';
import { ACL } from '../../src/acl.js';
import { Middleware } from '../../src/middleware/base.js';
import { MetricsCollector, MetricsMiddleware } from '../../src/observability/metrics.js';
import { InMemoryExporter, TracingMiddleware } from '../../src/observability/tracing.js';

describe('E2E Flow', () => {
  it('full pipeline: register, execute, collect metrics', async () => {
    const registry = new Registry();
    const greet = new FunctionModule({
      execute: (inputs) => ({ greeting: `Hello, ${inputs['name']}!` }),
      moduleId: 'example.greet',
      inputSchema: Type.Object({ name: Type.String() }),
      outputSchema: Type.Object({ greeting: Type.String() }),
      description: 'Greet a user',
    });
    registry.register('example.greet', greet);

    const metrics = new MetricsCollector();
    const exporter = new InMemoryExporter();
    const executor = new Executor({
      registry,
      middlewares: [
        new MetricsMiddleware(metrics),
        new TracingMiddleware(exporter),
      ],
    });

    const result = await executor.call('example.greet', { name: 'World' });
    expect(result['greeting']).toBe('Hello, World!');

    const snap = metrics.snapshot();
    const counters = snap['counters'] as Record<string, number>;
    expect(counters['apcore_module_calls_total|module_id=example.greet,status=success']).toBe(1);

    const spans = exporter.getSpans();
    expect(spans).toHaveLength(1);
    expect(spans[0].status).toBe('ok');
  });

  it('ACL deny blocks execution', async () => {
    const registry = new Registry();
    registry.register('secret', new FunctionModule({
      execute: () => ({ data: 'classified' }),
      moduleId: 'secret',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ data: Type.String() }),
      description: 'Secret module',
    }));

    const acl = new ACL([
      { callers: ['@external'], targets: ['secret'], effect: 'deny', description: 'block externals' },
    ], 'deny');

    const executor = new Executor({ registry, acl });
    await expect(executor.call('secret')).rejects.toThrow();
  });

  it('multiple module calls with context chaining', async () => {
    const registry = new Registry();
    const modA = new FunctionModule({
      execute: (inputs) => ({ value: (inputs['x'] as number) * 2 }),
      moduleId: 'math.double',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ value: Type.Number() }),
      description: 'Double a number',
    });
    const modB = new FunctionModule({
      execute: (inputs) => ({ value: (inputs['x'] as number) + 10 }),
      moduleId: 'math.add_ten',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ value: Type.Number() }),
      description: 'Add ten',
    });
    registry.register('math.double', modA);
    registry.register('math.add_ten', modB);

    const executor = new Executor({ registry });
    const ctx = Context.create(createIdentity('test_user'));

    const r1 = await executor.call('math.double', { x: 5 }, ctx);
    expect(r1['value']).toBe(10);

    const r2 = await executor.call('math.add_ten', { x: r1['value'] as number }, ctx);
    expect(r2['value']).toBe(20);
  });

  it('middleware intercepts and transforms', async () => {
    const registry = new Registry();
    registry.register('echo', new FunctionModule({
      execute: (inputs) => ({ echo: inputs['msg'] }),
      moduleId: 'echo',
      inputSchema: Type.Object({ msg: Type.String() }),
      outputSchema: Type.Object({ echo: Type.String() }),
      description: 'Echo back',
    }));

    class UppercaseMiddleware extends Middleware {
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
      ): Record<string, unknown> {
        return { echo: (output['echo'] as string).toUpperCase() };
      }
    }

    const executor = new Executor({ registry, middlewares: [new UppercaseMiddleware()] });
    const result = await executor.call('echo', { msg: 'hello' });
    expect(result['echo']).toBe('HELLO');
  });
});
