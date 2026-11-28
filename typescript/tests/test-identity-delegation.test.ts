import { describe, expect, it, vi } from 'vitest';
import { ACL, auditEntryToWire, type ACLRule } from '../src/acl.js';
import {
  Context, createAnnotations, createApprovalResult, createIdentity, Executor, Middleware, Registry,
  STREAMING_MARKER, type ApprovalHandler, type ApprovalRequest, type PolicyEvaluationRequest,
} from '../src/index.js';
import { extractAuditIdentity } from '../src/sys-modules/audit.js';

const moduleId = 'delegation.target';
const permit: ACLRule = {
  callers: ['@external'], targets: ['delegation.*'], effect: 'allow', description: 'Grant scope',
  conditions: { scopes: ['read'] },
};

function actorContext(subject = 'alice', scopes: readonly string[] = ['read']): Context {
  const root = createIdentity(subject, 'user', ['reviewer'], { tenant: { name: 'example' } }, { scopes: ['read', 'write'] });
  return Context.create(root.delegate(createIdentity('worker', 'agent', ['automation']), scopes));
}

async function setup() {
  const registry = new Registry();
  const execute = vi.fn(() => ({ done: true }));
  const stream = vi.fn(async function* () { yield { done: true }; });
  const preview = vi.fn(() => null);
  for (const id of [moduleId, 'delegation.other']) {
    await registry.register(id, {
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      description: 'Task',
      annotations: createAnnotations({ streaming: true, discoverable: true }),
      [STREAMING_MARKER]: true,
      execute, stream, preview,
    });
  }
  return { registry, execute, stream, preview };
}

async function collect(chunks: AsyncGenerator<Record<string, unknown>>): Promise<Record<string, unknown>[]> {
  const results: Record<string, unknown>[] = [];
  for await (const chunk of chunks) results.push(chunk);
  return results;
}

describe('scoped identity delegation', () => {
  it('keeps legacy, empty, and delegated authority distinct across context restoration', () => {
    const legacy = createIdentity('alice');
    const root = createIdentity('alice', 'user', ['reviewer'], {}, { scopes: ['write', 'read'], issuer: 'issuer' });
    const actor = createIdentity('worker', 'agent', ['automation'], {}, { issuer: 'issuer' });
    const leaf = root.delegate(actor, ['read']);
    const empty = root.delegate(actor, []);

    expect(legacy.authorityKey()).toBeNull();
    expect(actor.authorityKey()).toBeNull();
    expect(actor.ownerKey()).toBeNull();
    expect(leaf.rootSubject).toBe(root);
    expect(leaf.delegationDepth).toBe(1);
    expect(empty.ownerKey()).toBe(leaf.ownerKey());
    expect(empty.authorityKey()).not.toBe(leaf.authorityKey());
    expect(Object.isFrozen(leaf.scopes)).toBe(true);
    expect(Context.fromJSON(Context.create(leaf).toJSON()).identity?.authorityKey()).toBe(leaf.authorityKey());
    expect(Context.create(legacy).toJSON().identity).not.toHaveProperty('scopes');
    expect(() => root.delegate(actor, ['admin'])).toThrow();
    expect(() => root.delegate(leaf, ['read'])).toThrow();
    const restricted = createIdentity('worker', 'agent', [], {}, { scopes: [] });
    expect(() => root.delegate(restricted, ['read'])).toThrow();
    expect(() => root.delegate(actor, ['not\u00a0a.scope'])).toThrow();
    expect(() => legacy.delegate(actor, [])).toThrow();

    const wire = Context.create(leaf).toJSON();
    expect(() => Context.fromJSON({ ...wire, identity: { ...(wire.identity as object), scopes: ['admin'] } })).toThrow();
    let nested = leaf;
    for (let depth = 2; depth <= 8; depth++) nested = nested.delegate(actor, ['read']);
    expect(nested.delegationDepth).toBe(8);
    expect(() => nested.delegate(actor, ['read'])).toThrow();
    const cycle: Record<string, unknown> = { id: 'cycle', type: 'agent', scopes: ['read'] };
    cycle.on_behalf_of = cycle;
    expect(() => Context.fromJSON({ identity: cycle })).toThrow();
  });

  it('requires the first matching ACL grant to prove all scopes and uses the root subject separately', async () => {
    const root = createIdentity('alice', 'user', ['reviewer'], {}, { scopes: ['read', 'write'] });
    const leaf = root.delegate(createIdentity('worker', 'agent', ['automation']), ['read']);
    const context = Context.create(leaf);
    const allow = { callers: ['@external'], targets: ['*'], effect: 'allow', description: 'grant' };
    const conditions = {
      scopes: ['read'], identity_types: ['agent'], roles: ['automation'],
      subject_types: ['user'], subject_roles: ['reviewer'], max_delegation_depth: 1,
    };
    const rejecting = [
      new ACL([], 'allow'),
      new ACL([allow, { ...allow, conditions }]),
      new ACL([{ ...allow, conditions: { $or: [{ scopes: ['read'] }] } }]),
      new ACL([{ ...allow, conditions: { ...conditions, scopes: ['read', 'write'] } }], 'allow'),
    ];
    for (const acl of rejecting) {
      expect(acl.check(null, 'fixture.target', context)).toBe(false);
      expect(await acl.asyncCheck(null, 'fixture.target', context)).toBe(false);
      expect(acl.evaluateVisibility(null, 'fixture.target', context).visible).toBe(false);
    }
    const acl = new ACL([{ ...allow, conditions }]);
    expect(acl.check(null, 'fixture.target', context)).toBe(true);
    expect(await acl.asyncCheck(null, 'fixture.target', context)).toBe(true);
    expect(acl.evaluateVisibility(null, 'fixture.target', context).visible).toBe(true);
  });

  it('enforces a stock scope gate for call, stream, preflight, and capability discovery', async () => {
    const { registry, execute, preview, stream } = await setup();
    const context = actorContext();
    class CustomAcl extends ACL {
      override asyncCheckAccess = vi.fn(async () => ({
        access: 'allow' as const, approvalRequired: false, matchedRuleIndex: null, reason: 'custom',
      }));
    }
    for (const options of [{}, { strategy: 'minimal' }, { acl: new CustomAcl([permit]) }]) {
      const executor = new Executor({ registry, ...options });
      await expect(executor.call(moduleId, {}, context)).rejects.toMatchObject({ code: 'ACL_DENIED' });
      await expect(collect(executor.stream(moduleId, {}, context))).rejects.toMatchObject({ code: 'ACL_DENIED' });
      expect(await executor.validate(moduleId, {}, context)).toMatchObject({ valid: false });
      expect(executor.listCallable(context).items).toEqual([]);
    }
    expect(execute).not.toHaveBeenCalled();
    expect(stream).not.toHaveBeenCalled();
    expect(preview).not.toHaveBeenCalled();

    const executor = new Executor({ registry, acl: new ACL([permit]) });
    expect(await executor.call(moduleId, {}, context)).toEqual({ done: true });
    expect(await collect(executor.stream(moduleId, {}, context))).toEqual([{ done: true }]);
    expect(await executor.validate(moduleId, {}, context)).toMatchObject({ valid: true });
    const first = executor.listCallable(context, { limit: 1 });
    expect(first.nextCursor).not.toBeNull();
    const options = { limit: 1, cursor: first.nextCursor };
    expect(executor.listCallable(actorContext(), options).items).toHaveLength(1);
    expect(() => executor.listCallable(actorContext('alice', ['read', 'write']), options)).toThrow('Invalid or expired');
    expect(() => executor.listCallable(actorContext('bob'), options)).toThrow('Invalid or expired');
  });

  it('passes detached delegation into policy and binds pending approval to current authority', async () => {
    const { registry, execute } = await setup();
    const context = actorContext();
    const evaluate = vi.fn(async (_request: PolicyEvaluationRequest) => ({ decision: 'require_approval' as const }));
    let pending: ApprovalRequest | undefined;
    const resume = vi.fn(async (_token: string, request: ApprovalRequest) => {
      if (pending === undefined || pending.authorityKey !== request.authorityKey ||
          pending.canonicalArgsDigest !== request.canonicalArgsDigest) {
        return createApprovalResult({ status: 'rejected' });
      }
      pending = undefined;
      return createApprovalResult({ status: 'approved' });
    });
    const handler: ApprovalHandler = {
      supportsAuthorityScope: true,
      supportsFinalScope: true,
      requestApproval: async (request) => {
        pending = request;
        return createApprovalResult({ status: 'pending', approvalId: 'credential' });
      },
      checkApproval: vi.fn(),
      checkApprovalForCall: resume,
    };
    const executor = new Executor({
      registry, acl: new ACL([permit]), approvalHandler: handler, policyEvaluator: { evaluate },
    });
    await expect(executor.call(moduleId, {}, context)).rejects.toMatchObject({ code: 'APPROVAL_PENDING' });
    expect(pending?.authorityKey).toBe(context.identity?.authorityKey());
    const detached = evaluate.mock.calls[0][0].identity;
    expect(detached?.rootSubject.id).toBe(context.identity?.rootSubject.id);
    expect(detached?.scopes).toEqual(context.identity?.scopes);
    expect(detached?.rootSubject.attrs.tenant).not.toBe(context.identity?.rootSubject.attrs.tenant);
    const credentials = { _approval_token: 'credential' };
    await expect(executor.call(moduleId, credentials, actorContext('alice', ['read', 'write']))).rejects
      .toMatchObject({ code: 'APPROVAL_DENIED' });
    expect(pending).toBeDefined();
    expect(execute).not.toHaveBeenCalled();
    expect(await executor.call(moduleId, credentials, context)).toEqual({ done: true });
    expect(resume).toHaveBeenCalledTimes(2);
    expect(pending).toBeUndefined();
  });

  it.each(['acl_check', 'execute'])('keeps scoped dispatch bound to stock lookup before %s', async (step) => {
    const { registry } = await setup();
    const replacement = registry.get('delegation.other');
    const executor = new Executor({ registry, acl: new ACL([permit]) });
    const recover = vi.fn(async () => ({ recovered: true }));
    class RecoveringMiddleware extends Middleware {
      override onError = recover;
    }
    executor.use(new RecoveringMiddleware());
    executor.currentStrategy.insertBefore(step, {
      name: 'substitute_registration',
      description: 'Substitute the selected module',
      removable: true,
      replaceable: true,
      execute: async ctx => {
        ctx.module = replacement;
        return { action: 'continue' };
      },
    });
    await expect(executor.call(moduleId, {}, actorContext())).rejects.toMatchObject({ code: 'ACL_DENIED' });
    await expect(collect(executor.stream(moduleId, {}, actorContext()))).rejects.toMatchObject({ code: 'ACL_DENIED' });
    expect(recover).not.toHaveBeenCalled();
  });

  it('rejects pending scoped approval and skips resumption when a handler lacks authority binding', async () => {
    const { registry, execute } = await setup();
    const context = actorContext();
    const resume = vi.fn(async () => createApprovalResult({ status: 'approved' }));
    const executor = new Executor({
      registry,
      acl: new ACL([{ ...permit, approval: 'required' }]),
      approvalHandler: {
        requestApproval: async () => createApprovalResult({ status: 'pending', approvalId: 'unbound' }),
        checkApproval: vi.fn(),
        checkApprovalForCall: resume,
      },
    });
    await expect(executor.call(moduleId, {}, context)).rejects.toMatchObject({ code: 'APPROVAL_DENIED' });
    await expect(executor.call(moduleId, { _approval_token: 'unbound' }, context)).rejects
      .toMatchObject({ code: 'APPROVAL_DENIED' });
    expect(resume).not.toHaveBeenCalled();
    expect(execute).not.toHaveBeenCalled();
  });

  it('attributes ACL decisions and redacts nested contextual audit credentials', async () => {
    const root = createIdentity('alice', 'user', [], { profile: { access_token: 'raw-root', region: 'eu' } }, { scopes: ['read'] });
    const actor = createIdentity('worker', 'agent', [], { api_key: 'raw-actor', subject_id: 'forged' });
    const context = Context.create(root.delegate(actor, ['read']));
    const audit = vi.fn();
    const acl = new ACL([permit], 'deny', audit);
    expect(await acl.asyncCheck(null, moduleId, context)).toBe(true);
    expect(auditEntryToWire(audit.mock.calls[0][0])).toMatchObject({
      actor_id: 'worker', subject_id: 'alice', delegation_depth: 1,
    });
    expect(extractAuditIdentity(context).identity).toMatchObject({
      actor_id: 'worker', subject_id: 'alice', delegation_depth: 1, api_key: '<redacted>',
      on_behalf_of: { id: 'alice', profile: { access_token: '<redacted>', region: 'eu' } },
    });
  });
});
