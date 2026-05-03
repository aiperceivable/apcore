# System Modules

## Overview

Built-in `system.*` modules provide AI bidirectional introspection — allowing AI agents to query, monitor, and control the apcore runtime. System modules are registered automatically when `sys_modules.enabled: true` in config, and use the reserved `system.*` namespace (see PROTOCOL_SPEC §2.5, §6.6).

## Requirements

### Health Monitoring
- `system.health.summary` — Aggregate health status across all modules with classification (healthy / degraded / error / unknown).
- `system.health.module` — Per-module health detail with latency metrics and recent errors.

### Manifest & Discovery
- `system.manifest.module` — Single module introspection (schema, annotations, tags, source path).
- `system.manifest.full` — Full registry manifest with filtering by tags and prefix.

### Usage Analytics
- `system.usage.summary` — Usage statistics across all modules with trend detection.
- `system.usage.module` — Per-module usage detail with caller breakdown and hourly distribution.

### Runtime Control
- `system.control.update_config` — Hot-patch runtime config values with constraint validation.
- `system.control.reload_module` — Hot-reload a module from disk without restart.
- `system.control.toggle_feature` — Enable/disable modules at runtime with reason tracking.

Control modules require `requires_approval: true` and are only registered when `sys_modules.enabled: true`.

## Module Reference

### system.health.summary

Aggregated health overview of all registered modules.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `error_rate_threshold` | float | 0.01 | Threshold for healthy status (1%) |
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
| healthy | error rate < 1% (configurable via `error_rate_threshold`) |
| degraded | error rate 1% – 10% |
| error | error rate >= 10% |
| unknown | No calls recorded |

---

### system.health.module

Detailed health information for a single module.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(required)* | Module to query |
| `error_limit` | int | 10 | Max recent errors to return |

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

---

### system.manifest.module

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
    "destructive": false
  },
  "tags": ["math", "utility"],
  "dependencies": [],
  "metadata": {}
}
```

---

### system.manifest.full

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
  "module_count": 5,
  "modules": [ ... ]
}
```

---

### system.usage.summary

Usage overview with trend detection across all modules.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | string | "24h" | Time period |

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

**Trend values:** `stable`, `rising`, `declining`, `new`, `inactive`

---

### system.usage.module

Detailed usage for a single module with caller breakdown.

**Annotations:** `readonly=True`, `idempotent=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(required)* | Module to query |
| `period` | string | "24h" | Time period |

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
    { "hour": "2026-03-08T10:00:00Z", "call_count": 200, "error_count": 0 },
    { "hour": "2026-03-08T11:00:00Z", "call_count": 350, "error_count": 1 }
  ]
}
```

Hourly distribution padded with zeros for gaps.

---

### system.control.update_config

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

**Restrictions:**
- Cannot change `sys_modules.enabled` (restricted key).
- Sensitive keys (containing `token`, `secret`, `key`, `password`, `auth`, `credential`) are logged with masked values.
- Changes are in-memory only; not persisted to YAML.
- Emits `apcore.config.updated` event.

## Contract: system.control.update_config

### Inputs
- `key`: string, required
  - validation: non-empty string
  - reject_with: `InvalidInputError(message="'key' is required and must not be empty")`
- `value`: any, required
  - validation: none — any JSON-serializable value accepted; constraint checking applied post-set
- `reason`: string, required
  - validation: non-empty string
  - reject_with: `InvalidInputError(message="'reason' is required and must not be empty")`

### Preconditions
- `key` must not be in the restricted keys set (currently: `sys_modules.enabled`)
  - reject_with: `ModuleError(code=CONFIG_KEY_RESTRICTED)`
- If `key` has a registered constraint, `value` must satisfy it; checked immediately after `Config.set`
  - reject_with: `ConfigError` — `Config` is rolled back to `old_value` before raising

### Side Effects (ordered)
1. Read current value of `key` from `Config` (captures `old_value`)
2. Set `key` to `value` in `Config` (in-memory only; not persisted to YAML)
3. Validate constraint for `key` if one exists; on failure, roll back and raise `ConfigError`
4. Emit `apcore.config.updated` event via `EventEmitter` (values masked for sensitive keys)
5. Log change at INFO level (values masked for sensitive keys)

### Postconditions
- On success: `config.get(key)` returns `value`
- On `ConfigError`: `config.get(key)` returns the original `old_value` (atomically rolled back)

### Errors
- `InvalidInputError` — `key` is absent or empty; or `reason` is absent or empty
- `ModuleError(code=CONFIG_KEY_RESTRICTED)` — `key` is in the restricted set
- `ConfigError` — `value` violates a registered constraint; `Config` rolled back before raising

### Returns
- On success: `dict` — `{success: true, key: str, old_value: any, new_value: any}`
  - `old_value` and `new_value` replaced with redaction sentinel for sensitive key segments

### Properties
- idempotent: false — repeated calls with different values produce different state
- thread_safe: false — `Config.set` is not internally locked; concurrent callers must serialize
- async: false
- pure: false — mutates `Config` state and emits an event
- reentrant: false

---

### system.control.reload_module

Hot-reload a module from disk without restart.

**Annotations:** `requires_approval=True`

**Input:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(required)* | Module to reload |
| `reason` | string | *(required)* | Audit reason |

**Output:**

```json
{
  "success": true,
  "module_id": "math.add",
  "previous_version": "1.0.0",
  "new_version": "1.1.0",
  "reload_duration_ms": 45.2
}
```

**Process:** `safe_unregister()` with drain → `discover()` re-load → re-register → emit `apcore.module.reloaded` event.

## Contract: system.control.reload_module

### Inputs
- `module_id`: string, required
  - validation: non-empty string
  - reject_with: `InvalidInputError`
- `reason`: string, required
  - validation: non-empty string
  - reject_with: `InvalidInputError(message="'reason' is required and must be a non-empty string")`

### Preconditions
- `module_id` must be present in the `Registry` before reload begins
  - reject_with: `ModuleNotFoundError(module_id=module_id)`

### Side Effects (ordered)
1. Read current module from `Registry` to capture `previous_version`
2. Call `module.on_suspend()` if the method is defined — captures suspended state; errors are logged at ERROR and suppressed (best-effort)
3. Call `registry.safe_unregister(module_id)` — drains in-flight calls then removes the module
4. Call `registry.discover()` to reload module source from disk; if `module_id` is absent after discovery, raise `ReloadFailedError`
5. Call `registry.register_internal(module_id, new_module)` to re-register the freshly loaded module
6. Call `new_module.on_resume(suspended_state)` if the method is defined and state is non-`None`; errors are logged at ERROR and suppressed (best-effort)
7. Emit `apcore.module.reloaded` event via `EventEmitter`
8. Log reload at INFO level

### Postconditions
- On success: `registry.get(module_id)` returns a freshly loaded module instance
- If step 4 raises, `module_id` is unregistered and callers must handle the partial state

### Errors
- `InvalidInputError` — `module_id` or `reason` is absent, wrong type, or empty
- `ModuleNotFoundError` — `module_id` is not registered before reload begins
- `ReloadFailedError` — `registry.discover()` raised or `module_id` was absent after discovery

### Returns
- On success: `dict` — `{success: true, module_id: str, previous_version: str, new_version: str, reload_duration_ms: float}`

### Properties
- idempotent: false — each call unregisters and re-registers; invoking twice reloads twice
- thread_safe: false — concurrent reload calls are not serialized beyond `safe_unregister`
- async: false
- pure: false — mutates `Registry`, performs file I/O, emits an event
- reentrant: false

---

### system.control.toggle_feature

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

Disabled modules remain registered but calls raise `ModuleDisabledError`. Toggle state is thread-safe (via `ToggleState` class) and survives module reload. Emits `apcore.module.toggled` event.

## Contract: system.control.toggle_feature

### Inputs
- `module_id`: string, required
  - validation: non-empty string
  - reject_with: `InvalidInputError(message="'module_id' is required and must be a non-empty string")`
- `enabled`: bool, required
  - validation: must be a `bool` instance (not `None`, not a string or integer)
  - reject_with: `InvalidInputError(message="'enabled' is required and must be a boolean")`
- `reason`: string, required
  - validation: non-empty string
  - reject_with: `InvalidInputError(message="'reason' is required and must be a non-empty string")`

### Preconditions
- `module_id` must be registered in the `Registry`
  - reject_with: `ModuleNotFoundError(module_id=module_id)`

### Side Effects (ordered)
1. Query `Registry.has(module_id)` (read-only existence check)
2. Acquire internal lock on `ToggleState._lock`
3. Mutate `ToggleState._disabled` set: add `module_id` when `enabled=false`; discard it when `enabled=true`
4. Release `ToggleState._lock`
5. Emit `apcore.module.toggled` event via `EventEmitter`
6. Log toggle at INFO level

### Postconditions
- When `enabled=false`: `is_module_disabled(module_id)` returns `true`; calls raise `ModuleDisabledError(code=MODULE_DISABLED)`
- When `enabled=true`: `is_module_disabled(module_id)` returns `false`; module calls proceed normally
- Toggle state persists across module reload (held by `ToggleState`, external to `Registry`)

### Errors
- `InvalidInputError` — `module_id` is absent/empty, `enabled` is absent/non-boolean, or `reason` is absent/empty
- `ModuleNotFoundError` — `module_id` is not registered in the `Registry`

### Returns
- On success: `dict` — `{success: true, module_id: str, enabled: bool}`

### Properties
- idempotent: true — toggling to the current state produces the same outcome
- thread_safe: true — `ToggleState` uses `threading.Lock` to serialize all mutations
- async: false
- pure: false — mutates `ToggleState`, emits an event, writes a log entry
- reentrant: false

## Registration & Setup

### Auto-Registration via `register_sys_modules()`

```python
from apcore.sys_modules.registration import register_sys_modules

context = register_sys_modules(
    registry=registry,
    executor=executor,
    config=config,
    metrics_collector=None,  # auto-created if needed
)
```

**Workflow:**

1. Check `config.get("sys_modules.enabled")` — exit if `false`.
2. Create `ErrorHistory` and register `ErrorHistoryMiddleware`.
3. Create `UsageCollector` and register `UsageMiddleware`.
4. Register health, manifest, and usage modules.
5. If `sys_modules.events.enabled`:
   - Create `EventEmitter` and `PlatformNotifyMiddleware`.
   - Register control modules.
   - Instantiate event subscribers from config.
   - Bridge registry events to EventEmitter.

**Return value:**

```python
{
    "error_history": ErrorHistory,
    "error_history_middleware": ErrorHistoryMiddleware,
    "usage_collector": UsageCollector,
    "usage_middleware": UsageMiddleware,
    "event_emitter": EventEmitter,               # if events enabled
    "platform_notify_middleware": PlatformNotifyMiddleware,  # if events enabled
}
```

### Via APCore Client (Recommended)

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # System modules auto-registered! Query them directly:
    health = client.call("system.health.summary", {})
    usage = client.call("system.usage.summary", {"period": "24h"})

    # Control via convenience methods:
    client.disable("some.module", reason="maintenance")
    client.enable("some.module", reason="done")
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const config = Config.load('apcore.yaml');
    const client = new APCore({ config });

    // System modules auto-registered! Query them directly:
    const health = await client.call('system.health.summary', {});
    const usage = await client.call('system.usage.summary', { period: '24h' });

    // Control via convenience methods:
    await client.disable('some.module', 'maintenance');
    await client.enable('some.module', 'done');
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use serde_json::json;

    let client = APCore::from_path("apcore.yaml")?;

    // System modules auto-registered! Query them directly:
    let health = client.call("system.health.summary", json!({}), None, None).await?;
    let usage = client.call("system.usage.summary", json!({"period": "24h"}), None, None).await?;

    // Control via convenience methods:
    client.disable("some.module", Some("maintenance"))?;
    client.enable("some.module", Some("done"))?;
    ```

## Configuration

```yaml
sys_modules:
  enabled: true
  error_history:
    max_entries_per_module: 50    # Ring buffer per-module capacity
    max_total_entries: 1000       # Ring buffer total capacity
  events:
    enabled: true                 # Required for control modules
    thresholds:
      error_rate: 0.1             # 10% triggers apcore.error.threshold_exceeded
      latency_p99_ms: 5000.0      # 5s triggers apcore.latency.threshold_exceeded
    subscribers:
      - type: "webhook"
        url: "https://platform.example.com/events"
        headers:
          Authorization: "Bearer token"
```

## Dependencies

### Internal
- `Registry` — Module lookup and registration.
- `Executor` — Module execution and middleware management.
- `Config` — Configuration values and hot reload.
- `MetricsCollector` — Call counts and latency histograms for health modules.
- `ErrorHistory` — Recent error tracking for health modules.
- `UsageCollector` — Call tracking for usage modules.
- `EventEmitter` — Event dispatch for control modules.

### Permissions
System modules use the reserved `system.*` namespace. Registration bypasses reserved word checks via `registry.register_internal()`. See [PROTOCOL_SPEC §6.6](../../PROTOCOL_SPEC.md) for the defense-in-depth permission model.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `src/apcore/sys_modules/registration.py` | `register_sys_modules()`, subscriber factory registry |
    | `src/apcore/sys_modules/health.py` | `HealthSummaryModule`, `HealthModuleModule` |
    | `src/apcore/sys_modules/manifest.py` | `ManifestModuleModule`, `ManifestFullModule` |
    | `src/apcore/sys_modules/usage.py` | `UsageSummaryModule`, `UsageModuleModule` |
    | `src/apcore/sys_modules/control.py` | `UpdateConfigModule`, `ReloadModuleModule`, `ToggleFeatureModule`, `ToggleState` |

## Contract: checkModuleDisabled / check_module_disabled

### Inputs

| Parameter | Type | Required | Description |
|---|---|---|---|
| `module_id` | `str` | Yes | Fully-qualified module ID to inspect. |
| `registry` | `Registry` | Yes | Registry that holds toggle state. |

### Errors

| Code | Condition |
|---|---|
| `MODULE_DISABLED` | The module's current `ToggleState` is `DISABLED`. |

### Returns

`None` — raises/throws on disabled; returns normally when enabled.

### Properties

- **Pure:** Yes — reads registry state only, no side effects.
- **Throws:** `ModuleDisabledError` (code `MODULE_DISABLED`).

---

## Contract: isModuleDisabled / is_module_disabled

### Inputs

| Parameter | Type | Required | Description |
|---|---|---|---|
| `module_id` | `str` | Yes | Fully-qualified module ID to inspect. |
| `registry` | `Registry` | Yes | Registry that holds toggle state. |

### Errors

- None — this function never raises; returns `false` for unknown module IDs.

### Returns

`bool` — `true` if disabled, `false` if enabled or toggle state not set.

### Properties

- **Pure:** Yes — reads registry state only, no side effects.
- **Does not throw.**

---

## Testing Strategy

- **Health modules**: Verify status classification thresholds, error aggregation from ErrorHistory, latency metrics from MetricsCollector.
- **Manifest modules**: Verify schema/annotation extraction, prefix/tag filtering, source path computation.
- **Usage modules**: Verify call counting, trend computation, hourly distribution padding, per-caller breakdown.
- **Control modules**: Verify approval requirement, config update with constraint validation, module reload lifecycle, toggle state persistence across reload.
- **Registration**: Verify auto-registration workflow, config-driven subscriber creation, middleware ordering.

---

## System Modules Hardening (Issue #45)

### 1.1 Config and Feature Toggle Persistence

Currently `system.control.update_config` and `system.control.toggle_feature` changes are in-memory only (see [line 299](#systemcontrolupdate_config)).

#### Normative Rules

- Implementations MUST support an optional `overrides_path` configuration field. When set, changes from `system.control.update_config` and `system.control.toggle_feature` MUST be persisted to `overrides_path` as YAML.
- The overrides file MUST be loaded on startup and applied AFTER the base config, so manual restores of the base config do not erase runtime overrides.
- Implementations SHOULD support KV-store persistence (Redis, etcd) as an alternative backend via a pluggable `OverridesStore` interface with methods: `set(key, value)`, `get(key) → value | None`, `get_all() → dict`, `delete(key)`.
- When no `overrides_path` or KV store is configured, the existing in-memory-only behavior MUST be preserved (backward compatible).

#### YAML Configuration

```yaml
sys_modules:
  control:
    overrides_path: "/etc/apcore/overrides.yaml"
    # OR
    overrides_store:
      type: "redis"
      url: "redis://localhost:6379"
      key_prefix: "apcore:overrides:"
```

#### Usage Examples

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    # Startup: load base config, then apply overrides from disk
    config = Config.load("apcore.yaml")
    # overrides_path is declared in apcore.yaml under sys_modules.control
    client = APCore(config=config)

    # Runtime update — persisted to overrides_path automatically
    await client.executor.call(
        "system.control.update_config",
        {"key": "executor.default_timeout", "value": 60000, "reason": "increase timeout"},
        context,
    )
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    // Startup: load base config, then apply overrides from disk
    const config = Config.load('apcore.yaml');
    // overrides_path is declared in apcore.yaml under sys_modules.control
    const client = new APCore({ config });

    // Runtime update — persisted to overrides_path automatically
    await client.executor.call(
        'system.control.update_config',
        { key: 'executor.default_timeout', value: 60000, reason: 'increase timeout' },
        context,
    );
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use serde_json::json;

    // Startup: load base config, then apply overrides from disk
    // overrides_path is declared in apcore.yaml under sys_modules.control
    let client = APCore::from_path("apcore.yaml")?;

    // Runtime update — persisted to overrides_path automatically
    client.executor.call(
        "system.control.update_config",
        json!({
            "key": "executor.default_timeout",
            "value": 60000,
            "reason": "increase timeout"
        }),
        None,
        None,
    ).await?;
    ```

#### Persistent Overrides — pluggable `OverridesStore`

All three SDKs ship a pluggable `OverridesStore` interface plus a `FileOverridesStore` implementation backed by `overrides_path`. This brings TypeScript to parity with the Python and Rust SDKs, which had file-backed override persistence prior to v0.20.

The interface MUST expose:

| Method | Behavior |
|---|---|
| `save(key, value)` | Persist a single override; subsequent reads MUST return the new value |
| `get(key) → value \| None` | Return the persisted override or `None`/`null` if absent |
| `get_all() → dict` | Snapshot of every persisted override (used at startup to apply on top of base config) |
| `delete(key)` | Remove an override; deleting an absent key MUST be a no-op |

Wiring during APCore construction:

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config
    from apcore.sys_modules.overrides import FileOverridesStore, InMemoryOverridesStore

    config = Config.load("apcore.yaml")

    # Production: persist runtime overrides to disk
    overrides_store = FileOverridesStore(path="/etc/apcore/overrides.yaml")
    client = APCore(config=config, overrides_store=overrides_store)

    # Tests: in-memory only, no disk side effects
    test_client = APCore(config=config, overrides_store=InMemoryOverridesStore())
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from "apcore-js";
    import { FileOverridesStore, InMemoryOverridesStore } from "apcore-js/sys-modules/overrides";

    const config = Config.load("apcore.yaml");

    // Production: persist runtime overrides to disk
    const overridesStore = new FileOverridesStore({ path: "/etc/apcore/overrides.yaml" });
    const client = new APCore({ config, overridesStore });

    // Tests: in-memory only, no disk side effects
    const testClient = new APCore({ config, overridesStore: new InMemoryOverridesStore() });
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use apcore::sys_modules::overrides::{FileOverridesStore, InMemoryOverridesStore, OverridesStore};
    use std::sync::Arc;

    // Production: persist runtime overrides to disk
    let store: Arc<dyn OverridesStore> = Arc::new(FileOverridesStore::new("/etc/apcore/overrides.yaml"));
    let client = APCore::from_path("apcore.yaml")?
        .with_overrides_store(store);

    // Tests: in-memory only, no disk side effects
    let test_store: Arc<dyn OverridesStore> = Arc::new(InMemoryOverridesStore::new());
    let test_client = APCore::from_path("apcore.yaml")?
        .with_overrides_store(test_store);
    ```

`FileOverridesStore` MUST treat a missing `path` on first run as an empty store (no error) — the file is created lazily on the first `save()` call. This makes first-run, fresh-install, and ephemeral CI environments behave identically to long-lived installations.

---

### 1.2 Contextual Audit Trail

System control modules that modify state MUST record an audit entry for every change.

#### Normative Rules

- `system.control.update_config`, `system.control.reload_module`, and `system.control.toggle_feature` MUST extract the caller identity from `context.identity` and record it in a structured audit entry.
- Each audit entry MUST contain: `timestamp`, `module_id` (the target), `action` (update_config/reload_module/toggle_feature), `actor_id` (from `context.identity.id`), `actor_type` (from `context.identity.type`), `change` (before/after for config; enabled/disabled for toggle; module_version for reload).
- Implementations MUST support an `AuditStore` interface with `append(entry)` and `query(module_id?, actor_id?, since?) → List[AuditEntry]`.
- When no AuditStore is configured, audit entries SHOULD be logged at INFO level and discarded (not stored).

#### AuditEntry Schema

```yaml
AuditEntry:
  timestamp: str      # ISO 8601
  action: enum        # update_config | reload_module | toggle_feature
  target_module_id: str
  actor_id: str
  actor_type: str     # user | service | agent | api_key | system
  trace_id: str
  change:
    before: any       # previous value / null
    after: any        # new value / null
```

#### Usage Examples

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config
    from apcore.sys_modules.audit import InMemoryAuditStore

    config = Config.load("apcore.yaml")
    audit_store = InMemoryAuditStore()

    client = APCore(config=config, audit_store=audit_store)

    # After a control call, query the audit log
    await client.executor.call(
        "system.control.toggle_feature",
        {"module_id": "risky.module", "enabled": False, "reason": "maintenance"},
        context,
    )

    entries = audit_store.query(module_id="risky.module")
    # entries[0].actor_id == context.identity.id
    # entries[0].change == {"before": True, "after": False}
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';
    import { InMemoryAuditStore } from 'apcore-js/sys-modules/audit';

    const config = Config.load('apcore.yaml');
    const auditStore = new InMemoryAuditStore();

    const client = new APCore({ config, auditStore });

    // After a control call, query the audit log
    await client.executor.call(
        'system.control.toggle_feature',
        { module_id: 'risky.module', enabled: false, reason: 'maintenance' },
        context,
    );

    const entries = await auditStore.query({ moduleId: 'risky.module' });
    // entries[0].actorId === context.identity.id
    // entries[0].change === { before: true, after: false }
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use apcore::sys_modules::audit::InMemoryAuditStore;
    use std::sync::Arc;
    use serde_json::json;

    let audit_store = Arc::new(InMemoryAuditStore::new());
    let client = APCore::from_path("apcore.yaml")?
        .with_audit_store(audit_store.clone());

    // After a control call, query the audit log
    client.executor.call(
        "system.control.toggle_feature",
        json!({ "module_id": "risky.module", "enabled": false, "reason": "maintenance" }),
        None,
        None,
    ).await?;

    let entries = audit_store.query(Some("risky.module"), None, None)?;
    // entries[0].actor_id == context.identity.id
    // entries[0].change.before == Some(json!(true)), entries[0].change.after == Some(json!(false))
    ```

---

## Contextual Auditing

Control modules (`system.control.update_config`, `system.control.toggle_feature`, `system.control.reload_module`) **MUST** include `caller_id` and (if present) a redacted `identity` snapshot in their emitted audit events. When the caller is unauthenticated, `caller_id` **MUST** default to `"@external"`.

This requirement complements the structured audit-store contract in §1.2. While §1.2 governs the persisted `AuditEntry` shape, this section governs the **event payload** that is published on the event bus (e.g., `apcore.config.updated`, `apcore.module.toggled`, `apcore.module.reloaded`) so that real-time subscribers see the same identity context the audit store retains.

### Normative rules

- The event payload (`event.data`) **MUST** carry a `caller_id` string field.
- When `context.caller_id` is `None`/`null`/`""`, the payload `caller_id` **MUST** be the literal string `"@external"`.
- When `context.identity` is set, the payload **MUST** include an `identity` object containing `id`, `type`, and (optionally) `display_name`. Any field marked `x-sensitive: true` in the Identity schema **MUST** be redacted before inclusion (replaced with `"<redacted>"`).
- When `context.identity` is `None`/`null`, the payload **MUST NOT** contain an `identity` field (omit, do not emit `null`).
- Implementations **MUST NOT** include raw bearer tokens, API keys, or other credential material in the audit event payload — only the redacted Identity snapshot is permitted.

### Audit event payload shape

=== "Python"

    ```python
    # Emitted by system.control.update_config when context.caller_id and
    # context.identity are populated.
    event = ApCoreEvent(
        event_type="apcore.config.updated",
        module_id="system.control.update_config",
        timestamp="2026-05-03T12:00:00Z",
        severity="info",
        data={
            "key": "executor.default_timeout",
            "old_value": 30000,
            "new_value": 60000,
            "reason": "increase timeout",
            "caller_id": "ops.console",
            "identity": {
                "id": "user-42",
                "type": "user",
                "display_name": "alice@example.com",
            },
        },
    )

    # Unauthenticated caller — caller_id defaults to @external, identity omitted.
    event = ApCoreEvent(
        event_type="apcore.module.toggled",
        module_id="system.control.toggle_feature",
        timestamp="2026-05-03T12:00:00Z",
        severity="info",
        data={
            "module_id": "risky.module",
            "enabled": False,
            "reason": "incident-1234",
            "caller_id": "@external",
        },
    )
    ```

=== "TypeScript"

    ```typescript
    // Emitted by system.control.update_config when context.caller_id and
    // context.identity are populated.
    const event = new ApCoreEvent({
        eventType: "apcore.config.updated",
        moduleId: "system.control.update_config",
        timestamp: "2026-05-03T12:00:00Z",
        severity: "info",
        data: {
            key: "executor.default_timeout",
            old_value: 30000,
            new_value: 60000,
            reason: "increase timeout",
            caller_id: "ops.console",
            identity: {
                id: "user-42",
                type: "user",
                display_name: "alice@example.com",
            },
        },
    });

    // Unauthenticated caller — caller_id defaults to @external, identity omitted.
    const externalEvent = new ApCoreEvent({
        eventType: "apcore.module.toggled",
        moduleId: "system.control.toggle_feature",
        timestamp: "2026-05-03T12:00:00Z",
        severity: "info",
        data: {
            module_id: "risky.module",
            enabled: false,
            reason: "incident-1234",
            caller_id: "@external",
        },
    });
    ```

=== "Rust"

    ```rust
    use apcore::events::ApCoreEvent;
    use serde_json::json;

    // Emitted by system.control.update_config when context.caller_id and
    // context.identity are populated.
    let event = ApCoreEvent {
        event_type: "apcore.config.updated".to_string(),
        module_id: "system.control.update_config".to_string(),
        timestamp: "2026-05-03T12:00:00Z".to_string(),
        severity: "info".to_string(),
        data: json!({
            "key": "executor.default_timeout",
            "old_value": 30000,
            "new_value": 60000,
            "reason": "increase timeout",
            "caller_id": "ops.console",
            "identity": {
                "id": "user-42",
                "type": "user",
                "display_name": "alice@example.com"
            }
        }),
    };

    // Unauthenticated caller — caller_id defaults to @external, identity omitted.
    let external_event = ApCoreEvent {
        event_type: "apcore.module.toggled".to_string(),
        module_id: "system.control.toggle_feature".to_string(),
        timestamp: "2026-05-03T12:00:00Z".to_string(),
        severity: "info".to_string(),
        data: json!({
            "module_id": "risky.module",
            "enabled": false,
            "reason": "incident-1234",
            "caller_id": "@external"
        }),
    };
    ```

### Relationship to §1.2 AuditStore

When both an `AuditStore` (§1.2) and the event bus are configured, implementations **MUST** populate both surfaces from the same in-memory snapshot of `caller_id` + `identity` so the persisted `AuditEntry.actor_id` and the event payload `caller_id` agree. Implementations **MUST NOT** drop the audit event when no `AuditStore` is configured — the event bus is the minimum surface for contextual auditing.

---

### 1.3 Prometheus Exporter for UsageCollector

#### Normative Rules

- When `observability.prometheus.enabled: true`, the UsageCollector MUST expose its data via the `/metrics` endpoint established in observability hardening (§ Observability Hardening 1.6).
- The UsageCollector MUST emit these additional Prometheus metrics:
    - `apcore_usage_calls_total{module_id, status}` — counter
    - `apcore_usage_error_rate{module_id}` — gauge (0.0–1.0)
    - `apcore_usage_p50_latency_ms{module_id}`, `apcore_usage_p95_latency_ms{module_id}`, `apcore_usage_p99_latency_ms{module_id}` — gauges
- The Prometheus exporter MUST call `collector.get_module_stats()` and transform to the text format; MUST NOT block the HTTP handler for more than `export_timeout_ms` (default 1000ms).

#### YAML Configuration

```yaml
observability:
  prometheus:
    enabled: true
    export_timeout_ms: 1000   # default; controls UsageCollector export budget
```

#### Usage Examples

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    # observability.prometheus.enabled: true in apcore.yaml
    client = APCore(config=config)

    # UsageCollector metrics are now included in GET /metrics:
    #   apcore_usage_calls_total{module_id="math.add",status="success"} 5000
    #   apcore_usage_error_rate{module_id="math.add"} 0.0004
    #   apcore_usage_p99_latency_ms{module_id="math.add"} 45.0
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const config = Config.load('apcore.yaml');
    // observability.prometheus.enabled: true in apcore.yaml
    const client = new APCore({ config });

    // UsageCollector metrics are now included in GET /metrics:
    //   apcore_usage_calls_total{module_id="math.add",status="success"} 5000
    //   apcore_usage_error_rate{module_id="math.add"} 0.0004
    //   apcore_usage_p99_latency_ms{module_id="math.add"} 45.0
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    // observability.prometheus.enabled: true in apcore.yaml
    let client = APCore::from_path("apcore.yaml")?;

    // UsageCollector metrics are now included in GET /metrics:
    //   apcore_usage_calls_total{module_id="math.add",status="success"} 5000
    //   apcore_usage_error_rate{module_id="math.add"} 0.0004
    //   apcore_usage_p99_latency_ms{module_id="math.add"} 45.0
    ```

---

### 1.4 Granular Reload via Path Filtering

Currently `system.control.reload_module` reloads a single module by ID.

#### Normative Rules

- Implementations MUST support a `path_filter` input field on `system.control.reload_module` that accepts a glob pattern. When specified, the module MUST reload all modules whose IDs match the pattern.
- `path_filter` and `module_id` MUST be mutually exclusive. If both are provided, implementations MUST raise a `MODULE_RELOAD_CONFLICT` error.
- Reload order for multiple matches MUST follow the dependency topological order (leaf modules first, then modules that depend on them).

#### Updated Input for `system.control.reload_module`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_id` | string | *(one of required)* | Single module to reload (mutually exclusive with `path_filter`) |
| `path_filter` | string | *(one of required)* | Glob pattern for bulk reload (mutually exclusive with `module_id`) |
| `reload_dependents` | bool | `false` | When `true`, also reload modules that depend on matched modules |
| `reason` | string | *(required)* | Audit reason |

#### Usage Examples

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # Reload all executor modules
    result = await client.executor.call(
        "system.control.reload_module",
        {"path_filter": "executor.*", "reload_dependents": False, "reason": "deploy"},
        context,
    )
    # result["reloaded_modules"] == ["executor.email.send", "executor.math.add", ...]

    # Reload a single module by ID (existing behavior unchanged)
    result = await client.executor.call(
        "system.control.reload_module",
        {"module_id": "executor.email.send", "reason": "hotfix"},
        context,
    )
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const config = Config.load('apcore.yaml');
    const client = new APCore({ config });

    // Reload all executor modules
    const result = await client.executor.call(
        'system.control.reload_module',
        { path_filter: 'executor.*', reload_dependents: false, reason: 'deploy' },
        context,
    );
    // result.reloaded_modules === ['executor.email.send', 'executor.math.add', ...]

    // Passing both fields raises MODULE_RELOAD_CONFLICT
    // await client.executor.call('system.control.reload_module',
    //   { module_id: 'x', path_filter: 'y.*', reason: 'test' }, context);
    // → throws ModuleReloadConflictError
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use serde_json::json;

    let client = APCore::from_path("apcore.yaml")?;

    // Reload all executor modules
    let result = client.executor.call(
        "system.control.reload_module",
        json!({ "path_filter": "executor.*", "reload_dependents": false, "reason": "deploy" }),
        None,
        None,
    ).await?;
    // result["reloaded_modules"] contains the list of reloaded module IDs
    ```

---

### 1.5 Startup Failure Handling

#### Normative Rules

- **Python**: `register_sys_modules()` MUST accept a `fail_on_error: bool = False` parameter. When `True`, any system module registration failure MUST raise immediately. When `False` (default), failures MUST be logged at ERROR level but execution continues.
- **TypeScript**: `registerSysModules()` MUST accept `failOnError: boolean = false` with the same behavior.
- **Rust**: `register_sys_modules()` MUST return `Result<(), SysModuleError>` instead of returning `Option` or panicking. The caller MUST handle the Result.

#### Usage Examples

=== "Python"

    ```python
    from apcore.sys_modules.registration import register_sys_modules

    # Default: log errors and continue
    context = register_sys_modules(
        registry=registry,
        executor=executor,
        config=config,
        fail_on_error=False,   # default — errors logged at ERROR, execution continues
    )

    # Strict: raise immediately on any failure
    try:
        context = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            fail_on_error=True,
        )
    except SysModuleRegistrationError as exc:
        print(f"System module registration failed: {exc}")
        raise SystemExit(1)
    ```

=== "TypeScript"

    ```typescript
    import { registerSysModules, SysModuleRegistrationError } from 'apcore-js/sys-modules';

    // Default: log errors and continue
    const context = await registerSysModules({
        registry,
        executor,
        config,
        failOnError: false,   // default — errors logged at ERROR, execution continues
    });

    // Strict: raise immediately on any failure
    try {
        const context = await registerSysModules({
            registry,
            executor,
            config,
            failOnError: true,
        });
    } catch (err) {
        if (err instanceof SysModuleRegistrationError) {
            console.error(`System module registration failed: ${err.message}`);
            process.exit(1);
        }
        throw err;
    }
    ```

=== "Rust"

    ```rust
    use apcore::sys_modules::registration::register_sys_modules;
    use apcore::sys_modules::errors::SysModuleError;

    // register_sys_modules always returns Result — caller MUST handle it
    let context = register_sys_modules(&registry, &executor, &config)?;

    // Explicit match for fine-grained handling
    match register_sys_modules(&registry, &executor, &config) {
        Ok(ctx) => {
            // All system modules registered successfully
            serve(ctx).await;
        }
        Err(SysModuleError::RegistrationFailed { module_id, source }) => {
            eprintln!("Failed to register system module {module_id}: {source}");
            std::process::exit(1);
        }
    }
    ```

---

## Contract: register_sys_modules

### Inputs

| Parameter | Type | Required | Description |
|---|---|---|---|
| `executor` | `Executor` | Yes | The executor to register modules on. |
| `registry` | `Registry` | Yes | Registry to register system modules into. |
| `config` | `Config` | Yes | Config instance; reads `sys_modules.*` keys. |
| `metrics_collector` | `MetricsCollector \| None` | No | If `None`, a new one is created and attached. |
| `fail_on_error` | `bool` | No (default `False`) | Whether to raise on registration failure. [Python/TypeScript only] |

### Errors

| Code | Condition |
|---|---|
| `SYS_MODULE_REGISTRATION_FAILED` | A system module failed to register (only raised when `fail_on_error=True` in Python/TypeScript; always returned as `Err` in Rust). |

### Returns

- **On success:** `SysModulesContext` — all system modules registered (void/None/() in Rust: `Result<(), SysModuleError>`).
- **Rust:** `Result<(), SysModuleError>`

### Properties

- **async:** false
- **thread_safe:** false — call once at startup before serving requests
- **pure:** false — registers modules into executor
- **idempotent:** false — registering twice causes `MODULE_ALREADY_REGISTERED`
