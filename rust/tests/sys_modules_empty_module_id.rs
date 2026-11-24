//! [sys-empty-moduleid] The single-module sys readers (health.module,
//! manifest.module, usage.module) MUST reject an empty `module_id` with
//! InvalidInput (GENERAL_INVALID_INPUT), not ModuleNotFound. Mirrors
//! apcore-python / apcore-typescript.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::errors::ErrorCode;
use apcore::module::Module;
use apcore::registry::registry::Registry;
use apcore::{
    ErrorHistory, HealthModule, ManifestModule, MetricsCollector, UsageCollector, UsageModule,
};
use serde_json::json;
use tokio::sync::Mutex;

fn dummy_ctx() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::new(Identity::new(
        "@test".to_string(),
        "test".to_string(),
        vec![],
        HashMap::default(),
    ))
}

#[tokio::test]
async fn health_module_rejects_empty_module_id_with_invalid_input() {
    let registry = Arc::new(Registry::new());
    let module = HealthModule::new(
        Arc::clone(&registry),
        Some(MetricsCollector::new()),
        ErrorHistory::new(100),
    );
    let err = module
        .execute(json!({ "module_id": "" }), &dummy_ctx())
        .await
        .expect_err("empty module_id must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::GeneralInvalidInput,
        "empty module_id must be InvalidInput, not ModuleNotFound"
    );
}

#[tokio::test]
async fn manifest_module_rejects_empty_module_id_with_invalid_input() {
    let registry = Arc::new(Registry::new());
    let config = Arc::new(Mutex::new(Config::from_defaults()));
    let module = ManifestModule::new(Arc::clone(&registry), config);
    let err = module
        .execute(json!({ "module_id": "" }), &dummy_ctx())
        .await
        .expect_err("empty module_id must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

#[tokio::test]
async fn usage_module_rejects_empty_module_id_with_invalid_input() {
    let registry = Arc::new(Registry::new());
    let module = UsageModule::new(Arc::clone(&registry), UsageCollector::new());
    let err = module
        .execute(json!({ "module_id": "" }), &dummy_ctx())
        .await
        .expect_err("empty module_id must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}
