"""Regression tests for canonical 3-part error fingerprint (finding A-D-15).

The authoritative fingerprint (see apcore/docs/features/observability.md
§1.4 / §"Error fingerprinting") is:

    SHA-256(error_code + ":" + module_id + ":" + normalized_message)

as a 64-char lowercase hex string. It MUST NOT include a top-frame
component, and ``normalize_message`` MUST be exactly the five canonical
steps (no extra hex-run collapsing).
"""

from __future__ import annotations

import hashlib

from apcore.errors import ModuleError
from apcore.observability.error_history import (
    compute_error_fingerprint,
    normalize_message,
)


def _raise_with(message: str, code: str = "TEST_ERR") -> ModuleError:
    """Build a ModuleError carrying a real ``__traceback__``."""
    try:
        raise ModuleError(code=code, message=message)
    except ModuleError as exc:
        return exc


def test_fingerprint_is_canonical_three_part_digest() -> None:
    """Fingerprint equals SHA-256(code:module_id:normalized_message).

    Sample message mixes a UUID, an ISO-8601 timestamp and a 5-digit run,
    so it exercises the full normalization pipeline.
    """
    code = "DB_TIMEOUT"
    module_id = "executor.db.query"
    message = "request 550e8400-e29b-41d4-a716-446655440000 at 2026-01-01T10:00:00Z exceeded 30000 ms"
    err = _raise_with(message, code=code)

    normalized = normalize_message(message)
    expected = hashlib.sha256(f"{code}:{module_id}:{normalized}".encode("utf-8")).hexdigest()

    assert compute_error_fingerprint(err, module_id) == expected


def test_fingerprint_ignores_call_site() -> None:
    """Same (code, module_id, normalized message) from different call sites collapse.

    Previously the top-frame signature made these diverge; the canonical
    form drops the frame so they MUST be identical.
    """

    def _raise_at_other_site(message: str) -> ModuleError:
        try:
            raise ModuleError(code="TEST_ERR", message=message)
        except ModuleError as exc:
            return exc

    err_a = _raise_with("same message")
    err_b = _raise_at_other_site("same message")

    assert compute_error_fingerprint(err_a, "m") == compute_error_fingerprint(err_b, "m")


def test_normalize_does_not_collapse_hex_runs() -> None:
    """Only decimal runs >= 4 digits become <ID>; hex letters are preserved.

    A long hex run such as ``deadbeefcafe1234`` MUST NOT be replaced with
    ``<HEX>`` (that extra step is not part of the canonical algorithm).
    The trailing digits are part of the same word (no word boundary), so
    the ``\\b\\d{4,}\\b`` step leaves the run untouched.
    """
    normalized = normalize_message("digest deadbeefcafe1234 mismatch")

    assert "<hex>" not in normalized
    assert normalized == "digest deadbeefcafe1234 mismatch"
