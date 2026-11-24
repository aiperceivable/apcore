//! [sys-manifest-desc] system.manifest.module MUST source `description` from
//! the registered descriptor (which YAML overrides may have set), not the live
//! module instance. Mirrors apcore-python / apcore-typescript.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{ManifestModule, DEFAULT_MODULE_VERSION};
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

struct DummyModule;

#[async_trait::async_trait]
impl Module for DummyModule {
    fn description(&self) -> &'static str {
        "instance-level description"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, apcore::errors::ModuleError> {
        Ok(json!({}))
    }
}

#[tokio::test]
async fn manifest_description_comes_from_descriptor_not_instance() {
    let registry = Arc::new(Registry::new());
    // Descriptor description deliberately differs from the instance's.
    let descriptor = ModuleDescriptor {
        module_id: "demo.mod".to_string(),
        name: None,
        description: "descriptor-level description".to_string(),
        documentation: None,
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    registry
        .register("demo.mod", Box::new(DummyModule), descriptor)
        .expect("register");

    let config = Arc::new(Mutex::new(Config::from_defaults()));
    let module = ManifestModule::new(Arc::clone(&registry), config);
    let out = module
        .execute(json!({ "module_id": "demo.mod" }), &dummy_ctx())
        .await
        .expect("manifest");

    assert_eq!(
        out.get("description").and_then(|v| v.as_str()),
        Some("descriptor-level description"),
        "manifest description must come from the descriptor, not the instance"
    );
}
