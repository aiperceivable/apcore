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

Control modules require `requires_approval: true` and are only registered when `sys_modules.events.enabled: true`.

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
- Emits `config_changed` event.

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

**Process:** `safe_unregister()` with drain → `discover()` re-load → re-register → emit `config_changed` event.

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

Disabled modules remain registered but calls raise `ModuleDisabledError`. Toggle state is thread-safe (via `ToggleState` class) and survives module reload. Emits `module_health_changed` event.

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

=== "Rust"

    ```rust
    use apcore::{APCore, Config};
    use serde_json::json;

    let config = Config::load("apcore.yaml")?;
    let client = APCore::with_config(config);

    // System modules auto-registered! Query them directly:
    let health = client.call("system.health.summary", json!({})).await;
    let usage = client.call("system.usage.summary", json!({"period": "24h"})).await;

    // Control via convenience methods:
    client.disable("some.module", "maintenance").await;
    client.enable("some.module", "done").await;
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
      error_rate: 0.1             # 10% triggers error_threshold_exceeded
      latency_p99_ms: 5000.0      # 5s triggers latency_threshold_exceeded
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

## Key Files

| File | Purpose |
|------|---------|
| `src/apcore/sys_modules/registration.py` | `register_sys_modules()`, subscriber factory registry |
| `src/apcore/sys_modules/health.py` | `HealthSummaryModule`, `HealthModuleModule` |
| `src/apcore/sys_modules/manifest.py` | `ManifestModuleModule`, `ManifestFullModule` |
| `src/apcore/sys_modules/usage.py` | `UsageSummaryModule`, `UsageModuleModule` |
| `src/apcore/sys_modules/control.py` | `UpdateConfigModule`, `ReloadModuleModule`, `ToggleFeatureModule`, `ToggleState` |

## Testing Strategy

- **Health modules**: Verify status classification thresholds, error aggregation from ErrorHistory, latency metrics from MetricsCollector.
- **Manifest modules**: Verify schema/annotation extraction, prefix/tag filtering, source path computation.
- **Usage modules**: Verify call counting, trend computation, hourly distribution padding, per-caller breakdown.
- **Control modules**: Verify approval requirement, config update with constraint validation, module reload lifecycle, toggle state persistence across reload.
- **Registration**: Verify auto-registration workflow, config-driven subscriber creation, middleware ordering.
