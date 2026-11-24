"""Cross-language module ID normalization (Algorithm A02)."""

from __future__ import annotations

import re
from dataclasses import dataclass
from enum import Enum

__all__ = ["CanonicalNameError", "CanonicalNameResult", "canonicalize_name", "normalize_to_canonical_id"]


class CanonicalNameError(str, Enum):
    """Diagnostics for a bare name that cannot form a canonical segment."""

    EMPTY_NAME = "empty_name"
    NON_ASCII = "non_ascii"
    INVALID_START = "invalid_start"
    NAME_TOO_LONG = "name_too_long"


@dataclass(frozen=True)
class CanonicalNameResult:
    """A lossless original name together with its canonical segment or error."""

    original_name: str
    canonical_name: str | None
    error: CanonicalNameError | None


#: Language-specific separators used to split local IDs.
_SEPARATORS: dict[str, str] = {
    "python": ".",
    "rust": "::",
    "go": ".",
    "java": ".",
    "typescript": ".",
}

_SUPPORTED_LANGUAGES: frozenset[str] = frozenset(_SEPARATORS)

#: Canonical ID format from PROTOCOL_SPEC §2.7 EBNF grammar.
_CANONICAL_ID_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")
_NAME_EDGE_RE = re.compile(r"^[^A-Za-z0-9_]+|[^A-Za-z0-9_]+$")
_NAME_SEPARATOR_RE = re.compile(r"[^A-Za-z0-9_]+")


def canonicalize_name(name: str) -> CanonicalNameResult:
    """Canonicalize one bare ASCII name without throwing for any string.

    Preserves the original and existing underscores; Unicode is rejected,
    never trimmed or transliterated. Module-ID reservation and collision
    handling remain registration/scanning concerns, not name conversion.
    Unlike A02's non-repairing module-ID conversion, this utility trims edge
    punctuation and turns each internal punctuation run into one underscore.
    """
    if not name.isascii():
        return CanonicalNameResult(name, None, CanonicalNameError.NON_ASCII)
    candidate = _NAME_SEPARATOR_RE.sub("_", _to_snake_case(_NAME_EDGE_RE.sub("", name)))
    if not candidate:
        error = CanonicalNameError.EMPTY_NAME
    elif not "a" <= candidate[0] <= "z":
        error = CanonicalNameError.INVALID_START
    elif len(candidate) > 192:
        error = CanonicalNameError.NAME_TOO_LONG
    else:
        return CanonicalNameResult(name, candidate, None)
    return CanonicalNameResult(name, None, error)


def _to_snake_case(segment: str) -> str:
    """Apply A02's non-repairing, ASCII-only identifier case conversion."""
    if not segment or not any("A" <= char <= "Z" for char in segment):
        return segment

    res = []
    for i, char in enumerate(segment):
        if i > 0:
            prev = segment[i - 1]
            # Case 1: lowercase/digit followed by uppercase -> add underscore
            if ("a" <= prev <= "z") or ("0" <= prev <= "9"):
                if "A" <= char <= "Z":
                    res.append("_")
            # Case 2: uppercase followed by uppercase followed by lowercase -> add underscore before the middle one
            # e.g., HTTPAPIHandler: ...PIH... -> ...PI_H...
            elif ("A" <= prev <= "Z") and ("A" <= char <= "Z"):
                if i + 1 < len(segment) and ("a" <= segment[i + 1] <= "z"):
                    res.append("_")
        res.append(char.lower() if "A" <= char <= "Z" else char)

    return "".join(res)


def normalize_to_canonical_id(local_id: str, language: str) -> str:
    """Convert a language-local module ID to Canonical ID format (Algorithm A02).

    Steps:
        1. Split by language-specific separator.
        2. Normalize each segment from PascalCase/camelCase to snake_case.
        3. Join with ``"."`` and validate against Canonical ID EBNF.

    Args:
        local_id: Language-local format ID (e.g. ``"executor::validator::DbParams"``).
        language: Source language (``"python"`` | ``"rust"`` | ``"go"`` | ``"java"`` | ``"typescript"``).

    Returns:
        Dot-separated snake_case Canonical ID.

    Raises:
        ValueError: If *language* is unsupported or the result is not a valid Canonical ID.
    """
    if not local_id:
        raise ValueError("local_id must be a non-empty string")

    if language not in _SUPPORTED_LANGUAGES:
        raise ValueError(
            f"Unsupported language '{language}'. Must be one of: {', '.join(sorted(_SUPPORTED_LANGUAGES))}"
        )

    separator = _SEPARATORS[language]
    segments = local_id.split(separator)

    normalized = [_to_snake_case(seg) for seg in segments]
    canonical_id = ".".join(normalized)

    if not _CANONICAL_ID_RE.match(canonical_id):
        raise ValueError(
            f"Normalized ID '{canonical_id}' (from '{local_id}', language='{language}') "
            f"does not conform to Canonical ID grammar"
        )

    return canonical_id
