/**
 * Cross-language conformance driver for `pipeline_step_middleware.json`
 * (Issue #33 §2.2 — the StepMiddleware lifecycle).
 *
 * Fixture source: apcore/conformance/fixtures/pipeline_step_middleware.json
 * (canonical). This file LOADS it. `tests/test-pipeline-step-middleware.test.ts`
 * is a hand-written unit test over the same area; a hand copy cannot notice
 * when the canonical fixture gains a case, which is why both exist and why only
 * this one counts as conformance coverage.
 *
 * The fixture's `driver_contract` names the ways this file is easy to write
 * vacuously, and all of them are avoided here:
 *
 *  - `order_is_the_assertion` — asserting the SET of invoked middlewares rather
 *    than the ORDERED list. A set passes against a straight-through
 *    implementation, which is exactly what the onion model exists to rule out.
 *  - `first_recovery_wins` — testing recovery with a single recovering handler.
 *    With one recovery, first-wins and last-wins yield the same value; the case
 *    must have two handlers that would BOTH recover.
 *  - `assert_the_wire_code` / `wrapper_is_load_bearing` — every error
 *    expectation is a WIRE CODE (`wrapper_error_code`, `original_error_code`),
 *    never a class name. Removing the `MiddlewareChainError` wrapping from the
 *    SDK was verified to leave this whole file GREEN before those codes were
 *    asserted; it is the single behaviour the `beforeStep` fix exists to
 *    provide, so it is now pinned.
 *  - `step_output_is_a_MUST` — the recovery value BECOMES the step's output
 *    (features/middleware-system.md → Normative Rules). apcore-typescript used
 *    to treat it as informational and leave `ctx.output` untouched, alone among
 *    the three SDKs; `expected.step_output` is now asserted against `ctx.output`.
 *  - `before_step_failure_is_terminal` — asserting only that an error was
 *    THROWN. An implementation that honours the `beforeStep` recovery and then
 *    happens to fail somewhere later satisfies a throw-only assertion while the
 *    authorization bypass is live, so `recovery_honored: false` is driven by
 *    observing that the FOLLOWING step never executed.
 *  - `two_recovery_paths` — the two recovery paths are deliberate opposites and
 *    MUST NOT share an implementation: a `beforeStep` failure discards the
 *    recovery and fires no `afterStep`; a recovered STEP BODY publishes the
 *    recovery and MUST still close the onion with `afterStep`.
 *  - `assert_the_exact_key_set` — asserting that one key is ABSENT from
 *    `state.outputs`. `not.toContain('second')` also passes against an
 *    implementation that lost `first` and against one that never populated the
 *    map at all, so the exact key set is asserted instead; and the three
 *    expectations differ on purpose, `onStepError` being observed on the THIRD
 *    step so that an all-empty map cannot satisfy them.
 *
 * `beforeStep(stepName, state)` is an OBSERVATION hook. A Step is
 * `execute(ctx)` and takes no inputs argument, so nothing it returns can reach
 * the module — the last case asserts precisely that.
 */

import * as fs from 'node:fs';
import * as path from 'node:path';
import { describe, expect, it } from 'vitest';

import { Context } from '../src/context.js';
import { ModuleError } from '../src/errors.js';
import { ExecutionStrategy, PipelineEngine } from '../src/pipeline.js';
import type {
  PipelineContext,
  PipelineState,
  Step,
  StepMiddleware,
  StepResult,
} from '../src/pipeline.js';
import { findFixturesRoot } from './spec-repo.js';

interface StepMiddlewareCase {
  readonly id: string;
  readonly description: string;
  readonly input: Record<string, unknown>;
  readonly expected: Record<string, unknown>;
}

const fixture: { readonly test_cases: readonly StepMiddlewareCase[] } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'pipeline_step_middleware.json'), 'utf-8'),
);

function caseById(id: string): StepMiddlewareCase {
  const tc = fixture.test_cases.find((c) => c.id === id);
  if (!tc) throw new Error(`pipeline_step_middleware.json no longer carries case '${id}'`);
  return tc;
}

const registerOrder = (tc: StepMiddlewareCase): string[] => tc.input['register_order'] as string[];

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

function okStep(name: string, opts: { ignoreErrors?: boolean } = {}): Step {
  return {
    name,
    description: `Conformance stand-in for ${name}`,
    removable: true,
    replaceable: true,
    ignoreErrors: opts.ignoreErrors ?? false,
    execute: async (): Promise<StepResult> => ({ action: 'continue' }),
  };
}

/**
 * A step that records the fact that it ran.
 *
 * `before_step_failure_is_terminal` requires the driver to prove the discarded
 * recovery by observing that the FOLLOWING step did not execute — asserting
 * only that an error was thrown passes against an implementation that honours
 * the recovery, advances past `acl_check`, and then fails later for an
 * unrelated reason, which is precisely the bypass being ruled out.
 */
function recordingStep(name: string, ran: string[]): Step {
  return {
    name,
    description: `Conformance stand-in that records that it ran`,
    removable: true,
    replaceable: true,
    execute: async (): Promise<StepResult> => {
      ran.push(name);
      return { action: 'continue' };
    },
  };
}

function failingStep(name: string, tc: StepMiddlewareCase): Step {
  const spec = tc.input['step_raises'] as { code: string; message: string };
  return {
    name,
    description: `Conformance stand-in that fails`,
    removable: true,
    replaceable: true,
    execute: async (): Promise<StepResult> => {
      // A typed ModuleError carrying the fixture's own `code`, not a bare
      // Error: `original_error_code` is only observable if the step raises
      // something that HAS a code.
      throw new ModuleError(spec.code, spec.message);
    },
  };
}

function makeContext(inputs: Record<string, unknown> = {}): PipelineContext {
  return {
    moduleId: 'executor.conformance.step_mw',
    inputs,
    context: new Context('trace-id', null, []),
  } as PipelineContext;
}

/**
 * Records each hook labelled with the middleware's own name, so assertions are
 * on ORDER rather than on membership.
 */
function recorder(
  label: string,
  log: string[],
  opts: {
    recovery?: unknown;
    beforeFails?: boolean;
    returns?: unknown;
    /**
     * Confine every hook to a single step name.
     *
     * Multi-step strategies NEED this. A middleware whose `beforeStep` throws
     * unconditionally throws on the FOLLOWING step too, so that step's body
     * never runs either — and "the following step did not execute" would then
     * hold even against an implementation that honours the `beforeStep`
     * recovery and sails past `acl_check`. The assertion the driver_contract
     * calls load-bearing would be vacuous at exactly the point it matters. This
     * was observed: with the recovery honoured, the unscoped recorder still
     * left the following step unexecuted.
     */
    onlyStep?: string;
  } = {},
): StepMiddleware {
  const inScope = (stepName: string): boolean =>
    opts.onlyStep === undefined || stepName === opts.onlyStep;
  return {
    // Deliberately returns a value, cast past the interface. `beforeStep` is
    // declared `=> void | Promise<void>`, which is itself the type-level
    // statement that its return carries no meaning; the cast lets the test
    // prove the RUNTIME ignores it too, rather than relying on the compiler.
    beforeStep: ((stepName: string): unknown => {
      if (!inScope(stepName)) return undefined;
      log.push(`before:${label}`);
      if (opts.beforeFails) throw new Error(`${label} beforeStep exploded`);
      return opts.returns;
    }) as StepMiddleware['beforeStep'],
    afterStep: (stepName: string) => {
      if (!inScope(stepName)) return;
      log.push(`after:${label}`);
    },
    onStepError: (stepName: string) => {
      if (!inScope(stepName)) return null;
      log.push(`error:${label}`);
      return opts.recovery ?? null;
    },
  };
}

const hooks = (log: string[], prefix: string): string[] =>
  log.filter((e) => e.startsWith(prefix)).map((e) => e.slice(prefix.length));

/** Runs on the CALLER'S engine — the one carrying the registered middlewares. */
async function runQuietly(
  engine: PipelineEngine,
  strategy: ExecutionStrategy,
  ctx: PipelineContext,
): Promise<{ threw: boolean; error: unknown }> {
  try {
    await engine.run(strategy, ctx);
    return { threw: false, error: null };
  } catch (err) {
    return { threw: true, error: err };
  }
}

/** The wire code of a thrown error — the only cross-language error assertion. */
function wireCode(error: unknown): string | undefined {
  return error instanceof ModuleError ? error.code : undefined;
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

describe('Conformance: StepMiddleware lifecycle (pipeline_step_middleware.json)', () => {
  it('before_after_invocation_order', async () => {
    const tc = caseById('before_after_invocation_order');
    const log: string[] = [];
    const engine = new PipelineEngine();
    for (const label of registerOrder(tc)) engine.addStepMiddleware(recorder(label, log));

    await engine.run(
      new ExecutionStrategy('conformance', [okStep(tc.input['step'] as string)]),
      makeContext(),
    );

    expect(hooks(log, 'before:'), 'beforeStep runs in REGISTRATION order').toEqual(
      tc.expected['before_step_order'],
    );
    expect(hooks(log, 'after:'), 'afterStep runs in REVERSE order (onion model)').toEqual(
      tc.expected['after_step_order'],
    );
  });

  it('on_step_error_recovery_short_circuits', async () => {
    const tc = caseById('on_step_error_recovery_short_circuits');
    const log: string[] = [];
    // Two of the three handlers offer a recovery, so this case can genuinely
    // tell first-wins from last-wins.
    const returns = tc.input['on_step_error_returns'] as Record<string, unknown>;
    const engine = new PipelineEngine();
    for (const label of registerOrder(tc)) {
      engine.addStepMiddleware(recorder(label, log, { recovery: returns[label] ?? null }));
    }

    const ctx = makeContext();
    const { threw } = await runQuietly(
      engine,
      new ExecutionStrategy('conformance', [failingStep(tc.input['step'] as string, tc)]),
      ctx,
    );

    expect(
      hooks(log, 'error:'),
      'onStepError runs in REVERSE order and the FIRST recovery short-circuits the rest',
    ).toEqual(tc.expected['on_step_error_invoked']);
    expect(threw, 'a recovered step must not propagate the original error').toBe(
      tc.expected['error_propagated'],
    );
    // step_output_is_a_MUST: the recovery value BECOMES the step's output.
    // This also proves first-recovery-wins on the VALUE and not only on the
    // invocation list — mw_b's recovery must be the one published, never mw_a's.
    expect(ctx.output, 'the recovery value MUST become the step output (Normative Rules)').toEqual(
      tc.expected['step_output'],
    );
  });

  it('on_step_error_null_propagates_error', async () => {
    const tc = caseById('on_step_error_null_propagates_error');
    const log: string[] = [];
    const engine = new PipelineEngine();
    for (const label of registerOrder(tc)) engine.addStepMiddleware(recorder(label, log));

    const { threw, error } = await runQuietly(
      engine,
      new ExecutionStrategy('conformance', [failingStep(tc.input['step'] as string, tc)]),
      makeContext(),
    );

    expect(
      hooks(log, 'error:'),
      'every handler is consulted, in reverse order, when none recovers',
    ).toEqual(tc.expected['on_step_error_invoked']);
    expect(threw).toBe(tc.expected['error_propagated']);
    // Wire codes, never class names: apcore-rust has no PipelineStepError type,
    // only an ErrorCode variant, so a class-name assertion is undriveable there.
    expect(wireCode(error), 'an unrecovered step failure is wrapped').toBe(
      tc.expected['wrapper_error_code'],
    );
    expect(
      wireCode((error as ModuleError).cause),
      'the original error MUST survive inside the wrapper',
    ).toBe(tc.expected['original_error_code']);
  });

  it('on_step_error_only_executed_middlewares', async () => {
    const tc = caseById('on_step_error_only_executed_middlewares');
    const log: string[] = [];
    const failing = tc.input['before_step_raises_in'] as string;
    const engine = new PipelineEngine();
    for (const label of registerOrder(tc)) {
      engine.addStepMiddleware(recorder(label, log, { beforeFails: label === failing }));
    }

    const { threw, error } = await runQuietly(
      engine,
      new ExecutionStrategy('conformance', [okStep(tc.input['step'] as string)]),
      makeContext(),
    );

    expect(
      hooks(log, 'before:'),
      'the chain stops at the middleware whose beforeStep failed',
    ).toEqual(tc.expected['before_step_invoked']);
    expect(
      hooks(log, 'error:'),
      'only middlewares that actually ran may observe the failure, in reverse order',
    ).toEqual(tc.expected['on_step_error_invoked']);
    expect(
      hooks(log, 'after:'),
      'a failed beforeStep must prevent the step body, hence any afterStep',
    ).toEqual([]);
    expect(threw, 'a beforeStep failure must not be swallowed').toBe(true);
    // wrapper_is_load_bearing: this is the one behaviour the beforeStep fix
    // exists to provide, and it went unasserted — removing the wrapping from
    // src/pipeline.ts left this entire file green.
    expect(wireCode(error), 'a throwing beforeStep MUST surface wrapped, not bare').toBe(
      tc.expected['wrapper_error_code'],
    );
  });

  it('async_middleware_awaited', async () => {
    const tc = caseById('async_middleware_awaited');
    const log: string[] = [];
    const beforeMark = tc.input['async_before_step_records'] as string;
    const afterMark = tc.input['async_after_step_records'] as string;

    const engine = new PipelineEngine();
    engine.addStepMiddleware({
      // Each hook yields before recording. An unawaited promise would never
      // resume, so the mark would be missing entirely — which is the only way
      // "was it awaited" is observable when there is no inputs parameter to
      // inspect.
      beforeStep: async () => {
        await Promise.resolve();
        log.push(beforeMark);
      },
      afterStep: async () => {
        await Promise.resolve();
        log.push(afterMark);
      },
    });

    await engine.run(
      new ExecutionStrategy('conformance', [okStep(tc.input['step'] as string)]),
      makeContext(),
    );

    expect(
      log,
      'async StepMiddleware callbacks MUST be awaited before the pipeline advances',
    ).toEqual(tc.expected['recorded_in_order']);
  });

  it('before_step_return_value_is_ignored', async () => {
    const tc = caseById('before_step_return_value_is_ignored');
    const log: string[] = [];
    const returns = tc.input['before_step_returns'] as Record<string, unknown>;
    const engine = new PipelineEngine();
    for (const label of registerOrder(tc)) {
      engine.addStepMiddleware(recorder(label, log, { returns: returns[label] }));
    }

    const ctx = makeContext({ ...(tc.input['original_call_inputs'] as Record<string, unknown>) });
    const { threw } = await runQuietly(
      engine,
      new ExecutionStrategy('conformance', [okStep(tc.input['step'] as string)]),
      ctx,
    );

    expect(
      ctx.inputs,
      'beforeStep is an observation hook: a Step is execute(ctx) and has no inputs ' +
        'parameter, so nothing a middleware returns may reach the module',
    ).toEqual(tc.expected['module_received_inputs']);
    expect(hooks(log, 'before:')).toEqual(tc.expected['before_step_invoked']);
    expect(threw).toBe(tc.expected['error_raised']);
  });

  it('before_step_failure_recovery_is_discarded', async () => {
    const tc = caseById('before_step_failure_recovery_is_discarded');
    const log: string[] = [];
    const ran: string[] = [];
    const failing = tc.input['before_step_raises_in'] as string;
    const returns = tc.input['on_step_error_returns'] as Record<string, unknown>;

    const engine = new PipelineEngine();
    for (const label of registerOrder(tc)) {
      // mw_b's beforeStep throws AND its onStepError offers a recovery. That
      // combination is the whole point: it is the shape a middleware would use
      // to skip a gate it has no authority over.
      engine.addStepMiddleware(
        recorder(label, log, {
          beforeFails: label === failing,
          recovery: returns[label] ?? null,
          // Confined to acl_check so the following step is reachable — see
          // `onlyStep`. Without it the bypass assertion below cannot fail.
          onlyStep: tc.input['step'] as string,
        }),
      );
    }

    // The failing step is `acl_check` — the rule exists to protect the gates —
    // and it carries ignoreErrors: true, taken from the fixture. `execute`
    // follows it and records whether it ran.
    const guardedStep = tc.input['step'] as string;
    const followingStep = tc.input['following_step'] as string;
    const strategy = new ExecutionStrategy('conformance', [
      okStep(guardedStep, { ignoreErrors: tc.input['ignore_errors'] as boolean }),
      recordingStep(followingStep, ran),
    ]);

    const ctx = makeContext();
    const { threw, error } = await runQuietly(engine, strategy, ctx);

    // before_step_failure_is_terminal: `recovery_honored: false` is proven by
    // the FOLLOWING step never running, not by the throw. A throw-only
    // assertion is satisfied by an implementation that honours the recovery,
    // sails past acl_check, and fails later — the live bypass. It is asserted
    // FIRST because it is the security property; everything below is mechanics.
    expect(
      ran.includes(followingStep),
      'honouring the recovery would advance the pipeline past a step whose body ' +
        'never ran — acl_check and approval_gate live in that sequence, so this ' +
        'is a silent authorization bypass',
    ).toBe(tc.expected['following_step_executed']);

    expect(
      hooks(log, 'before:'),
      'the chain stops at the middleware whose beforeStep failed',
    ).toEqual(tc.expected['before_step_invoked']);
    // No short-circuit on this path: there is no recovery to shop for, so every
    // middleware that entered beforeStep gets its cleanup call — including
    // mw_a, which sits BEHIND the mw_b that returned a value.
    expect(
      hooks(log, 'error:'),
      'onStepError runs for cleanup on every already-entered middleware, in reverse order',
    ).toEqual(tc.expected['on_step_error_invoked']);
    expect(
      hooks(log, 'after:'),
      'afterStep MUST NOT fire: no step body ran, so there is nothing to close over',
    ).toEqual(tc.expected['after_step_invoked']);
    // `PipelineContext.output` is optional, so "no output" is `undefined` here
    // and `null` in the fixture; normalising is what makes the two comparable.
    // The assertion still bites: publishing the recovery would put the
    // `{ recovered: true, by: 'mw_b' }` object here.
    expect(ctx.output ?? null, 'a discarded recovery MUST NOT become the step output').toBe(
      tc.expected['step_output'],
    );

    expect(threw, 'a beforeStep failure must not be swallowed').toBe(
      tc.expected['error_propagated'],
    );
    // ignore_errors_applies: false — the step above carries ignoreErrors: true.
    // ignore_errors declares that THIS STEP's failure is tolerable; a broken
    // middleware chain is not a step failure, so MiddlewareChainError
    // propagates regardless (and is not re-wrapped in PipelineStepError).
    expect(
      wireCode(error),
      "the step's ignoreErrors MUST NOT apply — MiddlewareChainError propagates regardless",
    ).toBe(tc.expected['wrapper_error_code']);
  });

  it('after_step_fires_after_a_recovered_step', async () => {
    const tc = caseById('after_step_fires_after_a_recovered_step');
    const log: string[] = [];
    const returns = tc.input['on_step_error_returns'] as Record<string, unknown>;

    const engine = new PipelineEngine();
    for (const label of registerOrder(tc)) {
      engine.addStepMiddleware(recorder(label, log, { recovery: returns[label] ?? null }));
    }

    const ctx = makeContext();
    const { threw } = await runQuietly(
      engine,
      new ExecutionStrategy('conformance', [failingStep(tc.input['step'] as string, tc)]),
      ctx,
    );

    expect(
      hooks(log, 'error:'),
      'the first recovery short-circuits the rest on the STEP-BODY path',
    ).toEqual(tc.expected['on_step_error_invoked']);
    // two_recovery_paths: the deliberate opposite of the case above. A recovered
    // step body PRODUCED an output and the pipeline CONTINUED, so the onion must
    // close — a middleware that acquired something in beforeStep must get its
    // afterStep or the recovery path leaks.
    expect(
      hooks(log, 'after:'),
      'afterStep MUST fire after a recovered step body, in reverse registration order',
    ).toEqual(tc.expected['after_step_invoked']);
    expect(ctx.output, 'on THIS path the recovery value DOES become the step output').toEqual(
      tc.expected['step_output'],
    );
    expect(threw, 'a recovered step body must not propagate the original error').toBe(
      tc.expected['error_propagated'],
    );
  });

  it('state_outputs_excludes_the_current_step_in_every_hook', async () => {
    const tc = caseById('state_outputs_excludes_the_current_step_in_every_hook');
    const stepNames = tc.input['steps'] as string[];
    const outputsByStep = tc.input['step_outputs'] as Record<string, Record<string, unknown>>;
    const observed = tc.input['observe_hooks_on'] as string;
    const raises = tc.input['third_step_raises'] as { code: string; message: string };
    const failingName = stepNames[stepNames.length - 1];

    // Every hook observation, in arrival order. `state.outputs` is a LIVE
    // reference to the engine's map — the same object the engine keeps
    // mutating — so the key set MUST be snapshotted INSIDE the hook. Holding
    // the reference and reading it after `run()` returns would observe the
    // final map on every entry and could not fail.
    const seen: { hook: string; step: string; keys: string[] }[] = [];
    const snapshot = (hook: string, step: string, state: PipelineState): void => {
      seen.push({ hook, step, keys: Object.keys(state.outputs) });
    };

    const engine = new PipelineEngine();
    engine.addStepMiddleware({
      beforeStep: (stepName, state) => snapshot('before', stepName, state),
      afterStep: (stepName, state) => snapshot('after', stepName, state),
      onStepError: (stepName, state) => {
        snapshot('error', stepName, state);
        return null; // no recovery — the failure must reach the caller
      },
    });

    const ctx = makeContext();
    const steps: Step[] = stepNames.map((name) =>
      name === failingName
        ? {
            name,
            description: `Conformance stand-in that fails`,
            removable: true,
            replaceable: true,
            execute: async (): Promise<StepResult> => {
              throw new ModuleError(raises.code, raises.message);
            },
          }
        : {
            name,
            description: `Conformance stand-in that publishes an output`,
            removable: true,
            replaceable: true,
            // The engine snapshots `ctx.output` into `state.outputs` under the
            // step's name, so a step must publish one to appear in the map.
            execute: async (c: PipelineContext): Promise<StepResult> => {
              c.output = { ...outputsByStep[name] };
              return { action: 'continue' };
            },
          },
    );

    const { threw } = await runQuietly(engine, new ExecutionStrategy('conformance', steps), ctx);
    expect(threw, 'the last step fails and nothing recovers, so the error propagates').toBe(true);

    const keysAt = (hook: string, step: string): string[] | undefined =>
      seen.find((e) => e.hook === hook && e.step === step)?.keys;

    // assert_the_exact_key_set: the EXACT key set, never "key X is absent".
    // `expect(keys).not.toContain('second')` also passes against an
    // implementation that lost `first`, and against one that never populated
    // the map at all. Sorting both sides compares the set exactly without
    // pinning JS object insertion order, which is not a cross-language
    // property.
    const exactly = (actual: string[] | undefined, expected: unknown): void => {
      expect([...(actual ?? [])].sort()).toEqual([...(expected as string[])].sort());
    };

    // beforeStep on `second`: `first` has completed, `second` has not run.
    exactly(keysAt('before', observed), tc.expected['outputs_keys_in_before_step']);
    // afterStep on `second`: it SUCCEEDED, and its output is the `result`
    // parameter — carrying the same value down two paths is how the two drift
    // apart, so it MUST NOT also be in the map. This is the assertion the
    // engine's ordering (`_runAfterStepHooks` BEFORE `stepOutputs[...] = …`)
    // exists to satisfy, on both the success and the recovery path.
    exactly(keysAt('after', observed), tc.expected['outputs_keys_in_after_step']);
    // onStepError is observed on the THIRD step on purpose: it proves the map
    // DOES keep earlier steps while excluding the failing one. An all-empty
    // map would satisfy the other two expectations.
    exactly(keysAt('error', failingName), tc.expected['outputs_keys_in_on_step_error']);

    // The rule stated once over every observation rather than three times:
    // whatever hook, whatever step, the step being observed is never its own
    // key. This is what makes the rule ONE rule — a middleware reading
    // `state.outputs` never has to know which hook it is in.
    expect(tc.expected['current_step_never_present']).toBe(true);
    for (const entry of seen) {
      expect(
        entry.keys,
        `${entry.hook}Step('${entry.step}') must not see its own step in state.outputs`,
      ).not.toContain(entry.step);
    }
    // …and the loop above is only meaningful if it actually ran over all three
    // hooks on all three steps: before×3, after×2 (the third never succeeds),
    // error×1.
    expect(
      seen.map((e) => `${e.hook}:${e.step}`),
      'every hook on every step must have been observed',
    ).toEqual([
      `before:${stepNames[0]}`,
      `after:${stepNames[0]}`,
      `before:${stepNames[1]}`,
      `after:${stepNames[1]}`,
      `before:${stepNames[2]}`,
      `error:${stepNames[2]}`,
    ]);
  });

  it('drives every fixture case', () => {
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'before_after_invocation_order',
      'on_step_error_recovery_short_circuits',
      'on_step_error_null_propagates_error',
      'on_step_error_only_executed_middlewares',
      'async_middleware_awaited',
      'before_step_return_value_is_ignored',
      'before_step_failure_recovery_is_discarded',
      'after_step_fires_after_a_recovered_step',
      'state_outputs_excludes_the_current_step_in_every_hook',
    ]);
  });
});
