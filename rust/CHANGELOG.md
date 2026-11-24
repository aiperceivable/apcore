# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note on versioning:** This crate starts at `0.13.0` rather than `0.1.0` to stay in sync
> with the [apcore-python](https://github.com/aiperceivable/apcore-python) and
> [apcore-typescript](https://github.com/aiperceivable/apcore-typescript) packages.
> All three SDKs implement the same protocol specification and share a unified version line.

---

## [0.33.0] - 2026-11-18

### Added

- Binding files accept a `resources` block (`timeout` in milliseconds). The
  loader merges it into `annotations.extra["resources"]`, where `BuiltinExecute`
  already reads the per-module budget; previously a `resources:` key was
  rejected by `BindingEntry`'s `deny_unknown_fields` and a budget could not be
  declared from YAML at all.

### Fixed

- `MODULE_TIMEOUT` raised by a module call is no longer retryable by default.
  It carries `retryable: true` only when the module declares
  `annotations.idempotent`. This includes deadlines that expire after a stream
  has emitted chunks. Step-budget timeouts keep the spec default.

### Changed

- Refresh `Cargo.lock` (`tokio` 1.54.1, `openssl` 0.10.82,
  `rustls-webpki` 0.103.17, `mio` 1.2.5, `libc` 0.2.191, and other patch
  updates). No other crate API changes.

## [0.32.3] - 2026-10-25

### Fixed

- Drain the actual module execution tree when cancelling owned background tasks or shutting down their manager. Serialize cancellation, preserve winning terminal states, and finish local teardown even when the task store fails during shutdown. Executor deadlines still report immediately and allow cooperative cleanup; they do not forcibly terminate module code.
- Track execution lifetime through linked `CancelToken` scopes. The additive `drain()` method waits for module futures and their resources to finish, and `has_active_executions()` lets integrations retain only live cleanup work. Reclaim manager bookkeeping after late cleanup even if the terminal task record was already deleted.

## [0.32.2] - 2026-10-19

### Fixed

- Preserve optional enum, const and composed-schema constraints during A23 strict conversion by wrapping the complete schema in a nullable union. Required fields remain unchanged, null-only types are not duplicated, and shared conformance cases verify both schema parity and value validation. Tracked in [apcore#124](https://github.com/aiperceivable/apcore/issues/124).

## [0.32.1] - 2026-10-14

### Fixed

- Resolve local schema references during sensitive-value redaction, including
  escaped JSON Pointers, scoped embedded resource identifiers/anchors and recursive/composed schemas.
  Preserve object-root shape and apply dialect-specific schema indexing/array rules.
  Preserve deep-copy behavior, configured redaction and protected prefix rules;
  external schema resources are never retrieved. (aiperceivable/apcore-rust#41)

## [0.32.0] - 2026-10-08

### Added

- **Public bare-name canonicalization** (#40) — `canonicalize_name`, `CanonicalNameResult`, and `CanonicalNameError` are exported from the crate root and `utils`. One arbitrary ASCII name becomes a canonical segment or a structured snake_case diagnostic, preserving the original input without transliteration, invented prefixes, truncation, or changes to Algorithm A02 module ID normalization.

### Security

- **Providers reach the gate they configure** (D-129, spec 1.61.0) — the built-in `acl_check` / `approval_gate` steps hold no provider and read the executor's ACL, `ApprovalHandler` and `ExecutionPolicy` on every call, however the strategy was supplied, so `governance_state()` reports what the running gate enforces; pinned by `gate_provider_binding.json`.
- **Governance gates cannot be weakened by configuration** (D-130) — `ignore_errors: true` and any `match_modules` on `acl_check` / `approval_gate`, and `pure: true` on `approval_gate`, fail with `PIPELINE_CONFIGURATION_ERROR` naming the step and key, from `pipeline.configure` and from `ExecutionStrategy::new` / `configure_step` / `replace` / `replace_with` / `insert_*`; a gate configured with `timeout_ms` keeps its gate identity and capability contract.
- **Logging middleware logs the captured values** (D-131) — `ObsLoggingMiddleware` and `LoggingMiddleware` log `context.redacted_inputs` / `redacted_output`, never the raw inputs or output, so an `x-sensitive` field is redacted with or without a `RedactionConfig`.

### Fixed

- **Test emptiness assertions use value comparisons for Clippy compatibility.** Failed assertions now display unexpected values; runtime behavior and warning-as-error lint enforcement remain unchanged.
- **Require async-trait 0.1.92 or newer** to avoid redundant generated `must_use` attributes under Rust 1.99 Clippy, without suppressing lint checks or changing async trait behavior.
- **Track Cargo.lock** so local checks and CI resolve the same dependency versions.
- **Reload test failure counters use a compare-exchange loop** instead of the deprecated `fetch_update`, preserving atomic conditional decrement behavior and the minimum supported Rust version.
- **Issue #123 conformance hardening** — timeouts signal isolated child cancellation without dropping cooperative module cleanup; preflight collects independent failures and always serializes predictions; binding files reject unknown fields and uninferable targets; schema errors carry JSON Pointer paths and keywords; raw and typed Anthropic/OpenAI exports remove extension keywords without removing property or definition names, and MCP moves true-only approval/streaming hints to `_meta`.
- **Contextual ephemeral audit and configuration parity** — context-aware registry registration/unregistration emits one credential-safe audit event, environment dispatch reserves the exact `APCORE` prefix and routes unmatched overrides into the framework namespace, and configured subscribers use the standard circuit-breaker wrapper.

- **`OTLPExporter` posts to its endpoint as given** (#123) — it appended `/v1/traces` to a URL that already ended in it, so config-driven OTLP tracing posted to `/v1/traces/v1/traces`; the endpoint is the full traces URL, as in apcore-python and apcore-typescript.
- **`BatchSpanProcessor` stops only when its last handle is dropped** (#123) — dropping the original after handing a clone to the middleware stopped the worker.
- **`MetricsCollector::new`, `UsageCollector::new`, `ErrorHistory::new` / `with_limits` / `with_store*` attach the in-memory storage backend** (D-113, #123); `MetricsCollector` and `UsageCollector` gain `storage_backend()`.
- **A module registered through `APCore` can make nested calls** (#123) — `ctx.executor()` returns the running `Executor`; `APCore` shares its Executor, `Executor::into_shared` / `enable_nested_calls` make any other one reachable, and `APCore::executor_arc` exposes it.
- **`ContextLogger` redacts `extra` with the default `obs.redaction` rules at every depth**, or with a config set via `set_redaction_config` (#123); it redacted only top-level `_secret_*` keys.
- **`RedactionConfigBuilder` compiles `value_patterns` case-insensitively**, as `from_config` and the other SDKs do (#123).
- **`Registry::discover(&discoverer)` passes the registry's extension roots** (#123), and a `DefaultDiscoverer` without a module factory fails with `MODULE_LOAD_ERROR` on a discovered file instead of registering nothing.
- **`Registry::watch` replaces a changed module and unregisters a removed one** (#123) — it only registered new files.
- **`register_versioned` accepts another version of a registered module** (#123) — `get` / `get_definition` return the highest; the same version twice is still `DUPLICATE_MODULE_ID`; `unregister` removes every version.
- **The `sys_modules.control.overrides_path` config key is written back to, and `reload_module`'s `reload_config` works for an `APCore` client** (#123) — only an `overrides_path` option was persisted, and the reload module was registered without a `Config`.
- **An unknown strategy name raises `STRATEGY_NOT_FOUND`** (#123), not `GENERAL_INVALID_INPUT`.
- **`Executor::stream`'s non-streaming fallback runs the strategy's own `execute` step** (#123) — a replaced `execute` step was bypassed.
- **`AsyncTaskManager` gains `list_tasks_async` / `cleanup_async`, and `TaskStore::store_type_name` has a default** (#123).
- **`APCore::use_middleware` / `Executor::use_middleware` warn on a duplicate middleware `name()`** (#123), and identities stay aligned with registrations after priority-ordered inserts.
- **`TraceContext::inject` uses the current span as the parent** (#123) — `TracingMiddleware` keeps its stack under `_apcore.mw.tracing.spans`; `InMemoryExporter` defaults to 10 000 spans; `ErrorHistory::new(n)` keeps the 1000 total limit and `get` orders by creation.
- **`system.control.toggle_feature` is declared `idempotent: true`** (#123).
- **A custom module-validator rejection raises `GENERAL_INVALID_INPUT`** (#123), not `MODULE_LOAD_ERROR`.

## [0.31.0] - 2026-09-22

### BREAKING

- **The deprecation warning moves from registration to `get_definition`, and its dedupe key carries the notice** (D-89, v1.59.0) — reads never warned at all before; the key is now `(module_id, version, canonical notice)`.
- **`get`, `get_all` and `unregister` now reject an unknown extension point** (D-108) — signature change: all three now return `Result`.
- **`ContextFactory::create_context` takes the runtime request, not a pre-extracted `Identity`** (D-76) — matches the spec's canonical signature; add `type Request = YourRequest`.
- **`ExtensionKind` holds `Arc`, not `Box`; `ExtensionManager::register` returns an `ExtensionHandle`** (D-78, D-91) — `apply` no longer consumes registrations, so applying one manager to two registry/executor pairs now wires both, and removal is expressible from outside the manager.
- **`Registry::on` returns `Result<u64, ModuleError>` and rejects unknown event names** (D-80) — a typo used to silently subscribe to nothing.
- **Every `AsyncTaskManager` method that touches the store now propagates the store's error** (D-81) — previously discarded, so a store outage reported "not found" and `cancel()` could report success on a failed write.
- **`guard_call_chain` takes the chain, not a `Context`** (D-83) — the spec's own Rust example did not compile against the old signature, and a non-`serde_json::Value` services type could not call the guard at all. Use `guard_call_chain_with_repeat`.
- **The approval gate now fires on the UNION of the live module's and the registry descriptor's governance annotations** (D-96) — previously fail-open when the two disagreed.
- **Per-class markers are the only multi-class opt-in; the file-level `DiscoveryConfig` gate is inert** (D-107).
- **A binding's declared `annotations` reach registration in full** — a `streaming: true` binding with no `stream()` is now rejected (DEC-002); seven of twelve annotation keys were previously silently discarded into `extra`.
- **`DependencyInfo` serializes its constraint as `version`, not `version_constraint`** (SYS-12).
- **`Executor::stream` now runs the full pipeline tail on the non-streaming fallback path** (STR-1) — output-schema validation and after-middleware previously never ran on it.
- **Stream Phase 3 now runs the strategy's own post steps instead of a hardcoded pair** (STR-2) — a `minimal`-preset stream previously ran steps the strategy had explicitly removed.
- **A call with no identity now reaches the module with `context.identity == None`**, instead of a synthesized `@external` principal (D-103).
- **`RedactionConfigError::InvalidFieldPattern` is removed** (spec v1.37.0 §9.2.3) — every string is now a valid A25 pattern; nothing can be malformed.

### Removed

- **The associated function `Executor::list_strategies()`** (deprecated since 0.20.0, no known callers) — superseded by the instance method (see Fixed).

### Changed

- **`AsyncTaskManager::shutdown()` now attempts every cancellation before it reports** (D-122) — previously stopped at the first failure, leaking active `max_tasks` slots.
- **A panicking ACL audit callback no longer changes the access decision** (D-66) — auditing must not hold a veto over access.
- **`_config.allow_unknown: false` now drops unregistered namespaces** (D-69).
- **Verified conforming, unchanged: D-74, D-77, D-85, D-86, D-87, D-90** — six decisions this crate already satisfied; recorded to close the audit.
- **`FilterSubscriber::matches` calls `match_glob` directly** — a redundant one-line delegation removed.
- **Verified conforming, unchanged: three of ERR-001's five reported sites** — they emit an error *class name* by design, matching Python/TypeScript's own class-name fields.

### Deprecated

- **`acl.default_effect` in `apcore.yaml`** — reaches nothing; the ACL file's own `default_effect` is authoritative. Removed at v2.0 (D-73).
- **`Context::logger`** — no configuration door, and not routed through `tracing`. Removed at v2.0 (#121).
- **`logging.level`/`logging.format`**, no replacement (D-67).
- **Ten declared config keys plus an ACL `audit:` block reach no consumer in any SDK** — now warn once per load; removed no earlier than v2.0 (#118).

### Fixed

- **`schema.strategy` was silently ignored** — `SchemaLoader::with_config` hardcoded `YamlFirst`, and the field was never read by any branching logic at all (A-C-003, see Added: `get_schema`).
- **The skip-asymmetry guard could not see this crate's `#[ignore]`d tests** — four contract tests were `#[ignore]`d for symbols that exist, and two live ones asserted a stand-in.
- **The executor's ACL step is pinned to the ASYNC path** (D-105) — no behaviour change, only test coverage; a whole extension point was reachable from every door except the enforcing one.
- **A symlinked module file is now discovered once, under its real path** (D-127) — `follow_symlinks` previously governed directories only, and an aliased directory was traversed twice.
- **A malformed annotation value (`extra`/`cache_ttl`) is dropped with a warning** instead of rejecting the whole module (D-115).
- **`MiddlewareManager::remove`/`remove_handle` now clear the duplicate-identity entry** (D-114).
- **Every `system.*` module declares `open_world: false`** (D-119) — previously defaulted to the opposite.
- **A malformed dependency constraint is now reported as malformed**, not as a version mismatch (D-85).
- **`ErrorCode::TaskStoreUnavailable` now exists and is framework-reserved** (D-92).
- **`version`, `examples` and `dependencies` were declared, validated, then discarded at registration** — the unfixed half of D-97; a binding declaring `version: "2.3.0"` registered as `1.0.0`.
- **SECURITY: `system.manifest.*` annotations advertised governance the gate does not enforce** (D-125) — the manifest projected from the descriptor alone while the gate reads the union.
- **SECURITY: a `$ref` into an external schema could bind to the caller's definitions**, including past an `x-sensitive` marking (D-124) — fallback now scoped to the declaring document.
- **SECURITY: the approval gate decided from the registry descriptor, not the module — two independent bypasses that compose** (D-96, D-97) — see BREAKING; a module could execute ungated on this SDK and gated on the peers.
- **SECURITY: `$ref` sibling keys, including `x-sensitive`, were discarded during schema resolution** (D-98) — a data leak, not cosmetic: redaction is driven by `x-sensitive` on the resolved schema.
- **`Executor::call` now rejects an over-length module ID at the entry guard** (D-75), instead of reaching the registry and reporting `MODULE_NOT_FOUND`.
- **A stalled topological sort is no longer reported as a cycle** (D-79).
- **`InMemoryTaskStore` now lists tasks in submission order**, not by random UUID (D-82).
- **The call-chain limit floors now carry `ai_guidance`** (D-84).
- **The ACL now emits the §6.5 "conditions present but no context" warning**, deduped per rule index and cleared on rule mutation (D-88).
- **The deprecation warning now fires at most once per `(module_id, version)` per registry** (D-89).
- **`Executor::list_strategies()` is now an instance method and includes the executor's own current strategy.**
- **`_approval_token` is now removed from arguments on EVERY exit from the approval gate**, not only the gated-and-handled path.
- **`PlatformNotifyMiddleware`'s error rate was permanently 0.0** due to a label-key mismatch between writer and reader — `apcore.health.error_threshold_exceeded` could never fire. A second defect in the same area (`f64` counters read as `u64`) is fixed alongside.
- **A malformed version constraint (`"latest"`, `"v1.0.0"`) no longer silently passes** as satisfied or mismatched.
- **`descriptor.sunset_date` is now derived from `metadata["x-deprecation"]`**, and a deprecated module logs a warning.
- **`merge_module_metadata` now merges annotations field-by-field and emits `dependencies`** — previously whole-replaced, dropping fields YAML did not override.
- **The depth and circular arms of `guard_call_chain_with_repeat` now carry `ai_guidance`.**
- **`CancelToken::check()` no longer fabricates the module id `"@unknown"`.**
- **`AsyncTaskManager::shutdown` now stops the reaper, and dropping a `ReaperHandle` no longer arms a second one** — a sweep loop previously outlived `shutdown` and could double-run.
- **A task's `JoinHandle` can no longer be registered after the task has already finished** — closed a race that let `cancel()` abort an already-finished task.
- **SECURITY-adjacent: the executor's ACL step took the synchronous path**, so the entire `register_async_condition` extension point was dead in the only path that enforces (D-105).
- **A local `#/…` `$ref` now falls back to the schema node when it does not resolve at the file root** (D-104) — previously only one of two normative layouts loaded.
- **A p99 estimate beyond the largest bucket is now that bucket, not zero** (D-106) — the existing unit test had pinned the defect.
- **Prometheus label values are now escaped** (OBS-002) — an unescaped `"` previously broke the entire scrape.
- **`TASK_LIMIT_EXCEEDED` now resolves to `retryable: true`** (ERR-003), matching the peers.
- **Five error codes now carry the peers' default `ai_guidance`** (ERR-004).
- **The tracing span `error_code` and obs-logging `error_code` now carry the canonical wire code**, instead of the Rust enum's Debug name (ERR-001).
- **A streamed call's audit record now carries the same output projection as its non-streamed twin** (MW-002).
- **`system.manifest.module`/`.full` now share one entry builder** and stop hardcoding `documentation`/`metadata`/`source_path` (SYS-6–SYS-9, SYS-11).
- **`system.health.summary` now reports the most FREQUENT error as `top_error`**, not the most recent (SYS-4).
- **Bulk `system.control.reload_module` now emits real versions**, instead of the literal `"unknown"` (SYS-15).
- **The four `system.health.*`/`system.manifest.*` output schemas now declare their field contracts**, instead of a bare `{"type": "object"}` (SYS-24).
- **`collect_constraint_errors` and `validate_key_constraint` were two hand-written copies of one table that had drifted** — `validate()` now delegates, so a new constraint is enforced automatically.
- **Four pattern-valued config surfaces each used a different glob dialect** — unified under `match_glob` (Algorithm A25, #116/#117); the redaction-key case had a real disclosure consequence (a case-sensitivity bug hid uppercase secret-key patterns).
- **`stream.max_merge_depth` is now read from config** instead of ignored; a misconfigured value still falls back to the safe default of 32 (#118).
- **The four per-group `sys_modules.*.enabled` flags are now read**, `control.enabled` included (#118).

### Added

- **`BatchSpanProcessor::force_flush(timeout_ms: u64) -> bool`** (A-C-002) — mirrors apcore-python; previously only a private, background-loop-only `flush_batch` existed.
- **`SchemaLoader::get_schema(module_id, native_input_schema, native_output_schema)`** (A-C-003) — mirrors Python/TypeScript's strategy-driven resolution; `self.strategy` was previously set but never consulted.
- **`Registry::set_discoverer_shared`/`set_validator_shared`, `Executor::set_acl_shared`/`set_approval_handler_shared`/`use_middleware_shared`, `MiddlewareManager::add_shared`, `TracingMiddleware::set_exporter_shared`, `CompositeExporter::from_shared`** — `Arc`-taking counterparts of the existing `Box`/value-taking setters.
- **`ExtensionManager::unregister_handle` and `ExtensionHandle`** (D-91).
- **`apcore::utils::guard_call_chain_for_context`** (D-83).
- **`registry_events::ALL`/`RegistryEvents::ALL`** (D-80).
- **`Module::documentation` and `Module::metadata`** (D-97).
- **`ExtensionManager::get`/`get_all`/`unregister`** — previously the manager could register, count and clear extensions but not read one back.
- **`ACL::with_audit_config`/`ACL::set_audit_config`** — direct construction previously had no way to declare an `audit:` block.
- **`AsyncTaskManager::stop_reaper`** — mirrors apcore-python.
- **`RunOptions::only_steps`** (STR-1, STR-2) — the Rust counterpart of apcore-python's `post_stream` sub-strategy slice.
- **`RefResolver::with_node_fallback` and `RefResolver::resolve_in_document`** (D-104).
- **`DiscoveredClass::new`/`with_multi_class`, `MultiClassEntry::with_multi_class`** (D-107).
- **`ModuleError::module_not_found`/`module_disabled`/`module_timeout`/`dependency_not_found`/`dependency_version_mismatch`** (ERR-004).
- **A configured `pipeline:` section is applied** (D-72).
- **The five `observability.tracing.*` keys are wired** (D-68).
- **`extensions.roots`' namespace half is honoured** (D-70) — this crate read the key but dropped the namespaces.
- **`id_map.overrides` is read by `DefaultDiscoverer::from_config`** (D-71).
- **`apcore::utils::match_glob` — Algorithm A25**, the one matcher for every glob-dialect pattern-valued config value (spec §9.2.3).
- **The six `validation.*` limits are implemented**, unconstrained by default (#118) — purely additive.
- **`stream::resolve_merge_depth` and `stream::deep_merge_chunks_checked_with_depth`** — the config-aware entry points behind the `stream.max_merge_depth` fix.
- **`Config::validate()` reports an `obs.redaction.regex_patterns` entry that does not compile** (§9.2.3 requirement 6d).
- **A diagnostic for `?` in an ACL pattern** (§6.2.2) — `?` is a literal in ACL matching; reported, never rejected.

---

## [0.30.0] - 2026-09-06

### Added

- **`bindings.dir` and `bindings.pattern` are canonical defaults (spec v1.36.0).** `schemas/defaults.schema.json` gained the `bindings` section it never had, so `Config::get("bindings.dir")` now resolves `./bindings` and `Config::get("bindings.pattern")` `*.binding.yaml` for a configuration that declares neither. Until this release the two keys were declared in PROTOCOL_SPEC §9.1.1 and in `apcore-config.schema.json` but absent from the file that IS the canonical default tier, and `conformance/fixtures/config_key_governance.json` pins this SDK's default table to that file — so no SDK *could* carry the default, and all three hardcoded `./bindings` at their binding loader instead. `BindingLoader`'s `DEFAULT_BINDING_DIR` / `DEFAULT_BINDING_PATTERN` are kept for the one invocation that supplies no `Config` at all (`load_binding_dir_with_config(None, None, None)`), and are now pinned equal to `Config::default_for` by a test so the two cannot drift.

  **One consequence worth knowing:** `bindings.dir` is path-typed, so a configuration that leaves it undeclared now has a *relative* path-typed value (`./bindings`) standing in its merged view where it previously had none. A config whose other path-typed keys are all absolute, loaded from a directory that is not the process working directory, will therefore emit the §9.2.2 deprecation warning it did not emit before. Spelling `bindings.dir` absolutely silences it, exactly as for its three sibling roots.

### Fixed

- **A set-but-empty `APCORE_*` variable silently blanked a path-typed configuration key (spec v1.36.0 §9.2.1 requirement 5).** §9.2 counts a *set but empty* variable as an override, so `export APCORE_ACL_ROOT=` — and a variable inherited empty from a container spec, which is indistinguishable to the tooling — replaced a directory the configuration file correctly declared with `""`, which every filesystem API accepts as a legal relative path meaning the working directory. Nothing errored and nothing reported a difference: the config said `./acl`, the process looked in `.`. The same shape is on record for `APCORE_CONFIG_FILE` ([apcore#88](https://github.com/aiperceivable/apcore/issues/88)), where an empty value injected a phantom `config.file` key; the path-typed keys are the population where it fails silently rather than loudly.

  An empty value for any key in `Config::path_typed_keys()` is now **discarded**, and resolution falls through to the configuration file and then to the canonical default exactly as if the variable had been unset. A warning naming both the key and the variable is logged. The guard sits at the single point an environment tier becomes a configuration value, not at each consumer, so no present or future reader of a path-typed key can see the empty string. Keys outside the closed path-typed set are untouched: an empty `APCORE_LOGGING_LEVEL` is still an override, and so is `APCORE_BINDINGS_PATTERN`, which §9.2.1 requirement 4 states is not path-typed despite sitting in the same section as `bindings.dir`.

### Changed

- **The §9.2.2 deprecation warning's once-per-load cadence is now normative rather than a local choice.** Behaviour is unchanged — this SDK already emitted per load — but spec v1.36.0 adopted that reading and forbids suppressing the warning with process-global state, so the doc comment cites the clause instead of arguing the point, and two tests pin it: a once-per-process flag passes every single-load assertion, and only the *second* affected document goes silent.

- **The three §9.2.x / §5.12.6 conformance drivers track the repaired fixtures, and carry no divergences.** `bindings_dir_resolution` grew a ninth case for §5.12.6 clause 2 (the environment tier must reach the loader through §9.2's mechanism, never a raw `APCORE_BINDINGS_DIR` read), its descriptors moved from one shared entry to a named map with distinct module IDs, and its target field is `target` rather than `target_id`. `config_project_root`'s tier-6 and tier-7 cases name the *tier* through tokens the driver resolves at the running platform's §9.14 location, instead of a POSIX spelling that failed on macOS. Both `#[ignore]`d divergence cases are gone: §5.12.6 clause 5 settled "a missing binding directory raises" in this SDK's favour, and `no_warning_when_all_path_values_absolute` became satisfiable once the case spelled every §9.2.1 key absolutely.

---

## [0.29.0] - 2026-09-05

### BREAKING

- **`APCore::on` / `on_subscriber` / `on_fn` / `off` / `off_by_type` now return `Result`, and error with `SYS_MODULES_DISABLED` when the client has no event bus.** They previously could not fail: `on()` called `get_or_insert_with` on `event_emitter`, so subscribing on a client built without `sys_modules` silently created a *standalone* bus — the subscription reported success, and no framework event ever reached it, because the sys-modules and registry emitters are a different bus. `off()` mirrored the gap from the other side, returning a plain `false` that read as "no such subscriber" when the real answer was "there is no bus at all". apcore-python and apcore-typescript both raise `SysModulesDisabledError` here, and `docs/features/apcore-client.md`'s Error Behavior table has required it all along; this SDK's own conformance tests `on_error_sys_modules_disabled` and `off_error_sys_modules_disabled` were carrying `#[ignore]` attributes naming the gap, and are now enabled.

  The guard is the one `disable()` / `enable()` already use, so the whole governance-adjacent surface answers consistently. `off()` keeps its `bool` payload inside the `Ok` (Python and TypeScript return void): "no such subscriber" and "events are off" are genuinely different answers, and collapsing the first into the second is what made the original defect invisible. `off_by_type` is guarded for the same reason, so no entry point on the subscription surface is left silently returning a zero value.

  **Migration** — add `?` or `.expect(...)` at the call site; a client built with `sys_modules.enabled=true` (and `sys_modules.events.enabled=true`) always returns `Ok`:

  ```rust
  // Before (0.28.0 and earlier)
  let id = client.on("apcore.module.toggled", |e| println!("{}", e.event_type));
  client.off(&id);

  // After
  let id = client.on("apcore.module.toggled", |e| println!("{}", e.event_type))?;
  client.off(&id)?;
  ```

- **SECURITY: a pattern list with no operands made an ACL rule inert, and under `default_effect: allow` that permitted the call the rule named (spec v1.31.0 §6.2.1, [apcore#112](https://github.com/aiperceivable/apcore/issues/112)).** `callers` / `targets` of `[]`, `["$or"]` or `["$not"]` can never match; the matcher returned `false` and `validate_rules()` reported nothing, so a `deny` rule an operator wrote, loaded and validated contributed nothing to the decision — the outcome tracked `default_effect` exactly across all twelve combinations of the three shapes, both effects and both defaults. Reachable from a plain YAML file: `ACL::load` rejects an *omitted* `callers` / `targets` and permitted an *empty* one. `schemas/acl-config.schema.json` had declared `minItems: 1` and `minLength: 1` on both fields since the file existed, enforced by nothing — the same shape as [apcore#107](https://github.com/aiperceivable/apcore/issues/107) and [apcore#111](https://github.com/aiperceivable/apcore/issues/111).

  A pattern array's shape is now **closed at every entry point**: at least one operand, every element a non-empty string, `$or` at index 0 followed by at least one pattern, `$not` at index 0 followed by exactly one, and `$or` / `$not` nowhere but index 0. Refused with `ErrorCode::ACLRuleError` from `ACL::load`, `ACL::try_new` and `ACL::try_add_rule`, and — since runtime insertion is one of the three doors — by a panic from the infallible `ACL::new` and `ACL::add_rule`, the pairing those methods already have for an out-of-enum `effect`. The refusal names the field and the rule index. All three doors share one `ACL::validate_rule`, so they cannot drift apart.

  **A configuration or a call carrying any of these shapes will stop working.** The affected population is exactly the deployments that believe they have a rule and do not. `targets: []` meaning "everything" becomes `targets: ["*"]`; meaning "nothing" means the rule should be deleted; `["$or"]` and `["$not"]` are the same two readings. **`["$not", p1, p2]` is the one shape with no mechanical migration:** `["$not", p1]` preserves what the rule has actually been doing, but if `NOT (p1 OR p2)` was intended, a leading `deny` is *not* equivalent — a non-matching rule lets evaluation continue to later rules and a `deny` ends the scan — so the rewrite has to be done by hand against the rule's position.

  Three normative statements are replaced rather than reinterpreted. §6.5's edge-case table required an empty list to make the rule "never match", which is the behaviour above. §6.2.1 required `["$not"]` to "evaluate to false (fail-closed)"; the parenthetical predates §6.1.1 (spec v1.22.0) and is wrong — a non-match is fail-closed on an `allow` rule and fail-**open** on a `deny` one. And `["$not", p1, p2, …]` was implementation-defined, consult the first operand and drop the rest: this SDK did exactly that, so `targets: ["$not", "secrets.a", "secrets.b"]` on an `allow` rule **granted** `secrets.b` — the second target the operator excluded.

  §6.2.1 also states for the first time that a pattern array is **FLAT** — the operators do not nest and there is no precedence, unlike the same tokens in `conditions` — which is why a reserved token outside index 0 is now refused. `["$or", "$not", "a"]` was never "or of not": it was an OR of two literals that matched `a` and also matched a module literally named `$not`, violating §6.2.1's own long-unenforced "MUST NOT match a literal module ID equal to `$or`". Rejecting the token outside index 0 makes that clause hold by construction. Detection is by **equality**, never by a `$` prefix: `["api.*", "$orders.*"]` still loads.

  **The order in which a rule's faults are reported changed too (spec v1.31.0 §6.2.1 point 2).** A rule bad on more than one axis is now refused for the first of `effect` -> `approval` -> `callers` / `targets`, and a rule *set* with more than one bad rule for the **lowest-indexed** bad rule — the index chooses the rule, the axis order chooses the fault inside it. This SDK checked the patterns first, so `{callers: [], targets: [], effect: "Allow"}` was refused here for its patterns and in apcore-python for its `effect`; both were conformant while nothing said otherwise. §6.2.1 states the ordering for the first time — §6.1.6 rule 2 only *implies* `effect` before `approval` — and this SDK is the one that moved. `ACL::load` additionally interleaved its rule-key closure (apcore#107) into the per-rule loop: it used to run that axis across the whole file before any rule's `effect` was looked at, so a file whose rule 0 carried `effect: "Allow"` and whose rule 1 carried an unknown key was refused for rule 1. The error *text* is unchanged; which of several errors you get is not. An invalid `default_effect` is now judged before any rule at `ACL::load`, as it already was at `ACL::try_new`, so the two doors agree on a file that is wrong in both places — and "first" reaches past the individual rules to the file-level checks on the `rules` collection itself, so a document both missing `rules` and carrying an unrecognised `default_effect` names the `default_effect`. It used to name the absent `rules` key, a fault `ACL::try_new` cannot see at all. Both points were unstated when this SDK was measured and are now normative: §6.2.1 says outright that "axis" means every per-rule check a door performs — the rule key set, the missing-field check and the value types included, not only the three it names — and that `default_effect`, carrying no index for the rule ordering to reach, is judged first at every door.

  Two further pieces ship with it. §6.1.4.1 is extended from the pattern field's *type* to its type **and** shape, as the backstop for the one route no door covers — a field assigned onto an already-constructed rule, which no constructor intercepts and which, unlike an unrecognised `effect`, the matcher still reads. Such a rule is UNEVALUABLE: a `deny` rule takes effect and denies, an `allow` rule does not grant, `AuditEntry.handler_error` names the field, and an `approval: required` it carried stays pending (§6.1.1 rule 5's "unknowable scope counts as scope"). And a **validator-only tier 2** reports arrays that are well-formed and still match nothing — `["$not", "*"]`, `["$not", "**"]`, `["@external"]` as a `targets` pattern — which keep loading and change no decision.

- **`ACLRule` is now `#[non_exhaustive]` (apcore#38).** Downstream struct-literal construction no longer compiles — `..Default::default()` included, since that is a struct expression too (`api-surface-conventions.md` §9.1). Build a rule with the new `ACLRule::new` and assign the optional fields afterwards:

  ```rust
  // Before (0.28.0 and earlier)
  let rule = ACLRule {
      callers: vec!["agent.*".to_string()],
      targets: vec!["data.export".to_string()],
      effect: "allow".to_string(),
      approval: None,
      description: Some("Data-admin agents may export.".to_string()),
      conditions: Some(json!({ "roles": ["data_admin"] })),
  };

  // After
  let mut rule = ACLRule::new(
      vec!["agent.*".to_string()],
      vec!["data.export".to_string()],
      "allow",
  );
  rule.description = Some("Data-admin agents may export.".to_string());
  rule.conditions = Some(json!({ "roles": ["data_admin"] }));
  ```

  This is a **decision recorded on the type**, which is what apcore#38 asked for and what the three neighbouring spec-derived types in the same file already had. `AccessDecision`, `AuditEntry` and `RuleValidationFinding` carry the attribute; `ACLRule` — the type the spec extends most often and the type downstream constructs most often — was the one it was never applied to, and no record said whether that was a decision or an oversight. It was an oversight, and it has a measured cost: v1.28.0's `approval` field was a hard compile break for `apexe` (5 sites) and `apcore-mcp-rust` (2, recorded verbatim in its own 0.19.0 changelog: *"`ACLRule` is not `#[non_exhaustive]`, so these were compile breaks under the corrected dependency"*).

  The attribute does not undo that break; it prevents the next one. §6.1.5 closed the rule key set in spec v1.27.0 and §6.1.6 added `approval` in v1.28.0, and the spec is still extending the type — so this is the last release in which adding a field to a rule costs downstream a compile error. The migration is the same work v1.28.0 already forced, done once more and for the last time, which is the trade this issue put plainly and which is settled here rather than left open.

  Pinned by a compile-fail test (`tests/compile_fail/acl_rule_non_exhaustive.rs`) rather than by a doc comment alone: a later change that drops the attribute to "let downstream keep its literals" would reopen the hole silently, and a doc comment cannot fail CI.

- **Framework errors now resolve a default `retryable` from their error code (apcore#36).** `ModuleError::new` left the field `None` for every code, so a `RetryMiddleware` installed by the host only ever retried errors that had called `.with_retryable(true)` explicitly. Three codes now default to `Some(true)`: `MODULE_TIMEOUT`, `APPROVAL_TIMEOUT` and `GENERAL_INTERNAL_ERROR`. This is the behaviour apcore-python and apcore-typescript have always had (both gate on `retryable === true` in the same place).

  **Of those three, `RetryMiddleware` will now automatically retry `MODULE_TIMEOUT` and a module-raised `GENERAL_INTERNAL_ERROR`** — both are raised at pipeline step 8 (`BuiltinExecute`), after `BuiltinMiddlewareBefore` at step 6 has registered the middleware. A host that relies on a module timeout *not* being re-attempted — a non-idempotent module without its own guard — should set `max_retries: 0` or override the field with `with_retryable(false)`.

  `APPROVAL_TIMEOUT` is **not** affected despite its new default, and neither is `ACL_DENIED`: the `on_error` chain runs only when `pipe_ctx.executed_middlewares` is non-empty, and the approval gate (step 5) and ACL check (step 4) both precede `BuiltinMiddlewareBefore` (step 6), so no middleware has run when they fail. The field value still matters to every consumer that reads it off the wire — an A2A caller deciding whether to re-request approval — it just does not reach this crate's retry middleware. See the `Fixed` entry below for why the defaults themselves are not optional.

### Added

- **`ACLRule::new(callers, targets, effect)`** — the semantic constructor `api-surface-conventions.md` §9.2 rule 4 requires for a type carrying required fields, and, with `#[non_exhaustive]`, the only construction path available to a downstream crate. `effect` is deliberately not validated here: the closed `allow` / `deny` set is enforced at the three doors that accept a rule — `ACL::load`, `ACL::try_new` and `ACL::try_add_rule` (apcore#111) — so a rule carrying an effect outside the set cannot reach a live ACL by any route, and a rule under construction is not yet a governance decision.

- **`ModuleError::acl_denied(caller_id, target_id)`** — the builder that carries the default AI recovery guidance and the structured `caller_id` / `target_id` detail for an ACL denial, matching what apcore-python's and apcore-typescript's `ACLDeniedError` constructors have always attached. Overridable through `with_ai_guidance`, which is what `setdefault` and `??` buy in those two.

- **Conformance driver for `acl_pattern_arity.json`** (`tests/test_acl_pattern_arity_conformance.rs`, in the consolidated `it` binary) — all **51** cases, 0 skipped. The 42 `closure` cases are offered at every door this SDK exposes (five API invocations each, both halves of `ACL::new` / `ACL::try_new` and `ACL::add_rule` / `ACL::try_add_rule`), with `expected_refused_axis` and `expected_refused_rule_index` asserted where the fixture carries them. Of the nine `backstop` cases, the two that mutate nothing are executed in full — structured `access`, the legacy boolean, the audit `handler_error` and the `validate_rules()` findings, each with a null key and both resolvability flags false. The remaining seven need a rule mutated **after it is installed in an ACL**, which this SDK has no route to: `ACL::rules` hands back `&[ACLRule]`, there is no `rules_mut`, `ACL` derives nothing and `ACL::new_unchecked` is private. Per the fixture's own instruction those are reported as **satisfied by construction, never as skipped**, and the closure is asserted rather than described — a coerced `fn(&ACL) -> &[ACLRule]`, a new `tests/compile_fail/acl_rules_immutable.rs` proving the assignment does not compile, the already-mutated rule refused at all three doors, and a check that `src/acl.rs` still carries an in-crate backstop test of the same fixture ID.

- **`apcore::EXTERNAL_CALLER`** — the `@external` sentinel an absent `caller_id` resolves to (§6.5), previously a bare literal repeated at nine sites across `acl.rs`, `executor.rs`, `builtin_steps.rs` and `sys_modules/`, plus a second private constant holding the same value. It has to agree across four surfaces resolved in different modules — the ACL matcher, the `AuditEntry`, the `ACL_DENIED` message a refused caller reads, and the context `Executor` synthesizes for a context-less call — and a literal drifting apart at one of them is what the `ACL_DENIED` fix below had to correct. Exported so a downstream adapter can stop defining its own copy.

- **`ApprovalRequest.caller_id` and `ApprovalRequest.action` (decision D-03, `docs/spec/2026-05-decision-log.md`, spec v1.32.0 §7.3.1).** Every SDK's `ApprovalRequest` carried `module_id, arguments, context, annotations, description, tags` and neither field, though `docs/features/approval-system.md`'s Contract block had required both since the Contract blocks were added — the schema was the one place in the ecosystem that had not caught up. Both are additive on the already-`#[non_exhaustive]` type (issue #24) and populated at the one construction site that already exists, `BuiltinApprovalGate`: `caller_id = context.caller_id` (`None`/null for a top-level call, per §5.7 — no `"@external"` substitution, which is an ACL-evaluation-only convention) and `action = module_id` (a flat duplicate, not a separate free-text label, so a handler reads `request.action` without cross-referencing `request.module_id` under a second name). Conformance: `conformance/fixtures/approval_request_fields.json`.

- **`CancelToken::raise_if_cancelled`** — the spec's canonical name (`docs/features/cancellation.md` "## Contract: CancelToken.raise_if_cancelled") for the check this crate already had under the name `check()`. Identical behavior — one delegates to the other — and purely additive: `check()` and `check_for()` are unchanged, remain the primary spelling used internally throughout this crate, and are not deprecated.

### Fixed

- **An explicit `default_effect: null` in a YAML ACL file was silently normalized to `"deny"`, the same as an omitted key.** `schemas/acl-config.schema.json` declares `default_effect` as a plain `allow` / `deny` string enum, not nullable, so a written `null` is a type violation the operator asked for, not a request for the canonical default — the distinction apcore-python's `_validate_default_effect` already draws by rejecting `None` outright. `ACL::load`'s `raw.get("default_effect")` match folded `None` (key absent) and `Some(Value::Null)` (key present, explicitly `null`) into the same `"deny"` branch; only the absent-key case defaults now, and an explicit `null` falls through to the same `ACLRuleError` rejection as any other non-string value, naming the `null` it was given. `ACL::try_new` / `ACL::new` take `default_effect: impl Into<String>`, which cannot itself be a Rust `null`, so this was file-loading-only — both already reject an invalid string identically, and continue to.

- **`on()` / `on_subscriber()` dispatched through the same `*`-wildcard glob matcher as every other subscriber, instead of the exact-equality match the Contract requires (`docs/features/apcore-client.md` "## Contract: APCore.on").** apcore-python's `on()`-created subscriber compares with `!=` and apcore-typescript's with `===` — neither is a glob. `client.on("apcore.acl.*", handler)` subscribes to the literal event type `"apcore.acl.*"`; in this crate it also, incorrectly, matched `apcore.acl.rule_matched` and any other event type the string glob-matches. Every dispatch loop in `EventEmitter` (`emit`, `emit_sequential`, `emit_filtered`, `emit_spawn`, `emit_delivery_semantics`, and the DLQ recipient loop) now funnels through a new `subscriber_matches`, which checks `EventSubscriber::event_type_filter()` first: `Some(exact)` — the wrapper `on()` installs — is compared by equality; every other subscriber (`WebhookSubscriber`, `A2ASubscriber`, a bare `EventSubscriber` impl, and the DLQ's own `"*"` catch-all, A-D-026) still falls through to `matches_pattern`, unaffected. Narrow blast radius: identical outcome for any `on()` event-type string with no `*` / `?` characters, which is the common case.

- **`retryable` was never resolved from the error code: 18 of the 19 `error_recovery_metadata` fixture cases diverged (apcore#36).** The fixture states that every language SDK MUST produce a default `retryable` and `user_fixable` per framework error code, and calls both part of the cross-language contract. This binding implemented `user_fixable` and not `retryable`, so every framework error the pipeline raised carried `retryable: None`. The new `retryable_for_code` resolves it in `ModuleError::new`, mirroring the existing `user_fixable_for_code`; `with_retryable` remains the explicit override and `circuit_breaker_open()`'s `.with_retryable(true)` is unchanged.

  The peers resolve this per error *class* (`_default_retryable` in Python, `static DEFAULT_RETRYABLE` in TypeScript). This binding has no error-class hierarchy — one `ModuleError` keyed by `ErrorCode` — which is why the conformance driver previously deferred `retryable` to a class-level test "verified elsewhere": that reasoning transfers from Python, where such a test exists, and there was no elsewhere here. `conformance_error_recovery_metadata_defaults` now asserts **both** fields for all 19 cases, which is what makes the fixture binding rather than half-observed.

  What this restores is the distinction the fixture encodes for governance refusals: `APPROVAL_DENIED` is `false` — a human said no, which is terminal — while `APPROVAL_TIMEOUT` is `true`, because nobody answered and a later attempt may succeed. With both `None`, every adapter above this crate had to re-derive that itself or drop it, and at least one got it wrong by assuming a blanket `false` was safe for all refusals (see aiperceivable/apcore-a2a#2).

  Two tests asserted the divergent value and were corrected with the fix rather than treated as a regression: `test_module_error_not_retryable_by_default` (now `test_module_error_retryable_resolved_from_code`) and `test_module_error_creation` in the integration suite, which asserted the *correct* `user_fixable` on the line below.

- **`ACL_DENIED` carried no default `ai_guidance`, and its message rendered an `Option` with `Debug` (apcore#37).** Two defects at one construction site. An authenticated caller was rendered as ``caller 'Some("api.gateway")'`` in a caller-facing message, and the denial carried none of the recovery guidance both peers attach word for word. Both reach an external caller since `apcore-a2a` 0.6, whose `failure_text` appends `ai_guidance` and discloses the message when `disclose_refusal_reason` is on — so with disclosure enabled a Rust-served refusal said less than a Python- or TypeScript-served one, and leaked a Rust-specific `Debug` rendering while doing it. The message is now `Access denied: api.gateway -> cli.rm`, the shape the peers produce, and the guidance is byte-identical to theirs.

  An unauthenticated caller has no id to print, and the peers each print their own language's null spelling here (`None`, `null`), so there is no cross-language wording to match for that branch. The two surfaces now carry it differently, on purpose: `details.caller_id` stays `null` — the raw wire value, which is what the structured contract pins — while the message and guidance name `@external`, the sentinel `ACL::check_access` resolved that absence to before matching any rule and the one this denial's own `AuditEntry` records. That spelling is load-bearing rather than cosmetic: `@external` is matched as an exact literal, no wildcard reaches it, so guidance telling an agent to "verify the caller has the required role" has to name it for the resulting `callers:` rule to fire at all.

  Scope is `ACL_DENIED` only. `ApprovalDeniedError` and `ApprovalTimeoutError` carry no default guidance in any of the three SDKs, so adding it there is a cross-SDK feature request rather than a parity fix.

- **`retryable_for_code` and `user_fixable_for_code` disagreed about `INVALID_PARENT_ID`.** The `user_fixable` policy deliberately carries one code beyond the fixture; the new `retryable` policy now carries the same one, with the value apcore-python pins on the class. A malformed `parent_id` is fixed by sending a valid one, never by retrying the same call.

- **The `retryable` policy covered 12 of PROTOCOL_SPEC §8.6's 38 classification rows.** The fix above was scoped to the codes `error_recovery_metadata.json` pins, on the reasoning that a value invented on one side of a three-SDK contract is the divergence the policy exists to prevent. That reasoning does not survive reading §8.6: the spec carries a normative table for 38 codes and says implementations **SHOULD** use it as the default, and §9.12.4 goes further with a categorical **MUST** — *"All config errors ... are non-retryable (`retryable = false`)"*. The table was already the authority; the fixture only pins an executable subset of it. Transcribing the remaining rows is not invention.

  Now resolved: the 23 remaining §8.6 `No` rows, `RELOAD_FAILED` as §8.6 `Yes`, and the whole `CONFIG_*` family under §9.12.4 — including `CONFIG_KEY_RESTRICTED`, which is a config error the §9.12.4 table does not list, so a hand-kept list would have been short by one. 31 codes in total, every one of which agrees with both apcore-python and apcore-typescript where those pin a value; there are **no** codes where this crate now contradicts a peer.

  The visible effect is on the wire rather than in the pipeline. `ModuleError.retryable` is `skip_serializing_if = "Option::is_none"`, so an unset code omitted the field entirely while the peers emitted `"retryable": false` — a consumer testing `err.retryable === false` got `undefined` from a Rust-served error and `false` from the other two. In-process, `RetryMiddleware` gates on `retryable == Some(true)`, so `None` and `Some(false)` were already indistinguishable to it and none of the 30 `false` codes changes any behaviour. `RELOAD_FAILED` is the one code that becomes retryable; `Config::reload` is not module execution, so it reaches the retry gate only if a module calls it during `execute` and propagates the error.

  Two §8.6 rows are deliberately **not** carried, and both are pinned by a test so the omission reads as a decision. `MODULE_EXECUTE_ERROR` is §8.6 *"Depends"* — on the module's `annotations.idempotent` — which is exactly what `None` already means here. `EXECUTION_CANCELLED` is §8.6 *"Yes"*, and is the one row that would change behaviour rather than metadata: `CancelToken::check` raises it from **inside** module execution, so unlike `APPROVAL_TIMEOUT` it does reach `RetryMiddleware`, and `Some(true)` would auto-retry a call the caller had just explicitly cancelled. Neither peer implements the code at all, so there is no parity pressure and the question belongs upstream.

  `CIRCUIT_BREAKER_OPEN` and `TASK_LIMIT_EXCEEDED` stay out for a different reason: §8.6 does not list them, and the two peers **disagree** about the first — apcore-python pins `True`, apcore-typescript pins `false` while its own `aiGuidance` on the same class tells the caller to *"wait for the recovery window to elapse before retrying"*. `ModuleError::circuit_breaker_open()` keeps its explicit `.with_retryable(true)`, siding with Python and with TypeScript's own guidance text.

  `test_retryable_for_code_transcribes_the_spec_8_6_table` transcribes all 38 rows row for row — the table is prose in the spec repo, so nothing mechanical kept the map honest before. `test_every_config_error_code_is_non_retryable` sweeps `ErrorCode::ALL` rather than a list, so a config code added later fails until its value is pinned.

---

## [0.28.0] - 2026-08-31

### Added

- **Argument-scoped approval — the ACL can now *ask* about a call's arguments, not only refuse (spec v1.28.0 §6.1.6-§6.1.8 / §6.3.1 / §6.8.1 / §6.9 / §7.4 / §7.9.5, apcore#108).** Every decision point that could read a call's arguments was unable to escalate it to a human, and the one point that decides whether to ask a human is forbidden by §7.9.6 rule 2 from consulting them. The ACL could **refuse** on arguments and an `ApprovalHandler` could **wave through** on arguments; nothing could **ask**, and a refusal is not a question — so an operator who needed `git push --force` reviewed had to gate every `git push`, which dilutes `requires_approval` from "this needs approval" to "this might" and floods the audit trail. New public surface:

  - **`ACLRule.approval: Option<ApprovalRequirement>`** (§6.1.6). Authorization and approval are two independent results, and folding them into one enumeration makes "denied **and** needs approval" representable while forcing "allowed **but** ask first" to be spelled as a kind of denial. `effect` keeps authorization; this orthogonal field carries the requirement. `None` is the absent key and means `not_required`, so every rule written before v1.28.0 keeps its meaning exactly. `approval: required` on a `deny` rule is **rejected** by `ACL::load`, `ACL::try_new` and `ACL::try_add_rule` with `ACLRuleError`: the combination has no meaning, and silently ignoring one half of a governance rule is the failure mode §6.1.5 was written to end. `add_rule` stays infallible in signature and panics (see the `ACL::try_add_rule` entry below) — and `ACLRule::approval_required()` refuses to report a requirement off a `deny` rule regardless, so the meaningless state cannot reach a decision by any route. `approval` joins the closed `RULE_KEYS` set; without that every rule carrying it would have been refused as an unknown key by the v1.27.0 closure.

  - **The built-in `arguments` condition** (§6.1.7) — `has_key` (any named key present), `has_all_keys` (every one), `has_none_of` (none). Added to the condition language §6.1 already defines rather than beside it, and seeded through `seed_condition` with the other built-ins: there is no registration point of its own, because a deployment-registered *argument* handler would be exactly the unauditable host code §7.9.6 rule 2 exists to keep out of a governance verdict, and a fixed vocabulary keeps the decision reproducible from the ACL document. Being built-in also means §6.1.4's precheck covers the key for free — `argument:` written for `arguments:` is an unregistered condition key, so the rule is unevaluable rather than silently inert. **No predicate reads a value**, and the type system enforces it rather than a convention: the handler reads a `GovernanceProjection`, which has no field that can hold one.

  - **`GovernanceProjection` and `JsonType`** (§6.1.8). The key set plus each key's JSON type, computed by `module_lookup` (Step 3) and read by `acl_check` (Step 4) — an ordering §6.1.8 rule 1 makes normative rather than an implementation detail that happens to hold. Deliberately **not** `context.redacted_inputs` (rule 3): that field's contract is safe *logging*, and it is a raw copy of the arguments when the module declares no input schema, so it is not even safe here. `_approval_token` is excluded, on the same grounds §7.9.6 rule 5 strips it before policy resolution — it is protocol machinery rather than caller input, and including it would make a Phase B resume present a different argument shape to governance than the original call.

  - **`AccessDecision`, `ACL::check_access` and `ACL::async_check_access`** (§6.8.1). The structured result — `access` / `approval_required` / `matched_rule_index` / `reason` — alongside the booleans, which are kept. Provided on **both** paths because `check()` and `async_check()` resolve conditions from different registries (§6.1.3), so offering it on one would leave the other unable to see an approval requirement at all.

  - **`ACL::try_add_rule`** — the fallible form of `add_rule`, mirroring the `new` / `try_new` pairing. All three entry points that accept a rule (`load`, `try_new`, `try_add_rule`) refuse `approval: required` on a `deny` effect: a rule built in code is as meaningless as one parsed from YAML. `add_rule` stays infallible in signature and now panics on that one input, which cannot break a caller written before v1.28.0 because the field did not exist and the state was not constructible.

  - **`AuditEntry.approval_required`** (§6.3.1) — beside `decision`, never a third `decision` value: `decision` is a string downstream consumers parse and a third value breaks every existing parser (§6.9 row 7). `#[serde(default)]`, so a record written before v1.28.0 still deserializes.

  The projection **cannot be forged** (§6.1.8 rule 3). `GovernanceProjection` derives no serde traits, so it cannot be built from a wire payload at all; it rides `PipelineContext`, which is framework-internal and never deserialized; and `Context` — the object that *is* deserialized from untrusted input — does not carry it, which is one of the reasons it did not go there. A caller that could supply its own projection would satisfy `has_none_of` for a call whose arguments say otherwise, turning the condition into a caller-controlled switch. Source guards in `src/acl.rs` pin all three, so a later change that adds `Deserialize` or moves the field onto `Context` fails a test rather than opening the hole quietly. How the projection reaches the condition is left idiomatic by §6.1.8 rule 4; this SDK passes it as an argument to `check_access` and carries it down to the handler on the same per-call scope that already carries `handler_error`, because `ACLConditionHandler::evaluate(value, ctx)` has nowhere else to take it and widening that trait would hand the projection to every deployment-registered handler.

  Wired end to end. Step 5's gate fires on the **union** of the module's `annotations.requires_approval`, the Step-4 ACL decision, and `gate_destructive` (§7.4, §6.9 rows 3-5), and the `ApprovalRequest` handed to the handler carries `requires_approval: true` for an ACL-sourced requirement as §7.4 rule 3 requires. `Executor::validate()` reports the **governance-effective** requirement — the same union — because reporting only the policy-effective value would tell a caller no approval is needed for a call the gate will stop (§7.9.5).

- **`ConditionOutcome` and three-valued ACL condition evaluation (spec v1.22.0 / v1.25.0 §6.1.1, apcore#100).** Condition evaluation returned a plain `bool`, so "a handler answered no" and "no answer was obtainable" reached the rule loop identically and both meant *this rule does not match*. That is safe in one direction only: an `allow` rule that cannot evaluate its condition does not grant, but a `deny` rule that cannot evaluate its condition does not block — evaluation continued to the next rule and then to `default_effect`. See the BREAKING entry below for the behaviour change; the type is listed here because it is the new public surface. `ConditionOutcome::{and, or, negate}` carry §6.1.1's Kleene composition table, and `$not` of `Unevaluable` is `Unevaluable`, never `Satisfied` — negating "no answer" into "yes" would let a misspelled key inside a `$not` satisfy the rule it was meant to gate.

- **`ACLConditionHandler::evaluate_outcome` — a provided trait method.** Defaults to mapping `evaluate` onto `Satisfied` / `Unsatisfied`, which is correct for every leaf handler; only the built-in `$or` / `$not` override it, because a compound operator has to *propagate* an unevaluable sub-condition rather than collapse it. Additive with a default body, so a custom handler written before this release compiles and behaves unchanged. Handlers still cannot report "unevaluable" themselves, and do not need to: §6.1.1's three situations are all detected by the evaluator around the handler.

- **`ACL::validate_rules() -> Vec<RuleValidationFinding>` (spec v1.22.0 §6.1.2 rule 3, §6.1.3, §6.1.4, apcore#100).** The explicit validation entry point §6.1.2 makes a MUST: it reports every rule that fails §6.1.4's precheck — an unresolvable condition key, a compound operator whose operand has the wrong shape, or a `conditions` that is not a mapping — including faults nested inside `$or` / `$not`, so a deployment can assert on the result once handler registration is complete. Pure read: no mutation, no registration, no audit event. Findings are ordered by rule index, then lexicographically by condition **path**, and each carries `rule_index`, `condition_path`, `condition_key` (`None` where there is no key to name), `effect`, and **two** resolvability flags. `sync_resolvable` and `async_resolvable` are deliberately not collapsed into one boolean: `async_check()` consults the async registry and falls back to the sync one while `check()` consults only the sync one, so a key registered *only* as an async handler is a working condition on one path and an unevaluable one on the other, and a finding IS emitted for it (§6.1.3 rule 3).

  Named `validate_rules` rather than `validate_conditions` because it reports structural faults in the rule as a whole. §6.1.4.1's malformed `callers` / `targets` cannot arise here at all: both are `Vec<String>`, so serde rejects a bare string at deserialization and the compiler rejects one in a struct literal. §6.1.4.1 states that an implementation whose type system makes the value unrepresentable "satisfies this clause by construction and needs no runtime check", so there is deliberately no check for a state that cannot exist. That immunity matters — the same typo in apcore-python iterates the string character by character, and the `*` character matches everything, so `callers: "admin.*"` on an `allow` rule granted every caller (apcore#106).

- **`ACL::default_effect() -> &str` (spec v1.23.0 §6.8, apcore#101).** The single most consequential value in an ACL had no public reader — the field was non-`pub` and `rules()` was the only accessor — so tooling that reports or audits the enforced policy had to re-read and re-parse the ACL file to recover a value the loaded object already held. A pure read of the live object, so it reflects a `reload()`. `rules()` already satisfied §6.8: it hands out an immutable slice, through which the ACL's own list cannot be reordered or mutated.

- **`ExecutionPolicy::resolve_with_call_site(module_id, annotations, arguments, context)` (spec v1.24.0 / v1.25.0 §7.9.6, apcore#102).** Policy resolution now *receives* the call site. Added as a second method rather than by widening `resolve`: §7.9.6 rule 7 leaves the API shape to the language and then states the constraint — because rule 6 forbids the new inputs from changing any verdict, widening a published signature buys nothing and costs every external caller a compile error, so "a language without default arguments or overloading **SHOULD** add a method rather than widen". **The built-in pattern rules MUST NOT consult the call site**, and do not: `resolve_with_call_site` records the call site's *shape* — which keys were present and their types, never argument values (rule 3) — on the trace span and forwards to `resolve`, so a rule set's verdict stays a function of module ID and annotations alone and remains reproducible from the policy document. A differential test asserts bit-for-bit equality across a matrix of policies, modules, annotations and call sites. `arguments` here have **not** been schema-validated: the approval gate is Step 5 and input validation is Step 7, so a reader must treat them as untrusted. Wired at both call sites — the Step-5 gate in `builtin_steps.rs` and `Executor::validate`'s preflight, so the two cannot report different verdicts.

  §7.9.6 rule 3(b) — "a host-supplied policy implementation can decide on arguments" — was **withdrawn in v1.25.0** after this SDK reported that no SDK can satisfy it: `ExecutionPolicy` is a concrete type in all three and `set_policy` takes that concrete type, so there is no policy interface for a host to implement. Making it pluggable is deliberately not built here: no requirement has asked for it, and inventing the extension point ahead of a use for it is how the deferred argument predicate would arrive by the back door.

- **`_approval_token` is stripped from `arguments` before policy resolution (spec v1.25.0 §7.9.6 rule 5).** §7.4 already required the framework-owned Phase B token to be removed "before passing to subsequent steps", which does not reach this case: resolution happens *inside* Step 5, ahead of any subsequent step, so the gate satisfied §7.4 literally and still handed the token to the policy — verified in `builtin_steps.rs`, which resolved the policy near the top of `execute()` and stripped the token roughly 120 lines later. It is a protocol-level key, not caller input; leaving it in place puts a token into the audit trail and the `apcore.policy.override` payload. The strip happens at the policy API boundary rather than at each call site, so no present or future caller can forget it, and it costs one shallow clone only when the token is actually present — a Phase B resume, not the common path. The caller's own arguments are not mutated, so the gate still finds its token further down Step 5.

- **PROTOCOL_SPEC §6.1.4's structural and registry precheck (spec v1.25.0, apcore#100).** A context-free, handler-free walk of a rule's whole `conditions` tree — every branch inside `$or` and `$not` — that runs before any handler and, crucially, **before §6.5's no-context check**. Two requirements pull against each other otherwise: §6.1.1 rule 2 wants `handler_error` deterministic across SDKs, while the composition rules permit short-circuiting, which makes which failures are even *reached* implementation-defined. Splitting the passes resolves it — the precheck is exhaustive and MUST NOT short-circuit, so its findings are a pure function of the rule and identical in every SDK, while handler *execution* may still short-circuit and its diagnostics carry no such guarantee.

  The ordering closes a bypass that was live in all three SDKs: `conditions: {mispelled: true}` on a `deny` rule returned `true` for a context-less caller, because §6.5 short-circuited on the missing context before §6.1.1 ever ran. A rule that **fails** the precheck is now unevaluable whether or not a context was supplied. A rule that **passes** it and then finds no context still takes §6.5's path and does not match — `roles` is not unevaluable merely because this caller sent no identity. The line is between a question this caller did not supply the input for and a question nobody can answer, and it is deliberately sharp.

  **A precheck fault gates the rule; it does not enter the composition table** (§6.1.4 rule 5). §6.1.1's table says an outright "yes" wins an `$or` even when a sibling is unevaluable — but that governs conditions the implementation actually *evaluated*. A fault the precheck found is different in kind: the rule is not well-formed enough to evaluate, so it is unevaluable as a whole and no sibling rescues it. The discriminating case is `{"$or": [{mispelled: true}, {"roles": ["dev"]}]}` for a caller who **has** `dev`: composition alone would make the `$or` SATISFIED, and gating makes the rule unevaluable. Gating is correct — otherwise a typo stays silent for exactly as long as some sibling keeps matching, which is the failure mode §6.1.1 exists to end, one nesting level down. It is also required for consistency with the ordering rule above, since a context-less call evaluates no condition at all and the table has nothing to operate on.

  The split is **precheck versus execution**, not structural versus everything. A *registered* handler that throws is execution-origin, so the table does apply: the same shape with a throwing key and a `dev` caller is SATISFIED, and an `allow` rule carrying it **grants**. Both directions are pinned by fixture cases.

  **The precheck must not enlarge a rule's reach** (§6.1.4 rule 4). Pattern matching comes first: a rule that a well-formed `callers` / `targets` definitively excludes from this call does not participate in the decision, and its `conditions` faults are neither consulted nor reported. Without that, one misspelled key in a narrowly scoped rule would decide calls it was never written about — `callers: ["api.*"]` with a typo'd condition and `effect: deny` would deny a `worker.*` caller, breaking first-match-wins. The fault is still real: `validate_rules()` reports every fault in every rule regardless of any call, and that is where a scoped rule's typo is meant to surface.

- **Condition paths (spec v1.25.0 §6.1.4).** `handler_error` and validator findings name a *position*, not a key: `k`, `$or[i].k`, `$not.k`, `$` for the `conditions` object itself, and `$or[i]` for an element that is not a condition object. Paths nest — `$or[1].$not.k`. An `$or` element that is not an object is a **fault at its own position, never a silently skipped branch**: skipping is how `$or: ["typo"]` on a `deny` rule stays inert, the same defect one level down. A malformed operator **value** (`$or: "nope"`, `$not: 3`) keeps its key, since the fault is attached to it; a malformed **element** (`$or: ["typo"]`) is keyless, as are a non-mapping `conditions` and a malformed pattern field. Every structural fault reports **both resolvability flags false** regardless (§6.1.3 rule 3), because the flags describe the *fault* — "can this be resolved by evaluating on that path" — and not the key's registration; the latter would report a malformed `$or` value as resolvable on both paths, i.e. would say a malformed rule is fine. A **registry** fault, by contrast, reports the key's real resolvability, which is where the two flags differ for an async-only handler. Ordering moved from by-key to by-path because a key may occur at several positions in a nested tree, which left by-key ordering undefined. The compound operators carry a path prefix across their recursion (through the per-call capture scope, which travels with the tokio task rather than the thread), so an execution-origin failure two levels down still reports its full path where the public `evaluate_outcome(value, ctx)` signature has nowhere to carry one.

- **`CallbackApprovalHandler::new_sync` (apcore#104).** The in-process, no-I/O constructor. Returns `Result<ApprovalResult, ModuleError>`, like `new`.

- **`ApprovalResult::approved(by)`, `ApprovalResult::rejected(reason)`, `Change::new(action, target, summary)`, `PreviewResult::new(changes)` (`api-surface-conventions.md` §9.2 rules 2 and 4, apcore#103).** Semantic constructors for `#[non_exhaustive]` types carrying required semantic fields. `ApprovalResult::default()` leaves `status` as the **empty string**, which is not a decision; `Change`'s `action` / `target` / `summary` default to three empty strings where the preview RFC requires all three. A default-constructed value that is structurally valid and semantically empty is what §9.2 rule 4 exists to prevent. `AutoApproveHandler`, `AlwaysDenyHandler` and `CallbackApprovalHandler::check_approval` now build their results through these.

- **`Executor::governance_state()` (spec v1.16.0 §6.6.5, apcore#97).** A read-only accessor returning a `GovernanceState` of eight observations plus one derived flag: what is *configured* on this executor versus what is actually *wired* into the running pipeline. apcore-rust already exposed `acl`, `approval_handler` and `policy` as public struct fields — raw state with no defined semantics, which is exactly what an adapter would read to conclude "protected" from `acl.is_some()`. The gates are pipeline steps, and the `internal`, `testing` and `minimal` presets remove them, so an executor can hold an ACL that no step consults. **The existing public fields are unchanged**; this answers a different question.

- **`Step::builtin_gate() -> Option<BuiltinGate>` and the `BuiltinGate` enum.** The capability marker PROTOCOL_SPEC 6.6.5.2 requires: gate detection **MUST NOT** be by step name. It defaults to `None`, and only `BuiltinACLCheck` and `BuiltinApprovalGate` override it, so a custom step calling itself `acl_check` cannot be mistaken for the gate. Reporting a gate that is not present is the one direction the accessor must never fail in. Additive with a default body — existing `Step` implementations are unaffected.

- **`UsageCollector::get_module_summary_for_period`, `get_p99_latency_ms_for_period`, `get_caller_breakdown_for_period`, `get_hourly_distribution_for_period`.** Period-aware forms of the four accessors the detail usage module needs; none of the originals took a period.

### Changed

- **BREAKING (security): `ACL::check` and `ACL::async_check` return `false` for allow-with-approval-required (spec v1.28.0 §6.8.1, apcore#108).** A rule resolving to `allow` with `approval: required` makes the legacy booleans fail closed. `check()` is public API consumed by callers that are not the Executor — tooling, preflight helpers, third-party integrations — and such a caller can only read a boolean as "let it through / do not"; returning `true` would let it execute a call the ACL said needed a human. `false` is wrong in the benign direction: the caller sees a refusal where the truth was "ask first". The Executor consults `check_access` and is unaffected, and a legacy caller only meets this at all once an operator has authored a rule carrying `approval`.

- **BREAKING: `ACLRule` gains a field, and `AuditEntry` gains a field and `#[non_exhaustive]` (spec v1.28.0 §6.1.6 / §6.3.1, apcore#108).** `ACLRule { .. }` literals need `approval: None` (or `Some(ApprovalRequirement::Required)`); it is not marked `#[non_exhaustive]`, because forcing `Default` + mutation on a config-shaped struct that deployments construct directly costs more than the next field it would save. `AuditEntry` is the other case: it is produced by the ACL and consumed by audit loggers, so it takes the crate's standard remedy (apcore#24) — `#[non_exhaustive]` plus `Default`, constructed by mutation — which makes the *next* spec-driven field non-breaking.

- **BREAKING (security): a structurally broken `arguments` predicate is UNEVALUABLE, not false (spec v1.28.0 §6.1.7 / §6.1.8, apcore#108).** Four shapes resolve to `Unevaluable` under §6.1.1's principle, each because "false" would be safe on a `deny` rule and **fail open on an `allow` rule**: no projection available (an `allow` rule must not grant for a call whose arguments were never seen — `has_none_of` over an empty stand-in is *satisfied*); `arguments: {}`, an empty predicate object, for the reason §6.1's table already gives for an empty `$not`; an unrecognised predicate name, since the vocabulary is closed exactly as the rule key set is; and a predicate value that is not a list of key names (§6.1.1 case 4). Empty predicate *arrays* are well-formed and vacuous — `has_key: []` unsatisfied, `has_all_keys: []` and `has_none_of: []` satisfied. Cases 2-4 are decidable without a context and without running a handler, so §6.1.4's **precheck** reports them and `validate_rules()` surfaces them at deploy time rather than at the first call that trips one.

- **BREAKING (security): an `ExecutionPolicy` override cannot clear an approval requirement the ACL set (spec v1.28.0 §6.9 row 4, apcore#108).** The Step-5 gate ORs the ACL's requirement onto the policy-effective verdict rather than letting the policy decide alone. The ACL is a **caller-scoped** authorization layer and `ExecutionPolicy` is a **module-scoped** platform override, so letting the module-scoped one cancel the caller-scoped one is a privilege escalation: a policy rule written for `orders.*` with `requires_approval: false` would silently strip a requirement an ACL author attached to one untrusted caller. A policy may still **add** a requirement — union is the only safe composition, and `Executor::validate()` reports the same union so preflight and the gate cannot disagree.

- **BREAKING (security): an unevaluable ACL condition no longer silently disables a `deny` rule (spec v1.22.0 / v1.25.0 §6.1.1 / §6.3 / §6.3.1 / §6.5, apcore#100).** A condition is **unevaluable** when the implementation cannot answer it *as written* — a principle, not a closed list. The situations every implementation meets: the key has no resolvable handler, the handler panicked, the handler was asynchronous and not ready on the synchronous `check()` path, **the value is malformed for its key** (`$or` that is not a list, `$not` that is not an object), or **`conditions` itself is not a mapping**. All of them previously produced a plain `false`. A `deny` rule carrying a single misspelled key (`role:` for `roles:`) was therefore decoration: the rule did not match, evaluation fell through to the next rule and then to `default_effect`, and with `default_effect: allow` the call went through. §6.1.1 now requires an unevaluable condition to resolve the rule **toward refusing access**: a `deny` rule takes effect and the call is denied; an `allow` rule still does not match and still MUST NOT grant, so the permissive direction is unchanged.

  Applied on **both** paths — `matches_rule` and `matches_rule_async` are separate code, and the async-only-handler-on-the-sync-path case was specified as "unsatisfied" through spec v1.21.0, which left a `deny` rule guarded by such a handler inert by a second route. The malformed-operand cases are v1.25.0's addition: v1.22.0 enumerated exactly three situations and this SDK read that list as exhaustive, so `$or: "typo"` and a non-mapping `conditions` both returned `Unsatisfied` — a `deny` rule carrying either went inert, which is the v1.22.0 defect reached through a second door. Both now resolve per §6.1.1's effect table. Deliberately **not** changed: §6.5's "conditions present but no context provided" stays an ordinary non-match *for a rule that passes the precheck*, because calling with no context is a legitimate shape for external entry points, not a misconfiguration; treating it as a failure would flip the decision for every `@external` call meeting a well-formed conditional `deny` rule.

  `conformance/fixtures/acl_handler_error.json` still pins the OLD expectation. The corrected 12-case fixture is staged in the spec repo at `planning/acl-unevaluable-conditions/staged-fixtures/` and lands once all three SDK drivers do, so CI does not go red across every repository during the rollout. `tests/test_acl_handler_error_conformance.rs` now reads the staged copy in preference to the canonical one and needs no change when it is promoted. **13 of the 15 cases execute and pass; 2 are skipped**, both `skip_if_unrepresentable`: `string_callers_on_allow_rule_must_not_grant` and `scalar_callers_on_deny_rule_denies_without_raising` supply a non-list `callers`, which `Vec<String>` makes impossible to construct. The driver asserts that it skips *only* cases the fixture sanctions and that it skips *all* of them, so the immunity cannot silently become a gap.

- **BREAKING: `ACL::evaluate_conditions` returns `ConditionOutcome`, not `bool`.** §6.3 makes carrying the distinction a MUST — "an implementation whose evaluator returns a plain boolean MUST carry the distinction some other way (a sentinel, an out-parameter, a typed result)". A typed result is the option that cannot be ignored at the call site. `ACL::evaluate_conditions_async` keeps its `bool` return as a façade for callers that only want "was it satisfied"; `ACL::evaluate_conditions_async_outcome` and `acl_handlers::evaluate_conditions_async_outcome` are the three-valued forms. Both evaluators evaluate **every** child rather than short-circuiting. Spec v1.25.0 no longer needs this for determinism — §6.1.4's precheck is where the cross-SDK guarantee now lives, and it is required to be exhaustive — but full traversal stays conforming and keeps execution-origin diagnostics complete too. The decision is identical either way, since an outright `Unsatisfied` still wins a conjunction.

- **BREAKING: `CallbackApprovalHandler::new` takes an async, fallible callback (apcore#104).** It took `impl Fn(&ApprovalRequest) -> ApprovalResult` — synchronous and infallible — while apcore-python takes `Callable[[ApprovalRequest], Coroutine[…, ApprovalResult]]` and apcore-typescript takes `(request) => Promise<ApprovalResult>`. The same identifier meant two different things across three SDKs, which is how a documented guide came to pass an async closure to a constructor that never accepted one. Two capabilities were missing, not one: an approval decision that performs I/O (a Slack round-trip) did not fit the convenience handler at all, and — the worse half, because it is silent — a decision that *failed* could not be expressed, so an author whose approval service was down chose between panicking inside the approval gate and returning a `"rejected"` result that reads in the audit log exactly like a human saying no.

  `new` is now `Fn(ApprovalRequest) -> Fut` where `Fut: Future<Output = Result<ApprovalResult, ModuleError>> + Send`; the old shape moves to `new_sync`, which also gains a `Result`. Both erase into one boxed storage form, so the struct stays non-generic and `Box::new(handler)` still coerces to `Box<dyn ApprovalHandler>`. Taking the request by value is safe: `Context.data`, `cancel_token` and `executor` all sit behind an `Arc`, so the clone shares them and the spec's by-reference `data` semantics survive; what is deep-copied is `arguments`, `identity` and `call_chain`, the read-only inputs to an approval decision. Migration is one word — `new` → `new_sync` — plus wrapping the returned value in `Ok(...)`. A grep across every Rust crate in the workspace found **zero** call sites outside this repository's own tests and example.

- **`ACL::load`, `ACL::try_new` / `ACL::new` and `ACL::add_rule` warn for every precheck fault (spec v1.22.0 §6.1.2, apcore#100).** Each entry point that accepts rules now runs §6.1.4's precheck over the whole `conditions` tree — including branches nested inside `$or` / `$not` — and warns for each fault, naming the rule index, the condition path, the key and the rule's `effect`. The `effect` is in the message because a misconfigured `deny` rule is the consequential case. `add_rule` performed no validation at all before this; §6.1.2 rule 4 makes runtime insertion an entry point that MUST be covered, not just file loading. Loading and construction still **succeed**: `register_condition()` writes to a runtime, process-wide registry and `acl.root` discovery commonly runs during framework bootstrap ahead of application code, so failing here would reject valid configurations on ordering alone.

- **`AuditEntry.handler_error` reports every unevaluable condition, ordered lexicographically by condition **path** (spec v1.22.0 / v1.25.0 §6.1.1 rule 2, §6.3.1).** The per-call slot held one message and the last writer won. It now accumulates into a `BTreeMap` keyed by condition path and joins the messages with `"; "`. Path order rather than evaluation order because the two are not the same across languages — `serde_json`'s map is ordered while Python `dict` and JavaScript objects preserve insertion order, so "the first one encountered" would write a different entry into the audit log for the same rule in different SDKs. By path rather than by key because a nested `$or` may carry the same key at two positions, which leaves key ordering undefined. `report_handler_error` keeps its free-form signature and derives the sort key from the conventional `"{path}: {reason}"` prefix; SDK-internal call sites pass the position explicitly.

- **The twelve `#[non_exhaustive]` doc comments described a construction form that does not compile (apcore#103).** All of them read "so future spec extensions can add fields **without breaking downstream struct-literal construction**. Construct via `..Default::default()` or a builder pattern." Three things were wrong: `..Default::default()` is itself a struct expression and fails cross-crate with `error[E0639]`; the causality is backwards, since the attribute exists precisely **to** break downstream struct literals and that is the mechanism by which adding a field stays non-breaking; and none of the twelve types has a builder, which `api-surface-conventions.md` §9.2 rule 3 forbids citing. Rewritten across `approval.rs`, `module.rs`, `async_task.rs`, `config.rs` and `middleware/retry.rs`, and `Change` / `PreviewResult` — which carried no construction guidance at all — gained constructors instead. No behaviour change; every affected type verified to have a `Default`.

- **BREAKING: `system.usage.*` honours `period` (spec v1.14.0 §6.7.1.1, apcore#96).** Both modules read `period`, echoed it back, and then computed every statistic over the full retained history — `get_all_summaries()` for the summary; `get_module_summary` / `get_p99_latency_ms` / `get_caller_breakdown` / `get_hourly_distribution` for the detail module, not one of which took a period. Silent by construction: the response named a window it had not applied. `call_count`, `error_count`, `avg_latency_ms`, `p99_latency_ms`, `trend`, `callers` and `hourly_distribution` are now all computed over `[now − period, now]`.

- **BREAKING: `hourly_distribution[].hour` is `YYYY-MM-DDTHH` (spec v1.14.0 §6.7.1.2, apcore#96).** The `:00:00Z` suffix was appended in **two** places: `HOUR_KEY_FORMAT` in the sys-module layer, and `format!("{hour}:00:00Z")` inside `UsageCollector::get_hourly_distribution`. `HOUR_KEY_FORMAT` was documented as *"matching `UsageCollector` bucket hours"* while `bucket_key` produced `%Y-%m-%dT%H` — so the comment asserted an alignment that did not hold, and apcore-rust was internally inconsistent rather than merely different from the other two SDKs. The existing padding test asserted the entry count and the totals but never the key format, which is why this survived.

- **`output_schema()` declares the full field contract for both usage modules (spec v1.14.0 §6.7.1.6, apcore#96).** Both returned a bare `{"type": "object"}`, which satisfies §6.7's "equivalent output schemas" only in the sense that any two such declarations are equivalent to each other. `input_schema` also gains `"pattern": "^[1-9][0-9]*[hd]$"` on `period`, so a malformed value is rejected at input validation with `SCHEMA_VALIDATION_ERROR` rather than being silently ignored — apcore-rust previously parsed no period at all, so every spelling was accepted.


- **`tests/test_acl_argument_scoped_approval_conformance.rs` runs every case twice.** The fixture grew a second column (24 cases, up from 20): run 1 supplies a governance projection derived from `arguments`, run 2 supplies none at all, and both are contracts — §6.1.8 case 1 makes `check()` a public entry point that may be called without one. Both **structured** columns are asserted here via `check_access(.., Some(&projection))` and `check_access(.., None)`; `ACL::check` takes no projection in this SDK and so answers run 2's question, which is now asserted on all 24 cases against `expected_legacy_check_no_projection`. This replaces the driver-side skip that had left 17 of 20 cases unverified — the column the drivers were skipping is exactly where apcore#109 was sitting. `expected_legacy_check` (run 1's boolean, for an SDK whose `check()` does take a projection) is the one key this driver cannot assert, and says so where the assertion would have been.
### Removed

- **`UsageCollector::get_caller_breakdown` and `get_hourly_distribution` (`pub(crate)`, internal).** Replaced by their `_for_period` forms. Keeping a period-ignoring accessor beside a period-aware one is how the divergence above happened; there is now one way to read each statistic.

### Fixed

- **`requires_approval` was documented as if it answered "will this call need a human" (spec v1.29.0, [apcore#110](https://github.com/aiperceivable/apcore/issues/110)).** `ModuleAnnotations.requires_approval` carried no doc comment and `PreflightResult.requires_approval` read *"True if the module has `requires_approval` annotation"* — the latter is simply wrong, and has been since spec v1.28.0: `Executor.validate()` reports the **governance-effective** union (§7.9.5, §6.9 rows 3-5), so a module declaring `requires_approval: false` correctly reports `True` there when the ACL requires a human for the arguments this call carries. The code was right; the docstring described the behaviour it replaced.

  Both docstrings now say which question they answer: the annotation describes the **module** and `false` does not mean no approval will be required; the preflight result describes the **call** and is the same verdict the approval gate will enforce. Documentation only — no behaviour change.

- **SECURITY: a rule's `effect` was a closed set at two of three doors — `add_rule` accepted any string (spec v1.30.0 §6.1.5, §6.1.6 rule 3, [apcore#111](https://github.com/aiperceivable/apcore/issues/111)).** `effect` accepts `allow` and `deny` and nothing else — §6.1's field table has always said **MUST** and `schemas/acl-config.schema.json` has always declared the enum — and `ACL::load` and `ACL::try_new` did reject an out-of-enum value. `ACL::try_add_rule` did not: it checked only §6.1.6's `deny` + `approval: required` combination, so a rule this SDK refused to *load* was accepted at runtime insertion. This repo was reported as the one SDK conforming at all three entry points; it was conforming at two.

  **What the accepted rule then did.** Nothing rejected the value further down, so `finalize_rule_match` copied it verbatim into `AccessDecision::access` and into the audit entry's `decision` field. `effect: "Allow"` produced `access: "Allow"` — a decision string no consumer parses, from a rule the operator wrote to *permit*. Every downstream reader compares against `"allow"`, so the call was refused; the operator got a denial with no error and no warning, from a rule they wrote and the loader would have told them was wrong. On a `deny` rule the same fallback is only ever *accidentally* right, which §6.1.5 makes the point of: it is correct until someone revisits which way it points. `ACLRule::approval_required()` also reads `effect == "allow"` literally, so a mis-cased `allow` rule silently dropped its `approval: required` as well.

  **Fixed by giving the three doors one check instead of two copies.** `ACL::validate_rule(index, rule)` now holds both rule-level rejections — the `effect` enum and §6.1.6 rule 2's `deny` + `approval` combination — and `load`, `try_new` and `try_add_rule` all route through it, so they cannot drift apart again the way they had. The refusal names the rule index and the offending value at every door; for a runtime insertion the index is `0`, the position `add_rule` puts the rule in. `default_effect` was already validated at every door it reaches and now shares the same `EFFECTS` constant rather than a second pair of string literals.

  **The infallible entry points refuse too.** `ACL::new` and `ACL::add_rule` are infallible in signature and signal an unconstructable value the way Rust does — by panicking — which is what §6.1.6 rule 3 asks of an SDK whose insertion API returns nothing. Both now carry the underlying `ACLRuleError` message into the panic instead of a fixed `.expect()` string: the old one said *"invalid default_effect"* for every rejection, including rule-level ones that have nothing to do with that field, and named neither the rule index nor the offending value that §6.1.5 requires the refusal to surface.

  **Pinned by `conformance/fixtures/acl_effect_value_closure.json`** (10 cases). The fixture's substance is its `entry_points` list rather than a per-door expectation — a value legal through one door and illegal through another IS the defect, so the shape that could express it was left out deliberately. The driver exercises every listed door, and both halves of each fallible/infallible pair: `try_new` beside `new` and `try_add_rule` beside `add_rule`, the panicking halves observed through `catch_unwind`, because asserting only on the `Result` forms would have proved nothing about the two entry points a caller reaches for first. It also reconciles the fixture against the doors this SDK exposes, so a fixture that stopped listing one would fail rather than quietly stop testing it.

  **Not a breaking change for any working configuration.** An out-of-enum `effect` was never a rule that did what it said; it was a rule whose value travelled uninspected into the decision. A caller that inserted one at runtime was already having its calls refused.

- **SECURITY: an unevaluable approval rule stepped aside and the call was granted without approval (spec v1.29.0 §6.1.1 rule 5, §6.8.1, §6.9 rows 1-2, [apcore#109](https://github.com/aiperceivable/apcore/issues/109)).** The shape §6.1.7 was written for is a narrow approval rule ahead of a broad allow — `git push --force` needs a human, `git push` does not. When the narrow rule's condition could not be *evaluated*, §6.1.1 resolved it to "does not match, MUST NOT grant" and scanning continued; the broad rule then granted, carrying no requirement of its own. The result was `access: "allow"` with `approval_required: false` on exactly the call the operator gated, and `matched_rule_index` naming a rule that never mentioned approval — while `resolve_unevaluable_rule` logged *"the allow rule does not match and MUST NOT grant"* on the very call it was granting. Reproduced in all three SDKs.

  **The root cause is a section that outlived its assumptions.** §6.1.1 was written in spec v1.22.0, when a rule carried one axis, and "an `allow` rule MUST NOT grant" was a complete instruction then: the rule steps aside, and stepping aside was harmless because whatever granted next also said `allow`. Spec v1.28.0 gave rules a second axis (`approval`) and did not revisit it, so "does not grant" began silently discarding the requirement the rule carried.

  **It is not confined to the legacy boolean or to a missing projection.** The trigger is an unevaluable `allow` rule, and §6.1.1 is the path that misconfiguration, §6.1.2's warn-don't-fail registration ordering and handler failure all take. A misspelled predicate (`has_keys` for `has_all_keys`) or an unregistered condition key reaches it **with a governance projection present**, on the ordinary Executor pipeline; `default_effect: allow` reaches it with no second rule at all. `validate_rules()` is not a mitigation — §6.1.2 makes an unregistered condition key a warning rather than a load failure, so nothing stops such a rule reaching production.

  **The requirement is now pending rather than discarded.** An unevaluable `allow` rule carrying `approval: required` records a pending requirement and scanning continues; the rule itself still does not grant. Whatever grants next composes it by **disjunction** — a later `allow` rule *or* `default_effect: allow`, which makes `approval_required: true` with `matched_rule_index: None` a legal combination for the first time. A final decision of `deny` clears it, and `matched_rule_index` keeps naming the rule that actually decided rather than the unevaluable one. `handler_error` is untouched: a pending requirement neither suppresses nor substitutes for it, and `AuditEntry.approval_required` carries the **final** value. The §6.1.1 warning gained a `pending_approval` field and now names the surviving requirement, because the old wording was logged on the call the next rule then granted.

  **Scope is what keeps it from over-reaching.** `matches_rule` returns `Unevaluable` only *after* both pattern lists have matched, so a rule whose well-formed `callers` / `targets` do not cover this call has already left as `NoMatch` with its conditions never consulted (§6.1.4 rule 4) and raises nothing — a rule written about one target must not attach a human to calls it was never written about. Rule 5's other half, where a rule whose own pattern field is *malformed* (§6.1.4.1) **does** raise the requirement because its scope cannot be read, is satisfied structurally here: `callers` and `targets` are `Vec<String>`, so serde rejects a bare string at deserialization and the compiler rejects one in a struct literal, exactly as §6.1.4.1 anticipates. The fixture case marked `skip_if_unrepresentable` is skipped for that reason and named in the driver's skip list rather than passed over in silence.

  **Requiring a human rather than denying is deliberate.** The condition that could not be evaluated is the one that decides whether *this* call is the dangerous one, so refusing would turn every ordinary `git push` into the hard failure §6.1.7 exists to eliminate. "Ask" is the answer that is wrong in neither direction.

  Applied at **every** entry point — `check_access`, `async_check_access` and both legacy booleans. A requirement that survives on one and is lost on another is the same fail-open, reachable by choosing a different call; `check_inner` and `async_check_inner` are separate scan loops, so each carries its own accumulator and both compose it through the shared `finalize_rule_match` / `finalize_default_effect` helpers, which is what keeps the two paths from drifting. §6.8.1's fail-closed rule for `check()` is now a property of the **decision** rather than of the matched rule, and the booleans inherit it for free because they read `AccessDecision::approval_required` instead of re-deriving it.

  **Backward compatible for correct configurations:** across all 20 pre-existing cases of `conformance/fixtures/acl_argument_scoped_approval.json` **with a projection**, no decision changes. Without a projection, two change, both `approval_required: false` -> `true`.

---

## [0.27.0] - 2026-08-14

> **Release note:** this section contains BREAKING changes. It must ship as a
> **minor** (or major) version bump, never a patch.

### Changed

- **BREAKING (security): a failed `acl` check now withholds module-level introspection from `validate()` (spec v1.13.0 §12.8.5.1, apcore#96).** `validate()` looked the module up at Step 3 and ran `preflight()` and `preview()` at Check 7 on the strength of that lookup alone, so a caller the ACL had just denied still made module-authored code run and still received what it returned. For a command-wrapping module that is the resolved binary and its argv; for a writer it is the target of the side effect. All three SDKs did it, and `apcore-mcp-rust` had already grown a string-matched disclosure filter over the top of it, which is the evidence the gap was reachable in a shipped product rather than theoretical.

  `validate()` no longer invokes either hook, emits a `module_preflight` / `module_preview` check, or populates `predicted_changes` when the `acl` check failed. The failed `acl` check itself is still reported, so a denied caller still learns *why*, and no other check is suppressed: the rule is about **authorization**, not validity. A failed `schema` check does **not** suppress introspection — a caller the ACL permits is entitled to the module's account of what would happen even when its inputs are malformed, which is what it needs in order to fix the call. Pinned by `conformance/fixtures/preflight_disclosure.json` (4 cases), whose control case exists so that an implementation which never introspects at all cannot pass the denial cases for the wrong reason.

- **`ACL_CHECK_NAME` and `MODULE_PREFLIGHT_CHECK_NAME` are exported (apcore#96).** `MODULE_PREVIEW_CHECK_NAME` had been public since the preview hook landed, with doc text asking callers to use it "instead of the literal string to avoid drift with other SDKs" — while its two siblings stayed as bare literals in `Executor::validate` and in the private `step_to_check_name`. `apcore-mcp-rust` matches all three by string to keep argv away from a denied caller, and could bind only one of them; the other two carry a runtime guard test precisely because a rename upstream produces no compile error and no test failure, just a filter that silently stops matching. Both are now `pub const` and re-exported from the crate root, and `executor.rs` uses them at every site.

- **`validate()` emitted the same failure twice.** On the error path it extended `checks` from the pipeline trace — which already carries a failed entry for the aborting step, coded `STEP_<NAME>_FAILED` — and then pushed a second, categorized entry carrying the WIRE code. `errors()` therefore reported two problems where one existed, and §12.8.4's one-entry-per-check shape did not hold. The trace's entry for the aborting step is now dropped in favour of the typed one; the steps that **passed** are kept, which apcore-python loses by dropping the whole trace. Same failure count as apcore-python now, with a more informative list.

### Added


- **Load-path test coverage for the seven config sections that had none, and a guard that was reading a partial section list (apcore-rust#34).** **Test-only — no `src/` behaviour changed.** An audit of every `Config::load` / `from_yaml_file` / `from_json_file` / `discover` call in `tests/` found that `bindings`, `id_map`, `logging`, `middleware`, `obs`, `pipeline` and `validation` were never written into a real config file by any test. All seven are top-level keys `schemas/apcore-config.schema.json` accepts, and all seven were verified only through `Config::from_defaults()` + `set(…)` — which writes into `user_namespaces`, a shadow entry no YAML file can produce, and skips `Config::deserialize` entirely. That is the state `observability` was in the day before apcore-rust#33 and `executor` the day before its own gap.

  Each section was probed through `Config::load` before any test was written; **all seven survive every load path intact**, so this closes a verification gap rather than a defect. `tests/test_config_load_framework_sections.rs` now pins them in namespace mode, legacy mode, the JSON branch, across `reload()` and through a `data()` round-trip, asserting both that `get()` returns the file's value and that `namespace()` agrees with it key for key — the two-readers-disagree invariant that #33 and #34 each violated. A control test (`absent_sections_are_not_invented`) keeps the suite non-vacuous: none of the seven has a `CONFIG_DEFAULTS` entry, so an undeclared one must resolve to `None`.

  The reason they went unnoticed is that `tests/test_config_load_coverage_guard.rs` assembled its section list from five of the six places `src/config.rs` names a section and skipped `FRAMEWORK_SECTION_KEYS` — the longest list, and the only one projected from the canonical schema. All seven appear *only* there, so the guard reported nothing missing. It now reads that table too, and its fixture scan is position-aware rather than a substring search: `logging` is both the root `logging:` section and the `observability.logging.*` family, and the old `contains("\n  logging:")` marker read the nested family as coverage for the root section — a guard passing for the wrong reason, which is the same class of mistake it exists to catch.

### Changed


- **BEHAVIOUR CHANGE: an undeclared key inside a framework section now SURVIVES `Config::load` instead of being discarded.** `PROTOCOL_SPEC` §9.14 (`reject_unknown_framework_keys`) makes retention normative: with `_config.strict` absent or false, a key inside a framework section that `schemas/apcore-config.schema.json` does not declare **MUST** be retained and readable through `get()`, because "an SDK that models a section as a typed record still has to keep what the record does not model" — "the operator wrote it and it vanished" is indistinguishable from "the operator never wrote it".

  **Which sections this affects: `executor` only.** apcore-rust models exactly three fields outside the `#[serde(flatten)] user_namespaces` bag — `modules_path`, `executor` and `observability` — and serde silently drops whatever the typed struct does not model. `observability` was fixed in apcore-rust#33 and already retained its raw block; `modules_path` is a scalar with no subkeys to lose. `executor` was the remaining gap: `executor: {vendor_knob: hello}` reached `ExecutorConfig`, which does not model it, and it vanished at parse time. Every other section (`acl`, `extensions`, `schema`, `logging`, `middleware`, `pipeline`, `validation`, `id_map`, `bindings`, `sys_modules`, `stream`, `obs`, `project`, `_config`) lives in the flatten bag and always retained. So configuration that was previously discarded — any undeclared `executor.*` subkey written by an operator, a vendor, or a framework integration — is now present in `get()`, `namespace("executor")`, `bind("executor")` and `data()`.

  The reconciliation rule is unchanged and now applies to both typed sections through one code path (`typed_namespace_view`): the typed struct wins for the leaves it models, the retained raw tree owns everything else. A runtime `set("executor.max_call_depth", …)` still beats the file's now-retained stale copy in every reader. `tests/test_config_load_executor_namespace.rs::unmodelled_executor_key_in_the_file_is_not_resolvable` asserted the old behaviour and called the divergence from apcore-python and apcore-typescript "a spec question for the apcore repo"; §9.14 answered it the other way, and the test is renamed to `…_is_retained` and corrected in place.

- **`_config.strict: true` now rejects undeclared framework keys, enumerating every one of them.** §9.14's strict tier: the key raises `CONFIG_INVALID`, and the error names *all* offending keys rather than failing on the first, so one restart shows the whole problem. Applies in legacy mode too, where the whole file is the `apcore` namespace. `allow_unknown` does **not** soften it — that field is defined in §9.6.3 for unknown top-level *namespaces*. The section→key table (`config::FRAMEWORK_SECTION_KEYS`) is a projection of `schemas/apcore-config.schema.json` (unioned with `schemas/sys-modules.schema.json` under `sys_modules`, exactly as `config_key_governance.json` projects the canonical key surface); `framework_section_keys_match_the_canonical_schema` re-derives it from those files on every run, so a section added upstream fails the build instead of being silently exempt.

- **Fixed: `_config.strict: true` no longer reports framework sections as unknown namespaces.** Rust merges the `apcore:` block's members up to the top level of `user_namespaces`, which left framework sections sitting beside genuine Config Bus namespaces in the same map. §9.10 step 3b then rejected any strict namespace-mode document that declared an `acl:` block (`unknown namespace 'acl'`) — or the §9.1 required `project:` field. Namespaces are still checked; sections are governed by the §9.14 check instead.

- **Conformance driver follows the fixture.** `middleware_hardening.json` dropped its `tracing_span_created` case (it pinned §1.3) and `context_namespace_violation` now uses `_apcore.mw.tracing.spans` as its example framework-owned key instead of `_apcore.mw.tracing.span_id`. The corresponding driver case in `tests/test_middleware_hardening_conformance.rs` was deleted, and `tracing_noop_without_otel` now drives `observability::TracingMiddleware`: the fixture's `otel_available: false` reads in Rust as "no telemetry pipeline is wired", since the middleware links against no OpenTelemetry SDK at all.

- **BEHAVIOUR CHANGE: an unknown key under `pipeline.configure` is now a parse-time error, and `pipeline.steps` entries are closed (apcore#89).** `configure` logged `Unknown configurable field — ignored` and ran the unconfigured pipeline, while apcore-python and apcore-typescript both raised — so the same `apcore.yaml` behaved differently on each SDK and the operator's typo produced a silently unconfigured step here. The accepted set is unchanged and was correct all along: exactly `match_modules`, `ignore_errors`, `pure`, `timeout_ms` (`schemas/apcore-config.schema.json` `$defs/ConfigurableStepFields`, `DECLARATIVE_CONFIG_SPEC.md` §4.2). Only the failure mode changes, to `PIPELINE_CONFIGURATION_ERROR`.

  `requires` / `provides` were among the keys this SDK dropped, and dropping them was accidentally the right outcome: on apcore-python and apcore-typescript the same YAML *applied* them, moving built-in `input_validation` from `requires=["module"]` to `requires=["context"]` — deleting the dependency `module_lookup` satisfies, after which construction validates cleanly and the `PipelineDependencyError` MUST can never fire for that step. They are now rejected explicitly rather than ignored.

  `pipeline.steps` entries are closed over the ten `$defs/PipelineStep` keys. Measured on this SDK before the change: `{"name": "x", "type": "probe_noop", "after": "execute", "tiemout_ms": 5000}` built successfully with `timeout_ms == 0`, the operator's five-second timeout silently absent. Unlike apcore-typescript, the canonical snake_case spellings already worked here, so only the key set needed closing.

- **Both messages name EVERY offending key, not the first.** One restart shows the whole problem instead of one restart per typo — the precedent apcore-python set for its `_config.strict` framework-key check. It is also what makes the conformance assertion portable: reporting only the first ties the message to map iteration order, and `serde_json::Map` is a `BTreeMap` (sorted) while Python dicts and JS objects preserve insertion order, so a fixture naming one key passed on two SDKs and failed here for no behavioural reason.

- **BEHAVIOUR CHANGE: the library-level coercion knob narrows from twelve boolean spellings to two (apcore#95).** `SchemaValidator::with_coerce_types(true)` accepted `"true"`, `"yes"`, `"on"`, `"y"`, `"t"`, `"1"` and `"false"`, `"no"`, `"off"`, `"n"`, `"f"`, `"0"` for a declared `boolean`, **case-insensitively** — so `"True"`, `"YES"`, `"Off"` were accepted too. It now accepts **exactly `"true"` and `"false"`, case-sensitive**. Every other spelling, including every capitalisation of the two kept ones, is left unchanged for the jsonschema check to reject. **Input a caller's coercing validator previously accepted is now rejected**; a caller passing `{"flag": "1"}` or `{"flag": "True"}` against `{"type": "boolean"}` gets a `SCHEMA_VALIDATION_FAILED` where it used to get `true`.

  `TYPE_MAPPING` §11 "What the knob coerces, when it exists" (spec v1.12.0) is the source: offering the knob stays a **MAY**, but an SDK that offers one **MUST** coerce exactly `"42"`/`"-7"` → `integer`, `"1.5"`/`"-0.5"` → `number`, `"true"`/`"false"` → `boolean`, and **MUST NOT** coerce anything else. `"true"` and `"false"` are JSON's own spelling of a boolean; `"yes"`, `"on"`, `"y"`, `"t"`, `"1"`, `"0"` are shell and INI conventions that belong to whatever parses `argv`.

  `"0"` → `false` is the sharpest removal: R5 makes the *number* `0` a MUST-reject for `boolean` at the module-invocation boundary, so this SDK was answering opposite ways for two spellings of one value depending on which of its own paths you held. The dialect originated here — apcore-typescript ported `coerce_str_to_bool` verbatim during apcore#93 — and both SDKs narrow together while apcore-python gains the two literals it never had.

  **Nothing else changes.** The module-invocation boundary never coerced and still does not; the knob's default is still `false` (`SchemaValidator::new()`); integer and number coercion are untouched and were already conforming (`"3.14"` was already a MUST-reject for `integer`, re-verified by injection). `coerce_str_to_bool` had exactly one caller, `coerce_value`, so no config, env or CLI parsing is affected — `Config::coerce_env_value` is a separate function with its own `"true"`/`"false"` handling and is not governed by §11.

  Pinned by six new cases in `conformance/fixtures/schema_validation.json`, four of them asserting a spelling that MUST NOT coerce, each stating both `expected_valid_strict` and `expected_valid_coerce`. `tests/test_schema_validator.rs::test_coerce_string_to_bool` asserted the wide dialect and is **corrected in place** with the reason, not deleted — §11 caps the set rather than removing the feature. `tests/conformance_test.rs` now refuses a case that states `expected_valid_strict` without `expected_valid_coerce` instead of half-asserting it, per the fixture's `both_halves_or_neither` contract.

- **BREAKING: an operator `sensitive_keys` list REPLACES the default list instead of being merged into it.** `RedactionConfig::from_config` unconditionally seeded all 16 `DEFAULT_SENSITIVE_KEYS` and then ADDED the operator's entries, so an operator could only ever widen the redaction policy — removing an entry had no effect, and `sensitive_keys: []` produced no key-based redaction in apcore-python and apcore-typescript but the full 16-entry set in apcore-rust. D-54 and `features/observability.md` both state the override "**replaces** the default; it does not merge". Absent or explicitly `null` still means "not configured" and applies the shipped defaults; an explicitly empty list is an override like any other and disables key-based redaction entirely — including the `_secret_*` prefix, which is entry `[0]` of the default list and not a separate hardcoded rule. Deployments that relied on the union now redact strictly by the list they configured, so a policy that was silently widened by the defaults must be spelled out in full.

- **BREAKING: `ErrorCode::ConfigurationError` renamed to `ErrorCode::PipelineConfigurationError`.** The variant serialized to the wire string `CONFIGURATION_ERROR`, which appears in no canonical registry; the code for "the `pipeline:` config section names a step that does not exist" is `PIPELINE_CONFIGURATION_ERROR` (decision log 2026-05, `features/error-system.md`). apcore-python already emitted it. Callers matching on `ErrorCode::ConfigurationError` must update; the five emit sites are all in `pipeline_config.rs`. Its doc-comment also claimed the cross-language equivalent was Python/TS `CONFIGURATION_ERROR` — a string in neither SDK.

- **`inject_checked` returns `INVALID_PARENT_ID`.** It returned the generic `GENERAL_INVALID_INPUT`, making Rust the outlier against decision D-51 and apcore-typescript: a caller matching on the specific code had nothing to match.

- **MUST fix: correlation fields are exempt from the value-regex redaction rule too.** `NEVER_REDACT_FIELDS` guarded only the field-NAME rule, so a `trace_id` whose value happened to match a secret regex was redacted in Rust and not in Python/TS — breaking trace correlation exactly where it matters. `features/observability.md` states the exemption unconditionally. The per-entry decision now lives in one place (`redact_inner`) instead of being duplicated in `redact_object`.

- **`PeriodicUsageExporter::stop()` is idempotent**, keeping `shutdown()` exactly-once per `start()`.

- **`StepMiddleware::after_step` now fires after a RECOVERED step body**, not only after a naturally successful one. When `on_step_error` returned `Ok(Some(value))`, the engine jumped straight to the next step and skipped the `after_step` chain entirely — so a middleware that acquired something in `before_step` (a span, a lock, a connection) never released it on the recovery path, and Rust was the only SDK doing this: apcore-python and apcore-typescript both close the onion there. The hooks run in reverse registration order, exactly as on the success path, and an `Err` from one still surfaces as `PIPELINE_STEP_ERROR`. Pinned by the new fixture case `after_step_fires_after_a_recovered_step`.

  The `before_step` failure path is deliberately **not** unified with it: no step body ran there, so `after_step` MUST NOT fire, the `on_step_error` recovery value stays discarded, and the step's `ignore_errors` still does not apply — `MIDDLEWARE_CHAIN_ERROR` propagates regardless. apcore-rust already behaved this way and the specification adopted it (`features/middleware-system.md` → "A `before_step` failure terminates the step — it is not recoverable"); the new fixture case `before_step_failure_recovery_is_discarded` now guards it, asserting the discard by observing that the FOLLOWING step never executes rather than merely that an error came back. The same section's later rule — that first-recovery-wins MUST NOT apply to the cleanup pass, since no recovery is being sought and stopping early would strand the cleanup of every middleware registered behind the one that returned a value — was likewise already satisfied: the unwind loop has no early exit on any arm. The `Ok(Some(_))` arm is covered by the fixture; a cleanup hook that itself raises is not, so `before_step_unwind_notifies_every_entered_middleware_even_if_a_hook_fails` covers that arm directly.

- **`StepMiddleware` ordering and error wrapping corrected (Issue #33 §2.2)**, matching the contract now pinned by `conformance/fixtures/pipeline_step_middleware.json` — which apcore-rust drives from disk for the first time (`tests/test_pipeline_step_middleware_conformance.rs`).
- **BREAKING: a schema declaring an older draft is now actually validated.** `SchemaValidator` pinned `Draft202012` while leaving the document's own `$schema` in place — and under `jsonschema` 0.28 that combination compiles an **accept-everything** validator, so every draft-07 module schema had been silently unvalidated: `{"count": "abc"}` against `{"count": {"type": "integer"}}` returned `valid = true`, as did `{}` against a schema with required keys. The executor meanwhile used `validator_for`, which auto-detects the draft and validated correctly — but under draft-07 `format` is an assertion, so a `format: "email"` field failed hard, contradicting §7.2.1. The two validators reached opposite verdicts on the same input. Both now keep the document's declared draft (so draft-07 structural syntax such as tuple-form `items` still means what it says) and disable format assertions outright. Nested `$schema` declarations inside `$defs` are stripped first — they would otherwise re-open the same hole one level down.

- **BREAKING: `SchemaValidator::new()` no longer coerces types.** The validator and the module-invocation boundary previously disagreed: the boundary rejected `{"a": "42"}` for `{"a": {"type": "integer"}}` while `SchemaValidator::new().validate_detailed()` accepted it. Use `with_coerce_types(true)` for the opt-in library-level mode; it never reaches the module boundary (type-mapping §17.3).

- **BREAKING: `RefResolver::resolve()` preserves self-references.** A `$ref` re-entered after descending through a schema body — `{"$ref": "#"}`, the root `$id`, or a recursive `$defs` entry — is returned unchanged as a lazy reference instead of failing with `SCHEMA_CIRCULAR_REF` (§4.15). A `$ref` → `$ref` chain reaching no schema body still fails. Validation was already correct — `validate_against_schema` never ran the resolver — so this aligns the resolver with the specification rather than changing module behaviour.

### Removed


- **BREAKING: `OtelTracingMiddleware`, `OtelTracingBuilder` and `OtelTracingConfig` are removed**, together with the module behind them (`middleware::otel_tracing`), its three auxiliary context keys (`TRACING_ATTRIBUTES_KEY`, `TRACING_SPAN_NAME_KEY`, `TRACING_SPAN_STATUS_KEY`), the `namespace_keys::TRACING_SPAN_ID` constant, and the `opentelemetry` cargo feature that gated it. They implemented `features/middleware-system.md` §1.3, which has been **withdrawn**: `TracingMiddleware` is specified once, in [`features/observability.md` § Tracing Architecture](https://github.com/aiperceivable/apcore/blob/main/docs/features/observability.md), with `protocol-spec.md` §12 as the normative source for the span.

  §1.3 was the weaker of two formulations of the same middleware — both wrote the `_apcore.mw.tracing.*` namespace, which is a single framework middleware's private space by definition. It named the span after `module_id` where the protocol requires `apcore.module.execute` with the module id as an attribute (conformance `T08-007`; a span name per module is the high-cardinality pattern the OpenTelemetry semantic conventions advise against, so the section labelled "OpenTelemetry-Compatible" prescribed the less compatible of the two). It stored one span id in `_apcore.mw.tracing.span_id`, a single slot that the first nested module-to-module call overwrites, where the surviving contract keeps a **stack** in `_apcore.mw.tracing.spans` with explicit `parent_span_id` links. Its `traceparent`-propagation SHOULD was implemented by no SDK because the outbound-call hook it needs does not exist — which is why `TracingConfig::propagate_traceparent` was already documented here as behaviourally inert.

  **No usable surface is lost.** No SDK ever shipped §1.3 as a product: apcore-python never implemented it, apcore-typescript's copy was never re-exported from the package root, and this crate's copy existed under the `OtelTracing*` prefix only to dodge the name collision with the real one. Everything §1.3 still required that survives — the span, the stack, the `_apcore.mw.tracing.sampled` decision, the silent no-op when no telemetry backend is present — is already provided by `apcore::TracingMiddleware` (`observability::tracing_middleware`), which is unchanged and remains the only tracing middleware.

  **Migration:** replace `OtelTracingMiddleware` with `apcore::TracingMiddleware`, which takes a `SpanExporter` (`StdoutExporter`, `OTLPExporter`, `InMemoryExporter`, or your own) and optional `SamplingStrategy`. Remove `features = ["opentelemetry"]` from your `Cargo.toml` dependency entry — the feature no longer exists and cargo errors on an unknown feature.

- **BREAKING: `TracingMiddlewareConfig` loses `service_name`, `propagate_traceparent` and `enabled`.** The YAML `- type: tracing` entry (`middleware-system.md` §1.4, still normative) now builds the surviving `TracingMiddleware` — defaulting to `StdoutExporter`, which routes spans through the `tracing` crate and is replaceable at runtime via the `span_exporter` extension point. The three removed keys configured only the §1.3 middleware. An existing `apcore.yaml` carrying them still parses: serde ignores unknown keys, so they are inert rather than fatal. `priority` and `match_modules` are kept, and `priority` is now honoured through the same `PriorityOverride` wrapper the `custom` type uses — which also gained `as_any` forwarding, without which a priority-wrapped `TracingMiddleware` would be found by name by `ExtensionManager::apply` but fail to downcast, silently dropping the `span_exporter` extension.

### Fixed


- **Two binding errors the spec makes normative were unreachable in this SDK, found by making 31 dead conformance assertions real (apcore#93).** `conformance/check_case_pinning.py --sdk rust` asks the per-SDK question apcore#92 did not — *does **this** SDK run the case* — and reported 31 fixture cases across nine fixtures whose declared expectation reached no assertion here. Mutating any of them left apcore-rust green. Both `src/` defects below were exposed by writing the assertion the fixture asks for; everything else in this entry is test-only.

  **`BINDING_FILE_INVALID` did not use the canonical message.** `DECLARATIVE_CONFIG_SPEC.md` §7.2 fixes the template and `binding_errors.json` pins the exact string; apcore-python and apcore-typescript both emit `Invalid binding file '<path>': missing required top-level key 'bindings'`. `BindingLoader::load_from_file` / `load_from_yaml` surfaced serde's own `missing field \`bindings\`` text instead, sharing no wording with its peers for the same file. Both loaders now check the top-level key before deserializing and emit the canonical message. The driver for that case had read `expected_message` and then thrown it away — `let _ = (expected_msg, &err);` — with a comment saying the Rust text "may differ"; the difference was the finding.

  **`ErrorCode::BindingInvalidTarget` was declared, categorised, and raised by nothing.** §2.2 requires target-syntax validation at parse time (`"Such validation produces BindingInvalidTargetError at parse time"`), and the canonical regex requires the `<module_path>:<symbol>` split. `BindingLoader` performed none: `target: no-colon-here` loaded silently and failed much later as an unrelated handler-map miss. `ingest` now validates every entry's target and raises `BINDING_INVALID_TARGET` with apcore-python's message byte for byte. Only the separator form is enforced — Rust resolves a target through an opaque handler-map key and never touches the filesystem (§3.7 "Rust caveat"), so the traversal rejection §2.2 asks of TypeScript has no analogue here. The driver arm this replaces asserted `!target.contains(':')` — the fixture's own input restated, a tautology no implementation can break.

  **`config_defaults` was the largest gap and, measured, apcore-rust is correct.** The driver carried a six-entry `supported_keys` allowlist and `continue`d on everything else, so 12 of the 18 canonical defaults — every `extensions.*`, `schema.*`, `acl.*`, `sys_modules.*`, `stream.*` key plus `observability.tracing.sampling_rate` — were never compared against the canonical table. The allowlist's reason ("not part of the Rust SDK's typed Config struct ... and have no default") went stale when `CONFIG_DEFAULTS` landed and `Config::get` gained its fallback. All 18 now assert, a key resolving to `None` is a hard failure rather than a skip, and **all 18 agree with `schemas/defaults.schema.json`** — this closes a verification gap, not a divergence. `config_key_governance :: sdk_reproduces_every_canonical_default` was unpinned for the same reason one level up (it asserted `missing.is_empty()` and never read the case's declared `expected.missing`, unlike its two siblings in the same file) and now compares against the fixture's list.

  **The remaining shapes, all four catalogued in apcore#92 and all four present here.** `call_chain`'s six positive cases asserted only `result.is_ok()`, which a guard that inspects nothing also satisfies — they now assert the `"ok"` sentinel itself and an **observable post-condition**: the same chain re-run with the one limit it sits under tightened by one must produce the matching rejection. Its negative cases stopped matching message substrings and now resolve the fixture's wire code through serde. `config_env`'s four `env_style` cases were skipped wholesale because the namespace registry is process-global and `mcp` cannot be re-registered per style; each style now gets its own shadow namespace carrying the same `max_depth` and driven with the same env-var suffix, so §9.8's resolution algorithm decides the outcome. `schema_validation :: empty_schema_accepts_string` was skipped as a "known gap: empty schema + string input" — there is no gap, `{}` is the Draft 2020-12 always-true schema and the `jsonschema` crate accepts scalar roots, so the one case pinning non-object root handling was the one case being dropped. `async_task_evolution :: reaper_disabled_by_default`, `multi_module_discovery :: full_id_grammar_valid` and `system_modules_hardening :: reload_module_id_and_filter_conflict` each hardcoded what the case declares; all three now read inputs and expectations from the fixture, and `full_id_grammar_valid` no longer dismisses `expected.module_ids` as "illustrative only" on the strength of a comment describing a value the fixture does not contain.

  **Two guards added while the drivers were open.** `binding_errors`'s catch-all arm — `eprintln!("WARN ... unknown case")` — is now `panic!("teach the driver, do not skip it")`, with the one genuinely per-SDK case (`binding_schema_inference_failed_python`, already in the spec repo's `case_pinning_allowlist.json`) named explicitly. And every `schema_validation` case now cross-checks the executor's module-invocation boundary (`validate_against_schema`) against `SchemaValidator::with_coerce_types(false)`: `type-mapping.md` §17.3 names apcore-rust for having once shipped a validator whose default coerced while the executor path did not, so the two are asserted to agree on every case rather than trusted to.

  Verified with `check_case_pinning.py --sdk rust` over all nine fixtures: 99 cases mutated, **0 pinned by no driver**.


- **Four conformance drivers did not read the fixture they claim to drive, and 17 cases here could not be made to fail (apcore#92). Test-only — no `src/` behaviour changed; every assertion added passes against the SDK as it stands.** Measured with `conformance/check_case_pinning.py --sdk rust`, which mutates a case's expectation so no correct implementation can satisfy it and reports the case if nothing goes red: `error_codes` 12 of 18, `version_negotiation` 3 of 10, `context_create` 1, `identity_system` 1. A case that cannot go red is not coverage — it reads as covered in the fixture, in review, and in every count derived from the inventory, while nothing is checked.

  `dependency_version_constraints` measured **0 of 15** and is the shape the other four now follow: it dispatches on the expected *value* with a catch-all `other => panic!`, it compares the resolver's `ErrorCode` through its serde spelling against the code the fixture declares (plus all four `details` fields), and its positive cases assert an observable result — the load order, and for a skipped optional edge the dependent appearing *before* its dependency. Nothing in it is satisfied by an implementation that does nothing.

  Four defect shapes were found, all four of them present here and not only in apcore-python where they were first diagnosed:

  - **Branching on whether the expectation KEY exists rather than on what it says.** `conformance_error_codes` and `conformance_version_negotiation` both read `tc.get("expected_error").is_some()` and then asserted only `is_err()`. The declared wire code never reached an assertion, so changing `ERROR_CODE_COLLISION` or `VERSION_INCOMPATIBLE` in the fixture to any other string left the suite green.
  - **A positive case whose whole assertion is "did not return `Err`".** The nine `expected: "ok"` cases in `error_codes` called `ErrorCodeRegistry::register` and asserted nothing else — a `register` that stored nothing at all passed every one. They now assert the post-condition the registry actually offers: the code is queryable afterwards through `codes_for_module` **and** `all_codes`. The negative cases assert the mirror of it (a rejected registration stored nothing), and `unregister_allows_reuse` asserts the release is observable before the re-registration under another module is allowed to succeed for the right reason.
  - **An unrecognised expectation skipping the assertion.** Every dispatch in the four drivers now ends in a `panic!` naming the case and the value, the "teach the driver, do not skip it" pattern `tests/test_pipeline_failfast_config_conformance.rs` already used. `identity_system` goes further and requires *every* `expected*` key a case states to reach an assertion, so a fixture that grows a field cannot be silently half-read.
  - **Asserting a type name instead of the wire code.** A new `wire_error_code` helper resolves the fixture's declared code to `ErrorCode` through serde, so an unknown code is a hard failure rather than a skipped branch; the comparison is then variant against variant.

  Two fixture cases were rewritten upstream and the drivers now follow them. `identity_system :: identity_propagates_to_child_context` stated its expectation as the prose string `"child.identity === parent.identity"` — a sentence in a value slot, which no driver can assert, so every driver hardcoded the comparison behind an `if id == …` and the fixture value was decoration; it is now four named fields (`child_identity_id`, `child_identity_type`, `child_identity_roles`, `child_identity_equals_parent`) and each is read and asserted. `context_create :: executor_rejects_cross_executor_rebind` carried `expected_one_of: [raise, silent_accept]`; a driver cannot assert an alternation without deciding which branch its SDK takes, so this one hardcoded the raise and named the alternation only in a comment. **apcore v1.11.0 promoted the SHOULD to a MUST** — all three SDKs raise — and the expectation is now `{raises: true, error_code: "CONTEXT_BINDING_ERROR"}`, read from the fixture. The wire code matters here beyond style: `ContextBindingError` is a *class* apcore-python and apcore-typescript share and this SDK does not have, so a class-name assertion is green on two SDKs and unwritable on the third.

  `version_negotiation`'s `PARSE_ERROR` is the one expectation that is not a wire code, and the fixture says so: it is a language-specific sentinel for "this semver string does not parse". This SDK has no `ParseError` variant — `negotiate_version` reports the malformed input as `VERSION_INCOMPATIBLE` with a message naming the offending string — so the driver asserts exactly that, rather than the `is_err()` that accepts an error raised for any reason.

  All four fixtures now measure 0 unpinned, and `dependency_version_constraints` stays at 0.

- **BEHAVIOUR CHANGE: declared `dependencies` now survive two registration paths that discarded them, so a `path_filter` reload orders by the dependency graph instead of the alphabet (apcore-rust#35).** `ReloadModule::topo_sort_modules` runs Kahn's sort over `Registry::get_definition(...).dependencies` and always worked — when the descriptor carried the edge. Two of the three ways to declare one built the descriptor with `dependencies: vec![]` hard-coded, so **configuration that was silently discarded now takes effect**:

  **Filesystem discovery — the `dependencies:` block of a companion `_meta.yaml`.** `DefaultDiscoverer` parsed it into a value consumed *only* by stage 6's load-order topo sort, while `build_descriptor` hard-coded an empty list, and `Registry::register_discovered` stores that descriptor verbatim. The block therefore ordered the initial load and then vanished: `get_definition().dependencies` was empty for every filesystem-discovered module. The parse now happens once, before the descriptor is built, and feeds both consumers — so the two cannot drift apart.

  **`Registry::register_versioned(name, module, version, metadata)` — a `dependencies` key inside `metadata`.** This is the SDK's canonical four-argument form of the spec's `register(module_id, module, version?, metadata?)`, and the shape it now reads — `[{"module_id": "…", "version": "…", "optional": false}]` — is exactly what apcore-python and apcore-typescript accept in the same argument position. Until now the same call with the same arguments built a working dependency graph in those two SDKs and an empty one here; only the three-argument descriptor form worked. A `dependencies` value that is not an array is treated as "none declared" rather than an error, since no schema describes the key. The two-argument `register_module` still declares none — it takes no metadata and the `Module` trait has no `dependencies()` method to derive from — which is now stated at the field instead of being silently implied.

  **Why this stayed invisible.** It is the defect class apcore-python fixed in `ad2998d` and apcore-typescript in its `#35`, reached through a different code path: there the loss was in the metadata merge, here in descriptor construction. The signature is identical in all three — discovery-time dependency sorting keeps working, so `resolve_dependencies` looks healthy, while the post-registration accessor returns nothing and reload-order sorting degrades to Kahn's seed order, which is alphabetical and therefore plausible. A plausible-looking order is the worst shape for this: reloading a dependent before its dependency leaves the dependent briefly wired to a module that is about to be replaced, and which of the two happened to be right was decided by how the module ids sort.

  The canonical fixture case `reload_order_is_topological_not_alphabetical` registers through the descriptor form and so caught neither. `tests/test_reload_order_registration_paths.rs` drives the two broken paths against the same fixture case — one registering by real filesystem discovery from a tempdir tree, one through the four-argument form — and each asserts both halves of the fixture's `driver_contract`: the edge read back through `get_definition`, and the reload sequence observed from the registry's `unregister` events rather than from a field of the response. Reverting either fix alone reddens its own test and leaves the other green.

- **`Registry::on`'s documented usage no longer shows code that does not compile.** The `registry_events` doc comment, and the `RegistryEvents` / `REGISTRY_EVENTS` items beside it, advertised `registry.on(REGISTRY_EVENTS.REGISTER, callback)` — Python/TypeScript member access transcribed into Rust, where `REGISTER` is an associated const reachable through the type (`RegistryEvents::REGISTER`) or the free-standing module (`registry_events::REGISTER`), never through a value. Documentation only; no API change.
- **`tests/config_discovery.rs` no longer races itself.** Its five tests each mutate process-global state (`APCORE_CONFIG_FILE`, `APCORE_BINDINGS_DIR`, the working directory) and Cargo ran them on parallel threads of one binary, so `test_declared_env_override_still_reaches_the_declared_document` could set `APCORE_BINDINGS_DIR` while a sibling was asserting an exact declared-key set — failing it with a `bindings.dir` key its file never declared. The file's separate `[[test]]` binary isolated it from `tests/it.rs` but said nothing about its own tests; they now serialise on a file-local guard. Test-only; no library change.
- **`$APCORE_CONFIG_FILE` no longer injects a phantom `config.file` key into the declared document (apcore#88).** The variable is the documented way to point at a configuration file (PROTOCOL_SPEC §9.14 discovery, read by `discover_config_file`), but §9.2 also makes *every* `APCORE_*` variable a configuration override and nothing exempted this one. Its suffix lowered to the dot-path `config.file`, so `Config::load(path)` with the variable set left `config: { file: "/path/…" }` in `user_namespaces` — a key `schemas/` declares nowhere (checked against `conformance/fixtures/config_key_governance.json`) sitting inside the **declared** document that `validate()`'s §9.1 required-field check reads through `get_declared`. It is now dropped at the parse site: the variable is an *argument to* `load()` that happens to share a namespace with configuration, consumed to locate the file and then discarded, which is what every other argument-shaped input does. No spec change and no user-visible rename; `discover_config_file` is unaffected. **Rust needed the exemption in two places, not one:** the legacy branch of `apply_env_overrides`, and the un-matched-prefix fallback in `apply_namespace_env_overrides` — apcore-python's namespace path has no such fallback, so a single-site fix ported from it would have left namespace mode broken here. Both are pinned, with the exact declared key set asserted rather than the absence of `config.file`: absence alone also holds for an implementation that lost a key the file really declares. The exemption is one variable wide: `APCORE_BINDINGS_DIR` → `bindings.dir` is a declared key and keeps working, asserted by a third test. The distinguishing test for any future variable is whether its dot-path is in the canonical key surface.
- **BEHAVIOUR CHANGE: the documented nested `retry:` block on a subscriber is now read from config, on all five built-in types (apcore#85).** `features/event-system.md` documents a per-subscriber retry policy and shows it under a heading reading *"showing the policy on multiple subscriber types"* — an `a2a` entry with `max_attempts: 5` and a `file` entry with `max_attempts: 2`. **No SDK parsed it.** An operator who copied that example got the default policy, silently, with nothing to indicate the block had been ignored: `schemas/sys-modules.schema.json` does not describe subscriber entries beyond requiring a `type`, so nothing rejected the key either.

  The capability was already built at every other layer, which is why this survived. `EventRetryConfig` (`events::retry`) declares exactly the four keys the document shows, and `EventEmitter::deliver_with_dlq` calls `subscriber.retry()` with no type check and no allowlist, so whatever a subscriber declares is honoured. The single missing layer was config → object: none of the five `build_*_subscriber` factories constructed a policy.

  `events::subscribers::parse_retry_config` now parses the block and every built-in builder applies the result. Partial blocks merge over the spec defaults (`max_attempts=3`, `initial_backoff_ms=100`, `max_backoff_ms=30000`, `backoff_multiplier=2.0`), as the documented `file` example requires — it declares only two of the four keys. A `retry:` that is not an object is ignored rather than fatal.

  **`FileSubscriber`, `StdoutSubscriber` and `FilterSubscriber` gained the `retry` field they were missing**, each with a `with_retry` builder and a `retry()` trait override, matching what `WebhookSubscriber` and `A2ASubscriber` already had. Without the field they fell through to the trait default and no policy could reach them from any direction — not from config, and not from a caller constructing them directly. All three are real retry surfaces: `FileSubscriber::on_event` returns `Err` on open/write failure, `StdoutSubscriber` writes to stdout (EPIPE, closed stream), and `FilterSubscriber` forwards to its delegate, so a retry there re-runs delegate delivery.

  **`WebhookSubscriber::retry_count` was inert and is now wired.** `build_webhook_subscriber` wrote the flat legacy shorthand into `sub.retry_count`, but `retry()` returns `self.retry` and never consulted that field — so in this SDK the one spelling that *was* parsed had no effect on delivery either. The field's own doc comment claimed the precedence resolution "happens in `build_webhook_subscriber`", which it did not. Both spellings now reach `retry`: `retry_count` keeps its `max_attempts = retry_count + 1` translation as a deprecated webhook-only alias — that spelling is what deployments use today — and **the nested block wins when both are present.** `retry_count` is now applied only when the key is actually present in the config, rather than defaulting to `3` and overwriting whatever else was set.

  Also corrected: the `EventSubscriber::retry` doc comment claimed the default was "single-attempt (no retry)" while the body returns `EventRetryConfig::default()` — 3 attempts with backoff. The text now matches the code and points at `EventRetryConfig::no_retry` for callers who genuinely want one attempt.

  **This changes delivery behaviour for anyone who had already written the documented block**: a subscriber that was silently retrying 3 times now retries as configured. Pinned by `tests/test_subscriber_retry_config.rs`, one case per subscriber type plus two end-to-end cases — one asserting the DLQ payload's `attempt_count` for a real failing `file` subscriber, one counting actual `on_event` invocations. Every asserted value differs from the default, so a case cannot pass against a factory that ignores the block.

---

- **`Config::load` no longer discards every `observability.*` subkey outside the four typed leaves (apcore-rust#33).** `Config.observability` is a typed `ObservabilityConfig` modelling only `tracing.{enabled,sampling_rate,exporter}` and `metrics.enabled`, and it sat outside the `user_namespaces` bag — so `Config::deserialize` handed the whole `observability:` block to that struct and dropped everything it did not model, at parse time, before any accessor could see it. **Silently discarded from a loaded `apcore.yaml`:** the entire `observability.redaction.*` subtree (`sensitive_keys`, `regex_patterns`, `replacement`), `observability.tracing.strategy`, `observability.tracing.otlp_endpoint`, `observability.metrics.exporter`, all of `observability.logging.*` (`enabled`, `level`, `format`, `redact_sensitive`), all of `observability.error_history.*` (`max_entries_per_module`, `max_total_entries`), and all of `observability.platform_notify.*` (`enabled`, `error_rate_threshold`, `latency_p99_threshold_ms`). Every one of them is declared configurable by the §9.15.2 namespace registration; `APCORE_OBSERVABILITY_*` env vars were not an escape hatch. Worse than the loss: `Config::namespace("observability")` deep-merges the registered defaults under the loaded subtree, and with the subtree gone the default *was* the answer — a file saying `logging.enabled: false` read back `true`. The system reported a setting the operator had not chosen, as though they had.

  `Config::deserialize` now also keeps the raw `observability` object in `user_namespaces`. The four typed leaves are unchanged and remain authoritative: they resolve from the typed struct in `get()` (via `get_typed_field`, consulted first) and are overlaid last by the new `Config::observability_view`, which `get`, `namespace`, `bind` and `data`/`Serialize` all read through — so the file, a runtime `set()` and an env override can never disagree about which value is live.

  **This changes behaviour for anyone who wrote those keys and got nothing: a previously-ignored config now takes effect.** A deployment carrying `observability.logging.enabled: false`, a narrowed `observability.redaction.sensitive_keys`, a lowered `observability.error_history.max_total_entries`, `observability.platform_notify.enabled: true`, or an `observability.tracing.otlp_endpoint` has been running on the §9.15.2 defaults; after upgrading it runs on the values in its file. This also un-starves the deprecated-key fallback added below — that legacy branch was structurally reachable but could never receive data from a file.

  Two consequences of the same defect are fixed with it. `Config::data()` — the §9.1 wire form — used `#[derive(Serialize)]`, which wrote the typed `observability` field and then the flattened `user_namespaces` bag into the same map; once the bag held an `observability` entry the second write won and the typed leaves vanished from the wire form entirely (reachable before this change via `set()` on any unmodelled `observability.*` key, which dropped a just-set `tracing.sampling_rate` from `data()`). `Config` now has a hand-written `Serialize`. And `bind("observability")` special-cased the namespace to the typed struct, so a caller binding their own type received a payload with `redaction`, `logging`, `error_history` and `platform_notify` stripped out and no way to tell that from "unconfigured".

  No pre-existing test could have caught any of this: every redaction-config test built its `Config` with `Config::from_defaults()` + `.set(…)`, which writes straight into `user_namespaces` and skips deserialization — the one step that was broken. The new `tests/test_config_load_observability_subkeys.rs` goes through `Config::load` from a real file for every family above, and `tests/test_redaction_config_conformance.rs::legacy_config_key_read_from_a_real_apcore_yaml` is no longer `#[ignore]`d.

- **The `executor` namespace is reachable as a namespace, not only as a typed field (apcore-rust#34).** `Config.executor` is a typed `ExecutorConfig` living outside the `user_namespaces` bag, so the namespace-level readers had nothing to traverse: `Config::get("executor")` returned `None` and `Config::namespace("executor")` returned an EMPTY map — for a config whose `apcore.yaml` plainly declares an `executor:` block, and on which `get("executor.max_call_depth")` returned the file's value all along. The container fetch contradicted its own leaf. apcore-python (`Config.namespace` → `self._data["executor"]`) and apcore-typescript (`namespace(name)` → `this._data[name]`) both return the object; Rust was the only SDK returning nothing, because it is the only one that models the namespace as a typed struct. Affected keys — the four `$defs/ExecutorConfig` declares — are `executor.default_timeout`, `executor.global_timeout`, `executor.max_call_depth` and `executor.max_module_repeat`: all four were invisible to `namespace("executor")`, `get("executor")` and any `bind`/`namespace`-based consumer, in every mode, for every config.

  A destructive second half came with it. `set("executor.<key>", …)` for a key `set_typed_field` does not match — and `mount("executor", …)` — write into `user_namespaces`, giving the namespace two stores; `Serialize` then emitted the typed struct and let the flattened bag overwrite it, exactly as it did for `observability` before #33. A single `config.set("executor.vendor_knob", "x")` reduced `Config::data()["executor"]` to `{"vendor_knob": "x"}`, erasing `default_timeout`, `global_timeout`, `max_call_depth` and `max_module_repeat` — including values loaded from the operator's file — from the §9.1 wire form, and a `data()` → parse → `data()` round-trip then reset all four to their compiled-in defaults.

  Fixed by `Config::executor_view()`, which applies the same rule as `observability_view` (raw tree as the base, typed struct deep-merged over it — the typed struct stays authoritative for the four leaves it models) and which `get`, `namespace`, `bind` and `Serialize`/`data()` now all read through. The two views share one `typed_namespace_view` helper so their precedence rule cannot drift apart.

  **Deliberately NOT changed: an out-of-schema `executor.*` subkey written in a config FILE is still dropped.** This is where the defect is narrower than #33 and the symmetry stops. `ObservabilityConfig` modelled 4 of the 19 keys the §9.15.2 registration declares, so #33 was real data loss; `ExecutorConfig` models exactly the property set `$defs/ExecutorConfig` declares in `apcore/schemas/apcore-config.schema.json`, and that schema is `additionalProperties: false` with no §9.15 `executor` registration adding more — so there is no spec-declared executor subkey to lose. Keeping a raw file copy would make apcore-rust surface configuration the canonical schema rejects, which is a normative change rather than a bug fix. apcore-python and apcore-typescript do preserve such keys, because their config is an untyped dict; that divergence is a specification question for the apcore repo, not a defect here.

  Also unlike #33, the namespace-level failure was a missing value rather than a confidently wrong one: `executor` has no registered §9.15 default layer for `namespace()` to return in place of the operator's setting. `get("executor")` is correspondingly NOT guarded on "did the document declare the block" the way `observability` is — every `ExecutorConfig` leaf is non-optional, so `get("executor.max_call_depth")` already answered `Some(32)` for a document declaring no `executor:` block, and the container has to agree with its own leaf.

  No pre-existing test could have caught this: `src/config.rs`'s 39 unit tests reach `Config` through `Config::default()` or `serde_json::from_value` and assert on `cfg.executor.<field>` — the typed struct, which was never broken — and not one calls `namespace("executor")` or `get("executor")`. The 17 cases in `tests/test_config_load_executor_namespace.rs` all go through `Config::load` from a file on disk.

- **The canonical `obs.redaction.*` config keys are read at all.** `RedactionConfig::from_config` consulted ONLY the deprecated `observability.redaction.{sensitive_keys,regex_patterns,replacement}` spellings, so an operator who followed the documentation and wrote `obs.redaction.sensitive_keys` in `apcore.yaml` had their entire redaction policy silently discarded — no warning, no error, including a deliberate narrowing they believed was in force. The canonical namespace (D-53; `features/observability.md`, "Canonical Config keys (cross-SDK)", which calls any divergence a conformance bug) is now read first, with the deprecated keys still honoured for backwards compatibility behind a one-shot deprecation warning naming the canonical replacement, matching apcore-typescript. A key that is absent or explicitly `null` counts as unset at each level. Note that a previously-ignored canonical config now takes effect: a deployment carrying a narrower `obs.redaction.sensitive_keys` than the defaults will redact fewer fields by name than it did while the key was being dropped.

- **`auto_schema: strict` bindings are checked for OpenAI compatibility.** `ErrorCode::BindingStrictSchemaIncompatible` existed in the enum but had no constructor and no code path producing it. Detection lives in a new `schema::openai_strict` module, separate from `to_strict_schema()`.

- **The format warning walk reaches into combinators, and stops inventing warnings.** It descended only through `properties` and `items`, missing `anyOf` / `oneOf` / `allOf` branches, `additionalProperties` sub-schemas and `prefixItems` entries. `patternProperties` is now walked, and a key it matches is no longer also checked against `additionalProperties` — which had produced warnings for entirely valid data. Union branch membership is decided structurally, so a valid email is no longer reported as a malformed uuid because a sibling branch declared one; a branch that cannot be decided is kept rather than silently dropped.

- **`to_strict_schema` hardens the objects it was skipping** — an optional nested object came out with no `additionalProperties: false` and an incomplete `required` list, which OpenAI strict mode rejects; the same applied to an object declared only by its `properties`. `recurse_into_nested` now descends into `prefixItems`, and a nullable `$ref`-only property is wrapped in `anyOf` rather than `oneOf`.

- **Schema compilation failure reports `SCHEMA_PARSE_ERROR` with details** on both paths, instead of `SCHEMA_VALIDATION_ERROR` without them on one — the old code told the caller to fix their input when the fault was in the module's schema.

- **`validate()` preflight check errors carry the wire error code.** The check built from an unwrapped pipeline failure formatted the `ErrorCode` with `Debug`, so `PreflightResult.checks[].error["code"]` was the PascalCase variant name (`ModuleNotFound`) — a code in no registry. apcore-python builds the same dict from the error's `to_dict()` and apcore-typescript emits `{ code: e.code }`, both wire codes; Rust now emits `MODULE_NOT_FOUND` via `ErrorCode::wire_str()`. The separate trace-derived check keeps its `STEP_<NAME>_FAILED` code, which all three SDKs share. The `error_type` field of `apcore.stream.post_validation_failed` deliberately stays a class-name analogue, matching `type(exc).__name__` / `cause.constructor.name` in the peers.

### Performance


- **The format warning walk no longer degrades on nested unions or large arrays.** Deciding branch membership by compiling a validator per branch, per node, per call made a schema with 120 nested unions take ~1.2 s; a structural check brings it to ~3 ms. Deduplicating warnings was quadratic — an array producing 16 000 of them took ~900 ms, now ~19 ms.

### Changed


- Conformance: `schema_keyword_parity.json` grew to 119 cases and two new fixtures were added. No implementation change was needed for the applicator set — `validate_against_schema` hands the raw schema to the `jsonschema` crate and already matched the reference validator; the fixtures now pin that as a regression guard.

### Cross-language parity (sync findings)


The following close divergences from apcore-python and apcore-typescript found
by a side-by-side `sync` pass. Regression coverage lives in
`tests/test_sync_parity_critical.rs` and `tests/test_sync_parity_warning.rs`.

#### Added

- **`Config::get_declared(key)`** — like `get()` but WITHOUT the canonical-default fallback. `Config::validate()` uses it so a required field must be *declared*, not merely defaulted (preserving decision A-D-03 now that `get()` defaults).
- **`SchemaLoader::resolve()` / `SchemaLoader::max_ref_depth()`** — the load path now invokes `RefResolver`.
- **`RefResolver::with_schemas_dir()` / `with_current_file()` / `schemas_dir()`** — anchor and rebase cross-file `$ref` resolution.
- **`TracingConfig::sampling_rate` / `TracingConfig::exporter`** — real fields, so the `observability.tracing.sampling_rate ∈ [0.0, 1.0]` constraint is reachable.
- **`examples/execution_policy.rs`** — runnable end-to-end `ExecutionPolicy` demo, matching the Python and TypeScript examples. `examples/bindings/format_date.rs` now has an explicit `[[example]]` stanza (Cargo autodiscovers `examples/*.rs` only, so it had never been compiled).
- **`rust-version = "1.86"`** in `Cargo.toml`. The README claimed MSRV 1.75 with nothing enforcing it; the crate uses `std::task::Waker::noop()` (stable in 1.85) so 1.75 was never true.

#### Fixed

- **BREAKING: `auto_schema: strict` can now fail.** Rust's schema resolution fell back to a permissive `{"type":"object"}` pair, so `assert_openai_strict_compatible` passed *vacuously*, and `register_into_with_handlers` never called it at all. A binding whose normalized mode is `strict` is now rejected with `BINDING_SCHEMA_INFERENCE_FAILED` when no typed schema is available, matching apcore-python and apcore-typescript. The binding file path is threaded through to the error, so the `{file_path}: ` message prefix and `file_path` details key required by `DECLARATIVE_CONFIG_SPEC` §7.2 carry a real value.
- **BREAKING: `RefResolver::has_circular_refs()` agrees with `resolve()`.** It ran its own traversal that seeded an empty visited set and carried no `from_ref_chain` discriminator, so it answered `true` for every recursive schema `resolve()` accepts — including PROTOCOL_SPEC §4.15's own `TreeNode` example. It now delegates to `resolve()`, so one code path defines which re-entry is a cycle.
- **The child `Context` is derived in the non-removable `context_creation` step.** It was derived in `call_chain_guard`, which the `testing` and `minimal` presets remove — so under those presets `call_chain` never grew and depth limits, circular-call detection and frequency throttling all reset, with a wrong `caller_id` on nested calls. apcore-python and apcore-typescript both derive it in `context_creation`.
- **`Config::get()` resolves canonical defaults.** A config omitting `version`, `project.name`, `extensions.*`, `schema.*`, `acl.*`, `sys_modules.enabled` or `stream.max_merge_depth` returned `None` where both peers return the value declared in `apcore/schemas/defaults.schema.json`.
- **`observability.tracing.sampling_rate` is enforceable.** `observability` is a typed struct excluded from `user_namespaces` and `TracingConfig` had only `enabled`, so an out-of-range `sampling_rate: 5.0` was silently *accepted* (both peers reject with `CONFIG_INVALID`) and a legitimate `0.1` was discarded at deserialization.
- **The built-in `observability` namespace defaults match PROTOCOL_SPEC §9.15.2.** `metrics.exporter` was `"in_memory"` (spec/peers: `"stdout"`), `tracing.otlp_endpoint` was a live `http://localhost:4318` (spec/peers: `null` — a Rust service with tracing enabled would attempt OTLP export where its peers would not), and `logging.enabled`, `logging.redact_sensitive` and `platform_notify.enabled` were absent entirely.
- **Mounted namespaces survive `reload()`.** `mount("my-plugin", …)` followed by `reload()` dropped the mounted subtree; PROTOCOL_SPEC §9.11 requires it to persist, as it does in both peers.
- **Cross-file `$ref` formats.** The `apcore://` canonical form and the relative cross-file form required by PROTOCOL_SPEC §4.11 were absent — the fragment was never split off, so even registering the bare file path could not match. Both now resolve, anchored at a schemas root with a containment check so a reference cannot escape it.
- **A `#/…` pointer inside an external schema resolves in that schema's tree.** The root was threaded unchanged into resolved external documents, so the pointer was looked up in the *calling* document (JSON Schema 2020-12 §8.2 requires the rebase).
- **`$or` / `$not` evaluate sub-conditions in the enclosing mode.** Both were registered only in the sync registry yet delegated to the async evaluator, which consults the async registry first — so a sync `ACL::check` resolved sub-condition keys from the async registry. PROTOCOL_SPEC §6.1 requires same-mode evaluation.
- **A non-string `acl.default_effect` is reported, not coerced.** `ACL::load` silently turned `default_effect: true` into `"deny"`, so `try_new`'s validation never fired. Fail-closed either way, so this is a diagnostics fix; both peers raise `ACLRuleError`.
- **`handler_error` is populated for an unknown condition key and for a `Poll::Pending` handler.** Only the panic path recorded one, so a typo'd condition key produced a DENY with a null forensic record.
- **Discovery cannot populate the reserved `ephemeral.*` namespace.** `register_discovered` — the shared sink for `discover()` and `discover_internal()` — never checked, so a Discoverer could skip `warn_if_missing_approval` and the namespace's audit-provenance contract. PROTOCOL_SPEC:424 names `Registry::register()` as the only mechanism.
- **Streaming deep-merge preserves base-only keys at the 32-level cap.** Rust replaced the whole node; chunk A `{a:{a:…{x:1}}}` + chunk B `{a:{a:…{y:2}}}` accumulated to `{y:2}` where both peers produce `{x:1,y:2}`.
- **Per-instance `ToggleState` binds under every built-in preset.** `set_toggle_state` rebuilt `module_lookup` only for `standard`, yet all five presets derive from `build_standard_strategy()` and bind the process-global store — so `apcore.disable(module)` on one instance was silently ignored by any non-`standard` executor (issue #71).
- **`replace()` / `configure_step()` reject a replacement whose name collides with a different existing step.** The strategy could end up with two identically-named steps while `rebuild_index` kept only the last, so `skip_to` resolved past the first and `remove` targeted the wrong position. apcore-typescript already guarded this.
- **README:** the primary docs link was a 404 (mkdocs emits a `getting-started/` directory, not `getting-started.html`); the `async_task::RetryConfig` "intentionally NOT re-exported" note contradicted `src/lib.rs`, which does re-export it as `AsyncRetryConfig`; the parity matrix was still framed around "v0.23 hardening items" on a 0.26 crate; and `ExecutionPolicy` / `PolicyDecision` / `PolicyRule` were undocumented despite shipping in 0.26.0.

### Cross-language parity — spec decisions


Resolution of the sync findings that were escalated as NEEDS-SPEC-DECISION.
Regression coverage lives in `tests/test_spec_decision_followup.rs`.

#### Changed

- **BREAKING: `Registry::describe(name)` returns `Result<String, ModuleError>` and builds the cross-SDK Markdown document.** It returned the module's bare one-line `description()`, and the literal string `"Module not found"` for an unregistered ID — so a missing module looked like a successful description, and an AI agent (the primary consumer) got one line from Rust where apcore-python and apcore-typescript hand it a structured document. It now raises `MODULE_NOT_FOUND` for a missing module and otherwise emits the peers' envelope: `# {module_id}` heading, description, `**Tags:**`, a `**Parameters:**` list derived from `input_schema.properties` (with `(required)` markers), and `**Documentation:**`. A module that overrides `Module::describe()` to return a JSON **string** has that string returned verbatim — Rust has no `hasattr`, so the return *shape* is the detectable analogue of the peers' optional `describe()` member; any other shape (including the default structured object of PROTOCOL_SPEC §5.6) falls through to the generated envelope. `APCore::describe` now delegates here instead of re-implementing.

- **BREAKING: `Registry::export_schema(name, strict)` replaces both `export_schema(name)` and `export_schema_strict(name, strict)`.** The old `export_schema` returned the raw internal `{input, output}` cache entry, so a polyglot consumer reading `result["input_schema"]` — the key both peers return — silently got `null` rather than a compile error. The canonical envelope `{module_id, description, input_schema, output_schema}` (previously only reachable via `export_schema_strict`) is now what `export_schema` returns, and the raw cache accessor is gone. Rust has no default arguments, so `strict` is an explicit `bool` rather than a second method; pass `false` for the former `export_schema(name)` behaviour.

- **`Config::validate()` requires only `version` and `project.name`.** PROTOCOL_SPEC §9.1 now states the rule explicitly: a key is required **only when it has no canonical default**, and requiredness is evaluated against the *declared* document before defaults are merged. `schemas/apcore-config.schema.json` declares exactly those two. Rust additionally hard-required `extensions.root`, `schema.root` and `acl.default_effect` — all of which carry defaults in `defaults.schema.json` — so it rejected configurations both peers accept. `Config::get_declared()` (the declared-vs-defaulted mechanism from the previous batch) is unchanged; only the field list narrows. All three SDKs now accept and reject the same set of documents.

- **Removed the `global_timeout >= default_timeout` cross-field check.** `builtin_steps.rs` clamps the per-module timeout to the remaining global deadline (`if timeout_ms == 0 || remaining_ms < timeout_ms { timeout_ms = remaining_ms }`), so `global_timeout: 10000` with `default_timeout: 30000` is a meaningful configuration — "no single module over 30s, whole chain under 10s" — that the runtime already handles correctly. Neither peer nor the PROTOCOL_SPEC §9.3 constraint table has the check; Rust was rejecting a valid config.

#### Fixed

- **`remove` / `replace` / `insert_after` / `insert_before` / `configure_step` emit the step error codes that already existed.** `ErrorCode::StepNotFound`, `StepNotRemovable` and `StepNotReplaceable` were declared but never constructed; every one of these sites raised `GENERAL_INVALID_INPUT` instead. A missing step or anchor now reports `STEP_NOT_FOUND` (`ExecutionStrategy::find_step_index`), a pinned step reports `STEP_NOT_REMOVABLE` / `STEP_NOT_REPLACEABLE`, matching apcore-python and apcore-typescript exactly. The `configure_step` vs `replace` split on the *missing-step* code (`PIPELINE_STEP_NOT_FOUND` vs `STEP_NOT_FOUND`) is shared by all three SDKs and is preserved deliberately. `build_strategy_from_config` still re-classifies a missing `after` / `before` anchor as a structural configuration error (`PIPELINE_CONFIGURATION_ERROR` — see the rename above), now keyed on `StepNotFound` rather than the over-broad `GeneralInvalidInput` — so a duplicate step name in YAML is no longer mislabelled as "anchor not found".

- **The `auto_schema` permissive fallback is no longer silent.** Rust's automatic schema inference is unimplemented (F11) and `DECLARATIVE_CONFIG_SPEC` §12 now marks `auto_schema: true` / `permissive` as **not implemented** and `strict` as **partial**. Tightening the fallback into an error would break every working binding, so it stays permissive — but `resolve_schemas()` now emits a `tracing::warn!` naming the `module_id` and the binding file path, stating that inference is unimplemented and that a permissive `{"type": "object"}` is in use. One warning per binding; bindings supplying `input_schema` / `output_schema` / `schema_ref` return before this point and never warn.

## [0.26.0] - 2026-07-13

### Added

- **Execution-time governance policy (#76 RFC pilot).** New `ExecutionPolicy`, `PolicyRule`, and `PolicyDecision` types (exported from the crate root) let a platform operator override the governance annotations of already-registered modules at execution time — independent of how they were registered. A policy attaches to the `Executor` via the runtime `Executor::set_policy(Option<ExecutionPolicy>)` setter and is consulted by the approval gate (Step 5). Pattern matching reuses the ACL wildcard semantics (Algorithm A08, `utils::match_pattern`) and specificity scoring (Algorithm A10, `utils::calculate_specificity`); on a specificity tie the more restrictive rule wins. A matched rule overrides the module's own declared/scanned `requires_approval` / `destructive` annotations, and every policy-driven override is recorded in the audit trail (tracing log + span event). `ExecutionPolicy::from_value` parses a YAML/JSON governance document **strictly** — unknown keys (via serde `deny_unknown_fields`), a missing `pattern`, or an empty `pattern` error so a typo cannot silently disable a control. `Executor::validate()` preflight now reports the same `requires_approval` verdict the gate will enforce under a policy. When the gate is policy-forced, the `ApprovalRequest.annotations` handed to the handler carries the **effective** governance values, preserving the "requires_approval is guaranteed true" contract (PROTOCOL_SPEC §7).

- **Governance events on the event bus (#77 pilot).** When the `Executor` has an event emitter (`Executor::set_event_emitter`; auto-wired from the shared bus in `APCore::with_options`), the governance chain publishes three canonical events: `apcore.approval.decision` on every approval adjudication (handler decisions and the strict fail-closed rejection; severity `info` for approved/pending, `warn` for rejected/timeout), `apcore.policy.override` whenever a policy changes a module's effective governance, and `apcore.acl.denied` (severity `warn`) when an ACL check denies a call. Payloads carry `module_id`, `trace_id`, and event-specific keys (`status`/`approved_by`/`approval_id`, `pattern`/`requires_approval`/`destructive`, or `caller_id`). Canonical names are proposed in apcore#77, pending the PROTOCOL_SPEC §9.16.2 amendment. A skipped approval gate emits nothing (parity with the no-audit-log-when-skipped contract), and the `apcore.acl.denied` event is suppressed during `validate()` preflight (dry-run) so a probe never emits a spurious denial.

### Fixed

- **`Registry::register` concurrent-duplicate TOCTOU race.** Two threads registering the same module ID could both succeed: each passed the read-side conflict check before either published, and the `in_flight` guard did not cover the window where the winner published *and* cleared its `in_flight` slot while the loser was still between its early check and its own publish. The publish path now re-checks `core.modules` under the write lock, so exactly one concurrent registration wins (the rest get `DUPLICATE_MODULE_ID`) — restoring the `register_property_thread_safe_duplicate_single_winner` invariant. Surfaced when the integration tests were consolidated into fewer binaries, which run more tests concurrently in one process than the old one-binary-per-file layout.

- **`apcore.stream.post_validation_failed` is now actually emitted (#78).** A swallowed post-stream failure (output-schema validation or `middleware_after` failing after chunks were already delivered) previously only produced a `tracing::warn` — the `apcore.stream.post_validation_failed` event that apcore-python and apcore-typescript emit was a comment-only stub. `Executor` now threads its event emitter into the streaming Phase-3 path and publishes the event (payload `error_type` / `message` / `trace_id`, severity `error`) while still swallowing the failure from the chunk stream. Achieves cross-SDK observability parity for post-stream failures.

### Removed

- **Legacy dual-emission of unprefixed event names (#78).** The registry bridge and `PlatformNotifyMiddleware` no longer emit the deprecated unprefixed aliases `module_registered` / `module_unregistered` / `error_threshold_exceeded` / `latency_threshold_exceeded` alongside their canonical `apcore.registry.*` / `apcore.health.*` names. PROTOCOL_SPEC §9.16 declared these removed as of v0.22.0 (`MUST` emit only canonical names); the code had kept dual-emitting them (with a `deprecated: true` marker) for a back-compat window. An ecosystem audit found no remaining subscriber to the bare names, so the aliases are now gone — subscribers must use the canonical `apcore.<subsystem>.<event>` names (a `*` / `apcore.*` glob subscription is unaffected). Aligns Rust with the TypeScript SDK, which already emitted canonical-only.

### Changed

- **Resolve `destructive` ↔ approval semantics (#76).** `ExecutionPolicy::new(rules).with_gate_destructive(true)` makes any module whose effective `destructive` annotation is true require approval even when `requires_approval` is false — the opt-in resolution of the long-standing footgun where an inferred `DELETE` was `destructive=true` yet ungated. Orthogonality remains the default (no behavior change without a policy).

- **Approval gate fails loud, not silent (#76, security principle).** When a module needs approval but no `ApprovalHandler` is configured, the gate keeps the PROTOCOL_SPEC §7.4 skip behavior but now logs a `tracing::warn` (once per module, deduped via an executor-owned set) instead of silently continuing. `ExecutionPolicy::new(..).with_strict(true)` upgrades this to fail **closed** (returns `ErrorCode::ApprovalDenied`). A module annotated `destructive=true` that no approval gate covers is likewise warned about once per module. Existing behavior without a policy and with a handler configured is unchanged.

- **README:** bumped the install snippet to `apcore = "0.26"`.

## [0.25.0] - 2026-06-22

### Added

- **Config-driven ACL discovery (#74, D-64).** New `ACL::discover(&config) -> Result<Option<ACL>, ModuleError>` resolves `acl.root` (now defaulting to `./acl`) relative to the config file's directory and loads an ACL only when that path exists, returning `Ok(None)` otherwise — it errors solely when a *found* file is structurally invalid. An `acl.root` pointing at a directory loads `<root>/global_acl.yaml`; pointing at a file loads that file directly. CRITICAL invariant: a missing path attaches NO ACL (never a synthesized default-deny). Discovery is auto-wired in `APCore::with_options` and skipped when the caller supplies their own `Executor`. New tests plus `examples/acl_config_driven.rs`; cross-language contract locked by the apcore conformance fixture `acl_root_discovery.json`.

### Changed

- **`acl.root` is no longer hard-required (#74, D-64).** Previously omitting `acl.root` failed config validation with `CONFIG_INVALID`; it now defaults to `./acl`, matching apcore-python / apcore-typescript, so a config that omits the key is valid (and simply attaches no ACL when the default path does not exist). This is a non-breaking, more-permissive default change.
- **README:** bumped the install snippet to `apcore = "0.25"`.

---

## [0.24.0] - 2026-06-12

### Changed

- **`ToggleState` is now per-`APCore`-instance instead of process-global (#71).** Each `APCore` owns one `Arc<ToggleState>`, created fresh in `APCore::with_options`, and threads it into BOTH the execution-pipeline read path and the toggle write path. The pipeline `module_lookup` step (`BuiltinModuleLookup`) gained an `Arc<ToggleState>` field and now reads `self.toggle_state.is_disabled(id)` rather than the free `is_module_disabled()` global; the instance store is wired through `Executor::set_toggle_state` and `build_standard_strategy_with_toggle(...)`. The same `Arc` is passed to `ToggleFeatureModule` via the new `SysModulesOptions::toggle_state` field, so write and read share one store and toggles survive that instance's reload (A-D-12, re-scoped from process-global to instance-scoped). Disabling a module on one `APCore` no longer affects another instance in the same process. The free `is_module_disabled()` function keeps reading the process-global store as a fallback (unchanged signature); strategies built without an `APCore` default to that global store, preserving back-compat. New accessors: `APCore::toggle_state()` and `Executor::toggle_state()`.

### Added

- **Cross-language conformance coverage for agent governance and toggle isolation (#72).** Wired two canonical fixtures from the apcore spec repo into `tests/conformance_test.rs`: `toggle_state_isolation.json` (constructs real `APCore` instances in one process, drives each instance's toggle write path, and asserts the disabled-set via that instance's read path — proving disabling on A does not affect B and that toggles survive reload) and `acl_agent_scoping.json` (one shared default-deny ruleset scoping tool access by caller pattern + identity roles + call-chain depth; all 19 decision cases pass, locking the agent-tool-governance scenario as a cross-language contract). The depth cap is inclusive (`call_depth == max_call_depth` is allowed), matching apcore-python / apcore-typescript.

### Fixed

- **`Registry::register_module` and `register_versioned` now derive the descriptor's annotations from `module.annotations()` instead of discarding them with `ModuleAnnotations::default()`.** Previously a sensitive module declaring `requires_approval = true` registered via the canonical two-argument `register_module` ended up with `requires_approval = false` in its descriptor. Because the approval gate decides whether to fire from `registry.get_definition().annotations.requires_approval`, the gate was **silently bypassed** — a security-relevant divergence from apcore-python / apcore-typescript, whose `register` paths derive annotations from the module. Regression test: `tests/test_register_module_annotations.rs` (includes an end-to-end check that the gate now fires).
- **`APCore::on()` / `events()` now share the system event bus (D1-011 resolved).** Previously the client held a separate local `EventEmitter`, so subscribers added via `on()` never received `apcore.module.toggled`, `apcore.registry.module_registered`, or other system events — diverging from apcore-python / apcore-typescript, which expose a single bus. `EventEmitter` is now interior-mutable (`subscribers` behind a `RwLock`, so `subscribe`/`unsubscribe`/`shutdown` take `&self`), letting one shared `Arc<EventEmitter>` serve the registry (sync `emit_spawn`), the sys modules (async `emit`), and the client (`subscribe`) without an external lock. When `sys_modules` is enabled, `on()`/`events()` bind that shared bus; when disabled, a standalone bus is created lazily on first `on()`. The earlier documented plan (`Arc<Mutex<EventEmitter>>`) was unworkable because the registry emits from synchronous functions and cannot `.await` a lock. `events()` keeps its `Option<&EventEmitter>` signature (non-breaking).

#### Cross-SDK sync (2026-06-11) — Rust outlier convergence

- **Schema type coercion is now enabled by default (A-D-005/006).** `SchemaValidator::new()` defaults to `coerce_types = true` and a pydantic-lax-style pre-pass now coerces values toward the schema's declared scalar types before validation: string→integer (`"42"` → `42`, `"42.0"` → `42`), string→number (`"3.14"` → `3.14`), string→boolean (`true/false/yes/no/on/off/y/n/t/f/1/0`, case-insensitive), recursively through object `properties` and array `items`. Non-coercible values (e.g. `"abc"` for an integer, `5.5` for an integer) are left unchanged so the validator rejects them, and int→string is NOT coerced (matching pydantic). Opt out via the new `SchemaValidator::with_coerce_types(false)`. Mirrors apcore-python (`model_validate(strict=not coerce_types)`) and apcore-typescript (`Value.Decode`).
- **`SchemaValidator::validate_input` / `validate_output` now return the coerced value (A-D-017)** instead of the raw input clone, so `validate_input({"age":"42"})` returns `{"age":42}` (parity with Python `model_dump()` / TS `Value.Decode`).
- **Streaming honors the two-point cancellation invariant (A-D-002).** `Executor::stream` now checks `context.cancel_token` immediately before `module.stream()`, so a token cancelled during Phase-1 setup aborts with `ExecutionCancelled` before any chunk is yielded — matching the unary Step-8 check and Python/TS.
- **`_secret_`-prefix redaction recurses into array elements (A-D-003).** `redact_sensitive` now redacts sensitive keys in objects nested inside arrays (and nested arrays), mirroring apcore-python `_redact_in_list`.
- **`MaxCallDepthHandler` accepts integral float thresholds (A-D-005).** `max_call_depth: 5.0` is now treated as depth 5 (YAML/JSON often parse a bare integer as a float); non-integral floats (`5.5`) remain rejected. Matches apcore-typescript.
- **Legacy-mode config now applies the global env_map (A-D-007).** `Config::apply_env_overrides` consults `global_env_map()` (bare env var → dot-path) in legacy mode too, not only namespace mode — matching apcore-python (`config.py:266`) and apcore-typescript.
- **Namespace-mode `Config::get` falls back to the implicit `apcore` namespace (A-D-009).** A user key stored under `apcore.<key>` is now reachable by its bare name (§9.9.1), mirroring Python/TS.
- **Registry events are delivered-or-dead-lettered, never silently dropped (A-D-013).** The registry now dispatches via the DLQ-bearing path (retry + `apcore.event.delivery_failed`) instead of `emit_spawn` (single attempt, silent drop), matching the Python/TS registries.
- **Middleware `on_error` fires exactly once and honors `RetrySignal` (A-D-010/012).** Removed the step-level `on_error` recovery from the before/execute/after pipeline steps so the executor-level loop is the SOLE recovery site. Previously an after()-failure fired `on_error` twice (step + executor) and step-level recovery silently dropped `RetrySignal` (mapped to `None`). The executor handles both Recovery and Retry. Matches apcore-python / apcore-typescript.
- **Mid-stream errors run the middleware `on_error` recovery chain (A-D-015).** A non-cancellation error raised while iterating chunks now runs `execute_on_error_outcome` over the executed middlewares and yields a recovery value before the stream ends (Retry is ignored mid-stream and the original error surfaces), matching Python (`executor.py:1069`) and TS.
- **Middleware identity is recorded on first registration regardless of `allow_duplicate` (A-D-020).** `MiddlewareManager::add_with_opts` now always records the identity on first sight; only the duplicate warning is suppressed by `allow_duplicate`. Previously a first `allow_duplicate(true)` registration was never recorded, so a later duplicate went undetected. Matches Python/TS. Added `MiddlewareManager::identity_registered`.
- **README:** bumped the install snippet to `apcore = "0.24"` and fixed the `ModuleDescriptor` import (it is re-exported from `apcore::registry`, not `apcore::module`).

---

## [0.23.0] - 2026-06-10

### Added

- **AI error-recovery metadata is now populated at the source (#70).** Framework-deterministic errors carry a default `user_fixable` resolved from the error code via the new `user_fixable_for_code()` policy wired into `ModuleError::new`, so the recovery contract flows to every surface (MCP/CLI/A2A) from one definition. `Some(true)` for caller-fixable codes (`SCHEMA_VALIDATION_ERROR`, `GENERAL_INVALID_INPUT`, `MODULE_NOT_FOUND`, `VERSION_CONSTRAINT_INVALID`, `BINDING_SCHEMA_INFERENCE_FAILED`, `BINDING_SCHEMA_MODE_CONFLICT`, `BINDING_STRICT_SCHEMA_INCOMPATIBLE`, `DEPENDENCY_NOT_FOUND`, `DEPENDENCY_VERSION_MISMATCH`); `Some(false)` for governance/system/structural/transient codes (`ACL_DENIED`, `APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `MODULE_TIMEOUT`, `MODULE_DISABLED`, `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL`, `CALL_FREQUENCY_EXCEEDED`, `GENERAL_INTERNAL_ERROR`); unlisted codes (e.g. `MODULE_EXECUTE_ERROR`) stay `None` for the module author to supply. The `with_user_fixable` builder overrides the policy. Default `ai_guidance` filled for the invalid-input and call-frequency errors. Serialization stays sparse (`skip_serializing_if`). Locked by the shared conformance fixture `error_recovery_metadata.json` — at parity with apcore-python / apcore-typescript 0.23.0.


### Changed

- **`ApprovalRequest` annotations / description / tags are now sourced from the resolved live module instance (per PROTOCOL_SPEC §7.4) [D10-003].** `BuiltinApprovalGate` previously read these fields from the registry's `ModuleDescriptor`; it now reads them from the live module (`module.annotations()` / `.description()` / `.tags()`), matching apcore-python and apcore-typescript. Added a `Module::annotations()` trait method (default impl returns `ModuleAnnotations::default()`, so existing `impl Module` blocks are unaffected). The gate still decides whether to fire from the descriptor's `requires_approval` flag.


### Fixed

- **`A2ASubscriber` no longer retries 4xx responses (#69).** It previously returned `Err` on any HTTP `status >= 400`, contradicting the spec (`event-system.md`: 4xx MUST NOT be retried, for both Webhook and A2A) and diverging from `WebhookSubscriber`. `A2ASubscriber::on_event` now mirrors Webhook: 5xx (and connection/timeout) → `Err` → retried → `apcore.event.delivery_failed` on exhaustion; 4xx → logged permanent, `Ok` (no retry, no DLQ). Per-SDK regression tests lock both subscribers' 4xx/5xx behavior.


## [0.22.0] - 2026-05-28

### Changed

- **`Context::create` unified to the canonical six-parameter signature ([apcore Issue #66](https://github.com/aiperceivable/apcore/issues/66)).** The factory now takes `(identity, trace_parent, cancel_token, data, services, global_deadline)` matching the cross-language contract in [docs/features/core-executor.md §"Contract: Context.create"](https://github.com/aiperceivable/apcore/blob/main/docs/features/core-executor.md). The legacy `caller_id` parameter is removed — top-level Contexts always carry `caller_id = None`, which is managed exclusively by `Context::child()`. `cancel_token` and `global_deadline` are now first-class parameters, eliminating the post-hoc `ctx.cancel_token = Some(token)` anti-pattern. Pre-release breaking change; all in-tree call sites and examples are migrated.

- **`TraceParent` carries `tracestate` as a struct field.** Per the unified `Context::create` contract, the inbound W3C `tracestate` (vendor state) travels on `TraceParent` itself rather than via a separate factory parameter or builder setter. `TraceContext::extract_context` and `ContextBuilder::build` populate the field; `ContextBuilder::tracestate(entries)` is removed (replaced by setting `TraceParent::tracestate` directly). `ContextBuilder` gains a `cancel_token(...)` setter for parity with `Context::create`.

- **Executor binds itself to `Context::executor` at pipeline entry** ([docs/features/core-executor.md §"Contract: Executor binding to Context"](https://github.com/aiperceivable/apcore/blob/main/docs/features/core-executor.md)). `Executor::call`, `Executor::call_with_trace`, `Executor::validate`, and `Executor::stream` (via `prepare_stream`) bind the receiving Executor's identity handle to `ctx.context.executor` before pipeline step 1. Same-instance rebinds are idempotent; cross-Executor rebinds raise `ContextBindingError` (new `ErrorCode::ContextBindingError` / wire code `CONTEXT_BINDING_ERROR`). A new `Context::bind_executor(...)` helper exposes the binding rule for distributed runtimes that re-inject a Context after deserialization. A new SDK-internal `Executor::instance_handle()` accessor returns the type-erased handle compared by `Arc::ptr_eq`.

- **`EventRetryConfig::default()` is now spec-aligned (closes A-D-EVT-004).** Previously defaulted to single-attempt (`max_attempts = 1`) for backward compatibility; now defaults to `max_attempts=3, initial_backoff_ms=100, max_backoff_ms=30_000, backoff_multiplier=2.0` per [docs/features/event-system.md §Event Delivery Semantics (#61)](https://github.com/aiperceivable/apcore/blob/main/docs/features/event-system.md). Callers that genuinely want fire-and-forget single-attempt semantics should switch to the new `EventRetryConfig::no_retry()` helper.

- **`EventEmitter::emit()` and `emit_filtered()` now apply per-subscriber retry + DLQ inline (closes A-D-EVT-003).** The canonical await-completion delivery path runs each matching subscriber through the retry loop defined by `EventSubscriber::retry()` and emits `apcore.event.delivery_failed` on exhaustion (when `max_attempts > 1`). A new `emit_sequential()` helper preserves the legacy single-attempt sequential-dispatch shape for tests that need deterministic ordering without retry/DLQ noise. `emit_delivery_semantics()` (spawned fire-and-forget) and `emit_spawn()` (no-retry spawn) are retained for callers that cannot await delivery.

- **`WebhookSubscriber` and `A2ASubscriber` no longer swallow transient failures (closes A-D-EVT-001).** Both subscribers now return `Err(ModuleError)` from `on_event` on 5xx / network errors (and `>=400` for A2A), letting the surrounding `EventEmitter` apply the spec retry policy + DLQ uniformly. The ad-hoc internal retry loop on `WebhookSubscriber` is removed — `retry_count` is retained as a deprecated alias for `retry.max_attempts` and is no longer consulted on the delivery path. Both structs gain a public `retry: EventRetryConfig` field and a `with_retry()` builder; their `EventSubscriber::retry()` returns that field so per-subscriber policy flows through `EventEmitter` automatically.

- **`BuiltinExecute` honors per-module `resources.timeout` (closes A-D-EXEC-001 / D-11).** Reads `annotations.extra["resources"]["timeout"]` from the module descriptor before falling back to `config.executor.default_timeout`. The global deadline clamp applies after this lookup so per-module overrides cannot exceed the remaining global budget.

- **Cancel-token observed at two pipeline points (closes A-D-EXEC-002 / D-21).** `BuiltinCallChainGuard` calls `cancel_token.check_for(module_id)` before validation/ACL/middleware work, and `BuiltinExecute` repeats the check immediately before `module.execute` as a defensive backstop. A pre-cancelled token short-circuits the pipeline in microseconds; a token cancelled mid-pipeline is observed at the execute step.

- **`Executor::call` short-circuits on `ExecutionCancelled` (closes A-D-EXEC-003 / D-20).** Cancellation now propagates directly from the pipeline error path; it MUST NOT enter the `on_error` middleware chain, so logging middleware cannot swallow cancellation and retry middleware cannot resurrect a cancelled call via `RetrySignal`.

- **`Executor::call_with_trace` shares error-recovery semantics with `call` (closes A-D-EXEC-004 / D-19).** The trace variant now runs `on_error` middleware recovery, applies the cancellation short-circuit, and unwraps `MiddlewareChainError` identically to `call`. On successful recovery it returns `(recovered_value, trace)`.

- **`BuiltinContextCreation` preserves per-call context fields.** Previously replaced `ctx.context` with a fresh `Context::new(...)` when `caller_id` was `None`, silently dropping caller-supplied `cancel_token`, `global_deadline`, and `data`. Now mutates `caller_id`/`identity` in place so D-21 cancel-token checks (and other per-call resources) flow through unchanged.

- **`AsyncTaskManager.max_tasks` counts only active statuses (closes A-D-AT-01).** `check_capacity_and_save` filters `store.list(None)` to `Pending` + `Running` records so terminal-state tasks retained for TTL-based cleanup do not consume the active budget. Mirrors apcore-python `_ACTIVE_STATUSES`.

- **`AsyncTaskManager::start_reaper` is single-instance (closes A-D-AT-05).** A second `start_reaper` call while a prior `ReaperHandle` is still live now returns `Err(ModuleError { code: ReaperAlreadyRunning, .. })`. `ReaperHandle::stop` (and the `Drop` fallback) releases the manager's `reaper_running` flag so callers can stop-and-restart. Return type changes from `ReaperHandle` to `Result<ReaperHandle, ModuleError>` — callers must add `?` or `.unwrap()`.

- **`Executor::stream` Phase 1 now runs `on_error` middleware recovery (closes A-D-001).** A pre-execute (Phase-1) pipeline failure in `stream()` previously short-circuited via `?` without invoking the middleware `on_error` chain, so a recovery middleware that fires for the same failure in `call()` was silently bypassed in the streaming path. `prepare_stream` now mirrors `call()`'s error handling — unwrap `PipelineStepError` → unwrap `MiddlewareChainError` → `propagate_module_error` (A11) → `ExecutionCancelled` short-circuit (D-20) → run `execute_on_error_outcome` over the executed middlewares — yielding any recovery value as the stream's chunk; a `RetrySignal` is logged and the original error surfaces (retry is not meaningful mid-stream). Pipeline aborts still hard-error, matching `call()`. Aligns with apcore-python and apcore-typescript. Found via `/apcore-skills:sync --scope core`.

- **`ACL::check` (sync) now records `handler_error` in the audit entry (closes A-D-002).** A-D-ACL-001 added handler-error capture for the async path (`async_check` via `tokio::task_local!`), but synchronous `check()` ran `report_handler_error` outside any capture scope (a no-op), so `AuditEntry.handler_error` was always `None` on the sync path when a condition handler errored. A parallel thread-local capture (`HANDLER_ERROR_SYNC`) plus a `with_handler_error_capture_sync` wrapper now records the handler error on the sync path too, matching `async_check` and the Python/TypeScript SDKs. The access decision was already fail-closed and is unchanged. Found via `/apcore-skills:sync --scope core`.

#### Cross-language review (`/apcore-skills:sync --scope core`, 2026-05-26)

- **BREAKING: `EventEmitter::emit()` is now non-blocking (A-D-024).** `emit()` previously awaited the full per-subscriber retry loop (including backoff sleeps) inline, blocking the caller until delivery to all matching subscribers completed. It now spawns each subscriber's delivery on its own `tokio::task` and returns immediately, matching apcore-python (`ThreadPoolExecutor.submit`) and apcore-typescript (fire-and-forget). This supersedes the prior "emit applies retry + DLQ inline" behavior recorded above for A-D-EVT-003. Callers that need to wait for delivery MUST call `flush()`. `emit_sequential()` (deterministic single-attempt) and `emit_filtered()` retain inline delivery.
- **BREAKING: call-chain guard contract aligned to the canonical chain-includes-self form (A-D-039 / A-D-040).** `guard_call_chain_with_repeat` now treats `ctx.call_chain` as already including the current `module_id` at the end (the executor's `BuiltinCallChainGuard` calls `Context::child()` *before* guarding), matching apcore-python/typescript and the `call_chain.json` conformance fixture. Frequency detection now uses `count > max_module_repeat` over the full chain (was `count >= max_module_repeat`): a module appearing exactly `max_module_repeat` times (default 3) is now allowed; one more throws `CALL_FREQUENCY_EXCEEDED`. Circular detection always strips the trailing self-entry.
- **BREAKING: schema `$ref` depth-cap exhaustion now raises `SCHEMA_MAX_DEPTH_EXCEEDED` (A-D-038).** `RefResolver::resolve` previously raised `SCHEMA_CIRCULAR_REF` both for actual cycles and for hitting `max_depth`. Depth exhaustion now raises the distinct `ErrorCode::SchemaMaxDepthExceeded` (wire `SCHEMA_MAX_DEPTH_EXCEEDED`); genuine cycles still raise `SCHEMA_CIRCULAR_REF`. Cross-SDK note: apcore-python/typescript still report `CIRCULAR_REF` for the depth cap — Rust is canonical here and they should follow.
- **`EventEmitter::flush()` now awaits pending deliveries (A-D-027).** Previously a no-op. With non-blocking `emit()`, `flush(timeout_ms)` now tracks the spawned delivery tasks and awaits them up to the timeout (0 = wait indefinitely), matching apcore-python's per-future wait. `flush()` is now `async` (was sync). `shutdown()` awaits `flush()`.
- **DLQ event emitted on every delivery exhaustion (A-D-025).** `deliver_with_dlq` previously emitted `apcore.event.delivery_failed` only when `retry.max_attempts > 1`. It now emits on every exhaustion, including single-attempt (`no_retry`) subscribers, matching apcore-python/typescript.
- **Wildcard `'*'` subscribers excluded from DLQ delivery (A-D-026).** DLQ (`apcore.event.delivery_failed`) events are no longer delivered to catch-all `'*'` subscribers, preventing cascading failures where every wildcard subscriber recursively receives a DLQ about itself. Matches apcore-python.
- **DLQ `subscriber_type` uses the declared type (A-D-029).** New `EventSubscriber::subscriber_type()` trait method (default `"subscriber"`); built-in subscribers declare `"webhook"`/`"a2a"`/`"file"`/`"stdout"` and `FilterSubscriber` delegates. DLQ payloads now report this declared type instead of parsing the first dash-delimited segment of `subscriber_id`.
- **`Registry::get_definition` propagates the empty-id error (A-D-001).** `get_definition("")` previously returned `None`; it now returns `Err(ModuleError(ModuleNotFound))`, mirroring `Registry::get("")`. Return type changes from `Option<ModuleDescriptor>` to `Result<Option<ModuleDescriptor>, ModuleError>` (it routes through `get()`); all in-tree call sites are migrated.
- **Streaming inter-chunk deadline comparator normalized to `>` (A-D-007).** `Executor::stream` now aborts when `now > global_deadline` (was `>=`), matching apcore-python (`time.monotonic() > deadline`) and apcore-typescript (`Date.now() > deadline`). The internal unit remains epoch-seconds (set+compared consistently); the cross-SDK clock-unit difference is an accepted internal-contract divergence and is not changed.
- **Schema validation conformance fixture code aligned (cross-repo).** `test_schema_hardening_conformance` now accepts the canonical `SCHEMA_VALIDATION_ERROR` wire code (the spec fixtures were updated from the legacy `SCHEMA_VALIDATION_FAILED` spelling).

##### Deferred

- **A-D-016 / A-D-030 — instance-identity removal (trait-object constraint).** `MiddlewareManager::remove` (by `name()`) and `EventEmitter::unsubscribe` (by `subscriber_id()`) match by string identity rather than the instance identity used by apcore-python/typescript. Rust stores these as `Box`/`Arc<dyn Trait>` whose pointer identity is not preserved across the `Box`→`Arc` conversion, so instance matching is not feasible without a handle-returning API. Documented as the accepted Rust constraint (A-D-401); callers must assign unique names/ids.

### Added

- **`EventRetryConfig::no_retry()` helper.** Returns single-attempt configuration for fire-and-forget subscriber semantics.
- **`ACL::register_async_condition` writes to a separate registry (closes A-D-ACL-002).** New `acl_handlers::ASYNC_CONDITION_HANDLERS` `LazyLock` map is consulted by `evaluate_conditions_async` before the sync `CONDITION_HANDLERS` registry, enabling async-only handler overrides without disturbing `ACL::check`.
- **`AuditEntry.handler_error` populated via `tokio::task_local!` (closes A-D-ACL-001).** New `acl_handlers::HANDLER_ERROR` task-local slot, `report_handler_error()` setter, and `with_handler_error_capture()` scope wrapper. `ACL::async_check` runs the entire evaluation inside a fresh capture scope so concurrent ACL checks on different tokio tasks see only their own error messages.
- **`ErrorCode::ReaperAlreadyRunning`.** Raised by `AsyncTaskManager::start_reaper` when a reaper is already running.

---

### Added

- **`Config::reserved_namespaces()` associated function + `pub const RESERVED_NAMESPACES` (PROTOCOL_SPEC §9.9.5, [apcore#60](https://github.com/aiperceivable/apcore/issues/60)).** Implements the new normative requirement that all SDKs MUST expose a public, read-only query API returning the set of reserved top-level namespace names. The private `const RESERVED_NAMESPACES: &[&str]` is promoted to `pub const` and re-exported from the crate root; the new associated function returns it via `&'static [&'static str]`. Single source of truth — `Config::register_namespace` continues to consult the same slice and return `ErrorCode::ConfigNamespaceReserved` for any name it contains. Callable without a `Config` instance. Intended for third-party consumers (custom CLIs, framework integrations) that accept user-supplied namespace names and need fail-fast pre-validation. Idiomatic Rust slice (not `HashSet`) — the small compile-time-known set makes `.contains(&name)` trivially efficient and avoids runtime initialisation overhead.

- **`ContextKey<T>` promoted to documented public API ([apcore#63](https://github.com/aiperceivable/apcore/issues/63)).** `ContextKey<T>` and all six framework built-in key constants (`TRACING_SPANS`, `TRACING_SAMPLED`, `METRICS_STARTS`, `LOGGING_START`, `REDACTED_OUTPUT`, `RETRY_COUNT_BASE`) are now exported at the crate root and fully spec-aligned. The `scoped(suffix)` sub-key factory is part of the stable API.

- **`StreamingModule` trait + `Module::as_streaming()` accessor ([apcore#62](https://github.com/aiperceivable/apcore/issues/62)).** New `StreamingModule` supertrait with `stream_typed()` provides a typed handle for adapter/bridge code that needs to call the streaming path directly. Default `Module::as_streaming() -> Option<&dyn StreamingModule>` returns `None` so non-streaming modules need no changes. `Registry::register` enforces the consistency invariant: `annotations.streaming = true` without a `StreamingModule` impl returns `Err(StreamingInterfaceMismatch)`.

- **Duplicate middleware detection via `MiddlewareManager::add_with_opts()` ([apcore#64](https://github.com/aiperceivable/apcore/issues/64)).** `MiddlewareRegistration<M>` builder with `.allow_duplicate(bool)` and `.identity_key(impl Into<String>)` options. Default identity is `std::any::type_name::<M>()`. A second registration with the same identity emits `tracing::warn!` naming the first and duplicate registration sites (captured with `#[track_caller]`). Registration always succeeds; the middleware chain always includes all added middlewares.

- **Unified event delivery semantics — retry + DLQ + `on_failure` ([apcore#61](https://github.com/aiperceivable/apcore/issues/61)).** New `EventRetryConfig` struct with `max_attempts`, `initial_backoff_ms`, `max_backoff_ms`, and `backoff_multiplier`. New `EventSubscriber::retry()` and `EventSubscriber::on_failure()` default trait methods. New `EventEmitter::emit_delivery_semantics()` fire-and-forget method spawns one `tokio::task` per subscriber, runs the per-subscriber retry loop, and emits a `apcore.event.delivery_failed` DLQ event on exhaustion (when `max_attempts > 1`). Default behavior (`max_attempts = 1`) is unchanged — no retry, no DLQ, backward-compatible. `A2ASubscriber` gains a configurable `skill_id` field (default `"apevo.event_receiver"`).

- **Registry deferred-publish: module not visible until `on_load` completes ([apcore#65](https://github.com/aiperceivable/apcore/issues/65)).** `Registry::register()` now follows deferred-publish ordering: the module ID is reserved in an `in_flight` set, `on_load()` runs without any lock held, and the module is inserted into `core.modules` (visible) only after `on_load` succeeds. This guarantees `get()` / `list()` return `None` during `on_load`. Concurrent registration of the same ID returns `Err(DuplicateModuleId)`. Distinct-ID registrations run `on_load` in parallel (per-OS-thread). `Registry::on_load_failed()` registers callbacks invoked when `on_load` returns `Err`. Same deferred-publish invariant applied to `register_discovered`.

### Changed

- **`A2ASubscriber` retry behaviour ([apcore#61](https://github.com/aiperceivable/apcore/issues/61)).** `A2ASubscriber` now participates in the unified retry/DLQ path via `EventEmitter::emit_delivery_semantics()`. The hardcoded `"apevo.event_receiver"` skill ID is now a configurable `skill_id` field.

- **`ErrorCode::DuplicateModuleId` ([apcore#65](https://github.com/aiperceivable/apcore/issues/65)).** New error code used for exact-duplicate module-ID registration (both from `detect_id_conflicts` and from concurrent in-flight detection), replacing the previous `GeneralInvalidInput` code for the duplicate case. Cross-language: Python/TS `DUPLICATE_MODULE_ID`.

---

## [0.21.0] - 2026-05-06

Aligns apcore-rust with PROTOCOL_SPEC.md v0.21.0 (apcore commit
[`c191b85`](https://github.com/aiperceivable/apcore/commit/c191b85) — RFCs
`docs/spec/rfc-preview-method.md` and `docs/spec/rfc-ephemeral-modules.md`
promoted to `Accepted`). Mirrors the
[apcore-python](https://github.com/aiperceivable/apcore-python) commit
`203a9a6` (Stage 2 reference impl) and the
[apcore-typescript](https://github.com/aiperceivable/apcore-typescript) commit
`577b09b` (Stage 3 reference impl).

### Added

- **`Module::preview()` optional trait method (PROTOCOL_SPEC §5.6).** Returns
  `Option<PreviewResult>` describing structured predictions of state changes
  the call would produce. Default impl returns `None`. MUST NOT have side
  effects. Mirrors `apcore-python Module.preview` and
  `apcore-typescript Module.preview?`.
- **`Change` and `PreviewResult` structs (PROTOCOL_SPEC §12.8).** Both
  `#[non_exhaustive]` with `Default` impls. `Change` uses the Rust idiomatic
  `^x-` extension encoding from RFC `rfc-preview-method.md`'s "Change.x-*
  extension fields" cross-SDK table: `#[serde(flatten)] extra: HashMap<String,
  Value>` paired with a constructor-time validator that rejects unknown
  non-`x-` keys. `PreflightResult.predicted_changes: Vec<Change>` field
  added; `PreflightCheckResult.check` enum extended with `module_preview`.
- **`Executor::validate()` wiring of `preview()`.** Invoked after the
  standard validation pipeline; the result is folded into
  `PreflightResult.predicted_changes`. Panics during `preview()` are caught
  via `catch_unwind` and surfaced as warnings on a `module_preview` check
  entry — validation does NOT fail. Mirrors `preflight()` exception
  semantics (RFC Open Question 1).
- **`ephemeral.*` namespace reservation (PROTOCOL_SPEC §2.5 / RFC
  `rfc-ephemeral-modules`).** New exported `EPHEMERAL_NAMESPACE_PREFIX`
  constant and `is_ephemeral_module_id()` helper. Filesystem discovery
  (`default_discoverer`) rejects any module ID falling under the
  `ephemeral.*` namespace with a clear error pointing the caller to
  `Registry::register()`. Reserved for programmatically-registered modules
  synthesized at runtime (Agent-synthesized tools, on-the-fly composition).
- **`ModuleAnnotations.discoverable: bool` (PROTOCOL_SPEC §4.4).** Defaults
  to `true`. When `false` the module is hidden from `Registry::list()` /
  `iter()` / `module_ids()` but remains callable by exact ID. Pass
  `include_hidden=true` to enumerate every registered module. `ephemeral.*`
  modules SHOULD set `discoverable: false`.
- **Audit-event single-emit rule for `ephemeral.*` registrations.** For
  `ephemeral.*` modules, exactly one canonical
  `apcore.registry.module_registered` / `module_unregistered` event is
  emitted with the D-35 contextual payload (`caller_id` defaulting to
  `"@external"`, optional `identity` snapshot, `namespace_class:
  "ephemeral"`). The `RegistryEvents` callback bridge short-circuits on
  `ephemeral.*` IDs so the empty-payload bridge emit does not double-fire.
  Non-ephemeral modules retain the existing empty-payload bridge behavior
  verbatim.
- **Soft-warning when an `ephemeral.*` module is registered without
  `requires_approval: true`.** `tracing::warn!(...)` per the RFC
  ("agent-synthesized modules SHOULD declare `requires_approval: true` so a
  human gates execution"). Registration is never refused — warning only.
- **`Registry::register_internal()` rejects `ephemeral.*` IDs.** Returns
  `Err(ModuleError::InvalidInput)` pointing the caller to
  `Registry::register()`. Per the RFC's "register_internal() interaction"
  rule: namespace prefix → registration mechanism is a 1:1 mapping;
  `system.*` only via `register_internal()`, `ephemeral.*` only via
  `register()`.

### Changed

- **`PreflightResult` extended with `predicted_changes` field.** Already
  marked `#[non_exhaustive]` in v0.20.x via [#24], so the addition is
  forward-compatible: external callers using the
  `Default::default()` + mutation pattern continue to compile unchanged.

### Lifecycle

- **Caller-managed.** `ephemeral.*` modules live until the caller explicitly
  calls `Registry::unregister(module_id)`. There is no TTL sweeper or
  background GC — TTL-driven cleanup is deferred to a v2 follow-up if
  leakage is observed in practice.

[apcore-c191b85]: https://github.com/aiperceivable/apcore/commit/c191b85
[apcore-rfc-preview]: https://github.com/aiperceivable/apcore/blob/main/docs/spec/rfc-preview-method.md
[apcore-rfc-ephemeral]: https://github.com/aiperceivable/apcore/blob/main/docs/spec/rfc-ephemeral-modules.md

### Changed

- **Spec-derived public structs marked `#[non_exhaustive]`** ([#24]). The
  following protocol-derived public structs are now `#[non_exhaustive]` so the
  SDK can add fields in future minor versions (e.g. the `predicted_changes`
  field proposed in the upstream `preview()` RFC) without breaking downstream
  consumers. **No behavior change** — this is forward-compatibility hygiene
  only. All affected structs now also implement `Default` so callers can
  construct them via mutation.

  Marked structs:

  - `apcore::module::PreflightResult` (`src/module.rs`)
  - `apcore::module::PreflightCheckResult` (`src/module.rs`)
  - `apcore::module::ValidationResult` (`src/module.rs`)
  - `apcore::module::ModuleExample` (`src/module.rs`)
  - `apcore::approval::ApprovalRequest` (`src/approval.rs`)
  - `apcore::approval::ApprovalResult` (`src/approval.rs`)
  - `apcore::config::ExecutorConfig` (`src/config.rs`)
  - `apcore::config::ObservabilityConfig` (`src/config.rs`)
  - `apcore::config::TracingConfig` (`src/config.rs`)
  - `apcore::config::MetricsConfig` (`src/config.rs`)
  - `apcore::async_task::RetryConfig` (`src/async_task.rs`)
  - `apcore::async_task::ReaperConfig` (`src/async_task.rs`)
  - `apcore::async_task::TaskInfo` (`src/async_task.rs`)
  - `apcore::middleware::RetryConfig` (`src/middleware/retry.rs`)

  **Migration for downstream consumers.** Per Rust's `#[non_exhaustive]`
  semantics, struct-literal construction from outside this crate is no longer
  permitted (E0639), even with functional record update (`..Default::default()`)
  syntax. Use `Default` + mutation instead:

  ```rust
  // Before:
  let r = PreflightResult { valid: true, checks: vec![], requires_approval: false };

  // After:
  let mut r = PreflightResult::default();
  r.valid = true;
  // r.checks and r.requires_approval default to empty / false
  ```

  Pattern matching against these structs from outside the crate must also use
  a trailing `..` rest pattern.

  `ApprovalRequest::module_id`, `ApprovalResult::status`, `TaskInfo::task_id`,
  and `TaskInfo::module_id` default to empty strings — callers SHOULD set them
  explicitly. `Default` is provided purely so downstream code can use the
  mutation-after-default construction pattern.

[#24]: https://github.com/aiperceivable/apcore-rust/issues/24

---

## [0.20.0] - 2026-05-05

### Added

- `UsageExporter` async trait + `NoopUsageExporter` + `PeriodicUsageExporter` for push-style usage summary export (#45 §3, parity with PY+TS).

### Changed

- `DEFAULT_SENSITIVE_KEYS` expanded to canonical 16-entry superset matching Python+TS (#43 §5, D-54).

### Cross-SDK Sync Alignment

#### Added

- **`OverridesStore` trait + `InMemoryOverridesStore` / `FileOverridesStore`
  reference impls** (sync finding CRITICAL #1). The runtime overrides layer
  is now a pluggable async trait with `load()` and `save()` methods, allowing
  callers to swap in custom backends (Redis, S3, in-memory test fakes).
  `SysModulesOptions::overrides_store: Option<Arc<dyn OverridesStore>>` is
  threaded through `register_sys_modules_with_options` and wired into
  `UpdateConfigModule` / `ToggleFeatureModule`. When set, the store takes
  precedence over the legacy `overrides_path`. Aligns with `apcore-python`
  and `apcore-typescript`. Re-exported from the crate root as
  `OverridesStore`, `InMemoryOverridesStore`, `FileOverridesStore`,
  `OverridesError`.
- **`RetryConfig::compute_delay_ms`** (sync finding CRITICAL #2 / D-08).
  Canonical name for the retry-delay calculation, matching PY/TS. The legacy
  `delay_for_attempt` is retained as a `#[deprecated(since = "0.21.0")]`
  alias that delegates to `compute_delay_ms`; it will be removed in the next
  minor.
- **`TraceContext::inject_checked`** (sync finding W-6). Validating variant
  of `inject_with_options` that returns `ErrorCode::GeneralInvalidInput` when
  a caller-supplied `parent_id` does not match `^[0-9a-f]{16}$`, instead of
  silently falling back to a random value. Matches PY/TS behaviour.
- **`TRACE_FLAGS_KEY` constant** (sync finding CRITICAL #3). Public string
  constant `"_apcore.trace.flags"` that names the context-data slot used for
  inbound `trace-flags` propagation.
- **`ErrorCode::ConfigurationError`** (sync finding W-7). Distinct error
  code for structural pipeline-configuration errors (missing-step in
  `remove`/`configure`, missing `after`/`before` anchor) — keeps these cases
  disambiguated from `PipelineDependencyError` (reserved for `requires` /
  `provides` graph violations).

#### Changed

- **`TraceContext::inject` propagates inbound `trace-flags`** (sync finding
  CRITICAL #3). When `context.data` carries a value at `TRACE_FLAGS_KEY`
  (e.g. `"00"` or `"01"`), `inject` uses that as the outbound `trace-flags`
  byte instead of always emitting `0x01`. The default of `0x01` (sampled) is
  preserved when no inbound flags are present. `ContextBuilder::build` seeds
  the key automatically when a `trace_parent` is supplied, so transports
  that build contexts via the canonical builder get propagation for free.
  Matches `apcore-python._TRACE_FLAGS_KEY` semantics.
- **Pipeline configuration errors use `ConfigurationError`** instead of
  `PipelineConfigInvalid` for missing-step / missing-anchor cases (sync
  finding W-7). Tests that previously matched `PipelineConfigInvalid` should
  also accept `ConfigurationError`.

### Event-Naming Standardization & Contextual Auditing

#### Changed

- **Canonical event names (Issue #36).** The four threshold / registry
  events that previously emitted under bare names now also emit under their
  canonical `apcore.<subsystem>.<event>` names. Both names are dispatched on
  every occurrence so existing subscribers continue to fire while consumers
  migrate; the legacy events carry `deprecated: true` and a
  `canonical_event` pointer in their payload.
  | Legacy (still emitted)         | Canonical                                       |
  | ------------------------------ | ----------------------------------------------- |
  | `module_registered`            | `apcore.registry.module_registered`             |
  | `module_unregistered`          | `apcore.registry.module_unregistered`           |
  | `error_threshold_exceeded`     | `apcore.health.error_threshold_exceeded`        |
  | `latency_threshold_exceeded`   | `apcore.health.latency_threshold_exceeded`      |
- **Registry hooks now emit `ApCoreEvent`s.** `module_registered` /
  `module_unregistered` were previously logged via `tracing::info!` only;
  they are now full `ApCoreEvent`s dispatched through the `EventEmitter`,
  so subscribers can pattern-match against `apcore.registry.*` (Issue #36).
- **Audit-event payloads include caller identity (Issue #45.2).** The
  `apcore.config.updated`, `apcore.module.toggled`, and
  `apcore.module.reloaded` events now embed `caller_id` (defaulted to
  `"@external"` when absent) and `actor_id` / `actor_type` extracted from
  the `Context`. Aligns the Rust SDK with `apcore-python` and
  `apcore-typescript`'s contextual-audit behaviour.

### Pipeline StepMiddleware + fail-fast configuration (Issue #33)

#### Added

- **`StepMiddleware` trait** — formal step-scoped interceptor with default-method
  async hooks `before_step`, `after_step`, and `on_step_error`. Multiple
  middlewares run in registration order in the before phase and may recover
  from a step failure by returning `Ok(Some(value))` from `on_step_error`.
  Register via `ExecutionStrategy::add_step_middleware(Arc::new(...))`. Mirrors
  apcore-python `step_middleware` (Issue #33 §2.2). The trait is re-exported
  from the crate root.
- **`ErrorCode::PipelineDependencyError`** — new error variant returned from
  `ExecutionStrategy::new` / `insert_after` / `insert_before` when a step's
  declared `requires` field is not produced by any preceding step's `provides`
  (Issue #33 §2.1). Cross-language: Python/TS `PIPELINE_DEPENDENCY_ERROR`.

#### Changed

- **`ExecutionStrategy` dependency validation** is now **fail-fast at
  construction** rather than emitting a `tracing::warn!`. Strategies with
  unmet `requires`/`provides` declarations now return
  `Err(PipelineDependencyError)` from the constructor. Pipelines that already
  satisfy their declarations are unaffected (Issue #33 §2.1).
- **`build_strategy_from_config`** now returns
  `Err(PipelineConfigInvalid)` when the YAML `pipeline.remove`,
  `pipeline.configure`, or `pipeline.steps` section references a step that
  does not exist or omits both `after`/`before` anchors. Previously these
  conditions logged a warning and silently dropped the directive (Issue #33
  §1.2). This is a behaviour change: configurations that previously
  surfaced only a log warning will now refuse to construct the strategy.

### Cross-Language Sync — Storage Backend & Multi-Alignment Fixes

This batch introduces the protocol-canonical `StorageBackend` abstraction
(Issue #43 §1), wires it through the three observability collectors as an
optional persistence surface, and resolves five additional cross-language
alignment findings (D-14, D-58, D-25, D-27, D-28). Behavior of the streaming
chunk-merge path now surfaces a structured `STREAM_CHUNK_NOT_OBJECT` error,
the runtime config-key policy emits a distinct `CONFIG_KEY_RESTRICTED`
ErrorCode, and `ContextLogger`/`ObsLoggingMiddleware` align on lowercase
levels, nested-`extra` schema, and the `module_id` / `inputs` field names.

#### Added

- **`observability::storage::StorageBackend`** trait + `InMemoryStorageBackend`
  default — namespace/key/value persistence surface for cross-process
  durability, per observability.md §1 (Issue #43 §1). The default in-memory
  implementation is thread-safe (`RwLock`-guarded), namespace-isolated, and
  treats `delete` as idempotent. Re-exported from the crate root.
- **`ErrorHistory::with_storage_backend(per_module, total, backend)`** and
  **`ErrorHistory::with_storage(backend)`** — optional `StorageBackend`
  wiring; every recorded `ErrorEntry` is also persisted under namespace
  `"error_history"` keyed by fingerprint when the backend is supplied.
- **`UsageCollector::with_storage_backend(backend)`** — same pattern; usage
  records persist under namespace `"usage"`.
- **`MetricsCollector::with_storage_backend(backend)` / `.with_storage(...)`**
  — metric points persist under namespace `"metrics"` with a key derived
  from `(name, timestamp)`.
- **`UsageCollector::record_at(...)`** and
  **`UsageCollector::get_summary_for_period(period)`** (D-27) — `record_at`
  honors an explicit `DateTime<Utc>` timestamp, and `get_summary_for_period`
  filters records by recency. Trend is now derived from current-vs-previous
  sample counts (`stable` / `rising` / `declining` / `new` / `inactive`),
  matching `apcore-python` and `apcore-typescript` exactly.
- **`executor::deep_merge_chunks_checked(chunks)`** (D-58) — public helper
  that merges streaming chunks while enforcing each chunk is a JSON object;
  a non-object chunk yields `ModuleError::GeneralInvalidInput` with
  `details["code"] = "STREAM_CHUNK_NOT_OBJECT"` (cross-language: Python's
  `_deep_merge` AttributeError, TypeScript's TypeError). The streaming
  pipeline now uses this checked helper in Phase 3.
- **`ErrorCode::ConfigKeyRestricted`** (D-25) — distinct wire-format
  `CONFIG_KEY_RESTRICTED` code so callers can match the policy-deny case
  separately from value-shape errors. Emitted by
  `system.control.update_config` for keys in `RESTRICTED_KEYS`.
- **`ContextLogger::set_writer(...)`** — substitute the output sink (default
  stderr) so tests can capture emitted records.

#### Changed

- **`RetryConfig::default().max_retries`** is now `0` (was `3`) (D-14) —
  retries are explicitly opt-in, matching `apcore-python` and
  `apcore-typescript`. Behavior change: callers that were relying on the
  silent `3` default must now opt in by setting `max_retries` explicitly.
- **`ContextLogger`** JSON output schema (D-28):
  * `level` field is now lowercase (`"info"`), not uppercase (`"INFO"`).
  * User-supplied extras are nested under a single `"extra"` object instead
    of flattened to the top level.
- **`ObsLoggingMiddleware`** extras (D-28):
  * `module_id` (was `module`).
  * `inputs` (was `input`).
- **`UsageRecord.timestamp`** is now `DateTime<Utc>` (was `String`); the
  field was crate-private so the public surface is unaffected.

### Reload & Observability Hardening (Rust)

#### Added

- Granular reload via `path_filter` input in `ReloadModule` (#45.4).
- `Config::reload_from_disk()` for refreshing static config without binary restart (#45.5).
- Error fingerprinting in `ErrorHistory` — dedup by (error_code, top-frame hash, sanitized template) (#43 §4).
- Configurable redaction via `obs.redaction.regex_patterns` / `obs.redaction.sensitive_keys` Config keys (#43 §5).

### Cross-Language Sync — Review-Mode Hardening

This release applies the next batch of cross-language audit findings, focused
on ACL TOCTOU correctness, middleware closure ergonomics, event-emitter
fire-and-forget semantics, and documentation cleanup.

#### Added

- **`ACL::try_new`** is now the only fallible constructor; **`ACL::new`**
  panics on invalid `default_effect`, matching the constructor-throws
  behaviour of apcore-python and apcore-typescript (sync finding A-D-302).
  YAML loading still surfaces validation failures as `Result` via
  `ACL::load`.
- **`middleware::adapters::BeforeAdapter` / `AfterAdapter`** — closure-based
  middleware adapters that implement the `Middleware` trait, allowing
  before-only and after-only async closures to be registered directly via
  `MiddlewareManager::add(Box::new(BeforeAdapter::new(...)))` without
  defining a struct (sync finding A-D-402).
- **`EventEmitter::emit_spawn`** — fire-and-forget event dispatch that
  spawns one `tokio::task` per matching subscriber and returns
  immediately. Use this for the canonical fire-and-forget path; the
  existing `emit` method remains for sequential / deterministic test
  ordering (sync finding A-D-501).
- **`EventEmitter::shutdown` / `is_shutdown`** — idempotent shutdown
  that flushes pending work and turns subsequent `emit` / `emit_spawn`
  calls into no-ops (sync finding A-D-502).
- **`TraceContext::inject_with_options`** — extended W3C inject API
  accepting an optional 16-hex `parent_id` override, an optional
  propagated `trace_flags` byte, and an optional `tracestate` slice.
  When a tracestate is supplied (and non-empty) it is emitted as the
  `tracestate` header alongside `traceparent`. Invalid `parent_id`
  values fall back to a freshly generated random span ID. The
  zero-argument `TraceContext::inject` retains its existing public
  signature and is now a thin shim over `inject_with_options`
  (issue #35).
- **`TraceContext::extract_context`** — returns a full `TraceContext`
  parsed from an inbound header map, populating `tracestate` from
  the `tracestate` header per W3C §3.3.1 (comma-separated, capped at
  32 entries, malformed entries silently dropped) (issue #35).

#### Fixed

- **`TraceContext::extract` case-insensitive header KEY lookup**
  (issue #35) — RFC 7230 §3.2 requires HTTP header field names to be
  treated case-insensitively. `extract` and `extract_context` now
  match `traceparent` / `tracestate` header keys regardless of map
  key casing (`traceparent`, `Traceparent`, `TRACEPARENT`, etc.) via
  a new internal `lookup_header_ci` helper. Previously the `extract`
  path required exact lowercase keys.

#### Changed

- **`APCore::disable` / `APCore::enable`** signatures are
  `async fn (&self, name: &str, reason: Option<&str>) -> Result<Value, ModuleError>`,
  routing through `system.control.toggle_feature` and returning a status
  payload. **This is a breaking change** relative to pre-0.20 releases
  that exposed `disable(&mut self, name, reason) -> Result<(), ModuleError>`;
  cross-language parity is now restored with apcore-python and
  apcore-typescript (sync finding A-003).
- **`ACL::async_check`** now snapshots `rules` and `default_effect` at
  entry, eliminating a TOCTOU race where a concurrent `add_rule` /
  `reload` could mutate the rule list mid-evaluation. Mirrors the sync
  `check()` snapshot and matches apcore-python's `_snapshot()` and
  apcore-typescript's `rules.slice()` (sync finding A-D-301).
- **`ACL::reload`** restructured so the `&mut self` borrow on
  `self.yaml_path` ends *before* the blocking file read in `load()`,
  closing a deadlock window when the ACL is held in an
  `Arc<RwLock<ACL>>` wrapper and a reader hits the lock concurrently
  (sync finding A-D-303).
- **`EventEmitter` internal storage** changed from
  `Vec<Box<dyn EventSubscriber>>` to `Vec<Arc<dyn EventSubscriber>>` so
  subscribers can be cloned into `tokio::spawn` tasks for `emit_spawn`.
  No public-API surface change — `subscribe(Box<dyn ...>)` continues to
  work via `Arc::from`.
- **`CancelToken::check`** return type changed from
  `Result<(), ModuleError>` to `Result<(), ExecutionCancelledError>`
  (sync finding CANCEL-001). The typed variant matches Python's
  `ExecutionCancelledError(ModuleError)` subclass and TypeScript's
  `ExecutionCancelledError extends ModuleError` hierarchy, letting
  callers `match` on cancellation specifically. A `From<ExecutionCancelledError>
  for ModuleError` impl plus an `ExecutionCancelledError::to_module_error()`
  helper are provided for ergonomic widening, so most callers (`?` against
  a `ModuleError` context) need no changes. A new
  `CancelToken::check_for(&str)` method preserves the caller-supplied
  module ID in the typed error.

#### Documentation

- **`MiddlewareManager::remove`** doc clarified: name-based, not
  identity-based. Two middlewares registered with the same name cannot
  be removed independently (sync finding A-D-401).
- **README install pin** bumped from `apcore = "0.19"` to
  `apcore = "0.20"` (sync finding B-001).

### System Modules Hardening (Issue #45, system-modules.md §1.1–§1.5)

Implements the cross-language System Modules Hardening normative rules:
overrides persistence, contextual audit trail, Prometheus exporter for the
UsageCollector, glob-based bulk reload, and a strict registration entry
point. Aligns Rust with the `apcore-python` reference implementation.

#### Added

- **`sys_modules::audit`** — `AuditAction`, `AuditChange`, `AuditEntry`, the
  async `AuditStore` trait, and `InMemoryAuditStore`. Every state-changing
  control call (`update_config`, `reload_module`, `toggle_feature`) records
  an entry with `actor_id` / `actor_type` extracted from `context.identity`
  and the call's `trace_id`. When no store is configured, entries are logged
  at INFO level and discarded.
- **`sys_modules::overrides`** — `load_overrides` and `write_override` for
  YAML-backed runtime override persistence. Writes use a per-path lock and
  tempfile + rename to avoid partial-write corruption. Loaded after the
  base `Config`, so manual restores never erase runtime overrides.
- **`UpdateConfigModule::with_overrides_path` / `with_audit_store`,
  `ToggleFeatureModule::with_overrides_path` / `with_audit_store`,
  `ReloadModule::with_audit_store`** — opt-in builders for §1.1/§1.2.
- **Sensitive-key redaction in `UpdateConfigModule`** — `old_value` / `new_value`
  in the response payload, the `apcore.config.updated` event payload, and the
  `AuditChange` are replaced with `***REDACTED***` whenever the key matches a
  sensitive segment (`token`, `secret`, `key`, `password`, `auth`, `credential`).
  The in-memory `Config` still holds the real value — redaction is for egress
  only. Aligned with `apcore-python` (`utils/redaction.REDACTED_VALUE`).
- **Misconfiguration warning in `register_sys_modules_with_options`** — when
  `overrides_path` or `audit_store` is set but `sys_modules.events.enabled=false`,
  a `WARN`-level tracing event flags that control modules are not registered
  and the options have no effect.
- **`ReloadModule` `path_filter` input** — accepts a glob pattern and
  reloads every matching module in dependency-topological order (leaves
  first). `module_id` and `path_filter` are mutually exclusive — providing
  both raises `ErrorCode::ModuleReloadConflict`.
- **`UsageCollector::export_prometheus`** — emits
  `apcore_usage_calls_total{module_id,status}` (counter),
  `apcore_usage_error_rate{module_id}` (gauge), and
  `apcore_usage_p50/p95/p99_latency_ms{module_id}` (gauges).
  `PrometheusExporter::with_usage_collector` wires the new metrics into
  the existing `/metrics` endpoint.
- **`SysModulesOptions`** + **`register_sys_modules_with_options`** — passes
  `overrides_path`, `audit_store`, and `fail_on_error` into the registration
  flow without breaking the simpler 4-arg call site.
- **`SysModuleError`** — `RegistrationFailed { module_id, source }` returned
  from `register_sys_modules` when `fail_on_error` is `true`.
- **`ErrorCode::ModuleReloadConflict`** (`MODULE_RELOAD_CONFLICT`) and
  **`ErrorCode::SysModuleRegistrationFailed`** (`SYS_MODULE_REGISTRATION_FAILED`).
- **`tests/test_system_modules_hardening_conformance.rs`** — 10/10 cases from
  `apcore/conformance/fixtures/system_modules_hardening.json`.

#### Changed (BREAKING)

- **`register_sys_modules` signature** — now returns
  `Result<SysModulesContext, SysModuleError>` instead of
  `Option<SysModulesContext>`. When `sys_modules.enabled` is `false`, the
  function returns `Ok(SysModulesContext { … empty … })`. Callers that
  previously matched on `Option::Some` / `None` must switch to `Result`.
  `client::APCore::with_options` updated to log and continue on failure,
  preserving lenient default behavior.

---

### Middleware Architecture Hardening (Issue #42, middleware-system.md §1.x)

Implements the cross-language Middleware Architecture Hardening normative
rules: context-data namespace partitioning, the `CircuitBreakerMiddleware`
state machine, the OpenTelemetry-compatible `TracingMiddleware`, and the
YAML-driven middleware chain configuration.

#### Added

- **`middleware::context_namespace`** — `ContextWriter`, `validate_context_key`,
  and `enforce_context_key` helpers enforcing the `_apcore.*` (framework) /
  `ext.*` (user) prefix rules. Canonical key constants exposed via
  `middleware::namespace_keys` (`LOGGING_START_TIME`, `TRACING_SPAN_ID`,
  `CIRCUIT_STATE`).
- **`middleware::circuit_breaker::CircuitBreakerMiddleware`** — per-`(module_id,
  caller_id)` rolling-window breaker with `CLOSED → OPEN → HALF_OPEN → CLOSED`
  state machine. Emits `apcore.circuit.opened` and `apcore.circuit.closed`
  via an injected `Arc<EventEmitter>`. Writes `CLOSED` / `OPEN` / `HALF_OPEN`
  into `context.data["_apcore.mw.circuit.state"]` on every call.
- **`middleware::otel_tracing::TracingMiddleware`** (OTel-compatible) — opens
  a logical span on `before()` with attributes `apcore.trace_id`,
  `apcore.caller_id`, `apcore.module_id`, writes the span id under
  `_apcore.mw.tracing.span_id`, and records lifecycle status (`ok` / `error`)
  on `after()` / `on_error()`. Gated by the new compile-time `opentelemetry`
  feature with a runtime `enabled(bool)` builder override; silent no-op when
  disabled.
- **`ErrorCode::CircuitBreakerOpen`** — serialized as `CIRCUIT_OPEN`.
  Constructor: `ModuleError::circuit_breaker_open(module_id, caller_id)`.
- **`middleware::yaml_config`** — declarative middleware chain config:
  `MiddlewareConfig` enum (`Tracing`, `CircuitBreaker`, `Logging`, `Custom`),
  `MiddlewareChainConfig::from_yaml` / `from_json`, and `MiddlewareFactory`
  with custom-handler registration and optional event-emitter injection.
- **`tests/test_middleware_hardening_conformance.rs`** — 10/10 cases from
  `apcore/conformance/fixtures/middleware_hardening.json`.

#### Notes

- Async-handler detection is satisfied statically by Rust's type system; the
  conformance test asserts compile-time witness via the `Middleware` trait.
- The pre-existing `observability::tracing_middleware::TracingMiddleware`
  (span-exporter based) is unchanged. The new OTel-compatible middleware is
  re-exported at the crate root as `OtelTracingMiddleware` to avoid a name
  collision.

### Multi-Module Discovery (Issue #32, PROTOCOL_SPEC §2.1.1, multi-module-discovery.md)

Adds opt-in multi-class discovery: multiple `Module` implementations may
coexist in a single source file, each receiving an ID of the form
`base_id.snake_case(struct_name)`. Off by default — single-class files are
unaffected and produce identical IDs regardless of whether the feature is
enabled (single-class identity guarantee).

#### Added

- **`registry::multi_class` module** — new module hosting the cross-language
  ID-derivation primitives.
- **`DiscoveryConfig { multi_class: bool }`** — opt-in flag (default `false`).
- **`class_name_to_segment(&str) -> String`** — snake_case conversion
  algorithm aligned with `apcore-python.class_name_to_segment`. Handles
  `Addition` → `addition`, `MathOps` → `math_ops`, `HTTPSender` →
  `http_sender`, `MyModule_V2` → `my_module_v2`.
- **`compute_base_id(&Path, &str) -> String`** — Algorithm A01 base ID
  derivation from file path + extensions root.
- **`derive_module_ids(&Path, &str, &[DiscoveredClass], &DiscoveryConfig)`** —
  pure ID-derivation function returning the list of derived IDs (or
  `MODULE_ID_CONFLICT` / `INVALID_SEGMENT` / `ID_TOO_LONG` errors).
- **`DiscoveredClass`** struct (`name`, `implements_module`) for the
  conformance-fixture interface.
- **`MultiClassEntry`** struct + **`Registry::register_multi_class()`** — the
  user-facing registration helper. Atomic registration: if any per-module
  registration fails, already-registered modules from the batch are rolled
  back so the file is registered all-or-nothing.
- **`ErrorCode::ModuleIdConflict`** (`MODULE_ID_CONFLICT`) — two or more
  classes in the same file produce the same `class_segment` after snake_case
  conversion. Details carry `file_path`, `class_names`, and
  `conflicting_segment`.
- **`ErrorCode::InvalidSegment`** (`INVALID_SEGMENT`) — derived segment does
  not conform to the canonical ID grammar.
- **`ErrorCode::IdTooLong`** (`ID_TOO_LONG`) — full derived `module_id`
  exceeds `MAX_MODULE_ID_LEN` (192).
- **`ModuleError::module_id_conflict()`**, **`invalid_segment()`**,
  **`id_too_long()`** builders.
- **`MAX_MODULE_ID_LEN: usize = 192`** constant in the multi_class module
  (mirrors the existing `MAX_MODULE_ID_LENGTH` in the registry module for
  cross-SDK naming consistency).
- **Cross-language conformance tests** for all eight Issue #32 fixture cases
  (`single_class_id_unchanged`, `two_classes_distinct_ids`,
  `class_name_snake_case_addition`, `class_name_snake_case_math_ops`,
  `class_name_snake_case_https_sender`, `conflict_same_segment`,
  `full_id_grammar_valid`, `disabled_by_default`) in
  `tests/test_multi_module_discovery_conformance.rs`.

#### Notes

- **Rust integration model**: Rust has no runtime reflection, so
  multi-class discovery cannot enumerate `impl Module for X` at scan time
  the way Python `inspect.getmembers` does. Module authors register a list
  of `(class_name, instance)` pairs explicitly via
  `Registry::register_multi_class`. The pure ID-derivation logic is shared
  with the conformance fixture so all three SDKs validate against the same
  test cases.
- The single-class identity guarantee applies regardless of
  `multi_class` mode: a file with exactly one qualifying class always
  receives the bare `base_id` (no `.class_segment` suffix). This preserves
  all existing module IDs.
- Multi-class disabled with multiple classes in a file: the file is
  treated as single-class — only the first qualifying class is loaded
  under `base_id`. Mirrors the `disabled_by_default` fixture case and
  apcore-python policy.

### Pipeline Architecture Hardening (Issue #33, core-executor.md §Pipeline Hardening)

This release adds the cross-SDK pipeline hardening primitives required by
Issue #33. Public APIs (`Executor::call`, `Executor::validate`,
`Executor::stream`) preserve their existing typed errors — `PipelineEngine`
wraps step failures in `PipelineStepError` internally and the executor
unwraps before returning, mirroring the apcore-python reference.

#### Added

- **`ErrorCode::PipelineStepError`** (`PIPELINE_STEP_ERROR`) — fail-fast wrapper
  carrying the failing step's name and the original `ModuleError` cause.
- **`ErrorCode::PipelineStepNotFound`** (`PIPELINE_STEP_NOT_FOUND`) — surfaced by
  `ExecutionStrategy::configure_step` when the target step does not exist.
- **`ModuleError::pipeline_step_error(step_name, &cause)`** builder, plus the
  `is_pipeline_step_error()`, `step_name()`, and `unwrap_pipeline_step_error()`
  accessors for inspecting / unwrapping wrapped errors.
- **`ExecutionStrategy::configure_step(name, step)`** — replace-semantic that is
  idempotent and preserves the step's position in the execution order (§1.2).
- **`ExecutionStrategy::name_to_idx()`** — exposes the maintained
  `HashMap<String, usize>` so the O(1) lookup is observable per §1.5. The map
  is rebuilt after every mutation (`new`, `insert_after`, `insert_before`,
  `remove`, `replace`, `replace_with`, `configure_step`).
- **`PipelineState`** — snapshot type passed to `run_until` predicates, carrying
  `step_name`, `outputs`, and a borrowed reference to the live `PipelineContext`.
- **`RunUntilPredicate`** type alias and **`RunOptions`** struct for the new
  `PipelineEngine::run_with_options` entry point.
- **`PipelineEngine::run_until(strategy, ctx, predicate)`** — predicate-based
  termination per §1.4. Evaluated after each step's clean continue; returning
  `true` halts the pipeline and reports success.
- **Cross-language conformance tests** for the five Issue #33 fixtures
  (`fail_fast_on_step_error`, `continue_on_ignored_error`,
  `replace_semantic_no_duplicate`, `run_until_stops_early`,
  `step_lookup_is_not_linear`) in
  `tests/test_pipeline_hardening_conformance.rs`.

#### Changed

- **`PipelineEngine` step-error behavior** — when a step returns `Err` and its
  `ignore_errors` is `false`, the engine now wraps the error in a
  `PipelineStepError` (§1.1). `Executor::call`, `Executor::validate`, and
  `Executor::stream` unwrap before returning so user-visible error codes are
  unchanged. Callers that drive `PipelineEngine::run` directly will observe the
  wrapped code and should call `unwrap_pipeline_step_error()` for the cause.
- **`PipelineEngine::run_until` previously took `stop_before_step: &str`** for
  the streaming pre-execute phase. That method is renamed to
  **`run_until_step`**; the `run_until` name now hosts the spec-conformant
  predicate-based API. The two streaming callers in `executor.rs` and
  `pipeline.rs` were migrated.
- **`skip_to` lookup** in the engine now uses `ExecutionStrategy::name_to_idx`
  (O(1)) and explicitly rejects same-position / backward targets to prevent
  infinite loops.

#### Notes

- **§1.3 step-level middleware** is `SHOULD` in the spec, has no conformance
  fixture, and is not yet implemented in apcore-python. Per the
  reference-implementation alignment policy (apcore-python is canonical),
  this is deferred — to be revisited once Python lands the API.

### Schema System Hardening (Issue #44, PROTOCOL_SPEC §4.15)

This release replaces the hand-written schema validator with the `jsonschema`
crate (Draft 2020-12) and adds the cross-SDK hardening primitives required by
Issue #44. All previously passing schema tests continue to pass.

### Added

- **`jsonschema = "0.28"` Draft 2020-12 backend** — `SchemaValidator` now wraps
  `jsonschema::Validator` and gains complete support for `anyOf` / `oneOf` /
  `allOf` / `not`, recursive `$ref` (e.g. self-referencing TreeNode schemas via
  `"$ref": "#"`), and all numerical / string constraints (`minimum`, `maximum`,
  `exclusiveMinimum`, `minLength`, `maxLength`, `pattern`).
- **`SchemaValidator::validate_detailed`** — new structured-result variant that
  returns mapped error codes (`SchemaUnionNoMatch`, `SchemaUnionAmbiguous`,
  `SchemaValidationError`) and non-fatal format warnings.
- **`schema::content_hash(&Value) -> String`** — SHA-256 hex digest of the
  canonical (sorted-keys) JSON form of a schema. Two byte-equivalent schemas
  hash to the same digest, satisfying the cross-SDK deduplication invariant.
- **Content-addressable compile cache** on `SchemaValidator` — repeated
  validation against the same schema (or a key-reordered copy) compiles the
  schema exactly once. `cache_len()` and `clear_cache()` accessors included.
- **`schema::format_warnings(&Value, &Value)`** — opt-in semantic format check
  for `date-time`, `date`, `time`, `email`, `uri`, `uuid`, `ipv4`, `ipv6`.
  Format enforcement is SHOULD-level: invalid values produce a warning rather
  than failing validation, matching the Python and TypeScript SDKs.
- **New `ErrorCode` variants**: `SchemaUnionNoMatch` (`SCHEMA_UNION_NO_MATCH`),
  `SchemaUnionAmbiguous` (`SCHEMA_UNION_AMBIGUOUS`), `SchemaMaxDepthExceeded`
  (`SCHEMA_MAX_DEPTH_EXCEEDED`).
- **`sha2 = "0.10"`** dependency for the canonical-JSON content hash.
- **Cross-language conformance tests** for the five new fixtures
  (`schema_hardening_union`, `schema_hardening_recursive`,
  `schema_hardening_constraints`, `schema_hardening_formats`,
  `schema_hardening_cache`) in `tests/test_schema_hardening_conformance.rs`.

### Changed

- **`SchemaValidator` is no longer a unit struct.** It now owns an internal
  `Arc<Mutex<HashMap<String, Arc<jsonschema::Validator>>>>` compile cache.
  Existing constructors (`SchemaValidator::new()`,
  `SchemaValidator::default()`) and the `validate` / `validate_or_error`
  methods remain source-compatible. Code that relied on the unit-struct form
  (`SchemaValidator;`) must switch to `SchemaValidator::default()` or
  `SchemaValidator::new()`.

### AsyncTask Evolution (Issue #34, async-tasks.md §AsyncTaskManager Evolution)

Adds three capability extensions to `AsyncTaskManager`: a pluggable
`TaskStore` trait, configurable retry with exponential backoff, and an
opt-in TTL-based Reaper background task. The pre-existing 3-arg
`AsyncTaskManager::new(executor, max_concurrent, max_tasks)` constructor is
preserved; it now defaults to the new `InMemoryTaskStore` so existing
callers and tests are unaffected.

#### Added

- **`async_task::TaskStore`** trait (`#[async_trait]`) — pluggable storage
  backend with `save / get / list / delete / list_expired` and
  `store_type_name`. Decouples task state from in-process memory and enables
  distributed deployments / persistence across process restarts. Ships with
  `InMemoryTaskStore` (default, `dashmap`-backed) — third-party backends
  (`RedisTaskStore`, `SqlTaskStore`) live in downstream crates.
- **`AsyncTaskManager::with_store(executor, max_concurrent, max_tasks, store)`** —
  constructor accepting a caller-provided `Arc<dyn TaskStore>`.
- **`AsyncTaskManager::store_type_name()`** — exposes the active backend's
  identifier for tooling / introspection.
- **`AsyncTaskManager::store()`** — returns a clone of the underlying
  `Arc<dyn TaskStore>` for direct interaction.
- **`async_task::RetryConfig` { `max_retries`, `retry_delay_ms`,
  `backoff_multiplier`, `max_retry_delay_ms` }** — per-task retry policy.
  `delay_for_attempt(attempt)` computes
  `min(retry_delay_ms * (backoff_multiplier ^ attempt), max_retry_delay_ms)`.
  Not re-exported at the crate root to avoid colliding with
  `middleware::RetryConfig` — import via `apcore::async_task::RetryConfig`.
- **`AsyncTaskManager::submit_with_retry(module_id, inputs, ctx, retry)`** —
  submission variant that accepts an optional retry policy. On failure the
  task is rescheduled with `tokio::time::sleep` and `status` returns to
  `Pending` until `max_retries` is exhausted, after which it transitions to
  `Failed` with `error` populated.
- **`async_task::ReaperConfig` { `ttl_seconds`, `sweep_interval_ms`}** and
  **`async_task::ReaperHandle`** — opt-in background reaper.
  `AsyncTaskManager::start_reaper(ReaperConfig)` returns a handle; calling
  `handle.stop().await` signals graceful shutdown via a
  `tokio::sync::watch` channel and awaits the join. The sweep calls
  `store.list_expired(now - ttl)` which only returns terminal-state tasks,
  so pending and running tasks are never deleted by the reaper.
- **`async_task::AsyncTaskManager::get_status_async` / `get_result_async`** —
  async variants of the synchronous facade methods, intended for callers
  with network-backed `TaskStore` implementations.
- **`TaskInfo.retry_count` and `TaskInfo.max_retries`** fields (both
  `#[serde(default)]`, so wire-format compatibility is preserved).
- **`dashmap = "6"`** dependency (used internally by `InMemoryTaskStore` for
  lock-free concurrent access).
- **`tests/test_async_task_evolution_conformance.rs`** — 10/10 cases from
  `apcore/conformance/fixtures/async_task_evolution.json` plus a smoke test
  for the `ReaperHandle` lifecycle.

#### Notes

- The synchronous facade (`submit`, `cancel`, `cleanup`, `shutdown`,
  `list_tasks`, `get_status`, `get_result`, `task_count`) is preserved and
  internally drives the async `TaskStore` through a single-poll no-op-waker
  helper. Custom `TaskStore` implementations whose futures actually yield
  MUST use the `_async` variants — the synchronous facade panics if a future
  returns `Pending`.
- Concurrent `submit_with_retry` calls are serialised through an
  `admission_lock` so the `len() < max_tasks` check and the subsequent
  `save` are atomic — without this, two racing submits could both pass the
  cap check and exceed `max_tasks`.
- Cross-language alignment: `RETRYING` is not a separate `TaskStatus` in the
  Rust SDK (the cross-language spec lifecycle pins five states); a task
  awaiting its next retry attempt stays in `Pending`. `started_at` is set
  on the first execution and preserved across retries to match Python.

## [0.19.1] - 2026-04-27

### Added

- **`Registry::export_schema_strict(name, strict)`** — Adds a strict-mode variant of `Registry::export_schema` that returns the full schema envelope (`module_id`, `description`, `input_schema`, `output_schema`) with strict-mode transformation applied when `strict=true` (sets `additionalProperties:false` on objects, marks all properties required, rewrites optional fields as nullable). This aligns the Rust SDK with `apcore-python` and `apcore-typescript` `Registry` interfaces for MCP compatibility.

## [0.19.0] - 2026-04-19

### Added

- **`ErrorCode::DependencyVersionMismatch`** — new error code raised by `resolve_dependencies` when a declared `version` constraint is not satisfied by the registered version of the target module. `ModuleError` details include `module_id`, `dependency_id`, `required`, `actual`.
- **`resolve_dependencies(modules, known_ids, module_versions)`** — new third argument `Option<&HashMap<String, String>>` mapping `module_id → version`. When provided, declared dependency version constraints are enforced per PROTOCOL_SPEC §5.3. When absent, the `DepInfo.version` field is silently ignored.
- **Caret (`^`) and tilde (`~`) constraint support** in `matches_version_hint` / `select_best_version` (npm/Cargo semantics): `^1.2.3 → >=1.2.3,<2.0.0`, `^0.2.3 → >=0.2.3,<0.3.0`, `^0.0.3 → >=0.0.3,<0.0.4`, `~1.2.3 → >=1.2.3,<1.3.0`, `~1.2 → >=1.2.0,<1.3.0`, `~1 → >=1.0.0,<2.0.0`.
- **`TypedBindingHandler`** and **`typed_handler<I, O>()`** — Generic function that bundles an async handler with auto-derived JSON Schemas from `schemars::JsonSchema` trait bounds. When used with `register_into_with_typed_handlers()`, schemas from `schemars` are used for `auto_schema` bindings instead of the permissive `{"type":"object"}` fallback. No proc-macro crate needed. See DECLARATIVE_CONFIG_SPEC.md §6.5.
- **`auto_schema: true | permissive | strict`** — `AutoSchemaValue` enum accepts boolean or mode string. Strict mode reserved for OpenAI/Anthropic schema compliance (enforcement via `schemars` + post-processing tracked for 0.20.0).
- **New `ErrorCode` variants**: `BindingSchemaInferenceFailed`, `BindingSchemaModeConflict`, `BindingStrictSchemaIncompatible`, `BindingPolicyViolation`, `PipelineConfigInvalid`, `PipelineHandlerNotSupported`, `PipelineStepInsertionAmbiguous`, `EntryPointNotFound`, `EntryPointAmbiguous`, `EntryPointRuntimeUnsupported` (reserved). See DECLARATIVE_CONFIG_SPEC.md §7.1.
- **Pipeline `handler:` parse-time rejection** — `PipelineHandlerNotSupportedError` with remediation message pointing to `register_step_type()`. See DECLARATIVE_CONFIG_SPEC.md §4.4.
- **Pipeline metadata fields honored**: `match_modules`, `ignore_errors`, `pure`, `timeout_ms` now applied to resolved steps via `ConfiguredStep` wrapper. Previously silently dropped.
- **Pipeline `configure:` section** — Overlay metadata fields on existing built-in steps via `ExecutionStrategy::replace_with()`.
- **`ExecutionStrategy::replace_with(name, wrapper_fn)`** — Replace a step in-place by applying a wrapper function.
- **`schema_ref` loading implemented** — External YAML schema files referenced by `schema_ref` field now actually loaded and parsed (previously the field was stored but never processed).
- **`spec_version`** handling in binding YAML with deprecation warning when absent.
- **`schemars` dependency** (`0.8`) for JSON Schema generation from Rust types.
- **Cross-SDK conformance fixtures** in `apcore/conformance/fixtures/`.
- **Reintroduced `AsyncTaskManager`** (`src/async_task.rs`) — background task execution with `submit`, `get_status`, `get_result`, `cancel`, `list_tasks`, `cleanup`, `shutdown`; bounded by `max_concurrent` and `max_tasks`. 24 tests in `tests/test_async_task.rs`. Re-exported from crate root. Was temporarily removed in 0.18.0 pending `Executor` integration.
- **Reintroduced `ExtensionManager` / `ExtensionPoint`** (`src/extensions.rs`) — plugin registry with `register`, `get`, `get_all`, `unregister`, `list_points`, `apply`; supported extension points include `discoverer`, `module_validator`, `middleware`, `span_exporter`, `acl`, `approval_handler`. 21 tests in `tests/test_extensions.rs`. Re-exported from crate root. Was temporarily removed in 0.18.0.
- **`Context::builder()`** — New builder API supporting W3C trace_parent inheritance: `Context::builder().trace_parent(Option<TraceParent>).identity(id).services(s).build()`. The builder validates the incoming `trace_parent.trace_id` against `^[0-9a-f]{32}$` (rejecting all-zero and all-f per W3C), inheriting valid values verbatim and regenerating with `tracing::warn!` otherwise. See PROTOCOL_SPEC §10.5 `external_trace_parent_handling`. Existing `Context::new`, `Context::anonymous`, and `Context::create` constructors remain backward-compatible.

### Fixed

- **`resolve_dependencies` cycle path accuracy** — `extract_cycle` previously returned a phantom path (all remaining nodes plus the first one re-appended) when the arbitrarily-picked start node had no outgoing edge inside `remaining`. This could happen when a module is blocked on an external `known_ids` dependency while another subset contains a real cycle. Rewritten to DFS from each remaining node (sorted) and return a true back-edge cycle `[n0, ..., nk, n0]`; falls back to the sorted `remaining` set only when no back-edge exists.
- **`CircularDependencyError` now carries `cycle_path` in `ModuleError.details`** (as a JSON string array), matching the Python `details["cycle_path"]` / TypeScript `details.cyclePath` contract. Previously the path was only embedded in the message string, forcing downstream consumers to parse it.

### Changed

- **`Context` trace_id format** changed from 36-char UUID (with dashes) to **32-char lowercase hex** (aligned with W3C Trace Context `trace-id` field). Affects all internal constructors and external observability output. Downstream Jaeger/Tempo/Honeycomb/Datadog/OTLP consumers gain direct interoperability; the `TraceContext::inject()` dash-stripping workaround is now a no-op for freshly-created contexts but retained for backward compatibility with any persisted 36-char IDs.
- **`resolve_dependencies` signature** changed from `(modules, known_ids) -> Result<...>` to `(modules, known_ids, module_versions) -> Result<...>`. Pass `None` for `module_versions` to preserve prior behavior. All in-crate call sites updated.
- **Missing required dependencies now return `ErrorCode::DependencyNotFound` instead of `ErrorCode::ModuleLoadError`.** Brings Rust into compliance with PROTOCOL_SPEC §5.15.2. The error's `details` map now includes `module_id` and `dependency_id`. Upgrade path: match on `ErrorCode::DependencyNotFound` where you previously matched `ErrorCode::ModuleLoadError` for missing-dep cases.
- **Binding YAML format migrated to canonical** — Top-level `bindings:` list with `module_id` and string `target: "module:callable"`. Old format (`- name:` flat list, `target: {module_name, callable}`, `metadata:` wrapper) removed. See DECLARATIVE_CONFIG_SPEC.md §8.1 for migration guide.
- **`BindingDefinition` and `BindingTarget` removed** — Replaced by `BindingEntry` and `BindingsFile`. Public re-exports updated.
- **Handler-map key changed** from binding `name` to full `target` string (e.g., `"format_date:format_date_string"`).
- **`BindingSchemaMissing` ErrorCode variant deprecated** — Superseded by `BindingSchemaInferenceFailed`. Kept for backward-compatible deserialization.
- **`description`, `documentation`, `tags`, `version`, `annotations`, `display`, `metadata`** now parsed from top-level binding entry fields (previously some were nested under `metadata`).

## [0.18.1] - 2026-04-16

### Changed

- **`ModuleDescriptor` unified with the cross-language protocol shape.** The slim Rust-only descriptor and the auxiliary `FullModuleDescriptor` (previously in `src/registry/types.rs`) have been merged into a single `ModuleDescriptor` in `apcore::registry` that matches `apcore-python.ModuleDescriptor` and `apcore-typescript.ModuleDescriptor` field-for-field. Changes relative to v0.18.0:
  - `name: String` (previously the canonical module ID) is now `name: Option<String>` (human-readable display name).
  - New required field `module_id: String` carries the canonical identifier that used to live in `name`.
  - New optional fields: `description`, `documentation`, `version` (default `"1.0.0"`), `examples`, `metadata`, `sunset_date`.
  - `annotations: ModuleAnnotations` is now `annotations: Option<ModuleAnnotations>` (matches Python `None` / TS `null`).
  - `enabled: bool` (Rust-only runtime toggle) is kept but marked `#[serde(skip_serializing)]` so it never leaks into cross-language wire payloads; it still deserializes with a default of `true`.
  - `FullModuleDescriptor` is removed from the public API. All callers should use `ModuleDescriptor`.

  **Migration**: callers constructing `ModuleDescriptor` literals must rename `name` to `module_id`, wrap `annotations` in `Some(...)`, and supply the new fields (all have sensible defaults — `description: String::new()`, `documentation: None`, `version: "1.0.0".into()`, `examples: vec![]`, `metadata: HashMap::new()`, `sunset_date: None`).

### Fixed

- Clippy `unnecessary_map_or` warnings in `builtin_steps.rs`, `executor.rs`, and `sys_modules/manifest.rs` (11 sites) — replaced `.map_or(false, |a| a.field)` with `.is_some_and(|a| a.field)`.

---

## [0.18.0] - 2026-04-15

### Added

- **`APCore::from_path()` factory method** — Ergonomic shorthand: `APCore::from_path("apcore.yaml")?` is equivalent to `let config = Config::load("apcore.yaml")?; APCore::with_config(config)`. Returns `Result<APCore, ModuleError>`. Existing `APCore::with_config()` usage is unchanged.
- **`pub const MAX_MODULE_ID_LENGTH: usize = 192`** in `apcore::registry::registry`, re-exported from `apcore::registry` and the crate root (`apcore::MAX_MODULE_ID_LENGTH`). Tracks PROTOCOL_SPEC §2.7 EBNF constraint #1 and aligns with `apcore-python` / `apcore-typescript`.
- **`Registry::register` now enforces module ID length** per PROTOCOL_SPEC §2.7. Module IDs longer than `MAX_MODULE_ID_LENGTH` are rejected with `ErrorCode::GeneralInvalidInput` carrying the message `"Module ID exceeds maximum length of {N}: {actual}"`. **This was a previously undetected spec compliance gap** — the constraint is `MUST` in the protocol but the Rust SDK never validated it. Python and TypeScript SDKs have always enforced it.
- **`module_id_pattern()` function** returning `&'static Regex` (lazy `OnceLock<Regex>`) for the canonical EBNF pattern. Re-exported at the crate root as `apcore::module_id_pattern`.
- **`REGISTRY_EVENTS` constants** — `pub mod registry_events { pub const REGISTER, UNREGISTER }`, `pub struct RegistryEvents` with associated consts, and the `pub const REGISTRY_EVENTS: RegistryEvents` singleton. Closes a §12.2 MUST violation: all SDKs must export these event names as named constants. Aligned with apcore-python (`REGISTRY_EVENTS` dict) and apcore-typescript (`REGISTRY_EVENTS` frozen object).
- **Crate-root re-exports for parity with apcore-python and apcore-typescript:** `MiddlewareManager`, `Middleware`, `BeforeMiddleware`, `AfterMiddleware`, `LoggingMiddleware`, `RetryMiddleware`, `RetryConfig`, `PlatformNotifyMiddleware`, `ErrorHistoryMiddleware`, `MetricsMiddleware`, `UsageMiddleware`, `ObsLoggingMiddleware`, `ErrorFormatterRegistry`, `ErrorFormatter`, `build_minimal_strategy`, `BindingLoader`, `BindingDefinition`, `BindingTarget`, `CancelToken`, `FunctionModule`, `ErrorHistory`, `ErrorEntry`, `MetricsCollector`, `UsageCollector`, `UsageStats`, `Span`, `SpanExporter`, `StdoutExporter`, `InMemoryExporter`, `OTLPExporter`, `SchemaLoader`, `SchemaValidator`, `SchemaExporter`, `RefResolver`, `TraceContext`, `TraceParent`. All previously required `apcore::module_path::*` access; now reachable directly from `apcore::*`. Note: `Extension`, `ExtensionManager`, `ExtensionPoint`, `AsyncTaskManager`, and `TaskInfo` are **not** re-exported — see the "Cross-Language Feature Parity" note in the README.
- **`Registry::register_internal` now enforces empty / EBNF pattern / length / duplicate checks** via the shared `validate_module_id()` helper (was previously bypassing all validation). The reserved-word check is the only step skipped (so sys modules can use the `system.*` prefix). Aligned with apcore-typescript `registerInternal()`.
- **Boundary tests** in `tests/test_registry.rs`: `test_max_module_id_length_matches_spec`, `test_register_accepts_module_id_at_max_length`, `test_register_rejects_module_id_exceeding_max_length`, plus 6 `test_register_internal_*` parity tests.
- **`tests/test_crate_root_exports.rs`** — 7 regression tests asserting that every spec-required and Python/TS-parity symbol is reachable from `apcore::*`.
- **`test_validate_accepts_optional_context`** regression test in `tests/test_executor.rs`.
- **`TraceContext::inject()` and `TraceContext::extract()`** — W3C trace context propagation utilities, aligned with apcore-python and apcore-typescript. `inject` serializes a `Context`'s trace ID into a `traceparent` header map; `extract` parses a `traceparent` header back into a `TraceParent`. Includes 8 unit tests.
- **`Executor::register_strategy()` and `Executor::list_strategies()` associated functions** — Delegates to existing module-level functions. Spec places these on Executor; aligned with apcore-python (classmethod/instance method) and apcore-typescript (static methods).

### Changed

- **`Executor::describe_pipeline()` now returns `StrategyInfo` instead of `String`** — Provides structured access to pipeline metadata (`name`, `step_count`, `step_names`, `description`). `StrategyInfo` implements `Display` for `.to_string()` backward compatibility. Aligned with apcore-typescript `describePipeline() -> StrategyInfo` and apcore-python `describe_pipeline() -> StrategyInfo`.

- **`ACL::check()` and `ACL::async_check()` consolidated** via three shared private helpers (`finalize_no_rules`, `finalize_rule_match`, `finalize_default_effect`). Audit-entry construction, debug-logging, and default-effect mapping now live in exactly one place (was duplicated across sync and async paths). Added `check_conditions_async` helper so `matches_rule_async` no longer inlines conditions extraction. Aligned with apcore-python `_finalize_check` helper pattern.
- **README documents annotation overlay cross-SDK difference** — New section explaining that Rust does not implement YAML annotation overlays, with rationale (spec §4.13 is conditional, Rust favors explicit code annotations) and a serde workaround for users who need it.
- **`Executor::validate()` signature gained an optional context parameter** — `pub async fn validate(&self, module_id: &str, inputs: &Value, ctx: Option<&Context<Value>>) -> Result<PreflightResult, ModuleError>`. Aligns with PROTOCOL_SPEC §12.2 line 6405 and matches apcore-python / apcore-typescript. When `None` is passed, an anonymous `@external` context is synthesized internally for backward compatibility (existing behavior preserved). When a real context is passed, call-chain checks (depth limit, circular call detection) and ACL caller-identity matching see real caller state. **This is a source-incompatible change for any code calling `executor.validate(id, inputs)` — add a third `None` argument.**
- **`Registry::register` and `Registry::register_internal` duplicate-detection error code changed from `ErrorCode::ModuleLoadError` to `ErrorCode::GeneralInvalidInput`.** Aligns with apcore-python / apcore-typescript which use `InvalidInputError` (`GENERAL_INVALID_INPUT`) for the same condition. `ModuleLoadError` is reserved for actual module load failures (file I/O, parse errors); a duplicate ID is invalid input from the caller. **Clients catching errors by code in Rust will see a different code than before** — update any `match` arms.
- **Duplicate-registration error message canonicalized** to `"Module ID '<name>' is already registered"` (was `"Module '<name>' is already registered"` — added the "ID" word). Both `register()` and `register_internal()` emit the same string. Now byte-identical to apcore-python and apcore-typescript.
- **README installation snippet bumped from `apcore = "0.16"` to `apcore = "0.18"`** — was 2 minor versions stale and would have given new users a broken install of the v0.16 surface.

### Compatibility (in addition to the BREAKING items below)

- **`Executor::validate()` is source-incompatible** for callers that passed only two arguments — add `None` as the third argument. The semantics with `None` are identical to the previous two-arg behavior.
- **Duplicate-error consumers in Rust must update error-code matches** from `ErrorCode::ModuleLoadError` to `ErrorCode::GeneralInvalidInput` for the registration-duplicate path.
- **`register_internal()` is source-compatible** but stricter at runtime. Existing in-tree callers (`apcore::sys_modules::*`) all use canonical-shape IDs and are unaffected. External adapter authors who used `register_internal()` as a generic escape hatch for non-canonical IDs should review.
- **New rejection path for over-length IDs.** Code that previously registered module IDs longer than 192 characters in Rust will now fail at `register()`. Python and TypeScript already rejected such IDs at the previous 128-char threshold; no consistent cross-language behavior was possible before this fix.

### Changed (BREAKING)

- **`Config` struct restructured to namespaced form per PROTOCOL_SPEC §9.1.** Executor and observability settings now live under nested sub-structs `ExecutorConfig` and `ObservabilityConfig` instead of being flat fields on `Config`. The mapping is:

  | Before (≤ 0.17.x)              | After (0.18.0+)                          |
  |--------------------------------|------------------------------------------|
  | `config.max_call_depth`        | `config.executor.max_call_depth`         |
  | `config.max_module_repeat`     | `config.executor.max_module_repeat`      |
  | `config.default_timeout_ms`    | `config.executor.default_timeout`        |
  | `config.global_timeout_ms`     | `config.executor.global_timeout`         |
  | `config.enable_tracing`        | `config.observability.tracing.enabled`   |
  | `config.enable_metrics`        | `config.observability.metrics.enabled`   |
  | `config.settings`              | `config.user_namespaces`                 |

  The `_ms` suffix is dropped from timeout fields to align with spec §9.1 and the Python/TypeScript SDKs (units stay milliseconds; documented in field doc comments).

  **Wire format (YAML/JSON) is also breaking.** Producers MUST use the canonical nested form:

  ```yaml
  executor:
    max_call_depth: 32
    default_timeout: 30000
  observability:
    tracing:
      enabled: true
  ```

  Loading a v0.17.x-style config with root-level `max_call_depth`, `default_timeout_ms`, etc. now produces a hard error pointing at `MIGRATION-v0.18.md`. There is no silent migration. See the migration guide for the rationale.

- **`Config::get()` / `Config::set()` no longer accept legacy bare field names** — `config.get("max_call_depth")` returns `None` in v0.18.0; use `config.get("executor.max_call_depth")` instead. Cross-language parity with Python/TypeScript.

- **`Config::bind()` now special-cases canonical namespaces** — `config.bind::<ExecutorConfig>("executor")` returns the typed sub-struct directly without going through `user_namespaces`.

### Fixed

- **README `Identity` struct literal replaced with `Identity::new()` constructor.** The greet example used a struct literal `Identity { id: ..., ... }` but Identity fields are private — the correct constructor is `Identity::new(id, identity_type, roles, attrs)`.
- **`Config` no longer silently ignores spec-conformant YAML.** A YAML file using the canonical `executor: { max_call_depth: 100 }` shape would previously be captured into the unused `settings` HashMap and the typed `max_call_depth` field would remain at default 32. Discovered during the v0.18.0 cross-language audit.

- **`ModuleAnnotations.extra` wire format aligned with PROTOCOL_SPEC §4.4.1** — Removed `#[serde(flatten)]` on the `extra` field. The struct now serializes `extra` as a nested `"extra"` object, matching `apcore-python` and `apcore-typescript`. This fixes a silent cross-language data-loss bug where Python/TypeScript payloads carrying nested `extra` would deserialize on the Rust side as `extra["extra"] = {...}` (one level too deep). The custom `Deserialize` impl tolerates legacy flattened input from `apcore-rust ≤ 0.17.1` for one MINOR backward-compat cycle. When the same key appears in both forms, the nested value wins per spec rule 7.

### Changed

- **`ModuleAnnotations` Deserialize is now hand-rolled** — Replaced `#[derive(Deserialize)]` with a custom `Visitor` to support both nested and legacy flattened wire forms with deterministic precedence. The public API of the struct is unchanged; only the on-the-wire format is corrected.

### Added

- **`ExecutorConfig`, `ObservabilityConfig`, `TracingConfig`, `MetricsConfig`** — New public sub-structs for canonical namespace binding. Available via `apcore::config::*` and re-exported from the crate root.

### Removed

- **`Config.settings` field** → renamed to `Config.user_namespaces` to clarify intent (it captures user-defined namespaces only, not canonical ones).
- **`default_true` and `default_pagination_style` private helpers** in `module.rs` — No longer needed now that `Deserialize` for `ModuleAnnotations` is custom; defaults flow through `Default::default()`.
- **`AsyncTaskManager`** and **`ExtensionManager` / `ExtensionPoint`** — The Rust implementations were non-functional stubs and have been removed from the crate root and all re-export paths. Python and TypeScript SDKs retain working implementations. See the [Cross-Language Feature Parity](./README.md#cross-language-feature-parity) section of the README for the tracked gap and reintroduction plan.

## [0.17.1] - 2026-04-06

### Added

- **`build_minimal_strategy()`** — 4-step pipeline (context → lookup → execute → return) for pre-validated internal hot paths.
- **`resolve_strategy_by_name()`** — Resolves preset strategy names (`"standard"`, `"internal"`, `"testing"`, `"performance"`, `"minimal"`) to `ExecutionStrategy` instances. Cross-language parity with Python/TypeScript string-based resolution.
- **`Executor::with_strategy_name()`** — Constructor accepting a strategy name string instead of an `ExecutionStrategy` instance.
- **`requires()` / `provides()` on `Step` trait** — Optional advisory methods declaring step dependencies. `ExecutionStrategy` validates dependency chains at construction and insertion, emitting `tracing::warn!` for unmet requirements.
- **`Module::stream()` / `Module::supports_stream()`** — Default trait methods enabling streaming module execution. Returns `Option<Result<Vec<Value>>>` — `None` signals fallback to `execute()`.

### Fixed

- **`BuiltinExecute` global deadline clamp** — Effective timeout is now `min(default_timeout_ms, remaining_global_deadline)`, matching Python/TypeScript dual-timeout model. Returns `ModuleTimeout` immediately if global deadline already exceeded.
- **Streaming support** — `Executor::stream()` now implements the three-phase streaming protocol (pipeline → chunk collection → post-stream validation) instead of wrapping `call()` in a `vec![]`.

---

## [0.17.0] - 2026-04-05

### Added

- **Step Metadata**: Four default trait methods on `Step`: `match_modules()`, `ignore_errors()`, `pure()`, `timeout_ms()` with sensible defaults.
- **YAML Pipeline Configuration**: `register_step_type()`, `unregister_step_type()`, `registered_step_types()`, `build_strategy_from_config()` in new `pipeline_config` module. Uses `OnceLock<RwLock<HashMap>>` global registry.
- **PipelineContext fields**: `dry_run`, `version_hint`, `executed_middlewares`, plus executor resource injection (`registry`, `config`, `acl`, `approval_handler`, `middleware_manager` as `Arc`).
- **StepTrace**: `skip_reason: Option<String>`.
- **Builtin steps with real execution logic**: All 11 steps now contain production-grade logic (was macro-generated no-ops).

### Changed

- **Step order**: `BuiltinMiddlewareBefore` now runs BEFORE `BuiltinInputValidation`.
- **Executor delegation**: `call()` and `validate()` fully delegate to `PipelineEngine::run()`. Removed ~740 lines of inline step code. `strategy` field is now non-optional (`ExecutionStrategy`, not `Option<ExecutionStrategy>`).
- **Renamed**: `safety_check` → `call_chain_guard`, `BuiltinSafetyCheck` → `BuiltinCallChainGuard`.
- **Registry**: Module storage changed from `Box<dyn Module>` to `Arc<dyn Module>` for sharing with pipeline context.

### Fixed

- Middleware input transforms were never validated against schema.
- `validate()` now uses pipeline dry-run mode.

---

## [0.16.0] - 2026-04-05

### Added

- **Config Bus**: `EnvStyle` enum (Auto/Nested/Flat), `max_depth`, `env_prefix` auto-derivation, `env_map: Option<HashMap<String, String>>`, `Config::env_map()`, `ConfigEnvMapConflict` error code.
- **Context**: `ContextKey<T>` with `Cow<'static, str>` for zero-alloc static keys and `scoped()` for per-module sub-keys. Built-in key constants. `Context.serialize()`/`deserialize()` with `_context_version: 1`.
- **Annotations**: `extra: HashMap<String, Value>` with `#[serde(flatten)]` for unknown key capture.
- **ACL**: `ACLConditionHandler` async trait. `ACL::register_condition()` with global `RwLock` registry. `$or`/`$not` compound operators. `async_check()` returning `Result<bool, ModuleError>`. Fail-closed for unknown conditions.
- **Pipeline**: `Step` async trait, `StepResult`, `PipelineContext`, `PipelineTrace`, `ExecutionStrategy`, `PipelineEngine`. 11 `BuiltinStep` structs (via macro). Preset strategies (standard/internal/testing/performance). `Executor::with_strategy()`, `call_with_trace()`, `describe_pipeline()`.

### Changed

- `system.control` module extracted into dedicated `control.rs` file.

### Fixed

- **`ApprovalRequest` spec alignment** — Added required `context: Option<Context<Value>>` field and changed `annotations` from `HashMap<String, Value>` to `ModuleAnnotations` per spec §7.3.1.
- **`DependencyInfo` field rename** — Renamed `name` to `module_id` for cross-SDK consistency with Python/TypeScript.
- **Config env fallback path** — Fixed namespace-mode `APCORE_*` env var fallback to resolve to top-level dot-paths instead of incorrectly prepending `apcore.` namespace prefix.
- **`config_env` conformance test** — Added missing `config_env.json` conformance test (was 9/10, now 10/10 fixtures).
- Removed non-spec Context fields: `created_at`, `parent_trace_id`, `trace_context`.
- `global_deadline` changed from `Option<Instant>` to `Option<f64>` (epoch seconds).
- `Identity` fields made private with pub getters (`id()`, `identity_type()`, `roles()`, `attrs()`). Serde compat via `IdentityRaw` deserialization pattern.
- Empty `callers` list in ACL rules now matches none (aligned with Python/TypeScript).

---

## [0.15.1] - 2026-03-31

### Changed

- **Env prefix convention simplified** — Removed the `^APCORE_[A-Z0-9]` reservation rule from `Config::register_namespace()`. Sub-packages now use single-underscore prefixes (`APCORE_MCP`, `APCORE_OBSERVABILITY`, `APCORE_SYS`) instead of the double-underscore form. Only the exact `APCORE` prefix is reserved for the core namespace.
- Built-in namespace env prefixes: `APCORE__OBSERVABILITY` → `APCORE_OBSERVABILITY`, `APCORE__SYS` → `APCORE_SYS`.

---

## [0.14.0] - 2026-03-24

### Breaking Changes
- Middleware default priority changed from `0` to `100` per PROTOCOL_SPEC §11.2. Middleware without explicit priority will now execute before priority-0 middleware.
- `use_middleware()` now returns `Result<(), ModuleError>` (previously returned nothing)
- Metric names changed: `apcore_calls_total` → `apcore_module_calls_total`, `apcore_errors_total` → `apcore_module_errors_total`, `apcore_duration_seconds` → `apcore_module_duration_seconds`

### Added
- **Middleware priority** — `Middleware` trait now has `fn priority(&self) -> u16` (default 0). Higher priority executes first; equal priority preserves registration order.
- **Input validation (Step 6)** — JSON Schema validation of inputs against `module.input_schema()` using `jsonschema` crate
- **Output validation (Step 9)** — JSON Schema validation of outputs against `module.output_schema()`
- **Dual-timeout enforcement** — `global_timeout_ms` now propagated via `Context.global_deadline`; effective timeout is `min(per_module, remaining_global)`
- **Approval error differentiation** — `rejected` → `ApprovalDenied`, `timeout` → `ApprovalTimeout`, `pending` → `ApprovalPending` (previously all mapped to `ApprovalDenied`)
- **`_approval_token` Phase B** — Token stripped from inputs, `check_approval()` called instead of `request_approval()`; non-string tokens rejected with error
- **Sensitive field redaction** — `redact_sensitive()` function handles `x-sensitive` schema fields and `_secret_` prefix keys; populates `context.redacted_inputs`
- **`LoggingMiddleware`** — New middleware (priority 700) with configurable `log_inputs`/`log_outputs`/`log_errors` flags, duration tracking, and redacted input support
- **`ContextLogger` `_secret_` redaction** — Keys prefixed with `_secret_` are now redacted in JSON log output
- **Priority range validation** — `add()` returns `Result` and rejects priority > 1000

### Fixed
- **TracingMiddleware rewrite** — Replaced `HashMap<String, Span>` with stack-based `Vec<Span>` per trace_id for correct nested module-to-module parent-child span linking; merged dual mutexes into single `TraceState` to eliminate TOCTOU race
- **`increment_errors` signature** — Added `error_code` parameter to match Python/TypeScript/spec
- **Sampling strategy naming** — Added explicit serde renames: `Always` → `"full"`, `Probabilistic` → `"proportional"`, `ErrorFirst` → `"error_first"`, `Never` → `"off"` to match cross-language convention
- **Preflight in validate()** — `validate()` now calls `module.preflight()` and returns `ValidationResult` with warnings (diagnostic, non-blocking), matching Python behavior
- **Metric names** — Renamed `apcore_calls_total` → `apcore_module_calls_total`, `apcore_errors_total` → `apcore_module_errors_total`, `apcore_duration_seconds` → `apcore_module_duration_seconds` to match cross-language convention
- **Step numbering** — Fixed duplicate step numbers in `call()` and `validate()` executor methods

## [0.13.1] - 2026-03-22

### Changed
- Rebrand: aipartnerup → aiperceivable

## [0.13.0] - 2026-03-12

Initial Rust release. Implements the full apcore protocol specification in Rust,
feature-aligned with `apcore-python` 0.13.0.

### Added

#### Core
- **`Module` trait** — Async `execute` with `input_schema` / `output_schema`, `description`, `annotations`, `preflight`
- **`ModuleAnnotations`** — Behavioral metadata: `readonly`, `destructive`, `idempotent`, `cacheable`, `cache_ttl`, `cache_key_fields`, `paginated`, `pagination_style`, `sunset_date`, `tags`, `examples`, `metadata`
- **`ModuleExample`** — Named input/output pair for AI-perceivable documentation
- **`APCore`** client — `register`, `unregister`, `call`, `stream`, `use_middleware`
- **`Config`** — Load from YAML / JSON file, `get` / `set` values
- **`Context<T>`** — Request context with `trace_id`, `identity`, `call_chain`, `cancel_token`, `metadata`
- **`ContextFactory`** — Builder for execution contexts
- **`Identity`** — Caller identity with `id`, `name`, `roles`, `attributes`
- **`Executor`** — Execution engine with middleware pipeline, ACL enforcement, approval gate, call-depth guard, timeout

#### Access Control & Approval
- **`ACL`** — Pattern-based, first-match-wins rules with wildcard support
- **`ACLRule`** — Rule entry with caller patterns, target patterns, effect (`allow`/`deny`), and priority
- **`ApprovalHandler`** trait — Pluggable async approval gate
- **`AutoApproveHandler`** / **`AlwaysDenyHandler`** / **`CallbackApprovalHandler`** (planned) — Built-in handlers
- **`ApprovalRequest`** / **`ApprovalResult`** — Request/response types for the approval pipeline

#### Middleware
- **`Middleware`** trait — `before` / `after` / `on_error` pipeline hooks
- **`BeforeMiddleware`** / **`AfterMiddleware`** — Single-phase adapter types
- **`MiddlewareManager`** — Ordered middleware chain execution
- **`ObsLoggingMiddleware`** — Structured context-aware logging
- **`RetryMiddleware`** — Automatic retry with configurable backoff
- **`ErrorHistoryMiddleware`** — Records errors into `ErrorHistory` ring buffer
- **`PlatformNotifyMiddleware`** — Emits events on error-rate / latency threshold breaches

#### Observability
- **`TracingMiddleware`** — Distributed tracing with span lifecycle and pluggable `SpanExporter`
- **`Span`** / **`SpanExporter`** trait — W3C-compatible span model
- **`StdoutExporter`** / **`InMemoryExporter`** — Built-in exporters
- **`MetricsCollector`** / **`MetricsMiddleware`** — Call count, latency, and error-rate metrics
- **`ContextLogger`** — Context-aware structured log sink (`info`, `warn`, `error`)
- **`ObsLoggingMiddleware`** — Middleware wrapper around `ContextLogger`
- **`ErrorHistory`** — Fixed-capacity ring buffer with per-error-code querying
- **`UsageCollector`** / **`UsageMiddleware`** — Per-module call statistics and hourly trend data

#### Schema
- **`SchemaLoader`** — Load schemas from YAML files or inline `serde_json::Value`
- **`SchemaValidator`** — Validate data against JSON Schema (strict / lenient modes)
- **`SchemaExporter`** — Export schemas for MCP, OpenAI, Anthropic, and generic targets via `ExportProfile`
- **`RefResolver`** — Resolve `$ref` references in JSON Schema documents

#### Registry
- **`Registry`** — Module storage with `register`, `unregister`, `get`, `list`, `watch`
- **`ModuleDescriptor`** — Metadata envelope: id, version, tags, source path, `sunset_date`
- **`Discoverer`** trait — Pluggable module discovery backends

#### Events & Extensions
- **`EventEmitter`** — Async event bus with pattern-based subscribe / emit / flush
- **`ApCoreEvent`** — Typed event (module lifecycle, errors, config changes)
- **`WebhookSubscriber`** / **`A2ASubscriber`** (planned) — Built-in event delivery subscribers
- **`ExtensionManager`** — Unified extension point registry for discoverers, middleware, ACL, approval, exporters, and validators

#### Async Tasks & Cancellation
- **`AsyncTaskManager`** — Background module execution with status tracking, cancellation, and concurrency limiting
- **`TaskInfo`** / **`TaskStatus`** — Task lifecycle state machine
- **`CancelToken`** — Cloneable, shared cooperative cancellation signal

#### Bindings & Utilities
- **`BindingLoader`** — Declarative YAML module registration without modifying source code
- **`BindingDefinition`** — Schema + metadata for a YAML-bound module
- **`TraceParent`** / **`TraceContext`** — W3C `traceparent` header injection and extraction for distributed tracing interop
- **`ErrorCode`** enum — 37 variants covering the full protocol error taxonomy
- **`ModuleError`** — Structured error with code, message, and optional cause chain

#### Developer Experience
- 8 integration test files covering `CancelToken`, `Identity`, `Context`, `Module` trait, `ACL`, `Registry`, `TraceContext`, `ErrorCode`
- 5 runnable examples: `simple_client`, `greet`, `get_user`, `send_email`, `cancel_token`
