# Task 3: Wire Format

TypeScript toJSON/fromJSON with snake_case canonical wire format.

## Implementation
- `annotationsToJSON(a)` -> `Record<string, unknown>` with snake_case keys
- `annotationsFromJSON(data)` -> `ModuleAnnotations` reading snake_case, unknown keys -> extra
- `KNOWN_WIRE_KEYS` set for filtering

## Tests
- `toJSON()` produces snake_case keys
- `fromJSON()` converts snake_case to camelCase
- Round-trip preserves all fields including extra
- Unknown keys in JSON go to extra
