import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Context, createIdentity } from '../src/context.js';
import { Executor, redactSensitive, REDACTED_VALUE } from '../src/executor.js';
import { FunctionModule } from '../src/decorator.js';
import { MAX_MODULE_ID_LENGTH, Registry } from '../src/registry/registry.js';
import { ACL } from '../src/acl.js';
import { Middleware } from '../src/middleware/base.js';
import {
  ModuleNotFoundError,
  ACLDeniedError,
  CallDepthExceededError,
  CircularCallError,
  CallFrequencyExceededError,
  SchemaValidationError,
  InvalidInputError,
} from '../src/errors.js';

function createSimpleModule(id: string): FunctionModule {
  return new FunctionModule({
    execute: (inputs) => ({ greeting: `Hello, ${inputs['name'] ?? 'world'}!` }),
    moduleId: id,
    inputSchema: Type.Object({ name: Type.Optional(Type.String()) }),
    outputSchema: Type.Object({ greeting: Type.String() }),
    description: 'Greet module',
  });
}

describe('redactSensitive', () => {
  it('redacts fields marked x-sensitive', () => {
    const data = { name: 'Alice', password: 'secret123' };
    const schema = {
      properties: {
        name: { type: 'string' },
        password: { type: 'string', 'x-sensitive': true },
      },
    };
    const result = redactSensitive(data, schema);
    expect(result['name']).toBe('Alice');
    expect(result['password']).toBe(REDACTED_VALUE);
  });

  it('redacts _secret_ prefix keys', () => {
    const data = { _secret_token: 'abc123', name: 'Bob' };
    const schema = { properties: { name: { type: 'string' } } };
    const result = redactSensitive(data, schema);
    expect(result['_secret_token']).toBe(REDACTED_VALUE);
    expect(result['name']).toBe('Bob');
  });

  it('does not modify original data', () => {
    const data = { password: 'secret' };
    const schema = { properties: { password: { type: 'string', 'x-sensitive': true } } };
    redactSensitive(data, schema);
    expect(data['password']).toBe('secret');
  });

  it('handles nested objects', () => {
    const data = { user: { name: 'Alice', token: 'abc' } };
    const schema = {
      properties: {
        user: {
          type: 'object',
          properties: {
            name: { type: 'string' },
            token: { type: 'string', 'x-sensitive': true },
          },
        },
      },
    };
    const result = redactSensitive(data, schema);
    expect((result['user'] as Record<string, unknown>)['name']).toBe('Alice');
    expect((result['user'] as Record<string, unknown>)['token']).toBe(REDACTED_VALUE);
  });

  it('redacts _secret_ keys nested inside array elements (A-D-003)', () => {
    // No x-sensitive schema marker — the prefix walk alone must reach the
    // _secret_ key inside an object nested in an array, matching Python
    // _redact_in_list (utils/redaction.py).
    const data = { items: [{ _secret_api_key: 'k', label: 'one' }] };
    const schema = { properties: {} };
    const result = redactSensitive(data, schema);
    const items = result['items'] as Array<Record<string, unknown>>;
    expect(items[0]['_secret_api_key']).toBe(REDACTED_VALUE);
    expect(items[0]['label']).toBe('one');
  });

  it('recurses into nested arrays for _secret_ redaction (A-D-003)', () => {
    const data = { groups: [[{ _secret_token: 't' }]] };
    const schema = { properties: {} };
    const result = redactSensitive(data, schema);
    const groups = result['groups'] as Array<Array<Record<string, unknown>>>;
    expect(groups[0][0]['_secret_token']).toBe(REDACTED_VALUE);
  });
});

describe('Executor', () => {
  it('executes a simple module', async () => {
    const registry = new Registry();
    const mod = createSimpleModule('greet');
    registry.register('greet', mod);

    const executor = new Executor({ registry });
    const result = await executor.call('greet', { name: 'Alice' });
    expect(result['greeting']).toBe('Hello, Alice!');
  });

  it('throws ModuleNotFoundError for unknown module', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });

    await expect(executor.call('nonexistent')).rejects.toThrow(ModuleNotFoundError);
  });

  it('throws InvalidInputError for invalid module ID in call()', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });

    await expect(executor.call('INVALID-MODULE-ID!!')).rejects.toThrow(InvalidInputError);
  });

  it('validates input against schema', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: (inputs) => ({ result: inputs['count'] }),
      moduleId: 'strict',
      inputSchema: Type.Object({ count: Type.Number() }),
      outputSchema: Type.Object({ result: Type.Number() }),
      description: 'Strict module',
    });
    registry.register('strict', mod);

    const executor = new Executor({ registry });
    await expect(executor.call('strict', { count: 'not-a-number' })).rejects.toThrow(SchemaValidationError);
  });

  it('validates input when inputSchema is raw JSON Schema (not TypeBox)', async () => {
    const registry = new Registry();
    const rawJsonSchema = {
      type: 'object',
      properties: {
        name: { type: 'string' },
        age: { type: 'number' },
      },
      required: ['name', 'age'],
    };
    const mod = {
      inputSchema: rawJsonSchema,
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Module with raw JSON Schema inputSchema',
      execute: () => ({ ok: true }),
    };
    registry.register('raw_schema', mod);

    const executor = new Executor({ registry });
    // Valid input should succeed
    const result = await executor.call('raw_schema', { name: 'Alice', age: 30 });
    expect(result).toEqual({ ok: true });

    // Invalid input should throw SchemaValidationError, not "Unknown type"
    await expect(executor.call('raw_schema', { name: 'Alice', age: 'not-a-number' })).rejects.toThrow(SchemaValidationError);
  });

  it('validates output when outputSchema is raw JSON Schema (not TypeBox)', async () => {
    const registry = new Registry();
    const rawOutputSchema = {
      type: 'object',
      properties: {
        count: { type: 'number' },
      },
      required: ['count'],
    };
    const mod = {
      inputSchema: Type.Object({}),
      outputSchema: rawOutputSchema,
      description: 'Module with raw JSON Schema outputSchema',
      execute: () => ({ count: 'not-a-number' }),
    };
    registry.register('raw_output', mod);

    const executor = new Executor({ registry });
    await expect(executor.call('raw_output', {})).rejects.toThrow(SchemaValidationError);
  });

  it('validates both input and output when both schemas are raw JSON Schema', async () => {
    const registry = new Registry();
    const mod = {
      inputSchema: {
        type: 'object',
        properties: { x: { type: 'number' } },
        required: ['x'],
      },
      outputSchema: {
        type: 'object',
        properties: { y: { type: 'number' } },
        required: ['y'],
      },
      description: 'Both schemas are raw JSON Schema',
      execute: (inputs: Record<string, unknown>) => ({ y: (inputs['x'] as number) * 2 }),
    };
    registry.register('dual_raw', mod);

    const executor = new Executor({ registry });
    // Valid round-trip
    const result = await executor.call('dual_raw', { x: 5 });
    expect(result).toEqual({ y: 10 });

    // Invalid input
    await expect(executor.call('dual_raw', { x: 'bad' })).rejects.toThrow(SchemaValidationError);
  });

  it('caches converted TypeBox schema on the module object after first call', async () => {
    const registry = new Registry();
    const rawSchema = {
      type: 'object',
      properties: { val: { type: 'string' } },
      required: ['val'],
    };
    const mod = {
      inputSchema: rawSchema,
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Cache test module',
      execute: () => ({ ok: true }),
    } as Record<string, unknown>;
    registry.register('cache_test', mod);

    const executor = new Executor({ registry });
    await executor.call('cache_test', { val: 'a' });

    // After first call, inputSchema should have been replaced with a TypeBox schema (has Kind symbol)
    const { Kind: KindSymbol } = await import('@sinclair/typebox');
    expect(KindSymbol in (mod['inputSchema'] as object)).toBe(true);

    // Second call should reuse the same cached schema object
    const cachedRef = mod['inputSchema'];
    await executor.call('cache_test', { val: 'b' });
    expect(mod['inputSchema']).toBe(cachedRef);
  });

  it('enforces ACL deny', async () => {
    const registry = new Registry();
    registry.register('secret', createSimpleModule('secret'));

    const acl = new ACL([
      { callers: ['@external'], targets: ['secret'], effect: 'deny', description: 'deny all' },
    ], 'deny');

    const executor = new Executor({ registry, acl });
    await expect(executor.call('secret')).rejects.toThrow(ACLDeniedError);
  });

  it('enforces ACL allow', async () => {
    const registry = new Registry();
    registry.register('public', createSimpleModule('public'));

    const acl = new ACL([
      { callers: ['*'], targets: ['*'], effect: 'allow', description: 'allow all' },
    ], 'deny');

    const executor = new Executor({ registry, acl });
    const result = await executor.call('public', { name: 'World' });
    expect(result['greeting']).toBe('Hello, World!');
  });

  it('calls middleware before and after', async () => {
    const registry = new Registry();
    registry.register('echo', createSimpleModule('echo'));

    const calls: string[] = [];
    class TrackingMiddleware extends Middleware {
      override before() { calls.push('before'); return null; }
      override after() { calls.push('after'); return null; }
    }

    const executor = new Executor({ registry, middlewares: [new TrackingMiddleware()] });
    await executor.call('echo');
    expect(calls).toEqual(['before', 'after']);
  });

  it('runs middleware onError on execution failure', async () => {
    const registry = new Registry();
    const failMod = new FunctionModule({
      execute: () => { throw new Error('boom'); },
      moduleId: 'fail',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'Failing module',
    });
    registry.register('fail', failMod);

    let errorSeen = false;
    class ErrorTracker extends Middleware {
      override onError() { errorSeen = true; return null; }
    }

    const executor = new Executor({ registry, middlewares: [new ErrorTracker()] });
    await expect(executor.call('fail')).rejects.toThrow('boom');
    expect(errorSeen).toBe(true);
  });

  it('unwraps MiddlewareChainError so callers see the original error class', async () => {
    // Regression: before-middleware throwing InvalidInputError was wrapped
    // inside MiddlewareChainError and then re-thrown as a generic
    // ModuleError('MODULE_EXECUTE_ERROR', ...), losing the original class
    // and error code.
    const registry = new Registry();
    registry.register('echo', createSimpleModule('echo'));

    class FailingBeforeMiddleware extends Middleware {
      override before(): null {
        throw new InvalidInputError('bad input from middleware');
      }
    }

    const executor = new Executor({ registry, middlewares: [new FailingBeforeMiddleware()] });
    await expect(executor.call('echo', { name: 'World' })).rejects.toBeInstanceOf(InvalidInputError);
  });

  it('middleware onError can recover', async () => {
    const registry = new Registry();
    const failMod = new FunctionModule({
      execute: () => { throw new Error('boom'); },
      moduleId: 'fail',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'Failing module',
    });
    registry.register('fail', failMod);

    class RecoveryMiddleware extends Middleware {
      override onError() { return { recovered: true }; }
    }

    const executor = new Executor({ registry, middlewares: [new RecoveryMiddleware()] });
    const result = await executor.call('fail');
    expect(result['recovered']).toBe(true);
  });

  it('validate() returns PreflightResult with all checks passed for valid inputs', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'v',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Validate test',
    });
    registry.register('v', mod);

    const executor = new Executor({ registry });
    const result = await executor.validate('v', { x: 42 });
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
    // Pipeline dry_run: pure steps run, non-pure skip. Check count varies.
    expect(result.checks.length).toBeGreaterThanOrEqual(1);
    expect(result.checks.every((c: { passed: boolean }) => c.passed)).toBe(true);
  });

  it('validate() returns schema failure for invalid inputs', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'v',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Validate test',
    });
    registry.register('v', mod);

    const executor = new Executor({ registry });
    const result = await executor.validate('v', { x: 'not-a-number' });
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it('validate() returns module_id failure for invalid ID format', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const result = await executor.validate('INVALID-ID!!');
    expect(result.valid).toBe(false);
    expect(result.errors.some((e: Record<string, unknown>) => e['code'] === 'INVALID_MODULE_ID')).toBe(true);
    expect(result.checks.find((c: { check: string }) => c.check === 'module_id')?.passed).toBe(false);
  });

  /* ---------------------------------------------------------
   * D-75 (spec v1.49.0) — core-executor.md "Contract: Executor.call"
   * requires empty / over-length / malformed IDs to be rejected
   * BEFORE the PipelineContext is constructed. MAX_MODULE_ID_LENGTH
   * was enforced in the registry only, one step later, so a
   * 300-character well-formed ID built a context and came back as
   * MODULE_NOT_FOUND instead of INVALID_MODULE_ID.
   * --------------------------------------------------------- */

  it('D-75: call() rejects an over-length module ID at the entry guard', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const overLong = `a.${'b'.repeat(MAX_MODULE_ID_LENGTH)}`;
    expect(overLong.length).toBeGreaterThan(MAX_MODULE_ID_LENGTH);

    // Well-formed against MODULE_ID_PATTERN — only the length is wrong.
    await expect(executor.call(overLong, {})).rejects.toThrow(InvalidInputError);
    try {
      await executor.call(overLong, {});
      expect.unreachable('expected InvalidInputError');
    } catch (e) {
      expect((e as InvalidInputError).code).toBe('INVALID_MODULE_ID');
      // Not MODULE_NOT_FOUND from a pipeline that should never have started.
      expect(e).not.toBeInstanceOf(ModuleNotFoundError);
    }
  });

  it('D-75: an ID exactly at MAX_MODULE_ID_LENGTH passes the entry guard', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const atLimit = `a${'b'.repeat(MAX_MODULE_ID_LENGTH - 1)}`;
    expect(atLimit.length).toBe(MAX_MODULE_ID_LENGTH);
    // Reaches module lookup — the length bound is exclusive at the boundary.
    await expect(executor.call(atLimit, {})).rejects.toThrow(ModuleNotFoundError);
  });

  it('D-75: validate() reports the over-length ID as a module_id check failure', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const overLong = `a.${'b'.repeat(MAX_MODULE_ID_LENGTH)}`;
    const result = await executor.validate(overLong);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e: Record<string, unknown>) => e['code'] === 'INVALID_MODULE_ID')).toBe(true);
    expect(result.checks.find((c: { check: string }) => c.check === 'module_id')?.passed).toBe(false);
    // The pipeline never ran, so no module_lookup check was recorded.
    expect(result.checks.find((c: { check: string }) => c.check === 'module_lookup')).toBeUndefined();
  });

  it('validate() returns module_lookup failure for unknown module', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const result = await executor.validate('unknown.module', {});
    expect(result.valid).toBe(false);
    expect(result.errors.some((e: Record<string, unknown>) => e['code'] === 'MODULE_NOT_FOUND')).toBe(true);
    expect(result.checks.find((c: { check: string }) => c.check === 'module_lookup')?.passed).toBe(false);
  });

  // A-D-014: validate() is a non-throwing preflight. A Context already bound
  // to a DIFFERENT executor must surface as a failed `executor_binding` check,
  // not an escaping ContextBindingError (mirrors Python/Rust).
  it('validate() reports executor_binding failure for a foreign-bound context (no throw)', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'v',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Validate test',
    });
    registry.register('v', mod);

    const executorA = new Executor({ registry });
    const executorB = new Executor({ registry });

    // Bind the context to executorA, then preflight against executorB.
    const ctx = Context.create()._withExecutor(executorA);

    const result = await executorB.validate('v', { x: 42 }, ctx);
    expect(result.valid).toBe(false);
    const bindingCheck = result.checks.find(
      (c: { check: string }) => c.check === 'executor_binding',
    );
    expect(bindingCheck).toBeDefined();
    expect(bindingCheck?.passed).toBe(false);
    expect(
      result.errors.some(
        (e: Record<string, unknown>) => e['code'] === 'CONTEXT_BINDING_ERROR',
      ),
    ).toBe(true);
  });

  it('validate() reports ACL denial without executing', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'access.test',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'ACL test',
    });
    registry.register('access.test', mod);

    const acl = new ACL([], 'deny');
    const executor = new Executor({ registry, acl });
    const result = await executor.validate('access.test', { x: 42 });
    expect(result.valid).toBe(false);
    expect(result.errors.some((e: Record<string, unknown>) => e['code'] === 'ACL_DENIED')).toBe(true);
  });

  it('validate() detects requiresApproval annotation', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'approval.test',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Approval test',
      annotations: { requiresApproval: true, readonly: false, destructive: false, idempotent: false, openWorld: true, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const, extra: {} },
    });
    registry.register('approval.test', mod);

    const executor = new Executor({ registry });
    const result = await executor.validate('approval.test', { x: 42 });
    expect(result.valid).toBe(true);
    expect(result.requiresApproval).toBe(true);
  });

  it('validate() detects snake_case requires_approval annotation', async () => {
    const registry = new Registry();
    const mod = {
      execute: () => ({ ok: true }),
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Snake case approval test',
      annotations: { requires_approval: true },
    };
    registry.register('snake.approval', mod as any);

    const executor = new Executor({ registry });
    const result = await executor.validate('snake.approval', {});
    expect(result.requiresApproval).toBe(true);
  });

  it('auto-creates context when none provided', async () => {
    const registry = new Registry();
    registry.register('ctx', createSimpleModule('ctx'));

    const executor = new Executor({ registry });
    const result = await executor.call('ctx');
    expect(result['greeting']).toBeDefined();
  });

  it('uses provided context', async () => {
    const registry = new Registry();
    let capturedCtx: Context | null = null;
    const mod = new FunctionModule({
      execute: (_inputs, ctx) => { capturedCtx = ctx; return { ok: true }; },
      moduleId: 'ctx_test',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Context capture',
    });
    registry.register('ctx_test', mod);

    const executor = new Executor({ registry });
    const ctx = Context.create(createIdentity('user1'));
    await executor.call('ctx_test', {}, ctx);

    expect(capturedCtx).not.toBeNull();
    expect(capturedCtx!.traceId).toBe(ctx.traceId);
    expect(capturedCtx!.identity?.id).toBe('user1');
  });

  it('use/remove middleware chaining', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const mw = new Middleware();

    const result = executor.use(mw);
    expect(result).toBe(executor);
    expect(executor.middlewares).toHaveLength(1);

    executor.remove(mw);
    expect(executor.middlewares).toHaveLength(0);
  });
});

describe('Executor.fromRegistry', () => {
  it('creates an executor with the given registry', () => {
    const registry = new Registry();
    const executor = Executor.fromRegistry(registry);
    expect(executor.registry).toBe(registry);
  });

  it('creates a functional executor that can call registered modules', async () => {
    const registry = new Registry();
    const module = createSimpleModule('greet');
    registry.register('greet', module);

    const executor = Executor.fromRegistry(registry);
    const result = await executor.call('greet', { name: 'World' });
    expect(result).toEqual({ greeting: 'Hello, World!' });
  });

  it('accepts optional middleware list', () => {
    const registry = new Registry();
    const mw = new Middleware();
    const executor = Executor.fromRegistry(registry, [mw]);
    expect(executor.middlewares).toHaveLength(1);
    expect(executor.middlewares[0]).toBe(mw);
  });

  it('accepts optional ACL', async () => {
    const registry = new Registry();
    const module = createSimpleModule('secure.mod');
    registry.register('secure.mod', module);

    const acl = new ACL([], 'deny');
    const executor = Executor.fromRegistry(registry, null, acl);

    await expect(executor.call('secure.mod', {})).rejects.toBeInstanceOf(ACLDeniedError);
  });

  it('returns an Executor instance (not a subclass)', () => {
    const registry = new Registry();
    const executor = Executor.fromRegistry(registry);
    expect(executor).toBeInstanceOf(Executor);
  });
});

describe('setAcl / setApprovalHandler on strategies without those steps', () => {
  it('warns via console.warn when setAcl is called on a strategy without BuiltinACLCheck', () => {
    const registry = new Registry();
    const executor = new Executor({ registry, strategy: 'internal' });
    const acl = new ACL([], 'deny');
    const warns: string[] = [];
    const origWarn = console.warn;
    console.warn = (...args: unknown[]) => warns.push(String(args[0]));
    try {
      executor.setAcl(acl);
      expect(warns.some(w => w.includes('setAcl') && w.includes('BuiltinACLCheck'))).toBe(true);
    } finally {
      console.warn = origWarn;
    }
  });

  it('warns via console.warn when setApprovalHandler is called on a strategy without BuiltinApprovalGate', () => {
    const registry = new Registry();
    const executor = new Executor({ registry, strategy: 'internal' });
    const warns: string[] = [];
    const origWarn = console.warn;
    console.warn = (...args: unknown[]) => warns.push(String(args[0]));
    try {
      executor.setApprovalHandler({ requestApproval: async () => ({ status: 'approved' as const, approvalId: 'x', approvedBy: null, reason: null, metadata: null }), checkApproval: async () => ({ status: 'approved' as const, approvalId: 'x', approvedBy: null, reason: null, metadata: null }) });
      expect(warns.some(w => w.includes('setApprovalHandler') && w.includes('BuiltinApprovalGate'))).toBe(true);
    } finally {
      console.warn = origWarn;
    }
  });
});

describe('Executor.callWithTrace — PipelineStepError unwrap', () => {
  it('surfaces original typed error (not PipelineStepError) for unknown module', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    await expect(
      executor.callWithTrace('nonexistent', {}),
    ).rejects.toThrow(ModuleNotFoundError);
  });
});

// A-D-017: middleware can request a pipeline retry with new inputs via RetrySignal.
describe('Executor.call — middleware RetrySignal retry semantics', () => {
  it('re-runs the pipeline when middleware.onError returns RetrySignal', async () => {
    const { RetrySignal } = await import('../src/middleware/base.js');

    let attempts = 0;
    const mod = new FunctionModule({
      execute: (inputs) => {
        attempts += 1;
        if (inputs['retried'] === true) {
          return { ok: true, attempt: attempts };
        }
        throw new Error('first attempt always fails');
      },
      moduleId: 'test.retry',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'fails once, succeeds on retry',
    });

    class RetryMiddleware extends Middleware {
      override onError(
        _moduleId: string,
        inputs: Record<string, unknown>,
        _err: Error,
        _ctx: Context,
      ): Record<string, unknown> | InstanceType<typeof RetrySignal> | null {
        if (inputs['retried'] === true) return null;
        return new RetrySignal({ ...inputs, retried: true });
      }
    }

    const registry = new Registry();
    registry.register('test.retry', mod);
    const executor = new Executor({ registry });
    executor.use(new RetryMiddleware());

    const result = await executor.call('test.retry', {});
    expect(attempts).toBe(2);
    expect(result['ok']).toBe(true);
  });
});
