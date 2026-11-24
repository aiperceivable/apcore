// D-25 alignment: system.control.update_config MUST raise an error coded
// CONFIG_KEY_RESTRICTED when the caller targets a restricted key. The previous
// Rust impl folded this into ConfigInvalid/GeneralInvalidInput; Python and TS
// both expose a distinct CONFIG_KEY_RESTRICTED code so consumers can match on it.

use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::events::emitter::EventEmitter;
use apcore::module::Module;
use apcore::sys_modules::control::UpdateConfigModule;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

#[tokio::test]
async fn update_config_returns_config_key_restricted() {
    let cfg = Arc::new(TokioMutex::new(Config::default()));
    let emitter = Arc::new(EventEmitter::new());
    let module = UpdateConfigModule::new(cfg, emitter);

    let inputs = json!({
        // sys_modules.enabled is in RESTRICTED_KEYS — see sys_modules::mod.rs
        "key": "sys_modules.enabled",
        "value": false,
        "reason": "test",
    });
    let ctx = Context::<serde_json::Value>::new(Identity::new(
        "@test".to_string(),
        "user".to_string(),
        Vec::new(),
        HashMap::new(),
    ));
    let err = module
        .execute(inputs, &ctx)
        .await
        .expect_err("restricted key must error");

    let code_str = serde_json::to_value(err.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    assert_eq!(
        code_str, "CONFIG_KEY_RESTRICTED",
        "restricted-key error must use CONFIG_KEY_RESTRICTED code (got {code_str})"
    );
}
