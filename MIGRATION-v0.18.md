# Migration Guide — apcore v0.18.0

This document collects all breaking changes shipped in apcore v0.18.0 across the
spec, the JSON Schemas, and the three SDKs. Each section gives the rationale,
the exact change, the failure mode if you don't migrate, and a one-line fix.

If you are upgrading from any v0.17.x release, work through the sections in
order. Cross-language users (i.e. you write configs / annotations in one
language and read them in another) **must** read every section.

---

## TL;DR

| Change | Repos affected | Action required |
|---|---|---|
| `ModuleAnnotations.extra` wire format → nested object | python, typescript, rust | None if you use SDK getters; rewrite raw JSON producers |
| `apcore-rust` `Config` struct restructured to nested namespaces | rust | Rename callsites; rewrite YAML/JSON config files |
| `apcore-python` legacy event aliases removed (`module_health_changed`, `config_changed`) | python | Migrate event listeners to canonical names |
| Assorted internal cleanup (TS unused imports, dead fields) | typescript | None |

---

## 1. `ModuleAnnotations.extra` wire format (SPEC §4.4.1)

### What changed

PROTOCOL_SPEC §4.4.1 (new section in v0.18.0) declares that the `extra` field
on `ModuleAnnotations` **MUST** be serialized as a nested JSON object under the
key `"extra"`. Implementations **MUST NOT** flatten extension keys into the
annotations root.

### Why

Before v0.18.0 the spec was silent on the wire format and the three SDKs had
drifted:

- `apcore-python` and `apcore-typescript` already produced nested form
- `apcore-rust` ≤ 0.17.1 used `#[serde(flatten)]` and produced **flattened**
  extension keys at the annotations root

A binding round-tripped through Rust would silently lose the `extra` payload —
the nested object collapsed into `extra["extra"]` (one level too deep).
Discovered during a cross-language audit. See `conformance/fixtures/annotations_extra_round_trip.json`
for the locked-down test cases.

### Failure mode

- **Producers (post-migration):** none if you use the SDK constructors and
  serializers (`annotationsToJSON`, `dataclasses.asdict`, `serde_json::to_value`).
- **Consumers (post-migration):** legacy flattened input from `apcore-rust ≤ 0.17.1`
  is still accepted by all three SDKs for one MINOR cycle (will be removed in
  v0.19.0). When the same key appears in both nested and flat form, the
  **nested value wins** per spec rule 7. (This intentionally inverts the
  pre-v0.18 Python/TypeScript "overflow wins" behavior.)

### Migration

| Surface | Action |
|---|---|
| Hand-written JSON / YAML annotations | Move extension keys under a top-level `"extra": {...}` object |
| Python `ModuleAnnotations(extra={...})` | No change |
| TypeScript `createAnnotations({ extra: {...} })` | No change |
| Rust `ModuleAnnotations { extra, ..Default::default() }` | No change to construction; serializer output now matches Python/TS |

---

## 2. `apcore-rust` `Config` struct restructured

### What changed

`apcore-rust` v0.18.0 restructures the `Config` struct from a flat layout
(executor, observability, etc. as root fields) into nested sub-structs that
match PROTOCOL_SPEC §9.1 namespacing and the Python/TypeScript SDKs:

```rust
// v0.17.x
config.max_call_depth      // u32 at root
config.default_timeout_ms  // u64 at root

// v0.18.0
config.executor.max_call_depth     // u32 under ExecutorConfig
config.executor.default_timeout    // u64 under ExecutorConfig (no _ms suffix)
```

### Why

Three independent issues collapsed into one root cause:

1. **Spec §9.1 says** the canonical config keys are `executor.max_call_depth`,
   `executor.default_timeout`, etc. — namespaced under `executor.*`.
2. **Python and TypeScript implement the spec correctly** — both use namespaced
   key-value registries with `executor.max_call_depth` paths.
3. **Rust did not.** It declared typed fields at the root of `Config` and
   captured user namespaces into a `#[serde(flatten)] settings: HashMap` bag.

The consequence: a **spec-conformant YAML config silently failed to load on
Rust**. A user writing `executor: { max_call_depth: 100 }` would get the
default 32 because the typed field expected `max_call_depth: 100` at the root,
not nested. The misplaced data ended up in the unused `settings["executor"]`
HashMap entry. This was a long-standing functional bug in apcore-rust, not a
v0.18 regression.

### Failure mode (loud, by design)

Loading a v0.17.x-style config file with root-level executor fields now
produces a hard error pointing at this document:

```
ConfigInvalid: apcore v0.18.0 changed Config layout: root-level fields
'max_call_depth' → 'executor.max_call_depth', 'default_timeout_ms' →
'executor.default_timeout' are no longer accepted. Move them to their
canonical nested namespace. See MIGRATION-v0.18.md for the full migration
guide.
```

There is **no silent migration**. The conservative tradition would be to
add a deprecation shim that auto-migrates legacy fields with a warning, then
drop the shim a cycle later. We deliberately did not do this:

- The bug was that legacy YAMLs were already silently loaded **wrong** (default
  values, not user values). A warning-then-shim approach would mean continuing
  to ship a broken default-yields path under a different name.
- The user base in v0.17.x is small (early-stage development). Forcing a clean
  migration once is cheaper than carrying compat code for two MINOR cycles.
- A hard error with a specific fix instruction is friendlier than a warning
  the user might miss.

### Migration: Rust callsites

| Before (v0.17.x)                       | After (v0.18.0)                              |
|----------------------------------------|----------------------------------------------|
| `config.max_call_depth`                | `config.executor.max_call_depth`             |
| `config.max_module_repeat`             | `config.executor.max_module_repeat`          |
| `config.default_timeout_ms`            | `config.executor.default_timeout`            |
| `config.global_timeout_ms`             | `config.executor.global_timeout`             |
| `config.enable_tracing`                | `config.observability.tracing.enabled`       |
| `config.enable_metrics`                | `config.observability.metrics.enabled`       |
| `config.settings`                      | `config.user_namespaces`                     |
| `config.get("max_call_depth")`         | `config.get("executor.max_call_depth")`      |
| `config.set("default_timeout_ms", v)`  | `config.set("executor.default_timeout", v)`  |

The `_ms` suffix is dropped from `default_timeout` and `global_timeout` to
align with spec §9.1 and the Python/TypeScript SDKs. **Units are unchanged**
(still milliseconds); see the field doc comments.

### Migration: YAML / JSON config files

```diff
- max_call_depth: 32
- max_module_repeat: 3
- default_timeout_ms: 30000
- global_timeout_ms: 60000
- enable_tracing: true
- enable_metrics: false
+ executor:
+   max_call_depth: 32
+   max_module_repeat: 3
+   default_timeout: 30000
+   global_timeout: 60000
+ observability:
+   tracing:
+     enabled: true
+   metrics:
+     enabled: false
```

User-defined namespaces (`my-vendor: {...}`, `plugins: {...}`) are unchanged
— they still flow through the renamed `Config.user_namespaces` field.

### New public types

Three sub-structs are now part of the public Rust API and available via
`apcore::config::*` or the crate root:

- `ExecutorConfig`
- `ObservabilityConfig`
- `TracingConfig`
- `MetricsConfig`

`Config::bind::<ExecutorConfig>("executor")` returns the typed sub-struct
directly.

---

## 3. `apcore-python` legacy event aliases removed

### What changed

Four dual-emission code paths in `apcore-python` are deleted. Listeners that
subscribed to the legacy short-form event names will no longer receive events:

| Legacy name (removed)         | Canonical name (use this)        | Emitted by                                    |
|-------------------------------|----------------------------------|-----------------------------------------------|
| `module_health_changed`       | `apcore.module.toggled`          | `system.control.toggle_feature`               |
| `module_health_changed`       | `apcore.health.recovered`        | `PlatformNotifyMiddleware` (error rate recovery) |
| `config_changed`              | `apcore.config.updated`          | `system.control.update_config`                |
| `config_changed`              | `apcore.module.reloaded`         | `system.control.reload_module`                |

### Why

These dual-emissions were introduced in v0.15 with a removal deadline of
v0.16.0. The deadline was missed and the code continued to ship through
v0.17.x. v0.18.0 completes the cleanup that should have happened ~2 MINOR
releases ago.

### Failure mode

Listeners that subscribe to `module_health_changed` or `config_changed` will
silently stop receiving events. There is no warning at runtime — the events
simply cease.

### Migration

```python
# Before (v0.17.x and earlier)
emitter.subscribe("module_health_changed", my_handler)
emitter.subscribe("config_changed", my_handler)

# After (v0.18.0+)
emitter.subscribe("apcore.module.toggled", my_handler)
emitter.subscribe("apcore.health.recovered", my_handler)
emitter.subscribe("apcore.config.updated", my_handler)
emitter.subscribe("apcore.module.reloaded", my_handler)
```

If a single listener was handling both old names because they collided
(`module_health_changed` was used for two unrelated semantic events — toggles
AND error-rate recovery), you now need to subscribe to both canonical names
and dispatch internally. This is the whole point of the v0.15 §9.16 split:
the two old "module_health_changed" emissions were semantically different and
should never have shared a name.

### Internal rename

`apcore.sys_modules.control._emit_config_changed` is renamed to
`_emit_module_reloaded` to reflect the canonical event it emits. This is
private API; it does not affect external code.

---

## 4. Misc cleanup (no migration needed)

These are the non-breaking pieces of v0.18.0 that don't require user action:

- `apcore-typescript`: 7 unused imports removed from `src/`, 4 dead private
  fields removed from `Executor`, 1 dead `_envStyle` field removed from
  `Config`. Pure cleanup; zero behavior change.
- `apcore`: `docs/spec/design-context-annotations-acl.md` now carries a
  superseded banner pointing at PROTOCOL_SPEC §4.4.1.
- `apcore`: `mkdocs.yml` Home nav points at the existing `README.md` instead
  of a non-existent `index.md`.
- `apcore-python` / `apcore-typescript`: a code comment now explains why the
  legacy-mode `version` baseline is frozen at `0.16.0` (intentional, not drift).

---

## 5. Asking for help

If a config file or piece of code does not migrate cleanly with the table
above, the most likely causes are:

1. **Custom user namespace named `executor` or `observability`.** These names
   are now reserved for canonical sub-structs in apcore-rust. Rename your
   custom namespace to something else (e.g. `my-executor`).
2. **Hand-written JSON consumers that read flattened `extra` keys.** Switch
   to reading from the nested `extra` object.
3. **Python/TypeScript code that subscribes to legacy event names.** Update
   the subscription string per Section 3.

For anything else, file an issue with:
- Your apcore version (`apcore-python --version` or equivalent)
- A minimal config file or code snippet that fails
- The exact error message

---

## 6. Conformance verification

After migrating, verify cross-language behavior using the apcore conformance
fixtures:

```bash
# In any of the three SDK repos:
python -m pytest tests/test_conformance.py     # apcore-python
npx vitest run tests/conformance.test.ts       # apcore-typescript
cargo test --test conformance_test             # apcore-rust
```

The new `conformance/fixtures/annotations_extra_round_trip.json` fixture
locks the §4.4.1 wire format across all three SDKs. If your migration is
correct, all three should pass.
