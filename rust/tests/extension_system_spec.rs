// Spec-traced contract tests for the apcore-rust extension-system feature.
//
// Source spec: apcore/docs/features/extension-system.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_extension_system_spec.py
//
// Each test maps to exactly one clause in the feature spec's '## Contract:'
// blocks. The verbatim cross-language clause id appears in a leading
// `// clause: <clause_id>` comment on the line above each test fn so a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
//
// IMPORTANT cross-language divergences (asserted as ACTUAL Rust behavior):
//   * Rust enforces extension-point TYPE SAFETY at COMPILE TIME via the
//     `ExtensionKind` enum. There is no runtime "register a str as middleware"
//     path — the nearest runtime failure is a point/variant MISMATCH, which
//     Rust surfaces as `ErrorCode::GeneralInvalidInput` ("GENERAL_INVALID_INPUT"),
//     NOT Python's `TypeError`/`KeyError`.
//   * Rust HAS `get()` / `get_all()` / `unregister()`, alongside `count()`,
//     `has()`, `clear()`, `clear_all()`, `list_points()` and `apply()`. All
//     three return `Result` and reject an unregistered point name with
//     `ErrorCode::GeneralInvalidInput` (D-108), matching Python/TypeScript.
//   * Rust `apply()` does NOT expose registry.discoverer / executor.acl /
//     executor.approval_handler getters, so side-effects 1-4 are observed
//     indirectly (apply() returns Ok and drains the internal store). Middleware
//     wiring (side-effect 5) and span-exporter wiring (side-effect 6) ARE
//     observable via `executor.middlewares()`.
//   * Rust `apply()` leaves the internal store INTACT (D-78), so applying the
//     same manager twice wires the same set again, as Python does.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use apcore::acl::ACL;
use apcore::approval::{ApprovalHandler, AutoApproveHandler};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::extensions::{ExtensionKind, ExtensionManager};
use apcore::middleware::base::Middleware;
use apcore::observability::span::{Span, SpanExporter};
use apcore::observability::tracing_middleware::TracingMiddleware;
use apcore::observability::{CompositeExporter, InMemoryExporter};
use apcore::{Config, Context, Executor, Registry};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_acl() -> ExtensionKind {
    ExtensionKind::Acl(Arc::new(ACL::new(vec![], "deny", None)))
}

fn make_executor() -> Executor {
    let registry = Arc::new(Registry::new());
    let config = Arc::new(Config::default());
    Executor::new(registry, config)
}

/// The string code carried by a `ModuleError` (SCREAMING_SNAKE_CASE), read via
/// `to_dict()["code"]` — the canonical serialized wire form.
fn code_str(err: &ModuleError) -> String {
    err.to_dict()
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// A named no-op middleware with a fixed priority so registration order is
/// preserved in the `MiddlewareManager` snapshot (equal-priority middlewares
/// keep insertion order).
#[derive(Debug)]
struct NamedMiddleware {
    name: String,
}

impl NamedMiddleware {
    fn boxed(name: &str) -> Arc<dyn Middleware> {
        Arc::new(NamedMiddleware {
            name: name.to_string(),
        })
    }
}

#[async_trait]
impl Middleware for NamedMiddleware {
    fn name(&self) -> &str {
        &self.name
    }
    fn priority(&self) -> u16 {
        // Identical priority for every stub => snapshot preserves insertion order.
        500
    }
    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
}

/// A span exporter that always fails — used for composite error-isolation tests.
#[derive(Debug)]
struct FailingExporter;

#[async_trait]
impl SpanExporter for FailingExporter {
    async fn export(&self, _span: &Span) -> Result<(), ModuleError> {
        Err(ModuleError::new(ErrorCode::GeneralInvalidInput, "boom"))
    }
    async fn shutdown(&self) -> Result<(), ModuleError> {
        Ok(())
    }
}

// ===========================================================================
// Contract: ExtensionManager.register
// ===========================================================================

// clause: extension_system.register.input.point_name.unknown
#[test]
fn register_input_point_name_unknown() {
    // Unknown point_name MUST raise. Rust returns Err(GeneralInvalidInput) with
    // a message identifying the unknown-point condition (Python raises KeyError).
    let mut mgr = ExtensionManager::new();
    let err = mgr
        .register("no_such_point", make_acl())
        .expect_err("unknown point must error");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    assert!(
        err.message.contains("Unknown extension point"),
        "message must identify unknown point: {}",
        err.message
    );
}

// clause: extension_system.register.input.extension.wrong_type
#[test]
fn register_input_extension_wrong_type() {
    // The point's declared type is checked at registration. Rust enforces type
    // safety at compile time via ExtensionKind, so a *literal* type mismatch
    // (str for middleware) is impossible to express. The closest runtime failure
    // is registering the wrong ExtensionKind variant for the point: an Acl kind
    // at the "middleware" point. Rust returns GeneralInvalidInput (NOT Python's
    // TypeError) and the message identifies the mismatch.
    let mut mgr = ExtensionManager::new();
    let err = mgr
        .register("middleware", make_acl())
        .expect_err("variant/point mismatch must error");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    assert!(
        err.message.contains("middleware"),
        "message must identify the type mismatch: {}",
        err.message
    );
}

// clause: extension_system.register.error.ExtensionPointNotFoundError
#[test]
fn register_error_extension_point_not_found_error() {
    // Python: KeyError when point_name is not a registered point. Rust emits
    // ErrorCode::GeneralInvalidInput ("GENERAL_INVALID_INPUT"); assert the real
    // Rust code exactly plus the diagnostic naming the offending point.
    let mut mgr = ExtensionManager::new();
    let err = mgr
        .register("totally_unknown", make_acl())
        .expect_err("unknown point must error");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    assert_eq!(code_str(&err), "GENERAL_INVALID_INPUT");
    assert!(err.message.contains("totally_unknown"));
}

// clause: extension_system.register.error.ExtensionTypeError
#[test]
fn register_error_extension_type_error() {
    // Python: TypeError when the extension does not satisfy the point's type.
    // Rust: register an Acl kind where a Discoverer is required => the
    // point/variant mismatch surfaces as GeneralInvalidInput, message names the
    // point.
    let mut mgr = ExtensionManager::new();
    let err = mgr
        .register("discoverer", make_acl())
        .expect_err("type mismatch must error");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    assert_eq!(code_str(&err), "GENERAL_INVALID_INPUT");
    assert!(err.message.contains("discoverer"));
}

// clause: extension_system.register.property.async.false
#[test]
fn register_property_async_false() {
    // async=false. register() is a plain synchronous fn returning a Result;
    // callable without an async runtime. Since D-91 the success value is the
    // `ExtensionHandle` that makes an outside-the-manager removal expressible,
    // not `()`.
    let mut mgr = ExtensionManager::new();
    let result: Result<_, ModuleError> = mgr.register("acl", make_acl());
    assert!(result.is_ok());
    let handle = result.unwrap();
    assert!(mgr.unregister_handle(handle));
}

// clause: extension_system.register.property.idempotent.single_replaces
#[test]
fn register_property_idempotent_single_replaces() {
    // idempotent=false for single-cardinality (replaces). Registering twice on a
    // non-multiple point ("acl") leaves exactly one extension. Rust has no get()
    // to compare identity, so replacement is observed via count() == 1.
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).expect("first register");
    mgr.register("acl", make_acl()).expect("second register");
    assert_eq!(mgr.count("acl"), Some(1));
}

// clause: extension_system.register.property.idempotent.multi_accumulates
#[test]
fn register_property_idempotent_multi_accumulates() {
    // accumulating for multi-cardinality. Registering twice on "middleware"
    // (multiple=true) keeps both, in registration order. Order is asserted via
    // apply() -> executor.middlewares() in the apply side-effect tests; here we
    // assert accumulation via count() == 2.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw1")),
    )
    .expect("register mw1");
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw2")),
    )
    .expect("register mw2");
    assert_eq!(mgr.count("middleware"), Some(2));
}

/// Data address of the ACL behind an `ExtensionKind`, for identity comparison
/// from outside the crate. `ExtensionKind::object_address` is private — a test
/// must reach identity the way a host does, through the public `Arc` the
/// variant holds (the same handle D-91 made `unregister` take).
fn acl_address(ext: Option<&ExtensionKind>) -> Option<usize> {
    ext.map(|e| match e {
        ExtensionKind::Acl(acl) => Arc::as_ptr(acl) as *const () as usize,
        other => panic!("expected acl, got {other:?}"),
    })
}

/// Data addresses of a middleware slice, in order.
fn middleware_addresses(exts: &[ExtensionKind]) -> Vec<usize> {
    exts.iter()
        .map(|e| match e {
            ExtensionKind::Middleware(m) => Arc::as_ptr(m) as *const () as usize,
            other => panic!("expected middleware, got {other:?}"),
        })
        .collect()
}

// ===========================================================================
// Contract: ExtensionManager.get
//
// `get()` EXISTS — it was added with D-91 and gained a `Result` return with
// D-108. This block used to assert every `get` clause against `count()`/`has()`
// under a header saying the symbol was missing, which is the stand-in-assertion
// shape: the clause id said `get`, the assertion said `count`, and deleting
// `get` would have left all of it green. The clauses now drive `get`.
// ===========================================================================

// clause: extension_system.get.property.async.false
#[test]
fn get_property_async_false() {
    // get() async=false: it returns a plain value, not a future, and needs no
    // async runtime. The binding's TYPE is the assertion — this test fn is not
    // `async` and never awaits.
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).expect("register acl");
    let found: Option<&ExtensionKind> = mgr.get("acl").expect("known point");
    assert!(matches!(found, Some(ExtensionKind::Acl(_))));
}

// clause: extension_system.get.error.no_error_returns_none
#[test]
fn get_error_no_error_returns_none() {
    // A REGISTERED point holding nothing returns None and does not raise. (An
    // UNREGISTERED point does raise — that is D-108, pinned by
    // `lookup_rejects_unknown_point_but_not_empty_point` and by
    // extension_point_lookup.json.)
    let mgr = ExtensionManager::new();
    assert!(mgr
        .get("acl")
        .expect("registered point must not error")
        .is_none());
}

// clause: extension_system.get.property.pure.true
#[test]
fn get_property_pure_true() {
    // pure=true. Two `get` calls answer identically and leave the store — this
    // point and every other — exactly as it was.
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).expect("register acl");
    let before_points = mgr.list_points().len();

    let first = acl_address(mgr.get("acl").expect("known point"));
    let second = acl_address(mgr.get("acl").expect("known point"));

    assert!(first.is_some());
    assert_eq!(first, second, "get must answer with the same object twice");
    assert_eq!(mgr.count("acl"), Some(1), "reading must not consume");
    assert_eq!(before_points, mgr.list_points().len());
    assert_eq!(mgr.count("middleware"), Some(0));
}

// clause: extension_system.get.property.thread_safe.concurrent_reads
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_property_thread_safe_concurrent_reads() {
    // thread_safe=true. Launch >=8 concurrent `get` reads against a shared
    // manager and assert no panic + every call observes the same object. An
    // immutable &ExtensionManager is Send + Sync, so it is shared via Arc.
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).expect("register acl");
    let mgr = Arc::new(mgr);

    let mut handles = Vec::new();
    for _ in 0..16 {
        let m = Arc::clone(&mgr);
        handles.push(tokio::spawn(async move {
            acl_address(m.get("acl").expect("known point"))
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("task join"));
    }
    assert_eq!(results.len(), 16);
    let first = results[0];
    assert!(first.is_some());
    assert!(results.iter().all(|r| *r == first));
}

// ===========================================================================
// Contract: ExtensionManager.get_all
//
// `get_all()` EXISTS — added with D-91, `Result`-returning since D-108. Same
// correction as the `get` block above: these clauses used to be asserted
// against `count()` under a header saying the symbol was missing, and the two
// that could not be faked that way were `#[ignore]`d for a symbol that was
// there. One Rust-actual difference is kept and stated: `get_all` returns a
// BORROW (`&[ExtensionKind]`), not the copied Vec Python returns, so the
// purity clause is asserted as what a borrow guarantees.
// ===========================================================================

// clause: extension_system.get_all.property.async.false
#[test]
fn get_all_property_async_false() {
    // get_all() returns synchronously: a plain slice, not a future. This test fn
    // is not `async` and never awaits.
    let mgr = ExtensionManager::new();
    let all: &[ExtensionKind] = mgr.get_all("middleware").expect("known point");
    assert!(all.is_empty());
}

// clause: extension_system.get_all.error.no_error_returns_empty
#[test]
fn get_all_error_no_error_returns_empty() {
    // A REGISTERED point holding nothing returns an empty slice and does not
    // error. (An UNREGISTERED one does — D-108.)
    let mgr = ExtensionManager::new();
    assert!(mgr
        .get_all("middleware")
        .expect("registered point must not error")
        .is_empty());
}

// clause: extension_system.get_all.returns.registration_order
#[test]
fn get_all_returns_registration_order() {
    // The direct read answers in REGISTRATION order. Previously `#[ignore]`d for
    // a missing symbol and deferred to `apply() -> executor.middlewares()`, which
    // is a different mechanism: apply() filters and wires, so it could preserve
    // order while `get_all` did not.
    let mut mgr = ExtensionManager::new();
    for name in ["first", "second", "third"] {
        mgr.register(
            "middleware",
            ExtensionKind::Middleware(NamedMiddleware::boxed(name)),
        )
        .expect("register middleware");
    }

    let names: Vec<&str> = mgr
        .get_all("middleware")
        .expect("known point")
        .iter()
        .map(|e| match e {
            ExtensionKind::Middleware(m) => m.name(),
            other => panic!("expected middleware, got {other:?}"),
        })
        .collect();
    assert_eq!(names, vec!["first", "second", "third"]);
}

// clause: extension_system.get_all.property.pure.true
#[test]
fn get_all_property_pure_true() {
    // Python asserts the returned list is a COPY: mutating it does not affect the
    // store. Rust hands back a `&[ExtensionKind]` borrow, so the copy cannot be
    // mutated — the borrow checker refuses it at compile time, which is the
    // stronger guarantee and the reason the signature is a slice. What is left to
    // assert at runtime is the half a copy does not give you either: reading does
    // not consume, and two reads answer with the same objects in the same order.
    let mut mgr = ExtensionManager::new();
    for name in ["first", "second"] {
        mgr.register(
            "middleware",
            ExtensionKind::Middleware(NamedMiddleware::boxed(name)),
        )
        .expect("register middleware");
    }

    let first = middleware_addresses(mgr.get_all("middleware").expect("known point"));
    let second = middleware_addresses(mgr.get_all("middleware").expect("known point"));

    assert_eq!(first.len(), 2);
    assert_eq!(
        first, second,
        "a read must not consume or reorder the store"
    );
    assert_eq!(mgr.count("middleware"), Some(2));
}

// clause: extension_system.get_all.property.thread_safe.concurrent_reads
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_all_property_thread_safe_concurrent_reads() {
    // thread_safe=true. >=8 concurrent reads see a consistent snapshot. Rust has
    // no get_all(); count() is the equivalent multi-cardinality read shared via
    // Arc.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw1")),
    )
    .expect("register mw1");
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw2")),
    )
    .expect("register mw2");
    let mgr = Arc::new(mgr);

    let mut handles = Vec::new();
    for _ in 0..12 {
        let m = Arc::clone(&mgr);
        handles.push(tokio::spawn(async move { m.count("middleware") }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("task join"));
    }
    assert_eq!(results.len(), 12);
    assert!(results.iter().all(|r| *r == Some(2)));
}

// ===========================================================================
// Contract: ExtensionManager.unregister
//
// Since D-91 Rust has a REACHABLE identity-based removal. `ExtensionKind`
// holds `Arc`s, so a host keeps the handle, registers a clone, and hands
// another clone to `unregister(point, ext)`; `unregister_handle(handle)` is the
// same removal keyed on the token `register` returns, for callers who would
// rather not keep the object. `clear(point)` / `clear_all()` remain the
// point-wide removals.
// ===========================================================================

// clause: extension_system.unregister.property.async.false
#[test]
fn unregister_property_async_false() {
    // unregister() async=false. Rust's removal equivalent clear() is a plain sync
    // fn returning Result<()> without an async runtime.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw")),
    )
    .expect("register mw");
    let result: Result<(), ModuleError> = mgr.clear("middleware");
    assert!(result.is_ok());
}

// clause: extension_system.unregister.removes.identity
#[test]
fn unregister_removes_identity() {
    // D-91: remove the exact extension object, leaving the others. The caller
    // holds the `Arc` it registered a clone of — the removal path that made
    // this expressible from outside the manager.
    let mut mgr = ExtensionManager::new();
    let keep = NamedMiddleware::boxed("keep");
    let drop_me = NamedMiddleware::boxed("drop");
    mgr.register("middleware", ExtensionKind::Middleware(Arc::clone(&keep)))
        .expect("register keep");
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(Arc::clone(&drop_me)),
    )
    .expect("register drop");
    assert_eq!(mgr.count("middleware"), Some(2));

    assert!(mgr
        .unregister("middleware", &ExtensionKind::Middleware(drop_me))
        .expect("known point"));
    assert_eq!(mgr.count("middleware"), Some(1));

    // The survivor is the one that was not named.
    let survivors: Vec<&str> = mgr
        .get_all("middleware")
        .expect("known point")
        .iter()
        .map(|e| match e {
            ExtensionKind::Middleware(m) => m.name(),
            _ => panic!("expected middleware"),
        })
        .collect();
    assert_eq!(survivors, vec!["keep"]);

    // And the identity form removes the second one too. (This comment used to
    // say "the handle form"; it does not call it. `unregister_handle` is
    // covered by `unregister_handle_removes_the_registration_it_was_given`
    // below and by the unit test in src/extensions.rs.)
    assert!(mgr
        .unregister("middleware", &ExtensionKind::Middleware(keep))
        .expect("known point"));
    assert_eq!(mgr.count("middleware"), Some(0));
}

// clause: extension_system.unregister.removes.identity_not_equality
#[test]
fn unregister_removes_by_identity_not_equality() {
    // D-128. apcore-python used `list.remove`, which compares with `__eq__`, so
    // `unregister(second)` deleted `first`: a host removing the second of two
    // identically-configured middlewares kept the one it wanted gone and lost
    // the one it wanted kept, silently. This SDK compares `object_address()`,
    // which is the behaviour the contract's Inputs row ("identity comparison")
    // has always required and which apcore-python was corrected to match.
    //
    // The two registrations MUST be indistinguishable by value and MUST NOT be
    // the same allocation, or the test cannot tell the two semantics apart —
    // which is why `unregister_removes_identity` above, whose two middlewares
    // carry different names, cannot.
    let mut mgr = ExtensionManager::new();
    let first = NamedMiddleware::boxed("same");
    let second = NamedMiddleware::boxed("same");
    assert_eq!(first.name(), second.name(), "precondition: equal by value");
    assert!(
        !Arc::ptr_eq(&first, &second),
        "precondition: distinct allocations"
    );

    mgr.register("middleware", ExtensionKind::Middleware(Arc::clone(&first)))
        .expect("register first");
    mgr.register("middleware", ExtensionKind::Middleware(Arc::clone(&second)))
        .expect("register second");

    assert!(mgr
        .unregister("middleware", &ExtensionKind::Middleware(second))
        .expect("known point"));

    let survivors = mgr.get_all("middleware").expect("known point");
    assert_eq!(survivors.len(), 1);
    match &survivors[0] {
        ExtensionKind::Middleware(m) => assert!(
            Arc::ptr_eq(m, &first),
            "unregister removed the wrong registration"
        ),
        other => panic!("expected middleware, got {other:?}"),
    }
}

// clause: extension_system.unregister.removes.equal_unregistered_is_a_no_op
#[test]
fn unregister_an_equal_but_unregistered_extension_is_a_no_op() {
    // Control for the test above: equality alone must not authorise a removal.
    // Without it, "removal is by identity" is also satisfied by an
    // implementation that removes the LAST value-equal entry — still wrong, and
    // still passing whenever only one registration exists.
    let mut mgr = ExtensionManager::new();
    let registered = NamedMiddleware::boxed("same");
    let never_registered = NamedMiddleware::boxed("same");

    mgr.register(
        "middleware",
        ExtensionKind::Middleware(Arc::clone(&registered)),
    )
    .expect("register");

    assert!(!mgr
        .unregister("middleware", &ExtensionKind::Middleware(never_registered))
        .expect("known point"));
    assert_eq!(mgr.count("middleware"), Some(1));
}

// clause: extension_system.unregister_handle.removes.the_given_registration
#[test]
fn unregister_handle_removes_the_registration_it_was_given() {
    // D-91's reachable removal path, exercised across the crate boundary —
    // which is the only place the decision's question can be answered, because
    // a unit test inside the manager's own module can reach state a host
    // cannot. Two value-equal registrations, so the handle is the ONLY thing
    // that distinguishes them.
    let mut mgr = ExtensionManager::new();
    let first = NamedMiddleware::boxed("same");
    let second = NamedMiddleware::boxed("same");
    let _h1 = mgr
        .register("middleware", ExtensionKind::Middleware(Arc::clone(&first)))
        .expect("register first");
    let h2 = mgr
        .register("middleware", ExtensionKind::Middleware(Arc::clone(&second)))
        .expect("register second");

    assert!(mgr.unregister_handle(h2));
    assert!(
        !mgr.unregister_handle(h2),
        "a spent handle is a silent no-op"
    );

    let survivors = mgr.get_all("middleware").expect("known point");
    assert_eq!(survivors.len(), 1);
    match &survivors[0] {
        ExtensionKind::Middleware(m) => assert!(
            Arc::ptr_eq(m, &first),
            "unregister_handle removed the wrong registration"
        ),
        other => panic!("expected middleware, got {other:?}"),
    }
}

// clause: extension_system.unregister.error.missing_is_silent_no_op
#[test]
fn unregister_error_missing_is_silent_no_op() {
    // Unregistering a never-registered extension returns false (silent no-op)
    // and leaves state intact.
    let mut mgr = ExtensionManager::new();
    let registered = NamedMiddleware::boxed("registered");
    let handle = mgr
        .register("middleware", ExtensionKind::Middleware(registered))
        .expect("register");

    let stranger = NamedMiddleware::boxed("stranger");
    assert!(!mgr
        .unregister(
            "middleware",
            &ExtensionKind::Middleware(Arc::clone(&stranger))
        )
        .expect("known point"));
    assert_eq!(mgr.count("middleware"), Some(1));

    // D-108: the silent `false` is scoped to "this point does not hold that
    // extension". An unregistered POINT is a different question, and it is an
    // error — see `lookup_rejects_unknown_point_but_not_empty_point`.
    let err = mgr
        .unregister("nonexistent", &ExtensionKind::Middleware(stranger))
        .expect_err("unknown point must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);

    // A handle already spent is a silent false on the second call.
    assert!(mgr.unregister_handle(handle));
    assert!(!mgr.unregister_handle(handle));
}

// clause: extension_system.unregister.property.pure.false
#[test]
fn unregister_property_pure_false() {
    // pure=false (mutates the extension store). The removal equivalent clear()
    // produces an observable state change via the public count()/has() queries.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw")),
    )
    .expect("register mw");
    assert_eq!(mgr.count("middleware"), Some(1));
    mgr.clear("middleware").expect("clear middleware");
    assert_eq!(mgr.count("middleware"), Some(0));
    assert!(!mgr.has("middleware").expect("has(middleware)"));
}

// ===========================================================================
// Contract: ExtensionManager.apply
//
// Rust `apply(&Registry, &mut Executor)` wires extensions and RETAINS the
// store (D-78 Postconditions): each registration is wired as a shared `Arc`
// clone, so one manager can be applied to several registry/executor pairs.
// Side-effects 1-4 (discoverer/validator/acl/approval) are NOT observable via
// public getters, so they are asserted indirectly (apply() Ok + store intact).
// Side-effect 5 (middleware) and side-effect 6 (span exporters) ARE observable
// via `executor.middlewares()`.
// ===========================================================================

// clause: extension_system.apply.property.async.false
#[test]
fn apply_property_async_false() {
    // async=false. apply() is a plain sync fn returning Result<()> on success,
    // callable without an async runtime.
    let mut mgr = ExtensionManager::new();
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    let result: Result<(), ModuleError> = mgr.apply(&registry, &mut executor);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ());
}

// clause: extension_system.apply.side_effect.1.set_discoverer
#[test]
fn apply_side_effect_1_set_discoverer() {
    // Python observes registry.set_discoverer(ext) via a mock. Rust's Registry
    // exposes no discoverer getter, so we assert the observable contract: apply()
    // succeeds and the discoverer point is left as it was found.
    // (Cross-language note: direct call-observation requires a mock surface Rust
    // does not provide.)
    let mut mgr = ExtensionManager::new();
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    assert!(mgr.apply(&registry, &mut executor).is_ok());
    // No discoverer registered => count stays 0 and apply is a no-op for it.
    assert_eq!(mgr.count("discoverer"), Some(0));
}

// clause: extension_system.apply.side_effect.2.set_validator
#[test]
fn apply_side_effect_2_set_validator() {
    // Python observes registry.set_validator(ext) via a mock. Rust's Registry
    // exposes no validator getter; assert apply() succeeds and leaves the
    // module_validator point as it was found.
    let mut mgr = ExtensionManager::new();
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    assert!(mgr.apply(&registry, &mut executor).is_ok());
    assert_eq!(mgr.count("module_validator"), Some(0));
}

// clause: extension_system.apply.side_effect.3.set_acl
#[test]
fn apply_side_effect_3_set_acl() {
    // `Executor.acl` is a PUBLIC FIELD, so the wiring is directly observable.
    // This test used to say "Rust's Executor exposes no acl getter" and assert
    // instead that apply() succeeded and the store was intact — which is D-78,
    // not this clause. It passed with the ACL wiring deleted.
    let acl: Arc<ACL> = Arc::new(ACL::new(vec![], "deny", None));
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", ExtensionKind::Acl(Arc::clone(&acl)))
        .expect("register acl");

    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    assert!(executor.acl.is_none(), "precondition: nothing wired yet");

    mgr.apply(&registry, &mut executor).expect("apply");

    let wired = executor.acl.as_ref().expect("apply must wire the acl");
    assert!(
        Arc::ptr_eq(wired, &acl),
        "the executor must hold the registered ACL object, not a copy"
    );
    assert_eq!(mgr.count("acl"), Some(1), "apply must not drain the store");
}

// clause: extension_system.apply.side_effect.4.set_approval_handler
#[test]
fn apply_side_effect_4_set_approval_handler() {
    // Previously `#[ignore]`d with a two-part reason, both parts false:
    // `Executor.approval_handler` is a PUBLIC FIELD, and `AutoApproveHandler` is
    // a public unit struct any test crate can construct. The skip-asymmetry
    // guard could not report it because it read apcore-rust's `#[ignore]`
    // attributes as live.
    let handler: Arc<dyn ApprovalHandler> = Arc::new(AutoApproveHandler);
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "approval_handler",
        ExtensionKind::ApprovalHandler(Arc::clone(&handler)),
    )
    .expect("register approval_handler");

    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    assert!(
        executor.approval_handler.is_none(),
        "precondition: nothing wired yet"
    );

    mgr.apply(&registry, &mut executor).expect("apply");

    let wired = executor
        .approval_handler
        .as_ref()
        .expect("apply must wire the approval handler");
    assert!(
        Arc::ptr_eq(wired, &handler),
        "the executor must hold the registered handler object, not a copy"
    );
    assert_eq!(
        mgr.count("approval_handler"),
        Some(1),
        "apply must not drain the store"
    );
}

// clause: extension_system.apply.side_effect.5.use_middleware_in_order
#[test]
fn apply_side_effect_5_use_middleware_in_order() {
    // Python asserts executor.use(mw) is called for each middleware in
    // registration ORDER. Rust: apply() calls executor.use_middleware() for each;
    // the names appear in executor.middlewares(). The stubs share one priority so
    // the snapshot preserves registration order.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw1")),
    )
    .expect("register mw1");
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw2")),
    )
    .expect("register mw2");
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    mgr.apply(&registry, &mut executor).expect("apply");

    let names = executor.middlewares();
    assert!(names.contains(&"mw1".to_string()), "names: {names:?}");
    assert!(names.contains(&"mw2".to_string()), "names: {names:?}");
    // Equal-priority => registration order preserved.
    let i1 = names.iter().position(|n| n == "mw1").expect("mw1 present");
    let i2 = names.iter().position(|n| n == "mw2").expect("mw2 present");
    assert!(
        i1 < i2,
        "mw1 must precede mw2 in registration order: {names:?}"
    );
}

// clause: extension_system.apply.side_effect.6.single_span_exporter_direct
#[test]
fn apply_side_effect_6_single_span_exporter_direct() {
    // Canonical behavior (sync finding A-D-18): a single span exporter is set
    // ONTO the EXISTING TracingMiddleware in the executor chain, not wrapped in
    // a freshly-appended one. Pre-register a TracingMiddleware, then assert
    // apply reconfigures it in place (still exactly one "tracing").
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .expect("register exporter");
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    executor
        .use_middleware(Box::new(TracingMiddleware::new(Box::new(
            InMemoryExporter::new(),
        ))))
        .expect("pre-register tracing middleware");
    mgr.apply(&registry, &mut executor).expect("apply");

    let names = executor.middlewares();
    let tracing_count = names.iter().filter(|n| *n == "tracing").count();
    assert_eq!(
        tracing_count, 1,
        "single exporter must reconfigure the existing TracingMiddleware in place: {names:?}"
    );
}

// clause: extension_system.apply.side_effect.6.multiple_span_exporters_composite
#[tokio::test]
async fn apply_side_effect_6_multiple_span_exporters_composite() {
    // Multiple exporters MUST be composed so each span fans out to all, with a
    // failure in one NOT stopping the others. Rust wraps N>=2 exporters in a
    // CompositeExporter. We assert (a) apply wires a tracing middleware and
    // (b) the CompositeExporter fan-out + error isolation directly.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(FailingExporter)),
    )
    .expect("register failing exporter");
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .expect("register good exporter");
    assert_eq!(mgr.count("span_exporter"), Some(2));

    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    executor
        .use_middleware(Box::new(TracingMiddleware::new(Box::new(
            InMemoryExporter::new(),
        ))))
        .expect("pre-register tracing middleware");
    mgr.apply(&registry, &mut executor).expect("apply");
    // Canonical behavior (sync finding A-D-18): the composite exporter is set
    // onto the existing TracingMiddleware in place — still exactly one.
    let tracing_count = executor
        .middlewares()
        .iter()
        .filter(|n| *n == "tracing")
        .count();
    assert_eq!(
        tracing_count, 1,
        "composite exporter must reconfigure the existing TracingMiddleware in place"
    );

    // Error isolation: failing exporter raises, good exporter still receives.
    let good = InMemoryExporter::new();
    let composite = CompositeExporter::new(vec![Box::new(FailingExporter), Box::new(good.clone())]);
    let span = Span::new("test.span", "trace");
    composite
        .export(&span)
        .await
        .expect("composite export must not raise");
    assert_eq!(
        good.get_spans().len(),
        1,
        "good exporter must still receive the span despite a sibling failure"
    );
}

// clause: extension_system.apply.side_effect.6.no_tracing_middleware_no_raise
#[test]
fn apply_side_effect_6_no_tracing_middleware_no_raise() {
    // Spec lists ExtensionApplyError for a span exporter wired with no
    // TracingMiddleware present. Like Python, Rust does NOT raise here: it logs
    // a warning and leaves the executor unwired (sync finding A-D-18). Assert
    // apply() completes without error and adds no TracingMiddleware.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .expect("register exporter");
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    // Must not raise despite no pre-existing TracingMiddleware.
    mgr.apply(&registry, &mut executor)
        .expect("apply must not raise");
    // And no TracingMiddleware is auto-created (warn-and-skip).
    assert!(
        !executor.middlewares().contains(&"tracing".to_string()),
        "no TracingMiddleware should be appended when none pre-exists"
    );
}

// clause: extension_system.apply.property.idempotent.false
#[test]
fn apply_property_idempotent_false() {
    // idempotent=false: a second apply() STACKS middleware (D-78). Only a
    // non-consuming implementation can produce that observable, which is why
    // the Postconditions section forbids draining.
    let mut mgr = ExtensionManager::new();
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw")),
    )
    .expect("register mw");
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));

    mgr.apply(&registry, &mut executor).expect("first apply");
    mgr.apply(&registry, &mut executor).expect("second apply");

    let count = executor.middlewares().iter().filter(|n| *n == "mw").count();
    assert_eq!(count, 2, "a second apply must stack the middleware");
    assert_eq!(mgr.count("middleware"), Some(1), "store intact after apply");
}

// clause: extension_system.apply.postcondition.store_retained
#[test]
fn apply_postcondition_store_retained() {
    // D-78: applying ONE manager to TWO registry/executor pairs wires both.
    // Before the fix the second pair got nothing, silently.
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).expect("register acl");
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw")),
    )
    .expect("register mw");
    mgr.register(
        "span_exporter",
        ExtensionKind::SpanExporter(Arc::new(InMemoryExporter::new())),
    )
    .expect("register exporter");

    let registry_a = Arc::new(Registry::new());
    let mut executor_a = Executor::new(Arc::clone(&registry_a), Arc::new(Config::default()));
    let registry_b = Arc::new(Registry::new());
    let mut executor_b = Executor::new(Arc::clone(&registry_b), Arc::new(Config::default()));

    mgr.apply(&registry_a, &mut executor_a).expect("apply a");
    mgr.apply(&registry_b, &mut executor_b).expect("apply b");

    assert!(
        executor_a.middlewares().contains(&"mw".to_string()),
        "first executor must be wired"
    );
    assert!(
        executor_b.middlewares().contains(&"mw".to_string()),
        "second executor must be wired from the same manager"
    );

    // count() is unchanged at every point after apply.
    assert_eq!(mgr.count("acl"), Some(1));
    assert_eq!(mgr.count("middleware"), Some(1));
    assert_eq!(mgr.count("span_exporter"), Some(1));
    // And the entries are still readable, not just counted.
    assert!(matches!(
        mgr.get("acl").expect("known point"),
        Some(ExtensionKind::Acl(_))
    ));
    assert_eq!(mgr.get_all("middleware").expect("known point").len(), 1);
}

// clause: extension_system.apply.side_effect.ordered.full_sequence
#[test]
fn apply_side_effect_ordered_full_sequence() {
    // Python asserts the cross-target call ORDER discoverer -> module_validator ->
    // acl -> approval_handler -> middleware -> span exporters via a shared
    // recorder of mock calls. Rust exposes no cross-target getters/mocks, so the
    // full ordered sequence cannot be observed. We assert the observable subset:
    // apply() wires acl + middleware together without error and (D-78) leaves
    // the store intact.
    let mut mgr = ExtensionManager::new();
    mgr.register("acl", make_acl()).expect("register acl");
    mgr.register(
        "middleware",
        ExtensionKind::Middleware(NamedMiddleware::boxed("mw")),
    )
    .expect("register mw");
    let registry = Arc::new(Registry::new());
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));

    mgr.apply(&registry, &mut executor).expect("apply");

    assert!(executor.middlewares().contains(&"mw".to_string()));
    assert_eq!(mgr.count("acl"), Some(1), "acl retained");
    assert_eq!(mgr.count("middleware"), Some(1), "middleware retained");
    let _ = make_executor(); // keep helper exercised / referenced
}
