//! Execution-time governance policy: external overrides for governance annotations.
//!
//! RFC pilot for apcore#76 ("first-class external governance / policy API").
//! An [`ExecutionPolicy`] lets a platform operator override the governance
//! annotations (`requires_approval`, `destructive`) of already-registered
//! modules at execution time, independent of how the modules were registered.
//! It attaches to the [`Executor`](crate::executor::Executor) (via
//! [`Executor::set_policy`](crate::executor::Executor::set_policy)) and is
//! consulted by the approval gate (Step 5).
//!
//! Precedence: a matched policy rule overrides the module's own declared or
//! scanned annotations — external governance is the platform's word against the
//! module author's. Matching reuses the same wildcard semantics as the ACL
//! system (Algorithm A08, [`match_pattern`](crate::utils::match_pattern)) and
//! the same specificity scoring (Algorithm A10,
//! [`calculate_specificity`](crate::utils::calculate_specificity)); on a
//! specificity tie the more restrictive rule wins.
//!
//! Governance principle (apcore#76): a misconfigured or unreachable governance
//! control must warn or error, never silently allow. [`ExecutionPolicy::from_value`]
//! therefore rejects unknown keys (a typo in a governance file must not silently
//! no-op), and `strict = true` makes the approval gate fail closed when approval
//! is required but no `ApprovalHandler` is configured.

use serde::Deserialize;

use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::module::ModuleAnnotations;
use crate::utils::{calculate_specificity, match_pattern};

/// The framework-owned Phase B approval token key (PROTOCOL_SPEC §7.4).
/// Stripped from `arguments` before policy resolution (§7.9.6 rule 5) and
/// excluded from the §6.1.8 governance projection on the same grounds.
pub(crate) const APPROVAL_TOKEN_KEY: &str = "_approval_token";

/// A single pattern-based governance override.
///
/// `requires_approval` / `destructive` are `Option<bool>`: `None` leaves the
/// module's own value in effect; `Some(v)` overrides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRule {
    pattern: String,
    requires_approval: Option<bool>,
    destructive: Option<bool>,
    reason: Option<String>,
}

impl PolicyRule {
    /// Build a rule from a non-empty wildcard `pattern` (Algorithm A08
    /// semantics, same as ACL rules; `*` matches any character sequence
    /// including dots).
    ///
    /// # Errors
    /// Returns [`ErrorCode::GeneralInvalidInput`] when `pattern` is empty — a
    /// malformed governance rule must fail loud.
    pub fn new(pattern: impl Into<String>) -> Result<Self, ModuleError> {
        let pattern = pattern.into();
        if pattern.is_empty() {
            return Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                "PolicyRule.pattern must be a non-empty string",
            ));
        }
        Ok(Self {
            pattern,
            requires_approval: None,
            destructive: None,
            reason: None,
        })
    }

    /// Set the `requires_approval` override.
    #[must_use]
    pub fn with_requires_approval(mut self, value: bool) -> Self {
        self.requires_approval = Some(value);
        self
    }

    /// Set the `destructive` override.
    #[must_use]
    pub fn with_destructive(mut self, value: bool) -> Self {
        self.destructive = Some(value);
        self
    }

    /// Set the human-readable rationale, carried into the audit trail.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// The rule's module-ID wildcard pattern.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The `requires_approval` override, or `None` when the rule does not touch it.
    #[must_use]
    pub fn requires_approval(&self) -> Option<bool> {
        self.requires_approval
    }

    /// The `destructive` override, or `None` when the rule does not touch it.
    #[must_use]
    pub fn destructive(&self) -> Option<bool> {
        self.destructive
    }

    /// The rule's rationale, if any.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Tie-break key: rules that force gating rank above rules that relax it.
    fn restrictiveness(&self) -> (bool, bool) {
        (
            self.requires_approval == Some(true),
            self.destructive == Some(true),
        )
    }
}

/// The effective governance verdict for one module under a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // spec-defined governance flags; each is a distinct verdict, not a state enum
pub struct PolicyDecision {
    /// The module the decision applies to.
    pub module_id: String,
    /// Effective `requires_approval` after overrides.
    pub requires_approval: bool,
    /// Effective `destructive` after overrides.
    pub destructive: bool,
    /// Final approval-gate verdict — true when the effective `requires_approval`
    /// is true, or when the policy gates destructive modules and the effective
    /// `destructive` is true.
    pub needs_approval: bool,
    /// The matched rule, or `None` when no rule matched.
    pub rule: Option<PolicyRule>,
    /// True when the matched rule changed a base annotation value.
    pub overridden: bool,
}

/// Declarative execution-time governance overrides (apcore#76 RFC pilot).
#[derive(Debug, Clone, Default)]
pub struct ExecutionPolicy {
    rules: Vec<PolicyRule>,
    /// When true, modules whose effective `destructive` annotation is true
    /// require approval even if `requires_approval` is false — the opt-in
    /// resolution of the destructive↔approval gap described in apcore#76.
    pub gate_destructive: bool,
    /// When true, the approval gate fails closed (denies) when a module needs
    /// approval but no `ApprovalHandler` is configured. When false (default),
    /// the gate keeps the `PROTOCOL_SPEC` §7.4 skip behavior but logs a warning.
    pub strict: bool,
}

impl ExecutionPolicy {
    /// Build a policy from ordered pattern-based overrides. The most specific
    /// matching rule wins (Algorithm A10 specificity); on a tie the more
    /// restrictive rule wins, then the earlier-defined rule.
    #[must_use]
    pub fn new(rules: Vec<PolicyRule>) -> Self {
        Self {
            rules,
            gate_destructive: false,
            strict: false,
        }
    }

    /// Set `gate_destructive` (builder form).
    #[must_use]
    pub fn with_gate_destructive(mut self, value: bool) -> Self {
        self.gate_destructive = value;
        self
    }

    /// Set `strict` (builder form).
    #[must_use]
    pub fn with_strict(mut self, value: bool) -> Self {
        self.strict = value;
        self
    }

    /// The policy's rules.
    #[must_use]
    pub fn rules(&self) -> &[PolicyRule] {
        &self.rules
    }

    /// Compute the effective governance decision for a module.
    ///
    /// Only `requires_approval` and `destructive` of `annotations` are consulted.
    ///
    /// Prefer [`Self::resolve_with_call_site`] from inside the execution
    /// pipeline: PROTOCOL_SPEC §7.9.6 rule 1 requires policy resolution to
    /// *receive* the invocation's arguments and `Context`. This form remains
    /// for callers that have no call site (preflight tooling, tests, an
    /// operator asking "what would this policy do to module X?").
    #[must_use]
    pub fn resolve(
        &self,
        module_id: &str,
        annotations: Option<&ModuleAnnotations>,
    ) -> PolicyDecision {
        let base_requires_approval = annotations.is_some_and(|a| a.requires_approval);
        let base_destructive = annotations.is_some_and(|a| a.destructive);

        let rule = self.match_rule(module_id);
        let effective_requires_approval = match rule.and_then(PolicyRule::requires_approval) {
            Some(v) => v,
            None => base_requires_approval,
        };
        let effective_destructive = match rule.and_then(PolicyRule::destructive) {
            Some(v) => v,
            None => base_destructive,
        };

        PolicyDecision {
            module_id: module_id.to_string(),
            requires_approval: effective_requires_approval,
            destructive: effective_destructive,
            needs_approval: effective_requires_approval
                || (self.gate_destructive && effective_destructive),
            rule: rule.cloned(),
            overridden: effective_requires_approval != base_requires_approval
                || effective_destructive != base_destructive,
        }
    }

    /// Compute the effective governance decision for a module, given the call
    /// site (PROTOCOL_SPEC §7.9.6, spec v1.24.0, apcore#102).
    ///
    /// Through v1.23.0 a policy could see only *which* module was being called,
    /// never *what it was being called with*, so an operator who needed to gate
    /// **some** calls to a module had to gate **all** of them — audit noise,
    /// and `requires_approval` weakened from "this needs approval" to "this
    /// might". The pipeline already holds the missing data at the point of the
    /// decision: the approval gate is Step 5 and the invocation's `arguments`
    /// and [`Context`] are in scope there.
    ///
    /// # The built-in rules do not consult the call site
    ///
    /// §7.9.6 rule 2 is a **MUST NOT**: a rule set's verdict stays a function
    /// of the module ID and the annotations alone, so it remains statically
    /// auditable and reproducible from the policy document. Rule 5 makes it a
    /// MUST that adding the call site changes no existing verdict, and this
    /// method delivers that by construction — it forwards to [`Self::resolve`].
    /// The call site is carried into the trace (rule 3a) so it reaches the
    /// audit trail alongside `apcore.policy.override`.
    ///
    /// # `arguments` have NOT been schema-validated
    ///
    /// §7.9.6 rule 4: the approval gate is Step 5 and input validation is
    /// Step 7 (§12.8), so `arguments` here is whatever the caller passed,
    /// possibly after `middleware_before` mutated it. It may be absent, of the
    /// wrong type, or missing required properties. Any code reading it MUST
    /// treat it as untrusted and MUST NOT assume it is well-formed.
    ///
    /// # The framework-owned approval token is stripped
    ///
    /// §7.9.6 rule 5: `_approval_token` (§7.4) **MUST** be stripped from
    /// `arguments` before policy resolution. §7.4 already requires it to be
    /// removed "before passing to subsequent steps", which does not reach this
    /// case — resolution happens *inside* Step 5, ahead of any subsequent step,
    /// so an implementation can satisfy §7.4 literally and still hand the token
    /// to the policy. It is a protocol-level key, not caller input; leaving it
    /// in place puts a token into the audit trail and the
    /// `apcore.policy.override` payload.
    ///
    /// The strip happens **here**, at the boundary of the policy API, rather
    /// than at each call site. A caller cannot then forget it, and no future
    /// call site can reintroduce the leak. It costs one shallow clone of the
    /// arguments object, and only when the token is actually present — a Phase
    /// B resume, not the common path.
    ///
    /// # Why an added method rather than a wider `resolve`
    ///
    /// §7.9.6 rule 7 leaves the API shape to the language and then notes the
    /// constraint explicitly: because rule 6 forbids the new inputs from
    /// changing any verdict, widening a published signature buys nothing and
    /// costs every external caller a compile error, so "a language without
    /// default arguments or overloading **SHOULD** add a method rather than
    /// widen". Rust is that language, and `resolve` is `pub` on a published
    /// crate.
    #[must_use]
    pub fn resolve_with_call_site(
        &self,
        module_id: &str,
        annotations: Option<&ModuleAnnotations>,
        arguments: Option<&serde_json::Value>,
        context: Option<&Context<serde_json::Value>>,
    ) -> PolicyDecision {
        let arguments = arguments.map(strip_approval_token);

        // §7.9.6 rule 3: carry the call site into the trace. What is recorded
        // MUST be the call site's *shape* — which keys were present, their
        // types — and MUST NOT be argument values: the gate runs before input
        // validation and an argument may hold anything, including a secret.
        tracing::trace!(
            module_id = %module_id,
            has_arguments = arguments.is_some(),
            argument_shape = ?arguments.as_deref().map(argument_shape),
            trace_id = context.map(|c| c.trace_id.as_str()),
            call_depth = context.map(|c| c.call_chain.len()),
            "policy resolution call site (§7.9.6; NOT consulted by the built-in pattern rules, \
             and NOT schema-validated — the approval gate is Step 5, input validation is Step 7)"
        );
        self.resolve(module_id, annotations)
    }

    /// Return the winning rule for a module ID, or `None`.
    fn match_rule(&self, module_id: &str) -> Option<&PolicyRule> {
        let mut best: Option<&PolicyRule> = None;
        let mut best_score: i64 = -1;
        for rule in &self.rules {
            if !match_pattern(&rule.pattern, module_id) {
                continue;
            }
            let score = i64::from(calculate_specificity(&rule.pattern));
            if score > best_score {
                best = Some(rule);
                best_score = score;
            } else if score == best_score {
                if let Some(current) = best {
                    if rule.restrictiveness() > current.restrictiveness() {
                        best = Some(rule);
                    }
                }
            }
        }
        best
    }

    /// Build a policy from a parsed mapping (YAML/JSON governance file).
    ///
    /// Parsing is strict: unknown keys, a missing `pattern`, or wrong types
    /// error so a typo in a governance file fails loud instead of silently
    /// disabling a control.
    ///
    /// Expected shape:
    /// ```yaml
    /// gate_destructive: true
    /// strict: true
    /// rules:
    ///   - pattern: "orders.delete_*"
    ///     requires_approval: true
    ///     reason: "destructive order operations need human sign-off"
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorCode::GeneralInvalidInput`] on unknown keys, a missing
    /// `pattern`, an empty `pattern`, or a structurally invalid document.
    pub fn from_value(value: serde_json::Value) -> Result<Self, ModuleError> {
        let doc: PolicyDoc = serde_json::from_value(value).map_err(|e| {
            ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!("Invalid policy document; a governance file must not silently no-op: {e}"),
            )
        })?;

        let mut rules: Vec<PolicyRule> = Vec::with_capacity(doc.rules.len());
        for raw in doc.rules {
            let mut rule = PolicyRule::new(raw.pattern)?;
            if let Some(v) = raw.requires_approval {
                rule = rule.with_requires_approval(v);
            }
            if let Some(v) = raw.destructive {
                rule = rule.with_destructive(v);
            }
            if let Some(r) = raw.reason {
                rule = rule.with_reason(r);
            }
            rules.push(rule);
        }

        Ok(Self {
            rules,
            gate_destructive: doc.gate_destructive,
            strict: doc.strict,
        })
    }
}

/// Remove the framework-owned `_approval_token` (§7.4) from `arguments`
/// (PROTOCOL_SPEC §7.9.6 rule 5).
///
/// Borrows unchanged in the common case; clones only when the token is
/// actually present, which is a Phase B resume.
fn strip_approval_token(arguments: &serde_json::Value) -> std::borrow::Cow<'_, serde_json::Value> {
    let Some(obj) = arguments.as_object() else {
        return std::borrow::Cow::Borrowed(arguments);
    };
    if !obj.contains_key(APPROVAL_TOKEN_KEY) {
        return std::borrow::Cow::Borrowed(arguments);
    }
    let mut cleaned = obj.clone();
    cleaned.remove(APPROVAL_TOKEN_KEY);
    std::borrow::Cow::Owned(serde_json::Value::Object(cleaned))
}

/// The call site's **shape** — each top-level key paired with its JSON type
/// name — and never a value (§7.9.6 rule 3).
fn argument_shape(arguments: &serde_json::Value) -> Vec<(&str, &'static str)> {
    arguments.as_object().map_or_else(Vec::new, |obj| {
        obj.iter()
            .map(|(key, value)| {
                let type_name = match value {
                    serde_json::Value::Null => "null",
                    serde_json::Value::Bool(_) => "boolean",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::String(_) => "string",
                    serde_json::Value::Array(_) => "array",
                    serde_json::Value::Object(_) => "object",
                };
                (key.as_str(), type_name)
            })
            .collect()
    })
}

// --- Strict serde intermediates for from_value -----------------------------
// `deny_unknown_fields` gives the fail-loud unknown-key behavior; the required
// `pattern` field errors when missing.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyDoc {
    #[serde(default)]
    rules: Vec<RuleDoc>,
    #[serde(default)]
    gate_destructive: bool,
    #[serde(default)]
    strict: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleDoc {
    pattern: String,
    #[serde(default)]
    requires_approval: Option<bool>,
    #[serde(default)]
    destructive: Option<bool>,
    #[serde(default)]
    reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn annotations(requires_approval: bool, destructive: bool) -> ModuleAnnotations {
        ModuleAnnotations {
            destructive,
            requires_approval,
            ..ModuleAnnotations::default()
        }
    }

    // --- PolicyRule validation ---------------------------------------------

    #[test]
    fn valid_rule() {
        let rule = PolicyRule::new("orders.*")
            .unwrap()
            .with_requires_approval(true)
            .with_reason("ops sign-off");
        assert_eq!(rule.pattern(), "orders.*");
        assert_eq!(rule.requires_approval(), Some(true));
        assert_eq!(rule.destructive(), None);
        assert_eq!(rule.reason(), Some("ops sign-off"));
    }

    #[test]
    fn empty_pattern_rejected() {
        let err = PolicyRule::new("").unwrap_err();
        assert!(err.message.contains("pattern"));
    }

    // --- from_value strict parsing -----------------------------------------

    #[test]
    fn full_document() {
        let policy = ExecutionPolicy::from_value(json!({
            "gate_destructive": true,
            "strict": true,
            "rules": [
                {"pattern": "orders.delete_*", "requires_approval": true, "reason": "sign-off"},
                {"pattern": "reports.*", "destructive": false},
            ],
        }))
        .unwrap();
        assert!(policy.gate_destructive);
        assert!(policy.strict);
        assert_eq!(policy.rules().len(), 2);
        assert_eq!(policy.rules()[0].pattern(), "orders.delete_*");
    }

    #[test]
    fn empty_document() {
        let policy = ExecutionPolicy::from_value(json!({})).unwrap();
        assert_eq!(policy.rules(), []);
        assert!(!policy.gate_destructive);
        assert!(!policy.strict);
    }

    #[test]
    fn non_mapping_rejected() {
        assert!(ExecutionPolicy::from_value(json!(["not", "a", "map"])).is_err());
    }

    #[test]
    fn unknown_policy_key_rejected() {
        let err = ExecutionPolicy::from_value(json!({"gate_destructiv": true})).unwrap_err();
        assert!(err.message.contains("gate_destructiv"));
    }

    #[test]
    fn rules_not_a_list_rejected() {
        assert!(ExecutionPolicy::from_value(json!({"rules": {"pattern": "a.b"}})).is_err());
    }

    #[test]
    fn rule_unknown_key_rejected() {
        let err = ExecutionPolicy::from_value(
            json!({"rules": [{"pattern": "a.b", "require_approval": true}]}),
        )
        .unwrap_err();
        assert!(err.message.contains("require_approval"));
    }

    #[test]
    fn rule_missing_pattern_rejected() {
        let err = ExecutionPolicy::from_value(json!({"rules": [{"requires_approval": true}]}))
            .unwrap_err();
        assert!(err.message.contains("pattern"));
    }

    #[test]
    fn rule_empty_pattern_rejected() {
        assert!(ExecutionPolicy::from_value(json!({"rules": [{"pattern": ""}]})).is_err());
    }

    // --- resolve: precedence and specificity -------------------------------

    #[test]
    fn no_rules_passes_annotations_through() {
        let decision = ExecutionPolicy::new(vec![]).resolve("a.b", Some(&annotations(true, true)));
        assert_eq!(
            decision,
            PolicyDecision {
                module_id: "a.b".to_string(),
                requires_approval: true,
                destructive: true,
                needs_approval: true,
                rule: None,
                overridden: false,
            }
        );
    }

    #[test]
    fn none_annotations_supported() {
        let decision = ExecutionPolicy::new(vec![]).resolve("a.b", None);
        assert!(!decision.needs_approval);
    }

    #[test]
    fn rule_forces_approval() {
        let policy = ExecutionPolicy::new(vec![PolicyRule::new("orders.delete_*")
            .unwrap()
            .with_requires_approval(true)]);
        let decision = policy.resolve("orders.delete_order", Some(&annotations(false, false)));
        assert!(decision.requires_approval);
        assert!(decision.needs_approval);
        assert!(decision.overridden);
        assert_eq!(decision.rule.unwrap().pattern(), "orders.delete_*");
    }

    #[test]
    fn rule_exempts_approval() {
        let policy = ExecutionPolicy::new(vec![PolicyRule::new("batch.*")
            .unwrap()
            .with_requires_approval(false)]);
        let decision = policy.resolve("batch.cleanup", Some(&annotations(true, false)));
        assert!(!decision.requires_approval);
        assert!(!decision.needs_approval);
        assert!(decision.overridden);
    }

    #[test]
    fn none_fields_do_not_override() {
        let policy =
            ExecutionPolicy::new(vec![PolicyRule::new("a.*").unwrap().with_destructive(true)]);
        let decision = policy.resolve("a.b", Some(&annotations(true, false)));
        assert!(decision.requires_approval); // untouched
        assert!(decision.destructive); // overridden
        assert!(decision.overridden);
    }

    #[test]
    fn unmatched_rule_is_ignored() {
        let policy = ExecutionPolicy::new(vec![PolicyRule::new("other.*")
            .unwrap()
            .with_requires_approval(true)]);
        let decision = policy.resolve("a.b", Some(&annotations(false, false)));
        assert!(decision.rule.is_none());
        assert!(!decision.needs_approval);
        assert!(!decision.overridden);
    }

    #[test]
    fn most_specific_rule_wins() {
        let policy = ExecutionPolicy::new(vec![
            PolicyRule::new("*").unwrap().with_requires_approval(true),
            PolicyRule::new("orders.list_orders")
                .unwrap()
                .with_requires_approval(false),
        ]);
        assert!(
            !policy
                .resolve("orders.list_orders", Some(&annotations(false, false)))
                .needs_approval
        );
        assert!(
            policy
                .resolve("orders.delete_order", Some(&annotations(false, false)))
                .needs_approval
        );
    }

    #[test]
    fn specificity_tie_more_restrictive_wins() {
        let policy = ExecutionPolicy::new(vec![
            PolicyRule::new("orders.*")
                .unwrap()
                .with_requires_approval(false),
            PolicyRule::new("orders.*")
                .unwrap()
                .with_requires_approval(true),
        ]);
        assert!(
            policy
                .resolve("orders.delete_order", Some(&annotations(false, false)))
                .needs_approval
        );
    }

    #[test]
    fn gate_destructive_requires_approval() {
        let policy = ExecutionPolicy::new(vec![]).with_gate_destructive(true);
        let decision = policy.resolve("orders.delete_order", Some(&annotations(false, true)));
        assert!(!decision.requires_approval);
        assert!(decision.needs_approval);
    }

    #[test]
    fn gate_destructive_with_override() {
        let policy = ExecutionPolicy::new(vec![PolicyRule::new("orders.delete_*")
            .unwrap()
            .with_destructive(true)])
        .with_gate_destructive(true);
        let decision = policy.resolve("orders.delete_order", Some(&annotations(false, false)));
        assert!(decision.destructive);
        assert!(decision.needs_approval);
    }
}
