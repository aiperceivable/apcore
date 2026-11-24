//! Tests for the ExtensionManager — registration, retrieval, unregistration,
//! listing, type-checking, and wiring.

use apcore::acl::ACL;
use apcore::errors::ErrorCode;
use apcore::extensions::{ExtensionKind, ExtensionManager};
use apcore::{Executor, Registry};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_acl() -> ExtensionKind {
    ExtensionKind::Acl(Arc::new(ACL::new(vec![], "deny", None)))
}

// ---------------------------------------------------------------------------
// new() / list_points()
// ---------------------------------------------------------------------------

#[test]
fn test_new_has_six_built_in_points() {
    let mgr = ExtensionManager::new();
    assert_eq!(mgr.list_points().len(), 6);
}

#[test]
fn test_list_points_contains_all_expected_names() {
    let mgr = ExtensionManager::new();
    let names: Vec<String> = mgr.list_points().iter().map(|p| p.name.clone()).collect();
    for expected in &[
        "discoverer",
        "middleware",
        "acl",
        "span_exporter",
        "module_validator",
        "approval_handler",
    ] {
        assert!(
            names.iter().any(|n| n == *expected),
            "missing point: {expected}"
        );
    }
}

#[test]
fn test_default_equals_new() {
    let mgr = ExtensionManager::default();
    assert_eq!(mgr.list_points().len(), 6);
}

// ---------------------------------------------------------------------------
// Register a single extension
// ---------------------------------------------------------------------------

#[test]
fn test_register_acl_succeeds() {
    let mut mgr = ExtensionManager::new();
    let result = mgr.register("acl", make_acl());
    assert!(result.is_ok());
    assert_eq!(mgr.count("acl"), Some(1));
}

#[test]
fn test_register_replaces_previous_for_non_multiple_point() {
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).unwrap();
    mgr.register("acl", make_acl()).unwrap();
    // non-multiple: second register replaces the first
    assert_eq!(mgr.count("acl"), Some(1));
}

#[test]
fn test_has_returns_false_before_registration() {
    let mgr = ExtensionManager::new();
    assert!(!mgr.has("acl").unwrap());
}

#[test]
fn test_has_returns_true_after_registration() {
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).unwrap();
    assert!(mgr.has("acl").unwrap());
}

// ---------------------------------------------------------------------------
// Register multiple middleware extensions
// ---------------------------------------------------------------------------

#[test]
fn test_register_multiple_middleware_accumulates() {
    use apcore::LoggingMiddleware;

    let mut mgr = ExtensionManager::new();

    mgr.register(
        "middleware",
        ExtensionKind::Middleware(Arc::new(LoggingMiddleware::default())),
    )
    .unwrap();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(Arc::new(LoggingMiddleware::default())),
    )
    .unwrap();

    // middleware is a "multiple" point — both registrations should be retained
    assert_eq!(mgr.count("middleware"), Some(2));
}

// ---------------------------------------------------------------------------
// Unregister (clear) an extension
// ---------------------------------------------------------------------------

#[test]
fn test_clear_removes_all_at_point() {
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).unwrap();
    mgr.clear("acl").unwrap();
    assert!(!mgr.has("acl").unwrap());
    assert_eq!(mgr.count("acl"), Some(0));
}

#[test]
fn test_clear_unknown_point_returns_error() {
    let mut mgr = ExtensionManager::new();
    let result = mgr.clear("nonexistent_point");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::GeneralInvalidInput);
}

#[test]
fn test_clear_all_removes_everything() {
    use apcore::LoggingMiddleware;

    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).unwrap();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(Arc::new(LoggingMiddleware::default())),
    )
    .unwrap();
    mgr.clear_all();
    assert!(!mgr.has("acl").unwrap());
    assert!(!mgr.has("middleware").unwrap());
}

// ---------------------------------------------------------------------------
// List all extension points
// ---------------------------------------------------------------------------

#[test]
fn test_list_points_returns_correct_multiple_flags() {
    let mgr = ExtensionManager::new();
    let points = mgr.list_points();
    let mw_point = points.iter().find(|p| p.name == "middleware").unwrap();
    let acl_point = points.iter().find(|p| p.name == "acl").unwrap();
    assert!(mw_point.multiple, "middleware should allow multiple");
    assert!(!acl_point.multiple, "acl should not allow multiple");
}

#[test]
fn test_list_points_each_has_description() {
    let mgr = ExtensionManager::new();
    for point in mgr.list_points() {
        assert!(
            !point.description.is_empty(),
            "point '{}' has empty description",
            point.name
        );
    }
}

// ---------------------------------------------------------------------------
// Type checking — wrong kind for a point
// ---------------------------------------------------------------------------

#[test]
fn test_register_wrong_kind_returns_error() {
    let mut mgr = ExtensionManager::new();
    // Attempt to register an ACL at the "middleware" point — should fail
    let result = mgr.register("middleware", make_acl());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    assert!(err.message.contains("middleware"));
}

#[test]
fn test_register_unknown_point_returns_error() {
    let mut mgr = ExtensionManager::new();
    let result = mgr.register("bogus_point", make_acl());
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::GeneralInvalidInput);
}

#[test]
fn test_has_unknown_point_returns_error() {
    let mgr = ExtensionManager::new();
    let result = mgr.has("unknown_point");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::GeneralInvalidInput);
}

#[test]
fn test_count_unknown_point_returns_none() {
    let mgr = ExtensionManager::new();
    assert_eq!(mgr.count("unknown_point"), None);
}

// ---------------------------------------------------------------------------
// Apply wiring
// ---------------------------------------------------------------------------

#[test]
fn test_apply_wires_acl_into_executor() {
    use apcore::acl::ACLRule;

    let mut mgr = ExtensionManager::new();

    // Register an ACL with a permissive allow-all rule
    let acl = ACL::new(
        vec![{
            let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], "allow");
            rule.description = Some("allow all for test".to_string());
            rule
        }],
        "deny",
        None,
    );
    mgr.register("acl", ExtensionKind::Acl(Arc::new(acl)))
        .unwrap();

    let registry = Arc::new(Registry::new());
    let config = Arc::new(apcore::Config::default());
    let mut executor = Executor::new(Arc::clone(&registry), config);

    let result = mgr.apply(&registry, &mut executor);
    assert!(result.is_ok(), "apply() should succeed: {result:?}");
}

#[test]
fn test_apply_with_empty_manager_succeeds() {
    let mut mgr = ExtensionManager::new();
    let registry = Arc::new(Registry::new());
    let config = Arc::new(apcore::Config::default());
    let mut executor = Executor::new(Arc::clone(&registry), config);
    assert!(mgr.apply(&registry, &mut executor).is_ok());
}

#[test]
fn test_apply_wires_multiple_middleware() {
    use apcore::LoggingMiddleware;

    let mut mgr = ExtensionManager::new();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(Arc::new(LoggingMiddleware::default())),
    )
    .unwrap();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(Arc::new(LoggingMiddleware::default())),
    )
    .unwrap();

    let registry = Arc::new(Registry::new());
    let config = Arc::new(apcore::Config::default());
    let mut executor = Executor::new(Arc::clone(&registry), config);

    assert!(mgr.apply(&registry, &mut executor).is_ok());
    // D-78: apply retains the store, so the same manager can wire a second
    // registry/executor pair.
    assert_eq!(mgr.count("middleware"), Some(2));
    assert_eq!(
        executor
            .middlewares()
            .iter()
            .filter(|n| *n == "logging")
            .count(),
        2
    );
}

// ---------------------------------------------------------------------------
// Sync EXT-003 — CompositeExporter wiring for N>=2 span exporters
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_composite_exporter_fans_out_spans_to_all_wrapped_exporters() {
    use apcore::observability::span::{Span, SpanExporter};
    use apcore::observability::{CompositeExporter, InMemoryExporter};

    let a = InMemoryExporter::new();
    let b = InMemoryExporter::new();
    let composite = CompositeExporter::new(vec![Box::new(a.clone()), Box::new(b.clone())]);
    let span = Span::new("test.span", "trace");
    composite.export(&span).await.unwrap();
    assert_eq!(a.get_spans().len(), 1, "first exporter must receive span");
    assert_eq!(b.get_spans().len(), 1, "second exporter must receive span");
}

#[test]
fn test_apply_wires_two_span_exporters_via_composite() {
    use apcore::observability::InMemoryExporter;

    let mut mgr = ExtensionManager::new();
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .unwrap();
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .unwrap();
    assert_eq!(mgr.count("span_exporter"), Some(2));

    let registry = Arc::new(Registry::new());
    let config = Arc::new(apcore::Config::default());
    let mut executor = Executor::new(Arc::clone(&registry), config);

    // Apply must wire both exporters via a CompositeExporter rather than
    // silently dropping the second one.
    mgr.apply(&registry, &mut executor)
        .expect("apply should succeed with two span_exporters");
}

// ---------------------------------------------------------------------------
// A-D-18 — span_exporter reconfigures an existing TracingMiddleware in place,
// it must NOT append a second one.
// ---------------------------------------------------------------------------

#[test]
fn test_apply_span_exporter_does_not_add_second_tracing_middleware() {
    use apcore::observability::tracing_middleware::TracingMiddleware;
    use apcore::observability::InMemoryExporter;

    let registry = Arc::new(Registry::new());
    let config = Arc::new(apcore::Config::default());
    let mut executor = Executor::new(Arc::clone(&registry), config);

    // Pre-register a TracingMiddleware so the chain already has one.
    executor
        .use_middleware(Box::new(TracingMiddleware::new(Box::new(
            InMemoryExporter::new(),
        ))))
        .expect("register tracing middleware");
    let tracing_count_before = executor
        .middlewares()
        .iter()
        .filter(|n| *n == "tracing")
        .count();
    assert_eq!(tracing_count_before, 1);

    // Apply a span_exporter extension.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .unwrap();
    mgr.apply(&registry, &mut executor)
        .expect("apply should succeed");

    // Still exactly one TracingMiddleware — the exporter was set in place, not
    // appended as a second middleware.
    let tracing_count_after = executor
        .middlewares()
        .iter()
        .filter(|n| *n == "tracing")
        .count();
    assert_eq!(
        tracing_count_after, 1,
        "span_exporter must reconfigure the existing TracingMiddleware, not add a second one"
    );
}

#[test]
fn test_apply_span_exporter_without_tracing_middleware_warns_and_adds_nothing() {
    use apcore::observability::InMemoryExporter;

    let registry = Arc::new(Registry::new());
    let config = Arc::new(apcore::Config::default());
    let mut executor = Executor::new(Arc::clone(&registry), config);

    let mut mgr = ExtensionManager::new();
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .unwrap();
    mgr.apply(&registry, &mut executor)
        .expect("apply should succeed even with no TracingMiddleware");

    // No TracingMiddleware was present, so none should have been added.
    let tracing_count = executor
        .middlewares()
        .iter()
        .filter(|n| *n == "tracing")
        .count();
    assert_eq!(tracing_count, 0, "no TracingMiddleware should be appended");
}

// ---------------------------------------------------------------------------
// Debug output
// ---------------------------------------------------------------------------

#[test]
fn test_debug_output_contains_manager_name() {
    let mgr = ExtensionManager::new();
    let debug_str = format!("{mgr:?}");
    assert!(debug_str.contains("ExtensionManager"));
}
