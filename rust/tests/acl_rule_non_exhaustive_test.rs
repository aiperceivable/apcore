//! Compile-fail test: `ACLRule` is `#[non_exhaustive]` (apcore#38).
//!
//! The decision this pins is on the type itself: the spec may add a field to a
//! rule in any minor release, and doing so must not break a downstream crate.
//! The attribute is the mechanism, and its cost — no struct expression outside
//! apcore — is the thing a well-meaning "let downstream keep its literals"
//! change would quietly undo. A doc comment cannot fail CI; this can.

#[test]
fn acl_rule_must_not_be_struct_literal_constructible_downstream() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/acl_rule_non_exhaustive.rs");
}
