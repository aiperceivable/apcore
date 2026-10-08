---
description: "Decision record for the specification/SDK divergences found by the 2026-09-30 documentation audit (D-129 onward): security fixes, spec corrections, and SDK alignment."
title: Divergence decisions (D-129 onward, 2026-09)
date: 2026-09-30
status: in progress — phases 1 (security) and 2 (spec corrections) done
audience: maintainers + spec reviewers
---

# Divergence decisions (D-129 onward)

> **Historical decision record.** Current behaviour is defined by [protocol-spec.md](./protocol-spec.md); decision status is tracked in [decision-register.md](./decision-register.md).

The 2026-09-30 rewrite of the documentation checked every page against the three SDKs and found places where the specification and the implementations disagree. Each was classified before anything changed:

- **Security** — a gate that can silently stop gating. Fixed first, in all three SDKs.
- **Spec error** — the three SDKs agree and are right; the specification is corrected.
- **SDK divergence** — the SDKs disagree; the correct behaviour is chosen and the minority SDKs change, pinned by a fixture.
- **Design question** — the SDKs agree on behaviour that is questionable; decided on its merits.

The order of precedence when deciding which side is right: security, then the specification's intent, then whether the rule can be implemented in all three languages, then consistency with existing implementations. Three SDKs agreeing is evidence, not proof.

## Phase 1 — security (spec v1.61.0)

### D-129 — Providers reach the gate they configure

**Problem.** An ACL, `ApprovalHandler` or `ExecutionPolicy` given to an executor was bound into the built-in gate steps only when the executor built the strategy itself. A pre-built strategy instance received none of them: with a deny-all ACL the call ran, while `governance_state()` reported `acl_configured` and `builtin_acl_gate_wired` as `true` — the combination §6.6.5.2 exists to make trustworthy. Reproduced in apcore-python and apcore-typescript. apcore-rust was unaffected: its gate steps hold no provider and read the executor's on every call.

**Decision.** A provider given to an executor, by constructor or setter, is bound into the corresponding built-in gate step of the running strategy however that strategy was supplied, and replaces the provider the step held; an executor given no provider leaves the step alone. Replacing the strategy rebinds. `governance_state()` reports what the running built-in gate holds. Spec §6.6.5.1, §6.6.5.5.

**Pinned by** `conformance/fixtures/gate_provider_binding.json`.

### D-130 — `configure` cannot weaken a governance gate

**Problem.** `pipeline.configure` accepted `ignore_errors: true` on `acl_check`, which turns an ACL denial into a warning and runs the call; `match_modules` on either gate exempted every module it did not match; `pure: true` on `approval_gate` would make `validate()` consult the `ApprovalHandler` during a dry run. Only `remove:` warned (§5.16 requirement 7), although removing a gate is visible in `governance_state()` and weakening one is not.

**Decision.** On `acl_check` and `approval_gate`, `ignore_errors: true` and any `match_modules` are rejected, and so is `pure: true` on `approval_gate` (`acl_check` is pure by default, since `validate()` runs it), with `PIPELINE_CONFIGURATION_ERROR`, from configuration and from programmatic step configuration alike. `timeout_ms` stays configurable; writing a default value is accepted. Spec §5.16.1.

**Pinned by** `conformance/fixtures/gate_step_configure.json`.

### D-131 — Built-in logging middleware logs the captured values

**Problem.** A built-in logging middleware that logs the raw `inputs` / `output` it is handed bypasses the `x-sensitive` rule: in apcore-rust, `ObsLoggingMiddleware` wrote a field marked `x-sensitive` in plain text unless a `RedactionConfig` happened to list it, apcore-python's `ObsLoggingMiddleware.after` logged the raw output, and in apcore-typescript the JSON-Schema-to-TypeBox conversion dropped every `x-` keyword, so `x-sensitive` was ignored at the capture point for any module declared with plain JSON Schema.

**Decision.** Built-in logging middleware logs `context.redacted_inputs` and `context.redacted_output`, never the raw values; an `x-sensitive` field never appears unredacted, with or without a redaction configuration. Spec §10.6.1 requirement 5.

**Pinned by** per-SDK tests (log formats differ by language).

## Phase 2 — spec corrections (spec v1.62.0)

### D-132 — The specification follows three agreeing, correct implementations

**Problem.** In about fifty places the specification described behaviour that none of the three SDKs has, and the SDKs agreed with each other on behaviour that is correct. Several were MUSTs no implementation could satisfy — a reserved-word rule applied to every segment, a `__` ban, two entry-point error codes that exist in no registry, a "MUST reuse `generate_schema_from_function`" that TypeScript and Rust cannot meet, an `env_prefix` rejection that would refuse the built-in namespaces, a "MUST ignore unknown configuration fields" that contradicts `_config.strict`, a duplicate-registration error code, output re-validation after `after()` middleware, and "timeout in `before()`" rows.

**Decision.** Where all three SDKs agree and the behaviour is safe and implementable, the specification is corrected to it. Every correction was checked against all three SDK sources. Corrections to a MUST are listed in the v1.62.0 revision row; the full list, with evidence, is in the change that introduced them. Where the SDKs disagree, or agree on questionable behaviour, the specification was left unchanged and the item is handled in phase 3.

**Areas corrected.** §2.1.1 snake_case derivation; §2.2 ID map; §2.5 / §2.7 reserved words and ID grammar; §4.8.5; §4.17 export profiles; §5.2 entry-point inference; §5.3 dependency stalls (D-79); §5.11.5 optional types; §5.12.5 schema inference; §5.16 requirements 3–4 (`configure_step`, `run_until`); §6.3 / §6.3.1 audit records; §6.6.3.2 presets; §7.4 resume semantics; §7.10 approval tiers; §8.2 / §8.4 / §8.6 / §8.7 error codes; §9.1 / §9.2 / §9.8 / §9.12 / §9.13 / §9.15 environment variables and examples; §9.16.2 events; §10.1 span names; §11.2–§11.3 registration; §11.8.3–§11.8.4 middleware errors and timeouts; §12.2 / §12.4 / §12.7.4 / §12.8 validate and registry contracts; §13.5 unknown configuration fields.

**Pinned by** the existing fixtures and SDK tests of each area; no behaviour changed.

## Phase 3 — SDK alignment (spec v1.63.0)

Decided on the order of precedence above. Each changes at least one SDK and is pinned by a fixture unless noted.

### D-133 — A timeout cancels the call's own token and returns immediately

A per-module or global-deadline timeout raises `MODULE_TIMEOUT` at once and cancels the timed-out call's cancel token; the executor does not wait for the module to exit. There is no grace period and no forced termination — neither can be implemented portably (a Python thread and a JavaScript promise cannot be killed; a tokio task stops only at an await point). Every call receives a child token linked to its caller's: cancelling the caller cancels the child, cancelling the child does not reach the caller, so a nested call's timeout is catchable by its caller. Spec §12.7.5, §12.7.8, A22.

### D-134 — `validate()` reports every check

A dry run evaluates each pure step and records its check; a failing check does not stop later checks. The one exception stays: module-level `preflight()` / `preview()` are withheld after an ACL failure (§12.8.5.1). Passed checks are never dropped when a later step fails. Spec §12.8.

### D-135 — Retryable defaults are explicit

`EXECUTION_CANCELLED` (with a fresh token), `APPROVAL_TIMEOUT` and `CIRCUIT_BREAKER_OPEN` are retryable; `PIPELINE_CONFIGURATION_ERROR` is not. Every SDK sets the default explicitly. Spec §8.6.

### D-136 — Input validation accepts the JSON the contract describes

Inputs arrive as JSON. A `date-time`, `uuid` or enum field receives a string, and validation MUST accept a string the schema accepts; apcore-python validates in JSON mode rather than strict Python mode. Spec type-mapping §17.3.

### D-137 — `middleware.disabled` is deprecated

It is read by no implementation and names built-in middleware that does not exist; declaring it warns once per load, and it is removed at 2.0. Use `remove()` or `pipeline.remove`. Spec §9.2.4, §11.4.

### D-138 — `id_map.overrides` is a path-typed key

Its value is the path of an ID map file `{mappings: [{file, id}]}`; `file` is relative to the extensions root in every SDK; the unused `class` field is removed; an empty environment value falls through to the file value (§9.2.1 requirement 5). `sys_modules.control.overrides_path` gains the path marker too. Spec §2.2, §9.2.1, §9.2.2; schema `IDMapConfig`.

### D-139 — Binding files fail loudly

Unknown top-level or entry keys, an explicit schema pair with one side missing, and an invalid `auto_schema` value raise `BINDING_FILE_INVALID`; schema inference that cannot produce a schema raises `BINDING_SCHEMA_INFERENCE_FAILED` in every mode, implicit included — a permissive fallback turns a typo into an unvalidated module. Spec §5.12.2, §5.12.5, §5.12.7.

### D-140 — Export profiles follow §4.17 and Appendix D.1

The `anthropic` profile strips `x-*` keys; the `mcp` profile carries `requires_approval` and `streaming` in `_meta`, not in `annotations`, since MCP tool annotations define neither.

### D-141 — Preflight check reporting

A module that does not implement `preflight()` MAY still produce a passed `module_preflight` check (some languages cannot tell a default method from an implemented one). `preview()` returning null adds no check. `predicted_changes` is always present, empty when there is nothing to report. Spec §12.8.3, §12.8.5.1.

### D-142 — A per-step timeout raises `MODULE_TIMEOUT`

Spec §5.16.1.

### D-143 — The specification drops what no implementation has

The `apcore.module.validate` span (§10.1), extension registration from `apcore.yaml` and extension chaining (§11.3, §11.7), and apcore-rust's unwired `middleware:` YAML chain parser. Conformance deviation closed.

### D-144 — The observability namespace declares what the schema declares

`observability` registers `tracing` and `metrics` only; the unread `logging`, `error_history`, `platform_notify` (and `redaction`) blocks are removed. Their live equivalents are `sys_modules.error_history.*`, `sys_modules.events.thresholds.*` and `obs.redaction.*`, and every SDK pre-registers the `obs` namespace that carries `obs.redaction.*` (apcore-rust did not). Spec §9.15, §9.15.2, §9.15.4.

### D-145 — Subscriber circuit events carry `subscriber_id`

Spec §9.16.2.

### D-146 — Environment-variable dispatch

Only the `apcore` namespace may use the exact prefix `APCORE` (`CONFIG_NAMESPACE_RESERVED`). In namespace mode a variable is dispatched to the namespace with the longest matching prefix only, and an `APCORE_` variable matching no registered prefix belongs to `apcore`; it is never written to two namespaces. Spec §9.8.

### D-147 — Multi-class discovery registers marked classes only

Once a file opts in, only the classes carrying the per-class marker receive IDs (D-107). A marked class's ID is always `base_id.segment`, whatever the number of marked classes in the file — otherwise marking a second class would rename the first. §2.1.1 rule 5 still gives a file with exactly one Module class its bare `base_id`. Spec §2.1.1, features/multi-module-discovery.

### D-148 — The ephemeral contract is implemented as written

Every SDK emits exactly one audit event per ephemeral register / unregister under the standard bootstrap, with `caller_id`, `identity` (or null) and `namespace_class: "ephemeral"`; the bare ID `ephemeral` is ephemeral everywhere; rejections raise `INVALID_MODULE_ID`. Spec §2.5.1.

### D-149 — Error details have one wire shape

`SchemaValidationError.details.errors` items are `{path, keyword, message}` with `path` a JSON Pointer (`/count`, `""` for the root); `details` keys are snake_case in every SDK. Spec §8.1.

### D-150 — `extensions.auto_discover` is deprecated

Discovery runs when `discover()` is called; the key is read only by a validation warning. Deprecated, removed at 2.0. Spec §9.2.4.

### D-151 — Each SDK declares its conformance

Each SDK repository ships the `apcore-conformance.yaml` declaration that conformance.md §6.1 requires.

### D-152 — `x-sensitive` is honoured inside schema combinators

A property marked `x-sensitive` inside an `anyOf` / `oneOf` / `allOf` branch — which is where an optional field (`Secret | None`) puts it — is redacted. Neither §10.6 / A13 nor any SDK looked inside combinators, so the value was logged and captured in plain text. The redaction walk descends into combinator branches, and a value is redacted when any branch that describes it marks it sensitive. Spec §10.6, A13.

Single-SDK defects found by the audit that change no contract are fixed as bugs in the same release and listed in each SDK's CHANGELOG.
