/**
 * Tests for pipeline-config.ts: step type registry, _resolveStep, and buildStrategyFromConfig.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  ConfigurationError,
  _resolveStep,
  buildStrategyFromConfig,
  buildStrategyFromConfigSync,
  registerStepType,
  registeredStepTypes,
  unregisterStepType,
} from '../src/pipeline-config.js';
import { fileURLToPath } from 'node:url';
import { Config } from '../src/config.js';
import { ExecutionStrategy, PipelineDependencyError } from '../src/pipeline.js';
import type { Step, StepResult } from '../src/pipeline.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeMinimalStep(name: string): Step {
  return {
    name,
    description: `Step ${name}`,
    removable: true,
    replaceable: true,
    execute: async (): Promise<StepResult> => ({ action: 'continue' }),
  };
}

const TEST_TYPE_PREFIX = '__test_pc_';

function uniqueType(suffix: string): string {
  return `${TEST_TYPE_PREFIX}${suffix}`;
}

// ---------------------------------------------------------------------------
// registerStepType / unregisterStepType / registeredStepTypes
// ---------------------------------------------------------------------------

describe('registerStepType', () => {
  const registeredNames: string[] = [];

  afterEach(() => {
    for (const name of registeredNames) {
      unregisterStepType(name);
    }
    registeredNames.length = 0;
  });

  it('registers a step type successfully', () => {
    const name = uniqueType('basic');
    registeredNames.push(name);
    registerStepType(name, (_config) => makeMinimalStep('from-factory'));
    expect(registeredStepTypes()).toContain(name);
  });

  it('throws when name is empty', () => {
    expect(() => registerStepType('', () => makeMinimalStep('x'))).toThrow(
      /Invalid step type name/,
    );
  });

  it('throws when name contains whitespace', () => {
    expect(() => registerStepType('has space', () => makeMinimalStep('x'))).toThrow(
      /Invalid step type name/,
    );
    expect(() => registerStepType('tab\there', () => makeMinimalStep('x'))).toThrow(
      /Invalid step type name/,
    );
  });

  it('throws when the same name is registered twice', () => {
    const name = uniqueType('dup');
    registeredNames.push(name);
    registerStepType(name, (_config) => makeMinimalStep('x'));
    expect(() => registerStepType(name, (_config) => makeMinimalStep('y'))).toThrow(
      /already registered/,
    );
  });
});

describe('unregisterStepType', () => {
  it('returns true and removes the registered type', () => {
    const name = uniqueType('to-remove');
    registerStepType(name, (_config) => makeMinimalStep('x'));
    const removed = unregisterStepType(name);
    expect(removed).toBe(true);
    expect(registeredStepTypes()).not.toContain(name);
  });

  it('returns false when type was not registered', () => {
    const result = unregisterStepType(uniqueType('nonexistent'));
    expect(result).toBe(false);
  });

  it('allows re-registration after unregister', () => {
    const name = uniqueType('reuse');
    registerStepType(name, (_config) => makeMinimalStep('a'));
    unregisterStepType(name);
    expect(() => registerStepType(name, (_config) => makeMinimalStep('b'))).not.toThrow();
    unregisterStepType(name);
  });
});

describe('registeredStepTypes', () => {
  it('returns an array of registered type names', () => {
    const name = uniqueType('listed');
    registerStepType(name, (_config) => makeMinimalStep('x'));
    expect(registeredStepTypes()).toContain(name);
    unregisterStepType(name);
  });
});

// ---------------------------------------------------------------------------
// _resolveStep
// ---------------------------------------------------------------------------

describe('_resolveStep', () => {
  const TEST_TYPE = uniqueType('resolve-test');

  beforeEach(() => {
    registerStepType(TEST_TYPE, (config) =>
      makeMinimalStep((config['stepName'] as string) ?? 'resolved'),
    );
  });

  afterEach(() => {
    unregisterStepType(TEST_TYPE);
  });

  it('resolves a step from a registered type', async () => {
    const step = await _resolveStep({ type: TEST_TYPE, config: { stepName: 'my-step' } });
    expect(step.name).toBe('my-step');
  });

  it('applies name override from step definition', async () => {
    const step = await _resolveStep({ type: TEST_TYPE, name: 'override-name', config: {} });
    expect(step.name).toBe('override-name');
  });

  it('rejects handler path missing colon separator', async () => {
    await expect(_resolveStep({ handler: 'no-colon-here' })).rejects.toThrow(
      /Expected format: 'module:exportName'/,
    );
  });

  it('rejects handler path containing path-traversal segments', async () => {
    await expect(_resolveStep({ handler: '../escape:fn' })).rejects.toThrow(
      /must not contain '\.\.'/,
    );
  });

  it('rejects handler path using file: URLs', async () => {
    await expect(_resolveStep({ handler: 'file:///etc/passwd:fn' })).rejects.toThrow(
      /must not use 'file:' URLs/,
    );
  });

  it('rejects handler path using http: URLs', async () => {
    await expect(_resolveStep({ handler: 'http://evil.com/mod.js:fn' })).rejects.toThrow(
      /must not use 'http:' URLs/,
    );
  });

  it('rejects handler path using data: URLs', async () => {
    await expect(
      _resolveStep({ handler: 'data:text/javascript,export const x=1:fn' }),
    ).rejects.toThrow(/must not use 'data:' URLs/);
  });

  it('rejects handler module that cannot be imported', async () => {
    await expect(_resolveStep({ handler: './nonexistent-module-xyz:fn' })).rejects.toThrow(
      /Cannot import handler module/,
    );
  });

  it.each([
    ['node:path:missingExport', /Export 'missingExport' not found/],
    ['node:path:sep', /resolved to a non-callable/],
  ])('reports an unusable handler export in %s', async (handler, error) => {
    await expect(_resolveStep({ handler })).rejects.toThrow(error);
  });

  it.each(['createStep', 'createArrowStep', 'ConfiguredTestStep'])(
    'passes config to the handler export %s',
    async (exportName) => {
      const fixture = fileURLToPath(new URL('./fixtures/pipeline-step-handlers.ts', import.meta.url));
      const step = await _resolveStep({
        handler: `${fixture}:${exportName}`,
        config: { label: 'from handler' },
      });
      expect(step.name).toBe('from handler');
    },
  );

  it('throws when type is unknown', async () => {
    await expect(_resolveStep({ type: uniqueType('not-registered') })).rejects.toThrow(
      /not registered/,
    );
  });

  it('throws when neither type nor handler is provided', async () => {
    await expect(_resolveStep({ name: 'bare-step' })).rejects.toThrow(
      /neither 'type' nor 'handler'/,
    );
  });

  it('forwards matchModules, ignoreErrors, pure, and timeoutMs overrides', async () => {
    const step = await _resolveStep({
      type: TEST_TYPE,
      matchModules: ['foo.*'],
      ignoreErrors: true,
      pure: true,
      timeoutMs: 500,
      config: {},
    });
    expect((step as any).matchModules).toEqual(['foo.*']);
    expect((step as any).ignoreErrors).toBe(true);
    expect((step as any).pure).toBe(true);
    expect((step as any).timeoutMs).toBe(500);
  });
});

// ---------------------------------------------------------------------------
// buildStrategyFromConfig
// ---------------------------------------------------------------------------

describe('buildStrategyFromConfig', () => {
  // We use a real StandardStrategyDeps with minimal stubs because
  // buildStandardStrategy expects specific shape. Instead we test via
  // buildStrategyFromConfig with a registered custom step type.

  const CUSTOM_TYPE = uniqueType('build-test');

  beforeEach(() => {
    registerStepType(CUSTOM_TYPE, (_config) => makeMinimalStep('custom-step'));
  });

  afterEach(() => {
    unregisterStepType(CUSTOM_TYPE);
  });

  it('produces a strategy with standard steps when given empty config', async () => {
    const deps = makeFakeDeps();
    const strategy = await buildStrategyFromConfig({}, deps);
    expect(strategy.stepNames().length).toBeGreaterThan(0);
  });

  it('throws ConfigurationError when remove targets a nonexistent step (Issue #33 §1.2)', async () => {
    const deps = makeFakeDeps();
    await expect(
      buildStrategyFromConfig({ remove: ['nonexistent_step_xyz'] }, deps),
    ).rejects.toThrow(/nonexistent_step_xyz/);
  });

  it('throws ConfigurationError when a step has neither after nor before in steps list (Issue #33 §1.2)', async () => {
    const deps = makeFakeDeps();
    await expect(
      buildStrategyFromConfig(
        {
          steps: [{ type: CUSTOM_TYPE, name: 'custom-step' }],
        },
        deps,
      ),
    ).rejects.toThrow(/neither 'after' nor 'before'/);
  });

  describe('configured validation limits', () => {
    const deps = {
      ...makeFakeDeps(),
      config: new Config({
        validation: { pipeline: { step_name_max_length: 5, timeout_ms_max: 50 } },
      }),
    };

    it('accepts name and timeout values at the configured limits', () => {
      const { strategy } = buildStrategyFromConfigSync(
        {
          configure: { execute: { timeout_ms: 50 }, input_validation: { pure: true } },
          steps: [
            { type: CUSTOM_TYPE, name: 'probe', after: 'execute', timeout_ms: 50 },
            { type: CUSTOM_TYPE, after: 'probe' },
          ],
        },
        deps,
      );
      expect(strategy.steps.find((step) => step.name === 'probe')?.timeoutMs).toBe(50);
      expect(strategy.steps.find((step) => step.name === 'execute')?.timeoutMs).toBe(50);
      expect(strategy.stepNames()).toContain('custom-step');
    });

    it('rejects a custom step name above the configured limit', () => {
      expect(() => buildStrategyFromConfigSync(
        { steps: [{ type: CUSTOM_TYPE, name: 'longer', after: 'execute' }] },
        deps,
      )).toThrow(/name is 6 characters, over the 5 configured/);
    });

    it('rejects a custom step timeout above the configured limit', () => {
      expect(() => buildStrategyFromConfigSync(
        { steps: [{ type: CUSTOM_TYPE, name: 'probe', after: 'execute', timeout_ms: 51 }] },
        deps,
      )).toThrow(/Step 'probe': timeout_ms 51 is over the 50 configured/);
    });

    it('rejects an existing step timeout override above the configured limit', () => {
      expect(() => buildStrategyFromConfigSync(
        { configure: { execute: { timeout_ms: 51 } } },
        deps,
      )).toThrow(/Step 'execute': timeout_ms 51 is over the 50 configured/);
    });

    it('enforces the name limit when no timeout limit is configured', () => {
      const nameOnlyDeps = {
        ...deps,
        config: new Config({ validation: { pipeline: { step_name_max_length: 5 } } }),
      };
      const { strategy } = buildStrategyFromConfigSync(
        { steps: [{ type: CUSTOM_TYPE, name: 'probe', after: 'execute', timeout_ms: 500 }] },
        nameOnlyDeps,
      );
      expect(strategy.steps.find((step) => step.name === 'probe')?.timeoutMs).toBe(500);
    });
  });

  it('inserts a custom step after a named standard step', async () => {
    const deps = makeFakeDeps();
    const baseStrategy = await buildStrategyFromConfig({}, deps);
    const firstName = baseStrategy.stepNames()[0];

    const insertType = uniqueType('insert-after');
    registerStepType(insertType, (_c) => makeMinimalStep('inserted-after'));
    try {
      const strategy = await buildStrategyFromConfig(
        {
          steps: [{ type: insertType, name: 'inserted-after', after: firstName }],
        },
        deps,
      );
      const names = strategy.stepNames();
      const firstIdx = names.indexOf(firstName);
      expect(names[firstIdx + 1]).toBe('inserted-after');
    } finally {
      unregisterStepType(insertType);
    }
  });

  it('inserts a custom step before a named standard step', async () => {
    const deps = makeFakeDeps();
    const baseStrategy = await buildStrategyFromConfig({}, deps);
    const lastName = baseStrategy.stepNames().at(-1)!;

    const insertType = uniqueType('insert-before');
    registerStepType(insertType, (_c) => makeMinimalStep('inserted-before'));
    try {
      const strategy = await buildStrategyFromConfig(
        {
          steps: [{ type: insertType, name: 'inserted-before', before: lastName }],
        },
        deps,
      );
      const names = strategy.stepNames();
      const lastIdx = names.indexOf(lastName);
      expect(names[lastIdx - 1]).toBe('inserted-before');
    } finally {
      unregisterStepType(insertType);
    }
  });
});

// ---------------------------------------------------------------------------
// pipeline.steps entry field validation (apcore#89)
//
// `schemas/apcore-config.schema.json` `$defs/PipelineStep` is
// `additionalProperties: false` and nothing enforced it. Two defects lived
// behind that, and the second is the one that bites an operator who spells
// everything correctly:
//
//   1. An unknown key was accepted and dropped. `tiemout_ms: 5000` built a
//      step with `timeoutMs === 0` and said nothing.
//   2. The CANONICAL snake_case keys were dropped too. `StepDefinition`,
//      `_resolveStep` and `ConfiguredStep` are all camelCase, so on a `steps:`
//      entry `timeout_ms` / `ignore_errors` / `match_modules` never reached the
//      step — only `pure` worked, because it is spelled the same in both. This
//      is the identical defect that was fixed for `configure:` one pass
//      earlier, still fully present one key over.
// ---------------------------------------------------------------------------

describe('buildStrategyFromConfig — pipeline.steps entry field validation', () => {
  it('rejects an unknown key on a steps entry and names it', async () => {
    const type = uniqueType('closed-set');
    registerStepType(type, () => makeMinimalStep('probe'));
    try {
      await expect(
        buildStrategyFromConfig(
          { steps: [{ name: 'probe', type, after: 'execute', tiemout_ms: 5000 }] },
          makeFakeDeps(),
        ),
      ).rejects.toThrow(/Pipeline step 'probe' has no field 'tiemout_ms'/);
    } finally {
      unregisterStepType(type);
    }
  });

  it('names the position when the entry has no usable name', async () => {
    await expect(
      buildStrategyFromConfig({ steps: [{ nope: 1 }] }, makeFakeDeps()),
    ).rejects.toThrow(/at pipeline\.steps\[0\] has no field 'nope'/);
  });

  // The canonical wire spelling has to reach the built step, or closing the
  // key set would only have certified these three as valid while they kept
  // doing nothing.
  it('applies the canonical snake_case fields to the inserted step', async () => {
    const type = uniqueType('snake-entry');
    registerStepType(type, () => makeMinimalStep('snake-probe'));
    try {
      const strategy = await buildStrategyFromConfig(
        {
          steps: [
            {
              name: 'snake-probe',
              type,
              after: 'execute',
              match_modules: ['executor.*'],
              ignore_errors: true,
              pure: true,
              timeout_ms: 5000,
            },
          ],
        },
        makeFakeDeps(),
      );
      const step = strategy.steps.find((s) => s.name === 'snake-probe')!;
      expect(step.matchModules).toEqual(['executor.*']);
      expect(step.ignoreErrors).toBe(true);
      expect(step.pure).toBe(true);
      expect(step.timeoutMs).toBe(5000);
    } finally {
      unregisterStepType(type);
    }
  });

  // This SDK's own idiomatic alias surface, deliberately not exercised by the
  // fixture. It kept working across the fix — it was the ONLY spelling that
  // worked before it.
  it('still applies the camelCase aliases to the inserted step', async () => {
    const type = uniqueType('camel-entry');
    registerStepType(type, () => makeMinimalStep('camel-probe'));
    try {
      const strategy = await buildStrategyFromConfig(
        {
          steps: [
            {
              name: 'camel-probe',
              type,
              after: 'execute',
              matchModules: ['api.*'],
              ignoreErrors: true,
              timeoutMs: 250,
            },
          ],
        },
        makeFakeDeps(),
      );
      const step = strategy.steps.find((s) => s.name === 'camel-probe')!;
      expect(step.matchModules).toEqual(['api.*']);
      expect(step.ignoreErrors).toBe(true);
      expect(step.timeoutMs).toBe(250);
    } finally {
      unregisterStepType(type);
    }
  });

  it('raises before the step factory runs (parse time, not build time)', async () => {
    const type = uniqueType('never-called');
    let called = 0;
    registerStepType(type, () => {
      called += 1;
      return makeMinimalStep('never');
    });
    try {
      await expect(
        buildStrategyFromConfig(
          { steps: [{ name: 'never', type, after: 'execute', bogus: true }] },
          makeFakeDeps(),
        ),
      ).rejects.toThrow(ConfigurationError);
      expect(called).toBe(0);
    } finally {
      unregisterStepType(type);
    }
  });
});

// ---------------------------------------------------------------------------
// pipeline.configure field validation (Issue #34 §2)
// ---------------------------------------------------------------------------

describe('buildStrategyFromConfig — pipeline.configure field validation', () => {
  // `BuiltinInputValidation` declares `pure`, `requires` and `provides` but NOT
  // `ignoreErrors` / `matchModules` / `timeoutMs`. The previous `key in step`
  // test therefore rejected exactly the fields its own error message advertised
  // as valid.
  it('the built-in step really does lack the optional fields (regression guard)', async () => {
    const strategy = await buildStrategyFromConfig({}, makeFakeDeps());
    const step = strategy.steps.find((s) => s.name === 'input_validation')!;
    expect('ignoreErrors' in step).toBe(false);
    expect('matchModules' in step).toBe(false);
    expect('timeoutMs' in step).toBe(false);
  });

  // The canonical spelling. schemas/apcore-config.schema.json $defs/PipelineStep
  // is snake_case, so this is what an operator actually writes in apcore.yaml
  // and what every conformance fixture carries. Accepting only camelCase left
  // three of the four fields unreachable from real YAML.
  it('accepts the canonical snake_case spellings from apcore.yaml', async () => {
    const strategy = await buildStrategyFromConfig(
      {
        configure: {
          input_validation: {
            ignore_errors: true,
            match_modules: ['executor.*'],
            pure: false,
            timeout_ms: 500,
          },
        } as never,
      },
      makeFakeDeps(),
    );
    const step = strategy.steps.find((s) => s.name === 'input_validation')!;
    expect(step.ignoreErrors).toBe(true);
    expect(step.matchModules).toEqual(['executor.*']);
    expect(step.pure).toBe(false);
    expect(step.timeoutMs).toBe(500);
  });

  it('accepts the camelCase aliases (the TypeScript Step surface)', async () => {
    const strategy = await buildStrategyFromConfig(
      {
        configure: {
          input_validation: { ignoreErrors: true, matchModules: ['api.*'], timeoutMs: 250 },
        },
      },
      makeFakeDeps(),
    );
    const step = strategy.steps.find((s) => s.name === 'input_validation')!;
    expect(step.ignoreErrors).toBe(true);
    expect(step.matchModules).toEqual(['api.*']);
    expect(step.timeoutMs).toBe(250);
  });

  // CORRECTED (apcore#89). This test used to be
  // `accepts requires/provides, as the spec example configures them`, and it
  // asserted that `configure` wrote both fields onto the step. It encoded the
  // wrong behaviour, from the wrong source: the "canonical example" it cited
  // in docs/features/middleware-system.md was itself the defect, and that
  // page now carries a warning box saying so.
  //
  // A step's `requires` / `provides` are its capability contract, declared by
  // the implementation. `schemas/apcore-config.schema.json`
  // `$defs/ConfigurableStepFields` is `additionalProperties: false` over four
  // fields and protocol-spec §5.16.1 says the same in words.
  //
  // Measured on this SDK before the fix: the assignment landed as an own
  // property on the built step, and `_validateDependencies` read the
  // rewritten value — so the built-in `input_validation`, which requires
  // `module` from `module_lookup`, could be rewritten to require `context`,
  // after which a strategy missing `module_lookup` constructed cleanly and
  // the `PipelineDependencyError` MUST could never fire for it. The
  // regression guard below pins that.
  it('rejects requires/provides — a step capability contract is not configurable', async () => {
    await expect(
      buildStrategyFromConfig(
        { configure: { input_validation: { requires: ['context'] } } as never },
        makeFakeDeps(),
      ),
    ).rejects.toThrow(/non-configurable field(?:s)?: 'requires'/);
    await expect(
      buildStrategyFromConfig(
        { configure: { input_validation: { provides: ['validated_inputs'] } } as never },
        makeFakeDeps(),
      ),
    ).rejects.toThrow(/non-configurable field(?:s)?: 'provides'/);
  });

  // The rejection above is only worth anything because the contract it
  // protects is enforced. This is the other half: the built-in contract is
  // still what the step carries after `configure` has run, and
  // `_validateDependencies` still throws on it.
  it('the built-in capability contract survives configure and is enforced', async () => {
    const strategy = await buildStrategyFromConfig(
      { configure: { input_validation: { ignore_errors: true } } as never },
      makeFakeDeps(),
    );
    const contextCreation = strategy.steps.find((s) => s.name === 'context_creation')!;
    const inputValidation = strategy.steps.find((s) => s.name === 'input_validation')!;
    expect(inputValidation.requires).toEqual(['module']);
    expect(inputValidation.provides).toEqual(['validated_inputs']);
    // `module` is provided by module_lookup; drop it and construction MUST
    // fail. Rewriting requires to ['context'] through YAML made this pass.
    expect(() => new ExecutionStrategy('probe', [contextCreation, inputValidation])).toThrow(
      PipelineDependencyError,
    );
  });

  it('rejects an unknown field and names the canonical spellings', async () => {
    await expect(
      buildStrategyFromConfig(
        { configure: { input_validation: { no_such_field: 1 } } as never },
        makeFakeDeps(),
      ),
    ).rejects.toThrow(
      /non-configurable field(?:s)?: 'no_such_field'.*match_modules, ignore_errors, pure, timeout_ms/s,
    );
  });

  // The valid-field list in the message must be exactly four. A regex that
  // only checks the four are present still passes if a fifth creeps back in.
  it('the error message advertises exactly four canonical fields', async () => {
    let message = '';
    try {
      await buildStrategyFromConfig(
        { configure: { input_validation: { no_such_field: 1 } } as never },
        makeFakeDeps(),
      );
    } catch (err) {
      message = (err as Error).message;
    }
    const listed = /Valid fields are: ([^(]+)\(/.exec(message)![1];
    expect(listed.trim().split(', ')).toEqual([
      'match_modules',
      'ignore_errors',
      'pure',
      'timeout_ms',
    ]);
  });

  // Narrower than the old `key in step` on purpose: these are the step's
  // identity and its own mutation guards, and `in` walked the prototype chain
  // so `execute` was accepted too — a config file could replace the step body.
  it.each(['name', 'description', 'removable', 'replaceable', 'execute'])(
    'rejects the non-configurable field %s',
    async (field) => {
      await expect(
        buildStrategyFromConfig(
          { configure: { input_validation: { [field]: 'x' } } as never },
          makeFakeDeps(),
        ),
      ).rejects.toThrow(new RegExp(`non-configurable field(?:s)?: '${field}'`));
    },
  );
});

// ---------------------------------------------------------------------------
// Minimal stub deps for buildStrategyFromConfig
// ---------------------------------------------------------------------------

function makeFakeDeps() {
  // StandardStrategyDeps shape -- provide minimal stubs
  return {
    registry: {
      get: (_id: string) => null,
    } as any,
    acl: null as any,
    config: {
      get: (_key: string, _def?: unknown) => undefined,
    } as any,
    middlewareManager: null as any,
    approvalHandler: null as any,
  };
}
