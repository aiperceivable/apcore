/**
 * Tests for pipeline.ts: ExecutionStrategy, PipelineEngine, and error types.
 */

import { describe, expect, it } from 'vitest';
import { ModuleTimeoutError } from '../src/errors.js';
import { Context } from '../src/context.js';
import {
  ExecutionStrategy,
  PipelineAbortError,
  PipelineEngine,
  PipelineStepError,
  PipelineStepNotFoundError,
  StepNameDuplicateError,
  StepNotFoundError,
  StepNotRemovableError,
  StepNotReplaceableError,
  StrategyNotFoundError,
} from '../src/pipeline.js';
import type { PipelineContext, PipelineState, Step, StepResult } from '../src/pipeline.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeStep(
  name: string,
  opts: {
    removable?: boolean;
    replaceable?: boolean;
    action?: 'continue' | 'abort' | 'skip_to';
    skipTo?: string;
    explanation?: string;
    ignoreErrors?: boolean;
    pure?: boolean;
    matchModules?: string[] | null;
    timeoutMs?: number;
    throws?: boolean;
    requires?: string[];
    provides?: string[];
    confidence?: number;
  } = {},
): Step {
  return {
    name,
    description: `Step ${name}`,
    removable: opts.removable ?? true,
    replaceable: opts.replaceable ?? true,
    ignoreErrors: opts.ignoreErrors,
    pure: opts.pure,
    matchModules: opts.matchModules,
    timeoutMs: opts.timeoutMs,
    requires: opts.requires,
    provides: opts.provides,
    execute: async (): Promise<StepResult> => {
      if (opts.throws) throw new Error(`Step ${name} failed`);
      return {
        action: opts.action ?? 'continue',
        skipTo: opts.skipTo,
        explanation: opts.explanation,
        confidence: opts.confidence,
      };
    },
  };
}

function makeContext(moduleId: string = 'test.module'): PipelineContext {
  return {
    moduleId,
    inputs: {},
    context: new Context('trace-id', null, []),
  };
}

// ---------------------------------------------------------------------------
// ExecutionStrategy: construction
// ---------------------------------------------------------------------------

describe('ExecutionStrategy constructor', () => {
  it('creates a strategy with the given name and steps', () => {
    const s = new ExecutionStrategy('default', [makeStep('a'), makeStep('b')]);
    expect(s.name).toBe('default');
    expect(s.steps).toHaveLength(2);
  });

  it('throws StepNameDuplicateError for duplicate step names', () => {
    expect(() => new ExecutionStrategy('dup', [makeStep('a'), makeStep('a')])).toThrow(
      StepNameDuplicateError,
    );
  });

  it('throws PipelineDependencyError when requires are not satisfied by any preceding step (Issue #33 §2.1)', () => {
    expect(() => new ExecutionStrategy('bad', [makeStep('a', { requires: ['output'] })])).toThrow(
      /requires/,
    );
  });

  it('does not throw when requires are satisfied by a preceding step', () => {
    const stepA = makeStep('a', { provides: ['output'] });
    const stepB = makeStep('b', { requires: ['output'] });
    expect(() => new ExecutionStrategy('ok', [stepA, stepB])).not.toThrow();
  });

  it('exposes read-only steps array', () => {
    const s = new ExecutionStrategy('s', [makeStep('x')]);
    expect(s.steps).toHaveLength(1);
    expect(s.steps[0].name).toBe('x');
  });
});

// ---------------------------------------------------------------------------
// ExecutionStrategy: stepNames and info
// ---------------------------------------------------------------------------

describe('ExecutionStrategy.stepNames and info', () => {
  it('stepNames returns ordered names', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b'), makeStep('c')]);
    expect(s.stepNames()).toEqual(['a', 'b', 'c']);
  });

  it('info returns correct StrategyInfo', () => {
    const s = new ExecutionStrategy('myStrategy', [makeStep('x'), makeStep('y')]);
    const info = s.info();
    expect(info.name).toBe('myStrategy');
    expect(info.stepCount).toBe(2);
    expect(info.stepNames).toEqual(['x', 'y']);
    expect(info.description).toContain('→');
  });
});

// ---------------------------------------------------------------------------
// ExecutionStrategy: insertAfter / insertBefore
// ---------------------------------------------------------------------------

describe('ExecutionStrategy.insertAfter', () => {
  it('inserts a step after the named anchor', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('c')]);
    s.insertAfter('a', makeStep('b'));
    expect(s.stepNames()).toEqual(['a', 'b', 'c']);
  });

  it('throws StepNotFoundError when anchor not found', () => {
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    expect(() => s.insertAfter('missing', makeStep('b'))).toThrow(StepNotFoundError);
  });

  it('throws StepNameDuplicateError when step name already exists', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b')]);
    expect(() => s.insertAfter('a', makeStep('b'))).toThrow(StepNameDuplicateError);
  });
});

describe('ExecutionStrategy.insertBefore', () => {
  it('inserts a step before the named anchor', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('c')]);
    s.insertBefore('c', makeStep('b'));
    expect(s.stepNames()).toEqual(['a', 'b', 'c']);
  });

  it('throws StepNotFoundError when anchor not found', () => {
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    expect(() => s.insertBefore('missing', makeStep('b'))).toThrow(StepNotFoundError);
  });

  it('throws StepNameDuplicateError when step name already exists', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b')]);
    expect(() => s.insertBefore('b', makeStep('a'))).toThrow(StepNameDuplicateError);
  });
});

// ---------------------------------------------------------------------------
// ExecutionStrategy: remove / replace
// ---------------------------------------------------------------------------

describe('ExecutionStrategy.remove', () => {
  it('removes a removable step by name', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b'), makeStep('c')]);
    s.remove('b');
    expect(s.stepNames()).toEqual(['a', 'c']);
  });

  it('throws StepNotFoundError when step not found', () => {
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    expect(() => s.remove('missing')).toThrow(StepNotFoundError);
  });

  it('throws StepNotRemovableError when step is not removable', () => {
    const s = new ExecutionStrategy('s', [makeStep('locked', { removable: false })]);
    expect(() => s.remove('locked')).toThrow(StepNotRemovableError);
  });
});

describe('ExecutionStrategy.replace', () => {
  it('replaces a replaceable step by name', () => {
    const s = new ExecutionStrategy('s', [makeStep('a', { replaceable: true }), makeStep('b')]);
    s.replace('a', makeStep('a-new'));
    expect(s.stepNames()[0]).toBe('a-new');
  });

  it('throws StepNotFoundError when step not found', () => {
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    expect(() => s.replace('missing', makeStep('b'))).toThrow(StepNotFoundError);
  });

  it('throws StepNotReplaceableError when step is not replaceable', () => {
    const s = new ExecutionStrategy('s', [makeStep('locked', { replaceable: false })]);
    expect(() => s.replace('locked', makeStep('new'))).toThrow(StepNotReplaceableError);
  });

  it('throws StepNameDuplicateError when the new name belongs to a different step', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b'), makeStep('c')]);
    expect(() => s.replace('a', makeStep('c'))).toThrow(StepNameDuplicateError);
    // The strategy is untouched: no second 'c', index map still consistent.
    expect(s.stepNames()).toEqual(['a', 'b', 'c']);
    expect(s.findStepIndex('c')).toBe(2);
  });

  it('accepts a replacement that keeps the same name', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b')]);
    s.replace('a', makeStep('a'));
    expect(s.stepNames()).toEqual(['a', 'b']);
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: basic execution
// ---------------------------------------------------------------------------

describe('PipelineEngine.run', () => {
  it('runs all steps and returns output', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    ctx.output = { result: 'ok' };
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b')]);
    const [output, trace] = await engine.run(s, ctx);
    expect(output).toEqual({ result: 'ok' });
    expect(trace.success).toBe(true);
    expect(trace.steps).toHaveLength(2);
  });

  it('returns null output when ctx.output is not set', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    const [output] = await engine.run(s, ctx);
    expect(output).toBeNull();
  });

  it('records step traces with durationMs and result', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('step1')]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.steps[0].name).toBe('step1');
    expect(trace.steps[0].durationMs).toBeGreaterThanOrEqual(0);
    expect(trace.steps[0].skipped).toBe(false);
    expect(trace.steps[0].result.action).toBe('continue');
  });

  it('records moduleId and strategyName in trace', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext('mymod.something');
    const s = new ExecutionStrategy('myStrategy', [makeStep('a')]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.moduleId).toBe('mymod.something');
    expect(trace.strategyName).toBe('myStrategy');
  });

  it('attaches trace to ctx.trace', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    await engine.run(s, ctx);
    expect(ctx.trace).toBeDefined();
    expect(ctx.trace!.success).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: abort action
// ---------------------------------------------------------------------------

describe('PipelineEngine.run - abort', () => {
  it('throws PipelineAbortError when a step returns abort', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [
      makeStep('a', { action: 'abort', explanation: 'fatal' }),
    ]);
    await expect(engine.run(s, ctx)).rejects.toThrow(PipelineAbortError);
  });

  it('PipelineAbortError contains step name and explanation', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [
      makeStep('check', { action: 'abort', explanation: 'blocked' }),
    ]);
    try {
      await engine.run(s, ctx);
    } catch (e) {
      expect(e).toBeInstanceOf(PipelineAbortError);
      const err = e as PipelineAbortError;
      expect(err.step).toBe('check');
      expect(err.explanation).toBe('blocked');
    }
  });

  it('PipelineAbortError attaches pipeline trace', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [
      makeStep('first'),
      makeStep('stopper', { action: 'abort' }),
    ]);
    try {
      await engine.run(s, ctx);
    } catch (e) {
      const err = e as PipelineAbortError;
      expect(err.pipelineTrace).not.toBeNull();
      expect(err.pipelineTrace!.success).toBe(false);
    }
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: skip_to action
// ---------------------------------------------------------------------------

describe('PipelineEngine.run - skip_to', () => {
  it('skips steps when skip_to action is returned', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const executedSteps: string[] = [];
    const trackStep = (
      name: string,
      action: 'continue' | 'skip_to' = 'continue',
      skipTo?: string,
    ): Step => ({
      name,
      description: name,
      removable: true,
      replaceable: true,
      execute: async (): Promise<StepResult> => {
        executedSteps.push(name);
        return { action, skipTo };
      },
    });
    const s = new ExecutionStrategy('s', [
      trackStep('a', 'skip_to', 'c'),
      trackStep('b'),
      trackStep('c'),
    ]);
    await engine.run(s, ctx);
    expect(executedSteps).toEqual(['a', 'c']);
  });

  it('throws StepNotFoundError when skip_to target does not exist', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [
      makeStep('a', { action: 'skip_to', skipTo: 'nonexistent' }),
      makeStep('b'),
    ]);
    await expect(engine.run(s, ctx)).rejects.toThrow(StepNotFoundError);
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: matchModules filter
// ---------------------------------------------------------------------------

describe('PipelineEngine.run - matchModules', () => {
  it('skips a step whose matchModules does not match moduleId', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext('other.module');
    const s = new ExecutionStrategy('s', [makeStep('filtered', { matchModules: ['specific.*'] })]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.steps[0].skipped).toBe(true);
    expect(trace.steps[0].skipReason).toBe('no_match');
  });

  it('runs a step whose matchModules matches the moduleId', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext('specific.thing');
    const s = new ExecutionStrategy('s', [makeStep('matched', { matchModules: ['specific.*'] })]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.steps[0].skipped).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: dry_run filter
// ---------------------------------------------------------------------------

describe('PipelineEngine.run - dryRun', () => {
  it('skips non-pure steps in dryRun mode', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    ctx.dryRun = true;
    const s = new ExecutionStrategy('s', [makeStep('sideEffect', { pure: false })]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.steps[0].skipped).toBe(true);
    expect(trace.steps[0].skipReason).toBe('dry_run');
  });

  it('runs pure steps in dryRun mode', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    ctx.dryRun = true;
    const s = new ExecutionStrategy('s', [makeStep('pureStep', { pure: true })]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.steps[0].skipped).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: ignoreErrors
// ---------------------------------------------------------------------------

describe('PipelineEngine.run - ignoreErrors', () => {
  it('continues after a step that throws when ignoreErrors=true', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [
      makeStep('risky', { throws: true, ignoreErrors: true }),
      makeStep('after'),
    ]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.success).toBe(true);
    expect(trace.steps[0].skipReason).toBe('error_ignored');
    expect(trace.steps).toHaveLength(2);
  });

  it('propagates error when ignoreErrors=false', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('risky', { throws: true })]);
    await expect(engine.run(s, ctx)).rejects.toThrow('Step risky failed');
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: per-step timeout
// ---------------------------------------------------------------------------

describe('PipelineEngine.run - timeoutMs', () => {
  it('throws when step exceeds its timeoutMs', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const slowStep: Step = {
      name: 'slow',
      description: 'slow step',
      removable: true,
      replaceable: true,
      timeoutMs: 10,
      execute: async (): Promise<StepResult> => {
        await new Promise((resolve) => setTimeout(resolve, 200));
        return { action: 'continue' };
      },
    };
    const s = new ExecutionStrategy('s', [slowStep]);
    await expect(engine.run(s, ctx)).rejects.toThrow(/timed out/);
  }, 3000);

  function stalledStep(ignoreErrors = false): Step {
    return {
      ...makeStep('slow', { timeoutMs: 5, ignoreErrors }),
      execute: () => new Promise<StepResult>(() => {}),
    };
  }

  it('preserves the typed timeout cause, step identity and failure trace', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const strategy = new ExecutionStrategy('s', [stalledStep()]);
    const failure: unknown = await engine.run(strategy, ctx).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(PipelineStepError);
    const error = failure as PipelineStepError;
    expect(error.stepName).toBe('slow');
    expect(error.cause).toBeInstanceOf(ModuleTimeoutError);
    expect(error.cause).toMatchObject({
      code: 'MODULE_TIMEOUT', moduleId: 'test.module', timeoutMs: 5, retryable: true,
    });
    expect(error.pipelineTrace).toBe(ctx.trace);
    expect(error.pipelineTrace).toMatchObject({
      success: false,
      steps: [{ name: 'slow', result: { action: 'abort' }, skipped: false }],
    });
  });

  it('continues after an ignored timeout and records the ignored step', async () => {
    const engine = new PipelineEngine();
    const strategy = new ExecutionStrategy('s', [stalledStep(true), makeStep('after')]);
    const [, trace] = await engine.run(strategy, makeContext());
    expect(trace.success).toBe(true);
    expect(trace.steps.map((step) => step.name)).toEqual(['slow', 'after']);
    expect(trace.steps[0].skipReason).toBe('error_ignored');
  });

  it('allows timeout recovery and closes the step middleware hooks in reverse order', async () => {
    const engine = new PipelineEngine();
    const events: string[] = [];
    engine.addStepMiddleware({
      beforeStep: () => { events.push('before:outer'); },
      onStepError: () => { events.push('error:outer'); return null; },
      afterStep: (_name, _state, output) => {
        events.push('after:outer');
        expect(output).toEqual({ recovered: true });
      },
    });
    engine.addStepMiddleware({
      beforeStep: () => { events.push('before:inner'); },
      onStepError: (_name, _state, error) => {
        events.push('error:inner');
        expect(error).toBeInstanceOf(ModuleTimeoutError);
        return { recovered: true };
      },
      afterStep: () => { events.push('after:inner'); },
    });
    const [output, trace] = await engine.run(new ExecutionStrategy('s', [stalledStep()]), makeContext());
    expect(output).toEqual({ recovered: true });
    expect(trace.success).toBe(true);
    expect(trace.steps[0].skipReason).toBe('error_recovered');
    expect(events).toEqual(['before:outer', 'before:inner', 'error:inner', 'after:inner', 'after:outer']);
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine: decisionPoint tracking
// ---------------------------------------------------------------------------

describe('PipelineEngine.run - decisionPoint', () => {
  it('marks step as decisionPoint when confidence is non-null', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('decision', { confidence: 0.95 })]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.steps[0].decisionPoint).toBe(true);
  });

  it('does not mark step as decisionPoint when confidence is null', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('normal')]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.steps[0].decisionPoint).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

describe('Pipeline error types', () => {
  it('PipelineAbortError has correct code and name', () => {
    const err = new PipelineAbortError('my-step', 'something went wrong');
    expect(err.code).toBe('PIPELINE_ABORT');
    expect(err.name).toBe('PipelineAbortError');
    expect(err.step).toBe('my-step');
    expect(err.explanation).toBe('something went wrong');
  });

  it('PipelineAbortError stores alternatives', () => {
    const err = new PipelineAbortError('step', null, ['alt1', 'alt2']);
    expect(err.alternatives).toEqual(['alt1', 'alt2']);
  });

  it('StepNotFoundError has correct code and name', () => {
    const err = new StepNotFoundError('Step x not found');
    expect(err.code).toBe('STEP_NOT_FOUND');
    expect(err.name).toBe('StepNotFoundError');
  });

  it('StepNotRemovableError has correct code', () => {
    const err = new StepNotRemovableError("Step 'x' is not removable");
    expect(err.code).toBe('STEP_NOT_REMOVABLE');
  });

  it('StepNotReplaceableError has correct code', () => {
    const err = new StepNotReplaceableError("Step 'x' is not replaceable");
    expect(err.code).toBe('STEP_NOT_REPLACEABLE');
  });

  it('StepNameDuplicateError has correct code', () => {
    const err = new StepNameDuplicateError('Duplicate: x');
    expect(err.code).toBe('STEP_NAME_DUPLICATE');
  });

  it('StrategyNotFoundError has correct code and name', () => {
    const err = new StrategyNotFoundError('Strategy not found');
    expect(err.code).toBe('STRATEGY_NOT_FOUND');
    expect(err.name).toBe('StrategyNotFoundError');
  });

  it('PipelineStepError has correct code, name, stepName', () => {
    const cause = new Error('step broke');
    const err = new PipelineStepError('my_step', cause, null);
    expect(err.code).toBe('PIPELINE_STEP_ERROR');
    expect(err.name).toBe('PipelineStepError');
    expect(err.stepName).toBe('my_step');
    expect(err.cause).toBe(cause);
    expect(err.pipelineTrace).toBeNull();
  });

  it('PipelineStepNotFoundError has correct code, name, stepName', () => {
    const err = new PipelineStepNotFoundError('missing_step');
    expect(err.code).toBe('PIPELINE_STEP_NOT_FOUND');
    expect(err.name).toBe('PipelineStepNotFoundError');
    expect(err.stepName).toBe('missing_step');
  });
});

// ---------------------------------------------------------------------------
// Pipeline Hardening (Issue #33)
// ---------------------------------------------------------------------------

describe('Pipeline Hardening §1.1 — Fail-fast', () => {
  it('wraps step exception in PipelineStepError (not raw rethrow)', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('bad', { throws: true })]);
    await expect(engine.run(s, ctx)).rejects.toBeInstanceOf(PipelineStepError);
  });

  it('PipelineStepError carries step name and cause', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('failing_step', { throws: true })]);
    try {
      await engine.run(s, ctx);
    } catch (e) {
      expect(e).toBeInstanceOf(PipelineStepError);
      const err = e as PipelineStepError;
      expect(err.stepName).toBe('failing_step');
      expect(err.cause).toBeInstanceOf(Error);
      expect((err.cause as Error).message).toContain('failing_step');
    }
  });

  it('PipelineStepError carries pipeline trace', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b', { throws: true })]);
    try {
      await engine.run(s, ctx);
    } catch (e) {
      const err = e as PipelineStepError;
      expect(err.pipelineTrace).not.toBeNull();
      expect(err.pipelineTrace!.steps).toHaveLength(2);
    }
  });

  it('stops pipeline on error and does not run subsequent steps', async () => {
    const executed: string[] = [];
    const trackStep = (name: string, throws = false): Step => ({
      name,
      description: name,
      removable: true,
      replaceable: true,
      execute: async (): Promise<StepResult> => {
        executed.push(name);
        if (throws) throw new Error(`${name} failed`);
        return { action: 'continue' };
      },
    });
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [trackStep('a'), trackStep('b', true), trackStep('c')]);
    await expect(engine.run(s, ctx)).rejects.toBeInstanceOf(PipelineStepError);
    expect(executed).toEqual(['a', 'b']);
  });

  it('does NOT throw PipelineStepError when ignoreErrors=true', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const s = new ExecutionStrategy('s', [
      makeStep('risky', { throws: true, ignoreErrors: true }),
      makeStep('after'),
    ]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.success).toBe(true);
  });
});

describe('Pipeline Hardening §1.2 — Replace Semantic (configureStep)', () => {
  it('configureStep replaces existing step at same position', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b'), makeStep('c')]);
    const newB = makeStep('b-v2');
    s.configureStep('b', newB);
    expect(s.stepNames()).toEqual(['a', 'b-v2', 'c']);
  });

  it('configureStep called twice leaves exactly one step at that position', () => {
    const s = new ExecutionStrategy('s', [makeStep('validate_input')]);
    s.configureStep('validate_input', makeStep('validate_input'));
    s.configureStep('validate_input', makeStep('validate_input'));
    expect(s.steps.filter((st) => st.name === 'validate_input')).toHaveLength(1);
    expect(s.steps).toHaveLength(1);
  });

  it('configureStep throws PipelineStepNotFoundError for unknown step', () => {
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    expect(() => s.configureStep('nonexistent', makeStep('x'))).toThrow(PipelineStepNotFoundError);
  });

  it('configureStep throws StepNotReplaceableError for non-replaceable step', () => {
    const s = new ExecutionStrategy('s', [makeStep('locked', { replaceable: false })]);
    expect(() => s.configureStep('locked', makeStep('new'))).toThrow(StepNotReplaceableError);
  });

  it('configureStep throws StepNameDuplicateError when newStep name collides with a different existing step', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b')]);
    // Replacing 'a' with a step named 'b' would create a duplicate
    expect(() => s.configureStep('a', makeStep('b'))).toThrow(StepNameDuplicateError);
    // _steps and _nameToIdx must remain intact after the rejected call
    expect(s.stepNames()).toEqual(['a', 'b']);
    expect(s.findStepIndex('a')).toBe(0);
    expect(s.findStepIndex('b')).toBe(1);
  });

  it('configureStep index map stays consistent after replace with different name', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b'), makeStep('c')]);
    s.configureStep('b', makeStep('b-new'));
    // Old name 'b' must be gone from the index
    expect(s.findStepIndex('b')).toBeUndefined();
    // New name 'b-new' must occupy position 1
    expect(s.findStepIndex('b-new')).toBe(1);
    // Other steps unchanged
    expect(s.findStepIndex('a')).toBe(0);
    expect(s.findStepIndex('c')).toBe(2);
  });
});

describe('Pipeline Hardening §1.4 — run_until', () => {
  it('halts pipeline after step where predicate returns true', async () => {
    const executed: string[] = [];
    const trackStep = (name: string): Step => ({
      name,
      description: name,
      removable: true,
      replaceable: true,
      execute: async (): Promise<StepResult> => {
        executed.push(name);
        return { action: 'continue' };
      },
    });
    const engine = new PipelineEngine();
    const ctx = makeContext();
    ctx.runUntil = (state: PipelineState) => state.stepName === 'b';
    const s = new ExecutionStrategy('s', [trackStep('a'), trackStep('b'), trackStep('c')]);
    const [, trace] = await engine.run(s, ctx);
    expect(trace.success).toBe(true);
    expect(executed).toEqual(['a', 'b']);
  });

  it('returns accumulated output when run_until fires', async () => {
    const engine = new PipelineEngine();
    const ctx = makeContext();
    ctx.output = { value: 42 };
    ctx.runUntil = (state: PipelineState) => state.stepName === 'first';
    const s = new ExecutionStrategy('s', [makeStep('first'), makeStep('second')]);
    const [output, trace] = await engine.run(s, ctx);
    expect(output).toEqual({ value: 42 });
    expect(trace.success).toBe(true);
  });

  it('runs full pipeline when run_until never fires', async () => {
    const executed: string[] = [];
    const trackStep = (name: string): Step => ({
      name,
      description: name,
      removable: true,
      replaceable: true,
      execute: async (): Promise<StepResult> => {
        executed.push(name);
        return { action: 'continue' };
      },
    });
    const engine = new PipelineEngine();
    const ctx = makeContext();
    ctx.runUntil = () => false;
    const s = new ExecutionStrategy('s', [trackStep('a'), trackStep('b'), trackStep('c')]);
    await engine.run(s, ctx);
    expect(executed).toEqual(['a', 'b', 'c']);
  });
});

describe('Pipeline Hardening §1.5 — O(1) step lookup', () => {
  it('findStepIndex returns correct index for each step', () => {
    const s = new ExecutionStrategy('s', [makeStep('x'), makeStep('y'), makeStep('z')]);
    expect(s.findStepIndex('x')).toBe(0);
    expect(s.findStepIndex('y')).toBe(1);
    expect(s.findStepIndex('z')).toBe(2);
  });

  it('findStepIndex returns undefined for unknown step', () => {
    const s = new ExecutionStrategy('s', [makeStep('a')]);
    expect(s.findStepIndex('missing')).toBeUndefined();
  });

  it('findStepIndex updates correctly after insertAfter', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('c')]);
    s.insertAfter('a', makeStep('b'));
    expect(s.findStepIndex('a')).toBe(0);
    expect(s.findStepIndex('b')).toBe(1);
    expect(s.findStepIndex('c')).toBe(2);
  });

  it('findStepIndex updates correctly after remove', () => {
    const s = new ExecutionStrategy('s', [makeStep('a'), makeStep('b'), makeStep('c')]);
    s.remove('b');
    expect(s.findStepIndex('a')).toBe(0);
    expect(s.findStepIndex('c')).toBe(1);
    expect(s.findStepIndex('b')).toBeUndefined();
  });

  it('PipelineEngine uses O(1) lookup for skip_to (findStepIndex)', async () => {
    const executed: string[] = [];
    const engine = new PipelineEngine();
    const ctx = makeContext();
    const steps: Step[] = [
      {
        name: 'a',
        description: 'a',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('a');
          return { action: 'skip_to', skipTo: 'c' };
        },
      },
      {
        name: 'b',
        description: 'b',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('b');
          return { action: 'continue' };
        },
      },
      {
        name: 'c',
        description: 'c',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('c');
          return { action: 'continue' };
        },
      },
    ];
    const s = new ExecutionStrategy('s', steps);
    await engine.run(s, ctx);
    expect(executed).toEqual(['a', 'c']);
  });
});
