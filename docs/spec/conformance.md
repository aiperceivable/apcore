---
description: "Defines apcore's three conformance levels (Level 0 Core, Level 1 Standard, Level 2 Full) with per-level MUST/SHOULD/MAY components, test suite requirements, and conformance declaration rules."
---

# apcore — Conformance Definitions

> This document defines conformance levels for apcore framework implementations, test suite requirements, and conformance declaration specifications.

## 1. Overview

### 1.1 Purpose

As a cross-language AI-Perceivable module standard, apcore needs to ensure behavioral consistency among SDK implementations in various languages. This conformance specification defines three progressive conformance levels, with each level clearly listing **MUST**, **SHOULD**, and **may** implement components, as well as corresponding test requirements.

Implementers can choose their target conformance level based on their needs and verify conformance through the corresponding test suite.

### 1.2 Conformance Level Overview

| Level | Name | Positioning | Applicable Scenarios |
|------|------|------|---------|
| **Level 0** | Core | Minimal viable implementation | Rapid prototyping, embedded, resource-constrained environments |
| **Level 1** | Standard | Production-ready implementation | Most production environments |
| **Level 2** | Full | Full-featured implementation | Enterprise-grade, large-scale distributed scenarios |

### 1.3 Terminology and Keywords

The keywords used in this document follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) definitions, consistent with [PROTOCOL_SPEC §1.5](./protocol-spec.md).

---

## 2. Level 0 — Core Conformance

### 2.1 Overview

Level 0 defines the minimal viable implementation of apcore. SDKs reaching this level can complete module definition, registration, discovery, and basic execution, but do not include advanced features such as permission control, middleware, and observability.

### 2.2 Must Implement (MUST)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Module interface** | Module base class/interface, includes `execute()`, `input_schema`, `output_schema`, `description` | PROTOCOL_SPEC §5.6 |
| **Schema validation** | Input/output validation based on JSON Schema Draft 2020-12, supports `type`, `properties`, `required`, `enum`, `$ref` (local references) | PROTOCOL_SPEC §4.2 |
| **Registry** | Module discovery (directory scanning), registration, retrieval (`discover()`, `get()`, `list()`) | PROTOCOL_SPEC §12.2 |
| **Executor** | Module call execution, includes input validation, module location, execution, output validation | PROTOCOL_SPEC §12.2 |
| **Directory as ID** | `directory_to_canonical_id()` algorithm implementation, directory path automatically maps to Canonical ID | PROTOCOL_SPEC §2.1 |
| **ID format validation** | EBNF syntax validation for Canonical ID | PROTOCOL_SPEC §2.7 |
| **ID conflict detection** | `detect_id_conflicts()` algorithm, detects duplicate IDs and reserved word conflicts | PROTOCOL_SPEC §2.6 |
| **Basic error handling** | Unified error format (`code`, `message`), framework error codes (MODULE_*, SCHEMA_*, GENERAL_*) | PROTOCOL_SPEC §8.1, §8.2 |
| **Error propagation** | `propagate_error()` algorithm, module errors wrapped as ModuleError | PROTOCOL_SPEC §8.3 |
| **Configuration loading** | `apcore.yaml` basic configuration loading and validation | PROTOCOL_SPEC §9.1 |
| **Schema loading** | YAML Schema file loading and parsing | PROTOCOL_SPEC §4.9 |
| **Scanning algorithm** | `scan_extensions()` directory scanning algorithm | PROTOCOL_SPEC §3.6 |
| **Hidden file filtering** | Ignore hidden files and special directories when scanning | PROTOCOL_SPEC §3.5 |
| **Function-based module definition** | `module()` mechanism, wraps callable as standard module, auto-generates Schema from type annotations | PROTOCOL_SPEC §5.11 |
| **External Schema binding** | YAML binding file loading, target resolution, Schema validation | PROTOCOL_SPEC §5.12 |
| **Type inference Schema generation** | `generate_schema_from_function()` algorithm, generates JSON Schema from function signature | PROTOCOL_SPEC §5.11.4 |

### 2.3 Should Implement (SHOULD)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **ID Map cross-language conversion** | `normalize_to_canonical_id()` algorithm, supports at least two languages | PROTOCOL_SPEC §2.2 |
| **Entry point auto-inference** | `resolve_entry_point()` algorithm | PROTOCOL_SPEC §5.2 |
| **Dependency resolution** | `resolve_dependencies()` topological sort algorithm | PROTOCOL_SPEC §5.3 |
| **Schema version declaration** | `version` field support in Schema files | PROTOCOL_SPEC §4.11 |
| **Environment variable override** | `APCORE_*` environment variables override configuration | PROTOCOL_SPEC §9.2 |
| **Schema $ref cross-file references** | `resolve_ref()` algorithm, supports relative path references | PROTOCOL_SPEC §4.10 |
| **`additionalProperties` validation** | additionalProperties handling in input_schema | PROTOCOL_SPEC §4.2 |

### 2.4 May Implement (MAY)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Module metadata files** | `*_meta.yaml` metadata file loading | PROTOCOL_SPEC §5.2 |
| **Annotations** | Behavior annotations | PROTOCOL_SPEC §4.4 |
| **Examples** | Usage examples | PROTOCOL_SPEC §4.5 |
| **Metadata** | Extension metadata | PROTOCOL_SPEC §4.6 |
| **Version number handling** | Filename version suffix parsing | PROTOCOL_SPEC §2.4 |
| **LLM extension fields** | `x-llm-description`, `x-examples`, etc. | PROTOCOL_SPEC §4.3 |

### 2.5 Level 0 Required Test Categories

- **ID naming tests** (10+ cases)
- **Directory scanning tests** (5+ cases)
- **Schema validation tests** (15+ cases)
- **Module registration and execution tests** (10+ cases)
- **Error handling tests** (10+ cases)
- **Configuration loading tests** (5+ cases)
- **Function-based module definition tests** (10+ cases)
- **External Binding tests** (8+ cases)

---

## 3. Level 1 — Standard Conformance

### 3.1 Overview

Level 1 adds permission control, middleware, basic observability, and structured logging on top of Level 0. SDKs reaching this level meet the needs of most production environments.

### 3.2 Must Implement (MUST)

**All MUST components from Level 0**, plus:

| Component | Responsibility | Reference Section |
|------|------|---------|
| **ACL engine** | Permission rule loading, pattern matching (`match_pattern()`), rule evaluation (`evaluate_acl()`) | PROTOCOL_SPEC §6.2, §6.3 |
| **ACL default policy** | Support `default_effect: deny \| allow` | PROTOCOL_SPEC §6.1 |
| **ACL edge cases** | When caller_id is null, treat as `@external`, empty rules use default policy | PROTOCOL_SPEC §6.5 |
| **Middleware framework** | Onion model middleware chain, supports `before`, `after`, `on_error` hooks | PROTOCOL_SPEC §11.1 |
| **Middleware priority** | Higher numbers execute first | PROTOCOL_SPEC §11.2 |
| **Built-in middleware (required)** | `schema_validation` (priority 1000), `acl_check` (priority 999) | PROTOCOL_SPEC §11.4 |
| **Trace ID** | `trace_id` generation (32-char lowercase hex, W3C Trace Context), propagation (child calls inherit) | PROTOCOL_SPEC §10.4 |
| **Structured logging** | JSON format logs, includes `timestamp`, `level`, `message`, `trace_id`, `module_id` | PROTOCOL_SPEC §10.2 |
| **Context complete implementation** | `trace_id`, `caller_id`, `call_chain`, `executor`, `identity`, `data` | PROTOCOL_SPEC §5.7 |
| **Circular call detection** | Detect circular calls based on `call_chain` | PROTOCOL_SPEC §5.7 |
| **Call depth limit** | Limit maximum call depth based on `call_chain` | PROTOCOL_SPEC §5.7 |
| **Error hierarchy system** | Flat error hierarchy under `ModuleError` base class | PROTOCOL_SPEC §8.7 |
| **Custom error codes** | Module custom error code registration and collision detection | PROTOCOL_SPEC §8.4 |
| **ID Map all-language conversion** | Support ID conversion for all five languages | PROTOCOL_SPEC §2.2 |
| **Dependency resolution** | `resolve_dependencies()` topological sort and circular dependency detection | PROTOCOL_SPEC §5.3 |
| **Configuration validation algorithm** | `validate_config()` complete implementation | PROTOCOL_SPEC §9.3 |

### 3.3 Should Implement (SHOULD)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **ACL pattern specificity** | `calculate_specificity()` algorithm | PROTOCOL_SPEC §6.4 |
| **ACL audit logs** | Record permission check results, including denied calls | PROTOCOL_SPEC §6.1 |
| **Built-in middleware (default enabled)** | `tracing`, `logging`, `metrics`, `error_wrapper` | PROTOCOL_SPEC §11.4 |
| **Middleware disabling** | Disable default-enabled built-in middleware via configuration | PROTOCOL_SPEC §11.4 |
| **Sensitive data redaction** | `redact_sensitive()` algorithm, redact `x-sensitive` fields in logs | PROTOCOL_SPEC §10.5 |
| **Trace Span** | Span creation and ending, follow naming conventions | PROTOCOL_SPEC §10.7 |
| **Metrics collection** | Basic counter and histogram metrics | PROTOCOL_SPEC §10.3 |
| **Retry semantics** | Classify error code retryability and reject inappropriate retries | PROTOCOL_SPEC §8.6 |
| **Annotation conflict rules** | YAML takes precedence over code, code takes precedence over defaults | PROTOCOL_SPEC §4.12 |
| **Schema validation error format** | Structured validation errors (path, message, constraint, expected, actual) | PROTOCOL_SPEC §4.13 |

### 3.4 May Implement (MAY)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Middleware code registration** | Runtime dynamic middleware registration | PROTOCOL_SPEC §11.2 |
| **W3C Trace Context** | Distributed tracing standard compliance | PROTOCOL_SPEC §10.4 |
| **Sampling strategies** | Full/proportional/error-first sampling | PROTOCOL_SPEC §10.6 |
| **Module describe interface** | `describe()` method returns LLM-usable complete description | PROTOCOL_SPEC §5.6 |
| **Symlink handling** | Configurable symlink following and cycle detection | PROTOCOL_SPEC §3.4 |

### 3.5 Level 1 Required Test Categories

**All tests from Level 0**, plus:

- **ACL tests** (15+ cases)
- **Middleware tests** (10+ cases)
- **Observability tests** (10+ cases)
- **Context propagation tests** (5+ cases)
- **Error guidance tests** (3+ cases): Verify framework errors include `ai_guidance` field with actionable recovery hints

---

## 4. Level 2 — Full Conformance

### 4.1 Overview

Level 2 adds all extension points, async modules, hot loading, and advanced observability on top of Level 1. SDKs reaching this level are suitable for enterprise-grade, large-scale distributed scenarios.

### 4.2 Must Implement (MUST)

**All MUST components from Level 1**, plus:

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Extension point framework** | All five extension points (discoverer, middleware, acl, span_exporter, module_validator) via `ExtensionManager` with `register()`, `get()`, `get_all()`, `unregister()`, `apply()`, `list_points()`. Note: these names map to actual runtime extension needs rather than the original theoretical design names (SchemaLoader, ModuleLoader, IDConverter, ACLChecker, Executor). | PROTOCOL_SPEC §11.3, §11.6 |
| **Extension point chain** | `first_success`, `all`, `fallback` strategies | PROTOCOL_SPEC §11.3 |
| **Extension loading order** | `load_extensions()` algorithm | PROTOCOL_SPEC §11.7 |
| **Async modules** | `submit()`, `get_status()`, `cancel()`, `list_tasks()` via `AsyncTaskManager` | PROTOCOL_SPEC §5.8 |
| **Async state machine** | State transition rules (PENDING → RUNNING → COMPLETED/FAILED/CANCELLED) via `TaskStatus` enum | PROTOCOL_SPEC §5.8 |
| **Middleware state machine** | Complete state transitions (init → before → execute → after → done, with error branches) | PROTOCOL_SPEC §11.5 |
| **Version negotiation** | `negotiate_version()` algorithm | PROTOCOL_SPEC §13.3 |
| **Schema migration** | `migrate_schema()` algorithm | PROTOCOL_SPEC §13.4 |
| **Compatibility matrix** | Backward/forward compatibility rules | PROTOCOL_SPEC §13.5 |
| **Context serialization** | Cross-process Context JSON serialization/deserialization | PROTOCOL_SPEC §5.7 |
| **All framework built-in middleware** | schema_validation, acl_check, tracing, logging, metrics, error_wrapper | PROTOCOL_SPEC §11.4 |

### 4.3 Should Implement (SHOULD)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Module hot loading** | Runtime reload modules without restart | PROTOCOL_SPEC §12.5 Phase 4 |
| **OpenTelemetry integration** | Standard OTLP exporter | PROTOCOL_SPEC §10.1 |
| **Prometheus metrics export** | Standard metrics format | PROTOCOL_SPEC §10.3 |
| **Advanced sampling strategies** | Error-first sampling | PROTOCOL_SPEC §10.6 |
| **W3C Trace Context** | `traceparent` header propagation | PROTOCOL_SPEC §10.4 |
| **Module isolation** | Process-level or container-level isolation | PROTOCOL_SPEC §5.5 |
| **Multi-version coexistence** | Multiple versions of same module running | PROTOCOL_SPEC §5.4 |
| **Schema deprecation markers** | `x-deprecated` handling and warnings | PROTOCOL_SPEC §4.11 |
| **Async callbacks** | `on_progress`, `on_complete`, `on_error` callbacks | PROTOCOL_SPEC §5.8 |

### 4.4 May Implement (MAY)

| Component | Responsibility | Reference Section |
|------|------|---------|
| **Protocol adapters** | MCP / A2A / OpenAI / Anthropic / LangChain mapping | PROTOCOL_SPEC §D |
| **CLI tools** | `init`, `create`, `run` and other developer tools | PROTOCOL_SPEC §12.5 Phase 3 |
| **Schema code generation** | Generate language native types from YAML Schema | PROTOCOL_SPEC §4.9 |
| **Remote module loading** | Load modules from remote services/repositories | PROTOCOL_SPEC §11.3 |
| **Distributed execution** | Cross-process/cross-network module execution | PROTOCOL_SPEC §11.3 |
| **Container-level isolation** | Docker/Wasm sandbox | PROTOCOL_SPEC §5.5 |

### 4.5 Level 2 Required Test Categories

**All tests from Level 1**, plus:

- **Extension point tests** (10+ cases)
- **Async module tests** (10+ cases)
- **Version compatibility tests** (5+ cases)
- **Middleware state machine tests** (5+ cases)
- **Schema export round-trip tests** (3+ cases): Verify field preservation across export profiles (especially `default` values in non-strict exports)

---

## 5. Test Suite Design

### 5.1 Test Category Definitions

| Number | Category Name | English ID | Applicable Level | Minimum Cases |
|------|---------|---------|---------|-----------|
| T01 | Naming specification tests | `naming` | Level 0+ | 10 |
| T02 | Directory scanning tests | `directory` | Level 0+ | 5 |
| T03 | Schema validation tests | `schema` | Level 0+ | 15 |
| T04 | Module lifecycle tests | `module` | Level 0+ | 10 |
| T05 | ACL permission tests | `acl` | Level 1+ | 15 |
| T06 | Error handling tests | `error` | Level 0+ | 10 |
| T07 | Configuration loading tests | `config` | Level 0+ | 5 |
| T08 | Observability tests | `observability` | Level 1+ | 10 |
| T09 | Middleware tests | `middleware` | Level 1+ | 10 |
| T10 | Extension point tests | `extension` | Level 2 | 10 |
| T11 | Async module tests | `async` | Level 2 | 10 |
| T12 | Version compatibility tests | `versioning` | Level 2 | 5 |

### 5.2 Test Case Format

Each test case **MUST** contain the following structure:

```yaml
test_case:
  # Unique identifier, format: T{category_number}-{sequence}
  id: "T01-001"

  # Test name
  name: "Directory path conversion to Canonical ID - standard path"

  # Category
  category: "naming"

  # Minimum conformance level
  level: 0

  # Whether mandatory test
  mandatory: true

  # Preconditions
  preconditions:
    - "Extension root directory is 'extensions'"
    - "'extensions/executor/validator/db_params.py' exists in filesystem"

  # Execution steps
  steps:
    - action: "Call directory_to_canonical_id('extensions/executor/validator/db_params.py', 'extensions')"

  # Expected result
  expected:
    result: "executor.validator.db_params"
    error: null

  # Tags
  tags: [id, conversion, basic]
```

### 5.3 Detailed Test Cases by Category

#### T01: Naming Specification Tests (`naming`)

| Case ID | Name | Input | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T01-001 | Standard path conversion | `extensions/executor/validator/db_params.py` | `executor.validator.db_params` | Yes |
| T01-002 | Multi-level path conversion | `extensions/api/handler/task_submit.py` | `api.handler.task_submit` | Yes |
| T01-003 | Invalid path - empty segment | `extensions/executor//db_params.py` | INVALID_PATH error | Yes |
| T01-004 | Invalid path - uppercase letter | `extensions/Executor/Validator.py` | INVALID_SEGMENT error | Yes |
| T01-005 | Invalid path - digit prefix | `extensions/123module/test.py` | INVALID_SEGMENT error | Yes |
| T01-006 | ID too long (>192 chars) | Overlong path | ID_TOO_LONG error | Yes |
| T01-007 | Reserved word detection | `extensions/system/core.py` | reserved_word conflict | Yes |
| T01-008 | Case collision detection | Register `A.B` after `a.b` | case_collision warning | Yes |
| T01-009 | Duplicate ID detection | Register same ID twice | duplicate_id error | Yes |
| T01-010 | Cross-language ID conversion (Rust) | `executor::validator::db_params` | `executor.validator.db_params` | No |
| T01-011 | Cross-language ID conversion (PascalCase) | `DbParamsValidator` | `db_params_validator` | No |

#### T02: Directory Scanning Tests (`directory`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T02-001 | Standard directory scan | Normal directory structure | Discover all modules | Yes |
| T02-002 | Hidden file ignore | Contains `.git/`, `__pycache__/` | Does not include hidden directories | Yes |
| T02-003 | Maximum depth limit | Over 8 levels of nesting | Skip deep directories and warn | Yes |
| T02-004 | Empty directory | Extension directory exists but empty | Return empty list | Yes |
| T02-005 | Directory does not exist | Extension root directory missing | CONFIG_ERROR | Yes |

#### T03: Schema Validation Tests (`schema`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T03-001 | Valid input passes validation | Input fully conforms to Schema | Validation passes | Yes |
| T03-002 | Missing required field | Missing `required` field | SCHEMA_VALIDATION_ERROR | Yes |
| T03-003 | Type mismatch | Pass integer to string field | SCHEMA_VALIDATION_ERROR | Yes |
| T03-004 | Extra field rejection | Pass extra field when `additionalProperties: false` | SCHEMA_VALIDATION_ERROR | Yes |
| T03-005 | Enum value validation | Pass value not in enum | SCHEMA_VALIDATION_ERROR | Yes |
| T03-006 | Number range validation | `minimum`/`maximum` validation | Validation fails when out of range | Yes |
| T03-007 | String pattern validation | Regex constraint | Validation fails when not matching | Yes |
| T03-008 | Nested object validation | Nested object field validation | Deep error path correct | Yes |
| T03-009 | Array element validation | Array items type validation | Validation fails on element type error | Yes |
| T03-010 | Default value filling | Optional field uses default value | Default value filled correctly | Yes |
| T03-011 | $ref local reference | Same file `#/definitions/...` | Reference resolves correctly | Yes |
| T03-012 | $ref circular reference detection | Circular $ref | SCHEMA_CIRCULAR_REF error | No |
| T03-013 | $ref cross-file reference | Relative path reference | Reference resolves correctly | No |
| T03-014 | x-sensitive marker recognition | Field with `x-sensitive: true` | Marker correctly recognized | No |
| T03-015 | Output Schema validation | Module return value doesn't conform to output_schema | Validation fails | Yes |

#### T04: Module Lifecycle Tests (`module`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T04-001 | Module registration and retrieval | Retrieve by ID after registration | Successfully retrieve | Yes |
| T04-002 | Module list | List after registering multiple modules | Return all registered IDs | Yes |
| T04-003 | Module does not exist | Get unregistered module ID | MODULE_NOT_FOUND | Yes |
| T04-004 | Normal execution | Valid input, module executes normally | Return result conforming to output_schema | Yes |
| T04-005 | Input validation failure | Input doesn't conform to input_schema | SCHEMA_VALIDATION_ERROR | Yes |
| T04-006 | Module execution exception | Module throws exception internally | MODULE_EXECUTE_ERROR | Yes |
| T04-007 | Inter-module call | Call other module via context.executor | Successfully call and propagate context | Yes |
| T04-008 | Entry point inference | No meta.yaml, auto-infer entry point | Correctly find module class | No |
| T04-009 | Lifecycle hooks | on_load / on_unload | Hooks called correctly | No |
| T04-010 | Dependency ordering | Modules with dependencies | Load in topological order | No |

#### T05: ACL Permission Tests (`acl`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T05-001 | Exact match allow | caller and target exactly match allow rule | Allow | Yes |
| T05-002 | Exact match deny | caller and target exactly match deny rule | Deny | Yes |
| T05-003 | Wildcard `*` match | `api.*` matches `api.handler.task_submit` | Match succeeds | Yes |
| T05-004 | Global wildcard | `*` matches all IDs | Match succeeds | Yes |
| T05-005 | deny takes precedence over allow | Same priority deny and allow both match | deny takes effect | Yes |
| T05-006 | Priority ordering | High priority allow, low priority deny | allow takes effect | Yes |
| T05-007 | Default policy deny | No matching rule, default deny | Deny | Yes |
| T05-008 | Default policy allow | No matching rule, default allow | Allow | Yes |
| T05-009 | External call | caller_id is null | Treat as `@external` | Yes |
| T05-010 | Empty rule list | rules is empty | Use default policy | Yes |
| T05-011 | Empty callers/targets | Rule has empty callers array | Rule doesn't match | Yes |
| T05-012 | Middle wildcard | `*.validator.*` matches | Match succeeds | Yes |
| T05-013 | Self call | Module calls itself | Normal ACL check execution | Yes |
| T05-014 | Specificity scoring | Exact match vs wildcard | Score correct | No |
| T05-015 | ACL audit log | ACL check triggers audit | Log includes check result | No |

#### T06: Error Handling Tests (`error`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T06-001 | Unified error format | Any framework error | Contains code, message | Yes |
| T06-002 | trace_id attached | Error includes trace_id | Yes | Yes |
| T06-003 | Error propagation | Module exception wrapped | ModuleError format | Yes |
| T06-004 | Error chain | Nested call error | cause field preserves original error | Yes |
| T06-005 | Framework error codes not overridable | Module defines MODULE_* error code | Collision detection error | Yes |
| T06-006 | Error type mapping | SchemaValidationError → SCHEMA_VALIDATION_ERROR | Mapping correct | Yes |
| T06-007 | ACLDeniedError mapping | ACL denial → ACL_DENIED | Mapping correct | Yes |
| T06-008 | TimeoutError mapping | Timeout → MODULE_TIMEOUT | Mapping correct | Yes |
| T06-009 | Custom error code registration | Module registers custom error code | Registration succeeds | No |
| T06-010 | Error code collision detection | Two modules register same error code | Collision error | No |

#### T07: Configuration Loading Tests (`config`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T07-001 | Valid configuration loading | Complete apcore.yaml | Load succeeds | Yes |
| T07-002 | Missing required field | Missing `extensions.root` | CONFIG_INVALID | Yes |
| T07-003 | Type error | `max_depth: "abc"` | CONFIG_INVALID | Yes |
| T07-004 | Environment variable override | Set APCORE_EXTENSIONS_ROOT | Environment variable takes precedence | No |
| T07-005 | Default value fallback | Not specify `auto_discover` in config | Use default value true | Yes |

#### T08: Observability Tests (`observability`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T08-001 | trace_id generation | Top-level call | Auto-generate 32-char lowercase hex | Yes |
| T08-002 | trace_id propagation | Child call | Inherit parent call's trace_id | Yes |
| T08-003 | Structured logging | Module execution log | JSON format, contains necessary fields | Yes |
| T08-004 | Log level | Configure level: warn | Don't output info level logs | Yes |
| T08-005 | Sensitive data redaction | x-sensitive field | Log shows `***REDACTED***` | No |
| T08-006 | Nested object redaction | x-sensitive in nested object | Recursively redact | No |
| T08-007 | Span creation | Module execution | Create `apcore.module.execute` Span | No |
| T08-008 | Span attributes | Span ends | Includes module_id, duration_ms, success | No |
| T08-009 | Metrics counter | After module execution | `apcore_module_calls_total` increments | No |
| T08-010 | Metrics histogram | After module execution | `apcore_module_duration_seconds` records | No |

#### T09: Middleware Tests (`middleware`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T09-001 | Onion model order | Multiple middleware | before in priority descending, after in ascending | Yes |
| T09-002 | before modifies input | Middleware modifies inputs | Module receives modified input | Yes |
| T09-003 | after modifies output | Middleware modifies output | Caller receives modified output | Yes |
| T09-004 | before abort | Middleware throws exception | Skip subsequent before and execute | Yes |
| T09-005 | on_error fallback | Error handling returns fallback result | Stop error propagation | Yes |
| T09-006 | Priority ordering | Different priority middleware | High priority executes first | Yes |
| T09-007 | Non-disableable middleware | Try to disable schema_validation | Disable fails/invalid | Yes |
| T09-008 | Disableable middleware | Disable metrics | metrics doesn't execute | No |
| T09-009 | Runtime registration | Register middleware via code | Registration succeeds and executes | No |
| T09-010 | Middleware state machine | Error branch transition | on_error triggers correctly | No |

#### T10: Extension Point Tests (`extension`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T10-001 | Custom SchemaLoader | Replace default Schema loader | Custom loader takes effect | Yes |
| T10-002 | Custom ModuleLoader | Replace default module loader | Custom loader takes effect | Yes |
| T10-003 | Custom ACLChecker | Replace default ACL checker | Custom checker takes effect | Yes |
| T10-004 | Extension chain first_success | Multiple loaders, first succeeds | Use first successful result | Yes |
| T10-005 | Extension chain fallback | First fails, second succeeds | Fall back to second implementation | Yes |
| T10-006 | No available implementation | Extension point has no registered implementation | Use framework default implementation | Yes |
| T10-007 | Extension priority | Multiple implementations different priorities | High priority tries first | Yes |
| T10-008 | Configuration-registered extension | Register via apcore.yaml | Registration succeeds | No |
| T10-009 | Code-registered extension | Runtime code registration | Registration succeeds | No |
| T10-010 | Custom Executor | Replace execution method | Custom executor takes effect | No |

#### T11: Async Module Tests (`async`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T11-001 | Async execution start | Call call_async() on an async module | Return task_id and pending status | Yes |
| T11-002 | Status query | Query running task | Return running and progress | Yes |
| T11-003 | Success completion | Task completes normally | Status is completed, contains result | Yes |
| T11-004 | Execution failure | Task execution exception | Status is failed, contains error | Yes |
| T11-005 | Cancel task | Call cancel() | Status is cancelled | Yes |
| T11-006 | Forbidden state transition | Try to transition after completed | Reject transition | Yes |
| T11-007 | Skip pending | idle directly to running | Reject transition | Yes |
| T11-008 | Progress callback | Set on_progress callback | Callback triggered | No |
| T11-009 | Completion callback | Set on_complete callback | Callback triggered | No |
| T11-010 | Retry | Resubmit failed status | Transition to pending | No |

#### T12: Version Compatibility Tests (`versioning`)

| Case ID | Name | Scenario | Expected Result | Mandatory |
|---------|------|------|---------|------|
| T12-001 | Same major version compatible | declared: 1.0.0, sdk: 1.3.0 | Compatible, use 1.0.0 | Yes |
| T12-002 | Different major version incompatible | declared: 2.0.0, sdk: 1.3.0 | VERSION_INCOMPATIBLE | Yes |
| T12-003 | SDK version too low | declared: 1.5.0, sdk: 1.3.0 | VERSION_INCOMPATIBLE | Yes |
| T12-004 | Deprecation warning | declared: 1.0.0, sdk: 1.5.0 | DEPRECATION_WARNING | No |
| T12-005 | Schema migration | Migrate from 1.0 to 1.2 | Migration succeeds | No |

### 5.4 Passing Standards

#### Level 0 Passing Standard

| Standard | Requirement |
|------|------|
| Mandatory test pass rate | **100%** (all tests with `mandatory: true` **MUST** pass) |
| Overall test pass rate | **>=80%** (including non-mandatory tests) |
| Coverage categories | All mandatory tests from T01, T02, T03, T04, T06, T07 |

#### Level 1 Passing Standard

| Standard | Requirement |
|------|------|
| Mandatory test pass rate | **100%** |
| Overall test pass rate | **>=85%** |
| Coverage categories | Level 0 categories + all mandatory tests from T05, T08, T09 |

#### Level 2 Passing Standard

| Standard | Requirement |
|------|------|
| Mandatory test pass rate | **100%** |
| Overall test pass rate | **>=90%** |
| Coverage categories | Level 1 categories + all mandatory tests from T10, T11, T12 |

---

## 6. Conformance Declaration

### 6.1 Declaration Format

When declaring conformance, implementers **MUST** include the following structured declaration in the project root or documentation:

```yaml
# apcore-conformance.yaml

# Implementation information
implementation:
  name: "apcore-python"              # Implementation name
  version: "0.1.0"                   # Implementation version
  language: "python"                 # Implementation language
  spec_version: "1.2.0-draft"       # Corresponding PROTOCOL_SPEC version
  maintainer: "AI Perceivable"        # Maintainer
  repository: "https://github.com/aiperceivable/apcore-python"  # Repository address

# Conformance declaration
conformance:
  level: 1                           # Declared conformance level (0 | 1 | 2)
  date: "2026-02-07"                 # Declaration date

  # Test results summary
  test_results:
    total: 120                       # Total tests
    passed: 115                      # Passed
    failed: 3                        # Failed
    skipped: 2                       # Skipped
    mandatory_passed: 85             # Mandatory tests passed
    mandatory_total: 85              # Mandatory tests total

  # Category results
  categories:
    naming:
      passed: 11
      total: 11
    directory:
      passed: 5
      total: 5
    schema:
      passed: 14
      total: 15
    module:
      passed: 10
      total: 10
    acl:
      passed: 15
      total: 15
    error:
      passed: 10
      total: 10
    config:
      passed: 5
      total: 5
    observability:
      passed: 9
      total: 10
    middleware:
      passed: 10
      total: 10

  # Known deviations (if any)
  # Format: - { id: "T03-xxx", reason: "...", severity: "minor|major" }
  known_deviations: []

  # Optional feature support
  optional_features:
    - name: "hot_reload"
      supported: false
    - name: "opentelemetry"
      supported: true
    - name: "distributed_execution"
      supported: false
```

### 6.2 Declaration Rules

| Rule | Level | Description |
|------|------|------|
| Declared level **MUST** be consistent with test results | **MUST** | Cannot falsely claim conformance level |
| Mandatory tests **MUST** all pass | **MUST** | Cannot claim that level if any mandatory test fails |
| Known deviations **MUST** be listed honestly | **MUST** | — |
| Declaration **SHOULD** be accompanied by reproducible test results | **SHOULD** | Such as CI report link |
| Declaration **SHOULD** be regularly updated | **SHOULD** | At least once per minor version |

### 6.3 Conformance Badges

Implementations that pass conformance verification **may** use the following badges in documentation:

```
apcore Conformant — Level 0 (Core)
apcore Conformant — Level 1 (Standard)
apcore Conformant — Level 2 (Full)
```

---

## 7. Known Deviations

The following features are specified in PROTOCOL_SPEC but not yet fully implemented in current SDK releases (apcore-python, apcore-typescript). Implementers **SHOULD** document these deviations in their conformance declarations.

| Feature | Spec Reference | Current Status |
|---------|---------------|----------------|
| `Config` class (YAML loading, env override, schema validation) | PROTOCOL_SPEC §9.1, §9.2, §9.3 | Stub implementation only. YAML loading, environment variable override, and `validate_config()` schema validation are not implemented. |
| Registry schema query/export methods (`get_schema`, `export_schema`, etc.) | PROTOCOL_SPEC §12.2 | Not on `Registry`. A standalone `SchemaExporter` class is available for schema export. |
| Error codes `GENERAL_NOT_IMPLEMENTED` and `DEPENDENCY_NOT_FOUND` | PROTOCOL_SPEC §8.2, §8.7 | Implemented in both SDKs as `FeatureNotImplementedError` and `DependencyNotFoundError`. |
| Version negotiation | PROTOCOL_SPEC §13.3 | `negotiate_version()` algorithm not yet implemented. |
| Schema migration | PROTOCOL_SPEC §13.4 | `migrate_schema()` algorithm not yet implemented. |
| Module isolation | PROTOCOL_SPEC §5.5 | Process-level or container-level isolation not yet implemented. |
| Multi-version coexistence | PROTOCOL_SPEC §5.4 | Multiple versions of the same module running concurrently not yet implemented. |
| `AsyncTaskManager.submit()` / `cancel()` sync vs async | PROTOCOL_SPEC §5.8 | Python `AsyncTaskManager.submit()` and `cancel()` are async methods; TypeScript equivalents are synchronous. |

Implementations declaring conformance **MUST** list any of these deviations that apply in their `known_deviations` section.

---

## 8. Conformance Test Fixtures

The repository ships **43 cross-language fixture files** under `conformance/fixtures/` covering **370 test cases**. Each fixture is a JSON document of shape `{ "description": "...", "test_cases": [...] }` consumed by all three SDK test runners (apcore-python, apcore-typescript, apcore-rust). A SDK declaring a conformance level **MUST** pass every fixture whose tested feature is required at that level (see §2 Level 0, §3 Level 1, §4 Level 2 for the per-feature breakdown).

### 8.1 Fixture Inventory

| Fixture | Cases | Tested feature |
|---------|------:|----------------|
| [`acl_evaluation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/acl_evaluation.json) | 21 | ACL rule evaluation, first-match-wins (spec §6) |
| [`annotations_extra_round_trip`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/annotations_extra_round_trip.json) | 8 | `ModuleAnnotations.extra` wire-format round-trip (spec §4.4) |
| [`approval_gate`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/approval_gate.json) | 5 | Approval gate enforcement at Executor Step 5 |
| [`async_task_evolution`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/async_task_evolution.json) | 10 | Pluggable `TaskStore`, retry with backoff |
| [`binding_errors`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/binding_errors.json) | 6 | Binding error message conformance (DECLARATIVE_CONFIG_SPEC §7.2) |
| [`call_chain`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/call_chain.json) | 9 | Call-chain safety guard (Algorithm A20) |
| [`config_defaults`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/config_defaults.json) | 18 | Config default values cross-SDK identity |
| [`config_env`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/config_env.json) | 13 | Env-var → Config path mapping (Algorithm A12-NS, spec §9.8) |
| [`context_serialization`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/context_serialization.json) | 8 | Context JSON round-trip (spec §5.7) |
| [`context_trace_parent`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/context_trace_parent.json) | 10 | `Context.create` trace_parent input handling |
| [`contextual_audit`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/contextual_audit.json) | 7 | Contextual audit trail for control-plane modules (#45.2) |
| [`dependency_version_constraints`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/dependency_version_constraints.json) | 15 | Dependency version constraint enforcement (spec §5) |
| [`error_codes`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/error_codes.json) | 6 | Error code collision detection (Algorithm A17, spec §8.4) |
| [`error_fingerprinting`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/error_fingerprinting.json) | 5 | Error fingerprint composition for ErrorHistory dedup |
| [`event_management_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/event_management_hardening.json) | 10 | SubscriberFactory parity, built-in subscribers |
| [`event_naming`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/event_naming.json) | 8 | Event-name canonicalization (Issue #36 / D-34) |
| [`identity_system`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/identity_system.json) | 8 | Identity construction, propagation (AC-014, AC-015) |
| [`middleware_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/middleware_hardening.json) | 10 | Context namespacing, CircuitBreaker |
| [`middleware_on_error_recovery`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/middleware_on_error_recovery.json) | 4 | Middleware after-chain error recovery |
| [`multi_module_discovery`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/multi_module_discovery.json) | 8 | Multi-class discovery, snake_case conversion, conflict detection |
| [`normalize_id`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/normalize_id.json) | 16 | ID normalization (Algorithm A02) |
| [`observability_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/observability_hardening.json) | 10 | Pluggable storage, BatchSpan, OTel parity |
| [`overrides_store`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/overrides_store.json) | 5 | OverridesStore pluggable persistence |
| [`pattern_matching`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pattern_matching.json) | 12 | ACL pattern matching (Algorithm A09) |
| [`pipeline_failfast_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_failfast_config.json) | 4 | Pipeline configuration fail-fast (Issue #33) |
| [`pipeline_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_hardening.json) | 5 | Pipeline execution hardening: fail-fast, replace-step, run_until |
| [`pipeline_step_middleware`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_step_middleware.json) | 6 | Pipeline StepMiddleware lifecycle (Issue #33) |
| [`redaction_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/redaction_config.json) | 4 | Redaction config via `obs.redaction.regex_patterns` / `sensitive_keys` |
| [`reload_path_filter`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/reload_path_filter.json) | 4 | Granular reload via `path_filter` glob (`system.control.reload_module`) |
| [`schema_hardening_cache`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_cache.json) | 5 | Content-addressable schema cache (SHA-256 of canonical JSON) |
| [`schema_hardening_constraints`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_constraints.json) | 12 | Numerical constraints (minimum/maximum/exclusive*, multipleOf) |
| [`schema_hardening_formats`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_formats.json) | 9 | Semantic format validation (date-time, date, email, uri, uuid, etc.) |
| [`schema_hardening_recursive`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_recursive.json) | 6 | Recursive schema support (`$ref` self-reference, depth 1–5) |
| [`schema_hardening_union`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_hardening_union.json) | 8 | Union type evaluation (anyOf / oneOf / allOf) |
| [`schema_validation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/schema_validation.json) | 16 | Schema validation edge cases (spec §4.15) |
| [`sensitive_keys_default`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/sensitive_keys_default.json) | 4 | Canonical default `obs.redaction.sensitive_keys` list (D-54) |
| [`specificity`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/specificity.json) | 10 | ACL pattern specificity scoring (Algorithm A10) |
| [`storage_backend`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/storage_backend.json) | 5 | StorageBackend pluggable persistence (shared by ErrorHistory) |
| [`stream_aggregation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/stream_aggregation.json) | 9 | Stream chunk aggregation (recursive deep merge) |
| [`system_modules_hardening`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/system_modules_hardening.json) | 10 | System modules hardening: persistence, audit, Prometheus |
| [`trace_context`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/trace_context.json) | 8 | W3C TraceContext alignment (Issue #35) |
| [`usage_exporter`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/usage_exporter.json) | 3 | `UsageExporter` push interface (#45 §3, D-55) |
| [`version_negotiation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/version_negotiation.json) | 10 | Version negotiation (Algorithm A14) |
| **Total** | **370** | **43 fixtures** |

### 8.2 Loading Fixtures from a Test Runner

=== "Python"

    ```python title="apcore-python — pytest example"
    import json
    import pathlib

    FIXTURES = pathlib.Path("conformance/fixtures")


    def load_fixture(name: str) -> dict:
        return json.loads((FIXTURES / f"{name}.json").read_text())


    def test_acl_evaluation():
        fixture = load_fixture("acl_evaluation")
        for case in fixture["test_cases"]:
            # ... evaluate `case` against your ACL implementation,
            # asserting the case's `expected` outcome
            ...
    ```

=== "TypeScript"

    ```typescript title="apcore-js — vitest example"
    import * as fs from 'node:fs';
    import * as path from 'node:path';
    import { test } from 'vitest';

    const FIXTURES = 'conformance/fixtures';

    interface Fixture {
      description: string;
      test_cases: Array<Record<string, unknown>>;
    }

    function loadFixture(name: string): Fixture {
      return JSON.parse(
        fs.readFileSync(path.join(FIXTURES, `${name}.json`), 'utf-8'),
      );
    }

    test('acl_evaluation', () => {
      const fixture = loadFixture('acl_evaluation');
      for (const c of fixture.test_cases) {
        // ... evaluate `c` against your ACL implementation,
        // asserting the case's `expected` outcome
      }
    });
    ```

=== "Rust"

    ```rust title="apcore-rust — cargo test example"
    use std::fs;
    use std::path::PathBuf;

    use serde_json::Value;

    fn load_fixture(name: &str) -> Value {
        let path: PathBuf = ["conformance", "fixtures", &format!("{}.json", name)]
            .iter()
            .collect();
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn acl_evaluation() {
        let fixture = load_fixture("acl_evaluation");
        for case in fixture["test_cases"].as_array().unwrap() {
            // ... evaluate `case` against your ACL implementation,
            // asserting the case's `expected` outcome
            let _ = case;
        }
    }
    ```

### 8.3 Adding a New Fixture

1. Create `conformance/fixtures/<name>.json` with `{ "description": "...", "test_cases": [...] }`.
2. Each case **MUST** carry a stable `id`, the input fields, and the `expected` outcome (`{ "expected": ... }` or `{ "expected_error": "<CODE>" }`).
3. Use canonical terminology: `caller_id` / `target_id` (never bare `caller` / `target`).
4. Add a row to the table in §8.1 above; bump the total.
5. Reference the fixture from the relevant spec section so the bidirectional traceability is maintained.

---

## 9. References

- [PROTOCOL_SPEC §12 — SDK Implementation Guide](./protocol-spec.md#12-sdk-implementation-guide)
- [PROTOCOL_SPEC §12.4 — Consistency Testing Requirements](./protocol-spec.md#124-consistency-testing-requirements)
- [PROTOCOL_SPEC §12.5 — Implementation Roadmap](./protocol-spec.md#125-implementation-roadmap)
- [PROTOCOL_SPEC §1.5 — Specification Keywords](./protocol-spec.md#15-specification-keywords)
- [RFC 2119 — Key words for use in RFCs to Indicate Requirement Levels](https://www.rfc-editor.org/rfc/rfc2119)
