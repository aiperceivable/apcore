//! D-121 — `reload_dependents` is deprecated for removal.
//!
//! Declared in all three SDKs' input schemas and read by none: a spec MUST
//! ("also reload modules that depend on matched modules") that no
//! implementation satisfied. That is the §9.1.3 "declared surface reaches no
//! mechanism" shape the spec forbids for configuration keys, here applied to a
//! module INPUT FIELD, and three independent implementations skipping it is the
//! evidence the maintainer decision rests on — deprecate now, remove at 2.0, do
//! NOT implement.
//!
//! Deprecated rather than removed today because the input schema sets
//! `additionalProperties: false`: at 2.0 the same call stops being a silent
//! no-op and becomes a VALIDATION ERROR, so a caller passing it needs a release
//! in which they are told.

use apcore::config::Config;
use apcore::context::Context;
use apcore::events::emitter::EventEmitter;
use apcore::module::Module;
use apcore::registry::registry::Registry;
use apcore::sys_modules::control::ReloadModule;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct WarnCapture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for WarnCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for WarnCapture {
    type Writer = Self;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

/// Install a capturing subscriber for the current thread.
///
/// `tracing::subscriber::with_default` takes a SYNC closure, and the calls
/// under test are async. `set_default` returns a guard instead, and the default
/// subscriber is thread-local — a `#[tokio::test]` runs its future on one
/// thread, so the guard covers everything awaited inside it.
fn start_capture() -> (WarnCapture, tracing::subscriber::DefaultGuard) {
    let buf = WarnCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (buf, guard)
}

/// Lines mentioning `reload_dependents` captured so far.
fn captured(buf: &WarnCapture) -> Vec<String> {
    let bytes = buf
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|l| l.contains("reload_dependents"))
        .map(ToString::to_string)
        .collect()
}

fn reload_module() -> ReloadModule {
    ReloadModule::new(Arc::new(Registry::new()), Arc::new(EventEmitter::new()))
}

/// Drive the module the way the executor does: through `execute`. The reload
/// itself fails (no such module), which is fine — the warning is emitted before
/// any of that, and asserting on `execute` rather than a private helper is what
/// makes this a test of the door a caller comes through.
async fn call(module: &ReloadModule, inputs: Value) {
    let ctx = Context::<Value>::anonymous();
    let _ = module.execute(inputs, &ctx).await;
}

#[tokio::test]
async fn passing_it_warns_and_names_the_replacement_and_the_removal() {
    // A deprecation notice that does not say what to do instead, or when the
    // field stops being ignored, leaves the caller to discover both.
    let module = reload_module();
    let (buf, _guard) = start_capture();
    call(
        &module,
        json!({"module_id": "a.b", "reason": "x", "reload_dependents": true}),
    )
    .await;
    let warnings = captured(&buf);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("path_filter"), "{}", warnings[0]);
    assert!(warnings[0].contains("2.0"), "{}", warnings[0]);
    assert!(warnings[0].contains("validation error"), "{}", warnings[0]);
}

#[tokio::test]
async fn it_warns_once_per_instance() {
    // `reload` is called by hot-reload loops and watchers, so an advisory whose
    // volume is proportional to traffic is one operators learn to filter out —
    // the cadence D-89 settled.
    let module = reload_module();
    let (buf, _guard) = start_capture();
    for _ in 0..5 {
        call(
            &module,
            json!({"module_id": "a.b", "reason": "x", "reload_dependents": true}),
        )
        .await;
    }
    assert_eq!(captured(&buf).len(), 1, "{:?}", captured(&buf));
}

#[tokio::test]
async fn the_cadence_is_per_instance_not_per_process() {
    // A process-wide one-shot tells the FIRST caller and leaves every later one
    // to discover it in production — the reason D-90's notice is per token and
    // D-89's dedupe is per registry instance.
    let (buf, _guard) = start_capture();
    for _ in 0..2 {
        let module = reload_module();
        call(
            &module,
            json!({"module_id": "a.b", "reason": "x", "reload_dependents": true}),
        )
        .await;
    }
    assert_eq!(captured(&buf).len(), 2, "{:?}", captured(&buf));
}

#[tokio::test]
async fn control_omitting_it_or_passing_false_says_nothing() {
    // Without this, "it warns" is also satisfied by warning on every reload,
    // which is noise for the ordinary call the method exists for.
    for inputs in [
        json!({"module_id": "a.b", "reason": "x"}),
        json!({"module_id": "a.b", "reason": "x", "reload_dependents": false}),
    ] {
        let module = reload_module();
        let (buf, _guard) = start_capture();
        call(&module, inputs.clone()).await;
        let warnings = captured(&buf);
        assert!(warnings.is_empty(), "{inputs} -> {warnings:?}");
    }
}

#[test]
fn the_schema_marks_it_deprecated_and_says_what_replaces_it() {
    // The notice has to be readable WITHOUT triggering it: a host reading the
    // input schema — which is what `describe` / `get_definition` hand an agent
    // — must see the deprecation without having to call with the field.
    let schema = reload_module().input_schema();
    let field = &schema["properties"]["reload_dependents"];
    assert_eq!(field["deprecated"], json!(true), "{field}");
    let description = field["description"].as_str().expect("description");
    assert!(description.contains("path_filter"), "{description}");
    assert!(description.contains("2.0"), "{description}");
    let _ = Config::default();
}
