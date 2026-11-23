//! D-112 — a failed reload restores the previous module.
//!
//! apcore-typescript re-registered the original on failure; this SDK and
//! apcore-python left it unregistered, and the contract endorsed that ("callers
//! must handle the partial state"). For a control plane that is the wrong
//! default: a failed hot-fix should not make a WORKING module disappear.
//!
//! Restoring the PREVIOUS INSTANCE was not expressible here before this change.
//! Every registration entry point takes `Box<dyn Module>` and `Registry::get`
//! hands back an `Arc`, which cannot be converted back — the same "provided but
//! uncallable" shape D-91 settled for `ExtensionManager::unregister`, reached
//! from the other side. `Registry::reinstate_internal` is the door that makes
//! the decision implementable here, and it runs the full registration path so
//! the restored module's `on_load` re-runs (rule 2).

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::events::emitter::EventEmitter;
use apcore::module::Module;
use apcore::module::ModuleAnnotations;
use apcore::registry::registry::{DiscoveredModule, Discoverer, ModuleDescriptor, Registry};
use apcore::sys_modules::control::ReloadModule;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Records its lifecycle hooks so the restore path can be observed.
#[derive(Debug)]
struct HookRecordingModule {
    tag: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    load_failures_remaining: AtomicUsize,
}

impl HookRecordingModule {
    fn new(tag: &'static str, calls: &Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            tag,
            calls: Arc::clone(calls),
            load_failures_remaining: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Module for HookRecordingModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "records lifecycle hooks"
    }
    fn on_load(&self) -> Result<(), ModuleError> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("{}:on_load", self.tag));
        // Preserve conditional decrement behavior on the minimum supported Rust version.
        let mut remaining = self.load_failures_remaining.load(Ordering::SeqCst);
        while let Some(next) = remaining.checked_sub(1) {
            match self.load_failures_remaining.compare_exchange(
                remaining,
                next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    return Err(ModuleError::new(
                        apcore::errors::ErrorCode::ModuleLoadError,
                        format!("{} refuses to load", self.tag),
                    ));
                }
                Err(actual) => remaining = actual,
            }
        }
        Ok(())
    }
    fn on_unload(&self) {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("{}:on_unload", self.tag));
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"tag": self.tag}))
    }
}

/// A discoverer that either fails, or reinstates a replacement.
struct ScriptedDiscoverer {
    fail: bool,
    replacement: Option<(String, Arc<Mutex<Vec<String>>>)>,
}

#[async_trait]
impl Discoverer for ScriptedDiscoverer {
    async fn discover(&self, _roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
        if self.fail {
            return Err(ModuleError::new(
                apcore::errors::ErrorCode::GeneralInternalError,
                "discovery is broken",
            ));
        }
        let Some((id, calls)) = &self.replacement else {
            return Ok(vec![]);
        };
        Ok(vec![DiscoveredModule {
            name: id.clone(),
            source: "test".to_string(),
            descriptor: descriptor(id),
            module: Arc::new(HookRecordingModule::new("replacement", calls)),
        }])
    }
}

fn descriptor(id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: "probe".to_string(),
        documentation: None,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: std::collections::HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

fn reload_module(registry: &Arc<Registry>) -> ReloadModule {
    ReloadModule::new(Arc::clone(registry), Arc::new(EventEmitter::new()))
}

fn ctx() -> Context<Value> {
    Context::<Value>::anonymous()
}

#[tokio::test]
async fn the_original_instance_is_back_after_a_failed_rediscovery() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(Registry::new());
    let original: Arc<dyn Module> = Arc::new(HookRecordingModule::new("original", &calls));
    registry
        .reinstate_internal(
            "executor.probe",
            Arc::clone(&original),
            descriptor("executor.probe"),
        )
        .expect("seed the registry");
    registry.set_discoverer(Box::new(ScriptedDiscoverer {
        fail: true,
        replacement: None,
    }));

    let result = reload_module(&registry)
        .execute(
            json!({"module_id": "executor.probe", "reason": "hot-fix"}),
            &ctx(),
        )
        .await;
    assert!(
        result.is_err(),
        "a failed re-discovery must fail the reload"
    );

    let restored = registry
        .get("executor.probe")
        .expect("get")
        .expect("present");
    assert!(
        Arc::ptr_eq(&restored, &original),
        "a failed hot-fix must not make a working module disappear"
    );
}

#[tokio::test]
async fn the_restore_re_runs_on_load() {
    // Rule 2. `on_unload` already ran during the unregister, so a restore that
    // skips `on_load` republishes a module that is visible but torn down —
    // harder to diagnose than one that is absent.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(Registry::new());
    let original: Arc<dyn Module> = Arc::new(HookRecordingModule::new("original", &calls));
    registry
        .reinstate_internal("executor.probe", original, descriptor("executor.probe"))
        .expect("seed");
    calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    registry.set_discoverer(Box::new(ScriptedDiscoverer {
        fail: true,
        replacement: None,
    }));

    let _ = reload_module(&registry)
        .execute(
            json!({"module_id": "executor.probe", "reason": "hot-fix"}),
            &ctx(),
        )
        .await;

    let observed = calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        observed,
        vec![
            "original:on_unload".to_string(),
            "original:on_load".to_string()
        ],
        "the restore must re-run on_load"
    );
}

#[tokio::test]
async fn rule_3_if_the_restoring_load_also_fails_the_module_stays_unavailable() {
    // There is no good state left to return to, and publishing a module whose
    // load hook failed is the defect deferred publication exists to prevent.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(Registry::new());
    let original = Arc::new(HookRecordingModule::new("original", &calls));
    let handle = Arc::clone(&original);
    registry
        .reinstate_internal(
            "executor.probe",
            original as Arc<dyn Module>,
            descriptor("executor.probe"),
        )
        .expect("seed: the first load succeeds");
    registry.set_discoverer(Box::new(ScriptedDiscoverer {
        fail: true,
        replacement: None,
    }));

    // Arm the failure for the RESTORING load only, so the seed above is a
    // genuine "was working" starting point rather than a module that never
    // loaded.
    handle.load_failures_remaining.store(1, Ordering::SeqCst);

    let result = reload_module(&registry)
        .execute(
            json!({"module_id": "executor.probe", "reason": "hot-fix"}),
            &ctx(),
        )
        .await;

    assert!(result.is_err(), "the reload still fails");
    assert!(
        registry.get("executor.probe").expect("get").is_none(),
        "a module whose restoring load failed must stay unavailable"
    );
}

#[tokio::test]
async fn control_a_successful_reload_does_not_restore_the_old_instance() {
    // Without this, "the original is registered afterwards" is also satisfied
    // by an implementation that never swaps anything in.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(Registry::new());
    let original: Arc<dyn Module> = Arc::new(HookRecordingModule::new("original", &calls));
    registry
        .reinstate_internal(
            "executor.probe",
            Arc::clone(&original),
            descriptor("executor.probe"),
        )
        .expect("seed");
    registry.set_discoverer(Box::new(ScriptedDiscoverer {
        fail: false,
        replacement: Some(("executor.probe".to_string(), Arc::clone(&calls))),
    }));

    reload_module(&registry)
        .execute(
            json!({"module_id": "executor.probe", "reason": "hot-fix"}),
            &ctx(),
        )
        .await
        .expect("a successful reload");

    let now = registry
        .get("executor.probe")
        .expect("get")
        .expect("present");
    assert!(
        !Arc::ptr_eq(&now, &original),
        "a successful reload must publish the NEW instance"
    );
}
