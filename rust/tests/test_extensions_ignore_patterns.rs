//! PROTOCOL_SPEC §3.5 / §3.6 A04 step 3a — `extensions.ignore_patterns`.
//!
//! A MUST with no supplier until spec v1.42.0: the key was registered in all
//! three SDKs' configuration key surfaces and read by none of them, so a
//! project that excluded a directory from discovery had it scanned and its
//! modules registered anyway. The failure direction is what earns it a file of
//! its own — a skip rule that fails **open** loads code the operator asked not
//! to load.
//!
//! This SDK needed a second thing the other two already had: a path from a
//! `Config` to the scanner at all. `max_depth`, `follow_symlinks` and
//! `ignore_patterns` were builder options only, so all three declared keys
//! reached nothing here. [`DefaultDiscoverer::from_config`] is that path, and
//! the last case below is what holds it.

use apcore::config::Config;
use apcore::registry::scanner::scan_extensions;
use std::path::{Path, PathBuf};

const SUBDIRS: &[&str] = &["keep", "fixtures", "vendor"];

fn tree(subdirs: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in subdirs {
        let leaf = dir.path().join("executor").join(sub);
        std::fs::create_dir_all(&leaf).expect("mkdir");
        std::fs::write(leaf.join("mod.rs"), "pub struct Mod;").expect("write");
    }
    dir
}

fn discover(root: &Path, patterns: &[&str]) -> Vec<String> {
    let owned: Vec<String> = patterns.iter().map(|s| (*s).to_string()).collect();
    let mut ids: Vec<String> = scan_extensions(root, 8, false, None, &owned)
        .expect("scan")
        .into_iter()
        .map(|f| f.canonical_id)
        .collect();
    ids.sort();
    ids
}

fn all() -> Vec<String> {
    vec![
        "executor.fixtures.mod".to_string(),
        "executor.keep.mod".to_string(),
        "executor.vendor.mod".to_string(),
    ]
}

#[test]
fn nothing_configured_discovers_everything() {
    // The half that keeps this additive: an absent key changes nothing.
    let dir = tree(SUBDIRS);
    assert_eq!(discover(dir.path(), &[]), all());
}

#[test]
fn a_configured_pattern_excludes_the_entry() {
    let dir = tree(SUBDIRS);
    for (patterns, expected) in [
        (
            vec!["fixtures"],
            vec!["executor.keep.mod", "executor.vendor.mod"],
        ),
        (
            vec!["ven*"],
            vec!["executor.fixtures.mod", "executor.keep.mod"],
        ),
        (
            vec!["?endor"],
            vec!["executor.fixtures.mod", "executor.keep.mod"],
        ),
        (vec!["fixtures", "vendor"], vec!["executor.keep.mod"]),
    ] {
        assert_eq!(
            discover(dir.path(), &patterns),
            expected,
            "patterns {patterns:?}"
        );
    }
}

#[test]
fn matching_is_case_sensitive() {
    // §9.2.3 declares this surface sensitive, unlike `obs.redaction.sensitive_keys`:
    // these are filenames, and folding them would make one configuration behave
    // differently on a case-insensitive filesystem than on the case-sensitive
    // one it was written against.
    let dir = tree(SUBDIRS);
    assert_eq!(discover(dir.path(), &["FIXTURES"]), all());
}

#[test]
fn the_pattern_matches_a_segment_not_a_path() {
    // A04 step 3a says ENTRY NAME, so `*` cannot cross a directory boundary:
    // the segments here are `executor`, `fixtures`, `mod.rs`. An implementation
    // matching against the path would exclude everything.
    let dir = tree(SUBDIRS);
    assert_eq!(discover(dir.path(), &["executor/fixtures"]), all());
}

#[test]
fn a_configured_pattern_cannot_switch_off_a_builtin_row() {
    // §3.5: the two lists are a UNION. "Extend the ignore list" could be read
    // as "replace it", and a configuration that re-enabled `.git/` would be a
    // discovery surface nobody expects.
    let dir = tree(&["keep", ".hidden"]);
    assert_eq!(
        discover(dir.path(), &["nothing_matches_this"]),
        vec!["executor.keep.mod".to_string()]
    );
}

#[test]
fn an_empty_entry_is_dropped() {
    // A25 anchors, so `""` would match only the empty name — an operator who
    // leaves a blank line in a YAML list means nothing by it.
    let dir = tree(SUBDIRS);
    assert_eq!(
        discover(dir.path(), &["", "fixtures"]),
        vec![
            "executor.keep.mod".to_string(),
            "executor.vendor.mod".to_string()
        ]
    );
}

#[test]
fn the_discoverer_reads_all_three_scan_keys_from_a_config() {
    // The case this SDK specifically needed. Before v1.42.0 there was no path
    // from a `Config` to the scanner here at all — the three keys were builder
    // options — so the MUST in §3.5 was unsatisfiable in apcore-rust while
    // apcore-python and apcore-typescript both read them.
    let raw = serde_json::json!({
        "version": "1.0",
        "project": {"name": "probe"},
        "extensions": {
            "max_depth": 3,
            "follow_symlinks": true,
            "ignore_patterns": ["fixtures", "ven*"]
        }
    });
    let config: Config = serde_json::from_value(raw).expect("probe config parses");
    let discoverer = apcore::registry::DefaultDiscoverer::from_config(&config);

    // `Debug` is the only public window onto the three fields, and it is
    // enough: the point is that they arrived, not how they are stored.
    let shown = format!("{discoverer:?}");
    for expected in ["max_depth: 3", "follow_symlinks: true", "fixtures", "ven*"] {
        assert!(
            shown.contains(expected),
            "from_config did not carry {expected:?} through: {shown}"
        );
    }

    // And a builder call afterwards still wins, per D-73's precedence:
    // API argument > Config > declared default.
    let overridden = discoverer.with_ignore_patterns(["only_this"]);
    let shown = format!("{overridden:?}");
    assert!(shown.contains("only_this") && !shown.contains("fixtures"));

    let _ = PathBuf::new();
}
