//! Canonical conformance-fixture locator, shared by every conformance driver.
//!
//! The fixtures and schemas are the single source of truth in the apcore spec
//! repo; this SDK reads them in place rather than vendoring a copy, so a
//! spec-side edit reaches Rust on the next test run.
//!
//! Search order for `conformance/fixtures/` (conformance.md §8.2.1):
//!   1. `$CONFORMANCE_FIXTURES` — a fixtures DIRECTORY, used as-is.
//!   2. `$CONFORMANCE_SPEC_REPO` — the spec repo root (set by CI).
//!   3. `../apcore/` beside this repo — the standard workspace layout.
//!
//! Every OTHER spec-repo subdirectory — `schemas/` — skips step 1: it names a
//! single directory, not a repo, so there is nothing to append to.
//!
//! This file is not a test target of its own (`autotests = false`); it is
//! included by `tests/it.rs` and `tests/conformance_test.rs`, each of which
//! uses a subset of it — hence the crate-wide `dead_code` allow.

#![allow(dead_code)]

use std::path::PathBuf;

const SPEC_REPO_ENV: &str = "CONFORMANCE_SPEC_REPO";

/// Transitional fallback (apcore#86). The locator used to be
/// `APCORE_SPEC_REPO`, but PROTOCOL_SPEC §9.2 makes *every* `APCORE_*`
/// variable a config override: the suffix is lowercased and split into a dot
/// path, so `APCORE_SPEC_REPO=/path` injected `spec.repo` into the declared
/// config document that §9.1's required-field check runs against. The locator
/// is test infrastructure, not configuration, so it moved out of the claimed
/// prefix. Reading the old name keeps a developer who still exports it
/// working. REMOVE once all three SDK CI workflows are on
/// `CONFORMANCE_SPEC_REPO`.
const LEGACY_SPEC_REPO_ENV: &str = "APCORE_SPEC_REPO";

/// A `conformance/fixtures` DIRECTORY, taking precedence over the repo-root
/// form (conformance.md §8.2.1 rule 1).
///
/// It names a directory with no repository around it, which is what makes it
/// useful: a driver can run against a SYNTHESISED fixture set — an older shape,
/// a single edited case — without producing a whole spec repo to hold it.
/// Drivers land one push before fixtures here, so a driver must tolerate the
/// fixture shape that predates the keys it reads, and that cannot be verified
/// from a working tree which already holds the newer fixture.
const FIXTURES_ENV: &str = "CONFORMANCE_FIXTURES";

/// Transitional fallback for [`FIXTURES_ENV`] (apcore#88), the exact twin of
/// [`LEGACY_SPEC_REPO_ENV`] and for the same reason: PROTOCOL_SPEC §9.2 lowers
/// every `APCORE_*` variable to a config key, so `APCORE_FIXTURES` declared
/// `fixtures` in a document no schema knows about.
/// REMOVE once all three SDK CI workflows are on `CONFORMANCE_FIXTURES`.
const LEGACY_FIXTURES_ENV: &str = "APCORE_FIXTURES";

/// Return the fixtures-directory override as `(variable_name, value)`, if set.
///
/// Same shape and same reason as [`spec_repo_env`]: the name travels with the
/// value so a panic message names the variable the developer actually set.
fn fixtures_env() -> Option<(&'static str, String)> {
    for name in [FIXTURES_ENV, LEGACY_FIXTURES_ENV] {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => return Some((name, value)),
            _ => {}
        }
    }
    None
}

/// Return the spec-repo override as `(variable_name, value)`, if set.
///
/// The name travels with the value so a panic message can name the variable
/// the developer actually set rather than the one they did not.
fn spec_repo_env() -> Option<(&'static str, String)> {
    for name in [SPEC_REPO_ENV, LEGACY_SPEC_REPO_ENV] {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => return Some((name, value)),
            _ => {}
        }
    }
    None
}

/// Resolve `<spec repo>/<relative>`, panicking with a fix-it message if absent.
fn spec_repo_subdir(relative: &[&str], what: &str) -> PathBuf {
    if let Some((name, value)) = spec_repo_env() {
        let dir = relative
            .iter()
            .fold(PathBuf::from(&value), |acc, part| acc.join(part));
        if dir.is_dir() {
            return dir;
        }
        panic!("{name}={value} does not contain {what}");
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let apcore = manifest_dir.parent().unwrap().join("apcore");
    let sibling = relative
        .iter()
        .fold(apcore.clone(), |acc, part| acc.join(part));
    if sibling.is_dir() {
        return sibling;
    }

    panic!(
        "Cannot find apcore {what}\n\
         Fix one of:\n\
         1. Set {SPEC_REPO_ENV} to the apcore spec repo path\n\
         2. Set {FIXTURES_ENV} to a conformance/fixtures directory (fixtures only)\n\
         3. Clone apcore as a sibling at {}",
        apcore.display()
    );
}

/// Locate the canonical `conformance/fixtures/` directory.
///
/// conformance.md §8.2.1: `CONFORMANCE_FIXTURES` (a directory), then
/// `CONFORMANCE_SPEC_REPO` (a repo root), then a sibling checkout. A variable
/// that is set but does not resolve panics rather than falling through to the
/// next source — silently testing against different fixtures than the operator
/// named is worse than not running.
pub fn find_fixtures_root() -> PathBuf {
    if let Some((name, value)) = fixtures_env() {
        let dir = PathBuf::from(&value);
        if dir.is_dir() {
            return dir;
        }
        panic!("{name}={value} is not a directory");
    }
    spec_repo_subdir(&["conformance", "fixtures"], "conformance/fixtures/")
}

/// Locate a fixture that is deliberately **staged** outside
/// `conformance/fixtures/` while a cross-SDK rollout is in flight.
///
/// A fixture whose expectations changed cannot land in the canonical directory
/// until every SDK driver implements the new behaviour, or CI goes red in each
/// repository for the duration. The spec repo therefore stages it under
/// `planning/<topic>/staged-fixtures/`, and a driver that has already converged
/// reads it from there.
///
/// Returns `None` when the staged copy is absent — which is what happens once
/// the fixture is promoted, so the driver falls back to the canonical location
/// with no code change.
pub fn find_staged_fixture(topic: &str, file_name: &str) -> Option<PathBuf> {
    let roots: Vec<PathBuf> = match spec_repo_env() {
        Some((_, value)) => vec![PathBuf::from(value)],
        None => vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("apcore")],
    };
    roots
        .into_iter()
        .map(|root| {
            root.join("planning")
                .join(topic)
                .join("staged-fixtures")
                .join(file_name)
        })
        .find(|path| path.is_file())
}

/// Locate the canonical `schemas/` directory.
/// §8.2.1 rule 4: this deliberately does NOT consult `CONFORMANCE_FIXTURES`,
/// which names one directory rather than a repo, so there is nothing to append
/// `schemas/` to.
pub fn find_schemas_root() -> PathBuf {
    spec_repo_subdir(&["schemas"], "schemas/")
}
