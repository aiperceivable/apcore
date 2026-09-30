---
description: "Index of every apcore decision ID (D-01–D-131, E-01–E-03, O-1) with its status, the spec version or release that carries it, and a link to its record."
---

# Decision Register

Every design decision recorded against apcore, in one table. A decision record explains *why*
a rule exists; the rule itself is whatever [protocol-spec.md](./protocol-spec.md) and the
feature documents say today. Decision records are history, not guidance; protocol-spec wins on
any conflict.

**Status:** **In force** — resolved, and no later recorded decision replaces it ·
**Amended** — still in force, with a later change named in the cell ·
**Superseded (in part)** — (partly) replaced by the decision named ·
**Resolved as** — an open item that became the decision named · **Proposed** — not adopted.

**Spec / release:** `v1.x.y` is the protocol-spec version that carries the decision; `0.x.y` is
the apcore release whose `CHANGELOG.md` records it, for decisions that predate per-decision spec
versions; — means the record names no version.

Records: [2026-05 log](./2026-05-decision-log.md) (D-01–D-65) ·
[2026-09 audit log](./2026-09-decision-log.md) (E-01–E-03) ·
[config-surface log](./2026-09-config-surface-decisions.md) (D-66–D-73) ·
[deep-chain log](./2026-09-deep-chain-decisions.md) (D-74–D-128, O-1) ·
[divergence log](./2026-09-divergence-decisions.md) (D-129 onward) ·
[CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27)
(D-17–D-24, the executor and async-task hardening decisions, which have no log). Wave-2
decisions D-92–D-107 are recorded as paragraphs, so their links go to the section that holds
them.

| ID | Decision | Status | Spec / release | Record |
|---|---|---|---|---|
| D-01 | `Identity.type` is free-form; well-known values are examples | In force | — | [2026-05 log](./2026-05-decision-log.md#d-01-identitytype-closed-enum-vs-free-form) |
| D-02 | `ApprovalStatus` value is `approved`, not `granted` | In force | — | [2026-05 log](./2026-05-decision-log.md#d-02-approvalstatus-enum-value-granted-vs-approved) |
| D-03 | `ApprovalRequest` carries `caller_id` and `action` | In force | v1.32.0 | [2026-05 log](./2026-05-decision-log.md#d-03-approvalrequest-fields-add-caller_id-and-action) |
| D-04 | Error and audit wire format is snake_case | In force | — | [2026-05 log](./2026-05-decision-log.md#d-04-audit-entry-error-wire-format-snake_case-vs-camelcase-ts-outlier) |
| D-05 | `Module.stream()` falls back to `execute()` | In force | — | [2026-05 log](./2026-05-decision-log.md#d-05-modulestream-fallback-implement-everywhere) |
| D-06 | No global `multi_class_enabled` toggle; per-class opt-in | Superseded by D-107 | — | [2026-05 log](./2026-05-decision-log.md#d-06-multi_class_enabled-config-plumbing) |
| D-07 | The error is `CallFrequencyExceededError` | In force | — | [2026-05 log](./2026-05-decision-log.md#d-07-frequencyexceedederror-vs-callfrequencyexceedederror) |
| D-08 | `RetryConfig.compute_delay_ms` is the canonical name | In force (implemented via D-49) | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-08-retryconfigcompute_delay_ms-method-name-canonicalization) |
| D-09 | No `start()` / `stop()` lifecycle; `close()` is Python-only | In force | — | [2026-05 log](./2026-05-decision-log.md#d-09-apcore-lifecycle-close-vs-start-vs-stop) |
| D-10 | `TaskStore` methods: `save` / `get` / `list` / `delete` / `list_expired` | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-10-taskstore-protocol-method-names-put-vs-save-list_expired-presence) |
| D-11 | `start_reaper(ttl_seconds, sweep_interval_ms)` returns a `ReaperHandle` | In force (implemented via D-42; scope clarified by E-01) | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-11-start_reaper-signature-alignment) |
| D-12 | Drop Python's `TaskStatus.RETRYING` | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-12-taskstatus-enum-drop-pythons-retrying) |
| D-13 | Python `TaskInfo.retry_count` field name | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-13-taskinforetry_count-field-name-in-python) |
| D-14 | Rust `RetryConfig` default `max_retries` is 0 | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-14-retryconfigdefault-in-rust-max_retries3-vs-spec-max_retries0) |
| D-15 | `Registry.discover_multi_class` is a method | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-15-registrydiscover_multi_class-free-function-vs-registry-method) |
| D-16 | Rust `Module.stream` doc signature corrected | In force | — | [2026-05 log](./2026-05-decision-log.md#d-16-modulestream-rust-signature-in-docs-is-wrong) |
| D-17 | `TaskStore` is async in every SDK | In force | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-18 | `cancel()` is a real interrupt; TypeScript uses `AbortController` | In force; refined by D-90 | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-19 | `call_with_trace` shares `call()` error semantics | In force | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-20 | Cancellation short-circuits the `on_error` chain | In force | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-21 | The cancel token is checked at Step 2 and Step 8 | In force | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-22 | `MiddlewareChainError` is unwrapped before propagation | In force | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-23 *(2026-05 log)* | `RefResolver` max-depth integration | In force | — | [2026-05 log](./2026-05-decision-log.md#d-23-refresolver-max_depth-integration) |
| D-23 *(0.22.0)* | `get_status` / `list_tasks` return shallow copies | In force | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-24 *(2026-05 log)* | `update_config` validates constraints and rolls back | In force | — | [2026-05 log](./2026-05-decision-log.md#d-24-update_config-constraint-registry-rollback) |
| D-24 *(0.22.0)* | `Context.create()` takes six caller-supplied fields | In force; refined by D-99–D-103 | 0.22.0 | [CHANGELOG 0.22.0](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md#0220---2026-05-27) |
| D-25 | `update_config` raises `CONFIG_KEY_RESTRICTED` | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-25-update_config-restricted-key-error-code) |
| D-26 | Identity equality is structural; hashability is per language | In force | — | [2026-05 log](./2026-05-decision-log.md#d-26-identity-hashability) |
| D-27 | UsageCollector trend / period filtering / record timestamp | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-27-usagecollector-trend-period-filtering-record-timestamp) |
| D-28 | `ContextLogger` output schema aligned across SDKs | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-28-contextlogger-output-schema-rust-uppercase-flattened-extra) |
| D-29 | `EventEmitter.shutdown` in every SDK | In force | — | [2026-05 log](./2026-05-decision-log.md#d-29-eventemittershutdown-parity-already-partially-fixed) |
| D-30 | `pre_approval_hook` is Python-only | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-30-discover_multi_class-pre_approval_hook-python-only-param) |
| D-31 | Discovery file extensions are per-language defaults | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-31-file-extension-scope-for-discovery) |
| D-32 | Discovery pipeline has eight stages in every SDK | In force (implemented via D-56) | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-32-discovery-pipeline-stage-count-alignment) |
| D-33 | Trace context W3C alignment | In force | — | [2026-05 log](./2026-05-decision-log.md#d-33-trace-context-w3c-alignment) |
| D-34 | Event naming canonicalization | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-34-event-naming-canonicalization) |
| D-35 | Contextual auditing for control plane | In force; refined by D-93, D-118 | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-35-contextual-auditing-for-control-plane) |
| D-36 | Pipeline StepMiddleware | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-36-pipeline-stepmiddleware) |
| D-37 | Pipeline configuration fail-fast | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-37-pipeline-configuration-fail-fast) |
| D-38 | BatchSpanProcessor cross-SDK parity | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-38-batchspanprocessor-cross-sdk-parity) |
| D-39 | StorageBackend cross-SDK abstraction | Amended by D-113 | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-39-storagebackend-cross-sdk-abstraction) |
| D-40 | TS overrides persistence parity | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-40-ts-overrides-persistence-parity) |
| D-41 | Async middleware is detected by return value | In force | — | [2026-05 log](./2026-05-decision-log.md#d-41-async-middleware-correctness-return-value-detection) |
| D-42 | Reaper signature alignment (D-11 closeout) | In force | — | [2026-05 log](./2026-05-decision-log.md#d-42-reaper-signature-alignment-d-11-closeout) |
| D-43 | Granular reload via `path_filter` (Issue #45.4 closeout) | In force; extended by D-111, D-112 | — | [2026-05 log](./2026-05-decision-log.md#d-43-granular-reload-via-path_filter-issue-454-closeout) |
| D-44 | Rust `Config::reload_from_disk()` (Issue #45.5 closeout) | In force | — | [2026-05 log](./2026-05-decision-log.md#d-44-rust-configreload_from_disk-issue-455-closeout) |
| D-45 | Error fingerprinting in ErrorHistory (Issue #43 §4 closeout) | In force | — | [2026-05 log](./2026-05-decision-log.md#d-45-error-fingerprinting-in-errorhistory-issue-43-4-closeout) |
| D-46 | Redaction configuration (Issue #43 §5 closeout) | Amended in v1.37.0 (§10.6.1 matching rules) | — | [2026-05 log](./2026-05-decision-log.md#d-46-redaction-configuration-issue-43-5-closeout) |
| D-47 | OverridesStore cross-SDK trait/interface/protocol parity | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-47-overridesstore-cross-sdk-traitinterfaceprotocol-parity) |
| D-48 | Reaper sweep_interval_ms default alignment | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-48-reaper-sweep_interval_ms-default-alignment) |
| D-49 | D-08 follow-through: rename TS+Rust to canonical `compute_delay_ms` | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-49-d-08-follow-through-rename-tsrust-to-canonical-compute_delay_ms) |
| D-50 | Rust trace inject inbound flag propagation | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-50-rust-trace-inject-inbound-flag-propagation) |
| D-51 | Trace inject malformed parent_id rejection | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-51-trace-inject-malformed-parent_id-rejection) |
| D-52 | Rust pipeline ConfigurationError variant | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-52-rust-pipeline-configurationerror-variant) |
| D-53 | TS redaction Config key alignment | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-53-ts-redaction-config-key-alignment) |
| D-54 | `sensitive_keys` canonical default list | Amended in v1.37.0 (`_secret_*` is an anchored pattern) | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-54-sensitive_keys-canonical-default-list) |
| D-55 | `UsageExporter` push interface (#45 §3) | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-55-usageexporter-push-interface-45-3) |
| D-56 | TS `_discoverDefault` 8-stage refactor (D-32 follow-through) | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-56-ts-_discoverdefault-8-stage-refactor-d-32-follow-through) |
| D-57 | §4.6 reasoning / context / dry-run conventions | In force | 0.21.0 | [2026-05 log](./2026-05-decision-log.md#d-57-46-reasoning-context-dry-run-conventions) |
| D-58 | Stream chunks are objects | In force | 0.20.0 | [2026-05 log](./2026-05-decision-log.md#d-58-stream-chunk-shape-dict-only-vs-any-json) |
| D-59 | `caller_id_for_unknown` is the fixed `@external` | In force | — | [2026-05 log](./2026-05-decision-log.md#d-59-caller_id_for_unknown-configurable) |
| D-60 | ACL rules: insertion order, first match wins (no priority field) | In force | — | [2026-05 log](./2026-05-decision-log.md#d-60-acl-priority-field-implement-deny-wins-at-equal-priority) |
| D-61 | Rust `compute_delay_ms` truncates to whole milliseconds | In force | — | [2026-05 log](./2026-05-decision-log.md#d-61-compute_delay-rounding-for-fractional-values) |
| D-62 | `ExtensionManager.apply()` idempotency | Superseded by D-78 | — | [2026-05 log](./2026-05-decision-log.md#d-62-apply-idempotency-for-extensionmanager) |
| D-63 | `ExtensionManager` `get` / `get_all` / `unregister` in every SDK | In force; refined by D-91, D-108, D-128 | — | [2026-05 log](./2026-05-decision-log.md#d-63-extensionmanagergetget_allunregister-api) |
| D-64 | `acl.root` drives ACL discovery; default `./acl` | In force | 0.25.0 | [2026-05 log](./2026-05-decision-log.md#d-64-aclroot-dead-key-activate-config-driven-acl-discovery-unify-default) |
| D-65 | `include:` cross-file configuration composition | Proposed — RFC not adopted | — | [2026-05 log](./2026-05-decision-log.md#d-65-include-cross-file-configuration-composition) |
| D-66 | ACL audit: the ACL file's `audit:` block is the home | In force | v1.45.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-66-aclaudit-two-declared-homes-one-has-to-go) |
| D-67 | `logging.level` / `logging.format` withdrawn; logging is host-owned | In force | v1.48.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-67-logging-what-replaces-logginglevel-loggingformat-at-v20) |
| D-68 | One tracing configuration surface, five keys wired | In force | v1.44.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-68-the-tracing-configuration-surface-is-declared-in-two-places-that-disagree) |
| D-69 | `_config.allow_unknown` implemented | In force | v1.46.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-69-_configallow_unknown-implement-963s-row-or-withdraw-the-key) |
| D-70 | `extensions.roots` implemented with namespaces | In force | v1.46.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-70-extensionsroots-converge-the-other-two-sdks-or-withdraw) |
| D-71 | `id_map.overrides` wired from config | In force | v1.46.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-71-id_mapoverrides-wire-the-config-key-or-withdraw-it) |
| D-72 | `pipeline.*` wired from config | In force | v1.43.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-72-pipelinesteps-configure-remove-the-largest-inert-surface-and-the-one-with-teeth) |
| D-73 | A declared key must reach its mechanism from a `Config` | In force | v1.47.0 | [config-surface log](./2026-09-config-surface-decisions.md#d-73-the-general-rule-may-a-declared-key-be-reachable-only-through-an-api) |
| D-74 | `Config.get("")` is not an error | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-74-configget-is-not-an-error) |
| D-75 | `Executor.call` must enforce the module-ID length bound at entry | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-75-executorcall-must-enforce-the-module-id-length-bound-at-entry) |
| D-76 | `ContextFactory.create_context(request)` | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-76-contextfactorycreate_contextrequest) |
| D-77 | `Registry.describe` returns a string; a structured override falls through | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-77-registrydescribe-returns-a-string-a-structured-override-falls-through) |
| D-78 | `ExtensionManager.apply` must not drain the store | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-78-extensionmanagerapply-must-not-drain-the-store) |
| D-79 | A stalled topological sort is not a cycle | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-79-a-stalled-topological-sort-is-not-a-cycle) |
| D-80 | The registry event set is closed, and `file_changed` is in it | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-80-the-registry-event-set-is-closed-and-file_changed-is-in-it) |
| D-81 | `TaskStoreError` must reach the caller | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-81-taskstoreerror-must-reach-the-caller) |
| D-82 | `list_tasks` insertion order is normative | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-82-list_tasks-insertion-order-is-normative) |
| D-83 | The published `guard_call_chain` signature is normative | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-83-the-published-guard_call_chain-signature-is-normative) |
| D-84 | A non-positive call-chain limit raises a typed error | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-84-a-non-positive-call-chain-limit-raises-a-typed-error) |
| D-85 | Malformed version constraints must be reportable | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-85-malformed-version-constraints-must-be-reportable) |
| D-86 | `Registry.register` validation order | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-86-registryregister-validation-order) |
| D-87 | The ACL audit-block surface is a language idiom | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-87-the-acl-audit-block-surface-is-a-language-idiom) |
| D-88 | Index-keyed warning dedupe must be cleared on index shift | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-88-index-keyed-warning-dedupe-must-be-cleared-on-index-shift) |
| D-89 | Deprecation warning cadence | Amended in v1.59.0 | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-89-deprecation-warning-cadence) |
| D-90 | `reset()` must not substitute the cancellation handle | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-90-reset-must-not-substitute-the-cancellation-handle) |
| D-91 | A removal method a host cannot call does not satisfy the contract | In force | v1.49.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-91-a-removal-method-a-host-cannot-call-does-not-satisfy-the-contract) |
| D-92 | `TaskStoreError` must exist before it can be raised | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-remaining-adjudications) |
| D-93 | The contextual-audit redaction list is a superset with bare substrings | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#security-defects) |
| D-94 | Symlink confinement runs before the dir/file split | In force; extended by D-127 | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#security-defects) |
| D-95 | A configuration dot-path addresses data, never the host object graph | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#security-defects) |
| D-96 | The approval gate fires on the union of module and descriptor | Superseded in part by D-125 | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#security-defects) |
| D-97 | `FunctionModule` exposes binding-declared annotations through its accessors | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#security-defects) |
| D-98 | `$ref` sibling keys are preserved | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#security-defects) |
| D-99 | `global_deadline` is epoch seconds | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-global_deadline-group-d-99-d-102) |
| D-100 | The `global_deadline` field is the storage | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-global_deadline-group-d-99-d-102) |
| D-101 | The deadline belongs to the call tree | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-global_deadline-group-d-99-d-102) |
| D-102 | A deserialized Context recomputes the deadline unconditionally | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-global_deadline-group-d-99-d-102) |
| D-103 | A null `identity` stays null | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-remaining-adjudications) |
| D-104 | Base document for a local `#/…` reference: file root, then schema node | Amended by D-124 (fallback scoped) | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-remaining-adjudications) |
| D-105 | The executor's ACL step takes the async path | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-remaining-adjudications) |
| D-106 | A p99 beyond the largest bucket is that bucket, not zero | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-remaining-adjudications) |
| D-107 | Per-class markers are the only multi-class opt-in | In force | v1.50.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#the-remaining-adjudications) |
| D-108 | An unknown extension point is an error; an empty one is not | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-108-an-unknown-extension-point-is-an-error-an-empty-one-is-not) |
| D-109 | Only the healthy/degraded boundary is configurable | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-109-only-the-healthydegraded-boundary-is-configurable) |
| D-110 | `project_name` defaults to `"apcore"` | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-110-project_name-defaults-to-apcore) |
| D-111 | A bulk reload audits per module, with a correlation id | In force (audit schema corrected in v1.58.0) | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-111-a-bulk-reload-audits-per-module-with-a-correlation-id) |
| D-112 | A failed reload restores the previous module | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-112-a-failed-reload-restores-the-previous-module) |
| D-113 | Storage-backend namespaces, and the omitted-argument default | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-113-storage-backend-namespaces-and-the-omitted-argument-default) |
| D-114 | `remove()` clears the duplicate-identity entry | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-114-remove-clears-the-duplicate-identity-entry) |
| D-115 | A malformed annotation value is tolerated and dropped | In force (authority line withdrawn 2026-09-17) | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-115-a-malformed-annotation-value-is-tolerated-and-dropped) |
| D-116 | Circuit-breaker events carry the declared subscriber type | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-116-circuit-breaker-events-carry-the-declared-subscriber-type) |
| D-117 | Registered-namespace defaults do not answer for a legacy document | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-117-registered-namespace-defaults-do-not-answer-for-a-legacy-document) |
| D-118 | An empty `roles` list is omitted from the audit snapshot | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-118-an-empty-roles-list-is-omitted-from-the-audit-snapshot) |
| D-119 | `system.*` modules declare `open_world: false` explicitly | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-119-system-modules-declare-open_world-false-explicitly) |
| D-120 | Error timestamps are `Z` with millisecond precision | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-120-error-timestamps-are-z-with-millisecond-precision) |
| D-121 | `reload_dependents` is deprecated for removal | In force | v1.51.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-121-reload_dependents-is-deprecated-for-removal) |
| D-122 | `shutdown()` attempts every cancellation before it reports | In force | v1.52.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-122-shutdown-attempts-every-cancellation-before-it-reports) |
| D-123 | A hot-reloaded module MUST NOT become visible before its `on_load()` has run | In force | v1.52.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-123-a-hot-reloaded-module-must-not-become-visible-before-its-on_load-has-run) |
| D-124 | The node fallback is scoped to its own document | In force | v1.53.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-124-the-node-fallback-is-scoped-to-its-own-document) |
| D-125 | D-96 binds every SDK and every governance reader | In force | v1.54.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-125-d-96-binds-every-sdk-and-every-governance-reader) |
| D-126 | A `version_hint` an implementation does not resolve by MUST NOT be silent | In force | v1.55.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-126-a-version_hint-an-implementation-does-not-resolve-by-must-not-be-silent) |
| D-127 | A symlink is recorded once, under its canonical real path | In force | v1.56.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#o-1-resolved-as-d-127-spec-v1560) |
| D-128 | `unregister` removes by identity, not equality | In force | v1.57.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#d-128-unregister-removes-by-identity-not-equality) |
| D-129 | Providers reach the gate they configure | In force | v1.61.0 | [divergence log](./2026-09-divergence-decisions.md#d-129-providers-reach-the-gate-they-configure) |
| D-130 | `configure` cannot weaken a governance gate | In force | v1.61.0 | [divergence log](./2026-09-divergence-decisions.md#d-130-configure-cannot-weaken-a-governance-gate) |
| D-131 | Built-in logging middleware logs the captured values | In force | v1.61.0 | [divergence log](./2026-09-divergence-decisions.md#d-131-built-in-logging-middleware-logs-the-captured-values) |
| E-01 | `start_reaper` starts synchronously; only `stop()` is awaited | In force | — | [2026-09 audit log](./2026-09-decision-log.md#e-01-asynctaskmanagerstart_reaper-does-the-call-itself-need-to-be-awaited) |
| E-02 | `APCore.discover` is sync in Python, async in TypeScript and Rust | In force | — | [2026-09 audit log](./2026-09-decision-log.md#e-02-apcorediscover-synchronous-in-all-languages-contradicted-two-of-three-implementations) |
| E-03 | `ACL(rules=[…])` validates every rule it is handed | In force | v1.33.0 | [2026-09 audit log](./2026-09-decision-log.md#e-03-aclrules-did-not-re-validate-a-rule-mutated-before-its-first-construction) |
| O-1 | `follow_symlinks` does not reach files in two SDKs | Resolved as D-127 | v1.56.0 | [deep-chain log](./2026-09-deep-chain-decisions.md#o-1-resolved-as-d-127-spec-v1560) |

## Numbering collisions

Some IDs were assigned more than once. Read a citation by its date and subject:

- **D-17 – D-22.** The executor and async-task hardening decisions of release 0.22.0 own these
  numbers (table above). Before 2026-05-26 the same numbers named six 2026-05 log entries,
  which were then renumbered: D-17→**D-59**, D-18→**D-60**, D-19→**D-58**, D-20→**D-61**,
  D-21→**D-62**, D-22→**D-63**. See the log's
  [renumber note](./2026-05-decision-log.md#decision-id-renumber-2026-05-26).
- **D-23 and D-24.** Never renumbered, so each has two meanings, both listed above: the 2026-05
  log's `RefResolver` max depth (D-23) and `update_config` rollback (D-24), and release 0.22.0's
  `get_status` / `list_tasks` copies (D-23) and six-parameter `Context.create()` (D-24).
- **D-24, a third use.** SDK source comments and `features/registry-system.md` cite "D-24" for
  the `Registry.list(visibility=…)` filter that defaults to `["public"]`. No decision record
  matches: neither recorded D-24 is about visibility.
