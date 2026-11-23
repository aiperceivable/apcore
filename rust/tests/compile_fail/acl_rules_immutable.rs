// This file MUST NOT compile -- PROTOCOL_SPEC §6.2.1 (spec v1.31.0, apcore#112).
//
// The section's backstop covers "assigning `callers` or `targets` on an
// already-INSTALLED rule", and `acl_pattern_arity.json` carries it as nine
// `kind: "backstop"` cases. In apcore-python and apcore-typescript
// `acl.rules[0].targets = []` reaches the matcher directly. This SDK closes
// that route: `ACL::rules` hands back `&[ACLRule]`, there is no `rules_mut`,
// no public field and no `Deserialize` on `ACL`, and `ACL::new_unchecked` is
// private.
//
// That closure is what the conformance driver reports as "satisfied by
// construction" rather than skipped, so it is asserted here rather than
// asserted in prose. (The absence of a `rules_mut` is asserted in the driver,
// which reads the source -- putting a second, name-resolution error in this
// file would abort compilation before borrow checking and suppress the one
// below.)
use apcore::acl::{ACLRule, ACL};

fn main() {
    let acl = ACL::new(
        vec![ACLRule::new(
            vec!["*".to_string()],
            vec!["*".to_string()],
            "deny",
        )],
        "allow",
        None,
    );
    acl.rules()[0].targets = Vec::new(); // ERROR: cannot assign through a `&` reference
}
