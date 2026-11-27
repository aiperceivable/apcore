//! Exercise registry and host lifecycle through the public SDK.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apcore::errors::{ErrorCode, ModuleError};
use apcore::registry::registry::{DiscoveredModule, Discoverer, ModuleDescriptor};
use apcore::{
    APCore, APCoreOptions, AlwaysDenyHandler, ApprovalHandler, AutoApproveHandler, ChunkStream,
    Config, Context, Executor, LifecycleOperation, Module, ShutdownOptions, StreamingModule,
    UnregisterOptions,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::conformance_env::find_fixtures_root;

type EventLog = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}

struct Counter {
    label: Option<&'static str>,
    events: EventLog,
    gate: Arc<Gate>,
    closed: AtomicBool,
    count: Arc<AtomicU64>,
}

impl Counter {
    fn new(events: &EventLog, gate: &Arc<Gate>) -> Self {
        Self {
            label: None,
            events: Arc::clone(events),
            gate: Arc::clone(gate),
            closed: AtomicBool::new(false),
            count: Arc::new(AtomicU64::new(0)),
        }
    }

    fn record(&self, event: &str) {
        let event = self
            .label
            .map_or_else(|| event.to_owned(), |label| format!("{event}:{label}"));
        self.events.lock().unwrap().push(event);
    }
}

#[async_trait]
impl Module for Counter {
    fn description(&self) -> &str {
        "Record module lifecycle and transfer a counter"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {"hold": {"type": "boolean"}}})
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object", "properties": {"count": {"type": "integer"}}, "required": ["count"]})
    }

    fn on_load(&self) -> Result<(), ModuleError> {
        if self.label.is_some() {
            self.record("load");
        }
        Ok(())
    }

    async fn execute(&self, inputs: Value, _: &Context<Value>) -> Result<Value, ModuleError> {
        if inputs["hold"] == true {
            self.gate.entered.notify_one();
            self.gate.release.notified().await;
            assert!(
                !self.closed.load(Ordering::SeqCst),
                "module was unloaded during its call"
            );
            self.count.fetch_add(1, Ordering::SeqCst);
            self.record("finish");
        }
        Ok(json!({"count": self.count.load(Ordering::SeqCst)}))
    }

    fn on_suspend(&self) -> Option<Value> {
        self.record("suspend");
        Some(json!({"count": self.count.load(Ordering::SeqCst)}))
    }

    fn on_unload(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.record("unload");
    }

    fn on_resume(&self, state: Value) {
        self.count
            .store(state["count"].as_u64().unwrap(), Ordering::SeqCst);
        self.record("resume");
    }
}

struct RestoreCounter {
    descriptor: ModuleDescriptor,
    events: EventLog,
}

#[async_trait]
impl Discoverer for RestoreCounter {
    async fn discover(&self, _: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
        Ok(vec![DiscoveredModule {
            name: self.descriptor.module_id.clone(),
            source: "counter".to_owned(),
            descriptor: self.descriptor.clone(),
            module: Arc::new(Counter::new(&self.events, &Arc::new(Gate::default()))),
        }])
    }
}

async fn admission_closed(client: &APCore, module_id: &str) -> String {
    timeout(Duration::from_secs(2), async {
        loop {
            match client.call(module_id, json!({}), None, None).await {
                Err(error) if error.code == ErrorCode::ModuleNotFound => {
                    return error.wire_code().into_owned();
                }
                outcome => assert!(outcome.is_ok(), "unexpected error: {outcome:?}"),
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("module should stop accepting calls")
}

fn counter_box(events: &EventLog, gate: &Arc<Gate>) -> Box<dyn Module> {
    Box::new(Counter::new(events, gate))
}

async fn exercise_batch(
    client: &APCore,
    module_id: &str,
    other_id: &str,
    events: &EventLog,
) -> Value {
    let gate = Arc::new(Gate::default());
    let error = client
        .register_many([
            (module_id, counter_box(events, &gate)),
            (module_id, counter_box(events, &gate)),
            (other_id, counter_box(events, &gate)),
        ])
        .unwrap_err();
    let registered = client.list_modules(None, None);
    let removed = client
        .unregister_many([module_id, other_id, module_id])
        .await
        .unwrap();
    json!({"batch_error": error.wire_code(), "registered": registered, "removed_items": removed})
}

async fn exercise_batch_drain(
    client: Arc<APCore>,
    module_id: &str,
    other_id: &str,
    events: &EventLog,
) -> Value {
    let gate = Arc::new(Gate::default());
    let first: Box<dyn Module> = Box::new(Counter {
        label: Some("first"),
        ..Counter::new(events, &gate)
    });
    let second: Box<dyn Module> = Box::new(Counter {
        label: Some("second"),
        ..Counter::new(events, &Arc::new(Gate::default()))
    });
    client
        .register_many([(module_id, first), (other_id, second)])
        .unwrap();
    let owner = Arc::clone(&client);
    let id = module_id.to_owned();
    let active =
        tokio::spawn(async move { owner.call(&id, json!({"hold": true}), None, None).await });
    timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    let owner = Arc::clone(&client);
    let ids = [module_id.to_owned(), other_id.to_owned()];
    let batch = tokio::spawn(async move { owner.unregister_many(ids).await });
    let new_call_error = admission_closed(&client, module_id).await;
    let other_output = client.call(other_id, json!({}), None, None).await.unwrap();
    assert!(!batch.is_finished());
    gate.release.notify_one();
    let active_output = active.await.unwrap().unwrap();
    let removed = timeout(Duration::from_secs(2), batch)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    json!({
        "new_call_error": new_call_error,
        "other_output_during_drain": other_output,
        "active_output": active_output,
        "removed_items": removed
    })
}

async fn exercise(
    client: Arc<APCore>,
    module_id: &str,
    operation: &str,
    events: &EventLog,
) -> Value {
    let gate = Arc::new(Gate::default());
    client
        .register(module_id, counter_box(events, &gate))
        .unwrap();
    let owner = Arc::clone(&client);
    let id = module_id.to_owned();
    let active =
        tokio::spawn(async move { owner.call(&id, json!({"hold": true}), None, None).await });
    timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();

    if operation == "unregister" {
        let removed = client.registry().unregister(module_id).unwrap();
        let new_call_error = admission_closed(&client, module_id).await;
        gate.release.notify_one();
        let active_output = active.await.unwrap().unwrap();
        return json!({"removed": removed, "new_call_error": new_call_error, "active_output": active_output});
    }
    if operation == "shutdown" {
        let owner = Arc::clone(&client);
        let shutdown = tokio::spawn(async move { owner.shutdown().await });
        let new_call_error = admission_closed(&client, module_id).await;
        assert!(!shutdown.is_finished());
        gate.release.notify_one();
        let active_output = active.await.unwrap().unwrap();
        timeout(Duration::from_secs(2), shutdown)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        client.shutdown().await.unwrap();
        return json!({"new_call_error": new_call_error, "active_output": active_output});
    }

    let descriptor = client
        .registry()
        .get_definition(module_id)
        .unwrap()
        .unwrap();
    client.registry().set_discoverer(Box::new(RestoreCounter {
        descriptor,
        events: Arc::clone(events),
    }));
    let mut args = json!({"reason": "conformance"});
    match operation {
        "reload" => args["module_id"] = json!(module_id),
        "reload_bulk" => args["path_filter"] = json!(module_id),
        _ => panic!("unknown lifecycle operation {operation}"),
    }
    let owner = Arc::clone(&client);
    let reload = tokio::spawn(async move {
        owner
            .call("system.control.reload_module", args, None, None)
            .await
    });
    admission_closed(&client, module_id).await;
    gate.release.notify_one();
    let active_output = active.await.unwrap().unwrap();
    let result = timeout(Duration::from_secs(2), reload)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let next_output = client.call(module_id, json!({}), None, None).await.unwrap();
    json!({"active_output": active_output, "reload_succeeded": result["success"], "next_output": next_output})
}

async fn exercise_control_guard(client: &APCore, case: &Value, events: &EventLog) -> Value {
    let module_id = case["module_id"].as_str().unwrap();
    client
        .register(
            "aaa.counter",
            counter_box(events, &Arc::new(Gate::default())),
        )
        .unwrap();
    let callback_events = Arc::clone(events);
    client
        .registry()
        .on(
            "unregister",
            Box::new(move |id, _| {
                callback_events
                    .lock()
                    .unwrap()
                    .push(format!("unregister:{id}"))
            }),
        )
        .unwrap();
    let registered = client.list_modules(None, None);
    let (mut inputs, mut followup) = if case["operation"] == "toggle_self" {
        (
            json!({"module_id": module_id, "enabled": false}),
            json!({"module_id": module_id, "enabled": true}),
        )
    } else {
        (
            case["selector"].clone(),
            json!({"path_filter": "missing.module"}),
        )
    };
    inputs["reason"] = json!("conformance");
    followup["reason"] = json!("conformance");
    let failure = client
        .call(module_id, inputs, None, None)
        .await
        .unwrap_err();
    let preserved = client.list_modules(None, None) == registered;
    let next = client.call(module_id, followup, None, None).await.unwrap();
    json!({
        "control_error": failure.wire_code(),
        "registrations_preserved": preserved,
        "control_usable": next["success"],
    })
}

async fn begin_held_call(
    client: &Arc<APCore>,
    id: &str,
    gate: &Gate,
) -> tokio::task::JoinHandle<Result<Value, ModuleError>> {
    let owner = Arc::clone(client);
    let id = id.to_owned();
    let active =
        tokio::spawn(async move { owner.call(&id, json!({"hold": true}), None, None).await });
    timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    active
}

async fn drain_expired(operation: &LifecycleOperation) -> Value {
    timeout(Duration::from_secs(2), async {
        loop {
            let report = json!(operation.snapshot());
            if report["modules"][0]["drain_timed_out"] == true {
                return report;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("operation should record its drain deadline")
}

async fn final_report(operation: &LifecycleOperation) -> Value {
    json!(timeout(Duration::from_secs(2), operation.wait(None))
        .await
        .unwrap()
        .unwrap())
}

fn outcomes(report: &Value) -> Vec<Value> {
    report["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["outcome"].clone())
        .collect()
}

async fn exercise_managed_shutdown_waiter(
    client: Arc<APCore>,
    case: &Value,
    events: &EventLog,
) -> Value {
    let id = case["module_id"].as_str().unwrap();
    let gate = Arc::new(Gate::default());
    client.register(id, counter_box(events, &gate)).unwrap();
    let active = begin_held_call(&client, id, &gate).await;
    let timeout_ms = case["drain_timeout_ms"]
        .as_u64()
        .unwrap()
        .try_into()
        .unwrap();
    let operation = client
        .start_shutdown(Some(ShutdownOptions {
            drain_timeout_ms: Some(timeout_ms),
            ..Default::default()
        }))
        .unwrap();
    let abandoned = tokio::spawn({
        let operation = operation.clone();
        async move { operation.wait(None).await }
    });
    tokio::task::yield_now().await;
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());
    let new_call_error = admission_closed(&client, id).await;
    let wait_ms = case["wait_timeout_ms"]
        .as_u64()
        .unwrap()
        .try_into()
        .unwrap();
    let waiter = json!(operation.wait(Some(wait_ms)).await.unwrap());
    let drain = drain_expired(&operation).await;
    assert_eq!(drain["modules"][0]["phase"], "draining");
    gate.release.notify_one();
    let active_output = active.await.unwrap().unwrap();
    timeout(Duration::from_secs(2), async {
        while !events.lock().unwrap().contains(&"unload".to_owned()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner should finish without a waiter");
    let joined = client.start_shutdown(None).unwrap();
    let report = final_report(&joined).await;
    json!({
        "wait_status": waiter["status"],
        "drain_timed_out": drain["modules"][0]["drain_timed_out"],
        "new_call_error": new_call_error,
        "active_output": active_output,
        "same_operation": report["operation_id"] == waiter["operation_id"],
        "final_status": report["status"],
        "outcomes": outcomes(&report)
    })
}

async fn exercise_managed_shutdown_shared(
    client: &APCore,
    case: &Value,
    events: &EventLog,
) -> Value {
    let id = case["module_id"].as_str().unwrap();
    client
        .register(id, counter_box(events, &Arc::new(Gate::default())))
        .unwrap();
    let peer = APCore::with_options(
        None,
        Some(Executor::new(client.registry_arc(), Config::default())),
        None,
        None,
    );
    let timeout_ms: u32 = case["drain_timeout_ms"]
        .as_u64()
        .unwrap()
        .try_into()
        .unwrap();
    let options = ShutdownOptions {
        drain_timeout_ms: Some(timeout_ms),
        ..Default::default()
    };
    let operation = client.start_shutdown(Some(options)).unwrap();
    let joined = peer.start_shutdown(Some(options)).unwrap();
    let conflict = peer
        .start_shutdown(Some(ShutdownOptions {
            drain_timeout_ms: Some(timeout_ms + 1),
            ..options
        }))
        .err()
        .expect("conflicting Registry policy");
    let report = final_report(&joined).await;
    let original = final_report(&operation).await;
    json!({
        "same_operation": report["operation_id"] == original["operation_id"],
        "conflict_error": conflict.wire_code(),
        "final_status": report["status"],
        "outcomes": outcomes(&report)
    })
}

async fn exercise_managed_batch(client: Arc<APCore>, case: &Value, events: &EventLog) -> Value {
    let id = case["module_id"].as_str().unwrap();
    let other = case["other_module_id"].as_str().unwrap();
    let gate = Arc::new(Gate::default());
    for (module_id, label) in [(id, "first"), (other, "second")] {
        client
            .register(
                module_id,
                Box::new(Counter {
                    label: Some(label),
                    ..Counter::new(events, &gate)
                }),
            )
            .unwrap();
    }
    let active = begin_held_call(&client, id, &gate).await;
    let deadline = case["operation_timeout_ms"]
        .as_u64()
        .map(|value| value.try_into().unwrap());
    let operation = client
        .start_unregister_many(
            [id, other],
            Some(UnregisterOptions {
                operation_timeout_ms: deadline,
                ..Default::default()
            }),
        )
        .unwrap();
    admission_closed(&client, id).await;
    let mut actual = if deadline.is_some() {
        let waiter = json!(operation.wait(Some(0)).await.unwrap());
        assert_eq!(waiter["modules"][1]["phase"], "pending");
        let output = client.call(other, json!({}), None, None).await.unwrap();
        let drain = drain_expired(&operation).await;
        json!({
            "wait_status": waiter["status"],
            "drain_timed_out": drain["modules"][0]["drain_timed_out"],
            "other_output_during_drain": output
        })
    } else {
        assert!(client.registry().unregister(other).unwrap());
        client
            .register(
                other,
                Box::new(Counter {
                    label: Some("replacement"),
                    ..Counter::new(events, &gate)
                }),
            )
            .unwrap();
        json!({})
    };
    gate.release.notify_one();
    let active_output = active.await.unwrap().unwrap();
    let report = final_report(&operation).await;
    actual["final_status"] = report["status"].clone();
    actual["outcomes"] = json!(outcomes(&report));
    let other_output = client.call(other, json!({}), None, None).await.unwrap();
    if deadline.is_some() {
        actual["active_output"] = active_output;
        actual["stop_reason"] = report["stop_reason"].clone();
        actual["other_output_after"] = other_output;
    } else {
        actual["replacement_output"] = other_output;
    }
    actual
}

#[tokio::test]
async fn managed_reload_timeout_restores_the_same_live_registration() {
    let mut config = Config::from_defaults();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let client = Arc::new(
        APCore::try_start(APCoreOptions {
            config: Some(config),
            approval_handler: Some(Box::new(AutoApproveHandler)),
            ..Default::default()
        })
        .unwrap(),
    );
    let id = "lifecycle.counter";
    let gate = Arc::new(Gate::default());
    let events = Arc::new(Mutex::new(Vec::new()));
    client.register(id, counter_box(&events, &gate)).unwrap();
    let active = begin_held_call(&client, id, &gate).await;
    let outcome = timeout(
        Duration::from_millis(500),
        client.call(
            "system.control.reload_module",
            json!({"module_id": id, "reason": "test", "lifecycle": {"drain_timeout_ms": 0}}),
            None,
            None,
        ),
    )
    .await;
    gate.release.notify_one();
    assert_eq!(active.await.unwrap().unwrap()["count"], 1);
    let outcome = outcome
        .expect("managed abort should settle without waiting for the live invocation")
        .unwrap();
    assert_eq!(outcome["success"], false);
    assert_eq!(
        outcome["lifecycle_report"]["modules"][0]["outcome"],
        "retained_ready"
    );
    assert_eq!(
        outcome["lifecycle_report"]["modules"][0]["abort_reason"],
        "drain_timeout"
    );
    assert_eq!(
        client.call(id, json!({}), None, None).await.unwrap()["count"],
        1
    );
    assert_eq!(&*events.lock().unwrap(), &["finish"]);
}

#[derive(Clone, Copy)]
enum ManagedBehavior {
    ReleaseCall,
    ReleaseStream,
    HoldRecovery,
    DrainFails,
    Replacement,
    ReplacementFails,
    Cancelable,
}

struct ManagedModule {
    counter: Counter,
    behavior: ManagedBehavior,
    hook_gate: Arc<Gate>,
}

#[async_trait]
impl Module for ManagedModule {
    fn description(&self) -> &str {
        self.counter.description()
    }
    fn input_schema(&self) -> Value {
        self.counter.input_schema()
    }
    fn output_schema(&self) -> Value {
        self.counter.output_schema()
    }
    fn as_drainable(&self) -> Option<&dyn apcore::DrainableModule> {
        matches!(
            self.behavior,
            ManagedBehavior::ReleaseCall
                | ManagedBehavior::ReleaseStream
                | ManagedBehavior::HoldRecovery
                | ManagedBehavior::DrainFails
        )
        .then_some(self)
    }
    fn as_async_lifecycle(&self) -> Option<&dyn apcore::AsyncLifecycleModule> {
        matches!(self.behavior, ManagedBehavior::ReplacementFails).then_some(self)
    }
    fn as_streaming(&self) -> Option<&dyn StreamingModule> {
        matches!(self.behavior, ManagedBehavior::ReleaseStream).then_some(self)
    }
    fn on_load(&self) -> Result<(), ModuleError> {
        if matches!(self.behavior, ManagedBehavior::Replacement) {
            self.counter.record("load");
        }
        Ok(())
    }
    fn on_suspend(&self) -> Option<Value> {
        self.counter.on_suspend()
    }
    fn on_unload(&self) {
        self.counter.on_unload();
    }
    fn on_resume(&self, state: Value) {
        self.counter.on_resume(state);
    }

    async fn execute(&self, inputs: Value, context: &Context<Value>) -> Result<Value, ModuleError> {
        if matches!(self.behavior, ManagedBehavior::Cancelable) {
            let token = context.cancel_token.as_ref().unwrap();
            self.counter.gate.entered.notify_one();
            token.cancelled().await;
            token.check()?;
        }
        self.counter.execute(inputs, context).await
    }
}

impl StreamingModule for ManagedModule {
    fn stream_typed(&self, _: Value, _: &Context<Value>) -> ChunkStream {
        let gate = Arc::clone(&self.counter.gate);
        let count = Arc::clone(&self.counter.count);
        let events = Arc::clone(&self.counter.events);
        Box::pin(async_stream::stream! {
            gate.entered.notify_one();
            gate.release.notified().await;
            let value = count.fetch_add(1, Ordering::SeqCst) + 1;
            yield Ok(json!({"count": value}));
            events.lock().unwrap().push("stream_finished".to_owned());
        })
    }
}

#[async_trait]
impl apcore::DrainableModule for ManagedModule {
    async fn on_drain(&self, _: &apcore::DrainContext) -> Result<(), ModuleError> {
        self.counter.record("drain");
        match self.behavior {
            ManagedBehavior::ReleaseCall | ManagedBehavior::ReleaseStream => {
                self.counter.gate.release.notify_one();
            }
            ManagedBehavior::DrainFails => {
                return Err(ModuleError::new(
                    ErrorCode::ModuleLoadError,
                    "private lifecycle failure",
                ))
            }
            _ => {}
        }
        Ok(())
    }

    async fn on_drain_aborted(&self, _: &apcore::DrainAbortedContext) -> Result<(), ModuleError> {
        self.counter.record("drain_aborted");
        if matches!(self.behavior, ManagedBehavior::HoldRecovery) {
            self.hook_gate.entered.notify_one();
            self.hook_gate.release.notified().await;
        }
        Ok(())
    }
}

#[async_trait]
impl apcore::AsyncLifecycleModule for ManagedModule {
    async fn on_load_async(&self) -> Result<(), ModuleError> {
        self.hook_gate.entered.notify_one();
        self.hook_gate.release.notified().await;
        Err(ModuleError::new(
            ErrorCode::ModuleLoadError,
            "private replacement failure",
        ))
    }
}

struct Replacement {
    descriptor: ModuleDescriptor,
    module: Arc<dyn Module>,
}

#[async_trait]
impl Discoverer for Replacement {
    async fn discover(&self, _: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
        Ok(vec![DiscoveredModule {
            name: self.descriptor.module_id.clone(),
            source: "replacement".to_owned(),
            descriptor: self.descriptor.clone(),
            module: Arc::clone(&self.module),
        }])
    }
}

fn reload_validator() -> jsonschema::Validator {
    let schema_dir = find_fixtures_root()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("schemas");
    let json: Value = serde_json::from_slice(
        &std::fs::read(schema_dir.join("sys-control-reload-module.schema.json")).unwrap(),
    )
    .unwrap();
    jsonschema::draft202012::new(&json).unwrap()
}

async fn run_managed_reload(
    client: Arc<APCore>,
    case: &Value,
    events: &EventLog,
    validator: &jsonschema::Validator,
) -> Value {
    let observations = LifecycleObservations::default();
    let notifications = Arc::clone(&observations.0);
    client.events().unwrap().subscribe(Box::new(observations));
    let operation = case["operation"].as_str().unwrap();
    let id = case["module_id"].as_str().unwrap();
    let gate = Arc::new(Gate::default());
    let hook_gate = Arc::new(Gate::default());
    let behavior = match operation {
        "managed_reload_drain" => ManagedBehavior::ReleaseCall,
        "managed_reload_stream" => ManagedBehavior::ReleaseStream,
        "managed_reload_abort" => ManagedBehavior::HoldRecovery,
        "managed_reload_hook_failure" => ManagedBehavior::DrainFails,
        _ => ManagedBehavior::Replacement,
    };
    if operation != "managed_reload_no_match" {
        client
            .register(
                id,
                Box::new(ManagedModule {
                    counter: Counter::new(events, &gate),
                    behavior,
                    hook_gate: Arc::clone(&hook_gate),
                }),
            )
            .unwrap();
    }
    if matches!(
        operation,
        "managed_reload_drain" | "managed_reload_stream" | "managed_reload_replacement_failure"
    ) {
        let descriptor = client.registry().get_definition(id).unwrap().unwrap();
        let behavior = if operation == "managed_reload_replacement_failure" {
            ManagedBehavior::ReplacementFails
        } else {
            ManagedBehavior::Replacement
        };
        client.registry().set_discoverer(Box::new(Replacement {
            descriptor,
            module: Arc::new(ManagedModule {
                counter: Counter::new(events, &gate),
                behavior,
                hook_gate: Arc::clone(&hook_gate),
            }),
        }));
    }

    let active = if matches!(operation, "managed_reload_drain" | "managed_reload_abort") {
        Some(begin_held_call(&client, id, &gate).await)
    } else {
        None
    };
    let streaming = if operation == "managed_reload_stream" {
        let requester = Arc::clone(&client);
        let target = id.to_owned();
        let stream = tokio::spawn(async move {
            requester
                .stream(&target, json!({}), None, None)
                .map(Result::unwrap)
                .collect::<Vec<_>>()
                .await
        });
        timeout(Duration::from_secs(2), gate.entered.notified())
            .await
            .unwrap();
        Some(stream)
    } else {
        None
    };
    let mut reload_args = json!({"reason": "conformance", "lifecycle": {}});
    if operation == "managed_reload_no_match" {
        reload_args["path_filter"] = case["path_filter"].clone();
    } else {
        reload_args["module_id"] = json!(id);
    }
    if let Some(value) = case.get("drain_timeout_ms") {
        reload_args["lifecycle"]["drain_timeout_ms"] = value.clone();
    }
    let owner = Arc::clone(&client);
    let reload = tokio::spawn(async move {
        owner
            .call("system.control.reload_module", reload_args, None, None)
            .await
    });
    let guarded = matches!(
        operation,
        "managed_reload_abort" | "managed_reload_replacement_failure"
    );
    let new_call_error = if guarded {
        timeout(Duration::from_secs(2), hook_gate.entered.notified())
            .await
            .unwrap();
        let code = admission_closed(&client, id).await;
        hook_gate.release.notify_one();
        Some(code)
    } else {
        None
    };
    let result = timeout(Duration::from_secs(2), reload)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    validator.validate(&result).unwrap();
    let report = &result["lifecycle_report"];
    let mut actual = json!({
        "reload_succeeded": result["success"],
        "report_status": report["status"],
        "outcomes": outcomes(report),
    });
    if let Some(active) = active {
        if operation == "managed_reload_abort" {
            gate.release.notify_one();
        }
        actual["active_output"] = active.await.unwrap().unwrap();
    }
    if let Some(streaming) = streaming {
        actual["chunks"] = json!(timeout(Duration::from_secs(2), streaming)
            .await
            .unwrap()
            .unwrap());
    }
    if operation != "managed_reload_no_match" {
        actual["next_output"] = client.call(id, json!({}), None, None).await.unwrap();
    } else {
        actual["report_kind"] = report["kind"].clone();
        actual["event_flush"] = report["event_flush"].clone();
        actual["operation_id_present"] = json!(report["operation_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()));
        actual["reloaded_modules"] = result["reloaded_modules"].clone();
    }
    if let Some(code) = new_call_error {
        actual["new_call_error"] = json!(code);
    }
    if matches!(
        operation,
        "managed_reload_abort" | "managed_reload_hook_failure"
    ) {
        actual["abort_reason"] = report["modules"][0]["abort_reason"].clone();
    }
    if operation == "managed_reload_replacement_failure" {
        actual["compensation"] = report["modules"][0]["compensation"].clone();
        actual["primary_error"] = report["modules"][0]["error"]["code"].clone();
        client.events().unwrap().flush_default().await.unwrap();
        let notifications = notifications.lock().unwrap();
        assert!(notifications.iter().any(|event| {
            event.event_type == "apcore.registry.module_reload_failed"
                && event.data["operation_id"] == report["operation_id"]
        }));
        assert!(notifications.iter().all(|event| {
            !event
                .data
                .to_string()
                .contains("private replacement failure")
        }));
    }
    actual
}

async fn run_managed_reload_denied(client: &APCore, case: &Value, events: &EventLog) -> Value {
    let id = case["module_id"].as_str().unwrap();
    let gate = Arc::new(Gate::default());
    let counter = Counter::new(events, &gate);
    counter.count.store(1, Ordering::SeqCst);
    client
        .register(
            id,
            Box::new(ManagedModule {
                counter,
                behavior: ManagedBehavior::ReleaseCall,
                hook_gate: gate,
            }),
        )
        .unwrap();
    let observer = LifecycleObservations::default();
    let notifications = Arc::clone(&observer.0);
    let emitter = client.events().unwrap();
    emitter.flush_default().await.unwrap();
    emitter.subscribe(Box::new(observer));
    let error = client
        .call(
            "system.control.reload_module",
            json!({"module_id": id, "reason": "conformance", "lifecycle": {
                "drain_timeout_ms": case["drain_timeout_ms"],
            }}),
            None,
            None,
        )
        .await
        .unwrap_err();
    let next_output = client.call(id, json!({}), None, None).await.unwrap();
    emitter.flush_default().await.unwrap();
    let lifecycle_event_count = notifications.lock().unwrap().len();
    json!({
        "control_error": error.wire_code(), "next_output": next_output,
        "lifecycle_event_count": lifecycle_event_count,
    })
}

async fn run_managed_force(client: Arc<APCore>, case: &Value, events: &EventLog) -> Value {
    let target = case["module_id"].as_str().unwrap();
    let other = case["other_module_id"].as_str().unwrap();
    let target_gate = Arc::new(Gate::default());
    let other_gate = Arc::new(Gate::default());
    client
        .register(
            target,
            Box::new(ManagedModule {
                counter: Counter::new(events, &target_gate),
                behavior: ManagedBehavior::Cancelable,
                hook_gate: Arc::new(Gate::default()),
            }),
        )
        .unwrap();
    client
        .register(other, counter_box(events, &other_gate))
        .unwrap();
    let parent_token = apcore::CancelToken::new();
    let mut context = Context::anonymous();
    context.cancel_token = Some(parent_token.clone());
    let spawn_invocation = |name: &str| {
        let owner = Arc::clone(&client);
        let name = name.to_owned();
        let context = context.clone();
        tokio::spawn(async move {
            owner
                .call(&name, json!({"hold": true}), Some(&context), None)
                .await
        })
    };
    let target_call = spawn_invocation(target);
    let other_call = spawn_invocation(other);
    timeout(Duration::from_secs(2), target_gate.entered.notified())
        .await
        .unwrap();
    timeout(Duration::from_secs(2), other_gate.entered.notified())
        .await
        .unwrap();
    let operation = client
        .start_unregister_many(
            [target],
            Some(UnregisterOptions {
                drain_timeout_ms: Some(
                    case["drain_timeout_ms"]
                        .as_u64()
                        .unwrap()
                        .try_into()
                        .unwrap(),
                ),
                cancel_grace_ms: Some(
                    case["cancel_grace_ms"]
                        .as_u64()
                        .unwrap()
                        .try_into()
                        .unwrap(),
                ),
                on_drain_timeout: Some(apcore::DrainTimeoutPolicy::CancelThenForce),
                ..Default::default()
            }),
        )
        .unwrap();
    let code = timeout(Duration::from_secs(2), target_call)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .wire_code()
        .into_owned();
    let report = final_report(&operation).await;
    assert!(!other_call.is_finished());
    other_gate.release.notify_one();
    let other_result = other_call.await.unwrap().unwrap();
    json!({
        "target_cancel_error": code, "parent_cancelled": parent_token.is_cancelled(),
        "other_output_after": other_result, "final_status": report["status"],
        "unload_count": events.lock().unwrap().iter().filter(|event| event.as_str() == "unload").count(),
    })
}

#[derive(Debug, Default)]
struct ShutdownObservations(Arc<Mutex<Vec<Value>>>);

#[derive(Debug, Default)]
struct LifecycleObservations(Arc<Mutex<Vec<apcore::ApCoreEvent>>>);

#[async_trait]
impl apcore::EventSubscriber for LifecycleObservations {
    fn event_pattern(&self) -> &str {
        "apcore.registry.*"
    }

    async fn on_event(&self, event: &apcore::ApCoreEvent) -> Result<(), ModuleError> {
        self.0.lock().unwrap().push(event.clone());
        Ok(())
    }
}

#[async_trait]
impl apcore::EventSubscriber for ShutdownObservations {
    fn event_pattern(&self) -> &str {
        "apcore.client.shutdown_resources_completed"
    }
    async fn on_event(&self, event: &apcore::ApCoreEvent) -> Result<(), ModuleError> {
        self.0.lock().unwrap().push(event.data.clone());
        Ok(())
    }
}

async fn run_shutdown_event(client: &APCore, case: &Value, events: &EventLog) -> Value {
    let observer = ShutdownObservations::default();
    let observed = Arc::clone(&observer.0);
    client.events().unwrap().subscribe(Box::new(observer));
    let id = case["module_id"].as_str().unwrap();
    client
        .register(id, counter_box(events, &Arc::new(Gate::default())))
        .unwrap();
    let operation = client.start_shutdown(None).unwrap();
    let report = final_report(&operation).await;
    let repeated = final_report(&client.start_shutdown(None).unwrap()).await;
    assert_eq!(report, repeated);
    let deliveries = observed.lock().unwrap();
    json!({
        "final_status": report["status"], "event_flush": report["event_flush"],
        "event_count": deliveries.len(),
        "event_matches_operation": deliveries.first().is_some_and(|event| event["operation_id"] == report["operation_id"]),
    })
}

#[tokio::test]
async fn conformance_module_lifecycle() {
    let source = std::fs::read(find_fixtures_root().join("module_lifecycle.json")).unwrap();
    let fixture: Value = serde_json::from_slice(&source).unwrap();
    let validator = reload_validator();
    for case in fixture["test_cases"].as_array().unwrap() {
        let module_id = case["module_id"].as_str().unwrap();
        let operation = case["operation"].as_str().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut config = Config::from_defaults();
        let controls = matches!(
            operation,
            "reload" | "reload_bulk" | "toggle_self" | "reload_self" | "managed_shutdown_event"
        ) || operation.starts_with("managed_reload_");
        config.set("sys_modules.enabled", json!(controls));
        config.set("sys_modules.events.enabled", json!(controls));
        let approval_handler: Box<dyn ApprovalHandler> = if operation == "managed_reload_denied" {
            Box::new(AlwaysDenyHandler)
        } else {
            Box::new(AutoApproveHandler)
        };
        let client = Arc::new(
            APCore::try_start(APCoreOptions {
                config: Some(config),
                approval_handler: Some(approval_handler),
                ..APCoreOptions::default()
            })
            .unwrap(),
        );
        let mut actual = match operation {
            "batch" => {
                exercise_batch(
                    &client,
                    module_id,
                    case["other_module_id"].as_str().unwrap(),
                    &events,
                )
                .await
            }
            "batch_drain" => {
                exercise_batch_drain(
                    client,
                    module_id,
                    case["other_module_id"].as_str().unwrap(),
                    &events,
                )
                .await
            }
            "toggle_self" | "reload_self" => exercise_control_guard(&client, case, &events).await,
            "managed_shutdown_waiter" => {
                exercise_managed_shutdown_waiter(client, case, &events).await
            }
            "managed_shutdown_shared" => {
                exercise_managed_shutdown_shared(&client, case, &events).await
            }
            "managed_batch_budget" | "managed_batch_stale_generation" => {
                exercise_managed_batch(client, case, &events).await
            }
            "managed_force_scope" => run_managed_force(client, case, &events).await,
            "managed_shutdown_event" => run_shutdown_event(&client, case, &events).await,
            "managed_reload_denied" => run_managed_reload_denied(&client, case, &events).await,
            operation if operation.starts_with("managed_reload_") => {
                run_managed_reload(client, case, &events, &validator).await
            }
            _ => exercise(client, module_id, operation, &events).await,
        };
        let expected = case["expected"].as_object().unwrap();
        if expected.contains_key("events") {
            actual["events"] = json!(&*events.lock().unwrap());
        }
        for (key, want) in expected {
            assert_eq!(&actual[key], want, "case {} field {key}", case["id"]);
        }
        assert_eq!(actual, case["expected"], "case {}", case["id"]);
    }
}

fn event_only_client(registry: Arc<apcore::Registry>) -> APCore {
    let mut config = Config::from_defaults();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    for group in ["health", "manifest", "usage", "control"] {
        config.set(&format!("sys_modules.{group}.enabled"), json!(false));
    }
    APCore::with_options(
        None,
        Some(Executor::new(registry, config.clone())),
        Some(config),
        None,
    )
}

#[tokio::test]
async fn shared_registry_emits_one_resources_event_per_participating_client() {
    let registry = Arc::new(apcore::Registry::new());
    let first = event_only_client(Arc::clone(&registry));
    let second = event_only_client(registry);
    let destinations: Vec<_> = [&first, &second]
        .into_iter()
        .map(|client| {
            let observer = ShutdownObservations::default();
            let observed = Arc::clone(&observer.0);
            client.events().unwrap().subscribe(Box::new(observer));
            observed
        })
        .collect();
    let first_report = final_report(&first.start_shutdown(None).unwrap()).await;
    let second_report = final_report(&second.start_shutdown(None).unwrap()).await;
    assert_eq!(first_report["operation_id"], second_report["operation_id"]);
    for events in destinations {
        let deliveries = events.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0]["operation_id"], first_report["operation_id"]);
        assert_eq!(deliveries[0]["registry_outcome"], "completed");
    }
}

struct AsyncProbe {
    loading: Arc<Gate>,
    unloading: Arc<Gate>,
}

#[async_trait]
impl Module for AsyncProbe {
    fn description(&self) -> &str {
        "Async lifecycle probe"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn as_async_lifecycle(&self) -> Option<&dyn apcore::AsyncLifecycleModule> {
        Some(self)
    }
    fn on_load(&self) -> Result<(), ModuleError> {
        panic!("sync load selected for an async adapter")
    }
    fn on_unload(&self) {
        panic!("sync unload selected for an async adapter");
    }
    async fn execute(&self, _: Value, _: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

#[async_trait]
impl apcore::AsyncLifecycleModule for AsyncProbe {
    async fn on_load_async(&self) -> Result<(), ModuleError> {
        self.loading.entered.notify_one();
        self.loading.release.notified().await;
        Ok(())
    }
    async fn on_unload_async(&self) -> Result<(), ModuleError> {
        self.unloading.entered.notify_one();
        self.unloading.release.notified().await;
        Ok(())
    }
}

#[tokio::test]
async fn async_registration_outlives_its_waiter_and_ordinary_unload_is_joinable() {
    let client = Arc::new(APCore::new());
    let loading = Arc::new(Gate::default());
    let unloading = Arc::new(Gate::default());
    let new_probe = || {
        Box::new(AsyncProbe {
            loading: Arc::clone(&loading),
            unloading: Arc::clone(&unloading),
        })
    };
    let id = "lifecycle.async_probe";
    let error = client.register(id, new_probe()).unwrap_err();
    assert_eq!(error.code, ErrorCode::ModuleLoadError);
    let waiter = tokio::spawn({
        let client = Arc::clone(&client);
        let probe = new_probe();
        async move { client.register_async(id, probe).await }
    });
    timeout(Duration::from_secs(2), loading.entered.notified())
        .await
        .unwrap();
    assert!(!client.registry().has(id));
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    loading.release.notify_one();
    timeout(Duration::from_secs(2), async {
        while !client.registry().has(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(client.registry().unregister(id).unwrap());
    assert!(!client.registry().has(id));
    timeout(Duration::from_secs(2), unloading.entered.notified())
        .await
        .unwrap();
    let operation = client.start_shutdown(None).unwrap();
    assert_eq!(
        operation.wait(Some(1)).await.unwrap().status,
        apcore::LifecycleStatus::Pending
    );
    unloading.release.notify_one();
    let report = final_report(&operation).await;
    assert_eq!(report["status"], "completed");
    assert_eq!(report["modules"][0]["outcome"], "already_removing");
}

#[tokio::test]
async fn safe_unregister_cleanup_outlives_its_waiter() {
    let client = APCore::new();
    let loading = Arc::new(Gate::default());
    let unloading = Arc::new(Gate::default());
    let id = "lifecycle.async_probe";
    loading.release.notify_one();
    client
        .register_async(
            id,
            Box::new(AsyncProbe {
                loading,
                unloading: Arc::clone(&unloading),
            }),
        )
        .await
        .unwrap();

    let registry = client.registry_arc();
    let waiter = tokio::spawn(async move { registry.safe_unregister(id, 500).await });
    timeout(Duration::from_secs(2), unloading.entered.notified())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let shutdown = client.start_shutdown(None).unwrap();
    assert_eq!(
        shutdown.wait(Some(5)).await.unwrap().status,
        apcore::LifecycleStatus::Pending
    );
    unloading.release.notify_one();
    assert_eq!(
        final_report(&shutdown).await["modules"][0]["outcome"],
        "already_removing"
    );
}

#[tokio::test]
async fn legacy_reload_finishes_replacement_after_waiter_leaves_async_unload() {
    let client = APCore::new();
    let registry = client.registry_arc();
    let loading = Arc::new(Gate::default());
    let unloading = Arc::new(Gate::default());
    let events = Arc::new(Mutex::new(Vec::new()));
    let id = "lifecycle.async_probe";
    loading.release.notify_one();
    client
        .register_async(
            id,
            Box::new(AsyncProbe {
                loading,
                unloading: Arc::clone(&unloading),
            }),
        )
        .await
        .unwrap();
    registry.set_discoverer(Box::new(Replacement {
        descriptor: registry.get_definition(id).unwrap().unwrap(),
        module: Arc::new(Counter::new(&events, &Arc::new(Gate::default()))),
    }));
    let reload = apcore::sys_modules::control::ReloadModule::new(
        Arc::clone(&registry),
        Arc::new(apcore::EventEmitter::new()),
    );
    let waiter = tokio::spawn(async move {
        reload
            .execute(
                json!({"module_id":id,"reason":"review"}),
                &Context::anonymous(),
            )
            .await
    });
    timeout(Duration::from_secs(2), unloading.entered.notified())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    unloading.release.notify_one();
    timeout(Duration::from_secs(2), async {
        while !registry.has(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn ordinary_async_cleanup_survives_the_registration_runtime() {
    let client = APCore::new();
    let loading = Arc::new(Gate::default());
    let unloading = Arc::new(Gate::default());
    let id = "lifecycle.async_probe";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    loading.release.notify_one();
    runtime
        .block_on(client.register_async(
            id,
            Box::new(AsyncProbe {
                loading,
                unloading: Arc::clone(&unloading),
            }),
        ))
        .unwrap();
    drop(runtime);
    assert!(client.registry().unregister(id).unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        timeout(Duration::from_secs(2), unloading.entered.notified())
            .await
            .unwrap();
        let shutdown = client.start_shutdown(None).unwrap();
        unloading.release.notify_one();
        assert_eq!(
            final_report(&shutdown).await["modules"][0]["outcome"],
            "already_removing"
        );
    });
}

#[test]
fn lifecycle_budgets_use_timers_on_a_host_runtime_without_time() {
    let client = APCore::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(Gate::default());
    let id = "lifecycle.counter";
    client.register(id, counter_box(&events, &gate)).unwrap();
    let registry = client.registry();
    let _lease = registry.acquire(id).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let shutdown = client
            .start_shutdown(Some(ShutdownOptions {
                drain_timeout_ms: Some(100),
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(
            shutdown.wait(Some(5)).await.unwrap().status,
            apcore::LifecycleStatus::Pending
        );
        registry.release(id);
        assert_eq!(
            shutdown.wait(Some(1000)).await.unwrap().status,
            apcore::LifecycleStatus::Completed
        );
    });
}

struct RetiringStream {
    gate: Arc<Gate>,
    cleanup: Arc<Gate>,
    counter: Counter,
}

#[async_trait]
impl Module for RetiringStream {
    fn description(&self) -> &str {
        self.counter.description()
    }
    fn input_schema(&self) -> Value {
        self.counter.input_schema()
    }
    fn output_schema(&self) -> Value {
        self.counter.output_schema()
    }

    fn on_unload(&self) {
        self.counter.on_unload();
    }

    async fn execute(&self, _: Value, _: &Context<Value>) -> Result<Value, ModuleError> {
        unreachable!("use the native streaming adapter")
    }

    fn stream(&self, _: Value, context: &Context<Value>) -> Option<apcore::ChunkStream> {
        let token = context.cancel_token.clone().unwrap();
        let stream = Arc::clone(&self.gate);
        let cleanup = Arc::clone(&self.cleanup);
        Some(Box::pin(async_stream::stream! {
            stream.entered.notify_one();
            token.cancelled().await;
            cleanup.entered.notify_one();
            cleanup.release.notified().await;
            yield Ok(json!({"count":1}));
        }))
    }
}

#[tokio::test]
async fn retiring_native_stream_keeps_its_lease_during_cooperative_cleanup() {
    exercise_retiring_native_stream(false).await;
}

#[tokio::test]
async fn forced_native_stream_removal_keeps_tracking_late_cleanup() {
    exercise_retiring_native_stream(true).await;
}

async fn exercise_retiring_native_stream(force: bool) {
    let client = Arc::new(APCore::new());
    let events = Arc::new(Mutex::new(Vec::new()));
    let stream = Arc::new(Gate::default());
    let cleanup = Arc::new(Gate::default());
    let id = "lifecycle.native_stream";
    client
        .register(
            id,
            Box::new(RetiringStream {
                gate: Arc::clone(&stream),
                cleanup: Arc::clone(&cleanup),
                counter: Counter::new(&events, &stream),
            }),
        )
        .unwrap();
    let caller = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            let (mut chunks, completion) = client.stream_with_completion(id, json!({}), None, None);
            assert_eq!(
                chunks.next().await.unwrap().unwrap_err().code,
                ErrorCode::ExecutionCancelled
            );
            assert_eq!(completion.await.reason, apcore::StreamEndReason::Cancelled);
        }
    });
    timeout(Duration::from_secs(2), stream.entered.notified())
        .await
        .unwrap();
    let retiring = client
        .start_unregister_many(
            [id],
            Some(UnregisterOptions {
                drain_timeout_ms: Some(0),
                cancel_grace_ms: Some(if force { 0 } else { 1000 }),
                on_drain_timeout: Some(apcore::DrainTimeoutPolicy::CancelThenForce),
                ..Default::default()
            }),
        )
        .unwrap();
    timeout(Duration::from_secs(2), caller)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(2), cleanup.entered.notified())
        .await
        .unwrap();
    if force {
        let snapshot = timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = retiring.snapshot();
                if snapshot.modules[0].outcome == Some(apcore::LifecycleOutcome::ForcedRemoved) {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(snapshot.status, apcore::LifecycleStatus::Pending);
        assert_eq!(snapshot.modules[0].active_at_unload, Some(1));
        assert_eq!(snapshot.modules[0].cleanup_pending, Some(true));
        assert_eq!(&*events.lock().unwrap(), &["unload"]);
    } else {
        assert_eq!(
            retiring.wait(Some(5)).await.unwrap().status,
            apcore::LifecycleStatus::Pending
        );
        assert!(events.lock().unwrap().is_empty());
    }
    cleanup.release.notify_one();
    let report = final_report(&retiring).await;
    let row = &report["modules"][0];
    if force {
        assert_eq!(row["outcome"], "forced_removed");
        assert_eq!(row["cleanup_pending"], false);
    } else {
        assert_eq!(row["outcome"], "removed");
    }
    assert_eq!(&*events.lock().unwrap(), &["unload"]);
}

struct BlockingDiscovery {
    replacement: Replacement,
    gate: Arc<Gate>,
}

#[async_trait]
impl Discoverer for BlockingDiscovery {
    async fn discover(&self, roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
        self.gate.entered.notify_one();
        self.gate.release.notified().await;
        self.replacement.discover(roots).await
    }
}

#[tokio::test]
async fn managed_reload_reserves_id_through_unload_and_source_discovery() {
    let mut config = Config::from_defaults();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let client = Arc::new(
        APCore::try_start(APCoreOptions {
            config: Some(config),
            approval_handler: Some(Box::new(AutoApproveHandler)),
            ..Default::default()
        })
        .unwrap(),
    );
    let loading = Arc::new(Gate::default());
    let unloading = Arc::new(Gate::default());
    let discovery = Arc::new(Gate::default());
    let id = "lifecycle.probe";
    loading.release.notify_one();
    client
        .register_async(
            id,
            Box::new(AsyncProbe {
                loading,
                unloading: Arc::clone(&unloading),
            }),
        )
        .await
        .unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(Gate::default());
    client
        .registry()
        .set_discoverer(Box::new(BlockingDiscovery {
            replacement: Replacement {
                descriptor: client.registry().get_definition(id).unwrap().unwrap(),
                module: Arc::new(Counter::new(&events, &gate)),
            },
            gate: Arc::clone(&discovery),
        }));
    let reload = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            client
                .call(
                    "system.control.reload_module",
                    json!({"module_id": id, "reason": "test", "lifecycle": {}}),
                    None,
                    None,
                )
                .await
        }
    });
    timeout(Duration::from_secs(2), unloading.entered.notified())
        .await
        .unwrap();
    let during_unload = client.register(id, counter_box(&events, &gate));
    client
        .register("lifecycle.unrelated", counter_box(&events, &gate))
        .unwrap();
    unloading.release.notify_one();
    timeout(Duration::from_secs(2), discovery.entered.notified())
        .await
        .unwrap();
    let during_discovery = client.register(id, counter_box(&events, &gate));
    discovery.release.notify_one();
    let response = timeout(Duration::from_secs(2), reload)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    for rejected in [during_unload, during_discovery] {
        assert_eq!(rejected.unwrap_err().code, ErrorCode::DuplicateModuleId);
    }
    assert_eq!(response["success"], true);
    assert!(client.registry().has("lifecycle.unrelated"));
}

#[tokio::test]
async fn shutdown_joins_a_managed_replacement_after_its_wire_waiter_leaves() {
    let mut config = Config::from_defaults();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let client = Arc::new(
        APCore::try_start(APCoreOptions {
            config: Some(config),
            approval_handler: Some(Box::new(AutoApproveHandler)),
            ..Default::default()
        })
        .unwrap(),
    );
    let id = "lifecycle.probe";
    let events = Arc::new(Mutex::new(Vec::new()));
    let invocation = Arc::new(Gate::default());
    let discovery = Arc::new(Gate::default());
    client
        .register(id, counter_box(&events, &invocation))
        .unwrap();
    client
        .registry()
        .set_discoverer(Box::new(BlockingDiscovery {
            replacement: Replacement {
                descriptor: client.registry().get_definition(id).unwrap().unwrap(),
                module: Arc::new(Counter::new(&events, &invocation)),
            },
            gate: Arc::clone(&discovery),
        }));
    let waiter = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            client
                .call(
                    "system.control.reload_module",
                    json!({"module_id": id, "reason": "test", "lifecycle": {}}),
                    None,
                    None,
                )
                .await
        }
    });
    timeout(Duration::from_secs(2), discovery.entered.notified())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let operation = client.start_shutdown(None).unwrap();
    let report = operation.wait(Some(1)).await.unwrap();
    assert_eq!(report.status, apcore::LifecycleStatus::Pending);
    assert!(report
        .modules
        .iter()
        .any(|row| row.module_id == id && row.outcome.is_none()));
    discovery.release.notify_one();
    let report = final_report(&operation).await;
    assert_eq!(report["status"], "completed");
    assert!(!client.registry().has(id));
}

#[tokio::test]
async fn async_discovery_opts_into_the_async_adapter() {
    let client = APCore::new();
    let loading = Arc::new(Gate::default());
    let id = "lifecycle.async_candidate";
    let module = AsyncProbe {
        loading: Arc::clone(&loading),
        unloading: Arc::new(Gate::default()),
    };
    let descriptor = serde_json::from_value(json!({
        "module_id": id, "description": module.description(),
        "input_schema": module.input_schema(), "output_schema": module.output_schema(),
    }))
    .unwrap();
    client.registry().set_discoverer(Box::new(Replacement {
        descriptor,
        module: Arc::new(module),
    }));
    assert_eq!(client.discover().await.unwrap(), 0);
    assert!(!client.registry().has(id));
    loading.release.notify_one();
    assert_eq!(client.discover_async().await.unwrap(), 1);
    assert!(client.call(id, json!({}), None, None).await.is_ok());
}
