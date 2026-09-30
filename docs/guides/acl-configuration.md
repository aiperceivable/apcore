---
description: "How to write and wire apcore ACL files: callers, targets, allow/deny effects, conditions, approval, audit, default-deny, and how to test the rules."
---

# ACL Configuration Guide

> Configure access control rules between modules.

!!! note "Cross-language applicability"
    The ACL file format (YAML) is identical in every SDK. SDK code is shown side by side for Python, TypeScript and Rust. Evaluation semantics — pattern matching, conditions, unevaluable conditions, audit records — are defined in the [ACL System feature spec](../features/acl-system.md) and [PROTOCOL_SPEC §6](../spec/protocol-spec.md#6-acl-specification); this guide shows how to use them.

## 1. Overview

The ACL (Access Control List) decides whether one module may call another. It runs as pipeline step 4, after module lookup and before the approval gate.

| Concept | Description |
|---------|-------------|
| **Callers** | Caller module ID patterns (`caller_id`); `@external` matches top-level calls |
| **Targets** | Target module ID patterns (`target_id`) |
| **Effect** | `allow` or `deny` |
| **Conditions** | Optional extra requirements on identity, call depth or argument keys |
| **Approval** | Optional `approval: required` — the call is allowed but must be signed off first |
| **Default effect** | What happens when no rule matches — always `deny` in these examples |
| **Audit** | The file's `audit:` block records every decision |

---

## 2. Quick Start

### 2.1 Create the ACL file

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["@external"]
    targets: ["api.*"]
    effect: allow
    description: "Top-level callers enter through the API layer"

  - callers: ["api.*"]
    targets: ["orchestrator.*"]
    effect: allow

  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  - callers: ["*"]
    targets: ["common.*"]
    effect: allow
    description: "Shared utilities"
```

Every call not allowed by a rule is denied by `default_effect: deny`.

### 2.2 Wire it through `apcore.yaml` (recommended)

Point `acl.root` at the directory that holds the file. When `APCore` is built with a `Config`, it loads `<acl.root>/global_acl.yaml` and attaches it:

```yaml
# apcore.yaml
version: "1.0.0"
project:
  name: my-app
acl:
  root: ./acl   # the default; relative to apcore.yaml
```

=== "Python"

    ```python
    from apcore import APCore, Config

    client = APCore(config=Config.load("apcore.yaml"))
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const client = new APCore({ config: Config.load('apcore.yaml') });
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, ModuleError};

    fn build_client() -> Result<APCore, ModuleError> {
        APCore::from_path("apcore.yaml")
    }
    ```

If the file does not exist, **no ACL is attached and no call is checked** — the SDK does not invent a deny-all ACL. `default_effect` and `audit:` belong in the ACL file; the `acl.default_effect` and `acl.audit.*` keys in `apcore.yaml` have no effect. Discovery is skipped when you pass your own `Executor` to `APCore`.

### 2.3 Wire it in code

Load the file yourself when you need a custom audit destination or build the `Executor` yourself. `audit_logger` receives every decision; when you pass one, drop the `audit:` block from the file (the SDK warns that its settings no longer apply).

=== "Python"

    ```python
    import logging

    from apcore import APCore
    from apcore.acl import ACL, AuditEntry

    audit_log = logging.getLogger("acl.audit")


    def log_decision(entry: AuditEntry) -> None:
        audit_log.info(
            "ACL %s: %s -> %s (%s, rule=%s)",
            entry.decision, entry.caller_id, entry.target_id, entry.reason, entry.matched_rule,
        )


    client = APCore()
    client.executor.set_acl(ACL.load("./acl/global_acl.yaml", audit_logger=log_decision))
    ```

=== "TypeScript"

    ```typescript
    import { ACL, APCore } from 'apcore-js';
    import type { AuditEntry } from 'apcore-js';

    const logDecision = (entry: AuditEntry): void => {
      console.info(
        `ACL ${entry.decision}: ${entry.callerId} -> ${entry.targetId} (${entry.reason}, rule=${entry.matchedRule})`,
      );
    };

    const client = new APCore();
    client.executor.setAcl(ACL.load('./acl/global_acl.yaml', logDecision));
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, AuditEntry, Config, Executor, ModuleError, Registry, ACL};
    use std::sync::Arc;

    fn build_client() -> Result<APCore, ModuleError> {
        let mut acl = ACL::load("./acl/global_acl.yaml")?;
        acl.set_audit_logger(|entry: &AuditEntry| {
            println!(
                "ACL {}: {} -> {} ({}, rule={:?})",
                entry.decision, entry.caller_id, entry.target_id, entry.reason, entry.matched_rule
            );
        });

        // APCore::executor() is read-only, so attach the ACL before handing the Executor over.
        let mut executor = Executor::new(Arc::new(Registry::new()), Arc::new(Config::default()));
        executor.set_acl(acl);
        Ok(APCore::with_options(None, Some(executor), None, None))
    }
    ```

### 2.4 Check that the ACL is enforced

An attached ACL is only consulted by strategies that contain the `acl_check` step. The `standard` strategy (the default) has it; the `internal`, `testing` and `minimal` strategies do not, so an ACL attached there is never evaluated. `governance_state()` reports both facts:

=== "Python"

    ```python
    state = client.executor.governance_state()
    assert state.acl_configured and state.builtin_acl_gate_wired
    ```

=== "TypeScript"

    ```typescript
    const state = client.executor.governanceState();
    console.assert(state.aclConfigured && state.builtinAclGateWired);
    ```

=== "Rust"

    ```rust
    let state = client.executor().governance_state();
    assert!(state.acl_configured && state.builtin_acl_gate_wired);
    ```

---

## 3. Configuration Format

### 3.1 Basic Structure

```yaml
# acl/global_acl.yaml
version: "1.0"          # optional
default_effect: deny    # optional; deny when omitted

audit:                  # optional; declaring it turns on the SDK's audit log
  enabled: true
  include_denied: true
  log_level: info       # trace | debug | info | warn | error

rules:                  # required; evaluated top to bottom, first match wins
  - callers: ["<pattern>"]
    targets: ["<pattern>"]
    effect: allow            # allow | deny
    description: "Optional rule description"
```

The file is validated against [`schemas/acl-config.schema.json`](https://github.com/aiperceivable/apcore/blob/main/schemas/acl-config.schema.json). An invalid file — including an unknown rule key or an `effect` other than `allow`/`deny` — fails to load with `ACL_RULE_ERROR`.

### 3.2 Rule Fields

| Field | Required | Description |
|------|------|------|
| `callers` | Yes | Caller module ID patterns (see section 4) |
| `targets` | Yes | Target module ID patterns |
| `effect` | Yes | `allow` or `deny` |
| `description` | No | Human-readable purpose; recorded in audit entries as the matched rule |
| `conditions` | No | Extra requirements the call must meet for the rule to match (section 3.3) |
| `approval` | No | `required` or `not_required` (default). `required` lets the call through the ACL but sends it to the approval gate; only valid with `effect: allow` |

### 3.3 Conditions

All keys in a `conditions` object must hold for the rule to match.

| Key | Matches when |
|-----|-------------|
| `roles` | The caller's `Identity` has at least one of the listed roles |
| `identity_types` | The caller's identity type is one of the listed values |
| `max_call_depth` | The call chain is no deeper than this value |
| `arguments` | The call's argument **keys** satisfy `has_key`, `has_all_keys` or `has_none_of` |
| `$or` | Any of the listed condition objects holds |
| `$not` | The wrapped condition object does not hold |

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["agent.*"]
    targets: ["billing.refund"]
    effect: allow
    approval: required
    conditions:
      $or:
        - roles: ["finance"]
        - identity_types: ["service"]
      $not:
        arguments:
          has_key: ["override_limit"]
    description: "Finance agents and services may refund, with sign-off, but never with override_limit"
```

Custom condition keys can be registered in code (section 8.2). A condition that cannot be evaluated — no handler registered, or the handler fails — never grants access: an `allow` rule carrying it does not match, and a `deny` rule carrying it takes effect. See [features/acl-system.md § Conditional Rules](../features/acl-system.md#conditional-rules) for the full semantics.

### 3.4 Audit

The `audit:` block is the only place audit logging is configured.

- **Declared with `enabled: true`** — the SDK logs one `apcore.acl.audit` record per decision at `log_level`. `include_denied: false` drops denied decisions from that log.
- **Not declared** — no audit output unless you pass a callback (section 2.3).
- **A callback** receives every decision, allow and deny, regardless of the block.

Keep auditing on in production: the records are the durable trail of every access decision that security review and incident response depend on.

---

## 4. Pattern Matching

`*` matches any run of characters, including dots, and is the only wildcard. Every other character — `?` included — is literal. Matching is case-sensitive.

| Pattern | Matches | Does not match |
|---------|---------|----------------|
| `executor.email.send_email` | exactly that ID | `executor.email.send_template` |
| `executor.*` | `executor.email.send_email`, `executor.db.insert` | `executor` |
| `executor.email.*` | `executor.email.send_email` | `executor.sms.send` |
| `*.read` | `executor.user.read`, `billing.read` | `executor.user.read_all` |
| `*` | every ID | — |
| `@external` (callers only) | top-level calls with no `caller_id` | calls from a module |
| `@system` (callers only) | calls whose identity type is `system` | — |

`callers` and `targets` lists may also start with the `$or` / `$not` operators; see [features/acl-system.md § Pattern Matching](../features/acl-system.md#pattern-matching).

---

## 5. Common Configuration Patterns

### 5.1 Layered Architecture

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["@external"]
    targets: ["api.*"]
    effect: allow

  - callers: ["api.*"]
    targets: ["orchestrator.*"]
    effect: allow

  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  - callers: ["*"]
    targets: ["common.*"]
    effect: allow

  # Already denied by default_effect; stated explicitly so the audit
  # entry names the reason.
  - callers: ["api.*"]
    targets: ["executor.*"]
    effect: deny
    description: "API cannot call Executor directly"
```

### 5.2 Allowlist of Specific Calls

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["orchestrator.user.register"]
    targets: ["executor.email.send_email", "executor.database.insert"]
    effect: allow

  - callers: ["orchestrator.order.create"]
    targets: ["executor.payment.charge"]
    effect: allow
```

### 5.3 Carve-outs Inside a Broad Allow

First match wins, so an exception goes **above** the broader rule it narrows.

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  # 1. The one module allowed into the key vault.
  - callers: ["admin.security_audit"]
    targets: ["vault.keys.*"]
    effect: allow

  # 2. Nobody else, even callers the next rule would allow.
  - callers: ["*"]
    targets: ["vault.keys.*"]
    effect: deny
    description: "Key vault modules are closed to everything but the audit job"

  # 3. The broad rule.
  - callers: ["admin.*"]
    targets: ["vault.*"]
    effect: allow
```

### 5.4 Development-Only Rules

Keep permissive rules in a separate ACL file that only development configs point at (`acl.root: ./acl-dev`), never in the production file.

```yaml
# acl-dev/global_acl.yaml — development only
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: debug

rules:
  - callers: ["dev.*"]
    targets: ["*"]
    effect: allow
    description: "Development tools may call any module"

  - callers: ["*"]
    targets: ["mock.*"]
    effect: allow
    description: "Mock modules are callable by anyone in development"
```

---

## 6. Rule Evaluation

### 6.1 Matching Order

Rules are checked in file order and the **first rule whose caller and target patterns (and conditions) match** decides. If none matches, `default_effect` decides.

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  # Rule 1: exact match
  - callers: ["orchestrator.user.register"]
    targets: ["executor.email.send_email"]
    effect: allow

  # Rule 2: wildcard
  - callers: ["orchestrator.*"]
    targets: ["executor.email.*"]
    effect: deny
```

```text
Call: orchestrator.user.register → executor.email.send_email
Match: Rule 1 → allow

Call: orchestrator.order.create → executor.email.send_email
Match: Rule 2 → deny

Call: api.handler.test → common.util.format
Match: none → default_effect → deny
```

### 6.2 Best Practices

- Keep `default_effect: deny` and grant only what is needed.
- Put exceptions (exact IDs, narrow patterns) above the broad rules they carve out of.
- Give every `deny` rule and every non-obvious `allow` rule a `description`; it appears in audit entries.
- Test the file (section 10.2) whenever it changes.

### 6.3 Matching Walkthrough

```text
Check: caller="api.handler.user" target="executor.email.send"

Rule 1: callers=["admin.*"] targets=["*"]
  caller match: "admin.*" vs "api.handler.user" → ✗ no match
  → skip

Rule 2: callers=["api.*"] targets=["executor.*"]
  caller match: "api.*" vs "api.handler.user" → ✓ match
  target match: "executor.*" vs "executor.email.send" → ✓ match
  → hit! effect=allow

Result: allow
```

### 6.4 Edge Cases

| Scenario | Result |
|------|------|
| No ACL file under `acl.root` (or no ACL attached in code) | No ACL check at all — the ACL step lets every call through |
| ACL file with an empty `rules` list | Every call gets `default_effect` |
| `caller_id` is null (top-level call) | Matched as `@external` |
| Module calls itself | Checked like any other call; the call-chain guard limits recursion separately |
| Rule has `conditions` but the call has no context | The rule does not match (a warning is logged) |
| A condition cannot be evaluated | An `allow` rule does not grant; a `deny` rule takes effect |
| Rule matches with `approval: required` | The ACL allows the call and the approval gate must then sign it off |

---

## 7. Runtime Behavior

### 7.1 Check Timing

```text
client.call(module_id, inputs, context)
    │
    ├─ 1–3. Context creation, call-chain guard, module lookup
    │
    ├─ 4. ACL check
    │      └─ caller_id = context.caller_id (null → @external)
    │      └─ acl.check(caller_id, target_id, context)
    │      └─ denied → ACLDeniedError (ACL_DENIED)
    │
    └─ 5. Approval gate, then middleware, validation, execution…
```

### 7.2 Error Handling

=== "Python"

    ```python
    from apcore.errors import ACLDeniedError

    try:
        client.call("vault.read_secret", {"key": "value"})
    except ACLDeniedError as e:
        print(f"Access denied: {e.caller_id} -> {e.target_id} ({e.code})")
    ```

=== "TypeScript"

    ```typescript
    import { ACLDeniedError } from 'apcore-js';

    try {
      await client.call('vault.read_secret', { key: 'value' });
    } catch (e) {
      if (!(e instanceof ACLDeniedError)) throw e;
      console.log(`Access denied: ${e.callerId} -> ${e.targetId} (${e.code})`);
    }
    ```

=== "Rust"

    ```rust
    use apcore::ErrorCode;
    use serde_json::json;

    match client.call("vault.read_secret", json!({"key": "value"}), None, None).await {
        Ok(result) => println!("{result}"),
        Err(e) if e.code == ErrorCode::ACLDenied => {
            // caller_id / target_id travel in the error's details.
            println!("Access denied: {:?} -> {:?}", e.details.get("caller_id"), e.details.get("target_id"));
        }
        Err(e) => return Err(e),
    }
    ```

### 7.3 Debug Logging

Python and TypeScript ACLs have a `debug` flag that logs each rule evaluation inside `check()` — useful while writing rules, too noisy for production:

=== "Python"

    ```python
    from apcore.acl import ACL

    acl = ACL.load("./acl/global_acl.yaml")
    acl.debug = True
    ```

=== "TypeScript"

    ```typescript
    import { ACL } from 'apcore-js';

    const acl = ACL.load('./acl/global_acl.yaml');
    acl.debug = true;
    ```

For a record of decisions rather than evaluation steps, use the audit block or an audit callback (section 3.4).

---

## 8. Dynamic ACL

### 8.1 Runtime Modification

=== "Python"

    ```python
    from apcore.acl import ACL, ACLRule

    acl = ACL.load("./acl/global_acl.yaml")

    # Insert at position 0 — highest priority.
    acl.add_rule(ACLRule(
        callers=["temp.module"],
        targets=["executor.*"],
        effect="allow",
        description="Temporary debug rule",
    ))

    # Remove the first rule with these callers and targets.
    removed = acl.remove_rule(callers=["temp.module"], targets=["executor.*"])

    # Re-read the file from disk.
    acl.reload()
    ```

=== "TypeScript"

    ```typescript
    import { ACL } from 'apcore-js';

    const acl = ACL.load('./acl/global_acl.yaml');

    // Insert at position 0 — highest priority.
    acl.addRule({
      callers: ['temp.module'],
      targets: ['executor.*'],
      effect: 'allow',
      description: 'Temporary debug rule',
    });

    // Remove the first rule with these callers and targets.
    const removed: boolean = acl.removeRule(['temp.module'], ['executor.*']);

    // Re-read the file from disk.
    acl.reload();
    ```

=== "Rust"

    ```rust
    use apcore::{ACLRule, ModuleError, ACL};

    fn adjust_rules() -> Result<(), ModuleError> {
        let mut acl = ACL::load("./acl/global_acl.yaml")?;

        // ACLRule is #[non_exhaustive]: build it with new() and set optional fields.
        let mut rule = ACLRule::new(vec!["temp.module".to_string()], vec!["executor.*".to_string()], "allow");
        rule.description = Some("Temporary debug rule".to_string());
        acl.add_rule(rule); // inserted at position 0 — highest priority

        // Remove the first rule with these callers and targets.
        let removed: bool = acl.remove_rule(&["temp.module".to_string()], &["executor.*".to_string()]);
        println!("removed: {removed}");

        // Re-read the file from disk.
        acl.reload()?;
        Ok(())
    }
    ```

An ACL attached to an executor is shared: changes made through the object you attached apply to later calls.

### 8.2 Custom Conditions

For decisions that depend on time, identity attributes or external lookups, register a condition handler and reference its key from `conditions`. This one limits `maintenance.*` modules to a nightly window.

=== "Python"

    ```python
    from datetime import datetime
    from typing import Any

    from apcore.acl import ACL
    from apcore.context import Context


    class TimeWindowHandler:
        """Satisfied when the current hour is within [start_hour, end_hour]."""

        def evaluate(self, value: Any, context: Context) -> bool:
            if not isinstance(value, dict):
                return False
            hour = datetime.now().hour
            return int(value.get("start_hour", 0)) <= hour <= int(value.get("end_hour", 23))


    ACL.register_condition("time_window", TimeWindowHandler())
    ```

=== "TypeScript"

    ```typescript
    import { ACL, Context } from 'apcore-js';

    class TimeWindowHandler {
      evaluate(value: unknown, _context: Context): boolean {
        if (typeof value !== 'object' || value === null) return false;
        const { start_hour = 0, end_hour = 23 } = value as { start_hour?: number; end_hour?: number };
        const hour = new Date().getHours();
        return hour >= start_hour && hour <= end_hour;
      }
    }

    ACL.registerCondition('time_window', new TimeWindowHandler());
    ```

=== "Rust"

    ```rust
    // Cargo.toml: apcore, async-trait, chrono, serde_json
    use apcore::{ACLConditionHandler, Context, ACL};
    use async_trait::async_trait;
    use chrono::{Local, Timelike};
    use serde_json::Value;
    use std::sync::Arc;

    struct TimeWindowHandler;

    #[async_trait]
    impl ACLConditionHandler for TimeWindowHandler {
        async fn evaluate(&self, value: &Value, _ctx: &Context<Value>) -> bool {
            let Some(obj) = value.as_object() else { return false };
            let start = obj.get("start_hour").and_then(Value::as_u64).unwrap_or(0);
            let end = obj.get("end_hour").and_then(Value::as_u64).unwrap_or(23);
            let hour = u64::from(Local::now().hour());
            (start..=end).contains(&hour)
        }
    }

    fn register_conditions() {
        ACL::register_condition("time_window", Arc::new(TimeWindowHandler));
    }
    ```

Reference the key from the ACL file:

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["scheduler.*"]
    targets: ["maintenance.*"]
    effect: allow
    conditions:
      time_window: { start_hour: 2, end_hour: 6 }
    description: "Maintenance jobs run only between 02:00 and 06:59"
```

Register handlers before the first call. Loading a file that references an unregistered key only warns; until the handler exists, the condition is unevaluable and the `allow` rule never grants.

---

## 9. Configuration Examples

### 9.1 Microservice Architecture

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["@external"]
    targets: ["gateway.*"]
    effect: allow
    description: "External callers can only reach the gateway"

  - callers: ["gateway.*"]
    targets: ["service.*"]
    effect: allow

  - callers: ["service.*"]
    targets: ["repository.*", "external.*"]
    effect: allow

  - callers: ["*"]
    targets: ["common.*"]
    effect: allow
```

### 9.2 Multi-tenant Architecture

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  # Tenant isolation
  - callers: ["tenant.a.*"]
    targets: ["tenant.a.*"]
    effect: allow

  - callers: ["tenant.b.*"]
    targets: ["tenant.b.*"]
    effect: allow

  # Shared services
  - callers: ["tenant.*"]
    targets: ["shared.*"]
    effect: allow

  # Admin console crosses tenants, only for admin identities
  - callers: ["admin.*"]
    targets: ["tenant.*", "shared.*"]
    effect: allow
    conditions:
      roles: ["admin"]
```

### 9.3 Security-Sensitive System

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["@external"]
    targets: ["public.*"]
    effect: allow

  - callers: ["auth.verified.*"]
    targets: ["protected.*"]
    effect: allow

  - callers: ["auth.admin.*"]
    targets: ["sensitive.*"]
    effect: allow
    approval: required
    description: "Sensitive operations need an admin caller and a sign-off"

  # Audit trail: anyone may write, only compliance may read.
  - callers: ["*"]
    targets: ["audit.write"]
    effect: allow

  - callers: ["compliance.*"]
    targets: ["audit.read"]
    effect: allow
```

### 9.4 AI Agent Tool Governance

Scoping **which tools an AI agent may invoke**, by the agent's identity roles and the depth of its call chain. The rules are the reference policy in [`examples/acl/agent-tool-governance.yaml`](https://github.com/aiperceivable/apcore/blob/main/examples/acl/agent-tool-governance.yaml), which the conformance fixture `conformance/fixtures/acl_agent_scoping.json` pins so every SDK makes the same decisions. Add the `audit:` block when you vendor it:

```yaml
# acl/global_acl.yaml — rules from examples/acl/agent-tool-governance.yaml
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  # 1. External / unauthenticated entry points (no caller_id) — read-only.
  - callers: ["@external"]
    targets: ["executor.*.read"]
    effect: allow
    description: "External (unauthenticated) callers may only read."

  # 2. Reader-role agents — read + query, depth-capped to fuse runaway chains.
  - callers: ["agent.*"]
    targets: ["executor.*.read", "executor.*.query"]
    effect: allow
    conditions:
      roles: ["reader"]
      max_call_depth: 3
    description: "Reader agents may read and query, depth-capped."

  # 3. Data-admin agents — exports and sensitive deletes (no depth cap).
  - callers: ["agent.*"]
    targets: ["data.export", "executor.*.delete"]
    effect: allow
    conditions:
      roles: ["data_admin"]
    description: "Data-admin agents may export data and perform sensitive deletes."
```

**Privilege gradient** (least → most): `@external` < `reader` agent < `data_admin` agent.

| Caller | Identity `roles` | May reach | Depth cap |
|---|---|---|---|
| `@external` (no caller) | — | `executor.*.read` | — |
| `agent.*` | `reader` | `executor.*.read`, `executor.*.query` | `max_call_depth: 3` |
| `agent.*` | `data_admin` | `data.export`, `executor.*.delete` | none |

- **`roles`** matches if the context `Identity` holds any listed role. A `reader` agent cannot reach `data.export`; a `data_admin` agent cannot reach `executor.*.query`. An agent holding both roles satisfies either rule.
- **`max_call_depth`** stops runaway tool chains: a `reader` agent may read or query only while its call chain is at most 3 deep (depth 3 allowed, depth 4 denied). The `data_admin` rule has no cap because exports and deletes are deliberate, audited operations.
- **`@external`** gets the smallest surface — the `read` verb only.

Mapping a request's authentication onto an `Identity` with the right `roles` happens in your web layer; see [Integrating Existing Projects](./integrating-existing-projects.md).

---

## 10. Troubleshooting

### 10.1 Common Issues

**Issue 1: A call you expected to work is denied**

No rule allows it, so `default_effect: deny` decides:

```yaml
# acl/global_acl.yaml — before: only orchestrator → executor is allowed
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow
```

Add a rule for exactly the call that should work — not a catch-all:

```yaml
# acl/global_acl.yaml — after
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  - callers: ["api.reports.*"]
    targets: ["common.format.*"]
    effect: allow
    description: "Report endpoints use the shared formatters"
```

The denied audit entry (`reason: default_effect`) names the exact `caller_id` / `target_id` pair to allow.

**Issue 2: A `deny` rule never takes effect**

A broader `allow` above it matches first:

```yaml
# acl/global_acl.yaml — wrong order
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["api.*"]
    targets: ["vault.*"]
    effect: allow        # matches first
  - callers: ["api.public.*"]
    targets: ["vault.*"]
    effect: deny         # never reached
```

Move the narrower rule up:

```yaml
# acl/global_acl.yaml — fixed
version: "1.0"
default_effect: deny

audit:
  enabled: true
  include_denied: true
  log_level: info

rules:
  - callers: ["api.public.*"]
    targets: ["vault.*"]
    effect: deny
  - callers: ["api.*"]
    targets: ["vault.*"]
    effect: allow
```

**Issue 3: The rules have no effect at all**

Nothing is being checked. Either no ACL is attached — the file is not at `<acl.root>/global_acl.yaml`, or you passed your own `Executor` to `APCore` — or the running strategy has no `acl_check` step. `governance_state()` tells you which (section 2.4).

**Issue 4: `default_effect` or audit settings in `apcore.yaml` are ignored**

`acl.default_effect` and `acl.audit.*` in `apcore.yaml` do nothing. Set both in the ACL file.

### 10.2 Testing ACL

Test the file you ship, with the decisions you expect. This checks the §9.1 file:

=== "Python"

    ```python
    import pytest

    from apcore.acl import ACL

    ACL_FILE = "./acl/global_acl.yaml"


    @pytest.mark.parametrize(
        ("caller_id", "target_id", "expected"),
        [
            (None, "gateway.orders.create", True),        # None = @external
            (None, "service.orders.create", False),
            ("gateway.orders.create", "service.orders.create", True),
            ("service.orders.create", "repository.orders.insert", True),
            ("gateway.orders.create", "repository.orders.insert", False),
            ("repository.orders.insert", "common.util.format", True),
        ],
    )
    def test_acl_decision(caller_id: str | None, target_id: str, expected: bool) -> None:
        assert ACL.load(ACL_FILE).check(caller_id, target_id) is expected
    ```

=== "TypeScript"

    ```typescript
    import { describe, expect, it } from 'vitest';
    import { ACL } from 'apcore-js';

    const acl = ACL.load('./acl/global_acl.yaml');

    describe('ACL decisions', () => {
      it.each([
        [null, 'gateway.orders.create', true], // null = @external
        [null, 'service.orders.create', false],
        ['gateway.orders.create', 'service.orders.create', true],
        ['service.orders.create', 'repository.orders.insert', true],
        ['gateway.orders.create', 'repository.orders.insert', false],
        ['repository.orders.insert', 'common.util.format', true],
      ] as const)('%s -> %s is %s', (callerId, targetId, expected) => {
        expect(acl.check(callerId, targetId)).toBe(expected);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::ACL;

    #[test]
    fn acl_decisions() {
        let acl = ACL::load("./acl/global_acl.yaml").expect("ACL file loads");
        let cases: &[(Option<&str>, &str, bool)] = &[
            (None, "gateway.orders.create", true), // None = @external
            (None, "service.orders.create", false),
            (Some("gateway.orders.create"), "service.orders.create", true),
            (Some("service.orders.create"), "repository.orders.insert", true),
            (Some("gateway.orders.create"), "repository.orders.insert", false),
            (Some("repository.orders.insert"), "common.util.format", true),
        ];
        for &(caller_id, target_id, expected) in cases {
            assert_eq!(acl.check(caller_id, target_id, None), expected, "{caller_id:?} -> {target_id}");
        }
    }
    ```

Rules with `conditions` need a `Context` carrying the identity and call chain the condition reads; pass it as the third argument to `check()`.

---

## Next Steps

- [ACL System](../features/acl-system.md) — evaluation semantics, audit records, API contracts
- [Core Executor](../features/core-executor.md) — where the ACL step sits in the pipeline
- [Cookbook — Approval-Gated Modules](./cookbook-approval-flow.md) — handling `approval: required`
- [Architecture](../architecture.md) — overall system architecture
