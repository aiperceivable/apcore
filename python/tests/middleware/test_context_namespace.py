"""Unit tests for `apcore.middleware.context_namespace` (Issue #42).

The three fixture-pinned cases live in
`tests/conformance/test_middleware_hardening.py`; these cover the behaviours the
canonical fixture does NOT pin — the legacy (unprefixed) key allowance and the
`enforce_context_key` logging wrapper — so they cannot silently regress.

Cross-language parity target: apcore-rust
`src/middleware/context_namespace.rs::tests` and apcore-typescript
`src/middleware/context-namespace.ts`.
"""

from __future__ import annotations

import logging

import pytest

from apcore.middleware import (
    APCORE_KEY_PREFIX,
    EXT_KEY_PREFIX,
    NamespaceCheck,
    enforce_context_key,
    validate_context_key,
)


class TestValidateContextKey:
    @pytest.mark.parametrize(
        ("writer", "key", "valid"),
        [
            ("framework", "_apcore.mw.logging.start_time", True),
            ("framework", "_apcore.mw.tracing.spans", True),
            ("user", "ext.my_company.request_id", True),
            ("user", "_apcore.mw.tracing.spans", False),
            ("framework", "ext.user_payload", False),
        ],
    )
    def test_namespace_ownership(self, writer: str, key: str, valid: bool) -> None:
        check = validate_context_key(writer, key)  # type: ignore[arg-type]
        assert check.valid is valid
        assert check.warning is (not valid), "warning is set exactly when the write is invalid"

    @pytest.mark.parametrize("writer", ["framework", "user"])
    def test_unprefixed_keys_are_tolerated_for_both_writers(self, writer: str) -> None:
        """middleware-system.md §1.1: keys with neither prefix are allowed for
        backward compatibility (SHOULD be migrated, MUST NOT be rejected)."""
        check = validate_context_key(writer, "legacy_key")  # type: ignore[arg-type]
        assert check == NamespaceCheck(valid=True, warning=False)

    def test_is_pure_and_never_raises(self) -> None:
        assert validate_context_key("user", "") == NamespaceCheck(valid=True, warning=False)
        # A key equal to the prefix without a trailing segment still matches.
        assert validate_context_key("user", APCORE_KEY_PREFIX).valid is False
        assert validate_context_key("framework", EXT_KEY_PREFIX).valid is False

    # `test_canonical_key_table_matches_the_spec` was deleted, not adjusted: it
    # asserted each `namespace_keys` constant against a literal copy of its own
    # value, for a container this package no longer defines. `namespace_keys`
    # was a hand-maintained mirror of `apcore.context_keys` with no readers, and
    # it had already drifted — it declared a tracing span-id key that nothing
    # writes, while the canonical registry declares the
    # `_apcore.mw.tracing.spans` stack that `observability/tracing.py` actually
    # maintains. The surviving key names are pinned in
    # `tests/test_context_keys.py`, against the one registry that is left.


class TestEnforceContextKey:
    def test_violation_logs_a_warning_and_returns_the_check(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.middleware.context_namespace"):
            check = enforce_context_key("user", "_apcore.mw.tracing.spans")
        assert check == NamespaceCheck(valid=False, warning=True)
        assert any("_apcore." in rec.getMessage() for rec in caplog.records)

    def test_framework_writing_ext_logs_the_other_direction(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.middleware.context_namespace"):
            check = enforce_context_key("framework", "ext.user_payload")
        assert check.valid is False
        assert any("ext." in rec.getMessage() for rec in caplog.records)

    def test_valid_write_is_silent(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.middleware.context_namespace"):
            check = enforce_context_key("framework", "_apcore.mw.circuit.state")
        assert check == NamespaceCheck(valid=True, warning=False)
        assert caplog.records == []
