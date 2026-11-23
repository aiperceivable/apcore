//! Pure canonicalization of one external ASCII name into a canonical segment.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::helpers::to_snake_case;

/// Diagnostic returned when a bare name cannot become a canonical segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Error)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalNameError {
    /// No characters remain after trimming ASCII edge separators.
    #[error("The name is empty after trimming ASCII separators")]
    EmptyName,
    /// The original input contains a non-ASCII character.
    #[error("The name contains non-ASCII characters")]
    NonAscii,
    /// The canonical candidate does not start with an ASCII lowercase letter.
    #[error("The canonical name must start with an ASCII lowercase letter")]
    InvalidStart,
    /// The canonical candidate exceeds 192 ASCII characters.
    #[error("The canonical name exceeds 192 characters")]
    NameTooLong,
}

/// A canonical segment or a diagnostic, retaining the exact original input.
///
/// Successful results contain `canonical_name` and no `error`; failed results
/// contain `error` and no `canonical_name`. Both optional fields serialize,
/// including JSON `null`, for the cross-language result contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalNameResult {
    pub original_name: String,
    pub canonical_name: Option<String>,
    pub error: Option<CanonicalNameError>,
}

/// Canonicalize one bare external name without throwing or changing module IDs.
///
/// Rejects non-ASCII input before trimming ASCII edge punctuation/whitespace.
/// Applies Algorithm A02 case boundaries, then replaces each maximal ASCII
/// separator run with one underscore, preserving existing underscore runs.
/// The candidate must start with `[a-z]` and contain at most 192 characters;
/// no prefix is invented and no candidate is truncated. Diagnostic precedence
/// is `non_ascii`, `empty_name`, `invalid_start`, then `name_too_long`.
///
/// This is a single segment, not a dotted or language-specific module ID.
/// `system` is valid here; namespace reservation and collision detection remain
/// registration/scanning responsibilities. The function is pure and does not
/// replace [`super::normalize_to_canonical_id`].
///
/// ```
/// use apcore::{canonicalize_name, CanonicalNameError};
/// assert_eq!(canonicalize_name("cat-file").canonical_name.as_deref(), Some("cat_file"));
/// assert_eq!(canonicalize_name("7z").error, Some(CanonicalNameError::InvalidStart));
/// ```
#[must_use]
pub fn canonicalize_name(name: &str) -> CanonicalNameResult {
    let mut result = CanonicalNameResult {
        original_name: name.to_owned(),
        canonical_name: None,
        error: None,
    };
    if !name.is_ascii() {
        result.error = Some(CanonicalNameError::NonAscii);
        return result;
    }
    let trimmed = name.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
    let candidate = replace_separator_runs(&to_snake_case(trimmed));
    let error = if candidate.is_empty() {
        Some(CanonicalNameError::EmptyName)
    } else if !candidate.as_bytes()[0].is_ascii_lowercase() {
        Some(CanonicalNameError::InvalidStart)
    } else if candidate.len() > 192 {
        Some(CanonicalNameError::NameTooLong)
    } else {
        None
    };
    if let Some(error) = error {
        result.error = Some(error);
    } else {
        result.canonical_name = Some(candidate);
    }
    result
}

fn replace_separator_runs(name: &str) -> String {
    let mut candidate = String::with_capacity(name.len());
    let mut in_separator = false;
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            candidate.push(char::from(byte));
            in_separator = false;
        } else if !in_separator {
            candidate.push('_');
            in_separator = true;
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_separator_runs_preserves_existing_underscores() {
        assert_eq!(replace_separator_runs("a__- \t_b"), "a____b");
        assert_eq!(replace_separator_runs("a/b\\c"), "a_b_c");
    }

    #[test]
    fn test_canonicalize_name_success_is_idempotent() {
        for input in ["cat-file", "HTTPClient", "a___b", "trail_", "system"] {
            let first = canonicalize_name(input);
            let name = first.canonical_name.unwrap();
            let second = canonicalize_name(&name);
            assert_eq!(second.canonical_name.as_deref(), Some(name.as_str()));
            assert_eq!(second.error, None);
        }
    }
}
