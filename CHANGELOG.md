# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.16.0] - 2026-04-05

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
