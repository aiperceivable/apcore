"""Lightweight semver utilities and versioned storage for module version negotiation (F18)."""

from __future__ import annotations

import re
import threading
from typing import Generic, TypeVar

from apcore.errors import VersionConstraintError

__all__ = [
    "parse_semver",
    "matches_version_hint",
    "select_best_version",
    "VersionedStore",
]

T = TypeVar("T")

_SEMVER_RE = re.compile(r"^(\d+)(?:\.(\d+))?(?:\.(\d+))?")
# Operand must start with a digit — rejects "v1.0", "not_a_version", a bare
# operator with no operand, or empty input. Prevents malformed constraints
# from silently degrading to (0,0,0) comparisons that always pass.
_CONSTRAINT_RE = re.compile(r"^(>=|<=|>|<|\^|~|=)?(\d[\w.\-+]*)$")


def parse_semver(version: str) -> tuple[int, int, int]:
    """Parse a version string into a (major, minor, patch) tuple.

    Supports full semver (1.2.3), major.minor (1.2), and major-only (1).
    Returns (0, 0, 0) for unparseable input — callers that need strictness
    (e.g., ``_check_single_constraint``) validate first via the regex.
    """
    m = _SEMVER_RE.match(version.strip())
    if m is None:
        return (0, 0, 0)
    major = int(m.group(1))
    minor = int(m.group(2)) if m.group(2) is not None else 0
    patch = int(m.group(3)) if m.group(3) is not None else 0
    return (major, minor, patch)


def _caret_upper_bound(target: tuple[int, int, int]) -> tuple[int, int, int]:
    """Compute the exclusive upper bound for a caret (^) constraint.

    npm/Cargo semantics:
      ^1.2.3 -> <2.0.0     (bump major, zero out rest)
      ^0.2.3 -> <0.3.0     (bump minor when major is 0)
      ^0.0.3 -> <0.0.4     (bump patch when major and minor are 0)
    """
    major, minor, patch = target
    if major > 0:
        return (major + 1, 0, 0)
    if minor > 0:
        return (0, minor + 1, 0)
    return (0, 0, patch + 1)


def _tilde_upper_bound(
    target: tuple[int, int, int],
    part_count: int,
) -> tuple[int, int, int]:
    """Compute the exclusive upper bound for a tilde (~) constraint.

    npm semantics (based on how many version components were supplied):
      ~1.2.3 -> <1.3.0     (3 parts: allow patch bumps)
      ~1.2   -> <1.3.0     (2 parts: allow patch bumps)
      ~1     -> <2.0.0     (1 part: allow minor + patch bumps)
    """
    major, minor, _ = target
    if part_count >= 2:
        return (major, minor + 1, 0)
    return (major + 1, 0, 0)


def _check_single_constraint(version_tuple: tuple[int, int, int], constraint: str) -> bool:
    """Check a single constraint against a version tuple.

    Supported operators: `=`, `>=`, `>`, `<=`, `<`, `^`, `~`. When no operator is
    supplied, the constraint is treated as an exact match (with partial-version
    shortcuts: `"1"` matches any `1.x.x`, `"1.2"` matches any `1.2.x`).

    Raises:
        VersionConstraintError: If the constraint is malformed (empty,
            operator-without-operand, non-digit-leading operand). Callers
            that want soft-failure can catch and handle.
    """
    constraint = constraint.strip()
    if not constraint:
        raise VersionConstraintError(constraint="", reason="empty constraint")
    m = _CONSTRAINT_RE.match(constraint)
    if m is None:
        raise VersionConstraintError(
            constraint=constraint,
            reason="operand must start with a digit (e.g., '1.2.3', not 'v1.2.3' or 'latest')",
        )
    op = m.group(1) or "="
    target = parse_semver(m.group(2))
    parts = m.group(2).strip().split(".")

    if op == "^":
        upper = _caret_upper_bound(target)
        return target <= version_tuple < upper
    if op == "~":
        upper = _tilde_upper_bound(target, len(parts))
        return target <= version_tuple < upper

    # Partial match for bare versions: "1" matches 1.x.x, "1.2" matches 1.2.x
    if op == "=" and len(parts) == 1:
        return version_tuple[0] == target[0]
    if op == "=" and len(parts) == 2:
        return version_tuple[0] == target[0] and version_tuple[1] == target[1]

    if op == "=":
        return version_tuple == target
    if op == ">=":
        return version_tuple >= target
    if op == ">":
        return version_tuple > target
    if op == "<=":
        return version_tuple <= target
    if op == "<":
        return version_tuple < target
    return False


def matches_version_hint(version: str, hint: str) -> bool:
    """Check if a version string satisfies a version hint.

    The hint can be:
    - An exact version: "1.0.0"
    - A partial version: "1" (matches major 1.x.x)
    - A constraint: ">=1.0.0", "<2.0.0"
    - A comma-separated set of constraints: ">=1.0.0,<2.0.0"
    """
    version_tuple = parse_semver(version)
    constraints = [c.strip() for c in hint.split(",")]
    return all(_check_single_constraint(version_tuple, c) for c in constraints)


def select_best_version(versions: list[str], version_hint: str | None = None) -> str | None:
    """Select the best matching version from a list.

    If version_hint is None, returns the latest (highest) version.
    If version_hint is given, returns the highest version that matches.
    Returns None if no version matches.
    """
    if not versions:
        return None

    sorted_versions = sorted(versions, key=parse_semver)

    if version_hint is None:
        return sorted_versions[-1]

    matching = [v for v in sorted_versions if matches_version_hint(v, version_hint)]
    return matching[-1] if matching else None


class VersionedStore(Generic[T]):
    """Thread-safe storage for multiple versions of items keyed by ID.

    Stores items as dict[module_id, dict[version, T]].
    """

    def __init__(self) -> None:
        self._data: dict[str, dict[str, T]] = {}
        self._lock = threading.RLock()

    def add(self, module_id: str, version: str, item: T) -> None:
        """Add an item for a given module_id and version."""
        with self._lock:
            if module_id not in self._data:
                self._data[module_id] = {}
            self._data[module_id][version] = item

    def get(self, module_id: str, version: str) -> T | None:
        """Get a specific version of an item. Returns None if not found."""
        with self._lock:
            versions = self._data.get(module_id)
            if versions is None:
                return None
            return versions.get(version)

    def get_latest(self, module_id: str) -> T | None:
        """Get the latest (highest semver) version of an item."""
        with self._lock:
            versions = self._data.get(module_id)
            if not versions:
                return None
            best = select_best_version(list(versions.keys()), version_hint=None)
            return versions[best] if best else None

    def resolve(self, module_id: str, version_hint: str | None) -> T | None:
        """Resolve a module by ID and optional version hint."""
        with self._lock:
            versions = self._data.get(module_id)
            if not versions:
                return None
            best = select_best_version(list(versions.keys()), version_hint=version_hint)
            return versions[best] if best else None

    def list_versions(self, module_id: str) -> list[str]:
        """List all registered versions for a module_id, sorted by semver."""
        with self._lock:
            versions = self._data.get(module_id, {})
            return sorted(versions.keys(), key=parse_semver)

    def list_ids(self) -> list[str]:
        """List all unique module IDs."""
        with self._lock:
            return list(self._data.keys())

    def remove(self, module_id: str, version: str) -> T | None:
        """Remove a specific version. Returns the removed item or None."""
        with self._lock:
            versions = self._data.get(module_id)
            if versions is None:
                return None
            item = versions.pop(version, None)
            if not versions:
                del self._data[module_id]
            return item

    def remove_all(self, module_id: str) -> dict[str, T]:
        """Remove all versions for a module_id. Returns removed versions."""
        with self._lock:
            return self._data.pop(module_id, {})

    def has(self, module_id: str) -> bool:
        """Check if any version of a module_id is registered."""
        with self._lock:
            return module_id in self._data and len(self._data[module_id]) > 0

    def has_version(self, module_id: str, version: str) -> bool:
        """Check if a specific version is registered."""
        with self._lock:
            return module_id in self._data and version in self._data[module_id]
