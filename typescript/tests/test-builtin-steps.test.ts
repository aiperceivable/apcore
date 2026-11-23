/**
 * Tests for built-in pipeline steps.
 */

import { describe, it, expect, vi } from 'vitest';
import { Context } from '../src/context.js';
import { CancelToken } from '../src/cancel.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import { BeforeMiddleware, AfterMiddleware } from '../src/middleware/index.js';
import { Registry } from '../src/registry/registry.js';
import { Config } from '../src/config.js';
import { CTX_GLOBAL_DEADLINE } from '../src/executor.js';
import type { PipelineContext, StepResult } from '../src/pipeline.js';
import type { ApprovalHandler, ApprovalResult } from '../src/approval.js';
import { createApprovalResult } from '../src/approval.js';
import type { ACL } from '../src/acl.js';
import {
  BuiltinContextCreation,
  BuiltinCallChainGuard,
  BuiltinModuleLookup,
  BuiltinACLCheck,
  BuiltinApprovalGate,
  BuiltinInputValidation,
  BuiltinMiddlewareBefore,
  BuiltinExecute,
  BuiltinOutputValidation,
  BuiltinMiddlewareAfter,
  BuiltinReturnResult,
  buildStandardStrategy,
} from '../src/builtin-steps.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makePipelineContext(overrides: Partial<PipelineContext> = {}): PipelineContext {
  const ctx = Context.create().child('test.module');
  return {
    moduleId: 'test.module',
    inputs: {},
    context: ctx,
    module: null,
    validatedInputs: null,
    output: null,
    validatedOutput: null,
    stream: false,
    outputStream: null,
    strategy: null,
    trace: null,
    ...overrides,
  };
}

function makeModule(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: 'test.module',
    execute: async (inputs: Record<string, unknown>) => ({ result: 'ok', ...inputs }),
    ...overrides,
  };
}

function makeRegistry(modules: Record<string, Record<string, unknown>> = {}): Registry {
  const reg = new Registry();
  for (const [id, mod] of Object.entries(modules)) {
    reg.register(id, mod);
  }
  return reg;
}

function makeMockACL(allowed: boolean): ACL {
  return {
    check: vi.fn().mockReturnValue(allowed),
    asyncCheck: vi.fn().mockResolvedValue(allowed),
  } as unknown as ACL;
}

function makeMockApprovalHandler(result: ApprovalResult): ApprovalHandler {
  return {
    requestApproval: vi.fn().mockResolvedValue(result),
    checkApproval: vi.fn().mockResolvedValue(result),
  };
}

// ---------------------------------------------------------------------------
// 1. BuiltinContextCreation
// ---------------------------------------------------------------------------

describe('BuiltinContextCreation', () => {
  it('has correct metadata', () => {
    const step = new BuiltinContextCreation(null);
    expect(step.name).toBe('context_creation');
    expect(step.removable).toBe(false);
    expect(step.replaceable).toBe(false);
  });

  // Spec v1.50.0 D-99/D-100/D-101: the budget is the first-class
  // `Context.globalDeadline` field on the epoch-SECONDS clock, stamped onto
  // the context DERIVED for this call. These cases previously asserted the
  // private `context.data[CTX_GLOBAL_DEADLINE]` slot in milliseconds.
  it('sets global deadline on root call', async () => {
    const step = new BuiltinContextCreation(null);
    const pctx = makePipelineContext();
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.context.globalDeadline).toBeTypeOf('number');
    // Seconds, not milliseconds: a ms value would be ~1e12.
    expect(pctx.context.globalDeadline as number).toBeLessThan(1e11);
    expect(CTX_GLOBAL_DEADLINE in pctx.context.data).toBe(false);
  });

  it('does not overwrite a deadline already carried by the context', async () => {
    const step = new BuiltinContextCreation(null);
    const supplied = Date.now() / 1000 + 999;
    const pctx = makePipelineContext({
      context: Context.create(null, null, null, undefined, null, supplied),
    });
    await step.execute(pctx);
    expect(pctx.context.globalDeadline).toBe(supplied);
  });

  it('does not write the deadline onto the caller-supplied context (D-101)', async () => {
    const step = new BuiltinContextCreation(null);
    const caller = Context.create();
    const pctx = makePipelineContext({ context: caller });
    await step.execute(pctx);
    expect(pctx.context.globalDeadline).toBeTypeOf('number');
    expect(caller.globalDeadline).toBeNull();
  });

  it('creates context when context is null', async () => {
    const step = new BuiltinContextCreation(null);
    const pctx = makePipelineContext();
    // Force null context to simulate missing context
    (pctx as unknown as Record<string, unknown>).context = null as unknown as Context;
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.context).toBeInstanceOf(Context);
  });
});

// ---------------------------------------------------------------------------
// 2. BuiltinCallChainGuard
// ---------------------------------------------------------------------------

describe('BuiltinCallChainGuard', () => {
  it('has correct metadata', () => {
    const step = new BuiltinCallChainGuard(null);
    expect(step.name).toBe('call_chain_guard');
    expect(step.removable).toBe(true);
    expect(step.replaceable).toBe(true);
  });

  it('continues when call depth is within limits', async () => {
    const step = new BuiltinCallChainGuard(null);
    const pctx = makePipelineContext();
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
  });

  it('throws CallDepthExceededError when max call depth exceeded', async () => {
    const step = new BuiltinCallChainGuard(null);
    // Build a context with a call chain longer than 32
    const chain: string[] = [];
    for (let i = 0; i < 33; i++) {
      chain.push(`mod${i}`);
    }
    const ctx = new Context('trace-id', null, chain, null);
    const pctx = makePipelineContext({ context: ctx, moduleId: 'mod32' });
    await expect(step.execute(pctx)).rejects.toThrow();
  });

  it('throws ExecutionCancelledError when cancel token is cancelled', async () => {
    const step = new BuiltinCallChainGuard(null);
    const cancelToken = new CancelToken();
    cancelToken.cancel();
    const ctx = new Context('trace-id', null, ['test.module'], null, null, null, {}, cancelToken);
    const pctx = makePipelineContext({ context: ctx });
    await expect(step.execute(pctx)).rejects.toThrow();
  });
});

// ---------------------------------------------------------------------------
// 3. BuiltinModuleLookup
// ---------------------------------------------------------------------------

describe('BuiltinModuleLookup', () => {
  it('has correct metadata', () => {
    const reg = makeRegistry();
    const step = new BuiltinModuleLookup(reg);
    expect(step.name).toBe('module_lookup');
    expect(step.removable).toBe(false);
    expect(step.replaceable).toBe(false);
  });

  it('sets ctx.module when module is found', async () => {
    const mod = makeModule();
    const reg = makeRegistry({ 'test.module': mod });
    const step = new BuiltinModuleLookup(reg);
    const pctx = makePipelineContext();
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.module).toBeDefined();
  });

  it('throws ModuleNotFoundError when module is not found', async () => {
    const reg = makeRegistry();
    const step = new BuiltinModuleLookup(reg);
    const pctx = makePipelineContext({ moduleId: 'nonexistent.module' });
    await expect(step.execute(pctx)).rejects.toThrow(/not found/i);
  });
});

// ---------------------------------------------------------------------------
// 4. BuiltinACLCheck
// ---------------------------------------------------------------------------

describe('BuiltinACLCheck', () => {
  it('has correct metadata', () => {
    const step = new BuiltinACLCheck(null);
    expect(step.name).toBe('acl_check');
    expect(step.removable).toBe(true);
    expect(step.replaceable).toBe(true);
  });

  it('continues when acl is null', async () => {
    const step = new BuiltinACLCheck(null);
    const pctx = makePipelineContext();
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
  });

  it('continues when access is allowed', async () => {
    const acl = makeMockACL(true);
    const step = new BuiltinACLCheck(acl);
    const pctx = makePipelineContext();
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
  });

  it('throws ACLDeniedError when access is denied', async () => {
    const acl = makeMockACL(false);
    const step = new BuiltinACLCheck(acl);
    const pctx = makePipelineContext();
    await expect(step.execute(pctx)).rejects.toThrow(/denied/i);
  });
});

// ---------------------------------------------------------------------------
// 5. BuiltinApprovalGate
// ---------------------------------------------------------------------------

describe('BuiltinApprovalGate', () => {
  it('has correct metadata', () => {
    const step = new BuiltinApprovalGate(null);
    expect(step.name).toBe('approval_gate');
    expect(step.removable).toBe(true);
    expect(step.replaceable).toBe(true);
  });

  it('continues when handler is null', async () => {
    const step = new BuiltinApprovalGate(null);
    const pctx = makePipelineContext();
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
  });

  it('continues when module does not require approval', async () => {
    const handler = makeMockApprovalHandler(createApprovalResult({ status: 'approved' }));
    const step = new BuiltinApprovalGate(handler);
    const mod = makeModule({ annotations: { requiresApproval: false } });
    const pctx = makePipelineContext({ module: mod });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(handler.requestApproval).not.toHaveBeenCalled();
  });

  it('continues when approved', async () => {
    const handler = makeMockApprovalHandler(createApprovalResult({ status: 'approved', approvedBy: 'admin' }));
    const step = new BuiltinApprovalGate(handler);
    const mod = makeModule({ annotations: { requiresApproval: true } });
    const pctx = makePipelineContext({ module: mod, inputs: { key: 'value' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
  });

  it('throws ApprovalDeniedError when rejected', async () => {
    const handler = makeMockApprovalHandler(createApprovalResult({ status: 'rejected', reason: 'not allowed' }));
    const step = new BuiltinApprovalGate(handler);
    const mod = makeModule({ annotations: { requiresApproval: true } });
    const pctx = makePipelineContext({ module: mod });
    await expect(step.execute(pctx)).rejects.toThrow(/denied/i);
  });

  it('throws ApprovalTimeoutError when timed out', async () => {
    const handler = makeMockApprovalHandler(createApprovalResult({ status: 'timeout' }));
    const step = new BuiltinApprovalGate(handler);
    const mod = makeModule({ annotations: { requiresApproval: true } });
    const pctx = makePipelineContext({ module: mod });
    await expect(step.execute(pctx)).rejects.toThrow(/timed out/i);
  });

  it('throws ApprovalPendingError when pending', async () => {
    const handler = makeMockApprovalHandler(createApprovalResult({ status: 'pending', approvalId: 'abc123' }));
    const step = new BuiltinApprovalGate(handler);
    const mod = makeModule({ annotations: { requiresApproval: true } });
    const pctx = makePipelineContext({ module: mod });
    await expect(step.execute(pctx)).rejects.toThrow(/pending/i);
  });

  it('strips _approval_token from inputs and uses checkApproval', async () => {
    const handler = makeMockApprovalHandler(createApprovalResult({ status: 'approved', approvedBy: 'admin' }));
    const step = new BuiltinApprovalGate(handler);
    const mod = makeModule({ annotations: { requiresApproval: true } });
    const pctx = makePipelineContext({
      module: mod,
      inputs: { key: 'value', _approval_token: 'tok123' },
    });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(handler.checkApproval).toHaveBeenCalledWith('tok123');
    expect(pctx.inputs).not.toHaveProperty('_approval_token');
  });

  it('rejects a non-string _approval_token with GENERAL_INVALID_INPUT', async () => {
    const handler = makeMockApprovalHandler(createApprovalResult({ status: 'approved' }));
    const step = new BuiltinApprovalGate(handler);
    const mod = makeModule({ annotations: { requiresApproval: true } });
    const pctx = makePipelineContext({
      module: mod,
      inputs: { key: 'value', _approval_token: 12345 },
    });
    await expect(step.execute(pctx)).rejects.toMatchObject({
      code: 'GENERAL_INVALID_INPUT',
    });
    expect(handler.checkApproval).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// 6. BuiltinInputValidation
// ---------------------------------------------------------------------------

describe('BuiltinInputValidation', () => {
  it('has correct metadata', () => {
    const step = new BuiltinInputValidation();
    expect(step.name).toBe('input_validation');
    expect(step.removable).toBe(true);
    expect(step.replaceable).toBe(true);
  });

  it('continues when no input schema', async () => {
    const step = new BuiltinInputValidation();
    const mod = makeModule();
    const pctx = makePipelineContext({ module: mod, inputs: { foo: 'bar' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.validatedInputs).toEqual({ foo: 'bar' });
  });

  it('validates inputs against schema', async () => {
    const step = new BuiltinInputValidation();
    const mod = makeModule({
      inputSchema: {
        type: 'object',
        properties: { name: { type: 'string' } },
        required: ['name'],
      },
    });
    const pctx = makePipelineContext({ module: mod, inputs: { name: 'test' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.validatedInputs).toEqual({ name: 'test' });
  });

  it('throws SchemaValidationError on schema validation failure', async () => {
    const step = new BuiltinInputValidation();
    const mod = makeModule({
      inputSchema: {
        type: 'object',
        properties: { name: { type: 'string' } },
        required: ['name'],
      },
    });
    const pctx = makePipelineContext({ module: mod, inputs: {} });
    await expect(step.execute(pctx)).rejects.toThrow(/validation failed/i);
  });

  it('performs no type coercion — rejects a numeric string for an integer', async () => {
    // TYPE_MAPPING §17.3: the module-invocation boundary never coerces, under
    // any host configuration. A contract that declares `integer` receives an
    // integer. apcore-python accepted `"42"` here until it was aligned.
    const step = new BuiltinInputValidation();
    const mod = makeModule({
      inputSchema: {
        type: 'object',
        properties: { a: { type: 'integer' } },
        required: ['a'],
      },
    });
    const pctx = makePipelineContext({ module: mod, inputs: { a: '42' } });
    await expect(step.execute(pctx)).rejects.toThrow(/validation failed/i);
  });

  it('performs no type coercion on outputs either', async () => {
    const step = new BuiltinOutputValidation();
    const mod = makeModule({
      outputSchema: {
        type: 'object',
        properties: { a: { type: 'integer' } },
        required: ['a'],
      },
    });
    const pctx = makePipelineContext({ module: mod, output: { a: '42' } });
    await expect(step.execute(pctx)).rejects.toThrow(/validation failed/i);
  });

  it('enforces oneOf mutual exclusivity — rejects ambiguous input (regression: builtin-steps bypass)', async () => {
    // Both branches of the oneOf schema match { value: 42 } — should be rejected.
    // Before the fix, the local validateSchema used Value.Check which accepts anyOf semantics,
    // silently passing ambiguous inputs.
    const step = new BuiltinInputValidation();
    const mod = makeModule({
      inputSchema: {
        oneOf: [
          { type: 'object', properties: { value: { type: 'integer' } }, required: ['value'] },
          { type: 'object', properties: { value: { type: 'number' } }, required: ['value'] },
        ],
      },
    });
    const pctx = makePipelineContext({ module: mod, inputs: { value: 42 } });
    await expect(step.execute(pctx)).rejects.toThrow(/validation failed/i);
  });

  it('redacts sensitive fields with _secret_ prefix convention', async () => {
    const step = new BuiltinInputValidation();
    const mod = makeModule({
      inputSchema: {
        type: 'object',
        properties: {
          name: { type: 'string' },
          _secret_password: { type: 'string' },
        },
        required: ['name', '_secret_password'],
      },
    });
    const pctx = makePipelineContext({
      module: mod,
      inputs: { name: 'test', _secret_password: 'secret123' },
    });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.context.redactedInputs).toBeDefined();
    expect(pctx.context.redactedInputs!['_secret_password']).toBe('***REDACTED***');
    expect(pctx.context.redactedInputs!['name']).toBe('test');
  });
});

// ---------------------------------------------------------------------------
// 7. BuiltinMiddlewareBefore
// ---------------------------------------------------------------------------

describe('BuiltinMiddlewareBefore', () => {
  it('has correct metadata', () => {
    const mm = new MiddlewareManager();
    const step = new BuiltinMiddlewareBefore(mm);
    expect(step.name).toBe('middleware_before');
    expect(step.removable).toBe(true);
    expect(step.replaceable).toBe(false);
  });

  it('continues with empty middleware list', async () => {
    const mm = new MiddlewareManager();
    const step = new BuiltinMiddlewareBefore(mm);
    const pctx = makePipelineContext({ inputs: { key: 'value' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.inputs).toEqual({ key: 'value' });
  });

  it('applies before-middleware transformation', async () => {
    const mm = new MiddlewareManager();
    mm.add(new BeforeMiddleware((_moduleId, inputs, _ctx) => {
      return { ...inputs, injected: true };
    }));
    const step = new BuiltinMiddlewareBefore(mm);
    const pctx = makePipelineContext({ inputs: { key: 'value' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.inputs).toEqual({ key: 'value', injected: true });
  });

  it('throws original error on middleware chain error (no recovery)', async () => {
    const mm = new MiddlewareManager();
    mm.add(new BeforeMiddleware(() => {
      throw new Error('middleware boom');
    }));
    const step = new BuiltinMiddlewareBefore(mm);
    const pctx = makePipelineContext();
    await expect(step.execute(pctx)).rejects.toThrow('middleware boom');
  });
});

// ---------------------------------------------------------------------------
// 8. BuiltinExecute
// ---------------------------------------------------------------------------

describe('BuiltinExecute', () => {
  it('has correct metadata', () => {
    const step = new BuiltinExecute(null);
    expect(step.name).toBe('execute');
    expect(step.removable).toBe(false);
    expect(step.replaceable).toBe(true);
  });

  it('executes module and sets ctx.output', async () => {
    const step = new BuiltinExecute(null);
    const mod = makeModule();
    const pctx = makePipelineContext({ module: mod, inputs: { x: 1 } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.output).toEqual({ result: 'ok', x: 1 });
  });

  it('throws InvalidInputError when module has no execute method', async () => {
    const step = new BuiltinExecute(null);
    const mod = { id: 'test.module' };
    const pctx = makePipelineContext({ module: mod });
    await expect(step.execute(pctx)).rejects.toThrow(/no execute method/i);
  });

  it('throws ExecutionCancelledError on cancel token', async () => {
    const step = new BuiltinExecute(null);
    const cancelToken = new CancelToken();
    cancelToken.cancel();
    const ctx = new Context('trace-id', null, ['test.module'], null, null, null, {}, cancelToken);
    const mod = makeModule();
    const pctx = makePipelineContext({ module: mod, context: ctx });
    await expect(step.execute(pctx)).rejects.toThrow();
  });

  it('throws ModuleTimeoutError when global deadline exceeded', async () => {
    const step = new BuiltinExecute(null);
    // Epoch seconds, one second in the past (D-99).
    const ctx = Context.create(null, null, null, undefined, null, Date.now() / 1000 - 1)
      .child('test.module');
    const mod = makeModule();
    const pctx = makePipelineContext({ module: mod, context: ctx });
    await expect(step.execute(pctx)).rejects.toThrow(/timed out/i);
  });

  it('handles streaming mode by setting outputStream', async () => {
    const step = new BuiltinExecute(null);
    async function* streamGen(_inputs: Record<string, unknown>): AsyncGenerator<Record<string, unknown>> {
      yield { chunk: 1 };
      yield { chunk: 2 };
    }
    const mod = makeModule({ stream: streamGen });
    const pctx = makePipelineContext({ module: mod, stream: true });
    const result = await step.execute(pctx);
    // Stream path returns skip_to=return_result so post-execute steps record as
    // skipped in PipelineTrace, matching apcore-python (sync A-D-019).
    expect(result.action).toBe('skip_to');
    expect((result as { skipTo?: string }).skipTo).toBe('return_result');
    expect(pctx.outputStream).toBeDefined();
  });
});

// ---------------------------------------------------------------------------
// 9. BuiltinOutputValidation
// ---------------------------------------------------------------------------

describe('BuiltinOutputValidation', () => {
  it('has correct metadata', () => {
    const step = new BuiltinOutputValidation();
    expect(step.name).toBe('output_validation');
    expect(step.removable).toBe(true);
    expect(step.replaceable).toBe(true);
  });

  it('continues when no output schema', async () => {
    const step = new BuiltinOutputValidation();
    const mod = makeModule();
    const pctx = makePipelineContext({ module: mod, output: { result: 'ok' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.validatedOutput).toEqual({ result: 'ok' });
  });

  it('validates output against schema', async () => {
    const step = new BuiltinOutputValidation();
    const mod = makeModule({
      outputSchema: {
        type: 'object',
        properties: { result: { type: 'string' } },
        required: ['result'],
      },
    });
    const pctx = makePipelineContext({ module: mod, output: { result: 'ok' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.validatedOutput).toEqual({ result: 'ok' });
  });

  it('throws SchemaValidationError on output schema validation failure', async () => {
    const step = new BuiltinOutputValidation();
    const mod = makeModule({
      outputSchema: {
        type: 'object',
        properties: { result: { type: 'string' } },
        required: ['result'],
      },
    });
    const pctx = makePipelineContext({ module: mod, output: {} });
    await expect(step.execute(pctx)).rejects.toThrow(/validation failed/i);
  });
});

// ---------------------------------------------------------------------------
// 10. BuiltinMiddlewareAfter
// ---------------------------------------------------------------------------

describe('BuiltinMiddlewareAfter', () => {
  it('has correct metadata', () => {
    const mm = new MiddlewareManager();
    const step = new BuiltinMiddlewareAfter(mm);
    expect(step.name).toBe('middleware_after');
    expect(step.removable).toBe(true);
    expect(step.replaceable).toBe(false);
  });

  it('continues with empty middleware list', async () => {
    const mm = new MiddlewareManager();
    const step = new BuiltinMiddlewareAfter(mm);
    const pctx = makePipelineContext({ output: { result: 'ok' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.output).toEqual({ result: 'ok' });
  });

  it('applies after-middleware transformation', async () => {
    const mm = new MiddlewareManager();
    mm.add(new AfterMiddleware((_moduleId, _inputs, output, _ctx) => {
      return { ...output, enriched: true };
    }));
    const step = new BuiltinMiddlewareAfter(mm);
    const pctx = makePipelineContext({ output: { result: 'ok' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    expect(pctx.output).toEqual({ result: 'ok', enriched: true });
  });
});

// ---------------------------------------------------------------------------
// 11. BuiltinReturnResult
// ---------------------------------------------------------------------------

describe('BuiltinReturnResult', () => {
  it('has correct metadata', () => {
    const step = new BuiltinReturnResult();
    expect(step.name).toBe('return_result');
    expect(step.removable).toBe(false);
    expect(step.replaceable).toBe(false);
  });

  it('returns continue (no-op finalization)', async () => {
    const step = new BuiltinReturnResult();
    const pctx = makePipelineContext({ output: { result: 'ok' } });
    const result = await step.execute(pctx);
    expect(result.action).toBe('continue');
    // Output unchanged
    expect(pctx.output).toEqual({ result: 'ok' });
  });
});

// ---------------------------------------------------------------------------
// buildStandardStrategy
// ---------------------------------------------------------------------------

describe('buildStandardStrategy', () => {
  it('creates an ExecutionStrategy with 11 steps', () => {
    const reg = makeRegistry();
    const mm = new MiddlewareManager();
    const strategy = buildStandardStrategy({
      config: null,
      registry: reg,
      acl: null,
      approvalHandler: null,
      middlewareManager: mm,
    });
    expect(strategy.name).toBe('standard');
    // PROTOCOL_SPEC §5 11-step pipeline; toggle check inlined into module_lookup (sync A-D-011)
    expect(strategy.steps).toHaveLength(11);
  });

  it('has steps in the correct order', () => {
    const reg = makeRegistry();
    const mm = new MiddlewareManager();
    const strategy = buildStandardStrategy({
      config: null,
      registry: reg,
      acl: null,
      approvalHandler: null,
      middlewareManager: mm,
    });
    const names = strategy.stepNames();
    // PROTOCOL_SPEC §5 11-step pipeline (sync A-D-011)
    expect(names).toEqual([
      'context_creation',
      'call_chain_guard',
      'module_lookup',
      'acl_check',
      'approval_gate',
      'middleware_before',
      'input_validation',
      'execute',
      'output_validation',
      'middleware_after',
      'return_result',
    ]);
  });

  it('has correct removable/replaceable flags for each step', () => {
    const reg = makeRegistry();
    const mm = new MiddlewareManager();
    const strategy = buildStandardStrategy({
      config: null,
      registry: reg,
      acl: null,
      approvalHandler: null,
      middlewareManager: mm,
    });
    const steps = strategy.steps;
    // 11-step pipeline indexes (toggle inlined into module_lookup, sync A-D-011):
    // [0] context_creation, [1] call_chain_guard, [2] module_lookup,
    // [3] acl_check, [4] approval_gate, [5] middleware_before,
    // [6] input_validation, [7] execute, [8] output_validation,
    // [9] middleware_after, [10] return_result
    expect(steps[0].removable).toBe(false);   // context_creation
    expect(steps[2].removable).toBe(false);   // module_lookup
    expect(steps[7].removable).toBe(false);   // execute
    expect(steps[10].removable).toBe(false);  // return_result

    expect(steps[0].replaceable).toBe(false);  // context_creation
    expect(steps[2].replaceable).toBe(false);  // module_lookup
    expect(steps[5].replaceable).toBe(false);  // middleware_before
    expect(steps[9].replaceable).toBe(false);  // middleware_after
    expect(steps[10].replaceable).toBe(false); // return_result
  });
});
