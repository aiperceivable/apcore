//! Shared helper for reload tests.
//!
//! `system.control.reload_module` unregisters before re-discovering, so a
//! registry with no discoverer attached cannot restore the module and the
//! reload fails with RELOAD_FAILED — which is what apcore-python does too
//! (verified 2026-08-27: `ReloadFailedError`, module absent afterwards).
//!
//! Tests that use a reload as a vehicle for asserting something else
//! (previous_version capture, topological order, audit payloads) therefore
//! need a discoverer that puts the modules back. That is this helper.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::registry::registry::{DiscoveredModule, Discoverer, ModuleDescriptor, Registry};

/// Re-supplies a fixed set of module ids on `discover()`, standing in for a
/// filesystem discoverer whose files are untouched by the reload.
pub struct RestoringDiscoverer {
    pub ids: Vec<String>,
}

impl RestoringDiscoverer {
    /// Attach a discoverer restoring exactly the ids currently registered.
    pub fn attach_for(registry: &Arc<Registry>, ids: &[String]) {
        registry.set_discoverer(Box::new(RestoringDiscoverer { ids: ids.to_vec() }));
    }
}

#[async_trait::async_trait]
impl Discoverer for RestoringDiscoverer {
    async fn discover(&self, _roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
        Ok(self
            .ids
            .iter()
            .map(|id| DiscoveredModule {
                name: id.clone(),
                source: "test".to_string(),
                descriptor: ModuleDescriptor {
                    module_id: id.clone(),
                    name: None,
                    description: "restored".to_string(),
                    documentation: None,
                    input_schema: serde_json::json!({"type": "object"}),
                    output_schema: serde_json::json!({"type": "object"}),
                    version: "1.0.0".to_string(),
                    tags: vec![],
                    annotations: None,
                    examples: vec![],
                    metadata: HashMap::new(),
                    display: None,
                    sunset_date: None,
                    dependencies: vec![],
                    enabled: true,
                },
                module: Arc::new(RestoredModule),
            })
            .collect())
    }
}

struct RestoredModule;

#[async_trait::async_trait]
impl Module for RestoredModule {
    fn description(&self) -> &'static str {
        "restored"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn output_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(
        &self,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        Ok(serde_json::json!({}))
    }
}
