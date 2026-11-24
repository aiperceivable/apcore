# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Expose `APCore({ failOnError: true })` and `sysModulesErrors` for system-module startup, and `Executor.ungatedApprovalModules()` for approval readiness. ([#230](https://github.com/aiperceivable/apcore/issues/230), [#231](https://github.com/aiperceivable/apcore/issues/231))

### Changed

- Resolve relative configuration paths against the `Config` project root across all filesystem consumers. The root stays stable across working-directory changes and reloads. ([#113](https://github.com/aiperceivable/apcore/issues/113))

### Fixed

- Send governance events through the enabled APCore event bus and normalize typed module results consistently across calls and streams. ([#125](https://github.com/aiperceivable/apcore/issues/125), [#126](https://github.com/aiperceivable/apcore/issues/126))


## [0.33.0] - 2026-11-18

### Added

- `APCore.module()` / `FunctionModule` / `module()` accept a first-class
  `resources` block (`ResourceLimits`), with `timeout` in milliseconds. The
  executor already read `resources.timeout`; the high-level factory used to
  drop it, so a declared budget silently fell back to
  `executor.default_timeout` (30 s by default).
- Unknown module options now warn at registration. Timeout-like keys
  (`timeoutSec`, `timeout_ms`, …) get a message naming the supported
  `resources.timeout` field instead of disappearing silently.

### Security

- Upgrade the runtime dependencies `js-yaml` to `^4.3.2` and `uuid` to `^11.1.1`.

### Fixed

- Read a module's budget from `annotations.extra.resources.timeout` as well as the
  first-class resources field, matching Python and Rust.
- `MODULE_TIMEOUT` raised by a module call is no longer retryable by default.
  It carries `retryable: true` only when the module declares
  `annotations.idempotent: true`. This includes deadlines that expire after a
  stream has emitted chunks. Step-budget timeouts keep the spec default.

### Changed

- Refresh the development toolchain to `vitest` / `@vitest/coverage-v8` 5.1.0.
  No other runtime API changes.

## [0.32.3] - 2026-10-25

### Added

- Add `CancelToken.drain()` so lifecycle owners can await actual module execution and asynchronous finalizers, including descendant cancellation scopes. Ordinary cancellation and executor deadlines remain prompt.

### Fixed

- `AsyncTaskManager.shutdown()` now joins owned workers even when their task records are already cancelled or reclaimed, and reports task-store errors only after local cleanup settles.
- Retain concurrency slots until module execution and finalizers finish, preventing another task or retry from overlapping deferred cleanup.
- Publish task failure and retry-pending status before awaiting physical cleanup, so timeout errors and task outcomes remain promptly observable.
- `ReaperHandle.stop()` now waits for an in-flight sweep before resolving, preventing store cleanup from continuing after the reaper is reported stopped.

## [0.32.2] - 2026-10-19

### Fixed

- Preserve optional enum, const and composed-schema constraints during A23 strict conversion by wrapping the complete schema in a nullable union. Required fields remain unchanged, null-only types are not duplicated, and shared conformance cases verify both schema parity and value validation. Tracked in [apcore#124](https://github.com/aiperceivable/apcore/issues/124).

## [0.32.1] - 2026-10-14

### Security

- Resolve local JSON Schema references before sensitive-field redaction, including pointers, anchors, root and embedded resource identifiers, scoped anchors, recursive schemas, reference chains, positional/legacy tuple arrays and combinator branches. Referenced sensitive values no longer leak through executor captures or SDK consumers such as MCP routers. Resolution is cycle-safe and never retrieves external schemas or files; public redaction signatures and source data remain unchanged.

### Fixed

- Fix existing lint errors by expressing async-handler detection with a callable signature and making the test event emitter's void override explicit; runtime behavior is unchanged.

## [0.32.0] - 2026-10-08

### Added

- **`canonicalizeName()` is exported from both Node and browser entry points**, with `CanonicalNameResult` and four structured `CanonicalNameError` diagnostics. It normalizes one ASCII bare name without throwing, inventing prefixes, truncating, or changing Algorithm A02; shared fixtures align the Python, TypeScript, and Rust APIs.

### Security

- **An executor's ACL, approval handler and policy are bound into the built-in gates of whatever strategy it runs** (D-129) — including a pre-built instance, a registered strategy name and a per-call `callWithTrace` strategy; `governanceState()` reports what the running gate holds.
- **`acl_check` / `approval_gate` cannot be weakened** (D-130) — `ignore_errors: true`, any `match_modules` and `pure: true` on `approval_gate` raise `PIPELINE_CONFIGURATION_ERROR`, from `pipeline.configure` and from every step-entry API (`configureStep`, `replace`, `insertAfter`/`insertBefore`, the `ExecutionStrategy` constructor).
- **`LoggingMiddleware` and `ObsLoggingMiddleware` log only `context.redactedInputs` / `redactedOutput`, never the raw values** (D-131) — `x-sensitive` now survives JSON Schema conversion, so it is honoured at the capture point, and schemaless inputs/outputs are captured with the configured rules.

### Fixed
- **CI verifies declared canonical fixtures before running the full suite**, with an explicit spec `main` checkout independent of SDK branches. An incomplete snapshot reports every missing declared fixture and instructs publishing spec changes before SDK changes; missing fixtures and cases are never skipped or replaced with private copies.
- **Timeouts cancel only the timed-out call's linked cancellation scope** (D-133); parent cancellation propagates down, child deadlines do not cancel parents, and abort-listener races still report `MODULE_TIMEOUT` immediately.
- **Preflight collects all evaluable pure checks and preserves earlier results** (D-134), with module-authored hooks withheld after ACL denial; null previews omit their check and `predictedChanges` is always an array (D-141).
- **Binding declarations reject unknown keys, incomplete schema pairs, invalid inference modes and uninferable targets** (D-139), including implicit inference.
- **Namespace environment overrides dispatch once to the longest prefix** (D-146); the exact `APCORE` prefix is reserved for the framework.
- **MCP approval and streaming flags appear only when true in `_meta`** (D-140); schema extension stripping preserves property and definition names.
- **Schema diagnostics use exact `path`, `keyword`, `message` fields and RFC 6901 pointers** (D-149), with required-property failures located at the containing object.
- **Bare `ephemeral` registration emits one contextual event and privileged registration rejects ephemeral IDs with `INVALID_MODULE_ID`** (D-148).
- **Configured event subscribers honor their `circuit_breaker` block**, including default circuit-breaker wrapping.
- **Asynchronous watcher initialization respects `unwatch()` and concurrent startup**, preventing leaked filesystem listeners; Prometheus shutdown drains idle connections and HTTP lifecycle tests isolate ephemeral sockets.
- **`APCore.register()` returns the registry's `Promise<void>`** (#123) — it settles once an async `onLoad` completes and rejects with its error; validation errors are still thrown synchronously.
- **`TraceContext.inject` throws `InvalidParentIdError`** (#123) — a `ModuleError` with `code: INVALID_PARENT_ID`, `userFixable: true`, `retryable: false` and `toJSON()`, exported from both entry points.
- **`Registry.watch()` calls a changed or deleted module's `onUnload` once** (#123).
- **A bulk (`path_filter`) reload fails with `RELOAD_FAILED` when a module is missing after re-discovery** (#123, D-112 rule 4) — the missing modules are restored, modules that reloaded stay reloaded, and no audit entries are written for a failed operation.
- **`useBefore` / `useAfter` key each wrapper's identity on its callback** (#123) — a second distinct callback no longer triggers the duplicate-middleware warning; the same callback registered twice still does.
- **`ExecutionStrategy.replace()` rejects a new step name that belongs to a different step** with `StepNameDuplicateError` (#123), as `configureStep` and the insert methods do.
- **Step middleware can be attached through `Executor.addStepMiddleware()` / `APCore.addStepMiddleware()`** (#123, protocol-spec §5.16 requirement 5); `Executor.stepMiddlewares` lists them.

## [0.31.0] - 2026-09-22

### Added

- **A configured `pipeline:` section is applied** (D-72) — `remove:`/custom steps in `apcore.yaml` previously had no effect; also gained a synchronous strategy path.
- **The five `observability.tracing.*` keys are wired** (D-68) — `enabled`/`exporter`/`strategy`/`samplingRate`/`otlpEndpoint` now install and configure `TracingMiddleware`.
- **`ACL.load(path, auditLogger?)`** (D-66).
- **`extensions.roots` is read**, both element shapes, with namespace isolation (D-70).
- **`id_map.overrides` is read at registry construction** (D-71).
- **The six `validation.*` limits are wired**, unconstrained by default (#118) — purely additive.
- **`BatchSpanProcessor.forceFlush(timeoutMs = 30000): Promise<boolean>`** (A-C-001) — mirrors apcore-python; previously no way to drain the queue before exit.

### Changed

- **`AsyncTaskManager.startReaper` throws `ModuleError(REAPER_ALREADY_RUNNING)` instead of a bare `Error`** (A-C-005). **Breaking:** catch `ModuleError` instead.
- **The `config_bus.get.input.key.empty` contract test asserted a rule D-74 deleted** (D-74).
- **The deprecation-warning dedupe key now carries the notice** (D-89, v1.59.0) — a changed `x-deprecation` block now re-warns; an unchanged one on hot-reload does not.
- **An unknown extension point throws `InvalidInputError`, not a bare `Error`** (D-108). **Breaking.**
- **A throwing ACL audit callback no longer changes the access decision** (D-66) — auditing must not hold a veto over access.
- **`_config.allow_unknown: false` now drops unregistered namespaces** (D-69) — only affects configs that explicitly set it.
- **The README no longer presents inert or nonexistent configuration as working** — corrected examples, documented the ten inert keys, and the reverse trap (undeclared keys rejected under `_config.strict`).

### Deprecated

- **`Registry.get(moduleId, versionHint)`** — the version hint is ignored and now warns once per module ID; removed at 2.0 (D-126).
- **`acl.default_effect` in `apcore.yaml`** — reaches nothing; the ACL file's own `default_effect` is authoritative. Removed at v2.0 (D-73).
- **`Context.logger`** — no configuration door; use the host application's own logger. Removed at v2.0 (#121). This SDK memoises the logger per `Context`; Python/Rust rebuild it per access.
- **`logging.level`/`logging.format`**, no replacement (D-67).
- **Ten declared config keys reach no consumer in any SDK** — now warn once per load instead of silently doing nothing (#118).
- **An `audit:` block at the root of an ACL YAML file has never been read by any SDK** — the same settings live under `acl.audit.*`, and one of the two declarations is removed no earlier than v2.0.

### Fixed

- **A symlinked module file is discovered once, under its real path** (D-127) — also fixed a containment-check + `id_map.overrides` canonicalization bug on the same root cause.
- **A malformed annotation value (`extra`/`cacheTtl`) is dropped with a warning** instead of surviving or crashing unclamped (D-115).
- **`system.manifest.full` reports `project_name` correctly** (D-110).
- **An empty `roles` list is now omitted from the audit identity snapshot** instead of always emitted (D-118).
- **`TaskStoreError`/`TASK_STORE_UNAVAILABLE` now exist and are framework-reserved** (D-92).
- **`shutdown()` now stops the reaper before cancelling, not after** — closes a window where a sweep could delete records the cancel loop was walking.
- **SECURITY: a governance requirement declared in module metadata no longer bypasses the approval gate** (D-125).
- **SECURITY: a `$ref` into an external schema could bind to the caller's local definitions**, including past an `x-sensitive` marking (D-124) — fallback is now scoped to the declaring document.
- **`file_changed` is now a subscribable registry event** — every hot-reload notification was previously silently discarded (D-80).
- **`Executor.call()` enforces the module-ID length bound at the entry guard**, instead of reaching the registry and reporting `MODULE_NOT_FOUND` (D-75).
- **`Registry.describe()` no longer returns a non-string through its `: string` signature** (D-77).
- **A stalled topological sort is no longer misreported as a circular dependency** (D-79).
- **A non-positive call-chain limit raises a typed apcore error** instead of a bare `Error` (D-84).
- **`AsyncTaskManager.shutdown()` now propagates a store write failure** instead of absorbing it (D-81).
- **`Executor.listStrategies()` is now an instance method** and includes the executor's current strategy.
- **The custom-discoverer registration path now reserves the module ID for the whole of `onLoad`**, closing a re-entrant-registration race.
- **`AsyncTaskManager.submit()` now serializes admission**, so concurrent submits cannot exceed `maxTasks`.
- **`cancel()` no longer returns `false` for an active task with no in-process handle** — repairs `shutdown()`'s postcondition.
- **The task runner now re-reads the stored status before every write**, so an out-of-band cancel is no longer overwritten.
- **In-process task handles are now pruned when a task settles**, instead of leaking for the process lifetime.
- **`CancelToken` now keeps one `AbortController` for its lifetime** — `reset()` no longer detaches a holder of the signal.
- **`ModuleDescriptor.sunsetDate` is now derived from the `x-deprecation` block**, and a deprecated module now warns.
- **`register()` now runs a custom validator before duplicate detection**, matching Python/Rust.
- **Added missing test coverage for `TPreviewResult` and `ObservabilityStore.getErrors`/`getMetrics`.**
- **`sys_modules.control.overrides_path` now persists a toggle, not just restores one** (SYS-2).
- **The client's `Registry` is now the executor's, and is built from the `Config`** (CLI-1, CLI-3).
- **`global_deadline` is now a correctly-scoped epoch-seconds value** (D-99/D-100/D-101) — three related defects in one field.
- **YAML annotation overrides now match the wire (snake_case) spelling** (MOD-001) — five of thirteen fields, including `requires_approval`, were previously silently ignored.
- **A local `#/…` schema reference resolves against the file root first**, then falls back to the schema node (D-104).
- **The exported `discoverMultiClass` now honours the per-class marker** instead of a withdrawn file-level toggle (D-107).
- **The after-middleware chain now runs for a module that returns nothing** (MW-001).
- **`registerInternal` now enforces the streaming-annotation check** (STR-5).
- **Stream Phase 3 now actually rethrows a cancellation** instead of swallowing it (STR-4/CAN-002).
- **Span correlation attributes are now snake_case**, matching Python/Rust (OBS-003).
- **Metric series are now keyed on their labels** rather than a string a label value could forge (OBS-006).
- **Three framework error codes the A17 collision guard could not see are now covered** (ERR-002).
- **`CircuitBreakerWrapper` now forwards the wrapped subscriber's identity** instead of widening a filtered subscriber to catch-all (EVT-001).
- **Four `system.*` wire/annotation divergences fixed** (SYS-5, SYS-8, SYS-13, SYS-19).
- **Four pattern-valued config surfaces previously used inconsistent glob dialects** — unified under Algorithm A25 (#116/#117).
- **`Config.validate()` now reports an uncompilable `obs.redaction.regex_patterns` entry** (#117).
- **`stream.max_merge_depth` is now read from config** instead of ignored (#118).
- **The four per-group `sys_modules.*` registration flags are now read**, `control.enabled` included (#118).

### Removed

- **The `@types/uuid` devDependency** — `uuid@11` ships its own declarations.
- **The internal `fnmatch(text, pattern)` passthrough in `events/retry.ts`** — a reversed-argument-order wrapper around `matchGlob`.

---

## [0.30.0] - 2026-09-06

### Added

- **`Config.projectRoot` — the base a relative path-typed value is *about* ([apcore#113](https://github.com/aiperceivable/apcore/issues/113), spec §9.2.2).** §9.1.1 gives four keys a relative default (`extensions.root`, `schema.root`, `acl.root`, `bindings.dir`), §9.2.1 declares the closed path-typed set, and nothing said what a relative value is relative *to* — which is why `APCORE_ACL_ROOT=./x` resolves against the config file's directory while `APCORE_SCHEMA_ROOT=./x` resolves against the CWD, in shipped code, with neither behaviour violating the spec. `projectRoot` is the first half of the answer: the directory of the config file when it came from a §9.14 tier 1-5 source (explicit `$APCORE_CONFIG_FILE`, a path passed to `Config.load`, or a project-local `./project.yaml|.yml` / `./apcore.yaml|.yml`), and the process CWD when it came from tier 6-7 (`~/.config/apcore/config.yaml`, `~/.apcore/config.yaml`) or when no config file was found. Always absolute.

  **The tier split is the point.** For tiers 2-5 — the overwhelmingly common case — the config file's directory *is* the CWD and the two candidate bases coincide. They diverge for a config explicitly pointed at from elsewhere, where the file's directory is the better answer, and for a **user-level** config, where it is the wrong one: `acl.root: ./acl` written in `~/.config/apcore/config.yaml` means "this project's `acl/`", never `~/.config/apcore/acl/`. That second case is live today in `acl.root`, the one key already resolving file-relative — a user-level config silently supplies an ACL policy to every project that user runs while the project's own `./acl/` is ignored, the inverse of what a default-deny system is for.

  The §9.14 user-level paths are also exported as `userLevelConfigPaths()`, split out of `discoverConfigFile` so the two answers cannot drift.

- **`bindings.dir` and `bindings.pattern` are canonical defaults ([apcore#115](https://github.com/aiperceivable/apcore/issues/115), spec §5.12.6 / §9.1.1).** `schemas/defaults.schema.json` gained a `bindings` section it never had, so `DEFAULTS` in `src/config-defaults.ts` now carries `bindings.dir: './bindings'` and `bindings.pattern: '*.binding.yaml'` — twenty canonical defaults, checked key for key against the canonical schema by `conformance/fixtures/config_key_governance.json`. Until this landed the `./bindings` default was unreachable from the mechanism meant to serve it: `config.get('bindings.dir')` answered `undefined` for a key the spec gives a canonical value, and every consumer had to carry its own literal.

  **`loadBindingDir` keeps its `'./bindings'` / `'*.binding.yaml'` literals**, deliberately. Its `config` parameter is optional and typed `BindingConfigLike` — any object with a `get` method — so a loader invoked with no configuration at all, or with a configuration source that is not this SDK's `Config`, has nothing to read the default from. The literals are the loader's own last-resort tier, not a duplicate of the table; `tests/test-bindings-config-dir.test.ts` pins both no-`Config` paths (`./bindings` is used, and a set `APCORE_BINDINGS_DIR` is still not read at the loader).

  Consequence for the §13.2 notice below: `bindings.dir` now sits in the merged configuration of every load, so a configuration that spells `extensions.root`, `schema.root` and `acl.root` absolutely but leaves `bindings.dir` unstated still carries a relative path-typed value and still warns. Spelling all four absolutely is what silences it.

### Deprecated

- **Notice that relative path-typed values will one day resolve against one base ([apcore#113](https://github.com/aiperceivable/apcore/issues/113), spec §13.2).** `Config.load` now warns **once per configuration load** when `projectRoot` differs from the CWD **and** the configuration carries at least one relative path-typed value — naming the affected keys, the project root and the CWD. Both conditions have to hold for the coming change to move anything; a blanket warning on every out-of-tree config would train everyone to ignore it, and apcore#113 rules it out explicitly. In the ordinary tier 2-5 project the first condition is false and nothing is printed. Relative values inherited from the `DEFAULTS` table count, because those re-root under a new base just as a written value does; `extensions.roots` counts when any element is relative, in either its bare-string or `{ root, namespace }` form, and is reported under the `extensions.roots[]` key §9.2.1 gives it.

  **No resolution behaviour changes in this release.** `SchemaLoader` still resolves `schema.root` against the CWD and `ACL.discover` still resolves `acl.root` against the config file's directory for every tier, both pinned by test so a premature adoption of the new base cannot land silently. Adopting `projectRoot` as *the* base for every path-typed key is a behaviour change to deployed configurations and therefore a major-version move; this is its deprecation phase.

  **Cadence: once per configuration load, never once per process (spec §9.2.2 requirement 2).** An earlier draft of this entry described a once-per-process flag with a `_resetProjectRootDeprecationWarned()` test hook. Both are **gone**, and neither was ever released. §9.2.2 now forbids suppressing the notice with process-global state for two reasons this SDK demonstrated: it made emission **order-dependent** — in a process that loads several configurations, whichever load ran first consumed the warning, so a later affected document was silent and the operator could not tell which one triggered it — and the same global was a test-isolation hazard, one test consuming the warning another test needed. The notice is a property of *the document being loaded*: every load satisfying both conditions emits it, a `reload()` that re-reads an edited file re-evaluates and may legitimately warn again, and de-duplication for log volume is the host logging layer's job. `tests/test-config-project-root.test.ts` pins the case the flag made impossible — two different affected configurations in one process, each producing its own notice naming its own key and directory.

### Fixed

- **A set-but-empty `APCORE_*` variable silently blanked the directory a configuration file declared ([apcore#115](https://github.com/aiperceivable/apcore/issues/115), spec §9.2.1 requirement 5).** §9.2's override rule and shell ergonomics collide: `export APCORE_ACL_ROOT=`, and a variable inherited empty from a container spec, are both *set* as far as the tooling is concerned, so `''` won the top precedence tier — and `''` is a legal relative path that every filesystem API resolves to the working directory. The result was a configuration key that quietly pointed at the CWD instead of the directory the operator had correctly written down, with no error and no log line.

  **An empty path-typed value is now discarded where the override is APPLIED**, in `applyEnvOverrides`, and resolution falls through to the next tier exactly as if the variable had been unset — configuration file, then the §9.1.1 default. The guard is driven by `Config.pathTypedKeys()`, so it covers §9.2.1's whole closed set (`acl.root`, `bindings.dir`, `extensions.root`, `extensions.roots`, `schema.root`) at once and a key added to that set later is covered without any consumer remembering to guard it. Putting it at the merge rather than at each consumer is the point: this SDK previously handled the case defensively for `bindings.dir` alone, in `src/bindings.ts`, which left the other four keys exposed. A warning naming the variable and the key is emitted (§9.2.1's MAY) — dropping an override the operator believes they set is precisely the silent failure the requirement exists to end.

  Scoped to path-typed keys only: `bindings.pattern` sits in the same configuration section, is explicitly **not** path-typed (§9.2.1 requirement 4, a glob matched against filenames inside `bindings.dir`), and an empty value for it is still a legal override that still applies.

- **`loadBindingDir` re-read `APCORE_BINDINGS_DIR` instead of the `Config` that had already resolved it, so `bindings.dir` in `apcore.yaml` was ignored and the `./bindings` default threw ([#36](https://github.com/aiperceivable/apcore-typescript/issues/36), [apcore#114](https://github.com/aiperceivable/apcore/issues/114), spec §5.12.6).** This SDK asserted in one place that `APCORE_BINDINGS_DIR` is a §9.2 configuration override — `applyEnvOverrides` lowers it into the declared key `bindings.dir`, pinned by `tests/test-config-discovery.test.ts` and stated in the 0.24.0 entry below — and read the raw variable in another, at `src/bindings.ts`. One variable, two readers, free to disagree: a `bindings.dir` written in a config file was returned by `Config.get` and ignored by the only code path that scans a binding directory, the §9.1.1 `./bindings` default never applied because the method threw `BindingFileInvalidError` instead, and precedence was effectively inverted — `Config` implements env > file > default correctly and then the sole consumer honoured the env tier alone.

  **`loadBindingDir` now takes an optional `Config`**, added as a fourth parameter after `pattern`, and resolves the directory as **explicit argument > `config.get('bindings.dir')` > `'./bindings'`**. `pattern` resolves the same way against `bindings.pattern` with the `'*.binding.yaml'` default, so it is no longer a value that exists only in the loader signature. The raw `process.env.APCORE_BINDINGS_DIR` read is **deleted**: §5.12.6 clause 2 states that an implementation MUST NOT read the variable at the loader, precisely so that one precedence chain governs the key. The parameter is typed `BindingConfigLike` — the `get` method and nothing else, structural like `AclConfigLike` — and is exported.

  **`APCORE_BINDINGS_DIR` keeps working**, because the environment tier now arrives through `applyEnvOverrides` like every other `APCORE_*` variable; the loader has to be handed the `Config` to see it. Every existing call site passes an explicit directory and is unaffected. **There is no automatic scan at client or framework initialisation**, which §5.12.6 clause 3 forbids: loading bindings stays an action the application takes.

  The pre-existing `loadBindingDir` tests all pass an explicit directory, the one path that behaves identically before and after, so they could not have caught this. `tests/test-bindings-config-dir.test.ts` adds the discriminating cases — a directory declared in a config *file* with `APCORE_BINDINGS_DIR` unset, the `./bindings` default, and a set `APCORE_BINDINGS_DIR` that must *not* reach a loader given no `Config` — plus the environment-tier back-compat guarantee end to end.

---

## [0.29.0] - 2026-09-05

### Added

- **`ApprovalRequest.callerId` and `ApprovalRequest.action` (decision log 2026-05 D-03).** `docs/features/approval-system.md`'s Contract block already required a request to carry `module_id`, `caller_id` and `action`; this SDK's `ApprovalRequest` (`src/approval.ts`) carried neither of the last two, so a handler — a Slack approver, an audit log — had to traverse `context.identity` and treat `moduleId` as double duty for "the thing being approved" instead of reading one flat record. `BuiltinApprovalGate` (`src/builtin-steps.ts`) now populates both at the one call site that constructs a real request: `callerId: ctx.context.callerId`, `action: ctx.moduleId`. `callerId` is read straight off `Context.callerId` with no substitution — `null` on a top-level call, exactly as `Context.callerId` itself is, never the `"@external"` sentinel ACL evaluation substitutes internally; `action` always equals the invoked module's id. Both fields are additive on `createApprovalRequest`'s options bag (`callerId` defaults to `null`, `action` to `moduleId`), so every existing call site keeps compiling and gets the right value without being touched.
- **`CancelToken.raiseIfCancelled()` (`docs/features/cancellation.md` "Contract: CancelToken.raise_if_cancelled").** Additive alias for `check()` — identical behavior (throws `ExecutionCancelledError` iff the token is cancelled) under the spec's canonical method name. `check()` is unchanged and not deprecated; it stays as the primary internal name (`src/builtin-steps.ts`) and remains available for existing external callers.

### Changed

- **`AsyncTaskManager.startReaper()` now returns `Promise<ReaperHandle>` (`docs/features/async-tasks.md` "Contract: AsyncTaskManager.start_reaper", decision D-11).** The canonical signature table names `await manager.startReaper({...}) -> Promise<ReaperHandle>` for TypeScript, uniform with Python's `await manager.start_reaper(...)` and Rust's `.start_reaper(...).await`; this SDK's `startReaper()` returned a plain `ReaperHandle`. Every call site in this repo already read fine as `await manager.startReaper(...)` (JS allows awaiting a non-Promise value), which is exactly what hid the divergence rather than fixing it. The method is deliberately still NOT declared `async`: there is no async setup to perform before the reaper is ready today (the sweep loop's own awaits happen inside the scheduled sweep, never before the handle is returned), and wrapping only the return value in `Promise.resolve(handle)` keeps the idempotency guard ("a reaper is already running") a genuine SYNCHRONOUS throw rather than turning it into a rejected Promise a non-awaiting caller could leave unhandled. **BREAKING for `const handle = manager.startReaper(...)` without `await` — `handle` is now a `Promise` and no longer exposes `.stop()` directly; add `await`.** Callers already awaiting the call are unaffected.

- **A pattern list with no operands made an ACL rule inert, and under `defaultEffect: 'allow'` that permitted the call the rule named (spec v1.31.0 §6.2.1 with §6.1.4.1 and §6.1.1, [apcore#112](https://github.com/aiperceivable/apcore/issues/112)).** `callers` / `targets` of `[]`, `['$or']` or `['$not']` can never match; the matcher returned `false` and `validateRules()` reported nothing, so a `deny` rule an operator wrote, loaded and validated contributed nothing to the decision — with one rule in the ACL the outcome tracked `defaultEffect` exactly. Reachable from a plain YAML file, because `ACL.load()` rejected an *omitted* `callers` / `targets` and permitted an *empty* one, and `schemas/acl-config.schema.json` had declared `minItems: 1` on both fields since the file existed, enforced by nothing. That is the same shape as [apcore#107](https://github.com/aiperceivable/apcore/issues/107) and [apcore#111](https://github.com/aiperceivable/apcore/issues/111): the constraint was declared in a normative artefact and no door enforced it.

  **A pattern array is FLAT, and its shape is now closed at every entry point.** The operators do not nest, there is no precedence, an operand is always a plain pattern string, and there is exactly one operator position — index 0 — which this specification had never stated while the same tokens nest arbitrarily in `conditions`. The closed set: the array **MUST NOT** be empty, every element **MUST** be a non-empty string, `$or` at index 0 **MUST** be followed by at least one pattern, `$not` at index 0 by exactly one, and `$or` / `$not` **MUST NOT** appear at any other index. A rule outside it is rejected with `ACLRuleError` naming the field and the rule index, at file loading, direct construction and `addRule()` alike (§6.1.6 rule 3) — and identically on `callers` and `targets`, which §6.2.1 constrains the same way. Reserved-token detection is by **equality**, never by `$` prefix: `['api.*', '$orders.*']` is an ordinary policy and still loads.

  **`$not` takes exactly one operand, and the multi-operand form was a silent privilege escalation.** §6.2.1 previously made `['$not', p1, p2, …]` implementation-defined — consult `p1`, ignore the rest — which every SDK did, so the form was uniform across implementations and uniformly **wider than written**: `targets: ['$not', 'secrets.a', 'secrets.b']` on an `allow` rule reads as "anything but `secrets.a`", and `secrets.b` — the second target the operator excluded — was **granted**. Rejecting it now is also what keeps open the option of a future version defining it as `NOT (p1 OR p2 …)`.

  **BREAKING for any deployment carrying one of these shapes**, which is exactly the population that believes it has a rule and does not. `targets: []` intending "everything" becomes `targets: ['*']`; intending "nothing" means the rule should be deleted; `['$or']` and `['$not']` have the same two readings, and the error names the field. The multi-operand `$not` is the one shape with **no mechanical migration**: `['$not', p1]` preserves what the rule has actually been doing, but if `NOT (p1 OR p2)` was intended a leading `deny` is not equivalent — a non-matching rule lets evaluation continue to later rules and a `deny` ends it — so the rewrite has to be done by hand against the rule's position and everything below it.

  **The backstop, for the one route no door covers.** `ACLRule` is a plain interface with mutable properties, so `rule.targets = []` on an already-constructed rule bypasses every constructor — and unlike an unrecognised `effect`, which is never read again once the doors are closed, a mutated pattern array **is** read: the matcher consults it on the next `check()`. Such a value is a §6.1.4.1 precheck fault on the same terms as a malformed type: the rule's scope is unreadable, the rule is UNEVALUABLE, and §6.1.1's effect table decides — a `deny` rule takes effect and denies, an `allow` rule does not match and **MUST NOT** grant. It never raises out of `check()`, it sets `handlerError` naming `callers` / `targets`, it warns, and `validateRules()` reports it with a **null** key and both resolvability flags `false`. The precheck does not short-circuit, so a rule faulty on both fields reports both paths in lexicographic order. §6.1.1 rule 5's "unknowable scope counts as scope" applies unchanged: such a rule carrying `approval: 'required'` still raises the pending requirement, because there is no partially-readable tier and deciding per fault kind whether a field is "readable enough" is the per-implementation judgement call that produced three different answers in [apcore#100](https://github.com/aiperceivable/apcore/issues/100).

  **A second, validator-only tier**, because closing the arities does not exhaust the inert class. `['$not', '*']` has legal arity, exactly one operand, and matches nothing — the identical fail-open. `validateRules()` now also reports a pattern array that is well-formed under every structural rule and still matches no legal module ID, with the same finding shape; it **loads**, it is **never** rejected, and it changes **no** access decision. The MUST-detect minimum is `['$not', p]` where `p` is `*`, `**` or any pattern consisting only of wildcards, and `['@external']` as a **`targets`** pattern — `@external` being the caller-side sentinel §6.5 substitutes for a null `callerId`, entirely legal and unreported in `callers`, which is what it is for. It is stated as a criterion with a minimum rather than an enumeration because the predicate cannot be closed without freezing the pattern language, and an incomplete predicate at a *door* would mean the same ACL file loads in one language and fails in another. Divergence in this finding set between SDKs is therefore acceptable and expected: it is diagnostics, not enforcement (§6.1.3).

  **Two points of order, resolved so three implementations cannot answer them three ways (§6.2.1, v1.31.0).** First, `addRule()` **re-validates the rule it is handed**, whatever its history — including a rule that was well-formed when constructed and has since had `callers` or `targets` assigned. This SDK already did (the door runs its own checks rather than trusting the rule type), and it is now pinned by a test, because the fixture's `entry_points` deliberately carries no per-door expectation and so cannot express the difference. Second, **validation order is `effect` -> `approval` -> `callers` / `targets`, and rule index dominates all three** — so one file produces one error, whichever door it arrives through. The pattern fields are **one axis** covering §6.1.4.1's type fault and §6.2.1's shape closure together; within it the type fault is named first and `callers` before `targets`. Two behaviour changes here, both about which of several faults is named and neither about whether a rule is refused:

  - **The approval axis moved ahead of the pattern fields.** A rule carrying `approval: 'required'` on an `effect: 'deny'` rule *and* a malformed pattern field was refused for the pattern field and is now refused for the approval pair, at `ACL.load()`, `new ACL([...])` and `addRule()` alike. (`{callers: [], targets: [], effect: 'Allow'}` was already refused for its `effect`, which is the resolved order.)
  - **`new ACL([...])` now validates rule by rule instead of sweeping one axis across the whole list.** A rule set whose rule 0 was bad on a late axis and whose rule 1 was bad on an early one reported **rule 1**, while `ACL.load()` — which parses rule by rule — reported rule 0 for the same content. The constructor is now one pass per rule, so both doors name the lowest-indexed bad rule, as §6.2.1 point 2 requires. The prohibition binds **every** per-rule check a door performs, including the axes only a loader has — §6.1's rule-key closure, the missing-key check, the value-type checks; `ACL.load()` already ran all of those inside its per-rule parse, so nothing moved there.
  - **`default_effect` is judged first, before any rule, at every door.** It is not a rule and has no index, so the rule ordering never reaches it, and `ACL.load()` parsed **every** rule on its way to the constructor that checked it — so a file wrong in both was refused for rule 0 through the file door and for `default_effect` through the other. The loader now judges it before it parses a rule, through the same function the constructor uses, so there is one check and one message.

  The error type (`ACLRuleError`) and every single-fault message are unchanged.

  `_matchPatterns()` and `_matchPatternsAsync()` keep their `false` returns for the illegal shapes as defence in depth; the precheck runs first and the rule never reaches either.

  Verified against `conformance/fixtures/acl_pattern_arity.json` (51 cases, all passing) by `tests/conformance-acl-pattern-arity.test.ts`. Nine of those carry `expected_refused_axis` and five of them `expected_refused_rule_index` as well, so the driver asserts which axis and which rule the refusal names rather than only that it refuses — `expected_load` sees neither. Seven cases carry a `rules` list rather than one `rule`, offered at the `load` and `construct` doors only, and they are what pins the cross-rule half across the three SDKs rather than in this repo alone. `acl_evaluation.json` lost `empty_callers_matches_none` and `empty_targets_matches_none` in the same spec change — the shapes they described are no longer constructible — so its driver drops the transitional branch that asserted this SDK's rejection of them and runs against the 19-case fixture unchanged.

---

## [0.28.0] - 2026-08-31

### Added

- **Argument-scoped approval — an ACL rule can now ask a human about *this* call (spec v1.28.0 §6.1.6–§6.1.8, §6.3.1, §6.8.1, §6.9, §7.4, §7.9.5, apcore#108).** Every decision point that could read a call's arguments was unable to escalate it to approval. The ACL could **refuse** on arguments and an `ApprovalHandler` could **wave through** on arguments; nothing could **ask**, and a refusal is not a question — so an operator who needed `git push --force` reviewed had to gate every `git push`, which dilutes `requiresApproval` from "this needs approval" to "this might" and floods the audit trail.

  **`approval` is a rule field beside `effect` (§6.1.6).** A rule answers two independent questions — may this caller reach this target at all, and must *this particular call* be put to a human — and folding them into one enumeration makes the meaningless state "denied and needs approval" representable while forcing the real one, "allowed but ask first", to be spelled as a kind of denial. `approval` is optional and absent means `'not_required'`, so every rule written before v1.28.0 keeps its meaning exactly. `approval: required` on an `effect: deny` rule is rejected with `ACLRuleError` — at file load, at direct construction and at `addRule()` alike, because a rule built in code is exactly as meaningless as one parsed from YAML. A value outside the two-member enumeration is rejected rather than coerced. Adding the field was only safe once v1.27.0 closed the rule key set: an SDK that still dropped unknown keys would read a `deny`-with-`approval` rule as a bare rule and act on half of what its author wrote.

  **`arguments` is a new built-in condition (§6.1.7)** with three predicates — `has_key` (any named key present), `has_all_keys` (every one), `has_none_of` (none) — AND-ed when several appear together. **No predicate reads a value.** The argument view available at Step 4 is not reliably redacted (redaction is driven by `x-sensitive` markers in the module's `inputSchema`, and a module without one gets none) and the arguments are unvalidated, because the ACL check is Step 4 and input schema validation is Step 7. Key presence is the one question well-defined on what is available. It is built in with **no registration point**: a deployment-registered argument handler is exactly the unauditable host code §7.9.6 rule 2 keeps out of a governance verdict, and being built in is also what makes §6.1.4's precheck cover it for free — `argument:` written for `arguments:` is an unregistered condition key, so the rule is unevaluable rather than silently inert. A malformed predicate value, an unknown predicate name and an `arguments: {}` that asks nothing are all **UNEVALUABLE** under §6.1.1's principle, never UNSATISFIED, and all three are reported by `validateRules()` because the whole condition is structural and therefore checkable without a context or a handler.

  **The governance projection (§6.1.8).** The `arguments` condition reads a `GovernanceProjection` — the argument key set plus each key's JSON type, and structurally **no value at all** — computed during module lookup (Step 3) and handed to the ACL check (Step 4). That ordering is normative, so `BuiltinModuleLookup` declares `governanceProjection` in `provides` and `BuiltinACLCheck` in `requires` rather than trusting step order to hold by habit. It is deliberately **not** `Context.redactedInputs`: that field's contract is safe *logging*, it is a raw copy when the module declares no input schema, and one field serving both "safe to log" and "input to a security decision" will eventually break one of them in a change made for the other. `_approval_token` is excluded, for the same reason §7.9.6 rule 5 strips it before policy resolution — it is a protocol-level key, not caller input, so a retry carrying it must present the same argument shape to governance as the original call did.

  **`checkAccess()` / `asyncCheckAccess()` return an `AccessDecision` (§6.8.1)** carrying `access`, `approvalRequired`, `matchedRuleIndex` and `reason`. `check()` and `asyncCheck()` are kept and now delegate, so one audit entry is still emitted per call. **SECURITY: the legacy boolean fails closed on an approval requirement** — a rule resolving to `allow` with `approvalRequired: true` makes `check()` return `false`. `check()` is public API consumed by callers that are not the Executor — tooling, preflight helpers, third-party integrations — and such a caller can only read a boolean as "let it through / do not"; returning `true` would let it execute a call the ACL said needed a human. `false` is wrong in the benign direction. The Executor uses the structured accessor and is unaffected, so "allowed but ask first" reaches the gate instead of surfacing as `ACL_DENIED`.

  **`AuditEntry.approvalRequired` (§6.3.1)** is a new field **beside** `decision`, not a third `decision` value: `decision` is a string downstream consumers parse and a third value would break every existing parser.

  **The Step 5 gate fires on a union (§7.4, §6.9 rows 3–5)** of the module's `requiresApproval` annotation, the ACL decision for this call, and `gateDestructive`. **SECURITY: an `ExecutionPolicy` override may ADD a requirement and MUST NOT remove one the ACL set (§6.9 row 4).** The ACL is a **caller**-scoped authorization layer and an `ExecutionPolicy` is a **module**-scoped platform override; letting the module-scoped one cancel a caller-scoped decision is a privilege escalation, because a policy rule written for `orders.*` would silently strip an approval requirement an ACL author attached to one untrusted caller. A policy `requiresApproval: false` still overrides the module's *annotation* — it is the ACL's decision it cannot reach. The `ApprovalRequest` handed to the handler carries `requiresApproval: true` for an ACL-sourced requirement, preserving the §7 contract for the one source that has no annotation to read.

  **`Executor.validate()` reports the governance-effective requirement (§7.9.5)** — the union above, not merely the policy-effective value, which would have told a caller no approval was needed for a call the gate will stop.

  `AccessDecision`, `AccessCheckOptions`, `ACLApproval` and `GovernanceProjection` are exported from both the Node and the browser entry points.

- **`ACL.defaultEffect` and `ACL.rules` read-only accessors (spec v1.23.0 §6.8, apcore#101).** Both fields were `private` with no getter, so a TypeScript consumer could not reach either of them at all — not the rule list `check()` evaluates, and not the single most consequential value in an ACL, the effect applied when nothing matches. Tooling that reports or audits the enforced policy — an admin surface, a preflight report, a linter over the rules §6.1.2 flags — had to re-read and re-parse the ACL file to recover a value the loaded object already held, and that second copy drifts from the object across `reload()`.

  Both are pure reads: no audit entry, no mutation, no lock. `rules` returns a frozen snapshot rather than the ACL's own array, because §6.8 rule 3 forbids handing out a reference through which a caller could mutate the enforced policy — a snapshot taken before an `addRule()` keeps the length it had. Both read the live object, so both reflect a `reload()`.

- **`ACL.validateRules()` (spec v1.22.0 §6.1.2 rule 3, §6.1.3; spec v1.25.0 §6.1.4, apcore#100).** Reports every rule that fails §6.1.4's precheck on the sync path, so a deployment can assert on the result once handler registration is complete. Each finding carries `ruleIndex`, `conditionPath`, `conditionKey`, `effect`, `syncResolvable` and `asyncResolvable`, ordered by rule index and then lexicographically by **path**. Faults nested inside `$or` / `$not` are reported at their own positions. It mutates nothing, registers nothing, and emits no audit event.

  Named `validateRules` rather than `validateConditions` because it reports structural faults in `callers` and `targets` too (§6.1.4.1), not only faults inside `conditions`. Ordered by path rather than by key because a nested `$or` can carry the same key at several positions, which leaves key ordering undefined — `$not.k` sorts before `$or[0].k` whatever the insertion order was.

  The two flags are reported **separately** and deliberately: `asyncCheck()` consults the async registry and falls back to the sync one while `check()` consults only the sync one, so a key registered *only* as an async handler is a working condition on one path and an unevaluable one on the other. A finding is emitted whenever `syncResolvable` is false, **including** when `asyncResolvable` is true — an application calling `check()` then has a condition it cannot evaluate. Whether that matters is the caller's judgement, not the validator's, which is why one collapsed boolean would have been the wrong shape. They are `*Resolvable` and not `*Registered` because they mean "resolvable on that evaluation path", not "present in that registry": `asyncResolvable` is the union of the two registries, so `asyncRegistered` would have read as an async-registry lookup and been false for every built-in leaf handler.

  The intended use is to call it once bootstrap has finished registering handlers and to treat any finding on a `deny` rule as a startup error. Nothing depends on anyone calling it: the guarantee that a broken `deny` rule cannot silently pass traffic is the entry below.

- **`ExecutionPolicy.resolve()` receives the call site (spec v1.24.0 §7.9.6, apcore#102).** A third optional `PolicyCallSite` parameter carries the invocation `arguments` and the `Context` alongside the module ID and annotations. Governance could previously decide on *which* module was being called and never on *what it was being called with*, so an operator who needed to gate *some* calls to a module had to gate *all* of them — audit noise, and `requiresApproval` weakened from "this needs approval" to "this might". The data was never missing: the approval gate is Step 5 and both halves are in scope at the call site, which passed only the module ID.

  The built-in pattern rules **do not consult it** (§7.9.6(2)): a rule set's verdict stays a function of the module ID and the annotations alone, so it remains statically auditable and reproducible from the policy document, and every existing verdict is bit-for-bit unchanged. It exists so an implementation can carry the call site into the audit trail and the `apcore.policy.override` event. Those arguments have **not** been schema-validated, because the gate is Step 5 and input validation is Step 7 (§12.8): nothing may assume the call site is well-formed, present, or of the declared type.

  §7.9.6 rule 7 requires the **capability**, not one API shape — an added method, keyword-only parameters and an options object are all conforming — so the optional third parameter satisfies it while leaving every existing two-argument caller compiling and behaving identically. `PolicyCallSite` is exported from the package root. The approval gate (`BuiltinApprovalGate`) and `Executor.validate()`'s preflight verdict both pass it, so preflight resolves against the same call site the gate will see.

  `_approval_token` is stripped from `arguments` **before** policy resolution, which §7.9.6 rule 5 requires explicitly: §7.4's "before passing to subsequent steps" does not reach a decision made *inside* Step 5, so an implementation can satisfy §7.4 literally and still hand the token to the policy — and from there into the audit trail and the `apcore.policy.override` payload. The gate already stripped it first; the preflight path now does too. A declarative argument predicate on `PolicyRule` is deliberately **not** added — see the note under §7.9.6.

- **`Executor.governanceState()` (spec v1.16.0 §6.6.5, apcore#97).** A read-only accessor returning a `GovernanceState` of eight observations plus one derived flag: what is *configured* on this executor versus what is actually *wired* into the running pipeline. `acl != null` was never the answer to "what is gating this registry" — the ACL and approval gates are pipeline steps, and the `internal`, `testing` and `minimal` presets all remove them, so an executor can hold an ACL that no step consults. `setAcl()` already warned about exactly this case; the warning is a one-shot log line, and this is the observable.

  Gate detection is by **type** (`step instanceof BuiltinACLCheck`), never by step name — the same test `setAcl()` performs when it wires a gate. A custom step named `acl_check` that never reads an ACL must not set `builtinAclGateWired`, because a false `true` there reports a gate that is not present, which is the one direction the flag must never fail in.

  `allControlModulesRequireApproval` is a required conjunct of the derived flag, because the two gates are not symmetric (§6.6.5.1.1): `acl_check` evaluates every call, while `approval_gate` returns before consulting the handler when the module does not declare `requiresApproval`. It reads the annotation through the same `needsApproval` predicate the gate itself uses (now exported), so the accessor cannot disagree with the pipeline it describes.

  The accessor is a pure read — it never enforces, warns, throws or mutates — computes live rather than caching, and returns booleans only: no ACL object, handler or policy leaks out. `GovernanceState` is exported from the package root.

### Changed

- **SECURITY: an ACL condition that could not be evaluated silently disabled the `deny` rule carrying it (spec v1.22.0 §6.1.1/§6.1.2/§6.3/§6.3.1/§6.5, apcore#100).** Condition evaluation returned a plain boolean, so "a handler answered no" and "no answer was obtainable" reached the rule loop identically and both meant *this rule does not match*. That is safe in exactly one direction: an `allow` rule that cannot evaluate its condition does not grant, but a `deny` rule that cannot evaluate its condition does not block — evaluation continued to the next rule and then to `default_effect`. A single misspelled key (`role:` for `roles:`) turned a rule its author believed was blocking into decoration.

  Evaluation now carries three outcomes. "Unevaluable" is a **principle, not a closed list** (§6.1.1 as of v1.25.0): the implementation cannot answer the condition *as written*. Five situations arise here, four of which existed in `src/acl.ts` as a `return false` — the key has no handler resolvable on the path in use (`Unknown ACL condition`), the handler threw, the handler returned a `Promise` on the sync `check()` path (which JavaScript cannot inspect synchronously), the value is **malformed for its key** (`$or` that is not a list, `$not` that is not an object, a branch of `$or` that is not a condition object), and `conditions` itself is not a mapping. An unevaluable condition resolves the rule toward **refusing access**: a `deny` rule matches and the call is denied; an `allow` rule does not match and does not grant. Nothing raises out of `check()` — §6.3's return contract is unchanged.

  The malformed-value case is a behaviour change of its own and a second door onto the same defect. A handler handed `$or: "typo"` returns false, runs to completion, and looks from the outside exactly like a handler that answered "no" — so a `deny` rule carrying that typo was inert. All three SDKs classified it as UNSATISFIED, and all three were following §6.1.1 v1.22.0's enumeration of exactly three situations, which is why the list became a principle.

  The three outcomes compose through AND, `$or` and `$not` by §6.1.1's three-valued table, which had to be normative because without it the same rule set resolves differently per SDK. An outright "no" wins an AND and an outright "yes" wins an `$or` even when a sibling was unevaluable; anything else with an unevaluable child is unevaluable. **`$not` of an unevaluable child is unevaluable, never satisfied** — negating "no answer" into "yes" would let a misspelled key inside a `$not` satisfy the very rule it was meant to gate, which is the same bypass one nesting level down. AND short-circuits on the first UNSATISFIED child and `$or` on the first SATISFIED child; neither short-circuits on UNEVALUABLE, because a later sibling may still be decisive. A child skipped by a legitimate short-circuit was never evaluated, so it is not unevaluable and does not set `handlerError`.

  `AuditEntry.handlerError` is non-null if and only if a condition was unevaluable, and it reports **every** such condition it determined in one `check()`, ordered **lexicographically by condition path** and joined with `"; "`. Paths are §6.1.4's: `k` at the root, `$or[i].k`, `$not.k`, `$` for the `conditions` object itself, `callers` / `targets` for the pattern fields — and they nest, so `$or[1].$not.k`. By path rather than evaluation order because the two are not the same across languages (`serde_json`'s map is ordered while a JavaScript object preserves insertion order), and by path rather than key because a key can occur at several positions in a nested tree, which leaves key ordering undefined. `$` replaces the `$conditions` placeholder this SDK used before v1.25.0 pinned the token.

  **Discovery, without breaking bootstrap order (§6.1.2).** `registerCondition()` writes to a runtime, process-wide registry and `acl.root` discovery commonly runs during framework bootstrap, ahead of the application code that registers handlers — so loading **warns and never fails**. The `ACL` constructor, `ACL.load()` (which reaches that constructor) and `addRule()` each run the same precheck and warn for every fault, naming the rule index, the condition path and the rule's `effect`. The `effect` is in the message because a misconfigured `deny` rule is the consequential case. Faults nested in `$or` / `$not` count, and so do structural faults in `callers` / `targets`. `validateRules()` above is the deterministic check to run once registration is complete.

  Also from §6.5: a conditional rule skipped because the call supplied no `Context` warns once per rule index, naming the index and the effect. That case stays a plain **non-match** and is deliberately *not* unevaluable — calling with no context is a legitimate shape for an external entry point, not a misconfiguration, and treating it as a failure would flip the decision for every `@external` call meeting a conditional `deny` rule. The consequence is worth stating anyway: a conditional `deny` rule is not a backstop for context-less callers. Express a backstop as an unconditional `deny` rule or as `default_effect: deny`.

  **This changes a decision, and the canonical `conformance/fixtures/acl_handler_error.json` still pins the old one** under a case named `throwing_handler_does_not_flip_default_allow_to_deny_unsafely`, which expects a `deny` rule with a crashing handler to let the call through. The corrected 15-case fixture is staged in the spec repo at `planning/acl-unevaluable-conditions/staged-fixtures/` and moves into `conformance/fixtures/` only once all three SDK drivers have landed, so CI does not go red across every SDK repository for the duration of the rollout. `tests/conformance-acl-handler-error.test.ts` reads the staged file when it is present and the canonical one otherwise, saying out loud which it got; the superseded case id — which the corrected fixture drops entirely — is overridden through a table that goes inert by itself once the fixture lands.

  Internally, `ACLConditionHandler` is unchanged: a handler that implements only `evaluate()` keeps the two-outcome contract, where `true` is satisfied, `false` is unsatisfied and a throw is unevaluable. The built-in compound operators, whose children can themselves be unevaluable, additionally implement `ACLOutcomeConditionHandler.evaluateOutcome()`, which also receives its own condition path so a fault inside a branch is reported at its own position. `ConditionOutcome` is exported from the package root.

- **SECURITY: a misconfigured `deny` rule still passed traffic for a caller that supplied no `Context`, and `handler_error` was only as complete as the evaluation order happened to make it (spec v1.25.0 §6.1.4, apcore#100).** Two problems with one cause: everything was decided *during* evaluation. §6.5 keeps "conditions present but no context provided" a non-match — deliberately, because calling with no context is a legitimate shape for an external entry point — and that check ran first, so `conditions: {mispelled: true}` on a `deny` rule fell straight through to `default_effect`. Measured in all three SDKs. Meanwhile the composition rules permit short-circuiting, which makes *which* faults are even reached implementation-defined, and that is incompatible with §6.1.1 rule 2's requirement that `handler_error` be deterministic.

  A **precheck** now runs before evaluation. It walks the rule's whole structure — `callers`, `targets`, and the entire `conditions` tree including every `$or` and `$not` branch — checking structure and the handler registries only. It supplies no context, invokes no handler, and does not short-circuit. A rule that **fails** it is unevaluable whether or not the call carried a context: a misspelled key is misconfigured regardless of who is calling. A rule that **passes** it and then finds no context still takes §6.5's path and does not match — `roles` is answerable in principle and this caller merely supplied no input for it, so it is not unevaluable. That distinction is the one the precheck exists to draw, and it is pinned by a control case in the conformance fixture whose `handler_error` must stay null.

  Because the precheck is context-free, handler-free and exhaustive, its findings are a pure function of the rule, so every SDK reports the same set in the same order. Diagnostics originating in handler *execution* — a registered handler that throws, an async handler on the sync path — carry no such guarantee and are reported as encountered; short-circuiting stays permitted there.

  **A precheck fault gates the rule; it does not enter §6.1.1's composition table** (§6.1.4 rule 5). `{$or: [{mispelled: true}, {roles: ['dev']}]}` for a caller who *has* `dev` is the discriminating case: the `$or` table alone says SATISFIED, because an outright "yes" wins. Gating makes the rule unevaluable instead — otherwise a typo is silent for exactly as long as some sibling keeps matching, which is the failure mode this whole entry is about, reappearing one nesting level down. Gating is also what makes rule 1 coherent: a precheck fault has to resolve a context-less call, where nothing is ever evaluated and the table has nothing to operate on.

  The split is precheck versus execution, not structural versus everything. A **registered** handler that throws is execution-origin, so the table does apply: `{$or: [{throwing_key: true}, {roles: ['dev']}]}` for a `dev` caller is SATISFIED, and an `allow` rule carrying it **grants** — with `handlerError` still set, because §6.3.1's "if and only if" binds to conditions, not to outcomes. Both directions are pinned by the conformance fixture and by local tests.

  **The precheck must not enlarge the set of calls a rule applies to** (§6.1.4 rule 4). Pattern fields are prechecked always, and a *malformed* one makes the rule unevaluable because its scope is unknowable. But when both are well-formed and either fails to match, the rule does not apply to this call: its `conditions` faults are neither consulted, nor reported in `handlerError`, nor allowed to change the decision. Without that, one misspelled key in a `callers: ["api.*"]` rule would deny a `worker.*` caller, which breaks first-match-wins. The fault is still real — `validateRules()` reports every fault in every rule regardless of any call, and that is where a scoped rule's typo is meant to surface.

- **SECURITY: a `callers` or `targets` written as a string instead of a list (spec v1.25.0 §6.1.4.1, apcore#106).** A bare string is iterable, so `callers: "admin.*"` written where `["admin.*"]` was meant is read character by character, and the `*` character is a pattern that matches everything — an `allow` rule carrying that typo grants access to **every caller**. Measured in apcore-python. apcore-typescript failed closed instead, by throwing a `TypeError` out of `check()`, which is better but still violates `Contract: ACL.check`'s "check MUST NOT raise to indicate a deny" — and whether a given typo was dangerous depended only on whether the mistyped string happened to contain a `*`.

  The precheck now classifies a `callers`/`targets` that is not a list of strings as unevaluable, at path `callers` / `targets`: an `allow` rule does not grant, a `deny` rule takes effect, nothing raises, and the value is never read as a pattern set. `ACL.load()` already rejected it via `_parseAclRule` and still does; the gap was the programmatic constructor and `addRule()`. An empty list is unaffected — that is a well-formed rule that never matches (§6.5).

- **`period` on `system.usage.summary` / `system.usage.module` is constrained by the schema (spec v1.14.0 §6.7.1.1, apcore#96).** `inputSchema` now declares `pattern: '^[1-9][0-9]*[hd]$'`, so a malformed value is rejected at input validation with `SCHEMA_VALIDATION_ERROR` rather than by `parsePeriod` throwing a plain `Error` from inside `execute()`. The accepted set does not change — `parsePeriod` already rejected `0h`, `-5d` and `+3h` — but the rejection now happens at the same boundary, with the same wire code, as in the other two SDKs. apcore-python accepted all three and silently produced an empty or negative window.

- **`tests/conformance-acl-argument-scoped-approval.test.ts` runs every case twice.** The fixture grew a second column (24 cases, up from 20): run 1 supplies a governance projection derived from `arguments`, run 2 supplies none at all, and both are contracts — §6.1.8 case 1 makes `check()` a public entry point that may be called without one. TypeScript can hand a projection to `check()` as well as to `checkAccess()`, so this SDK asserts all five keys in both columns rather than skipping the second; the column the drivers had been skipping is exactly where apcore#109 was sitting. Each column is now asserted against the async entry points too.

### Fixed

- **`requires_approval` was documented as if it answered "will this call need a human" (spec v1.29.0, [apcore#110](https://github.com/aiperceivable/apcore/issues/110)).** `ModuleAnnotations.requiresApproval` and `PreflightResult.requiresApproval` carried no doc comment at all, so a reader took the field name at its word. Since spec v1.28.0 that reading is wrong: `Executor.validate()` reports the **governance-effective** union (§7.9.5, §6.9 rows 3-5), so a module declaring `requiresApproval: false` correctly reports `True` there when the ACL requires a human for the arguments this call carries. The code was already correct; nothing said so.

  Both docstrings now say which question they answer: the annotation describes the **module** and `false` does not mean no approval will be required; the preflight result describes the **call** and is the same verdict the approval gate will enforce. Documentation only — no behaviour change.

- **A rule's `effect` accepted any string outside the YAML loader, and the accepted value was then read as `deny` (spec v1.30.0 §6.1.5 with §6.1.6 rule 3, [apcore#111](https://github.com/aiperceivable/apcore/issues/111)).** §6.1's field table has always said `effect` **MUST** be `allow | deny` and `schemas/acl-config.schema.json` has always declared the enum, but the check lived inline in the YAML rule parser and so guarded one of three doors: `ACL.load()` rejected `effect: "Allow"` — the capitalisation an operator writes by hand — while `new ACL([...])` and `addRule()` accepted it. This is [apcore#107](https://github.com/aiperceivable/apcore/issues/107) one level down: there the rule **key** set was closed while an unknown key was dropped in silence; here the key is legal and its **value** was dropped, in the same silence.

  **The inconsistency was internal, not only cross-language.** `defaultEffect` — the same two legal values one field up — has always been validated in the constructor, which `ACL.load()` also reaches, so it was guarded at every door while a rule's `effect` was guarded at one. §6.1.6 rule 3 has required rejection at *every* entry point that accepts a rule since v1.28.0, and had never been applied to the field it is named after. apcore-rust already rejected all three doors, so an ACL built in code ran under this SDK and could not be constructed under that one.

  **Not a privilege escalation** — an unrecognised value could never grant, because every decision site resolved "not `allow`" to a denial. It is a silent functional break in the other direction: under `defaultEffect: 'allow'` a rule the operator wrote to **permit** denies everything it matches, flipping those calls with no error, no warning and nothing from `validateRules()`. On a `deny` rule the fallback was only *accidentally* right, which holds until someone revisits which way it points.

  The parser's check is now one shared `rejectInvalidEffect()` behind all three doors rather than a second copy beside it — two copies of a validation rule drift, which is how this one came to guard a single entry point. It runs **before** §6.1.6 rule 2's `deny` + `approval` check, so `effect: 'Deny'` with `approval: 'required'` fails on the effect rather than slipping past that rule's `!== 'deny'` early return. `addRule()` returns nothing by its own contract, which §6.1.6 rule 3 says is not an exemption: throwing `ACLRuleError` is how TypeScript signals an unconstructable value, exactly as the constructor does, so there is no fallible twin to add beside it — and the rule list is left untouched, so a caller that swallowed the throw cannot end up enforcing the rule anyway. `defaultEffect` is now stated on the same terms instead of left correct-by-convention.

  The two decision sites keep their `effect === 'allow' ? 'allow' : 'deny'` form, which is now **total** over the closed set rather than a fallback: §6.1.1 makes `check()` a call that MUST NOT raise, so closing the doors — not throwing at evaluation time — is what satisfies "an implementation **MUST NOT** resolve an unrecognised `effect` to a decision".

  **Backward compatible:** no ACL that was already legal changes behaviour. Pinned by `tests/conformance-acl-effect-value-closure.test.ts`, which runs each of `conformance/fixtures/acl_effect_value_closure.json`'s 10 cases against **every** entry point the case names, and by a fixture-independent `tests/acl-effect-closure_spec.test.ts` so the closure stays pinned on a checkout with no spec repo beside it. Without the change 5 of the 10 fixture cases and 7 of the 14 local cases fail.

- **SECURITY: an unevaluable approval rule stepped aside and the call was granted without approval (spec v1.29.0 §6.1.1 rule 5, §6.8.1, §6.9 rows 1-2, [apcore#109](https://github.com/aiperceivable/apcore/issues/109)).** The shape §6.1.7 was written for is a narrow approval rule ahead of a broad allow — `git push --force` needs a human, `git push` does not. When the narrow rule's condition could not be *evaluated*, §6.1.1 resolved it to "does not match, MUST NOT grant" and scanning continued; the broad rule then granted, carrying no requirement of its own. The result was `access: 'allow'` with `approvalRequired: false` on exactly the call the operator gated, and `matchedRuleIndex` naming a rule that never mentioned approval — while `acl.ts` logged *"the allow rule does not match and MUST NOT grant"* on the very call it was granting. Reproduced in all three SDKs.

  **The root cause is a section that outlived its assumptions.** §6.1.1 was written in spec v1.22.0, when a rule carried one axis, and "an `allow` rule MUST NOT grant" was a complete instruction then: the rule steps aside, and stepping aside was harmless because whatever granted next also said `allow`. Spec v1.28.0 gave rules a second axis (`approval`) and did not revisit it, so "does not grant" began silently discarding the requirement the rule carried.

  **It is not confined to the legacy boolean or to a missing projection.** The trigger is an unevaluable `allow` rule, and §6.1.1 is the path that misconfiguration, §6.1.2's warn-don't-fail registration ordering and handler failure all take. A misspelled predicate (`has_keys` for `has_all_keys`) or an unregistered condition key reaches it **with a governance projection present**, on the ordinary Executor pipeline; `defaultEffect: 'allow'` reaches it with no second rule at all. `validateRules()` is not a mitigation — §6.1.2 makes an unregistered condition key a warning rather than a load failure, so nothing stops such a rule reaching production.

  **The requirement is now pending rather than discarded.** An unevaluable `allow` rule carrying `approval: 'required'` records a pending requirement and scanning continues; the rule itself still does not grant. Whatever grants next composes it by **disjunction** — a later `allow` rule *or* `defaultEffect: 'allow'`, which makes `approvalRequired: true` with `matchedRuleIndex: null` a legal combination for the first time. A final decision of `deny` clears it, and `matchedRuleIndex` keeps naming the rule that actually decided rather than the unevaluable one. `handlerError` is untouched: a pending requirement neither suppresses nor substitutes for it, and `AuditEntry.approvalRequired` carries the **final** value. The §6.1.1 warning now names the surviving requirement, because the old wording was logged on the call the next rule then granted.

  **Scope is what keeps it from over-reaching.** A rule whose well-formed `callers` / `targets` do not match this call returns `'no_match'` before its conditions are read at all (§6.1.4 rule 4c), so it raises nothing — a rule written about one target must not attach a human to calls it was never written about. A rule whose own pattern field is *malformed* (§6.1.4.1) does raise it: its scope cannot be read, so it cannot be shown not to apply, which is the posture that field already produces under `deny`, where an unreadable scope denies every call.

  **Requiring a human rather than denying is deliberate.** The condition that could not be evaluated is the one that decides whether *this* call is the dangerous one, so refusing would turn every ordinary `git push` into the hard failure §6.1.7 exists to eliminate. "Ask" is the answer that is wrong in neither direction.

  Applied to `checkAccess()`, `asyncCheckAccess()` and both legacy booleans — a requirement that survives on one entry point and is lost on another is the same fail-open, reachable by choosing a different call. §6.8.1's fail-closed rule for `check()` is now a property of the **decision**, not of the matched rule, so the boolean fails closed on a pending requirement identically.

  **Backward compatible for correct configurations:** across all 20 pre-existing cases of `conformance/fixtures/acl_argument_scoped_approval.json` **with a projection**, no decision changes. Without a projection, two change, both `approvalRequired: false` → `true`.

---

## [0.27.0] - 2026-08-14

> **Release note:** this section contains BREAKING changes. It must ship as a
> **minor** (or major) version bump, never a patch.

### Changed

- **BREAKING (security): a failed `acl` check now withholds module-level introspection from `validate()` (spec v1.13.0 §12.8.5.1, apcore#96).** `validate()` looked the module up at Step 3 and ran `preflight()` and `preview()` at Check 7 on the strength of that lookup alone, so a caller the ACL had just denied still made module-authored code run and still received what it returned. For a command-wrapping module that is the resolved binary and its argv; for a writer it is the target of the side effect. All three SDKs did it, and `apcore-mcp-rust` had already grown a string-matched disclosure filter over the top of it, which is the evidence the gap was reachable in a shipped product rather than theoretical.

  `validate()` no longer invokes either hook, emits a `module_preflight` / `module_preview` check, or populates `predicted_changes` when the `acl` check failed. The failed `acl` check itself is still reported, so a denied caller still learns *why*, and no other check is suppressed: the rule is about **authorization**, not validity. A failed `schema` check does **not** suppress introspection — a caller the ACL permits is entitled to the module's account of what would happen even when its inputs are malformed, which is what it needs in order to fix the call. Pinned by `conformance/fixtures/preflight_disclosure.json` (4 cases), whose control case exists so that an implementation which never introspects at all cannot pass the denial cases for the wrong reason.

### Added


- **`_config.strict: true` now also rejects keys a framework section does not declare** (PROTOCOL_SPEC §9.6.3 clause (b), §9.10 sub-algorithm `reject_unknown_framework_keys`). Every framework section in `schemas/apcore-config.schema.json` is `additionalProperties: false`; that closedness was previously enforced by nothing at run time, so `executor: { max_call_dept: 7 }` resolved `executor.max_call_depth` to its default while the operator read the file and believed they had overridden it.

  **Opt-in — non-strict deployments are unaffected.** With `_config.strict` absent or `false` (the default) the sub-algorithm returns at step 1 and behaviour is byte-for-byte what it was: the unknown key is **retained** in the tree and readable through `get()`, never pruned. That tier is now pinned by a test rather than merely true by accident — an SDK that models a section as a typed record and silently drops what the record does not model turns "the operator wrote it and it vanished" into "the operator never wrote it", and apcore-rust did exactly that for every `observability.*` subkey until apcore-rust#33. With `strict: true`, the key raises `CONFIG_INVALID`, and the error **enumerates every** offending key rather than stopping at the first, so one restart shows the whole problem instead of one restart per typo.

  `allow_unknown` deliberately does not participate: §9.6.3 defines it for unknown top-level *namespaces*, and stretching one field across two granularities would make its meaning depend on where it is read.

  Applies in **both** file layouts — step 1 of Algorithm A12-NS runs the sub-algorithm in legacy mode too, where the whole document *is* the `apcore` namespace, not only over `data.apcore` in namespace mode.

  New `src/config-key-surface.ts` carries the declared key surface and the walk. It stops at declared leaves, so the payload-shaped values the schemas declare — `pipeline.steps`, `pipeline.configure`, `id_map.overrides` — are not descended into and reported key by key. The surface is a committed projection of `schemas/apcore-config.schema.json`, `schemas/defaults.schema.json` and `schemas/sys-modules.schema.json` rather than a run-time parse of them (the schemas live in the spec repo; the npm package ships `dist` only), and `tests/conformance-config-key-governance.test.ts` asserts it equals the fixture's `allowed_keys` — itself regenerated from those schemas — on every run, so a section added upstream surfaces as a test diff instead of as a key strict mode wrongly rejects. All three schemas feed it, not `apcore-config.schema.json` alone: that file's `SysModulesConfig` declares `enabled` and nothing else, so enforcing against it in isolation would reject `sys_modules.events.enabled` — documented in `features/system-modules.md` and validated by every SDK's constraint table.

### Changed


- **BEHAVIOUR CHANGE: the library-level coercion knob no longer accepts ten of the twelve boolean spellings it used to. `new SchemaValidator(true).validate()` accepted `"yes"`, `"no"`, `"on"`, `"off"`, `"y"`, `"n"`, `"t"`, `"f"`, `"1"`, `"0"` — and any casing of any of them, including `"True"`, `"TRUE"`, `"False"` — for a declared `boolean`. All of those are now rejected. Only `"true"` and `"false"`, lowercase, still coerce (apcore#95).**

  **What the spec now says.** `docs/spec/type-mapping.md` §11 "What the knob coerces, when it exists" became normative at spec v1.12.0. *Offering* the switch stays a **MAY** — an SDK with no coercing mode at all is conforming — but an SDK that offers one **MUST** coerce exactly `string → integer` (entire content parses as an integer; `"3.14"` **MUST NOT**), `string → number` (entire content parses as a number), and `string → boolean` for **exactly `"true"` and `"false"`, case-sensitive** — and **MUST NOT** coerce anything else. Before v1.12.0 §11 constrained only *where* the knob may be used, never *what* it does, so a twelve-spelling dialect and a flat refusal were both conforming and neither was written down.

  **Where the twelve spellings came from.** `coerceStrToBool` was written during apcore#93 as a port of `apcore-rust::coerce_str_to_bool`, whose match arms were `"true" | "yes" | "on" | "y" | "t" | "1"` and the negatives, case-insensitively. apcore-python coerced no string to a boolean at all. `conformance/fixtures/schema_validation.json` pinned the coercing mode in exactly one case — `wrong_type_string_for_integer`, on the one axis where all three SDKs agreed — which is why a three-way divergence this wide never turned a suite red.

  **`"0"` → `false` is why this is a fix and not a preference.** R5 (`type-mapping.md:856`) makes the *number* `0` a MUST-reject for a declared `boolean`, and this SDK enforces that at the module-invocation boundary. The knob converting the *string* `"0"` to `false` put two paths of the same library on opposite sides of one value, and which answer a caller got depended on which validator they happened to hold. `"yes"`, `"on"`, `"y"`, `"t"` are shell and INI conventions: each is somebody's default and none is anybody's standard, so they belong to whatever parses `argv`. `"true"` / `"false"` survive because they are JSON's own spelling of a boolean — accepting them is reading the same value written as text.

  **Blast radius is the standalone validator only.** `coerceTypes` defaults to `false`, both production construction sites pass `false` explicitly (`src/builtin-steps.ts`, `src/config.ts`), and there is no `schema.validation.coerce_types` setting — the module-invocation boundary never coerced any of these spellings and is byte-for-byte unaffected (TYPE_MAPPING §17.3). `src/config.ts`'s environment-variable coercion is a separate function (`coerceEnvValue`, int-then-float, no boolean handling at all) and is untouched. Only a caller who explicitly constructed `new SchemaValidator(true)` and fed it `"yes"`-family input sees a change, and what they see is a `SCHEMA_VALIDATION_ERROR` where they previously got a boolean.

  **Migration.** Map your own spellings before validating — `flag === 'on' ? true : flag` — or keep the wider vocabulary in the argv/form parser where it belongs. Integer and number coercion are unchanged; `"42"`, `"-7"`, `"42.0"` and `" 42 "` still become integers and `"3.14"` still does not, matching apcore-python and apcore-rust.

  Driven by six new cases in `conformance/fixtures/schema_validation.json`, four of which assert a spelling that MUST NOT coerce. Every one states both `expected_valid_strict` and `expected_valid_coerce`, and the driver asserts both halves against a validator built in the matching mode — a fixture carrying only `"true"` is satisfied by an implementation that coerces any non-empty string, which is close to what this SDK actually shipped. `tests/schema/test-validator.test.ts` pins the whole §11 table locally, including each of the ten withdrawn spellings by name.


- **BEHAVIOUR CHANGE: `pipeline.configure` no longer accepts `requires` / `provides`. A configuration that carried them was accepted and is now a startup error (apcore#89).** The configurable set is exactly the four behavioural modifiers of `schemas/apcore-config.schema.json` `$defs/ConfigurableStepFields` (`additionalProperties: false`) and `docs/spec/DECLARATIVE_CONFIG_SPEC.md` §4.2 — `match_modules`, `ignore_errors`, `pure`, `timeout_ms`. Anything else raises `PIPELINE_CONFIGURATION_ERROR` at parse time, which is what the unknown-field path already did; `requires` and `provides` simply move from the accepted side of that check to the rejected side.

  **What accepting them cost.** A step's `requires` / `provides` are its capability contract, declared by the implementation, and `ExecutionStrategy._validateDependencies` enforces it — `features/middleware-system.md` § Configuration safety states the `PipelineDependencyError` as a MUST. Measured on this SDK before the fix: the override landed as an own property on the built step, overwriting the class field, and the validator then read the rewritten value. The built-in `input_validation` declares `requires = ["module"]`, supplied upstream by `module_lookup`; a `[context_creation, input_validation]` strategy therefore threw `PipelineDependencyError`, and after `configure: { input_validation: { requires: ["context"] } }` the same two steps constructed cleanly. The dependency was deleted, and the MUST could never fire for that step again. apcore-python measured identically.

  **The example that taught it.** That YAML was not invented for the test — it shipped as the canonical `pipeline.configure` example in `features/middleware-system.md`, eleven lines below the MUST it defeats, carrying the comment *"matching `schemas/apcore-config.schema.json`"* while the schema pinned nothing. The documented way to exercise the dependency contract was the way to disable it. The page now carries a warning box saying so, and the schema is closed over the four fields.

  **Migration.** Delete the keys from `apcore.yaml`; declare the contract on the step class instead (`requires` / `provides` as fields on your `Step`). The error message names the four valid fields and says why these two are not among them.

  **Unchanged, and deliberately kept:** the canonical snake_case spellings, the camelCase aliases `matchModules` / `ignoreErrors` / `timeoutMs` this SDK accepts at its own API boundary (§4.2 permits an idiomatic alias; a configuration *file* carries the canonical spelling only), and raising rather than warning. `tests/conformance-pipeline-failfast-config.test.ts` now drives all six cases of `conformance/fixtures/pipeline_failfast_config.json`, including the three new ones, and reads `expected.configured_step_fields` back off the built step object — `raises: false` alone passes against an implementation that accepts the four keys and applies none of them.

- **BEHAVIOUR CHANGE: an unknown key on a `pipeline.steps` entry is now a startup error, and the canonical snake_case keys on such an entry now reach the step at all (apcore#89).** `schemas/apcore-config.schema.json` `$defs/PipelineStep` has been `additionalProperties: false` since it was written and nothing enforced it. Measured on this SDK before the fix, `steps: [{name: 'x', type: 'noop', after: 'execute', tiemout_ms: 5000}]` built successfully with `timeoutMs === 0` — the operator's five-second timeout had no effect and nothing said so. It now raises `PIPELINE_CONFIGURATION_ERROR` at parse time, before any step factory runs, naming the offending key and the ten valid ones.

  **The second half is the one that bit an operator who spelled everything correctly.** `StepDefinition`, `_resolveStep` and `ConfiguredStep` are camelCase while the wire spelling is snake_case, so on a `steps:` entry the CANONICAL keys were dropped as silently as the typos: `timeout_ms: 5000` produced `timeoutMs = 0`, `ignore_errors: true` produced `false`, `match_modules` produced `null`. Only `pure` survived, because it is spelled the same in both. This is the identical defect fixed for `configure:` one pass earlier — three of four fields unreachable from real YAML — still fully present one key over, and closing the key set without also accepting the canonical spellings would have certified those three as valid while they continued to do nothing. The new `_normalizeStepDefinition` validates against `$defs/PipelineStep` and maps snake_case to the property the builder reads; the camelCase aliases `matchModules` / `ignoreErrors` / `timeoutMs` keep working as this SDK's own idiomatic surface.

  `PipelineConfig.steps` is typed `Array<Record<string, unknown>>` rather than `StepDefinition[]` as a consequence: the narrow type made TypeScript's excess-property check reject the very spelling an `apcore.yaml` carries, so every call site writing canonical YAML needed an `as never`. This widens what the function accepts at compile time and narrows what it accepts at run time, which is the right way round for a parser whose input is untyped YAML.

  Driven by the fixture's seventh case, `unknown_key_on_a_steps_entry_raises_configuration_error`. Its `type` must be a REGISTERED step type or the entry never reaches the insertion path and the case passes on an unrelated "type not registered" error, so the driver registers a no-op factory under the fixture's own `type` value and first asserts the same entry inserts cleanly with the bad key removed.

- **`Step.requires` / `Step.provides` are documented as enforced, not "Advisory only".** The doc comments on `src/pipeline.ts` still described the pre-#33 `console.warn`; that warn became a `PipelineDependencyError` throw at strategy construction and the comments were left behind. No behaviour change — the throw has been there since #33 — but the comments said the opposite of what `_validateDependencies` does and of what the spec makes a MUST.

- **Namespace-mode `validate()` now reports unknown namespaces alongside every other error instead of throwing on the first one.** §9.10 step 4 collects all errors before raising; this threw from inside the loop, *after* the accumulate-and-throw block, so a file with two unregistered namespaces cost two restarts and hid every schema error behind whichever namespace the registry map happened to yield first. Message text per namespace is unchanged; it is now one of the bullets under `Configuration validation failed (N error(s))`.

- **`ConfigurationError` now emits `PIPELINE_CONFIGURATION_ERROR`.** It emitted `PIPELINE_CONFIG_INVALID`, which is a **different** code in the canonical registry (a malformed field value, not a missing step). apcore-python already emitted the correct code and apcore-rust emitted a third one; the conformance fixture asserted the CLASS name — shared by all three SDKs — and so reported green across a three-way split. The class name is unchanged (it is public API); only the wire code moved.

- **`pipeline.configure` accepts the canonical snake_case field names, and `requires` / `provides`.** Validation used `key in step`; `matchModules` / `ignoreErrors` / `pure` / `timeoutMs` are optional on the `Step` interface, so a concrete step that does not declare one — `BuiltinInputValidation` declares no `ignoreErrors` — had that field rejected, with an error message that listed the field as valid. Validation is now against a configurable-field set keyed by the **snake_case** spelling that `schemas/apcore-config.schema.json` `$defs/PipelineStep`, every `apcore.yaml` and every conformance fixture actually use (`match_modules`, `ignore_errors`, `pure`, `timeout_ms`, `requires`, `provides`), with the camelCase spellings accepted as aliases. Previously three of the four fields were unreachable from real YAML. `requires` / `provides` are included because `features/middleware-system.md` "Configuration safety" documents configuring them. Narrower than the old test on purpose: `name`, `description`, `removable`, `replaceable` and `execute` are **not** configurable — `key in step` walked the prototype chain, so a config file could previously replace a step's `execute` body.

- **`StepMiddleware` ordering and error wrapping corrected (Issue #33 §2.2).** `afterStep` and `onStepError` run in reverse registration order, `onStepError` short-circuits on the first non-null recovery **when a step body failed**, and a throwing `beforeStep` is wrapped in `MiddlewareChainError` (it previously escaped bare) and fires `onStepError` on the already-executed middlewares only. On the `beforeStep` path there is no recovery to shop for, so there is no short-circuit either — every already-entered middleware is notified so its cleanup runs; see the terminal-`beforeStep` entry below.

- **BREAKING: a `StepMiddleware.onStepError` recovery value now becomes the step's output.** `features/middleware-system.md` → "Normative Rules" states it as a MUST ("the recovery value becomes the step's output") and both apcore-python (`ctx.output = recovery`) and apcore-rust (`ctx.output = Some(value)`) implement it; this SDK deliberately treated the value as informational and left `ctx.output` untouched, making it the only SDK violating the rule. A plain-object recovery is now published to `ctx.output` — arrays and scalars still suppress the error and still reach `afterStep`, but cannot be represented in `PipelineContext.output`, matching apcore-python's `isinstance(recovery, dict)` guard. Middlewares that already mutated `ctx.output` by hand inside `onStepError` are unaffected; middlewares relying on the recovery value being *discarded* will now see it in the output. **This applies to the STEP-BODY failure path only.** A recovery returned after a `beforeStep` failure is still discarded and is deliberately excluded — publishing it there was an authorization bypass, not a fix; see the next entry.

- **BREAKING: a `StepMiddleware.beforeStep` failure now terminates the step and is NOT recoverable.** `features/middleware-system.md` → "A `before_step` failure terminates the step — it is not recoverable" makes the `beforeStep` path categorically different from the step-body path, and the two MUST NOT share a recovery path. Previously a `beforeStep` failure ran `onStepError`, honoured a returned value as recovery, published it as the step's output, fired `afterStep`, and advanced to the next step. **That was a silent authorization bypass:** the built-in strategy places `acl_check` and `approval_gate` in the step sequence, so a middleware that made its own `beforeStep` throw and then returned a value from `onStepError` skipped the gate outright — from an extension point that carries no authority. Now, when `beforeStep` throws: the step body does not execute; `onStepError` still runs in reverse order over **every** already-entered middleware (observation and cleanup only — no short-circuit, since no recovery is being sought) and its return value is **discarded**; `afterStep` does **not** fire, because no step body ran and the onion is already torn; the step's `ignoreErrors` does **not** apply, because a broken middleware chain is not a step failure; and `MiddlewareChainError` propagates regardless, un-re-wrapped. Note the deliberate asymmetry with the module-level contract: `Middleware.onError` **does** honour a before-phase recovery, and that is consistent because a module-level recovery *terminates the call* and is the return value, whereas a step-level recovery *resumes a pipeline*. Matches apcore-rust. Pinned by `pipeline_step_middleware.json` → `before_step_failure_recovery_is_discarded`, whose driver asserts the discard by observing that the FOLLOWING step never executed — a throw-only assertion is satisfied by an implementation that honours the recovery and fails later, with the bypass live. Its opposite, `after_step_fires_after_a_recovered_step`, pins that a recovered **step body** still closes the onion.

- **BREAKING: an explicitly empty `obs.redaction.sensitive_keys` now disables key-based redaction.** `RedactionConfig.fromConfig` fell back to the shipped 16-entry default list whenever the configured list was *empty*, not merely absent — so an operator who deliberately wrote `sensitive_keys: []` to switch key-based redaction off got the default list anyway. D-54 and `features/observability.md` require the override to **replace** the default, and an empty override is still an override; only a missing or `null` value falls back. apcore-python already behaved this way. **Security-relevant in both directions:** anyone already carrying `sensitive_keys: []` was silently getting default redaction and will now get none, and note that `_secret_*` is entry `[0]` of the default list rather than a separate hard-coded rule, so an empty list disables it too. The value-regex rule (`obs.redaction.regex_patterns`) is independent and unaffected. Absent and `null` are unchanged.

- **`PeriodicUsageExporter.stop()` is idempotent.** It called `exporter.shutdown()` on every invocation, so a second `stop()` drove `shutdown_call_count` to 2; `stop()` before `start()` also shut down an exporter that had never been taken over. Both now no-op, matching apcore-python and apcore-rust.
- **BREAKING: applicator keywords are enforced.** `prefixItems`, `patternProperties`, `propertyNames`, `dependentRequired`, `dependentSchemas`, `if`/`then`/`else`, `unevaluatedItems` and `unevaluatedProperties` were dropped by the JSON-Schema-to-TypeBox conversion, so a contract declaring them was silently unenforced while apcore-rust and apcore-python rejected the same input. They are now asserted by a self-contained applicator evaluator registered as a custom TypeBox kind (type-mapping §17). "TypeBox has no node for it" was never a reason to drop a keyword.

- **BREAKING: recursive schemas can be registered.** `RefResolver` no longer throws `SchemaCircularRefError` for a self-reference, and `jsonSchemaToTypeBox()` binds the preserved `$ref` lazily through a new `apcore:Ref` node resolved against the document root at check time. Covers `#`, the root `$id` (previously a filesystem lookup ending in `SchemaNotFoundError`) and `#/$defs/…`. A `$ref` → `$ref` cycle still throws.

- **BREAKING: `oneOf` is exclusive at every nesting depth.** `oneOf` compiled to a plain TypeBox `Type.Union`, whose semantics are `anyOf`, and the validator only looked for the keyword marker at the root and along `allOf` — so a `oneOf` inside `properties` or `items` accepted an input matching several branches. It now compiles to an `apcore:OneOf` node that counts matches, so the rule holds wherever `Value.Check` reaches rather than only where the validator thought to look.

- **BREAKING: `toStrictSchema()` hardens the objects it was skipping and wraps rather than appends.** An object carrying `properties` with no `type`, or `type: ["object", "null"]`, was returned unhardened; `prefixItems` was not recursed into. Separately, an optional property already carrying `oneOf`/`anyOf` had `{type: "null"}` **pushed into the author's union** — rewriting the contract they declared, and for `oneOf` the very branch count the validator checks against. It is now wrapped as `{anyOf: [<original>, {type: "null"}]}`, matching the other two SDKs and the A23 pseudo-code.

- **BREAKING: `ExecutionPolicy.fromObject` rejects a non-boolean `gate_destructive` / `strict`.** They were read through `Boolean(...)`, so JS truthiness silently decided a governance switch: `gate_destructive: []` became `true` in TypeScript where apcore-python reads `False` and apcore-rust errors, and `"false"` became `true`. Both keys now require a real boolean (`null` / absent still mean the documented `false` default), extending the existing fail-loud-on-unknown-keys discipline from the keys to their values.

- **BREAKING: `Registry.register()` validates module structure before duplicate detection.** The two steps were inverted relative to `registry-system.md` "Side Effects (ordered)" (structure at step 2, duplicate at step 3) and to apcore-python. Re-registering an existing ID with a module declaring `streaming: true` but no `stream()` now raises `StreamingInterfaceError` — as Python and Rust do — instead of `DuplicateModuleIdError`.

- **BREAKING: config required-field validation is real, and narrow.** `REQUIRED_FIELDS` listed six keys but could never fire: `DEFAULTS` was deep-merged into the parsed file *before* `validate()` looked for them, so the merge had already supplied every key the loop asked about — a required-field list that read like validation and validated nothing. Two of those defaults were invented to keep it that way (`version: '0.16.0'`, which was neither the SDK version nor a spec value, and `project: { name: 'apcore' }`); both are removed. Per PROTOCOL_SPEC §9.1 a key is required **only when it has no canonical default**, which leaves exactly `version` and `project.name`, and §9.3 step 1 evaluates them against the **declared** document. `Config.getDeclared(key)` (mirroring apcore-rust's `get_declared`) reads the file + env + `set()`/`mount()` view with no defaults merged, and `validate()` uses it. Net effect: a config omitting `extensions.root`, `schema.root` or `acl.*` now loads fine, and a config omitting `version` or `project.name` now raises `ConfigError(CONFIG_INVALID)` where it previously passed silently. A bare `Config.fromDefaults()` therefore fails `validate()` unless the caller declares those two — defaults resolve values, they do not declare a project.

- **`Config.reload()` re-validates.** It reloaded with `{ validate: false }` and never validated afterwards, so a config that had since become invalid was adopted silently. `reload()` now re-runs `validate()` when the originating `Config.load()` requested validation (the default), after mounts and env overrides are re-applied; an explicit `{ validate: false }` at load time is still honoured on reload.

### Removed


- **`src/middleware/tracing.ts` — the §1.3 `TracingMiddleware` — is deleted. NOT a breaking change: it was never reachable from the package root.** `features/middleware-system.md` §1.3 is withdrawn; `TracingMiddleware` is specified once, in `features/observability.md` § Tracing Architecture, with `docs/spec/protocol-spec.md` §12 normative for the span. The §1.3 class was a second, weaker formulation of the same middleware and contradicted the surviving one on every rule that mattered: it named the span after `module_id` (the protocol spec requires the constant `apcore.module.execute`, conformance T08-007, with the module id as an attribute — a span name per module is high-cardinality, which the OpenTelemetry semantic conventions advise against, so the section labelled "OpenTelemetry-Compatible" prescribed the *less* OTel-compatible of the two), and it stored a single span id in `context.data["_apcore.mw.tracing.span_id"]`, a slot that the first nested module call overwrites. The surviving `observability/tracing.ts` `TracingMiddleware` keeps a **stack** in `_apcore.mw.tracing.spans` with explicit `parent_span_id` links, records the sampling decision in `_apcore.mw.tracing.sampled`, and needs no OpenTelemetry SDK at all. Both classes shared the `_apcore.mw.tracing.*` namespace, which is one framework middleware's private space by definition — which is what made them the same middleware rather than two.

  **Reachability, and therefore the semver impact.** `src/index.ts` exports `TracingMiddleware` from `./observability/tracing.js`; its `./middleware/index.js` re-export list deliberately omitted the name to dodge the collision. The §1.3 class was therefore importable only via the internal deep path `apcore-js/dist/middleware/tracing.js`, which is not a declared `exports` subpath — no consumer could reach it through the package's public surface, and no capability is lost.

  Removed alongside it, all of them satellites of that one class:
  - `src/middleware/tracing-otel-default.ts`, the Node-only side-effect module whose sole job was `createRequire`-loading `@opentelemetry/api` into the deleted class's default-tracer slot; with it goes its `package.json` `sideEffects` entry and the side-effect `import` in `src/index.ts`.
  - `CTX_TRACING_SPAN_ID` (the `_apcore.mw.tracing.span_id` key constant) and the types `OtelTracer`, `OtelSpan`, `TracingMiddlewareOptions`. **These four *were* re-exported from the package root and from the browser entry** — but only ever as accessories to an unreachable class. `TracingMiddlewareOptions` in particular did not describe the options of the root's `TracingMiddleware`: the root exported the type from one class and the class from the other. Nothing in `src/`, `tests/` or `examples/` read `_apcore.mw.tracing.span_id` or `_apcore.mw.tracing._active_span` outside the deleted files.
  - `tests/middleware/test-tracing-middleware.test.ts` (deleted outright, not skipped) and the `tracing_span_created` conformance driver in `tests/conformance.test.ts` (replaced by a comment recording why). Upstream `conformance/fixtures/middleware_hardening.json` dropped that case with the section; its `context_namespace_violation` case now uses `_apcore.mw.tracing.spans` as the example framework key. The surviving `tracing_noop_without_otel` case is now driven against `observability/tracing.ts`.

### Fixed


- **`SchemaValidator(true)` now actually coerces. `"42"` was rejected for `{type: "integer"}` where apcore-python and apcore-rust both accept it (apcore#93).** This was the only behaviour change needed by the per-SDK pinning sweep below, and it was a real cross-SDK divergence rather than a driver artefact: apcore-python passes `strict=not coerce_types` to pydantic and apcore-rust runs a `coerce_value` pre-pass, while this SDK switched `Value.Check` for `Value.Decode` — which applies TypeBox *transforms*, never type conversion. The class's own doc comment described the gap as "unimplemented" and blocked on "picking a conversion semantics all three can agree on"; that blocker was stale, because the other two had already agreed on **pydantic-lax** and shipped it.

  New `coerceValue(value, schema)` in `src/schema/validator.ts` ports apcore-rust's `coerce_value` rule for rule (module-level export, deliberately **not** added to the package root — apcore-rust keeps its equivalent private too, and the public surface is unchanged): coerce only FROM a string, only toward a declared `boolean` / `integer` / `number`, only when the conversion is exact. `"42"` and `"42.0"` become `42`; `"3.14"` for `{type: "integer"}` stays a string and still fails, because pydantic refuses a lossy conversion. That is deliberately **not** `Value.Convert`, which mutates its argument in place and truncates `"4.5"` to `4` — a silent data change no other SDK makes. `coerceValue` is pure; a caller's object is never rewritten. A union node (`anyOf` / `oneOf`, which is also how the converter renders `type: ["string", "boolean"]`) declares no single scalar target and is left untouched, so the existing enum-over-union tests are unaffected in both modes.

  The pre-pass runs once per public entry point, so the structural check, the collected error paths and the value `validateInput` returns all see the same coerced value.

  **The module-invocation boundary is unaffected, and that is the point of the knob.** `coerceTypes` is opt-in and still defaults to `false`; `builtin-steps.ts` and `config.ts` hold `new SchemaValidator(false)` and never coerce under any host configuration (TYPE_MAPPING §17.3). Every production construction site in this repo passes `false` — the only `true` was in tests.

- **Test-only: 24 more conformance cases across four fixtures were run by a driver that could not go red, and now are (apcore#93).** Same instrument as #92, asking the **per-SDK** question — *which cases does apcore-typescript not run* — rather than *does any driver run this*. A case one SDK checks and two skip proves one implementation, not three, so #92's default sweep went green here as soon as apcore-python pinned the shared fixtures. Before the fix: `call_chain` **11 of 11 — the entire fixture**, `schema_validation` 11 of 16, `config_env` 1 of 13, `event_management_hardening` 1 of 10. All four now report 0, verified against an isolated spec copy and re-confirmed by mutating nine representative cases by hand and watching each redden exactly its own test.

  **`schema_validation` lost eleven cases to one line.** The driver wrapped its assertion in `try { expect(result.valid).toBe(expectedValid) } catch (e) { if (expectedValid) throw e }` — a catch-all that swallowed *the assertion itself*. Flipping a case's `expected_valid: true` to `false` routed the resulting failure straight into the silent arm, so every positive case was unfalsifiable by construction and only the five negatives could ever go red. A conversion failure is now a test failure, never a silent "treat as invalid". Two further losses sat beside it: `if (typeof input !== 'object') return` quietly dropped `empty_schema_accepts_string`, whose entire point is a non-object input, and an `XFAIL_IDS` set quarantined two more. Both quarantines are gone — `empty_schema_accepts_string` was **stale** (an empty schema converts to `{}` and accepts a string today) and `wrong_type_string_for_integer` was the real coercion gap fixed above. The block now dispatches explicitly on which expectation key the case states, hard-fails on one it does not recognise, asserts `expected_error_path` for negative cases, and — for the case declaring both halves — asserts `expected_valid_strict` and `expected_valid_coerce` each against a validator built in the matching mode, plus the coerced *value*, which a validator that accepted the input without converting it would otherwise pass.

  **`call_chain` is the fixture that carries its own `driver_contract`, and this driver violated both clauses.** The negative half dispatched through a driver-local `CALL_CHAIN_ERROR_MAP` and asserted the **class**, so a fixture value the map did not know degraded to `toThrow(undefined)` — "something threw", satisfied by every wrong wire code. The map is deleted and the declared value now reaches `error.code` through the SDK's own `ErrorCodes` registry (apcore-typescript#34 item 2 is the record of what a private translation table in this file costs). `INVALID_LIMIT` joins `PARSE_ERROR` as a documented non-wire expectation, on the authority of the case's own note — "Python ValueError / TS Error / Rust ModuleError GENERAL_INVALID_INPUT", divergence T-B-005 — and anything else unrecognised is still a hard failure. The positive half's whole assertion was `.not.toThrow()`, which a `guardCallChain` that returned immediately also satisfies. Positive cases now assert the observable post-conditions the fixture asks for: the guard returns nothing, it does not mutate the caller's chain, and a **boundary probe** re-runs it with the one limit the chain sits under tightened by one and requires the matching rejection — proving the chain was accepted for being within the limits rather than for not being inspected.

  **`config_env :: nested_path_match` was quarantined as a "pre-existing SDK bug" that does not exist.** The comment claimed auto mode with `max_depth: 2` could not resolve `APCORE_MCP_ROUTER_MAX_TIMEOUT`. It resolves: `envSuffixToDotPathWithDepth('ROUTER_MAX_TIMEOUT', 2)` returns `router.max_timeout` and `config.get()` reads `30000` back, from an algorithm that is line-for-line apcore-python's. The quarantine is removed and the case is driven with the other twelve.

  **`event_management_hardening :: subscriber_factory_registered_type` asserted the driver against itself** — it registered the literal `'slack'` and checked `instanceof SlackSubscriber`, a class defined three lines earlier in the test. Neither `expected.subscriber_created` nor `expected.subscriber_type` was ever read. Registration is now driven off the fixture's `input.registered_types` and both declared expectations reach an assertion.

- **Test-only: 31 conformance cases across five fixtures were run by a driver that could not go red, and now are (apcore#92).** `conformance/check_case_pinning.py --sdk typescript` mutates a case's declared expectation so no correct implementation can satisfy it and reports the case if nothing turns red. Measured on this SDK before the fix: `error_codes` **18 of 18 — the entire fixture**, `dependency_version_constraints` 7 of 15, `version_negotiation` 3 of 10, `context_create` 2 of 15, `identity_system` 1 of 8. All five now report 0. **The same fixtures and, where they overlap, the same counts as apcore-python**, which is the signal that one driver shape was copied across the repos rather than three independent mistakes; only `context_create` differed, at 2 here against 1 there.

  The issue reports `error_codes` at 12, not 18, and the difference is the instrument rather than the driver: that sweep ran while the tool still treated the top-level `error_code` key as an expectation. In this fixture it is an *input* — the code being registered — so mutating it turned six `exact_framework_code_*` cases red by changing what the test **did**, not what it **checked**, and they were scored as covered. Corrected upstream; re-measured against the corrected tool, none of the eighteen was pinned.

  **`dependency_version_constraints` is the one worth reading twice.** The seven unpinned cases were *every* `*_violated` case in the fixture — the whole negative branch. The `else` arm asserted the class `DependencyVersionMismatchError` and nothing else, so `error_code`, `module_id`, `dependency_id`, `required` and `actual` never reached an assertion. **An implementation that always reported "constraint satisfied" would have passed every case this fixture actually ran**, and the satisfied path is also the one a broken checker silently takes. The driver now dispatches on `expected.outcome` with a hard failure on anything else, asserts the wire code, and asserts the remaining declared fields against the error's wire form (`toJSON().details`, snake_cased per A-D-008) so the fixture's own spelling reaches the assertion without a driver-local key map. This fixture was not in the issue's TypeScript scope — it was found by re-measuring rather than inferring, and the inference would have been wrong.

  Four mechanisms, all in `tests/conformance.test.ts`:

  - **The class name was asserted, never the wire code.** `expect(() => registry.register(…)).toThrow()` — no argument at all — and `toThrow(ContextBindingError)`, `toThrow(DependencyVersionMismatchError)`. The fixture declares `ERROR_CODE_COLLISION` / `VERSION_INCOMPATIBLE` / `CONTEXT_BINDING_ERROR`, and the driver never compared its value to anything, so mutating the declared code left the suite green. Class-name assertions are also structurally unavailable to apcore-rust, which has no such classes, only `ErrorCode` variants — a check one SDK cannot write is not a cross-language contract. The new `assertWireCode()` reads `error.code`.
  - **Branching on key presence, not value.** `if (tc.expected_error)` tests that the key is there and truthy, which every wrong value also is.
  - **An unrecognised expectation skipped the assertion.** `version_negotiation`'s `PARSE_ERROR` fell into `.toThrow()` with no argument — "something threw", satisfied by any error at all. Unknown expectations are now a hard failure naming the case and the value, the `pipeline_failfast_config` "teach the driver, do not skip it" pattern. `PARSE_ERROR` is handled as what the fixture says it is — a deliberately non-wire, language-specific parse failure — and asserted as a native `Error` that is **not** a `ModuleError`, which is the discriminating form. The recognised-code set is read off the SDK's own `ErrorCodes` registry, **not** a driver-local translation table: apcore-typescript#34 item 2 is the record of what a private map in this file costs.
  - **Nine positive cases asserted nothing.** `expected: "ok"` means "no error", and calling `register()` and not throwing satisfies it — an `ErrorCodeRegistry.register()` with an empty body passed all nine, which together with the nine negative cases is how one fixture reached 18 of 18. They now assert an observable post-condition, that the accepted code is queryable through `registry.allCodes`; `unregister_allows_reuse` additionally asserts the released code actually leaves the registry, so the reuse is not the only evidence that `unregister()` did anything.

  Two fixture cases were rewritten upstream and the driver now reads them instead of hardcoding what it expected them to say. `identity_system :: identity_propagates_to_child_context` replaced the prose string `"child.identity === parent.identity"` — a sentence in a value slot — with four fields, each asserted. `context_create :: executor_rejects_cross_executor_rebind` dropped `expected_one_of: [raise, silent_accept]` for `{raises: true, error_code: "CONTEXT_BINDING_ERROR"}` (spec v1.11.0: the old SHOULD permitted a silent acceptance no SDK ever took, so it is a MUST); the driver had hardcoded the raise branch and named the alternation only in a comment. No behaviour change here — this SDK already raised, `tests/core-executor_spec.test.ts` and `tests/integration/test-binding-executor.test.ts` already asserted it, and both stay as they are.

  Also converted, since it is the same shape one key over: eight `if (expected['some_flag']) { assert }` gates in the `context_create` driver, where a fixture stating the opposite would have skipped the assertion rather than failed. `create_rejects_executor_input` was unpinned for exactly that reason and is the case this SDK had that apcore-python did not.

  **No SDK defect surfaced.** Every newly-real assertion passes against the implementation as it stands.

- **`$APCORE_CONFIG_FILE` no longer injects a phantom `config.file` key into the declared document (apcore#88).** The variable is the documented way to point at a configuration file (PROTOCOL_SPEC §9.14 discovery, read by `discoverConfigFile`), but §9.2 also makes *every* `APCORE_*` variable a configuration override and nothing exempted this one. Its suffix lowered to the dot-path `config.file`, so `Config.load(path)` with the variable set produced a declared document carrying `config: { file: '/path/…' }` — a key `schemas/` declares nowhere (checked against `conformance/fixtures/config_key_governance.json`) sitting inside the view §9.1's required-field check runs against, which is what `getDeclared()` reads. It is now dropped at the parse site in `applyEnvOverrides`: the variable is an *argument to* `load()` that happens to share a namespace with configuration, consumed to locate the file and then discarded, which is what every other argument-shaped input does. No spec change and no user-visible rename; `discoverConfigFile()` is unaffected. TypeScript was hit harder than the other two SDKs: `process.env` keeps a variable that was set to the empty string, so even a test that "cleared" `APCORE_CONFIG_FILE` by stubbing it to `''` injected `config.file: ''`. Both legacy and namespace mode were affected and both are pinned by the new `APCORE_CONFIG_FILE is not a configuration override` suite, which asserts the **exact** declared key set rather than the absence of `config.file` — absence alone also holds for an implementation that lost a key the file really declares. The exemption is one variable wide: `APCORE_BINDINGS_DIR` → `bindings.dir` is a declared key and keeps working, asserted by a third case in the same suite. The distinguishing test for any future variable is whether its dot-path is in the canonical key surface.
- **BEHAVIOUR CHANGE: the documented nested `retry:` block on a subscriber is now read from config, on all five built-in types (apcore#85).** `features/event-system.md` documents a per-subscriber retry policy and shows it under a heading reading *"showing the policy on multiple subscriber types"* — an `a2a` entry with `max_attempts: 5` and a `file` entry with `max_attempts: 2`. **No SDK parsed it.** An operator who copied that example got `DEFAULT_RETRY`, silently, with nothing to indicate the block had been ignored: `schemas/sys-modules.schema.json` does not describe subscriber entries beyond requiring a `type`, so nothing rejected the key either.

  The capability was already built at every other layer, which is why this survived. `RetryConfig` (`events/retry.ts`) declares exactly the four keys the document shows, and `EventEmitter._deliver` resolves `subscriber.retry` through `resolveRetry` with no type check and no allowlist, so whatever a subscriber carries is honoured. The single missing layer was config → object: only the `webhook` factory built a policy, and only from the *legacy flat* `retry_count` shorthand. `a2a`, `file`, `stdout` and `filter` never constructed one.

  The new `_parseRetryConfig` in `src/sys-modules/registration.ts` parses the block (snake_case wire keys → camelCase `RetryConfig`) and every built-in factory passes the result through. Partial blocks merge over `DEFAULT_RETRY` (`maxAttempts=3`, `initialBackoffMs=100`, `maxBackoffMs=30000`, `backoffMultiplier=2.0`), as the documented `file` example requires — it declares only two of the four keys. A `retry:` that is not an object is ignored rather than fatal.

  **`FileSubscriber` and `StdoutSubscriber` gained the `retry` field they were missing.** `WebhookSubscriber`, `A2ASubscriber` and `FilterSubscriber` already declared one; the other two did not, so even a caller constructing them directly could not set a policy. Both now take an optional trailing `opts?: { retry?: RetryConfig }` and merge over `DEFAULT_RETRY` exactly as their peers do — a purely additive signature change. Both are real retry surfaces: `FileSubscriber.onEvent` re-throws after logging, and `StdoutSubscriber` writes straight to `process.stdout`, whose failures (EPIPE, closed stream) propagate.

  **Flat `retry_count` still works** for `webhook` as a deprecated alias, with its existing `maxAttempts = retry_count + 1` translation — that spelling is what deployments use today. **The nested block wins when both are present.**

  **This changes delivery behaviour for anyone who had already written the documented block**: a subscriber that was silently retrying 3 times now retries as configured. Pinned by `tests/test-subscriber-retry-config.test.ts`, one case per subscriber type plus an end-to-end case asserting the configured `maxAttempts` drives the real number of `onEvent` invocations. Every asserted value differs from `DEFAULT_RETRY`, so a case cannot pass against a factory that ignores the block.

- **`reload_module` with a `path_filter` now reloads in dependency topological order instead of alphabetical order (#35).** `conformance/fixtures/system_modules_hardening.json` declares `reload_order: "topological"` for the `reload_with_path_filter` case; `_reloadWithPathFilter` called `.sort()` and never consulted the dependency graph. Reloading a dependent before its dependency leaves the dependent briefly wired to a module that is about to be replaced, and which of the two orders was right ended up decided by how the module ids happened to sort alphabetically.

  **This is two fixes, and the second was blocked on the first.** `mergeModuleMetadata` (`src/registry/metadata-pure.ts`) built the stored metadata from a fixed eight-key list — `description`, `name`, `tags`, `version`, `annotations`, `examples`, `metadata`, `documentation` — and `dependencies` was not among them. The consequence was narrow and hard to see: `Registry._resolveLoadOrder` reads `dependencies` off the raw YAML metadata at **discovery** time, before the merge, so LOAD-order topological sorting worked correctly and `resolveDependencies` looked healthy. The merge then dropped the key before it reached `_moduleMeta`, so no **post-registration** reader could ever see it. Adding a topological sort on top of that would have reproduced the defect rather than fixed it — a sort over a graph that is empty by construction is indistinguishable from the alphabetical order it replaced. `dependencies` now merges under the same rule as the other fields, YAML winning over code, spelled with an explicit `!= null` check (as `tags` is) so a deliberate `dependencies: []` overrides a code-declared list instead of falling through to it.

  This is the identical defect apcore-python carried and fixed in `ad2998d`; the two SDKs now agree on both halves.

  **New public accessor.** `Registry.getModuleMetadata(moduleId)` returns the merged metadata for a module (a shallow copy; `{}` when unknown) — the mirror of apcore-python `Registry.get_module_metadata`, and the only public surface carrying fields `ModuleDescriptor` does not model, `dependencies` among them. `ReloadModule._topoSortModules` reads it, filters to edges *within* the matched set (a dependency outside the `path_filter` is not being reloaded, so it constrains nothing) and runs the existing `resolveDependencies` Kahn sort. That sort seeds its queue from a sorted zero-in-degree list, so a module set declaring no dependencies still comes back alphabetically: **the previous behaviour is the degenerate case of the new one, not a separate branch.** A cycle or unresolvable edge warns and falls back to alphabetical rather than failing the reload, matching apcore-python.

  **The canonical fixture cannot discriminate the two orders and still needs a case that can.** Its three modules declare no dependencies on each other, so every permutation is a valid linearization and an assertion against it passes whatever the SDK does. The conformance driver in `tests/conformance.test.ts` now asserts what that case genuinely pins — determinism — and `tests/sys-modules/test-reload-path-filter.test.ts` carries the discriminating cases, where `executor.alpha` depends on `executor.zulu` so the lexicographic order is the wrong one. Promoting a case of that shape into `system_modules_hardening.json` is a spec-repo decision and remains outstanding; apcore-python pins the same contract repo-locally in `test_declared_dependency_reloads_before_its_dependent`.

---

- **`auto_schema: strict` bindings are checked for OpenAI compatibility.** `BindingStrictSchemaIncompatibleError` was defined, documented and exported since 1.0, but **no code path ever threw it** — strict bindings silently produced schemas OpenAI would reject. Detection lives in a new `schema/openai-strict.ts`, separate from `toStrictSchema()` (which `Registry` and `schema-export` use for general, non-OpenAI export). It honours the `x-apcore-keyword` marker left on a union lowered from `oneOf`; without that, TypeScript would have accepted schemas Python and Rust reject.

- **Validation keywords on a schema with no `type` are enforced.** A type-less schema went straight to the combinator path, which produced `Type.Unknown()` when no combinator was present — so `{"minLength": 3}` and `{"minimum": 3}` converted to accept-anything. These appear as `anyOf` / `oneOf` / `allOf` branches and as `additionalProperties` / `items` / `contains` / `not` sub-schemas. They are now applied per §6: enforced on instances of the keyword's own type, **inert on every other** — `{"minimum": 3}` still accepts `"x"`, `[1]` and `null`.

- **A bare `{"required": [...]}` sub-schema is enforced.** It converted to accept-anything, which made every `if` / `then` / `dependentSchemas` condition vacuously true.

- **Array and object validation keywords are no longer dropped.** `minItems`, `maxItems`, `uniqueItems`, `contains`, `minContains`, `maxContains`, `minProperties` and `maxProperties` vanished during conversion. `minContains` / `maxContains` are emitted only alongside a `contains`, per §6.4.4–§6.4.5 — TypeBox enforces them unconditionally, so emitting them bare would turn `{"type": "array", "minContains": 5}` into a schema rejecting every array.

- **Non-scalar `enum` / `const` members compare by value.** Built with `Type.Literal`, which accepts only scalars, `{"type": "object", "const": {"a": 1}}` produced a node rejecting **every** value including the exact object named.

- **`_approval_token` no longer reaches the module contract.** The protocol-level key was stripped only on the path where a module required approval *and* a handler was configured; the other two exits left it in `ctx.inputs`, where `additionalProperties: false` rejected the whole call as an undeclared key.

- **`format` neutralisation is scoped to one check** instead of mutating the process-global `FormatRegistry`, which was order-dependent in one direction and leaked into the host application in the other.

- **`SchemaValidator` defaults to `coerceTypes: false`,** matching the boundary and the other two SDKs. Note the knob is currently a no-op — see its doc comment.

- **ACL fail-open: the executor pipeline never took the async ACL path.** `BuiltinACLCheck.execute` was `async` but called the **synchronous** `acl.check()`, so `ACL.registerAsyncCondition()` was dead API in the runtime — grepping `src/` for `asyncCheck` matched only `acl.ts` itself. The sync evaluator fails a Promise-returning condition closed, which means a `deny` rule guarded by an async condition did not match and a later catch-all `allow` won: the call was **allowed** where apcore-python (which awaits `async_check`) denies it. The step now prefers `asyncCheck()` and falls back to `check()` for duck-typed providers.

- **Namespace-mode config silently discarded legacy `APCORE_*` env overrides.** In namespace mode only the two registered prefixes (`APCORE_OBSERVABILITY`, `APCORE_SYS`) were dispatched and every other `APCORE_*` var fell through with no fallback, so `APCORE_EXECUTOR_DEFAULT__TIMEOUT` did nothing. PROTOCOL_SPEC §9.6.2 routes the `apcore` namespace through the §9.2 legacy merge rules; those overrides are now applied to `merged['apcore']` before per-namespace dispatch, mirroring apcore-python `_load_namespace_mode`.

- **The canonical `obs` namespace is registered.** Only `observability` and `sys_modules` were, while apcore-python registers all three and the canonical `redaction_config.json` fixture names `obs.redaction.regex_patterns` / `obs.redaction.sensitive_keys` normatively — TypeScript *read* those dot-paths without registering the namespace, so `APCORE_OBS_*` env dispatch and `config.namespace('obs')` were unavailable.

- **`minContains: 0` no longer rejects every array with no match.** TypeBox's array check short-circuits on a zero match count before consulting `minContains`, so the JSON Schema 2020-12 §6.4.5 case that makes `contains` vacuously satisfiable was rejected where apcore-python and apcore-rust accept it. When `minContains === 0` the `contains` trio is routed through the applicator evaluator instead (`maxContains` still enforced).

- **A malformed `$ref` with two `#` is no longer silently truncated.** `RefResolver` used JS `split('#', 2)`, whose *limit* semantics drop the tail, so `file.yaml#/a#/bogus` resolved successfully; Python's maxsplit-1 `split("#", 1)` keeps the tail and correctly fails with `SCHEMA_NOT_FOUND`.

- **A symlink cannot escape the schemas directory.** `_assertWithinSchemasDir` ran on a **lexically** resolved path (`node:path.resolve` never touches the filesystem), so a symlink inside the schemas directory pointing outside it passed containment and was then read. Both the ref path and `schemasDir` itself now go through `fs.realpathSync.native()` (non-strict, matching Python's realpath-based `Path.resolve()`).

- **ACL handler-error capture is per-call.** `ACL._lastHandlerError` was a module-level static with read-and-clear and no save/restore, so a nested `check()` consumed the outer call's error and two interleaved `asyncCheck()` calls could read each other's. Replaced with a push/pop capture frame bound at evaluator entry — the JS equivalent of Python's `_handler_error_var` ContextVar token pair.

- **Discovery-time registration reserves the in-flight ID.** `_registerInOrder` awaited an async `onLoad` without reserving the ID in `_inFlight`, so a concurrent `register()` of the same ID slipped into the await gap and silently overwrote the module. Python and Rust both reserve; `registry-system.md` is explicit that "SDKs MUST NOT create per-path exceptions".

- **The `minimal` strategy preset honours the per-instance `ToggleState`.** `buildMinimalStrategy` omitted `deps.toggleState`, falling back to `DEFAULT_TOGGLE_STATE`, so `disable()` on one instance leaked across instances (issue #71). The other four preset builders already passed it.

- **`ContextKey` is exported from the `apcore-js/context-keys` subpath.** The README advertises the subpath as the tree-shakeable entry for the typed-context surface, but the module only re-exported the six pre-built key constants — `import { ContextKey } from 'apcore-js/context-keys'` failed.

### Changed


- Conformance drivers for the union and recursive hardening fixtures now go through `SchemaLoader.resolve()` (RefResolver + converter) rather than calling the converter directly, which had skipped `$ref` resolution entirely.

- **Four conformance fixtures gained real drivers.** `binding_errors.json` and `binding_yaml_canonical.yaml` were permanently `it.skip`'d with the reason "BindingLoader requires real file I/O and dynamic imports" — which was wrong; both assert error-message parity and YAML round-trip exactly as Python and Rust drive them. `registry_load_ordering.json` and `event_delivery_semantics.json` had no TypeScript driver at all. `schema_content_hash.json` is now loaded from the fixture instead of being hand-mirrored in the test file.

- **`examples/` is typechecked.** The root `tsconfig.json` `include` covered only `src/` and `tests/`, so no example was ever compiled by `pnpm typecheck` or CI — which is why two example modules had been failing `tsc` (TS2739 on a partial `ModuleAnnotations` literal) and the only YAML-binding example could neither load nor run. A new `tsconfig.examples.json` (with `apcore-js` mapped to `src/`, so no build is needed) runs as `pnpm run typecheck:examples`, wired into pre-commit and CI. apcore-rust compiles all its examples in CI; this is the TypeScript equivalent.

### Fixed (docs & examples)


- The `examples/bindings/format-date/` binding is loadable and runnable: the target now uses the `(inputs, context)` calling convention `BindingLoader` invokes, exports the `inputSchema` / `outputSchema` that `auto_schema: true` infers from, and ships a `run.ts` driver (the counterpart of apcore-python's `run.py`).
- `examples/modules/get-user.ts` and `send-email.ts` build annotations with `createAnnotations()`; `send-email.ts` marks its `apiKey` input `x-sensitive: true` so it actually demonstrates the redaction the docs claim.
- README: corrected the `sysModules` → `sys_modules` namespace name, the `APCORE_EXECUTOR_DEFAULT_TIMEOUT` → `APCORE_EXECUTOR_DEFAULT__TIMEOUT` env spelling (with the `__` → literal `_` rule stated), the 404 `getting-started.html` docs links, and the `version: "0.22.0"` pin in the config sample. Added the missing `Bindings` and `Execution policy` sections (`ExecutionPolicy` / `PolicyRule` were exported and changelogged but appeared nowhere in the README) and the missing 0.24/0.25/0.26 "What's New" entries.
- `examples/README.md`: documents the required `pnpm build`, lists `v022-tour.ts`, and links the real README "Bindings" section. `examples/v022-tour.ts`: middleware duplicate detection warns rather than throws (the catch was dead code), `Config.registerNamespace` takes a single options object, and the run instructions no longer name the non-dependency `ts-node`.

## [0.26.0] - 2026-07-13

### Added

- **Execution-time governance policy (#76 RFC pilot).** New `ExecutionPolicy` and `PolicyRule` classes plus a `PolicyDecision` type (exported from the package root) let a platform operator override the governance annotations of already-registered modules at execution time — independent of how they were registered. A policy attaches to the `Executor` via a new `policy` option (also on `Executor.fromRegistry` and the `APCore` facade) and the runtime `Executor.setPolicy()` setter, and is consulted by the approval gate (Step 5). Pattern matching reuses the ACL wildcard semantics (Algorithm A08) and specificity scoring (Algorithm A10) via `utils/pattern`; on a specificity tie the more restrictive rule wins. A matched rule overrides the module's own declared/scanned `requiresApproval` / `destructive` annotations, and every policy-driven override is recorded in the audit trail (tracing span event + optional bus event). `ExecutionPolicy.fromObject` parses a parsed YAML/JSON governance document **strictly** — unknown keys throw so a typo cannot silently disable a control. `Executor.validate()` preflight now reports the same `requiresApproval` verdict the gate will enforce under a policy. When the gate is policy-forced, the `ApprovalRequest.annotations` handed to the handler carries the **effective** governance values, preserving the "requiresApproval is guaranteed true" contract (PROTOCOL_SPEC §7). Adds `tests/policy.test.ts` and `examples/execution-policy.ts`.

- **Governance events on the event bus (#77 pilot).** When the `Executor` has an `eventEmitter`, the governance chain now publishes three canonical events: `apcore.approval.decision` on every approval adjudication (handler decisions and the strict fail-closed rejection; severity `info` for approved/pending, `warn` for rejected/timeout), `apcore.policy.override` whenever a policy changes a module's effective governance, and `apcore.acl.denied` (severity `warn`) when an ACL check denies a call. Payloads carry `module_id`, `trace_id`, and event-specific keys (`status`/`approved_by`/`approval_id`, `pattern`/`requires_approval`/`destructive`, or `caller_id`). Canonical names are proposed in apcore#77, pending the PROTOCOL_SPEC §9.16.2 amendment. A skipped approval gate emits nothing (parity with the no-audit-log-when-skipped contract), and the `apcore.acl.denied` event is suppressed during `validate()` preflight (dry-run) so a probe never emits a spurious denial.

### Changed

- **Resolve `destructive` ↔ approval semantics (#76).** `new ExecutionPolicy(rules, { gateDestructive: true })` makes any module whose effective `destructive` annotation is true require approval even when `requiresApproval` is false — the opt-in resolution of the long-standing footgun where an inferred `DELETE` was `destructive=true` yet ungated. Orthogonality remains the default (no behavior change without a policy).

- **Approval gate fails loud, not silent (#76, security principle).** When a module needs approval but no `ApprovalHandler` is configured, the gate keeps the PROTOCOL_SPEC §7.4 skip behavior but now emits a `console.warn` (once per module) instead of silently no-opping. `new ExecutionPolicy(rules, { strict: true })` upgrades this to fail **closed** (throws `ApprovalDeniedError`). A module annotated `destructive=true` that no approval gate covers is likewise warned about once per module. Existing behavior without a policy and with a handler configured is unchanged.

## [0.25.0] - 2026-06-22

### Added

- **Config-driven ACL discovery (#74, D-64).** New `ACL.discover(config)` static method (Node-only, installed via the `acl-file.ts` side-effect module) resolves `acl.root` (default `./acl`) relative to the config file's directory, loads an ACL only if the resolved path exists, and returns `null` otherwise. An `acl.root` that points at a directory loads `<root>/global_acl.yaml` (the directory convention — a directory without that file is a no-op); an `acl.root` that points at a file loads that file directly. **Critical invariant:** a missing path attaches NO ACL — never a synthesized default-deny. Discovery is auto-wired in the `APCore` constructor and skipped when the caller supplies their own `Executor`. Adds new tests and `examples/acl-config-driven.ts`. Cross-language contract locked by the apcore conformance fixture `acl_root_discovery.json`. The directory-convention handling was the fix that brought TS to parity with apcore-python / apcore-rust.

## [0.24.1] - 2026-06-18
### Changed
refactor(context): rename _withExecutor to withExecutor, add deprecation alias

Rename the internal `_withExecutor` method to the public `withExecutor` while retaining a deprecated alias for backward compatibility. Update all internal usages in executor.ts to use the new method name, and add a helper function to handle both method names for robust cross-context executor binding during the deprecation window. Also update JSDoc comments to reflect the new API and apcore spec references.

## [0.24.0] - 2026-06-11

### Added

- **Per-instance `ToggleState` isolation (#71).** Each `APCore` instance now owns one `ToggleState` (`new APCore({ toggleState? })`, exposed read-only via `client.toggleState`) that is injected into BOTH the write path (`ToggleFeatureModule` / `system.control.toggle_feature`) and the read path (`Executor` → `BuiltinModuleLookup`). Disabling a module on one instance no longer affects another instance in the same process, and a disable survives a reload of its own instance (the `ToggleState` lives outside the `Registry`). `registerSysModules` accepts a `toggleState` option and forwards it to `ToggleFeatureModule`; `APCore` constructs and threads the same instance into both the `Executor` and the sys-modules installer. The module-level `DEFAULT_TOGGLE_STATE` remains the fallback for no-injection paths (direct `Executor` / `ToggleFeatureModule` construction without a `toggleState`). Re-scopes A-D-12 from process-global to instance-scoped. Locked by the shared conformance fixture `toggle_state_isolation.json`.
- **Agent-governance conformance coverage (#72).** New conformance drivers wire the canonical fixtures `toggle_state_isolation.json` (4 cases) and `acl_agent_scoping.json` (19 cases). The ACL fixture locks the AI-agent tool-governance scenario (`@external` < `reader` < `data_admin` privilege gradient, role + `max_call_depth` conditions, first-match-wins) as a cross-language contract; all decisions match with no ACL-engine change required (`max_call_depth` is inclusive — depth == cap is allowed).

### Fixed

- **`Registry.unregister()` clears hot-reload/drain state [A-D-001].** A direct `unregister()` (not via `safeUnregister()`) previously left stale entries in `_draining`, `_refCounts`, and `_drainResolvers`. Re-registering the same module id afterward made `acquire()` throw `ModuleNotFoundError` because the leftover `_draining` flag persisted. `unregister()` now deletes these entries alongside `_modules` / `_moduleMeta` / `_schemaCache` / `_lowercaseMap`, matching apcore-python (`registry.py`) and apcore-rust (`registry.rs`).
- **Sensitive-key redaction recurses into arrays [A-D-003].** The `_secret_*` prefix/keyword redaction walk in `redactSensitive` skipped array values, so sensitive keys inside objects nested in arrays (e.g. `{ items: [{ _secret_api_key: "k" }] }`) were not redacted. The walk now recurses into array elements (and nested arrays), matching apcore-python `utils/redaction.py` `_redact_in_list`.
- **Env value coercion matches Python `int()`/`float()` [A-D-008].** `coerceEnvValue` previously kept integral strings like `08`, `+5`, and exponent forms like `1e0` as strings because of a `String(parsed) === value` round-trip guard. It now coerces integral strings to integers (leading zeros / signs accepted) and decimal/exponent strings to numbers, aligning with apcore-python `_coerce_env_value` and apcore-rust `coerce_env_value`.
- **Before-middleware failure no longer fires `on_error` twice [A-D-011].** When a middleware `before()` hook raised, the built-in `middleware_before` step ran the `on_error` chain itself AND the executor re-ran it, double-firing stateful handlers. With `RetryMiddleware` this double-incremented the retry counter, halving the retry budget. The before-step now records the executed middlewares and re-raises, leaving the executor as the sole `on_error` site (matching apcore-python `builtin_steps.py`).
- **`ACL.removeRule(..., null)` ignores conditions when matching [A-D-016].** An explicit `null` conditions argument was treated as "conditions must equal null", so rules that carried conditions could not be removed without re-specifying them. Explicit `null` is now treated the same as an omitted argument — conditions are ignored during matching — matching apcore-python `acl.py` and apcore-rust. A concrete conditions object still disambiguates by deep equality.
- **`CALL_DEPTH_EXCEEDED` / `CIRCULAR_CALL` / `CALL_FREQUENCY_EXCEEDED` error `details` keys are now `snake_case` [A-D-019].** These errors previously serialized `details` with `camelCase` keys (`maxDepth`, `moduleId`, `maxRepeat`, `callChain`), diverging from the spec, apcore-python, and apcore-rust. The keys are now `max_depth` / `module_id` / `max_repeat` / `call_chain`. The error CODE strings are unchanged, and the public TS getters (`maxDepth`, `moduleId`, `maxRepeat`) keep their `camelCase` names.


## [0.23.0] - 2026-06-10

### Changed (breaking)

- **Webhook/A2A subscriber wire keys are now `snake_case` [D11-003].** `WebhookSubscriber` and `A2ASubscriber` previously serialized the HTTP body with `camelCase` keys (`eventType`, `moduleId`), diverging from apcore-python / apcore-rust and violating the protocol spec requirement that wire JSON MUST be `snake_case` (`protocol-spec.md`). The Webhook body and the A2A inner `event` object now emit `event_type` / `module_id` (other keys unchanged). The A2A outer wrapper key `skillId` is unchanged. In-memory `ApCoreEvent` properties (`event.eventType`, `event.moduleId`) are unaffected — only the serialized wire body changed. **BREAKING for receivers parsing `eventType` / `moduleId` from the wire.**
- **DLQ `original_event` nests envelope fields under `metadata` [D11-002].** The `apcore.event.delivery_failed` (DLQ) payload's `original_event` object now uses the canonical `{name, payload, metadata}` shape (`event-system.md`): `module_id` and `timestamp` moved from the top level of `original_event` into a nested `metadata` object. **BREAKING for DLQ consumers reading `original_event.module_id` / `original_event.timestamp` — read `original_event.metadata.module_id` / `original_event.metadata.timestamp`.**
- **`RetryMiddleware` now performs real retries via `RetrySignal` [D1-001].** The built-in `RetryMiddleware` was a silent no-op: its `onError` only recorded advisory hint keys in `context.data` and returned `null`, so the error always propagated and the module was never re-invoked — diverging from apcore-python (returns `RetrySignal`) and apcore-rust (returns `Ok(Some(inputs))`). On a retryable error while attempts remain, `RetryMiddleware.onError` now sleeps for the configured backoff delay and returns a `RetrySignal` carrying the original inputs, causing the executor to re-run the module; non-retryable errors and exhausted retries still propagate. A new `after()` hook clears the per-module retry counter on success to prevent unbounded `context.data` growth. `RetryHintMiddleware` is now a deprecated alias for the real `RetryMiddleware` (the advisory no-op variant is removed). **BREAKING for code that relied on `RetryMiddleware` not re-invoking the module or on the `_apcore.mw.retry.delay_ms.*` hint key.**

### Added

- **AI error-recovery metadata is now populated at the source (#70).** Framework-deterministic errors carry a default `userFixable` resolved from the error code via the new `USER_FIXABLE_BY_CODE` policy in the `ModuleError` constructor, so the recovery contract flows to every surface (MCP/CLI/A2A) from one definition. `true` for caller-fixable codes (`SCHEMA_VALIDATION_ERROR`, `GENERAL_INVALID_INPUT`, `MODULE_NOT_FOUND`, `VERSION_CONSTRAINT_INVALID`, `BINDING_SCHEMA_INFERENCE_FAILED`, `BINDING_SCHEMA_MODE_CONFLICT`, `BINDING_STRICT_SCHEMA_INCOMPATIBLE`, `DEPENDENCY_NOT_FOUND`, `DEPENDENCY_VERSION_MISMATCH`); `false` for governance/system/structural/transient codes (`ACL_DENIED`, `APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `MODULE_TIMEOUT`, `MODULE_DISABLED`, `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL`, `CALL_FREQUENCY_EXCEEDED`, `GENERAL_INTERNAL_ERROR`); unlisted codes (e.g. `MODULE_EXECUTE_ERROR`) leave it unset for the module author. Default `aiGuidance` filled on `InvalidInputError` and `CallFrequencyExceededError`. Explicit constructor values still override the policy; serialization stays sparse. Locked by the shared conformance fixture `error_recovery_metadata.json` — at parity with apcore-python / apcore-rust 0.23.0.


### Fixed

- **`A2ASubscriber` no longer retries 4xx responses (#69).** It previously threw on any HTTP `status >= 400`, contradicting the spec (`event-system.md`: 4xx MUST NOT be retried, for both Webhook and A2A) and diverging from `WebhookSubscriber`. `A2ASubscriber.onEvent` now mirrors Webhook: 5xx (and connection/timeout) → throw → retried → `apcore.event.delivery_failed` on exhaustion; 4xx → logged permanent, no throw, no retry, no DLQ. Per-SDK regression tests lock both subscribers' 4xx/5xx behavior.


## [0.22.0] - 2026-05-28

### Changed

- **Middleware circuit-breaker state enum public export renamed `MiddlewareCircuitState` → `CircuitBreakerState` for cross-SDK naming parity (audit D2-001).** The middleware circuit-breaker state enum is now publicly exported as `CircuitBreakerState`, matching apcore-rust's `CircuitBreakerState` (apcore-python names the equivalent `CircuitState`). The internal enum in `src/middleware/circuit-breaker.ts` was also renamed to `CircuitBreakerState`. The unrelated events circuit-breaker enum (`CircuitState`, exported from `./events`) is unchanged. **BREAKING for importers of `MiddlewareCircuitState` — switch to `CircuitBreakerState`.** No deprecation alias is provided.

- **`Context.create()` signature unified across SDKs ([apcore#66](https://github.com/aiperceivable/apcore/issues/66)).** Per the v0.22.0 spec §"Contract: Context.create", the public input list is now `(identity, traceParent, cancelToken, data, services, globalDeadline)`. **BREAKING:**
  - `executor` has been removed as a parameter. The Executor now auto-binds itself to the Context on the first `call()` / `callWithTrace()` / `stream()` / `validate()` entry (see §"Contract: Executor binding to Context"). Idempotent for the same Executor instance; cross-executor rebind raises the new `ContextBindingError` (`code: CONTEXT_BINDING_ERROR`).
  - `callerId` is no longer accepted as an input — it has always been managed exclusively by `Context.child()`; this just makes the contract explicit.
  - `cancelToken` is now a first-class slot at position 3, replacing the cast-through-`unknown` workaround in `AsyncTaskManager._buildTaskContext` and the manual `new Context(...)` construction pattern documented in `examples/cancel-token.ts`.
  - Two new internal helpers, `Context._withExecutor(executor)` and `Context._withCancelToken(token)`, return a new immutable Context with the given field bound; same-instance rebind is a noop, cross-instance rebind throws `ContextBindingError`.
  - Call-site migration: `Context.create(executor, identity)` → `Context.create(identity)`; `Context.create(null, null, undefined, traceParent)` → `Context.create(null, traceParent)`; `Context.create(executor)` → `Context.create()`. A new `tests/conformance.test.ts` block drives the canonical `context_create.json` fixture against the SDK to lock in cross-language behavioral parity.

### Added

- **Per-module `resources.timeout` honored by `BuiltinExecute` (spec D-11).** Modules MAY declare their own timeout in milliseconds via the top-level `resources.timeout` field or `annotations.resources.timeout`; the executor uses the module value ahead of `executor.default_timeout`, and the global deadline (when set) further clamps the effective timeout. Closes finding A-D-EXEC-001.
- **`CancelToken.signal` and `Context.signal` (spec D-18).** `CancelToken` is now backed by an `AbortController` and exposes its `AbortSignal`. `Context.signal` returns the bound token's signal or a never-aborted fallback so modules using standard Web-API I/O (`fetch`, `setTimeout` via `AbortSignal.timeout`, Web Streams) participate in real abort. `BuiltinExecute` races module execution against the cancel signal so cancellation surfaces as a typed `ExecutionCancelledError` even when the module is sitting on a non-Web-API await point. `AsyncTaskManager.cancel()` now interrupts in-flight executor calls instead of merely flagging cooperative state. Closes A-D-AT-02.

### Changed

- **`TaskStore` interface is fully asynchronous (spec D-17).** Every method on `TaskStore` now returns `Promise<T>` (`save`, `get`, `list`, `delete`, `listExpired`) so Redis/SQL/HTTP-backed stores can plug in without blocking the event loop. `InMemoryTaskStore` exposes async signatures even though its operations are CPU-only — uniform shape lets stores compose generically. `AsyncTaskManager.getStatus`, `getResult`, `listTasks`, `cleanup`, and the reaper sweep are now `async`. **BREAKING for custom `TaskStore` implementations and direct `await mgr.getStatus(id)` / `await mgr.getResult(id)` / `await mgr.listTasks()` callers — update to `await`.** Closes A-D-AT-04.
- **`AsyncTaskManager.max_tasks` counts active tasks only (A-D-AT-01).** Tasks in `PENDING` or `RUNNING` status count toward `max_tasks`; completed/failed/cancelled tasks remain in `_internal` for bookkeeping but no longer cap new submissions. Mirrors apcore-python `_ACTIVE_STATUSES`.
- **`AsyncTaskManager` preserves `startedAt` across retries (A-D-AT-08).** `_enqueue` no longer resets `startedAt`/`completedAt` between retries — `startedAt` reflects the first-run wall-clock, matching apcore-python and apcore-rust.
- **`callWithTrace` shares `call()`'s error semantics (spec D-19).** The trace variant now unwraps `MiddlewareChainError`, propagates the typed cause via `propagateError`, applies the cancellation short-circuit (D-20), and runs the `on_error` middleware chain. Middleware that returns a recovery dict has its result paired with the trace; otherwise the wrapped error rethrows. Closes A-D-EXEC-004.
- **`Registry.register()` sync `onLoad` failure now emits `apcore.registry.module_load_failed` (spec #65 strong-guarantee invariant).** The sync-onLoad branch previously bubbled the throw without firing the event, leaving subscribers blind to sync init failures. `register()` now always returns a Promise; sync `onLoad` failures surface as a rejection. **BREAKING for callers that relied on synchronous throw from `register()` with a sync `onLoad` — switch to `await registry.register(...)`.** Closes A-D-REG-001.
- **Deferred-publish applies to `_registerImpl` (discover path) and `registerInternal` too (spec REG-003).** Both paths now reserve `_moduleMeta` + `_lowercaseMap` then run `onLoad`, only inserting into the visible `_modules` map on success. Failures roll back the reservation and emit `apcore.registry.module_load_failed`. Aligns every registration path with the Issue #65 strong-guarantee invariant. Closes A-D-REG-003 and A-D-REG-004.
- **`WebhookSubscriber` and `A2ASubscriber` use the unified `EventEmitter` retry policy.** Per spec event delivery semantics (#61), the subscriber-internal retry loop in `WebhookSubscriber` and the silent error swallow in `A2ASubscriber` have been removed. Both subscribers now rethrow on 5xx / network errors (Webhook 4xx still returns normally), letting `EventEmitter._deliver` apply the spec-default retry policy (3 attempts, 100 ms initial backoff, 2× multiplier, 30 s cap). Both classes expose a public `retry: RetryConfig` field and accept a `{ timeoutMs, retry, id }` options object as their third constructor argument. The legacy `retry_count` YAML field is automatically translated to `retry.maxAttempts = retry_count + 1` by `registerSysModules`. Closes A-D-EVT-001.
- **`engines.node` bumped to `>=20.0.0`.** D-18's real-abort implementation targets Node 20+ idioms (`AbortSignal` composability is fully reliable on 20+). Node 18 LTS reached EOL on 2025-04 and is no longer supported.
- **Subscribers without an explicit `retry` config now honor the spec-default 3 attempts (A-D-005).** `resolveRetry(undefined)` previously returned `maxAttempts: 1` and `EventEmitter.emit()` routed retry-less subscribers through a single-attempt path (`_emitSingleAttempt`), so a custom subscriber that omitted `retry` got fire-and-forget delivery — contradicting event-system.md §Event Delivery Semantics (default `max_attempts: 3`, which MUST be honored uniformly for built-in and user subscribers) and the unified-retry change above. Absent `retry` now resolves to the full `DEFAULT_RETRY` policy and every subscriber routes through `_deliver`; an explicit `retry: { maxAttempts: 1 }` still disables retry. Aligns with apcore-python and apcore-rust. Found via `/apcore-skills:sync --scope core`.

### Removed

- **`emitWithLegacy()` helper and legacy event aliases ([apcore#36](https://github.com/aiperceivable/apcore/issues/36)).** Per spec v0.22.0 finalization, the legacy event names `module_registered`, `module_unregistered`, `error_threshold_exceeded`, and `latency_threshold_exceeded` are no longer emitted alongside their canonical `apcore.<subsystem>.<event>` counterparts. The `emitWithLegacy` helper has been deleted from `src/events/emitter.ts` and the public surface. Subscribers MUST listen on the canonical names (`apcore.registry.module_registered`, `apcore.health.error_threshold_exceeded`, etc.). Closes finding A-D-EVT-002.

### Added

- **`ContextKey<T>` promoted to documented public API ([apcore#63](https://github.com/aiperceivable/apcore/issues/63)).** Added `./context-keys` subpath export in `package.json` for direct import of built-in context key constants. All 6 built-in constants (`TRACING_SPANS`, `TRACING_SAMPLED`, `METRICS_STARTS`, `LOGGING_START`, `REDACTED_OUTPUT`, `RETRY_COUNT_BASE`) verified against spec §1.5. `ContextKey` and all constants continue to be exported from the main entry point.

- **`StreamingModule` interface with `STREAMING_MARKER` symbol ([apcore#62](https://github.com/aiperceivable/apcore/issues/62)).** Introduces `STREAMING_MARKER = Symbol.for('apcore.streaming')` and the `StreamingModule` interface extending `Module`. `isStreamingModule()` guard provides detection with a transitional duck-typing fallback (warns once per instance, will be removed in next major). `StreamingInterfaceError` is thrown at registration time if a module declares `annotations.streaming=true` but does not implement the interface. Exports: `STREAMING_MARKER`, `isStreamingModule`, `StreamingModule` (type), `StreamingInterfaceError`.

- **Duplicate middleware detection in `MiddlewareManager` ([apcore#64](https://github.com/aiperceivable/apcore/issues/64)).** `add(middleware, opts?)` now accepts `{ allowDuplicate?: boolean; identityKey?: string }`. Identity defaults to `constructor.name`; on a collision, a `console.warn` with both prior and current registration stack traces is emitted. `allowDuplicate: true` or distinct `identityKey` suppress the warning. `remove()` cleans up the identity registry so re-registration after removal does not warn.

- **Unified event delivery semantics: per-subscriber retry, DLQ, and `onFailure` ([apcore#61](https://github.com/aiperceivable/apcore/issues/61)).** New `src/events/retry.ts` provides `RetryConfig`, `DEFAULT_RETRY` (3 attempts, 100 ms initial backoff, 2× multiplier, 30 s cap), `resolveRetry()`, and `computeDelayMs()`. `EventSubscriber` gains optional `subscriberId`, `eventPattern` (glob), `retry`, and `onFailure` fields. Subscribers without `retry` keep the existing fire-and-forget behavior (backward-compatible). Subscribers with `retry` get exponential-backoff retries; after exhaustion a `apcore.event.delivery_failed` DLQ event is emitted and `onFailure` is called. DLQ delivery is single-attempt (errors logged and discarded). `eventPattern` glob-filters which events each subscriber receives.

- **Deferred-publish for async `onLoad` in `Registry` ([apcore#65](https://github.com/aiperceivable/apcore/issues/65)).** `register()` now returns `Promise<void>`. All sync validation (ID format, duplicate detection, streaming annotation check) still throws synchronously for backward compat. For modules whose `onLoad()` returns a Promise, the module is NOT visible via `get()`/`has()` until the Promise resolves; `_moduleMeta` and `_lowercaseMap` are committed immediately for conflict detection. Async `onLoad` failure rolls back all state. Concurrent registration of the same ID while a prior onLoad is in-flight throws synchronously (`InvalidInputError`).

- **`Config.reservedNamespaces` static getter + top-level `RESERVED_NAMESPACES` export (PROTOCOL_SPEC §9.9.5, [apcore#60](https://github.com/aiperceivable/apcore/issues/60)).** Implements the new normative requirement that all SDKs MUST expose a public, read-only query API returning the set of reserved top-level namespace names. Typed as `ReadonlySet<string>` so caller-side mutation attempts fail at compile time. Returns the existing private `_RESERVED_NAMESPACES` `Set` — single source of truth, no parallel list — so `Config.registerNamespace({ name: 'apcore' })` continues to throw `ConfigNamespaceReservedError` and the query API reports exactly the names that drive that enforcement. Static access (no `new Config()` needed). Intended for third-party consumers (custom CLIs, framework integrations) that accept user-supplied namespace names and need fail-fast pre-validation. The private constant `_RESERVED_NAMESPACES` is unchanged — internal callers keep using it.

### Fixed

Cross-language consistency fixes from `/apcore-skills:sync --scope core` review (2026-05-26):

- **Schema validation error code aligned to canonical `SCHEMA_VALIDATION_ERROR` (A-D-033, A-D-034).** `SchemaValidator.validate()` previously emitted `SCHEMA_VALIDATION_FAILED`, contradicting protocol-spec §8.2 and apcore-rust (both `SCHEMA_VALIDATION_ERROR`). Additionally, `validationResultToError` discarded `result.errorCode`, so union codes (`SCHEMA_UNION_NO_MATCH` / `SCHEMA_UNION_AMBIGUOUS`) detected by the validator were lost when raising. `SchemaValidationError` now accepts an `errorCode` argument; `validate_input` / `validate_output` route through `validate()` so the thrown error carries the precise union code. Conformance fixtures and `schema-system.md` updated to the canonical code.
- **`Context.redactedOutput` now round-trips through serialization (A-D-009).** The `Context` constructor gained a `redactedOutput` parameter (last positional, defaults `null`) and `deserialize`/`fromJSON` now restore `redacted_output`, matching Python/Rust. Binding helpers (`_withExecutor`, `_withCancelToken`) preserve it.
- **ACL `max_call_depth` fails closed on fractional thresholds (A-D-013, security).** The handler now requires `Number.isInteger(threshold)` for both the bare-number and `{ lte }` forms; a non-integer threshold (e.g. `5.5`) no longer matches, matching the Python/Rust fail-closed behavior.
- **Cancellation bypasses `on_error` recovery even when step-wrapped (A-D-003, A-D-004; spec D-20).** `call()`, `stream()`, and `callWithTrace()` now re-check the unwrapped `PipelineStepError.cause`; a step-thrown `ExecutionCancelledError` rethrows before any middleware `onError` runs.
- **`callWithTrace` forwards `versionHint` (A-D-005; spec D-19).** Added a `versionHint` parameter so the trace variant shares `call()`'s version-resolution semantics.
- **Stream Phase-3 post-validation failure emits `apcore.stream.post_validation_failed` (A-D-006).** The `Executor` constructor accepts an optional `eventEmitter`; when wired, a post-stream output-validation failure emits the event (matching apcore-python). Behavior is unchanged when no emitter is provided.
- **`Registry.get()` returns `null` for an in-flight (mid-registration) id (A-D-002).** Previously threw `ModuleNotFoundError`; now returns `null`, matching Python/Rust, `getDefinition()`, and the well-formed-unregistered → null contract.
- **Flat-style namespace env keys are lowercased verbatim (A-D-048).** `resolveEnvSuffix` no longer collapses `__`→`_` or strips a leading `_` for `env_style: 'flat'`; `MYAPP_FOO__BAR` → `foo__bar`, matching Python/Rust.
- **`EventEmitter.shutdown()` gates `emit()` (A-D-028).** Added `shutdown()`; `emit()` is a no-op after shutdown, matching Python/Rust drop-on-shutdown.
- **DLQ events are not delivered to catch-all subscribers (A-D-026).** Subscribers with no pattern or pattern `'*'` no longer receive `apcore.event.delivery_failed`, preventing delivery-failure cascades. Explicitly-scoped DLQ subscribers are unaffected.
- **DLQ `subscriber_type` reads a declared field (A-D-029).** `EventSubscriber.subscriberType` is read first, falling back to constructor-name derivation. Built-in subscribers (`Webhook`/`A2A`/`File`/`Stdout`/`Filter`) expose their declared type.
- **`EventEmitter.flush()` uses per-pending timeout semantics (A-D-027).** Each pending delivery now gets up to `timeoutMs` to settle, instead of sharing a single total deadline, matching Python.
- **`MiddlewareManager.add()` rejects priority > 1000 (A-D-017)** and applies first-registration-wins identity bookkeeping (A-D-019/021/022): a duplicate no longer overwrites the recorded registration, and identity is recorded even under `allowDuplicate` (only the warning is suppressed). Default identity remains `constructor.name` (A-D-018(a) deferred — module-qualified identity is not reliably available at runtime in JS).
- **README Node requirement corrected to Node 20+ (B-006).** The badge and Requirements section said Node 18 while `engines.node` is `>=20.0.0`.
- **`$ref` depth-cap exhaustion now throws `SchemaMaxDepthExceededError` (A-D-038).** When `RefResolver` reaches `schema.max_ref_depth` it previously threw `SchemaCircularRefError` (`SCHEMA_CIRCULAR_REF`), conflating the depth cap with genuine cycle detection. It now throws the new `SchemaMaxDepthExceededError` carrying the dedicated `SCHEMA_MAX_DEPTH_EXCEEDED` code, reserving `SCHEMA_CIRCULAR_REF` for actual cycles — aligning with apcore-rust and PROTOCOL_SPEC §8.2 / `error-system.md`. The new error class is additive (non-breaking) and exported from both the Node and browser entries; note the **error code returned for the depth-cap path changes** from `SCHEMA_CIRCULAR_REF` to `SCHEMA_MAX_DEPTH_EXCEEDED`.

---

## [0.21.1] - 2026-05-09

Patch release. Fixes the init-time deadlock observed under older Bun
runtimes when `apcore-js` is consumed via the `apcore-cli` /
`apcore-toolkit` chain, and ships a separate browser build so consumers
that bundle apcore-js for the browser stop pulling Node built-ins into
their bundle. Public Node API is unchanged — every existing
`import { ... } from 'apcore-js'` keeps working.

### Fixed

- **Init-time deadlock under Bun (root cause: 18 top-level
  `await import('node:*')` chains).** The browser-compat soft-import
  pattern (`let _nodeFs = null; try { _nodeFs = await import('node:fs') }
  catch {}` at module top) chained through the import graph and
  deadlocked Bun 1.2.x at module-resolution time. Every soft-import has
  been converted: Node-only files now use static `import * as fs from
  'node:fs'`; browser-shared files keep no top-level `await` at all.
  Touches `acl.ts`, `bindings.ts`, `config.ts`, `schema/loader.ts`,
  `schema/ref-resolver.ts`, `registry/scanner.ts`,
  `registry/metadata.ts`, `utils/index.ts`. `registry/registry.ts`
  retains its existing in-method lazy `await import(...)` pattern,
  which is browser-bundler-safe and never fired the deadlock.

### Added

- **Browser build (`./dist/browser/index.js`) selected automatically
  via `package.json` `exports.browser`.** Consumers keep the unified
  `import { APCore, Registry, Executor, ... } from 'apcore-js'` —
  bundlers (Vite / Next.js / webpack 5 / esbuild / Rollup / Parcel)
  pick the browser build at resolve time; Node and Bun pick the Node
  build via the `node` / `default` condition. A top-level
  `"browser": "./dist/browser/index.js"` field is also published as a
  fallback for legacy webpack 4. **No `./browser` subpath is exposed**
  — the unified entry is the only supported import string.
- **`contentHashAsync(schema)`** — WebCrypto SubtleCrypto-backed
  SHA-256 of the canonical JSON form. Output matches the existing
  Node-only sync `contentHash` byte-for-byte (PROTOCOL_SPEC
  §schema-system §4.15.5). Use this from browser code; the sync
  variant remains Node-only because `node:crypto.createHash` is
  unavailable in the browser.
- **`tests/browser-entry.test.ts`** — static import-graph guard +
  runtime contract tests. Walks every relative static import reachable
  from `src/browser/index.ts` and fails CI if any `node:*` reference
  appears in the closure. Pins the runtime contracts: `ACL.load()`
  throws a guidance error in browser builds, `Registry` +
  `FunctionModule` + `Executor` + `AutoApproveHandler` chain runs
  end-to-end in browser, and `contentHashAsync` agrees with the sync
  `contentHash` digest across a fixture set (cross-language hash
  alignment).

### Changed

- **Side-effect installer pattern for Node-only methods on browser-
  shared classes.** `ACL.load(yamlPath)`, `registerSysModules`'s
  `overridesPath` file loader, `TracingMiddleware`'s
  `@opentelemetry/api` auto-detection, and `APCore`'s sys-modules
  auto-registration all moved into Node-only side-effect modules
  (`src/acl-file.ts`, `src/sys-modules/overrides-file.ts`,
  `src/middleware/tracing-otel-default.ts`, `src/sys-modules/install.ts`)
  imported by the Node entry. Browser bundles never reach these
  files, so `node:fs` / `node:module` stay out of the closure; calls
  to the Node-only methods from browser code throw a clear error
  directing the caller to construct the equivalent in-memory.
- **`Config.isBrowser()` deprecated** (`@deprecated`). Environment
  detection is now bundler-time via the `exports` conditions. The
  Node build always returns `false`; the browser build never imports
  `Config` at all. Method retained for downstream code that
  historically branched on it; will be removed in a future minor.
- **`package.json` `sideEffects` from `false` to an explicit allow-
  list** (`./dist/index.js`, `./dist/acl-file.js`,
  `./dist/middleware/tracing-otel-default.js`,
  `./dist/sys-modules/install.js`,
  `./dist/sys-modules/overrides-file.js`). The previous blanket
  `false` would let aggressive tree-shakers remove the side-effect
  installer imports, leaving Node consumers with the throw stubs.
- **Pure helpers extracted into runtime-neutral files** — `DEFAULTS`
  / `getDefault` from `config.ts` → `src/config-defaults.ts`;
  `parseDependencies` / `mergeModuleMetadata` from
  `registry/metadata.ts` → `src/registry/metadata-pure.ts`;
  `jsonSchemaToTypeBox` / `contentHashAsync` /
  `sortedKeysStringify` from `schema/loader.ts` →
  `src/schema/loader-pure.ts`. Consumers continue importing from the
  original paths (the originals re-export); browser entry imports
  from the pure files directly.
- **`tsconfig.build.json` `stripInternal: true`.** Symbols marked
  `@internal` (`_setAclFileLoader`, `_setSysModulesInstaller`,
  `_setOverridesLoader`, `_setDefaultTrace`, `_parseAclRule`,
  `ACL._setYamlPath`) used by the side-effect installer pattern are
  now omitted from the published `.d.ts` bundle.
- **`utils/randomHex` no longer imports `node:crypto`.** Uses
  `globalThis.crypto.getRandomValues` exclusively (available in Node
  ≥ 19 and every browser / edge runtime). The `node:crypto.webcrypto`
  fallback was dead code on every supported runtime.

### Migration

No source code changes required for existing Node consumers; the
public API surface is byte-identical from the Node entry. Browser
consumers that previously got a partial runtime via the dynamic-
import soft-load pattern (e.g. `import { Registry } from 'apcore-js'`
in a Next.js client component) keep working unchanged — the bundler
now resolves to the curated browser build instead, which exposes
every runtime symbol the soft-import pattern previously exposed
minus the filesystem-bound classes (`Config`, `BindingLoader`,
`SchemaLoader`, `RefResolver`, `contentHash` sync). Code that was
already calling those in the browser was already broken at runtime;
it now fails at import resolution with a clearer error.

Cross-SDK parity unchanged. PROTOCOL_SPEC version unchanged.


## [0.21.0] - 2026-05-06

Aligns apcore-typescript with PROTOCOL_SPEC.md v0.21.0 (apcore commit
[`c191b85`](https://github.com/aiperceivable/apcore/commit/c191b85) — RFC
`docs/spec/rfc-ephemeral-modules.md` promoted to `Accepted`). Mirrors the
[apcore-python PR #26](https://github.com/aiperceivable/apcore-python/pull/26)
reference implementation.

### Added

- **`ephemeral.*` namespace reservation (PROTOCOL_SPEC §2.5 / RFC
  `rfc-ephemeral-modules`).** New exported constant
  `EPHEMERAL_NAMESPACE_PREFIX = "ephemeral."` and `isEphemeralModuleId(id)`
  helper. Filesystem discovery (`Registry._discoverDefault` /
  `Registry._discoverCustom`) rejects any module ID falling under the
  `ephemeral.*` namespace — the default-discoverer raises `InvalidInputError`
  with a message pointing the caller to `Registry.register()`, the custom
  discoverer skips the entry with a `console.warn`. The namespace is reserved
  for programmatically-registered modules synthesized at runtime
  (Agent-synthesized tools, on-the-fly composition).
- **`ModuleAnnotations.discoverable: boolean` (PROTOCOL_SPEC §4.4).**
  Defaults to `true`; declared optional on the interface so v0.20.x
  callers building literals keep compiling. When set to `false` the
  module is hidden from `Registry.list()`, `Registry.iter()`, and
  `Registry.moduleIds` — but remains callable by exact ID through
  `get()` / `has()` / `Executor.execute()`. Pass
  `Registry.list({ includeHidden: true })` (or
  `iter({ includeHidden: true })`) to enumerate every registered module
  (mirrors apcore-python's `include_hidden` kwarg). `ephemeral.*` modules
  SHOULD set `discoverable: false`.
- **Audit-event single-emit rule for `ephemeral.*` registrations.** New
  `Registry.setEventEmitter(emitter)` wires an `EventEmitter` onto the
  registry; ephemeral.* `register()` / `unregister()` calls emit exactly
  one canonical `apcore.registry.module_registered` /
  `apcore.registry.module_unregistered` event with the D-35 contextual
  payload (`caller_id` defaulting to `"@external"`, `identity` snapshot,
  `namespace_class: "ephemeral"`). The bridge in
  `sys-modules/registration.ts` short-circuits on `ephemeral.*` IDs so
  the empty-payload bridge emit does not double-fire — one registration,
  one event. Non-ephemeral modules retain the existing empty-payload
  bridge behavior verbatim.
- **`Registry.register()` / `Registry.unregister()` accept an optional
  `{ context?: Context | null }` argument** (5th positional / 2nd
  positional respectively). Forwards `Context.callerId` and
  `Context.identity` into the ephemeral.* audit-event payload. Ignored
  for non-ephemeral modules.
- **Soft-warning when an `ephemeral.*` module is registered without
  `requiresApproval: true`.** `Registry.register()` emits
  `console.warn(...)` per the RFC ("agent-synthesized modules SHOULD
  declare `requires_approval: true` so a human gates execution"). The
  registry never refuses the registration — warning only.
- **`Registry.registerInternal()` rejects `ephemeral.*` IDs.** Throws
  `InvalidInputError` with a clear pointer to `Registry.register()`.
  Per the RFC's "register_internal() interaction" rule, namespace
  prefix → registration mechanism is a 1:1 mapping: `system.*` only
  via `registerInternal()`, `ephemeral.*` only via `register()`. Mixing
  the two backdoors blurs the audit-trail distinction between
  framework-emitted (`system.*`) and caller-emitted (`ephemeral.*`)
  modules.
- **PreflightResult.predictedChanges finalized
  ([#29](https://github.com/aiperceivable/apcore-typescript/pull/29)).**
  Stage 2 of the v0.21.0 alignment, shipped to `main` ahead of this
  release: the optional `predictedChanges?: Change[]` field on
  `PreflightResult` plus the `Module.preview()` method, the `Change` /
  `PreviewResult` types, and the TypeBox `TChange` / `TPreviewResult`
  schemas (with `Type.Unsafe` + `patternProperties` for `x-*` extension
  keys per [iter-11]). v0.21.0 finalizes this surface alongside the
  Stage 3 ephemeral pilot.
- 21 new tests covering namespace reservation, filesystem-discovery
  rejection, `discoverable` filter on `list` / `iter` / `moduleIds`,
  audit-event single-emit, soft-warn on missing `requiresApproval`, and
  `registerInternal` rejection (`tests/registry/test-ephemeral-namespace.test.ts`).

### Changed

- **`Registry.discoverMultiClass` signature cleanup
  ([#28](https://github.com/aiperceivable/apcore-typescript/pull/30) /
  apcore decision-log D-06).** Already on `main` ahead of this release.
  The 4th `multiClassEnabled` argument is dropped from the canonical
  method surface; the method is now
  `discoverMultiClass(filePath, classes, extensionsRoot?)`. Per-class
  opt-in via `ClassDescriptor.multiClass?: boolean` is the sole source
  of truth — when at least one qualifying class sets `multiClass: true`,
  the discovery routine derives a distinct module ID per class;
  otherwise whole-file mode applies. Mirrors apcore commit
  [`973410b`](https://github.com/aiperceivable/apcore/commit/973410b).
  - **DEPRECATION** — the legacy 4-arg overload
    `discoverMultiClass(filePath, classes, extensionsRoot, multiClassEnabled)`
    is retained with a one-shot `console.warn` and is **functionally
    inert**. Removal scheduled for **v0.22.0**. Migration: drop the
    boolean and mark each `ClassDescriptor` with `multiClass: true`.
  - The free function `discoverMultiClass(...)` re-exported from
    `apcore-js/registry` keeps its existing 4-arg shape for internal
    callers and is unchanged.
- **`RESERVED_WORDS` unchanged.** The `ephemeral` segment is intentionally
  **not** added to `RESERVED_WORDS` because that set is consulted by
  `_validateModuleId` to *reject* IDs whose first segment matches; adding
  `ephemeral` there would block the very registration path the spec
  prescribes. The reservation is enforced through the discovery-path
  rejection and `registerInternal` rejection paths instead. Mirrors
  apcore-python's `RESERVED_WORDS` frozenset.
- Conformance test runner is **pilot-tolerant** for the rollout window —
  when `expected_serialized` / `expected_reserialized` lacks the
  `discoverable` field (the canonical fixture has not yet been updated
  per the RFC's "Conformance plan / Transitional fixture handling"), the
  field is stripped from the actual serialized output before equality
  comparison. Mirrors the apcore-python PR #26 pattern; will be removed
  once the synchronized `conformance/fixtures/annotations_extra_round_trip.json`
  update lands.

### Lifecycle

- **Caller-managed.** `ephemeral.*` modules live until the caller
  explicitly calls `Registry.unregister(moduleId)`. There is no TTL
  sweeper or background GC — TTL-driven cleanup is deferred to a v2
  follow-up if leakage is observed in practice.

## [0.20.0] - 2026-05-05

### Changed

- **Issue #28 — `Registry.discoverMultiClass` signature cleanup (apcore decision-log D-06).** The 4th `multiClassEnabled` argument is dropped from the canonical method surface; the method is now `discoverMultiClass(filePath, classes, extensionsRoot?)`. Per-class opt-in via the new `ClassDescriptor.multiClass?: boolean` field is the sole source of truth — when at least one qualifying class sets `multiClass: true`, the discovery routine derives a distinct module ID per class; otherwise whole-file mode applies. Mirrors the upstream apcore doc-side cleanup in commit [`973410b`](https://github.com/aiperceivable/apcore/commit/973410b) which removed the dead global `extensions.multi_class_discovery` config toggle.
  - **DEPRECATION:** the legacy 4-arg overload `discoverMultiClass(filePath, classes, extensionsRoot, multiClassEnabled)` is retained for backward compatibility and emits a one-shot `console.warn` deprecation notice on first use. The `multiClassEnabled` argument is **functionally inert** — the per-class `multiClass` field is read regardless of what is passed. The 4-arg overload will be removed in **v0.22.0**. Migration: drop the boolean and mark each `ClassDescriptor` you want as a separate module with `multiClass: true`.
  - The free function `discoverMultiClass(...)` re-exported from `apcore-js/registry` keeps its existing 4-arg shape for internal callers and is unchanged.

## [0.20.0] - 2026-05-05

### Added

#### Pipeline Hardening (Issue #33)

- **`StepMiddleware` interface** (Issue #33 §2.2) — Public interface in `src/pipeline.ts` exposing optional `beforeStep` / `afterStep` / `onStepError` hooks around every pipeline step. Hooks may be sync or async; the engine awaits any thenable return value (mirroring the Issue #42 fix in `MiddlewareManager`) so plain functions returning a Promise are not silently dropped. `onStepError` returning a non-null value suppresses the error and continues the pipeline — first non-null wins, later middlewares are skipped. Multiple middlewares run in registration order.
- **`PipelineEngine.addStepMiddleware(mw)`** and **`PipelineEngine.stepMiddlewares`** — Register lifecycle interceptors on the engine. Backward-compatible: pipelines with zero middlewares behave exactly as before.
- **`PipelineDependencyError`** — New error raised at `ExecutionStrategy` construction when a step's `requires` are not satisfied by a preceding step's `provides` (Issue #33 §2.1). Replaces the previous `console.warn` that allowed misconfigured strategies to fail later with a confusing runtime error. Carries `stepName` and `missingRequires` for programmatic inspection.
- **`ExecutionStrategy` constructor `seedProvides` option** — Lets callers building a sub-strategy (e.g. `Executor.stream()`'s post-stream phase) declare context fields that are guaranteed to be pre-populated, so dependency validation does not raise on legitimate use.
- **`ConfigurationError`** — New error raised by `buildStrategyFromConfig()` when YAML pipeline configuration references a non-existent step in `remove`, `configure`, `after`, or `before`, or when a custom step has neither `after` nor `before` (Issue #33 §1.2). Replaces the previous warn-and-continue behaviour. Exported from `apcore-js` for typed catches.
- **Issue #43 §1 — `StorageBackend` interface** (`src/observability/storage.ts`). Pluggable key/value storage with `save` / `get` / `list` / `delete` operations and namespace partitioning. Default `InMemoryStorageBackend` is the implicit fallback. `ErrorHistory`, `UsageCollector`, and `MetricsCollector` accept an optional `storage` constructor option so SDK consumers can wire redis, postgres, etc. without forking the collectors. Re-exported from the package root.
- **Issue #45.1 — `OverridesStore` interface** (`src/sys-modules/overrides.ts`). Pluggable persistent override store mirroring the Python `_load_overrides` / `_write_overrides` and Rust `load_overrides` / `write_override` flows. `FileOverridesStore` writes a YAML file with atomic tempfile + rename semantics; `InMemoryOverridesStore` is provided for tests. `registerSysModules` accepts `overridesStore` and applies persisted overrides on startup before registering modules. `UpdateConfigModule` and `ToggleFeatureModule` persist each successful mutation through the store. Re-exported from the package root.
- **D-15 — `Registry.discoverMultiClass` method.** New instance method on `Registry` matching the Python `Registry.discover_multi_class` and Rust trait surface. Wraps the existing free function (now also re-exported as `_discoverMultiClass` for internal scanner use) so cross-language code can call `registry.discoverMultiClass(filePath, classes, ...)` consistently.
- Granular reload via `path_filter` input in `ReloadModule` (#45.4). Supports glob-pattern bulk reload that scopes safe-unregister + re-discovery to matching module IDs and returns a `reloaded_modules` array.
- Error fingerprinting in `ErrorHistory` — dedup by `(error_code, module_id, normalized_message)` SHA-256 with UUID/ISO-timestamp/integer-ID placeholders, exported as `computeFingerprint` and `normalizeMessage` (#43 §4).
- Configurable redaction via `observability.redaction.field_patterns` / `observability.redaction.value_patterns` / `observability.redaction.replacement` Config keys, plus `RedactionConfig.fromConfig(config)` and exported `DEFAULT_REDACTION_FIELD_PATTERNS` (`_secret_*`, `apiKey`, `api_key`, `token`, `authorization`, `password`, `passwd`, `secret`). Value patterns compile case-insensitively (#43 §5).
- **Sync finding D-08 — `RetryConfig.computeDelayMs`** is the canonical cross-language method name on `RetryConfig` (mirrors apcore-python `compute_delay_ms` / apcore-rust `compute_delay_ms`). The legacy `computeDelay` alias still works but emits a one-shot deprecation warning per process (`[apcore] RetryConfig.computeDelay is deprecated; use computeDelayMs`) and will be removed in the next minor release.
- **Sync finding CRITICAL #4 — canonical `obs.redaction.*` Config keys.** `RedactionConfig.fromConfig` now reads `obs.redaction.sensitive_keys`, `obs.redaction.regex_patterns`, and `obs.redaction.replacement` first (matching apcore-python / apcore-rust) and falls back to the legacy `observability.redaction.field_patterns` / `observability.redaction.value_patterns` / `observability.redaction.replacement` keys for backwards compatibility. Reading any legacy key emits a one-shot deprecation warning pointing migrators at the canonical namespace.

### Changed

- `ExecutionStrategy._validateDependencies` now throws `PipelineDependencyError` instead of emitting `console.warn`. Strategies that declared unsatisfied `requires` will now fail to construct — fix the strategy or use the new `seedProvides` option.
- `buildStrategyFromConfig()` now throws `ConfigurationError` instead of emitting `console.warn` for missing-step / missing-anchor / missing-after-or-before configuration mistakes.

### Fixed

- Async middleware hooks (`before` / `after` / `onError`) — `MiddlewareManager` now awaits the *return value* (already implemented) and `Middleware` base method signatures admit Promise-of-X return types, so higher-order-function-wrapped (Promise-returning) handlers compose without leaking unresolved Promises into `currentInputs` / `currentOutput` / recovery values (#42).
- **Sync findings A-D-101 / A-D-102** — `Registry._registerInOrder` and `Registry._discoverCustom` now apply PROTOCOL_SPEC §2.7 ID validation (empty → pattern → length → reserved-word) and Algorithm A03 conflict detection before registering each discovered module. Invalid or conflicting IDs are skipped with a `console.warn` instead of being registered. Mirrors `apcore-python._filter_id_conflicts` and `apcore-rust::Registry::filter_id_conflicts`.
- **Sync finding A-D-202** — `Executor.stream()` now reads the global deadline from `context.data[CTX_GLOBAL_DEADLINE]` (ms-since-epoch, where `BuiltinContextCreation` writes it) and compares against `Date.now()` directly, instead of reading the unset `Context.globalDeadline` field and dividing `Date.now()` by 1000. Stream-mode global timeout now actually triggers between chunks.
- **Sync finding A-D-404** — `MiddlewareManager.executeOnError` now requires recovery values to be a `RetrySignal` or a non-null object before treating them as recovery. Arrow functions returning `undefined` (the default for handlers without an explicit return) no longer accidentally short-circuit the chain. Mirrors apcore-python's strict type check.

### Changed

- **Issue #36 — canonical event prefixes** — Four registry/health events that previously lacked the canonical `apcore.<subsystem>.<event>` prefix are now emitted under their canonical names: `module_registered` → `apcore.registry.module_registered`, `module_unregistered` → `apcore.registry.module_unregistered`, `error_threshold_exceeded` → `apcore.health.error_threshold_exceeded`, `latency_threshold_exceeded` → `apcore.health.latency_threshold_exceeded`. **DEPRECATION:** during the deprecation window each emission also produces the legacy event with `{ deprecated: true, canonical_event: <canonical> }` in its payload, so existing subscribers continue to receive events. Migrate subscribers to the canonical names; the legacy aliases will be removed in a future release. New helper `emitWithLegacy()` is exported from `apcore-js/events`.
- **Issue #45.2 — contextual audit identity** — `system.control.update_config`, `system.control.toggle_feature` and `system.control.reload_module` now extract `caller_id` (defaulting to `"@external"` when absent) and `identity` (a snapshot of `Context.identity` or `null`) from the execution `Context` and include both fields in the `apcore.config.updated`, `apcore.module.toggled`, and `apcore.module.reloaded` event payloads. New helper `extractAuditIdentity()` is exported from `apcore-js/sys-modules/audit`.
- **Sync finding A-D-104** — `Registry.watch()` is now documented as event-only on the TypeScript SDK. On a file change the module is unregistered (`onUnload` runs) and a `file_changed` event is emitted with `{ filePath }`. Unlike apcore-python (`importlib.reload`) and apcore-rust (full re-discovery), the SDK does not transparently re-import: ES modules cannot be reliably evicted from Node's loader cache without leaks. Consumers needing hot-reload must subscribe to `file_changed` and call `discover()` (or re-import) themselves. See JSDoc on `Registry.watch`.
- **Sync findings A-D-503 / A-D-504** — `EventEmitter.flush(timeoutMs)` default changes from `0` (infinite wait) to `5000` (5 s), matching apcore-python's 5 s semantic default and apcore-rust's ms unit. Pass `0` explicitly to wait indefinitely. Subscriber overflow behaviour switches from drop-and-warn to bounded back-pressure: when `_pending` is at `maxPending`, new dispatches queue and start as slots free, so events are no longer silently dropped under burst load.
- **Sync finding A-D-403** — `MiddlewareManager.executeBefore` / `executeAfter` / `executeOnError` are now `async` and `await` each middleware hook. Removes the silent-Promise-into-currentInputs trap when a `before()` or `after()` hook is async. Public callers in `Executor` and built-in steps already awaited; ad-hoc consumers calling these methods directly now need to `await` the result.

### Documentation

- **Sync finding B-002** — README now documents that `APCore.disable()` / `APCore.enable()` (and the `on`/`off` toggle event) require `sys_modules.enabled: true` in the `Config` passed to `APCore`. Quick Start gains a Config-passing variant that wires sys-modules.

### Added — PROTOCOL_SPEC hardening (Issues #32–#45)

#### Event Management Hardening (Issue #36)

- **`CircuitBreakerWrapper`** — Subscriber-level circuit breaker for `EventEmitter` with configurable failure threshold, timeout (backoff), and automatic OPEN → HALF_OPEN → CLOSED recovery. Exported from `apcore-js/events`.
- **`CircuitState`** enum — `CLOSED`, `OPEN`, `HALF_OPEN` states for `CircuitBreakerWrapper`.
- **`FileSubscriber`** — Event subscriber that appends to a log file with optional rotation (`rotate_bytes`) and format (`json`/`text`). Registered as built-in type `"file"` in the subscriber factory.
- **`StdoutSubscriber`** — Event subscriber that writes to stdout with optional level filtering. Registered as built-in type `"stdout"`.
- **`FilterSubscriber`** — Decorator subscriber filtering events by `include_events`/`exclude_events` lists. Registered as built-in type `"filter"`, accepting any `delegate_type`.
- `registerSubscriberType` / `unregisterSubscriberType` / `resetSubscriberRegistry` / `createSubscriberFromConfig` — now public, documented, and no longer marked deprecated. Custom subscriber types can be registered and used in config-driven instantiation.

#### Middleware Architecture Hardening (Issue #42)

- **`CircuitBreakerMiddleware`** — Per-`(module_id, caller_id)` circuit breaker middleware. Opens on consecutive failures beyond a configurable threshold; enters HALF_OPEN after cooldown; probes with one request and closes on success. Throws `CircuitBreakerOpenError` (code `CIRCUIT_BREAKER_OPEN`) when open.
- **`CircuitBreakerOpenError`** (code `CIRCUIT_BREAKER_OPEN`) — new error class; `DEFAULT_RETRYABLE = false`. Carries `moduleId` and `callerId` details.
- **`MiddlewareCircuitState`** enum — `CLOSED`, `OPEN`, `HALF_OPEN` states for `CircuitBreakerMiddleware`.
- **`validateContextKey()`** — validates that a context key string is non-empty and does not collide with apcore reserved keys.
- **`ContextKeyWriter`** / **`ContextKeyValidation`** interfaces — typed context-key contract for middleware that writes into execution context.
- **`TracingMiddleware`** — OTel-compatible span tracing middleware. Accepts any tracer implementing the `OtelTracer` / `OtelSpan` interfaces. Configurable via `TracingMiddlewareOptions` (sampler, span name builder, attribute extractor). Does not depend on `@opentelemetry/*` packages at runtime.
- **`isAsyncHandler()`** utility — detects whether a middleware method returns a `Promise`.

#### Observability Hardening (Issue #43)

- **`BatchSpanProcessor`** — Buffered async span exporter with configurable `maxQueueSize`, `scheduleDelayMs`, `maxExportBatchSize`. Drops spans when queue is full and tracks `spansDropped`. Exported `BatchSpanProcessorOptions`.
- **`SimpleSpanProcessor`** — Synchronous pass-through processor for testing.
- **`InMemoryObservabilityStore`** — Default pluggable backing store for `ErrorHistory` and `MetricsCollector`. Implements `ObservabilityStore` interface (`record`, `query`, `count`, `clear`).
- **`ObservabilityStore`** interface + **`MetricPoint`** type — public contracts for custom store implementations.
- **`RedactionConfig`** — Field-pattern and value-pattern based input redaction for `ContextLogger`/`ObsLoggingMiddleware`. Configurable `fieldPatterns` (glob), `valuePatterns` (RegExp), and `replacement` string.
- **`PrometheusExporter`** — HTTP server serving Prometheus text format at `/metrics`, liveness at `/healthz`, and readiness at `/readyz`. New optional `usageCollector` constructor option enables usage metrics (see System Modules Hardening below).
- `ErrorHistory` constructor now accepts an options object `{ maxEntriesPerModule?, maxTotalEntries? }` (positional args still accepted for backward compatibility).
- `MetricsCollector` constructor now accepts `MetricsCollectorOptions { buckets?, store? }` in addition to positional args.

#### Registry — Multi-Class Discovery

- **`discoverMultiClass(filePath, options?)`** — discovers multiple module classes from a single file by PascalCase-to-dotted-id naming convention.
- **`classNameToSegment(className)`** — converts a PascalCase class name to a lowercase dotted-id segment.
- **`ModuleIdConflictError`** (code `MODULE_ID_CONFLICT`) — thrown when two classes in the same file produce the same module ID segment.
- **`InvalidSegmentError`** (code `INVALID_SEGMENT`) — thrown when a derived segment does not match `^[a-z][a-z0-9_]*$`.
- **`IdTooLongError`** (code `ID_TOO_LONG`) — thrown when a derived module ID exceeds 192 characters.

#### Async Task Evolution

- **`InMemoryTaskStore`** — default in-memory `TaskStore` implementation, now injectable via `AsyncTaskManager({ executor, store })` for custom backends (Redis, Postgres, etc.). Implements `TaskStore` interface.
- **`RetryConfig`** — configurable retry policy with `maxRetries`, `retryDelayMs`, `backoffMultiplier`, `maxRetryDelayMs`, and `computeDelay(attemptIndex)` for exponential backoff with jitter. Pass to `manager.submit(moduleId, inputs, { retry })`.
- **`AsyncTaskManager.startReaper({ ttlSeconds, sweepIntervalMs })`** — starts a background reaper that deletes expired completed/failed tasks after `ttlSeconds`. Returns a `{ stop() }` handle. Reaper is opt-in; the manager remains zero-dependency when no reaper is configured. Skips `RUNNING` tasks regardless of age.

#### System Modules Hardening (Issue #45)

- **`AuditStore`** interface with `append(entry)` / `query(filter?)` — pluggable audit log for control module actions. Exported from `apcore-js`.
- **`InMemoryAuditStore`** — default in-memory implementation of `AuditStore`. Supports filtering by `moduleId`, `actorId`, and `since` timestamp.
- **`AuditEntry`** type — `{ timestamp, action, targetModuleId, actorId, actorType, traceId, change: { before, after } }`. Actor is extracted from `context.identity`.
- **`buildAuditEntry(action, targetModuleId, context, change)`** — helper that extracts actor information from `Context.identity`.
- **`registerSysModules()`** now accepts an optional 5th `options` parameter (`RegisterSysModulesOptions`):
  - `overridesPath?: string` — YAML file for persisting `update_config` and `toggle_feature` changes. Loaded on startup after base config so overrides survive restarts without modifying the base config file.
  - `auditStore?: AuditStore` — routes all control module audit entries to the store; falls back to `console.warn` when absent.
  - `failOnError?: boolean` (default `false`) — when `true`, first registration failure throws `SysModuleRegistrationError` immediately; when `false`, logs at ERROR level and continues registering remaining modules.
- **`system.control.reload_module`** — new `path_filter: string` input field. When provided, reloads all registered modules whose IDs match the glob pattern, in topological (sorted) order. Mutually exclusive with `module_id`.
- **`system.control.update_config`** / **`system.control.toggle_feature`** — now record a structured `AuditEntry` (actor, timestamp, trace ID, before/after change) when an `AuditStore` is configured; otherwise logs at INFO/WARN level.
- **`PrometheusExporter`** — `usageCollector?: UsageCollector` constructor option. When set, appends usage metrics to `/metrics` output: `apcore_usage_calls_total{module_id, status}` (counter), `apcore_usage_error_rate{module_id}` (gauge), `apcore_usage_p50_latency_ms{module_id}`, `apcore_usage_p95_latency_ms{module_id}`, `apcore_usage_p99_latency_ms{module_id}` (gauges). Prometheus text format is valid (HELP/TYPE lines immediately precede each metric family).
- **`ModuleReloadConflictError`** (code `MODULE_RELOAD_CONFLICT`) — thrown when both `module_id` and `path_filter` are supplied to `system.control.reload_module`.
- **`SysModuleRegistrationError`** (code `SYS_MODULE_REGISTRATION_FAILED`) — thrown by `registerSysModules()` when `failOnError: true` and any system module fails to register.

### Changed — PROTOCOL_SPEC hardening (Issue #45)

- **`registerSysModules()`** 5th parameter changed from (none) to optional `RegisterSysModulesOptions`. Fully backward-compatible — existing calls with 3–4 arguments are unaffected.
- **`UpdateConfigModule`** constructor now accepts optional `UpdateConfigOptions { auditStore?, overridesPath? }` as third argument. Existing two-argument construction is unchanged.
- **`ReloadModule`** constructor now accepts optional `auditStore?: AuditStore` as third argument.
- **`ToggleFeatureModule`** constructor now accepts optional `auditStore?: AuditStore` as fourth argument (after the existing optional `toggleState`).
- **`ReloadModule` input schema** — `module_id` is no longer statically `required`; validation is enforced at runtime to support mutual exclusion with `path_filter`. Callers that previously relied on schema-level rejection of missing `module_id` will now receive the same `InvalidInputError` from runtime validation.
- **`system.control.toggle_feature`** now emits an `[apcore:control]` INFO-level log on every toggle, consistent with `update_config` and `reload_module`.
- **`ErrorCodes`** — added `MODULE_RELOAD_CONFLICT`, `SYS_MODULE_REGISTRATION_FAILED`, `MODULE_ID_CONFLICT`, `INVALID_SEGMENT`, `ID_TOO_LONG`, `CIRCUIT_BREAKER_OPEN`.

---

## [0.19.0] - 2026-04-19

### Added

- **`DependencyNotFoundError`** (error code `DEPENDENCY_NOT_FOUND`) — thrown by `resolveDependencies` when a module's required dependency is not registered. Aligns TypeScript with PROTOCOL_SPEC §5.15.2 which has always mandated this error code. Details include `moduleId` and `dependencyId`. Exported from `apcore`.
- **`DependencyVersionMismatchError`** (error code `DEPENDENCY_VERSION_MISMATCH`) — thrown by `resolveDependencies` when a declared `version` constraint is not satisfied by the registered version of the target module. Details include `moduleId`, `dependencyId`, `required`, `actual`. Exported from `apcore`.
- **`resolveDependencies(modules, knownIds, moduleVersions)`** — new optional third argument accepting `Map<string, string>` or `Record<string, string>` mapping module id → version. When provided, declared dependency version constraints are enforced per PROTOCOL_SPEC §5.3. When absent, the `DependencyInfo.version` field is silently ignored. `ModuleRegistry._resolveLoadOrder` now populates this map from YAML version / class `version` / `"1.0.0"` fallback, and includes already-registered modules so inter-batch constraints resolve against the live registry.
- **Caret (`^`) and tilde (`~`) constraint support** in `matchesVersionHint` / `selectBestVersion` (npm/Cargo semantics): `^1.2.3 → >=1.2.3,<2.0.0`, `^0.2.3 → >=0.2.3,<0.3.0`, `^0.0.3 → >=0.0.3,<0.0.4`, `~1.2.3 → >=1.2.3,<1.3.0`, `~1.2 → >=1.2.0,<1.3.0`, `~1 → >=1.0.0,<2.0.0`. `matchesVersionHint` is now exported.
- **Auto-schema multi-adapter chain** (`src/schema/extractor.ts`) — `SchemaExtractorRegistry` with pluggable adapters. Built-in: TypeBox (priority 100, detects `Symbol.for('TypeBox.Kind')`), JsonSchema (priority 30, detects `type`/`properties`). Custom adapters (zod, class-validator, typia) registered via `SchemaExtractorRegistry.register()`. See DECLARATIVE_CONFIG_SPEC.md §6.3.
- **`auto_schema: true | permissive | strict`** in binding YAML — triggers module export scanning (`inputSchema`/`outputSchema` named exports, or `<symbolName>InputSchema`/`<symbolName>OutputSchema` companion naming). Implicit default when no schema mode specified.
- **`BindingSchemaInferenceFailedError`** and **`BindingSchemaModeConflictError`** — canonical errors per DECLARATIVE_CONFIG_SPEC.md §7.1. `BindingSchemaMissingError` is now a deprecated alias.
- **`spec_version`** field support in binding YAML with deprecation warning when absent.
- **`documentation`, `annotations`, `metadata`** fields pass through `BindingLoader` → `FunctionModule`. Annotations converted from YAML snake_case to TypeScript camelCase via `parseAnnotations()`.
- **Pipeline `handler:` dynamic import** — `_resolveStep` and `buildStrategyFromConfig` are now `async`. Handler modules loaded via `await import()` with security checks (rejects `..` segments, `file:` URLs). See DECLARATIVE_CONFIG_SPEC.md §4.4.
- **Cross-SDK conformance fixtures** in `apcore/conformance/fixtures/`.

### Fixed

- **`resolveDependencies` cycle path accuracy** — `extractCycle` previously returned a phantom path (all remaining nodes plus the first one re-appended) when the arbitrarily-picked start node had no outgoing edge inside `remaining`. This could happen when a module is blocked on an external `knownIds` dependency while another subset contains a real cycle. Rewritten to DFS from each remaining node (sorted) and return a true back-edge cycle `[n0, ..., nk, n0]`; falls back to `sortedRemaining` only when no back-edge exists.

### Changed
- **Missing required dependencies now throw `DependencyNotFoundError` (code `DEPENDENCY_NOT_FOUND`) instead of `ModuleLoadError` (code `MODULE_LOAD_ERROR`).** Brings TypeScript into compliance with PROTOCOL_SPEC §5.15.2. Upgrade path: catch `DependencyNotFoundError` specifically, or catch the `ModuleError` base class. Code-based dispatch (`err.code === 'DEPENDENCY_NOT_FOUND'`) also works and is recommended for cross-language consumers.
- **`Context.create({ traceParent })`** — strict input validation per PROTOCOL_SPEC §10.5. trace_ids that are all-zero or all-f (W3C-invalid) now trigger regeneration, and any regeneration now emits `console.warn` (previously silent). No auto-normalization (dashed-UUID stripping or case folding) is performed at `Context.create`; such normalization is the caller's ContextFactory responsibility. Valid 32-hex inputs remain accepted verbatim. Covered by new conformance fixture `context_trace_parent.json`.

### Changed (BREAKING)

- **`buildStrategyFromConfig()` is now `async`** — returns `Promise<ExecutionStrategy>`. Callers must `await` it. Necessary because `handler:` resolution uses `await import()`.
- **`_resolveStep()` is now `async`** — returns `Promise<Step>`.
- **`BindingSchemaMissingError`** renamed to `BindingSchemaInferenceFailedError`. Constructor signature changed: `(target, moduleId?, filePath?, remediation?, options?)`. Old name kept as alias.

## [0.18.0] - 2026-04-15

### Added

- **Registry length boundary tests** — `tests/registry/test-registry.test.ts` now covers `MAX_MODULE_ID_LENGTH` constant equality, exact-length registration acceptance, and over-length rejection (parity with `apcore-python`'s `TestRegisterConstants`).
- **8 new parity tests** in `tests/registry/test-registry.test.ts` covering: invalid pattern rejection (uppercase, hyphens, leading digit, etc.), reserved word in any segment rejection, `registerInternal` accepting reserved first segment, accepting reserved word in any segment, still rejecting empty, still rejecting invalid pattern, still rejecting over-length, and rejecting duplicate.

### Changed

- **ACL singular condition handler aliases removed** (`identity_type`, `role`, `call_depth`). Spec §6.1 only defines the plural forms (`identity_types`, `roles`, `max_call_depth`); the singular aliases were a cross-language divergence. Aligned with apcore-python (commit `2c204fb`) and apcore-rust (plural-only since initial implementation).
- **`module()` factory now throws `InvalidInputError` when `id` is not provided**, per PROTOCOL_SPEC §5.11.6. JavaScript cannot derive `{module_path}.{name}` at runtime (unlike Python's `__module__`), so explicit `id` is required. Previously defaulted to `'anonymous'`. Aligned with apcore-rust which also requires explicit name.
- **`MAX_MODULE_ID_LENGTH` raised from 128 to 192** (`registry/registry.ts`). Tracks PROTOCOL_SPEC §2.7 EBNF constraint #1 — accommodates Java/.NET deep-namespace FQN-derived IDs while remaining filesystem-safe (`192 + ".binding.yaml".length = 205 < 255`-byte filename limit on ext4/xfs/NTFS/APFS/btrfs). Module IDs valid before this change remain valid; only the upper bound moved. **Forward-compatible relaxation:** older 0.17.x/0.18.x readers will reject IDs in the 129–192 range emitted by this version.
- **`Registry.register()` and `Registry.registerInternal()` now share a private `validateModuleId()` helper** that runs validation in canonical order (empty → EBNF pattern → length → reserved word per-segment). Deduplicated 2 enforcement sites in the same file. Aligned cross-language with apcore-python and apcore-rust.
- **Duplicate registration error message canonicalized** to `` `Module ID '${moduleId}' is already registered` `` (was `` `Module already exists: ${moduleId}` ``). Both `register()` and `registerInternal()` now emit the same message. Aligned with apcore-python and apcore-rust byte-for-byte.
- **Helper error message style aligned with apcore-python / apcore-rust:**
  - Empty error: `'module_id must be a non-empty string'` (was `'Module ID must be a non-empty string'` — now lowercase to match Python/Rust).
  - Pattern error: single quotes around the offending ID (was double quotes).
  - Pattern error format string: uses `${MODULE_ID_PATTERN.source}` (bare regex source) instead of `${MODULE_ID_PATTERN}` (which produced `/.../` slashes via `RegExp.toString()`).

### Changed (cross-language sync)

- **`Executor.listStrategies()` now returns `StrategyInfo[]` instead of `string[]`** — Provides step count, step names, and description alongside the strategy name. Aligned with apcore-python `list_strategies() -> list[StrategyInfo]` and apcore-rust `list_strategies() -> Vec<StrategyInfo>`.

### Removed

- **`FeatureNotImplementedError` and `DependencyNotFoundError`** — zero throw-sites across the codebase. Error codes `GENERAL_NOT_IMPLEMENTED` and `DEPENDENCY_NOT_FOUND` remain in `ErrorCodes` for use via the generic `ModuleError` constructor. Aligned with apcore-python (commit `91e951a`).

### Fixed

- **README Quick Start — missing `await` on `client.validate()` call.** `validate()` is async and returns `Promise<PreflightResult>`; the example assigned the Promise directly instead of awaiting it.

- **Dead fallback in `getDefinition` dropped** (`registry.ts:516-530`). A `module.description ?? metadata.description` chain was unreachable because `module.description` is always set by the `Module` base class constructor. Removed the dead branch.
- **Spec §4.13 annotation merge — YAML annotations are no longer silently dropped at registration.** Two coupled bugs were repaired in `registry/metadata.ts:mergeModuleMetadata` and `registry/registry.ts:getDefinition`. The merge step was doing whole-replacement of the `annotations` field instead of the field-level merge mandated by §4.13 ("If YAML only defines `readonly: true`, other fields **must** retain values from code or defaults."), and `getDefinition` was reading directly from the module class object even when the merge result was available. The fix wires `mergeAnnotations` and `mergeExamples` from `schema/annotations.ts` (defined and unit-tested but never previously called from production) into the registry pipeline, and updates `getDefinition` to consume the merged metadata. **User-observable behavior change:** modules that supplied `annotations:` in their `*_meta.yaml` companion files were previously seeing those annotations silently ignored; they will now be honored. Modules that relied on the broken behavior should audit their meta files. Identical fix to `apcore-python` commit `9c0fde9`. Adds 5 regression tests covering field-level merge, YAML-only, neither-defined, examples-yaml-wins, and unknown-key-drop scenarios.
- **`annotationsFromJSON` precedence inversion** — Per PROTOCOL_SPEC §4.4.1 rule 7, when the same key appears both in a nested `extra` object and as a top-level overflow key, the **nested value now wins** (previously the spread order `{...explicitExtra, ...overflow}` made overflow win). Behavior change is observable only in the pathological case where an input contains both forms of the same key — no conformant producer emits this. Top-level overflow keys are still tolerated and merged into `extra` for backward compatibility.

## [0.17.1] - 2026-04-06

### Added

- **`buildMinimalStrategy()`** — 4-step pipeline (context → lookup → execute → return) for pre-validated internal hot paths. Registered as `"minimal"` in Executor built-in factories.
- **`requires` / `provides` on `Step` interface** — Optional advisory fields declaring step dependencies. `ExecutionStrategy` validates dependency chains at construction and insertion, emitting `console.warn` for unmet `requires`.

### Fixed

- **`buildTestingStrategy` aligned with Python/Rust** — Now removes `acl_check`, `approval_gate`, and `call_chain_guard` (8 steps) instead of stripping to 4 minimal steps. Cross-language strategy parity restored.
- **`buildPerformanceStrategy` aligned with Python/Rust** — Now removes `middleware_before` and `middleware_after` instead of `approval_gate` and `output_validation`. Cross-language strategy parity restored.

---

## [0.17.0] - 2026-04-05

### Added

- **Step Metadata**: Four optional fields on `Step` interface: `matchModules` (glob patterns), `ignoreErrors` (fault-tolerant), `pure` (safe for validate dry-run), `timeoutMs` (per-step timeout via `Promise.race`).
- **YAML Pipeline Configuration**: `registerStepType()`, `unregisterStepType()`, `registeredStepTypes()`, `buildStrategyFromConfig()` in new `pipeline-config.ts` module.
- **PipelineContext fields**: `dryRun`, `versionHint`, `executedMiddlewares`.
- **StepTrace**: `skipReason` field.

### Changed

- **Step order**: `BuiltinMiddlewareBefore` now runs BEFORE `BuiltinInputValidation`. Middleware transforms are validated.
- **Executor delegation**: `callAsync()`, `validate()`, and `stream()` fully delegate to `PipelineEngine.run()`. Removed inline step code.
- **Renamed**: `safety_check` → `call_chain_guard`, `BuiltinSafetyCheck` → `BuiltinCallChainGuard`.
- **Removed `builtin.` prefix**: All step names changed from `builtin.context_creation` to `context_creation`.
- **`validate()` is now async**: Returns `Promise<PreflightResult>`.

### Fixed

- Middleware input transforms were never validated against schema.
- `validate()` now uses pipeline dry-run mode — user-added `pure=true` steps automatically participate.

---

## [0.16.0] - 2026-04-05

### Added

- **Config Bus**: `envStyle` (auto/nested/flat), `maxDepth`, `envPrefix` auto-derivation, `envMap` (namespace + global), `Config.envMap()`, `ConfigEnvMapConflictError`.
- **Context**: `ContextKey<T>` typed accessor with `get()`/`set()`/`delete()`/`exists()`/`scoped()`. Built-in key constants. `globalDeadline: number | null` field. `Context.serialize()`/`deserialize()` with `_context_version: 1`.
- **Annotations**: `extra: Readonly<Record<string, unknown>>` extension field. `paginationStyle` changed from union to `string`. All optional fields now required with defaults. `createAnnotations()` factory. `annotationsToJSON()`/`annotationsFromJSON()` wire format.
- **ACL**: `ACLConditionHandler` interface (`boolean | Promise<boolean>`). `ACL.registerCondition()`. `$or`/`$not` compound operators. `asyncCheck()` method. Fail-closed for unknown conditions. `removeRule` fixed to element-wise comparison.
- **Pipeline**: `Step` interface, `StepResult`, `PipelineContext`, `PipelineTrace`, `ExecutionStrategy`, `PipelineEngine`. 11 `BuiltinStep` classes. Preset strategies (standard/internal/testing/performance). `Executor.strategy` option. `callWithTrace()`. `registerStrategy()`/`listStrategies()`/`describePipeline()`.

### Changed

- Toggle system module now has PROTOCOL_SPEC reference comment.

---

## [0.15.1] - 2026-03-31

### Changed

- **Env prefix convention simplified** — Removed the `^APCORE_[A-Z0-9]` reservation rule from `Config.registerNamespace()`. Sub-packages now use single-underscore prefixes (`APCORE_MCP`, `APCORE_OBSERVABILITY`, `APCORE_SYS`) instead of the double-underscore form. Only the exact `APCORE` prefix is reserved for the core namespace.
- Built-in namespace env prefixes: `APCORE__OBSERVABILITY` → `APCORE_OBSERVABILITY`, `APCORE__SYS` → `APCORE_SYS`.

---

## [0.15.0] - 2026-03-30

### Added

#### Config Bus Architecture (§9.4–§9.14)

`Config` is upgraded from an internal configuration tool to an ecosystem-level Config Bus. Any package — apcore ecosystem or third-party — can register a named namespace with optional JSON Schema validation, environment variable prefix, and default values.

- **`Config.registerNamespace(name, options?)`** — Register a namespace on the global (class-level) registry shared across all `Config` instances. Options:
  - `schema?` — JSON Schema object for namespace-level validation
  - `envPrefix?` — Environment variable prefix for this namespace (e.g. `'APCORE_MCP'`)
  - `defaults?` — Default values merged before file and env overrides
  - Late registration is permitted; call `config.reload()` afterward to apply defaults and env overrides
  - Throws `CONFIG_NAMESPACE_DUPLICATE` if the name is already registered
  - Throws `CONFIG_NAMESPACE_RESERVED` for reserved names (e.g. `_config`)
- **`config.get("namespace.key.path")`** — Dot-path access with namespace resolution. The first segment resolves to a registered namespace; remaining segments traverse its subtree
- **`config.namespace(name)`** — Returns the full subtree for a registered namespace as a plain object
- **`config.bind<T>(namespace, type)`** — Returns a typed view of a namespace subtree; throws `CONFIG_BIND_ERROR` on schema mismatch
- **`config.getTyped<T>(path, type)`** — Typed single-value accessor with runtime type guard
- **`config.mount(namespace, options)`** — Attach an external configuration source to a namespace without requiring a unified YAML file. `options` accepts `fromFile` (path string) or `fromDict` (plain object). Throws `CONFIG_MOUNT_ERROR` on failure
- **`Config.registeredNamespaces()`** — Returns a string array of all currently registered namespace names
- **`config.reload()`** — Extended: re-reads YAML (when loaded via `Config.load()`), re-detects legacy/namespace mode, re-applies namespace defaults and env overrides, re-validates, and re-reads mounted files

##### Unified YAML with namespace sections

Config files now support a namespace mode when an `apcore:` top-level key is present. Each registered namespace occupies its own top-level section. The `_config` reserved meta-namespace controls validation behavior (`strict`, `allowUnknown`). Legacy files (no `apcore:` key) remain fully backward compatible.

##### Per-namespace environment variable overrides

Each namespace declares its own `envPrefix`. The loader uses a longest-prefix-match dispatch algorithm to route env vars to the correct namespace. Apcore sub-packages use `APCORE_` prefixed names (e.g. `APCORE_MCP`, `APCORE_OBSERVABILITY`); the longest-prefix-match dispatch disambiguates from the core `APCORE` flat-key prefix.

##### New error codes

| Code | When thrown |
|------|-------------|
| `CONFIG_NAMESPACE_DUPLICATE` | `registerNamespace()` called with an already-registered name |
| `CONFIG_NAMESPACE_RESERVED` | `registerNamespace()` called with a reserved name (e.g. `_config`) |
| `CONFIG_ENV_PREFIX_CONFLICT` | Two namespaces declare the same `envPrefix` |
| `CONFIG_MOUNT_ERROR` | `mount()` cannot read or parse the external source |
| `CONFIG_BIND_ERROR` | `bind<T>()` or `getTyped<T>()` type guard fails |

#### Built-in Namespace Registrations (§9.15)

apcore pre-registers two namespaces for its own subsystems:

- **`observability`** (`APCORE_OBSERVABILITY`) — Wraps the existing `apcore.observability.*` flat keys (tracing, metrics, logging, errorHistory, platformNotify) into a dedicated namespace. Adapter packages (apcore-mcp, apcore-a2a, apcore-cli) should read from this namespace instead of maintaining independent logging defaults.
- **`sysModules`** (`APCORE_SYS`) — Promotes `apcore.sys_modules.*` flat keys into a dedicated namespace. `registerSysModules()` prefers `config.namespace("sysModules")` in namespace mode and falls back to `config.get("sys_modules.*")` in legacy mode.

#### Error Formatter Registry (§8.8)

New `ErrorFormatter` interface and `ErrorFormatterRegistry` singleton for adapter-specific error serialization:

- **`ErrorFormatterRegistry.register(surface, formatter)`** — Register a named formatter (e.g. `'mcp'`, `'a2a'`). Throws `ERROR_FORMATTER_DUPLICATE` if already registered.
- **`ErrorFormatterRegistry.get(surface)`** — Retrieve a registered formatter by surface name.
- **`ErrorFormatterRegistry.format(surface, error)`** — Format a `ModuleError` using the registered formatter; falls back to `error.toDict()` when no formatter is registered for the surface.

New error code: `ERROR_FORMATTER_DUPLICATE`.

#### Event Type Naming Convention and Collision Fix (§9.16)

Two confirmed event-type collisions in the emitted event stream are resolved. Canonical dot-namespaced names replace the ambiguous short-form names:

| Legacy name (alias, still emitted) | Canonical name | Meaning |
|------------------------------------|----------------|---------|
| `"module_health_changed"` | `apcore.module.toggled` | Module enabled/disabled toggle |
| `"module_health_changed"` | `apcore.health.recovered` | Error-rate recovery after spike |
| `"config_changed"` | `apcore.config.updated` | Config key updated at runtime |
| `"config_changed"` | `apcore.module.reloaded` | Module reloaded from disk |

Naming convention: `apcore.*` is reserved for core events. Adapter packages use their own prefix (`apcore-mcp.*`, `apcore-a2a.*`, `apcore-cli.*`). All four legacy short-form names remain emitted as aliases during the transition period.

---

## [0.14.1] - 2026-03-29

### Fixed
- **Executor schema validation** — `Executor.call()` now accepts raw JSON Schema (e.g. from `zodToJsonSchema`) as `inputSchema`/`outputSchema`, not just TypeBox `TSchema`. Previously, passing raw JSON Schema caused TypeBox `Value.Check()` to throw "Unknown type". The fix auto-converts via `jsonSchemaToTypeBox()` on first use and caches the result on the module object to avoid repeated conversion.

## [0.14.0] - 2026-03-24

### Breaking Changes
- Middleware default priority changed from `0` to `100` per PROTOCOL_SPEC §11.2. Middleware without explicit priority will now execute before priority-0 middleware.

### Added
- **Middleware priority** — `Middleware` base class now accepts `priority: number` (default 0). Higher priority executes first; equal priority preserves registration order. `BeforeMiddleware` and `AfterMiddleware` adapters also accept `priority`.
- **Priority range validation** — `RangeError` thrown for values outside 0-1000

## [0.13.1] - 2026-03-22

### Changed
- Rebrand: aipartnerup → aiperceivable

## [0.13.0] - 2026-03-12

### Added
- **Caching/pagination annotations** — `ModuleAnnotations` gains 5 optional fields: `cacheable`, `cacheTtl`, `cacheKeyFields`, `paginated`, `paginationStyle` (backward compatible)
- **`paginationStyle` union** — Typed as `'cursor' | 'offset' | 'page'` matching Python SDK and spec
- **`sunsetDate`** — New field on `ModuleDescriptor` and `LLMExtensions` for module deprecation lifecycle
- **`onSuspend()` / `onResume()` lifecycle hooks** — Optional methods on `Module` interface for state preservation during hot-reload; integrated into control module reload flow
- **MCP `_meta` export** — Schema exporter includes `cacheable`, `cacheTtl`, `cacheKeyFields`, `paginated`, `paginationStyle` in `_meta` sub-dict
- **Suspend/resume tests** — 5 test cases in `test-control.test.ts` covering happy path, null return, no hooks, error paths
- **README Links section** — Footer with Documentation, Specification, GitHub, npm, Issues links

### Changed
- **Rebranded** — "module development framework" → "module standard" in package.json, index.ts, README, and internal JSDoc
- **README** — Three-tier slogan/subtitle/definition format, annotation features in feature list
- **`dictToAnnotations`** — Snake_case fallbacks for new fields (`cache_ttl`, `cache_key_fields`, `pagination_style`)
- **All sys-module annotations** — Updated with new fields (9 modules across 5 files)

---

## [0.12.0] - 2026-03-11

### Added
- **`Module.preflight()`** — Optional method for domain-specific pre-execution warnings (spec §5.6)
- **`Module.describe()`** — Optional method returning `ModuleDescription` for LLM/AI tool discovery (spec §5.6)
- **`ModuleDescription`** interface — Typed return type for `Module.describe()`, exported from package index

### Changed
- **`ExecutionCancelledError`** now extends `ModuleError` (was bare `Error`) with error code `EXECUTION_CANCELLED`, aligning with PROTOCOL_SPEC §8.7 error hierarchy
- **`ErrorCodes`** — Added `EXECUTION_CANCELLED` constant

### Fixed
- **Removed phantom CHANGELOG entry** — `ModuleAnnotations.batchProcessing` (v0.4.0) was never implemented

---

## [0.11.0] - 2026-03-08

### Added
- **Full lifecycle integration tests** (`tests/integration/test-full-lifecycle.test.ts`) — 8 tests covering the complete 11-step pipeline with all gates (ACL + Approval + Middleware + Schema validation) enabled simultaneously, nested module calls, shared `context.data`, error propagation, schema validation, and safe hot-reload lifecycle.

#### System Modules — AI Bidirectional Introspection
Built-in `system.*` modules that allow AI agents to query, monitor

- **`system.health.summary`** / **`system.health.module`** — Health status classification with error history integration.
- **`system.manifest.module`** / **`system.manifest.full`** — Module introspection and full registry manifest with filtering.
- **`system.usage.summary`** / **`system.usage.module`** — Usage statistics with hourly trend data.
- **`system.control.update_config`** — Runtime config hot-patching.
- **`system.control.reload_module`** — Hot-reload modules from disk.
- **`system.control.toggle_feature`** — Enable/disable modules at runtime.
- **`registerSysModules()`** — Auto-registration wiring for all system modules.

#### Observability
- **`ErrorHistory`** — Ring buffer tracking recent errors with deduplication.
- **`ErrorHistoryMiddleware`** — Middleware recording `ModuleError` details.
- **`UsageCollector`** / **`UsageMiddleware`** — Per-module call counting, latency histograms, and hourly trends.
- **`PlatformNotifyMiddleware`** — Threshold-based sensor emitting events on error rate spikes.

#### Event System
- **`EventEmitter`** — Global event bus with async subscriber dispatch.
- **`WebhookSubscriber`** — HTTP POST event delivery with retry.
- **`A2ASubscriber`** — Agent-to-Agent protocol event bridge.

#### APCore Unified Client
- **`APCore.on()`** / **`APCore.off()`** — Event subscription management via the unified client.
- **`APCore.disable()`** / **`APCore.enable()`** — Module toggle control via the unified client.

#### Registry
- **Module toggle** — `ToggleState` with `disable()`/`enable()`, `ModuleDisabledError` enforcement.

#### Examples
- **`examples/`** directory — 7 runnable examples mirroring apcore-python: simple client, minimal module, readonly module, full-featured module with ContextLogger, `module()` function, and YAML binding with target function.

### Fixed
- **Stale `VERSION` constant** in built dist (`0.9.0` vs `0.11.0`). Rebuilt dist to match `package.json`.
- README architecture tree updated to include ~20 missing source files (`client.ts`, `events/`, `sys-modules/`, etc.).
- README error class count corrected to 35.

---

## [0.10.0] - 2026-03-07

### Added

#### APCore Unified Client
- **`APCore.stream()`** — Stream module output chunk by chunk via the unified client.
- **`APCore.validate()`** — Non-destructive preflight check via the unified client.
- **`APCore.describe()`** — Get module description info (for AI/LLM use).
- **`APCore.useBefore()`** — Add before function middleware via the unified client.
- **`APCore.useAfter()`** — Add after function middleware via the unified client.
- **`APCore.remove()`** — Remove middleware by identity via the unified client.

#### Module Interface
- **Optional methods** added to `Module` interface: `stream?()`, `validate?()`, `onLoad?()`, `onUnload?()`.

#### Error Hierarchy
- **`FeatureNotImplementedError`** — New error class for `GENERAL_NOT_IMPLEMENTED` code.
- **`DependencyNotFoundError`** — New error class for `DEPENDENCY_NOT_FOUND` code.

### Changed
- APCore client now provides full feature parity with `Executor`.

---

## [0.9.0] - 2026-03-06

### Added

#### Enhanced Executor.validate() Preflight
- **`PreflightCheckResult`** — New readonly interface representing a single preflight check result with `check`, `passed`, and `error` fields.
- **`PreflightResult`** — New readonly interface returned by `Executor.validate()`, containing per-check results, `requiresApproval` flag, and computed `errors` array. Duck-type compatible with `ValidationResult`.
- **`createPreflightResult()`** — Factory function for constructing `PreflightResult` from a checks array.
- **Full 6-check preflight** — `validate()` now runs Steps 1–6 of the pipeline (module_id format, module lookup, call chain safety, ACL, approval detection, schema validation) without executing module code or middleware.

### Changed

#### Executor Pipeline
- **Step renumbering** — Approval Gate renumbered from Step 4.5 to Step 5; all subsequent steps shifted +1 (now 11 clean steps).
- **`validate()` return type** — Changed from `ValidationResult` to `PreflightResult`. Backward compatible: `.valid` and `.errors` still work identically for existing consumers.
- **`validate()` signature** — Added optional `context` parameter for call-chain checks; `inputs` now optional (defaults to `{}`).

#### Public API
- Exported `PreflightCheckResult`, `PreflightResult`, and `createPreflightResult` from top-level `index.ts`.

## [0.8.0] - 2026-03-05

### Added

#### Executor Enhancements
- **Dual-timeout model** — Global deadline enforcement (`executor.global_timeout`) alongside per-module timeout. The shorter of the two is applied, preventing nested call chains from exceeding the global budget.
- **Error propagation (Algorithm A11)** — All execution paths wrap exceptions via `propagateError()`, ensuring middleware always receives `ModuleError` instances with trace context.

#### Error System
- **ErrorCodeRegistry** — Custom module error codes are validated against framework prefixes and other modules to prevent collisions. Raises `ErrorCodeCollisionError` on conflict.
- **VersionIncompatibleError** — New error class for SDK/config version mismatches with `negotiateVersion()` utility.
- **MiddlewareChainError** — Now explicitly `DEFAULT_RETRYABLE = false` per PROTOCOL_SPEC §8.6.
- **ErrorCodes** — Added `VERSION_INCOMPATIBLE` and `ERROR_CODE_COLLISION` constants (34 total).

#### Utilities
- **`guardCallChain()`** — Standalone Algorithm A20 implementation for call chain safety checks (depth, circular, frequency). Executor delegates to this utility instead of inline logic.
- **`propagateError()`** — Standalone Algorithm A11 implementation for error wrapping and trace context attachment.
- **`normalizeToCanonicalId()`** — Cross-language module ID normalization (Python snake_case, Go PascalCase, etc.).
- **`calculateSpecificity()`** — ACL pattern specificity scoring for deterministic rule ordering.

#### ACL Enhancements
- **Audit logging** — `ACL` constructor accepts optional `auditLogger` callback. All access decisions emit `AuditEntry` with timestamp, caller/target IDs, matched rule, identity, and trace context.
- **Condition-based rules** — ACL rules support `conditions` for identity type, role, and call depth filtering.

#### Config System
- **Full validation** — `Config.validate()` checks schema structure, value types, and range constraints.
- **Hot reload** — `Config.reload()` re-reads the YAML source and re-validates.
- **Environment overrides** — `APCORE_*` environment variables override config values (e.g., `APCORE_EXECUTOR_DEFAULT_TIMEOUT=5000`).
- **`Config.fromDefaults()`** — Factory method for default configuration.

#### Middleware
- **RetryMiddleware** — Configurable retry with exponential/fixed backoff, jitter, and max delay. Only retries errors marked `retryable: true`.

#### Context
- **Generic `services` typing** — `Context<T>` supports typed dependency injection via the `services` field.

### Changed

#### Executor Internals
- `_checkSafety()` now delegates to standalone `guardCallChain()` instead of inline duplicated logic.
- Global deadline set on root call only, propagated to child contexts via shared `data['_global_deadline']`.

#### Public API
- Expanded `index.ts` exports with new symbols: `RetryMiddleware`, `RetryConfig`, `ErrorCodeRegistry`, `ErrorCodeCollisionError`, `VersionIncompatibleError`, `negotiateVersion`, `guardCallChain`, `propagateError`, `normalizeToCanonicalId`, `calculateSpecificity`, `AuditEntry`.

## [0.7.2] - 2026-03-04

### Fixed
- **CHANGELOG cleanup** — Removed duplicate entries that were incorrectly repeated in the 0.4.0 and 0.3.0 sections.

### Changed
- **README.md** — Added documentation link section pointing to the official Getting Started guide. Updated project structure to reflect files added in recent releases (`async-task.ts`, `cancel.ts`, `extensions.ts`, `trace-context.ts`), and corrected error class count from 20+ to 30+.

## [0.7.1] - 2026-03-03

### Changed
- **`license` field aligned** — Updated `package.json` `license` field from `"MIT"` to `"Apache-2.0"` to match the license file change made in 0.7.0.

## [0.7.0] - 2026-03-02

### Added
- **Approval system** — Pluggable approval gate (Step 4.5) in the executor pipeline between ACL enforcement and input validation. Modules with `requiresApproval: true` annotation trigger an approval flow before execution proceeds.
  - `ApprovalHandler` interface with `requestApproval()` and `checkApproval()` methods for synchronous and async (polling) approval flows
  - `ApprovalRequest` and `ApprovalResult` types carrying invocation context and decision state (`approved`, `rejected`, `timeout`, `pending`)
  - Three built-in handlers: `AutoApproveHandler` (dev/testing), `AlwaysDenyHandler` (safe default), `CallbackApprovalHandler` (user-provided async callback)
  - `createApprovalRequest()` and `createApprovalResult()` factory functions
  - `Executor.setApprovalHandler()` method for runtime handler configuration
  - Approval audit events emitted to tracing spans for observability
- **Approval error types** — `ApprovalError` (base), `ApprovalDeniedError`, `ApprovalTimeoutError` (retryable), `ApprovalPendingError` (carries `approvalId` for polling). Error codes `APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `APPROVAL_PENDING` added to `ErrorCodes`.
- **`approval_handler` extension point** — Single-handler extension point in `ExtensionManager` for wiring approval handlers via the extension system.
- **Approval test suites** — `test-approval.test.ts`, `test-approval-executor.test.ts`, `test-approval-integration.test.ts`, and `test-errors.test.ts` covering handler behavior, executor pipeline integration, async polling, and error class correctness.

### Changed
- **License changed from MIT to Apache-2.0**.
- Added `"approval"` to `package.json` keywords.

## [0.6.0] - 2026-02-23

### Fixed
- **Critical publishing bug** — Previous releases (0.1.0–0.5.0) shipped without `dist/` directory because `.gitignore` excluded `dist/` and npm fell back to it as the exclusion list (no `files` field or `.npmignore` existed). `require("apcore-js")` and `import("apcore-js")` would fail at runtime with "module not found". This is the first version where the package is actually usable from npm.
- **VERSION constant out of sync** — `VERSION` export was stuck at `'0.3.0'` while `package.json` was at `0.5.0`.

### Added
- `"files": ["dist", "README.md"]` in `package.json` to restrict npm publish scope to compiled output only (previously published src/, tests/, planning/, .claude/, .github/ — 902 KB of dev files).
- `"prepublishOnly": "pnpm run build"` script to ensure `tsc` runs before every `npm publish` / `pnpm publish`.
- **Package integrity test suite** (`tests/test-package-integrity.test.ts`) — 10 tests that verify:
  - `files` field configuration and exclusion of dev directories
  - `prepublishOnly` script exists and invokes build
  - All entry points (`main`, `types`, `exports`) resolve to files in `dist/`
  - `dist/index.js` is importable and exports all 16+ core symbols
  - `VERSION` constant matches `package.json` version

### Changed
- **Version aligned with apcore-python** — Bumped to 0.6.0 for cross-language version consistency.
- Package size reduced from 192.6 kB (source-only, broken) to 86.3 kB (compiled, working).
- **Full browser / frontend compatibility** — All `node:fs` and `node:path` imports across 7 source files (`acl.ts`, `bindings.ts`, `schema/loader.ts`, `schema/ref-resolver.ts`, `registry/metadata.ts`, `registry/scanner.ts`, `registry/registry.ts`) converted from static top-level imports to lazy-load via ESM top-level `await import()` with `try/catch`. Importing any module from `apcore-js` in a browser bundler no longer crashes at parse time.
- **`node:crypto` removed** — `trace-context.ts` and `observability/tracing.ts` now use a new `randomHex()` utility based on the Web Crypto API (`globalThis.crypto.getRandomValues()`), compatible with Node 18+ and all modern browsers.
- **`process.stdout` / `process.stderr` removed** — `StdoutExporter` uses `console.info()`, `ContextLogger` default output uses `console.error()` for universal runtime compatibility.
- `Registry.watch()` signature changed from `watch(): void` to `async watch(): Promise<void>` (backward-compatible — existing fire-and-forget calls still work).
- Added `"sideEffects": false` to `package.json` to enable bundler tree-shaking of Node.js-only code paths.

### Added (new in browser-compat)
- `randomHex(byteLength: number): string` utility function in `utils/index.ts` — generates hex strings using Web Crypto API, replacing `node:crypto.randomBytes`.
- **Browser compatibility test suite** (`tests/test-browser-compat.test.ts`) — 26 tests across 4 groups:
  - Module import health (8 tests) — all lazy-load modules importable
  - Pure-logic APIs without filesystem (10 tests) — ACL, metadata, jsonSchemaToTypeBox, RefResolver inline $ref, Registry register/get/event
  - Filesystem-dependent APIs in Node.js (5 tests) — ACL.load, loadMetadata, scanExtensions, SchemaLoader, RefResolver with lazy-loaded fs/path
  - Source file guard (1 test) — scans all 10 refactored files to assert zero static `node:` imports

## [0.5.0] - 2026-02-23

### Added
- **Cancellation support** with `CancelToken` and `ExecutionCancelledError`, including executor pre-execution cancellation checks.
- **Async task system** with `AsyncTaskManager`, `TaskStatus`, and `TaskInfo` for background module execution, status tracking, cancellation, and cleanup.
- **Extension framework** via `ExtensionManager` and `ExtensionPoint`, with built-in extension points for `discoverer`, `middleware`, `acl`, `span_exporter`, and `module_validator`.
- **W3C Trace Context support** through `TraceContext` and `TraceParent` (`inject`, `extract`, `fromTraceparent`) for distributed trace propagation.
- **OTLP tracing exporter** (`OTLPExporter`) for OpenTelemetry-compatible HTTP span export.
- **Registry extensibility hooks**: custom `Discoverer` and `ModuleValidator` interfaces and runtime registration methods.
- **Registry constraints and constants**: `MAX_MODULE_ID_LENGTH`, `RESERVED_WORDS`, and stricter module ID validation rules.
- **Context interoperability APIs**: `Context.toJSON()`, `Context.fromJSON()`, and `ContextFactory` interface.

### Changed
- `Context.create()` now accepts optional `traceParent` and can derive `traceId` from inbound distributed trace headers.
- `Registry.discover()` now supports async custom discovery/validation flow in addition to default filesystem discovery.
- `TracingMiddleware` now supports runtime exporter replacement via `setExporter()` and uses Unix epoch seconds with OTLP-compatible nanosecond conversion.
- Public exports were expanded in `index.ts` to expose new cancellation, extension, tracing, registry, and async-task APIs.
- `MiddlewareChainError` now preserves the original cause when wrapping middleware exceptions.

### Fixed
- Improved cancellation correctness by bypassing middleware error recovery for `ExecutionCancelledError`.
- Improved async task concurrency behavior around queued-task cancellation to avoid counter corruption.
- Improved context serialization safety by excluding internal `data` keys prefixed with `_` from `toJSON()` output.

### Tests
- Added comprehensive tests for cancellation, async task management, extension wiring, trace context parsing/injection, registry hot-reload/custom hooks, and OTLP export behavior.

## [0.4.0] - 2026-02-23

### Changed
- Improved performance of `Executor.stream()` with optimized buffering.

### Added
- Added new logging features for better observability in the execution pipeline.
- **ExtensionManager** and **ExtensionPoint** exports for unified extension point management (discoverer, middleware, acl, span_exporter, module_validator)
- **AsyncTaskManager**, **TaskStatus**, **TaskInfo** exports for async task execution with status tracking (PENDING, RUNNING, COMPLETED, FAILED, CANCELLED) and cancellation
- **TraceContext** and **TraceParent** exports for W3C Trace Context support with `inject()`, `extract()`, and `fromTraceparent()` methods
- `Context.create()` accepts optional `traceParent` parameter for distributed trace propagation

### Fixed
- Resolved issues with error handling in `context.ts`.

### Co-Authors
- Claude Opus 4.6 <noreply@anthropic.com>
- New Contributor <newcontributor@example.com>

### Added

- **Error classes and constants**
  - `ModuleExecuteError` — New error class for module execution failures
  - `InternalError` — New error class for general internal errors
  - `ErrorCodes` — Frozen object with all 26 error code strings for consistent error code usage
  - `ErrorCode` — Type definition for all error codes
- **Registry constants**
  - `REGISTRY_EVENTS` — Frozen object with standard event names (`register`, `unregister`)
  - `MODULE_ID_PATTERN` — Regex pattern enforcing lowercase/digits/underscores/dots for module IDs (no hyphens allowed to ensure bijective MCP tool name normalization)
- **Executor methods**
  - `Executor.callAsync()` — Alias for `call()` for compatibility with MCP bridge packages

### Changed

- **Module ID validation** — Registry now validates module IDs against `MODULE_ID_PATTERN` on registration, rejecting IDs with hyphens or invalid characters
- **Event handling** — Registry event validation now uses `REGISTRY_EVENTS` constants instead of hardcoded strings
- **Test updates** — Updated tests to use underscore-separated module IDs instead of hyphens (e.g., `math.add_ten` instead of `math.addTen`, `ctx_test` instead of `ctx-test`)

### Fixed

- **String literals in Registry** — Replaced hardcoded `'register'` and `'unregister'` strings with `REGISTRY_EVENTS.REGISTER` and `REGISTRY_EVENTS.UNREGISTER` constants in event triggers for consistency

## [0.3.0] - 2026-02-20

### Changed
- Use shallow merge for `stream()` accumulation instead of last-chunk.

### Added
- Add `Executor.stream()` async generator and `ModuleAnnotations.streaming` for streaming support in the core execution pipeline.

### Co-Authors
- Claude Opus 4.6 <noreply@anthropic.com>

### Added

- **Error classes and constants**
  - `ModuleExecuteError` — New error class for module execution failures
  - `InternalError` — New error class for general internal errors
  - `ErrorCodes` — Frozen object with all 26 error code strings for consistent error code usage
  - `ErrorCode` — Type definition for all error codes
- **Registry constants**
  - `REGISTRY_EVENTS` — Frozen object with standard event names (`register`, `unregister`)
  - `MODULE_ID_PATTERN` — Regex pattern enforcing lowercase/digits/underscores/dots for module IDs (no hyphens allowed to ensure bijective MCP tool name normalization)
- **Executor methods**
  - `Executor.callAsync()` — Alias for `call()` for compatibility with MCP bridge packages

### Changed

- **Module ID validation** — Registry now validates module IDs against `MODULE_ID_PATTERN` on registration, rejecting IDs with hyphens or invalid characters
- **Event handling** — Registry event validation now uses `REGISTRY_EVENTS` constants instead of hardcoded strings
- **Test updates** — Updated tests to use underscore-separated module IDs instead of hyphens (e.g., `math.add_ten` instead of `math.addTen`, `ctx_test` instead of `ctx-test`)

### Fixed

- **String literals in Registry** — Replaced hardcoded `'register'` and `'unregister'` strings with `REGISTRY_EVENTS.REGISTER` and `REGISTRY_EVENTS.UNREGISTER` constants in event triggers for consistency

## [0.2.0] - 2026-02-20

### Added

- **Error classes and constants**
  - `ModuleExecuteError` — New error class for module execution failures
  - `InternalError` — New error class for general internal errors
  - `ErrorCodes` — Frozen object with all 26 error code strings for consistent error code usage
  - `ErrorCode` — Type definition for all error codes
- **Registry constants**
  - `REGISTRY_EVENTS` — Frozen object with standard event names (`register`, `unregister`)
  - `MODULE_ID_PATTERN` — Regex pattern enforcing lowercase/digits/underscores/dots for module IDs (no hyphens allowed to ensure bijective MCP tool name normalization)
- **Executor methods**
  - `Executor.callAsync()` — Alias for `call()` for compatibility with MCP bridge packages

### Changed

- **Module ID validation** — Registry now validates module IDs against `MODULE_ID_PATTERN` on registration, rejecting IDs with hyphens or invalid characters
- **Event handling** — Registry event validation now uses `REGISTRY_EVENTS` constants instead of hardcoded strings
- **Test updates** — Updated tests to use underscore-separated module IDs instead of hyphens (e.g., `math.add_ten` instead of `math.addTen`, `ctx_test` instead of `ctx-test`)

### Fixed

- **String literals in Registry** — Replaced hardcoded `'register'` and `'unregister'` strings with `REGISTRY_EVENTS.REGISTER` and `REGISTRY_EVENTS.UNREGISTER` constants in event triggers for consistency

## [0.1.2] - 2026-02-18

### Fixed

- **Timer leak in executor** — `_executeWithTimeout` now calls `clearTimeout` in `.finally()` to prevent timer leak on normal completion
- **Path traversal protection** — `resolveTarget` in binding loader rejects module paths containing `..` segments before dynamic `import()`
- **Bare catch blocks** — 6 silent `catch {}` blocks in registry and middleware manager now log warnings with `[apcore:<subsystem>]` prefix
- **Python-style error messages** — Fixed `FuncMissingTypeHintError` and `FuncMissingReturnTypeError` to use TypeScript syntax (`: string`, `: Record<string, unknown>`)
- **Console.log in production** — Replaced `console.log` with `console.info` in logging middleware and `process.stdout.write` in tracing exporter

### Changed

- **Long method decomposition** — Broke up 4 oversized methods to meet ≤50 line guideline:
  - `Executor.call()` (108 → 6 private helpers)
  - `Registry.discover()` (110 → 7 private helpers)
  - `ACL.load()` (71 → extracted `parseAclRule`)
  - `jsonSchemaToTypeBox()` (80 → 5 converter helpers)
- **Deeply readonly callChain** — `Context.callChain` type narrowed from `readonly string[]` to `readonly (readonly string[])` preventing mutation via push/splice
- **Consolidated `deepCopy`** — Removed 4 duplicate `deepCopy` implementations; single shared version now lives in `src/utils/index.ts`

### Added

- **42 new tests** for previously uncovered modules:
  - `tests/schema/test-annotations.test.ts` — 16 tests for `mergeAnnotations`, `mergeExamples`, `mergeMetadata`
  - `tests/schema/test-exporter.test.ts` — 14 tests for `SchemaExporter` across all 4 export profiles
  - `tests/test-logging-middleware.test.ts` — 12 tests for `LoggingMiddleware` before/after/onError

## [0.1.1] - 2026-02-17

### Fixed

- Updated logo URL in README

### Changed

- Renamed package from `apcore` to `apcore-js`
- Updated installation instructions

## [0.1.0] - 2026-02-16

### Added

- **Core executor** — 10-step async execution pipeline with timeout support via `Promise.race`
- **Context system** — Execution context with trace IDs, call chains, identity, and redacted inputs
- **Config** — Dot-path configuration accessor
- **Registry system**
  - File-based module discovery (`scanExtensions`, `scanMultiRoot`)
  - Dynamic entry point resolution with duck-type validation
  - YAML metadata loading and merging (code values + YAML overrides)
  - Dependency parsing with topological sort (Kahn's algorithm) and cycle detection
  - ID map support for custom module IDs
  - Schema export in JSON/YAML with strict and compact modes
- **FunctionModule** — Schema-driven module wrapper with TypeBox schemas
- **Binding loader** — YAML-based module registration with three schema modes (inline, external ref, permissive fallback)
- **ACL (Access Control List)**
  - Pattern-based rules with glob matching
  - Identity type and role-based conditions
  - Call depth conditions
  - Dynamic rule management (`addRule`, `removeRule`, `reload`)
  - YAML configuration loading
- **Middleware system**
  - Onion-model execution (before forward, after reverse)
  - Error recovery via `onError` hooks
  - `BeforeMiddleware` and `AfterMiddleware` adapters
  - `LoggingMiddleware` for structured execution logging
- **Observability**
  - **Tracing** — Span creation, `InMemoryExporter`, `StdoutExporter`, `TracingMiddleware` with sampling strategies (full, off, proportional, error_first)
  - **Metrics** — `MetricsCollector` with counters, histograms, Prometheus text format export, `MetricsMiddleware`
  - **Logging** — `ContextLogger` with JSON/text formats, level filtering, `_secret_` field redaction, `ObsLoggingMiddleware`
- **Schema system**
  - JSON Schema to TypeBox conversion
  - `$ref` resolution
  - Schema validation
  - Strict transforms (`additionalProperties: false`)
  - LLM description injection and extension stripping
- **Error hierarchy** — 20+ typed error classes with error codes, details, trace IDs, and timestamps
- **Pattern matching** — Glob-style pattern matching for ACL rules and module targeting
- **Comprehensive test suite** — 385 tests across 29 test files

---

[0.20.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.19.0...v0.20.0
[0.19.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.17.1...v0.18.0
[0.17.1]: https://github.com/aiperceivable/apcore-typescript/compare/v0.17.0...v0.17.1
[0.17.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.15.1...v0.16.0
[0.15.1]: https://github.com/aiperceivable/apcore-typescript/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.14.1...v0.15.0
[0.14.1]: https://github.com/aiperceivable/apcore-typescript/compare/v0.14.0...v0.14.1
[0.14.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.13.1...v0.14.0
[0.13.1]: https://github.com/aiperceivable/apcore-typescript/compare/v0.13.0...v0.13.1
[0.13.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/aiperceivable/apcore-typescript/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/aiperceivable/apcore-typescript/releases/tag/v0.1.0
