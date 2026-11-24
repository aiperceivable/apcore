// APCore Protocol — Helper utilities
// Spec reference: Pattern matching, call chain guards, error propagation

use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};

/// Default maximum call chain depth before `CallDepthExceeded` is returned.
pub const DEFAULT_MAX_CALL_DEPTH: usize = 32;
/// Default maximum repeat count for a single module in the call chain.
pub const DEFAULT_MAX_MODULE_REPEAT: usize = 3;

/// Match a **module ID** against an ACL pattern (Algorithm A08).
///
/// `*` is the ONLY metacharacter; every other character is a literal, `?`,
/// `[`, `]`, `{`, `}` and `\` included. Used by ACL rule matching and by
/// pipeline `match_modules`, and by nothing else.
///
/// This is deliberately NOT [`match_glob`] (Algorithm A25), which is the
/// matcher for every other pattern-valued value in the specification and which
/// also honours `?`. PROTOCOL_SPEC §9.2.3 requirement 5 gives the reason:
/// promoting `?` here would widen ACL `allow` rules that are inert today
/// (§2.7 forbids `?` in a module ID), which is the one direction an
/// authorization matcher must not move silently. §6.2.2 closes that hole with
/// a diagnostic instead.
///
/// Ported from apcore-python `utils/pattern.py::match_pattern`.
#[must_use]
pub fn match_pattern(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == value;
    }

    let segments: Vec<&str> = pattern.split('*').collect();
    let mut pos: usize = 0;

    // If pattern does not start with '*', value must start with the first segment.
    if !pattern.starts_with('*') {
        if !value.starts_with(segments[0]) {
            return false;
        }
        pos = segments[0].len();
    }

    // Check each interior segment can be found in order.
    for segment in &segments[1..] {
        if segment.is_empty() {
            continue;
        }
        match value[pos..].find(segment) {
            Some(idx) => {
                pos += idx + segment.len();
            }
            None => return false,
        }
    }

    // If pattern does not end with '*', value must end with the last segment.
    pattern.ends_with('*') || value.ends_with(segments[segments.len() - 1])
}

/// Match `value` against a glob-dialect `pattern` (Algorithm A25).
///
/// PROTOCOL_SPEC §9.2.3. The matcher for every glob-dialect pattern-valued
/// value in the specification: `bindings.pattern`,
/// `obs.redaction.sensitive_keys` glob entries, event `event_pattern` /
/// `include_events` / `exclude_events`, and `path_filter`.
///
/// Exactly two metacharacters:
///
/// - `*` — zero or more characters, crossing `.` and `/`
/// - `?` — exactly one character
///
/// **Every other character is a literal**, `[`, `]`, `{`, `}`, `\`, `!`, `^`
/// and `-` included. There is no escape character, and the match is anchored
/// to the whole value.
///
/// **Do not delegate to `glob::Pattern`.** It reads `[…]` as a character class
/// with its own negation spelling, and — the part that bit hardest — it
/// *rejects* patterns: `a[b` is an "invalid range pattern" and `a**b` a
/// misplaced recursive wildcard, so a control-plane request the other two SDKs
/// served was refused here (#117). A25 has no parse phase: every string is a
/// valid pattern and this function never fails.
///
/// Operates on `char`s, not bytes, so `?` matches one character rather than
/// one UTF-8 byte.
#[must_use]
pub fn match_glob(pattern: &str, value: &str) -> bool {
    let segments: Vec<Vec<char>> = pattern.split('*').map(|s| s.chars().collect()).collect();
    let value: Vec<char> = value.chars().collect();

    if segments.len() == 1 {
        return match_exact(&segments[0], &value);
    }

    if !match_prefix(&segments[0], &value) {
        return false;
    }
    let mut pos = segments[0].len();

    for segment in &segments[1..segments.len() - 1] {
        if segment.is_empty() {
            continue;
        }
        if segment.len() > value.len() {
            return false;
        }
        let mut found = None;
        for j in pos..=(value.len() - segment.len()) {
            if match_exact(segment, &value[j..j + segment.len()]) {
                found = Some(j);
                break;
            }
        }
        match found {
            Some(j) => pos = j + segment.len(),
            None => return false,
        }
    }

    let last = &segments[segments.len() - 1];
    if last.is_empty() {
        return true;
    }
    if value.len() - pos < last.len() {
        return false;
    }
    match_exact(last, &value[value.len() - last.len()..])
}

/// True when `text` starts with `segment`, treating `?` as any character.
fn match_prefix(segment: &[char], text: &[char]) -> bool {
    if text.len() < segment.len() {
        return false;
    }
    segment
        .iter()
        .zip(text.iter())
        .all(|(s, t)| *s == '?' || s == t)
}

/// True when `segment` covers `text` exactly, treating `?` as any character.
fn match_exact(segment: &[char], text: &[char]) -> bool {
    segment.len() == text.len() && match_prefix(segment, text)
}

/// Guard against call depth, frequency, and circular call violations
/// (Algorithm A20).
///
/// This is the signature `call-chain-guard.md` publishes for Rust and is
/// normative since spec v1.49.0 (D-83). It takes the chain directly and is
/// free of the `Context<T>` type parameter, so a host whose services type is
/// not `serde_json::Value` can call it — the crate previously exposed only a
/// `&Context<serde_json::Value>` form, which meant such a host could not reach
/// the guard at all and ran nested calls with no depth, cycle or frequency
/// enforcement.
///
/// The cross-language canonical contract (matching apcore-python
/// `utils/call_chain.py` and apcore-typescript) is that `call_chain` ALREADY
/// includes `module_id` at the end (appended by
/// [`Context::child`](crate::context::Context::child) before this guard runs).
/// The three checks run in order:
///
/// 1. **Depth** — `len(call_chain) > max_call_depth` → `CallDepthExceeded`.
/// 2. **Circular** — strip the trailing self-entry, then if `module_id`
///    appears in the prior chain forming a cycle of length >= 2 →
///    `CircularCall`.
/// 3. **Frequency** — count occurrences of `module_id` over the FULL chain
///    (including the trailing self); if `count > max_module_repeat` →
///    `CallFrequencyExceeded`.
///
/// Because the chain includes the trailing self, this is equivalent to the
/// spec pseudocode form (which excludes self and uses `>=`): a module
/// appearing exactly `max_module_repeat` times is allowed; one more throws.
///
/// [`DEFAULT_MAX_CALL_DEPTH`] (32) and [`DEFAULT_MAX_MODULE_REPEAT`] (3) are
/// the documented defaults; Rust has no default arguments, so pass them
/// explicitly or use [`guard_call_chain_for_context`], which applies both.
///
/// ```
/// use apcore::utils::{guard_call_chain, DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT};
///
/// let chain = vec!["a".to_string(), "b".to_string(), "a".to_string()];
/// let err = guard_call_chain("a", &chain, DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT)
///     .expect_err("a -> b -> a is a cycle");
/// assert_eq!(err.code, apcore::errors::ErrorCode::CircularCall);
/// ```
///
/// # Errors
///
/// - [`ErrorCode::GeneralInvalidInput`] when either limit is below 1.
/// - [`ErrorCode::CallDepthExceeded`], [`ErrorCode::CircularCall`] or
///   [`ErrorCode::CallFrequencyExceeded`] per the checks above.
pub fn guard_call_chain(
    module_id: &str,
    call_chain: &[String],
    max_call_depth: usize,
    max_module_repeat: usize,
) -> Result<(), ModuleError> {
    // 0. Floor validation — reject non-positive limits defensively, matching
    // apcore-python `call_chain.py` and apcore-typescript `call-chain.ts`
    // (both raise on max_call_depth < 1 / max_module_repeat < 1).
    // D-84: the typed error is what a cross-language caller can catch, and the
    // only form carrying a code from the registry. It goes through
    // `ModuleError::invalid_input` so it carries `ai_guidance` like the three
    // guard errors below — `ai_guidance` is `skip_serializing_if`, so leaving
    // it unset drops the field from the wire envelope entirely.
    if max_call_depth < 1 {
        return Err(ModuleError::invalid_input(format!(
            "max_call_depth must be >= 1, got {max_call_depth}"
        ))
        .with_ai_guidance(
            "The call-depth limit is not a positive integer. Set `executor.max_call_depth` \
             to at least 1 (the documented default is 32) and retry.",
        ));
    }
    if max_module_repeat < 1 {
        return Err(ModuleError::invalid_input(format!(
            "max_module_repeat must be >= 1, got {max_module_repeat}"
        ))
        .with_ai_guidance(
            "The module-repeat limit is not a positive integer. Set \
             `executor.max_module_repeat` to at least 1 (the documented default is 3) and retry.",
        ));
    }

    // 1. Depth check — chain length must not exceed max_call_depth.
    if call_chain.len() > max_call_depth {
        let depth = call_chain.len();
        // Structured details mirror apcore-python CallDepthExceededError
        // (errors.py:624): {depth, max_depth, call_chain} — sync finding A-D-17.
        let mut details = std::collections::HashMap::new();
        details.insert("depth".to_string(), serde_json::json!(depth));
        details.insert("max_depth".to_string(), serde_json::json!(max_call_depth));
        details.insert("call_chain".to_string(), serde_json::json!(call_chain));
        return Err(ModuleError::new(
            ErrorCode::CallDepthExceeded,
            format!("Call depth exceeded: chain length {depth} > max_depth {max_call_depth}"),
        )
        .with_details(details)
        // `ai_guidance` is `skip_serializing_if = "Option::is_none"`, so
        // omitting it drops the field from the wire envelope entirely — while
        // apcore-python `CallDepthExceededError` and apcore-typescript
        // `CallDepthExceededError` always carry it. Same wording as both.
        .with_ai_guidance(format!(
            "Call depth {depth} exceeds maximum {max_call_depth}. Simplify the module call chain \
             or restructure to reduce nesting depth."
        )));
    }

    // 2. Circular detection: strict cycles of length >= 2.
    // call_chain already includes module_id at the end (from child()),
    // so always strip the last entry and inspect the prior chain for a
    // previous occurrence forming A->...->A.
    let prior = if call_chain.is_empty() {
        call_chain
    } else {
        &call_chain[..call_chain.len() - 1]
    };
    if let Some(last_idx) = prior.iter().rposition(|n| n.as_str() == module_id) {
        let subsequence = &prior[last_idx + 1..];
        if !subsequence.is_empty() {
            // Structured details mirror apcore-python CircularCallError
            // (errors.py): {module_id, call_chain} — sync finding A-D-17.
            let mut details = std::collections::HashMap::new();
            details.insert("module_id".to_string(), serde_json::json!(module_id));
            details.insert("call_chain".to_string(), serde_json::json!(call_chain));
            return Err(ModuleError::new(
                ErrorCode::CircularCall,
                format!(
                    "Circular call detected: '{module_id}' already in call chain {call_chain:?}"
                ),
            )
            .with_details(details)
            // Same wording as apcore-python / apcore-typescript
            // `CircularCallError`; see the depth arm above for why the field
            // must not be left unset.
            .with_ai_guidance(
                "A circular call was detected in the module call chain. Review the call_chain \
                 in error details and restructure to eliminate the cycle.",
            ));
        }
    }

    // 3. Frequency throttle: count over the FULL chain (including the trailing
    // self); the module must not appear MORE than max_module_repeat times.
    let count = call_chain
        .iter()
        .filter(|name| name.as_str() == module_id)
        .count();

    if count > max_module_repeat {
        // Structured details mirror apcore-python CallFrequencyExceededError
        // (errors.py:683): {module_id, count, max_repeat, call_chain} — sync
        // finding A-D-17.
        let mut details = std::collections::HashMap::new();
        details.insert("module_id".to_string(), serde_json::json!(module_id));
        details.insert("count".to_string(), serde_json::json!(count));
        details.insert(
            "max_repeat".to_string(),
            serde_json::json!(max_module_repeat),
        );
        details.insert("call_chain".to_string(), serde_json::json!(call_chain));
        return Err(ModuleError::new(
            ErrorCode::CallFrequencyExceeded,
            format!(
                "Module '{module_id}' called {count} times, exceeds max repeat limit of {max_module_repeat}"
            ),
        )
        .with_details(details)
        .with_ai_guidance(format!(
            "Module '{module_id}' was called {count} times in this chain (limit \
             {max_module_repeat}), tripping the frequency guard. Reduce repeated calls or \
             batch the work before retrying."
        )));
    }

    Ok(())
}

/// Guard a [`Context`]'s call chain using the documented defaults
/// [`DEFAULT_MAX_CALL_DEPTH`] (32) and [`DEFAULT_MAX_MODULE_REPEAT`] (3).
///
/// A thin wrapper over [`guard_call_chain`], generic over the context's
/// services type so it is reachable from any host (D-83). Use
/// [`guard_call_chain_with_repeat`] when the limits come from configuration,
/// as the executor's `call_chain_guard` pipeline step does.
///
/// # Errors
///
/// As [`guard_call_chain`].
pub fn guard_call_chain_for_context<T>(
    ctx: &Context<T>,
    module_id: &str,
) -> Result<(), ModuleError> {
    guard_call_chain(
        module_id,
        &ctx.call_chain,
        DEFAULT_MAX_CALL_DEPTH,
        DEFAULT_MAX_MODULE_REPEAT,
    )
}

/// Guard a [`Context`]'s call chain with explicit limits.
///
/// A thin wrapper over [`guard_call_chain`], generic over the context's
/// services type (D-83). This is the form the executor's `call_chain_guard`
/// pipeline step uses, with the limits read from
/// `executor.max_call_depth` / `executor.max_module_repeat`.
///
/// # Errors
///
/// As [`guard_call_chain`].
pub fn guard_call_chain_with_repeat<T>(
    ctx: &Context<T>,
    module_name: &str,
    max_depth: usize,
    max_module_repeat: usize,
) -> Result<(), ModuleError> {
    guard_call_chain(module_name, &ctx.call_chain, max_depth, max_module_repeat)
}

/// Convert a single segment to `snake_case` by detecting case boundaries.
///
/// Matches Algorithm A02 from the apcore protocol spec:
/// - Leaves segments with no ASCII uppercase byte-for-byte unchanged.
/// - Inserts `_` before an ASCII uppercase letter preceded by an ASCII lowercase/digit.
/// - Inserts `_` between consecutive uppercase letters when followed by a lowercase letter
///   (e.g., "`HTTPClient`" -> "`http_client`", "`HTMLParser`" -> "`html_parser`").
/// - Converts ASCII uppercase letters only and never repairs separators.
pub(super) fn to_snake_case(segment: &str) -> String {
    if !segment.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return segment.to_owned();
    }

    let chars: Vec<char> = segment.chars().collect();
    let mut result = String::with_capacity(segment.len() + 4);

    for (i, &ch) in chars.iter().enumerate() {
        if i > 0 {
            let prev = chars[i - 1];
            let boundary = if (prev.is_ascii_lowercase() || prev.is_ascii_digit())
                && ch.is_ascii_uppercase()
            {
                true
            } else {
                prev.is_ascii_uppercase()
                    && ch.is_ascii_uppercase()
                    && i + 1 < chars.len()
                    && chars[i + 1].is_ascii_lowercase()
            };
            if boundary {
                result.push('_');
            }
        }
        result.push(ch.to_ascii_lowercase());
    }

    result
}

/// Supported source languages for [`normalize_to_canonical_id`], with their
/// local-ID separators. Mirrors apcore-python `_SEPARATORS` (normalize.py:10).
const SUPPORTED_LANGUAGES: &[(&str, &str)] = &[
    ("python", "."),
    ("rust", "::"),
    ("go", "."),
    ("java", "."),
    ("typescript", "."),
];

/// Language-specific separator used to split local IDs into segments, or
/// `None` if the language is not supported.
fn separator_for_language(language: &str) -> Option<&'static str> {
    SUPPORTED_LANGUAGES
        .iter()
        .find(|(lang, _)| *lang == language)
        .map(|(_, sep)| *sep)
}

/// Normalize a local module identifier to its canonical dotted `snake_case` form.
///
/// Implements Algorithm A02 from the apcore protocol spec. Splits `local_id`
/// by the language-specific separator, converts each segment to `snake_case`,
/// and joins with `"."`.
///
/// # Errors
///
/// Returns `Err(ModuleError)` with `ErrorCode::GeneralInvalidInput` if:
/// - `local_id` is empty,
/// - `language` is not one of the supported languages (python, rust, go,
///   java, typescript), or
/// - the normalized result does not conform to the Canonical ID grammar
///   (`^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`).
///
/// Mirrors apcore-python `normalize_to_canonical_id` (normalize.py:75), which
/// raises on each of these conditions (sync finding A-D-21).
pub fn normalize_to_canonical_id(local_id: &str, language: &str) -> Result<String, ModuleError> {
    if local_id.is_empty() {
        return Err(ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            "local_id must be a non-empty string",
        ));
    }

    let separator = separator_for_language(language).ok_or_else(|| {
        let supported: Vec<&str> = SUPPORTED_LANGUAGES.iter().map(|(l, _)| *l).collect();
        ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            format!(
                "Unsupported language '{}'. Must be one of: {}",
                language,
                supported.join(", ")
            ),
        )
    })?;

    let canonical_id = local_id
        .split(separator)
        .map(to_snake_case)
        .collect::<Vec<_>>()
        .join(".");

    // Validate against the Canonical ID grammar (Algorithm A02).
    // SAFETY: the pattern is a compile-time constant known to be a valid regex.
    let re = regex::Regex::new(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")
        .expect("canonical ID regex is a valid compile-time pattern");
    if !re.is_match(&canonical_id) {
        return Err(ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            format!(
                "Normalized ID '{canonical_id}' (from '{local_id}', language='{language}') \
                 does not conform to Canonical ID grammar"
            ),
        ));
    }

    Ok(canonical_id)
}

/// Calculate the specificity of a pattern for ACL rule ordering.
///
/// Ported from apcore-python `utils/pattern.py::calculate_specificity`.
/// - Wildcard-only `"*"` returns 0.
/// - Each dot-separated segment scores: exact literal = 2, partial wildcard = 1, pure `"*"` = 0.
#[must_use]
pub fn calculate_specificity(pattern: &str) -> u32 {
    if pattern == "*" {
        return 0;
    }
    let mut score: u32 = 0;
    for segment in pattern.split('.') {
        if segment == "*" {
            // +0
        } else if segment.contains('*') {
            score += 1;
        } else {
            score += 2;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::errors::ErrorCode;

    #[test]
    fn test_match_pattern_wildcard_matches_everything() {
        assert!(match_pattern("*", "anything"));
        assert!(match_pattern("*", ""));
        assert!(match_pattern("*", "a.b.c"));
    }

    #[test]
    fn test_match_pattern_exact_match() {
        assert!(match_pattern("foo.bar", "foo.bar"));
        assert!(!match_pattern("foo.bar", "foo.baz"));
        assert!(!match_pattern("foo.bar", "foo.bar.baz"));
    }

    #[test]
    fn test_match_pattern_no_wildcards_no_match() {
        assert!(!match_pattern("abc", "def"));
    }

    #[test]
    fn test_match_pattern_prefix_wildcard() {
        assert!(match_pattern("foo.*", "foo.bar"));
        assert!(match_pattern("foo.*", "foo.anything"));
        assert!(!match_pattern("foo.*", "bar.baz"));
    }

    #[test]
    fn test_match_pattern_suffix_wildcard() {
        assert!(match_pattern("*.bar", "foo.bar"));
        assert!(match_pattern("*.bar", "x.y.bar"));
        assert!(!match_pattern("*.bar", "foo.baz"));
    }

    #[test]
    fn test_match_pattern_middle_wildcard() {
        assert!(match_pattern("a.*.c", "a.b.c"));
        assert!(match_pattern("a.*.c", "a.xyz.c"));
        assert!(!match_pattern("a.*.c", "a.b.d"));
    }

    #[test]
    fn test_match_pattern_multiple_wildcards() {
        assert!(match_pattern("a.*.*.d", "a.b.c.d"));
    }

    #[test]
    fn test_guard_call_chain_empty_chain_passes() {
        let ctx = Context::<serde_json::Value>::anonymous();
        assert!(guard_call_chain_with_repeat(&ctx, "mod.a", 10, DEFAULT_MAX_MODULE_REPEAT).is_ok());
    }

    #[test]
    fn test_guard_call_chain_depth_exceeded() {
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let result = guard_call_chain_with_repeat(&ctx, "e", 3, DEFAULT_MAX_MODULE_REPEAT);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CallDepthExceeded);
    }

    #[test]
    fn test_guard_call_chain_circular_detection() {
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["mod.a".into(), "mod.b".into(), "mod.a".into()];
        let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CircularCall);
    }

    #[test]
    fn test_guard_call_chain_depth_error_carries_ai_guidance() {
        // `ai_guidance` is `skip_serializing_if = "Option::is_none"`, so leaving
        // it unset removed the field from the wire envelope for the two most
        // common guard trips while Python and TypeScript always carried it.
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let err = guard_call_chain_with_repeat(&ctx, "e", 3, DEFAULT_MAX_MODULE_REPEAT)
            .expect_err("depth guard should trip");
        assert_eq!(
            err.ai_guidance.as_deref(),
            Some(
                "Call depth 4 exceeds maximum 3. Simplify the module call chain or restructure \
                 to reduce nesting depth."
            )
        );
    }

    #[test]
    fn test_guard_call_chain_circular_error_carries_ai_guidance() {
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["mod.a".into(), "mod.b".into(), "mod.a".into()];
        let err = guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT)
            .expect_err("circular guard should trip");
        assert_eq!(
            err.ai_guidance.as_deref(),
            Some(
                "A circular call was detected in the module call chain. Review the call_chain \
                 in error details and restructure to eliminate the cycle."
            )
        );
    }

    #[test]
    fn test_guard_call_chain_frequency_at_default_limit_passes() {
        // A-D-040: canonical frequency uses `count > max_module_repeat` over the
        // FULL chain (which includes the trailing self). A module appearing
        // exactly max_module_repeat (default 3) times must PASS, not throw.
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["mod.a".into(), "mod.a".into(), "mod.a".into()];
        let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT);
        assert!(
            result.is_ok(),
            "exactly max_module_repeat (3) occurrences must pass, got {result:?}"
        );
    }

    #[test]
    fn test_guard_call_chain_frequency_exceeded() {
        // Four occurrences with default max_module_repeat=3: count(4) > 3 → throw.
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec![
            "mod.a".into(),
            "mod.a".into(),
            "mod.a".into(),
            "mod.a".into(),
        ];
        let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CallFrequencyExceeded);
    }

    #[test]
    fn test_guard_call_chain_with_repeat_custom_limit() {
        // max_module_repeat=1: a chain ["mod.a", "mod.a"] has count=2 > 1 → throw.
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["mod.a".into(), "mod.a".into()];
        let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, 1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CallFrequencyExceeded);
    }

    #[test]
    fn test_guard_call_chain_with_repeat_single_self_within_limit() {
        // max_module_repeat=1, chain=["mod.a"] (count=1): 1 > 1 is false → ok.
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["mod.a".into()];
        let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, 1);
        assert!(
            result.is_ok(),
            "count==max_module_repeat must pass: {result:?}"
        );
    }

    #[test]
    fn test_guard_call_chain_ok_within_limits() {
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["mod.a".into(), "mod.b".into()];
        assert!(guard_call_chain_with_repeat(&ctx, "mod.c", 10, DEFAULT_MAX_MODULE_REPEAT).is_ok());
    }

    #[test]
    fn test_normalize_python_dotted() {
        assert_eq!(
            normalize_to_canonical_id("MyModule.SendEmail", "python").unwrap(),
            "my_module.send_email"
        );
    }

    #[test]
    fn test_normalize_rust_double_colon() {
        assert_eq!(
            normalize_to_canonical_id("MyModule::SendEmail", "rust").unwrap(),
            "my_module.send_email"
        );
    }

    #[test]
    fn test_normalize_already_snake_case() {
        assert_eq!(
            normalize_to_canonical_id("my_module.send_email", "python").unwrap(),
            "my_module.send_email"
        );
    }

    #[test]
    fn test_normalize_acronym_handling() {
        assert_eq!(
            normalize_to_canonical_id("HTTPClient", "python").unwrap(),
            "http_client"
        );
        assert_eq!(
            normalize_to_canonical_id("HTMLParser", "python").unwrap(),
            "html_parser"
        );
    }

    #[test]
    fn test_normalize_camel_case_boundary() {
        assert_eq!(
            normalize_to_canonical_id("getValue", "python").unwrap(),
            "get_value"
        );
    }

    #[test]
    fn test_normalize_digit_boundary() {
        assert_eq!(
            normalize_to_canonical_id("log2Base", "python").unwrap(),
            "log2_base"
        );
    }

    // A-D-21: validation in normalize_to_canonical_id.
    #[test]
    fn test_normalize_empty_local_id_is_error() {
        let err = normalize_to_canonical_id("", "python").expect_err("empty must error");
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    }

    #[test]
    fn test_normalize_unsupported_language_is_error() {
        let err =
            normalize_to_canonical_id("MyModule", "cobol").expect_err("unsupported must error");
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    }

    #[test]
    fn test_normalize_invalid_canonical_grammar_is_error() {
        // A leading digit yields a normalized id that violates the grammar
        // (`^[a-z]...`), so it must be rejected rather than silently returned.
        let err =
            normalize_to_canonical_id("123bad", "python").expect_err("invalid grammar must error");
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    }

    #[test]
    fn test_normalize_valid_returns_ok() {
        assert_eq!(
            normalize_to_canonical_id("MyModule.SendEmail", "python").unwrap(),
            "my_module.send_email"
        );
    }

    // A-D-17: call-chain guard errors carry structured details.
    #[test]
    fn test_depth_guard_error_carries_structured_details() {
        let mut ctx = Context::<serde_json::Value>::anonymous();
        ctx.call_chain = vec!["a".into(), "b".into(), "c".into()];
        let err =
            guard_call_chain_with_repeat(&ctx, "c", 2, 3).expect_err("depth guard should trip");
        assert_eq!(err.code, ErrorCode::CallDepthExceeded);
        assert!(
            err.details.contains_key("max_depth"),
            "details must contain max_depth: {:?}",
            err.details
        );
        assert!(
            err.details.contains_key("call_chain"),
            "details must contain call_chain: {:?}",
            err.details
        );
    }

    #[test]
    fn test_frequency_guard_error_carries_structured_details() {
        let mut ctx = Context::<serde_json::Value>::anonymous();
        // "x" appears 3 times; max_repeat 2 → frequency guard trips.
        ctx.call_chain = vec!["x".into(), "x".into(), "x".into()];
        let err = guard_call_chain_with_repeat(&ctx, "x", 10, 2)
            .expect_err("frequency guard should trip");
        assert_eq!(err.code, ErrorCode::CallFrequencyExceeded);
        assert_eq!(
            err.details.get("count").and_then(serde_json::Value::as_u64),
            Some(3)
        );
        assert_eq!(
            err.details
                .get("max_repeat")
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );
        assert!(err.details.contains_key("call_chain"));
    }

    #[test]
    fn test_specificity_wildcard_only() {
        assert_eq!(calculate_specificity("*"), 0);
    }

    #[test]
    fn test_specificity_exact_segments() {
        assert_eq!(calculate_specificity("foo.bar"), 4);
    }

    #[test]
    fn test_specificity_partial_wildcard() {
        assert_eq!(calculate_specificity("foo.*"), 2);
    }

    #[test]
    fn test_specificity_partial_wildcard_in_segment() {
        assert_eq!(calculate_specificity("foo.ba*"), 3);
    }

    #[test]
    fn test_specificity_single_exact() {
        assert_eq!(calculate_specificity("executor"), 2);
    }

    #[test]
    fn test_specificity_all_wildcards() {
        assert_eq!(calculate_specificity("*.*.*"), 0);
    }

    #[test]
    fn test_specificity_mixed() {
        assert_eq!(calculate_specificity("a.*.b.c*"), 5);
    }
}
