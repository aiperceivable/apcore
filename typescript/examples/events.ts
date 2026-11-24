/**
 * Lifecycle event subscription (event bus) — end-to-end demo.
 *
 * apcore ships a global event bus for framework lifecycle events: module
 * registration, feature toggles, config updates, and health thresholds. This
 * example wires the bus into an `APCore` client and watches real lifecycle
 * events fire as the client is driven — see the feature spec at
 * `apcore/docs/features/event-system.md` ("Event Naming Convention" and the
 * "Event Types" table for the canonical `apcore.<subsystem>.<event>` names).
 *
 * It
 *
 *   1. builds an `APCore` with `sys_modules` + `events` ENABLED in `Config`
 *      (the bus is off by default; `on()` throws `SysModulesDisabledError`
 *      otherwise),
 *   2. subscribes via `client.on(eventType, callback)` to two canonical
 *      lifecycle events — `apcore.registry.module_registered` and
 *      `apcore.module.toggled` — collecting each into a list,
 *   3. performs the actions that fire them: registers a real tool (fires
 *      `module_registered`) and disables + re-enables it (fires
 *      `module.toggled` twice), then
 *   4. prints the collected events and unsubscribes via `client.off()`.
 *
 * Each expected event carries its count, so this script doubles as a smoke
 * test: it exits non-zero if the observed lifecycle stream drifts.
 *
 * Run (from the apcore-typescript repo root):
 *     node examples/events.ts    # Node 23+ (or 22.6+ with --experimental-strip-types)
 *     npx tsx examples/events.ts  # any Node
 */

import { Type } from '@sinclair/typebox';
import { APCore, Config } from 'apcore-js';
import type { ApCoreEvent, EventSubscriber } from 'apcore-js';

// ---------------------------------------------------------------------------
// 1. Build an APCore with the event bus enabled. The bus is off by default;
//    sys_modules.enabled + sys_modules.events.enabled turn it on so that
//    client.events / client.on / client.off are live.
// ---------------------------------------------------------------------------
const config = new Config({
  sys_modules: { enabled: true, events: { enabled: true } },
});
const client = new APCore({ config });

// ---------------------------------------------------------------------------
// 2. Subscribe to canonical lifecycle events; every delivery is collected.
// ---------------------------------------------------------------------------
const collected: ApCoreEvent[] = [];

const subRegistered: EventSubscriber = client.on('apcore.registry.module_registered', (e) => {
  collected.push(e);
});
const subToggled: EventSubscriber = client.on('apcore.module.toggled', (e) => {
  collected.push(e);
});

// ---------------------------------------------------------------------------
// 3. Drive the actions that fire those events.
// ---------------------------------------------------------------------------
async function runLifecycle(): Promise<void> {
  // Registration fires apcore.registry.module_registered.
  client.module({
    id: 'executor.crm.read',
    description: 'Read a single CRM record',
    inputSchema: Type.Object({ record_id: Type.String() }),
    outputSchema: Type.Object({
      record_id: Type.String(),
      name: Type.String(),
      tier: Type.String(),
    }),
    execute: (i) => ({ record_id: i.record_id as string, name: 'Acme Corp', tier: 'gold' }),
  });

  // A real call through the pipeline — the result is unaffected by the bus.
  await client.call('executor.crm.read', { record_id: 'C-7' });

  // Each toggle fires apcore.module.toggled (enabled=false, then enabled=true).
  await client.disable('executor.crm.read', 'maintenance window');
  await client.enable('executor.crm.read', 'maintenance complete');

  // Delivery is async dispatch; block until the bus drains.
  await client.events!.flush();
}

// [eventType, expected count]
const expected: [string, number][] = [
  ['apcore.registry.module_registered', 1],
  ['apcore.module.toggled', 2],
];

async function main(): Promise<number> {
  console.log('Lifecycle event subscription (event bus) — end-to-end demo');
  console.log('APCore with sys_modules.events enabled; canonical apcore.<subsystem>.<event> names');
  console.log('='.repeat(92));

  await runLifecycle();

  console.log(`Collected ${collected.length} lifecycle events (in delivery order):`);
  console.log('-'.repeat(92));
  for (const e of collected) {
    const detail =
      e.eventType === 'apcore.module.toggled'
        ? `module_id=${e.data['module_id']}, enabled=${e.data['enabled']}`
        : `module_id=${e.moduleId}`;
    console.log(`  ${e.severity.padEnd(6)}${e.eventType.padEnd(40)}${detail}`);
  }

  // Unsubscribe; subsequent events would no longer be collected.
  client.off(subRegistered);
  client.off(subToggled);

  console.log('='.repeat(92));
  let failures = 0;
  for (const [eventType, want] of expected) {
    const got = collected.filter((e) => e.eventType === eventType).length;
    const ok = got === want;
    if (!ok) failures += 1;
    const mark = ok ? 'PASS' : 'FAIL';
    console.log(`  ${mark}  ${eventType.padEnd(40)} received ${got}, expected ${want}`);
  }

  console.log('='.repeat(92));
  const total = expected.length;
  console.log(`${total - failures}/${total} lifecycle event expectations met.`);
  return failures > 0 ? 1 : 0;
}

process.exit(await main());
