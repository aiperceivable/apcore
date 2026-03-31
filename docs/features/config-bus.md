# Config Bus

## Overview

The Config Bus (§9.4–§9.13) turns `Config` into an ecosystem-level namespace registry. Any package can register a named configuration namespace with optional JSON Schema validation, environment variable prefix routing, default values, and hot-reload support. Multiple sources can be merged into a namespace at runtime (YAML files, in-memory dicts).

Namespace mode is activated automatically when the loaded YAML contains a top-level `apcore:` key. Legacy mode (flat YAML) is fully backward compatible.

## Concepts

### Configuration Modes

| Mode | Activation | Description |
|------|-----------|-------------|
| Legacy | No `apcore:` key in YAML | Flat YAML; all keys resolved by dot-path |
| Namespace | `apcore:` key present | Each top-level key is an independent namespace |

### Namespace Registration

Namespaces are registered globally (class-level) before loading a config file:

```python
# Python
from apcore.config import Config

Config.register_namespace(
    name="my_plugin",
    env_prefix="MY_PLUGIN",
    defaults={"timeout": 5000, "retries": 3},
    schema=None,  # optional JSON Schema dict
)
```

```typescript
// TypeScript
import { Config } from 'apcore-js';

Config.registerNamespace('myPlugin', {
  envPrefix: 'MY_PLUGIN',
  defaults: { timeout: 5000, retries: 3 },
  schema: { type: 'object', properties: { timeout: { type: 'number' } } },
});
```

```rust
// Rust
use apcore::Config;

Config::register_namespace(apcore::NamespaceRegistration {
    name: "my_plugin".to_string(),
    env_prefix: Some("MY_PLUGIN".to_string()),
    defaults: Some(serde_json::json!({ "timeout": 5000, "retries": 3 })),
    schema: None,
})?;
```

### Error Conditions

| Error Code | Cause |
|------------|-------|
| `CONFIG_NAMESPACE_RESERVED` | Name is in the reserved set (`apcore`, `_config`) |
| `CONFIG_NAMESPACE_DUPLICATE` | Name already registered |
| `CONFIG_ENV_PREFIX_CONFLICT` | `envPrefix` already in use, or matches `APCORE_[A-Z0-9]` pattern |

## Environment Variable Routing (§9.10)

Env vars are dispatched to namespaces using **longest-prefix-match**. Sort all registered `envPrefix` values by length descending; the first match wins.

```
APCORE_OBSERVABILITY_TRACING_ENABLED=true
   → namespace "observability", key "tracing.enabled"

MY_PLUGIN_TIMEOUT=10000
   → namespace "my_plugin", key "timeout"
```

Separator rules:
- Double `__` in the suffix → literal `_` in the key
- Single `_` in the suffix → `.` separator in the key

**Reserved prefix:** Any env var matching `APCORE_[A-Z0-9]` is reserved for apcore's legacy flat-key override scheme and cannot be used as a namespace `envPrefix`. Double-underscore `APCORE_` prefixes are allowed for apcore sub-package namespaces.

## Namespace Access

```python
# Python
config = Config.load("apcore.yaml")

# Full namespace dict
plugin_cfg = config.namespace("my_plugin")  # dict[str, Any]

# Dot-path access
timeout = config.get("my_plugin.timeout", 5000)

# Typed deserialization
from dataclasses import dataclass

@dataclass
class PluginConfig:
    timeout: int = 5000
    retries: int = 3

typed = config.bind("my_plugin", PluginConfig)
# typed.timeout == 5000
```

```typescript
// TypeScript
const config = Config.load('apcore.yaml');

// Full namespace dict
const pluginCfg = config.namespace('myPlugin');

// Dot-path access
const timeout = config.get('myPlugin.timeout', 5000);

// Typed deserialization (class constructor)
class PluginConfig {
  timeout: number;
  retries: number;
  constructor(data: Record<string, unknown>) {
    this.timeout = (data['timeout'] as number) ?? 5000;
    this.retries = (data['retries'] as number) ?? 3;
  }
}
const typed = config.bind('myPlugin', PluginConfig);
```

```rust
// Rust
let config = Config::load("apcore.yaml")?;

// Full namespace dict
let plugin_cfg = config.namespace("my_plugin"); // HashMap<String, Value>

// Typed deserialization
#[derive(serde::Deserialize)]
struct PluginConfig { timeout: u64, retries: u32 }

let typed: PluginConfig = config.get_typed("my_plugin")?;
```

## Mounting External Sources (§9.7)

Attach data from a file or in-memory dict to a namespace. Mounted data is merged over namespace defaults.

```python
# Python
config.mount("my_plugin", source={"timeout": 10000})          # dict source
config.mount("my_plugin", source="./my-plugin.yaml")           # file source
```

```typescript
// TypeScript
config.mount('myPlugin', { fromDict: { timeout: 10000 } });    // dict source
config.mount('myPlugin', { fromFile: './my-plugin.yaml' });    // file source
```

```rust
// Rust
use apcore::MountSource;

config.mount("my_plugin", MountSource::Dict(data))?;           // dict source
config.mount("my_plugin", MountSource::File("./my-plugin.yaml".into()))?;  // file source
```

**Error:** `CONFIG_MOUNT_ERROR` if namespace is `_config`, source file is missing, or file is not a valid YAML mapping.

## Typed Bind (§9.8)

Deserialize a namespace subtree into a typed value:

| Language | API |
|----------|-----|
| Python | `config.bind(namespace, dataclass_type)` → instance |
| TypeScript | `config.bind<T>(namespace, ClassName)` → `T` |
| Rust | `config.get_typed::<T>(namespace)` → `Result<T, ...>` |

**Error:** `CONFIG_BIND_ERROR` if instantiation fails.

## Hot Reload (§9.9)

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

`Config.load()` with no path argument searches these locations in order:

1. `$APCORE_CONFIG_FILE` environment variable
2. `./project.yaml`
3. `./project.yml`
4. `./apcore.yaml`
5. `./apcore.yml`
6. `~/.config/apcore/config.yaml` (XDG)
7. `~/.apcore/config.yaml`

Falls back to `Config.from_defaults()` (Python) / `Config::from_defaults()` (Rust) if nothing is found.

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

my_plugin:
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

## Key Files

| File | Purpose |
|------|---------|
| `src/apcore/config.py` (Python) | `Config` class, `register_namespace()`, `namespace()`, `bind()`, `mount()`, `discover_config_file()` |
| `src/config.ts` (TypeScript) | `Config` class with identical API shape |
| `src/config.rs` (Rust) | `Config` struct, `NamespaceRegistration`, `MountSource`, `ConfigMode` |

## Testing Strategy

- **Namespace registration:** reserved name rejection, duplicate rejection, env prefix conflict.
- **Env routing:** longest-prefix-match correctness, separator conversion, reserved prefix enforcement.
- **Mount:** dict mount merge, file mount parse, `_config` rejection, missing file error.
- **Bind:** successful deserialization, bind error on invalid data.
- **Mode detection:** legacy YAML activates legacy mode, `apcore:` key activates namespace mode.
- **Config discovery:** `$APCORE_CONFIG_FILE` env precedence, CWD candidates, XDG path, fallback to defaults.
- **Hot reload:** changes in YAML picked up, mounts re-applied.
