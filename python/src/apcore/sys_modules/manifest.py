"""System modules for manifests and capability discovery."""

from __future__ import annotations

from dataclasses import asdict, replace
from typing import Any

from apcore.config import Config
from apcore.context import Context
from apcore.errors import InvalidInputError, ModuleNotFoundError
from apcore.module import ModuleAnnotations
from apcore.registry.registry import Registry
from apcore.registry.types import DependencyInfo
from apcore.schema.annotations import governance_union

__all__ = ["ManifestFullModule", "ManifestModule", "ManifestModuleModule", "ManifestSearchModule"]


def _compute_source_path(config: Config | None, module_id: str) -> str | None:
    """Compute the source file path from project.source_root and module_id."""
    if config is None:
        return None
    source_root = config.get("project.source_root", "")
    if not source_root:
        return None
    relative_path = module_id.replace(".", "/") + ".py"
    return f"{source_root}/{relative_path}"


def _serialize_annotations(
    annotations: ModuleAnnotations | None,
) -> dict[str, Any] | None:
    """Convert ModuleAnnotations dataclass to a dict."""
    if annotations is None:
        return None
    return asdict(annotations)


def _governance_view(
    registry: Any,
    module_id: str,
    descriptor_annotations: ModuleAnnotations | None,
) -> dict[str, Any] | None:
    """Project the annotations an agent will actually be held to.

    The two governance flags come from the D-96 union of the live module
    instance and the registry's declared annotations — the same source the
    approval gate reads. Everything else is the descriptor's, unchanged.

    The manifest is what an agent reads to decide whether to call a module, so
    advertising a governance value the gate does not enforce is worse than
    advertising none: the descriptor alone could say `requires_approval: false`
    for a module whose instance declares it, and `true` for one the gate lets
    straight through.
    """
    if descriptor_annotations is None:
        return None
    module = registry.get(module_id) if hasattr(registry, "get") else None
    declared = registry.get_declared_annotations(module_id) if hasattr(registry, "get_declared_annotations") else None
    effective = governance_union(getattr(module, "annotations", None), declared)
    view = asdict(descriptor_annotations)
    if effective is not None:
        view["requires_approval"] = effective.requires_approval
        view["destructive"] = effective.destructive
    return view


def _dependency_items(dependencies: list[DependencyInfo]) -> list[dict[str, Any]]:
    items = []
    for dependency in dependencies:
        item: dict[str, Any] = {"module_id": dependency.module_id, "optional": dependency.optional}
        if dependency.version is not None:
            item["version"] = dependency.version
        items.append(item)
    return items


class ManifestModule:
    """Return the full manifest (metadata, schemas, annotations, source path) for a single registered module."""

    description = "Full manifest for a registered module including source path"
    # D-119: written out rather than inherited — the ModuleAnnotations default is
    # `open_world=True`, which means the opposite of the intended value, and relying
    # on it is how the divergence arose. No system module reaches an external system.
    annotations = ModuleAnnotations(readonly=True, idempotent=True, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "module_id": {
                "type": "string",
                "description": "ID of the module to inspect",
            },
        },
        "required": ["module_id"],
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "additionalProperties": False,
        "properties": {
            "module_id": {"type": "string", "description": "Module identifier"},
            "description": {"type": "string", "description": "Module description"},
            "documentation": {"type": ["string", "null"], "description": "Module documentation (Markdown)"},
            "source_path": {
                "type": ["string", "null"],
                "description": "Computed source file path",
            },
            "input_schema": {
                "type": ["object", "null"],
                "description": "Module input JSON Schema",
            },
            "output_schema": {
                "type": ["object", "null"],
                "description": "Module output JSON Schema",
            },
            "annotations": {"type": ["object", "null"], "description": "Module annotations"},
            "tags": {"type": "array", "items": {"type": "string"}, "description": "Module tags"},
            "dependencies": {
                "type": "array",
                "description": "Module dependencies",
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["module_id"],
                    "properties": {
                        "module_id": {"type": "string"},
                        "version": {"type": "string"},
                        "optional": {"type": "boolean"},
                    },
                },
            },
            "metadata": {"type": ["object", "null"], "description": "Additional metadata"},
        },
        "required": ["module_id", "description"],
    }

    def __init__(
        self,
        registry: Registry,
        config: Config | None = None,
    ) -> None:
        self._registry = registry
        self._config = config

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        """Execute the manifest module lookup.

        Args:
            inputs: Must contain 'module_id' (str).
            context: Execution context (unused).

        Returns:
            Full manifest dict for the requested module.

        Raises:
            InvalidInputError: If module_id is missing or empty.
            ModuleNotFoundError: If the module is not registered.
        """
        module_id = inputs.get("module_id")
        if not module_id:
            raise InvalidInputError(message="module_id is required")

        descriptor = self._registry.get_definition(module_id)
        if descriptor is None:
            raise ModuleNotFoundError(module_id=module_id)

        source_path = _compute_source_path(self._config, module_id)
        annotations_dict = _governance_view(self._registry, module_id, descriptor.annotations)
        metadata = self._get_metadata(module_id)

        return {
            "module_id": descriptor.module_id,
            "description": descriptor.description,
            "documentation": descriptor.documentation,
            "source_path": source_path,
            "input_schema": descriptor.input_schema,
            "output_schema": descriptor.output_schema,
            "annotations": annotations_dict,
            "tags": descriptor.tags,
            "dependencies": _dependency_items(descriptor.dependencies),
            "metadata": metadata,
        }

    def _get_metadata(self, module_id: str) -> dict[str, Any]:
        """Retrieve extra metadata from registry metadata."""
        meta = self._registry.get_module_metadata(module_id)
        return meta.get("metadata", {})


#: Backward-compatible alias for :class:`ManifestModule`.
ManifestModuleModule = ManifestModule


class ManifestFullModule:
    """Return a complete system manifest with all registered modules, supporting filtering."""

    description = "Complete system manifest with filtering by prefix and tags"
    # D-119: written out rather than inherited — the ModuleAnnotations default is
    # `open_world=True`, which means the opposite of the intended value, and relying
    # on it is how the divergence arose. No system module reaches an external system.
    annotations = ModuleAnnotations(readonly=True, idempotent=True, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "include_schemas": {
                "type": "boolean",
                "description": "Whether to include input/output schemas",
                "default": True,
            },
            "include_source_paths": {
                "type": "boolean",
                "description": "Whether to include source paths",
                "default": True,
            },
            "prefix": {"type": "string", "description": "Filter modules by ID prefix"},
            "tags": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Filter modules by tags",
            },
        },
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "additionalProperties": False,
        "properties": {
            "project_name": {
                "type": "string",
                "description": "Project name from config",
            },
            "module_count": {
                "type": "integer",
                "description": "Number of modules returned",
            },
            "modules": {
                "type": "array",
                "description": "Module manifest entries",
                "items": ManifestModule.output_schema,
            },
        },
        "required": ["project_name", "module_count", "modules"],
    }

    def __init__(
        self,
        registry: Registry,
        config: Config | None = None,
    ) -> None:
        self._registry = registry
        self._config = config

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        """Execute the full manifest query.

        Args:
            inputs: Optional keys: include_schemas (bool), include_source_paths (bool),
                    prefix (str|None), tags (list[str]|None).
            context: Execution context (unused).

        Returns:
            Dict with project info, module_count, and modules array.
        """
        include_schemas: bool = inputs.get("include_schemas", True)
        include_source_paths: bool = inputs.get("include_source_paths", True)
        prefix: str | None = inputs.get("prefix")
        tags: list[str] | None = inputs.get("tags")

        module_ids = self._registry.list(prefix=prefix, tags=tags)
        modules = self._build_module_entries(
            module_ids,
            include_schemas,
            include_source_paths,
        )

        project_name = self._get_project_name()

        return {
            "project_name": project_name,
            "module_count": len(modules),
            "modules": modules,
        }

    def _build_module_entries(
        self,
        module_ids: list[str],
        include_schemas: bool,
        include_source_paths: bool,
    ) -> list[dict[str, Any]]:
        """Build manifest entry dicts for the given module IDs."""
        entries: list[dict[str, Any]] = []
        for mid in module_ids:
            entry = self._build_single_entry(mid, include_schemas, include_source_paths)
            if entry is not None:
                entries.append(entry)
        return entries

    def _build_single_entry(
        self,
        module_id: str,
        include_schemas: bool,
        include_source_paths: bool,
    ) -> dict[str, Any] | None:
        """Build a single module manifest entry dict."""
        descriptor = self._registry.get_definition(module_id)
        if descriptor is None:
            return None

        source_path = _compute_source_path(self._config, module_id) if include_source_paths else None
        annotations_dict = _governance_view(self._registry, module_id, descriptor.annotations)
        metadata = descriptor.metadata or {}

        return {
            "module_id": descriptor.module_id,
            "description": descriptor.description,
            "documentation": descriptor.documentation,
            "source_path": source_path,
            "input_schema": descriptor.input_schema if include_schemas else None,
            "output_schema": descriptor.output_schema if include_schemas else None,
            "annotations": annotations_dict,
            "tags": descriptor.tags,
            "dependencies": _dependency_items(descriptor.dependencies),
            "metadata": metadata,
        }

    #: D-110. `system.health.summary` already reported ``"apcore"`` here while
    #: this module reported ``""``, so the two system modules disagreed about the
    #: same fact in the same process. The decision was taken on that internal
    #: consistency rather than an SDK majority — ``""`` would have meant changing
    #: two system modules per SDK and leaving the contradiction in place.
    DEFAULT_PROJECT_NAME = "apcore"

    def _get_project_name(self) -> str:
        """Get project name from config, or the D-110 default."""
        if self._config is None:
            return self.DEFAULT_PROJECT_NAME
        return self._config.get("project.name", "") or self.DEFAULT_PROJECT_NAME


class ManifestSearchModule:
    """Return ACL-authorized summaries using the original caller's context."""

    description = "Search public modules visible to the requesting caller"
    annotations = ModuleAnnotations(readonly=True, idempotent=True, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "additionalProperties": False,
        "properties": {
            "query": {"type": "string"},
            "prefix": {"type": "string"},
            "tags": {"type": "array", "items": {"type": "string"}},
            "readonly": {"type": "boolean"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 100},
            "cursor": {"type": "string"},
            "include_approval_required": {"type": "boolean"},
        },
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "additionalProperties": False,
        "required": ["items", "next_cursor"],
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["module_id", "version", "description", "tags", "readonly", "access_hint"],
                    "properties": {
                        "module_id": {"type": "string"},
                        "version": {"type": "string"},
                        "description": {"type": "string"},
                        "tags": {"type": "array", "items": {"type": "string"}},
                        "readonly": {"type": "boolean"},
                        "access_hint": {"enum": ["allowed", "requires_approval", "conditional"]},
                    },
                },
            },
            "next_cursor": {"type": ["string", "null"]},
        },
    }

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        from apcore.executor import Executor

        if not isinstance(context.executor, Executor) or context.call_chain[-1:] != ["system.manifest.search"]:
            raise InvalidInputError("Manifest search requires a bound invocation context")
        if any(value is None for value in inputs.values()):
            raise InvalidInputError("Manifest search filters must not be null")
        callers = context.call_chain[:-1]
        original = replace(context, call_chain=callers, caller_id=callers[-2] if len(callers) > 1 else None)
        page = context.executor.list_callable(original, **inputs)
        return {
            "items": [{**asdict(item), "tags": list(item.tags)} for item in page.items],
            "next_cursor": page.next_cursor,
        }
