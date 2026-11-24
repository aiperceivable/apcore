/**
 * Auto-registration of sys.* modules and middleware from config.
 */

import type { Registry } from '../registry/registry.js';
import type { Executor } from '../executor.js';
import type { Config } from '../config.js';
import type { MetricsCollector } from '../observability/metrics.js';
import type { AuditStore } from './audit.js';
import type { OverridesStore } from './overrides.js';
import { FileOverridesStore } from './overrides.js';
import { SysModuleRegistrationError } from '../errors.js';
import { ErrorHistory } from '../observability/error-history.js';
import { ErrorHistoryMiddleware } from '../middleware/error-history.js';
import { UsageCollector, UsageMiddleware } from '../observability/usage.js';
import type { EventSubscriber } from '../events/emitter.js';
import { createEvent, EventEmitter } from '../events/emitter.js';
import { WebhookSubscriber, A2ASubscriber, FileSubscriber, StdoutSubscriber, FilterSubscriber } from '../events/subscribers.js';
import type { RetryConfig } from '../events/retry.js';
import { CircuitBreakerWrapper } from '../events/circuit-breaker.js';
import { PlatformNotifyMiddleware } from '../middleware/platform-notify.js';
import { HealthSummaryModule, HealthModule } from './health.js';
import { ManifestFullModule, ManifestModule } from './manifest.js';
import { ToggleFeatureModule, DEFAULT_TOGGLE_STATE } from './toggle.js';
import type { ToggleState } from './toggle.js';
import { UpdateConfigModule, ReloadModule } from './control.js';
import { UsageSummaryModule, UsageModule } from './usage.js';

// ---------------------------------------------------------------------------
// Namespace-mode helpers — §9.15.3
// ---------------------------------------------------------------------------

/** §9.15.3: Return sys_modules namespace dict in namespace mode, null in legacy. */
function _resolveSysCfg(config: Config): Record<string, unknown> | null {
  // C-6: Access config.mode directly via the public getter — no unsafe cast needed.
  if (config.mode === 'namespace') {
    return (config.namespace('sys_modules') as Record<string, unknown>) ?? {};
  }
  return null;
}

function _nestedGet(data: Record<string, unknown>, dottedKey: string, defaultValue: unknown): unknown {
  let current: unknown = data;
  for (const key of dottedKey.split('.')) {
    if (typeof current !== 'object' || current === null) return defaultValue;
    const obj = current as Record<string, unknown>;
    if (!(key in obj)) return defaultValue;
    current = obj[key];
  }
  return current;
}

function _cfgGet(
  sysCfg: Record<string, unknown> | null,
  config: Config,
  subKey: string,
  defaultValue: unknown,
): unknown {
  if (sysCfg !== null) {
    return _nestedGet(sysCfg, subKey, defaultValue);
  }
  return config.get(`sys_modules.${subKey}`, defaultValue);
}

// ---------------------------------------------------------------------------
// Subscriber type registry — extensible factory for EventSubscriber types
// ---------------------------------------------------------------------------

type SubscriberFactory = (config: Record<string, unknown>) => EventSubscriber;

const _subscriberFactories: Map<string, SubscriberFactory> = new Map();

/**
 * Parse a subscriber's nested `retry:` block into a RetryConfig.
 *
 * Implements the per-subscriber retry policy documented in
 * docs/features/event-system.md §"Per-Subscriber Retry Policy" (apcore#85).
 * The block is optional and may be partial — omitted keys fall back to
 * DEFAULT_RETRY when the subscriber merges it.
 *
 * Returns `undefined` when no `retry:` block is present, so the subscriber
 * applies DEFAULT_RETRY unchanged.
 */
function _parseRetryConfig(config: Record<string, unknown>): RetryConfig | undefined {
  const nested = config['retry'];
  if (typeof nested !== 'object' || nested === null || Array.isArray(nested)) {
    return undefined;
  }
  const raw = nested as Record<string, unknown>;
  const retry: RetryConfig = {};
  if (typeof raw['max_attempts'] === 'number') retry.maxAttempts = raw['max_attempts'];
  if (typeof raw['initial_backoff_ms'] === 'number') {
    retry.initialBackoffMs = raw['initial_backoff_ms'];
  }
  if (typeof raw['max_backoff_ms'] === 'number') retry.maxBackoffMs = raw['max_backoff_ms'];
  if (typeof raw['backoff_multiplier'] === 'number') {
    retry.backoffMultiplier = raw['backoff_multiplier'];
  }
  return retry;
}

function _registerBuiltInFactories(): void {
  _subscriberFactories.set('webhook', (config: Record<string, unknown>): EventSubscriber => {
    const url = config['url'] as string;
    const headers = config['headers'] as Record<string, string> | undefined;
    const retryCount = config['retry_count'] as number | undefined;
    const timeoutMs = config['timeout_ms'] as number | undefined;
    const opts: { timeoutMs?: number; retry?: RetryConfig } = {};
    if (timeoutMs !== undefined) opts.timeoutMs = timeoutMs;
    // The nested `retry:` block is canonical (apcore#85) and wins outright.
    // v0.22.0 A-D-EVT-001: subscriber-internal retry loop removed; the legacy
    // flat retry_count shorthand (deprecated, webhook-only) is still translated
    // to maxAttempts (1 + retry_count) when no nested block is present.
    const nestedRetry = _parseRetryConfig(config);
    if (nestedRetry !== undefined) {
      opts.retry = nestedRetry;
    } else if (retryCount !== undefined) {
      opts.retry = { maxAttempts: retryCount + 1 };
    }
    return new WebhookSubscriber(url, headers, opts);
  });

  _subscriberFactories.set('a2a', (config: Record<string, unknown>): EventSubscriber => {
    const platformUrl = config['platform_url'] as string;
    const auth = config['auth'] as string | Record<string, string> | undefined;
    const timeoutMs = config['timeout_ms'] as number | undefined;
    const opts: { timeoutMs?: number; retry?: RetryConfig } = {};
    if (timeoutMs !== undefined) opts.timeoutMs = timeoutMs;
    const retry = _parseRetryConfig(config);
    if (retry !== undefined) opts.retry = retry;
    return new A2ASubscriber(platformUrl, auth, opts);
  });

  _subscriberFactories.set('file', (config: Record<string, unknown>): EventSubscriber => {
    const path = config['path'];
    if (typeof path !== 'string' || path.length === 0) {
      throw new Error(`file subscriber requires a non-empty "path" string field`);
    }
    const append = config['append'] as boolean | undefined;
    const format = config['format'] as string | undefined;
    const rotateBytes = config['rotate_bytes'] as number | undefined;
    const retry = _parseRetryConfig(config);
    // `id` is positional #5 and is not read from config in any SDK — pass
    // undefined so the auto-generated subscriber ID is used, as before.
    return new FileSubscriber(
      path,
      append,
      format,
      rotateBytes,
      undefined,
      retry !== undefined ? { retry } : undefined,
    );
  });

  _subscriberFactories.set('stdout', (config: Record<string, unknown>): EventSubscriber => {
    const format = config['format'] as string | undefined;
    const levelFilter = config['level_filter'] as string | undefined;
    const retry = _parseRetryConfig(config);
    // `id` is positional #3 and is not read from config in any SDK — pass
    // undefined so the auto-generated subscriber ID is used, as before.
    return new StdoutSubscriber(
      format,
      levelFilter,
      undefined,
      retry !== undefined ? { retry } : undefined,
    );
  });

  _subscriberFactories.set('filter', (config: Record<string, unknown>): EventSubscriber => {
    const delegateType = config['delegate_type'];
    if (typeof delegateType !== 'string') {
      throw new Error(`filter subscriber requires a "delegate_type" string field`);
    }
    const delegateConfig = (config['delegate_config'] ?? {}) as Record<string, unknown>;
    const delegateFactory = _subscriberFactories.get(delegateType);
    if (delegateFactory === undefined) {
      throw new Error(`Unknown delegate subscriber type '${delegateType}' for filter subscriber`);
    }
    const delegate = delegateFactory(delegateConfig);
    const includeEvents = config['include_events'] as string[] | undefined;
    const excludeEvents = config['exclude_events'] as string[] | undefined;
    const retry = _parseRetryConfig(config);
    return new FilterSubscriber(
      delegate,
      includeEvents,
      excludeEvents,
      retry !== undefined ? { retry } : undefined,
    );
  });
}

// Register built-in factories on module load
_registerBuiltInFactories();

/** Register a custom subscriber type for config-driven instantiation. */
export function registerSubscriberType(
  typeName: string,
  factory: SubscriberFactory,
): void {
  _subscriberFactories.set(typeName, factory);
}

/** Remove a previously registered subscriber type. */
export function unregisterSubscriberType(typeName: string): void {
  if (!_subscriberFactories.has(typeName)) {
    throw new Error(`Subscriber type '${typeName}' is not registered`);
  }
  _subscriberFactories.delete(typeName);
}

/** Reset the registry to built-in types only. Intended for test teardown. */
export function resetSubscriberRegistry(): void {
  _subscriberFactories.clear();
  _registerBuiltInFactories();
}

/**
 * Instantiate a single subscriber from a config object.
 * The `type` field selects the factory; all other fields are passed as config.
 */
export function createSubscriberFromConfig(config: Record<string, unknown>): EventSubscriber {
  const typeName = config['type'] as string | undefined;
  if (typeof typeName !== 'string') {
    throw new Error('Subscriber config missing "type" field');
  }
  const factory = _subscriberFactories.get(typeName);
  if (factory === undefined) {
    throw new Error(`Unknown subscriber type '${typeName}'`);
  }
  return factory(config);
}

/**
 * Instantiate subscribers from config and subscribe them to the emitter.
 */
export function _instantiateSubscribers(
  config: Config,
  eventEmitter: EventEmitter,
  sysCfg: Record<string, unknown> | null = null,
): void {
  const subscriberConfigs = _cfgGet(sysCfg, config, 'events.subscribers', []) as
    ReadonlyArray<Record<string, unknown>>;

  for (const subscriberConfig of subscriberConfigs) {
    const typeName = subscriberConfig['type'] as string | undefined;
    if (typeof typeName !== 'string') {
      console.warn('[apcore:events]', 'Subscriber config missing "type" field, skipping');
      continue;
    }

    const factory = _subscriberFactories.get(typeName);
    if (factory === undefined) {
      console.warn('[apcore:events]', `Unknown subscriber type '${typeName}', skipping`);
      continue;
    }

    try {
      const subscriber = factory(subscriberConfig);
      const circuitConfig = (subscriberConfig['circuit_breaker'] ?? {}) as Record<string, unknown>;
      eventEmitter.subscribe(new CircuitBreakerWrapper(subscriber, eventEmitter, {
        timeoutMs: circuitConfig['timeout_ms'] as number | undefined,
        openThreshold: circuitConfig['open_threshold'] as number | undefined,
        recoveryWindowMs: circuitConfig['recovery_window_ms'] as number | undefined,
      }));
    } catch (err: unknown) {
      console.warn('[apcore:events]', `Failed to create subscriber of type '${typeName}':`, err);
    }
  }
}

export interface SysModulesContext {
  errorHistory?: ErrorHistory;
  errorHistoryMiddleware?: ErrorHistoryMiddleware;
  usageCollector?: UsageCollector;
  usageMiddleware?: UsageMiddleware;
  eventEmitter?: EventEmitter;
  platformNotifyMiddleware?: PlatformNotifyMiddleware;
}

export interface RegisterSysModulesOptions {
  failOnError?: boolean;
  overridesPath?: string;
  /** Pluggable persistent override store (Issue #45.1). */
  overridesStore?: OverridesStore;
  auditStore?: AuditStore;
  /**
   * Per-instance ToggleState (Issue #71). When provided, the toggle module
   * (write path) shares this instance with the Executor's pipeline lookup
   * (read path) so disabling a module is isolated to the owning APCore
   * instance. When omitted, ToggleFeatureModule falls back to the
   * module-level DEFAULT_TOGGLE_STATE.
   */
  toggleState?: ToggleState;
}

/**
 * Reader for an overrides YAML file. Installed by the Node-only side-effect
 * module `./overrides-file.ts`; remains `null` in browser bundles so that
 * `node:fs` does not enter the browser dependency graph.
 */
type OverridesLoader = (path: string) => Record<string, unknown> | null;
/**
 * Prefix under which `system.control.toggle_feature` persists its decisions.
 * Must match `toggle.ts` `_persistToggleOverride`, which writes
 * `toggle.${moduleId}`.
 */
const TOGGLE_OVERRIDE_PREFIX = 'toggle.';

/**
 * Apply a loaded overrides map.
 *
 * `toggle.<module_id>` keys are the persisted form of an approval-gated
 * `system.control.toggle_feature` call (toggle.ts `_persistToggleOverride`).
 * They are STATE, not configuration: routing them through `config.set()` — as
 * every override key used to be — turned the restore into an inert config
 * entry, so a module disabled before a restart came back enabled. apcore-python
 * (registration.py:285) and apcore-rust (overrides.rs:282) both strip the
 * prefix and drive the ToggleState.
 *
 * The state written to must be the one `ToggleFeatureModule` will read, which
 * is `toggleState ?? DEFAULT_TOGGLE_STATE` (toggle.ts:100) — restoring into a
 * per-instance state the module does not consult would silently do nothing.
 */
function _applyOverrides(
  overrides: Record<string, unknown>,
  config: Config,
  toggleState: ToggleState | undefined,
): void {
  const effectiveToggleState = toggleState ?? DEFAULT_TOGGLE_STATE;
  for (const [key, value] of Object.entries(overrides)) {
    if (key.startsWith(TOGGLE_OVERRIDE_PREFIX)) {
      const moduleId = key.slice(TOGGLE_OVERRIDE_PREFIX.length);
      if (moduleId !== '' && typeof value === 'boolean') {
        if (value) {
          effectiveToggleState.enable(moduleId);
        } else {
          effectiveToggleState.disable(moduleId);
        }
        continue;
      }
      // A malformed toggle key (empty id, or a non-boolean value) is not a
      // config key either — drop it rather than writing `toggle.x` into Config.
      console.warn(
        `[apcore:sys-modules] Ignoring malformed toggle override '${key}' (value must be a boolean)`,
      );
      continue;
    }
    config.set(key, value);
  }
}

let _overridesLoader: OverridesLoader | null = null;

/** @internal — used by `./overrides-file.ts` to install the Node-side reader. */
export function _setOverridesLoader(fn: OverridesLoader): void {
  _overridesLoader = fn;
}

/**
 * Auto-register all sys.* modules and middleware based on config.
 */
export function registerSysModules(
  registry: Registry,
  executor: Executor,
  config: Config,
  metricsCollector?: MetricsCollector | null,
  options?: RegisterSysModulesOptions,
): SysModulesContext {
  const result: SysModulesContext = {};
  const failOnError = options?.failOnError ?? false;
  const explicitOverridesStore = options?.overridesStore ?? null;
  const auditStore = options?.auditStore ?? null;
  // Per-instance ToggleState (Issue #71): undefined here lets
  // ToggleFeatureModule fall back to DEFAULT_TOGGLE_STATE.
  const toggleState = options?.toggleState ?? undefined;

  // §9.15.3: prefer config.namespace('sys_modules') in namespace mode
  const sysCfg = _resolveSysCfg(config);

  if (!_cfgGet(sysCfg, config, 'enabled', false)) {
    return result;
  }

  // Explicit option wins; otherwise fall back to the `sys_modules.control.
  // overrides_path` CONFIG key, as apcore-python does (registration.py:382).
  // Reading options alone meant a project that declared the key in apcore.yaml
  // got neither persistence nor restore — and APCore never passes the option
  // (client.ts installs sys modules with `{ toggleState }` only), so the config
  // key was the ONLY way a real deployment could reach it (finding A-D-015).
  const overridesPath =
    options?.overridesPath ??
    (_cfgGet(sysCfg, config, 'control.overrides_path', null) as string | null);

  // SYS-2: the path must reach the WRITE side too, not only the restore.
  // `ToggleFeatureModule` has no path field — it persists a decision only
  // `if (this._overridesStore !== null)` — and nothing here ever turned
  // `overridesPath` into a store. So a deployment that set
  // `sys_modules.control.overrides_path` and passed no programmatic store
  // restored toggles at startup and wrote none: `toggle_feature
  // {enabled: false}` reported success, persisted nothing, and the module was
  // enabled again after a restart. The round trip looked functional and was
  // not. apcore-python (`registration.py:674`) and apcore-rust
  // (`mod.rs:706`) both pass the path through to the toggle module.
  //
  // An explicitly supplied store still wins — a caller that injected a store
  // is naming the storage layer, and synthesizing a second one beside it
  // would split the overrides across two backends.
  const overridesStore: OverridesStore | null =
    explicitOverridesStore ??
    (overridesPath !== null ? new FileOverridesStore(overridesPath) : null);

  // Load overrides file and apply after base config. The actual file
  // reader is installed by the Node-only side-effect module
  // `./overrides-file.js` (imported by the package's Node entry). When
  // running in a browser bundle the loader is unset and overridesPath is
  // silently ignored.
  //
  // Only needed when an explicit store displaced the synthesized file store:
  // otherwise the `FileOverridesStore` built above reads the same YAML file
  // in the store branch below, and running both would apply it twice.
  if (explicitOverridesStore !== null && overridesPath !== null && _overridesLoader !== null) {
    // Loaders installed via `_setOverridesLoader` already swallow file IO
    // errors and return null, but we still defend against a future loader
    // that throws — preserves the v0.21.0 fail-soft semantics.
    let parsed: Record<string, unknown> | null = null;
    try {
      parsed = _overridesLoader(overridesPath);
    } catch (err) {
      console.warn('[apcore:sys-modules] Overrides loader threw:', err);
    }
    if (parsed && typeof parsed === 'object') {
      _applyOverrides(parsed, config, toggleState);
    }
  }

  // Apply overrides from the OverridesStore (Issue #45.1) — async stores are
  // not supported here; if `load()` returns a Promise we log and skip.
  if (overridesStore !== null) {
    try {
      const loaded = overridesStore.load();
      if (loaded !== null && typeof (loaded as { then?: unknown }).then === 'function') {
        console.warn(
          '[apcore:sys-modules] OverridesStore.load() returned a Promise; async stores cannot be applied during synchronous registerSysModules. Pre-load and inject into Config directly.',
        );
      } else {
        _applyOverrides(loaded as Record<string, unknown>, config, toggleState);
      }
    } catch (err) {
      console.warn('[apcore:sys-modules] Failed to load overrides from OverridesStore:', err);
    }
  }

  // Error history
  const maxPerModule = Number(_cfgGet(sysCfg, config, 'error_history.max_entries_per_module', 50));
  const maxTotal = Number(_cfgGet(sysCfg, config, 'error_history.max_total_entries', 1000));
  const errorHistory = new ErrorHistory({ maxEntriesPerModule: maxPerModule, maxTotalEntries: maxTotal });
  result.errorHistory = errorHistory;

  const ehMiddleware = new ErrorHistoryMiddleware(errorHistory);
  executor.use(ehMiddleware);
  result.errorHistoryMiddleware = ehMiddleware;

  // Usage tracking
  const usageCollector = new UsageCollector();
  result.usageCollector = usageCollector;
  const usageMiddleware = new UsageMiddleware(usageCollector);
  executor.use(usageMiddleware);
  result.usageMiddleware = usageMiddleware;

  // Register sys modules via registerInternal to bypass reserved-word checks
  const reg = (id: string, mod: unknown): void => {
    try {
      registry.registerInternal(id, mod);
    } catch (err) {
      const cause = err instanceof Error ? err : new Error(String(err));
      if (failOnError) {
        throw new SysModuleRegistrationError(id, cause);
      }
      console.error(`[apcore:sys-modules] Failed to register system module '${id}':`, err);
    }
  };

  // The three per-group flags select WHICH modules register once
  // `sys_modules.enabled` has activated the section — the split PROTOCOL_SPEC
  // 9.15.3 states (v1.17.0) and `schemas/sys-modules.schema.json` declares key
  // by key: "Whether system.health.summary and system.health.module are
  // registered", and so on. All three default to true, so a flag narrows and
  // never widens.
  //
  // They were read by nothing until now (apcore#118): every flag false
  // registered the same six modules as every flag true, while the master switch
  // worked — a half-alive section, so a smoke test of it passes.
  if (_cfgGet(sysCfg, config, 'health.enabled', true)) {
    reg('system.health.summary', new HealthSummaryModule(registry, metricsCollector ?? null, errorHistory, config));
    reg('system.health.module', new HealthModule(registry, metricsCollector ?? null, errorHistory));
  }

  if (_cfgGet(sysCfg, config, 'manifest.enabled', true)) {
    reg('system.manifest.module', new ManifestModule(registry, config));
    reg('system.manifest.full', new ManifestFullModule(registry, config));
  }

  if (_cfgGet(sysCfg, config, 'usage.enabled', true)) {
    reg('system.usage.summary', new UsageSummaryModule(usageCollector));
    reg('system.usage.module', new UsageModule(registry, usageCollector));
  }

  // Events system
  if (_cfgGet(sysCfg, config, 'events.enabled', false)) {
    const eventEmitter = new EventEmitter();
    result.eventEmitter = eventEmitter;

    // Config-driven subscribers
    _instantiateSubscribers(config, eventEmitter, sysCfg);

    // PlatformNotifyMiddleware
    const errorRateThreshold = Number(_cfgGet(sysCfg, config, 'events.thresholds.error_rate', 0.1));
    const latencyP99Threshold = Number(_cfgGet(sysCfg, config, 'events.thresholds.latency_p99_ms', 5000));
    const pnMiddleware = new PlatformNotifyMiddleware(eventEmitter, metricsCollector ?? null, errorRateThreshold, latencyP99Threshold);
    executor.use(pnMiddleware);
    result.platformNotifyMiddleware = pnMiddleware;

    // Control modules (require EventEmitter). `control.enabled` selects
    // whether the Level 2 WRITE plane registers once `events.enabled` has
    // activated it — schemas/sys-modules.schema.json: "Whether the
    // system.control.* modules are registered". Default true, so it narrows
    // and never widens. Read by nothing until now (apcore#118), which made it
    // the most consequential of the four inert sub-flags: an operator writing
    // `control.enabled: false` to keep the approval-gated write surface off a
    // deployment got all three modules anyway.
    if (_cfgGet(sysCfg, config, 'control.enabled', true)) {
      reg('system.control.toggle_feature', new ToggleFeatureModule(
        registry,
        eventEmitter,
        toggleState,
        auditStore ?? undefined,
        overridesStore ?? undefined,
      ));
      // `overridesPath` is deliberately NOT forwarded: `overridesStore` above
      // is already a `FileOverridesStore` over that same path whenever the key
      // is set, and `UpdateConfigModule` applies store-XOR-path precedence, so
      // passing both would leave its legacy `_persistOverride` writer
      // permanently unreachable from here. One writer, one file.
      reg('system.control.update_config', new UpdateConfigModule(config, eventEmitter, {
        auditStore: auditStore ?? undefined,
        overridesStore: overridesStore ?? undefined,
      }));
      reg('system.control.reload_module', new ReloadModule(registry, eventEmitter, auditStore ?? undefined));
    }

    // Wire the EventEmitter on the registry so ephemeral.* register /
    // unregister calls emit the rich D-35 contextual audit payload directly
    // (protocol-spec §2.5.1 "Audit events"). Non-ephemeral modules continue to flow through
    // the bridge below with empty payloads for backward compatibility.
    registry.setEventEmitter(eventEmitter);

    // Bridge registry events.
    // §Issue #36: legacy aliases removed in v0.22.0 — emit only the canonical
    // apcore.registry.* names.
    //
    // Single-emit rule for ephemeral.* registrations: the registry-side
    // direct emit (`Registry._emitEphemeralAudit`) already fires the
    // canonical event with the full D-35 contextual payload. We
    // short-circuit the empty-payload bridge for ephemeral.* IDs to avoid
    // dual emission for the same event_type. Mirrors apcore-python
    // `_bridge_registry_events`. See protocol-spec
    // §2.5.1 "Audit events".
    registry.on('register', (moduleId: string) => {
      if (moduleId === 'ephemeral' || moduleId.startsWith('ephemeral.')) return;
      eventEmitter.emit(createEvent(
        'apcore.registry.module_registered',
        moduleId,
        'info',
        {},
      ));
    });
    registry.on('unregister', (moduleId: string) => {
      if (moduleId === 'ephemeral' || moduleId.startsWith('ephemeral.')) return;
      eventEmitter.emit(createEvent(
        'apcore.registry.module_unregistered',
        moduleId,
        'info',
        {},
      ));
    });
  }

  return result;
}
