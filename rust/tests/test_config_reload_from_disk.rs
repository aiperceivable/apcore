//! Issue #45.5 — `Config::reload_from_disk()` re-reads the on-disk source file.
//!
//! In Rust we cannot dynamically reload compiled module code (no .so/.rlib
//! plugin model in the SDK), but we MUST be able to refresh static
//! configuration without a binary restart. This test validates that
//! `Config::reload_from_disk()` picks up file mutations performed since the
//! original load and that values previously set in-memory via `set()` are
//! discarded by the reload.

use std::io::Write;

use apcore::config::Config;

#[test]
fn reload_from_disk_picks_up_file_changes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");

    // A valid legacy-mode config must carry the spec-required top-level fields
    // (version/project/extensions/schema/acl), or `from_yaml_file`'s validation
    // rejects it. Keep them stable across both writes; only the executor knobs
    // change between loads.
    let required_fields = "version: \"0.23.0\"\n\
         project:\n  name: demo\n\
         extensions:\n  root: ./extensions\n\
         schema:\n  root: ./schemas\n\
         acl:\n  root: ./acl\n  default_effect: deny\n";

    // Initial: max_call_depth = 8.
    {
        let mut f = std::fs::File::create(&path).expect("create");
        writeln!(
            f,
            "{required_fields}executor:\n  max_call_depth: 8\n  default_timeout: 1000\n  global_timeout: 60000"
        )
        .unwrap();
    }
    let mut cfg = Config::from_yaml_file(&path).expect("from_yaml_file");
    assert_eq!(cfg.executor.max_call_depth, 8);

    // Mutate in memory — should be discarded by reload_from_disk.
    cfg.set("executor.max_call_depth", serde_json::Value::from(64u64));
    assert_eq!(cfg.executor.max_call_depth, 64);

    // Rewrite file with a new value.
    {
        let mut f = std::fs::File::create(&path).expect("rewrite");
        writeln!(
            f,
            "{required_fields}executor:\n  max_call_depth: 16\n  default_timeout: 2000\n  global_timeout: 60000"
        )
        .unwrap();
    }

    cfg.reload_from_disk()
        .expect("reload_from_disk should succeed");

    assert_eq!(
        cfg.executor.max_call_depth, 16,
        "reload_from_disk must re-read the file (got {})",
        cfg.executor.max_call_depth
    );
    assert_eq!(cfg.executor.default_timeout, 2000);
}

#[test]
fn reload_from_disk_without_source_path_errors() {
    // Config built from defaults has no on-disk source.
    let mut cfg = Config::from_defaults();
    assert!(
        cfg.reload_from_disk().is_err(),
        "reload_from_disk without source path must return an error"
    );
}
