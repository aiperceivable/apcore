// APCore Protocol — Dependency resolution via Kahn's topological sort
// Spec reference: Module dependency ordering

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::json;

use crate::errors::{ErrorCode, ModuleError};
use crate::registry::types::DepInfo;
use crate::registry::version::try_matches_version_hint;

/// Resolve module load order using Kahn's topological sort.
///
/// Modules with dependencies are loaded after their dependencies.
/// Optional dependencies that are missing are silently skipped.
///
/// When `module_versions` is provided, declared dependency version constraints
/// (per PROTOCOL_SPEC §5.3) are enforced against the target module's registered
/// version. Dependencies whose target has no entry in the map are accepted
/// without version check.
///
/// Aligned with `apcore-python.resolve_dependencies` and
/// `apcore-typescript.resolveDependencies`.
#[allow(clippy::implicit_hasher)] // public API: callers always use the default hasher
pub fn resolve_dependencies(
    modules: &[(String, Vec<DepInfo>)],
    known_ids: Option<&HashSet<String>>,
    module_versions: Option<&HashMap<String, String>>,
) -> Result<Vec<String>, ModuleError> {
    if modules.is_empty() {
        return Ok(Vec::new());
    }

    let derived_ids: HashSet<String> = modules.iter().map(|(id, _)| id.clone()).collect();
    let effective_ids = known_ids.unwrap_or(&derived_ids);

    // Build graph and in-degree
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    for (mod_id, _) in modules {
        in_degree.entry(mod_id.clone()).or_insert(0);
    }

    for (module_id, deps) in modules {
        for dep in deps {
            if !effective_ids.contains(&dep.module_id) {
                if dep.optional {
                    tracing::warn!(
                        "Optional dependency '{}' for module '{}' not found, skipping",
                        dep.module_id,
                        module_id
                    );
                    continue;
                }
                return Err(ModuleError::dependency_not_found(module_id, &dep.module_id));
            }
            match check_version_constraint(module_id, dep, module_versions) {
                VersionCheck::Ok => {}
                VersionCheck::SkipOptional => continue,
                VersionCheck::Err(err) => return Err(err),
            }
            graph
                .entry(dep.module_id.clone())
                .or_default()
                .push(module_id.clone());
            *in_degree.entry(module_id.clone()).or_insert(0) += 1;
        }
    }

    // Initialize queue with zero-in-degree nodes (sorted for determinism)
    let mut zero_degree: Vec<String> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(id, _)| id.clone())
        .collect();
    zero_degree.sort();
    let mut queue: VecDeque<String> = zero_degree.into_iter().collect();

    let mut load_order: Vec<String> = Vec::new();
    while let Some(mod_id) = queue.pop_front() {
        load_order.push(mod_id.clone());
        if let Some(dependents) = graph.get(&mod_id) {
            let mut sorted_deps = dependents.clone();
            sorted_deps.sort();
            for dependent in sorted_deps {
                if let Some(deg) = in_degree.get_mut(&dependent) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dependent);
                    }
                }
            }
        }
    }

    if load_order.len() < modules.len() {
        return Err(classify_stall(modules, &load_order));
    }

    Ok(load_order)
}

/// Report a stalled Kahn's sort (D-79).
///
/// When the sort terminates with nodes remaining there are two causes, and
/// they need opposite fixes — break an edge, versus add a missing module to
/// the batch — so they are reported differently. Only a real back edge is a
/// `CIRCULAR_DEPENDENCY`; a stall with no back edge is a `MODULE_LOAD_ERROR`
/// naming the blocked modules, and MUST NOT carry a fabricated `cycle_path`.
/// Reporting a loop that does not exist sends the author looking for something
/// that is not there.
fn classify_stall(modules: &[(String, Vec<DepInfo>)], load_order: &[String]) -> ModuleError {
    let ordered_set: HashSet<&String> = load_order.iter().collect();
    let remaining: HashSet<String> = modules
        .iter()
        .filter(|(id, _)| !ordered_set.contains(id))
        .map(|(id, _)| id.clone())
        .collect();

    if let Some(cycle_path) = find_back_edge_cycle(modules, &remaining) {
        let mut details: HashMap<String, serde_json::Value> = HashMap::new();
        details.insert("cycle_path".to_string(), json!(cycle_path));
        return ModuleError::new(
            ErrorCode::CircularDependency,
            format!("Circular dependency detected: {}", cycle_path.join(" -> ")),
        )
        .with_details(details);
    }

    let mut blocked: Vec<String> = remaining.into_iter().collect();
    blocked.sort();
    let mut details: HashMap<String, serde_json::Value> = HashMap::new();
    details.insert("module_id".to_string(), json!(blocked.join(",")));
    details.insert("blocked_modules".to_string(), json!(blocked));
    ModuleError::new(
        ErrorCode::ModuleLoadError,
        format!(
            "{} module(s) could not be loaded — blocked by unresolved required \
             dependencies: {:?}",
            blocked.len(),
            blocked
        ),
    )
    .with_details(details)
}

enum VersionCheck {
    Ok,
    SkipOptional,
    Err(ModuleError),
}

fn check_version_constraint(
    module_id: &str,
    dep: &DepInfo,
    module_versions: Option<&HashMap<String, String>>,
) -> VersionCheck {
    let (Some(constraint), Some(versions)) = (dep.version.as_deref(), module_versions) else {
        return VersionCheck::Ok;
    };
    let Some(actual) = versions.get(&dep.module_id) else {
        tracing::warn!(
            module_id = %module_id,
            dep_module_id = %dep.module_id,
            constraint = %constraint,
            "Version constraint for dependency declared, but target version is unknown \
             (missing from module_versions map); constraint skipped"
        );
        return VersionCheck::Ok;
    };
    // The FALLIBLE form (D-85). `matches_version_hint` fails closed to `false`
    // for a malformed constraint, and reporting that `false` from here turns
    // "your constraint is not a version" into "your versions do not line up" —
    // so an operator who typed `latest` or `v1.0.0` is sent to look at versions.
    // apcore-python and apcore-typescript both surface VERSION_CONSTRAINT_INVALID
    // from this path, naming the rule the operand broke.
    //
    // A malformed constraint is NOT downgraded for an optional dependency
    // either: `optional` means "this dependency may be absent", not "this
    // declaration may be nonsense", and skipping the edge would leave the typo
    // in place and silent.
    match try_matches_version_hint(actual, constraint) {
        Ok(true) => return VersionCheck::Ok,
        Ok(false) => {}
        Err(err) => return VersionCheck::Err(err),
    }
    if dep.optional {
        tracing::warn!(
            "Optional dependency '{}' for module '{}' has version '{}' which does \
             not satisfy constraint '{}', skipping",
            dep.module_id,
            module_id,
            actual,
            constraint
        );
        return VersionCheck::SkipOptional;
    }
    VersionCheck::Err(ModuleError::dependency_version_mismatch(
        module_id,
        &dep.module_id,
        constraint,
        actual,
    ))
}

/// Return a back-edge cycle `[n0, n1, ..., nk, n0]` if one exists among the
/// remaining unprocessed modules, else `None`.
///
/// Runs DFS from each remaining node (sorted for determinism) until a back edge
/// is found. Returning `None` rather than the sorted remaining set is what lets
/// the caller tell a true cycle from a stall (D-79); the previous fallback
/// turned a module blocked on a dependency outside the batch into a
/// one-element "cycle" reported as `CIRCULAR_DEPENDENCY`.
///
/// Mirrors `apcore-python._find_back_edge_cycle`.
fn find_back_edge_cycle(
    modules: &[(String, Vec<DepInfo>)],
    remaining: &HashSet<String>,
) -> Option<Vec<String>> {
    let mut dep_map: HashMap<String, Vec<String>> = HashMap::new();
    for (mod_id, deps) in modules {
        if remaining.contains(mod_id) {
            let mut edges: Vec<String> = deps
                .iter()
                .filter(|d| remaining.contains(&d.module_id))
                .map(|d| d.module_id.clone())
                .collect();
            edges.sort();
            edges.dedup();
            dep_map.insert(mod_id.clone(), edges);
        }
    }

    let mut sorted_remaining: Vec<String> = remaining.iter().cloned().collect();
    sorted_remaining.sort();

    for start in &sorted_remaining {
        if let Some(cycle) = dfs_find_cycle(&dep_map, start) {
            return Some(cycle);
        }
    }

    None
}

/// Iterative DFS that returns a back-edge cycle `[n0, ..., n0]` or `None`.
fn dfs_find_cycle(dep_map: &HashMap<String, Vec<String>>, start: &str) -> Option<Vec<String>> {
    let mut path: Vec<String> = Vec::new();
    let mut on_path: HashSet<String> = HashSet::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut stack: Vec<(String, usize)> = vec![(start.to_string(), 0)];

    while let Some((node, idx)) = stack.last().cloned() {
        if idx == 0 {
            if on_path.contains(&node) {
                // INVARIANT: `on_path.insert` is always paired with `path.push` (line 230-231),
                // so `on_path.contains(&node)` implies `path` contains `&node`.
                let start_idx = path.iter().position(|n| n == &node).unwrap();
                let mut cycle: Vec<String> = path[start_idx..].to_vec();
                cycle.push(node);
                return Some(cycle);
            }
            if visited.contains(&node) {
                stack.pop();
                continue;
            }
            visited.insert(node.clone());
            on_path.insert(node.clone());
            path.push(node.clone());
        }

        let neighbors = dep_map.get(&node).cloned().unwrap_or_default();
        if idx < neighbors.len() {
            if let Some(frame) = stack.last_mut() {
                frame.1 = idx + 1;
            }
            stack.push((neighbors[idx].clone(), 0));
        } else {
            on_path.remove(&node);
            path.pop();
            stack.pop();
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_modules() {
        let result = resolve_dependencies(&[], None, None).unwrap();
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn test_no_dependencies() {
        let modules = vec![("a".to_string(), vec![]), ("b".to_string(), vec![])];
        let result = resolve_dependencies(&modules, None, None).unwrap();
        assert_eq!(result, vec!["a", "b"]);
    }

    #[test]
    fn test_linear_dependencies() {
        let modules = vec![
            (
                "b".to_string(),
                vec![DepInfo {
                    module_id: "a".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
            ("a".to_string(), vec![]),
        ];
        let result = resolve_dependencies(&modules, None, None).unwrap();
        let a_pos = result.iter().position(|x| x == "a").unwrap();
        let b_pos = result.iter().position(|x| x == "b").unwrap();
        assert!(a_pos < b_pos);
    }

    #[test]
    fn test_circular_dependency() {
        let modules = vec![
            (
                "a".to_string(),
                vec![DepInfo {
                    module_id: "b".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
            (
                "b".to_string(),
                vec![DepInfo {
                    module_id: "a".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
        ];
        let result = resolve_dependencies(&modules, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_required_dependency() {
        let modules = vec![(
            "a".to_string(),
            vec![DepInfo {
                module_id: "missing".to_string(),
                version: None,
                optional: false,
            }],
        )];
        let err = resolve_dependencies(&modules, None, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::DependencyNotFound);
        assert_eq!(
            err.details.get("module_id").and_then(|v| v.as_str()),
            Some("a")
        );
        assert_eq!(
            err.details.get("dependency_id").and_then(|v| v.as_str()),
            Some("missing")
        );
    }

    #[test]
    fn test_circular_dependency_details_cycle_path() {
        let modules = vec![
            (
                "a".to_string(),
                vec![DepInfo {
                    module_id: "b".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
            (
                "b".to_string(),
                vec![DepInfo {
                    module_id: "a".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
        ];
        let err = resolve_dependencies(&modules, None, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::CircularDependency);
        let cycle = err
            .details
            .get("cycle_path")
            .and_then(|v| v.as_array())
            .expect("cycle_path must be present in details as an array");
        let nodes: Vec<&str> = cycle.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            nodes.first(),
            nodes.last(),
            "cycle must start/end at same node: {nodes:?}"
        );
        let interior: HashSet<&str> = nodes[..nodes.len() - 1].iter().copied().collect();
        assert_eq!(
            interior,
            ["a", "b"].into_iter().collect::<HashSet<&str>>(),
            "cycle nodes must be {{a,b}}: {nodes:?}"
        );
    }

    #[test]
    fn test_cycle_path_is_actual_cycle() {
        // Regression: module C blocked on external known-but-not-in-batch dep
        // must not get lumped into the cycle path alongside the real A<->B cycle.
        let modules = vec![
            (
                "A".to_string(),
                vec![DepInfo {
                    module_id: "B".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
            (
                "B".to_string(),
                vec![DepInfo {
                    module_id: "A".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
            (
                "C".to_string(),
                vec![DepInfo {
                    module_id: "external".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
        ];
        let known_ids: HashSet<String> = ["A", "B", "C", "external"]
            .into_iter()
            .map(String::from)
            .collect();
        // Reach into the private extractor directly — it receives the exact
        // `remaining` set that resolve_dependencies would compute (all three IDs).
        let remaining: HashSet<String> = ["A", "B", "C"].iter().map(|s| (*s).to_string()).collect();
        let path = find_back_edge_cycle(&modules, &remaining).expect("a real A->B->A cycle");
        // Sanity check: resolve_dependencies itself still fails with a cycle error.
        assert!(resolve_dependencies(&modules, Some(&known_ids), None).is_err());
        // cycle_path must start and end at the same node
        assert_eq!(
            path.first(),
            path.last(),
            "path must form a cycle: {path:?}"
        );
        // cycle nodes must be exactly {A, B}; C was only blocked on an external
        let interior: HashSet<&str> = path[..path.len() - 1].iter().map(String::as_str).collect();
        assert_eq!(
            interior,
            ["A", "B"].into_iter().collect::<HashSet<&str>>(),
            "cycle should span {{A,B}}, got {path:?}"
        );
    }

    #[test]
    fn test_genuine_cycle_reports_circular_dependency() {
        // D-79, branch 1: a real A -> B -> A back edge IS a cycle.
        let modules = vec![
            (
                "a".to_string(),
                vec![DepInfo {
                    module_id: "b".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
            (
                "b".to_string(),
                vec![DepInfo {
                    module_id: "a".to_string(),
                    version: None,
                    optional: false,
                }],
            ),
        ];
        let err = resolve_dependencies(&modules, None, None).expect_err("cycle must fail");
        assert_eq!(err.code, ErrorCode::CircularDependency);
        let cycle = err
            .details
            .get("cycle_path")
            .and_then(|v| v.as_array())
            .expect("a real cycle carries cycle_path");
        assert!(cycle.len() >= 3, "cycle_path: {cycle:?}");
    }

    #[test]
    fn test_stall_without_cycle_reports_module_load_error() {
        // D-79, branch 2: `b` is registered but is NOT in this batch, so `a`
        // never reaches in-degree zero. That is a blocked load, not a cycle —
        // the two need opposite fixes, so reporting CIRCULAR_DEPENDENCY with a
        // one-element cycle_path sent the author looking for a loop that does
        // not exist.
        let modules = vec![(
            "a".to_string(),
            vec![DepInfo {
                module_id: "b".to_string(),
                version: None,
                optional: false,
            }],
        )];
        let known_ids: HashSet<String> = ["a", "b"].into_iter().map(String::from).collect();
        let err = resolve_dependencies(&modules, Some(&known_ids), None)
            .expect_err("a blocked batch member must fail");
        assert_eq!(err.code, ErrorCode::ModuleLoadError);
        assert!(
            !err.details.contains_key("cycle_path"),
            "a stall with no cycle must NOT fabricate a cycle_path: {:?}",
            err.details
        );
        assert!(
            err.message.contains('a'),
            "the blocked module must be named: {}",
            err.message
        );
        assert_eq!(
            err.details.get("blocked_modules"),
            Some(&json!(["a"])),
            "details: {:?}",
            err.details
        );
    }

    #[test]
    fn test_missing_optional_dependency() {
        let modules = vec![(
            "a".to_string(),
            vec![DepInfo {
                module_id: "missing".to_string(),
                version: None,
                optional: true,
            }],
        )];
        let result = resolve_dependencies(&modules, None, None).unwrap();
        assert_eq!(result, vec!["a"]);
    }

    fn make_dep(module_id: &str, version: Option<&str>) -> DepInfo {
        DepInfo {
            module_id: module_id.to_string(),
            version: version.map(String::from),
            optional: false,
        }
    }

    #[test]
    fn test_version_constraint_satisfied() {
        let modules = vec![
            ("a".to_string(), vec![make_dep("b", Some(">=1.0.0"))]),
            ("b".to_string(), vec![]),
        ];
        let versions: HashMap<String, String> = [("a", "1.0.0"), ("b", "1.2.3")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let result = resolve_dependencies(&modules, None, Some(&versions)).unwrap();
        assert_eq!(result, vec!["b", "a"]);
    }

    #[test]
    fn test_version_constraint_violated() {
        let modules = vec![
            ("a".to_string(), vec![make_dep("b", Some(">=2.0.0"))]),
            ("b".to_string(), vec![]),
        ];
        let versions: HashMap<String, String> = [("a", "1.0.0"), ("b", "1.2.3")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let err = resolve_dependencies(&modules, None, Some(&versions)).unwrap_err();
        assert_eq!(err.code, ErrorCode::DependencyVersionMismatch);
        assert_eq!(
            err.details.get("module_id").and_then(|v| v.as_str()),
            Some("a")
        );
        assert_eq!(
            err.details.get("dependency_id").and_then(|v| v.as_str()),
            Some("b")
        );
        assert_eq!(
            err.details.get("required").and_then(|v| v.as_str()),
            Some(">=2.0.0")
        );
        assert_eq!(
            err.details.get("actual").and_then(|v| v.as_str()),
            Some("1.2.3")
        );
    }

    #[test]
    fn test_caret_and_tilde_constraints() {
        let make_modules = |constraint: &str| {
            vec![
                ("a".to_string(), vec![make_dep("b", Some(constraint))]),
                ("b".to_string(), vec![]),
            ]
        };
        let mk_versions = |b: &str| -> HashMap<String, String> {
            [("a", "1.0.0"), ("b", b)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // ^1.2.3 accepts 1.x but not 2.x
        assert!(
            resolve_dependencies(&make_modules("^1.2.3"), None, Some(&mk_versions("1.9.0")))
                .is_ok()
        );
        assert!(
            resolve_dependencies(&make_modules("^1.2.3"), None, Some(&mk_versions("2.0.0")))
                .is_err()
        );
        // ~1.2.3 accepts 1.2.x but not 1.3.0
        assert!(
            resolve_dependencies(&make_modules("~1.2.3"), None, Some(&mk_versions("1.2.9")))
                .is_ok()
        );
        assert!(
            resolve_dependencies(&make_modules("~1.2.3"), None, Some(&mk_versions("1.3.0")))
                .is_err()
        );
    }

    #[test]
    fn test_version_ignored_when_map_absent() {
        let modules = vec![
            ("a".to_string(), vec![make_dep("b", Some(">=99.0.0"))]),
            ("b".to_string(), vec![]),
        ];
        let result = resolve_dependencies(&modules, None, None).unwrap();
        assert_eq!(result, vec!["b", "a"]);
    }
}
