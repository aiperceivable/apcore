//! Cross-language conformance for three high-drift fixtures wired into the
//! data-driven runner. Pulls each fixture from the canonical `apcore` spec
//! repository (resolved via `CONFORMANCE_SPEC_REPO` or the sibling `../apcore/`
//! directory) and exercises the corresponding Rust SDK behavior.
//!
//! Fixtures:
//!   * `sensitive_keys_default` (D-54) — canonical default `sensitive_keys`
//!     list, redaction behavior, and override-replaces semantics.
//!   * `error_fingerprinting`   — UUID/timestamp/numeric normalization and
//!     fingerprint-based dedup in `ErrorHistory`.
//!   * `contextual_audit`       — `system.control.*` audit events include
//!     `caller_id` (defaulted to `"@external"`) and a redacted `identity`
//!     snapshot when present.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;

use apcore::config::Config;
use apcore::context::{Context, ContextBuilder, Identity};
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::subscribers::EventSubscriber;
use apcore::observability::error_history::{compute_fingerprint, ErrorHistory};
use apcore::observability::redaction::{RedactionConfig, DEFAULT_SENSITIVE_KEYS};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::sys_modules::control::{ReloadModule, ToggleFeatureModule};
use apcore::sys_modules::ToggleState;
use apcore::{ErrorCode, Module, ModuleAnnotations, ModuleError, UpdateConfigModule};

// ---------------------------------------------------------------------------
// Fixture loading (mirrors conformance_test.rs::load_fixture)
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

// ---------------------------------------------------------------------------
// 1. sensitive_keys_default (D-54)
// ---------------------------------------------------------------------------

fn build_redaction(tc: &Value) -> RedactionConfig {
    match tc["construction"].as_str().unwrap_or("default") {
        "default" => RedactionConfig::with_default_sensitive_keys(),
        "override" => {
            let entries: Vec<String> = tc["override_sensitive_keys"]
                .as_array()
                .expect("override case requires override_sensitive_keys array")
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            RedactionConfig::builder().sensitive_keys(entries).build()
        }
        other => panic!("unknown construction mode: {other}"),
    }
}

#[test]
fn conformance_sensitive_keys_default() {
    let fixture = load_fixture("sensitive_keys_default");
    let cases = fixture["test_cases"].as_array().unwrap();

    for tc in cases {
        let id = tc["id"].as_str().unwrap();

        // Case 1: assert the canonical default list verbatim.
        if id == "default_list_is_canonical_16_entries" {
            let expected: Vec<String> = tc["expected"]["sensitive_keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            let length = usize::try_from(tc["expected"]["length"].as_u64().unwrap())
                .expect("length fits in usize");

            assert_eq!(
                DEFAULT_SENSITIVE_KEYS.len(),
                length,
                "FAIL [{id}]: DEFAULT_SENSITIVE_KEYS length"
            );
            let actual: Vec<String> = DEFAULT_SENSITIVE_KEYS
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            assert_eq!(
                actual, expected,
                "FAIL [{id}]: DEFAULT_SENSITIVE_KEYS list does not match canonical order"
            );
            continue;
        }

        // Cases 2..4: redaction-behavior assertions.
        let cfg = build_redaction(tc);
        let mut input = tc["input"].clone();
        cfg.redact(&mut input);

        let expected = &tc["expected"];
        for (key, expected_value) in expected.as_object().unwrap() {
            let actual = input
                .get(key)
                .unwrap_or_else(|| panic!("FAIL [{id}]: missing key {key:?} after redact"));
            assert_eq!(actual, expected_value, "FAIL [{id}]: key {key:?} mismatch");
        }
    }
}

// ---------------------------------------------------------------------------
// 2. error_fingerprinting
// ---------------------------------------------------------------------------

#[test]
fn conformance_error_fingerprinting() {
    use std::collections::HashSet;

    let fixture = load_fixture("error_fingerprinting");
    let cases = fixture["test_cases"].as_array().unwrap();

    for tc in cases {
        let id = tc["id"].as_str().unwrap();
        let errors = tc["errors"].as_array().unwrap();
        let expected = &tc["expected"];

        // Compute fingerprints and group counts. Per the fixture description,
        // callers SHOULD substitute caller_id when stack traces are
        // unavailable; if `top_frame` is supplied we use that as the
        // top_frame_hash surrogate instead.
        let mut fp_counts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        let mut fps_in_order: Vec<String> = Vec::new();
        let mut distinct: HashSet<String> = HashSet::new();

        for err in errors {
            let error_code = err["error_code"].as_str().unwrap();
            let frame = err
                .get("top_frame")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| err["caller_id"].as_str().unwrap());
            let message = err["message"].as_str().unwrap();
            let fp = compute_fingerprint(error_code, frame, message);
            *fp_counts.entry(fp.clone()).or_insert(0) += 1;
            distinct.insert(fp.clone());
            fps_in_order.push(fp);
        }

        if let Some(expected_distinct) = expected.get("fingerprints_distinct") {
            assert_eq!(
                distinct.len() as u64,
                expected_distinct.as_u64().unwrap(),
                "FAIL [{id}]: fingerprints_distinct"
            );
        }
        if let Some(expected_entry_count) = expected.get("entry_count") {
            assert_eq!(
                distinct.len() as u64,
                expected_entry_count.as_u64().unwrap(),
                "FAIL [{id}]: entry_count (distinct fingerprints)"
            );
        }
        if let Some(expected_first_count) = expected.get("first_entry_count") {
            // first_entry_count = number of times the first-seen fingerprint
            // recurred (its count after all errors are recorded).
            let first_fp = &fps_in_order[0];
            let count = fp_counts[first_fp];
            assert_eq!(
                count,
                expected_first_count.as_u64().unwrap(),
                "FAIL [{id}]: first_entry_count"
            );
        }
        if let Some(pattern_src) = expected.get("timestamp_pattern").and_then(|v| v.as_str()) {
            // D-120. Driven through the real `ErrorHistory` and its own
            // serialization, because the decision is about the record the
            // PRODUCER writes: this SDK's health summary used to reformat the
            // same entry with `to_rfc3339()` into a third form, so asserting on
            // a reader would have passed while the stored record diverged.
            let pattern = regex::Regex::new(pattern_src).expect("timestamp_pattern compiles");
            let history = apcore::observability::error_history::ErrorHistory::new(100);
            for err in errors {
                history.record(
                    err["caller_id"].as_str().unwrap(),
                    &apcore::errors::ModuleError::new(
                        apcore::errors::ErrorCode::GeneralInternalError,
                        err["message"].as_str().unwrap(),
                    ),
                );
            }
            let entries = history.get_all(None);
            assert!(!entries.is_empty(), "FAIL [{id}]: no entries recorded");
            for entry in &entries {
                let json = serde_json::to_value(entry).expect("entry serializes");
                for field in expected["timestamp_fields"].as_array().unwrap() {
                    let name = field.as_str().unwrap();
                    let value = json[name]
                        .as_str()
                        .unwrap_or_else(|| panic!("FAIL [{id}]: {name} is not a string: {json}"));
                    assert!(
                        pattern.is_match(value),
                        "FAIL [{id}]: {name} = {value:?} does not match {pattern_src}"
                    );
                }
            }
        }
    }

    // Spot-check the same algorithm via the canonical `ErrorHistory` storage
    // path for the simplest dedup scenario, so we exercise the production
    // code path beyond the bare `compute_fingerprint` helper.
    let history = ErrorHistory::with_limits(50, 1000);
    history.record(
        "executor.db.query",
        &ModuleError::new(
            ErrorCode::ModuleExecuteError,
            "Connection timed out for request a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        ),
    );
    history.record(
        "executor.db.query",
        &ModuleError::new(
            ErrorCode::ModuleExecuteError,
            "Connection timed out for request 11111111-2222-3333-4444-555555555555",
        ),
    );
    let entries = history.get("executor.db.query", None);
    assert_eq!(entries.len(), 1, "ErrorHistory must dedup UUID-only diffs");
    assert_eq!(entries[0].count, 2, "dedup should bump count");
}

// ---------------------------------------------------------------------------
// 3. contextual_audit (Issue #45.2)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CaptureSubscriber {
    events: Arc<parking_lot::Mutex<Vec<ApCoreEvent>>>,
}

#[async_trait::async_trait]
impl EventSubscriber for CaptureSubscriber {
    fn subscriber_id(&self) -> &'static str {
        "conformance-capture"
    }

    fn event_pattern(&self) -> &'static str {
        "*"
    }

    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.events.lock().push(event.clone());
        Ok(())
    }
}

fn build_ctx_from_fixture(ctx_spec: &Value) -> Context<Value> {
    let caller_id = ctx_spec
        .get("caller_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    let identity = ctx_spec.get("identity").and_then(|v| {
        if v.is_null() {
            None
        } else {
            let id = v["id"].as_str().unwrap_or("").to_string();
            let identity_type = v["type"].as_str().unwrap_or("user").to_string();
            let roles: Vec<String> = v
                .get("roles")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|r| r.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            // Surface every additional field as an `attrs` entry so
            // `display_name` / `bearer_token` from the fixture flow through
            // the identity snapshot.
            let mut attrs: std::collections::HashMap<String, Value> =
                std::collections::HashMap::new();
            if let Some(obj) = v.as_object() {
                for (k, val) in obj {
                    if matches!(k.as_str(), "id" | "type" | "roles") {
                        continue;
                    }
                    attrs.insert(k.clone(), val.clone());
                }
            }
            Some(Identity::new(id, identity_type, roles, attrs))
        }
    });

    ContextBuilder::<Value>::new()
        .identity(identity)
        .caller_id(caller_id)
        .services(Value::Null)
        .build()
}

fn register_dummy(registry: &Arc<Registry>, module_id: &str) {
    struct DummyModule;
    #[async_trait::async_trait]
    impl Module for DummyModule {
        fn description(&self) -> &'static str {
            "dummy"
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
        ) -> Result<Value, ModuleError> {
            Ok(json!({}))
        }
    }

    let descriptor = ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: json!({}),
        output_schema: json!({}),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: std::collections::HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    registry
        .register_internal(module_id, Box::new(DummyModule), descriptor)
        .expect("register_internal should succeed");
}

async fn invoke_module(
    fixture_module_id: &str,
    inputs: Value,
    ctx: &Context<Value>,
    config: Arc<Mutex<Config>>,
    emitter: Arc<EventEmitter>,
) -> Result<Value, ModuleError> {
    match fixture_module_id {
        "system.control.update_config" => {
            let module = UpdateConfigModule::new(config, emitter);
            module.execute(inputs, ctx).await
        }
        "system.control.toggle_feature" => {
            let registry = Arc::new(Registry::new());
            let module_id = inputs["module_id"].as_str().unwrap_or("risky.module");
            register_dummy(&registry, module_id);
            let toggle_state = Arc::new(ToggleState::new());
            let module = ToggleFeatureModule::new(registry, emitter, toggle_state);
            module.execute(inputs, ctx).await
        }
        "system.control.reload_module" => {
            let registry = Arc::new(Registry::new());
            let module_id = inputs["module_id"]
                .as_str()
                .unwrap_or("executor.email.send");
            register_dummy(&registry, module_id);
            // reload unregisters before re-discovering; restore it so this
            // audit-event fixture asserts the event, not a RELOAD_FAILED.
            crate::reload_support::RestoringDiscoverer::attach_for(
                &registry,
                &[module_id.to_string()],
            );
            let module = ReloadModule::new(registry, emitter);
            module.execute(inputs, ctx).await
        }
        other => panic!("unsupported fixture module_id: {other}"),
    }
}

#[tokio::test]
async fn conformance_contextual_audit() {
    let fixture = load_fixture("contextual_audit");
    let cases = fixture["test_cases"].as_array().unwrap();

    for tc in cases {
        let id = tc["id"].as_str().unwrap();
        let module_id = tc["module_id"].as_str().unwrap();
        let inputs = tc["input"].clone();
        let ctx = build_ctx_from_fixture(&tc["context"]);

        // Capture subscriber per case so events don't leak across iterations.
        let captured: Arc<parking_lot::Mutex<Vec<ApCoreEvent>>> =
            Arc::new(parking_lot::Mutex::new(Vec::new()));
        let emitter = Arc::new(EventEmitter::new());
        {
            let em = &emitter;
            em.subscribe(Box::new(CaptureSubscriber {
                events: Arc::clone(&captured),
            }));
        }

        let config = Arc::new(Mutex::new(Config::default()));
        // update_config consults the existing value; seed it so the
        // event payload reports `old_value: 30000` per the fixture.
        if module_id == "system.control.update_config" {
            let mut c = config.lock().await;
            c.set("executor.default_timeout", json!(30000));
        }

        invoke_module(
            module_id,
            inputs,
            &ctx,
            Arc::clone(&config),
            Arc::clone(&emitter),
        )
        .await
        .unwrap_or_else(|e| panic!("FAIL [{id}]: module execute returned {e}"));

        let expected = &tc["expected"];
        let expected_event_type = expected["event_type"].as_str().unwrap();

        // emit() dispatches deliveries on spawned tasks (A-D-024), so poll
        // until the expected event arrives or a short timeout elapses.
        for _ in 0..200 {
            if captured
                .lock()
                .iter()
                .any(|e| e.event_type == expected_event_type)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let evts = captured.lock().clone();
        let evt = evts
            .iter()
            .find(|e| e.event_type == expected_event_type)
            .unwrap_or_else(|| {
                panic!(
                    "FAIL [{id}]: expected event {expected_event_type}, got {:?}",
                    evts.iter().map(|e| &e.event_type).collect::<Vec<_>>()
                )
            });

        // data_contains: every key/value MUST appear (deep equality on the
        // value at the same path).
        if let Some(must_contain) = expected.get("data_contains").and_then(Value::as_object) {
            for (key, expected_value) in must_contain {
                let actual = evt.data.get(key).unwrap_or_else(|| {
                    panic!(
                        "FAIL [{id}]: event payload missing key {key:?}; payload={}",
                        evt.data
                    )
                });
                assert_eq!(
                    actual, expected_value,
                    "FAIL [{id}]: payload[{key:?}] mismatch"
                );
            }
        }

        // data_must_not_contain_keys: every listed key MUST be absent.
        if let Some(forbidden) = expected
            .get("data_must_not_contain_keys")
            .and_then(Value::as_array)
        {
            for key in forbidden {
                let key_str = key.as_str().unwrap();
                assert!(
                    evt.data.get(key_str).is_none(),
                    "FAIL [{id}]: payload MUST NOT contain key {key_str:?}; payload={}",
                    evt.data
                );
            }
        }

        // D-118 needs key ABSENCE inside the identity snapshot, which
        // `data_contains` cannot express: it is a SUBSET match, so an extra
        // `roles: []` passes it, and asserting `roles: []` is exactly what the
        // diverging SDK emits.
        if let Some(forbidden) = expected
            .get("identity_must_not_contain_keys")
            .and_then(Value::as_array)
        {
            let identity = evt.data.get("identity").cloned().unwrap_or(Value::Null);
            for key in forbidden {
                let key_str = key.as_str().unwrap();
                assert!(
                    identity.get(key_str).is_none(),
                    "FAIL [{id}]: identity snapshot MUST NOT contain key {key_str:?}; identity={identity}"
                );
            }
        }
    }
}
