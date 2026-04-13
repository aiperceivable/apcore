# Access Control System

## Overview

Pattern-based Access Control List (ACL) with first-match-wins evaluation for module access control. The system enforces which callers may invoke which target modules, using wildcard patterns, special identity patterns (`@external`, `@system`), and optional conditions based on identity type, roles, and call depth. Configuration can be loaded from YAML files and hot-reloaded at runtime.

## Requirements

- Implement first-match-wins rule evaluation: rules are evaluated in order, and the first rule whose patterns match the caller and target determines the access decision (allow or deny).
- Support wildcard patterns for caller and target matching (e.g., `admin.*`, `*`), delegating to a shared pattern-matching utility.
- Handle special patterns: `@external` matches calls with no caller (external entry points), and `@system` matches calls where the execution context has a system-type identity.
- Support conditional rules with `identity_types` (identity type must be in list), `roles` (at least one role must overlap), and `max_call_depth` (call chain length must not exceed threshold).
- Provide `default_effect` fallback (allow or deny) when no rule matches.
- Load ACL configuration from YAML files via `ACL.load()`, with strict validation of structure and rule fields.
- Support runtime rule management: `add_rule()` inserts at highest priority (position 0), `remove_rule()` removes by caller/target pattern match.
- Support hot reload from the original YAML file via `reload()`.
- All public methods must be thread-safe.

## Technical Design

### Architecture

The ACL system consists of two primary components: the `ACLRule` dataclass representing individual rules, and the `ACL` class that manages a rule list and evaluates access decisions.

#### Rule Evaluation

```
check(caller_id, target_id, context)
  |
  +--> effective_caller = "@external" if caller_id is None else caller_id
  |
  +--> for each rule in rules (first-match-wins):
  |      1. Test caller patterns (OR logic: any pattern matching is sufficient)
  |      2. Test target patterns (OR logic)
  |      3. Test conditions (AND logic: all conditions must pass)
  |      4. If all pass -> return rule.effect == "allow"
  |
  +--> No rule matched -> return default_effect == "allow"
```

#### Pattern Matching

Pattern matching is handled at two levels:
- **Special patterns** (`@external`, `@system`) are resolved directly in `ACL._match_pattern()` using caller identity and context.
- **All other patterns** (exact strings, wildcard `*`, prefix wildcards like `executor.*`) are delegated to the foundation `match_pattern()` utility in `utils/pattern.py`, which implements Algorithm A08 with support for `*` wildcards matching any character sequence including dots.

#### Conditional Rules

When a rule has a `conditions` dict, all specified conditions must be satisfied (AND logic):
- `identity_types`: Context identity's type must be in the provided list.
- `roles`: At least one of the context identity's roles must overlap with the condition's role list (set intersection).
- `max_call_depth`: The length of `context.call_chain` must not exceed the threshold.

If no context is provided but conditions are present, the rule does not match.

### Components

- **`ACLRule`** -- Dataclass with fields: `callers` (list of patterns), `targets` (list of patterns), `effect` ("allow" or "deny"), optional `description`, and optional `conditions` dict.
- **`ACL`** -- Main class managing an ordered rule list. Provides `check()`, `add_rule()`, `remove_rule()`, `reload()`, and the `ACL.load()` classmethod for YAML loading. All public methods are protected by a lock for thread safety.
- **`match_pattern()`** -- Wildcard pattern matcher in `utils/pattern.py`. Supports `*` as a wildcard matching any character sequence. Handles prefix, suffix, and infix wildcards via segment splitting.

### Thread Safety

The `ACL` class uses an internal lock on all public methods. The `check()` method copies the rule list and default effect under the lock, then performs evaluation outside the lock. `add_rule()`, `remove_rule()`, and `reload()` all hold the lock for the duration of their mutations. Single-threaded language runtimes (e.g., JavaScript) MAY treat the lock as a no-op.

### YAML Configuration Format

```yaml
version: "1.0"
default_effect: deny
rules:
  - callers: ["api.*"]
    targets: ["db.*"]
    effect: allow
    description: "API modules can access database modules"
  - callers: ["@external"]
    targets: ["public.*"]
    effect: allow
  - callers: ["*"]
    targets: ["admin.*"]
    effect: deny
    conditions:
      identity_types: ["service"]
      roles: ["admin"]
      max_call_depth: 5
    # Compound conditions with $or and $not
  - callers: ["agent.*"]
    targets: ["data.export"]
    effect: allow
    conditions:
      $or:
        - roles: ["data_admin"]
        - identity_types: ["service"]
      $not:
        max_call_depth: 1  # Deny if call depth is exactly 1
```

Compound operators `$or` and `$not` can combine conditions. `$or` passes if any sub-condition passes. `$not` inverts its sub-condition.

## Usage

=== "Python"
    ```python
    from apcore import APCore
    from apcore.acl import ACL, ACLRule
    from apcore.context import Context, Identity

    # Load ACL from YAML
    acl = ACL.load("acl.yaml")

    # Check access
    identity = Identity(id="api.gateway", type="service", roles=["reader"])
    ctx = Context.create(identity=identity)
    allowed = acl.check("api.gateway", "db.query", ctx)  # True / False

    # Runtime modification
    acl.add_rule(ACLRule(
        callers=["admin.*"],
        targets=["*"],
        effect="allow",
        description="Admins can call any module",
    ))

    # Wire into executor via APCore
    client = APCore()
    client.executor.acl = acl
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";
    import { ACL, ACLRule } from "apcore-js/acl";
    import { Context, Identity } from "apcore-js/context";

    // Load ACL from YAML
    const acl = await ACL.load("acl.yaml");

    // Check access
    const identity: Identity = { id: "api.gateway", type: "service", roles: ["reader"] };
    const ctx = Context.create({ identity });
    const allowed = acl.check("api.gateway", "db.query", ctx);

    // Runtime modification
    acl.addRule(new ACLRule({
        callers: ["admin.*"],
        targets: ["*"],
        effect: "allow",
        description: "Admins can call any module",
    }));

    // Wire into executor via APCore
    const client = new APCore();
    client.executor.acl = acl;
    ```
=== "Rust"
    ```rust
    use apcore::acl::{ACL, ACLRule};
    use apcore::context::{Context, Identity};
    use apcore::APCore;

    // Load ACL from YAML
    let acl = ACL::load("acl.yaml")?;

    // Check access
    use std::collections::HashMap;
    let identity = Identity::new(
        "api.gateway".to_string(),
        "service".to_string(),
        vec!["reader".to_string()],
        HashMap::new(),
    );
    let ctx = Context::create(Some(identity), None);
    let allowed = acl.check("api.gateway", "db.query", Some(&ctx));

    // Runtime modification
    acl.add_rule(ACLRule {
        callers: vec!["admin.*".to_string()],
        targets: vec!["*".to_string()],
        effect: "allow".to_string(),
        description: Some("Admins can call any module".to_string()),
        conditions: None,
    });

    // Wire into executor via APCore
    let mut client = APCore::new();
    client.executor_mut().set_acl(acl);
    ```

## Dependencies

- `apcore.context.Context` -- Provides `identity`, `call_chain`, and other context fields for conditional rule evaluation.
- `apcore.context.Identity` -- Dataclass with `id`, `type`, and `roles` fields used by `@system` pattern and condition checks.
- `apcore.errors.ACLRuleError` -- Raised for invalid ACL configuration (bad YAML structure, missing keys, invalid effect values).
- `apcore.errors.ConfigNotFoundError` -- Raised when the YAML file path does not exist.
- `apcore.utils.pattern.match_pattern` -- Foundation wildcard matching for non-special patterns.

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Lines | Purpose |
    |------|-------|---------|
    | `src/apcore/acl.py` | 279 | `ACLRule` dataclass and `ACL` class with pattern matching, YAML loading, and runtime management |
    | `src/apcore/utils/pattern.py` | 46 | `match_pattern()` wildcard utility (Algorithm A08) |

    **Runtime dependencies:**

    - `yaml` (PyYAML) -- YAML parsing for configuration loading.
    - `threading` (stdlib) -- Lock for thread-safe access to the rule list.
- `os` (stdlib) -- File existence checks in `ACL.load()`.
- `logging` (stdlib) -- Debug-level logging of access decisions.

## Testing Strategy

### Unit Tests (`tests/test_acl.py`)

- **Pattern matching**: Tests for `@external` matching None callers (and not matching string callers), `@system` matching system-type identities (and failing for None or non-system identities), exact patterns, wildcard `*`, and prefix wildcards like `executor.*`.
- **First-match-wins evaluation**: Verifies that the first matching allow returns True, first matching deny returns False, and that rule order takes precedence over specificity.
- **Default effect**: Tests both `default_effect="deny"` and `default_effect="allow"` when no rule matches.
- **YAML loading**: Validates correct loading of rules with descriptions and conditions, and error handling for missing files (`ConfigNotFoundError`), invalid YAML, missing `rules` key, non-list `rules`, missing required keys (`callers`, `targets`, `effect`), invalid effect values, and non-list `callers`.
- **Conditional rules**: Tests `identity_types` matching and failing, `roles` intersection matching and failing, `max_call_depth` within and exceeding limits, and conditions failing when context or identity is None.
- **Runtime modification**: `add_rule()` inserts at position 0, `remove_rule()` returns True/False, `reload()` re-reads the YAML file and updates rules.
- **Context interaction**: Verifies `caller_id=None` maps to `@external`, and context is forwarded to conditional evaluation.
- **Thread safety**: Concurrent `check()` calls (10 threads x 200 iterations) with no errors, and concurrent `add_rule()` + `check()` with no corruption.

### Integration Tests (`tests/integration/test_acl_enforcement.py`)
- End-to-end tests exercising ACL enforcement through the `Executor` pipeline.
