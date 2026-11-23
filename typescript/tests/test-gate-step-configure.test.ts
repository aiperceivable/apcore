/**
 * PROTOCOL_SPEC 5.16.1 "Governance gates cannot be weakened by `configure`"
 * (D-130).
 *
 * The YAML cases live in conformance-gate-step-configure.test.ts. These cover
 * the programmatic paths — every way a step enters an `ExecutionStrategy` —
 * plus the camelCase aliases and the values that stay accepted.
 */

import { describe, expect, it } from 'vitest';

import { ACL } from '../src/acl.js';
import {
  BuiltinACLCheck,
  BuiltinApprovalGate,
  BuiltinInputValidation,
  buildStandardStrategy,
} from '../src/builtin-steps.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import { ConfigurationError, buildStrategyFromConfig } from '../src/pipeline-config.js';
import { ExecutionStrategy } from '../src/pipeline.js';
import type { Step } from '../src/pipeline.js';
import { Registry } from '../src/registry/registry.js';

function deps(): Parameters<typeof buildStandardStrategy>[0] {
  return {
    config: null,
    registry: new Registry(),
    acl: new ACL([], 'deny'),
    approvalHandler: null,
    middlewareManager: new MiddlewareManager(),
  };
}

function thrown(fn: () => unknown): unknown {
  try {
    fn();
  } catch (err) {
    return err;
  }
  return null;
}

async function thrownAsync(fn: () => Promise<unknown>): Promise<unknown> {
  try {
    await fn();
  } catch (err) {
    return err;
  }
  return null;
}

function expectGateRejection(error: unknown, step: string, keys: readonly string[]): void {
  expect(error).toBeInstanceOf(ConfigurationError);
  expect((error as ConfigurationError).code).toBe('PIPELINE_CONFIGURATION_ERROR');
  expect((error as Error).message).toContain(step);
  for (const key of keys) expect((error as Error).message).toContain(`'${key}'`);
}

const weakenedAcl = (): Step =>
  Object.assign(new BuiltinACLCheck(new ACL([], 'deny')), { ignoreErrors: true });

describe('D-130: programmatic step configuration cannot weaken a governance gate', () => {
  it('rejects a weakened gate passed to configureStep', () => {
    const strategy = buildStandardStrategy(deps());
    expectGateRejection(thrown(() => strategy.configureStep('acl_check', weakenedAcl())), 'acl_check', [
      'ignore_errors',
    ]);
    // The running gate is untouched by the rejected call.
    expect(strategy.steps.find((s) => s.name === 'acl_check')?.ignoreErrors).toBeUndefined();
  });

  it('rejects a weakened gate passed to replace', () => {
    const strategy = buildStandardStrategy(deps());
    const pureGate = Object.assign(new BuiltinApprovalGate(null), { pure: true });
    expectGateRejection(thrown(() => strategy.replace('approval_gate', pureGate)), 'approval_gate', [
      'pure',
    ]);
  });

  it('rejects a weakened gate inserted with insertAfter / insertBefore', () => {
    const strategy = buildStandardStrategy(deps());
    strategy.remove('acl_check');
    const scoped = Object.assign(new BuiltinACLCheck(null), { matchModules: ['public.*'] });
    expectGateRejection(thrown(() => strategy.insertAfter('module_lookup', scoped)), 'acl_check', [
      'match_modules',
    ]);
    expectGateRejection(thrown(() => strategy.insertBefore('approval_gate', scoped)), 'acl_check', [
      'match_modules',
    ]);
  });

  it('rejects a weakened gate handed to the ExecutionStrategy constructor', () => {
    expectGateRejection(thrown(() => new ExecutionStrategy('custom', [weakenedAcl()])), 'acl_check', [
      'ignore_errors',
    ]);
  });

  it('recognises a subclass of a built-in gate as a gate', () => {
    class LenientACL extends BuiltinACLCheck {
      readonly ignoreErrors = true;
    }
    expectGateRejection(
      thrown(() => new ExecutionStrategy('custom', [new LenientACL(null)])),
      'acl_check',
      ['ignore_errors'],
    );
  });

  it('leaves non-gate steps configurable', () => {
    const lenient = Object.assign(new BuiltinInputValidation(), {
      ignoreErrors: true,
      matchModules: ['a.*'],
    });
    const strategy = buildStandardStrategy(deps());
    expect(() => strategy.configureStep('input_validation', lenient)).not.toThrow();
  });
});

describe('D-130: pipeline.configure on a gate', () => {
  it('names every offending key, in both spellings', async () => {
    const error = await thrownAsync(() =>
      buildStrategyFromConfig(
        { configure: { approval_gate: { ignoreErrors: true, match_modules: ['x.*'], pure: true } } },
        deps(),
      ),
    );
    expectGateRejection(error, 'approval_gate', ['match_modules', 'ignore_errors', 'pure']);
  });

  it('rejects an empty match_modules list on either gate', async () => {
    for (const gate of ['acl_check', 'approval_gate']) {
      const error = await thrownAsync(() =>
        buildStrategyFromConfig({ configure: { [gate]: { match_modules: [] } } }, deps()),
      );
      expectGateRejection(error, gate, ['match_modules']);
    }
  });

  it('accepts pure: false on acl_check', async () => {
    const strategy = await buildStrategyFromConfig(
      { configure: { acl_check: { pure: false } } },
      deps(),
    );
    expect(strategy.steps.find((s) => s.name === 'acl_check')?.pure).toBe(false);
  });

  it('accepts the default values and timeout_ms', async () => {
    const strategy = await buildStrategyFromConfig(
      {
        configure: {
          // `pure: true` is acl_check's own value — validate() must run it.
          acl_check: { pure: true, ignore_errors: false, match_modules: null, timeout_ms: 250 },
          approval_gate: { pure: false, timeoutMs: 1000 },
        } as never,
      },
      deps(),
    );
    expect(strategy.steps.find((s) => s.name === 'acl_check')?.timeoutMs).toBe(250);
    expect(strategy.steps.find((s) => s.name === 'approval_gate')?.timeoutMs).toBe(1000);
  });
});
