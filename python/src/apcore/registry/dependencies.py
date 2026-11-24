"""Dependency resolution via Kahn's topological sort."""

from __future__ import annotations

import logging
from collections import defaultdict, deque

from apcore.errors import (
    CircularDependencyError,
    DependencyNotFoundError,
    DependencyVersionMismatchError,
    ModuleLoadError,
)
from apcore.registry.types import DependencyInfo
from apcore.registry.version import matches_version_hint

logger = logging.getLogger(__name__)

__all__ = ["resolve_dependencies"]


class _SkipDep(Exception):
    """Internal signal: an optional dependency failed its check and should be skipped."""


def _enforce_version_constraint(
    module_id: str,
    dep: DependencyInfo,
    module_versions: dict[str, str] | None,
) -> None:
    """Enforce a single dependency's version constraint.

    Raises:
        DependencyVersionMismatchError: required dep's actual version
            does not satisfy the declared constraint.
        _SkipDep: optional dep whose version does not satisfy — caller
            should log-and-skip instead of raising.
    """
    if not dep.version or module_versions is None:
        return
    actual = module_versions.get(dep.module_id)
    if actual is None or matches_version_hint(actual, dep.version):
        return
    if dep.optional:
        logger.warning(
            "Optional dependency '%s' for module '%s' has version "
            "'%s' which does not satisfy constraint '%s', skipping",
            dep.module_id,
            module_id,
            actual,
            dep.version,
        )
        raise _SkipDep
    raise DependencyVersionMismatchError(
        module_id=module_id,
        dependency_id=dep.module_id,
        required=dep.version,
        actual=actual,
    )


def resolve_dependencies(
    modules: list[tuple[str, list[DependencyInfo]]],
    known_ids: set[str] | None = None,
    module_versions: dict[str, str] | None = None,
) -> list[str]:
    """Resolve module load order using Kahn's topological sort.

    Args:
        modules: List of (module_id, dependencies) tuples.
        known_ids: Set of all module IDs in the batch. If None, derived from modules list.
        module_versions: Optional map of module_id -> registered version string. When
            provided, declared dependency version constraints (per PROTOCOL_SPEC §5.3)
            are enforced against the target module's registered version. Dependencies
            whose target has no entry in this map are accepted without version check.

    Returns:
        List of module_ids in topological load order (dependencies first).

    Raises:
        CircularDependencyError: If circular dependencies are detected.
        DependencyNotFoundError: If a required dependency is not in known_ids.
        DependencyVersionMismatchError: If a declared version constraint is not
            satisfied by the registered version of the target module.
        ModuleLoadError: If Kahn's sort cannot complete and no back-edge cycle
            exists — i.e., modules are blocked on an external or disabled dep
            rather than a true cycle.
    """
    if not modules:
        return []

    if known_ids is None:
        known_ids = {mod_id for mod_id, _ in modules}

    # Build graph and in-degree
    graph: dict[str, set[str]] = defaultdict(set)
    in_degree: dict[str, int] = {mod_id: 0 for mod_id, _ in modules}

    for module_id, deps in modules:
        for dep in deps:
            if dep.module_id not in known_ids:
                if dep.optional:
                    logger.warning(
                        "Optional dependency '%s' for module '%s' not found, skipping",
                        dep.module_id,
                        module_id,
                    )
                    continue
                else:
                    raise DependencyNotFoundError(
                        module_id=module_id,
                        dependency_id=dep.module_id,
                    )
            try:
                _enforce_version_constraint(module_id, dep, module_versions)
            except _SkipDep:
                continue
            graph[dep.module_id].add(module_id)
            in_degree[module_id] += 1

    # Initialize queue with zero-in-degree nodes (sorted for determinism)
    queue: deque[str] = deque(sorted(mod_id for mod_id in in_degree if in_degree[mod_id] == 0))

    load_order: list[str] = []
    while queue:
        mod_id = queue.popleft()
        load_order.append(mod_id)
        for dependent in sorted(graph.get(mod_id, set())):
            in_degree[dependent] -= 1
            if in_degree[dependent] == 0:
                queue.append(dependent)

    # Kahn's sort did not consume all modules — either a real cycle exists or
    # modules are blocked on a known-but-unresolvable peer (e.g., a required dep
    # of an optional dep, which is not modelled in `known_ids` alone).
    if len(load_order) < len(modules):
        remaining = {mod_id for mod_id, _ in modules if mod_id not in set(load_order)}
        cycle_path = _find_back_edge_cycle(modules, remaining)
        if cycle_path is not None:
            raise CircularDependencyError(cycle_path=cycle_path)
        # No true cycle — fail with a specific error naming the blocked modules
        # so callers don't mistake a blocker for a cycle.
        raise ModuleLoadError(
            module_id=",".join(sorted(remaining)),
            reason=(
                f"{len(remaining)} module(s) could not be loaded — "
                f"blocked by unresolved required dependencies: {sorted(remaining)}"
            ),
        )

    return load_order


def _find_back_edge_cycle(
    modules: list[tuple[str, list[DependencyInfo]]],
    remaining: set[str],
) -> list[str] | None:
    """Return a back-edge cycle ``[n0, ..., nk, n0]`` if one exists in ``remaining``, else None.

    Unlike the previous ``_extract_cycle`` which fell back to ``sorted(remaining)``
    on no-cycle, this returns None — letting the caller distinguish between a
    true cycle and a non-cycle blockage (e.g., optional-dep tracking failure).
    """
    dep_map: dict[str, list[str]] = {}
    for mod_id, deps in modules:
        if mod_id in remaining:
            dep_map[mod_id] = sorted({d.module_id for d in deps if d.module_id in remaining})

    for start in sorted(remaining):
        cycle = _dfs_find_cycle(dep_map, start)
        if cycle is not None:
            return cycle

    return None


def _dfs_find_cycle(dep_map: dict[str, list[str]], start: str) -> list[str] | None:
    """Iterative DFS that returns a back-edge cycle [n0, ..., n0] or None.

    Frame-index contract: each stack frame is ``(node, idx)`` where ``idx`` is
    the index of the next neighbor to explore. ``idx == 0`` is the first-visit
    marker (push the node onto ``path`` / ``on_path`` then advance); ``idx > 0``
    means we are resuming the frame after a nested call returned.
    """
    path: list[str] = []
    on_path: set[str] = set()
    visited: set[str] = set()
    stack: list[tuple[str, int]] = [(start, 0)]

    while stack:
        node, idx = stack[-1]
        if idx == 0:
            if node in on_path:
                start_idx = path.index(node)
                return path[start_idx:] + [node]
            if node in visited:
                stack.pop()
                continue
            visited.add(node)
            on_path.add(node)
            path.append(node)

        neighbors = dep_map.get(node, [])
        if idx < len(neighbors):
            stack[-1] = (node, idx + 1)
            stack.append((neighbors[idx], 0))
        else:
            on_path.discard(node)
            path.pop()
            stack.pop()

    return None
