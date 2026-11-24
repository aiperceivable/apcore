/**
 * Cross-language conformance driver for `pipeline_failfast_config.json`
 * (Issue #33 / apcore#89 — docs/features/middleware-system.md,
 * "Configuration safety").
 *
 * Fixture source: apcore/conformance/fixtures/pipeline_failfast_config.json
 * (canonical). Its `driver_contract` block is the contract; every rule in it
 * is honoured below and named at the assertion that satisfies it.
 *
 * NO NAMING TRANSLATION  (driver_contract.snake_case_is_the_wire_spelling)
 * ---------------------
 * The fixture's YAML is fed to `buildStrategyFromConfig` UNMANGLED. An earlier
 * revision of this driver carried a private `FIELD_NAME_TRANSLATION` map
 * (`ignore_errors` -> `ignoreErrors`, ...) so the SDK would accept the
 * fixture's snake_case. That map hid the real defect: the schema, every
 * `apcore.yaml` and every fixture are snake_case, so a user copying the
 * canonical YAML still got `ConfigurationError`. The SDK now normalises both
 * spellings itself (src/pipeline-config.ts CONFIGURABLE_STEP_FIELDS) and this
 * driver asserts that by passing the fixture bytes straight through. Do not
 * reintroduce it in any form. The camelCase alias path is this SDK's own API
 * surface, deliberately not exercised by the fixture; it is covered in
 * tests/test-pipeline-config.test.ts instead.
 *
 * THE CONFIGURABLE SET IS FOUR  (driver_contract.configurable_set_is_four)
 * ----------------------------
 * `match_modules`, `ignore_errors`, `pure`, `timeout_ms` — and nothing else.
 * `schemas/apcore-config.schema.json` `$defs/ConfigurableStepFields` is
 * `additionalProperties: false`; protocol-spec §5.16.1 says so in
 * words. `requires` / `provides` were accepted by this SDK until apcore#89.
 * Measured here before the fix: `configure` set them as own properties on the
 * built step and `ExecutionStrategy._validateDependencies` read the rewritten
 * values, so a `[context_creation, input_validation]` strategy that threw
 * `PipelineDependencyError` under the built-in `requires = ["module"]`
 * constructed cleanly once a config file replaced it with `["context"]`.
 *
 * FOUR DEFECTS THIS DRIVER USED TO PIN, ALL NOW FIXED
 * ---------------------------------------------------
 *  1. The fixture encoded `pipeline.configure` as a LIST of `{name, ...}`.
 *     It is an object map keyed by step name — what $defs/PipelineConfig
 *     declares and what all three SDKs parse. Corrected in the fixture.
 *  2. The fixture named the built-in step `validate_input`. No SDK has a step
 *     by that name; it is `input_validation` everywhere. This mattered: the
 *     case needs one VALID key to isolate the invalid one, so with both keys
 *     invalid the assertion it exists to make was unreachable.
 *  3. `buildStrategyFromConfig` validated override fields with `key in step`.
 *     matchModules / ignoreErrors / pure / timeoutMs are OPTIONAL on the Step
 *     interface, so configuring `ignoreErrors` on BuiltinInputValidation —
 *     which does not declare it — was rejected, with an error message that
 *     listed `ignoreErrors` as valid. Fixed in src/pipeline-config.ts by
 *     validating against the configurable-field set — which is keyed by the
 *     canonical snake_case spelling and accepts the camelCase alias, so the
 *     fixture's YAML needs no massaging on the way in.
 *  4. That configurable-field set contained `requires` and `provides`.
 *     Removed (apcore#89); see above.
 *
 * A fifth case, `missing_step_in_step_middleware_raises_configuration_error`,
 * was removed from the fixture: `pipeline.step_middleware:` is a config section
 * no SDK has ever parsed, and inventing one to satisfy a fixture is backwards.
 *
 * ASSERT THE WIRE CODE, NOT THE CLASS NAME  (driver_contract.assert_the_wire_code,
 * .canonical_code, .one_way_to_say_it). All three SDKs name this class
 * `ConfigurationError` while they emitted three different codes, so a
 * class-name assertion passed everywhere and proved nothing.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import {
  ConfigurationError,
  buildStrategyFromConfig,
  registerStepType,
  unregisterStepType,
} from '../src/pipeline-config.js';
import {
  ExecutionStrategy,
  PipelineDependencyError,
  PipelineEngine,
  type PipelineContext,
  type Step,
  type StepResult,
} from '../src/pipeline.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

interface FailfastCase {
  readonly id: string;
  readonly description: string;
  readonly input: {
    readonly yaml?: { readonly pipeline: Record<string, unknown> };
    readonly strategy?: {
      readonly name: string;
      readonly steps: ReadonlyArray<{
        readonly name: string;
        readonly requires?: readonly string[];
        readonly provides?: readonly string[];
      }>;
    };
  };
  readonly expected: Record<string, unknown>;
}

interface Fixture {
  readonly description: string;
  readonly test_cases: readonly FailfastCase[];
  readonly driver_contract: Record<string, string>;
}

function loadFixture(name: string): Fixture {
  return JSON.parse(fs.readFileSync(path.join(findFixturesRoot(), `${name}.json`), 'utf-8'));
}

const fixture = loadFixture('pipeline_failfast_config');

function caseById(id: string): FailfastCase {
  const tc = fixture.test_cases.find((c) => c.id === id);
  if (tc === undefined) {
    throw new Error(
      `pipeline_failfast_config.json no longer contains case '${id}'. The fixture is ` +
        'canonical — update this driver to match it, do not edit the fixture.',
    );
  }
  return tc;
}

/** The `pipeline.configure` map of a case, verbatim — no key translation. */
function configureOf(tc: FailfastCase): Record<string, Record<string, unknown>> {
  return tc.input.yaml!.pipeline['configure'] as Record<string, Record<string, unknown>>;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeFakeDeps() {
  return {
    registry: { get: () => null },
    acl: null,
    config: { get: () => undefined },
    middlewareManager: null,
    approvalHandler: null,
    // Only the config-parsing path runs; no step ever executes.
  } as unknown as Parameters<typeof buildStrategyFromConfig>[1];
}

/** A no-op step carrying the fixture's declared requires/provides. */
function makeStep(name: string, requires?: readonly string[], provides?: readonly string[]): Step {
  return {
    name,
    description: `Conformance stand-in for ${name}`,
    removable: true,
    replaceable: true,
    requires,
    provides,
    execute: async (): Promise<StepResult> => ({ action: 'continue' }),
  };
}

function strategyFrom(tc: FailfastCase): ExecutionStrategy {
  const def = tc.input.strategy!;
  return new ExecutionStrategy(
    def.name,
    def.steps.map((s) => makeStep(s.name, s.requires, s.provides)),
  );
}

/**
 * Build from the case's `configure` map and return whatever came out — the
 * error, or the strategy. `raised_at: parse_time` /
 * `deferred_to_first_call: false` (driver_contract.parse_time) is what this
 * shape asserts: nothing is ever executed, so a throw seen here can only have
 * come from turning the config into a strategy.
 */
async function buildFrom(
  tc: FailfastCase,
): Promise<{ error: unknown; strategy: ExecutionStrategy | null }> {
  try {
    return { error: null, strategy: await buildStrategyFromConfig({ configure: configureOf(tc) }, makeFakeDeps()) };
  } catch (err) {
    return { error: err, strategy: null };
  }
}

/** Assert the shared expectation shape of every parse-time rejection case. */
function expectParseTimeRejection(tc: FailfastCase, error: unknown): void {
  expect(error !== null, `expected ${tc.id} to raise, but construction succeeded`).toBe(
    tc.expected['raises'],
  );
  expect(error).toBeInstanceOf(ConfigurationError);
  // `error_message_contains` is a string OR an array of fragments — every one
  // must appear. The array form arrived with driver_contract.name_every_offending_key:
  // an error MUST name every offending key, so a case can require both.
  const fragments = tc.expected['error_message_contains'];
  for (const fragment of Array.isArray(fragments) ? fragments : [fragments]) {
    expect((error as Error).message).toContain(fragment as string);
  }
  // The wire code is the contract. The class name is shared by all three
  // SDKs and therefore distinguishes nothing.
  expect((error as ConfigurationError).code).toBe(tc.expected['error_code']);
  expect(tc.expected['raised_at']).toBe('parse_time');
  expect(tc.expected['deferred_to_first_call']).toBe(false);
}

describe('Conformance: pipeline configuration fail-fast (pipeline_failfast_config.json)', () => {
  // -------------------------------------------------------------------------
  it('missing_step_in_configure_raises_configuration_error', async () => {
    const tc = caseById('missing_step_in_configure_raises_configuration_error');
    // Straight from the fixture — snake_case keys, real step names, no map.
    const { error } = await buildFrom(tc);
    expectParseTimeRejection(tc, error);
  });

  // -------------------------------------------------------------------------
  it('unmet_requires_raises_pipeline_dependency_error', () => {
    const tc = caseById('unmet_requires_raises_pipeline_dependency_error');

    let error: unknown = null;
    try {
      strategyFrom(tc);
    } catch (err) {
      error = err;
    }

    expect(error !== null).toBe(tc.expected['raises']);
    expect(error).toBeInstanceOf(PipelineDependencyError);
    // The wire code, not the class name — the fixture has no `error_type` key
    // for exactly the reason stated in the header.
    expect((error as PipelineDependencyError).code).toBe(tc.expected['error_code']);
    for (const fragment of tc.expected['error_message_contains'] as string[]) {
      expect((error as Error).message).toContain(fragment);
    }
    // raised_at: strategy_construction — the throw came from the
    // `new ExecutionStrategy(...)` call itself, not from a later run.
    expect(tc.expected['raised_at']).toBe('strategy_construction');
  });

  // -------------------------------------------------------------------------
  it('satisfied_requires_constructs_successfully', async () => {
    const tc = caseById('satisfied_requires_constructs_successfully');

    let error: unknown = null;
    let strategy: ExecutionStrategy | null = null;
    try {
      strategy = strategyFrom(tc);
    } catch (err) {
      error = err;
    }
    expect(error !== null, `unexpected error: ${String(error)}`).toBe(tc.expected['raises']);

    // strategy_callable: prove it by actually running it end to end.
    const ctx = {
      moduleId: 'demo.greet',
      inputs: {},
      // Stand-in Context: the stub steps read no context fields.
      context: {} as PipelineContext['context'],
      output: null,
    } as PipelineContext;
    const [, trace] = await new PipelineEngine().run(strategy!, ctx);
    expect(trace.success).toBe(tc.expected['strategy_callable']);
    expect(trace.steps.map((s) => s.name)).toEqual(tc.input.strategy!.steps.map((s) => s.name));
  });

  // -------------------------------------------------------------------------
  // A typo — `ignore_error` for `ignore_errors`. The row on which the three
  // SDKs diverged most sharply: Rust logged "Unknown configurable field —
  // ignored" and ran the unconfigured pipeline while reporting success.
  it('unknown_configure_field_raises_configuration_error', async () => {
    const tc = caseById('unknown_configure_field_raises_configuration_error');
    const { error } = await buildFrom(tc);
    expectParseTimeRejection(tc, error);
  });

  // -------------------------------------------------------------------------
  // The capability contract belongs to the step implementation. This exact
  // YAML shipped as the canonical example in features/middleware-system.md.
  it('configure_must_not_rewrite_a_step_capability_contract', async () => {
    const tc = caseById('configure_must_not_rewrite_a_step_capability_contract');
    const { error, strategy } = await buildFrom(tc);
    expectParseTimeRejection(tc, error);
    // No strategy is handed back, so nothing can have been mutated on the way
    // to the throw.
    expect(strategy).toBeNull();
    // driver_contract.name_every_offending_key: BOTH keys, not whichever the
    // map happened to visit first. The order-tolerant `(requires|provides)`
    // this assertion used to carry passed against an implementation that
    // reported one and stopped — and that implementation fails the same
    // fixture on apcore-rust, whose serde_json::Map is sorted rather than
    // insertion-ordered.
    for (const key of ['requires', 'provides']) {
      expect((error as Error).message).toContain(`'${key}'`);
    }

    // And the rule that this rejection exists to protect still fires. Under
    // the built-in contract `requires = ["module"]`, an input_validation with
    // no upstream module_lookup MUST raise — which is precisely what the
    // rejected YAML used to switch off by rewriting it to ["context"].
    const clean = await buildStrategyFromConfig({}, makeFakeDeps());
    const contextCreation = clean.steps.find((s) => s.name === 'context_creation')!;
    const inputValidation = clean.steps.find((s) => s.name === 'input_validation')!;
    expect(inputValidation.requires).toEqual(['module']);
    expect(() => new ExecutionStrategy('probe', [contextCreation, inputValidation])).toThrow(
      PipelineDependencyError,
    );
  });

  // -------------------------------------------------------------------------
  // driver_contract.read_the_field_back_off_the_step: `raises: false` alone
  // passes against an implementation that accepts the keys and applies none of
  // them, so every field is read back off the built step object.
  it('all_four_configurable_fields_are_accepted', async () => {
    const tc = caseById('all_four_configurable_fields_are_accepted');
    const { error, strategy } = await buildFrom(tc);
    expect(error === null, `unexpected error: ${String(error)}`).toBe(!tc.expected['raises']);

    const want = tc.expected['configured_step_fields'] as Record<string, unknown>;
    const step = strategy!.steps.find((s) => s.name === want['step_name'])!;
    expect(step, `no step named ${String(want['step_name'])} in the built strategy`).toBeDefined();
    expect(step.matchModules).toEqual(want['match_modules']);
    expect(step.ignoreErrors).toBe(want['ignore_errors']);
    expect(step.pure).toBe(want['pure']);
    expect(step.timeoutMs).toBe(want['timeout_ms']);

    // strategy_callable: the configured strategy is intact and would construct
    // again from its own steps — i.e. `configure` left the dependency graph
    // valid. The standard strategy's steps need live registry/ACL deps to
    // actually execute, so callability is asserted structurally here; the
    // end-to-end run lives in `satisfied_requires_constructs_successfully`.
    expect(tc.expected['strategy_callable']).toBe(true);
    expect(strategy!.steps.length).toBeGreaterThan(0);
    for (const s of strategy!.steps) {
      expect(typeof s.execute).toBe('function');
    }
    expect(() => new ExecutionStrategy('revalidate', [...strategy!.steps])).not.toThrow();
  });

  // -------------------------------------------------------------------------
  // driver_contract.steps_entries_are_closed_too. `$defs/PipelineStep` has been
  // `additionalProperties: false` since it was written and nothing enforced it.
  // The `type` in the case must be REGISTERED, or the entry never reaches the
  // insertion path and the test passes on an unrelated "type not registered"
  // error — which is why this registers a no-op factory under the fixture's own
  // `type` value rather than swapping in a built-in step name.
  it('unknown_key_on_a_steps_entry_raises_configuration_error', async () => {
    const tc = caseById('unknown_key_on_a_steps_entry_raises_configuration_error');
    const steps = tc.input.yaml!.pipeline['steps'] as ReadonlyArray<Record<string, unknown>>;
    const typeName = steps[0]['type'] as string;

    unregisterStepType(typeName);
    registerStepType(typeName, () => ({
      name: typeName,
      description: `Conformance no-op for type '${typeName}'`,
      removable: true,
      replaceable: true,
      execute: async (): Promise<StepResult> => ({ action: 'continue' }),
    }));
    try {
      // The registration is real: without the unknown key the same entry
      // inserts cleanly, so the throw below can only be about that key.
      const control = steps.map((e) => {
        const copy = { ...e };
        delete copy[tc.expected['error_message_contains'] as string];
        return copy;
      });
      const ok = await buildStrategyFromConfig({ steps: control }, makeFakeDeps());
      expect(ok.stepNames()).toContain(steps[0]['name']);

      let error: unknown = null;
      try {
        await buildStrategyFromConfig({ steps: [...steps] }, makeFakeDeps());
      } catch (err) {
        error = err;
      }
      expectParseTimeRejection(tc, error);
    } finally {
      unregisterStepType(typeName);
    }
  });

  // -------------------------------------------------------------------------
  // driver_contract.configurable_set_is_four. The set is module-private, so it
  // is asserted through the surface an operator actually meets: the rejection
  // message names the canonical spellings, and there are exactly four.
  it('the configurable set is exactly the four canonical fields', async () => {
    let error: unknown = null;
    try {
      await buildStrategyFromConfig(
        { configure: { input_validation: { totally_made_up: 1 } } as never },
        makeFakeDeps(),
      );
    } catch (err) {
      error = err;
    }
    expect(error).toBeInstanceOf(ConfigurationError);
    const listed = /Valid fields are: ([^(]+)\(/.exec((error as Error).message)![1];
    expect(listed.trim().split(', ')).toEqual([
      'match_modules',
      'ignore_errors',
      'pure',
      'timeout_ms',
    ]);
  });

  // -------------------------------------------------------------------------
  it('drives every fixture case', () => {
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'missing_step_in_configure_raises_configuration_error',
      'unmet_requires_raises_pipeline_dependency_error',
      'satisfied_requires_constructs_successfully',
      'unknown_configure_field_raises_configuration_error',
      'configure_must_not_rewrite_a_step_capability_contract',
      'all_four_configurable_fields_are_accepted',
      'unknown_key_on_a_steps_entry_raises_configuration_error',
    ]);
    // The driver_contract is part of the fixture; if a rule is added, this
    // driver has to grow an assertion for it rather than silently ignore it.
    expect(Object.keys(fixture.driver_contract).sort()).toEqual([
      'assert_the_wire_code',
      'canonical_code',
      'configurable_set_is_four',
      'name_every_offending_key',
      'one_way_to_say_it',
      'parse_time',
      'read_the_field_back_off_the_step',
      'snake_case_is_the_wire_spelling',
      'steps_entries_are_closed_too',
      'the_valid_key_must_really_exist',
    ]);
  });
});
