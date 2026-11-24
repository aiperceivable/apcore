/**
 * Tests for the execution-time governance policy (apcore#76 RFC pilot).
 *
 * Covers PolicyRule validation, ExecutionPolicy.fromObject strict parsing,
 * resolve() precedence/specificity semantics, the approval-gate integration
 * (policy overrides, gateDestructive, strict fail-closed, fail-loud warnings),
 * and the governance events on the event bus (apcore#77).
 *
 * Ported from apcore-python tests/test_policy.py.
 */

import { describe, expect, it, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { ACL } from '../src/acl.js';
import { AlwaysDenyHandler, AutoApproveHandler, createApprovalResult } from '../src/approval.js';
import type { ApprovalRequest, ApprovalResult } from '../src/approval.js';
import { Context } from '../src/context.js';
import { ACLDeniedError, ApprovalDeniedError } from '../src/errors.js';
import type { ApCoreEvent, EventEmitter } from '../src/events/emitter.js';
import { Executor } from '../src/executor.js';
import { createAnnotations } from '../src/module.js';
import type { ModuleAnnotations } from '../src/module.js';
import { ExecutionPolicy, PolicyRule } from '../src/policy.js';
import { Registry } from '../src/registry/registry.js';

// ---------------------------------------------------------------------------
// Test module implementations
// ---------------------------------------------------------------------------

const PermissiveInput = Type.Object({}, { additionalProperties: true });
const PermissiveOutput = Type.Object({}, { additionalProperties: true });

function makeModule(annotations: ModuleAnnotations, description = ''): Record<string, unknown> {
  return {
    inputSchema: PermissiveInput,
    outputSchema: PermissiveOutput,
    annotations,
    description,
    execute: (_inputs: Record<string, unknown>) => ({ status: 'executed' }),
  };
}

/** Handler that records requests and returns a fixed decision. */
class RecordingHandler {
  requests: ApprovalRequest[] = [];
  private readonly _status: ApprovalResult['status'];

  constructor(status: ApprovalResult['status'] = 'approved') {
    this._status = status;
  }

  async requestApproval(request: ApprovalRequest): Promise<ApprovalResult> {
    this.requests.push(request);
    return createApprovalResult({ status: this._status });
  }

  async checkApproval(_approvalId: string): Promise<ApprovalResult> {
    return createApprovalResult({ status: 'rejected', reason: 'not supported' });
  }
}

/** Duck-typed EventEmitter that records events synchronously. */
class CollectingEmitter {
  events: ApCoreEvent[] = [];

  emit(event: ApCoreEvent): void {
    this.events.push(event);
  }

  ofType(eventType: string): ApCoreEvent[] {
    return this.events.filter((e) => e.eventType === eventType);
  }
}

function asEmitter(collector: CollectingEmitter): EventEmitter {
  return collector as unknown as EventEmitter;
}

function makeRegistry(): Registry {
  const reg = new Registry();
  reg.register('orders.list_orders', makeModule(createAnnotations()));
  reg.register(
    'orders.delete_order',
    makeModule(createAnnotations({ destructive: true }), 'Destructive but ungated'),
  );
  reg.register(
    'admin.reset',
    makeModule(createAnnotations({ requiresApproval: true }), 'Gated operation'),
  );
  return reg;
}

// ---------------------------------------------------------------------------
// PolicyRule validation
// ---------------------------------------------------------------------------

describe('PolicyRule', () => {
  it('accepts a valid rule', () => {
    const rule = new PolicyRule('orders.*', { requiresApproval: true, reason: 'ops sign-off' });
    expect(rule.pattern).toBe('orders.*');
    expect(rule.requiresApproval).toBe(true);
    expect(rule.destructive).toBeNull();
  });

  it('rejects an empty pattern', () => {
    expect(() => new PolicyRule('')).toThrow(/pattern/);
  });

  it('rejects a non-string pattern', () => {
    expect(() => new PolicyRule(null as unknown as string)).toThrow(/pattern/);
  });

  it('rejects a non-boolean override', () => {
    expect(() => new PolicyRule('a.b', { requiresApproval: 'yes' as unknown as boolean })).toThrow(
      /requiresApproval/,
    );
    expect(() => new PolicyRule('a.b', { destructive: 1 as unknown as boolean })).toThrow(
      /destructive/,
    );
  });

  it('rejects a non-string reason', () => {
    expect(() => new PolicyRule('a.b', { reason: 42 as unknown as string })).toThrow(/reason/);
  });
});

// ---------------------------------------------------------------------------
// ExecutionPolicy.fromObject — strict parsing (fail loud on typos)
// ---------------------------------------------------------------------------

describe('ExecutionPolicy.fromObject', () => {
  it('parses a full document', () => {
    const policy = ExecutionPolicy.fromObject({
      gate_destructive: true,
      strict: true,
      rules: [
        { pattern: 'orders.delete_*', requires_approval: true, reason: 'sign-off' },
        { pattern: 'reports.*', destructive: false },
      ],
    });
    expect(policy.gateDestructive).toBe(true);
    expect(policy.strict).toBe(true);
    expect(policy.rules).toHaveLength(2);
    expect(policy.rules[0].pattern).toBe('orders.delete_*');
  });

  it('parses an empty document', () => {
    const policy = ExecutionPolicy.fromObject({});
    expect(policy.rules).toHaveLength(0);
    expect(policy.gateDestructive).toBe(false);
    expect(policy.strict).toBe(false);
  });

  it('rejects a non-mapping', () => {
    expect(() => ExecutionPolicy.fromObject(['not', 'a', 'dict'])).toThrow(/mapping/);
  });

  it('rejects an unknown policy key', () => {
    expect(() => ExecutionPolicy.fromObject({ gate_destructiv: true })).toThrow(/gate_destructiv/);
  });

  it('rejects rules that are not a list', () => {
    expect(() => ExecutionPolicy.fromObject({ rules: { pattern: 'a.b' } })).toThrow(/list/);
  });

  it('rejects a rule that is not a mapping', () => {
    expect(() => ExecutionPolicy.fromObject({ rules: ['a.b'] })).toThrow(/rule #0/);
  });

  it('rejects an unknown rule key', () => {
    expect(() =>
      ExecutionPolicy.fromObject({ rules: [{ pattern: 'a.b', require_approval: true }] }),
    ).toThrow(/require_approval/);
  });

  it('rejects a rule missing pattern', () => {
    expect(() => ExecutionPolicy.fromObject({ rules: [{ requires_approval: true }] })).toThrow(
      /pattern/,
    );
  });

  it('rejects a non-PolicyRule in the constructor', () => {
    expect(() => new ExecutionPolicy([{ pattern: 'a.b' } as unknown as PolicyRule])).toThrow(
      /PolicyRule/,
    );
  });
});

// ---------------------------------------------------------------------------
// ExecutionPolicy.resolve — precedence and specificity
// ---------------------------------------------------------------------------

describe('ExecutionPolicy.resolve', () => {
  it('passes annotations through when there are no rules', () => {
    const policy = new ExecutionPolicy();
    const decision = policy.resolve(
      'a.b',
      createAnnotations({ requiresApproval: true, destructive: true }),
    );
    expect(decision).toEqual({
      moduleId: 'a.b',
      requiresApproval: true,
      destructive: true,
      needsApproval: true,
      rule: null,
      overridden: false,
    });
  });

  it('supports dict annotations', () => {
    const decision = new ExecutionPolicy().resolve('a.b', { requires_approval: true });
    expect(decision.requiresApproval).toBe(true);
    expect(decision.needsApproval).toBe(true);
  });

  it('supports null annotations', () => {
    const decision = new ExecutionPolicy().resolve('a.b', null);
    expect(decision.needsApproval).toBe(false);
  });

  it('forces approval via a rule', () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.delete_*', { requiresApproval: true }),
    ]);
    const decision = policy.resolve('orders.delete_order', createAnnotations());
    expect(decision.requiresApproval).toBe(true);
    expect(decision.needsApproval).toBe(true);
    expect(decision.overridden).toBe(true);
    expect(decision.rule?.pattern).toBe('orders.delete_*');
  });

  it('exempts approval via a rule', () => {
    const policy = new ExecutionPolicy([new PolicyRule('batch.*', { requiresApproval: false })]);
    const decision = policy.resolve('batch.cleanup', createAnnotations({ requiresApproval: true }));
    expect(decision.requiresApproval).toBe(false);
    expect(decision.needsApproval).toBe(false);
    expect(decision.overridden).toBe(true);
  });

  it('does not override with null fields', () => {
    const policy = new ExecutionPolicy([new PolicyRule('a.*', { destructive: true })]);
    const decision = policy.resolve('a.b', createAnnotations({ requiresApproval: true }));
    expect(decision.requiresApproval).toBe(true); // untouched
    expect(decision.destructive).toBe(true); // overridden
    expect(decision.overridden).toBe(true);
  });

  it('ignores an unmatched rule', () => {
    const policy = new ExecutionPolicy([new PolicyRule('other.*', { requiresApproval: true })]);
    const decision = policy.resolve('a.b', createAnnotations());
    expect(decision.rule).toBeNull();
    expect(decision.needsApproval).toBe(false);
    expect(decision.overridden).toBe(false);
  });

  it('lets the most specific rule win', () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('*', { requiresApproval: true }),
      new PolicyRule('orders.list_orders', { requiresApproval: false }),
    ]);
    expect(policy.resolve('orders.list_orders', createAnnotations()).needsApproval).toBe(false);
    expect(policy.resolve('orders.delete_order', createAnnotations()).needsApproval).toBe(true);
  });

  it('breaks a specificity tie toward the more restrictive rule', () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.*', { requiresApproval: false }),
      new PolicyRule('orders.*', { requiresApproval: true }),
    ]);
    expect(policy.resolve('orders.delete_order', createAnnotations()).needsApproval).toBe(true);
  });

  it('gates destructive modules when gateDestructive is set', () => {
    const policy = new ExecutionPolicy(null, { gateDestructive: true });
    const decision = policy.resolve(
      'orders.delete_order',
      createAnnotations({ destructive: true }),
    );
    expect(decision.requiresApproval).toBe(false);
    expect(decision.needsApproval).toBe(true);
  });

  it('gates destructive via a rule override + gateDestructive', () => {
    const policy = new ExecutionPolicy([new PolicyRule('orders.delete_*', { destructive: true })], {
      gateDestructive: true,
    });
    const decision = policy.resolve('orders.delete_order', createAnnotations());
    expect(decision.destructive).toBe(true);
    expect(decision.needsApproval).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Approval gate integration (Executor end-to-end)
// ---------------------------------------------------------------------------

describe('Approval gate with policy', () => {
  it('policy forces approval — handler is consulted', async () => {
    const handler = new RecordingHandler('approved');
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.delete_*', { requiresApproval: true, reason: 'sign-off' }),
    ]);
    const executor = new Executor({ registry: makeRegistry(), approvalHandler: handler, policy });
    const result = await executor.call('orders.delete_order');
    expect(result['status']).toBe('executed');
    expect(handler.requests).toHaveLength(1);
    expect(handler.requests[0].moduleId).toBe('orders.delete_order');
  });

  it('policy forces approval — deny blocks', async () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.delete_*', { requiresApproval: true }),
    ]);
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: new AlwaysDenyHandler(),
      policy,
    });
    await expect(executor.call('orders.delete_order')).rejects.toThrow(ApprovalDeniedError);
  });

  it('policy exempts a module — handler is not consulted', async () => {
    const handler = new RecordingHandler('rejected');
    const policy = new ExecutionPolicy([
      new PolicyRule('admin.reset', { requiresApproval: false }),
    ]);
    const executor = new Executor({ registry: makeRegistry(), approvalHandler: handler, policy });
    const result = await executor.call('admin.reset');
    expect(result['status']).toBe('executed');
    expect(handler.requests).toHaveLength(0);
  });

  it('strict policy fails closed without a handler', async () => {
    const policy = new ExecutionPolicy(null, { strict: true });
    const executor = new Executor({ registry: makeRegistry(), policy });
    await expect(executor.call('admin.reset')).rejects.toThrow(/fails closed/);
  });

  it('strict policy without gated modules still executes', async () => {
    const policy = new ExecutionPolicy(null, { strict: true });
    const executor = new Executor({ registry: makeRegistry(), policy });
    const result = await executor.call('orders.list_orders');
    expect(result['status']).toBe('executed');
  });

  it('warns once and skips when a handler is missing (fail-loud default)', async () => {
    const executor = new Executor({ registry: makeRegistry() });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      expect((await executor.call('admin.reset'))['status']).toBe('executed');
      expect((await executor.call('admin.reset'))['status']).toBe('executed');
      const warnings = warnSpy.mock.calls.filter(
        (args) =>
          typeof args[0] === 'string' && args[0].includes('no ApprovalHandler is configured'),
      );
      expect(warnings).toHaveLength(1);
      expect(warnings[0][0]).toContain('admin.reset');
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('warns once about a destructive-ungated module', async () => {
    const executor = new Executor({ registry: makeRegistry() });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await executor.call('orders.delete_order');
      await executor.call('orders.delete_order');
      const warnings = warnSpy.mock.calls.filter(
        (args) => typeof args[0] === 'string' && args[0].includes('destructive=true'),
      );
      expect(warnings).toHaveLength(1);
      expect(warnings[0][0]).toContain('orders.delete_order');
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('does not warn about a non-destructive module', async () => {
    const executor = new Executor({ registry: makeRegistry() });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await executor.call('orders.list_orders');
      expect(warnSpy.mock.calls).toHaveLength(0);
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('gateDestructive covers a destructive module', async () => {
    const handler = new RecordingHandler('approved');
    const policy = new ExecutionPolicy(null, { gateDestructive: true });
    const executor = new Executor({ registry: makeRegistry(), approvalHandler: handler, policy });
    const result = await executor.call('orders.delete_order');
    expect(result['status']).toBe('executed');
    expect(handler.requests).toHaveLength(1);
  });

  it('set policy at runtime', async () => {
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: new AlwaysDenyHandler(),
    });
    expect((await executor.call('orders.delete_order'))['status']).toBe('executed');

    executor.setPolicy(
      new ExecutionPolicy([new PolicyRule('orders.delete_*', { requiresApproval: true })]),
    );
    await expect(executor.call('orders.delete_order')).rejects.toThrow(ApprovalDeniedError);

    executor.setPolicy(null);
    expect((await executor.call('orders.delete_order'))['status']).toBe('executed');
  });

  it('policy-gated request upholds the annotations contract', async () => {
    const handler = new RecordingHandler('approved');
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.delete_*', { requiresApproval: true }),
    ]);
    const executor = new Executor({ registry: makeRegistry(), approvalHandler: handler, policy });
    await executor.call('orders.delete_order');
    expect(handler.requests).toHaveLength(1);
    const annotations = handler.requests[0].annotations;
    expect(annotations.requiresApproval).toBe(true); // effective, not the raw false
    expect(annotations.destructive).toBe(true); // module's own value untouched
  });

  it('gateDestructive request upholds the annotations contract', async () => {
    const handler = new RecordingHandler('approved');
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: handler,
      policy: new ExecutionPolicy(null, { gateDestructive: true }),
    });
    await executor.call('orders.delete_order');
    expect(handler.requests).toHaveLength(1);
    expect(handler.requests[0].annotations.requiresApproval).toBe(true);
    expect(handler.requests[0].annotations.destructive).toBe(true);
  });

  it('policy destructive override is visible to the handler', async () => {
    const handler = new RecordingHandler('approved');
    const policy = new ExecutionPolicy([new PolicyRule('admin.reset', { destructive: true })]);
    const executor = new Executor({ registry: makeRegistry(), approvalHandler: handler, policy });
    await executor.call('admin.reset');
    expect(handler.requests).toHaveLength(1);
    expect(handler.requests[0].annotations.requiresApproval).toBe(true);
    expect(handler.requests[0].annotations.destructive).toBe(true); // policy-effective
  });

  it('fromRegistry accepts a policy', async () => {
    const policy = new ExecutionPolicy([new PolicyRule('admin.*', { requiresApproval: false })]);
    const handler = new RecordingHandler('rejected');
    const executor = Executor.fromRegistry(makeRegistry(), null, null, null, handler, policy);
    expect((await executor.call('admin.reset'))['status']).toBe('executed');
    expect(handler.requests).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// Policy-override audit (span event)
// ---------------------------------------------------------------------------

describe('Policy override audit', () => {
  it('records a policy_override span event', async () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.delete_*', { requiresApproval: true, reason: 'sign-off' }),
    ]);
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: new AutoApproveHandler(),
      policy,
    });
    const ctx = Context.create();
    const spansStack = [{ events: [] as Record<string, unknown>[] }];
    ctx.data['_apcore.mw.tracing.spans'] = spansStack;
    await executor.call('orders.delete_order', {}, ctx);
    const overrides = spansStack[0].events.filter((e) => e['name'] === 'policy_override');
    expect(overrides).toHaveLength(1);
    expect(overrides[0]['module_id']).toBe('orders.delete_order');
    expect(overrides[0]['reason']).toBe('sign-off');
  });

  it('records no policy_override span event when the policy does not override', async () => {
    const policy = new ExecutionPolicy([new PolicyRule('other.*', { requiresApproval: true })]);
    const executor = new Executor({ registry: makeRegistry(), policy });
    const ctx = Context.create();
    const spansStack = [{ events: [] as Record<string, unknown>[] }];
    ctx.data['_apcore.mw.tracing.spans'] = spansStack;
    await executor.call('orders.list_orders', {}, ctx);
    expect(spansStack[0].events.filter((e) => e['name'] === 'policy_override')).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// Governance events on the event bus (apcore#77)
// ---------------------------------------------------------------------------

describe('Governance events', () => {
  it('emits an approved decision event', async () => {
    const emitter = new CollectingEmitter();
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: new AutoApproveHandler(),
      eventEmitter: asEmitter(emitter),
    });
    await executor.call('admin.reset');
    const events = emitter.ofType('apcore.approval.decision');
    expect(events).toHaveLength(1);
    expect(events[0].moduleId).toBe('admin.reset');
    expect(events[0].severity).toBe('info');
    expect(events[0].data['status']).toBe('approved');
    expect(events[0].data['approved_by']).toBe('auto');
    expect(events[0].data['trace_id']).toBeTruthy();
  });

  it('emits a rejected decision event with warn severity', async () => {
    const emitter = new CollectingEmitter();
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: new AlwaysDenyHandler(),
      eventEmitter: asEmitter(emitter),
    });
    await expect(executor.call('admin.reset')).rejects.toThrow(ApprovalDeniedError);
    const events = emitter.ofType('apcore.approval.decision');
    expect(events).toHaveLength(1);
    expect(events[0].severity).toBe('warn');
    expect(events[0].data['status']).toBe('rejected');
  });

  it('emits a decision event on strict fail-closed', async () => {
    const emitter = new CollectingEmitter();
    const executor = new Executor({
      registry: makeRegistry(),
      policy: new ExecutionPolicy(null, { strict: true }),
      eventEmitter: asEmitter(emitter),
    });
    await expect(executor.call('admin.reset')).rejects.toThrow(ApprovalDeniedError);
    const events = emitter.ofType('apcore.approval.decision');
    expect(events).toHaveLength(1);
    expect(events[0].severity).toBe('warn');
    expect(events[0].data['status']).toBe('rejected');
  });

  it('emits a policy override event', async () => {
    const emitter = new CollectingEmitter();
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.delete_*', { requiresApproval: true, reason: 'sign-off' }),
    ]);
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: new AutoApproveHandler(),
      policy,
      eventEmitter: asEmitter(emitter),
    });
    await executor.call('orders.delete_order');
    const overrides = emitter.ofType('apcore.policy.override');
    expect(overrides).toHaveLength(1);
    const data = overrides[0].data;
    expect(data['module_id']).toBe('orders.delete_order');
    expect(data['pattern']).toBe('orders.delete_*');
    expect(data['requires_approval']).toBe(true);
    expect(data['needs_approval']).toBe(true);
    expect(data['reason']).toBe('sign-off');
    expect(emitter.ofType('apcore.approval.decision')).toHaveLength(1);
  });

  it('emits no events when the gate is not involved', async () => {
    const emitter = new CollectingEmitter();
    const executor = new Executor({
      registry: makeRegistry(),
      approvalHandler: new AutoApproveHandler(),
      eventEmitter: asEmitter(emitter),
    });
    await executor.call('orders.list_orders');
    expect(emitter.ofType('apcore.approval.decision')).toHaveLength(0);
    expect(emitter.ofType('apcore.policy.override')).toHaveLength(0);
  });

  it('emits no decision event when the gate is skipped without a handler', async () => {
    const emitter = new CollectingEmitter();
    const executor = new Executor({ registry: makeRegistry(), eventEmitter: asEmitter(emitter) });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await executor.call('admin.reset');
    } finally {
      warnSpy.mockRestore();
    }
    expect(emitter.ofType('apcore.approval.decision')).toHaveLength(0);
  });

  it('emits an acl.denied event', async () => {
    const emitter = new CollectingEmitter();
    const executor = new Executor({
      registry: makeRegistry(),
      acl: new ACL([], 'deny'),
      eventEmitter: asEmitter(emitter),
    });
    await expect(executor.call('orders.list_orders')).rejects.toThrow(ACLDeniedError);
    const events = emitter.ofType('apcore.acl.denied');
    expect(events).toHaveLength(1);
    expect(events[0].severity).toBe('warn');
    expect(events[0].data['module_id']).toBe('orders.list_orders');
    expect('caller_id' in events[0].data).toBe(true);
    expect(events[0].data['trace_id']).toBeTruthy();
  });

  it('does not emit an acl.denied event in preflight', async () => {
    const emitter = new CollectingEmitter();
    const executor = new Executor({
      registry: makeRegistry(),
      acl: new ACL([], 'deny'),
      eventEmitter: asEmitter(emitter),
    });
    await executor.validate('orders.list_orders', {});
    expect(emitter.ofType('apcore.acl.denied')).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// validate() preflight reflects policy
// ---------------------------------------------------------------------------

describe('validate() with policy', () => {
  it('reports policy-forced approval', async () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.delete_*', { requiresApproval: true }),
    ]);
    const executor = new Executor({ registry: makeRegistry(), policy });
    const preflight = await executor.validate('orders.delete_order', {});
    expect(preflight.requiresApproval).toBe(true);
  });

  it('reports gateDestructive', async () => {
    const executor = new Executor({
      registry: makeRegistry(),
      policy: new ExecutionPolicy(null, { gateDestructive: true }),
    });
    const preflight = await executor.validate('orders.delete_order', {});
    expect(preflight.requiresApproval).toBe(true);
  });

  it('reports a policy exemption', async () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('admin.reset', { requiresApproval: false }),
    ]);
    const executor = new Executor({ registry: makeRegistry(), policy });
    const preflight = await executor.validate('admin.reset', {});
    expect(preflight.requiresApproval).toBe(false);
  });

  it('is unchanged without a policy', async () => {
    const executor = new Executor({ registry: makeRegistry() });
    expect((await executor.validate('admin.reset', {})).requiresApproval).toBe(true);
    expect((await executor.validate('orders.list_orders', {})).requiresApproval).toBe(false);
  });
});
