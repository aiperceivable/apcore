---
description: "Consolidated reference of the protocol-spec pseudocode algorithms A01–A25 (canonical-ID derivation, $ref resolution, ACL evaluation, etc.) with I/O types, pre/post-conditions, complexity, and notes."
---

# apcore — Core Algorithm Reference

!!! note "Derived from protocol-spec.md; protocol-spec wins on any conflict."
    Each algorithm here restates the corresponding algorithm in [protocol-spec.md](./protocol-spec.md) and adds implementation notes. Where an algorithm has no pseudocode in protocol-spec (A20, A23, A24), protocol-spec points here and this page is the pseudocode of record.

## 1. Overview

### 1.1 Purpose

The protocol specification defines algorithms that implementations must or should implement across various chapters. This document consolidates them to provide SDK implementers with a unified algorithm reference, including input/output types, pre/post-conditions, pseudocode, complexity analysis, and implementation notes.

### 1.2 Algorithm Index

| No. | Algorithm Name | Description | Source Section | Implementation Requirement |
|-----|---------------|-------------|----------------|---------------------------|
| A01 | `directory_to_canonical_id()` | Directory path to Canonical ID | §2.1 | **MUST** |
| A02 | `normalize_to_canonical_id()` | Cross-language ID normalization | §2.2 | **MUST** |
| A03 | `detect_id_conflicts()` | ID conflict detection | §2.6 | **MUST** |
| A04 | `scan_extensions()` | Extension directory scanning | §3.6 | **MUST** |
| A05 | `resolve_ref()` | Schema $ref reference resolution | §4.11 | **MUST** |
| A06 | `resolve_entry_point()` | Module entry point resolution | §5.2 | **MUST** |
| A07 | `resolve_dependencies()` | Dependency topological sort | §5.3 | **MUST** |
| A08 | `match_pattern()` | ACL pattern matching | §6.2 | **MUST** |
| A09 | `evaluate_acl()` | ACL rule evaluation | §6.3 | **MUST** |
| A10 | `calculate_specificity()` | Pattern specificity scoring | §6.4 | **SHOULD** |
| A11 | `propagate_error()` | Error propagation | §8.3 | **MUST** |
| A12 | `validate_config()` | Configuration validation | §9.3 | **MUST** |
| A13 | `redact_sensitive()` | Sensitive data redaction | §10.6 | **MUST** |
| A14 | `negotiate_version()` | Version negotiation | §13.3 | **MUST** |
| A15 | `migrate_schema()` | Schema migration | §13.4 | **SHOULD** |
| A16 | `load_extensions()` | Extension loading | §11.7 | **MUST** |
| A17 | `detect_error_code_collisions()` | Error code collision detection | §8.4 | **MUST** |
| A18 | `generate_schema_from_function()` | Generate JSON Schema from function signature | §5.11.4 | **SHOULD** |
| A19 | `resolve_target()` | Resolve binding target for function-based modules | §5.12.3 | **SHOULD** |
| A20 | `guard_call_chain()` | Call chain safety check | §9.1.1 keys; [features/call-chain-guard.md](../features/call-chain-guard.md) | **MUST** |
| A21 | `safe_unregister()` | Hot-reload safe unregistration | §12.7.4 | **MUST** |
| A22 | `enforce_timeout()` | Timeout enforcement | §12.7.5 | **MUST** |
| A23 | `to_strict_schema()` | Strict Mode Schema conversion | §4.16 | **SHOULD** |
| A24 | `deep_merge_chunks()` | Stream chunk aggregation (recursive deep merge, depth-capped) | §12.2 Streaming Execution Protocol | **MUST** |
| A25 | `match_glob()` | Portable glob matching for pattern-valued values | §9.2.3 | **MUST** |

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

**Source**: protocol-spec §2.1

**Description**: Converts the relative path of a module file to a dot-separated snake_case Canonical ID. This is the foundational implementation of apcore's "directory as ID" core concept.

**Input Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `file_path` | `String` | Full relative path of the module file (e.g., `"extensions/executor/validator/db_params.py"`) |
| `extensions_root` | `String` | Extension root directory name (default `"extensions"`) |

**Output:**

| Return Value | Type | Description |
|--------------|------|-------------|
| `canonical_id` | `String` | Dot-separated module ID (e.g., `"executor.validator.db_params"`) |

**Preconditions:**

- `file_path` must start with `extensions_root + "/"`
- `file_path` must contain a file extension

**Postconditions:**

- The returned `canonical_id` conforms to the EBNF grammar (§2.7)
- `canonical_id` length does not exceed 192 characters

**Pseudocode:**

```text
Algorithm: directory_to_canonical_id(file_path, extensions_root)

Steps:
  1. relative_path ← remove extensions_root + "/" prefix from file_path
  2. relative_path ← remove file extension (last "." and everything after it)
  3. segments ← split relative_path by "/"
  4. For each segment, perform validation:
     a. If segment is empty string → throw INVALID_PATH error
     b. If segment does not match /^[a-z][a-z0-9_]*$/ → throw INVALID_SEGMENT error
  5. canonical_id ← join all segments with "."
  6. If len(canonical_id) > 192 → throw ID_TOO_LONG error
  7. Return canonical_id
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
- A04 calls A01 with the file's **canonical real path**, never a symlink alias (D-127)
- In multi-root mode (`extensions.roots`) the root's namespace is prefixed to the ID A01 returns; see A04

---

### A02: `normalize_to_canonical_id()` — Cross-language ID Normalization

**Source**: protocol-spec §2.2

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

```text
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

- PascalCase → snake_case conversion needs to handle acronyms (see protocol-spec §2.3)
- Acronyms are treated as regular words: `HttpJsonParser` → `http_json_parser` (not `h_t_t_p_j_s_o_n_parser`)
- Common acronym list: `http`, `api`, `db`, `id`, `url`, `sql`, `json`, `xml`, `html`, `css`, `tcp`, `udp`, `ip`

---

### A03: `detect_id_conflicts()` — ID Conflict Detection

**Source**: protocol-spec §2.6

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

```text
Algorithm: detect_id_conflicts(new_id, existing_ids, reserved_words)

Steps:
  1. Exact duplicate detection:
     If new_id ∈ existing_ids → return { type: "duplicate_id", severity: "error" }
  2. Reserved word detection (FIRST SEGMENT ONLY):
     first_segment ← new_id up to the first "." (the whole ID when it has none)
     If first_segment ∈ reserved_words → return { type: "reserved_word", severity: "error" }
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
- Reserved words are the framework words of §2.5: `system`, `internal`, `core`, `apcore`, `plugin`, `schema`, `acl` (`ephemeral` is governed by its own namespace rule). Pinned by fixture `id_conflict_reserved_words`
- Step 2 tests the first segment only: a reserved word claims a namespace, and only the first segment can assert one. `executor.schema.validate` is a legal ID

---

## 3. Directory Scanning Algorithms

### A04: `scan_extensions()` — Extension Directory Scanning

**Source**: protocol-spec §3.6 (symlinks §3.4, hidden files §3.5)

**Description**: Recursively scans the extensions root, discovers module files and derives their Canonical IDs. This is the core of `Registry.discover()`.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `extensions_root` | `String` | Extensions root directory path |
| `config` | `Object` | Scan configuration: `follow_symlinks`, `ignore_patterns`, `max_depth` |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `modules` | `List<(String, String)>` | List of `(file_path, canonical_id)` tuples |

**Preconditions:**

- None — a missing or non-directory root is reported by step 1

**Postconditions:**

- Every returned `canonical_id` passed format validation and conflict detection
- Hidden files, built-in ignored entries and `extensions.ignore_patterns` matches are excluded
- Every recorded file's canonical real path lies inside the canonical `extensions_root`, and each real file is recorded once

**Pseudocode:**

```text
Algorithm: scan_extensions(extensions_root, config)

Steps:
  1. If extensions_root doesn't exist or is not a directory → throw CONFIG_ERROR
  2. modules ← []
  3. Recursively traverse extensions_root:
     For each entry (file or directory):
       a. If entry name matches a built-in row of §3.5, or matches any
          config.ignore_patterns entry under A25 (§9.2.3) → skip
       b. If entry is a symbolic link:
          - If config.follow_symlinks == false → skip
          - Otherwise resolve it to its canonical real path. If that path is not
            inside the canonical extensions_root → skip and issue a warning
            (D-94). Otherwise continue with the resolved target (D-127).
       c. If entry is a directory:
          - If current depth >= config.max_depth (default 8) → skip and issue warning
          - If its canonical real path has already been visited → skip (D-127:
            this terminates a directory cycle and stops an aliased directory
            being discovered twice)
          - Otherwise → recurse into it
       d. If entry is a file and its extension belongs to supported_extensions:
          - real ← canonical real path of entry.path
          - If real has already been recorded → skip (D-127: recorded once)
          - canonical_id ← directory_to_canonical_id(real, extensions_root)
          - If canonical_id passes validation → append (entry.path, canonical_id) to modules
          - If validation fails → log warning
  4. Perform detect_id_conflicts batch detection on modules
  5. Return modules
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is the total number of filesystem entries (files + directories) |
| Space | O(m + d) | m is the number of modules (result list), d is maximum recursion depth (call stack) |

**Implementation Notes:**

- Built-in ignored entries (§3.5): names starting with `.` or `_`, `__pycache__/`, `node_modules/`, `*.pyc`. `extensions.ignore_patterns` adds to them and cannot remove one; entries are matched against the entry name with A25, case-sensitively
- `supported_extensions` depends on the SDK's language (`.py`; `.ts` / `.js`; `.rs`)
- `max_depth` defaults to 8 and is configurable in `[1, 16]`
- **Containment runs before the directory/file split** (step 3b), so a symlinked file and a symlinked directory are checked alike. Containment and de-duplication are both keyed on the canonical real path, and the module ID is derived from that path, never from whichever alias the traversal reached first
- The check is made when the scanner looks, not when the loader opens the file; see [Discovery TOCTOU](./security-considerations.md#28-discovery-toctou-ot9)

**Multi-root discovery (`extensions.roots`):**

When the configuration declares `extensions.roots`, it takes precedence over `extensions.root`, and each entry is scanned with A04. Every entry carries a namespace — explicit in the `{ root, namespace }` form, otherwise the last path segment of the root (`./plugins` → `plugins`) — and each ID the root yields is prefixed `namespace + "."`. Two entries claiming the same namespace are a configuration error, reported before anything is scanned. Conflict detection runs across the combined result. Pinned by fixture `multi_root_discovery`.

---

## 4. Schema-Related Algorithms

### A05: `resolve_ref()` — Schema $ref Reference Resolution

**Source**: protocol-spec §4.11

**Description**: Resolves `$ref` references in JSON Schema, supporting local references, cross-file references, and Canonical ID references. **MUST** reject circular references — a `$ref` → `$ref` chain that never reaches a schema body — and **MUST** preserve self-references as lazy `$ref` nodes. protocol-spec §4.15 ("Self-reference vs. circular reference") is the sole authority on which re-entry is which.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `ref_string` | `String` | `$ref` value (e.g., `"./common/error.schema.yaml#/definitions/ErrorDetail"`) |
| `current_file` | `String` | Current Schema file path |
| `schemas_dir` | `String` | schemas root directory |
| `visited_refs` | `Set<String>` | Refs already on the resolution stack. Seeded by the caller with the aliases of the document being resolved (`#`, `#/`, and the document's own `$id`), so a reference naming that document is lazy from the **first** encounter |
| `depth` | `Integer` | `$ref` hops taken so far (starts at 0) |
| `from_ref_chain` | `Boolean` | True when the previous node was itself a bare `$ref`, i.e. no schema body was traversed between the two. False on any structural descent |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `resolved_schema` | `Object` | Resolved Schema object, or the `$ref` node unchanged when the reference is a self-reference |

**Preconditions:**

- `ref_string` is not empty

**Postconditions:**

- The returned Schema object contains no unresolved `$ref`s **except** self-references, which are preserved verbatim as lazy `$ref` nodes for the converter to bind at validation time
- No `$ref` → `$ref` cycle remains: such a chain raises `SCHEMA_CIRCULAR_REF` instead of returning
- `depth` never exceeded `schema.max_ref_depth`

**Pseudocode:**

```text
Algorithm: resolve_ref(ref_string, current_file, schemas_dir, visited_refs,
                       depth, from_ref_chain)

Steps:
  1. If ref_string ∈ visited_refs:
     a. If from_ref_chain → throw SCHEMA_CIRCULAR_REF error
     b. Otherwise → return { "$ref": ref_string } unchanged, preserving any
        sibling keys (legal self-reference — bind lazily, do not inline)
  2. If depth >= schema.max_ref_depth → throw SCHEMA_MAX_DEPTH_EXCEEDED error
  3. visited_refs ← visited_refs ∪ {ref_string}
  4. Parse ref_string into (file_part, json_pointer):
     a. If starts with "#" → file_part = current_file, json_pointer = ref_string[1:]
        Resolve the pointer against the FILE ROOT first; if it does not resolve
        there, fall back to the schema node being resolved (the `input_schema` /
        `output_schema` document itself) (D-104). The fallback is available ONLY
        while resolution is still inside the document the schema node belongs
        to; once a reference has been followed into another document, a local
        pointer that does not resolve in THAT document MUST throw
        SCHEMA_NOT_FOUND (D-124).
     b. If contains "#" → split by "#" into file_part and json_pointer
     c. If starts with "apcore://" → convert to file path under schemas_dir
     d. Otherwise → file_part is relative to current_file's directory
  5. schema_doc ← load and parse YAML/JSON file corresponding to file_part
  6. resolved ← locate target node by json_pointer in schema_doc
  7. If resolved is itself a bare $ref → recursively call resolve_ref(...) with
     depth + 1 and from_ref_chain = True
  8. Walk resolved's children; for each nested $ref reached by structural
     descent call resolve_ref(...) with depth + 1 and from_ref_chain = False
  9. Return resolved
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(d) | d is reference depth |
| Space | O(d) | visited_refs set size + recursive call stack |

**Implementation Notes:**

- Implementations **SHOULD** limit maximum reference depth to 32 (configurable via `schema.max_ref_depth`). Exhausting the cap raises `SCHEMA_MAX_DEPTH_EXCEEDED`, which is a **distinct** condition from an actual cycle — do not conflate it with `SCHEMA_CIRCULAR_REF`
- **Depth is consumed by `$ref` hops only.** Steps 7 and 8 both increment `depth` because both follow a reference; ordinary structural descent into `properties` or `items` does not
- **`from_ref_chain` is the whole discriminator.** A reference re-entered after descending through a schema body consumes one level of the *instance* per hop, so resolution terminates as soon as the data does — that is the recursive-structure shape JSON Schema 2020-12 §8.2.3 sanctions, and it **MUST NOT** raise. A reference re-entered along a chain of bare `$ref`s consumes no instance and cannot terminate — that **MUST** raise
- Reference formats come in three types:
  - Local reference: `#/definitions/ErrorDetail`
  - Relative path reference: `./common/error.schema.yaml#/definitions/ErrorDetail`
  - Canonical ID reference: `apcore://common.types.error/ErrorDetail`
- `apcore://` protocol conversion rule: `apcore://{canonical_id}/{pointer}` → `{schemas_dir}/{canonical_id}.schema.yaml#/{pointer}`
- JSON Pointer follows RFC 6901 specification
- **Both local-pointer layouts are supported** (D-104): definitions at the top level of the schema file beside `input_schema` (file root), and `$defs` nested inside `input_schema` (schema node). The two lookups cannot collide — a pointer either resolves at the file root or it does not
- **The node fallback is per document, not per resolver** (D-124). Carry it with each hop rather than on the resolver object; otherwise a pointer inside an external schema binds to a same-named definition in the calling module's schema
- **Sibling keys beside a `$ref` are preserved** and applied to the resolved node (D-98). §10.6 reads `x-sensitive` from the resolved schema, so dropping a sibling `x-sensitive` leaks the field into logs

---

## 5. Module-Related Algorithms

### A06: `resolve_entry_point()` — Module Entry Point Resolution

**Source**: protocol-spec §5.2

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

```text
Algorithm: resolve_entry_point(meta_yaml, file_path, language)

Steps:
  1. If meta_yaml exists and contains entry_point field:
     a. Parse format "filename:ClassName"
     b. Return { file: filename, class_name: ClassName }
  2. Otherwise, auto-infer:
     a. file ← filename from file_path (without extension)
     b. class_name ← convert file from snake_case to PascalCase
     c. If the language loads module files at runtime: candidates ← the classes the
        file provides that satisfy the Module interface (protocol-spec §5.6 — duck-typed,
        no base class is required)
     d. If exactly one candidate → return that class
     e. If more than one candidate → throw MODULE_LOAD_ERROR (ambiguous entry point)
     f. If no candidate → throw MODULE_LOAD_ERROR (no module class in the file)
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
- Candidates are found by reflection over the loaded file (Python `inspect`, ECMAScript module exports) and matched by the Module interface's shape, not by inheritance
- A language that cannot load a module file at runtime (Rust) resolves only the entry-point name — from `entry_point`, or by step 2b — and obtains the module from an application-supplied factory keyed by that name (protocol-spec §5.2)

---

### A07: `resolve_dependencies()` — Dependency Topological Sort

**Source**: protocol-spec §5.3

**Description**: Uses topological sort (Kahn's algorithm) to resolve module loading order. **MUST** detect circular dependencies, and **MUST** distinguish a real cycle from a sort that stalls for another reason (§5.15.2).

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
- A real cycle throws `CIRCULAR_DEPENDENCY` with the actual cycle path; a stall with no cycle throws `MODULE_LOAD_ERROR` naming the blocked modules

**Pseudocode:**

```text
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
     - cycle ← a dependency cycle among remaining (back-edge search), or null
     - If cycle is not null → throw CIRCULAR_DEPENDENCY with cycle_path = cycle
     - Otherwise → throw MODULE_LOAD_ERROR naming remaining; MUST NOT report
       CIRCULAR_DEPENDENCY and MUST NOT fabricate a cycle_path (D-79)
  7. Return load_order
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(V + E) | V is number of modules, E is number of dependency relationships |
| Space | O(V + E) | Storage for dependency graph and in-degree table |

**Implementation Notes:**

- A cycle error carries the real cycle path (e.g., `A → B → C → A`). A sort can also stall with no cycle — e.g. a batch member depends on a module that is registered but not in this batch, so its in-degree never reaches zero — and that case needs the opposite fix (add the missing dependency, not break an edge)
- A missing required dependency throws `DEPENDENCY_NOT_FOUND`; a missing optional dependency (`optional: true`) is skipped (§5.15.2)
- Version constraints (`version: ">=1.0.0"`) are checked before building the dependency graph; a required dependency whose registered version does not satisfy the constraint throws `DEPENDENCY_VERSION_MISMATCH`, an optional one is skipped with a warning (§5.15.2)
- Modules with same in-degree have no guaranteed order, implementations **may** sort by ID alphabetically for stable output

---

## 6. ACL-Related Algorithms

### A08: `match_pattern()` — ACL Pattern Matching

**Source**: protocol-spec §6.2

**Description**: Matches one ACL rule pattern (or one `match_modules` pattern, §5.16) against a module Canonical ID. `*` is the only metacharacter.

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

```text
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
- **Every other character is a literal**, `?`, `[`, `]`, `{`, `}` and `\` included. A pattern containing `?` can never match (§2.7 forbids `?` in a module ID); loading such a rule warns and `validate_rules()` reports it, but its meaning is unchanged (§6.2.2)
- A08 matches one pattern string. The compound forms `["$or", …]` and `["$not", p]` are array-level operators handled before A08 is called (§6.2.1)
- If pattern starts with `*`, first segment is empty, matching starts from beginning of module_id
- Exact match (no wildcard) should prioritize string comparison to avoid unnecessary splitting
- Match results can be cached for performance (pattern and module_id combination as cache key)

---

### A09: `evaluate_acl()` — ACL Rule Evaluation

**Source**: protocol-spec §6.3 (conditions §6.1.1, compound pattern arrays §6.2.1, approval requirement §6.9)

**Description**: Evaluates an ordered rule list to decide whether a call is allowed. This is the core algorithm of the ACL engine.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `caller_id` | `String \| null` | Caller module ID (`null` means external call) |
| `target_id` | `String` | Callee module ID |
| `rules` | `List<Rule>` | Rules in definition order |
| `default_effect` | `String` | Default policy (`"allow"` \| `"deny"`) |
| `context` | `Context \| null` | Execution context, used for condition evaluation |

Where `Rule` structure is (the key set is closed, §6.1.5):

```text
Rule {
  callers:     List<String>,        // pattern array (§6.2.1)
  targets:     List<String>,        // pattern array (§6.2.1)
  effect:      "allow" | "deny",
  description: String,              // SHOULD
  conditions:  Object | null,       // MAY (§6.1)
  approval:    "required" | "not_required" | null   // MAY (§6.1.6)
}
```

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `decision` | `Object` | `{ effect: "allow" \| "deny", matched_rule: Rule \| null }` |

**Preconditions:**

- `target_id` conforms to Canonical ID format
- Every rule passed §6.1.4's structural precheck at load (an arity or key fault is rejected with `ACLRuleError` before evaluation)

**Postconditions:**

- Returns a deterministic decision
- An unevaluable condition never lets an `allow` rule grant

**Pseudocode:**

```text
Algorithm: evaluate_acl(caller_id, target_id, rules, default_effect, context)

Steps:
  1. effective_caller_id ← caller_id ?? "@external"
  2. For each rule ∈ rules (in definition order):
     a. caller_matched ← false
        For each pattern ∈ rule.callers:
          If pattern is "@external" and caller_id is null → caller_matched ← true; break
          If pattern is "@system" and context.identity.type == "system" → caller_matched ← true; break
          If match_pattern(pattern, effective_caller_id) → caller_matched ← true; break
     b. target_matched ← false
        For each pattern ∈ rule.targets:
          If match_pattern(pattern, target_id) → target_matched ← true; break
     c. If caller_matched and target_matched:
        If rule.conditions is not empty:
          verdict ← evaluate_conditions(rule.conditions, context)
              // verdict ∈ { SATISFIED, UNSATISFIED, UNEVALUABLE }   (§6.1.1)
          If verdict is UNSATISFIED → continue
          If verdict is UNEVALUABLE:
            record handler_error on the audit entry and warn (§6.1.1)
            If rule.effect == "deny" → Return { effect: "deny", matched_rule: rule }
            Else                     → continue          // an allow rule MUST NOT grant
        → Return { effect: rule.effect, matched_rule: rule }
  3. Return { effect: default_effect, matched_rule: null }
```

The per-pattern loops above show plain pattern arrays. An array whose index 0 is `$or` or `$not` is evaluated as that operator over its operands (§6.2.1).

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(R * P * M) | R is number of rules, P is average patterns per rule, M is match_pattern complexity |
| Space | O(1) | No sort buffer; rules iterated in stored order |

**Implementation Notes:**

- When `caller_id` is `null`, it **MUST** be replaced with `"@external"`
- Rules are evaluated in **definition order**, first match wins. There is no `priority` field and no deny tie-break (D-60); `id`, `actions` and `priority` are reserved keys and are rejected at load (§6.1.5)
- A rule whose `callers` or `targets` array is empty, or otherwise outside §6.2.1's shape, is rejected with `ACLRuleError` at every entry point (§6.5)
- `evaluate_conditions` has three outcomes, not two. An implementation whose evaluator returns a boolean **MUST** carry UNEVALUABLE some other way (§6.3)
- When `conditions` are present but the call carries no context, the rule does not match; this is not UNEVALUABLE (§6.5)
- Modules calling themselves also need ACL checking
- The approval requirement is a separate result from the decision; see §6.1.6 and §6.9 for how a matched rule's `approval` composes with pending requirements

---

### A10: `calculate_specificity()` — Pattern Specificity Scoring

**Source**: protocol-spec §6.4

**Description**: Calculates the specificity score of a module-ID pattern. Higher score means more specific pattern. ACL evaluation itself is first-match-wins and does not use it; `ExecutionPolicy` rule selection does (§7.9.1).

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

```text
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
- This algorithm is **SHOULD** level for the ACL (§6.4) and required wherever `ExecutionPolicy` is implemented (§7.9.1)
- Beyond policy selection, specificity is useful for ACL debugging and conflict analysis

---

## 7. Error Handling Algorithms

### A11: `propagate_error()` — Error Propagation

**Source**: protocol-spec §8.3

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

```text
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

**Source**: protocol-spec §8.4

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

```text
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

- The reserved framework prefixes are the fourteen of §8.4: `ACL_`, `APPROVAL_`, `BINDING_`, `CALL_`, `CIRCULAR_`, `CONFIG_`, `DEPENDENCY_`, `ERROR_CODE_`, `FUNC_`, `GENERAL_`, `MIDDLEWARE_`, `MODULE_`, `SCHEMA_`, `VERSION_`. A module code with a reserved prefix is rejected, as is a one-off code equal to an exact framework code (fixture `error_codes`)
- Module error code naming **SHOULD** follow `{MODULE_PREFIX}_{ERROR_NAME}` format
- This detection should be executed during framework startup, report error immediately upon finding conflict

---

## 8. Configuration-Related Algorithms

### A12: `validate_config()` — Configuration Validation

**Source**: protocol-spec §9.3

**Description**: Validates the configuration during framework startup: required fields are present in the declared document, types are correct, and constraints are satisfied.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `config` | `Object` | Merged configuration object (env + file + defaults) |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `validated_config` | `Object` | Validated configuration object |

**Preconditions:**

- Configuration object has been merged by priority (env > file > defaults); the declared document (before defaults) is still available for step 1

**Postconditions:**

- All required fields are populated
- All field types are correct
- All constraints are satisfied

**Pseudocode:**

```text
Algorithm: validate_config(config)

Steps:
  1. For each required field — `version` and `project.name`, the only two keys
     with no canonical default (§9.1):
     If missing from the DECLARED document (before the default table is merged)
       → throw CONFIG_INVALID with the missing field path
     A key that carries a default in defaults.schema.json is NEVER required;
     checking it after merging defaults is a no-op and MUST NOT be relied on.
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
- Environment variable naming convention: `APCORE_{SECTION}_{KEY}`, all uppercase, hyphens converted to underscores (§9.2; namespace mode §9.8)
- `acl.default_effect` in `apcore.yaml` is validated but inert; the effective `default_effect` is the ACL file's own (§6.1)
- Validation errors should collect all errors before reporting, rather than stopping at first error
- The namespace-mode variant is A12-NS (§9.10)

---

## 9. Observability Algorithms

### A13: `redact_sensitive()` — Sensitive Data Redaction

**Source**: protocol-spec §10.6 (configured rules §10.6.1)

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

```text
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

- Redaction **MUST** operate on a copy and **MUST NOT** modify the original data
- The replacement is a constant token (`"***REDACTED***"` by default) and must not leak the length of the original value
- `x-sensitive` fields in nested objects must also be recursively processed
- Each element in array needs independent redaction (if items schema contains `x-sensitive`)
- If `data` contains fields not defined in Schema, A13 does not redact them; the configured rules below may
- `schema` is the **resolved** schema. A05 preserves `x-sensitive` written beside a `$ref` (D-98) and never binds an external document's pointer to the caller's definitions (D-124); either defect turns a sensitive field into plaintext here
- A13 is one of three rules applied as a union: `x-sensitive` (A13), `obs.redaction.sensitive_keys` against field names, and `obs.redaction.regex_patterns` against string values. The union applies at log emission **and** at the executor's input/output capture point; with no configuration the default `sensitive_keys` list still applies. The correlation fields `trace_id`, `span_id`, `caller_id`, `module_id` and `target_id` are never redacted (§10.6.1)
- Setting `obs.redaction.sensitive_keys` replaces the default list rather than extending it

---

## 10. Version Management Algorithms

### A14: `negotiate_version()` — Version Negotiation

**Source**: protocol-spec §13.3

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

```text
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
- The three SDKs export `negotiate_version()` but do not yet call it when loading configuration or schema files ([conformance.md §7](./conformance.md#7-known-deviations))

---

### A15: `migrate_schema()` — Schema Migration

**Source**: protocol-spec §13.4

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

```text
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

**Source**: protocol-spec §11.7

**Description**: Loads extension point implementations by priority and strategy. Supports three strategies: `first_success` (first successful takes effect), `all` (execute all), `fallback` (fallback chain).

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `config` | `Object` | Framework configuration |
| `extension_points` | `Map<String, List<ExtensionImpl>>` | Mapping from extension point name → implementation list |

Where `ExtensionImpl` structure is:

```text
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

```text
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

- The SDKs' `ExtensionManager` exposes six extension points — `discoverer`, `middleware`, `acl`, `span_exporter`, `module_validator`, `approval_handler` (§11.3). No SDK implements A16's priority ordering or chaining strategies yet ([conformance.md §7](./conformance.md#7-known-deviations))
- `first_success` strategy: stop when loading succeeds, suitable for Schema loading (cache → filesystem)
- `all` strategy: all implementations execute, suitable for notification extensions
- `fallback` strategy: similar to `first_success`, but continues trying next on failure
- Extension loading failure should log warning, but should not cause framework startup failure (unless no available implementation and no default implementation)

---

## 12. Function-based Module Definition and Binding Algorithms

### A18: generate_schema_from_function — Generate Schema from Function Signature

**Source**: protocol-spec §5.11.4

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

```text
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

**Source**: protocol-spec §5.12.3

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

```text
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

**Source**: pipeline step `call_chain_guard` (Step 2); limits `executor.max_call_depth` / `executor.max_module_repeat` (protocol-spec §9.1.1); contract in [features/call-chain-guard.md](../features/call-chain-guard.md). protocol-spec carries no pseudocode for A20; this section is the pseudocode of record.

**Description**: Three-layer call chain protection executed by the executor on each call: depth limit, cycle detection, frequency detection. Cycle detection rejects any re-entry separated by another module (A→B→A); frequency detection bounds direct self-recursion (A→A→A).

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `module_id` | `String` | Target module ID being called |
| `call_chain` | `List<String>` | Call chain in current Context, **already including `module_id` as its last element** (appended by `Context.child()`) |
| `max_call_depth` | `Integer` | Maximum call depth (default 32). **MUST** be >= 1 |
| `max_module_repeat` | `Integer` | Maximum occurrences of same module (default 3). **MUST** be >= 1 |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| — | `void` | Returns nothing if check passes; throws corresponding error if fails |

**Preconditions:**

- `call_chain` is automatically managed by Executor, cannot be tampered by module code
- `module_id` has passed Canonical ID format validation

**Postconditions:**

- If no exception thrown, the call described by `call_chain` may proceed
- If exception thrown, call chain is not modified

!!! warning "The chain includes the target"
    `call_chain` arrives with `module_id` already appended, so a well-formed
    single call has `call_chain[-1] == module_id`. Treating the chain as
    *excluding* the target inverts every check: `[a, b, c]` with
    `module_id = "c"` would read as a cycle, and a 32-element chain would
    exceed a limit of 32. `conformance/fixtures/call_chain.json` pins the
    correct reading in `valid_chain`, `single_element`,
    `self_call_not_circular` and `default_depth_32_ok`.

**Pseudocode:**

```text
Algorithm: guard_call_chain(module_id, call_chain, max_call_depth, max_module_repeat)

Precondition: call_chain[-1] == module_id (appended by Context.child())

Steps:
  0. Limit floors (before inspecting the chain):
     If max_call_depth < 1 OR max_module_repeat < 1:
       → throw GENERAL_INVALID_INPUT (D-84)

  1. Depth check:
     If len(call_chain) > max_call_depth:
       → throw CALL_DEPTH_EXCEEDED {
           module_id: module_id,
           current_depth: len(call_chain),
           max_depth: max_call_depth,
           call_chain: call_chain
         }
     Note the comparison is strict: a chain of exactly max_call_depth passes.

  2. Cycle detection:
     prior ← call_chain[0 .. len(call_chain)-2]   // everything but the target
     If module_id ∈ prior:
       i ← last index of module_id in prior
       If i < len(prior) - 1:                     // other modules called since
         → throw CIRCULAR_CALL {
             module_id: module_id,
             call_chain: call_chain,
             cycle_start: i
           }
     A repeat with nothing in between is a self-call, not a cycle: [a, a] is
     permitted, [a, b, a] is not.

  3. Frequency detection:
     repeat_count ← count of module_id in call_chain   // full chain
     If repeat_count > max_module_repeat:
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

- Execution order of the three checks **MUST** be: depth → cycle → frequency (depth check is cheapest, should execute first). The step 0 limit floors precede all of them.
- `max_module_repeat` default value is 3, configurable via `apcore.yaml`'s `executor.max_module_repeat`, range `[1, 100]`
- Step 2 cycle detection covers direct cycle (A→B→A) and indirect cycle (A→B→C→A). It does **not** flag an immediate self-repeat (A→A), which step 3 governs instead
- Both step 1 and step 3 compare strictly (`>`), so a chain sitting exactly on a limit passes. `default_depth_32_ok` and `frequency_within_limit` in `conformance/fixtures/call_chain.json` pin this boundary
- Because step 2 rejects every repeat separated by another module, a chain that reaches step 3 with `count > 1` is direct self-recursion; step 3 bounds how deep a module may recurse into itself
- If step 2 already threw `CIRCULAR_CALL`, step 3 won't execute (short-circuit)
- All errors **MUST** carry complete `call_chain` copy for debugging and observability

**Configuration Reference:**

```yaml
# apcore.yaml
executor:
  max_call_depth: 32          # Maximum call depth
  max_module_repeat: 3        # Maximum occurrences of same module
```

**Examples:**

```text
call_chain: ["orchestrator.ai_planner", "executor.b", "executor.c", "executor.b"]
module_id:  "executor.b"
→ prior chain holds "executor.b" at index 1, with "executor.c" after it
→ CIRCULAR_CALL (step 2)

call_chain: ["orchestrator.ai_planner", "executor.retry", "executor.retry", "executor.retry", "executor.retry"]
module_id:  "executor.retry"
→ no other module between the repeats, so step 2 passes
→ count("executor.retry") == 4 > max_module_repeat (3)
→ CALL_FREQUENCY_EXCEEDED (step 3)
```

---

## 14. Algorithm Dependency Relationships

The following diagram shows the calling/dependency relationships between algorithms:

```text
scan_extensions() ──────────────────┐
  ├── match_glob()  (ignore_patterns)│
  ├── directory_to_canonical_id()   │
  └── detect_id_conflicts()         │
                                    ▼
                              Registry.discover()
                                    │
                              ┌─────┴──────┐
                              ▼            ▼
                    resolve_entry_point()  resolve_dependencies()

validate_config()  ←── framework startup
resolve_ref()      ←── Schema loading

Executor pipeline (per call)
  ├── guard_call_chain()            Step 2
  ├── evaluate_acl()                Step 4
  │     └── match_pattern()
  ├── input/output validation       Steps 7, 9
  ├── redact_sensitive()            capture point and log output
  ├── enforce_timeout()             around Step 8
  ├── deep_merge_chunks()           stream() only
  └── propagate_error()             on error

negotiate_version()             ←── configuration / Schema loading
migrate_schema()                ←── Schema version mismatch
load_extensions()               ←── framework startup
detect_error_code_collisions()  ←── framework startup (after all modules loaded)
safe_unregister()               ←── unregister / hot reload

generate_schema_from_function() ←── module() registration / Binding auto_schema
resolve_target()                ←── Binding file loading
to_strict_schema()              ←── export_schema(strict=true)
calculate_specificity()         ←── ExecutionPolicy rule selection
```

---

## 15. Concurrency Model Related Algorithms

### A21: `safe_unregister()` — Hot-reload Safe Unregistration

**Source**: protocol-spec §12.7.4

**Purpose**: Unregister a module that may be executing, without racing in-flight calls or leaking resources.

**Signature**:

```text
safe_unregister(module_id: string, registry: Registry) → (bool, state | null)
```

**Input**:

- `module_id`: Module ID to unregister
- `registry`: Registry instance

**Output**:

- `true, state`: Unloaded cleanly; `state` is what `on_suspend()` returned (or `null`)
- `false, null`: The wait timed out and the module was force-unloaded

**Preconditions**:

- Registry is initialized

**Postconditions**:

- New calls to `module_id` throw `MODULE_NOT_FOUND` from step 2 onward
- Calls that had already started run to completion (or until the timeout)
- `on_unload()` has been called

**Pseudocode**:

```text
Algorithm: safe_unregister(module_id, registry)

Steps:
  1. Mark module as "unloading" state
  2. Remove from registry (new call() will throw MODULE_NOT_FOUND)
  3. Wait for all executing calls to complete:
     - Maintain reference count (number of executing calls)
     - Block until count reaches zero or timeout (default 5 seconds)
  4. Call on_suspend() hook (if implemented) → save returned state
  5. Call on_unload() hook
  6. Release module instance

Return:
  - If successfully unloaded → true, state (dict or null)
  - If timeout → Log ERROR, force unload, return false, null
```

**Complexity Analysis**:

- Time complexity: O(1) + wait time (up to the timeout)
- Space complexity: O(1)

**Implementation Notes**:

1. **Reference count maintenance**: increment when a call starts and decrement when it ends, whether it succeeds or fails. Use atomic operations or a lock.
2. **Timeout handling**: the default wait is 5 seconds. On timeout, log the module ID and the remaining reference count.
3. **Hook failures**: an `on_unload()` that throws is logged at ERROR and the unload continues (§5.15.3).
4. **Concurrency table**: `unregister()` of an absent or already-removed ID succeeds silently; `call()` racing `unregister()` completes if it had started and otherwise throws `MODULE_NOT_FOUND` (§12.7.4).

---

### A22: `enforce_timeout()` — Timeout Enforcement

**Source**: protocol-spec §12.7.5

**Purpose**: Ensure module execution completes within the configured time, using cooperative cancellation first and forced termination where the language supports it.

**Signature**:

```text
enforce_timeout(module_id: string, inputs: dict, context: Context, timeout_ms: int) → dict | error
```

**Input**:

- `module_id`: Module ID
- `inputs`: Input parameters
- `context`: Execution context
- `timeout_ms`: Timeout duration (milliseconds); `0` disables the per-module limit (`executor.default_timeout`)

**Output**:

- Success: Module output (dict)
- Failure: `MODULE_TIMEOUT` error

**Preconditions**:

- Module is registered

**Postconditions**:

- If execution completes within the limit, returns its output
- If the limit is reached, throws `MODULE_TIMEOUT`

**Pseudocode**:

```text
Algorithm: enforce_timeout(module_id, inputs, context, timeout_ms)

Steps:
  1. Start timer (from first before() middleware)
  2. Concurrent execution:
     a. Main task: execute_with_middleware(module_id, inputs, context)
     b. Timeout monitor: sleep(timeout_ms)
  3. If main task completes first → Cancel timer, return result
  4. If timeout triggers first:
     - Send cancellation signal (cooperative)
     - Wait maximum grace_period (default 5 seconds)
     - If still hasn't exited → Forcibly terminate (if supported)
     - Throw MODULE_TIMEOUT error

Return:
  - Success → Module output
  - Failure → MODULE_TIMEOUT error
```

**Complexity Analysis**:

- Time complexity: O(1) + module execution time (up to `timeout_ms` + grace period)
- Space complexity: O(1)

**Implementation Notes**:

1. **Cooperative cancellation first**: a long-running module checks `context.cancel_token` and exits when it is cancelled.
2. **Forced termination** may leak resources (open files, held locks). Use it only after cooperative cancellation fails, and log ERROR with `module_id` and the timeout after doing so.
3. **Two limits**: the per-module limit (`executor.default_timeout`, or the module's declared `resources.timeout`) and the call-chain limit (`executor.global_timeout`), which covers before + execute + after for the whole chain. `0` disables either.
4. **Grace period**: no SDK currently waits the step 4 grace period; see [conformance.md §7](./conformance.md#7-known-deviations).

---

## 16. Schema Export Algorithm

### A23: `to_strict_schema()` — Strict Mode Schema Conversion

**Source**: protocol-spec §4.16

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

```text
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
  2. If node declares an object schema — node contains properties AND
     (node has no "type" keyword at all
      OR node.type declares "object", in either the string form "object"
         or the array form ["object", "null"]):
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
        If prop does not contain type (e.g., pure $ref or a composition):
          prop ← { anyOf: [prop, { type: "null" }] }  # Wrap as anyOf, never oneOf
     f. node.required ← sort(all_property_names)   # ALL fields, lexicographically
        # Sorted, not insertion-ordered: the output has to be byte-identical
        # across SDKs whose object types iterate in different orders.
        # Compare by Unicode code point (not UTF-16 code unit) so a property
        # name outside the BMP orders the same everywhere.
  3. Recursively process nested structures:
     a. If node.properties exists:
        For each (key, prop) ∈ node.properties:
          node.properties[key] ← convert_to_strict(prop)
     b. If node.items exists:
        node.items ← convert_to_strict(node.items)
     c. If node.prefixItems exists:
        For each entry ∈ node.prefixItems:
          entry ← convert_to_strict(entry)
     d. For keyword ∈ ["oneOf", "anyOf", "allOf"]:
        If node[keyword] exists:
          For each sub_schema ∈ node[keyword]:
            sub_schema ← convert_to_strict(sub_schema)
     e. For defs_key ∈ ["definitions", "$defs"]:
        If node[defs_key] exists:
          For each (name, defn) ∈ node[defs_key]:
            node[defs_key][name] ← convert_to_strict(defn)
  4. Return node
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(n) | n is total number of nodes in Schema (recursive traversal) |
| Space | O(n) | deep_copy space overhead |

**Implementation Notes:**

- Conversion **MUST** be performed on a copy and **MUST NOT** modify the original Schema
- `x-llm-description` **SHOULD** first replace corresponding field's `description` before stripping (see §4.3), then execute `strip_extensions()`
- Pure `$ref` nodes (no `type`) use **`anyOf`** wrapping when making nullable, rather than directly adding `type`. `anyOf`, not `oneOf`: OpenAI structured outputs — the consumer strict mode exists to feed — accepts only `anyOf` as the nullable-union spelling. A `oneOf` the module author wrote is preserved untouched inside the wrapper; rewriting it would drop the exclusivity their contract asserts
- An author-written `oneOf` / `anyOf` on an optional property is **wrapped** by the same rule, never appended to. There is exactly one nullable spelling — `{anyOf: [<original>, {type: "null"}]}` — and pushing a `null` branch into the author's union rewrites the contract they declared (for `oneOf`, it also changes the exclusivity count the validator counts branches against)
- **`properties` alone identifies an object schema.** A node carrying `properties` **MUST** be hardened when it has no `type` keyword at all, or when its `type` declares `"object"` in either the string form (`"object"`) or the array form (`["object", "null"]` — which is exactly what the nullable wrapping in step 2e produces for an optional nested object). A node with `properties` and no hardening is rejected by OpenAI structured outputs. Conversely, `properties` sitting beside a **non-object** `type` is inert ([type-mapping §17.1](./type-mapping.md#171-general-rules) R2) and **MUST NOT** be hardened
- If `additionalProperties` already exists and is `true`, **MUST** change to `false`
- For fields already in `type: ["string", "null"]` form, should not add `"null"` again
- `object` inside `items` also need recursive processing, and so does an `object` at a `prefixItems` tuple position — the Draft 2020-12 tuple form is not an exception
- `default` values are invalid in Strict Mode (AI won't auto-fill), so remove them as well
- **`required` is emitted sorted, and array order is significant.** Step 2f replaces `required` wholesale with every property name in lexicographic order, discarding the author's original ordering. Insertion order would make the output depend on each language's object-iteration order, so the fixture below could not assert equality. Sort by Unicode **code point**: JavaScript's default `Array.prototype.sort()` compares UTF-16 code units and orders supplementary-plane names differently from Python's `sorted()` and Rust's `Vec<String>::sort()`, so a JS implementation needs an explicit code-point comparator
- Conformance to this algorithm is asserted by `conformance/fixtures/schema_strict_conversion.json`, which pins the exact strict schema all three SDKs must emit for each input

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
required: [cc, config, to]     # every property, sorted (step 2f)
additionalProperties: false
```

---

## 17. Stream Aggregation Algorithm

### A24: `deep_merge_chunks()` — Stream Chunk Aggregation {#a24-stream-chunk-aggregation}

**Source**: protocol-spec §12.2, Streaming Execution Protocol. protocol-spec points here for the pseudocode. Pinned by `conformance/fixtures/stream_aggregation.json`.

**Purpose.** Aggregate the chunks yielded by a streaming module's `stream()` generator into the final output dict that the executor validates against `output_schema`.

```text
Algorithm: deep_merge_chunks(chunks, max_depth=32)

Input:
  chunks    — Ordered list of dict objects yielded by module.stream()
  max_depth — Maximum recursion depth (stream.max_merge_depth, default 32)

Output:
  result — A single dict that is the recursive deep-merge of all chunks

Steps:
  1. result ← {}
  2. For each chunk in chunks (in order):
       deep_merge(result, chunk, depth=0, max_depth=max_depth)
  3. Return result

Helper: deep_merge(base, override, depth, max_depth)
  1. If depth >= max_depth:
       For each (key, value) in override:
         base[key] ← value        (assign wholesale; do NOT recurse further)
       Return
  2. For each (key, value) in override:
       a. If key NOT in base:
            base[key] ← value
       b. Else if base[key] is dict AND value is dict:
            deep_merge(base[key], value, depth+1, max_depth)
       c. Else:
            base[key] ← value   (overwrite — arrays REPLACE, primitives overwrite)
```

**Properties:**

- Arrays are replaced (not concatenated) at matching keys.
- `null` overwrites (does not delete) the previous value.
- Recursion is depth-capped to prevent stack exhaustion via adversarial input.
- At the depth cap all three SDKs assign the override's values wholesale (right value wins): traversal stops, but no data is dropped (fixture case `deep_merge_depth_cap_right_wins`).

**Reference implementations:**

- apcore-python `executor.py` — `_deep_merge()`
- apcore-typescript `executor.ts` — `deepMergeChunk()`
- apcore-rust `executor.rs` — `deep_merge_chunks_checked()` and `deep_merge_value()`

**Notes:**

- Implementations **MUST** reject a non-object chunk (array, string, number, boolean, null) *before* delivering it to the consumer, raising `InvalidInputError` with `code=GENERAL_INVALID_INPUT` and `details.code = STREAM_CHUNK_NOT_OBJECT`, so the invalid chunk is never yielded. See [features/streaming.md](../features/streaming.md) (D-58).
- The merge is performed in-place on an accumulator dict; iteration over the chunk stream is single-pass.

---

## 18. Pattern Matching Algorithm

### A25: `match_glob()` — Portable Glob Matching

**Source**: protocol-spec §9.2.3

**Description**: Matches a **pattern-valued** configuration value or system-module input
against a name — a filename, a field name, an event type, a module ID. A25 is the matcher
for every pattern-valued value in this specification **except** ACL rule patterns and
`match_modules`, which are module-ID matching and use A08.

Naming the algorithm, not only the syntax, is what keeps the SDKs in agreement: host-language
glob libraries each implement a different dialect.

**Input Parameters:**

| Parameter | Type | Description |
|------|------|------|
| `pattern` | `String` | Pattern-valued string. **Every** string is valid; there is no parse error. |
| `value` | `String` | The name to test |

**Output:**

| Return Value | Type | Description |
|--------|------|------|
| `matched` | `Boolean` | Whether the **whole** value matches the pattern |

**Metacharacters — exactly two:**

| Character | Meaning |
|---|---|
| `*` | zero or more characters, including `.` and `/` |
| `?` | exactly one character |

Every other character is a literal, **including `[` `]` `{` `}` `\` `!` `^` `-`**. There is
no escape character.

**Preconditions:** none. A25 accepts any pair of strings.

**Postconditions:**

- The match is **anchored**: the pattern must cover the entire value, not a substring of it.
- Deterministic: same inputs always return the same result.
- Total: never raises, never rejects a pattern.

**Pseudocode:**

```text
Algorithm: match_glob(pattern, value)

Steps:
  1. segments ← split(pattern, "*")            # n = len(segments) >= 1
  2. If n == 1:
       → return match_exact(segments[0], value)
  3. If NOT match_prefix(segments[0], value) → return false
     pos ← len(segments[0])
  4. For i in 1 .. n-2:                        # interior segments, leftmost-first
       j ← smallest index >= pos such that
             match_exact(segments[i], value[j : j + len(segments[i])])
       If no such j → return false
       pos ← j + len(segments[i])
  5. last ← segments[n-1]
     If last == "" → return true               # pattern ended with "*"
     If len(value) - pos < len(last) → return false
     → return match_exact(last, value[len(value) - len(last) :])

  where
    match_exact(seg, text)  ≡ len(seg) == len(text) AND match_prefix(seg, text)
    match_prefix(seg, text) ≡ len(text) >= len(seg) AND
                              for every k in 0 .. len(seg)-1:
                                  seg[k] == "?" OR seg[k] == text[k]
```

**Complexity Analysis:**

| Dimension | Complexity | Description |
|------|--------|------|
| Time | O(m * n) | m is pattern length, n is value length |
| Space | O(m) | Storage for split segments |

**Implementation Notes:**

- **Do not delegate to the host library.** `fnmatch`, `pathlib.Path.glob`, the `glob` crate
  and a hand-translated `RegExp` each differ from A25 and from one another. A25 is roughly
  twenty lines per language, with no dependency.
- **Step 4 is leftmost-first and does not backtrack**, which is the same greedy strategy A08
  specifies. Stating the procedure — not only the syntax — is the point: two matchers can
  both honour `*` and `?` and still disagree on `a*a` against `aaa`.
- **Case sensitivity is not part of A25.** It compares characters exactly. A surface that is
  case-insensitive (§9.2.3's table marks them) folds **both** the pattern and the value
  before calling A25. Folding one side only is a silent bypass, not a partial fix.
- `?` matches exactly one character and therefore never matches the empty string:
  `a?` does not match `a`.
- Consecutive stars collapse naturally — `a**b` splits to `["a", "", "b"]` and the empty
  interior segment is a no-op — so `**` is not a distinct construct and needs no special case.
- Results may be cached per `(pattern, value)` pair; a compiled representation of the split
  is also safe to cache per pattern.

**Relationship to A08:**

| | A08 `match_pattern` | A25 `match_glob` |
|---|---|---|
| Matches | module IDs, in ACL rules and `match_modules` | filenames, field names, event types, `path_filter` |
| Metacharacters | `*` | `*` and `?` |
| Anchored | yes | yes |
| Interior search | leftmost-first, greedy | leftmost-first, greedy |

The two agree on every pattern whose only metacharacter is `*`. They are kept separate
because promoting `?` to a wildcard in A08 would widen ACL `allow` rules that are inert
today — a module ID cannot contain `?` (§2.7), so such a rule matches nothing — and an
authorization matcher must not widen silently. §6.2.2 requires that dead pattern to be
reported instead.

---

## 19. References

- [protocol-spec §2 — Naming Specification](./protocol-spec.md#2-naming-specification)
- [protocol-spec §3 — Directory Specification](./protocol-spec.md#3-directory-specification)
- [protocol-spec §4 — Schema Specification](./protocol-spec.md#4-schema-specification)
- [protocol-spec §5 — Module Specification](./protocol-spec.md#5-module-specification)
- [protocol-spec §5.11 — Function-based Module Definition](./protocol-spec.md#511-function-based-module-definition-function-based-module-definition)
- [protocol-spec §5.12 — External Schema Binding](./protocol-spec.md#512-external-schema-binding-external-schema-binding)
- [protocol-spec §6 — ACL Specification](./protocol-spec.md#6-acl-specification)
- [protocol-spec §7 — Approval System](./protocol-spec.md#7-approval-system)
- [protocol-spec §8 — Error Handling Specification](./protocol-spec.md#8-error-handling-specification)
- [protocol-spec §9 — Configuration Specification](./protocol-spec.md#9-configuration-specification)
- [protocol-spec §10 — Observability Specification](./protocol-spec.md#10-observability-specification)
- [protocol-spec §11 — Extension Mechanism](./protocol-spec.md#11-extension-mechanism)
- [protocol-spec §13 — Versioning](./protocol-spec.md#13-versioning)
