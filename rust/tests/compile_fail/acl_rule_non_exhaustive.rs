// This file MUST NOT compile -- pins ACLRule's construction policy (apcore#38).
//
// `ACLRule` is `#[non_exhaustive]`, so a struct expression is rejected for every
// crate except apcore itself. The supported form from outside is `ACLRule::new`
// plus field assignment (api-surface-conventions.md §9.3). If this file ever
// compiles, the attribute has been dropped and the next spec-driven field is a
// hard compile break for every downstream crate again -- which is exactly what
// v1.28.0's `approval` was.
use apcore::ACLRule;

fn main() {
    let _rule = ACLRule {
        callers: vec!["admin".to_string()],
        targets: vec!["admin.*".to_string()],
        effect: "allow".to_string(),
        approval: None,
        description: None,
        conditions: None,
    }; // ERROR: cannot create non-exhaustive struct using struct expression
}
