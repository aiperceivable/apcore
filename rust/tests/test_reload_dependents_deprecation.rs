//! D-121 removal through the configured reload module's public call path.

use apcore::{APCore, APCoreOptions, AutoApproveHandler, Config, ErrorCode};
use serde_json::json;

#[tokio::test]
async fn removed_reload_dependents_is_rejected_by_the_module_input_schema() {
    let mut config = Config::in_memory();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let client = APCore::try_start(APCoreOptions {
        config: Some(config),
        approval_handler: Some(Box::new(AutoApproveHandler)),
        ..Default::default()
    })
    .unwrap();
    let module_id = "system.control.reload_module";
    let descriptor = client
        .registry()
        .get_definition(module_id)
        .unwrap()
        .unwrap();
    assert_eq!(descriptor.input_schema["additionalProperties"], false);
    assert!(descriptor.input_schema["properties"]
        .get("reload_dependents")
        .is_none());

    let error = client
        .call(
            module_id,
            json!({"path_filter": "demo.*", "reason": "test reload", "reload_dependents": true}),
            None,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SchemaValidationError);
}
