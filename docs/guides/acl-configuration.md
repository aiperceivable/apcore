# ACL Configuration Guide

> Configure access control rules between modules.

!!! note "Cross-language applicability"
    Code examples in this guide use Python syntax. The ACL configuration format (YAML) is identical across all SDKs. For TypeScript and Rust API equivalents, see the [ACL System feature spec](../features/acl-system.md).

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

```python
from apcore import Registry, Executor, ACL

registry = Registry(extensions_dir="./extensions")
registry.discover()

# Load ACL
acl = ACL.load("./acl/global_acl.yaml")

# Create Executor
executor = Executor(registry=registry, acl=acl)

# Permissions are automatically checked on invocation
result = executor.call(
    module_id="executor.email.send_email",
    inputs={...},
    context=context
)
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

```python
from apcore import ACLDeniedError

try:
    result = executor.call(
        module_id="internal.secret",
        inputs={...},
        context=context
    )
except ACLDeniedError as e:
    print(f"Access denied!")
    print(f"Caller: {e.caller_id}")
    print(f"Target: {e.target_id}")
    print(f"Matched rule: {e.matched_rule}")
```

### 7.3 Debug Mode

```python
from apcore import ACL

acl = ACL.load("./acl/global_acl.yaml")

# Enable debug mode
acl.debug = True

# All checks will output logs
# [ACL] Check: orchestrator.user.register → executor.email.send_email
# [ACL] Matched rule #1: allow
```

---

## 8. Dynamic ACL

### 8.1 Runtime Modification

```python
from apcore import ACL, ACLRule

acl = ACL.load("./acl/global_acl.yaml")

# Add rule
acl.add_rule(ACLRule(
    callers=["temp.module"],
    targets=["executor.*"],
    effect="allow"
))

# Remove rule
acl.remove_rule(callers=["temp.module"], targets=["executor.*"])

# Reload
acl.reload()
```

### 8.2 Dynamic Check Based on Context

```python
class DynamicACL(ACL):
    """ACL with dynamic permission checking support"""

    def check(self, caller_id: str, target_id: str, context: Context) -> bool:
        # Check static rules first
        if not super().check(caller_id, target_id, context):
            return False

        # Dynamic check: based on identity roles
        if context.identity:
            if "admin" in context.identity.roles:
                return True  # Admins can call any module

        # Dynamic check: based on time
        from datetime import datetime
        hour = datetime.now().hour
        if target_id.startswith("maintenance.") and not (2 <= hour <= 6):
            return False  # Maintenance modules only available at night

        return True
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

```python
from apcore import ACL

acl = ACL.load("./acl/global_acl.yaml")

# Test specific calls
test_cases = [
    ("orchestrator.user.register", "executor.email.send_email", True),
    ("api.handler.test", "internal.secret", False),
    ("admin.panel", "internal.secret", True),
]

for caller, target, expected in test_cases:
    result = acl.check(caller, target)
    status = "✓" if result == expected else "✗"
    print(f"{status} {caller} → {target}: {result}")
```

---

## Next Steps

- [Core Executor](../features/core-executor.md) - Learn how ACL integrates with Executor
- [Middleware Guide](./middleware.md) - Extend ACL functionality with middleware
- [Architecture](../architecture.md) - System overall architecture
