// Integration tests for built-in pipeline steps

use apcore::{
    build_internal_strategy, build_performance_strategy, build_standard_strategy,
    build_testing_strategy,
};

/// The standard strategy has exactly 11 steps.
#[test]
fn test_standard_strategy_has_11_steps() {
    let strategy = build_standard_strategy();
    assert_eq!(strategy.steps().len(), 11);
    assert_eq!(strategy.name(), "standard");
}

/// Step names follow the expected order.
#[test]
fn test_standard_strategy_step_names() {
    let strategy = build_standard_strategy();
    let expected = vec![
        "context_creation",
        "call_chain_guard",
        "module_lookup",
        "acl_check",
        "approval_gate",
        "middleware_before",
        "input_validation",
        "execute",
        "output_validation",
        "middleware_after",
        "return_result",
    ];
    assert_eq!(strategy.step_names(), expected);
}

/// Removable and replaceable flags match the spec.
#[test]
fn test_standard_strategy_step_flags() {
    let strategy = build_standard_strategy();

    // (name, removable, replaceable)
    let expected_flags: Vec<(&str, bool, bool)> = vec![
        ("context_creation", false, false),
        ("call_chain_guard", true, true),
        ("module_lookup", false, false),
        ("acl_check", true, true),
        ("approval_gate", true, true),
        ("middleware_before", true, false),
        ("input_validation", true, true),
        ("execute", false, true),
        ("output_validation", true, true),
        ("middleware_after", true, false),
        ("return_result", false, false),
    ];

    for (step, (name, removable, replaceable)) in strategy.steps().iter().zip(expected_flags.iter())
    {
        assert_eq!(step.name(), *name, "name mismatch");
        assert_eq!(
            step.removable(),
            *removable,
            "removable mismatch for step '{name}'"
        );
        assert_eq!(
            step.replaceable(),
            *replaceable,
            "replaceable mismatch for step '{name}'"
        );
    }
}

/// Internal strategy removes acl_check and approval_gate (11 - 2 = 9).
#[test]
fn test_internal_strategy_has_9_steps() {
    let s = build_internal_strategy();
    assert_eq!(s.steps().len(), 9);
}

/// Testing strategy removes acl_check, approval_gate, and safety_check (11 - 3 = 8).
#[test]
fn test_testing_strategy_has_8_steps() {
    let s = build_testing_strategy();
    assert_eq!(s.steps().len(), 8);
}

/// Performance strategy removes middleware_before and middleware_after (11 - 2 = 9).
#[test]
fn test_performance_strategy_has_9_steps() {
    let s = build_performance_strategy();
    assert_eq!(s.steps().len(), 9);
}
