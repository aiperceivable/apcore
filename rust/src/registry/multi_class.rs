// APCore Protocol — Multi-class module discovery (Issue #32)
// Spec reference: PROTOCOL_SPEC §2.1.1, docs/features/multi-module-discovery.md
//
// Multi-class discovery is an opt-in extension that lets multiple Module
// implementations coexist in a single source file.  Each qualifying class
// receives an ID of the form `base_id.class_segment`, where `class_segment`
// is the snake_case conversion of the class name.  A file with exactly one
// class always receives the bare `base_id` (single-class identity guarantee).
//
// Aligned with `apcore-python.discover_multi_class` and the cross-language
// conformance fixture `multi_module_discovery.json`.
//
// Note on the Rust integration model: Rust has no runtime reflection, so the
// discoverer cannot enumerate `impl Module for X` at scan time the way Python
// `inspect.getmembers` can.  Module authors register a list of `(class_name,
// instance)` pairs explicitly via [`Registry::register_multi_class`]; this
// module exposes the pure ID-derivation logic so the same conformance fixture
// drives all three SDKs.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::errors::ModuleError;
use crate::module::Module;
use crate::registry::registry::{
    ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION, MAX_MODULE_ID_LENGTH,
};

/// Maximum length of a derived module ID (PROTOCOL_SPEC §2.7).
///
/// Bound to [`crate::registry::registry::MAX_MODULE_ID_LENGTH`] so the two
/// public constants cannot drift.  Both names are kept in the public API
/// surface — `MAX_MODULE_ID_LEN` matches the corresponding constant name in
/// `apcore-python.registry.multi_class` for cross-SDK naming parity, while
/// `MAX_MODULE_ID_LENGTH` is the registry-wide name preserved for
/// backward compatibility.
pub const MAX_MODULE_ID_LEN: usize = MAX_MODULE_ID_LENGTH;

/// Configuration controlling discovery-time behavior.
///
/// **This no longer gates multi-class discovery (D-107, spec v1.50.0).** The
/// `multi_class` flag was a FILE-LEVEL toggle whose doc comment cited
/// `extensions.multi_class_discovery` — a config key
/// [decision-log D-06](https://apcore.aiperceivable.com/spec/2026-05-decision-log/)
/// removed, and which no SDK ever implemented. Per-class markers are the only
/// opt-in path: see [`DiscoveredClass::with_multi_class`]. The type and its
/// field are retained so existing call sites keep compiling; nothing reads the
/// flag.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryConfig {
    /// Retained for source compatibility. **Inert** — a file-level toggle
    /// cannot express the case the feature exists for (two participating
    /// classes beside a helper class that must not become a module).
    pub multi_class: bool,
}

impl DiscoveryConfig {
    /// Construct a config with the (inert) multi-class flag set.
    #[must_use]
    pub fn with_multi_class() -> Self {
        Self { multi_class: true }
    }
}

/// A class candidate found in a source file.
///
/// `name` is the original (PascalCase) class/struct name; `implements_module`
/// is `true` when the class implements the [`Module`] trait.  Non-qualifying
/// classes are filtered out before ID derivation.
///
/// `multi_class` is the **per-class marker** — the Rust equivalent of Python's
/// `@multi_class` decorator and TypeScript's `@multiClass()`
/// (`multi-module-discovery.md`, D-107). A file opts in when at least one of
/// its qualifying classes carries it, mirroring apcore-typescript's
/// `classes.some(c => c.multiClass)`.
#[derive(Debug, Clone, Default)]
pub struct DiscoveredClass {
    pub name: String,
    pub implements_module: bool,
    pub multi_class: bool,
}

impl DiscoveredClass {
    /// A class candidate with no multi-class marker.
    #[must_use]
    pub fn new(name: impl Into<String>, implements_module: bool) -> Self {
        Self {
            name: name.into(),
            implements_module,
            multi_class: false,
        }
    }

    /// Set the per-class multi-class marker (D-107).
    #[must_use]
    pub fn with_multi_class(mut self, multi_class: bool) -> Self {
        self.multi_class = multi_class;
        self
    }
}

/// Convert a class/struct name to a snake_case ID segment per
/// PROTOCOL_SPEC §2.1.1.
///
/// Algorithm (mirrors `apcore-python.class_name_to_segment`):
/// 1. Insert `_` between an ALLCAPS run and a following capitalized word
///    (`HTTPSender` → `HTTP_Sender`).
/// 2. Insert `_` between a lowercase/digit and an uppercase character
///    (`MathOps` → `Math_Ops`).
/// 3. Replace every non-alphanumeric character with `_`.
/// 4. Lowercase.
/// 5. Collapse consecutive `_` to a single `_`.
/// 6. Strip leading and trailing `_`.
#[must_use]
pub fn class_name_to_segment(class_name: &str) -> String {
    let chars: Vec<char> = class_name.chars().collect();
    let mut intermediate = String::with_capacity(chars.len() * 2);

    for (i, &c) in chars.iter().enumerate() {
        if i > 0 {
            let prev = chars[i - 1];
            // Rule 1: ALLCAPS run followed by capword (HTTPSender → HTTP_Sender)
            if prev.is_ascii_uppercase() && c.is_ascii_uppercase() {
                if let Some(&next) = chars.get(i + 1) {
                    if next.is_ascii_lowercase() {
                        intermediate.push('_');
                    }
                }
            }
            // Rule 2: lowercase/digit followed by uppercase (MathOps → Math_Ops)
            else if (prev.is_ascii_lowercase() || prev.is_ascii_digit()) && c.is_ascii_uppercase()
            {
                intermediate.push('_');
            }
        }
        intermediate.push(c);
    }

    // Replace non-alphanumeric with `_`, lowercase as we go.
    let mut sanitized = String::with_capacity(intermediate.len());
    for c in intermediate.chars() {
        if c.is_ascii_alphanumeric() {
            sanitized.push(c.to_ascii_lowercase());
        } else {
            sanitized.push('_');
        }
    }

    // Collapse consecutive underscores.
    let mut collapsed = String::with_capacity(sanitized.len());
    let mut prev_underscore = false;
    for c in sanitized.chars() {
        if c == '_' {
            if !prev_underscore {
                collapsed.push('_');
            }
            prev_underscore = true;
        } else {
            collapsed.push(c);
            prev_underscore = false;
        }
    }

    collapsed.trim_matches('_').to_string()
}

/// Compute the base module ID from a file path using Algorithm A01.
///
/// Walks the path components looking for one equal to `extensions_root`; the
/// segments after it form the canonical ID (joined by `.`, with the file
/// extension stripped from the last segment).  When `extensions_root` is not
/// found, the ID falls back to the bare file stem — directory context is
/// dropped, mirroring the Python reference (`StopIteration` → `Path(stem)`).
///
/// Aligned with `apcore-python._compute_base_id`.
#[must_use]
pub fn compute_base_id(file_path: &Path, extensions_root: &str) -> String {
    let parts: Vec<String> = file_path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    let Some(idx) = parts.iter().position(|p| p == extensions_root) else {
        return file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
    };

    let rel: &[String] = &parts[idx + 1..];
    if rel.is_empty() {
        return String::new();
    }

    let mut joined: Vec<String> = rel[..rel.len() - 1].to_vec();
    let last = &rel[rel.len() - 1];
    let stem = Path::new(last)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(last)
        .to_string();
    joined.push(stem);
    joined.join(".")
}

fn segment_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^[a-z][a-z0-9_]*$").unwrap())
}

fn canonical_id_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$").unwrap())
}

/// Derive module IDs for the qualifying classes of a single source file.
///
/// Returns the list of derived IDs in the same order as the qualifying input
/// classes.  Non-qualifying classes (`implements_module == false`) are
/// filtered out.
///
/// Behavior:
///
/// - **Empty / no qualifying classes** → returns `Ok(vec![])`.
/// - **Exactly one qualifying class** → returns `Ok(vec![base_id])` regardless
///   of `config.multi_class` (single-class identity guarantee).
/// - **Multiple qualifying classes, none carrying the per-class marker** →
///   returns `Ok(vec![base_id])` (only the first class is loaded; the file is
///   treated as single-class per backward-compat policy).
/// - **Multiple qualifying classes, at least one carrying the marker** →
///   derives one ID per marked class as `base_id.class_segment`.  Conflicts (two
///   classes mapping to the same segment) raise
///   [`ErrorCode::ModuleIdConflict`].  Invalid segments raise
///   [`ErrorCode::InvalidSegment`]; over-length IDs raise
///   [`ErrorCode::IdTooLong`].
///
/// `config` is accepted for source compatibility and is **not** read: D-107
/// makes the per-class marker the only opt-in path.
///
/// [`ErrorCode::ModuleIdConflict`]: crate::errors::ErrorCode::ModuleIdConflict
/// [`ErrorCode::InvalidSegment`]: crate::errors::ErrorCode::InvalidSegment
/// [`ErrorCode::IdTooLong`]: crate::errors::ErrorCode::IdTooLong
pub fn derive_module_ids(
    file_path: &Path,
    extensions_root: &str,
    classes: &[DiscoveredClass],
    _config: &DiscoveryConfig,
) -> Result<Vec<String>, ModuleError> {
    let qualifying: Vec<&DiscoveredClass> =
        classes.iter().filter(|c| c.implements_module).collect();

    if qualifying.is_empty() {
        return Ok(Vec::new());
    }

    let base_id = compute_base_id(file_path, extensions_root);

    // Single-class identity guarantee: one class → bare base_id, regardless of
    // multi_class mode.
    if qualifying.len() == 1 {
        return Ok(vec![base_id]);
    }

    // D-107: the opt-in is resolved from the PER-CLASS markers, never from a
    // file-level toggle. `DiscoveryConfig::multi_class` cited a config key
    // decision-log D-06 removed and could not express the case the feature
    // exists for — two participating classes beside a helper class that must
    // not become a module. Mirrors apcore-typescript's
    // `Registry.discoverMultiClass` (`classes.some(c => c.multiClass)`) and
    // apcore-python, which honours a per-class marker and nothing else.
    //
    // No marker on any qualifying class: the file is treated as single-class
    // and only base_id is returned (first qualifying class wins).
    if !qualifying.iter().any(|c| c.multi_class) {
        return Ok(vec![base_id]);
    }

    let file_path_str = file_path.to_string_lossy().into_owned();
    let mut seen: Vec<(String, String)> = Vec::with_capacity(qualifying.len());
    let mut results: Vec<String> = Vec::with_capacity(qualifying.len());

    for class in qualifying.into_iter().filter(|class| class.multi_class) {
        let segment = class_name_to_segment(&class.name);

        if !segment_pattern().is_match(&segment) {
            return Err(ModuleError::invalid_segment(
                &file_path_str,
                &class.name,
                &segment,
            ));
        }

        if let Some((prior_class, _)) = seen.iter().find(|(_, s)| s == &segment) {
            tracing::error!(
                file_path = %file_path_str,
                class_a = %prior_class,
                class_b = %class.name,
                segment = %segment,
                "Module ID conflict: classes produce the same snake_case segment"
            );
            return Err(ModuleError::module_id_conflict(
                &file_path_str,
                &[prior_class.clone(), class.name.clone()],
                &segment,
            ));
        }
        seen.push((class.name.clone(), segment.clone()));

        let module_id = format!("{base_id}.{segment}");

        if !canonical_id_pattern().is_match(&module_id) {
            return Err(ModuleError::invalid_segment(
                &file_path_str,
                &class.name,
                &segment,
            ));
        }

        if module_id.len() > MAX_MODULE_ID_LEN {
            return Err(ModuleError::id_too_long(&file_path_str, &module_id));
        }

        results.push(module_id);
    }

    Ok(results)
}

/// A class candidate paired with a live [`Module`] instance, used when
/// registering a multi-class file with [`Registry::register_multi_class`].
///
/// `multi_class` is the same per-class marker [`DiscoveredClass`] carries
/// (D-107): in a file with multiple entries, only marked entries register when
/// any entry carries the marker; otherwise only the first entry registers.
/// A single entry keeps the bare file ID. [`Self::new`] leaves the marker unset,
/// so participating entries in a multi-class file must opt in — with
/// [`Self::with_multi_class`] — exactly as a Python `@multi_class` decorator or
/// a TypeScript `@multiClass()` does.
pub struct MultiClassEntry {
    pub class_name: String,
    pub module: Box<dyn Module>,
    pub multi_class: bool,
}

impl MultiClassEntry {
    pub fn new(class_name: impl Into<String>, module: Box<dyn Module>) -> Self {
        Self {
            class_name: class_name.into(),
            module,
            multi_class: false,
        }
    }

    /// Set the per-class multi-class marker (D-107).
    #[must_use]
    pub fn with_multi_class(mut self, multi_class: bool) -> Self {
        self.multi_class = multi_class;
        self
    }
}

impl Registry {
    /// Register the participating classes discovered in a multi-class source file.
    ///
    /// Computes the IDs via [`derive_module_ids`] and registers each module
    /// under its derived ID.  The whole batch is registered atomically — if
    /// any ID derivation fails (conflict / invalid segment / too long), no
    /// modules from the file are registered, mirroring the spec's
    /// "no partial registration" requirement.
    ///
    /// On any per-module registration failure (e.g. duplicate ID across
    /// files), already-registered modules from this batch are unregistered to
    /// keep the all-or-nothing guarantee.
    ///
    /// Aligned with `apcore-python.Registry.discover_multi_class` (which
    /// returns the `(module_id, class)` pairs and registers separately).
    pub fn register_multi_class(
        &self,
        file_path: &Path,
        extensions_root: &str,
        entries: Vec<MultiClassEntry>,
        config: &DiscoveryConfig,
    ) -> Result<Vec<String>, ModuleError> {
        let classes: Vec<DiscoveredClass> = entries
            .iter()
            .map(|e| DiscoveredClass {
                name: e.class_name.clone(),
                implements_module: true,
                multi_class: e.multi_class,
            })
            .collect();

        let module_ids = derive_module_ids(file_path, extensions_root, &classes, config)?;

        // Match derivation's participant selection so an unmarked helper
        // cannot be paired with a marked class's ID, regardless of ordering.
        let multi_class = entries.len() > 1 && entries.iter().any(|entry| entry.multi_class);
        let participants = entries
            .into_iter()
            .filter(|entry| !multi_class || entry.multi_class);
        let mut registered: Vec<String> = Vec::with_capacity(module_ids.len());

        for (module_id, entry) in module_ids.iter().zip(participants) {
            let descriptor = ModuleDescriptor {
                module_id: module_id.clone(),
                name: None,
                description: entry.module.description().to_string(),
                documentation: None,
                input_schema: entry.module.input_schema(),
                output_schema: entry.module.output_schema(),
                version: DEFAULT_MODULE_VERSION.to_string(),
                tags: vec![],
                annotations: Some(crate::module::ModuleAnnotations::default()),
                examples: vec![],
                metadata: std::collections::HashMap::new(),
                display: None,
                sunset_date: None,
                dependencies: vec![],
                enabled: true,
            };

            if let Err(e) = self.register(module_id, entry.module, descriptor) {
                // Roll back already-registered modules from this batch so the
                // file is registered atomically (no partial registration).
                for prior in &registered {
                    let _ = self.unregister(prior);
                }
                return Err(e);
            }
            registered.push(module_id.clone());
        }

        Ok(module_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_max_module_id_len_matches_registry_max_module_id_length() {
        // Drift guard: MAX_MODULE_ID_LEN must remain bound to MAX_MODULE_ID_LENGTH
        // so cross-SDK naming aliases never diverge in value.
        assert_eq!(MAX_MODULE_ID_LEN, MAX_MODULE_ID_LENGTH);
    }

    #[test]
    fn test_class_name_to_segment_addition() {
        assert_eq!(class_name_to_segment("Addition"), "addition");
    }

    #[test]
    fn test_class_name_to_segment_math_ops() {
        assert_eq!(class_name_to_segment("MathOps"), "math_ops");
    }

    #[test]
    fn test_class_name_to_segment_https_sender() {
        assert_eq!(class_name_to_segment("HTTPSender"), "http_sender");
    }

    #[test]
    fn test_class_name_to_segment_my_module_v2() {
        assert_eq!(class_name_to_segment("MyModule_V2"), "my_module_v2");
    }

    #[test]
    fn test_class_name_to_segment_collapses_underscores() {
        assert_eq!(class_name_to_segment("My__Module"), "my_module");
    }

    #[test]
    fn test_class_name_to_segment_my_module_and_my_underscore_module_collide() {
        assert_eq!(class_name_to_segment("MyModule"), "my_module");
        assert_eq!(class_name_to_segment("My_Module"), "my_module");
    }

    #[test]
    fn test_compute_base_id_with_extensions_root() {
        let p = PathBuf::from("extensions/math/math_ops.py");
        assert_eq!(compute_base_id(&p, "extensions"), "math.math_ops");
    }

    #[test]
    fn test_compute_base_id_deep_path() {
        let p = PathBuf::from("extensions/executor/math/arithmetic.py");
        assert_eq!(
            compute_base_id(&p, "extensions"),
            "executor.math.arithmetic"
        );
    }

    #[test]
    fn test_compute_base_id_root_not_found_falls_back_to_stem() {
        let p = PathBuf::from("/some/random/path/foo.py");
        assert_eq!(compute_base_id(&p, "extensions"), "foo");
    }

    #[test]
    fn test_derive_single_class_returns_base_id_unchanged() {
        let p = PathBuf::from("extensions/math/math_ops.py");
        let classes = vec![DiscoveredClass::new("MathOps", true)];
        let config = DiscoveryConfig::with_multi_class();
        let ids = derive_module_ids(&p, "extensions", &classes, &config).unwrap();
        assert_eq!(ids, vec!["math.math_ops"]);
    }

    #[test]
    fn test_derive_two_classes_distinct_ids() {
        let p = PathBuf::from("extensions/math/math_ops.py");
        // D-107: the per-class marker is what opts the file in.
        let classes = vec![
            DiscoveredClass::new("Addition", true).with_multi_class(true),
            DiscoveredClass::new("Subtraction", true).with_multi_class(true),
        ];
        let config = DiscoveryConfig::default();
        let ids = derive_module_ids(&p, "extensions", &classes, &config).unwrap();
        assert_eq!(
            ids,
            vec!["math.math_ops.addition", "math.math_ops.subtraction"]
        );
    }

    #[test]
    fn test_derive_conflict_raises_module_id_conflict() {
        let p = PathBuf::from("extensions/math/math_ops.py");
        let classes = vec![
            DiscoveredClass::new("MyModule", true).with_multi_class(true),
            DiscoveredClass::new("My_Module", true).with_multi_class(true),
        ];
        let config = DiscoveryConfig::default();
        let err = derive_module_ids(&p, "extensions", &classes, &config).unwrap_err();
        assert_eq!(err.code, crate::errors::ErrorCode::ModuleIdConflict);
        assert_eq!(
            err.details
                .get("conflicting_segment")
                .and_then(|v| v.as_str()),
            Some("my_module")
        );
    }

    #[test]
    fn test_derive_without_a_per_class_marker_returns_only_base_id() {
        // D-107: no marker on any qualifying class -> the file is single-class,
        // and the retired file-level toggle cannot opt it in.
        let p = PathBuf::from("extensions/math/math_ops.py");
        let classes = vec![
            DiscoveredClass::new("Addition", true),
            DiscoveredClass::new("Subtraction", true),
        ];
        let config = DiscoveryConfig::with_multi_class();
        let ids = derive_module_ids(&p, "extensions", &classes, &config).unwrap();
        assert_eq!(ids, vec!["math.math_ops"]);
    }

    #[test]
    fn test_derive_full_id_grammar_valid() {
        let p = PathBuf::from("extensions/executor/math/arithmetic.py");
        let classes = vec![DiscoveredClass::new("Addition", true)];
        let config = DiscoveryConfig::with_multi_class();
        let ids = derive_module_ids(&p, "extensions", &classes, &config).unwrap();
        assert_eq!(ids, vec!["executor.math.arithmetic"]);
        assert!(canonical_id_pattern().is_match(&ids[0]));
    }

    #[test]
    fn test_derive_filters_non_qualifying_classes() {
        let p = PathBuf::from("extensions/math/math_ops.py");
        let classes = vec![
            DiscoveredClass::new("Addition", true),
            DiscoveredClass::new("InternalHelper", false),
        ];
        let config = DiscoveryConfig::with_multi_class();
        let ids = derive_module_ids(&p, "extensions", &classes, &config).unwrap();
        // After filtering, only one qualifying class -> single-class identity guarantee.
        assert_eq!(ids, vec!["math.math_ops"]);
    }

    #[test]
    fn test_derive_no_qualifying_classes_returns_empty() {
        let p = PathBuf::from("extensions/math/math_ops.py");
        let classes = vec![DiscoveredClass::new("InternalHelper", false)];
        let config = DiscoveryConfig::with_multi_class();
        let ids = derive_module_ids(&p, "extensions", &classes, &config).unwrap();
        assert_eq!(ids, Vec::<String>::new());
    }

    #[test]
    fn test_derive_id_too_long_raises_id_too_long() {
        // Construct a path whose base_id is near the 192 char ceiling so that
        // appending `.addition` (9 chars) trips the limit.
        // 184 (base) + 1 (".") + 8 ("addition") = 193 > 192.
        let long_segment = "a".repeat(184);
        let path_str = format!("extensions/{long_segment}.py");
        let p = PathBuf::from(&path_str);
        let classes = vec![
            DiscoveredClass::new("Addition", true).with_multi_class(true),
            DiscoveredClass::new("Subtraction", true).with_multi_class(true),
        ];
        let config = DiscoveryConfig::default();
        let err = derive_module_ids(&p, "extensions", &classes, &config).unwrap_err();
        assert_eq!(err.code, crate::errors::ErrorCode::IdTooLong);
    }
}
