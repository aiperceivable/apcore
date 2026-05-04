# Access Control System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §6 ACL Specification.


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
  +--> No rule matched -> return default_effect  (MUST be "deny" in production; see warning below)
```

!!! danger "default_effect: always use deny in production"
    Setting `default_effect: allow` means every caller that does not match any
    explicit rule is **automatically allowed**. This creates an open-by-default
    system and violates the protocol's security model.  
    **Always use `default_effect: deny`** in production configurations. The
    `allow` value exists only for narrow opt-in scenarios (e.g., public-read
    APIs) and MUST be accompanied by explicit `deny` rules for all sensitive
    targets.

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
    # Compound operators in callers/targets pattern arrays
  - callers: ["$or", "admin.*", "moderator.*"]   # match if either pattern matches
    targets: ["audit.*"]
    effect: allow
  - callers: ["$not", "banned.*"]                # match anything EXCEPT banned.*
    targets: ["public.*"]
    effect: allow
```

`$or` and `$not` are compound operators with **two distinct surface forms**:

1. **Inside `conditions`** — combine condition sub-objects.
   - `$or` (list of condition objects): passes if **any** sub-object's conditions all pass.
   - `$not` (single condition object): passes if the wrapped condition **fails**.
   - Within a single `conditions` block all keys are AND-ed; nest `$or` to express OR.

2. **As the first element of `callers` or `targets` pattern arrays** — combine ID patterns.
   - `["$or", p1, p2, ...]`: matches if **any** of `p1, p2, …` match the module ID. (This is observably equivalent to a flat list, which is already OR-ed; the explicit form documents intent.)
   - `["$not", p]`: matches if `p` does **not** match the module ID. An empty `$not` array (`["$not"]`, no subsequent pattern) MUST evaluate to false (fail-closed). All current SDKs consult only `patterns[1]` and silently ignore any further elements; authors SHOULD therefore supply exactly one pattern after `$not` and treat additional patterns as undefined behavior.

**Async sub-conditions:** `$or`/`$not` evaluate their children using the same evaluator mode (sync or async) as the outer call. Implementations register both sync and async compound handlers; mixing an async handler under a sync evaluator MUST fail closed with a warning. See `docs/spec/design-context-annotations-acl.md` §"Compound + async limitation" for the rationale.

## Contract: ACL.check

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `caller_id`: string, optional (default `None` / `null`). When omitted, the effective caller is `@external`.
- `target_id`: string, required. Module ID being accessed.
- `context`: ExecutionContext, optional. Provides identity type, roles, and call chain for conditional rule evaluation.

### Preconditions

- The rule-list snapshot MUST be taken under the ACL lock; evaluation MAY then proceed outside the lock.

### Side Effects (ordered)

1. Acquire ACL lock.
2. Snapshot the rule list and `default_effect` under the lock.
3. Release the ACL lock.
4. Evaluate rules in order (first-match-wins).
5. Emit an audit event carrying the decision (via the finalize path).

### Errors

- None under normal operation. `check` MUST NOT raise to indicate a deny; it MUST return `false`. Raising is reserved for unrecoverable internal failures (e.g., a corrupted rule list) that the host language's idioms require be surfaced as exceptions.

### Returns

- On success: plain `bool` (`true` = allow, `false` = deny). The return type MUST NOT be wrapped in a `Result`/`Either` type.

### Properties

- `async`: `false`.
- `thread_safe`: `true` -- snapshot-under-lock pattern.
- `pure`: `false` -- emits an audit event on every call.
- `idempotent`: `true` -- repeated calls with identical inputs yield identical decisions (audit events are still emitted each time).

!!! info "Sync handler resolution (cross-language)"
    When a registered condition handler returns a Future / coroutine / Promise from sync `check()`:

    - **If the awaitable completes without suspending** (e.g., an `async def` whose body never reaches an `await`, or a Promise that resolves synchronously on Rust), `check()` MUST use the resolved value.
    - **If the awaitable genuinely suspends** (Pending on first poll, or Promise that resolves later), `check()` MUST treat the condition as unsatisfied. Callers requiring true async handlers MUST use `async_check()`.

    Implementation:

    - **apcore-python** advances the coroutine one step via `coroutine.send(None)` and captures `StopIteration.value` for sync-only bodies.
    - **apcore-rust** polls the future once with a noop `Waker`; `Poll::Ready(v)` uses `v`, `Poll::Pending` denies.
    - **apcore-typescript** can NOT inspect a Promise synchronously; if the handler returns a `Promise`, sync `check()` treats it as unsatisfied. Use `asyncCheck()` to support Promise-returning handlers.

## Contract: ACL.load

### Inputs

- `yaml_path`: string, required. Path to the YAML configuration file.
  - validation: file must exist at the given path (`os.path.isfile(yaml_path)` must return true)
  - reject_with: `ConfigNotFoundError(config_path=yaml_path)`

### Preconditions

- The file at `yaml_path` must be readable and contain valid YAML that parses to a mapping.

### Side Effects (ordered)

1. Open and parse the YAML file from disk.
2. Validate the top-level structure and each rule entry.
3. Construct a new `ACL` instance (no mutation of any existing ACL state).
4. Set `_yaml_path` on the returned instance to `yaml_path` (enabling future `reload()` calls).

### Postconditions

- The returned `ACL` instance has `_yaml_path` set to `yaml_path`.
- `default_effect` is `"deny"` if not explicitly specified in the file.
- Rules are ordered identically to their order in the YAML file.

### Errors

- `ConfigNotFoundError(config_path=yaml_path)` — file does not exist at `yaml_path`.
- `ACLRuleError` — YAML parse failure, top-level value is not a mapping, `rules` key is absent, `rules` value is not a list, any rule entry is not a mapping, any rule is missing a required key (`callers`, `targets`, or `effect`), `effect` value is not `"allow"` or `"deny"`, or `callers`/`targets` value is not a list.

### Returns

- On success: a new `ACL` instance populated from the file.

### Properties

- `async`: `false`
- `thread_safe`: `true` — creates a new instance; no shared mutable state accessed
- `pure`: `false` — reads from the filesystem
- `idempotent`: `true` — repeated calls with identical file content return equivalent instances
- `reentrant`: `true`

## Contract: ACL.add_rule

### Inputs

- `rule`: `ACLRule`, optional (default `None`). A pre-built rule to insert. If provided, all keyword arguments are ignored.
- `callers`: `list[str] | str`, required if `rule` is `None`. Caller pattern(s). A bare string is coerced to a single-element list.
  - validation: must not be `None` when `rule` is `None`
  - reject_with: `ValueError("Must provide either 'rule' or both 'callers' and 'targets'")`
- `targets`: `list[str] | str`, required if `rule` is `None`. Target pattern(s). A bare string is coerced to a single-element list.
  - validation: must not be `None` when `rule` is `None`
  - reject_with: `ValueError("Must provide either 'rule' or both 'callers' and 'targets'")`
- `effect`: string, optional (default `"deny"`). Rule effect when `rule` is `None`.
- `description`: string, optional (default `""`). Human-readable description when `rule` is `None`.
- `conditions`: `dict[str, Any] | None`, optional (default `None`). Condition map when `rule` is `None`.

### Preconditions

- Either `rule` is provided, or both `callers` and `targets` are provided.

### Side Effects (ordered)

1. If `rule` is `None`, construct a new `ACLRule` from keyword arguments.
2. Acquire the ACL lock.
3. Insert the rule at index 0 of the internal rule list (highest priority).
4. Release the ACL lock.

### Postconditions

- The rule is the first entry in the rule list; all prior rules shift up by one index.
- Any subsequent `check()` call evaluates the new rule before all previously inserted rules.

### Errors

- `ValueError` — `rule` is `None` and either `callers` or `targets` is also `None`.

### Returns

- On success: `None`

### Properties

- `async`: `false`
- `thread_safe`: `true` — insert is performed under the ACL lock
- `pure`: `false` — mutates internal rule list
- `idempotent`: `false` — each call inserts an additional rule at position 0; calling twice with identical inputs adds two identical rules
- `reentrant`: `false` — acquires the internal lock; re-entrant call from within the same thread would deadlock on non-reentrant lock implementations

## Contract: ACL.remove_rule

### Inputs

- `callers`: `list[str]`, required. Caller patterns to match (exact list equality).
- `targets`: `list[str]`, required. Target patterns to match (exact list equality).

### Side Effects (ordered)

1. Acquire the ACL lock.
2. Iterate the rule list to find the first rule where `rule.callers == callers` and `rule.targets == targets`.
3. Remove that rule from the list (if found).
4. Release the ACL lock.

### Postconditions

- If a matching rule was found, it is no longer present in the rule list; all subsequent rules shift down by one index.
- At most one rule is removed per call (the first match).

### Errors

- _(none — infallible; absence of a matching rule returns `False`, not an exception)_

### Returns

- `True` — a matching rule was found and removed.
- `False` — no rule with the given `callers` and `targets` patterns exists.

### Properties

- `async`: `false`
- `thread_safe`: `true` — removal is performed under the ACL lock
- `pure`: `false` — mutates internal rule list
- `idempotent`: `false` — the first call removes the rule and returns `True`; a second identical call finds no match and returns `False`
- `reentrant`: `false` — acquires the internal lock

## Contract: ACL.reload

### Inputs

_(none — operates on the YAML path stored during `ACL.load`)_

### Preconditions

- The ACL instance must have been created via `ACL.load()` (i.e., `_yaml_path` is not `None`).
  - reject_with: `ACLRuleError("Cannot reload: ACL was not loaded from a YAML file")`
- The file at the stored `_yaml_path` must still exist and be valid YAML.
  - reject_with: `ConfigNotFoundError` or `ACLRuleError` (propagated from `ACL.load`)

### Side Effects (ordered)

1. Acquire the ACL lock.
2. Snapshot `_yaml_path` under the lock.
3. Release the ACL lock.
4. Call `ACL.load(yaml_path)` outside the lock (reads and validates the YAML file).
5. Acquire the ACL lock again.
6. Replace `_rules` with the newly loaded rule list.
7. Replace `_default_effect` with the newly loaded default effect.
8. Release the ACL lock.

### Postconditions

- `_rules` and `_default_effect` reflect the current content of the YAML file.
- `_yaml_path` is unchanged.
- `_audit_logger` is unchanged (not replaced from the reloaded instance).
- Any `add_rule()` or `remove_rule()` mutations made between the two lock acquisitions (steps 2–5) are discarded.

### Errors

- `ACLRuleError` — instance was not created via `ACL.load()` (no stored YAML path), or the YAML file fails structural validation.
- `ConfigNotFoundError` — the stored YAML file no longer exists at the original path.

### Returns

- On success: `None`

### Properties

- `async`: `false`
- `thread_safe`: `true` — mutations to `_rules` and `_default_effect` are performed under the ACL lock; note that two separate lock acquisitions are used (snapshot then write), so concurrent mutations between the two acquisitions are possible (see Postconditions)
- `pure`: `false` — reads from the filesystem and mutates internal state
- `idempotent`: `true` — repeated calls with the same file content produce the same rule list
- `reentrant`: `false` — acquires the internal lock

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
