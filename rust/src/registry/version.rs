// APCore Protocol — Version handling for registered modules (F18)
// Spec reference: Semver utilities and versioned storage for module version negotiation

use parking_lot::RwLock;
use std::collections::HashMap;

use crate::errors::{ErrorCode, ModuleError};

/// Parse a version string into a `(major, minor, patch)` tuple.
///
/// Supports full semver (`1.2.3`), major.minor (`1.2`), and major-only (`1`).
///
/// Aligned with `apcore-python.parse_semver` and
/// `apcore-typescript.parseSemver`.
pub fn parse_semver(version: &str) -> (u64, u64, u64) {
    let trimmed = version.trim();
    let mut parts = trimmed.splitn(3, '.');

    let major = parts.next().and_then(parse_numeric_prefix).unwrap_or(0);
    let minor = parts.next().and_then(parse_numeric_prefix).unwrap_or(0);
    let patch = parts.next().and_then(parse_numeric_prefix).unwrap_or(0);

    (major, minor, patch)
}

/// Parse the leading numeric portion of a string (handles pre-release suffixes like "3-beta").
fn parse_numeric_prefix(s: &str) -> Option<u64> {
    let numeric: String = s.chars().take_while(char::is_ascii_digit).collect();
    if numeric.is_empty() {
        None
    } else {
        numeric.parse().ok()
    }
}

/// Compute the exclusive upper bound for a caret (`^`) constraint.
///
/// npm/Cargo semantics:
/// - `^1.2.3` -> `<2.0.0`
/// - `^0.2.3` -> `<0.3.0`
/// - `^0.0.3` -> `<0.0.4`
fn caret_upper_bound(target: (u64, u64, u64)) -> (u64, u64, u64) {
    let (major, minor, patch) = target;
    if major > 0 {
        (major + 1, 0, 0)
    } else if minor > 0 {
        (0, minor + 1, 0)
    } else {
        (0, 0, patch + 1)
    }
}

/// Compute the exclusive upper bound for a tilde (`~`) constraint.
///
/// npm semantics:
/// - `~1.2.3` -> `<1.3.0` (3 parts: patch bumps)
/// - `~1.2`   -> `<1.3.0` (2 parts: patch bumps)
/// - `~1`     -> `<2.0.0` (1 part: minor + patch bumps)
fn tilde_upper_bound(target: (u64, u64, u64), part_count: usize) -> (u64, u64, u64) {
    let (major, minor, _) = target;
    if part_count >= 2 {
        (major, minor + 1, 0)
    } else {
        (major + 1, 0, 0)
    }
}

/// Build the typed error for a malformed constraint.
///
/// Mirrors apcore-python `VersionConstraintError` (`VERSION_CONSTRAINT_INVALID`)
/// field for field, including the `constraint` / `reason` details and the
/// `ai_guidance` wording.
fn version_constraint_error(constraint: &str, reason: &str) -> ModuleError {
    let mut details: HashMap<String, serde_json::Value> = HashMap::new();
    details.insert(
        "constraint".to_string(),
        serde_json::Value::String(constraint.to_string()),
    );
    details.insert(
        "reason".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    ModuleError::new(
        ErrorCode::VersionConstraintInvalid,
        format!("Invalid version constraint '{constraint}': {reason}"),
    )
    .with_details(details)
    .with_ai_guidance(format!(
        "Constraint '{constraint}' is not a valid semver expression. Use forms like '1.2.3', \
         '>=1.2.0,<2.0.0', '^1.2.3', or '~1.2'. {reason}"
    ))
}

/// Split a constraint into `(operator, operand)`, validating the operand.
///
/// The operand MUST start with a digit. Without that check `"latest"` matched no
/// operator prefix, fell through to `("=", "latest")`, and `parse_semver` turned
/// it into `(0, 0, 0)` — whereupon the single-part exact branch compared major
/// against 0 and reported a match for any `0.x.y` module, so the constraint was
/// never enforced. The mirror image was equally wrong: `"v1.0.0"` failed to
/// match an actual `1.0.0`. Same operand check as apcore-python's
/// `_CONSTRAINT_RE` (`^(>=|<=|>|<|\^|~|=)?(\d[\w.\-+]*)$`).
fn split_constraint(constraint: &str) -> Result<(&'static str, &str), ModuleError> {
    let trimmed = constraint.trim();
    if trimmed.is_empty() {
        return Err(version_constraint_error("", "empty constraint"));
    }

    let (op, operand) = if let Some(rest) = trimmed.strip_prefix(">=") {
        (">=", rest)
    } else if let Some(rest) = trimmed.strip_prefix("<=") {
        ("<=", rest)
    } else if let Some(rest) = trimmed.strip_prefix('>') {
        (">", rest)
    } else if let Some(rest) = trimmed.strip_prefix('<') {
        ("<", rest)
    } else if let Some(rest) = trimmed.strip_prefix('^') {
        ("^", rest)
    } else if let Some(rest) = trimmed.strip_prefix('~') {
        ("~", rest)
    } else if let Some(rest) = trimmed.strip_prefix('=') {
        ("=", rest)
    } else {
        ("=", trimmed)
    };

    let valid_operand = operand.chars().next().is_some_and(|c| c.is_ascii_digit())
        && operand
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'));
    if !valid_operand {
        return Err(version_constraint_error(
            trimmed,
            "operand must start with a digit (e.g., '1.2.3', not 'v1.2.3' or 'latest')",
        ));
    }
    Ok((op, operand))
}

/// Check if a version string satisfies a single constraint.
///
/// Supported operators: `=`, `>=`, `>`, `<=`, `<`, `^`, `~`. When no operator
/// is supplied, the constraint is treated as exact with partial-version
/// shortcuts (`"1"` matches any `1.x.x`, `"1.2"` matches any `1.2.x`).
///
/// # Errors
///
/// Returns [`ErrorCode::VersionConstraintInvalid`] when the constraint is
/// malformed — empty, an operator with no operand, or an operand that does not
/// start with a digit.
fn check_single_constraint(
    version_tuple: (u64, u64, u64),
    constraint: &str,
) -> Result<bool, ModuleError> {
    let (op, target_str) = split_constraint(constraint)?;
    let target = parse_semver(target_str);
    let parts: Vec<&str> = target_str.trim().split('.').collect();

    if op == "^" {
        let upper = caret_upper_bound(target);
        return Ok(version_tuple >= target && version_tuple < upper);
    }
    if op == "~" {
        let upper = tilde_upper_bound(target, parts.len());
        return Ok(version_tuple >= target && version_tuple < upper);
    }

    // Partial match for exact comparisons
    if op == "=" {
        if parts.len() == 1 {
            return Ok(version_tuple.0 == target.0);
        }
        if parts.len() == 2 {
            return Ok(version_tuple.0 == target.0 && version_tuple.1 == target.1);
        }
        return Ok(version_tuple == target);
    }

    Ok(match op {
        ">=" => version_tuple >= target,
        ">" => version_tuple > target,
        "<=" => version_tuple <= target,
        "<" => version_tuple < target,
        _ => false,
    })
}

/// Check if a version string satisfies a version hint.
///
/// The hint can be:
/// - An exact version: `"1.0.0"`
/// - A partial version: `"1"` (matches major 1.x.x)
/// - A constraint: `">=1.0.0"`, `"<2.0.0"`
/// - A comma-separated set of constraints: `">=1.0.0,<2.0.0"`
///
/// Aligned with `apcore-python.matches_version_hint` and
/// `apcore-typescript.matchesVersionHint`.
#[must_use]
pub fn matches_version_hint(version: &str, hint: &str) -> bool {
    match try_matches_version_hint(version, hint) {
        Ok(matched) => matched,
        Err(err) => {
            // A malformed constraint must never be reported as "satisfied":
            // that is how `"latest"` silently disabled enforcement for every
            // `0.x.y` module. Fail CLOSED and say so. Callers that want the
            // typed error use `try_matches_version_hint`.
            tracing::warn!(
                version = %version,
                hint = %hint,
                "{}; treating the hint as unsatisfied",
                err.message
            );
            false
        }
    }
}

/// Check if a version string satisfies a version hint, reporting a malformed
/// hint as a typed error instead of a `false`.
///
/// Same hint grammar as [`matches_version_hint`]; the two differ only in how a
/// malformed constraint is surfaced.
///
/// # Errors
///
/// Returns [`ErrorCode::VersionConstraintInvalid`] when any comma-separated
/// constraint in `hint` is empty, is a bare operator, or has an operand that
/// does not start with a digit (`"latest"`, `"v1.0.0"`, `""`). Mirrors
/// apcore-python, which raises `VersionConstraintError` for exactly these.
pub fn try_matches_version_hint(version: &str, hint: &str) -> Result<bool, ModuleError> {
    let version_tuple = parse_semver(version);
    for constraint in hint.split(',') {
        if !check_single_constraint(version_tuple, constraint.trim())? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Select the best matching version from a list.
///
/// If `version_hint` is `None`, returns the latest (highest) version.
/// If `version_hint` is given, returns the highest version that matches.
/// Returns `None` if no version matches.
///
/// Aligned with `apcore-python.select_best_version` and
/// `apcore-typescript.selectBestVersion`.
#[must_use]
pub fn select_best_version(versions: &[String], version_hint: Option<&str>) -> Option<String> {
    // A malformed hint selects nothing rather than everything — `matches_version_hint`
    // fails closed and warns. `try_select_best_version` returns the typed error.
    try_select_best_version(versions, version_hint).unwrap_or_else(|err| {
        tracing::warn!("{}; no version selected", err.message);
        None
    })
}

/// Select the best matching version from a list, reporting a malformed hint as
/// a typed error instead of a `None`.
///
/// # Errors
///
/// Returns [`ErrorCode::VersionConstraintInvalid`] when `version_hint` contains
/// a malformed constraint. See [`try_matches_version_hint`].
pub fn try_select_best_version(
    versions: &[String],
    version_hint: Option<&str>,
) -> Result<Option<String>, ModuleError> {
    if versions.is_empty() {
        return Ok(None);
    }

    let mut sorted: Vec<&String> = versions.iter().collect();
    sorted.sort_by_key(|a| parse_semver(a));

    let Some(hint) = version_hint else {
        return Ok(sorted.last().map(|v| (*v).clone()));
    };
    for candidate in sorted.iter().rev() {
        if try_matches_version_hint(candidate, hint)? {
            return Ok(Some((*candidate).clone()));
        }
    }
    Ok(None)
}

/// Thread-safe storage for multiple versions of items keyed by ID.
///
/// Stores items as `HashMap<module_id, HashMap<version, T>>`.
///
/// Uses `parking_lot::RwLock` for consistency with the rest of the registry.
///
/// Aligned with `apcore-python.VersionedStore` and
/// `apcore-typescript.VersionedStore`.
pub struct VersionedStore<T> {
    data: RwLock<HashMap<String, HashMap<String, T>>>,
}

impl<T: Clone> VersionedStore<T> {
    /// Create a new empty versioned store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }

    /// Add an item for a given `module_id` and version.
    pub fn add(&self, module_id: &str, version: &str, item: T) {
        let mut data = self.data.write();
        data.entry(module_id.to_string())
            .or_default()
            .insert(version.to_string(), item);
    }

    /// Get a specific version of an item. Returns `None` if not found.
    pub fn get(&self, module_id: &str, version: &str) -> Option<T> {
        let data = self.data.read();
        data.get(module_id)
            .and_then(|versions| versions.get(version))
            .cloned()
    }

    /// Get the latest (highest semver) version of an item.
    pub fn get_latest(&self, module_id: &str) -> Option<T> {
        let data = self.data.read();
        let versions = data.get(module_id)?;
        let keys: Vec<String> = versions.keys().cloned().collect();
        let best = select_best_version(&keys, None)?;
        versions.get(&best).cloned()
    }

    /// Resolve a module by ID and optional version hint.
    ///
    /// A malformed `version_hint` resolves to `None` (with a warning) rather
    /// than to an arbitrary version; [`Self::try_resolve`] surfaces the typed
    /// error instead.
    pub fn resolve(&self, module_id: &str, version_hint: Option<&str>) -> Option<T> {
        let data = self.data.read();
        let versions = data.get(module_id)?;
        let keys: Vec<String> = versions.keys().cloned().collect();
        let best = select_best_version(&keys, version_hint)?;
        versions.get(&best).cloned()
    }

    /// Resolve a module by ID and optional version hint, reporting a malformed
    /// hint as a typed error instead of a `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::VersionConstraintInvalid`] when `version_hint`
    /// contains a malformed constraint. See [`try_matches_version_hint`].
    pub fn try_resolve(
        &self,
        module_id: &str,
        version_hint: Option<&str>,
    ) -> Result<Option<T>, ModuleError> {
        let data = self.data.read();
        let Some(versions) = data.get(module_id) else {
            return Ok(None);
        };
        let keys: Vec<String> = versions.keys().cloned().collect();
        let Some(best) = try_select_best_version(&keys, version_hint)? else {
            return Ok(None);
        };
        Ok(versions.get(&best).cloned())
    }

    /// List all registered versions for a `module_id`, sorted by semver.
    pub fn list_versions(&self, module_id: &str) -> Vec<String> {
        let data = self.data.read();
        match data.get(module_id) {
            Some(versions) => {
                let mut keys: Vec<String> = versions.keys().cloned().collect();
                keys.sort_by_key(|a| parse_semver(a));
                keys
            }
            None => Vec::new(),
        }
    }

    /// List all unique module IDs.
    pub fn list_ids(&self) -> Vec<String> {
        let data = self.data.read();
        data.keys().cloned().collect()
    }

    /// Remove a specific version. Returns the removed item or `None`.
    pub fn remove(&self, module_id: &str, version: &str) -> Option<T> {
        let mut data = self.data.write();
        let versions = data.get_mut(module_id)?;
        let item = versions.remove(version);
        if versions.is_empty() {
            data.remove(module_id);
        }
        item
    }

    /// Remove all versions for a `module_id`. Returns removed versions.
    pub fn remove_all(&self, module_id: &str) -> HashMap<String, T> {
        let mut data = self.data.write();
        data.remove(module_id).unwrap_or_default()
    }

    /// Check if any version of a `module_id` is registered.
    pub fn has(&self, module_id: &str) -> bool {
        let data = self.data.read();
        data.get(module_id).is_some_and(|v| !v.is_empty())
    }

    /// Check if a specific version is registered.
    pub fn has_version(&self, module_id: &str, version: &str) -> bool {
        let data = self.data.read();
        data.get(module_id).is_some_and(|v| v.contains_key(version))
    }
}

impl<T: Clone> Default for VersionedStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    #[test]
    fn test_parse_semver_full() {
        assert_eq!(parse_semver("1.2.3"), (1, 2, 3));
    }

    #[test]
    fn test_parse_semver_partial() {
        assert_eq!(parse_semver("1.2"), (1, 2, 0));
        assert_eq!(parse_semver("1"), (1, 0, 0));
    }

    #[test]
    fn test_parse_semver_invalid() {
        assert_eq!(parse_semver("abc"), (0, 0, 0));
    }

    #[test]
    fn test_matches_version_hint_exact() {
        assert!(matches_version_hint("1.2.3", "1.2.3"));
        assert!(!matches_version_hint("1.2.4", "1.2.3"));
    }

    #[test]
    fn test_matches_version_hint_partial() {
        assert!(matches_version_hint("1.5.0", "1"));
        assert!(!matches_version_hint("2.0.0", "1"));
    }

    // ── Malformed constraints (operand validation) ───────────────────
    //
    // Before the operand check, `"latest"` matched no operator prefix, fell
    // through to `("=", "latest")`, and `parse_semver` degraded it to (0,0,0):
    // the single-part exact branch then compared major against 0 and reported
    // a MATCH for any 0.x.y module, so the constraint enforced nothing. The
    // reverse was just as wrong — `"v1.0.0"` never matched an actual 1.0.0.
    // apcore-python raises `VersionConstraintError` for all three of these.

    #[test]
    fn test_malformed_constraint_latest_is_rejected() {
        let err = try_matches_version_hint("0.5.0", "latest")
            .expect_err("'latest' is not a semver constraint");
        assert_eq!(err.code, ErrorCode::VersionConstraintInvalid);
        assert_eq!(
            err.details.get("constraint").and_then(|v| v.as_str()),
            Some("latest")
        );
        // The soft-failing wrapper must fail CLOSED, never report a match.
        assert!(!matches_version_hint("0.5.0", "latest"));
        assert!(!matches_version_hint("1.2.3", "latest"));
    }

    #[test]
    fn test_malformed_constraint_v_prefix_is_rejected() {
        let err =
            try_matches_version_hint("1.0.0", "v1.0.0").expect_err("a 'v' prefix is not supported");
        assert_eq!(err.code, ErrorCode::VersionConstraintInvalid);
        assert!(!matches_version_hint("1.0.0", "v1.0.0"));
    }

    #[test]
    fn test_malformed_constraint_empty_is_rejected() {
        let err =
            try_matches_version_hint("1.0.0", "").expect_err("an empty constraint is invalid");
        assert_eq!(err.code, ErrorCode::VersionConstraintInvalid);
        assert!(!matches_version_hint("1.0.0", ""));
        // A bare operator with no operand is the same class of typo.
        assert!(try_matches_version_hint("1.0.0", ">=").is_err());
    }

    #[test]
    fn test_malformed_constraint_selects_no_version() {
        let versions = vec!["0.5.0".to_string(), "1.0.0".to_string()];
        assert_eq!(select_best_version(&versions, Some("latest")), None);
        assert!(try_select_best_version(&versions, Some("latest")).is_err());

        let store: VersionedStore<u8> = VersionedStore::new();
        store.add("mod.a", "0.5.0", 1);
        assert_eq!(store.resolve("mod.a", Some("latest")), None);
        assert!(store.try_resolve("mod.a", Some("latest")).is_err());
    }

    #[test]
    fn test_prerelease_operand_still_accepted() {
        // The operand may carry a pre-release / build suffix; only the LEADING
        // character has to be a digit.
        assert!(matches_version_hint("1.2.3", ">=1.0.0-beta.1"));
        assert!(try_matches_version_hint("1.2.3", "1.2.3+build.7").is_ok());
    }

    #[test]
    fn test_matches_version_hint_range() {
        assert!(matches_version_hint("1.5.0", ">=1.0.0,<2.0.0"));
        assert!(!matches_version_hint("2.0.0", ">=1.0.0,<2.0.0"));
        assert!(!matches_version_hint("0.9.0", ">=1.0.0,<2.0.0"));
    }

    #[test]
    fn test_select_best_version_latest() {
        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "1.5.0".to_string(),
        ];
        assert_eq!(
            select_best_version(&versions, None),
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn test_select_best_version_with_hint() {
        let versions = vec![
            "1.0.0".to_string(),
            "1.5.0".to_string(),
            "2.0.0".to_string(),
        ];
        assert_eq!(
            select_best_version(&versions, Some(">=1.0.0,<2.0.0")),
            Some("1.5.0".to_string())
        );
    }

    #[test]
    fn test_select_best_version_no_match() {
        let versions = vec!["1.0.0".to_string()];
        assert_eq!(select_best_version(&versions, Some(">=2.0.0")), None);
    }

    #[test]
    fn test_select_best_version_empty() {
        let versions: Vec<String> = vec![];
        assert_eq!(select_best_version(&versions, None), None);
    }

    #[test]
    fn test_versioned_store_basic() {
        let store: VersionedStore<String> = VersionedStore::new();
        store.add("foo", "1.0.0", "v1".to_string());
        store.add("foo", "2.0.0", "v2".to_string());

        assert_eq!(store.get("foo", "1.0.0"), Some("v1".to_string()));
        assert_eq!(store.get_latest("foo"), Some("v2".to_string()));
        assert!(store.has("foo"));
        assert!(store.has_version("foo", "1.0.0"));
        assert!(!store.has("bar"));
    }

    #[test]
    fn test_versioned_store_resolve() {
        let store: VersionedStore<String> = VersionedStore::new();
        store.add("foo", "1.0.0", "v1".to_string());
        store.add("foo", "1.5.0", "v15".to_string());
        store.add("foo", "2.0.0", "v2".to_string());

        assert_eq!(
            store.resolve("foo", Some(">=1.0.0,<2.0.0")),
            Some("v15".to_string())
        );
        assert_eq!(store.resolve("foo", None), Some("v2".to_string()));
    }

    #[test]
    fn test_versioned_store_remove() {
        let store: VersionedStore<String> = VersionedStore::new();
        store.add("foo", "1.0.0", "v1".to_string());
        store.add("foo", "2.0.0", "v2".to_string());

        assert_eq!(store.remove("foo", "1.0.0"), Some("v1".to_string()));
        assert!(!store.has_version("foo", "1.0.0"));
        assert!(store.has("foo"));

        let removed = store.remove_all("foo");
        assert_eq!(removed.len(), 1);
        assert!(!store.has("foo"));
    }

    #[test]
    fn test_versioned_store_list() {
        let store: VersionedStore<String> = VersionedStore::new();
        store.add("foo", "2.0.0", "v2".to_string());
        store.add("foo", "1.0.0", "v1".to_string());
        store.add("bar", "1.0.0", "bv1".to_string());

        let versions = store.list_versions("foo");
        assert_eq!(versions, vec!["1.0.0", "2.0.0"]);

        let mut ids = store.list_ids();
        ids.sort();
        assert_eq!(ids, vec!["bar", "foo"]);
    }
}
