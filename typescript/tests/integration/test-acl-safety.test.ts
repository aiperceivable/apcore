import { describe, it, expect } from 'vitest';
import { v4 as uuidv4 } from 'uuid';
import { Type } from '@sinclair/typebox';
import { Executor } from '../../src/executor.js';
import { FunctionModule } from '../../src/decorator.js';
import { Registry } from '../../src/registry/registry.js';
import { ACL } from '../../src/acl.js';
import { Context, createIdentity } from '../../src/context.js';
import { Config } from '../../src/config.js';
import {
  ACLDeniedError,
  CallDepthExceededError,
  CallFrequencyExceededError,
  CircularCallError,
} from '../../src/errors.js';

function makeModule(id: string, fn?: (inputs: Record<string, unknown>, ctx: Context) => Record<string, unknown> | Promise<Record<string, unknown>>): FunctionModule {
  return new FunctionModule({
    execute: fn ?? (() => ({ value: 'ok' })),
    moduleId: id,
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({ value: Type.String() }),
    description: `Module ${id}`,
  });
}

describe('ACL Integration', () => {
  it('wildcard pattern allows matching modules', async () => {
    const registry = new Registry();
    registry.register('math.add', new FunctionModule({
      execute: (inputs) => ({ result: (inputs['a'] as number) + (inputs['b'] as number) }),
      moduleId: 'math.add',
      inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
      outputSchema: Type.Object({ result: Type.Number() }),
      description: 'Add two numbers',
    }));
    registry.register('io.read', makeModule('io.read'));

    const acl = new ACL([
      { callers: ['*'], targets: ['math.*'], effect: 'allow', description: 'Allow math' },
    ], 'deny');

    const executor = new Executor({ registry, acl });

    const result = await executor.call('math.add', { a: 2, b: 3 });
    expect(result['result']).toBe(5);

    await expect(executor.call('io.read', {})).rejects.toThrow(ACLDeniedError);
  });

  it('external caller denied by default', async () => {
    const registry = new Registry();
    registry.register('test.mod', makeModule('test.mod'));

    const acl = new ACL([], 'deny');
    const executor = new Executor({ registry, acl });

    // No context = null caller = @external
    await expect(executor.call('test.mod', {})).rejects.toThrow(ACLDeniedError);
  });

  it('system identity type allowed via conditions', async () => {
    const registry = new Registry();
    registry.register('test.mod', makeModule('test.mod'));

    const acl = new ACL([
      {
        callers: ['*'],
        targets: ['*'],
        effect: 'allow',
        description: 'Allow system identity',
        conditions: { identity_types: ['system'] },
      },
    ], 'deny');

    const executor = new Executor({ registry, acl });

    // System identity - allowed
    const sysCtx = Context.create(createIdentity('sys', 'system'));
    const result = await executor.call('test.mod', {}, sysCtx);
    expect(result['value']).toBe('ok');

    // User identity - denied
    const userCtx = Context.create(createIdentity('user1', 'user'));
    await expect(executor.call('test.mod', {}, userCtx)).rejects.toThrow(ACLDeniedError);
  });

  it('role-based conditions', async () => {
    const registry = new Registry();
    registry.register('test.mod', makeModule('test.mod'));

    const acl = new ACL([
      {
        callers: ['*'],
        targets: ['*'],
        effect: 'allow',
        description: 'Allow admin role',
        conditions: { roles: ['admin'] },
      },
    ], 'deny');

    const executor = new Executor({ registry, acl });

    // Admin role - allowed
    const adminCtx = Context.create(createIdentity('admin1', 'user', ['admin']));
    const result = await executor.call('test.mod', {}, adminCtx);
    expect(result['value']).toBe('ok');

    // Viewer role - denied
    const viewerCtx = Context.create(createIdentity('viewer1', 'user', ['viewer']));
    await expect(executor.call('test.mod', {}, viewerCtx)).rejects.toThrow(ACLDeniedError);
  });

  it('max call depth condition in ACL', async () => {
    const registry = new Registry();
    registry.register('test.mod', makeModule('test.mod'));

    const acl = new ACL([
      {
        callers: ['*'],
        targets: ['*'],
        effect: 'allow',
        description: 'Allow up to depth 2',
        conditions: { max_call_depth: 2 },
      },
    ], 'deny');

    const executor = new Executor({ registry, acl });

    // Shallow call - allowed
    const ctx = Context.create();
    const result = await executor.call('test.mod', {}, ctx);
    expect(result['value']).toBe('ok');

    // Deep call chain (depth > 2) - denied by ACL condition
    const deepCtx = new Context(
      uuidv4(),
      'caller1',
      ['mod1', 'mod2', 'mod3'],
      executor,
      null,
      null,
      {},
    );
    await expect(executor.call('test.mod', {}, deepCtx)).rejects.toThrow(ACLDeniedError);
  });

  it('multiple rules: first match wins', async () => {
    const registry = new Registry();
    registry.register('secret.data', makeModule('secret.data'));
    registry.register('public.data', makeModule('public.data'));

    const acl = new ACL([
      { callers: ['*'], targets: ['secret.*'], effect: 'deny', description: 'Deny secret' },
      { callers: ['*'], targets: ['*'], effect: 'allow', description: 'Allow rest' },
    ], 'deny');

    const executor = new Executor({ registry, acl });

    await expect(executor.call('secret.data', {})).rejects.toThrow(ACLDeniedError);

    const result = await executor.call('public.data', {});
    expect(result['value']).toBe('ok');
  });

  it('addRule inserts at front and takes precedence', async () => {
    const registry = new Registry();
    registry.register('test.mod', makeModule('test.mod'));

    const acl = new ACL([
      { callers: ['*'], targets: ['*'], effect: 'deny', description: 'Deny all' },
    ], 'deny');

    const executor = new Executor({ registry, acl });

    // Initially denied
    await expect(executor.call('test.mod', {})).rejects.toThrow(ACLDeniedError);

    // Add allow rule at front
    acl.addRule({
      callers: ['*'],
      targets: ['test.mod'],
      effect: 'allow',
      description: 'Allow test.mod',
    });

    // Now allowed
    const result = await executor.call('test.mod', {});
    expect(result['value']).toBe('ok');
  });
});

describe('Safety Checks', () => {
  it('call depth exceeded', async () => {
    const registry = new Registry();
    registry.register('mod.recursive', new FunctionModule({
      execute: async (inputs: Record<string, unknown>, context: Context) => {
        const depth = inputs['depth'] as number;
        if (depth > 0) {
          const exec = context.executor as Executor;
          return await exec.call('mod.recursive', { depth: depth - 1 }, context);
        }
        return { depth: 0 };
      },
      moduleId: 'mod.recursive',
      inputSchema: Type.Object({ depth: Type.Number() }),
      outputSchema: Type.Object({ depth: Type.Number() }),
      description: 'Recursive module',
    }));

    const config = new Config({ executor: { max_call_depth: 3 } });
    const executor = new Executor({ registry, config });

    await expect(executor.call('mod.recursive', { depth: 10 })).rejects.toThrow(CallDepthExceededError);
  });

  it('circular call detection', async () => {
    const registry = new Registry();

    registry.register('mod.a', new FunctionModule({
      execute: async (_inputs: Record<string, unknown>, context: Context) => {
        const exec = context.executor as Executor;
        return await exec.call('mod.b', {}, context);
      },
      moduleId: 'mod.a',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ value: Type.String() }),
      description: 'Module A',
    }));

    registry.register('mod.b', new FunctionModule({
      execute: async (_inputs: Record<string, unknown>, context: Context) => {
        const exec = context.executor as Executor;
        return await exec.call('mod.a', {}, context);
      },
      moduleId: 'mod.b',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ value: Type.String() }),
      description: 'Module B',
    }));

    const executor = new Executor({ registry });

    await expect(executor.call('mod.a', {})).rejects.toThrow(CircularCallError);
  });

  it('call frequency exceeded', async () => {
    const registry = new Registry();
    registry.register('mod.freq', new FunctionModule({
      execute: async (inputs: Record<string, unknown>, context: Context) => {
        const count = inputs['count'] as number;
        if (count > 0) {
          const exec = context.executor as Executor;
          return await exec.call('mod.freq', { count: count - 1 }, context);
        }
        return { count: 0 };
      },
      moduleId: 'mod.freq',
      inputSchema: Type.Object({ count: Type.Number() }),
      outputSchema: Type.Object({ count: Type.Number() }),
      description: 'Frequent module',
    }));

    const config = new Config({ executor: { max_module_repeat: 2 } });
    const executor = new Executor({ registry, config });

    await expect(executor.call('mod.freq', { count: 5 })).rejects.toThrow(CallFrequencyExceededError);
  });
});
