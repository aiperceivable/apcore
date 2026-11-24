//! A-D-13: `system.usage.module` MUST zero-pad `hourly_distribution` to
//! exactly 24 entries, filling gaps with zeros (spec MUST; mirrors
//! apcore-python `_pad_hourly_distribution`).

use std::collections::HashMap;
use std::sync::Arc;

use apcore::context::{Context, Identity};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{UsageCollector, UsageModule};
use chrono::{Duration, Utc};
use serde_json::json;

fn dummy_ctx() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::new(Identity::new(
        "@test".to_string(),
        "test".to_string(),
        vec![],
        HashMap::default(),
    ))
}

fn register_dummy(registry: &Arc<Registry>, id: &str) {
    struct DummyModule;
    #[async_trait::async_trait]
    impl Module for DummyModule {
        fn description(&self) -> &'static str {
            "dummy"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }
        fn output_schema(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, apcore::errors::ModuleError> {
            Ok(json!({}))
        }
    }

    let descriptor = ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: json!({}),
        output_schema: json!({}),
        version: "1.0.0".to_string(),
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
        .register_internal(id, Box::new(DummyModule), descriptor)
        .expect("register_internal should succeed");
}

#[tokio::test]
async fn usage_module_pads_hourly_distribution_to_24_entries() {
    let registry = Arc::new(Registry::new());
    register_dummy(&registry, "demo.module");

    let collector = UsageCollector::new();
    // Record sparse data: only two distinct hours within the last 24h.
    let now = Utc::now();
    collector.record_at("demo.module", Some("@a"), 10.0, true, now);
    collector.record_at(
        "demo.module",
        Some("@a"),
        20.0,
        false,
        now - Duration::hours(3),
    );

    let module = UsageModule::new(Arc::clone(&registry), collector);
    let result = module
        .execute(json!({"module_id": "demo.module"}), &dummy_ctx())
        .await
        .expect("usage.module should succeed");

    let hourly = result["hourly_distribution"]
        .as_array()
        .expect("hourly_distribution must be an array");
    assert_eq!(
        hourly.len(),
        24,
        "hourly_distribution must be padded to 24 entries, got {}",
        hourly.len()
    );

    // The total recorded calls across the 24 buckets must equal what we wrote.
    let total_calls: u64 = hourly
        .iter()
        .map(|h| h["call_count"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(total_calls, 2, "padding must preserve recorded call counts");

    // PROTOCOL_SPEC 6.7.1.2: the key is the collector's own bucket key,
    // YYYY-MM-DDTHH. This assertion did not exist, which is why the module
    // layer could reformat it to `%Y-%m-%dT%H:00:00Z` -- a spelling neither
    // apcore-python nor apcore-typescript emits -- with every test green.
    for entry in hourly {
        let hour = entry["hour"].as_str().expect("hour must be a string");
        assert_eq!(
            hour.len(),
            13,
            "hour key must be YYYY-MM-DDTHH (13 chars), got {hour:?}"
        );
        assert!(
            !hour.ends_with('Z') && !hour.contains(":00:00"),
            "hour key must not be reformatted with a time suffix, got {hour:?}"
        );
        let (date, hh) = hour.split_at(10);
        assert!(
            date.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }),
            "hour key date part malformed: {hour:?}"
        );
        assert!(
            hh.starts_with('T') && hh[1..].chars().all(|c| c.is_ascii_digit()),
            "hour key hour part malformed: {hour:?}"
        );
    }
}

#[tokio::test]
async fn usage_module_pads_when_no_data() {
    let registry = Arc::new(Registry::new());
    register_dummy(&registry, "idle.module");

    let collector = UsageCollector::new();
    let module = UsageModule::new(Arc::clone(&registry), collector);
    let result = module
        .execute(json!({"module_id": "idle.module"}), &dummy_ctx())
        .await
        .expect("usage.module should succeed");

    let hourly = result["hourly_distribution"]
        .as_array()
        .expect("hourly_distribution must be an array");
    assert_eq!(
        hourly.len(),
        24,
        "hourly_distribution must be 24 entries even with no data"
    );
    for entry in hourly {
        assert_eq!(entry["call_count"].as_u64(), Some(0));
        assert_eq!(entry["error_count"].as_u64(), Some(0));
    }
}
