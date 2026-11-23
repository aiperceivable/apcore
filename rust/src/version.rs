// APCore Protocol — Version negotiation
// Spec reference: Protocol version compatibility checks

use crate::errors::{ErrorCode, ModuleError, VersionIncompatibleError};

/// A parsed semver version with optional prerelease suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    /// `None` means a release version; `Some(tag)` means a prerelease.
    prerelease: Option<String>,
}

impl SemVer {
    /// Format back to a version string.
    fn to_string_repr(&self) -> String {
        match &self.prerelease {
            Some(pre) => format!("{}.{}.{}-{}", self.major, self.minor, self.patch, pre),
            None => format!("{}.{}.{}", self.major, self.minor, self.patch),
        }
    }
}

/// Semver ordering: major > minor > patch, then prerelease < release.
/// Among prereleases with the same major.minor.patch, compare lexicographically.
impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then(match (&self.prerelease, &other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                // Release > prerelease
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Parse a semver string into a `SemVer` struct.
///
/// Accepts versions like `"1.2.3"` and `"1.2.3-alpha"`.
fn parse_semver(version: &str) -> Option<SemVer> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;

    // The patch segment may contain a prerelease suffix: "3-alpha"
    let (patch, prerelease) = if let Some(dash_pos) = parts[2].find('-') {
        let patch_str = &parts[2][..dash_pos];
        let pre_str = &parts[2][dash_pos + 1..];
        if pre_str.is_empty() {
            return None;
        }
        (patch_str.parse::<u64>().ok()?, Some(pre_str.to_string()))
    } else {
        (parts[2].parse::<u64>().ok()?, None)
    };

    Some(SemVer {
        major,
        minor,
        patch,
        prerelease,
    })
}

/// Negotiate a compatible protocol version between a declared version and
/// the SDK version (Algorithm A14).
///
/// Returns the effective version string, or an error if the versions are
/// incompatible (major mismatch or declared minor exceeds SDK minor).
pub fn negotiate_version(declared_version: &str, sdk_version: &str) -> Result<String, ModuleError> {
    let declared = parse_semver(declared_version).ok_or_else(|| {
        ModuleError::new(
            ErrorCode::VersionIncompatible,
            format!("Invalid declared version: {declared_version}"),
        )
    })?;

    let sdk = parse_semver(sdk_version).ok_or_else(|| {
        ModuleError::new(
            ErrorCode::VersionIncompatible,
            format!("Invalid SDK version: {sdk_version}"),
        )
    })?;

    // Major mismatch -> error
    if declared.major != sdk.major {
        let err = VersionIncompatibleError {
            message: format!(
                "Major version mismatch: declared {declared_version} vs SDK {sdk_version}"
            ),
        };
        return Err(err.to_module_error());
    }

    // Declared minor > SDK minor -> error (SDK too old)
    if declared.minor > sdk.minor {
        let err = VersionIncompatibleError {
            message: format!(
                "Declared minor version {} exceeds SDK minor version {} (SDK too old)",
                declared.minor, sdk.minor
            ),
        };
        return Err(err.to_module_error());
    }

    // Declared minor < SDK minor -> return declared (backward compat)
    if declared.minor < sdk.minor {
        return Ok(declared_version.to_string());
    }

    // Same minor -> return max(declared, sdk)
    let effective = std::cmp::max(&declared, &sdk);
    Ok(effective.to_string_repr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_semver_basic() {
        let v = parse_semver("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.prerelease, None);
    }

    #[test]
    fn test_parse_semver_with_prerelease() {
        let v = parse_semver("1.0.0-alpha").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 0);
        assert_eq!(v.prerelease, Some("alpha".to_string()));
    }

    #[test]
    fn test_parse_semver_invalid_too_few_parts() {
        assert!(parse_semver("1.2").is_none());
    }

    #[test]
    fn test_parse_semver_invalid_too_many_parts() {
        assert!(parse_semver("1.2.3.4").is_none());
    }

    #[test]
    fn test_parse_semver_invalid_non_numeric() {
        assert!(parse_semver("a.b.c").is_none());
    }

    #[test]
    fn test_parse_semver_empty_prerelease_rejected() {
        assert!(parse_semver("1.0.0-").is_none());
    }

    #[test]
    fn test_semver_ordering_major() {
        let v1 = parse_semver("1.0.0").unwrap();
        let v2 = parse_semver("2.0.0").unwrap();
        assert!(v1 < v2);
    }

    #[test]
    fn test_semver_ordering_minor() {
        let v1 = parse_semver("1.1.0").unwrap();
        let v2 = parse_semver("1.2.0").unwrap();
        assert!(v1 < v2);
    }

    #[test]
    fn test_semver_ordering_patch() {
        let v1 = parse_semver("1.0.1").unwrap();
        let v2 = parse_semver("1.0.2").unwrap();
        assert!(v1 < v2);
    }

    #[test]
    fn test_semver_prerelease_less_than_release() {
        let pre = parse_semver("1.0.0-alpha").unwrap();
        let rel = parse_semver("1.0.0").unwrap();
        assert!(pre < rel);
    }

    #[test]
    fn test_semver_prerelease_lexicographic() {
        let alpha = parse_semver("1.0.0-alpha").unwrap();
        let beta = parse_semver("1.0.0-beta").unwrap();
        assert!(alpha < beta);
    }

    #[test]
    fn test_semver_to_string_repr() {
        let v = parse_semver("1.2.3").unwrap();
        assert_eq!(v.to_string_repr(), "1.2.3");

        let v_pre = parse_semver("1.2.3-rc1").unwrap();
        assert_eq!(v_pre.to_string_repr(), "1.2.3-rc1");
    }

    #[test]
    fn test_negotiate_same_version() {
        let result = negotiate_version("1.2.3", "1.2.3").unwrap();
        assert_eq!(result, "1.2.3");
    }

    #[test]
    fn test_negotiate_declared_older_minor() {
        let result = negotiate_version("1.1.0", "1.3.0").unwrap();
        assert_eq!(result, "1.1.0");
    }

    #[test]
    fn test_negotiate_declared_newer_minor_fails() {
        let result = negotiate_version("1.5.0", "1.3.0");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::VersionIncompatible);
    }

    #[test]
    fn test_negotiate_major_mismatch_fails() {
        let result = negotiate_version("2.0.0", "1.0.0");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::VersionIncompatible);
    }

    #[test]
    fn test_negotiate_same_minor_higher_sdk_patch() {
        let result = negotiate_version("1.2.0", "1.2.5").unwrap();
        assert_eq!(result, "1.2.5");
    }

    #[test]
    fn test_negotiate_same_minor_higher_declared_patch() {
        let result = negotiate_version("1.2.5", "1.2.0").unwrap();
        assert_eq!(result, "1.2.5");
    }

    #[test]
    fn test_negotiate_prerelease_vs_release_same_version() {
        let result = negotiate_version("1.2.0-alpha", "1.2.0").unwrap();
        assert_eq!(result, "1.2.0");
    }

    #[test]
    fn test_negotiate_invalid_declared_version() {
        let result = negotiate_version("not.a.version", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_negotiate_invalid_sdk_version() {
        let result = negotiate_version("1.0.0", "bad");
        assert!(result.is_err());
    }
}
