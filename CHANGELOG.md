# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added

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
- **`docs/spec/2026-05-decision-log.md` — `## Resolution status — 2026-05-03 addendum`** marking D-10, D-12, D-13, D-14, D-15, D-19, D-25, D-27, and D-28 resolved; added new entries `## D-39 — StorageBackend cross-SDK abstraction` (resolved) and `## D-40 — TS overrides persistence parity` (resolved).
- **`conformance/fixtures/storage_backend.json`** — 5 cross-language test cases covering `StorageBackend` save+get round-trip, list-with-prefix filtering, idempotent delete, namespace isolation, and save-overwrites semantics.
- **`conformance/fixtures/overrides_store.json`** — 5 cross-language test cases covering `OverridesStore`: save persists across reopen, startup applies overrides after base config, in-memory store for tests, missing path on first run is OK, delete idempotency.

### Changed

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
- Rust streaming chunk merge raises `STREAM_CHUNK_NOT_OBJECT` for non-object chunks (D-19).
- Rust `update_config` raises `CONFIG_KEY_RESTRICTED` for restricted keys (D-25).
- Rust `UsageCollector` now computes trend from samples and supports period filter (D-27).
- Rust `ContextLogger` output schema aligned with Python+TS (lowercase level, nested `extra`, `module_id`) (D-28).

### Fixed

- **Cross-language naming alignment for the `context_namespace` middleware module** (audit N-003) — Python and Rust shipped `ContextWriter` / `NamespaceCheck`; TypeScript shipped `ContextKeyWriter` / `ContextKeyValidation`. TypeScript renamed in `apcore-typescript` v0.20.x to match the Python+Rust majority; the prior TS names are retained as deprecated aliases for one release cycle. No spec text change needed — all three SDKs now agree on the canonical names.

---

## [v0.20.0]

### Added

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

### Changed

- **PROTOCOL_SPEC §7 — Approval Phase B Resume semantics clarified.** Added a normative paragraph after the Step 5 Algorithm formalizing existing reference behavior: when a caller retries an `APPROVAL_PENDING` call by injecting `_approval_token` into `arguments`, the executor MUST re-enter the pipeline from Step 1 with no preserved intermediate `PipelineContext` state. Pre-approval middleware side effects re-execute on resume; middleware needing at-most-once semantics across an approval gate SHOULD inspect `_approval_token` itself. No behavioral change — documents existing implementation. Cross-references `docs/spec/design-durability-boundary.md` §2.2.

### Fixed

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
