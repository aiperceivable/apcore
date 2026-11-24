//! Crate-root re-export parity test.
//!
//! Verifies that the symbols required by PROTOCOL_SPEC §12.2 (Core Component
//! Interface Contracts) and §8.8 (ErrorFormatterRegistry), plus the symbols
//! that `apcore-python` and `apcore-typescript` expose at their package root,
//! are all accessible directly from `apcore::*` without requiring callers to
//! navigate the internal module path.
//!
//! Regression for sync findings A-003, A-004, A-005, A-007, A-008 from
//! 2026-04-08, and D1-004, D1-005 from 2026-04-10.
//! Adding a new spec-required symbol? Add a row here too.

#![allow(unused_imports, dead_code)]

#[test]
fn test_middleware_manager_at_crate_root() {
    // A-003: PROTOCOL_SPEC §12.2 — MiddlewareManager is a required core component.
    use apcore::{Middleware, MiddlewareManager};
    let _: fn() -> MiddlewareManager = MiddlewareManager::new;
}

#[test]
fn test_middleware_concrete_classes_at_crate_root() {
    // Parity with apcore-python / apcore-typescript built-in middleware exports.
    use apcore::{
        AfterMiddleware, BeforeMiddleware, ErrorHistoryMiddleware, LoggingMiddleware,
        MetricsMiddleware, ObsLoggingMiddleware, PlatformNotifyMiddleware, RetryMiddleware,
        UsageMiddleware,
    };
    // Compile-time only — referencing the type ensures it resolves at the crate root.
    let _: Option<RetryMiddleware> = None;
    let _: Option<LoggingMiddleware> = None;
    let _: Option<MetricsMiddleware> = None;
    let _: Option<UsageMiddleware> = None;
    let _: Option<ErrorHistoryMiddleware> = None;
    let _: Option<ObsLoggingMiddleware> = None;
}

#[test]
fn test_error_formatter_registry_at_crate_root() {
    // A-004: PROTOCOL_SPEC §8.8 — ErrorFormatterRegistry is normative.
    use apcore::{ErrorFormatter, ErrorFormatterRegistry};
    // Calling a static method confirms the type and its inherent impl are reachable.
    let _ = ErrorFormatterRegistry::is_registered("nonexistent");
}

#[test]
fn test_build_minimal_strategy_at_crate_root() {
    // A-005: parity with apcore-typescript's `buildMinimalStrategy` export.
    use apcore::{
        build_internal_strategy, build_minimal_strategy, build_performance_strategy,
        build_standard_strategy, build_testing_strategy,
    };
    let _strategy = build_minimal_strategy();
}

#[test]
fn test_module_id_pattern_at_crate_root() {
    // A-007: parity with Python `MODULE_ID_PATTERN` and TypeScript
    // `MODULE_ID_PATTERN` constants — exposed in Rust as a function returning
    // `&'static Regex` due to lazy initialization, but reachable from the crate root.
    use apcore::module_id_pattern;
    let pattern = module_id_pattern();
    assert!(pattern.is_match("foo.bar"));
    assert!(pattern.is_match("a"));
    assert!(!pattern.is_match("Foo.bar"));
    assert!(!pattern.is_match("foo-bar"));
}

#[test]
fn test_registry_events_constants_at_crate_root() {
    // A-008: PROTOCOL_SPEC §12.2 MUST — "All SDKs MUST export these event
    // names as named constants. Consumers MUST NOT hardcode event name strings."
    use apcore::{registry_events, RegistryEvents, REGISTRY_EVENTS};

    // Module-style access: apcore::registry_events::REGISTER
    assert_eq!(registry_events::REGISTER, "register");
    assert_eq!(registry_events::UNREGISTER, "unregister");

    // Struct-associated-const access: apcore::RegistryEvents::REGISTER
    assert_eq!(RegistryEvents::REGISTER, "register");
    assert_eq!(RegistryEvents::UNREGISTER, "unregister");

    // Singleton instance access (parity with Python `REGISTRY_EVENTS["REGISTER"]`
    // and TypeScript `REGISTRY_EVENTS.REGISTER`): apcore::REGISTRY_EVENTS::REGISTER
    let _ = REGISTRY_EVENTS;
    assert_eq!(RegistryEvents::REGISTER, registry_events::REGISTER);
}

#[test]
fn test_pipeline_engine_at_crate_root() {
    // D1-004: parity with apcore-typescript's `PipelineEngine` export.
    use apcore::{
        ExecutionStrategy, PipelineContext, PipelineEngine, PipelineTrace, Step, StepResult,
        StepTrace, StrategyInfo,
    };
    // Compile-time — confirms the type resolves at the crate root.
    let _: Option<PipelineTrace> = None;
    let _: Option<StepTrace> = None;
    let _: Option<StrategyInfo> = None;
}

#[test]
fn test_builtin_pipeline_steps_at_crate_root() {
    // D1-005: parity with apcore-typescript's 11 Builtin* step exports.
    use apcore::{
        BuiltinACLCheck, BuiltinApprovalGate, BuiltinCallChainGuard, BuiltinContextCreation,
        BuiltinExecute, BuiltinInputValidation, BuiltinMiddlewareAfter, BuiltinMiddlewareBefore,
        BuiltinModuleLookup, BuiltinOutputValidation, BuiltinReturnResult,
    };
    // Compile-time references — if any aren't at the crate root, this test fails to compile.
    let _ = BuiltinContextCreation;
    let _ = BuiltinCallChainGuard;
    // BuiltinModuleLookup carries a per-instance toggle store (issue #71), so it
    // is constructed via Default rather than referenced as a unit value.
    let _ = BuiltinModuleLookup::default();
    let _ = BuiltinACLCheck;
    let _ = BuiltinApprovalGate;
    let _ = BuiltinInputValidation;
    let _ = BuiltinMiddlewareBefore;
    let _ = BuiltinExecute;
    let _ = BuiltinOutputValidation;
    let _ = BuiltinMiddlewareAfter;
    let _ = BuiltinReturnResult;
}

#[test]
fn test_other_required_exports_at_crate_root() {
    // Parity sweep — every symbol Python/TypeScript expose at the package root
    // should also be reachable from `apcore::*`.
    use apcore::{
        // Bindings
        AutoSchemaValue,
        BindingEntry,
        BindingHandler,
        BindingLoader,
        BindingsFile,
        // Cancel
        CancelToken,
        // Observability extras
        ErrorEntry,
        ErrorHistory,
        // Decorator
        FunctionModule,
        InMemoryExporter,
        MetricsCollector,
        OTLPExporter,
        // Schema
        RefResolver,
        SchemaExporter,
        SchemaLoader,
        SchemaValidator,
        Span,
        SpanExporter,
        StdoutExporter,
        // Tracing
        TraceContext,
        TraceParent,
        TypedBindingHandler,
        UsageCollector,
        UsageStats,
    };
    // Compile-time references — if any of these aren't at the crate root, this test fails to compile.
    let _: Option<CancelToken> = None;
}

#[test]
fn test_detect_id_conflicts_at_crate_root() {
    // D1-004: detect_id_conflicts, ConflictResult, ConflictType, ConflictSeverity
    // must be accessible from the crate root, matching apcore-python's
    // detect_id_conflicts function and ConflictResult/ConflictType/ConflictSeverity.
    use apcore::{detect_id_conflicts, ConflictResult, ConflictSeverity, ConflictType};
    use std::collections::HashSet;

    let existing: HashSet<String> = ["foo.bar".to_string()].into_iter().collect();

    // Duplicate ID -> Some(ConflictResult)
    let result: Option<ConflictResult> = detect_id_conflicts("foo.bar", &existing, &[], None);
    assert!(result.is_some());
    let conflict = result.unwrap();
    // ConflictType and ConflictSeverity should be usable as enum values.
    let _: ConflictType = conflict.conflict_type;
    let _: ConflictSeverity = conflict.severity;

    // No conflict
    assert!(detect_id_conflicts("baz.qux", &existing, &[], None).is_none());
}

#[test]
fn test_execution_cancelled_error_at_crate_root() {
    // D1-003: ExecutionCancelledError must be accessible from the crate root,
    // matching apcore-python's ExecutionCancelledError(Exception) class.
    use apcore::ExecutionCancelledError;
    let err = ExecutionCancelledError {
        module_id: Some("executor.email.send_email".to_string()),
        message: "Cancelled by user request".to_string(),
    };
    assert_eq!(err.module_id.as_deref(), Some("executor.email.send_email"));
    // Verify Display (thiserror) is implemented.
    let display = format!("{err}");
    assert!(!display.is_empty());
}

#[test]
fn test_event_subscriber_types_at_crate_root() {
    // D1-001/D1-002: EventSubscriber trait and concrete subscriber types must be
    // accessible from the crate root without navigating internal module paths.
    use apcore::{A2ASubscriber, EventSubscriber, WebhookSubscriber};
    // Compile-time — constructing a concrete subscriber confirms the types resolve.
    let sub = WebhookSubscriber::new("wh1", "https://example.com/hook", "*");
    let _sub2 = A2ASubscriber::new("a2a1", "https://platform.example.com", "module.*");
    // Verify trait object coercion works from crate root type path.
    let _: &dyn EventSubscriber = &sub;
}

#[test]
fn test_subscriber_registry_functions_at_crate_root() {
    // D1-002: create_subscriber, register_subscriber_type, unregister_subscriber_type,
    // reset_subscriber_registry must all be accessible from the crate root, matching
    // apcore-python create_subscriber_from_config/register_subscriber_factory and
    // apcore-typescript createSubscriberFromConfig/registerSubscriberType at root.
    use apcore::{
        create_subscriber, register_subscriber_type, reset_subscriber_registry,
        unregister_subscriber_type,
    };
    // reset_subscriber_registry restores built-ins — safe to call in tests.
    reset_subscriber_registry();
    // Confirm the function signatures are callable with expected argument types.
    let result = unregister_subscriber_type("nonexistent_type_xyz");
    assert!(result.is_err());
    // create_subscriber resolves at the crate root; an unknown type errors.
    let made = create_subscriber(&serde_json::json!({"type": "nonexistent_type_xyz"}));
    assert!(made.is_err());
}

#[test]
fn test_sys_modules_concrete_classes_at_crate_root() {
    // D1-004: every concrete system-module struct must be reachable from the crate
    // root, matching apcore-python (HealthModule, HealthSummaryModule, ManifestModule,
    // ManifestFullModule, UsageModule, UsageSummaryModule, ReloadModule,
    // ToggleFeatureModule, UpdateConfigModule) and the equivalent apcore-typescript
    // root exports. Previously only UpdateConfigModule was re-exported at the root.
    use apcore::{
        HealthModule, HealthSummaryModule, ManifestFullModule, ManifestModule, ReloadModule,
        ToggleFeatureModule, UpdateConfigModule, UsageModule, UsageSummaryModule,
    };
    // Compile-time references — if any struct isn't at the crate root, this fails to compile.
    let _: Option<HealthModule> = None;
    let _: Option<HealthSummaryModule> = None;
    let _: Option<ManifestModule> = None;
    let _: Option<ManifestFullModule> = None;
    let _: Option<UsageModule> = None;
    let _: Option<UsageSummaryModule> = None;
    let _: Option<ReloadModule> = None;
    let _: Option<ToggleFeatureModule> = None;
    let _: Option<UpdateConfigModule> = None;
}

#[test]
fn test_async_task_manager_at_crate_root() {
    // sync B-020 / v0.19.0: AsyncTaskManager is re-exported from the crate root
    // after being reintroduced in 0.19.0 with full Executor integration. Parity with
    // apcore-python AsyncTaskManager and apcore-typescript AsyncTaskManager exports.
    use apcore::{AsyncTaskManager, TaskInfo, TaskStatus};
    // Compile-time — referencing the types ensures they resolve at the crate root.
    let _: Option<TaskInfo> = None;
    let _: Option<TaskStatus> = None;
    // AsyncTaskManager takes an Executor + concurrency knobs; verifying the
    // type path is enough here.
    let _: Option<AsyncTaskManager> = None;
}

#[test]
fn test_extension_manager_at_crate_root() {
    // sync B-020 / v0.19.0: ExtensionManager and ExtensionPoint are re-exported
    // from the crate root after being reintroduced in 0.19.0. Parity with
    // apcore-python ExtensionManager/ExtensionPoint and apcore-typescript exports.
    use apcore::{ExtensionKind, ExtensionManager, ExtensionPoint};
    let manager = ExtensionManager::new();
    // Compile-time references confirm the types resolve at the crate root.
    let _: &ExtensionManager = &manager;
    let _: Option<ExtensionPoint> = None;
    let _: Option<ExtensionKind> = None;
}
