//! `Contract: APCore.remove` removes by IDENTITY, and a name cannot express it.
//!
//! apcore-python and apcore-typescript take the middleware object back and
//! compare it with `is` / `===`. Rust cannot: `use_middleware` consumes the
//! `Box`, so the caller has no object left to hand over. The published reason
//! was different and wrong — a doc comment on `APCore::remove_middleware` said
//! trait objects do not support identity comparison, but `Arc::ptr_eq` has
//! ignored vtable metadata since Rust 1.76, below this crate's MSRV. The reason
//! determines the fix, so it matters which one is true.
//!
//! What makes it reachable rather than theoretical: duplicate registration only
//! WARNS. `add` always succeeds, so two instances answering the same `name()`
//! coexist, and `remove(name)` drops whichever comes first in pipeline order.
//!
//! Sync finding A-C-001.

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::middleware::base::Middleware;
use apcore::middleware::manager::MiddlewareManager;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Two of these coexist under one name; `fired` tells them apart.
#[derive(Debug)]
struct Tagged {
    fired: Arc<AtomicBool>,
}

#[async_trait]
impl Middleware for Tagged {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "audit"
    }
    async fn before(
        &self,
        _: &str,
        _: Value,
        _: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.fired.store(true, Ordering::SeqCst);
        Ok(None)
    }
    async fn after(
        &self,
        _: &str,
        _: Value,
        _: Value,
        _: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
    async fn on_error(
        &self,
        _: &str,
        _: Value,
        _: &ModuleError,
        _: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
}

fn tagged() -> (Box<Tagged>, Arc<AtomicBool>) {
    let fired = Arc::new(AtomicBool::new(false));
    (
        Box::new(Tagged {
            fired: Arc::clone(&fired),
        }),
        fired,
    )
}

#[tokio::test]
async fn remove_handle_drops_the_registration_it_names_not_the_first_by_name() {
    let mgr = MiddlewareManager::new();
    let (first, first_fired) = tagged();
    let (second, second_fired) = tagged();

    let _first_handle = mgr.add(first).expect("first registers");
    let second_handle = mgr.add(second).expect("duplicate name only warns");
    assert_eq!(
        mgr.snapshot(),
        vec!["audit".to_string(), "audit".to_string()],
        "duplicate registration must succeed — otherwise this whole case is unreachable"
    );

    assert!(mgr.remove_handle(second_handle), "the handle resolves");

    let ctx = Context::anonymous();
    mgr.execute_before("executor.x", Value::Null, &ctx)
        .await
        .expect("before chain runs");

    assert!(
        first_fired.load(Ordering::SeqCst),
        "the middleware the caller did NOT name must still be in the chain"
    );
    assert!(
        !second_fired.load(Ordering::SeqCst),
        "the middleware named by the handle must be gone — this is the assertion \
         `remove(name)` cannot make, since it drops the first match in pipeline order"
    );
}

#[tokio::test]
async fn remove_by_name_drops_the_first_match_which_may_not_be_the_one_meant() {
    // Not a bug being pinned as correct — the documented behaviour of the
    // name-based form, kept here so the contrast with `remove_handle` is
    // explicit and so a change to either is a deliberate one.
    let mgr = MiddlewareManager::new();
    let (first, first_fired) = tagged();
    let (second, second_fired) = tagged();
    mgr.add(first).expect("first registers");
    mgr.add(second).expect("second registers");

    assert!(mgr.remove("audit"));

    let ctx = Context::anonymous();
    mgr.execute_before("executor.x", Value::Null, &ctx)
        .await
        .expect("before chain runs");

    assert!(!first_fired.load(Ordering::SeqCst), "the FIRST one went");
    assert!(second_fired.load(Ordering::SeqCst), "the second one stayed");
}

#[test]
fn a_stale_handle_is_false_not_a_panic() {
    let mgr = MiddlewareManager::new();
    let (mw, _) = tagged();
    let handle = mgr.add(mw).expect("registers");

    assert!(mgr.remove_handle(handle));
    assert!(
        !mgr.remove_handle(handle),
        "removing twice returns false, matching the idempotence the contract \
         declares for `remove`"
    );
}

#[test]
fn handles_track_priority_insertion_order() {
    // `add` inserts by priority, not at the end. The handle vector is parallel
    // to the middleware vector, so it has to be inserted at the same position —
    // otherwise a high-priority registration silently shifts every existing
    // handle onto the wrong middleware.
    #[derive(Debug)]
    struct Prioritised(u16, &'static str);

    #[async_trait]
    impl Middleware for Prioritised {
        fn name(&self) -> &str {
            self.1
        }
        fn priority(&self) -> u16 {
            self.0
        }
        async fn before(
            &self,
            _: &str,
            _: Value,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
        async fn after(
            &self,
            _: &str,
            _: Value,
            _: Value,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
        async fn on_error(
            &self,
            _: &str,
            _: Value,
            _: &ModuleError,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }

    let mgr = MiddlewareManager::new();
    let low = mgr.add(Box::new(Prioritised(10, "low"))).expect("low");
    let high = mgr.add(Box::new(Prioritised(900, "high"))).expect("high");
    assert_eq!(
        mgr.snapshot(),
        vec!["high".to_string(), "low".to_string()],
        "the higher priority sorts to the front, ahead of the earlier registration"
    );

    assert!(mgr.remove_handle(low), "the low-priority handle resolves");
    assert_eq!(
        mgr.snapshot(),
        vec!["high".to_string()],
        "and removes the low-priority one — not whatever sits at its old index"
    );
    assert!(mgr.remove_handle(high));
    assert!(mgr.snapshot().is_empty());
}
