"""apcore registry and module discovery system.

Provides module registration, discovery, validation, and schema export.

Usage::

    from apcore.registry import Registry

    registry = Registry(extensions_dir="./extensions")
    count = registry.discover()
"""

from __future__ import annotations

from apcore.registry.conflicts import ConflictResult, detect_id_conflicts
from apcore.registry.dependencies import resolve_dependencies
from apcore.registry.entry_point import resolve_entry_point
from apcore.registry.metadata import load_id_map, load_metadata
from apcore.registry.multi_class import (
    MAX_MODULE_ID_LEN,
    class_name_to_segment,
    discover_multi_class,
    multi_class,
)
from apcore.registry.registry import (
    DEFAULT_MODULE_VERSION,
    EPHEMERAL_NAMESPACE_PREFIX,
    MAX_MODULE_ID_LENGTH,
    MODULE_ID_PATTERN,
    REGISTRY_EVENTS,
    RESERVED_WORDS,
    Registry,
)
from apcore.registry.scanner import scan_extensions, scan_multi_root
from apcore.registry.types import DependencyInfo, DiscoveredModule, ModuleDescriptor
from apcore.registry.validation import validate_module

__all__ = [
    "ConflictResult",
    "DEFAULT_MODULE_VERSION",
    "DependencyInfo",
    "DiscoveredModule",
    "EPHEMERAL_NAMESPACE_PREFIX",
    "MAX_MODULE_ID_LENGTH",
    "MODULE_ID_PATTERN",
    "ModuleDescriptor",
    "REGISTRY_EVENTS",
    "RESERVED_WORDS",
    "Registry",
    "MAX_MODULE_ID_LEN",
    "class_name_to_segment",
    "detect_id_conflicts",
    "discover_multi_class",
    "load_id_map",
    "load_metadata",
    "multi_class",
    "resolve_dependencies",
    "resolve_entry_point",
    "scan_extensions",
    "scan_multi_root",
    "validate_module",
]
