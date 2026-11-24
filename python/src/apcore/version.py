"""Version negotiation (Algorithm A14).

Compares caller-requested semver ranges against a module's declared version
and selects the best compatible match, enabling safe multi-version coexistence.
"""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass
from typing import Any

from apcore.errors import ModuleError

__all__ = ["negotiate_version", "VersionIncompatibleError"]

_logger = logging.getLogger(__name__)

#: Regex for parsing semver strings (major.minor.patch with optional pre-release).
_SEMVER_RE = re.compile(
    r"^(?P<major>0|[1-9]\d*)"
    r"\.(?P<minor>0|[1-9]\d*)"
    r"\.(?P<patch>0|[1-9]\d*)"
    r"(?:-(?P<pre>[0-9A-Za-z\-]+(?:\.[0-9A-Za-z\-]+)*))?$"
)

#: Deprecation warning threshold: if SDK minor exceeds declared minor by more
#: than this, emit a warning.
_DEPRECATION_THRESHOLD = 2


@dataclass(frozen=True)
class _SemVer:
    """Parsed semantic version for comparison.

    Ordering follows semver: pre-release versions have lower precedence
    than the same version without pre-release (1.2.3-alpha < 1.2.3).
    """

    major: int
    minor: int
    patch: int
    pre: str | None = None

    def _sort_key(self) -> tuple[int, int, int, int, tuple[tuple[int, int, str], ...]]:
        # No pre-release (None) → higher precedence than any pre-release string.
        # Per semver spec, numeric identifiers sort numerically, alphanumeric lexically,
        # and numeric < alphanumeric.
        if self.pre is None:
            return (self.major, self.minor, self.patch, 1, ())
        parts: list[tuple[int, int, str]] = []
        for ident in self.pre.split("."):
            if ident.isdigit():
                parts.append((0, int(ident), ""))  # numeric sorts first
            else:
                parts.append((1, 0, ident))  # alphanumeric sorts after numeric
        return (self.major, self.minor, self.patch, 0, tuple(parts))

    def __lt__(self, other: object) -> bool:
        if not isinstance(other, _SemVer):
            return NotImplemented
        return self._sort_key() < other._sort_key()

    def __le__(self, other: object) -> bool:
        if not isinstance(other, _SemVer):
            return NotImplemented
        return self._sort_key() <= other._sort_key()

    def __gt__(self, other: object) -> bool:
        if not isinstance(other, _SemVer):
            return NotImplemented
        return self._sort_key() > other._sort_key()

    def __ge__(self, other: object) -> bool:
        if not isinstance(other, _SemVer):
            return NotImplemented
        return self._sort_key() >= other._sort_key()

    def __str__(self) -> str:
        base = f"{self.major}.{self.minor}.{self.patch}"
        return f"{base}-{self.pre}" if self.pre else base


def _parse_semver(version: str) -> _SemVer:
    """Parse a semver string into components."""
    m = _SEMVER_RE.match(version.strip())
    if not m:
        raise ValueError(f"Invalid semantic version: '{version}'")
    return _SemVer(
        major=int(m.group("major")),
        minor=int(m.group("minor")),
        patch=int(m.group("patch")),
        pre=m.group("pre"),
    )


class VersionIncompatibleError(ModuleError):
    """Raised when version negotiation fails due to incompatibility."""

    _default_retryable: bool | None = False

    def __init__(self, declared: str, sdk: str, reason: str, **kwargs: Any) -> None:
        super().__init__(
            code="VERSION_INCOMPATIBLE",
            message=f"Version incompatible: declared={declared}, sdk={sdk} — {reason}",
            details={
                "declared_version": declared,
                "sdk_version": sdk,
                "reason": reason,
            },
            **kwargs,
        )


def negotiate_version(declared_version: str, sdk_version: str) -> str:
    """Negotiate the effective version between declared and SDK versions (Algorithm A14).

    Steps:
        1. Parse both versions as semver.
        2. Major mismatch → error.
        3. Declared minor > SDK minor → error (SDK too old).
        4. Declared minor < SDK minor by >2 → deprecation warning.
        5. Same minor → effective = max(declared, sdk).

    Args:
        declared_version: Version declared in configuration or schema.
        sdk_version: Maximum version supported by the current SDK.

    Returns:
        The effective version string.

    Raises:
        VersionIncompatibleError: When versions are incompatible.
        ValueError: When a version string is not valid semver.
    """
    declared = _parse_semver(declared_version)
    sdk = _parse_semver(sdk_version)

    # Step 3: Major version mismatch
    if declared.major != sdk.major:
        raise VersionIncompatibleError(
            declared=declared_version,
            sdk=sdk_version,
            reason="Major version mismatch",
        )

    # Step 4: Declared minor > SDK minor → SDK too old
    if declared.minor > sdk.minor:
        raise VersionIncompatibleError(
            declared=declared_version,
            sdk=sdk_version,
            reason="SDK version too low, please upgrade",
        )

    # Step 5: Declared minor < SDK minor
    if declared.minor < sdk.minor:
        gap = sdk.minor - declared.minor
        if gap > _DEPRECATION_THRESHOLD:
            _logger.warning(
                "Declared version %s is %d minor versions behind SDK %s — consider upgrading your configuration",
                declared_version,
                gap,
                sdk_version,
            )
        return declared_version  # Backward compatibility mode

    # Step 6: Same minor → effective = max(declared, sdk)
    effective = max(declared, sdk)
    return str(effective)
