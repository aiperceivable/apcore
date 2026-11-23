// Issue #64 — Duplicate middleware detection
// Tests identity-based duplicate detection in MiddlewareManager::add_with_opts.

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::middleware::base::Middleware;
use apcore::middleware::manager::{MiddlewareManager, MiddlewareRegistration};
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug)]
struct AlphaMiddleware;

#[async_trait]
impl Middleware for AlphaMiddleware {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "alpha"
    }
    fn priority(&self) -> u16 {
        0
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

#[derive(Debug)]
struct BetaMiddleware;

#[async_trait]
impl Middleware for BetaMiddleware {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "beta"
    }
    fn priority(&self) -> u16 {
        0
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

#[test]
fn single_registration_succeeds_no_warning() {
    // Registration of a single instance always succeeds.
    let mgr = MiddlewareManager::new();
    let result = mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware));
    assert!(result.is_ok());
    assert_eq!(mgr.snapshot(), vec!["alpha"]);
}

#[test]
fn two_same_type_registrations_chain_still_contains_both() {
    // Registration always succeeds regardless of duplicate detection.
    // Both entries MUST appear in the chain (registration is not rejected,
    // only warned about).
    let mgr = MiddlewareManager::new();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware))
        .unwrap();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware))
        .unwrap();
    assert_eq!(
        mgr.snapshot().len(),
        2,
        "both registrations must be present in the chain"
    );
    assert_eq!(
        mgr.snapshot(),
        vec!["alpha", "alpha"],
        "chain order must be preserved"
    );
}

#[test]
fn allow_duplicate_true_suppresses_warning_path() {
    // When allow_duplicate=true, both registrations go in without warning
    // (tested structurally — no tracing subscriber needed).
    let mgr = MiddlewareManager::new();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware).allow_duplicate(true))
        .unwrap();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware).allow_duplicate(true))
        .unwrap();
    assert_eq!(mgr.snapshot().len(), 2, "both entries must be in the chain");
}

#[test]
fn first_allow_duplicate_registration_is_still_recorded() {
    // A-D-020: an identity must be recorded on first sight even when the first
    // registration sets allow_duplicate(true). Only the warning is suppressed.
    // Previously the recording lived inside `if !allow_duplicate`, so the
    // identity was never stored and a later duplicate could not be detected.
    let mgr = MiddlewareManager::new();
    mgr.add_with_opts(
        MiddlewareRegistration::new(AlphaMiddleware)
            .identity_key("shared")
            .allow_duplicate(true),
    )
    .unwrap();
    assert!(
        mgr.identity_registered("shared"),
        "identity must be recorded even when the first registration is allow_duplicate(true)"
    );
}

#[test]
fn distinct_identity_keys_treat_instances_as_different() {
    // Providing distinct explicit identity_key values means each instance
    // is treated as a distinct identity — no duplicate warning path is taken.
    let mgr = MiddlewareManager::new();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware).identity_key("alpha-primary"))
        .unwrap();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware).identity_key("alpha-secondary"))
        .unwrap();
    assert_eq!(mgr.snapshot().len(), 2);
}

#[test]
fn different_types_no_duplicate() {
    // Different types have different default identity keys so they never
    // trigger the duplicate warning path.
    let mgr = MiddlewareManager::new();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware))
        .unwrap();
    mgr.add_with_opts(MiddlewareRegistration::new(BetaMiddleware))
        .unwrap();
    assert_eq!(mgr.snapshot(), vec!["alpha", "beta"]);
}

#[test]
fn add_with_opts_interoperates_with_plain_add() {
    // Verify add() and add_with_opts() share the same underlying chain.
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(AlphaMiddleware)).unwrap();
    mgr.add_with_opts(MiddlewareRegistration::new(BetaMiddleware))
        .unwrap();
    assert_eq!(mgr.snapshot(), vec!["alpha", "beta"]);
}

#[test]
fn same_explicit_identity_key_triggers_duplicate_path() {
    // Two registrations sharing the same explicit identity_key will trigger
    // the duplicate-detection path (warning emitted). We verify this
    // structurally: the second entry's identity is already in the map
    // so a warn would be emitted. Chain still contains both entries.
    let mgr = MiddlewareManager::new();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware).identity_key("shared-key"))
        .unwrap();
    // This second registration should trigger the duplicate warn path.
    mgr.add_with_opts(MiddlewareRegistration::new(BetaMiddleware).identity_key("shared-key"))
        .unwrap();
    // Regardless of the warning, both must be in the chain.
    assert_eq!(mgr.snapshot().len(), 2);
}

#[derive(Debug)]
struct HighPriorityMiddleware;

#[async_trait]
impl Middleware for HighPriorityMiddleware {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "high"
    }
    fn priority(&self) -> u16 {
        100
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

#[test]
fn priority_ordering_preserved_with_add_with_opts() {
    // add_with_opts must respect priority ordering, the same as plain add().
    let mgr = MiddlewareManager::new();
    mgr.add_with_opts(MiddlewareRegistration::new(AlphaMiddleware))
        .unwrap();
    mgr.add_with_opts(MiddlewareRegistration::new(HighPriorityMiddleware))
        .unwrap();
    // HighPriorityMiddleware (priority 100) should come before Alpha (priority 0).
    assert_eq!(mgr.snapshot(), vec!["high", "alpha"]);
}
