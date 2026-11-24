"""Tests for version negotiation (Algorithm A14)."""

from __future__ import annotations

import logging

import pytest

from apcore.version import VersionIncompatibleError, negotiate_version


class TestNegotiateVersion:
    """Tests for negotiate_version()."""

    # --- Happy path: same major.minor, effective = max ---

    def test_same_version_returns_itself(self) -> None:
        assert negotiate_version("1.2.3", "1.2.3") == "1.2.3"

    def test_same_minor_declared_higher_patch(self) -> None:
        assert negotiate_version("1.2.5", "1.2.3") == "1.2.5"

    def test_same_minor_sdk_higher_patch(self) -> None:
        assert negotiate_version("1.2.3", "1.2.5") == "1.2.5"

    # --- Major mismatch → error ---

    def test_major_mismatch_raises(self) -> None:
        with pytest.raises(VersionIncompatibleError) as exc_info:
            negotiate_version("2.0.0", "1.5.0")
        assert "Major version mismatch" in str(exc_info.value)
        assert exc_info.value.details["declared_version"] == "2.0.0"
        assert exc_info.value.details["sdk_version"] == "1.5.0"

    def test_major_mismatch_zero_vs_one(self) -> None:
        with pytest.raises(VersionIncompatibleError):
            negotiate_version("0.9.0", "1.0.0")

    # --- Declared minor > SDK minor → SDK too old ---

    def test_declared_minor_exceeds_sdk(self) -> None:
        with pytest.raises(VersionIncompatibleError) as exc_info:
            negotiate_version("1.5.0", "1.3.0")
        assert "SDK version too low" in str(exc_info.value)

    # --- Declared minor < SDK minor → backward compat ---

    def test_backward_compat_returns_declared(self) -> None:
        result = negotiate_version("1.2.0", "1.4.0")
        assert result == "1.2.0"

    def test_backward_compat_gap_within_threshold(self) -> None:
        result = negotiate_version("1.3.0", "1.5.0")
        assert result == "1.3.0"

    # --- Deprecation warning when gap > 2 ---

    def test_deprecation_warning_emitted(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.version"):
            result = negotiate_version("1.1.0", "1.4.0")
        assert result == "1.1.0"
        assert "consider upgrading" in caplog.text.lower()

    def test_no_deprecation_warning_within_threshold(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.version"):
            negotiate_version("1.3.0", "1.5.0")
        assert "consider upgrading" not in caplog.text.lower()

    # --- Invalid semver → ValueError ---

    def test_invalid_declared_version(self) -> None:
        with pytest.raises(ValueError, match="Invalid semantic version"):
            negotiate_version("not.a.version", "1.0.0")

    def test_invalid_sdk_version(self) -> None:
        with pytest.raises(ValueError, match="Invalid semantic version"):
            negotiate_version("1.0.0", "abc")

    def test_empty_string(self) -> None:
        with pytest.raises(ValueError):
            negotiate_version("", "1.0.0")

    def test_partial_version(self) -> None:
        with pytest.raises(ValueError):
            negotiate_version("1.0", "1.0.0")

    # --- Pre-release versions ---

    def test_prerelease_lower_than_release(self) -> None:
        # Per semver: 1.2.3-alpha < 1.2.3 (release has higher precedence)
        result = negotiate_version("1.2.3-alpha", "1.2.3")
        assert result == "1.2.3"

    def test_prerelease_both(self) -> None:
        result = negotiate_version("1.2.3-alpha", "1.2.3-beta")
        assert result in ("1.2.3-alpha", "1.2.3-beta")

    # --- Edge cases ---

    def test_zero_major_version(self) -> None:
        assert negotiate_version("0.1.0", "0.1.2") == "0.1.2"

    def test_large_version_numbers(self) -> None:
        assert negotiate_version("99.100.200", "99.100.300") == "99.100.300"

    def test_whitespace_trimmed(self) -> None:
        assert negotiate_version(" 1.2.3 ", " 1.2.3 ") == "1.2.3"


class TestVersionIncompatibleError:
    """Tests for VersionIncompatibleError."""

    def test_is_not_retryable(self) -> None:
        err = VersionIncompatibleError(declared="1.0.0", sdk="2.0.0", reason="test")
        assert err.retryable is False

    def test_error_code(self) -> None:
        err = VersionIncompatibleError(declared="1.0.0", sdk="2.0.0", reason="test")
        assert err.code == "VERSION_INCOMPATIBLE"

    def test_details_populated(self) -> None:
        err = VersionIncompatibleError(declared="1.0.0", sdk="2.0.0", reason="Major mismatch")
        assert err.details["declared_version"] == "1.0.0"
        assert err.details["sdk_version"] == "2.0.0"
        assert err.details["reason"] == "Major mismatch"
