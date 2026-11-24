/**
 * Tests for pipeline types: Step, StepResult, PipelineContext, ExecutionStrategy, errors.
 */

import { describe, it, expect } from 'vitest';
import {
  ExecutionStrategy,
  PipelineEngine,
  PipelineAbortError,
  PipelineStepError,
  StepNotFoundError,
  StepNotRemovableError,
  StepNotReplaceableError,
  StepNameDuplicateError,
  StrategyNotFoundError,
  ModuleError,
} from '../src/index.js';
import type {
  Step,
  StepResult,
  PipelineContext,
  StepTrace,
  PipelineTrace,
  StrategyInfo,
} from '../src/index.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeStep(
  name: string,
  opts: { removable?: boolean; replaceable?: boolean } = {},
): Step {
  return {
    name,
    description: `Step ${name}`,
    removable: opts.removable ?? true,
    replaceable: opts.replaceable ?? true,
    execute: async (): Promise<StepResult> => ({ action: 'continue' }),
  };
}

function makeStrategy(
  name: string = 'test',
  steps?: Step[],
): ExecutionStrategy {
  return new ExecutionStrategy(
    name,
    steps ?? [makeStep('a'), makeStep('b'), makeStep('c')],
  );
}

// ---------------------------------------------------------------------------
// StepResult interface shape
// ---------------------------------------------------------------------------

describe('StepResult', () => {
  it('supports continue action with no optional fields', () => {
    const result: StepResult = { action: 'continue' };
    expect(result.action).toBe('continue');
    expect(result.skipTo).toBeUndefined();
  });

  it('supports skip_to action with target', () => {
    const result: StepResult = { action: 'skip_to', skipTo: 'execute' };
    expect(result.action).toBe('skip_to');
    expect(result.skipTo).toBe('execute');
  });

  it('supports abort action with explanation and alternatives', () => {
    const result: StepResult = {
      action: 'abort',
      explanation: 'denied',
      alternatives: ['mod_b'],
      confidence: 0.95,
    };
    expect(result.action).toBe('abort');
    expect(result.explanation).toBe('denied');
    expect(result.alternatives).toEqual(['mod_b']);
    expect(result.confidence).toBe(0.95);
  });
});

// ---------------------------------------------------------------------------
// ExecutionStrategy
// ---------------------------------------------------------------------------

describe('ExecutionStrategy', () => {
  it('creates a strategy with correct name and steps', () => {
    const strategy = makeStrategy('default');
    expect(strategy.name).toBe('default');
    expect(strategy.steps).toHaveLength(3);
    expect(strategy.stepNames()).toEqual(['a', 'b', 'c']);
  });

  it('returns a readonly steps array', () => {
    const strategy = makeStrategy();
    const steps = strategy.steps;
    // readonly prevents push at compile time; verify steps is frozen-like
    expect(Array.isArray(steps)).toBe(true);
    expect(steps).toHaveLength(3);
  });

  it('rejects duplicate step names in constructor', () => {
    expect(
      () => new ExecutionStrategy('dup', [makeStep('a'), makeStep('a')]),
    ).toThrow(StepNameDuplicateError);
  });

  describe('insertAfter', () => {
    it('inserts a step after the anchor', () => {
      const strategy = makeStrategy();
      strategy.insertAfter('a', makeStep('x'));
      expect(strategy.stepNames()).toEqual(['a', 'x', 'b', 'c']);
    });

    it('inserts after the last step', () => {
      const strategy = makeStrategy();
      strategy.insertAfter('c', makeStep('z'));
      expect(strategy.stepNames()).toEqual(['a', 'b', 'c', 'z']);
    });

    it('throws StepNotFoundError when anchor does not exist', () => {
      const strategy = makeStrategy();
      expect(() => strategy.insertAfter('missing', makeStep('x'))).toThrow(
        StepNotFoundError,
      );
    });

    it('throws StepNameDuplicateError when step name already exists', () => {
      const strategy = makeStrategy();
      expect(() => strategy.insertAfter('a', makeStep('b'))).toThrow(
        StepNameDuplicateError,
      );
    });
  });

  describe('insertBefore', () => {
    it('inserts a step before the anchor', () => {
      const strategy = makeStrategy();
      strategy.insertBefore('b', makeStep('x'));
      expect(strategy.stepNames()).toEqual(['a', 'x', 'b', 'c']);
    });

    it('inserts before the first step', () => {
      const strategy = makeStrategy();
      strategy.insertBefore('a', makeStep('z'));
      expect(strategy.stepNames()).toEqual(['z', 'a', 'b', 'c']);
    });

    it('throws StepNotFoundError when anchor does not exist', () => {
      const strategy = makeStrategy();
      expect(() => strategy.insertBefore('missing', makeStep('x'))).toThrow(
        StepNotFoundError,
      );
    });

    it('throws StepNameDuplicateError when step name already exists', () => {
      const strategy = makeStrategy();
      expect(() => strategy.insertBefore('a', makeStep('c'))).toThrow(
        StepNameDuplicateError,
      );
    });
  });

  describe('remove', () => {
    it('removes a removable step', () => {
      const strategy = makeStrategy();
      strategy.remove('b');
      expect(strategy.stepNames()).toEqual(['a', 'c']);
    });

    it('throws StepNotRemovableError for non-removable step', () => {
      const strategy = new ExecutionStrategy('locked', [
        makeStep('core', { removable: false }),
        makeStep('opt'),
      ]);
      expect(() => strategy.remove('core')).toThrow(StepNotRemovableError);
    });

    it('throws StepNotFoundError when step does not exist', () => {
      const strategy = makeStrategy();
      expect(() => strategy.remove('missing')).toThrow(StepNotFoundError);
    });
  });

  describe('replace', () => {
    it('replaces a replaceable step', () => {
      const strategy = makeStrategy();
      const replacement = makeStep('b');
      (replacement as { description: string }).description = 'New B';
      strategy.replace('b', replacement);
      expect(strategy.steps[1].description).toBe('New B');
    });

    it('throws StepNotReplaceableError for non-replaceable step', () => {
      const strategy = new ExecutionStrategy('locked', [
        makeStep('core', { replaceable: false }),
        makeStep('opt'),
      ]);
      expect(() => strategy.replace('core', makeStep('core'))).toThrow(
        StepNotReplaceableError,
      );
    });

    it('throws StepNotFoundError when step does not exist', () => {
      const strategy = makeStrategy();
      expect(() => strategy.replace('missing', makeStep('x'))).toThrow(
        StepNotFoundError,
      );
    });
  });

  describe('info', () => {
    it('returns correct StrategyInfo', () => {
      const strategy = makeStrategy('myStrategy');
      const info: StrategyInfo = strategy.info();
      expect(info.name).toBe('myStrategy');
      expect(info.stepCount).toBe(3);
      expect(info.stepNames).toEqual(['a', 'b', 'c']);
      expect(info.description).toBe('a \u2192 b \u2192 c');
    });

    it('returns empty description for empty strategy', () => {
      const strategy = new ExecutionStrategy('empty', []);
      const info = strategy.info();
      expect(info.stepCount).toBe(0);
      expect(info.stepNames).toEqual([]);
      expect(info.description).toBe('');
    });
  });
});

// ---------------------------------------------------------------------------
// Step execute contract
// ---------------------------------------------------------------------------

describe('Step execute', () => {
  it('execute returns a StepResult', async () => {
    const step = makeStep('test_step');
    const result = await step.execute({} as PipelineContext);
    expect(result.action).toBe('continue');
  });
});

// ---------------------------------------------------------------------------
// PipelineTrace and StepTrace shapes
// ---------------------------------------------------------------------------

describe('PipelineTrace', () => {
  it('satisfies the interface shape', () => {
    const stepTrace: StepTrace = {
      name: 'acl_check',
      durationMs: 1.5,
      result: { action: 'continue', explanation: 'ACL passed' },
      skipped: false,
      decisionPoint: false,
    };

    const trace: PipelineTrace = {
      moduleId: 'my.module',
      strategyName: 'default',
      steps: [stepTrace],
      totalDurationMs: 10.2,
      success: true,
    };

    expect(trace.moduleId).toBe('my.module');
    expect(trace.steps).toHaveLength(1);
    expect(trace.steps[0].name).toBe('acl_check');
    expect(trace.success).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

describe('PipelineAbortError', () => {
  it('extends ModuleError', () => {
    const err = new PipelineAbortError('acl_check', 'Access denied');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err).toBeInstanceOf(PipelineAbortError);
  });

  it('has correct code and message', () => {
    const err = new PipelineAbortError('acl_check', 'Access denied', [
      'alt_module',
    ]);
    expect(err.code).toBe('PIPELINE_ABORT');
    expect(err.message).toContain('acl_check');
    expect(err.message).toContain('Access denied');
    expect(err.step).toBe('acl_check');
    expect(err.explanation).toBe('Access denied');
    expect(err.alternatives).toEqual(['alt_module']);
  });

  it('has default null fields', () => {
    const err = new PipelineAbortError('step1');
    expect(err.explanation).toBeNull();
    expect(err.alternatives).toBeNull();
    expect(err.pipelineTrace).toBeNull();
  });

  it('carries pipeline trace', () => {
    const trace: PipelineTrace = {
      moduleId: 'mod',
      strategyName: 'default',
      steps: [],
      totalDurationMs: 0,
      success: false,
    };
    const err = new PipelineAbortError('step1', 'fail', null, trace);
    expect(err.pipelineTrace).toBe(trace);
  });

  it('has DEFAULT_RETRYABLE set to false', () => {
    expect(PipelineAbortError.DEFAULT_RETRYABLE).toBe(false);
  });
});

describe('StepNotFoundError', () => {
  it('extends ModuleError with correct code', () => {
    const err = new StepNotFoundError('Step x not found');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('STEP_NOT_FOUND');
    expect(err.message).toBe('Step x not found');
  });
});

describe('StepNotRemovableError', () => {
  it('extends ModuleError with correct code', () => {
    const err = new StepNotRemovableError('cannot remove');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('STEP_NOT_REMOVABLE');
  });
});

describe('StepNotReplaceableError', () => {
  it('extends ModuleError with correct code', () => {
    const err = new StepNotReplaceableError('cannot replace');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('STEP_NOT_REPLACEABLE');
  });

  it('passes options to parent constructor', () => {
    const cause = new Error('root');
    const err = new StepNotReplaceableError('no replace', {
      cause,
      traceId: 'trace-r1',
      retryable: false,
      aiGuidance: 'use a different step',
      userFixable: true,
      suggestion: 'replace with allowed step',
    });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe('trace-r1');
  });
});

describe('StepNameDuplicateError', () => {
  it('extends ModuleError with correct code', () => {
    const err = new StepNameDuplicateError('dup name');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('STEP_NAME_DUPLICATE');
  });

  it('passes options to parent constructor', () => {
    const cause = new Error('root');
    const err = new StepNameDuplicateError('duplicate', {
      cause,
      traceId: 'trace-d1',
      retryable: false,
      aiGuidance: 'rename the step',
      userFixable: true,
      suggestion: 'use unique step names',
    });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe('trace-d1');
  });
});

describe('StrategyNotFoundError', () => {
  it('extends ModuleError with correct code', () => {
    const err = new StrategyNotFoundError('no strategy');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('STRATEGY_NOT_FOUND');
  });

  it('passes options to parent constructor', () => {
    const cause = new Error('root');
    const err = new StrategyNotFoundError('missing', {
      cause,
      traceId: 'trace-s1',
      retryable: false,
      aiGuidance: 'check strategy name',
      userFixable: true,
      suggestion: 'use a valid strategy',
    });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe('trace-s1');
  });
});

// ---------------------------------------------------------------------------
// PipelineEngine
// ---------------------------------------------------------------------------

function makePipelineContext(moduleId: string = 'test.mod'): PipelineContext {
  return {
    moduleId,
    inputs: {},
    context: {} as any,
  };
}

function makeStepWithResult(
  name: string,
  result: StepResult,
): Step {
  return {
    name,
    description: `Step ${name}`,
    removable: true,
    replaceable: true,
    execute: async (): Promise<StepResult> => result,
  };
}

describe('PipelineEngine', () => {
  it('runs all steps in order and returns success trace', async () => {
    const executed: string[] = [];
    const steps: Step[] = ['s1', 's2', 's3'].map((n) => ({
      name: n,
      description: `Step ${n}`,
      removable: true,
      replaceable: true,
      execute: async (ctx: PipelineContext): Promise<StepResult> => {
        executed.push(n);
        if (n === 's3') ctx.output = { value: 42 };
        return { action: 'continue' };
      },
    }));
    const strategy = new ExecutionStrategy('default', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    const [output, trace] = await engine.run(strategy, ctx);

    expect(executed).toEqual(['s1', 's2', 's3']);
    expect(trace.success).toBe(true);
    expect(trace.steps).toHaveLength(3);
    expect(trace.strategyName).toBe('default');
    expect((output as Record<string, unknown>).value).toBe(42);
  });

  it('handles skip_to by jumping to the target step', async () => {
    const executed: string[] = [];
    const steps: Step[] = [
      {
        name: 'first',
        description: 'First',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('first');
          return { action: 'skip_to', skipTo: 'last' };
        },
      },
      {
        name: 'middle',
        description: 'Middle',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('middle');
          return { action: 'continue' };
        },
      },
      {
        name: 'last',
        description: 'Last',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('last');
          return { action: 'continue' };
        },
      },
    ];
    const strategy = new ExecutionStrategy('skip', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    const [, trace] = await engine.run(strategy, ctx);

    expect(executed).toEqual(['first', 'last']);
    expect(trace.success).toBe(true);
    // trace should contain: first (executed), middle (skipped), last (executed)
    expect(trace.steps).toHaveLength(3);
    expect(trace.steps[0].skipped).toBe(false);
    expect(trace.steps[1].skipped).toBe(true);
    expect(trace.steps[1].name).toBe('middle');
    expect(trace.steps[2].skipped).toBe(false);
  });

  it('throws PipelineAbortError when a step aborts', async () => {
    const steps: Step[] = [
      makeStepWithResult('ok', { action: 'continue' }),
      makeStepWithResult('fail', {
        action: 'abort',
        explanation: 'denied',
        alternatives: ['alt_mod'],
      }),
      makeStepWithResult('never', { action: 'continue' }),
    ];
    const strategy = new ExecutionStrategy('abort_test', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    await expect(engine.run(strategy, ctx)).rejects.toThrow(PipelineAbortError);
    try {
      await engine.run(strategy, makePipelineContext());
    } catch (e) {
      const err = e as PipelineAbortError;
      expect(err.step).toBe('fail');
      expect(err.explanation).toBe('denied');
      expect(err.alternatives).toEqual(['alt_mod']);
      expect(err.pipelineTrace).toBeDefined();
      expect(err.pipelineTrace!.success).toBe(false);
    }
  });

  it('throws StepNotFoundError when skip_to target does not exist', async () => {
    const steps: Step[] = [
      makeStepWithResult('s1', { action: 'skip_to', skipTo: 'nonexistent' }),
      makeStepWithResult('s2', { action: 'continue' }),
    ];
    const strategy = new ExecutionStrategy('bad_skip', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    await expect(engine.run(strategy, ctx)).rejects.toThrow(StepNotFoundError);
  });

  it('accumulates trace with correct timing for each step', async () => {
    const steps: Step[] = [
      makeStepWithResult('fast', { action: 'continue' }),
      makeStepWithResult('also_fast', { action: 'continue' }),
    ];
    const strategy = new ExecutionStrategy('timing', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    const [, trace] = await engine.run(strategy, ctx);

    expect(trace.steps).toHaveLength(2);
    for (const st of trace.steps) {
      expect(st.durationMs).toBeGreaterThanOrEqual(0);
      expect(st.name).toBeTruthy();
    }
    expect(trace.totalDurationMs).toBeGreaterThanOrEqual(0);
    expect(trace.moduleId).toBe('test.mod');
  });

  it('skips step when matchModules does not match the current moduleId', async () => {
    const executed: string[] = [];
    const steps: Step[] = [
      {
        name: 'filtered',
        description: 'Filtered step',
        removable: true,
        replaceable: true,
        matchModules: ['other.module'],
        execute: async (): Promise<StepResult> => {
          executed.push('filtered');
          return { action: 'continue' };
        },
      },
      {
        name: 'always',
        description: 'Always runs',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('always');
          return { action: 'continue' };
        },
      },
    ];
    const strategy = new ExecutionStrategy('match_filter', steps);
    const ctx = makePipelineContext('test.mod');
    const engine = new PipelineEngine();

    const [, trace] = await engine.run(strategy, ctx);

    expect(executed).toEqual(['always']);
    expect(trace.steps[0].skipped).toBe(true);
    expect(trace.steps[0].skipReason).toBe('no_match');
    expect(trace.steps[1].skipped).toBe(false);
  });

  it('runs step when matchModules matches the current moduleId', async () => {
    const executed: string[] = [];
    const steps: Step[] = [
      {
        name: 'matched',
        description: 'Matches',
        removable: true,
        replaceable: true,
        matchModules: ['test.mod'],
        execute: async (): Promise<StepResult> => {
          executed.push('matched');
          return { action: 'continue' };
        },
      },
    ];
    const strategy = new ExecutionStrategy('match_hit', steps);
    const ctx = makePipelineContext('test.mod');
    const engine = new PipelineEngine();

    await engine.run(strategy, ctx);

    expect(executed).toEqual(['matched']);
  });

  it('skips non-pure steps in dry_run mode', async () => {
    const executed: string[] = [];
    const steps: Step[] = [
      {
        name: 'impure',
        description: 'Has side effects',
        removable: true,
        replaceable: true,
        pure: false,
        execute: async (): Promise<StepResult> => {
          executed.push('impure');
          return { action: 'continue' };
        },
      },
      {
        name: 'pure',
        description: 'Pure read-only',
        removable: true,
        replaceable: true,
        pure: true,
        execute: async (): Promise<StepResult> => {
          executed.push('pure');
          return { action: 'continue' };
        },
      },
    ];
    const strategy = new ExecutionStrategy('dry_run_test', steps);
    const ctx: PipelineContext = { ...makePipelineContext(), dryRun: true };
    const engine = new PipelineEngine();

    const [, trace] = await engine.run(strategy, ctx);

    expect(executed).toEqual(['pure']);
    expect(trace.steps[0].skipped).toBe(true);
    expect(trace.steps[0].skipReason).toBe('dry_run');
    expect(trace.steps[1].skipped).toBe(false);
  });

  it('continues execution when ignoreErrors is true on a failing step', async () => {
    const executed: string[] = [];
    const steps: Step[] = [
      {
        name: 'failing',
        description: 'Throws but ignored',
        removable: true,
        replaceable: true,
        ignoreErrors: true,
        execute: async (): Promise<StepResult> => {
          throw new Error('step failed');
        },
      },
      {
        name: 'after',
        description: 'Runs after failure',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          executed.push('after');
          return { action: 'continue' };
        },
      },
    ];
    const strategy = new ExecutionStrategy('ignore_err', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    const [, trace] = await engine.run(strategy, ctx);

    expect(executed).toEqual(['after']);
    expect(trace.success).toBe(true);
    expect(trace.steps[0].skipReason).toBe('error_ignored');
    expect(trace.steps[0].result.explanation).toContain('step failed');
  });

  it('records String(exc) when ignoreErrors throws a non-Error value', async () => {
    const steps: Step[] = [
      {
        name: 'throws_string',
        description: 'Throws a string, not an Error',
        removable: true,
        replaceable: true,
        ignoreErrors: true,
        execute: async (): Promise<StepResult> => {
          // eslint-disable-next-line no-throw-literal
          throw 'raw string thrown';
        },
      },
    ];
    const strategy = new ExecutionStrategy('non_error_throw', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    const [, trace] = await engine.run(strategy, ctx);
    expect(trace.steps[0].result.explanation).toBe('raw string thrown');
  });

  it('records String(exc) in abort trace when a non-Error is thrown without ignoreErrors', async () => {
    const steps: Step[] = [
      {
        name: 'throws_string',
        description: 'Throws a string',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => {
          // eslint-disable-next-line no-throw-literal
          throw 'direct string error';
        },
      },
    ];
    const strategy = new ExecutionStrategy('non_error_abort', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    // Per §1.1, non-Error throws are wrapped in PipelineStepError (not propagated raw).
    await expect(engine.run(strategy, ctx)).rejects.toBeInstanceOf(PipelineStepError);
  });

  it('abort step without explanation or alternatives uses null defaults', async () => {
    const steps: Step[] = [
      makeStepWithResult('abort', { action: 'abort' }),
    ];
    const strategy = new ExecutionStrategy('no_explanation_abort', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    try {
      await engine.run(strategy, ctx);
    } catch (e) {
      const err = e as PipelineAbortError;
      expect(err.explanation).toBeNull();
      expect(err.alternatives).toBeNull();
    }
  });

  it('per-step timeout aborts the step when exceeded', async () => {
    const steps: Step[] = [
      {
        name: 'slow',
        description: 'Takes too long',
        removable: true,
        replaceable: true,
        timeoutMs: 20,
        execute: async (): Promise<StepResult> => {
          await new Promise<void>((r) => setTimeout(r, 200));
          return { action: 'continue' };
        },
      },
    ];
    const strategy = new ExecutionStrategy('timeout_test', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    await expect(engine.run(strategy, ctx)).rejects.toThrow("timed out");
  });

  it('per-step timeout clears timer when step completes within timeout', async () => {
    const steps: Step[] = [
      {
        name: 'fast_with_timeout',
        description: 'Completes before timeout',
        removable: true,
        replaceable: true,
        timeoutMs: 5000,
        execute: async (): Promise<StepResult> => ({ action: 'continue' }),
      },
    ];
    const strategy = new ExecutionStrategy('fast_timeout', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    const [, trace] = await engine.run(strategy, ctx);
    expect(trace.success).toBe(true);
  });

  it('throws StepNotFoundError for unknown step action', async () => {
    const steps: Step[] = [
      {
        name: 'bad_action',
        description: 'Returns unknown action',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => ({ action: 'unknown_action' as StepResult['action'] }),
      },
    ];
    const strategy = new ExecutionStrategy('bad_action_test', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    await expect(engine.run(strategy, ctx)).rejects.toThrow(StepNotFoundError);
  });

  it('skip_to with undefined skipTo falls back to empty string target', async () => {
    const steps: Step[] = [
      {
        name: 'skipper',
        description: 'Skips without target',
        removable: true,
        replaceable: true,
        execute: async (): Promise<StepResult> => ({ action: 'skip_to' }),
      },
      makeStepWithResult('next', { action: 'continue' }),
    ];
    const strategy = new ExecutionStrategy('skip_no_target', steps);
    const ctx = makePipelineContext();
    const engine = new PipelineEngine();

    // skipTo is undefined, target becomes '' which findIndex won't find → throws StepNotFoundError
    await expect(engine.run(strategy, ctx)).rejects.toThrow(StepNotFoundError);
  });
});
