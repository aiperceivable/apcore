/**
 * Cross-language conformance driver for `governance_state.json`
 * (PROTOCOL_SPEC 6.6.5 — configured vs. actually wired).
 *
 * Fixture source: apcore/conformance/fixtures/governance_state.json (canonical).
 *
 * Drives the real `Executor.governanceState()` on a real Registry and a real
 * strategy. All nine booleans are asserted as the SDK returned them — the
 * derived flag included, per `driver_contract.derived_flag_is_asserted_not_recomputed`:
 * a driver that recomputes it from the other eight is green against an
 * implementation that never computes it at all.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import { ACL, ExecutionPolicy, Executor, Registry } from '../src/index.js';
import { Config } from '../src/config.js';
import { ExecutionStrategy } from '../src/pipeline.js';
import type { PipelineContext, Step, StepResult } from '../src/pipeline.js';
import { findFixturesRoot } from './spec-repo.js';

interface ControlModuleSpec {
  module_id: string;
  requires_approval: boolean;
}

interface Setup {
  control_modules: ControlModuleSpec[];
  read_modules: boolean;
  strategy: string;
  acl_attached: boolean;
  approval_handler_attached: boolean;
  policy_strict: boolean;
}

interface Case {
  id: string;
  note: string;
  setup: Setup;
  expected: Record<string, boolean>;
}

const fixture: { test_cases: Case[] } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'governance_state.json'), 'utf-8'),
);

/** 6.6.5.3 — the accessor reports a handler is attached; it must never consult it. */
const neverCalledHandler = {
  requestApproval: () => {
    throw new Error('governanceState() must not invoke the approval handler');
  },
  checkApproval: () => {
    throw new Error('governanceState() must not invoke the approval handler');
  },
};

/** A step whose NAME is `acl_check` and which is not the built-in gate. */
class LookalikeACLCheck implements Step {
  readonly name = 'acl_check';
  readonly description = 'looks like the ACL gate, consults no ACL';
  readonly requires = [] as const;
  readonly provides = [] as const;
  readonly removable = true;
  readonly replaceable = true;
  async execute(_ctx: PipelineContext): Promise<StepResult> {
    return { action: 'continue' };
  }
}

function registerControlModule(registry: Registry, id: string, requiresApproval: boolean): void {
  registry.registerInternal(id, {
    description: 'conformance control module',
    annotations: { requiresApproval },
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: () => ({}),
  });
}

function build(setup: Setup): Executor {
  const registry = new Registry();
  for (const entry of setup.control_modules) {
    registerControlModule(registry, entry.module_id, entry.requires_approval);
  }
  if (setup.read_modules) {
    registerControlModule(registry, 'system.health.summary', false);
  }

  const config = new Config({});
  let executor: Executor;
  if (setup.strategy === 'lookalike_acl_check') {
    executor = new Executor({
      registry,
      config,
      strategy: new ExecutionStrategy('lookalike_acl_check', [new LookalikeACLCheck()]),
    });
  } else if (setup.strategy === 'standard') {
    executor = new Executor({ registry, config });
  } else {
    executor = new Executor({ registry, config, strategy: setup.strategy });
  }

  if (setup.acl_attached) executor.setAcl(new ACL([], 'deny'));
  if (setup.approval_handler_attached) executor.setApprovalHandler(neverCalledHandler);
  if (setup.policy_strict) executor.setPolicy(new ExecutionPolicy([], { strict: true }));
  return executor;
}

/** snake_case field name in the fixture -> camelCase accessor field. */
function camel(field: string): string {
  const [head, ...rest] = field.split('_');
  return head + rest.map((w) => w[0].toUpperCase() + w.slice(1)).join('');
}

describe('conformance: governance_state.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, () => {
      const state = build(testCase.setup) as unknown as {
        governanceState(): Record<string, boolean>;
      };
      const actual = state.governanceState();
      for (const [field, expected] of Object.entries(testCase.expected)) {
        expect(actual[camel(field)], `${testCase.id}: ${field} — ${testCase.note}`).toBe(expected);
      }
    });
  }

  it('driver_contract.purity — two reads are equal, the handler is never invoked', () => {
    const executor = build({
      control_modules: [{ module_id: 'system.control.reload_module', requires_approval: true }],
      read_modules: true,
      strategy: 'standard',
      acl_attached: false,
      approval_handler_attached: true,
      policy_strict: false,
    });
    expect(executor.governanceState()).toEqual(executor.governanceState());
  });

  it('driver_contract.liveness — a cached value passes every static case', () => {
    const executor = build({
      control_modules: [{ module_id: 'system.control.reload_module', requires_approval: true }],
      read_modules: true,
      strategy: 'standard',
      acl_attached: false,
      approval_handler_attached: false,
      policy_strict: false,
    });
    const before = executor.governanceState();
    executor.setAcl(new ACL([], 'deny'));
    const after = executor.governanceState();

    expect(before.aclConfigured).toBe(false);
    expect(after.aclConfigured).toBe(true);
  });
});
