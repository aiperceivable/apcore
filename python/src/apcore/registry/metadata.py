"""Metadata and ID map loading for the registry system."""

from __future__ import annotations

import dataclasses
import logging
from pathlib import Path
from typing import Any

import yaml

from apcore.errors import ConfigError, ConfigNotFoundError
from apcore.module import ModuleAnnotations
from apcore.registry.types import DependencyInfo
from apcore.schema.annotations import merge_annotations, merge_examples

logger = logging.getLogger(__name__)

__all__ = [
    "load_metadata",
    "parse_dependencies",
    "merge_module_metadata",
    "load_id_map",
]


def load_metadata(meta_path: Path) -> dict[str, Any]:
    """Load a *_meta.yaml companion metadata file.

    Returns empty dict if file does not exist (metadata is optional).
    """
    if not meta_path.exists():
        return {}

    content = meta_path.read_text(encoding="utf-8")
    try:
        parsed = yaml.safe_load(content)
    except yaml.YAMLError as e:
        raise ConfigError(message=f"Invalid YAML in metadata file: {meta_path}") from e

    if parsed is None:
        return {}
    if not isinstance(parsed, dict):
        raise ConfigError(message=f"Metadata file must be a YAML mapping: {meta_path}")
    return parsed


def parse_dependencies(deps_raw: Any) -> list[DependencyInfo]:
    """Convert raw dependency dicts from YAML to typed DependencyInfo objects.

    ``deps_raw`` arrives from hand-written filesystem YAML and is described by
    no schema, so anything can appear there. A malformed value MUST degrade to
    an empty dependency list rather than raise: a bare ``AttributeError`` here
    aborts the whole ``discover()`` pass instead of skipping the one bad
    module. Matches the TypeScript (``Array.isArray``) and Rust
    (``.as_array()``) guards.
    """
    if not deps_raw:
        return []

    if not isinstance(deps_raw, list):
        logger.warning(
            "Module 'dependencies' must be a list, got %s; ignoring: %r",
            type(deps_raw).__name__,
            deps_raw,
        )
        return []

    result: list[DependencyInfo] = []
    for dep in deps_raw:
        if not isinstance(dep, dict):
            logger.warning("Dependency entry must be a mapping, skipping: %r", dep)
            continue
        module_id = dep.get("module_id")
        if not module_id:
            logger.warning("Dependency entry missing 'module_id', skipping: %s", dep)
            continue
        result.append(
            DependencyInfo(
                module_id=module_id,
                version=dep.get("version"),
                optional=dep.get("optional", False),
            )
        )
    return result


def _coerce_code_annotations(raw: Any) -> ModuleAnnotations | None:
    """Narrow a module's `annotations` attribute to ``ModuleAnnotations | None``.

    Modules express annotations in three forms in the wild:

    - ``ModuleAnnotations(...)`` instance — pass through.
    - ``dict[str, Any]`` (dict-style annotations) — filter to canonical
      field names and construct a ModuleAnnotations.
    - Anything else (None, MagicMock test stubs, custom objects) —
      return None so :func:`merge_annotations` does not crash on
      ``getattr`` chasing nonexistent fields.
    """
    if isinstance(raw, ModuleAnnotations):
        return raw
    if isinstance(raw, dict):
        valid = {f.name for f in dataclasses.fields(ModuleAnnotations)}
        return ModuleAnnotations(**{k: v for k, v in raw.items() if k in valid})
    return None


def merge_module_metadata(module: Any, meta: dict[str, Any]) -> dict[str, Any]:
    """Merge YAML metadata over code-level attributes per spec §4.13.

    Accepts either a module instance OR a class. ``getattr`` resolves
    instance-level attributes first and then falls through to class
    attributes via Python's normal lookup chain, so passing the instance
    correctly handles modules that set their version/annotations/etc. in
    ``__init__``.

    Scalar fields (description, name, tags, version, documentation) follow
    "YAML wins, code is the fallback". The ``annotations`` field uses
    field-level merging via :func:`apcore.schema.merge_annotations` so a YAML
    annotation override does not blow away unrelated code-set flags. The
    ``examples`` field uses :func:`apcore.schema.merge_examples` (YAML wins
    fully when present). The ``metadata`` field is a shallow dict merge.
    """
    code_desc = getattr(module, "description", "")
    code_name = getattr(module, "name", None)
    code_tags = getattr(module, "tags", [])
    code_version = getattr(module, "version", "1.0.0")
    code_annotations = _coerce_code_annotations(getattr(module, "annotations", None))
    code_examples = getattr(module, "examples", [])
    code_metadata = getattr(module, "metadata", {})
    code_docs = getattr(module, "documentation", None)
    code_deps = getattr(module, "dependencies", [])

    yaml_metadata = meta.get("metadata", {})
    merged_metadata = {**(code_metadata or {}), **(yaml_metadata or {})}

    yaml_annotations = meta.get("annotations")
    merged_annotations: ModuleAnnotations | None
    if yaml_annotations is None and code_annotations is None:
        merged_annotations = None
    else:
        merged_annotations = merge_annotations(yaml_annotations, code_annotations)

    # Defend against unusable code_examples (e.g. MagicMock test stubs):
    # only forward when it is actually a list, otherwise treat as absent.
    safe_code_examples = code_examples if isinstance(code_examples, list) else None

    return {
        "description": meta.get("description") or (code_desc if isinstance(code_desc, str) else ""),
        "name": meta.get("name") or (code_name if isinstance(code_name, (str, type(None))) else None),
        "tags": (
            meta.get("tags") if meta.get("tags") is not None else (code_tags if isinstance(code_tags, list) else [])
        ),
        "version": meta.get("version") or (code_version if isinstance(code_version, str) else "1.0.0"),
        "annotations": merged_annotations,
        "examples": merge_examples(meta.get("examples"), safe_code_examples),
        "metadata": merged_metadata if isinstance(merged_metadata, dict) else {},
        "documentation": meta.get("documentation") or (code_docs if isinstance(code_docs, str) else None),
        # `dependencies` was absent from this dict, so it survived discovery
        # (Registry reads it off raw_metadata before merging) and was then lost
        # before storage — leaving `get_module_metadata()` unable to see it.
        # Load-order topological sorting therefore worked while RELOAD-order
        # sorting silently degenerated: ReloadModule._topo_sort_modules read
        # `meta.get("dependencies", [])` and always got an empty list, so it
        # topologically sorted an empty graph. See apcore-typescript#35.
        "dependencies": (
            meta["dependencies"]
            if isinstance(meta.get("dependencies"), list)
            else (code_deps if isinstance(code_deps, list) else [])
        ),
    }


def load_id_map(id_map_path: Path) -> dict[str, dict[str, Any]]:
    """Load an ID Map YAML file for canonical ID overrides.

    Raises ConfigNotFoundError if file does not exist (ID map is explicitly requested).
    """
    if not id_map_path.exists():
        raise ConfigNotFoundError(config_path=str(id_map_path))

    content = id_map_path.read_text(encoding="utf-8")
    try:
        parsed = yaml.safe_load(content)
    except yaml.YAMLError as e:
        raise ConfigError(message=f"Invalid YAML in ID map file: {id_map_path}") from e

    if not isinstance(parsed, dict) or "mappings" not in parsed:
        raise ConfigError(message="ID map must contain a 'mappings' list")

    mappings = parsed["mappings"]
    if not isinstance(mappings, list):
        raise ConfigError(message="ID map must contain a 'mappings' list")

    result: dict[str, dict[str, Any]] = {}
    for entry in mappings:
        file_path = entry.get("file")
        if not file_path:
            logger.warning("ID map entry missing 'file' field, skipping")
            continue
        # An entry carries `file` and `id` only (protocol-spec §2.2, D-138).
        result[file_path] = {"id": entry.get("id", file_path)}
    return result
