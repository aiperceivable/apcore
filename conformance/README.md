# APCore Conformance Test Fixtures

Shared JSON fixtures for cross-language conformance testing. All SDK implementations **should** consume these fixtures to guarantee behavioral parity.

## Format

All fixture files use **JSON** format (`.json`), with one intentional exception:

- `binding_yaml_canonical.yaml` — a `.yaml` file used to test YAML binding-file loading behavior; it is itself a binding descriptor, not a conformance test-case container.

SDK conformance runners **must** load `.json` files with a JSON parser. The `.yaml` file is consumed directly by binding-loader tests, not by the generic conformance runner.

> **Note for tooling authors:** Some ecosystem skill docs describe conformance fixtures as YAML; this is incorrect. The canonical format for all conformance test cases is JSON. The README is the authoritative format reference.

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
| `schema_hardening_union.json` | §4.15 Hardening | anyOf/oneOf union evaluation: all-branches, exactly-one, ambiguous-error |
| `schema_hardening_recursive.json` | §4.15 Hardening | Recursive schema support: TreeNode pattern, depth 1-5 |
| `schema_hardening_constraints.json` | §4.15 Hardening | Rust validator parity: min/max/minLength/maxLength/pattern/not constraints |
| `schema_hardening_formats.json` | §4.15 Hardening | Semantic format mapping: date-time/date/email/uri/uuid/ipv4/ipv6 |
| `schema_hardening_cache.json` | §4.15 Hardening | Content-addressable schema cache: deduplication by content hash |
| `annotations_extra_round_trip.json` | §4.4 | ModuleAnnotations.extra wire-format (nested form, legacy flattened tolerance, precedence) |
| `config_defaults.json` | — | Canonical default values conformance across all SDKs |
| `stream_aggregation.json` | — | Stream chunk deep-merge aggregation algorithm |
| `approval_gate.json` | A05 | Approval gate Step 5: skip/fire conditions and all four result-status outcomes |
| `middleware_on_error_recovery.json` | A11 | After-middleware error recovery: first-dict-wins, null passthrough, success non-override |
| `identity_system.json` | — | Identity construction, field access, and context propagation (AC-014, AC-015) |
| `context_trace_parent.json` | §10.5 | Context.create trace_parent strict validation: 32-hex only, W3C-invalid rejection, no auto-normalization |
| `dependency_version_constraints.json` | §5.3, §5.15.2 | Dependency `version` constraint enforcement: exact, `>=`, `<=`, `^`, `~`, ranges, optional skip |
| `binding_errors.json` | — | 6 canonical cross-SDK error message parity test cases (`BindingFileInvalidError`, `BindingSchemaModeConflictError`, `BindingSchemaInferenceFailedError`, `PipelineHandlerNotSupportedError`, `BindingInvalidTargetError`, `BindingModuleNotFoundError`) |
| `binding_yaml_canonical.yaml` | — | Cross-SDK binding YAML canonical fixture (`.yaml` format): permissive auto_schema, explicit schemas with display overlay, strict auto_schema mode |
| `multi_module_discovery.json` | §2.1.1 | Multi-class discovery: ID derivation, snake_case conversion, conflict detection, backward compatibility |
| `pipeline_hardening.json` | §5.16 | Pipeline execution hardening: fail-fast, continue-on-ignored-error, replace-semantic, `run_until` early termination, O(1) lookup verification |
| `event_management_hardening.json` | §9 (Events Hardening) | Cross-language SubscriberFactory parity, built-in File/Stdout/Filter subscribers, circuit-breaker state machine |
| `middleware_hardening.json` | §MW (Middleware Hardening) | Context namespace rules, CircuitBreakerMiddleware state machine, TracingMiddleware span lifecycle, async handler detection |
| `async_task_evolution.json` | Issue #34 | Pluggable TaskStore backends (InMemory/Redis), retry with configurable exponential backoff, Reaper TTL-based auto-cleanup |
| `observability_hardening.json` | Issue #43 | Pluggable ObservabilityStore, BatchSpanProcessor queue/drop behaviour, O(log N) ErrorHistory eviction, error fingerprint deduplication and normalization, configurable redaction (field+value patterns), Prometheus required-metric presence |
| `system_modules_hardening.json` | Issue #45 | Config/toggle persistence to overrides file, overrides loaded after base config on startup, audit entry actor extraction, audit entry before/after change recording, Prometheus UsageCollector metrics export, path-filter bulk reload, module_id+path_filter conflict error, startup fail_on_error=True raises, fail_on_error=False continues, Rust Result return type |

## Coverage Gaps

The following PROTOCOL_SPEC algorithms do **not** yet have conformance fixtures:

| Algorithm | Spec Section | Description |
|-----------|-------------|-------------|
| A01 | §2, §3.3 | Directory to canonical ID conversion |
| A07 | §5.3 | Dependency topological sorting |
| A12 | §9.3–§9.7 | Config validation (non-namespace mode) |
| A21 | §12.8 | Safe module unregister |
| A22 | §12.7.5 | Enforce timeout |
| A23 | §4.16 | Strict schema conversion (`to_strict_schema`) — remaining gap |

The five `schema_hardening_*.json` fixtures added in Issue #44 close the §4.15 composition keyword gap: union type exhaustive evaluation, recursive `$ref` resolution, numerical/string constraint parity, semantic format mapping, and content-hash deduplication are now all covered.

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
