---
description: "How to write apcore ACL rules in YAML: callers, targets, allow/deny effects, wildcard ID patterns, and default-deny configuration, shared identically across all SDKs."
---

# ACL Configuration Guide

> Configure access control rules between modules.

!!! note "Cross-language applicability"
    The ACL configuration format (YAML) is identical across all SDKs and is shown as bare YAML blocks. SDK code examples are shown side-by-side for Python, TypeScript, and Rust. See the [ACL System feature spec](../features/acl-system.md) for the full API reference.

## 1. Overview

ACL (Access Control List) is used to control invocation permissions between modules, preventing unauthorized module calls.

**Core Concepts:**

| Concept | Description |
|---------|-------------|
| **Callers** | List of caller module ID patterns |
| **Targets** | List of target module ID patterns |
| **Effect** | Allow or deny |
| **Wildcards** | `*` matches any characters, `executor.*` matches all modules starting with `executor.` |

---

## 2. Quick Start

### 2.1 Create Configuration File

```yaml
# acl/global_acl.yaml
rules:
  # Allow orchestrator.* to call executor.*
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  # Allow all calls by default
  - callers: ["*"]
    targets: ["*"]
    effect: allow
```

### 2.2 Load and Use

=== "Python"

    ```python
    from apcore import Registry, Executor
    from apcore.acl import ACL

    registry = Registry(extensions_dir="./extensions")
    registry.discover()

    # Load ACL from YAML
    acl = ACL.load("./acl/global_acl.yaml")

    # Create Executor with ACL wired in
    executor = Executor(registry=registry, acl=acl)

    # Permissions are automatically checked on invocation
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={"to": "alice@example.com", "subject": "Hello", "body": "Hi"},
        context=None,  # caller_id defaults to @external
    )
    ```

=== "TypeScript"

    ```typescript
    import { Registry, Executor } from 'apcore-js';
    import { ACL } from 'apcore-js';

    const registry = new Registry({ extensionsDir: './extensions' });
    await registry.discover();

    // Load ACL from YAML
    const acl = ACL.load('./acl/global_acl.yaml');

    // Create Executor with ACL wired in
    const executor = new Executor({ registry, acl });

    // Permissions are automatically checked on invocation
    const result = await executor.call(
      'executor.email.send_email',
      { to: 'alice@example.com', subject: 'Hello', body: 'Hi' },
      null, // caller_id defaults to @external
    );
    ```

=== "Rust"

    ```rust
    use apcore::acl::ACL;
    use apcore::config::Config;
    use apcore::executor::Executor;
    use apcore::registry::Registry;
    use serde_json::json;
    use std::sync::Arc;

    # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let registry = Arc::new(Registry::new());
    // discover() takes a Discoverer; see registry docs for FsDiscoverer setup
    let config = Arc::new(Config::default());

    // Load ACL from YAML
    let acl = ACL::load("./acl/global_acl.yaml")?;

    // Create Executor with ACL wired in
    let mut executor = Executor::new(registry, config);
    executor.set_acl(acl);

    // Permissions are automatically checked on invocation
    let inputs = json!({
        "to": "alice@example.com",
        "subject": "Hello",
        "body": "Hi"
    });
    let result = executor
        .call("executor.email.send_email", inputs, None, None)
        .await?;
    # Ok(())
    # }
    ```

---

## 3. Configuration Format

### 3.1 Basic Structure

```yaml
# acl/global_acl.yaml
version: "1.0"  # Optional

# Default behavior (optional, defaults to deny)
default_effect: deny

# Rules list
rules:
  - callers: ["<pattern>"]
    targets: ["<pattern>"]
    effect: allow | deny
    description: "Optional rule description"
```

### 3.2 Rule Fields

| Field | Required | Description |
|------|------|------|
| `callers` | Yes | List of caller module ID patterns |
| `targets` | Yes | List of target module ID patterns |
| `effect` | Yes | `allow` or `deny` |
| `description` | No | Rule description |

---

## 4. Pattern Matching

### 4.1 Exact Match

```yaml
rules:
  # Only allow specific module to call specific module
  - callers: ["orchestrator.user.register"]
    targets: ["executor.email.send_email"]
    effect: allow
```

### 4.2 Prefix Wildcards

```yaml
rules:
  # Allow all modules under orchestrator to call all modules under executor
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  # Allow calling all modules under common
  - callers: ["*"]
    targets: ["common.*"]
    effect: allow
```

### 4.3 Multi-level Wildcards

```yaml
rules:
  # Match any depth
  - callers: ["api.*"]
    targets: ["executor.email.*"]
    effect: allow

  # api.handler.user_api → executor.email.send_email ✓
  # api.v2.handler.user_api → executor.email.send_template ✓
```

### 4.4 Special Callers

```yaml
rules:
  # Top-level calls (no caller_id)
  - callers: ["@external"]
    targets: ["api.*"]
    effect: allow

  # System internal calls
  - callers: ["@system"]
    targets: ["internal.*"]
    effect: allow
```

---

## 5. Common Configuration Patterns

### 5.1 Layered Architecture

```yaml
# Typical three-tier architecture ACL
rules:
  # API layer can call Orchestrator layer
  - callers: ["api.*"]
    targets: ["orchestrator.*"]
    effect: allow

  # Orchestrator layer can call Executor layer
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  # All layers can call Common
  - callers: ["*"]
    targets: ["common.*"]
    effect: allow

  # Prohibit cross-layer calls
  - callers: ["api.*"]
    targets: ["executor.*"]
    effect: deny
    description: "API cannot call Executor directly"

  # Default deny
  - callers: ["*"]
    targets: ["*"]
    effect: deny
```

### 5.2 Whitelist Mode

```yaml
default_effect: deny

rules:
  # Only allow specific call relationships
  - callers: ["orchestrator.user.register"]
    targets: ["executor.email.send_email"]
    effect: allow

  - callers: ["orchestrator.user.register"]
    targets: ["executor.database.insert"]
    effect: allow

  - callers: ["orchestrator.order.create"]
    targets: ["executor.payment.charge"]
    effect: allow
```

### 5.3 Blacklist Mode

```yaml
default_effect: deny

rules:
  # Deny calling sensitive modules
  - callers: ["*"]
    targets: ["internal.admin.*"]
    effect: deny

  - callers: ["*"]
    targets: ["internal.security.*"]
    effect: deny

  # Only admin modules can call
  - callers: ["admin.*"]
    targets: ["internal.admin.*"]
    effect: allow
```

### 5.4 Environment Isolation

```yaml
# Development environment allows more permissive calls
rules:
  # Development tools
  - callers: ["dev.*"]
    targets: ["*"]
    effect: allow
    description: "Development environment: allow dev modules to call any module"

  # Mock modules
  - callers: ["*"]
    targets: ["mock.*"]
    effect: allow
    description: "Allow calling mock modules"
```

---

## 6. Rule Evaluation

### 6.1 Matching Order

Rules are evaluated using **first-match-wins in definition order** per PROTOCOL_SPEC. Rules are checked in the order they appear in the YAML file, and the **first matching rule** determines the outcome. If no rule matches, the `default_effect` applies:

```yaml
rules:
  # Rule 1: Exact match (checked first)
  - callers: ["orchestrator.user.register"]
    targets: ["executor.email.send_email"]
    effect: allow

  # Rule 2: Wildcard match (checked second)
  - callers: ["orchestrator.*"]
    targets: ["executor.email.*"]
    effect: deny

  # Rule 3: Default rule (checked last)
  - callers: ["*"]
    targets: ["*"]
    effect: allow
```

```
Call: orchestrator.user.register → executor.email.send_email
Match: Rule 1 (allow — first rule to match)

Call: orchestrator.order.create → executor.email.send_email
Match: Rule 2 (deny — first rule to match)

Call: api.handler.test → common.util.format
Match: Rule 3 (allow — first rule to match)
```

### 6.2 Best Practices

```yaml
rules:
  # 1. Define exceptions first (exact match)
  - callers: ["orchestrator.admin.audit"]
    targets: ["internal.security.log"]
    effect: allow

  # 2. Then define general rules
  - callers: ["*"]
    targets: ["internal.*"]
    effect: deny

  # 3. Finally define default rule
  - callers: ["*"]
    targets: ["*"]
    effect: allow
```

### 6.3 Matching Algorithm Visualization

```
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

| Scenario | Result | Description |
|------|------|------|
| No ACL file | Use `default_effect` | Defaults to deny |
| Empty ACL file (no rules) | Use `default_effect` | All calls handled by default policy |
| caller_id is null | Treated as `@external` | External/top-level call |
| Module calls itself | Normal ACL check | No special handling |
| Wildcard `*.*.*` | Equivalent to `*` | Matches all |

### 6.5 Performance Impact

| Number of Rules | Expected Latency | Recommendation |
|--------|---------|------|
| < 50 | < 0.1ms | No optimization needed |
| 50-500 | 0.1-1ms | Consider rule ordering optimization |
| > 500 | > 1ms | Consider using prefix tree indexing |

**Performance Optimization Recommendations:**
- Put frequently matched rules at the beginning of the list
- Use exact matches instead of wildcards (exact matching is faster)
- Cache ACL check results in production (for same caller+target combinations)

---

## 7. Runtime Behavior

### 7.1 Check Timing

ACL is checked at the following times:

```
executor.call(module_id, inputs, context)
    │
    ├─ 1. Get caller_id
    │      └─ From context.caller_id
    │      └─ If None, treated as @external
    │
    ├─ 2. ACL check
    │      └─ acl.check(caller_id, target_id)
    │      └─ If denied, throw ACLDeniedError
    │
    └─ 3. Continue execution...
```

### 7.2 Error Handling

=== "Python"

    ```python
    from apcore.errors import ACLDeniedError

    try:
        result = executor.call(
            module_id="internal.secret",
            inputs={"key": "value"},
            context=context,
        )
    except ACLDeniedError as e:
        print("Access denied!")
        print(f"Caller: {e.caller_id}")
        print(f"Target: {e.target_id}")
        print(f"Code:   {e.code}")
    ```

=== "TypeScript"

    ```typescript
    import { ACLDeniedError } from 'apcore-js';

    try {
      const result = await executor.call(
        'internal.secret',
        { key: 'value' },
        context,
      );
    } catch (e) {
      if (e instanceof ACLDeniedError) {
        console.log('Access denied!');
        console.log(`Caller: ${e.callerId}`);
        console.log(`Target: ${e.targetId}`);
        console.log(`Code:   ${e.code}`);
      } else {
        throw e;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::errors::{ErrorCode, ModuleError};
    use serde_json::json;

    # async fn run(executor: &apcore::executor::Executor) -> Result<(), ModuleError> {
    match executor
        .call("internal.secret", json!({"key": "value"}), None, None)
        .await
    {
        Ok(result) => { /* use result */ }
        Err(e) if e.code == ErrorCode::ACLDenied => {
            println!("Access denied!");
            // caller_id / target_id are surfaced via ModuleError.details
            if let Some(caller) = e.details.get("caller_id") {
                println!("Caller: {caller}");
            }
            if let Some(target) = e.details.get("target_id") {
                println!("Target: {target}");
            }
        }
        Err(e) => return Err(e),
    }
    # Ok(())
    # }
    ```

### 7.3 Debug Mode

For deeper visibility, install an `audit_logger` that receives a structured `AuditEntry` for every ACL decision (matched rule index, decision, identity, trace ID, handler errors). Audit logging MUST stay enabled in production: it is the canonical record of every allow/deny outcome.

=== "Python"

    ```python
    import logging
    from apcore.acl import ACL, AuditEntry

    def log_audit(entry: AuditEntry) -> None:
        logging.info(
            "ACL %s: %s -> %s rule=%s reason=%s",
            entry.decision,
            entry.caller_id,
            entry.target_id,
            entry.matched_rule,
            entry.reason,
        )

    loaded = ACL.load("./acl/global_acl.yaml")
    acl = ACL(loaded.rules, loaded.default_effect, audit_logger=log_audit)
    acl.debug = True  # enables verbose check() debug logging
    ```

=== "TypeScript"

    ```typescript
    import { ACL, AuditEntry } from 'apcore-js';

    const auditLogger = (entry: AuditEntry): void => {
      console.info(
        `ACL ${entry.decision}: ${entry.callerId} -> ${entry.targetId} ` +
          `rule=${entry.matchedRule} reason=${entry.reason}`,
      );
    };

    const rules = ACL.load('./acl/global_acl.yaml').rules ?? [];
    const acl = new ACL(rules, 'deny', auditLogger);
    acl.debug = true; // enables verbose check() debug logging
    ```

=== "Rust"

    ```rust
    use apcore::acl::ACL;

    # fn run() -> Result<(), apcore::errors::ModuleError> {
    let mut acl = ACL::load("./acl/global_acl.yaml")?;
    acl.set_audit_logger(|entry| {
        tracing::info!(
            decision = %entry.decision,
            caller   = %entry.caller_id,
            target   = %entry.target_id,
            rule     = ?entry.matched_rule,
            reason   = %entry.reason,
            "ACL check"
        );
    });
    # Ok(())
    # }
    ```

!!! warning "Always keep an audit logger in production"
    Audit entries are the only durable trail of access decisions — security review and incident response depend on them. Do not strip the audit block from your wiring just because checks are passing.

---

## 8. Dynamic ACL

### 8.1 Runtime Modification

=== "Python"

    ```python
    from apcore.acl import ACL, ACLRule

    acl = ACL.load("./acl/global_acl.yaml")

    # Add rule (inserted at position 0 — highest priority)
    acl.add_rule(ACLRule(
        callers=["temp.module"],
        targets=["executor.*"],
        effect="allow",
        description="Temporary debug rule",
    ))

    # Remove the matching rule
    removed = acl.remove_rule(
        callers=["temp.module"],
        targets=["executor.*"],
    )

    # Reload from the original YAML (re-reads file from disk)
    acl.reload()
    ```

=== "TypeScript"

    ```typescript
    import { ACL } from 'apcore-js';

    const acl = ACL.load('./acl/global_acl.yaml');

    // Add rule (inserted at position 0 — highest priority)
    acl.addRule({
      callers: ['temp.module'],
      targets: ['executor.*'],
      effect: 'allow',
      description: 'Temporary debug rule',
    });

    // Remove the matching rule
    const removed: boolean = acl.removeRule(
      ['temp.module'],
      ['executor.*'],
    );

    // Reload from the original YAML (re-reads file from disk)
    acl.reload();
    ```

=== "Rust"

    ```rust
    use apcore::acl::{ACL, ACLRule};

    # fn run() -> Result<(), apcore::errors::ModuleError> {
    let mut acl = ACL::load("./acl/global_acl.yaml")?;

    // Add rule (inserted at position 0 — highest priority)
    acl.add_rule(ACLRule {
        callers: vec!["temp.module".to_string()],
        targets: vec!["executor.*".to_string()],
        effect: "allow".to_string(),
        description: Some("Temporary debug rule".to_string()),
        conditions: None,
    });

    // Remove the matching rule
    let removed: bool = acl.remove_rule(
        &["temp.module".to_string()],
        &["executor.*".to_string()],
    );

    // Reload from the original YAML (re-reads file from disk)
    acl.reload()?;
    # Ok(())
    # }
    ```

### 8.2 Dynamic Check Based on Context

For dynamic decisions (time windows, identity attributes, external lookups) prefer **registering a custom condition handler** over subclassing `ACL`. Custom handlers are first-class in every SDK and can be referenced from YAML rules. The example below adds a `time_window` condition that gates `maintenance.*` modules to a nightly window.

=== "Python"

    ```python
    from datetime import datetime
    from typing import Any

    from apcore.acl import ACL
    from apcore.context import Context

    class TimeWindowHandler:
        """Allow only when current hour is within [start, end]."""

        def evaluate(self, value: Any, context: Context) -> bool:
            if not isinstance(value, dict):
                return False
            start = int(value.get("start_hour", 0))
            end = int(value.get("end_hour", 23))
            hour = datetime.now().hour
            return start <= hour <= end

    ACL.register_condition("time_window", TimeWindowHandler())

    # YAML can now reference the new condition:
    #   - callers: ["*"]
    #     targets: ["maintenance.*"]
    #     effect: allow
    #     conditions:
    #       time_window: { start_hour: 2, end_hour: 6 }
    ```

=== "TypeScript"

    ```typescript
    import { ACL, Context } from 'apcore-js';

    // Structural shape matches the package's internal ACLConditionHandler.
    class TimeWindowHandler {
      evaluate(value: unknown, _context: Context): boolean {
        if (typeof value !== 'object' || value === null) return false;
        const v = value as { start_hour?: number; end_hour?: number };
        const start = v.start_hour ?? 0;
        const end = v.end_hour ?? 23;
        const hour = new Date().getHours();
        return hour >= start && hour <= end;
      }
    }

    ACL.registerCondition('time_window', new TimeWindowHandler());

    // YAML can now reference the new condition:
    //   - callers: ["*"]
    //     targets: ["maintenance.*"]
    //     effect: allow
    //     conditions:
    //       time_window: { start_hour: 2, end_hour: 6 }
    ```

=== "Rust"

    ```rust
    use apcore::acl::ACL;
    use apcore::acl_handlers::ACLConditionHandler;
    use apcore::context::Context;
    use async_trait::async_trait;
    use chrono::{Local, Timelike};
    use serde_json::Value;
    use std::sync::Arc;

    pub struct TimeWindowHandler;

    #[async_trait]
    impl ACLConditionHandler for TimeWindowHandler {
        async fn evaluate(&self, value: &Value, _ctx: &Context<Value>) -> bool {
            let Some(obj) = value.as_object() else { return false };
            let start = obj.get("start_hour").and_then(|v| v.as_u64()).unwrap_or(0);
            let end = obj.get("end_hour").and_then(|v| v.as_u64()).unwrap_or(23);
            let hour = Local::now().hour() as u64;
            hour >= start && hour <= end
        }
    }

    ACL::register_condition("time_window", Arc::new(TimeWindowHandler));

    // YAML can now reference the new condition:
    //   - callers: ["*"]
    //     targets: ["maintenance.*"]
    //     effect: allow
    //     conditions:
    //       time_window: { start_hour: 2, end_hour: 6 }
    ```

---

## 9. Configuration Examples

### 9.1 Microservice Architecture

```yaml
# acl/global_acl.yaml
version: "1.0"
default_effect: deny

rules:
  # Gateway entry
  - callers: ["@external"]
    targets: ["gateway.*"]
    effect: allow
    description: "External can only access Gateway"

  # Gateway → Service
  - callers: ["gateway.*"]
    targets: ["service.*"]
    effect: allow

  # Service → Repository
  - callers: ["service.*"]
    targets: ["repository.*"]
    effect: allow

  # Service → External API
  - callers: ["service.*"]
    targets: ["external.*"]
    effect: allow

  # Common utilities
  - callers: ["*"]
    targets: ["common.*"]
    effect: allow
```

### 9.2 Multi-tenant Architecture

```yaml
version: "1.0"
default_effect: deny

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

  # Admin console (can cross tenants)
  - callers: ["admin.*"]
    targets: ["*"]
    effect: allow
```

### 9.3 Security-Sensitive System

```yaml
version: "1.0"
default_effect: deny

rules:
  # Public API
  - callers: ["@external"]
    targets: ["public.*"]
    effect: allow

  # Require authentication to access
  - callers: ["auth.verified.*"]
    targets: ["protected.*"]
    effect: allow

  # Sensitive operations require additional permissions
  - callers: ["auth.admin.*"]
    targets: ["sensitive.*"]
    effect: allow

  # Audit log (write-only, no read)
  - callers: ["*"]
    targets: ["audit.write"]
    effect: allow

  - callers: ["*"]
    targets: ["audit.read"]
    effect: deny

  - callers: ["compliance.*"]
    targets: ["audit.read"]
    effect: allow
```

### 9.4 AI Agent Tool Governance

This is the canonical scenario for scoping **which tools an AI agent may invoke**, by the agent's identity roles and the depth of its call chain. Unlike the layered/microservice patterns above (which key off plain caller strings), agent governance leans on the `Identity` (`type` / `roles`) and `max_call_depth` conditions — exactly the features that make ACL valuable for per-agent control.

The reference artifact ships in the repo at [`examples/acl/agent-tool-governance.yaml`](https://github.com/aiperceivable/apcore/blob/main/examples/acl/agent-tool-governance.yaml) — vendor it directly. It is locked as a cross-language contract by the conformance fixture `conformance/fixtures/acl_agent_scoping.json`, so every SDK and framework integration produces identical decisions.

```yaml
# examples/acl/agent-tool-governance.yaml
version: "1.0"
default_effect: deny

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

How the conditions drive scoping:

- **`roles`** matches by set intersection: the rule fires only when the context `Identity` carries the required role. A `reader` agent cannot reach `data.export`; a `data_admin` agent cannot reach `executor.*.query` (its rule's targets don't include `query`). Roles are additive — an agent holding both `reader` and `data_admin` satisfies either rule.
- **`max_call_depth`** fuses runaway tool chains: a `reader` agent may issue reads/queries only while its call chain length does not exceed `3` (the comparison is inclusive — depth `3` is allowed, depth `4` is denied). The `data_admin` rule intentionally omits a cap because exports/deletes are deliberate, audited operations rather than exploratory chains.
- **`@external`** matches calls with no `caller_id` (top-level / unauthenticated entry points) and is restricted to the smallest surface — the `read` verb only.

!!! note "Framework glue stays in the integration"
    Mapping an incoming request's auth into an `Identity` with the right `roles` (e.g. a Django request, a FastAPI dependency) lives in the respective integration repo (`django-apcore`, `fastapi-apcore`, …). apcore ships only the language-agnostic policy (this YAML + the conformance fixture).

---

## 10. Troubleshooting

### 10.1 Common Issues

**Issue 1: All calls are denied**

```yaml
# Error: No default allow rule
rules:
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow
  # Missing default rule, all other calls will be denied

# Fix: Add default rule
rules:
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow
  - callers: ["*"]
    targets: ["*"]
    effect: allow  # Or set default_effect: allow
```

**Issue 2: Rule not taking effect**

```yaml
# Error: Rule order problem
rules:
  - callers: ["*"]
    targets: ["*"]
    effect: allow  # This will match first
  - callers: ["api.*"]
    targets: ["internal.*"]
    effect: deny   # Will never be matched

# Fix: Adjust order
rules:
  - callers: ["api.*"]
    targets: ["internal.*"]
    effect: deny   # Match specific rule first
  - callers: ["*"]
    targets: ["*"]
    effect: allow
```

### 10.2 Testing ACL

=== "Python"

    ```python
    from apcore.acl import ACL

    acl = ACL.load("./acl/global_acl.yaml")

    test_cases = [
        ("orchestrator.user.register", "executor.email.send_email", True),
        ("api.handler.test", "internal.secret", False),
        ("admin.panel", "internal.secret", True),
    ]

    for caller, target, expected in test_cases:
        result = acl.check(caller, target)
        status = "OK" if result == expected else "FAIL"
        print(f"[{status}] {caller} -> {target}: {result}")
    ```

=== "TypeScript"

    ```typescript
    import { ACL } from 'apcore-js';

    const acl = ACL.load('./acl/global_acl.yaml');

    const testCases: Array<[string, string, boolean]> = [
      ['orchestrator.user.register', 'executor.email.send_email', true],
      ['api.handler.test', 'internal.secret', false],
      ['admin.panel', 'internal.secret', true],
    ];

    for (const [caller, target, expected] of testCases) {
      const result = acl.check(caller, target);
      const status = result === expected ? 'OK' : 'FAIL';
      console.log(`[${status}] ${caller} -> ${target}: ${result}`);
    }
    ```

=== "Rust"

    ```rust
    use apcore::acl::ACL;

    # fn run() -> Result<(), apcore::errors::ModuleError> {
    let acl = ACL::load("./acl/global_acl.yaml")?;

    let test_cases: &[(&str, &str, bool)] = &[
        ("orchestrator.user.register", "executor.email.send_email", true),
        ("api.handler.test", "internal.secret", false),
        ("admin.panel", "internal.secret", true),
    ];

    for (caller, target, expected) in test_cases {
        let result = acl.check(Some(caller), target, None);
        let status = if result == *expected { "OK" } else { "FAIL" };
        println!("[{status}] {caller} -> {target}: {result}");
    }
    # Ok(())
    # }
    ```

---

## Next Steps

- [Core Executor](../features/core-executor.md) - Learn how ACL integrates with Executor
- [Middleware Guide](./middleware.md) - Extend ACL functionality with middleware
- [Architecture](../architecture.md) - System overall architecture
