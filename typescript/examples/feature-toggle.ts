/**
 * Runtime feature-toggle + per-instance isolation — end-to-end demo (issue #71).
 *
 * apcore is a standalone product: this example governs real tool calls through
 * the execution pipeline, with NO framework integration required. It
 * demonstrates the `system.control.toggle_feature` capability via the `APCore`
 * convenience methods (`client.disable` / `client.enable`):
 *
 *   1. registers real `executor.*` tools on an `APCore` instance,
 *   2. calls a tool and gets a real result,
 *   3. disables it — the next call throws `ModuleDisabledError` while a
 *      *different* tool keeps working,
 *   4. re-enables it — the call works again, and
 *   5. proves **per-instance isolation (#71)**: two independent `APCore`
 *      instances each register the same tool; disabling it on instance A blocks
 *      A's call while instance B's identical call still succeeds. Each instance
 *      owns its own `ToggleState` (locked cross-language by
 *      `conformance/fixtures/toggle_state_isolation.json`).
 *
 * Toggle control requires `sys_modules` (and `sys_modules.events`) enabled in
 * config, so the example constructs that `Config` itself.
 *
 * Each step carries its expected outcome, so this script doubles as a smoke
 * test: it exits non-zero if any step drifts from the cross-language contract.
 *
 * Run (from the apcore-typescript repo root):
 *     node examples/feature-toggle.ts    # Node 23+ (or 22.6+ with --experimental-strip-types)
 *     npx tsx examples/feature-toggle.ts  # any Node
 */

import { Type } from '@sinclair/typebox';
import { APCore, Config, ModuleDisabledError } from 'apcore-js';

// ---------------------------------------------------------------------------
// Config with sys_modules + events enabled — required for toggle control.
// ---------------------------------------------------------------------------
function sysConfig(): Config {
  return new Config({ sys_modules: { enabled: true, events: { enabled: true } } });
}

/** Build a client with two real CRM tools registered. */
function buildClient(): APCore {
  const client = new APCore({ config: sysConfig() });
  client.module({
    id: 'executor.crm.read',
    description: 'Read a single CRM record',
    inputSchema: Type.Object({ record_id: Type.String() }),
    outputSchema: Type.Object({ record_id: Type.String(), name: Type.String(), tier: Type.String() }),
    execute: (i) => ({ record_id: i.record_id as string, name: 'Acme Corp', tier: 'gold' }),
  });
  client.module({
    id: 'executor.crm.query',
    description: 'Query CRM records by filter',
    inputSchema: Type.Object({ filter: Type.String() }),
    outputSchema: Type.Object({ filter: Type.String(), matched: Type.Number() }),
    execute: (i) => ({ filter: i.filter as string, matched: 3 }),
  });
  return client;
}

/** Call `target`; return [allowed, resultOrError]. false == ModuleDisabledError. */
async function callOk(
  client: APCore,
  target: string,
  inputs: Record<string, unknown>,
): Promise<[boolean, unknown]> {
  try {
    return [true, await client.call(target, inputs)];
  } catch (e) {
    if (e instanceof ModuleDisabledError) return [false, e];
    throw e;
  }
}

/** Records one PASS/FAIL line per step and tracks the failure count. */
class Checker {
  failures = 0;
  total = 0;

  check(label: string, allowed: boolean, expected: boolean, detail: unknown): void {
    this.total += 1;
    const ok = allowed === expected;
    if (!ok) this.failures += 1;
    const outcome = allowed ? 'ALLOW' : 'DISABLED';
    const shown = allowed ? JSON.stringify(detail) : 'ModuleDisabledError';
    const mark = ok ? 'PASS' : 'FAIL';
    console.log(`  ${mark.padEnd(7)}${label.padEnd(46)}${outcome.padEnd(10)}${shown}`);
    if (!ok) console.log(`         ^ expected ${expected ? 'ALLOW' : 'DISABLED'}`);
  }
}

async function main(): Promise<number> {
  console.log('Runtime feature-toggle + per-instance isolation — end-to-end demo (issue #71)');
  console.log('Real tool calls through APCore; disable/enable via system.control.toggle_feature');
  console.log('='.repeat(92));
  console.log(`  ${'RESULT'.padEnd(7)}${'STEP'.padEnd(46)}${'OUTCOME'.padEnd(10)}DETAIL`);
  console.log('-'.repeat(92));

  const chk = new Checker();

  // --- Single-instance toggle lifecycle ---------------------------------
  const client = buildClient();

  let [allowed, payload] = await callOk(client, 'executor.crm.read', { record_id: 'C-7' });
  chk.check('1. read works before toggle', allowed, true, payload);

  await client.disable('executor.crm.read', 'incident-1234');
  [allowed, payload] = await callOk(client, 'executor.crm.read', { record_id: 'C-7' });
  chk.check('2. read disabled -> blocked', allowed, false, payload);

  [allowed, payload] = await callOk(client, 'executor.crm.query', { filter: 'tier=gold' });
  chk.check('3. different tool still works', allowed, true, payload);

  await client.enable('executor.crm.read', 'incident resolved');
  [allowed, payload] = await callOk(client, 'executor.crm.read', { record_id: 'C-7' });
  chk.check('4. read re-enabled -> works', allowed, true, payload);

  // --- Per-instance isolation (#71) -------------------------------------
  const clientA = buildClient();
  const clientB = buildClient();
  await clientA.disable('executor.crm.read', 'A-only maintenance');

  [allowed, payload] = await callOk(clientA, 'executor.crm.read', { record_id: 'C-7' });
  chk.check('5. instance A disabled -> blocked', allowed, false, payload);

  [allowed, payload] = await callOk(clientB, 'executor.crm.read', { record_id: 'C-7' });
  chk.check('6. instance B unaffected -> works', allowed, true, payload);

  console.log('='.repeat(92));
  console.log(`${chk.total - chk.failures}/${chk.total} steps match the cross-language contract.`);
  return chk.failures > 0 ? 1 : 0;
}

process.exit(await main());
