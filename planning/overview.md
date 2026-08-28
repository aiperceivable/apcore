# apcore Implementation Plans — Project Overview

This directory tracks internal implementation plans for spec-derived features. Each subdirectory contains an `overview.md` (goals + scope) and a `state.json` (machine-readable progress). User-facing documentation lives in `docs/`, never here (per `.claude/rules/planning.md`).

## Completed features

| Feature | Tasks | Source spec |
|---------|-------|-------------|
| [acl-conditions-redesign](./acl-conditions-redesign/overview.md) | 7 | `docs/features/acl-system.md` |
| [annotations-redesign](./annotations-redesign/overview.md) | 5 | `docs/features/schema-system.md` |
| [async-task-evolution](./async-task-evolution/overview.md) | 3/3 | [`docs/features/async-tasks.md`](../docs/features/async-tasks.md) |
| [context-redesign](./context-redesign/overview.md) | 7/7 | [`docs/api/context-object.md`](../docs/api/context-object.md) |
| [event-management-hardening](./event-management-hardening/overview.md) | 3/3 | [`docs/features/event-system.md`](../docs/features/event-system.md) |
| [execution-pipeline](./execution-pipeline/overview.md) | 7 | `docs/features/core-executor.md` |
| [middleware-hardening](./middleware-hardening/overview.md) | 3/3 | [`docs/features/middleware-system.md`](../docs/features/middleware-system.md) |
| [multi-module-discovery](./multi-module-discovery/overview.md) | 4/4 | [`docs/features/multi-module-discovery.md`](../docs/features/multi-module-discovery.md) |
| [observability-hardening](./observability-hardening/overview.md) | 3/3 | [`docs/features/observability.md`](../docs/features/observability.md) |
| [pipeline-hardening](./pipeline-hardening/overview.md) | 4/4 | [`docs/features/core-executor.md`](../docs/features/core-executor.md) |
| [schema-hardening](./schema-hardening/overview.md) | 6/6 | [`docs/features/schema-system.md`](../docs/features/schema-system.md) |
| [system-modules-hardening](./system-modules-hardening/overview.md) | 3/3 | [`docs/features/system-modules.md`](../docs/features/system-modules.md) |
| [trace-context-unification](./trace-context-unification/overview.md) | 7/7 | `docs/features/observability.md` (TraceContext §) |

Tasks listed without a `done/total` ratio (e.g. `7`) lack a per-task array in their `state.json`; the feature is marked `status: completed` but task-level completion records weren't captured at the time.

## Spec patch drafts

Lightweight `docs/spec/protocol-spec.md` patch drafts that haven't been picked up as full features yet:

- [acl-compound-operators-spec-patch](./acl-compound-operators-spec-patch/overview.md) — proposed ACL compound-operators extension
- [validate-step-count-spec-patch](./validate-step-count-spec-patch/overview.md) — proposed `validate()` step-count clarification

These do not have `state.json` files; they are pre-feature design notes.

## Specification merged, SDK rollout pending

Spec text is on `main`; no SDK implements it yet. Until the rollout completes these are **known**
divergences between the specification and every implementation, and should not be reported as fresh
audit findings.

- [acl-unevaluable-conditions](./acl-unevaluable-conditions/overview.md) — spec v1.22.0 / v1.23.0 / v1.24.0, issues [#100](https://github.com/aiperceivable/apcore/issues/100) / [#101](https://github.com/aiperceivable/apcore/issues/101) / [#102](https://github.com/aiperceivable/apcore/issues/102). Unevaluable ACL conditions resolve toward denial (§6.1.1–§6.1.2, §6.3.1); ACL read-only accessors (§6.8); call-site inputs to policy resolution (§7.9.6). Carries a **staged** conformance fixture that must not move into `conformance/fixtures/` until all three drivers land — see that overview for why.

## Upcoming

In-flight RFC drafts live under `docs/spec/`, with planning artifacts under `planning/`:

- [`docs/spec/rfc-preview-method.md`](../docs/spec/rfc-preview-method.md) — optional `Module.preview()` method (Stage 2 of frontier-research alignment audit). Planning: [`preview-method`](./preview-method/overview.md). Cross-repo blocker: `apcore-rust` `#[non_exhaustive]` hygiene ([apcore-rust#24](https://github.com/aiperceivable/apcore-rust/issues/24)).
- [`docs/spec/rfc-ephemeral-modules.md`](../docs/spec/rfc-ephemeral-modules.md) — reserved `ephemeral.*` namespace (Stage 3). Planning: [`ephemeral-modules`](./ephemeral-modules/overview.md). Pilot queued in `apcore-python` ([apcore-python#25](https://github.com/aiperceivable/apcore-python/issues/25)).

When an RFC is accepted and promoted to a full implementation feature, its planning subdirectory will be added to the **Completed features** table above.
