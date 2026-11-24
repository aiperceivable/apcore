/**
 * Config-driven ACL discovery — the `acl.root` path (D-64 / issue #74).
 *
 * apcore can wire ACL enforcement two ways:
 *
 *   - MANUAL (see `acl-agent-governance.ts`): build an `ACL` in code and call
 *     `client.executor.setAcl(acl)` yourself.
 *   - CONFIG-DRIVEN (this file): point `acl.root` in `apcore.yaml` at a policy
 *     file. `new APCore({ config })` then discovers and attaches that ACL
 *     AUTOMATICALLY at construction time — no `setAcl` call anywhere.
 *
 * This script lays out a tiny throwaway project on disk so `acl.root` resolves
 * relative to the config file (the canonical anchor):
 *
 *     <tmp>/apcore.yaml            acl: { root: ./acl, default_effect: deny }
 *     <tmp>/acl/global_acl.yaml    first-match-wins, default-deny policy
 *
 * It then loads the config, constructs `new APCore({ config })`, and proves the
 * discovered ACL is live: an allowed call (`@external` -> greet) returns a real
 * result, and a denied inter-module call throws `ACLDeniedError` — with NO
 * manual `setAcl`.
 *
 * `acl.root` is a directory by convention (the default is `./acl`); discovery
 * loads the conventional `<root>/global_acl.yaml` inside it. (acl.root MAY also
 * point directly at a YAML file.) This matches apcore-python and apcore-rust.
 *
 * Each scenario carries its expected outcome, so this script doubles as a smoke
 * test: it exits non-zero if any decision drifts from the policy.
 *
 * Run (from the apcore-typescript repo root):
 *     node examples/acl-config-driven.ts     # Node 23+ (or 22.6+ with --experimental-strip-types)
 *     npx tsx examples/acl-config-driven.ts  # any Node
 */

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Type } from '@sinclair/typebox';
import { ACLDeniedError, APCore, Config, Context, createIdentity } from 'apcore-js';

// ---------------------------------------------------------------------------
// 1. Lay out a throwaway project: apcore.yaml + co-located acl/global_acl.yaml.
//    `acl.root` is relative, so it anchors at the config file's directory.
// ---------------------------------------------------------------------------
const projectDir = mkdtempSync(join(tmpdir(), 'apcore-acl-config-'));
mkdirSync(join(projectDir, 'acl'));

// First-match-wins, default-deny policy: external callers may reach `greet`;
// everything else (including inter-module calls) falls through to deny.
const GLOBAL_ACL = `default_effect: deny
rules:
  - callers: ["@external"]
    targets: ["executor.greeter.greet"]
    effect: allow
    description: External callers may greet.
`;

// `version` and `project.name` are the only two keys a config file must
// declare — every other key here carries a canonical default (§9.1).
const APCORE_YAML = `version: "1.0.0"
project:
  name: acl-config-driven-demo
acl:
  root: ./acl
  default_effect: deny
`;

writeFileSync(join(projectDir, 'acl', 'global_acl.yaml'), GLOBAL_ACL);
writeFileSync(join(projectDir, 'apcore.yaml'), APCORE_YAML);

// ---------------------------------------------------------------------------
// 2. Load the config and construct the client. Enforcement is wired
//    AUTOMATICALLY by the APCore bootstrap — note the absence of setAcl().
// ---------------------------------------------------------------------------
const config = Config.load(join(projectDir, 'apcore.yaml'));
const client = new APCore({ config });

client.module({
  id: 'executor.greeter.greet',
  description: 'Return a friendly greeting',
  inputSchema: Type.Object({ name: Type.String() }),
  outputSchema: Type.Object({ message: Type.String() }),
  execute: (i) => ({ message: `Hello, ${i.name as string}!` }),
});

// ---------------------------------------------------------------------------
// 3. Drive one allowed and one denied call against the discovered ACL.
// ---------------------------------------------------------------------------
interface Scenario {
  label: string;
  callerId: string | null;
  target: string;
  inputs: Record<string, unknown>;
  expected: boolean;
}

/** Call `target` as `callerId`, returning [allowed, resultOrError]. */
async function attempt(s: Scenario): Promise<[boolean, unknown]> {
  // `@external` = a null caller_id (top-level, unauthenticated). An
  // inter-module call carries a real caller_id in the call chain.
  const identity = s.callerId ? createIdentity(s.callerId, 'ai', []) : null;
  const callChain = s.callerId ? [s.callerId] : [];
  const ctx = new Context('trace-id', s.callerId, callChain, null, identity);
  try {
    return [true, await client.call(s.target, s.inputs, ctx)];
  } catch (e) {
    if (e instanceof ACLDeniedError) return [false, e];
    throw e;
  }
}

const scenarios: Scenario[] = [
  { label: 'External -> greet (allowed by rule)', callerId: null, target: 'executor.greeter.greet', inputs: { name: 'Ada' }, expected: true },
  { label: 'Inter-module -> greet (default-deny)', callerId: 'executor.workflow.run', target: 'executor.greeter.greet', inputs: { name: 'Ada' }, expected: false },
];

async function main(): Promise<number> {
  console.log('Config-driven ACL discovery — acl.root path (D-64 / issue #74)');
  console.log(`Project: ${projectDir}`);
  console.log('Enforcement was attached AUTOMATICALLY by new APCore({ config }) — no setAcl() call.');
  console.log('Policy: default_effect: deny; allow @external -> executor.greeter.greet.');
  console.log('='.repeat(92));
  console.log(`  ${'RESULT'.padEnd(8)}${'CALLER'.padEnd(26)}${'TARGET'.padEnd(28)}${'OUTCOME'.padEnd(8)}DETAIL`);
  console.log('-'.repeat(92));

  let failures = 0;
  for (const s of scenarios) {
    const [allowed, payload] = await attempt(s);
    const ok = allowed === s.expected;
    if (!ok) failures += 1;

    const callerDisp = s.callerId ?? '@external';
    const outcome = allowed ? 'ALLOW' : 'DENY';
    const detail = allowed ? JSON.stringify(payload) : 'ACLDeniedError';
    const mark = ok ? 'PASS' : 'FAIL';
    console.log(`  ${mark.padEnd(8)}${callerDisp.padEnd(26)}${s.target.padEnd(28)}${outcome.padEnd(8)}${detail}`);
    if (!ok) console.log(`          ^ expected ${s.expected ? 'ALLOW' : 'DENY'} — ${s.label}`);
  }

  console.log('='.repeat(92));
  const total = scenarios.length;
  console.log(`${total - failures}/${total} decisions match the config-driven policy.`);
  return failures > 0 ? 1 : 0;
}

let exitCode = 1;
try {
  exitCode = await main();
} finally {
  // Clean up the throwaway project directory.
  rmSync(projectDir, { recursive: true, force: true });
}
process.exit(exitCode);
