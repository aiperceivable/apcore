//! A symlink whose target escapes the extensions root is never discovered
//! (spec v1.50.0, D-94).
//!
//! The confinement check has to run BEFORE the directory/file split, not inside
//! the directory branch. apcore-python's lived inside it, so a symlinked `.py`
//! whose target was outside the root was discovered and imported — code
//! execution from outside the tree the operator configured, on the one branch
//! that yields importable files.
//!
//! This SDK is correct today. The test exists because nothing said so: D-94 is a
//! security decision, and discovery is the one place where a silent regression
//! means executing a file the operator never put there. A behaviour with no test
//! is a behaviour the next refactor is free to change.

use std::fs;
use std::path::{Path, PathBuf};

use apcore::registry::scanner::scan_extensions;

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("extensions");
    let outside = tmp.path().join("sibling");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&outside).expect("outside");
    Fixture {
        _tmp: tmp,
        root,
        outside,
    }
}

fn scan(root: &Path, follow: bool) -> Vec<String> {
    let ignore: Vec<String> = vec![];
    scan_extensions(root, 8, follow, None, &ignore)
        .expect("scan")
        .iter()
        .map(|f| format!("{f:?}"))
        .collect()
}

#[test]
fn a_symlinked_file_whose_target_escapes_the_root_is_not_discovered() {
    // The file branch is the one that matters: a symlinked directory that
    // escapes is a traversal problem, a symlinked module file is an execution
    // problem.
    let fx = fixture();
    fs::write(fx.outside.join("escape.rs"), "pub struct Escape;").expect("write");
    std::os::unix::fs::symlink(fx.outside.join("escape.rs"), fx.root.join("innocent.rs"))
        .expect("symlink");

    assert!(
        scan(&fx.root, true).is_empty(),
        "follow_symlinks=true must skip it"
    );
    assert!(
        scan(&fx.root, false).is_empty(),
        "follow_symlinks=false must skip it"
    );
}

#[test]
fn a_symlinked_directory_that_escapes_the_root_is_not_traversed() {
    let fx = fixture();
    fs::create_dir_all(fx.outside.join("pkg")).expect("pkg");
    fs::write(fx.outside.join("pkg/escape.rs"), "pub struct Escape;").expect("write");
    std::os::unix::fs::symlink(fx.outside.join("pkg"), fx.root.join("linked")).expect("symlink");

    assert!(scan(&fx.root, true).is_empty());
    assert!(scan(&fx.root, false).is_empty());
}

#[test]
fn a_real_file_inside_the_root_is_still_discovered() {
    // The control. Without it a scanner that discovers NOTHING passes both cases
    // above, and confinement would be indistinguishable from a broken scan.
    let fx = fixture();
    fs::write(fx.root.join("real.rs"), "pub struct Real;").expect("write");

    assert_eq!(scan(&fx.root, true).len(), 1);
    assert_eq!(scan(&fx.root, false).len(), 1);
}

#[test]
fn a_symlinked_module_uses_the_canonical_companion_file() {
    let fx = fixture();
    let module = fx.root.join("real.rs");
    let metadata = fx.root.join("real_meta.yaml");
    fs::write(&module, "pub struct Real;").expect("write");
    fs::write(&metadata, "description: Canonical metadata\n").expect("write");
    std::os::unix::fs::symlink(&module, fx.root.join("alias.rs")).expect("symlink");

    let modules = scan_extensions(&fx.root, 8, true, None, &[]).expect("scan");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].canonical_id, "real");
    assert_eq!(modules[0].meta_path, Some(metadata.canonicalize().unwrap()));
}

#[test]
fn test_companion_metadata_link_requires_opt_in() {
    let fx = fixture();
    fs::write(fx.root.join("real.rs"), "pub struct Real;").unwrap();
    let target = fx.root.join("metadata.yaml");
    fs::write(&target, "description: Module metadata\n").unwrap();
    std::os::unix::fs::symlink(&target, fx.root.join("real_meta.yaml")).unwrap();

    let modules = scan_extensions(&fx.root, 8, false, None, &[]).unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].meta_path, None);
    let modules = scan_extensions(&fx.root, 8, true, None, &[]).unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].meta_path, Some(target.canonicalize().unwrap()));
}

#[test]
fn a_symlink_whose_target_stays_inside_the_root_is_still_followed() {
    // The second control: confinement must reject what escapes, not every
    // symlink.
    let fx = fixture();
    fs::create_dir_all(fx.root.join("real")).expect("dir");
    fs::write(fx.root.join("real/inside.rs"), "pub struct Inside;").expect("write");
    std::os::unix::fs::symlink(fx.root.join("real"), fx.root.join("alias")).expect("symlink");

    assert!(!scan(&fx.root, true).is_empty());
}
