# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0a1]

### Added

- Add the `stream_with_completion()` handle for a single invocation's chunks, validated output, and terminal outcome.
- Add owned shutdown and removal operations, managed drain and recovery hooks, and opt-in governed reload reports.
- Expose Prometheus text rendering without starting an HTTP listener.
- Expose `APCore(fail_on_error=True)` and `sys_modules_errors` for system-module startup, and `Executor.ungated_approval_modules()` for approval readiness. ([#230](https://github.com/aiperceivable/apcore/issues/230), [#231](https://github.com/aiperceivable/apcore/issues/231))

### Changed

- Resolve relative filesystem configuration values against `Config.project_root`. The root stays fixed across configuration reloads and working-directory changes. `Config.resolve_path()` exposes the same resolution to consumers. ([#113](https://github.com/aiperceivable/apcore/issues/113))

### Fixed

- Send governance events through the enabled APCore event bus and normalize typed module results consistently across calls and streams. ([#125](https://github.com/aiperceivable/apcore/issues/125), [#126](https://github.com/aiperceivable/apcore/issues/126))
- Support I/O-backed task stores in synchronous task-manager calls made outside a running event loop. ([#127](https://github.com/aiperceivable/apcore/issues/127))
- Concurrent sync executor calls reserve the cached event loop atomically and use independent bridge loops when it is busy. ([#232](https://github.com/aiperceivable/apcore/issues/232))
- Derive the sync thread-bridge wait from the call deadline and support explicit per-call, executor, and Python Config Bus overrides. The pipeline still enforces its own timeouts. ([#229](https://github.com/aiperceivable/apcore/issues/229))
- Ship `py.typed` with the wheel so downstream type checkers can use APCore's inline annotations. ([#233](https://github.com/aiperceivable/apcore/issues/233))


## [0.33.0] - 2026-11-18

### Added

- `APCore.module()`, the package-level `@module` decorator, `FunctionModule`,
  and binding files accept a
  first-class `resources` block (`timeout` in milliseconds). The executor
  already read `module.resources`; the factory dropped it, so a declared budget
  silently fell back to `executor.default_timeout` (30 s by default). Binding
  files needed the matching `resources` key in the binding schema.

### Changed

- Regenerate `uv.lock` against the current dependency graph (`jsonschema`
  4.26.0, `referencing` 0.37.0, `rpds-py` 2026.9.1, `pydantic` 2.14.3, and
  other patch updates). Runtime dependency constraints stay the same.

### Fixed

- `MODULE_TIMEOUT` raised by a module call is no longer retryable by default.
  It carries `retryable: True` only when the module declares
  `annotations.idempotent`. This includes deadlines that expire after a stream
  has emitted chunks. Step-budget timeouts keep the spec default.

## [0.32.2] - 2026-10-19

### Fixed

- Preserve optional enum, const and composed-schema constraints during A23 strict conversion by wrapping the complete schema in a nullable union. Required fields remain unchanged, null-only types are not duplicated, and shared conformance cases verify both schema parity and value validation. Tracked in [apcore#124](https://github.com/aiperceivable/apcore/issues/124).

## [0.32.1] - 2026-10-14

### Fixed

- Honor `x-sensitive` annotations through local schema references, anchors, embedded resource IDs, recursive schemas, arrays and composition branches without external retrieval. Keep the original data and schema unchanged; public redaction signatures and default rules remain compatible. Tracked in [#35](https://github.com/aiperceivable/apcore-python/issues/35).
- Declare the referencing runtime dependency used by local schema traversal.

## [0.32.0] - 2026-10-08

### Added

- **`canonicalize_name()` converts bare ASCII names into canonical segments and preserves the original alongside structured diagnostics** (apcore-rust #40, protocol §2.7). Exported with immutable `CanonicalNameResult` and `CanonicalNameError` from `apcore` and `apcore.utils`. Unicode is rejected, existing underscore runs are preserved, and names are never truncated. A02 module-ID normalization is unchanged; namespace reservation and collision handling remain registration concerns.

### Security

- **An executor's ACL, `ApprovalHandler` and `ExecutionPolicy` are bound into the built-in `acl_check` / `approval_gate` steps of every strategy it runs** — default, preset name, `pipeline:` section, registered name, pre-built instance, or per-call override — located by type; a provider the executor was not given leaves the step's own. `governance_state()` reports what the running gate holds (D-129, spec §6.6.5.5). Previously a pre-built strategy received none of them: a deny-all ACL let every call through while `acl_configured` and `builtin_acl_gate_wired` both read `true`.
- **`ignore_errors: true`, any `match_modules`, and `pure: true` on `approval_gate` are rejected on the built-in gate steps** with `PIPELINE_CONFIGURATION_ERROR` naming the step and key — from `pipeline.configure` and from assigning the field on the step object (D-130, spec §5.16.1). `timeout_ms` and default values stay accepted.
- **`ObsLoggingMiddleware` and `LoggingMiddleware` log only `context.redacted_inputs` / `redacted_output`**, never the raw inputs or output, so an `x-sensitive` field is redacted with or without a `RedactionConfig`; nothing captured means nothing logged (D-131, spec §10.6.1 requirement 5). `ObsLoggingMiddleware.after` previously logged the raw output.

### Fixed
- **CI checks out the canonical spec's `main` independently of SDK branch names and verifies declared fixtures before the full test run.** An incomplete checkout fails with publication-order guidance: publish the spec changes before the SDK changes, then rerun CI. Missing canonical fixtures are never skipped or replaced with private copies.
- **Timeouts cancel a linked, invocation-owned cooperative token and return without waiting for module cleanup** (D-133). Late outputs and errors are discarded; cancellation never cancels an ancestor token or reverses external costs.
- **Preflight preserves all evaluable checks after a failure while withholding module hooks from ACL-denied callers** (D-134). Null previews emit no check (D-141).
- **Strict JSON input validation accepts contract-valid datetime, UUID and enum strings without enabling coercion** (D-136); native Python values remain supported.
- **Binding files reject unknown top-level and entry keys** (D-139).
- **Anthropic exports preserve `x-*` property names while removing extension keywords; MCP approval and streaming hints are emitted only when true in `_meta`** (D-140).
- **Standard bootstrap delivers exactly one contextual ephemeral registry audit event, including bare `ephemeral` IDs and null identities** (D-148).
- **Native validation errors serialize canonical JSON Pointer `path`, JSON Schema `keyword`, and `message` fields** (D-149).
- **The eight issue #123 canonical fixtures are now driven through public SDK paths** (47 cases), with additional timeout, native-input, JSON Pointer, and subscriber circuit-breaker regressions.

- **`module(...)` / `client.module(...)` accept a bound method** — it gets a transparent forwarder carrying `apcore_module`; previously the module was registered and then `AttributeError` was raised (#123).
- **An auto-derived module ID for a function defined in `__main__` raises `INVALID_MODULE_ID` asking for an explicit `id`**, before anything is registered, instead of producing the illegal ID `__main__.<name>` (#123).
- **Hot reload registers a newly created file under the ID `discover()` would give it** — path relative to its root, namespace prefix, ID-map override — instead of its bare basename; deletion unregisters by the same ID (#123).
- **The legacy `RedactionConfig.value_patterns` rule tests string values only**, never a stringified number or container (§10.6.1 requirement 2) (#123).
- **`AsyncTaskManager.get_result_async()` added** — the `get_result` counterpart for I/O-backed stores, alongside `get_status_async` / `list_tasks_async` (#123).
- **`Executor.set_acl()` logs a warning when the running strategy has no built-in `acl_check` step**, as apcore-typescript does (#123).


## [0.31.0] - 2026-09-22

### Added

- **`TaskStoreError(code=TASK_STORE_UNAVAILABLE)` now exists and is exported** (async-tasks.md, D-92). Declared on every `TaskStore`/`AsyncTaskManager` surface but implemented in no SDK; now a `ModuleError` subclass.
- **`ACL.check_access`/`async_check_access`/`check`/`async_check` accept a `projection` keyword** (CTX-2, see Fixed).
- **A configured `pipeline:` section is applied** (D-72) — `remove:`/custom steps in `apcore.yaml` previously had no effect.
- **The five `observability.tracing.*` keys are wired** (D-68) — `enabled`/`exporter`/`strategy`/`sampling_rate`/`otlp_endpoint` now install and configure `TracingMiddleware`.
- **`ACL.load(path, audit_logger=…)`** (D-66).
- **`extensions.roots` is read**, both element shapes, with namespace isolation (D-70).
- **`id_map.overrides` is read at registry construction** (D-71).
- **The six `validation.*` limits are wired**, unconstrained by default (D-118 config-surface audit) — purely additive.

### Changed

- **`AsyncTaskManager.start_reaper` raises `ModuleError(REAPER_ALREADY_RUNNING)` instead of `RuntimeError`** (A-C-004). **Breaking:** catch `ModuleError` instead.
- **An unknown extension point raises a coded error** — `KeyError` → `InvalidInputError(GENERAL_INVALID_INPUT)` (D-108). **Breaking.**
- **The deprecation-warning dedupe key now carries the notice and survives `unregister`** (D-89, v1.59.0) — a changed `x-deprecation` block now re-warns; an unchanged one on hot-reload does not.
- **`APCore` adopts a supplied Executor's Registry instead of building a second one** (CLI-1) — fixed a split where `register`/`call` addressed different registries.
- **The client's `Config` now reaches the auto-created Registry** (CLI-3) — `extensions.*`/`id_map.overrides` were previously inert through `APCore`.
- **A per-step `timeout_ms` now takes the ordinary step-error path** (EXE-006) — previously bypassed `on_step_error` and recovery.
- **`ExecutionStrategy.replace`/`configure_step` reject a duplicate step name** (EXE-007).
- **The three `system.control.*` modules declare `idempotent` explicitly** (SYS-19) — `toggle_feature` was wrongly defaulted `False`.
- **A bulk `system.control.reload_module` failure is now fatal** (SYS-17) — previously reported success after unregistering modules it failed to restore.
- **`apcore.registry.callback_errors` renamed to `apcore_registry_callback_errors_total`** (OBS-007) — the old dotted name broke Prometheus scraping entirely.
- **`guard_call_chain` raises a typed error instead of `ValueError`** (D-84). **Breaking.**
- **A deprecation warning now fires at most once per `(module_id, version)` per registry** (D-89) — previously fired on every `get_definition` read.
- **`Registry.register` reports validation failures in the spec's documented order** (D-86) — the three SDKs had drifted into three different orders.
- **`AsyncTaskManager.shutdown()` now attempts every task's cancellation before reporting** (D-122) — previously stopped at the first failure, leaking active `max_tasks` slots.
- **A failing ACL audit callback no longer changes the access decision** (D-66) — auditing must not hold a veto over access.
- **`_config.allow_unknown: false` now drops unregistered namespaces** (D-69) — previously a no-op; only affects configs that explicitly set it.

### Fixed

- **A symlinked module is discovered once, under its real path, instead of registering twice** (D-127).
- **A malformed annotation value (`extra`/`cache_ttl`) no longer takes the whole registration down** (D-115).
- **`Executor.remove()` now clears the middleware duplicate-identity entry** (D-114) — a legitimate use/remove/use swap warned about a phantom duplicate.
- **`system.manifest.full` reports `project_name` correctly, and every `system.*` module declares `open_world: false`** (D-110/D-119).
- **SECURITY: a governance requirement declared in module metadata no longer bypasses the approval gate** (D-125) — preflight and the live descriptor disagreed, so the gate never fired.
- **SECURITY: a `$ref` into an external schema could bind to the caller's local definitions**, including past an `x-sensitive` marking (D-124) — the fallback is now scoped to the document it was declared for.
- **`global_deadline` now uses epoch seconds, belongs to the per-call context rather than a reused caller Context, and is recomputed for a cross-process call** (D-99/D-101/D-102) — the deadline previously never fired.
- **`_approval_token` no longer reaches policy resolution from `Executor.validate()`** (APR-2) — it leaked into the audit trail and policy-override payload.
- **A local `#/…` `$ref` resolves against the schema file root, then falls back to the schema node** (D-104) — the SDKs previously accepted opposite layouts.
- **`Identity.roles` is copied and frozen at construction** (IDN-2) — a caller mutating its own list after construction previously leaked into the identity.
- **`Context.deserialize` raises a typed error on a malformed wire identity** instead of a bare `KeyError`/`TypeError` (CTX-1).
- **The governance projection is scoped to one ACL evaluation** via a `projection` keyword, instead of leaking onto a reused execution `Context` (CTX-2).
- **A filtered subscriber wrapped in a circuit breaker no longer widens to catch-all**, and now forwards the wrapped subscriber's id/type (EVT-001).
- **A subscriber with no declared id gets a generated `{type}-{N}` id** instead of a heap-address `repr()` (EVT-004).
- **Hot reload no longer drops `_module_meta`** — a reloaded module previously came back with an empty descriptor, silently degrading dependency-ordered reloads to alphabetical.
- **A scalar `dependencies:` value no longer aborts the whole `discover()` pass.**
- **`cancel()` no longer returns `False` for an active task with no in-process handle** — every such task, typical after a process restart, was previously uncancellable.
- **The async-task runner no longer clobbers an out-of-band CANCELLED with FAILED, or resurrects a terminal record as PENDING.**
- **The reaper now calls `TaskStore.list_expired` instead of a full-table `cleanup()` scan.**
- **The §10.6.1 sensitive-key redaction rule, previously duplicated in two modules, now has one implementation.**
- **Module-level metrics extraction, previously duplicated in two modules, now has one implementation.**
- **Two dead private constants removed** (`_CASE_BOUNDARY`, `_REDACTED`).
- **Three docstrings claiming nonexistent peer APIs corrected.**
- **`ACL.add_rule`/`remove_rule` now clear the §6.5 warning-dedupe set** (D-88) — a stale marker suppressed or misattributed the "no context" warning.
- **An over-length module ID is now rejected before the pipeline runs**, instead of reaching the registry and reporting `MODULE_NOT_FOUND` (D-75).
- **`Registry.describe` no longer returns a Python `repr()`** for a module supplying its own `describe()` (D-77).
- **A hot-reloaded module is no longer visible before its `on_load()` has run** (D-123).
- **Four pattern-valued config surfaces (bindings, redaction keys, event patterns, `path_filter`) previously used three different glob dialects** — unified under `apcore.utils.pattern.match_glob` (Algorithm A25, #116/#117).
- **An ACL pattern containing `?` now warns** instead of silently matching nothing (#116).
- **An uncompilable `obs.redaction.regex_patterns` entry is now reported once, at config-validation time**, instead of silently discarded per log record (#117).
- **`stream.max_merge_depth` is now read from config** instead of ignored; a misconfigured value still falls back to the safe default of 32 (#118).
- **The four per-group `sys_modules.*` registration flags are now read** — `control.enabled: false` previously still registered the gated `system.control.*` modules (#118).

### Deprecated

- **`acl.default_effect` in `apcore.yaml`** — reaches nothing; the ACL file's own `default_effect` is authoritative. Removed at v2.0 (D-73).
- **`Context.logger`** — no configuration door; use the host application's own logger. Removed at v2.0 (#121).
- **`logging.level` and `logging.format`**, no replacement — apcore does not own the host's logging policy (D-67).
- **Ten declared config keys that reach no consumer in any SDK** are deprecated for removal at 2.0 (`observability.tracing.*`, `observability.metrics.*`, `logging.*`, `acl.audit.*`) — parsing is unchanged; each now warns once per load (#118).

---

## [0.30.0] - 2026-09-06

### Fixed

- **The §9.2.2 deprecation warning was suppressed after its first emission from a given call site (PROTOCOL_SPEC §9.2.2 requirement 2 "Cadence", spec v1.36.0, [apcore#113](https://github.com/aiperceivable/apcore/issues/113)).** v1.35.0 required the warning but not its cadence; v1.36.0 fixes it at **once per configuration load, never once per process**, and forbids suppressing it with process-global state. This SDK never wrote such a flag — `warnings.warn` supplied one: under Python's `"default"` filter action it records `(message text, category, lineno)` in the calling frame's `__warningregistry__` and drops a repeat. Two *different* affected configurations were unaffected, because the message embeds the project root, so the obvious test does not detect this. What was silenced is the case §9.2.2 names in the same breath — the same document loaded again from the same source line, "a reload that re-reads an edited file" — and, in production, every caller after the first of an application's own `load_config()` wrapper. The warning is now emitted through `warnings.warn_explicit` with no registry, so no de-duplication state survives the call while every filter the host set still applies exactly as before: `-W error` still raises, `-W ignore` still silences, `catch_warnings(record=True)` still records. No filter is mutated, at import time or at load time.

- **The missing-binding-directory error described a directory as a file (PROTOCOL_SPEC §5.12.6 clause 5, spec v1.36.0, [apcore#114](https://github.com/aiperceivable/apcore/issues/114)).** Clause 5 is new — a resolved binding directory that does not exist **MUST** raise, naming that directory, and **MUST NOT** return an empty result — and this SDK already satisfied it. The `BindingFileInvalidError` reason is now `"Resolved binding directory does not exist"` rather than `"Directory does not exist"`, so the message no longer reads as a complaint about a file. The error code (`BINDING_FILE_INVALID`), the `file_path` detail and the raise itself are unchanged. New tests cover the clause on all three provenances it names — an explicit directory argument, `bindings.dir` from the file and from the environment, and the `./bindings` default reached both through the loader's own last tier and through the merged default table.

- **A set-but-empty `APCORE_*` variable silently blanked a path-typed directory (PROTOCOL_SPEC §9.2.1 requirement 5, spec v1.36.0, [apcore#113](https://github.com/aiperceivable/apcore/issues/113)).** §9.2 treats a variable that is *set* as an override whatever it holds, so `export APCORE_ACL_ROOT=` — or a variable inherited empty from a container spec — replaced a directory the configuration file correctly declared with `""`. Because `""` is a legal relative path to the filesystem API, nothing failed: resolution landed on the process working directory, which is never the directory the operator meant. An empty value in a path-typed key (§9.2.1's closed set — `acl.root`, `bindings.dir`, `extensions.root`, `schema.root`, and the elements of `extensions.roots`) is now discarded and resolution falls through to the next tier exactly as if nothing had been set: the environment tier falls through to the file, the file tier to the default table. A warning naming the key is logged (§9.2.1 requirement 5's MAY). The guard is one value wide — exactly `""`, not whitespace and not any other falsy value — and it covers the path-typed set only, so `bindings.pattern`, which sits beside `bindings.dir` and is deliberately not path-typed, may still be set empty.

### Added

- **`bindings.dir` and `bindings.pattern` are now canonical defaults (`schemas/defaults.schema.json`, spec v1.36.0, [apcore#114](https://github.com/aiperceivable/apcore/issues/114)).** `defaults.schema.json` never carried a `bindings` section, so the `"./bindings"` / `"*.binding.yaml"` values §5.12.6 promises were unreachable from the mechanism meant to serve them: `Config.get("bindings.dir")` answered `None` for a fully loaded `Config`, and each SDK kept the canonical value as a private literal inside its own binding loader. The spec repo added the section and regenerated `config_key_governance.json` (65 allowed keys, 20 canonical defaults), which pins every SDK's default table against it; `_DEFAULTS` gains both keys to match. `BindingLoader.load_binding_dir` keeps its own last tier — `config` is optional and an in-memory `Config(data=...)` never merges `_DEFAULTS`, so the loader is still the only place that can supply a default on those two paths — but it now reads that tier from `Config.get_default()` instead of a literal, so the two cannot disagree. Visible consequence: a relative `bindings.dir` is now present in every merged configuration, so it is counted by §9.2.2's deprecation-warning condition alongside `extensions.root`, `schema.root` and `acl.root`.

---

## [0.29.0] - 2026-09-05

### Added

- **`ApprovalRequest.caller_id` and `ApprovalRequest.action` (spec decision D-03, `docs/spec/2026-05-decision-log.md`, PROTOCOL_SPEC §7.3.1).** `docs/features/approval-system.md`'s Contract block already required the request a handler receives to carry `caller_id` and `action`; the dataclass carried neither, so a handler (Slack approver, audit log) that needed "who is asking" or "what module is this" had to traverse `request.context` and re-derive `action` from `request.module_id` under a second name. Both fields are additive with safe defaults (`caller_id: str | None = None`, `action: str = ""`) so no existing keyword-constructed `ApprovalRequest` breaks; `BuiltinApprovalGate` (`builtin_steps.py`) always supplies both explicitly at its one construction site — `caller_id=ctx.context.caller_id` (`None` on a top-level call, read straight off `Context.caller_id` with no `"@external"`-style substitution) and `action=ctx.module_id`. Conformance fixture `approval_request_fields.json` (apcore repo) pins both scenarios; this SDK's own suite covers the same two cases (`test_approval_executor.py`, `test_approval_system_spec.py`).

- **`CancelToken.raise_if_cancelled()` (`docs/features/cancellation.md`, "Contract: CancelToken.raise_if_cancelled").** The spec's Contract block names this method; the Python SDK only had `check()`, an identical-behavior method under a different name, which `tests/test_cancellation_spec.py` had been recording as a documented cross-language naming gap (three skipped clauses). `raise_if_cancelled()` is purely additive — it delegates to `check()`, which remains the primary name used throughout this SDK and by existing external callers and is **not** deprecated. The three previously-skipped spec clauses now run for real against the new method.

### Fixed

- **`ACL(rules=[...])` did not re-validate a rule mutated before it was ever passed in, unlike apcore-typescript and apcore-rust, which already rejected the identical input (PROTOCOL_SPEC §6.1.4.1 / §6.2.1, spec v1.33.0).** `ACLRule.__post_init__` validates a rule's `effect`, `approval` and pattern-array shape once, at true construction time; a rule built well-formed and then mutated (`rule.targets = []`) before ever being handed to `ACL(rules=[rule])` walked straight past that check, because `ACL.__init__` itself ran no validation of its own — relying entirely on `add_rule`'s equivalent check (already fixed in `f6fed56`/`9dd6ed5`), which covers a *different* door. `ACL.__init__` now calls the same `_validate_rule` `add_rule` calls, on every rule it is handed, and raises `ACLRuleError` on the first (lowest-index) invalid one — direct construction is one of the three entry points §6.1.6 rule 3 names, and a rule's construction history is not legible to the door receiving it. **This does not touch `TestPatternArrayArityBackstop`**: every one of its cases now constructs the `ACL` from a well-formed rule *first* and mutates the already-installed rule through the public `rules` accessor afterward — the one route no door runs again to intercept, and precisely the scenario PROTOCOL_SPEC's disambiguated §6.1.4.1/§6.2.1 language now says the backstop is *for*. New `TestConstructionRejectsAPreviouslyMutatedRule` in `tests/test_acl.py` and a new `construct`-door driver (`_door_construct_mutated`) in `conformance/test_acl_pattern_arity.py` (apcore repo) cover the newly-rejected sequence; apcore-typescript and apcore-rust already pass it unchanged.

## [0.28.0] - 2026-08-31

> **Release note:** this section contains BREAKING changes (input validation now
> runs for dict-declared schemas, `p99_latency_ms` changes value, an ACL
> `deny` rule whose condition cannot be evaluated now denies, an ACL rule
> carrying an `effect` outside `allow` / `deny` now fails at construction
> instead of being read as `deny`, and an ACL rule whose `callers` / `targets`
> pattern array carries no operand now fails to load instead of matching
> nothing). It must
> ship as a **minor** (or major) version bump, never a patch.

### Added

- **Argument-scoped approval: an ACL rule can now ask a human about *what a call carried* (spec v1.28.0 §6.1.6-§6.1.8, §6.8.1, §6.9, apcore#108).** Every decision point that could read a call's arguments was unable to escalate it to a human, and the one point that decides whether to ask a human is forbidden by §7.9.6 rule 2 from consulting them. The ACL could **refuse** on arguments and an `ApprovalHandler` could **wave through** on arguments; nothing could **ask**, and a refusal is not a question — so an operator who needed `git push --force` reviewed had to gate *every* `git push`, which dilutes `requires_approval` from "this needs approval" to "this might" and floods the audit trail. Five surfaces land together:

  **`ACLRule.approval` (§6.1.6).** `effect` keeps authorization; an orthogonal optional `approval` field (`"required"` / `"not_required"`, default `"not_required"`) carries the approval requirement, so every rule written before this release keeps its meaning exactly. **`approval: required` on a `deny` rule is rejected with `ACLRuleError`** — "denied *and* put it to a human" is not a state that means anything, and half-applying a governance rule is the failure mode §6.1.5 was written to end. The rejection covers `ACL.load()`, direct `ACLRule(...)` construction and `add_rule()`, because the last two never reach the loader's parser — the same door §6.1.1 case 5 and §6.1.4.1 exist for. `approval` joins the closed `_RULE_KEYS` set; adding it was only *safe* because v1.27.0 closed that set first, since an SDK that still dropped unknown keys would read a `deny`-with-`approval` rule as a bare rule.

  **The built-in `arguments` condition (§6.1.7).** One new condition key with a closed predicate vocabulary — `has_key` (any named key present), `has_all_keys` (every one), `has_none_of` (none) — registered beside `identity_types` / `roles` / `max_call_depth` rather than through a new registration point, because `register_condition` writes runtime code into a process-wide registry and a deployment-registered argument handler is exactly the unauditable host code §7.9.6 rule 2 keeps out of a verdict. Being an ordinary registry entry also gets it §6.1.4's precheck for free: `argument:` written for `arguments:` is an unregistered key, so the rule is unevaluable rather than silently inert. **No predicate reads a value**, and that is a design constraint: redaction is schema-driven so a module with no input schema gets none, and the ACL runs at Step 4 while input validation runs at Step 7, so key presence is the one question well-defined on what is available. A malformed predicate — `has_key: "force"` where a list was meant, an unrecognised predicate name, a non-mapping block, or an **empty** `arguments: {}` that constrains nothing — is **UNEVALUABLE per §6.1.1, never false**: answering "no" would put an `allow` rule's `has_none_of` typo back into the silently-inert state §6.1.1 exists to end. Empty predicate *arrays* are well-formed and vacuous by ordinary quantifier semantics: `has_key: []` is unsatisfied, `has_all_keys: []` and `has_none_of: []` are satisfied. Every one of these checks is context-free and handler-free, so the whole predicate structure is examined in the §6.1.4 precheck — not merely the `arguments` key's registry status — and `validate_rules()` reports it at path `arguments`.

  **The governance projection (§6.1.8).** Computed at pipeline Step 3 (module lookup) and carried to Step 4 on `Context.governance_projection` — the ordering is normative, not an implementation detail that happens to hold. `GovernanceProjection` holds the argument **key set** and each key's JSON type and **has no field a value could live in**; a projection that structurally cannot hold a value cannot leak one, whatever a future predicate does with it. It is deliberately **not** `redacted_inputs`, which §6.1.8 rule 3 forbids substituting: that field's contract is safe *logging*, and with no input schema it is a raw copy of the arguments, values and all. The framework-owned `_approval_token` is excluded, so the ACL reaches the same Step 4 verdict on a `_approval_token` resume as on the call a human just approved. `GovernanceProjection` is exported from the package root; it is transient and does not appear in `Context.serialize()`.

  **`AccessDecision` and the fail-closed boolean (§6.8.1).** `ACL.check_access()` / `ACL.async_check_access()` return `access` / `approval_required` / `matched_rule_index` / `reason`. `check()` and `async_check()` are kept, and **the legacy boolean now returns `False` for an allow-with-approval-required decision**: `check()` is consumed by callers that are not the Executor — tooling, preflight helpers, third-party integrations — and such a caller can only read a boolean as "let it through", so returning `True` would run a call the ACL said needed a human. `False` is wrong in the benign direction. A legacy caller meets this at all only once an operator has authored a rule carrying `approval`.

  **`AuditEntry.approval_required` (§6.3.1).** A new boolean beside `decision`, not a third `decision` value: `decision` is a string downstream consumers parse, and a third value would break every existing parser. `False` when no rule matched or the matched rule required none.

- **`ACL.default_effect` and `ACL.rules` read-only accessors (spec v1.23.0 §6.8, apcore#101).** `acl.py` defined no `@property` at all, so neither the ordered rule list nor `default_effect` — the single most consequential value in an ACL, the one §6.1 carries a `danger` admonition about — could be read back from the loaded object. Tooling that reports or audits the enforced policy had to re-read and re-parse the ACL file to recover a value the object already held, and that second copy could drift across `reload()`. Both are pure reads: no audit event, no mutation, no lock the caller has to release. `rules` returns an immutable `tuple` taken under the same snapshot discipline `check()` uses, so a caller cannot reach through it into the ACL's own list, and both reflect a `reload()`.

- **`ACL.validate_rules()` (spec §6.1.2 rule 3 / §6.1.3 / §6.1.4, apcore#100, #106).** Returns a possibly-empty tuple of `RuleValidationFinding`, one per rule that fails the §6.1.4 precheck — an unresolvable condition key on the sync path, a malformed `$or` / `$not` operand, a non-mapping `conditions`, or a `callers` / `targets` that is not a list of strings, with faults nested inside `$or` / `$not` included. Each finding carries `rule_index`, `condition_path`, `condition_key`, `effect`, `sync_resolvable` and `async_resolvable`, ordered by rule index and then lexicographically by path.

  It is named `validate_rules`, not `validate_conditions`, because it reports structural faults in `callers` and `targets` too — the narrower name could not cover §6.1.4.1. The two flags are reported **separately** and are not collapsed into one boolean, and they mean **resolvable on that evaluation path**, not "present in that registry": `async_check()` falls back to the sync registry, so `async_resolvable` is the union of both and every built-in leaf handler is resolvable on both paths. A finding is emitted whenever `sync_resolvable` is false, **including** when `async_resolvable` is true. Pure read: it does not mutate the ACL, register handlers, or emit an audit event.

  The intended shape is to call this once bootstrap has finished registering handlers and to treat any finding on a `deny` rule as a startup error. The guarantee that a broken `deny` rule cannot silently pass traffic does not depend on anyone calling it — that is the §6.1.1 change below.

- **Loading, constructing or inserting an ACL rule warns for a rule that fails the precheck (spec §6.1.2, apcore#100).** `ACL.load()`, the `ACL` constructor and `add_rule()` each emit a warning naming the **rule index, the condition key and the rule's `effect`** — the `effect` because a misconfigured `deny` rule is the consequential case. It is a warning and never a failure: `register_condition` writes to a runtime, process-wide registry and `acl.root` discovery commonly runs during framework bootstrap ahead of application code, so failing here would reject valid configurations on ordering alone. `add_rule()` performed no validation at all before, which §6.1.2 rule 4 makes an entry point that MUST be covered. A conditional rule skipped for want of a context also warns once per rule/effect (§6.5), because such a rule blocks nothing at all for a caller that supplies no identity and is therefore not a backstop.

- **`ConditionOutcome` and `RuleValidationFinding` are exported from the package root.** A custom condition handler MAY now return `ConditionOutcome.UNEVALUABLE` to report "no answer obtainable" for its own reasons; returning a plain `bool` keeps the historical satisfied/unsatisfied meaning.

- **A structural and registry precheck runs before every rule evaluation (spec v1.25.0 §6.1.4, apcore#100).** Before a rule's conditions are evaluated, the whole `conditions` tree — every branch nested inside `$or` and `$not` — is walked for structural and registry faults, with no context and without invoking any handler. It **runs before** §6.5's "conditions present but no context provided" check, which closes a bypass measured in all three SDKs: `conditions: {mispelled: true}` on a `deny` rule returned `true` for a context-less caller, because the rule was skipped for want of a context before anyone noticed the key was misspelled. A rule that *passes* the precheck and then finds no context still takes §6.5's path and does not match — `roles` is answerable in principle, and this caller merely supplied no input for it. That distinction is the whole design, and a dedicated control test pins it.

  The precheck does **not** short-circuit, and its completeness is what makes `handler_error` deterministic across implementations: because it is context-independent, handler-free and exhaustive, its findings are a pure function of the rule, so every SDK reports the same set in the same order. Diagnostics that originate in handler *execution* — a handler that raises, an async handler on the sync path — carry no such guarantee, because execution may still short-circuit; the existing short-circuit logic is unchanged.

  **The precheck gates; it does not enter the composition table (§6.1.4 rule 5).** A rule that fails the precheck is unevaluable even when a `$or` sibling would have been satisfied — `{"$or": [{unregistered: true}, {"roles": ["dev"]}]}` on a `deny` rule denies a caller who *has* `dev`, because §6.1.1's table governs conditions that were actually **evaluated** and a precheck fault means the rule never got that far. The split is precheck-vs-execution, not structural-vs-everything: the same shape with a *registered* handler that throws passes the precheck, so the table does apply and the satisfied branch wins. Both directions are pinned locally and by the staged fixture.

  **The precheck does not widen a rule's reach (§6.1.4 rule 4).** The structure of `callers` / `targets` is checked first; a *malformed* field makes the rule unevaluable, because its scope is unknowable. But when both fields are well-formed and either fails to match, the rule does not apply to this call at all: its conditions are not consulted, its faults do not reach `handler_error`, and it does not change the decision. Otherwise `callers: ["api.*"]` with `conditions: {mispelled: true}` and `effect: deny` would deny a `worker.*` caller — a typo in a narrowly scoped rule deciding calls it was never written about, which breaks first-match-wins. The fault is still real: `validate_rules()` looks at every rule and no call, and is where a scoped rule's typo surfaces.

- **Findings and `handler_error` now name a condition path, not just a key (spec v1.25.0 §6.1.4).** `k`, `$or[i].k`, `$not.k`, `$` for the `conditions` object itself, `$or[i]` for a malformed `$or` element, and `callers` / `targets` for a malformed pattern field. Paths nest — `$or[1].$not.k`. Ordering is by path rather than by key because a nested `$or` may carry the same key at several positions, which left ordering by key undefined. This also replaces the `$conditions` synthetic key introduced earlier in this cycle with `$`, the JSONPath root, which stays consistent with `$or[1].$not.k` where no root token otherwise appears.

  A fault that is not attached to a key — a malformed pattern field, a non-mapping `conditions`, a malformed `$or` element — reports `condition_key` as **null** and both resolvability flags as **false**. The flags mean "can this fault be resolved by evaluating on that path", not "is the key present in that registry": read as a registry lookup they would report a malformed `$or` *value* as resolvable on both paths, because `$or` itself has a handler. A malformed `$or` element is a fault rather than something to skip — skipping is how a whole `$or` branch disappears without anyone being told.

- **`ExecutionPolicy.resolve()` receives the call site (spec v1.24.0 §7.9.6, apcore#102).** Resolution now additionally takes the invocation `arguments` and the `Context`, as keyword-only optional parameters, and the approval gate (`builtin_steps.py`) and `validate()` preflight (`executor.py`) both pass them. Governance decided on *which* module was being called and never on *what it was being called with*, which forces an operator who needs to gate *some* calls to a module to gate *all* of them — audit noise, and `requires_approval` weakened from "this needs approval" to "this might".

  The built-in pattern rules of §7.9.1 **do not consult** either input, so a rule set's verdict stays a function of the module ID and the annotations alone and remains reproducible from the policy document; adding the call site cannot change the verdict any existing policy produces, and a parametrised test asserts the two `PolicyDecision`s are equal across the rule shapes and annotation shapes. The inputs exist so a **host-supplied** policy — a subclass overriding `resolve` — can decide on them. Those `arguments` have **NOT** been schema-validated: the approval gate is pipeline Step 5 and input validation is Step 7 (§12.8), so an override must not assume them well-formed, present, or of the declared type. `_approval_token` is stripped before the policy sees them, so a protocol-level key never reaches a rule — normative as §7.9.6 rule 5 since spec v1.25.0, because §7.4's "remove before passing to subsequent steps" does not reach a resolution that happens *inside* Step 5.

  Spec v1.25.0 withdrew rule 3(b), which had promised that a host-supplied policy implementation could decide on arguments: `ExecutionPolicy` is a concrete class here and in apcore-typescript and a concrete `struct` in apcore-rust, and `set_policy` takes that concrete type in all three, so no host can supply one. Subclassing happens to work in Python but is not a specified extension point and nothing here builds toward one. Rule 7 now requires the **capability** rather than one API shape, and names keyword-only optional parameters as conforming.

- **`Executor.governance_state()` (spec v1.16.0 §6.6.5, apcore#97).** A read-only accessor returning a `GovernanceState` of eight observations plus one derived flag: what is *configured* on this executor versus what is actually *wired* into the running pipeline. `acl is not None` was never the answer to "what is gating this registry" — the ACL and approval gates are pipeline steps, and the `internal`, `testing` and `minimal` presets all remove them, so an executor can hold an ACL that no step consults.

  Gate detection is by **type** (`isinstance(step, BuiltinACLCheck)`), never by step name: a custom step named `acl_check` that never reads an ACL must not set `builtin_acl_gate_wired`, because a false `True` there reports a gate that is not present — the one direction the flag must never fail in.

  `all_control_modules_require_approval` is a required conjunct of the derived flag, because the two gates are not symmetric (§6.6.5.1.1): `acl_check` evaluates every call, while `approval_gate` returns before consulting the handler when the module does not declare `requires_approval`. It reads the annotation through the same `_module_requires_approval` predicate the gate itself uses, so the accessor cannot disagree with the pipeline it describes.

  The accessor is a pure read — it never enforces, warns, throws or mutates — computes live rather than caching, and returns booleans only: no ACL object, handler or policy leaks out. `GovernanceState` is exported from the package root.

### Changed

- **BREAKING: a pattern list with no operands made an ACL rule inert, and under `default_effect: allow` that permitted the call the rule named (spec v1.31.0 §6.2.1, [apcore#112](https://github.com/aiperceivable/apcore/issues/112)).** `callers` / `targets` of `[]`, `["$or"]` or `["$not"]` can never match; all three SDKs returned `false` from the matcher and `validate_rules()` reported nothing, so a `deny` rule an operator wrote, loaded and validated contributed **nothing** to the decision — with one rule in the ACL the outcome tracked `default_effect` exactly across all twelve combinations of the three shapes, both effects and both defaults. On an `allow` rule that is merely useless. On a `deny` rule under `default_effect: allow` it is a **fail-open**: the call the rule was written to block is permitted, by a rule that loaded without error and a validator that called it clean. Reachable from a plain YAML file, not only from direct construction — `ACL.load` rejects an *omitted* `callers` / `targets` and permitted an *empty* one.

  **Arity is now closed at every entry point**, on the mechanism §6.1.5 chose for [apcore#107](https://github.com/aiperceivable/apcore/issues/107) and [apcore#111](https://github.com/aiperceivable/apcore/issues/111): at least one operand, every element a non-empty string, `$or` at index 0 followed by at least one pattern, `$not` by exactly one, and `$or` / `$not` nowhere but index 0 — rejected with `ACLRuleError` naming the field, and the rule index wherever the entry point has one, at file loading, direct `ACLRule(...)` construction and `add_rule()` alike. One `_pattern_array_fault` predicate serves all of them, threaded exactly as `_validate_effect` is, because two copies of a shape rule are two things that drift. `schemas/acl-config.schema.json` had declared `minItems: 1` and `minLength: 1` on both fields since the file existed and nothing enforced either — the third instance of the same shape, in which the constraint was declared in the schema and no door enforced it, because no implementation validates an ACL file against the schema at load time.

  **Three normative statements are replaced, not reinterpreted.** §6.5's edge-case table required an empty list to make the rule "never match", which is the fail-open above. §6.2.1 required `["$not"]` to "evaluate to false (fail-closed)"; the parenthetical predates §6.1.1 (v1.22.0) and is wrong — a non-match is fail-closed on an `allow` rule and fail-**open** on a `deny` one. And `["$not", p1, p2, …]` was *implementation-defined* — consult `p1`, ignore the rest — which every SDK did, so the form was uniform across implementations and uniformly **wider than written**: `targets: ["$not", "secrets.a", "secrets.b"]` on an `allow` rule **granted** `secrets.b`, the second target the operator excluded. `SHOULD NOT rely on this form` reported nothing and rejected nothing.

  **A pattern array is also stated to be FLAT** for the first time — the operators do not nest and there is no precedence, unlike the same tokens in `conditions`, where `$or[1].$not.k` is a defined path. An operator who learned the condition grammar and wrote `["$or", "$not", "a"]` got an OR of two literals that matched `a` and also matched a module literally named `$not`, violating §6.2.1's own long-unenforced "MUST NOT match a literal module ID equal to `$or`". A reserved token outside index 0 is now rejected, which makes that clause hold **by construction**. Detection is by **equality**, never by a `$` prefix: `["api.*", "$orders.*"]` still loads.

  **The backstop, for the one route no door covers.** `ACLRule` is a non-frozen dataclass, so `rule.targets = []` reaches the evaluator whatever the constructors check — and unlike an unrecognised `effect`, which is never read again once the doors are shut, a mutated pattern array *is* read by the matcher. `_precheck_patterns` therefore classifies it under §6.1.4.1 on the same terms as a malformed *type*: the scope is unreadable, the rule is UNEVALUABLE, and §6.1.1's effect table decides — a `deny` rule takes effect and denies, an `allow` rule does not grant. It never raises out of `check()`; `handler_error` names `callers` / `targets`, a warning is emitted, and `validate_rules()` reports it with a **null** key and both resolvability flags `False`. Both fields are examined without short-circuiting, so a rule faulty on both reports both paths in lexicographic order. §6.1.1 rule 5's "unknowable scope counts as scope" applies unchanged: such a rule carrying `approval: required` still raises the pending approval requirement.

  **`add_rule()` re-validates the rule it is HANDED, whatever that rule's history (spec v1.31.0 §6.2.1).** The closure was threaded through `ACLRule.__post_init__`, which reaches runtime insertion only *through construction* — so a rule that was well-formed when built and has had `callers` or `targets` assigned since walked straight past the door and was installed:

  ```python
  r = ACLRule(callers=["*"], targets=["*"], effect="deny")
  r.targets = []
  acl.add_rule(r)          # inserted without raising; now raises ACLRuleError
  ```

  §6.1.5's v1.30.0 text leaves mutation-then-use to a **MAY** for `effect`, which is sound there because a closed `effect` is never read again; a pattern array **is** read, by the matcher, on the next `check()`, so the rule the ACL then held was UNEVALUABLE at every subsequent check rather than a rule. `acl_pattern_arity.json` cannot express the difference — `entry_points` carries no per-door expectation — which is why §6.2.1 resolves it as a normative point of order: an implementation MUST NOT rely on the rule type's own construction-time check to cover this door. Two of the three SDKs already re-validated; this one did not.

  **Validation order is `effect` -> `approval` -> `callers` / `targets`, and rule index dominates all three (spec v1.31.0 §6.2.1).** A rule bad on more than one axis is refused for the first axis it fails, and a rule *set* with more than one bad rule is refused for the **lowest-indexed** bad rule — never by sweeping one axis across every rule before looking at the next. "Axis" is **every per-rule check a door performs**, which for `ACL.load` includes #107's rule-key closure, the missing-field check and the value-type checks; all of them already lived inside one per-rule loop in file order here, so there was no sweep to fix. **`default_effect` sits outside the scheme and is judged first, before any rule, at every door** — and *that* was a real defect: `ACL.load` left it entirely to the `ACL` constructor it reaches at the very bottom, after every rule has been parsed and validated, so a file carrying a bad `default_effect` AND a bad rule 0 was refused for the RULE through the loader and for `default_effect` through direct construction. One configuration, two answers, from two doors of one SDK. `_validate_default_effect` is now the one predicate both doors call, and `ACL.load` calls it before it reads `rules` at all — "first" being ahead of the file-level checks on the `rules` **collection** too, so a document both missing `rules` and carrying an unrecognised `default_effect` names the `default_effect`. No conformance case covers that combination, so `TestDefaultEffectIsJudgedBeforeAnyRule` is its only guard. The pattern fields are **one axis**, covering §6.1.4.1's *type* fault and §6.2.1's *shape* closure together, with the type fault first and `callers` before `targets`. §6.2.1 states this for the first time: §6.1.6 rule 2 *implies* `effect` before `approval`, since judging "`deny` plus `approval: required`" requires knowing the effect, but it states no order — which is why three implementations had three, one of them running `effect` -> patterns -> `approval`. Unchanged here, where `ACL.load` already validated rule by rule in file order and `ACL(rules=[...])` already validated each rule as it was constructed; pinned by tests now, on both halves.

  **A second, validator-only tier is added**, because closing the arities does not exhaust the inert class: `["$not", "*"]` has legal arity, exactly one operand, and matches nothing, producing the identical fail-open. `validate_rules()` now reports an array that is well-formed under every structural clause and still matches **no legal module ID for any input** — with the same finding shape — while such a rule keeps loading and changes **no** access decision. `ACL._never_matches` is a separate predicate from the precheck and feeds neither `handler_error` nor the decision. The MUST-detect minimum is `["$not", p]` where `p` is any pattern consisting only of wildcards (`*`, `**`), and `["@external"]` as a **`targets`** pattern — legal and unreported in `callers`, which is what the sentinel is for. It is a criterion with a minimum rather than an enumeration, because its predicate cannot be closed without freezing the pattern language, and it judges the array **as a whole**: `["api.*", "@external"]` in `targets` is not reported, because `api.*` still matches.

  **Driven by `conformance/fixtures/acl_pattern_arity.json`, 51 cases**, in `tests/conformance/test_acl_pattern_arity.py`. Its `closure` cases offer the rule at every door in `entry_points` — and this driver adds a third `add_rule` construction beside the pre-built and kwargs paths, a rule mutated *after* construction, which is the route the fixture cannot express. Its `backstop` cases mutate a rule already installed in the ACL and assert **both** decision surfaces separately: `expected_access` is the string on `check_access()` and `expected_legacy_check` the legacy boolean, and they diverge on the approval case, where §6.8.1 makes the boolean fail-closed. A case carries either `rule` (one) or `rules` (an ordered list); the list form pins the cross-rule half — the index chooses the rule, then the axis order chooses the fault inside it — and is offered at `load` and `construct` only, since `add_rule` takes one rule at a time. `expected_refused_axis` and `expected_refused_rule_index` carry the ordering that `expected_load` cannot see; the index is asserted positively at the loader, the only door that has one, and at every other door as the negative §6.1.5 implies — that no position was invented. The two `acl_evaluation.json` cases this change superseded — `empty_callers_matches_none` / `empty_targets_matches_none`, which asserted the reading §6.2.1 removed — were deleted from that fixture (21 -> 19 cases), and the transitional branch that carried them is gone with them.

  **Breaking for a configuration that was never doing what it said:** a file or a code path carrying `[]`, `["$or"]`, `["$not"]`, `["$not", p1, p2, …]`, an empty pattern string, or a reserved token outside index 0 **stops loading**. The affected population is exactly the deployments that believe they have a rule and do not. Migration is mechanical for the empty shapes — `["*"]` for "everything", deletion for "nothing", and the error message says both — and is **not** mechanical for the multi-operand `$not`: `["$not", p1]` preserves what the rule has actually been doing, but if `NOT (p1 OR p2)` was intended there is no general transform, because `["$not", p]` makes the rule *not match* and lets evaluation continue to later rules while a leading `deny` **ends** the scan. Rewrite that one by hand, against the rule's position.

- **The approval gate fires on the union of three sources, and an `ExecutionPolicy` can no longer clear what the ACL set (spec v1.28.0 §6.9, §7.4, apcore#108).** Step 5 read only the module annotation and the policy verdict, so an ACL rule carrying `approval` would have loaded, matched, and done nothing — the §6.1.1 / §6.1.5 failure arriving through a third door. The gate now fires when the module's `annotations.requires_approval` is true **or** the ACL decision for this call carried `approval_required` **or** `gate_destructive` applies (§6.9 rows 3-5), and Step 4 records its second result on the pipeline context for Step 5 to read.

  **Row 4 is the one that is not obvious, and it is a privilege-escalation guard.** A policy may **add** an approval requirement and **MUST NOT** remove one the ACL set. The ACL is a **caller-scoped** authorization layer while `ExecutionPolicy` is a **module-scoped** platform override, so letting the module-scoped one cancel the caller-scoped one means a policy rule written for `orders.*` silently strips a requirement an ACL author attached to one untrusted caller. A policy `requires_approval: false` still overrides the module's *annotation* — that half is unchanged — and both directions are pinned by tests.

  The `ApprovalRequest` handed to the handler carries the **effective** annotations for an ACL-sourced requirement as it already did for a policy-sourced one, so §7.3's "`requires_approval` is guaranteed true" holds whatever the source. `ExecutionPolicy(strict=True)` fails closed on an ACL-sourced requirement like any other. Step 4 deliberately does **not** deny a call that merely needs approval: it consumes `check_access()` rather than the fail-closed boolean, or "ask a human" would silently become a flat refusal and Step 5 would never see the requirement. An ACL object with only the legacy boolean contributes no requirement — inferring one would be inventing governance.

- **`Executor.validate()` reports the governance-effective `requires_approval`, not just the policy-effective one (spec v1.28.0 §7.9.5, apcore#108).** Preflight now unions the ACL's requirement into the flag it reports. Reporting only the policy-effective value would tell a caller no approval is needed for a call the gate will stop — including in the case §6.9 row 4 covers, where a policy clears the module annotation and the ACL's requirement stands. The dry run still emits no governance events and still puts nothing to a human.

- **BREAKING (security): an ACL condition that cannot be evaluated now makes a `deny` rule deny (spec v1.22.0 §6.1.1, apcore#100).** `_evaluate_conditions` returned a plain boolean, so "a handler answered no" and "no answer was obtainable" arrived at the rule loop identically, and both meant *this rule does not match*. That is safe in one direction only. An `allow` rule that cannot evaluate its condition does not grant — correct, and unchanged. A `deny` rule that cannot evaluate its condition did not block: evaluation continued to the next rule and then to `default_effect`, so **a single misspelled key (`role:` for `roles:`) turned a rule its author believed was blocking into decoration**. Reproduced end-to-end with `effect: deny` + a misspelled key + `default_effect: allow`, where `check()` returned `True`. The warning and `handler_error` were already emitted on that path, so the diagnostics were right the whole time and only the decision was wrong.

  The evaluator now returns `ConditionOutcome` — `SATISFIED` / `UNSATISFIED` / `UNEVALUABLE` — and `_matches_rule` / `_matches_rule_async` propagate it. Exactly three situations produce `UNEVALUABLE`, and they are the three `return False` sites that used to be indistinguishable from a plain "no": the condition key has **no registered handler**; the handler **raised**; the handler was **asynchronous and could not be resolved** on the sync `check()` path (previously specified as "treated as unsatisfied", which made a `deny` rule guarded by an async-only handler inert on that path — the same failure mode as a misspelled key). An unevaluable condition still never raises out of `check()`, which still returns a boolean.

  **Compound operators compose by three-valued logic**, which is normative because without it the same rule set resolves differently in different SDKs: an outright `UNSATISFIED` wins an AND, an outright `SATISFIED` wins an `$or`, and anything else with an unevaluable child is unevaluable. **`$not` of `UNEVALUABLE` is `UNEVALUABLE`, never `SATISFIED`** — negating "no answer" into "yes" would let a misspelled key inside a `$not` satisfy the very rule it was meant to gate, reintroducing the bypass one nesting level down. Short-circuiting is kept on the decisive child (`UNSATISFIED` in an AND, `SATISFIED` in an `$or`) and **not** on an unevaluable one, since a later sibling may still decide it; a child skipped by a legitimate short-circuit was never evaluated and therefore records no `handler_error`.

  `AuditEntry.handler_error` now reports **every** unevaluable condition in a check rather than the last one written, ordered **lexicographically by condition key** and joined with `"; "`. Lexicographic rather than evaluation order because the two are not the same across languages — `serde_json`'s map is ordered while Python dicts and JavaScript objects preserve insertion order — so "the first one encountered" would put a different key in the audit log for the same rule in each SDK. It stays null for an ordinary `UNSATISFIED` condition, which is what makes the two outcomes tellable apart after the fact.

  **Widened at spec v1.25.0 (§6.1.1).** "Unevaluable" is now a **principle** — the implementation cannot answer the condition *as written* — with five non-exhaustive examples rather than a closed list of three, and an implementation meeting an unlisted case MUST classify it by the principle rather than defaulting it to UNSATISFIED. Two cases change behaviour here: **a value malformed for its key** (`$or` whose value is not a list, `$not` whose value is not an object) and **a `conditions` that is not a mapping**. All three SDKs had independently classified the first as UNSATISFIED, on the grounds that a handler handed a malformed value does run to completion — which left a `deny` rule carrying `$or: "typo"` inert, the v1.22.0 defect reached through a second door.

  **Deliberately unchanged:** "conditions present but no context provided" stays a non-match (§6.5) **for a rule that passes the precheck**. Calling with no context is a legitimate shape for external entry points, not a misconfiguration, and treating it as an evaluation failure would flip the decision for every `@external` call meeting a conditional `deny` rule. It gains a warning instead.

  `conformance/fixtures/acl_handler_error.json` in the spec repo still pins the pre-v1.22.0 behaviour under the case `throwing_handler_does_not_flip_default_allow_to_deny_unsafely`; the corrected four-case fixture is staged there and lands once all three SDK drivers do. That one case carries a `strict` `xfail` naming the reason, and the driver already reads the corrected fixture's shape (verified against it: 4/4 pass), so the transition needs no further change on this side.

- **BREAKING: input validation now runs for modules that declare `input_schema` / `output_schema` as a raw dict.** `_DictSchemaAdapter.model_validate` was a pass-through. Its own comment justified this by `jsonschema` being "not currently declared" as a dependency — stale, since it is a hard dependency in `pyproject.toml`, and `apcore.schema.hardening.validate_schema_dict` has existed for some time. The effect was that **every constraint a dict schema declared was inert**: `required`, `enum`, `minimum`, `pattern` and even `type`, on user modules and on all nine `system.*` modules. `{"n": "not-an-int"}` against `{"n": {"type": "integer"}, "required": ["n"]}` was accepted, and so was `{}`.

  apcore-typescript (`validateSchema`) and apcore-rust (`validate_against_schema`) both enforced these, so the same module contract meant two different things depending on which SDK loaded it. Validation failures now raise `SchemaValidationError` with wire code `SCHEMA_VALIDATION_ERROR` — the same code the Pydantic path produces, so a caller cannot tell how a module declared its schema.

  **Modules that relied on the laxness will now fail.** That is the point: they were being validated everywhere except here. `system.control.update_config` had already grown a hand-written re-check of its own `required` list inside `execute()` to work around this.

- **BREAKING: `p99_latency_ms` changes value (spec v1.14.0 §6.7.1.3, apcore#96).** `_compute_p99` computed the nearest-rank index and then discarded it, returning `sorted[rank]` — one element past the rank it had just computed — whenever `rank < N`. For 100 samples `1..100` it answered 100 where apcore-typescript and apcore-rust answered 99. It now returns `sorted[min(ceil(0.99·N), N) − 1]`, with no interpolation and `0.0` for an empty sample set.

  A value disagreement inside a `number` field is invisible to any schema, which is why this survived: the test that covered it asserted `p99 >= 100.0` for a sample of 99×10ms plus one 100ms outlier. The nearest-rank 99th percentile of that distribution is 10.0 — 99% of the samples are at or below it — so the assertion was pinning the defect. Replaced with four tests asserting exact values, including the spec's normative worked example.

- **`period` on `system.usage.summary` / `system.usage.module` is constrained by the schema (spec v1.14.0 §6.7.1.1, apcore#96).** `input_schema` now declares `"pattern": "^[1-9][0-9]*[hd]$"`, so a malformed value is rejected at input validation with `SCHEMA_VALIDATION_ERROR` rather than reaching `_parse_period`. Previously `"0h"`, `"-5d"` and `"+3h"` were accepted and silently produced an empty or negative window — a report that reads as "no traffic" rather than as bad input — while apcore-typescript rejected all three.

### Fixed

- **BREAKING: a rule's `effect` accepted any string outside `ACL.load()`, and was silently read as `deny` (spec v1.30.0 §6.1.5, apcore#111).** §6.1's field table says `effect` **MUST** be `allow | deny` and `schemas/acl-config.schema.json` declares the enum, but the check lived inline in the YAML loader and nowhere else. `effect: "Allow"` — the capitalisation an operator writes by hand — therefore failed from a file and was **accepted** through `ACLRule(...)` and `add_rule()`, then read as `deny` at check time, because `_finalize_check` resolved any non-`allow` string to a denial. This is the closed-key-set fix ([apcore#107](https://github.com/aiperceivable/apcore/issues/107)) one level down: there an unknown **key** was dropped in silence, here a legal key's **value** is, in the same silence.

  **Not a privilege escalation** — the fallback resolved toward `deny`, so an unknown value could never grant. It was a silent functional break: under `default_effect: allow`, a rule the operator wrote to permit became a rule that **denied everything it matched**, flipping those decisions with no error, no warning and nothing from `validate_rules()`. On a `deny` rule the reading was only *accidentally* right — correct until someone revisited which way the fallback pointed.

  **The inconsistency was internal as well as cross-language.** This SDK already validated `default_effect` — the same two values one field up — in the `ACL` constructor, so the loader guarded one door, `default_effect` guarded all of them, and a rule's `effect` guarded only the file path. apcore-rust rejected the value at all three doors, so one ACL built in code ran here and could not be constructed there.

  `effect` is now rejected with `ACLRuleError` naming the rule index and the offending value at **every** entry point §6.1.6 rule 3 lists — file loading, direct construction and runtime insertion. One `_validate_effect` helper serves all three, carrying the loader's existing message unchanged (apcore-typescript and apcore-rust emit the same one), because two copies of a value set are two things that drift, which is how the loader came to be the only door that checked. `ACLRule.__post_init__` checks it **before** the `approval` pairing rule, so `effect: "DENY"` with `approval: required` reports the value that is actually wrong. `add_rule()` returning `None` is not an exemption — it raises, the way this SDK already signals an unconstructable rule for the `approval`-on-`deny` pair, on both the pre-built and the kwargs path. `default_effect` reads the same closed set rather than an inline literal, so the two fields cannot drift on which values are legal.

  **The evaluation-time fallback is gone.** `_finalize_check` now *reads* the matched rule's effect instead of resolving it, and raises `ACLRuleError` naming the rule index if it is somehow out of enum — §6.1.5 forbids resolving an unrecognised `effect` to a decision, and with every door closed the only way to arrive there is to assign `rule.effect` on an already-constructed dataclass. This is **not** the §6.1.1 "`check` MUST NOT raise" case, which is about a condition that could not be *evaluated*; this value could not be *read*. `conformance/fixtures/acl_effect_value_closure.json` pins the contract with 10 cases across the three doors; 5 of them fail against the previous behaviour.

  **Breaking for a configuration that was never doing what it said:** code that built an ACL rule with an out-of-enum `effect` used to construct and deny; it now raises at construction.

- **`requires_approval` was documented as if it answered "will this call need a human" (spec v1.29.0, [apcore#110](https://github.com/aiperceivable/apcore/issues/110)).** `ModuleAnnotations.requires_approval` read *"Whether human approval is needed before execution"*, and `PreflightResult.requires_approval` read *"True if the module has requires_approval annotation"* — the second is simply wrong, and has been since spec v1.28.0: `Executor.validate()` reports the **governance-effective** union (§7.9.5, §6.9 rows 3-5), so a module declaring `requires_approval=False` correctly reports `True` there when the ACL requires a human for the arguments this call carries. The code was right; the docstring described the behaviour it replaced.

  Both docstrings now say which question they answer: the annotation describes the **module** and `false` does not mean no approval will be required; the preflight result describes the **call** and is the same verdict the approval gate will enforce. Documentation only — no behaviour change.

- **Security: an unevaluable `allow` rule discarded the `approval: required` it carried, and a broader rule granted the call unapproved (spec v1.29.0 §6.1.1 rule 5 / §6.8.1 / §6.9 rows 1-2, apcore#109).** §6.1.1 was written when a rule carried one axis, and "an `allow` rule MUST NOT grant" was then a complete instruction: the rule steps aside, and stepping aside was harmless because whatever granted next also said `allow`. The `approval` field above gives a rule a **second** axis, and "does not grant" silently discarded it — so on exactly the shape argument-scoped approval exists for, a narrow approval rule ahead of a broad allow, the gate stepped aside, the broad rule granted, and `git push --force` ran with `approval_required: False` and `matched_rule_index` naming a rule that never mentioned approval. The requirement was not overridden, argued with, or logged away; it ceased to exist.

  **It is not confined to the projection-less `check()` route.** §6.1.1 is the path a misspelled predicate (`has_keys` for `has_key`), an unregistered condition key and a raising handler all take, so the defect reaches the ordinary Executor pipeline **with a projection present**, and `default_effect: allow` reaches it with no second rule at all. `validate_rules()` is not a mitigation that can be assumed: §6.1.2 makes an unregistered key a warning rather than a load failure, and it cannot see the projection-less route.

  The requirement is now **pending** rather than discarded. An unevaluable `allow` rule carrying `approval: required` records it and the scan continues — the rule itself still does not grant — and whatever grants next reports the **disjunction** of its own requirement and the pending one, including `default_effect: allow`, which makes `approval_required: True` with `matched_rule_index: None` a legal combination for the first time. A final `deny` clears it, because "denied *and* put it to a human" is the same meaningless state §6.1.6 rejects on a single rule, and `matched_rule_index` keeps naming the rule that actually decided. `handler_error` is untouched: a pending requirement neither suppresses nor substitutes for it, and the `AuditEntry` carries the final value. Requiring a human rather than refusing outright is deliberate — the condition that could not be evaluated is the one that decides whether *this* call is the dangerous one, so refusing would turn every ordinary `git push` into the hard failure argument-scoped approval exists to eliminate.

  **Scope is required, and that is what keeps the fix from over-reaching.** A rule whose `callers` / `targets` do not match this call raises nothing — its conditions are never consulted (§6.1.4 rule 4), and a rule written about one target must not attach a human to calls it was never written about. The one exception is a rule whose own pattern field is **malformed** (§6.1.4.1): it is unevaluable before any pattern is read, so its scope cannot be read and it cannot be shown not to apply here — the same posture that field already produces under `deny`, where an unreadable scope denies every call.

  Applied identically to `check()`, `check_access()`, `async_check()` and `async_check_access()`, since a governance result must not depend on which entry point was called, and §6.8.1's fail-closed boolean is now a property of the **decision** rather than of the matched rule, so it closes on a pending requirement too. The §6.1.1 warning additionally says the requirement is pending when the rule carried one, because the old wording read as "this rule had no further effect" — the exact misreading this fixes. `conformance/fixtures/acl_argument_scoped_approval.json` grows to 24 cases and a two-column contract (every case run with a projection and again with none); across all 20 pre-existing cases **with** a projection, no decision changes.

- **BREAKING (security): a `callers` / `targets` that is not a list of strings granted access to every caller (spec v1.25.0 §6.1.4.1, apcore#106).** A bare string is iterable, so `callers: "admin.*"` written where `callers: ["admin.*"]` was meant was read **character by character** — and `*` is a valid pattern matching everything, so an `allow` rule carrying that typo granted access to *every* caller under `default_effect: deny`. Whether a given typo was dangerous depended only on whether the mistyped string happened to contain a `*`: `"api.gateway"` returned `false` by luck, not by design. A non-subscriptable scalar (`callers: 5`) raised `TypeError` out of `check()` and a mapping raised `KeyError`, both violating the contract that `check` MUST NOT raise.

  Both fields are now prechecked before any pattern is read, and a value that is not a list of strings makes the rule **unevaluable** per §6.1.1's effect table: an `allow` rule does not grant, a `deny` rule takes effect, and `check()` does not raise. Both fields are examined without short-circuiting, so the finding set is a pure function of the rule. An empty list stays well-formed and simply never matches (§6.5), unchanged. `ACL.load` already rejects a non-list value, so a YAML file cannot reach this; direct construction and `add_rule()` can.

- **`ACL.check()` raised out of the call for a rule whose `conditions` value is not a mapping.** `_evaluate_conditions` went straight to `conditions.items()`, so a scalar or a list escaped as `AttributeError: 'str' object has no attribute 'items'` — violating the `ACL.check` contract, which states that `check` MUST NOT raise to indicate a deny and reserves raising for unrecoverable internal failures. A malformed rule supplied by the host is not one of those. The shape is reachable because `ACL(rules=[...])` and `add_rule()` build rules programmatically and never reach `ACL.load`'s parser, and because `ACL.load` does not type-check `conditions` either.

  A non-mapping `conditions` is now classified **UNEVALUABLE** (§6.1.1) on both the sync and async paths: a `deny` rule takes effect and the call is denied, an `allow` rule does not grant, and `handler_error` is set under the synthetic key `$conditions` — `$` is reserved by §6.1 for compound operators, so it cannot collide with a key a deployment registered a handler for. UNSATISFIED would not do: it would let a `deny` rule fall through to the next rule and then to `default_effect`, which is precisely the bypass §6.1.1 exists to close. Parity with apcore-typescript, which records the same synthetic key; apcore-rust returns `true` here (an inert `deny` rule) and is corrected in a later round.

  A malformed operand of a compound operator — `$or: "not-a-list"`, `$not: 3` — is unevaluable for the same reason, settled by §6.1.1 case 4 at spec v1.25.0. `ACL.load` now also rejects a non-mapping `conditions` at parse time with `ACLRuleError`, as apcore-typescript's parser does, so a YAML file cannot reach §6.1.1 case 5 at all; direct construction and `add_rule()` still can, which is what the runtime precheck is for.

- **`CallbackApprovalHandler` handed a synchronous callback now fails with an actionable message (apcore#104).** The constructor declares an async callback and nothing checked, so a plain function reached `await self._callback(request)` and died with `TypeError: object ApprovalResult can't be used in 'await' expression` — which names neither the callback nor the fix, and surfaces at the approval gate rather than where the handler was built. The returned value is now tested for awaitability and the `TypeError` names the callback, what it returned, and both remedies (declare it `async def`, or wrap a synchronous one with `asyncio.to_thread`; `AutoApproveHandler` / `AlwaysDenyHandler` for a fixed verdict). Detection is on the returned value rather than `iscoroutinefunction`, so a callable object with an `async def __call__` and a `functools.partial` both keep working. The callback contract is unchanged — async, and able to express failure — which is what apcore-typescript already declares and what apcore-rust's synchronous, infallible closure does not.

---

## [0.27.0] - 2026-08-14

> **Release note:** this section contains BREAKING changes. It must ship as a
> **minor** (or major) version bump, never a patch.

### Changed

- **BREAKING (security): a failed `acl` check now withholds module-level introspection from `validate()` (spec v1.13.0 §12.8.5.1, apcore#96).** `validate()` looked the module up at Step 3 and ran `preflight()` and `preview()` at Check 7 on the strength of that lookup alone, so a caller the ACL had just denied still made module-authored code run and still received what it returned. For a command-wrapping module that is the resolved binary and its argv; for a writer it is the target of the side effect. All three SDKs did it, and `apcore-mcp-rust` had already grown a string-matched disclosure filter over the top of it, which is the evidence the gap was reachable in a shipped product rather than theoretical.

  `validate()` no longer invokes either hook, emits a `module_preflight` / `module_preview` check, or populates `predicted_changes` when the `acl` check failed. The failed `acl` check itself is still reported, so a denied caller still learns *why*, and no other check is suppressed: the rule is about **authorization**, not validity. A failed `schema` check does **not** suppress introspection — a caller the ACL permits is entitled to the module's account of what would happen even when its inputs are malformed, which is what it needs in order to fix the call. Pinned by `conformance/fixtures/preflight_disclosure.json` (4 cases), whose control case exists so that an implementation which never introspects at all cannot pass the denial cases for the wrong reason.

### Added


- **`_config.strict` now also rejects undeclared keys *inside* the framework sections (PROTOCOL_SPEC §9.6.3 clause (b), §9.14 `reject_unknown_framework_keys`).** Every section in `schemas/apcore-config.schema.json` is `additionalProperties: false`, and that closedness was enforced by nothing at load time: `executor:` with a `zz_undeclared:` typo under it loaded clean in every SDK, so the operator read a default they believed they had overridden. Enforcement now has two tiers. **Default (`_config.strict` absent or false) is unchanged and unaffected** — the key is retained and readable through `get()`, which apcore-python already did by storing the parsed document as an untyped dict, now pinned by a test that asserts the retained value by *reading it back* rather than by observing that the load did not raise (an implementation that discarded the key at parse time also does not raise). **`_config.strict: true` raises `CONFIG_INVALID`**, and the error enumerates **every** offending key rather than failing on the first, so one restart shows the whole problem instead of one restart per typo. The rule applies in **legacy mode too**, where the whole document *is* the `apcore` namespace (§9.14 step 1), not only to the `apcore:` subtree of a namespace-mode file. `allow_unknown` deliberately plays no part: §9.6.3 defines it for unknown top-level *namespaces*, and stretching one field across two granularities would make its meaning depend on where it is read. Because the schema files ship with the spec repo and not with this package, the section→keys projection lives in `config._FRAMEWORK_SECTION_KEYS`, which `tests/conformance/test_config_key_governance.py` rebuilds from `apcore-config.schema.json` — resolving `$ref` and the `oneOf` branches `ExtensionsConfig` splits `root`/`roots` across — and fails on any difference, so a section added upstream breaks a test instead of going silently unenforced; `sys_modules` unions in `sys-modules.schema.json`, which is where §9.15.3 declares its subsections. Pinned by the new `unknown_framework_key_is_retained_by_default` and `unknown_framework_key_is_rejected_under_strict` cases of `conformance/fixtures/config_key_governance.json`, both driven in legacy **and** namespace mode. **This is opt-in: a deployment that does not set `_config.strict: true` sees no behaviour change.**

- **`SchemaValidator(coerce_types=True)` now coerces the strings `"true"` and `"false"` to a boolean, and nothing else gains coercion (apcore#95, TYPE_MAPPING §11 "What the knob coerces, when it exists", normative as of spec v1.12.0).** Offering the knob at all stays a **MAY**; what it coerces *when offered* is now a **MUST**, and the table is `string → integer` (entire content parses as an integer — `"42"`, `"-7"`; `"3.14"` MUST NOT), `string → number` (`"1.5"`, `"42"`, `"-0.5"`) and `string → boolean` (**exactly `"true"` and `"false"`, case-sensitive**). apcore-python already matched the two numeric rows through Pydantic's own lax mode and coerced no string to a boolean in *either* mode, so this closes the one gap; apcore-rust and apcore-typescript move the other way, narrowing from an undocumented twelve-spelling case-insensitive dialect (`"yes"`, `"on"`, `"y"`, `"t"`, `"1"`, `"0"` and negatives) that no document described. `"0"` → `false` was the sharpest of those: R5 makes the *number* `0` a MUST-reject for `boolean`, so accepting the string put two paths of one SDK on opposite sides of a single value.

  **Where the coercion lives, and why not in the model.** The guard that rejects `"true"` for a `boolean` is `_ONLY_BOOL`, a `BeforeValidator` installed by `schema/loader.py:_scalar_annotation` and baked into the Pydantic model by `SchemaLoader.generate_model()` — while the knob lives on `SchemaValidator`, so **the model is built without knowing the knob's value** and the guard cannot consult it. Threading a `coerce` flag through `generate_model` would work at the cost of a second model-cache key and two compiled models per schema. Instead the lax boolean handling is a **validator pre-pass**, `schema/validator.coerce_value()`, which is the direct twin of `apcore-typescript::coerceValue` and `apcore-rust::coerce_value`: all three SDKs now do this at the same layer rather than each somewhere different, `generate_model` stays cacheable, and `_ONLY_BOOL` keeps its job. The pre-pass walks `properties` and `items`, rebuilds containers rather than mutating the caller's input, and coerces **from a string only, toward a declared type only** — a number is never coerced to a boolean, a boolean never to a number, nothing is coerced toward `string`. The numeric rows are deliberately left to Pydantic's lax mode rather than reimplemented, so one behaviour keeps one source of truth.

  **The module-invocation boundary is unchanged, and that is the point.** `coerce_types` still defaults to `False`, `BuiltinInputValidation` / `BuiltinOutputValidation` still call `model_validate(strict=True)` directly and construct no `SchemaValidator` at all (TYPE_MAPPING §17.3), and on the strict path `"true"`, `1` and `0` are still rejected for a `boolean` — R5 makes that a MUST. Pinned by six new cases in `conformance/fixtures/schema_validation.json`, **four of which assert a spelling that MUST NOT coerce** (`"yes"`, `"0"`, `"True"`, and `"3.14"` for an `integer`): a fixture carrying only `"true"` is satisfied by an implementation that coerces every non-empty string, which is close to what two SDKs actually shipped. Every case states `expected_valid_strict` **and** `expected_valid_coerce` and the driver now asserts both halves plus `expected_coerced_value` — validity alone cannot tell `"false"` → `False` from `"false"` → `True`.

- **`validate_context_key` / `enforce_context_key` (Issue #42, middleware-system.md §1.1).** apcore-python shipped no context-data namespace validator, so the normative rules — user middleware MUST NOT write `_apcore.*`, framework middleware MUST NOT write `ext.*`, unprefixed keys tolerated — were enforced by nothing, while apcore-typescript (`validateContextKey`) and apcore-rust (`validate_context_key`) both had one and both drove the three `middleware_hardening.json` `context_namespace_*` cases. `apcore.middleware.context_namespace` adds the peer implementation, returning the same `NamespaceCheck(valid, warning)` pair for the same `(writer, key)`; `validate_context_key`, `NamespaceCheck`, `APCORE_KEY_PREFIX` and `EXT_KEY_PREFIX` are exported from the top-level package.

### Changed


- **BEHAVIOUR CHANGE: `pipeline.configure` now accepts exactly four fields; every other key is a start-up error (apcore#89).** The gate was `hasattr(target, key)` (`pipeline_config.py`) — any attribute the concrete step object happened to declare. `schemas/apcore-config.schema.json` `$defs/ConfigurableStepFields` and `DECLARATIVE_CONFIG_SPEC.md` §4.2 declare the set as exactly **`match_modules`, `ignore_errors`, `pure`, `timeout_ms`** — the §4.3 step-entry fields that still mean something applied to a step that already exists — and anything else MUST raise `PIPELINE_CONFIGURATION_ERROR` at parse time.

  **The key this was really about is `requires` / `provides`.** Both are attributes of every `BaseStep`, so `hasattr` accepted them and `setattr` applied them. Fed the example that shipped in `features/middleware-system.md` — `configure: {input_validation: {requires: ["context"], provides: ["validated_inputs"]}}` — apcore-python moved `input_validation` from `requires=('module',)` to `requires=('context',)`, **deleting the dependency `module_lookup` satisfies**. Construction then validates cleanly, so the `PIPELINE_DEPENDENCY_ERROR` that `features/middleware-system.md` § Configuration safety states as a MUST can never fire for that step: the documented way to exercise the dependency contract was the way to switch it off. A step's capability contract is declared by its implementation and is not an operator knob.

  **What an operator does instead.** `requires` / `provides` move to the step class (`BaseStep.__init__(..., requires=..., provides=...)`) — configuration cannot set them at all. `name`, `type`, `handler`, `after`, `before` are structural and `config` is constructor arguments: all six belong on the `pipeline.steps` entry that inserts the step, not on `configure`, which only overrides fields of a step that already exists. Every other key — `description`, `removable`, `replaceable`, and any attribute of a custom step class — is simply not configurable. The error names the offending key, lists the four valid ones, and for the six relocated fields says where they live now, so the fix does not require opening the spec.

  **This is a start-up error for configuration that previously loaded.** An `apcore.yaml` carrying any of these keys under `pipeline.configure:` raised nothing before and raises `ConfigurationError` / `PIPELINE_CONFIGURATION_ERROR` now, at parse time, before any module runs. That is the intent: on apcore-typescript the same YAML already raised and on apcore-rust it was dropped with a warning, so a config that worked here was not portable — the one property `configure` exists to provide. Pinned by the three new cases of `conformance/fixtures/pipeline_failfast_config.json` (`unknown_configure_field_raises_configuration_error`, `configure_must_not_rewrite_a_step_capability_contract`, `all_four_configurable_fields_are_accepted`). The accept case reads all four values **back off the built step**, per `driver_contract.read_the_field_back_off_the_step`, and each differs from the built-in default — `raises: false` alone is also satisfied by an implementation that takes the keys and applies none of them.

- **BEHAVIOUR CHANGE: `pipeline.steps` entries now reject keys `$defs/PipelineStep` does not declare (apcore#89).** `_resolve_step` read the ten fields it knew — `name`, `type`, `handler`, `config`, `match_modules`, `ignore_errors`, `pure`, `timeout_ms`, `after`, `before` — and ignored everything else, while `$defs/PipelineStep` had been `additionalProperties: false` since it was written and nothing enforced it. Measured against the shipped loader, `{"name": "x", "type": "noop", "after": "execute", "tiemout_ms": 5000}` built successfully with `timeout_ms == 0`: the operator's five-second timeout silently absent, no warning, no error. This is the same silent-drop failure mode the `configure:` change above removes, one key over, and it is a start-up error now for the same reason — a typo you are told about costs one restart, a typo you are not told about costs an incident. Pinned by `unknown_key_on_a_steps_entry_raises_configuration_error`, whose `driver_contract` also names the trap: reaching the insertion path at all requires a step *type* registered under the case's `type` value, so a driver that switches the case to a built-in step name tests the lookup path instead and passes without exercising anything.

- **BREAKING (narrow): `TraceContext.inject()` now raises `InvalidParentIdError` for a malformed `parent_id` override (Issue #32, decision D-51).** It raised a bare `ValueError` carrying no `code`, so a polyglot caller matching on `INVALID_PARENT_ID` got the code from apcore-typescript (`src/trace-context.ts` sets `code = 'INVALID_PARENT_ID'`) and apcore-rust (`ErrorCode::InvalidParentId`) and nothing from apcore-python. The new `apcore.InvalidParentIdError` subclasses **both** `ModuleError` and `ValueError`, so `.code == "INVALID_PARENT_ID"` is available while every existing `except ValueError` caller keeps working; `str(exc)` now renders `"[INVALID_PARENT_ID] parent_id must be 16 lowercase hex chars, got 'ZZZZ'"`, which is also what `docs/features/observability.md` §"Optional `parent_id` Override on `inject()`" documents its Python example asserting (that example did not hold before this change). `ErrorCodes.INVALID_PARENT_ID` is registered, so the code is reserved against module collision. `TraceContext.from_traceparent()` deliberately keeps raising a bare `ValueError`: D-51 pins the code for the `inject()` override only, and apcore-typescript's `fromTraceparent` also throws a codeless error.

- **`StepMiddleware` ordering and error wrapping corrected (Issue #33 §2.2).** `after_step` and `on_step_error` now run in **reverse** registration order (onion model) — `after_step` already did, `on_step_error` ran forward and returned the LAST non-`None` value instead of short-circuiting on the first. A `before_step` that raises is now wrapped in `MiddlewareChainError` and stops the chain: the step body does not run, and `on_step_error` fires only on the middlewares whose `before_step` had already executed. It was previously swallowed, so a broken middleware let the step run anyway. Pinned by `conformance/fixtures/pipeline_step_middleware.json`, which apcore-python now drives from disk (`tests/conformance/test_pipeline_step_middleware.py`).

- **BREAKING: a `before_step` failure is terminal and is no longer recoverable (middleware-system.md § "A `before_step` failure terminates the step — it is not recoverable").** `PipelineEngine` routed the `MiddlewareChainError` raised by a failing `before_step` into the *same* handler as a step-body failure, so an `on_step_error` handler that returned a value made that value the step's output and let the pipeline advance **past a step whose body never ran**. `acl_check` and `approval_gate` are steps in the built-in strategy, so any registered `StepMiddleware` could make its own `before_step` raise, recover from it, and skip the ACL check or the approval gate outright — a silent authorization bypass reachable from an extension point that carries no authority. The step's `ignore_errors` swallowed the chain error for the same reason. The `before_step` invocation now lives outside the step-body `try` and terminates the run: `on_step_error` still fires on the already-entered middlewares in reverse order, but **for observation and cleanup only**, through a separate `_invoke_step_cleanup_hooks` pass (mirroring apcore-typescript `_runStepCleanupHooks`) rather than the recovery-seeking `_invoke_on_step_error`: every return value is discarded, and first-recovery-wins does not apply, because no recovery is being sought and short-circuiting would strand the cleanup of every middleware registered behind the first one to return a value. `after_step` does not fire (no body ran, nothing to close over) and `ignore_errors` does not apply — `MiddlewareChainError` propagates regardless. **Anyone relying on `on_step_error` to recover from a `before_step` failure will now see the error propagate.** Recovery from a *step-body* failure is unchanged, and `after_step` correctly fires after a recovered body so the onion still closes. Pinned by the new `before_step_failure_recovery_is_discarded` and `after_step_fires_after_a_recovered_step` cases of `conformance/fixtures/pipeline_step_middleware.json`; the driver asserts the bypass by observing that the **following** step did not execute, not merely that an error was raised.

- **BREAKING: `state.outputs` no longer contains the current step in `after_step` (middleware-system.md § "What `state.outputs` contains").** `PipelineEngine` recorded `step_outputs[step.name]` *before* invoking the `after_step` hook, so `after_step` was the one hook whose `state.outputs` included the step being observed — `before_step` (not yet run) and `on_step_error` (ran, produced nothing) both excluded it. The map now holds exactly the steps that completed **before** the current one in all three hooks, and the snapshot is taken after the hook returns, which is the ordering apcore-typescript (`src/pipeline.ts`) and apcore-rust (`src/pipeline.rs`) already had — apcore-python was the sole outlier. The rule is one rule with one meaning by design: under the more obvious reading, "outputs of completed steps", a middleware would have to know which hook it was in before it could read the map. **Any middleware reading `state.outputs[current_step]` inside `after_step` will now get a `KeyError`** — the value it wants is the `result` parameter, and always was; carrying the same value down two paths is how the two drift apart. The step-body **recovery** path is corrected with it, and in apcore-python it is corrected by the *same line*: the recovery branch assigns `result = recovery` and falls through to the shared success tail, so there is one snapshot site where apcore-typescript and apcore-rust each have two. `after_step` on a recovered step therefore also sees only the earlier steps, and the recovery value is snapshotted afterwards. `run_until` predicates are unaffected — they are evaluated after the step completes and deliberately do see it, so the snapshot sits between the `after_step` hook and the predicate. Pinned at **both** snapshot sites: `state_outputs_excludes_the_current_step_in_every_hook` covers the natural success path and `after_step_fires_after_a_recovered_step` covers the recovery path, which the first case cannot reach because it recovers from nothing. Both drivers assert the **exact** key set rather than the absence of one key — `"second" not in outputs` also passes against an implementation that lost `first`, or that never populated the map at all — and both snapshot that key set *inside* the hook, because `state.outputs` aliases the engine's live map (as it does in apcore-typescript): a driver that stashed the reference and read it after `run()` returned would see the final map on every entry and could never fail.

- **`before_step` documented as an observation hook.** The protocol docstring described a `(step_name, ctx, inputs)` signature with a return value that replaced the step's inputs. A `Step` is `execute(ctx)` and has no `inputs` parameter, so that convention existed nowhere; the spec was corrected to match the three implementations rather than the reverse.

- **`PipelineContext.trace` is now assigned.** The field existed and was never written, so `Executor.call_async_with_trace` had nothing to return on the middleware-recovery path and fabricated an empty `PipelineTrace`. Parity with apcore-typescript `pipelineCtx.trace` and apcore-rust `pipeline_ctx.trace`.
- **BREAKING: the module-invocation boundary no longer coerces types.** `BuiltinInputValidation` / `BuiltinOutputValidation` now call `model_validate(strict=True)`, so `{"a": "42"}` against `{"a": {"type": "integer"}}` is rejected instead of silently becoming `42` (type-mapping §17.3). apcore-typescript and apcore-rust already rejected it. `SchemaValidator(coerce_types=…)` is unchanged as a library-level knob and does not affect this path — though its **default** flipped to `False`, matching the other two SDKs.

- **BREAKING: recursive schemas can be registered.** `RefResolver` no longer raises `SchemaCircularRefError` for a self-reference — `{"$ref": "#"}`, the root `$id`, or a `#/$defs/…` entry re-entered through `properties` / `items`. The reference is preserved and `generate_model()` binds it at validation time. A `$ref` → `$ref` cycle still raises. Previously **any** self-referencing schema failed to load, despite the specification having a Recursive Schema Support section and a conformance fixture for it — both fixtures drove a path that skipped `RefResolver` entirely.

- **BREAKING: applicator keywords are enforced.** `prefixItems`, `patternProperties`, `propertyNames`, `dependentRequired`, `dependentSchemas`, `unevaluatedItems` and `unevaluatedProperties` were dropped by `generate_model()`; they are now delegated to the jsonschema library as a sub-schema assertion, the same mechanism combinator siblings already used. Applicators on the root schema are enforced too. `if`/`then`/`else` previously raised `SchemaParseError("not yet supported")` — rejecting a valid contract outright — and is now enforced per §10.2.2.

- **BREAKING: `to_strict_schema()` hardens the objects it was skipping.** An object carrying `properties` with no `type` keyword, or `type: ["object", "null"]`, was returned unhardened, so the resulting strict schema was rejected by OpenAI structured outputs. It also now recurses into `prefixItems`.

### Removed


- **`apcore.middleware.namespace_keys` (a second context-key registry that had drifted).** apcore-python had two registries for the same `_apcore.*` namespace: `apcore.context_keys`, the canonical one — typed `ContextKey[T]` slots, exported from the package root, imported by the logging/retry/metrics/usage middleware that actually write those keys — and `namespace_keys`, an untyped string mirror of a subset of it with **no readers anywhere in the package**. They disagreed: the mirror declared `_apcore.mw.tracing.span_id` (docstring: *"TracingMiddleware.before() writes the active span ID for the call"*), a key **nothing writes**, while the canonical registry declares `_apcore.mw.tracing.spans`, the span **stack** `observability/tracing.py` maintains and `executor.py` / `builtin_steps.py` / `trace_context.py` read. The single-slot key came from the second `TracingMiddleware` formulation of middleware-system.md §1.3, which apcore-python never implemented and which has since been **withdrawn** — a single slot is overwritten on the first nested call, which is why the surviving contract stores a stack and links `parent_span_id` explicitly. The mirror was deleted rather than re-exported from the canonical registry: a re-export would leave two spellings of every key (`namespace_keys.LOGGING_START_TIME` alongside `LOGGING_START`) and the same ambiguity about which to import, and hand-maintained duplication is the mechanism that produced the divergence in the first place. `apcore.middleware.context_namespace` keeps its actual job — `validate_context_key` / `enforce_context_key` / `APCORE_KEY_PREFIX` / `EXT_KEY_PREFIX`, the prefix rules of middleware-system.md §1.1 — and no longer carries a copy of the key list. **Migration:** `namespace_keys.LOGGING_START_TIME` → `apcore.context_keys.LOGGING_START.name`; `namespace_keys.CIRCUIT_STATE` → `apcore.middleware.circuit_breaker.CTX_CIRCUIT_STATE` (unchanged, and next to its writer); `namespace_keys.TRACING_SPAN_ID` has no replacement because it never had a writer — the span stack is `apcore.context_keys.TRACING_SPANS`.

### Fixed


- **25 conformance fixture cases across five per-fixture drivers reached no assertion (apcore#93). Test-only: no SDK behaviour changed, and every newly-real assertion passed against the shipped implementation on its first run.** apcore#92 answered the weaker question — *does ANY driver run this case* — and drove it to zero. This is the per-SDK question, `check_case_pinning.py --sdk python`, and it found a second population that #92's default mode reported as covered because apcore-typescript or apcore-rust pinned it: `event_management_hardening` (**all 10**), `observability_hardening` (9 of 10), `pipeline_hardening` (4 of 5), `async_task_cancellation` (1), `multi_module_discovery` (1). A fixture case one SDK checks and two do not proves one implementation, not three.

  **One shape dominates, and it is not among #92's four: wholesale transcription.** These drivers do not dispatch on the fixture at all — they load it only to run a coverage guard (`TestFixtureCoverage`, which asserts a case *id* has a class, never that the class reads the case) and then re-type every input and every expected value as a Python literal. `event_management_hardening`'s driver named its circuit-breaker thresholds, its subscriber configs, its events and its expected states in the test body; `observability_hardening`'s did the same for nine of its ten cases. The fixture and the driver then agree only by the diligence of whoever last edited both, which is exactly the drift conformance fixtures exist to prevent, and the file reads as fully covered throughout. All 25 now take their inputs **and** their expectations from the case body.

  **Three cases needed an observable post-condition rather than a transcription fix**, because their declared expectation is not a value any single assertion can compare against:

  - `pipeline_hardening :: step_lookup_is_not_linear` declares `lookup_complexity: "O(1)"`. The old assertions were that a `_name_to_idx` attribute exists and is a `dict` — true of an implementation that keeps the map and scans the list anyway. The cost is now **measured**: `_CountingIndex` wraps the strategy's index, a pipeline skips from its first step to its last, and the driver asserts exactly one index probe at the fixture's declared `step_count` and the same count at twenty times that size. Constant means the two agree.
  - `event_management_hardening :: event_naming_canonical` declares a regex and `all_match_pattern: true`. Checking the fixture's own `events_to_check` against the fixture's own `pattern` is symmetric — mutate the pattern to something unmatchable and the boolean to false in the same edit and it still passes — so the declared pattern is additionally held against event names taken from a **live circuit breaker**, driven open and then closed. The two tautological tests that matched the driver's regex against the driver's own string literals are folded into that check rather than deleted.
  - `multi_module_discovery :: disabled_by_default` allows either SDK policy for a second qualifying class ("silently ignored **or** causes an error per SDK policy"), and apcore-python takes the error branch — which consulted the fixture nowhere, so `expected.module_ids` reached an assertion only on the branch this SDK never takes. The case's one unconditional statement, "the base_id is never suffixed", is a claim about ID *derivation*, which happens on both branches, so `_compute_base_id` is now asserted against the declared bare id. "`resolve_entry_point` raised" is satisfied by an implementation that derives nothing at all.

  **Two catch-alls of #92's shape 3 were also corrected in place, with the reason recorded at the assertion.** `circuit_half_open_after_window` asserted `cb.state != CircuitState.OPEN` after delivery — satisfied by CLOSED, by HALF_OPEN and by any state added later, and reading the declared `circuit_state` not at all; the wrapped subscriber now records the state it was *called in*, which is the transition the case actually describes, and a new sibling test asserts the circuit does **not** leave OPEN before `recovery_window_ms` has elapsed, without which a wrapper that transitions unconditionally passes. `redaction_field_pattern_match` / `redaction_value_pattern_match` checked that `module_id` and `caller_id` were *keys* of the log record ("value may be None") and ignored `trace_id_present` entirely; all three correlation fields are now asserted **by value** against the log entry that declared them, through `ContextLogger.from_context`.

  **The helpers are the ones apcore#92 introduced, moved rather than copied.** `expectation_keys` / `reject_unknown_expectations` and a generalised `dispatch_or_fail` (the shape of `_exc_class_for`: an unrecognised declared value is a `pytest.fail` naming the case and the value, never a skipped branch) now live in `tests/conformance/canonical_fixtures.py`, the module every conformance driver in this repo already imports; `tests/test_conformance.py` keeps its private spellings as thin aliases. Five driver files share one mechanism instead of growing five.

  **Verified the way the tool asks the question.** `check_case_pinning.py --sdk python --fixture <name>` reports **0 unpinned** for all five fixtures. The tool spawns pytest with the ambient environment, so `$CONFORMANCE_SPEC_REPO` must be **exported** for the run: without it the drivers resolve fixtures through the sibling-directory fallback and validate the *unmutated* canonical copy, every case reports unpinned, and the result looks like a total regression rather than a broken invocation. That is a fifth way to get a meaningless number out of this tool, alongside the four already recorded in its docstring.

- **36 conformance fixture cases were driven by `tests/test_conformance.py` without ever reaching an assertion (apcore#92). Test-only: no SDK behaviour changed, and every newly-real assertion passed against the shipped implementation on its first run.** The spec repo's `conformance/check_case_pinning.py` mutates a case's declared expectation so no correct implementation can satisfy it, runs the drivers, and reports the case if nothing goes red — a case that cannot go red is not coverage. Measured per SDK (`--sdk python`), 36 cases across six fixtures survived that mutation with this driver: `error_codes` (**all 18**), `dependency_version_constraints` (7 — every `*_violated` case, the entire negative half), `call_chain` (6), `version_negotiation` (3), `context_create` (1), `identity_system` (1).

  **Four shapes, all of which make the driver's own literal the contract and the fixture decoration.** (1) *Branching on key presence*: `if "expected_error" in case:` tests that the key exists, then asserted the Python class `ErrorCodeCollisionError` — the fixture's declared wire code `ERROR_CODE_COLLISION` was never compared to anything, so it could have said anything at all. (2) *Dispatching on the expected value with no `else`*: `if error_code == "DEPENDENCY_VERSION_MISMATCH":` and nothing after it, so an expectation the driver did not recognise skipped the whole assertion block and passed. (3) *An `else` that accepts anything*: `else: pytest.raises(Exception)` in `test_version_negotiation`, satisfied by any error whatsoever. (4) *Positive cases whose entire assertion was "did not raise"*: the 15 `expected: "ok"` cases of `call_chain` and `error_codes`, which an implementation that does nothing also satisfies. The consequence for `dependency_version_constraints` is the sharpest: the satisfied branch was driven and the violated branch was not, so **an implementation that always reported "version constraint satisfied" passed every case that fixture actually ran**.

  **The fix, in the shape the fixtures' own `driver_contract` blocks now require.** Every declared error is asserted as a **wire code**, mapped to this SDK's exception type through a lookup (`_CALL_CHAIN_ERROR_MAP` was already the model — it is why `call_chain`'s negative cases were the ones that survived mutation) and then re-checked against `ModuleError.code`, so a class that stops carrying the code it is mapped to fails. An expectation the driver does not recognise is a `pytest.fail` naming the case and the value — never a skipped branch, never a `skip`. Positive cases assert an **observable post-condition**: after `register`, the code is queryable through `ErrorCodeRegistry.all_codes`, and after `unregister` it is gone (so the reuse case proves the release rather than assuming it); after a `guard_call_chain` that accepted a chain, the caller's list is unmutated and the *same* chain one step over the tightened limit is rejected, which a no-op guard fails. A new `_reject_unknown_expectations` helper fails any case carrying an `expected*` key the driver does not read.

  **Two fixture cases were rewritten upstream and are now asserted field by field.** `identity_system :: identity_propagates_to_child_context` replaced the prose string `"child.identity === parent.identity"` — a sentence in a value slot that no driver could assert — with four declared fields, all four now compared. `context_create :: executor_rejects_cross_executor_rebind` replaced `expected_one_of: [raise, silent_accept]` (an alternation no driver can assert without hardcoding its branch, which all three did) with `{raises: true, error_code: "CONTEXT_BINDING_ERROR"}`; both fields are now observed, the code off the raised error rather than off a class name apcore-rust does not have.

  **The same audit over the rest of the file** corrected four more instances of the shape, none of which the issue listed: `test_binding_errors` ended in `else: pytest.skip(f"Unknown error_code …")`, so a code the driver did not recognise was reported as a deliberate skip while nothing was checked — a skip reads as intentional, which is worse than a failure; `test_approval_gate`'s `else: pytest.raises(Exception)`; `test_middleware_on_error_recovery`'s outcome dispatch with no `else`; and `test_schema_validation`, which selected on `expected_valid_strict` and then asserted `expected_valid_coerce`, so the strict half of that fixture was named in the driver and asserted nowhere — it is now checked against a second validator built with `coerce_types=False`.

  **Verified the way the tool asks the question**, not by inspection: `check_case_pinning.py --fixture <name> --sdk python` reports **0 unpinned** for all six fixtures (`error_codes` 18 cases mutated, `context_create` 15, `dependency_version_constraints` 15, `call_chain` 11, `version_negotiation` 10, `identity_system` 8) and for `approval_gate`, `middleware_on_error_recovery` and `schema_validation`. Because a clean result from that tool is also what a broken invocation produces, every run was checked for a non-zero mutated count and for the absence of an INDETERMINATE verdict, and the pre-fix driver was re-measured as a control on each fixture — it names exactly the 36 cases above, so the instrument was measuring this driver rather than manufacturing green. The counts are measured against a `check_case_pinning.py` that no longer treats a top-level `error_code` as an expectation: it is the code being *registered* in `error_codes.json`, an input, and mutating it made six negative cases go red for the wrong reason. That correction is why `error_codes` is 18 here and 12 in the issue.

  **One case remains unpinned for Python and is not a defect:** `binding_errors :: pipeline_handler_not_supported_rust` declares a Rust-only error code that apcore-python does not implement and its driver skips explicitly. That is the asymmetric-skip case `conformance/case_pinning_allowlist.json` exists for and it needs an entry there (spec repo), not a driver change.

  **Not asserted, deliberately, and recorded rather than faked:** `approval_gate`'s `expected.http_status` (403 / 202) has no surface in apcore-python — no error carries an HTTP status — and `dependency_version_constraints`' driver additionally now compares the `required` and `actual` strings off the raised error, which is what separates "the checker rejected the right pair" from "the checker rejected something".

- **`system.control.reload_module` raised `DUPLICATE_MODULE_ID` on every filesystem-backed reload (apcore-python#33).** `ReloadModule` unregisters the module, calls `_rediscover_module` — which runs `Registry.discover()` — and then calls `_reregister_module`, which called `Registry.register_internal`. But `discover()` *registers* what it finds, so the module was already published under its id by the time `register_internal` was asked to publish it again, and `register_internal` rejects an already-registered id with `InvalidInputError(code=DUPLICATE_MODULE_ID)`. Neither of the two escapes that could have made the sequence benign existed: `_register_in_order` skips ids already in `_modules`, but the reload unregisters first, so the id is absent and discovery does register it; and `register_internal` has no same-instance idempotent path — `detect_id_conflicts` returns an ERROR for the exact-duplicate id regardless of what object is being registered.

  **A single reload raised. A bulk reload lied.** `_execute_single` let the error propagate, so `reload_module` with a `module_id` failed outright. `_reload_one`, reached through a `path_filter`, raised inside `_execute_bulk`'s per-module `try`, which logs and continues — the call returned `{"success": True, "reloaded_modules": []}` while reloading nothing.

  `_reregister_module` now publishes only when the registry is not already holding that exact instance. When discovery published it the reload is already complete, and its entry is the higher-fidelity one: discovery writes `_versioned_modules` and merged metadata, whereas `register_internal` deliberately skips version tracking (D11-001), so the old code would have *downgraded* a discovered module into a sys/internal entry that `get(id, version_hint=...)` cannot resolve — a second defect the fix removes. A programmatic reload that hands over an unregistered instance still goes through `register_internal` unchanged, and a *different* instance registered under the id is still a genuine duplicate and still raises.

  **Why no test caught it:** every existing test that reaches the reload path stubs one of the two halves (`_rediscover_module`, `_reregister_module`, or `_reload_one` itself), so nothing ever registered before `register_internal` ran. The new `TestReloadModuleRealDiscoveryPath` in `tests/sys_modules/test_control.py` writes a real module file to a temporary extensions root, registers it through `Registry.discover()`, and reloads it with **nothing patched** — single, bulk, and repeated — plus a case asserting the reloaded module stays version-tracked, which distinguishes this fix from an unregister-then-`register_internal` one.

- **`$APCORE_CONFIG_FILE` no longer injects a phantom `config.file` key into the declared document (apcore#88).** The variable is the documented way to point at a configuration file (PROTOCOL_SPEC §9.14 discovery, read by `discover_config_file`), but §9.2 also makes *every* `APCORE_*` variable a configuration override and nothing exempted this one. Its suffix lowered to the dot-path `config.file`, so `Config.load(path)` with the variable set produced `{'version': …, 'project': …, 'config': {'file': '/path/…'}}` — a key `schemas/` declares nowhere (checked against `conformance/fixtures/config_key_governance.json`) sitting inside the **declared** document, which is the view §9.1's required-field check runs against. It is now dropped at the parse site in `_apply_env_overrides`: the variable is an *argument to* `load()` that happens to share a namespace with configuration, consumed to locate the file and then discarded, which is what every other argument-shaped input does. No spec change and no user-visible rename; file discovery is unaffected. Both legacy and namespace mode were affected and both are pinned by `TestConfigFileEnvVarIsNotAnOverride`, which asserts the **exact** declared key set rather than the absence of `config.file` — absence alone also holds for an implementation that lost a key the file really declares. The exemption is one variable wide: `APCORE_BINDINGS_DIR` → `bindings.dir` is a declared key and keeps working, asserted by a third case in the same class. The distinguishing test for any future variable is whether its dot-path is in the canonical key surface.
- **The conformance fixtures-directory locator moved out of the `APCORE_` prefix: `$APCORE_FIXTURES` → `$CONFORMANCE_FIXTURES` (apcore#88).** The exact twin of the `APCORE_SPEC_REPO` → `CONFORMANCE_SPEC_REPO` rename of apcore#86, and for the same reason: §9.2 lowered `APCORE_FIXTURES` to the config key `fixtures`, which no schema declares. This is test infrastructure, not configuration, so it does not belong in the prefix the spec has claimed. The old name is still read as a transitional fallback (`_LEGACY_FIXTURES_ENV`), and failure messages name whichever variable was actually set. No CI workflow sets either name.
- **BEHAVIOUR CHANGE: the documented nested `retry:` block on a subscriber is now read from config, on all five built-in types (apcore#85).** `features/event-system.md` documents a per-subscriber retry policy and shows it under a heading reading *"showing the policy on multiple subscriber types"* — an `a2a` entry with `max_attempts: 5` and a `file` entry with `max_attempts: 2`. **No SDK parsed it.** An operator who copied that example got the default policy, silently, with nothing to indicate the block had been ignored: `schemas/sys-modules.schema.json` does not describe subscriber entries beyond requiring a `type`, so nothing rejected the key either.

  The capability was already built at every other layer, which is why this survived. `EventRetryConfig` (`events/retry.py`) declares exactly the four keys the document shows and is documented as the policy "for event delivery to a single subscriber"; **every** subscriber constructor already accepted one and stored `self.retry`; and `emitter._get_retry_config` reads `getattr(subscriber, "retry", None)` with no type check and no allowlist, so whatever a subscriber carries is honoured. The single missing layer was YAML → object: only `_default_webhook_factory` built a policy, and only from the *legacy flat* `retry_count` shorthand. `a2a`, `file`, `stdout` and `filter` never constructed one.

  The new `sys_modules.registration._parse_retry_config` parses the block and every built-in factory passes the result through. Partial blocks merge over the spec defaults (`max_attempts=3`, `initial_backoff_ms=100`, `max_backoff_ms=30000`, `backoff_multiplier=2.0`), as the documented `file` example requires — it declares only two of the four keys. A `retry:` that is not a mapping is ignored rather than fatal.

  **All five types, not three.** An earlier reading held that `stdout` and `filter` are local and synchronous, so a policy on them would be inert. That is wrong on inspection: `FileSubscriber.on_event` catches, logs and **re-raises**; `StdoutSubscriber` is a bare `print(...)` whose failure propagates (EPIPE, closed stdout); and `FilterSubscriber` delegates, so a retry there re-runs delegate delivery. The emitter's retry loop acts on all five.

  **Flat `retry_count` still works** for `webhook` as a deprecated alias, with its existing `max_attempts = retry_count + 1` translation — that spelling is what deployments use today. **The nested block wins when both are present.**

  **This changes delivery behaviour for anyone who had already written the documented block**: a subscriber that was silently retrying 3 times now retries as configured. Pinned by `tests/events/test_subscriber_retry_config.py`, one case per subscriber type plus an end-to-end case asserting the configured `max_attempts` drives the real number of `on_event` invocations. Every asserted value differs from the default, so a case cannot pass against a factory that ignores the block.

---

- **Tautological assertions swept out of `tests/conformance/` (Issue #32, aiperceivable/apcore#81).** Twenty assertions compared a fixture value against a literal transcription of itself — `assert expected["error"]["code"] == "INVALID_PARENT_ID"`, `assert expected["span_created"] is False`, `assert case["expected"]["raised_at"] == "strategy_construction"` and the like. That shape cannot fail on SDK behaviour, and in `test_trace_context.py` it was the *only* place the driver mentioned the error code, which is why the missing `code` above shipped undetected. Every expectation is now bound to an **observed** value (the raised exception's `.code`, the number of probes the circuit breaker actually admitted, the phase that actually raised, the disk artefact that actually exists); the four remaining fixture-vs-literal comparisons are input preconditions explicitly labelled as such. Files touched: `test_trace_context.py`, `test_config_key_governance.py`, `test_middleware_hardening.py`, `test_overrides_store.py`, `test_pipeline_failfast_config.py`, `test_pipeline_step_middleware.py`, `test_usage_exporter.py`.

- **Step middlewares get their `on_step_error` when the `middleware_before` step fails.** The engine narrowed the `on_step_error` audience to `exc.executed_middlewares` for *any* `MiddlewareChainError`. That set is correct only for a `before_step` failure; the `middleware_before` step body re-raises the **module-level** chain error, whose `executed_middlewares` are `Middleware` instances, not `StepMiddleware`s — so every registered step middleware was silently swapped out for objects that have no `on_step_error`, and none was notified. Now that a `before_step` failure is handled on its own path, the step-body path simply notifies all registered step middlewares.

- **Correlation IDs are no longer redacted (MUST-violation, observability.md § Redaction configuration).** *"Implementations MUST NOT redact `trace_id`, `caller_id`, `module_id`, or `span_id`; these correlation fields MUST appear unmodified in every log entry"* — the exemption existed only in `_apply_redaction_config`, a flat non-recursive helper that **neither** mandated surface calls. The spec requires redaction *"both at log emission (in `ContextLogger`) and at the executor's input/output capture point"*, and both of those run recursive engines (`_redact_secrets_recursive` via `ContextLogger._emit`; `_redact_by_keys_and_regex` via `redact_sensitive`) that had no guard at all — so a broad `regex_patterns` or `sensitive_keys` entry scrambled `trace_id` on the value rule *and* on the name rule, breaking trace stitching exactly where it matters. `PROTECTED_LOG_FIELDS` now lives in `apcore.utils.redaction` as the single canonical set, and both engines consult it ahead of the name rule **and** the value regex, at every depth. Matching apcore-rust `NEVER_REDACT_FIELDS` (`redact_inner`): containers under a protected key are still descended into — only the protected field's own scalar value is immune, and array elements, having no key, are never protected. The `correlation_fields_never_redacted` case of `conformance/fixtures/redaction_config.json` was a `strict=True` xfail in both driver classes and now passes for real.

- **`auto_schema: strict` no longer rejects schemas OpenAI accepts.** The previous detector flagged every `anyOf` — including the nullable wrapper Pydantic emits for `X | None`, so **any optional field** failed — and rejected the supported `date-time` / `date` / `time` / `email` / `uuid` formats. It also missed genuinely unsupported keywords (`allOf`, `minLength`/`maxLength`, `patternProperties`, `uniqueItems`) and never traversed `$defs`, leaving nested models unchecked. Replaced by `apcore.schema.openai_strict`, kept separate from `to_strict_schema()` so the OpenAI dialect does not leak into general registry export.

- **Root-level combinators are enforced on every call path.** A schema whose top level is `oneOf` / `anyOf` / `allOf` / `enum` / `const` / `not` produced a field-less `extra="allow"` model that accepted any input; only `SchemaValidator._validate_top_level_union` applied the union rules, and module invocation does not go through it. The assertion now lives on the model itself.

- **`{"type": "integer"}` accepts `4.0`.** JSON Schema §6.1.1 makes any number with a zero fractional part an integer; pydantic's strict mode would have rejected it, which would have turned the coercion fix above into a *new* three-way divergence. `4.5` and `"4"` are still rejected.

- **`uniqueItems` over objects no longer raises `TypeError`.** `len(v) != len(set(v))` escaped the module call as a bare `TypeError` instead of `SCHEMA_VALIDATION_ERROR`; members are now compared by canonical JSON, so key order is correctly irrelevant.

- **A combinator on `items` is no longer dropped**, which had widened every array element to `Any`.

- **`_DictSchemaAdapter.model_validate()` accepts `strict`**, so a module declaring its schema as a raw dict keeps working.

- **BREAKING: §6/§10.3 keywords are enforced at a `type`-less position.** A sub-schema with no `type` widened to `Any` and only the eight applicator keywords were delegated onward, so `required`, `items`, `contains`, `minItems`/`maxItems`, `uniqueItems`, `minProperties`/`maxProperties`, `additionalProperties` and `properties` were asserted by *nothing* — `{"required": ["b"]}`, the usual shape of an `if` / `then` / `dependentSchemas` branch, accepted `{"a": 1}` (type-mapping §17.1 R1). apcore-typescript and apcore-rust both rejected it.

- **BREAKING: `type`-less constraints are inert on other instance types.** `minLength` / `pattern` / `minimum` and friends were collected into a Pydantic `Field` even with no `type` sibling, so `{"minLength": 3}` rejected `[1]` and `{"a": 1}` — instances §17.1 R2 requires it to pass. The `minimum` and `pattern` variants escaped as a bare `TypeError` from pydantic's `apply_known_metadata` fallback, which `BuiltinInputValidation` does not catch, killing the module call with an uncoded error. Both now route through the jsonschema delegation, which is inert by construction.

- **`allOf` with non-object members is registrable.** It raised `SchemaParseError` at model-build time, making a contract apcore-typescript and apcore-rust both accept *and enforce* impossible to register. It now widens to `Any` and the existing sibling assertion enforces every member.

- **Per-module `resources.timeout` is honoured.** `BuiltinExecute` read `module.timeout_ms`, an attribute no apcore-python module ever defines, so the per-module half of the dual-timeout model was dead code and the "negative timeout → `GENERAL_INVALID_INPUT`" rule was unreachable. It now reads `resources.timeout` from the module attribute or `annotations.extra`, matching apcore-typescript and apcore-rust; `0` means "no per-module limit" on both sides of the fallback.

- **`call_with_trace` shares `call()`'s error semantics (D-19).** It passed the raw `PipelineStepError` into recovery, so one step failure surfaced as `PIPELINE_STEP_ERROR` here and as (say) `MODULE_NOT_FOUND` through `call()` — to on_error middleware and to the caller alike. It also fabricated an empty `PipelineTrace` on the middleware-recovery path; `PipelineEngine` now publishes the live trace onto the `PipelineContext` and the real record is returned.

- **BREAKING: filesystem discovery validates module IDs.** `_validate_module_id` was reachable only from `register()` / `register_internal()`, so no ID-grammar or length check stood between a scanned or ID-map-overridden name and the registry — `{'id': 'Foo-Bar'}` or a 200-character ID went straight in. Offenders are now skipped with a warning, as in apcore-typescript and apcore-rust.

- **`register_internal` bypasses only the reserved-word check, as documented.** The `ephemeral.*` rejection raised a bare builtin `ValueError` with no error code (now `InvalidInputError` / `INVALID_MODULE_ID`, matching the other SDKs), and the streaming-annotation ⇔ streaming-interface check ran on `register()` alone.

- **ACL `handler_error` is set on every fail-closed path.** An unknown condition key and an awaitable that suspends in sync context denied with a null diagnostic, so a typo'd key (`role:` for `roles:`) was indistinguishable from a correctly-spelled unmet condition. apcore-typescript records on both.

- **`_approval_token` is stripped unconditionally (§7.4).** It was removed only inside the gated-and-handler-configured branch, so on every early return the protocol key reached input validation — where `additionalProperties: false` rejects it — and the module's `execute()`. The strip also mutated the *caller's* dict; the inputs are rebuilt instead, as in apcore-typescript.

- **BREAKING: numeric config keys reject booleans.** `bool` subclasses `int`, so ten of the fourteen constrained numeric keys read `max_call_depth: true` as a limit of 1. `docs/features/config-bus.md` states booleans are rejected for all numeric fields, and the other two SDKs reject them natively.

- **BREAKING: `ExecutionPolicy.from_dict` rejects non-boolean governance flags.** `gate_destructive: "false"` **enabled** the gate (a non-empty string is truthy) and `gate_destructive: []` disabled it, while apcore-rust's serde-typed `bool` and apcore-typescript's `_requireBoolean` reject either. `gate_destructive` is what turns a `destructive` annotation into an approval gate (§7.9.2), so the coercion was a governance decision made by accident. `null` and an absent key still mean the documented `false`.

- **BREAKING: config required-field validation is no longer dead code.** `_DEFAULTS` was deep-merged into the parsed document and `validate()` then looked for required fields in the *merged* result, so the check could never fail — the merge had already supplied every key. `_DEFAULTS` even carried an invented `version: "0.16.0"` and a `project` subtree that `schemas/defaults.schema.json` has never declared, which existed only to keep that check vacuous. Per PROTOCOL_SPEC §9.1 a key is required only when it has no canonical default, so `_REQUIRED_FIELDS` narrows from six entries to the two that qualify — `version` and `project.name` — and §9.3 step 1 evaluates them against the **declared** document, now exposed as the `Config.declared` property (apcore-rust: `Config::get_declared()`). A config omitting `extensions.root`, `schema.root`, `acl.root` or `acl.default_effect` still loads — it did before too, but for the wrong reason — while one omitting `version` or `project.name` now raises `CONFIG_INVALID` where it previously passed silently. `schemas/apcore-config.schema.json` declares the matching `required: ["version", "project"]`, and apcore-typescript is deleting the identical invented pair from `config-defaults.ts`. Consumers wanting a fallback for an undeclared `project.*` value pass it at the call site (`config.get("project.source_root", "")`), as `sys_modules` already did.

### Changed


- **Conformance drivers read the canonical fixture.** Every driver under `tests/conformance/` now resolves through `conformance.canonical_fixtures` (`$APCORE_FIXTURES` → `$APCORE_SPEC_REPO` → sibling checkout) instead of a vendored copy under `tests/conformance/fixtures/`, so a spec-side edit reaches Python on the next run rather than leaving it on a stale snapshot — the contract apcore-typescript and apcore-rust already honour. `pipeline_hardening.json` and `system_modules_hardening.json` were vendored but read by nothing; their hand-written drivers now carry a `TestFixtureCoverage` guard that fails when the canonical fixture gains a case, as do the `event_management_hardening` and `observability_hardening` drivers. `tests/conformance/fixtures/` has since been **deleted** — every remaining copy was verified byte-identical to its canonical counterpart first — and `test_no_vendored_fixture_drift.py` now asserts the directory stays gone rather than that its contents still match.

- **Conformance drivers moved to the module-invocation path.** The union and recursive hardening fixtures were driven through `validate_schema_dict()` — the raw JSON Schema validator, which is not what a module call goes through. That is what hid the recursive-schema defect. New fixtures `schema_keyword_parity.json` (119 cases), `schema_strict_conversion.json` (16) and `openai_strict_compat.json` (30) have drivers with the same contract.

## [0.26.0] - 2026-07-13

### Added

- **Execution-time governance policy (#76 RFC pilot).** New `ExecutionPolicy`, `PolicyRule`, and `PolicyDecision` types (exported from the `apcore` root) let a platform operator override the governance annotations of already-registered modules at execution time — independent of how they were registered. A policy attaches to the `Executor` via a new `policy=` parameter (also on `Executor.from_registry`) and the runtime `Executor.set_policy()` setter, and is consulted by the approval gate (Step 5). Pattern matching reuses the ACL wildcard semantics (Algorithm A08) and specificity scoring (Algorithm A10); on a specificity tie the more restrictive rule wins. A matched rule overrides the module's own declared/scanned `requires_approval` / `destructive` annotations, and every policy-driven override is recorded in the audit trail (log + tracing span event). `ExecutionPolicy.from_dict` parses a YAML/JSON governance document **strictly** — unknown keys raise `ValueError` so a typo cannot silently disable a control. `Executor.validate()` preflight now reports the same `requires_approval` verdict the gate will enforce under a policy. When the gate is policy-forced, the `ApprovalRequest.annotations` handed to the handler carries the **effective** governance values, preserving the "requires_approval is guaranteed true" contract (PROTOCOL_SPEC §7).

- **Governance events on the event bus (#77 pilot).** When the `Executor` has an `event_emitter`, the governance chain now publishes three canonical events: `apcore.approval.decision` on every approval adjudication (handler decisions and the strict fail-closed rejection; severity `info` for approved/pending, `warn` for rejected/timeout), `apcore.policy.override` whenever a policy changes a module's effective governance, and `apcore.acl.denied` (severity `warn`) when an ACL check denies a call. Payloads carry `module_id`, `trace_id`, and event-specific keys (`status`/`approved_by`/`approval_id`, `pattern`/`requires_approval`/`destructive`, or `caller_id`). Canonical names are proposed in apcore#77, pending the PROTOCOL_SPEC §9.16.2 amendment. A skipped approval gate emits nothing (parity with the no-audit-log-when-skipped contract), and the `apcore.acl.denied` event is suppressed during `validate()` preflight (dry-run) so a probe never emits a spurious denial.

### Removed

- **Legacy dual-emission of unprefixed event names (#78).** The registry bridge and `PlatformNotifyMiddleware` no longer emit the deprecated unprefixed aliases `module_registered` / `module_unregistered` / `error_threshold_exceeded` / `latency_threshold_exceeded` alongside their canonical `apcore.registry.*` / `apcore.health.*` names. PROTOCOL_SPEC §9.16 declared these removed as of v0.22.0 (`MUST` emit only canonical names); the code had kept dual-emitting them for a back-compat window. An ecosystem audit found no remaining subscriber to the bare names, so the aliases are now gone — subscribers must use the canonical `apcore.<subsystem>.<event>` names (a `*` / `apcore.*` glob subscription is unaffected). Aligns Python with the TypeScript SDK, which already emitted canonical-only.

### Changed

- **Resolve `destructive` ↔ approval semantics (#76).** `ExecutionPolicy(gate_destructive=True)` makes any module whose effective `destructive` annotation is true require approval even when `requires_approval` is false — the opt-in resolution of the long-standing footgun where an inferred `DELETE` was `destructive=True` yet ungated. Orthogonality remains the default (no behavior change without a policy).

- **Approval gate fails loud, not silent (#76, security principle).** When a module needs approval but no `ApprovalHandler` is configured, the gate keeps the PROTOCOL_SPEC §7.4 skip behavior but now logs a `warning` (once per module) instead of silently no-opping. `ExecutionPolicy(strict=True)` upgrades this to fail **closed** (raises `ApprovalDeniedError`). A module annotated `destructive=True` that no approval gate covers is likewise warned about once per module. Existing behavior without a policy and with a handler configured is unchanged.

## [0.25.0] - 2026-06-22

### Added

- **Config-driven ACL discovery (#74, D-64).** New `ACL.discover(config)` classmethod resolves `acl.root` (default `./acl`) relative to the config file's directory, loads an ACL only when the path exists, and returns `None` otherwise. An `acl.root` pointing at a directory loads `<root>/global_acl.yaml`; pointing at a file loads that file directly. **Critical invariant:** a missing path attaches **no** ACL — it never synthesizes a default-deny ACL. Discovery is auto-wired in `APCore.__init__`, and is skipped when the caller supplies their own `Executor` so an explicitly configured ACL is never clobbered. New tests and `examples/acl_config_driven.py` cover the behavior; the cross-language contract is locked by the apcore conformance fixture `acl_root_discovery.json`.

- **Registry module-id constants are now part of the public surface (#30).** `MAX_MODULE_ID_LENGTH`, `RESERVED_WORDS`, `REGISTRY_EVENTS`, `EPHEMERAL_NAMESPACE_PREFIX`, `DEFAULT_MODULE_VERSION`, and `MODULE_ID_PATTERN` are re-exported from both the `apcore.registry` package and the `apcore` root (previously reachable only via the internal `apcore.registry.registry` module). Canonical `MAX_MODULE_ID_LENGTH` is now public; the `MAX_MODULE_ID_LEN` alias is retained. Export surface only — no behavior change.

## [0.24.1] - 2026-06-18

### Changed
- rename private _bind_executor to public bind_executor on Context
- add deprecated alias _bind_executor with deprecation warning
- replace all internal calls to _bind_executor with bind_executor
- update docstrings to reflect the new method name

## [0.24.0] - 2026-06-12

### Changed

- **`ToggleState` is now per-`APCore`-instance instead of process-global (#71).** Each `APCore` instance owns one `ToggleState` (`APCore.toggle_state`), injected into BOTH the write path (`ToggleFeatureModule`, via `register_sys_modules(..., toggle_state=...)` → `_register_control_modules`) and the read path (`BuiltinModuleLookup`, via the Executor's strategy). `Executor.__init__` gains a keyword-only `toggle_state` parameter that is threaded into `build_standard_strategy(..., toggle_state=...)` (and the internal/testing/performance/minimal factories that build a `BuiltinModuleLookup`). Disabling a module on one `APCore` no longer disables it on another instance in the same process, and an instance's toggles survive a registry reload of that instance (re-scopes A-D-12 from process-global to instance-scoped). The module-global `_default_toggle_state` is retained as the fallback only for the free `is_module_disabled(module_id)` function (signature unchanged) and for Executors constructed directly without a `toggle_state` (back-compat).

### Added

- **Conformance coverage for per-instance ToggleState isolation and AI-agent ACL governance (#71, #72).** Wired two new cross-language fixtures into `tests/test_conformance.py`: `toggle_state_isolation.json` (4 cases) constructs real `APCore` instances in one process and asserts that toggles written through one instance's `disable`/`enable`/`reload` write path are observed only through that instance's read path; `acl_agent_scoping.json` (19 cases) locks the canonical default-deny agent-tool-governance ruleset, exercising first-match-wins with `{roles, max_call_depth}` conditions and the `@external` special caller (`max_call_depth` is inclusive: `depth == max` is allowed). All 23 cases pass against the existing ACL engine with no engine changes required.

### Fixed

- **ACL `max_call_depth` now accepts an integral float threshold [A-D-004].** A threshold loaded from YAML/JSON as `5.0` (instead of `5`) previously failed the `isinstance(..., int)` check in `_MaxCallDepthHandler`, so the allow-rule silently did not match and the call fell through to default-deny. The handler now accepts a `float` threshold when it is integral (`5.0` → depth 5) — same limit, no security change — matching apcore-typescript. `bool` thresholds and non-integral floats (e.g. `5.5`) are still rejected (fail closed).

- **Registry custom discovery guards against a non-list discoverer result [A-D-014].** `Registry._discover_custom` previously iterated the value returned by a custom discoverer directly; a non-list return (e.g. `None` or a scalar) raised an uncaught `TypeError`. It now checks the result is a `list`, logs a warning, and returns `0`, matching apcore-typescript's `!Array.isArray` guard.

- **README built-in namespaces table corrected [B-011, B-012].** Added the missing pre-registered `obs` / `APCORE_OBS` namespace (redaction.*) and fixed the `sys_modules` key paths to the actual nesting under `events.thresholds.*` (`events.thresholds.error_rate`, `events.thresholds.latency_p99_ms`).


## [0.23.0] - 2026-06-10

### Added

- **AI error-recovery metadata is now populated at the source (#70).** Framework-deterministic errors carry default recovery metadata so the contract flows to every surface (MCP/CLI/A2A) from one definition instead of being backfilled per adapter. A new declarative `_USER_FIXABLE_BY_CODE` policy in `errors.py` resolves `user_fixable` from the error code in `ModuleError.__init__`: `True` for caller-fixable codes (`SCHEMA_VALIDATION_ERROR`, `GENERAL_INVALID_INPUT`, `MODULE_NOT_FOUND`, `VERSION_CONSTRAINT_INVALID`, `BINDING_SCHEMA_INFERENCE_FAILED`, `BINDING_SCHEMA_MODE_CONFLICT`, `BINDING_STRICT_SCHEMA_INCOMPATIBLE`, `DEPENDENCY_NOT_FOUND`, `DEPENDENCY_VERSION_MISMATCH`); `False` for governance/system/structural/transient codes (`ACL_DENIED`, `APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `MODULE_TIMEOUT`, `MODULE_DISABLED`, `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL`, `CALL_FREQUENCY_EXCEEDED`, `GENERAL_INTERNAL_ERROR`). Codes absent from the policy (e.g. `MODULE_EXECUTE_ERROR`) leave `user_fixable` unset for the module author to supply. Missing `ai_guidance` defaults are filled on `InvalidInputError` and `CallFrequencyExceededError`. Explicit constructor values still override the policy; `to_dict()` now emits `user_fixable` for the mapped codes. Locked across SDKs by the new conformance fixture `error_recovery_metadata.json`. `suggestion` is intentionally left unset (redundant with `ai_guidance`); `x-*` metadata remains author-owned.


### Changed (breaking)

- **`CircuitBreakerMiddleware` rewritten to the spec-mandated rolling-window error-rate model [D11-001].** Per `middleware-system.md` §1.2, the breaker now tracks a bounded rolling window of recent outcomes per **`(module_id, caller_id)`** pair (keyed off `context.caller_id`, empty string when absent) and opens when the window error rate meets or exceeds `open_threshold` once at least `min_samples` outcomes are recorded. The old consecutive-failure-count model is gone. **Breaking constructor change**: now keyword-only — `open_threshold` (default `0.5`), `recovery_window_ms` (default `30000`), `window_size` (default `20`), `min_samples` (default `5`), plus `emitter`, `priority` (`100`), and an optional `clock` seam for tests. The previous `failure_threshold` / `success_threshold` parameters are **removed**. `get_state()` and `reset()` now accept an optional `caller_id`. The middleware writes the current state string to `context.data["_apcore.mw.circuit.state"]` on every call and emits `apcore.circuit.opened` / `apcore.circuit.closed` (payload `{module_id, caller_id, error_rate}`) via the injected `EventEmitter` on transitions. `CircuitBreakerOpenError` now also carries `caller_id` (new property; `module_id` and the legacy `CircuitOpenError` alias unchanged). Config keys under `middleware.circuit_breaker` change accordingly: `failure_threshold`/`success_threshold` → `open_threshold` (number in `[0,1]`), `window_size` (int ≥ 1), `min_samples` (int ≥ 1); `recovery_window_ms` default is now `30000`. Matches the Rust and TypeScript SDKs. **Breaking** for callers constructing the middleware with the old positional/threshold parameters — pre-1.0 0.x, acceptable.


### Fixed

- **DLQ `original_event` now nests `module_id`/`timestamp` under `metadata` [D11-002].** The `apcore.event.delivery_failed` payload built by `EventEmitter._emit_dlq` previously emitted `original_event.metadata` as an empty `{}`, dropping the originating module and timestamp. It now carries `metadata: {module_id, timestamp}` from the original `ApCoreEvent` envelope, matching the canonical `{name, payload, metadata:{...}}` shape in `event-system.md`.

- **`A2ASubscriber` no longer retries 4xx responses (#69).** It previously raised on any HTTP `status >= 400`, contradicting the spec (`event-system.md`: 4xx MUST NOT be retried, for both Webhook and A2A) and diverging from `WebhookSubscriber`. `A2ASubscriber.on_event` now mirrors Webhook: 5xx (and connection/timeout) → raise → retried → `apcore.event.delivery_failed` on exhaustion; 4xx → logged permanent, no retry, no DLQ. Per-SDK regression tests lock both subscribers' 4xx/5xx behavior.


## [0.22.0] - 2026-05-28

### Changed

- **`Context.create()` signature unified across all SDKs (apcore PROTOCOL_SPEC §"Contract: Context.create", [apcore#66](https://github.com/aiperceivable/apcore/issues/66)).** The factory now accepts **exactly** six caller-supplied fields in this order: `identity`, `trace_parent`, `cancel_token`, `data`, `services`, `global_deadline`. The previous `executor=` parameter is **removed**; `executor` is now bound by the Executor at pipeline entry via the new SDK-internal `Context._bind_executor(executor)` helper (see PROTOCOL_SPEC §"Contract: Executor binding to Context"). `caller_id` remains managed exclusively by `Context.child()` and was never a public input. Two parameters are newly first-class: `cancel_token` (eliminates the post-hoc `ctx.cancel_token = token` anti-pattern) and `global_deadline` (previously only settable via mutation). `Executor.call` / `call_async` / `stream` / `call_async_with_trace` / `_validate_async` all bind `self` to the supplied or auto-created Context before pipeline step 1; same-instance rebinds are idempotent noops, cross-Executor rebinds raise the new `ContextBindingError` (code `CONTEXT_BINDING_ERROR`). The root-call `global_deadline` computation moved out of "context-was-None" branch into a general "root call" check in `BuiltinContextStep`, ensuring local-config-driven recomputation per PROTOCOL_SPEC §"Contract: `global_deadline` distributed semantics" — including deserialized Contexts arriving at remote nodes. Tests and `examples/cancel_token.py` are updated to the new shape. **Breaking** for callers that passed `executor=` to `Context.create()` — pre-release v0.22.0, acceptable.

- **`TaskStore` Protocol is now fully async (D-17 / A-D-AT-04).** All five methods (`save`, `get`, `delete`, `list`, `list_expired`) are declared `async def` on the Protocol and on `InMemoryTaskStore`. Custom stores written against the pre-0.22.x sync surface must be migrated; `AsyncTaskManager` keeps a transitional compatibility shim that awaits a returned coroutine if a legacy store still exposes sync methods. The uniform async shape unblocks Redis/SQL/network-backed stores without an extra blocking adapter, matching the TypeScript and Rust SDKs.

- **`ReaperHandle.stop()` and `AsyncTaskManager.stop_reaper()` are now `async` and drain the reaper task (D-11 / A-D-AT-03).** Callers must `await handle.stop()` / `await manager.stop_reaper()`. After the coroutine returns the underlying `asyncio.Task` is guaranteed to be settled — previously the sync `stop()` only requested cancellation and required a manual `await asyncio.sleep(0)` for the task to finish. `AsyncTaskManager.shutdown()` now awaits `stop_reaper()` directly.

- **`AsyncTaskManager.get_status()` and `list_tasks()` return defensive snapshots (A-D-AT-06).** Both methods now hand back shallow copies of `TaskInfo` via `dataclasses.replace`, matching the TypeScript SDK's `{ ...info }` and the Rust SDK's `info.clone()`. Mutating the returned objects no longer corrupts the live store. Async-friendly twins `get_status_async()` / `list_tasks_async()` are available for I/O-backed stores.

- **`AsyncTaskManager.cleanup()` is now `async`.** Required because the store contract is async; callers must `await manager.cleanup(...)`. The reaper background loop already awaits internally — only direct in-process callers are affected.

- **Legacy `RetryPolicy` defaults `max_retries` to `0` and emits `DeprecationWarning` on instantiation (D-14 / A-D-AT-09).** Earlier builds silently enabled three retries when callers used `RetryPolicy()` without arguments, contradicting the opt-in retry contract. The class is retained for one release; new code should use `RetryConfig` (canonical, ms-based, no deprecation noise).

- **BREAKING: `Config.get()` no longer falls back to the implicit `apcore` namespace for a scalar segment + remainder (A-D-049).** In namespace mode, when a top-level segment resolves to a non-dict scalar and a deeper sub-path is requested (e.g. data `{foo: 5, apcore: {foo: {bar: 9}}}`, `get("foo.bar")`), Python previously recursed into `apcore.foo.bar` and returned `9`. Spec §9.9.1 does not define this fallback and the Rust SDK is spec-correct, so `get()` now returns the default (`None`). **Breaking** for any caller that relied on the implicit-apcore scalar fallback — pre-1.0 0.x, acceptable.

- **BREAKING: `Config.set()` is now namespace-aware, symmetric with `get()` (A-D-050).** `set()` previously wrote via a plain dot-path while `get()` resolved the registered-namespace longest-prefix. For a registered namespace whose name contains dots (e.g. `my.app`), `set("my.app.transport", v)` wrote to `data["my"]["app"]["transport"]` while `get("my.app.transport")` read `data["my.app"]["transport"]` — a silent round-trip failure. `set()` now resolves the registered-namespace prefix before writing, so `set`/`get` round-trip to the same location. **Breaking** only for callers that depended on the previous asymmetric naive-split write — pre-1.0 0.x, acceptable.

- **Middleware circuit-breaker state enum renamed `CircuitState` → `CircuitBreakerState` (audit D2-001).** Adopts the cross-SDK canonical name for naming parity (matches `apcore-rust`; the TypeScript SDK uses `MiddlewareCircuitState`) and disambiguates from the unrelated events circuit-breaker `CircuitState` (`apcore.events`), which is unchanged. The top-level export `from apcore import CircuitBreakerState` and `apcore.middleware.CircuitBreakerState` now resolve to the renamed enum; members (`CLOSED`, `OPEN`, `HALF_OPEN`) are unchanged. No deprecation alias is provided. **Breaking** for direct importers of the middleware `CircuitState` — pre-1.0 0.x, acceptable.

### Fixed

- **Cancel token observed at Step 2 of the execution pipeline (D-21 / A-D-EXEC-002).** `BuiltinCallChainGuard` now checks `context.cancel_token.is_cancelled` before running any guard work; a cancelled token short-circuits with `ExecutionCancelledError` before ACL, middleware, validation, or module execution. Combined with the existing Step 8 check, the pipeline now satisfies the two-point cancel-token invariant — single-check implementations were leaking compute through ACL/middleware/validation even when the caller had already cancelled.

- **`MiddlewareChainError` unwrap rule (D-22 / A-D-EXEC-005).** `Executor._recover_from_call_error` now unwraps `MiddlewareChainError` and propagates the original typed cause (e.g. `ApprovalDeniedError`, `ACLDeniedError`) unchanged. Previously the wrapper was collapsed to a generic `ModuleExecuteError`, breaking callers that dispatch on the typed error (notably MCP/A2A bridges keying on `APPROVAL_DENIED` vs `MODULE_EXECUTE_ERROR`). Mirrors the TypeScript and Rust SDK semantics.

- **`Registry.register_internal` ephemeral-namespace rejection covers bare `"ephemeral"` (A-D-REG-002).** Previously the check used `module_id.startswith("ephemeral.")`, which missed the bare ID `"ephemeral"` and contradicted the canonical `_is_ephemeral` classifier used everywhere else in the registry. The helper is now shared between both call sites.

- **Discover-path registration enforces the Issue #65 deferred-publish invariant (A-D-REG-003).** `_register_in_order` now reserves an in-flight slot, runs `on_load()` outside the lock, and only publishes into `_modules` / `_versioned_modules` on success. Previously the discover path published *before* invoking `on_load` and relied on rollback, leaving a window in which `registry.get()` callers could observe a module whose `on_load`-installed state (warmed pools, primed caches) was incomplete.

- **`Registry.register_internal` enforces the Issue #65 deferred-publish invariant (A-D-REG-004).** `register_internal` now routes through the same three-phase protocol as `register()`: reserve in-flight slot → run `on_load` outside the lock → publish. On failure it removes the in-flight slot, emits `apcore.registry.module_load_failed`, and re-raises the original exception unchanged. The invariant now holds uniformly for every registration path (public `register`, `register_internal`, and discover).

- **Discover-path emits `apcore.registry.module_load_failed` on `on_load` failure (A-D-REG-005).** Earlier the discover path logged at ERROR and silently dropped the module; subscribers had no portable hook to detect partial-init failures from the auto-discovery pipeline. The event payload matches the one emitted by `register()` (Issue #65) so a single subscriber covers all registration paths.
- **`InMemoryTaskStore.list_expired` no longer reaps terminal tasks with a null `completed_at` (A-D-004).** The method previously fell back to `submitted_at` when `completed_at` was `None`, so a terminal task lacking a completion timestamp could be returned (and reaped). The `TaskStore.list_expired` contract — and the TypeScript and Rust SDKs — require `completed_at` to be non-null; the fallback is removed. `AsyncTaskManager.cleanup()` retains its own distinct, contract-specified `submitted_at` reference selection and is unaffected. Found via `/apcore-skills:sync --scope core`.

- **ACL `max_call_depth` rejects boolean thresholds — fail-closed security fix (A-D-D-012).** Because Python `bool` is a subclass of `int`, an ACL condition like `max_call_depth: true` previously coerced to threshold `1` and could ALLOW a shallow call (call-chain length ≤ 1) where the TypeScript and Rust SDKs fail closed. `_MaxCallDepthHandler` now rejects `bool` explicitly (including inside the `{lte: ...}` form) and does not match — closing an allow/deny divergence.

- **Cancellation short-circuits before `on_error` middleware recovery (D-20 / A-D-003, A-D-004).** When a pipeline step (module or middleware) raises `ExecutionCancelledError`, the engine wraps it in `PipelineStepError`; `Executor.call_async` and `Executor.stream` previously routed that into `_recover_from_call_error`, so a recovering `on_error` handler could swallow the cancellation. Both paths now re-check the unwrapped cause (also unwrapping `MiddlewareChainError`) and re-raise `ExecutionCancelledError` immediately, skipping the recovery chain — matching the Rust SDK.

- **`Executor.validate()` is non-throwing on a foreign-bound Context (A-D-008).** When the supplied Context was already bound to a *different* Executor, the unguarded `_bind_executor` call let `ContextBindingError` escape `validate()`, which must always return a `PreflightResult`. The bind is now wrapped; a binding conflict yields `PreflightResult(valid=False)` with a failed `executor_binding` check instead of raising. Mirrors the Rust SDK.

- **`SchemaValidator.validate` populates the canonical `error_code` (A-D-036 / A-D-034).** The public Pydantic validation path returned a `SchemaValidationResult` with `error_code=None` on failure; it now sets `error_code = "SCHEMA_VALIDATION_ERROR"` (spec §8.2) on plain validation failure, aligning with `SchemaValidationError.code` and cross-SDK expectations.

- **`$ref` depth-cap exhaustion now raises `SchemaMaxDepthExceededError` (`SCHEMA_MAX_DEPTH_EXCEEDED`) instead of `SchemaCircularRefError` (A-D-038).** `RefResolver.resolve_ref` previously emitted `SchemaCircularRefError` (`SCHEMA_CIRCULAR_REF`) for both genuine cycles **and** depth-cap exhaustion (`depth >= max_depth`). The two are now distinct: the depth-cap branch raises the new `SchemaMaxDepthExceededError` while genuine cycle detection still raises `SchemaCircularRefError`, matching the Rust SDK and spec §8.2 (`apcore/docs/features/error-system.md`). The new error class is **additive (non-breaking)** and exported from the top-level `apcore` package, but this is a **code CHANGE for the depth-cap path**: callers that caught `SchemaCircularRefError` / `SCHEMA_CIRCULAR_REF` to handle depth-cap exhaustion must now also handle `SchemaMaxDepthExceededError` / `SCHEMA_MAX_DEPTH_EXCEEDED`.

### Removed

- **Misplaced spec-style docs (B-005).** Deleted `docs/features/async-task-evolution.md`, `docs/features/middleware-architecture-hardening.md`, and `docs/async-task-evolution/test-cases.md`. Per the apcore protocol-spec repo policy (`apcore/CLAUDE.md`), implementation repos contain only code and a README — feature specs, test-case matrices, and design notes live in the apcore spec repo. The deleted files were also stale (referenced the obsolete `TaskStore.put` method and the removed `TaskStatus.RETRYING` enum value); the canonical authority is the implementation plus the upstream `apcore/docs/features/async-tasks.md` spec.

### Added

- **`Config.reserved_namespaces()` classmethod + top-level `RESERVED_NAMESPACES` constant (PROTOCOL_SPEC §9.9.5, [apcore#60](https://github.com/aiperceivable/apcore/issues/60)).** Implements the new normative requirement that all SDKs MUST expose a public, read-only query API returning the set of reserved top-level namespace names. Returns the existing private `_RESERVED_NAMESPACES` `frozenset` — single source of truth, no parallel list — so `Config.register_namespace("apcore")` continues to raise `ConfigNamespaceReservedError` and the query API reports exactly the names that drive that enforcement. Class-level access (no `Config()` instance needed). Intended for third-party consumers (custom CLIs, framework integrations like `django-apcore`, application code) that accept user-supplied namespace names and need fail-fast pre-validation. The private constant `_RESERVED_NAMESPACES` is unchanged — internal callers keep using it.

- **Unified event delivery semantics: per-subscriber retry, DLQ, and `on_failure` callback ([apcore#61](https://github.com/aiperceivable/apcore/issues/61)).** `EventEmitter._deliver` now runs each subscriber's delivery in an independent async task via `asyncio.gather`, with an exponential-backoff retry loop controlled by the new `EventRetryConfig` frozen dataclass (`max_attempts`, `initial_backoff_ms`, `max_backoff_ms`, `backoff_multiplier`). On retry exhaustion, emits an `apcore.event.delivery_failed` dead-letter event (DLQ) to all non-wildcard subscribers; calls the optional `on_failure(event, error, attempt_count)` hook. DLQ delivery is single-attempt: a DLQ subscriber that fails is logged at ERROR and discarded — no second-order DLQ. Subscribers gain `subscriber_id` (explicit or auto-generated `{type}-{n}`), `retry: EventRetryConfig`, `event_pattern: str`, and `subscriber_type` fields. `EventRetryConfig` is exported from `apcore.events.retry` and the top-level `apcore` package.

- **`StreamingModule` Protocol with registration-time signature validation ([apcore#62](https://github.com/aiperceivable/apcore/issues/62)).** New `@runtime_checkable` `StreamingModule` Protocol in `apcore.streaming` declares `async def stream(self, inputs: dict, context: Context) -> AsyncIterator[dict]`. `Registry.register` validates the signature (arity, async, method presence) when `annotations.streaming = True`, raising `StreamingInterfaceError` (code `STREAMING_INTERFACE_MISMATCH`) on mismatch with `mismatch_reason` literals `missing_marker | not_async | wrong_arity`. `builtin_steps.BuiltinExecute` now uses `isinstance(module, StreamingModule)` instead of `hasattr`. Both `StreamingModule` and `StreamingInterfaceError` are exported from the top-level `apcore` package.

- **`ContextKey[T]` promoted as documented public API ([apcore#63](https://github.com/aiperceivable/apcore/issues/63)).** `ContextKey`, all 6 built-in constants (`TRACING_SPANS`, `TRACING_SAMPLED`, `METRICS_STARTS`, `LOGGING_START`, `REDACTED_OUTPUT`, `RETRY_COUNT_BASE`), and their `_apcore.*` identifier strings are confirmed exported at the top-level `apcore` package. `ContextKey.scoped(suffix)` creates `{name}.{suffix}` sub-keys for vendor namespacing.

- **Duplicate middleware detection in `MiddlewareManager.use()` ([apcore#64](https://github.com/aiperceivable/apcore/issues/64)).** New `MiddlewareManager.use(middleware, *, allow_duplicate=False, identity_key=None)` method. Identity defaults to `{module}.{qualname}`; overridable via `identity_key`. On duplicate detection (same identity, `allow_duplicate=False`), emits a `WARNING` log naming both call sites. Registration always proceeds; order is preserved. Keys starting with `apcore.` are reserved for framework middleware.

- **`Registry` on_load ordering — deferred-publish invariant ([apcore#65](https://github.com/aiperceivable/apcore/issues/65)).** `Registry.register` now uses a three-phase deferred-publish protocol: (1) validate + mark `module_id` in `_in_flight` set; (2) call `on_load()` outside the lock — module is NOT visible during this phase; (3) atomically insert into visible store. `get()`, `list()`, and all discovery APIs see the module only after `on_load` completes. On `on_load` failure: module is never added to the visible store, `register()` re-raises the original exception, and an `apcore.registry.module_load_failed` event is emitted (payload: `module_id`, `callback_name`, `error_type`, `error_message`, `timestamp`). Concurrent same-ID registrations during `on_load` raise `InvalidInputError(code=DUPLICATE_MODULE_ID)`. Distinct-ID `on_load` callbacks run in parallel (no global lock held during `on_load`).

### Changed

- **`A2ASubscriber` follows unified retry policy ([apcore#61](https://github.com/aiperceivable/apcore/issues/61)).** Removed A2A's previous "log-and-suppress on failure" behavior. `A2ASubscriber.on_event` now raises on HTTP/network errors so the `EventEmitter` retry loop handles backoff and DLQ emission. Adds `skill_id` constructor parameter (default `"apevo.event_receiver"`) replacing the hardcoded constant.

- **`Registry.register` concurrent same-ID rejection via in-flight set ([apcore#65](https://github.com/aiperceivable/apcore/issues/65)).** A second `register()` call for a module_id that is already executing `on_load()` now raises `InvalidInputError(code=DUPLICATE_MODULE_ID)` immediately, preventing concurrent duplicate registrations and making the registry's visible-store consistent with the deferred-publish invariant.

---

## [0.21.0] - 2026-05-06

### Added

- **`Module.preview()` + `PreflightResult.predicted_changes` (PROTOCOL_SPEC v0.21.0 §5.6 / §12.8 — promoted from RFC `apcore/docs/spec/rfc-preview-method.md`, apcore commit [`c191b85`](https://github.com/aiperceivable/apcore))** — New optional `preview(inputs, context)` method on the `Module` Protocol. Modules implementing `preview()` return a `PreviewResult` (or `None` when prediction is unavailable) whose `changes` list is a structured prediction of the state changes the call would produce — answering the AI-orchestrator-driven question *"if I were to call this module with these inputs, what would change in the world?"*. Detection mirrors the existing `preflight()` optional-method pattern (`hasattr(module, "preview") and callable(module.preview)`). `Executor.validate()` invokes the method (awaiting the result if it's a coroutine — both sync and async implementations are supported), folds `PreviewResult.changes` into the new `PreflightResult.predicted_changes` field, and records a `module_preview` advisory check on `PreflightResult.checks`. Exception semantics match `preflight()`: a raised exception is surfaced as a warning on the `module_preview` check and **does not** fail validation. New `Change` and `PreviewResult` pydantic models exported from the top-level `apcore` package. `Change` uses the Python idiomatic encoding called out in the RFC's "Change.x-* extension fields" cross-SDK schema-encoding table — `pydantic.ConfigDict(extra='allow')` paired with a model-validator that rejects extra keys not matching `^x-`, mirroring the `^x-` extension convention used elsewhere in the protocol (§4.6). Reference parity: `apcore-typescript` PR #29.

- **`ephemeral.*` namespace + `discoverable` annotation pilot (apcore RFC `docs/spec/rfc-ephemeral-modules.md`, [#25](https://github.com/aiperceivable/apcore-python/issues/25))** — Pilot implementation **ahead of upstream RFC acceptance** (RFC is in `Draft / RFC` state). Reserves the `ephemeral.*` namespace for programmatically-registered modules synthesized at runtime by LLM-agent pipelines (e.g. ToolMaker, ACL 2025, arXiv 2502.11705). Filesystem discovery now refuses to register any ID that falls under `ephemeral.*` and raises `InvalidInputError` with code `INVALID_MODULE_ID`; the namespace is reachable only via `Registry.register()`. New `discoverable: bool = True` field on `ModuleAnnotations` — when `False`, the module is excluded from `Registry.list()` (default behaviour; `include_hidden=True` returns the full set), `Registry.iter()`, `Registry.module_ids`, and downstream manifest export, while remaining callable via `Registry.get()` / `Executor.execute()`. New `Registry.set_event_emitter(emitter)` opt-in: when wired, `ephemeral.*` registrations / unregistrations emit canonical `apcore.registry.module_registered` / `apcore.registry.module_unregistered` events whose `data` payload mirrors the D-35 contextual-audit shape (`caller_id` defaulting to `"@external"`, plus a redacted `identity` snapshot when `context.identity` is set). Without an emitter the same audit information is logged at INFO so it never silently disappears. `Registry.register()` / `Registry.unregister()` accept an optional `context=` keyword used solely to enrich those audit payloads. A soft `logging.warning(...)` fires when an `ephemeral.*` module is registered without `requires_approval=True` (per the RFC). Lifecycle is caller-managed via `Registry.unregister()`; TTL/GC sweeper and host-side sandboxing are deliberately out of scope for the v1 pilot. New top-level constant `EPHEMERAL_NAMESPACE_PREFIX` exported from `apcore.registry.registry`. **Pilot disclaimer:** the upstream RFC is not yet accepted; downstream SDKs (`apcore-typescript`, `apcore-rust`) will follow once Python pilot findings are reported back.

### Changed

- **iter-11 alignment with upstream apcore RFC `rfc-ephemeral-modules.md` (apcore commit [`81df336`](https://github.com/aiperceivable/apcore/commit/81df336))** — Tightens the `ephemeral.*` pilot against two new normative rules added during the RFC iter-11 reconciliation round:
  1. **Audit-event single-emit rule** (RFC §"Audit-event single-emit rule"). The legacy `_bridge_registry_events` callback in `apcore.sys_modules.registration` now short-circuits for `ephemeral.*` module IDs so that exactly **one** `apcore.registry.module_registered` / `apcore.registry.module_unregistered` event is emitted per registration — the rich registry-side direct emit carrying the full D-35 contextual payload — instead of being followed by a second empty-payload copy from the bridge. Non-ephemeral registrations are unaffected (legacy bridge behaviour preserved for backwards compatibility).
  2. **`register_internal()` rejection** (RFC §"`register_internal()` interaction"). `Registry.register_internal()` now raises `ValueError` when called with an `ephemeral.*` module ID, directing the caller to `Registry.register()`. Rationale: namespace → registration-mechanism is a 1:1 mapping; mixing the two paths blurs the audit-trail distinction between framework-emitted (`system.*`) and caller-emitted (`ephemeral.*`) modules.

## [0.20.0] - 2026-05-05

### Added

- **Pluggable `OverridesStore` interface (sync alignment, CRITICAL #1)** — New `apcore.sys_modules.overrides` module exposes the `OverridesStore` Protocol with default `InMemoryOverridesStore` and `FileOverridesStore` (atomic YAML write via tempfile + `os.replace`) implementations, mirroring TypeScript's `apcore-typescript/src/sys-modules/overrides.ts` (`OverridesStore` / `InMemoryOverridesStore` / `FileOverridesStore`). `register_sys_modules(..., overrides_store=...)` accepts any `OverridesStore`; loaded overrides are applied to `Config` (and the live `ToggleState` for `toggle.*` keys) at startup, and `UpdateConfigModule` / `ToggleFeatureModule` persist back through the store on every successful mutation. The legacy `sys_modules.control.overrides_path` config key is retained as a backwards-compat shim that auto-constructs a `FileOverridesStore`. The previously-private `_load_overrides` helper now delegates to `FileOverridesStore` for symmetry. New top-level exports: `OverridesStore`, `InMemoryOverridesStore`, `FileOverridesStore`.

- **Public `SubscriberFactory` API (Issue #36)** — `apcore.events.register_subscriber_factory(type_name, factory)` and `apcore.events.create_subscriber_from_config(config)` (also re-exported from `apcore`) bring Python to parity with TypeScript's `createSubscriberFromConfig` / `registerSubscriberFactory` and Rust's `create_subscriber` / `register_factory`. Built-in factory types (`webhook`, `a2a`, `file`, `stdout`, `filter`) are auto-registered on import. The previously-private `_create_subscriber` helper remains for back-compat.
- **Pipeline `StepMiddleware` (Issue #33 §2.2)** — Formal middleware mechanism for pipeline steps. New `StepMiddleware` Protocol exposes optional `before_step(step_name, state)`, `after_step(step_name, state, result)`, and `on_step_error(step_name, state, error)` hooks; both sync and async implementations are supported (return values detected via `inspect.isawaitable()`, mirroring the Issue #42 async `on_error` fix). `ExecutionStrategy` gains a `step_middlewares: list[StepMiddleware]` field plus an `add_step_middleware()` registration method. `before_step` and `on_step_error` run in registration order; `after_step` runs in reverse (onion semantics). A non-`None` return from `on_step_error` is treated as a recovery `StepResult` and execution continues normally; returning `None` lets the original exception propagate. Exported from `apcore` as `StepMiddleware`.
- **`apcore.observability.batch_span_processor` module (Issue #43 §2)** — Dedicated module hosting the canonical `BatchSpanProcessor` and `SimpleSpanProcessor` implementations, mirroring the layout of the TypeScript (`src/observability/batch-span-processor.ts`) and Rust (`src/observability/processor.rs`) SDKs. Adds a synchronous `force_flush(timeout_ms=30000) -> bool` method that drains the queue while the processor remains alive (returns `True` once empty, `False` on deadline). `shutdown()` is now idempotent. Queue-full enqueues now log a (rate-limited) `WARNING` so dropped spans surface in operator logs rather than only via the `spans_dropped` counter. The classes remain re-exported from `apcore.observability.tracing` and `apcore.observability` for backward compatibility.
- **Generic pluggable `StorageBackend`** (PROTOCOL_SPEC Issue #43 §1) — New `apcore.observability.storage` module defines a namespaced `save / get / list / delete` Protocol and the default `InMemoryStorageBackend` implementation, mirroring the shape of `TaskStore`. `ErrorHistory`, `MetricsCollector`, and `UsageCollector` now accept an optional `storage: StorageBackend | None = None` constructor argument; when supplied, error entries are mirrored to the backend's `"errors"` namespace keyed by fingerprint, enabling cross-process persistence. External backends (Redis, SQL, S3) remain user-supplied. Exports: `StorageBackend`, `InMemoryStorageBackend` from `apcore.observability` and the top-level `apcore` package.
- **`TaskStore.list_expired(before_timestamp)`** (cross-language alignment D-10) — New method on the `TaskStore` Protocol returning terminal-state (`COMPLETED` / `FAILED` / `CANCELLED`) tasks whose `completed_at` precedes `before_timestamp`. Implemented on `InMemoryTaskStore`. Drives TTL-based reaper logic; non-terminal tasks are never returned. The method is REQUIRED on the Protocol — custom stores written before this release must add an implementation.
- **`Registry.discover_multi_class(file_path, extensions_root="extensions")`** (cross-language alignment D-15) — New instance method on `Registry` wrapping the existing free function `apcore.registry.multi_class.discover_multi_class`. The registry's configured `pre_approval_hook` is forwarded to the underlying scanner so signature-verification and audit policies apply uniformly. The free function remains importable for existing callers; new code SHOULD prefer the method.
- Granular reload via `path_filter` input in `ReloadModule` (#45.4) — `Registry.discover(path_filter=...)` accepts a glob string or list of patterns and only walks matching files; previously-registered modules outside the filter remain untouched. Patterns are matched (via `pathlib.PurePath.match`) against both the absolute file path and its path relative to each configured extension root.
- Error fingerprinting in `ErrorHistory` — dedup by (error_code, top-frame hash, sanitized message template) (#43 §4). New `compute_error_fingerprint(error, module_id)` folds the deepest stack-frame `file:lineno:func` (basename only, for cross-machine stability) into the SHA-256 digest in addition to the existing code/module/normalized-message inputs. Long hex runs (≥ 8 chars) are now collapsed to `<HEX>` alongside the existing UUID/timestamp/integer placeholders. Legacy 3-arg `compute_fingerprint` retained.
- Configurable redaction via `obs.redaction.regex_patterns` and `obs.redaction.sensitive_keys` Config keys (#43 §5). New `obs` namespace ships with sensible defaults (`password`, `secret`, `token`, `api_key`, `authorization`, `cookie`, `_secret_*`, …); operators can override via `apcore.yaml`. `RedactionConfig.from_config(config)` / `RedactionConfig.default()` build the runtime config; `_secret_` prefix matching becomes a default entry rather than a hard-coded rule. Field-name match is case-insensitive substring with `-`/`_`/space normalization (so `"X-API-Key"` matches `"api_key"`); value-regex match is case-insensitive. `apcore.utils.redaction.redact_sensitive` accepts new keyword overrides (`sensitive_keys`, `regex_patterns`, `replacement`).

### Changed

- **Event names normalized to `apcore.<subsystem>.<event>` form (#36)** — Four legacy event types (`module_registered`, `module_unregistered`, `error_threshold_exceeded`, `latency_threshold_exceeded`) now also emit canonical aliases `apcore.registry.module_registered`, `apcore.registry.module_unregistered`, `apcore.health.error_threshold_exceeded`, `apcore.health.latency_threshold_exceeded`. Both forms are emitted during the deprecation window so existing subscribers keep working; the legacy emission carries `deprecated: true` in `data`. Glob subscribers using `apcore.registry.*` and `apcore.health.*` now match correctly. **Deprecation:** legacy bare names will be removed in v0.22.0.
- **Contextual auditing for system control modules (Issue #45.2)** — Audit events emitted by `system.control.update_config` (`apcore.config.updated`), `system.control.toggle_feature` (`apcore.module.toggled`), and `system.control.reload_module` (`apcore.module.reloaded`) now include the requester's `caller_id` from `context.caller_id` (defaults to the `@external` sentinel when unset) and a redacted `identity` dict (`id`, `type`, `roles`) when `context.identity` is present.
- **Pipeline configuration is fail-fast (Issue #33 §1.2)** — `build_strategy_from_config` now raises `ConfigurationError` (new typed error, code `PIPELINE_CONFIGURATION_ERROR`) instead of logging a warning when YAML refers to a step that does not exist (in `remove`, `configure`, or `insert.before`/`insert.after`), assigns an unknown field via `configure`, or omits both `after` and `before` anchors on an inserted step. Misconfigurations now surface at start-up rather than producing inscrutable runtime failures.
- **Pipeline strategy dependency validation is fail-fast (Issue #33 §2.1)** — `ExecutionStrategy.__init__` and `insert_after`/`insert_before` now raise `PipelineDependencyError` (new typed error, code `PIPELINE_DEPENDENCY_ERROR`) when a step's `requires` keys are not provided by any preceding step's `provides`. The error names the offending step and the missing keys. A new `validate_dependencies: bool = True` keyword on `ExecutionStrategy.__init__` lets internal callers (e.g. `Executor.stream`'s post-stream sub-strategy) opt out when assembling derived strategies from an already-validated parent. Both new errors are exported from `apcore`.
- **Cross-language alignment (sync A-001)** — Renamed `CircuitOpenError` (code `CIRCUIT_OPEN`) to canonical `CircuitBreakerOpenError` (code `CIRCUIT_BREAKER_OPEN`) to match TypeScript and Rust SDKs and the protocol spec. The legacy `CircuitOpenError` class is retained as a deprecated subclass alias of `CircuitBreakerOpenError` so existing `except CircuitOpenError:` blocks raising the legacy class continue to work; the legacy class will be removed in a future major release. The wire error code emitted by `CircuitBreakerMiddleware` is now `CIRCUIT_BREAKER_OPEN` for both classes. New `ErrorCodes.CIRCUIT_BREAKER_OPEN` constant added; `ErrorCodes.CIRCUIT_OPEN` retained as a deprecated alias. `CircuitBreakerOpenError` is exported from the top-level `apcore` package.
- **`TaskStore.put` → `save`** (cross-language alignment D-10) — Renamed the canonical write method on the `TaskStore` Protocol. `InMemoryTaskStore.put` is retained as a deprecated shim that delegates to `save` and emits a `DeprecationWarning`; it will be removed in a future minor release. Internal `AsyncTaskManager` calls now route through a `_save` helper that prefers `save` and falls back to `put` for legacy custom stores.
- **`TaskStatus.RETRYING` removed** (cross-language alignment D-12) — During retry backoff, the task status is now `TaskStatus.PENDING` to match the TypeScript and Rust SDKs and the protocol spec. `TaskStatus.RETRYING` remains accessible for one minor release as a deprecated attribute that resolves to `TaskStatus.PENDING` and emits a `DeprecationWarning` on access. The `"retrying"` enum value is no longer present in `TaskStatus.__members__`.
- **`TaskInfo.attempt_number` → `retry_count`** (cross-language alignment D-13) — Renamed the dataclass field. `attempt_number` is retained as a deprecated property (with both getter and setter) that reads/writes `retry_count` and emits a `DeprecationWarning`. It will be removed in a future minor release.
- **`ErrorHistory` eviction is min-heap-based** (PROTOCOL_SPEC Issue #43 §3) — Confirmed the in-place O(log N) min-heap eviction keyed on `last_occurred` with lazy deletion of stale entries from dedup-driven timestamp refreshes. Replaces the prior O(excess × M) linear scan for the global-oldest entry; per-insert eviction cost is bounded regardless of the number of tracked modules.
- `AsyncTaskManager.start_reaper` aligned with TS / Rust D-11 surface — accepts `ttl_seconds` (seconds) and `sweep_interval_ms` (milliseconds) keyword arguments and returns a new `ReaperHandle` (with `stop()` / `is_running()`). The legacy `interval_seconds` / `max_age_seconds` arguments still work but emit `DeprecationWarning`; passing both legacy and new aliases for the same value raises `TypeError`. `ReaperHandle` is exported from `apcore.async_task`.
- **`AsyncTaskManager.start_reaper` default `sweep_interval_ms` aligned to 300_000 (sync alignment, WARNING #5)** — Default sweep cadence changed from 3_600_000 ms (1 hour) to **300_000 ms (5 minutes)**, matching TypeScript and Rust. Callers that relied on the 1-hour default must now pass `sweep_interval_ms=3_600_000` explicitly.

### Fixed

- Async `on_error` middleware now detects awaitable **return values** via `inspect.isawaitable(...)` rather than `inspect.iscoroutinefunction(mw.on_error)` (#42). The previous gate missed `functools.partial` wrappers and decorator-wrapped async handlers (no `__wrapped__`), causing the recovery coroutine to be silently dropped — `isinstance(recovery, dict)` then evaluated against an un-awaited coroutine and the chain aborted. The same fix applies to `execute_before` and `execute_after`. Truly synchronous handlers continue to run through `asyncio.to_thread` so blocking calls (`time.sleep` in `RetryMiddleware`) do not stall the event loop.

### Added — PROTOCOL_SPEC hardening (Issues #32–#45)

- **AsyncTaskManager Evolution** (PROTOCOL_SPEC Issue #34) — Pluggable `TaskStore` protocol with `InMemoryTaskStore` default; custom backends (Redis, SQL) can be injected at construction time. Per-task retry configuration via new `RetryPolicy` dataclass (`max_retries`, `retry_delay_ms`, `backoff_multiplier`, `max_retry_delay_ms`) and `BackoffStrategy` enum; tasks move to `TaskStatus.RETRYING` between attempts and `FAILED` after exhaustion. `AsyncTaskManager.start_reaper(interval_seconds, max_age_seconds)` / `stop_reaper()` — opt-in background task for automatic TTL-based deletion of terminal-state (`COMPLETED`, `FAILED`, `CANCELLED`) tasks. Exports: `TaskStore`, `InMemoryTaskStore`, `RetryPolicy`, `BackoffStrategy`.
- **Observability Hardening** (PROTOCOL_SPEC Issue #43) — Pluggable `ObservabilityStore` protocol with `InMemoryObservabilityStore` default (`apcore.observability.store`). `BatchSpanProcessor` for non-blocking OTEL span export with configurable queue and drop-on-full `spans_dropped` counter (now exported from `apcore.observability.tracing`). O(log N) `ErrorHistory` eviction via min-heap keyed on `last_occurred` plus O(1) fingerprint index replacing prior O(M) ring-buffer scan. `compute_fingerprint()` — SHA-256 content-addressable error deduplication with UUID/timestamp normalization (exported from `apcore.observability.error_history`). `RedactionConfig` in `ContextLogger` for glob `field_patterns` and regex `value_patterns` applied at log time. `PrometheusExporter` HTTP server serving `/metrics` (Prometheus text format), `/healthz` (liveness), and `/readyz` (readiness) endpoints (`apcore.observability.prometheus_exporter`). `MetricsCollector.export_prometheus()` emits `apcore_module_calls_total`, `apcore_module_errors_total`, `apcore_module_duration_seconds`. `UsageCollector.export_prometheus()` emits `apcore_usage_calls_total`, `apcore_usage_error_rate`, `apcore_usage_p50/p95/p99_latency_ms`.
- **System Modules Hardening** (PROTOCOL_SPEC Issue #45) — `overrides_path` parameter for `register_sys_modules()` loads a YAML/JSON override file after base config on startup (via `AuditStore` / `OverridesStore` pattern). Structured audit trail: `AuditEntry`, `AuditStore` protocol, and `InMemoryAuditStore` default record all state-modifying control-module calls with timestamp, action, actor_id, actor_type, trace_id, and before/after change dict (`apcore.sys_modules.audit`). `fail_on_error: bool = False` on `register_sys_modules()` — when `True` raises `SysModuleRegistrationError`; when `False` (default) logs `ERROR` and continues. `path_filter` glob on `system.control.reload_module` for bulk reload in dependency topological order; mutually exclusive with `module_id` (raises `ModuleReloadConflictError` on conflict). New error classes exported from `apcore`: `SysModuleRegistrationError` (code `SYS_MODULE_REGISTRATION_FAILED`), `ModuleReloadConflictError` (code `MODULE_RELOAD_CONFLICT`).
- **Schema System Hardening** (PROTOCOL_SPEC Issue #44) — New `apcore.schema.hardening` module: `content_hash(schema)` returns the SHA-256 of canonical JSON for content-addressable schema deduplication; `validate_schema_dict(schema, data)` uses `Draft202012Validator` to exhaustively evaluate all `anyOf`/`oneOf` branches (no short-circuit), resolve recursive `$ref`, enforce numerical/string constraints, and emit SHOULD-level warnings (not hard errors) on unrecognized format values. Conformance fixtures added: `schema_hardening_union.json`, `schema_hardening_recursive.json`, `schema_hardening_constraints.json`, `schema_hardening_formats.json`, `schema_hardening_cache.json`.
- **Multi-Class Module Discovery** (PROTOCOL_SPEC §2.1.1) — New `apcore.registry.multi_class` module: `@multi_class` decorator opts a class into multi-class per-file scanning; `class_name_to_segment()` derives a snake_case ID segment from a class name (CamelCase → `snake_case`); `discover_multi_class()` scans a file and produces IDs of the form `base_id.class_segment`. Single-class files with one decorated class receive the bare `base_id` (backward-compatible). `ModuleIdConflictError` (code `MODULE_ID_CONFLICT`) raised when two classes in the same file produce identical snake_case segments.
- **Middleware Architecture Hardening** (PROTOCOL_SPEC Issue #42) — `CircuitBreakerMiddleware` tracks per-module consecutive failures in a rolling window; transitions through CLOSED → OPEN → HALF_OPEN state machine. When OPEN, `before()` raises `CircuitOpenError` (code `CIRCUIT_OPEN`) to short-circuit execution entirely. On state changes emits `apcore.circuit.opened` / `apcore.circuit.closed` events. `CircuitState` enum and `CircuitBreakerMiddleware` are exported from `apcore`.
- **Event Management Hardening** (PROTOCOL_SPEC Issue #36) — `CircuitBreakerWrapper` in `apcore.events.circuit_breaker` wraps any `EventSubscriber` with independent circuit-breaker protection (CLOSED/OPEN/HALF_OPEN state machine, configurable `open_threshold`, `recovery_window_ms`); emits `apcore.subscriber.circuit_opened` / `apcore.subscriber.circuit_closed` events via the parent `EventEmitter`.
- **Conformance test suite expansion** — `tests/conformance/test_pipeline_hardening.py` (5 cases: fail-fast, continue-on-ignored-error, replace semantic, `run_until` termination, O(1) lookup), `tests/conformance/test_schema_hardening.py` (35 cases across union, recursive, constraints, formats, cache fixtures), `tests/conformance/test_system_modules_hardening.py` (10 cases: overrides persistence, audit entry extraction, Prometheus metrics, path_filter bulk reload, conflict error, fail_on_error behaviour).

---

## [0.19.0] - 2026-04-19

### Added

- **`DependencyNotFoundError`** (error code `DEPENDENCY_NOT_FOUND`) — raised by `resolve_dependencies` when a module's required dependency is not registered. Aligns Python with PROTOCOL_SPEC §5.15.2, which has always mandated this error code. Details include `module_id` and `dependency_id`. Exported from `apcore`.
- **`DependencyVersionMismatchError`** (error code `DEPENDENCY_VERSION_MISMATCH`) — raised by `resolve_dependencies` when a declared `version` constraint is not satisfied by the registered version of the target module. Details include `module_id`, `dependency_id`, `required`, `actual`. Exported from `apcore`.
- **`TaskLimitExceededError`** (error code `TASK_LIMIT_EXCEEDED`) — raised by `AsyncTaskManager.submit` when the manager is at capacity. Replaces the previous untyped `RuntimeError` and makes the failure dispatchable via `error.code` across language SDKs. Retryable=True.
- **`VersionConstraintError`** (error code `VERSION_CONSTRAINT_INVALID`) — raised by `matches_version_hint` / `VersionedStore` callers on malformed constraint strings (empty, operator-without-operand, non-digit-leading operand such as `"v1.0"` or `"latest"`). Previously, malformed constraints silently degraded to `(0,0,0)` comparisons that always passed.
- **`resolve_dependencies(..., module_versions=...)`** — new optional keyword argument mapping `module_id → version_string`. When provided, declared dependency version constraints are enforced per PROTOCOL_SPEC §5.3. When absent, the `DependencyInfo.version` field is silently ignored (back-compat for callers that do not wire versions through yet). `ModuleRegistry._resolve_load_order` now populates this map from YAML version / class `version` attr / `DEFAULT_MODULE_VERSION` (`"1.0.0"`) fallback, and includes already-registered modules (from `_versioned_modules`) so inter-batch constraints resolve against the live registry's multi-version state — not just the latest-only primary map.
- **Caret (`^`) and tilde (`~`) constraint support** in `matches_version_hint` / `select_best_version` (npm/Cargo semantics): `^1.2.3 → >=1.2.3,<2.0.0`, `^0.2.3 → >=0.2.3,<0.3.0`, `^0.0.3 → >=0.0.3,<0.0.4`, `~1.2.3 → >=1.2.3,<1.3.0`, `~1.2 → >=1.2.0,<1.3.0`, `~1 → >=1.0.0,<2.0.0`.
- **`apcore.registry.registry.DEFAULT_MODULE_VERSION`** constant (`"1.0.0"`) — canonical default applied by every registration path (`register`, `_register_in_order`, `ModuleDescriptor.version` fallback) for modules without an explicit `version=` argument or `version` class/instance attribute.
- **`ExecutorProtocol`** — Protocol describing the minimal async-call surface required by `AsyncTaskManager`. Concrete `Executor` still satisfies it. Decouples the task manager from the concrete executor for testing.
- **`Executor.close()`** plus sync (`__enter__` / `__exit__`) and async (`__aenter__` / `__aexit__`) context-manager support — releases the cached `_sync_loop` deterministically. Long-lived singleton executors can continue to ignore this; short-lived executors (per-request, per-test) should call `close()` or use `with Executor(...) as executor:`.
- **`Registry.get_callback_errors(event=None)`** — public accessor returning the per-event callback-exception count recorded by `_trigger_event`. Ops can watch these counters to spot misbehaving subscribers. Event-callback exceptions remain logged + suppressed so the registry's register/unregister contract stays crash-free.

### Fixed

- **`resolve_dependencies` cycle path accuracy** — `_extract_cycle` previously returned a phantom path (all remaining nodes plus the first one re-appended) when the arbitrarily-picked start node had no outgoing edge inside `remaining`. Rewritten to DFS from each remaining node (sorted) and return a true back-edge cycle `[n0, ..., nk, n0]`. When no back-edge exists (e.g., Kahn's sort stalled on a non-cycle blocker), the resolver now raises `ModuleLoadError` naming the blocked modules instead of emitting a `CircularDependencyError` with a phantom `sorted(remaining)` path.
- **`Registry._register_in_order` populates `_versioned_modules` and `_versioned_meta`** — discover()-path modules were previously written only to the latest-only `_modules` map, leaving `Registry.get(id, version_hint=…)` unable to resolve them (version-hint queries route through the versioned store first). All registration paths now produce equivalent state.
- **Default version alignment** — `Registry.register()` with no `version=` now falls back to `DEFAULT_MODULE_VERSION` (`"1.0.0"`), matching `_resolve_load_order` and `ModuleDescriptor.version`. Previously `register()` used `"0.0.0"` while `_resolve_load_order` used `"1.0.0"` — the same module routed through the two paths got different effective versions.
- **Non-string versions warn-and-coerce** — a module with `version: 1` in YAML (integer) or a numeric class attribute is no longer silently dropped from constraint enforcement. The resolver logs a WARN naming the module and coerces via `str(...)`.
- **Malformed version constraints raise `VersionConstraintError`** — previously `">=not_a_version"`, `"v1.0"`, and `"~"` alone passed `_CONSTRAINT_RE` and degraded to `(0,0,0)` comparisons that always returned True. The constraint regex now requires a digit-leading operand, and `_check_single_constraint` raises explicitly on malformed input.
- **Scanner confines `follow_symlinks=True` to the extension root** — symlinks whose real path escapes the root (e.g., into `/etc`, `$HOME`, a sibling project) are now refused with a WARN log. The previous code would walk any target once per real-path visit. Combined with a WARN log on the first discover() when `follow_symlinks=True` is configured, this makes the trust boundary visible.
- **`Executor._run_in_new_thread` bounds `thread.join()` by `_global_timeout`** — a dead-locked coroutine can no longer indefinitely hang the sync caller. On timeout the daemon thread is left running (process exit stays clean) and the caller receives a `ModuleTimeoutError`.
- **`Executor.stream` post-stream validation failures log at WARNING (not DEBUG)** — unvalidated output that already reached the consumer is worth investigating even if it can't be un-sent. Previously such failures were invisible in default production observability.
- **`AsyncTaskManager.cancel` narrows its `except`** — catches `asyncio.CancelledError` specifically (the expected cancellation path). Unexpected exceptions from the cancelled task are now logged at WARNING with a stack trace instead of being silently swallowed alongside CancelledError.
- **`errors.py __all__`** — adds `DependencyNotFoundError`, `DependencyVersionMismatchError`, `TaskLimitExceededError`, `VersionConstraintError`. `from apcore.errors import *` now picks them up.
- **Inline `__import__('time')` / `__import__('os')`** in `_ModuleChangeHandler` replaced with top-level imports. No behavior change.

### Changed (BREAKING)

- **Missing required dependencies now raise `DependencyNotFoundError` (code `DEPENDENCY_NOT_FOUND`) instead of `ModuleLoadError` (code `MODULE_LOAD_ERROR`).** Brings Python into compliance with PROTOCOL_SPEC §5.15.2 which has always mandated `DEPENDENCY_NOT_FOUND`. Upgrade path: catch `DependencyNotFoundError` specifically, or catch the `ModuleError` base class for any dependency-related failure. The error-code-based dispatch (via `ErrorCodes.DEPENDENCY_NOT_FOUND`) also works and is recommended for cross-language consumers.
- **`AsyncTaskManager.submit` now raises `TaskLimitExceededError`, not `RuntimeError`.** Callers catching `RuntimeError("Task limit reached …")` must migrate to `except TaskLimitExceededError:` (or catch `ModuleError` for any task-manager failure).
- **`Registry.register()` default version is now `"1.0.0"` instead of `"0.0.0"`** for modules registered without an explicit `version=` argument and without a class/instance `version` attribute. Callers that relied on `"0.0.0"` as an "unset" marker must pass `version="0.0.0"` explicitly. `ModuleDescriptor.version` has always defaulted to `"1.0.0"` — this aligns the internal state with the externally-visible view.
- **Malformed version constraint strings now raise `VersionConstraintError`** instead of silently evaluating to False (which, via the degraded-parse-semver path, effectively became True). Callers that relied on silent no-op behavior must wrap in try/except or sanitize upstream.
- **Dependency upper bounds** — `pydantic`, `pyyaml`, and `jsonschema` are now pinned to `<3`, `<7`, `<5` respectively. Prevents silent breakage from downstream major releases; raise the cap deliberately after a compatibility check.

### Added

- **`DECLARATIVE_CONFIG_SPEC.md` v1.0** — Canonical spec for bindings, pipeline config, and entry-point YAML. Lives in `apcore/docs/spec/`. Defines cross-SDK YAML syntax parity, error model, configurable policy limits, and auto_schema semantics. All three SDKs (Python, TypeScript, Rust) now conform to this unified specification.
- **`auto_schema: true | permissive | strict`** — Explicit auto-schema mode selection. `strict` mode enforces OpenAI/Anthropic-compatible schemas (`additionalProperties: false`, all properties required, restricted type set). Incompatible features produce `BindingStrictSchemaIncompatibleError` at parse time.
- **`auto_schema` as implicit default** — When no schema mode is specified in a binding entry, auto-schema inference is attempted automatically. This formalizes Python's existing behavior as cross-SDK spec.
- **Schema mode conflict detection** — Specifying multiple schema modes (e.g., `auto_schema` + `input_schema`) now produces `BindingSchemaModeConflictError` at parse time.
- **`spec_version` field** in binding YAML files. Defaults to `"1.0"` with deprecation warning when absent (mandatory in spec 1.1).
- **New canonical error classes**: `BindingSchemaInferenceFailedError`, `BindingSchemaModeConflictError`, `BindingStrictSchemaIncompatibleError`, `BindingPolicyViolationError`. Each includes file path, line, module ID, and spec section reference.
- **`display` field** support on `FunctionModule` and binding YAML entries. Surface overlay for CLI/MCP/A2A presentation per `binding.schema.json#/DisplayOverlay`.
- **`documentation`, `annotations`, `metadata`** fields now round-trip through `BindingLoader` → `FunctionModule`.
- **Cross-SDK conformance fixtures** in `apcore/conformance/fixtures/`: `binding_yaml_canonical.yaml` (YAML parse parity), `binding_errors.json` (error message parity).

### Changed

- **`Context.create(trace_parent=...)`** — strict input validation per PROTOCOL_SPEC §10.5. trace_ids that are all-zero or all-f (W3C-invalid) now trigger regeneration + WARN log, matching the other SDKs. No auto-normalization (dashed-UUID stripping or case folding) is performed at `Context.create`; such normalization is the caller's ContextFactory responsibility. Previously valid 32-hex inputs remain accepted verbatim. Covered by new conformance fixture `context_trace_parent.json`.
- **`BindingSchemaMissingError`** is now a deprecated alias for `BindingSchemaInferenceFailedError`. Error code changed from `BINDING_SCHEMA_MISSING` to `BINDING_SCHEMA_INFERENCE_FAILED`. Existing `except BindingSchemaMissingError` catch clauses continue to work.
- **`BindingLoader._create_module_from_binding`** rewritten to implement DECLARATIVE_CONFIG_SPEC.md §3.4 schema resolution: explicit schemas > schema_ref > explicit auto > implicit auto default. Replaces the previous if/elif chain.
- **`FunctionModule.__init__`** now accepts `display: dict | None` parameter.
- **`Annotations` in `binding.schema.json`** expanded from 5 to 12 fields (added `streaming`, `cacheable`, `cache_ttl`, `cache_key_fields`, `paginated`, `pagination_style`, `extra`) to align with `module-meta.schema.json`.

### Removed

- **Implicit auto_schema as Python-specific behavior** — this is now a cross-SDK spec-defined behavior, not a Python-only quirk.

## [0.18.0] - 2026-04-15

### Added

- **`APCore` unified client class** (`apcore.client.APCore`) — High-level facade over `Registry` + `Executor` providing a single entry point for all module operations. Constructor accepts optional `registry`, `executor`, `config`, and `metrics_collector` (auto-created when `sys_modules.enabled`). Public API surface:
  - **Module management**: `module()` decorator, `register()`, `list_modules(tags=, prefix=)`, `discover()`, `describe()`
  - **Execution**: `call()`, `call_async()`, `stream()`, `validate()` — all accept `version_hint` for semver negotiation (A14)
  - **Middleware**: `use()`, `use_before()`, `use_after()`, `remove()` — `use`/`use_before`/`use_after` return `self` for chaining
  - **Events**: `events` property, `on(event_type, handler)`, `off(subscriber)` — requires `sys_modules.events.enabled` in config
  - **Module toggle**: `disable(module_id, reason=)`, `enable(module_id, reason=)` — wrappers around `system.control.toggle_feature`
  - Cross-language parity: matches apcore-typescript `APCore` class and apcore-rust `APCore` struct public API surface
- **Package-level global convenience functions** (`apcore.call`, `apcore.call_async`, `apcore.stream`, `apcore.validate`, `apcore.register`, `apcore.describe`, `apcore.use`, `apcore.use_before`, `apcore.use_after`, `apcore.remove`, `apcore.discover`, `apcore.list_modules`, `apcore.on`, `apcore.off`, `apcore.disable`, `apcore.enable`, `apcore.module`) — delegate to a module-level `_default_client = APCore()` instance for zero-setup usage (`import apcore; apcore.call("math.add", {"a": 1, "b": 2})`). Python-specific ergonomic; apcore-typescript and apcore-rust require explicit client construction.
- **Pipeline preset builders re-exported at package root** — `build_standard_strategy`, `build_internal_strategy`, `build_testing_strategy`, `build_performance_strategy`, `build_minimal_strategy` are now importable directly from `apcore`. These functions existed in `apcore.builtin_steps` but were not previously in `apcore.__all__`. Parity with apcore-typescript (`buildXxxStrategy`) and apcore-rust (`build_xxx_strategy` at the crate root).
- **`TestRegisterInternalValidation`** test class in `tests/registry/test_registry.py` (6 parity tests covering empty rejection, pattern rejection, over-length rejection, reserved-word bypass, duplicate rejection, accept-at-max-length) plus `test_pipeline_preset_builders_*` in `tests/test_public_api.py`.
- **`Registry.export_schema()`** — New method returning a module's schema definition as a dict, with optional `strict=True` for OpenAI/Anthropic strict schema compliance (`additionalProperties: false`). Aligned with apcore-rust `Registry::export_schema()`.

### Changed

- **`Executor.describe_pipeline()` now returns `StrategyInfo` instead of `str`** — Provides structured access to `step_count`, `step_names`, `name`, and `description`. `str(result)` produces the original formatted string via `StrategyInfo.__str__`. Aligned with apcore-typescript `describePipeline() -> StrategyInfo`.

- **`Executor.call()` and `Executor.call_with_trace()` are now thin sync wrappers** over `call_async()` and `call_async_with_trace()` via a shared `_run_async_in_sync(coro, module_id)` dispatcher. The cached-event-loop / thread-bridge logic that was previously inlined in three places lives in one helper. Sync semantics preserved: nested calls inside a running event loop still route through a background thread.
- **`Executor.call_async_with_trace()` now uses the unified A11 error recovery path** (`_translate_abort` + `_recover_from_call_error` + middleware `on_error` chain). Previously it called `engine.run` raw and let `PipelineAbortError` leak; behavior now matches `call_async`. When a middleware `on_error` recovers, the recovery dict is returned alongside a sentinel `PipelineTrace` (per-step trace detail is unavailable in the recovery branch — use `call_async` if you don't need the trace, or attach a tracing middleware).
- **`BuiltinApprovalGate` now self-contains the full approval flow.** Audit-log emission, span-event emission, and full status→error mapping (including `timeout` and unknown-status warning) used to live on private methods of `Executor`, with `BuiltinApprovalGate` reaching into them via `hasattr(executor, '_check_approval_async')`. The reach-into-private cheat is gone; `BuiltinApprovalGate` does everything itself. The `executor=` parameter on `BuiltinApprovalGate.__init__` is removed (was unused after consolidation). **Approval audit logs are now emitted from logger `apcore.builtin_steps`** (was `apcore.executor`) — update any log filters accordingly.
- **`BuiltinACLCheck` and `BuiltinApprovalGate` now expose public `set_acl()` / `set_handler()` setters.** `Executor.set_acl` and `set_approval_handler` use the public setters instead of poking step `._acl` / `._handler`. Custom user-supplied ACL or approval steps without these setters are silently skipped — re-register the strategy if you need to swap providers on a custom step.
- **`Registry._discover_default()` decomposed** from a 153-line god method into a 23-line orchestrator + 9 named stage helpers (`_scan_params`, `_scan_roots`, `_apply_id_map_overrides`, `_load_all_metadata`, `_resolve_all_entry_points`, `_validate_all`, `_resolve_load_order`, `_filter_id_conflicts`, `_register_in_order`, `_invoke_on_load`). Pure refactor — no behavior change. Mirrors the structure of `apcore-typescript`'s `_discoverDefault`.
- **`ACL.check()` and `ACL.async_check()` consolidated** via shared `_snapshot()` and `_finalize_check()` helpers. Audit-entry construction and debug-logging now live in exactly one place (was duplicated four times). Fixed `_matches_rule_async` to call `_match_patterns()` instead of inlining a variant that bypassed compound operators (`$or`/`$not`).
- **ACL singular condition handler aliases removed** (`identity_type`, `role`, `call_depth`). Spec §6.1 only defines the plural forms (`identity_types`, `roles`, `max_call_depth`); the singular aliases were a python-only divergence.
- **`builtin_steps.py` strategy builders no longer use `object.__setattr__`** for the `name` field. `ExecutionStrategy` was never a frozen dataclass — `s.name = X` always worked. Cargo-cult code removed.
- **`ErrorCodes` class `__setattr__`/`__delattr__` traps dropped.** The traps only fired on *instance* attribute mutation (`ErrorCodes().X = ...`), never on *class* attribute mutation (`ErrorCodes.X = ...`) which is how `ErrorCodes` is actually used. Cargo-cult immutability that gave a false sense of protection. Aligned with apcore-typescript (`Object.freeze`) and apcore-rust (enum).
- **Pydantic v1/v2/dataclass/constructor fallback cascade collapsed in `config.py`.** Previously maintained a 4-branch compatibility chain for Pydantic v1 → v2 migration. The project requires Pydantic v2 since 0.16.0; dead branches removed.
- **`Registry._handle_file_change()` refactored** — replaced fragile `dir(mod)` module-attribute discovery with explicit registry lookup. More predictable behavior on hot-reload events.
- **`Registry.register()` / `register_internal()` now populate `_module_meta`** at registration time, not lazily at first `get_definition()` call. Consistent with `_discover_default` path.
- **31 pre-existing pyright type errors resolved** across `executor.py`, `config.py`, `registry.py`, `builtin_steps.py`, and `acl.py`. No runtime behavior change; strict type-checking now passes cleanly.
- **`MAX_MODULE_ID_LENGTH` raised from 128 to 192** (`apcore.registry.registry`). Tracks PROTOCOL_SPEC §2.7 EBNF constraint #1 update — accommodates Java/.NET deep-namespace FQN-derived IDs while remaining filesystem-safe (`192 + len('.binding.yaml') = 205 < 255`-byte filename limit on ext4/xfs/NTFS/APFS/btrfs). Module IDs valid before this change remain valid; only the upper bound moved. **Forward-compatible relaxation:** older 0.17.x/0.18.x readers will reject IDs in the 129–192 range emitted by this version.
- **`Registry.register()` and `Registry.register_internal()` now share a `_validate_module_id()` helper** that runs validation in canonical order (empty → EBNF pattern → length → reserved word per-segment). The reserved-word check is the only step `register_internal()` skips (so sys modules can use the `system.*` prefix); empty/pattern/length/duplicate now apply uniformly. Aligned cross-language with apcore-typescript and apcore-rust.
- **`register_internal()` now enforces empty / pattern / length / duplicate checks.** Previously bypassed every validation step. Production callers (`apcore.sys_modules.*`) all use canonical-shape IDs so no in-tree caller is broken; external adapters that used `register_internal` as a generic escape hatch should review.
- **Duplicate registration error message canonicalized** to `"Module ID '<id>' is already registered"` (was `"Module already exists: <id>"` for `register_internal`). Both `register()` and `register_internal()` now emit the same message via the shared error path. Aligned with apcore-rust and apcore-typescript byte-for-byte.

### Removed

- **`FeatureNotImplementedError` and `DependencyNotFoundError`** — zero raise-sites across the codebase; `grep -rn` confirmed no production or test code instantiated either class. Error codes `GENERAL_NOT_IMPLEMENTED` and `DEPENDENCY_NOT_FOUND` remain in `ErrorCodes` for use via the generic `ModuleError` constructor. Aligned with apcore-typescript (commit `01ea84d`).

### Removed (BREAKING)

- **`Context.to_dict()` and `Context.from_dict()`** — superseded by the spec-compliant `Context.serialize()` and `Context.deserialize()` (shipped in v0.16.0). The two pairs were silently inconsistent (`to_dict` always emitted `redacted_inputs` even when `None` while `serialize` omitted it; `serialize` included `_context_version: 1`, `to_dict` did not), so mixing them produced divergent dicts. Migration:
  - `ctx.to_dict()` → `ctx.serialize()`
  - `Context.from_dict(data, executor=x)` → `Context.deserialize(data); ctx.executor = x` (the `executor=` parameter is removed; reassign directly on the returned `Context`, which is non-frozen)
- **Private `Executor` approval helpers removed** as part of the `BuiltinApprovalGate` consolidation. No public API impact unless your code reached into `Executor._check_approval_async`, `_build_approval_request`, `_handle_approval_result`, `_emit_approval_event`, `_needs_approval`, `_check_approval_sync`, the timeout-aware `_run_async_in_sync` (the new same-named method has a different `(coro, module_id)` signature), `_async_cache`, or `_async_cache_lock`.
- **Legacy event aliases removed.** Per the §9.16 naming convention shipped in v0.15, the dual-emission transition period for `module_health_changed` and `config_changed` ended in this release (the original removal deadline was v0.16.0). Listeners that subscribed to these legacy names will no longer receive events. Migrate subscriptions to the canonical names:
  - `module_health_changed` → `apcore.module.toggled` (from `system.control.toggle_feature`) **or** `apcore.health.recovered` (from `PlatformNotifyMiddleware`)
  - `config_changed` → `apcore.config.updated` (from `system.control.update_config`) **or** `apcore.module.reloaded` (from `system.control.reload_module`)
- **Renamed private method `_emit_config_changed` → `_emit_module_reloaded`** in `system.control.reload_module` to reflect the canonical event it emits. Private API, no public-surface impact.

### Fixed

- **Global convenience functions `call()`, `call_async()`, `stream()` missing `version_hint` parameter** — These `apcore/__init__.py` wrappers previously forwarded only `(module_id, inputs, context)` to the `APCore` client, silently dropping `version_hint`. Users calling `apcore.call(..., version_hint=">=1.0.0")` would have had the hint ignored. Now all three wrappers accept and forward `version_hint: str | None = None`, matching the `APCore` class signature and cross-language SDKs.
- **Spec §4.13 annotation merge — YAML annotations are no longer silently dropped at registration.** Two coupled bugs were repaired: (1) `registry/metadata.py:merge_module_metadata` was doing whole-replacement of the `annotations` field instead of the field-level merge mandated by §4.13 ("If YAML only defines `readonly: true`, other fields **must** retain values from code or defaults."), and (2) `registry/registry.py:get_definition` was ignoring even that broken merge result and reading directly from the module's class attribute. The fix wires the previously-unwired `apcore.schema.annotations.merge_annotations` and `merge_examples` (which were defined and unit-tested but never called from production) into the registry pipeline, and updates `get_definition` to consume the merged metadata. **User-observable behavior change:** modules that supplied `annotations:` in their `*_meta.yaml` companion files were previously seeing those annotations silently ignored. Those annotations will now be honored. Modules that relied on the broken behavior should audit their `*_meta.yaml`. Adds 5 regression tests covering field-level merge, YAML-only, neither-defined, examples override, and an end-to-end `discover() → get_definition()` round-trip.
- **`ModuleAnnotations.from_dict` precedence inversion** — Per PROTOCOL_SPEC §4.4.1 rule 7, when the same key appears both in a nested `extra` object and as a top-level overflow key, the **nested value now wins** (previously the top-level overflow would silently overwrite it). Behavior change is observable only in the pathological case where an input contains both forms of the same key — no conformant producer emits this. Top-level overflow keys are still tolerated and merged into `extra` for backward compatibility.

## [0.17.1] - 2026-04-06

### Added

- **`build_minimal_strategy()`** — 4-step pipeline (context → lookup → execute → return) for pre-validated internal hot paths. Registered as `"minimal"` in Executor preset builders.
- **`requires` / `provides` on `BaseStep`** — Optional advisory fields declaring step dependencies. `ExecutionStrategy` validates dependency chains at construction and insertion, emitting warnings for unmet `requires`.

### Fixed

- **`"minimal"` added to preset builders** — `Executor(strategy="minimal")` now works. Previously missing from `_resolve_strategy_name()` preset dict.
- **Executor docstrings updated** — Constructor and `_resolve_strategy_name` docstrings now list all 5 presets (was missing `"minimal"`).

---

## [0.17.0] - 2026-04-05

### Added

- **Step Metadata**: Four declarative fields on `BaseStep`: `match_modules` (glob patterns for selective execution), `ignore_errors` (fault-tolerant steps), `pure` (safe for `validate()` dry-run), `timeout_ms` (per-step timeout).
- **YAML Pipeline Configuration**: `register_step_type()`, `unregister_step_type()`, `registered_step_types()`, `build_strategy_from_config()` — configure pipeline steps via `apcore.yaml` at startup.
- **PipelineContext fields**: `dry_run`, `version_hint`, `executed_middlewares` for pipeline-aware execution.
- **StepTrace**: `skip_reason` field for understanding why steps were skipped ("no_match", "dry_run", "error_ignored").

### Changed

- **Step order**: `middleware_before` now runs BEFORE `input_validation` (was after). Middleware input transforms are now validated by the schema check.
- **Executor delegation**: `call()`, `call_async()`, `validate()`, and `stream()` fully delegate to `PipelineEngine.run()`. Removed ~300 lines of duplicated inline step code.
- **Renamed**: `safety_check` step → `call_chain_guard` (accurately describes call-chain depth/cycle/repeat checking).
- **Renamed**: `BuiltinSafetyCheck` class → `BuiltinCallChainGuard`.

### Fixed

- Middleware input transforms were never re-validated against the schema (now validated after middleware runs).
- `validate()` was hardcoded to 7 inline checks; now uses `dry_run=True` pipeline mode — user-added `pure=True` steps automatically participate.

---

## [0.16.0] - 2026-04-05

### Added

- **Config Bus**: `env_style` (auto/nested/flat), `max_depth`, `env_prefix` auto-derivation, `env_map` (namespace + global), `Config.env_map()`, `CONFIG_ENV_MAP_CONFLICT` error.
- **Context**: `ContextKey[T]` typed accessor with `get()`/`set()`/`delete()`/`exists()`/`scoped()`. Built-in key constants (`TRACING_SPANS`, `METRICS_STARTS`, etc.). `Context.serialize()`/`deserialize()` with `_context_version: 1`.
- **Annotations**: `extra: dict[str, Any]` extension field on `ModuleAnnotations`. `pagination_style` changed from `Literal` to `str`. `DEFAULT_ANNOTATIONS` constant. `from_dict()` classmethod with unknown key capture.
- **ACL**: `SyncACLConditionHandler` / `AsyncACLConditionHandler` protocols. `ACL.register_condition()`. `$or`/`$not` compound operators. `async_check()` method. Fail-closed for unknown conditions.
- **Pipeline**: `Step` protocol, `BaseStep` ABC, `StepResult`, `PipelineContext`, `PipelineTrace`, `ExecutionStrategy`, `PipelineEngine`. 11 `BuiltinStep` classes. Preset strategies (standard/internal/testing/performance). `Executor.strategy` parameter. `call_with_trace()`/`call_async_with_trace()`. `register_strategy()`/`list_strategies()`/`describe_pipeline()`.

### Changed

- Middleware data keys migrated from legacy names (`_metrics_starts` etc.) to `_apcore.mw.*` convention using typed `ContextKey`.

---

## [0.15.1] - 2026-03-31

### Changed

- **Env prefix convention simplified** — Removed the `^APCORE_[A-Z0-9]` reservation rule from `Config._validate_env_prefix()`. Sub-packages now use single-underscore prefixes (`APCORE_MCP`, `APCORE_OBSERVABILITY`, `APCORE_SYS`) instead of the double-underscore form. Only the exact `APCORE` prefix is reserved for the core namespace.
- Built-in namespace env prefixes: `APCORE__OBSERVABILITY` → `APCORE_OBSERVABILITY`, `APCORE__SYS` → `APCORE_SYS`.

---

## [0.15.0] - 2026-03-30

### Added

#### Config Bus Architecture (§9.4–§9.14)
- **`Config.register_namespace(name, schema=None, env_prefix=None, defaults=None)`** — Class-level namespace registration. Any package can claim a named config subtree with optional JSON Schema validation, env prefix, and default values. Global registry is shared across all `Config` instances. Late registration is allowed; call `config.reload()` afterward to apply defaults and env overrides.
- **`config.get("namespace.key.path")`** — Dot-path access with namespace resolution. First segment resolves to a registered namespace; remaining segments traverse the subtree.
- **`config.namespace(name)`** — Returns the full config subtree for a registered namespace as a dict.
- **`config.bind(ns, type)` / `config.get_typed(path, type)`** — Typed namespace access; `bind` returns a view of the namespace deserialized into `type`, `get_typed` deserializes a single dot-path value.
- **`config.mount(namespace, from_file=...|from_dict=...)`** — Attach external config sources to a namespace without a unified YAML file. Primary integration path for third-party packages with existing config systems.
- **`Config.registered_namespaces()`** — Class-level introspection; returns names of all registered namespaces.
- **Unified YAML with namespace partitioning** — Single YAML file with namespace-keyed top-level sections. Automatic mode detection: legacy mode (no `apcore:` key, fully backward compatible) vs. namespace mode (`apcore:` key present). `_config` is a reserved meta-namespace (`strict`, `allow_unknown`).
- **Per-namespace env override with longest-prefix-match dispatch** — Each namespace declares its own `env_prefix`. Apcore sub-packages use `APCORE_` prefixed names (e.g., `APCORE_OBSERVABILITY`, `APCORE_SYS`); the longest-prefix-match dispatch algorithm resolves any ambiguity with the core `APCORE` prefix.
- **Hot-reload namespace support** — `config.reload()` re-reads YAML, re-detects mode, re-applies namespace defaults and env overrides, re-validates, and re-reads mounted files.
- **New error codes** — `CONFIG_NAMESPACE_DUPLICATE`, `CONFIG_NAMESPACE_RESERVED`, `CONFIG_ENV_PREFIX_CONFLICT`, `CONFIG_MOUNT_ERROR`, `CONFIG_BIND_ERROR`

#### Error Formatter Registry (§8.8)
- **`ErrorFormatter` protocol** — Interface for adapter-specific error formatters. Implementations transform `ModuleError` into the surface-specific wire format (e.g., MCP camelCase, JSON-RPC code mapping).
- **`ErrorFormatterRegistry`** — Shared registry for surface-specific formatters:
  - `ErrorFormatterRegistry.register(surface, formatter)` — register a formatter for a named surface
  - `ErrorFormatterRegistry.get(surface)` — retrieve a registered formatter
  - `ErrorFormatterRegistry.format(surface, error)` — format an error, falling back to `error.to_dict()` if no formatter is registered for that surface
- **New error code** — `ERROR_FORMATTER_DUPLICATE`

#### Built-in Namespace Registrations (§9.15)
- **`observability` namespace** (`APCORE_OBSERVABILITY` env prefix) — apcore pre-registers this namespace, promoting the existing `apcore.observability.*` flat config keys (tracing, metrics, logging, error_history, platform_notify) into a named subtree. Adapter packages (apcore-mcp, apcore-a2a, apcore-cli) should read from this namespace rather than independent logging defaults.
- **`sys_modules` namespace** (`APCORE_SYS` env prefix) — apcore pre-registers this namespace, promoting the existing `apcore.sys_modules.*` flat keys into a named subtree. `register_sys_modules()` prefers `config.namespace("sys_modules")` in namespace mode with `config.get("sys_modules.*")` legacy fallback. Both registrations are 1:1 migrations of existing keys; there are no breaking changes.

#### Event Type Naming Convention and Collision Fix (§9.16)
- **Canonical event names** — Two confirmed event type collisions in apcore-python are resolved:
  - `"module_health_changed"` (previously used for both enable/disable toggles and error-rate recovery) split into `apcore.module.toggled` (toggle on/off) and `apcore.health.recovered` (error rate recovery)
  - `"config_changed"` (previously used for both key updates and module reload) split into `apcore.config.updated` (runtime key update via `system.control.update_config`) and `apcore.module.reloaded` (hot-reload via `system.control.reload_module`)
- **Naming convention** — `apcore.*` is reserved for core framework events. Adapter packages use their own prefix: `apcore-mcp.*`, `apcore-a2a.*`, `apcore-cli.*`.
- **Transition aliases** — All four legacy short-form names (`module_health_changed`, `config_changed`) continue to be emitted alongside the canonical names during the transition period.

---

## [0.14.0] - 2026-03-24

### Added
- **Middleware priority** — `Middleware` base class now accepts `priority: int` (0-1000, default 0). Higher priority executes first; equal priority preserves registration order. `BeforeMiddleware` and `AfterMiddleware` adapters also accept `priority`.
- **Priority range validation** — `ValueError` raised for priority values outside 0-1000

### Breaking Changes
- Middleware default priority changed from `0` to `100` per PROTOCOL_SPEC §11.2. Middleware without explicit priority will now execute before priority-0 middleware.


## [0.13.2] - 2026-03-22

### Changed
- Rebrand: aipartnerup → aiperceivable

## [0.13.1] - 2026-03-19

### Added
- **Dict schema support** — Modules can now define `input_schema` / `output_schema` as plain JSON Schema dicts instead of Pydantic model classes. A `_DictSchemaAdapter` transparently wraps dict schemas at registration time so all internal code paths (executor, schema exporter, `get_definition`) work without changes.

### Fixed
- **`get_definition()` crash on dict schemas** — Previously called `.model_json_schema()` on dict objects, causing `AttributeError`
- **Executor crash on dict schemas** — `call()`, `call_async()`, and `stream()` all called `.model_validate()` on dict objects

### Improved
- **File header docstrings** — Enhanced docstrings for `errors.py`, `executor.py`, and `version.py`

---

## [0.13.0] - 2026-03-12

### Added
- **Caching/pagination annotations** — `ModuleAnnotations` gains 5 new fields: `cacheable`, `cache_ttl`, `cache_key_fields`, `paginated`, `pagination_style` (all optional with defaults, backward compatible)
- **`pagination_style` Literal type** — Typed as `Literal["cursor", "offset", "page"]` instead of free-form `str`
- **`sunset_date`** — New field on `ModuleDescriptor` for module deprecation lifecycle (ISO 8601 date)
- **`on_suspend()` / `on_resume()` lifecycle hooks** — Duck-typed optional hooks for state preservation during hot-reload; integrated into `ReloadModuleModule` and registry watchdog
- **MCP `_meta` export** — Schema exporter includes `cacheable`, `cacheTtl`, `cacheKeyFields`, `paginated`, `paginationStyle` in `_meta` sub-dict
- **Suspend/resume tests** — `tests/test_suspend_resume.py` covering state transfer, backward compatibility, error handling

### Changed
- **Rebranded** — "module development framework" → "module standard" in pyproject.toml, `__init__.py`, README, and internal docstrings
- **`Module` Protocol** — `on_suspend`/`on_resume` deliberately kept OUT of Protocol (duck-typed via `hasattr`/`callable`)

---

## [0.12.0] - 2026-03-10

### Changed
- **`ExecutionCancelledError`** now extends `ModuleError` (was bare `Exception`) with error code `EXECUTION_CANCELLED`, aligning with PROTOCOL_SPEC §8.7 error hierarchy
- **`ErrorCodes`** — Added `EXECUTION_CANCELLED` constant

---

## [0.11.0] - 2026-03-08

### Added
- **Full lifecycle integration tests** (`tests/integration/test_full_lifecycle.py`) — 8 tests covering the complete 11-step pipeline with all gates (ACL + Approval + Middleware + Schema validation) enabled simultaneously, nested module calls, shared `context.data`, error propagation, and ACL conditions.

#### System Modules — AI Bidirectional Introspection
Built-in `system.*` modules that allow AI agents to query, monitor

- **`system.health.summary`** — Aggregate health status across all registered modules (healthy/degraded/unhealthy classification based on error rate thresholds).
- **`system.health.module`** — Per-module health detail including recent errors from `ErrorHistory`.
- **`system.manifest.module`** — Single module introspection (schema, annotations, tags, source path).
- **`system.manifest.full`** — Full registry manifest with filtering by tags/prefix.
- **`system.usage.summary`** — Usage statistics across all modules (call counts, error rates, avg latency).
- **`system.usage.module`** — Per-module usage detail with hourly trend data.
- **`system.control.update_config`** — Runtime config hot-patching with constraint validation.
- **`system.control.reload_module`** — Hot-reload a module from disk without restart.
- **`system.control.toggle_feature`** — Enable/disable modules at runtime with reason tracking.
- **`register_sys_modules()`** — Auto-registration wiring for all system modules.

#### Observability
- **`ErrorHistory`** — Ring buffer tracking recent errors with deduplication and per-module querying.
- **`ErrorHistoryMiddleware`** — Middleware that records `ModuleError` details into `ErrorHistory`.
- **`UsageCollector`** — Per-module call counting, latency histograms, and hourly bucketed trend data.
- **`PlatformNotifyMiddleware`** — Threshold-based sensor that emits events on error rate spikes.

#### Event System
- **`EventEmitter`** — Global event bus with async subscriber dispatch and thread-pool execution.
- **`EventSubscriber`** protocol — Interface for event consumers.
- **`ApCoreEvent`** — Frozen dataclass for typed events (module lifecycle, errors, config changes).
- **`WebhookSubscriber`** — HTTP POST event delivery with retry.
- **`A2ASubscriber`** — Agent-to-Agent protocol event bridge.

#### APCore Unified Client
- **`APCore.on()`** / **`APCore.off()`** — Event subscription management via the unified client.
- **`APCore.disable()`** / **`APCore.enable()`** — Module toggle control via the unified client.
- **`APCore.discover()`** / **`APCore.list_modules()`** — Discovery and listing via the unified client.

#### Public API Exports
- **`ModuleDisabledError`** — Error class for `MODULE_DISABLED` code, raised when a disabled module is called.
- **`ReloadFailedError`** — Error class for `RELOAD_FAILED` code (retryable).
- **`SchemaStrategy`** — Enum for schema resolution strategy (`yaml_first`, `native_first`, `yaml_only`).
- **`ExportProfile`** — Enum for schema export profiles (`mcp`, `openai`, `anthropic`, `generic`).

#### Registry
- **Module toggle** — APCore client now supports `disable()`/`enable()` for module toggling via `system.control.toggle_feature`, with `ModuleDisabledError` enforcement and event emission.
- **Version negotiation** — `negotiate_version()` for SDK/module version compatibility checking.


### Changed
- **`WebhookSubscriber` / `A2ASubscriber`** now require optional dependency `aiohttp`. Install with `pip install apcore[events]`. Core SDK no longer fails to import when `aiohttp` is not installed.

### Fixed
- **`aiohttp` hard import** in `events/subscribers.py` broke core SDK import when `aiohttp` was not installed. Changed to `try/except ImportError` guard with clear error message at runtime.
- **`A2ASubscriber.on_event`** `ImportError` for missing `aiohttp` was silently swallowed by the broad `except Exception` block. Moved guard before the `try` block to surface the error correctly.
- README Access Control example now includes required `Executor` and `Registry` imports.
- `pyproject.toml` repository/issues/changelog URLs now point to `apcore-python` (was incorrectly pointing to `apcore`).
- CHANGELOG `[0.7.1]` compare link added (was missing from link references).

---

## [0.10.0] - 2026-03-07

### Added

#### APCore Unified Client
- **`APCore.stream()`** — Stream module output chunk by chunk via the unified client.
- **`APCore.validate()`** — Non-destructive preflight check via the unified client.
- **`APCore.describe()`** — Get module description info (for AI/LLM use).
- **`APCore.use_before()`** — Add before function middleware via the unified client.
- **`APCore.use_after()`** — Add after function middleware via the unified client.
- **`APCore.remove()`** — Remove middleware by identity via the unified client.

#### Global Entry Points (`apcore.*`)
- **`apcore.stream()`** — Global convenience for streaming module calls.
- **`apcore.validate()`** — Global convenience for preflight validation.
- **`apcore.register()`** — Global convenience for direct module registration.
- **`apcore.describe()`** — Global convenience for module description.
- **`apcore.use()`** — Global convenience for adding middleware.
- **`apcore.use_before()`** — Global convenience for adding before middleware.
- **`apcore.use_after()`** — Global convenience for adding after middleware.
- **`apcore.remove()`** — Global convenience for removing middleware.

#### Error Hierarchy
- **`FeatureNotImplementedError`** — New error class for `GENERAL_NOT_IMPLEMENTED` code (renamed from `NotImplementedError` to avoid Python stdlib clash).
- **`DependencyNotFoundError`** — New error class for `DEPENDENCY_NOT_FOUND` code.

### Changed
- APCore client and `apcore.*` global functions now provide full feature parity with `Executor`.

---

## [0.9.0] - 2026-03-06

### Added

#### Enhanced Executor.validate() Preflight
- **`PreflightCheckResult`** — New frozen dataclass representing a single preflight check result with `check`, `passed`, and `error` fields.
- **`PreflightResult`** — New dataclass returned by `Executor.validate()`, containing per-check results and `requires_approval` flag. Duck-type compatible with `ValidationResult` via `.valid` and `.errors` properties.
- **Full 6-check preflight** — `validate()` now runs Steps 1–6 of the pipeline (module_id format, module lookup, call chain safety, ACL, approval detection, schema validation) without executing module code or middleware.

### Changed

#### Executor Pipeline
- **Step renumbering** — Approval Gate renumbered from Step 4.5 to Step 5; all subsequent steps shifted +1 (now 11 clean steps).
- **`validate()` return type** — Changed from `ValidationResult` to `PreflightResult`. Backward compatible: `.valid` and `.errors` still work identically for existing consumers (e.g., apcore-mcp router).
- **`validate()` signature** — Added optional `context` parameter for call-chain checks; `inputs` now defaults to `{}`.

#### Public API
- Exported `PreflightCheckResult` and `PreflightResult` from `apcore` top-level package.

## [0.8.0] - 2026-03-05

### Added

#### Executor Enhancements
- **Dual-timeout model** — Global deadline enforcement (`executor.global_timeout`) alongside per-module timeout. The shorter of the two is applied, preventing nested call chains from exceeding the global budget.
- **Cooperative cancellation** — On module timeout, the executor sends `CancelToken.cancel()` and waits a 5-second grace period before raising `ModuleTimeoutError`. Modules that check `cancel_token` can clean up gracefully.
- **Error propagation (Algorithm A11)** — All execution paths (sync, async, stream) now wrap exceptions via `propagate_error()`, ensuring middleware always receives `ModuleError` instances with trace context.
- **Deep merge for streaming** — Streaming chunk accumulation uses recursive deep merge (depth-capped at 32) instead of shallow merge, correctly handling nested response structures.

#### Error System
- **ErrorCodeRegistry** — Custom module error codes are validated against framework prefixes and other modules to prevent collisions. Raises `ErrorCodeCollisionError` on conflict.
- **VersionIncompatibleError** — New error class for SDK/config version mismatches with `negotiate_version()` utility.
- **MiddlewareChainError** — Now explicitly `_default_retryable = False` per PROTOCOL_SPEC §8.6.

#### Utilities
- **`guard_call_chain()`** — Standalone Algorithm A20 implementation for call chain safety checks (depth, circular, frequency). Executor delegates to this utility.
- **`propagate_error()`** — Standalone Algorithm A11 implementation for error wrapping and trace context attachment.
- **`normalize_to_canonical_id()`** — Cross-language module ID normalization (Python snake_case, Go PascalCase, etc.).
- **`calculate_specificity()`** — ACL pattern specificity scoring for deterministic rule ordering.
- **`parse_docstring()`** — Docstring parser for extracting parameter descriptions from function docstrings.

#### ACL Enhancements
- **Audit logging** — `ACL` constructor accepts optional `audit_logger` callback. All access decisions emit `AuditEntry` with timestamp, caller/target IDs, matched rule, identity, and trace context.
- **Condition-based rules** — ACL rules support `conditions` for identity type, role, and call depth filtering.

#### Config System
- **Full validation** — `Config.validate()` checks schema structure, value types, and range constraints.
- **Hot reload** — `Config.reload()` re-reads the YAML source and re-validates.
- **Environment overrides** — `APCORE_*` environment variables override config values (e.g., `APCORE_EXECUTOR_DEFAULT_TIMEOUT=5000`).
- **`Config.from_defaults()`** — Factory method for default configuration.

#### Middleware
- **RetryMiddleware** — Configurable retry with exponential/fixed backoff, jitter, and max delay. Only retries errors marked `retryable=True`.

#### Registry Enhancements
- **ID conflict detection** — Registry detects and prevents registration of conflicting module IDs.
- **Safe unregister** — `safe_unregister()` with drain timeout for graceful module removal.

#### Context
- **Generic `services` typing** — `Context[T]` supports typed dependency injection via the `services` field.

#### Testing
- **Conformance test suite** — JSON fixture-driven tests for error codes, call chain safety, ACL evaluation, pattern matching, specificity, ID normalization, and version negotiation.
- **New unit tests** — 17 new test files covering all added features.

### Changed

#### Executor Internals
- `_check_safety()` now delegates to standalone `guard_call_chain()` instead of inline logic.
- Error handling wraps exceptions with `propagate_error()` and re-raises with `raise wrapped from exc`.
- Global deadline set on root call only, propagated to child contexts via `Context._global_deadline`.

#### Public API
- Expanded `__all__` in `apcore.__init__` with new exports: `RetryMiddleware`, `RetryConfig`, `ErrorCodeRegistry`, `ErrorCodeCollisionError`, `VersionIncompatibleError`, `negotiate_version`, `guard_call_chain`, `propagate_error`, `normalize_to_canonical_id`, `calculate_specificity`, `AuditEntry`, `parse_docstring`.

## [0.7.1] - 2026-03-04

### Added

#### Public API Extensions
- **Module Protocol** — Introduced `Module` protocol in `apcore.module` for standardized module typing.
- **Schema System** — Exposed schema APIs (`SchemaLoader`, `SchemaValidator`, `SchemaExporter`, `RefResolver`, `to_strict_schema`) to the top-level `apcore` exports.
- **Utilities** — Exposed `match_pattern` utility to the top-level `apcore` exports.

## [0.7.0] - 2026-03-01

### Added

#### Approval System (PROTOCOL_SPEC §7)
- **ApprovalHandler Protocol** - Async protocol for pluggable approval handlers with `request_approval()` and `check_approval()` methods
- **ApprovalRequest / ApprovalResult** - Frozen dataclasses carrying invocation context and handler decisions with `Literal` status typing
- **Phase A (synchronous)** - Handler blocks until approval decision; denied/timeout raise immediately
- **Phase B (asynchronous)** - `pending` status returns `_approval_token` for async resume via `check_approval()`
- **Built-in handlers** - `AlwaysDenyHandler` (safe default), `AutoApproveHandler` (testing), `CallbackApprovalHandler` (custom logic)
- **Approval errors** - `ApprovalError`, `ApprovalDeniedError`, `ApprovalTimeoutError`, `ApprovalPendingError` with `result`, `module_id`, and `reason` properties
- **Audit events (Level 3)** - Dual-channel emission: `logging.info()` always + span events when tracing is active
- **Extension point** - `approval_handler` registered as a built-in extension point in `ExtensionManager`
- **ErrorCodes** - Added `APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `APPROVAL_PENDING` constants

#### Executor Integration
- **Step 4.5 approval gate** - Inserted between ACL (Step 4) and input validation (Step 5) in `call()`, `call_async()`, and `stream()`
- **Executor.set_approval_handler()** - Runtime handler configuration
- **Executor.from_registry()** - Added `approval_handler` parameter
- **Dict and dataclass annotations** - Both `ModuleAnnotations` and dict-style `requires_approval` supported
- **Unknown status fail-closed** - Unrecognized approval statuses treated as denied with warning log

### Changed

#### Structural Alignment
- Approval errors re-exported from `apcore.approval` for multi-language SDK consistency; canonical definitions remain in `errors.py`
- `ApprovalResult.status` typed as `Literal["approved", "rejected", "timeout", "pending"]` per PROTOCOL_SPEC §7.3.2

## [0.6.0] - 2026-02-23

### Added

#### Extension System
- **ExtensionManager / ExtensionPoint** - Added a unified extension-point framework for `discoverer`, `middleware`, `acl`, `span_exporter`, and `module_validator`
- **Extension wiring** - Added `apply()` support to connect registered extensions into `Registry` and `Executor`

#### Async Task & Cancellation
- **AsyncTaskManager** - Added background task orchestration with status tracking, cancellation, concurrency limits, shutdown, and cleanup
- **TaskStatus / TaskInfo** - Added task lifecycle enum and metadata dataclass for async task management
- **CancelToken / ExecutionCancelledError** - Added cooperative cancellation primitives and integrated cancellation checks into executor flows

#### Trace Context & Observability
- **TraceContext / TraceParent** - Added W3C Trace Context utilities for `inject()`, `extract()`, and strict parsing via `from_traceparent()`
- **Context.create(trace_parent=...)** - Added distributed-tracing entry support by accepting inbound trace context
- **OTLPExporter top-level export** - Added OTLP exporter re-exports in observability and top-level public API

#### Registry Enhancements
- **Custom discoverer/validator hooks** - Added `set_discoverer()` and `set_validator()` integration paths
- **Module describe support** - Added `Registry.describe()` for human-readable module descriptions
- **Hot-reload APIs** - Added `watch()`, `unwatch()`, and file-change handling helpers for extension directories
- **Validation constants/protocols** - Added `MAX_MODULE_ID_LENGTH`, `RESERVED_WORDS`, `Discoverer`, and `ModuleValidator` exports

### Changed

#### Public API Surface
- Expanded top-level `apcore` exports to include cancellation, extensions, async task types, trace context types, additional registry protocols/constants, and new error classes

#### Error System
- Added `ModuleExecuteError` and `InternalError` to the framework error hierarchy and exports
- Extended `ErrorCodes` with additional constants used by newer execution/extension paths

### Fixed

#### Execution & Redaction
- **executor** - Added recursive `_secret_` key redaction for nested dictionaries
- **executor** - Preserved explicit cancellation semantics by re-raising `ExecutionCancelledError`

#### Import Graph Robustness
- Reduced import-coupling risk across middleware/observability/trace typing paths while preserving existing runtime behavior and public interfaces

## [0.5.0] - 2026-02-22

### Changed

#### API Naming
- **decorator** - Renamed `_generate_input_model` / `_generate_output_model` to `generate_input_model` / `generate_output_model` as public API
- **context_logger** - Renamed `format` parameter to `output_format` to avoid shadowing Python builtin
- **registry** - Renamed `_write_lock` to `_lock` for clearer intent

#### Type Annotations
- **decorator** - Replaced bare `dict` with `dict[str, Any]` in `_normalize_result`, `annotations`, `metadata`, `_async_execute`, `_sync_execute`
- **bindings** - Fixed `_build_model_from_json_schema` parameter type from `dict` to `dict[str, Any]`
- **scanner** - Fixed `roots` parameter type from `list[dict]` to `list[dict[str, Any]]`
- **metrics** - Fixed `snapshot` return type from `dict` to `dict[str, Any]`
- **executor** - Removed redundant string-quoted forward references in `from_registry`; fixed `middlewares` parameter type to `list[Middleware] | None`

#### Code Quality
- **executor** - Extracted `_convert_validation_errors()` helper to eliminate 6 duplicated validation error conversion patterns
- **executor** - Refactored `call_async()` and `stream()` to use new async middleware manager methods
- **executor** - Removed internal `_execute_on_error_async` method (replaced by `MiddlewareManager.execute_on_error_async`)
- **loader** - Use `self._resolver.clear_cache()` instead of accessing private `_file_cache` directly
- **tracing** - Replaced `print()` with `sys.stdout.write()` in `StdoutExporter`
- **acl / loader** - Changed hardcoded logger names to `logging.getLogger(__name__)`

### Added

#### Level 2 Conformance (Phase 1)
- **ExtensionManager** and **ExtensionPoint** for unified extension point management (discoverer, middleware, acl, span_exporter, module_validator) with `register()`, `get()`, `get_all()`, `unregister()`, `apply()`, `list_points()` methods
- **AsyncTaskManager**, **TaskStatus**, **TaskInfo** for async task execution with status tracking (PENDING, RUNNING, COMPLETED, FAILED, CANCELLED), cancellation, and concurrency limiting
- **TraceContext** and **TraceParent** for W3C Trace Context support with `inject()`, `extract()`, and `from_traceparent()` methods
- `Context.create()` now accepts optional `trace_parent` parameter for distributed trace propagation

#### Async Middleware
- **MiddlewareManager** - Added `execute_before_async()`, `execute_after_async()`, `execute_on_error_async()` for proper async middleware dispatch with `inspect.iscoroutinefunction` detection
- **RefResolver** - Added `clear_cache()` public method for cache management
- **Executor** - Added `clear_async_cache()` public method
#### Schema Export
- **SchemaExporter** - Added `streaming` hint to `export_mcp()` annotations from `ModuleAnnotations`

### Fixed

#### Memory Safety
- **context** - Changed `Identity.roles` from mutable `list[str]` to immutable `tuple[str, ...]` in frozen dataclass

#### Observability
- **context_logger / metrics** - Handle cases where `before()` was never called in `ObsLoggingMiddleware` and `MetricsMiddleware`

#### Security
- **acl** - Added explicit `encoding="utf-8"` to YAML file open


## [0.4.0] - 2026-02-20

### Added

#### Streaming Support
- **Executor.stream()** - New async generator method for streaming module execution
  - Implements same 6-step pipeline as `call_async()` (context, safety, lookup, ACL, input validation, middleware before)
  - Falls back to `call_async()` yielding single chunk for non-streaming modules
  - For streaming modules, iterates `module.stream()` and yields each chunk
  - Accumulates chunks via shallow merge for output validation and after-middleware
  - Full error handling with middleware recovery
- **ModuleAnnotations.streaming** - New `streaming: bool = False` field to indicate if a module supports streaming execution
- **Test coverage** - Added 5 comprehensive tests in `test_executor_stream.py`:
  - Fallback behavior for non-streaming modules
  - Multi-chunk streaming
  - Module not found error handling
  - Before/after middleware integration
  - Disjoint key accumulation via shallow merge


## [0.3.0] - 2026-02-20

### Added

#### Public API Extensions
- **ErrorCodes** - New `ErrorCodes` class with all framework error code constants; replaces hardcoded error strings
- **ContextFactory Protocol** - New `ContextFactory` protocol for creating Context from framework-specific requests (e.g., Django, FastAPI)
- **Registry constants** - Exported `REGISTRY_EVENTS` dict and `MODULE_ID_PATTERN` regex for consistent module ID validation
- **Executor.from_registry()** - Convenience factory method for creating an Executor from a Registry with optional middlewares, ACL, and config

#### Schema System
- **Comprehensive schema system** - Full implementation with loading, validation, and export capabilities
  - Schema loading from JSON/YAML files
  - Runtime schema validation
  - Schema export functionality

### Fixed
- **ErrorCodes class** - Prevent attribute deletion to ensure error code constants remain immutable
- **Planning documentation** - Updated progress bar style in overview.md


## [0.2.3] - 2026-02-20

### Added

#### Public API
- **ContextFactory Protocol** - New `ContextFactory` protocol for creating Context from framework-specific requests (e.g., Django, FastAPI)
- **ErrorCodes** - New `ErrorCodes` class with all framework error code constants; replaces hardcoded error strings
- **Registry constants** - Exported `REGISTRY_EVENTS` dict and `MODULE_ID_PATTERN` regex for consistent module ID validation
- **Executor.from_registry()** - Convenience factory method for creating an Executor from a Registry with optional middlewares, ACL, and config

### Changed

#### Core Improvements
- **Module ID validation** - Strengthened to enforce lowercase letters, digits, underscores, and dots only; no hyphens allowed. Pattern: `^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)*$`
- **Registry events** - Replaced hardcoded event strings with `REGISTRY_EVENTS` constant dict
- **Test fixtures** - Updated registry test module IDs to comply with new module ID pattern

#### Configuration
- **.code-forge.json** - Updated directory mappings: `base` from `planning/` to `./`; `input` from `features/` to `../apcore/docs/features`

### Improved
- Better type hints and protocol definitions for framework integration
- Consistent error handling with standardized error codes


## [0.2.2] - 2026-02-16

### Removed

#### Planning & Documentation
- **planning/features/** - Moved all feature specifications to `apcore/docs/features/` for better organization with documentation
- **planning/implementation/** - Restructured implementation planning to consolidate with overall project architecture

### Changed

#### Planning & Documentation Structure
- **Implementation planning** - Reorganized implementation plans to streamline project structure and improve maintainability



## [0.2.1] - 2026-02-14

### Added

#### Planning & Documentation Infrastructure
- **code-forge integration** - Added `.code-forge.json` configuration (v0.2.0 spec) with `_tool` metadata, directory mappings, and execution settings
- **Feature specifications** - 7 feature documents in `planning/features/` covering all core modules: core-executor, schema-system, registry-system, middleware-system, acl-system, observability, decorator-bindings
- **Implementation plans** - Complete implementation plans in `planning/implementation/` for all 7 features, each containing `overview.md`, `plan.md`, `tasks/*.md`, and `state.json`
- **Project-level overview** - Auto-generated `planning/implementation/overview.md` with module dependency graph, progress tracking, and phased implementation order
- **Task breakdown** - 42 task files with TDD-oriented steps, acceptance criteria, dependency tracking, and time estimates (~91 hours total estimated effort)

## [0.2.0] - 2026-02-14

### Fixed

#### Thread Safety
- **MiddlewareManager** - Added internal locking and snapshot pattern; `add()`, `remove()`, `execute_before()`, `execute_after()` are now thread-safe
- **Executor** - Added lock to async module cache; use `snapshot()` for middleware iteration in `call_async()` and `middlewares` property
- **ACL** - Internally synchronized; `check()`, `add_rule()`, `remove_rule()`, `reload()` are now safe for concurrent use
- **Registry** - Extended existing `RLock` to cover all read paths (`get`, `has`, `count`, `module_ids`, `list`, `iter`, `get_definition`, `on`, `_trigger_event`, `clear_cache`)

#### Memory Leak
- **InMemoryExporter** - Replaced unbounded `list` with `collections.deque(maxlen=10_000)` and added `threading.Lock` for thread-safe access

#### Robustness
- **TracingMiddleware** - Added empty span stack guard in `after()` and `on_error()` to log a warning instead of raising `IndexError`
- **Executor** - Set `daemon=True` on timeout and async bridge threads to prevent blocking process exit

### Added

#### Development Tooling
- **apdev integration** - Added `apdev[dev]` as development dependency for code quality checks and project tooling
- **pip install support** - Moved dev dependencies to `[project.optional-dependencies]` so `pip install -e ".[dev]"` works alongside `uv sync --group dev`
- **pre-commit hooks** - Fixed `check-chars` and `check-imports` hooks to run as local hooks via `apdev` instead of incorrectly nesting under `ruff-pre-commit` repo

### Changed

- **Context.child()** - Added docstring clarifying that `data` is intentionally shared between parent and child for middleware state propagation

## [0.1.0] - 2026-02-13

### Added

#### Core Framework
- **Schema-driven modules** - Define modules with Pydantic input/output schemas and automatic validation
- **@module decorator** - Zero-boilerplate decorator to turn functions into schema-aware modules
- **Executor** - 10-step execution pipeline with comprehensive safety and security checks
- **Registry** - Module registration and discovery system with metadata support

#### Security & Safety
- **Access Control (ACL)** - Pattern-based, first-match-wins rule system with wildcard support
- **Call depth limits** - Prevent infinite recursion and stack overflow
- **Circular call detection** - Detect and prevent circular module calls
- **Frequency throttling** - Rate limit module execution
- **Timeout support** - Configure execution timeouts per module

#### Middleware System
- **Composable pipeline** - Before/after hooks for request/response processing
- **Error recovery** - Graceful error handling and recovery in middleware chain
- **LoggingMiddleware** - Structured logging for all module calls
- **TracingMiddleware** - Distributed tracing with span support for observability

#### Bindings & Configuration
- **YAML bindings** - Register modules declaratively without modifying source code
- **Configuration system** - Centralized configuration management
- **Environment support** - Environment-based configuration override

#### Observability
- **Tracing** - Span-based distributed tracing integration
- **Metrics** - Built-in metrics collection for execution monitoring
- **Context logging** - Structured logging with execution context propagation

#### Async Support
- **Sync/Async modules** - Seamless support for both synchronous and asynchronous execution
- **Async executor** - Non-blocking execution for async-first applications

#### Developer Experience
- **Type safety** - Full type annotations across the framework (Python 3.11+)
- **Comprehensive tests** - 90%+ test coverage with unit and integration tests
- **Documentation** - Quick start guide, examples, and API documentation
- **Examples** - Sample modules demonstrating decorator-based and class-based patterns

### Dependencies

- **pydantic** >= 2.0 - Schema validation and serialization
- **pyyaml** >= 6.0 - YAML binding support

### Supported Python Versions

- Python 3.11+

---

[0.13.0]: https://github.com/aiperceivable/apcore-python/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/aiperceivable/apcore-python/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/aiperceivable/apcore-python/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/aiperceivable/apcore-python/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/aiperceivable/apcore-python/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/aiperceivable/apcore-python/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/aiperceivable/apcore-python/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/aiperceivable/apcore-python/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/aiperceivable/apcore-python/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/aiperceivable/apcore-python/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/aiperceivable/apcore-python/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/aiperceivable/apcore-python/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/aiperceivable/apcore-python/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/aiperceivable/apcore-python/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/aiperceivable/apcore-python/releases/tag/v0.2.1
[0.2.0]: https://github.com/aiperceivable/apcore-python/releases/tag/v0.2.0
[0.1.0]: https://github.com/aiperceivable/apcore-python/releases/tag/v0.1.0
