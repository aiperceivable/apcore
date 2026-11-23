//! D-89 — at most one `x-deprecation` warning per `(module_id, version)` per
//! registry instance.
//!
//! This SDK already had a unit test for the cadence, and it asserted the SIZE
//! OF THE PRIVATE `deprecation_warned` SET. "The set has one entry" and "one
//! warning reached the operator" are different claims: a `log_deprecation_warning`
//! that returned early, logged at `debug!`, or was dropped from the call site
//! entirely would leave the set exactly as the unit test requires. The set is
//! the artifact that records the decision; the emitted warning is the mechanism
//! it exists for. This file asserts the mechanism.
//!
//! Two dimensions left open by v1.49.0 were adjudicated at v1.59.0 and are
//! asserted here. Both changed this SDK:
//!
//!   * WHERE the warning fires — the READ (`get_definition`). This SDK warned
//!     at REGISTRATION and its reads never warned at all, which loses the
//!     notice outright for a host that registers before installing a `tracing`
//!     subscriber: the dedupe marker is written whether or not anything was
//!     listening, so no later read can re-emit it. `discover()` at startup is
//!     the ordinary case. D-89's own rationale ("`get_definition` is a read
//!     that hosts call in loops") was written about the read path.
//!   * What an unregister + re-register does — the key carries the
//!     `x-deprecation` BLOCK and is never cleared on unregister. The same
//!     notice is silent; a changed or newly added one warns. This SDK stayed
//!     silent for both, swallowing a genuinely new notice.

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::registry::registry::Registry;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct Noop;

#[async_trait]
impl Module for Noop {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "a deprecated module"
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

fn deprecation_metadata() -> HashMap<String, Value> {
    let mut m = HashMap::new();
    m.insert(
        "x-deprecation".to_string(),
        json!({
            "deprecated_since": "1.0.0",
            "sunset_version": "3.0.0",
            "migration_guide": "Use mod.new instead.",
        }),
    );
    m
}

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

/// Count `x-deprecation` warnings actually emitted while `f` runs.
fn deprecation_warnings(f: impl FnOnce()) -> usize {
    let buf = WarnCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let bytes = buf
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|l| l.contains("is deprecated"))
        .count()
}

fn register(reg: &Registry, id: &str, version: &str) {
    register_with(reg, id, version, deprecation_metadata());
}

fn register_with(reg: &Registry, id: &str, version: &str, metadata: HashMap<String, Value>) {
    reg.register_versioned(id, Box::new(Noop), Some(version), Some(metadata))
        .expect("register");
}

/// The same notice with its sunset brought forward — a CHANGED block.
fn changed_deprecation_metadata() -> HashMap<String, Value> {
    let mut m = HashMap::new();
    m.insert(
        "x-deprecation".to_string(),
        json!({
            "deprecated_since": "1.0.0",
            "sunset_version": "2.0.0",
            "migration_guide": "Use mod.new instead.",
        }),
    );
    m
}

#[test]
fn one_warning_reaches_the_operator_per_module_and_version() {
    let reg = Registry::new();
    register(&reg, "cadence.once", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            for _ in 0..10 {
                let _ = reg.get_definition("cadence.once");
            }
        }),
        1,
        "ten reads must produce one warning"
    );
}

#[test]
fn the_warning_fires_on_the_read_not_on_registration() {
    // D-89 / spec v1.59.0. Registration runs at startup, frequently before a
    // `tracing` subscriber exists; the marker is written either way, so a
    // registration-time warning is lost with no later chance to re-emit.
    let reg = Registry::new();
    assert_eq!(
        deprecation_warnings(|| register(&reg, "cadence.read_path", "1.0.0")),
        0,
        "registration must not warn"
    );
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.read_path");
        }),
        1,
        "the read must warn"
    );
}

#[test]
fn a_re_registration_carrying_the_same_notice_stays_silent() {
    // D-89 / spec v1.59.0. `watch()` re-runs discovery as an unregister +
    // re-register, so clearing the key there re-warns for every deprecated
    // module on every hot reload.
    let reg = Registry::new();
    register(&reg, "cadence.same_notice", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.same_notice");
        }),
        1
    );

    reg.unregister("cadence.same_notice").expect("unregister");
    register(&reg, "cadence.same_notice", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.same_notice");
        }),
        0,
        "an unchanged notice must not be announced twice"
    );
}

#[test]
fn a_re_registration_carrying_a_changed_notice_warns_again() {
    // The other half, and the control for the test above: without it, "stays
    // silent" is equally satisfied by a registry that never warns for a
    // re-registered module at all — which is what this SDK did.
    let reg = Registry::new();
    register(&reg, "cadence.changed_notice", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.changed_notice");
        }),
        1
    );

    reg.unregister("cadence.changed_notice")
        .expect("unregister");
    register_with(
        &reg,
        "cadence.changed_notice",
        "1.0.0",
        changed_deprecation_metadata(),
    );
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.changed_notice");
        }),
        1,
        "a CHANGED notice on a re-registered module must be announced"
    );
}

#[test]
fn a_notice_added_on_re_registration_warns() {
    let reg = Registry::new();
    reg.register_versioned("cadence.added_notice", Box::new(Noop), Some("1.0.0"), None)
        .expect("register");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.added_notice");
        }),
        0
    );

    reg.unregister("cadence.added_notice").expect("unregister");
    register(&reg, "cadence.added_notice", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.added_notice");
        }),
        1
    );
}

#[test]
fn control_a_module_with_no_deprecation_block_never_warns() {
    // Without this, "exactly one warning" would also hold for a registry that
    // warns once for any module at all.
    let reg = Registry::new();
    assert_eq!(
        deprecation_warnings(|| {
            reg.register_versioned("cadence.plain", Box::new(Noop), Some("1.0.0"), None)
                .expect("register");
            let _ = reg.get_definition("cadence.plain");
        }),
        0
    );
}

#[test]
fn distinct_modules_warn_separately() {
    let reg = Registry::new();
    register(&reg, "cadence.first", "1.0.0");
    register(&reg, "cadence.second", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            for _ in 0..3 {
                let _ = reg.get_definition("cadence.first");
                let _ = reg.get_definition("cadence.second");
            }
        }),
        2
    );
}

#[test]
fn the_dedupe_is_per_registry_instance_not_process_global() {
    // A process-global set would silence the second registry's advisory for a
    // module its operator has never been told about.
    let a = Registry::new();
    let b = Registry::new();
    register(&a, "cadence.instance", "1.0.0");
    register(&b, "cadence.instance", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = a.get_definition("cadence.instance");
            let _ = b.get_definition("cadence.instance");
        }),
        2
    );
}

#[test]
fn the_dedupe_key_includes_the_version() {
    // A newly registered version is a new deprecation notice with its own
    // sunset. Keying on the module id alone silences it.
    let reg = Registry::new();
    register(&reg, "cadence.versions", "1.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.versions");
        }),
        1
    );
    reg.unregister("cadence.versions").expect("unregister");
    register(&reg, "cadence.versions", "2.0.0");
    assert_eq!(
        deprecation_warnings(|| {
            let _ = reg.get_definition("cadence.versions");
        }),
        1,
        "a different version is a different notice"
    );
}
