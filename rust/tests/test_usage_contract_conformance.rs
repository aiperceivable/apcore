//! Cross-language conformance driver for `usage_contract.json`
//! (PROTOCOL_SPEC 6.7.1 — the value semantics no JSON Schema can assert).
//!
//! Fixture source: apcore/conformance/fixtures/usage_contract.json (canonical).
//!
//! Drives the real `system.usage.*` modules against a real `UsageCollector`,
//! per `driver_contract.path`: every divergence this fixture pins lived in the
//! sys-module layer's choice of accessor, so a driver that calls the
//! collector's own period-aware methods asserts the layer that was never wrong.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::context::{Context, Identity};
use apcore::middleware::Middleware;
use apcore::module::{Module, ModuleAnnotations};
use apcore::observability::usage::UsageMiddleware;
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{UsageCollector, UsageModule, UsageSummaryModule};
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::conformance_env::{find_fixtures_root, find_schemas_root};

/// driver_contract.output_validates_against_the_canonical_schema: the same file
/// the spec repo ships, not a copy. `additionalProperties: false` is the point —
/// a field one SDK emits and the others do not fails here — and the `hour`
/// pattern rejects the `YYYY-MM-DDTHH:00:00Z` spelling this SDK used to emit.
///
/// Validated through `apcore::executor::validate_against_schema`, the same
/// entry point the executor's own input/output validation uses, so the driver
/// cannot pass through machinery a real module call never touches.
fn validate_against_canonical_schema(module: &str, output: &Value) {
    let file = match module {
        "system.usage.summary" => "sys-usage-summary.schema.json",
        "system.usage.module" => "sys-usage-module.schema.json",
        other => panic!("fixture names an unknown module {other:?}"),
    };
    // §8.2.1 rule 4: resolve `schemas/` on its own rather than climbing out of
    // the fixtures root. Under `CONFORMANCE_FIXTURES` there is no repo above it.
    let path = find_schemas_root().join(file);
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("failed to read {}", path.display())),
    )
    .expect("canonical schema is valid JSON");

    apcore::executor::validate_against_schema(output, &schema, "Output")
        .unwrap_or_else(|e| panic!("{module} output does not satisfy the canonical schema: {e}"));
}

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

fn dummy_ctx() -> Context<Value> {
    Context::<Value>::new(Identity::new(
        "@test".to_string(),
        "test".to_string(),
        vec![],
        HashMap::default(),
    ))
}

/// A Context carrying no caller identity.
fn no_caller_ctx() -> Context<Value> {
    let mut ctx = dummy_ctx();
    ctx.caller_id = None;
    ctx
}

fn parse_offset(offset: &str) -> Duration {
    let body = offset
        .strip_prefix('-')
        .expect("at_offset must be negative");
    let (digits, unit) = body.split_at(body.len() - 1);
    let n: i64 = digits.parse().expect("at_offset amount");
    match unit {
        "h" => Duration::hours(n),
        "d" => Duration::days(n),
        other => panic!("unsupported at_offset unit {other:?}"),
    }
}

fn register(registry: &Arc<Registry>, id: &str) {
    struct Dummy;
    #[async_trait::async_trait]
    impl Module for Dummy {
        fn description(&self) -> &'static str {
            "conformance target"
        }
        fn input_schema(&self) -> Value {
            json!({})
        }
        fn output_schema(&self) -> Value {
            json!({})
        }
        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, apcore::errors::ModuleError> {
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
        .register_internal(id, Box::new(Dummy), descriptor)
        .expect("register_internal should succeed");
}

async fn collector_for(case: &Value, module_id: &str) -> UsageCollector {
    let collector = UsageCollector::new();
    if let Some(latencies) = case["latencies_ms"].as_array() {
        for latency in latencies {
            collector.record(module_id, Some("caller-a"), latency.as_f64().unwrap(), true);
        }
    }
    for record in case["records"].as_array().unwrap_or(&Vec::new()) {
        let success = record["success"].as_bool().unwrap_or(true);
        if record["caller_id"].is_null() {
            // driver_contract.unattributed_caller: a call with NO caller
            // identity goes through this SDK's usage-recording path.
            // apcore-rust substitutes "unknown" in the caller breakdown,
            // because record() takes Option<&str>; apcore-python and
            // apcore-typescript substitute in UsageMiddleware.
            let middleware = UsageMiddleware::new(collector.clone());
            let ctx = no_caller_ctx();
            middleware
                .before(module_id, json!({}), &ctx)
                .await
                .expect("before");
            if success {
                middleware
                    .after(module_id, json!({}), json!({}), &ctx)
                    .await
                    .expect("after");
            }
            continue;
        }
        let at = Utc::now() - parse_offset(record["at_offset"].as_str().expect("at_offset"));
        collector.record_at(
            module_id,
            record["caller_id"].as_str(),
            record["latency_ms"].as_f64().unwrap_or(0.0),
            success,
            at,
        );
    }
    collector
}

async fn run(case: &Value) -> Value {
    let module_id = case["module_id"].as_str().unwrap_or("math.add");
    let registry = Arc::new(Registry::new());
    register(&registry, module_id);
    let collector = collector_for(case, module_id).await;

    let mut inputs = case["inputs"].clone();
    if inputs.is_null() {
        inputs = json!({});
    }

    if case["module"] == "system.usage.summary" {
        return UsageSummaryModule::new(collector)
            .execute(inputs, &dummy_ctx())
            .await
            .expect("summary should succeed");
    }
    if inputs["module_id"].is_null() {
        inputs["module_id"] = json!(module_id);
    }
    UsageModule::new(registry, collector)
        .execute(inputs, &dummy_ctx())
        .await
        .expect("detail should succeed")
}

/// Execute `case` through a real client and report the error code it raised.
///
/// `None` when the call succeeded, so a case whose input is wrongly ACCEPTED
/// fails the assertion rather than panicking somewhere less legible.
async fn rejection_code(case: &serde_json::Value) -> Option<String> {
    use apcore::client::APCore;
    use apcore::config::Config;

    let raw = serde_json::json!({
        "version": "1.0",
        "project": {"name": "usage-contract"},
        "sys_modules": {"enabled": true, "usage": {"enabled": true}}
    });
    let config: Config = serde_json::from_value(raw).expect("probe config parses");
    let client = APCore::with_config(config);
    let module = case["module"].as_str().expect("module");
    let inputs = case["inputs"].clone();
    client
        .call(module, inputs, None, None)
        .await
        .err()
        // The WIRE code, not the Rust variant name: `ErrorCode` serialises as
        // SCREAMING_SNAKE_CASE, and the wire spelling is what the fixture
        // declares and what an operator sees.
        .map(|e| {
            serde_json::to_value(e.code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{:?}", e.code))
        })
}

#[tokio::test]
async fn conformance_usage_contract() {
    let fixture = load_fixture("usage_contract");
    let cases = fixture["test_cases"].as_array().expect("test_cases");
    assert!(!cases.is_empty(), "fixture must declare cases");

    for case in cases {
        let id = case["id"].as_str().expect("id");
        let note = case["note"].as_str().unwrap_or("");
        let expected = case["expected"].as_object().expect("expected");

        // Rejection cases assert the declared grammar. It lives in
        // input_schema (6.7.1.1), so rejection happens at input validation
        // with SCHEMA_VALIDATION_ERROR rather than inside a private parser.
        if expected.contains_key("error_code") {
            let schema = UsageSummaryModule::new(UsageCollector::new()).input_schema();
            let pattern = schema["properties"]["period"]["pattern"]
                .as_str()
                .expect("period pattern must be declared");
            assert_eq!(pattern, "^[1-9][0-9]*[hd]$", "{id}: {note}");
            let period = case["inputs"]["period"].as_str().expect("period");
            assert!(
                !regex::Regex::new(pattern).unwrap().is_match(period),
                "{id}: fixture expects {period:?} to be rejected, but the pattern accepts it"
            );

            // ...and then assert what the case actually SAYS. Both checks above
            // are about the schema; neither reads `expected["error_code"]`, so
            // this branch passed whatever wire code the fixture declared — the
            // case ran and its expectation asserted nothing. Found by
            // `check_case_pinning.py`: mutating the code left all three SDKs
            // green, which is how they came to share the same proxy.
            //
            // Rejection happens at input validation (§6.7.1.1), the pipeline's
            // job rather than `execute`'s, so this goes through a real client.
            let want = expected["error_code"].as_str().expect("error_code");
            let err = rejection_code(case).await;
            assert_eq!(
                err.as_deref(),
                Some(want),
                "{id}: {period:?} was rejected as {err:?}, and the fixture declares \
                 {want:?} as the wire code an operator sees"
            );
            continue;
        }

        let result = run(case).await;
        validate_against_canonical_schema(case["module"].as_str().unwrap(), &result);

        if let Some(want) = expected.get("caller_ids") {
            let ids: Vec<&str> = result["callers"]
                .as_array()
                .expect("callers")
                .iter()
                .map(|c| c["caller_id"].as_str().expect("caller_id"))
                .collect();
            let want: Vec<&str> = want
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(ids, want, "{id}: {note}");
        }

        if let Some(len) = expected.get("hourly_distribution_length") {
            let hourly = result["hourly_distribution"].as_array().expect("hourly");
            assert_eq!(hourly.len() as u64, len.as_u64().unwrap(), "{id}: {note}");

            let pattern = expected["hourly_distribution_key_format"].as_str().unwrap();
            let re = regex::Regex::new(pattern).unwrap();
            let mut hours: Vec<&str> = Vec::new();
            for entry in hourly {
                let hour = entry["hour"].as_str().expect("hour");
                assert!(
                    re.is_match(hour),
                    "{id}: hour key {hour:?} is not YYYY-MM-DDTHH"
                );
                hours.push(hour);
            }
            let total: u64 = hourly
                .iter()
                .map(|e| e["call_count"].as_u64().unwrap_or(0))
                .sum();
            assert_eq!(
                total,
                expected["hourly_distribution_total_calls"]
                    .as_u64()
                    .unwrap(),
                "{id}: {note}"
            );
            if expected["hourly_distribution_sorted_ascending"]
                .as_bool()
                .unwrap_or(false)
            {
                let mut sorted = hours.clone();
                sorted.sort_unstable();
                assert_eq!(hours, sorted, "{id}: hourly buckets must be ascending");
            }
        }

        for (name, want) in expected {
            if name.starts_with("hourly_distribution_") || name == "caller_ids" {
                continue;
            }
            assert_eq!(&result[name], want, "{id}: {name} — {note}");
        }
    }
}
