---
description: "Normative naming and visibility conventions for apcore SDK API surfaces — defines when a symbol is public API, why cross-boundary contract members MUST NOT carry a private name, and the per-language idioms for hiding a symbol from application code without lying about its visibility."
---

# apcore — API Surface & Naming Conventions

> This document defines how apcore SDKs (`apcore-python`, `apcore-typescript`, `apcore-rust`, and downstream bridges such as `apcore-mcp-*`) MUST name and scope the symbols they expose. It exists to keep the *public API surface* of every SDK aligned and to prevent a recurring mistake: marking a cross-package contract member as "private" by name while still requiring foreign code to call or implement it.

## 1. Overview

### 1.1 Purpose

A leading underscore (`_foo`), a `private` modifier, or a non-`pub` declaration is a **promise**: *"this is internal — application code and other packages may ignore it, and it MAY change without notice."* When the apcore Executor (one package) calls a method on a caller-supplied or duck-typed `Context` (another package), that method is — by definition — part of the cross-package contract. Naming it as private breaks the promise: every foreign `Context` implementation is now forced to implement a method whose name says "do not touch."

This specification separates two concerns that are routinely conflated:

- **Visibility** — *can* a symbol be referenced across a package boundary? This is a hard, mechanical property (export, `pub`, public method).
- **Discoverability** — *should* an application developer reach for it? This is a soft, advisory property (documentation, naming clarity, IDE de-emphasis).

A symbol can be fully public (visibility) yet de-emphasized for application authors (discoverability). The two MUST be controlled by **separate mechanisms**.

### 1.2 Scope

These conventions are **normative** for all official apcore SDKs and for any package that implements an apcore contract type (e.g., a bridge that supplies its own `Context`). They govern naming and visibility only; they do not constrain internal algorithms, formatting, or lint configuration. RFC 2119 keywords (`MUST`, `MUST NOT`, `SHOULD`, `MAY`) apply.

### 1.3 Terminology

- **Public API surface** — the set of symbols an SDK exports for consumption by application code *or* by another package.
- **Cross-boundary contract member** — a symbol that one package invokes on, or requires to be implemented by, a type owned by another package. The Executor's call to `Context`'s executor-binding method is the canonical example.
- **Internal helper** — a symbol used only within the package that declares it; no other package references it.

---

## 2. Core Principle

> **If a symbol must be invoked across a package boundary, or implemented by a foreign type to satisfy an apcore contract, it is public API. It MUST have a public name (no language "private" marker) and MUST be documented as part of the relevant contract. "Not for application code" MUST be expressed through discoverability mechanisms — never by giving the symbol a private name or by stripping it from the type definitions that foreign implementers compile against.**

Conversely, a true internal helper (referenced only within its own package) MUST use the language's private mechanism, so that the public surface stays minimal.

---

## 3. Normative Rules

1. **Contract members are public.** A cross-boundary contract member MUST be visible across package boundaries (exported / `pub` / public method) and MUST NOT carry a leading-underscore name, a `private` modifier, or a non-`pub` declaration.

2. **Internal helpers are private.** A symbol referenced only within its declaring package MUST use the language's private mechanism and MUST NOT be exported.

3. **Discoverability is separate from visibility.** To signal "public but not for application code," an SDK MUST use a discoverability mechanism that does **not** alter the symbol's name or remove it from the API that foreign implementers compile against:
   - Python — a docstring beginning `SDK-internal contract member.` (single leading underscore is **discouraged** for contract members; see §5).
   - TypeScript — a `/** Contract member. Not for application code. */` doc comment. `@internal` + `stripInternal` MUST NOT be used on a member that a foreign `Context` (e.g., a bridge) must declare, because stripping it from the emitted `.d.ts` breaks the foreign implementer.
   - Rust — `#[doc(hidden)]` on a `pub` item. This is the reference pattern: `pub` keeps it implementable/callable; `#[doc(hidden)]` keeps it out of the rendered docs.

4. **Consistent contract names.** The same contract member MUST use the same core noun across SDKs (e.g., `executor`). The verb MAY differ to reflect language-idiomatic mutation semantics — `bind_*` / `set_*` for in-place mutation, `with_*` for copy-on-write that returns a new instance — but the verb MUST accurately describe the semantics in that SDK and MUST NOT be a privacy marker.

5. **No private name in a public type.** A public interface, trait, or protocol MUST NOT declare a member whose name marks it private. If a member appears in a public type, it is public — name it accordingly.

---

## 4. Per-language Idioms

| Concern | Python | TypeScript | Rust |
|---|---|---|---|
| Internal helper (single package) | leading `_name` | `private`/`#name`, not exported | non-`pub` (module-private) or `pub(crate)` |
| Public, application-facing | plain name, in `__all__` | exported, in public `.d.ts` | `pub` |
| Public **contract member**, de-emphasized for apps | plain name + `SDK-internal contract member.` docstring | exported plain name + doc comment (NOT `@internal`-stripped) | `pub` + `#[doc(hidden)]` |

Rust's `pub fn` + `#[doc(hidden)]` is the model the other SDKs SHOULD mirror: the symbol stays callable and implementable (visibility), while staying out of the rendered API reference (discoverability).

---

## 5. Worked Example — Executor binding to Context

The contract is defined normatively in [Core Executor §Contract: Executor binding to Context](../features/core-executor.md#contract-executor-binding-to-context): the Executor MUST bind itself to a null-`executor` `Context` before pipeline step 1. That binding method is a **cross-boundary contract member** — the Executor (one package) calls it, and a bridge's duck-typed `Context` (another package) MUST implement it. Therefore it MUST be public-named in every SDK.

=== "Python"
    ```python
    from typing import Any
    from dataclasses import dataclass, field

    @dataclass
    class Context:
        executor: Any = None

        # Contract member: public name, no leading underscore. The Executor
        # calls this; foreign Context implementations must provide it.
        def bind_executor(self, executor: Any) -> None:
            """SDK-internal contract member. Bind the Executor to this Context.

            Implements PROTOCOL_SPEC §"Contract: Executor binding to Context".
            Not intended for application code — invoked by the Executor before
            pipeline step 1.
            """
            from apcore.errors import ContextBindingError
            if self.executor is None:
                self.executor = executor
            elif self.executor is not executor:
                raise ContextBindingError(
                    "Context already bound to a different Executor instance"
                )
            # same instance: idempotent noop
    ```

=== "TypeScript"
    ```typescript
    import { ContextBindingError } from './errors';

    export interface ContextContract {
      readonly executor: unknown;
      /** Contract member. Not for application code. Bind the Executor
       *  to this Context (copy-on-write). Bridges MUST implement this. */
      withExecutor(executor: unknown): ContextContract;
    }

    export class Context<T = null> implements ContextContract {
      constructor(readonly executor: unknown = null /*, ...other fields */) {}

      // `with*` because TypeScript Context fields are `readonly`: binding
      // returns a NEW instance rather than mutating in place. Public name —
      // it appears on the public ContextContract interface.
      withExecutor(executor: unknown): Context<T> {
        if (this.executor === executor) return this;
        if (this.executor != null) {
          throw new ContextBindingError(
            'Context already bound to a different Executor instance',
          );
        }
        return new Context<T>(executor /*, ...copied fields */);
      }
    }
    ```

=== "Rust"
    ```rust
    use std::sync::Arc;
    use crate::errors::{ModuleError, ErrorCode};

    impl<T> Context<T> {
        /// SDK-internal contract member. Bind the Executor to this Context.
        ///
        /// Implements PROTOCOL_SPEC §"Contract: Executor binding to Context".
        /// `pub` so the Executor (and conformance harnesses) can call it;
        /// `#[doc(hidden)]` so it stays out of the rendered API reference.
        #[doc(hidden)]
        pub fn bind_executor(
            &mut self,
            executor: Arc<dyn std::any::Any + Send + Sync>,
        ) -> Result<(), ModuleError> {
            match &self.executor {
                None => { self.executor = Some(executor); Ok(()) }
                Some(existing) if Arc::ptr_eq(existing, &executor) => Ok(()),
                Some(_) => Err(ModuleError::new(
                    ErrorCode::ContextBindingError,
                    "Context already bound to a different Executor instance",
                )),
            }
        }
    }
    ```

Note the verb difference is intentional and rule-compliant: Python and Rust mutate in place (`bind_executor`), while TypeScript returns a new instance over `readonly` fields (`withExecutor`). What MUST match is the public visibility and the core noun (`executor`); what MAY differ is the verb, because the semantics genuinely differ per §3 rule 4.

---

## 6. Known Divergences (migration tracking)

The following reflects the state observed at apcore SDK v0.24.0 and the additive-alias migration applied on 2026-06-18. These are **non-normative** notes for SDK maintainers; resolving them brings the SDKs into line with §3. Each fix lives in the respective SDK repo, not in this spec.

| SDK | Symbol | Issue vs. §3 | Status |
|---|---|---|---|
| `apcore-rust` | `pub fn bind_executor` + `#[doc(hidden)]` | Compliant — reference pattern | ✅ no change (no `_`-prefixed variant exists) |
| `apcore-python` | `_bind_executor` | Rule 1 — leading underscore on a contract member | ✅ added public `bind_executor` (real impl); `_bind_executor` is now a `DeprecationWarning` alias; all internal callers use the new name |
| `apcore-typescript` | `_withExecutor()` | Rule 1 & 5 — private-named member that `apcore-mcp-typescript`'s `BridgeContext` interface must declare and implement | ✅ added public `withExecutor()` (real impl); `_withExecutor()` is now a `@deprecated` alias; **Executor caller prefers the new name and falls back to the old** (per §6.1 step 3) |
| `apcore-mcp-typescript` | `BridgeContext._withExecutor()` | Rule 5 — private-named member on a public interface | ✅ interface + impl now expose public `withExecutor()`; `_withExecutor()` kept as `@deprecated` alias for older apcore-js peers |
| `apcore-typescript` | `_withCancelToken()` | Rule 1 — same private-name pattern | Classified **single-package internal** (no cross-file, exported-interface, or foreign-implementer use) → no alias needed; candidate for a true `private`/`#name` in a future cleanup |

!!! note "Removal of the deprecated `_`-aliases (step 5) is still pending"
    The migrations above are non-breaking (steps 1–4). The deprecated `_bind_executor` / `_withExecutor` aliases remain callable and are exercised by existing tests. They are removed only in a later **major** version, once the deprecation window closes.

### 6.1 Recommended migration — additive alias, no breaking change

A hard rename of a de-facto-public member is a breaking change. Prefer an **additive** migration that introduces the public name without removing the old one, so existing callers and foreign implementations keep working:

1. **Add the public-named member** as the real implementation (`bind_executor` / `withExecutor`), documented as a contract member per §3.
2. **Demote the old `_`-prefixed member to a thin alias** that delegates to the new one, marked deprecated with the language's standard mechanism (so users get a migration signal, not a break):
   - Python — `warnings.warn(..., DeprecationWarning, stacklevel=2)` inside `_bind_executor`, or the `@warnings.deprecated(...)` decorator (3.13+ / `typing_extensions`).
   - TypeScript — a `/** @deprecated Use withExecutor(). */` JSDoc tag on `_withExecutor` (surfaces a strikethrough in editors).
3. **Update the caller (Executor) to prefer the new name** while tolerating the old one during the deprecation window — call `withExecutor` if present, else fall back to `_withExecutor`. This lets a foreign `Context` that implements **only** the new name work immediately, without forcing every bridge to upgrade in lockstep.
4. **Migrate bridges** (`apcore-mcp-*`, `BridgeContext`) to implement the public name; they MAY keep the `_`-prefixed alias for older apcore-js peers.
5. **Remove the deprecated alias in a later major** once the window closes.

Steps 1–4 are non-breaking and can ship in a minor version; only step 5 is breaking and waits for a major bump.

=== "Python"
    ```python
    import warnings
    from typing import Any

    class Context:
        executor: Any = None

        def bind_executor(self, executor: Any) -> None:
            """SDK-internal contract member. Bind the Executor to this Context."""
            ...  # real implementation

        def _bind_executor(self, executor: Any) -> None:
            """Deprecated alias for :meth:`bind_executor`."""
            warnings.warn(
                "Context._bind_executor is deprecated; use bind_executor.",
                DeprecationWarning,
                stacklevel=2,
            )
            self.bind_executor(executor)
    ```

=== "TypeScript"
    ```typescript
    class Context<T = null> {
      // Public contract member — the real implementation.
      withExecutor(executor: unknown): Context<T> {
        // ...real implementation (copy-on-write)...
        return this;
      }

      /** @deprecated Use {@link withExecutor}. Kept for older callers. */
      _withExecutor(executor: unknown): Context<T> {
        return this.withExecutor(executor);
      }
    }

    // Caller (Executor) prefers the new name, tolerates the old one:
    function bind(ctx: any, executor: unknown) {
      return typeof ctx.withExecutor === 'function'
        ? ctx.withExecutor(executor)
        : ctx._withExecutor(executor);
    }
    ```

!!! note "Why the caller must change too"
    Adding the public alias on `Context` is only half the fix. As long as the Executor calls `_withExecutor`, a foreign `Context` that implements only the new `withExecutor` would still break. Step 3 (prefer-new-fall-back-to-old in the caller) is what makes the contract genuinely public during the window.

---

## 7. SDK Author Checklist

When adding or reviewing a symbol on a contract type (`Context`, `Executor`, `Module`, …):

1. Is it referenced by another package, or required of a foreign implementation? → **public name, documented as a contract member.** Never a leading underscore / `private` / non-`pub`.
2. Is it referenced only inside its own package? → **private mechanism**, not exported.
3. Want to keep a public contract member out of the application-facing docs? → use the **discoverability** mechanism for the language (§3 rule 3), not its name.
4. Naming a contract member that already exists in a sibling SDK? → reuse the **same core noun**; pick the verb that matches this SDK's mutation semantics (§3 rule 4).
5. Adding a member to a public interface/trait/protocol? → it is public by construction; its name MUST NOT mark it private (§3 rule 5).

---

## 8. Cross-SDK Surface Equivalence

Two SDKs can be **surface-equivalent** without exposing byte-identical flat symbol lists. Parity is judged by *capability reachable through a documented public path*, not by a literal diff of exported names. This section names the structural divergences that are **expected and idiomatic** (and MUST NOT be reported as a missing symbol by an audit), and draws the line past which a divergence becomes a real surface break.

### 8.1 Expected structural divergences

The following differences are by-design consequences of each language's idioms. A cross-language surface audit MUST normalize them away before reporting a gap.

| Axis | Python / TypeScript | Rust | Why it is not a gap |
|---|---|---|---|
| **Error taxonomy shape** | one distinct public exception class per failure (`ModuleNotFoundError`, `ApprovalDeniedError`, …) | variants of a single `pub enum ErrorCode` carried by one `pub struct ModuleError` | Equivalence is at the level of *the error identity being representable*, not *one exported type per error*. The per-type classes have **no** standalone Rust equivalent, and that is correct. |
| **Namespace depth** | a cohesive area MAY sit behind a public sub-package (`apcore.observability.PrometheusExporter`) | the same symbol is often flattened to the crate root (`PrometheusExporter`) | Both are public. The depth of the documented path is a style choice, not a surface gap. |

The error-taxonomy difference is concrete:

=== "Python"
    ```python
    # Each failure is its own public exception class.
    from apcore import ModuleNotFoundError, ApprovalDeniedError

    raise ModuleNotFoundError("executor.email.send_email not registered")
    ```
=== "TypeScript"
    ```typescript
    // Same shape: one exported class per failure.
    import { ModuleNotFoundError, ApprovalDeniedError } from 'apcore-js';

    throw new ModuleNotFoundError('executor.email.send_email not registered');
    ```
=== "Rust"
    ```rust
    // One enum of codes + one error struct. There is NO `ModuleNotFoundError`
    // type to export — `ModuleNotFound` is a variant, and that is idiomatic.
    use apcore::{ErrorCode, ModuleError};

    return Err(ModuleError::new(
        ErrorCode::ModuleNotFound,
        "executor.email.send_email not registered",
    ));
    ```

### 8.2 The reachability rule (where a divergence becomes a bug)

> **Every public symbol MUST be reachable through at least one documented, stable public path: the package root, or a public sub-package / module whose own export list (`__all__`, `index.ts`, `lib.rs` / `mod.rs`) includes it. A symbol reachable *only* through an internal implementation module — one callers were never meant to import from — is NOT acceptable namespacing; it is a silent surface break and MUST be fixed by re-exporting it through a public path.**

This is the line between §8.1's namespace-depth divergence (fine) and a genuine break:

- `apcore.observability.PrometheusExporter` — **fine**: `observability` is a public sub-package with its own `__all__`; the import path is one a caller is meant to use.
- `apcore.registry.registry.MAX_MODULE_ID_LENGTH` — **break**: `registry.registry` is the *implementation module* inside the `registry` package. The constant is absent from both the `apcore` root and the `apcore.registry` package export list, so the only working import reaches into an internal path — while the same constant is root-public in TypeScript and Rust. (Tracked in the `apcore-python` repo.)

Quick test: *if the only import path that works names a module callers were never meant to import from, the symbol is private by accident — re-export it through a public path.*

### 8.3 Auditing guidance (non-normative)

When diffing public surfaces across SDKs, normalize before reporting, so the recurring §8.1 divergences stay out of audit reports while genuine §8.2 breaks still surface:

1. Collapse each SDK's error types to the **error-identity set** (a Rust `ErrorCode` variant ↔ a Python/TS exception class), then compare the sets — not the exported-type counts.
2. Resolve each symbol to its **shallowest documented public path** (root or public sub-package) and compare *capabilities*, treating a symbol with a valid sub-package path as present.
3. Flag a gap only when a capability is reachable in one SDK through a documented public path **and** in another SDK only through an internal module, or not at all.

---

## 9. References

- [Core Executor §Contract: Executor binding to Context](../features/core-executor.md#contract-executor-binding-to-context)
- [Context Object](../features/context-object.md)
- [Canonical Protocol Spec](./protocol-spec.md)
- [Cross-language Type Mapping](./type-mapping.md)
</content>
</invoke>
