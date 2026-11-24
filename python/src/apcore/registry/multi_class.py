"""Multi-class module discovery: opt-in scanner, snake_case ID derivation, conflict detection.

Implements PROTOCOL_SPEC §2.1.1 (Multi-Module Discovery).
"""

from __future__ import annotations

import inspect
import logging
import re
from pathlib import Path

from apcore.errors import IdTooLongError, InvalidSegmentError, ModuleIdConflictError
from apcore.registry.entry_point import PreApprovalHook, _import_module_from_file

logger = logging.getLogger(__name__)

__all__ = [
    "multi_class",
    "class_name_to_segment",
    "discover_multi_class",
    "MAX_MODULE_ID_LEN",
]

_MULTI_CLASS_ATTR = "_apcore_multi_class"
_SEGMENT_RE = re.compile(r"^[a-z][a-z0-9_]*$")
_CANONICAL_ID_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")
MAX_MODULE_ID_LEN: int = 192


def multi_class(cls: type) -> type:
    """Opt a class into multi-class discovery in apcore.

    Apply to each class in a file to enable multi-class scanning for that file.
    Each decorated Module class receives its own ID of the form
    ``base_id.class_segment`` where ``class_segment`` is the snake_case
    conversion of the class name.

    A file with exactly one decorated Module class still receives the bare
    ``base_id`` (single-class identity guarantee).
    """
    setattr(cls, _MULTI_CLASS_ATTR, True)
    return cls


def class_name_to_segment(class_name: str) -> str:
    """Convert a class name to a snake_case segment per PROTOCOL_SPEC §2.1.1.

    Algorithm:
    1. Insert word boundaries at CamelCase transitions (e.g. HTTPSender → HTTP_Sender).
    2. Replace every non-alphanumeric character with ``_``.
    3. Lowercase.
    4. Collapse consecutive ``_`` to a single ``_``.
    5. Strip leading and trailing ``_``.
    """
    # Handle ALLCAPS run followed by capitalized word: HTTPSender → HTTP_Sender
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", class_name)
    # Handle transition from lowercase/digit to uppercase: MathOps → Math_Ops
    s = re.sub(r"([a-z\d])([A-Z])", r"\1_\2", s)
    # Replace non-alphanumeric with _
    s = re.sub(r"[^a-zA-Z0-9]", "_", s)
    # Lowercase
    s = s.lower()
    # Collapse consecutive _
    s = re.sub(r"_+", "_", s)
    return s.strip("_")


def _looks_like_module(cls: type) -> bool:
    """Return True if cls has the Module shape, marked or not."""
    return hasattr(cls, "input_schema") and hasattr(cls, "output_schema") and callable(getattr(cls, "execute", None))


def _is_multi_class_candidate(cls: type) -> bool:
    """Return True if cls is @multi_class-decorated and looks like a Module."""
    return bool(getattr(cls, _MULTI_CLASS_ATTR, False)) and _looks_like_module(cls)


def _compute_base_id(file_path: Path, extensions_root: str) -> str:
    """Compute the base module ID for a file using Algorithm A01.

    Walks ``file_path.parts`` looking for a component equal to ``extensions_root``
    and derives the ID from the relative path beneath it.  If ``extensions_root``
    is not found (e.g. the caller passes an absolute path from outside the
    extensions tree), the ID falls back to the bare file stem — all directory
    context is dropped.  Callers that need directory-aware IDs must ensure
    ``extensions_root`` appears somewhere in ``file_path``.
    """
    parts = file_path.parts
    try:
        root_idx = next(i for i, p in enumerate(parts) if p == extensions_root)
        rel = Path(*parts[root_idx + 1 :])
    except StopIteration:
        # extensions_root not found: drop directory context, use stem only.
        rel = Path(file_path.stem)
    return str(rel.with_suffix("")).replace("\\", "/").replace("/", ".")


def discover_multi_class(
    file_path: str | Path,
    extensions_root: str = "extensions",
    pre_approval_hook: PreApprovalHook | None = None,
) -> list[tuple[str, type]]:
    """Discover all @multi_class Module implementations in a single file.

    Implements the ``Registry.discover_multi_class`` contract from PROTOCOL_SPEC.

    Args:
        file_path: Path to the Python file to scan.
        extensions_root: Root directory name used by Algorithm A01 to derive
            the base module ID.
        pre_approval_hook: Optional callable invoked with ``file_path`` before
            the file is imported.  Raise from the hook to reject the load
            (wrapped as ``ModuleLoadError``).  Mirrors the same parameter on
            ``resolve_entry_point`` — use it for signature verification, path
            allowlists, or audit logging.

    Returns:
        List of ``(module_id, class_ref)`` pairs, one per marked class. A file
        whose only Module class is marked returns ``[(base_id, cls)]``
        (single-class identity guarantee — no segment appended); in a file with
        two or more Module classes each marked class gets ``base_id.segment``
        and unmarked ones are not registered (D-147).

    Raises:
        ModuleIdConflictError: Two or more classes produce the same segment;
            no modules from the file are registered.
        InvalidSegmentError: A derived segment does not conform to the
            canonical ID grammar (``^[a-z][a-z0-9_]*$``).
        IdTooLongError: The derived ``module_id`` exceeds 192 characters.
        ModuleLoadError: The file cannot be imported or was rejected by the
            pre-approval hook.
    """
    file_path = Path(file_path)
    base_id = _compute_base_id(file_path, extensions_root)

    loaded = _import_module_from_file(file_path, pre_approval_hook=pre_approval_hook)

    module_classes: list[tuple[str, type]] = [
        (name, obj)
        for name, obj in inspect.getmembers(loaded, inspect.isclass)
        if _looks_like_module(obj) and obj.__module__ == loaded.__name__
    ]
    qualifying = [(name, obj) for name, obj in module_classes if _is_multi_class_candidate(obj)]

    if not qualifying:
        return []

    # Single-class identity guarantee (§2.1.1 rule 5) is about the FILE: only a
    # file whose one Module class is marked keeps the bare base_id. Beside an
    # unmarked Module class the marked one takes its segment (D-147), so marking
    # a second class later never renames the first.
    if len(module_classes) == 1:
        return [(base_id, qualifying[0][1])]

    # Multi-class path: derive IDs, detect conflicts, validate grammar and length
    seen_segments: dict[str, str] = {}  # segment → class_name (for conflict reporting)
    results: list[tuple[str, type]] = []

    for class_name, cls in qualifying:
        segment = class_name_to_segment(class_name)

        if not _SEGMENT_RE.match(segment):
            raise InvalidSegmentError(
                file_path=str(file_path),
                class_name=class_name,
                segment=segment,
            )

        if segment in seen_segments:
            logger.error(
                "Module ID conflict in '%s': classes '%s' and '%s' both produce segment '%s'",
                file_path,
                seen_segments[segment],
                class_name,
                segment,
            )
            raise ModuleIdConflictError(
                file_path=str(file_path),
                class_names=[seen_segments[segment], class_name],
                conflicting_segment=segment,
            )
        seen_segments[segment] = class_name

        module_id = f"{base_id}.{segment}"

        if not _CANONICAL_ID_RE.match(module_id):
            raise InvalidSegmentError(
                file_path=str(file_path),
                class_name=class_name,
                segment=segment,
            )

        if len(module_id) > MAX_MODULE_ID_LEN:
            raise IdTooLongError(
                file_path=str(file_path),
                module_id=module_id,
            )

        results.append((module_id, cls))

    return results
