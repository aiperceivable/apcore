//! Drive `event_naming.json` — canonical `apcore.<subsystem>.<event>` names
//! (Issue #36 / D-34, PROTOCOL_SPEC §9.16).
//!
//! `tests/test_event_naming_canonical.rs` hand-transcribes part of this; that
//! copy cannot notice a case being added upstream, which is why the canonical
//! fixture is loaded here.
//!
//! `data_contains.module_id`: in this SDK `ApCoreEvent` carries `module_id` as
//! a top-level field (src/events/emitter.rs:26) rather than inside `data`. The
//! driver looks in `data` first and falls back to the field, so the fixture's
//! claim is checked wherever the SDK actually puts the value.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::subscribers::EventSubscriber;
use apcore::executor::Executor;
use apcore::middleware::{Middleware, PlatformNotifyMiddleware};
use apcore::module::Module;
use apcore::observability::metrics::MetricsCollector;
use apcore::registry::registry::Registry;
use apcore::sys_modules::{register_sys_modules_with_options, SysModulesOptions};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("event_naming.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("event_naming.json parses")
}

fn case_by_id(fx: &Value, id: &str) -> Value {
    fx["test_cases"]
        .as_array()
        .expect("test_cases is an array")
        .iter()
        .find(|tc| tc["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("event_naming.json no longer carries case `{id}`"))
        .clone()
}

/// Cases held out of the always-on run. Empty: the three cases that used to sit
/// here were quarantined against FIXTURE defects, all since corrected upstream —
/// `legacy_dual_emit` / `legacy_health_dual_emit` required the v0.21.x
/// dual-emission that apcore#78 removed (replaced by the inverse case
/// `legacy_names_are_not_emitted`), and `health_threshold_canonical` pinned an
/// exact p99 of 6000.0 where p99 is a bucketed estimate (now `data_at_least`).
const QUARANTINED: &[&str] = &[];

// ---------------------------------------------------------------------------
// Recording subscriber
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct RecordingSub {
    pattern: String,
    received: Arc<Mutex<Vec<ApCoreEvent>>>,
}

#[async_trait]
impl EventSubscriber for RecordingSub {
    fn subscriber_id(&self) -> &'static str {
        "event-naming-conformance"
    }
    fn event_pattern(&self) -> &str {
        &self.pattern
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.received.lock().push(event.clone());
        Ok(())
    }
}

fn recorder(pattern: &str) -> (Box<RecordingSub>, Arc<Mutex<Vec<ApCoreEvent>>>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    (
        Box::new(RecordingSub {
            pattern: pattern.to_string(),
            received: Arc::clone(&received),
        }),
        received,
    )
}

/// `EventEmitter::emit` dispatches on spawned tasks (A-D-024), so events arrive
/// after `emit` returns. Poll until `want` events have landed or the budget is
/// spent; the assertions then work on whatever actually arrived.
async fn drain(received: &Arc<Mutex<Vec<ApCoreEvent>>>, want: usize) -> Vec<ApCoreEvent> {
    for _ in 0..200 {
        if received.lock().len() >= want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    // Small settle window so an unexpected EXTRA event still shows up and can
    // fail a "must not receive" assertion.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    received.lock().clone()
}

struct DummyModule;
#[async_trait]
impl Module for DummyModule {
    fn description(&self) -> &'static str {
        "conformance fixture module"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

// ---------------------------------------------------------------------------
// Harnesses — one per `action` family the fixture uses
// ---------------------------------------------------------------------------

/// Run `registry.register` / `registry.unregister` triggers, returning every
/// event a subscriber on `pattern` received.
async fn run_registry_triggers(pattern: &str, triggers: &[Value]) -> Vec<ApCoreEvent> {
    let registry = Arc::new(Registry::new());
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    let sys = register_sys_modules_with_options(
        Arc::clone(&registry),
        &executor,
        &config,
        None,
        SysModulesOptions::default(),
    )
    .expect("register_sys_modules");

    let (sub, received) = recorder(pattern);
    sys.emitter.subscribe(sub);

    let mut expected_events = 0usize;
    for trigger in triggers {
        let target = trigger["target_id"].as_str().expect("trigger.target_id");
        match trigger["action"].as_str().expect("trigger.action") {
            "registry.register" => {
                registry
                    .register_module(target, Box::new(DummyModule))
                    .expect("register_module");
                expected_events += 1;
            }
            "registry.unregister" => {
                // Register first when the case did not do so explicitly, so
                // there is something to unregister.
                if !registry.has(target) {
                    registry
                        .register_module(target, Box::new(DummyModule))
                        .expect("register_module");
                }
                registry.unregister(target).expect("unregister");
                expected_events += 1;
            }
            other => panic!("registry harness cannot run action `{other}`"),
        }
    }
    drain(&received, expected_events).await
}

/// Run `platform_notify.*` triggers, returning every event a subscriber on
/// `pattern` received.
async fn run_platform_notify_triggers(pattern: &str, triggers: &[Value]) -> Vec<ApCoreEvent> {
    let emitter = EventEmitter::new();
    let (sub, received) = recorder(pattern);
    emitter.subscribe(sub);

    let metrics = MetricsCollector::new();
    // Thresholds come from the fixture when it states them, so a fixture edit
    // moves this driver too.
    let error_threshold = triggers
        .iter()
        .find_map(|t| {
            (t["action"] == json!("platform_notify.error_threshold_crossed"))
                .then(|| t["threshold"].as_f64())
                .flatten()
        })
        .unwrap_or(0.10);
    let latency_threshold = triggers
        .iter()
        .find_map(|t| {
            (t["action"] == json!("platform_notify.latency_threshold_crossed"))
                .then(|| t["threshold"].as_f64())
                .flatten()
        })
        .unwrap_or(5000.0);
    let pn = PlatformNotifyMiddleware::new(
        emitter,
        Some(metrics.clone()),
        error_threshold,
        latency_threshold,
    );
    let ctx = Context::<Value>::anonymous();

    let mut expected_events = 0usize;
    for trigger in triggers {
        let module_id = trigger["target_id"].as_str().expect("trigger.target_id");
        match trigger["action"].as_str().expect("trigger.action") {
            "platform_notify.error_threshold_crossed" => {
                // Drive the collector to the fixture's error_rate (default: a
                // rate comfortably above the threshold) so the middleware's own
                // computation crosses the line.
                let rate = trigger["error_rate"]
                    .as_f64()
                    .unwrap_or(error_threshold * 1.5);
                let errors = (rate * 100.0).round();
                let successes = 100.0 - errors;
                let mut labels = HashMap::new();
                labels.insert("module_id".to_string(), module_id.to_string());
                labels.insert("status".to_string(), "error".to_string());
                metrics.increment("apcore_module_calls_total", labels.clone(), errors);
                labels.insert("status".to_string(), "success".to_string());
                metrics.increment("apcore_module_calls_total", labels, successes);

                pn.on_error(
                    module_id,
                    json!({}),
                    &ModuleError::new(ErrorCode::GeneralInternalError, "synthetic"),
                    &ctx,
                )
                .await
                .expect("on_error");
                expected_events += 1;
            }
            "platform_notify.latency_threshold_crossed" => {
                let p99_ms = trigger["p99_latency_ms"]
                    .as_f64()
                    .unwrap_or(latency_threshold * 1.2);
                let mut labels = HashMap::new();
                labels.insert("module_id".to_string(), module_id.to_string());
                metrics.observe("apcore_module_duration_seconds", labels, p99_ms / 1000.0);
                pn.after(module_id, json!({}), json!({}), &ctx)
                    .await
                    .expect("after");
                expected_events += 1;
            }
            "platform_notify.recovered" => {
                // Recovery requires the error rate to fall below half the
                // threshold while an alert is outstanding. Counters only grow,
                // so drive it down with successes.
                let mut labels = HashMap::new();
                labels.insert("module_id".to_string(), module_id.to_string());
                labels.insert("status".to_string(), "success".to_string());
                metrics.increment("apcore_module_calls_total", labels, 5000.0);
                pn.after(module_id, json!({}), json!({}), &ctx)
                    .await
                    .expect("after");
                expected_events += 1;
            }
            other => panic!("platform_notify harness cannot run action `{other}`"),
        }
    }
    drain(&received, expected_events).await
}

/// Triggers of a case, whether stated as `trigger` or `trigger_sequence`.
fn triggers_of(tc: &Value, id: &str) -> Vec<Value> {
    if let Some(seq) = tc["trigger_sequence"].as_array() {
        return seq.clone();
    }
    if tc["trigger"].is_object() {
        return vec![tc["trigger"].clone()];
    }
    panic!("[{id}] case has neither `trigger` nor `trigger_sequence`")
}

async fn events_for(tc: &Value, id: &str) -> Vec<ApCoreEvent> {
    let triggers = triggers_of(tc, id);
    let pattern = tc["subscription_pattern"].as_str().unwrap_or("*");
    let family = triggers[0]["action"]
        .as_str()
        .expect("trigger.action")
        .split('.')
        .next()
        .unwrap();
    match family {
        "registry" => run_registry_triggers(pattern, &triggers).await,
        "platform_notify" => run_platform_notify_triggers(pattern, &triggers).await,
        other => panic!("[{id}] unknown trigger family `{other}`"),
    }
}

/// `data_contains` lookup: `event.data[key]`, falling back to the top-level
/// `module_id` field this SDK stores outside `data`.
fn event_field<'a>(event: &'a ApCoreEvent, key: &str) -> Option<&'a Value> {
    if let Some(v) = event.data.get(key) {
        return Some(v);
    }
    None
}

fn assert_data_contains(id: &str, event: &ApCoreEvent, want: &Value) {
    for (key, want_value) in want.as_object().expect("data_contains is an object") {
        if key == "module_id" {
            let actual = event_field(event, key).cloned().or_else(|| {
                event
                    .module_id
                    .as_ref()
                    .map(|m| Value::String(m.to_string()))
            });
            assert_eq!(
                actual.as_ref(),
                Some(want_value),
                "[{id}] event {} module_id",
                event.event_type
            );
            continue;
        }
        let actual = event_field(event, key)
            .unwrap_or_else(|| panic!("[{id}] event {} has no `{key}`", event.event_type));
        let equal = if actual.is_number() && want_value.is_number() {
            (actual.as_f64().unwrap() - want_value.as_f64().unwrap()).abs() < 1e-9
        } else {
            actual == want_value
        };
        assert!(
            equal,
            "[{id}] event {} field `{key}`: expected {want_value}, got {actual}",
            event.event_type
        );
    }
}

/// Lower-bound assertion for values a conformant SDK may legitimately report
/// higher than the fixture's floor. `p99_latency_ms` is the motivating case: it
/// is a histogram-bucket ESTIMATE, and apcore-rust's DEFAULT_BUCKETS has no
/// 6.0s boundary, so it reports the enclosing 10000.0. Pinning an exact value
/// made the fixture assert an implementation detail of one SDK's bucket table.
fn assert_data_at_least(id: &str, event: &ApCoreEvent, want: &Value) {
    for (key, floor) in want.as_object().expect("data_at_least is an object") {
        let actual = event_field(event, key)
            .unwrap_or_else(|| panic!("[{id}] event {} has no `{key}`", event.event_type));
        let actual_f = actual
            .as_f64()
            .unwrap_or_else(|| panic!("[{id}] `{key}` is not numeric: {actual}"));
        let floor_f = floor.as_f64().expect("data_at_least values are numeric");
        assert!(
            actual_f >= floor_f,
            "[{id}] event {} field `{key}`: expected at least {floor_f}, got {actual_f}",
            event.event_type
        );
    }
}

async fn run_case(tc: &Value) {
    let id = tc["id"].as_str().expect("every case needs an id");
    let events = events_for(tc, id).await;
    let types: Vec<String> = events.iter().map(|e| e.event_type.clone()).collect();

    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

    for (field, want) in expected {
        match field.as_str() {
            "canonical_event" => {
                let want_type = want["event_type"].as_str().expect("event_type");
                let event = events
                    .iter()
                    .find(|e| e.event_type == want_type)
                    .unwrap_or_else(|| panic!("[{id}] no `{want_type}` event; saw {types:?}"));
                if let Some(data) = want.get("data_contains") {
                    assert_data_contains(id, event, data);
                }
                if let Some(data) = want.get("data_at_least") {
                    assert_data_at_least(id, event, data);
                }
            }
            "events" => {
                for want_event in want.as_array().expect("expected.events is an array") {
                    let want_type = want_event["event_type"].as_str().expect("event_type");
                    let event = events
                        .iter()
                        .find(|e| e.event_type == want_type)
                        .unwrap_or_else(|| panic!("[{id}] no `{want_type}` event; saw {types:?}"));
                    if let Some(data) = want_event.get("data_contains") {
                        assert_data_contains(id, event, data);
                    }
                    if let Some(data) = want_event.get("data_at_least") {
                        assert_data_at_least(id, event, data);
                    }
                }
            }
            "received_event_types" => {
                let want_types: Vec<String> = want
                    .as_array()
                    .expect("received_event_types is an array")
                    .iter()
                    .map(|v| v.as_str().expect("event type is a string").to_string())
                    .collect();
                let mut got = types.clone();
                got.sort();
                got.dedup();
                let mut want_sorted = want_types.clone();
                want_sorted.sort();
                assert_eq!(
                    got,
                    want_sorted,
                    "[{id}] subscriber on `{}` received the wrong event set",
                    tc["subscription_pattern"].as_str().unwrap_or("*")
                );
            }
            "forbidden_event_types" => {
                // Asserted per-case, on whichever case's trigger can actually
                // produce the name (fixture `driver_contract.
                // forbidden_names_need_a_reachable_trigger`): the registry
                // bare names sit on `legacy_names_are_not_emitted`, the health
                // aliases on `health_threshold_canonical`. Both are checked
                // here — `conformance_forbidden_names_have_a_reachable_trigger`
                // below keeps that pairing from drifting back apart, since a
                // forbidden name under a trigger that could never emit it
                // passes for free.
                let forbidden: Vec<&str> = want
                    .as_array()
                    .expect("forbidden_event_types is an array")
                    .iter()
                    .map(|v| v.as_str().expect("event type is a string"))
                    .collect();
                let leaked: Vec<&&str> = forbidden
                    .iter()
                    .filter(|name| types.iter().any(|t| t == **name))
                    .collect();
                assert!(
                    leaked.is_empty(),
                    "[{id}] dual-emission ended at v0.22.0; these names MUST NOT be \
                     emitted but were: {leaked:?}. Emitted: {types:?}"
                );
            }
            other => panic!(
                "[{id}] event_naming.json grew expectation `{other}` that this driver \
                 does not check — teach the driver, do not skip it"
            ),
        }
    }
}

#[tokio::test]
async fn conformance_event_naming() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    for id in QUARANTINED {
        let _ = case_by_id(&fx, id);
    }

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        if QUARANTINED.contains(&id) {
            continue; // QUARANTINED is empty — see its doc comment
        }
        run_case(tc).await;
    }
}

/// The fixture's `forbidden_names_need_a_reachable_trigger` contract, executed.
///
/// A `forbidden_event_types` entry asserts something only if the case's trigger
/// could plausibly emit it. `legacy_names_are_not_emitted` once also listed the
/// two health-threshold aliases while triggering `registry.register` — a path
/// no implementation emits them on — so half of it was free while making the
/// case look twice as thorough. Verified rather than trusted: re-running this
/// driver against the pre-change distribution with a live legacy-alias emission
/// injected into `PlatformNotifyMiddleware` passed GREEN; with the current
/// distribution the same regression fails RED.
///
/// The pairing is derived from the NAME, so a newly forbidden name filed under
/// an unreachable trigger fails here rather than silently asserting nothing.
#[test]
fn conformance_forbidden_names_have_a_reachable_trigger() {
    /// Which trigger family is capable of emitting `name`.
    fn emitting_family(name: &str) -> &'static str {
        if name.contains("threshold") {
            "platform_notify"
        } else if name.contains("module_registered") || name.contains("module_unregistered") {
            "registry"
        } else {
            panic!(
                "event_naming.json forbids `{name}`, which this driver cannot map to a \
                 trigger family — teach it which trigger emits the name, otherwise the \
                 entry cannot be shown to assert anything"
            )
        }
    }

    let fx = fixture();
    let mut checked = 0usize;
    for tc in fx["test_cases"].as_array().expect("test_cases is an array") {
        let id = tc["id"].as_str().expect("every case needs an id");
        let Some(forbidden) = tc["expected"]["forbidden_event_types"].as_array() else {
            continue;
        };
        let triggers = triggers_of(tc, id);
        let families: Vec<&str> = triggers
            .iter()
            .map(|t| {
                t["action"]
                    .as_str()
                    .expect("trigger.action")
                    .split('.')
                    .next()
                    .unwrap()
            })
            .collect();
        for name in forbidden {
            let name = name.as_str().expect("event type is a string");
            let needed = emitting_family(name);
            assert!(
                families.contains(&needed),
                "[{id}] forbids `{name}`, which only a `{needed}.*` trigger can emit, \
                 but this case triggers {families:?} — the assertion is vacuous where \
                 it stands and belongs on a case that can actually produce the name"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 4,
        "expected the four removed legacy names (2 registry + 2 health) to still be \
         pinned somewhere in event_naming.json; only {checked} forbidden entries found"
    );
}
