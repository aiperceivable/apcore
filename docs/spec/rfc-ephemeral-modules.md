# RFC — Ephemeral / Programmatically-Registered Modules

## Status

**Draft / RFC.** No target acceptance version. This RFC is intentionally exploratory: a pilot implementation in one SDK (`apcore-python`, leveraging existing `register_internal()` precedent) is expected before any normative spec text lands.

## Motivation

A growing class of LLM-agent workflows synthesize callable units at runtime:

- Code-generation agents that compile a Python function from a task description and a repository URL (**ToolMaker**, ACL 2025, arXiv 2502.11705 — 80% task implementation correctness vs. 20% baseline).
- On-the-fly composition of existing modules into a new callable surface (LATM and follow-on work in the LLM-ToolMaker line).
- Per-session "scratch tools" assembled by an orchestrator and discarded after the session ends.

Today, the canonical apcore registration path is filesystem-based (PROTOCOL_SPEC.md §2.1 Algorithm A01: directory path → canonical Module ID). Authors of agents that synthesize tools at runtime have to either:

- Reuse `Registry.register(module_id, instance)` ad-hoc, but with no spec'd convention for naming, lifecycle, or audit; or
- Manage a parallel registry outside apcore, losing ACL, audit, and observability.

This RFC proposes a sanctioned `ephemeral.*` namespace that gives a name + minimum guardrails to programmatically-registered modules without disturbing the filesystem-rooted core.

## Why this is **not** a §2.1 violation

A common misreading is that "Module ID = directory path" is a runtime invariant. It is not.

- **§2.1 Algorithm A01** is the **scanner-stage** path-to-ID conversion. It governs how the default discoverer derives canonical IDs from filesystem layout. It does not constrain `Registry.register()`.
- **§2.1 trailing paragraph (around line 256)** specifies the *only* runtime requirement for `Registry.register(module_id, ...)`: `module_id` matches the canonical regex `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`.
- **§6.6.1 `register_internal()`** (around line 3598) is the existing precedent: `system.*` modules are programmatically registered with caller-supplied IDs that have no filesystem origin. The spec already accepts non-filesystem programmatic registration; this RFC just gives a class of such modules a name and a discipline.

A four-round audit (PROTOCOL_SPEC text + features docs + Module-interface contract + 3 SDK source reads) confirmed that all three SDKs already accept programmatic registration with caller-supplied IDs:

- `apcore-python` `Registry.register(module_id: str, module: Any, ...)` — `registry.py:857-965`.
- `apcore-typescript` `register(moduleId: string, module: unknown, ...)` — `registry/registry.ts:562-587`.
- `apcore-rust` `register(name: &str, module: Box<dyn Module>, descriptor: ModuleDescriptor)` — `registry.rs:469-476`. The `Module` trait has no `id()` method; ID is registry metadata, not module-instance state.

So the implementation primitives **all exist**. What's missing is convention + audit + lifecycle.

## Non-goals

- **Sandboxing.** Code execution isolation (Wasm, container, language VM) is the host's responsibility. Spec only governs registration, ACL, audit, and lifecycle.
- **Codegen.** This RFC does not specify how an agent generates the module body; that's pipeline territory (see ToolMaker for a reference design).
- **Replacing filesystem discovery.** `ephemeral.*` is additive. Filesystem-rooted modules retain their conventions unchanged.

## Proposed `ephemeral.*` namespace

Add a new entry to PROTOCOL_SPEC.md §2.5 reserved namespaces, parallel to `system.*` and `core.*`:

| Namespace | Purpose | Registration | Discoverable | Mandatory annotations |
|---|---|---|---|---|
| `system.*` (existing) | Built-in framework modules | `register_internal()` only | Yes | (existing) |
| `core.*` (existing) | Reserved for future spec promotion | (reserved — none yet) | — | — |
| `ephemeral.*` (new) | Programmatically-generated runtime modules | Standard `register()` | **No** (see annotation below) | `requires_approval: true` |

Modules in the `ephemeral.*` namespace:

- **MAY** be registered via standard `register(module_id, module, ...)` (in contrast to `system.*`, which uses `register_internal()`).
- **SHOULD** declare `requires_approval: true` to prevent agent-synthesized tools from running unattended.
- **SHOULD** declare `discoverable: false` (a new annotation; see below).
- **MUST** be subject to ACL `default_effect: deny`. ACL `target` patterns supporting `ephemeral.*` wildcards already work (per existing first-match-wins evaluation).
- **MUST** emit audit events through the framework `EventEmitter`, mirroring `system.control.*` write modules' contextual-audit shape (D-35).
- **SHOULD** declare a TTL (open question: convention key — see below).

## Proposed new `discoverable` annotation

A new boolean annotation on `Module`:

```yaml
annotations:
  discoverable: false   # default: true; ephemeral.* modules SHOULD set false
```

Semantics:

- `discoverable: true` (default) — module appears in `Registry.list()`, `Registry.find()`, manifest export, MCP `tools/list`, etc.
- `discoverable: false` — module is callable through `Registry.invoke()` / `Executor.execute()` but is hidden from enumeration surfaces. Caller must already know the module ID.

This is independently useful: filesystem-rooted modules can also opt out of enumeration (e.g., for internal-only utilities), and existing `internal: true` patterns in some SDK ports converge on the same idea.

## Lifecycle / GC contract — **open**

Two designs are on the table:

- **Caller-managed.** `ephemeral.*` modules live until the caller explicitly calls `Registry.unregister(module_id)`. Simple; matches `register_internal()` precedent. Risk: leakage if agent crashes mid-session.
- **TTL-driven.** Modules declare a TTL (e.g., `x-ephemeral-ttl-seconds: 3600`); framework runs a sweeper that unregisters expired entries. Mirrors `Reaper` for async tasks. Risk: another background loop to manage.

**Recommendation for pilot:** start with caller-managed (cheaper); add TTL as a v2 follow-up if leakage is observed in practice.

## Sandboxing — out of scope (host concern)

Apcore does not specify how `ephemeral.*` module bodies are sandboxed. Hosts that accept agent-synthesized code (e.g., `apcore-mcp` bridges, `apcore-cli` plugins) MUST apply their own isolation:

- Process-level (subprocess with restricted environment).
- Wasm runtime (if SDK ecosystem supports).
- Language-level (Python `RestrictedPython`, V8 isolates, `wasmtime` for Rust-loaded user code).

The spec's responsibility is bounded at registration → ACL → audit → lifecycle. It is not bounded at code execution.

## Migration / pilot plan

1. **Pilot in `apcore-python` first** — already has `register_internal()` precedent and the most permissive runtime. Pilot exposes:
   - `ephemeral.*` namespace as reserved (no enforcement yet, just convention).
   - `discoverable: false` annotation honored by `Registry.list()`.
   - Audit-event emission on `register()` and `unregister()` for `ephemeral.*` IDs.
2. **Track via tracking issue** — file against `apcore-python` repo. Stage outcome reports into `docs/spec/2026-05-decision-log.md`.
3. **Promote to spec when pilot confirms ergonomic** — at that point, this RFC becomes normative §2.5 + §4.4 amendments.
4. **TypeScript and Rust SDKs follow** — once Python pilot ships, replicate.

## Conformance plan (for post-acceptance)

`conformance/fixtures/ephemeral_modules.json` (not created in this RFC stage):

1. Register `ephemeral.test_v1` programmatically; assert `Registry.list()` does not include it; `Registry.invoke("ephemeral.test_v1", ...)` succeeds.
2. ACL `target: "ephemeral.*"` rule applies first-match-wins as expected.
3. Audit event `apcore.registry.module_registered` emitted with `module_id: "ephemeral.test_v1"` on register.
4. Audit event `apcore.registry.module_unregistered` emitted on unregister.
5. Re-registering same `ephemeral.*` ID replaces (or rejects per spec — open question for pilot).
6. `requires_approval: false` on an `ephemeral.*` module produces a SHOULD-warning (not error) at registration.

## Open questions

1. **Namespace name.** `ephemeral.*` vs `volatile.*` vs `temp.*` vs `dynamic.*` vs `agent.*`. Recommendation: `ephemeral.*` — most precise about lifecycle without implying source ("dynamic" is too broad; "agent" conflates with caller identity).
2. **Annotation name.** `discoverable: false` vs `internal: true` vs `hidden: true`. Recommendation: `discoverable: false` because it directly names the property being toggled.
3. **TTL key name (if pursued).** `x-ephemeral-ttl-seconds` vs `x-ttl-seconds` vs annotation `ttl_seconds`. Recommendation: keep as `x-*` convention initially; promote to first-class annotation only if usage justifies.
4. **Re-registration semantics.** Allow `register()` to overwrite an existing `ephemeral.*` ID, or require `unregister()` first? Recommendation: require unregister; agents can clear-and-rebuild explicitly.
5. **Cross-language collision risk.** Should the spec reserve `ephemeral.*` even before pilot completes, to prevent third-party use? Recommendation: yes — file a 1-line spec patch reserving the namespace immediately, separate from this full RFC's acceptance.
6. **Interaction with `apcore-mcp` `tools/list`.** Should `discoverable: false` modules be exposed through MCP `tools/list`? Recommendation: no by default; MCP servers can opt in via configuration if their host needs it.

## Adjacent literature

- **ToolMaker** (ACL 2025, arXiv 2502.11705 / GitHub: KatherLab/ToolMaker) — autonomous tool creation from scientific code repositories with closed-loop self-correction; 80% task implementation correctness vs. 20% baseline. **Real, verified.**
- **LATM (LLM-ToolMaker)** — earlier work in the same line.
- **Dynamic Tool Generation** survey topic — emergentmind, ai-scholar coverage.

ToolMaker is a *pipeline* (codegen + sandbox + verify); apcore's role under this RFC is to provide the *registration surface* a ToolMaker-like pipeline lands its outputs into. The two are complementary.

## Cross-refs

- §2.1 — Module ID specification and `Registry.register()` regex constraint.
- §2.5 — reserved-words registry (where `ephemeral.*` would be added).
- §4.4 — ModuleAnnotations (where `discoverable` would be added).
- §6.6.1 — `register_internal()` (existing precedent for non-filesystem registration).
- §4.6 D-57 — `x-supports-dry-run`, `x-required-context-keys`, `x-reasoning-demand` (registered in companion patch).
- `docs/spec/rfc-preview-method.md` — sibling RFC.
- `docs/features/system-modules.md` D-35 (contextual auditing for control plane) — audit-event shape `ephemeral.*` should mirror.
