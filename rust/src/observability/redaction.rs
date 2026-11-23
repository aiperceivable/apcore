// APCore Protocol — Configurable redaction rules
// Spec reference: observability.md §1.5 Configurable Redaction Rules

use std::sync::atomic::{AtomicBool, Ordering};

use regex::{Regex, RegexBuilder};
use serde_json::{Map, Value};

use crate::config::Config;
use crate::utils::helpers::match_glob;

/// Default replacement string for redacted values.
pub const DEFAULT_REPLACEMENT: &str = "***REDACTED***";

/// Field names that MUST NEVER be redacted (observability.md §1.5, D-54).
///
/// Correlation identifiers `trace_id`, `caller_id`, `target_id`, `module_id`,
/// and `span_id` MUST always appear in logs unmodified, regardless of any
/// `sensitive_keys` content.
pub const NEVER_REDACT_FIELDS: &[&str] =
    &["trace_id", "caller_id", "target_id", "module_id", "span_id"];

/// Default sensitive-key glob patterns applied when the operator has not
/// supplied `obs.redaction.sensitive_keys` in their Config (D-54, Issue #43 §5).
///
/// An operator-supplied list **replaces** this one; it does not merge
/// (`features/observability.md`, "Canonical default `sensitive_keys`": "the
/// override **replaces** the default; it does not merge"). Both peers were read
/// to confirm the claim rather than assumed: apcore-python passes
/// `_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS` to `config.get` as the FALLBACK
/// argument only (`observability/context_logger.py`,
/// `RedactionConfig.from_config`), and apcore-typescript spreads
/// `DEFAULT_REDACTION_FIELD_PATTERNS` only when the configured value is not an
/// array (`observability/context-logger.ts`, `RedactionConfig.fromConfig`).
/// Neither merges the operator's entries into the defaults.
///
/// Includes snake_case, camelCase, and lowercase parity (`api_key` / `apiKey` /
/// `apikey`) so common framework conventions are covered out of the box.
pub const DEFAULT_SENSITIVE_KEYS: &[&str] = &[
    "_secret_*",
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "apiKey",
    "access_key",
    "private_key",
    "authorization",
    "auth",
    "credential",
    "cookie",
    "session",
    "bearer",
];

/// Canonical `RedactionConfig` config keys (D-53). `features/observability.md`
/// ("Canonical Config keys (cross-SDK)") requires all three SDKs to read these
/// and calls any divergence a conformance bug: apcore-python reads exactly
/// these, apcore-typescript reads them first.
const CANONICAL_SENSITIVE_KEYS: &str = "obs.redaction.sensitive_keys";
const CANONICAL_REGEX_PATTERNS: &str = "obs.redaction.regex_patterns";
const CANONICAL_REPLACEMENT: &str = "obs.redaction.replacement";

/// The keys this SDK read before the canonical namespace was honoured. Still
/// accepted so existing apcore-rust deployments keep working, but reading one
/// emits a one-shot deprecation warning. (apcore-typescript keeps its OWN
/// pre-D-53 spellings — `observability.redaction.field_patterns` /
/// `.value_patterns` — which this SDK never read and therefore does not adopt.)
const LEGACY_SENSITIVE_KEYS: &str = "observability.redaction.sensitive_keys";
const LEGACY_REGEX_PATTERNS: &str = "observability.redaction.regex_patterns";
const LEGACY_REPLACEMENT: &str = "observability.redaction.replacement";

/// One-shot bookkeeping for the legacy-key deprecation warning.
static LEGACY_REDACTION_KEY_WARNING_EMITTED: AtomicBool = AtomicBool::new(false);

/// Warn once per process that deprecated `observability.redaction.*` keys were
/// read, naming the keys actually used and their canonical replacements.
/// Mirrors apcore-typescript's `_emitRedactionLegacyDeprecation`, which is
/// likewise one-shot per process.
fn emit_legacy_redaction_key_deprecation(legacy_keys: &[&str]) {
    if LEGACY_REDACTION_KEY_WARNING_EMITTED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::warn!(
        "[apcore] Config keys {} are deprecated; use {CANONICAL_SENSITIVE_KEYS} / \
         {CANONICAL_REGEX_PATTERNS} / {CANONICAL_REPLACEMENT} instead. Legacy keys \
         will be removed in a future release.",
        legacy_keys.join(", ")
    );
}

/// Read one redaction setting: canonical key first, deprecated key second.
///
/// Absent and explicitly `null` both count as "not configured", so a `null`
/// written against the canonical key falls through to the legacy key and then
/// to the built-in default instead of being mistaken for a configured value.
/// Matches apcore-typescript (`undefined || null` -> fall through) and
/// apcore-python (which coerces a `None` back to the default list).
///
/// Every legacy key actually consulted is appended to `legacy_used` so the
/// caller can emit one warning naming all of them, rather than one per key.
fn read_redaction_key(
    config: &Config,
    canonical: &str,
    legacy: &'static str,
    legacy_used: &mut Vec<&'static str>,
) -> Option<Value> {
    // Registered namespace defaults are effective values, not an operator's
    // canonical setting. A legacy value must still override those defaults;
    // otherwise a real legacy-only config is silently ignored once `obs` is
    // registered at bootstrap.
    if let Some(value) = config.get_declared(canonical) {
        if !value.is_null() {
            return Some(value);
        }
    }
    match config.get(legacy) {
        Some(value) if !value.is_null() => {
            legacy_used.push(legacy);
            Some(value)
        }
        _ => None,
    }
}

/// Runtime-configurable redaction rules layered on top of schema-level
/// (`x-sensitive`) annotations.
#[derive(Debug, Clone, Default)]
pub struct RedactionConfig {
    /// Glob-dialect entries (those containing `*` or `?`), LOWER-CASED.
    ///
    /// PROTOCOL_SPEC 10.6.1: they are matched with Algorithm A25 against the
    /// lower-cased field name, so the case fold applies to the pattern AND the
    /// name alike. Storing them folded is what makes that structural rather
    /// than a call-site convention: the previous code lowered only the name,
    /// so `"*Token*"` — or any capitalised spelling — was a perfectly valid
    /// pattern that matched nothing, redacting in Python and TypeScript and
    /// leaving PLAINTEXT here, with no warning (#117 section 1).
    field_patterns: Vec<String>,
    /// Plain substring patterns, pre-normalized via [`normalize_key_for_match`].
    field_substrings: Vec<String>,
    value_patterns: Vec<Regex>,
    replacement: String,
}

/// Normalize a key or pattern for cross-separator, case-insensitive substring
/// matching (D-54). Lower-cases the input, treats `-` / `_` / whitespace as
/// equivalent, and inserts an `_` at every `lowercase->uppercase` boundary so
/// camelCase keys like `AccessKey` match the `access_key` substring pattern.
#[must_use]
pub fn normalize_key_for_match(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_lower = false;
    for ch in s.chars() {
        if ch.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        out.push(ch);
        prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    out.to_lowercase()
        .replace('-', "_")
        .replace(char::is_whitespace, "_")
}

impl RedactionConfig {
    /// Create an empty redaction config (no rules, default replacement).
    #[must_use]
    pub fn new() -> Self {
        Self {
            field_patterns: Vec::new(),
            field_substrings: Vec::new(),
            value_patterns: Vec::new(),
            replacement: DEFAULT_REPLACEMENT.to_string(),
        }
    }

    /// Construct a redaction config seeded with the canonical default
    /// `sensitive_keys` list (D-54). The 16-entry list is the spec-defined
    /// superset shipped by every SDK when no operator override is supplied.
    /// Operator overrides MUST replace this list, not merge.
    #[must_use]
    pub fn with_default_sensitive_keys() -> Self {
        let mut cfg = Self::new();
        for key in DEFAULT_SENSITIVE_KEYS {
            cfg.add_sensitive_key(key);
        }
        cfg
    }

    /// Construct a redaction config with the canonical default sensitive_keys
    /// list. Equivalent to [`Self::with_default_sensitive_keys`], and the
    /// counterpart of apcore-python's `RedactionConfig.default()` classmethod.
    ///
    /// Note the one-letter trap: the DERIVED [`Default`] impl
    /// (`RedactionConfig::default()`, no `s`) yields zero rules and an empty
    /// replacement string, not this policy.
    #[must_use]
    pub fn defaults() -> Self {
        Self::with_default_sensitive_keys()
    }

    /// Append one entry to the matcher (PROTOCOL_SPEC 10.6.1).
    ///
    /// An entry containing `*` or `?` is a glob-dialect pattern stored
    /// lower-cased and matched with A25; anything else is a substring stored
    /// pre-normalized. `[` is not a trigger and a pattern can never fail.
    fn add_sensitive_key(&mut self, key: &str) {
        if key.is_empty() {
            return;
        }
        // PROTOCOL_SPEC 10.6.1: `[` is NOT a trigger. Brackets are literals
        // under A25 (9.2.3 requirement 4), so `[!p]assword` carries no `*` and
        // no `?` and is an ordinary substring — inert, rather than read as a
        // negated character class the way `glob::Pattern` read it. There is
        // also nothing left to fail: A25 has no parse phase, so an entry can
        // no longer be silently dropped as an "invalid glob".
        if key.contains(['*', '?']) {
            self.field_patterns.push(key.to_lowercase());
        } else {
            self.field_substrings.push(normalize_key_for_match(key));
        }
    }

    /// Builder entry point.
    #[must_use]
    pub fn builder() -> RedactionConfigBuilder {
        RedactionConfigBuilder::default()
    }

    /// Build a redaction config from the canonical `obs.redaction.*` keys of a
    /// loaded [`Config`] (D-53), falling back to the deprecated
    /// `observability.redaction.*` keys this SDK read before that alignment.
    ///
    /// Reads, in order of precedence:
    ///   1. `obs.redaction.sensitive_keys: Vec<String>` — glob/substring
    ///      patterns applied to field NAMES.
    ///      `obs.redaction.regex_patterns: Vec<String>` — regular expressions
    ///      applied to string VALUES, compiled case-insensitively.
    ///      `obs.redaction.replacement: String` — the substitution string
    ///      (defaults to [`DEFAULT_REPLACEMENT`]).
    ///   2. The same three settings under `observability.redaction.*`, which
    ///      are **deprecated**. Reading any of them emits a one-shot `warn`
    ///      naming the canonical replacements, as apcore-typescript does.
    ///
    /// `sensitive_keys` semantics (**D-54**): an operator-supplied list
    /// **replaces** [`DEFAULT_SENSITIVE_KEYS`] (`_secret_*`, `password`,
    /// `passwd`, `secret`, `token`, `api_key`, `apikey`, `apiKey`,
    /// `access_key`, `private_key`, `authorization`, `auth`, `credential`,
    /// `cookie`, `session`, `bearer`); it does not merge with them, so an
    /// operator can narrow the list and not only widen it. "Not supplied"
    /// means the key is absent or explicitly `null`. An explicitly EMPTY list
    /// is an override like any other — it disables key-based redaction
    /// entirely, including the `_secret_*` prefix, which is entry `[0]` of the
    /// default list and not a separate hardcoded rule. It MUST NOT be re-read
    /// as "unset" (matches apcore-python and apcore-typescript). The
    /// value-regex rule is independent and still applies.
    ///
    /// Malformed patterns are logged at `warn` and skipped — a typo in one
    /// rule MUST NOT disable redaction for every other rule.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let mut cfg = Self::new();
        let mut legacy_used: Vec<&'static str> = Vec::new();

        let sensitive_keys = read_redaction_key(
            config,
            CANONICAL_SENSITIVE_KEYS,
            LEGACY_SENSITIVE_KEYS,
            &mut legacy_used,
        );
        let regex_patterns = read_redaction_key(
            config,
            CANONICAL_REGEX_PATTERNS,
            LEGACY_REGEX_PATTERNS,
            &mut legacy_used,
        );
        let replacement = read_redaction_key(
            config,
            CANONICAL_REPLACEMENT,
            LEGACY_REPLACEMENT,
            &mut legacy_used,
        );

        if !legacy_used.is_empty() {
            emit_legacy_redaction_key_deprecation(&legacy_used);
        }

        // D-54: the operator's list REPLACES the default rather than being
        // unioned with it. Only an unconfigured key (absent or null) — or a
        // value that is not a list at all — falls back to the canonical
        // defaults; a configured empty list means "redact nothing by name".
        match sensitive_keys.as_ref().and_then(Value::as_array) {
            Some(entries) => {
                for entry in entries {
                    if let Some(s) = entry.as_str() {
                        cfg.add_sensitive_key(s);
                    }
                }
            }
            None => {
                for default in DEFAULT_SENSITIVE_KEYS {
                    cfg.add_sensitive_key(default);
                }
            }
        }

        let mut value_patterns: Vec<Regex> = Vec::new();
        if let Some(entries) = regex_patterns.as_ref().and_then(Value::as_array) {
            for entry in entries {
                if let Some(s) = entry.as_str() {
                    match RegexBuilder::new(s).case_insensitive(true).build() {
                        Ok(r) => value_patterns.push(r),
                        Err(e) => tracing::warn!(
                            pattern = %s,
                            error = %e,
                            "Skipping invalid regex_patterns entry"
                        ),
                    }
                }
            }
        }

        cfg.value_patterns = value_patterns;
        cfg.replacement = replacement
            .as_ref()
            .and_then(|v| v.as_str())
            .map_or_else(|| DEFAULT_REPLACEMENT.to_string(), str::to_owned);
        cfg
    }

    /// Apply both field-name and value-pattern rules to a JSON value in place.
    /// Fields named in [`NEVER_REDACT_FIELDS`] are excluded.
    pub fn redact(&self, value: &mut Value) {
        self.redact_inner(value, /*field_name=*/ None);
    }

    fn redact_inner(&self, value: &mut Value, field_name: Option<&str>) {
        // Correlation identifiers are exempt from BOTH redaction rules, not
        // just the field-name one: observability.md ("Implementations MUST NOT
        // redact `trace_id`, `caller_id`, `module_id`, or `span_id`; these
        // correlation fields MUST appear unmodified in every log entry") states
        // the exemption unconditionally. apcore-typescript checks
        // `PROTECTED_LOG_FIELDS` at the top of `_shouldRedact` and
        // apcore-python does the same at the top of `_apply_redaction_config`,
        // i.e. before either the name rule or the value regex is evaluated.
        // Guarding only the name rule made a `trace_id` whose VALUE happened to
        // match a secret regex get redacted in Rust but not in Python/TS,
        // breaking trace correlation exactly where it matters most.
        let protected = field_name.is_some_and(|name| NEVER_REDACT_FIELDS.contains(&name));

        // Field-name rule applies based on the field this value is bound to.
        if let Some(name) = field_name {
            if !protected && self.field_matches(name) {
                *value = Value::String(self.replacement.clone());
                return;
            }
        }

        match value {
            Value::Object(map) => {
                self.redact_object(map);
            }
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    self.redact_inner(item, None);
                }
            }
            // Value rule. Containers below a protected key are still descended
            // into (matching TS, which recurses through `redact` after
            // `_shouldRedact` returns false) — only the protected field's own
            // scalar value is immune.
            Value::String(s) if !protected && self.value_matches(s) => {
                s.clone_from(&self.replacement);
            }
            _ => {}
        }
    }

    fn redact_object(&self, map: &mut Map<String, Value>) {
        // `iter_mut` hands out the key and its own value slot as independent
        // borrows, so the per-entry decision lives entirely in `redact_inner`
        // and the protected-field guard is encoded in exactly one place.
        for (key, slot) in map.iter_mut() {
            self.redact_inner(slot, Some(key.as_str()));
        }
    }

    /// Check whether a field name matches any sensitive-key entry.
    ///
    /// Glob entries are matched with Algorithm A25 against the lower-cased
    /// name, with the pattern lower-cased at construction so the fold applies
    /// to both sides (PROTOCOL_SPEC 10.6.1 clause 1). Bare substrings are
    /// evaluated against the normalized form (lower-cased, `-`/whitespace
    /// mapped to `_`, and an `_` inserted at every `lowercase->uppercase`
    /// boundary so `AccessKey` matches `access_key`).
    #[must_use]
    pub fn field_matches(&self, name: &str) -> bool {
        // Both sides are folded: the patterns at construction, the name here.
        // Folding only the name is a silent bypass on a redaction surface —
        // see the field's own documentation.
        let lowered = name.to_lowercase();
        if self.field_patterns.iter().any(|p| match_glob(p, &lowered)) {
            return true;
        }
        if self.field_substrings.is_empty() {
            return false;
        }
        let normalized = normalize_key_for_match(name);
        self.field_substrings
            .iter()
            .any(|sub| normalized.contains(sub))
    }

    /// Check whether a value matches any value regex.
    #[must_use]
    pub fn value_matches(&self, value: &str) -> bool {
        self.value_patterns.iter().any(|r| r.is_match(value))
    }

    /// The configured replacement string.
    #[must_use]
    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

/// Builder for [`RedactionConfig`]. Returns errors for malformed patterns.
#[derive(Debug, Default)]
pub struct RedactionConfigBuilder {
    field_patterns: Vec<String>,
    sensitive_keys: Vec<String>,
    value_patterns: Vec<String>,
    replacement: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RedactionConfigError {
    // `InvalidFieldPattern` was removed in v1.37.0: PROTOCOL_SPEC 9.2.3
    // requirement 2 makes every string a valid A25 pattern, so a field pattern
    // has no failure mode left. The variant it replaced existed only because
    // `glob::Pattern` rejects `a[b` and `a**b`.
    #[error("invalid value regex '{pattern}': {source}")]
    InvalidValuePattern {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

impl RedactionConfigBuilder {
    #[must_use]
    pub fn field_patterns<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.field_patterns
            .extend(patterns.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn value_patterns<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.value_patterns
            .extend(patterns.into_iter().map(Into::into));
        self
    }

    /// Append `sensitive_keys` entries (D-54). Each entry is auto-detected:
    /// strings containing `*` or `?` are glob-dialect patterns (A25); bare
    /// strings become case-insensitive substring matchers (with separator +
    /// camelCase normalization). Operator-supplied lists MUST replace the
    /// canonical default — call [`RedactionConfig::with_default_sensitive_keys`]
    /// if you want both.
    #[must_use]
    pub fn sensitive_keys<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.sensitive_keys.extend(keys.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn replacement(mut self, replacement: impl Into<String>) -> Self {
        self.replacement = Some(replacement.into());
        self
    }

    /// Compile the value regexes. Returns an error if any is malformed.
    ///
    /// Field patterns can no longer fail: PROTOCOL_SPEC 9.2.3 requirement 2
    /// makes every string a valid A25 pattern, so there is nothing left to
    /// reject. They are lower-cased here for the same reason
    /// [`RedactionConfig::add_sensitive_key`] does — the case fold has to
    /// reach the pattern, not only the field name.
    pub fn try_build(self) -> Result<RedactionConfig, RedactionConfigError> {
        let field_patterns: Vec<String> = self
            .field_patterns
            .into_iter()
            .map(|p| p.to_lowercase())
            .collect();
        let value_patterns = self
            .value_patterns
            .into_iter()
            .map(|p| {
                // Case-insensitive, as `from_config` and the other two SDKs
                // compile `regex_patterns` — the same pattern must not redact
                // `SECRET-1` through one constructor and leak it through the other.
                RegexBuilder::new(&p)
                    .case_insensitive(true)
                    .build()
                    .map_err(|source| RedactionConfigError::InvalidValuePattern {
                        pattern: p,
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut cfg = RedactionConfig {
            field_patterns,
            field_substrings: Vec::new(),
            value_patterns,
            replacement: self
                .replacement
                .unwrap_or_else(|| DEFAULT_REPLACEMENT.to_string()),
        };
        for key in self.sensitive_keys {
            cfg.add_sensitive_key(&key);
        }
        Ok(cfg)
    }

    /// Build, panicking on invalid patterns. Prefer [`Self::try_build`] when
    /// patterns come from external configuration.
    #[must_use]
    pub fn build(self) -> RedactionConfig {
        self.try_build().expect("invalid redaction pattern")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::config::Config;
    use serde_json::json;

    /// Serializes the tests that reach into the process-global one-shot flag.
    /// They reset it, so running two of them concurrently would let one consume
    /// the other's warning.
    static LEGACY_WARNING_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Run `f` with a thread-local subscriber and return everything it logged.
    fn capture_logs(f: impl FnOnce()) -> String {
        let buf = CaptureWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = buf.0.lock().unwrap().clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn field_glob_redacts_matching_field() {
        let cfg = RedactionConfig::builder()
            .field_patterns(["*password*"])
            .build();
        let mut payload = json!({
            "username": "alice",
            "user_password": "hunter2",
        });
        cfg.redact(&mut payload);
        assert_eq!(payload["username"], json!("alice"));
        assert_eq!(payload["user_password"], json!(DEFAULT_REPLACEMENT));
    }

    #[test]
    fn value_regex_redacts_matching_value() {
        let cfg = RedactionConfig::builder()
            .value_patterns([r"^Bearer .*"])
            .build();
        let mut payload = json!({
            "url": "https://api.example.com/data",
            "authorization": "Bearer abc123xyz",
        });
        cfg.redact(&mut payload);
        assert_eq!(payload["authorization"], json!(DEFAULT_REPLACEMENT));
        assert_eq!(payload["url"], json!("https://api.example.com/data"));
    }

    #[test]
    fn never_redact_fields_are_preserved() {
        let cfg = RedactionConfig::builder()
            .field_patterns(["*"]) // would otherwise match everything
            .build();
        let mut payload = json!({
            "trace_id": "trace-1",
            "caller_id": "api.gateway",
            "module_id": "executor.auth",
            "other_field": "secret",
        });
        cfg.redact(&mut payload);
        assert_eq!(payload["trace_id"], json!("trace-1"));
        assert_eq!(payload["caller_id"], json!("api.gateway"));
        assert_eq!(payload["module_id"], json!("executor.auth"));
        assert_eq!(payload["other_field"], json!(DEFAULT_REPLACEMENT));
    }

    #[test]
    fn invalid_regex_returns_error() {
        let result = RedactionConfig::builder()
            .value_patterns(["[invalid"])
            .try_build();
        assert!(result.is_err());
    }

    /// Issue #32. Reading a deprecated `observability.redaction.*` key MUST
    /// still work and MUST warn, naming both the key read and its canonical
    /// replacement — the operator otherwise has no signal that they are on a
    /// key scheduled for removal. One-shot per process, as apcore-typescript's
    /// `_emitRedactionLegacyDeprecation` is.
    ///
    /// Lives here rather than in `tests/` because only this module can reset
    /// the one-shot flag; an integration test could observe the warning at most
    /// once per binary, whichever test happened to run first.
    #[test]
    fn legacy_redaction_keys_emit_a_one_shot_deprecation_warning() {
        let _guard = LEGACY_WARNING_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        LEGACY_REDACTION_KEY_WARNING_EMITTED.store(false, Ordering::Relaxed);

        let mut config = Config::from_defaults();
        config.set(LEGACY_SENSITIVE_KEYS, json!(["legacy_key"]));

        let logs = capture_logs(|| {
            let cfg = RedactionConfig::from_config(&config);
            assert!(
                cfg.field_matches("legacy_key"),
                "the deprecated key must still be applied, not merely warned about"
            );
        });
        assert!(
            logs.contains(LEGACY_SENSITIVE_KEYS),
            "the warning must name the deprecated key that was read: {logs}"
        );
        assert!(
            logs.contains(CANONICAL_SENSITIVE_KEYS),
            "the warning must name the canonical replacement: {logs}"
        );

        let again = capture_logs(|| {
            let _ = RedactionConfig::from_config(&config);
        });
        assert!(
            !again.contains("deprecated"),
            "the deprecation warning is one-shot per process: {again}"
        );
    }

    /// The canonical keys are not deprecated, so using them must stay silent —
    /// a warning fired on the documented path would train operators to ignore
    /// it.
    #[test]
    fn canonical_redaction_keys_emit_no_deprecation_warning() {
        let _guard = LEGACY_WARNING_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        LEGACY_REDACTION_KEY_WARNING_EMITTED.store(false, Ordering::Relaxed);

        let mut config = Config::from_defaults();
        config.set(CANONICAL_SENSITIVE_KEYS, json!(["canonical_key"]));

        let logs = capture_logs(|| {
            let cfg = RedactionConfig::from_config(&config);
            assert!(cfg.field_matches("canonical_key"));
        });
        assert!(
            !logs.contains("deprecated"),
            "the canonical key path must not warn: {logs}"
        );
    }
}
