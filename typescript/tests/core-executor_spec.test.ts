/**
 * Spec-traced contract tests for the apcore TypeScript SDK core-executor feature.
 *
 * Source spec: apcore/docs/features/core-executor.md
 * Canonical clause set mirrored from apcore-python
 *   tests/test_core_executor_spec.py (31 tests).
 *
 * Each `it(...)` name begins with the verbatim cross-language clause id so a
 * cross-language diff tool can line up the Python / TypeScript rows. The clause
 * ids follow the pattern `core_executor.<method>.<kind>.<detail>`.
 *
 * Contract blocks covered:
 *   - Executor.call
 *   - Context.create
 *   - Executor binding to Context
 *   - Pipeline.configure_step
 *   - Distributed cancellation
 *   - global_deadline distributed semantics
 *
 * NOTE on divergences (recorded in the agent report):
 *   - Module-id entry-guard rejection raises InvalidInputError with code
 *     INVALID_MODULE_ID (aligned with apcore-python).
 *   - TS Context is immutable: _withExecutor() returns a NEW bound Context
 *     instead of mutating the caller's instance, so binding is observed on the
 *     returned instance, not on the passed-in one.
 */

import { describe, it, expect } from 'vitest';
import { Executor } from '../src/executor.js';
import { Context, Identity } from '../src/context.js';
import { Config } from '../src/config.js';
import { CancelToken } from '../src/cancel.js';
import {
  CallDepthExceededError,
  ContextBindingError,
  InvalidInputError,
  ModuleNotFoundError,
} from '../src/errors.js';
import {
  ExecutionStrategy,
  PipelineStepNotFoundError,
  StepNotReplaceableError,
  type PipelineContext,
  type Step,
  type StepResult,
} from '../src/pipeline.js';
import { Registry } from '../src/registry/registry.js';

// ---------------------------------------------------------------------------
// Minimal module + registry/executor helpers
// ---------------------------------------------------------------------------

/** Schema-less module: accepts any input, echoes a deterministic dict. */
const echoModule = {
  inputSchema: null,
  outputSchema: null,
  annotations: null,
  async execute(inputs: Record<string, unknown>): Promise<Record<string, unknown>> {
    return { echo: inputs['name'] ?? 'anon' };
  },
};

async function makeRegistry(moduleId = 'test.echo'): Promise<Registry> {
  const reg = new Registry();
  await reg.register(moduleId, { ...echoModule });
  return reg;
}

async function makeExecutor(moduleId = 'test.echo', config: Config | null = null): Promise<Executor> {
  return new Executor({ registry: await makeRegistry(moduleId), config });
}

// ---------------------------------------------------------------------------
// Custom pipeline step used by configure_step contract tests
// ---------------------------------------------------------------------------

class NoopStep implements Step {
  readonly name: string;
  readonly description: string;
  readonly removable = true;
  readonly replaceable: boolean;

  constructor(name: string, description = '', replaceable = true) {
    this.name = name;
    this.description = description;
    this.replaceable = replaceable;
  }

  async execute(_ctx: PipelineContext): Promise<StepResult> {
    return { action: 'continue' };
  }
}

function strategyWith(...names: string[]): ExecutionStrategy {
  const steps = names.map((n) => new NoopStep(n, `step ${n}`));
  return new ExecutionStrategy('custom', steps);
}

// ===========================================================================
// Contract: Executor.call
// ===========================================================================

describe('Contract: Executor.call', () => {
  it('core_executor.call.input.module_id.empty: empty module_id fails entry-guard before pipeline', async () => {
    const ex = await makeExecutor();
    await expect(ex.call('', { name: 'x' })).rejects.toBeInstanceOf(InvalidInputError);
    // Capture the error to assert the code field.
    let caught: unknown;
    try {
      await ex.call('', { name: 'x' });
    } catch (e) {
      caught = e;
    }
    // Entry-guard raises INVALID_MODULE_ID (aligned with apcore-python).
    expect((caught as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('core_executor.call.input.module_id.malformed: malformed module_id rejected at entry-guard', async () => {
    const ex = await makeExecutor();
    let caught: unknown;
    try {
      await ex.call('not a valid id!!', { name: 'x' });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(InvalidInputError);
    expect((caught as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('core_executor.call.error.INVALID_MODULE_ID: invalid module_id raises InvalidInputError', async () => {
    const ex = await makeExecutor();
    let caught: unknown;
    try {
      await ex.call('   ', {});
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(InvalidInputError);
    expect((caught as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('core_executor.call.error.MODULE_NOT_FOUND: valid-but-absent module raises ModuleNotFoundError', async () => {
    const ex = await makeExecutor();
    let caught: unknown;
    try {
      await ex.call('test.absent_module', { name: 'x' });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ModuleNotFoundError);
    expect((caught as ModuleNotFoundError).code).toBe('MODULE_NOT_FOUND');
  });

  it('core_executor.call.error.CALL_DEPTH_EXCEEDED: chain length over max raises CALL_DEPTH_EXCEEDED', async () => {
    const config = new Config({ executor: { max_call_depth: 2 } });
    const ex = await makeExecutor('test.echo', config);
    // TS Context is immutable; construct a Context with a pre-filled callChain
    // directly (Context.create does not accept call_chain — it is Executor-managed).
    const ctx = new Context('a'.repeat(32), null, ['a', 'b', 'c']);
    let caught: unknown;
    try {
      await ex.call('test.echo', { name: 'x' }, ctx);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(CallDepthExceededError);
    expect((caught as CallDepthExceededError).code).toBe('CALL_DEPTH_EXCEEDED');
  });

  it('core_executor.call.side_effect.1.validate_module_id: entry-guard runs before pipeline lookup', async () => {
    const ex = await makeExecutor();
    let caught: unknown;
    try {
      await ex.call('bad id', { name: 'x' });
    } catch (e) {
      caught = e;
    }
    // If the guard were deferred into the pipeline, lookup would have produced
    // ModuleNotFoundError; the typed entry-guard error proves ordering.
    expect(caught).toBeInstanceOf(InvalidInputError);
    expect(caught).not.toBeInstanceOf(ModuleNotFoundError);
    expect((caught as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('core_executor.call.side_effect.2.run_pipeline: pipeline runs and returns validated output', async () => {
    const ex = await makeExecutor();
    const result = await ex.call('test.echo', { name: 'Alice' });
    expect(result).toEqual({ echo: 'Alice' });
  });

  it('core_executor.call.property.thread_safe: >=8 concurrent calls each observe their own result', async () => {
    const ex = await makeExecutor();
    const results = await Promise.all(
      Array.from({ length: 8 }, (_, i) => ex.call('test.echo', { name: `u${i}` })),
    );
    expect(results.map((r) => r['echo'])).toEqual(Array.from({ length: 8 }, (_, i) => `u${i}`));
  });

  it('core_executor.call.property.async: call() returns a Promise resolving to the module output', async () => {
    const ex = await makeExecutor();
    const promise = ex.call('test.echo', { name: 'Bob' });
    expect(promise).toBeInstanceOf(Promise);
    const result = await promise;
    expect(result).toEqual({ echo: 'Bob' });
  });

  it('core_executor.call.property.pure_false: distinct inputs yield distinct outputs (not a frozen constant)', async () => {
    const ex = await makeExecutor();
    const r1 = await ex.call('test.echo', { name: 'x' });
    const r2 = await ex.call('test.echo', { name: 'y' });
    expect(r1).toEqual({ echo: 'x' });
    expect(r2).toEqual({ echo: 'y' });
    expect(r1).not.toEqual(r2);
  });
});

// ===========================================================================
// Contract: Context.create
// ===========================================================================

describe('Contract: Context.create', () => {
  it('core_executor.context_create.return.trace_id_32hex: fresh Context has 32-char lowercase hex traceId', () => {
    const ctx = Context.create();
    expect(ctx.traceId).toHaveLength(32);
    expect(/^[0-9a-f]{32}$/.test(ctx.traceId)).toBe(true);
  });

  it('core_executor.context_create.return.unset_managed_fields: executor/callerId/callChain unset at top level', () => {
    const ident = new Identity('svc');
    const ctx = Context.create(ident);
    expect(ctx.executor).toBeNull();
    expect(ctx.callerId).toBeNull();
    expect(ctx.callChain).toEqual([]);
    expect(ctx.identity).toBe(ident);
  });

  it('core_executor.context_create.property.idempotent_false: two calls yield different traceIds', () => {
    const a = Context.create();
    const b = Context.create();
    expect(a.traceId).not.toBe(b.traceId);
  });

  it('core_executor.context_create.property.pure_false: new traceId generated each call', () => {
    const seen = new Set(Array.from({ length: 5 }, () => Context.create().traceId));
    expect(seen.size).toBe(5);
  });

  it('core_executor.context_create.property.thread_safe: >=8 concurrent constructions produce distinct trace_ids', async () => {
    const ctxs = await Promise.all(
      Array.from({ length: 8 }, () => Promise.resolve(Context.create())),
    );
    const traceIds = ctxs.map((c) => c.traceId);
    expect(new Set(traceIds).size).toBe(8);
    expect(traceIds.every((t) => t.length === 32)).toBe(true);
  });

  it('core_executor.context_create.property.async_false: create() returns a Context, not a Promise', () => {
    const result = Context.create();
    expect(result).toBeInstanceOf(Context);
    expect(result instanceof Promise).toBe(false);
  });

  it('core_executor.context_create.error.invalid_trace_parent_no_raise: invalid trace_parent regenerates instead of raising', () => {
    const badTraceParent = {
      traceId: 'zzzz',
      traceFlags: '01',
      tracestate: undefined,
    } as unknown as Parameters<typeof Context.create>[1];
    const ctx = Context.create(null, badTraceParent);
    expect(ctx.traceId).toHaveLength(32);
    expect(ctx.traceId).not.toBe('zzzz');
  });
});

// ===========================================================================
// Contract: Executor binding to Context
// ===========================================================================

describe('Contract: Executor binding to Context', () => {
  it('core_executor.binding.side_effect.1.bind_before_pipeline: executor bound onto Context before pipeline', async () => {
    const ex = await makeExecutor();
    const ctx = Context.create();
    expect(ctx.executor).toBeNull();
    // TS divergence: Context is immutable — _withExecutor returns a NEW bound
    // instance. The bound executor is observed on that instance (the SDK
    // mechanism the Executor uses internally before pipeline step 1).
    const bound = ctx._withExecutor(ex);
    expect(bound.executor).toBe(ex);
  });

  it('core_executor.binding.idempotent.same_executor_noop: same-executor rebind is a noop and does not raise', async () => {
    const ex = await makeExecutor();
    const ctx = Context.create();
    const r1 = await ex.call('test.echo', { name: 'same' }, ctx);
    const r2 = await ex.call('test.echo', { name: 'same' }, ctx);
    expect(r1).toEqual({ echo: 'same' });
    expect(r2).toEqual({ echo: 'same' });
    // Rebinding the same executor on the same context returns the identical
    // instance (idempotent noop) and never raises.
    const b1 = ctx._withExecutor(ex);
    const b2 = b1._withExecutor(ex);
    expect(b2).toBe(b1);
  });

  it('core_executor.binding.error.CONTEXT_BINDING_ERROR: cross-executor conflict raises ContextBindingError', async () => {
    const exA = await makeExecutor();
    const exB = new Executor({ registry: await makeRegistry() });
    const ctx = Context.create();
    const boundA = ctx._withExecutor(exA);
    expect(boundA.executor).toBe(exA);
    // A Context already bound to a DIFFERENT executor must raise on rebind.
    let caught: unknown;
    try {
      boundA._withExecutor(exB);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ContextBindingError);
    expect((caught as ContextBindingError).code).toBe('CONTEXT_BINDING_ERROR');
  });

  it('core_executor.binding.side_effect.2.stability: once bound, executor reference is stable', async () => {
    const ex = await makeExecutor();
    const ctx = Context.create();
    const bound1 = ctx._withExecutor(ex);
    const first = bound1.executor;
    const bound2 = bound1._withExecutor(ex);
    expect(bound2.executor).toBe(first);
    expect(bound2.executor).toBe(ex);
  });
});

// ===========================================================================
// Contract: Pipeline.configure_step
// ===========================================================================

describe('Contract: Pipeline.configure_step', () => {
  it('core_executor.configure_step.input.step_name.not_found: missing step_name fails lookup', () => {
    const strat = strategyWith('alpha', 'beta');
    expect(() => strat.configureStep('does_not_exist', new NoopStep('does_not_exist', ''))).toThrow(
      PipelineStepNotFoundError,
    );
  });

  it('core_executor.configure_step.error.PIPELINE_STEP_NOT_FOUND: absent step_name raises PIPELINE_STEP_NOT_FOUND', () => {
    const strat = strategyWith('alpha');
    let caught: unknown;
    try {
      strat.configureStep('ghost', new NoopStep('ghost', ''));
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(PipelineStepNotFoundError);
    expect((caught as PipelineStepNotFoundError).code).toBe('PIPELINE_STEP_NOT_FOUND');
  });

  it('core_executor.configure_step.error.StepNotReplaceableError: non-replaceable step raises StepNotReplaceableError', () => {
    const fixed = new NoopStep('fixed', '', /* replaceable */ false);
    const strat = new ExecutionStrategy('custom', [fixed]);
    expect(() => strat.configureStep('fixed', new NoopStep('fixed', ''))).toThrow(
      StepNotReplaceableError,
    );
  });

  it('core_executor.configure_step.side_effect.1.replace_in_place: replacing keeps one step and preserves position', () => {
    const strat = strategyWith('alpha', 'beta', 'gamma');
    const replacement = new NoopStep('beta', 'replaced beta');
    strat.configureStep('beta', replacement);
    const names = strat.stepNames();
    expect(names).toEqual(['alpha', 'beta', 'gamma']);
    expect(names.filter((n) => n === 'beta')).toHaveLength(1);
    expect(strat.steps[1]).toBe(replacement);
  });

  it('core_executor.configure_step.property.idempotent_true: replacing the same step twice leaves exactly one', () => {
    const strat = strategyWith('alpha', 'beta');
    strat.configureStep('beta', new NoopStep('beta', 'v'));
    const namesAfterFirst = strat.stepNames();
    strat.configureStep('beta', new NoopStep('beta', 'v'));
    expect(strat.stepNames()).toEqual(namesAfterFirst);
    expect(strat.stepNames()).toEqual(['alpha', 'beta']);
    expect(strat.stepNames().filter((n) => n === 'beta')).toHaveLength(1);
  });

  it('core_executor.configure_step.return.none: configureStep returns void on success', () => {
    const strat = strategyWith('alpha');
    expect(strat.configureStep('alpha', new NoopStep('alpha', ''))).toBeUndefined();
  });
});

// ===========================================================================
// Contract: Distributed cancellation
// ===========================================================================

describe('Contract: Distributed cancellation', () => {
  it('core_executor.distributed_cancellation.input.cancel_token_in_process: caller token carried on Context', () => {
    const token = new CancelToken();
    const ctx = Context.create(null, null, token);
    expect(ctx.cancelToken).toBe(token);
  });

  it('core_executor.distributed_cancellation.side_effect.1.no_serialize: cancel_token not serialized across round-trip', () => {
    const token = new CancelToken();
    const ctx = Context.create(null, null, token);
    const serialized = ctx.serialize();
    expect(serialized['cancel_token'] === undefined || serialized['cancel_token'] === null).toBe(
      true,
    );
    const revived = Context.deserialize(serialized);
    expect(revived.cancelToken).not.toBe(token);
    expect(revived.cancelToken).toBeNull();
  });
});

// ===========================================================================
// Contract: global_deadline distributed semantics
// ===========================================================================

describe('Contract: global_deadline distributed semantics', () => {
  it('core_executor.global_deadline.input.local_only: global_deadline carried locally on the Context', () => {
    const deadline = Date.now() + 30_000;
    const ctx = Context.create(null, null, null, undefined, undefined, deadline);
    expect(ctx.globalDeadline).toBe(deadline);
  });

  it('core_executor.global_deadline.side_effect.1.no_serialize: global_deadline not serialized across round-trip', () => {
    const ctx = Context.create(null, null, null, undefined, undefined, Date.now() + 30_000);
    const serialized = ctx.serialize();
    expect(
      serialized['global_deadline'] === undefined || serialized['global_deadline'] === null,
    ).toBe(true);
    const revived = Context.deserialize(serialized);
    expect(revived.globalDeadline).toBeNull();
  });
});
