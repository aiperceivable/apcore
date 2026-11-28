#[test]
fn unsupported_annotation_has_a_compile_time_diagnostic() {
    // Use the proc-macro crate directly so the UI test does not depend on
    // trybuild recovering the SDK's optional features from Cargo fingerprints.
    trybuild::TestCases::new().compile_fail("tests/ui/typed_module_invalid_annotation.rs");
}
