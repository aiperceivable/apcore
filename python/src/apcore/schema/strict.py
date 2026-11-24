"""Strict mode conversion for JSON Schemas (Algorithm A23)."""

from __future__ import annotations

import copy
from typing import Any


def to_strict_schema(schema: dict[str, Any]) -> dict[str, Any]:
    """Convert a JSON Schema to OpenAI/Anthropic Strict Mode format.

    Deep-copies the input, strips extensions and defaults, then enforces
    strict mode rules (additionalProperties: false, all properties required,
    optional fields become nullable).
    """
    result = copy.deepcopy(schema)
    _strip_extensions(result)
    _convert_to_strict(result)
    return result


def _apply_llm_descriptions(node: Any) -> None:
    """Replace description with x-llm-description where present.

    Mutates the node in place. Called by exporters before to_strict_schema().
    """
    if not isinstance(node, dict):
        return

    if "x-llm-description" in node and "description" in node:
        node["description"] = node["x-llm-description"]

    # Recurse into nested structures
    if "properties" in node and isinstance(node["properties"], dict):
        for prop in node["properties"].values():
            _apply_llm_descriptions(prop)
    if "items" in node and isinstance(node["items"], dict):
        _apply_llm_descriptions(node["items"])
    if "prefixItems" in node and isinstance(node["prefixItems"], list):
        for item in node["prefixItems"]:
            _apply_llm_descriptions(item)
    for keyword in ("oneOf", "anyOf", "allOf"):
        if keyword in node and isinstance(node[keyword], list):
            for sub in node[keyword]:
                _apply_llm_descriptions(sub)
    for defs_key in ("definitions", "$defs"):
        if defs_key in node and isinstance(node[defs_key], dict):
            for defn in node[defs_key].values():
                _apply_llm_descriptions(defn)


def _strip_extensions(node: Any, *, strip_defaults: bool = True, _names: bool = False) -> None:
    """Remove x-* schema keywords, preserving named properties and data values.

    Schema maps such as ``properties`` and ``$defs`` contain names, not keyword
    positions. Their child schemas are still processed recursively. Mutates
    in place and optionally removes schema defaults for OpenAI strict mode.

    Args:
        node: JSON Schema node to process.
        strip_defaults: If True (default), also remove ``default`` keys.
            OpenAI strict mode requires this; Anthropic export does not.
    """
    if not isinstance(node, dict):
        return

    keys_to_remove = [
        k
        for k in node
        if not _names and ((isinstance(k, str) and k.startswith("x-")) or (strip_defaults and k == "default"))
    ]
    for k in keys_to_remove:
        del node[k]

    for key, value in node.items():
        if not _names and key in {"const", "enum", "default", "examples"}:
            continue
        if isinstance(value, dict):
            _strip_extensions(
                value,
                strip_defaults=strip_defaults,
                _names=not _names
                and key in {"properties", "patternProperties", "$defs", "definitions", "dependentSchemas"},
            )
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, dict):
                    _strip_extensions(item, strip_defaults=strip_defaults)


def _declares_object_type(type_value: Any) -> bool:
    """True when a ``type`` keyword declares "object", as a string or inside a list."""
    if isinstance(type_value, str):
        return type_value == "object"
    if isinstance(type_value, list):
        return "object" in type_value
    return False


def _convert_to_strict(node: Any) -> None:
    """Enforce strict mode rules on an object schema. Mutates in place."""
    if not isinstance(node, dict):
        return

    # `type` may be a string ("object"), a list (["object", "null"] — what the
    # nullable wrapper below produces for an optional nested object), or absent
    # entirely: `properties` alone already implies an object schema, and
    # requiring a `type` keyword let ``{"properties": {...}}`` through
    # unhardened, which OpenAI structured outputs then rejects (A23 / §4.16).
    if "properties" in node and ("type" not in node or _declares_object_type(node["type"])):
        node["additionalProperties"] = False
        existing_required = set(node.get("required", []))
        all_names = list(node["properties"].keys())
        optional_names = [n for n in all_names if n not in existing_required]

        for name in optional_names:
            prop = node["properties"][name]
            constrained = any(key in prop for key in ("enum", "const", "$ref", "allOf", "anyOf", "oneOf", "not", "if"))
            if "type" not in prop or constrained:
                # Constraints can reject null independently of the type keyword.
                # Preserve the complete original schema in the non-null branch.
                node["properties"][name] = {"anyOf": [prop, {"type": "null"}]}
            elif isinstance(prop["type"], str):
                if prop["type"] != "null":
                    prop["type"] = [prop["type"], "null"]
            elif isinstance(prop["type"], list) and "null" not in prop["type"]:
                prop["type"].append("null")

        node["required"] = sorted(all_names)

    # Recurse into nested structures
    if "properties" in node and isinstance(node["properties"], dict):
        for prop in node["properties"].values():
            _convert_to_strict(prop)
    if "items" in node and isinstance(node["items"], dict):
        _convert_to_strict(node["items"])
    # ``prefixItems`` — the Draft 2020-12 tuple form. Without this, an object
    # sitting at a tuple position was never hardened.
    if "prefixItems" in node and isinstance(node["prefixItems"], list):
        for item in node["prefixItems"]:
            _convert_to_strict(item)
    for keyword in ("oneOf", "anyOf", "allOf"):
        if keyword in node and isinstance(node[keyword], list):
            for sub in node[keyword]:
                _convert_to_strict(sub)
    for defs_key in ("definitions", "$defs"):
        if defs_key in node and isinstance(node[defs_key], dict):
            for defn in node[defs_key].values():
                _convert_to_strict(defn)
