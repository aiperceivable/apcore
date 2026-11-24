/**
 * AI Agent Tool-Governance ACL — end-to-end demo (issue #72).
 *
 * apcore is a standalone product: this example governs real tool calls through
 * the execution pipeline, with NO framework integration required. It
 *
 *   1. registers real `executor.*` / `data.*` tools,
 *   2. wires the canonical default-deny agent-governance ACL into an `APCore`
 *      instance (the same policy as the spec repo's
 *      `examples/acl/agent-tool-governance.yaml`, locked cross-language by
 *      `conformance/fixtures/acl_agent_scoping.json`),
 *   3. has agents of different roles actually CALL the tools, and
 *   4. prints the resulting audit trail.
 *
 * Allowed calls return real results; denied calls throw `ACLDeniedError`. The two
 * conditions that make ACL valuable for per-agent tool governance:
 *   - `roles`          — set-intersection against the caller's Identity.
 *   - `max_call_depth` — fuses runaway tool chains (the pipeline measures the
 *                        caller's live call-chain length, so a deeper chain trips
 *                        the fuse even for the same role).
 *
 * Each scenario carries its expected outcome, so this script doubles as a smoke
 * test: it exits non-zero if any decision drifts from the cross-language contract.
 *
 * Run (from the apcore-typescript repo root):
 *     node examples/acl-agent-governance.ts    # Node 23+ (or 22.6+ with --experimental-strip-types)
 *     npx tsx examples/acl-agent-governance.ts  # any Node
 */

import { Type } from '@sinclair/typebox';
import { ACL, APCore, ACLDeniedError, Context, createIdentity } from 'apcore-js';
import type { ACLRule, AuditEntry } from 'apcore-js';

// ---------------------------------------------------------------------------
// 1. Register real tools (executor / data modules) on an APCore instance.
// ---------------------------------------------------------------------------
const client = new APCore();

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
client.module({
  id: 'executor.crm.delete',
  description: 'Delete a CRM record',
  inputSchema: Type.Object({ record_id: Type.String() }),
  outputSchema: Type.Object({ record_id: Type.String(), deleted: Type.Boolean() }),
  execute: (i) => ({ record_id: i.record_id as string, deleted: true }),
});
client.module({
  id: 'data.export',
  description: 'Export a dataset to cold storage',
  inputSchema: Type.Object({ dataset: Type.String() }),
  outputSchema: Type.Object({ dataset: Type.String(), rows: Type.Number() }),
  execute: (i) => ({ dataset: i.dataset as string, rows: 1280 }),
});

// ---------------------------------------------------------------------------
// 2. Canonical agent-tool-governance policy (default-deny) + audit logger,
//    wired into the executor.
// ---------------------------------------------------------------------------
const rules: ACLRule[] = [
  // External / unauthenticated callers (no caller_id) — read-only.
  { callers: ['@external'], targets: ['executor.*.read'], effect: 'allow', description: 'External callers may only read.', conditions: null },
  // Reader-role agents — read + query, depth-capped to fuse runaway chains.
  { callers: ['agent.*'], targets: ['executor.*.read', 'executor.*.query'], effect: 'allow', description: 'Reader agents may read/query, depth-capped.', conditions: { roles: ['reader'], max_call_depth: 3 } },
  // Data-admin agents — exports and sensitive deletes (no depth cap).
  { callers: ['agent.*'], targets: ['data.export', 'executor.*.delete'], effect: 'allow', description: 'Data-admin agents may export and delete.', conditions: { roles: ['data_admin'] } },
];

const auditTrail: AuditEntry[] = [];
const acl = new ACL(rules, 'deny', (e) => auditTrail.push(e));
client.executor.setAcl(acl);

// ---------------------------------------------------------------------------
// 3. Drive real tool calls as different agents.
// ---------------------------------------------------------------------------
interface Scenario {
  label: string;
  callerId: string | null;
  roles: string[] | null;
  upstreamHops: number;
  target: string;
  inputs: Record<string, unknown>;
  expected: boolean;
}

/** Call `target` as `callerId`, returning [allowed, resultOrError]. */
async function attempt(s: Scenario): Promise<[boolean, unknown]> {
  const identity = s.roles ? createIdentity(s.callerId ?? 'unknown', 'ai', s.roles) : null;
  const prior = Array.from({ length: s.upstreamHops }, (_, i) => `upstream.hop${i}`);
  const callChain = s.callerId ? [...prior, s.callerId] : prior;
  const ctx = new Context('trace-id', s.callerId, callChain, null, identity);
  try {
    return [true, await client.call(s.target, s.inputs, ctx)];
  } catch (e) {
    if (e instanceof ACLDeniedError) return [false, e];
    throw e;
  }
}

const scenarios: Scenario[] = [
  { label: 'External — read',                callerId: null,             roles: null,           upstreamHops: 0, target: 'executor.crm.read',   inputs: { record_id: 'C-7' },    expected: true },
  { label: 'External — query (blocked)',     callerId: null,             roles: null,           upstreamHops: 0, target: 'executor.crm.query',  inputs: { filter: 'tier=gold' }, expected: false },
  { label: 'External — delete (blocked)',    callerId: null,             roles: null,           upstreamHops: 0, target: 'executor.crm.delete', inputs: { record_id: 'C-7' },    expected: false },
  { label: 'Reader — read',                  callerId: 'agent.research', roles: ['reader'],     upstreamHops: 0, target: 'executor.crm.read',   inputs: { record_id: 'C-7' },    expected: true },
  { label: 'Reader — query',                 callerId: 'agent.research', roles: ['reader'],     upstreamHops: 0, target: 'executor.crm.query',  inputs: { filter: 'tier=gold' }, expected: true },
  { label: 'Reader — query at depth cap',    callerId: 'agent.research', roles: ['reader'],     upstreamHops: 1, target: 'executor.crm.query',  inputs: { filter: 'tier=gold' }, expected: true },
  { label: 'Reader — query over depth cap',  callerId: 'agent.research', roles: ['reader'],     upstreamHops: 2, target: 'executor.crm.query',  inputs: { filter: 'tier=gold' }, expected: false },
  { label: 'Reader — delete (blocked)',      callerId: 'agent.research', roles: ['reader'],     upstreamHops: 0, target: 'executor.crm.delete', inputs: { record_id: 'C-7' },    expected: false },
  { label: 'Reader — export (blocked)',      callerId: 'agent.research', roles: ['reader'],     upstreamHops: 0, target: 'data.export',         inputs: { dataset: 'crm' },      expected: false },
  { label: 'Data-admin — export',            callerId: 'agent.etl',      roles: ['data_admin'], upstreamHops: 0, target: 'data.export',         inputs: { dataset: 'crm' },      expected: true },
  { label: 'Data-admin — delete',            callerId: 'agent.etl',      roles: ['data_admin'], upstreamHops: 0, target: 'executor.crm.delete', inputs: { record_id: 'C-7' },    expected: true },
  { label: 'Data-admin — delete deep chain', callerId: 'agent.etl',      roles: ['data_admin'], upstreamHops: 3, target: 'executor.crm.delete', inputs: { record_id: 'C-7' },    expected: true },
  { label: 'Data-admin — query (blocked)',   callerId: 'agent.etl',      roles: ['data_admin'], upstreamHops: 0, target: 'executor.crm.query',  inputs: { filter: 'tier=gold' }, expected: false },
  { label: 'Unknown-role agent (blocked)',   callerId: 'agent.guest',    roles: ['guest'],      upstreamHops: 0, target: 'executor.crm.read',   inputs: { record_id: 'C-7' },    expected: false },
];

async function main(): Promise<number> {
  console.log('AI Agent Tool-Governance ACL — end-to-end demo (issue #72)');
  console.log('Real tool calls through APCore, default_effect: deny, gradient @external < reader < data_admin');
  console.log('='.repeat(100));
  console.log(`  ${'RESULT'.padEnd(8)}${'CALLER'.padEnd(17)}${'ROLES'.padEnd(13)}${'TARGET'.padEnd(22)}${'OUTCOME'.padEnd(8)}DETAIL`);
  console.log('-'.repeat(100));

  let failures = 0;
  for (const s of scenarios) {
    const [allowed, payload] = await attempt(s);
    const ok = allowed === s.expected;
    if (!ok) failures += 1;

    const callerDisp = s.callerId ?? '@external';
    const rolesDisp = s.roles ? s.roles.join(',') : '-';
    const outcome = allowed ? 'ALLOW' : 'DENY';
    const detail = allowed ? JSON.stringify(payload) : 'ACLDeniedError';
    const mark = ok ? 'PASS' : 'FAIL';
    console.log(`  ${mark.padEnd(8)}${callerDisp.padEnd(17)}${rolesDisp.padEnd(13)}${s.target.padEnd(22)}${outcome.padEnd(8)}${detail}`);
    if (!ok) console.log(`          ^ expected ${s.expected ? 'ALLOW' : 'DENY'} — ${s.label}`);
  }

  console.log('='.repeat(100));
  console.log(`Audit trail (${auditTrail.length} decisions recorded by the ACL):`);
  for (const e of auditTrail) {
    const roles = e.roles.length ? e.roles.join(',') : '-';
    console.log(
      `  ${e.callerId.padEnd(17)} -> ${e.targetId.padEnd(22)} ${e.decision.padEnd(5)}` +
        ` (reason=${e.reason}, roles=${roles}, depth=${e.callDepth})`,
    );
  }

  console.log('='.repeat(100));
  const total = scenarios.length;
  console.log(`${total - failures}/${total} decisions match the cross-language contract.`);
  return failures > 0 ? 1 : 0;
}

process.exit(await main());
