// APCore Protocol — Directory scanner for discovering extension modules
// Spec reference: Module discovery via filesystem scanning

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::errors::{ErrorCode, ModuleError};
use crate::registry::types::DiscoveredFile;

/// Directory names to skip during scanning.
const SKIP_DIR_NAMES: &[&str] = &["__pycache__", "node_modules", "target", ".git"];

/// File suffixes to skip during scanning.
const SKIP_FILE_SUFFIXES: &[&str] = &[".pyc", ".pyo"];

/// Default file extensions considered as module source files.
///
/// In Rust-centric discovery this would typically be `[".rs"]`, but the
/// scanner is designed to be language-agnostic so callers may override.
const DEFAULT_MODULE_EXTENSIONS: &[&str] = &[".rs"];

/// Recursively scan an extensions directory for module files.
///
/// Aligned with `apcore-python.scan_extensions` and
/// `apcore-typescript.scanExtensions`.
/// `ignore_patterns` is `extensions.ignore_patterns`, matched with Algorithm
/// A25 (`PROTOCOL_SPEC` §9.2.3) against the ENTRY NAME — one path segment,
/// never a path, case-sensitively. It is a UNION with [`SKIP_DIR_NAMES`] and
/// §3.5's hidden/internal prefixes: a configured pattern adds to those and
/// cannot switch one off.
///
/// Until spec v1.42.0 the key was registered in every SDK's configuration key
/// surface and read by none, so §3.6 step 3a was a MUST whose input nothing
/// supplied and a directory a project had excluded from discovery was scanned
/// and registered anyway — a skip rule that failed OPEN (aiperceivable/apcore#118).
pub fn scan_extensions(
    root: &Path,
    max_depth: u32,
    follow_symlinks: bool,
    extensions: Option<&[&str]>,
    ignore_patterns: &[String],
) -> Result<Vec<DiscoveredFile>, ModuleError> {
    let root = root.canonicalize().map_err(|e| {
        ModuleError::new(
            ErrorCode::ConfigNotFound,
            format!("Extensions root not found: {} ({})", root.display(), e),
        )
    })?;

    let ext_list = extensions.unwrap_or(DEFAULT_MODULE_EXTENSIONS);
    let mut results: Vec<DiscoveredFile> = Vec::new();
    let mut seen_ids: HashMap<String, PathBuf> = HashMap::new();
    let mut seen_ids_lower: HashMap<String, String> = HashMap::new();
    let mut visited_real_paths: HashSet<PathBuf> = HashSet::new();
    visited_real_paths.insert(root.clone());

    scan_dir(
        &root,
        &root,
        1,
        max_depth,
        follow_symlinks,
        ext_list,
        ignore_patterns,
        &mut results,
        &mut seen_ids,
        &mut seen_ids_lower,
        &mut visited_real_paths,
    );

    Ok(results)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)] // complex filesystem traversal with symlink/duplicate/case checks
fn scan_dir(
    root: &Path,
    dir_path: &Path,
    depth: u32,
    max_depth: u32,
    follow_symlinks: bool,
    extensions: &[&str],
    ignore_patterns: &[String],
    results: &mut Vec<DiscoveredFile>,
    seen_ids: &mut HashMap<String, PathBuf>,
    seen_ids_lower: &mut HashMap<String, String>,
    visited_real_paths: &mut HashSet<PathBuf>,
) {
    if depth > max_depth {
        tracing::info!(
            "Max depth {} exceeded at {}, skipping",
            max_depth,
            dir_path.display()
        );
        return;
    }

    let entries = match std::fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Error scanning directory {}: {}", dir_path.display(), e);
            return;
        }
    };

    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("Error reading directory entry: {}", e);
                continue;
            }
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden and private entries
        if name_str.starts_with('.') || name_str.starts_with('_') {
            continue;
        }
        // Empty entries are dropped rather than treated as a pattern: A25
        // anchors, so `""` would match only the empty name, and an operator who
        // leaves a blank line in a YAML list means nothing by it.
        if ignore_patterns
            .iter()
            .any(|p| !p.is_empty() && crate::utils::helpers::match_glob(p, &name_str))
        {
            continue;
        }
        if SKIP_DIR_NAMES.contains(&name_str.as_ref()) {
            continue;
        }

        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                tracing::error!("Error accessing {}: {}", entry_path.display(), e);
                continue;
            }
        };

        if file_type.is_dir() || (file_type.is_symlink() && follow_symlinks) {
            if file_type.is_symlink() {
                if !follow_symlinks {
                    continue;
                }
                match entry_path.canonicalize() {
                    Ok(real) => {
                        // Confinement check — reject symlinks whose target escapes the extension root
                        if !real.starts_with(root) {
                            tracing::warn!(
                                "Symlink target outside extension root, skipping: {:?} -> {:?}",
                                entry_path,
                                real
                            );
                            continue;
                        }
                        if visited_real_paths.contains(&real) {
                            tracing::warn!(
                                "Symlink cycle detected at {} -> {}, skipping",
                                entry_path.display(),
                                real.display()
                            );
                            continue;
                        }
                        visited_real_paths.insert(real);
                    }
                    Err(_) => continue,
                }
            }
            if entry_path.is_dir() {
                scan_dir(
                    root,
                    &entry_path,
                    depth + 1,
                    max_depth,
                    follow_symlinks,
                    extensions,
                    ignore_patterns,
                    results,
                    seen_ids,
                    seen_ids_lower,
                    visited_real_paths,
                );
            }
        } else if file_type.is_file() || (file_type.is_symlink() && follow_symlinks) {
            // D-127: a symlinked FILE reaches here too. It used to be routed into
            // the directory branch above by `is_dir() || (is_symlink() && follow)`,
            // where `entry_path.is_dir()` is false and it fell out unappended —
            // so `follow_symlinks` governed directories and did nothing for
            // files, which is a declared key that does not reach its mechanism.
            let ext = entry_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"));

            // Check skip suffixes
            if let Some(ref ext_str) = ext {
                if SKIP_FILE_SUFFIXES.contains(&ext_str.as_str()) {
                    continue;
                }
            }

            // Check allowed extensions
            let matches_ext = match &ext {
                Some(e) => extensions.contains(&e.as_str()),
                None => false,
            };
            if !matches_ext {
                continue;
            }

            // D-127: identity and the module ID are keyed on the CANONICAL
            // REAL PATH, never on whichever alias the traversal reached first.
            // Deriving the ID from the alias makes the registered ID depend on
            // directory iteration order, which is not stable across filesystems
            // or platforms — and recording both paths made one file two modules
            // with different IDs, which the duplicate check cannot catch
            // precisely because they differ.
            let Ok(real_file) = entry_path.canonicalize() else {
                continue;
            };
            if visited_real_paths.contains(&real_file) {
                continue;
            }
            let Ok(rel) = real_file.strip_prefix(root) else {
                // Containment already ran before the dir/file split (D-94).
                tracing::warn!(
                    "Resolved path escapes the extensions root, skipping: {}",
                    entry_path.display()
                );
                continue;
            };
            let rel = rel.to_path_buf();
            visited_real_paths.insert(real_file.clone());
            let canonical_id = rel
                .with_extension("")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, ".");

            // Duplicate check
            if let Some(existing_path) = seen_ids.get(&canonical_id) {
                tracing::error!(
                    "Duplicate module ID '{}' at {}, already found at {}. Skipping.",
                    canonical_id,
                    entry_path.display(),
                    existing_path.display()
                );
                continue;
            }

            // Case collision warning
            let lower_id = canonical_id.to_lowercase();
            if let Some(existing_id) = seen_ids_lower.get(&lower_id) {
                if *existing_id != canonical_id {
                    tracing::warn!(
                        "Case collision: '{}' and '{}' differ only by case",
                        canonical_id,
                        existing_id
                    );
                }
            }

            // Check for companion _meta.yaml
            let meta_path = entry_path.with_file_name(format!(
                "{}_meta.yaml",
                entry_path.file_stem().unwrap_or_default().to_string_lossy()
            ));
            let meta_path = if meta_path.exists() {
                Some(meta_path)
            } else {
                None
            };

            seen_ids.insert(canonical_id.clone(), entry_path.clone());
            seen_ids_lower.insert(lower_id, canonical_id.clone());

            results.push(DiscoveredFile {
                // D-127: the REAL path, so the loader opens the target rather
                // than the alias — the alias is a name, not the module.
                file_path: real_file,
                canonical_id,
                meta_path,
                namespace: None,
            });
        }
    }
}

/// Scan multiple extension roots with namespace prefixing.
///
/// Each entry in `roots` must have a `"root"` key (path string) and an
/// optional `"namespace"` key (defaults to the directory name).
///
/// Aligned with `apcore-python.scan_multi_root` and
/// `apcore-typescript.scanMultiRoot`.
pub fn scan_multi_root<S: std::hash::BuildHasher>(
    roots: &[HashMap<String, String, S>],
    max_depth: u32,
    follow_symlinks: bool,
    extensions: Option<&[&str]>,
    ignore_patterns: &[String],
) -> Result<Vec<DiscoveredFile>, ModuleError> {
    let mut all_results: Vec<DiscoveredFile> = Vec::new();
    let mut seen_namespaces: HashSet<String> = HashSet::new();

    // Validate all namespaces before scanning
    let mut resolved: Vec<(PathBuf, String)> = Vec::new();
    for entry in roots {
        let root_str = entry.get("root").ok_or_else(|| {
            ModuleError::new(
                ErrorCode::ConfigInvalid,
                "Multi-root entry missing 'root' key",
            )
        })?;
        let root_path = PathBuf::from(root_str);
        let namespace = entry.get("namespace").cloned().unwrap_or_else(|| {
            root_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        });
        if seen_namespaces.contains(&namespace) {
            return Err(ModuleError::new(
                ErrorCode::ConfigInvalid,
                format!("Duplicate namespace: '{namespace}'"),
            ));
        }
        seen_namespaces.insert(namespace.clone());
        resolved.push((root_path, namespace));
    }

    for (root_path, namespace) in resolved {
        let modules = scan_extensions(
            &root_path,
            max_depth,
            follow_symlinks,
            extensions,
            ignore_patterns,
        )?;
        for m in modules {
            all_results.push(DiscoveredFile {
                file_path: m.file_path,
                canonical_id: format!("{}.{}", namespace, m.canonical_id),
                meta_path: m.meta_path,
                namespace: Some(namespace.clone()),
            });
        }
    }

    Ok(all_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temporary directory tree for scanning tests.
    fn make_test_dir(base: &std::path::Path, files: &[&str]) {
        for file in files {
            let path = base.join(file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, b"// test module").unwrap();
        }
    }

    #[test]
    fn scan_nonexistent_root_returns_error() {
        let path = std::path::Path::new("/tmp/apcore_test_nonexistent_xyz_abc");
        let result = scan_extensions(path, 5, false, None, &[]);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not found") || msg.contains("No such file"));
    }

    #[test]
    fn scan_empty_directory_returns_no_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = scan_extensions(tmp.path(), 5, false, None, &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn scan_returns_rs_files_with_canonical_ids() {
        let tmp = tempfile::tempdir().expect("tempdir");
        make_test_dir(tmp.path(), &["email/send.rs", "math/add.rs"]);

        let result = scan_extensions(tmp.path(), 5, false, None, &[]).unwrap();
        let ids: Vec<&str> = result.iter().map(|f| f.canonical_id.as_str()).collect();

        assert!(ids.contains(&"email.send"), "email/send.rs → email.send");
        assert!(ids.contains(&"math.add"), "math/add.rs → math.add");
    }

    #[test]
    fn scan_respects_max_depth() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // depth-1 file
        make_test_dir(tmp.path(), &["shallow.rs"]);
        // depth-2 file (nested)
        make_test_dir(tmp.path(), &["deep/nested.rs"]);

        // max_depth=1 — only the top level file; subdirectory is allowed at depth 1
        // but files inside it are at depth 2 and should be included when max_depth=2
        let result_shallow = scan_extensions(tmp.path(), 1, false, None, &[]).unwrap();
        let result_deep = scan_extensions(tmp.path(), 2, false, None, &[]).unwrap();

        let shallow_ids: Vec<&str> = result_shallow
            .iter()
            .map(|f| f.canonical_id.as_str())
            .collect();
        let deep_ids: Vec<&str> = result_deep
            .iter()
            .map(|f| f.canonical_id.as_str())
            .collect();

        assert!(
            shallow_ids.contains(&"shallow"),
            "shallow.rs should always be found"
        );
        assert!(
            !shallow_ids.contains(&"deep.nested"),
            "deep/nested.rs too deep for max_depth=1"
        );
        assert!(
            deep_ids.contains(&"deep.nested"),
            "deep/nested.rs included when max_depth=2"
        );
    }

    #[test]
    fn scan_skips_hidden_files_and_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        make_test_dir(tmp.path(), &[".hidden/module.rs", "visible.rs"]);
        let result = scan_extensions(tmp.path(), 5, false, None, &[]).unwrap();
        let ids: Vec<&str> = result.iter().map(|f| f.canonical_id.as_str()).collect();
        assert!(ids.contains(&"visible"), "visible.rs should be found");
        assert!(
            !ids.iter().any(|id| id.contains("hidden")),
            "hidden dir should be skipped"
        );
    }

    #[test]
    fn scan_skips_underscore_prefixed_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        make_test_dir(tmp.path(), &["_private.rs", "public.rs"]);
        let result = scan_extensions(tmp.path(), 5, false, None, &[]).unwrap();
        let ids: Vec<&str> = result.iter().map(|f| f.canonical_id.as_str()).collect();
        assert!(ids.contains(&"public"), "public.rs should be found");
        assert!(!ids.contains(&"_private"), "_private.rs should be skipped");
    }

    #[test]
    fn scan_custom_extension_filter() {
        let tmp = tempfile::tempdir().expect("tempdir");
        make_test_dir(tmp.path(), &["module.py", "module.rs"]);
        let result = scan_extensions(tmp.path(), 5, false, Some(&[".py"]), &[]).unwrap();
        let ids: Vec<&str> = result.iter().map(|f| f.canonical_id.as_str()).collect();
        assert!(ids.contains(&"module"), "module.py should match .py filter");
        // When filtering for .py, .rs files should NOT appear
        assert_eq!(result.len(), 1, "only one file should match");
    }

    #[test]
    fn scan_multi_root_prefixes_with_namespace() {
        let tmp1 = tempfile::tempdir().expect("tempdir 1");
        let tmp2 = tempfile::tempdir().expect("tempdir 2");
        make_test_dir(tmp1.path(), &["add.rs"]);
        make_test_dir(tmp2.path(), &["send.rs"]);

        let roots: Vec<HashMap<String, String>> = vec![
            [
                (
                    "root".to_string(),
                    tmp1.path().to_string_lossy().into_owned(),
                ),
                ("namespace".to_string(), "math".to_string()),
            ]
            .into_iter()
            .collect(),
            [
                (
                    "root".to_string(),
                    tmp2.path().to_string_lossy().into_owned(),
                ),
                ("namespace".to_string(), "email".to_string()),
            ]
            .into_iter()
            .collect(),
        ];

        let result = scan_multi_root(&roots, 5, false, None, &[]).unwrap();
        let ids: Vec<&str> = result.iter().map(|f| f.canonical_id.as_str()).collect();

        assert!(ids.contains(&"math.add"), "math namespace should prefix");
        assert!(ids.contains(&"email.send"), "email namespace should prefix");
    }

    #[test]
    fn scan_multi_root_rejects_duplicate_namespaces() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let roots: Vec<HashMap<String, String>> = vec![
            [
                (
                    "root".to_string(),
                    tmp.path().to_string_lossy().into_owned(),
                ),
                ("namespace".to_string(), "same".to_string()),
            ]
            .into_iter()
            .collect(),
            [
                (
                    "root".to_string(),
                    tmp.path().to_string_lossy().into_owned(),
                ),
                ("namespace".to_string(), "same".to_string()),
            ]
            .into_iter()
            .collect(),
        ];
        let result = scan_multi_root(&roots, 5, false, None, &[]);
        assert!(result.is_err(), "duplicate namespace should fail");
    }

    #[test]
    fn scan_multi_root_requires_root_key() {
        let roots: Vec<HashMap<String, String>> =
            vec![[("namespace".to_string(), "ns".to_string())]
                .into_iter()
                .collect()];
        let result = scan_multi_root(&roots, 5, false, None, &[]);
        assert!(result.is_err(), "missing root key should fail");
    }
}
