//! Compile-fail test: an installed ACL rule cannot be mutated from outside the
//! crate (PROTOCOL_SPEC §6.2.1, spec v1.31.0, apcore#112).
//!
//! §6.2.1's backstop is the one route no door covers — assigning `callers` or
//! `targets` on a rule that is already installed in an ACL — and
//! `acl_pattern_arity.json` carries it as nine `kind: "backstop"` cases whose
//! `mutation_route` is `installed_rule`. The fixture states that an SDK which
//! has no such route satisfies those cases **by construction** and must assert
//! the closure rather than skip them.
//!
//! This is half of that assertion: `ACL::rules` returns `&[ACLRule]`, so
//! writing through it does not compile. The other half — that no `rules_mut`
//! exists — is asserted in `tests/test_acl_pattern_arity_conformance.rs`,
//! which reads the source, because a name-resolution error here would abort
//! compilation before borrow checking and suppress the error this file exists
//! for. A doc comment cannot fail CI; this can — and if a future in-place
//! editing API opens the route, the driver's "satisfied by construction" claim
//! has to be re-earned rather than silently inherited.

#[test]
fn an_installed_acl_rule_must_not_be_mutable_downstream() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/acl_rules_immutable.rs");
}
