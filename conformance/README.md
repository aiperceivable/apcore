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
| `trace_context.json` | Issue #35 | TraceContext W3C alignment: ordered tracestate roundtrip, 32-entry cap, malformed-entry tolerance, case-insensitive `traceparent`/`tracestate` header lookup, dynamic `trace_flags` honoring, optional `parent_id` override on `inject()` (`^[0-9a-f]{16}$`) with `INVALID_PARENT_ID` rejection |
| `event_naming.json` | Issue #36 / D-34 | Event-name canonicalization: `apcore.<subsystem>.<event>` form for registry/health events, legacy dual-emit during v0.21.x with `deprecated:true`, glob subscription matching for `apcore.registry.*` / `apcore.health.*`, scoping that prevents cross-subsystem glob bleed |
| `contextual_audit.json` | Issue #45.2 / D-35 | Contextual auditing for control-plane modules: `caller_id` populated in audit event payloads for `update_config` / `toggle_feature` / `reload_module`, `@external` default for unauthenticated callers, redacted `identity` snapshot inclusion, `x-sensitive` field redaction, audit emission even when no `AuditStore` is configured |
| `pipeline_step_middleware.json` | Issue #33 | Pipeline `StepMiddleware` lifecycle: before/after onion order, on_step_error first-recovery-wins, null-passthrough propagation, executed-only on_step_error invocation when before_step raises, async callbacks awaited, before_step input replacement |
| `pipeline_failfast_config.json` | Issue #33 | Pipeline configuration fail-fast: `ConfigurationError` at parse time for missing step references in `configure:` and `step_middleware:`, `PipelineDependencyError` at strategy construction for unmet `requires`/`provides` |
| `storage_backend.json` | Issue #43, D-39 | StorageBackend trait/interface: save+get round-trip, list-with-prefix, idempotent delete, namespace isolation, save-overwrites |
| `overrides_store.json` | Issue #45.1, D-40 | OverridesStore (FileOverridesStore + InMemoryOverridesStore): save persists across reopen, startup applies overrides after base config, in-memory store for tests, missing path on first run is OK, delete idempotency |
| `error_fingerprinting.json` | Issue #43 §4 | Error fingerprint = (error_code, top-frame hash, sanitized message template). UUID/timestamp/numeric-ID dedup, distinct error codes never collapse, distinct call sites never collapse |
| `redaction_config.json` | Issue #43 §5 | Configurable redaction via `obs.redaction.regex_patterns` and `obs.redaction.sensitive_keys`. Default sensitive_keys cover common credential terms; trace_id/caller_id/target_id/module_id/span_id MUST never be redacted |
| `reload_path_filter.json` | Issue #45.4 | Granular reload — `path_filter` glob restricts re-discovery, no `path_filter` = single-`module_id` reload unchanged, zero-match filter is a no-op, both fields together raises `MODULE_RELOAD_CONFLICT` |
| `usage_exporter.json` | Issue #45 §3, D-55 | `UsageExporter` push interface — `NoopUsageExporter` drops summaries (default), `PeriodicUsageExporter` polls `UsageCollector.summary()` at `interval_seconds` (default 3600) and calls `exporter.export(summary)`, `stop()` halts the loop, awaits `exporter.shutdown()`, and is idempotent |
| `sensitive_keys_default.json` | Issue #43 §5, D-54 | Canonical default `obs.redaction.sensitive_keys` 16-entry list shipped by all 3 SDKs (`_secret_*` legacy glob + 15 credential terms), case-insensitive substring matching, override replaces (does not merge), and the never-redact correlation fields (`trace_id`/`caller_id`/`target_id`/`module_id`/`span_id`) |

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

Most test cases use `expected` for the expected result. Four fixtures use alternative or extended patterns:

- **`context_serialization.json`** — `all_identity_types_serialize` uses a `sub_cases` array instead of a single `expected`. Test runners should iterate each sub-case and validate `input_identity` → `expected_type`.
- **`annotations_extra_round_trip.json`** — `producer_must_not_emit_both_forms` uses `forbidden_root_keys` instead of `expected`. Test runners should verify that none of the listed keys appear in the serialized output.
- **`schema_hardening_recursive.json`** — has a root-level `schema` object (the shared `TreeNode` recursive schema) used by all test cases. Test runners should read the top-level `schema` as the schema-under-test for every case in this file; individual test cases do not carry their own `schema` field.
- **`schema_hardening_formats.json`** — has a root-level `format_mappings` object listing the canonical language-native type for each JSON Schema `format` keyword across Python, TypeScript, and Rust. This is reference metadata for SDK implementors; test runners may ignore it and process `test_cases` normally.

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
