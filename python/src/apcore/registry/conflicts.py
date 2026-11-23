"""ID conflict detection (Algorithm A03)."""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum

__all__ = ["detect_id_conflicts", "ConflictResult", "ConflictSeverity", "ConflictType"]


class ConflictSeverity(StrEnum):
    """Severity level for a detected module ID conflict."""

    ERROR = "error"
    WARNING = "warning"


class ConflictType(StrEnum):
    """Category of a detected module ID conflict."""

    DUPLICATE_ID = "duplicate_id"
    RESERVED_WORD = "reserved_word"
    CASE_COLLISION = "case_collision"


@dataclass(frozen=True)
class ConflictResult:
    """Result of an ID conflict check.

    Use :class:`ConflictType` and :class:`ConflictSeverity` members to compare
    ``type`` and ``severity`` values (e.g. ``result.severity == ConflictSeverity.ERROR``).
    """

    type: str  # one of the ConflictType values
    severity: str  # one of the ConflictSeverity values
    message: str


def detect_id_conflicts(
    new_id: str,
    existing_ids: set[str],
    reserved_words: frozenset[str],
    *,
    lowercase_map: dict[str, str] | None = None,
) -> ConflictResult | None:
    """Check if a new module ID conflicts with existing IDs or reserved words (Algorithm A03).

    Steps:
        1. Exact duplicate detection.
        2. Reserved word detection (first segment).
        3. Case collision detection.

    Args:
        new_id: Canonical ID to be registered.
        existing_ids: Set of already registered IDs.
        reserved_words: Reserved words that cannot be used as the first ID segment.
        lowercase_map: Optional pre-built lowercase-to-original_id mapping for O(1) case collision.

    Returns:
        ConflictResult if a conflict is found, None if the ID is safe.
    """
    # Step 1: Exact duplicate
    if new_id in existing_ids:
        return ConflictResult(
            type=ConflictType.DUPLICATE_ID,
            severity=ConflictSeverity.ERROR,
            message=f"Module ID '{new_id}' is already registered",
        )

    # Step 2: Reserved word check (first segment only)
    first_segment = new_id.split(".")[0]
    if first_segment in reserved_words:
        return ConflictResult(
            type=ConflictType.RESERVED_WORD,
            severity=ConflictSeverity.ERROR,
            message=f"Module ID '{new_id}' contains reserved word '{first_segment}'",
        )

    # Step 3: Case collision
    normalized_new = new_id.lower()
    if lowercase_map is not None:
        if normalized_new in lowercase_map and lowercase_map[normalized_new] != new_id:
            existing = lowercase_map[normalized_new]
            return ConflictResult(
                type=ConflictType.CASE_COLLISION,
                severity=ConflictSeverity.WARNING,
                message=f"Module ID '{new_id}' has a case collision with existing '{existing}'",
            )
    else:
        for existing_id in existing_ids:
            if existing_id.lower() == normalized_new and existing_id != new_id:
                return ConflictResult(
                    type=ConflictType.CASE_COLLISION,
                    severity=ConflictSeverity.WARNING,
                    message=f"Module ID '{new_id}' has a case collision with existing '{existing_id}'",
                )

    return None
