/**
 * Cross-language sync regressions for the ACL surface.
 *
 * C1 — the executor pipeline must take the ASYNC ACL path so that
 *      `ACL.registerAsyncCondition()` handlers are actually awaited.
 * W2 — the handler-error slot must be per-call, not a shared module-level
 *      static that nested / interleaved evaluations can steal from each other.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { ACL, type AuditEntry } from '../src/acl.js';
import type { ACLConditionHandler } from '../src/acl-handlers.js';
import { Context } from '../src/context.js';
import { BuiltinACLCheck } from '../src/builtin-steps.js';
import type { PipelineContext } from '../src/pipeline.js';

function makePipelineContext(
  callerId: string,
  moduleId: string,
): PipelineContext {
  const ctx = Context.create().child(callerId).child(moduleId);
  return {
    moduleId,
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
  } as unknown as PipelineContext;
}

// ---------------------------------------------------------------------------
// C1 — async ACL path in the pipeline
// ---------------------------------------------------------------------------

describe('C1: BuiltinACLCheck uses the async ACL path', () => {
  const KEY = 'sync_c1_untrusted_ip';

  beforeEach(() => {
    // Async-only handler: registered ONLY via registerAsyncCondition, so the
    // sync evaluator cannot see it at all.
    const handler: ACLConditionHandler = {
      evaluate: async (value: unknown, _context: Context): Promise<boolean> => {
        await Promise.resolve();
        return value === true;
      },
    };
    ACL.registerAsyncCondition(KEY, handler);
  });

  it('denies when an async-only condition on a deny rule matches, before the catch-all allow', async () => {
    const acl = new ACL(
      [
        {
          callers: ['agent.*'],
          targets: ['data.export'],
          effect: 'deny',
          description: 'deny agents from untrusted IPs',
          conditions: { [KEY]: true },
        },
        {
          callers: ['*'],
          targets: ['*'],
          effect: 'allow',
          description: 'catch-all allow',
        },
      ],
      'deny',
    );

    const step = new BuiltinACLCheck(acl);
    const pctx = makePipelineContext('agent.bot', 'data.export');

    await expect(step.execute(pctx)).rejects.toThrow(/denied/i);
  });

  it('still allows when the async-only condition does not match', async () => {
    const acl = new ACL(
      [
        {
          callers: ['agent.*'],
          targets: ['data.export'],
          effect: 'deny',
          description: 'deny agents from untrusted IPs',
          conditions: { [KEY]: false },
        },
        {
          callers: ['*'],
          targets: ['*'],
          effect: 'allow',
          description: 'catch-all allow',
        },
      ],
      'deny',
    );

    const step = new BuiltinACLCheck(acl);
    const pctx = makePipelineContext('agent.bot', 'data.export');

    await expect(step.execute(pctx)).resolves.toMatchObject({ action: 'continue' });
  });

  it('falls back to the sync check() for ACL providers without asyncCheck', async () => {
    let seen = 0;
    const duckTyped = {
      check: (): boolean => {
        seen += 1;
        return false;
      },
    } as unknown as ACL;

    const step = new BuiltinACLCheck(duckTyped);
    const pctx = makePipelineContext('agent.bot', 'data.export');

    await expect(step.execute(pctx)).rejects.toThrow(/denied/i);
    expect(seen).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// W2 — handler-error isolation
// ---------------------------------------------------------------------------

describe('W2: handler-error capture is isolated per check() call', () => {
  const THROWING = 'sync_w2_throwing';
  const NESTING = 'sync_w2_nesting';

  it('a nested check() does not consume the outer call\'s handler error', () => {
    const entries: AuditEntry[] = [];
    const inner = new ACL([], 'allow', () => {
      /* inner audit ignored */
    });

    ACL.registerCondition(THROWING, {
      evaluate: (): boolean => {
        throw new Error('boom');
      },
    });
    // A handler that performs a NESTED ACL evaluation while the outer
    // evaluation already recorded a handler error.
    ACL.registerCondition(NESTING, {
      evaluate: (_v: unknown, ctx: Context): boolean => {
        inner.check('nested.caller', 'nested.target', ctx);
        return true;
      },
    });

    const acl = new ACL(
      [
        {
          callers: ['*'],
          targets: ['*'],
          effect: 'allow',
          description: 'conditional allow',
          // Object key order matters: the throwing condition runs first and
          // records the handler error; but $or lets evaluation continue into
          // the nesting condition which runs a nested check().
          conditions: { $or: [{ [THROWING]: true }, { [NESTING]: true }] },
        },
      ],
      'deny',
      (e) => entries.push(e),
    );

    const ctx = Context.create().child('outer.caller');
    const allowed = acl.check('outer.caller', 'outer.target', ctx);

    expect(allowed).toBe(true);
    expect(entries).toHaveLength(1);
    // The outer audit entry MUST still carry the handler error raised during
    // its own evaluation; the nested check() must not have swallowed it.
    expect(entries[0].handlerError).toMatch(/boom/);
  });

  it('a concurrent asyncCheck() does not clear an in-flight call\'s handler error', async () => {
    const SLOW_OK = 'sync_w2_slow_ok';
    const FAST_THROW = 'sync_w2_fast_throw';

    ACL.registerAsyncCondition(SLOW_OK, {
      evaluate: async (): Promise<boolean> => {
        await new Promise((r) => setTimeout(r, 40));
        return true;
      },
    });
    ACL.registerAsyncCondition(FAST_THROW, {
      evaluate: async (): Promise<boolean> => {
        throw new Error('fast-boom');
      },
    });

    const aEntries: AuditEntry[] = [];
    const bEntries: AuditEntry[] = [];

    // A: rule 0's condition throws (no match, error captured), rule 1 matches
    // only after a 40 ms await — a wide window for another call to interleave.
    const aclA = new ACL(
      [
        {
          callers: ['*'],
          targets: ['*'],
          effect: 'allow',
          description: 'throwing rule',
          conditions: { [FAST_THROW]: true },
        },
        {
          callers: ['*'],
          targets: ['*'],
          effect: 'allow',
          description: 'slow allow',
          conditions: { [SLOW_OK]: true },
        },
      ],
      'deny',
      (e) => aEntries.push(e),
    );
    // B: plain, error-free evaluation that starts while A is suspended.
    const aclB = new ACL(
      [{ callers: ['*'], targets: ['*'], effect: 'allow', description: 'plain allow' }],
      'deny',
      (e) => bEntries.push(e),
    );

    const ctx = Context.create().child('c');
    const aPromise = aclA.asyncCheck('a', 'b', ctx);
    await new Promise((r) => setTimeout(r, 5));
    const bPromise = aclB.asyncCheck('a', 'b', ctx);
    await Promise.all([aPromise, bPromise]);

    expect(aEntries).toHaveLength(1);
    // A's own handler error must survive B's interleaved evaluation.
    expect(aEntries[0].handlerError).toMatch(/fast-boom/);
    expect(bEntries).toHaveLength(1);
    expect(bEntries[0].handlerError).toBeNull();
  });
});
