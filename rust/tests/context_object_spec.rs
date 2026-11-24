//! Spec-traced contract tests for the Context Object feature (Rust SDK).
//!
//! Source spec: apcore/docs/features/context-object.md
//! Contract under test: `ContextKey<T>` (the only `## Contract:` block in the spec).
//!
//! Mirrors the canonical Python suite
//! `apcore-python/tests/test_context_object_spec.py` clause-for-clause. Each test
//! carries the verbatim clause id in a leading `// clause:` comment so
//! cross-language diffs line up.
//!
//! Real Rust API:
//! - `apcore::context::Context` (constructed via `Context::anonymous()`),
//!   exposing `data: Arc<RwLock<HashMap<String, Value>>>` via `.read()` / `.write()`.
//! - `apcore::context_key::ContextKey<T>` with `new`, `scoped`, `get`, `set`,
//!   `exists`, `delete`, and a public `name: Cow<'static, str>`.
//!
//! Contract declares NO errors and all methods are synchronous, so there are no
//! `Err`/code assertions to mirror; "no-raise" maps to "no panic" in Rust.
//!
//! TESTS ONLY — no production source is modified here.

use apcore::context::Context;
use apcore::context_key::ContextKey;
use serde_json::{json, Value};
use std::sync::Arc;

/// Create a minimal Context exposing a `data` map.
fn make_ctx() -> Context<Value> {
    Context::anonymous()
}

// ---------------------------------------------------------------------------
// Inputs — exercise declared parameter rules. The contract declares NO
// `reject_with` rules and NO errors, so "invalid"-style inputs must NOT panic;
// instead we assert the declared graceful fallback behavior.
// ---------------------------------------------------------------------------

// clause: context_object.get.input.default.absent_key_returns_default_not_raise
#[test]
fn context_object_get_input_default_absent_key_returns_default_not_raise() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.absent");
    let ctx = make_ctx();
    // Rust `get` has no `default` parameter; a missing key returns `None`
    // (the idiomatic Rust equivalent of Python's default-or-None fallback) and
    // never panics. The caller supplies a default via `Option::unwrap_or`.
    assert_eq!(key.get(&ctx), None);
    assert_eq!(key.get(&ctx).unwrap_or(7), 7);
}

// clause: context_object.delete.input.absent.delete_absent_is_noop_no_raise
#[test]
fn context_object_delete_input_absent_delete_absent_is_noop_no_raise() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.never_set");
    let ctx = make_ctx();
    // Spec Errors: "delete is a no-op on an absent key." Must not panic.
    key.delete(&ctx);
    assert!(!key.exists(&ctx));
}

// clause: context_object.scoped.input.suffix.appends_dotted_segment
#[test]
fn context_object_scoped_input_suffix_appends_dotted_segment() {
    let base: ContextKey<i64> = ContextKey::new("ext.spec.retry");
    let scoped = base.scoped("mod.a");
    // Spec Returns: scoped(suffix) -> a new key named "{name}.{suffix}".
    assert_eq!(scoped.name.as_ref(), "ext.spec.retry.mod.a");
    // The receiver is untouched (a new key was allocated).
    assert_eq!(base.name.as_ref(), "ext.spec.retry");
}

// ---------------------------------------------------------------------------
// Errors — the contract declares NONE. We assert the absence of panics across
// the full surface so a future regression that adds a panic is caught.
// ---------------------------------------------------------------------------

// clause: context_object.contextkey.error.none.no_method_raises
#[test]
fn context_object_contextkey_error_none_no_method_raises() {
    let key: ContextKey<String> = ContextKey::new("ext.spec.noerr");
    let ctx = make_ctx();
    // set / get / exists / delete / scoped must all complete without panicking.
    key.set(&ctx, "v".to_string());
    assert_eq!(key.get(&ctx), Some("v".to_string()));
    assert!(key.exists(&ctx));
    assert_eq!(key.scoped("s").name.as_ref(), "ext.spec.noerr.s");
    key.delete(&ctx);
    assert!(!key.exists(&ctx));
    // Second delete (now-absent) still must not panic.
    key.delete(&ctx);
}

// ---------------------------------------------------------------------------
// Returns — assert the exact declared return shapes.
// ---------------------------------------------------------------------------

// clause: context_object.set.returns.none.set_yields_no_value
#[test]
fn context_object_set_returns_none_set_yields_no_value() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.ret_set");
    let ctx = make_ctx();
    // Rust `set` returns `()` — the unit-type equivalent of Python `None`.
    let ret: () = key.set(&ctx, 1);
    assert_eq!(ret, ());
    // Observable effect confirms the call did its work.
    assert_eq!(key.get(&ctx), Some(1));
}

// clause: context_object.get.returns.value.present_returns_stored
#[test]
fn context_object_get_returns_value_present_returns_stored() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.ret_get");
    let ctx = make_ctx();
    key.set(&ctx, 123);
    assert_eq!(key.get(&ctx), Some(123));
}

// clause: context_object.get.returns.default.absent_returns_default
#[test]
fn context_object_get_returns_default_absent_returns_default() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.ret_get_def");
    let ctx = make_ctx();
    // Rust returns `None` when absent; the caller narrows via `unwrap_or`.
    assert_eq!(key.get(&ctx), None);
    assert_eq!(key.get(&ctx).unwrap_or(0), 0);
}

// clause: context_object.exists.returns.bool.true_iff_present
#[test]
fn context_object_exists_returns_bool_true_iff_present() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.ret_exists");
    let ctx = make_ctx();
    assert!(!key.exists(&ctx));
    key.set(&ctx, 9);
    assert!(key.exists(&ctx));
}

// clause: context_object.delete.returns.none.removes_name_from_data
#[test]
fn context_object_delete_returns_none_removes_name_from_data() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.ret_delete");
    let ctx = make_ctx();
    key.set(&ctx, 5);
    let ret: () = key.delete(&ctx);
    assert_eq!(ret, ());
    assert!(!ctx.data.read().contains_key("ext.spec.ret_delete"));
}

// clause: context_object.scoped.returns.key.new_contextkey_named_name_dot_suffix
#[test]
fn context_object_scoped_returns_key_new_contextkey_named_name_dot_suffix() {
    let base: ContextKey<i64> = ContextKey::new("ext.spec.ret_scoped");
    let child: ContextKey<i64> = base.scoped("x");
    // Type is `ContextKey<i64>` (statically guaranteed); name is "{name}.{suffix}".
    assert_eq!(child.name.as_ref(), "ext.spec.ret_scoped.x");
}

// ---------------------------------------------------------------------------
// Properties.
// ---------------------------------------------------------------------------

// clause: context_object.contextkey.property.async.all_methods_synchronous
#[test]
fn context_object_contextkey_property_async_all_methods_synchronous() {
    // Spec Properties: "async: false — all methods are synchronous."
    // In Rust this is a compile-time fact: the methods return plain values, not
    // futures, so they can be called and used directly inside a non-async fn
    // without `.await`. Reaching these assertions proves synchrony.
    let key: ContextKey<i64> = ContextKey::new("ext.spec.async");
    let ctx = make_ctx();
    key.set(&ctx, 1);
    let got: Option<i64> = key.get(&ctx);
    assert_eq!(got, Some(1));
    let present: bool = key.exists(&ctx);
    assert!(present);
    let child: ContextKey<i64> = key.scoped("s");
    assert_eq!(child.name.as_ref(), "ext.spec.async.s");
}

// clause: context_object.contextkey.property.thread_safe.concurrent_distinct_keys
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn context_object_contextkey_property_thread_safe_concurrent_distinct_keys() {
    // Spec Properties: "the Rust implementation guards context.data with a
    // read/write lock." Spawn >=8 concurrent writers on distinct keys, join all,
    // and assert no panic + consistent final state.
    let ctx = Arc::new(make_ctx());
    let n: i64 = 16;

    let mut handles = Vec::new();
    for i in 0..n {
        let ctx = Arc::clone(&ctx);
        handles.push(tokio::spawn(async move {
            let key: ContextKey<i64> =
                ContextKey::new("ext.spec.concurrent").scoped(&i.to_string());
            key.set(&ctx, i);
            key.get(&ctx).unwrap_or(-1)
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        // `.await` resolves (no panic propagated from the spawned task).
        results.push(h.await.expect("spawned task panicked"));
    }
    results.sort_unstable();
    assert_eq!(results, (0..n).collect::<Vec<_>>());

    // Every distinct key landed its own value in the shared map.
    for i in 0..n {
        let key: ContextKey<i64> = ContextKey::new("ext.spec.concurrent").scoped(&i.to_string());
        assert_eq!(key.get(&ctx), Some(i));
    }
}

// clause: context_object.set.property.idempotent.repeat_same_value_same_state
#[test]
fn context_object_set_property_idempotent_repeat_same_value_same_state() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.idem_set");
    let ctx = make_ctx();
    key.set(&ctx, 42);
    let first: std::collections::HashMap<String, Value> = ctx.data.read().clone();
    key.set(&ctx, 42);
    let second: std::collections::HashMap<String, Value> = ctx.data.read().clone();
    assert_eq!(first, second);
    assert_eq!(key.get(&ctx), Some(42));
}

// clause: context_object.delete.property.idempotent.repeat_delete_same_state
#[test]
fn context_object_delete_property_idempotent_repeat_delete_same_state() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.idem_delete");
    let ctx = make_ctx();
    key.set(&ctx, 1);
    key.delete(&ctx);
    let after_first: std::collections::HashMap<String, Value> = ctx.data.read().clone();
    key.delete(&ctx);
    let after_second: std::collections::HashMap<String, Value> = ctx.data.read().clone();
    assert_eq!(after_first, after_second);
    assert!(!key.exists(&ctx));
}

// clause: context_object.exists.property.idempotent.repeat_query_same_state
#[test]
fn context_object_exists_property_idempotent_repeat_query_same_state() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.idem_query");
    let ctx = make_ctx();
    key.set(&ctx, 5);
    let snapshot: std::collections::HashMap<String, Value> = ctx.data.read().clone();
    assert!(key.exists(&ctx));
    assert!(key.exists(&ctx));
    assert_eq!(key.get(&ctx), Some(5));
    assert_eq!(key.get(&ctx), Some(5));
    // Repeated queries observed no state change.
    assert_eq!(*ctx.data.read(), snapshot);
}

// clause: context_object.get.property.pure.read_only_no_self_mutation
#[test]
fn context_object_get_property_pure_read_only_no_self_mutation() {
    let key: ContextKey<i64> = ContextKey::new("ext.spec.pure_get");
    let ctx = make_ctx();
    key.set(&ctx, 3);
    let snapshot: std::collections::HashMap<String, Value> = ctx.data.read().clone();
    let _ = key.get(&ctx);
    let _ = key.get(&ctx).unwrap_or(99);
    // Spec Properties: "get and exists are side-effect-free (read-only)."
    assert_eq!(*ctx.data.read(), snapshot);
}

// clause: context_object.scoped.property.pure.allocates_new_key_no_mutation
#[test]
fn context_object_scoped_property_pure_allocates_new_key_no_mutation() {
    let base: ContextKey<i64> = ContextKey::new("ext.spec.pure_scoped");
    let base_name_before = base.name.to_string();
    let child = base.scoped("leaf");
    // Spec Properties: "scoped is pure and allocates a new key" / "never mutates
    // the receiver."
    assert_eq!(base.name.as_ref(), base_name_before);
    assert_eq!(child.name.as_ref(), "ext.spec.pure_scoped.leaf");
}

// clause: context_object.contextkey.property.immutable_key.name_is_readonly
#[test]
fn context_object_contextkey_property_immutable_key_name_is_readonly() {
    // Spec: "A key is immutable — ... Rust a value type." There is no runtime
    // mutation error in Rust (the analogue of Python's frozen-dataclass
    // AttributeError is compile-time `&self`-only methods). We assert the
    // value-type immutability semantics observable at runtime: every operation
    // takes `&self` and never mutates `name`; `scoped` produces a distinct key
    // while the receiver's `name` is unchanged.
    let key: ContextKey<i64> = ContextKey::new("ext.spec.frozen");
    let ctx = make_ctx();
    let name_before = key.name.to_string();
    key.set(&ctx, 1);
    let _ = key.get(&ctx);
    let _ = key.exists(&ctx);
    let child = key.scoped("changed");
    key.delete(&ctx);
    // The receiver's name survived every method call untouched.
    assert_eq!(key.name.as_ref(), name_before);
    assert_eq!(child.name.as_ref(), "ext.spec.frozen.changed");
}

// ---------------------------------------------------------------------------
// Side Effects — set / delete mutate context.data in place; observe ordering
// via the public data map.
// ---------------------------------------------------------------------------

// clause: context_object.contextkey.side_effect.1.set_then_delete_ordered
#[test]
fn context_object_contextkey_side_effect_1_set_then_delete_ordered() {
    let key: ContextKey<String> = ContextKey::new("ext.spec.effect");
    let ctx = make_ctx();
    let mut observed: Vec<bool> = Vec::new();
    observed.push(key.exists(&ctx)); // absent before set
    key.set(&ctx, "v".to_string());
    observed.push(key.exists(&ctx)); // present after set
    key.delete(&ctx);
    observed.push(key.exists(&ctx)); // absent after delete
    assert_eq!(observed, vec![false, true, false]);
}

// ---------------------------------------------------------------------------
// Namespace Convention (Normative) — the contract's MUST rules. Identifier
// strings round-trip verbatim into context.data (one shared namespace with raw
// string keys), so a raw read sees what a ContextKey wrote and vice versa.
// ---------------------------------------------------------------------------

// clause: context_object.contextkey.namespace.ext.shared_with_raw_string_keys
#[test]
fn context_object_contextkey_namespace_ext_shared_with_raw_string_keys() {
    let key: ContextKey<i64> = ContextKey::new("ext.my_company.retry.count");
    let ctx = make_ctx();
    key.set(&ctx, 4);
    // Two views of one map: raw access sees the ContextKey-written value.
    assert_eq!(
        ctx.data.read().get("ext.my_company.retry.count"),
        Some(&json!(4))
    );
    // And a raw write is visible through the typed key.
    ctx.data
        .write()
        .insert("ext.my_company.retry.count".to_string(), json!(9));
    assert_eq!(key.get(&ctx), Some(9));
}

// clause: context_object.contextkey.namespace.apcore_prefix.collides_with_raw
#[test]
fn context_object_contextkey_namespace_apcore_prefix_collides_with_raw() {
    // Spec: ContextKey("_apcore.foo") and context.data["_apcore.foo"] collide.
    let key: ContextKey<i64> = ContextKey::new("_apcore.foo");
    let ctx = make_ctx();
    ctx.data.write().insert("_apcore.foo".to_string(), json!(1));
    assert!(key.exists(&ctx));
    assert_eq!(key.get(&ctx), Some(1));
}
