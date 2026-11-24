"""Issue #43 §5 — configurable redaction via Config.

Verifies that ``obs.redaction.regex_patterns`` and
``obs.redaction.sensitive_keys`` Config keys drive the behaviour of
:class:`RedactionConfig`, :class:`ContextLogger`, and
:func:`apcore.utils.redaction.redact_sensitive`, replacing the legacy
hard-coded ``_secret_`` prefix logic.
"""

from __future__ import annotations

import io
import json

import pytest

from apcore.config import Config
from apcore.observability.context_logger import (
    ContextLogger,
    RedactionConfig,
    _key_matches_sensitive,
    _value_matches_regex,
)
from apcore.utils.redaction import redact_sensitive


class TestRedactionConfigFromConfig:
    """``RedactionConfig.from_config`` reads the obs.redaction.* keys."""

    def test_reads_sensitive_keys_from_config(self) -> None:
        cfg = Config.from_defaults()
        cfg.set("obs.redaction.sensitive_keys", ["foo", "bar"])
        rc = RedactionConfig.from_config(cfg)
        assert rc.sensitive_keys == ["foo", "bar"]

    def test_reads_regex_patterns_from_config(self) -> None:
        cfg = Config.from_defaults()
        cfg.set("obs.redaction.regex_patterns", [r"^Bearer\s+\S+$", r"^sk-[A-Za-z0-9]+$"])
        rc = RedactionConfig.from_config(cfg)
        assert rc.regex_patterns == [r"^Bearer\s+\S+$", r"^sk-[A-Za-z0-9]+$"]

    def test_reads_replacement_from_config(self) -> None:
        cfg = Config.from_defaults()
        cfg.set("obs.redaction.replacement", "<<HIDDEN>>")
        rc = RedactionConfig.from_config(cfg)
        assert rc.replacement == "<<HIDDEN>>"

    def test_default_sensitive_keys_cover_common_terms(self) -> None:
        """The default list MUST cover password / token / api_key / authorization."""
        rc = RedactionConfig.default()
        for term in ("password", "token", "api_key", "authorization", "secret"):
            assert term in rc.sensitive_keys, f"default sensitive_keys missing {term!r}"

    def test_default_includes_legacy_secret_prefix(self) -> None:
        """Backward-compat: the legacy ``_secret_*`` glob remains in the defaults."""
        rc = RedactionConfig.default()
        assert "_secret_*" in rc.sensitive_keys


class TestKeyMatching:
    """``_key_matches_sensitive`` — case-insensitive substring + glob."""

    def test_case_insensitive_substring_match(self) -> None:
        assert _key_matches_sensitive("X-API-Key", ["api_key"])
        assert _key_matches_sensitive("Password", ["password"])
        assert _key_matches_sensitive("AUTHORIZATION", ["authorization"])

    def test_glob_pattern_matches_legacy_prefix(self) -> None:
        assert _key_matches_sensitive("_secret_token", ["_secret_*"])
        assert _key_matches_sensitive("_SECRET_API_KEY", ["_secret_*"])

    def test_non_matching_keys_pass_through(self) -> None:
        assert not _key_matches_sensitive("username", ["password", "token"])
        assert not _key_matches_sensitive("email", ["password"])


class TestValueRegexMatching:
    """``_value_matches_regex`` — case-insensitive value match."""

    def test_regex_matches_bearer_token(self) -> None:
        assert _value_matches_regex("Bearer abc.def.ghi", [r"^Bearer\s+.+$"])
        # case-insensitive: lowercase "bearer" still matches.
        assert _value_matches_regex("bearer abc.def.ghi", [r"^Bearer\s+.+$"])

    def test_regex_does_not_match_non_string_unless_stringified(self) -> None:
        assert not _value_matches_regex(42, [r"^Bearer"])
        assert not _value_matches_regex(None, [r"^Bearer"])

    def test_invalid_regex_does_not_crash(self) -> None:
        # Operator typo should not blow up the redactor.
        assert not _value_matches_regex("anything", ["[invalid"])


class TestContextLoggerWithConfig:
    """End-to-end: ContextLogger emits redacted entries based on RedactionConfig."""

    def _last_entry(self, buf: io.StringIO) -> dict:
        return json.loads(buf.getvalue().splitlines()[-1])

    def test_default_logger_redacts_common_keys(self) -> None:
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf)
        logger.info("msg", extra={"password": "hunter2", "name": "alice"})
        entry = self._last_entry(buf)
        assert entry["extra"]["password"] == "***REDACTED***"
        assert entry["extra"]["name"] == "alice"

    def test_default_logger_redacts_token_substring(self) -> None:
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf)
        logger.info("msg", extra={"X-API-Key": "sk-abc"})
        entry = self._last_entry(buf)
        # "api_key" substring matches "X-API-Key" case-insensitively.
        assert entry["extra"]["X-API-Key"] == "***REDACTED***"

    def test_explicit_config_overrides_defaults(self) -> None:
        buf = io.StringIO()
        # Empty sensitive_keys disables key-name matching entirely.
        rc = RedactionConfig(sensitive_keys=[])
        logger = ContextLogger(name="t", output=buf, redaction_config=rc)
        logger.info("msg", extra={"password": "hunter2"})
        entry = self._last_entry(buf)
        assert entry["extra"]["password"] == "hunter2"

    def test_regex_value_pattern_redacts_string(self) -> None:
        buf = io.StringIO()
        rc = RedactionConfig(
            sensitive_keys=[],
            regex_patterns=[r"^Bearer\s+.+$"],
        )
        logger = ContextLogger(name="t", output=buf, redaction_config=rc)
        logger.info("msg", extra={"header": "Bearer xyz.abc"})
        entry = self._last_entry(buf)
        assert entry["extra"]["header"] == "***REDACTED***"

    def test_legacy_secret_prefix_still_works(self) -> None:
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf)
        logger.info("msg", extra={"_secret_token": "abc"})
        entry = self._last_entry(buf)
        assert entry["extra"]["_secret_token"] == "***REDACTED***"


class TestRedactSensitiveUtility:
    """``apcore.utils.redaction.redact_sensitive`` — Config-driven defaults."""

    def test_default_sensitive_keys_redact_password(self) -> None:
        result = redact_sensitive({"password": "hunter2", "name": "alice"}, {})
        assert result["password"] == "***REDACTED***"
        assert result["name"] == "alice"

    def test_default_sensitive_keys_redact_authorization(self) -> None:
        result = redact_sensitive({"Authorization": "Bearer xyz"}, {})
        assert result["Authorization"] == "***REDACTED***"

    def test_explicit_empty_sensitive_keys_disables_key_matching(self) -> None:
        result = redact_sensitive({"password": "hunter2"}, {}, sensitive_keys=[])
        assert result["password"] == "hunter2"

    def test_regex_pattern_redacts_value(self) -> None:
        result = redact_sensitive(
            {"header": "Bearer abc.def"},
            {},
            sensitive_keys=[],
            regex_patterns=[r"^Bearer\s+.+$"],
        )
        assert result["header"] == "***REDACTED***"

    def test_case_insensitive_key_match(self) -> None:
        result = redact_sensitive(
            {"X-API-KEY": "sk-abc"},
            {},
            sensitive_keys=["api_key"],
        )
        assert result["X-API-KEY"] == "***REDACTED***"

    def test_replacement_token_override(self) -> None:
        result = redact_sensitive(
            {"password": "hunter2"},
            {},
            sensitive_keys=["password"],
            replacement="<<HIDDEN>>",
        )
        assert result["password"] == "<<HIDDEN>>"

    def test_legacy_secret_prefix_default(self) -> None:
        result = redact_sensitive({"_secret_x": "v"}, {})
        assert result["_secret_x"] == "***REDACTED***"


class TestConfigDefaults:
    """The ``obs`` namespace ships with sane defaults via ``RedactionConfig.from_config``.

    ``Config.from_defaults()`` runs in legacy (flat) mode and does not
    materialise namespace defaults into the data dict — that is the job of
    namespace-mode loading.  Verify the user-facing helper
    :meth:`RedactionConfig.from_config` falls back to the same default list
    and that the namespace registration is correctly wired.
    """

    def test_from_config_returns_defaults_when_keys_missing(self) -> None:
        cfg = Config.from_defaults()
        rc = RedactionConfig.from_config(cfg)
        assert "password" in rc.sensitive_keys
        assert "token" in rc.sensitive_keys
        assert "_secret_*" in rc.sensitive_keys
        assert rc.regex_patterns == []
        assert rc.replacement == "***REDACTED***"

    def test_obs_namespace_registered(self) -> None:
        from apcore.config import _GLOBAL_NS_REGISTRY

        assert "obs" in _GLOBAL_NS_REGISTRY
        reg = _GLOBAL_NS_REGISTRY["obs"]
        assert reg.defaults is not None
        assert "redaction" in reg.defaults
        defaults = reg.defaults["redaction"]
        assert "password" in defaults["sensitive_keys"]
        assert "_secret_*" in defaults["sensitive_keys"]
        assert defaults["regex_patterns"] == []
        assert defaults["replacement"] == "***REDACTED***"


if __name__ == "__main__":
    pytest.main([__file__, "-q"])


class TestLegacyValuePatternsStringOnly:
    """#123: §10.6.1 requirement 2 — the legacy ``value_patterns`` rule never stringifies a value."""

    def test_non_string_values_are_not_tested(self) -> None:
        from apcore.observability.context_logger import _apply_redaction_config

        config = RedactionConfig(value_patterns=[r"\d+", r"True", r"\{"])
        data = {"amount": 42, "flag": True, "nested": {"a": 1}, "items": [1, 2]}
        assert _apply_redaction_config(data, config) == data

    def test_string_values_still_match(self) -> None:
        from apcore.observability.context_logger import _apply_redaction_config

        config = RedactionConfig(value_patterns=[r"\d+"], replacement="X")
        assert _apply_redaction_config({"amount": "42"}, config) == {"amount": "X"}
