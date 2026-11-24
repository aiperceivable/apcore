/**
 * Regression tests for D-20 cancellation bypass (A-D-003 / A-D-004) and
 * D-19 versionHint forwarding through callWithTrace (A-D-005).
 *
 * D-20: a cancellation raised mid-pipeline MUST bypass the on_error middleware
 * chain even when the pipeline engine wrapped it in a PipelineStepError. The
 * pre-fix code only short-circuited a *bare* ExecutionCancelledError, so a
 * step-wrapped cancellation fell into executeOnError and could be "recovered".
 */

import { describe, expect, it } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { Middleware } from '../src/middleware/base.js';
import { ExecutionCancelledError } from '../src/cancel.js';
import { ExecutionStrategy, type PipelineContext, type StepResult } from '../src/pipeline.js';

class AlwaysRecoverMiddleware extends Middleware {
  recovered = false;
  override onError(): Record<string, unknown> | null {
    this.recovered = true;
    return { recovered: true };
  }
}

function makeCancellingRegistry(id: string): Registry {
  const reg = new Registry();
  reg.register(id, {
    id,
    description: 'cancels mid-pipeline',
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({}),
    execute: () => {
      throw new ExecutionCancelledError('cancelled mid-pipeline');
    },
  });
  return reg;
}

describe('D-20 cancellation bypasses on_error (A-D-003 call)', () => {
  it('does not trigger onError recovery for a step-wrapped cancellation', async () => {
    const reg = makeCancellingRegistry('test.cancel');
    const mw = new AlwaysRecoverMiddleware();
    const exec = new Executor({ registry: reg });
    exec.use(mw);

    await expect(exec.call('test.cancel', {})).rejects.toBeInstanceOf(ExecutionCancelledError);
    expect(mw.recovered).toBe(false);
  });
});

describe('D-20 cancellation bypasses on_error (A-D-004 stream)', () => {
  it('does not trigger onError recovery for a step-wrapped cancellation in stream', async () => {
    const reg = makeCancellingRegistry('test.cancel.stream');
    const mw = new AlwaysRecoverMiddleware();
    const exec = new Executor({ registry: reg });
    exec.use(mw);

    await expect(async () => {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      for await (const _ of exec.stream('test.cancel.stream', {})) {
        // drain
      }
    }).rejects.toBeInstanceOf(ExecutionCancelledError);
    expect(mw.recovered).toBe(false);
  });
});

describe('D-20 cancellation bypasses on_error (callWithTrace)', () => {
  it('does not trigger onError recovery for a step-wrapped cancellation', async () => {
    const reg = makeCancellingRegistry('test.cancel.trace');
    const mw = new AlwaysRecoverMiddleware();
    const exec = new Executor({ registry: reg });
    exec.use(mw);

    await expect(exec.callWithTrace('test.cancel.trace', {})).rejects.toBeInstanceOf(
      ExecutionCancelledError,
    );
    expect(mw.recovered).toBe(false);
  });
});

describe('callWithTrace forwards versionHint into the pipeline context (A-D-005)', () => {
  it('exposes the versionHint argument on PipelineContext', async () => {
    let captured: string | null | undefined = '__unset__';
    const captureStep = {
      name: 'capture-version-hint',
      description: 'Captures versionHint from context for assertion',
      removable: false,
      replaceable: false,
      execute: async (ctx: PipelineContext): Promise<StepResult> => {
        captured = ctx.versionHint;
        ctx.output = {};
        return { action: 'continue' };
      },
    };
    const strategy = new ExecutionStrategy('capture-only', [captureStep]);
    const exec = new Executor({ registry: new Registry() });

    await exec.callWithTrace('any.module', {}, null, { strategy }, '^1.2.0');
    expect(captured).toBe('^1.2.0');
  });
});
