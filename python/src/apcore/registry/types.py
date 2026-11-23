"""Registry types: ModuleDescriptor, DiscoveredModule, DependencyInfo."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from apcore.module import ModuleAnnotations, ModuleExample

__all__ = [
    "ModuleDescriptor",
    "DiscoveredModule",
    "DependencyInfo",
]


@dataclass
class ModuleDescriptor:
    """Cross-language compatible module descriptor."""

    module_id: str
    name: str | None
    description: str
    documentation: str | None
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]
    version: str = "1.0.0"
    tags: list[str] = field(default_factory=list)
    annotations: ModuleAnnotations | None = None
    examples: list[ModuleExample] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)
    sunset_date: str | None = None
    """ISO 8601 date string (YYYY-MM-DD) after which this module is removed."""
    dependencies: list[DependencyInfo] = field(default_factory=list)
    """Declared module dependencies, parsed from ``metadata["dependencies"]``.

    Promoted to a typed field rather than left as raw JSON under ``metadata``.
    ``metadata`` is specified as "arbitrary extension metadata" — the ``x-``
    layer of the three-layer model — while dependencies are structural data the
    framework itself consumes for load and reload ordering. Leaving them in the
    extension bag pushed the ``{module_id, version?, optional?}`` parse onto
    every caller: ``sys_modules/control.py`` imported ``parse_dependencies``
    inside its reload function to do exactly that.

    PROTOCOL_SPEC §12.2 requires a ``dependencies`` entry in ``metadata`` to
    reach the registered module's descriptor. apcore-rust carried a typed
    ``Vec<DependencyInfo>``; this SDK surfaced only the unparsed list nested
    under ``metadata`` (sync finding A-D-004).
    """


@dataclass
class DiscoveredModule:
    """Intermediate representation of a discovered module file."""

    file_path: Path
    canonical_id: str
    meta_path: Path | None = None
    namespace: str | None = None


@dataclass
class DependencyInfo:
    """Parsed dependency information from module metadata."""

    module_id: str
    version: str | None = None
    optional: bool = False
