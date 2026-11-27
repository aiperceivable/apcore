import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import {
  ACL, APCore, Context, ExecutionPolicy, Executor, Registry, createAnnotations, createIdentity,
  type ACLRule, type CapabilitySearchOptions,
} from '../src/index.js';
import { Config, ExtensionManager, registerSysModules } from '../src/full.js';
import { findFixturesRoot } from './spec-repo.js';

interface DiscoveryCase {
  id: string;
  input: {
    rules: Array<Omit<ACLRule, 'description'>>;
    roles?: string[];
    caller_id?: string;
    policy_evaluator?: boolean;
    options?: { include_approval_required?: boolean; query?: string; tags?: string[]; prefix?: string };
  };
  expected: {
    items: Array<{ module_id: string; access_hint: string }>;
    definitions?: string[];
    audit_entries: number;
  };
}

const fixture = JSON.parse(readFileSync(join(findFixturesRoot(), 'capability_discovery.json'), 'utf8')) as {
  driver_contract: {
    modules: Array<{
      module_id: string; description: string; tags?: string[]; readonly?: boolean;
      discoverable?: boolean; requires_approval?: boolean;
    }>;
  };
  test_cases: DiscoveryCase[];
};

const inputSchema = {
  type: 'object', properties: { message: { type: 'string' } }, required: ['message'],
};
const outputSchema = { type: 'object' };

async function fixtureRegistry(): Promise<{ registry: Registry; execute: ReturnType<typeof vi.fn> }> {
  const registry = new Registry();
  const execute = vi.fn(async () => ({}));
  for (const module of fixture.driver_contract.modules) {
    await registry.register(module.module_id, {
      inputSchema, outputSchema, description: module.description,
      tags: module.tags ?? [],
      annotations: createAnnotations({
        discoverable: module.discoverable ?? true,
        readonly: module.readonly ?? false,
        requiresApproval: module.requires_approval ?? false,
      }),
      execute,
    }, '1.0.0');
  }
  return { registry, execute };
}

function requester(roles: string[] = [], id = 'reader', attrs = {}): Context {
  return Context.create(createIdentity(id, 'user', roles, attrs));
}

describe('capability_discovery.json', () => {
  it.each(fixture.test_cases)('$id', async ({ input, expected }) => {
    const { registry, execute } = await fixtureRegistry();
    const audit = vi.fn();
    const evaluate = vi.fn(async () => ({ decision: 'allow' as const }));
    const acl = new ACL(input.rules.map(rule => ({ description: '', ...rule })), 'deny', audit);
    const executor = new Executor({
      registry, acl, policyEvaluator: input.policy_evaluator ? { evaluate } : undefined,
    });
    const base = requester(input.roles);
    const context = input.caller_id ? base.child(input.caller_id) : base;
    const options: CapabilitySearchOptions = {
      ...input.options, includeApprovalRequired: input.options?.include_approval_required,
    };
    const page = executor.listCallable(context, options);
    expect(page.items.map(({ moduleId, accessHint }) => ({
      module_id: moduleId, access_hint: accessHint,
    }))).toEqual(expected.items);
    for (const summary of page.items) {
      expect(summary).not.toHaveProperty('inputSchema');
      expect(summary).not.toHaveProperty('outputSchema');
    }
    const definitions = executor.getCallableDefinitions([
      'demo.hidden', 'demo.echo', 'demo.review', 'demo.unknown', 'demo.echo',
    ], context);
    expect(definitions.map(row => row.moduleId)).toEqual(
      expected.definitions ?? expected.items.map(row => row.module_id).sort(),
    );
    for (const definition of definitions) {
      expect(definition).toMatchObject({ version: '1.0.0', inputSchema, outputSchema });
    }
    expect(audit).toHaveBeenCalledTimes(expected.audit_entries);
    expect(execute).not.toHaveBeenCalled();
    expect(evaluate).not.toHaveBeenCalled();
  });
});

describe('capability discovery lifecycle and identity', () => {
  it('reports approval only when the active gate enforces it', async () => {
    const { registry } = await fixtureRegistry();
    const policy = new ExecutionPolicy(null, { strict: false });
    const executor = new Executor({ registry, policy });
    const context = requester();

    expect(executor.listCallable(context).items.map(item => item.accessHint)).toEqual(['allowed', 'allowed']);
    await expect(executor.call('demo.review', { message: 'test' }, context)).resolves.toEqual({});

    const evaluate = vi.fn(async () => ({ decision: 'allow' as const }));
    executor.setPolicyEvaluator({ evaluate });
    expect(executor.listCallable(context).items.map(item => item.accessHint)).toEqual([
      'conditional', 'requires_approval',
    ]);
    expect(evaluate).not.toHaveBeenCalled();

    const withoutGate = new Executor({ registry });
    withoutGate.currentStrategy.remove('approval_gate');
    expect(withoutGate.listCallable(context).items.map(item => item.accessHint)).toEqual(['allowed', 'allowed']);
  });

  it('withholds discovery when the active strategy has no built-in ACL gate', async () => {
    const { registry } = await fixtureRegistry();
    const executor = new Executor({ registry, strategy: 'minimal' });
    expect(executor.listCallable(requester()).items).toEqual([]);
    expect(executor.getCallableDefinition('demo.echo', requester())).toBeNull();
  });

  it('scopes a continuation to principal, attrs, roles, parent frame, executor and filters', async () => {
    const { registry } = await fixtureRegistry();
    const client = new APCore({ registry });
    const context = requester(['reader'], 'user', { team: 'east' });
    const page = client.listCallable(context, { query: ' message ', limit: 1 });
    expect(page.items.map(item => item.moduleId)).toEqual(['demo.echo']);
    const options = { query: 'message', limit: 1, cursor: page.nextCursor! };
    expect(options.cursor).toEqual(expect.any(String));
    expect(client.listCallable(context, options).items.map(item => item.moduleId)).toEqual(['demo.review']);
    const badContexts = [
      requester(['reader'], 'another', { team: 'east' }),
      requester(['reader'], 'user', { team: 'west' }),
      requester(['writer'], 'user', { team: 'east' }),
      context.child('other.parent'),
    ];
    for (const invalid of badContexts) {
      expect(() => client.listCallable(invalid, options)).toThrow(/cursor/i);
    }
    expect(() => client.listCallable(context, { ...options, prefix: 'demo.' })).toThrow(/cursor/i);
    expect(() => new APCore({ registry }).listCallable(context, options)).toThrow(/cursor/i);
  });

  it('filters revoked, inactive and unknown candidates on every request and returns detached definitions', async () => {
    const { registry } = await fixtureRegistry();
    const search = vi.fn(() => ['demo.hidden', 'demo.echo', 'demo.review', 'demo.unknown', 'demo.echo']);
    const acl = new ACL([{ callers: ['@external'], targets: ['demo.*'], effect: 'allow', description: '' }]);
    const executor = new Executor({ registry, acl });
    const extensions = new ExtensionManager();
    extensions.register('capability_search_provider', { search });
    extensions.apply(registry, executor);
    const first = executor.listCallable(requester(), { limit: 1 });
    expect(first.items.map(item => item.moduleId)).toEqual(['demo.echo']);
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledWith(undefined, ['demo.echo', 'demo.review']);
    expect(executor.getCallableDefinition('demo.echo', requester(), '2.0.0')).toBeNull();
    const current = executor.getCallableDefinition('demo.echo', requester(), '1.0.0');
    expect(current?.inputSchema).toEqual(inputSchema);
    delete current!.inputSchema['properties'];
    expect(executor.getCallableDefinition('demo.echo', requester())?.inputSchema).toEqual(inputSchema);
    executor.toggleState.disable('demo.review');
    expect(executor.listCallable(requester(), { limit: 1, cursor: first.nextCursor! }).items).toEqual([]);
    expect(executor.getCallableDefinition('demo.review', requester())).toBeNull();
  });

  it('does not invoke custom conditions or disclose a candidate after the provider revokes ACL access', async () => {
    const { registry } = await fixtureRegistry();
    const evaluate = vi.fn(async () => false);
    ACL.registerAsyncCondition('discovery-test-custom', { evaluate });
    const broad = { callers: ['@external'], targets: ['demo.*'], effect: 'allow', description: '' };
    const acl = new ACL([{
      ...broad, targets: ['demo.review'], effect: 'deny', conditions: { 'discovery-test-custom': true },
    }, broad]);
    const search = vi.fn(() => {
      acl.removeRule(broad.callers, broad.targets);
      return ['demo.echo', 'demo.review'];
    });
    const executor = new Executor({ registry, acl, capabilitySearchProvider: { search } });
    expect(executor.listCallable(requester()).items).toEqual([]);
    expect(search).toHaveBeenCalledWith(undefined, ['demo.echo']);
    expect(evaluate).not.toHaveBeenCalled();
  });
});

it('authorizes a manifest query on its own ACL and uses only the originating caller for visibility', async () => {
  const { registry } = await fixtureRegistry();
  const acl = new ACL([
    { callers: ['@external'], targets: ['system.manifest.search', 'demo.echo'], effect: 'allow', description: '' },
    { callers: ['system.manifest.search'], targets: ['demo.review'], effect: 'allow', description: '' },
  ]);
  const executor = new Executor({ registry, acl });
  const config = Config.fromDefaults();
  config.set('sys_modules.enabled', true);
  registerSysModules(registry, executor, config);
  const canonical = JSON.parse(readFileSync(
    join(findFixturesRoot(), '..', '..', 'schemas', 'sys-manifest-search.schema.json'), 'utf8',
  )) as Record<string, unknown>;
  expect(registry.getDefinition('system.manifest.search')?.outputSchema).toEqual({
    type: canonical.type, additionalProperties: canonical.additionalProperties,
    required: canonical.required, properties: canonical.properties,
  });
  const result = await executor.call('system.manifest.search', { prefix: 'demo.' }, requester());
  expect(result).toMatchObject({ items: [{ module_id: 'demo.echo', access_hint: 'allowed' }], next_cursor: null });
  await expect(executor.call('system.manifest.search', { cursor: null }, requester())).rejects.toMatchObject({
    code: 'SCHEMA_VALIDATION_ERROR',
  });
  expect(registry.getDefinition('system.manifest.full')).not.toBeNull();
});
