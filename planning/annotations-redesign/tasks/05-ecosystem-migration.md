# Task 5: Ecosystem Migration

Update exports and constants.

## Python
- Add `DEFAULT_ANNOTATIONS` to `__init__.py` imports and `__all__`
- Make optional fields required with defaults (already done by frozen dataclass)

## TypeScript
- Make 5 optional fields (`cacheable?`, `cacheTtl?`, etc.) required
- Export `createAnnotations`, `annotationsToJSON`, `annotationsFromJSON` from index.ts
- Negative `cacheTtl` clamping in `createAnnotations()`

## Tests
- `createAnnotations({destructive: true})` fills all defaults
- `DEFAULT_ANNOTATIONS` exported and accessible
