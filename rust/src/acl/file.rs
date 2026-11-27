//! YAML loading and discovery for access control lists.

use std::path::Path;

use serde_yaml_ng as serde_yaml;

use crate::errors::{ErrorCode, ModuleError};

use super::{
    warn_audit_block_overridden, ACLRule, AuditConfig, AuditSink, ACL, AUDIT_FIELDS, EFFECTS,
};

/// The complete set of keys an ACL rule may carry (PROTOCOL_SPEC §6.1).
///
/// Closed on purpose: a key nothing evaluates is otherwise dropped in silence,
/// which widens an `allow` rule with no warning (#107).
const RULE_KEYS: &[&str] = &[
    "callers",
    "targets",
    "effect",
    "approval",
    "description",
    "conditions",
];

/// Reserved in earlier revisions of §6.1 and evaluated by no implementation.
///
/// Rejected like any other unknown key, but named as reserved in the message:
/// an operator who wrote `actions: ["describe"]` meant to restrict the rule and
/// is better served by "not implemented" than by "unknown key".
const RESERVED_RULE_KEYS: &[&str] = &["id", "actions", "priority"];

/// Reject any rule key outside [`RULE_KEYS`].
fn reject_unknown_rule_keys(
    index: usize,
    path: &str,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ModuleError> {
    let mut unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !RULE_KEYS.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let (reserved, other): (Vec<&str>, Vec<&str>) = unknown
        .into_iter()
        .partition(|k| RESERVED_RULE_KEYS.contains(k));
    let quote = |ks: &[&str]| {
        ks.iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts: Vec<String> = Vec::new();
    if !reserved.is_empty() {
        parts.push(format!(
            "{} reserved for a future specification version and evaluated by no implementation",
            quote(&reserved)
        ));
    }
    if !other.is_empty() {
        parts.push(format!("{} unrecognised", quote(&other)));
    }
    Err(ModuleError::new(
        ErrorCode::ACLRuleError,
        format!(
            "ACL rule {index} in '{path}' carries {}. The rule key set is closed ({}); \
             a key nothing evaluates would be dropped silently and leave the rule wider than written.",
            parts.join("; "),
            RULE_KEYS.join(", ")
        ),
    ))
}

/// Validate an ACL file's `audit:` block (§6.3.2 requirement 8).
///
/// Returns `None` only when the document declares no `audit` key at all.
/// **Presence, not truthiness**: `audit:` with nothing under it parses to null,
/// and the operator still wrote the block — a declaration with every setting at
/// its default, not an absence.
///
/// Validates the SUBTREE only: types and unknown keys inside the block. Every
/// other unrecognised root key in an ACL file keeps being ignored.
fn parse_audit_block(
    raw: &serde_json::Value,
    path: &str,
) -> Result<Option<AuditConfig>, ModuleError> {
    let Some(value) = raw.get("audit") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(AuditConfig::default()));
    }
    let Some(block) = value.as_object() else {
        return Err(ModuleError::new(
            ErrorCode::ConfigInvalid,
            format!("{path}: 'audit' must be a mapping (PROTOCOL_SPEC §6.3.2)"),
        ));
    };

    let mut unknown: Vec<&str> = block
        .keys()
        .map(String::as_str)
        .filter(|k| !AUDIT_FIELDS.contains(k))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        return Err(ModuleError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "{path}: unknown key(s) in the 'audit' block: {}. The block accepts exactly \
                 {} ($defs/AuditConfig in schemas/acl-config.schema.json).",
                unknown.join(", "),
                AUDIT_FIELDS.join(", ")
            ),
        ));
    }

    let mut config = AuditConfig::default();
    for key in ["enabled", "include_denied"] {
        if let Some(v) = block.get(key) {
            let Some(b) = v.as_bool() else {
                return Err(ModuleError::new(
                    ErrorCode::ConfigInvalid,
                    format!("{path}: 'audit.{key}' must be a boolean, got {v}"),
                ));
            };
            if key == "enabled" {
                config.enabled = b;
            } else {
                config.include_denied = b;
            }
        }
    }
    if let Some(v) = block.get("log_level") {
        let levels = ["trace", "debug", "info", "warn", "error"];
        let ok = v.as_str().is_some_and(|s| levels.contains(&s));
        if !ok {
            return Err(ModuleError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "{path}: 'audit.log_level' must be one of {}, got {v}",
                    levels.join(", ")
                ),
            ));
        }
        config.log_level = v.as_str().unwrap_or("info").to_string();
    }

    if !config.include_denied {
        // §6.3.2 requirement 6 — a notice, not a refusal. It withholds the
        // security-relevant half of the record, so an operator who wrote it
        // deliberately gets told once per load rather than stopped.
        tracing::warn!(
            path = %path,
            "This ACL file sets audit.include_denied: false, so DENIED access attempts will \
             not be recorded by the default audit sink (PROTOCOL_SPEC §6.3.2 requirement 6). \
             Allowed calls are still recorded. Remove the entry to restore denials."
        );
    }
    Ok(Some(config))
}

impl ACL {
    /// Load ACL rules from a YAML file.
    pub fn load(path: &str) -> Result<Self, ModuleError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            ModuleError::new(
                ErrorCode::ConfigNotFound,
                format!("Failed to read ACL file '{path}': {e}"),
            )
        })?;

        // Sync finding A-D-022: structural ACL parse/validation errors carry
        // `ErrorCode::ACLRuleError` per spec contract — apcore-python and
        // apcore-typescript both raise `ACLRuleError`. Previously Rust used
        // `ErrorCode::ConfigInvalid`, which broke cross-language fixtures
        // asserting on the error code.
        let raw: serde_json::Value = serde_yaml::from_str(&content).map_err(|e| {
            ModuleError::new(
                ErrorCode::ACLRuleError,
                format!("Failed to parse ACL file '{path}': {e}"),
            )
        })?;

        // PROTOCOL_SPEC §9.2.4.1 (aiperceivable/apcore#118): an `audit:` block
        // in an ACL file has never been read. Deleting it from
        // `acl-config.schema.json` would produce no signal at all — no
        // implementation validates an ACL file against that schema, and this
        // loader parses into a `serde_json::Value` and takes the fields it
        // wants, so any unknown root key is dropped in silence. The diagnostic
        // therefore has to live here.
        //
        // Scoped to `audit` deliberately: §6.3.2 requirement 8 validates this
        // SUBTREE and nothing else, so every other unrecognised root key in an
        // ACL file keeps being ignored exactly as before. This was never
        // unknown-key closure for ACL files, and wiring the block does not make
        // it one.
        //
        // The §9.2.4.1 deprecation notice that used to stand here is gone: spec
        // v1.45.0 gave the block a delivery contract, and a key that has gained
        // a consumer must stop being announced as going away.
        let audit_config = parse_audit_block(&raw, path)?;

        // §6.2.1 point 2: `default_effect` is judged FIRST — ahead of the
        // individual rules AND ahead of the file-level checks on the `rules`
        // collection itself. It used to sit behind the missing-`rules` check
        // below, so a document wrong in both was refused for the absent
        // `rules` key at this door while `ACL::try_new`, which cannot see that
        // fault at all, named the `default_effect`. A non-list `rules` is
        // caught further down still, by the deserialization, and was already
        // behind this.
        //
        // Distinguish "key absent" (→ canonical default `deny`) from "key
        // present but not a string". The previous
        // `.and_then(as_str).unwrap_or("deny")` silently coerced a non-string
        // such as `default_effect: true` into a valid value, so `try_new`'s
        // validation never fired and an operator typo went unreported.
        // apcore-python and apcore-typescript both raise `ACLRuleError` here.
        // The outcome was fail-closed either way, so this is a diagnostics fix,
        // not a bypass.
        //
        // An explicit `default_effect: null` is NOT the same as an absent key:
        // `schemas/acl-config.schema.json` declares `default_effect` as a plain
        // `allow` / `deny` string enum, not nullable, so a written `null` is a
        // type violation the operator asked for, not a request for the
        // canonical default. Only OMISSION defaults to `deny`; `Value::Null`
        // falls through to the same rejection as any other non-string value.
        let default_effect = match raw.get("default_effect") {
            None => "deny".to_string(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => {
                return Err(ModuleError::new(
                    ErrorCode::ACLRuleError,
                    format!(
                        "ACL file '{path}': default_effect must be the string 'allow' or 'deny' (got {other})"
                    ),
                ))
            }
        };

        // PROTOCOL_SPEC §6.2.1 point 2 (spec v1.31.0, #112): the RULE INDEX
        // dominates every axis, so a file with more than one bad rule is
        // refused for the LOWEST-INDEXED one. The `default_effect` is not a
        // rule and has no index, so it is judged first — which is also the
        // order `try_new` uses, and this door and that one have to agree.
        //
        // Checked here rather than left to `try_new` below because the rule
        // loop that follows now raises the rule-level refusals itself: leaving
        // `default_effect` to `try_new` would put every rule fault ahead of it
        // at THIS door and behind it at the constructor, which is precisely
        // the cross-door disagreement §6.2.1 point 2 exists to end.
        if !EFFECTS.contains(&default_effect.as_str()) {
            return Err(ModuleError::new(
                ErrorCode::ACLRuleError,
                format!(
                    "Invalid ACL config in '{path}': Invalid default_effect \
                     '{default_effect}': must be 'allow' or 'deny'"
                ),
            ));
        }

        // Expect top-level "rules" key — the first file-level check on the
        // collection, and behind `default_effect` on §6.2.1 point 2.
        let rules_val = raw.get("rules").ok_or_else(|| {
            ModuleError::new(
                ErrorCode::ACLRuleError,
                format!("ACL file '{path}' missing 'rules' key"),
            )
        })?;

        // A12-ACL: every rule MUST explicitly declare `callers` and `targets`.
        // `#[serde(default)]` on those fields would otherwise let an omitted
        // key load as an empty Vec, silently turning a deny rule inert. Reject
        // a missing key here (the key may be an empty list — only OMISSION is
        // rejected), matching apcore-python acl.py:270 and apcore-typescript
        // acl.ts:74 which raise ACLRuleError at load (sync finding A-D-09).
        //
        // Every axis for rule `i` is judged before rule `i + 1` is looked at.
        // The loop used to run the key checks across the WHOLE list and leave
        // the rest to `try_new`, so a file whose rule 0 carried an unrecognised
        // `effect` and whose rule 1 carried an unknown key was refused for
        // rule 1 — an axis swept across every rule ahead of the next axis,
        // which §6.2.1 point 2 forbids by name.
        if let Some(rules_arr) = rules_val.as_array() {
            for (i, raw_rule) in rules_arr.iter().enumerate() {
                let Some(obj) = raw_rule.as_object() else {
                    return Err(ModuleError::new(
                        ErrorCode::ACLRuleError,
                        format!("ACL rule {i} in '{path}' must be a mapping"),
                    ));
                };
                for key in ["callers", "targets"] {
                    if !obj.contains_key(key) {
                        return Err(ModuleError::new(
                            ErrorCode::ACLRuleError,
                            format!("ACL rule {i} in '{path}' missing required key '{key}'"),
                        ));
                    }
                }

                // A missing key was already rejected above so an omission cannot
                // render a rule inert; an unknown key is the same hazard pointing
                // the other way, and was dropped in silence until #107.
                reject_unknown_rule_keys(i, path, obj)?;

                // ...and this rule's `effect` / `approval` / patterns, in
                // §6.2.1 point 2's order, through the same `validate_rule`
                // every other door uses. `try_new` re-runs it below; that is
                // deliberate — the funnel stays the single place the three
                // axes are defined, and this loop only decides WHEN it runs.
                let rule: ACLRule = serde_json::from_value(raw_rule.clone()).map_err(|e| {
                    ModuleError::new(
                        ErrorCode::ACLRuleError,
                        format!("Invalid ACL rules in '{path}': {e}"),
                    )
                })?;
                Self::validate_rule(i, &rule).map_err(|e| {
                    ModuleError::new(
                        ErrorCode::ACLRuleError,
                        format!("Invalid ACL config in '{path}': {}", e.message),
                    )
                })?;
            }
        }

        let rules: Vec<ACLRule> = serde_json::from_value(rules_val.clone()).map_err(|e| {
            ModuleError::new(
                ErrorCode::ACLRuleError,
                format!("Invalid ACL rules in '{path}': {e}"),
            )
        })?;

        // Propagate try_new validation errors as Result rather than panicking
        // — YAML errors must not crash the host process (sync finding A-D-302).
        let mut acl = Self::try_new(rules, default_effect, None).map_err(|e| {
            ModuleError::new(
                ErrorCode::ACLRuleError,
                format!("Invalid ACL config in '{path}': {}", e.message),
            )
        })?;
        acl.yaml_path = Some(path.to_string());
        // §6.3.2 requirement 2 — the declared block becomes the default sink's
        // configuration. `try_new` above built a sink with no config because it
        // knows nothing about files.
        acl.audit_config = audit_config;
        acl.audit_sink = AuditSink::new(acl.audit_logger.clone(), acl.audit_config.clone());
        Ok(acl)
    }

    /// Activate `acl.root` config-driven ACL discovery (D-64, Recommendation A).
    ///
    /// Resolves the `acl.root` config key (default `"./acl"`) and loads an ACL
    /// from it when the path exists. Relative values use the project root
    /// captured by [`crate::config::Config::project_root`].
    ///
    /// `acl.root` is a directory by spec convention (`acl/{scope}_acl.yaml`,
    /// PROTOCOL_SPEC §3.1). When the resolved path is a directory, the
    /// conventional `global_acl.yaml` within it is loaded; if that file is
    /// absent the result is `None` (the missing-path no-op still holds). When
    /// the resolved path is itself a file, it is loaded directly. Either way the
    /// actual load goes through [`ACL::load`].
    ///
    /// **Critical invariant:** a missing path returns `None` and attaches
    /// NOTHING. It MUST NOT synthesize an empty default-deny ACL — doing so
    /// would silently deny every inter-module call in every project that lacks
    /// an `acl` dir. Missing path means "no enforcement", identical to pre-D-64
    /// behavior. [`ACL::load`] reads `default_effect` from the ACL file itself.
    ///
    /// # Errors
    /// Returns [`ModuleError`] when a policy path cannot be inspected or
    /// the policy cannot be loaded. A missing path yields `Ok(None)`.
    pub fn discover(config: &crate::config::Config) -> Result<Option<Self>, ModuleError> {
        let Some(root_path) = config.get_path("acl.root") else {
            return Ok(None);
        };

        let Some(metadata) = Self::discover_metadata(&root_path)? else {
            return Ok(None);
        };

        let (path, metadata) = if metadata.is_dir() {
            let path = root_path.join("global_acl.yaml");
            let Some(metadata) = Self::discover_metadata(&path)? else {
                return Ok(None);
            };
            (path, metadata)
        } else {
            (root_path, metadata)
        };
        if !metadata.is_file() {
            return Err(ModuleError::new(
                ErrorCode::ConfigInvalid,
                format!("ACL path '{}' is not a regular file", path.display()),
            ));
        }
        Self::load(&path.to_string_lossy()).map(Some)
    }

    fn discover_metadata(path: &Path) -> Result<Option<std::fs::Metadata>, ModuleError> {
        let inspect_error = |error: std::io::Error| {
            ModuleError::new(
                ErrorCode::ConfigInvalid,
                format!("Cannot inspect ACL path '{}': {error}", path.display()),
            )
        };
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.is_symlink() => {
                std::fs::metadata(path).map(Some).map_err(inspect_error)
            }
            Ok(meta) => Ok(Some(meta)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(inspect_error(error)),
        }
    }

    /// Reload rules from the stored YAML path.
    ///
    /// **Deadlock avoidance:** the borrow on `self.yaml_path` is released
    /// (via clone + scope) *before* the blocking file I/O in [`Self::load`]
    /// begins. This matters when the caller holds the ACL inside an
    /// `Arc<RwLock<ACL>>`-style wrapper and an audit logger or condition
    /// handler tries to read the same lock from another thread mid-reload.
    /// Holding `&mut self` across `Self::load` would block any concurrent
    /// reader for the duration of the file read; the brace scope below
    /// makes that explicitly impossible (sync finding A-D-303).
    pub fn reload(&mut self) -> Result<(), ModuleError> {
        let path = {
            // Narrow scope: the immutable borrow on `self.yaml_path` ends
            // at the closing brace, *before* file I/O is initiated below.
            self.yaml_path.clone().ok_or_else(|| {
                ModuleError::new(
                    ErrorCode::ACLRuleError,
                    "Cannot reload: ACL was not loaded from a YAML file".to_string(),
                )
            })?
        };

        // File I/O happens here with no outstanding borrow on `self`.
        let reloaded = Self::load(&path)?;

        self.rules = reloaded.rules;
        self.default_effect = reloaded.default_effect;
        // §6.3.2 requirement 7: a reload refreshes the `audit:` block and
        // PRESERVES a programmatic callback, which was never read from the
        // file. Before v1.45.0 this method refreshed only the rules and the
        // default effect, so the audit block was the one part of the document a
        // reload did not pick up.
        //
        // Building a NEW sink is also what scopes requirement 5's
        // once-per-failure suppression: a reload that changes the sink's
        // configuration starts a fresh report rather than hiding a new failure
        // behind an old one.
        self.audit_config = reloaded.audit_config;
        self.audit_sink = AuditSink::new(self.audit_logger.clone(), self.audit_config.clone());
        warn_audit_block_overridden(self.audit_logger.as_ref(), self.audit_config.as_ref());
        // D-88: the reload may have repointed every index at a different rule.
        self.clear_missing_context_warnings();
        // `self.yaml_path` is intentionally left untouched: reload re-reads the
        // *stored* path, so reassigning it is a no-op (reloaded.yaml_path always
        // equals the existing path). Matches apcore-python / apcore-typescript,
        // which do not re-set the path on reload.
        Ok(())
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;
    use std::io::Write;

    // [acl-reload-yamlpath] reload() must leave `yaml_path` unchanged (it
    // re-reads the stored path). Matches apcore-python / apcore-typescript,
    // which do not re-set the path on reload.
    #[test]
    fn reload_leaves_yaml_path_unchanged() {
        let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
        writeln!(tmp, "default_effect: deny\nrules: []\n").expect("write tempfile");
        let path = tmp.path().to_str().expect("utf8").to_string();

        let mut acl = ACL::load(&path).expect("initial load");
        let before = acl.yaml_path.clone();
        assert_eq!(before, Some(path.clone()));

        acl.reload().expect("reload");
        assert_eq!(
            acl.yaml_path, before,
            "reload must not change the stored yaml_path"
        );
    }
}
