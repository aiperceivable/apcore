//! Compile-fail test: Identity fields must be private (AC-015).

#[test]
fn identity_fields_must_be_private() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/identity_immutable.rs");
}
