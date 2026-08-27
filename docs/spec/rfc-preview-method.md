---
description: "Accepted RFC adding the optional module preview() method and PreflightResult.predicted_changes so AI orchestrators can see what a destructive call would change before executing."
---

# RFC — Module-level `preview()` Method and `PreflightResult.predicted_changes`

## Status

**Accepted** (2026-05-06). Promoted to protocol-spec.md normative sections in v0.21.0:

- `§5.6 Module Interface Protocol` — optional `preview()` method added to optional_methods + pseudocode interface
- `§12.8 Executor.validate() type contracts` — new `Change` and `PreviewResult` types; `PreflightResult.predicted_changes` field added; `PreflightCheckResult.check` enum extended with `module_preview`

This RFC document is retained as **design rationale + cross-SDK schema-encoding reference**. Implementations should consult protocol-spec.md for the normative contract; this document for the *why* and the cross-SDK encoding patterns (pydantic / serde-flatten / TypeBox `Type.Unsafe`).

## Motivation

`Executor.validate()` (protocol-spec.md §12.2) runs the pipeline in `dry_run=true` mode and skips non-`pure` steps; it returns a `PreflightResult` describing whether each pipeline check passed and whether the call would require approval. `Module.preflight()` (protocol-spec.md §5.6, contract at §12.8.5.1) lets a module emit advisory `list[str]` warnings.

Neither surface answers the question that AI orchestrators consuming destructive modules increasingly need to ask:

> **"If I were to call this module with these inputs, what would change in the world?"**

Adjacent research has been converging on this answer:

- **AgentSpec (ICSE 2026, arXiv 2503.18666)** proposes runtime enforcement frameworks for high-stakes LLM-agent workflows (financial transactions, medical records, corporate decisions).
- **AgentHarm** benchmarks measure agent safety on destructive operations.
- **Superego architectures** layer real-time configurable oversight atop LLM planning to nearly eliminate harmful outputs.

Apcore today gives orchestrators no canonical channel for a module to **self-report its predicted changes**. This RFC proposes one.

## Non-goals

- Replacing or deprecating `preflight()`. Warnings remain useful and orthogonal.
- Mandating `preview()` for every module. The method is **optional**; modules that don't implement it leave `predicted_changes` empty.
- Imposing a closed schema for change records. Different module classes (DB writes, network calls, file I/O, email sends) have heterogeneous side effects.
- Defining sandboxing or runtime enforcement. That sits one layer above (host concern; `apcore-mcp` / `apcore-a2a` may wrap this surface).

## Proposed `Module` interface addition

A new optional method, parallel to `preflight()`:

```python
# Conceptual signature; per-language variants below
def preview(inputs: dict, context: Context) -> PreviewResult | None:
    """
    Return a structured prediction of changes this call would produce.
    Optional — modules that don't implement it return None (or omit the method).
    MUST NOT have side effects.
    """
```

`preview()` is invoked by `Executor.validate()` after the standard validation pipeline succeeds, only on modules that override it. The result is folded into the existing `PreflightResult` via a new optional field (see below).

## Proposed `PreviewResult` schema

Minimum viable schema:

```yaml
PreviewResult:
  type: object
  required: [changes]
  properties:
    changes:
      type: array
      items: { $ref: '#/$defs/Change' }
  description: Module's prediction of what would change if the call were executed.

Change:
  type: object
  required: [action, target, summary]
  properties:
    action:
      type: string
      description: |
        Free-form verb describing the kind of change
        (e.g. "write", "delete", "send", "charge", "publish").
        Free-form rather than enum — module authors define their own taxonomy.
    target:
      type: string
      description: |
        Free-form identifier of what is changed (e.g. "users.42",
        "table=orders row=…", "stripe:charge:ch_abc", "smtp:user@example.com").
    summary:
      type: string
      description: |
        Required, human-readable single-line summary of the change.
        Floor for destructive modules — at minimum, modules MUST be able
        to surface this for human review. Aligns with HCI guidance from
        Superego-style oversight layers.
    before:
      description: |
        Optional. Snapshot of the prior state, when observable.
        OMIT for unobservable side effects (email send, opaque network call).
        Schema is `any` — module-class specific.
    after:
      description: |
        Optional. Predicted new state.
        OMIT when unknown (e.g. server-assigned IDs).
    # x-* extension fields permitted (consistent with §4.4 extras)
```

The required `summary` field is the floor: even a module that wraps an opaque external API can produce `{action: "send", target: "smtp:user@example.com", summary: "Send order confirmation email to user@example.com"}`.

### `Change.x-*` extension fields — cross-SDK schema-encoding note

The `Change` object MAY contain any number of `^x-`-prefixed keys with arbitrary values, mirroring the §4.6 metadata-extension convention. **This RFC does not prescribe a runtime-validation mechanism**; each SDK uses an idiomatic encoding:

| SDK | Idiomatic encoding | Note |
|-----|-------------------|------|
| `apcore-python` | `pydantic.BaseModel` with `model_config = ConfigDict(extra='allow')`, plus a model-validator that asserts unknown keys match `^x-` | Native; round-trips through `model_dump()` |
| `apcore-rust` | `#[serde(flatten)] extra: HashMap<String, Value>` + custom validator at construction time | Native; flatten preserves wire format |
| `apcore-typescript` | TypeBox 0.34 has no native template-literal index keys for `Type.Object`; use `Type.Unsafe<Change>(...)` to inject raw JSON Schema `patternProperties: { "^x-": {} }` while preserving the TypeScript type. `Type.Intersect([Object, Record])` is **not** equivalent because it loses `additionalProperties: false`. | Escape-hatch; TypeBox idiom |

Conformance fixtures MUST cover at least: (a) a `Change` with required fields only and no `x-*` keys; (b) a `Change` with one `x-foo: <value>` key that round-trips identically. This guards against an SDK silently dropping `x-*` keys during serialization.

## Proposed `PreflightResult` extension

A new optional field on the existing `PreflightResult` (protocol-spec.md §12.8.4 type table):

```yaml
PreflightResult:
  # ... existing fields ...
  predicted_changes:
    type: array
    items: { $ref: '#/$defs/Change' }
    description: |
      Optional. Populated when Executor.validate() skips a non-`pure` step
      AND the target module implements `preview()`. Empty otherwise.
```

`Executor.validate()` populates `predicted_changes` by:

1. Running pipeline in `dry_run=true` (existing behavior).
2. After existing checks pass, calling `module.preview(inputs, context)` if the method is present.
3. Folding `result.changes` into `predicted_changes`.
4. Catching exceptions from `preview()` as advisory warnings (do **not** fail validation if `preview()` raises — match existing `preflight()` semantics).

## Cross-language sketch

=== "Python"
    ```python
    from apcore import Module, Context, PreviewResult, Change

    class DeleteUser(Module):
        # ... existing input_schema, output_schema, execute() ...

        def preview(self, inputs: dict, ctx: Context) -> PreviewResult | None:
            user = self.repo.get_user(inputs["user_id"])
            if user is None:
                return None
            return PreviewResult(changes=[
                Change(
                    action="delete",
                    target=f"users.{user.id}",
                    summary=f"Permanently delete user {user.email}",
                    before={"id": user.id, "email": user.email, "tier": user.tier},
                ),
            ])
    ```
    Detection mirrors `preflight()`: `hasattr(module, "preview") and callable(module.preview)`.

=== "TypeScript"
    ```typescript
    import { Module, Context, PreviewResult } from 'apcore-js';

    class DeleteUser implements Module {
      // ... existing inputSchema, outputSchema, execute() ...

      async preview(inputs: Record<string, unknown>, ctx: Context): Promise<PreviewResult | null> {
        const user = await this.repo.getUser(inputs.user_id as string);
        if (!user) return null;
        return {
          changes: [{
            action: 'delete',
            target: `users.${user.id}`,
            summary: `Permanently delete user ${user.email}`,
            before: { id: user.id, email: user.email, tier: user.tier },
          }],
        };
      }
    }
    ```
    Detection mirrors `preflight?`: `typeof module.preview === 'function'`.

=== "Rust"
    <!-- apcore-example: fragment -->
    ```rust
    use apcore::{Module, Context, PreviewResult, Change};

    impl Module for DeleteUser {
        // ... existing input_schema, output_schema, execute() ...

        fn preview(
            &self,
            inputs: &serde_json::Value,
            _ctx: Option<&Context<serde_json::Value>>,
        ) -> Option<PreviewResult> {
            let user = self.repo.get_user(inputs["user_id"].as_str()?)?;

            // `Change` and `PreviewResult` are `#[non_exhaustive]`, so a downstream
            // crate builds them from `Default::default()` and assigns fields. A
            // struct literal — with or without `..Default::default()` — is E0639.
            // See "Migration pattern for downstream Rust consumers" below.
            let mut change = Change::default();
            change.action = "delete".into();
            change.target = format!("users.{}", user.id);
            change.summary = format!("Permanently delete user {}", user.email);
            change.before = Some(serde_json::json!({
                "id": user.id, "email": user.email, "tier": user.tier
            }));

            let mut preview = PreviewResult::default();
            preview.changes = vec![change];
            Some(preview)
        }
    }
    ```
    Default impl on the `Module` trait returns `None`, matching the existing
    `preflight()` / `stream()` pattern (no separate sub-trait needed).

## Pre-conditions (Rust struct hygiene) — **resolved**

!!! success "Landed — this section is retained as rationale"
    The attribute change described below shipped with the RFC's implementation.
    `PreflightResult`, `PreflightCheckResult`, `PreviewResult` and `Change` all
    carry `#[non_exhaustive]` in `apcore-rust/src/module.rs` today, and all four
    derive `Default`. The migration pattern below is therefore the **current**
    construction rule for downstream crates, not a future one. It is stated
    normatively in
    [API Surface & Naming Conventions §9](./api-surface-conventions.md#9-constructing-sdk-owned-data-types).

As originally written: `apcore-rust`'s `PreflightResult` and `PreflightCheckResult` were defined as plain `pub struct {...}` with no `#[non_exhaustive]` attribute. Adding a new field (`predicted_changes`) **would hard-break** any downstream Rust consumer that constructs these structs via struct-literal syntax (e.g. `PreflightResult { valid: true, checks: vec![], requires_approval: false }`).

The three steps this section required before Stage 2 SDK work could proceed have all been carried out — they are recorded here as history, not as outstanding work:

1. ~~Open a tracking issue against `apcore-rust`~~ — opened as [apcore-rust#24](https://github.com/aiperceivable/apcore-rust/issues/24).
2. ~~List `PreflightResult`, `PreflightCheckResult`, and any other spec-derived public struct that may be extended~~ — the attribute now covers those two plus `PreviewResult`, `Change`, `ModuleExample`, `ValidationResult`, `ApprovalRequest`, `ApprovalResult`, three types in `async_task.rs`, two in `config.rs`, and `middleware::RetryConfig`.
3. ~~Land the attribute change in an `apcore-rust` minor bump~~ — shipped.

Nothing in this section remains to be done.

### Migration pattern for downstream Rust consumers — **important: not what you'd guess**

A common misconception (this RFC's earlier text included) is that `..Default::default()` (functional record update / FRU) bypasses `#[non_exhaustive]`. **It does not.** Per Rust's `E0639`, `#[non_exhaustive]` blocks struct-literal construction from outside the defining crate **entirely**, including FRU syntax. Within the crate that defines the struct, FRU still works.

**Correct downstream migration pattern**:

```rust
// ❌ NOT permitted from external crates (E0639):
let r = PreflightResult { valid: true, checks: vec![], ..Default::default() };

// ✅ Use Default + field assignment:
let mut r = PreflightResult::default();
r.valid = true;
r.checks = vec![];
// (or a future builder API the SDK may add)
```

The `apcore-rust` v0.21.0 CHANGELOG entry (commit landing alongside the `#[non_exhaustive]` attribute) is the canonical reference for the migration pattern. SDKs MAY ship a builder API in a follow-up minor; this RFC does not mandate one.

### Cross-SDK forward-compat semantics

| SDK | Adding a field is breaking? | Mechanism |
|-----|----------------------------|-----------|
| `apcore-python` | ❌ No | `dataclass` field with default; existing keyword-arg call sites unaffected |
| `apcore-typescript` | ❌ No | `interface { foo?: T }` is forward-compatible by structural-typing |
| `apcore-rust` | ⚠️ Yes — without `#[non_exhaustive]` declared upfront | Rust's only forward-compat declaration mechanism; cost is downstream construction-syntax tax (E0639) |

This asymmetry is **why** the `#[non_exhaustive]` work is staged as a Rust-specific pre-condition; the other two SDKs need no preparation step.

## Conformance plan

When this RFC is accepted, a new fixture `conformance/fixtures/preview_method.json` is added with at least the following test cases:

1. Module without `preview()` — `Executor.validate()` returns `PreflightResult` with `predicted_changes: []`.
2. Module with `preview()` returning `None` — same as above.
3. Module with `preview()` returning `PreviewResult` with one `Change` having all required fields and no optional fields — `predicted_changes` contains exactly that record.
4. Module with `preview()` returning multiple `Change`s including ones with `before`/`after` — all surface in `predicted_changes` in order.
5. Module whose `preview()` raises — validation does **not** fail; warning surfaces in `PreflightResult.warnings` (or equivalent existing surface).

The fixture is **not** created in this RFC stage; it ships with the PR that adds normative spec text in v0.21.0.

## Open questions

1. **`preview()` raising semantics.** Match `preflight()` (warning, no fail) or stricter (fail validation)? Recommendation: match `preflight()` for ergonomic parity.
2. **Streaming modules.** Should `preview()` apply to streaming modules? Recommendation: yes — `preview()` describes *what would change*, not *what would be returned*. Streaming output is orthogonal to side effects.
3. **`Change.before`/`after` schema.** Stay `any` (free-form per module class) or impose JSON Schema? Recommendation: stay `any` for v0.21.0; possible §4.6 conventions for typed `Change` shapes per module class in a follow-up.
4. **`x-supports-dry-run` interplay.** Should the new `x-supports-dry-run` convention (registered in §4.6 D-57) be set automatically when a module implements `preview()`? Recommendation: no — they're orthogonal signals. `x-supports-dry-run=true` says "validate() is meaningful"; implementing `preview()` is one *way* to make it meaningful.
5. **`PreviewResult` vs returning `Change[]` directly.** Wrapping in a struct lets us add fields later (e.g., a top-level `confidence: float`). Recommendation: keep the wrapper.

## Adjacent literature

- **AgentSpec** (ICSE 2026, arXiv 2503.18666) — Customizable Runtime Enforcement for Safe and Reliable LLM Agents.
- **AgentHarm** — LLM Agent Safety Benchmark (emergentmind summary).
- **The Superego agent architecture** — real-time, user-configurable oversight layered atop LLM planning.

These works approach the problem from the **outside** (external spec + runtime enforcement on untrusted agents). `preview()` approaches the same problem from the **inside** (modules in a trusted ecosystem self-report their predicted changes). The two surfaces are complementary, not competing.

## Cross-refs

- §4.6 D-57 — `x-supports-dry-run` convention (orthogonal but adjacent).
- §5.6 — `Module.preflight()` advisory warnings (parallel surface).
- §12.2 — `Executor.validate()` (entry point).
- §12.8.4 / §12.8.5.1 — `PreflightResult` / `PreflightCheck` type tables (extension target).
- `docs/spec/rfc-ephemeral-modules.md` — sibling RFC.
