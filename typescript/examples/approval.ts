/**
 * Human-in-the-loop Approval gate — end-to-end demo.
 *
 * apcore is a standalone product: this example runs real tool calls through the
 * execution pipeline, with NO framework integration required. It is the natural
 * companion to `examples/acl-agent-governance.ts` — ACL governs *who* may call a
 * tool; Approval is the *human-in-the-loop gate* for the sensitive ones. The gate
 * fires at Executor Step 5, after ACL passes and before middleware runs. See the
 * spec repo's `docs/features/approval-system.md`.
 *
 * It
 *
 *   1. registers a sensitive tool that declares `requiresApproval=true`
 *      (`executor.crm.delete`) and a normal tool that does not
 *      (`executor.crm.read`),
 *   2. wires a `CallbackApprovalHandler` whose decision depends on the request —
 *      it approves only when the caller passes `confirmed=true`, and rejects
 *      otherwise (the kind of policy a real reviewer UI would enforce), and
 *   3. runs scenarios that actually CALL the tools.
 *
 * A normal tool runs without ever reaching the gate. A sensitive tool with the
 * handler APPROVING returns its real result; with the handler REJECTING it is
 * blocked and `client.call(...)` throws `ApprovalDeniedError`.
 *
 * Each scenario carries its expected outcome, so this script doubles as a smoke
 * test: it exits non-zero if any decision drifts from the cross-language contract.
 *
 * Run (from the apcore-typescript repo root):
 *     node examples/approval.ts    # Node 23+ (or 22.6+ with --experimental-strip-types)
 *     npx tsx examples/approval.ts  # any Node
 */

import { Type } from '@sinclair/typebox';
import {
  APCore,
  ApprovalDeniedError,
  CallbackApprovalHandler,
  Context,
  createAnnotations,
  createApprovalResult,
  createIdentity,
} from 'apcore-js';
import type { ApprovalRequest, ApprovalResult } from 'apcore-js';

// ---------------------------------------------------------------------------
// 1. Register tools. The delete tool is sensitive: it declares
//    requiresApproval=true, so the gate intercepts it. The read tool is normal.
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
  id: 'executor.crm.delete',
  description: 'Delete a CRM record (sensitive — requires human approval)',
  annotations: createAnnotations({ requiresApproval: true, destructive: true }),
  inputSchema: Type.Object({ record_id: Type.String(), confirmed: Type.Optional(Type.Boolean()) }),
  outputSchema: Type.Object({ record_id: Type.String(), deleted: Type.Boolean() }),
  // `confirmed` is part of the declared input the handler reviews; the tool
  // itself only runs once the approval gate has passed.
  execute: (i) => ({ record_id: i.record_id as string, deleted: true }),
});

// ---------------------------------------------------------------------------
// 2. Wire a request-dependent ApprovalHandler. A real handler would block on a
//    reviewer UI / Slack / e-mail; here the decision is driven by the request so
//    the demo is deterministic: approve only when the caller passed confirmed=true.
// ---------------------------------------------------------------------------
const review = async (request: ApprovalRequest): Promise<ApprovalResult> => {
  const approver = request.context.identity?.id ?? 'anonymous';
  if (request.arguments['confirmed'] === true) {
    return createApprovalResult({ status: 'approved', approvedBy: approver });
  }
  return createApprovalResult({
    status: 'rejected',
    approvedBy: approver,
    reason: 'caller did not confirm the destructive operation',
  });
};

client.executor.setApprovalHandler(new CallbackApprovalHandler(review));

// ---------------------------------------------------------------------------
// 3. Drive real tool calls.
// ---------------------------------------------------------------------------
interface Scenario {
  label: string;
  callerId: string;
  target: string;
  inputs: Record<string, unknown>;
  expected: boolean;
}

/** Call `target` as `callerId`, returning [executed, resultOrError]. */
async function attempt(s: Scenario): Promise<[boolean, unknown]> {
  const identity = createIdentity(s.callerId, 'user', ['operator']);
  const ctx = Context.create(identity);
  try {
    return [true, await client.call(s.target, s.inputs, ctx)];
  } catch (e) {
    if (e instanceof ApprovalDeniedError) return [false, e];
    throw e;
  }
}

const scenarios: Scenario[] = [
  { label: 'Normal tool — no gate',              callerId: 'alice', target: 'executor.crm.read',   inputs: { record_id: 'C-7' },                     expected: true },
  { label: 'Sensitive — confirmed (approved)',   callerId: 'alice', target: 'executor.crm.delete', inputs: { record_id: 'C-7', confirmed: true },    expected: true },
  { label: 'Sensitive — unconfirmed (rejected)', callerId: 'alice', target: 'executor.crm.delete', inputs: { record_id: 'C-7' },                     expected: false },
  { label: 'Sensitive — confirmed=false (rejected)', callerId: 'bob', target: 'executor.crm.delete', inputs: { record_id: 'C-9', confirmed: false }, expected: false },
];

async function main(): Promise<number> {
  console.log('Human-in-the-loop Approval gate — end-to-end demo');
  console.log('Real tool calls through APCore; gate fires at Step 5 for requiresApproval modules');
  console.log('='.repeat(100));
  console.log(`  ${'RESULT'.padEnd(8)}${'CALLER'.padEnd(10)}${'TARGET'.padEnd(22)}${'OUTCOME'.padEnd(10)}DETAIL`);
  console.log('-'.repeat(100));

  let failures = 0;
  for (const s of scenarios) {
    const [executed, payload] = await attempt(s);
    const ok = executed === s.expected;
    if (!ok) failures += 1;

    const outcome = executed ? 'EXECUTED' : 'BLOCKED';
    const detail = executed
      ? JSON.stringify(payload)
      : `ApprovalDeniedError: ${(payload as ApprovalDeniedError).reason}`;
    const mark = ok ? 'PASS' : 'FAIL';
    console.log(`  ${mark.padEnd(8)}${s.callerId.padEnd(10)}${s.target.padEnd(22)}${outcome.padEnd(10)}${detail}`);
    if (!ok) console.log(`          ^ expected ${s.expected ? 'EXECUTED' : 'BLOCKED'} — ${s.label}`);
  }

  console.log('='.repeat(100));
  const total = scenarios.length;
  console.log(`${total - failures}/${total} decisions match the cross-language contract.`);
  return failures > 0 ? 1 : 0;
}

process.exit(await main());
