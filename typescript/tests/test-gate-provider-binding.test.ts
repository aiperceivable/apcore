/**
 * PROTOCOL_SPEC 6.6.5.5 "Providers reach the gate they configure" (D-129).
 *
 * The cross-language cases live in conformance-gate-provider-binding.test.ts;
 * these cover the SDK-specific ways a strategy reaches an executor that the
 * fixture does not name: a strategy registered by name and a per-call strategy
 * passed to `callWithTrace`.
 */

import { describe, expect, it } from 'vitest';

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

async function registryWithTarget(requiresApproval = false): Promise<Registry> {
  const registry = new Registry();
  await registry.register('demo.target', {
    description: 'D-129 target',
    annotations: { requiresApproval },
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: () => ({ ok: true }),
  });
  return registry;
}

function bareStandard(registry: Registry) {
  return buildStandardStrategy({
    config: null,
    registry,
    acl: null,
    approvalHandler: null,
    middlewareManager: new MiddlewareManager(),
  });
}

async function errorCodeOf(promise: Promise<unknown>): Promise<string> {
  try {
    await promise;
    return 'ok';
  } catch (err) {
    if (err instanceof ModuleError) return err.code;
    throw err;
  }
}

describe('D-129: an executor binds its providers into the strategy it runs', () => {
  it('denies through a pre-built strategy instance given a deny-all ACL', async () => {
    const registry = await registryWithTarget();
    const executor = new Executor({
      registry,
      acl: new ACL([], 'deny'),
      strategy: bareStandard(registry),
    });

    expect(await errorCodeOf(executor.call('demo.target', {}))).toBe('ACL_DENIED');
    const state = executor.governanceState();
    expect(state.aclConfigured).toBe(true);
    expect(state.builtinAclGateWired).toBe(true);
  });

  it('binds the approval handler and the policy into a pre-built instance', async () => {
    const registry = await registryWithTarget(true);
    const withHandler = new Executor({
      registry,
      approvalHandler: new AlwaysDenyHandler(),
      strategy: bareStandard(registry),
    });
    expect(await errorCodeOf(withHandler.call('demo.target', {}))).toBe('APPROVAL_DENIED');

    const withStrictPolicy = new Executor({
      registry,
      policy: new ExecutionPolicy([], { strict: true }),
      strategy: bareStandard(registry),
    });
    expect(await errorCodeOf(withStrictPolicy.call('demo.target', {}))).toBe('APPROVAL_DENIED');
    expect(withStrictPolicy.governanceState().policyStrict).toBe(true);
  });

  it('binds providers into a strategy resolved from the named-strategy registry', async () => {
    const registry = await registryWithTarget();
    Executor.registerStrategy('d129-registered-standard', bareStandard(registry));
    const executor = new Executor({
      registry,
      acl: new ACL([], 'deny'),
      strategy: 'd129-registered-standard',
    });

    expect(await errorCodeOf(executor.call('demo.target', {}))).toBe('ACL_DENIED');
  });

  it('binds providers into a per-call strategy passed to callWithTrace', async () => {
    const registry = await registryWithTarget();
    const executor = new Executor({ registry, acl: new ACL([], 'deny'), strategy: 'internal' });

    // The running strategy has no gate: the call runs, as reported.
    expect(await errorCodeOf(executor.call('demo.target', {}))).toBe('ok');
    expect(executor.governanceState().builtinAclGateWired).toBe(false);

    // A per-call standard strategy's gate enforces this executor's ACL.
    const perCall = bareStandard(registry);
    expect(
      await errorCodeOf(executor.callWithTrace('demo.target', {}, null, { strategy: perCall })),
    ).toBe('ACL_DENIED');
  });

  it('reports what the running gate holds when the executor holds nothing', async () => {
    const registry = await registryWithTarget();
    const strategy = buildStandardStrategy({
      config: null,
      registry,
      acl: new ACL([], 'deny'),
      approvalHandler: new AlwaysDenyHandler(),
      middlewareManager: new MiddlewareManager(),
      policy: new ExecutionPolicy([], { strict: true }),
    });
    const state = new Executor({ registry, strategy }).governanceState();
    expect(state.aclConfigured).toBe(true);
    expect(state.approvalHandlerConfigured).toBe(true);
    expect(state.policyStrict).toBe(true);
  });
});
