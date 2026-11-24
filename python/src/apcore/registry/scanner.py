"""Directory scanner for discovering Python extension modules."""

from __future__ import annotations

import logging
import os
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from apcore.errors import ConfigError, ConfigNotFoundError
from apcore.registry.types import DiscoveredModule
from apcore.utils.pattern import match_glob

logger = logging.getLogger(__name__)

__all__ = ["scan_extensions", "scan_multi_root"]

_SKIP_DIR_NAMES = {"__pycache__", "node_modules"}
_SKIP_FILE_SUFFIXES = {".pyc"}


def scan_extensions(
    root: Path,
    max_depth: int = 8,
    follow_symlinks: bool = False,
    ignore_patterns: Sequence[str] | None = None,
) -> list[DiscoveredModule]:
    """Recursively scan an extensions directory for Python module files.

    `ignore_patterns` is `extensions.ignore_patterns`, matched with Algorithm
    A25 (PROTOCOL_SPEC §9.2.3) against the ENTRY NAME — one path segment, never
    a path, case-sensitively. It is a UNION with `_SKIP_DIR_NAMES` and the
    hidden/internal prefixes of §3.5: a configured pattern adds to those and
    cannot switch one off.

    Until spec v1.42.0 the key was registered in every SDK's configuration key
    surface and read by none, so §3.6 step 3a was a MUST whose input nothing
    supplied and a directory a project had excluded from discovery was scanned
    and registered anyway — a skip rule that failed OPEN (apcore#118).
    """
    root = Path(root).resolve()
    if not root.exists():
        raise ConfigNotFoundError(config_path=str(root))

    # Empty entries are dropped rather than treated as a pattern matching
    # nothing in particular: A25 anchors, so `""` would match only the empty
    # name, but an operator who leaves a blank line in a YAML list means
    # nothing by it.
    patterns = [p for p in (ignore_patterns or ()) if p]

    visited_real_paths: set[Path] = {root.resolve()}
    results: list[DiscoveredModule] = []
    seen_ids: dict[str, Path] = {}
    seen_ids_lower: dict[str, str] = {}

    def _scan_dir(dir_path: Path, depth: int) -> None:
        if depth > max_depth:
            logger.info("Max depth %d exceeded at %s, skipping", max_depth, dir_path)
            return

        try:
            entries = list(os.scandir(dir_path))
        except PermissionError as e:
            logger.error("Permission denied scanning %s: %s", dir_path, e)
            return
        except OSError as e:
            logger.error("OS error scanning %s: %s", dir_path, e)
            return

        for entry in entries:
            name = entry.name
            if name.startswith(".") or name.startswith("_"):
                continue
            if name in _SKIP_DIR_NAMES:
                continue
            if any(match_glob(pattern, name) for pattern in patterns):
                continue

            try:
                is_dir = entry.is_dir(follow_symlinks=follow_symlinks)
                is_file = entry.is_file(follow_symlinks=follow_symlinks)
                is_symlink = entry.is_symlink()
            except OSError as e:
                logger.error("OS error accessing %s: %s", entry.path, e)
                continue

            entry_path = Path(entry.path)

            # Symlink handling runs BEFORE the dir/file split, because both
            # branches are reachable through a symlink and only one of them was
            # ever guarded. The confinement check below used to live inside the
            # `is_dir` branch, so a symlinked FILE whose target escaped the root
            # -- `extensions/evil.py -> /outside/evil.py` -- reached `elif
            # is_file` with no check at all and was discovered, and the registry
            # then imported and executed it. That is the exact failure the
            # check's own comment describes, on the one branch that actually
            # yields importable files. apcore-typescript (scanner.ts) and
            # apcore-rust (scanner.rs) both check before the split; this now
            # matches them.
            if is_symlink:
                if not follow_symlinks:
                    continue
                real = entry_path.resolve()
                # Confinement: refuse any symlink whose real path escapes the
                # extension root, whether it resolves to a directory or a file.
                try:
                    real.relative_to(root)
                except ValueError:
                    logger.warning(
                        "Symlink target outside extension root, skipping: %s -> %s",
                        entry_path,
                        real,
                    )
                    continue

            if is_dir:
                if is_symlink:
                    real = entry_path.resolve()
                    if real in visited_real_paths:
                        logger.warning(
                            "Symlink cycle detected at %s -> %s, skipping",
                            entry_path,
                            real,
                        )
                        continue
                    # Confinement already ran above, before the dir/file
                    # split, so a target outside the root never reaches here.
                    visited_real_paths.add(real)
                _scan_dir(entry_path, depth + 1)
            elif is_file:
                suffix = Path(name).suffix
                if suffix in _SKIP_FILE_SUFFIXES:
                    continue
                if suffix != ".py":
                    continue

                # D-127: identity and the module ID are keyed on the CANONICAL
                # REAL PATH, never on whichever alias the traversal reached
                # first. Deriving the ID from the alias makes the registered ID
                # depend on directory iteration order, which is not stable
                # across filesystems or platforms — and recording both paths
                # made one file two modules with different IDs, which the
                # duplicate check below cannot catch precisely because they
                # differ.
                real_file = entry_path.resolve()
                if real_file in visited_real_paths:
                    continue
                try:
                    rel = real_file.relative_to(root)
                except ValueError:
                    # Containment already ran before the dir/file split (D-94);
                    # a target outside the root never reaches here. Defensive.
                    logger.warning("Resolved path escapes the extensions root, skipping: %s", entry_path)
                    continue
                visited_real_paths.add(real_file)
                canonical_id = str(rel.with_suffix("")).replace(os.sep, ".")

                if canonical_id in seen_ids:
                    logger.error(
                        "Duplicate module ID '%s' at %s, already found at %s. Skipping.",
                        canonical_id,
                        entry_path,
                        seen_ids[canonical_id],
                    )
                    continue

                lower_id = canonical_id.lower()
                if lower_id in seen_ids_lower and seen_ids_lower[lower_id] != canonical_id:
                    logger.warning(
                        "Case collision: '%s' and '%s' differ only by case",
                        canonical_id,
                        seen_ids_lower[lower_id],
                    )

                meta_path = entry_path.with_name(entry_path.stem + "_meta.yaml")
                if not meta_path.exists():
                    meta_path = None

                dm = DiscoveredModule(
                    file_path=real_file,
                    canonical_id=canonical_id,
                    meta_path=meta_path,
                    namespace=None,
                )
                seen_ids[canonical_id] = real_file
                seen_ids_lower[lower_id] = canonical_id
                results.append(dm)

    _scan_dir(root, depth=1)
    return results


def scan_multi_root(
    roots: list[dict[str, Any]],
    max_depth: int = 8,
    follow_symlinks: bool = False,
    ignore_patterns: Sequence[str] | None = None,
) -> list[DiscoveredModule]:
    """Scan multiple extension roots with namespace prefixing."""
    all_results: list[DiscoveredModule] = []
    seen_namespaces: set[str] = set()

    # Validate all namespaces before scanning
    resolved: list[tuple[Path, str]] = []
    for entry in roots:
        root_path = Path(entry["root"])
        namespace = entry.get("namespace") or root_path.name
        if namespace in seen_namespaces:
            raise ConfigError(message=f"Duplicate namespace: '{namespace}'")
        seen_namespaces.add(namespace)
        resolved.append((root_path, namespace))

    for root_path, namespace in resolved:
        modules = scan_extensions(
            root_path,
            max_depth=max_depth,
            follow_symlinks=follow_symlinks,
            ignore_patterns=ignore_patterns,
        )
        for m in modules:
            all_results.append(
                DiscoveredModule(
                    file_path=m.file_path,
                    canonical_id=f"{namespace}.{m.canonical_id}",
                    meta_path=m.meta_path,
                    namespace=namespace,
                )
            )

    return all_results
