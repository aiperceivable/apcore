---
description: "Defines apcore's three conformance levels (Level 0 Core, Level 1 Standard, Level 2 Full) with per-level MUST/SHOULD/MAY components, the cross-language fixture suite that verifies them, known deviations, and conformance declaration rules."
---

# apcore — Conformance Definitions

> This document defines conformance levels for apcore SDK implementations, the conformance fixture suite, and the conformance declaration format. [protocol-spec.md](./protocol-spec.md) is the single source of truth; where this page and the protocol spec disagree, the protocol spec wins.

## 1. Overview

### 1.1 Purpose

apcore is implemented in several languages, and every implementation has to behave the same way. This specification defines three progressive conformance levels. Each level lists the components an implementation **MUST**, **SHOULD** and **MAY** provide, and the shared fixture suite (§5, §8) verifies them.

### 1.2 Conformance Level Overview

| Level | Name | Positioning | Applicable Scenarios |
|------|------|------|---------|
| **Level 0** | Core | Minimal viable implementation | Rapid prototyping, embedded, resource-constrained environments |
| **Level 1** | Standard | Production-ready implementation | Most production environments |
| **Level 2** | Full | Full-featured implementation | Enterprise-grade, large-scale distributed scenarios |

### 1.3 Terminology and Keywords

The keywords used in this document follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119), consistent with [protocol-spec §1.5](./protocol-spec.md#15-specification-keywords).

Pipeline steps are referred to by name and number from the standard execution strategy: `context_creation` (1), `call_chain_guard` (2), `module_lookup` (3), `acl_check` (4), `approval_gate` (5), `middleware_before` (6), `input_validation` (7), `execute` (8), `output_validation` (9), `middleware_after` (10), `return_result` (11).

---

## 2. Level 0 — Core Conformance

### 2.1 Overview

Level 0 defines the minimal viable implementation of apcore. SDKs reaching this level can complete module definition, registration, discovery, and basic execution, but do not include access control, approval, middleware, or observability.

### 2.2 Must Implement (MUST)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Module interface** | Module base class/interface, includes `execute()`, `input_schema`, `output_schema`, `description` | protocol-spec §5.6 |
| **Schema validation** | Input/output validation based on JSON Schema Draft 2020-12, supports `type`, `properties`, `required`, `enum`, `$ref` (local references) | protocol-spec §4.2 |
| **Registry** | Module discovery (directory scanning), registration, retrieval (`discover()`, `get()`, `list()`) | protocol-spec §12.2 |
| **Executor** | Runs the pipeline steps `context_creation` (1), `module_lookup` (3), `input_validation` (7), `execute` (8), `output_validation` (9) and `return_result` (11) | protocol-spec §5.16, §12.2 |
| **Directory as ID** | `directory_to_canonical_id()` algorithm implementation, directory path automatically maps to Canonical ID | protocol-spec §2.1 |
| **ID format validation** | EBNF syntax validation for Canonical ID | protocol-spec §2.7 |
| **Bare-name canonicalization** | Public `canonicalize_name()` / `canonicalizeName()` returns a segment or a structured diagnostic without exceptions for string input | protocol-spec §2.2.1 |
| **ID conflict detection** | `detect_id_conflicts()` algorithm, detects duplicate IDs and reserved-word conflicts on the first segment | protocol-spec §2.6 |
| **Basic error handling** | Unified error format (`code`, `message`), framework error codes (MODULE_*, SCHEMA_*, GENERAL_*) | protocol-spec §8.1, §8.2 |
| **Error propagation** | `propagate_error()` algorithm, module errors wrapped as ModuleError | protocol-spec §8.3 |
| **Configuration loading** | `apcore.yaml` loading and validation; every declared key reaches its mechanism | protocol-spec §9.1, §9.1.3 |
| **Schema loading** | YAML Schema file loading and parsing | protocol-spec §4.9 |
| **Scanning algorithm** | `scan_extensions()` directory scanning, including symlink containment on the canonical real path | protocol-spec §3.4, §3.6 |
| **Hidden file filtering** | Ignore hidden files and special directories when scanning | protocol-spec §3.5 |
| **Function-based module definition** | `module()` mechanism, wraps callable as standard module, auto-generates Schema from type annotations | protocol-spec §5.11 |
| **External Schema binding** | YAML binding file loading, target resolution, Schema validation | protocol-spec §5.12 |
| **Type inference Schema generation** | `generate_schema_from_function()` algorithm, generates JSON Schema from function signature | protocol-spec §5.11.4 |

### 2.3 Should Implement (SHOULD)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **ID Map cross-language conversion** | `normalize_to_canonical_id()` algorithm, supports at least two languages | protocol-spec §2.2 |
| **Entry point auto-inference** | `resolve_entry_point()` algorithm | protocol-spec §5.2 |
| **Dependency resolution** | `resolve_dependencies()` topological sort algorithm | protocol-spec §5.3 |
| **Schema version declaration** | `version` field support in Schema files | protocol-spec §4.12 |
| **Environment variable override** | `APCORE_*` environment variables override configuration | protocol-spec §9.2 |
| **Schema $ref cross-file references** | `resolve_ref()` algorithm, supports relative path references | protocol-spec §4.11 |
| **`additionalProperties` validation** | additionalProperties handling in input_schema | protocol-spec §4.2 |

### 2.4 May Implement (MAY)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Module metadata files** | `*_meta.yaml` metadata file loading | protocol-spec §5.2 |
| **Annotations** | Behavior annotations | protocol-spec §4.4 |
| **Examples** | Usage examples | protocol-spec §4.5 |
| **Metadata** | Extension metadata | protocol-spec §4.6 |
| **Version number handling** | Filename version suffix parsing | protocol-spec §2.4 |
| **LLM extension fields** | `x-llm-description`, `x-examples`, etc. | protocol-spec §4.3 |

### 2.5 Level 0 Verification

Every case of every fixture in §8.1 that tests a Level 0 component (§5.3).

---

## 3. Level 1 — Standard Conformance

### 3.1 Overview

Level 1 adds the governance pipeline steps (call-chain guard, ACL, approval), middleware, tracing and structured logging on top of Level 0. SDKs reaching this level meet the needs of most production environments.

### 3.2 Must Implement (MUST)

**All MUST components from Level 0**, plus:

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Governance steps** | The standard strategy contains `call_chain_guard` (2), `acl_check` (4) and `approval_gate` (5). Removing `acl_check` or `approval_gate` through `pipeline.remove` emits a diagnostic; an attached ACL or handler under a strategy without its step is reported through the governance-posture accessor | protocol-spec §5.16, §6.6.3.2, §6.6.5 |
| **Pipeline control flow** | Fail-fast on step error, O(1) step lookup, replace semantics for `configure`, `run_until`, and a configured `pipeline:` section applied to the running strategy | protocol-spec §5.16 |
| **ACL engine** | Permission rule loading, pattern matching (`match_pattern()`), rule evaluation (`evaluate_acl()`) | protocol-spec §6.2, §6.3 |
| **ACL default policy** | `default_effect: deny \| allow` in the ACL file; a missing ACL file attaches no ACL | protocol-spec §6.1, §6.6.3.1 |
| **ACL edge cases** | When caller_id is null, treat as `@external`, empty rules use default policy | protocol-spec §6.5 |
| **Approval gate (Phase A)** | `ApprovalHandler` protocol; the gate fires on the union of every governance source and is skipped with a warning when no handler is configured | protocol-spec §7.2, §7.4, §7.8 |
| **Middleware framework** | `middleware_before` (6) and `middleware_after` (10) run an onion-model chain with `before`, `after` and `on_error` hooks | protocol-spec §11.1, §11.5 |
| **Middleware priority** | Higher numbers execute first | protocol-spec §11.2 |
| **Trace ID** | `trace_id` generation (32-char lowercase hex, W3C Trace Context), propagation (child calls inherit) | protocol-spec §10.5 |
| **Tracing from configuration** | `observability.tracing.enabled: true` installs a tracing middleware built from `observability.tracing.*`, with the four sampling strategies | protocol-spec §10.1.1, §10.7 |
| **Structured logging** | JSON format logs, includes `timestamp`, `level`, `message`, `trace_id`, `module_id` | protocol-spec §10.2 |
| **Sensitive data redaction** | `redact_sensitive()` for `x-sensitive` fields, plus the configured `obs.redaction.*` rules, at log emission and at the input/output capture point | protocol-spec §10.6, §10.6.1 |
| **Context complete implementation** | `trace_id`, `caller_id`, `call_chain`, `executor`, `identity`, `data` | protocol-spec §5.7 |
| **Call-chain guard** | Circular-call detection, depth limit (`executor.max_call_depth`) and repeat limit (`executor.max_module_repeat`) based on `call_chain` | protocol-spec §5.7, §9.1.1; [algorithms.md A20](./algorithms.md#a20-guard_call_chain-call-chain-safety-check) |
| **Error hierarchy system** | Flat error hierarchy under `ModuleError` base class | protocol-spec §8.7 |
| **Custom error codes** | Module custom error code registration and collision detection | protocol-spec §8.4 |
| **ID Map all-language conversion** | Support ID conversion for all five languages | protocol-spec §2.2 |
| **Dependency resolution** | `resolve_dependencies()` topological sort and circular dependency detection | protocol-spec §5.3 |
| **Configuration validation algorithm** | `validate_config()` complete implementation | protocol-spec §9.3 |
| **System read modules** | Ship `system.health.*`, `system.manifest.*` and `system.usage.*` (registered when `sys_modules.enabled` is true) | protocol-spec §6.7 |

### 3.3 Should Implement (SHOULD)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **ACL pattern specificity** | `calculate_specificity()` algorithm | protocol-spec §6.4 |
| **ACL audit** | Audit record per decision, delivered through a callback or the ACL file's `audit:` block | protocol-spec §6.3.1, §6.3.2 |
| **Observability middleware** | Logging, tracing and metrics middleware the host can install | protocol-spec §10.1, §10.2, §10.3 |
| **Span naming** | `apcore.{component}.{operation}` span names | protocol-spec §10.8 |
| **Metrics collection** | Basic counter and histogram metrics | protocol-spec §10.3 |
| **Retry semantics** | Classify error code retryability and reject inappropriate retries | protocol-spec §8.6 |
| **Annotation conflict rules** | YAML takes precedence over code, code takes precedence over defaults | protocol-spec §4.13 |
| **Schema validation error format** | Structured validation errors (path, message, constraint, expected, actual) | protocol-spec §4.14 |
| **Preflight** | `Executor.validate()` runs Steps 1–5 and 7 without executing the module | protocol-spec §12.3, §12.8 |
| **Step-level middleware** | Middleware scoped to individual pipeline steps | protocol-spec §5.16 |

### 3.4 May Implement (MAY)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Middleware code registration** | Runtime dynamic middleware registration | protocol-spec §11.2 |
| **W3C Trace Context** | Distributed tracing standard compliance | protocol-spec §10.5 |
| **Module describe interface** | `describe()` method returns LLM-usable complete description | protocol-spec §5.6 |
| **Symlink following** | `extensions.follow_symlinks: true` with loop detection and containment | protocol-spec §3.4, §3.6 |

### 3.5 Level 1 Verification

All Level 0 verification, plus every case of every fixture in §8.1 that tests a Level 1 component (§5.3).

---

## 4. Level 2 — Full Conformance

### 4.1 Overview

Level 2 adds extension points, async modules, the control-plane system modules, and version management on top of Level 1. Conformance at this level confirms feature coverage; deployment suitability still depends on workload testing, operational controls, and the implementation's support policy.

### 4.2 Must Implement (MUST)

**All MUST components from Level 1**, plus:

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Extension point framework** | Six extension points (`discoverer`, `middleware`, `acl`, `span_exporter`, `module_validator`, `approval_handler`) via `ExtensionManager` with `register()`, `get()`, `get_all()`, `unregister()`, `apply()`, `list_points()`. An unknown point is `GENERAL_INVALID_INPUT`; a known point holding nothing is not an error | protocol-spec §11.3, §11.6 |
| **Extension point chain** | `first_success`, `all`, `fallback` strategies | protocol-spec §11.3 |
| **Extension loading order** | `load_extensions()` algorithm | protocol-spec §11.7 |
| **Async modules** | `submit()`, `get_status()`, `cancel()`, `list_tasks()` via `AsyncTaskManager` | protocol-spec §5.8 |
| **Async state machine** | State transition rules (PENDING → RUNNING → COMPLETED/FAILED/CANCELLED) via `TaskStatus` enum | protocol-spec §5.8 |
| **Middleware state machine** | Complete state transitions (init → before → execute → after → done, with error branches) | protocol-spec §11.5 |
| **Version negotiation** | `negotiate_version()` algorithm | protocol-spec §13.3 |
| **Compatibility matrix** | Backward/forward compatibility rules | protocol-spec §13.5 |
| **Context serialization** | Cross-process Context JSON serialization/deserialization | protocol-spec §5.7 |
| **System control modules** | Ship `system.control.update_config`, `system.control.reload_module` and `system.control.toggle_feature` | protocol-spec §6.7 |
| **Execution strategies** | Strategy presets, custom step insertion and removal, and the `pipeline:` configuration section | protocol-spec §5.16, §6.6.3.2 |

### 4.3 Should Implement (SHOULD)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Schema migration** | `migrate_schema()` algorithm | protocol-spec §13.4 |
| **Module hot loading** | Runtime reload of modules without restart, with safe unregister | protocol-spec §12.7.3, §12.7.4 |
| **OpenTelemetry integration** | OTLP exporter selected by `observability.tracing.exporter` | protocol-spec §10.1.1 |
| **Prometheus metrics export** | Standard metrics format | protocol-spec §10.3 |
| **W3C Trace Context** | `traceparent` header propagation | protocol-spec §10.5 |
| **Approval Phase B and Execution Policy** | `check_approval` / `_approval_token`, `CallbackApprovalHandler`, `ExecutionPolicy` (`gate_destructive`, `strict`) | protocol-spec §7.8, §7.9 |
| **Module isolation** | Process-level or container-level isolation | protocol-spec §5.5 |
| **Multi-version coexistence** | Multiple versions of same module running | protocol-spec §5.4 |
| **Schema deprecation markers** | `x-deprecated` handling and warnings | protocol-spec §4.12 |
| **Async callbacks** | `on_progress`, `on_complete`, `on_error` callbacks | protocol-spec §5.8 |

### 4.4 May Implement (MAY)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Protocol adapters** | MCP / A2A / OpenAI / Anthropic / LangChain mapping | protocol-spec Appendix D |
| **CLI tools** | `init`, `create`, `run` and other developer tools | protocol-spec §12.5 |
| **Schema code generation** | Generate language native types from YAML Schema | protocol-spec §4.9 |
| **Remote module loading** | Load modules from remote services/repositories | protocol-spec §11.3 |
| **Distributed execution** | Cross-process/cross-network module execution | protocol-spec §11.3 |
| **Container-level isolation** | Docker/Wasm sandbox | protocol-spec §5.5 |

### 4.5 Level 2 Verification

All Level 1 verification, plus every case of every fixture in §8.1 that tests a Level 2 component (§5.3).

---

## 5. Conformance Test Suite

The conformance suite is the fixture set in `conformance/fixtures/`, inventoried in §8.1. There is no separate per-level test catalogue: a fixture belongs to the level of the component it tests.

### 5.1 Fixture format

Each fixture is a JSON document `{ "description": "...", "test_cases": [...] }`. Every case carries a stable `id`, its input fields, and the expected outcome — `expected`, or `expected_error` naming a wire error code. Some fixtures use extended patterns (`expected_valid`, `expected_features`, `sub_cases`, a shared root-level `schema`), and many carry a root-level `driver_contract` stating which SDK entry point the driver must call and how to compare results. These are listed in [`conformance/README.md` § Non-Standard Test Patterns](https://github.com/aiperceivable/apcore/blob/main/conformance/README.md#non-standard-test-patterns). `binding_yaml_canonical.yaml` is a binding document read by binding-loader tests, not a case container.

### 5.2 How SDKs run the suite

Each SDK keeps its fixture drivers in its own test suite and runs them in CI against the spec repository, checked out beside the SDK and located as §8.2.1 specifies:

| SDK | Drivers | Runner |
|---|---|---|
| apcore-python | `tests/conformance/test_*.py`, `tests/test_conformance.py` | `pytest` |
| apcore-typescript | `tests/conformance-*.test.ts` | `vitest run` |
| apcore-rust | `tests/*conformance*.rs` | `cargo test` |

A driver loads a fixture, drives each case through the SDK's public API (or the entry point its `driver_contract` names), and asserts the expected outcome — the wire error code, never an SDK-local class name.

This repository's CI guards the seam between fixtures and drivers: every fixture is driven by all three SDKs, every `expected` key is read by some driver, and a clause one SDK skips while another exercises it is recorded. A scheduled mutation sweep checks that each case can actually fail. See [`conformance/README.md` § Guards](https://github.com/aiperceivable/apcore/blob/main/conformance/README.md#guards).

### 5.3 Passing standard

| Standard | Requirement |
|------|------|
| Applicable fixtures | Every fixture that tests a component required at the declared level (MUST rows of that level and all lower levels) |
| Pass rate | **100%** of the cases in every applicable fixture. There is no percentage threshold |
| SHOULD / MAY components | A fixture that tests a SHOULD or MAY component is applicable once the implementation ships that component |
| Skipped cases | Count as failures unless listed in the declaration's `known_deviations` (§6.1) |

---

## 6. Conformance Declaration

### 6.1 Declaration Format

When declaring conformance, implementers **MUST** include the following structured declaration in the project root or documentation:

```yaml
# apcore-conformance.yaml

# Implementation information
implementation:
  name: "apcore-python"              # Implementation name
  version: "0.31.0"                  # Implementation version
  language: "python"                 # Implementation language
  spec_version: "1.60.0"             # Corresponding protocol-spec version
  maintainer: "AI Perceivable"       # Maintainer
  repository: "https://github.com/aiperceivable/apcore-python"  # Repository address

# Conformance declaration
conformance:
  level: 1                           # Declared conformance level (0 | 1 | 2)
  date: "2026-09-30"                 # Declaration date

  # Fixture results against the spec_version above
  fixture_results:
    fixtures: 80                     # Applicable fixtures run
    cases: 982                       # Cases in those fixtures
    passed: 982
    failed: 0
    skipped: 0
    report: "https://github.com/aiperceivable/apcore-python/actions"  # Reproducible CI run

  # Known deviations (if any)
  # Format: - { feature: "...", spec: "§x.y", reason: "...", severity: "minor|major" }
  known_deviations: []

  # Optional feature support
  optional_features:
    - name: "hot_reload"
      supported: true
    - name: "opentelemetry"
      supported: true
    - name: "distributed_execution"
      supported: false
```

### 6.2 Declaration Rules

| Rule | Level | Description |
|------|------|------|
| Declared level **MUST** be consistent with fixture results | **MUST** | Cannot falsely claim conformance level |
| Every applicable fixture case **MUST** pass | **MUST** | Cannot claim a level if any applicable case fails, except a listed deviation |
| Known deviations **MUST** be listed honestly | **MUST** | — |
| Declaration **SHOULD** be accompanied by reproducible test results | **SHOULD** | Such as CI report link |
| Declaration **SHOULD** be regularly updated | **SHOULD** | At least once per minor version |

### 6.3 Conformance Badges

Implementations that pass conformance verification **MAY** use the following badges in documentation:

```text
apcore Conformant — Level 0 (Core)
apcore Conformant — Level 1 (Standard)
apcore Conformant — Level 2 (Full)
```

---

## 7. Known Deviations

The following requirements are not met by the current SDK releases (apcore-python, apcore-typescript and apcore-rust). Each applies to all three unless the row says otherwise. Implementations declaring conformance **MUST** list the ones that apply in `known_deviations`.

| Requirement | Spec reference | Level | Current status |
|---|---|---|---|
| Version negotiation on load | protocol-spec §13.3 | Level 2 MUST | `negotiate_version()` is exported as a function in all three SDKs, but no SDK calls it when loading configuration or schema files. |
| Schema migration | protocol-spec §13.4 | Level 2 SHOULD | `migrate_schema()` is not implemented. |
| Module isolation | protocol-spec §5.5 | Level 2 SHOULD | Process- or container-level isolation is not implemented; modules run in the host process. |
| Multi-version coexistence | protocol-spec §5.4 | Level 2 SHOULD | apcore-python resolves a `version_hint` on `Registry.get`. apcore-typescript accepts the hint, ignores it, and warns that it is deprecated. apcore-rust takes no hint. |

---

## 8. Conformance Test Fixtures

The repository ships **91 cross-language fixture files** under `conformance/fixtures/` covering **1107 test cases**. `conformance-integrity` checks these two numbers, §8.1's per-fixture counts and its Total row against the fixtures themselves. Each fixture is consumed by all three SDK test runners (§5.2). An SDK declaring a conformance level **MUST** pass every fixture whose tested feature is required at that level (§5.3).

### 8.1 Fixture Inventory

| Fixture | Cases | Tested feature |
|---------|------:|----------------|
| [`canonicalize_name`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/canonicalize_name.json) | 32 | Public bare-name normalization: ASCII punctuation repair, underscore preservation, non-throwing diagnostics and length limits (§2.2.1) |
| [`acl_agent_scoping`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_agent_scoping.json) | 19 | Agent-scoped ACL governance: per-agent caller patterns and scoping rules (spec §6) |
| [`allow_unknown_namespaces`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/allow_unknown_namespaces.json) | 8 | `_config.allow_unknown`: drop or store an unregistered namespace (§9.6.3, D-69) |
| [`acl_audit_delivery`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_audit_delivery.json) | 17 | ACL audit delivery: one effective sink, containment, the wire record (§6.3.2, D-66) |
| [`acl_evaluation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_evaluation.json) | 19 | ACL rule evaluation, first-match-wins (spec §6) |
| [`acl_handler_error`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_handler_error.json) | 15 | An unevaluable ACL condition resolves toward refusing access; `handler_error` names the condition path (spec §6.1.1 / §6.1.4 / §6.1.4.1) |
| [`acl_argument_scoped_approval`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_argument_scoped_approval.json) | 25 | Authorization and approval requirement are two orthogonal results; the built-in `arguments` condition scopes a rule to this call; an unevaluable rule's requirement is pending, not discarded (spec §6.1.1/§6.1.6/§6.1.7/§6.1.8/§6.9) |
| [`acl_rule_key_closure`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_rule_key_closure.json) | 10 | ACL rule keys are a closed set; an unknown or reserved key fails the load (spec §6.1) |
| [`acl_effect_value_closure`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_effect_value_closure.json) | 10 | A rule's `effect` value is a closed set at every entry point — file loading, direct construction and runtime insertion; `default_effect` on the same terms (spec §6.1.5) |
| [`acl_pattern_arity`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_pattern_arity.json) | 51 | A `callers` / `targets` pattern array's shape is a closed set at every entry point, plus a validator-only tier for well-formed arrays that match nothing (spec §6.2.1) |
| [`acl_root_discovery`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_root_discovery.json) | 10 | `ACL.discover()` config-driven `acl.root` resolution; missing path MUST attach nothing (spec §6.6.3.1, D-64) |
| [`annotations_extra_round_trip`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/annotations_extra_round_trip.json) | 12 | `ModuleAnnotations.extra` wire-format round-trip (spec §4.4.1) |
| [`approval_gate`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/approval_gate.json) | 8 | Approval gate enforcement at Executor Step 5 (spec §7.4) |
| [`approval_request_fields`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/approval_request_fields.json) | 2 | `ApprovalRequest` carries `caller_id` (read straight off `Context.caller_id` — null on a top-level call, never the `@external` ACL sentinel) and `action` (= `module_id`), populated by the approval gate at Executor Step 5 (spec §7.3.1, D-03) |
| [`async_task_cancellation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/async_task_cancellation.json) | 2 | `AsyncTaskManager.cancel()` real abort via CancelToken (D-18) |
| [`async_task_evolution`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/async_task_evolution.json) | 10 | Pluggable `TaskStore`, retry with backoff |
| [`bindings_dir_resolution`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/bindings_dir_resolution.json) | 13 | Binding-directory resolution: a loader invoked without an explicit directory resolves `bindings.dir` / `bindings.pattern` under §9.2 precedence, an explicit argument wins, and no scan happens at client initialisation (spec §5.12.6) |
| [`binding_errors`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/binding_errors.json) | 6 | Binding error message conformance (protocol-spec §5.12.8) |
| [`binding_file_validation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/binding_file_validation.json) | 9 | Binding files fail loudly: unknown keys, a one-sided schema pair and an invalid `auto_schema` are `BINDING_FILE_INVALID`; an uninferable schema is `BINDING_SCHEMA_INFERENCE_FAILED` in every mode (spec §5.12.2, §5.12.5, D-139) |
| [`call_chain`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/call_chain.json) | 11 | Call-chain safety guard (Algorithm A20) |
| [`config_defaults`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/config_defaults.json) | 18 | Config default values cross-SDK identity (spec §9.1.1) |
| [`config_env`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/config_env.json) | 13 | Env-var → Config path mapping (Algorithm A12-NS, spec §9.8) |
| [`env_prefix_dispatch`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/env_prefix_dispatch.json) | 4 | Namespace-mode env dispatch: `APCORE` reserved for `apcore`, one namespace per variable by longest prefix, unmatched `APCORE_` variables belong to `apcore` (spec §9.8.2, D-146) |
| [`config_key_governance`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/config_key_governance.json) | 8 | Configuration key surface governance (§9.1 / §9.3 / §9.15.3) |
| [`config_path_typed_keys`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/config_path_typed_keys.json) | 8 | Closed set of path-typed configuration keys (§9.2.1) |
| [`config_project_root`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/config_project_root.json) | 14 | The project root that path-typed values resolve against from v2.0 — config-file directory for §9.14 tiers 1-5, CWD for the user-level tiers 6-7 and when no file is found; one case per tier, plus the deprecation-warning condition (spec §9.2.2) |
| [`context_create`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/context_create.json) | 15 | `Context.create()` canonical 6-parameter factory and executor binding |
| [`context_serialization`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/context_serialization.json) | 8 | Context JSON round-trip (spec §5.7) |
| [`context_trace_parent`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/context_trace_parent.json) | 10 | `Context.create` trace_parent input handling (spec §10.5) |
| [`contextual_audit`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/contextual_audit.json) | 10 | Contextual audit trail for control-plane modules (D-35) |
| [`dependency_version_constraints`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/dependency_version_constraints.json) | 18 | Dependency version constraint enforcement (spec §5.3, §5.15.2) |
| [`error_codes`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/error_codes.json) | 19 | Error code collision detection (Algorithm A17, spec §8.4) |
| [`error_details_shape`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/error_details_shape.json) | 5 | `SCHEMA_VALIDATION_ERROR` details items are `{path, keyword, message}` with a JSON Pointer path; `details` keys are snake_case (spec §8.1, D-149) |
| [`ephemeral_modules`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/ephemeral_modules.json) | 7 | `ephemeral.*`: one audit event per register / unregister under the standard bootstrap, the bare ID `ephemeral`, `INVALID_MODULE_ID` rejections (spec §2.5.1, D-148) |
| [`error_fingerprinting`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/error_fingerprinting.json) | 6 | Error fingerprint = SHA-256 of `code:module_id:normalized_message` for ErrorHistory dedup |
| [`error_recovery_metadata`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/error_recovery_metadata.json) | 22 | `retryable` / `ai_guidance` / `user_fixable` / `suggestion` recovery metadata (spec §8) |
| [`error_serialization`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/error_serialization.json) | 2 | `ModuleError.to_dict()` snake_case wire form (spec §8) |
| [`event_delivery_semantics`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/event_delivery_semantics.json) | 6 | Event retry, DLQ and `apcore.event.delivery_failed` (spec §9.16) |
| [`event_management_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/event_management_hardening.json) | 17 | SubscriberFactory parity, built-in subscribers |
| [`event_naming`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/event_naming.json) | 7 | Event-name canonicalization (spec §9.16, D-34) |
| [`executor_trace_cancellation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/executor_trace_cancellation.json) | 1 | `call_with_trace()` cancellation short-circuit (D-19 / D-20) |
| [`gate_provider_binding`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/gate_provider_binding.json) | 9 | Governance providers reach the gate they configure: an ACL, handler or policy given to the executor is enforced by the running built-in gate however the strategy was supplied, and `governance_state()` reports what the gate holds (spec §6.6.5.5, D-129) |
| [`json_input_native_types`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/json_input_native_types.json) | 4 | A natively typed contract accepts the JSON strings its schema accepts — date-time, UUID, enum — and still rejects what the schema rejects (type-mapping §17.3, D-136) |
| [`gate_step_configure`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/gate_step_configure.json) | 8 | `configure` cannot weaken `acl_check` / `approval_gate`: `ignore_errors: true`, `match_modules` and `pure: true` are rejected at load (spec §5.16.1, D-130) |
| [`governance_state`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/governance_state.json) | 13 | `Executor.governance_state()` — configured vs. actually wired (spec §6.6.5) |
| [`identity_system`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/identity_system.json) | 8 | Identity construction and propagation (spec §5.7) |
| [`multi_root_discovery`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/multi_root_discovery.json) | 6 | `extensions.roots` multi-root discovery with namespace isolation (§9.1.1, D-70) |
| [`middleware_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/middleware_hardening.json) | 11 | Context namespacing, CircuitBreaker |
| [`middleware_on_error_recovery`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/middleware_on_error_recovery.json) | 4 | Middleware after-chain error recovery |
| [`multi_module_discovery`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/multi_module_discovery.json) | 10 | Multi-class discovery, snake_case conversion, conflict detection; only marked classes receive IDs (spec §2.1.1, D-147) |
| [`normalize_id`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/normalize_id.json) | 23 | ID normalization (Algorithm A02): ASCII case conversion preserves existing underscores and rejects invalid source identifiers without repair (#122) |
| [`id_map_from_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/id_map_from_config.json) | 7 | `id_map.overrides` reaches the ID-map mechanism (§9.1.1, D-71); `file` is relative to the extension root, an empty env value falls through (§2.2, D-138) |
| [`id_conflict_reserved_words`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/id_conflict_reserved_words.json) | 9 | Reserved-word ID conflicts on the first segment only; later segments unrestricted (spec §2.6 step 2) |
| [`observability_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/observability_hardening.json) | 10 | Pluggable storage, BatchSpan, OTel parity |
| [`openai_strict_compat`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/openai_strict_compat.json) | 30 | OpenAI structured-outputs strict-mode incompatibility detection (protocol-spec §5.12.5) |
| [`overrides_store`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/overrides_store.json) | 5 | OverridesStore pluggable persistence |
| [`pattern_matching`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pattern_matching.json) | 12 | ACL / `match_modules` module-ID pattern matching (Algorithm A08) |
| [`extension_point_lookup`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/extension_point_lookup.json) | 12 | An unknown extension point is an error, an empty one is not (D-108) |
| [`export_profiles`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/export_profiles.json) | 5 | Export profiles: `anthropic` strips every `x-*` keyword and keeps properties named `x-…`; `mcp` carries `requiresApproval` / `streaming` in `_meta`, never in `annotations` (spec §4.17, Appendix D.1, D-140) |
| [`glob_matching`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/glob_matching.json) | 30 | Portable glob matching for pattern-valued values (Algorithm A25, §9.2.3) |
| [`pipeline_failfast_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_failfast_config.json) | 7 | Pipeline configuration fail-fast (spec §5.16) |
| [`pipeline_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_hardening.json) | 7 | Pipeline execution hardening: fail-fast, replace-step, run_until, a step timeout raises `MODULE_TIMEOUT` (spec §5.16, D-142) |
| [`pipeline_section_wiring`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_section_wiring.json) | 6 | A configured `pipeline:` section reaches the running strategy (§5.16 requirements 6 and 7, D-72) |
| [`pipeline_step_middleware`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_step_middleware.json) | 9 | Pipeline StepMiddleware lifecycle (spec §5.16 requirement 5) |
| [`preflight_disclosure`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/preflight_disclosure.json) | 4 | `validate()` withholds `preflight()` / `preview()` from an ACL-denied caller (spec §12.8.5.1) |
| [`preflight_check_reporting`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/preflight_check_reporting.json) | 7 | `validate()` reports every check, passed and failed; preview() returning null adds no check; `predicted_changes` is always present (spec §12.8, D-134, D-141) |
| [`redaction_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/redaction_config.json) | 19 | Redaction config via `obs.redaction.regex_patterns` / `sensitive_keys` (spec §10.6.1); `x-sensitive` inside `anyOf` / `oneOf` / `allOf` (spec §10.6, D-152) |
| [`registry_load_ordering`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/registry_load_ordering.json) | 4 | Discovery load order and dependency topological sort (Algorithm A07) |
| [`reload_path_filter`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/reload_path_filter.json) | 10 | Granular reload via `path_filter` glob (`system.control.reload_module`) |
| [`schema_content_hash`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_content_hash.json) | 5 | Schema content-hash cache key — key-order invariant (spec §4) |
| [`schema_export_envelope`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_export_envelope.json) | 5 | `Registry.export_schema` envelope parity (§4.16) |
| [`schema_hardening_cache`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_cache.json) | 5 | Content-addressable schema cache (SHA-256 of canonical JSON) |
| [`schema_hardening_constraints`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_constraints.json) | 12 | Numerical constraints (minimum/maximum/exclusive*, multipleOf) |
| [`schema_hardening_formats`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_formats.json) | 9 | `format` values (date-time, date, email, uri, uuid, ipv4, ipv6) are annotations and never fail validation ([type-mapping §11](./type-mapping.md#11-format-constraint-mappings)) |
| [`schema_hardening_recursive`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_recursive.json) | 6 | Recursive schema support (`$ref` self-reference, depth 1–5) |
| [`schema_hardening_union`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_union.json) | 8 | Union type evaluation (anyOf / oneOf / allOf) |
| [`schema_keyword_parity`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_keyword_parity.json) | 122 | JSON Schema 2020-12 keyword conformance at the validation boundary ([type-mapping §17](./type-mapping.md#17-validation-keyword-conformance)) |
| [`schema_strict_conversion`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_strict_conversion.json) | 16 | `to_strict_schema()` output parity (Algorithm A23 / spec §4.16) |
| [`schema_validation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_validation.json) | 22 | Schema validation edge cases (spec §4.15) |
| [`sensitive_keys_default`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/sensitive_keys_default.json) | 4 | Canonical default `obs.redaction.sensitive_keys` list (D-54) |
| [`specificity`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/specificity.json) | 10 | ACL pattern specificity scoring (Algorithm A10) |
| [`storage_backend`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/storage_backend.json) | 7 | StorageBackend pluggable persistence (shared by ErrorHistory) |
| [`stream_aggregation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/stream_aggregation.json) | 10 | Stream chunk aggregation (recursive deep merge, Algorithm A24) |
| [`system_modules_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/system_modules_hardening.json) | 16 | System modules hardening: persistence, audit, Prometheus (spec §6.7) |
| [`toggle_state_isolation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/toggle_state_isolation.json) | 4 | Per-instance `ToggleState` isolation |
| [`timeout_cancellation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/timeout_cancellation.json) | 6 | A timeout cancels the timed-out call's own token and returns at once; every call gets a child token of its caller's (spec §12.7.5, A22, D-133) |
| [`tracing_from_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/tracing_from_config.json) | 12 | `observability.tracing.*` reaches the running middleware (§10.1.1, D-68) |
| [`trace_context`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/trace_context.json) | 8 | W3C TraceContext alignment (spec §10.5) |
| [`usage_contract`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/usage_contract.json) | 11 | `system.usage.*` value semantics no schema can assert (spec §6.7.1) |
| [`usage_exporter`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/usage_exporter.json) | 3 | `UsageExporter` push interface (D-55) |
| [`version_negotiation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/version_negotiation.json) | 10 | Version negotiation (Algorithm A14) |
| **Total** | **1107** | **91 fixtures** |

### 8.2 Loading Fixtures from a Test Runner

=== "Python"

    ```python title="apcore-python — pytest example"
    import json
    import pathlib

    FIXTURES = pathlib.Path("conformance/fixtures")


    def load_fixture(name: str) -> dict:
        return json.loads((FIXTURES / f"{name}.json").read_text())


    def test_acl_evaluation():
        fixture = load_fixture("acl_evaluation")
        for case in fixture["test_cases"]:
            # ... evaluate `case` against your ACL implementation,
            # asserting the case's `expected` outcome
            ...
    ```

=== "TypeScript"

    ```typescript title="apcore-js — vitest example"
    import * as fs from 'node:fs';
    import * as path from 'node:path';
    import { test } from 'vitest';

    const FIXTURES = 'conformance/fixtures';

    interface Fixture {
      description: string;
      test_cases: Array<Record<string, unknown>>;
    }

    function loadFixture(name: string): Fixture {
      return JSON.parse(
        fs.readFileSync(path.join(FIXTURES, `${name}.json`), 'utf-8'),
      );
    }

    test('acl_evaluation', () => {
      const fixture = loadFixture('acl_evaluation');
      for (const c of fixture.test_cases) {
        // ... evaluate `c` against your ACL implementation,
        // asserting the case's `expected` outcome
      }
    });
    ```

=== "Rust"

    ```rust title="apcore-rust — cargo test example"
    use std::fs;
    use std::path::PathBuf;

    use serde_json::Value;

    fn load_fixture(name: &str) -> Value {
        let path: PathBuf = ["conformance", "fixtures", &format!("{}.json", name)]
            .iter()
            .collect();
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn acl_evaluation() {
        let fixture = load_fixture("acl_evaluation");
        for case in fixture["test_cases"].as_array().unwrap() {
            // ... evaluate `case` against your ACL implementation,
            // asserting the case's `expected` outcome
            let _ = case;
        }
    }
    ```

#### 8.2.1 Locating the fixtures (normative)

The examples above hardcode a relative path for brevity. No SDK can: the fixtures live in the **spec repo**, the SDK lives in its own repo, and CI checks the spec repo out somewhere the SDK cannot guess. Every SDK therefore resolves the directory at run time, and because all three do it, the resolution order is a **cross-SDK contract** rather than three private conventions.

An SDK's conformance runner **MUST** resolve `conformance/fixtures/` in this order, taking the first that exists:

| # | Source | Names | Meaning |
|---|---|---|---|
| 1 | Environment | `CONFORMANCE_FIXTURES` | A `conformance/fixtures` **directory**, used directly |
| 2 | Environment | `CONFORMANCE_SPEC_REPO` | The spec repo **root**; `conformance/fixtures` is appended |
| 3 | Filesystem | — | `../apcore/conformance/fixtures` beside the SDK repo |

1. `CONFORMANCE_FIXTURES` **MUST** take precedence over `CONFORMANCE_SPEC_REPO`. It names a directory of fixture files with no repository around it, which is what makes it useful: a driver can be run against a **synthesised** fixture set — an older shape, a single edited case, a mutation — without producing a whole spec repo to hold it.
2. A variable that is set but does not resolve **MUST** fail loudly, naming the variable **that was actually set**. Falling through to the next source would silently test against different fixtures than the operator named.
3. `APCORE_FIXTURES` and `APCORE_SPEC_REPO` are **transitional** fallbacks for 1 and 2. They are not the canonical names and **MUST NOT** be documented to users: protocol-spec §9.2 makes every `APCORE_*` variable a configuration override, so a test locator under that prefix leaks into the configuration document. A test locator is infrastructure, not configuration.
4. Resolution for **other** spec-repo subdirectories — `schemas/`, most importantly — **MUST NOT** consult `CONFORMANCE_FIXTURES`. It names one directory, not a repo, so there is nothing to append to.

!!! warning "Drivers land before fixtures, so a driver MUST tolerate the older fixture"

    A new fixture turns CI red in all three SDK repositories until every driver exists, so the landing order is **drivers first, fixture last** (§8.3). A driver therefore runs against a fixture that predates the keys it reads, and **MUST** degrade rather than fail: an absent expectation key means "this case does not pin that property", never "compare against nothing".

    This cannot be verified from a working tree, which already holds the newer fixture — it is exactly what `CONFORMANCE_FIXTURES` is for. Point it at a copy with the new keys removed and the suite **MUST** still pass.

    A fixture that pins a **shared constant** rather than new behaviour inverts the order: it **MUST** land *before* the SDKs. `acl_rule_key_closure.json` carries the closed ACL rule-key set, so an SDK that adds a key before the fixture lists it goes red against the set it is meant to agree with.

### 8.3 Adding a New Fixture

1. Create `conformance/fixtures/<name>.json` with `{ "description": "...", "test_cases": [...] }`.
2. Each case **MUST** carry a stable `id`, the input fields, and the `expected` outcome (`{ "expected": ... }` or `{ "expected_error": "<CODE>" }`).
3. Use canonical terminology: `caller_id` / `target_id` (never bare `caller` / `target`).
4. Add a row to the table in §8.1 with its case count, update the Total row and the file and case counts in §8's opening sentence, and add a row to `conformance/README.md`. `conformance-integrity` fails until all of them match.
5. Land drivers in all three SDKs, respecting the landing order in §8.2.1.
6. Reference the fixture from the relevant spec section so the bidirectional traceability is maintained.

---

## 9. References

- [protocol-spec §12 — SDK Implementation Guide](./protocol-spec.md#12-sdk-implementation-guide)
- [protocol-spec §12.4 — Consistency Testing Requirements](./protocol-spec.md#124-consistency-testing-requirements)
- [protocol-spec §1.5 — Specification Keywords](./protocol-spec.md#15-specification-keywords)
- [conformance/README.md](https://github.com/aiperceivable/apcore/blob/main/conformance/README.md) — fixture catalogue, non-standard patterns, guards
- [RFC 2119 — Key words for use in RFCs to Indicate Requirement Levels](https://www.rfc-editor.org/rfc/rfc2119)
