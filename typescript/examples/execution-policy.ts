/**
 * Execution-time governance policy — end-to-end demo (apcore#76 RFC pilot).
 *
 * apcore is a standalone product: this example runs real tool calls through the
 * execution pipeline, with NO framework integration required. It is the natural
 * companion to `examples/approval.ts` — that demo gates a module that *declares*
 * `requiresApproval=true`; here a platform operator applies an **external**
 * `ExecutionPolicy` that overrides the governance of already-registered modules
 * at execution time, without touching their code or annotations.
 *
 * It demonstrates the three #76 pillars:
 *
 *   1. External override — `orders.delete_order` ships with NO requiresApproval,
 *      yet a policy rule forces it through the approval gate.
 *   2. destructive -> approval — `orders.purge` is annotated `destructive=true`
 *      but ungated; `gateDestructive=true` makes destructive imply approval.
 *   3. Fail loud, not silent — with `strict=true`, a module that needs approval
 *      but has no handler configured fails CLOSED (ApprovalDeniedError) instead
 *      of silently executing.
 *
 * Each scenario carries its expected outcome, so this script doubles as a smoke
 * test: it exits non-zero if any decision drifts from the intended semantics.
 *
 * Run (from the apcore-typescript repo root):
 *     node examples/execution-policy.ts    # Node 23+ (or 22.6+ with --experimental-strip-types)
 *     npx tsx examples/execution-policy.ts  # any Node
 */

import { Type } from '@sinclair/typebox';
import {
  APCore,
  ApprovalDeniedError,
  CallbackApprovalHandler,
  Context,
  ExecutionPolicy,
  PolicyRule,
  createAnnotations,
  createApprovalResult,
  createIdentity,
} from 'apcore-js';
import type { ApprovalHandler, ApprovalRequest, ApprovalResult } from 'apcore-js';

// ---------------------------------------------------------------------------
// 1. A declarative governance policy, as an operator would write it in YAML and
//    load via ExecutionPolicy.fromObject(). Here we build it directly.
//
//    - Force approval on every delete, regardless of the module's own annotations.
//    - gateDestructive: any module annotated destructive=true also needs approval.
//    - strict: if a gated module has no ApprovalHandler, FAIL CLOSED (deny),
//      never silently execute.
// ---------------------------------------------------------------------------
const policy = new ExecutionPolicy(
  [
    new PolicyRule('orders.delete_*', {
      requiresApproval: true,
      reason: 'destructive order ops need human sign-off',
    }),
  ],
  { gateDestructive: true, strict: true },
);

// Equivalent declarative form (what an operator ships in a governance file):
//   const policy = ExecutionPolicy.fromObject(YAML.parse(readFileSync('governance.yaml', 'utf8')));
//   # governance.yaml:
//   #   gate_destructive: true
//   #   strict: true
//   #   rules:
//   #     - pattern: "orders.delete_*"
//   #       requires_approval: true
//   #       reason: "destructive order ops need human sign-off"

// ---------------------------------------------------------------------------
// 2. Register tools that are NAIVE about governance — none declares
//    requiresApproval. The policy governs them from the outside.
// ---------------------------------------------------------------------------
const client = new APCore({ policy });

client.module({
  id: 'orders.list_orders',
  description: 'List orders (read-only)',
  inputSchema: Type.Object({}, { additionalProperties: true }),
  outputSchema: Type.Object({}, { additionalProperties: true }),
  execute: () => ({ orders: [1, 2, 3] }),
});
client.module({
  id: 'orders.delete_order',
  description: 'Delete an order',
  inputSchema: Type.Object({ order_id: Type.Number(), confirmed: Type.Optional(Type.Boolean()) }),
  outputSchema: Type.Object({ deleted: Type.Number() }),
  // `confirmed` is declared input the reviewer inspects; the tool runs only
  // once the policy-driven approval gate has passed.
  execute: (i) => ({ deleted: i.order_id as number }),
});
client.module({
  id: 'orders.purge',
  description: 'Purge soft-deleted orders',
  // destructive but NOT requiresApproval — gateDestructive governs it.
  annotations: createAnnotations({ destructive: true }),
  inputSchema: Type.Object({ confirmed: Type.Optional(Type.Boolean()) }),
  outputSchema: Type.Object({ purged: Type.Number() }),
  execute: () => ({ purged: 7 }),
});

// ---------------------------------------------------------------------------
// 3. A reviewer that approves only when the caller passed confirmed=true.
// ---------------------------------------------------------------------------
const review = async (request: ApprovalRequest): Promise<ApprovalResult> => {
  const approver = request.context.identity?.id ?? 'anonymous';
  if (request.arguments['confirmed'] === true) {
    return createApprovalResult({ status: 'approved', approvedBy: approver });
  }
  return createApprovalResult({
    status: 'rejected',
    approvedBy: approver,
    reason: 'caller did not confirm',
  });
};

/** Call `target`, optionally with the approval handler wired, for display. */
async function attempt(
  target: string,
  inputs: Record<string, unknown>,
  withHandler: boolean,
): Promise<[boolean, string]> {
  // setPolicy / setApprovalHandler are runtime setters — here we swap the
  // handler in/out to contrast strict fail-closed (no handler) vs. review.
  client.executor.setApprovalHandler(
    withHandler ? new CallbackApprovalHandler(review) : (null as unknown as ApprovalHandler),
  );
  const ctx = Context.create(createIdentity('alice', 'user', ['operator']));
  try {
    return [true, JSON.stringify(await client.call(target, inputs, ctx))];
  } catch (e) {
    if (e instanceof ApprovalDeniedError) return [false, `ApprovalDeniedError: ${e.reason}`];
    throw e;
  }
}

// [label, target, inputs, withHandler, expectedExecuted]
interface Scenario {
  label: string;
  target: string;
  inputs: Record<string, unknown>;
  withHandler: boolean;
  expected: boolean;
}

const scenarios: Scenario[] = [
  { label: 'Read — no gate',                       target: 'orders.list_orders',  inputs: {},                              withHandler: true,  expected: true },
  { label: 'Policy forces approval — confirmed',   target: 'orders.delete_order', inputs: { order_id: 1, confirmed: true }, withHandler: true,  expected: true },
  { label: 'Policy forces approval — unconfirmed', target: 'orders.delete_order', inputs: { order_id: 1 },                 withHandler: true,  expected: false },
  { label: 'gateDestructive — confirmed',          target: 'orders.purge',        inputs: { confirmed: true },             withHandler: true,  expected: true },
  { label: 'strict fail-closed — no handler',      target: 'orders.delete_order', inputs: { order_id: 1 },                 withHandler: false, expected: false },
];

async function main(): Promise<number> {
  console.log('Execution-time governance policy — end-to-end demo (apcore#76)');
  console.log('External ExecutionPolicy overrides governance of naive, already-registered modules');
  console.log('='.repeat(104));
  console.log(`  ${'RESULT'.padEnd(8)}${'TARGET'.padEnd(22)}${'OUTCOME'.padEnd(10)}DETAIL`);
  console.log('-'.repeat(104));

  let failures = 0;
  for (const s of scenarios) {
    const [executed, detail] = await attempt(s.target, s.inputs, s.withHandler);
    const ok = executed === s.expected;
    if (!ok) failures += 1;

    const outcome = executed ? 'EXECUTED' : 'BLOCKED';
    const mark = ok ? 'PASS' : 'FAIL';
    console.log(`  ${mark.padEnd(8)}${s.target.padEnd(22)}${outcome.padEnd(10)}${detail}`);
    if (!ok) console.log(`          ^ expected ${s.expected ? 'EXECUTED' : 'BLOCKED'} — ${s.label}`);
  }

  console.log('='.repeat(104));
  const total = scenarios.length;
  console.log(`${total - failures}/${total} decisions match the intended policy semantics.`);
  return failures > 0 ? 1 : 0;
}

process.exit(await main());
