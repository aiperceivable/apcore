# apcore — Core Algorithm Reference

> This document summarizes all pseudocode algorithms defined in the PROTOCOL_SPEC, providing a unified reference index.

## 1. Overview

### 1.1 Purpose

The apcore specification defines multiple algorithms that must or should be implemented across various chapters. This document consolidates these algorithms to provide SDK implementers with a unified algorithm reference, including input/output types, pre/post-conditions, pseudocode, complexity analysis, and implementation notes.

### 1.2 Algorithm Index

| No. | Algorithm Name | Description | Source Section | Implementation Requirement |
|-----|---------------|-------------|----------------|---------------------------|
| A01 | `directory_to_canonical_id()` | Directory path to Canonical ID | §2.1 | **MUST** |
| A02 | `normalize_to_canonical_id()` | Cross-language ID normalization | §2.2 | **MUST** |
| A03 | `detect_id_conflicts()` | ID conflict detection | §2.6 | **MUST** |
| A04 | `scan_extensions()` | Extension directory scanning | §3.6 | **MUST** |
| A05 | `resolve_ref()` | Schema $ref reference resolution | §4.10 | **MUST** |
| A06 | `resolve_entry_point()` | Module entry point resolution | §5.2 | **MUST** |
| A07 | `resolve_dependencies()` | Dependency topological sort | §5.3 | **MUST** |
| A08 | `match_pattern()` | ACL pattern matching | §6.2 | **MUST** |
| A09 | `evaluate_acl()` | ACL rule evaluation | §6.3 | **MUST** |
| A10 | `calculate_specificity()` | Pattern specificity scoring | §6.4 | **SHOULD** |
| A11 | `propagate_error()` | Error propagation | §8.3 | **MUST** |
| A12 | `validate_config()` | Configuration validation | §9.3 | **MUST** |
| A13 | `redact_sensitive()` | Sensitive data redaction | §10.5 | **MUST** |
| A14 | `negotiate_version()` | Version negotiation | §13.3 | **MUST** |
| A15 | `migrate_schema()` | Schema migration | §13.4 | **SHOULD** |
| A16 | `load_extensions()` | Extension loading | §11.7 | **MUST** |
| A17 | `detect_error_code_collisions()` | Error code collision detection | §8.4 | **MUST** |
| A18 | `generate_schema_from_function()` | Generate JSON Schema from function signature | §5.11.4 | **SHOULD** |
| A19 | `resolve_target()` | Resolve binding target for function-based modules | §5.12.3 | **SHOULD** |
| A20 | `guard_call_chain()` | Call chain safety check | §Executor | **MUST** |
| A21 | `safe_unregister()` | Hot-reload safe unregistration | §12.7.4 | **MUST** |
| A22 | `enforce_timeout()` | Timeout enforcement | §12.7.5 | **MUST** |
| A23 | `to_strict_schema()` | Strict Mode Schema conversion | §4.16 | **SHOULD** |

### 1.3 Conventions

- Pseudocode uses Python-like syntax but is not bound to any specific language
- `←` denotes assignment
- `∈` denotes set membership
- `∪` denotes set union
- `→` denotes function return
- All string comparisons are case-sensitive by default unless otherwise stated

---

## 2. Naming Algorithms

### A01: `directory_to_canonical_id()` — Directory Path to Canonical ID

**Source**: PROTOCOL_SPEC §2.1

**Description**: Converts the relative path of a module file to a dot-separated snake_case Canonical ID. This is the foundational implementation of apcore's "directory as ID" core concept.

**Input Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `file_path` | `String` | Full relative path of the module file (e.g., `"extensions/executor/validator/db_params.py"`) |
| `extensions_root` | `String` | Extension root directory name (default `"extensions"`) |
| `namespace` | `String \| null` | Namespace prefix (used for ID isolation in multi-root mode, default `null`) |

**Output:**

| Return Value | Type | Description |
|--------------|------|-------------|
| `canonical_id` | `String` | Dot-separated module ID (e.g., `"executor.validator.db_params"`, or `"core.executor.validator.db_params"` if namespace is `"core"`) |

**Preconditions:**

- `file_path` must start with `extensions_root + "/"`
- `file_path` must contain a file extension

**Postconditions:**

- The returned `canonical_id` conforms to the EBNF grammar (§2.7)
- `canonical_id` length does not exceed 192 characters

**Pseudocode:**

```
Algorithm: directory_to_canonical_id(file_path, extensions_root, namespace)

Steps:
  1. relative_path ← remove extensions_root + "/" prefix from file_path
  2. relative_path ← remove file extension (last "." and everything after it)
  3. segments ← split relative_path by "/"
  4. For each segment, perform validation:
     a. If segment is empty string → throw INVALID_PATH error
     b. If segment does not match /^[a-z][a-z0-9_]*$/ → throw INVALID_SEGMENT error
  5. canonical_id ← join all segments with "."
  6. If namespace is not null and not empty:
     a. Validate namespace matches /^[a-z][a-z0-9_]*$/
     b. canonical_id ← namespace + "." + canonical_id
  7. If len(canonical_id) > 192 → throw ID_TOO_LONG error
  8. Return canonical_id
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|-----------|-----------|-------------|
| Time | O(n) | n is the number of characters in the path |
| Space | O(n) | Storage for split segments and result string |

**Implementation Notes:**

- File extension removal should be based on the **last** `.` (e.g., `my.module.py` becomes `my.module` after removing `.py`)
- Path separators must uniformly use `/`; Windows systems need to preprocess `\` to `/`
- The regex `^[a-z][a-z0-9_]*$` implicitly prohibits double-underscore prefixes and digit prefixes
- The constraint against double underscores requires additional checking (EBNF §2.7 note 4)
- The `namespace` parameter is used for ID isolation in multi-root mode; pass `null` in single-root mode
- In multi-root mode, if `namespace` is not explicitly configured, the root directory name is used by default (e.g., `./plugins` → `namespace = "plugins"`)

---

### A02: `normalize_to_canonical_id()` — Cross-language ID Normalization

**Source**: PROTOCOL_SPEC §2.2

**Description**: Converts module IDs from various programming language local formats to a unified Canonical ID format. Supports five languages: Python, Rust, Go, Java, and TypeScript.

**Input Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `local_id` | `String` | Language-local format ID (e.g., Rust's `"executor::validator::db_params"`) |
| `language` | `String` | Source language identifier (`python` \| `rust` \| `go` \| `java` \| `typescript`) |

**Output:**

| Return Value | Type | Description |
|--------------|------|-------------|
| `canonical_id` | `String` | Dot-separated snake_case Canonical ID |

**Preconditions:**

- `language` must be one of the five supported languages

**Postconditions:**

- The return value conforms to the Canonical ID EBNF grammar

**Pseudocode:**

```
Algorithm: normalize_to_canonical_id(local_id, language)

Steps:
  1. Determine separator sep based on language:
     - python: "."  |  rust: "::"  |  go: "."  |  java: "."  |  typescript: "."
  2. segments ← split local_id by sep
  3. For each segment, perform case normalization:
     - If segment is PascalCase → convert to snake_case
     - If segment is camelCase → convert to snake_case
     - If segment is already snake_case → keep unchanged
  4. canonical_id ← join all segments with "."
  5. Validate canonical_id conforms to ID EBNF grammar (see §2.7)
  6. Return canonical_id
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|-----------|-----------|-------------|
| Time | O(n) | n is the number of characters in the input ID |
| Space | O(n) | Storage for conversion result |

**Implementation Notes:**

- PascalCase → snake_case conversion needs to handle acronyms (see PROTOCOL_SPEC §2.3)
- Acronyms are treated as regular words: `HttpJsonParser` → `http_json_parser` (not `h_t_t_p_j_s_o_n_parser`)
- Common acronym list: `http`, `api`, `db`, `id`, `url`, `sql`, `json`, `xml`, `html`, `css`, `tcp`, `udp`, `ip`

---

### A03: `detect_id_conflicts()` — ID Conflict Detection

**Source**: PROTOCOL_SPEC §2.6

**Description**: Detects whether a new ID conflicts with existing IDs or reserved words during module registration. Implementations **MUST** execute this algorithm during module scanning, module registration, and dynamic loading.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `new_id` | `String` | Canonical ID to be registered |
| `existing_ids` | `Set<String>` | Set of already registered IDs |
| `reserved_words` | `Set<String>` | Set of reserved words (see §2.5) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `conflict_result` | `Object \| null` | `{ type: string, severity: "error" \| "warning", message: string }` or `null` (no conflict) |

**Preconditions:**

- `new_id` has passed EBNF format validation

**Postconditions:**

- If returns `null`, then `new_id` can be safely registered
- If returns a result with `severity: "error"`, registration **MUST** be aborted
- If returns a result with `severity: "warning"`, registration can continue but **MUST** output a warning

**Pseudocode:**

```
Algorithm: detect_id_conflicts(new_id, existing_ids, reserved_words)

Steps:
  1. Exact duplicate detection:
     If new_id ∈ existing_ids → return { type: "duplicate_id", severity: "error" }
  2. Reserved word detection:
     For each segment of new_id (split by "."):
       If segment ∈ reserved_words → return { type: "reserved_word", severity: "error" }
  3. Case collision detection:
     normalized_new ← lowercase(new_id)
     For each existing_id ∈ existing_ids:
       normalized_existing ← lowercase(existing_id)
       If normalized_new == normalized_existing and new_id ≠ existing_id:
         → return { type: "case_collision", severity: "warning" }
  4. Return null (no conflict)
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is the number of registered IDs (case collision detection requires traversing all) |
| Space | O(1) | Only constant extra space (excluding input) |

**Implementation Notes:**

- Exact duplicate detection can use HashSet, O(1) lookup
- Case collision detection can maintain a lowercase → original_id mapping in advance, optimizing lookup to O(1)
- Reserved word set see §2.5: `system`, `internal`, `core`, `apcore`, `plugin`, `schema`, `acl` and keywords from various languages

---

## 3. Directory Scanning Algorithms

### A04: `scan_extensions()` — Extension Directory Scanning

**Source**: PROTOCOL_SPEC §3.6

**Description**: Recursively scans the extension root directory, discovers all module files and generates Canonical IDs. This is the core implementation of Registry's `discover()` method. Supports both single-root and multi-root directory modes.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `extensions_roots` | `List<Object>` | List of extension root directories, each item contains `{ root: String, namespace: String \| null }`. Single-root mode uses a single-element list |
| `config` | `Object` | Scanning configuration, includes `follow_symlinks`, `ignore_patterns`, `max_depth` |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `modules` | `List<(String, String)>` | List of `(file_path, canonical_id)` tuples |

**Preconditions:**

- Each `root` in `extensions_roots` exists and is a directory

**Postconditions:**

- All returned `canonical_id`s pass format validation and conflict detection
- Does not include hidden files and ignored patterns
- In multi-root mode, modules from different roots are isolated by namespace and do not cause ID conflicts

**Pseudocode:**

```
Algorithm: scan_extensions(extensions_roots, config)

Steps:
  1. modules ← []
  2. For each root_entry ∈ extensions_roots:
     a. extensions_root ← root_entry.root
     b. namespace ← root_entry.namespace
     c. If namespace is null and len(extensions_roots) > 1:
        - namespace ← extract directory name from extensions_root path (e.g., "./plugins" → "plugins")
     d. If extensions_root does not exist or is not a directory → throw CONFIG_ERROR
     e. Recursively traverse extensions_root:
        For each entry (file or directory):
          i.   If entry name matches ignore_patterns (§3.5) → skip
          ii.  If entry is a symlink and config.follow_symlinks == false → skip
          iii. If entry is a directory:
               - If current depth >= config.max_depth (default 8) → skip and issue warning
               - Otherwise → recurse into it
          iv.  If entry is a file and extension belongs to supported_extensions:
               - canonical_id ← directory_to_canonical_id(entry.path, extensions_root, namespace)
               - If canonical_id passes validation → append (entry.path, canonical_id) to modules
               - If validation fails → log warning
  3. Execute detect_id_conflicts batch detection on modules
  4. Return modules
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is the total number of filesystem entries (files + directories) |
| Space | O(m + d) | m is the number of modules (result list), d is maximum recursion depth (call stack) |

**Implementation Notes:**

- Ignore patterns (§3.5): starting with `.`, starting with `_`, `__pycache__/`, `node_modules/`, `*.pyc`
- `supported_extensions` depends on language: `.py`, `.rs`, `.go`, `.java`, `.ts`/`.tsx`
- Default depth limit is 8, maximum configurable to 16
- Symlink following is disabled by default, when enabled **MUST** detect cycles
- Batch conflict detection should be executed uniformly after module discovery from **all root directories** to ensure cross-root ID uniqueness
- In multi-root mode, if `namespace` is not explicitly configured, it is automatically derived from the directory name (e.g., `"./plugins"` → `"plugins"`)
- In single-root mode (`extensions_roots` has only one element), `namespace` defaults to `null`, maintaining backward compatibility

---

## 4. Schema-Related Algorithms

### A05: `resolve_ref()` — Schema $ref Reference Resolution

**Source**: PROTOCOL_SPEC §4.10

**Description**: Resolves `$ref` references in JSON Schema, supporting local references, cross-file references, and Canonical ID references. **MUST** detect and reject circular references.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `ref_string` | `String` | `$ref` value (e.g., `"./common/error.schema.yaml#/definitions/ErrorDetail"`) |
| `current_file` | `String` | Current Schema file path |
| `schemas_dir` | `String` | schemas root directory |
| `visited_refs` | `Set<String>` | Set of visited refs (for cycle detection) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `resolved_schema` | `Object` | Resolved Schema object |

**Preconditions:**

- `ref_string` is not empty

**Postconditions:**

- The returned Schema object does not contain unresolved `$ref`s (or has reached maximum depth limit)
- No circular references exist

**Pseudocode:**

```
Algorithm: resolve_ref(ref_string, current_file, schemas_dir, visited_refs)

Steps:
  1. If ref_string ∈ visited_refs → throw SCHEMA_CIRCULAR_REF error
  2. visited_refs ← visited_refs ∪ {ref_string}
  3. Parse ref_string into (file_part, json_pointer):
     a. If starts with "#" → file_part = current_file, json_pointer = ref_string[1:]
     b. If contains "#" → split by "#" into file_part and json_pointer
     c. If starts with "apcore://" → convert to file path under schemas_dir
     d. Otherwise → file_part is relative to current_file's directory
  4. schema_doc ← load and parse YAML/JSON file corresponding to file_part
  5. resolved ← locate target node by json_pointer in schema_doc
  6. If resolved still contains $ref → recursively call resolve_ref(...)
  7. Return resolved
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(d) | d is reference depth |
| Space | O(d) | visited_refs set size + recursive call stack |

**Implementation Notes:**

- Implementations **SHOULD** limit maximum reference depth to 32 (configurable via `schema.max_ref_depth`)
- Reference formats come in three types:
  - Local reference: `#/definitions/ErrorDetail`
  - Relative path reference: `./common/error.schema.yaml#/definitions/ErrorDetail`
  - Canonical ID reference: `apcore://common.types.error/ErrorDetail`
- `apcore://` protocol conversion rule: `apcore://{canonical_id}/{pointer}` → `{schemas_dir}/{canonical_id}.schema.yaml#/{pointer}`
- JSON Pointer follows RFC 6901 specification

---

## 5. Module-Related Algorithms

### A06: `resolve_entry_point()` — Module Entry Point Resolution

**Source**: PROTOCOL_SPEC §5.2

**Description**: Resolves the code entry point (filename and class name) of a module. Prioritizes explicit configuration in metadata files, otherwise auto-infers.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `meta_yaml` | `Object \| null` | Metadata file content (may not exist) |
| `file_path` | `String` | Module file path |
| `language` | `String` | File language (determined by extension) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `entry_point` | `Object` | `{ file: string, class_name: string }` |

**Preconditions:**

- `file_path` points to a valid file

**Postconditions:**

- Both `entry_point.file` and `entry_point.class_name` are not empty

**Pseudocode:**

```
Algorithm: resolve_entry_point(meta_yaml, file_path, language)

Steps:
  1. If meta_yaml exists and contains entry_point field:
     a. Parse format "filename:ClassName"
     b. Return { file: filename, class_name: ClassName }
  2. Otherwise, auto-infer:
     a. file ← filename from file_path (without extension)
     b. class_name ← convert file from snake_case to PascalCase
     c. If language == "python": search file for class inheriting from Module
     d. If found unique match → return that class
     e. If found multiple matches → throw AMBIGUOUS_ENTRY_POINT error
     f. If no match found → throw NO_MODULE_CLASS error
  3. Return entry_point
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is file content size (auto-inference requires file scanning) |
| Space | O(1) | Constant extra space |

**Implementation Notes:**

- `entry_point` format is `"filename:ClassName"` (e.g., `"db_params:DbParamsValidator"`)
- snake_case → PascalCase conversion must follow acronym rules (§2.3)
- In Python implementation, "finding class inheriting from Module" can be done via AST parsing or reflection
- Other languages may have different class discovery mechanisms (e.g., Rust trait implementation, Go interface implementation)

---

### A07: `resolve_dependencies()` — Dependency Topological Sort

**Source**: PROTOCOL_SPEC §5.3

**Description**: Uses topological sort (Kahn's algorithm) to resolve module loading order. **MUST** detect circular dependencies.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `modules` | `List<Object>` | Module collection, each module contains `{ id: String, dependencies: List<String> }` |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `load_order` | `List<String>` | List of module IDs arranged in dependency order |

**Preconditions:**

- All module IDs have passed format validation

**Postconditions:**

- For any module M in `load_order`, all dependencies of M are placed before M
- Throws error if circular dependencies exist

**Pseudocode:**

```
Algorithm: resolve_dependencies(modules)

Steps:
  1. Build dependency graph: Map<module_id, Set<dependency_id>>
  2. Calculate in-degree: Map<module_id, int>
  3. queue ← all modules with in-degree 0
  4. load_order ← []
  5. While queue is not empty:
     a. current ← queue.dequeue()
     b. load_order.append(current)
     c. For each dependent of current:
        - in_degree[dependent] -= 1
        - If in_degree[dependent] == 0 → queue.enqueue(dependent)
  6. If len(load_order) < len(modules):
     - remaining ← modules not in load_order
     - Throw CIRCULAR_DEPENDENCY error with cycle path
  7. Return load_order
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(V + E) | V is number of modules, E is number of dependency relationships |
| Space | O(V + E) | Storage for dependency graph and in-degree table |

**Implementation Notes:**

- Circular dependency detection should provide meaningful error messages, including cycle path (e.g., `A → B → C → A`)
- Optional dependencies (`optional: true`) should be skipped rather than error when missing
- Version constraints (`version: ">=1.0.0"`) need to be resolved before building dependency graph
- Modules with same in-degree have no guaranteed order, implementations **may** sort by ID alphabetically for stable output

---

## 6. ACL-Related Algorithms

### A08: `match_pattern()` — ACL Pattern Matching

**Source**: PROTOCOL_SPEC §6.2

**Description**: Matches ACL rule patterns with module Canonical IDs. Supports `*` wildcard.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `pattern` | `String` | ACL pattern (e.g., `"api.*"`, `"*.validator.*"`, `"executor.email.send_email"`) |
| `module_id` | `String` | Module Canonical ID to match |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `matched` | `Boolean` | Whether matched |

**Preconditions:**

- `module_id` conforms to Canonical ID format

**Postconditions:**

- Match result is deterministic (same inputs always return same result)

**Pseudocode:**

```
Algorithm: match_pattern(pattern, module_id)

Steps:
  1. If pattern == "*" → return true
  2. If pattern does not contain "*":
     → return pattern == module_id (exact match)
  3. Split pattern by "*" into segments
  4. Use greedy matching algorithm:
     a. pos ← 0
     b. For each segment (non-empty):
        - Search for segment in module_id[pos:]
        - If not found → return false
        - pos ← position found + len(segment)
     c. If pattern does not end with "*" → module_id must end with last segment
  5. Return true
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(m * n) | m is pattern length, n is module_id length |
| Space | O(m) | Storage for split segments |

**Implementation Notes:**

- `*` matches any number of any characters (including `.`), meaning `api.*` can match `api.handler.task_submit` (cross-level)
- If pattern starts with `*`, first segment is empty, matching starts from beginning of module_id
- Exact match (no wildcard) should prioritize string comparison to avoid unnecessary splitting
- Match results can be cached for performance (pattern and module_id combination as cache key)

---

### A09: `evaluate_acl()` — ACL Rule Evaluation

**Source**: PROTOCOL_SPEC §6.3

**Description**: Evaluates a set of ACL rules to decide whether to allow a specific call. This is the core algorithm of the ACL engine.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `caller_id` | `String \| null` | Caller module ID (`null` means external call) |
| `target_id` | `String` | Callee module ID |
| `rules` | `List<Rule>` | List of rules |
| `default_effect` | `String` | Default policy (`"allow"` \| `"deny"`) |

Where `Rule` structure is:

```
Rule {
  id: String,
  callers: List<String>,   // Pattern list
  targets: List<String>,   // Pattern list
  actions: List<String>,
  effect: "allow" | "deny",
  priority: Integer         // Default 0
}
```

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `decision` | `Object` | `{ effect: "allow" \| "deny", matched_rule: Rule \| null }` |

**Preconditions:**

- `target_id` conforms to Canonical ID format

**Postconditions:**

- Returns deterministic decision result

**Pseudocode:**

```
Algorithm: evaluate_acl(caller_id, target_id, rules, default_effect)

Steps:
  1. effective_caller ← caller_id ?? "@external"
  2. Sort rules by priority descending (maintain definition order when priority equal)
  3. For rules with same priority, deny comes before allow
  4. For each rule ∈ sorted_rules:
     a. caller_matched ← false
        For each pattern ∈ rule.callers:
          If match_pattern(pattern, effective_caller) → caller_matched ← true; break
     b. target_matched ← false
        For each pattern ∈ rule.targets:
          If match_pattern(pattern, target_id) → target_matched ← true; break
     c. If caller_matched and target_matched:
        → return { effect: rule.effect, matched_rule: rule }
  5. Return { effect: default_effect, matched_rule: null }
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(R * P * M) | R is number of rules, P is average patterns per rule, M is match_pattern complexity |
| Space | O(R) | Extra space for sorting |

**Implementation Notes:**

- When `caller_id` is `null`, it **MUST** be replaced with `"@external"`
- Sorting stability is important: within same priority, deny must come before allow
- Missing `actions` field is treated as `["*"]` (matches all operations)
- Empty `callers` or `targets` array means the rule never matches
- Modules calling themselves also need ACL checking

---

### A10: `calculate_specificity()` — Pattern Specificity Scoring

**Source**: PROTOCOL_SPEC §6.4

**Description**: Calculates the specificity score of an ACL pattern, used to further distinguish rule matching precision within the same priority. Higher score means more specific pattern.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `pattern` | `String` | ACL pattern string |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `score` | `Integer` | Specificity score (integer, higher is more specific) |

**Preconditions:**

- None

**Postconditions:**

- Pure wildcard `"*"` has score 0
- Fully exact match has highest score

**Pseudocode:**

```
Algorithm: calculate_specificity(pattern)

Steps:
  1. If pattern == "*" → return 0
  2. segments ← split pattern by "."
  3. score ← 0
  4. For each segment:
     a. If segment == "*" → score += 0
     b. If segment contains "*" (partial wildcard) → score += 1
     c. If segment does not contain "*" (exact match) → score += 2
  5. Return score
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is number of segments in pattern |
| Space | O(n) | Storage for split segments |

**Implementation Notes:**

- Scoring examples: `"*"` → 0, `"api.*"` → 2, `"api.handler.*"` → 4, `"api.handler.task_submit"` → 6
- This algorithm is **SHOULD** level, Level 0 implementations can skip
- Specificity scoring is mainly used for ACL debugging and conflict analysis

---

## 7. Error Handling Algorithms

### A11: `propagate_error()` — Error Propagation

**Source**: PROTOCOL_SPEC §8.3

**Description**: Wraps raw errors/exceptions generated during module execution into standardized ModuleError objects, preserving error chain and trace information.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `error` | `Exception` | Raw exception/error object |
| `module_id` | `String` | Module ID where error occurred |
| `context` | `Context` | Current execution context |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `module_error` | `ModuleError` | Standardized error object |

**Preconditions:**

- `context` contains valid `trace_id` and `call_chain`

**Postconditions:**

- Returned `ModuleError` contains `code`, `message`, `trace_id`, `module_id`
- Original error is saved in `cause` field

**Pseudocode:**

```
Algorithm: propagate_error(error, module_id, context)

Steps:
  1. If error is already ModuleError type:
     a. Keep original error.code
     b. Append current module_id to error.chain
     c. Return error
  2. Construct module_error:
     a. code ← map based on error type:
        - SchemaValidationError → "SCHEMA_VALIDATION_ERROR"
        - ACLDeniedError → "ACL_DENIED"
        - TimeoutError → "MODULE_TIMEOUT"
        - Other → "MODULE_EXECUTE_ERROR"
     b. message ← human-readable message from error
     c. details ← extract structured details from error
     d. cause ← error (preserve original error)
     e. trace_id ← context.trace_id
     f. module_id ← module_id
     g. call_chain ← copy of context.call_chain
     h. timestamp ← current UTC time (ISO 8601)
  3. Return module_error
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(1) | Constant time operations |
| Space | O(c) | c is call_chain length (needs copy) |

**Implementation Notes:**

- Errors already ModuleError should append call chain rather than re-wrap, avoiding information loss
- `timestamp` **MUST** be UTC time, format ISO 8601 (e.g., `"2026-02-07T10:30:00Z"`)
- `call_chain` **MUST** be a copy not a reference, preventing subsequent modifications affecting error record
- Error type mapping should be extensible, allowing implementations to add custom mapping rules

---

### A17: `detect_error_code_collisions()` — Error Code Collision Detection

**Source**: PROTOCOL_SPEC §8.4

**Description**: Detects conflicts between module custom error codes and framework reserved error codes as well as other module error codes.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `framework_codes` | `Set<String>` | Set of framework reserved error codes |
| `module_codes_map` | `Map<String, Set<String>>` | Mapping from module ID → module custom error codes |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `all_codes` | `Set<String>` | Complete error code registry |

**Preconditions:**

- None

**Postconditions:**

- All error codes are unique, no conflicts

**Pseudocode:**

```
Algorithm: detect_error_code_collisions(framework_codes, module_codes_map)

Steps:
  1. all_codes ← copy of framework_codes
  2. For each (module_id, codes) ∈ module_codes_map:
     For each code ∈ codes:
       a. If code ∈ framework_codes → throw error: module cannot use framework reserved code
       b. If code ∈ all_codes → throw error: error code already registered by another module
       c. all_codes ← all_codes ∪ {code}
  3. Return all_codes (complete error code registry)
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is total number of all module error codes |
| Space | O(n) | Storage for complete error code registry |

**Implementation Notes:**

- Framework reserved error code prefixes include `MODULE_`, `SCHEMA_`, `ACL_`, `GENERAL_`, `CONFIG_`, `CIRCULAR_`, `DEPENDENCY_`
- Module error code naming **SHOULD** follow `{MODULE_PREFIX}_{ERROR_NAME}` format
- This detection should be executed during framework startup, report error immediately upon finding conflict

---

## 8. Configuration-Related Algorithms

### A12: `validate_config()` — Configuration Validation

**Source**: PROTOCOL_SPEC §9.3

**Description**: Validates merged configuration object (environment variables + config file + defaults) during framework startup, ensuring all required fields exist, types are correct, and constraints are satisfied.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `config` | `Object` | Merged configuration object (env + file + defaults) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `validated_config` | `Object` | Validated configuration object |

**Preconditions:**

- Configuration object has been merged by priority (env > file > defaults)

**Postconditions:**

- All required fields are populated
- All field types are correct
- All constraints are satisfied

**Pseudocode:**

```
Algorithm: validate_config(config)

Steps:
  1. For each required field (MUST):
     If missing → throw CONFIG_INVALID with missing field path
  2. Type validation:
     For each field, validate value type conforms to Schema definition
  3. Constraint validation:
     - extensions.root MUST be valid directory path
     - schema.root MUST be valid directory path
     - acl.default_effect MUST be "allow" or "deny"
     - observability.tracing.sampling_rate MUST be in [0.0, 1.0] range
     - extensions.max_depth MUST be in [1, 16] range
  4. Semantic validation:
     - If extensions.auto_discover == true and extensions.root does not exist → warning
     - If schema.strategy == "yaml_only" and schema.root does not exist → error
  5. Return validated_config
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is total number of configuration fields |
| Space | O(1) | In-place validation, constant extra space |

**Implementation Notes:**

- Configuration merge priority: environment variables > config file > defaults (see §9.2)
- Environment variable naming convention: `APCORE_{SECTION}_{KEY}`, all uppercase, hyphens converted to underscores
- Required field list: `version`, `extensions.root`, `schema.root`, `acl.root`, `acl.default_effect`, `project.name`
- Validation errors should collect all errors before reporting, rather than stopping at first error

---

## 9. Observability Algorithms

### A13: `redact_sensitive()` — Sensitive Data Redaction

**Source**: PROTOCOL_SPEC §10.5

**Description**: Redacts fields marked with `x-sensitive` in log and trace output, replacing sensitive values with `"***REDACTED***"`.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `data` | `Object` | Data object to be redacted |
| `schema` | `Object` | Corresponding JSON Schema (contains `x-sensitive` markers) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `redacted_data` | `Object` | Redacted data copy |

**Preconditions:**

- Structure of `data` is consistent with `schema`

**Postconditions:**

- All fields with `x-sensitive: true` have values replaced with `"***REDACTED***"`
- Original `data` is unaffected (returns copy)

**Pseudocode:**

```
Algorithm: redact_sensitive(data, schema)

Steps:
  1. redacted ← deep_copy(data)
  2. For each (field_name, field_schema) in schema.properties:
     a. If field_schema["x-sensitive"] == true:
        - If redacted[field_name] exists and is not null:
          redacted[field_name] ← "***REDACTED***"
     b. If field_schema.type == "object" and has properties:
        - Recurse: redacted[field_name] ← redact_sensitive(redacted[field_name], field_schema)
     c. If field_schema.type == "array" and items has x-sensitive:
        - Execute redaction on each element in array
  3. Return redacted
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is total number of data fields (including nested) |
| Space | O(n) | deep_copy space overhead |

**Implementation Notes:**

- Redaction **MUST** operate on copy, **forbidden** to modify original data
- Replacement value is fixed as `"***REDACTED***"`, must not leak length information of original value
- `x-sensitive` fields in nested objects must also be recursively processed
- Each element in array needs independent redaction (if items schema contains `x-sensitive`)
- If `data` contains fields not defined in Schema, these fields are not redacted

---

## 10. Version Management Algorithms

### A14: `negotiate_version()` — Version Negotiation

**Source**: PROTOCOL_SPEC §13.3

**Description**: Performs version negotiation when SDK loads configuration or Schema, determines effective version number. Ensures major version compatibility, provides appropriate handling for minor version differences.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `declared_version` | `String` | Version declared in configuration/Schema (e.g., `"1.2.0"`) |
| `sdk_version` | `String` | Maximum version supported by current SDK (e.g., `"1.3.0"`) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `effective_version` | `String` | Effective version number |

**Preconditions:**

- Both version numbers conform to semantic versioning (semver) specification

**Postconditions:**

- Throws error when major versions differ
- Throws error when declared version is higher than SDK version

**Pseudocode:**

```
Algorithm: negotiate_version(declared_version, sdk_version)

Steps:
  1. Parse declared_version into (major_d, minor_d, patch_d)
  2. Parse sdk_version into (major_s, minor_s, patch_s)
  3. If major_d ≠ major_s:
     → throw VERSION_INCOMPATIBLE ("Major version incompatible")
  4. If minor_d > minor_s:
     → throw VERSION_INCOMPATIBLE ("SDK version too low, please upgrade")
  5. If minor_d < minor_s:
     → issue DEPRECATION_WARNING (if minor_s - minor_d > 2)
     → effective_version ← declared_version (backward compatibility mode)
  6. If minor_d == minor_s:
     → effective_version ← max(declared_version, sdk_version)
  7. Return effective_version
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(1) | Constant time version number comparison |
| Space | O(1) | Constant space |

**Implementation Notes:**

- semver parsing needs to handle pre-release tags (e.g., `1.0.0-draft`, `1.0.0-alpha`)
- `max()` comparison follows semver specification: major > minor > patch > prerelease
- Deprecation warning threshold is minor version difference > 2 (e.g., SDK 1.5.0 loading config declared as 1.2.0)
- Framework **SHOULD** log version negotiation result

---

### A15: `migrate_schema()` — Schema Migration

**Source**: PROTOCOL_SPEC §13.4

**Description**: Automatically performs migration when Schema version changes. Converts Schema from old version to target version through migration function chain.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `schema` | `Object` | Original Schema object |
| `from_version` | `String` | Original version number |
| `to_version` | `String` | Target version number |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `migrated_schema` | `Object` | Migrated Schema object |

**Preconditions:**

- Both `from_version` and `to_version` are valid semver version numbers
- Migration path exists from `from_version` to `to_version`

**Postconditions:**

- Migrated Schema conforms to target version specification
- Original Schema is unaffected (operation on copy)

**Pseudocode:**

```
Algorithm: migrate_schema(schema, from_version, to_version)

Steps:
  1. If from_version == to_version → return schema (no migration needed)
  2. migration_path ← find migration path from from_version to to_version
     (e.g., 1.0 → 1.1 → 1.2, each step has corresponding migration function)
  3. If migration_path is empty → throw MIGRATION_FAILED ("No available migration path")
  4. current_schema ← deep_copy(schema)
  5. For each (step_from, step_to, migrate_fn) in migration_path:
     a. current_schema ← migrate_fn(current_schema)
     b. Validate current_schema conforms to step_to version Schema specification
     c. If validation fails → throw MIGRATION_FAILED with step information
  6. Return current_schema
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(s * n) | s is number of migration steps, n is number of Schema fields (each step requires validation) |
| Space | O(n) | deep_copy space overhead |

**Implementation Notes:**

- Migration path finding can use graph shortest path algorithm
- Migration function types include: `add_field` (add field), `rename_field` (rename), `remove_field` (remove), `change_type` (type change)
- `change_type` only allowed in major version changes
- `remove_field` needs to go through at least 2 minor versions of deprecated period first
- Validation after each migration step ensures correctness of intermediate state

---

## 11. Extension Mechanism Algorithms

### A16: `load_extensions()` — Extension Loading

**Source**: PROTOCOL_SPEC §11.7

**Description**: Loads extension point implementations by priority and strategy. Supports three strategies: `first_success` (first successful takes effect), `all` (execute all), `fallback` (fallback chain).

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `config` | `Object` | Framework configuration |
| `extension_points` | `Map<String, List<ExtensionImpl>>` | Mapping from extension point name → implementation list |

Where `ExtensionImpl` structure is:

```
ExtensionImpl {
  class: String,          // Implementation class name
  priority: Integer,      // Priority
  config: Object          // Implementation configuration
}
```

**Output:**

- Active implementations of each extension point are registered to framework

**Preconditions:**

- Extension implementation classes declared in configuration can be loaded

**Postconditions:**

- Each extension point has at least one active implementation (framework default implementation as fallback)

**Pseudocode:**

```
Algorithm: load_extensions(config, extension_points)

Steps:
  1. Sort implementations of each extension point by priority descending
  2. For each extension point:
     a. If strategy == "first_success": try in order, first successful takes effect
     b. If strategy == "all": all implementations execute, merge results
     c. If strategy == "fallback": try in order, try next on failure
  3. If extension point has no available implementation → use framework default implementation
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n log n + n) | n is total number of extension implementations (sort + traverse) |
| Space | O(n) | Space required for sorting |

**Implementation Notes:**

- Framework default implementations as follows (see §11.3):
  - `SchemaLoader` → `YAMLSchemaLoader`
  - `ModuleLoader` → `DirectoryModuleLoader`
  - `IDConverter` → `DefaultIDConverter`
  - `ACLChecker` → `YAMLACLChecker`
  - `Executor` → `LocalExecutor`
- `first_success` strategy: stop when loading succeeds, suitable for Schema loading (cache → filesystem)
- `all` strategy: all implementations execute, suitable for notification extensions
- `fallback` strategy: similar to `first_success`, but continues trying next on failure
- Extension loading failure should log warning, but should not cause framework startup failure (unless no available implementation and no default implementation)

---

## 12. Function-based Module Definition and Binding Algorithms

### A18: generate_schema_from_function — Generate Schema from Function Signature

**Source Section:** PROTOCOL_SPEC §5.11.4

**Input:**

| Parameter | Type | Description |
|------|------|------|
| `callable` | Function/Method | Target function or method |

**Output:**

| Field | Type | Description |
|------|------|------|
| `input_schema` | JSONSchema | Input JSON Schema |
| `output_schema` | JSONSchema | Output JSON Schema |
| `description` | String | Module description |

**Preconditions:**

- callable is a valid function or method
- All parameters (except self/cls and context: Context) have type annotations

**Postconditions:**

- Generated Schema conforms to JSON Schema Draft 2020-12
- Schema behavior is equivalent to Class-based Module defined Schema

**Pseudocode:**

```
Algorithm: generate_schema_from_function(callable)

Steps:
  1. Extract function parameter list params (exclude self/cls and context: Context)
  2. For each param:
     a. Get type annotation type_hint
     b. If type_hint is missing → throw FUNC_MISSING_TYPE_HINT error
     c. Map type_hint to JSON Schema type (see §5.11.5 mapping table)
     d. Extract constraints from Annotated metadata, default values, etc.
     e. Extract description from docstring parameter comments
  3. Construct input_schema:
     a. type: "object"
     b. properties: Schema mapping of all parameters
     c. required: list of parameters without default values
  4. Extract return type annotation return_type
     a. If return_type is missing → throw FUNC_MISSING_RETURN_TYPE error
     b. Map return_type to output_schema
  5. Extract description:
     a. docstring/comment first line → description
     b. parameter comments → each field description
  6. Return { input_schema, output_schema, description }
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is number of parameters |
| Space | O(n) | Schema object size |

**Implementation Notes:**

- `self` and `cls` parameters must be automatically excluded
- `context: Context` type parameter must be excluded and auto-injected
- Constraints in `Annotated[T, Field(...)]` (min, max, pattern, etc.) should be extracted to Schema
- `Optional[T]` should map to nullable type
- Nested object types (Python `BaseModel`, TypeScript object schema, Rust `struct`) should recursively generate

---

### A19: resolve_target — Resolve Binding Target

**Source Section:** PROTOCOL_SPEC §5.12.3

**Input:**

| Parameter | Type | Description |
|------|------|------|
| `target_string` | String | Target callable path (e.g., `myapp.services.email:send_email`) |

**Output:**

| Field | Type | Description |
|------|------|------|
| `callable` | Function/Method | Resolved callable object |

**Preconditions:**

- target_string conforms to `module.path:callable_name` format

**Postconditions:**

- Returned object is callable

**Pseudocode:**

```
Algorithm: resolve_target(target_string)

Steps:
  1. Split target_string by ":" into (module_path, callable_name)
     If no ":" → throw BINDING_INVALID_TARGET error
  2. import module_path
     If import fails → throw BINDING_MODULE_NOT_FOUND error
  3. If callable_name contains ".":
     a. Split by "." into (class_name, method_name)
     b. Find class_name in module
     c. Find method_name on class instance
     d. If any step fails → throw BINDING_CALLABLE_NOT_FOUND error
  4. Otherwise:
     a. Find callable_name in module
     b. If not found → throw BINDING_CALLABLE_NOT_FOUND error
  5. Validate result is callable
     If not callable → throw BINDING_NOT_CALLABLE error
  6. Return callable
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(1) | Excluding module loading time |
| Space | O(1) | Only reference objects |

**Implementation Notes:**

- Class methods (`ClassName.method_name`) need to instantiate class first or resolve to bound method
- Import failure should provide clear error message, including module path and possible reason
- For compiled languages like Go/Rust, target resolution is done at compile time or startup

---

## 13. Call Chain Safety Check Algorithm

### A20: `guard_call_chain()` — Call Chain Safety Check

**Source**: Executor API §6.1

**Description**: Three-layer call chain protection executed by Executor on each `call()`: depth limit, cycle detection, frequency detection. This algorithm unifies previously scattered security check logic in modules, ensuring all module calls are automatically protected. Frequency detection mainly defends against non-strict cyclic patterns in AI orchestration scenarios (e.g., A→B→C→B→C→B→C...).

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `module_id` | `String` | Target module ID to be called |
| `call_chain` | `List<String>` | Call chain in current Context |
| `max_depth` | `Integer` | Maximum call depth (default 32) |
| `max_module_repeat` | `Integer` | Maximum occurrences of same module (default 3) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| — | `void` | Returns nothing if check passes; throws corresponding error if fails |

**Preconditions:**

- `call_chain` is automatically managed by Executor, cannot be tampered by module code
- `module_id` has passed Canonical ID format validation

**Postconditions:**

- If no exception thrown, `module_id` can be safely appended to `call_chain` and executed
- If exception thrown, call chain is not modified

**Pseudocode:**

```
Algorithm: guard_call_chain(module_id, call_chain, max_depth, max_module_repeat)

Steps:
  1. Depth check:
     If len(call_chain) >= max_depth:
       → throw CALL_DEPTH_EXCEEDED {
           module_id: module_id,
           current_depth: len(call_chain),
           max_depth: max_depth,
           call_chain: call_chain
         }

  2. Cycle detection (strict cycle):
     If module_id ∈ call_chain:
       → throw CIRCULAR_CALL {
           module_id: module_id,
           call_chain: call_chain,
           cycle_start: call_chain.index(module_id)
         }

  3. Frequency detection:
     repeat_count ← 0
     For each id ∈ call_chain:
       If id == module_id → repeat_count += 1
     If repeat_count >= max_module_repeat:
       → throw CALL_FREQUENCY_EXCEEDED {
           module_id: module_id,
           count: repeat_count,
           max_repeat: max_module_repeat,
           call_chain: call_chain
         }

  4. Check passed, return normally
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is call_chain length (traverse to check frequency) |
| Space | O(1) | Only counter needed, no extra space |

**Implementation Notes:**

- Execution order of three checks **MUST** be: depth → cycle → frequency (depth check is cheapest, should execute first)
- `max_module_repeat` default value is 3, configurable via `apcore.yaml`'s `executor.max_module_repeat`, range `[1, 32]`
- Step 2 cycle detection covers direct cycle (A→B→A) and indirect cycle (A→B→C→A)
- Step 3 frequency detection covers non-cycle repeated calls (e.g., same module called multiple times in parallel orchestration then re-entered)
- If step 2 already threw `CIRCULAR_CALL`, step 3 won't execute (short-circuit)
- For modules with legitimate multiple call needs (e.g., retry logic), can configure `max_repeat_override` in module metadata to override default limit
- All errors **MUST** carry complete `call_chain` copy for debugging and observability

**Configuration Reference:**

```yaml
# apcore.yaml
executor:
  max_call_depth: 32          # Maximum call depth
  max_module_repeat: 3        # Maximum occurrences of same module
```

**Error Example:**

```
Scenario: AI orchestrator repeatedly calls B and C

call_chain: ["orchestrator.ai_planner", "executor.b", "executor.c", "executor.b", "executor.c", "executor.b"]
                                                                                                         ↑
Target: executor.c → count("executor.c") == 2 → still within limit (< 3)

call_chain: ["orchestrator.ai_planner", "executor.b", "executor.c", "executor.b", "executor.c", "executor.b", "executor.c"]
                                                                                                                          ↑
Target: executor.b → count("executor.b") == 3 → triggers CALL_FREQUENCY_EXCEEDED
```

---

## 14. Algorithm Dependency Relationships

The following diagram shows the calling/dependency relationships between algorithms:

```
scan_extensions() ──────────────────┐
  ├── directory_to_canonical_id(namespace)  │
  └── detect_id_conflicts()         │
                                    ▼
                              Registry.discover()
                                    │
                              ┌─────┴──────┐
                              ▼            ▼
                    resolve_entry_point()  resolve_dependencies()
                              │
                              ▼
                        Executor.execute()
                          ├── validate_config()  (at startup)
                          ├── guard_call_chain()  (per call)
                          ├── evaluate_acl()
                          │     └── match_pattern()
                          │           └── calculate_specificity()
                          ├── resolve_ref()      (during Schema loading)
                          ├── redact_sensitive()  (during log output)
                          └── propagate_error()   (on error)

negotiate_version() ←── framework startup / Schema loading
migrate_schema()    ←── when Schema version mismatch
load_extensions()   ←── framework startup

detect_error_code_collisions() ←── framework startup (after all modules loaded)

generate_schema_from_function() ←── module() registration / Binding auto_schema
resolve_target()                ←── Binding file loading
to_strict_schema()              ←── export_schema(strict=true)
```

---

## 15. Concurrency Model Related Algorithms

### A21: `safe_unregister()` — Hot-reload Safe Unregistration

**Source**: PROTOCOL_SPEC §12.7.4

**Purpose**: Safely unregister module when it may be executing, avoiding race conditions and resource leaks.

**Signature**:

```
safe_unregister(module_id: string, registry: Registry) → bool
```

**Input**:
- `module_id`: Module ID to unregister
- `registry`: Registry instance

**Output**:
- `true`: Successfully unregistered
- `false`: Timeout or failure

**Preconditions**:
- Registry is initialized

**Postconditions**:
- If returns `true`, module is completely unregistered, `on_unload()` has been called
- If returns `false`, module may not be completely cleaned up (resource leak risk)

**Pseudocode**:

```
Algorithm: safe_unregister(module_id, registry)

Input: module_id (string), registry (Registry instance)
Output: boolean (whether successfully unregistered)

Steps:
  1. If module_id not in registry:
     return true  # Idempotent

  2. module ← registry.get(module_id)

  3. Atomic operation: mark module.state ← "UNLOADING"

  4. Remove module_id from registry.modules
     # New call() will throw MODULE_NOT_FOUND

  5. Wait for execution to complete:
     timeout ← 30 seconds
     start_time ← now()
     Loop:
       If module.ref_count == 0:
         break  # All calls completed
       If (now() - start_time) > timeout:
         log ERROR: "Module {module_id} unregister timeout, force unloading"
         return false  # Timeout, force unload
       sleep(100ms)  # Wait

  6. Call module.on_unload():
     try:
       module.on_unload()
     catch exception e:
       log ERROR: "on_unload() failed: {e}"

  7. Release module instance (GC reclaim)

  8. return true
```

**Complexity Analysis**:
- Time complexity: O(1) + wait time (up to 30 seconds)
- Space complexity: O(1)

**Implementation Notes**:

1. **Reference count maintenance**:
   - At `call()` start `ref_count++`
   - At `call()` end (whether success/failure) `ref_count--`
   - Use atomic operations to ensure thread safety

2. **Timeout handling**:
   - Default 30 seconds adjustable via configuration
   - After timeout **MUST** log detailed info (module_id, current ref_count)

3. **Idempotency**:
   - Multiple `unregister()` of same ID should silently succeed

**Example**:

```python
# Python example
import time
import threading

class Registry:
    def __init__(self):
        self.modules = {}
        self.locks = {}

    def safe_unregister(self, module_id):
        if module_id not in self.modules:
            return True  # Idempotent

        module = self.modules[module_id]
        module.state = "UNLOADING"

        # Remove from registry (new calls will fail)
        del self.modules[module_id]

        # Wait for execution to complete
        timeout = 30
        start = time.time()
        while module.ref_count > 0:
            if time.time() - start > timeout:
                logger.error(f"Module {module_id} unregister timeout")
                return False
            time.sleep(0.1)

        # Call hook
        try:
            module.on_unload()
        except Exception as e:
            logger.error(f"on_unload() failed: {e}")

        return True
```

---

### A22: `enforce_timeout()` — Timeout Enforcement

**Source**: PROTOCOL_SPEC §12.7.5

**Purpose**: Ensure module execution completes within specified time, cooperative cancellation or forced termination after timeout.

**Signature**:

```
enforce_timeout(module_id: string, inputs: dict, context: Context, timeout_ms: int) → dict | error
```

**Input**:
- `module_id`: Module ID
- `inputs`: Input parameters
- `context`: Execution context
- `timeout_ms`: Timeout duration (milliseconds)

**Output**:
- Success: Module output (dict)
- Failure: `MODULE_TIMEOUT` error

**Preconditions**:
- `timeout_ms > 0`
- Module is registered

**Postconditions**:
- If completes within time limit, returns normal output
- If timeout, throws `MODULE_TIMEOUT` error

**Pseudocode**:

```
Algorithm: enforce_timeout(module_id, inputs, context, timeout_ms)

Input: module_id (string), inputs (dict), context (Context), timeout_ms (integer)
Output: Module output (dict) or MODULE_TIMEOUT error

Steps:
  1. If timeout_ms == 0:
     # Disable timeout
     return execute_with_middleware(module_id, inputs, context)

  2. Create cancellation token: cancel_token ← new CancellationToken()

  3. Start main task (async):
     task ← async execute_with_middleware(
       module_id, inputs, context, cancel_token
     )

  4. Start timeout monitor (async):
     timeout_task ← async sleep(timeout_ms)

  5. Wait for race:
     result ← await race(task, timeout_task)

  6. If task completes first:
     cancel timeout_task
     return result  # Normal return

  7. If timeout_task completes first (timeout):
     a. Send cancellation signal: cancel_token.cancel()
     b. Wait grace_period (default 5 seconds):
        try:
          result ← await wait(task, timeout=5000ms)
          return result  # Module responded to cancellation, normal exit
        catch timeout:
          continue to step c

     c. Force termination (if language supports):
        If supports thread/coroutine termination:
          force terminate task
          log ERROR: "Module {module_id} force killed after timeout"
        Otherwise:
          log WARN: "Module {module_id} timeout but cannot be killed"

     d. Throw error:
        throw MODULE_TIMEOUT(
          message: f"Module {module_id} execution timeout ({timeout_ms}ms)",
          module_id: module_id,
          timeout: timeout_ms
        )
```

**Complexity Analysis**:
- Time complexity: O(1) + module execution time (up to timeout_ms + grace_period)
- Space complexity: O(1)

**Implementation Notes**:

1. **Cooperative cancellation first**:
   - Module should check `cancel_token.is_cancelled()` and actively exit
   - Example (Python):
     ```python
     async def execute(self, inputs, context):
         for i in range(1000):
             if context.cancel_token.is_cancelled():
                 raise CancelledError()
             await process_item(i)
     ```

2. **Force termination risk**:
   - Force termination may cause resource leaks (unclosed files, unreleased locks)
   - **SHOULD** only use after cooperative cancellation fails
   - Some languages (e.g., Java, Go) don't recommend forcing thread termination

3. **Timeout timing start point**:
   - From first `before()` middleware
   - Includes total time for Schema validation, ACL checking, all middleware and `execute()`

**Example**:

```python
# Python example
import asyncio

async def enforce_timeout(module_id, inputs, context, timeout_ms):
    if timeout_ms == 0:
        return await execute_with_middleware(module_id, inputs, context)

    cancel_token = CancellationToken()

    # Main task
    task = asyncio.create_task(
        execute_with_middleware(module_id, inputs, context, cancel_token)
    )

    # Timeout monitor
    try:
        result = await asyncio.wait_for(task, timeout=timeout_ms / 1000)
        return result
    except asyncio.TimeoutError:
        # Cooperative cancellation
        cancel_token.cancel()
        try:
            result = await asyncio.wait_for(task, timeout=5)  # grace period
            return result
        except asyncio.TimeoutError:
            # Force termination (Python doesn't support, only log)
            logger.error(f"Module {module_id} timeout, cannot force kill")
            raise ModuleError(
                code="MODULE_TIMEOUT",
                message=f"Module {module_id} timeout ({timeout_ms}ms)"
            )
```

---

## 16. Schema Export Algorithm

### A23: `to_strict_schema()` — Strict Mode Schema Conversion

**Source**: PROTOCOL_SPEC §4.16

**Description**: Converts apcore standard JSON Schema (with `x-*` extension fields and optional properties) to OpenAI / Anthropic Strict Mode compatible JSON Schema. Strict Mode requires all nested objects to set `additionalProperties: false`, all properties must be in `required` array, optional fields expressed through nullable types.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `schema` | `Object` | apcore standard JSON Schema (may contain `x-*` extension fields) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `strict_schema` | `Object` | Strict Mode compatible JSON Schema |

**Preconditions:**

- `schema` is a valid JSON Schema object

**Postconditions:**

- All `type: "object"` nodes have `additionalProperties: false`
- All field names in `properties` are in `required` array
- Does not contain any `x-*` prefixed fields
- Does not contain `default` field
- Original `schema` is unaffected (operation on copy)

**Pseudocode:**

```
Algorithm: to_strict_schema(schema)

Steps:
  1. result ← deep_copy(schema)
  2. result ← strip_extensions(result)
  3. result ← convert_to_strict(result)
  4. Return result

---

Sub-algorithm: strip_extensions(node)

Steps:
  1. If node is not Object → return node
  2. For each key in node:
     a. If key starts with "x-" → delete this key-value pair
     b. If key == "default" → delete this key-value pair
     c. If node[key] is Object → node[key] ← strip_extensions(node[key])
     d. If node[key] is Array:
        For each element elem:
          If elem is Object → elem ← strip_extensions(elem)
  3. Return node

---

Sub-algorithm: convert_to_strict(node)

Steps:
  1. If node is not Object → return node
  2. If node.type == "object" and node contains properties:
     a. node.additionalProperties ← false
     b. existing_required ← node.required ?? []
     c. all_property_names ← all keys in node.properties
     d. optional_names ← all_property_names - existing_required
     e. For each name ∈ optional_names:
        prop ← node.properties[name]
        If prop.type is string (single type):
          prop.type ← [prop.type, "null"]
        If prop.type is array and does not contain "null":
          prop.type ← prop.type ∪ ["null"]
        If prop does not contain type (e.g., pure $ref):
          prop ← { oneOf: [prop, { type: "null" }] }  # Wrap as oneOf
     f. node.required ← all_property_names (all fields)
  3. Recursively process nested structures:
     a. If node.properties exists:
        For each (key, prop) ∈ node.properties:
          node.properties[key] ← convert_to_strict(prop)
     b. If node.items exists:
        node.items ← convert_to_strict(node.items)
     c. For keyword ∈ ["oneOf", "anyOf", "allOf"]:
        If node[keyword] exists:
          For each sub_schema ∈ node[keyword]:
            sub_schema ← convert_to_strict(sub_schema)
  4. Return node
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is total number of nodes in Schema (recursive traversal) |
| Space | O(n) | deep_copy space overhead |

**Implementation Notes:**

- Conversion **MUST** be performed on copy, **forbidden** to modify original Schema
- `x-llm-description` **SHOULD** first replace corresponding field's `description` before stripping (see §4.3 export rules), then execute `strip_extensions()`
- Pure `$ref` nodes (no `type`) use `oneOf` wrapping when making nullable, rather than directly adding `type`
- If `additionalProperties` already exists and is `true`, **MUST** change to `false`
- For fields already in `type: ["string", "null"]` form, should not add `"null"` again
- `object` inside `items` also need recursive processing
- `default` values are invalid in Strict Mode (AI won't auto-fill), so remove them as well

**Example — Complex Conversion:**

```yaml
# Input (apcore standard Schema)
type: object
properties:
  to:
    type: string
    description: "Recipient email"
    x-llm-description: "Recipient email address, must be valid email format"
    x-examples: ["user@example.com"]
  cc:
    type: array
    items:
      type: string
    default: []
  config:
    type: object
    properties:
      retry:
        type: integer
        default: 3
      timeout:
        type: integer
required: [to]

# Output (Strict Mode)
type: object
properties:
  to:
    type: string
    description: "Recipient email address, must be valid email format"
  cc:
    type: ["array", "null"]
    items:
      type: string
  config:
    type: ["object", "null"]
    properties:
      retry:
        type: ["integer", "null"]
      timeout:
        type: ["integer", "null"]
    required: [retry, timeout]
    additionalProperties: false
required: [to, cc, config]
additionalProperties: false
```

---

## A24. Stream Chunk Aggregation

### A24.1 deep_merge_chunks (Recursive Deep Merge with Depth Cap)

**Purpose.** Aggregate the chunks yielded by a streaming module's `stream()` generator into the final output dict that the executor validates against `output_schema`.

**Source section.** PROTOCOL_SPEC §5 streaming semantics. Verified by `conformance/fixtures/stream_aggregation.json` (9 cases).

```
Algorithm: deep_merge_chunks(chunks, max_depth=32)

Input:
  chunks    — Ordered list of dict objects yielded by module.stream()
  max_depth — Maximum recursion depth (canonical default 32)

Output:
  result — A single dict that is the recursive deep-merge of all chunks

Steps:
  1. result ← {}
  2. For each chunk in chunks (in order):
       deep_merge(result, chunk, depth=0, max_depth=max_depth)
  3. Return result

Helper: deep_merge(base, override, depth, max_depth)
  1. If depth >= max_depth → Return  (do not recurse further; truncate)
  2. For each (key, value) in override:
       a. If key NOT in base:
            base[key] ← value
       b. Else if base[key] is dict AND value is dict:
            deep_merge(base[key], value, depth+1, max_depth)
       c. Else:
            base[key] ← value   (overwrite — arrays REPLACE, primitives overwrite)

Properties:
  - Arrays are replaced (not concatenated) at matching keys.
  - null overwrites (does not delete) the previous value.
  - Recursion is depth-capped to prevent stack exhaustion via adversarial input.
  - Beyond the depth cap, sub-objects are kept as-is from the override
    (the truncation point loses no data, only stops further traversal).
```

**Reference implementations:**

- `apcore-python/src/apcore/executor.py` — `_deep_merge()` (`_MAX_MERGE_DEPTH = 32`)
- `apcore-typescript/src/executor.ts` — `deepMergeChunk()`
- `apcore-rust/src/executor.rs` — `deep_merge_chunks()` and `deep_merge_value()`

**Notes:**

- Implementations **MAY** raise `STREAM_CHUNK_NOT_OBJECT` if a yielded chunk is not a dict; the canonical Python implementation matches this on `AttributeError` and the Rust mirror raises `STREAM_CHUNK_NOT_OBJECT` explicitly.
- The merge is performed in-place on an accumulator dict; iteration over the chunk stream is single-pass.

---

## 17. References

- [PROTOCOL_SPEC §2 — Naming Specification](../../PROTOCOL_SPEC.md#2-naming-specification)
- [PROTOCOL_SPEC §3 — Directory Specification](../../PROTOCOL_SPEC.md#3-directory-specification)
- [PROTOCOL_SPEC §4 — Schema Specification](../../PROTOCOL_SPEC.md#4-schema-specification)
- [PROTOCOL_SPEC §5 — Module Specification](../../PROTOCOL_SPEC.md#5-module-specification)
- [PROTOCOL_SPEC §5.11 — Function-based Module Definition](../../PROTOCOL_SPEC.md#511-function-based-module-definition)
- [PROTOCOL_SPEC §5.12 — External Schema Binding](../../PROTOCOL_SPEC.md#512-external-schema-binding)
- [PROTOCOL_SPEC §6 — ACL Specification](../../PROTOCOL_SPEC.md#6-acl-specification)
- [PROTOCOL_SPEC §7 — Approval System](../../PROTOCOL_SPEC.md#7-approval-system)
- [PROTOCOL_SPEC §8 — Error Handling Specification](../../PROTOCOL_SPEC.md#8-error-handling-specification)
- [PROTOCOL_SPEC §9 — Configuration Specification](../../PROTOCOL_SPEC.md#9-configuration-specification)
- [PROTOCOL_SPEC §10 — Observability Specification](../../PROTOCOL_SPEC.md#10-observability-specification)
- [PROTOCOL_SPEC §11 — Extension Mechanism](../../PROTOCOL_SPEC.md#11-extension-mechanism)
- [PROTOCOL_SPEC §13 — Versioning](../../PROTOCOL_SPEC.md#13-versioning)
