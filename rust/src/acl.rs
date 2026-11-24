// APCore Protocol — Access Control Lists
// Spec reference: ACL rules, audit entries

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_yaml_ng as serde_yaml;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};

use crate::acl_handlers::{
    evaluate_conditions_async_outcome as handlers_evaluate_conditions_async_outcome,
    precheck_conditions, register_builtin_handlers, ConditionOutcome, PrecheckPath, RuleFault,
    CONDITION_HANDLERS,
};
use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::utils::match_pattern;

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

/// The complete set of values a rule's `effect` may carry (PROTOCOL_SPEC §6.1).
///
/// Closed at **every** entry point since spec v1.30.0 (#111), not only at file
/// loading: an unrecognised `effect` must never be resolved to a decision.
/// Reading it as `deny` looks safe and is not — under `default_effect: allow` a
/// rule the operator wrote to permit becomes one that denies everything it
/// matches, with no error and no warning, and on a `deny` rule the fallback is
/// only ever *accidentally* right. `default_effect` is the same two values one
/// field up and is validated against this set too.
const EFFECTS: &[&str] = &["allow", "deny"];

/// Reserved in earlier revisions of §6.1 and evaluated by no implementation.
///
/// Rejected like any other unknown key, but named as reserved in the message:
/// an operator who wrote `actions: ["describe"]` meant to restrict the rule and
/// is better served by "not implemented" than by "unknown key".
const RESERVED_RULE_KEYS: &[&str] = &["id", "actions", "priority"];

/// The caller identity an absent `caller_id` resolves to (PROTOCOL_SPEC §6.5).
///
/// A call that carries no caller — an inbound external request, or an
/// in-process call made without a context — is not *unidentified* to the ACL:
/// it is identified as external, and rules are written against that. The
/// sentinel is matched as an **exact literal** by [`ACL::match_acl_pattern`];
/// no wildcard reaches it, so an operator granting external access must write
/// `callers: ["@external"]` verbatim.
///
/// Named rather than spelled inline because it has to agree across four
/// surfaces that are resolved in different modules — the ACL matcher, the
/// `AuditEntry` this crate writes, the `ACL_DENIED` error message a refused
/// caller reads, and the synthesized context `Executor` builds for a
/// context-less call. A literal drifting apart at one of them produced
/// apcore#37's follow-up: guidance that named an identity no rule could match.
pub const EXTERNAL_CALLER: &str = "@external";

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

/// The tokens a `callers` / `targets` array reserves for its compound
/// operators (PROTOCOL_SPEC §6.2.1).
///
/// Detected by **equality**, never by a `$` prefix or a substring: `$orders.*`
/// is an ordinary pattern that happens to begin with the same character, and a
/// prefix test would refuse a rule the specification permits.
const PATTERN_OPERATORS: &[&str] = &["$or", "$not"];

/// TIER 1 — why a `callers` / `targets` array is **structurally** illegal, or
/// `None` when its shape is legal (PROTOCOL_SPEC §6.2.1, spec v1.31.0, #112).
///
/// A pattern array is **FLAT**: the operators do not nest, there is no
/// precedence, an operand is always a plain pattern string, and there is
/// exactly one operator position — index 0. The predicate below reads only the
/// array's length, its elements' emptiness and the position of a reserved
/// token; it is finite, total, context-free and handler-free, which is what
/// makes it safe at a door rather than only in a validator (§6.1.4's
/// determinism guarantee).
///
/// Every clause applies to `callers` and `targets` **identically**. An
/// implementation that checks one field and infers the other passes almost
/// every rejection case in `acl_pattern_arity.json` and still ships the defect.
///
/// The order of the checks decides which fault an operator is told about
/// first, and runs from the most basic outward: empty array, then an empty
/// element, then a reserved token outside index 0, then the leading operator's
/// arity. It is deliberately **not** the same question as
/// [`pattern_never_matches`], which reasons about the match relation and is a
/// validator finding rather than a rejection.
///
/// **No message may contain the two characters `"; "`.** §6.1.1 rule 2 joins
/// several `handler_error` entries with that separator, and a reader recovers
/// the reported paths by splitting on it, so a message carrying it splits into
/// two paths that name nothing — a failure that reads as a short-circuit bug
/// rather than as the wording bug it is.
/// `no_fault_message_contains_the_handler_error_separator` pins this.
fn pattern_shape_fault(field: &str, patterns: &[String]) -> Option<String> {
    if patterns.is_empty() {
        return Some(format!(
            "'{field}' is empty. A pattern array MUST carry at least one operand \
             (PROTOCOL_SPEC §6.2.1): an array that can never match makes the rule inert, \
             and under 'default_effect: allow' an inert 'deny' rule permits the very call \
             it was written to block. Write ['*'] if the rule was meant to cover \
             everything, or delete the rule if it was meant to cover nothing."
        ));
    }
    if let Some(index) = patterns.iter().position(String::is_empty) {
        return Some(format!(
            "'{field}[{index}]' is the empty string, which matches no legal module ID. \
             Every element of a pattern array MUST be a non-empty pattern \
             (PROTOCOL_SPEC §6.2.1)."
        ));
    }
    if let Some(index) = patterns
        .iter()
        .skip(1)
        .position(|p| PATTERN_OPERATORS.contains(&p.as_str()))
    {
        let index = index + 1;
        return Some(format!(
            "'{field}[{index}]' is the reserved token '{}'. A pattern array is FLAT — the \
             operators do not nest and there is no precedence — so a reserved token is an \
             operator at index 0 and nothing anywhere else (PROTOCOL_SPEC §6.2.1). \
             ['$or', '$not', p] is not \"or of not\" and [p, '$not', q] is not \"p but not \
             q\" — no such form exists. Use first-match-wins with two rules (§6.3).",
            patterns[index]
        ));
    }
    match patterns[0].as_str() {
        "$or" if patterns.len() < 2 => Some(format!(
            "'{field}' is ['$or'] — an OR over nothing, which matches no module ID. \
             '$or' at index 0 MUST be followed by at least one pattern \
             (PROTOCOL_SPEC §6.2.1)."
        )),
        "$not" if patterns.len() != 2 => Some(format!(
            "'{field}' gives '$not' {} operands. '$not' at index 0 MUST be followed by \
             EXACTLY one pattern (PROTOCOL_SPEC §6.2.1). Through spec v1.30.0 a \
             multi-operand '$not' was implementation-defined — every SDK consulted the \
             first operand and dropped the rest — so an 'allow' rule excluding two \
             targets GRANTED the second one. There is no single-array form for \
             \"neither p nor q\": use first-match-wins with two rules (§6.3).",
            patterns.len() - 1
        )),
        _ => None,
    }
}

/// TIER 2 — why a **well-formed** `callers` / `targets` array still matches no
/// legal module ID, or `None` (PROTOCOL_SPEC §6.2.1, spec v1.31.0, #112).
///
/// Reached only from [`ACL::validate_rules`], and deliberately so: this is
/// diagnostics, not enforcement (§6.1.3). Such an array **MUST NOT** be
/// rejected and **MUST NOT** change any access decision — a `deny` rule reading
/// `["$not", "*"]` keeps falling through to the next rule exactly as it did
/// before v1.31.0, and only the finding is new. Wiring this into
/// [`ACL::precheck_pattern_faults`] would turn a well-formed rule into one that
/// denies every call, which is the failure the tier split exists to prevent.
///
/// The criterion — "matches no legal module ID for any input" — is normative
/// and the detected set is a **minimum**, not a closed one: it reasons about
/// the glob language, which §6.2 defines but does not freeze, so divergence
/// between implementations is expected and harmless here in a way it would
/// never be at a door.
///
/// The criterion is over the array **as a whole**, never over one element. The
/// array's own combinator is OR, so a flat or `$or` form is reported only when
/// *every* operand is unmatchable: `["@external"]` in `targets` is reported and
/// `["api.*", "@external"]` is not, because `api.*` still matches and the rule
/// still protects something. Implement the criterion, not the MUST-detect
/// list — the list's entries happen to be whole arrays and read as though the
/// token alone were the fault.
///
/// Assumes [`pattern_shape_fault`] already returned `None` for this array.
fn pattern_never_matches(field: &str, patterns: &[String]) -> Option<String> {
    let first = patterns.first()?.as_str();
    if first == "$not" {
        let operand = patterns.get(1)?;
        if is_universal_pattern(operand) {
            return Some(format!(
                "'{field}' negates '{operand}', which matches every module ID, so the rule \
                 fires for none of them (PROTOCOL_SPEC §6.2.1 tier 2). The rule is \
                 well-formed and still protects nothing."
            ));
        }
        return None;
    }
    // The remaining forms are an OR over their operands, explicit or implicit.
    let operands = if first == "$or" {
        &patterns[1..]
    } else {
        patterns
    };
    // PROTOCOL_SPEC §6.2.2 (v1.37.0): A08 has `*` as its ONLY metacharacter,
    // and §2.7 forbids `?` in a module ID, so a pattern carrying one matches
    // nothing and can never match anything. The author meant a wildcard — `?`
    // IS one in every other pattern surface the spec defines (A25, §9.2.3) —
    // and got a rule that is silently inert. Reported, never rejected:
    // promoting `?` in A08 would widen `allow` rules that are inert in every
    // deployed policy today, which is the one direction an authorization
    // matcher must not move without an operator's consent.
    if !operands.is_empty() && operands.iter().all(|p| p.contains('?')) {
        return Some(format!(
            "every '{field}' pattern contains '?', which is a LITERAL in ACL matching \
             (Algorithm A08 — '*' is the only metacharacter) and which §2.7 forbids in a \
             module ID, so this rule matches nothing and can never match anything. '?' is a \
             wildcard in every other pattern surface (A25, §9.2.3), which is where the \
             expectation comes from. The rule loads and changes no decision — it simply \
             never fires."
        ));
    }
    if field == "targets" && operands.iter().all(|p| p == EXTERNAL_CALLER) {
        return Some(format!(
            "'{field}' matches only '{EXTERNAL_CALLER}', which is the caller-side sentinel \
             §6.5 substitutes for an absent 'caller_id' and is never a module ID, so this \
             rule names no target (PROTOCOL_SPEC §6.2.1 tier 2). The same token in \
             'callers' is entirely legal and is what it exists for."
        ));
    }
    None
}

/// Whether a pattern matches every module ID because it is nothing but
/// wildcards.
///
/// `*` and `**` both reach [`match_pattern`] as "match anything", and so does
/// any longer run — the MUST-detect minimum is "any pattern consisting only of
/// wildcards", not the single literal `*`.
fn is_universal_pattern(pattern: &str) -> bool {
    !pattern.is_empty() && pattern.chars().all(|c| c == '*')
}

/// Wrap a pattern-field fault in §6.1.3 rule 3's **keyless structural** shape.
///
/// The path is the field name (`callers` / `targets`), the key is `None` —
/// there is no key to name, exactly as for a `conditions` value that is not a
/// mapping — and both resolvability flags are `false`, because the field as
/// written cannot be read on either evaluation path (§6.1.3 rule 2). Nothing
/// about a pattern array is registry-dependent, so the two flags could never
/// differ here.
fn pattern_rule_fault(field: &str, reason: String) -> RuleFault {
    RuleFault {
        path: field.to_string(),
        key: None,
        reason,
        sync_resolvable: false,
        async_resolvable: false,
    }
}

/// Whether a rule requires the calls it matches to be put to a human before
/// they run (PROTOCOL_SPEC §6.1.6).
///
/// Orthogonal to [`ACLRule::effect`], which answers a different question. An
/// ACL rule resolves **two** results, and folding them into one enumeration
/// makes a meaningless state ("denied *and* needs approval") representable
/// while forcing a real one ("allowed, but ask first") to be spelled as a kind
/// of denial:
///
/// * **Authorization** — may this caller reach this target at all? `effect`.
/// * **Approval requirement** — must *this particular call* be put to a human?
///   This type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirement {
    /// The call runs without an approval gate on the ACL's account. The
    /// module's own `annotations.requires_approval` and any `ExecutionPolicy`
    /// still apply — §6.9 row 3 composes the sources by **union**, and this
    /// value cancels neither.
    NotRequired,
    /// A call matching this rule must be put to a human before it runs.
    ///
    /// Only meaningful on an `allow` rule. `approval: required` on a `deny`
    /// rule is rejected at load with `ACLRuleError` (§6.1.6 rule 2).
    Required,
}

impl ApprovalRequirement {
    /// `true` only for [`ApprovalRequirement::Required`].
    #[must_use]
    pub fn is_required(self) -> bool {
        self == Self::Required
    }
}

impl Default for ApprovalRequirement {
    /// `not_required` — the meaning every rule written before spec v1.28.0
    /// already had (§6.1.6 rule 1).
    fn default() -> Self {
        Self::NotRequired
    }
}

/// Defines an access control rule.
///
/// # Construction policy (apcore#38)
///
/// This type is `#[non_exhaustive]`, which is the decision this attribute
/// records: **the spec may add a field to a rule in any minor release, and doing
/// so must not break a downstream crate.** §6.1.5 closed the rule key set in
/// spec v1.27.0 and §6.1.6 added `approval` in v1.28.0; more will follow, and
/// the attribute is what makes the *next* one additive rather than a compile
/// break for everyone who builds a rule in code.
///
/// The cost is paid once, here: `#[non_exhaustive]` removes struct-expression
/// construction for every crate except this one, `..Default::default()`
/// included (`api-surface-conventions.md` §9.1). Downstream builds a rule
/// through [`ACLRule::new`] and assigns the optional fields afterwards, which
/// is the same shape [`AccessDecision::new`], [`RuleValidationFinding::new`]
/// and [`Change`](crate::Change) already use, and the form §9.3 names as the
/// only one that compiles across a package boundary.
///
/// The three neighbours in this file — [`AccessDecision`], [`AuditEntry`] and
/// [`RuleValidationFinding`] — carry the attribute for the same reason
/// (apcore#24). `ACLRule` was the one spec-derived type it was never applied
/// to, which is what made v1.28.0's `approval` a hard break for `apexe` and
/// `apcore-mcp-rust`; that is now closed rather than left as an oversight.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ACLRule {
    #[serde(default)]
    pub callers: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    pub effect: String,
    /// Whether a call matching this rule must be put to a human before it runs
    /// (PROTOCOL_SPEC §6.1.6, spec v1.28.0, apcore#108).
    ///
    /// `None` is the absent key and means [`ApprovalRequirement::NotRequired`],
    /// so every rule written before v1.28.0 keeps its meaning exactly. The
    /// absent and the explicitly-written forms are kept distinguishable rather
    /// than normalised, so `rules()` reports the document as authored.
    ///
    /// Adding this field was only safe once §6.1.5 closed the rule key set
    /// (v1.27.0): an SDK that still dropped unknown keys would read a
    /// `deny`-with-`approval` rule as a bare rule and act on half of what the
    /// operator wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalRequirement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<serde_json::Value>,
}

impl ACLRule {
    /// Build a rule from its three required fields
    /// (`api-surface-conventions.md` §9.2 rule 4).
    ///
    /// The type is `#[non_exhaustive]`, so this is the only construction path
    /// available to a downstream crate. `approval`, `description` and
    /// `conditions` are left unset and assigned on the returned value — the
    /// form §9.3 names as the one that compiles from outside this crate.
    ///
    /// `effect` is **not** validated here, and neither is the shape of
    /// `callers` / `targets`. Both are enforced at the three doors that accept
    /// a rule — [`ACL::load`], [`ACL::try_new`] and [`ACL::try_add_rule`]
    /// (apcore#111 and apcore#112) — so a rule carrying an effect outside the
    /// closed set, or a pattern array outside §6.2.1's table, cannot reach a
    /// live ACL by any route, and a rule under construction is not yet a
    /// governance decision. The refusal names the rule's index, which a rule
    /// under construction does not have; §6.2.1 forbids inventing one.
    ///
    /// ```
    /// use apcore::ACLRule;
    ///
    /// let mut rule = ACLRule::new(
    ///     vec!["agent.*".to_string()],
    ///     vec!["data.export".to_string()],
    ///     "allow",
    /// );
    /// rule.description = Some("Data-admin agents may export.".to_string());
    /// rule.conditions = Some(serde_json::json!({ "roles": ["data_admin"] }));
    ///
    /// assert_eq!(rule.effect, "allow");
    /// assert!(!rule.approval_required());
    /// ```
    #[must_use]
    pub fn new(callers: Vec<String>, targets: Vec<String>, effect: impl Into<String>) -> Self {
        Self {
            callers,
            targets,
            effect: effect.into(),
            approval: None,
            description: None,
            conditions: None,
        }
    }

    /// Whether this rule requires a human decision for the calls it matches
    /// (PROTOCOL_SPEC §6.1.6).
    ///
    /// Always `false` for a `deny` rule. All three entry points that accept a
    /// rule — [`ACL::load`], [`ACL::try_new`] and [`ACL::try_add_rule`] —
    /// refuse the combination, so it is unrepresentable in a live ACL. The
    /// guard is restated here because a governance requirement read off a rule
    /// that refuses the call would be a state with no meaning, and because
    /// `rules` is `pub`-readable and a future entry point must not be able to
    /// reintroduce it.
    #[must_use]
    pub fn approval_required(&self) -> bool {
        self.effect == "allow" && self.approval.is_some_and(ApprovalRequirement::is_required)
    }
}

/// The JSON type of one argument, as carried by a [`GovernanceProjection`].
///
/// A type name, never a value — see the type-level note on
/// [`GovernanceProjection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonType {
    /// `null`
    Null,
    /// `true` / `false`
    Boolean,
    /// Any JSON number, integral or not.
    Number,
    /// A JSON string.
    String,
    /// A JSON array.
    Array,
    /// A JSON object.
    Object,
}

impl JsonType {
    /// The canonical lowercase name, matching `ExecutionPolicy`'s call-site
    /// shape vocabulary (§7.9.6 rule 3) and JSON Schema's `type` keyword.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
        }
    }

    fn of(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(_) => Self::Boolean,
            serde_json::Value::Number(_) => Self::Number,
            serde_json::Value::String(_) => Self::String,
            serde_json::Value::Array(_) => Self::Array,
            serde_json::Value::Object(_) => Self::Object,
        }
    }
}

/// The **governance projection** of a call's arguments — the only view of them
/// the ACL ever sees (PROTOCOL_SPEC §6.1.8).
///
/// It carries the argument **key set** and each key's JSON type, and it
/// **cannot carry a value**: there is no field for one. A projection that
/// structurally cannot hold a value cannot leak one, whatever a future
/// predicate does with it — which is the whole reason the type exists rather
/// than a `&serde_json::Value` being passed down.
///
/// # Why not `context.redacted_inputs`
///
/// §6.1.8 rule 3 is a MUST NOT, for two independent reasons. Its documented
/// contract is safe *logging*, not "input to a security decision", and one
/// field serving both will eventually break one of them in a change made for
/// the other. And it is not even safe here: redaction is driven by
/// `x-sensitive` markers in the module's input schema, so a module with no
/// input schema gets no field redaction at all and `redacted_inputs` is a raw
/// copy of the arguments.
///
/// # `_approval_token` is not an argument
///
/// The framework-owned Phase B resume token (§7.4) is excluded, on the same
/// grounds §7.9.6 rule 5 excludes it from policy resolution: it is a
/// protocol-level key, not caller input. Excluding it also keeps Step 4's
/// verdict identical across the approval suspend/resume boundary, which §7.4
/// re-enters from Step 1 with the token present in `arguments`.
///
/// # It cannot be forged
///
/// §6.1.8 rule 3: the projection **MUST** be computed by the framework and
/// **MUST NOT** be accepted from caller-supplied input. A caller that could
/// supply its own would satisfy `has_none_of` for a call whose arguments say
/// otherwise, turning the condition into a caller-controlled switch.
///
/// apcore-rust satisfies this **structurally**, not with a runtime check.
/// This type derives no serde traits, so there is no way to build one from a
/// wire payload at all; it is carried on `PipelineContext`, which is
/// framework-internal and never deserialized; and [`Context`] — the object
/// that *is* deserialized from the wire — does not carry it, which is one of
/// the reasons the projection did not go there. `projection_forgery_tests`
/// below pins all three, so a later change that adds `Deserialize` or moves
/// the field onto `Context` fails a test rather than opening the hole quietly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GovernanceProjection {
    /// Key -> JSON type. `BTreeMap` so iteration order is the document order a
    /// diagnostic reader expects, and identical across runs.
    keys: std::collections::BTreeMap<String, JsonType>,
}

impl GovernanceProjection {
    /// Project a call's arguments (PROTOCOL_SPEC §6.1.8 rule 2).
    ///
    /// Top-level keys only: §6.1.7's three predicates are defined over the
    /// call's argument keys, and descending into nested objects would widen
    /// the projection without widening what can be asked of it.
    ///
    /// Arguments that are not a JSON object project to the empty key set — the
    /// ACL check is Step 4 and input validation is Step 7, so the value here is
    /// whatever the caller passed and may be a scalar, an array, or `null`.
    #[must_use]
    pub fn from_arguments(arguments: &serde_json::Value) -> Self {
        let keys = arguments
            .as_object()
            .map_or_else(std::collections::BTreeMap::new, |obj| {
                obj.iter()
                    .filter(|(key, _)| key.as_str() != crate::policy::APPROVAL_TOKEN_KEY)
                    .map(|(key, value)| (key.clone(), JsonType::of(value)))
                    .collect()
            });
        Self { keys }
    }

    /// Whether the call carried an argument named `key`.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.keys.contains_key(key)
    }

    /// The projected key set, in ascending lexicographic order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.keys.keys().map(String::as_str)
    }

    /// The JSON type of one argument, or `None` when the call did not carry it.
    #[must_use]
    pub fn type_of(&self, key: &str) -> Option<JsonType> {
        self.keys.get(key).copied()
    }

    /// Number of projected keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the call carried no arguments at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// The structured result of an ACL check (PROTOCOL_SPEC §6.8.1).
///
/// [`ACL::check`] returns a `bool`, which can carry authorization but not the
/// second axis of §6.1.6. This type carries both, and is what the Executor
/// consults — the boolean entry points are kept for existing callers and
/// **fail closed** on an approval requirement.
///
/// Marked `#[non_exhaustive]`: it is a spec-derived record that a future
/// revision may widen, and the attribute is the mechanism by which that stays
/// non-breaking. Build one with [`AccessDecision::new`]; the fields are public
/// to read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccessDecision {
    /// `"allow"` or `"deny"` — the authorization result, with exactly the
    /// semantics [`ACL::check`]'s boolean has always had.
    pub access: String,
    /// Whether **this call** must be put to a human before it runs.
    ///
    /// Only ever `true` alongside `access == "allow"`: "denied and needs
    /// approval" is not a state that means anything (§6.1.6).
    ///
    /// The requirement does not have to come from the rule
    /// [`Self::matched_rule_index`] names. An `allow` rule carrying
    /// `approval: required` whose conditions turned out UNEVALUABLE does not
    /// grant, but leaves its requirement **pending**, and whatever grants next
    /// inherits it (§6.1.1 rule 5, spec v1.29.0) — including
    /// `default_effect: allow`, where `matched_rule_index` is `None`.
    pub approval_required: bool,
    /// Index of the rule that decided, in definition order, or `None` when the
    /// decision came from `default_effect` or from an empty rule list.
    ///
    /// It names the rule that decided **`access`**, which since spec v1.29.0
    /// need not be the rule the approval requirement came from: `None`
    /// alongside `approval_required: true` is a legal combination (§6.9 row 2).
    pub matched_rule_index: Option<usize>,
    /// Which branch of §6.3 produced the decision: `"rule_match"`,
    /// `"default_effect"` or `"no_rules"`.
    pub reason: String,
}

impl AccessDecision {
    /// Build a decision. Provided because `#[non_exhaustive]` removes
    /// struct-literal construction for downstream crates
    /// (`api-surface-conventions.md` §9.2 rule 2).
    #[must_use]
    pub fn new(
        access: impl Into<String>,
        approval_required: bool,
        matched_rule_index: Option<usize>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            access: access.into(),
            approval_required,
            matched_rule_index,
            reason: reason.into(),
        }
    }

    /// Whether the ACL authorized the call, **ignoring** the approval
    /// requirement.
    ///
    /// This is the authorization axis alone. A caller that is not the approval
    /// gate wants [`ACL::check`], which folds the two axes together and fails
    /// closed.
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        self.access == "allow"
    }
}

/// Audit log entry produced by ACL checks (PROTOCOL_SPEC §6.3.1).
///
/// Marked `#[non_exhaustive]` in the same change that added
/// [`Self::approval_required`] (spec v1.28.0): the field is a breaking
/// addition for downstream struct-literal construction either way, and the
/// attribute is what makes the *next* spec-driven field non-breaking. Build
/// one with `AuditEntry::default()` and mutate, per the migration note the
/// crate uses for every other spec-derived record (apcore#24).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuditEntry {
    pub timestamp: String,
    pub caller_id: String,
    pub target_id: String,
    pub decision: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_type: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Error message from a condition handler that panicked or returned an error
    /// during evaluation, if any. Cross-language parity with apcore-python
    /// AuditEntry.handler_error and apcore-typescript AuditEntry.handlerError
    /// (sync finding A-D-024).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler_error: Option<String>,
    /// Whether the **decision** required this call to be put to a human
    /// (PROTOCOL_SPEC §6.1.6, §6.3.1) — the final value, matching the
    /// [`AccessDecision`] emitted alongside it.
    ///
    /// Usually the matched rule's own requirement, but since spec v1.29.0 it
    /// may also be one raised by an unevaluable `allow` rule that did not
    /// itself match (§6.1.1 rule 5), including on the `default_effect: allow`
    /// path where `matched_rule_index` is `None`. `false` on any `deny`.
    ///
    /// A field **beside** `decision` rather than a third `decision` value:
    /// `decision` is a string downstream consumers parse, and a third value
    /// would break every existing parser (§6.9 row 7). `#[serde(default)]` so
    /// an audit record written before v1.28.0 still deserializes.
    #[serde(default)]
    pub approval_required: bool,
}

/// Type alias for the audit logger callback.
pub type AuditLoggerFn = dyn Fn(&AuditEntry) + Send + Sync;

// ---------------------------------------------------------------------------
// Audit delivery (PROTOCOL_SPEC §6.3.2, apcore#118 decision D-66)
// ---------------------------------------------------------------------------

/// The three settings of an ACL file's `audit:` block.
pub const AUDIT_FIELDS: [&str; 3] = ["enabled", "include_denied", "log_level"];

/// The stable name the default sink emits under (§10.3's event table).
pub const AUDIT_EVENT_NAME: &str = "apcore.acl.audit";

/// An ACL file's `audit:` block, as declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditConfig {
    pub enabled: bool,
    pub include_denied: bool,
    pub log_level: String,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_denied: true,
            log_level: "info".to_string(),
        }
    }
}

/// The ONE effective sink for an ACL, per §6.3.2 requirement 1.
///
/// Never two. A callback supplied through [`ACL::set_audit_logger`] or the
/// constructor receives every entry and is not narrowed, levelled or silenced
/// by the `audit:` block: an API argument beats configuration, and the
/// alternative lets a file silently truncate a compliance sink a developer
/// installed deliberately. The block configures the **default sink** and
/// nothing else.
///
/// A sink also owns its failure state. Requirement 5 suppresses diagnostics
/// after the first, scoped to one ACL instance **and one effective sink
/// configuration** — replacing the sink or reloading a configuration that
/// changes it builds a new `AuditSink`, so a new failure is never hidden behind
/// an old one.
pub(crate) struct AuditSink {
    callback: Option<Arc<AuditLoggerFn>>,
    /// `None` means the document declared no `audit:` key. Requirement 2:
    /// DECLARATION activates the default sink, never the default value —
    /// `enabled` defaults to true, so reading the merged view would switch a
    /// log record per check on for every ACL file in existence.
    config: Option<AuditConfig>,
    failed: AtomicBool,
}

impl std::fmt::Debug for AuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditSink")
            .field("callback", &self.callback.as_ref().map(|_| "..."))
            .field("config", &self.config)
            .field("failed", &self.failed.load(Ordering::Relaxed))
            .finish()
    }
}

impl Clone for AuditSink {
    fn clone(&self) -> Self {
        // A clone is a NEW sink configuration for requirement 5's purposes: the
        // clone delivers independently, so carrying the original's failure flag
        // across would suppress the clone's first real failure.
        Self::new(self.callback.clone(), self.config.clone())
    }
}

impl AuditSink {
    pub(crate) fn new(callback: Option<Arc<AuditLoggerFn>>, config: Option<AuditConfig>) -> Self {
        Self {
            callback,
            config,
            failed: AtomicBool::new(false),
        }
    }

    fn is_active(&self) -> bool {
        if self.callback.is_some() {
            return true;
        }
        self.config.as_ref().is_some_and(|c| c.enabled)
    }

    /// Deliver one entry. NEVER propagates (§6.3.2 requirement 3).
    fn deliver(&self, entry: &AuditEntry) {
        if let Some(ref callback) = self.callback {
            // Requirement 3, bounded to RECOVERABLE failures. `AuditLoggerFn` is
            // `Fn(&AuditEntry)` with no error channel, so an unwinding panic is
            // the only failure this can contain; a build with `panic = "abort"`
            // cannot be, and the specification says so rather than promising
            // what the language cannot deliver.
            //
            // The default panic hook still prints its own message before this
            // returns. That is the runtime's, not ours: requirement 5 governs
            // the diagnostic THIS sink emits.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                callback(entry);
            }));
            if outcome.is_err() {
                self.report_failure();
            }
            return;
        }
        let Some(ref config) = self.config else {
            return;
        };
        if !config.enabled {
            return;
        }
        if entry.decision == "deny" && !config.include_denied {
            // Requirement 6 — the load-time notice for this is emitted once, at
            // load; withholding here is silent by design.
            return;
        }
        Self::emit_default(entry, config);
    }

    /// §6.3.2 requirement 2 — the default sink.
    ///
    /// All thirteen §6.3.1 fields go out as STRUCTURED data under their
    /// `snake_case` wire names, not interpolated into the message. Without
    /// that, one specification yields three different "structured records"
    /// across the SDKs and nothing downstream consumes all three.
    #[allow(clippy::cognitive_complexity)] // five level arms, one body each
    fn emit_default(entry: &AuditEntry, config: &AuditConfig) {
        macro_rules! emit {
            ($level:expr) => {
                tracing::event!(
                    $level,
                    timestamp = %entry.timestamp,
                    caller_id = %entry.caller_id,
                    target_id = %entry.target_id,
                    decision = %entry.decision,
                    reason = %entry.reason,
                    matched_rule = ?entry.matched_rule,
                    matched_rule_index = ?entry.matched_rule_index,
                    identity_type = ?entry.identity_type,
                    roles = ?entry.roles,
                    call_depth = ?entry.call_depth,
                    trace_id = ?entry.trace_id,
                    handler_error = ?entry.handler_error,
                    approval_required = entry.approval_required,
                    // The literal, because `tracing` requires one for the
                    // message. `audit_event_name_matches_the_constant` pins it
                    // against AUDIT_EVENT_NAME so the two cannot drift.
                    "apcore.acl.audit"
                )
            };
        }
        match config.log_level.as_str() {
            "trace" | "debug" => emit!(tracing::Level::DEBUG),
            "warn" => emit!(tracing::Level::WARN),
            "error" => emit!(tracing::Level::ERROR),
            _ => emit!(tracing::Level::INFO),
        }
    }

    fn report_failure(&self) {
        if self.failed.swap(true, Ordering::Relaxed) {
            return;
        }
        tracing::warn!(
            "ACL audit delivery failed (the callback panicked). The access decision is \
             unaffected (PROTOCOL_SPEC §6.3.2 requirement 3) — auditing is a side channel \
             and does not hold a veto over access. Further failures from this sink are not \
             reported; replacing the sink or reloading the ACL starts a new report."
        );
    }
}

/// §6.3.2 requirement 1 — name EVERY field the callback overrides.
///
/// Not only the most visible one: an operator who set `include_denied` and
/// `log_level` and hears about one of them has been told the smaller half of
/// what happened.
pub(crate) fn warn_audit_block_overridden(
    callback: Option<&Arc<AuditLoggerFn>>,
    config: Option<&AuditConfig>,
) {
    if callback.is_none() || config.is_none() {
        return;
    }
    tracing::warn!(
        fields = %AUDIT_FIELDS
            .iter()
            .map(|f| format!("audit.{f}"))
            .collect::<Vec<_>>()
            .join(", "),
        "An audit logger was supplied to this ACL, so it is the effective sink and the ACL \
         file's 'audit:' block does not apply. The callback receives EVERY entry, allow and \
         deny alike. The listed declared settings have no effect (PROTOCOL_SPEC §6.3.2 \
         requirement 1)."
    );
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
pub(crate) fn parse_audit_block(
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

/// One rule that fails PROTOCOL_SPEC §6.1.4's structural and registry
/// precheck.
///
/// Produced by [`ACL::validate_rules`] (§6.1.2 rule 3, §6.1.3). A fault is
/// either a condition key with no resolvable handler, a compound operator
/// whose operand has the wrong shape (`$or` that is not a list, `$not` that is
/// not an object), or a `conditions` value that is not a mapping at all.
///
/// `sync_resolvable` and `async_resolvable` are reported separately and MUST
/// NOT be collapsed into a single boolean: `async_check()` consults the async
/// registry and falls back to the sync one, while `check()` consults only the
/// sync registry, so a key registered **only** as an async handler is a working
/// condition on one path and an unevaluable one on the other. They are named
/// `*_resolvable` rather than `*_registered` because they mean "resolvable on
/// that evaluation path", not "present in that registry" — `async_resolvable`
/// is the union of both, and would otherwise read as false for every built-in
/// leaf handler (§6.1.3 rule 2).
///
/// Marked `#[non_exhaustive]` so a future spec revision can add a field without
/// a major version bump. That works by removing struct-literal construction
/// from every crate but this one — `..Default::default()` included, since it is
/// itself a struct expression (`error[E0639]`). Build one with
/// [`RuleValidationFinding::new`]; the fields are public to read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuleValidationFinding {
    /// Index of the offending rule in definition order.
    pub rule_index: usize,
    /// Where the fault sits in the rule (§6.1.4 condition paths): `roles`,
    /// `$or[1].mispelled`, `$not.k`, or `$` for a non-mapping `conditions`.
    ///
    /// Findings order by this, not by key, because a nested `$or` may carry the
    /// same key at several positions and ordering by key alone is undefined.
    pub condition_path: String,
    /// The offending key, where one exists. `None` for a `conditions` value
    /// that is not a mapping — there is no key to name.
    pub condition_key: Option<String>,
    /// The rule's effect. A finding on a `deny` rule is the consequential one:
    /// per §6.1.1 that rule now denies every call it matches.
    pub effect: String,
    /// Whether the condition resolves for `check()`.
    pub sync_resolvable: bool,
    /// Whether it resolves for `async_check()`.
    pub async_resolvable: bool,
}

impl RuleValidationFinding {
    /// Build a finding. Provided because `#[non_exhaustive]` removes
    /// struct-literal construction for downstream crates
    /// (`api-surface-conventions.md` §9.2 rule 2).
    #[must_use]
    pub fn new(
        rule_index: usize,
        condition_path: impl Into<String>,
        condition_key: Option<String>,
        effect: impl Into<String>,
        sync_resolvable: bool,
        async_resolvable: bool,
    ) -> Self {
        Self {
            rule_index,
            condition_path: condition_path.into(),
            condition_key,
            effect: effect.into(),
            sync_resolvable,
            async_resolvable,
        }
    }
}

/// Outcome of matching one ACL rule against a call (PROTOCOL_SPEC §6.3).
///
/// Three variants rather than a `bool`, because a rule whose conditions could
/// not be evaluated is neither "matched" nor "did not match" until the rule's
/// `effect` is consulted (§6.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleMatch {
    /// Patterns did not match, or the conditions were UNSATISFIED.
    NoMatch,
    /// Patterns matched and the conditions (if any) were SATISFIED.
    Match,
    /// Patterns matched but the conditions were UNEVALUABLE (§6.1.1).
    Unevaluable,
}

/// What PROTOCOL_SPEC §6.1.1 says to do with a rule whose conditions were
/// UNEVALUABLE.
///
/// A `bool` was enough while §6.1.1 said one thing about such a rule ("a `deny`
/// rule takes effect, an `allow` rule does not grant"). Rule 5 (spec v1.29.0,
/// apcore#109) added a second thing an `allow` rule can leave behind on its way
/// out, so the resolution now carries both answers rather than returning one
/// and leaving the caller to re-derive the other — sync and async both consume
/// this, and a value re-derived at two call sites is a value that can drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnevaluableResolution {
    /// The rule **takes effect** — a `deny` rule denies the call (§6.1.1 rule
    /// 1). Scanning stops here.
    TakesEffect,
    /// The rule **steps aside**: an `allow` rule MUST NOT grant on a condition
    /// nobody could answer, so scanning continues with the next rule.
    ///
    /// `pending_approval` is `true` when the rule carried `approval: required`.
    /// The rule does not grant, but the requirement it carried is **pending**,
    /// not discarded (§6.1.1 rule 5).
    StepsAside { pending_approval: bool },
}

/// Access control list manager.
///
/// Thread safety: Rust's borrow checker enforces exclusive access for mutation
/// (&mut self for `add_rule/remove_rule/reload`). The `check()` method takes &self
/// and is safe for concurrent reads. No internal lock is needed.
pub struct ACL {
    rules: Vec<ACLRule>,
    default_effect: String,
    yaml_path: Option<String>,
    audit_logger: Option<Arc<AuditLoggerFn>>,
    /// The ACL file's `audit:` block, as declared (§6.3.2 requirement 2).
    audit_config: Option<AuditConfig>,
    /// §6.3.2 requirement 1: one effective sink, never two.
    audit_sink: AuditSink,
    /// D-88: dedupe for the §6.5 "conditions present but no context" warning,
    /// keyed by `(rule index, effect)` exactly as apcore-python and
    /// apcore-typescript key it.
    ///
    /// `check()` takes `&self`, so the set needs interior mutability. Every
    /// operation that inserts, removes or reorders rules MUST clear it — the
    /// key is an INDEX, and `add_rule` inserts at 0 and shifts every existing
    /// rule, so a retained entry would suppress the warning for a different
    /// rule than the one it was recorded for, and specifically for the rule the
    /// operator just added.
    warned_missing_context: parking_lot::Mutex<HashSet<(usize, String)>>,
}

impl std::fmt::Debug for ACL {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ACL")
            .field("rules", &self.rules)
            .field("default_effect", &self.default_effect)
            .field("yaml_path", &self.yaml_path)
            .field("audit_logger", &self.audit_logger.as_ref().map(|_| "..."))
            .field("audit_config", &self.audit_config)
            .field("audit_sink", &self.audit_sink)
            .finish_non_exhaustive()
    }
}

impl Clone for ACL {
    fn clone(&self) -> Self {
        Self {
            rules: self.rules.clone(),
            default_effect: self.default_effect.clone(),
            yaml_path: self.yaml_path.clone(),
            audit_logger: self.audit_logger.clone(),
            audit_config: self.audit_config.clone(),
            audit_sink: self.audit_sink.clone(),
            // A clone starts with a clean dedupe set. Carrying the markers
            // across would suppress a warning on the new instance for a rule
            // it has never evaluated; starting fresh can only warn more, never
            // less.
            warned_missing_context: parking_lot::Mutex::new(HashSet::new()),
        }
    }
}

impl ACL {
    /// Create a new ACL with the given rules, default effect, and optional audit logger.
    ///
    /// # Errors
    ///
    /// Returns `ModuleError` with `ErrorCode::ACLRuleError` if `default_effect`
    /// is not `"allow"` or `"deny"` (matching the constructor validation in
    /// apcore-typescript — sync finding A-D-025), or if any rule fails
    /// [`ACL::validate_rule`]: an `effect` outside the same closed set
    /// (§6.1.5, spec v1.30.0, #111), `approval: required` on a `deny` effect
    /// (§6.1.6 rule 2), or a `callers` / `targets` array whose shape is outside
    /// §6.2.1's table (spec v1.31.0, #112).
    pub fn try_new(
        rules: Vec<ACLRule>,
        default_effect: impl Into<String>,
        audit_logger: Option<Arc<AuditLoggerFn>>,
    ) -> Result<Self, ModuleError> {
        let default_effect = default_effect.into();
        if !EFFECTS.contains(&default_effect.as_str()) {
            return Err(ModuleError::new(
                ErrorCode::ACLRuleError,
                format!("Invalid default_effect '{default_effect}': must be 'allow' or 'deny'"),
            ));
        }
        for (index, rule) in rules.iter().enumerate() {
            Self::validate_rule(index, rule)?;
        }
        Ok(Self::new_unchecked(rules, default_effect, audit_logger))
    }

    /// Create a new ACL with the given rules, default effect, and optional
    /// audit logger.
    ///
    /// **Validates** and panics on invalid input, which matches apcore-python
    /// and apcore-typescript constructor behaviour (both throw) — sync finding
    /// A-D-302.
    ///
    /// For fallible construction (e.g., when `default_effect` or a rule
    /// originates from user input or a YAML file), prefer [`ACL::try_new`].
    ///
    /// # Panics
    ///
    /// Panics on everything [`ACL::try_new`] returns an error for: a
    /// `default_effect` outside `allow` / `deny`, a rule `effect` outside the
    /// same closed set (§6.1.5), `approval: required` on a `deny` rule
    /// (§6.1.6 rule 2), or a `callers` / `targets` array whose shape is outside
    /// §6.2.1's table (spec v1.31.0, #112). Construction is one of the three
    /// doors §6.1.6 rule 3 closes, so an invalid rule cannot enter here merely
    /// because the signature is infallible.
    pub fn new(
        rules: Vec<ACLRule>,
        default_effect: impl Into<String>,
        audit_logger: Option<Arc<AuditLoggerFn>>,
    ) -> Self {
        // Carry `try_new`'s own message into the panic rather than replacing
        // it: §6.1.5 wants the refusal to name the offending value, and an
        // `.expect()` string cannot — it named `default_effect` for every
        // rejection, including the rule-level ones that have nothing to do
        // with that field.
        Self::try_new(rules, default_effect, audit_logger).unwrap_or_else(|e| {
            panic!("{} — use ACL::try_new for fallible construction", e.message)
        })
    }

    fn new_unchecked(
        rules: Vec<ACLRule>,
        default_effect: impl Into<String>,
        audit_logger: Option<Arc<AuditLoggerFn>>,
    ) -> Self {
        // Auto-register built-in condition handlers ($or, $not, identity_types,
        // roles, max_call_depth) — matches apcore-python and apcore-typescript
        // module-load auto-registration. Idempotent via std::sync::Once
        // (sync finding A-D-027), and each built-in is seeded only where the
        // key is unclaimed, so a handler the deployment registered before the
        // first ACL is not replaced by the permissive default (A-D-010).
        Self::init_builtin_handlers();
        // PROTOCOL_SPEC §6.1.2: warn (never fail) for every rule that fails
        // §6.1.4's precheck on the sync path. Emitted AFTER
        // `init_builtin_handlers` so the built-ins are never reported. Rule 4
        // makes direct construction an entry point that MUST be covered, not
        // just file loading — `try_new`, `new`, `load`, `discover` and
        // `reload` all funnel through here.
        Self::warn_rule_faults(&rules, 0);
        Self {
            rules,
            default_effect: default_effect.into(),
            yaml_path: None,
            audit_sink: AuditSink::new(audit_logger.clone(), None),
            audit_config: None,
            audit_logger,
            warned_missing_context: parking_lot::Mutex::new(HashSet::new()),
        }
    }

    /// Reject a rule that cannot mean anything, whichever door it arrived at.
    ///
    /// The single check behind `load`, `try_new` and `try_add_rule`, so the
    /// three entry points PROTOCOL_SPEC §6.1.6 rule 3 names cannot drift apart
    /// — which they had: the `effect` enum was checked in `try_new`'s body and
    /// nowhere else, so a rule `ACL::load` refused was accepted by
    /// `try_add_rule` (spec v1.30.0, #111).
    ///
    /// `index` is the position the rule occupies in the rule list: its index in
    /// the loaded document, or `0` for a runtime insertion, which is where
    /// `add_rule` puts it.
    ///
    /// # Errors
    ///
    /// * `effect` outside [`EFFECTS`] (§6.1.5). The value set is closed and an
    ///   unrecognised value MUST NOT be resolved to a decision — leaving it to
    ///   the rule loop would hand `AccessDecision::access` a string no consumer
    ///   knows, having silently changed what the operator wrote.
    /// * `approval: required` on a `deny` effect (§6.1.6 rule 2). "Denied AND
    ///   needs approval" is not a state that means anything, and silently
    ///   ignoring one half of a governance rule is the failure mode §6.1.5 was
    ///   written to end.
    /// * A `callers` or `targets` array whose **shape** is outside §6.2.1's
    ///   table (spec v1.31.0, #112) — empty, an empty element, `$or` with no
    ///   operand, `$not` with none or more than one, or a reserved token
    ///   outside index 0. `schemas/acl-config.schema.json` has declared
    ///   `minItems: 1` and `minLength: 1` on both fields since the file
    ///   existed and no door enforced either, because no implementation
    ///   validates an ACL file against the schema at load time. An array that
    ///   can never match leaves the rule inert, and an inert `deny` rule under
    ///   `default_effect: allow` permits the call it was written to block.
    ///
    /// All three name the rule index and the offending field or value, because
    /// an operator reading the refusal needs to find the rule in their file.
    ///
    /// # The order of the three checks is normative
    ///
    /// §6.2.1 point 2 (spec v1.31.0, #112) fixes it as **`effect` ->
    /// `approval` -> `callers` / `targets`**: a rule bad on more than one axis
    /// is refused for the first of these it fails, so the same rule produces
    /// the same error in every implementation. `callers` precedes `targets`
    /// within the pattern axis, which is the whole of that axis here —
    /// §6.1.4.1's *type* fault is unrepresentable in a `Vec<String>`, leaving
    /// only the shape closure.
    ///
    /// §6.2.1 states this ordering **for the first time**. §6.1.6 rule 2 only
    /// *implies* `effect` before `approval` — judging "`deny` plus
    /// `approval: required`" requires knowing the effect — and states no
    /// order, so it is not the citation for this and was never a second
    /// scheme to extend.
    ///
    /// This SDK checked the **patterns first** until v1.31.0 landed, so
    /// `{callers: [], targets: [], effect: "Allow"}` was refused here for its
    /// patterns and in apcore-python for its `effect`. Both were conformant
    /// while nothing said otherwise, which is what the ordering ends. Pinned by
    /// `validation_order_is_effect_then_approval_then_patterns` in
    /// `tests/test_acl_pattern_arity.rs`.
    ///
    /// **Rule index dominates all three.** A rule set with more than one bad
    /// rule is refused for the lowest-indexed bad rule, and no implementation
    /// may sweep one axis across every rule before looking at the next. Every
    /// door here loops rule by rule through this one function, so the property
    /// holds by construction at `try_new` and `try_add_rule`; [`ACL::load`]
    /// carries a fourth axis of its own — #107's rule-key closure, which needs
    /// the raw document — and interleaves it into the same per-rule loop for
    /// exactly this reason. Pinned by
    /// `the_lowest_indexed_bad_rule_is_the_one_reported`.
    ///
    /// Reporting order is a different question and is unchanged:
    /// [`ACL::validate_rules`] still orders findings by rule index then
    /// lexicographically by path (§6.1.2 rule 3), so a pattern fault
    /// interleaves with condition faults rather than being grouped ahead of
    /// them. This ordering governs which single refusal a *rejecting* door
    /// raises, not what a *reporting* validator returns.
    fn validate_rule(index: usize, rule: &ACLRule) -> Result<(), ModuleError> {
        if !EFFECTS.contains(&rule.effect.as_str()) {
            return Err(ModuleError::new(
                ErrorCode::ACLRuleError,
                format!(
                    "Rule {index} has invalid effect '{}', must be 'allow' or 'deny'",
                    rule.effect
                ),
            ));
        }
        if rule.effect == "deny" && rule.approval.is_some_and(ApprovalRequirement::is_required) {
            return Err(ModuleError::new(
                ErrorCode::ACLRuleError,
                format!(
                    "Rule {index} carries 'approval: required' on a 'deny' rule. \
                     Authorization and approval are two independent results \
                     (PROTOCOL_SPEC §6.1.6): a refusal is not a question, so the \
                     combination has no meaning. Use 'effect: allow' with \
                     'approval: required' to ask a human, or drop the 'approval' key \
                     to refuse outright."
                ),
            ));
        }
        for (field, patterns) in [("callers", &rule.callers), ("targets", &rule.targets)] {
            if let Some(reason) = pattern_shape_fault(field, patterns) {
                return Err(ModuleError::new(
                    ErrorCode::ACLRuleError,
                    format!("Rule {index}: {reason}"),
                ));
            }
        }
        Ok(())
    }

    /// Emit PROTOCOL_SPEC §6.1.2 rule 2's load-time warning for every rule that
    /// fails §6.1.4's precheck on the sync path.
    ///
    /// `index_offset` is added to the position within `rules`, so `add_rule`
    /// can report the index the rule will actually occupy.
    ///
    /// Loading MUST NOT fail here: `register_condition()` writes to a runtime,
    /// process-wide registry, and `acl.root` discovery commonly runs during
    /// framework bootstrap ahead of application code, so failing would reject
    /// valid configurations on ordering alone. [`ACL::validate_rules`] is the
    /// deterministic check to run once registration is complete.
    fn warn_rule_faults(rules: &[ACLRule], index_offset: usize) {
        // PROTOCOL_SPEC §6.2.2 clause 1 — the load-time half of the `?`
        // diagnostic, on the §6.1.2 precedent: warn, never fail, and change no
        // decision. `validate_rules()` reports the same condition at deploy.
        for (i, rule) in rules.iter().enumerate() {
            for (field, patterns) in [("callers", &rule.callers), ("targets", &rule.targets)] {
                for pattern in patterns.iter().filter(|p| p.contains('?')) {
                    tracing::warn!(
                        rule_index = index_offset + i,
                        field = field,
                        pattern = %pattern,
                        effect = %rule.effect,
                        "ACL pattern contains '?', which is a LITERAL in ACL matching \
                         (Algorithm A08 — '*' is the only metacharacter) and which \
                         PROTOCOL_SPEC §2.7 forbids in a module ID. The pattern therefore \
                         matches nothing and can never match anything: a 'deny' rule guards \
                         nothing, an 'allow' rule never fires. '?' IS a wildcard in every \
                         other pattern surface (Algorithm A25, §9.2.3), which is where the \
                         expectation comes from. Nothing about this rule's meaning has \
                         changed — see §6.2.2."
                    );
                }
            }
        }
        for (i, rule) in rules.iter().enumerate() {
            let Some(conditions) = rule.conditions.as_ref() else {
                continue;
            };
            for fault in precheck_conditions(conditions, PrecheckPath::Sync) {
                tracing::warn!(
                    rule_index = index_offset + i,
                    condition_path = %fault.path,
                    condition_key = fault.key.as_deref().unwrap_or("-"),
                    effect = %rule.effect,
                    async_resolvable = fault.async_resolvable,
                    reason = %fault.reason,
                    "ACL rule fails the §6.1.4 precheck; it will be UNEVALUABLE \
                     (PROTOCOL_SPEC §6.1.1) — a deny rule denies, an allow rule does not \
                     grant. Fix the rule or register a handler, and call \
                     ACL::validate_rules() after bootstrap to assert on this."
                );
            }
        }
    }

    /// Attach (or clear) the ACL file's `audit:` block on an ACL that was built
    /// directly rather than loaded from a file.
    ///
    /// `new` / `try_new` take three arguments and leave `audit_config` at
    /// `None`, and only `load` / `reload` ever populated it — so a deployment
    /// that builds its rules in code could not declare an audit block at all,
    /// while apcore-python (`ACL(..., audit_config=...)`) and apcore-typescript
    /// both accept one as an optional fourth constructor parameter. This keeps
    /// the existing three-argument arity intact and supplies the setting
    /// separately.
    pub fn set_audit_config(&mut self, audit_config: Option<AuditConfig>) {
        self.audit_config = audit_config;
        // §6.3.2 requirement 1: one effective sink, rebuilt from the new
        // configuration — the same rule `set_audit_logger` follows, and what
        // scopes requirement 5's once-per-failure suppression to this
        // configuration rather than to the ACL's lifetime.
        self.audit_sink = AuditSink::new(self.audit_logger.clone(), self.audit_config.clone());
        warn_audit_block_overridden(self.audit_logger.as_ref(), self.audit_config.as_ref());
    }

    /// Builder form of [`Self::set_audit_config`], for chaining off
    /// [`ACL::new`] / [`ACL::try_new`].
    #[must_use]
    pub fn with_audit_config(mut self, audit_config: Option<AuditConfig>) -> Self {
        self.set_audit_config(audit_config);
        self
    }

    /// Set the audit logger callback.
    pub fn set_audit_logger(&mut self, logger: impl Fn(&AuditEntry) + Send + Sync + 'static) {
        self.audit_logger = Some(Arc::new(logger));
        // §6.3.2 requirement 1: the callback becomes the effective sink, and a
        // NEW sink is what scopes requirement 5's once-per-failure suppression
        // to this configuration rather than to the ACL's lifetime.
        self.audit_sink = AuditSink::new(self.audit_logger.clone(), self.audit_config.clone());
        warn_audit_block_overridden(self.audit_logger.as_ref(), self.audit_config.as_ref());
    }

    /// Evaluate all conditions with three-valued AND logic using the sync
    /// handler registry (PROTOCOL_SPEC §6.1.1).
    ///
    /// Returns a [`ConditionOutcome`], not a `bool`: "a handler answered no"
    /// and "no answer was obtainable" reach the rule loop as different values,
    /// because a `deny` rule resolves them in opposite directions. §6.3 makes
    /// carrying that distinction a MUST — collapsing the two is what let a
    /// misspelled condition key silently disable a `deny` rule before v1.22.0.
    ///
    /// This is a **sync** function. It drives each handler's future by polling
    /// it once with a noop waker:
    ///
    /// * `Poll::Ready(v)` — the handler's answer, `Satisfied` / `Unsatisfied`.
    /// * `Poll::Pending` — §6.1.1 case 3: the handler is genuinely async
    ///   and could not be resolved on this path, so the condition is
    ///   **`Unevaluable`**, not unsatisfied. Callers with I/O-performing
    ///   handlers must use [`ACL::async_check`].
    /// * a panic — §6.1.1 case 2, caught here and reported `Unevaluable`.
    ///
    /// A key with no handler in [`CONDITION_HANDLERS`] is §6.1.1 case 1.
    /// Note that the async registry is deliberately NOT consulted: §6.1.3 makes
    /// an async-only key unevaluable on the sync path by design.
    ///
    /// **Every** child is evaluated — no short-circuit. §6.1.1 permits
    /// short-circuiting AND on the first `Unsatisfied` and explicitly allows
    /// evaluating every child instead "for deterministic diagnostics"; the
    /// latter is what makes `handler_error` list every unevaluable key (rule 2)
    /// rather than whichever one map iteration happened to reach first. The
    /// decision is identical either way.
    ///
    /// **Architecture note:** two parallel paths exist — this sync path and the
    /// async [`Self::evaluate_conditions_async_outcome`]. Keep both in sync when
    /// adding new condition logic to avoid drift. New conditions should be
    /// tested against both paths.
    pub fn evaluate_conditions(
        conditions: &HashMap<String, serde_json::Value>,
        ctx: &Context<serde_json::Value>,
    ) -> ConditionOutcome {
        // Resolve handlers first; an unresolved key is a leaf outcome of its own.
        let mut to_evaluate = Vec::with_capacity(conditions.len());
        {
            let handlers = CONDITION_HANDLERS.read();
            for (key, value) in conditions {
                to_evaluate.push((key, handlers.get(key.as_str()).cloned(), value));
            }
        }

        // AND over three-valued children, starting from the vacuous truth of an
        // empty `conditions` object (which keeps `$not: {}` fail-closed).
        let mut outcome = ConditionOutcome::Satisfied;
        for (key, handler, value) in to_evaluate {
            let Some(handler) = handler else {
                // §6.1.1 case 1: no resolvable handler.
                tracing::warn!(
                    condition = %key,
                    "Unknown ACL condition — unevaluable (PROTOCOL_SPEC §6.1.1)"
                );
                crate::acl_handlers::report_condition_unevaluable(key, "unknown ACL condition");
                outcome = outcome.and(ConditionOutcome::Unevaluable);
                continue;
            };

            // A-D-011 (SECURITY): wrap both the future construction and the
            // single poll in catch_unwind so a panicking custom handler does
            // NOT unwind out of the ACL gate. Mirrors the Python `try/except`
            // and TypeScript `try/catch` around handler.evaluate.
            let poll_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let fut = handler.evaluate_outcome(value, ctx);
                let mut fut = std::pin::pin!(fut);
                let waker = std::task::Waker::noop();
                let mut cx = std::task::Context::from_waker(waker);
                fut.as_mut().poll(&mut cx)
            }));
            let child = match poll_result {
                Ok(std::task::Poll::Ready(val)) => val,
                Ok(std::task::Poll::Pending) => {
                    // §6.1.1 case 3. Through spec v1.21.0 this was
                    // "treated as unsatisfied", which made a `deny` rule
                    // guarded by an async-only handler inert on the sync path
                    // — the same failure mode as a misspelled key, reached by
                    // a different route.
                    tracing::warn!(
                        condition = %key,
                        "Async ACL condition not ready in a sync context — unevaluable \
                         (PROTOCOL_SPEC §6.1.1; use ACL::async_check)"
                    );
                    crate::acl_handlers::report_condition_unevaluable(
                        key,
                        "handler is not ready synchronously (use ACL::async_check)",
                    );
                    ConditionOutcome::Unevaluable
                }
                Err(payload) => {
                    let msg = crate::acl_handlers::panic_message(payload.as_ref());
                    tracing::error!(
                        condition = %key,
                        panic = %msg,
                        "ACL condition handler panicked — unevaluable (PROTOCOL_SPEC §6.1.1)"
                    );
                    crate::acl_handlers::report_condition_unevaluable(
                        key,
                        format!("handler panicked: {msg}"),
                    );
                    ConditionOutcome::Unevaluable
                }
            };
            outcome = outcome.and(child);
        }
        outcome
    }

    /// Async evaluate all conditions with AND logic using the handler registry.
    ///
    /// Boolean façade over [`Self::evaluate_conditions_async_outcome`]. An
    /// unevaluable result maps to `false`, which is exactly the collapse
    /// §6.1.1 forbids the **rule loop** from making — use the `_outcome` form
    /// anywhere the rule's `effect` matters.
    pub async fn evaluate_conditions_async(
        conditions: &HashMap<String, serde_json::Value>,
        ctx: &Context<serde_json::Value>,
    ) -> bool {
        Self::evaluate_conditions_async_outcome(conditions, ctx)
            .await
            .is_satisfied()
    }

    /// Async, three-valued counterpart to [`Self::evaluate_conditions`]
    /// (PROTOCOL_SPEC §6.1.1).
    ///
    /// Delegates to `acl_handlers::evaluate_conditions_async_outcome` so
    /// compound operators (`$or`, `$not`) share the same async evaluation path.
    pub async fn evaluate_conditions_async_outcome(
        conditions: &HashMap<String, serde_json::Value>,
        ctx: &Context<serde_json::Value>,
    ) -> ConditionOutcome {
        handlers_evaluate_conditions_async_outcome(conditions, ctx).await
    }

    /// Add a rule to the ACL (inserted at position 0, highest priority).
    ///
    /// Spec contract `acl-system.md` §Contract.ACL.add_rule declares
    /// `On success: None` — the body is infallible, so no `Result` wrapper.
    ///
    /// PROTOCOL_SPEC §6.1.2 rule 4 makes runtime rule insertion an entry point
    /// that MUST be covered by the load-time condition-key check: if the rule
    /// carries `conditions`, every key it references — including keys nested
    /// inside `$or` / `$not` — is checked against the sync handler registry and
    /// a warning naming the rule index (`0`), the key and the rule's `effect`
    /// is emitted for each that does not resolve. Insertion still succeeds;
    /// this is the same warn-never-fail contract [`ACL::load`] has, for the
    /// same reason.
    ///
    /// # Panics
    ///
    /// Panics when the rule carries an `effect` outside `allow` / `deny`
    /// (PROTOCOL_SPEC §6.1.5, spec v1.30.0, #111) or `approval: required` on a
    /// `deny` effect (§6.1.6 rule 2). Use [`ACL::try_add_rule`] for fallible
    /// insertion — the pairing [`ACL::new`] and [`ACL::try_new`] already use.
    /// It panics on a `callers` / `targets` array whose shape is outside
    /// §6.2.1's table too (spec v1.31.0, #112) — runtime insertion is the third
    /// of the doors that rule closes, and the panic is what this method's
    /// infallible signature makes of a refusal, not an exemption from one.
    ///
    /// Neither of the first two can break a caller written before the release
    /// that added it. The `approval` field did not exist before v1.28.0, so
    /// that state was not constructible; an out-of-enum `effect` was never a
    /// *working* rule, only one whose value travelled uninspected into
    /// `AccessDecision::access`. A rule built in code is as meaningless as one
    /// parsed from YAML, which is why all three entry points refuse it rather
    /// than two refusing and one letting it through (§6.1.6 rule 3).
    ///
    /// The shape rejection **is** a break, deliberately: a caller inserting
    /// `targets: []` used to succeed and get a rule that decided nothing. The
    /// affected population is exactly the callers who believe they inserted a
    /// rule and did not, and the migration is `["*"]` for "everything" or
    /// deleting the call for "nothing".
    pub fn add_rule(&mut self, rule: ACLRule) {
        // Same reasoning as [`ACL::new`]: the panic carries `try_add_rule`'s
        // message, which names the rule index and the offending value.
        self.try_add_rule(rule).unwrap_or_else(|e| {
            panic!(
                "{} — use ACL::try_add_rule for fallible insertion",
                e.message
            )
        });
    }

    /// [`ACL::add_rule`], returning the §6.1.6 rejection instead of panicking.
    ///
    /// # Errors
    ///
    /// Returns `ModuleError` with `ErrorCode::ACLRuleError` when the rule
    /// carries an `effect` outside `allow` / `deny` (§6.1.5, spec v1.30.0,
    /// #111), `approval: required` on a `deny` effect (§6.1.6 rule 2), or a
    /// `callers` / `targets` array whose shape is outside §6.2.1's table (spec
    /// v1.31.0, #112). "Denied **and** needs approval" is not a state that
    /// means anything, an array that can never match is not a narrow rule but
    /// no rule at all, and accepting half of a governance rule is the failure
    /// mode §6.1.5 was written to end.
    pub fn try_add_rule(&mut self, rule: ACLRule) -> Result<(), ModuleError> {
        // Index 0 — the position the rule is about to occupy, so the refusal
        // names the rule the caller will find at the head of `rules()`.
        // Validated BEFORE the precheck warning and before insertion: a rule
        // that cannot be accepted should not leave a warning behind about a
        // condition nobody will ever evaluate.
        Self::validate_rule(0, &rule)?;
        Self::warn_rule_faults(std::slice::from_ref(&rule), 0);
        self.rules.insert(0, rule);
        // D-88: insertion at index 0 shifts every existing rule down by one.
        self.clear_missing_context_warnings();
        Ok(())
    }

    /// Report every rule that fails PROTOCOL_SPEC §6.1.4's precheck
    /// (§6.1.2 rule 3, §6.1.3).
    ///
    /// Named `validate_rules` rather than `validate_conditions` because it
    /// reports structural faults in the rule as a whole, not only unresolvable
    /// condition keys: a `$or` whose value is not a list, a `$not` whose value
    /// is not an object, a `conditions` that is not a mapping, and a `callers`
    /// / `targets` array whose **shape** is outside §6.2.1's table are all
    /// findings. (§6.1.4.1's malformed `callers` / `targets` *type* cannot
    /// arise here — see the note below.)
    ///
    /// Condition handlers are registered at runtime into a process-wide
    /// registry, and an ACL may legitimately be loaded before a deployment
    /// registers its custom handlers, so loading only warns. This method is the
    /// deterministic check to run **after** registration is complete: the
    /// intended shape is to call it once bootstrap has finished and to treat
    /// any finding on a `deny` rule as a startup error.
    ///
    /// A finding is emitted whenever `sync_resolvable` is false — **including**
    /// when `async_resolvable` is true. §6.1.3: `check()` consults only the
    /// sync registry, so an async-only key is a working condition under
    /// `async_check()` and an unevaluable one under `check()`. A caller that
    /// only ever uses `async_check()` may ignore such a finding; that choice
    /// belongs to the caller, not to the validator.
    ///
    /// Pure read — never mutates the ACL, never registers a handler, never
    /// emits an audit event. Findings are ordered by rule index, then
    /// ascending by condition **path**. An empty result is not a guarantee
    /// about the future: a later [`Self::add_rule`] can introduce a new fault.
    ///
    /// This is diagnostics, not enforcement. The guarantee that a broken `deny`
    /// rule cannot silently pass traffic is §6.1.1's, and holds whether or not
    /// anyone calls this.
    ///
    /// # `callers` / `targets`: the **type** is checked by the type system, the
    /// **shape** is not
    ///
    /// §6.1.4.1 requires the precheck to classify a non-list `callers` /
    /// `targets` as unevaluable, because a bare string is iterable in several
    /// host languages: `callers: "admin.*"` iterates character by character,
    /// and the `*` character matches everything, so the typo grants access to
    /// every caller. [`ACLRule::callers`] and [`ACLRule::targets`] are
    /// `Vec<String>`, so that malformed *value* is unrepresentable — serde
    /// rejects a bare string at deserialization and the compiler rejects one in
    /// a struct literal. §6.1.4.1 states explicitly that such an implementation
    /// "satisfies this clause by construction and needs no runtime check", so
    /// there is deliberately no type check here for a state that cannot exist.
    ///
    /// That argument reaches the element type and **stops there**: a
    /// `Vec<String>` places no constraint on its length and none on what its
    /// elements say, so `targets: vec![]`, `vec!["$or".into()]` and
    /// `vec!["$not".into(), a, b]` are all perfectly constructible (spec
    /// v1.31.0, #112). §6.2.1 closes those at every door, and this method
    /// reports whatever arrives around one — see
    /// [`ACL::precheck_pattern_faults`].
    #[must_use]
    pub fn validate_rules(&self) -> Vec<RuleValidationFinding> {
        let mut findings = Vec::new();
        for (idx, rule) in self.rules.iter().enumerate() {
            // §6.2.1's two tiers, per field and in that order: a structural
            // fault is reported instead of a semantic one, never alongside it,
            // because tier 2's predicate assumes a well-formed array. Tier 2
            // is reachable from HERE ONLY — it must never feed the per-call
            // handler-error scope or a decision.
            let mut faults: Vec<RuleFault> = Vec::new();
            for (field, patterns) in [("callers", &rule.callers), ("targets", &rule.targets)] {
                if let Some(reason) = pattern_shape_fault(field, patterns)
                    .or_else(|| pattern_never_matches(field, patterns))
                {
                    faults.push(pattern_rule_fault(field, reason));
                }
            }
            if let Some(conditions) = rule.conditions.as_ref() {
                faults.extend(precheck_conditions(conditions, PrecheckPath::Sync));
            }
            // `precheck_conditions` already returns faults ordered by path, but
            // the pattern faults are appended from a separate walk, so the
            // merged list is re-sorted: rules are visited in definition order,
            // so the result is ordered by (rule_index, path) as §6.1.2 rule 3
            // requires.
            faults.sort_by(|a, b| a.path.cmp(&b.path));
            for fault in faults {
                findings.push(RuleValidationFinding::new(
                    idx,
                    fault.path,
                    fault.key,
                    rule.effect.clone(),
                    fault.sync_resolvable,
                    fault.async_resolvable,
                ));
            }
        }
        findings
    }

    /// Remove the first rule matching the given callers and targets.
    /// Returns true if a rule was removed.
    pub fn remove_rule(&mut self, callers: &[String], targets: &[String]) -> bool {
        self.remove_rule_with_conditions(callers, targets, None)
    }

    /// Remove the first rule matching callers, targets, and (optional) conditions.
    ///
    /// When `conditions` is `Some(value)`, additionally disambiguate by
    /// `ACLRule.conditions` via JSON value equality. Two rules with identical
    /// callers+targets but different conditions can be selectively removed by
    /// passing the matching conditions. Cross-language parity with
    /// apcore-typescript removeRule (sync finding A-D-026).
    pub fn remove_rule_with_conditions(
        &mut self,
        callers: &[String],
        targets: &[String],
        conditions: Option<&serde_json::Value>,
    ) -> bool {
        if let Some(pos) = self.rules.iter().position(|r| {
            if r.callers != callers || r.targets != targets {
                return false;
            }
            match conditions {
                Some(want) => r.conditions.as_ref() == Some(want),
                None => true,
            }
        }) {
            self.rules.remove(pos);
            // D-88: removing rule `pos` shifts every rule after it up by one.
            self.clear_missing_context_warnings();
            true
        } else {
            false
        }
    }

    /// Check whether the given caller is allowed to access the target.
    /// Uses first-match-wins evaluation. Maps `None` caller to `@external`.
    ///
    /// Returns `true` for allow, `false` for deny. Never errors — deny is
    /// signalled via the return value, not an `Err`, per the protocol spec.
    ///
    /// # This boolean fails closed on an approval requirement
    ///
    /// A **decision** resolving to `allow` with an approval requirement
    /// (§6.1.6) makes this method return **`false`** — see
    /// [`Self::check_access`] for the result that distinguishes the two axes.
    ///
    /// §6.8.1 states this of the decision rather than of the matched rule on
    /// purpose: since spec v1.29.0 the requirement may be a pending one raised
    /// by an unevaluable `allow` rule that did not itself match, or carried
    /// through `default_effect: allow` (§6.1.1 rule 5), and the boolean fails
    /// closed on those identically. It does so here for free, because it reads
    /// [`AccessDecision::approval_required`] rather than re-deriving it.
    ///
    /// `check()` is public API consumed by callers that are not the Executor —
    /// tooling, preflight helpers, third-party integrations — and such a caller
    /// can only read a boolean as "let it through / do not". Returning `true`
    /// would let it execute a call the ACL said needed a human. `false` is
    /// wrong in the benign direction: the caller sees a refusal where the truth
    /// was "ask first" (PROTOCOL_SPEC §6.8.1). The Executor uses
    /// [`Self::check_access`] and is unaffected, and a legacy caller only meets
    /// this at all once an operator has authored a rule carrying `approval`.
    ///
    /// Sync entry point. The shared post-decision audit logic lives in
    /// `finalize_*` helpers so this method and `async_check` cannot drift.
    pub fn check(
        &self,
        caller_id: Option<&str>,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> bool {
        let decision = self.check_access(caller_id, target_id, ctx, None);
        decision.is_allowed() && !decision.approval_required
    }

    /// The structured result of a check — authorization **and** approval
    /// requirement (PROTOCOL_SPEC §6.8.1).
    ///
    /// `projection` is the §6.1.8 governance projection of the call's
    /// arguments, which the built-in `arguments` condition (§6.1.7) reads. The
    /// execution pipeline computes it during module lookup (Step 3) and hands
    /// it to this method at Step 4. `None` means the caller has no call site —
    /// tooling asking "may X reach Y?" rather than "may X reach Y with these
    /// arguments?" — and any `arguments` condition is then **unevaluable**
    /// (§6.1.1): the question cannot be answered as written, so a `deny` rule
    /// carrying one takes effect and an `allow` rule carrying one does not
    /// grant. It does, however, leave any `approval: required` it carried
    /// **pending** for whatever grants next (§6.1.1 rule 5) — not granting is
    /// not the same as having asked for nothing.
    pub fn check_access(
        &self,
        caller_id: Option<&str>,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
        projection: Option<&GovernanceProjection>,
    ) -> AccessDecision {
        // Wrap the entire evaluation in a synchronous handler-error capture
        // scope (A-D-002) so any condition handler that calls
        // `report_handler_error(...)` lands its message in this call's audit
        // entry — mirroring `async_check`'s task-local scope and Python's
        // sync `_handler_error_var.set(None)` / `reset(token)` pairing.
        // Without this, `build_audit_entry` always read `None` on the sync
        // path because the only scope was tokio-task-local. The same scope
        // carries the governance projection down to the `arguments` handler,
        // whose `evaluate(value, ctx)` signature has nowhere else to take it.
        let (decision, _captured) = crate::acl_handlers::with_acl_evaluation_scope_sync(
            projection.cloned().map(Arc::new),
            || self.check_inner(caller_id, target_id, ctx),
        );
        decision
    }

    fn check_inner(
        &self,
        caller_id: Option<&str>,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> AccessDecision {
        let caller = caller_id.unwrap_or(EXTERNAL_CALLER);

        // Snapshot rules + default_effect before evaluation so any concurrent
        // add_rule/reload caller (wrapped in Arc<RwLock<ACL>> by the user) does
        // not mutate the list mid-check. Matches apcore-python's
        // _snapshot() (acl.py:282) and apcore-typescript's rules.slice()
        // (acl.ts:203) — sync finding A-D-021.
        let rules: Vec<ACLRule> = self.rules.clone();
        let default_effect = self.default_effect.clone();

        if rules.is_empty() {
            return self.finalize_no_rules(&default_effect, caller, target_id, ctx);
        }

        // §6.1.1 rule 5: an `allow` rule that carried `approval: required` and
        // turned out UNEVALUABLE steps aside without granting, but the
        // requirement it carried survives the scan and attaches to whatever
        // grants next — a later `allow` rule or `default_effect: allow`. A
        // denial clears it; the `finalize_*` helpers own that composition so
        // this loop and its async twin cannot disagree about it.
        let mut pending_approval = false;

        for (idx, rule) in rules.iter().enumerate() {
            let paths_before = crate::acl_handlers::reported_condition_paths();
            match self.matches_rule(idx, rule, caller, target_id, ctx) {
                RuleMatch::Match => {
                    return self.finalize_rule_match(
                        idx,
                        rule,
                        caller,
                        target_id,
                        ctx,
                        pending_approval,
                    )
                }
                RuleMatch::Unevaluable => {
                    match Self::resolve_unevaluable_rule(idx, rule, &paths_before) {
                        UnevaluableResolution::TakesEffect => {
                            return self.finalize_rule_match(
                                idx,
                                rule,
                                caller,
                                target_id,
                                ctx,
                                pending_approval,
                            )
                        }
                        UnevaluableResolution::StepsAside {
                            pending_approval: p,
                        } => {
                            pending_approval |= p;
                        }
                    }
                }
                RuleMatch::NoMatch => {}
            }
        }

        // Use the snapshotted default_effect rather than re-reading
        // self.default_effect to maintain consistency with the snapshotted
        // rules (sync finding A-D-021 / A-D-301).
        self.finalize_default_effect(&default_effect, caller, target_id, ctx, pending_approval)
    }

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
    /// from it when the path exists. The path is resolved relative to the
    /// directory of the config's source file when known
    /// ([`Config::source_path`]), otherwise relative to the current working
    /// directory.
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
    /// behavior. The `acl.default_effect` config key only takes effect once a
    /// real ACL file is loaded (it is read by [`ACL::load`] from the ACL file
    /// itself); it never feeds a synthesized ACL here.
    ///
    /// # Errors
    /// Returns [`ModuleError`] only when a resolved ACL file exists but is
    /// structurally invalid (propagated from [`ACL::load`] with
    /// `ErrorCode::ACLRuleError`). A missing root is never an error — it yields
    /// `Ok(None)`.
    pub fn discover(config: &crate::config::Config) -> Result<Option<Self>, ModuleError> {
        let Some(root) = config
            .get("acl.root")
            .and_then(|v| v.as_str().map(std::string::ToString::to_string))
        else {
            return Ok(None);
        };

        let mut root_path = std::path::PathBuf::from(&root);
        if !root_path.is_absolute() {
            let base = match config.source_path() {
                Some(source) => source
                    .canonicalize()
                    .unwrap_or_else(|_| source.to_path_buf())
                    .parent()
                    .map_or_else(
                        || std::path::PathBuf::from("."),
                        std::path::Path::to_path_buf,
                    ),
                None => std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            };
            root_path = base.join(&root_path);
        }

        if !root_path.exists() {
            // Missing path -> no enforcement. Do NOT synthesize an ACL.
            return Ok(None);
        }

        if root_path.is_dir() {
            // Directory convention: acl/{scope}_acl.yaml (PROTOCOL_SPEC §3.1).
            let acl_file = root_path.join("global_acl.yaml");
            if !acl_file.is_file() {
                // Directory present but no conventional ACL file -> no-op.
                return Ok(None);
            }
            return Self::load(&acl_file.to_string_lossy()).map(Some);
        }

        Self::load(&root_path.to_string_lossy()).map(Some)
    }

    /// Register a custom condition handler. Delegates to `acl_handlers::register_condition`.
    pub fn register_condition(
        key: impl Into<String>,
        handler: std::sync::Arc<dyn crate::acl_handlers::ACLConditionHandler>,
    ) {
        crate::acl_handlers::register_condition(key, handler);
    }

    /// Register a custom async-only condition handler.
    ///
    /// Stored in a **separate registry** from sync handlers — `async_check`
    /// consults the async registry first, then falls back to the sync
    /// registry. This lets callers override a sync handler for a given key
    /// with an async-only variant without affecting the sync `ACL::check`
    /// path. Cross-language parity with apcore-python
    /// `register_async_condition` and apcore-typescript
    /// `registerAsyncCondition` (closes A-D-ACL-002).
    pub fn register_async_condition(
        key: impl Into<String>,
        handler: std::sync::Arc<dyn crate::acl_handlers::ACLConditionHandler>,
    ) {
        crate::acl_handlers::register_async_condition(key, handler);
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

    /// The current rule list, in definition order (PROTOCOL_SPEC §6.8).
    ///
    /// Read-only introspection: an immutable slice, so a caller can neither
    /// reorder nor mutate the ACL's own list through it (§6.8 rule 3). It is a
    /// pure read — no audit event, no state change, no lock the caller has to
    /// release — and it reads the live object, so it reflects a
    /// [`Self::reload`] (§6.8 rule 4).
    #[must_use]
    pub fn rules(&self) -> &[ACLRule] {
        &self.rules
    }

    /// The effect applied when no rule matches — `"allow"` or `"deny"`
    /// (PROTOCOL_SPEC §6.8).
    ///
    /// `default_effect` is the single most consequential value in an ACL, and
    /// §6.8 makes reading it back a MUST: without it, tooling that reports or
    /// audits the enforced policy has to re-read and re-parse the ACL file to
    /// recover a value the loaded object already holds — a second copy that can
    /// drift across [`Self::reload`], and on Rust the only option available at
    /// all while the field was private.
    ///
    /// Pure read, and reads the live object, so it reflects a `reload()`.
    #[must_use]
    pub fn default_effect(&self) -> &str {
        &self.default_effect
    }

    // --- Private helpers ---

    /// Apply PROTOCOL_SPEC §6.1.1 to a rule whose conditions were UNEVALUABLE.
    ///
    /// Returns [`UnevaluableResolution::TakesEffect`] when the rule must take
    /// effect (so the caller finalizes it as a rule match — a `deny` rule
    /// denies), and [`UnevaluableResolution::StepsAside`] when evaluation must
    /// continue with the next rule (an `allow` rule MUST NOT grant).
    ///
    /// **Rule 5 (spec v1.29.0, apcore#109).** Stepping aside was a complete
    /// instruction while a rule carried one axis; since §6.1.6 it carries two,
    /// and the second one is not lost when the first resolves to unevaluable.
    /// An `allow` rule carrying `approval: required` therefore reports
    /// `pending_approval: true` on its way out — it still does not grant, but
    /// whatever grants *later* inherits the requirement. Without this the
    /// shape §6.1.7 exists for (a narrow approval rule ahead of a broad allow)
    /// resolved to `allow` with `approval_required: false` on exactly the call
    /// the operator gated.
    ///
    /// A rule written about another caller or target has already left as
    /// `NoMatch` with its conditions never consulted (§6.1.4 rule 4), so a
    /// *readable* scope is not re-checked here. Rule 5's "unknowable scope
    /// counts as scope" clause is a different matter and does reach this
    /// function: `matches_rule` runs §6.1.4.1's pattern precheck first, so a
    /// rule whose `callers` / `targets` **shape** is outside §6.2.1's table
    /// arrives as `Unevaluable` before either list is matched, and an `allow`
    /// rule carrying `approval: required` raises the pending requirement from
    /// here (spec v1.31.0, #112). Only the field's *type* is satisfied
    /// structurally in this SDK — `Vec<String>` constrains the element type and
    /// nothing about the array's length (see the `validate_rules` note).
    ///
    /// Also emits §6.1.1 rule 3's warning, which must name the fault's
    /// **path**, the rule's index and the rule's `effect`. The `effect` is in
    /// the message because a misconfigured `deny` rule is the consequential
    /// case. The paths are recovered by diffing the per-call handler-error
    /// scope, which is where both precheck-origin and execution-origin faults
    /// land — including one nested inside `$or` / `$not`, and including the
    /// bare `callers` / `targets` path a pattern-shape fault reports at.
    fn resolve_unevaluable_rule(
        idx: usize,
        rule: &ACLRule,
        paths_before: &[String],
    ) -> UnevaluableResolution {
        let paths_after = crate::acl_handlers::reported_condition_paths();
        let new_paths: Vec<String> = paths_after
            .into_iter()
            .filter(|p| !paths_before.contains(p))
            .collect();
        let conditions = if new_paths.is_empty() {
            // Already reported by an earlier rule in this same check().
            "(see AuditEntry.handler_error)".to_string()
        } else {
            new_paths.join(", ")
        };

        let takes_effect = rule.effect == "deny";
        // `approval_required()` is already false for a `deny` rule, and a deny
        // rule that takes effect returns before the pending value is read, so
        // the two branches cannot both be live.
        let pending_approval = rule.approval_required();
        tracing::warn!(
            rule_index = idx,
            effect = %rule.effect,
            condition_paths = %conditions,
            pending_approval,
            "ACL rule is UNEVALUABLE — {} (PROTOCOL_SPEC §6.1.1)",
            if takes_effect {
                "the deny rule TAKES EFFECT and the call is denied"
            } else if pending_approval {
                "the allow rule does not match and MUST NOT grant, but the `approval: required` \
                 it carried is PENDING and attaches to whatever grants next (rule 5)"
            } else {
                "the allow rule does not match and MUST NOT grant"
            }
        );
        if takes_effect {
            UnevaluableResolution::TakesEffect
        } else {
            UnevaluableResolution::StepsAside { pending_approval }
        }
    }

    /// Check if a rule matches the caller, target, and context.
    ///
    /// Returns [`RuleMatch`] rather than a `bool`: a rule whose conditions were
    /// unevaluable is neither matched nor unmatched until its `effect` is
    /// consulted (PROTOCOL_SPEC §6.1.1).
    fn matches_rule(
        &self,
        rule_index: usize,
        rule: &ACLRule,
        caller: &str,
        target: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> RuleMatch {
        // §6.1.4.1 runs BEFORE the matcher: a rule whose scope cannot be read
        // is unevaluable, not unmatched, and reading "does not match" off a
        // malformed field is the fail-open #112 was opened about.
        if Self::pattern_precheck_failed(rule) {
            return RuleMatch::Unevaluable;
        }

        if !Self::match_patterns(&rule.callers, caller, ctx) {
            return RuleMatch::NoMatch;
        }

        if !Self::match_patterns(&rule.targets, target, ctx) {
            return RuleMatch::NoMatch;
        }

        // Conditions check.
        if let Some(ref conditions) = rule.conditions {
            return match self.check_conditions(rule_index, rule, conditions, ctx) {
                ConditionOutcome::Satisfied => RuleMatch::Match,
                ConditionOutcome::Unsatisfied => RuleMatch::NoMatch,
                ConditionOutcome::Unevaluable => RuleMatch::Unevaluable,
            };
        }

        RuleMatch::Match
    }

    /// Warn ONCE per `(rule index, effect)` that a conditional rule was skipped
    /// for want of a context (§6.5, D-88).
    ///
    /// The dedupe key is the rule INDEX, so
    /// [`clear_missing_context_warnings`](Self::clear_missing_context_warnings)
    /// must run on every mutation that shifts indices. Word-for-word the
    /// message apcore-python and apcore-typescript emit.
    fn warn_conditional_rule_without_context(&self, rule_index: usize, effect: &str) {
        {
            let mut warned = self.warned_missing_context.lock();
            if !warned.insert((rule_index, effect.to_string())) {
                return;
            }
        }
        tracing::warn!(
            rule_index,
            effect,
            "ACL rule {rule_index} (effect={effect}) has conditions but the check supplied no \
             context, so the rule does not match (PROTOCOL_SPEC §6.5). A conditional 'deny' rule \
             is therefore not a backstop for context-less callers — express a backstop as an \
             unconditional 'deny' rule or as default_effect: deny."
        );
    }

    /// Drop every §6.5 warning marker (D-88).
    ///
    /// Called by every operation that inserts, removes or reorders rules. The
    /// markers are keyed by index; once the indices move, a retained marker
    /// suppresses the warning for a rule it was never recorded for.
    fn clear_missing_context_warnings(&self) {
        self.warned_missing_context.lock().clear();
    }

    /// Run PROTOCOL_SPEC §6.1.4's precheck and report any faults into the
    /// per-call handler-error scope. Returns `true` when the rule is
    /// unevaluable on structural or registry grounds alone.
    ///
    /// This runs **before** §6.5's no-context check (§6.1.4 rule 1), which is
    /// what closes the bypass where `conditions: {mispelled: true}` on a `deny`
    /// rule passed traffic simply because the caller carried no identity. A
    /// rule that *passes* here and then finds no context still takes §6.5's
    /// path and does not match: `roles` is answerable in principle, and this
    /// caller merely supplied no input for it (§6.1.4 rule 2).
    fn precheck_failed(conditions: &serde_json::Value, path: PrecheckPath) -> bool {
        let faults: Vec<RuleFault> = precheck_conditions(conditions, path);
        if faults.is_empty() {
            return false;
        }
        for fault in &faults {
            crate::acl_handlers::report_condition_unevaluable_at(&fault.path, &fault.reason);
        }
        true
    }

    /// Run PROTOCOL_SPEC §6.1.4.1's precheck over a rule's **pattern fields**
    /// (spec v1.31.0, #112).
    ///
    /// The backstop for the one route §6.2.1's door-closing cannot reach.
    /// [`ACLRule`]'s fields are `pub`, so `rule.targets = vec![]` on an
    /// already-constructed rule bypasses [`ACL::try_new`], [`ACL::try_add_rule`]
    /// and [`ACL::load`] alike. Unlike an unrecognised `effect` — which, once
    /// the doors are closed, is never read again — a mutated pattern array
    /// **is** read: the matcher would consult it on the next `check()`.
    ///
    /// **Both fields are always examined** (§6.1.4 rule 3): the walk does not
    /// short-circuit on the first fault, which is what makes the reported set a
    /// pure function of the rule. Faults come back in field order, which is
    /// already the lexicographic path order §6.1.1 rule 2 requires
    /// (`callers` < `targets`).
    ///
    /// Tier 1 only. [`pattern_never_matches`] is deliberately **not** consulted
    /// here: a well-formed array that happens to match nothing is a finding,
    /// and routing it through this function would make it deny.
    fn precheck_pattern_faults(rule: &ACLRule) -> Vec<RuleFault> {
        let mut faults = Vec::new();
        for (field, patterns) in [("callers", &rule.callers), ("targets", &rule.targets)] {
            if let Some(reason) = pattern_shape_fault(field, patterns) {
                faults.push(pattern_rule_fault(field, reason));
            }
        }
        faults
    }

    /// [`Self::precheck_pattern_faults`], reporting each fault into the
    /// per-call handler-error scope and answering whether the rule is
    /// UNEVALUABLE on its scope alone.
    ///
    /// Shaped exactly like [`Self::precheck_failed`], and reporting through the
    /// same sink, so a malformed pattern field reaches `AuditEntry.handler_error`
    /// by the route a malformed condition already did.
    fn pattern_precheck_failed(rule: &ACLRule) -> bool {
        let faults = Self::precheck_pattern_faults(rule);
        if faults.is_empty() {
            return false;
        }
        for fault in &faults {
            crate::acl_handlers::report_condition_unevaluable_at(&fault.path, &fault.reason);
        }
        true
    }

    /// Match a list of patterns against a value.
    /// Supports compound operators: `$or` (any match) and `$not` (negate).
    ///
    /// The `false` returns for an empty list and for a `$not` with nothing to
    /// negate are **defence in depth only** (PROTOCOL_SPEC §6.2.1, spec
    /// v1.31.0, #112). Those shapes are refused at every entry point and
    /// classified UNEVALUABLE by [`Self::pattern_precheck_failed`], which runs
    /// first, so a malformed array never reaches this function — and reading
    /// an arity fault as a non-match here is exactly the fail-open #112 was
    /// opened about. They are kept rather than replaced with a panic because a
    /// matcher is not the right place to decide a rule's fate.
    fn match_patterns(
        patterns: &[String],
        value: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> bool {
        if patterns.is_empty() {
            return false;
        }

        let first = patterns[0].as_str();
        if first == "$or" {
            return patterns[1..]
                .iter()
                .any(|p| Self::match_acl_pattern_with_ctx(p, value, ctx));
        }
        if first == "$not" {
            if patterns.len() < 2 {
                return false;
            }
            return !Self::match_acl_pattern_with_ctx(&patterns[1], value, ctx);
        }

        // Standard OR: any pattern matches
        patterns
            .iter()
            .any(|p| Self::match_acl_pattern_with_ctx(p, value, ctx))
    }

    /// Pattern matching for ACL patterns. Handles `@external`, `@system`, and
    /// delegates to `match_pattern()` for wildcard/glob matching.
    fn match_acl_pattern(pattern: &str, value: &str) -> bool {
        if pattern == EXTERNAL_CALLER {
            return value == EXTERNAL_CALLER;
        }
        // @system is handled in match_acl_pattern_with_ctx (needs identity check)
        if pattern == "@system" {
            return false; // caller string is never literally "@system"
        }
        match_pattern(pattern, value)
    }

    fn match_acl_pattern_with_ctx(
        pattern: &str,
        value: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> bool {
        if pattern == "@system" {
            return ctx
                .and_then(|c| c.identity.as_ref())
                .is_some_and(|id| id.identity_type() == "system");
        }
        Self::match_acl_pattern(pattern, value)
    }

    /// Evaluate conditions block against the context using registered handlers.
    ///
    /// A missing context is `Unsatisfied`, NOT `Unevaluable`: PROTOCOL_SPEC
    /// §6.5 keeps "conditions present but no context provided" an ordinary
    /// non-match on purpose, because calling with no context is a legitimate
    /// shape for external entry points rather than a misconfiguration. Treating
    /// it as an evaluation failure would flip the decision for every
    /// `@external` call that meets a conditional `deny` rule.
    fn check_conditions(
        &self,
        rule_index: usize,
        rule: &ACLRule,
        conditions: &serde_json::Value,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> ConditionOutcome {
        // §6.1.4 rule 1: the precheck is context-independent and runs FIRST.
        if Self::precheck_failed(conditions, PrecheckPath::Sync) {
            return ConditionOutcome::Unevaluable;
        }

        let Some(ctx) = ctx else {
            // §6.5: conditions require context. Warn so the consequence is
            // visible — a well-formed conditional `deny` rule is not a backstop
            // for context-less callers.
            self.warn_conditional_rule_without_context(rule_index, &rule.effect);
            return ConditionOutcome::Unsatisfied;
        };

        // The precheck already established that `conditions` is a mapping.
        let Some(obj) = conditions.as_object() else {
            return ConditionOutcome::Unevaluable;
        };

        let map: HashMap<String, serde_json::Value> =
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        Self::evaluate_conditions(&map, ctx)
    }

    /// Async counterpart to `check_conditions`. Drives async condition handlers
    /// via `evaluate_conditions_async_outcome` so handlers that genuinely
    /// suspend are awaited rather than reported unevaluable.
    async fn check_conditions_async(
        &self,
        rule_index: usize,
        rule: &ACLRule,
        conditions: &serde_json::Value,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> ConditionOutcome {
        // §6.1.4 rule 1, on the async path's registries (§6.1.3).
        if Self::precheck_failed(conditions, PrecheckPath::Async) {
            return ConditionOutcome::Unevaluable;
        }

        let Some(ctx) = ctx else {
            // §6.5, as in the sync twin.
            self.warn_conditional_rule_without_context(rule_index, &rule.effect);
            return ConditionOutcome::Unsatisfied;
        };

        // The precheck already established that `conditions` is a mapping.
        let Some(obj) = conditions.as_object() else {
            return ConditionOutcome::Unevaluable;
        };

        let map: HashMap<String, serde_json::Value> =
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        Self::evaluate_conditions_async_outcome(&map, ctx).await
    }

    /// Audit + return for the empty-rules path. Shared by `check` and `async_check`.
    ///
    /// `default_effect` is passed in as a parameter (rather than re-read from
    /// `self`) so that callers can supply a consistent snapshot taken at the
    /// entry of the check, eliminating TOCTOU drift if a concurrent
    /// add_rule/reload mutates the ACL during evaluation (sync finding A-D-301).
    fn finalize_no_rules(
        &self,
        default_effect: &str,
        caller: &str,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> AccessDecision {
        // §6.9 row 2: `default_effect` is `allow` / `deny` only. There is no
        // default approval *source*, and no match means `false`.
        //
        // Unlike `finalize_default_effect`, this branch takes no pending
        // requirement (§6.1.1 rule 5): it is reached only when the rule list is
        // EMPTY, and a requirement can only be raised by a rule. There is
        // nothing to carry, so no parameter to carry it.
        let entry = self.build_audit_entry(
            caller,
            target_id,
            default_effect,
            "no_rules",
            None,
            None,
            false,
            ctx,
        );
        self.emit_audit(&entry);
        AccessDecision::new(default_effect, false, None, "no_rules")
    }

    /// Audit + return for a matched rule. Shared by `check` and `async_check`.
    ///
    /// `pending_approval` is the §6.1.1 rule 5 requirement accumulated from
    /// `allow` rules that were unevaluable earlier in the same scan. Composing
    /// it here rather than at the call sites is what keeps the sync and async
    /// loops from drifting on it, exactly as they already share audit
    /// construction.
    fn finalize_rule_match(
        &self,
        idx: usize,
        rule: &ACLRule,
        caller: &str,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
        pending_approval: bool,
    ) -> AccessDecision {
        // §6.9 row 1: the matched rule determines `access`; the approval
        // requirement is that rule's own UNION any pending one (§6.1.1 rule 5),
        // which may therefore originate in a rule that did not match.
        //
        // A denial clears it. `approval_required()` is already false on a
        // `deny` rule, but the pending term is not, and "denied and needs
        // approval" is not a state that means anything (§6.1.6) — the call is
        // not going to run, so there is nothing to put to a human.
        // `matched_rule_index` still names the rule that actually decided
        // rather than the unevaluable one.
        let approval_required =
            rule.effect == "allow" && (rule.approval_required() || pending_approval);
        let entry = self.build_audit_entry(
            caller,
            target_id,
            &rule.effect,
            "rule_match",
            rule.description.as_deref(),
            Some(idx),
            approval_required,
            ctx,
        );
        self.emit_audit(&entry);
        AccessDecision::new(
            rule.effect.clone(),
            approval_required,
            Some(idx),
            "rule_match",
        )
    }

    /// Audit + return for the no-rule-matched path. Shared by `check` and
    /// `async_check`.
    ///
    /// `default_effect` is passed in as a parameter — see
    /// [`Self::finalize_no_rules`] for rationale (sync finding A-D-301).
    fn finalize_default_effect(
        &self,
        default_effect: &str,
        caller: &str,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
        pending_approval: bool,
    ) -> AccessDecision {
        // §6.9 row 2 as amended in v1.29.0: there is still no default approval
        // *source*, but `default_effect: allow` MUST carry a pending
        // requirement (§6.1.1 rule 5) through to the result. This is the
        // boundary a "a later rule grants" reading misses — there may BE no
        // later rule, and the requirement would then be lost with nothing to
        // carry it. The result is `approval_required: true` with
        // `matched_rule_index: None`, which is a legal combination since
        // v1.29.0. A `deny` default clears it, per `finalize_rule_match`.
        let approval_required = default_effect == "allow" && pending_approval;
        let entry = self.build_audit_entry(
            caller,
            target_id,
            default_effect,
            "default_effect",
            None,
            None,
            approval_required,
            ctx,
        );
        self.emit_audit(&entry);
        AccessDecision::new(default_effect, approval_required, None, "default_effect")
    }

    /// Build an audit entry from the check parameters and context.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::unused_self)] // method must be on `&self` for trait-object dispatch consistency
    fn build_audit_entry(
        &self,
        caller_id: &str,
        target_id: &str,
        decision: &str,
        reason: &str,
        matched_rule_desc: Option<&str>,
        matched_rule_index: Option<usize>,
        approval_required: bool,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> AuditEntry {
        AuditEntry {
            approval_required,
            timestamp: Utc::now().to_rfc3339(),
            caller_id: caller_id.to_string(),
            target_id: target_id.to_string(),
            decision: decision.to_string(),
            reason: reason.to_string(),
            matched_rule: matched_rule_desc.map(std::string::ToString::to_string),
            matched_rule_index,
            identity_type: ctx
                .and_then(|c| c.identity.as_ref().map(|id| id.identity_type().to_string())),
            roles: ctx
                .and_then(|c| c.identity.as_ref().map(|id| id.roles().to_vec()))
                .unwrap_or_default(),
            call_depth: ctx.map(|c| c.call_chain.len()),
            trace_id: ctx.map(|c| c.trace_id.clone()),
            // Read the per-call handler-error slot populated by
            // `acl_handlers::report_handler_error`. Consults the async
            // task-local scope (from `async_check`) and the synchronous
            // thread-local scope (from `check`), so a handler error populates
            // the audit entry on BOTH paths. Returns `None` when no handler
            // reported an error or when built outside a capture scope. Mirrors
            // Python `_handler_error_var.get()` and TypeScript
            // `_lastHandlerError` (closes A-D-ACL-001 / A-D-002).
            handler_error: crate::acl_handlers::current_handler_error(),
        }
    }

    /// Async check whether the given caller is allowed to access the target.
    /// Uses first-match-wins evaluation with async condition handler support.
    ///
    /// Async entry point. Shares all audit construction with `check` via the
    /// `finalize_*` helpers so the two methods cannot drift on logging fields,
    /// reason strings, or default-effect mapping.
    pub async fn async_check(
        &self,
        caller_id: Option<&str>,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> bool {
        let decision = self
            .async_check_access(caller_id, target_id, ctx, None)
            .await;
        decision.is_allowed() && !decision.approval_required
    }

    /// Async counterpart to [`Self::check_access`] (PROTOCOL_SPEC §6.8.1).
    ///
    /// §6.8.1 requires the structured accessor on **both** paths: `check()`
    /// and `async_check()` resolve conditions from different registries
    /// (§6.1.3), so an implementation that offered it on one path only would
    /// leave the other unable to see an approval requirement at all.
    pub async fn async_check_access(
        &self,
        caller_id: Option<&str>,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
        projection: Option<&GovernanceProjection>,
    ) -> AccessDecision {
        // Wrap the entire evaluation in a per-call handler-error capture
        // scope so any handler that calls `report_handler_error(...)` lands
        // its message in this call's audit entry. The captured value is
        // read by `build_audit_entry` via the `HANDLER_ERROR` task-local
        // (closes A-D-ACL-001). The same scope carries the §6.1.8 projection.
        let (decision, _captured) = crate::acl_handlers::with_acl_evaluation_scope(
            projection.cloned().map(Arc::new),
            self.async_check_inner(caller_id, target_id, ctx),
        )
        .await;
        decision
    }

    async fn async_check_inner(
        &self,
        caller_id: Option<&str>,
        target_id: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> AccessDecision {
        let caller = caller_id.unwrap_or(EXTERNAL_CALLER);

        // Snapshot rules + default_effect at entry so any concurrent mutator
        // (e.g., another task calling add_rule/reload through an
        // Arc<RwLock<ACL>> wrapper) cannot cause TOCTOU drift mid-evaluation.
        // Mirrors the sync `check()` snapshot and apcore-python /
        // apcore-typescript async paths (sync finding A-D-301).
        let rules: Vec<ACLRule> = self.rules.clone();
        let default_effect: String = self.default_effect.clone();

        if rules.is_empty() {
            return self.finalize_no_rules(&default_effect, caller, target_id, ctx);
        }

        // §6.1.1 rule 5 — see the identical accumulator in `check_inner`. The
        // async path resolves conditions from a different registry (§6.1.3),
        // not by a different rule, so it must carry the pending requirement the
        // same way or the two paths answer the same ACL differently.
        let mut pending_approval = false;

        for (idx, rule) in rules.iter().enumerate() {
            let paths_before = crate::acl_handlers::reported_condition_paths();
            match self
                .matches_rule_async(idx, rule, caller, target_id, ctx)
                .await
            {
                RuleMatch::Match => {
                    return self.finalize_rule_match(
                        idx,
                        rule,
                        caller,
                        target_id,
                        ctx,
                        pending_approval,
                    )
                }
                RuleMatch::Unevaluable => {
                    match Self::resolve_unevaluable_rule(idx, rule, &paths_before) {
                        UnevaluableResolution::TakesEffect => {
                            return self.finalize_rule_match(
                                idx,
                                rule,
                                caller,
                                target_id,
                                ctx,
                                pending_approval,
                            )
                        }
                        UnevaluableResolution::StepsAside {
                            pending_approval: p,
                        } => {
                            pending_approval |= p;
                        }
                    }
                }
                RuleMatch::NoMatch => {}
            }
        }

        self.finalize_default_effect(&default_effect, caller, target_id, ctx, pending_approval)
    }

    /// Async version of `matches_rule` that awaits async condition handlers.
    /// Mirrors the sync `matches_rule` exactly except it routes condition
    /// evaluation through `check_conditions_async` so async handlers are awaited
    /// rather than polled-once.
    async fn matches_rule_async(
        &self,
        rule_index: usize,
        rule: &ACLRule,
        caller: &str,
        target: &str,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> RuleMatch {
        // §6.1.4.1, exactly as in the sync twin. The pattern precheck is
        // context-free and registry-free, so the two paths cannot legitimately
        // disagree about it — but they are separate code paths (#100), so the
        // call has to be written in both.
        if Self::pattern_precheck_failed(rule) {
            return RuleMatch::Unevaluable;
        }

        if !Self::match_patterns(&rule.callers, caller, ctx) {
            return RuleMatch::NoMatch;
        }

        if !Self::match_patterns(&rule.targets, target, ctx) {
            return RuleMatch::NoMatch;
        }

        if let Some(ref conditions) = rule.conditions {
            return match self
                .check_conditions_async(rule_index, rule, conditions, ctx)
                .await
            {
                ConditionOutcome::Satisfied => RuleMatch::Match,
                ConditionOutcome::Unsatisfied => RuleMatch::NoMatch,
                ConditionOutcome::Unevaluable => RuleMatch::Unevaluable,
            };
        }

        RuleMatch::Match
    }

    /// Emit an audit entry to the registered audit logger, if any.
    fn emit_audit(&self, entry: &AuditEntry) {
        // Through the SINK, not the raw callback: delivery has to be contained
        // (§6.3.2 requirement 3) and the failure state that scopes requirement
        // 5's suppression lives on the sink, so a bare `Fn` could honour
        // neither.
        if self.audit_sink.is_active() {
            self.audit_sink.deliver(entry);
        }
    }

    /// Initialize built-in handlers. Call once during application startup.
    pub fn init_builtin_handlers() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            register_builtin_handlers();
        });
    }
}

impl Default for ACL {
    fn default() -> Self {
        Self::new(vec![], "deny", None)
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

#[cfg(test)]
mod projection_forgery_tests {
    //! PROTOCOL_SPEC §6.1.8 rule 3 — the governance projection MUST NOT be
    //! accepted from caller-supplied input.
    //!
    //! Source guards rather than `trybuild` compile-fail fixtures: a `.stderr`
    //! for an unsatisfied serde trait bound is rustc-version sensitive, and CI
    //! runs a newer toolchain than the local default. These assert the same
    //! property and cannot go stale on a compiler upgrade.

    /// The `#[...]` attribute lines immediately preceding `decl` in `src`.
    ///
    /// Attribute lines only — doc comments are skipped, because this very
    /// type's own documentation discusses `Deserialize` at length and a naive
    /// text scan would match the prose instead of the derive.
    fn derives_before(src: &str, decl: &str) -> String {
        let at = src
            .find(decl)
            .unwrap_or_else(|| panic!("{decl} still exists"));
        src[..at]
            .lines()
            .rev()
            .take_while(|line| line.trim_start().starts_with("#["))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_projection_derives_no_serde_traits() {
        let attrs = derives_before(include_str!("acl.rs"), "pub struct GovernanceProjection {");
        assert!(
            !attrs.contains("Serialize") && !attrs.contains("Deserialize"),
            "GovernanceProjection must not be constructible from a wire payload \
             (PROTOCOL_SPEC §6.1.8 rule 3) — a caller that could supply its own \
             projection would satisfy `has_none_of` for a call whose arguments say \
             otherwise:\n{attrs}"
        );
    }

    #[test]
    fn the_wire_context_carries_no_projection() {
        // `Context` is deserialized from untrusted input (§4). The projection
        // lives on `PipelineContext`, which is framework-internal, precisely so
        // that no wire payload can name it.
        let src = include_str!("context.rs");
        let start = src
            .find("pub struct Context<T> {")
            .expect("Context still exists");
        let body_end = src[start..].find("\n}").expect("brace-terminated") + start;
        let body = &src[start..body_end];
        assert!(
            !body.contains("projection"),
            "the governance projection must not sit on the deserialized Context \
             (PROTOCOL_SPEC §6.1.8 rule 3); if it ever moves there the field MUST be \
             transient — `#[serde(skip)]`:\n{body}"
        );
    }
}

/// PROTOCOL_SPEC §6.1.4.1's **backstop** for a pattern field assigned onto an
/// already-constructed rule (§6.2.1, spec v1.31.0, apcore#112).
///
/// # Why these live in the crate and not in `tests/`
///
/// §6.2.1 closes every entry point that accepts a rule, and in this SDK that
/// leaves the backstop with no route in from outside: [`ACL::rules`] hands back
/// `&[ACLRule]`, and there is no `rules_mut`, no public field and no
/// `Deserialize` on [`ACL`], so a downstream crate cannot install a rule it
/// mutated after construction. In apcore-python and apcore-typescript
/// `acl.rules[0].targets = []` reaches the matcher directly, which is why the
/// conformance fixture carries the route as nine `kind: "backstop"` cases.
///
/// That asymmetry is a reason to test the backstop from inside the crate, not a
/// reason to leave it untested: `self.rules` is reachable from here, a future
/// `rules_mut` or in-place editing API would open the same route from outside,
/// and §6.1.4.1's classification has to already be correct when it does. These
/// mirror the fixture's backstop cases **by name**, and that is load-bearing:
/// `tests/test_acl_pattern_arity_conformance.rs` reports those seven cases as
/// satisfied by construction rather than skipped, and reads this file to check
/// that a test of the same name exists. Renaming one here fails the driver.
#[cfg(test)]
mod pattern_arity_backstop_tests {
    use super::*;
    use std::sync::Mutex;

    /// A one-rule ACL plus the audit entries it emits.
    ///
    /// Built through `new_unchecked` so the rule can be planted in the state a
    /// mutation would leave it in — every public door refuses it, which is the
    /// point of the closure and the reason this helper is private.
    fn planted(rule: ACLRule, default_effect: &str) -> (ACL, Arc<Mutex<Vec<AuditEntry>>>) {
        ACL::init_builtin_handlers();
        let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captured);
        let logger: Arc<AuditLoggerFn> = Arc::new(move |entry: &AuditEntry| {
            sink.lock().expect("audit sink").push(entry.clone());
        });
        let acl = ACL::new_unchecked(vec![rule], default_effect, Some(logger));
        (acl, captured)
    }

    fn base_rule(effect: &str) -> ACLRule {
        ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], effect)
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    /// The `handler_error` paths on the single audit entry the check emitted.
    ///
    /// `handler_error` is the joined `"{path}: {reason}"` form (§6.1.1 rule 2),
    /// already ordered lexicographically by path, so the paths are recovered by
    /// splitting rather than re-sorted — the order under test is the one the
    /// entry was written in.
    fn handler_error_paths(captured: &Arc<Mutex<Vec<AuditEntry>>>) -> Vec<String> {
        let entries = captured.lock().expect("audit sink");
        assert_eq!(entries.len(), 1, "exactly one audit entry per check()");
        let Some(joined) = entries[0].handler_error.as_ref() else {
            return Vec::new();
        };
        joined
            .split("; ")
            .map(|part| part.split(':').next().unwrap_or_default().to_string())
            .collect()
    }

    fn finding_paths(acl: &ACL) -> Vec<String> {
        acl.validate_rules()
            .into_iter()
            .map(|f| f.condition_path)
            .collect()
    }

    #[test]
    fn mutated_empty_targets_on_deny_rule_denies() {
        let mut rule = base_rule("deny");
        rule.targets = Vec::new();
        let (acl, captured) = planted(rule, "allow");

        let decision = acl.check_access(Some("api.gateway"), "cli.rm", None, None);
        assert_eq!(
            decision.access, "deny",
            "a rule whose scope cannot be read is UNEVALUABLE, and §6.1.1's effect table \
             makes a deny rule take effect — reading it as a non-match is #112's fail-open"
        );
        assert!(!decision.approval_required, "a denial carries no question");
        assert_eq!(handler_error_paths(&captured), vec!["targets".to_string()]);
        // The legacy boolean agrees, as it must whenever no approval is pending.
        assert!(!acl.check(Some("api.gateway"), "cli.rm", None));
        assert_eq!(finding_paths(&acl), vec!["targets".to_string()]);
    }

    #[test]
    fn mutated_empty_targets_on_allow_rule_does_not_grant() {
        let mut rule = base_rule("allow");
        rule.targets = Vec::new();
        let (acl, captured) = planted(rule, "deny");

        assert!(
            !acl.check(Some("api.gateway"), "cli.rm", None),
            "an unevaluable allow rule does not match and MUST NOT grant (§6.1.1)"
        );
        // The decision is unchanged from v1.30.0 — an inert `allow` rule also
        // failed to grant. The audit entry and the finding are what is new, and
        // they are what tells the operator the rule is broken rather than
        // merely unmatched.
        assert_eq!(handler_error_paths(&captured), vec!["targets".to_string()]);
        assert_eq!(finding_paths(&acl), vec!["targets".to_string()]);
    }

    #[test]
    fn mutated_or_with_no_operands_on_deny_rule_denies() {
        let mut rule = base_rule("deny");
        rule.targets = strings(&["$or"]);
        let (acl, captured) = planted(rule, "allow");

        assert!(
            !acl.check(Some("api.gateway"), "cli.rm", None),
            "an implementation that special-cases only the empty array fails here"
        );
        assert_eq!(handler_error_paths(&captured), vec!["targets".to_string()]);
        assert_eq!(finding_paths(&acl), vec!["targets".to_string()]);
    }

    #[test]
    fn mutated_not_with_no_operands_on_deny_rule_denies() {
        let mut rule = base_rule("deny");
        rule.callers = strings(&["$not"]);
        let (acl, captured) = planted(rule, "allow");

        assert!(
            !acl.check(Some("api.gateway"), "cli.rm", None),
            "the fault is per field — an implementation that checks only `targets` fails here"
        );
        assert_eq!(handler_error_paths(&captured), vec!["callers".to_string()]);
        assert_eq!(finding_paths(&acl), vec!["callers".to_string()]);
    }

    #[test]
    fn mutated_not_with_two_operands_on_allow_rule_does_not_grant() {
        let mut rule = base_rule("allow");
        rule.targets = strings(&["$not", "secrets.a", "secrets.b"]);
        let (acl, captured) = planted(rule, "deny");

        assert!(
            !acl.check(Some("api.gateway"), "secrets.b", None),
            "the escalation case at runtime: through v1.30.0 the matcher read the first \
             operand and dropped the rest, so the SECOND target the operator excluded was \
             GRANTED. A `true` here is the pre-v1.31.0 matcher"
        );
        assert_eq!(handler_error_paths(&captured), vec!["targets".to_string()]);
        assert_eq!(finding_paths(&acl), vec!["targets".to_string()]);
    }

    #[test]
    fn mutated_both_fields_report_both_paths() {
        let mut rule = base_rule("deny");
        rule.callers = strings(&["$not"]);
        rule.targets = Vec::new();
        let (acl, captured) = planted(rule, "allow");

        assert!(!acl.check(Some("api.gateway"), "cli.rm", None));
        // §6.1.4 rule 3: the precheck MUST NOT short-circuit, so both fields
        // are examined and both faults reported, ordered lexicographically by
        // path (§6.1.1 rule 2).
        assert_eq!(
            handler_error_paths(&captured),
            vec!["callers".to_string(), "targets".to_string()]
        );
        assert_eq!(
            finding_paths(&acl),
            vec!["callers".to_string(), "targets".to_string()]
        );
    }

    #[test]
    fn mutated_empty_targets_on_approval_rule_raises_pending_requirement() {
        let mut rule = base_rule("allow");
        rule.approval = Some(ApprovalRequirement::Required);
        rule.targets = Vec::new();
        let (acl, captured) = planted(rule, "allow");

        let decision = acl.check_access(Some("api.gateway"), "cli.rm", None, None);
        // §6.1.1 rule 5, "unknowable scope counts as scope": the rule's scope
        // cannot be read, so it cannot be shown not to apply here. An arity
        // fault is a malformed pattern field like any other — §6.1.4.1 has no
        // partially-readable tier, and inventing one would put back the
        // per-implementation judgement call that produced three answers in #100.
        assert_eq!(
            decision.access, "allow",
            "the grant comes from default_effect: allow"
        );
        assert!(
            decision.approval_required,
            "the pending requirement composes onto whatever grants next (§6.1.1 rule 5)"
        );
        assert_eq!(
            decision.matched_rule_index, None,
            "approval_required with no matched rule is the legal combination rule 5 names"
        );
        assert_eq!(handler_error_paths(&captured), vec!["targets".to_string()]);
        // §6.8.1: the legacy boolean `check()` folds the two axes into one and
        // fails closed on an outstanding approval requirement, so it answers
        // `false` here while `access` is `"allow"`. Asserted separately from
        // the structured accessor, because the two are different questions and
        // reading the boolean as the access decision would make this case look
        // like a denial.
        assert!(
            !acl.check(Some("api.gateway"), "cli.rm", None),
            "the boolean entry point fails closed while approval is pending (§6.8.1)"
        );
        assert_eq!(finding_paths(&acl), vec!["targets".to_string()]);
    }

    #[test]
    fn mutated_pattern_field_is_unevaluable_on_the_async_path_too() {
        // `matches_rule` and `matches_rule_async` are separate code paths, as
        // they were in #100. The precheck is context-free and registry-free, so
        // the two cannot legitimately disagree — which is exactly why a test
        // has to say so rather than the reader assuming it.
        let mut rule = base_rule("deny");
        rule.targets = Vec::new();
        let (acl, captured) = planted(rule, "allow");

        let allowed = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async { acl.async_check(Some("api.gateway"), "cli.rm", None).await });

        assert!(
            !allowed,
            "the async twin must classify the fault identically"
        );
        assert_eq!(handler_error_paths(&captured), vec!["targets".to_string()]);
    }

    /// Every clause of §6.2.1, in the vocabulary the fault messages are built
    /// from: the field, the array, and whether the array is legal.
    fn every_illegal_shape() -> Vec<(&'static str, Vec<String>)> {
        vec![
            ("callers", Vec::new()),
            ("targets", Vec::new()),
            ("targets", strings(&[""])),
            ("callers", strings(&["$or", ""])),
            ("targets", strings(&["$or"])),
            ("callers", strings(&["$not"])),
            ("targets", strings(&["$not", "a", "b"])),
            ("callers", strings(&["$or", "$not", "admin"])),
            ("targets", strings(&["api.*", "$not", "cli.*"])),
        ]
    }

    #[test]
    fn no_fault_message_contains_the_handler_error_separator() {
        // §6.1.1 rule 2 joins several `handler_error` entries with "; ", and a
        // reader — a driver, or the test helper above — recovers the reported
        // paths by splitting on it. A message carrying the separator splits
        // into two bogus paths, and the resulting failure reads as a
        // short-circuit bug rather than as the message-format bug it is.
        for (field, patterns) in every_illegal_shape() {
            let reason = pattern_shape_fault(field, &patterns)
                .unwrap_or_else(|| panic!("{field}: {patterns:?} is illegal under §6.2.1"));
            assert!(
                !reason.contains("; "),
                "the tier-1 message for {field}: {patterns:?} carries the handler_error \
                 separator: {reason}"
            );
        }
        for (field, patterns) in [
            ("targets", strings(&["$not", "*"])),
            ("targets", strings(&["$not", "**"])),
            ("targets", strings(&["@external"])),
        ] {
            let reason = pattern_never_matches(field, &patterns)
                .unwrap_or_else(|| panic!("{field}: {patterns:?} matches nothing"));
            assert!(
                !reason.contains("; "),
                "the tier-2 message for {field}: {patterns:?} carries the handler_error \
                 separator: {reason}"
            );
        }
    }

    #[test]
    fn every_illegal_shape_is_reported_on_both_fields() {
        // §6.2.1 constrains `callers` and `targets` identically. The predicate
        // takes the field name only to name it in the message, so a shape that
        // is a fault on one field must be a fault on the other — asserted
        // rather than assumed, because an implementation that checked one field
        // and inferred the other is the defect the fixture's `*_in_callers_*`
        // mirrors exist to catch.
        for (_, patterns) in every_illegal_shape() {
            for field in ["callers", "targets"] {
                assert!(
                    pattern_shape_fault(field, &patterns).is_some(),
                    "{patterns:?} is illegal under §6.2.1 but was accepted on '{field}'"
                );
            }
        }
    }

    #[test]
    fn a_well_formed_planted_rule_raises_no_finding() {
        // The control without which an implementation that flags every rule
        // passes every other test in this module.
        let mut rule = base_rule("deny");
        rule.targets = strings(&["cli.*"]);
        let (acl, captured) = planted(rule, "allow");

        assert!(!acl.check(Some("api.gateway"), "cli.rm", None));
        assert_eq!(handler_error_paths(&captured), Vec::<String>::new());
        assert_eq!(finding_paths(&acl), Vec::<String>::new());
    }
}

#[cfg(test)]
mod direct_audit_config_tests {
    //! PROTOCOL_SPEC §6.3.2 requirement 2 — an ACL built in code must be able
    //! to declare an `audit:` block, the way apcore-python and
    //! apcore-typescript accept one as a constructor parameter. Only
    //! `load` / `reload` could populate it here, and `new` / `try_new` hard-coded
    //! `None`.
    use super::*;

    #[test]
    fn with_audit_config_sets_the_block_and_rebuilds_the_sink() {
        let acl = ACL::new(vec![], "deny", None);
        assert!(acl.audit_config.is_none(), "no block by default");
        assert!(
            !acl.audit_sink.is_active(),
            "without a callback or a declared block there is no sink"
        );

        let config = AuditConfig {
            enabled: true,
            include_denied: false,
            log_level: "debug".to_string(),
        };
        let acl = acl.with_audit_config(Some(config.clone()));
        assert_eq!(acl.audit_config.as_ref(), Some(&config));
        assert!(
            acl.audit_sink.is_active(),
            "the sink must be rebuilt from the new block, not left on the old one"
        );
    }

    #[test]
    fn set_audit_config_none_clears_the_block() {
        let mut acl =
            ACL::new(vec![], "deny", None).with_audit_config(Some(AuditConfig::default()));
        assert!(acl.audit_sink.is_active());

        acl.set_audit_config(None);
        assert!(acl.audit_config.is_none());
        assert!(!acl.audit_sink.is_active());
    }

    #[test]
    fn a_declared_block_that_is_disabled_leaves_the_sink_inactive() {
        // Requirement 2: DECLARATION activates the default sink, never the
        // default value — a declared `enabled: false` stays off.
        let acl = ACL::new(vec![], "deny", None).with_audit_config(Some(AuditConfig {
            enabled: false,
            ..AuditConfig::default()
        }));
        assert!(!acl.audit_sink.is_active());
    }
}
