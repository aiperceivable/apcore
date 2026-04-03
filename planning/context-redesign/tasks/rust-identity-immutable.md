# Task: Rust Identity Immutability

## Goal

Make Rust `Identity` fields private, add `Identity::new()` constructor and pub getter methods, and add serde compatibility via the raw struct deserialization pattern. This enforces immutability after construction, matching the spec requirement.

## Files Involved

### Rust SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/context.rs` (Identity struct definition)
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/tests/compile_fail/identity_immutable.rs` (compile-fail test)
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/tests/context_field_removal_test.rs` (add identity compile-fail, or create separate test runner)
- **Modify:** Any files that directly access `identity.id`, `identity.roles`, etc. as public fields (must switch to getter methods)

### Files to check for usages
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/acl.rs`
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/executor.rs`
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/middleware/*.rs`
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/observability/*.rs`
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/tests/*.rs`

## Steps

### Step 1: Grep for all direct field access on Identity

```bash
cd apcore-rust && grep -rn "identity\.\(id\|identity_type\|roles\|attrs\)" src/ tests/
```

### Step 2: Write compile-fail test (AC-015)

Create `apcore-rust/tests/compile_fail/identity_immutable.rs`:

```rust
// This file MUST NOT compile -- verifies AC-015
use apcore::context::Identity;
use std::collections::HashMap;

fn main() {
    let mut identity = Identity::new(
        "user-1".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        HashMap::new(),
    );
    identity.roles = vec![];  // ERROR: field `roles` is private
}
```

Add to test runner (or extend existing `context_field_removal_test.rs`):

```rust
#[test]
fn identity_fields_must_be_private() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/identity_immutable.rs");
}
```

### Step 3: Write unit tests for getters

Add to `apcore-rust/src/context.rs` inline tests or `tests/`:

```rust
#[test]
fn test_identity_getters() {
    let identity = Identity::new(
        "user-1".to_string(),
        "user".to_string(),
        vec!["admin".to_string(), "reader".to_string()],
        HashMap::from([("org".to_string(), json!("acme"))]),
    );
    assert_eq!(identity.id(), "user-1");
    assert_eq!(identity.identity_type(), "user");
    assert_eq!(identity.roles(), &["admin", "reader"]);
    assert_eq!(identity.attrs().get("org"), Some(&json!("acme")));
}

#[test]
fn test_identity_serde_roundtrip() {
    let identity = Identity::new(
        "u1".to_string(),
        "service".to_string(),
        vec!["role1".to_string()],
        HashMap::new(),
    );
    let json = serde_json::to_string(&identity).unwrap();
    let deserialized: Identity = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "u1");
    assert_eq!(deserialized.identity_type(), "service");
}
```

### Step 4: Implement private fields, constructor, and getters

Modify `Identity` in `apcore-rust/src/context.rs`:

```rust
/// Identity represents the authenticated caller.
/// Fields are private to enforce immutability after construction.
#[derive(Debug, Clone, Serialize)]
pub struct Identity {
    id: String,
    #[serde(rename = "type")]
    identity_type: String,
    roles: Vec<String>,
    attrs: HashMap<String, serde_json::Value>,
}

impl Identity {
    pub fn new(
        id: String,
        identity_type: String,
        roles: Vec<String>,
        attrs: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self { id, identity_type, roles, attrs }
    }

    pub fn id(&self) -> &str { &self.id }
    pub fn identity_type(&self) -> &str { &self.identity_type }
    pub fn roles(&self) -> &[String] { &self.roles }
    pub fn attrs(&self) -> &HashMap<String, serde_json::Value> { &self.attrs }
}
```

### Step 5: Add serde deserialization compatibility

Since `#[derive(Deserialize)]` does not work with private fields in the standard way, use the `serde(from)` pattern:

```rust
/// Raw intermediate struct for deserializing Identity.
#[derive(Deserialize)]
struct IdentityRaw {
    id: String,
    #[serde(rename = "type")]
    identity_type: String,
    roles: Vec<String>,
    #[serde(default)]
    attrs: HashMap<String, serde_json::Value>,
}

impl From<IdentityRaw> for Identity {
    fn from(raw: IdentityRaw) -> Self {
        Identity::new(raw.id, raw.identity_type, raw.roles, raw.attrs)
    }
}

// Add this attribute to Identity:
#[derive(Debug, Clone, Serialize)]
#[serde(from = "IdentityRaw")]
pub struct Identity { /* ... */ }
```

### Step 6: Update all call sites

Change every `identity.id` to `identity.id()`, `identity.roles` to `identity.roles()`, etc. across:
- `acl.rs` (ACL checks likely read roles/attrs)
- `executor.rs`
- `middleware/*.rs`
- `observability/*.rs`
- `tests/*.rs`

### Step 7: Run full test suite

```bash
cd apcore-rust && cargo test
```

## Acceptance Criteria

- [x] **AC-015**: Rust `Identity` fields are immutable (compile-fail test confirms `identity.roles = vec![]` fails)
- [ ] `Identity::new()` constructor works
- [ ] Getter methods return correct references: `id() -> &str`, `identity_type() -> &str`, `roles() -> &[String]`, `attrs() -> &HashMap`
- [ ] Serde roundtrip (serialize then deserialize) preserves all fields
- [ ] `#[serde(rename = "type")]` on `identity_type` produces `"type"` in JSON
- [ ] All existing tests pass after migration

## Dependencies

- **Depends on:** none
- **Required by:** serialization

## Estimated Time

2 hours
