/**
 * High-level client for apcore to simplify interaction.
 */

import type { ACL } from './acl.js';
import type { TSchema } from '@sinclair/typebox';
import type { Config } from './config.js';
import type { Context } from './context.js';
import { type FunctionModule, module as createModule } from './decorator.js';
import { InvalidInputError, SysModulesDisabledError } from './errors.js';
import { createEvent, type ApCoreEvent, type EventEmitter, type EventSubscriber } from './events/emitter.js';
import { Executor } from './executor.js';
import {
  freezeLifecycleReport,
  type LifecycleCallbacks,
  lifecycleError,
  lifecycleHandle,
  LifecycleCompletion,
  type LifecycleError,
  type LifecycleEventFlush,
  type LifecycleOperation,
  type ShutdownOptions,
  type UnregisterOptions,
  validateLifecycleOptions,
} from './lifecycle.js';
import type { StepMiddleware } from './pipeline.js';
import type { ExecutionPolicy } from './policy.js';
import type { Middleware } from './middleware/index.js';
import type { ModuleAnnotations, ModuleExample, PreflightResult, ResourceLimits } from './module.js';
import type { MetricsCollector } from './observability/metrics.js';
import type { StreamExecution } from './stream-completion.js';
import { Registry } from './registry/registry.js';
import type { RegisterSysModulesOptions, SysModulesContext } from './sys-modules/registration.js';
import { ToggleState } from './sys-modules/toggle.js';

type SysModulesInstaller = (
  registry: Registry,
  executor: Executor,
  config: Config,
  metricsCollector?: MetricsCollector | null,
  options?: RegisterSysModulesOptions,
) => SysModulesContext;

/** Optional services used to assemble an APCore instance. */
export interface APCorePlugin {
  readonly createRegistry?: (config: Config | null, callbacks?: LifecycleCallbacks) => Registry;
  readonly createLifecycleCallbacks?: () => LifecycleCallbacks;
  readonly discoverAcl?: (config: Config) => ACL | null;
  readonly registerSysModules?: SysModulesInstaller;
  readonly createTracingMiddleware?: (config: Config) => Middleware | null;
}

function pluginCapability<K extends keyof APCorePlugin>(
  plugins: readonly APCorePlugin[] | undefined, key: K,
): APCorePlugin[K] {
  const capabilities = (plugins ?? []).map(plugin => plugin[key]).filter(capability => capability !== undefined);
  if (capabilities.length > 1) {
    throw new InvalidInputError(`Multiple APCore plugins provide '${key}'`);
  }
  return capabilities[0];
}

export interface APCoreOptions {
  /** Optional instance-owned services. A supplied executor or registry takes precedence. */
  plugins?: readonly APCorePlugin[];
  registry?: Registry;
  executor?: Executor;
  config?: Config;
  metricsCollector?: MetricsCollector;
  /**
   * Per-instance ToggleState (Issue #71). Each APCore instance owns one
   * ToggleState that is injected into both the toggle module (write path)
   * and the Executor's pipeline lookup (read path), so disabling a module
   * is isolated to this instance and survives reload. Defaults to a fresh
   * `new ToggleState()`. Ignored when a pre-built `executor` is supplied.
   */
  toggleState?: ToggleState;
  /**
   * Optional ExecutionPolicy with execution-time governance overrides
   * (apcore#76 RFC pilot). Applied to the auto-created Executor's approval
   * gate. Ignored when the caller supplies their own `executor` — wire the
   * policy on that Executor directly (parity with config-driven ACL discovery).
   */
  policy?: ExecutionPolicy;
  /** Throw on system-module registration failures during startup. Defaults to false. */
  failOnError?: boolean;
}

export interface ModuleOptions {
  id?: string;
  inputSchema: TSchema;
  outputSchema: TSchema;
  description?: string;
  documentation?: string | null;
  annotations?: ModuleAnnotations | null;
  tags?: string[] | null;
  version?: string;
  metadata?: Record<string, unknown> | null;
  examples?: ModuleExample[] | null;
  display?: Record<string, unknown> | null;
  /**
   * Declared execution budget (PROTOCOL_SPEC §5.2 / D-11). `timeout` is in
   * milliseconds and overrides `executor.default_timeout` for this module.
   */
  resources?: ResourceLimits | null;
  execute: (inputs: Record<string, unknown>, context: Context) => Promise<Record<string, unknown>> | Record<string, unknown>;
}

interface ClientShutdown {
  readonly operation: LifecycleOperation;
  readonly completion: Promise<void>;
  readonly eventFlushTimeoutMs: number;
  readonly waitError: () => InvalidInputError | undefined;
}

/**
 * A high-level client that manages Registry and Executor.
 *
 * Provides a unified entry point for apcore, making it easier
 * for beginners to get started without manually managing multiple objects.
 */
export class APCore {
  readonly registry: Registry;
  readonly executor: Executor;
  readonly config: Config | null;
  readonly metricsCollector: MetricsCollector | null;
  /**
   * Per-instance ToggleState (Issue #71). Shared between the toggle module
   * (write path, via the sys-modules installer) and the Executor's pipeline
   * lookup (read path) so toggles are isolated to this instance.
   */
  private readonly _toggleState: ToggleState;
  private _sysModulesContext: SysModulesContext = {};
  private readonly _sysModulesErrors: Error[] = [];
  private _shutdown: ClientShutdown | undefined;

  constructor(options?: APCoreOptions);
  /** @internal Profile-specific defaults that do not consult the legacy adapters. */
  constructor(options: APCoreOptions | undefined, profile: APCorePlugin);
  constructor(options?: APCoreOptions, profile?: APCorePlugin) {
    this.config = options?.config ?? null;
    this.metricsCollector = options?.metricsCollector ?? null;
    const prebuiltExecutor = options?.executor;
    const createRegistry = pluginCapability(options?.plugins, 'createRegistry') ?? profile?.createRegistry;
    const discoverAcl = pluginCapability(options?.plugins, 'discoverAcl') ?? profile?.discoverAcl;
    const installSysModules = pluginCapability(options?.plugins, 'registerSysModules') ?? profile?.registerSysModules;
    const createTracingMiddleware = pluginCapability(options?.plugins, 'createTracingMiddleware')
      ?? profile?.createTracingMiddleware;
    this._toggleState = prebuiltExecutor?.toggleState ?? options?.toggleState ?? new ToggleState();

    // CLI-1: a supplied Executor brings its own Registry, and that is the one
    // this client must expose. Building a second one here meant `register()`
    // wrote into `this.registry` while `call()` resolved against
    // `executor.registry` — so `listModules()` listed a module that could not
    // be called, and with sys modules enabled `disable()` failed with
    // MODULE_NOT_FOUND for `system.control.toggle_feature`, which the
    // installer had registered into the executor's registry. apcore-rust
    // adopts `executor.registry` (client.rs:104-107) and
    // `apcore-client.md` states the precedence for the equivalent
    // constructor: the registry argument is "Ignored when executor is also
    // provided".
    //
    // CLI-3: the fallback Registry is built FROM the Config. Its constructor
    // resolves `extensions.roots` / `extensions.root` / `id_map.overrides`
    // from one, and `_scanRoots` applies `extensions.*` only when
    // `this._config !== null` — so a no-argument `new Registry()` left every
    // registry-side config key inert through the client door and made
    // `discover()` scan the hardcoded `./extensions` default whatever the
    // config said. A Config that declares none of those keys still resolves
    // to exactly the previous default, so this only ever narrows.
    const suppliedRegistry = prebuiltExecutor?.registry ?? options?.registry;
    const callbacks = suppliedRegistry === undefined
      ? (pluginCapability(options?.plugins, 'createLifecycleCallbacks') ?? profile?.createLifecycleCallbacks)?.()
      : undefined;
    this.registry = suppliedRegistry
      ?? createRegistry?.(this.config, callbacks)
      ?? new Registry({ config: this.config, lifecycleCallbacks: callbacks });
    this.executor =
      prebuiltExecutor ??
      new Executor({
        registry: this.registry,
        config: this.config,
        // Read path: the pipeline's BuiltinModuleLookup observes this same
        // ToggleState instance (Issue #71).
        toggleState: this._toggleState,
        // Execution-time governance overrides for the approval gate (apcore#76).
        policy: options?.policy ?? null,
      });

    // Respect ACLs already configured on a supplied Executor.
    if (this.config && prebuiltExecutor === undefined) {
      const aclRoot = this.config.getDeclared('acl.root');
      if (discoverAcl === undefined && aclRoot != null && aclRoot !== '') {
        throw new InvalidInputError('Configured ACL paths require an ACL discovery plugin or a configured executor');
      }
      const discovered = discoverAcl?.(this.config);
      if (discovered) this.executor.setAcl(discovered);
    }

    // Config-driven tracing (PROTOCOL_SPEC §10.1.1). `observability.tracing.*`
    // was five declared keys that reached nothing: no SDK had ever built a
    // TracingMiddleware from configuration, so `enabled: true` installed
    // nothing and the other four configured a middleware that did not exist.
    //
    // Skipped when the caller supplied their own Executor — an Executor the
    // caller built is respected as-is, tracing included, exactly as
    // config-driven ACL discovery above treats it. That is also what makes
    // §10.1.1 requirement 6 hold without a second check: the Executor built
    // here has an empty middleware chain, so configuration can never be the
    // thing that adds a SECOND tracing middleware. A caller who calls
    // `use(new TracingMiddleware(...))` afterwards is adding one deliberately.
    if (this.config && prebuiltExecutor === undefined && createTracingMiddleware) {
      const tracingMw = createTracingMiddleware(this.config);
      if (tracingMw !== null) this.executor.use(tracingMw);
    }

    if (this.config && installSysModules) {
      try {
        this._sysModulesContext = installSysModules(
          this.registry,
          this.executor,
          this.config,
          options?.metricsCollector,
          {
            toggleState: this._toggleState,
            failOnError: options?.failOnError ?? false,
            onRegistrationError: (error) => { this._sysModulesErrors.push(error); },
          },
        );
      } catch (err) {
        if (options?.failOnError) throw err;
        this._sysModulesErrors.push(err instanceof Error ? err : new Error(String(err)));
        console.warn('[apcore:client] Failed to register system modules:', err);
      }
      const emitter = this.events;
      if (prebuiltExecutor === undefined && emitter !== null) {
        this.executor.setEventEmitter(emitter);
      }
    }
  }

  /** System-module registration errors recovered during startup. */
  get sysModulesErrors(): readonly Error[] {
    return this._sysModulesErrors.slice();
  }

  /**
   * The per-instance ToggleState backing this client's enable/disable calls
   * and pipeline disabled-module checks (Issue #71). Exposed for inspection
   * and conformance testing; mutate via {@link disable}/{@link enable}.
   */
  get toggleState(): ToggleState {
    return this._toggleState;
  }

  /**
   * Create and register a FunctionModule.
   *
   * Await the returned module's `registration` before calling it with an asynchronous registry.
   * TypeScript version requires explicit schemas (no runtime type inference).
   */
  module(options: ModuleOptions): FunctionModule {
    return createModule({
      ...options,
      registry: this.registry,
    });
  }

  /**
   * Register a module object directly.
   *
   * Returns the registry's promise, which settles once an async `onLoad` has
   * completed: a module with an async `onLoad` is not callable until then, and
   * an `onLoad` rejection surfaces here instead of as an unhandled rejection.
   * Validation errors are still thrown synchronously.
   */
  register(moduleId: string, moduleObj: unknown): Promise<void> {
    return this.registry.register(moduleId, moduleObj);
  }

  /** Register modules in input order and retain earlier successes if an entry fails. */
  registerMany(entries: Iterable<readonly [string, unknown]>): Promise<void> {
    return this.registry.registerMany(entries);
  }

  /** Drain and unregister modules in input order, returning the outcome for each ID. */
  unregisterMany(moduleIds: Iterable<string>, timeoutMs?: number): Promise<boolean[]> {
    return this.registry.unregisterMany(moduleIds, timeoutMs);
  }

  /** Start a generation-specific batch with its own lifetime and progress. */
  startUnregisterMany(moduleIds: Iterable<string>, options?: UnregisterOptions): LifecycleOperation {
    return this.registry.startUnregisterMany(moduleIds, options);
  }

  /** Start or join shutdown while retaining progress independently of callers. */
  startShutdown(options: ShutdownOptions = {}): LifecycleOperation {
    validateLifecycleOptions(options, 'eventFlushTimeoutMs');
    const { eventFlushTimeoutMs, ...drainOptions } = options;
    if (this._shutdown && eventFlushTimeoutMs !== undefined && eventFlushTimeoutMs !== this._shutdown.eventFlushTimeoutMs) {
      throw new InvalidInputError('eventFlushTimeoutMs conflicts with the active client shutdown');
    }
    const registry = this.registry.startShutdown(drainOptions);
    return (this._shutdown ?? this._joinShutdown(registry, this.registry.shutdown(), eventFlushTimeoutMs ?? 5_000, false)).operation;
  }

  /** Stop the registry watcher, drain modules, and shut down this client's event emitter. */
  shutdown(timeoutMs: number = 5000): Promise<void> {
    const shutdown = this._shutdown ?? this._joinLegacyShutdown(timeoutMs);
    const error = shutdown.waitError();
    return error ? Promise.reject(error) : shutdown.completion;
  }

  private _joinLegacyShutdown(timeoutMs: number): ClientShutdown {
    const completion = this.registry.shutdown(timeoutMs);
    return this._joinShutdown(this.registry.startShutdown(), completion, timeoutMs, true);
  }

  private _joinShutdown(
    registry: LifecycleOperation,
    registryCompletion: Promise<void>,
    eventFlushTimeoutMs: number,
    legacy: boolean,
  ): ClientShutdown {
    const events = this.events;
    let eventFlush: LifecycleEventFlush = events ? 'pending' : 'not_applicable';
    let eventError: LifecycleError | undefined;
    let complete = false;
    const settled = new LifecycleCompletion();
    const callbackOwners = events ? [this.registry, events] : [this.registry];
    const callbackWaitError = () => this.registry.lifecycleCallbacks.waitError(callbackOwners)
      ?? events?.lifecycleCallbacks.waitError(callbackOwners);
    const operation = lifecycleHandle(() => {
      const report = registry.snapshot();
      return freezeLifecycleReport({
        ...report, eventFlush,
        status: complete ? (eventError ? 'failed' : report.status) : 'pending',
        ...(eventError && report.error === undefined ? { error: eventError } : {}),
      });
    }, settled, callbackWaitError);
    const completion = registryCompletion.finally(async () => {
      if (!events) return;
      try {
        const report = registry.snapshot();
        events.emit(createEvent('apcore.client.shutdown_resources_completed', null, report.status === 'completed' ? 'info' : 'error', {
          operation_id: report.operationId, registry_outcome: report.status,
          forced_count: report.modules.filter(row => row.activeAtUnload !== undefined).length,
          failure_count: report.modules.filter(row => row.error !== undefined).length,
        }));
        if (legacy) {
          await events.shutdown(eventFlushTimeoutMs);
          eventFlush = events.hasPendingDeliveries() ? 'timeout' : 'completed';
        } else {
          eventFlush = await events.shutdownWithin(eventFlushTimeoutMs);
        }
      } catch (error) {
        eventError = lifecycleError(error, 'event_flush');
        eventFlush = 'timeout';
        throw error;
      }
    }).finally(() => {
      complete = true;
      settled.settle();
    });
    void completion.catch(() => {});
    this._shutdown = { operation, completion, eventFlushTimeoutMs,
      waitError: () => complete ? undefined : callbackWaitError(),
    };
    return this._shutdown;
  }

  /**
   * Execute an async module call.
   */
  async call(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    versionHint?: string | null,
  ): Promise<Record<string, unknown>> {
    return this.executor.call(moduleId, inputs, context, versionHint);
  }

  /**
   * Async module call (alias for call).
   */
  async callAsync(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    versionHint?: string | null,
  ): Promise<Record<string, unknown>> {
    return this.executor.call(moduleId, inputs, context, versionHint);
  }

  /**
   * Stream module output chunk by chunk.
   */
  async *stream(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    versionHint?: string | null,
  ): AsyncGenerator<Record<string, unknown>> {
    yield* this.executor.stream(moduleId, inputs, context, versionHint);
  }

  /** Stream one invocation and inspect its canonical completion. */
  streamWithCompletion(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
    versionHint?: string | null,
  ): StreamExecution {
    return this.executor.streamWithCompletion(moduleId, inputs, context, versionHint);
  }

  /**
   * Non-destructive preflight check without execution.
   */
  async validate(
    moduleId: string,
    inputs?: Record<string, unknown> | null,
    context?: Context | null,
  ): Promise<PreflightResult> {
    return this.executor.validate(moduleId, inputs, context);
  }

  /**
   * Get module description info (for AI/LLM use).
   */
  describe(moduleId: string): string {
    return this.registry.describe(moduleId);
  }

  /**
   * Add class-based middleware. Returns self for chaining.
   */
  use(middleware: Middleware): APCore {
    this.executor.use(middleware);
    return this;
  }

  /**
   * Add before function middleware. Returns self for chaining.
   */
  useBefore(callback: (moduleId: string, inputs: Record<string, unknown>, context: Context) => Record<string, unknown> | null): APCore {
    this.executor.useBefore(callback);
    return this;
  }

  /**
   * Add after function middleware. Returns self for chaining.
   */
  useAfter(callback: (moduleId: string, inputs: Record<string, unknown>, output: Record<string, unknown>, context: Context) => Record<string, unknown> | null): APCore {
    this.executor.useAfter(callback);
    return this;
  }

  /**
   * Remove middleware by identity. Returns true if found and removed.
   */
  remove(middleware: Middleware): boolean {
    return this.executor.remove(middleware);
  }

  /**
   * Attach a step middleware to every pipeline step. Delegates to
   * {@link Executor.addStepMiddleware}. Returns self for chaining.
   */
  addStepMiddleware(middleware: StepMiddleware): APCore {
    this.executor.addStepMiddleware(middleware);
    return this;
  }

  /**
   * Discover and register modules from configured extension directories.
   */
  async discover(): Promise<number> {
    return this.registry.discover();
  }

  /**
   * Return sorted list of registered module IDs, optionally filtered.
   */
  listModules(options?: { tags?: string[]; prefix?: string }): string[] {
    return this.registry.list(options);
  }

  /**
   * Access the event emitter (available when sys_modules.events is enabled).
   */
  get events(): EventEmitter | null {
    return this._sysModulesContext.eventEmitter ?? null;
  }

  /**
   * Subscribe to events of a specific type with a simple callback.
   *
   * Returns the subscriber for later unsubscription via off().
   */
  on(
    eventType: string,
    handler: (event: ApCoreEvent) => void | Promise<void>,
  ): EventSubscriber {
    const emitter = this.events;
    if (!emitter) {
      throw new SysModulesDisabledError(
        'Events are not enabled. Set sys_modules.enabled=true and '
        + 'sys_modules.events.enabled=true in config.',
      );
    }
    const subscriber: EventSubscriber = {
      onEvent(event: ApCoreEvent) {
        if (event.eventType === eventType) {
          return handler(event);
        }
      },
    };
    emitter.subscribe(subscriber);
    return subscriber;
  }

  /**
   * Unsubscribe a previously registered event subscriber.
   *
   * @throws {SysModulesDisabledError} If events are not enabled
   *   (code `SYS_MODULES_DISABLED`).
   */
  off(subscriber: EventSubscriber): void {
    const emitter = this.events;
    if (!emitter) {
      throw new SysModulesDisabledError(
        'Events are not enabled. Set sys_modules.enabled=true and '
        + 'sys_modules.events.enabled=true in config.',
      );
    }
    emitter.unsubscribe(subscriber);
  }

  /**
   * Disable a module without unloading it.
   * Convenience wrapper around system.control.toggle_feature.
   */
  async disable(moduleId: string, reason: string = 'Disabled via APCore client'): Promise<Record<string, unknown>> {
    this._requireSysModules('disable');
    return this.executor.call('system.control.toggle_feature', {
      module_id: moduleId,
      enabled: false,
      reason,
    });
  }

  /**
   * Re-enable a previously disabled module.
   * Convenience wrapper around system.control.toggle_feature.
   */
  async enable(moduleId: string, reason: string = 'Enabled via APCore client'): Promise<Record<string, unknown>> {
    this._requireSysModules('enable');
    return this.executor.call('system.control.toggle_feature', {
      module_id: moduleId,
      enabled: true,
      reason,
    });
  }

  private _requireSysModules(method: string): void {
    if (!this._sysModulesContext || Object.keys(this._sysModulesContext).length === 0) {
      throw new SysModulesDisabledError(
        `Cannot call ${method}(): sys_modules must be enabled in config.`,
      );
    }
  }
}
