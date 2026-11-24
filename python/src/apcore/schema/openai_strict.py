"""OpenAI structured-outputs strict-mode compatibility detection.

This module is deliberately **separate** from :mod:`apcore.schema.strict`.
``to_strict_schema()`` is a general-purpose registry/export transform; baking an
OpenAI-specific dialect check into it would leak one vendor's constraints into
every other consumer. Detection lives here and is invoked only on the
``auto_schema: strict`` binding path (protocol-spec §5.12.5).

Detection **never rewrites** the schema. In particular an author-written
``oneOf`` is reported, not silently downgraded to ``anyOf``: the two differ in
exclusivity, and apcore's own validator relies on the distinction (it counts
matching branches and raises ``SCHEMA_UNION_AMBIGUOUS``). Rewriting at export
time would trade an explicit export-time rejection for a silent runtime
mismatch.

---------------------------------------------------------------------------
Source of truth for the keyword set below:
    OpenAI — "Structured model outputs", sections "Supported schemas /
    Supported properties" and "Some type-specific keywords are not yet
    supported".
    https://developers.openai.com/api/docs/guides/structured-outputs
    Verified 2026-08-03.

What that page states, in substance:
    - Supported types: String, Number, Boolean, Integer, Object, Array, Enum,
      anyOf.
    - Supported ``string`` properties: ``pattern``, ``format`` (limited to the
      nine values in ``SUPPORTED_FORMATS`` below).
    - Supported ``number`` properties: ``multipleOf``, ``maximum``,
      ``exclusiveMaximum``, ``minimum``, ``exclusiveMinimum``.
    - Supported ``array`` properties: ``minItems``, ``maxItems``.
    - Not supported — composition: ``allOf``, ``not``, ``dependentRequired``,
      ``dependentSchemas``, ``if``, ``then``, ``else``.
    - ``anyOf`` is supported but MUST NOT appear at the root (the root must be
      an object).
    - ``$ref`` / ``$defs`` (including recursive ``#`` self-references) are
      supported.
    - ``additionalProperties: false`` and "every property listed in
      ``required``" are requirements, not incompatibilities —
      ``to_strict_schema()`` already produces both, so they are transformed
      rather than reported here.

Anything appearing in neither the supported nor the explicitly-unsupported list
(``oneOf``, ``minLength``/``maxLength``, ``patternProperties``,
``uniqueItems``, ...) is treated as unsupported: OpenAI rejects unknown or
unlisted keywords when ``strict: true``.

NOTE: this set tracks a moving target. When OpenAI expands structured-output
support, update the three SDK copies (``schema/openai_strict.py``,
``schema/openai-strict.ts``, ``schema/openai_strict.rs``) together and re-run
the ``openai_strict_compat.json`` conformance fixture, which pins the
cross-SDK answer.
---------------------------------------------------------------------------
"""

from __future__ import annotations

from typing import Any

from apcore.errors import BindingStrictSchemaIncompatibleError

__all__ = [
    "SUPPORTED_FORMATS",
    "UNSUPPORTED_KEYWORDS",
    "assert_openai_strict_compatible",
    "detect_openai_strict_incompatibilities",
]

#: JSON Schema keywords OpenAI structured outputs does not accept under
#: ``strict: true``. Kept sorted for reviewability; emission order is
#: normalised by sorting the result, so cross-SDK output does not depend on it.
UNSUPPORTED_KEYWORDS: tuple[str, ...] = (
    # Composition — explicitly listed as unsupported by OpenAI.
    "allOf",
    "dependentRequired",
    "dependentSchemas",
    "else",
    "if",
    "not",
    "then",
    # `oneOf` is absent from the supported list (only `anyOf` is named).
    "oneOf",
    # String constraints beyond `pattern` / `format`.
    "maxLength",
    "minLength",
    # Object keywords beyond `properties` / `required` / `additionalProperties`.
    "maxProperties",
    "minProperties",
    "patternProperties",
    "propertyNames",
    "unevaluatedProperties",
    # Array keywords beyond `items` / `minItems` / `maxItems`.
    "additionalItems",
    "contains",
    "maxContains",
    "minContains",
    "prefixItems",
    "unevaluatedItems",
    "uniqueItems",
)

#: The nine ``format`` values OpenAI accepts for strings.
SUPPORTED_FORMATS: frozenset[str] = frozenset(
    {
        "date-time",
        "time",
        "date",
        "duration",
        "email",
        "hostname",
        "ipv4",
        "ipv6",
        "uuid",
    }
)


def detect_openai_strict_incompatibilities(schema: dict[str, Any]) -> list[str]:
    """Return every feature OpenAI structured outputs rejects under ``strict: true``.

    Entries are JSON-path-ish descriptors, sorted and de-duplicated so that
    Python, TypeScript and Rust produce identical lists:

    - ``$.tags.uniqueItems``   — unsupported keyword at that location
    - ``$.created.format=uri`` — unsupported ``format`` value
    - ``$.anyOf``              — ``anyOf`` at the root (the root must be an object)

    An empty list means the schema is compatible.
    """
    found: list[str] = []

    def walk(node: Any, path: str, is_root: bool) -> None:
        if not isinstance(node, dict):
            return

        for keyword in UNSUPPORTED_KEYWORDS:
            if keyword in node:
                found.append(f"{path}.{keyword}")

        # `anyOf` is supported, but not as the root schema.
        if is_root and "anyOf" in node:
            found.append(f"{path}.anyOf")

        fmt = node.get("format")
        if isinstance(fmt, str) and fmt not in SUPPORTED_FORMATS:
            found.append(f"{path}.format={fmt}")

        properties = node.get("properties")
        if isinstance(properties, dict):
            for key in sorted(properties):
                walk(properties[key], f"{path}.{key}", False)

        items = node.get("items")
        if isinstance(items, dict):
            walk(items, f"{path}[]", False)

        additional_properties = node.get("additionalProperties")
        if isinstance(additional_properties, dict):
            walk(additional_properties, f"{path}.additionalProperties", False)

        # Recurse into union branches only. `allOf`/`not`/`if`/`then`/`else`
        # are already reported by keyword; descending into them would add noise
        # beneath a finding the caller must fix by removing the branch anyway.
        for combinator in ("anyOf", "oneOf"):
            branches = node.get(combinator)
            if isinstance(branches, list):
                for index, branch in enumerate(branches):
                    walk(branch, f"{path}.{combinator}[{index}]", False)

        for defs_key in ("$defs", "definitions"):
            defs = node.get(defs_key)
            if isinstance(defs, dict):
                for key in sorted(defs):
                    walk(defs[key], f"{path}.{defs_key}.{key}", False)

    walk(schema, "$", True)
    return sorted(set(found))


def assert_openai_strict_compatible(
    schema: dict[str, Any],
    *,
    module_id: str,
    side: str | None = None,
    file_path: str | None = None,
    line: int | None = None,
) -> None:
    """Raise :class:`BindingStrictSchemaIncompatibleError` for an incompatible schema.

    Invoked on the ``auto_schema: strict`` binding path only
    (protocol-spec §5.12.5). No-op for compatible schemas.

    Args:
        schema: The JSON Schema to check.
        module_id: Binding ``module_id``, used in the error message.
        side: Prefix applied to every reported feature, e.g. ``input`` / ``output``.
        file_path: Binding file path, for the ``{file_path}:{line}:`` prefix.
        line: Binding file line, for the ``{file_path}:{line}:`` prefix.
    """
    features = detect_openai_strict_incompatibilities(schema)
    if not features:
        return

    listed = features if side is None else [f"{side}:{feature}" for feature in features]
    raise BindingStrictSchemaIncompatibleError(
        module_id=module_id,
        features_listed=listed,
        file_path=file_path,
        line=line,
    )
