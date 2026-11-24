// Tests for ContextKey typed accessor.

use apcore::context::Context;
use apcore::context_key::ContextKey;
use serde_json::json;

fn make_ctx() -> Context<serde_json::Value> {
    Context::anonymous()
}

// AC-001: get() returns typed value from context.data
#[test]
fn test_get_returns_typed_value() {
    let key: ContextKey<i64> = ContextKey::new("test.counter");
    let ctx = make_ctx();
    key.set(&ctx, 42);
    assert_eq!(key.get(&ctx), Some(42));
}

// AC-016: get() with absent key returns None
#[test]
fn test_get_absent_returns_none() {
    let key: ContextKey<i64> = ContextKey::new("test.absent");
    let ctx = make_ctx();
    assert_eq!(key.get(&ctx), None);
}

// AC-001: set() writes value to context.data
#[test]
fn test_set_writes_to_data() {
    let key: ContextKey<String> = ContextKey::new("test.name");
    let ctx = make_ctx();
    key.set(&ctx, "hello".to_string());
    let map = ctx.data.read();
    assert_eq!(map.get("test.name"), Some(&json!("hello")));
}

// delete() removes key from context.data
#[test]
fn test_delete_removes_key() {
    let key: ContextKey<i64> = ContextKey::new("test.temp");
    let ctx = make_ctx();
    key.set(&ctx, 10);
    key.delete(&ctx);
    let map = ctx.data.read();
    assert!(!map.contains_key("test.temp"));
}

// AC-017: delete() on absent key is no-op
#[test]
fn test_delete_absent_is_noop() {
    let key: ContextKey<i64> = ContextKey::new("test.absent");
    let ctx = make_ctx();
    key.delete(&ctx); // Should not panic
}

// AC-018: exists() returns false for absent, true for present
#[test]
fn test_exists_false_when_absent() {
    let key: ContextKey<i64> = ContextKey::new("test.flag");
    let ctx = make_ctx();
    assert!(!key.exists(&ctx));
}

#[test]
fn test_exists_true_when_present() {
    let key: ContextKey<i64> = ContextKey::new("test.flag");
    let ctx = make_ctx();
    key.set(&ctx, 1);
    assert!(key.exists(&ctx));
}

// AC-002: scoped(suffix) creates sub-key
#[test]
fn test_scoped_creates_subkey() {
    let base: ContextKey<i64> = ContextKey::new("_apcore.mw.retry.count");
    let scoped = base.scoped("mod1");
    assert_eq!(scoped.name.as_ref(), "_apcore.mw.retry.count.mod1");
}

// Scoped key is independent from base key
#[test]
fn test_scoped_key_is_independent() {
    let base: ContextKey<i64> = ContextKey::new("base");
    let scoped = base.scoped("child");
    let ctx = make_ctx();
    base.set(&ctx, 1);
    scoped.set(&ctx, 2);
    assert_eq!(base.get(&ctx), Some(1));
    assert_eq!(scoped.get(&ctx), Some(2));
}
