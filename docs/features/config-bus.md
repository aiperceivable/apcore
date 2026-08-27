---
description: "Config Bus turns Config into a namespace registry: per-package namespaces with JSON Schema validation, env-var routing, defaults, multi-source merge, hot-reload; enabled by apcore key."
---

# Config Bus

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §9.4 Config Bus Architecture.


## Overview

The Config Bus (§9.4–§9.13) turns `Config` into an ecosystem-level namespace registry. Any package can register a named configuration namespace with optional JSON Schema validation, environment variable routing, default values, and hot-reload support. Multiple sources can be merged into a namespace at runtime (YAML files, in-memory dicts).

Namespace mode is activated automatically when the loaded YAML contains a top-level `apcore:` key. Legacy mode (flat YAML) is fully backward compatible.

## Concepts

### Configuration Modes

| Mode | Activation | Description |
|------|-----------|-------------|
| Legacy | No `apcore:` key in YAML | Flat YAML; all keys resolved by dot-path |
| Namespace | `apcore:` key present | Each top-level key is an independent namespace |

### Namespace Registration

Namespaces are registered globally (class-level) before loading a config file. The `env_prefix` is auto-derived from the namespace name when not specified:

=== "Python"

    ```python
    # Python — minimal (env_prefix auto-derived as "MY_PLUGIN")
    Config.register_namespace("my-plugin")

    # Python — full options
    Config.register_namespace(
        "my-plugin",
        env_prefix="MY_PLUGIN",         # optional, auto-derived if omitted
        defaults={"timeout": 5000, "retries": 3},
        env_style="auto",               # "auto" (default), "nested", or "flat"
        max_depth=5,                     # max nesting depth (default 5)
        env_map={"REDIS_URL": "cache"},  # bare env var → namespace key
        schema=None,                     # optional JSON Schema
    )
    ```

=== "TypeScript"

    ```typescript
    // TypeScript — minimal
    Config.registerNamespace({ name: 'my-plugin' });

    // TypeScript — full options
    Config.registerNamespace({
      name: 'my-plugin',
      envPrefix: 'MY_PLUGIN',
      defaults: { timeout: 5000, retries: 3 },
      envStyle: 'auto',
      maxDepth: 5,
      envMap: { REDIS_URL: 'cache' },
    });
    ```

=== "Rust"

    ```rust
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};
    use std::collections::HashMap;

    Config::register_namespace(NamespaceRegistration {
        name: "my-plugin".into(),
        env_prefix: None,  // auto-derived as "MY_PLUGIN"
        defaults: Some(serde_json::json!({ "timeout": 5000, "retries": 3 })),
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: Some(HashMap::from([("REDIS_URL".into(), "cache".into())])),
        schema: None,
    })?;
    ```

### env_prefix Auto-Derivation

When `env_prefix` is not provided (or `None`), it is auto-derived from the namespace name:

```
name.upper().replace("-", "_")

"myapp"      → "MYAPP"
"apcore-mcp" → "APCORE_MCP"
"my-plugin"  → "MY_PLUGIN"
```

Use explicit `env_prefix` only when the derived name doesn't fit (e.g., `name="reach"` but you want `env_prefix="REACHFORGE"`).

### Error Conditions

| Error Code | Cause |
|------------|-------|
| `CONFIG_NAMESPACE_RESERVED` | Name is in the reserved set (`apcore`, `_config`) |
| `CONFIG_NAMESPACE_DUPLICATE` | Name already registered |
| `CONFIG_ENV_PREFIX_CONFLICT` | `envPrefix` already in use |
| `CONFIG_ENV_MAP_CONFLICT` | A bare env var name in `env_map` is already claimed by another mapping |

## Environment Variable Sources

There are three ways environment variables can feed into config:

### 1. Global env_map — bare env vars to top-level config keys

=== "Python"

    ```python
    from apcore import Config

    Config.env_map({"PORT": "port", "DATABASE_URL": "db_url"})

    # PORT=3000 → config.get("port") = 3000
    # DATABASE_URL=pg:// → config.get("db_url") = "pg://"
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.envMap({ PORT: 'port', DATABASE_URL: 'db_url' });

    // PORT=3000 → config.get("port") = 3000
    // DATABASE_URL=pg:// → config.get("db_url") = "pg://"
    ```

=== "Rust"

    ```rust
    use apcore::Config;
    use std::collections::HashMap;

    Config::env_map(HashMap::from([
        ("PORT".to_string(), "port".to_string()),
        ("DATABASE_URL".to_string(), "db_url".to_string()),
    ]))?;

    // PORT=3000 → config.get("port") = 3000
    // DATABASE_URL=pg:// → config.get("db_url") = "pg://"
    ```

Global env_map maps to the **config root level**, not inside any namespace. This is for well-known env vars (`PORT`, `DATABASE_URL`, `HOST`) that don't conceptually belong to a namespace.

### 2. Namespace env_map — bare env vars to namespace keys

=== "Python"

    ```python
    from apcore import Config

    Config.register_namespace("myapp", env_map={"REDIS_URL": "cache_url"})

    # REDIS_URL=redis://... → config.get("myapp.cache_url") = "redis://..."
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.registerNamespace({
      name: 'myapp',
      envMap: { REDIS_URL: 'cache_url' },
    });

    // REDIS_URL=redis://... → config.get("myapp.cache_url") = "redis://..."
    ```

=== "Rust"

    ```rust
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};
    use std::collections::HashMap;

    Config::register_namespace(NamespaceRegistration {
        name: "myapp".into(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: Some(HashMap::from([("REDIS_URL".into(), "cache_url".into())])),
    })?;

    // REDIS_URL=redis://... → config.get("myapp.cache_url") = "redis://..."
    ```

Namespace env_map maps into the specified namespace. Use for env vars with well-known names that belong to a specific namespace.

### 3. Prefix-based routing — env vars with namespace prefix

=== "Python"

    ```python
    from apcore import Config

    Config.register_namespace("myapp")  # env_prefix auto-derived as "MYAPP"

    # MYAPP_DEBUG=true → config.get("myapp.debug") = True
    # MYAPP_API_TIMEOUT=60 → depends on env_style (see below)
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.registerNamespace({ name: 'myapp' });  // envPrefix auto-derived as "MYAPP"

    // MYAPP_DEBUG=true → config.get("myapp.debug") = true
    // MYAPP_API_TIMEOUT=60 → depends on envStyle (see below)
    ```

=== "Rust"

    ```rust
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};

    Config::register_namespace(NamespaceRegistration {
        name: "myapp".into(),
        env_prefix: None,  // auto-derived as "MYAPP"
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })?;

    // MYAPP_DEBUG=true → config.get("myapp.debug") = true
    // MYAPP_API_TIMEOUT=60 → depends on env_style (see below)
    ```

**env_map is exact-match. It does not go through env_style conversion.** Only prefix-based routing uses env_style.

### Processing order

For each environment variable, the system checks in this order:

1. **Global env_map** — exact match → top-level config key
2. **Namespace env_map** — exact match → namespace config key
3. **Prefix-based dispatch** — longest-prefix-match → env_style conversion

First match wins. An env var is processed by at most one source.

### Conflict detection

The same bare env var name cannot appear in more than one env_map (global or namespace). Attempting to register a duplicate raises `CONFIG_ENV_MAP_CONFLICT`:

=== "Python"

    ```python
    from apcore import Config

    Config.env_map({"PORT": "port"})
    Config.register_namespace("myapp", env_map={"PORT": "server_port"})
    # → ConfigEnvMapConflictError: "PORT" already mapped by "__global__"
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.envMap({ PORT: 'port' });
    Config.registerNamespace({
      name: 'myapp',
      envMap: { PORT: 'server_port' },
    });
    // → ConfigEnvMapConflictError: "PORT" already mapped by "__global__"
    ```

=== "Rust"

    ```rust
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};
    use std::collections::HashMap;

    Config::env_map(HashMap::from([("PORT".into(), "port".into())]))?;
    Config::register_namespace(NamespaceRegistration {
        name: "myapp".into(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: Some(HashMap::from([("PORT".into(), "server_port".into())])),
    })?;
    // → Err(ConfigEnvMapConflictError) — "PORT" already mapped by "__global__"
    ```

## Environment Variable Styles (env_style)

`env_style` controls how the suffix after `env_prefix` is converted to a config key. Only affects prefix-based routing (not env_map).

### auto (default, recommended)

Matches the env var suffix against the `defaults` tree structure. Flat keys match flat, nested paths match nested:

=== "Python"

    ```python
    from apcore import Config

    Config.register_namespace(
        "myapp",
        defaults={"devto_api_key": "", "publish": {"delay": 5, "retry": 3}},
    )

    # MYAPP_DEVTO_API_KEY=abc → myapp.devto_api_key  (flat key found in defaults)
    # MYAPP_PUBLISH_DELAY=10  → myapp.publish.delay   (nested path found in defaults)
    # MYAPP_UNKNOWN_KEY=x     → myapp.unknown.key     (not in defaults → fallback to nested)
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.registerNamespace({
      name: 'myapp',
      defaults: { devto_api_key: '', publish: { delay: 5, retry: 3 } },
    });

    // MYAPP_DEVTO_API_KEY=abc → myapp.devto_api_key  (flat key found in defaults)
    // MYAPP_PUBLISH_DELAY=10  → myapp.publish.delay   (nested path found in defaults)
    // MYAPP_UNKNOWN_KEY=x     → myapp.unknown.key     (not in defaults → fallback to nested)
    ```

=== "Rust"

    ```rust
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};

    Config::register_namespace(NamespaceRegistration {
        name: "myapp".into(),
        env_prefix: None,
        defaults: Some(serde_json::json!({
            "devto_api_key": "",
            "publish": { "delay": 5, "retry": 3 },
        })),
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })?;

    // MYAPP_DEVTO_API_KEY=abc → myapp.devto_api_key  (flat key found in defaults)
    // MYAPP_PUBLISH_DELAY=10  → myapp.publish.delay   (nested path found in defaults)
    // MYAPP_UNKNOWN_KEY=x     → myapp.unknown.key     (not in defaults → fallback to nested)
    ```

When `defaults` is not provided, auto falls back entirely to nested behavior.

### nested

Single `_` → `.` (section separator), double `__` → literal `_`:

```
MYAPP_API_TIMEOUT=60         → myapp.api.timeout
MYAPP_API_SERVER__URL=http:  → myapp.api.server_url
```

### flat

No conversion. Suffix lowercased as-is:

```
MYAPP_DEVTO_API_KEY=abc → myapp.devto_api_key
MYAPP_LLM_MODEL=gemini  → myapp.llm_model
```

### When to use each

| Scenario | Style |
|----------|-------|
| Mixed flat keys + nested sections | `auto` (default, don't specify) |
| Pure hierarchical config (e.g., `api.server.url`) | `nested` |
| Pure flat snake_case config (e.g., `devto_api_key`) | `flat` |

## Max Depth (max_depth)

Limits the nesting depth for `nested` and `auto` styles. Default: **5**. After `max_depth` segments, remaining `_` characters are preserved as literal underscores.

```
max_depth=5 (default):
  A_B_C_D_E_F_G → a.b.c.d.e_f_g  (5 segments, F_G kept literal)

max_depth=3:
  A_B_C_D_E → a.b.c_d_e  (3 segments)
```

Ignored for `flat` style.

## Configuration Priority

From highest to lowest:

```
env_map / env_prefix overrides  >  YAML file  >  namespace defaults
```

## Namespace Access

=== "Python"

    ```python
    from dataclasses import dataclass
    from apcore import Config

    config = Config.load("apcore.yaml")

    # Full namespace dict
    plugin_cfg = config.namespace("my-plugin")  # dict[str, Any]

    # Dot-path access
    timeout = config.get("my-plugin.timeout", 5000)

    # Top-level key (from global env_map)
    port = config.get("port")

    # Typed deserialization
    @dataclass
    class PluginConfig:
        timeout: int = 5000
        retries: int = 3

    typed = config.bind("my-plugin", PluginConfig)
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    const config = Config.load('apcore.yaml');

    const pluginCfg = config.namespace('my-plugin');
    const timeout = config.get('my-plugin.timeout', 5000);
    const port = config.get('port');  // from global env_map

    class PluginConfig {
      timeout: number;
      retries: number;
      constructor(data: Record<string, unknown>) {
        this.timeout = (data['timeout'] as number) ?? 5000;
        this.retries = (data['retries'] as number) ?? 3;
      }
    }
    const typed = config.bind('my-plugin', PluginConfig);
    ```

=== "Rust"

    ```rust
    use apcore::Config;
    use std::path::Path;

    let config = Config::load(Path::new("apcore.yaml"))?;

    let plugin_cfg = config.namespace("my-plugin");
    let timeout: u64 = config.get_typed("my-plugin.timeout")?;

    #[derive(serde::Deserialize)]
    struct PluginConfig { timeout: u64, retries: u32 }
    let typed: PluginConfig = config.bind("my-plugin")?;
    ```

## Mounting External Sources (§9.7)

Attach data from a file or in-memory dict to a namespace. Mounted data is merged over namespace defaults.

=== "Python"

    ```python
    config.mount("my-plugin", source={"timeout": 10000})        # dict source
    config.mount("my-plugin", source="./my-plugin.yaml")         # file source
    ```

=== "TypeScript"

    ```typescript
    config.mount('my-plugin', { fromDict: { timeout: 10000 } });
    config.mount('my-plugin', { fromFile: './my-plugin.yaml' });
    ```

=== "Rust"

    ```rust
    config.mount("my-plugin", MountSource::Dict(data))?;
    config.mount("my-plugin", MountSource::File("./my-plugin.yaml".into()))?;
    ```

**Error:** `CONFIG_MOUNT_ERROR` if namespace is `_config`, source file is missing, or file is not a valid YAML mapping.

## Typed Bind (§9.9.3)

Deserialize a namespace subtree into a typed value:

| Language | API |
|----------|-----|
| Python | `config.bind(namespace, dataclass_type)` → instance |
| TypeScript | `config.bind<T>(namespace, ClassName)` → `T` |
| Rust | `config.bind::<T>(namespace)` → `Result<T, ...>` |

**Error:** `CONFIG_BIND_ERROR` if instantiation fails.

## Hot Reload (§9.11)

Re-read the source YAML, re-detect mode, re-apply namespace defaults, env overrides, validation, and mounts:

=== "Python"

    ```python
    config.reload()
    ```

=== "TypeScript"

    ```typescript
    config.reload();
    ```

=== "Rust"

    ```rust
    config.reload()?;
    ```

## Built-in Namespaces (§9.15)

apcore pre-registers two namespaces at startup:

| Namespace | Env prefix | Description |
|-----------|-----------|-------------|
| `observability` | `APCORE_OBSERVABILITY` | Tracing, metrics, logging, error history, platform notify config |
| `sys_modules` | `APCORE_SYS` | System modules enable/disable and threshold config |

## Config Discovery (§9.14)

`Config.discover()` (or `Config.load()` with no path) searches these locations in order:

1. `$APCORE_CONFIG_FILE` environment variable (any path/filename)
2. `./project.yaml`
3. `./project.yml`
4. `./apcore.yaml`
5. `./apcore.yml`
6. `~/.config/apcore/config.yaml` (XDG)

Falls back to `Config.from_defaults()` if nothing is found.

**Any YAML filename works** when loaded explicitly:

=== "Python"

    ```python
    import os
    from apcore import Config

    Config.load("my-custom-config.yaml")              # explicit path
    os.environ["APCORE_CONFIG_FILE"] = "custom.yaml"  # via env var
    Config.discover()                                  # auto-discovery
    ```

=== "TypeScript"

    ```typescript
    import process from 'node:process';
    import { Config } from 'apcore-js';

    Config.load('my-custom-config.yaml');               // explicit path
    process.env.APCORE_CONFIG_FILE = 'custom.yaml';     // via env var
    Config.discover();                                   // auto-discovery
    ```

=== "Rust"

    ```rust
    use std::path::Path;
    use apcore::Config;

    Config::load(Path::new("my-custom-config.yaml"))?;          // explicit path
    std::env::set_var("APCORE_CONFIG_FILE", "custom.yaml");     // via env var
    Config::discover()?;                                          // auto-discovery
    ```

## YAML File Format

### Legacy Mode

Standard flat YAML — backward compatible with all previous apcore versions:

```yaml
version: "0.15.0"
executor:
  default_timeout: 30000
extensions:
  root: ./extensions
```

### Namespace Mode

Activated by the presence of a top-level `apcore:` key:

```yaml
apcore:
  version: "0.15.0"

_config:
  strict: true   # Reject unknown namespace keys

observability:
  tracing:
    enabled: true
    sampling_rate: 1.0

my-plugin:
  timeout: 10000
  retries: 5
```

The `_config` reserved namespace controls validation behavior. `strict: true` causes `validate()` to reject unknown top-level keys.

## Contract: Config.validate

`validate()` enforces the same required-field set and value constraints in **all three SDKs**, in both legacy and namespace mode. Any violation is reported as `ConfigError(code=CONFIG_INVALID)`. (Prior to this contract each SDK enforced a different subset, so the same config could pass in one SDK and fail in another — implementations MUST converge on the set below.)

### Required fields

A key is required **only when it has no canonical default** (PROTOCOL_SPEC §9.1). Exactly two qualify — absence of either MUST be rejected with `CONFIG_INVALID`:

- `version`
- `project.name`

`extensions.root`, `schema.root`, `acl.root` and `acl.default_effect` were previously on this list. They all carry defaults in `schemas/defaults.schema.json`, so requiring them rejected configurations the framework resolves perfectly well; they were removed from `schemas/apcore-config.schema.json`'s `required` array for the same reason.

**Requiredness is evaluated against the declared document, before the default table is merged.** An implementation that deep-merges its defaults into the parsed document and *then* checks for required fields can never fail the check — the merge has already supplied every key. That is not validation, it is dead code that looks like validation, and it is how all three SDKs came to ship a required-field list that could not fire. Each SDK exposes the pre-merge view for this purpose (`Config.get_declared()` / `getDeclared()`).

A consequence worth stating: a defaults-only configuration (`Config.from_defaults()`) declares nothing, so `validate()` on it MUST fail. The no-config bootstrap path is unaffected — `Config.load()` with no discoverable file returns the defaults without validating them.

### Value constraints
Out-of-range values MUST be rejected with `CONFIG_INVALID`:

| Key | Constraint |
|-----|------------|
| `acl.default_effect` | one of `allow`, `deny` |
| `observability.tracing.sampling_rate` | number `0.0 ≤ x ≤ 1.0` |
| `extensions.max_depth` | integer in `[1, 16]` (discovery-recursion safety cap) |
| `executor.default_timeout`, `executor.global_timeout` | integer `≥ 0` (milliseconds) |
| `executor.max_call_depth`, `executor.max_module_repeat` | integer `≥ 1` |
| `sys_modules.error_history.max_entries_per_module` | integer `≥ 1` |
| `sys_modules.error_history.max_total_entries` | integer `≥ 1` |
| `sys_modules.events.thresholds.error_rate` | number `0.0 ≤ x ≤ 1.0` |
| `sys_modules.events.thresholds.latency_p99_ms` | number `> 0` |

> ⚠️ Booleans are rejected for all numeric fields.
>
> **`middleware.circuit_breaker.*` is not a configuration namespace.** Earlier revisions listed four such keys here, and all three SDKs validated them — but `apcore-config.schema.json` declares `MiddlewareConfig` as `{ disabled }` with `additionalProperties: false`, so a config that set them was *rejected by the canonical schema and accepted by every SDK*, then ignored at runtime: no SDK ever read them. They have been removed from all three constraint tables. A circuit breaker is configured through its constructor options (`open_threshold` 0.5, `window_size` 20, `recovery_window_ms` 30000, `min_samples` 5 — identical in all three SDKs) or through the declarative middleware-chain config, which is per-entry rather than global.

### Namespace mode (additional)
- For each registered namespace that declares a JSON Schema, the namespace subtree MUST validate against that schema; a failure is `CONFIG_INVALID`.
- Under `_config.strict: true`, an unregistered top-level namespace MUST be rejected with `CONFIG_INVALID`.

> **This applies to the value-constraint table above, not to the required-field set.** Every constraint listed is enforced by all three SDKs; an implementation missing one aligns **up**, and dropping a constraint weakens the configuration gate.
>
> The required-field set is governed by a different rule and was deliberately narrowed — see above. It is anchored to PROTOCOL_SPEC §9.1 ("required only when no canonical default exists"), **not** to any reference SDK. An earlier revision of this contract anchored it to `apcore-python` as "the superset"; that was a mistake, because Python's required-field check was unreachable dead code — it merged its defaults in before checking. Deferring to whichever SDK enforces the most is only sound when that SDK's enforcement actually runs.

## Introspection

=== "Python"

    ```python
    from apcore import Config

    namespaces = Config.registered_namespaces()
    # [{"name": "observability", "env_prefix": "APCORE_OBSERVABILITY", "has_schema": False}, ...]
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    const namespaces = Config.registeredNamespaces();
    // [{ name: 'observability', envPrefix: 'APCORE_OBSERVABILITY', hasSchema: false }, ...]
    ```

=== "Rust"

    ```rust
    use apcore::Config;

    let namespaces = Config::registered_namespaces();
    // Vec<NamespaceInfo> with name, env_prefix, has_schema fields
    ```

## Quick Reference

### Simplest usage

=== "Python"

    ```python
    from apcore import Config

    Config.register_namespace("myapp")
    cfg = Config.load("apcore.yaml")
    cfg.get("myapp.debug")  # reads from MYAPP_DEBUG env var or YAML
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.registerNamespace({ name: 'myapp' });
    const cfg = Config.load('apcore.yaml');
    cfg.get('myapp.debug');  // reads from MYAPP_DEBUG env var or YAML
    ```

=== "Rust"

    ```rust
    use std::path::Path;
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};

    Config::register_namespace(NamespaceRegistration {
        name: "myapp".into(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })?;
    let cfg = Config::load(Path::new("apcore.yaml"))?;
    let _debug = cfg.get("myapp.debug");  // reads from MYAPP_DEBUG env var or YAML
    ```

### With bare env vars

=== "Python"

    ```python
    from apcore import Config

    Config.env_map({"PORT": "port"})
    Config.register_namespace("myapp", env_map={"REDIS_URL": "cache_url"})

    cfg = Config.load("apcore.yaml")
    cfg.get("port")              # from PORT env var (top-level)
    cfg.get("myapp.cache_url")   # from REDIS_URL env var (namespace)
    cfg.get("myapp.debug")       # from MYAPP_DEBUG env var (prefix)
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.envMap({ PORT: 'port' });
    Config.registerNamespace({
      name: 'myapp',
      envMap: { REDIS_URL: 'cache_url' },
    });

    const cfg = Config.load('apcore.yaml');
    cfg.get('port');             // from PORT env var (top-level)
    cfg.get('myapp.cache_url');  // from REDIS_URL env var (namespace)
    cfg.get('myapp.debug');      // from MYAPP_DEBUG env var (prefix)
    ```

=== "Rust"

    ```rust
    use std::collections::HashMap;
    use std::path::Path;
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};

    Config::env_map(HashMap::from([("PORT".into(), "port".into())]))?;
    Config::register_namespace(NamespaceRegistration {
        name: "myapp".into(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: Some(HashMap::from([("REDIS_URL".into(), "cache_url".into())])),
    })?;

    let cfg = Config::load(Path::new("apcore.yaml"))?;
    cfg.get("port");              // from PORT env var (top-level)
    cfg.get("myapp.cache_url");   // from REDIS_URL env var (namespace)
    cfg.get("myapp.debug");       // from MYAPP_DEBUG env var (prefix)
    ```

### Full example with mixed config

=== "Python"

    ```python
    from apcore import Config

    Config.env_map({"PORT": "port", "DATABASE_URL": "db_url"})
    Config.register_namespace(
        "myapp",
        defaults={"devto_api_key": "", "publish": {"delay": 5}},
        env_map={"REDIS_URL": "cache_url"},
    )

    # Environment:
    #   PORT=3000
    #   DATABASE_URL=postgres://prod
    #   REDIS_URL=redis://localhost
    #   MYAPP_DEVTO_API_KEY=abc123
    #   MYAPP_PUBLISH_DELAY=10

    cfg = Config.load("apcore.yaml")
    cfg.get("port")                    # → 3000 (global env_map)
    cfg.get("db_url")                  # → "postgres://prod" (global env_map)
    cfg.get("myapp.cache_url")         # → "redis://localhost" (namespace env_map)
    cfg.get("myapp.devto_api_key")     # → "abc123" (auto: flat key in defaults)
    cfg.get("myapp.publish.delay")     # → 10 (auto: nested key in defaults)
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    Config.envMap({ PORT: 'port', DATABASE_URL: 'db_url' });
    Config.registerNamespace({
      name: 'myapp',
      defaults: { devto_api_key: '', publish: { delay: 5 } },
      envMap: { REDIS_URL: 'cache_url' },
    });

    // Environment:
    //   PORT=3000
    //   DATABASE_URL=postgres://prod
    //   REDIS_URL=redis://localhost
    //   MYAPP_DEVTO_API_KEY=abc123
    //   MYAPP_PUBLISH_DELAY=10

    const cfg = Config.load('apcore.yaml');
    cfg.get('port');                    // → 3000 (global env_map)
    cfg.get('db_url');                  // → "postgres://prod" (global env_map)
    cfg.get('myapp.cache_url');         // → "redis://localhost" (namespace env_map)
    cfg.get('myapp.devto_api_key');     // → "abc123" (auto: flat key in defaults)
    cfg.get('myapp.publish.delay');     // → 10 (auto: nested key in defaults)
    ```

=== "Rust"

    ```rust
    use std::collections::HashMap;
    use std::path::Path;
    use apcore::config::DEFAULT_MAX_DEPTH;
    use apcore::{Config, EnvStyle, NamespaceRegistration};

    Config::env_map(HashMap::from([
        ("PORT".into(), "port".into()),
        ("DATABASE_URL".into(), "db_url".into()),
    ]))?;
    Config::register_namespace(NamespaceRegistration {
        name: "myapp".into(),
        env_prefix: None,
        defaults: Some(serde_json::json!({
            "devto_api_key": "",
            "publish": { "delay": 5 },
        })),
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: Some(HashMap::from([("REDIS_URL".into(), "cache_url".into())])),
    })?;

    // Environment:
    //   PORT=3000
    //   DATABASE_URL=postgres://prod
    //   REDIS_URL=redis://localhost
    //   MYAPP_DEVTO_API_KEY=abc123
    //   MYAPP_PUBLISH_DELAY=10

    let cfg = Config::load(Path::new("apcore.yaml"))?;
    cfg.get("port");                    // → 3000 (global env_map)
    cfg.get("db_url");                  // → "postgres://prod" (global env_map)
    cfg.get("myapp.cache_url");         // → "redis://localhost" (namespace env_map)
    cfg.get("myapp.devto_api_key");     // → "abc123" (auto: flat key in defaults)
    cfg.get("myapp.publish.delay");     // → 10 (auto: nested key in defaults)
    ```

## Contract: Config.register_namespace

### Inputs
- `name` (str/string/&str, required) — namespace name; must not be `"apcore"` or `"_config"` (reserved); reject with `ConfigNamespaceReservedError(code=CONFIG_NAMESPACE_RESERVED)`
- `env_prefix` (str/string/&str, optional) — env var prefix; auto-derived from `name` when absent; reject duplicate prefix with `ConfigEnvPrefixConflictError(code=CONFIG_ENV_PREFIX_CONFLICT)`
- `defaults` (dict/object/Value, optional) — default values merged under this namespace before YAML/env overrides
- `env_style` (str/string/EnvStyle, optional) — `"auto"` (default), `"nested"`, or `"flat"`; controls env var suffix conversion
- `max_depth` (int/number/u32, optional) — maximum nesting depth for nested/auto routing; default 5
- `env_map` (dict/object/HashMap, optional) — bare env var → namespace key mapping; each key must be globally unique; reject conflicts with `ConfigEnvMapConflictError(code=CONFIG_ENV_MAP_CONFLICT)`
- `schema` (dict/object/Value, optional) — JSON Schema Draft 2020-12 for namespace values; validation runs at load time

### Errors
- `ConfigNamespaceReservedError(code=CONFIG_NAMESPACE_RESERVED)` — `name` is `"apcore"` or `"_config"`
- `ConfigNamespaceDuplicateError(code=CONFIG_NAMESPACE_DUPLICATE)` — namespace already registered under this name
- `ConfigEnvPrefixConflictError(code=CONFIG_ENV_PREFIX_CONFLICT)` — derived or explicit `env_prefix` already claimed by another namespace
- `ConfigEnvMapConflictError(code=CONFIG_ENV_MAP_CONFLICT)` — a bare env var in `env_map` is already mapped by another namespace or the global env_map

### Returns
- On success: void/None/() — namespace is registered class-globally for all future `Config.load()` calls

### Properties
- async: false
- thread_safe: false (call before any concurrent `Config.load()`)
- pure: false (mutates class-level namespace registry)
- idempotent: false (duplicate registration raises an error)

## Contract: Config.load

### Inputs
- `path` (str/string/&str or Path, optional) — file path to a YAML config file; if absent, falls back to `Config.discover()` search order

### Errors
- `ConfigNotFoundError(code=CONFIG_NOT_FOUND)` — path was provided but does not exist on disk
- `ConfigInvalidError(code=CONFIG_INVALID)` — file exists but is not valid YAML or its structure cannot be parsed

### Returns
- On success: `Config` instance with namespace data, env overrides, and defaults merged

### Properties
- async: false
- thread_safe: false (safe to call from multiple threads only if namespace registry is fully populated and frozen)
- pure: false (reads filesystem and environment variables)
- idempotent: true (loading the same file twice produces equivalent Config instances)

## Contract: Config.get

### Inputs
- `key` (str/string/&str, required) — dot-path key (e.g., `"my-plugin.timeout"` or `"port"`); empty string is rejected with `ValueError`/`ConfigInvalidError`
- `default` (Any/unknown/Value, optional) — value returned when key is absent; when absent and key is missing, returns `None`/`null`/`None`

### Errors
- No errors raised under normal operation (missing key returns `default` or `None`)

### Returns
- On success: the resolved value at `key`, or `default` if absent

### Properties
- async: false
- thread_safe: true
- pure: true (no side effects; reads merged config snapshot)
- idempotent: true

## Contract: Config.namespace

### Inputs
- `name` (str/string/&str, required) — registered namespace name; returns empty dict/object when namespace has no values (does not raise)

### Errors
- No errors under normal operation; unregistered namespace returns empty result

### Returns
- On success: `dict[str, Any]` / `Record<string, unknown>` / `HashMap<String, Value>` — all values under that namespace, merged from defaults + YAML + env overrides

### Properties
- async: false
- thread_safe: true
- pure: true

## Contract: Config.bind

### Inputs
- `namespace` (str/string/&str, required) — namespace name to deserialize
- `type` (type/class/generic, required) — dataclass type (Python), constructor class (TypeScript), or Rust generic `T: Deserialize`

### Errors
- `ConfigBindError(code=CONFIG_BIND_ERROR)` — deserialization/instantiation fails (type mismatch, missing required field, constructor raises)

### Returns
- On success: an instance of `type` populated with namespace values

### Properties
- async: false
- thread_safe: true
- pure: true (reads snapshot; does not mutate config)

## Contract: Config.mount

### Inputs
- `namespace` (str/string/&str, required) — target namespace; must NOT be `"_config"` (reserved)
- `source` (dict/object/MountSource, required) — either an in-memory dict or a path to a YAML file (relative paths resolved from CWD)

### Errors
- `ConfigMountError(code=CONFIG_MOUNT_ERROR)` — namespace is `_config`; or source is a file path that does not exist; or file is not a valid YAML mapping

### Returns
- On success: void/None/() — source data is merged into the namespace, over defaults, under env overrides

### Properties
- async: false
- thread_safe: false (do not call concurrently with reads)
- pure: false (mutates config state)
- idempotent: false (mounting the same source twice stacks; call once per source)

## Contract: Config.reload

### Inputs
- No inputs

### Errors
- `ConfigNotFoundError(code=CONFIG_NOT_FOUND)` — source file no longer exists
- `ConfigInvalidError(code=CONFIG_INVALID)` — source file is now invalid YAML

### Returns
- On success: void/None/() — config is refreshed from disk; env overrides and mounts are re-applied

### Properties
- async: false
- thread_safe: false (call outside of concurrent request handling; no in-flight read protection)
- pure: false (re-reads filesystem and environment variables)
- idempotent: true (calling reload twice with unchanged files produces identical state)

## Key Files

| File | Purpose |
|------|---------|
| `src/apcore/config.py` (Python) | `Config` class, `register_namespace()`, `env_map()`, `namespace()`, `bind()`, `mount()` |
| `src/config.ts` (TypeScript) | `Config` class with identical API shape |
| `src/config.rs` (Rust) | `Config` struct, `NamespaceRegistration`, `EnvStyle`, `MountSource`, `ConfigMode` |

## Testing Strategy

- **Namespace registration:** reserved name rejection, duplicate rejection, env prefix conflict, env_map conflict.
- **Env routing:** longest-prefix-match, env_style (auto/nested/flat), max_depth enforcement, env_map dispatch.
- **env_prefix auto-derivation:** name → uppercase, hyphen → underscore.
- **Global env_map:** bare env var → top-level key in both legacy and namespace mode.
- **Mount:** dict mount merge, file mount parse, `_config` rejection, missing file error.
- **Bind:** successful deserialization, bind error on invalid data.
- **Mode detection:** legacy YAML activates legacy mode, `apcore:` key activates namespace mode.
- **Config discovery:** `$APCORE_CONFIG_FILE` env precedence, CWD candidates, XDG path, fallback to defaults.
- **Hot reload:** changes in YAML picked up, mounts re-applied.
