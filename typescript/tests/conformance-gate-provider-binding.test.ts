/**
 * Cross-language conformance driver for `gate_provider_binding.json`
 * (PROTOCOL_SPEC 6.6.5.5, decision D-129).
 *
 * Fixture source: apcore/conformance/fixtures/gate_provider_binding.json
 * (canonical). Its `driver_contract` block is the contract:
 *
 *  - `path`: a real Executor built through the public constructor, one real
 *    call to `demo.target` as `@external` (no context, so no caller_id), then
 *    `governanceState()`. The call is asserted as well as the accessor — the
 *    defect this fixture pins is a call that RAN while the accessor reported a
 *    gate in front of it.
 *  - `strategy_form`: `preset:<name>` passes the preset NAME, `instance:*`
 *    builds the standard strategy with the public builder and passes the
 *    instance, `default` passes no strategy.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import {
  ACL,
  AlwaysDenyHandler,
  ExecutionPolicy,
  Executor,
  Registry,
  buildStandardStrategy,
} from '../src/index.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import { ModuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

type AclName = 'deny_all' | 'allow_all' | null;

interface Setup {
  strategy_form: string;
  executor_acl: AclName;
  strategy_acl: AclName;
  approval_handler: 'always_deny' | null;
  policy_strict: boolean;
  requires_approval: boolean;
}

interface Case {
  id: string;
  note?: string;
  setup: Setup;
  expected: { call: string; governance: Record<string, boolean> };
}

const fixture: { test_cases: Case[]; driver_contract: Record<string, string> } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'gate_provider_binding.json'), 'utf-8'),
);

function makeAcl(name: AclName): ACL | null {
  if (name === null) return null;
  if (name === 'deny_all') return new ACL([], 'deny');
  return new ACL([{ callers: ['*'], targets: ['*'], effect: 'allow', description: 'allow all' }], 'deny');
}

async function build(setup: Setup): Promise<Executor> {
  const registry = new Registry();
  await registry.register('demo.target', {
    description: 'gate_provider_binding target',
    annotations: { requiresApproval: setup.requires_approval },
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: () => ({ ok: true }),
  });

  const acl = makeAcl(setup.executor_acl);
  const approvalHandler = setup.approval_handler === 'always_deny' ? new AlwaysDenyHandler() : null;
  const policy = setup.policy_strict ? new ExecutionPolicy([], { strict: true }) : null;
  const providers = { acl, approvalHandler, policy };

  const form = setup.strategy_form;
  if (form === 'default') {
    return new Executor({ registry, ...providers });
  }
  if (form.startsWith('preset:')) {
    return new Executor({ registry, strategy: form.slice('preset:'.length), ...providers });
  }
  if (form === 'instance:bare' || form === 'instance:own_acl') {
    // The public builder, given no provider except (for `own_acl`) the
    // strategy's own ACL. The executor's providers reach the instance only
    // through the Executor constructor — which is the path D-129 is about.
    const strategy = buildStandardStrategy({
      config: null,
      registry,
      acl: form === 'instance:own_acl' ? makeAcl(setup.strategy_acl) : null,
      approvalHandler: null,
      middlewareManager: new MiddlewareManager(),
    });
    return new Executor({ registry, strategy, ...providers });
  }
  throw new Error(`gate_provider_binding.json: unknown strategy_form '${form}'`);
}

async function callOutcome(executor: Executor): Promise<string> {
  try {
    const out = await executor.call('demo.target', {});
    expect(out).toEqual({ ok: true });
    return 'ok';
  } catch (err) {
    if (err instanceof ModuleError) return err.code;
    throw err;
  }
}

/** snake_case field name in the fixture -> camelCase accessor field. */
function camel(field: string): string {
  const [head, ...rest] = field.split('_');
  return head + rest.map((w) => w[0].toUpperCase() + w.slice(1)).join('');
}

describe('conformance: gate_provider_binding.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, async () => {
      const executor = await build(testCase.setup);
      expect(await callOutcome(executor), `${testCase.id}: call`).toBe(testCase.expected.call);
      const state = executor.governanceState() as unknown as Record<string, boolean>;
      for (const [field, expected] of Object.entries(testCase.expected.governance)) {
        expect(state[camel(field)], `${testCase.id}: ${field} — ${testCase.note ?? ''}`).toBe(
          expected,
        );
      }
    });
  }

  it('honours every driver_contract rule', () => {
    expect(Object.keys(fixture.driver_contract).sort()).toEqual([
      'expected',
      'module',
      'path',
      'providers',
      'strategy_form',
    ]);
  });
});
