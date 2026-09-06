---
description: "Pattern-based ACL with first-match-wins rule evaluation for inter-module access control: wildcard and @external/@system patterns, identity/role/depth conditions, default-deny, YAML hot-reload."
---

# Access Control System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §6 ACL Specification.


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

These three are the built-ins. The set is open: `register_condition()` adds a condition key at runtime, and a rule may reference any key a handler has been registered for.

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

**An unevaluable `allow` rule does not take its approval requirement with it (spec v1.29.0).** "Does not grant" means the rule steps aside, and a rule now carries two axes. If it carried `approval: required`, the requirement is recorded as **pending** and composed by disjunction with whatever grants later — a subsequent `allow` rule, or `default_effect: allow`. A final `deny` clears it, and `matched_rule_index` keeps naming the rule that actually decided. A rule whose `callers`/`targets` do not match this call raises nothing; a rule whose pattern field is itself malformed does, because its scope cannot be read and so cannot be shown not to apply here.

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

!!! danger "A misspelled condition key used to make a `deny` rule inert"
    Before spec v1.22.0 an unevaluable condition made the rule *not match*, so
    `deny` rules failed open: `role:` written for `roles:` produced a rule that
    blocked nothing, and the call fell through to the next rule or to
    `default_effect`. Warnings were emitted the whole time — the diagnostics were
    right and only the decision was wrong.

### Components

- **`ACLRule`** -- Dataclass with fields: `callers` (list of patterns), `targets` (list of patterns), `effect` ("allow" or "deny"), optional `description`, and optional `conditions` dict.
- **`ACL`** -- Main class managing an ordered rule list. Provides `check()`, `add_rule()`, `remove_rule()`, `reload()`, the read-only accessors `default_effect` and `rules`, the diagnostic `validate_rules()`, and the `ACL.load()` classmethod for YAML loading. All mutating methods are protected by a lock for thread safety.
- **`AuditEntry`** -- Structured record of one `check()` decision, emitted through the configured audit logger on every call. Field contract: [PROTOCOL_SPEC §6.3.1](../spec/protocol-spec.md#631-audit-entry).
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
   - `["$or", p1, p2, ...]`: matches if **any** of `p1, p2, …` match the module ID. (This is observably equivalent to a flat list, which is already OR-ed; the explicit form documents intent.) At least one operand.
   - `["$not", p]`: matches if `p` does **not** match the module ID. **Exactly one** operand.

**Only the first form nests.** A pattern array is **flat**: there is one operator position — index 0 — and every element after it is a plain pattern string, never a nested array and never another operator. `["$or", "$not", "a"]` is *not* or-of-not, and `["api.*", "$not", "cli.*"]` is *not* "api.* but not cli.*"; both are rejected. This is the difference that catches people out, because the same two tokens nest arbitrarily inside `conditions` (`$or[1].$not.k` is a defined path there — [PROTOCOL_SPEC §6.1.4](../spec/protocol-spec.md#614-structural-and-registry-precheck-v1250-100)).

**The array's shape is a closed set, rejected with `ACLRuleError` at every entry point** — file loading, direct construction and runtime insertion ([PROTOCOL_SPEC §6.2.1](../spec/protocol-spec.md#621-compound-operators-in-pattern-arrays)): at least one element, every element a non-empty string, `$or` with at least one operand, `$not` with exactly one, and `$or` / `$not` nowhere but index 0. Before v1.31.0 each of these made the rule match nothing instead — which is harmless on an `allow` rule and, on a `deny` rule under `default_effect: allow`, permits the call the rule was written to block.

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
```

!!! warning "That two-rule form is not a drop-in replacement inside an existing rule list"
    `["$not", p]` makes the rule **not match** `p`, so evaluation **continues** and a later
    rule may still decide the call. A leading `deny` on `p` **ends** the scan. They agree
    only when nothing after the rule could have matched `p` and `default_effect` would have
    refused it anyway — true of the complete policy above, not true in general. Rewriting a
    rule into this form changes the policy's order, not just one field.

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
4. Evaluate rules in order (first-match-wins). A rule's conditions resolve to one of three outcomes — satisfied, unsatisfied, or unevaluable — and an unevaluable condition resolves the rule toward refusing access (§ Unevaluable Conditions above).
5. Emit an audit event carrying the decision (via the finalize path). When a condition was unevaluable, `handler_error` on that entry MUST be non-null and MUST name the condition key and the reason.

### Errors

- None under normal operation. `check` MUST NOT raise to indicate a deny; it MUST return `false`. Raising is reserved for unrecoverable internal failures (e.g., a corrupted rule list) that the host language's idioms require be surfaced as exceptions.
- An unevaluable condition is NOT such a failure: it MUST NOT propagate out of `check()`. A handler that raises, throws, or panics MUST be caught, recorded in `handler_error`, and resolved per § Unevaluable Conditions.

### Returns

- On success: plain `bool` (`true` = allow, `false` = deny). The return type MUST NOT be wrapped in a `Result`/`Either` type.

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

    !!! warning "Changed in spec v1.22.0"
        Through v1.21.0 this case was specified as "treated as unsatisfied", which
        made a `deny` rule guarded by an async-only handler **inert** on the sync
        path — the same failure mode as a misspelled key. It is now one of §6.1.1's
        three unevaluable situations.

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
- A warning is emitted for every rule that references a condition key with no handler registered **at load time**, naming the rule index, the key, and the rule's `effect`. The load still succeeds — see Errors below.

### Errors

- `ConfigNotFoundError(config_path=yaml_path)` — file does not exist at `yaml_path`.
- `ACLRuleError` — YAML parse failure, top-level value is not a mapping, `rules` key is absent, `rules` value is not a list, any rule entry is not a mapping, any rule is missing a required key (`callers`, `targets`, or `effect`), `effect` value is not `"allow"` or `"deny"`, `callers`/`targets` value is not a list, or a `callers`/`targets` array's shape is outside [§6.2.1](../spec/protocol-spec.md#621-compound-operators-in-pattern-arrays)'s closure (empty, an empty element, `$or` with no operands, `$not` with none or more than one, or a reserved token away from index 0).
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

Config-driven activation of the `acl.root` key (decision D-64). `discover()` resolves `acl.root` and loads an ACL **only when the configured path exists**, so that ACL enforcement can be turned on by configuration alone — without application code calling `ACL.load()` + `set_acl()` by hand. The application bootstrap (`APCore`) calls `discover()` automatically and attaches the result.

!!! danger "Missing-path invariant — MUST NOT synthesize a default-deny ACL"
    When the resolved `acl.root` path does **not** exist, `discover()` MUST return "no ACL" (`None`/`null`/`Option::None`) and attach nothing. It MUST NOT construct an empty ACL — an empty ACL with `default_effect: deny` would deny **every** inter-module call in every project that has no ACL today. A missing path means *no enforcement*, identical to the pre-D-64 default. `acl.default_effect` takes effect only once a real ACL file is loaded.

### Inputs

- `config`: the loaded `Config`, required. `acl.root` is read from it.
  - `acl.root` default: `"./acl"` in all SDKs (Rust no longer hard-requires the key).

### Preconditions

- None. `acl.root` MAY be unset (the default applies) and MAY point at a path that does not exist (no-op).

### Side Effects (ordered)

1. Read `acl.root` from `config` (apply the `"./acl"` default if unset).
2. Resolve the path: relative to the config file's directory when the `Config` knows its source path, else relative to the current working directory. **This is the current (1.x) behaviour and is superseded at 2.0** — see the note below.
3. If the resolved path is a **directory**, target `<root>/global_acl.yaml` (the `acl/{scope}_acl.yaml` convention, PROTOCOL_SPEC §3.1); if it is a **file**, target it directly.
4. If the target file exists, load it via `ACL.load()` and return the new `ACL`.
5. If the resolved path / target file does not exist, return "no ACL" and attach nothing.

!!! warning "Step 2's base is superseded at 2.0 by PROTOCOL_SPEC §9.2.2"
    Step 2 records what `discover()` does **today**, and it is unchanged for the whole 1.x
    line — implement it exactly as written. It is also the reason `acl.root` is the one
    path-typed key (PROTOCOL_SPEC §9.2.1) whose base differs from its siblings: `schema.root`
    and `extensions.root` resolve against the process CWD, unconditionally.

    [PROTOCOL_SPEC §9.2.2 Path Resolution Base](../spec/protocol-spec.md#922-path-resolution-base)
    declares the target rule and opens the deprecation window (spec v1.35.0, issue #113): from
    2.0, every relative path-typed value resolves against a single **project root** — the
    configuration file's directory when that file came from §9.14 discovery tiers 1-5, and the
    process CWD when it came from the user-level tiers 6-7 or when no file was found.

    For a project-local config (tiers 2-5) the two rules coincide and nothing changes. The
    difference is the **user-level tiers**, where D-64's rule is actively wrong: a config at
    `~/.config/apcore/config.yaml` carrying `acl.root: ./acl` loads its policy from
    `~/.config/apcore/acl/` into every project that user runs, while the project's own `./acl/`
    is ignored — the inverse of what a default-deny ACL is for. Do **not** change this
    behaviour before 2.0; the §9.2.2 migration path is the supported route.

### Postconditions

- Returns a loaded `ACL` **iff** the resolved target file exists; otherwise returns the language's "no ACL" value.
- No empty/synthesized ACL is ever returned for a missing path.
- Skipped entirely when the caller supplies their own `Executor` to `APCore`, so an explicitly-wired ACL is never overwritten.

### Errors

- `ACLRuleError` — only when a target file **exists but is structurally invalid** (propagated from `ACL.load()`). A missing path is never an error.

### Returns

- A new `ACL` instance, or the language "no ACL" value (`None` / `null` / `Option::None`).

### Properties

- `async`: `false`
- `thread_safe`: `true` — creates a new instance; no shared mutable state accessed
- `pure`: `false` — reads from the filesystem
- `idempotent`: `true`
- `reentrant`: `true`

### Usage

With `acl.root` set in `apcore.yaml`, enforcement is wired automatically — no manual `ACL.load()` / `set_acl()` needed:

```yaml
# apcore.yaml
acl:
  root: ./acl            # directory holding global_acl.yaml (default: ./acl)
  default_effect: deny   # applies only once an ACL file is actually loaded
```

=== "Python"
    ```python
    from apcore import APCore, Config

    # APCore calls ACL.discover(config) and attaches the result.
    # If ./acl/global_acl.yaml exists, enforcement is active; if not, it is a no-op.
    app = APCore(config=Config.load("apcore.yaml"))

    # Equivalent explicit form (also still supported):
    from apcore.acl import ACL
    acl = ACL.discover(Config.load("apcore.yaml"))
    if acl is not None:
        app.executor.set_acl(acl)
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config, ACL } from 'apcore-js';

    // APCore calls ACL.discover(config) and attaches the result.
    const app = new APCore({ config: Config.load('apcore.yaml') });

    // Equivalent explicit form:
    const acl = ACL.discover(Config.load('apcore.yaml'));
    if (acl !== null) {
      app.executor.setAcl(acl);
    }
    ```
=== "Rust"
    ```rust
    use apcore::{APCore, config::Config, acl::ACL};

    // APCore calls ACL::discover(&config) and attaches the result.
    let config = Config::load("apcore.yaml")?;
    let app = APCore::new(config.clone())?;

    // Equivalent explicit form:
    if let Some(acl) = ACL::discover(&config)? {
        // set_acl needs &mut Executor; `executor()` yields &Executor.
        let mut executor = Executor::new(registry.clone(), config.clone());
        executor.set_acl(acl);
    }
    ```

## Contract: ACL.add_rule

### Inputs

- `rule`: pre-built `ACLRule` to insert at the front of the rule list (highest priority).

> **Cross-language ergonomic note (D10-006).** Python additionally exposes a
> kwargs-form overload `add_rule(*, callers, targets, effect="deny",
> description="", conditions=None)` that constructs the rule on the caller's
> behalf. This kwargs surface is **Python-only** — TypeScript and Rust callers
> use struct/object literals to build `ACLRule` directly, which is already
> idiomatic in those languages and offers equivalent ergonomics. The
> kwargs path is therefore not normative for cross-language conformance;
> only the prebuilt-rule form is required across SDKs.

### Preconditions

- `rule` is a well-formed `ACLRule` (callers + targets non-empty, effect ∈ {"allow", "deny"}). Since spec v1.31.0 this is **enforced, not assumed**: `add_rule` re-validates the rule it is handed — including one that was well-formed when constructed and has since had `callers` or `targets` assigned — and raises `ACLRuleError` ([§6.2.1](../spec/protocol-spec.md#621-compound-operators-in-pattern-arrays)). Validation order within a rule is `effect` → `approval` → `callers` / `targets`.

### Side Effects (ordered)

1. Acquire the ACL lock.
2. Insert the rule at index 0 of the internal rule list (highest priority).
3. Release the ACL lock.
4. If the rule carries `conditions`, check each key — including keys nested inside `$or` / `$not` — against the handler registries, and emit a warning for every key that does not resolve on the sync path. The warning MUST name the rule index (`0`), the key, and the rule's `effect`. Insertion still succeeds: this is the same warn-never-fail contract [`load()`](#contract-aclload) has, for the same reason ([PROTOCOL_SPEC §6.1.2](../spec/protocol-spec.md#612-load-time-validation-of-condition-keys-v1220-100) rule 4 makes runtime insertion an entry point that MUST be covered).

### Postconditions

- The rule is the first entry in the rule list; all prior rules shift up by one index.
- Any subsequent `check()` call evaluates the new rule before all previously inserted rules.
- A warning has been emitted for each unresolvable condition key the rule references. No exception is raised for one.

### Errors

- `ValueError` (Python kwargs path only) — when `rule` is `None` and either `callers` or `targets` is also `None`. Not raised on the prebuilt-rule path used uniformly across SDKs.

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

    # Wire into executor via APCore.
    # Use set_acl(): it propagates the ACL to the pipeline's acl_check step.
    # Plain attribute assignment does NOT wire enforcement.
    client = APCore()
    client.executor.set_acl(acl)

    # set_acl() warns when the running strategy has no acl_check step, but the
    # warning is a one-shot log line. To OBSERVE the state at any later point:
    assert client.executor.governance_state().builtin_acl_gate_wired
    ```

> **An attached ACL is not an enforced one.** `acl_check` is a pipeline step, and the `internal`, `testing` and `minimal` strategies all remove it — so `set_acl()` on an executor running one of those leaves the ACL attached and never consulted. [`governance_state()`](./core-executor.md#governance-state-api) reports `acl_configured` and `builtin_acl_gate_wired` separately for exactly this reason ([PROTOCOL_SPEC §6.6.5](../spec/protocol-spec.md#665-governance-state-query)).
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";
    import { ACL, ACLRule } from "apcore-js";
    import { Context, Identity } from "apcore-js";

    // Load ACL from YAML
    const acl = await ACL.load("acl.yaml");

    // Check access
    const identity: Identity = { id: "api.gateway", type: "service", roles: ["reader"] };
    const ctx = Context.create(identity);
    const allowed = acl.check("api.gateway", "db.query", ctx);

    // Runtime modification
    // ACLRule is an interface — pass a plain object literal.
    acl.addRule({
        callers: ["admin.*"],
        targets: ["*"],
        effect: "allow",
        description: "Admins can call any module",
    });

    // Wire into executor via APCore.
    // Use setAcl(): it propagates the ACL to the pipeline's acl_check step.
    // Plain field assignment does NOT wire enforcement.
    const client = new APCore();
    client.executor.setAcl(acl);
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
    let ctx = Context::create(Some(identity), None, None, None, Value::Null, None);
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
    // `APCore` exposes only `executor() -> &Executor`, and set_acl needs &mut.
    // Build the Executor yourself when you need to attach an ACL after the fact:
    let mut executor = Executor::new(registry.clone(), config.clone());
    executor.set_acl(acl);
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
