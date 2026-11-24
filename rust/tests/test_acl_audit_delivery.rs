//! PROTOCOL_SPEC §6.3.2 — ACL audit delivery (apcore#118, decision D-66).
//!
//! §6.3.1 has always specified the *record*. Nothing specified **delivery**:
//! `ACL::set_audit_logger` was the whole surface, with no default sink, no
//! statement of what happens when delivery fails, and no meaning for the
//! `audit:` block's three settings — which were declared in two places and read
//! in neither.
//!
//! The sharpest consequence was measured, not inferred: a panicking audit
//! callback propagated out of `check()` and turned an **allowed** call into a
//! panic. That is the one behaviour change here, and it is a fix.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use apcore::acl::{AuditEntry, ACL};
use apcore::errors::ErrorCode;

const RULES: &str = "default_effect: deny\nrules:\n  - callers: [\"api.*\"]\n    targets: [\"executor.*\"]\n    effect: allow\n";

fn write(audit: &str, extra: &str) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("acl.yaml");
    std::fs::write(&path, format!("{RULES}{extra}{audit}")).expect("write");
    (path.to_string_lossy().into_owned(), dir)
}

// --- log capture -----------------------------------------------------------

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for CaptureWriter {
    type Writer = Self;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
    // Tracing caches callsite interest process-wide. With only one registered
    // dispatcher it consults the current thread, which may have no subscriber
    // while another test is entering a capture scope. Keep two dispatchers alive
    // to retain the multi-dispatcher path even between captures. Neither becomes
    // a default subscriber; output still goes only to each scope's own buffer.
    static DISPATCHERS: OnceLock<[tracing::Dispatch; 2]> = OnceLock::new();
    DISPATCHERS.get_or_init(|| {
        std::array::from_fn(|_| tracing::Dispatch::new(tracing_subscriber::registry()))
    });
    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let bytes = buf.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (out, String::from_utf8_lossy(&bytes).into_owned())
}

fn audit_lines(logs: &str) -> Vec<&str> {
    logs.lines()
        .filter(|l| l.contains(apcore::acl::AUDIT_EVENT_NAME))
        .collect()
}

/// The macro that emits the record needs a string LITERAL for the message, so
/// the name exists twice in the source. This pins them together.
#[test]
fn audit_event_name_matches_the_constant() {
    assert_eq!(apcore::acl::AUDIT_EVENT_NAME, "apcore.acl.audit");
}

// ---------------------------------------------------------------------------
// Requirement 2 — declaration activates the default sink, not the default value
// ---------------------------------------------------------------------------

#[test]
fn no_audit_block_produces_no_audit_output() {
    // The compatibility boundary. `enabled` defaults to true, so a merged-view
    // reading would switch a log record per ACL check on for every ACL file in
    // existence — a behaviour change measured in volume.
    let (path, _dir) = write("", "");
    let acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    assert!(audit_lines(&logs).is_empty(), "logs:\n{logs}");
}

#[test]
fn a_declared_block_activates_the_default_sink() {
    let (path, _dir) = write("audit:\n  enabled: true\n", "");
    let acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    assert_eq!(audit_lines(&logs).len(), 1, "logs:\n{logs}");
}

#[test]
fn an_empty_block_is_declared_with_every_setting_at_its_default() {
    // `audit:` with nothing under it parses to null, and the operator still
    // wrote the block. Requirement 2 makes DECLARATION the switch, so presence
    // — not truthiness — is what activates the default sink.
    let (path, _dir) = write("audit:\n", "");
    let acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    assert_eq!(audit_lines(&logs).len(), 1, "logs:\n{logs}");
}

#[test]
fn a_declared_block_with_enabled_false_is_silent() {
    let (path, _dir) = write("audit:\n  enabled: false\n", "");
    let acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    assert!(audit_lines(&logs).is_empty(), "logs:\n{logs}");
}

#[test]
fn the_default_sink_carries_all_thirteen_fields_under_their_wire_names() {
    let (path, _dir) = write("audit:\n  enabled: true\n", "");
    let acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    let line = audit_lines(&logs)[0];
    for field in [
        "timestamp",
        "caller_id",
        "target_id",
        "decision",
        "reason",
        "matched_rule",
        "matched_rule_index",
        "identity_type",
        "roles",
        "call_depth",
        "trace_id",
        "handler_error",
        "approval_required",
    ] {
        assert!(line.contains(field), "field {field} missing from:\n{line}");
    }
    assert!(line.contains("caller_id=api.x"), "{line}");
    assert!(line.contains("decision=allow"), "{line}");
}

#[test]
fn log_level_sets_the_default_sinks_level() {
    for (level, marker) in [
        ("trace", "DEBUG"),
        ("debug", "DEBUG"),
        ("info", "INFO"),
        ("warn", "WARN"),
        ("error", "ERROR"),
    ] {
        let (path, _dir) = write(
            &format!("audit:\n  enabled: true\n  log_level: {level}\n"),
            "",
        );
        let acl = ACL::load(&path).expect("loads");
        let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
        let line = audit_lines(&logs)[0];
        assert!(line.contains(marker), "level {level}: {line}");
    }
}

// ---------------------------------------------------------------------------
// Requirement 1 — one effective sink, never two
// ---------------------------------------------------------------------------

#[test]
fn a_callback_receives_every_entry_and_the_block_does_not_apply() {
    // The API-beats-configuration rule in the direction that matters:
    // `include_denied: false` must not silently truncate a compliance sink.
    let (path, _dir) = write("audit:\n  enabled: false\n  include_denied: false\n", "");
    let mut acl = ACL::load(&path).expect("loads");
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    acl.set_audit_logger(move |e: &AuditEntry| {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(e.decision.clone());
    });

    let (_, logs) = capture(|| {
        acl.check(Some("api.x"), "executor.y", None);
        acl.check(Some("worker.x"), "executor.y", None);
    });
    assert_eq!(
        *seen.lock().unwrap_or_else(PoisonError::into_inner),
        vec!["allow".to_string(), "deny".to_string()]
    );
    assert!(audit_lines(&logs).is_empty(), "logs:\n{logs}");
}

#[test]
fn an_overridden_block_names_every_field_that_does_not_apply() {
    // Not only the most visible one: an operator told about one of three
    // settings has been told the smaller half of what happened.
    let (path, _dir) = write(
        "audit:\n  enabled: true\n  include_denied: false\n  log_level: error\n",
        "",
    );
    let mut acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| acl.set_audit_logger(|_e: &AuditEntry| {}));
    assert!(logs.contains("does not apply"), "logs:\n{logs}");
    for field in ["audit.enabled", "audit.include_denied", "audit.log_level"] {
        assert!(logs.contains(field), "{field} missing from:\n{logs}");
    }
}

#[test]
fn no_override_notice_without_a_block() {
    let (path, _dir) = write("", "");
    let mut acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| acl.set_audit_logger(|_e: &AuditEntry| {}));
    assert!(!logs.contains("does not apply"), "logs:\n{logs}");
}

// ---------------------------------------------------------------------------
// Requirement 3 — delivery never changes the access decision
// ---------------------------------------------------------------------------

fn with_panicking_sink() -> ACL {
    let (path, dir) = write("", "");
    std::mem::forget(dir); // the ACL keeps the path for reload()
    let mut acl = ACL::load(&path).expect("loads");
    acl.set_audit_logger(|_e: &AuditEntry| panic!("the audit sink is down"));
    acl
}

#[test]
fn a_panicking_callback_does_not_change_the_decision() {
    // Measured before spec v1.45.0: this propagated, and an ALLOWED call became
    // a panic. Both decisions are driven — deny is a different branch.

    let acl = with_panicking_sink();
    let (allowed, _) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    let (denied, _) = capture(|| acl.check(Some("worker.x"), "executor.y", None));
    assert!(allowed, "an allowed call must stay allowed");
    assert!(!denied, "a denied call must stay denied");
}

#[tokio::test]
async fn a_panicking_callback_does_not_change_the_async_decision() {
    let acl = with_panicking_sink();
    let allowed = acl.async_check(Some("api.x"), "executor.y", None).await;
    let denied = acl.async_check(Some("worker.x"), "executor.y", None).await;
    assert!(allowed);
    assert!(!denied);
}

#[test]
fn a_failing_sink_is_reported_once() {
    // §6.3.2 requirement 5. A sink that is down otherwise produces one
    // diagnostic per check — the flood §9.2.2 rejects.
    let acl = with_panicking_sink();
    let (_, logs) = capture(|| {
        for _ in 0..5 {
            acl.check(Some("api.x"), "executor.y", None);
        }
    });
    assert_eq!(
        logs.matches("audit delivery failed").count(),
        1,
        "logs:\n{logs}"
    );
}

#[test]
fn concurrent_captures_preserve_each_sinks_first_failure_report() {
    const THREADS: usize = 8;
    let barrier = std::sync::Barrier::new(THREADS);
    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|| {
                barrier.wait();
                for _ in 0..100 {
                    let mut acl = ACL::new(vec![], "deny", None);
                    acl.set_audit_logger(|_: &AuditEntry| panic!("the audit sink is down"));
                    let (_, logs) = capture(|| {
                        for _ in 0..5 {
                            assert!(!acl.check(Some("api.x"), "executor.y", None));
                        }
                    });
                    assert_eq!(logs.matches("audit delivery failed").count(), 1, "{logs}");
                }
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Requirement 6 — include_denied
// ---------------------------------------------------------------------------

#[test]
fn include_denied_false_withholds_denials_from_the_default_sink() {
    let (path, _dir) = write("audit:\n  enabled: true\n  include_denied: false\n", "");
    let acl = ACL::load(&path).expect("loads");
    let (_, logs) = capture(|| {
        acl.check(Some("api.x"), "executor.y", None);
        acl.check(Some("worker.x"), "executor.y", None);
    });
    let lines = audit_lines(&logs);
    assert_eq!(lines.len(), 1, "logs:\n{logs}");
    assert!(lines[0].contains("decision=allow"), "{}", lines[0]);
}

#[test]
fn include_denied_false_warns_at_load() {
    // Security friction, not a refusal: it withholds the security-relevant
    // half, so the operator is told rather than stopped.
    let (path, _dir) = write("audit:\n  include_denied: false\n", "");
    let (acl, logs) = capture(|| ACL::load(&path));
    assert!(acl.is_ok());
    assert_eq!(logs.matches("include_denied").count(), 1, "logs:\n{logs}");
    assert!(logs.contains("DENIED"), "logs:\n{logs}");
}

#[test]
fn include_denied_true_is_silent() {
    let (path, _dir) = write("audit:\n  include_denied: true\n", "");
    let (_, logs) = capture(|| ACL::load(&path));
    assert!(!logs.contains("include_denied"), "logs:\n{logs}");
}

// ---------------------------------------------------------------------------
// Requirement 7 — reload
// ---------------------------------------------------------------------------

#[test]
fn reload_refreshes_the_block_and_preserves_the_callback() {
    // Before spec v1.45.0 `reload()` refreshed only rules and the default
    // effect, so `audit:` was the one part of the document a reload missed.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("acl.yaml");
    std::fs::write(
        &path,
        format!("{RULES}audit:\n  enabled: true\n  log_level: info\n"),
    )
    .expect("write");
    let mut acl = ACL::load(path.to_str().expect("utf-8")).expect("loads");

    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    assert!(audit_lines(&logs)[0].contains("INFO"), "logs:\n{logs}");

    std::fs::write(
        &path,
        format!("{RULES}audit:\n  enabled: true\n  log_level: error\n"),
    )
    .expect("rewrite");
    acl.reload().expect("reloads");

    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    assert!(audit_lines(&logs)[0].contains("ERROR"), "logs:\n{logs}");
}

#[test]
fn reload_starts_a_new_failure_report_scope() {
    // §6.3.2 requirement 5's scoping: per ACL instance AND per effective sink
    // configuration, so a new failure is never hidden behind an old one.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("acl.yaml");
    std::fs::write(&path, format!("{RULES}audit:\n  enabled: true\n")).expect("write");
    let mut acl = ACL::load(path.to_str().expect("utf-8")).expect("loads");
    acl.set_audit_logger(|_e: &AuditEntry| panic!("down"));

    let (_, logs) = capture(|| {
        acl.check(Some("api.x"), "executor.y", None);
        acl.check(Some("api.x"), "executor.y", None);
    });
    assert_eq!(logs.matches("audit delivery failed").count(), 1);

    acl.reload().expect("reloads");
    let (_, logs) = capture(|| acl.check(Some("api.x"), "executor.y", None));
    assert_eq!(
        logs.matches("audit delivery failed").count(),
        1,
        "logs:\n{logs}"
    );
}

// ---------------------------------------------------------------------------
// Requirement 8 — the block is validated, nothing else gets stricter
// ---------------------------------------------------------------------------

#[test]
fn a_malformed_audit_block_is_rejected_at_load() {
    for bad in [
        "audit:\n  enabled: \"yes\"\n",
        "audit:\n  log_level: verbose\n",
        "audit:\n  include_denied: 1\n",
        "audit:\n  enabled: true\n  unknown_key: true\n",
        "audit: not-a-mapping\n",
    ] {
        let (path, _dir) = write(bad, "");
        let err = ACL::load(&path).expect_err("must be rejected");
        assert_eq!(err.code, ErrorCode::ConfigInvalid, "for {bad:?}: {err}");
    }
}

#[test]
fn other_unknown_root_keys_are_still_ignored() {
    // Wiring one block is not unknown-key closure for ACL files — the same
    // scoping §9.2.4.1 gave its own notice.
    let (path, _dir) = write(
        "audit:\n  enabled: true\n",
        "telemetry:\n  enabled: true\nx_vendor_note: kept\n",
    );
    let (acl, logs) = capture(|| ACL::load(&path));
    assert!(acl.is_ok(), "{logs}");
    assert!(!logs.contains("telemetry"), "logs:\n{logs}");
}
