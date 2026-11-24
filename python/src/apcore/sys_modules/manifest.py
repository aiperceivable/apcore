"""system.manifest.module and system.manifest.full sys modules."""

from __future__ import annotations

from dataclasses import asdict
from typing import Any

from apcore.config import Config
from apcore.errors import InvalidInputError, ModuleNotFoundError
from apcore.module import ModuleAnnotations
from apcore.schema.annotations import governance_union
from apcore.registry.registry import Registry

__all__ = ["ManifestFullModule", "ManifestModule", "ManifestModuleModule"]


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
        "properties": {
            "module_id": {"type": "string", "description": "Module identifier"},
            "description": {"type": "string", "description": "Module description"},
            "documentation": {"description": "Module documentation (Markdown)"},
            "source_path": {
                "type": "string",
                "description": "Computed source file path",
            },
            "input_schema": {
                "type": "object",
                "description": "Module input JSON Schema",
            },
            "output_schema": {
                "type": "object",
                "description": "Module output JSON Schema",
            },
            "annotations": {"type": "object", "description": "Module annotations"},
            "tags": {"type": "array", "description": "Module tags"},
            "dependencies": {"type": "array", "description": "Module dependencies"},
            "metadata": {"type": "object", "description": "Additional metadata"},
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
        dependencies = self._get_dependencies(module_id)
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
            "dependencies": dependencies,
            "metadata": metadata,
        }

    def _get_dependencies(self, module_id: str) -> list[dict[str, Any]]:
        """Retrieve dependencies from registry metadata."""
        meta = self._registry.get_module_metadata(module_id)
        return meta.get("dependencies", [])

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
        "properties": {
            "project_name": {
                "type": "string",
                "description": "Project name from config",
            },
            "module_count": {
                "type": "integer",
                "description": "Number of modules returned",
            },
            "modules": {"type": "array", "description": "Module manifest entries"},
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
        dependencies = self._get_dependencies(module_id)
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
            "dependencies": dependencies,
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

    def _get_dependencies(self, module_id: str) -> list[dict[str, Any]]:
        """Retrieve dependencies from registry metadata."""
        meta = self._registry.get_module_metadata(module_id)
        return meta.get("dependencies", [])
