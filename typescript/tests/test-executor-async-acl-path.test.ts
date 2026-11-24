/**
 * The executor's ACL step takes the ASYNC path (spec v1.50.0, D-105).
 *
 * §6.1.3 defines what each ACL entry point resolves and never said which one the
 * pipeline calls. apcore-rust called the synchronous `checkAccess` from inside an
 * already-async step, which makes any condition registered through
 * `registerAsyncCondition` "async only" on that path — it resolves to
 * UNEVALUABLE, so an `allow` rule carrying it stops granting and a `deny` rule
 * carrying it denies unconditionally. Both directions wrong, and the entire async
 * condition registry unreachable from the only path that enforces.
 *
 * All three SDKs take the async path today. Nothing said so: a whole extension
 * point reachable from every door except the enforcing one is invisible from the
 * door, and a registry that accepts a handler is not evidence anything calls it.
 *
 * The discriminator is a SYNC handler answering false and an ASYNC handler
 * answering true for the same key. Counting invocations would not separate the
 * paths — both invoke *a* handler. Only the verdict does.
 */

import { describe, it, expect } from 'vitest';
import { ACL } from '../src/acl.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';

const MODULE_ID = 'executor.probe.async_acl';
// One key per test. `registerCondition` / `registerAsyncCondition` are STATIC,
// writing into class-level maps, so a handler registered by one test is visible
// to every later ACL in the process.
const KEY_ASYNC_WINS = 'probe_ts_async_only';
const KEY_SYNC_ONLY = 'probe_ts_sync_only';

const probeModule = {
  inputSchema: { type: 'object' },
  outputSchema: { type: 'object' },
  description: 'probe',
  execute: async () => ({ ran: true }),
};

function executorWith(key: string, withAsync: boolean): Executor {
  const registry = new Registry();
  registry.register(MODULE_ID, probeModule);
  // The sync handler also satisfies the structural precheck, which rejects a
  // rule naming a condition no handler claims — without it the rule would be
  // unevaluable for a second, unrelated reason.
  ACL.registerCondition(key, { evaluate: () => false } as never);
  if (withAsync) {
    ACL.registerAsyncCondition(key, { evaluate: async () => true } as never);
  }
  const acl = new ACL(
    [{ callers: ['*'], targets: [MODULE_ID], effect: 'allow', conditions: { [key]: true } }] as never,
    'deny',
  );
  return new Executor({ registry, acl } as never);
}

describe("the executor's ACL step takes the async path (D-105)", () => {
  it('an async-only condition decides the call', async () => {
    const executor = executorWith(KEY_ASYNC_WINS, true);

    await expect(executor.call(MODULE_ID, {})).resolves.toEqual({ ran: true });
  });

  it('the sync handler is the one that would deny', async () => {
    // The control: it proves the two handlers genuinely disagree, so the test
    // above separates the paths rather than passing because the condition is
    // satisfied either way.
    const executor = executorWith(KEY_SYNC_ONLY, false);

    await expect(executor.call(MODULE_ID, {})).rejects.toThrow(/denied/i);
  });
});
