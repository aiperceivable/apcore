"""Annotation conflict resolution — merge YAML and code metadata."""

from __future__ import annotations

import dataclasses
from typing import Any

from apcore.module import ModuleAnnotations, ModuleExample

__all__ = ["governance_union", "merge_annotations", "merge_examples", "merge_metadata"]

_ANNOTATION_FIELDS = frozenset(ModuleAnnotations.__dataclass_fields__.keys())


def merge_annotations(
    yaml_annotations: dict[str, Any] | None,
    code_annotations: ModuleAnnotations | None,
) -> ModuleAnnotations:
    """Merge YAML and code annotations with priority: YAML > code > defaults."""
    defaults = ModuleAnnotations()
    values: dict[str, Any] = {f: getattr(defaults, f) for f in _ANNOTATION_FIELDS}

    if code_annotations is not None:
        for f in _ANNOTATION_FIELDS:
            values[f] = getattr(code_annotations, f)

    if yaml_annotations is not None:
        for key, val in yaml_annotations.items():
            if key in _ANNOTATION_FIELDS:
                values[key] = val

    return ModuleAnnotations(**values)


def governance_union(
    module_annotations: Any,
    declared_annotations: Any,
) -> ModuleAnnotations | None:
    """Union the two governance sources a module's requirement can come from.

    PROTOCOL_SPEC §7.4 D-96: the approval gate fires when *either* the live
    module instance or the registry's declared (descriptor) annotations ask for
    it. Only ``requires_approval`` and ``destructive`` are unioned; every other
    field describes behaviour rather than governance and is taken from the live
    instance, which is authoritative for it.

    Why a union and not :func:`merge_annotations`. That function implements
    YAML > code > defaults, which is right for a DESCRIPTOR — the operator's
    document is the more specific statement about what a module is. It is wrong
    for a gate, because it lets the weaker declaration win in both directions: a
    YAML ``requires_approval: false`` would cancel a module that asks to be
    gated, and a YAML ``requires_approval: true`` reached only the descriptor
    while the gate read the instance and let the call through ungated. Both are
    fail-OPEN, and on an approval gate the direction is the whole argument —
    requiring an approval that was not strictly needed costs a prompt, skipping
    one that was needed is a bypass.

    ``None`` only when neither source exists. Accepts a ``ModuleAnnotations`` or
    the dict shape hosts sometimes set, matching the rest of this module.
    """
    module_ann = _coerce(module_annotations)
    declared_ann = _coerce(declared_annotations)
    if module_ann is None and declared_ann is None:
        return None
    if declared_ann is None:
        return module_ann
    if module_ann is None:
        return declared_ann
    return dataclasses.replace(
        module_ann,
        requires_approval=module_ann.requires_approval or declared_ann.requires_approval,
        destructive=module_ann.destructive or declared_ann.destructive,
    )


def _coerce(annotations: Any) -> ModuleAnnotations | None:
    """Accept a ``ModuleAnnotations``, the dict wire shape, or ``None``."""
    if annotations is None:
        return None
    if isinstance(annotations, ModuleAnnotations):
        return annotations
    if isinstance(annotations, dict):
        return merge_annotations(annotations, None)
    return None


def merge_examples(
    yaml_examples: list[dict[str, Any]] | None,
    code_examples: list[ModuleExample] | None,
) -> list[ModuleExample]:
    """Merge YAML and code examples. YAML takes full priority when present."""
    if yaml_examples is not None:
        return [
            ModuleExample(
                title=d["title"],
                inputs=d.get("inputs", {}),
                output=d.get("output", {}),
                description=d.get("description"),
            )
            for d in yaml_examples
        ]
    if code_examples is not None:
        return code_examples
    return []


def merge_metadata(
    yaml_metadata: dict[str, Any] | None,
    code_metadata: dict[str, Any] | None,
) -> dict[str, Any]:
    """Merge YAML and code metadata dicts. YAML keys override code keys."""
    result = dict(code_metadata) if code_metadata is not None else {}
    if yaml_metadata is not None:
        result.update(yaml_metadata)
    return result
