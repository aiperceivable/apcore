# Annotations Redesign - Overview

## Purpose
Enable ecosystem packages (apcore-mcp, apcore-cli, apcore-a2a) to store custom annotations without modifying the core type, via an `extra` extension dictionary on `ModuleAnnotations`.

## Changes by SDK

### Python
- `extra: dict[str, Any]` field with `field(default_factory=dict)`
- `cache_key_fields` type: `list` -> `tuple[str, ...] | None`
- `pagination_style` type: `Literal[...]` -> `str`
- `__post_init__`: list->tuple coercion, extra shallow copy, negative cache_ttl clamping
- `from_dict()` classmethod for deserialization with unknown key capture
- `DEFAULT_ANNOTATIONS` constant
- `_CANONICAL_FIELDS` set

### TypeScript
- `extra: Readonly<Record<string, unknown>>` field
- All 5 optional fields -> required with defaults
- `paginationStyle` type: union -> `string`
- `createAnnotations()` factory function
- `annotationsToJSON()` / `annotationsFromJSON()` for snake_case wire format
- `KNOWN_WIRE_KEYS` set

### Rust
- `pub extra: HashMap<String, serde_json::Value>` with `#[serde(default, flatten)]`
- Updated `Default` impl to include `extra: HashMap::new()`
- `pagination_style` already `String` - no change needed

## Key Convention
Extra keys use `{namespace}.{key}` format: `mcp.category`, `cli.approval_message`, `a2a.guidance`.
