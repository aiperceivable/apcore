/**
 * Executor and related utilities for apcore.
 *
 * Async-only execution pipeline. Python's call() + call_async() merge into one async call().
 * Timeout uses Promise.race instead of threading.
 */

import type { ACL } from './acl.js';
import {
  createRedactionReferenceContext,
  resolveRedactionReference,
  type RedactionReferenceContext,
} from './utils/redaction-references.js';
import type { ApprovalHandler } from './approval.js';
import type { ExecutionPolicy } from './policy.js';
import {
  BuiltinACLCheck,
  BuiltinApprovalGate,
  needsApproval,
  buildInternalStrategy,
  buildMinimalStrategy,
  buildPerformanceStrategy,
  buildStandardStrategy,
  buildTestingStrategy,
} from './builtin-steps.js';
import type { StandardStrategyDeps } from './builtin-steps.js';
import { ExecutionCancelledError } from './cancel.js';
import type { Config } from './config.js';
import { buildStrategyFromConfigSync } from './pipeline-config.js';
// Import directly from emitter.js (browser-safe) — NOT events/index.js, which
// re-exports subscribers.ts (Node-only: node:fs, process) and would drag
// Node-only code into the browser dependency graph.
import { type EventEmitter, createEvent } from './events/emitter.js';
import { Context } from './context.js';
import { governanceUnion } from './schema/annotations.js';
import { RedactionConfig } from './observability/context-logger.js';
import {
  ContextBindingError,
  ErrorCodes,
  InvalidInputError,
  ModuleError,
  ModuleTimeoutError,
} from './errors.js';
import {
  AfterMiddleware,
  BeforeMiddleware,
  type Middleware,
  RetrySignal,
} from './middleware/index.js';
import { MiddlewareChainError, MiddlewareManager } from './middleware/manager.js';
import type {
  Change,
  Module,
  ModuleAnnotations,
  PreflightCheckResult,
  PreflightResult,
  PreviewResult,
} from './module.js';
import { createPreflightResult, isModuleIdempotent } from './module.js';
import type {
  GovernanceState,
  PipelineContext,
  PipelineTrace,
  StepMiddleware,
  StrategyInfo,
} from './pipeline.js';
import {
  ExecutionStrategy,
  PipelineAbortError,
  PipelineEngine,
  PipelineStepError,
  StrategyNotFoundError,
} from './pipeline.js';
import { MAX_MODULE_ID_LENGTH, MODULE_ID_PATTERN } from './registry/registry.js';
import type { Registry } from './registry/registry.js';
import type { ToggleState } from './sys-modules/toggle.js';
import { propagateError } from './utils/error-propagation.js';

export const REDACTED_VALUE: string = '***REDACTED***';

/** Steps whose removal withdraws a protection (PROTOCOL_SPEC §5.16 req 7). */
const SECURITY_STEPS = ['acl_check', 'approval_gate'] as const;

/** The `pipeline:` block of a loaded configuration, or `null`. */
/**
 * A per-callback identity for the `useBefore` / `useAfter` wrappers.
 *
 * Every wrapper is a `BeforeMiddleware` / `AfterMiddleware`, so the manager's
 * default identity (`constructor.name`) made the second distinct callback look
 * like a duplicate registration. Keying on the callback instead, as
 * apcore-python does with `id(callback)`, still warns when the SAME callback
 * is registered twice.
 */
const callbackIds = new WeakMap<object, number>();
let nextCallbackId = 0;

function callbackIdentity(kind: 'before' | 'after', callback: object): string {
  let id = callbackIds.get(callback);
  if (id === undefined) {
    id = nextCallbackId++;
    callbackIds.set(callback, id);
  }
  return `apcore.${kind}.${id}`;
}

function pipelineSectionOf(config: Config | null): Record<string, unknown> | null {
  if (config === null) return null;
  const section = config.get('pipeline');
  if (section === null || typeof section !== 'object' || Array.isArray(section)) return null;
  return Object.keys(section).length > 0 ? (section as Record<string, unknown>) : null;
}

/**
 * §5.16 requirement 7 — removing `acl_check` or `approval_gate` warns.
 *
 * The reason is the TRANSITION, not the steady state. Requirement 6 makes a
 * previously ignored section take effect, so a configuration that has been
 * carrying `remove: [acl_check]` while ACL was enforced anyway starts having
 * ACL genuinely removed. That is the operator getting what they asked for, and
 * equally the one direction in which honouring configuration can withdraw a
 * protection that was in place a moment earlier.
 *
 * A notice, never a refusal: the configuration is valid and was written
 * deliberately, and rejecting it would break projects whose `pipeline:` block
 * is harmless.
 */
function warnRemovedSecuritySteps(section: Record<string, unknown>): void {
  const remove = Array.isArray(section['remove']) ? (section['remove'] as unknown[]) : [];
  const removed = remove.filter((n): n is string =>
    (SECURITY_STEPS as readonly string[]).includes(n as string),
  );
  if (removed.length === 0) return;
  const consequence = removed.includes('acl_check')
    ? 'Inter-module calls are no longer access-checked'
    : 'Calls requiring approval are no longer gated';
  console.warn(
    `[apcore] pipeline.remove takes ${removed.map((n) => `'${n}'`).join(' and ')} out of the ` +
      'execution pipeline. Until spec v1.43.0 a `pipeline:` section was accepted and then ' +
      'ignored, so this removal may not have been in effect before this release even though ' +
      `it was configured. ${consequence}. See PROTOCOL_SPEC 5.16 requirement 7.`,
  );
}

/**
 * @deprecated Nothing reads or writes this key any more. Spec v1.50.0 D-100
 * makes the first-class `Context.globalDeadline` field the storage for the
 * execution budget: `data` is caller-visible, caller-writable and shared by
 * reference with child contexts, so a budget kept there could not be read
 * back through the documented `Context.create` parameter and could not be
 * scoped to one call. Read `context.globalDeadline` (epoch seconds) instead.
 * The constant is retained only so the public export does not disappear
 * mid-major; it will be removed at 2.0.
 */
export const CTX_GLOBAL_DEADLINE = '_apcore.executor.global_deadline';

/** Well-known context.data keys used internally by the runtime. */
export const CTX_TRACING_SPANS = '_apcore.mw.tracing.spans';

/**
 * Redact for the executor's input/output capture point.
 *
 * PROTOCOL_SPEC §10.6.1 "Where the rules apply": the union of §10.6's
 * `x-sensitive` rule and the two configured `obs.redaction.*` rules MUST hold
 * at BOTH log emission and this capture point, with the same rules at each.
 * Until `rules` existed this function had no parameter for them at all — the
 * signature was `(data, schemaDict)`, faithfully mirroring §10.6's published
 * algorithm — so an operator's `regex_patterns` entry redacted a bearer token
 * in the log line they were watching and stored it in the audit record they
 * were not (aiperceivable/apcore#120).
 *
 * `rules` omitted means the spec DEFAULTS, per requirement 3: "no
 * configuration" means the defaults, never no redaction. That is what this
 * function did before, so an unconfigured caller sees no change.
 */
export function redactSensitive(
  data: Record<string, unknown>,
  schemaDict: Record<string, unknown>,
  rules?: RedactionConfig | null,
): Record<string, unknown> {
  const redacted = JSON.parse(JSON.stringify(data));
  redactFields(redacted, schemaDict, createRedactionReferenceContext(schemaDict));
  redactSecretPrefix(redacted);
  const configured = rules ?? RedactionConfig.default();
  return configured.redact(redacted) as Record<string, unknown>;
}

const COMBINATORS = ['anyOf', 'oneOf', 'allOf'] as const;

/**
 * A schema and every `anyOf` / `oneOf` / `allOf` branch beneath it.
 *
 * The redaction walk does not decide which branch a value matched: a value is
 * sensitive when ANY schema describing it carries `x-sensitive`, and the
 * properties declared in any branch are fields of the object like those
 * declared directly (protocol-spec §10.6, A13, D-152). `Secret | None` puts the
 * marker in `anyOf[0]`, so a walk that read only the top level logged it.
 */
function schemaVariants(
  schema: Record<string, unknown>,
  context: RedactionReferenceContext,
): Record<string, unknown>[] {
  const variants: Record<string, unknown>[] = [];
  const pending = [schema];
  const visited = new Set<object>();
  while (pending.length > 0) {
    const variant = pending.pop();
    if (variant === undefined || visited.has(variant)) continue;
    visited.add(variant);
    variants.push(variant);
    const reference = variant['$ref'];
    if (typeof reference === 'string') {
      const target = resolveRedactionReference(reference, context, variant);
      if (target !== undefined) pending.push(target);
    }
    for (const keyword of COMBINATORS) {
      const branches = variant[keyword];
      if (!Array.isArray(branches)) continue;
      for (const branch of branches) if (isPlainObject(branch)) pending.push(branch);
    }
  }
  return variants;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Redact `value` as described by `schema`; returns the value to store. */
function redactBySchema(
  value: unknown,
  schema: Record<string, unknown>,
  context: RedactionReferenceContext,
): unknown {
  if (value === null || value === undefined) return value;
  const variants = schemaVariants(schema, context);
  if (variants.some((v) => v['x-sensitive'] === true)) return REDACTED_VALUE;

  if (Array.isArray(value)) {
    for (const variant of variants) redactArrayBySchema(value, variant, context);
    return value;
  }

  if (isPlainObject(value)) {
    redactFields(value, schema, context);
  }
  return value;
}

function redactArrayBySchema(value: unknown[], schema: Record<string, unknown>, context: RedactionReferenceContext): void {
  const dialect = context.dialects.get(schema) ?? context.rootSchema['$schema'];
  const legacyTuple = typeof dialect === 'string' && (dialect.includes('/draft-07/') || dialect.includes('/2019-09/'));
  const tuple = legacyTuple && Array.isArray(schema['items']) ? schema['items'] : undefined;
  const prefix = !legacyTuple && Array.isArray(schema['prefixItems']) ? schema['prefixItems'] : undefined;
  for (let index = 0; index < value.length; index++) {
    const itemSchema = prefix ? prefix[index] ?? schema['items']
      : tuple ? tuple[index] ?? schema['additionalItems'] : schema['items'];
    if (isPlainObject(itemSchema)) value[index] = redactBySchema(value[index], itemSchema, context);
  }
}

function redactFields(
  data: Record<string, unknown>,
  schemaDict: Record<string, unknown>,
  context: RedactionReferenceContext,
): void {
  for (const variant of schemaVariants(schemaDict, context)) {
    const properties = variant['properties'];
    if (!isPlainObject(properties)) continue;
    for (const [fieldName, fieldSchema] of Object.entries(properties)) {
      if (!(fieldName in data) || !isPlainObject(fieldSchema)) continue;
      data[fieldName] = redactBySchema(data[fieldName], fieldSchema, context);
    }
  }
}

function redactSecretPrefix(data: Record<string, unknown>): void {
  for (const key of Object.keys(data)) {
    const value = data[key];
    if (key.startsWith('_secret_') && value !== null && value !== undefined) {
      data[key] = REDACTED_VALUE;
    } else if (Array.isArray(value)) {
      // Recurse into array elements so _secret_ keys inside objects (or
      // deeper nested arrays) are redacted too. Mirrors apcore-python
      // utils/redaction.py `_redact_in_list` (sync finding A-D-003).
      redactSecretPrefixInList(value);
    } else if (typeof value === 'object' && value !== null) {
      redactSecretPrefix(value as Record<string, unknown>);
    }
  }
}

function redactSecretPrefixInList(items: unknown[]): void {
  for (const item of items) {
    if (Array.isArray(item)) {
      redactSecretPrefixInList(item);
    } else if (typeof item === 'object' && item !== null) {
      redactSecretPrefix(item as Record<string, unknown>);
    }
  }
}

const MAX_MERGE_DEPTH = 32;

/**
 * Resolve `stream.max_merge_depth` (PROTOCOL_SPEC §5; canonical default 32).
 *
 * §5 calls 32 the *canonical default*, which implies an override — and until
 * now there was none: the key was declared, schema-documented and read by no
 * code path (apcore#118), so the constant WAS the contract.
 *
 * A non-positive or non-integer value falls back to the canonical default
 * rather than disabling the cap. The cap exists to prevent stack exhaustion
 * from adversarial chunk shapes, so a misconfiguration must not remove it.
 */
export function resolveMergeDepth(config: Config | null | undefined): number {
  if (config == null) return MAX_MERGE_DEPTH;
  let value: unknown;
  try {
    value = config.get('stream.max_merge_depth');
  } catch {
    return MAX_MERGE_DEPTH;
  }
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 1) {
    return MAX_MERGE_DEPTH;
  }
  return value;
}

export function deepMergeChunk(
  base: Record<string, unknown>,
  overlay: Record<string, unknown>,
  depth = 0,
  maxDepth: number = MAX_MERGE_DEPTH,
): void {
  if (depth >= maxDepth) {
    // At the depth cap, replace rather than recurse to avoid stack overflow.
    // Spec mandates right-value-wins, so shallow-assign each overlay key onto base.
    for (const [key, value] of Object.entries(overlay)) {
      base[key] = value;
    }
    return;
  }
  for (const [key, value] of Object.entries(overlay)) {
    if (
      key in base &&
      base[key] != null &&
      typeof base[key] === 'object' &&
      !Array.isArray(base[key]) &&
      value != null &&
      typeof value === 'object' &&
      !Array.isArray(value)
    ) {
      deepMergeChunk(
        base[key] as Record<string, unknown>,
        value as Record<string, unknown>,
        depth + 1,
        maxDepth,
      );
    } else {
      base[key] = value;
    }
  }
}

/**
 * JSON type name for a value, matching apcore-rust `json_type_name` EXACTLY
 * (D-58 / D10-001 cross-SDK parity): null→"null", boolean→"bool",
 * number→"number", string→"string", array→"array", object→"object".
 * Used to populate the STREAM_CHUNK_NOT_OBJECT error's `actual_type`.
 */
function jsonTypeName(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  switch (typeof value) {
    case 'boolean':
      return 'bool';
    case 'number':
      return 'number';
    case 'string':
      return 'string';
    default:
      return 'object';
  }
}

/**
 * A valid stream chunk is a plain JSON object (not null, not an array, not a
 * scalar). Mirrors apcore-rust `Value::is_object()`.
 */
function isPlainObjectChunk(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Bind the Executor to a Context, tolerating both the current public method
 * (`withExecutor`) and the deprecated alias (`_withExecutor`). This keeps the
 * Executor robust against foreign Context implementations (e.g. the duck-typed
 * BridgeContext in apcore-mcp-typescript) during the deprecation window, where
 * either name may be the only one present.
 */
function bindExecutorToCtx<C>(ctx: C, executor: unknown): C {
  const c = ctx as any;
  return (typeof c.withExecutor === 'function'
    ? c.withExecutor(executor)
    : c._withExecutor(executor)) as C;
}

export class Executor {
  private _registry: Registry;
  private _middlewareManager: MiddlewareManager;
  private _acl: ACL | null;
  private _config: Config | null;
  private _approvalHandler: ApprovalHandler | null;
  private _policy: ExecutionPolicy | null;
  private _toggleState: ToggleState | null;
  private _eventEmitter: EventEmitter | null;
  private _strategy: ExecutionStrategy;
  private _pipelineEngine: PipelineEngine;

  /** Global strategy registry for name-based resolution. */
  private static _strategyRegistry = new Map<string, ExecutionStrategy>();

  constructor(options: {
    registry: Registry;
    strategy?: ExecutionStrategy | string | null;
    middlewares?: Middleware[] | null;
    acl?: ACL | null;
    config?: Config | null;
    approvalHandler?: ApprovalHandler | null;
    policy?: ExecutionPolicy | null;
    toggleState?: ToggleState | null;
    eventEmitter?: EventEmitter | null;
  }) {
    this._registry = options.registry;
    this._middlewareManager = new MiddlewareManager();
    this._acl = options.acl ?? null;
    this._config = options.config ?? null;
    this._approvalHandler = options.approvalHandler ?? null;
    this._policy = options.policy ?? null;
    this._toggleState = options.toggleState ?? null;
    this._eventEmitter = options.eventEmitter ?? null;

    if (options.middlewares) {
      for (const mw of options.middlewares) {
        this._middlewareManager.add(mw);
      }
    }

    // Resolve strategy option (default to standard)
    const strategyOpt = options.strategy;
    if (strategyOpt === undefined || strategyOpt === null) {
      // PROTOCOL_SPEC §5.16 requirement 6: a configured `pipeline:` section MUST
      // be applied. `buildStrategyFromConfig` has always existed and takes the
      // section as an object, but nothing extracted that object from a loaded
      // Config — so `pipeline: remove: [acl_check]` in apcore.yaml left all
      // eleven steps in place and a declared custom step silently never ran
      // (apcore#118, decision D-72).
      //
      // Only when the caller supplied no explicit strategy: an explicit one is
      // an API argument and wins over configuration, per D-73.
      const pipelineSection = pipelineSectionOf(options.config ?? null);
      if (pipelineSection !== null) {
        warnRemovedSecuritySteps(pipelineSection);
        const { strategy, deferred } = buildStrategyFromConfigSync(
          pipelineSection,
          this._buildStrategyDeps(),
        );
        if (deferred.length > 0) {
          // A `handler:` step is an ESM specifier and needs `await import()`,
          // which a synchronous constructor cannot do. Reported rather than
          // dropped: omitting it silently is the defect §5.16 requirement 6
          // exists to remove, and throwing would stop a project from starting
          // over a step that has never once run.
          console.warn(
            `[apcore] ${deferred.length} pipeline step(s) declare a 'handler:' target and ` +
              `were NOT inserted: ${deferred.join(', ')}. Handler targets are ESM specifiers ` +
              `and need an await, which the Executor constructor cannot do — build the ` +
              `strategy with the async buildStrategyFromConfig() and pass it as ` +
              `options.strategy. Steps declared with 'type:' were inserted normally.`,
          );
        }
        this._strategy = strategy;
      } else {
        this._strategy = buildStandardStrategy(this._buildStrategyDeps());
      }
    } else if (typeof strategyOpt === 'string') {
      // Resolve by name from the global registry
      const resolved = Executor._strategyRegistry.get(strategyOpt);
      if (resolved === undefined) {
        // Try built-in factory names
        const deps = this._buildStrategyDeps();
        const builtinFactories: Record<string, (d: StandardStrategyDeps) => ExecutionStrategy> = {
          standard: buildStandardStrategy,
          internal: buildInternalStrategy,
          testing: buildTestingStrategy,
          performance: buildPerformanceStrategy,
          minimal: buildMinimalStrategy,
        };
        const factory = builtinFactories[strategyOpt];
        if (factory !== undefined) {
          this._strategy = factory(deps);
        } else {
          throw new StrategyNotFoundError(`Strategy '${strategyOpt}' not found in registry`);
        }
      } else {
        this._strategy = resolved;
      }
    } else {
      // ExecutionStrategy instance
      this._strategy = strategyOpt;
    }

    // PROTOCOL_SPEC 6.6.5.5 (D-129): however the strategy arrived — default,
    // preset name, `pipeline:` section, a registered name or a pre-built
    // instance — the executor's providers MUST end up in its built-in gates.
    // The instance and registered-name branches above never passed them, so a
    // deny-all ACL given here let every call through while governanceState()
    // reported the gate as configured and wired.
    this._bindProviders();

    this._pipelineEngine = new PipelineEngine();
  }

  /**
   * Bind the executor's providers into the running strategy's built-in gate
   * steps (PROTOCOL_SPEC 6.6.5.5, D-129).
   *
   * Gates are located by TYPE, as `governanceState()` detects them (6.6.5.2):
   * a custom step that happens to be named `acl_check` is not handed the ACL.
   * A provider the executor holds replaces the step's; a provider it does not
   * hold leaves the step's provider untouched, so a strategy built with its
   * own ACL keeps enforcing it.
   *
   * Mutates the step objects of the strategy it is given. A strategy instance
   * shared between executors therefore carries the providers of the executor
   * that bound it last — the same effect `setAcl()` has always had.
   */
  private _bindProviders(strategy: ExecutionStrategy = this._strategy): void {
    for (const step of strategy.steps) {
      if (step instanceof BuiltinACLCheck) {
        if (this._acl !== null) step.setAcl(this._acl);
      } else if (step instanceof BuiltinApprovalGate) {
        if (this._approvalHandler !== null) step.setApprovalHandler(this._approvalHandler);
        if (this._policy !== null) step.setPolicy(this._policy);
      }
    }
  }

  /** Build the dependency bag for strategy factories. */
  private _buildStrategyDeps(): StandardStrategyDeps {
    return {
      config: this._config,
      registry: this._registry,
      acl: this._acl,
      approvalHandler: this._approvalHandler,
      middlewareManager: this._middlewareManager,
      toggleState: this._toggleState,
      policy: this._policy,
      eventEmitter: this._eventEmitter,
    };
  }

  // -----------------------------------------------------------------------
  // Static strategy registry (introspection - Task 4)
  // -----------------------------------------------------------------------

  /** Register a named strategy in the global registry. */
  static registerStrategy(name: string, strategy: ExecutionStrategy): void {
    Executor._strategyRegistry.set(name, strategy);
  }

  /**
   * List info for the executor's current strategy plus every strategy
   * registered via {@link Executor.registerStrategy}.
   *
   * `core-executor.md` "Contract: Executor.list_strategies" specifies one
   * entry for the executor's current strategy followed by one entry per
   * registered strategy, deduplicated by name. That needs the instance's
   * `_strategy`, which a static method cannot reach: because no built-in
   * strategy is pre-seeded into the static registry, a default-constructed
   * Executor running the built-in `standard` strategy reported an empty
   * list where apcore-python `Executor.list_strategies` returned one entry.
   * `docs/features/execution-pipeline.md` (Introspection) documents this call
   * as returning the current strategy first, so the empty list was not merely
   * an introspection gap.
   */
  listStrategies(): StrategyInfo[] {
    const seen = new Set<string>();
    const result: StrategyInfo[] = [];

    // Current strategy first.
    const current = this._strategy.info();
    result.push(current);
    seen.add(current.name);

    // Then the registered strategies, in name order so this SDK and
    // apcore-python (which iterates `sorted(...)`) list the same sequence.
    for (const name of [...Executor._strategyRegistry.keys()].sort()) {
      if (seen.has(name)) continue;
      const strategy = Executor._strategyRegistry.get(name);
      if (strategy === undefined) continue;
      result.push(strategy.info());
      seen.add(name);
    }

    return result;
  }

  /**
   * List info for the strategies in the static registry only.
   *
   * @deprecated Use the instance method {@link Executor#listStrategies},
   * which also includes the executor's current strategy as the spec
   * requires. This static overload never could: it has no instance to read
   * `_strategy` from. Retained for callers written against the old surface.
   */
  static listStrategies(): StrategyInfo[] {
    return [...Executor._strategyRegistry.values()].map((s) => s.info());
  }

  /** Describe the pipeline of the executor's current strategy. */
  describePipeline(): StrategyInfo {
    return this._strategy.info();
  }

  /** Get the current execution strategy. */
  get currentStrategy(): ExecutionStrategy {
    return this._strategy;
  }

  /**
   * Return what is actually gating this executor (PROTOCOL_SPEC 6.6.5).
   *
   * A **pure read**: it never enforces, warns, throws or mutates. What to do
   * about an unprotected control surface belongs to the caller — a serve-time
   * adapter may warn or refuse, a test may assert, a health endpoint may
   * report. Putting the reaction here would make it unavoidable and
   * untestable.
   *
   * Computed fresh on every call, so attaching an ACL or swapping the strategy
   * is visible in the next one.
   */
  governanceState(): GovernanceState {
    // Gate detection is by TYPE, never by step name (PROTOCOL_SPEC 6.6.5.2).
    // `StrategyInfo` carries names only, and a custom step named `acl_check`
    // that never consults an ACL would satisfy a name test — producing
    // `builtinAclGateWired: true` for a gate that is not there. That is the one
    // direction this accessor must never fail in.
    const steps = this._strategy.steps;
    const aclGates = steps.filter(
      (step): step is BuiltinACLCheck => step instanceof BuiltinACLCheck,
    );
    const approvalGates = steps.filter(
      (step): step is BuiltinApprovalGate => step instanceof BuiltinApprovalGate,
    );
    const builtinAclGateWired = aclGates.length > 0;
    const builtinApprovalGateWired = approvalGates.length > 0;

    // `visibility` must include hidden: the accessor reports what is
    // REGISTERED, and `list()` defaults to public-only. A control module
    // registered with `discoverable: false` is still callable by ID, so
    // omitting it would under-report the write surface.
    const allIds = this._registry.list({ visibility: ['public', 'hidden'] });
    const controlIds = allIds.filter((id) => id.startsWith('system.control.'));
    const readModulesRegistered = allIds.some(
      (id) =>
        id.startsWith('system.health.') ||
        id.startsWith('system.usage.') ||
        id.startsWith('system.manifest.'),
    );

    // Read governance through the SAME union the approval gate uses (D-96).
    // A second, narrower reading here would be a second way for the accessor to
    // disagree with the pipeline it describes: a control module whose
    // requirement is declared in a metadata source, not on the instance, is
    // gated by the pipeline and was reported here as ungated — which a
    // serve-time adapter may act on by refusing to start.
    const allControlModulesRequireApproval =
      controlIds.length > 0 &&
      controlIds.every((id) =>
        Boolean(
          governanceUnion(
            (this._registry.get(id) as Record<string, unknown> | null)?.['annotations'],
            this._registry.getDeclaredAnnotations(id),
          )?.requiresApproval,
        ),
      );

    // PROTOCOL_SPEC 6.6.5.5 requirement 3 (D-129): when a built-in gate is
    // wired, report what that running gate HOLDS — not what the executor was
    // given. A gate enforcing an ACL the executor never saw is configured; an
    // executor-held provider that no running gate enforces is configured and
    // not wired. `every` keeps a strategy with several gates from reporting a
    // provider one of them lacks — the direction 6.6.5.2 forbids.
    const aclConfigured = builtinAclGateWired
      ? aclGates.every((gate) => gate.acl !== null)
      : this._acl !== null;
    const approvalHandlerConfigured = builtinApprovalGateWired
      ? approvalGates.every((gate) => gate.approvalHandler !== null)
      : this._approvalHandler !== null;
    const policyStrict = builtinApprovalGateWired
      ? approvalGates.every((gate) => gate.policy !== null && gate.policy.strict === true)
      : this._policy !== null && this._policy.strict === true;

    // PROTOCOL_SPEC 6.6.5.1. The approval conjunct carries
    // `allControlModulesRequireApproval` because `approval_gate` is per-module
    // conditional while `acl_check` is not (6.6.5.1.1).
    const unprotectedControlSurface =
      controlIds.length > 0 &&
      !(aclConfigured && builtinAclGateWired) &&
      !(
        builtinApprovalGateWired &&
        allControlModulesRequireApproval &&
        (approvalHandlerConfigured || policyStrict)
      );

    return {
      controlModulesRegistered: controlIds.length > 0,
      readModulesRegistered,
      aclConfigured,
      builtinAclGateWired,
      approvalHandlerConfigured,
      builtinApprovalGateWired,
      policyStrict,
      allControlModulesRequireApproval,
      unprotectedControlSurface,
    };
  }

  static fromRegistry(
    registry: Registry,
    middlewares?: Middleware[] | null,
    acl?: ACL | null,
    config?: Config | null,
    approvalHandler?: ApprovalHandler | null,
    policy?: ExecutionPolicy | null,
  ): Executor {
    return new Executor({ registry, middlewares, acl, config, approvalHandler, policy });
  }

  get registry(): Registry {
    return this._registry;
  }

  get middlewares(): Middleware[] {
    return this._middlewareManager.snapshot();
  }

  /** Set the access control provider. Updates both the executor field and the strategy's ACL step. */
  setAcl(acl: ACL): void {
    this._acl = acl;
    // Located by type, like `_bindProviders()` and `governanceState()`
    // (PROTOCOL_SPEC 6.6.5.2 / 6.6.5.5).
    let found = false;
    for (const step of this._strategy.steps) {
      if (step instanceof BuiltinACLCheck) {
        step.setAcl(acl);
        found = true;
      }
    }
    if (!found) {
      console.warn(
        '[apcore:executor] setAcl() called but current strategy has no BuiltinACLCheck step — ACL will not be enforced',
      );
    }
  }

  /** Set the approval handler. Updates both the executor field and the strategy's approval step. */
  setApprovalHandler(handler: ApprovalHandler): void {
    this._approvalHandler = handler;
    let found = false;
    for (const step of this._strategy.steps) {
      if (step instanceof BuiltinApprovalGate) {
        step.setApprovalHandler(handler);
        found = true;
      }
    }
    if (!found) {
      console.warn(
        '[apcore:executor] setApprovalHandler() called but current strategy has no BuiltinApprovalGate step — approval will not be enforced',
      );
    }
  }

  /**
   * Set the execution-time governance policy for the Step 5 gate (apcore#76).
   *
   * Updates both the executor field and the strategy's `approval_gate` step via
   * its `setPolicy` setter. Custom user-supplied approval steps without that
   * setter are silently skipped — re-register the strategy to replace a custom
   * step's policy.
   */
  setPolicy(policy: ExecutionPolicy | null): void {
    this._policy = policy;
    for (const step of this._strategy.steps) {
      if (step instanceof BuiltinApprovalGate) {
        step.setPolicy(policy);
      }
    }
  }

  use(middleware: Middleware): Executor {
    this._middlewareManager.add(middleware);
    return this;
  }

  useBefore(
    callback: (
      moduleId: string,
      inputs: Record<string, unknown>,
      context: Context,
    ) => Record<string, unknown> | null,
  ): Executor {
    this._middlewareManager.add(new BeforeMiddleware(callback), {
      identityKey: callbackIdentity('before', callback),
    });
    return this;
  }

  useAfter(
    callback: (
      moduleId: string,
      inputs: Record<string, unknown>,
      output: Record<string, unknown>,
      context: Context,
    ) => Record<string, unknown> | null,
  ): Executor {
    this._middlewareManager.add(new AfterMiddleware(callback), {
      identityKey: callbackIdentity('after', callback),
    });
    return this;
  }

  remove(middleware: Middleware): boolean {
    return this._middlewareManager.remove(middleware);
  }

  /**
   * Attach a step middleware that wraps every pipeline step of every call
   * (protocol-spec §5.16 requirement 5; docs/features/middleware-system.md
   * "Pipeline Step Middleware"). `beforeStep` runs in registration order,
   * `afterStep` / `onStepError` in reverse.
   *
   * Step middleware belongs to this executor, not to a strategy, so it also
   * wraps a per-call strategy passed to {@link callWithTrace}.
   */
  addStepMiddleware(middleware: StepMiddleware): Executor {
    this._pipelineEngine.addStepMiddleware(middleware);
    return this;
  }

  /** The step middlewares attached to this executor, in registration order. */
  get stepMiddlewares(): readonly StepMiddleware[] {
    return this._pipelineEngine.stepMiddlewares;
  }

  async call(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    versionHint?: string | null,
  ): Promise<Record<string, unknown>> {
    this._validateModuleId(moduleId);

    // Issue #66: auto-bind executor to context on every entry. withExecutor
    // is idempotent for the same instance and raises ContextBindingError on
    // cross-executor rebind, so the rule "Executor binds itself on first call"
    // is enforced regardless of how the caller built the Context.
    let ctx = context ?? Context.create();
    ctx = bindExecutorToCtx(ctx, this);
    const pipeCtx: PipelineContext = {
      moduleId,
      inputs: inputs ?? {},
      context: ctx,
      module: null,
      validatedInputs: null,
      output: null,
      validatedOutput: null,
      stream: false,
      outputStream: null,
      strategy: this._strategy,
      trace: null,
      versionHint: versionHint ?? null,
    };

    // Loop iterates only when a RetrySignal is returned from middleware
    // onError; every other path returns or throws on the first attempt
    // (sync finding A-D-017).
    try {
      while (true) {
        try {
          const [output, _trace] = await this._pipelineEngine.run(this._strategy, pipeCtx);
          return (output ?? {}) as Record<string, unknown>;
        } catch (exc) {
          if (exc instanceof ExecutionCancelledError) throw exc;
          // PipelineStepError is the engine-level contract (protocol-spec §5.16 requirement 1). Unwrap the cause
          // so the executor's public API surfaces the original typed error.
          const unwrapped =
            exc instanceof PipelineStepError ? (exc.cause instanceof Error ? exc.cause : exc) : exc;
          // D-20: cancellation MUST bypass on_error recovery even when the engine
          // wrapped it in a PipelineStepError. Re-check the unwrapped cause and
          // rethrow before any middleware onError handling runs.
          if (unwrapped instanceof ExecutionCancelledError) throw unwrapped;
          // MiddlewareChainError wraps the original; unwrap it so callers see the
          // real error class/code instead of a generic MODULE_EXECUTE_ERROR.
          const ctxObj = pipeCtx.context;
          const underlying =
            unwrapped instanceof MiddlewareChainError ? unwrapped.original : (unwrapped as Error);
          const wrapped = propagateError(underlying, moduleId, ctxObj);
          const executedMw = pipeCtx.executedMiddlewares;
          if (executedMw && executedMw.length > 0) {
            const recovery = await this._middlewareManager.executeOnError(
              moduleId,
              pipeCtx.inputs,
              wrapped as Error,
              ctxObj,
              executedMw as Middleware[],
            );
            if (recovery instanceof RetrySignal) {
              this._resetPipeCtxForRetry(pipeCtx, recovery.inputs);
              if (pipeCtx.context.cancelToken !== ctx.cancelToken)
                pipeCtx.context.cancelToken?.dispose();
              pipeCtx.context = ctx;
              continue;
            }
            if (recovery !== null) return recovery;
          }
          throw wrapped;
        }
      }
    } finally {
      if (pipeCtx.context.cancelToken !== ctx.cancelToken) pipeCtx.context.cancelToken?.dispose();
    }
  }

  /**
   * Reset PipelineContext for another attempt triggered by a RetrySignal.
   *
   * Preserves the top-level Context (so retry counters in context.data carry
   * across attempts) while clearing per-run fields that the next attempt
   * will re-populate. (sync finding A-D-017)
   */
  private _resetPipeCtxForRetry(
    pipeCtx: PipelineContext,
    newInputs: Record<string, unknown>,
  ): void {
    pipeCtx.inputs = newInputs;
    pipeCtx.validatedInputs = null;
    pipeCtx.module = null;
    pipeCtx.output = null;
    pipeCtx.validatedOutput = null;
    pipeCtx.executedMiddlewares = [];
  }

  /**
   * Alias for call(). Provided for compatibility with MCP bridge packages
   * that may call callAsync() by convention.
   */
  async callAsync(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    versionHint?: string | null,
  ): Promise<Record<string, unknown>> {
    return this.call(moduleId, inputs, context, versionHint);
  }

  /**
   * Execute a module through the pipeline strategy and return both the output
   * and a full execution trace. Requires a strategy to be set (either on the
   * executor or passed via options).
   */
  async callWithTrace(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    options?: { strategy?: ExecutionStrategy | null } | null,
    versionHint?: string | null,
  ): Promise<[Record<string, unknown>, PipelineTrace]> {
    const strategy = options?.strategy ?? this._strategy;
    // A per-call strategy is still a strategy this executor runs: its built-in
    // gates enforce this executor's providers (PROTOCOL_SPEC 6.6.5.5, D-129).
    if (strategy !== this._strategy) this._bindProviders(strategy);

    // Issue #66: auto-bind executor (see call() for rationale).
    let ctx = context ?? Context.create();
    ctx = bindExecutorToCtx(ctx, this);
    const pipelineCtx: PipelineContext = {
      moduleId,
      inputs: inputs ?? {},
      context: ctx,
      module: null,
      validatedInputs: null,
      output: null,
      validatedOutput: null,
      stream: false,
      outputStream: null,
      strategy,
      trace: null,
      // D-19: callWithTrace shares call() semantics modulo return shape, so the
      // version hint must reach the pipeline context (e.g. registry version
      // resolution) identically to call().
      versionHint: versionHint ?? null,
    };

    try {
      const [output, trace] = await this._pipelineEngine.run(strategy, pipelineCtx);
      return [(output ?? {}) as Record<string, unknown>, trace];
    } catch (exc) {
      // Spec D-19/D-20/D-22: `callWithTrace` MUST share `call()`'s error
      // semantics — same on_error chain, same cancellation short-circuit,
      // same MiddlewareChainError unwrap. The trace is the observable record
      // of execution, including any middleware recovery, not a sanitized
      // projection. (Closes A-D-EXEC-004; mirrors Python
      // `call_async_with_trace`.)
      if (exc instanceof ExecutionCancelledError) throw exc;

      const unwrapped =
        exc instanceof PipelineStepError ? (exc.cause instanceof Error ? exc.cause : exc) : exc;
      // D-20: step-wrapped cancellation bypasses on_error here too.
      if (unwrapped instanceof ExecutionCancelledError) throw unwrapped;
      const ctxObj = pipelineCtx.context;
      const underlying =
        unwrapped instanceof MiddlewareChainError ? unwrapped.original : (unwrapped as Error);
      const wrapped = propagateError(underlying, moduleId, ctxObj);

      const executedMw = pipelineCtx.executedMiddlewares;
      if (executedMw && executedMw.length > 0) {
        const recovery = await this._middlewareManager.executeOnError(
          moduleId,
          pipelineCtx.inputs,
          wrapped as Error,
          ctxObj,
          executedMw as Middleware[],
        );
        if (recovery !== null && !(recovery instanceof RetrySignal)) {
          // RetrySignal is intentionally not honoured in the trace variant
          // (D-19 leaves retry-loop semantics to `call()`); a non-null
          // recovery dict counts as a successful middleware repair and is
          // returned alongside the trace already captured in pipelineCtx.
          const trace = (pipelineCtx.trace ?? null) as unknown as PipelineTrace;
          return [recovery, trace];
        }
      }
      throw wrapped;
    } finally {
      if (pipelineCtx.context.cancelToken !== ctx.cancelToken)
        pipelineCtx.context.cancelToken?.dispose();
    }
  }

  /**
   * Streaming execution pipeline. If the module exposes a stream() async generator,
   * yields each chunk. Otherwise falls back to call() and yields a single chunk.
   *
   * Pipeline: context -> safety -> lookup -> ACL -> validate inputs -> before-middleware
   *   -> stream (or fallback to execute) -> validate accumulated output -> after-middleware
   *
   * Note: In the streaming path, after-middleware runs on the accumulated output for
   * validation/side-effects but its return value is not yielded since chunks were already
   * emitted. In the non-streaming fallback, after-middleware can transform the output.
   */
  /**
   * Streaming execution pipeline using split-pipeline design.
   *
   * Phase 1: Pipeline runs all steps with ctx.stream=true. BuiltinExecute detects
   *   stream mode and sets ctx.outputStream if module has stream().
   * Phase 2: Iterate stream, accumulate chunks and yield each.
   * Phase 3: Run output_validation + middleware_after on accumulated output.
   *
   * If the module has no stream(), the pipeline executes normally and yields ctx.output.
   */
  async *stream(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    versionHint?: string | null,
  ): AsyncGenerator<Record<string, unknown>> {
    this._validateModuleId(moduleId);

    // Issue #66: auto-bind executor (see call() for rationale).
    let ctx = context ?? Context.create();
    ctx = bindExecutorToCtx(ctx, this);
    const pipeCtx: PipelineContext = {
      moduleId,
      inputs: inputs ?? {},
      context: ctx,
      module: null,
      validatedInputs: null,
      output: null,
      validatedOutput: null,
      stream: true,
      outputStream: null,
      strategy: this._strategy,
      trace: null,
      versionHint: versionHint ?? null,
    };

    try {
      // Phase 1: Run the full pipeline. BuiltinExecute detects ctx.stream=true.
      try {
        await this._pipelineEngine.run(this._strategy, pipeCtx);
      } catch (exc) {
        if (exc instanceof ExecutionCancelledError) throw exc;
        const ctxObj = pipeCtx.context;
        // Unwrap PipelineStepError to expose the original typed cause (protocol-spec §5.16 requirement 1).
        const unwrapped =
          exc instanceof PipelineStepError ? (exc.cause instanceof Error ? exc.cause : exc) : exc;
        // D-20: a step-wrapped cancellation MUST bypass on_error recovery in
        // stream mode too. Rethrow the unwrapped cancellation before recovery.
        if (unwrapped instanceof ExecutionCancelledError) throw unwrapped;
        const wrapped = propagateError(unwrapped as Error, moduleId, ctxObj);
        if (pipeCtx.executedMiddlewares && pipeCtx.executedMiddlewares.length > 0) {
          const recovery = await this._middlewareManager.executeOnError(
            moduleId,
            pipeCtx.inputs,
            wrapped as Error,
            ctxObj,
            pipeCtx.executedMiddlewares as Middleware[],
          );
          // RetrySignal is not supported in stream mode — re-running a stream
          // mid-flight is not well-defined. Fall through to throwing the wrapped
          // error so the caller sees a normal failure (sync finding A-D-017).
          if (recovery !== null && !(recovery instanceof RetrySignal)) {
            yield recovery;
            return;
          }
        }
        throw wrapped;
      }

      // If no outputStream, pipeline already executed normally — yield single result
      if (pipeCtx.outputStream == null) {
        yield (pipeCtx.output ?? {}) as Record<string, unknown>;
        return;
      }

      // Phase 2: Iterate stream, accumulate chunks
      const outputStream = pipeCtx.outputStream as AsyncGenerator<Record<string, unknown>>;
      const accumulated: Record<string, unknown> = {};
      // PROTOCOL_SPEC §5: resolved ONCE per stream, not per chunk. A
      // configuration mutated mid-stream — `system.control.update_config` can do
      // exactly that — must not change the cap of a stream already in flight.
      // This used to be called inside the chunk loop, which contradicted the
      // guarantee its own commit message stated.
      const mergeDepthCap = resolveMergeDepth(this._config);
      // Read the first-class `Context.globalDeadline` field, which
      // `BuiltinContextCreation` now stamps onto the context derived for this
      // call (spec v1.50.0 D-99 / D-100). The previous private `data` slot is
      // gone: `data` is caller-visible, caller-writable and shared by reference
      // with child contexts, and keeping the budget there left the documented
      // `Context.create` parameter unreadable by the pipeline.
      const globalDeadline = pipeCtx.context?.globalDeadline ?? null;
      let chunkIndex = 0;
      try {
        for await (const chunk of outputStream) {
          // Enforce global_deadline between chunks — matches apcore-python
          // executor.py:872-879 (sync finding A-D-014). D-99: the deadline is
          // epoch SECONDS, so the wall clock is scaled to match rather than the
          // deadline being read as milliseconds.
          if (globalDeadline !== null && Date.now() / 1000 > globalDeadline) {
            pipeCtx.context.cancelToken?.cancel();
            throw new ModuleTimeoutError(moduleId, 0, {
              retryable: isModuleIdempotent(pipeCtx.module),
            });
          }
          // Enforce stream-chunk shape BEFORE merge and BEFORE yield (D10-001 /
          // apcore-rust deep_merge_chunks_checked, D-58). A non-object chunk is
          // rejected so it is never delivered to the consumer; deep_merge can
          // only accumulate objects. The throw routes through the catch below
          // (propagateError / onError) like other stream errors.
          if (!isPlainObjectChunk(chunk)) {
            const actualType = jsonTypeName(chunk);
            throw new InvalidInputError(
              `Streaming chunk at index ${chunkIndex} is not a JSON object ` +
                `(got ${actualType}); chunks must be objects so deep_merge can ` +
                `accumulate them.`,
              {
                details: {
                  code: 'STREAM_CHUNK_NOT_OBJECT',
                  chunk_index: chunkIndex,
                  actual_type: actualType,
                },
              },
            );
          }
          deepMergeChunk(accumulated, chunk, 0, mergeDepthCap);
          yield chunk;
          chunkIndex += 1;
        }
      } catch (exc) {
        if (exc instanceof ExecutionCancelledError) throw exc;
        const ctxObj = pipeCtx.context;
        const wrapped = propagateError(exc as Error, moduleId, ctxObj);
        if (pipeCtx.executedMiddlewares && pipeCtx.executedMiddlewares.length > 0) {
          const recovery = await this._middlewareManager.executeOnError(
            moduleId,
            pipeCtx.inputs,
            wrapped as Error,
            ctxObj,
            pipeCtx.executedMiddlewares as Middleware[],
          );
          // RetrySignal not supported mid-stream (sync finding A-D-017).
          if (recovery !== null && !(recovery instanceof RetrySignal)) {
            yield recovery;
            return;
          }
        }
        throw wrapped;
      }

      // Phase 3: Output validation + middleware_after on accumulated result
      pipeCtx.output = accumulated;
      const postSteps = this._strategy.steps.filter(
        (s) =>
          s.name === 'output_validation' ||
          s.name === 'middleware_after' ||
          s.name === 'return_result',
      );
      if (postSteps.length > 0) {
        // The post-stream sub-strategy starts with module + output already
        // populated on pipeCtx (set by Phase 1 / Phase 2 above), so seed those
        // names into the dependency check (§2.1).
        const postStrategy = new ExecutionStrategy('post_stream', postSteps, {
          seedProvides: ['module', 'output'],
        });
        try {
          await this._pipelineEngine.run(postStrategy, pipeCtx);
        } catch (exc) {
          // Chunks are already delivered to the caller and cannot be recalled.
          // Swallow the phase-3 error and log a warning — matches apcore-python
          // executor.py which emits an ApCoreEvent("apcore.stream.post_validation_failed")
          // and does NOT re-raise (sync finding A-D-012 / A-D-006).
          //
          // STR-4 / CAN-002: cancellation is the ONE exception (D-20 — a
          // cancellation must never be swallowed), and the check has to run on
          // the UNWRAPPED error. The engine wraps every step failure in
          // `PipelineStepError` (pipeline.ts:1109), so an
          // `ExecutionCancelledError` raised inside a phase-3 step arrived here
          // already wrapped and the guard — which used to sit above this unwrap
          // — never matched. The one error that must always reach the caller was
          // being swallowed with all the others, invisibly, because the guard
          // that says otherwise was still right there in the source.
          const ctxObj = pipeCtx.context;
          const unwrappedPost =
            exc instanceof PipelineStepError ? (exc.cause instanceof Error ? exc.cause : exc) : exc;
          if (unwrappedPost instanceof ExecutionCancelledError) throw unwrappedPost;
          const wrapped = propagateError(unwrappedPost as Error, moduleId, ctxObj);
          // A-D-006: when an EventEmitter is wired, emit the post-validation-failed
          // event so observers/trace exporters surface the failure (mirrors
          // apcore-python executor.py:1096).
          if (this._eventEmitter !== null) {
            const cause = unwrappedPost instanceof Error ? unwrappedPost : (wrapped as Error);
            this._eventEmitter.emit(
              createEvent('apcore.stream.post_validation_failed', moduleId, 'error', {
                error_type: cause.constructor?.name ?? 'Error',
                message: cause.message,
                trace_id: ctxObj?.traceId ?? null,
              }),
            );
          }
          // Sync finding A-D-011: phase-3 errors must NOT invoke middleware
          // `on_error` once chunks have already been yielded. The middleware
          // recovery contract is "produce a recovery output before any output is
          // visible"; running on_error after partial-output emission breaks that
          // contract. apcore-python and apcore-rust only log/emit at this point;
          // TS now matches.
          console.warn(
            `[apcore:executor] stream phase-3 failure for '${moduleId}' (chunks already delivered): ${wrapped.message}`,
          );
          // Do not rethrow — phase-3 errors are swallowed per spec.
        }
      }
    } finally {
      if (pipeCtx.context.cancelToken !== ctx.cancelToken) pipeCtx.context.cancelToken?.dispose();
    }
  }

  /**
   * Non-destructive preflight check using pipeline dry_run mode.
   *
   * Runs all pure steps (context creation, call chain guard, module lookup,
   * ACL, input validation). Steps with pure=false (approval, middleware,
   * execute) are automatically skipped. Returns a PreflightResult.
   */
  async validate(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
  ): Promise<PreflightResult> {
    const effectiveInputs = inputs ?? {};
    const checks: PreflightCheckResult[] = [];

    // Check 0: module_id format (before pipeline)
    try {
      this._validateModuleId(moduleId);
      checks.push({ check: 'module_id', passed: true });
    } catch (e) {
      checks.push({
        check: 'module_id',
        passed: false,
        error: (e as InvalidInputError).toJSON(),
      });
      return createPreflightResult(checks);
    }

    // Run pipeline in dry_run mode — pure=false steps are skipped.
    // Issue #66: auto-bind executor on every entry (see call() for rationale).
    // A-D-014: validate() MUST be non-throwing. A Context already bound to a
    // DIFFERENT executor raises ContextBindingError on bind; surface that as a
    // failed `executor_binding` check rather than letting it escape (mirrors
    // apcore-python executor.py and apcore-rust executor.rs).
    let validateCtx = context ?? Context.create();
    try {
      validateCtx = bindExecutorToCtx(validateCtx, this);
    } catch (e) {
      if (e instanceof ContextBindingError) {
        checks.push({
          check: 'executor_binding',
          passed: false,
          error: { code: e.code, message: e.message },
        });
        return createPreflightResult(checks);
      }
      throw e;
    }
    const pipeCtx: PipelineContext = {
      moduleId,
      inputs: effectiveInputs,
      context: validateCtx,
      module: null,
      validatedInputs: null,
      output: null,
      validatedOutput: null,
      stream: false,
      outputStream: null,
      strategy: this._strategy,
      trace: null,
      dryRun: true,
    };

    try {
      let trace: PipelineTrace | null = null;
      try {
        const [, t] = await this._pipelineEngine.run(this._strategy, pipeCtx);
        trace = t;
      } catch (e) {
        // Step raised a domain error (ModuleNotFoundError, ACLDeniedError, etc.)
        if (e instanceof PipelineAbortError) {
          trace = e.pipelineTrace;
        } else {
          // Unwrap PipelineStepError to expose the original typed cause (protocol-spec §5.16 requirement 1).
          const underlying =
            e instanceof PipelineStepError ? (e.cause instanceof Error ? e.cause : e) : e;
          const errorDict =
            underlying instanceof ModuleError
              ? { code: underlying.code, message: underlying.message }
              : {
                  code: (underlying as Error).constructor?.name ?? 'Error',
                  message: String(underlying),
                };
          const code =
            underlying instanceof ModuleError
              ? underlying.code
              : ((underlying as Error).constructor?.name ?? 'Error');

          let checkName: string;
          if (code === 'MODULE_NOT_FOUND') checkName = 'module_lookup';
          else if (code === 'ACL_DENIED') checkName = Executor.ACL_CHECK;
          else if (code === 'SCHEMA_VALIDATION_ERROR' || code === 'INVALID_INPUT')
            checkName = 'schema';
          else if (
            code === 'CALL_DEPTH_EXCEEDED' ||
            code === 'CIRCULAR_CALL' ||
            code === 'CALL_FREQUENCY_EXCEEDED'
          )
            checkName = 'call_chain';
          else checkName = 'unknown';

          checks.push({ check: checkName, passed: false, error: errorDict });
        }
      }

      // Convert pipeline trace to PreflightResult checks
      if (trace !== null) {
        checks.push(...this._traceToChecks(trace));
      }

      // Detect requires_approval
      let requiresApproval = false;
      if (pipeCtx.module != null) {
        const mod = pipeCtx.module as Record<string, unknown>;
        // §7.9.5 binds this to the verdict the Step-5 gate will enforce, so it
        // MUST read the same governance source: the D-96 union of the live
        // instance and the registry's DECLARED annotations. Reading the instance
        // alone reported "no approval needed" for a requirement an operator
        // declared in a `*_meta.yaml` / `metadata` source — and the gate, now
        // reading the union, would stop the call the preflight waved through.
        // That disagreement is the one thing this method exists to prevent.
        const governance = governanceUnion(
          mod['annotations'],
          this._registry.getDeclaredAnnotations(moduleId),
        );
        if (this._policy !== null) {
          // Policy overrides win over declared annotations (apcore#76), so
          // preflight reports the same verdict the gate will enforce.
          // §7.9.6 rule 5: `_approval_token` is a protocol-level key, not caller
          // input, and MUST be stripped BEFORE policy resolution — §7.4's
          // "before passing to subsequent steps" does not reach a decision made
          // inside Step 5. Leaving it in place puts a token into the audit trail
          // and the apcore.policy.override payload.
          const { _approval_token: _policyToken, ...policyArguments } = effectiveInputs;
          requiresApproval = this._policy.resolve(moduleId, governance, {
            // §7.9.6: preflight resolves against the same call site the gate
            // will see, so the reported verdict matches the enforced one.
            arguments: policyArguments,
            context: validateCtx,
          }).needsApproval;
        } else {
          requiresApproval = Boolean(governance?.requiresApproval);
        }
      }
      // §7.9.5 — report the GOVERNANCE-effective requirement, the union of §6.9
      // rows 3–5, which since v1.28.0 includes an ACL rule carrying `approval`
      // (§6.1.6). Reporting only the policy-effective value would tell a caller
      // no approval is needed for a call the gate will stop. The ACL check runs
      // in the dry-run pipeline above (it is `pure`), so its verdict is already
      // on `pipeCtx` — and §6.9 row 4 forbids a policy from clearing it, which is
      // why this is OR-ed on top rather than folded into the branch above.
      if (pipeCtx.aclApprovalRequired === true) {
        requiresApproval = true;
      }

      // Module-level introspection is gated on TWO conditions, not one.
      //
      // 1. Module lookup succeeded (`pipeCtx.module` is not null).
      // 2. The ACL did not deny the call — PROTOCOL_SPEC §12.8.5.1.
      //
      // Condition 2 is the security half. `preflight()` and `preview()` are
      // module-authored code, and what they return names what the call would do:
      // the resolved binary and argv of a command-wrapping module, the target of
      // a write. Module lookup is Step 3 and the ACL check is Step 4, so gating
      // on lookup alone runs module code for a caller the ACL just denied and
      // hands back what it said.
      //
      // Scoped to authorization deliberately: a failed `schema` check does NOT
      // suppress introspection, because a caller the ACL permits is entitled to
      // the module's account of what would happen even when its inputs are
      // malformed.
      const aclDenied = checks.some((c) => c.check === Executor.ACL_CHECK && !c.passed);

      // Module-level preflight (optional)
      if (!aclDenied && pipeCtx.module != null) {
        const mod = pipeCtx.module as Record<string, unknown>;
        const modWithPreflight = mod as { preflight?: Module['preflight'] };
        if (typeof modWithPreflight.preflight === 'function') {
          try {
            const preflightWarnings = modWithPreflight.preflight(effectiveInputs, pipeCtx.context);
            if (Array.isArray(preflightWarnings) && preflightWarnings.length > 0) {
              checks.push({
                check: Executor.MODULE_PREFLIGHT_CHECK,
                passed: true,
                warnings: preflightWarnings,
              });
            } else {
              checks.push({ check: Executor.MODULE_PREFLIGHT_CHECK, passed: true });
            }
          } catch (exc: unknown) {
            const excName = exc instanceof Error ? exc.constructor.name : 'Error';
            const excMsg = exc instanceof Error ? exc.message : String(exc);
            checks.push({
              check: Executor.MODULE_PREFLIGHT_CHECK,
              passed: true,
              warnings: [`preflight() raised ${excName}: ${excMsg}`],
            });
          }
        }
      }

      // Module-level preview() (optional, protocol-spec §5.6 / §12.8.5.1).
      // Invoked only after the standard validation pipeline has been processed.
      // Returning null is equivalent to omitting the method. Exceptions (sync
      // throws or async rejections) are treated as advisory warnings and do NOT
      // fail validation, mirroring `preflight()` semantics (RFC Open Question 1).
      let predictedChanges: Change[] = [];
      if (!aclDenied && pipeCtx.module != null) {
        const mod = pipeCtx.module as Record<string, unknown>;
        const modWithPreview = mod as { preview?: Module['preview'] };
        if (typeof modWithPreview.preview === 'function') {
          try {
            const raw = modWithPreview.preview(effectiveInputs, pipeCtx.context);
            // Support both sync and async preview() implementations.
            const result: PreviewResult | null =
              raw != null && typeof (raw as Promise<unknown>).then === 'function'
                ? await (raw as Promise<PreviewResult | null>)
                : (raw as PreviewResult | null);
            if (result != null) {
              predictedChanges = Array.isArray(result.changes) ? result.changes : [];
              checks.push({ check: Executor.MODULE_PREVIEW_CHECK, passed: true });
            }
          } catch (exc: unknown) {
            const excName = exc instanceof Error ? exc.constructor.name : 'Error';
            const excMsg = exc instanceof Error ? exc.message : String(exc);
            checks.push({
              check: Executor.MODULE_PREVIEW_CHECK,
              passed: true,
              warnings: [`preview() raised ${excName}: ${excMsg}`],
            });
          }
        }
      }

      return createPreflightResult(checks, requiresApproval, predictedChanges);
    } finally {
      if (pipeCtx.context.cancelToken !== validateCtx.cancelToken)
        pipeCtx.context.cancelToken?.dispose();
    }
  }

  /**
   * Canonical PreflightResult check names (PROTOCOL_SPEC §12.8.4 enum). Named
   * rather than spelled inline because §12.8.5.1 makes `acl` the switch that
   * withholds the other two — a rename that silently stopped matching would
   * turn the disclosure gate off. apcore-rust exports the same three as public
   * constants (`apcore::ACL_CHECK_NAME` and friends).
   */
  private static readonly ACL_CHECK = 'acl';
  private static readonly MODULE_PREFLIGHT_CHECK = 'module_preflight';
  private static readonly MODULE_PREVIEW_CHECK = 'module_preview';

  /** Map pipeline step names to PreflightResult check names. */
  private static readonly _STEP_TO_CHECK: Record<string, string> = {
    context_creation: 'context',
    call_chain_guard: 'call_chain',
    module_lookup: 'module_lookup',
    acl_check: Executor.ACL_CHECK,
    approval_gate: 'approval',
    middleware_before: 'middleware',
    input_validation: 'schema',
  };

  /** Convert PipelineTrace steps to PreflightCheckResult list. */
  private _traceToChecks(trace: PipelineTrace): PreflightCheckResult[] {
    const checks: PreflightCheckResult[] = [];
    for (const st of trace.steps) {
      if (st.skipped) continue;
      const checkName = Executor._STEP_TO_CHECK[st.name] ?? st.name;
      const passed = st.result.action !== 'abort';
      let error: Record<string, unknown> | undefined;
      if (!passed && st.result.explanation) {
        error = st.error ?? {
          code: `STEP_${st.name.toUpperCase()}_FAILED`,
          message: st.result.explanation,
        };
      }
      checks.push({ check: checkName, passed, error });
    }
    return checks;
  }

  /** Validate module_id format at public entry points. */
  private _validateModuleId(moduleId: string): void {
    if (!moduleId || !MODULE_ID_PATTERN.test(moduleId)) {
      throw new InvalidInputError(
        `Invalid module ID: '${moduleId}'. Must match pattern: ${MODULE_ID_PATTERN.source}`,
        undefined,
        ErrorCodes.INVALID_MODULE_ID,
      );
    }
    // The length bound is part of this entry guard, not only of the registry:
    // core-executor.md "Contract: Executor.call" requires empty / over-length /
    // malformed IDs to be rejected *before* the PipelineContext is constructed.
    // Enforcing it in the registry alone let a 300-character well-formed ID
    // build a context and come back as MODULE_NOT_FOUND (spec v1.49.0, D-75).
    if (moduleId.length > MAX_MODULE_ID_LENGTH) {
      throw new InvalidInputError(
        `Module ID exceeds maximum length of ${MAX_MODULE_ID_LENGTH}: ${moduleId.length}`,
        undefined,
        ErrorCodes.INVALID_MODULE_ID,
      );
    }
  }

  /** Check if a module requires approval, handling both interface and dict annotations. */
  private _needsApproval(mod: Record<string, unknown>): boolean {
    const annotations = mod['annotations'];
    if (annotations == null) return false;
    if (typeof annotations !== 'object') return false;
    if ('requiresApproval' in annotations) {
      return Boolean((annotations as ModuleAnnotations).requiresApproval);
    }
    if ('requires_approval' in annotations) {
      return Boolean((annotations as Record<string, unknown>)['requires_approval']);
    }
    return false;
  }
}
