/**
 * Executor.governanceState() — configured vs. actually wired.
 *
 * PROTOCOL_SPEC 6.6.5. The accessor exists because `acl != null` is not the
 * answer to "what is gating this registry": the gates are pipeline steps, and
 * the `internal` / `testing` / `minimal` presets remove them.
 */
import { describe, expect, it } from 'vitest';

import { ACL, APCore, ExecutionPolicy, Executor } from '../src/index.js';
import { Config } from '../src/config.js';
import { ExecutionStrategy } from '../src/pipeline.js';
import type { Step, StepResult, PipelineContext } from '../src/pipeline.js';

const SYS_ON = { sys_modules: { enabled: true, events: { enabled: true } } };
const READ_ONLY = { sys_modules: { enabled: true, events: { enabled: false } } };
const SYS_OFF = { sys_modules: { enabled: false } };

function client(cfg: Record<string, unknown>): APCore {
  return new APCore({ config: new Config(cfg) });
}

function denyAll(): ACL {
  return new ACL([], 'deny');
}

function registerControlModule(c: APCore, id: string, requiresApproval: boolean): void {
  c.registry.registerInternal(id, {
    description: 'test control module',
    annotations: { requiresApproval },
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: () => ({}),
  });
}

describe('governanceState — registration observations', () => {
  it('reports nothing when no system modules are registered', () => {
    const state = client(SYS_OFF).executor.governanceState();
    expect(state.controlModulesRegistered).toBe(false);
    expect(state.readModulesRegistered).toBe(false);
    expect(state.unprotectedControlSurface).toBe(false);
  });

  it('read modules only is not a control surface', () => {
    // Six read-only modules and no ACL is an information-disclosure question,
    // not a control-plane one. The flag must not fire where there is no write
    // surface at all.
    const state = client(READ_ONLY).executor.governanceState();
    expect(state.readModulesRegistered).toBe(true);
    expect(state.controlModulesRegistered).toBe(false);
    expect(state.unprotectedControlSurface).toBe(false);
  });

  it('control modules with no gates is the condition the flag exists for', () => {
    const state = client(SYS_ON).executor.governanceState();
    expect(state.controlModulesRegistered).toBe(true);
    expect(state.unprotectedControlSurface).toBe(true);
  });
});

describe('governanceState — configured is not enforced', () => {
  it('acl attached to a standard strategy is wired', () => {
    const ex = client(SYS_ON).executor;
    ex.setAcl(denyAll());
    const state = ex.governanceState();
    expect(state.aclConfigured).toBe(true);
    expect(state.builtinAclGateWired).toBe(true);
    expect(state.unprotectedControlSurface).toBe(false);
  });

  it('acl attached to the internal strategy is NOT wired', () => {
    const c = client(SYS_ON);
    const ex = new Executor({ registry: c.registry, config: c.config, strategy: 'internal' });
    ex.setAcl(denyAll());
    const state = ex.governanceState();
    expect(state.aclConfigured).toBe(true);
    expect(state.builtinAclGateWired).toBe(false);
    // `acl != null` would report this as protected. It is not.
    expect(state.unprotectedControlSurface).toBe(true);
  });
});

describe('governanceState — gate detection is by type, not by name', () => {
  it('a custom step named acl_check is not the built-in gate', () => {
    // PROTOCOL_SPEC 6.6.5.2. A name test would set the flag here, and a false
    // `builtinAclGateWired` is the one direction that must never happen: it
    // reports a gate that is not there.
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

    const c = client(SYS_ON);
    const strategy = new ExecutionStrategy('lookalike', [new LookalikeACLCheck()]);
    const ex = new Executor({ registry: c.registry, config: c.config, strategy });
    ex.setAcl(denyAll());

    const state = ex.governanceState();
    expect(state.aclConfigured).toBe(true);
    expect(state.builtinAclGateWired).toBe(false);
    expect(state.unprotectedControlSurface).toBe(true);
  });
});

describe('governanceState — the approval gate is per-module conditional', () => {
  // PROTOCOL_SPEC 6.6.5.1.1. `acl_check` evaluates every call; `approval_gate`
  // returns before consulting the handler when the module does not need
  // approval, so a wired gate plus a handler gates nothing for an unannotated
  // control module.

  it('the SDK control modules declare requiresApproval', () => {
    const state = client(SYS_ON).executor.governanceState();
    expect(state.allControlModulesRequireApproval).toBe(true);
  });

  it('an unannotated control module leaves the surface unprotected', () => {
    const c = client(SYS_ON);
    registerControlModule(c, 'system.control.custom_thing', false);
    const ex = c.executor;
    ex.setApprovalHandler({
      requestApproval: () => {
        throw new Error('governanceState must not invoke the handler');
      },
      checkApproval: () => {
        throw new Error('governanceState must not invoke the handler');
      },
    });

    const state = ex.governanceState();
    expect(state.approvalHandlerConfigured).toBe(true);
    expect(state.builtinApprovalGateWired).toBe(true);
    expect(state.allControlModulesRequireApproval).toBe(false);
    // The v1.15.0 formula answered false here — a gate that is not there.
    expect(state.unprotectedControlSurface).toBe(true);
  });

  it('strict policy does not gate an unannotated module either', () => {
    const c = client(SYS_ON);
    registerControlModule(c, 'system.control.custom_thing', false);
    const ex = c.executor;
    ex.setPolicy(new ExecutionPolicy([], { strict: true }));

    const state = ex.governanceState();
    expect(state.policyStrict).toBe(true);
    expect(state.allControlModulesRequireApproval).toBe(false);
    expect(state.unprotectedControlSurface).toBe(true);
  });

  it('all annotated with a handler attached is gated', () => {
    const c = client(SYS_ON);
    registerControlModule(c, 'system.control.custom_thing', true);
    const ex = c.executor;
    ex.setApprovalHandler({
      requestApproval: () => {
        throw new Error('not invoked');
      },
      checkApproval: () => {
        throw new Error('not invoked');
      },
    });

    const state = ex.governanceState();
    expect(state.allControlModulesRequireApproval).toBe(true);
    expect(state.unprotectedControlSurface).toBe(false);
  });
});

describe('governanceState — accessor contract', () => {
  it('is a pure read', () => {
    const ex = client(SYS_ON).executor;
    expect(ex.governanceState()).toEqual(ex.governanceState());
  });

  it('is live, not cached', () => {
    const ex = client(SYS_ON).executor;
    const before = ex.governanceState();
    ex.setAcl(denyAll());
    const after = ex.governanceState();
    expect(before.aclConfigured).toBe(false);
    expect(after.aclConfigured).toBe(true);
  });

  it('returns booleans only — no ACL, handler or policy leaks out', () => {
    const state = client(SYS_ON).executor.governanceState();
    for (const [field, value] of Object.entries(state)) {
      expect(typeof value, `${field}`).toBe('boolean');
    }
  });
});
