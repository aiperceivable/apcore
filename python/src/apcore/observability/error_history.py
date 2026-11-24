"""Thread-safe error history with min-heap eviction, fingerprinting, and pluggable storage."""

from __future__ import annotations

import hashlib
import heapq
import os
import re
import threading
from typing import Any
import traceback
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone

from apcore.errors import ModuleError
from apcore.observability.storage import (
    STORAGE_NAMESPACE_ERROR_HISTORY,
    STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY,
    InMemoryStorageBackend,
    StorageBackend,
)
from apcore.observability.store import ObservabilityStore


def utc_millis_z() -> str:
    """Now, as ``2026-09-16T10:30:00.123Z`` (D-120).

    Exactly three fractional digits and a ``Z`` suffix. ``datetime.isoformat()``
    gives microseconds and ``+00:00``, so a consumer diffing two SDKs' error
    records saw three precisions behind two suffixes. Specifying the suffix
    alone would have left the precisions diverging behind one ``Z`` — the same
    divergence, harder to see — which is why the decision fixes both.

    Produced HERE, at the writer, rather than at a reader: a health summary that
    reformats on the way out leaves every other consumer of the record, and the
    storage backend, on the old form.
    """
    # ONE reading. Two `datetime.now()` calls can straddle a millisecond
    # boundary and splice the seconds from one with the fraction of another.
    now = datetime.now(timezone.utc)
    return f"{now.strftime('%Y-%m-%dT%H:%M:%S')}.{now.microsecond // 1000:03d}Z"


def normalize_message(msg: str) -> str:
    """Replace ephemeral values with placeholders before fingerprint hashing.

    Canonical algorithm (error-history.md § "Error fingerprinting") — exactly five
    steps, no others:
    1. UUID patterns (8-4-4-4-12 hex) → <UUID>
    2. ISO 8601 timestamps → <TIMESTAMP>  (must precede integer step: years are 4 digits)
    3. Integer runs ≥ 4 digits → <ID>
    4. Strip leading/trailing whitespace
    5. Lowercase entire string
    """
    # Step 1: UUID patterns (8-4-4-4-12 hex)
    msg = re.sub(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        "<UUID>",
        msg,
    )
    # Step 2: ISO 8601 timestamps (before integers, to protect 4-digit years)
    msg = re.sub(
        r"\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?",
        "<TIMESTAMP>",
        msg,
    )
    # Step 3: integers > 3 digits (word-boundary on both sides)
    msg = re.sub(r"\b\d{4,}\b", "<ID>", msg)
    # Steps 4 & 5: strip then lowercase.
    return msg.strip().lower()


def compute_fingerprint(error_code: str, module_id: str, message: str) -> str:
    """Compute SHA-256(error_code:module_id:normalized_message) as 64-char hex.

    This is the canonical cross-language fingerprint (error-history.md
    § "Error fingerprinting"). :func:`compute_error_fingerprint`
    is a thin adapter that extracts ``code``/``message`` from an exception
    and delegates here.
    """
    normalized = normalize_message(message)
    raw = f"{error_code}:{module_id}:{normalized}"
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def top_frame_signature(error: BaseException) -> str:
    """Return ``file:lineno:func`` for the deepest frame of ``error.__traceback__``.

    Returns the empty string when no traceback is attached (e.g. errors
    constructed without ``raise``).  Only the basename of the file is used
    so the value is stable across machines / tmp paths.

    This is a **language-local diagnostic** only — it is exposed on
    :class:`ErrorEntry.top_frame` but MUST NOT feed the cross-language
    ``fingerprint`` (stack-frame identities differ per SDK; see
    observability spec §"Error fingerprinting").
    """
    tb = getattr(error, "__traceback__", None)
    if tb is None:
        return ""
    frames = traceback.extract_tb(tb)
    if not frames:
        return ""
    last = frames[-1]
    return f"{os.path.basename(last.filename)}:{last.lineno}:{last.name}"


def compute_error_fingerprint(
    error: ModuleError | BaseException,
    module_id: str,
) -> str:
    """Compute the canonical fingerprint for an exception.

    Adapter over :func:`compute_fingerprint`: extracts the error code
    (``ModuleError.code`` or the exception class name) and message, then
    hashes ``code:module_id:normalized_message``.

    The top stack frame is deliberately **not** part of the digest — it is
    not portable across languages, so including it would break the shared
    cross-language deduplication contract (observability spec §"Error
    fingerprinting"). Use :func:`top_frame_signature` for local diagnostics.

    Returns a 64-char hex digest.
    """
    code = getattr(error, "code", None) or type(error).__name__
    message = getattr(error, "message", None)
    if message is None:
        message = str(error)
    return compute_fingerprint(code, module_id, message)


@dataclass
class ErrorEntry:
    """A single deduplicated error history entry."""

    module_id: str
    code: str
    message: str
    ai_guidance: str | None
    timestamp: str
    count: int
    first_occurred: str
    last_occurred: str
    fingerprint: str = field(default="")
    # Language-local diagnostic only (file:lineno:func of the originating
    # frame). NOT part of ``fingerprint`` — see observability spec
    # §"Error fingerprinting".
    top_frame: str = field(default="")


class ErrorHistory:
    """Thread-safe error tracker with min-heap O(log N) eviction and SHA-256 deduplication.

    Data structure:
      _fp_index: fingerprint → ErrorEntry        O(1) dedup lookup
      _module_index: module_id → deque[entry]    O(1) module get; O(1) popleft eviction
      _heap: min-heap (last_occurred, seq, entry) O(log N) eviction of oldest by last_seen_at

    Heap note: lazy deletion is used — stale heap entries (from dedup timestamp refreshes)
    are skipped on pop. The heap may grow to O(total_records) in a high-dedup workload;
    it is bounded in practice by max_total_entries × dedup_factor.

    Deduplication is keyed on SHA-256(code:module_id:normalize(message)) so ephemeral
    values in messages (UUIDs, timestamps, large integers) do not create duplicate entries.

    _secret_ prefix note: the ``_secret_`` redaction convention (ContextLogger) only applies
    to top-level extra dict keys, not to nested module input fields.  Use ``RedactionConfig``
    with ``field_patterns=["_secret_*"]`` on ``ObsLoggingMiddleware`` to cover input fields.
    """

    def __init__(
        self,
        max_entries_per_module: int = 50,
        max_total_entries: int = 1000,
        store: ObservabilityStore | None = None,
        storage: StorageBackend | None = None,
    ) -> None:
        from apcore.observability.store import InMemoryObservabilityStore

        self._max_entries_per_module = max_entries_per_module
        self._max_total_entries = max_total_entries
        self._store: ObservabilityStore = store if store is not None else InMemoryObservabilityStore()
        # Optional generic storage backend (Issue #43 §1).  When supplied,
        # error entries are mirrored to the ``"errors"`` namespace using the
        # fingerprint as the key, enabling cross-process persistence.
        # D-113: an OMITTED backend means the bundled in-memory one, not "no
        # storage". Only apcore-typescript honoured that, so the same omission
        # produced a working store there and a silent no-op here — and a caller
        # reading records back got an empty list rather than an error.
        self._storage: StorageBackend = storage if storage is not None else InMemoryStorageBackend()
        self._lock = threading.Lock()
        self._fp_index: dict[str, ErrorEntry] = {}
        # Insertion order per fingerprint, for `get_all`'s tie-break.
        #
        # D-120 cut the recorded precision to milliseconds, and sorting on
        # the recorded string alone then leaves same-millisecond entries in
        # an order a stable sort preserves rather than reverses — three
        # errors logged in one millisecond came back oldest-first. The
        # WIRE precision and the ORDERING key are different concerns, and
        # this was fragile before the change too: microsecond stamps are
        # not guaranteed distinct either.
        self._order: dict[str, int] = {}
        self._module_index: dict[str, deque[ErrorEntry]] = {}
        # min-heap: (last_occurred_str, seq, entry)
        self._heap: list[tuple[str, int, ErrorEntry]] = []
        self._seq = 0

    @property
    def store(self) -> ObservabilityStore:
        return self._store

    @property
    def storage(self) -> StorageBackend | None:
        return self._storage

    def record(self, module_id: str, error: ModuleError) -> None:
        """Record an error, deduplicating by fingerprint.

        Dedup is keyed on the canonical cross-language fingerprint
        ``SHA-256(code:module_id:normalized_message)``. The originating
        stack frame is captured into ``ErrorEntry.top_frame`` purely as a
        local diagnostic and does NOT influence dedup.
        """
        now = utc_millis_z()
        fp = compute_error_fingerprint(error, module_id)
        with self._lock:
            existing = self._fp_index.get(fp)
            if existing is not None:
                existing.count += 1
                existing.last_occurred = now
                self._seq += 1
                self._order[fp] = self._seq
                heapq.heappush(self._heap, (now, self._seq, existing))
                entry_to_notify = existing
            else:
                entry = ErrorEntry(
                    module_id=module_id,
                    code=error.code,
                    message=error.message,
                    ai_guidance=error.ai_guidance,
                    timestamp=now,
                    count=1,
                    first_occurred=now,
                    last_occurred=now,
                    fingerprint=fp,
                    top_frame=top_frame_signature(error),
                )
                self._fp_index[fp] = entry
                self._module_index.setdefault(module_id, deque()).append(entry)
                self._seq += 1
                self._order[fp] = self._seq
                heapq.heappush(self._heap, (now, self._seq, entry))
                self._evict_module(module_id)
                self._evict_total()
                entry_to_notify = entry
        # Notify store outside the internal lock to avoid lock-ordering issues.
        self._store.record_error(entry_to_notify)
        if self._storage is not None:
            # D-113: the canonical namespace. This SDK wrote `errors` and
            # apcore-rust wrote `error_history`, so the same records were
            # unqueryable across the two — a namespace IS the key a caller reads
            # by, and an unnamed one made the argument write-only.
            self._storage.save(
                STORAGE_NAMESPACE_ERROR_HISTORY,
                entry_to_notify.fingerprint,
                {
                    "module_id": entry_to_notify.module_id,
                    "code": entry_to_notify.code,
                    "message": entry_to_notify.message,
                    "ai_guidance": entry_to_notify.ai_guidance,
                    "timestamp": entry_to_notify.timestamp,
                    "count": entry_to_notify.count,
                    "first_occurred": entry_to_notify.first_occurred,
                    "last_occurred": entry_to_notify.last_occurred,
                    "fingerprint": entry_to_notify.fingerprint,
                },
            )

    def stored_entries(self) -> list[tuple[str, dict[str, Any]]]:
        """Every persisted error record, newest namespace first (D-113).

        DUAL-READ MIGRATION WINDOW. Writes go only to
        ``error_history``; reads also check the legacy ``errors`` namespace
        this SDK used before the decision, because a rename is invisible until
        someone queries old data and finds nothing. A fingerprint present in
        both is returned once, from the canonical namespace.

        Remove the legacy half when the window closes — the migration note in
        `observability.md` names the release.
        """
        if self._storage is None:
            return []
        canonical = self._storage.list(STORAGE_NAMESPACE_ERROR_HISTORY)
        seen = {key for key, _ in canonical}
        legacy = [
            (key, value) for key, value in self._storage.list(STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY) if key not in seen
        ]
        return canonical + legacy

    def get(self, module_id: str, limit: int | None = None) -> list[ErrorEntry]:
        """Return entries for a module, newest first (by insertion order)."""
        with self._lock:
            module_entries = self._module_index.get(module_id, deque())
            result = list(reversed(module_entries))
        if limit is not None:
            result = result[:limit]
        return result

    def get_all(self, limit: int | None = None) -> list[ErrorEntry]:
        """Return all entries sorted by last_occurred, newest first."""
        with self._lock:
            all_entries = list(self._fp_index.values())
        all_entries.sort(key=lambda e: (e.last_occurred, self._order.get(e.fingerprint, 0)), reverse=True)
        if limit is not None:
            all_entries = all_entries[:limit]
        return all_entries

    def _evict_module(self, module_id: str) -> None:
        """Remove oldest entries (O(1) popleft) when per-module limit is exceeded."""
        module_entries = self._module_index.get(module_id)
        if module_entries is None:
            return
        while len(module_entries) > self._max_entries_per_module:
            evicted = module_entries.popleft()
            self._fp_index.pop(evicted.fingerprint, None)
            self._order.pop(evicted.fingerprint, None)

    def _evict_total(self) -> None:
        """Evict the oldest entry (by last_occurred) until total is within limit."""
        while len(self._fp_index) > self._max_total_entries:
            self._pop_oldest()

    def _pop_oldest(self) -> None:
        """Pop the oldest live entry from the heap and remove it from all indexes."""
        while self._heap:
            ts, _, entry = heapq.heappop(self._heap)
            # Skip stale heap entries: fingerprint evicted already, or refreshed by dedup.
            if entry.fingerprint in self._fp_index and entry.last_occurred == ts:
                self._fp_index.pop(entry.fingerprint, None)
                self._order.pop(entry.fingerprint, None)
                module_entries = self._module_index.get(entry.module_id)
                if module_entries is not None:
                    try:
                        module_entries.remove(entry)
                    except ValueError:
                        pass
                    if not module_entries:
                        self._module_index.pop(entry.module_id, None)
                return


__all__ = [
    "ErrorEntry",
    "ErrorHistory",
    "normalize_message",
    "compute_fingerprint",
    "compute_error_fingerprint",
    "top_frame_signature",
]
