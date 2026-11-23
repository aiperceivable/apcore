/**
 * Cross-language conformance tests driven by canonical JSON fixtures.
 *
 * Fixture source: apcore/conformance/fixtures/*.json (single source of truth).
 *
 * Fixture discovery order:
 *   1. CONFORMANCE_SPEC_REPO env var (explicit override)
 *   2. Sibling ../apcore/ directory (standard workspace layout & CI)
 */

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import * as yaml from 'js-yaml';
import { normalizeToCanonicalId } from '../src/utils/normalize.js';
import { matchPattern, matchGlob, calculateSpecificity } from '../src/utils/pattern.js';
import {
  ExecutionStrategy,
  PipelineEngine,
  PipelineStepError,
  PipelineStepNotFoundError,
} from '../src/pipeline.js';
import type { Step, StepResult, PipelineContext, PipelineState } from '../src/pipeline.js';
import { ACL, ACLRule } from '../src/acl.js';
import { Context, createIdentity } from '../src/context.js';
import {
  Config,
  _globalNsRegistry,
  _globalEnvMap,
  _envMapClaimed,
  _envPrefixUsed,
  applyEnvOverrides,
  applyNamespaceEnvOverrides,
} from '../src/config.js';
import { negotiateVersion } from '../src/version.js';
import { ErrorCodeRegistry } from '../src/error-code-registry.js';
import { ErrorCodes } from '../src/errors.js';
import { guardCallChain } from '../src/utils/call-chain.js';
import {
  // CallDepthExceededError / CallFrequencyExceededError stay: the call_chain
  // positive cases probe the limit boundary with them. CircularCallError is
  // gone with CALL_CHAIN_ERROR_MAP — `CIRCULAR_CALL` is now asserted as a wire
  // code through the SDK's own registry, not as a class (apcore#93).
  CallDepthExceededError,
  CallFrequencyExceededError,
  ApprovalDeniedError,
  ApprovalPendingError,
  ContextBindingError,
} from '../src/errors.js';
import { CancelToken } from '../src/cancel.js';
import { SchemaLoader, jsonSchemaToTypeBox, contentHash } from '../src/schema/loader.js';
import { SchemaValidator } from '../src/schema/validator.js';
import type { SchemaDefinition } from '../src/schema/types.js';
import type { TSchema } from '@sinclair/typebox';
import { deepMergeChunk } from '../src/executor.js';
import {
  createAnnotations,
  annotationsToJSON,
  annotationsFromJSON,
} from '../src/module.js';
import { BuiltinApprovalGate, buildStandardStrategy } from '../src/builtin-steps.js';
import type { ApprovalResult } from '../src/approval.js';
import { resolveDependencies } from '../src/registry/dependencies.js';
import { parseDependencies } from '../src/registry/metadata-pure.js';
import { classNameToSegment, discoverMultiClass } from '../src/registry/multi-class.js';
import { ModuleIdConflictError, CircuitBreakerOpenError } from '../src/errors.js';
import {
  CircuitBreakerWrapper,
  resolveSubscriberType,
  CircuitState,
  FileSubscriber,
  StdoutSubscriber,
  FilterSubscriber,
} from '../src/events/index.js';
import type { ApCoreEvent, EventSubscriber } from '../src/events/index.js';
import {
  registerSubscriberType,
  resetSubscriberRegistry,
  createSubscriberFromConfig,
  registerSysModules,
} from '../src/sys-modules/registration.js';
import { UpdateConfigModule, ReloadModule } from '../src/sys-modules/control.js';
import { ToggleFeatureModule } from '../src/sys-modules/toggle.js';
import { InMemoryAuditStore } from '../src/sys-modules/audit.js';
import { SysModuleRegistrationError, ModuleReloadConflictError } from '../src/errors.js';
import { EventEmitter } from '../src/events/index.js';
import { UsageCollector } from '../src/observability/index.js';
import {
  CircuitBreakerMiddleware,
  Middleware,
  MiddlewareManager,
  validateContextKey,
  isAsyncHandler,
} from '../src/middleware/index.js';
// Public export name (audit D2-001): middleware circuit-breaker state enum is
// publicly exported as `CircuitBreakerState` for cross-SDK parity (matches apcore-rust).
import { CircuitBreakerState } from '../src/index.js';
import {
  AsyncTaskManager,
  TaskStatus,
  InMemoryTaskStore,
  RetryConfig,
} from '../src/async-task.js';
import type { TaskStore } from '../src/async-task.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { APCore } from '../src/client.js';
import { ManifestFullModule } from '../src/sys-modules/manifest.js';
import { HealthSummaryModule } from '../src/sys-modules/health.js';
import { FunctionModule } from '../src/decorator.js';
import { Type } from '@sinclair/typebox';
import { ModuleError } from '../src/errors.js';
import {
  ErrorHistory,
  InMemoryObservabilityStore,
  normalizeMessage,
  computeFingerprint,
  BatchSpanProcessor,
  InMemoryExporter,
  createSpan,
  TracingMiddleware,
  MetricsCollector,
  PrometheusExporter,
  RedactionConfig,
} from '../src/observability/index.js';
import { DEFAULT_REDACTION_FIELD_PATTERNS } from '../src/observability/context-logger.js';
import { TraceContext } from '../src/trace-context.js';
import { findFixturesRoot, findSchemasRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture discovery
// ---------------------------------------------------------------------------

const FIXTURES_ROOT = findFixturesRoot();

const SCHEMAS_ROOT = findSchemasRoot();

function loadFixture(name: string): any {
  const fullPath = path.join(FIXTURES_ROOT, `${name}.json`);
  return JSON.parse(fs.readFileSync(fullPath, 'utf-8'));
}

function loadSchema(name: string): Record<string, unknown> {
  const fullPath = path.join(SCHEMAS_ROOT, `${name}.schema.json`);
  return JSON.parse(fs.readFileSync(fullPath, 'utf-8'));
}

// ---------------------------------------------------------------------------
// Wire-code assertions (apcore#92)
// ---------------------------------------------------------------------------
//
// A fixture's `expected_error` / `error_code` is a WIRE CODE, not a class name.
// Asserting the class (`toThrow(ErrorCodeCollisionError)`) is green whatever the
// fixture says — mutating the declared code left every suite green on all three
// SDKs — and it is unwritable in apcore-rust, which has no error classes at all,
// only `ErrorCode` variants. So the declared value must reach `error.code`.
//
// The membership set below is the SDK's OWN canonical registry, deliberately not
// a driver-local translation table: apcore-typescript#34 item 2 is the record of
// what a private map in this file costs — it papered over four config fields the
// SDK could not actually read.

/** Wire codes this SDK recognises, read off the SDK's own registry. */
const SDK_WIRE_CODES: ReadonlySet<string> = new Set<string>(Object.values(ErrorCodes));

/**
 * Fixture expectations that are deliberately NOT wire codes.
 *
 * `version_negotiation`'s `invalid_version_error` documents its own value
 * inline: "language-specific: ValueError in Python, Error in TS, ParseError in
 * Rust". apcore has no `PARSE_ERROR` code, so the discriminating assertion is
 * the inverse of a wire-code check — a native Error that is *not* a ModuleError.
 * Anything else unrecognised is a hard failure, never a skipped branch.
 *
 * `call_chain`'s `INVALID_LIMIT` used to be listed here for the same reason, and
 * is NOT any more. Spec v1.49.0 decision D-84 closed divergence T-B-005: all
 * three SDKs now raise the typed error with `GENERAL_INVALID_INPUT`, because a
 * builtin `ValueError` / bare `Error` is catchable by no cross-language caller
 * and carries no code from the registry. The fixture now spells that wire code
 * directly, so no alias is needed — exempting it instead would have kept
 * asserting the very defect D-84 fixed.
 */
const NON_WIRE_EXPECTATIONS: ReadonlySet<string> = new Set(['PARSE_ERROR']);

/** Run `fn`, returning what it threw, or `null` when it completed. */
function captureThrow(fn: () => unknown): unknown {
  try {
    fn();
    return null;
  } catch (e) {
    return e;
  }
}

/**
 * Assert that `thrown` is the failure the fixture declared, by wire code.
 *
 * An expectation this driver does not recognise is a hard failure naming the
 * case and the unknown value — the `pipeline_failfast_config` "teach the driver,
 * do not skip it" pattern. A skipped branch turns a wrong fixture value into a
 * passing test and makes the driver's literal, not the fixture, the contract.
 */
function assertWireCode(caseId: string, expectedError: unknown, thrown: unknown): void {
  if (typeof expectedError !== 'string') {
    throw new Error(
      `conformance driver: case '${caseId}' declares a non-string error expectation ` +
        `${JSON.stringify(expectedError)}. Teach the driver, do not skip it.`,
    );
  }
  if (!(thrown instanceof Error)) {
    throw new Error(
      `conformance driver: case '${caseId}' expects ${expectedError} to be raised, ` +
        `but nothing was thrown (got ${JSON.stringify(thrown)}).`,
    );
  }
  if (NON_WIRE_EXPECTATIONS.has(expectedError)) {
    expect(thrown).not.toBeInstanceOf(ModuleError);
    return;
  }
  if (!SDK_WIRE_CODES.has(expectedError)) {
    throw new Error(
      `conformance driver: case '${caseId}' declares error expectation ` +
        `'${expectedError}', which is neither a member of the SDK's ErrorCodes ` +
        `registry nor a documented non-wire expectation ` +
        `(${[...NON_WIRE_EXPECTATIONS].join(', ')}). Teach the driver, do not skip it.`,
    );
  }
  expect(thrown).toBeInstanceOf(ModuleError);
  expect((thrown as ModuleError).code).toBe(expectedError);
}

/** Structural equality, for expectations the fixture states as a boolean. */
function deepEq(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Top-level keys of a fixture case that state an expectation, for diagnostics.
 *
 * The `expected` prefix is the whole rule. A bare top-level `error_code` is an
 * INPUT — in `error_codes.json` it is the code being registered — and naming it
 * as an expectation is what let six cases read as covered when they were not.
 */
function expectationKeys(tc: Record<string, unknown>): string[] {
  return Object.keys(tc)
    .filter((k) => k === 'expected' || k.startsWith('expected_'))
    .sort();
}

describe('apcore Conformance Suite (TypeScript)', () => {
  // --- 1. ID Normalization (A02) ---
  const normalizeFixture = loadFixture('normalize_id');
  describe('ID Normalization (Algorithm A02)', () => {
    normalizeFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        if (tc.expected_error) {
          expect(() => normalizeToCanonicalId(tc.local_id, tc.language))
            .toThrow(`Normalized ID '${tc.expected_normalized}'`);
          return;
        }
        expect(normalizeToCanonicalId(tc.local_id, tc.language)).toBe(tc.expected);
      });
    });
  });

  // --- 2. Pattern Matching (A09) ---
  const patternFixture = loadFixture('pattern_matching');
  describe('Module-ID Pattern Matching (Algorithm A08)', () => {
    patternFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        expect(matchPattern(tc.pattern, tc.value)).toBe(tc.expected);
      });
    });
  });

  // --- 2b. Glob Matching (A25) — PROTOCOL_SPEC 9.2.3 (#116, #117) ---
  //
  // The matcher for every glob-dialect pattern-valued value EXCEPT module-ID
  // matching: `bindings.pattern`, `obs.redaction.sensitive_keys` glob entries,
  // event patterns, and `path_filter`. Kept in its own fixture rather than
  // folded into `pattern_matching` on purpose — 9.2.3 requirement 5 states why
  // the two algorithms are separate, and one shared fixture would hide it.
  const globFixture = loadFixture('glob_matching');
  describe('Glob Matching (Algorithm A25)', () => {
    globFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        expect(matchGlob(tc.pattern, tc.value)).toBe(tc.expected);
      });
    });

    // 9.2.3 requirement 2: every string is a valid pattern, so there is no
    // parse phase. Pinned separately because the failure it guards is a THROW,
    // not a wrong boolean — `glob::Pattern` rejects `a[b` and `a**b`.
    it('never throws, whatever the pattern', () => {
      for (const pattern of ['a[b', 'a**b', '[!', '{a,b}', '\\', '***', '[]', '?']) {
        for (const value of ['', 'a', 'a[b', 'executor.email.send']) {
          expect(() => matchGlob(pattern, value)).not.toThrow();
        }
      }
    });
  });

  // --- 3. Specificity Scoring (A10) ---
  const specificityFixture = loadFixture('specificity');
  describe('Specificity Scoring (Algorithm A10)', () => {
    specificityFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        expect(calculateSpecificity(tc.pattern)).toBe(tc.expected_score);
      });
    });
  });

  // --- 4. ACL Evaluation ---
  const aclFixture = loadFixture('acl_evaluation');
  describe('ACL Evaluation', () => {
    aclFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const rules: ACLRule[] = tc.rules.map((r: any) => ({
          callers: r.callers,
          targets: r.targets,
          effect: r.effect,
          description: r.description || '',
          conditions: r.conditions || null,
        }));
        // §6.2.1 (spec v1.31.0, apcore#112) closed a pattern array's shape at
        // every entry point, which made `empty_callers_matches_none` and
        // `empty_targets_matches_none` unconstructable. Both were deleted from
        // this fixture in the same change (21 cases -> 19), and the branch that
        // asserted the SDK's rejection of them went with them: the shape is now
        // covered by `acl_pattern_arity.json` at both layers, structurally at
        // every door and behaviourally through the §6.1.4.1 backstop.
        const acl = new ACL(rules, tc.default_effect);

        let ctx: Context | null = null;
        const needsContext =
          tc.caller_identity != null ||
          (tc.call_depth ?? 0) > 0 ||
          tc.rules.some((r: any) => r.conditions);

        if (needsContext) {
          const identity = tc.caller_identity
            ? createIdentity(
                tc.caller_id || 'unknown',
                tc.caller_identity.type,
                tc.caller_identity.roles || [],
              )
            : null;
          const callChain = Array(tc.call_depth || 0)
            .fill(0)
            .map((_: unknown, i: number) => `_depth_${i}`);
          ctx = new Context('trace-id', tc.caller_id, callChain, null, identity);
        }

        expect(acl.check(tc.caller_id, tc.target_id, ctx)).toBe(tc.expected);
      });
    });
  });

  // --- 5. Version Negotiation (A14) ---
  const versionFixture = loadFixture('version_negotiation');
  describe('Version Negotiation (Algorithm A14)', () => {
    versionFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        // apcore#92: this branched on `tc.expected_error` being truthy and then
        // asserted `.toThrow()` with no argument — "something threw", the
        // weakest assertion available, satisfied by any error at all and by
        // every wrong wire code. The declared value now reaches `error.code`.
        if (tc.expected_error !== undefined) {
          assertWireCode(tc.id, tc.expected_error, captureThrow(() => negotiateVersion(tc.declared, tc.sdk)));
        } else {
          expect(negotiateVersion(tc.declared, tc.sdk)).toBe(tc.expected);
        }
      });
    });
  });

  // --- 6. Call Chain Safety (A20) ---
  const callChainFixture = loadFixture('call_chain');
  describe('Call Chain Safety (Algorithm A20)', () => {
    /** Invoke the guard with this case's limits, overriding any of them. */
    const callGuard = (
      tc: any,
      chain: readonly string[],
      overrides: { maxCallDepth?: number; maxModuleRepeat?: number } = {},
    ): void =>
      guardCallChain(
        tc.module_id,
        chain,
        overrides.maxCallDepth ?? tc.max_call_depth,
        overrides.maxModuleRepeat ?? tc.max_module_repeat,
      );

    callChainFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        // apcore#93: the negative half dispatched through a driver-local
        // `CALL_CHAIN_ERROR_MAP` and asserted the CLASS, so a fixture value the
        // map did not know became `toThrow(undefined)` — "something threw",
        // satisfied by every wrong code. The declared value now reaches
        // `error.code` through the SDK's own registry (apcore-typescript#34
        // item 2: a private translation table in this file is what let four
        // unreadable config fields look covered).
        if (tc.expected_error !== undefined) {
          assertWireCode(tc.id, tc.expected_error, captureThrow(() => callGuard(tc, tc.call_chain)));
          return;
        }
        if (tc.expected !== 'ok') {
          throw new Error(
            `conformance driver: case '${tc.id}' states an expectation the driver does ` +
              `not recognise (${JSON.stringify(expectationKeys(tc))} => ` +
              `${JSON.stringify(tc.expected)}). Teach the driver, do not skip it.`,
          );
        }

        // Positive case. apcore#93: the whole assertion was `.not.toThrow()`,
        // which a `guardCallChain` that returned immediately also satisfies —
        // the fixture's own `driver_contract.positive_cases_need_a_post_condition`
        // names this shape. Assert what is observably true after a guard that
        // ACCEPTED the chain.
        const chain = [...tc.call_chain];
        expect(callGuard(tc, chain)).toBeUndefined();
        expect(chain).toEqual(tc.call_chain); // MUST NOT mutate the caller's chain

        // Boundary probe: the guard accepted this chain because it is WITHIN the
        // limits, not because it inspects nothing. Re-run with the one limit the
        // chain sits under tightened by one and require the matching rejection.
        // Skipped where the chain cannot be pushed over a LEGAL limit
        // (empty_chain, single_element — max_* < 1 is itself invalid input).
        const depth = chain.length;
        if (depth >= 2) {
          expect(() => callGuard(tc, chain, { maxCallDepth: depth - 1 })).toThrow(
            CallDepthExceededError,
          );
        }
        const repeats = chain.filter((id: string) => id === tc.module_id).length;
        if (repeats >= 2) {
          expect(() => callGuard(tc, chain, { maxModuleRepeat: repeats - 1 })).toThrow(
            CallFrequencyExceededError,
          );
        }
      });
    });
  });

  // --- 7. Error Code Collision (A17) ---
  const errorCodeFixture = loadFixture('error_codes');
  describe('Error Code Collision (Algorithm A17)', () => {
    errorCodeFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const registry = new ErrorCodeRegistry();
        // Codes this case registers, read off the fixture so the post-condition
        // below stays fixture-driven rather than a hardcoded literal.
        const registeredCodes: string[] = [];
        const owned = new Map<string, string[]>();

        const run = () => {
          const doRegister = (moduleId: string, code: string) => {
            registry.register(moduleId, new Set([code]));
            registeredCodes.push(code);
            owned.set(moduleId, [...(owned.get(moduleId) ?? []), code]);
          };
          if (tc.action === 'register') {
            doRegister(tc.module_id, tc.error_code);
          } else if (tc.action === 'register_sequence') {
            for (const step of tc.steps) doRegister(step.module_id, step.error_code);
          } else if (tc.action === 'register_unregister_register') {
            for (const step of tc.steps) {
              if (step.action === 'register') {
                doRegister(step.module_id, step.error_code);
              } else if (step.action === 'unregister') {
                const released = owned.get(step.module_id) ?? [];
                registry.unregister(step.module_id);
                owned.delete(step.module_id);
                // Observable post-condition on the unregister itself: the
                // released codes must actually leave the registry, otherwise
                // "unregister allows reuse" is proven by the reuse alone and an
                // unregister() that did nothing would still look correct.
                for (const code of released) {
                  expect(registry.allCodes.has(code)).toBe(false);
                }
              } else {
                throw new Error(
                  `conformance driver: case '${tc.id}' has a step action ` +
                    `${JSON.stringify(step.action)} the driver does not recognise. ` +
                    `Teach the driver, do not skip it.`,
                );
              }
            }
          } else {
            throw new Error(
              `conformance driver: case '${tc.id}' has action ${JSON.stringify(tc.action)} ` +
                `the driver does not recognise. Teach the driver, do not skip it.`,
            );
          }
        };

        // apcore#92: the old driver branched on `expected_error` being PRESENT
        // and then asserted the class `ErrorCodeCollisionError` — never the wire
        // code the fixture declares — while every `expected: "ok"` case asserted
        // nothing at all and was satisfied by a register() that did nothing.
        if (tc.expected_error !== undefined) {
          assertWireCode(tc.id, tc.expected_error, captureThrow(run));
        } else if (tc.expected === 'ok') {
          run();
          // Observable post-condition: an accepted registration is queryable.
          expect(registeredCodes.length).toBeGreaterThan(0);
          for (const code of registeredCodes) {
            expect(registry.allCodes.has(code)).toBe(true);
          }
        } else {
          throw new Error(
            `conformance driver: case '${tc.id}' states an expectation the driver does ` +
              `not recognise (${JSON.stringify(expectationKeys(tc))} => ` +
              `${JSON.stringify(tc.expected)}). Teach the driver, do not skip it.`,
          );
        }
      });
    });
  });

  // --- 8. Config Env Mapping (A12-NS) ---
  const configEnvFixture = loadFixture('config_env');
  // apcore#93: `nested_path_match` was quarantined here as a "pre-existing SDK
  // bug: auto mode with max_depth=2 cannot resolve ROUTER_MAX_TIMEOUT". The
  // quarantine is STALE — `envSuffixToDotPathWithDepth('ROUTER_MAX_TIMEOUT', 2)`
  // returns `router.max_timeout` and `config.get()` reads 30000 back, matching
  // apcore-python's identical algorithm. It is driven with the rest.

  describe('Config Env Mapping (Algorithm A12-NS)', () => {
    beforeEach(() => {
      _globalNsRegistry.clear();
      _globalEnvMap.clear();
      _envMapClaimed.clear();
      _envPrefixUsed.clear();
      // Pins the file selector to a fixed value so a developer's own
      // APCORE_CONFIG_FILE cannot vary what these cases see. Inert since
      // apcore#88: the variable is no longer an override at all, so it can
      // neither reach `data` nor inject the phantom `config.file` it used to.
      vi.stubEnv('APCORE_CONFIG_FILE', '/dev/null');
    });

    configEnvFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const envStyle = tc.env_style || 'auto';
        configEnvFixture.namespaces.forEach((ns: any) => {
          if (ns.name === 'global' && ns.env_map) {
            Config.envMap(ns.env_map);
          } else {
            Config.registerNamespace({
              name: ns.name,
              envPrefix: ns.env_prefix,
              envMap: ns.env_map,
              maxDepth: ns.max_depth || 5,
              envStyle,
            });
          }
        });

        vi.stubEnv(tc.env_var, tc.env_value);

        let data: Record<string, any> = { apcore: {} };
        configEnvFixture.namespaces.forEach((ns: any) => {
          if (ns.name !== 'global') data.apcore[ns.name] = {};
        });

        data = applyNamespaceEnvOverrides(data);
        data = applyEnvOverrides(data);

        const config = new Config(data, envStyle);
        // @ts-ignore
        config._mode = 'namespace';

        if (tc.expected_path === null) {
          expect(config.get(tc.env_var)).toBeUndefined();
        } else {
          const result = config.get(tc.expected_path);
          const actualStr = String(result).toLowerCase();
          const expectedStr = String(tc.expected_value).toLowerCase();
          expect(actualStr).toBe(expectedStr);
        }

        vi.unstubAllEnvs();
      });
    });
  });

  // --- 9. Context Serialization ---
  const ctxSerFixture = loadFixture('context_serialization');
  describe('Context Serialization', () => {
    const standardCases = ctxSerFixture.test_cases.filter((tc: any) => !tc.sub_cases);
    const subCaseEntries = ctxSerFixture.test_cases.filter((tc: any) => tc.sub_cases);

    standardCases.forEach((tc: any) => {
      it(tc.id, () => {
        const input = tc.input;
        const expected = tc.expected;

        if (tc.id === 'deserialization_round_trip') {
          const ctx = Context.deserialize(input as Record<string, unknown>);
          expect(ctx.traceId).toBe(expected.trace_id);
          expect(ctx.callerId).toBe(expected.caller_id);
          expect(ctx.callChain).toEqual(expected.call_chain);
          if (expected.identity_id != null) {
            expect(ctx.identity).not.toBeNull();
            expect(ctx.identity!.id).toBe(expected.identity_id);
            expect(ctx.identity!.type).toBe(expected.identity_type);
          }
          expect(expected.data_contains in ctx.data).toBe(true);
          return;
        }

        if (tc.id === 'unknown_context_version_warns_but_proceeds') {
          const ctx = Context.deserialize(input as Record<string, unknown>);
          expect(expected.should_succeed).toBe(true);
          expect(ctx.traceId).toBe(expected.trace_id);
          return;
        }

        // Build context from fixture input and serialize
        const identity = input.identity
          ? createIdentity(
              input.identity.id,
              input.identity.type ?? 'user',
              input.identity.roles ?? [],
              input.identity.attrs ?? {},
            )
          : null;

        const ctx = new Context(
          input.trace_id ?? '',
          input.caller_id ?? null,
          input.call_chain ?? [],
          null,
          identity,
          input.redacted_inputs ?? null,
          input.data ?? {},
        );

        const result = ctx.serialize();

        if (tc.id === 'redacted_inputs_serialized') {
          expect(result.trace_id).toBe(expected.trace_id);
          expect(result.redacted_inputs).toEqual(expected.redacted_inputs);
          return;
        }

        expect(result._context_version).toBe(expected._context_version);
        expect(result.trace_id).toBe(expected.trace_id);
        expect(result.caller_id).toBe(expected.caller_id);
        expect(result.call_chain).toEqual(expected.call_chain);
        expect(result.identity).toEqual(expected.identity);
        expect(result.data).toEqual(expected.data);
      });
    });

    // Identity type sub-cases
    if (subCaseEntries.length > 0) {
      const subCases = subCaseEntries[0].sub_cases;
      subCases.forEach((sub: any) => {
        it(`identity_type_${sub.expected_type}`, () => {
          const idData = sub.input_identity;
          const identity = createIdentity(
            idData.id,
            idData.type,
            idData.roles ?? [],
            idData.attrs ?? {},
          );
          const ctx = new Context('test-trace', null, [], null, identity);
          const serialized = ctx.serialize();
          expect((serialized.identity as any).type).toBe(sub.expected_type);

          const restored = Context.deserialize(serialized as Record<string, unknown>);
          expect(restored.identity).not.toBeNull();
          expect(restored.identity!.type).toBe(sub.expected_type);
        });
      });
    }
  });

  // --- 10. Schema Validation (S4.15) ---
  const schemaValFixture = loadFixture('schema_validation');
  describe('Schema Validation', () => {
    // apcore#93: this block used to lose 11 of its 16 cases at once.
    //
    //   * `try { expect(result.valid).toBe(expectedValid) } catch { if
    //     (expectedValid) throw }` — a catch-all that SWALLOWS the assertion
    //     itself. Every `expected_valid: true` case became unfalsifiable the
    //     moment the expectation was flipped, because the flipped value routed
    //     the failure into the silent arm. Only the five negative cases could
    //     ever go red.
    //   * `if (typeof input !== 'object') return` quietly dropped
    //     `empty_schema_accepts_string`, whose whole point is a non-object input.
    //   * two ids were quarantined in an XFAIL set. Both quarantines are gone:
    //     `empty_schema_accepts_string` was stale (an empty schema converts to
    //     `{}` and accepts a string today), and `wrong_type_string_for_integer`
    //     was a real gap in `SchemaValidator(true)`, now closed by the
    //     pydantic-lax `coerceValue` pre-pass that apcore-python and apcore-rust
    //     already shipped.
    //
    // A conversion failure is now a test failure, never a silent "treat as
    // invalid".
    schemaValFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const typeboxSchema = jsonSchemaToTypeBox(tc.schema);
        const input = tc.input;

        /** Validate in one mode and assert the fixture's declared verdict. */
        const assertVerdict = (coerce: boolean, expectedValid: boolean, key: string): void => {
          const result = new SchemaValidator(coerce).validate(input, typeboxSchema);
          expect(
            result.valid,
            `[schema_validation :: ${tc.id}] validate(coerceTypes=${coerce}).valid = ` +
              `${result.valid}, fixture ${key} = ${expectedValid}; ` +
              `errors=${JSON.stringify(result.errors)}`,
          ).toBe(expectedValid);

          // A declared failure path must be the path actually reported,
          // otherwise "invalid" is proven by any unrelated rejection.
          if (!expectedValid && tc.expected_error_path !== undefined) {
            const wanted = String(tc.expected_error_path);
            const paths = result.errors.map((e: any) => e.path);
            expect(
              paths.includes(wanted),
              `[schema_validation :: ${tc.id}] expected an error at ${wanted}, got ${JSON.stringify(paths)}`,
            ).toBe(true);
          }
        };

        if ('expected_valid' in tc) {
          // Coercion must not change the verdict for a case that declares a
          // single one: `coerceValue` only rewrites strings toward a declared
          // numeric/boolean target, so both modes agree here by construction.
          assertVerdict(false, tc.expected_valid, 'expected_valid');
          assertVerdict(true, tc.expected_valid, 'expected_valid');
          return;
        }

        if ('expected_valid_strict' in tc) {
          // Both halves reach an assertion, each against a validator built in
          // the matching mode. Selecting on one key and asserting the other is
          // how the strict expectation became decoration (the apcore#92 shape).
          // The coercing mode is the opt-in LIBRARY-level knob; the
          // module-invocation boundary never coerces (TYPE_MAPPING §17.3) and is
          // covered by schema_keyword_parity.json.
          assertVerdict(false, tc.expected_valid_strict, 'expected_valid_strict');
          assertVerdict(true, tc.expected_valid_coerce, 'expected_valid_coerce');

          if (tc.expected_coerced_value !== undefined) {
            // The coerced VALUE, not just the verdict: a validator that accepted
            // the input without converting it would pass the check above.
            const coerced = new SchemaValidator(true).validateInput(input, typeboxSchema);
            const field = Object.keys(tc.input)[0];
            expect(coerced[field]).toBe(tc.expected_coerced_value);
          }
          return;
        }

        throw new Error(
          `conformance driver: case '${tc.id}' states an expectation the driver does ` +
            `not recognise (${JSON.stringify(expectationKeys(tc))}). ` +
            `Teach the driver, do not skip it.`,
        );
      });
    });
  });

  // --- 11. Config Defaults ---
  const configDefaultsFixture = loadFixture('config_defaults');
  describe('Config Defaults Conformance', () => {
    configDefaultsFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const config = Config.fromDefaults();
        const result = config.get(tc.key);
        expect(result).toEqual(tc.expected);
      });
    });
  });

  // --- 12. Stream Aggregation (deep merge) ---
  const streamAggFixture = loadFixture('stream_aggregation');
  describe('Stream Aggregation Conformance', () => {
    streamAggFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        if (tc.chunks.length === 0) {
          expect(tc.expected).toBeNull();
          return;
        }
        const accumulated: Record<string, unknown> = {};
        for (const chunk of tc.chunks) {
          deepMergeChunk(accumulated, chunk);
        }
        expect(accumulated).toEqual(tc.expected);
      });
    });
  });

  // --- 13. Defaults Schema Conformance ---
  describe('Defaults Schema Conformance', () => {
    const schema = loadSchema('defaults');

    function extractDefaults(
      props: Record<string, any>,
      prefix = '',
    ): Array<[string, unknown]> {
      const results: Array<[string, unknown]> = [];
      for (const [key, prop] of Object.entries(props)) {
        const dotPath = prefix ? `${prefix}.${key}` : key;
        if ('default' in prop) {
          results.push([dotPath, prop.default]);
        }
        if (prop.type === 'object' && prop.properties) {
          results.push(...extractDefaults(prop.properties, dotPath));
        }
      }
      return results;
    }

    const defaults = extractDefaults((schema as any).properties ?? {});

    defaults.forEach(([dotPath, expected]) => {
      it(`default: ${dotPath}`, () => {
        const config = Config.fromDefaults();
        const actual = config.get(dotPath as string);
        expect(actual).toEqual(expected);
      });
    });
  });

  // --- 14. Sys Module Output Schema Conformance ---
  describe('Sys Module Output Schema Conformance', () => {
    const cases = [
      { schema: 'sys-control-update-config', required: ['success', 'key', 'old_value', 'new_value'] },
      { schema: 'sys-control-reload-module', required: ['success', 'module_id'] },
      { schema: 'sys-control-toggle-feature', required: ['success', 'module_id', 'enabled'] },
      { schema: 'sys-health-summary', required: ['project', 'summary', 'modules'] },
      { schema: 'sys-health-module', required: ['module_id', 'status', 'total_calls', 'error_count', 'error_rate'] },
      { schema: 'sys-manifest-module', required: ['module_id', 'description'] },
      { schema: 'sys-manifest-full', required: ['project_name', 'module_count', 'modules'] },
    ];

    cases.forEach(({ schema: name, required }) => {
      it(`${name} schema matches spec`, () => {
        const specSchema = loadSchema(name) as any;
        expect(new Set(specSchema.required)).toEqual(new Set(required));
        for (const key of required) {
          expect(specSchema.properties).toHaveProperty(key);
        }
      });
    });
  });

  // --- identity_system ---
  const identityFixture = loadFixture('identity_system');
  describe('Identity System', () => {
    identityFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const identity = createIdentity(
          tc.input_id,
          tc.input_type ?? 'user',
          tc.input_roles ?? [],
          tc.input_attrs ?? {},
        );
        let asserted = 0;
        if (tc.expected_type !== undefined) {
          expect(identity.type).toBe(tc.expected_type);
          asserted += 1;
        }
        if (tc.expected_roles !== undefined) {
          expect(identity.roles).toEqual(tc.expected_roles);
          asserted += 1;
        }
        if (tc.expected_attrs !== undefined) {
          expect(identity.attrs).toEqual(tc.expected_attrs);
          asserted += 1;
        }
        if (tc.expected !== undefined) {
          // apcore#92: `expected` used to be the prose string
          // "child.identity === parent.identity" — a sentence in a value slot,
          // which no driver can assert, so this branch was keyed on the case id
          // and compared the child's identity against the case INPUT. The
          // fixture value was decoration. It is now four fields; assert each.
          const parent = new Context('trace', null, [], null, identity);
          const child = parent.child(tc.child_module_id);
          expect(child.identity?.id).toBe(tc.expected.child_identity_id);
          expect(child.identity?.type).toBe(tc.expected.child_identity_type);
          expect(child.identity?.roles).toEqual(tc.expected.child_identity_roles);
          expect(child.identity === parent.identity).toBe(tc.expected.child_identity_equals_parent);
          asserted += 4;
        }
        if (asserted === 0) {
          throw new Error(
            `conformance driver: case '${tc.id}' states expectations the driver does not ` +
              `read (${JSON.stringify(expectationKeys(tc))}). Teach the driver, do not skip it.`,
          );
        }
      });
    });
  });

  // --- annotations_extra_round_trip ---
  const annotationsFixture = loadFixture('annotations_extra_round_trip');
  /**
   * Pilot-tolerant comparator for the v0.21.0 `discoverable` rollout.
   *
   * Per the ephemeral-namespace rollout (protocol-spec §2.5.1), the canonical
   * `annotations_extra_round_trip.json` fixture MUST NOT be updated to
   * require `discoverable` until ALL three SDKs have shipped support.
   * During the rollout window, SDKs that have shipped the field strip it
   * from the actual serialized output before equality comparison so the
   * conformance suite stays green. apcore-python PR #26 implements the
   * same pattern; once the synchronized fixture update lands, this
   * helper can be removed.
   */
  const stripDiscoverableForPilot = (
    actual: Record<string, unknown>,
    expected: Record<string, unknown>,
  ): Record<string, unknown> => {
    if ('discoverable' in actual && !('discoverable' in expected)) {
      const { discoverable: _omit, ...rest } = actual;
      return rest;
    }
    return actual;
  };
  describe('Annotations Extra Round-trip', () => {
    annotationsFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        // D-115: the malformed-value cases assert that the REST survives, which
        // `expected_deserialized_extra` alone cannot say — an SDK that rejected
        // the whole descriptor never reaches the assertion, and one that dropped
        // everything would satisfy it.
        if (tc.expected_survivors) {
          const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
          const ann = annotationsFromJSON(tc.input_serialized) as unknown as Record<
            string,
            unknown
          >;
          const warned = warn.mock.calls.length > 0;
          warn.mockRestore();
          expect(ann['extra']).toEqual(tc.expected_deserialized_extra);
          for (const [wireKey, want] of Object.entries(tc.expected_survivors)) {
            const field = wireKey === 'cache_ttl' ? 'cacheTtl' : wireKey;
            expect(ann[field]).toEqual(want);
          }
          if (tc.expected_warning === false) expect(warned).toBe(false);
          return;
        }
        if (tc.id === 'deserialize_legacy_flattened_form' || tc.id === 'nested_takes_precedence_over_flattened') {
          // Deserialize from wire format (may include legacy flattened keys)
          const ann = annotationsFromJSON(tc.input_serialized);
          expect(ann.extra).toEqual(tc.expected_deserialized_extra);
          if (tc.expected_reserialized) {
            const reserialized = annotationsToJSON(ann);
            expect(stripDiscoverableForPilot(reserialized, tc.expected_reserialized))
              .toEqual(tc.expected_reserialized);
          }
          return;
        }

        if (tc.id === 'producer_must_not_emit_both_forms') {
          const ann = createAnnotations({
            readonly: tc.input.readonly,
            destructive: tc.input.destructive,
            idempotent: tc.input.idempotent,
            requiresApproval: tc.input.requires_approval,
            openWorld: tc.input.open_world,
            streaming: tc.input.streaming,
            cacheable: tc.input.cacheable,
            cacheTtl: tc.input.cache_ttl,
            cacheKeyFields: tc.input.cache_key_fields,
            paginated: tc.input.paginated,
            paginationStyle: tc.input.pagination_style,
            discoverable: tc.input.discoverable,
            extra: tc.input.extra ?? {},
          });
          const serialized = annotationsToJSON(ann);
          for (const forbiddenKey of tc.forbidden_root_keys) {
            expect(Object.keys(serialized)).not.toContain(forbiddenKey);
          }
          return;
        }

        // Standard round-trip cases
        const ann = createAnnotations({
          readonly: tc.input.readonly,
          destructive: tc.input.destructive,
          idempotent: tc.input.idempotent,
          requiresApproval: tc.input.requires_approval,
          openWorld: tc.input.open_world,
          streaming: tc.input.streaming,
          cacheable: tc.input.cacheable,
          cacheTtl: tc.input.cache_ttl,
          cacheKeyFields: tc.input.cache_key_fields,
          paginated: tc.input.paginated,
          paginationStyle: tc.input.pagination_style,
          discoverable: tc.input.discoverable,
          extra: tc.input.extra ?? {},
        });
        const serialized = annotationsToJSON(ann);
        expect(stripDiscoverableForPilot(serialized, tc.expected_serialized))
          .toEqual(tc.expected_serialized);

        // Deserialize back and check extra
        const restored = annotationsFromJSON(serialized as Record<string, unknown>);
        expect(restored.extra).toEqual(tc.expected_deserialized_extra);
      });
    });
  });

  // --- approval_gate ---
  //
  // Not asserted here: the fixture's HTTP-status expectation (403 for
  // APPROVAL_DENIED, 202 for APPROVAL_PENDING). apcore-typescript exposes no
  // code-to-HTTP mapping — neither does apcore-python or apcore-rust — because
  // the mapping lives in the transport adapters (express-apcore, fastapi-apcore)
  // rather than in the SDK. There is nothing on the thrown error to observe, and
  // restating the number from the fixture would assert nothing. The wire code,
  // which IS the SDK's contract, is asserted below.
  /**
   * Resolve a case's declared approval requirement.
   *
   * Two shapes. The original cases carry a single `module_requires_approval`
   * boolean, which each SDK was free to satisfy from whichever governance
   * source it preferred — and that is exactly how an approval bypass survived
   * this fixture: apcore-rust decided gate firing from the registry DESCRIPTOR
   * while its test module left `annotations()` at the default, so all three
   * SDKs passed the same case off two different sources of truth (D-96).
   *
   * The newer cases state `governance_sources` with the module and descriptor
   * declarations SEPARATELY. apcore-typescript derives the descriptor from the
   * module, so the two are not independently settable here; per D-96 the gate
   * fires on the UNION of the sources an implementation has, and the union of
   * what this SDK can express is `module || descriptor`. That reaches the same
   * expected outcome, which is why the union costs this SDK nothing.
   */
  function declaredRequiresApproval(tc: any): boolean {
    const sources = tc.governance_sources;
    if (sources === undefined || sources === null) {
      return Boolean(tc.module_requires_approval);
    }
    return Boolean(sources.module) || Boolean(sources.descriptor);
  }

  describe('Approval Gate', () => {
    const approvalFixture = loadFixture('approval_gate');

    approvalFixture.test_cases.forEach((tc: any) => {
      it(tc.id, async () => {
        // Build a mock handler whenever the fixture says one is configured.
        // (It used to be built only when `approval_result` was also present,
        // which made `skip_when_approval_not_required` — handler configured,
        // module does not declare requires_approval — run with no handler at
        // all, i.e. it tested the wrong skip branch.)
        let handler: any = null;
        let handlerCalls = 0;
        if (tc.approval_handler_configured) {
          const raw = tc.approval_result ?? { status: 'approved' };
          const result: ApprovalResult = {
            status: raw.status,
            approvedBy: raw.approved_by ?? null,
            reason: raw.reason ?? null,
            approvalId: raw.approval_id ?? null,
            metadata: raw.metadata ?? null,
          };
          handler = {
            requestApproval: async () => {
              handlerCalls += 1;
              return result;
            },
            checkApproval: async () => {
              handlerCalls += 1;
              return result;
            },
          };
        }

        const gate = new BuiltinApprovalGate(handler);

        const mod: Record<string, unknown> = {
          annotations: {
            requiresApproval: declaredRequiresApproval(tc),
          },
          description: null,
          tags: [],
        };

        const ctx = Context.create();
        const pipeCtx: any = {
          moduleId: 'test.module',
          module: mod,
          inputs: {},
          context: ctx,
        };

        if (tc.expected.outcome === 'proceed') {
          const result = await gate.execute(pipeCtx);
          expect(result.action).toBe('continue');
        } else {
          // error expected
          let thrown: Error | null = null;
          try {
            await gate.execute(pipeCtx);
          } catch (e) {
            thrown = e as Error;
          }
          expect(thrown).not.toBeNull();
          // The wire code is the cross-language contract; the class name is
          // per-SDK (apcore-rust has ErrorCode variants, not classes).
          expect((thrown as ModuleError).code).toBe(tc.expected.error_code);
          if (tc.expected.error_code === 'APPROVAL_DENIED') {
            expect(thrown).toBeInstanceOf(ApprovalDeniedError);
          } else if (tc.expected.error_code === 'APPROVAL_PENDING') {
            expect(thrown).toBeInstanceOf(ApprovalPendingError);
            expect((thrown as ApprovalPendingError).approvalId).toBe(tc.expected.approval_id);
          }
        }
        // Whether the gate actually consulted the handler — the discriminator
        // for both skip cases, which otherwise look identical to "proceed".
        expect(handlerCalls > 0).toBe(tc.expected.gate_invoked);
      });
    });
  });

  // --- binding_errors / binding_yaml_canonical ---
  // Driven by tests/conformance-bindings.test.ts. (They were skipped here with
  // the reason "BindingLoader requires real file I/O and dynamic imports",
  // which was wrong — both fixtures assert error-message parity and YAML
  // round-trip, exactly as apcore-python and apcore-rust drive them.)

  // --- dependency_version_constraints ---
  const depVersionFixture = loadFixture('dependency_version_constraints');
  describe('Dependency Version Constraints', () => {
    depVersionFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        // Build modules list: [moduleId, DependencyInfo[]]
        const modulesList: Array<[string, Array<{ moduleId: string; version: string | null; optional: boolean }>]> =
          tc.modules.map((m: any) => [
            m.module_id,
            (m.dependencies ?? []).map((d: any) => ({
              moduleId: d.module_id,
              version: d.version ?? null,
              optional: d.optional ?? false,
            })),
          ]);

        // Build version map
        const moduleVersions = new Map<string, string>();
        // D-79 needs a dependency that is a KNOWN id and absent from the batch
        // — the only way to stall Kahn's algorithm without a cycle. Cases that
        // do not declare `known_ids` keep the default.
        const knownIds: Set<string> | null = tc.known_ids ? new Set<string>(tc.known_ids) : null;
        for (const m of tc.modules) {
          if (m.version) moduleVersions.set(m.module_id, m.version);
        }

        if (tc.expected.outcome === 'ok') {
          let loadOrder: string[];
          const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
          try {
            expect(() => {
              loadOrder = resolveDependencies(modulesList, knownIds, moduleVersions);
            }).not.toThrow();
            if (tc.expected.load_order) {
              expect(loadOrder!).toEqual(tc.expected.load_order);
            }
            for (const [from, to] of (tc.expected['skipped_edges'] ?? []) as string[][]) {
              // A skipped edge has two observable consequences. (1) The SDK says
              // so: resolveDependencies warns naming both ends before dropping
              // the edge. (2) The edge is gone from the graph, so `from` is no
              // longer forced after `to` — with the deterministic sorted
              // zero-in-degree queue that puts `from` first. Keeping the edge
              // would invert the order.
              const messages = warn.mock.calls.map((c) => c.map(String).join(' '));
              expect(
                messages.some(
                  (m) => m.includes(from) && m.includes(to) && m.includes('skipping'),
                ),
              ).toBe(true);
              expect(loadOrder!.indexOf(from)).toBeLessThan(loadOrder!.indexOf(to));
            }
          } finally {
            warn.mockRestore();
          }
        } else if (tc.expected.outcome === 'error') {
          // apcore#92 §1: this was an unconditional `else` asserting the CLASS
          // `DependencyVersionMismatchError`, so `error_code`, `module_id`,
          // `dependency_id`, `required` and `actual` never reached an assertion
          // and mutating any of them left the suite green. All seven `*_violated`
          // cases — every negative branch in this fixture — were unpinned, which
          // is the worst possible split for a constraint checker: an
          // implementation that always reported "satisfied" passed everything
          // this fixture actually ran.
          const thrown = captureThrow(() => resolveDependencies(modulesList, knownIds, moduleVersions));
          assertWireCode(tc.id, tc.expected['error_code'], thrown);
          // The remaining `expected` fields are the error's details. Asserted
          // against the WIRE form (`toJSON().details`, snake_cased per A-D-008)
          // so the fixture's own spelling reaches the assertion directly —
          // no driver-local key map (apcore-typescript#34 item 2).
          const details = ((thrown as ModuleError).toJSON()['details'] ?? {}) as Record<
            string,
            unknown
          >;
          for (const [key, want] of Object.entries(tc.expected)) {
            if (key === 'outcome' || key === 'error_code') continue;
            expect(details[key]).toBe(want);
          }
        } else {
          throw new Error(
            `conformance driver: case '${tc.id}' declares outcome ` +
              `${JSON.stringify(tc.expected.outcome)}, which the driver does not ` +
              `recognise. Teach the driver, do not skip it.`,
          );
        }
      });
    });
  });

  // --- middleware_on_error_recovery (A11) ---
  // Mirrors apcore-python tests/test_conformance.py::test_middleware_on_error_recovery.
  // Each fixture middleware records invocations; on the error path the manager's
  // executeOnError walks the chain in reverse and short-circuits at the first
  // non-null dict (first-dict-wins). On the success path executeAfter is called
  // and any after() return value is allowed to mutate the output (the fixture
  // asserts the on_error path is not entered).
  const middlewareOnErrorFixture = loadFixture('middleware_on_error_recovery');
  describe('Middleware On-Error Recovery (A11)', () => {
    class FixtureAfterMiddleware extends Middleware {
      readonly mwId: string;
      private readonly _returns: Record<string, unknown> | null;
      invoked = false;

      constructor(mwId: string, returns: Record<string, unknown> | null) {
        super();
        this.mwId = mwId;
        this._returns = returns;
      }

      override after(): Record<string, unknown> | null {
        this.invoked = true;
        return this._returns;
      }

      override onError(): Record<string, unknown> | null {
        this.invoked = true;
        return this._returns;
      }
    }

    middlewareOnErrorFixture.test_cases.forEach((tc: any) => {
      it(tc.id, async () => {
        const manager = new MiddlewareManager();
        const instances = new Map<string, FixtureAfterMiddleware>();
        for (const mwSpec of tc.after_middleware) {
          const mw = new FixtureAfterMiddleware(mwSpec.id, mwSpec.returns ?? null);
          instances.set(mwSpec.id, mw);
          manager.add(mw);
        }

        const ctx = Context.create();
        const inputs: Record<string, unknown> = {};

        let recovery: Record<string, unknown> | null = null;
        let finalOutput: Record<string, unknown> | null = null;

        if (tc.module_raises_error) {
          const err = new ModuleError('TEST_ERROR', 'test error');
          const executed = manager.snapshot();
          recovery = (await manager.executeOnError(
            'test.module',
            inputs,
            err,
            ctx,
            executed,
          )) as Record<string, unknown> | null;
        } else {
          const moduleOutput = (tc.module_output ?? {}) as Record<string, unknown>;
          finalOutput = await manager.executeAfter('test.module', inputs, moduleOutput, ctx);
        }

        // At least one declared middleware must have been invoked. The error
        // path short-circuits on the first dict, so reverse-order execution
        // can leave earlier-declared middlewares untouched.
        const invokedExpected = (tc.expected.after_middleware_invoked as string[]).filter(
          (id) => instances.get(id)!.invoked,
        );
        expect(invokedExpected.length).toBeGreaterThan(0);

        if (tc.expected.outcome === 'error') {
          if (tc.module_raises_error) {
            expect(recovery === null || typeof recovery !== 'object').toBe(true);
          }
        } else {
          if (tc.module_raises_error) {
            expect(recovery).not.toBeNull();
            expect(typeof recovery).toBe('object');
            const expectedResults = (tc.after_middleware as any[])
              .map((mw) => mw.returns)
              .filter((r) => r !== null && r !== undefined);
            // Reverse-order short-circuit means the winner is one of the declared dicts.
            expect(expectedResults).toContainEqual(recovery);
          } else {
            // Success path: executeAfter is allowed to mutate output. The contract
            // verified here is that an output is produced (on_error not invoked).
            expect(finalOutput).not.toBeNull();
          }
        }
      });
    });
  });

  // --- multi_module_discovery (PROTOCOL_SPEC §2.1.1) ---
  const multiModuleFixture = loadFixture('multi_module_discovery');
  describe('Multi-Module Discovery (PROTOCOL_SPEC §2.1.1)', () => {
    const CANONICAL_ID_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$/;

    multiModuleFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        // Snake-case conversion test cases have only class_name in input
        if ('class_name' in tc.input) {
          expect(classNameToSegment(tc.input.class_name)).toBe(tc.expected.class_segment);
          return;
        }

        const { file_path, extensions_root, classes } = tc.input;
        // Spec v1.50.0 D-107: the per-class `multi_class` marker is the ONLY
        // multi-class opt-in. The fixture used to carry a file-level
        // `multi_class_enabled`, the model decision-log D-06 withdrew, and the
        // driver briefly read it as a fallback while the fixture was corrected.
        // That fallback is gone deliberately: tolerating both models is exactly
        // what let three SDKs implement three different ones and stay green, so
        // a driver that accepts the withdrawn spelling cancels out the
        // discrimination the corrected fixture exists to provide.
        const descriptors = (classes as any[]).map((c: any) => ({
          name: c.name,
          implementsModule: c.implements_module,
          multiClass: c.multi_class === true,
        }));

        if (tc.expected.error) {
          let thrown: unknown = null;
          try {
            discoverMultiClass(file_path, descriptors, extensions_root);
          } catch (e) {
            thrown = e;
          }
          expect(thrown).not.toBeNull();
          expect(thrown).toBeInstanceOf(ModuleIdConflictError);
          const err = thrown as ModuleIdConflictError;
          expect(err.code).toBe(tc.expected.error.code);
          expect(err.details['conflictingSegment']).toBe(tc.expected.error.conflicting_segment);
          return;
        }

        const result = discoverMultiClass(file_path, descriptors, extensions_root);
        const moduleIds = result.map(r => r.moduleId);

        expect(moduleIds).toEqual(tc.expected.module_ids);

        if (tc.expected.grammar_valid === true) {
          for (const id of moduleIds) {
            expect(CANONICAL_ID_RE.test(id)).toBe(true);
          }
        }
      });
    });
  });

  // --- Core Schema Structure Conformance ---
  describe('Core Schema Structure Conformance', () => {
    it('acl-config schema has required fields', () => {
      const s = loadSchema('acl-config') as any;
      expect(s.required).toContain('rules');
      expect(s.properties).toHaveProperty('rules');
      expect(s.properties).toHaveProperty('default_effect');
      expect(s.properties).toHaveProperty('audit');
    });

    it('apcore-config schema requires exactly version and project', () => {
      // PROTOCOL_SPEC §9.1: a key is required only when it has no canonical
      // default. `extensions`, `schema` and `acl` all carry defaults in
      // defaults.schema.json, so requiring them rejected configurations the
      // framework resolves fine; they were dropped from `required`.
      const s = loadSchema('apcore-config') as any;
      expect(s.required).toContain('version');
      expect(s.required).toContain('project');
      expect(s.required).toHaveLength(2);
      for (const key of ['extensions', 'schema', 'acl']) {
        expect(s.required).not.toContain(key);
      }
    });

    it('binding schema has required fields', () => {
      const s = loadSchema('binding') as any;
      expect(s.required).toContain('bindings');
      expect(s.$defs.BindingEntry.required).toContain('module_id');
      expect(s.$defs.BindingEntry.required).toContain('target');
    });

    it('module-meta schema has required properties', () => {
      const s = loadSchema('module-meta') as any;
      for (const key of ['description', 'dependencies', 'annotations', 'version']) {
        expect(s.properties).toHaveProperty(key);
      }
    });

    it('module-schema schema has required fields', () => {
      const s = loadSchema('module-schema') as any;
      for (const key of ['module_id', 'description', 'input_schema', 'output_schema']) {
        expect(s.required).toContain(key);
      }
    });
  });

  // --------------------------------------------------------------------------
  // Context.create trace_parent handling (PROTOCOL_SPEC §10.5)
  // --------------------------------------------------------------------------
  describe('Context.create trace_parent', () => {
    const fixture = loadFixture('context_trace_parent');

    fixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const incoming: string | null = tc.input.trace_parent_trace_id;
        const expected = tc.expected;

        // Spy on console.warn to capture the WARN log.
        const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

        const traceParent =
          incoming === null
            ? null
            : {
                version: '00',
                traceId: incoming,
                parentId: '0000000000000001',
                traceFlags: '01',
              };

        const ctx = Context.create(null, traceParent as any);

        // trace_id must always be valid 32-char lowercase hex
        expect(ctx.traceId).toMatch(/^[0-9a-f]{32}$/);
        expect(ctx.traceId).not.toBe('0'.repeat(32));
        expect(ctx.traceId).not.toBe('f'.repeat(32));

        if (expected.regenerated) {
          expect(ctx.traceId).not.toBe(incoming);
        } else {
          expect(ctx.traceId).toBe(expected.trace_id);
        }

        const warnSeen = warnSpy.mock.calls.some((call) =>
          String(call[0]).includes('Invalid trace_id format'),
        );
        expect(warnSeen).toBe(expected.warn_logged);

        warnSpy.mockRestore();
      });
    });
  });

  // --------------------------------------------------------------------------
  // Schema System Hardening (Issue #44, §4.15)
  // --------------------------------------------------------------------------

  // --- SH-1 / SH-2 share the module-invocation path ---
  //
  // DRIVER CONTRACT: both fixtures MUST be driven through `SchemaLoader.resolve()`
  // — RefResolver followed by `jsonSchemaToTypeBox()` — and not through the
  // converter alone. Calling `jsonSchemaToTypeBox()` on the raw fixture schema
  // skips `$ref` resolution entirely, which is exactly where the recursive-schema
  // defect lived: the converter handled `{"$ref": "#"}` fine while the resolver in
  // front of it rejected the same schema outright, so conformance stayed green on
  // a schema no module could actually register.
  const moduleSchema = (schema: Record<string, unknown>): TSchema => {
    const loader = new SchemaLoader(new Config({}), fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-conf-')));
    const definition: SchemaDefinition = {
      moduleId: 'conformance.schema_hardening',
      description: 'conformance fixture',
      inputSchema: schema,
      outputSchema: {},
      errorSchema: null,
      definitions: {},
      version: '1.0.0',
      documentation: null,
      schemaUrl: null,
    };
    return loader.resolve(definition)[0].schema;
  };

  // --- SH-1. Union type: anyOf/oneOf/allOf exhaustive evaluation ---
  describe('Schema Hardening: Union Type Evaluation', () => {
    const fixture = loadFixture('schema_hardening_union');

    fixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const validator = new SchemaValidator(false);
        const result = validator.validate(tc.input, moduleSchema(tc.schema));
        expect(result.valid).toBe(tc.expected.valid);
        if (tc.expected.error_code !== null) {
          expect(result.errorCode).toBe(tc.expected.error_code);
        }
      });
    });
  });

  // --- SH-2. Recursive schema: TreeNode self-referencing $ref ---
  describe('Schema Hardening: Recursive Schema', () => {
    const fixture = loadFixture('schema_hardening_recursive');
    const typeboxSchema = moduleSchema(fixture.schema);

    fixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const validator = new SchemaValidator(false);
        const result = validator.validate(tc.input, typeboxSchema);
        expect(result.valid).toBe(tc.expected.valid);
      });
    });
  });

  // --- SH-3. Constraints: min/max, minLength/maxLength, pattern, not ---
  describe('Schema Hardening: Constraint Enforcement', () => {
    const fixture = loadFixture('schema_hardening_constraints');

    fixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const typeboxSchema = jsonSchemaToTypeBox(tc.schema);
        const validator = new SchemaValidator(false);
        const result = validator.validate(tc.input, typeboxSchema);
        expect(result.valid).toBe(tc.expected.valid);
      });
    });
  });

  // --- SH-4. Semantic format mapping: warn on invalid format, pass structurally ---
  describe('Schema Hardening: Semantic Format Mapping', () => {
    const fixture = loadFixture('schema_hardening_formats');

    fixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
        const typeboxSchema = jsonSchemaToTypeBox(tc.schema);
        const validator = new SchemaValidator(false);
        const result = validator.validate(tc.input, typeboxSchema);
        const warnLogged = warnSpy.mock.calls.length > 0;
        warnSpy.mockRestore();
        expect(result.valid).toBe(tc.expected.valid);
        expect(warnLogged).toBe(tc.expected.warn_logged);
      });
    });
  });

  // --- SH-5. Content-addressable cache: SHA-256 of canonical JSON ---
  describe('Schema Hardening: Content Hash Cache', () => {
    const fixture = loadFixture('schema_hardening_cache');

    fixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const hash1 = contentHash(tc.schemas[0]);
        const hash2 = contentHash(tc.schemas[1]);
        expect(hash1 === hash2).toBe(tc.expected.same_hash);
      });
    });
  });

  // ---------------------------------------------------------------------------
  // Pipeline Hardening (Issue #33)
  // ---------------------------------------------------------------------------

  function makeHardeningStep(
    name: string,
    opts: {
      ignoreErrors?: boolean;
      throws?: boolean;
    } = {},
  ): Step {
    return {
      name,
      description: `Step ${name}`,
      removable: true,
      replaceable: true,
      ignoreErrors: opts.ignoreErrors,
      execute: async (): Promise<StepResult> => {
        if (opts.throws) throw new Error(`Step ${name} failed`);
        return { action: 'continue' };
      },
    };
  }

  function makeHardeningContext(moduleId = 'test.module'): PipelineContext {
    return {
      moduleId,
      inputs: {},
      context: new Context('trace-id', null, []),
    };
  }

  const pipelineHardeningFixture = loadFixture('pipeline_hardening');

  describe('Pipeline Hardening (Issue #33)', () => {
    // 1. fail_fast_on_step_error
    it('fail_fast_on_step_error', async () => {
      const tc = pipelineHardeningFixture.test_cases.find((t: any) => t.id === 'fail_fast_on_step_error');
      expect(tc).toBeDefined();

      const { step: failingStepName, raises, ignore_errors } = tc.input;
      const stepNames: string[] = tc.expected.steps_executed;

      const steps = stepNames.map((n: string) =>
        makeHardeningStep(n, {
          throws: n === failingStepName && raises === true,
          ignoreErrors: n === failingStepName ? ignore_errors : undefined,
        }),
      );
      const strategy = new ExecutionStrategy('default', steps);
      const engine = new PipelineEngine();
      const ctx = makeHardeningContext();

      let thrown: unknown = null;
      try {
        await engine.run(strategy, ctx);
      } catch (e) {
        thrown = e;
      }

      expect(thrown).not.toBeNull();
      expect(thrown).toBeInstanceOf(PipelineStepError);
      const err = thrown as PipelineStepError;
      expect(err.code).toBe(tc.expected.error_code);
      expect(tc.expected.stopped).toBe(true);

      const executedNames = err.pipelineTrace!.steps
        .filter((s: any) => !s.skipped)
        .map((s: any) => s.name);
      expect(executedNames).toEqual(stepNames);
    });

    // 2. continue_on_ignored_error
    it('continue_on_ignored_error', async () => {
      const tc = pipelineHardeningFixture.test_cases.find((t: any) => t.id === 'continue_on_ignored_error');
      expect(tc).toBeDefined();

      const strategy = new ExecutionStrategy('default', [
        makeHardeningStep('before'),
        makeHardeningStep(tc.input.step, { throws: true, ignoreErrors: tc.input.ignore_errors }),
        makeHardeningStep('after'),
      ]);
      const engine = new PipelineEngine();
      const ctx = makeHardeningContext();

      const [, trace] = await engine.run(strategy, ctx);
      expect(trace.success).toBe(true);
      expect(tc.expected.stopped).toBe(false);
      expect(tc.expected.continued).toBe(true);
    });

    // 3. replace_semantic_no_duplicate
    it('replace_semantic_no_duplicate', async () => {
      const tc = pipelineHardeningFixture.test_cases.find((t: any) => t.id === 'replace_semantic_no_duplicate');
      expect(tc).toBeDefined();

      const strategy = new ExecutionStrategy('default', [
        makeHardeningStep(tc.input.configure_step),
      ]);

      // Call configureStep the number of times specified in the fixture
      for (let i = 0; i < tc.input.times; i++) {
        strategy.configureStep(tc.input.configure_step, makeHardeningStep(tc.input.configure_step));
      }

      const count = strategy.steps.filter((s) => s.name === tc.input.configure_step).length;
      expect(count).toBe(tc.expected.step_count_for_name);
    });

    // 4. run_until_stops_early
    it('run_until_stops_early', async () => {
      const tc = pipelineHardeningFixture.test_cases.find((t: any) => t.id === 'run_until_stops_early');
      expect(tc).toBeDefined();

      const stopAfter: string = tc.input.run_until_after;
      const strategy = new ExecutionStrategy('default', [
        makeHardeningStep('context_creation'),
        makeHardeningStep('module_lookup'),
        makeHardeningStep('execute'),
        makeHardeningStep('return_result'),
      ]);
      const engine = new PipelineEngine();
      const ctx = makeHardeningContext();
      ctx.runUntil = (state: PipelineState) => state.stepName === stopAfter;

      const [, trace] = await engine.run(strategy, ctx);
      expect(trace.success).toBe(true);

      const executedSteps = trace.steps.filter((s) => !s.skipped);
      const lastExecuted = executedSteps[executedSteps.length - 1];
      expect(lastExecuted.name).toBe(tc.expected.last_step_executed);
      // `expect(tc.expected.steps_after_skipped).toBe(true)` used to stand here:
      // it compared the fixture to itself and could not fail on SDK behaviour.
      // The observation is the trace — steps after the run_until stop point must
      // not appear in it at all.
      const allNames = trace.steps.map((s) => s.name);
      if (tc.expected['steps_after_skipped']) {
        expect(allNames).not.toContain('execute');
        expect(allNames).not.toContain('return_result');
      }
    });

    // 5. step_lookup_is_not_linear
    it('step_lookup_is_not_linear', () => {
      const tc = pipelineHardeningFixture.test_cases.find((t: any) => t.id === 'step_lookup_is_not_linear');
      expect(tc).toBeDefined();
      expect(tc.expected.lookup_complexity).toBe('O(1)');

      const stepCount: number = tc.input.step_count;
      const steps = Array.from({ length: stepCount }, (_, i) =>
        makeHardeningStep(`step_${i}`),
      );
      const strategy = new ExecutionStrategy('default', steps);

      // Verify findStepIndex provides O(1) lookup (Map-backed)
      expect(typeof strategy.findStepIndex).toBe('function');
      for (let i = 0; i < stepCount; i++) {
        expect(strategy.findStepIndex(`step_${i}`)).toBe(i);
      }
      expect(strategy.findStepIndex('nonexistent')).toBeUndefined();
    });

    for (const tc of pipelineHardeningFixture.test_cases.filter((c: any) => c.input.custom_step)) {
      it(tc.id, async () => {
        const registry = new Registry();
        const moduleId = 'fixture.module';
        registry.register(moduleId, new FunctionModule({
          execute: () => ({}), moduleId,
          inputSchema: Type.Object({}), outputSchema: Type.Object({}),
          description: 'Step timeout fixture module',
        }));
        const strategy = buildStandardStrategy({
          registry, config: null, acl: null, approvalHandler: null,
          middlewareManager: new MiddlewareManager(),
        });
        const custom = tc.input.custom_step;
        let completed = false;
        strategy.insertAfter(custom.after, {
          name: custom.name, description: 'Fixture-driven timed step',
          removable: true, replaceable: true, timeoutMs: tc.input.timeout_ms,
          execute: async (): Promise<StepResult> => {
            await new Promise((resolve) => setTimeout(resolve, custom.sleep_ms));
            completed = true;
            return { action: 'continue' };
          },
        });
        const executor = new Executor({ registry, strategy });
        if ('call_error_code' in tc.expected) {
          await expect(executor.call(moduleId, {})).rejects.toMatchObject({
            code: tc.expected.call_error_code,
          });
          expect(completed).toBe(false);
        } else if ('call_succeeds' in tc.expected) {
          const result = await executor.call(moduleId, {});
          expect(result).toEqual({});
          expect(completed).toBe(tc.expected.call_succeeds);
        } else {
          throw new Error(`Unhandled timeout expectation in ${tc.id}`);
        }
      });
    }

  });

  // ---------------------------------------------------------------------------
  // Event Management Hardening (Issue #36)
  // ---------------------------------------------------------------------------

  const eventHardeningFixture = loadFixture('event_management_hardening');

  /** Every case id this file drives; checked against the fixture below. */
  const DRIVEN_EVENT_HARDENING_CASE_IDS = [
    'subscriber_factory_registered_type',
    'builtin_stdout_type',
    'builtin_file_type',
    'builtin_filter_passes_matching',
    'builtin_filter_discards_nonmatching',
    'circuit_open_after_threshold',
    'circuit_discards_in_open_state',
    'circuit_half_open_after_window',
    'circuit_closes_on_success',
    'event_naming_canonical',
    'filter_exclude_question_mark_is_a_wildcard',
    'filter_exclude_character_class_does_not_expand',
    'filter_include_question_mark_is_a_wildcard',
    'circuit_event_reports_the_declared_subscriber_type',
    'circuit_event_uses_the_dlq_default_for_an_undeclared_subscriber',
    'circuit_opened_event_carries_the_subscriber_id',
    'circuit_closed_event_carries_the_subscriber_id',
  ];

  function makeTestEvent(overrides: Partial<ApCoreEvent> = {}): ApCoreEvent {
    return {
      eventType: 'test.event',
      moduleId: null,
      timestamp: '2026-04-28T00:00:00Z',
      severity: 'info',
      data: {},
      ...overrides,
    };
  }

  describe('Event Management Hardening (Issue #36)', () => {
    beforeEach(() => {
      resetSubscriberRegistry();
    });

    afterEach(() => {
      resetSubscriberRegistry();
      vi.useRealTimers();
    });

    it('subscriber_factory_registered_type', () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'subscriber_factory_registered_type',
      );
      expect(tc).toBeDefined();

      // apcore#93: this registered the literal `'slack'` and asserted `instanceof
      // SlackSubscriber` — the driver's own class, against the driver's own
      // literal. Neither `expected.subscriber_created` nor
      // `expected.subscriber_type` was ever read, so the case could not go red
      // whatever the fixture declared. Both the registration and the assertions
      // are now driven off the fixture.
      class RegisteredSubscriber implements EventSubscriber {
        constructor(readonly typeName: string) {}
        async onEvent(_event: ApCoreEvent): Promise<void> {}
      }

      for (const typeName of tc.input.registered_types as string[]) {
        registerSubscriberType(typeName, () => new RegisteredSubscriber(typeName));
      }

      const subscriber = createSubscriberFromConfig(tc.input.subscriber_config);
      expect(subscriber !== null && subscriber !== undefined).toBe(
        tc.expected.subscriber_created,
      );
      // The factory selected must be the one registered under the declared type
      // name, not merely *a* subscriber.
      expect(subscriber).toBeInstanceOf(RegisteredSubscriber);
      expect((subscriber as RegisteredSubscriber).typeName).toBe(tc.expected.subscriber_type);
    });

    it('builtin_stdout_type', () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'builtin_stdout_type',
      );
      expect(tc).toBeDefined();
      expect(tc.expected.requires_registration).toBe(false);

      const subscriber = createSubscriberFromConfig(tc.input.subscriber_config);
      expect(subscriber).toBeInstanceOf(StdoutSubscriber);
    });

    it('builtin_file_type', () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'builtin_file_type',
      );
      expect(tc).toBeDefined();
      expect(tc.expected.requires_registration).toBe(false);

      const subscriber = createSubscriberFromConfig(tc.input.subscriber_config);
      expect(subscriber).toBeInstanceOf(FileSubscriber);
    });

    it('builtin_filter_passes_matching', async () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'builtin_filter_passes_matching',
      );
      expect(tc).toBeDefined();

      let deliveryAttempted = false;
      const mockDelegate: EventSubscriber = {
        async onEvent(_event: ApCoreEvent) {
          deliveryAttempted = true;
        },
      };

      registerSubscriberType('mock_passthrough', () => mockDelegate);

      const subscriber = createSubscriberFromConfig({
        type: 'filter',
        delegate_type: 'mock_passthrough',
        delegate_config: {},
        include_events: tc.input.subscriber_config.include_events,
      }) as FilterSubscriber;

      const event = makeTestEvent({
        eventType: tc.input.event.event_type,
        moduleId: tc.input.event.module_id,
        severity: tc.input.event.severity,
        data: tc.input.event.data,
      });

      await subscriber.onEvent(event);

      expect(deliveryAttempted).toBe(tc.expected.delivery_attempted);
      expect(!deliveryAttempted).toBe(tc.expected.discarded);
    });

    // PROTOCOL_SPEC 9.16.3 — the event-pattern dialect (#117). Written on
    // `exclude_events` wherever possible because exclude FAILS OPEN: a pattern
    // that does not match means the event is DELIVERED, so a matcher that
    // understands fewer metacharacters than the operator wrote opens the
    // filter rather than narrowing it.
    for (const caseId of [
      'filter_exclude_question_mark_is_a_wildcard',
      'filter_exclude_character_class_does_not_expand',
      'filter_include_question_mark_is_a_wildcard',
    ]) {
      it(caseId, async () => {
        const tc = eventHardeningFixture.test_cases.find((t: any) => t.id === caseId);
        expect(tc).toBeDefined();

        let deliveryAttempted = false;
        const mockDelegate: EventSubscriber = {
          async onEvent(_event: ApCoreEvent) {
            deliveryAttempted = true;
          },
        };
        registerSubscriberType('mock_passthrough', () => mockDelegate);

        const subscriber = createSubscriberFromConfig({
          type: 'filter',
          delegate_type: 'mock_passthrough',
          delegate_config: {},
          ...(tc.input.subscriber_config.include_events !== undefined
            ? { include_events: tc.input.subscriber_config.include_events }
            : {}),
          ...(tc.input.subscriber_config.exclude_events !== undefined
            ? { exclude_events: tc.input.subscriber_config.exclude_events }
            : {}),
        }) as FilterSubscriber;

        await subscriber.onEvent(
          makeTestEvent({
            eventType: tc.input.event.event_type,
            moduleId: tc.input.event.module_id,
            severity: tc.input.event.severity,
            data: tc.input.event.data,
          }),
        );

        expect(deliveryAttempted).toBe(tc.expected.delivery_attempted);
        expect(!deliveryAttempted).toBe(tc.expected.discarded);
      });
    }

    it('every case in the fixture is driven by this file', () => {
      // apcore-python and apcore-rust both guard this; this SDK did not, so
      // two cases added to the fixture were driven by nobody and the suite
      // stayed green. `check_driver_coverage.py` matches a quoted fixture-name
      // literal, which this file satisfies by loading the fixture at all — it
      // answers "does this SDK load it", not "does it run every case".
      const driven = new Set(DRIVEN_EVENT_HARDENING_CASE_IDS);
      const declared: string[] = eventHardeningFixture.test_cases.map((t: any) => t.id as string);
      expect(declared.filter((id: string) => !driven.has(id))).toEqual([]);
      expect([...driven].filter((id: string) => !declared.includes(id))).toEqual([]);
    });

    it('builtin_filter_discards_nonmatching', async () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'builtin_filter_discards_nonmatching',
      );
      expect(tc).toBeDefined();

      let deliveryAttempted = false;
      const mockDelegate: EventSubscriber = {
        async onEvent(_event: ApCoreEvent) {
          deliveryAttempted = true;
        },
      };

      registerSubscriberType('mock_sink', () => mockDelegate);

      const subscriber = createSubscriberFromConfig({
        type: 'filter',
        delegate_type: 'mock_sink',
        delegate_config: {},
        include_events: tc.input.subscriber_config.include_events,
      }) as FilterSubscriber;

      const event = makeTestEvent({
        eventType: tc.input.event.event_type,
        moduleId: tc.input.event.module_id,
        severity: tc.input.event.severity,
        data: tc.input.event.data,
      });

      await subscriber.onEvent(event);

      expect(deliveryAttempted).toBe(tc.expected.delivery_attempted);
      expect(!deliveryAttempted).toBe(tc.expected.discarded);
    });

    it('circuit_open_after_threshold', async () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_open_after_threshold',
      );
      expect(tc).toBeDefined();

      const emittedEventTypes: string[] = [];
      const mockEmitter = {
        emit(ev: ApCoreEvent) {
          emittedEventTypes.push(ev.eventType);
        },
      };

      const failingSub: EventSubscriber = {
        async onEvent() {
          throw new Error('simulated failure');
        },
      };

      const cb = new CircuitBreakerWrapper(failingSub, mockEmitter, {
        openThreshold: tc.input.circuit_breaker_config.open_threshold,
        recoveryWindowMs: tc.input.circuit_breaker_config.recovery_window_ms,
        timeoutMs: tc.input.circuit_breaker_config.timeout_ms,
      });

      const testEvent = makeTestEvent();
      for (const _attempt of tc.input.failure_sequence) {
        await cb.onEvent(testEvent);
      }

      expect(cb.state).toBe(tc.expected.circuit_state as CircuitState);
      expect(cb.consecutiveFailures).toBe(tc.expected.consecutive_failures);
      expect(emittedEventTypes).toContain(tc.expected.event_emitted);
    });

    // D-116: the circuit event reports the DECLARED subscriber type.
    //
    // Both cases build a subscriber whose id contains a HYPHEN and whose
    // declared kind is not a prefix of it. That is the whole discriminator:
    // this SDK reported the raw constructor name, and apcore-rust split the id
    // on the first hyphen so `health-alert` became `health`. An id without a
    // hyphen, or one whose prefix equals the declared kind, is answered
    // identically by every wrong implementation and by the right one.
    for (const caseId of [
      'circuit_event_reports_the_declared_subscriber_type',
      'circuit_event_uses_the_dlq_default_for_an_undeclared_subscriber',
      'circuit_opened_event_carries_the_subscriber_id',
    ]) {
      it(caseId, async () => {
        const tc = eventHardeningFixture.test_cases.find((t: any) => t.id === caseId);
        expect(tc).toBeDefined();

        const emitted: ApCoreEvent[] = [];
        const mockEmitter = {
          emit(ev: ApCoreEvent) {
            emitted.push(ev);
          },
        };

        const spec = tc.input.subscriber;
        const failingSub: EventSubscriber = {
          subscriberId: spec.subscriber_id,
          ...(spec.subscriber_type !== undefined ? { subscriberType: spec.subscriber_type } : {}),
          async onEvent() {
            throw new Error('simulated failure');
          },
        };

        const cb = new CircuitBreakerWrapper(failingSub, mockEmitter, {
          openThreshold: tc.input.circuit_breaker_config.open_threshold,
          recoveryWindowMs: tc.input.circuit_breaker_config.recovery_window_ms,
          timeoutMs: tc.input.circuit_breaker_config.timeout_ms,
        });

        const testEvent = makeTestEvent();
        for (const _attempt of tc.input.failure_sequence) {
          await cb.onEvent(testEvent);
        }

        expect(cb.state).toBe(tc.expected.circuit_state as CircuitState);
        const opened = emitted.filter((e) => e.eventType === tc.expected.event_emitted);
        expect(opened).toHaveLength(1);

        if (tc.expected.event_subscriber_type !== undefined) {
          expect(opened[0].data['subscriber_type']).toBe(tc.expected.event_subscriber_type);
        }
        if (tc.expected.event_subscriber_id !== undefined) {
          expect(opened[0].data['subscriber_id']).toBe(tc.expected.event_subscriber_id);
        }
        if (tc.expected.event_subscriber_type_equals_dlq_subscriber_type === true) {
          // Asserted as EQUAL TO the DLQ path's value rather than as a literal:
          // the default is a language-shaped derivation, and what is normative
          // is that the two surfaces AGREE. All three SDKs disagreed with their
          // own DLQ value, which is the finding.
          expect(opened[0].data['subscriber_type']).toBe(resolveSubscriberType(failingSub));
        }
      });
    }

    it('circuit_discards_in_open_state', async () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_discards_in_open_state',
      );
      expect(tc).toBeDefined();

      const mockEmitter = { emit(_ev: ApCoreEvent) {} };

      let deliveryAttempted = false;
      const trackingSub: EventSubscriber = {
        async onEvent() {
          deliveryAttempted = true;
          throw new Error('simulated failure');
        },
      };

      // Use openThreshold: 1 to reach OPEN quickly
      const cb = new CircuitBreakerWrapper(trackingSub, mockEmitter, { openThreshold: 1 });
      const testEvent = makeTestEvent();

      // Force OPEN
      await cb.onEvent(testEvent);
      expect(cb.state).toBe(CircuitState.OPEN);

      // Reset tracking, then attempt delivery in OPEN state
      deliveryAttempted = false;
      await cb.onEvent(makeTestEvent({ eventType: tc.input.event.event_type }));

      expect(deliveryAttempted).toBe(tc.expected.delivery_attempted);
      expect(cb.state).toBe(tc.expected.circuit_state as CircuitState);
    });

    it('circuit_half_open_after_window', async () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_half_open_after_window',
      );
      expect(tc).toBeDefined();

      vi.useFakeTimers();
      vi.setSystemTime(new Date(tc.input.last_failure_at));

      const mockEmitter = { emit(_ev: ApCoreEvent) {} };
      const failingSub: EventSubscriber = {
        async onEvent() {
          throw new Error('simulated failure');
        },
      };

      const cb = new CircuitBreakerWrapper(failingSub, mockEmitter, {
        openThreshold: tc.input.circuit_breaker_config.open_threshold,
        recoveryWindowMs: tc.input.circuit_breaker_config.recovery_window_ms,
        timeoutMs: tc.input.circuit_breaker_config.timeout_ms,
      });

      // Force OPEN by failing open_threshold times at last_failure_at
      const testEvent = makeTestEvent();
      for (let i = 0; i < tc.input.circuit_breaker_config.open_threshold; i++) {
        await cb.onEvent(testEvent);
      }
      expect(cb.state).toBe(CircuitState.OPEN);

      // Advance time past recovery window
      vi.setSystemTime(new Date(tc.input.current_time));
      cb.checkRecovery();

      expect(cb.state).toBe(tc.expected.circuit_state as CircuitState);
    });

    it('circuit_closes_on_success', async () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_closes_on_success',
      );
      expect(tc).toBeDefined();

      vi.useFakeTimers();

      const emittedEventTypes: string[] = [];
      const mockEmitter = {
        emit(ev: ApCoreEvent) {
          emittedEventTypes.push(ev.eventType);
        },
      };

      let shouldFail = true;
      const controlledSub: EventSubscriber = {
        async onEvent() {
          if (shouldFail) throw new Error('simulated failure');
        },
      };

      const cb = new CircuitBreakerWrapper(controlledSub, mockEmitter, {
        openThreshold: 1,
        recoveryWindowMs: 5000,
      });

      const testEvent = makeTestEvent();

      // Force OPEN with one failure
      await cb.onEvent(testEvent);
      expect(cb.state).toBe(CircuitState.OPEN);

      // Advance time past recovery window → HALF_OPEN
      vi.setSystemTime(Date.now() + 6000);
      cb.checkRecovery();
      expect(cb.state).toBe(CircuitState.HALF_OPEN);

      // Succeed in HALF_OPEN → CLOSED
      shouldFail = false;
      await cb.onEvent(testEvent);

      expect(cb.state).toBe(tc.expected.circuit_state as CircuitState);
      expect(cb.consecutiveFailures).toBe(tc.expected.consecutive_failures);
      expect(emittedEventTypes).toContain(tc.expected.event_emitted);
    });

    it('circuit_closed_event_carries_the_subscriber_id', async () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_closed_event_carries_the_subscriber_id',
      );
      expect(tc).toBeDefined();
      expect(tc.input.circuit_state).toBe('HALF_OPEN');
      expect(tc.input.delivery_outcome).toBe('success');

      vi.useFakeTimers();
      const emitted: ApCoreEvent[] = [];
      const mockEmitter = {
        emit(ev: ApCoreEvent) {
          emitted.push(ev);
        },
      };
      let shouldFail = true;
      const spec = tc.input.subscriber;
      const sub: EventSubscriber = {
        subscriberId: spec.subscriber_id,
        subscriberType: spec.subscriber_type,
        async onEvent() {
          if (shouldFail) throw new Error('simulated failure');
        },
      };
      const cb = new CircuitBreakerWrapper(sub, mockEmitter, {
        openThreshold: 1,
        recoveryWindowMs: 5000,
      });

      // Reach HALF_OPEN (the fixture's starting state), then succeed.
      await cb.onEvent(makeTestEvent());
      vi.setSystemTime(Date.now() + 6000);
      cb.checkRecovery();
      expect(cb.state).toBe(CircuitState.HALF_OPEN);
      shouldFail = false;
      await cb.onEvent(makeTestEvent());

      expect(cb.state).toBe(tc.expected.circuit_state as CircuitState);
      expect(cb.consecutiveFailures).toBe(tc.expected.consecutive_failures);
      const closed = emitted.filter((e) => e.eventType === tc.expected.event_emitted);
      expect(closed).toHaveLength(1);
      expect(closed[0].data['subscriber_id']).toBe(tc.expected.event_subscriber_id);
    });

    it('event_naming_canonical', () => {
      const tc = eventHardeningFixture.test_cases.find(
        (t: any) => t.id === 'event_naming_canonical',
      );
      expect(tc).toBeDefined();

      const pattern = new RegExp(tc.expected.pattern);
      for (const eventType of tc.input.events_to_check) {
        expect(pattern.test(eventType)).toBe(true);
      }
    });
  });

  // ---------------------------------------------------------------------------
  // Middleware Architecture Hardening (Issue #42)
  // ---------------------------------------------------------------------------

  const middlewareHardeningFixture = loadFixture('middleware_hardening');

  describe('Middleware Architecture Hardening (Issue #42)', () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    // Helper to build a minimal mock EventEmitter for circuit-breaker tests
    function makeCircuitEmitter() {
      const emitted: string[] = [];
      return {
        emitted,
        emit(ev: ApCoreEvent) {
          emitted.push(ev.eventType);
        },
      };
    }

    // Helper to drive a CircuitBreakerMiddleware to OPEN by filling the
    // rolling window with `count` errors then checking state.
    function driveToOpen(
      cb: CircuitBreakerMiddleware,
      moduleId: string,
      ctx: Context,
      windowSize: number,
    ): void {
      for (let i = 0; i < windowSize; i++) {
        try {
          cb.before(moduleId, {}, ctx);
        } catch {
          // circuit may already be OPEN on later iterations — ignore
        }
        cb.onError(moduleId, {}, new Error('simulated'), ctx);
      }
    }

    // --- 1. context_namespace_apcore_prefix ---
    it('context_namespace_apcore_prefix', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'context_namespace_apcore_prefix',
      );
      expect(tc).toBeDefined();

      const result = validateContextKey(tc.input.writer, tc.input.key);
      expect(result.valid).toBe(tc.expected.valid);
      expect(result.warning).toBe(tc.expected.warning);
    });

    // --- 2. context_namespace_ext_prefix ---
    it('context_namespace_ext_prefix', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'context_namespace_ext_prefix',
      );
      expect(tc).toBeDefined();

      const result = validateContextKey(tc.input.writer, tc.input.key);
      expect(result.valid).toBe(tc.expected.valid);
      expect(result.warning).toBe(tc.expected.warning);
    });

    // --- 3. context_namespace_violation ---
    it('context_namespace_violation', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'context_namespace_violation',
      );
      expect(tc).toBeDefined();

      const result = validateContextKey(tc.input.writer, tc.input.key);
      expect(result.valid).toBe(tc.expected.valid);
      expect(result.warning).toBe(tc.expected.warning);
    });

    // --- 4. circuit_breaker_opens_at_threshold ---
    it('circuit_breaker_opens_at_threshold', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_breaker_opens_at_threshold',
      );
      expect(tc).toBeDefined();

      const emitter = makeCircuitEmitter();
      const cb = new CircuitBreakerMiddleware({
        openThreshold: tc.input.open_threshold,
        windowSize: tc.input.window_size,
        recoveryWindowMs: 30000,
        emitter,
      });

      const moduleId: string = tc.input.module_id;
      const callerId: string = tc.input.caller_id;
      const ctx = new Context('trace', callerId, []);

      // Record successes_in_window successful calls
      for (let i = 0; i < tc.input.successes_in_window; i++) {
        cb.before(moduleId, {}, ctx);
        cb.after(moduleId, {}, {}, ctx);
      }

      // Record errors_in_window failed calls
      for (let i = 0; i < tc.input.errors_in_window; i++) {
        try {
          cb.before(moduleId, {}, ctx);
        } catch {
          // circuit may open during iteration — ignore CB open errors
        }
        cb.onError(moduleId, {}, new Error('fail'), ctx);
      }

      expect(cb.getState(moduleId, callerId)).toBe(tc.expected.circuit_state as CircuitBreakerState);
      expect(emitter.emitted).toContain(tc.expected.event_emitted);
    });

    // --- 5. circuit_breaker_short_circuits_open ---
    it('circuit_breaker_short_circuits_open', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_breaker_short_circuits_open',
      );
      expect(tc).toBeDefined();

      // Use a window of size 10, fill with failures to force OPEN
      const cb = new CircuitBreakerMiddleware({
        openThreshold: 0.5,
        windowSize: 10,
        recoveryWindowMs: tc.input.recovery_window_ms,
      });

      const moduleId: string = tc.input.module_id;
      const callerId: string = tc.input.caller_id;
      const ctx = new Context('trace', callerId, []);

      driveToOpen(cb, moduleId, ctx, 10);
      expect(cb.getState(moduleId, callerId)).toBe(CircuitBreakerState.OPEN);

      // A subsequent before() must throw CircuitBreakerOpenError (module never reached)
      let thrown: Error | null = null;
      let moduleReached = false;
      try {
        cb.before(moduleId, {}, ctx);
        moduleReached = true;
      } catch (e) {
        thrown = e as Error;
      }

      expect(thrown).toBeInstanceOf(CircuitBreakerOpenError);
      expect(moduleReached).toBe(tc.expected.module_reached); // false
      // CORRECTED: this read `expect(tc.expected.error).toBe('CircuitBreakerOpenError')`
      // — a fixture-to-itself comparison on a class name. apcore-rust has no such
      // class, only ErrorCode::CircuitBreakerOpen, so the wire code is the only
      // cross-language contract (the fixture now says so in
      // `error_class_name_is_not_the_contract`). Assert the code on the thrown error.
      expect((thrown as CircuitBreakerOpenError).code).toBe(tc.expected['error_code']);
    });

    // --- 6. circuit_breaker_half_open_probe ---
    it('circuit_breaker_half_open_probe', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_breaker_half_open_probe',
      );
      expect(tc).toBeDefined();

      vi.useFakeTimers();
      const baseTime = Date.now();

      const cb = new CircuitBreakerMiddleware({
        openThreshold: 0.5,
        windowSize: 10,
        recoveryWindowMs: tc.input.recovery_window_ms,
      });

      const moduleId: string = tc.input.module_id;
      const callerId: string = tc.input.caller_id;
      const ctx = new Context('trace', callerId, []);

      // Drive to OPEN (openedAt = baseTime)
      driveToOpen(cb, moduleId, ctx, 10);
      expect(cb.getState(moduleId, callerId)).toBe(CircuitBreakerState.OPEN);

      // Advance past recovery_window_ms
      vi.setSystemTime(baseTime + tc.input.ms_since_opened);

      // First before() — should transition to HALF_OPEN and allow probe
      cb.before(moduleId, {}, ctx);
      expect(ctx.data['_apcore.mw.circuit.state']).toBe(tc.expected.circuit_state); // 'HALF_OPEN'
      expect(tc.expected.probe_call_allowed).toBe(true);

      // A second concurrent before() must be blocked (max_concurrent_probes: 1)
      let secondBlocked = false;
      try {
        cb.before(moduleId, {}, ctx);
      } catch (e) {
        if (e instanceof CircuitBreakerOpenError) secondBlocked = true;
      }
      expect(secondBlocked).toBe(true);
      expect(tc.expected.max_concurrent_probes).toBe(1);
    });

    // --- 7. circuit_breaker_closes_on_success ---
    it('circuit_breaker_closes_on_success', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'circuit_breaker_closes_on_success',
      );
      expect(tc).toBeDefined();

      vi.useFakeTimers();
      const baseTime = Date.now();

      const emitter = makeCircuitEmitter();
      const cb = new CircuitBreakerMiddleware({
        openThreshold: 0.5,
        windowSize: 10,
        recoveryWindowMs: 30000,
        emitter,
      });

      const moduleId: string = tc.input.module_id;
      const callerId: string = tc.input.caller_id;
      const ctx = new Context('trace', callerId, []);

      // Drive to OPEN, advance time, allow probe
      driveToOpen(cb, moduleId, ctx, 10);
      vi.setSystemTime(baseTime + 35000); // past recovery window
      cb.before(moduleId, {}, ctx); // probe: OPEN → HALF_OPEN, probeInFlight=true

      // Successful probe → CLOSED, emit apcore.circuit.closed
      cb.after(moduleId, {}, {}, ctx);

      expect(cb.getState(moduleId, callerId)).toBe(tc.expected.circuit_state as CircuitBreakerState);
      expect(emitter.emitted).toContain(tc.expected.event_emitted);
    });

    // --- 8. tracing_noop_without_otel ---
    //
    // The `tracing_span_created` driver that used to sit here has been DELETED,
    // not skipped. It drove `src/middleware/tracing.ts` — the implementation of
    // middleware-system.md §1.3, which is now withdrawn — and asserted rules that
    // contradict the surviving contract: a span named after `module_id` (the
    // protocol spec §12 / T08-007 requires the constant `apcore.module.execute`,
    // with the module id as an attribute) and a single span id in
    // `_apcore.mw.tracing.span_id` (the surviving contract stores a *stack* in
    // `_apcore.mw.tracing.spans` with explicit `parent_span_id` links, because a
    // single slot is overwritten by the first nested module call). The fixture
    // case was removed upstream along with the section. Span creation is covered
    // by the observability suite against `observability/tracing.ts`.
    it('tracing_noop_without_otel', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'tracing_noop_without_otel',
      );
      expect(tc).toBeDefined();

      // Driver-input precondition (NOT an SDK expectation): this driver models
      // the no-OpenTelemetry environment the case names. apcore-js tracing is
      // self-contained, so that environment is simply the normal one.
      expect(tc.input.otel_available).toBe(false);

      const mw = new TracingMiddleware(new InMemoryExporter());
      const ctx = new Context(tc.input.trace_id, tc.input.caller_id, []);

      let error: unknown = null;
      try {
        mw.before(tc.input.module_id, {}, ctx);
        mw.after(tc.input.module_id, {}, {}, ctx);
      } catch (err) {
        error = err;
      }

      // Bind every expectation to an OBSERVATION. Reading
      // `expect(tc.expected.execution_continues).toBe(true)` would compare the
      // fixture to itself and could never fail on SDK behaviour.
      expect(error !== null).toBe(tc.expected.error_raised); // false
      expect(error === null).toBe(tc.expected.execution_continues); // true

      // `span_created: false` means "no *OpenTelemetry* span exists to create".
      // Observe it the only way that can actually fail: the package must not
      // pull in the OTel SDK at all, in any dependency class.
      const pkg = JSON.parse(
        fs.readFileSync(path.resolve(__dirname, '..', 'package.json'), 'utf-8'),
      );
      const otelDeps = [
        ...Object.keys(pkg.dependencies ?? {}),
        ...Object.keys(pkg.peerDependencies ?? {}),
        ...Object.keys(pkg.optionalDependencies ?? {}),
      ].filter((name: string) => name.startsWith('@opentelemetry/'));
      expect(otelDeps).toEqual([]);
      // An OTel span could only exist if the OTel SDK did.
      expect(otelDeps.length > 0).toBe(tc.expected.span_created);
    });

    // Bonus: HALF_OPEN → OPEN (failed probe re-opens circuit) — not in fixture but required by spec
    it('circuit_breaker_failed_probe_reopens', () => {
      vi.useFakeTimers();
      const baseTime = Date.now();

      const emitter = makeCircuitEmitter();
      const cb = new CircuitBreakerMiddleware({
        openThreshold: 0.5,
        windowSize: 10,
        recoveryWindowMs: 30000,
        emitter,
      });

      const moduleId = 'executor.payment.charge';
      const callerId = 'orchestrator.billing';
      const ctx = new Context('trace', callerId, []);

      // Drive to OPEN, advance past recovery window, allow probe
      driveToOpen(cb, moduleId, ctx, 10);
      vi.setSystemTime(baseTime + 35000);
      cb.before(moduleId, {}, ctx); // probe: transitions to HALF_OPEN

      // Failed probe → back to OPEN
      cb.onError(moduleId, {}, new Error('probe failure'), ctx);

      expect(cb.getState(moduleId, callerId)).toBe(CircuitBreakerState.OPEN);
      expect(emitter.emitted.filter((e) => e === 'apcore.circuit.opened').length).toBeGreaterThanOrEqual(2);
    });

    // --- 9. async_detection_coroutine_function ---
    it('async_detection_coroutine_function', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'async_detection_coroutine_function',
      );
      expect(tc).toBeDefined();
      expect(tc.expected.is_async).toBe(true);

      // TypeScript equivalent: handler.constructor.name === 'AsyncFunction'
      async function asyncFn() {
        return 'result';
      }
      function syncFn() {
        return 'result';
      }

      expect(isAsyncHandler(asyncFn)).toBe(true); // async function detected correctly
      expect(isAsyncHandler(syncFn)).toBe(false); // sync function correctly not async

      // Confirm that calling asyncFn() instanceof Promise is NOT a valid detection
      // method (per spec §1.5): it invokes the function side-effectfully
      expect(asyncFn instanceof Promise).toBe(false); // function ≠ Promise object
    });

    // --- 10-11. D-114 — `remove` clears the duplicate-identity entry ---
    //
    // This SDK is the authority for the decision and already conformed; the
    // cases exist so it cannot regress silently, which is what the coverage map
    // is for. The registry records the FIRST registration so a later duplicate
    // can be traced back to it, and a stale entry corrupts that record in both
    // directions.
    it('remove_clears_the_duplicate_identity_entry', () => {
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'remove_clears_the_duplicate_identity_entry',
      );
      const executor: any = new Executor({ registry: new Registry() });
      const mw = { name: tc.input.middleware_name, before: async () => null };

      executor.use(mw);
      // Only the steps AFTER the first `use` are observed: the first
      // registration never warns, and counting it would make both cases 1.
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      for (const step of (tc.input.sequence as string[]).slice(1)) {
        if (step === 'use') executor.use(mw);
        else if (step === 'remove') executor.remove(mw);
        else throw new Error(`unknown step ${step}`);
      }
      const duplicates = warn.mock.calls.filter((c) =>
        String(c[0]).toLowerCase().includes('duplicate'),
      ).length;
      warn.mockRestore();

      expect(duplicates).toBe(tc.expected.duplicate_warnings);
    });

    it('use_twice_still_warns_about_the_duplicate', () => {
      // The control. Without it an SDK that simply stopped warning passes, and
      // the decision is about CLEARING the entry, not dropping the warning.
      const tc = middlewareHardeningFixture.test_cases.find(
        (t: any) => t.id === 'use_twice_still_warns_about_the_duplicate',
      );
      const executor: any = new Executor({ registry: new Registry() });
      const mw = { name: tc.input.middleware_name, before: async () => null };

      executor.use(mw);
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      for (const step of (tc.input.sequence as string[]).slice(1)) {
        if (step === 'use') executor.use(mw);
        else if (step === 'remove') executor.remove(mw);
        else throw new Error(`unknown step ${step}`);
      }
      const duplicates = warn.mock.calls.filter((c) =>
        String(c[0]).toLowerCase().includes('duplicate'),
      ).length;
      warn.mockRestore();

      expect(duplicates).toBe(tc.expected.duplicate_warnings);
    });
  });

  // ---------------------------------------------------------------------------
  // Observability Hardening (Issue #43)
  // ---------------------------------------------------------------------------

  const obsHardeningFixture = loadFixture('observability_hardening');

  describe('Observability Hardening (Issue #43)', () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    // --- 1. pluggable_store_default_inmemory ---
    it('pluggable_store_default_inmemory', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'pluggable_store_default_inmemory',
      );
      expect(tc).toBeDefined();

      const history = new ErrorHistory();
      expect(history.store).toBeInstanceOf(InMemoryObservabilityStore);
      // Was `expect(tc.expected.store_type).toBe('InMemoryObservabilityStore')`
      // — the fixture compared to itself. Read the store the SDK actually built.
      expect(history.store.constructor.name).toBe(tc.expected['store_type']);
    });

    // --- 2. batch_processor_buffers_spans ---
    it('batch_processor_buffers_spans', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'batch_processor_buffers_spans',
      );
      expect(tc).toBeDefined();

      const exporter = new InMemoryExporter();
      const processor = new BatchSpanProcessor({
        exporter,
        scheduleDelayMs: tc.input.schedule_delay_ms,
      });

      const spansToSubmit: number = tc.input.spans_submitted;
      for (let i = 0; i < spansToSubmit; i++) {
        processor.onSpan(
          createSpan({ traceId: 'trace-id-01', name: `span-${i}`, startTime: Date.now() / 1000 }),
        );
      }

      expect(exporter.getSpans().length).toBe(tc.expected['spans_exported_immediately']);
      expect(processor.queueSize).toBe(tc.expected.queue_size);
      expect(processor.spansDropped).toBe(tc.expected.spans_dropped);

      void processor.shutdown();
    });

    // --- 3. batch_processor_drops_on_full_queue ---
    it('batch_processor_drops_on_full_queue', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'batch_processor_drops_on_full_queue',
      );
      expect(tc).toBeDefined();

      const exporter = new InMemoryExporter();
      const processor = new BatchSpanProcessor({
        exporter,
        maxQueueSize: tc.input.max_queue_size,
        scheduleDelayMs: 100_000,
      });

      // Pre-fill queue
      const queueSizeBefore: number = tc.input.queue_size_before;
      for (let i = 0; i < queueSizeBefore; i++) {
        processor.onSpan(
          createSpan({ traceId: 'trace-id-02', name: `span-${i}`, startTime: Date.now() / 1000 }),
        );
      }
      expect(processor.queueSize).toBe(queueSizeBefore);

      // Submit additional spans that should be dropped
      const newSpans: number = tc.input.new_spans_submitted;
      for (let i = 0; i < newSpans; i++) {
        processor.onSpan(
          createSpan({ traceId: 'trace-id-02', name: `extra-${i}`, startTime: Date.now() / 1000 }),
        );
      }

      expect(processor.queueSize).toBe(tc.expected.queue_size_after);
      expect(processor.spansDropped).toBe(tc.expected.spans_dropped);

      void processor.shutdown();
    });

    // --- 4. error_history_evicts_oldest_first ---
    it('error_history_evicts_oldest_first', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'error_history_evicts_oldest_first',
      );
      expect(tc).toBeDefined();

      vi.useFakeTimers();

      const history = new ErrorHistory({ maxTotalEntries: tc.input.max_total_entries });

      // Record existing entries with controlled timestamps
      for (const entry of tc.input.existing_entries) {
        vi.setSystemTime(new Date(entry.last_seen_at));
        history.record(entry.module_id, new ModuleError(entry.code, `Error ${entry.code}`));
      }

      // Record new entry
      const newEntry = tc.input.new_entry;
      vi.setSystemTime(new Date(newEntry.last_seen_at));
      history.record(newEntry.module_id, new ModuleError(newEntry.code, `Error ${newEntry.code}`));

      const all = history.getAll();
      const codes = all.map((e) => e.code);

      expect(codes).not.toContain(tc.expected.evicted_entry_code);
      for (const code of tc.expected.remaining_entry_codes) {
        expect(codes).toContain(code);
      }
      expect(all.length).toBe(tc.expected.total_entries);
    });

    // --- 5. error_fingerprint_dedup_same_error ---
    it('error_fingerprint_dedup_same_error', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'error_fingerprint_dedup_same_error',
      );
      expect(tc).toBeDefined();

      const history = new ErrorHistory();
      for (const record of tc.input.records) {
        history.record(record.module_id, new ModuleError(record.code, record.message));
      }

      const all = history.getAll();
      expect(all.length).toBe(tc.expected.total_entries);
      expect(all[0].count).toBe(tc.expected.entry_count);
    });

    // --- 6. error_fingerprint_normalization ---
    it('error_fingerprint_normalization', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'error_fingerprint_normalization',
      );
      expect(tc).toBeDefined();

      const messages: string[] = tc.input.messages;
      const normalized = messages.map((m: string) => normalizeMessage(m));

      for (let i = 0; i < normalized.length; i++) {
        expect(normalized[i]).toBe(tc.expected.normalized_messages[i]);
      }

      const fps = messages.map((m: string) =>
        computeFingerprint(tc.input.code, tc.input.module_id, m),
      );
      expect(fps[0] === fps[1]).toBe(tc.expected.fingerprints_equal);
    });

    // --- 7. fingerprint_different_errors_no_collision ---
    it('fingerprint_different_errors_no_collision', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'fingerprint_different_errors_no_collision',
      );
      expect(tc).toBeDefined();

      const fps = (tc.input.entries as any[]).map((e: any) =>
        computeFingerprint(e.code, e.module_id, e.message),
      );
      expect(fps[0] === fps[1]).toBe(tc.expected.fingerprints_equal);
    });

    // --- 8. redaction_field_pattern_match ---
    it('redaction_field_pattern_match', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'redaction_field_pattern_match',
      );
      expect(tc).toBeDefined();

      const cfg = tc.input.redaction_config;
      const redactionConfig = new RedactionConfig({
        fieldPatterns: cfg.field_patterns,
        valuePatterns: cfg.value_patterns.map((p: string) => new RegExp(p)),
        replacement: cfg.replacement,
      });

      const redacted = redactionConfig.apply(tc.input.log_entry.inputs);
      expect(redacted).toEqual(tc.expected.logged_inputs);

      // Verify protected fields are present (not redacted from log metadata)
      expect(tc.expected.trace_id_present).toBe(true);
      expect(tc.expected.caller_id_present).toBe(true);
      expect(tc.expected.module_id_present).toBe(true);
    });

    // --- 9. redaction_value_pattern_match ---
    it('redaction_value_pattern_match', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'redaction_value_pattern_match',
      );
      expect(tc).toBeDefined();

      const cfg = tc.input.redaction_config;
      const redactionConfig = new RedactionConfig({
        fieldPatterns: cfg.field_patterns,
        valuePatterns: cfg.value_patterns.map((p: string) => new RegExp(p)),
        replacement: cfg.replacement,
      });

      const redacted = redactionConfig.apply(tc.input.log_entry.inputs);
      expect(redacted).toEqual(tc.expected.logged_inputs);

      expect(tc.expected.trace_id_present).toBe(true);
      expect(tc.expected.caller_id_present).toBe(true);
      expect(tc.expected.module_id_present).toBe(true);
    });

    // --- 10. prometheus_format_includes_required_metrics ---
    it('prometheus_format_includes_required_metrics', () => {
      const tc = obsHardeningFixture.test_cases.find(
        (t: any) => t.id === 'prometheus_format_includes_required_metrics',
      );
      expect(tc).toBeDefined();

      const collector = new MetricsCollector();
      const state = tc.input.collector_state;

      collector.increment('apcore_module_calls_total', { module_id: 'test' }, state.apcore_module_calls_total);
      collector.increment('apcore_module_errors_total', { module_id: 'test' }, state.apcore_module_errors_total);
      for (const obs of state.apcore_module_duration_seconds_observations) {
        collector.observe('apcore_module_duration_seconds', { module_id: 'test' }, obs);
      }

      const exporter = new PrometheusExporter({ collector });
      const output = exporter.export();

      for (const metric of tc.expected.output_contains) {
        expect(output).toContain(metric);
      }
    });
  });

  // ---------------------------------------------------------------------------
  // AsyncTask Evolution (Issue #34)
  // ---------------------------------------------------------------------------

  const asyncTaskEvolutionFixture = loadFixture('async_task_evolution');

  describe('AsyncTask Evolution (Issue #34)', () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    // Helper: build a minimal executor with always-failing and always-succeeding modules
    function makeEvolutionExecutor(): Executor {
      const registry = new Registry();
      registry.register('worker.flaky_job', new FunctionModule({
        execute: () => { throw new Error('connection_error'); },
        moduleId: 'worker.flaky_job',
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        description: 'Flaky module',
      }));
      registry.register('worker.always_fails', new FunctionModule({
        execute: () => { throw new Error('persistent_error'); },
        moduleId: 'worker.always_fails',
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        description: 'Always-failing module',
      }));
      return new Executor({ registry });
    }

    // Helper: seed a store with raw fixture task records. D-17 made TaskStore
    // async; this helper awaits the saves in sequence so caller setup remains
    // deterministic.
    async function seedStore(store: TaskStore, tasks: any[]): Promise<void> {
      for (const t of tasks) {
        await store.save({
          taskId: t.task_id,
          moduleId: t.module_id,
          status: t.status as TaskStatus,
          submittedAt: t.submitted_at,
          startedAt: t.started_at,
          completedAt: t.completed_at,
          result: t.result,
          error: t.error,
          retryCount: t.retry_count ?? 0,
          maxRetries: t.max_retries ?? 0,
        });
      }
    }

    // --- 1. in_memory_store_default ---
    it('in_memory_store_default', () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'in_memory_store_default');
      expect(tc).toBeDefined();

      const executor = makeEvolutionExecutor();
      const manager = new AsyncTaskManager({ executor });
      expect(manager.store).toBeInstanceOf(InMemoryTaskStore);
      expect(tc.expected.store_type).toBe('InMemoryTaskStore');
    });

    // --- 2. custom_store_injected ---
    it('custom_store_injected', () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'custom_store_injected');
      expect(tc).toBeDefined();

      // Simulate injecting a custom store (RedisTaskStore not available; use a named subclass)
      class RedisTaskStore extends InMemoryTaskStore {}
      const store = new RedisTaskStore();
      const executor = makeEvolutionExecutor();
      const manager = new AsyncTaskManager({ executor, store });

      expect(manager.store).toBeInstanceOf(RedisTaskStore);
      expect(manager.store).toBe(store);
      expect(tc.expected.store_type).toBe('RedisTaskStore');
    });

    // --- 3. task_store_save_and_get ---
    it('task_store_save_and_get', async () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'task_store_save_and_get');
      expect(tc).toBeDefined();

      const store = new InMemoryTaskStore();
      await seedStore(store, [tc.task_info]);

      const found = await store.get(tc.lookup_id);
      expect(found).not.toBeNull();
      expect(found!.taskId).toBe(tc.expected.task_id);
      expect(found!.status).toBe(tc.expected.status);
      expect(found!.result).toEqual(tc.expected.result);
      expect(tc.expected.found).toBe(true);
    });

    // --- 4. task_store_list_by_status ---
    it('task_store_list_by_status', async () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'task_store_list_by_status');
      expect(tc).toBeDefined();

      const store = new InMemoryTaskStore();
      await seedStore(store, tc.stored_tasks);

      const results = await store.list(tc.status_filter as TaskStatus);
      expect(results.length).toBe(tc.expected.count);
      const ids = results.map(t => t.taskId);
      // D-82: insertion order is normative, so compare the list AS ORDERED.
      // This used to loop `expect(ids).toContain(expectedId)`, which asserts the
      // order away by construction — the case could not have caught a reordering
      // no matter how many tasks it seeded, and apcore-rust was the only driver
      // of the three actually enforcing the decision.
      expect(ids).toEqual(tc.expected.task_ids);
    });

    // --- 5. retry_scheduled_on_failure ---
    it('retry_scheduled_on_failure', async () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'retry_scheduled_on_failure');
      expect(tc).toBeDefined();

      vi.useFakeTimers();

      const executor = makeEvolutionExecutor();
      const store = new InMemoryTaskStore();
      const manager = new AsyncTaskManager({ executor, store });

      const rc = tc.retry_config;
      const retryConfig = new RetryConfig({
        maxRetries: rc.max_retries,
        retryDelayMs: rc.retry_delay_ms,
        backoffMultiplier: rc.backoff_multiplier,
        maxRetryDelayMs: rc.max_retry_delay_ms,
      });

      const taskId = await manager.submit('worker.flaky_job', {}, { retry: retryConfig });

      // Drain microtasks until the first execution fails and status transitions to pending/retryCount=1.
      // The setTimeout(1000) delay holds the retry, so we observe the intermediate state.
      let ticks = 0;
      while (ticks < 50) {
        await Promise.resolve();
        ticks++;
        const info = await store.get(taskId);
        if (info && info.status === TaskStatus.PENDING && info.retryCount === 1) break;
      }

      const info = (await store.get(taskId))!;
      expect(info.status).toBe(tc.expected.status_after_first_failure as TaskStatus);
      expect(info.retryCount).toBe(tc.expected.retry_count_after_first_failure);

      // Verify the backoff formula for the first retry (attempt index 0)
      expect(retryConfig.computeDelayMs(0)).toBe(tc.expected.next_retry_delay_ms);

      vi.useRealTimers();
    });

    // --- 6. backoff_multiplier_applied ---
    it('backoff_multiplier_applied', () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'backoff_multiplier_applied');
      expect(tc).toBeDefined();

      const rc = tc.retry_config;
      const retryConfig = new RetryConfig({
        retryDelayMs: rc.retry_delay_ms,
        backoffMultiplier: rc.backoff_multiplier,
        maxRetryDelayMs: rc.max_retry_delay_ms,
      });

      const exp = tc.expected;
      expect(retryConfig.computeDelayMs(0)).toBe(exp.attempt_0_delay_ms);
      expect(retryConfig.computeDelayMs(1)).toBe(exp.attempt_1_delay_ms);
      expect(retryConfig.computeDelayMs(2)).toBe(exp.attempt_2_delay_ms);
      expect(retryConfig.computeDelayMs(3)).toBe(exp.attempt_3_delay_ms);
      expect(retryConfig.computeDelayMs(4)).toBe(exp.attempt_4_delay_ms);
      expect(retryConfig.computeDelayMs(5)).toBe(exp.attempt_5_delay_ms);
    });

    // --- 7. max_retries_exhausted_becomes_failed ---
    it('max_retries_exhausted_becomes_failed', async () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'max_retries_exhausted_becomes_failed');
      expect(tc).toBeDefined();

      const executor = makeEvolutionExecutor();
      const store = new InMemoryTaskStore();
      const manager = new AsyncTaskManager({ executor, store });

      // Use 0ms delay so the test is fast (the fixture's 100ms delay would be slow)
      const retryConfig = new RetryConfig({
        maxRetries: tc.retry_config.max_retries,
        retryDelayMs: 0,
        backoffMultiplier: tc.retry_config.backoff_multiplier,
        maxRetryDelayMs: tc.retry_config.max_retry_delay_ms,
      });

      const taskId = await manager.submit('worker.always_fails', {}, { retry: retryConfig });

      // Wait for all retries to exhaust (max_retries=2, 0ms delay each).
      // setTimeout(0) is a macrotask so we must yield via real timers.
      const deadline = Date.now() + 2000;
      while (Date.now() < deadline) {
        await new Promise<void>((resolve) => setTimeout(resolve, 10));
        if ((await store.get(taskId))?.status === TaskStatus.FAILED) break;
      }

      const info = (await store.get(taskId))!;
      expect(info.status).toBe(tc.expected.final_status as TaskStatus);
      expect(info.retryCount).toBe(tc.expected.retry_count);
      expect(info.error).not.toBeNull();
      expect(tc.expected.error_populated).toBe(true);
    });

    // --- 8. reaper_disabled_by_default ---
    it('reaper_disabled_by_default', async () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'reaper_disabled_by_default');
      expect(tc).toBeDefined();

      const store = new InMemoryTaskStore();
      await seedStore(store, tc.stored_expired_tasks);

      const executor = makeEvolutionExecutor();
      // Construct without any reaper config — reaper is NOT started
      const manager = new AsyncTaskManager({ executor, store });

      // Without calling startReaper(), the expired task must still be in the store
      const found = await store.get(tc.stored_expired_tasks[0].task_id);
      expect(found).not.toBeNull();
      expect(tc.expected.reaper_running).toBe(false);
      expect(tc.expected.expired_task_still_present).toBe(true);
    });

    // --- 9. reaper_deletes_expired_tasks ---
    it('reaper_deletes_expired_tasks', async () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'reaper_deletes_expired_tasks');
      expect(tc).toBeDefined();

      vi.useFakeTimers();

      const store = new InMemoryTaskStore();
      await seedStore(store, tc.stored_tasks);

      const executor = makeEvolutionExecutor();
      const manager = new AsyncTaskManager({ executor, store });

      const reaperConfig = tc.config.reaper;
      // Drive the fixture's OWN now_timestamp. It is 1700002000, which puts the
      // cutoff at 1700002000 - 3600 = 1699998400 — after expired-task-001
      // (completed_at=1699990002) and before fresh-task-001 (1699999002), so the
      // declared deleted/remaining split is reachable. This driver used to
      // substitute a locally chosen 1700000000 because the fixture then declared
      // 1700003000, which expired both tasks; substituting a working timestamp
      // hid the broken fixture instead of failing on it. apcore#81 corrected the
      // fixture, so the substitution must go — otherwise this case silently stops
      // testing the value the fixture publishes.
      const stableNow = tc.now_timestamp as number;
      vi.setSystemTime(stableNow * 1000);

      const handle = await manager.startReaper({
        ttlSeconds: reaperConfig.ttl_seconds,
        sweepIntervalMs: reaperConfig.sweep_interval_ms,
      });

      // Advance time to trigger the first sweep (past sweep_interval_ms=300000ms)
      vi.advanceTimersByTime(reaperConfig.sweep_interval_ms + 1);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();

      for (const deletedId of tc.expected.deleted_task_ids) {
        expect(await store.get(deletedId)).toBeNull();
      }
      for (const remainingId of tc.expected.remaining_task_ids) {
        expect(await store.get(remainingId)).not.toBeNull();
      }

      await handle.stop();
      vi.useRealTimers();
    });

    // --- 10. reaper_skips_running_tasks ---
    it('reaper_skips_running_tasks', async () => {
      const tc = asyncTaskEvolutionFixture.test_cases.find((t: any) => t.id === 'reaper_skips_running_tasks');
      expect(tc).toBeDefined();

      vi.useFakeTimers();

      const store = new InMemoryTaskStore();
      await seedStore(store, tc.stored_tasks);

      const executor = makeEvolutionExecutor();
      const manager = new AsyncTaskManager({ executor, store });

      const reaperConfig = tc.config.reaper;
      vi.setSystemTime(tc.now_timestamp * 1000);

      const handle = await manager.startReaper({
        ttlSeconds: reaperConfig.ttl_seconds,
        sweepIntervalMs: reaperConfig.sweep_interval_ms,
      });

      vi.advanceTimersByTime(reaperConfig.sweep_interval_ms + 1);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();

      // Reaper must not have deleted any tasks
      expect(tc.expected.deleted_task_ids).toHaveLength(0);
      for (const remainingId of tc.expected.remaining_task_ids) {
        expect(await store.get(remainingId)).not.toBeNull();
      }

      await handle.stop();
      vi.useRealTimers();
    });
  });

  // ---------------------------------------------------------------------------
  // System Modules Hardening (Issue #45)
  // ---------------------------------------------------------------------------

  const sysHardeningFixture = loadFixture('system_modules_hardening');

  /**
   * Canonical case id -> the `it()` in this describe that asserts it.
   *
   * Each case needs its own registry / config / filesystem wiring, so the
   * assertions are hand-written rather than generated by iterating the fixture.
   * That used to mean a case added on the spec side left no trace here at all:
   * `find(id === ...)` only sees the ids it is asked for. The guard below turns
   * a spec-side addition into a failing test instead of a silent gap.
   */
  const SYS_HARDENING_COVERED_CASE_IDS = [
    'overrides_persisted_on_update',
    'overrides_loaded_on_startup',
    'audit_entry_records_actor',
    'audit_entry_records_change',
    'prometheus_usage_exports_calls_total',
    'reload_with_path_filter',
    'reload_module_id_and_filter_conflict',
    'startup_fail_on_error_true_raises',
    'startup_fail_on_error_false_continues',
    'rust_register_returns_result',
    'reload_order_is_topological_not_alphabetical',
    'manifest_full_project_name_defaults_to_apcore',
    'health_summary_project_name_agrees_with_manifest_full',
    'error_rate_threshold_moves_only_the_healthy_boundary',
    'the_error_boundary_stays_at_the_table_value',
    'system_modules_declare_open_world_false',
  ];

  describe('System Modules Hardening (Issue #45)', () => {
    let tmpOverridesPath: string;

    beforeEach(() => {
      tmpOverridesPath = path.join(os.tmpdir(), `apcore_test_overrides_${Date.now()}_${Math.random().toString(36).slice(2)}.yaml`);
    });

    afterEach(() => {
      if (fs.existsSync(tmpOverridesPath)) {
        fs.unlinkSync(tmpOverridesPath);
      }
    });

    // --- 1. overrides_persisted_on_update ---
    it('overrides_persisted_on_update', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'overrides_persisted_on_update');
      expect(tc).toBeDefined();

      const expected = tc['expected'];
      const config = new Config({});
      const emitter = new EventEmitter();
      const updateMod = new UpdateConfigModule(config, emitter, { overridesPath: tmpOverridesPath });

      const result = updateMod.execute(tc.action.input, null);
      expect(result['success']).toBe(expected['call_success']);

      expect(fs.existsSync(tmpOverridesPath)).toBe(expected['overrides_file_written']);
      const fileContent = fs.readFileSync(tmpOverridesPath, 'utf-8');
      const parsed = yaml.load(fileContent) as Record<string, unknown>;
      for (const [key, value] of Object.entries(expected['overrides_file_contains'])) {
        expect(parsed[key]).toBe(value);
      }
    });

    // --- 2. overrides_loaded_on_startup ---
    it('overrides_loaded_on_startup', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'overrides_loaded_on_startup');
      expect(tc).toBeDefined();

      const expected = tc['expected'];
      // Write overrides file with override values
      fs.writeFileSync(tmpOverridesPath, yaml.dump(tc.setup.overrides_file_content), 'utf-8');

      // The base config is a real file, not an in-memory dict, so
      // `base_not_modified` has something to observe: applying overrides at
      // startup must layer in memory and never write back to the config source.
      const baseConfigPath = `${tmpOverridesPath}.base.yaml`;
      const baseDoc: Record<string, unknown> = { sys_modules: { enabled: true } };
      for (const [key, value] of Object.entries(tc.setup.base_config as Record<string, unknown>)) {
        // Fixture keys are dot-paths; the file form is nested.
        const parts = key.split('.');
        let cursor = baseDoc;
        for (const part of parts.slice(0, -1)) {
          if (typeof cursor[part] !== 'object' || cursor[part] === null) cursor[part] = {};
          cursor = cursor[part] as Record<string, unknown>;
        }
        cursor[parts[parts.length - 1]] = value;
      }
      fs.writeFileSync(baseConfigPath, yaml.dump(baseDoc), 'utf-8');
      const baseBytesBefore = fs.readFileSync(baseConfigPath, 'utf-8');

      try {
        // validate:false keeps the case on the override-layering contract
        // rather than on canonical-schema coverage of the two keys used here.
        const config = Config.load(baseConfigPath, { validate: false });
        const resolved = expected['resolved_value'];
        // Pre-condition: before overrides land, the base file's value is what resolves.
        expect(config.get(resolved['key'])).toBe(
          (tc.setup.base_config as Record<string, unknown>)[resolved['key']],
        );

        const registry = new Registry();
        const executor = new Executor({ registry });
        registerSysModules(registry, executor, config, null, { overridesPath: tmpOverridesPath });

        expect(config.get(resolved['key'])).toBe(resolved['value']);

        if (expected['base_not_modified']) {
          expect(fs.readFileSync(baseConfigPath, 'utf-8')).toBe(baseBytesBefore);
          const reparsed = yaml.load(baseBytesBefore) as Record<string, any>;
          expect(reparsed['executor']['default_timeout']).toBe(
            (tc.setup.base_config as Record<string, unknown>)['executor.default_timeout'],
          );
        }
      } finally {
        if (fs.existsSync(baseConfigPath)) fs.unlinkSync(baseConfigPath);
      }
    });

    // --- 3. audit_entry_records_actor ---
    it('audit_entry_records_actor', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'audit_entry_records_actor');
      expect(tc).toBeDefined();

      const config = new Config({});
      const emitter = new EventEmitter();
      const auditStore = new InMemoryAuditStore();
      const updateMod = new UpdateConfigModule(config, emitter, { auditStore });

      const identity = createIdentity(tc.action.context_identity.id, tc.action.context_identity.type, []);
      const ctx = new Context('trace-abc', 'test-caller', [], null, identity);

      updateMod.execute(tc.action.input, ctx);

      const expected = tc['expected'];
      const entries = auditStore.query();
      expect(entries.length).toBe(expected['audit_entries_count']);

      const entry = entries[0];
      const expectedEntry = expected['audit_entry'];
      expect(entry.action).toBe(expectedEntry['action']);
      expect(entry.targetModuleId).toBe(expectedEntry['target_module_id']);
      expect(entry.actorId).toBe(expectedEntry['actor_id']);
      expect(entry.actorType).toBe(expectedEntry['actor_type']);
      // Was `expect(tc.expected.timestamp_present).toBe(true)` — the fixture
      // asserting itself. The entry the SDK wrote is the observation.
      expect(entry.timestamp != null && entry.timestamp !== '').toBe(
        expected['timestamp_present'],
      );
      expect(entry.traceId != null && entry.traceId !== '').toBe(expected['trace_id_present']);
    });

    // --- 4. audit_entry_records_change ---
    it('audit_entry_records_change', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'audit_entry_records_change');
      expect(tc).toBeDefined();

      const registry = new Registry();
      const emitter = new EventEmitter();
      const auditStore = new InMemoryAuditStore();

      // Register the module that will be toggled
      registry.registerInternal(tc.setup.initial_module_state.module_id, {
        description: 'test risky module',
        execute: () => ({}),
      });

      const toggleMod = new ToggleFeatureModule(registry, emitter, undefined, auditStore);

      const identity = createIdentity(tc.action.context_identity.id, tc.action.context_identity.type, []);
      const ctx = new Context('trace-xyz', 'test-caller', [], null, identity);

      toggleMod.execute(tc.action.input, ctx);

      const expected = tc['expected'];
      const entries = auditStore.query();
      expect(entries.length).toBe(expected['audit_entries_count']);

      const entry = entries[0];
      const expectedEntry = expected['audit_entry'];
      expect(entry.action).toBe(expectedEntry['action']);
      expect(entry.targetModuleId).toBe(expectedEntry['target_module_id']);
      expect(entry.actorId).toBe(expectedEntry['actor_id']);
      expect(entry.actorType).toBe(expectedEntry['actor_type']);
      expect(entry.change.before).toBe(expectedEntry['change']['before']);
      expect(entry.change.after).toBe(expectedEntry['change']['after']);
    });

    // --- 5. prometheus_usage_exports_calls_total ---
    it('prometheus_usage_exports_calls_total', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'prometheus_usage_exports_calls_total');
      expect(tc).toBeDefined();

      const usageCollector = new UsageCollector();
      const moduleData = tc.setup.usage_collector_data[0];
      const moduleId = moduleData.module_id;

      // Record success and error calls
      const successCount: number = moduleData.status_breakdown.success;
      const errorCount: number = moduleData.status_breakdown.error;
      for (let i = 0; i < successCount; i++) {
        usageCollector.record(moduleId, 'caller.test', 10 + i % 50, true);
      }
      for (let i = 0; i < errorCount; i++) {
        usageCollector.record(moduleId, 'caller.test', 5, false);
      }

      const collector = new MetricsCollector();
      const exporter = new PrometheusExporter({ collector, usageCollector });

      const start = Date.now();
      const output = exporter.export();
      const elapsed = Date.now() - start;

      expect(elapsed).toBeLessThan(tc['expected']['export_within_timeout_ms']);

      for (const expectedLine of tc['expected']['metrics_endpoint_contains'] as string[]) {
        expect(output).toContain(expectedLine);
      }
    });

    // --- 6. reload_with_path_filter ---
    it('reload_with_path_filter', async () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'reload_with_path_filter');
      expect(tc).toBeDefined();

      const registry = new Registry();
      const emitter = new EventEmitter();
      const dummyModule = { description: 'dummy', version: '1.0.0', execute: () => ({}) };

      const moduleStore: Record<string, unknown> = {};
      for (const moduleId of tc.setup.registered_modules as string[]) {
        registry.registerInternal(moduleId, { ...dummyModule });
        moduleStore[moduleId] = { ...dummyModule };
      }

      // Mock discover to re-register all modules in the store
      vi.spyOn(registry, 'discover').mockImplementation(async () => {
        let count = 0;
        for (const [id, mod] of Object.entries(moduleStore)) {
          if (!registry.has(id)) {
            registry.registerInternal(id, mod);
            count++;
          }
        }
        return count;
      });

      const reloadMod = new ReloadModule(registry, emitter);
      const result = await reloadMod.execute(tc.action.input, null) as Record<string, unknown>;

      const expected = tc['expected'];
      expect(result['success']).toBe(expected['call_success']);
      const reloadedModules = result['reloaded_modules'] as string[];
      for (const expectedId of expected['reloaded_modules'] as string[]) {
        expect(reloadedModules).toContain(expectedId);
      }
      for (const notReloadedId of expected['not_reloaded'] as string[]) {
        expect(reloadedModules).not.toContain(notReloadedId);
        // Not reloaded is not the same as unregistered — the module outside the
        // path filter must still be there afterwards.
        expect(registry.has(notReloadedId)).toBe(true);
      }
      // `reload_order: "topological"` (issue #35). Corrected: this used to say
      // the SDK had no topological path at all, which was true —
      // `_reloadWithPathFilter` called `.sort()` and never consulted the
      // dependency graph. It now runs Kahn's sort over the declared
      // dependencies, so the contract is implemented.
      //
      // What this fixture case can still pin is only the dependency-free half
      // of it: its three modules declare no dependencies on each other, so
      // every permutation is a valid linearization and the sole observable
      // constraint is determinism — Kahn's sort seeds from a sorted
      // zero-in-degree queue, so it emits alphabetical order here. The
      // discriminating case (a dependency whose direction disagrees with the
      // alphabet) is now canonical too —
      // `reload_order_is_topological_not_alphabetical`, driven below.
      expect(reloadedModules).toEqual([...(expected['reloaded_modules'] as string[])].sort());
    });

    // --- 7. reload_module_id_and_filter_conflict ---
    it('reload_module_id_and_filter_conflict', async () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'reload_module_id_and_filter_conflict');
      expect(tc).toBeDefined();

      const registry = new Registry();
      const emitter = new EventEmitter();
      const reloadMod = new ReloadModule(registry, emitter);
      const expected = tc['expected'];

      let succeeded = false;
      let thrown: any = null;
      try {
        await reloadMod.execute(tc.action.input, null);
        succeeded = true;
      } catch (err) {
        thrown = err;
      }
      expect(succeeded).toBe(expected['call_success']);
      expect(thrown).toBeInstanceOf(ModuleReloadConflictError);
      expect(thrown.code).toBe(expected['error_code']);
      expect(thrown.message).toContain(expected['error_message_contains']);
    });

    // --- 8. startup_fail_on_error_true_raises ---
    it('startup_fail_on_error_true_raises', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'startup_fail_on_error_true_raises');
      expect(tc).toBeDefined();

      const targetModuleId: string = tc.setup.simulated_failure.module_id;
      const failingRegistry = new (class extends Registry {
        override registerInternal(moduleId: string, module: unknown): void {
          if (moduleId === targetModuleId) {
            throw new Error(tc.setup.simulated_failure.error);
          }
          super.registerInternal(moduleId, module);
        }
      })();

      const config = new Config({ sys_modules: { enabled: true, events: { enabled: true } } });
      const executor = new Executor({ registry: failingRegistry });

      const expected = tc['expected'];
      let raised = false;
      let thrown: any = null;
      try {
        registerSysModules(failingRegistry, executor, config, null, { failOnError: true });
      } catch (err) {
        raised = true;
        thrown = err;
      }
      expect(raised).toBe(expected['raises']);
      expect(thrown).toBeInstanceOf(SysModuleRegistrationError);
      expect(thrown.code).toBe(expected['error_code']);
      // The operator has to be able to tell WHICH system module failed.
      expect(thrown.message).toContain(expected['error_includes_module_id']);
    });

    // --- 9. startup_fail_on_error_false_continues ---
    it('startup_fail_on_error_false_continues', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'startup_fail_on_error_false_continues');
      expect(tc).toBeDefined();

      const targetModuleId: string = tc.setup.simulated_failure.module_id;
      const failingRegistry = new (class extends Registry {
        override registerInternal(moduleId: string, module: unknown): void {
          if (moduleId === targetModuleId) {
            throw new Error(tc.setup.simulated_failure.error);
          }
          super.registerInternal(moduleId, module);
        }
      })();

      const config = new Config({ sys_modules: { enabled: true, events: { enabled: true } } });
      const executor = new Executor({ registry: failingRegistry });

      const expected = tc['expected'];
      // The fixture names a log LEVEL, so the assertion has to be able to tell
      // levels apart: the failure must reach the level it names and no other.
      // Downgrading the failure to a warning is exactly the regression this
      // catches, and it is invisible to a "did not throw" check.
      const consoleForLevel: Record<string, 'error' | 'warn' | 'log'> = {
        ERROR: 'error',
        WARN: 'warn',
        INFO: 'log',
      };
      const spies = {
        error: vi.spyOn(console, 'error').mockImplementation(() => undefined),
        warn: vi.spyOn(console, 'warn').mockImplementation(() => undefined),
        log: vi.spyOn(console, 'log').mockImplementation(() => undefined),
      };
      let raised = false;
      try {
        try {
          registerSysModules(failingRegistry, executor, config, null, { failOnError: false });
        } catch {
          raised = true;
        }
        const namingFailure = (level: 'error' | 'warn' | 'log'): string[] =>
          spies[level].mock.calls
            .map((c) => c.map((a) => String(a)).join(' '))
            .filter((m) => m.includes(targetModuleId));

        expect(raised).toBe(expected['raises']);
        const declared = consoleForLevel[expected['log_level_on_failure'] as string];
        expect(declared).toBeDefined();
        expect(namingFailure(declared).length).toBeGreaterThan(0);
        for (const level of ['error', 'warn', 'log'] as const) {
          if (level !== declared) expect(namingFailure(level)).toEqual([]);
        }
      } finally {
        for (const spy of Object.values(spies)) spy.mockRestore();
      }

      // Remaining modules (other than the failed one) are registered. This used
      // to be `expect(tc.expected.remaining_modules_registered).toBe(true)` — a
      // fixture-to-itself comparison — beside an ungated registry check.
      if (expected['remaining_modules_registered']) {
        expect(failingRegistry.has('system.health.module')).toBe(true);
        expect(failingRegistry.has(targetModuleId)).toBe(false);
      }
    });

    // --- 10. rust_register_returns_result ---
    // Out of scope for this SDK: the case is tagged language=rust and its
    // expectations describe the shape of Rust's return type
    // (Result / Ok / Err variants, no Option, no panic). TypeScript signals
    // failure by throwing, so there is no return value to inspect and no
    // honest TypeScript equivalent to invent — those keys belong to the Rust
    // driver. The language tag is asserted so that removing it (i.e. making the
    // case cross-language) surfaces here instead of silently staying unread.
    it('rust_register_returns_result', () => {
      const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === 'rust_register_returns_result');
      expect(tc).toBeDefined();
      expect(tc.language).toBe('rust');
    });

    // --- 11. reload_order_is_topological_not_alphabetical ---
    //
    // The discriminating half of `reload_order: "topological"`. Case 6 above
    // cannot tell the two apart — its three modules declare no dependencies on
    // each other, so every permutation is a valid topological order. Here
    // `executor.alpha` depends on `executor.zulu`, so `.sort()` and Kahn's sort
    // cannot both pass. Everything the case needs (ids, edge, expected order)
    // comes out of the fixture.
    it('reload_order_is_topological_not_alphabetical', async () => {
      const tc = sysHardeningFixture.test_cases.find(
        (t: any) => t.id === 'reload_order_is_topological_not_alphabetical',
      );
      expect(tc).toBeDefined();

      const expected = tc['expected'];
      const registeredModules = tc.setup.registered_modules as string[];
      const declared = (tc.setup.declared_dependencies ?? {}) as Record<string, string[]>;

      const registry = new Registry();
      const emitter = new EventEmitter();

      // `dependencies_must_survive_registration`, first half: the edge is
      // declared through `register(moduleId, module, version, metadata)` — the
      // path an application uses — not handed to the sort directly.
      const modules: Record<string, Record<string, unknown>> = {};
      const metadata: Record<string, Record<string, unknown> | null> = {};
      for (const moduleId of registeredModules) {
        modules[moduleId] = { description: 'dummy', version: '1.0.0', execute: () => ({}) };
        const deps = declared[moduleId];
        metadata[moduleId] = deps ? { dependencies: deps.map((d) => ({ module_id: d })) } : null;
        await registry.register(moduleId, modules[moduleId], null, metadata[moduleId]);
      }

      // ...second half: read it back through the post-registration accessor,
      // which is what `_topoSortModules` consults. `mergeModuleMetadata` used
      // to drop `dependencies` here while discovery-time ordering kept working
      // (apcore-typescript#35), so this is the assertion that separates a merge
      // regression from a sorting regression.
      for (const [moduleId, deps] of Object.entries(declared)) {
        const stored = registry.getModuleMetadata(moduleId)['dependencies'];
        expect(Array.isArray(stored)).toBe(true);
        expect(
          parseDependencies(stored as Array<Record<string, unknown>>).map((d) => d.moduleId),
        ).toEqual(deps);
      }

      // Re-discovery of an unchanged source tree re-registers what was
      // unregistered — with its metadata, as a real discovery pass would.
      vi.spyOn(registry, 'discover').mockImplementation(async () => {
        let n = 0;
        for (const [id, module] of Object.entries(modules)) {
          if (!registry.has(id)) {
            await registry.register(id, module, null, metadata[id] ?? null);
            n += 1;
          }
        }
        return n;
      });

      // `ordering_needs_a_disagreeing_graph`: the observation is the sequence
      // the SDK actually reloaded in. `result.reloaded_modules` is built by
      // walking the same id list a second time, so asserting it would only
      // check that the report matches itself.
      const safeUnregisterSpy = vi.spyOn(registry, 'safeUnregister');

      const reloadMod = new ReloadModule(registry, emitter);
      const result = (await reloadMod.execute(tc.action.input, null)) as Record<string, unknown>;

      expect(result['success']).toBe(expected['call_success']);

      const observed = expected['reload_order_observed'] as string[];
      const unregisterOrder = safeUnregisterSpy.mock.calls.map((c) => c[0]);
      expect(unregisterOrder).toEqual(observed);

      // `alphabetical_order_would_be` / `orders_differ`: pin the fixture's claim
      // against a real sort, so editing the ids into agreement fails here
      // instead of quietly disarming the assertion above.
      const alphabetical = expected['alphabetical_order_would_be'] as string[];
      expect([...registeredModules].sort()).toEqual(alphabetical);
      expect(JSON.stringify(observed) !== JSON.stringify(alphabetical)).toBe(
        expected['orders_differ'],
      );
      if (expected['orders_differ']) {
        expect(unregisterOrder).not.toEqual(alphabetical);
      }
    });

    // --- 12-14. D-110 / D-119 ---
    //
    // Both are v1.51.0 policy decisions: the spec was silent and each SDK had
    // answered reasonably, so these are written from the fixture rather than
    // from what any implementation happens to do.
    it('manifest_full_project_name_defaults_to_apcore', async () => {
      const tc = sysHardeningFixture.test_cases.find(
        (t: any) => t.id === 'manifest_full_project_name_defaults_to_apcore',
      );
      const mod = new ManifestFullModule(new Registry(), new Config({}));
      const out = (await (mod as any).execute({}, null)) as Record<string, unknown>;
      expect(out['project_name']).toBe(tc.expected.project_name);
    });

    // D-109: `error_rate_threshold` moves the HEALTHY/DEGRADED boundary only.
    // The degraded/error boundary is the classification table's fixed 0.10.
    //
    // This SDK computed it as `healthyThreshold * 10`, so with a threshold of
    // 0.001 a module erroring 5% of the time was `error` here and `degraded` on
    // apcore-rust — same module, same metrics, same configuration.
    for (const caseId of [
      'error_rate_threshold_moves_only_the_healthy_boundary',
      'the_error_boundary_stays_at_the_table_value',
    ]) {
      it(caseId, async () => {
        const tc = sysHardeningFixture.test_cases.find((t: any) => t.id === caseId);
        expect(tc).toBeDefined();

        const observed = tc.action.observed;
        const moduleId = observed.module_id as string;

        const registry = new Registry();
        registry.registerInternal(moduleId, {
          inputSchema: { type: 'object' as const },
          outputSchema: { type: 'object' as const },
          description: 'health probe',
          execute: async () => ({}),
        });

        const metrics = new MetricsCollector();
        for (let i = 0; i < observed.total_calls; i++) {
          metrics.incrementCalls(moduleId, i < observed.error_calls ? 'error' : 'success');
        }

        const summary = (await (
          new HealthSummaryModule(registry, metrics, new ErrorHistory(), new Config({})) as any
        ).execute({ ...tc.action.input }, null)) as any;

        const entry = (summary.modules as any[]).find((m) => m.module_id === moduleId);
        expect(entry, `${moduleId} missing from the summary`).toBeDefined();
        expect(entry.status).toBe(tc.expected.module_status);
      });
    }

    it('health_summary_project_name_agrees_with_manifest_full', async () => {
      // The pairing IS the decision: `manifest.full` alone could be satisfied
      // while the two system modules still disagreed, which is the state D-110
      // exists to remove.
      const tc = sysHardeningFixture.test_cases.find(
        (t: any) => t.id === 'health_summary_project_name_agrees_with_manifest_full',
      );
      const registry = new Registry();
      const config = new Config({});
      const summary = (await (
        new HealthSummaryModule(registry, null, new ErrorHistory(), config) as any
      ).execute({}, null)) as any;
      const manifest = (await (new ManifestFullModule(registry, config) as any).execute(
        {},
        null,
      )) as any;
      expect(summary.project.name).toBe(tc.expected.project_name);
      expect(summary.project.name).toBe(manifest.project_name);
    });

    it('system_modules_declare_open_world_false', async () => {
      // Reads the DECLARED annotation, not an effective value computed with
      // defaults applied: the language default means the opposite of the
      // intended value, and inheriting it is how the divergence arose.
      const tc = sysHardeningFixture.test_cases.find(
        (t: any) => t.id === 'system_modules_declare_open_world_false',
      );
      const client = new APCore({
        config: new Config({
          version: '1.0',
          project: { name: 'p' },
          sys_modules: { enabled: true, control: { enabled: true } },
        }),
      } as any);
      const registry = (client as any).registry ?? (client as any)._registry;
      const systemIds = (registry.moduleIds as string[]).filter((m) => m.startsWith('system.'));
      expect(systemIds.length).toBeGreaterThanOrEqual(tc.expected.at_least);
      // getDefinition().annotations, not a class attribute: that is the surface
      // system.manifest.* publishes, so it is what a consumer sees, and it is
      // the one shape all three SDKs share.
      const wrong = systemIds.filter(
        (m) => (registry.getDefinition(m) as any)?.annotations?.openWorld !== tc.expected.every_value,
      );
      expect(wrong).toEqual([]);
    });

    // --- fixture case inventory ---
    it('every canonical case in system_modules_hardening.json has a driver here', () => {
      const canonical = sysHardeningFixture.test_cases.map((t: any) => t.id as string);
      expect([...canonical].sort()).toEqual([...SYS_HARDENING_COVERED_CASE_IDS].sort());
    });
  });

  // ---------------------------------------------------------------------------
  // sensitive_keys_default — D-54 canonical RedactionConfig defaults
  // ---------------------------------------------------------------------------
  const sensitiveKeysFixture = loadFixture('sensitive_keys_default');
  describe('Sensitive Keys Default (D-54)', () => {
    function buildConfig(tc: any): RedactionConfig {
      if (tc.construction === 'override') {
        return new RedactionConfig({
          fieldPatterns: [...(tc.override_sensitive_keys as string[])],
        });
      }
      // default: use the canonical default list directly
      return new RedactionConfig({
        fieldPatterns: [...DEFAULT_REDACTION_FIELD_PATTERNS],
      });
    }

    sensitiveKeysFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        if (tc.id === 'default_list_is_canonical_16_entries') {
          // Assert the canonical default list (length + order) matches the
          // fixture exactly.
          expect([...DEFAULT_REDACTION_FIELD_PATTERNS]).toEqual(tc.expected.sensitive_keys);
          expect(DEFAULT_REDACTION_FIELD_PATTERNS.length).toBe(tc.expected.length);
          return;
        }
        const rc = buildConfig(tc);
        const result = rc.apply(tc.input as Record<string, unknown>);
        for (const [key, expectedVal] of Object.entries(tc.expected as Record<string, unknown>)) {
          expect(result[key]).toEqual(expectedVal);
        }
      });
    });
  });

  // ---------------------------------------------------------------------------
  // error_fingerprinting — fingerprint dedup with normalization
  // ---------------------------------------------------------------------------
  const errorFingerprintFixture = loadFixture('error_fingerprinting');
  describe('Error Fingerprinting', () => {
    errorFingerprintFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        const history = new ErrorHistory();
        const fingerprints = new Set<string>();

        for (const errSpec of tc.errors as any[]) {
          // Fingerprint composition uses error_code + a call-site signature +
          // normalized message. The fixture uses caller_id (and optional
          // top_frame) as the call-site signature; we feed both into the
          // moduleId slot of the existing TS fingerprint so that distinct
          // top_frames yield distinct fingerprints. (TS error_history.ts
          // uses module_id as the second component; the fixture description
          // says callers SHOULD substitute caller_id when stack traces are
          // unavailable.)
          const callSite = errSpec.top_frame
            ? `${errSpec.caller_id}|${errSpec.top_frame}`
            : errSpec.caller_id;
          const fp = computeFingerprint(errSpec.error_code, callSite, errSpec.message);
          fingerprints.add(fp);
          // Use ModuleError so ErrorHistory.record dedups via its own fingerprint
          // computation. We pass the same callSite as the moduleId so the dedup
          // key is consistent across calls.
          const err = new ModuleError(errSpec.error_code, errSpec.message);
          history.record(callSite, err);
        }

        const all = history.getAll();
        if (tc.expected.entry_count !== undefined) {
          expect(all.length).toBe(tc.expected.entry_count);
        }
        if (tc.expected.fingerprints_distinct !== undefined) {
          expect(fingerprints.size).toBe(tc.expected.fingerprints_distinct);
        }
        if (tc.expected.first_entry_count !== undefined) {
          // For dedup cases, the surviving entry's count MUST equal the
          // number of recorded errors.
          expect(all[0].count).toBe(tc.expected.first_entry_count);
        }
        if (tc.expected.timestamp_pattern !== undefined) {
          // D-120. Asserted on the record the PRODUCER writes, not on a health
          // summary: a reader that reformats on the way out leaves the stored
          // record and every other consumer on the old form, which is how one
          // SDK ended up with two forms inside itself.
          const pattern = new RegExp(tc.expected.timestamp_pattern);
          for (const entry of all) {
            for (const field of tc.expected.timestamp_fields as string[]) {
              const camel = field.replace(/_([a-z])/g, (_m, c: string) => c.toUpperCase());
              const value = (entry as unknown as Record<string, unknown>)[camel];
              expect(typeof value, `${field} must be a string`).toBe('string');
              expect(value as string, `${field} must match ${tc.expected.timestamp_pattern}`).toMatch(
                pattern,
              );
            }
          }
        }
      });
    });
  });

  // ---------------------------------------------------------------------------
  // contextual_audit — system.control.* events carry caller_id + identity
  // ---------------------------------------------------------------------------
  const contextualAuditFixture = loadFixture('contextual_audit');
  describe('Contextual Audit (Issue #45.2)', () => {
    contextualAuditFixture.test_cases.forEach((tc: any) => {
      it(tc.id, async () => {
        // Build context. The fixture identity may carry display_name and
        // x-sensitive attrs (bearer_token); pack those into Identity.attrs
        // since the apcore Identity type only models id/type/roles/attrs.
        let identity: ReturnType<typeof createIdentity> | null = null;
        if (tc.context.identity !== null && tc.context.identity !== undefined) {
          const idData = tc.context.identity as Record<string, unknown>;
          const attrs: Record<string, unknown> = {};
          for (const [k, v] of Object.entries(idData)) {
            if (k === 'id' || k === 'type' || k === 'roles') continue;
            attrs[k] = v;
          }
          identity = createIdentity(
            String(idData.id ?? ''),
            String(idData.type ?? 'user'),
            Array.isArray(idData.roles) ? (idData.roles as string[]) : [],
            attrs,
          );
        }
        const ctx = new Context(
          'a'.repeat(32),
          tc.context.caller_id ?? null,
          [],
          null,
          identity,
        );

        const emitter = new EventEmitter();
        const captured: ApCoreEvent[] = [];
        emitter.subscribe({ onEvent: (e) => { captured.push(e); } });

        if (tc.module_id === 'system.control.update_config') {
          const config = new Config({ executor: { default_timeout: 30000 } });
          const mod = new UpdateConfigModule(config, emitter);
          mod.execute(tc.input as Record<string, unknown>, ctx);
        } else if (tc.module_id === 'system.control.toggle_feature') {
          const registry = new Registry();
          registry.registerInternal('risky.module', { description: 'x', execute: () => ({}) });
          const mod = new ToggleFeatureModule(registry, emitter);
          mod.execute(tc.input as Record<string, unknown>, ctx);
        } else if (tc.module_id === 'system.control.reload_module') {
          const registry = new Registry();
          const dummy = { description: 'x', version: '1.0.0', execute: () => ({}) };
          registry.registerInternal('executor.email.send', dummy);
          const replacement = { description: 'x', version: '2.0.0', execute: () => ({}) };
          vi.spyOn(registry, 'discover').mockImplementation(async () => {
            registry.registerInternal('executor.email.send', replacement);
            return 1;
          });
          const mod = new ReloadModule(registry, emitter);
          await mod.execute(tc.input as Record<string, unknown>, ctx);
        } else {
          throw new Error(`Unhandled module_id in fixture case: ${tc.module_id}`);
        }

        const evt = captured.find((e) => e.eventType === tc.expected.event_type);
        expect(evt, `expected event ${tc.expected.event_type}, got ${captured.map((e) => e.eventType).join(', ')}`).toBeDefined();
        const data = evt!.data;

        // data_contains: subset / deep-match assertion (recursive)
        if (tc.expected.data_contains !== undefined) {
          for (const [key, expectedVal] of Object.entries(tc.expected.data_contains as Record<string, unknown>)) {
            if (expectedVal !== null && typeof expectedVal === 'object' && !Array.isArray(expectedVal)) {
              expect(data[key]).toMatchObject(expectedVal as Record<string, unknown>);
            } else {
              expect(data[key]).toEqual(expectedVal);
            }
          }
        }
        if (Array.isArray(tc.expected.data_must_not_contain_keys)) {
          for (const k of tc.expected.data_must_not_contain_keys as string[]) {
            expect(k in data).toBe(false);
          }
        }
        // D-118 needs key ABSENCE inside the identity snapshot, which
        // `data_contains` cannot express: it is a SUBSET match, so an extra
        // `roles: []` passes it, and asserting `roles: []` is exactly what the
        // diverging SDK emits.
        if (Array.isArray(tc.expected.identity_must_not_contain_keys)) {
          const identity = (data as Record<string, unknown>)['identity'] ?? {};
          for (const k of tc.expected.identity_must_not_contain_keys as string[]) {
            expect(k in (identity as Record<string, unknown>)).toBe(false);
          }
        }
      });
    });
  });

  // ---------------------------------------------------------------------------
  // trace_context — W3C TraceContext alignment
  // ---------------------------------------------------------------------------
  const traceContextFixture = loadFixture('trace_context');
  describe('Trace Context (W3C, Issue #35)', () => {
    traceContextFixture.test_cases.forEach((tc: any) => {
      it(tc.id, () => {
        // Build the headers map. Some cases use `tracestate_entry_count`
        // to request the harness synthesize N entries (vendorXX=opaqueXX).
        const headers: Record<string, string> = {};
        for (const [k, v] of Object.entries(tc.input.headers as Record<string, unknown>)) {
          if (k === 'tracestate_entry_count') {
            const n = Number(v);
            const entries: string[] = [];
            for (let i = 0; i < n; i++) {
              const idx = String(i).padStart(2, '0');
              entries.push(`vendor${idx}=opaque${idx}`);
            }
            headers['tracestate'] = entries.join(',');
          } else {
            headers[k] = String(v);
          }
        }

        if (tc.id === 'parent_id_override_rejected_malformed') {
          // Inject with a malformed parent_id MUST throw an INVALID_PARENT_ID error.
          const tp = TraceContext.extract(headers);
          expect(tp).not.toBeNull();
          const ctx = new Context(tp!.traceId, null, [], null, null);
          expect(() => TraceContext.inject(ctx, tc.input.inject_parent_id as string)).toThrow();
          try {
            TraceContext.inject(ctx, tc.input.inject_parent_id as string);
          } catch (err: any) {
            expect(err.code).toBe(tc.expected.error.code);
          }
          return;
        }

        const tp = TraceContext.extract(headers);
        if (tc.expected.extract_succeeded === true) {
          expect(tp).not.toBeNull();
        }
        if (tp === null) {
          throw new Error(`extract returned null for ${tc.id}; headers=${JSON.stringify(headers)}`);
        }

        if (tc.expected.trace_id !== undefined) {
          expect(tp.traceId).toBe(tc.expected.trace_id);
        }
        if (tc.expected.parent_id !== undefined) {
          expect(tp.parentId).toBe(tc.expected.parent_id);
        }
        if (tc.expected.trace_flags !== undefined) {
          expect(tp.traceFlags).toBe(tc.expected.trace_flags);
        }
        if (tc.expected.tracestate_entries !== undefined) {
          const got = tp.tracestate.map((p) => [p[0], p[1]]);
          expect(got).toEqual(tc.expected.tracestate_entries);
        }
        if (tc.expected.tracestate_retained_count !== undefined) {
          expect(tp.tracestate.length).toBe(tc.expected.tracestate_retained_count);
        }
        if (tc.expected.tracestate_first_key !== undefined) {
          expect(tp.tracestate[0][0]).toBe(tc.expected.tracestate_first_key);
        }
        if (tc.expected.tracestate_last_key !== undefined) {
          expect(tp.tracestate[tp.tracestate.length - 1][0]).toBe(tc.expected.tracestate_last_key);
        }
        if (tc.expected.tracestate_dropped_count !== undefined) {
          // Compute dropped from input vs retained. For the malformed case
          // (3 declared, 2 retained, 1 dropped) and the cap case (35 vs 32).
          const tsRaw = headers['tracestate'] ?? '';
          const declared = tsRaw.length === 0 ? 0 : tsRaw.split(',').length;
          const retained = tp.tracestate.length;
          const dropped = declared - retained;
          expect(dropped).toBe(tc.expected.tracestate_dropped_count);
        }

        if (tc.expected.injected_traceparent !== undefined || tc.expected.parent_id_in_output !== undefined || tc.expected.injected_trace_flags !== undefined || tc.expected.reinjected_tracestate !== undefined || tc.expected.extracted_trace_flags !== undefined) {
          // Build a Context that carries the inbound TraceParent so inject()
          // round-trips traceFlags and tracestate.
          const ctx = new Context(tp.traceId, null, [], null, null);
          ctx.data['_apcore.trace.inbound'] = tp;

          const overrideParent = tc.input.inject_parent_id as string | undefined;
          const out = TraceContext.inject(ctx, overrideParent);

          if (tc.expected.extracted_trace_flags !== undefined) {
            expect(tp.traceFlags).toBe(tc.expected.extracted_trace_flags);
          }
          if (tc.expected.injected_trace_flags !== undefined) {
            const parts = out['traceparent'].split('-');
            expect(parts[3]).toBe(tc.expected.injected_trace_flags);
          }
          if (tc.expected.injected_traceparent !== undefined) {
            expect(out['traceparent']).toBe(tc.expected.injected_traceparent);
          }
          if (tc.expected.parent_id_in_output !== undefined) {
            const parts = out['traceparent'].split('-');
            expect(parts[2]).toBe(tc.expected.parent_id_in_output);
          }
          if (tc.expected.reinjected_tracestate !== undefined) {
            expect(out['tracestate']).toBe(tc.expected.reinjected_tracestate);
          }
        }
      });
    });
  });

  // --------------------------------------------------------------------------
  // Context.create unified signature (apcore Issue #66 / v0.22.0)
  //
  // Drives the canonical context_create.json fixture against the TS SDK.
  // Verifies the v0.22.0 normative input list (identity, trace_parent,
  // cancel_token, data, services, global_deadline) and the Executor-binding
  // contract.
  // --------------------------------------------------------------------------
  describe('Context.create unified signature (Issue #66)', () => {
    const fixture = loadFixture('context_create');
    // Shared CancelToken instances keyed by the fixture handle so the
    // "matches_input" expectation can be checked cross-case.
    const tokens: Record<string, CancelToken> = {};

    function getToken(handle: string): CancelToken {
      if (!(handle in tokens)) tokens[handle] = new CancelToken();
      return tokens[handle];
    }

    // The v0.22.0 normative caller-input list. Pinned here so the
    // "X is not a parameter" checks below cannot pass vacuously: if
    // createParamNames() ever fails to parse the signature it returns
    // something that does not equal this list, and the case goes red
    // instead of silently finding no `executor` parameter in an empty array.
    const NORMATIVE_CREATE_PARAMS = [
      'identity',
      'traceParent',
      'cancelToken',
      'data',
      'services',
      'globalDeadline',
    ];

    /**
     * Parameter names declared by `Context.create`, read off the function
     * source at runtime.
     *
     * `executor_is_not_a_parameter`, `caller_id_is_not_a_parameter` and
     * `no_separate_tracestate_parameter` are *signature* contracts — the
     * fixture's wording is "MUST NOT be a public Context.create() parameter".
     * Passing a value positionally and watching where it lands only describes
     * what slot 1 happens to do today; reading the parameter list is what goes
     * red the moment someone re-adds the slot.
     */
    function createParamNames(): string[] {
      const src = Context.create.toString();
      const open = src.indexOf('(');
      let depth = 0;
      let close = -1;
      for (let i = open; i < src.length; i++) {
        if (src[i] === '(') depth += 1;
        else if (src[i] === ')') {
          depth -= 1;
          if (depth === 0) {
            close = i;
            break;
          }
        }
      }
      if (open === -1 || close === -1) return [];
      return src
        .slice(open + 1, close)
        .split(',')
        .map((p) => p.split('=')[0].trim())
        .filter((p) => p.length > 0);
    }

    /**
     * Executor over every module id the fixture calls, recording the
     * `context.executor` each invocation observes. The recording is what makes
     * `executor_bound_on_first_call` / `executor_identity_stable` assertable:
     * TypeScript's Context is copy-on-write, so the caller-held reference never
     * observes the binding (the fixture calls this out as conforming) and the
     * module's view is the only place the bound executor is visible.
     */
    function makeEchoExecutor(): {
      executor: Executor;
      executorsSeen: () => unknown[];
    } {
      const registry = new Registry();
      const seen: unknown[] = [];
      for (const moduleId of ['test.echo', 'test.target', 'local.target', 'local.slow']) {
        registry.register(
          moduleId,
          new FunctionModule({
            execute: (inputs, context) => {
              seen.push(context.executor);
              return { ok: true, echoed: inputs };
            },
            moduleId,
            inputSchema: Type.Object({}, { additionalProperties: true }),
            outputSchema: Type.Object({}, { additionalProperties: true }),
            description: 'Echo with executor capture',
          }),
        );
      }
      return { executor: new Executor({ registry }), executorsSeen: () => [...seen] };
    }

    fixture.test_cases.forEach((tc: any) => {
      it(tc.id, async () => {
        const expected = tc['expected'] ?? {};
        switch (tc.id) {
          case 'create_minimal_all_defaults': {
            const ctx = Context.create();
            expect(ctx.traceId).toMatch(new RegExp(expected['trace_id_pattern']));
            expect(ctx.identity).toBe(expected['identity']);
            expect(ctx.executor).toBe(expected['executor']);
            expect(ctx.cancelToken).toBe(expected['cancel_token']);
            expect(ctx.services).toBe(expected['services']);
            expect(ctx.globalDeadline).toBe(expected['global_deadline']);
            expect(ctx.callerId).toBe(expected['caller_id']);
            expect(ctx.callChain).toEqual(expected['call_chain']);
            expect(Object.keys(ctx.data).length === 0).toBe(expected['data_empty']);
            break;
          }
          case 'create_with_identity_only': {
            const id = createIdentity(
              tc.input.identity.id,
              tc.input.identity.type,
              tc.input.identity.roles,
            );
            const ctx = Context.create(id);
            expect(ctx.traceId).toMatch(new RegExp(expected['trace_id_pattern']));
            expect(ctx.identity?.id).toBe(expected['identity_id']);
            expect(ctx.executor).toBe(expected['executor']);
            expect(ctx.cancelToken).toBe(expected['cancel_token']);
            break;
          }
          case 'create_with_cancel_token': {
            const token = getToken(tc.input.cancel_token_handle);
            const ctx = Context.create(undefined, undefined, token);
            expect(ctx.cancelToken !== null).toBe(expected['cancel_token_bound']);
            expect(ctx.cancelToken === token).toBe(expected['cancel_token_matches_input']);
            expect(ctx.executor).toBe(expected['executor_at_create_time']);
            break;
          }
          case 'create_with_global_deadline': {
            const ctx = Context.create(
              undefined, undefined, undefined, undefined, undefined,
              tc.input.global_deadline as number,
            );
            expect(ctx.globalDeadline).toBe(expected['global_deadline']);
            expect(ctx.executor).toBe(expected['executor']);
            break;
          }
          case 'create_rejects_executor_input': {
            const params = createParamNames();
            expect(params).toEqual(NORMATIVE_CREATE_PARAMS);
            // apcore#92: this was `if (expected[...]) { assert }` — a gate on the
            // expectation's truthiness, so a fixture saying the opposite simply
            // skipped the assertion and the case passed. The value is asserted.
            expect(params.filter((p) => /executor/i.test(p)).length === 0).toBe(
              expected['executor_is_not_a_parameter'],
            );
            // Belt and braces: an Executor-shaped object pushed into the only
            // object-typed leading slot lands on `identity`, never on
            // `executor`, so "pass executor at construction" stays impossible
            // through the public API.
            const fakeExecutor = { name: 'fake' };
            const ctx = Context.create(fakeExecutor as any);
            expect(ctx.executor).toBeNull();
            break;
          }
          case 'create_rejects_caller_id_input': {
            const params = createParamNames();
            expect(params).toEqual(NORMATIVE_CREATE_PARAMS);
            // apcore#92: same truthiness gate as above; the value now asserts.
            expect(params.filter((p) => /caller/i.test(p)).length === 0).toBe(
              expected['caller_id_is_not_a_parameter'],
            );
            const ctx = Context.create();
            expect(ctx.callerId).toBe(expected['caller_id_after_create']);
            break;
          }
          case 'executor_binds_on_first_call_local': {
            const { executor, executorsSeen } = makeEchoExecutor();
            const ctx = Context.create();
            expect(ctx.executor).toBe(expected['executor_at_create_time']);

            let bindingError: unknown = null;
            let completed = false;
            try {
              await executor.call(tc.input.call_module, {}, ctx);
              completed = true;
            } catch (e) {
              bindingError = e;
            }
            // Binding first, then completion: a binding that starts refusing a
            // fresh Context trips `raised_binding_error`, while a call that dies
            // for any other reason trips `call_succeeded`. Asserting completion
            // first would have collapsed both regressions onto one expectation.
            expect(bindingError instanceof ContextBindingError).toBe(
              expected['raised_binding_error'],
            );
            expect(completed).toBe(expected['call_succeeded']);
            // The binding actually happened: the module saw the executor.
            expect(executorsSeen()).toEqual([executor]);
            break;
          }
          case 'executor_binds_idempotent_same_instance': {
            const { executor, executorsSeen } = makeEchoExecutor();
            const ctx = Context.create();
            const calls = tc.input.calls as string[];

            let raised: unknown = null;
            for (const mid of calls) {
              try {
                await executor.call(mid, {}, ctx);
              } catch (e) {
                raised = e;
                break;
              }
            }
            const seen = executorsSeen();
            // Reusing one Context across N calls on the same Executor is a
            // no-op from the 2nd call on: every call runs and none raises.
            // apcore#92: each expectation is asserted against its own observable
            // rather than gating a block on its truthiness, so a fixture that
            // said the opposite would go red instead of skipping the checks.
            // Still three separate assertions, so a rebind that starts throwing
            // is attributed to `rebind_noop` and not to `raised_error`.
            expect(seen.length === calls.length && raised === null).toBe(expected['rebind_noop']);
            expect(raised !== null).toBe(expected['raised_error']);
            if (tc.input.same_executor) {
              expect(new Set(seen).size === 1 && seen[0] === executor).toBe(
                expected['executor_identity_stable'],
              );
            }
            break;
          }
          case 'executor_rejects_cross_executor_rebind': {
            const { executor: a } = makeEchoExecutor();
            const { executor: b } = makeEchoExecutor();
            // Bind to A using the internal helper, then try B.
            //
            // apcore#92 / spec v1.11.0: this case used to carry
            // `expected_one_of: [raise, silent_accept]`. A driver cannot assert
            // an alternation, so this branch hardcoded "raise" and named the
            // alternation only in a comment — the fixture value never reached an
            // assertion. The SHOULD is now a MUST and the fixture states one
            // behaviour, `{raises: true, error_code: CONTEXT_BINDING_ERROR}`,
            // which is what is read here. The wire code, not the class:
            // `ContextBindingError` is a name apcore-rust does not have.
            const ctx = Context.create()._withExecutor(a);
            const thrown = captureThrow(() => ctx._withExecutor(b));
            expect(thrown !== null).toBe(expected['raises']);
            assertWireCode(tc.id, expected['error_code'], thrown);
            break;
          }
          case 'child_propagates_executor': {
            const { executor: a } = makeEchoExecutor();
            const root = tc.input.bind_executor
              ? Context.create()._withExecutor(a)
              : Context.create();
            // The parent is given a non-empty call_chain on purpose:
            // "caller_id from the parent's chain tip" is only distinguishable
            // from "caller_id copied from the parent" when the two differ.
            const parent = root.child('orchestrator.main');
            const child = parent.child(tc.input.create_child_module_id);

            expect(child.executor === parent.executor).toBe(
              expected['child_executor_matches_parent'],
            );
            // apcore#92: both of these gated their assertions on the
            // expectation's truthiness; the values now reach an assertion.
            expect(
              child.callerId === parent.callChain[parent.callChain.length - 1] &&
                child.callerId !== parent.callerId,
            ).toBe(expected['child_caller_id_from_parent_chain_tip']);
            expect(
              deepEq(child.callChain, [...parent.callChain, tc.input.create_child_module_id]),
            ).toBe(expected['child_call_chain_appends_target']);
            break;
          }
          case 'child_propagates_cancel_token': {
            const token = getToken(tc.input.create_with_cancel_token);
            const parent = Context.create(undefined, undefined, token);
            const child = parent.child(tc.input.create_child_module_id);
            expect(child.cancelToken !== null).toBe(expected['child_cancel_token_bound']);
            expect(child.cancelToken === parent.cancelToken).toBe(
              expected['child_cancel_token_matches_parent'],
            );
            break;
          }
          case 'deserialize_then_call_binds_local_executor': {
            const restored = Context.fromJSON({ ...tc.input.serialized_context });
            // §5.7 invariants: stripped fields are null after deserialize.
            expect(restored.executor).toBe(expected['executor_after_deserialize']);
            expect(restored.cancelToken).toBe(expected['cancel_token_after_deserialize']);
            expect(restored.services).toBe(expected['services_after_deserialize']);
            expect(restored.globalDeadline).toBe(expected['global_deadline_after_deserialize']);
            expect(restored.callerId).toBe(expected['caller_id_preserved']);

            const { executor, executorsSeen } = makeEchoExecutor();
            await executor.call(tc.input.call_module, {}, restored);
            // The receiving node's Executor bound itself to the arriving
            // Context under the same rule as a locally created one. apcore#92:
            // asserted against the value, not gated on its truthiness.
            const boundSeen = executorsSeen();
            expect(boundSeen.length === 1 && boundSeen[0] === executor).toBe(
              expected['executor_bound_on_first_call'],
            );
            break;
          }
          case 'distributed_cancel_token_post_deserialize_null': {
            // Serialize a Context that DOES hold a token: the invariant is that
            // a token cannot ride across a process boundary, so the outbound
            // wire form must not carry it in the first place. Round-tripping a
            // hand-written payload with no token in it asserts nothing.
            const withToken = Context.create(undefined, undefined, new CancelToken());
            const wire = withToken.toJSON();
            // apcore#92: asserted against the value, not gated on its truthiness.
            expect(
              Object.keys(wire).filter((k) => /cancel|token/i.test(k)).length === 0 &&
                !JSON.stringify(wire).includes('CancelToken'),
            ).toBe(expected['no_in_context_token_rides_across_processes']);
            const restored = Context.fromJSON(wire);
            expect(restored.cancelToken).toBe(expected['cancel_token_after_deserialize']);
            break;
          }
          case 'distributed_global_deadline_post_deserialize_null': {
            // Same shape: serialize a Context that HAS a global_deadline and
            // assert the field never reaches the wire, so a remote deadline
            // cannot ride in on it.
            const withDeadline = Context.create(
              undefined, undefined, undefined, undefined, undefined, 1234567890.5,
            );
            const wire = withDeadline.toJSON();
            // apcore#92: asserted against the value, not gated on its truthiness.
            expect(
              Object.keys(wire).filter((k) => /deadline/i.test(k)).length === 0 &&
                !JSON.stringify(wire).includes('1234567890.5'),
            ).toBe(expected['no_remote_deadline_rides_via_global_deadline_field']);
            const restored = Context.fromJSON(wire);
            expect(restored.globalDeadline).toBe(expected['global_deadline_after_deserialize']);
            break;
          }
          case 'tracestate_carried_inside_traceparent': {
            const tp = {
              version: '00',
              traceId: tc.input.trace_parent.trace_id,
              parentId: tc.input.trace_parent.parent_id,
              traceFlags: tc.input.trace_parent.trace_flags,
              tracestate: tc.input.trace_parent.tracestate,
            };
            const ctx = Context.create(null, tp as any);
            expect(ctx.traceId).toBe(expected['trace_id']);
            // Tracestate is preserved via the two scalar keys (cross-language
            // parity: `_apcore.trace.flags` + `_apcore.trace.state`).
            // apcore#92: asserted against the value, not gated on its truthiness.
            expect(deepEq(ctx.data['_apcore.trace.state'], tp.tracestate)).toBe(
              expected['tracestate_preserved'],
            );
            const params = createParamNames();
            expect(params).toEqual(NORMATIVE_CREATE_PARAMS);
            expect(params.filter((p) => /trace_?state/i.test(p)).length === 0).toBe(
              expected['no_separate_tracestate_parameter'],
            );
            break;
          }
          default:
            throw new Error(`unhandled fixture case: ${tc.id}`);
        }
      });
    });
  });
});
