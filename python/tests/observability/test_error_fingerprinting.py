"""Error fingerprinting via canonical code:module_id:normalized_message digest."""

from __future__ import annotations


from apcore.errors import ModuleError
from apcore.observability.error_history import (
    ErrorHistory,
    compute_error_fingerprint,
    compute_fingerprint,
)


def _raise_with(message: str, code: str = "TEST_ERR") -> ModuleError:
    """Build a ModuleError that has a real ``__traceback__`` attached."""
    try:
        raise ModuleError(code=code, message=message)
    except ModuleError as exc:
        return exc


def test_fingerprint_collapses_uuid_in_message() -> None:
    """Two errors that differ only by an embedded UUID share a fingerprint."""
    err_a = _raise_with("Failed to load record 550e8400-e29b-41d4-a716-446655440000")
    err_b = _raise_with("Failed to load record 11111111-2222-3333-4444-555555555555")
    fp_a = compute_error_fingerprint(err_a, module_id="m")
    fp_b = compute_error_fingerprint(err_b, module_id="m")
    assert fp_a == fp_b


def test_fingerprint_collapses_long_digit_runs() -> None:
    """Digit runs of length >= 4 are normalized to ``<ID>``."""
    err_a = _raise_with("DB row 12345 missing")
    err_b = _raise_with("DB row 99887 missing")
    assert compute_error_fingerprint(err_a, "m") == compute_error_fingerprint(err_b, "m")


def test_fingerprint_distinguishes_different_codes() -> None:
    """Same message but different error code => different fingerprint."""
    err_a = _raise_with("boom", code="A_CODE")
    err_b = _raise_with("boom", code="B_CODE")
    assert compute_error_fingerprint(err_a, "m") != compute_error_fingerprint(err_b, "m")


def test_fingerprint_ignores_top_frame() -> None:
    """Same code+module+message from different call sites => SAME fingerprint.

    The canonical fingerprint is ``code:module_id:normalized_message`` only.
    The originating stack frame is not portable across languages, so it is
    excluded from the digest (observability spec §"Error fingerprinting").
    """

    def _raise_b(message: str) -> ModuleError:
        try:
            raise ModuleError(code="TEST_ERR", message=message)
        except ModuleError as exc:
            return exc

    err_a = _raise_with("same message")
    err_b = _raise_b("same message")
    # Different originating line numbers, but the frame is not in the digest.
    assert compute_error_fingerprint(err_a, "m") == compute_error_fingerprint(err_b, "m")


def test_error_history_dedupes_uuid_bearing_messages() -> None:
    """ErrorHistory should treat two UUID-only-different errors as one entry."""
    hist = ErrorHistory()
    err_a = _raise_with("trace 550e8400-e29b-41d4-a716-446655440000 failed")
    err_b = _raise_with("trace abcd1234-5678-90ab-cdef-1234567890ab failed")
    hist.record("module.x", err_a)
    hist.record("module.x", err_b)

    entries = hist.get("module.x")
    assert len(entries) == 1, f"Expected dedup, got {[e.message for e in entries]}"
    assert entries[0].count == 2


def test_legacy_compute_fingerprint_still_works() -> None:
    """The legacy 3-arg ``compute_fingerprint(code, module_id, message)`` is preserved."""
    fp = compute_fingerprint("CODE", "module", "hello")
    assert isinstance(fp, str) and len(fp) == 64
