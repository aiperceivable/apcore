// Spec-traced contract tests for the apcore-rust identity-system feature.
//
// Source spec: apcore/docs/features/identity-system.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_identity_system_spec.py
//
// The spec's only `## Contract:` block describes
// `ContextFactory.create_context`: a factory extracts an `Identity` from a
// runtime request and returns a `Context` with an assigned trace id, defaulting
// to `@external` (no identity) when absent, generating a fresh trace id per
// call. In Python the contract is exercised through `Context.create` (the
// public, observable surface that the factory delegates to). The idiomatic Rust
// equivalent is `Context::create(identity, trace_parent, cancel_token, data,
// services, global_deadline)` (src/context.rs:509), plus the async
// `ContextFactory` trait whose `create_context(request)` method is the
// spec-named surface. Since spec v1.49.0 (D-76) that method takes the runtime
// request, typed by the implementation's `Request` associated type.
//
// Each test maps to exactly one clause. The verbatim cross-language clause id
// appears in a leading `// clause: <clause_id>` comment on the line above each
// test fn so a cross-language diff tool can line up Python / TypeScript / Rust
// rows by that exact string. The fn name is the clause id flattened to
// snake_case.
//
// TESTS ONLY — no production source is modified here.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::context::{Context, ContextFactory, Identity};
use apcore::errors::ModuleError;

// ---------------------------------------------------------------------------
// Helpers
//
// Rust's `Context::create` takes the canonical caller inputs positionally
// rather than a framework `request` object (the Python `_SpecFactory` pulls
// fields off a request; here those fields ARE the create() arguments). This
// helper mirrors the Python factory: build a top-level Context from the three
// contract-declared inputs (identity, caller_id, data), filling the remaining
// Rust-only create() parameters with their no-op defaults.
// ---------------------------------------------------------------------------

/// Build a Context the way the Python `_SpecFactory` does: delegate to the
/// public `Context::create` surface and then attach `caller_id` (which, per the
/// Rust contract, is not a `create` input — top-level contexts manage it
/// directly via the public field).
fn create_context(
    identity: Option<Identity>,
    caller_id: Option<String>,
    data: Option<HashMap<String, Value>>,
) -> Context<Value> {
    let mut ctx = Context::create(
        identity,
        None,        // trace_parent: not part of the create_context contract
        None,        // cancel_token: not part of the create_context contract
        data,        // data: optional initial context payload
        Value::Null, // services: DI container (Value services for these tests)
        None,        // global_deadline: not part of the create_context contract
    );
    if let Some(cid) = caller_id {
        ctx.caller_id = Some(cid);
    }
    ctx
}

/// A stand-in for a framework request object (a Django `HttpRequest`, an
/// Express `Request`, an Axum extractor). Opaque to apcore: only `SpecFactory`
/// knows how to read it.
struct FakeRequest {
    user_id: Option<String>,
    roles: Vec<String>,
}

/// Concrete `ContextFactory` implementation used to exercise the spec-named
/// async `create_context(request)` trait method (the Rust analog of Python's
/// `runtime_checkable` Protocol conformance).
struct SpecFactory;

#[async_trait]
impl ContextFactory for SpecFactory {
    type Request = FakeRequest;

    async fn create_context(&self, request: FakeRequest) -> Result<Context<Value>, ModuleError> {
        // An unauthenticated request is sanitized to the anonymous `@external`
        // identity rather than rejected — the contract's `### Errors` row.
        let identity = request
            .user_id
            .map(|id| Identity::new(id, "user".to_string(), request.roles, HashMap::new()));
        Ok(Context::create(
            identity,
            None,
            None,
            None,
            Value::Null,
            None,
        ))
    }
}

// ---------------------------------------------------------------------------
// Inputs — the contract declares NO reject rules: "invalid identity fields are
// sanitized, not rejected". So an absent/optional input must NOT error; instead
// we assert the declared graceful fallback behavior.
// ---------------------------------------------------------------------------

// clause: identity_system.create_context.input.identity.absent_defaults_to_external
#[test]
fn identity_system_create_context_input_identity_absent_defaults_to_external() {
    // Spec Inputs: identity is optional and "defaults to @external when absent".
    // The @external pattern (per the feature spec) is "identity is None".
    let ctx = create_context(None, None, None);
    assert!(ctx.identity.is_none());
}

// clause: identity_system.create_context.input.caller_id.optional_absent_is_none
#[test]
fn identity_system_create_context_input_caller_id_optional_absent_is_none() {
    // Spec Inputs: caller_id is optional for call-chain tracking; absent -> None.
    let ctx = create_context(None, None, None);
    assert!(ctx.caller_id.is_none());
    // When supplied, it is carried onto the produced Context.
    let ctx2 = create_context(None, Some("api.gateway".to_string()), None);
    assert_eq!(ctx2.caller_id.as_deref(), Some("api.gateway"));
}

// clause: identity_system.create_context.input.data.optional_absent_is_empty
#[test]
fn identity_system_create_context_input_data_optional_absent_is_empty() {
    // Spec Inputs: data is the optional initial context payload; absent -> {}.
    let ctx = create_context(None, None, None);
    assert!(ctx.data.read().is_empty());
    // When supplied, the payload is carried through verbatim.
    let mut data = HashMap::new();
    data.insert("k".to_string(), json!("v"));
    let ctx2 = create_context(None, None, Some(data));
    assert_eq!(ctx2.data.read().get("k"), Some(&json!("v")));
}

// clause: identity_system.create_context.input.identity.sanitized_not_rejected
#[test]
fn identity_system_create_context_input_identity_sanitized_not_rejected() {
    // Spec Inputs/Errors: "invalid identity fields are sanitized, not rejected".
    //
    // In Python this clause exercises `Identity(attrs="not-a-dict")` being
    // sanitized to `{}` by `__post_init__`. In Rust `Identity::attrs` is a
    // statically-typed `HashMap<String, Value>` (src/context.rs:80), so a
    // non-map value is a compile error, not a runtime input — there is no
    // "invalid attrs" surface to sanitize. We mirror the clause INTENT —
    // create_context must accept a well-formed Identity without erroring and
    // attach it unchanged — using an explicitly empty `attrs` map (the value
    // the Python sanitizer produces).
    let ident = Identity::new(
        "svc-1".to_string(),
        "service".to_string(),
        vec![],
        HashMap::new(),
    );
    assert!(ident.attrs().is_empty());
    let ctx = create_context(Some(ident.clone()), None, None);
    let attached = ctx.identity.as_ref().expect("identity attached");
    assert_eq!(attached, &ident);
    assert!(attached.attrs().is_empty());
}

// ---------------------------------------------------------------------------
// Errors — the contract declares NONE ("No errors raised"). `Context::create`
// returns a `Context` directly (infallible, no `Result`), so the Rust analog of
// "does not raise" is "every optional-input combination yields a usable
// Context". We assert that across the input surface so a future regression that
// adds a failure mode is caught.
// ---------------------------------------------------------------------------

// clause: identity_system.create_context.error.none.no_error_raised
#[test]
fn identity_system_create_context_error_none_no_error_raised() {
    // Every optional-input combination must complete and yield a usable Context.
    let _ = create_context(None, None, None);
    let _ = create_context(None, None, None);

    let mut attrs = HashMap::new();
    attrs.insert("d".to_string(), json!("eng"));
    let ident = Identity::new(
        "u-1".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        attrs,
    );
    let mut data = HashMap::new();
    data.insert("x".to_string(), json!(1));
    let ctx = create_context(
        Some(ident.clone()),
        Some("orchestrator.run".to_string()),
        Some(data),
    );
    // Proves we reached the return (the identity is attached), not an early
    // failure.
    assert_eq!(ctx.identity.as_ref(), Some(&ident));
}

// ---------------------------------------------------------------------------
// Returns — "Context with assigned trace ID and caller identity".
// ---------------------------------------------------------------------------

// clause: identity_system.create_context.returns.context.assigned_trace_id
#[test]
fn identity_system_create_context_returns_context_assigned_trace_id() {
    let ident = Identity::new(
        "admin@example.com".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        HashMap::new(),
    );
    let ctx = create_context(Some(ident.clone()), None, None);
    // A non-empty trace_id is assigned, and the caller identity is attached.
    assert!(!ctx.trace_id.is_empty());
    let attached = ctx.identity.as_ref().expect("identity attached");
    assert_eq!(attached.id(), "admin@example.com");
    assert_eq!(attached.roles(), &["admin".to_string()]);
}

// clause: identity_system.create_context.returns.context.identity_propagates_to_child
#[test]
fn identity_system_create_context_returns_context_identity_propagates_to_child() {
    let ident = Identity::new(
        "admin@example.com".to_string(),
        "user".to_string(),
        vec!["admin".to_string(), "operator".to_string()],
        HashMap::new(),
    );
    let ctx = create_context(Some(ident), None, None);
    // Spec requirement: Identity propagates to child contexts (by value in
    // Rust — `child()` clones identity and shares trace_id).
    let child = ctx.child("target.module");
    assert_eq!(child.identity, ctx.identity);
    assert_eq!(child.trace_id, ctx.trace_id);
}

// ---------------------------------------------------------------------------
// Properties — async: false, thread_safe: true, pure: false, idempotent: false.
// ---------------------------------------------------------------------------

// clause: identity_system.create_context.property.async.synchronous_not_awaitable
#[test]
fn identity_system_create_context_property_async_synchronous_not_awaitable() {
    // Spec Properties: async: false. The `Context::create` surface that the
    // factory delegates to is a plain synchronous fn — it returns a `Context`
    // directly (no future to await). A synchronous `#[test]` (no runtime)
    // suffices to construct and observe the result.
    let result = create_context(None, None, None);
    assert!(!result.trace_id.is_empty());
}

// clause: identity_system.create_context.property.thread_safe.concurrent_distinct_inputs
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identity_system_create_context_property_thread_safe_concurrent_distinct_inputs() {
    let n: usize = 16;
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        handles.push(tokio::spawn(async move {
            let ident = Identity::new(
                format!("user-{i}"),
                "user".to_string(),
                vec![format!("role-{i}")],
                HashMap::new(),
            );
            create_context(Some(ident), Some(format!("caller-{i}")), None)
        }));
    }

    let mut contexts = Vec::with_capacity(n);
    for h in handles {
        // No panic across any spawned task.
        contexts.push(h.await.expect("spawned task did not panic"));
    }

    // Each call produced its own consistent Context (no cross-task corruption).
    assert_eq!(contexts.len(), n);
    let mut trace_ids = std::collections::HashSet::new();
    for ctx in &contexts {
        let ident = ctx.identity.as_ref().expect("identity present");
        let id = ident.id().to_string();
        let suffix = id.strip_prefix("user-").expect("id has expected shape");
        assert_eq!(
            ctx.caller_id.as_deref(),
            Some(format!("caller-{suffix}").as_str())
        );
        trace_ids.insert(ctx.trace_id.clone());
    }
    // Spec Properties: a new trace ID per call -> all trace_ids are distinct.
    assert_eq!(trace_ids.len(), n);
}

// clause: identity_system.create_context.property.idempotent.distinct_trace_id_per_call
#[test]
fn identity_system_create_context_property_idempotent_distinct_trace_id_per_call() {
    let ident = Identity::new(
        "admin@example.com".to_string(),
        "user".to_string(),
        vec![],
        HashMap::new(),
    );
    // Spec Properties: idempotent: false — "generates a new trace ID on each
    // call". Two calls with identical input yield distinct trace IDs.
    let first = create_context(Some(ident.clone()), None, None);
    let second = create_context(Some(ident.clone()), None, None);
    assert_ne!(first.trace_id, second.trace_id);
    // Identity is still attached identically (only the trace ID differs).
    assert_eq!(first.identity.as_ref(), Some(&ident));
    assert_eq!(second.identity.as_ref(), Some(&ident));
}

// clause: identity_system.create_context.property.pure.not_pure_fresh_context_each_call
#[test]
fn identity_system_create_context_property_pure_not_pure_fresh_context_each_call() {
    let ident = Identity::new(
        "svc".to_string(),
        "service".to_string(),
        vec![],
        HashMap::new(),
    );
    // Spec Properties: pure: false — the call is observably non-deterministic
    // (a new trace ID per call) and yields a *fresh* Context with its own
    // shared-data allocation each time. Use independent data payloads so each
    // call's data map is its own object.
    let mut data_a = HashMap::new();
    data_a.insert("shared".to_string(), json!(1));
    let mut data_b = HashMap::new();
    data_b.insert("shared".to_string(), json!(1));
    let a = create_context(Some(ident.clone()), None, Some(data_a));
    let b = create_context(Some(ident.clone()), None, Some(data_b));
    // Distinct shared-data allocations (Arc identity differs) and distinct trace.
    assert!(!Arc::ptr_eq(&a.data, &b.data));
    assert_ne!(a.trace_id, b.trace_id);
    // The same input identity value is attached to both (only trace ID varies).
    assert_eq!(a.identity.as_ref(), Some(&ident));
    assert_eq!(b.identity.as_ref(), Some(&ident));
}

// ---------------------------------------------------------------------------
// Protocol conformance — Python's `ContextFactory` is a runtime_checkable
// Protocol asserting a type exposes `create_context`. Rust's `ContextFactory`
// is a compile-time async trait (src/context.rs:774); conformance is statically
// guaranteed by the `impl`. The runtime analog: a concrete impl is usable
// through `&dyn ContextFactory` and its spec-named `create_context` method
// `.await`-resolves to a usable Context.
// ---------------------------------------------------------------------------

// clause: identity_system.create_context.property.protocol.runtime_checkable_conformance
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_system_create_context_property_protocol_runtime_checkable_conformance() {
    let factory: &dyn ContextFactory<Request = FakeRequest> = &SpecFactory;
    let ident = Identity::new(
        "admin@example.com".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        HashMap::new(),
    );
    let ctx = factory
        .create_context(FakeRequest {
            user_id: Some("admin@example.com".to_string()),
            roles: vec!["admin".to_string()],
        })
        .await
        .expect("create_context resolves to a usable Context");
    assert!(!ctx.trace_id.is_empty());
    assert_eq!(ctx.identity.as_ref(), Some(&ident));
}

// clause: identity_system.create_context.input.request.extracts_identity
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_system_create_context_input_request_extracts_identity() {
    // D-76: `create_context` takes the runtime request, and the factory — not
    // the protocol — reads the identity out of it.
    let ctx = SpecFactory
        .create_context(FakeRequest {
            user_id: Some("u-7".to_string()),
            roles: vec!["viewer".to_string()],
        })
        .await
        .expect("a well-formed request yields a Context");
    let identity = ctx.identity.as_ref().expect("identity extracted");
    assert_eq!(identity.id(), "u-7");
    assert_eq!(identity.roles(), ["viewer".to_string()]);
}

// clause: identity_system.create_context.error.unauthenticated_request_is_external
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_system_create_context_error_unauthenticated_request_is_external() {
    // `### Errors`: a request that cannot be authenticated is sanitized to
    // `@external`, not rejected through the error channel.
    let ctx = SpecFactory
        .create_context(FakeRequest {
            user_id: None,
            roles: vec![],
        })
        .await
        .expect("an unauthenticated request still yields a Context");
    assert!(ctx.identity.is_none());
    assert!(!ctx.trace_id.is_empty());
}

// clause: identity_system.create_context.property.additive_members_have_defaults
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_system_create_context_property_additive_members_have_defaults() {
    // `create` / `create_child` are Rust-only additive members; the trait
    // supplies defaults so a host implementing the spec surface alone compiles.
    let parent = SpecFactory
        .create(None, Value::Null)
        .await
        .expect("default create");
    let child = SpecFactory
        .create_child(&parent, "executor.email.send")
        .await
        .expect("default create_child");
    assert_eq!(child.trace_id, parent.trace_id);
    assert_eq!(child.call_chain, vec!["executor.email.send".to_string()]);
}
