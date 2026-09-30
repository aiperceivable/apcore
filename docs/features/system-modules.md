---
description: "Built-in system.* control-plane modules for AI introspection: health summary/module, manifest discovery, usage analytics, approval-gated update_config, reload_module, toggle_feature."
---

# System Modules

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §6.6 System Module Permissions and §6.7 (system module output contracts).


## Overview

The built-in `system.*` modules let AI agents and operators query and control a running apcore instance: health, module manifests, usage statistics, and — behind approval — runtime configuration, hot reload and feature toggles. They live in the reserved `system.*` namespace (PROTOCOL_SPEC §2.5, §6.6) and are registered through `register_internal()`.

`APCore` registers them automatically when the loaded configuration sets `sys_modules.enabled: true`; the read-only modules need nothing else, the three `system.control.*` modules also need `sys_modules.events.enabled: true`.

| Module | Purpose | Annotations |
|---|---|---|
| [`system.health.summary`](#systemhealthsummary) | Health status of every module (healthy / degraded / error / unknown) | readonly, idempotent |
| [`system.health.module`](#systemhealthmodule) | Health detail for one module: latency and recent errors | readonly, idempotent |
| [`system.manifest.module`](#systemmanifestmodule) | One module's schema, annotations, tags, source path | readonly, idempotent |
| [`system.manifest.full`](#systemmanifestfull) | Full registry manifest, filterable by tags and prefix | readonly, idempotent |
| [`system.usage.summary`](#systemusagesummary) | Usage statistics for every module, with trends | readonly, idempotent |
| [`system.usage.module`](#systemusagemodule) | One module's usage, per-caller breakdown, hourly distribution | readonly, idempotent |
| [`system.control.update_config`](#systemcontrolupdate_config) | Change a runtime configuration value | requires_approval |
| [`system.control.reload_module`](#systemcontrolreload_module) | Hot-reload one module, or every module matching a pattern | requires_approval |
| [`system.control.toggle_feature`](#systemcontroltoggle_feature) | Disable or re-enable a module without unloading it | requires_approval |

Every system module declares `open_world: false` explicitly — none of them reaches an external system (D-119).

## Activation

| Key | Default | Gates |
|---|---|---|
| `sys_modules.enabled` | `false` | Master switch. When `true`, the error-history and usage middlewares are installed and the read modules below are registered. |
| `sys_modules.health.enabled` | `true` | `system.health.*` |
| `sys_modules.manifest.enabled` | `true` | `system.manifest.*` |
| `sys_modules.usage.enabled` | `true` | `system.usage.*` (the usage middleware is installed regardless) |
| `sys_modules.events.enabled` | `false` | The `EventEmitter`, `PlatformNotifyMiddleware`, configured event subscribers, the registry event bridge — and the control modules |
| `sys_modules.control.enabled` | `true` | `system.control.*`, which register only when **both** `events.enabled` and `control.enabled` are `true` |
| `sys_modules.control.overrides_path` | none | File that persists runtime overrides (see [Persistent overrides](#persistent-overrides-pluggable-overridesstore)) |

With the defaults this gives three states (§6.6.3):

| Config | Modules registered |
|---|---|
| `sys_modules.enabled: false` | **0** |
| `sys_modules.enabled: true`, `events.enabled: false` | **6** — `system.health.*`, `system.usage.*`, `system.manifest.*` |
| `sys_modules.enabled: true`, `events.enabled: true` | **9** — the six plus the three `system.control.*` write modules |

A registry holding only the six read modules has no write surface; anything reasoning about exposure should distinguish the two enabled states.

The control modules declare `requires_approval: true`, so the approval gate applies to them. Approval, like ACL, is inactive by absence ([§6.6.3.1](../spec/protocol-spec.md#6631-layers-2-and-3-are-inactive-by-absence)): with no `ApprovalHandler` the gate is skipped with a warning unless `ExecutionPolicy(strict=true)` is set, and a missing ACL file attaches no ACL. To see what actually gates a registry, read [`executor.governance_state()`](./core-executor.md#governance-state-api) (§6.6.5).

## system.health.summary

Aggregated health overview of all registered modules.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `error_rate_threshold` | float | 0.01 | Upper bound of the `healthy` band |
| `include_healthy` | bool | true | Include healthy modules in output |

**Output:**

```json
{
  "project": { "name": "my-project" },
  "summary": {
    "total_modules": 12,
    "healthy": 10,
    "degraded": 1,
    "error": 1,
    "unknown": 0
  },
  "modules": [
    {
      "module_id": "math.add",
      "status": "healthy",
      "error_rate": 0.002,
      "top_error": null
    },
    {
      "module_id": "email.send",
      "status": "degraded",
      "error_rate": 0.05,
      "top_error": {
        "code": "MODULE_TIMEOUT",
        "message": "Module timed out",
        "ai_guidance": "consider increasing timeout",
        "count": 3
      }
    }
  ]
}
```

**Health classification:**

| Status | Condition |
|--------|-----------|
| healthy | `error_rate < error_rate_threshold` (default 1%) |
| degraded | `error_rate_threshold <= error_rate < 0.10` |
| error | `error_rate >= 0.10` |
| unknown | No calls recorded |

Only the healthy/degraded boundary is configurable; the degraded/error boundary is fixed at 10% (D-109). With `error_rate_threshold: 0.001`, `healthy` is `< 0.1%`, `degraded` spans `0.1%–10%`, and `error` still begins at 10%.

## Contract: system.health.summary

### Inputs
- `error_rate_threshold`: float, optional, default `0.01`
- `include_healthy`: bool, optional, default `true`

### Errors
- None beyond standard input-schema validation.

### Returns
- `dict` — `{project, summary, modules[]}` per the output shape above

### Properties
- idempotent: true
- thread_safe: true — read-only aggregation over already-collected call statistics
- async: false
- pure: true
- reentrant: true

## system.health.module

Detailed health information for a single module.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(required)* | Module to query |
| `error_limit` | int | 10 | Max recent errors to return |

`status` uses the default classification (healthy below 1%, error from 10%); this module takes no threshold input.

**Output:**

```json
{
  "module_id": "email.send",
  "status": "degraded",
  "total_calls": 1542,
  "error_count": 77,
  "error_rate": 0.05,
  "avg_latency_ms": 245.3,
  "p99_latency_ms": 1200.0,
  "recent_errors": [
    {
      "code": "MODULE_TIMEOUT",
      "message": "Module timed out",
      "ai_guidance": "consider increasing timeout",
      "count": 3,
      "first_occurred": "2026-03-08T10:00:00Z",
      "last_occurred": "2026-03-08T11:30:00Z"
    }
  ]
}
```

## Contract: system.health.module

### Inputs
- `module_id`: string, required
  - reject_with: `ModuleNotFoundError` if `module_id` is not registered
- `error_limit`: int, optional, default `10`

### Errors
- `ModuleNotFoundError` — `module_id` is not registered

### Returns
- On success: `dict` — per the Output shape above

### Properties
- idempotent: true
- thread_safe: true
- async: false
- pure: true — read-only lookup over already-collected call statistics
- reentrant: true

---

## system.manifest.module

Full manifest for a single registered module.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(required)* | Module to describe |

**Output:**

```json
{
  "module_id": "math.add",
  "description": "Add two numbers",
  "documentation": "Adds two integers and returns the sum.",
  "source_path": "extensions/math/add.py",
  "input_schema": { "type": "object", "properties": { "a": { "type": "integer" }, "b": { "type": "integer" } } },
  "output_schema": { "type": "object", "properties": { "sum": { "type": "integer" } } },
  "annotations": {
    "readonly": true,
    "idempotent": true,
    "requires_approval": false,
    "destructive": false,
    "discoverable": true
  },
  "tags": ["math", "utility"],
  "dependencies": [],
  "metadata": {}
}
```

## Contract: system.manifest.module

### Inputs
- `module_id`: string, required
  - reject_with: `ModuleNotFoundError` if `module_id` is not registered

### Errors
- `ModuleNotFoundError` — `module_id` is not registered

### Returns
- On success: `dict` — per the Output shape above

### Properties
- idempotent: true
- thread_safe: true
- async: false
- pure: true — read-only lookup over registry metadata
- reentrant: true

---

## system.manifest.full

Complete system manifest with filtering.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `include_schemas` | bool | true | Include input/output schemas |
| `include_source_paths` | bool | true | Include source file paths |
| `prefix` | string | *(none)* | Filter by module ID prefix |
| `tags` | list[string] | *(none)* | Filter by tags (all must match) |

**Output:**

```json
{
  "project_name": "my-project",
  "module_count": 1,
  "modules": [
    {
      "module_id": "math.add",
      "description": "Add two numbers",
      "documentation": null,
      "source_path": "extensions/math/add.py",
      "input_schema": { "type": "object" },
      "output_schema": { "type": "object" },
      "annotations": { "readonly": true },
      "tags": ["math"],
      "dependencies": [],
      "metadata": {}
    }
  ]
}
```

Each `modules[]` entry has the shape of `system.manifest.module`'s output. `project_name` is `project.name`, or `"apcore"` when it is not configured (D-110).

## Contract: system.manifest.full

### Inputs
- `include_schemas`: bool, optional, default `true`
- `include_source_paths`: bool, optional, default `true`
- `prefix`: string, optional — filters by module ID prefix
- `tags`: list[string], optional — filters by tags; all listed tags must match

### Errors
- No errors under normal operation

### Returns
- On success: `dict` — `{project_name: str, module_count: int, modules: [...]}`

### Properties
- idempotent: true
- thread_safe: true
- async: false
- pure: true — read-only lookup over registry metadata
- reentrant: true

---

## system.usage.summary

Usage overview with trend detection across all modules.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | string | "24h" | Time window, `^[1-9][0-9]*[hd]$` (e.g. `1h`, `24h`, `7d`). Declared as a `pattern` in `input_schema`, so a malformed value fails input validation with `SCHEMA_VALIDATION_ERROR`. |

**Output:**

```json
{
  "period": "24h",
  "total_calls": 15420,
  "total_errors": 77,
  "modules": [
    {
      "module_id": "math.add",
      "call_count": 5000,
      "error_count": 2,
      "avg_latency_ms": 12.5,
      "unique_callers": 8,
      "trend": "stable"
    }
  ]
}
```

Modules sorted by `call_count` descending.

**Trend values:** `stable`, `rising`, `declining`, `new`, `inactive` — decided by the normative threshold table in [PROTOCOL_SPEC §6.7.1.5](../spec/protocol-spec.md#6715-trend), comparing the requested window against the preceding window of equal length.

> **`period` is a filter, not an echo.** `total_calls`, `total_errors` and every field of every `modules[]` entry MUST be computed over `[now − period, now]`. Echoing `period` back while computing over the full retained history is a conformance failure — and a silent one, because the response names the window it did not apply. See [PROTOCOL_SPEC §6.7.1.1](../spec/protocol-spec.md#6711-period-is-a-filter-not-an-echo); canonical shape in [`schemas/sys-usage-summary.schema.json`](https://github.com/aiperceivable/apcore/blob/main/schemas/sys-usage-summary.schema.json).

## Contract: system.usage.summary

### Inputs
- `period`: string, optional, default `"24h"`
  - validation: MUST match `^[1-9][0-9]*[hd]$`
  - reject_with: `SCHEMA_VALIDATION_ERROR` — malformed value fails `input_schema` pattern validation

### Errors
- `SCHEMA_VALIDATION_ERROR` — `period` does not match the required pattern

### Returns
- On success: `dict` — per the Output shape above; every field computed over `[now − period, now]`, never over full retained history ([PROTOCOL_SPEC §6.7.1.1](../spec/protocol-spec.md#6711-period-is-a-filter-not-an-echo))

### Properties
- idempotent: true
- thread_safe: true
- async: false
- pure: true — reads already-collected usage data and computes trend classification; no mutation
- reentrant: true

---

## system.usage.module

Detailed usage for a single module with caller breakdown.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(required)* | Module to query |
| `period` | string | "24h" | Time window, `^[1-9][0-9]*[hd]$`. Same grammar and same filter semantics as `system.usage.summary`. |

**Output:**

```json
{
  "module_id": "math.add",
  "period": "24h",
  "call_count": 5000,
  "error_count": 2,
  "avg_latency_ms": 12.5,
  "p99_latency_ms": 45.0,
  "trend": "stable",
  "callers": [
    {
      "caller_id": "orchestrator.main",
      "call_count": 3000,
      "error_count": 1,
      "avg_latency_ms": 11.2
    }
  ],
  "hourly_distribution": [
    { "hour": "2026-03-08T00", "call_count": 0,   "error_count": 0 },
    "... 21 more buckets, one per hour, none omitted ...",
    { "hour": "2026-03-08T10", "call_count": 200, "error_count": 0 },
    { "hour": "2026-03-08T11", "call_count": 350, "error_count": 1 }
  ]
}
```

**`hourly_distribution` invariants** ([PROTOCOL_SPEC §6.7.1.2](../spec/protocol-spec.md#6712-hourly_distribution)):

- `hour` is the UTC bucket key `YYYY-MM-DDTHH` — **not** `YYYY-MM-DDTHH:00:00Z`. This is the key `UsageCollector` produces in every SDK; the module layer MUST NOT reformat it.
- Exactly **24** entries, covering `now−23h .. now`, ascending by `hour`, gaps zero-filled rather than omitted — so a consumer can index positionally.
- The 24-entry span is fixed. `period` filters the counts inside each bucket; it does not change the array length.

**`p99_latency_ms`** is the nearest-rank 99th percentile — `sorted[min(ceil(0.99·N), N) − 1]`, no interpolation, `0` when there are no samples. For 100 samples `1..100` the answer is **99**, not 100 ([PROTOCOL_SPEC §6.7.1.3](../spec/protocol-spec.md#6713-p99_latency_ms)).

**`callers[].caller_id`** is the literal `"unknown"` for a call recorded with no caller identity — never `null`, never omitted, never `@external`.

Canonical shape: [`schemas/sys-usage-module.schema.json`](https://github.com/aiperceivable/apcore/blob/main/schemas/sys-usage-module.schema.json).

## Contract: system.usage.module

### Inputs
- `module_id`: string, required
  - reject_with: `ModuleNotFoundError` if `module_id` is not registered
- `period`: string, optional, default `"24h"`
  - validation: MUST match `^[1-9][0-9]*[hd]$` — same grammar and filter semantics as `system.usage.summary`
  - reject_with: `SCHEMA_VALIDATION_ERROR`

### Errors
- `ModuleNotFoundError` — `module_id` is not registered
- `SCHEMA_VALIDATION_ERROR` — `period` does not match the required pattern

### Returns
- On success: `dict` — per the Output shape above, including the fixed 24-entry `hourly_distribution` and nearest-rank `p99_latency_ms`

### Properties
- idempotent: true
- thread_safe: true
- async: false
- pure: true — read-only over already-collected usage data
- reentrant: true

---


---

## system.control.update_config

Update a runtime configuration value by dot-path key.

**Annotations:** `requires_approval=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `key` | string | *(required)* | Dot-path config key (e.g., `executor.default_timeout`) |
| `value` | any | *(required)* | New value |
| `reason` | string | *(required)* | Audit reason |

**Output:**

```json
{
  "success": true,
  "key": "executor.default_timeout",
  "old_value": 30000,
  "new_value": 60000
}
```

- `sys_modules.enabled` cannot be changed (the only restricted key).
- A key is **sensitive** when any dot-separated segment (lowercased) equals, starts with `<s>_`, or ends with `_<s>`, for `<s>` in `token`, `secret`, `key`, `password`, `auth`, `credential`. Its old and new values are replaced with `***REDACTED***` in the response, the event and the audit entry (the overrides store receives the real value).
- The change is in memory; it is persisted only when an overrides store is configured (see [Persistent overrides](#persistent-overrides-pluggable-overridesstore)).
- Emits `apcore.config.updated`.

## Contract: system.control.update_config

### Inputs
- `key`: string, required, non-empty — else `InvalidInputError`
- `value`: any JSON value, required — constraint checking happens after the set
- `reason`: string, required, non-empty — else `InvalidInputError`

### Preconditions
- `key` is not restricted (`sys_modules.enabled`) — else `CONFIG_KEY_RESTRICTED`
- If `key` has a value constraint (see [Config Bus § Value constraints](./config-bus.md#value-constraints)), `value` satisfies it — else the config is rolled back to `old_value` and `ConfigError` (`CONFIG_INVALID`) is raised

### Side Effects (ordered)
1. Read the current value of `key` (`old_value`)
2. Set `key` to `value` in `Config`
3. Check the key's constraint; on failure roll back and raise
4. Persist to the overrides store, if one is configured (best effort: a write failure is logged, not raised)
5. Emit `apcore.config.updated` (values masked for sensitive keys)
6. Record an audit entry, if an `AuditStore` is configured
7. Log the change at INFO (values masked for sensitive keys)

### Postconditions
- On success: `config.get(key)` returns `value`
- On `CONFIG_INVALID`: `config.get(key)` returns `old_value`

### Errors
- `InvalidInputError` (`GENERAL_INVALID_INPUT`) — `key` or `reason` missing or empty
- `CONFIG_KEY_RESTRICTED` — `key` is restricted
- `ConfigError` (`CONFIG_INVALID`) — `value` violates the key's constraint

### Returns
- `{success: true, key, old_value, new_value}`; values masked for sensitive keys

### Properties
- idempotent: false
- thread_safe: false — `Config.set` is not internally locked; serialize concurrent callers
- async: false
- pure: false — mutates `Config` and emits an event
- reentrant: false

---

## system.control.reload_module

Hot-reload one module from disk — or every module matching a pattern — without restarting.

**Annotations:** `requires_approval=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | one of `module_id` / `path_filter` | Single module to reload |
| `path_filter` | string | one of `module_id` / `path_filter` | Pattern (Algorithm A25: `*` and `?` only) matched against registered module IDs, for a bulk reload |
| `reason` | string | *(required)* | Audit reason |

`module_id` and `path_filter` are mutually exclusive (`MODULE_RELOAD_CONFLICT`); omitting both is `GENERAL_INVALID_INPUT`. A `path_filter` matching nothing is a no-op with an empty `reloaded_modules`. Bulk reloads run in dependency order, leaves first. `reload_dependents` is deprecated and ignored (it warns once); to reload dependents, use a `path_filter` that covers them (D-121). Rust additionally accepts `reload_config: true`, which re-reads the configuration file when the module was constructed with a bound `Config`.

**Output (single module):**

```json
{
  "success": true,
  "module_id": "math.add",
  "previous_version": "1.0.0",
  "new_version": "1.1.0",
  "reload_duration_ms": 45.2
}
```

**Output (`path_filter`):**

```json
{
  "success": true,
  "module_id": null,
  "reloaded_modules": ["executor.email.send", "executor.math.add"],
  "reload_duration_ms": 120.7
}
```

Emits `apcore.module.reloaded` for each reloaded module.

## Contract: system.control.reload_module

### Inputs
- `module_id` or `path_filter` (exactly one), `reason` (required, non-empty)

### Preconditions
- For a single reload, `module_id` is registered — else `MODULE_NOT_FOUND`

### Side Effects (ordered, per module)
1. Read the current module and its version
2. Call `on_suspend()` if defined (best effort; errors are logged)
3. `safe_unregister(module_id)` — drain in-flight calls, then remove the module
4. Re-run discovery to load the module from disk
5. Register the new instance (running its `on_load`)
6. Call `on_resume(state)` if defined and a state was captured (best effort)
7. Emit `apcore.module.reloaded`; record an audit entry per module (bulk entries share one `correlation_id`)

If step 4 or 5 fails, see [Reload failure semantics](#reload-failure-semantics).

### Errors
- `InvalidInputError` (`GENERAL_INVALID_INPUT`) — `reason` missing, or neither `module_id` nor `path_filter`
- `ModuleReloadConflictError` (`MODULE_RELOAD_CONFLICT`) — both `module_id` and `path_filter`
- `ModuleNotFoundError` (`MODULE_NOT_FOUND`) — `module_id` is not registered
- `ReloadFailedError` (`RELOAD_FAILED`) — discovery failed or the module was missing afterwards

### Returns
- Single: `{success, module_id, previous_version, new_version, reload_duration_ms}`
- Bulk: `{success, module_id: null, reloaded_modules, reload_duration_ms}`
- Rust adds `config_reloaded` to both.

### Properties
- idempotent: false — each call unregisters and re-registers
- thread_safe: false — concurrent reloads are serialized only by `safe_unregister`
- async: false
- pure: false — mutates the registry, reads files, emits events
- reentrant: false

---

## system.control.toggle_feature

Disable or enable a module without unloading it.

**Annotations:** `requires_approval=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(required)* | Module to toggle |
| `enabled` | bool | *(required)* | `true` to enable, `false` to disable |
| `reason` | string | *(required)* | Audit reason |

**Output:**

```json
{
  "success": true,
  "module_id": "risky.module",
  "enabled": false
}
```

A disabled module stays registered; calls to it fail with `MODULE_DISABLED` at the module-lookup step. Toggle state is held by a thread-safe `ToggleState` that belongs to the owning `APCore` instance — disabling a module on one instance does not affect another in the same process — and survives reloads of that module. Emits `apcore.module.toggled`.

## Contract: system.control.toggle_feature

### Inputs
- `module_id`: string, required, non-empty — else `InvalidInputError`
- `enabled`: boolean, required (not null, string or number) — else `InvalidInputError`
- `reason`: string, required, non-empty — else `InvalidInputError`

### Preconditions
- `module_id` is registered — else `MODULE_NOT_FOUND`

### Side Effects (ordered)
1. Check the module exists
2. Update the `ToggleState` under its lock (disable or enable)
3. Persist the toggle to the overrides store, if one is configured
4. Emit `apcore.module.toggled`; record an audit entry, if an `AuditStore` is configured
5. Log at INFO

### Postconditions
- `enabled=false`: calls to `module_id` fail with `ModuleDisabledError` (`MODULE_DISABLED`)
- `enabled=true`: calls proceed normally
- The state persists across reloads of the module and is isolated to the owning `APCore` instance

### Errors
- `InvalidInputError` (`GENERAL_INVALID_INPUT`) — invalid input as above
- `ModuleNotFoundError` (`MODULE_NOT_FOUND`) — `module_id` is not registered

### Returns
- `{success: true, module_id, enabled}`

### Properties
- idempotent: true — toggling to the current state changes nothing (Rust's registration descriptor currently marks it `idempotent: false`)
- thread_safe: true
- async: false
- pure: false — mutates `ToggleState`, emits an event
- reentrant: false

## System-module output and audit conventions

- `system.manifest.full` reports `project_name: "apcore"` when `project.name` is not configured (D-110).
- A bulk reload writes one audit entry **per module**, and all entries from one operation share a `correlation_id` (D-111).
- An identity snapshot with no roles omits the `roles` key instead of emitting `[]` (D-118).
- Every `system.*` module declares `open_world: false` explicitly (D-119).

## Reload failure semantics

When a reload fails after the old instance was removed, the previous instance is restored (D-112):

1. Restoration is **compensating, not atomic**. The module is genuinely unregistered for a window, and a concurrent call in that window gets `MODULE_NOT_FOUND`.
2. The restored instance's `on_load` runs again (its `on_unload` already ran during the unregister).
3. If that restoring `on_load` also fails, the module may stay unavailable — publishing a module whose load hook failed is what [registration ordering](./registry-system.md#registration-ordering-invariants) forbids.
4. On the bulk path restoration is per module; modules that already reloaded successfully are not rolled back, and the operation reports failure (`RELOAD_FAILED`).

| SDK | Single-module failure | Bulk (`path_filter`) failure |
|---|---|---|
| Python | restore, raise `RELOAD_FAILED`; a failed restore is logged and the module stays unavailable | restore the failing module and raise; earlier modules stay reloaded |
| TypeScript | restore, raise `RELOAD_FAILED`; a failed restore propagates its own error | raises only if discovery itself throws; a module missing after discovery is restored and left out of `reloaded_modules`, and the call returns `success: true` |
| Rust | restore, return `RELOAD_FAILED` | restore every affected module and return `RELOAD_FAILED` if discovery fails or any module is missing |

## Registration

### With `APCore`

`APCore` calls `register_sys_modules` itself when constructed with a configuration that sets `sys_modules.enabled: true`, sharing its own `ToggleState`. It also installs the error-history and usage middlewares and, when events are enabled, the `EventEmitter`.

=== "Python"

    ```python
    from apcore import APCore, Config

    config = Config.load("apcore.yaml")  # sys_modules.enabled: true, events.enabled: true
    client = APCore(config=config)

    health = client.call("system.health.summary", {})
    usage = client.call("system.usage.summary", {"period": "24h"})

    # Convenience wrappers around system.control.toggle_feature:
    client.disable("some.module", reason="maintenance")
    client.enable("some.module", reason="done")
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const config = Config.load('apcore.yaml'); // sys_modules.enabled: true, events.enabled: true
    const client = new APCore({ config });

    const health = await client.call('system.health.summary', {});
    const usage = await client.call('system.usage.summary', { period: '24h' });

    // Convenience wrappers around system.control.toggle_feature:
    await client.disable('some.module', 'maintenance');
    await client.enable('some.module', 'done');
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, ModuleError};
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        // apcore.yaml: sys_modules.enabled: true, events.enabled: true
        let client = APCore::from_path("apcore.yaml")?;

        let health = client.call("system.health.summary", json!({}), None, None).await?;
        let usage = client.call("system.usage.summary", json!({ "period": "24h" }), None, None).await?;
        println!("{health} {usage}");

        // Convenience wrappers around system.control.toggle_feature:
        client.disable("some.module", Some("maintenance")).await?;
        client.enable("some.module", Some("done")).await?;
        Ok(())
    }
    ```

`disable` / `enable` raise `SysModulesDisabledError` (`SYS_MODULES_DISABLED`) when the client registered no system modules. They call `system.control.toggle_feature` through the executor — so the approval gate applies, and the module exists only when `sys_modules.events.enabled` is `true`.

### Calling `register_sys_modules` directly

`APCore` accepts no audit store or custom overrides store. To supply one, build the registry and executor yourself (with no `APCore` over the same registry — a second registration fails with `DUPLICATE_MODULE_ID` for every system module) and call `register_sys_modules`:

=== "Python"

    ```python
    from apcore import Config, Executor, FileOverridesStore, Registry, register_sys_modules
    from apcore.sys_modules.audit import InMemoryAuditStore

    config = Config.load("apcore.yaml")  # sys_modules.enabled: true, events.enabled: true
    registry = Registry(config=config)
    executor = Executor(registry, config=config)
    audit_store = InMemoryAuditStore()

    ctx = register_sys_modules(
        registry,
        executor,
        config,
        audit_store=audit_store,
        overrides_store=FileOverridesStore("/etc/apcore/overrides.yaml"),
        fail_on_error=True,  # raise SysModuleRegistrationError instead of logging
    )

    executor.call("system.control.toggle_feature", {"module_id": "system.usage.summary", "enabled": False, "reason": "maintenance"})
    for entry in audit_store.query(module_id="system.usage.summary"):
        print(entry.action, entry.actor_id, entry.change)
    ```

=== "TypeScript"

    ```typescript
    import {
      Config,
      Executor,
      FileOverridesStore,
      InMemoryAuditStore,
      Registry,
      registerSysModules,
    } from 'apcore-js';

    const config = Config.load('apcore.yaml'); // sys_modules.enabled: true, events.enabled: true
    const registry = new Registry({ config });
    const executor = new Executor({ registry, config });
    const auditStore = new InMemoryAuditStore();

    // Synchronous; positional (registry, executor, config, metricsCollector, options).
    const ctx = registerSysModules(registry, executor, config, null, {
      auditStore,
      overridesStore: new FileOverridesStore('/etc/apcore/overrides.yaml'),
      failOnError: true, // throw SysModuleRegistrationError instead of logging
    });

    await executor.call('system.control.toggle_feature', {
      module_id: 'system.usage.summary',
      enabled: false,
      reason: 'maintenance',
    });
    for (const entry of auditStore.query({ moduleId: 'system.usage.summary' })) {
      console.log(entry.action, entry.actorId, entry.change);
    }
    ```

=== "Rust"

    ```rust
    use std::path::Path;
    use std::sync::Arc;

    use apcore::sys_modules::{register_sys_modules_with_options, AuditStore, InMemoryAuditStore, SysModulesOptions};
    use apcore::{Config, Executor, FileOverridesStore, Registry};
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let config = Config::load(Path::new("apcore.yaml"))?; // sys_modules.enabled / events.enabled: true
        let registry = Arc::new(Registry::new());
        let executor = Executor::new(Arc::clone(&registry), config.clone());
        let audit_store = Arc::new(InMemoryAuditStore::new());

        let options = SysModulesOptions {
            audit_store: Some(audit_store.clone()),
            overrides_store: Some(Arc::new(FileOverridesStore::new("/etc/apcore/overrides.yaml"))),
            fail_on_error: true,
            ..SysModulesOptions::default()
        };
        let _ctx = register_sys_modules_with_options(Arc::clone(&registry), &executor, &config, None, options)?;

        executor
            .call(
                "system.control.toggle_feature",
                json!({ "module_id": "system.usage.summary", "enabled": false, "reason": "maintenance" }),
                None,
                None,
            )
            .await?;
        let entries = audit_store.query(Some("system.usage.summary"), None, None).await?;
        println!("{} audit entries", entries.len());
        Ok(())
    }
    ```

## Configuration

```yaml
sys_modules:
  enabled: true
  error_history:
    max_entries_per_module: 50    # per-module ring-buffer capacity
    max_total_entries: 1000       # total ring-buffer capacity
  events:
    enabled: true                 # required for the control modules
    thresholds:
      error_rate: 0.1             # apcore.health.error_threshold_exceeded
      latency_p99_ms: 5000.0      # apcore.health.latency_threshold_exceeded
    subscribers:
      - type: "webhook"
        url: "https://platform.example.com/events"
        headers:
          Authorization: "Bearer <token>"
  control:
    enabled: true                 # default
    overrides_path: "/var/lib/apcore/overrides.yaml"
```

`sys_modules.usage.retention_hours` and `sys_modules.usage.bucketing_strategy` are accepted but read by no SDK (see [Config Bus § Deprecated and inert keys](./config-bus.md#deprecated-and-inert-keys)).

## Persistent overrides — pluggable `OverridesStore`

Changes made by `update_config` and `toggle_feature` are in memory unless an overrides store is configured. With one, every change is written to it, and at startup its contents are applied **after** the base configuration, so a runtime override survives a restart and a manual edit of the base file.

- `sys_modules.control.overrides_path` configures a `FileOverridesStore` (YAML). Under `APCore` this is the only way to configure one. In Python and TypeScript it is loaded at startup and written on every change; in Rust it is loaded at startup, but the control modules registered by `APCore` do not write back to it.
- `register_sys_modules` also accepts an `overrides_store` of any implementation (`FileOverridesStore`, `InMemoryOverridesStore`, or your own); it takes precedence over `overrides_path`.
- With neither, overrides are in memory only.

The store interface is the **whole map** — two methods (D-47):

| Method | Behaviour |
|---|---|
| `load() → mapping` | Every persisted override. Returns an empty mapping, never an error, when the store is empty or its file does not exist yet. |
| `save(mapping)` | Replace the entire override set. A single-key change is a read-modify-write, which is what the control modules do. |

| SDK | Form | Sync / async |
|---|---|---|
| Python | `apcore.OverridesStore` (runtime-checkable Protocol) | sync |
| TypeScript | `OverridesStore` interface | `load()` / `save()` may return a value or a promise; an async `load()` is skipped at startup with a warning |
| Rust | `apcore::OverridesStore` trait (`Send + Sync`) | async; an injected store is not loaded at startup (a warning is logged) |

`FileOverridesStore` creates its file lazily on the first `save()`, so a fresh install behaves like a long-lived one. There is no configuration key for other backends (Redis, a remote config service), and SDKs ship none ([same rule as storage backends](./metrics-and-usage.md#pluggable-storage-backends)): implement the interface and pass the instance to `register_sys_modules`.

## Contract: OverridesStore.load

### Inputs
- None

### Errors
- None for a missing backing file — that is an empty store. Other failures (malformed data, a remote backend being unreachable) are implementation-defined.

### Returns
- The full overrides map; empty when nothing has been saved

### Properties
- async: SDK-specific (see above)
- thread_safe: not specified — a custom backend should document its own guarantees
- pure: true against a stable backing store
- idempotent: true

## Contract: OverridesStore.save

### Inputs
- `mapping` (required) — the **entire** overrides map to persist

### Errors
- Backend-specific failures are implementation-defined; the control modules log them and continue.

### Returns
- Nothing; `FileOverridesStore` creates its file on the first call if needed

### Properties
- async: SDK-specific
- thread_safe: not specified
- pure: false — replaces the persisted map
- idempotent: true — saving the same mapping twice leaves the same state

## Audit trail

The control modules record who changed what. Every state change produces an audit **event** on the event bus, and — when an `AuditStore` is configured — a persisted `AuditEntry`. Both are populated from the same snapshot of the caller, so `AuditEntry.actor_id` and the event's `caller_id` agree.

### AuditStore

| SDK | Interface | `query` |
|---|---|---|
| Python | `AuditStore` Protocol; `InMemoryAuditStore` in `apcore.sys_modules.audit` | `query(module_id=None, actor_id=None, since=None) -> list[AuditEntry]` (sync; `since` is an ISO 8601 string) |
| TypeScript | `AuditStore` interface; `InMemoryAuditStore` | `query({ moduleId?, actorId?, since? }): AuditEntry[]` (sync) |
| Rust | async `AuditStore` trait; `apcore::sys_modules::InMemoryAuditStore` | `query(module_id, actor_id, since: Option<DateTime<Utc>>).await -> Result<Vec<AuditEntry>, ModuleError>` |

All three also have `append(entry)`. The sys-modules `AuditEntry` is distinct from the ACL's `AuditEntry` exported at the package root.

```yaml
AuditEntry:
  timestamp: str        # ISO 8601
  action: str           # update_config | reload_module | toggle_feature
  target_module_id: str
  actor_id: str         # context.identity.id, or "@external"
  actor_type: str       # context.identity.type
  trace_id: str
  change:
    before: any
    after: any
  correlation_id: str   # shared by the entries of one bulk reload; "" otherwise (D-111)
```

TypeScript uses camelCase field names (`targetModuleId`, `actorId`, …).

### Audit event payload

The `data` of `apcore.config.updated`, `apcore.module.toggled` and `apcore.module.reloaded` carries the operation's fields plus the caller:

- `caller_id` — always present; `"@external"` when the context has no caller ID.
- `identity` — present only when the context has an identity: `id`, `type`, `roles` (omitted when empty), and the identity's other attributes, with the value of any attribute whose name contains `token`, `secret`, `password`, `passwd`, `key`, `auth`, `credential`, `cookie`, `session` or `bearer` replaced by `"<redacted>"`. Omitted — not `null` — when there is no identity.
- Raw credentials (bearer tokens, API keys) never appear in the payload.

```json
{
  "event_type": "apcore.config.updated",
  "module_id": "system.control.update_config",
  "severity": "info",
  "data": {
    "key": "executor.default_timeout",
    "old_value": 30000,
    "new_value": 60000,
    "caller_id": "ops.console",
    "identity": { "id": "user-42", "type": "user", "display_name": "alice@example.com" }
  }
}
```

```json
{
  "event_type": "apcore.module.toggled",
  "module_id": "risky.module",
  "severity": "info",
  "data": { "module_id": "risky.module", "enabled": false, "caller_id": "@external" }
}
```

The event bus is the minimum auditing surface: events are emitted whether or not an `AuditStore` is configured. Rust's `update_config` event additionally carries `reason`, `actor_id` and `actor_type`.

## Usage metrics in Prometheus

`PrometheusExporter` serves the usage collector's data alongside the call metrics when you attach it:

- `apcore_usage_calls_total{module_id, status}` — counter
- `apcore_usage_error_rate{module_id}` — gauge (0.0–1.0)
- `apcore_usage_p50_latency_ms`, `apcore_usage_p95_latency_ms`, `apcore_usage_p99_latency_ms` `{module_id}` — gauges

The exporter is constructed and started in code — there is no configuration key for it. The usage collector to attach is the one `register_sys_modules` created (it is in the returned context), and the metrics collector should be the one registration used:

=== "Python"

    ```python
    from apcore import Config, Executor, Registry, register_sys_modules
    from apcore.observability import MetricsCollector, PrometheusExporter

    config = Config.load("apcore.yaml")  # sys_modules.enabled: true
    registry = Registry(config=config)
    executor = Executor(registry, config=config)
    collector = MetricsCollector()

    ctx = register_sys_modules(registry, executor, config, metrics_collector=collector)
    exporter = PrometheusExporter(collector, usage_collector=ctx["usage_collector"])
    exporter.start(port=9090, path="/metrics")
    ```

=== "TypeScript"

    ```typescript
    import { Config, Executor, MetricsCollector, PrometheusExporter, Registry, registerSysModules } from 'apcore-js';

    const config = Config.load('apcore.yaml'); // sys_modules.enabled: true
    const registry = new Registry({ config });
    const executor = new Executor({ registry, config });
    const collector = new MetricsCollector();

    const ctx = registerSysModules(registry, executor, config, collector);
    const exporter = new PrometheusExporter({ collector, usageCollector: ctx.usageCollector });
    exporter.start({ port: 9090, path: '/metrics' });
    ```

=== "Rust"

    ```rust
    use std::path::Path;
    use std::sync::Arc;

    use apcore::{register_sys_modules, Config, Executor, MetricsCollector, PrometheusExporter, Registry};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let config = Config::load(Path::new("apcore.yaml"))?; // sys_modules.enabled: true
        let registry = Arc::new(Registry::new());
        let executor = Executor::new(Arc::clone(&registry), config.clone());
        let collector = MetricsCollector::new();

        // The collectors are cheap clones over shared state.
        let ctx = register_sys_modules(Arc::clone(&registry), &executor, &config, Some(collector.clone()))?;
        let exporter = PrometheusExporter::new(collector).with_usage_collector(ctx.usage_collector.clone());
        exporter.start(9090, "/metrics").await?;
        Ok(())
    }
    ```

## Contract: register_sys_modules

### Inputs

| SDK | Signature |
|---|---|
| Python | `register_sys_modules(registry, executor, config, metrics_collector=None, fail_on_error=False, audit_store=None, overrides_store=None, toggle_state=None) -> dict` |
| TypeScript | `registerSysModules(registry, executor, config, metricsCollector?, options?: { failOnError?, overridesPath?, overridesStore?, auditStore?, toggleState? }): SysModulesContext` (synchronous) |
| Rust | `register_sys_modules(registry: Arc<Registry>, executor: &Executor, config: &Config, metrics_collector: Option<MetricsCollector>)` and `register_sys_modules_with_options(..., options: SysModulesOptions)` → `Result<SysModulesContext, SysModuleError>` |

- `metrics_collector` — created when omitted and needed.
- `toggle_state` — defaults to the process-global state; `APCore` passes its own.
- `fail_on_error` — when `true`, a module that fails to register raises; when `false` (default) the failure is logged and registration continues.

### Errors

- `SysModuleRegistrationError` (`SYS_MODULE_REGISTRATION_FAILED`) — a system module failed to register, raised only with `fail_on_error` (Rust: `SysModuleError::RegistrationFailed { module_id, source }`).

### Returns

- The registration context: the error history, usage collector and their middlewares, the event emitter and notify middleware (when events are enabled), the toggle state and the overrides / audit stores in use. Python returns a `dict` (empty when `sys_modules.enabled` is false).

### Properties

- async: false
- thread_safe: false — call once at startup, before serving requests
- pure: false — registers modules and installs middleware
- idempotent: false — a second call on the same registry fails with `DUPLICATE_MODULE_ID` for every system module and installs the middlewares again

## Contract: check_module_disabled / is_module_disabled

`is_module_disabled(module_id) -> bool` and `check_module_disabled(module_id)` (raises `ModuleDisabledError` / `MODULE_DISABLED` when disabled) are free functions in all three SDKs (`isModuleDisabled` / `checkModuleDisabled` in TypeScript; Python: `apcore.sys_modules.control`).

### Inputs

| Parameter | Type | Required | Description |
|---|---|---|---|
| `module_id` | string | yes | Module ID to inspect |

### Errors

- `is_module_disabled`: none; `false` for unknown IDs.
- `check_module_disabled`: `ModuleDisabledError` (`MODULE_DISABLED`) when the module is disabled.

### Returns

`is_module_disabled`: `true` if disabled. `check_module_disabled`: nothing.

### Properties

- pure: true — reads toggle state only

These functions read the **process-global** `ToggleState`. An `APCore` instance keeps its own, so toggles made through `client.disable()` are not visible to them — ask the client's toggle state instead (`client.toggle_state` in Python, `client.toggleState` in TypeScript, `client.toggle_state()` in Rust). The execution pipeline always consults the owning instance's state.

## Dependencies

- `Registry` — module lookup and registration (`register_internal()` bypasses the reserved-namespace check for `system.*`).
- `Executor` — execution and middleware.
- `Config` — `sys_modules.*` values, runtime updates.
- `MetricsCollector` — call counts and latency for the health modules.
- `ErrorHistory` — recent errors for the health modules ([Error History](./error-history.md)).
- `UsageCollector` — call tracking for the usage modules.
- `EventEmitter` — events from the control modules.

??? info "Python SDK reference"
    Not a protocol requirement — the `apcore-python` source layout.

    | File | Purpose |
    |------|---------|
    | `src/apcore/sys_modules/registration.py` | `register_sys_modules()`, subscriber factory |
    | `src/apcore/sys_modules/health.py` | `HealthSummaryModule`, `HealthModuleModule` |
    | `src/apcore/sys_modules/manifest.py` | `ManifestModuleModule`, `ManifestFullModule` |
    | `src/apcore/sys_modules/usage.py` | `UsageSummaryModule`, `UsageModuleModule` |
    | `src/apcore/sys_modules/control.py` | `UpdateConfigModule`, `ReloadModuleModule`, `ToggleFeatureModule`, `ToggleState` |
    | `src/apcore/sys_modules/overrides.py` | `OverridesStore`, `FileOverridesStore`, `InMemoryOverridesStore` |
    | `src/apcore/sys_modules/audit.py` | `AuditStore`, `InMemoryAuditStore`, `AuditEntry` |

## Testing strategy

- **Health** — classification thresholds, error aggregation from `ErrorHistory`, latency from `MetricsCollector`.
- **Manifest** — schema and annotation extraction, prefix and tag filtering, source paths, `project_name` default.
- **Usage** — call counting, `period` filtering, trend classification, the 24-bucket hourly distribution, nearest-rank p99, per-caller breakdown.
- **Control** — approval requirement; config update with constraint rollback and sensitive-key masking; reload lifecycle and failure restoration; `path_filter` / `module_id` exclusivity; toggle persistence across reload and isolation between instances.
- **Persistence and audit** — overrides applied after the base config; `OverridesStore` whole-map surface (`conformance/fixtures/overrides_store.json`); audit entries and event payloads (`@external`, identity redaction, correlation IDs).
- **Registration** — gating by each `sys_modules.*` key; `fail_on_error`.
