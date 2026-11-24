/**
 * Pipeline YAML configuration: step type registry and strategy builder.
 */

import { buildStandardStrategy } from './builtin-steps.js';
import type { StandardStrategyDeps } from './builtin-steps.js';
import {
  ConfigurationError,
  gateWeakeningError,
  gateWeakeningFields,
  governanceGateOf,
} from './pipeline.js';
import type { ExecutionStrategy, PipelineContext, Step, StepResult } from './pipeline.js';

// `ConfigurationError` lives in `pipeline.ts` so `ExecutionStrategy` can raise
// it for a weakened governance gate (D-130); re-exported here, where it has
// always been public.
export { ConfigurationError } from './pipeline.js';

// ---------------------------------------------------------------------------
// Global step type registry
// ---------------------------------------------------------------------------

type StepFactory = (config: Record<string, unknown>) => Step;

/** Global step type registry: name -> factory function. */
const _stepTypeRegistry = new Map<string, StepFactory>();

/**
 * Register a step type for YAML pipeline configuration.
 *
 * @param name - Type name referenced in YAML `type` field.
 *   Must be non-empty, no whitespace, unique.
 * @param factory - A callable `(config) => Step`.
 * @throws If name is empty, contains whitespace, or is already registered.
 */
export function registerStepType(name: string, factory: StepFactory): void {
  if (!name || /\s/.test(name)) {
    throw new Error(`Invalid step type name: '${name}'`);
  }
  if (_stepTypeRegistry.has(name)) {
    throw new Error(`Step type '${name}' is already registered`);
  }
  _stepTypeRegistry.set(name, factory);
}

/**
 * Remove a registered step type.
 * @returns True if found and removed.
 */
export function unregisterStepType(name: string): boolean {
  return _stepTypeRegistry.delete(name);
}

/**
 * Return a list of all registered step type names.
 */
export function registeredStepTypes(): string[] {
  return [..._stepTypeRegistry.keys()];
}

// ---------------------------------------------------------------------------
// Step resolution
// ---------------------------------------------------------------------------

interface StepDefinition {
  name?: string;
  type?: string;
  handler?: string;
  config?: Record<string, unknown>;
  matchModules?: string[];
  ignoreErrors?: boolean;
  pure?: boolean;
  timeoutMs?: number;
  after?: string;
  before?: string;
}

/**
 * The keys a `pipeline.steps` entry may carry, keyed by the CANONICAL YAML
 * spelling and mapped to the {@link StepDefinition} property name.
 *
 * `schemas/apcore-config.schema.json` `$defs/PipelineStep` declares exactly
 * these ten and is `additionalProperties: false`; protocol-spec §5.16.1 is
 * the same table in words. Nothing enforced that closedness until
 * apcore#89, and the consequence was the failure mode this whole cycle is
 * about — measured on this SDK, a `steps` entry
 * `{name, type, after, tiemout_ms: 5000}` built successfully with
 * `timeoutMs === 0`: the operator's five-second timeout had no effect and
 * nothing said so.
 *
 * The `configure:` path was fixed for this in an earlier pass; the `steps:`
 * path was not, and it was worse. `StepDefinition` is camelCase, `_resolveStep`
 * and `ConfiguredStep` read camelCase, and the YAML is snake_case — so on a
 * `steps:` entry the CANONICAL spellings were dropped too, not merely the
 * typos: `timeout_ms: 5000` produced `timeoutMs = 0`, `ignore_errors: true`
 * produced `false`, `match_modules` produced `null`. Only `pure` worked,
 * because it is spelled the same in both. Closing the set without also
 * accepting the canonical spellings would have certified those three keys as
 * valid while they continued to do nothing, which is the same defect with a
 * nicer error message.
 *
 * As with `CONFIGURABLE_STEP_FIELDS`, the camelCase spellings are accepted as
 * this SDK's own idiomatic aliases (§4.2 permits that at an API boundary); a
 * configuration *file* carries the canonical spelling only.
 */
const STEP_ENTRY_FIELDS: ReadonlyMap<string, keyof StepDefinition> = new Map([
  // canonical snake_case ($defs/PipelineStep + YAML + fixtures)
  ['name', 'name'],
  ['type', 'type'],
  ['handler', 'handler'],
  ['config', 'config'],
  ['match_modules', 'matchModules'],
  ['ignore_errors', 'ignoreErrors'],
  ['pure', 'pure'],
  ['timeout_ms', 'timeoutMs'],
  ['after', 'after'],
  ['before', 'before'],
  // camelCase aliases (the TypeScript `StepDefinition` surface)
  ['matchModules', 'matchModules'],
  ['ignoreErrors', 'ignoreErrors'],
  ['timeoutMs', 'timeoutMs'],
] as ReadonlyArray<[string, keyof StepDefinition]>);

/** The canonical (snake_case) spellings, for error messages. */
const CANONICAL_STEP_ENTRY_FIELDS: readonly string[] = [
  'name',
  'type',
  'handler',
  'config',
  'match_modules',
  'ignore_errors',
  'pure',
  'timeout_ms',
  'after',
  'before',
];

/**
 * Validate one `pipeline.steps` entry against the closed key set and return it
 * with every key spelled the way {@link StepDefinition} spells it.
 *
 * @throws ConfigurationError - on any key outside `$defs/PipelineStep`.
 *   Parse time, by construction: this runs before the step is resolved, so no
 *   factory has been called and nothing has been inserted into the strategy.
 */
function _normalizeStepDefinition(entry: Record<string, unknown>, index: number): StepDefinition {
  const normalized: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(entry)) {
    const property = STEP_ENTRY_FIELDS.get(key);
    if (property === undefined) {
      const label =
        typeof entry['name'] === 'string' ? `'${entry['name']}'` : `at pipeline.steps[${index}]`;
      throw new ConfigurationError(
        `Pipeline step ${label} has no field '${key}'. ` +
          `Valid fields are: ${CANONICAL_STEP_ENTRY_FIELDS.join(', ')} ` +
          `(the camelCase spellings ` +
          `${[...STEP_ENTRY_FIELDS.keys()]
            .filter((k) => !CANONICAL_STEP_ENTRY_FIELDS.includes(k))
            .join(', ')} are also accepted). ` +
          `schemas/apcore-config.schema.json $defs/PipelineStep is ` +
          `additionalProperties: false — a key outside that set is a typo or a ` +
          `field this SDK does not implement, and either way it would be dropped ` +
          `silently rather than take effect.`,
      );
    }
    normalized[property] = value;
  }
  return normalized as StepDefinition;
}

/** Wraps a resolved step with optional metadata overrides from YAML config. */
class ConfiguredStep implements Step {
  readonly description: string;
  readonly removable: boolean;
  readonly replaceable: boolean;

  name: string;
  matchModules?: string[] | null;
  ignoreErrors?: boolean;
  pure?: boolean;
  timeoutMs?: number;

  private _inner: Step;

  constructor(inner: Step, overrides: Partial<StepDefinition>) {
    this._inner = inner;
    this.name = overrides.name ?? inner.name;
    this.description = inner.description;
    this.removable = inner.removable;
    this.replaceable = inner.replaceable;
    this.matchModules = overrides.matchModules ?? inner.matchModules ?? null;
    this.ignoreErrors = overrides.ignoreErrors ?? inner.ignoreErrors ?? false;
    this.pure = overrides.pure ?? inner.pure ?? false;
    this.timeoutMs = overrides.timeoutMs ?? inner.timeoutMs ?? 0;
  }

  execute(ctx: PipelineContext): Promise<StepResult> {
    return this._inner.execute(ctx);
  }
}

/**
 * Resolve a step definition dict into a Step instance.
 *
 * Resolution order (protocol-spec §5.16.1):
 *   1. `type` field -> look up in registry (sync, fast path)
 *   2. `handler` field -> dynamic ESM import via `await import()`
 *      Format: `"module:exportName"`. The resolved export is invoked as
 *      `factory(config)` — wrap classes in a factory if needed.
 *   3. Neither -> throw Error
 *
 * NOTE: this function is async because handler resolution requires
 * `await import()`. Type-registry lookups still resolve synchronously
 * inside this async wrapper.
 */
export async function _resolveStep(stepDef: StepDefinition): Promise<Step> {
  const typeName = stepDef.type;
  const handlerPath = stepDef.handler;
  const config = stepDef.config ?? {};

  // (1) Try type registry
  if (typeName && _stepTypeRegistry.has(typeName)) {
    const factory = _stepTypeRegistry.get(typeName)!;
    const step = factory(config);
    return new ConfiguredStep(step, stepDef);
  }

  // (2) Handler path -- dynamic ESM import
  if (handlerPath) {
    const step = await _importStep(handlerPath, config);
    return new ConfiguredStep(step, stepDef);
  }

  // (3) Neither
  if (typeName) {
    throw new Error(
      `Step type '${typeName}' not registered. ` +
        `Register with: registerStepType('${typeName}', yourFactory)`,
    );
  }
  throw new Error(`Step '${stepDef.name ?? ''}' has neither 'type' nor 'handler'`);
}

/**
 * Dynamically import a Step factory from a `"module:exportName"` handler path.
 *
 * Mirrors `bindings.ts#resolveTarget` security model: rejects path-traversal
 * (`..`) segments and `file:` URLs at parse time. The resolved export must be
 * a callable `(config) => Step`. Classes should be wrapped in a thin factory.
 */
async function _importStep(handlerPath: string, config: Record<string, unknown>): Promise<Step> {
  // Security checks run on the whole path BEFORE the module:export split,
  // because 'file:' URLs contain a colon that would otherwise be misparsed.
  const FORBIDDEN_SCHEMES = ['file:', 'http:', 'https:', 'ftp:', 'data:', 'blob:'];
  for (const scheme of FORBIDDEN_SCHEMES) {
    if (handlerPath.startsWith(scheme)) {
      throw new Error(`Handler path '${handlerPath}' must not use '${scheme}' URLs.`);
    }
  }
  if (handlerPath.includes('..')) {
    throw new Error(`Handler path '${handlerPath}' must not contain '..' segments.`);
  }
  if (!handlerPath.includes(':')) {
    throw new Error(`Invalid handler path '${handlerPath}'. Expected format: 'module:exportName'.`);
  }

  // Split from the right so module specifiers containing ':' (e.g., URL-like)
  // don't get misparsed. Standard forms ('./mod:fn', '@scope/pkg:fn') split fine
  // either way; rsplit defends against future scheme-like specifiers.
  const lastColon = handlerPath.lastIndexOf(':');
  const modulePath = handlerPath.slice(0, lastColon);
  const exportName = handlerPath.slice(lastColon + 1);

  let mod: Record<string, unknown>;
  try {
    mod = await import(modulePath);
  } catch (e) {
    throw new Error(`Cannot import handler module '${modulePath}': ${(e as Error).message}`);
  }

  const resolved = mod[exportName];
  if (resolved == null) {
    throw new Error(`Export '${exportName}' not found in module '${modulePath}'.`);
  }
  if (typeof resolved !== 'function') {
    throw new Error(
      `Handler '${handlerPath}' resolved to a non-callable. ` +
        `Expected a (config) => Step factory; wrap classes in a factory if needed.`,
    );
  }

  // Distinguish class export (has a prototype with a `constructor` pointing
  // back to itself) from factory function. The previous implementation used
  // try/new-then-fallback-to-call, which silently swallowed legitimate
  // constructor errors (bad config, missing required field) and produced
  // misleading secondary failures when the fallback call was invoked on a
  // class that required `new`.
  const isLikelyClass =
    typeof resolved === 'function' &&
    resolved.prototype != null &&
    typeof resolved.prototype === 'object' &&
    resolved.prototype.constructor === resolved &&
    // Arrow functions have no prototype; factories with .prototype typically
    // have the default Object constructor, so also require the function's
    // name to start uppercase (class convention).
    /^[A-Z]/.test(resolved.name ?? '');

  const step: unknown = isLikelyClass
    ? new (resolved as new (cfg: Record<string, unknown>) => Step)(config)
    : (resolved as (cfg: Record<string, unknown>) => Step)(config);
  return step as Step;
}

// ---------------------------------------------------------------------------
// Strategy builder from YAML config
// ---------------------------------------------------------------------------

interface PipelineConfig {
  remove?: string[];
  configure?: Record<string, Record<string, unknown>>;
  /**
   * Custom step insertions, as they arrive from YAML.
   *
   * Typed open rather than as `StepDefinition[]` because the canonical wire
   * spelling is snake_case (`$defs/PipelineStep`) while `StepDefinition` is
   * camelCase: the narrow type made TypeScript's excess-property check reject
   * the very spelling an `apcore.yaml` carries, so every call site writing
   * canonical YAML needed an `as never`. Key validation moved to
   * `_normalizeStepDefinition`, which runs at parse time, names the offending
   * key and covers the untyped case a compile-time check never could.
   */
  steps?: Array<Record<string, unknown>>;
}

/**
 * The `Step` fields that `pipeline.configure` may override, keyed by the
 * CANONICAL YAML spelling and mapped to the TypeScript property name.
 *
 * The configurable set is EXACTLY FOUR: `match_modules`, `ignore_errors`,
 * `pure`, `timeout_ms` — `schemas/apcore-config.schema.json`
 * `$defs/ConfigurableStepFields` (`additionalProperties: false`) and
 * `docs/spec/protocol-spec.md` §5.16.1. They are the step-entry fields
 * that mean something applied to a step that already exists; the rest are
 * structural (`name`, `type`, `handler`, `after`, `before`) or constructor
 * arguments (`config`). Any other key MUST raise
 * `PIPELINE_CONFIGURATION_ERROR` at parse time.
 *
 * Two spellings are accepted for every field. The schema, every `apcore.yaml`
 * and every cross-language conformance fixture are snake_case
 * (`match_modules`, `ignore_errors`, `timeout_ms`), while the TypeScript
 * {@link Step} interface is camelCase. Accepting only camelCase made three of
 * the four fields unreachable from real YAML: the canonical
 *
 * ```yaml
 * pipeline:
 *   configure:
 *     input_validation:
 *       ignore_errors: true
 * ```
 *
 * threw `ConfigurationError` with a message that listed `ignoreErrors` as
 * valid. Normalising here rather than in the caller is deliberate — a
 * translation table living in a test (or in each downstream framework
 * integration) is the symptom, not the fix.
 *
 * NOT configurable, and narrower than the previous `key in step` test on
 * purpose:
 *
 *  - `requires` / `provides` — a step's capability contract, declared by its
 *    implementation. These were accepted here until apcore#89, and accepting
 *    them let a configuration file rewrite a built-in step's dependencies.
 *    Measured on this SDK: `configure: {input_validation: {requires:
 *    ["context"]}}` overwrites the class field `requires = ["module"]` as an
 *    own property, and `ExecutionStrategy._validateDependencies` then reads
 *    the rewritten value — a `[context_creation, input_validation]` strategy
 *    that threw `PipelineDependencyError` on the built-in contract constructs
 *    cleanly afterwards. The MUST in `docs/features/middleware-system.md`
 *    § Configuration safety can never fire for that step again. The
 *    documented way to exercise the dependency contract was the way to
 *    disable it.
 *  - `name` — the strategy keys an O(1) `name → index` map on it; renaming a
 *    step through YAML silently corrupts `findStepIndex`, `remove` and every
 *    `after`/`before` anchor.
 *  - `removable` / `replaceable` — a step's own guards against being removed or
 *    swapped. Letting a config file flip them defeats the guard.
 *  - `description` — not in `$defs/ConfigurableStepFields`; carries no
 *    behaviour.
 *  - `execute` — `key in step` walks the prototype chain, so the previous
 *    implementation accepted `execute` and let a config file replace the step
 *    body outright.
 */
const CONFIGURABLE_STEP_FIELDS: ReadonlyMap<string, string> = new Map([
  // canonical snake_case (schema + YAML + fixtures)
  ['match_modules', 'matchModules'],
  ['ignore_errors', 'ignoreErrors'],
  ['pure', 'pure'],
  ['timeout_ms', 'timeoutMs'],
  // camelCase aliases (the TypeScript `Step` surface)
  ['matchModules', 'matchModules'],
  ['ignoreErrors', 'ignoreErrors'],
  ['timeoutMs', 'timeoutMs'],
]);

/** The canonical (snake_case) spellings, for error messages. */
const CANONICAL_CONFIGURABLE_STEP_FIELDS: readonly string[] = [
  'match_modules',
  'ignore_errors',
  'pure',
  'timeout_ms',
];

/**
 * Map a `pipeline.configure` field key to the `Step` property it sets, or
 * `undefined` when the key is not configurable.
 */
function resolveConfigurableStepField(key: string): string | undefined {
  return CONFIGURABLE_STEP_FIELDS.get(key);
}


/**
 * Apply the `validation.pipeline.*` limits (PROTOCOL_SPEC §9.1.2).
 *
 * **Both are unconstrained by default.** apcore does not impose limits on the
 * content its users author; it offers them, and an operator opts in. With no
 * configuration, or with the keys left at their defaults, this checks nothing
 * and a pipeline that parsed before this function existed still parses.
 *
 * `remove:` is deliberately not name-checked: those names identify steps that
 * already exist rather than naming new ones, so a limit there would reject a
 * request to remove a step the operator did not author.
 */
function validatePipelineLimits(
  pipelineConfig: PipelineConfig,
  config: { get(key: string, defaultValue?: unknown): unknown } | null,
): void {
  if (config == null) return;
  const intLimit = (v: unknown): number | null =>
    typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : null;
  const maxName = intLimit(config.get('validation.pipeline.step_name_max_length'));
  const maxTimeout = intLimit(config.get('validation.pipeline.timeout_ms_max'));
  if (maxName === null && maxTimeout === null) return;

  const checkTimeout = (where: string, value: unknown): void => {
    if (maxTimeout === null || typeof value !== 'number' || !Number.isInteger(value)) return;
    if (value > maxTimeout) {
      throw new ConfigurationError(
        `${where}: timeout_ms ${value} is over the ${maxTimeout} configured by validation.pipeline.timeout_ms_max`,
      );
    }
  };

  for (const [stepName, overrides] of Object.entries(pipelineConfig.configure ?? {})) {
    if (overrides != null && typeof overrides === 'object') {
      checkTimeout(`Step '${stepName}'`, (overrides as Record<string, unknown>)['timeout_ms']);
    }
  }

  for (const stepDef of pipelineConfig.steps ?? []) {
    const raw = stepDef as unknown as Record<string, unknown>;
    const name = raw['name'];
    if (maxName !== null && typeof name === 'string' && [...name].length > maxName) {
      throw new ConfigurationError(
        `Step '${name}': name is ${[...name].length} characters, over the ${maxName} configured by validation.pipeline.step_name_max_length`,
      );
    }
    checkTimeout(`Step '${typeof name === 'string' ? name : '<unnamed>'}'`, raw['timeout_ms']);
  }
}

/**
 * Build an ExecutionStrategy from YAML pipeline configuration.
 *
 * Starts with `buildStandardStrategy()`, then applies:
 *   1. `remove` -- remove named steps
 *   2. `configure` -- update existing step fields
 *   3. `steps` -- resolve and insert custom steps
 *
 * @param pipelineConfig - The `pipeline` section from apcore.yaml.
 * @param deps - Forwarded to `buildStandardStrategy()`.
 * @returns Configured ExecutionStrategy.
 */
export async function buildStrategyFromConfig(
  pipelineConfig: PipelineConfig,
  deps: StandardStrategyDeps,
): Promise<ExecutionStrategy> {
  return applyPipelineConfig(pipelineConfig, deps, _resolveStep);
}

/**
 * Resolve a step WITHOUT `await import()`, or report that it cannot be.
 *
 * `handler:` targets are ESM specifiers, so this SDK — alone of the three —
 * needs `await` to build them: apcore-python imports synchronously and
 * apcore-rust looks the target up in a handler map. The synchronous path
 * therefore returns `null` for a `handler:` entry rather than throwing, and
 * the caller reports it. Skipping it silently is what §5.16 requirement 6
 * exists to stop; failing construction outright would take a project that
 * starts today and stop it from starting, over a step that has never once run.
 */
function _resolveStepSyncOrNull(stepDef: StepDefinition): Step | null {
  if (stepDef.type && _stepTypeRegistry.has(stepDef.type)) {
    return _resolveStepSync(stepDef);
  }
  return stepDef.handler ? null : _resolveStepSync(stepDef);
}

function _resolveStepSync(stepDef: StepDefinition): Step {
  const typeName = stepDef.type;
  const config = stepDef.config ?? {};
  if (typeName && _stepTypeRegistry.has(typeName)) {
    const factory = _stepTypeRegistry.get(typeName)!;
    return new ConfiguredStep(factory(config), stepDef);
  }
  throw new ConfigurationError(
    `Pipeline step at index has neither a registered 'type' nor a 'handler'. ` +
      `Register the type with registerStepType(), or give the step a handler.`,
  );
}

/**
 * PROTOCOL_SPEC §5.16 requirement 6, synchronously.
 *
 * The `Executor` constructor is synchronous and `_strategy` is read by
 * synchronous accessors (`info()`, `steps`), so a promise held for later would
 * leave those reporting the DEFAULT pipeline while the configured one was still
 * pending — a half-state of exactly the kind this requirement exists to remove.
 *
 * Everything except a `handler:` step resolves synchronously. Those are
 * returned in `deferred` for the caller to report.
 */
export function buildStrategyFromConfigSync(
  pipelineConfig: PipelineConfig,
  deps: StandardStrategyDeps,
): { strategy: ExecutionStrategy; deferred: string[] } {
  const deferred: string[] = [];
  const strategy = applyPipelineConfigSync(pipelineConfig, deps, (stepDef) => {
    const step = _resolveStepSyncOrNull(stepDef);
    if (step === null) deferred.push(stepDef.name ?? stepDef.handler ?? '<unnamed>');
    return step;
  });
  return { strategy, deferred };
}

function applyPipelineConfigSync(
  pipelineConfig: PipelineConfig,
  deps: StandardStrategyDeps,
  resolve: (def: StepDefinition) => Step | null,
): ExecutionStrategy {
  // The async entry point awaits its resolver; this one does not. The body is
  // shared so `remove`, `configure` and their error messages cannot drift.
  return applyPipelineConfigImpl(pipelineConfig, deps, resolve) as ExecutionStrategy;
}

async function applyPipelineConfig(
  pipelineConfig: PipelineConfig,
  deps: StandardStrategyDeps,
  resolve: (def: StepDefinition) => Promise<Step>,
): Promise<ExecutionStrategy> {
  const resolved = new Map<StepDefinition, Step>();
  const entries: ReadonlyArray<Record<string, unknown>> = pipelineConfig.steps ?? [];
  for (const [index, raw] of entries.entries()) {
    const def = _normalizeStepDefinition(raw, index);
    resolved.set(def, await resolve(def));
  }
  let seen = 0;
  return applyPipelineConfigImpl(pipelineConfig, deps, () => {
    const def = [...resolved.keys()][seen++];
    return def === undefined ? null : (resolved.get(def) ?? null);
  }) as ExecutionStrategy;
}

function applyPipelineConfigImpl(
  pipelineConfig: PipelineConfig,
  deps: StandardStrategyDeps,
  resolve: (def: StepDefinition) => Step | null,
): ExecutionStrategy {
  validatePipelineLimits(pipelineConfig, deps.config);

  const strategy = buildStandardStrategy(deps);

  // (1) Remove steps — fail fast if target does not exist (protocol-spec §5.16.1)
  for (const stepName of pipelineConfig.remove ?? []) {
    if (strategy.findStepIndex(stepName) === undefined) {
      throw new ConfigurationError(
        `Cannot remove pipeline step '${stepName}': no such step in the strategy. ` +
          `Either remove this entry from pipeline.remove or insert a matching step first.`,
      );
    }
    strategy.remove(stepName);
  }

  // (2) Configure existing step fields — fail fast if target does not exist (protocol-spec §5.16.1)
  for (const [stepName, overrides] of Object.entries(pipelineConfig.configure ?? {})) {
    if (strategy.findStepIndex(stepName) === undefined) {
      throw new ConfigurationError(
        `Cannot configure pipeline step '${stepName}': no such step in the strategy. ` +
          `Either remove this entry from pipeline.configure or insert a matching step first.`,
      );
    }
    for (const step of strategy.steps) {
      if (step.name === stepName) {
        // EVERY offending key, not the first. One restart shows the whole
        // problem instead of one restart per typo, and it keeps the conformance
        // assertion portable: stopping at the first makes the message depend on
        // key iteration order, which differs by language — `serde_json::Map` is
        // sorted while JS objects and Python dicts preserve insertion order, so
        // a fixture naming one key would pass here and fail on apcore-rust for
        // no behavioural reason.
        const unknownKeys = Object.keys(overrides).filter(
          (k) => resolveConfigurableStepField(k) === undefined,
        );
        if (unknownKeys.length > 0) {
          const named = unknownKeys.map((k) => `'${k}'`).join(', ');
          const noun = unknownKeys.length === 1 ? 'field' : 'fields';
          throw new ConfigurationError(
            `Pipeline step '${stepName}' has ${unknownKeys.length} non-configurable ` +
              `${noun}: ${named}. ` +
              `Valid fields are: ${CANONICAL_CONFIGURABLE_STEP_FIELDS.join(', ')} ` +
              `(the camelCase spellings ` +
              `${[...CONFIGURABLE_STEP_FIELDS.keys()]
                .filter((k) => !CANONICAL_CONFIGURABLE_STEP_FIELDS.includes(k))
                .join(', ')} are also accepted). ` +
              `'requires' and 'provides' are a step's capability contract and are ` +
              `declared by the step implementation, never by configuration; ` +
              `'name', 'description', 'removable', 'replaceable' and 'execute' are ` +
              `deliberately NOT configurable — see CONFIGURABLE_STEP_FIELDS.`,
          );
        }
        // PROTOCOL_SPEC 5.16.1 (D-130): the configurable set is four fields on
        // every step, but on a governance gate three of them can switch the
        // gate off for some or all calls. Checked on the whole entry BEFORE any
        // field is written, so a rejected entry leaves the step untouched.
        const gate = governanceGateOf(step);
        if (gate !== null) {
          const requested: Record<string, unknown> = {};
          for (const [key, value] of Object.entries(overrides)) {
            requested[resolveConfigurableStepField(key) as string] = value;
          }
          const weakening = gateWeakeningFields(requested, gate);
          if (weakening.length > 0) throw gateWeakeningError(stepName, weakening);
        }
        // Every key is known to be configurable — the check above rejected the
        // whole entry otherwise, so there is no per-key failure branch here.
        for (const [key, value] of Object.entries(overrides)) {
          const property = resolveConfigurableStepField(key) as string;
          (step as unknown as Record<string, unknown>)[property] = value;
        }
        break;
      }
    }
  }

  // (3) Resolve and insert custom steps — anchors must exist (protocol-spec §5.16.1).
  // The entry is validated against the closed `$defs/PipelineStep` key set and
  // normalised to the camelCase `StepDefinition` spelling FIRST, so an unknown
  // key raises before any step factory runs and the canonical snake_case
  // spellings actually reach the built step.
  const stepEntries: ReadonlyArray<Record<string, unknown>> = pipelineConfig.steps ?? [];
  for (const [index, rawStepDef] of stepEntries.entries()) {
    const stepDef = _normalizeStepDefinition(rawStepDef, index);
    const step = resolve(stepDef);
    if (step === null) continue;
    const after = stepDef.after;
    const before = stepDef.before;
    if (after) {
      if (strategy.findStepIndex(after) === undefined) {
        throw new ConfigurationError(
          `Cannot insert pipeline step '${step.name}' after '${after}': ` +
            `anchor step does not exist. Insert an anchor step first or use a valid anchor name.`,
        );
      }
      strategy.insertAfter(after, step);
    } else if (before) {
      if (strategy.findStepIndex(before) === undefined) {
        throw new ConfigurationError(
          `Cannot insert pipeline step '${step.name}' before '${before}': ` +
            `anchor step does not exist. Insert an anchor step first or use a valid anchor name.`,
        );
      }
      strategy.insertBefore(before, step);
    } else {
      throw new ConfigurationError(
        `Pipeline step '${step.name}' has neither 'after' nor 'before'. ` +
          `Specify one (or remove the step from pipeline.steps).`,
      );
    }
  }

  return strategy;
}
