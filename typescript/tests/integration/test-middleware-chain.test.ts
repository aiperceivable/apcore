import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Context } from '../../src/context.js';
import { Executor } from '../../src/executor.js';
import { FunctionModule } from '../../src/decorator.js';
import { Registry } from '../../src/registry/registry.js';
import { Middleware } from '../../src/middleware/base.js';
import { BeforeMiddleware, AfterMiddleware } from '../../src/middleware/adapters.js';

function createEchoModule(): FunctionModule {
  return new FunctionModule({
    execute: (inputs) => ({ value: inputs['x'] ?? 'default' }),
    moduleId: 'echo',
    inputSchema: Type.Object({ x: Type.Optional(Type.String()) }),
    outputSchema: Type.Object({ value: Type.String() }),
    description: 'Echo module',
  });
}

describe('Middleware Chain', () => {
  it('before middlewares run in order, after in reverse', async () => {
    const registry = new Registry();
    registry.register('echo', createEchoModule());

    const order: string[] = [];

    class MW1 extends Middleware {
      override before() { order.push('before-1'); return null; }
      override after() { order.push('after-1'); return null; }
    }
    class MW2 extends Middleware {
      override before() { order.push('before-2'); return null; }
      override after() { order.push('after-2'); return null; }
    }

    const executor = new Executor({ registry, middlewares: [new MW1(), new MW2()] });
    await executor.call('echo', { x: 'test' });

    expect(order).toEqual(['before-1', 'before-2', 'after-2', 'after-1']);
  });

  it('before middleware can transform inputs', async () => {
    const registry = new Registry();
    registry.register('echo', createEchoModule());

    class InputTransform extends Middleware {
      override before(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _context: Context,
      ): Record<string, unknown> {
        return { x: 'transformed' };
      }
    }

    const executor = new Executor({ registry, middlewares: [new InputTransform()] });
    const result = await executor.call('echo', { x: 'original' });
    expect(result['value']).toBe('transformed');
  });

  it('after middleware can transform output', async () => {
    const registry = new Registry();
    registry.register('echo', createEchoModule());

    class Transform extends Middleware {
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
      ): Record<string, unknown> {
        return { value: `transformed-${output['value']}` };
      }
    }

    const executor = new Executor({ registry, middlewares: [new Transform()] });
    const result = await executor.call('echo', { x: 'hello' });
    expect(result['value']).toBe('transformed-hello');
  });

  it('adapter middlewares work', async () => {
    const registry = new Registry();
    registry.register('echo', createEchoModule());

    const calls: string[] = [];
    const beforeMw = new BeforeMiddleware((_moduleId, _inputs, _ctx) => {
      calls.push('before-adapter');
      return null;
    });
    const afterMw = new AfterMiddleware((_moduleId, _inputs, _output, _ctx) => {
      calls.push('after-adapter');
      return null;
    });

    const executor = new Executor({ registry, middlewares: [beforeMw, afterMw] });
    await executor.call('echo', { x: 'test' });

    expect(calls).toEqual(['before-adapter', 'after-adapter']);
  });

  it('use/remove middleware at runtime', async () => {
    const registry = new Registry();
    registry.register('echo', createEchoModule());

    const calls: string[] = [];
    class Tracker extends Middleware {
      override before() { calls.push('tracked'); return null; }
    }

    const executor = new Executor({ registry });
    const mw = new Tracker();
    executor.use(mw);

    await executor.call('echo', { x: 'a' });
    expect(calls).toEqual(['tracked']);

    executor.remove(mw);
    await executor.call('echo', { x: 'b' });
    expect(calls).toEqual(['tracked']); // Still just 1 call
  });
});
