/**
 * Central module registry for discovering, registering, and querying modules.
 */

import type { Config } from '../config.js';
import { getDefault } from '../config-defaults.js';
import type { Context } from '../context.js';
import { createEvent, type ApCoreEvent, type EventEmitter } from '../events/emitter.js';
import type { CancelToken } from '../cancel.js';
import {
  DependencyVersionMismatchError,
  DuplicateModuleIdError,
  ErrorCodes,
  InvalidInputError,
  ModuleError,
  ModuleLoadError,
  ModuleNotFoundError,
  ModuleReloadConflictError,
  StreamingInterfaceError,
} from '../errors.js';
import {
  type DrainAbortReason,
  LifecycleCallbacks,
  LifecycleCompletion,
  lifecycleError,
  LifecycleProgress,
  type LifecycleOperation,
  type ModuleLifecycleProgress,
  type ShutdownOptions,
  type UnregisterOptions,
  type WatchOptions,
  validateLifecycleOptions,
} from '../lifecycle.js';
import { isStreamingModule } from '../streaming.js';
import type { Module, ModuleAnnotations, ModuleExample } from '../module.js';
import { detectIdConflicts } from './conflicts.js';
import { resolveDependencies } from './dependencies.js';
import { resolveEntryPoint } from './entry-point.js';
import { mergeModuleMetadata, parseDependencies, snapshotRegistryData } from './metadata-pure.js';
import { governanceUnion } from '../schema/annotations.js';
import { _discoverMultiClass } from './multi-class.js';
import { getSchema } from './schema-query.js';
import { toStrictSchema } from '../schema/strict.js';
import type { DependencyInfo, DiscoveredModule, ModuleDescriptor } from './types.js';
import { validateModule } from './validation.js';
import { matchesVersionHint } from './version.js';
import { type ManagedDrainPolicy, runManagedDrain } from './managed-drain.js';

/** @internal Registration state used to compensate for a failed reload. */
export interface RegistrationSnapshot {
  readonly moduleId: string;
  readonly module: unknown;
  readonly metadata: Record<string, unknown>;
  readonly sourceFile: string | undefined;
  readonly generation: ExecutionGeneration;
}

/** Filesystem services supplied by a discovery extension. */
export interface RegistryFilesystem {
  fs: {
    accessSync(path: string): void;
    realpathSync: { native(path: string): string };
    watch(
      path: string,
      options: { recursive: boolean },
      listener: (event: string, filename: string | null) => void,
    ): { close(): void; on(event: 'error', listener: (error: Error) => void): unknown };
  };
  path: {
    isAbsolute(path: string): boolean;
    resolve(...paths: string[]): string;
    join(...paths: string[]): string;
    basename(path: string, suffix?: string): string;
    extname(path: string): string;
  };
  entryPointSpecifier(filePath: string, reload: boolean): string;
  loadIdMap(idMapPath: string): Record<string, Record<string, unknown>>;
  loadMetadata(metaPath: string): Record<string, unknown>;
  scanExtensions(root: string, maxDepth?: number, followSymlinks?: boolean, ignorePatterns?: readonly string[]): DiscoveredModule[];
  scanMultiRoot(
    roots: Array<Record<string, unknown>>, maxDepth?: number, followSymlinks?: boolean, ignorePatterns?: readonly string[],
  ): DiscoveredModule[];
}

export interface RegistryOptions {
  config?: Config | null;
  extensionsDir?: string | null;
  extensionsDirs?: Array<string | Record<string, unknown>> | null;
  idMapPath?: string | null;
  /** Pass a capability to enable discovery explicitly, or null to use only in-memory operations. */
  filesystem?: RegistryFilesystem | null;
  lifecycleCallbacks?: LifecycleCallbacks;
}

interface DiscoveredRegistration {
  readonly sourceFile: string;
  readonly module: unknown;
  readonly metadata: Record<string, unknown>;
}

interface ExecutionGeneration {
  references: number;
  readonly drainWaiters: Set<() => void>;
  executions?: Map<CancelToken, number>;
}

interface ModuleRemoval {
  readonly generation: ExecutionGeneration;
  readonly completion: Promise<boolean>;
  readonly module: unknown;
  readonly sourceFile: string | undefined;
  readonly progress: ModuleLifecycleProgress;
  readonly setDrainDeadline?: (timeoutMs: number) => void;
}

interface RemovalOptions {
  readonly forceAfterMs?: number;
  readonly managed?: ManagedDrainPolicy;
  readonly progress?: ModuleLifecycleProgress;
  readonly owner?: LifecycleProgress;
  readonly context?: Context | null;
  readonly beforeUnload?: (module: unknown) => void | Promise<void>;
}

interface RegistryShutdown {
  readonly lifecycle: LifecycleProgress;
  readonly completion: Promise<void>;
  readonly timeoutMs: number;
  readonly force: boolean;
  readonly policy?: ManagedDrainPolicy;
  readonly rows: ReadonlyMap<ExecutionGeneration, ModuleLifecycleProgress>;
}

type BeforePublish = (moduleId: string, module: unknown) => void | Promise<void>;

interface ReloadReservation {
  readonly moduleId: string;
  readonly sourceFile?: string;
  readonly settled: LifecycleCompletion;
  publishedVersion?: string;
}

/**
 * One-shot deprecation flag for the legacy 4-arg
 * `Registry.discoverMultiClass(filePath, classes, extensionsRoot, multiClassEnabled)`
 * call shape. Cleared per process so the warning fires at most once even
 * across many invocations from the same caller. See apcore decision-log
 * D-06 (apcore commit 973410b).
 *
 * @internal exported for tests so they can reset between cases.
 */
let _multiClassEnabledDeprecationWarned = false;

/**
 * Reset the one-shot deprecation flag for the legacy 4-arg
 * `Registry.discoverMultiClass` overload. Test-only — production callers
 * never need this.
 *
 * @internal
 */
export function _resetMultiClassEnabledDeprecationWarned(): void {
  _multiClassEnabledDeprecationWarned = false;
}

function warnMultiClassEnabledDeprecated(): void {
  if (_multiClassEnabledDeprecationWarned) return;
  _multiClassEnabledDeprecationWarned = true;
  console.warn(
    '[apcore:registry] DEPRECATION: the `multiClassEnabled` argument to ' +
    '`Registry.discoverMultiClass()` is deprecated and ignored under apcore ' +
    'decision-log D-06. Mark each `ClassDescriptor` with `multiClass: true` ' +
    'instead. The 4-arg overload will be removed in v0.22.0.',
  );
}

/**
 * Standard registry event names.
 *
 * The set is **closed** (spec v1.49.0, D-80): `on`/`off` reject anything outside
 * it, and — the half that was missing here — an implementation MUST accept every
 * name it can itself emit. `file_changed` is emitted by {@link Registry.watch},
 * which is notify-only in this SDK, so it belongs in the set; while it was
 * absent, `on("file_changed", ...)` threw and every hot-reload notification was
 * fired into an empty callback list.
 */
export const REGISTRY_EVENTS = Object.freeze({
  REGISTER: "register",
  UNREGISTER: "unregister",
  FILE_CHANGED: "file_changed",
} as const);

/**
 * Valid module ID pattern. Only lowercase letters, digits, underscores, and dots.
 * Hyphens are prohibited to ensure bijective MCP/OpenAI tool name normalization.
 */
export const MODULE_ID_PATTERN = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$/;

/**
 * Maximum allowed length for a module ID.
 *
 * Per PROTOCOL_SPEC §2.7 EBNF constraint #1. 192 is filesystem-safe
 * (192 + ".binding.yaml".length = 205 < 255-byte filename limit on
 * ext4/xfs/NTFS/APFS/btrfs) and accommodates Java/.NET deep-namespace
 * FQN-derived IDs. Bumped from 128 in spec 1.6.0-draft (2026-04-08).
 */
export const MAX_MODULE_ID_LENGTH = 192;

/**
 * Reserved words that cannot appear as the first segment of a module ID.
 */
export const RESERVED_WORDS = new Set(['system', 'internal', 'core', 'apcore', 'plugin', 'schema', 'acl']);

/**
 * Namespace prefix reserved for programmatically-registered modules
 * synthesized at runtime (protocol-spec §2.5.1). IDs in this namespace MUST
 * be registered through {@link Registry.register} only — the filesystem
 * discoverer rejects matching IDs because the namespace has no
 * directory-rooted source of truth, and {@link Registry.registerInternal}
 * rejects them too so the audit-trail provenance distinction between
 * framework-emitted (`system.*`) and caller-emitted (`ephemeral.*`)
 * modules stays clean.
 *
 * The trailing dot is required so module IDs whose first segment merely
 * *starts with* `ephemeral` (e.g. `ephemerals`) are not falsely classified.
 */
export const EPHEMERAL_NAMESPACE_PREFIX = 'ephemeral.';

/** Return true when `moduleId` belongs to the reserved `ephemeral.*` namespace. */
export function isEphemeralModuleId(moduleId: string): boolean {
  return moduleId === 'ephemeral' || moduleId.startsWith(EPHEMERAL_NAMESPACE_PREFIX);
}

/**
 * Validate a module ID against PROTOCOL_SPEC §2.7 in canonical order.
 *
 * Order: empty → pattern → length → reserved (first-segment).
 * Duplicate detection is the caller's responsibility (it requires registry
 * state).
 *
 * When `allowReserved` is true the first-segment reserved word check is
 * skipped — used by `Registry.registerInternal` so sys modules can use the
 * `system.*` prefix. All other validations (empty, pattern, length) still
 * apply.
 *
 * Aligned with `apcore-python._validate_module_id` and
 * `apcore::registry::registry::validate_module_id`.
 *
 * @internal
 */
function validateModuleId(moduleId: string, allowReserved: boolean): void {
  // 1. empty check (message byte-aligned with apcore-python and apcore-rust)
  if (!moduleId || typeof moduleId !== 'string') {
    throw new InvalidInputError(
      'module_id must be a non-empty string',
      undefined,
      ErrorCodes.INVALID_MODULE_ID,
    );
  }

  // 2. EBNF pattern check (message byte-aligned with apcore-python and apcore-rust:
  // single quotes around the offending ID; bare regex source without /…/ delimiters)
  if (!MODULE_ID_PATTERN.test(moduleId)) {
    throw new InvalidInputError(
      `Invalid module ID: '${moduleId}'. Must match pattern: ${MODULE_ID_PATTERN.source} (lowercase, digits, underscores, dots only; no hyphens)`,
      undefined,
      ErrorCodes.INVALID_MODULE_ID,
    );
  }

  // 3. length check
  if (moduleId.length > MAX_MODULE_ID_LENGTH) {
    throw new InvalidInputError(
      `Module ID exceeds maximum length of ${MAX_MODULE_ID_LENGTH}: ${moduleId.length}`,
      undefined,
      ErrorCodes.INVALID_MODULE_ID,
    );
  }

  // 4. reserved word first-segment check (skipped for registerInternal)
  if (!allowReserved) {
    const firstSegment = moduleId.split('.')[0];
    if (RESERVED_WORDS.has(firstSegment)) {
      throw new InvalidInputError(
        `Module ID contains reserved word: '${firstSegment}'`,
        undefined,
        ErrorCodes.INVALID_MODULE_ID,
      );
    }
  }
}

/**
 * Interface for custom module discovery.
 */
export interface Discoverer {
  discover(roots: string[]): Array<{ moduleId: string; module: unknown }> | Promise<Array<{ moduleId: string; module: unknown }>>;
}

/**
 * Interface for custom module validation.
 */
export interface ModuleValidator {
  validate(module: unknown): string[] | Promise<string[]>;
}

/**
 * Registry event callback.
 *
 * The second argument is the module instance for `register` / `unregister`. For
 * `file_changed` it is the change payload (`{ filePath }`) — no module instance
 * exists, because the whole point of that event is that this SDK's notify-only
 * `watch()` did not construct one (spec v1.49.0, D-80).
 */
type EventCallback = (moduleId: string, module: unknown) => void;

/** First non-empty `dependencies` list among the merged views. */
function _pickDependencies(meta: Record<string, unknown>): Array<Record<string, unknown>> {
  const top = meta['dependencies'];
  if (Array.isArray(top) && top.length > 0) return top as Array<Record<string, unknown>>;
  const nested = (meta['metadata'] as Record<string, unknown> | undefined)?.['dependencies'];
  if (Array.isArray(nested) && nested.length > 0) return nested as Array<Record<string, unknown>>;
  return [];
}

/** `id_map.overrides` resolved against the project root, or `null`. */
function idMapFromConfig(config: Config | null): string | null {
  if (config === null) return null;
  const declared = config.get('id_map.overrides');
  if (typeof declared !== 'string' || declared.trim() === '') return null;
  return config.resolvePath(declared);
}


/**
 * `extensions.roots` if declared, else `extensions.root`, else the default.
 *
 * Both element shapes `$defs/ExtensionsConfig` admits are accepted: a bare path
 * string and an object carrying an explicit `namespace`.
 *
 * **Every entry gets a namespace here**, derived from the last path segment
 * when the entry does not name one — the same value `scanMultiRoot` would
 * derive. Deriving it at this door rather than leaving it to the scanner is
 * what makes a ONE-element `roots` list behave like an n-element one: the scan
 * dispatches on "more than one root or any namespace", so a lone
 * `roots: ['./beta']` would otherwise take the single-root branch and prefix
 * nothing. Nothing in the schema makes a one-element list special, and `roots`
 * versus `root` is the whole distinction between namespaced and
 * backward-compatible mode.
 *
 * A `roots` list that is present but yields no usable entry falls through to
 * the single-root branch rather than scanning nothing: an empty list is a
 * configuration that discovers no modules, and silently discovering none is the
 * failure this whole audit is about.
 */
function lastSegment(p: string): string {
  const trimmed = p.replace(/[\\/]+$/, '');
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return idx === -1 ? trimmed : trimmed.slice(idx + 1);
}

function rootsFromConfig(config: Config): Array<Record<string, unknown>> {
  const declared = config.get('extensions.roots');
  if (Array.isArray(declared)) {
    const entries: Array<Record<string, unknown>> = [];
    for (const item of declared) {
      if (typeof item === 'string' && item.trim() !== '') {
        const root = config.resolvePath(item);
        entries.push({ root, namespace: lastSegment(root) });
      } else if (item !== null && typeof item === 'object') {
        const record = item as Record<string, unknown>;
        if (typeof record['root'] === 'string' && record['root'] !== '') {
          const root = config.resolvePath(record['root']);
          const explicit = record['namespace'];
          entries.push({
            root,
            namespace:
              typeof explicit === 'string' && explicit !== ''
                ? explicit
                : lastSegment(root),
          });
        }
      }
    }
    if (entries.length > 0) return entries;
    if (declared.length > 0) {
      console.warn(
        `[apcore:registry] extensions.roots is declared with ${declared.length} ` +
          `entry/entries and none of them names a root, so it was ignored and ` +
          `extensions.root is used instead. An entry is either a path string or an ` +
          `object with a 'root' key.`,
      );
    }
  }
  const extRoot = config.get('extensions.root');
  const root = typeof extRoot === 'string' && extRoot !== ''
    ? extRoot
    : (getDefault('extensions.root') as string);
  return [{ root: config.resolvePath(root) }];
}


export class Registry {
  private _extensionRoots: Array<Record<string, unknown>>;
  private _modules: Map<string, unknown> = new Map();
  private _moduleMeta: Map<string, Record<string, unknown>> = new Map();
  private _callbacks: Map<string, EventCallback[]> = new Map([
    [REGISTRY_EVENTS.REGISTER, []],
    [REGISTRY_EVENTS.UNREGISTER, []],
    [REGISTRY_EVENTS.FILE_CHANGED, []],
  ]);
  private _idMap: Record<string, Record<string, unknown>> = {};
  private _lowercaseMap: Map<string, string> = new Map();
  private _schemaCache: Map<string, Record<string, unknown>> = new Map();
  private _config: Config | null;
  private _watchers?: Array<{ close(): void }>;
  private _watchGeneration = 0;
  private _debounceTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private _customDiscoverer: Discoverer | null = null;
  private _customValidator: ModuleValidator | null = null;
  private _idMapPath: string | null = null;
  private _idMapLoaded = false;

  /**
   * `<moduleId>@<version>@<canonical x-deprecation>` keys already warned about
   * by {@link _warnModuleDeprecated}. `getDefinition` is a read that callers
   * make repeatedly, so the notice is emitted once per module version per
   * registry rather than once per call — apcore-python routes its warning
   * through `logger.warning`, which a host filters by level; `console.warn`
   * has no such control.
   *
   * The NOTICE is part of the key (D-89, spec v1.59.0), and the key is never
   * cleared on `unregister`. Keying on `<moduleId>@<version>` alone made this
   * SDK swallow a genuinely new or changed deprecation block on a
   * re-registered module — `watch()` re-runs discovery as an unregister +
   * re-register, so the replacement's notice was silently deduped against the
   * one it replaced. Clearing on unregister is the other wrong answer: it
   * re-warns for every deprecated module on every hot reload, which is the
   * traffic-proportional spam D-89 exists to prevent.
   */
  private _deprecationWarned: Set<string> = new Set();

  // Safe hot-reload state (F09 / Algorithm A21)
  private readonly _sourceFiles = new Map<string, string>();
  private readonly _executionGenerations = new Map<string, ExecutionGeneration>();
  private readonly _manualReferences = new Map<string, Map<ExecutionGeneration, number>>();
  private readonly _removals = new Map<string, Set<ModuleRemoval>>();
  private _draining: Set<string> = new Set();
  private readonly _managedReloads = new Map<string, ReloadReservation>();
  private readonly _unavailable = new Set<string>();
  private _shutdown: RegistryShutdown | undefined;

  /**
   * In-flight async registrations: modules whose async onLoad has not yet
   * resolved. Keys in this map are NOT visible via get()/has() until the
   * promise resolves and the module is committed to _modules.
   * This enforces deferred-publish for async onLoad (issue #65).
   */
  private _inFlight: Map<string, Promise<void>> = new Map();

  /**
   * Optional EventEmitter for the ephemeral.* registry-side audit emits
   * (protocol-spec §2.5.1 "Audit events"). Set via {@link setEventEmitter}; when null, ephemeral audit
   * events are warn-logged so they never silently disappear.
   *
   * Mirrors apcore-python `Registry._event_emitter` /
   * `Registry.set_event_emitter`.
   */
  private _eventEmitter: EventEmitter | null = null;

  private readonly _filesystem: RegistryFilesystem | null;
  /** @internal Used by components that share this registry's lifecycle. */
  readonly lifecycleCallbacks: LifecycleCallbacks;

  constructor(options?: RegistryOptions) {
    this._filesystem = options?.filesystem ?? null;
    this.lifecycleCallbacks = options?.lifecycleCallbacks ?? new LifecycleCallbacks();
    const config = options?.config ?? null;
    const extensionsDir = options?.extensionsDir ?? null;
    const extensionsDirs = options?.extensionsDirs ?? null;

    if (extensionsDir !== null && extensionsDirs !== null) {
      throw new InvalidInputError('Cannot specify both extensionsDir and extensionsDirs');
    }

    if (extensionsDir !== null) {
      this._extensionRoots = [{ root: extensionsDir }];
    } else if (extensionsDirs !== null) {
      this._extensionRoots = extensionsDirs.map((item) =>
        typeof item === 'string' ? { root: item } : item,
      );
    } else if (config !== null) {
      // `extensions.roots` before `extensions.root`: §9.1.1 declares both and
      // `$defs/ExtensionsConfig` puts them in exclusive `oneOf` branches, so a
      // document carrying `roots` is in multi-root mode and `root` is only its
      // single-root sibling.
      //
      // apcore#118 decision D-70. This key was read by apcore-rust alone, so a
      // multi-root project worked on one SDK of three, silently. `scanMultiRoot`
      // has always been able to do the work — including the namespace prefixing
      // `roots` exists for — and nothing extracted the list from a `Config`.
      this._extensionRoots = rootsFromConfig(config);
    } else {
      this._extensionRoots = [{ root: getDefault('extensions.root') as string }];
    }

    this._config = config;
    // The ID map: explicit argument > `id_map.overrides` > none.
    //
    // PROTOCOL_SPEC §9.1.1 declares `id_map.overrides` and nothing read it
    // (apcore#118, decision D-71). The MECHANISM was implemented in all three
    // SDKs — `_applyIdMapOverrides` is stage 2 of discovery — and the map
    // arrived only through this constructor's `idMapPath`. Measured before the
    // fix: with `id_map.overrides` pointing at a map that renames
    // `executor/orig/mod.py`, discovery still registered `executor.orig.mod`.
    //
    // The precedence is D-73's, and the same one `extensions.root` follows
    // above: an API argument beats `Config`.
    this._idMapPath = options?.idMapPath ?? idMapFromConfig(config);
  }

  private _requireFilesystem(): RegistryFilesystem {
    const filesystem = this._filesystem;
    if (filesystem === null) {
      throw new Error('Filesystem registry operations require a discovery extension. Register modules programmatically without it.');
    }
    return filesystem;
  }

  /** Lazily load the ID map from disk on first discover(). */
  private async _ensureIdMap(): Promise<void> {
    if (this._idMapLoaded || this._idMapPath === null) return;
    this._idMap = this._requireFilesystem().loadIdMap(this._idMapPath);
    this._idMapLoaded = true;
  }

  setDiscoverer(discoverer: Discoverer): void {
    this._customDiscoverer = discoverer;
  }

  setValidator(validator: ModuleValidator): void {
    this._customValidator = validator;
  }

  discover(options?: { reloadFiles?: readonly string[] }): Promise<number>;
  /** @internal */
  discover(options?: { reloadFiles?: readonly string[]; beforePublish?: BeforePublish; managed?: ReloadReservation }): Promise<number>;
  async discover(options?: { reloadFiles?: readonly string[]; beforePublish?: BeforePublish; managed?: ReloadReservation }): Promise<number> {
    this._assertOpen();
    if (this._customDiscoverer !== null) {
      return this._discoverCustom(options?.beforePublish, options?.managed);
    }
    return this._discoverDefault(new Set(options?.reloadFiles), options?.beforePublish, options?.managed);
  }

  private async _discoverCustom(beforePublish?: BeforePublish, managed?: ReloadReservation): Promise<number> {
    const rootPaths = this._extensionRoots.map((r) => r['root'] as string);
    const customModules = await this._customDiscoverer!.discover(rootPaths);

    if (!Array.isArray(customModules)) {
      console.warn(
        `[apcore:registry] Custom discoverer returned non-array (${typeof customModules}); expected Array<{moduleId, module}>. Ignoring.`,
      );
      return 0;
    }

    let count = 0;
    for (const entry of customModules) {
      if (entry === null || typeof entry !== 'object') {
        console.warn(
          `[apcore:registry] Malformed entry from custom discoverer (expected object, got ${entry === null ? 'null' : typeof entry}); skipping.`,
        );
        continue;
      }

      const { moduleId, module: mod } = entry as { moduleId?: unknown; module?: unknown };
      if (managed && moduleId !== managed.moduleId) continue;
      if (typeof moduleId !== 'string' || mod === undefined) {
        console.warn(
          `[apcore:registry] Malformed entry from custom discoverer (missing 'moduleId' string or 'module'); skipping.`,
        );
        continue;
      }

      // Apply custom validator if set
      if (this._customValidator !== null) {
        const errors = await this._customValidator.validate(mod);
        if (errors.length > 0) {
          console.warn(
            `[apcore:registry] Custom validator rejected module '${moduleId}': ${errors.join('; ')}`,
          );
          continue;
        }
      }

      // PROTOCOL_SPEC §2.7 ID validation — sync finding A-D-102.
      // Mirrors apcore-python `Registry._discover_custom` which calls
      // `_validate_module_id` before registration. Invalid IDs are skipped
      // with a warning rather than aborting the whole discover run.
      try {
        validateModuleId(moduleId, false);
      } catch (e) {
        console.warn(
          `[apcore:registry] Skipping custom-discovered module with invalid ID '${moduleId}': ${(e as Error).message}`,
        );
        continue;
      }

      // protocol-spec §2.5.1: filesystem /
      // discoverer paths MUST reject `ephemeral.*` IDs. The custom
      // discoverer is functionally a discovery path too, so the same
      // contract applies — ephemeral.* must land via Registry.register().
      if (isEphemeralModuleId(moduleId)) {
        console.warn(
          `[apcore:registry] Skipping custom-discovered module '${moduleId}': ` +
          `'ephemeral.*' is reserved for programmatic registration via ` +
          `Registry.register(). See protocol-spec §2.5.1`,
        );
        continue;
      }

      try {
        this._assertModuleStructure(moduleId, mod);
        this._assertNoIdConflict(moduleId, RESERVED_WORDS, managed);
        await this._registerWithOnLoad(moduleId, mod, null, null, { beforePublish, managed });
        count++;
      } catch (e) {
        if (managed) throw e;
        console.warn(`[apcore:registry] Failed to register custom-discovered module '${moduleId}':`, e);
      }
    }

    return count;
  }

  /**
   * Default discovery pipeline (D-32 — 8 canonical stages, mirroring
   * apcore-rust `default_discoverer.rs`):
   *
   *   1. _ensureIdMap            — lazy-load the optional id_map.json
   *   2. _scanRoots              — walk extension roots
   *   3. _applyIdMapOverrides    — rewrite canonical IDs from the map
   *   4. _loadAllMetadata        — load each module's `module.yaml`
   *   5. _resolveAllEntryPoints  — import the JS/TS entry point
   *   6. _validateAll            — run module/custom validators
   *   7. _filterIdConflicts      — batch-drop conflicting / invalid IDs
   *   8. _resolveLoadOrder + _registerInOrder
   *                              — topological sort then register.
   */
  private async _discoverDefault(reloadFiles: ReadonlySet<string>, beforePublish?: BeforePublish, managed?: ReloadReservation): Promise<number> {
    await this._ensureIdMap();
    const discovered = await this._scanRoots();
    await this._applyIdMapOverrides(discovered);
    this._rejectEphemeralDiscoveries(discovered);

    const rawMetadata = await this._loadAllMetadata(discovered);
    const resolvedModules = await this._resolveAllEntryPoints(discovered, rawMetadata, reloadFiles);
    const validModules = await this._validateAll(resolvedModules);
    const filteredModules = this._filterIdConflicts(validModules);
    const loadOrder = this._resolveLoadOrder(filteredModules);
    return this._registerInOrder(loadOrder, filteredModules, beforePublish, managed);
  }

  private async _scanRoots(): Promise<import('./types.js').DiscoveredModule[]> {
    let maxDepth = 8;
    let followSymlinks = false;
    let ignorePatterns: readonly string[] = [];
    if (this._config !== null) {
      maxDepth = (this._config.get('extensions.max_depth', 8) as number);
      followSymlinks = (this._config.get('extensions.follow_symlinks', false) as boolean);
      ignorePatterns = (this._config.get('extensions.ignore_patterns', []) as string[]) ?? [];
    }

    const hasNamespace = this._extensionRoots.some((r) => 'namespace' in r);
    if (this._extensionRoots.length > 1 || hasNamespace) {
      return this._requireFilesystem().scanMultiRoot(this._extensionRoots, maxDepth, followSymlinks, ignorePatterns);
    }
    return this._requireFilesystem().scanExtensions(
      this._extensionRoots[0]['root'] as string,
      maxDepth,
      followSymlinks,
      ignorePatterns,
    );
  }

  /**
   * Reject filesystem-derived IDs that fall in the reserved `ephemeral.*`
   * namespace.
   *
   * Per protocol-spec §2.5.1, `ephemeral.*` is reserved for
   * programmatically-registered modules synthesized at runtime. Any
   * filesystem layout that produces such an ID is a configuration error —
   * either the directory is misnamed or the namespace prefix is being
   * misused. We surface this as an `InvalidInputError` so callers cannot
   * accidentally pollute the registry with filesystem-rooted ephemerals.
   *
   * Mirrors apcore-python `Registry._reject_ephemeral_discoveries`.
   */
  private _rejectEphemeralDiscoveries(
    discovered: import('./types.js').DiscoveredModule[],
  ): void {
    const offenders = discovered.filter((dm) => isEphemeralModuleId(dm.canonicalId));
    if (offenders.length === 0) return;
    const ids = [...new Set(offenders.map((dm) => dm.canonicalId))].sort();
    throw new InvalidInputError(
      `Filesystem discovery produced module ID(s) in the reserved ` +
      `'ephemeral.*' namespace: ${JSON.stringify(ids)}. The ephemeral.* ` +
      `namespace is reserved for programmatically-registered modules and ` +
      `may only be used via Registry.register(). Rename the offending ` +
      `directory or extension namespace.`,
      undefined,
      ErrorCodes.INVALID_MODULE_ID,
    );
  }

  private async _applyIdMapOverrides(discovered: import('./types.js').DiscoveredModule[]): Promise<void> {
    if (Object.keys(this._idMap).length === 0) return;

    const { fs: nodeFs, path: nodePath } = this._requireFilesystem();
    const resolvedRoots = this._extensionRoots.map((entry) => {
      const root = entry['root'] as string;
      return nodePath.isAbsolute(root) ? root : nodePath.resolve(root);
    });
    for (const dm of discovered) {
      for (const root of resolvedRoots) {
        try {
          // D-127: `filePath` is a canonical REAL path, so the root it is
          // measured against has to be one too. `resolve()` normalises `..` and
          // makes a path absolute; it does not resolve symlinks — so under a
          // root with a symlinked ancestor (`/var` -> `/private/var` on macOS)
          // the prefix test failed and every override was silently skipped.
          // A silently-skipped override is the worst shape for this surface:
          // the map loads, the discovery succeeds, and the IDs are simply not
          // the ones the operator declared.
          let rootReal: string;
          try {
            rootReal = nodeFs.realpathSync.native(root);
          } catch {
            rootReal = root;
          }
          const relPath = dm.filePath.startsWith(rootReal)
            ? dm.filePath.slice(rootReal.length + 1)
            : null;
          if (relPath && relPath in this._idMap) {
            const rawId = this._idMap[relPath]['id'];
            if (typeof rawId === 'string' && rawId.length > 0) {
              dm.canonicalId = rawId;
            } else {
              console.warn(`[apcore:registry] ID map entry for '${relPath}' has invalid 'id' field (got ${typeof rawId}), skipping override`);
            }
            break;
          }
        } catch (e) {
          console.warn(`[apcore:registry] Failed to apply ID map for ${dm.canonicalId}:`, e);
          continue;
        }
      }
    }
  }

  private async _loadAllMetadata(
    discovered: DiscoveredModule[],
  ): Promise<Map<DiscoveredModule, Record<string, unknown>>> {
    const rawMetadata = new Map<DiscoveredModule, Record<string, unknown>>();
    for (const dm of discovered) {
      rawMetadata.set(dm, dm.metaPath ? this._requireFilesystem().loadMetadata(dm.metaPath) : {});
    }
    return rawMetadata;
  }

  private async _resolveAllEntryPoints(
    discovered: DiscoveredModule[],
    rawMetadata: Map<DiscoveredModule, Record<string, unknown>>,
    reloadFiles: ReadonlySet<string>,
  ): Promise<Array<[string, DiscoveredRegistration]>> {
    const resolvedModules: Array<[string, DiscoveredRegistration]> = [];
    for (const dm of discovered) {
      const metadata = rawMetadata.get(dm) ?? {};
      try {
        const specifier = this._requireFilesystem().entryPointSpecifier(dm.filePath, reloadFiles.has(dm.filePath));
        const module = await resolveEntryPoint(dm.filePath, metadata, specifier);
        resolvedModules.push([dm.canonicalId, { module, metadata, sourceFile: dm.filePath }]);
      } catch (e) {
        console.warn(`[apcore:registry] Failed to resolve entry point for ${dm.canonicalId}:`, e);
      }
    }
    return resolvedModules;
  }

  private async _validateAll(
    resolvedModules: Array<[string, DiscoveredRegistration]>,
  ): Promise<Array<[string, DiscoveredRegistration]>> {
    const validModules: Array<[string, DiscoveredRegistration]> = [];
    for (const entry of resolvedModules) {
      const { module } = entry[1];
      if (this._customValidator !== null) {
        const errors = await this._customValidator.validate(module);
        if (errors.length === 0) {
          validModules.push(entry);
        }
      } else if (validateModule(module, { allowStaticSchemas: false }).length === 0) {
        validModules.push(entry);
      }
    }
    return validModules;
  }

  /**
   * Stage 7 (D-32) — batch-drop modules with invalid or conflicting IDs.
   *
   * Mirrors apcore-python `_filter_id_conflicts` and apcore-rust
   * `default_discoverer::filter_id_conflicts`. Two failure modes drop a
   * module here (warn + skip) rather than aborting the whole batch:
   *   - PROTOCOL_SPEC §2.7 ID validation (empty / pattern / length /
   *     reserved-word first segment), and
   *   - Algorithm A03 conflict detection (duplicate against an existing
   *     registration, lowercase collision, reserved-word collision).
   *
   * Soft-severity conflicts (e.g. case-insensitive match against an
   * already-registered ID at `warn` level) are NOT dropped here — the
   * warning is logged and the module flows through to registration so
   * existing behaviour is preserved. Only `error`-severity conflicts and
   * invalid IDs are filtered out.
   *
   * Keep candidates distinct until this stage so repeated IDs cannot erase
   * an earlier module or its metadata before conflict detection.
   */
  private _filterIdConflicts<T>(validModules: Iterable<[string, T]>): Map<string, T> {
    const filtered = new Map<string, T>();
    // Track within-batch IDs (case-insensitive) so two newly-discovered
    // modules whose IDs collide on lowercase don't both slip through.
    const batchLowercase = new Map<string, string>(this._lowercaseMap);
    const batchIds = new Set<string>(this._modules.keys());

    for (const [modId, mod] of validModules) {
      try {
        validateModuleId(modId, false);
      } catch (e) {
        console.warn(
          `[apcore:registry] Skipping discovered module with invalid ID '${modId}': ${(e as Error).message}`,
        );
        continue;
      }

      const conflict = detectIdConflicts(
        modId,
        batchIds,
        RESERVED_WORDS,
        batchLowercase,
      );
      if (conflict !== null) {
        if (conflict.severity === 'error') {
          console.warn(
            `[apcore:registry] Skipping discovered module '${modId}' due to ID conflict: ${conflict.message}`,
          );
          continue;
        }
        // Soft severity — log but keep the module.
        console.warn(`[apcore:registry] ID conflict: ${conflict.message}`);
      }

      filtered.set(modId, mod);
      batchIds.add(modId);
      batchLowercase.set(modId.toLowerCase(), modId);
    }

    return filtered;
  }

  private _resolveLoadOrder(
    validModules: Map<string, DiscoveredRegistration>,
  ): string[] {
    const modulesWithDeps: Array<[string, DependencyInfo[]]> = [];
    const moduleVersions = new Map<string, string>();
    for (const [modId, { module, metadata: meta }] of validModules) {
      const depsRaw = (meta['dependencies'] as Array<Record<string, unknown>>) ?? [];
      modulesWithDeps.push([modId, depsRaw.length > 0 ? parseDependencies(depsRaw) : []]);
      const yamlVersion = meta['version'];
      const codeVersion = (module as { version?: unknown })?.version;
      const resolvedVersion =
        (typeof yamlVersion === 'string' && yamlVersion) ||
        (typeof codeVersion === 'string' && codeVersion) ||
        '1.0.0';
      moduleVersions.set(modId, resolvedVersion);
    }
    // Include already-registered modules so inter-batch version constraints
    // resolve against the live registry too.
    for (const [existingId, existingMod] of this._modules.entries()) {
      if (!moduleVersions.has(existingId)) {
        const existingVersion = (existingMod as { version?: unknown })?.version;
        if (typeof existingVersion === 'string') {
          moduleVersions.set(existingId, existingVersion);
        }
      }
    }
    const knownIds = new Set([
      ...modulesWithDeps.map(([id]) => id),
      ...this._modules.keys(),
    ]);
    return resolveDependencies(modulesWithDeps, knownIds, moduleVersions);
  }

  private async _registerInOrder(
    loadOrder: string[],
    validModules: Map<string, DiscoveredRegistration>,
    beforePublish?: BeforePublish,
    managed?: ReloadReservation,
  ): Promise<number> {
    let count = 0;
    for (const moduleId of loadOrder) {
      if (managed && moduleId !== managed.moduleId) continue;
      const candidate = validModules.get(moduleId);
      if (candidate === undefined) continue;
      const { module, metadata, sourceFile } = candidate;
      try {
        const dependencies = parseDependencies((metadata['dependencies'] as Array<Record<string, unknown>>) ?? []);
        const dependencyFailure = () => this._discoveryDependencyError(moduleId, dependencies);
        const error = dependencyFailure();
        if (error) {
          if (!managed) this._emitModuleLoadFailed(moduleId, error);
          throw error;
        }
        for (const dep of dependencies) {
          if (dep.optional && validModules.has(dep.moduleId) && !this.has(dep.moduleId)) {
            console.warn(`[apcore:registry] Module '${moduleId}' is loading without optional dependency '${dep.moduleId}'`);
          }
        }
        this._assertModuleStructure(moduleId, module);
        this._assertNoIdConflict(moduleId, RESERVED_WORDS, managed);
        await this._registerWithOnLoad(moduleId, module, null, metadata, { sourceFile, dependencyFailure, beforePublish, managed });
        count++;
      } catch (error) {
        if (managed) throw error;
        console.warn(`[apcore:registry] Failed to load ${moduleId}, skipping:`, error);
      }
    }
    return count;
  }

  private _discoveryDependencyError(moduleId: string, dependencies: DependencyInfo[]): ModuleError | null {
    for (const dependency of dependencies) {
      if (dependency.optional) continue;
      if (!this.has(dependency.moduleId)) {
        return new ModuleLoadError(moduleId, `Required dependency '${dependency.moduleId}' did not load`);
      }
      const version = this._moduleMeta.get(dependency.moduleId)?.['version'];
      const actual = typeof version === 'string' ? version : '1.0.0';
      if (dependency.version && !matchesVersionHint(actual, dependency.version)) {
        return new DependencyVersionMismatchError(moduleId, dependency.moduleId, dependency.version, actual);
      }
    }
    return null;
  }

  /**
   * Register a module.
   *
   * Validation order (PROTOCOL_SPEC §2.7, aligned with apcore-python and
   * apcore-rust): empty → pattern → length → reserved (per-segment) →
   * duplicate.
   *
   * The optional `version` and `metadata` parameters mirror apcore-python's
   * `Registry.register(module_id, module, version=None, metadata=None)` for
   * cross-language signature parity (sync finding A-001). Multi-version
   * coexistence is not yet implemented in this SDK — when supplied, both
   * fields are merged into the module's metadata so callers can read them
   * back via `getDefinition()` and `list({tags})`. See PROTOCOL_SPEC §5.4.
   *
   * Returns a Promise that resolves once `onLoad` (if any) completes and the
   * module is fully visible. For modules without `onLoad` or with a sync
   * `onLoad`, the module is visible immediately (before the promise resolves)
   * so existing sync callers that do not `await` continue to work.
   *
   * All sync validation errors (invalid ID, duplicate, streaming mismatch,
   * sync onLoad failure) are thrown synchronously for backward compatibility
   * with callers that use `expect(() => register(...)).toThrow(...)` patterns.
   */
  register(
    moduleId: string,
    module: unknown,
    version?: string | null,
    metadata?: Record<string, unknown> | null,
    options?: { context?: Context | null },
  ): Promise<void> {
    this._assertOpen();
    // 1. ID validation (always sync — throws synchronously for backward compat)
    validateModuleId(moduleId, false);

    // ephemeral.* registrations only land via this programmatic path —
    // the filesystem discoverer rejects matching IDs upstream (see
    // `_rejectEphemeralDiscoveries`). When invoked here protocol-spec
    // §2.5.1 recommends `requires_approval=true` so a human gates execution of
    // agent-synthesized code; we soft-warn but never refuse.
    const ephemeral = isEphemeralModuleId(moduleId);
    if (ephemeral) {
      this._warnIfMissingApproval(moduleId, module, metadata);
    }

    // 2. Module-structure validation (sync — preserves .toThrow() compat).
    // Runs BEFORE duplicate detection: registry-system.md "Side Effects
    // (ordered)" puts "validate module_id and module structure" at step 2 and
    // the duplicate check at step 3, and apcore-python validates the streaming
    // signature before entering the lock. With the order inverted, a module
    // declaring `streaming: true` without `stream()` reported
    // DuplicateModuleIdError in TypeScript where Python and Rust report
    // StreamingInterfaceError.
    this._assertModuleStructure(moduleId, module);

    // 3. Custom validator (may be async).
    //
    // Runs BEFORE duplicate detection, matching apcore-python
    // (registry.py:1305) and apcore-rust (registry.rs:1094). With the order
    // inverted, `setValidator(v)` rejecting a module and then
    // `register(existingId, module)` reported DuplicateModuleIdError here
    // where Python reported InvalidInputError and Rust ModuleLoadError —
    // three codes for one call — and a stateful validator was invoked on
    // Python/Rust but skipped here.
    //
    // The duplicate check below stays SYNCHRONOUS for the no-validator and
    // sync-validator cases, which is what the `.toThrow()` callers depend on.
    // Only an async validator defers it into the returned Promise, and that
    // path already deferred everything after it (including onLoad failures),
    // so no caller that throws synchronously today stops doing so.
    if (this._customValidator !== null) {
      const result = this._customValidator.validate(module);
      if (result instanceof Promise) {
        return result.then((errors) => {
          if (errors.length > 0) {
            throw new InvalidInputError(`Custom validator rejected module '${moduleId}': ${errors.join('; ')}`);
          }
          this._assertNoIdConflict(moduleId);
          return this._registerWithOnLoad(moduleId, module, version, metadata, options, ephemeral);
        });
      }
      if (result.length > 0) {
        throw new InvalidInputError(`Custom validator rejected module '${moduleId}': ${result.join('; ')}`);
      }
    }

    // 4. Duplicate detection (sync — preserves backward compat with `.toThrow()` tests)
    this._assertNoIdConflict(moduleId);

    return this._registerWithOnLoad(moduleId, module, version, metadata, options, ephemeral);
  }

  /** Register modules in input order, retaining the successful prefix if a registration fails. */
  async registerMany(entries: Iterable<readonly [string, unknown]>): Promise<void> {
    for (const [moduleId, module] of entries) {
      await this.register(moduleId, module);
    }
  }

  /**
   * Algorithm A03 conflict detection for the public `register()` path.
   *
   * Extracted so the sync and async-validator branches run the identical
   * check: the duplicate set is `_modules ∪ _inFlight` (the reservation rule),
   * and a `duplicate_id` conflict is reported as DuplicateModuleIdError while
   * every other error-severity conflict is InvalidInputError.
   */
  private _assertNoIdConflict(moduleId: string, reservedWords: ReadonlySet<string> = RESERVED_WORDS, managed?: ReloadReservation): void {
    const reservation = this._managedReloads.get(moduleId);
    if (reservation !== undefined && reservation !== managed) throw new DuplicateModuleIdError(moduleId);
    const conflict = detectIdConflicts(
      moduleId,
      new Set([...this._modules.keys(), ...this._inFlight.keys()]),
      reservedWords,
      this._lowercaseMap,
    );
    if (conflict === null) return;
    if (conflict.severity === 'error') {
      if (conflict.type === 'duplicate_id') {
        throw new DuplicateModuleIdError(moduleId);
      }
      throw new InvalidInputError(conflict.message);
    }
    console.warn(`[apcore:registry] ID conflict: ${conflict.message}`);
  }

  /**
   * Core registration logic after all sync validation has passed.
   * Handles both sync and async onLoad with deferred-publish for async.
   *
   * For sync or absent onLoad: module becomes visible immediately, returns
   * a resolved Promise (backward-compatible sync usage continues to work).
   * For async onLoad: module is tracked in _inFlight and hidden from get()/has()
   * until onLoad resolves; then committed to _modules and made visible.
   */
  private _registerWithOnLoad(
    moduleId: string,
    module: unknown,
    version?: string | null,
    metadata?: Record<string, unknown> | null,
    options?: {
      context?: Context | null;
      sourceFile?: string;
      throwSyncLoadFailure?: boolean;
      mergedMetadata?: Record<string, unknown>;
      dependencyFailure?: () => ModuleError | null;
      beforePublish?: BeforePublish;
      managed?: ReloadReservation;
    },
    ephemeral?: boolean,
  ): Promise<void> {
    this._assertOpen();
    for (const removal of this._removals.get(moduleId) ?? []) {
      if (removal.module === module && removal.progress.phase !== 'done') throw new DuplicateModuleIdError(moduleId);
    }
    const overrides: Record<string, unknown> = { ...(metadata ?? {}) };
    if (version != null) overrides['version'] = version;
    const mod = module as Record<string, unknown>;
    const merged = options?.mergedMetadata ?? mergeModuleMetadata(mod, overrides);

    const onLoad = mod['onLoad'];
    const assertPublicationOpen = (): void => {
      if (options?.managed || this._shutdown?.force === false) this._assertOpen();
    };
    const failLoad = (error: unknown): void => {
      this._inFlight.delete(moduleId);
      if (!options?.managed) this._emitModuleLoadFailed(moduleId, error);
    };
    const commit = (): void => {
      assertPublicationOpen();
      if (options?.managed && this._managedReloads.get(moduleId) !== options.managed) {
        throw new ModuleReloadConflictError();
      }
      this._inFlight.delete(moduleId);
      this._modules.set(moduleId, module);
      this._executionGenerations.set(moduleId, { references: 0, drainWaiters: new Set() });
      this._moduleMeta.set(moduleId, merged);
      this._lowercaseMap.set(moduleId.toLowerCase(), moduleId);
      if (options?.sourceFile !== undefined) this._sourceFiles.set(moduleId, options.sourceFile);
      if (options?.managed) options.managed.publishedVersion = String(merged['version'] ?? '1.0.0');
      this._triggerEvent(REGISTRY_EVENTS.REGISTER, moduleId, module);
      if (ephemeral) {
        this._emitEphemeralAudit('apcore.registry.module_registered', moduleId, options?.context ?? null);
      }
    };
    const failPublication = (error: unknown): never | Promise<void> => {
      const fail = (): never => {
        failLoad(error);
        throw error;
      };
      const cleanup = typeof onLoad === 'function' || options?.managed && options.beforePublish
        ? this.lifecycleCallbacks.run([this], () => this._runOnUnload(moduleId, module)) : undefined;
      if (cleanup === undefined) return fail();
      const failedPublication = cleanup.then(fail);
      this._inFlight.set(moduleId, failedPublication);
      return failedPublication;
    };
    const publish = (): void | Promise<void> => {
      try {
        assertPublicationOpen();
        const error = options?.dependencyFailure?.();
        if (error) throw error;
        if (options?.beforePublish) {
          if (!this._inFlight.has(moduleId)) this._inFlight.set(moduleId, Promise.resolve());
          const before = this.lifecycleCallbacks.run([this], () => options.beforePublish!(moduleId, module));
          if (before !== undefined) {
            const publication = Promise.resolve(before).then(commit).catch(failPublication);
            this._inFlight.set(moduleId, publication);
            return publication;
          }
        }
        commit();
      } catch (error) {
        return failPublication(error);
      }
    };

    if (typeof onLoad === 'function') {
      this._inFlight.set(moduleId, Promise.resolve());
      try {
        const result: unknown = this.lifecycleCallbacks.run([this], () => onLoad.call(module));
        if (result != null && typeof (result as PromiseLike<unknown>).then === 'function') {
          const promise = Promise.resolve(result).then(publish, (error: unknown) => {
            failLoad(error);
            throw error;
          });
          this._inFlight.set(moduleId, promise);
          void promise.catch(() => {});
          return promise;
        }
      } catch (error) {
        failLoad(error);
        if (options?.throwSyncLoadFailure) throw error;
        const rejected = Promise.reject(error);
        void rejected.catch(() => {});
        return rejected;
      }
    }

    return publish() ?? Promise.resolve();
  }

  /**
   * Remove a registration immediately. If executions are still active, cleanup
   * waits for them to finish. Use safeUnregister to await cleanup.
   */
  unregister(moduleId: string, options?: { context?: Context | null }): boolean {
    if (this._shutdown?.force === false || this._managedReloads.has(moduleId)) return false;
    const generation = this._executionGenerations.get(moduleId);
    if (generation === undefined) return false;
    this._startRemoval(moduleId, generation, { context: options?.context });
    return true;
  }

  /** Drain and unregister modules in input order, returning the outcome for each ID. */
  async unregisterMany(moduleIds: Iterable<string>, timeoutMs: number = 5000): Promise<boolean[]> {
    const outcomes: boolean[] = [];
    for (const moduleId of moduleIds) {
      outcomes.push(await this.safeUnregister(moduleId, timeoutMs));
    }
    return outcomes;
  }

  /** Start an ordered batch that waits for each owned generation to finish. */
  startUnregisterMany(moduleIds: Iterable<string>, options: UnregisterOptions = {}): LifecycleOperation {
    validateLifecycleOptions(options, 'operationTimeoutMs');
    this._assertOpen();
    if (typeof moduleIds === 'string' || moduleIds == null || typeof moduleIds[Symbol.iterator] !== 'function') {
      throw new InvalidInputError('moduleIds must be an iterable of module IDs');
    }
    const ids = Array.from(moduleIds);
    for (const id of ids) validateModuleId(id, true);
    this._assertOpen();
    const generations = ids.map(id => this._executionGenerations.get(id) ?? this._latestRemoval(id)?.generation);
    const rows = ids.map((moduleId): ModuleLifecycleProgress => ({ moduleId, phase: 'pending' }));
    const lifecycle = new LifecycleProgress('unregister_many', rows, [], this.lifecycleCallbacks);
    const { drainTimeoutMs = 5_000, operationTimeoutMs, onDrainTimeout = 'continue_waiting', cancelGraceMs } = options;
    const policy: ManagedDrainPolicy = { timeoutMs: drainTimeoutMs, action: onDrainTimeout, cancelGraceMs };
    const deadline = operationTimeoutMs === undefined ? Infinity : performance.now() + operationTimeoutMs;
    if (rows.length === 0) lifecycle.finish();
    else void lifecycle.run(() => this._runUnregisterBatch(lifecycle, generations, policy, deadline));
    return lifecycle.operation;
  }

  private async _runUnregisterBatch(
    lifecycle: LifecycleProgress,
    generations: readonly (ExecutionGeneration | undefined)[],
    policy: ManagedDrainPolicy,
    deadline: number,
  ): Promise<void> {
    for (const [index, row] of lifecycle.report.modules.entries()) {
      const remaining = deadline - performance.now();
      if (remaining <= 0) {
        lifecycle.report.stopReason = 'operation_deadline';
        row.phase = 'done';
        row.outcome = 'not_started';
        continue;
      }
      const generation = generations[index];
      const id = row.moduleId;
      if (generation && generation === this._executionGenerations.get(id) && !this._shutdown && !this._managedReloads.has(id)) {
        await this._startRemoval(id, generation, {
          managed: { ...policy, timeoutMs: Math.min(policy.timeoutMs, Math.ceil(remaining)) }, progress: row, owner: lifecycle,
        }).completion;
      } else {
        row.phase = 'done';
        row.outcome = generation && (generation === this._executionGenerations.get(id) ||
          Array.from(this._removals.get(id) ?? []).some(removal => removal.generation === generation))
          ? 'already_removing' : 'absent';
      }
    }
  }

  private _detach(moduleId: string): unknown {
    const module = this._modules.get(moduleId);
    this._modules.delete(moduleId);
    this._moduleMeta.delete(moduleId);
    this._sourceFiles.delete(moduleId);
    this._schemaCache.delete(moduleId);
    this._lowercaseMap.delete(moduleId.toLowerCase());

    this._draining.delete(moduleId);
    this._unavailable.delete(moduleId);
    this._executionGenerations.delete(moduleId);
    return module;
  }

  private _notifyUnregister(moduleId: string, module: unknown, context?: Context | null): void {
    this._triggerEvent(REGISTRY_EVENTS.UNREGISTER, moduleId, module);
    if (isEphemeralModuleId(moduleId)) {
      this._emitEphemeralAudit(
        'apcore.registry.module_unregistered',
        moduleId,
        context ?? null,
      );
    }
  }

  private _runOnUnload(moduleId: string, module: unknown, onError?: (error: unknown) => void): Promise<void> | undefined {
    const warnUnloadFailure = (error: unknown): void => {
      console.warn(`[apcore:registry] onUnload failed for ${moduleId}:`, error);
      onError?.(error);
    };
    try {
      const onUnload = (module as Record<string, unknown>)['onUnload'];
      if (typeof onUnload === 'function') {
        const result: unknown = onUnload.call(module);
        if (result != null && typeof (result as PromiseLike<unknown>).then === 'function') {
          return Promise.resolve(result).then(() => undefined, warnUnloadFailure);
        }
      }
    } catch (error) {
      warnUnloadFailure(error);
    }
    return undefined;
  }

  /**
   * Look up a registered module by ID.
   *
   * @param moduleId - Module identifier (must be non-empty).
   * @param versionHint - Deprecated semver range. This SDK returns the
   *   single registered module and warns once per module when a hint is supplied (D-126).
   */
  get(moduleId: string, versionHint?: string | null): unknown | null {
    if (moduleId === '') {
      throw new ModuleNotFoundError('');
    }
    this._warnIgnoredVersionHint('get', moduleId, versionHint);
    // In-flight modules are not yet visible — spec #65: module MUST NOT be
    // observable via get() until onLoad completes. Cross-SDK canonical
    // behaviour (Python/Rust, A-D-002) is to return null for an in-flight id,
    // matching this SDK's own well-formed-unregistered → null contract and
    // getDefinition()'s null return for the same case (asymmetry removed).
    if (this._inFlight.has(moduleId)) {
      return null;
    }
    return this._modules.get(moduleId) ?? null;
  }

  private _warnIgnoredVersionHint(
    method: 'get' | 'getDefinition',
    moduleId: string,
    versionHint?: string | null,
  ): void {
    if (versionHint == null || versionHint === '') return;
    const key = `version-hint:${moduleId}`;
    // D-89 sets one warning per module per registry, shared by both lookup APIs.
    if (this._deprecationWarned.has(key)) return;
    this._deprecationWarned.add(key);
    console.warn(
      `[apcore:registry] ${method}('${moduleId}', '${versionHint}'): the version hint is ` +
      `ignored by this SDK and will be removed at 2.0 (spec D-126). ` +
      `Multi-version coexistence (spec §5.4) is optional and not implemented here, ` +
      `so the single registered module is returned regardless of the hint. ` +
      `Drop the argument, or use apcore-python if you need version resolution.`,
    );
  }

  has(moduleId: string): boolean {
    // In-flight modules are not visible (same invariant as get())
    if (this._inFlight.has(moduleId)) return false;
    return this._modules.has(moduleId);
  }

  /**
   * Return sorted list of registered module IDs, optionally filtered by
   * `prefix` / `tags`.
   *
   * Modules whose `ModuleAnnotations.discoverable === false` are excluded
   * by default per PROTOCOL_SPEC §4.4 (protocol-spec §2.5.1). Pass
   * `includeHidden: true` to enumerate every registered module — useful
   * for introspection tools, debug consoles, and tests.
   */
  /**
   * Return sorted list of unique registered module IDs, optionally filtered.
   *
   * @param options.tags When supplied, only modules carrying *all* of the given tags are returned.
   * @param options.prefix When supplied, only IDs starting with the prefix are returned.
   * @param options.visibility Filter by module visibility. Supported: `['public', 'hidden']`.
   *                           Defaults to `['public']` (protocol-spec §4.4 `discoverable`).
   * @param options.includeHidden Deprecated. Use `visibility: ['public', 'hidden']` instead.
   */
  list(options?: {
    tags?: string[];
    prefix?: string;
    visibility?: string[];
    includeHidden?: boolean;
  }): string[] {
    let ids = [...this._modules.keys()];

    // protocol-spec §4.4 `discoverable`: the visibility list takes precedence over the
    // deprecated includeHidden bool.
    const vis = options?.visibility ?? (options?.includeHidden === true ? ['public', 'hidden'] : ['public']);
    const showPublic = vis.includes('public');
    const showHidden = vis.includes('hidden');

    ids = ids.filter((id) => {
      const isDisc = this._isDiscoverable(id);
      return (isDisc && showPublic) || (!isDisc && showHidden);
    });

    if (options?.prefix != null) {
      ids = ids.filter((id) => id.startsWith(options.prefix!));
    }

    if (options?.tags != null) {
      const tagSet = new Set(options.tags);
      ids = ids.filter((id) => {
        const mod = this._modules.get(id) as Record<string, unknown>;
        const modTags = new Set((mod['tags'] as string[]) ?? []);
        const metaTags = (this._moduleMeta.get(id) ?? {})['tags'];
        if (Array.isArray(metaTags)) {
          for (const t of metaTags) modTags.add(t as string);
        }
        for (const t of tagSet) {
          if (!modTags.has(t)) return false;
        }
        return true;
      });
    }

    return ids.sort();
  }

  /**
   * Iterate `[moduleId, module]` pairs. Hides modules annotated
   * `discoverable: false` by default; pass `includeHidden: true` to walk
   * every registered module.
   */
  iter(options?: { includeHidden?: boolean }): IterableIterator<[string, unknown]> {
    if (options?.includeHidden === true) {
      return this._modules.entries();
    }
    const visible: Array<[string, unknown]> = [];
    for (const [id, mod] of this._modules.entries()) {
      if (this._isDiscoverable(id)) visible.push([id, mod]);
    }
    return visible[Symbol.iterator]();
  }

  /** Total number of registered modules (includes `discoverable: false`). */
  get count(): number {
    return this._modules.size;
  }

  /**
   * Sorted list of registered module IDs (excludes
   * `ModuleAnnotations.discoverable === false` modules). Use
   * {@link list} with `includeHidden: true` when the full set is required.
   */
  get moduleIds(): string[] {
    return [...this._modules.keys()]
      .filter((id) => this._isDiscoverable(id))
      .sort();
  }

  /**
   * Return false when the module's annotations declare
   * `discoverable: false`. Anything else (including missing annotations)
   * keeps the module visible — the default `true` preserves backward
   * compatibility.
   *
   * Resolution order matches `mergeModuleMetadata`:
   * 1. The merged `annotations` slot in `_moduleMeta` (which already
   *    accounts for YAML > code precedence).
   * 2. The module instance's `annotations` attribute (covers paths that
   *    bypassed the merge, e.g. `registerInternal`).
   */
  private _isDiscoverable(moduleId: string): boolean {
    const meta = this._moduleMeta.get(moduleId);
    if (meta != null) {
      const ann = meta['annotations'];
      if (ann != null) {
        const dv = this._readDiscoverable(ann);
        if (dv === false) return false;
        if (dv === true) return true;
      }
    }
    const mod = this._modules.get(moduleId);
    if (mod == null) return true;
    const ann = (mod as Record<string, unknown>)['annotations'];
    if (ann == null) return true;
    return this._readDiscoverable(ann) !== false;
  }

  private _readDiscoverable(ann: unknown): boolean | undefined {
    if (ann == null) return undefined;
    if (typeof ann === 'object') {
      // Accept both ModuleAnnotations instances and YAML-style dicts.
      const rec = ann as Record<string, unknown>;
      const v = rec['discoverable'];
      if (v === true || v === false) return v;
    }
    return undefined;
  }

  /**
   * Return the merged metadata stored for a module, or `{}` when the module is
   * unknown. Mirrors apcore-python `Registry.get_module_metadata`.
   *
   * This is the post-registration view of `mergeModuleMetadata` — the only
   * public surface carrying fields `ModuleDescriptor` does not model, notably
   * `dependencies`, which `ReloadModule` reads to order a bulk reload.
   * A shallow copy is returned so callers cannot mutate registry state.
   */
  getModuleMetadata(moduleId: string): Record<string, unknown> {
    return { ...(this._moduleMeta.get(moduleId) ?? {}) };
  }

  /**
   * Return the merged (descriptor) annotations for a module, or `null`.
   *
   * The SECOND governance source PROTOCOL_SPEC §7.4 (D-96) unions with the live
   * module instance. It is not a duplicate of the instance's own `annotations`:
   * `mergeModuleMetadata` folds a `*_meta.yaml` / `metadata` declaration into
   * this slot with YAML > code precedence, so an operator can declare
   * governance HERE that the instance never carries.
   *
   * Reading it is what closes a bypass: before this existed, a metadata source
   * declaring `requires_approval: true` reached `getDefinition()` and the
   * manifest, and reached no gate — the module executed ungated with an
   * approval handler configured.
   *
   * Cheap by design: one map read, because the approval gate consults it on
   * every call. Callers union it with the instance's annotations via
   * `governanceUnion`; they MUST NOT use it alone, since the merge's
   * YAML > code precedence would let a metadata `false` cancel a module that
   * asks to be gated.
   */
  getDeclaredAnnotations(moduleId: string): ModuleAnnotations | null {
    const meta = this._moduleMeta.get(moduleId);
    if (meta === undefined) return null;
    return (meta['annotations'] as ModuleAnnotations | null) ?? null;
  }

  /** Return the module descriptor with detached schemas and metadata records. */
  getDefinition(moduleId: string, versionHint?: string | null): ModuleDescriptor | null {
    // Keep the same invalid-ID error as the instance lookup.
    if (moduleId === '') {
      throw new ModuleNotFoundError('');
    }
    this._warnIgnoredVersionHint('getDefinition', moduleId, versionHint);
    const module = this._modules.get(moduleId);
    if (module == null) return null;
    // INVARIANT: every registration site (`register`, `registerInternal`,
    // `_discoverDefault`) populates `_moduleMeta` via `mergeModuleMetadata`,
    // so `meta` always contains the full set of canonical keys including
    // an `annotations` slot. Read fields directly from it. The schemas
    // come straight from the module instance because they are not part
    // of the merged metadata payload.
    const meta = this._moduleMeta.get(moduleId) ?? {};
    const mod = module as Record<string, unknown>;
    const effectiveMetadata = (meta['metadata'] as Record<string, unknown>) ?? {};
    const version = (meta['version'] as string) ?? '1.0.0';

    // `sunsetDate` is derived from the `x-deprecation` extension block, which
    // is where a module declares it — `mergeModuleMetadata` never writes a
    // top-level `sunsetDate` key, so reading one returned null for every
    // module ever registered and no deprecation notice was ever emitted.
    // Mirrors apcore-python `Registry.get_definition` /
    // `_log_deprecation_warning`.
    const deprecation = effectiveMetadata['x-deprecation'];
    let sunsetDate: string | null = null;
    if (deprecation != null && typeof deprecation === 'object') {
      const dep = deprecation as Record<string, unknown>;
      const declared = dep['sunset_date'];
      sunsetDate = typeof declared === 'string' ? declared : null;
      this._warnModuleDeprecated(moduleId, version, dep);
    }

    return snapshotRegistryData({
      moduleId,
      name: (meta['name'] as string | null) ?? null,
      description: (meta['description'] as string) ?? '',
      documentation: (meta['documentation'] as string | null) ?? null,
      inputSchema: (mod['inputSchema'] as Record<string, unknown>) ?? {},
      outputSchema: (mod['outputSchema'] as Record<string, unknown>) ?? {},
      version,
      tags: (meta['tags'] as string[]) ?? [],
      annotations: (meta['annotations'] as ModuleAnnotations | null) ?? null,
      examples: (meta['examples'] as ModuleExample[]) ?? [],
      metadata: effectiveMetadata,
      sunsetDate,
      // Parsed once here so consumers get typed DependencyInfo rather than
      // re-deriving `{module_id, version?, optional?}` from raw JSON at every
      // call site (PROTOCOL_SPEC §12.2; sync finding A-D-004).
      // `meta` is checked first because `mergeModuleMetadata` lifts a declared
      // `dependencies` to the top level; the nested `metadata.dependencies`
      // form is what the manual register path supplies.
      // `mergeModuleMetadata` lifts a declared `dependencies` to the top level,
      // but a module carrying them as CODE metadata (`module.metadata`) leaves
      // the top-level entry an empty array while the nested block holds them —
      // so fall through on empty, not merely on null.
      dependencies: parseDependencies(_pickDependencies(meta)),
    });
  }

  /**
   * Canonical rendering of an `x-deprecation` block, for the D-89 dedupe key.
   *
   * Sorted keys and no incidental whitespace, so two structurally equal
   * notices dedupe regardless of how they were built (spec v1.59.0).
   */
  private static _canonicalDeprecation(deprecation: Record<string, unknown>): string {
    const sorted: Record<string, unknown> = {};
    for (const k of Object.keys(deprecation).sort()) sorted[k] = deprecation[k];
    try {
      return JSON.stringify(sorted);
    } catch {
      // A cyclic or otherwise unserialisable notice: fall back to the key set,
      // which is stable enough for an equality key and never throws on a read.
      return Object.keys(sorted).join(',');
    }
  }

  /**
   * Warn that a module version is deprecated, once per
   * `<moduleId>@<version>@<canonical notice>`.
   *
   * Message shape mirrors apcore-python `Registry._log_deprecation_warning`
   * so an operator reading two SDKs' logs sees the same sentence.
   */
  private _warnModuleDeprecated(
    moduleId: string,
    version: string,
    deprecation: Record<string, unknown>,
  ): void {
    const key = `${moduleId}@${version}@${Registry._canonicalDeprecation(deprecation)}`;
    if (this._deprecationWarned.has(key)) return;
    this._deprecationWarned.add(key);

    const deprecatedSince = (deprecation['deprecated_since'] as string) ?? 'unknown';
    const sunsetVersion = (deprecation['sunset_version'] as string) ?? 'unknown';
    const migrationGuide = (deprecation['migration_guide'] as string) ?? '';
    let msg =
      `[apcore:registry] Module '${moduleId}' v${version} is deprecated ` +
      `(since ${deprecatedSince}, sunset in ${sunsetVersion}).`;
    if (migrationGuide) {
      msg += ` Migration: ${migrationGuide}`;
    }
    console.warn(msg);
  }

  describe(moduleId: string): string {
    const module = this.get(moduleId);
    if (module === null) {
      throw new ModuleNotFoundError(moduleId);
    }

    // Module-supplied override (spec v1.49.0, D-77 — "Contract: Registry.describe").
    //
    // `Module.describe()` is declared as returning a `ModuleDescription` mapping
    // (possibly a Promise of one), so passing its result straight through a
    // method this class types as `string` handed callers an object or a Promise
    // and broke the first `.split('\n')` they wrote. Only a genuine string is
    // the author's deliberate override; anything else — a structured return, a
    // Promise this synchronous method cannot resolve, null/undefined — falls
    // through to the generated envelope. Never stringify the mapping:
    // `get_definition` is already the structured accessor.
    const modObj = module as Record<string, unknown>;
    if (typeof modObj['describe'] === 'function') {
      const result = (modObj['describe'] as () => unknown).call(module);
      if (typeof result === 'string') {
        return result;
      }
      // A Promise we are about to discard must not become an unhandled
      // rejection — the caller never had a handle on it to attach one.
      if (result != null && typeof (result as PromiseLike<unknown>).then === 'function') {
        void Promise.resolve(result).catch(() => undefined);
      }
    }

    // Auto-generate from descriptor
    const descriptor = this.getDefinition(moduleId);
    if (descriptor === null) {
      return `Module: ${moduleId}\n\nNo description available.`;
    }

    const lines: string[] = [`# ${descriptor.moduleId}`];
    if (descriptor.description) {
      lines.push(`\n${descriptor.description}`);
    }
    if (descriptor.tags.length > 0) {
      lines.push(`\n**Tags:** ${descriptor.tags.join(', ')}`);
    }
    const props = descriptor.inputSchema['properties'] as Record<string, Record<string, unknown>> | undefined;
    if (props && Object.keys(props).length > 0) {
      lines.push('\n**Parameters:**');
      const requiredFields = (descriptor.inputSchema['required'] as string[]) ?? [];
      for (const [param, schema] of Object.entries(props)) {
        const paramType = (schema['type'] as string) ?? 'any';
        const paramDesc = (schema['description'] as string) ?? '';
        const isRequired = requiredFields.includes(param);
        const reqMarker = isRequired ? ' (required)' : '';
        lines.push(`- \`${param}\` (${paramType})${reqMarker}: ${paramDesc}`);
      }
    }
    if (descriptor.documentation) {
      lines.push(`\n**Documentation:**\n${descriptor.documentation}`);
    }
    return lines.join('\n');
  }

  on(event: string, callback: EventCallback): void {
    const validEvents = Object.values(REGISTRY_EVENTS) as string[];
    if (!validEvents.includes(event)) {
      throw new InvalidInputError(
        `Invalid event: ${event}. Must be one of: ${validEvents.map((e) => `'${e}'`).join(', ')}`,
      );
    }
    this._callbacks.get(event)!.push(callback);
  }

  off(event: string, callback: EventCallback): boolean {
    const validEvents = Object.values(REGISTRY_EVENTS) as string[];
    if (!validEvents.includes(event)) {
      throw new InvalidInputError(
        `Invalid event: ${event}. Must be one of: ${validEvents.map((e) => `'${e}'`).join(', ')}`,
      );
    }
    const callbacks = this._callbacks.get(event)!;
    const idx = callbacks.indexOf(callback);
    if (idx === -1) return false;
    callbacks.splice(idx, 1);
    return true;
  }

  async reload(): Promise<number> {
    return this.discover();
  }

  private _triggerEvent(event: string, moduleId: string, module: unknown): void {
    const callbacks = this._callbacks.get(event) ?? [];
    for (const cb of [...callbacks]) {
      try {
        cb(moduleId, module);
      } catch (e) {
        console.warn(`[apcore:registry] Event callback error for '${event}' on ${moduleId}:`, e);
      }
    }
  }

  /**
   * Watch the configured extension roots for filesystem changes and
   * unregister any module whose source file is modified or deleted.
   *
   * **Cross-language divergence (sync finding A-D-104):** unlike apcore-python
   * (which re-imports the file via `importlib.reload`) and apcore-rust (which
   * triggers full rediscovery), the TypeScript SDK is **event-only**. On a
   * file change the registry:
   *   1. unregisters the previously-loaded module (calling its `onUnload`),
   *   2. emits a `'file_changed'` event with `{ filePath }` payload.
   *
   * Consumers are expected to subscribe and re-register the module
   * themselves (e.g. by calling `registry.discover()` or registering a fresh
   * import). ES module specifiers are immutable in Node — there is no
   * portable "reload from disk" primitive — so a transparent dynamic
   * `import()` would silently return the cached old module on every
   * invocation. A workaround using a cache-busting query (`?v=Date.now()`)
   * leaks the old module each reload and breaks browser bundlers, so it is
   * intentionally **not** offered here.
   *
   * If your application needs Python-style hot-reload semantics, listen for
   * `'file_changed'` and re-discover or re-import explicitly.
   */
  async watch(options?: WatchOptions): Promise<void> {
    this._assertOpen();
    if (options !== undefined) validateLifecycleOptions(options);
    if (this._watchers && this._watchers.length > 0) {
      return; // Already watching
    }

    const generation = this._watchGeneration;
    const drainOptions = options === undefined ? undefined : { ...options };
    const { fs, path: nodePath } = this._requireFilesystem();
    const { join } = nodePath;

    this._watchers = [];
    for (const root of this._extensionRoots) {
      const rootPath = typeof root === "string" ? root : (root as Record<string, unknown>).root as string;
      if (!rootPath) continue;

      try {
        const watchedDir = fs.realpathSync.native(rootPath);
        const watcher = fs.watch(watchedDir, { recursive: true }, (eventType: string, filename: string | null) => {
          if (this._watchGeneration !== generation || !filename) return;
          if (!filename.endsWith(".ts") && !filename.endsWith(".js")) return;

          const fullPath = join(watchedDir, filename);
          if (this._debounceTimers.has(fullPath)) return;
          const debounce = setTimeout(() => { this._debounceTimers.delete(fullPath); }, 300);
          debounce.unref?.();
          this._debounceTimers.set(fullPath, debounce);

          const handle = (p: Promise<void>): void => {
            p.catch((e: unknown) => {
              console.warn(`[apcore:registry] Watch handler failed for ${fullPath}:`, e);
            });
          };

          if (eventType === "rename") {
            // Could be create or delete
            try {
              fs.accessSync(fullPath);
              handle(this._handleFileChange(fullPath, drainOptions));
            } catch {
              handle(this._handleFileDeletion(fullPath, drainOptions));
            }
          } else {
            handle(this._handleFileChange(fullPath, drainOptions));
          }
        });
        watcher.on('error', (error) => {
          console.warn(`[apcore:registry] Filesystem watch failed for '${rootPath}':`, error);
        });
        this._watchers.push(watcher);
      } catch (e) {
        // Surface real failures (EMFILE, EACCES, Linux kernels < 4.7 without
        // recursive support, etc.). A silently non-functional watch misleads
        // users who expect hot-reload to be active.
        console.warn(
          `[apcore:registry] fs.watch failed for '${rootPath}' — hot-reload disabled for this root:`,
          e,
        );
      }
    }
  }

  unwatch(): void {
    this._watchGeneration += 1;
    if (this._watchers) {
      for (const watcher of this._watchers) {
        watcher.close();
      }
      this._watchers = [];
    }
    for (const timer of this._debounceTimers.values()) clearTimeout(timer);
    this._debounceTimers.clear();
  }

  private async _handleFileChange(filePath: string, options?: WatchOptions): Promise<void> {
    const watchGeneration = this._watchGeneration;
    const { path: nodePath } = this._requireFilesystem();
    const { basename, extname } = nodePath;
    const removal = this._unregisterWatchedFile(filePath, options);
    if (removal !== null) await removal.completion;
    if (this._watchGeneration !== watchGeneration) return;

    // Re-import is complex in ES modules — tell consumers that a watched file
    // changed so they can re-import and re-register. The earlier design
    // emitted a 'register' event with a null module, which crashed any
    // consumer that accessed fields on the module argument.
    this._triggerEvent(
      REGISTRY_EVENTS.FILE_CHANGED,
      removal?.moduleId ?? basename(filePath, extname(filePath)),
      { filePath },
    );
  }

  private async _handleFileDeletion(path: string, options?: WatchOptions): Promise<void> {
    await this._unregisterWatchedFile(path, options)?.completion;
  }

  private _unregisterWatchedFile(filePath: string, options?: WatchOptions): { moduleId: string; completion: Promise<unknown> } | null {
    for (const reservation of this._managedReloads.values()) {
      if (reservation.sourceFile === filePath) return { moduleId: reservation.moduleId, completion: reservation.settled.wait() };
    }
    for (const [moduleId, sourceFile] of this._sourceFiles) {
      if (sourceFile === filePath) {
        const completion = options === undefined ? this.safeUnregister(moduleId) : this.startUnregisterMany([moduleId], options).wait();
        return { moduleId, completion };
      }
    }
    for (const [moduleId, removals] of this._removals) {
      const pending = Array.from(removals).filter(({ sourceFile }) => sourceFile === filePath);
      if (pending.length > 0) {
        return { moduleId, completion: Promise.all(pending.map(({ completion }) => completion)) };
      }
    }
    return null;
  }

  /**
   * Register a sys/internal module that bypasses **only** the reserved word
   * check. All other PROTOCOL_SPEC §2.7 validations (empty, EBNF pattern,
   * length, duplicate) still apply.
   *
   * Used exclusively by the sys-modules subsystem for `system.*` IDs.
   * Aligned with apcore-python `Registry.register_internal` and apcore-rust
   * `Registry::register_internal`.
   */
  private _assertModuleStructure(moduleId: string, module: unknown, requireMetadata = true): void {
    const errors = validateModule(module, { requireMetadata, allowStaticSchemas: !requireMetadata });
    if (errors.length > 0) {
      throw new InvalidInputError(`Invalid module '${moduleId}': ${errors.join('; ')}`);
    }
    const mod = module as Record<string, unknown>;
    const ann = mod['annotations'];
    if (ann == null || typeof ann !== 'object') return;
    if ((ann as Record<string, unknown>)['streaming'] !== true) return;
    if (isStreamingModule(module as unknown as import('../module.js').Module)) return;
    const hasStreamMethod = typeof mod['stream'] === 'function';
    const hasMarker =
      (mod as unknown as Record<symbol, unknown>)[Symbol.for('apcore.streaming')] === true;
    throw new StreamingInterfaceError(moduleId, true, hasStreamMethod, hasMarker);
  }

  registerInternal(moduleId: string, module: unknown): Promise<void> | void {
    // protocol-spec §2.5.1: ephemeral.* IDs MUST be rejected here. Namespace →
    // registration-mechanism is a 1:1 mapping; mixing blurs the audit-trail
    // distinction between framework-emitted (`system.*`) and caller-emitted
    // (`ephemeral.*`) modules.
    if (isEphemeralModuleId(moduleId)) {
      throw new InvalidInputError(
        `ephemeral.* module IDs must be registered via Registry.register(), ` +
        `not registerInternal() (got: '${moduleId}'). ` +
        `See protocol-spec §2.5.1 for rationale.`,
        undefined,
        ErrorCodes.INVALID_MODULE_ID,
      );
    }
    validateModuleId(moduleId, true);

    // STR-5: module-structure validation, before duplicate detection — the
    // order registry-system.md's "Side Effects (ordered)" states and D-86
    // settles (module_id -> structure/streaming -> validator -> duplicate).
    this._assertModuleStructure(moduleId, module, false);

    this._assertNoIdConflict(moduleId, new Set());
    return this._registerWithOnLoad(moduleId, module, null, null, { throwSyncLoadFailure: true });
  }

  /** @internal Capture the published registration before unloading its module. */
  snapshotRegistration(moduleId: string): RegistrationSnapshot | null {
    const generation = this._executionGenerations.get(moduleId);
    if (!generation) return null;
    return {
      moduleId,
      module: this._modules.get(moduleId),
      metadata: this.getModuleMetadata(moduleId),
      sourceFile: this._sourceFiles.get(moduleId),
      generation,
    };
  }

  /** @internal Republish a previous registration after its load hook completes. */
  async restoreRegistration(snapshot: RegistrationSnapshot, beforePublish?: BeforePublish, managed?: ReloadReservation): Promise<void> {
    const { moduleId, module, metadata, sourceFile } = snapshot;
    validateModuleId(moduleId, true);
    this._assertModuleStructure(moduleId, module, false);
    this._assertNoIdConflict(moduleId, new Set(), managed);
    await this._registerWithOnLoad(moduleId, module, null, null, {
      sourceFile,
      mergedMetadata: metadata,
      beforePublish,
      managed,
    }, isEphemeralModuleId(moduleId));
  }

  /** @internal Legacy reload cannot take over a managed replacement. */
  isManagedReloading(moduleId: string): boolean {
    return this._managedReloads.has(moduleId);
  }

  /** @internal Run an opt-in reload against the selected registration only. */
  async reloadManagedRegistration(
    snapshot: RegistrationSnapshot,
    policy: ManagedDrainPolicy,
    owner: LifecycleProgress,
    row: ModuleLifecycleProgress,
    emitter: EventEmitter,
  ): Promise<string | undefined> {
    const { moduleId, generation, sourceFile } = snapshot;
    const module = snapshot.module as Module;
    if (this._shutdown || this._managedReloads.has(moduleId) || this._draining.has(moduleId) ||
        this._executionGenerations.get(moduleId) !== generation) {
      row.phase = 'done';
      row.outcome = 'failed';
      row.compensation = 'not_required';
      row.error = lifecycleError(new ModuleReloadConflictError(), 'draining');
      return;
    }

    const reservation: ReloadReservation = { moduleId, sourceFile, settled: new LifecycleCompletion() };
    this._managedReloads.set(moduleId, reservation);
    this._draining.add(moduleId);
    try {
      const { abort } = await this._runManagedDrain(module, generation, owner, row, policy, emitter);
      if (abort) {
        await this._recoverManagedReload(module, owner, row, abort, emitter);
      } else if (this._shutdown) {
        row.phase = 'done';
        row.outcome = 'failed';
        row.compensation = 'not_required';
        row.error = lifecycleError(new ModuleReloadConflictError(), 'draining');
      } else {
        return await this._replaceManagedRegistration(snapshot, reservation, sourceFile, owner, row, emitter);
      }
    } finally {
      this._managedReloads.delete(moduleId);
      reservation.settled.settle();
    }
  }

  private async _recoverManagedReload(
    module: Module, owner: LifecycleProgress, row: ModuleLifecycleProgress, abortReason: DrainAbortReason, emitter: EventEmitter,
  ): Promise<void> {
    const { moduleId } = row;
    row.phase = 'recovering';
    row.abortReason = abortReason;
    try {
      if (typeof module.onDrain === 'function' && typeof module.onDrainAborted === 'function') {
        row.activeHook = 'on_drain_aborted';
        await this.lifecycleCallbacks.run([this, owner], () => module.onDrainAborted!({
          operationId: owner.report.operationId, moduleId, signal: new AbortController().signal, abortReason,
        }));
      }
      if (!this._shutdown) {
        this._draining.delete(moduleId);
        row.outcome = 'retained_ready';
      }
    } catch (error) {
      row.error = lifecycleError(error, 'recovering');
    } finally {
      delete row.activeHook;
    }
    if (!row.outcome) {
      this._unavailable.add(moduleId);
      row.outcome = 'retained_unavailable';
    }
    row.phase = 'done';
    row.compensation = 'not_required';
    this._emitLifecycleEvent(emitter, 'module_drain_recovered', owner, row,
      row.outcome === 'retained_ready' ? 'info' : 'error', { abort_reason: abortReason, outcome: row.outcome });
  }

  private async _replaceManagedRegistration(
    snapshot: RegistrationSnapshot, reservation: ReloadReservation, sourceFile: string | undefined,
    owner: LifecycleProgress, row: ModuleLifecycleProgress, emitter: EventEmitter,
  ): Promise<string | undefined> {
    const { moduleId, generation } = snapshot;
    let state: Record<string, unknown> | null = null;
    row.phase = 'unloading';
    const removalProgress: ModuleLifecycleProgress = { moduleId, phase: 'unloading' };
    await this._startRemoval(moduleId, generation, {
      owner, progress: removalProgress,
      beforeUnload: async module => {
        const instance = module as Module;
        if (!instance.onSuspend) return;
        row.activeHook = 'on_suspend';
        try { state = await instance.onSuspend(); } finally { delete row.activeHook; }
      },
    }).completion;
    const beforePublish: BeforePublish = async (_, candidate) => {
      const instance = candidate as Module;
      if (state == null || !instance.onResume) return;
      row.phase = 'resuming';
      row.activeHook = 'on_resume';
      try {
        await this.lifecycleCallbacks.run([this, owner], () => instance.onResume!(state!));
      } catch (error) {
        console.warn(`[apcore:registry] onResume failed for ${moduleId}:`, error);
      } finally {
        delete row.activeHook;
      }
    };
    try {
      if (removalProgress.error) throw new ModuleError(removalProgress.error.code, 'Module unload failed');
      row.phase = 'loading';
      await this.discover({ reloadFiles: sourceFile === undefined ? [] : [sourceFile], beforePublish, managed: reservation });
      if (reservation.publishedVersion === undefined) throw new ModuleLoadError(moduleId, 'The selected replacement was not discovered');
      row.outcome = 'reloaded';
      row.compensation = 'not_required';
      return reservation.publishedVersion;
    } catch (error) {
      const phase = row.phase === 'unloading' ? 'unloading' : 'loading';
      const cause = phase === 'loading' && !(error instanceof ModuleError) ? new ModuleLoadError(moduleId, 'Replacement load failed') : error;
      row.error = lifecycleError(cause, phase);
      row.outcome = 'failed';
      row.compensation = 'failed';
      try {
        await this.restoreRegistration(snapshot, beforePublish, reservation);
        row.compensation = 'restored';
      } catch (compensationError) {
        console.warn(`[apcore:registry] Failed to restore '${moduleId}' after reload failed:`, compensationError);
      }
      this._emitLifecycleEvent(emitter, 'module_reload_failed', owner, row, 'error', {
        phase: row.error.phase, error_code: row.error.code, compensation: row.compensation,
      });
    } finally {
      row.phase = 'done';
      delete row.activeHook;
    }
  }

  private async _runManagedDrain(
    module: Module, generation: ExecutionGeneration, owner: LifecycleProgress, row: ModuleLifecycleProgress,
    policy: ManagedDrainPolicy, emitter: EventEmitter | null = this._eventEmitter,
  ): Promise<{ forced: boolean; abort?: DrainAbortReason }> {
    row.phase = 'draining';
    const reason = owner.report.kind === 'unregister_many' ? 'unregister' : owner.report.kind;
    const drain = generation.references > 0 ? this._waitForDrain(generation) : undefined;
    this._emitLifecycleEvent(emitter, 'module_drain_started', owner, row, 'info', {
      reason_kind: reason, drain_timeout_ms: policy.timeoutMs,
    });
    try {
      return await runManagedDrain({
        owner, callbackOwner: this, module, progress: row, policy,
        activeCount: () => generation.references,
        idle: drain?.completion ?? Promise.resolve(),
        cancelExecutions: () => { for (const token of generation.executions?.keys() ?? []) token.cancel(); },
        reportTimeout: activeCount => this._emitLifecycleEvent(emitter, 'module_drain_timed_out', owner, row, 'warn', {
          reason_kind: reason, action: policy.action, active_count: activeCount,
        }),
      });
    } finally {
      drain?.stop();
    }
  }

  private _emitLifecycleEvent(
    emitter: EventEmitter | null, name: string, owner: LifecycleProgress, row: ModuleLifecycleProgress,
    severity: ApCoreEvent['severity'], data: Record<string, unknown>,
  ): void {
    if (!emitter) return;
    try {
      this.lifecycleCallbacks.run([this, owner], () => emitter.emit(createEvent(`apcore.registry.${name}`, row.moduleId, severity, {
        operation_id: owner.report.operationId, ...data,
      })));
    } catch (error) {
      console.warn(`[apcore:registry] Lifecycle event ${name} failed:`, error);
    }
  }

  /**
   * Export the JSON Schema for a registered module.
   *
   * Returns a detached schema object (`Record<string, unknown> | null`),
   * matching Python's `dict | None` and Rust's `Option<Value>` return types.
   * Returns `null` if the module is not registered.
   * Use the standalone `exportSchema` function from `schema-export.ts` for
   * serialized (JSON/YAML) output.
   */
  exportSchema(
    moduleId: string,
    strict: boolean = false,
  ): Record<string, unknown> | null {
    const schema = getSchema(this, moduleId);
    if (schema === null) return null;
    if (strict) {
      schema['input_schema'] = toStrictSchema(schema['input_schema'] as Record<string, unknown>);
      schema['output_schema'] = toStrictSchema(schema['output_schema'] as Record<string, unknown>);
    }
    return schema;
  }

  clearCache(): void {
    this._schemaCache.clear();
  }

  // ── Safe Hot-Reload (F09 / Algorithm A21) ───────────────────────

  /**
   * Discover module IDs for the classes in a single file under multi-class
   * mode (PROTOCOL_SPEC §2.1.1).
   *
   * D-15: cross-language alignment with Python `Registry.discover_multi_class`
   * and the Rust trait method. Internally delegates to the free function
   * {@link discoverMultiClass} (re-exported as `_discoverMultiClass` for
   * scanner internals), so behaviour is identical.
   *
   * **apcore decision-log D-06**: the per-class `ClassDescriptor.multiClass`
   * flag is the sole opt-in mechanism for multi-class discovery. When any
   * qualifying class sets `multiClass: true`, the discovery routine derives
   * a distinct module ID per class; otherwise whole-file mode is used.
   *
   * @param filePath - Source file path (relative to project root).
   * @param classes - Class descriptors; each may carry `multiClass: true`
   *   to opt into per-class module ID derivation.
   * @param extensionsRoot - Extensions root directory (defaults to
   *   `'extensions'`).
   *
   * **Deprecated 4-arg overload**: passing a `multiClassEnabled` boolean as
   * the fourth argument is retained for backward compatibility through one
   * minor release and will be removed in v0.22.0. The argument is now
   * functionally inert — the per-class `multiClass` field is the source of
   * truth. A one-shot deprecation notice is emitted on first use.
   */
  discoverMultiClass(
    filePath: string,
    classes: readonly import('./multi-class.js').ClassDescriptor[],
    extensionsRoot?: string,
  ): import('./multi-class.js').MultiClassEntry[];
  /** @deprecated Pass `multiClass: true` on each `ClassDescriptor` instead. The `multiClassEnabled` argument will be removed in v0.22.0. */
  discoverMultiClass(
    filePath: string,
    classes: readonly import('./multi-class.js').ClassDescriptor[],
    extensionsRoot: string,
    multiClassEnabled: boolean,
  ): import('./multi-class.js').MultiClassEntry[];
  discoverMultiClass(
    filePath: string,
    classes: readonly import('./multi-class.js').ClassDescriptor[],
    extensionsRoot: string = 'extensions',
    multiClassEnabled?: boolean,
  ): import('./multi-class.js').MultiClassEntry[] {
    if (multiClassEnabled !== undefined) {
      warnMultiClassEnabledDeprecated();
      // The argument is functionally inert under D-06: the per-class
      // `multiClass` field is the source of truth. We intentionally ignore
      // `multiClassEnabled` and recompute from the descriptors below.
    }
    // The free function now resolves the per-class opt-in itself (spec
    // v1.50.0 D-107), so the boolean is not forwarded — forwarding it would
    // trip its own deprecation notice on every call through this method.
    return _discoverMultiClass(filePath, classes, extensionsRoot);
  }

  /**
   * Number of in-flight executions per module.
   */
  get refCount(): ReadonlyMap<string, number> {
    const counts = new Map<string, number>();
    for (const [id, generation] of this._executionGenerations) {
      if (generation.references > 0) counts.set(id, generation.references);
    }
    return counts;
  }

  /**
   * Acquire a reference to a module for execution.
   *
   * Increments the reference count. Throws ModuleNotFoundError if the
   * module is draining or not registered. Call {@link release} when done.
   */
  acquire(moduleId: string): unknown {
    const generation = this._acquireGeneration(moduleId);
    const references = this._manualReferences.get(moduleId) ?? new Map();
    references.set(generation, (references.get(generation) ?? 0) + 1);
    this._manualReferences.set(moduleId, references);
    return this._modules.get(moduleId);
  }

  private _acquireGeneration(moduleId: string): ExecutionGeneration {
    const generation = this._executionGenerations.get(moduleId);
    if (this._shutdown || generation === undefined || this._draining.has(moduleId)) {
      throw new ModuleNotFoundError(moduleId);
    }
    generation.references++;
    return generation;
  }

  /** @internal Hold an execution reference to the instance resolved by lookup. */
  acquireExecution(moduleId: string, expectedModule: unknown, token?: CancelToken | null): () => void {
    if (this._modules.get(moduleId) !== expectedModule) {
      throw new ModuleNotFoundError(moduleId);
    }
    const generation = this._acquireGeneration(moduleId);
    if (token) {
      generation.executions ??= new Map();
      generation.executions.set(token, (generation.executions.get(token) ?? 0) + 1);
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (token && generation.executions) {
        const count = generation.executions.get(token)!;
        if (count <= 1) generation.executions.delete(token);
        else generation.executions.set(token, count - 1);
      }
      this._releaseGeneration(generation);
    };
  }

  /** @internal Capture the registration now and acquire its execution reference later. */
  bindExecution(moduleId: string, expectedModule: unknown, token?: CancelToken | null): () => () => void {
    const expectedGeneration = this._executionGenerations.get(moduleId);
    return () => {
      if (this._executionGenerations.get(moduleId) !== expectedGeneration) {
        throw new ModuleNotFoundError(moduleId);
      }
      return this.acquireExecution(moduleId, expectedModule, token);
    };
  }

  /**
   * Release a previously acquired module reference.
   *
   * Decrements the reference count and notifies drain waiters when it
   * reaches zero.
   */
  release(moduleId: string): void {
    const references = this._manualReferences.get(moduleId);
    if (references === undefined) return;
    const first = references.entries().next().value;
    if (first === undefined) return;
    const [generation, count] = first;
    if (count === 1) references.delete(generation);
    else references.set(generation, count - 1);
    if (references.size === 0) this._manualReferences.delete(moduleId);
    this._releaseGeneration(generation);
  }

  private _releaseGeneration(generation: ExecutionGeneration): void {
    generation.references--;
    if (generation.references === 0) {
      for (const resolve of generation.drainWaiters) resolve();
    }
  }

  /**
   * Check whether a module is marked for unload (draining).
   */
  isDraining(moduleId: string): boolean {
    return this._draining.has(moduleId) || (
      !this._executionGenerations.has(moduleId) &&
      (this._latestRemoval(moduleId)?.generation.references ?? 0) > 0
    );
  }

  /** @internal Whether this registry has stopped admitting executions. */
  get isClosed(): boolean {
    return this._shutdown !== undefined;
  }

  /**
   * Mark a module as draining so no new acquire() calls are accepted.
   */
  beginDrain(moduleId: string): void {
    this._draining.add(moduleId);
  }

  /**
   * Remove a draining mark set by beginDrain. Existing references remain valid.
   */
  endDrain(moduleId: string): void {
    if (!this._managedReloads.has(moduleId) && !this._unavailable.has(moduleId)) this._draining.delete(moduleId);
  }

  /**
   * Wait until all in-flight executions of a module have completed.
   *
   * Returns a promise that resolves to `true` if the module drained
   * cleanly, or `false` if the timeout was reached.
   */
  waitDrained(moduleId: string, timeoutMs: number = 5000): Promise<boolean> {
    const generation = this._executionGenerations.get(moduleId) ?? this._latestRemoval(moduleId)?.generation;
    if (generation === undefined || generation.references === 0) return Promise.resolve(true);
    const drain = this._waitForDrain(generation);
    drain.setDeadline(timeoutMs);
    return drain.completion.then((forcedAfterMs) => forcedAfterMs === undefined);
  }

  /** Wait indefinitely unless the removal owner or shutdown sets a deadline. */
  private _waitForDrain(generation: ExecutionGeneration): {
    completion: Promise<number | undefined>;
    setDeadline: (timeoutMs: number) => void;
    stop: () => void;
  } {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let finish!: (forcedAfterMs?: number) => void;
    const drained = (): void => { finish(); };
    const completion = new Promise<number | undefined>((resolve) => {
      finish = (forcedAfterMs): void => {
        if (timer !== undefined) clearTimeout(timer);
        generation.drainWaiters.delete(drained);
        resolve(forcedAfterMs);
      };
    });
    generation.drainWaiters.add(drained);
    return {
      completion,
      stop: drained,
      setDeadline: (timeoutMs): void => {
        if (timer !== undefined || !generation.drainWaiters.has(drained)) return;
        timer = setTimeout(() => finish(timeoutMs), timeoutMs);
      },
    };
  }

  // ── Ephemeral namespace pilot (protocol-spec §2.5.1) ──────

  /**
   * Wire an EventEmitter for registry-side audit emits.
   *
   * Per protocol-spec §2.5.1 "Audit events", `ephemeral.*` registrations and
   * unregistrations emit a single `apcore.registry.module_registered` /
   * `apcore.registry.module_unregistered` event with the D-35 contextual
   * payload. When no emitter is wired, the event is logged at INFO so it
   * never silently disappears — useful for the v1 pilot where most
   * callers will not have an emitter attached.
   *
   * Pilot scope: only `ephemeral.*` registrations trigger registry-side
   * emits. The pre-existing bridge in
   * `sys-modules/registration.ts` continues to emit empty-payload events
   * for non-ephemeral modules and short-circuits for ephemerals so the
   * single-emit rule holds.
   */
  setEventEmitter(emitter: EventEmitter | null): void {
    this._eventEmitter = emitter;
  }

  /**
   * Soft-warn when an `ephemeral.*` module is registered without
   * `requiresApproval: true`. Per protocol-spec §2.5.1,
   * agent-synthesized modules SHOULD declare `requiresApproval` so a
   * human gates execution. The registry only warns; it does not refuse
   * the registration.
   */
  private _warnIfMissingApproval(
    moduleId: string,
    module: unknown,
    metadata?: Record<string, unknown> | null,
  ): void {
    // The two sources are unioned (PROTOCOL_SPEC §7.4, D-96) because that is
    // what the approval gate reads: a requirement declared in the `metadata` /
    // `*_meta.yaml` source gates the module just as a code-level one does.
    // Reading the instance alone advised an author to set something they had
    // already set, in the one place the registry itself records it — an
    // advisory that fires against correct configuration is one operators learn
    // to ignore.
    const effective = governanceUnion(
      (module as { annotations?: unknown })?.annotations,
      metadata?.['annotations'],
    );
    if (effective === null || !effective.requiresApproval) {
      console.warn(
        `[apcore:registry] ephemeral.* module '${moduleId}' registered without ` +
        `requiresApproval=true. The apcore protocol-spec §2.5.1 ` +
        `recommends setting ModuleAnnotations.requiresApproval=true so ` +
        `agent-synthesized code does not run unattended.`,
      );
    }
  }

  /**
   * Build the D-35 audit payload (`caller_id` plus optional `identity`
   * snapshot) for an ephemeral.* register/unregister. Mirrors the shape
   * `extractAuditIdentity` produces for `system.control.*` events so
   * subscribers can apply identical redaction rules.
   */
  private _buildEphemeralAuditPayload(context: Context | null): Record<string, unknown> {
    const callerIdRaw = context?.callerId;
    const callerId = callerIdRaw == null || callerIdRaw === '' ? '@external' : callerIdRaw;
    const ident = context?.identity ?? null;
    if (!ident) {
      return { caller_id: callerId, identity: null, namespace_class: 'ephemeral' };
    }
    const snapshot: Record<string, unknown> = {
      id: ident.id,
      type: ident.type,
      roles: [...ident.roles],
    };
    const displayName = (ident.attrs as Record<string, unknown>)['display_name'];
    if (typeof displayName === 'string' && displayName.length > 0) {
      snapshot['display_name'] = displayName;
    }
    return { caller_id: callerId, identity: snapshot, namespace_class: 'ephemeral' };
  }

  /**
   * Emit `apcore.registry.module_load_failed` when onLoad raises (spec #65).
   * Delivered synchronously if the emitter is wired; logged as warn otherwise.
   */
  private _emitModuleLoadFailed(moduleId: string, err: unknown): void {
    const error = err instanceof Error ? err : new Error(String(err));
    const payload: Record<string, unknown> = {
      module_id: moduleId,
      callback_name: 'module.onLoad',
      error_type: error.constructor?.name ?? 'Error',
      error_message: error.message,
      timestamp: new Date().toISOString(),
    };

    const emitter = this._eventEmitter;
    if (emitter === null) {
      console.warn(
        `[apcore:registry] module_load_failed module_id=${moduleId} ` +
        `error_type=${payload['error_type']} message=${error.message}`,
      );
      return;
    }

    const event: ApCoreEvent = {
      eventType: 'apcore.registry.module_load_failed',
      moduleId,
      timestamp: new Date().toISOString(),
      severity: 'error',
      data: payload,
    };
    try {
      emitter.emit(event);
    } catch (emitErr) {
      console.error('[apcore:registry] Failed to emit module_load_failed:', emitErr);
    }
  }

  /**
   * Emit the canonical ephemeral.* audit event to the wired EventEmitter
   * (or warn-log if none is wired). The bridge in
   * `sys-modules/registration.ts` short-circuits on ephemeral.* IDs so
   * exactly one event lands per registration / unregistration.
   */
  private _emitEphemeralAudit(
    eventType: string,
    moduleId: string,
    context: Context | null,
  ): void {
    let payload: Record<string, unknown>;
    try {
      payload = this._buildEphemeralAuditPayload(context);
    } catch (e) {
      console.warn(
        `[apcore:registry] Failed to extract audit payload for ephemeral '${moduleId}': ${(e as Error).message}. ` +
        `Falling back to caller_id='@external'.`,
      );
      payload = { caller_id: '@external', identity: null, namespace_class: 'ephemeral' };
    }

    const emitter = this._eventEmitter;
    if (emitter === null) {
      console.info(
        `[apcore:registry] ephemeral audit event ${eventType} module_id=${moduleId} ` +
        `payload=${JSON.stringify(payload)} (no EventEmitter wired; call ` +
        `Registry.setEventEmitter to capture)`,
      );
      return;
    }

    const event: ApCoreEvent = {
      eventType,
      moduleId,
      timestamp: new Date().toISOString(),
      severity: 'info',
      data: payload,
    };
    try {
      emitter.emit(event);
    } catch (e) {
      console.error(
        `[apcore:registry] EventEmitter.emit failed for ephemeral audit event ` +
        `${eventType} on '${moduleId}':`, e,
      );
    }
  }

  /** Drain and remove the current registration. An absent or already detached ID returns false. */
  safeUnregister(moduleId: string, timeoutMs?: number): Promise<boolean>;
  /** @internal Reload can capture state after drain and before resource release. */
  safeUnregister(moduleId: string, timeoutMs?: number, beforeUnload?: RemovalOptions['beforeUnload']): Promise<boolean>;
  safeUnregister(
    moduleId: string,
    timeoutMs: number = 5000,
    beforeUnload?: RemovalOptions['beforeUnload'],
  ): Promise<boolean> {
    if (this._shutdown?.force === false || this._managedReloads.has(moduleId)) return Promise.resolve(false);
    const generation = this._executionGenerations.get(moduleId);
    if (generation === undefined) return Promise.resolve(false);
    return this._startRemoval(moduleId, generation, { forceAfterMs: timeoutMs, beforeUnload }).completion;
  }

  private _latestRemoval(moduleId: string): ModuleRemoval | undefined {
    const removals = this._removals.get(moduleId);
    return removals === undefined ? undefined : Array.from(removals).at(-1);
  }

  private _startRemoval(
    moduleId: string,
    generation: ExecutionGeneration,
    options: RemovalOptions,
  ): ModuleRemoval {
    const sourceFile = this._sourceFiles.get(moduleId);
    const module = this._detach(moduleId);
    const drain = !options.managed && generation.references > 0 ? this._waitForDrain(generation) : undefined;
    const progress = options.progress ?? this._shutdown?.rows.get(generation) ?? { moduleId, phase: 'pending' };
    progress.phase = options.managed || drain ? 'draining' : 'unloading';
    let finish!: (clean: boolean) => void;
    const completion = new Promise<boolean>((resolve) => { finish = resolve; });
    const removal: ModuleRemoval = {
      generation, completion, module, sourceFile, progress,
      setDrainDeadline: drain?.setDeadline,
    };
    const removals = this._removals.get(moduleId) ?? new Set();
    removals.add(removal);
    this._removals.set(moduleId, removals);

    const unload = (forcedAfterMs?: number): void => this._unloadRemoval(removal, removals, options, forcedAfterMs, finish);

    if (options.managed && options.owner) {
      const { managed, owner } = options;
      void this._runManagedDrain(module as Module, generation, owner, progress, managed)
        .then(({ forced }) => unload(forced ? managed.timeoutMs : undefined));
    } else if (drain === undefined) unload();
    else {
      if (options.forceAfterMs !== undefined) drain.setDeadline(options.forceAfterMs);
      void drain.completion.then(unload);
    }
    return removal;
  }

  private _unloadRemoval(
    removal: ModuleRemoval,
    removals: Set<ModuleRemoval>,
    options: RemovalOptions,
    forcedAfterMs: number | undefined,
    finish: (clean: boolean) => void,
  ): void {
    const { progress, generation, module } = removal;
    const { moduleId } = progress;
    const owners = options.owner ? [this, options.owner] : [this];
    progress.phase = 'unloading';
    if (forcedAfterMs !== undefined) {
      progress.drainTimedOut = true;
      progress.activeAtUnload = generation.references;
      console.warn(
        `[apcore:registry] Force-unloading module ${moduleId} after ${forcedAfterMs}ms timeout ` +
        `(${generation.references} in-flight executions)`,
      );
    }

    const finishRemoval = (): void => {
      removals.delete(removal);
      if (removals.size === 0) this._removals.delete(moduleId);
      progress.phase = 'done';
      finish(forcedAfterMs === undefined);
    };
    const completeUnload = (): void => {
      delete progress.activeHook;
      this.lifecycleCallbacks.run(owners, () => this._notifyUnregister(moduleId, module, options.context));
      progress.outcome = progress.error ? 'failed' : forcedAfterMs === undefined ? 'removed' : 'forced_removed';
      if (forcedAfterMs !== undefined && generation.references > 0) {
        progress.cleanupPending = true;
        void this._waitForDrain(generation).completion.then(() => {
          progress.cleanupPending = false;
          finishRemoval();
        });
        if (!options.managed) {
          // Compatibility waits for unload, while managed shutdown can still join the retiring generation.
          progress.phase = 'done';
          finish(false);
        }
        return;
      }
      finishRemoval();
    };
    const unload = (): void => {
      progress.activeHook = 'on_unload';
      const cleanup = this.lifecycleCallbacks.run(owners, () => this._runOnUnload(moduleId, module, error => {
        progress.error ??= lifecycleError(error, 'unloading');
      }));
      if (cleanup === undefined) completeUnload();
      else void cleanup.then(completeUnload);
    };

    if (forcedAfterMs === undefined && options.beforeUnload) {
      const warn = (error: unknown): void => { console.warn(`[apcore:registry] Pre-unload hook failed for ${moduleId}:`, error); };
      try {
        const before = this.lifecycleCallbacks.run(owners, () => options.beforeUnload!(module));
        if (before !== undefined) {
          void before.catch(warn).then(unload);
          return;
        }
      } catch (error) {
        warn(error);
      }
    }
    unload();
  }

  /** @internal Start or join teardown for APCore and its client-owned event stage. */
  startShutdown(options: Omit<ShutdownOptions, 'eventFlushTimeoutMs'> = {}): LifecycleOperation {
    validateLifecycleOptions(options);
    if (this._shutdown) {
      if (options.drainTimeoutMs !== undefined && options.drainTimeoutMs !== this._shutdown.timeoutMs) {
        throw new InvalidInputError('drainTimeoutMs conflicts with the active registry shutdown');
      }
      if (options.onDrainTimeout !== undefined && options.onDrainTimeout !== this._shutdown.policy?.action) {
        throw new InvalidInputError('onDrainTimeout conflicts with the active registry shutdown');
      }
      if (options.cancelGraceMs !== undefined && options.cancelGraceMs !== this._shutdown.policy?.cancelGraceMs) {
        throw new InvalidInputError('cancelGraceMs conflicts with the active registry shutdown');
      }
      return this._shutdown.lifecycle.operation;
    }
    const { drainTimeoutMs = 5_000, onDrainTimeout = 'continue_waiting', cancelGraceMs = 1_000 } = options;
    return this._startShutdown(drainTimeoutMs, { timeoutMs: drainTimeoutMs, action: onDrainTimeout, cancelGraceMs }).lifecycle.operation;
  }

  /** @internal Stop discovery and drain every registration, including hidden modules. */
  shutdown(timeoutMs: number = 5000): Promise<void> {
    return (this._shutdown ?? this._startShutdown(timeoutMs)).completion;
  }

  private _startShutdown(timeoutMs: number, policy?: ManagedDrainPolicy): RegistryShutdown {
    const force = policy === undefined;
    const rows = new Map<ExecutionGeneration, ModuleLifecycleProgress>();
    for (const [moduleId, generation] of this._executionGenerations) rows.set(generation, { moduleId, phase: 'pending' });
    const removals = new Set(Array.from(this._removals.values()).flatMap(entries => [...entries]));
    for (const removal of removals) rows.set(removal.generation, removal.progress);
    const lifecycle = new LifecycleProgress('shutdown', [...rows.values()], [this], this.lifecycleCallbacks);
    const completion = lifecycle.run(async () => {
      this.unwatch();
      this._scheduleShutdownRemovals(lifecycle, rows, removals, timeoutMs, policy);
      await Promise.all([...this._managedReloads.values()].map(reservation => reservation.settled.wait()));
      await Promise.allSettled(this._inFlight.values());
      this._scheduleShutdownRemovals(lifecycle, rows, removals, timeoutMs, policy);
      await Promise.all([...removals].map(removal => removal.completion));
      if (!force) {
        await Promise.all([...removals]
          .filter(removal => removal.generation.references > 0)
          .map(removal => this._waitForDrain(removal.generation).completion));
      }
    });
    const shutdown: RegistryShutdown = { lifecycle, completion, timeoutMs, force, policy, rows };
    this._shutdown = shutdown;
    return shutdown;
  }

  private _scheduleShutdownRemovals(
    lifecycle: LifecycleProgress,
    rows: Map<ExecutionGeneration, ModuleLifecycleProgress>,
    joined: Set<ModuleRemoval>,
    timeoutMs: number,
    policy?: ManagedDrainPolicy,
  ): void {
    for (const [id, generation] of this._executionGenerations) {
      if (this._managedReloads.has(id)) continue;
      const row = rows.get(generation) ?? { moduleId: id, phase: 'pending' };
      if (!rows.has(generation)) lifecycle.report.modules.push(row);
      rows.set(generation, row);
      joined.add(this._startRemoval(id, generation, {
        progress: row, owner: lifecycle, ...(policy ? { managed: policy } : { forceAfterMs: timeoutMs }),
      }));
    }
    for (const removals of this._removals.values()) {
      for (const removal of removals) {
        if (!policy) removal.setDrainDeadline?.(timeoutMs);
        if (joined.has(removal)) continue;
        joined.add(removal);
        if (!rows.has(removal.generation)) {
          lifecycle.report.modules.push(removal.progress);
          rows.set(removal.generation, removal.progress);
        }
      }
    }
  }

  private _assertOpen(): void {
    if (this._shutdown) throw new InvalidInputError('Registry has been shut down');
  }
}
