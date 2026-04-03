---
paths:
  - "conformance/**"
---

# Conformance Fixture Rules

Conformance fixtures ensure behavioral parity across Python, TypeScript, and Rust SDKs.

## File format
Each fixture is a JSON file with this structure:
```json
{
  "description": "Brief description of what this fixture tests",
  "test_cases": [
    {
      "id": "descriptive_snake_case_id",
      ...input fields...,
      "expected": ...expected result...
    }
  ]
}
```

## Naming
- Files: `snake_case.json` in `conformance/fixtures/`
- Test case IDs: `snake_case`, descriptive (e.g., `first_match_wins`, `default_deny`)

## Terminology
- Use `caller_id` and `target_id` (not `caller` / `target` / `source` / `destination`)

## Coverage
- Include positive cases (expected: true/pass)
- Include negative cases (expected: false/fail)
- Include edge cases (empty input, boundary values, Unicode)
- Each fixture should reference the algorithm it tests (e.g., A02, A09, A10) where applicable

## When adding a new fixture
1. Create the JSON file in `conformance/fixtures/`
2. Add a row to `conformance/README.md` fixtures table with Algorithm reference
3. Verify the JSON is valid and parseable
