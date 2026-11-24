// APCore Protocol — Default discovery pipeline
//
// Wires the building blocks (scanner, metadata, dependencies, validation,
// conflicts) into a Discoverer impl that runs the canonical 8-stage pipeline
// described in `registry-system.md §Discovery`. Cross-language parity with
// apcore-python's `Registry._discover_default` and apcore-typescript's
// `Registry._discoverDefault` (sync finding A-D-005).
//
// Rust limitation: unlike Python/TS which can `import` a discovered file at
// runtime, Rust modules are compiled at build time. The `DefaultDiscoverer`
// therefore takes a user-provided `module_factory` callback that maps
// `(canonical_id, entry_point_name)` to an `Arc<dyn Module>` — typically a
// closure that dispatches over a HashMap of pre-instantiated modules. This
// preserves the spec's pipeline structure (and the spec-mandated error
// surfaces: ConfigNotFoundError, CircularDependencyError) while staying
// honest about what Rust can do.
//
// The pipeline stages (matching the spec):
//   1. scan_extensions          (filesystem scan of every root)
//   2. apply_id_map_overrides   (rewrite canonical_ids per id_map.yaml)
//   3. load_all_metadata        (per-file `_meta.yaml`)
//   4. resolve_entry_points     (struct name from metadata or auto-inferred)
//   5. validate_descriptors     (descriptor structure)
//   6. resolve_load_order       (Kahn's topo sort, dependency version checks)
//   7. detect_id_conflicts      (case-insensitive duplicates, reserved words)
//   8. instantiate via module_factory + build DiscoveredModule list
//
// Stage 6 is where `CircularDependencyError` surfaces. Stage 1 is where
// `ConfigNotFoundError` surfaces (root path missing).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::errors::{ErrorCode, ModuleError};
use crate::module::Module;
use crate::registry::dependencies::resolve_dependencies;
use crate::registry::metadata::{load_id_map, load_metadata, parse_dependencies};
use crate::registry::registry::{
    is_ephemeral_module_id, DependencyInfo, DiscoveredModule, Discoverer, ModuleDescriptor,
    EPHEMERAL_NAMESPACE_PREFIX, MAX_MODULE_ID_LENGTH, RESERVED_WORDS,
};
use crate::registry::scanner::scan_extensions;
use crate::registry::types::{DepInfo, DiscoveredFile};

/// Closure type used by `DefaultDiscoverer` to instantiate a module from its
/// resolved entry-point name. Returning `Ok(None)` causes the file to be
/// skipped silently with a `tracing::warn` (the file was discovered but the
/// host application has no factory registered for it). Returning `Err(...)`
/// aborts the discovery pipeline.
pub type ModuleFactory = Arc<
    dyn Fn(&DiscoveredFile, &str) -> Result<Option<Arc<dyn Module>>, ModuleError> + Send + Sync,
>;

/// Default Discoverer implementation matching the Python/TS 8-stage pipeline.
///
/// Cross-language parity with apcore-python `Registry._discover_default`
/// and apcore-typescript `Registry._discoverDefault` (sync finding A-D-005).
///
/// # Construction
///
/// ```ignore
/// use std::sync::Arc;
/// use apcore::DefaultDiscoverer;
///
/// let factory = Arc::new(|file, entry_point: &str| {
///     // Look up `entry_point` in your application's module registry,
///     // construct an `Arc<dyn Module>`, and return it.
///     Ok(None)
/// });
///
/// let discoverer = DefaultDiscoverer::new()
///     .with_id_map(Some("path/to/id_map.yaml"))
///     .with_extensions(&[".toml"])
///     .with_factory(factory);
/// ```
pub struct DefaultDiscoverer {
    /// Optional path to an id_map.yaml file used at stage 2.
    id_map_path: Option<PathBuf>,
    /// `extensions.roots`' namespace half, keyed by root path (#118, D-70).
    ///
    /// The `Discoverer` trait takes `roots: &[String]` and is public, so the
    /// namespaces cannot ride along with the paths without a breaking change.
    /// They travel here instead, read from the same key by
    /// [`Self::from_config`], and are paired back up in `discover`. A root this
    /// map does not know simply has no namespace, so a caller passing roots
    /// explicitly is unaffected.
    root_namespaces: HashMap<String, String>,
    /// File extensions considered as module candidates. Defaults to `[".rs"]`.
    extensions: Vec<String>,
    /// Maximum directory depth for filesystem scan.
    max_depth: u32,
    /// Whether to follow symlinks during scan.
    follow_symlinks: bool,
    /// `extensions.ignore_patterns` — A25 globs matched against the ENTRY NAME
    /// (`PROTOCOL_SPEC` §3.5 / §9.2.3), a union with the built-in skip rows.
    ignore_patterns: Vec<String>,
    /// User-provided factory that turns a discovered entry point into a
    /// live `Arc<dyn Module>`. `None` until [`Self::with_factory`]: Rust
    /// cannot load a module from its source file, so without one a discovered
    /// file is an error rather than a module silently not registered.
    factory: Option<ModuleFactory>,
}

impl std::fmt::Debug for DefaultDiscoverer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultDiscoverer")
            .field("id_map_path", &self.id_map_path)
            .field("root_namespaces", &self.root_namespaces)
            .field("extensions", &self.extensions)
            .field("max_depth", &self.max_depth)
            .field("follow_symlinks", &self.follow_symlinks)
            .field("ignore_patterns", &self.ignore_patterns)
            .field("factory", &"<ModuleFactory>")
            .finish()
    }
}

impl DefaultDiscoverer {
    /// Create a new `DefaultDiscoverer` with no module factory. Supply one
    /// with [`Self::with_factory`] before passing this to a `Registry`:
    /// discovering a module file without one fails with
    /// [`ErrorCode::ModuleLoadError`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            id_map_path: None,
            root_namespaces: HashMap::new(),
            extensions: vec![".rs".to_string()],
            max_depth: 8,
            follow_symlinks: false,
            ignore_patterns: Vec::new(),
            factory: None,
        }
    }

    /// Set the optional id_map.yaml path (stage 2 override mappings).
    #[must_use]
    pub fn with_id_map(mut self, path: Option<impl AsRef<Path>>) -> Self {
        self.id_map_path = path.map(|p| p.as_ref().to_path_buf());
        self
    }

    /// Override the default file extensions (default: `[".rs"]`).
    #[must_use]
    pub fn with_extensions(mut self, exts: &[&str]) -> Self {
        self.extensions = exts.iter().map(|s| (*s).to_string()).collect();
        self
    }

    /// Set the maximum directory depth for the filesystem scan (default: 8).
    #[must_use]
    pub fn with_max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }

    /// Build a discoverer whose scan parameters come from a [`Config`].
    ///
    /// The reason this exists rather than only the builders below: until spec
    /// v1.42.0 apcore-rust had NO path from a `Config` to the scanner at all.
    /// `max_depth`, `follow_symlinks` and `ignore_patterns` were builder
    /// options an application had to pass by hand, so the three declared
    /// configuration keys reached nothing here while apcore-python and
    /// apcore-typescript read all three — the shape recorded as decision D-73
    /// (a declared key MUST reach its mechanism from a `Config`).
    ///
    /// Explicit builder calls made afterwards still win, per D-73's precedence:
    /// an API argument beats `Config`, which beats the declared default.
    #[must_use]
    pub fn from_config(config: &crate::config::Config) -> Self {
        let mut me = Self::new();
        if let Some(v) = config.get("extensions.max_depth").and_then(|v| v.as_u64()) {
            me.max_depth = u32::try_from(v).unwrap_or(8);
        }
        if let Some(v) = config
            .get("extensions.follow_symlinks")
            .and_then(|v| v.as_bool())
        {
            me.follow_symlinks = v;
        }
        if let Some(items) = config
            .get("extensions.ignore_patterns")
            .and_then(|v| v.as_array().cloned())
        {
            me.ignore_patterns = items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
        }
        // `id_map.overrides` (apcore#118, decision D-71). PROTOCOL_SPEC §9.1.1
        // declared it and nothing read it: the MECHANISM is stage 2 of this
        // discoverer and the map arrived only through `with_id_map`. Measured
        // before the fix — with the key pointing at a map that renames
        // `executor/orig/mod.py`, discovery still registered
        // `executor.orig.mod`.
        //
        // `with_id_map` after this still wins, per D-73's precedence and
        // exactly as the other four builder options above behave.
        // `extensions.roots`' namespace half (#118, decision D-70). The paths
        // reach the registry through `set_extension_roots_from_config`; this is
        // the other half of the same key, which apcore-rust dropped entirely —
        // it honoured "multiple roots" and not the "namespace isolation" the
        // schema's own description leads with, so module IDs were unprefixed
        // here and prefixed in the other two SDKs.
        if let Some(items) = config.get("extensions.roots").and_then(|v| match v {
            serde_json::Value::Array(a) => Some(a.clone()),
            _ => None,
        }) {
            for item in items {
                match item {
                    serde_json::Value::String(root) if !root.trim().is_empty() => {
                        // A bare path: the namespace is the last path segment,
                        // which `scan_multi_root` derives itself.
                        let derived = Path::new(&root)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned());
                        if let Some(ns) = derived {
                            me.root_namespaces.insert(root, ns);
                        }
                    }
                    serde_json::Value::Object(obj) => {
                        let Some(root) = obj.get("root").and_then(|r| r.as_str()) else {
                            continue;
                        };
                        let ns = obj
                            .get("namespace")
                            .and_then(|n| n.as_str())
                            .map(str::to_owned)
                            .or_else(|| {
                                Path::new(root)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                            });
                        if let Some(ns) = ns {
                            me.root_namespaces.insert(root.to_string(), ns);
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(declared) = config
            .get("id_map.overrides")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|s| !s.trim().is_empty())
        {
            // Used AS DECLARED: a relative value resolves against the process
            // working directory, exactly as `extensions.root` does. §9.2.1
            // leaves the base for path-typed keys deliberately unspecified
            // (#113) — `acl.root` uses the config file's directory,
            // `schema.root` the CWD — and this key follows its SIBLING rather
            // than settling that: the two are halves of one discovery
            // configuration, so a split base between them is worse than either.
            me.id_map_path = Some(PathBuf::from(declared));
        }
        me
    }

    /// Set the A25 ignore patterns explicitly (`extensions.ignore_patterns`).
    #[must_use]
    pub fn with_ignore_patterns<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.ignore_patterns = patterns.into_iter().map(Into::into).collect();
        self
    }

    /// Whether to follow symlinks during scanning (default: false).
    #[must_use]
    pub fn with_follow_symlinks(mut self, follow: bool) -> Self {
        self.follow_symlinks = follow;
        self
    }

    /// Set the module factory closure.
    #[must_use]
    pub fn with_factory(mut self, factory: ModuleFactory) -> Self {
        self.factory = Some(factory);
        self
    }
}

impl Default for DefaultDiscoverer {
    fn default() -> Self {
        Self::new()
    }
}

/// `file`'s path relative to the extension root it lies under, `/`-separated.
///
/// The longest matching root wins, so a root nested inside another resolves to
/// the inner one. Roots and files are compared as given and, failing that,
/// canonicalised, since a scanner that follows symlinks reports resolved paths.
fn path_relative_to_root(file: &Path, roots: &[PathBuf]) -> Option<String> {
    let canonical_file = std::fs::canonicalize(file).ok();
    roots
        .iter()
        .filter_map(|root| {
            let rel = file
                .strip_prefix(root)
                .ok()
                .map(Path::to_path_buf)
                .or_else(|| {
                    let root = std::fs::canonicalize(root).ok()?;
                    canonical_file
                        .as_ref()?
                        .strip_prefix(root)
                        .ok()
                        .map(Path::to_path_buf)
                })?;
            Some((root.components().count(), rel))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, rel)| {
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
}

/// Internal: discovery output for one file before the topo-sort stage.
struct Pending {
    file: DiscoveredFile,
    module: Arc<dyn Module>,
    descriptor: ModuleDescriptor,
    deps: Vec<DepInfo>,
}

#[async_trait]
impl Discoverer for DefaultDiscoverer {
    #[allow(clippy::too_many_lines)] // 8-stage pipeline naturally inlines stage steps
    async fn discover(&self, roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
        // Stage 1: scan every root recursively.
        let mut discovered_files: Vec<DiscoveredFile> = Vec::new();
        let ext_refs: Vec<&str> = self.extensions.iter().map(String::as_str).collect();

        // The same dispatch apcore-python and apcore-typescript make: any root
        // carrying a namespace puts the whole scan through `scan_multi_root`,
        // which prefixes each module ID and rejects a duplicate namespace
        // before scanning anything.
        if roots.iter().any(|r| self.root_namespaces.contains_key(r)) {
            let entries: Vec<HashMap<String, String>> = roots
                .iter()
                .map(|root| {
                    let mut entry = HashMap::new();
                    entry.insert("root".to_string(), root.clone());
                    if let Some(ns) = self.root_namespaces.get(root) {
                        entry.insert("namespace".to_string(), ns.clone());
                    }
                    entry
                })
                .collect();
            discovered_files = crate::registry::scanner::scan_multi_root(
                &entries,
                self.max_depth,
                self.follow_symlinks,
                Some(&ext_refs),
                &self.ignore_patterns,
            )?;
        } else {
            for root in roots {
                let path = Path::new(root);
                // scan_extensions returns ConfigNotFoundError when the root is
                // missing — exactly the spec contract for discover().
                let mut files = scan_extensions(
                    path,
                    self.max_depth,
                    self.follow_symlinks,
                    Some(&ext_refs),
                    &self.ignore_patterns,
                )?;
                discovered_files.append(&mut files);
            }
        }

        // Stage 2: apply id_map overrides if configured.
        let id_overrides: HashMap<String, HashMap<String, serde_json::Value>> =
            match &self.id_map_path {
                Some(path) => load_id_map(path)?,
                None => HashMap::new(),
            };
        // An entry's `file` is relative to the extension root the file is
        // discovered under (§2.2 rule 4, D-138) — never to the working
        // directory, and never the file's absolute path.
        let root_paths: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
        for file in &mut discovered_files {
            let Some(relative) = path_relative_to_root(&file.file_path, &root_paths) else {
                continue;
            };
            if let Some(override_entry) = id_overrides.get(&relative) {
                if let Some(new_id) = override_entry.get("id").and_then(|v| v.as_str()) {
                    file.canonical_id = new_id.to_string();
                }
            }
        }

        // Stage 3: load companion `*_meta.yaml` for each file.
        let mut metadata_per_file: HashMap<PathBuf, HashMap<String, serde_json::Value>> =
            HashMap::new();
        for file in &discovered_files {
            if let Some(meta_path) = &file.meta_path {
                let meta = load_metadata(meta_path)?;
                metadata_per_file.insert(file.file_path.clone(), meta);
            }
        }

        // Reject filesystem-derived IDs in the reserved ephemeral.* namespace.
        //
        // Per protocol-spec §2.5.1, `ephemeral.*` is reserved for programmatically-registered
        // modules synthesized at runtime. Any filesystem layout that produces
        // such an ID is a configuration error; we surface it as a hard
        // ModuleError to match apcore-python's `_reject_ephemeral_discoveries`
        // (which raises InvalidInputError).
        let offenders: Vec<String> = discovered_files
            .iter()
            .filter(|dm| is_ephemeral_module_id(&dm.canonical_id))
            .map(|dm| dm.canonical_id.clone())
            .collect();
        if !offenders.is_empty() {
            let mut sorted = offenders;
            sorted.sort();
            sorted.dedup();
            return Err(ModuleError::new(
                ErrorCode::InvalidModuleId,
                format!(
                    "Filesystem discovery produced module ID(s) in the reserved \
                     '{EPHEMERAL_NAMESPACE_PREFIX}*' namespace: {sorted:?}. \
                     The ephemeral.* namespace is reserved for \
                     programmatically-registered modules and may only be used \
                     via Registry::register(). Rename the offending directory \
                     or extension namespace."
                ),
            ));
        }

        // Stage 4 + 5: resolve entry-point name, validate, and instantiate via
        // the user-provided factory. We collect (canonical_id, descriptor,
        // module, deps) here; the topo-sort runs in stage 6.
        let mut pending: Vec<Pending> = Vec::new();
        for file in discovered_files {
            // Reserved-word check on the canonical_id (first segment).
            let first_seg = file.canonical_id.split('.').next().unwrap_or("");
            if RESERVED_WORDS.contains(&first_seg) {
                tracing::warn!(
                    canonical_id = %file.canonical_id,
                    "Skipping discovered file: first segment is a reserved word"
                );
                continue;
            }
            if file.canonical_id.len() > MAX_MODULE_ID_LENGTH {
                tracing::warn!(
                    canonical_id = %file.canonical_id,
                    "Skipping discovered file: module_id exceeds {MAX_MODULE_ID_LENGTH} chars"
                );
                continue;
            }

            // Stage 4: resolve entry-point name.
            let meta = metadata_per_file.get(&file.file_path);
            let entry_point_name =
                crate::registry::entry_point::resolve_entry_point_name(&file.file_path, meta)?
                    .unwrap_or_else(|| {
                        crate::registry::entry_point::infer_struct_name(&file.file_path)
                    });

            // Stage 4 (continued): instantiate via factory.
            let Some(factory) = self.factory.as_ref() else {
                return Err(ModuleError::new(
                    ErrorCode::ModuleLoadError,
                    format!(
                        "DefaultDiscoverer found '{}' but has no module factory. Rust cannot \
                         load a module from its source file; supply one with with_factory().",
                        file.file_path.display()
                    ),
                ));
            };
            let Some(module) = factory(&file, &entry_point_name)? else {
                tracing::debug!(
                    canonical_id = %file.canonical_id,
                    entry_point = %entry_point_name,
                    "DefaultDiscoverer factory returned None — skipping"
                );
                continue;
            };

            // Parse declared dependencies ONCE, before the descriptor is built.
            // They have two consumers, and until this fix only the second one
            // saw them: stage 6's load-order topo sort below, and the descriptor
            // itself — which is what `Registry::register_discovered` stores and
            // `Registry::get_definition` hands back. `build_descriptor` used to
            // hard-code an empty list, so every filesystem-discovered module
            // reported no dependencies after registration even though discovery
            // had just sorted by them. `ReloadModule::topo_sort_modules` reads
            // that accessor, so a `path_filter` reload silently degenerated to
            // alphabetical order.
            let deps = if let Some(m) = meta {
                m.get("dependencies")
                    .and_then(|v| v.as_array())
                    .map(|arr| parse_dependencies(arr))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            // Stage 5: build the descriptor from module + metadata (YAML wins).
            let descriptor = build_descriptor(&file, module.as_ref(), meta, &deps);

            pending.push(Pending {
                file,
                module,
                descriptor,
                deps,
            });
        }

        // Stage 7: detect id conflicts (case-insensitive duplicates).
        let mut seen_ids_lower: HashSet<String> = HashSet::new();
        for p in &pending {
            let lower = p.file.canonical_id.to_lowercase();
            if !seen_ids_lower.insert(lower.clone()) {
                return Err(ModuleError::new(
                    ErrorCode::ModuleIdConflict,
                    format!(
                        "Duplicate module ID '{}' (case-insensitive) discovered in roots {:?}",
                        p.file.canonical_id, roots,
                    ),
                ));
            }
        }

        // Stage 6: dependency topo sort. resolve_dependencies surfaces
        // CircularDependencyError when Kahn's queue empties before all nodes
        // are processed — matches the spec contract for discover().
        let modules_with_deps: Vec<(String, Vec<DepInfo>)> = pending
            .iter()
            .map(|p| (p.file.canonical_id.clone(), p.deps.clone()))
            .collect();
        let module_versions: HashMap<String, String> = pending
            .iter()
            .map(|p| (p.file.canonical_id.clone(), p.descriptor.version.clone()))
            .collect();
        let load_order = resolve_dependencies(&modules_with_deps, None, Some(&module_versions))?;

        // Stage 8: produce DiscoveredModule entries in dependency order.
        let by_id: HashMap<String, Pending> = pending
            .into_iter()
            .map(|p| (p.file.canonical_id.clone(), p))
            .collect();

        let mut result = Vec::with_capacity(load_order.len());
        for id in load_order {
            if let Some(p) = by_id.get(&id) {
                result.push(DiscoveredModule {
                    name: p.file.canonical_id.clone(),
                    source: p.file.file_path.to_string_lossy().into_owned(),
                    descriptor: p.descriptor.clone(),
                    module: Arc::clone(&p.module),
                });
            }
        }
        Ok(result)
    }
}

/// Build a `ModuleDescriptor` from the live module + optional YAML metadata.
///
/// `deps` is the caller's already-parsed view of the metadata's `dependencies`
/// array (see the call site) — passed in rather than re-parsed here so the
/// descriptor and the stage-6 load order are built from one parse, and cannot
/// drift apart.
fn build_descriptor(
    file: &DiscoveredFile,
    module: &dyn Module,
    meta: Option<&HashMap<String, serde_json::Value>>,
    deps: &[DepInfo],
) -> ModuleDescriptor {
    let yaml = meta.cloned().unwrap_or_default();

    let description = yaml
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map_or_else(|| module.description().to_string(), str::to_string);

    let version = yaml
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("1.0.0")
        .to_string();

    let tags: Vec<String> = yaml
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let documentation = yaml
        .get("documentation")
        .and_then(|v| v.as_str())
        .map(String::from);

    ModuleDescriptor {
        module_id: file.canonical_id.clone(),
        name: yaml.get("name").and_then(|v| v.as_str()).map(String::from),
        description,
        documentation,
        input_schema: module.input_schema(),
        output_schema: module.output_schema(),
        version,
        tags,
        annotations: None,
        examples: vec![],
        metadata: yaml,
        display: None,
        sunset_date: None,
        dependencies: deps.iter().cloned().map(DependencyInfo::from).collect(),
        enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    #[derive(Debug)]
    struct TestModule {
        desc: String,
    }

    #[async_trait]
    impl Module for TestModule {
        fn description(&self) -> &str {
            &self.desc
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn output_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            Ok(json!({}))
        }
    }

    #[tokio::test]
    async fn missing_root_yields_config_not_found() {
        let discoverer = DefaultDiscoverer::new();
        let result = discoverer
            .discover(&["/this/path/does/not/exist".to_string()])
            .await;
        let err = result.expect_err("missing root should error");
        assert_eq!(err.code, ErrorCode::ConfigNotFound);
    }

    #[tokio::test]
    async fn empty_root_yields_empty_discovery() {
        let tmp = tempdir().unwrap();
        let discoverer = DefaultDiscoverer::new();
        let result = discoverer
            .discover(&[tmp.path().to_string_lossy().into_owned()])
            .await
            .unwrap();
        assert!(result.is_empty(), "no .rs files → no discovered modules");
    }

    #[tokio::test]
    async fn factory_invoked_for_discovered_file() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("hello.rs"), "// stub").unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let factory: ModuleFactory = Arc::new(move |_file, entry_point| {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            assert_eq!(entry_point, "Hello");
            Ok(Some(Arc::new(TestModule {
                desc: "test".to_string(),
            }) as Arc<dyn Module>))
        });

        let discoverer = DefaultDiscoverer::new().with_factory(factory);
        let result = discoverer
            .discover(&[tmp.path().to_string_lossy().into_owned()])
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "hello");
    }

    #[tokio::test]
    async fn circular_dependency_yields_dependency_error() {
        // Two files with deps on each other (a → b, b → a). We need to
        // construct a metadata file with a "dependencies" entry to trigger
        // the cycle detection in resolve_dependencies.
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "// a").unwrap();
        std::fs::write(
            tmp.path().join("a_meta.yaml"),
            "dependencies:\n  - module_id: b\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("b.rs"), "// b").unwrap();
        std::fs::write(
            tmp.path().join("b_meta.yaml"),
            "dependencies:\n  - module_id: a\n",
        )
        .unwrap();

        let factory: ModuleFactory = Arc::new(|_file, _entry| {
            Ok(Some(Arc::new(TestModule {
                desc: "circular".to_string(),
            }) as Arc<dyn Module>))
        });

        let discoverer = DefaultDiscoverer::new().with_factory(factory);
        let err = discoverer
            .discover(&[tmp.path().to_string_lossy().into_owned()])
            .await
            .expect_err("cycle should error");
        // resolve_dependencies surfaces cycles via CircularDependency code
        // when Kahn's queue empties before all nodes are processed.
        assert!(
            matches!(
                err.code,
                ErrorCode::CircularDependency | ErrorCode::DependencyNotFound
            ),
            "expected CircularDependency or DependencyNotFound, got {:?}",
            err.code
        );
    }
}
