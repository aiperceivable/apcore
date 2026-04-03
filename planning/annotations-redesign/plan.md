# Annotations Redesign - Implementation Plan

## Feature
Add `extra` extension dictionary to `ModuleAnnotations`, change `pagination_style` to open string, introduce canonical snake_case wire format, and `createAnnotations()` factory in TypeScript.

## Tasks

| # | Task ID | Description | SDKs | Status |
|---|---------|-------------|------|--------|
| 1 | extra-field | Add `extra: dict/Record/HashMap` to ModuleAnnotations | Python, TS, Rust | pending |
| 2 | pagination-style | Change `pagination_style` from Literal/union to `str/string` | Python, TS | pending |
| 3 | wire-format | TS toJSON/fromJSON with snake_case canonical wire format | TS | pending |
| 4 | python-post-init | Python `__post_init__` for cache_key_fields tuple + extra copy + cache_ttl clamp | Python | pending |
| 5 | ecosystem-migration | Update exports, DEFAULT_ANNOTATIONS, from_dict | Python, TS | pending |

## Execution Order
1. extra-field + pagination-style + python-post-init (parallel across SDKs)
2. wire-format (depends on TS extra-field)
3. ecosystem-migration (depends on all above)

## TDD Approach
Each task: write failing tests first, implement, run full suite.
