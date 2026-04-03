# Task 7: Cross-Language Fixes

## TypeScript: removeRule fix
- Replace `JSON.stringify` comparison with element-wise comparison
- Add `arraysEqual` and `deepEqual` helper functions

## Rust: Empty callers fix
- Remove `if rule.callers.is_empty() { true }` wildcard behavior
- Empty callers should match nothing (align with Python/TS)

## Rust: audit_logger to constructor
- Add `audit_logger` parameter to `ACL::new()`
- Keep `set_audit_logger()` as convenience method
