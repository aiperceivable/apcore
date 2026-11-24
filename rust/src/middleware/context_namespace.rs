// APCore Protocol — Context-data namespace validation (Issue #42)
// Spec reference: middleware-system.md §Context Namespacing
//
// Two reserved prefixes partition `context.data`:
//
//   `_apcore.*` — owned by framework middleware. Examples:
//                 `_apcore.mw.logging.start_time`,
//                 `_apcore.mw.tracing.spans`,
//                 `_apcore.mw.circuit.state`.
//   `ext.*`     — owned by user-defined middleware. Examples:
//                 `ext.my_company.request_id`.
//
// Keys with neither prefix are tolerated for backward compatibility but should
// be migrated. A `User` writer attempting to write a `_apcore.*` key, or a
// `Framework` writer attempting to write a `ext.*` key, is reported as a
// namespace violation.

use serde::{Deserialize, Serialize};

/// Identifier for the party performing a `context.data` write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextWriter {
    /// Built-in or framework-shipped middleware.
    Framework,
    /// User-supplied middleware or extension.
    User,
}

/// Reserved prefix for framework-owned `context.data` keys.
pub const APCORE_KEY_PREFIX: &str = "_apcore.";
/// Reserved prefix for user-extension `context.data` keys.
pub const EXT_KEY_PREFIX: &str = "ext.";

/// Canonical framework-owned context-data keys (informational).
pub mod namespace_keys {
    /// `LoggingMiddleware.before()` writes the call start time (epoch seconds).
    pub const LOGGING_START_TIME: &str = "_apcore.mw.logging.start_time";
    /// `TracingMiddleware` owns the span STACK for the call — see the typed
    /// [`crate::context_keys::TRACING_SPANS`] key. (The single-slot
    /// `_apcore.mw.tracing.span_id` that used to be listed here belonged to the
    /// withdrawn `middleware-system.md` §1.3 formulation; a single slot is
    /// overwritten by the first nested module-to-module call.)
    pub const TRACING_SPANS: &str = "_apcore.mw.tracing.spans";
    /// `CircuitBreakerMiddleware` writes the current circuit state for the call.
    pub const CIRCUIT_STATE: &str = "_apcore.mw.circuit.state";
}

/// Outcome of a namespace validation check on a `context.data` write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamespaceCheck {
    /// `true` if the write conforms to the namespace rules.
    pub valid: bool,
    /// `true` if the implementation should emit a `tracing::warn!` for this
    /// write. Currently set whenever `valid == false`.
    pub warning: bool,
}

/// Validate a `context.data` write against the spec namespace rules.
///
/// Pure: never logs or panics. Callers (e.g. `LoggingMiddleware`,
/// `CircuitBreakerMiddleware`, `TracingMiddleware`, or user-supplied
/// middleware) should call this before writing and emit a
/// `tracing::warn!` when `warning == true`. The convenience
/// [`enforce_context_key`] wraps that pattern.
#[must_use]
pub fn validate_context_key(writer: ContextWriter, key: &str) -> NamespaceCheck {
    let in_apcore = key.starts_with(APCORE_KEY_PREFIX);
    let in_ext = key.starts_with(EXT_KEY_PREFIX);
    let valid = match writer {
        ContextWriter::Framework => !in_ext,
        ContextWriter::User => !in_apcore,
    };
    NamespaceCheck {
        valid,
        warning: !valid,
    }
}

/// Validate and, if invalid, emit a `tracing::warn!` describing the violation.
///
/// Returns the same [`NamespaceCheck`] as [`validate_context_key`] so callers
/// may decide whether to skip the write.
pub fn enforce_context_key(writer: ContextWriter, key: &str) -> NamespaceCheck {
    let check = validate_context_key(writer, key);
    if check.warning {
        match writer {
            ContextWriter::User => tracing::warn!(
                key = key,
                "User middleware wrote to reserved '_apcore.*' namespace; \
                 framework-owned keys must not be set by user code"
            ),
            ContextWriter::Framework => tracing::warn!(
                key = key,
                "Framework middleware wrote to user 'ext.*' namespace; \
                 user-extension keys must not be set by framework code"
            ),
        }
    }
    check
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framework_apcore_prefix_is_valid() {
        let check = validate_context_key(ContextWriter::Framework, "_apcore.mw.logging.start_time");
        assert!(check.valid);
        assert!(!check.warning);
    }

    #[test]
    fn user_ext_prefix_is_valid() {
        let check = validate_context_key(ContextWriter::User, "ext.my_company.request_id");
        assert!(check.valid);
        assert!(!check.warning);
    }

    #[test]
    fn user_writing_apcore_prefix_is_violation() {
        let check = validate_context_key(ContextWriter::User, "_apcore.mw.tracing.spans");
        assert!(!check.valid);
        assert!(check.warning);
    }

    #[test]
    fn framework_writing_ext_prefix_is_violation() {
        let check = validate_context_key(ContextWriter::Framework, "ext.user_payload");
        assert!(!check.valid);
        assert!(check.warning);
    }

    #[test]
    fn unprefixed_keys_are_tolerated_for_backward_compat() {
        // Neither prefix — both writers allowed (with no warning).
        let f = validate_context_key(ContextWriter::Framework, "legacy_key");
        let u = validate_context_key(ContextWriter::User, "legacy_key");
        assert!(f.valid && !f.warning);
        assert!(u.valid && !u.warning);
    }
}
