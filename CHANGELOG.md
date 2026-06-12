# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.24.0] - 2026-06-12

### Added

- **Per-instance `ToggleState` isolation (#71).** Each `APCore` instance now owns one `ToggleState`, injected into both the toggle module (write path) and the pipeline lookup step `BuiltinModuleLookup` (read path). Disabling a module on one `APCore` instance no longer affects another instance in the same process — closing the isolation gap for multi-tenant servers and test bleed. The process-global `ToggleState` is retained **only** as a fallback for the free `is_module_disabled` / `isModuleDisabled` function (callers that hold no instance handle). A-D-12 is re-scoped from "process-global, survives reload" to "isolated to the `APCore` instance, survives reload of that instance" (`docs/features/system-modules.md`). New conformance fixture `conformance/fixtures/toggle_state_isolation.json` locks the cross-language contract (cross-instance isolation + reload survival). Implemented across `apcore-python`, `apcore-typescript`, and `apcore-rust`.

- **Canonical AI-agent tool-governance ACL artifact + fixture (#72).** New runnable reference `examples/acl/agent-tool-governance.yaml` — a `default_effect: deny` policy that scopes tool access by caller pattern + identity `roles` + `max_call_depth`: `@external` → `executor.*.read` only; `agent.*` + `roles: [reader]` → `executor.*.read` / `executor.*.query` capped at `max_call_depth: 3`; `agent.*` + `roles: [data_admin]` → `data.export` / `executor.*.delete` (uncapped). New conformance fixture `conformance/fixtures/acl_agent_scoping.json` freezes the scenario as a cross-language contract (19 cases: the `@external < reader < data_admin` privilege gradient, the inclusive depth boundary — depth 3 allowed / 4 denied, role separation, and missing-identity deny). New guide section **"9.4 AI Agent Tool Governance"** in `docs/guides/acl-configuration.md`. Additive and non-normative — no `protocol-spec.md` change; the ACL engine already supports `conditions: {roles, max_call_depth}`. Framework integrations (`django-apcore`, `fastapi-apcore`, …) vendor the artifact and verify identical decisions.

### Changed

- **`docs/features/system-modules.md` — toggle-state contract re-scoped from process-global to per-instance (#71).** The "Disabled modules" note, the `system.control.toggle_feature` postconditions, and the `is_module_disabled` Contract block now state that toggle state is isolated to the owning `APCore` instance (with the standalone free function reading the global fallback). No public method-signature changes.

### Fixed

- **`docs/features/event-system.md` — `EventEmitter.emit` Contract internal contradiction resolved.** The `### Inputs` bullet said `event_type` "MUST NOT be empty" while the same block's `### Errors` ("No errors raised to the caller. emit is fire-and-forget") and `### Properties` ("never raises") sections — which all three SDKs implement — mandate never-raising. Surfaced by a cross-language `tester --category protocol` run (a green TypeScript placeholder test mislabeled `SKIP` had been miscounted as enforcing the rule). The `MUST` on the caller is preserved, reframed as a **caller precondition, not a validated rejection**, matching the `Errors`/`Properties` sections and the unanimous SDK behavior. No public surface change, no normative weakening, version stays `0.24.0`. The corresponding `apcore-typescript` placeholder test was converted to a real `it.skip()` so it stops registering as a false-positive pass.

### Notes

- **#70 (default AI error-recovery metadata) confirmed complete across all three SDKs.** `user_fixable`-by-code resolution and the `ai_guidance` defaults ship in `apcore-python` / `apcore-typescript` / `apcore-rust` and pass `error_recovery_metadata.json` (the 0.23.0 "apcore-typescript / apcore-rust pending" note is superseded). Remaining follow-ups are tracked outside this changelog entry: the `protocol-spec.md` §8 `user_fixable` alignment note (requires a linked issue + dual maintainer review per repo policy) and the downstream `apcore-mcp` floor bump to `apcore>=0.23.0`.

---

## [0.23.0] - 2026-06-10

### Added

- **Conformance fixture `error_recovery_metadata.json` — default AI error-recovery metadata per error code (#70).** Defines the cross-language contract for `retryable` + `user_fixable` defaults: `user_fixable` is resolved from the error code (caller-fixable-by-input = `true`; governance/system/structural/transient = `false`); codes whose `user_fixable` is `null` are left for the module author (e.g. `MODULE_EXECUTE_ERROR`). All SDKs MUST produce these defaults when an error is constructed without overrides. `ai_guidance` text is not pinned (human-readable, may carry runtime context). Registered in `conformance/README.md`. Implemented in `apcore-python` 0.23.0; `apcore-typescript` / `apcore-rust` pending.

### Changed

- **`error_serialization.json` `omits_null_optionals` case now uses a real policy-free code (`CONTEXT_BINDING_ERROR`) (#70).** Real codes such as `GENERAL_INVALID_INPUT` now carry a default `user_fixable`, so the null-optional-omission case must use a code absent from the user_fixable policy — and a real `ErrorCode` variant, since Rust models codes as a closed enum — to keep exercising sparse-output omission across all SDKs.

### Fixed

These are spec-consistency clarifications surfaced by a cross-language sync audit. They resolve internal contradictions/gaps in the feature specs so the three SDKs have a single source of truth to converge on; no public surface changes, version stays `0.23.0`.

- **Error fingerprint definition de-duplicated (`docs/features/observability.md`).** The "Error fingerprinting" section defined the digest as `SHA-256(error_code:top_frame_hash:sanitized_message)` while §1.4 defined it as `SHA-256(error_code:module_id:normalized_message)`. A `top_frame_hash` (stack frame file/function/line) is not portable across Python/TypeScript/Rust, so it can never produce an equal cross-language fingerprint. §1.4 (the 3-part, `module_id`-based form) is now the single authoritative definition; `top_frame_hash` is demoted to an optional language-local diagnostic field that MUST NOT influence the shared fingerprint, and the normalization is pinned to the exact five-step §1.4 algorithm (no extra hex-collapsing step). *Code follow-up: `apcore-python` must drop the top-frame component and the extra normalization step to match TS/Rust.*
- **Event-emitter overflow behavior is now normative (`docs/features/event-system.md`).** The `emit()` contract previously left buffer-overflow behavior undefined, allowing a bounded `maxPending` queue to silently drop deliveries (bypassing the dead-letter path). `emit()` now MUST NOT silently discard an accepted event: a bounded buffer's overflow MUST either apply backpressure or fail through the dead-letter path (`apcore.event.delivery_failed`, `reason: "pending_overflow"`). *Code follow-up: `apcore-typescript` must route `maxPending` overflow through the DLQ instead of `console.warn`.*
- **`Config.validate()` contract added (`docs/features/config-bus.md`).** The spec previously defined no required-field set or value constraints, so each SDK enforced a different subset (same config, three verdicts). Added a normative `Contract: Config.validate` with the canonical required fields, value constraints, and namespace-mode schema/strict rules, anchored to the reference SDK. *Code follow-up: `apcore-rust` and `apcore-typescript` align up to the full set.*
- **Conformance fixture `multi_module_discovery.json::full_id_grammar_valid` corrected.** A single-class input expected the appended id `executor.math.arithmetic.addition`, contradicting the single-class identity guarantee (and the `single_class_id_unchanged` case in the same fixture). Now expects the bare `executor.math.arithmetic`; verified green in all three SDKs.
- **`Contract: guard_call_chain` `### Inputs` corrected (`docs/features/call-chain-guard.md`).** The Contract block listed a `context` (Context) input and named the limit params `max_depth` / `max_repeat`, contradicting the function signature defined earlier in the same file and implemented identically across all three SDKs: `guard_call_chain(module_id, call_chain, *, max_call_depth, max_module_repeat)`. The block now lists `call_chain` (not `context`) and the correct keyword names, making the spec internally consistent; the `pure` property note now references `call_chain` rather than "context state". No public surface changes; surfaced by a cross-language `tester --category protocol` run.

---

## [0.22.0] - 2026-05-27

### Changed

- **Decision D-24 — `Context.create()` signature unified across all SDKs (#66).** Reduced to the six caller-supplied fields only: `identity`, `trace_parent`, `cancel_token`, `data`, `services`, `global_deadline`. Two prior inputs are removed from the public factory surface:
  - `executor` — Executor self-binds at pipeline entry under the new normative §"Contract: Executor binding to Context" (`docs/features/core-executor.md`). This unifies three previously distinct binding scenarios under one rule: local construction via `Context.create()`, cross-process deserialize, and hot-reload survivor restore.
  - `caller_id` — zero production / test / doc callers across the 25-repo ecosystem. Top-level Contexts always have `caller_id = null`; the value is managed exclusively by `Context.child()`. Reserved name for future revisions.

  Added `cancel_token` as a first-class parameter, eliminating the post-hoc `ctx.cancel_token = token` anti-pattern documented in 9 production sites across `apcore-mcp-{python,typescript}`, `apcore-typescript/async-task.ts` (which had to cast away `readonly`), `axum-apcore`, `django-apcore`, `fastapi-apcore`.

  Rust `TraceParent` struct gains a `tracestate: Vec<(String, String)>` field to align with Python/TS shape; the redundant `ContextBuilder::tracestate()` setter is removed.

  Two new normative sections clarify distributed semantics:
  - §"Contract: Distributed cancellation" — `cancel_token` is local-only; cross-process cancellation MUST go through out-of-band channels (e.g., AsyncTaskStore task_id lookup).
  - §"Contract: `global_deadline` distributed semantics" — `global_deadline` does not propagate across process boundaries; callers needing wall-clock deadline propagation SHOULD store it in `context.data` under an extension key.

  Conformance fixture `context_create.json` validates cross-SDK parameter parity, removal of `executor`/`caller_id`, idempotent same-executor rebinding, and cross-executor conflict behavior.

- Renumbered decision-log entries D-17–D-22 → D-58–D-63 to resolve a duplicate-ID collision with the v0.22.0 executor/async hardening decisions (which retain D-17–D-22). See `docs/spec/2026-05-decision-log.md`.

### Added

- **Decision D-17 — `TaskStore` is async across all SDKs** (`docs/features/async-tasks.md` §1.1). Pluggable backends like Redis or SQL cannot satisfy a sync `TaskStore` contract without blocking the runtime's event loop. New normative: every `TaskStore` method MUST be asynchronous in Python (`async def`), TypeScript (returns `Promise<T>`), and Rust (`async fn` via `#[async_trait]`). `InMemoryTaskStore` MUST still expose async signatures even though its operations are CPU-only — uniform shape lets stores compose generically. Supersedes the partially-sync contract present in apcore-python and apcore-typescript through v0.21.x. Found via `/apcore-skills:sync` (finding A-D-AT-04).
- **Decision D-18 — `cancel()` MUST be a real interrupt across SDKs** (`docs/features/async-tasks.md` §Cancellation Integration). Cooperative-flag-only cancellation is non-conforming. In TypeScript specifically, `CancelToken` MUST be backed by an `AbortController` and MUST expose `signal: AbortSignal` on `Context` so modules using `fetch` / `setTimeout` / Web Streams participate in real abort. Closes the cross-SDK divergence where Python `asyncio.Task.cancel()` and Rust `tokio handle.abort()` interrupted in-flight work but the TS flag-only path silently completed module side effects. Found via finding A-D-AT-02.
- **Decision D-19 — `call_with_trace` / `callWithTrace` shares `call()` error semantics** (`docs/features/core-executor.md` §Trace Variants). The trace variant MUST run the same `on_error` middleware chain, apply the same cancellation short-circuit (D-20), and apply the same `MiddlewareChainError` unwrap (D-22). The trace is the observable record of execution — including any middleware recovery — not a sanitized projection. Closes the divergence where TS+Rust `call_with_trace` rethrew unconditionally while Python ran `on_error` recovery. Found via finding A-D-EXEC-004.
- **Decision D-20 — Cancellation short-circuits the `on_error` chain** (`docs/features/core-executor.md` §Cancellation Short-Circuit). `ExecutionCancelledError` MUST be detected after pipeline-error unwrap and propagated directly, bypassing `on_error`. Rationale: cancellation is a caller-driven request to stop, not a recoverable failure; allowing `on_error` middleware to observe it lets logging middleware swallow it or retry middleware reissue a `RetrySignal` that restarts the loop. Closes the gap in apcore-rust where missing short-circuit allowed middleware to recover cancellation. Found via finding A-D-EXEC-003.
- **Decision D-21 — Cancel token MUST be checked at two pipeline points** (`docs/features/core-executor.md` §Cancel Token Mid-Pipeline Check). The pipeline MUST observe `cancel_token` cancellation at Step 2 (Call-Chain Guard) and again at Step 8 (Execute), in addition to honoring it inside `module.execute()` itself. Single-check implementations leak compute (the pipeline runs ACL/middleware/validation even though the caller has already cancelled) and are non-conforming. Closes the divergence where TS checked at step 2 but Python+Rust only checked at step 8 (and Rust missed step 8 entirely). Found via finding A-D-EXEC-002.
- **Decision D-22 — `MiddlewareChainError` MUST be unwrapped before propagation** (`docs/features/core-executor.md` §Error Unwrap Rule). When a middleware (`before` / `after` / `on_error`) raises a domain-typed error like `ApprovalDeniedError`, the chain machinery may wrap it in `MiddlewareChainError` for diagnostics. The executor MUST unwrap before propagating, surfacing the original typed cause. SDKs MUST NOT replace the cause with a generic `ModuleExecuteError`. Closes the gap where Python wrapped to `ModuleExecuteError` while TS+Rust unwrapped — breaking MCP/A2A bridges that key off the typed error. Found via finding A-D-EXEC-005.
- **Decision D-23 — `get_status` / `list_tasks` MUST return shallow copies in every SDK** (`docs/features/async-tasks.md` §Contract: AsyncTaskManager.get_status). Closes the contract contradiction where the prior spec said "the live dataclass reference (Python); callers MUST NOT rely on mutation" while finding A-D-AT-06 mandated copies. Python now returns `dataclasses.replace(info)`, TypeScript returns `{ ...info }`, Rust returns a clone — uniform mutation-safety contract across all three SDKs. Found while resolving A-D-AT-06.

- **`docs/features/registry-system.md` — Registration Ordering Invariants now explicitly apply to every register path.** Added a `!!! warning "Applies to every registration path"` admonition clarifying the invariants apply uniformly to the public `register()` API, internal helpers (`register_internal` used by sys-modules), and discovery-driven paths (`discover()`, `register_discovered`, hot-reload). SDKs MUST NOT create per-path exceptions; if a discover-time `on_load` callback needs to enumerate sibling modules, the callback MUST be re-shaped as a post-discover hook rather than as grounds for an early-visibility carveout. Closes implementation drift where apcore-python and apcore-typescript implemented deferred-publish on `register()` but inserted into the visible map BEFORE `on_load` on the discover path. Found via findings A-D-REG-003 and A-D-REG-004.

### Fixed

- **§9.16.2 Canonical Core Event Types table aligned with v0.22.0 #36 rename.** `docs/spec/protocol-spec.md` §9.16 still listed the pre-v0.22.0 canonical names `apcore.module.registered` / `apcore.module.unregistered` (subsystem `module`) and `apcore.error.threshold_exceeded` / `apcore.latency.threshold_exceeded` (categories `error` / `latency` as the subsystem segment, which violates the `apcore.<subsystem>.<event>` convention). The v0.22.0 rename had shipped only in `docs/features/event-system.md` (legacy-aliases table + canonical-events table), leaving the normative spec table contradicting the feature spec, the conformance fixture `event_naming.json`, the registry-system / rfc-ephemeral-modules / system-modules docs, and all three SDKs' v0.22.0 behavior. Updated rows 6035-6036 to `apcore.registry.module_registered` / `apcore.registry.module_unregistered` and rows 6040-6041 to `apcore.health.error_threshold_exceeded` / `apcore.health.latency_threshold_exceeded`; updated the §9.16.1 example row; rewrote the §9.16.2 preamble + collision-resolution note to distinguish the v0.18.0 short-form removals (Cohort A) from the v0.22.0 subsystem-segment renames (Cohort B).
- **User-facing examples no longer subscribe to v0.22.0-removed legacy event names.** `docs/features/apcore-client.md` (Python / TypeScript / Rust Production Setup tabs and the `event_type` parameter example), `docs/features/event-system.md` (three Subscriber examples), `docs/features/observability.md` ("Events emitted" table), and `docs/features/system-modules.md` (Configuration YAML comments) all referenced `apcore.error.threshold_exceeded` / `apcore.latency.threshold_exceeded`. Replaced with the v0.22.0 canonical `apcore.health.error_threshold_exceeded` / `apcore.health.latency_threshold_exceeded`. The legacy names remain documented in the `event-system.md` rename table for historical reference; no other doc still teaches users to subscribe to the removed names. Found via `/apcore-skills:sync --scope core` (findings B-001 through B-004).
- **Cross-language doc/guide consistency corrections (second `/apcore-skills:sync --scope core` pass).** `docs/guides/testing-modules.md` — removed the forbidden `executor=` argument from both the Python and TypeScript `Context.create()` examples; `executor` self-binds at pipeline entry per D-24 / §"Contract: Executor binding to Context" and is not a `create()` parameter. `docs/features/event-system.md` — past-tensed the legacy-event dual-emission rule: dual-emission applied through v0.21.x only and ended at v0.22.0, where implementations emit the canonical names exclusively (aligns with `docs/spec/protocol-spec.md` §9.16). `docs/features/registry-system.md` — added the missing `await` to the async TypeScript `registry.register()` example. `docs/features/middleware-system.md` — corrected the stale note claiming TypeScript catches `after()` errors per-hook; all three SDKs fail-fast on the first error.
- **Call / stream Contract blocks now cite the registered error code `SCHEMA_VALIDATION_ERROR`.** `docs/features/apcore-client.md` (the `call` and `stream` Contract `### Errors` blocks) and `docs/features/streaming.md` (the `Module.stream` Contract) declared `SchemaValidationError(code=SCHEMA_VALIDATION_FAILED)` for input-schema validation failures. `SCHEMA_VALIDATION_FAILED` is not the registered code for the public call/stream surface — the canonical code per `docs/spec/protocol-spec.md` §8.2 / §4.14 and conformance `T04-005` / `T06-006` is `SCHEMA_VALIDATION_ERROR`, which the Python and Rust SDKs emit. Corrected all three Contract-block citations. The `_FAILED` form remains intentionally documented as a validation-outcome alias in `docs/features/error-system.md` (Schema Edge-Case Errors table) and on the raise-on-failure `validate_input` / `validate_output` / `validate_or_error` entry points in `docs/features/schema-system.md`; those are left unchanged. Found via `/apcore-skills:sync --scope core` review (finding B-001).
- **`docs/features/error-system.md` — reserved-error-code-prefix list de-duplicated to a single canonical set of 14.** The requirements section (§"Error Code Registry") and the `ErrorCodeRegistry` section carried two contradicting reserved-prefix lists: the former still listed the 0.18-retired `ERROR_FORMATTER_` / `EXECUTION_` / `RELOAD_` / `TASK_` prefixes (plus an inconsistent count) while the latter listed only the 14 common-core prefixes. Both lists now declare the identical 14 (`ACL_`, `APPROVAL_`, `BINDING_`, `CALL_`, `CIRCULAR_`, `CONFIG_`, `DEPENDENCY_`, `ERROR_CODE_`, `FUNC_`, `GENERAL_`, `MIDDLEWARE_`, `MODULE_`, `SCHEMA_`, `VERSION_`). Added a normative clarification that one-off framework codes outside these prefixes (`CIRCUIT_BREAKER_OPEN`, `CONTEXT_BINDING_ERROR`, `STREAMING_INTERFACE_MISMATCH`, `STRATEGY_NOT_FOUND`, `PIPELINE_*`, `STEP_*`, `RELOAD_FAILED`, `EXECUTION_CANCELLED`, `TASK_LIMIT_EXCEEDED`) are protected by exact-code collision detection rather than prefix reservation. SDKs (`apcore-python`, `apcore-rust`) that had drifted to broader prefix sets are realigned to the 14 in their respective repos. Found via `/apcore-skills:sync --scope core` (findings B-001, A-D-006).
- **`docs/features/context-object.md` — repaired malformed Rust fenced code block.** The canonical `Context::create()` example had an unclosed `Identity::new(...)` call and two stray ` ```rust ` / ` ``` ` fences mid-block, causing the 6-parameter `Context::create` example to drop out of the rendered MkDocs page. The block is now one continuous, complete Rust example. Found via `/apcore-skills:sync --scope core` (finding B-002).
- **`docs/features/apcore-client.md` — Global Singleton scoped to Python-only.** The §"Global Singleton" SHOULD statement claimed both Python and TypeScript provide module-level convenience functions (`apcore.call()`); in practice only the Python SDK does. TypeScript is now grouped with Rust as explicit-instances-only, matching the shipped SDKs. Found via `/apcore-skills:sync --scope core` (finding B-004).

---

### Added

- **§9.9.5 Reserved Namespace Query — new normative API requirement (#60).** All SDKs **MUST** expose a public, read-only query API returning the set of reserved top-level namespace names (`apcore`, `_config` at minimum). The query API is the single source of truth used by `register_namespace` to enforce `CONFIG_NAMESPACE_RESERVED` (§9.5.1 rules 3 and 4). Intended for third-party consumers (custom CLIs, framework integrations) that accept user-supplied namespace names and want fail-fast pre-validation. Class-level / module-level access (no `Config` instance required). Cross-language examples added for Python (`Config.reserved_namespaces()`), TypeScript (`Config.reservedNamespaces`), and Rust (`Config::reserved_namespaces()`).
- **`docs/features/event-system.md` — Event Delivery Semantics section (#61).** Normative cross-subscriber delivery contract: every subscriber type (built-in and user-registered) MUST accept a `retry` block (`max_attempts` / `initial_backoff_ms` / `max_backoff_ms` / `backoff_multiplier`) governing retry on transient delivery failure; permanent failure MUST emit `apcore.event.delivery_failed` (with full payload schema) which itself MUST NOT be retried; `subscribe()` SHOULD accept an optional `on_failure` callback. New optional `id` field on every subscriber config surfaces a stable `subscriber_id` in DLQ events. The `a2a` subscriber `skill_id` is now configurable (default `"apevo.event_receiver"`). The `WebhookSubscriber` vs `A2ASubscriber` comparison table is updated: both now apply the unified retry policy. Discovered during apcore-a2a upgrade — closes the silent-drop gap for in-process subscribers that previously had no retry / DLQ path.
- **`docs/features/streaming.md` — Streaming Module Interface section (#62).** Explicit `StreamingModule` interface replaces duck-typing: Python `@runtime_checkable` Protocol with `isinstance` detection; TypeScript interface + `Symbol.for("apcore.streaming")` marker + `isStreamingModule` type-narrowing helper (transitional fallback to method-presence detection with a one-shot deprecation log; marker becomes MUST at next major); Rust `trait StreamingModule: Module` accessed via `Module::as_streaming() -> Option<&dyn StreamingModule>`, coexisting with the existing `Module::stream() -> Option<ChunkStream>` (the two paths MUST stay consistent per module). Defines `StreamingInterfaceError` raised at module-load time when a declared-streaming module's signature does not match the interface. Adapter / bridge code (apcore-a2a, apcore-mcp) MUST use the standard detection mechanism, not bare `hasattr` / `typeof` checks.
- **`docs/features/context-object.md` — Typed Access via ContextKey[T] section (#63).** Promotes the existing `ContextKey[T]` API ([design-context-annotations-acl.md §1.4](docs/spec/design-context-annotations-acl.md)) as the recommended pattern for stable, schema-bearing state on `context.data`. Cross-language examples with the correct **key-anchored** API (`KEY.set(ctx, value)` / `KEY.get(ctx, default)` / `KEY.exists(ctx)` / `KEY.delete(ctx)` / `KEY.scoped(suffix)`) for Python / TypeScript / Rust. Aligns the third-party namespace rule with the existing `ext.*` prefix mandated by middleware-hardening §1.1. Framework-reserved key list sourced from spec §1.5 (TRACING_SPANS / TRACING_SAMPLED / METRICS_STARTS / LOGGING_START / REDACTED_OUTPUT / RETRY_COUNT_BASE). No runtime behavior change — purely a discoverability / best-practice elevation.
- **`docs/features/middleware-system.md` — Duplicate Middleware Detection section (#64).** Normative SHOULD that SDKs detect prior registration of a middleware sharing the same identity (Python: `f"{module}.{qualname}"`; TS: `"<module-specifier>:<ClassName>"` with constructor-name fallback; Rust: `std::any::type_name::<T>()`) and emit a `WARNING`-level log naming both registration sites. Detection MUST be non-blocking and MUST NOT mutate registration order. Per-registration `allow_duplicate` flag SHOULD be available for intentional stacking; custom `identity_key` (vendor-prefixed; `apcore.*` reserved) lets the same class be registered for distinct purposes without triggering the warning.
- **`docs/features/registry-system.md` — Registration Ordering Invariants section (#65).** Strong-guarantee invariant: a module MUST NOT become visible to discovery APIs (`get` / `list` / `get_definition`) until all `on_load` callbacks have completed successfully. On callback failure the module MUST NOT become visible and the registry MUST emit `apcore.registry.module_load_failed` with `{module_id, callback_name, error_type, error_message, timestamp}`. The existing `Contract: Registry.register` side-effects ordering is updated to a deferred-publish pattern: reserve in-flight slot under registry lock → release lock → run `on_load` under per-module init lock → atomically publish or roll back. Callbacks for distinct modules MAY still run concurrently.
- **`conformance/fixtures/event_delivery_semantics.json`** (#61) — 4 cross-language test cases: retry succeeds before exhaustion (verifies attempt count + backoff delays), permanent failure emits `apcore.event.delivery_failed` with full normative payload, DLQ-subscriber failure is not re-retried (no infinite loop), SDK-generated `subscriber_id` when config omits `id`.
- **`conformance/fixtures/registry_load_ordering.json`** (#65) — 4 cross-language test cases: visibility only after successful `on_load`, callback failure blocks visibility and emits `apcore.registry.module_load_failed`, concurrent same-ID rejected with `DUPLICATE_MODULE_ID`, concurrent distinct-IDs run in parallel (wall-clock assertion).
- **`conformance/fixtures/schema_content_hash.json`** (A-D-037) — 5 cross-language schema content-hash canonicalization fixtures designed to expose canonicalization divergence: float rendering (`1.0`), non-ASCII Unicode keys/values, large integers beyond IEEE-754 exact range, unsorted nested object keys, and a baseline control. No `expected` hash is recorded — each SDK computes the content hash and the harness asserts cross-repo byte-for-byte agreement.

### Changed

- **`A2ASubscriber` delivery behavior — now retries on 5xx / connection / timeout (#61).** Previously single-attempt: A2A failures returned silently after one try. Now A2A applies the unified `retry` policy (default `max_attempts: 3`) like `WebhookSubscriber`. SDK consumers that relied on single-attempt semantics MUST set `retry: { max_attempts: 1 }` explicitly. Side effects of retries (duplicate downstream processing on retried 5xx) are the receiver's responsibility — implement idempotency at the A2A endpoint.
- **`Registry.register` — concurrent same-ID registration semantics tightened (#65).** Previously a race between two `register()` calls for the same `module_id` could resolve via lock-ordering with one succeeding and one failing late. Now an in-flight loading set rejects the second caller immediately with `InvalidInputError(code=DUPLICATE_MODULE_ID)`. Callers that previously assumed race-tolerant insertion MUST serialize per-module-ID registration explicitly. The new behavior is symmetric with sequential same-ID registration, which has always raised `DUPLICATE_MODULE_ID`.

### Notes

- **Issues #61–#65 were discovered during the apcore-a2a upgrade audit** and target gaps that bleed into adapter / bridge implementations. They are feature-level changes; `docs/spec/protocol-spec.md` is unchanged. SDK rollout follow-ups will be filed in `apcore-python`, `apcore-typescript`, and `apcore-rust`.
- **Behavior changes (A2A retry, registry concurrent same-ID) are normative spec changes**, not implementation drift. SDKs implementing v0.22.0 MUST adopt the new behavior on the next minor release; SDK CHANGELOGs SHOULD restate these `Changed` entries.

---

## [0.21.0] - 2026-05-06

### Added

- **§2.5 Reserved Words — `ephemeral` added to framework reserved list (RFC `rfc-ephemeral-modules.md` accepted).** New reserved namespace `ephemeral.*` for programmatically-generated runtime modules synthesized by LLM agents (à la ToolMaker, ACL 2025). Standard `Registry.register()` only — `register_internal()` MUST reject `ephemeral.*` IDs. Reserved-namespace semantics table added below the YAML block. See `docs/spec/rfc-ephemeral-modules.md` for the full namespace contract (registration / lifecycle / audit / sandboxing).
- **§4.4 ModuleAnnotations — new `discoverable: boolean` field (default `true`).** Controls visibility in enumeration surfaces (`Registry.list()`, `Registry.find()`, manifest export, MCP `tools/list`). `false` hides the module from discovery while keeping it callable by exact ID. `ephemeral.*` modules SHOULD set `discoverable: false`.
- **§5.6 Module Interface Protocol — optional `preview()` method (RFC `rfc-preview-method.md` accepted).** Modules implement `preview(inputs, context) -> PreviewResult | null` to self-report structured-diff predictions of state changes the call would produce. Sits alongside `preflight()` (warnings) — they are orthogonal surfaces. Exception semantics mirror `preflight()` (advisory warning via `module_preview` check entry; does not fail validation). Pseudocode interface block + `optional_methods` YAML both updated.
- **§12.8 PreflightResult schema — new `predicted_changes: List<Change>` field; `Change` and `PreviewResult` types defined.** `Change` has required `action` / `target` / `summary` (free-form strings) and optional `before` / `after` snapshots; `x-*` extension fields are permitted (cross-SDK encoding patterns documented in the RFC). `PreflightCheckResult.check` enum extended with `module_preview`.
- **§4.6 conventions table — three new `x-*` AI-routing keys (D-57).** Documentation-only registration of three metadata conventions surfaced by the 2025–2026 LLM-agent / tool-use frontier-research alignment audit:
  - `x-reasoning-demand` (`low` | `medium` | `high`) — hint of the minimum reasoning capability the calling agent needs; consumed by upstream model routers for tier selection. Aligns with RouteLLM (ICLR 2025), xRouter (arXiv 2510.08439), and Cost-Aware Model Orchestration (arXiv 2512.01099).
  - `x-required-context-keys` — array of `context.data` key names the module reads, enabling orchestrators to inject required state ahead of the call. Does **not** include framework-owned Context fields (`trace_id`, `caller_id`, etc.).
  - `x-supports-dry-run` (boolean) — module-level signal that `Executor.validate()` (§12.2) is meaningful for this module.
- **`docs/spec/2026-05-decision-log.md` — D-57 (§4.6 reasoning/context/dry-run conventions)** — marked **resolved**. Resolution status block updated to include D-57.

### Notes

- **No SDK behavior change** for §4.6 D-57 conventions. All three SDKs treat module `metadata` as a free-form dict / `Record<string, unknown>` / `serde_json::Value`; the framework explicitly does not validate metadata content (§4.6 "Metadata Design Principles"). New `x-*` keys are additive conventions only.
- **Strict-mode export unaffected** by D-57. §4.16 mandates that `to_strict_schema()` strips all `x-*` extension fields, so the new keys do not surface in strict-mode output.
- **SDK rollout for §2.5 / §4.4 / §5.6 / §12.8 spec promotions is synchronized.** All three SDKs ship the full v0.21.0 surface (Stage 2 `Module.preview()` + Stage 3 `ephemeral.*` namespace + `discoverable` annotation + audit single-emit + `register_internal()` rejection):
  - `apcore-python` v0.21.0 — pyproject.toml bumped; PR #26 + iter-11 alignment shipped Stage 3; commit `203a9a6` shipped Stage 2.
  - `apcore-typescript` v0.21.0 — package.json bumped; PR #29 + iter-11 shipped Stage 2 (with TypeBox `Type.Unsafe` form for `Change.x-*`); commit `577b09b` shipped Stage 3.
  - `apcore-rust` v0.21.0 — Cargo.toml bumped; PR #25 shipped Stage 2 prerequisite (`#[non_exhaustive]` hygiene); commits `afb6e05` + `e6abb7b` shipped Stage 2 `Module::preview()` + Stage 3 ephemeral/discoverable.
- **Conformance fixture `annotations_extra_round_trip.json` deferred.** Per `rfc-ephemeral-modules.md` "Transitional fixture handling during multi-SDK rollout", the fixture is NOT updated to require `discoverable` until all 3 SDKs have shipped support. SDKs implementing `discoverable` MAY make their conformance test runner pilot-tolerant in the interim. Synchronized fixture update will land in a follow-up PR after TypeScript and Rust ship `discoverable`.

---

## [0.20.0] - 2026-05-05

### Added

- **`UsageExporter` Protocol/interface/trait + `NoopUsageExporter` + `PeriodicUsageExporter` in all 3 SDKs (#45 §3, D-55).** Push-style usage summary export distinct from pull-style `PrometheusExporter`. `PeriodicUsageExporter` polls `UsageCollector.summary()` every `interval_seconds` (default 3600 = 1 hour) and calls `exporter.export(summary)`. `stop()` halts the loop, awaits `exporter.shutdown()`, and is idempotent. apcore SDKs do **not** ship transport-bound exporters (HTTP, Kafka, etc.) — those are explicitly out-of-tree. Documented in `docs/features/observability.md` "## UsageExporter (push-style)". Conformance: `conformance/fixtures/usage_exporter.json` (3 cases).
- **TypeScript registry `_filterIdConflicts` 8-stage decomposition matches Rust + Python (D-32, D-56).** TypeScript `_discoverDefault` refactored to expose `_filterIdConflicts` as a separate private helper between candidate enumeration and registration; `_registerInOrder` reduced to pure registration. Cross-language conformance no longer special-cases TS — all 3 SDKs follow the canonical 8-stage Algorithm A04 shape.
- **`conformance/fixtures/usage_exporter.json`** (#45 §3, D-55) — 3 cross-language test cases: `NoopUsageExporter` no-op, `PeriodicUsageExporter` pushes summary at interval, `stop()` is idempotent and drains in-flight exports.
- **`conformance/fixtures/sensitive_keys_default.json`** (#43 §5, D-54) — 4 cross-language test cases: canonical 16-entry default list shape, legacy `_secret_*` prefix redaction under default, common credential-term redaction (case-insensitive substring), operator override replaces (does not merge) the default.
- **`docs/spec/2026-05-decision-log.md`** — `## D-54 — sensitive_keys canonical default list`, `## D-55 — UsageExporter push interface (#45 §3)`, `## D-56 — TS _discoverDefault 8-stage refactor` — all marked resolved. Resolution status block updated to reflect D-30, D-31, D-32, D-54, D-55, D-56 as resolved.
- **`docs/features/async-tasks.md` — `Contract: ReaperHandle.stop` block** (audit N-001) — Normative cross-language declaration that `ReaperHandle.stop()` MUST be async in all three SDKs (`async def` in Python, `Promise<void>` in TypeScript, `async fn` in Rust) with drain semantics — cancel + await termination — and `idempotent: true`.
- **`conformance/fixtures/trace_context.json`** (#35) — 8 cross-language test cases for the TraceContext W3C alignment: ordered `tracestate` roundtrip, 32-entry cap, malformed-entry tolerance, case-insensitive `Traceparent`/`TRACESTATE` header lookup, dynamic `trace_flags` honoring on extract→inject, accepted `parent_id` override (`^[0-9a-f]{16}$`), and `INVALID_PARENT_ID` rejection of malformed overrides.
- **`docs/features/event-system.md` — Event Naming Convention section (#36)** — Normative `apcore.<subsystem>.<event>` form for framework-emitted events, glob-subscription examples (`apcore.registry.*` / `apcore.health.*`) across Python/TypeScript/Rust, and a deprecation table mapping legacy names (`module_registered`, `module_unregistered`, `apcore.error.threshold_exceeded`, `apcore.latency.threshold_exceeded`) to their canonical replacements with v0.22.0 removal target. Adds a "Configuration-driven subscribers" section listing all five built-in factories (`webhook`, `a2a`, `file`, `stdout`, `filter`) with a multi-subscriber YAML example.
- **`docs/features/system-modules.md` — Contextual Auditing subsection (#45.2)** — Normative rule that control modules (`update_config`, `toggle_feature`, `reload_module`) MUST include `caller_id` and (when present) a redacted `identity` snapshot in their emitted audit events; `caller_id` defaults to the literal string `"@external"` when unauthenticated; `x-sensitive` Identity fields are replaced with `"<redacted>"`. Cross-language Python/TypeScript/Rust examples of the audit event payload shape.
- Public SubscriberFactory API in Python SDK (parity with TS+Rust) (#36).
- **`conformance/fixtures/event_naming.json`** (#36 / D-34) — 8 cross-language test cases: canonical `apcore.registry.module_registered` / `apcore.registry.module_unregistered`, dual-emit of legacy names with `deprecated:true` during v0.21.x, glob subscription matching for `apcore.registry.*` and `apcore.health.*`, canonical `apcore.health.error_threshold_exceeded` / `apcore.health.latency_threshold_exceeded`, and cross-subsystem glob isolation.
- **`conformance/fixtures/contextual_audit.json`** (#45.2 / D-35) — 7 cross-language test cases: `caller_id` propagation in `apcore.config.updated`, `@external` default for `null`/empty `caller_id`, redacted `identity` snapshot inclusion for `apcore.module.toggled`, `x-sensitive` field redaction (e.g., `bearer_token` → `<redacted>`), `caller_id` + `identity` in `apcore.module.reloaded`, and audit event emission even when no `AuditStore` is configured.
- **`docs/spec/2026-05-decision-log.md` D-34 (event naming canonicalization)** and **D-35 (contextual auditing for control plane)** — both marked resolved with action: rename + dual-emit during v0.21.x for D-34; payload extension for D-35.
- Pipeline `StepMiddleware` extension point in all 3 SDKs (#33). Lifecycle-shaped API (`before_step` / `after_step` / `on_step_error`) mirroring module-level `Middleware` — onion ordering, first-recovery-wins on errors, async support across Python/TypeScript/Rust. Documented in `docs/features/middleware-system.md` "Pipeline Step Middleware (Issue #33)" with three contract blocks. Conformance: `conformance/fixtures/pipeline_step_middleware.json` (6 cases).
- Python `BatchSpanProcessor` for non-blocking span export (#43, parity with TS+Rust). Cross-SDK parity contract — identical default tunables (`max_queue_size=2048`, `max_export_batch_size=512`, `schedule_delay_ms=5000`, `export_timeout_ms=30000`), identical `on_end` / `force_flush` / `shutdown` lifecycle, identical drop-on-full-queue semantics. Documented in `docs/features/observability.md` "Batch span processing" section.
- `docs/spec/2026-05-decision-log.md` — D-36 (Pipeline StepMiddleware), D-37 (Pipeline configuration fail-fast), D-38 (BatchSpanProcessor cross-SDK parity), all resolved.
- `StorageBackend` trait/interface in all 3 SDKs with `InMemoryStorageBackend` default; `ErrorHistory`, `UsageCollector`, `MetricsCollector` accept injected backend (#43).
- `OverridesStore` and `FileOverridesStore` in TypeScript SDK (parity with Python+Rust) (#45.1).
- `Registry.discover_multi_class` / `Registry.discoverMultiClass` method in Python+TS (D-15).
- **`docs/features/observability.md` — `## Pluggable storage backends` and `## ErrorHistory eviction performance` subsections** documenting the cross-SDK `StorageBackend` surface, `InMemoryStorageBackend` default, out-of-tree policy for Redis/Postgres/S3, and the lazy-deletion semantics of the O(log N) min-heap eviction.
- **`docs/features/system-modules.md` — `## Persistent Overrides — pluggable OverridesStore`** subsection covering `OverridesStore` parity across all 3 SDKs, with cross-language `FileOverridesStore` wiring during APCore construction and the missing-path-on-first-run guarantee.
- **`docs/features/multi-module-discovery.md`** — Updated `Registry.discover_multi_class` Contract block to explicitly document that all 3 SDKs expose the discovery routine as a method on `Registry`; added a per-SDK method/internal-helper mapping table and cross-language usage examples (D-15).
- **`docs/features/async-tasks.md`** — Notes documenting the Python `TaskStore.put → save` rename + deprecation alias (D-10), the Python `TaskInfo.attempt_number → retry_count` rename + deprecation property (D-13), removal of Python's `TaskStatus.RETRYING` (D-12), and the Rust `RetryConfig::default().max_retries: 3 → 0` alignment (D-14).
- **`docs/spec/2026-05-decision-log.md` — `## Resolution status — 2026-05-03 addendum`** marking D-10, D-12, D-13, D-14, D-15, D-58, D-25, D-27, and D-28 resolved; added new entries `## D-39 — StorageBackend cross-SDK abstraction` (resolved) and `## D-40 — TS overrides persistence parity` (resolved).
- **`conformance/fixtures/storage_backend.json`** — 5 cross-language test cases covering `StorageBackend` save+get round-trip, list-with-prefix filtering, idempotent delete, namespace isolation, and save-overwrites semantics.
- **`conformance/fixtures/overrides_store.json`** — 5 cross-language test cases covering `OverridesStore`: save persists across reopen, startup applies overrides after base config, in-memory store for tests, missing path on first run is OK, delete idempotency.
- Granular reload via `path_filter` input in `ReloadModule` across all 3 SDKs (#45.4).
- Rust `Config::reload_from_disk()` for refreshing static config without binary restart (#45.5).
- Error fingerprinting in `ErrorHistory` across all 3 SDKs — dedup by (error_code, top-frame hash, sanitized message template) instead of exact message (#43 §4).
- Configurable redaction via `obs.redaction.regex_patterns` and `obs.redaction.sensitive_keys` Config keys (#43 §5).

### Changed

- **Default `obs.redaction.sensitive_keys` redaction list expanded to canonical 16-entry superset across all 3 SDKs** (#43 §5, D-54). Default now ships as `["_secret_*", "password", "passwd", "secret", "token", "api_key", "apikey", "apiKey", "access_key", "private_key", "authorization", "auth", "credential", "cookie", "session", "bearer"]`. The leading `_secret_*` glob preserves the legacy `_secret_`-prefix behavior under the canonical default; the redundant `apiKey`/`apikey` pair is intentional so cross-SDK fixtures byte-match. Operator overrides via `obs.redaction.sensitive_keys` in `apcore.yaml` **replace** (do not merge with) the default. Documented in `docs/features/observability.md` "## Canonical default `sensitive_keys`".
- **Trace context** (#35) — `docs/features/observability.md` W3C Trace Context section adds `tracestate` propagation (ordered, 32-entry cap, malformed-entry tolerance), case-insensitive header lookup (`Traceparent` / `TRACEPARENT` / `traceparent` all match), dynamic `trace_flags` honoring (sampling flag from incoming request, NOT hardcoded), and an optional `parent_id` argument on `inject()` validated against `^[0-9a-f]{16}$` with `INVALID_PARENT_ID` rejection. Cross-language Python/TypeScript/Rust examples added.
- **`docs/features/async-tasks.md` Reaper example** (sync B-004 + audit N-001) — Python tab now uses the canonical sync `start_reaper(ttl_seconds=, sweep_interval_ms=)` signature returning `ReaperHandle`, and `await reaper_handle.stop()` for graceful shutdown (was previously sync `stop()`). TS and Rust tabs unchanged.
- **`docs/features/observability.md` Prometheus example** (sync B-005) — All three tabs show wiring of `UsageCollector` into `PrometheusExporter` (Python `usage_collector=` parameter, TS options-object, Rust `with_usage_collector` builder), with an admonition explaining the cross-language constructor-vs-builder shape difference.
- **`docs/features/multi-module-discovery.md` Python import path** (sync B-001 / B-002) — Corrected `from apcore.discovery import multi_class` to `from apcore import multi_class`, removing an internal contradiction with the rest of the document. The TS tab now references the existing `multiClass()` decorator.
- Event names normalized to `apcore.<subsystem>.<event>` form (#36). Legacy names dual-emitted; removal target v0.22.0.
- Control-plane modules (`update_config`, `toggle_feature`, `reload`) now include `caller_id` + `identity` in audit events (#45.2).
- Pipeline configuration is now fail-fast: missing step references and unmet `requires`/`provides` raise errors instead of logging warnings (#33). `ConfigurationError` is raised at YAML parse time for unknown step names in `pipeline.configure[]` and `pipeline.step_middleware[]`. `PipelineDependencyError` is raised at strategy construction time for unsatisfied capability declarations. Documented in `docs/features/middleware-system.md` "Configuration safety" subsection. Conformance: `conformance/fixtures/pipeline_failfast_config.json` (4 cases).
- `ErrorHistory` eviction is now O(log N) via a min-heap (#43).
- Python `TaskStore.put` → `save` (deprecated alias retained); `TaskInfo.attempt_number` → `retry_count` (deprecated alias retained); `TaskStatus.RETRYING` removed (D-10, D-12, D-13).
- Rust `RetryConfig::default().max_retries` is now `0` (was `3`) (D-14).
- Rust streaming chunk merge raises `STREAM_CHUNK_NOT_OBJECT` for non-object chunks (D-58).
- Rust `update_config` raises `CONFIG_KEY_RESTRICTED` for restricted keys (D-25).
- Rust `UsageCollector` now computes trend from samples and supports period filter (D-27).
- Rust `ContextLogger` output schema aligned with Python+TS (lowercase level, nested `extra`, `module_id`) (D-28).
- Python `start_reaper` now uses `(ttl_seconds, sweep_interval_ms)` and returns `ReaperHandle` matching TS+Rust; old kwargs aliased with DeprecationWarning (D-11).
- `OverridesStore` is now a pluggable trait/interface/protocol in all 3 SDKs (#45.1, D-47); `FileOverridesStore` and `InMemoryOverridesStore` ship as defaults.
- Reaper default `sweep_interval_ms` aligned across 3 SDKs to `300_000` (D-48).
- TypeScript `RetryConfig.computeDelay` → `computeDelayMs`; Rust `RetryConfig::delay_for_attempt` → `compute_delay_ms` (old names deprecated, removal target v0.22.0) (D-08, D-49).
- Rust `inject()` now propagates inbound `trace_flags` from context data (D-50).
- Rust `inject()` now returns an error on malformed `parent_id` override, matching PY+TS (D-51).
- Rust pipeline `ConfigurationError` is a distinct error code from `PipelineDependencyError` (D-52).
- TypeScript redaction reads canonical `obs.redaction.{regex_patterns,sensitive_keys}` Config keys; legacy `observability.redaction.{field_patterns,value_patterns}` honored with deprecation warning (D-53).

### Fixed

- **Cross-language naming alignment for the `context_namespace` middleware module** (audit N-003) — Python and Rust shipped `ContextWriter` / `NamespaceCheck`; TypeScript shipped `ContextKeyWriter` / `ContextKeyValidation`. TypeScript renamed in `apcore-typescript` v0.20.x to match the Python+Rust majority; the prior TS names are retained as deprecated aliases for one release cycle. No spec text change needed — all three SDKs now agree on the canonical names.
- Async `on_error` middleware in Python+TS now detects awaitable/thenable RETURN values rather than inspecting function shape, fixing silent Promise leaks for `partial`-wrapped handlers (#42).

### Documentation

- **D-30: clarified that `pre_approval_hook` is Python-only.** `docs/features/multi-module-discovery.md` adds a "Python-only `pre_approval_hook`" subsection explaining that Python imports the file at scan time so the hook protects against arbitrary code execution; TypeScript and Rust parse static AST/source and never import code from disk for discovery, so the hook is not present in those SDKs. Cross-language tabs show the call shape in each SDK.
- **D-31: documented per-language file extension scanning defaults and skip patterns.** `docs/features/multi-module-discovery.md` adds a "File extensions and skip patterns" subsection with a per-SDK table: Python `.py` (skip `__pycache__/`, `*.pyc`, leading `_`), TypeScript `.ts`/`.js` (skip `*.d.ts`, `*.test.*`, `*.spec.*`), Rust `.rs` only (configurable via `with_extensions`).

### Added — PROTOCOL_SPEC & feature spec hardening (Issues #32–#45)

- **`docs/spec/design-durability-boundary.md` — Durability Boundary design document.** Vendor-neutral architectural document enumerating apcore's stable hooks for retry/replay/workflow layers built on top (Context JSON serialization, Approval Phase B `_approval_token` retry contract, the `TaskStore` interface, the six extension points, retry-safety annotations, `context.data` + §4.6 `x-` extension mechanism), explicit non-goals (mid-pipeline checkpointing, multi-call workflow orchestration, cost governance, first-class fields for application-level concerns apcore does not consume), five concrete integration patterns (transient retry, crash-durable single-call retry, long-running pause for external decisions, cross-process invocation, logical-call deduplication), and a gap watchlist of deliberately-deferred items. Establishes that apcore is the module standard, durable execution is a runtime concern; both can coexist without apcore absorbing workflow surface.

- **`docs/features/system-modules.md` — System Modules Hardening section (Issue #45)** — Five production-hardening extensions: (1) optional `overrides_path` / KV-store persistence for `system.control.update_config` and `system.control.toggle_feature` changes, loaded after base config on startup via pluggable `OverridesStore` interface; (2) contextual audit trail — all state-modifying control modules extract `context.identity` and append structured `AuditEntry` records (timestamp, action, actor_id, actor_type, trace_id, before/after change) via a pluggable `AuditStore` interface; (3) Prometheus integration for `UsageCollector` — five new gauges/counters (`apcore_usage_calls_total`, `apcore_usage_error_rate`, `apcore_usage_p50/p95/p99_latency_ms`) exported via the existing `/metrics` endpoint with configurable `export_timeout_ms`; (4) granular `path_filter` glob field on `system.control.reload_module` for bulk reload in dependency topological order, mutually exclusive with `module_id` (`MODULE_RELOAD_CONFLICT` on conflict); (5) startup failure handling — `fail_on_error: bool = False` parameter for Python/TypeScript `register_sys_modules()`, `failOnError: boolean = false` for TypeScript, and `Result<(), SysModuleError>` return type for Rust (never panics or returns `Option`). Updates `register_sys_modules` contract block with new `fail_on_error` parameter and `SYS_MODULE_REGISTRATION_FAILED` error code. Closes Issue #45.
- **`conformance/fixtures/system_modules_hardening.json`** — 10 cross-language test cases covering: overrides persisted on update_config, overrides loaded after base config on startup, audit entry actor_id extracted from context.identity, audit entry before/after change for toggle_feature, Prometheus /metrics includes apcore_usage_calls_total, path_filter reloads matching modules in topological order, module_id + path_filter conflict raises MODULE_RELOAD_CONFLICT, fail_on_error=True raises SysModuleRegistrationError, fail_on_error=False logs and continues, and Rust register_sys_modules returns Result not Option.

- **`docs/features/observability.md` — Observability Hardening section (Issue #43)** — Six production-hardening extensions: (1) pluggable `ObservabilityStore` interface with `InMemoryObservabilityStore` default and optional `RedisObservabilityStore`/`SqlObservabilityStore` backends injected at construction time; (2) `BatchSpanProcessor` for non-blocking OTEL export with configurable `max_queue_size` (2048), `schedule_delay_ms` (5000), `max_export_batch_size` (512), and `export_timeout_ms` (30000) — drops spans on full queue with `spans_dropped` counter; (3) O(log N) `ErrorHistory` eviction using a min-heap keyed on `last_seen_at` plus an O(1) module-id index, replacing the O(M) ring-buffer scan; (4) SHA-256 content-addressable error fingerprinting with UUID/ID/timestamp normalization for accurate deduplication; (5) `RedactionConfig` with glob `field_patterns` and regex `value_patterns` applied at log time (union with existing `x-sensitive` rules); (6) K8s/Prometheus integration hooks (`/metrics`, `/healthz`, `/readyz`) with required `apcore_module_calls_total`, `apcore_module_errors_total`, `apcore_module_duration_seconds` metrics and ServiceMonitor annotation guidance. Adds `PrometheusExporter.export` contract block. Closes Issue #43.
- **`conformance/fixtures/observability_hardening.json`** — 10 cross-language test cases covering: default InMemoryObservabilityStore construction, BatchSpanProcessor buffering without immediate export, BatchSpanProcessor drop-on-full-queue with `spans_dropped` increment, min-heap eviction of oldest `last_seen_at` entry, fingerprint-based deduplication (count increment, no duplicate entry), UUID normalization producing identical fingerprints, different error codes producing distinct fingerprints, field-pattern redaction (`*password*`), value-pattern redaction (`^Bearer .*`), and Prometheus export containing all three required metric names.

- **`docs/features/async-tasks.md` — AsyncTaskManager Evolution section (Issue #34)** — Three new capability extensions: (1) pluggable `TaskStore` interface with `InMemoryTaskStore` default and optional `RedisTaskStore`/`SqlTaskStore` backends injected at construction time; (2) per-task retry configuration (`max_retries`, `retry_delay_ms`, `backoff_multiplier`, `max_retry_delay_ms`) with exponential backoff scheduling and `FAILED` terminal state after exhaustion; (3) opt-in Reaper background task for automatic TTL-based deletion of terminal-state tasks, guarded by `reaper_enabled: true`. Includes `TaskStore.save` and `AsyncTaskManager.start_reaper` contract blocks. Closes Issue #34.
- **`conformance/fixtures/async_task_evolution.json`** — 10 cross-language test cases covering: default InMemoryTaskStore construction, custom store injection, save-and-get round-trip, list-by-status filtering, retry scheduling on first failure, backoff multiplier delay computation, max-retries-exhausted → FAILED transition, Reaper disabled by default, Reaper deletes expired terminal tasks, and Reaper skips PENDING/RUNNING tasks regardless of age.

- **`docs/features/schema-system.md` — Schema System Hardening section (Issue #44)** — Five hardening areas: (1) union type standardization — all SDKs MUST evaluate ALL branches of `anyOf`/`oneOf` (not short-circuit on first); (2) recursive schema support via lazy `$ref` resolution (`model_rebuild` in Python, `Type.Recursive` in TypeScript, `Box<T>` in Rust); (3) Rust validator parity — normative requirement to support `allOf`/`anyOf`/`oneOf`/`not` and numerical/string constraints, with `jsonschema-rs` as recommended path; (4) semantic format mapping — `SHOULD` map `date-time`, `date`, `time`, `email`, `uri`, `uuid`, `ipv4`, `ipv6` to language-native types; (5) content-addressable schema cache keyed by SHA-256 of canonical JSON — deduplicates identical schemas loaded from different paths. Adds `Schema.validate_union`, `Schema.validate_recursive`, and `Schema.content_hash` contract blocks. Closes Issue #44.
- **`conformance/fixtures/schema_hardening_union.json`** — 8 test cases for anyOf all-branches evaluation, oneOf exactly-one enforcement, oneOf ambiguous-match error, allOf all-satisfied and one-fails cases.
- **`conformance/fixtures/schema_hardening_recursive.json`** — 6 test cases for TreeNode `$ref: "#"` recursive schema at depths 1, 3, and 5; invalid child type; empty and absent children arrays.
- **`conformance/fixtures/schema_hardening_constraints.json`** — 12 test cases for minimum/maximum, exclusiveMinimum boundary, minLength/maxLength, pattern match/no-match, and `not` keyword.
- **`conformance/fixtures/schema_hardening_formats.json`** — 9 test cases for semantic format mapping across all 8 canonical formats; invalid formats produce warn_logged=true, not hard errors.
- **`conformance/fixtures/schema_hardening_cache.json`** — 5 test cases for content-hash deduplication: same-content two-paths, different-content, key-order canonical normalization, pre/post-`$ref` resolution, empty schema.

- **NestContextFactory in `nestjs-apcore` (Issue #35)** — New `NestContextFactory` service (`src/context/nest-context.factory.ts`) reads W3C `traceparent` header (header key normalized to lowercase), `x-correlation-id`/`x-request-id` correlation headers (stored in `context.data["x-correlation-id"]`), and `x-user-id`/`Authorization: Bearer` for identity. 13 unit tests covering all fixture semantics. Exported from package index. This completes the framework-factories task for Issue #35 — Django, Flask, and NestJS all now have default ContextFactory implementations. Also fixes `PROTOCOL_SPEC.md §8.5` error response example (`trace_id` updated from dashed UUID to 32-char lowercase hex, consistent with §5.7 and §10.5 changes shipped in v0.19.0). Closes Issue #35.

- **PROTOCOL_SPEC §5.16 — Pipeline Control Flow Requirements** — Normative rules for fail-fast error handling, O(1) step lookups, replace semantic for step configuration, `run_until` termination predicate, and step-level middleware ordering. Closes Issue #33.
- **`docs/features/core-executor.md` — Pipeline Hardening section** — Implementation guidance for all 5 hardening areas (fail-fast, replace semantic, step-level middleware, `run_until`, O(1) lookups) with cross-language Python/TypeScript/Rust examples and a `Pipeline.configure_step` contract block.
- **`conformance/fixtures/pipeline_hardening.json`** — 5 test cases for fail-fast, continue-on-ignored-error, replace-semantic, `run_until` early termination, and O(1) lookup verification.
- **`docs/features/event-system.md` — Event Management Hardening section** — Cross-language SubscriberFactory parity (TypeScript + Rust examples), three new built-in subscriber types (`file`, `stdout`, `filter`), circuit-breaker resilience spec with `OPEN`/`CLOSED`/`HALF_OPEN` state machine, `apcore.subscriber.circuit_opened`/`circuit_closed` events, `SubscriberCircuitBreaker.on_failure` contract. Closes Issue #36.
- **`conformance/fixtures/event_management_hardening.json`** — 10 test cases for SubscriberFactory registration, built-in subscriber types, filter pass/discard, circuit-breaker state transitions.
- **PROTOCOL_SPEC §2.1.1 — Multi-Class Discovery** — Opt-in mode allowing multiple Module classes per file. ID is derived as `base_id.snake_case(ClassName)`. Single-class files produce unchanged IDs (backward-compatible). Introduces `MODULE_ID_CONFLICT` error for classes that produce the same snake_case segment. Closes Issue #32.
- **`docs/features/multi-module-discovery.md`** — Full feature spec with discovery algorithm, conflict detection, backward compatibility guarantees, and cross-language usage examples.
- **`conformance/fixtures/multi_module_discovery.json`** — 8 test cases for ID derivation, snake_case conversion, conflict detection, and backward compatibility.
- **`docs/features/middleware-system.md` — Middleware Architecture Hardening section** — Context namespacing rules (`_apcore.*` vs `ext.*`), `CircuitBreakerMiddleware` spec (per-module error tracking, rolling window, OPEN/HALF_OPEN/CLOSED state machine, `CircuitBreakerOpenError`, `apcore.circuit.opened`/`closed` events), `TracingMiddleware` spec (OTLP-compatible span lifecycle, graceful no-op when OpenTelemetry is absent), YAML-driven declarative middleware configuration (`tracing`, `circuit_breaker`, `logging`, `custom` types), and async handler detection fix (`inspect.iscoroutinefunction` in Python, `handler.constructor.name` in TypeScript). Includes `Middleware.detect_async` contract block. Closes Issue #42.
- **`conformance/fixtures/middleware_hardening.json`** — 10 test cases covering context namespace validation (valid `_apcore.*`, valid `ext.*`, violation), circuit breaker state transitions (opens at threshold, short-circuits in OPEN, probes in HALF_OPEN, closes on success), tracing span creation and no-op without OTel, and async coroutine detection correctness.

### Changed — PROTOCOL_SPEC clarifications

- **PROTOCOL_SPEC §7 — Approval Phase B Resume semantics clarified.** Added a normative paragraph after the Step 5 Algorithm formalizing existing reference behavior: when a caller retries an `APPROVAL_PENDING` call by injecting `_approval_token` into `arguments`, the executor MUST re-enter the pipeline from Step 1 with no preserved intermediate `PipelineContext` state. Pre-approval middleware side effects re-execute on resume; middleware needing at-most-once semantics across an approval gate SHOULD inspect `_approval_token` itself. No behavioral change — documents existing implementation. Cross-references `docs/spec/design-durability-boundary.md` §2.2.

### Fixed — Documentation polish

- **`docs/features/async-tasks.md` — TypeScript `await` missing on async methods.** The TypeScript tab was calling `manager.submit()`, `manager.cancel()`, and `manager.shutdown()` without `await`, while the Python and Rust equivalents correctly used `await`/`.await`. All three calls are now correctly awaited.
- **`docs/features/event-system.md` — "Via Direct EventEmitter" section missing TypeScript and Rust tabs.** The section contained only bare Python code with no tab wrapper. Added `=== "Python"` / `=== "TypeScript"` / `=== "Rust"` tabs consistent with every other behavioral section in the file.
- **`docs/features/observability.md` — Lowercase `must` in normative Requirements section.** `InMemoryExporter must be bounded` corrected to `InMemoryExporter MUST be bounded` per RFC 2119.
- **`PROTOCOL_SPEC.md §5.16` — RFC 2119 keywords unbolded.** All `MUST`, `MUST NOT`, and `SHOULD` occurrences in the §5.16 Pipeline Control Flow Requirements section (intro sentence + 5 numbered items) were plain text. Updated to `**MUST**` / `**MUST NOT**` / `**SHOULD**` consistent with the rest of the specification.
- **`conformance/README.md` — Non-Standard Test Patterns section incomplete.** `schema_hardening_recursive.json` (shared root-level `schema`) and `schema_hardening_formats.json` (root-level `format_mappings` reference metadata) were not documented alongside the existing `context_serialization.json` and `annotations_extra_round_trip.json` entries. Added entries explaining both patterns for SDK test runner authors.

---

## [0.19.0] - 2026-04-19

### Changed

- **PROTOCOL_SPEC §5.7 — `trace_id` format pattern tightened** from `format: uuid` (ambiguous: dashed vs hex) to `pattern: "^[0-9a-f]{32}$"` (32-char lowercase hex, W3C Trace Context compatible). Resolves internal spec contradiction between §5.7 (`format: uuid`), §10.5 example (dashed UUID), and §10.5 `traceparent_header` (hex without dashes).
- **PROTOCOL_SPEC §10.5 — Trace ID Format section rewritten** to align with W3C Trace Context Level 2. Adds explicit `external_trace_parent_handling` rules: strict 32-hex validation only; all-zero and all-f rejected per W3C; no auto-normalization (dashed UUID stripping, case folding) at `Context.create` — pushed to TraceParent parser or user's ContextFactory. Implementations MUST regenerate + log WARN on invalid input; MUST NOT raise.

### Added
- **PROTOCOL_SPEC §5.15.2 — `DEPENDENCY_VERSION_MISMATCH` error code** (new, non-retryable). Raised when a module in `dependencies.requires` exists but its registered version does not satisfy the declared `version` constraint. For `dependencies.optional` the same situation logs WARN and skips the dependency edge.
- **`conformance/fixtures/dependency_version_constraints.json`** — 15 cross-SDK test cases covering exact match, `>=`/`<=`, range (`>=1.0.0,<2.0.0`), caret (`^1.2.3`, including `^0.2.3` major-zero semantics), tilde (`~1.2.3`), partial-version shortcuts (`"1"` matches `1.x.x`), no-constraint accepts any, and optional-dependency skip-on-mismatch.
- **PROTOCOL_SPEC §5.7 — Note on external correlation IDs.** Documents that existing projects' request/correlation identifiers (`X-Request-ID`, ULID, AWS X-Ray, etc.) SHOULD be preserved in `context.data["x-correlation-id"]` alongside `trace_id`, not used to overwrite it. Establishes the dual-ID model at the spec level.
- **PROTOCOL_SPEC §10.5 — Forward-compatibility Note.** Non-normative acknowledgment that `trace_id` format is a versioned contract, not a permanent guarantee. Future protocol versions MAY introduce alternative formats or structured trace IDs for multi-agent or non-linear trace topologies.
- **`docs/guides/integrating-existing-projects.md`** — New user guide covering Django, Express, and Actix integration patterns for projects already carrying their own request-ID / correlation-ID system.
- **`conformance/fixtures/context_trace_parent.json`** — Cross-language fixture with 10 test cases covering valid 32-hex, dashed UUID rejection, uppercase rejection, W3C-invalid all-zero/all-f rejection, length errors, non-hex characters, empty string, and the no-trace_parent baseline.
- **`docs/spec/DECLARATIVE_CONFIG_SPEC.md` v1.0** — New unified specification for declarative YAML configuration across all three SDKs. Covers bindings YAML (§3), pipeline config (§4), entry-point meta (§5), `auto_schema` semantics (§6), canonical error model with exact message templates (§7), configurable policy limits (§9). Establishes core principles: YAML syntax 100% consistent across SDKs; never silently drop fields; auto-processing as default; JSON Schema for structure, apcore.yaml for policy.
- **`schemas/binding.schema.json` updates** — Relaxed `target` regex to support TypeScript ESM specifiers (`./relative`, `@scope/pkg`) and Rust handler-map keys. `auto_schema` field now accepts boolean or `"true"` / `"permissive"` / `"strict"` enum. Schema mode mutex rules rewritten from `oneOf` to `allOf` + `if/then` (supports implicit auto default when no mode specified). `DisplayOverlay` changed from `additionalProperties: false` to `propertyNames` pattern + `additionalProperties: SurfaceOverride` for future surface extensibility. `Annotations` expanded from 5 to 12 fields (added `streaming`, `cacheable`, `cache_ttl`, `cache_key_fields`, `paginated`, `pagination_style`, `extra`) to align with `module-meta.schema.json`. `version` field pattern moved to configurable policy (`version_require_semver`). Added `spec_version` top-level field.
- **`schemas/apcore-config.schema.json` — `pipeline` and `validation` sections** — New `PipelineConfig` definition with `remove`, `configure`, `steps` structure; `PipelineStep` with `type`/`handler` mutual exclusion, `after`/`before` positioning, and metadata fields (`match_modules`, `ignore_errors`, `pure`, `timeout_ms`). New `ValidationConfig` with configurable policy limits for bindings (`description_max_length`, `documentation_max_length`, `tags_pattern`, `version_require_semver`) and pipeline (`step_name_max_length`, `timeout_ms_max`).
- **`conformance/fixtures/binding_yaml_canonical.yaml`** — Cross-SDK binding YAML conformance fixture with 3 entries testing explicit auto_schema (permissive), explicit input/output schemas with display overlay, and auto_schema strict mode. All three SDKs must parse this fixture identically.
- **`conformance/fixtures/binding_errors.json`** — 6 canonical error message test cases for cross-SDK byte-for-byte message parity (`BindingFileInvalidError`, `BindingSchemaModeConflictError`, `BindingSchemaInferenceFailedError`, `PipelineHandlerNotSupportedError`, `BindingInvalidTargetError`, `BindingModuleNotFoundError`).

### Fixed

- **PROTOCOL_SPEC §8 Error Code Registry** — Added missing `DEPENDENCY_VERSION_MISMATCH` entry alongside `DEPENDENCY_NOT_FOUND`. Previously the behavior was implied by §5.3 version constraint syntax (`^`, `~`, ranges) without a corresponding error code in §8, leaving SDKs without a normative failure mode.
- **SDK compliance with §5.15.2 `DEPENDENCY_NOT_FOUND`** — All three SDKs (`apcore-python`, `apcore-typescript`, `apcore-rust`) now raise the spec-mandated `DEPENDENCY_NOT_FOUND` error code for missing required dependencies. Previously all three raised `MODULE_LOAD_ERROR`, silently diverging from the spec.
- **SDK `CircularDependencyError.details.cycle_path` parity** — Rust SDK now carries `cycle_path` as structured details, matching Python (`details["cycle_path"]`) and TypeScript (`details.cyclePath`). Previously Rust only placed the path in the message string, forcing downstream consumers to parse it.
- **PROTOCOL_SPEC §8.5 — `trace_id` example updated to 32-char lowercase hex.** The error response example still used the old dashed UUID form (`550e8400-e29b-41d4-a716-446655440000`), contradicting the §5.7 and §10.5 tightening shipped in this same release. Updated to `4bf92f3577b34da6a3ce929d0e0e4736`.

### Deprecated

- The pre-existing `MUST NOT allow externally provided unvalidated trace_id` rule in §10.5 is superseded by the explicit validation/normalization pipeline — "unvalidated" is now impossible because every input is either accepted verbatim or replaced with a fresh trace_id.

---

## [0.18.0] - 2026-04-15

> **Breaking changes in this release.** See [`MIGRATION-v0.18.md`](./MIGRATION-v0.18.md) for the consolidated migration guide covering all four repositories.

### Added

- **`APCore` constructor simplified** — removed `config_path` parameter; use `Config.load()` instead (e.g. `APCore(config=Config.load("apcore.yaml"))`). This applies to all SDKs: Python, TypeScript, and Rust. The explicit two-step pattern keeps the constructor focused and avoids mutual-exclusivity validation.
- **System module JSON Schemas** — 6 new output schemas in `schemas/`: `sys-control-reload-module`, `sys-control-toggle-feature`, `sys-control-update-config`, `sys-health-module`, `sys-health-summary`, `sys-manifest-full`. Each declares `$schema`, `$id`, `additionalProperties: false`, and descriptions on every property.
- **Conformance README coverage gap table** — Documents 6 algorithms (A01, A07, A12, A21, A22, A23) that lack conformance fixtures.
- **Conformance README non-standard patterns section** — Documents `sub_cases` and `forbidden_root_keys` test patterns for SDK implementers.
- **PROTOCOL_SPEC §2.7 — `canonical_id` maximum length raised from 128 to 192 characters.** Motivated by deep-namespace languages (Java/.NET/Spring FQN-derived IDs) where snake_case-converted fully-qualified names can exceed 128 in edge cases. 192 is filesystem-safe (`192 + ".binding.yaml" = 205 bytes < 255-byte filename limit on ext4/xfs/NTFS/APFS/btrfs`) and remains within `VARCHAR(255)` for typical persistence layers. MCP alias 64-char hard limit (OpenAI function name spec) is unchanged and still requires alias mapping for any module_id > 64. Schemas updated: `binding.schema.json`, `module-schema.schema.json`, `module-meta.schema.json` declare `module_id.maxLength: 192`; `acl-config.schema.json` `callers`/`targets` pattern strings raised to 192 to remain symmetric. Algorithm A01 (`directory_to_canonical_id`) Step 6 length threshold updated to 192. Conformance test T01-006 boundary updated to `>192 chars`. **Forward-compatible relaxation:** producers targeting mixed-version ecosystems should keep IDs ≤ 128 until all consumers are upgraded.
- **PROTOCOL_SPEC §5.6 yaml block now declares `required_attributes:`** (`input_schema`, `output_schema`, `description`) explicitly. The pseudocode block already marked these as "Required definitions" but the yaml block beneath only listed `required_methods` and `optional_attributes`, so a reader of the yaml alone could not see the attribute requirement. Closes a sync audit contradiction.
- **PROTOCOL_SPEC §4.4.1 — Annotations Extension Field (`extra`) Wire Format** — New normative section defining the canonical on-the-wire shape of `ModuleAnnotations.extra`. Producers MUST serialize as a nested `{"extra": {...}}` object and MUST NOT flatten extension keys to the annotations root. Consumers MUST accept the nested form; legacy top-level overflow keys MAY be tolerated for one MINOR cycle. When both forms appear in the same input, the nested value wins.
- **`extra` field in `Annotations` schema** — `schemas/module-meta.schema.json` now declares `extra` as an object with `additionalProperties: true`. The outer `Annotations` object retains `additionalProperties: false`, so unknown root-level keys are no longer silently accepted at the schema layer.
- **`conformance/fixtures/annotations_extra_round_trip.json`** — 8 cross-language test cases locking the wire format: canonical nested round-trip, empty extra, namespaced keys, Unicode and nested object values, legacy flattened deserialization tolerance, nested-wins precedence, forbidden-root-keys negative case, and dotted-keys-are-not-paths.
- **`MIGRATION-v0.18.md`** — Consolidated migration guide covering all four breaking changes shipped in this release (annotations wire format, apcore-rust Config restructure, apcore-python event alias removal, misc cleanup).
- **8 new feature specification docs.** The following features were implemented in both `apcore-python` and `apcore-typescript` SDKs but had no corresponding feature spec in the protocol repo:
  - `docs/features/error-system.md` — Structured error hierarchy (30+ error types), error codes, AI guidance fields (`retryable`, `ai_guidance`, `user_fixable`, `suggestion`), `ErrorCodeRegistry` for custom module error codes.
  - `docs/features/extension-system.md` — `ExtensionManager` with 6 built-in extension points (`discoverer`, `middleware`, `acl`, `span_exporter`, `module_validator`, `approval_handler`), plugin wiring via `apply()`.
  - `docs/features/call-chain-guard.md` — Algorithm A20: call depth limiting, circular call detection, frequency throttling with configurable thresholds.
  - `docs/features/cancellation.md` — `CancelToken` cooperative cancellation, executor timeout integration with 5-second grace period.
  - `docs/features/async-tasks.md` — `AsyncTaskManager` for background module execution with concurrency semaphore, task lifecycle tracking, and cleanup.
  - `docs/features/streaming.md` — Three-phase streaming pipeline (setup → chunk emission → post-validation), deep merge with depth cap.
  - `docs/features/identity-system.md` — `Identity` data structure with well-known types (`user`, `service`, `ai`, `system`, `anonymous`), `ContextFactory` protocol for web framework integration.
  - `docs/features/apcore-client.md` — `APCore` unified client feature spec covering initialization modes, auto-registration behavior, and method summary.
- **`mkdocs.yml` navigation updated.** Feature Specifications section expanded from 11 to 19 entries (alphabetically sorted).
- **`README.md` Documentation Index updated.** Added all 8 new feature docs to the Feature Specifications table.

### Fixed

- **PROTOCOL_SPEC §10.8 — Span attribute `"caller"` corrected to `"caller_id"`.** The span naming convention section listed `"caller"` as a SHOULD attribute, inconsistent with the `caller_id` field name used everywhere else in the specification (§2.1, §6, §7, Context object). All SDKs already used `caller_id` in their span implementations.
- **PROTOCOL_SPEC §9.3 — RFC 2119 keyword compliance.** Five constraint validation statements in the `A12_validate_config` pseudocode used lowercase `must` instead of uppercase `MUST`. Corrected to match RFC 2119 normative intent.
- **`mkdocs.yml` navigation — 4 orphaned docs now reachable.** Added `api/client-api.md`, `features/config-bus.md`, `features/event-system.md`, and `features/system-modules.md` to the site navigation. These pages existed and were referenced in README.md but were not in the MkDocs nav tree.
- **JSON Schema property descriptions.** Added missing `description` fields to 4 properties in `module-schema.schema.json` (`ErrorSchemaObject.type`, `.properties`, `.required`, and the `description` property definition) and 3 surface-override objects in `binding.schema.json` (`cli`, `mcp`, `a2a`). All properties now comply with the "every property must have a description" rule.
- **`docs/features/streaming.md` — Step count corrected from "Steps 1–6" to "Steps 1–7".** The text listed 7 pipeline phases (Context Creation through Input Validation) but labelled them "Steps 1–6". PROTOCOL_SPEC §6574 explicitly states "Steps 1–7 identical to call()" for `stream()`.
- **`docs/api/executor-api.md` — `validate()` step count corrected.** Previously said "Steps 1-7"; now says "Steps 1-6, plus optional module-level preflight" to match PROTOCOL_SPEC §12.8.
- **`docs/spec/conformance.md` T11-001 — Deprecated `execute_async()` replaced with `call_async()`.** The `execute_async()` method was removed in v0.10; conformance table still referenced the old name.
- **`docs/guides/creating-modules.md` — Rust examples added.** Two tabbed code sections (module definition and module usage) only had Python and TypeScript examples. Added Rust tabs with complete, importable examples using `apcore::{Module, Context, Registry, Executor}`.
- **Cross-language `extra` serialization divergence** — Audit revealed that `apcore-rust` ≤ 0.17.1 used `#[serde(flatten)]` and emitted extension keys at the annotations root, while `apcore-python` and `apcore-typescript` emitted nested `extra` objects. A binding round-tripped through Rust would silently lose the `extra` payload (the nested object collapsed into `extra["extra"]`). All three SDKs are now aligned on the nested form per §4.4.1.
- **Python/TypeScript precedence inversion** — Both SDKs previously merged top-level overflow over explicit nested `extra` (`{**explicit, **overflow}`), making nested values losable. Per §4.4.1 rule 7, nested now wins. Behavior change is observable only when the same key appears in both forms in the same input — a pathological case that no conformant producer emits.
- **`apcore-rust Config` no longer silently ignored spec-conformant YAML.** The struct previously declared executor and observability fields at the root of `Config` instead of nested under `executor` and `observability` namespaces, contradicting PROTOCOL_SPEC §9.1 and the Python/TypeScript SDKs. Loading a YAML file that used the canonical nested form would cause typed fields to remain at default values while the user data ended up in an unused `settings` HashMap entry. **v0.18.0 restructures `apcore-rust Config` to a nested form**, drops the legacy short field names, and rejects v0.17.x-style YAML with a hard error pointing at `MIGRATION-v0.18.md`. See `apcore-rust/CHANGELOG.md` for the full type-by-type rename table.
- **`docs/spec/design-context-annotations-acl.md` no longer contradicts shipped spec.** The historical design document still claimed "ModuleAnnotations is frozen with 11 fields" and recommended `#[serde(flatten)]` for the Rust extra field. A superseded banner now points readers at PROTOCOL_SPEC §4.4.1 for current normative behavior; the original text is preserved for historical context.
- **`mkdocs.yml` Home nav** now points at `README.md` (which exists) instead of `index.md` (which never did), eliminating a noisy mkdocs build warning.
- **PROTOCOL_SPEC §2.1 — Algorithm A01 step 6 and the directory_to_id YAML format block** still said `max_length: 128` after the §2.7 bump to 192. Both updated to 192. Internal contradiction with the §2.7 EBNF constraint and the version history changelog entry resolved.
- **PROTOCOL_SPEC §3868 cross-reference cleanup** — `Self-Evolution: ... runtime reconfiguration (see §6.6, §10)` repointed to `(see §9.11 Hot-Reload, §10 Observability)`. §6.6 is "System Module Permissions" — unrelated to runtime reconfiguration. Stale reference from a §6.x renumbering.
- **`docs/spec/design-execution-pipeline.md:1248`** — Phase 4 implementation table replaced obsolete `VALIDATE_ONLY` preset with the canonical `MINIMAL` preset, matching the canonical 5-preset enumeration at line 680 (`standard, internal, testing, performance, minimal`). VALIDATE_ONLY was replaced by `dry_run` per §4.
- **`docs/spec/design-execution-pipeline.md:1192`** — `validate()` return type table updated to show all three SDKs unified on `PreflightResult` (Rust column previously showed `ValidationResult` with a "long-term plan to unify" note; the unification was completed in this release).
- **`docs/features/approval-system.md` — Wrong pipeline step numbers.** Input Validation was cited as "Step 6" in three places; it is actually Step 7. Middleware Before Chain is Step 6. Corrected all references.
- **`docs/features/event-system.md` — Missing canonical event name prefix.** `error_threshold_exceeded` and `latency_threshold_exceeded` were listed without the `apcore.` prefix. Per PROTOCOL_SPEC §9.16, canonical names are `apcore.error.threshold_exceeded` and `apcore.latency.threshold_exceeded`; the unprefixed forms are legacy aliases. Event Types table corrected.
- **`docs/features/config-bus.md` — Wrong PROTOCOL_SPEC section references.** "Typed Bind (§9.8)" corrected to §9.9.3 (§9.8 is Environment Variable Override). "Hot Reload (§9.9)" corrected to §9.11 (§9.9 is Namespace-Aware Access API).
- **`docs/features/streaming.md` — TypeScript example fixed.** Replaced incorrect `client.module()` with `async function*` (FunctionModule doesn't support streaming) with correct `Module` interface implementation using a `stream()` method. Fixed deep merge table: arrays are replaced, not concatenated.
- **`docs/features/error-system.md` — Reserved prefix list corrected.** Added missing prefixes (`DEPENDENCY_`, `CALL_`, `MIDDLEWARE_`, `VERSION_`, `ERROR_CODE_`), removed incorrect ones (`RELOAD_`, `EXECUTION_`, `ERROR_FORMATTER_`). Added missing error classes (`MiddlewareChainError`, `ErrorCodeCollisionError`, `DependencyNotFoundError`).
- **`docs/features/call-chain-guard.md` — Algorithm description corrected.** Circular detection now accurately describes the prior-chain extraction + last-occurrence check, matching both SDK implementations.
- **`docs/features/cancellation.md` — TypeScript Context example fixed.** `CancelToken` is passed via `new Context()` constructor, not `Context.create()` which doesn't accept it.
- **`docs/features/async-tasks.md` — Type corrections.** `TaskInfo.result` type corrected from `dict[str, Any]` to `Any`. `get_result()` return type corrected from `dict` to `Any`.
- **`docs/concepts.md` — Removed `Module` inheritance.** All `class XModule(Module):` examples changed to `class XModule:` to match the spec requirement "Modules MUST NOT inherit from an ABC."
- **`docs/architecture.md` — Fixed 3 broken links.** `../api/*.md` paths corrected to `./api/*.md` (architecture.md is in docs/, not a subdirectory).
- **`docs/guides/middleware.md` — Removed non-existent `AsyncMiddleware` class.** Replaced with standard `Middleware` subclass with async method overrides, which both SDKs support.
- **`docs/api/executor-api.md` — Sync/async clarity.** `call()` docstring updated from "Synchronously call module" to "Call module (synchronous in Python, async in TypeScript/Rust)."
- **`docs/api/client-api.md` — Sync/async clarity.** Section "4.1 Synchronous Call" renamed to "4.1 Basic Call" with note explaining Python sync vs TypeScript/Rust async.
- **`docs/spec/type-mapping.md` — Go/Java SDK scope clarified.** Added note that Go and Java type mappings are for future implementers; only Python, TypeScript, and Rust have official SDKs.
- **`docs/guides/` — Cross-language notes added.** `acl-configuration.md`, `adapter-development.md`, and `testing-modules.md` now have a note at the top explaining how Python examples apply to TypeScript and Rust.
- **PROTOCOL_SPEC.md RFC 2119 compliance** — 5 instances of lowercase bold `**must**`/`**should**` in normative contexts (§1.6, §4.2, §5.6, §11.6) corrected to uppercase `**MUST**`/`**SHOULD**`.
- **PROTOCOL_SPEC.md §11.6 extension point semantics** — Clarified ambiguous mixed requirement: "Implementations SHOULD support the following extension points. Each supported extension point MUST define a clear interface contract."
- **`docs/spec/algorithms.md` RFC 2119 compliance** — 5 lowercase `must` in A12 config validation constraints (lines 895–899) corrected to uppercase `MUST`.
- **`schemas/sys-control-update-config.schema.json`** — Added missing `type` declarations to `old_value` and `new_value` properties.
- **`schemas/defaults.schema.json`** — Added explicit `additionalProperties: false` to root and all 9 nested objects.
- **`schemas/sys-health-summary.schema.json`** — Added explicit `additionalProperties: false` to root and all 3 nested objects.
- **CHANGELOG.md v0.16.0 date** — Corrected from 2026-04-05 to 2026-04-03 (matching git history).
- **`docs/features/config-bus.md`** — Converted all code examples to cross-language tabbed format (`=== "Python"` / `=== "TypeScript"` / `=== "Rust"`). Added missing Rust tab in Introspection section.
- **Orphaned docs removed from `docs/features/`** — Moved 4 code-forge planning docs (`acl-conditions-redesign.md`, `annotations-redesign.md`, `context-redesign.md`, `overview.md`) out of the published docs tree; these already exist in `planning/`.
- **`docs/spec/design-execution-pipeline.md`** — Merged `design-pipeline-v2.md` content, removed stale legacy alias column.
- **Event naming standardization** — Canonical `apcore.*` prefix enforced across event system docs and identity rule schemas.
- **Multi-language documentation** — Added cross-language examples and tabbed sections to feature docs, API references, and getting-started guide.

### Removed (BREAKING — apcore-python)

- **Legacy event aliases.** `apcore-python` no longer emits `module_health_changed` or `config_changed` alongside the canonical `apcore.module.toggled`, `apcore.health.recovered`, `apcore.config.updated`, and `apcore.module.reloaded` events. The dual-emission deadline was originally v0.16.0 but was missed; this release completes the cleanup. See migration guide §3.

### Removed (BREAKING — apcore-rust)

- **Legacy flat `Config` field names.** `Config.max_call_depth`, `Config.default_timeout_ms`, `Config.global_timeout_ms`, `Config.max_module_repeat`, `Config.enable_tracing`, `Config.enable_metrics` are gone. Use the nested namespaces (`Config.executor.*`, `Config.observability.*`). The string-key API also drops the legacy bare-name aliases — `config.get("max_call_depth")` now returns `None`; use `config.get("executor.max_call_depth")`. See migration guide §2.

---

## [0.17.1] - 2026-04-06

### Added

- **`minimal` strategy preset** — 4-step pipeline (`context_creation` → `module_lookup` → `execute` → `return_result`) for pre-validated internal hot paths. Documented in all three SDKs and spec.
- **Core vs Optional steps overview** — New §4.0 in `design-execution-pipeline.md` with ASCII diagram showing which 4 steps are mandatory and which 7 are removable.
- **Middleware vs Custom Step selection guide** — Decision matrix in `design-execution-pipeline.md` §4.2 and practical guide with cross-language examples in `docs/guides/middleware.md` §11.
- **`requires` / `provides` step dependency metadata** — Optional advisory fields on `BaseStep` (Python), `Step` interface (TypeScript), and `Step` trait (Rust). `ExecutionStrategy` warns at construction and insertion time if a step's `requires` are not satisfied by preceding steps' `provides`.
- **Execute step replacement warning** — `!!! warning` admonition in `design-execution-pipeline.md` explaining risks of replacing the `execute` step.
- **Strategy summary table** — All preset strategies documented with step counts, removed steps, and use cases.

### Fixed

- **PROTOCOL_SPEC.md pipeline order** — Steps 6/7 swapped to match all SDK implementations (`middleware_before` → `input_validation`). Step 2 renamed from "Safety Checks" to "Call Chain Guard".
- **design-execution-pipeline.md alignment** — Step inventory table, code examples, JSON pipeline example, and strategy references updated for correct stage order, naming, and `validate_only` removal.
- **`validate_only` strategy removed from spec** — Never implemented in any SDK; `validate()` method with `dry_run=True` provides the same behavior more cleanly.

---

## [0.17.0] - 2026-04-04

### Added

#### Pipeline v2 — Declarative Step Metadata
- **4 new BaseStep fields** — `match_modules` (glob patterns for selective step execution), `ignore_errors` (fault-tolerant continuation on step failure), `pure` (no side effects, safe for dry-run/validate mode), `timeout_ms` (per-step timeout in milliseconds). Implemented across all three SDKs (Python, TypeScript, Rust).
- **`PipelineContext.dry_run`** — When `true`, PipelineEngine skips steps with `pure=false`, enabling `validate()` to run user-defined pure steps automatically.
- **`PipelineContext.version_hint`** — Passed through to module_lookup for version negotiation.
- **`PipelineContext.executed_middlewares`** — Tracks which middleware ran for on_error recovery chain.
- **`StepTrace.skip_reason`** — Records why a step was skipped: `"no_match"`, `"dry_run"`, or `"error_ignored"`.
- **YAML pipeline configuration** (Python) — `pipeline` section in `apcore.yaml` with `remove`, `configure`, and `steps` directives. Step resolution via `type` (registry) and `handler` (import path).

### Changed

#### Pipeline Step Rename
- **`safety_check` → `call_chain_guard`** — Renamed across all SDKs to accurately describe the step's purpose (call chain depth/cycle/repeat checks, not transport-level rate limiting).
- **TypeScript `builtin.` prefix removed** — All built-in step names in the TypeScript SDK dropped the `builtin.` prefix for cross-SDK consistency (e.g., `builtin.context_creation` → `context_creation`).

#### Pipeline Step Order
- **Steps 6 and 7 swapped** — `middleware_before` now executes before `input_validation` (was the reverse). Middleware transforms are now validated by the subsequent input_validation step, aligning with the Kubernetes Mutating → Validating admission order.

### Fixed
- **Middleware transforms now validated** — Previously, middleware modifications to inputs were either never re-validated (production code) or silently discarded (pipeline abstraction). The step order swap ensures transformed inputs pass schema validation.

---

## [0.16.0] - 2026-04-03

### Added

#### Config Bus Enhancements
- **`env_style` parameter** — Three modes for environment variable key conversion: `auto` (default, matches against `defaults` tree), `nested` (single `_` → `.`), `flat` (no conversion). Resolves flat snake_case config key conflicts.
- **`max_depth` parameter** — Limits nesting depth for env var key conversion (default: 5). Prevents excessively deep nesting from long env var names.
- **`env_prefix` auto-derivation** — When `env_prefix` is not provided, auto-derived from namespace name via `name.upper().replace("-", "_")`.
- **`env_map` parameter** — Explicit mapping of bare (unprefixed) env var names to config keys within a namespace (e.g., `{"REDIS_URL": "cache_url"}`).
- **`Config.env_map()` class method** — Global bare env var → top-level config key mapping (e.g., `{"PORT": "port"}`).
- **`CONFIG_ENV_MAP_CONFLICT` error** — Raised when the same env var is claimed by multiple env_map registrations.

#### Context Redesign
- **`ContextKey<T>` typed accessor** — Generic type-safe wrapper for `context.data` access with `get()`, `set()`, `delete()`, `exists()`, `scoped()` methods. Available in Python, TypeScript, and Rust.
- **Built-in context key constants** — `TRACING_SPANS`, `TRACING_SAMPLED`, `METRICS_STARTS`, `LOGGING_START`, `REDACTED_OUTPUT`, `RETRY_COUNT_BASE` exported for middleware authors.
- **`_context_version` serialization** — Context serialization now includes `_context_version: 1` for forward compatibility. Deserialization warns on unknown versions but proceeds.
- **Context `serialize()` / `deserialize()` methods** — Explicit serialization API with data key filtering (underscore-prefixed keys excluded).

#### Annotations Extension
- **`extra` field on `ModuleAnnotations`** — Free-form extension dictionary for ecosystem packages and user metadata (e.g., `extra={"mcp.category": "tools"}`).
- **`pagination_style` type relaxed** — Changed from `Literal["cursor", "offset", "page"]` to open `string`, allowing custom pagination strategies.
- **`DEFAULT_ANNOTATIONS` constant** — Exported frozen default annotations instance.
- **`from_dict()` classmethod** (Python) — Deserializes annotations with unknown keys captured in `extra`.
- **`createAnnotations()` factory** (TypeScript) — Convenience factory accepting partial overrides.
- **Canonical snake_case wire format** (TypeScript) — `annotationsToJSON()` / `annotationsFromJSON()` for cross-language serialization.

#### ACL Condition Handlers
- **`ACLConditionHandler` protocol** — Extensible condition evaluation interface. Python: sync + async protocols. TypeScript: `boolean | Promise<boolean>`. Rust: `#[async_trait]`.
- **`ACL.register_condition()` class method** — Register custom condition handlers (e.g., `ip_range`, `time_window`).
- **`$or` and `$not` compound operators** — Built-in compound condition handlers for OR and NOT logic in ACL rules.
- **`async_check()` method** — Async ACL check alongside existing sync `check()`, supporting async condition handlers.
- **Fail-closed for unknown conditions** — Unknown condition keys now log a warning and return False (deny), instead of being silently ignored.

#### Execution Pipeline Strategy
- **`Step` protocol / interface / trait** — Pluggable pipeline step with `name`, `description`, `removable`, `replaceable`, and async `execute()`.
- **`ExecutionStrategy` class** — Ordered list of steps with `insert_after()`, `insert_before()`, `remove()`, `replace()` modification API.
- **`PipelineEngine`** — Executes strategy steps with index-based loop, skip_to support, trace accumulation, and abort handling.
- **`PipelineTrace` / `StepTrace`** — Complete execution trace for AI introspection and learning.
- **11 built-in steps** — `BuiltinContextCreation`, `BuiltinSafetyCheck`, `BuiltinModuleLookup`, `BuiltinACLCheck`, `BuiltinApprovalGate`, `BuiltinInputValidation`, `BuiltinMiddlewareBefore`, `BuiltinExecute`, `BuiltinOutputValidation`, `BuiltinMiddlewareAfter`, `BuiltinReturnResult`.
- **Preset strategies** — `build_standard_strategy()` (11 steps), `build_internal_strategy()` (skip ACL/approval), `build_testing_strategy()` (minimal), `build_performance_strategy()` (skip middleware).
- **`Executor.strategy` parameter** — Optional, backward-compatible. When omitted, uses standard 11-step pipeline.
- **`call_with_trace()` / `call_async_with_trace()`** — Returns `(result, PipelineTrace)` tuple for observability.
- **`register_strategy()` / `list_strategies()` / `describe_pipeline()`** — Strategy introspection API.

### Changed

- **Data key naming convention** — Internal middleware keys migrated from legacy names (`_metrics_starts`, `_usage_starts`, `_obs_logging_starts`) to `_apcore.mw.*` convention. All middleware now uses typed `ContextKey` constants.

### Fixed

- **Rust `ApprovalRequest` spec alignment** — Added required `context` field (`Option<Context<Value>>`) and changed `annotations` type from `HashMap` to `ModuleAnnotations` per spec §7.3.1.
- **Rust `DependencyInfo` field rename** — Renamed `name` to `module_id` for cross-SDK consistency with Python/TypeScript.
- **Rust config env fallback** — Fixed namespace-mode `APCORE_*` env var fallback to resolve to top-level paths instead of incorrectly prepending `apcore.` prefix.
- **Rust `config_env` conformance test** — Added missing conformance test (was 9/10, now 10/10 fixtures).
- **Rust Context field alignment** — Removed non-spec fields (`created_at`, `parent_trace_id`, `trace_context`). Changed `global_deadline` from `Option<Instant>` to `Option<f64>` (epoch seconds).
- **Rust Identity immutability** — Fields made private with pub getters. Serde compatibility via `IdentityRaw` pattern.
- **TypeScript `globalDeadline` field** — Added `globalDeadline: number | null` to Context (was missing).
- **Rust system.control module** — Extracted into dedicated `control.rs` file (was inline in `mod.rs`).
- **TypeScript `removeRule` comparison** — Fixed to use element-wise array comparison instead of `JSON.stringify`.
- **Rust empty callers matching** — Empty callers list now matches none (aligned with Python/TypeScript).
- **API documentation audit (13 fixes)** — Corrected Executor constructor (missing `strategy` param), `cache_key_fields` type (tuple not list), `ModuleAnnotations.extra` field, Reserved Words, `extensions_dir` default, `ModuleExample` defaults, Context `serialize()`/`deserialize()`, `_global_deadline`, `preflight()`/`describe()` methods, `PreflightCheckResult.warnings`, `$or`/`$not` ACL examples.
- **Cross-language tabbed examples** — Added TypeScript tab to system-modules.md, converted client-api.md to 3-language tabs, fixed bare code blocks in 4 API docs.

---

## [0.15.1] - 2026-03-31

### Changed

- **Env prefix convention simplified** — Removed the `^APCORE_[A-Z0-9]` reservation rule from namespace registration. Sub-packages now use single-underscore prefixes (`APCORE_MCP`, `APCORE_OBSERVABILITY`, `APCORE_SYS`) instead of the double-underscore form. The longest-prefix-match dispatch algorithm already disambiguates correctly; the previous restriction was unnecessary.
- Built-in namespace env prefixes: `APCORE__OBSERVABILITY` → `APCORE_OBSERVABILITY`, `APCORE__SYS` → `APCORE_SYS`.

---

## [0.15.0] - 2026-03-29

### Added

#### Protocol Specification (v1.6.0-draft)
- **Config Bus Architecture (§9.4)** — apcore.Config upgraded from internal configuration tool to ecosystem-level Config Bus. Any package (apcore ecosystem or third-party) can register a namespace with optional JSON Schema validation, environment variable prefix, and defaults. Design principles: bus not center, zero-cost adoption, gradual integration, cross-language consistency, strict/flexible coexistence
- **Namespace Registration (§9.5)** — `Config.register_namespace(name, schema, env_prefix, defaults)` API with cross-language examples (Python, TypeScript, Rust, Go, Java). Global (class-level) registry shared across Config instances. Late registration permitted with explicit `reload()` to apply. No `unregister_namespace` in this version
- **Unified Configuration File (§9.6)** — Single YAML file with namespace-partitioned sections. Automatic mode detection: legacy mode (no `apcore:` key, fully backward compatible) vs namespace mode (`apcore:` key present). `_config` reserved namespace for meta-configuration (`strict`, `allow_unknown`)
- **Mount Mechanism (§9.7)** — `config.mount(namespace, from_file|from_dict)` for attaching external configuration sources without requiring a unified file. Primary integration path for third-party projects with existing config systems
- **Per-Namespace Env Override (§9.8)** — Each namespace declares its own `env_prefix`. Longest-prefix-match dispatch algorithm resolves ambiguity. `APCORE_MCP` double-underscore convention for apcore sub-packages to avoid collision with `APCORE_` prefix. Compatibility note for apflow's simpler env convention
- **Namespace-Aware Access API (§9.9)** — `config.get("namespace.key.path")` with dot-path namespace resolution algorithm. `config.namespace(name)` for full subtree retrieval. `config.bind(ns, type)` / `config.get_typed(path, type)` for typed access. `Config.registered_namespaces()` for introspection
- **Validation Algorithm A12-NS (§9.10)** — Extended A12 for namespace mode: validates `apcore` namespace with original algorithm, validates registered namespaces against their JSON Schema, handles unknown namespaces per strict/allow_unknown settings
- **Hot-Reload Namespace Support (§9.11)** — `config.reload()` re-reads YAML, re-detects mode, re-applies namespace defaults and env overrides, re-validates, and re-reads mounted files
- **Cross-Language Implementation Requirements (§9.12)** — MUST/SHOULD API surface table, language-idiomatic naming matrix, thread safety requirements, parameter passing style note
- **Ecosystem Integration Patterns (§9.13)** — Convention table for all apcore packages (namespace, env prefix, schema file). Third-party defensive integration pattern. Framework auto-registration examples (Django AppConfig.ready(), FastAPI module-level, NestJS)
- **Config Discovery (§9.14)** — Optional (`MAY`) automatic config file discovery with search order: `$APCORE_CONFIG_FILE` → `./project.yaml` → `./apcore.yaml` → user-level config
- **New error codes** — `CONFIG_NAMESPACE_DUPLICATE`, `CONFIG_NAMESPACE_RESERVED`, `CONFIG_ENV_PREFIX_CONFLICT`, `CONFIG_MOUNT_ERROR`, `CONFIG_BIND_ERROR`

#### Cross-Package Consistency (§8.8, §9.15, §9.16)

Three mechanisms addressing ecosystem consistency across apcore, apcore-mcp, apcore-cli, apcore-a2a and third-party packages:

- **Error Formatter Registry (§8.8)** — Shared `ErrorFormatter` protocol and registration point. apcore-mcp and apcore-a2a each independently implement protocol-specific error mappers (MCP camelCase/sanitization, A2A JSON-RPC code mapping); this registry makes the contract explicit and discoverable. Adoption is SHOULD-level for ecosystem adapters — apcore does not ship adapter-specific formatters. New error code: `ERROR_FORMATTER_DUPLICATE`

- **apcore Built-in Namespace Registrations (§9.15)** — The framework pre-registers two namespaces for its own subsystems, applying the Config Bus pattern to apcore's own internal configuration. Both promote existing flat keys already present in apcore-python's `config.py`; migration is 1:1 with no breaking changes:
  - **`observability`** (`APCORE_OBSERVABILITY`) — Extracts `apcore.observability.*` flat keys (tracing, metrics, logging, error_history, platform_notify) into a dedicated namespace. Adapter packages (apcore-mcp, apcore-a2a, apcore-cli) **should** read from this namespace rather than using independent logging defaults
  - **`sys_modules`** (`APCORE_SYS`) — Promotes `apcore.sys_modules.*` flat keys into a dedicated namespace. `register_sys_modules()` prefers `config.namespace("sys_modules")` in namespace mode with `config.get("sys_modules.*")` legacy fallback

- **Event Type Naming and Collision Fix (§9.16)** — Resolves two confirmed collisions in apcore-python's emitted event types:
  - `"module_health_changed"` was used for two distinct events (toggle on/off vs. error rate recovery); replaced by canonical names `apcore.module.toggled` and `apcore.health.recovered`
  - `"config_changed"` was used for two distinct events (key update vs. module reload); replaced by `apcore.config.updated` and `apcore.module.reloaded`
  - Establishes dot-namespaced naming convention: `apcore.*` reserved for core, `apcore-mcp.*` / `apcore-a2a.*` / `apcore-cli.*` for adapters
  - All four legacy short-form names remain emitted as aliases during transition

---

## [0.14.0] - 2026-03-24

### Fixed

#### Specification Documents
- **`docs/features/event-system.md`** — Fixed severity levels from `warning`/`critical` to `warn`/`fatal`, aligning with PROTOCOL_SPEC §10.2
- **`docs/features/core-executor.md`** — Fixed Identity description: removed incorrect `permissions` field, corrected to `id`, `type`, `roles`, `attrs` per PROTOCOL_SPEC §5.7
- **`docs/features/core-executor.md`** — Added missing `ApprovalPendingError` to Approval Gate description per PROTOCOL_SPEC §7.4
- **`docs/features/middleware-system.md`** — Added `retry.py` to Key Files table (was described in Components but missing from file list)
- **`docs/features/observability.md`** — Added concrete metric names (`apcore_module_calls_total`, `apcore_module_errors_total`, `apcore_module_duration_seconds`) to convenience method documentation

#### Protocol Specification
- Fixed duplicate `### 10.5` section numbering — renumbered Sensitive Data Redaction to §10.6, Sampling Strategy to §10.7, Span Naming Convention to §10.8
- Fixed `### 8.1.1` misnumbered under §9.1 — corrected to §9.1.1
- Fixed `### 10.8.x` misnumbered under §11.8 — corrected to §11.8.1–§11.8.4
- Fixed middleware priority model contradiction between §11.2 (explicit 0-1000) and §12 (registration order) — §12 now aligns with §11.2
- Fixed `on_error` examples in README.md and concepts.md — changed `raise error` to correct return-based contract (`return None` / `return dict`)
- Updated "Last Updated" date to 2026-03-24

#### Scope Document
- Added Approval System (§7) and Event System to "Core Protocol Includes" list in SCOPE.md

#### README
- Added AI-Perceivable brand definition block to README header
- Added "Perceived → Understood → Executed" progression table to "Why AI-Perceivable?" section

### Ecosystem

§5.13 Display Overlay (specified in v0.13.0) is now implemented across the official adapter stack:

| Package | Version | What was implemented |
|---------|---------|----------------------|
| `apcore-toolkit` | 0.4.0 | `DisplayResolver` — §5.13 resolve priority chain, MCP alias sanitization/64-char limit, CLI alias validation, `suggested_alias` fallback, `binding_path` file/directory loading |
| `apcore-cli` | 0.3.0 | CLI command routing from `metadata["display"]["cli"]["alias"]`; descriptor cache; JSON output reads display overlay |
| `apcore-mcp` | 0.11.0 | MCP tool name and description from `metadata["display"]["mcp"]`; `guidance` appended to tool description |
| `apcore-a2a` | 0.3.0 | A2A skill id/description/tags from `metadata["display"]["a2a"]`; removed dead `_build_extensions()` |
| `fastapi-apcore` | 0.4.0 | `binding_path` parameter on `create_cli()` / `create_mcp_server()`; `DeprecationWarning` for `simplify_ids=True` |

- **§5.14 Convention Module Discovery** — new optional protocol capability for zero-decorator module registration via `commands/` directory convention. Supports cross-language function discovery with schema inference from type annotations.

## [0.13.1] - 2026-03-22

### Changed
- Rebrand: aipartnerup → aiperceivable

## [0.13.0] - 2026-03-12

### Added

#### Protocol Specification
- **Caching annotations** (§4.4) — `cacheable`, `cache_ttl`, `cache_key_fields` annotation fields for AI-aware caching decisions
- **Pagination annotations** (§4.4) — `paginated`, `pagination_style` (`cursor`/`offset`/`page`) for paginated result handling
- **AI Metadata Conventions** (§4.6) — 13 standardized `x-` metadata keys across 4 categories: Intent, Planning, Performance/Cost, Trust/Verification
- **`sunset_date`** (§5.2) — ISO 8601 date field for module deprecation lifecycle
- **`on_suspend()` / `on_resume()`** (§5.6, §12.7.3) — Optional lifecycle hooks for state preservation during hot-reload
- **Hot Reload with State Migration** (§12.7.3) — New section with algorithm, constraints, and Python example
- **Ecosystem documentation** — Added apcore-mcp, apcore-a2a, apcore-cli, and apcore-testing to README and SCOPE

#### Schema
- **`module-meta.schema.json`** — Added `streaming`, `cacheable`, `cache_ttl`, `cache_key_fields`, `paginated`, `pagination_style`, `sunset_date` definitions

### Changed
- **Rebranded** from "universal module development framework" to "AI-Perceivable module standard" with three-tier messaging (slogan/subtitle/full definition)
- **SCOPE.md** — Expanded boundary decisions from 17 to 26 rows; updated requirements wording from "The framework" to "Implementations"
- **Section renumbering** — §12.7.3–12.7.8 renumbered after hot-reload insertion
- **Lifecycle table** (§12.7.1) — Reordered to show `on_resume` after `on_load`, with note clarifying old/new instance distinction

### Fixed
- **Cross-references** — Fixed 10+ stale §11.7.x and §12.7.x references across architecture, registry-api, algorithms, and context-object docs
- **Metadata key consistency** — Aligned `x-max-latency-ms` description between README and PROTOCOL_SPEC

---

## [0.12.0] - 2026-03-10

### Added

#### Protocol Specification (v1.5.0-draft)
- **Error catalog expanded** (§8.2) — Added `MODULE_DISABLED`, `EXECUTION_CANCELLED`, `RELOAD_FAILED` error codes with retryability classification and error hierarchy entries
- **UsageCollector formalized** (§10.4) — Added usage tracking specification for `UsageCollector` and `UsageMiddleware` backing `system.usage.*` modules

#### Conformance
- **Shared conformance fixtures** (`conformance/fixtures/`) — 7 JSON fixture files for cross-language testing: `pattern_matching`, `specificity`, `normalize_id`, `call_chain`, `error_codes`, `version_negotiation`, `acl_evaluation`

### Changed

#### Protocol Specification
- **Context.child() naming** (§12.7.2) — Standardized `derive()` → `child()` to match SDK implementations
- **Forward-declared errors resolved** — `GENERAL_NOT_IMPLEMENTED` and `DEPENDENCY_NOT_FOUND` now fully implemented in both SDKs

#### Documentation
- **Cross-reference links** — Added links between API docs and feature docs (executor-api.md, context-object.md)
- **Conformance known deviations** — Updated status of error code implementations

### Fixed
- **CHANGELOG count corrections** — System modules 10→9, APCore client methods 19→17
- **Phantom entry removed** — TypeScript `batchProcessing` annotation (never implemented)

---

## [0.11.0] - 2026-03-09

### Added

#### Documentation — New Files
- **APCore Client API** (`docs/api/client-api.md`) — Full API reference for the unified `APCore` client covering 17 public methods: `call`, `call_async`, `stream`, `validate`, `module`, `register`, `discover`, `list_modules`, `describe`, `use`, `use_before`, `use_after`, `remove`, `on`, `off`, `disable`, `enable`, plus `events`/`registry`/`executor` properties and global `apcore.*` entry points
- **Event System** (`docs/features/event-system.md`) — `EventEmitter`, `ApCoreEvent`, `EventSubscriber` protocol, `WebhookSubscriber` (retry strategy), `A2ASubscriber` (auth modes), subscriber type factory registry, event types table, and YAML configuration reference
- **System Modules** (`docs/features/system-modules.md`) — Complete reference for 9 built-in `system.*` modules with input/output schemas: `system.health.summary`, `system.health.module`, `system.manifest.module`, `system.manifest.full`, `system.usage.summary`, `system.usage.module`, `system.control.update_config`, `system.control.reload_module`, `system.control.toggle_feature`; plus `register_sys_modules()` setup guide and YAML configuration

#### Documentation — Updated Files
- **Observability features** — `ErrorHistory`, `UsageCollector`, `PlatformNotifyMiddleware` documented
- **Middleware guide** — Built-in `RetryMiddleware` + `RetryConfig` reference added
- **Schema system** — `SchemaStrategy` and `ExportProfile` enums documented

### Changed

#### Documentation — Updated Files
- **`docs/getting-started.md`** — Added §8 Global Entry Points (16 `apcore.*` module-level functions) and §9 System Modules quick start with health/usage/manifest examples
- **`docs/api/executor-api.md`** — `validate()` return type updated from `ValidationResult` to `PreflightResult` with 6-check breakdown and `requires_approval` flag; `call()`/`call_async()`/`stream()` signatures updated with `version_hint` parameter; added `ModuleDisabledError`, `ModuleTimeoutError`, `ReloadFailedError`, `FeatureNotImplementedError`, `DependencyNotFoundError` to error types; timeout section rewritten for dual-timeout model with cooperative cancellation (`CancelToken` + 5s grace period)
- **`docs/api/registry-api.md`** — Added `disable()`/`enable()` module toggle, `safe_unregister()` with cooperative drain (Algorithm A21), `acquire()` context manager, `is_draining()`, `describe()` for AI/LLM tool discovery, `negotiate_version()` (Algorithm A14)
- **`docs/features/observability.md`** — Added three subsystems: `ErrorHistory` (ring buffer with deduplication, `ErrorEntry` dataclass), `UsageCollector` (hourly bucketed storage, trend computation, `UsageMiddleware`), `PlatformNotifyMiddleware` (threshold-based alerting with hysteresis)
- **`docs/guides/middleware.md`** — Replaced hand-written `RetryMiddleware` example with built-in `RetryMiddleware` + `RetryConfig` reference (exponential/fixed backoff, jitter, retryable-only); added §5.5–5.7 cross-references for `ErrorHistoryMiddleware`, `UsageMiddleware`, `PlatformNotifyMiddleware`
- **`docs/features/schema-system.md`** — Added `SchemaStrategy` enum (`yaml_first`, `native_first`, `yaml_only`) and `ExportProfile` enum (`mcp`, `openai`, `anthropic`, `generic`)
- **`docs/features/core-executor.md`** — Added dual-timeout model (global deadline + per-module), cooperative cancellation with `CancelToken`, deep merge for streaming (depth cap 32), error propagation via `propagate_error()` (Algorithm A11), `PreflightResult` validation
- **`docs/README.md`** — Added `client-api.md`, `event-system.md`, `system-modules.md` to directory tree, API reference table, feature specifications table, and concept index

---

## [0.10.0] - 2026-03-07

### Added

#### Protocol Specification
- **IDConverter implementation note** (§12.2) — SDKs MAY implement as utility function instead of class
- **MiddlewareManager split-method pattern** (§12.2) — SDKs MAY use `execute_before`/`execute_after`/`execute_on_error` instead of unified `run_chain()`
- **Module.stream() optional method** (§5.6) — Documented `stream()` as optional method in Module interface for streaming support

#### Error Hierarchy
- **`DependencyNotFoundError`** — New error class for `DEPENDENCY_NOT_FOUND` code (previously forward-declared)
- **`FeatureNotImplementedError`** (Python) / **`NotImplementedError`** (TypeScript) — New error class for `GENERAL_NOT_IMPLEMENTED` code (previously forward-declared)

### Changed
- **Executor pipeline docs** — All references updated from "10-step" to "11-step" pipeline across README.md, docs/features/core-executor.md, docs/api/executor-api.md
- **Context `logger` property** — Upgraded from SHOULD to MUST in docs/api/context-object.md (both SDKs already provide it)
- **docs/api/module-interface.md** — Added optional `stream()` method documentation
- **docs/getting-started.md`** — Rewritten to recommend `APCore` unified client as primary approach

---

## [0.9.0] - 2026-03-06

### Added

#### Protocol Specification
- **Executor.validate() preflight** (§12.2) — `[SHOULD]` non-destructive preflight check through Steps 1–6 without invoking module code or middleware; new `PreflightResult` / `PreflightCheckResult` types with duck-type `ValidationResult` compatibility
- **§12.8 Executor.validate() Cross-Language Implementation Guide** — error handling mapping, type mapping for Python/TypeScript/Go/Rust/Java/C/C++, schema library requirements, naming conventions
- **Preflight Tests** added to §12.4 Consistency Test Suite (7 test cases)
- **Context optional extension fields** — `cancel_token`, `services`, `redacted_inputs` with serialization rules (§5.7)
- **New error codes** — `CONFIG_NOT_FOUND`, `CONFIG_INVALID`, `SCHEMA_CIRCULAR_REF`, `BINDING_FILE_INVALID`, `MIDDLEWARE_CHAIN_ERROR`, `VERSION_INCOMPATIBLE`, `ERROR_CODE_COLLISION`, `CIRCULAR_DEPENDENCY`, `DEPENDENCY_NOT_FOUND` (§8)
- **New error classes** — `BindingFileInvalidError`, `MiddlewareChainError`, `VersionIncompatibleError`, `ErrorCodeCollisionError` added to error hierarchy
- **AI error guidance fields** — `retryable`, `ai_guidance`, `user_fixable`, `suggestions` for improved LLM agent error handling
- **AI intent metadata keys** (§4.6) — `x-when-to-use`, `x-when-not-to-use`, `x-common-mistakes`, `x-workflow-hints` conventions for LLM agents
- **TypeScript** and **C/C++** added to §12.6 Language-Specific Implementation Notes

### Changed
- **`PROTOCOL_SPEC.md`** — bumped to v1.4.0-draft
- **Executor pipeline renumbered** from 10 steps (with Step 4.5) to clean 11 steps — Approval Gate is now Step 5, subsequent steps shifted +1
- **§7.4, §7.9, streaming protocol** references updated to match new 11-step numbering
- **Executor.validate() preflight** added to §12.3 cross-language requirements table
- **Section cross-references fixed** — §11.7→§12.7, §10.3→§11.3, §9.7→§10.7, §7→§8 (error code references)
- **Retryability table** updated with new error codes; `MIDDLEWARE_CHAIN_ERROR` removed from forward-declared list
- **`docs/api/context-object.md`** — added optional extension fields documentation
- **`docs/api/executor-api.md`** — added AI error guidance fields and new error types
- **`docs/concepts.md`** — expanded AI collaboration and cognitive interface concepts
- **`SCOPE.md`** — updated to reflect new features

---

## [0.8.0] - 2026-03-03

### Added
- **AI Collaboration Lifecycle** documentation: Integrated `description`, `metadata`, `requires_approval`, and `ai_guidance` into a unified narrative (Discovery, Strategy, Governance, Recovery).
- New "Cognitive Interface" concept in `README.md` and `docs/concepts.md`.
- Intent-oriented design tips in `docs/guides/creating-modules.md`.
- Comprehensive multi-language **"Getting Started" guide** covering both Python and TypeScript side-by-side
- Multi-language support (side-by-side examples) in **"Creating Modules" guide**
- Unified documentation links across all implementation READMEs (`apcore-python`, `apcore-typescript`)
- TypeScript examples for Registry, Executor, and Module definition in core documentation

### Changed
- **`README.md`** — Updated Quick Start with multi-language tabs and links to the new Getting Started guide

---

## [0.7.0] - 2026-03-01

### Added

#### Protocol Specification
- **Approval System (§7)** — new section in `PROTOCOL_SPEC.md` defining the `ApprovalHandler` protocol, `ApprovalRequest`/`ApprovalResult` data types, Executor Step 4.5 integration, error types (`APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `APPROVAL_PENDING`), built-in handlers, protocol bridge handlers, phased implementation (Phase A sync, Phase B async), and conformance levels

#### Feature Documentation
- `docs/features/approval-system.md` — full specification of the Approval System feature

### Changed
- **`PROTOCOL_SPEC.md`** — bumped to v1.3.0-draft; added "Recommended AI Intent Metadata Keys" (§4.6) outlining `x-when-to-use`, `x-when-not-to-use`, `x-common-mistakes`, and `x-workflow-hints` conventions for LLM agents; updated `requires_approval` annotation description to reference runtime enforcement; added approval error codes and error hierarchy; renumbered §7–§13 → §8–§14
- **`docs/api/executor-api.md`** — added `approval_handler` constructor parameter, `ApprovalDeniedError`/`ApprovalTimeoutError` to error types, Step 4.5 to execution flow and state machine
- **`docs/api/module-interface.md`** — updated `requires_approval` annotation description to reference Approval System and runtime enforcement
- **`docs/features/core-executor.md`** — added Step 4.5 (Approval Gate) to the execution pipeline
- **`docs/README.md`** — added Approval System to feature specifications table, directory tree, and concept index

---

## [0.6.0] - 2026-02-23

### Added

#### Protocol Specification
- **Streaming execution protocol** — new section in `PROTOCOL_SPEC.md` defining streaming execution model for long-running or real-time module outputs
- **Module ID constraints & naming conventions** — formal rules for module identifiers added to the protocol spec
- **SDK export requirements** — specified required exports for conformant SDK implementations
- **Module interface improvements** — refined module interface contracts with streaming semantics

#### Feature Documentation
- `docs/features/acl-system.md` — full specification of the Access Control List system
- `docs/features/core-executor.md` — detailed documentation of the execution pipeline
- `docs/features/decorator-bindings.md` — guide on the `@module` decorator and binding mechanics
- `docs/features/middleware-system.md` — composable middleware pipeline specification
- `docs/features/observability.md` — tracing, metrics, and structured logging documentation
- `docs/features/registry-system.md` — module registry and discovery system documentation
- `docs/features/schema-system.md` — schema-driven module input/output validation documentation

#### CI/CD & Site
- GitHub Actions workflow (`.github/workflows/deploy-docs.yml`) for automated documentation deployment
- MkDocs configuration (`mkdocs.yml`) for the documentation site

### Changed

- **README** — enhanced with unified SDK explanation, TypeScript SDK implementation examples, and updated architecture overview
- **SCOPE.md** — updated to reflect current project scope and feature set
- **`docs/concepts.md`** — rewritten with unified SDK explanation for multi-language context
- **`docs/architecture.md`** — updated to align with protocol spec changes
- **`docs/api/`** — updated `context-object.md`, `executor-api.md`, `module-interface.md`, and `registry-api.md` with version-aligned content
- **`docs/spec/conformance.md`** — revised conformance requirements to match 0.2.0 protocol spec
- **`docs/guides/adapter-development.md`** — updated adapter development guide
- **`mkdocs.yml`** — navigation structure updated to include new feature pages

### Removed

- `ROADMAP.md` — removed; roadmap references updated across documentation
- `docs/guides/creating-modules-translated.md` — removed translated guide from navigation

---

## [0.5.0] - 2026-02-22

### Added

#### Protocol Specification
- **Level 2 Conformance (Phase 1)** — Extension system, Async Task Management, and W3C Trace Context support added to protocol requirements
- **Extension System (§12.2)** — Unified extension point framework for pluggable components (discoverers, middleware, ACL, exporters)
- **Async Task Management (§12.7.3)** — Standardized lifecycle for background tasks (Pending, Running, Completed, Failed, Cancelled)
- **Trace Context (§12.7.4)** — W3C Trace Context (traceparent) support for distributed tracing propagation
- **Async Middleware Protocol** — Requirements for non-blocking middleware dispatch

---

## [0.4.0] - 2026-02-20

### Added

#### Protocol Specification
- **Streaming Support** — Formalized `ModuleAnnotations.streaming` and `Executor.stream()` behavior in protocol specification
- **Shallow Merge for Streaming** — Algorithm for accumulating streaming chunks for output validation and post-processing

---

## [0.3.0] - 2026-02-20

### Added

#### Protocol Specification
- **ErrorCodes Catalog** — Standardized error code constants (replaces hardcoded strings)
- **ContextFactory Protocol** — Interface for creating Context from platform-specific requests
- **Registry Constants** — Standardized module ID patterns and event types
- **Comprehensive Schema System** — Formalized schema loading, validation, and multi-profile export (MCP, OpenAI, Anthropic) requirements

---

## [0.2.0] - 2026-02-16

### Changed

#### Protocol Specification
- **Module ID Validation** — Strengthened pattern to `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$` (lowercase, digits, underscores, dots; no hyphens)
- **Registry Event Constants** — Standardized event names for module registration lifecycle

---

## [0.1.0] - 2026-01-01

Initial Release
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
- **Type safety** - Full type annotations across the framework
- **Comprehensive tests** - 90%+ test coverage with unit and integration tests
- **Documentation** - Quick start guide, examples, and API documentation
- **Examples** - Sample modules demonstrating decorator-based and class-based patterns
