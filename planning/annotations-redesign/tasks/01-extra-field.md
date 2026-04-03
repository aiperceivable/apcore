# Task 1: Extra Field

Add `extra` extension dictionary to `ModuleAnnotations` in all 3 SDKs.

## Python
- Add `extra: dict[str, Any] = field(default_factory=dict)` to `ModuleAnnotations`

## TypeScript
- Add `readonly extra: Readonly<Record<string, unknown>>` to `ModuleAnnotations` interface
- Add `extra: Object.freeze({})` to `DEFAULT_ANNOTATIONS`

## Rust
- Add `pub extra: HashMap<String, serde_json::Value>` with `#[serde(default, flatten)]`
- Update `Default` impl

## Tests
- Default-construct, assert `extra` is empty
- Construct with extra values, assert they round-trip
- Unknown JSON keys captured into `extra` (Rust via serde flatten)
