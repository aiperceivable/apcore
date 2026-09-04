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
| `id_conflict_reserved_words.json` | A02 | Reserved-word ID conflicts are detected on the FIRST SEGMENT only; later segments are unrestricted (spec §2.6 step 2, v1.26.0, #99) |
| `call_chain.json` | A20 | Call chain safety (depth, frequency, circularity) |
| `error_codes.json` | A17 | Error code registry collision detection; canonical 14 reserved prefixes + exact-framework-code rejection for one-off codes (sync A-D-006/A-D-007) |
| `version_negotiation.json` | A14 | SDK/module version compatibility |
| `acl_evaluation.json` | — | ACL rule evaluation with first-match-wins |
| `acl_handler_error.json` | A08 | An unevaluable ACL condition resolves toward refusing access — a `deny` rule takes effect, an `allow` rule does not grant — and `handler_error` names the condition path (spec §6.1.1/§6.1.4, #100/#106) |
| `acl_root_discovery.json` | D-64 / Issue #74 | Config-driven ACL discovery: `acl.root` activation, unified `./acl` default, directory `global_acl.yaml` convention, caller-supplied-executor skip, and the missing-path-no-op invariant (missing path attaches NO ACL — never a synthesized default-deny) |
| `acl_argument_scoped_approval.json` | — | Authorization and approval requirement are two orthogonal results; the built-in structure-only `arguments` condition scopes a rule to THIS call; an unevaluable `allow` rule's `approval: required` is pending, not discarded, so a broader rule cannot grant the call unapproved. Every case runs twice — with a projection and without (spec §6.1.1/§6.1.6/§6.1.7/§6.1.8/§6.9, #108/#109) |
| `acl_rule_key_closure.json` | — | ACL rule keys are a closed set; an unknown or reserved key fails the load (spec §6.1, #107) |
| `acl_effect_value_closure.json` | — | A rule's `effect` value is a closed set and MUST be rejected at every entry point that accepts a rule — file loading, direct construction, runtime insertion; `default_effect` on the same terms (spec §6.1.5, #111) |
| `acl_pattern_arity.json` | — | A `callers` / `targets` pattern array's shape is a closed set at every entry point — non-empty, non-empty elements, `$or` with at least one operand, `$not` with exactly one, and no reserved token away from index 0; plus a validator-only tier for arrays that are well-formed and still match nothing (spec §6.2.1, #112) |
| `error_serialization.json` | — | ModuleError serialization uses snake_case wire keys (sync A-D-008) |
| `error_recovery_metadata.json` | — | Default AI error-recovery metadata (retryable, user_fixable) resolved per error code; user_fixable=true when caller can fix by changing input/config |
| `async_task_cancellation.json` | Issue #34 | Over-capacity submit raises TASK_LIMIT_EXCEEDED; cancel during backoff stops further retries (sync A-D-003/A-D-004) |
| `executor_trace_cancellation.json` | — | call_with_trace cancellation bypasses on_error chain (sync A-D-001, D-19/D-20) |
| `config_env.json` | — | Configuration environment variable override |
| `context_serialization.json` | — | Cross-language Context serialize/deserialize round-trip |
| `schema_validation.json` | §4.15 | Schema validation edge cases, plus the **opt-in library-level** coercing mode. Cases carrying `expected_valid_strict` / `expected_valid_coerce` document BOTH modes and SHOULD be asserted against both, naming the mode explicitly rather than relying on a constructor default. The coercing mode is a knob on the standalone validator API only — it never reaches the module-invocation boundary, which is covered by `schema_keyword_parity.json` (TYPE_MAPPING §17.3). |
| `schema_hardening_union.json` | §4.15 Hardening | anyOf/oneOf union evaluation: all-branches, exactly-one, ambiguous-error |
| `schema_hardening_recursive.json` | §4.15 Hardening | Recursive schema support: TreeNode pattern, depth 1-5 |
| `schema_hardening_constraints.json` | §4.15 Hardening | Rust validator parity: min/max/minLength/maxLength/pattern/not constraints |
| `schema_hardening_formats.json` | type-mapping §11.1 / §4.15 | Semantic format mapping: date-time/date/email/uri/uuid/ipv4/ipv6. The `valid: true` half — `format` is an annotation and never fails validation (type-mapping §11.1) — holds on all three SDKs at the module-invocation boundary. The `warn_logged` half does **not** pin equivalent behaviour: see the fixture's `driver_contract` |
| `schema_keyword_parity.json` | §4.15 / type-mapping §17 / JSON Schema §6, §10.2, §10.3, §11 | Keyword handling parity **at the validation boundary**: `type` arrays as unions with per-branch option keywords, `type` and combinator siblings as independent assertions, non-scalar `enum`/`const`, array/object validation keywords, bool-vs-number instance types, `additionalProperties` sub-schema forms, `format` as an annotation, the applicator set (`prefixItems` + `items`, `patternProperties`, `propertyNames`, `dependentRequired`, `dependentSchemas`, `if`/`then`/`else`, `unevaluatedItems`/`unevaluatedProperties`) including their inertness on instances of other types, recursive `$ref` (`#`, root `$id`, `#/$defs/…`), `oneOf` exclusivity at nested and root locations, `uniqueItems` over object/array members, and the **no-type-coercion rule at the boundary** (`"42"` is not an `integer`, `1` is not a `boolean`, `42` is not a `string` — while `4.0` still IS an `integer` per §6.1.1, since the rule is about instance types, not renderings). Drivers MUST exercise the SDK's schema conversion + validator (the path a module call takes), not the raw-JSON-Schema validator, and MUST run it with coercion off — that path never coerces, under any host configuration (TYPE_MAPPING §17.3). See `driver_contract` in the fixture. |
| `openai_strict_compat.json` | DECLARATIVE_CONFIG_SPEC §6.2 / §6.6 | OpenAI structured-outputs strict-mode compatibility detection backing `BindingStrictSchemaIncompatibleError` on the `auto_schema: strict` binding path. Pins the supported/unsupported keyword set (`anyOf` supported except at root; `$ref`/`$defs` recursion supported; `oneOf`/`allOf`/`not`/`if`/`then`/`else`/`minLength`/`maxLength`/`patternProperties`/`uniqueItems`/… unsupported; nine supported `format` values) and the sorted, de-duplicated feature-path list all three SDKs must emit byte-identically. Detection never rewrites — an author-written `oneOf` is reported, not downgraded to `anyOf` — see `driver_contract`. |
| `schema_export_envelope.json` | §4.16 / A23 | `Registry.export_schema(module_id, strict)` envelope parity — exactly the four keys `schemas/module-schema-export.schema.json` declares. Descriptor metadata (`name`/`version`/`tags`/`annotations`/`examples`) belongs to `system.manifest.module`, and there is no sibling `definitions` key: `$defs` stay inside `input_schema`. Drivers MUST assert the key set EXACTLY — an extra key is the defect this pins. |
| `config_key_governance.json` | §9.1 / §9.3 / §9.15.3 | Governance of the configuration key surface: an SDK default or constraint table MUST stay inside the key set `apcore-config.schema.json` + `defaults.schema.json` + `sys-modules.schema.json` declare, and MUST reproduce every canonical default exactly. `allowed_keys` / `canonical_defaults` are derived from those schemas by `conformance/generate_config_key_governance.py` — never hand-edit them. |
| `schema_strict_conversion.json` | A23 / §4.16 | `to_strict_schema()` output parity: the exact strict-mode schema all three SDKs must emit. Pins the object-detection rule (`properties` alone identifies an object schema — `type` may be absent, `"object"`, or `["object", "null"]`; `properties` beside a non-object `type` is inert), the single nullable spelling (`{anyOf: [<original>, {type: "null"}]}` for a property with no `type` keyword — an author-written `oneOf`/`anyOf` is wrapped, never appended to), recursion into `prefixItems`/`items`/combinators/`$defs`, `x-*` + `default` stripping, and the sorted-and-complete `required` list. Drivers MUST call A23 directly — see `driver_contract`. |
| `schema_hardening_cache.json` | §4.15 Hardening | Content-addressable schema cache: deduplication by content hash |
| `schema_content_hash.json` | A-D-037 | Cross-language content-hash canonicalization parity: float rendering (1.0), non-ASCII Unicode keys/values, large integers, unsorted nested keys, baseline; SDKs compute and hashes are compared cross-repo (no `expected` recorded) |
| `annotations_extra_round_trip.json` | §4.4 | ModuleAnnotations.extra wire-format (nested form, legacy flattened tolerance, precedence) |
| `config_defaults.json` | — | Canonical default values conformance across all SDKs |
| `stream_aggregation.json` | — | Stream chunk deep-merge aggregation algorithm |
| `approval_gate.json` | A05 | Approval gate Step 5: skip/fire conditions and all four result-status outcomes |
| `preflight_disclosure.json` | §12.8.5.1 | `Executor.validate()` withholds module-level introspection from a caller the ACL denied: no `module_preflight` / `module_preview` check, empty `predicted_changes`, and the hooks themselves not invoked. Module lookup is Step 3 and the ACL check is Step 4, so all three SDKs gated these hooks on "lookup succeeded" alone and ran module-authored code for a denied caller. Scoped to AUTHORIZATION: a failed `schema` check does **not** suppress introspection. Drivers MUST run the real `validate()` against a real Registry and ACL, and MUST observe hook invocation from inside the hook bodies — see `driver_contract` |
| `middleware_on_error_recovery.json` | A11 | After-middleware error recovery: first-dict-wins, null passthrough, success non-override |
| `identity_system.json` | — | Identity construction, field access, and context propagation (AC-014, AC-015) |
| `context_trace_parent.json` | §10.5 | Context.create trace_parent strict validation: 32-hex only, W3C-invalid rejection, no auto-normalization |
| `context_create.json` | Issue #66 | Context.create unified-signature contract: 6-param input list, executor/caller_id NOT inputs, Executor binding (local + deserialize + hot-reload), idempotent same-executor rebind, cross-executor conflict, child() propagation of executor + cancel_token, distributed cancel_token/global_deadline semantics, TraceParent embeds tracestate |
| `dependency_version_constraints.json` | §5.3, §5.15.2 | Dependency `version` constraint enforcement: exact, `>=`, `<=`, `^`, `~`, ranges, optional skip |
| `binding_errors.json` | — | 6 canonical cross-SDK error message parity test cases (`BindingFileInvalidError`, `BindingSchemaModeConflictError`, `BindingSchemaInferenceFailedError`, `PipelineHandlerNotSupportedError`, `BindingInvalidTargetError`, `BindingModuleNotFoundError`) |
| `binding_yaml_canonical.yaml` | — | Cross-SDK binding YAML canonical fixture (`.yaml` format): permissive auto_schema, explicit schemas with display overlay, strict auto_schema mode |
| `multi_module_discovery.json` | §2.1.1 | Multi-class discovery: ID derivation, snake_case conversion, conflict detection, backward compatibility |
| `pipeline_hardening.json` | §5.16 | Pipeline execution hardening: fail-fast, continue-on-ignored-error, replace-semantic, `run_until` early termination, O(1) lookup verification |
| `event_management_hardening.json` | §9 (Events Hardening) | Cross-language SubscriberFactory parity, built-in File/Stdout/Filter subscribers, circuit-breaker state machine |
| `event_delivery_semantics.json` | Issue #61 | Per-subscriber retry policy (max_attempts / backoff), apcore.event.delivery_failed DLQ event payload schema, no-retry-on-DLQ rule, SDK-generated subscriber_id fallback |
| `middleware_hardening.json` | §MW (Middleware Hardening) | Context namespace rules, CircuitBreakerMiddleware state machine, TracingMiddleware span lifecycle, async handler detection |
| `registry_load_ordering.json` | Issue #65 | Registry strong-guarantee invariant: visibility-after-on_load, callback-failure-blocks-visibility, apcore.registry.module_load_failed payload, concurrent same-ID DUPLICATE_MODULE_ID rejection, per-module parallel callbacks |
| `async_task_evolution.json` | Issue #34 | Pluggable TaskStore backends (InMemory/Redis), retry with configurable exponential backoff, Reaper TTL-based auto-cleanup |
| `observability_hardening.json` | Issue #43 | Pluggable ObservabilityStore, BatchSpanProcessor queue/drop behaviour, O(log N) ErrorHistory eviction, error fingerprint deduplication and normalization, configurable redaction (field+value patterns), Prometheus required-metric presence |
| `system_modules_hardening.json` | Issue #45 | Config/toggle persistence to overrides file, overrides loaded after base config on startup, audit entry actor extraction, audit entry before/after change recording, Prometheus UsageCollector metrics export, path-filter bulk reload, module_id+path_filter conflict error, startup fail_on_error=True raises, fail_on_error=False continues, Rust Result return type |
| `trace_context.json` | Issue #35 | TraceContext W3C alignment: ordered tracestate roundtrip, 32-entry cap, malformed-entry tolerance, case-insensitive `traceparent`/`tracestate` header lookup, dynamic `trace_flags` honoring, optional `parent_id` override on `inject()` (`^[0-9a-f]{16}$`) with `INVALID_PARENT_ID` rejection |
| `event_naming.json` | Issue #36 / D-34 | Event-name canonicalization: `apcore.<subsystem>.<event>` form for registry/health events, the inverse assertion that the removed v0.21.x legacy names are NOT emitted (apcore#78), glob subscription matching for `apcore.registry.*` / `apcore.health.*`, scoping that prevents cross-subsystem glob bleed |
| `contextual_audit.json` | Issue #45.2 / D-35 | Contextual auditing for control-plane modules: `caller_id` populated in audit event payloads for `update_config` / `toggle_feature` / `reload_module`, `@external` default for unauthenticated callers, redacted `identity` snapshot inclusion, `x-sensitive` field redaction, audit emission even when no `AuditStore` is configured |
| `pipeline_step_middleware.json` | Issue #33 | Pipeline `StepMiddleware` lifecycle: before/after onion order, on_step_error first-recovery-wins, null-passthrough propagation, executed-only on_step_error invocation when before_step raises, async callbacks awaited, `before_step`'s return value being IGNORED (it is an observation hook — a Step is `execute(ctx)` and has no inputs parameter to replace), and the two OPPOSITE recovery paths: a `before_step` failure is terminal and its recovery value MUST be discarded (else the pipeline advances past `acl_check` / `approval_gate` — an authorization bypass), while a recovered step BODY MUST still fire `after_step` so the onion closes |
| `pipeline_failfast_config.json` | Issue #33, #89 | Pipeline configuration fail-fast: wire code `PIPELINE_CONFIGURATION_ERROR` at parse time for a missing step reference in the `configure:` map, `PIPELINE_DEPENDENCY_ERROR` at strategy construction for unmet `requires`/`provides`, and the configurable field set — exactly `match_modules`, `ignore_errors`, `pure`, `timeout_ms`, with any other key a parse-time error and `requires`/`provides` explicitly not configurable. Drivers MUST assert the wire code — asserting the class name `ConfigurationError` passes on all three SDKs while they emit three different codes — and MUST feed the fixture's snake_case keys verbatim rather than translating them into an SDK-local spelling |
| `storage_backend.json` | Issue #43, D-39 | StorageBackend trait/interface: save+get round-trip, list-with-prefix, idempotent delete, namespace isolation, save-overwrites. `value` is a JSON **object**, never bytes — all three collectors index into it |
| `overrides_store.json` | Issue #45.1, D-40 | OverridesStore (FileOverridesStore + InMemoryOverridesStore) on the D-47 whole-map surface `load()` / `save(mapping)`: save persists across reopen, startup applies overrides after base config, in-memory store for tests, missing path on first run is OK, key removal as read-modify-write |
| `error_fingerprinting.json` | Issue #43 §4 | Error fingerprint = (error_code, top-frame hash, sanitized message template). UUID/timestamp/numeric-ID dedup, distinct error codes never collapse, distinct call sites never collapse |
| `redaction_config.json` | Issue #43 §5 | Configurable redaction via `obs.redaction.regex_patterns` and `obs.redaction.sensitive_keys`. Default sensitive_keys cover common credential terms; trace_id/caller_id/target_id/module_id/span_id MUST never be redacted |
| `reload_path_filter.json` | Issue #45.4 | Granular reload — `path_filter` glob restricts re-discovery, no `path_filter` = single-`module_id` reload unchanged, zero-match filter is a no-op, both fields together raises `MODULE_RELOAD_CONFLICT` |
| `usage_exporter.json` | Issue #45 §3, D-55 | `UsageExporter` push interface — `NoopUsageExporter` drops summaries (default), `PeriodicUsageExporter` polls `UsageCollector.summary()` at `interval_seconds` (default 3600) and calls `exporter.export(summary)`, `stop()` halts the loop, awaits `exporter.shutdown()`, and is idempotent |
| `sensitive_keys_default.json` | Issue #43 §5, D-54 | Canonical default `obs.redaction.sensitive_keys` 16-entry list shipped by all 3 SDKs (`_secret_*` legacy glob + 15 credential terms), case-insensitive substring matching, override replaces (does not merge), and the never-redact correlation fields (`trace_id`/`caller_id`/`target_id`/`module_id`/`span_id`) |
| `acl_agent_scoping.json` | Issue #72 | Canonical AI-agent tool-governance ACL scenario: shared default-deny ruleset (mirrors `examples/acl/agent-tool-governance.yaml`) scoping tools by caller pattern + identity `roles` + `max_call_depth`; `@external` read-only, `reader` read/query with inclusive depth cap (3 allowed, 4 denied), `data_admin` export/delete uncapped; role-separation and missing-identity deny cases |
| `toggle_state_isolation.json` | Issue #71 | Per-APCore-instance ToggleState isolation: disable/enable on one instance does not affect another in the same process, independent per-instance disabled-sets, toggle survives reload of its own instance (A-D-12 re-scoped process-global → instance-scoped) |
| `governance_state.json` | Issue #97 | `Executor.governance_state()` — eight observations plus one derived flag separating *configured* from *actually wired*: the ACL and approval gates are pipeline steps, and the `internal` / `testing` / `minimal` presets remove them, so `acl != null` reports "protected" for an executor whose ACL no step consults. Includes the lookalike case (a custom step NAMED `acl_check`) and the three cases that discriminate the corrected v1.16.0 derived flag from the unsound one published in v1.15.0 |
| `usage_contract.json` | Issue #96 | `system.usage.*` value semantics the two canonical schemas cannot assert — nearest-rank `p99_latency_ms`, `period` as a filter rather than an echo, the `YYYY-MM-DDTHH` hour key, the 24-entry invariant, and the literal `unknown` caller |

## Coverage Gaps

The following PROTOCOL_SPEC algorithms do **not** yet have conformance fixtures:

| Algorithm | Spec Section | Description |
|-----------|-------------|-------------|
| A01 | §2, §3.3 | Directory to canonical ID conversion |
| A07 | §5.3 | Dependency topological sorting |
| A12 | §9.3–§9.7 | Config validation (non-namespace mode) |
| A21 | §12.8 | Safe module unregister |
| A22 | §12.7.5 | Enforce timeout |

A07 is partially exercised by `registry_load_ordering.json` (discovery load order) but has no dedicated fixture for the topological sort itself.

The five `schema_hardening_*.json` fixtures added in Issue #44 close the §4.15 composition keyword gap: union type exhaustive evaluation, recursive `$ref` resolution, numerical/string constraint parity, format annotation semantics, and content-hash deduplication are now all covered. A23 is covered by `schema_strict_conversion.json`.

## Non-Standard Test Patterns

Most test cases use `expected` for the expected result. Seven fixtures use alternative or extended patterns.

A root-level **`driver_contract`** object appears on several fixtures (including all three below). It states which SDK entry point the driver must call and how to compare results. It is a runner contract, **not** a test case — iterate `test_cases` and skip it. The same applies to any other root-level key, such as `keyword_set_source` in `openai_strict_compat.json`.


- **`context_serialization.json`** — `all_identity_types_serialize` uses a `sub_cases` array instead of a single `expected`. Test runners should iterate each sub-case and validate `input_identity` → `expected_type`.
- **`annotations_extra_round_trip.json`** — `producer_must_not_emit_both_forms` uses `forbidden_root_keys` instead of `expected`. Test runners should verify that none of the listed keys appear in the serialized output.
- **`schema_hardening_recursive.json`** — has a root-level `schema` object (the shared `TreeNode` recursive schema) used by all test cases. Test runners should read the top-level `schema` as the schema-under-test for every case in this file; individual test cases do not carry their own `schema` field.
- **`schema_keyword_parity.json`** — every case uses `expected_valid` (a boolean) instead of `expected`. Drive the SDK's schema-conversion + validator pair with coercion off and assert the boolean.
- **`openai_strict_compat.json`** — every case uses `expected_features` (a sorted, de-duplicated list of feature paths) instead of `expected`. Compare with order-sensitive list equality.
- **`schema_strict_conversion.json`** — uses `expected` normally, but array order **is** significant (the `required` list is emitted sorted and a nullable wrapper always puts the original sub-schema first), and the driver must additionally assert the input schema was not mutated.
- **`schema_hardening_formats.json`** — has a root-level `format_mappings` object listing the canonical language-native type for each JSON Schema `format` keyword across Python, TypeScript, and Rust. This is **aspirational reference metadata**, not a record of current behaviour: no SDK performs that mapping today — apcore-python's `generate_model()` annotates a `format: date-time` field as `str`, not `datetime`. Test runners MUST ignore it and process `test_cases` normally. See the fixture's `driver_contract` for which halves of this fixture are normative.

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

### Guards

Five scripts protect the seam between the fixtures, the docs and the SDKs. Each fails on a NEW violation while accepting a recorded backlog, so they can be switched on before the backlog is closed:

| Script | What it catches |
|---|---|
| `check_driver_coverage.py` | A fixture not driven by all three SDKs. A fixture only two SDKs load proves two implementations agree, not three. **Caveat:** it matches a quoted fixture-name literal — a name-mention proxy, not load-proof. |
| `check_expected_keys_read.py` | A top-level `expected` key that **no driver reads**. Such a key reads as covered in the fixture, in review and in every inventory, while nothing is asserted on any SDK. This is the guard that would have caught `wrapped_in` — declared by `pipeline_step_middleware.json`, read by nobody, which let the `MiddlewareChainError` wrapping be removed from two SDKs with every test still green. |
| `check_doc_examples.py` | A documentation example importing a symbol that does not exist in the SDK. |
| `generate_config_key_governance.py` | An SDK default or constraint outside the key surface the canonical schemas declare. |
| `check_module_namespace.py` | `sys.` written as a module-ID namespace. The control plane is `system.*` and `sys` is not reserved, so `sys.control.*` names nothing — in prose that misnames a module, in an ACL rule it is a pattern that matches nothing and is skipped in silence. Excludes host-language `sys.path` / `sys.exit` and the `sys.modules.*` Config Bus key path by construction; anything else needs an allowlist entry with a reason. |

Both fixture guards keep a baseline plus an allowlist. Allowlist entries require a reason and are reported **STALE** once the exemption is no longer needed, so a landed fix surfaces its own cleanup rather than leaving a permanent exemption behind.

**A case can be declared and run by nobody.** `check_driver_coverage.py` answers *does each SDK load this fixture*, `check_expected_keys_read.py` answers *does any driver read this `expected` key*. Neither answers *does any driver RUN this case* — both stay green when the fixture is loaded and the key names are read by some **other** case in the same file. `check_case_pinning.py` answers it directly: it mutates the case's expectation so no correct implementation can satisfy it, runs the drivers, and reports the case if nothing goes red. A case that cannot go red is not coverage. It runs test processes, so it is a scheduled sweep rather than a per-PR check; `conformance/case_pinning_baseline.json` records the known backlog.

**Asserting a class name is not asserting the contract.** Three separate instances were found in the 0.26 sweep: `pipeline_failfast_config.json` asserted `error_type: "ConfigurationError"`, a name all three SDKs share, and was green while they emitted three different wire codes; `pipeline_step_middleware.json` asserted `wrapped_in` against a string literal in one driver and was ignored by the other two; `test_trace_context.py` compared the fixture's own `code` value to a literal. Assert the **wire code**.
