# APCore Conformance Test Fixtures

Shared JSON fixtures for cross-language conformance testing. All SDK implementations **should** consume these fixtures to guarantee behavioral parity.

## Fixtures

| File | Algorithm | Description |
|------|-----------|-------------|
| `pattern_matching.json` | A09 | Wildcard pattern matching rules |
| `specificity.json` | A10 | ACL rule specificity scoring |
| `normalize_id.json` | A02 | Cross-language module ID normalization |
| `call_chain.json` | A20 | Call chain safety (depth, frequency, circularity) |
| `error_codes.json` | — | Error code registry collision detection |
| `version_negotiation.json` | A14 | SDK/module version compatibility |
| `acl_evaluation.json` | — | ACL rule evaluation with first-match-wins |
| `config_env.json` | — | Configuration environment variable override |
| `context_serialization.json` | — | Cross-language Context serialize/deserialize round-trip |
| `schema_validation.json` | §4.15 | Schema validation edge cases and type coercion |

## Usage

SDK test suites should load these fixtures and run each test case:

```python
# Python example
import json
with open("conformance/fixtures/pattern_matching.json") as f:
    cases = json.load(f)["test_cases"]
for case in cases:
    result = match_pattern(case["pattern"], case["value"])
    assert result == case["expected"]
```

```typescript
// TypeScript example
import cases from '../../../apcore/conformance/fixtures/pattern_matching.json';
for (const tc of cases.test_cases) {
  expect(matchPattern(tc.pattern, tc.value)).toBe(tc.expected);
}
```
