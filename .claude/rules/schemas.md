---
paths:
  - "schemas/**"
---

# JSON Schema Editing Rules

## Format
- Must use JSON Schema Draft 2020-12 (`"$schema": "https://json-schema.org/draft/2020-12/schema"`)
- File naming: `<name>.schema.json`
- Pretty-printed with 2-space indentation

## Required practices
- Every `property` must include a `description` field
- Use `"type"` explicitly — do not rely on implicit type inference
- Include `"required"` arrays where applicable
- All `$ref` references must resolve to valid targets within the schema set

## When modifying
- Check all files in `schemas/` for cross-references before renaming or removing fields
- SDK implementations validate against these schemas — breaking changes require a version bump
- Test with a JSON Schema validator after editing
