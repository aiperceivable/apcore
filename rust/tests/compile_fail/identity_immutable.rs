// This file MUST NOT compile -- verifies AC-015
// Identity fields are private and cannot be mutated after construction.
use apcore::context::Identity;
use std::collections::HashMap;

fn main() {
    let mut identity = Identity::new(
        "user-1".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        HashMap::new(),
    );
    identity.roles = vec![]; // ERROR: field `roles` is private
}
