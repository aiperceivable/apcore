//! `Config::get_declared` must answer "did the document declare this", not
//! "does the struct have a value" (aiperceivable/apcore#119).
//!
//! Nine keys are backed by typed struct fields — the four `executor.*`, the
//! four `observability.*`, and `modules_path` — and every one of them carries a
//! value whether the document mentioned it or not. `get_declared` resolved
//! through `get_typed_field` first, so a file saying nothing at all answered
//! `Some(false)` for `observability.tracing.enabled` and `Some(32)` for
//! `executor.max_call_depth`. The declared view is what §9.3 evaluates
//! requiredness against and what §9.2.4's deprecation notice is driven by, so
//! "declared" collapsing into "defaulted" is not a cosmetic difference: it is
//! the distinction those two rules are made of.
//!
//! ## Both halves, both modes, all three tiers
//!
//! A fix that answered `None` for everything would satisfy the first half and
//! destroy the method, so every key is asserted twice: absent from a document
//! that declares nothing, and present — with the WRITTEN value, not the
//! default — in one that declares it.
//!
//! Both config modes, because they retain the document differently and only
//! one of them was ever broken for `modules_path`: namespace mode lifts the
//! `apcore:` members into `user_namespaces` wholesale, legacy mode lets serde
//! consume the top-level scalar. The first version of this fix passed a
//! namespace-mode probe and left legacy mode returning `None` for a file that
//! plainly declared the key.
//!
//! And all three tiers, because `set()` and an `APCORE_*` override are
//! declarations too — `set()` routes a typed key into `set_typed_field` and
//! used to leave no as-written record at all.

use apcore::config::Config;
use serde_json::{json, Value};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Serialises EVERY case in this file, not only the one that writes `APCORE_*`.
///
/// The process environment is shared by every harness thread, so a variable set
/// by the environment-tier case is visible to the cases asserting that a clean
/// document declares nothing — and they go red, reporting a failure against a
/// test that touches no environment at all. Measured here before the guard was
/// widened: `observability.tracing.enabled` came back `Some(true)` in a
/// document that says nothing, from another test's variable.
///
/// Poisoning is ignored so one panic does not cascade. Same convention as
/// `config_discovery.rs`.
static ENV_GUARD: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The nine keys `get_typed_field` answers for, with the value each test
/// document declares — deliberately different from every canonical default, so
/// a reader that fell back to the default would report the wrong value rather
/// than merely the wrong presence.
fn declarations() -> Vec<(&'static str, Value)> {
    vec![
        ("observability.tracing.enabled", json!(true)),
        ("observability.tracing.sampling_rate", json!(0.5)),
        ("observability.tracing.exporter", json!("otlp")),
        ("observability.metrics.enabled", json!(true)),
        ("executor.max_call_depth", json!(7)),
        ("executor.max_module_repeat", json!(3)),
        ("executor.default_timeout", json!(11)),
        ("executor.global_timeout", json!(22)),
        ("modules_path", json!("./declared-modules")),
    ]
}

const BODY: &str = "version: \"1.0\"\n\
project:\n  name: p\n\
observability:\n\
\x20 tracing:\n    enabled: true\n    sampling_rate: 0.5\n    exporter: otlp\n\
\x20 metrics:\n    enabled: true\n\
executor:\n\
\x20 max_call_depth: 7\n  max_module_repeat: 3\n  default_timeout: 11\n  global_timeout: 22\n\
modules_path: ./declared-modules\n";

const EMPTY: &str = "version: \"1.0\"\nproject:\n  name: p\n";

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn load(text: &str) -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, text).expect("write");
    let cfg = Config::load(&path).expect("these documents must all load");
    (cfg, dir)
}

/// `(label, a document declaring everything, a document declaring nothing)`.
fn modes() -> Vec<(&'static str, String, String)> {
    vec![
        ("legacy", BODY.to_string(), EMPTY.to_string()),
        (
            "namespace",
            format!("apcore:\n{}\n", indent(BODY)),
            format!("apcore:\n{}\n", indent(EMPTY)),
        ),
    ]
}

#[test]
fn a_document_that_declares_nothing_declares_none_of_the_typed_keys() {
    let _env = env_guard();
    for (mode, _, empty) in modes() {
        let (cfg, _dir) = load(&empty);
        for (key, _) in declarations() {
            assert_eq!(
                cfg.get_declared(key),
                None,
                "[{mode}] `{key}` is backed by a typed struct field that always carries a \
                 value, but this document never mentions it. `get_declared` reports the \
                 DECLARED view — it is what §9.3 evaluates requiredness against and what \
                 §9.2.4's notice is driven by — so a default must not read as a declaration."
            );
        }
    }
}

#[test]
fn a_document_that_declares_them_reports_the_written_value() {
    let _env = env_guard();
    for (mode, full, _) in modes() {
        let (cfg, _dir) = load(&full);
        for (key, written) in declarations() {
            assert_eq!(
                cfg.get_declared(key),
                Some(written.clone()),
                "[{mode}] `{key}` is declared in this document. Answering `None` here would \
                 satisfy the negative case above by breaking the method, and answering the \
                 canonical default would mean the value came from the default table rather \
                 than the file."
            );
        }
    }
}

#[test]
fn the_two_modes_agree_key_for_key() {
    let _env = env_guard();
    // `modules_path` is where they did NOT agree: it is a top-level SCALAR, so
    // the typed-section retention could not carry it, and serde consumed it
    // without a trace in legacy mode while namespace mode lifted the whole
    // `apcore:` block. A per-mode assertion above would have passed on the mode
    // that worked; this compares them directly.
    let (legacy, _a) = load(BODY);
    let (namespace, _b) = load(&format!("apcore:\n{}\n", indent(BODY)));
    for (key, _) in declarations() {
        assert_eq!(
            legacy.get_declared(key),
            namespace.get_declared(key),
            "the same declaration written in legacy and in namespace mode must produce the \
             same declared view for `{key}`"
        );
    }
}

#[test]
fn set_is_a_declaration_for_a_typed_key() {
    let _env = env_guard();
    // `set()` routes a typed key into `set_typed_field` and returns before the
    // `user_namespaces` write, so it used to leave no as-written record at all.
    let (mut cfg, _dir) = load(EMPTY);
    for (key, written) in declarations() {
        assert_eq!(cfg.get_declared(key), None, "precondition for `{key}`");
        cfg.set(key, written.clone());
        assert_eq!(
            cfg.get_declared(key),
            Some(written.clone()),
            "`set(\"{key}\", …)` is a declaration; §9.2's tier list counts it beside the file"
        );
        assert_eq!(
            cfg.get(key),
            Some(written),
            "and the effective value must still be the one just set — the as-written record \
             is written BESIDE the typed struct, not instead of it"
        );
    }
}

#[test]
fn an_environment_override_is_a_declaration_for_a_typed_key() {
    let _env = env_guard();
    // SAFETY: `env_guard()` serialises every case in this file, and the file is
    // its own test binary, so nothing reads the variable concurrently.
    unsafe { std::env::set_var("APCORE_OBSERVABILITY_TRACING_ENABLED", "true") };
    let (cfg, _dir) = load(EMPTY);
    unsafe { std::env::remove_var("APCORE_OBSERVABILITY_TRACING_ENABLED") };

    assert_eq!(
        cfg.get_declared("observability.tracing.enabled"),
        Some(json!(true)),
        "an APCORE_* override is a declaration (§9.2 tier 2). `apply_env_overrides` goes \
         through `set()`, so this is the same path as the case above reached from the tier \
         an operator is most likely to use in a container."
    );
}

#[test]
fn requiredness_still_distinguishes_declared_from_defaulted() {
    let _env = env_guard();
    // §9.3 step 1, the caller this method exists for. Neither required field is
    // typed, so the change above cannot reach them — asserted rather than
    // assumed, because "the fix cannot affect X" is exactly the claim that
    // deserves a test.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, "version: \"1.0\"\n").expect("write");
    // `Config::load` runs `validate`, so the requiredness check shows up as a
    // load failure rather than a return value.
    let outcome = Config::load(&path);
    let message = outcome
        .as_ref()
        .err()
        .map(std::string::ToString::to_string)
        .unwrap_or_default();
    assert!(
        message.contains("project.name"),
        "a document without `project.name` must still fail the required-field check; got \
         {outcome:?}"
    );

    let (cfg, _dir) = load(EMPTY);
    assert_eq!(
        cfg.get_declared("version"),
        Some(json!("1.0")),
        "and a field the document DOES declare still reads as declared"
    );
    assert_eq!(cfg.get_declared("project.does_not_exist"), None);
}
