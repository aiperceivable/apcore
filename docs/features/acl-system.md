---
description: "Pattern-based ACL with first-match-wins evaluation: wildcard and @external/@system patterns, identity/role/depth/argument conditions, approval rules, audit delivery, default-deny, hot-reload."
---

# Access Control System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §6 ACL Specification.


## Overview

Pattern-based Access Control List (ACL) with first-match-wins evaluation for module access control. The system enforces which callers may invoke which target modules, using wildcard patterns, special identity patterns (`@external`, `@system`), and optional conditions based on identity type, roles, call depth and the call's argument keys. A rule can also require human approval for the calls it allows. Configuration is loaded from an ACL YAML file (which also configures audit delivery) and can be hot-reloaded at runtime.

An ACL is enforced only when one is attached to the Executor. If no ACL file is found at `acl.root` and none is set programmatically, **no ACL is attached and no access check runs** — a missing file does not produce a default-deny ACL (D-64). `default_effect` comes from the ACL file itself; the `acl.default_effect` key in `apcore.yaml` has no effect.

## Requirements

- Implement first-match-wins rule evaluation: rules are evaluated in order, and the first rule whose patterns match the caller and target determines the access decision (allow or deny).
- Support wildcard patterns for caller and target matching (e.g., `admin.*`, `*`), delegating to a shared pattern-matching utility.
- Handle special patterns: `@external` matches calls with no caller (external entry points), and `@system` matches calls where the execution context has a system-type identity.
- Support conditional rules with `identity_types` (identity type must be in list), `roles` (at least one role must overlap), `max_call_depth` (call chain length must not exceed threshold), and `arguments` (which argument keys the call carries).
- Support an `approval` field on `allow` rules (`required` / `not_required`) so a rule can grant access and require human approval for the same call.
- Provide `default_effect` fallback (allow or deny) when no rule matches.
- Deliver audit records per the ACL file's `audit:` block or a programmatic callback.
- Load ACL configuration from YAML files via `ACL.load()`, with strict validation of structure and rule fields.
- Support runtime rule management: `add_rule()` inserts at highest priority (position 0), `remove_rule()` removes by caller/target pattern match.
- Support hot reload from the original YAML file via `reload()`.
- All public methods must be thread-safe.

## Technical Design

### Architecture

The ACL system consists of two primary components: the `ACLRule` dataclass representing individual rules, and the `ACL` class that manages a rule list and evaluates access decisions.

#### Rule Evaluation

```text
check(caller_id, target_id, context)
  |
  +--> effective_caller = "@external" if caller_id is None else caller_id
  |
  +--> for each rule in rules (first-match-wins):
  |      1. Test caller patterns (OR logic: any pattern matching is sufficient)
  |      2. Test target patterns (OR logic)
  |      3. Test conditions (AND logic: all conditions must pass)
  |      4. If all pass -> decision = rule.effect, approval_required = (rule.approval == "required")
  |
  +--> No rule matched -> decision = default_effect  (MUST be "deny" in production; see warning below)
  |
  +--> check() returns true only for "allow" with no approval requirement;
       check_access() returns both results (AccessDecision)
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
- **Special patterns** (`@external`, `@system`) are resolved by the ACL itself using the caller ID and context: `@external` matches a missing `caller_id`, `@system` a system-type identity.
- **All other patterns** (exact strings, wildcard `*`, prefix wildcards like `executor.*`) are delegated to the shared `match_pattern()` utility, which implements Algorithm A08: `*` matches any character sequence including dots and is the only metacharacter (`?` is a literal, §6.2.2).

#### Conditional Rules

When a rule has a `conditions` dict, all specified conditions must be satisfied (AND logic):
- `identity_types`: Context identity's type must be in the provided list.
- `roles`: At least one of the context identity's roles must overlap with the condition's role list (set intersection).
- `max_call_depth`: The length of `context.call_chain` must not exceed the threshold.
- `arguments`: A structure-only test of the call's argument keys — `has_key` (any listed key present), `has_all_keys` (every listed key present), `has_none_of` (no listed key present); several predicates are AND-ed. It never reads argument values. It reads the call's governance projection (§6.1.8); when no projection is available the condition is unevaluable, never "no arguments" ([PROTOCOL_SPEC §6.1.7](../spec/protocol-spec.md)).

These four are the built-ins, plus the compound operators `$or` and `$not`. The set is open: `register_condition()` adds a condition key at runtime, and a rule may reference any key a handler has been registered for. `arguments` itself has no registration point.

If no context is provided but conditions are present, the rule does not match — **provided the rule passes the precheck below**. A malformed rule is unevaluable first, context or no context. See the warning in [PROTOCOL_SPEC §6.5](../spec/protocol-spec.md#65-edge-case-handling) — a *well-formed* conditional `deny` rule is not a backstop for context-less callers.

#### Unevaluable Conditions

A condition that is **false** and a condition that **cannot be evaluated** are different outcomes, and the difference decides what a `deny` rule does.

- **False** — a registered handler ran and returned false, having understood the value it was given. Ordinary non-match; evaluation continues to the next rule.
- **Unevaluable** — the implementation cannot answer the condition **as written**. This is a principle, not a closed list. The cases every implementation meets: the key has no handler resolvable on the path in use; the handler raised/threw/panicked; the handler was async and unresolvable on the synchronous `check()` path; **the value is malformed for its key** (`$or` that is not a list, `$not` that is not an object); **`conditions` itself is not a mapping**. An implementation that meets an unlisted case classifies it by the principle, never by defaulting to false.

When a condition is unevaluable the rule MUST resolve toward refusing access:

| Rule `effect` | Condition false | Condition unevaluable |
|---|---|---|
| `allow` | does not match → continue | does not match → continue (MUST NOT grant); a carried `approval: required` becomes **pending** |
| `deny` | does not match → continue | **rule takes effect → the call is denied** |

**An unevaluable `allow` rule does not take its approval requirement with it.** "Does not grant" means the rule steps aside, and a rule now carries two axes. If it carried `approval: required`, the requirement is recorded as **pending** and composed by disjunction with whatever grants later — a subsequent `allow` rule, or `default_effect: allow`. A final `deny` clears it, and `matched_rule_index` keeps naming the rule that actually decided. A rule whose `callers`/`targets` do not match this call raises nothing; a rule whose pattern field is itself malformed does, because its scope cannot be read and so cannot be shown not to apply here.

Without this, the shape the `arguments` condition exists for — a narrow approval rule ahead of a broad allow — fails open: the narrow rule steps aside, the broad one grants, and `git push --force` runs with no human asked. Normative text: [PROTOCOL_SPEC §6.1.1](../spec/protocol-spec.md#611-unevaluable-conditions-v1220-100) rule 5.

`AuditEntry.handler_error` MUST be non-null for an unevaluable condition and MUST be null for a merely-false one — it is what makes the two distinguishable after the fact. Normative text: [PROTOCOL_SPEC §6.1.1](../spec/protocol-spec.md#611-unevaluable-conditions-v1220-100).

The three outcomes compose through AND and the compound operators by three-valued logic: an outright "no" wins an AND, an outright "yes" wins an `$or`, and anything else with an unevaluable child is unevaluable. `$not` of an unevaluable condition is **unevaluable**, never satisfied — negating "no answer" into "yes" would let a misspelled key inside a `$not` satisfy the rule it was meant to gate. Full table: [PROTOCOL_SPEC §6.1.1](../spec/protocol-spec.md#611-unevaluable-conditions-v1220-100).

#### The precheck: structure and registry, before any handler runs

Before evaluating a rule's conditions, the implementation walks the **whole** `conditions` tree — every branch inside `$or` and `$not` — checking only structure and the handler registries. It supplies no context and runs no handler. Normative text: [PROTOCOL_SPEC §6.1.4](../spec/protocol-spec.md#614-structural-and-registry-precheck-v1250-100).

Two things follow, and both matter:

**It runs before the no-context check.** A rule that fails the precheck is unevaluable whether or not the call supplied a context — otherwise `conditions: {mispelled: true}` on a `deny` rule would pass traffic simply because the caller carried no identity. A rule that **passes** the precheck and then finds no context still takes the § Conditional Rules path and does not match: `roles` is answerable in principle, and this caller merely supplied no input for it. The line is between *a question this caller did not answer* and *a question nobody can answer*.

**It does not widen a rule's reach.** The precheck says whether a rule can be evaluated, never which calls it applies to. Pattern-field structure is checked first; a rule whose *well-formed* `callers` or `targets` fails to match simply does not apply to this call, and a fault in its `conditions` is neither consulted nor allowed to change the decision — otherwise one typo in a rule scoped to `api.*` would decide calls from `worker.*`. A *malformed* pattern field is different: the rule's scope is unknowable, so the rule is unevaluable. Faults in out-of-scope rules are still real and still reported — by `validate_rules()`, which looks at every rule and no call. Full ordering: [PROTOCOL_SPEC §6.1.4](../spec/protocol-spec.md#614-structural-and-registry-precheck-v1250-100) rule 4.

**Its diagnostics are deterministic.** Because the precheck is context-free, handler-free and exhaustive, its findings are a pure function of the rule, so every SDK reports the same set in the same order. Diagnostics that come from running a handler — one that throws, or an async one on the sync path — carry no such guarantee, because handler execution MAY short-circuit. Configuration mistakes are always reported identically; runtime failures are reported as encountered.

Findings name a **condition path**, not just a key, since a key can occur at several positions in a nested tree:

| Position | Path |
|---|---|
| `k` at the root of `conditions` | `k` |
| `k` in the *i*-th `$or` branch (0-based) | `$or[i].k` |
| `k` inside `$not` | `$not.k` |
| the `conditions` object itself | `$` |
| the rule's `callers` / `targets` | `callers` / `targets` |

Paths nest — `$or[1].$not.k`. `handler_error` and `validate_rules()` both order by path.

### Components

- **`ACLRule`** -- A rule with fields: `callers` (list of patterns), `targets` (list of patterns), `effect` (`"allow"` or `"deny"`), optional `approval` (`"required"` or `"not_required"`, default `"not_required"`; `required` on a `deny` rule is rejected with `ACLRuleError`), optional `description`, and optional `conditions` dict. The key set is closed: any other key is rejected with `ACLRuleError` (§6.1.5). Python dataclass; TypeScript plain object (`ACLRule` interface); Rust `#[non_exhaustive]` struct built with `ACLRule::new(callers, targets, effect)` and optional fields assigned afterwards.
- **`ACL`** -- Main class managing an ordered rule list. Provides `check()` / `check_access()` (and async variants), `add_rule()`, `remove_rule()`, `reload()`, the read-only accessors `default_effect` and `rules`, the diagnostic `validate_rules()`, and the `ACL.load()` / `ACL.discover()` constructors. All mutating methods are protected by a lock for thread safety.
- **`AccessDecision`** -- Structured result of `check_access()`: `access` (`"allow"`/`"deny"`), `approval_required`, `matched_rule_index`, `reason` ([PROTOCOL_SPEC §6.8.1](../spec/protocol-spec.md)). The Executor's `acl_check` step uses it and hands `approval_required` to the [approval gate](./approval-system.md).
- **`AuditEntry`** -- Structured record of one `check()` decision, emitted through the configured audit logger on every call. Field contract: [PROTOCOL_SPEC §6.3.1](../spec/protocol-spec.md#631-audit-entry).
- **`match_pattern()`** -- Shared wildcard pattern matcher (Algorithm A08). Supports `*` as a wildcard matching any character sequence, in prefix, suffix, and infix positions.

### Thread Safety

The `ACL` class uses an internal lock on all public methods. The `check()` method copies the rule list and default effect under the lock, then performs evaluation outside the lock. `add_rule()`, `remove_rule()`, and `reload()` all hold the lock for the duration of their mutations. Single-threaded language runtimes (e.g., JavaScript) MAY treat the lock as a no-op.

### YAML Configuration Format

```yaml
version: "1.0"
default_effect: deny
audit:                      # optional; declaring it activates the default audit sink
  enabled: true
  include_denied: true
  log_level: info
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
        max_call_depth: 1  # ...and only when called from another module (call depth > 1)
    # Allowed, but a call carrying a "force" argument must be approved by a human...
  - callers: ["agent.*"]
    targets: ["repo.push"]
    effect: allow
    approval: required
    conditions:
      arguments:
        has_key: ["force"]
    # ...while an ordinary push is allowed outright
  - callers: ["agent.*"]
    targets: ["repo.push"]
    effect: allow
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
   - `["$or", p1, p2, ...]`: matches if **any** of `p1, p2, …` match the module ID. (This is observably equivalent to a flat list, which is already OR-ed; the explicit form documents intent.) At least one operand.
   - `["$not", p]`: matches if `p` does **not** match the module ID. **Exactly one** operand.

**Only the first form nests.** A pattern array is **flat**: there is one operator position — index 0 — and every element after it is a plain pattern string, never a nested array and never another operator. `["$or", "$not", "a"]` is *not* or-of-not, and `["api.*", "$not", "cli.*"]` is *not* "api.* but not cli.*"; both are rejected. This is the difference that catches people out, because the same two tokens nest arbitrarily inside `conditions` (`$or[1].$not.k` is a defined path there — [PROTOCOL_SPEC §6.1.4](../spec/protocol-spec.md#614-structural-and-registry-precheck-v1250-100)).

**The array's shape is a closed set, rejected with `ACLRuleError` at every entry point** — file loading, direct construction and runtime insertion ([PROTOCOL_SPEC §6.2.1](../spec/protocol-spec.md#621-compound-operators-in-pattern-arrays)): at least one element, every element a non-empty string, `$or` with at least one operand, `$not` with exactly one, and `$or` / `$not` nowhere but index 0.

```yaml
# ---- legal ----
targets: ["executor.*"]                       # one pattern
targets: ["api.*", "worker.*"]                # OR, implicitly
targets: ["$or", "api.*", "worker.*"]         # OR, explicitly - same meaning, states intent
targets: ["$not", "executor.secrets.*"]       # "anything that is not executor.secrets.*"

# ---- rejected: shape ----
targets: []                                   # no operands - matches nothing, so the rule is no rule
targets: ["$or"]                              # OR over nothing
targets: ["$not"]                             # negation of nothing
targets: [""]                                 # the empty pattern matches no legal module ID
targets: ["$not", "a", "b"]                   # $not takes EXACTLY one operand
targets: ["$or", "$not", "a"]                 # no nesting - this was an OR of two literals
targets: ["api.*", "$not", "cli.*"]           # no such form exists

# ---- legal, but validate_rules() reports it as matching nothing ----
targets: ["$not", "*"]                        # "not everything" is well-formed and matches nothing
```

`NOT (a OR b)` has **no single-array form** — `$not` takes one operand and the array's own combinator is OR. Use a glob when the excluded patterns share a prefix, and otherwise first-match-wins with two rules:

```yaml
rules:
  - callers: ["*"]
    targets: ["$or", "executor.secrets.a", "executor.secrets.b"]
    effect: deny
    description: "Excluded targets, refused first"
  - callers: ["*"]
    targets: ["*"]
    effect: allow
    description: "Everything else"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info
```

!!! warning "That two-rule form is not a drop-in replacement inside an existing rule list"
    `["$not", p]` makes the rule **not match** `p`, so evaluation **continues** and a later
    rule may still decide the call. A leading `deny` on `p` **ends** the scan. They agree
    only when nothing after the rule could have matched `p` and `default_effect` would have
    refused it anyway — true of the complete policy above, not true in general. Rewriting a
    rule into this form changes the policy's order, not just one field.

**Async sub-conditions:** `$or`/`$not` evaluate their children using the same evaluator mode (sync or async) as the outer call. Implementations register both sync and async compound handlers; an async-only handler reached from the sync `check()` path is unevaluable (see [Sync handler resolution](#contract-aclcheck) below).

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
4. Evaluate rules in order (first-match-wins). A rule's conditions resolve to one of three outcomes — satisfied, unsatisfied, or unevaluable — and an unevaluable condition resolves the rule toward refusing access (§ Unevaluable Conditions above).
5. Emit an audit event carrying the decision (via the finalize path). When a condition was unevaluable, `handler_error` on that entry MUST be non-null and MUST name the condition key and the reason.

### Errors

- None under normal operation. `check` MUST NOT raise to indicate a deny; it MUST return `false`. Raising is reserved for unrecoverable internal failures (e.g., a corrupted rule list) that the host language's idioms require be surfaced as exceptions.
- An unevaluable condition is NOT such a failure: it MUST NOT propagate out of `check()`. A handler that raises, throws, or panics MUST be caught, recorded in `handler_error`, and resolved per § Unevaluable Conditions.

### Returns

- On success: plain `bool`. `true` only when the decision is `allow` **and** the call needs no approval; an allowed call whose rule carries `approval: required` returns `false`, because a non-Executor caller can only read the boolean as "let it through" (§6.8.1). The return type MUST NOT be wrapped in a `Result`/`Either` type.
- `check_access()` (`checkAccess()` in TypeScript) takes the same inputs and returns the full `AccessDecision` — `access` and `approval_required` separately. `async_check()` / `async_check_access()` are the async counterparts.

### Properties

- `async`: `false`.
- `thread_safe`: `true` -- snapshot-under-lock pattern.
- `pure`: `false` -- emits an audit event on every call.
- `idempotent`: `true` -- repeated calls with identical inputs yield identical decisions (audit events are still emitted each time).

!!! info "Sync handler resolution (cross-language)"
    When a registered condition handler returns a Future / coroutine / Promise from sync `check()`:

    - **If the awaitable completes without suspending** (e.g., an `async def` whose body never reaches an `await`, or a Promise that resolves synchronously on Rust), `check()` MUST use the resolved value — SATISFIED or UNSATISFIED as the value says.
    - **If the awaitable genuinely suspends** (Pending on first poll, or a Promise that resolves later), `check()` MUST treat the condition as **UNEVALUABLE**, not as unsatisfied. Per § Unevaluable Conditions that means a `deny` rule takes effect and an `allow` rule does not grant, and `handler_error` MUST be set. Callers requiring true async handlers MUST use `async_check()`.

    Implementation:

    - **apcore-python** advances the coroutine one step via `coroutine.send(None)` and captures `StopIteration.value` for sync-only bodies; a coroutine that suspends is closed and reported UNEVALUABLE.
    - **apcore-rust** polls the future once with a noop `Waker`; `Poll::Ready(v)` uses `v`, `Poll::Pending` is UNEVALUABLE.
    - **apcore-typescript** can NOT inspect a Promise synchronously; if the handler returns a `Promise`, sync `check()` reports UNEVALUABLE. Use `asyncCheck()` to support Promise-returning handlers.


## Contract: ACL.load

### Inputs

- `yaml_path`: string, required. Path to the YAML configuration file.
  - validation: a file must exist at the given path
  - reject_with: `ConfigNotFoundError(config_path=yaml_path)`
- `audit_logger` (optional) — audit callback (see [Audit Delivery](#audit-delivery)); Python keyword argument, TypeScript second positional argument. Rust takes only the path; set a logger afterwards with `set_audit_logger`.

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
- The file's `audit:` block, if declared, configures the default audit sink (see [Audit Delivery](#audit-delivery)).
- A warning is emitted for every rule that references a condition key with no handler registered **at load time**, naming the rule index, the key, and the rule's `effect`. The load still succeeds — see Errors below.

### Errors

- `ConfigNotFoundError(config_path=yaml_path)` — file does not exist at `yaml_path`.
- `ACLRuleError` — YAML parse failure, top-level value is not a mapping, `default_effect` is not `"allow"` or `"deny"`, `rules` key is absent, `rules` value is not a list, any rule entry is not a mapping, any rule is missing a required key (`callers`, `targets`, or `effect`) or carries a key outside the closed set (`callers`, `targets`, `effect`, `approval`, `description`, `conditions`), `effect` value is not `"allow"` or `"deny"`, `approval` is not `"required"` / `"not_required"` or is `"required"` on a `deny` rule, `callers`/`targets` value is not a list, or a `callers`/`targets` array's shape is outside [§6.2.1](../spec/protocol-spec.md#621-compound-operators-in-pattern-arrays)'s closure (empty, an empty element, `$or` with no operands, `$not` with none or more than one, or a reserved token away from index 0).
- `ConfigError` (`CONFIG_INVALID`) — the `audit:` block fails `$defs/AuditConfig` in `schemas/acl-config.schema.json` (wrong type or unknown key inside the block). Other unrecognised root keys are ignored.
- **NOT** an error: a rule referencing an unregistered condition key. `register_condition()` writes to a runtime, process-wide registry, and `acl.root` discovery commonly runs before application code has registered anything, so failing here would reject valid configurations on ordering alone. Loading warns; [`validate_rules()`](#contract-aclvalidate_rules) is the deterministic check to run once registration is complete; and [§6.1.1](../spec/protocol-spec.md#611-unevaluable-conditions-v1220-100) guarantees the rule cannot silently pass traffic either way.

### Returns

- On success: a new `ACL` instance populated from the file.

### Properties

- `async`: `false`
- `thread_safe`: `true` — creates a new instance; no shared mutable state accessed
- `pure`: `false` — reads from the filesystem
- `idempotent`: `true` — repeated calls with identical file content return equivalent instances
- `reentrant`: `true`

## Contract: ACL.discover

Config-driven activation of the `acl.root` key (D-64). `discover()` resolves `acl.root` and loads an ACL **only when the configured path exists**, so that ACL enforcement can be turned on by configuration alone — without application code calling `ACL.load()` + `set_acl()` by hand. The application bootstrap (`APCore`) calls `discover()` automatically and attaches the result.

!!! danger "Missing-path invariant — MUST NOT synthesize a default-deny ACL"
    When the resolved `acl.root` path does **not** exist, `discover()` MUST return "no ACL" (`None`/`null`/`Option::None`) and attach nothing. It MUST NOT construct an empty ACL — an empty ACL with `default_effect: deny` would deny **every** inter-module call in every project that has no ACL file. A missing path means *no enforcement*. `default_effect` is read from the ACL file once one is loaded; `acl.default_effect` in `apcore.yaml` is an inert key and never applies.

### Inputs

- `config`: the loaded `Config`, required. `acl.root` is read from it.
  - `acl.root` default: `"./acl"` in all SDKs.

### Preconditions

- None. `acl.root` MAY be unset (the default applies) and MAY point at a path that does not exist (no-op).

### Side Effects (ordered)

1. Read `acl.root` from `config` (apply the `"./acl"` default if unset).
2. Resolve the path: relative to the config file's directory when the `Config` knows its source path, else relative to the current working directory. (This makes `acl.root` the one path-typed key whose base differs from `schema.root` and `extensions.root`, which resolve against the CWD; [PROTOCOL_SPEC §9.2.2](../spec/protocol-spec.md#922-path-resolution-base) defines the single project-root rule planned for 2.0.)
3. If the resolved path is a **directory**, target `<root>/global_acl.yaml` (the `acl/{scope}_acl.yaml` convention, PROTOCOL_SPEC §3.1); if it is a **file**, target it directly.
4. If the target file exists, load it via `ACL.load()` and return the new `ACL`.
5. If the resolved path / target file does not exist, return "no ACL" and attach nothing.

### Postconditions

- Returns a loaded `ACL` **iff** the resolved target file exists; otherwise returns the language's "no ACL" value.
- No empty/synthesized ACL is ever returned for a missing path.
- Skipped entirely when the caller supplies their own `Executor` to `APCore`, so an explicitly-wired ACL is never overwritten.

### Errors

- `ACLRuleError` / `ConfigError` — only when a target file **exists but is invalid** (propagated from `ACL.load()`). A missing path is never an error.

### Returns

- A new `ACL` instance, or the language "no ACL" value (`None` / `null` / `Option::None`).

### Properties

- `async`: `false`
- `thread_safe`: `true` — creates a new instance; no shared mutable state accessed
- `pure`: `false` — reads from the filesystem
- `idempotent`: `true`
- `reentrant`: `true`

### Usage

With `acl.root` set in `apcore.yaml`, enforcement is wired automatically — no manual `ACL.load()` / `set_acl()` needed. `default_effect` and `audit:` belong in the ACL file, not here:

```yaml
# apcore.yaml
acl:
  root: ./acl            # directory holding global_acl.yaml (default: ./acl)
```

=== "Python"
    ```python
    from apcore import ACL, APCore, Config

    # APCore calls ACL.discover(config) and attaches the result.
    # If ./acl/global_acl.yaml exists, enforcement is active; if not, no ACL is attached.
    app = APCore(config=Config.load("apcore.yaml"))

    # Equivalent explicit form:
    acl = ACL.discover(Config.load("apcore.yaml"))
    if acl is not None:
        app.executor.set_acl(acl)
    ```
=== "TypeScript"
    ```typescript
    import { ACL, APCore, Config } from "apcore-js";

    // APCore calls ACL.discover(config) and attaches the result.
    const app = new APCore({ config: Config.load("apcore.yaml") });

    // Equivalent explicit form:
    const acl = ACL.discover(Config.load("apcore.yaml"));
    if (acl !== null) {
        app.executor.setAcl(acl);
    }
    ```
=== "Rust"
    ```rust
    use std::path::Path;

    use apcore::{ACL, APCore, Config, Executor, ModuleError, Registry};

    fn main() -> Result<(), ModuleError> {
        // APCore::with_config calls ACL::discover(&config) and attaches the result.
        let config = Config::load(Path::new("apcore.yaml"))?;
        let _app = APCore::with_config(config.clone());

        // Equivalent explicit form. `set_acl` needs `&mut Executor`, so configure
        // the Executor before handing it to APCore.
        let mut executor = Executor::new(Registry::default(), config.clone());
        if let Some(acl) = ACL::discover(&config)? {
            executor.set_acl(acl);
        }
        let _explicit = APCore::with_options(None, Some(executor), Some(config), None);
        Ok(())
    }
    ```

## Audit Delivery

Every `check()` produces an `AuditEntry` ([PROTOCOL_SPEC §6.3.1](../spec/protocol-spec.md#631-audit-entry)). Where it goes is decided by **exactly one effective sink** (§6.3.2):

| Condition | Effective sink |
|---|---|
| An audit callback was supplied (`audit_logger`) | That callback, which receives every entry, allow and deny alike |
| No callback, and the ACL file **declares** an `audit:` block with `enabled: true` | The default sink: one structured record per decision, event name `apcore.acl.audit`, at `log_level` |
| Otherwise | None — no audit records are produced |

The `audit:` block lives in the ACL file (see [YAML Configuration Format](#yaml-configuration-format)) and configures the default sink only:

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Whether the default sink is active. Consulted only when the block is declared — an ACL file without `audit:` produces no audit output unless a callback is supplied |
| `include_denied` | `true` | `false` withholds `deny` entries from the default sink, and warns once per load |
| `log_level` | `info` | `trace` / `debug` / `info` / `warn` / `error` |

The `acl.audit.*` keys in `apcore.yaml` are inert; configure auditing in the ACL file.

- A supplied callback is never narrowed or silenced by the block. If both are present, one diagnostic per ACL construction names the block's fields that do not apply.
- The callback must be synchronous; one that returns an awaitable is treated as a failed delivery. Delivery failures never change the access decision, and after the first one further diagnostics for the same sink are suppressed.
- `reload()` re-reads the `audit:` block and keeps a programmatic callback.

Supplying the callback or the audit configuration programmatically:

| SDK | Callback | Audit configuration |
|---|---|---|
| Python | `ACL(rules, default_effect, audit_logger=fn)`, `ACL.load(path, audit_logger=fn)` | `ACL(..., audit_config={"enabled": True, ...})` |
| TypeScript | `new ACL(rules, defaultEffect, auditLogger)`, `ACL.load(path, auditLogger)` | `new ACL(rules, defaultEffect, auditLogger, auditConfig)` |
| Rust | `ACL::new(rules, default_effect, Some(Arc::new(logger)))`, `acl.set_audit_logger(fn)` | `acl.with_audit_config(Some(AuditConfig { .. }))` / `set_audit_config` |

## Contract: ACL.add_rule

### Inputs

- `rule`: pre-built `ACLRule` to insert at the front of the rule list (highest priority).

Python additionally accepts keyword arguments (`add_rule(callers=..., targets=..., effect=..., ...)`) and builds the rule itself; TypeScript takes a plain `ACLRule` object; Rust takes an `ACLRule` built with `ACLRule::new` and also offers `try_add_rule`, which returns the `ACLRuleError` instead of panicking. Only the prebuilt-rule form is required across SDKs.

### Preconditions

- `rule` is a well-formed `ACLRule` (callers + targets non-empty, effect ∈ {"allow", "deny"}, valid `approval`). This is **enforced**: `add_rule` re-validates the rule it is handed — including one that was well-formed when constructed and has since had `callers` or `targets` assigned — and raises `ACLRuleError` ([§6.2.1](../spec/protocol-spec.md#621-compound-operators-in-pattern-arrays)). Validation order within a rule is `effect` → `approval` → `callers` / `targets`.

### Side Effects (ordered)

1. Acquire the ACL lock.
2. Insert the rule at index 0 of the internal rule list (highest priority).
3. Release the ACL lock.
4. If the rule carries `conditions`, check each key — including keys nested inside `$or` / `$not` — against the handler registries, and emit a warning for every key that does not resolve on the sync path. The warning MUST name the rule index (`0`), the key, and the rule's `effect`. Insertion still succeeds: this is the same warn-never-fail contract [`load()`](#contract-aclload) has, for the same reason ([PROTOCOL_SPEC §6.1.2](../spec/protocol-spec.md#612-load-time-validation-of-condition-keys-v1220-100) rule 4 makes runtime insertion an entry point that MUST be covered).

### Postconditions

- The rule is the first entry in the rule list; all prior rules shift up by one index.
- Any subsequent `check()` call evaluates the new rule before all previously inserted rules.
- A warning has been emitted for each unresolvable condition key the rule references. No exception is raised for one.
- The dedupe state of the §6.5 "conditions present but no context" warning is cleared, because the insertion shifted every rule index (D-88). `remove_rule` and `reload` clear it too.

### Errors

- `ACLRuleError` — the rule is malformed (see Preconditions). Rust's `add_rule` panics with the same message; use `try_add_rule` to receive it as an error.
- `ValueError` (Python keyword path only) — when `rule` is `None` and either `callers` or `targets` is also `None`.

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
7. Replace `_default_effect` with the newly loaded default effect, and the audit configuration with the reloaded `audit:` block.
8. Release the ACL lock.

### Postconditions

- `_rules` and `_default_effect` reflect the current content of the YAML file.
- `_yaml_path` is unchanged.
- The audit callback is unchanged (not replaced from the reloaded instance); the `audit:` block's configuration is re-applied from the file.
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

## Contract: ACL.default_effect

Normative behavioral contract ([PROTOCOL_SPEC §6.8](../spec/protocol-spec.md#68-acl-introspection-v1230-101)). Read-only accessor for the effect applied when no rule matches.

### Inputs

- None.

### Preconditions

- None. Valid on any constructed `ACL`, including one with an empty rule list.

### Side Effects (ordered)

- None. This is a pure read: it MUST NOT emit an audit event and MUST NOT mutate state.

### Postconditions

- The returned value is `"allow"` or `"deny"` and equals the effect `check()` would apply when no rule matches.
- After `reload()`, the value reflects the reloaded file.

### Errors

- None.

### Returns

- On success: `string` — `"allow"` or `"deny"`.

### Properties

- `async`: `false`.
- `thread_safe`: `true`.
- `pure`: `true`.
- `idempotent`: `true`.
- `reentrant`: `true` — MUST NOT acquire a lock the caller has to release.

## Contract: ACL.rules

Normative behavioral contract ([PROTOCOL_SPEC §6.8](../spec/protocol-spec.md#68-acl-introspection-v1230-101)). Read-only accessor for the current rule list.

### Inputs

- None.

### Preconditions

- None.

### Side Effects (ordered)

- None. Pure read, as for `default_effect`.

### Postconditions

- Rules are returned in definition order — the same order `check()` evaluates them in.
- The returned value MUST NOT be a mutable reference into the ACL's own list. Return an immutable view or a copy, taken under the same snapshot discipline `check()` uses.
- After `reload()`, the list reflects the reloaded file.

### Errors

- None.

### Returns

- On success: an ordered, immutable sequence of `ACLRule`.

### Properties

- `async`: `false`.
- `thread_safe`: `true`.
- `pure`: `true`.
- `idempotent`: `true`.
- `reentrant`: `true`.

## Contract: ACL.validate_rules

Normative behavioral contract ([PROTOCOL_SPEC §6.1.2](../spec/protocol-spec.md#612-load-time-validation-of-condition-keys-v1220-100)). Reports every rule that fails the [precheck](#the-precheck-structure-and-registry-before-any-handler-runs): a condition key with no resolvable handler, a value malformed for its key, a `conditions` that is not a mapping, or a `callers`/`targets` that is not a list of strings. Named `validate_rules` and not `validate_conditions` because of the last of those.

Condition handlers are registered at runtime into a process-wide registry, and an ACL may legitimately be loaded before a deployment registers its custom handlers — `acl.root` discovery commonly runs during framework bootstrap, ahead of application code. Loading therefore does not fail on an unregistered key. This method is the deterministic check to run **after** registration is complete.

### Inputs

- None. Operates on the ACL's current rule list and the process-wide handler registry as it stands at call time.

### Preconditions

- None. Calling before any handler is registered is valid and simply reports every non-built-in key.

### Side Effects (ordered)

- None. MUST NOT mutate the ACL, MUST NOT register handlers, and MUST NOT emit an audit event.

### Postconditions

- Every rule whose `conditions` tree fails the precheck is reported — an unresolvable key on the **sync** path, a malformed compound value, or a non-mapping `conditions` — including faults nested inside `$or` / `$not`.
- Each finding carries at least: the rule's index in definition order, the condition path, the condition key, the rule's `effect`, and `sync_resolvable` / `async_resolvable`.
- A rule with no `conditions` is never reported.
- An empty result means every rule currently passes the precheck. It is not a guarantee about the future: a later `add_rule()` can introduce a fault, and a handler can be unregistered.
- Faults are reported for **every** rule, independently of any call. A rule scoped to `callers: ["api.*"]` is reported even though no `worker.*` call would ever reach its conditions — [§6.1.4](../spec/protocol-spec.md#614-structural-and-registry-precheck-v1250-100) rule 4 deliberately keeps such a rule out of an unrelated call's decision, so this validator is the only place its typo surfaces.

!!! warning "Sync and async registries are separate — a key can resolve on one path only"
    SDKs keep two condition-handler registries. `async_check()` consults the async
    registry and falls back to the sync one; `check()` consults only the sync
    registry. So a key registered **only** as an async handler is a working
    condition under `async_check()` and an *unevaluable* one under `check()` —
    which, per § Unevaluable Conditions, makes a `deny` rule deny and an `allow`
    rule not grant on the sync path.

    That is why a finding reports two flags rather than one boolean. A finding is
    emitted whenever `sync_resolvable` is false, **including** when
    `async_resolvable` is true. An application that only ever calls `async_check()`
    may choose to ignore those; that judgement belongs to the caller, not to the
    validator. Full table:
    [PROTOCOL_SPEC §6.1.3](../spec/protocol-spec.md#613-sync-and-async-handler-registries-v1220-100).

    The built-ins are not symmetric either: `identity_types`, `roles` and
    `max_call_depth` are sync-registered (and so resolve on both paths via the
    fallback), while `$or` and `$not` are registered in both.

### Errors

- None. Findings are returned, not raised — a caller decides whether an unregistered key is fatal for its deployment.

### Returns

- On success: a possibly-empty ordered collection of findings. Each finding carries five fields:

| Field | Type | Meaning |
|---|---|---|
| `rule_index` | `integer` | Index of the offending rule in definition order |
| `condition_path` | `string` | Where the fault sits — `roles`, `$or[1].mispelled`, `$or[0]` for a malformed branch, `$` for a non-mapping `conditions`, `callers` / `targets` for a malformed pattern field |
| `condition_key` | `string \| null` | The key itself, or **null** for a fault that has no key (a malformed pattern field, a non-mapping `conditions`, a malformed `$or` element) |
| `effect` | `"allow" \| "deny"` | The rule's effect — a finding on a `deny` rule is the consequential one |
| `sync_resolvable` | `boolean` | Whether the condition resolves for `check()`; **false** for a keyless structural fault |
| `async_resolvable` | `boolean` | Whether it resolves for `async_check()`; **false** for a keyless structural fault |

  Findings are ordered by `rule_index`, then lexicographically by `condition_path` — by path and not by key, because a nested `$or` may carry the same key at several positions, which leaves ordering by key undefined.

  The two flags MUST be reported separately and MUST NOT be collapsed into one boolean ([PROTOCOL_SPEC §6.1.3](../spec/protocol-spec.md#613-sync-and-async-handler-registries-v1220-100)). They mean **resolvable on that path**, not "present in that registry": since `async_check()` falls back to the sync registry, `async_resolvable` is the union of both, and every built-in leaf handler is resolvable on both paths. A finding with `sync_resolvable: false, async_resolvable: true` is an async-only handler — usable under `async_check()`, unevaluable under `check()`.

### Properties

- `async`: `false`.
- `thread_safe`: `true`.
- `pure`: `true`.
- `idempotent`: `true` — for a fixed rule list and registry.
- `reentrant`: `true`.

!!! tip "Fail the deployment, not the load"
    The intended shape is to call this once bootstrap has finished registering
    handlers, and to treat any finding on a `deny` rule as a startup error. The
    guarantee that a broken `deny` rule cannot silently pass traffic does not
    depend on anyone calling it — that is
    [§6.1.1](../spec/protocol-spec.md#611-unevaluable-conditions-v1220-100)'s job.
    This method exists so the problem is found at deploy time rather than in an
    audit log.

## Usage

=== "Python"
    ```python
    from apcore import ACL, ACLRule, APCore, Context, Identity

    # Load ACL from YAML
    acl = ACL.load("acl.yaml")

    # Check access
    identity = Identity(id="api.gateway", type="service", roles=["reader"])
    ctx = Context.create(identity=identity)
    allowed = acl.check("api.gateway", "db.query", ctx)          # bool
    decision = acl.check_access("api.gateway", "db.query", ctx)  # access + approval_required

    # Runtime modification
    acl.add_rule(ACLRule(
        callers=["admin.*"],
        targets=["*"],
        effect="allow",
        description="Admins can call any module",
    ))

    # Wire into the executor. set_acl() propagates the ACL to the pipeline's
    # acl_check step; assigning an attribute does not.
    client = APCore()
    client.executor.set_acl(acl)

    # set_acl() warns when the running strategy has no acl_check step, but the
    # warning is a one-shot log line. To OBSERVE the state at any later point:
    assert client.executor.governance_state().builtin_acl_gate_wired
    ```
=== "TypeScript"
    ```typescript
    import { ACL, APCore, Context, createIdentity } from "apcore-js";

    // Load ACL from YAML (synchronous)
    const acl = ACL.load("acl.yaml");

    // Check access
    const identity = createIdentity("api.gateway", "service", ["reader"]);
    const ctx = Context.create(identity);
    const allowed = acl.check("api.gateway", "db.query", ctx);          // boolean
    const decision = acl.checkAccess("api.gateway", "db.query", ctx);  // access + approvalRequired

    // Runtime modification — ACLRule is a plain object
    acl.addRule({
        callers: ["admin.*"],
        targets: ["*"],
        effect: "allow",
        description: "Admins can call any module",
    });

    // Wire into the executor. setAcl() propagates the ACL to the pipeline's
    // acl_check step; assigning a field does not.
    const client = new APCore();
    client.executor.setAcl(acl);
    console.assert(client.executor.governanceState().builtinAclGateWired);
    ```
=== "Rust"
    ```rust
    use std::collections::HashMap;

    use apcore::{ACLRule, APCore, Config, Context, Executor, Identity, ModuleError, Registry, ACL};
    use serde_json::Value;

    fn main() -> Result<(), ModuleError> {
        // Load ACL from YAML
        let mut acl = ACL::load("acl.yaml")?;

        // Check access
        let identity = Identity::new(
            "api.gateway".to_string(),
            "service".to_string(),
            vec!["reader".to_string()],
            HashMap::new(),
        );
        let ctx: Context<Value> =
            Context::create(Some(identity), None, None, None, Value::Null, None);
        let _allowed = acl.check(Some("api.gateway"), "db.query", Some(&ctx));
        let _decision = acl.check_access(Some("api.gateway"), "db.query", Some(&ctx), None);

        // Runtime modification — ACLRule is #[non_exhaustive]: build with new(),
        // then assign the optional fields.
        let mut rule = ACLRule::new(vec!["admin.*".to_string()], vec!["*".to_string()], "allow");
        rule.description = Some("Admins can call any module".to_string());
        acl.try_add_rule(rule)?;

        // `APCore` exposes only `executor() -> &Executor`, and set_acl needs &mut,
        // so attach the ACL before handing the Executor to APCore.
        let mut executor = Executor::new(Registry::default(), Config::default());
        executor.set_acl(acl);
        let client = APCore::with_options(None, Some(executor), None, None);
        assert!(client.executor().governance_state().builtin_acl_gate_wired);
        Ok(())
    }
    ```

!!! warning "An attached ACL is not an enforced one"
    `acl_check` is a pipeline step, and the `internal`, `testing` and `minimal` strategies all remove it — so `set_acl()` on an executor running one of those leaves the ACL attached and never consulted. [`governance_state()`](./core-executor.md#governance-state-api) reports `acl_configured` and `builtin_acl_gate_wired` separately for exactly this reason ([PROTOCOL_SPEC §6.6.5](../spec/protocol-spec.md#665-governance-state-query)).

## Dependencies

- **Context** — Provides `identity`, `call_chain` and the governance projection for conditional rule evaluation.
- **Identity** — `id`, `type` and `roles`, used by the `@system` pattern and the `identity_types` / `roles` conditions.
- **Error System** — `ACLRuleError` for invalid rules, `ConfigNotFoundError` for a missing file, `ConfigError` for an invalid `audit:` block, `ACLDeniedError` raised by the Executor on a deny.
- **Pattern matching** — `match_pattern()` (Algorithm A08) for non-special patterns.
- **Approval System** — Receives `approval_required` from the Executor's `acl_check` step.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `src/apcore/acl.py` | `ACLRule`, `AccessDecision`, `AuditEntry`, and the `ACL` class (loading, evaluation, audit delivery, runtime management) |
    | `src/apcore/acl_handlers.py` | Built-in condition handlers (`identity_types`, `roles`, `max_call_depth`, `arguments`, `$or`, `$not`) |
    | `src/apcore/utils/pattern.py` | `match_pattern()` wildcard utility (Algorithm A08) |

## Testing Strategy

- **Pattern matching**: `@external` matches a missing `caller_id` (and not a string caller), `@system` matches system-type identities (and fails for none or non-system identities), exact patterns, wildcard `*`, prefix wildcards like `executor.*`, and the closed pattern-array shape.
- **First-match-wins evaluation**: the first matching allow grants, the first matching deny refuses, and rule order takes precedence over specificity.
- **Default effect**: both `default_effect: deny` and `default_effect: allow` when no rule matches; a missing ACL file attaches no ACL.
- **YAML loading**: rules with descriptions, conditions and `approval`; errors for a missing file, invalid YAML, missing or non-list `rules`, missing or unknown rule keys, invalid `effect` / `approval` values, `approval: required` on a `deny` rule, and an invalid `audit:` block.
- **Conditional rules**: `identity_types`, `roles`, `max_call_depth` and `arguments` matching and failing; unevaluable conditions (unknown key, malformed value, raising handler, async handler on the sync path) deny on a `deny` rule and do not grant on an `allow` rule.
- **Approval**: `check()` returns false for an allowed call that needs approval, while `check_access()` reports `access: allow, approval_required: true`.
- **Audit delivery**: callback vs default sink selection, `include_denied`, a failing sink not changing the decision, and `reload()` re-applying the `audit:` block.
- **Runtime modification**: `add_rule()` inserts at position 0, `remove_rule()` returns true/false, `reload()` re-reads the YAML file and updates rules.
- **Thread safety**: concurrent `check()` calls, and concurrent `add_rule()` + `check()`, with no corruption.
- **Integration**: ACL enforcement end to end through the Executor pipeline.

Cross-language cases live in `conformance/fixtures/acl_*.json` (evaluation, rule-key closure, effect closure, pattern arity, handler errors, argument-scoped approval, audit delivery, root discovery).
