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
| `annotations_extra_round_trip.json` | §4.4 | ModuleAnnotations.extra wire-format (nested form, legacy flattened tolerance, precedence) |
| `config_defaults.json` | — | Canonical default values conformance across all SDKs |
| `stream_aggregation.json` | — | Stream chunk deep-merge aggregation algorithm |
| `approval_gate.json` | A05 | Approval gate Step 5: skip/fire conditions and all four result-status outcomes |
| `middleware_on_error_recovery.json` | A11 | After-middleware error recovery: first-dict-wins, null passthrough, success non-override |
| `identity_system.json` | — | Identity construction, field access, and context propagation (AC-014, AC-015) |
| `context_trace_parent.json` | §10.5 | Context.create trace_parent strict validation: 32-hex only, W3C-invalid rejection, no auto-normalization |
| `dependency_version_constraints.json` | §5.3, §5.15.2 | Dependency `version` constraint enforcement: exact, `>=`, `<=`, `^`, `~`, ranges, optional skip |

## Coverage Gaps

The following PROTOCOL_SPEC algorithms do **not** yet have conformance fixtures:

| Algorithm | Spec Section | Description |
|-----------|-------------|-------------|
| A01 | §2, §3.3 | Directory to canonical ID conversion |
| A07 | §5.3 | Dependency topological sorting |
| A12 | §9.3–§9.7 | Config validation (non-namespace mode) |
| A21 | §12.8 | Safe module unregister |
| A22 | §12.7.5 | Enforce timeout |
| A23 | §4.16 | Strict schema conversion (`to_strict_schema`) |

## Non-Standard Test Patterns

Most test cases use `expected` for the expected result. Two fixtures use alternative patterns:

- **`context_serialization.json`** — `all_identity_types_serialize` uses a `sub_cases` array instead of a single `expected`. Test runners should iterate each sub-case and validate `input_identity` → `expected_type`.
- **`annotations_extra_round_trip.json`** — `producer_must_not_emit_both_forms` uses `forbidden_root_keys` instead of `expected`. Test runners should verify that none of the listed keys appear in the serialized output.

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
