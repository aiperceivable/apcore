//! `period` is a filter, not an echo (PROTOCOL_SPEC 6.7.1.1).
//!
//! Both usage modules used to read `period`, echo it back, and compute every
//! statistic over the full retained history — `get_all_summaries()` for the
//! summary, and `get_module_summary` / `get_p99_latency_ms` /
//! `get_caller_breakdown` / `get_hourly_distribution` for the detail module,
//! none of which took a period. Silent by construction: the response names the
//! window it did not apply.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::context::{Context, Identity};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{UsageCollector, UsageModule, UsageSummaryModule};
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

/// One call inside the 24h window, one well outside it.
fn collector_with_split_history(module_id: &str) -> UsageCollector {
    let collector = UsageCollector::new();
    let now = Utc::now();
    collector.record_at(
        module_id,
        Some("caller-in"),
        10.0,
        true,
        now - Duration::hours(2),
    );
    collector.record_at(
        module_id,
        Some("caller-out"),
        500.0,
        false,
        now - Duration::days(6),
    );
    collector
}

#[tokio::test]
async fn summary_totals_are_filtered_by_period() {
    let collector = collector_with_split_history("math.add");
    let module = UsageSummaryModule::new(collector);

    let result = module
        .execute(json!({"period": "24h"}), &dummy_ctx())
        .await
        .expect("summary should succeed");

    assert_eq!(result["period"], "24h");
    assert_eq!(
        result["total_calls"].as_u64(),
        Some(1),
        "the 6-day-old record is outside a 24h window; full history would answer 2"
    );
    assert_eq!(result["total_errors"].as_u64(), Some(0));
}

#[tokio::test]
async fn summary_widens_with_the_period() {
    // The control case. Without it an implementation that returns nothing at
    // all passes the filtering test for the wrong reason.
    let collector = collector_with_split_history("math.add");
    let module = UsageSummaryModule::new(collector);

    let result = module
        .execute(json!({"period": "30d"}), &dummy_ctx())
        .await
        .expect("summary should succeed");

    assert_eq!(result["total_calls"].as_u64(), Some(2));
    assert_eq!(result["total_errors"].as_u64(), Some(1));
}

#[tokio::test]
async fn every_detail_statistic_is_filtered_by_period() {
    let registry = Arc::new(Registry::new());
    register_dummy(&registry, "math.add");
    let collector = collector_with_split_history("math.add");
    let module = UsageModule::new(Arc::clone(&registry), collector);

    let result = module
        .execute(
            json!({"module_id": "math.add", "period": "24h"}),
            &dummy_ctx(),
        )
        .await
        .expect("detail should succeed");

    assert_eq!(result["call_count"].as_u64(), Some(1));
    assert_eq!(result["error_count"].as_u64(), Some(0));
    assert_eq!(result["avg_latency_ms"].as_f64(), Some(10.0));
    assert_eq!(
        result["p99_latency_ms"].as_f64(),
        Some(10.0),
        "the out-of-window 500ms sample must not reach p99"
    );

    let callers = result["callers"].as_array().expect("callers array");
    assert_eq!(callers.len(), 1, "only the in-window caller may appear");
    assert_eq!(callers[0]["caller_id"], "caller-in");

    let hourly_total: u64 = result["hourly_distribution"]
        .as_array()
        .expect("hourly array")
        .iter()
        .filter_map(|h| h["call_count"].as_u64())
        .sum();
    assert_eq!(
        hourly_total, 1,
        "hourly buckets must be period-filtered too"
    );
}

#[tokio::test]
async fn detail_widens_with_the_period() {
    let registry = Arc::new(Registry::new());
    register_dummy(&registry, "math.add");
    let collector = collector_with_split_history("math.add");
    let module = UsageModule::new(Arc::clone(&registry), collector);

    let result = module
        .execute(
            json!({"module_id": "math.add", "period": "30d"}),
            &dummy_ctx(),
        )
        .await
        .expect("detail should succeed");

    assert_eq!(result["call_count"].as_u64(), Some(2));
    assert_eq!(result["error_count"].as_u64(), Some(1));
    assert_eq!(
        result["p99_latency_ms"].as_f64(),
        Some(500.0),
        "with the wider window the 500ms sample IS the p99"
    );
}

#[tokio::test]
async fn unattributed_caller_is_the_literal_unknown() {
    // PROTOCOL_SPEC 6.7.1.4 — not null, not omitted, and not the ACL token
    // `@external`.
    let registry = Arc::new(Registry::new());
    register_dummy(&registry, "math.add");
    let collector = UsageCollector::new();
    collector.record("math.add", None, 5.0, true);
    let module = UsageModule::new(Arc::clone(&registry), collector);

    let result = module
        .execute(
            json!({"module_id": "math.add", "period": "24h"}),
            &dummy_ctx(),
        )
        .await
        .expect("detail should succeed");

    let callers = result["callers"].as_array().expect("callers array");
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0]["caller_id"], "unknown");
}

#[tokio::test]
async fn output_schema_declares_a_field_contract() {
    // PROTOCOL_SPEC 6.7.1.6. Both modules returned a bare {"type":"object"},
    // which satisfies 6.7's "equivalent output schemas" only in the sense that
    // any two such declarations are equivalent to each other.
    let registry = Arc::new(Registry::new());
    let summary = UsageSummaryModule::new(UsageCollector::new());
    let detail = UsageModule::new(registry, UsageCollector::new());

    for (label, schema) in [
        ("summary", summary.output_schema()),
        ("detail", detail.output_schema()),
    ] {
        assert!(
            schema.get("properties").is_some(),
            "{label} output_schema must declare properties"
        );
        assert!(
            schema.get("required").is_some(),
            "{label} output_schema must declare required"
        );
    }

    let detail_schema =
        UsageModule::new(Arc::new(Registry::new()), UsageCollector::new()).output_schema();
    let hourly = &detail_schema["properties"]["hourly_distribution"];
    assert_eq!(hourly["minItems"].as_u64(), Some(24));
    assert_eq!(hourly["maxItems"].as_u64(), Some(24));
    assert_eq!(
        hourly["items"]["properties"]["hour"]["pattern"],
        "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}$"
    );
}

#[tokio::test]
async fn period_pattern_is_declared_in_the_input_schema() {
    // PROTOCOL_SPEC 6.7.1.1 — so a malformed value is rejected at input
    // validation with SCHEMA_VALIDATION_ERROR, uniformly across SDKs.
    let summary = UsageSummaryModule::new(UsageCollector::new());
    let detail = UsageModule::new(Arc::new(Registry::new()), UsageCollector::new());

    for (label, schema) in [
        ("summary", summary.input_schema()),
        ("detail", detail.input_schema()),
    ] {
        assert_eq!(
            schema["properties"]["period"]["pattern"], "^[1-9][0-9]*[hd]$",
            "{label} must declare the period grammar"
        );
    }
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-016: a malformed `period` must fail, not silently widen
// the window to the full retained history.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_period_is_rejected_by_the_detail_module() {
    // `parse_period` returns Option and both call sites passed `None` straight
    // through to `get_*_for_period`, whose None arm aggregates EVERY retained
    // record — while the response still echoed the requested `period`. That is
    // the exact shape §6.7.1.1 forbids. The input_schema `pattern` is the first
    // line of defence, but §6.6.3.2 documents three presets that remove the
    // `input_validation` step, so a direct module call can arrive unvalidated.
    //
    // apcore-python raises ValueError and apcore-typescript throws for the same
    // input (verified 2026-08-27).
    let registry = Arc::new(Registry::new());
    register_dummy(&registry, "a.b");
    let module = UsageModule::new(Arc::clone(&registry), UsageCollector::new());

    for bad in ["xyz", "0h", "-5d", "+3h", "24", "24x", ""] {
        let err = module
            .execute(json!({ "module_id": "a.b", "period": bad }), &dummy_ctx())
            .await
            .expect_err(&format!("period {bad:?} must be rejected, not widened"));
        assert_eq!(
            err.code,
            apcore::errors::ErrorCode::SchemaValidationError,
            "period {bad:?} should fail schema validation, got {err:?}"
        );
    }
}

#[tokio::test]
async fn malformed_period_is_rejected_by_the_summary_module() {
    let module = UsageSummaryModule::new(UsageCollector::new());
    let err = module
        .execute(json!({ "period": "xyz" }), &dummy_ctx())
        .await
        .expect_err("a malformed period must be rejected by the summary module too");
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaValidationError);
}

#[tokio::test]
async fn well_formed_periods_still_parse() {
    let module = UsageSummaryModule::new(UsageCollector::new());
    for good in ["1h", "24h", "7d", "168h"] {
        module
            .execute(json!({ "period": good }), &dummy_ctx())
            .await
            .unwrap_or_else(|e| panic!("period {good:?} must parse, got {e:?}"));
    }
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-018: a module that has never been called is
// `current == 0 && previous == 0`, which §6.7.1.5 decides as "stable".
// ---------------------------------------------------------------------------

#[tokio::test]
async fn never_called_module_reports_stable_not_inactive() {
    // §6.7.1.5's table orders the zero-zero row FIRST, ahead of the
    // `current == 0` row, precisely so a module with no history reads "stable".
    // `inactive` means "had traffic, now has none".
    //
    // `UsageCollector::compute_trend` implements the table correctly; the
    // module layer bypassed it, because `get_module_summary_for_period` returns
    // None for a module with no records and the None arm hardcoded "inactive".
    // apcore-python and apcore-typescript route through _build_detail /
    // _buildDetail, which run the table, and answer "stable" (verified).
    let registry = Arc::new(Registry::new());
    register_dummy(&registry, "never.called");
    let module = UsageModule::new(Arc::clone(&registry), UsageCollector::new());

    let out = module
        .execute(json!({ "module_id": "never.called" }), &dummy_ctx())
        .await
        .expect("a module with no usage is not an error");

    assert_eq!(out["trend"], json!("stable"), "got {out:#?}");
    assert_eq!(out["call_count"], json!(0));
}
