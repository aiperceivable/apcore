# Config Bus

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

```rust
// Rust
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

```python
Config.env_map({"PORT": "port", "DATABASE_URL": "db_url"})

# PORT=3000 → config.get("port") = 3000
# DATABASE_URL=pg:// → config.get("db_url") = "pg://"
```

Global env_map maps to the **config root level**, not inside any namespace. This is for well-known env vars (`PORT`, `DATABASE_URL`, `HOST`) that don't conceptually belong to a namespace.

### 2. Namespace env_map — bare env vars to namespace keys

```python
Config.register_namespace("myapp", env_map={"REDIS_URL": "cache_url"})

# REDIS_URL=redis://... → config.get("myapp.cache_url") = "redis://..."
```

Namespace env_map maps into the specified namespace. Use for env vars with well-known names that belong to a specific namespace.

### 3. Prefix-based routing — env vars with namespace prefix

```python
Config.register_namespace("myapp")  # env_prefix auto-derived as "MYAPP"

# MYAPP_DEBUG=true → config.get("myapp.debug") = True
# MYAPP_API_TIMEOUT=60 → depends on env_style (see below)
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

```python
Config.env_map({"PORT": "port"})
Config.register_namespace("myapp", env_map={"PORT": "server_port"})
# → ConfigEnvMapConflictError: "PORT" already mapped by "__global__"
```

## Environment Variable Styles (env_style)

`env_style` controls how the suffix after `env_prefix` is converted to a config key. Only affects prefix-based routing (not env_map).

### auto (default, recommended)

Matches the env var suffix against the `defaults` tree structure. Flat keys match flat, nested paths match nested:

```python
Config.register_namespace(
    "myapp",
    defaults={"devto_api_key": "", "publish": {"delay": 5, "retry": 3}},
)

# MYAPP_DEVTO_API_KEY=abc → myapp.devto_api_key  (flat key found in defaults)
# MYAPP_PUBLISH_DELAY=10  → myapp.publish.delay   (nested path found in defaults)
# MYAPP_UNKNOWN_KEY=x     → myapp.unknown.key     (not in defaults → fallback to nested)
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

```python
# Python
config = Config.load("apcore.yaml")

# Full namespace dict
plugin_cfg = config.namespace("my-plugin")  # dict[str, Any]

# Dot-path access
timeout = config.get("my-plugin.timeout", 5000)

# Top-level key (from global env_map)
port = config.get("port")

# Typed deserialization
from dataclasses import dataclass

@dataclass
class PluginConfig:
    timeout: int = 5000
    retries: int = 3

typed = config.bind("my-plugin", PluginConfig)
```

```typescript
// TypeScript
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

```rust
// Rust
let config = Config::load("apcore.yaml")?;

let plugin_cfg = config.namespace("my-plugin");
let timeout: u64 = config.get_typed("my-plugin.timeout")?;

#[derive(serde::Deserialize)]
struct PluginConfig { timeout: u64, retries: u32 }
let typed: PluginConfig = config.bind("my-plugin")?;
```

## Mounting External Sources (§9.7)

Attach data from a file or in-memory dict to a namespace. Mounted data is merged over namespace defaults.

```python
config.mount("my-plugin", source={"timeout": 10000})        # dict source
config.mount("my-plugin", source="./my-plugin.yaml")         # file source
```

```typescript
config.mount('my-plugin', { fromDict: { timeout: 10000 } });
config.mount('my-plugin', { fromFile: './my-plugin.yaml' });
```

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

```python
config.reload()  # Python
```
```typescript
config.reload(); // TypeScript
```
```rust
config.reload()?; // Rust
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

```python
Config.load("my-custom-config.yaml")           # explicit path
os.environ["APCORE_CONFIG_FILE"] = "custom.yaml"  # via env var
Config.discover()                                # auto-discovery
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
    samplingRate: 1.0

my-plugin:
  timeout: 10000
  retries: 5
```

The `_config` reserved namespace controls validation behavior. `strict: true` causes `validate()` to reject unknown top-level keys.

## Introspection

```python
# Python
namespaces = Config.registered_namespaces()
# [{"name": "observability", "env_prefix": "APCORE_OBSERVABILITY", "has_schema": False}, ...]
```

```typescript
// TypeScript
const namespaces = Config.registeredNamespaces();
// [{ name: 'observability', envPrefix: 'APCORE_OBSERVABILITY', hasSchema: false }, ...]
```

## Quick Reference

### Simplest usage

```python
Config.register_namespace("myapp")
cfg = Config.load("apcore.yaml")
cfg.get("myapp.debug")  # reads from MYAPP_DEBUG env var or YAML
```

### With bare env vars

```python
Config.env_map({"PORT": "port"})
Config.register_namespace("myapp", env_map={"REDIS_URL": "cache_url"})

cfg = Config.load("apcore.yaml")
cfg.get("port")              # from PORT env var (top-level)
cfg.get("myapp.cache_url")   # from REDIS_URL env var (namespace)
cfg.get("myapp.debug")       # from MYAPP_DEBUG env var (prefix)
```

### Full example with mixed config

```python
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
