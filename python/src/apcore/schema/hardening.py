"""Schema hardening utilities: content-addressable hash and standards-compliant validation.

Implements PROTOCOL_SPEC §4.15 (Issue #44):
- SHA-256 content hash for schema deduplication (canonical JSON form)
- JSON Schema–direct validation (anyOf/oneOf exhaustive, recursive $ref, constraints)
- Format-level warning (SHOULD-level, not hard error)
"""

from __future__ import annotations

import hashlib
import ipaddress
import json
import logging
import re
from datetime import date, datetime, time
from typing import TYPE_CHECKING, Any
from uuid import UUID

if TYPE_CHECKING:
    from collections.abc import Callable

from jsonschema import Draft202012Validator
from jsonschema.exceptions import ValidationError as JsonschemaError
from pydantic import BaseModel

from apcore.schema.types import SchemaValidationErrorDetail, SchemaValidationResult

__all__ = ["content_hash", "validate_schema_dict", "warn_format_violations"]

logger = logging.getLogger(__name__)

_EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")
_URI_SCHEME_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9+\-.]*://")

# Depth cap for the format walks. `generate_model` stores the caller's dict by
# reference, so a self-referencing schema reaches these functions directly and
# would otherwise recurse until the interpreter's own limit. A `format` deeper
# than this goes unreported, which is acceptable for a SHOULD-level annotation.
_MAX_WALK_DEPTH = 64


def _is_datetime(v: str) -> bool:
    try:
        datetime.fromisoformat(v.replace("Z", "+00:00"))
        return True
    except ValueError:
        return False


def _is_date(v: str) -> bool:
    try:
        date.fromisoformat(v)
        return True
    except ValueError:
        return False


def _is_time(v: str) -> bool:
    try:
        time.fromisoformat(v)
        return True
    except ValueError:
        return False


def _is_uuid(v: str) -> bool:
    try:
        UUID(v)
        return True
    except ValueError:
        return False


def _is_ipv4(v: str) -> bool:
    try:
        ipaddress.IPv4Address(v)
        return True
    except ValueError:
        return False


def _is_ipv6(v: str) -> bool:
    try:
        ipaddress.IPv6Address(v)
        return True
    except ValueError:
        return False


_FORMAT_CHECKERS: dict[str, Callable[[str], bool]] = {
    "date-time": _is_datetime,
    "date": _is_date,
    "time": _is_time,
    "email": lambda v: bool(_EMAIL_RE.match(v)),
    "uri": lambda v: bool(_URI_SCHEME_RE.match(v)),
    "uuid": _is_uuid,
    "ipv4": _is_ipv4,
    "ipv6": _is_ipv6,
}


def content_hash(schema: dict[str, Any]) -> str:
    """Return the SHA-256 hex digest of the canonical JSON form of schema.

    Canonical form: json.dumps with sort_keys=True and no extra whitespace.
    Schemas with identical content but different key ordering produce the same hash.
    """
    canonical = json.dumps(schema, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode()).hexdigest()


def validate_schema_dict(data: Any, schema: dict[str, Any]) -> SchemaValidationResult:
    """Validate *data* against a raw JSON Schema dict.

    Uses Draft202012Validator (jsonschema library) so all of the following are
    handled correctly and exhaustively:
    - anyOf: accepted if ≥1 branch matches
    - oneOf: accepted if exactly 1 branch matches; SCHEMA_UNION_AMBIGUOUS if >1
    - allOf: accepted only if every branch matches
    - Recursive $ref (self-referencing via $id or "#")
    - Numeric and string constraints (minimum, maximum, minLength, maxLength, pattern)
    - not keyword

    Format violations (date-time, email, uri, …) emit a logger.warning but do NOT
    fail validation (SHOULD-level enforcement per PROTOCOL_SPEC §4.15.4).
    """
    validator = Draft202012Validator(schema)
    raw_errors = list(validator.iter_errors(data))

    if raw_errors:
        error_code = _map_error_code(raw_errors)
        details = [_error_to_detail(e) for e in raw_errors]
        return SchemaValidationResult(valid=False, errors=details, error_code=error_code)

    _check_formats_and_warn(data, schema)
    return SchemaValidationResult(valid=True, errors=[])


def warn_format_violations(data: Any, model: Any) -> None:
    """Emit the SHOULD-level format warnings for *data* against *model*'s source schema.

    Module invocation validates through Pydantic, which has no format-annotation
    concept, so without this call the warnings would only ever fire on the
    `validate_schema_dict` path — which the executor does not reach. Models carry
    their source JSON Schema as `__apcore_source_schema__` (set by `SchemaLoader`);
    a model without one (a natively declared Pydantic model) is skipped.

    Whether the schema declares any `format` at all is computed once and cached on
    the model, so the common no-format schema costs a single attribute lookup.
    """
    source_schema = getattr(model, "__apcore_source_schema__", None)
    if not isinstance(source_schema, dict):
        return

    # Read the cache off this class only: `getattr` walks the MRO, so a subclass
    # would inherit a parent's False and skip its own formats forever.
    declares_format = model.__dict__.get("__apcore_declares_format__") if isinstance(model, type) else None
    if declares_format is None:
        declares_format = _declares_format(source_schema)
        try:
            model.__apcore_declares_format__ = declares_format
        except (AttributeError, TypeError):  # pragma: no cover - defensive
            pass
    if not declares_format:
        return

    if isinstance(data, BaseModel):
        data = data.model_dump(mode="json")
    _check_formats_and_warn(data, source_schema)


# Sub-schema keywords whose value is itself a schema (or a collection of them).
# The `format` scan follows only these, so a `format` key sitting in data — a
# `default`, an `examples` entry, or a property literally named "format" — does
# not count as a declaration.
_SCHEMA_VALUED_KEYWORDS = ("items", "additionalProperties", "not", "contains", "propertyNames")
_SCHEMA_MAP_KEYWORDS = ("properties", "patternProperties", "$defs", "definitions")
_SCHEMA_LIST_KEYWORDS = ("anyOf", "oneOf", "allOf", "prefixItems")


def _declares_format(node: Any, _depth: int = 0) -> bool:
    """Return True when *node* declares a `format` in a schema position."""
    if _depth > _MAX_WALK_DEPTH or not isinstance(node, dict):
        return False
    if "format" in node:
        return True
    for keyword in _SCHEMA_VALUED_KEYWORDS:
        if _declares_format(node.get(keyword), _depth + 1):
            return True
    for keyword in _SCHEMA_MAP_KEYWORDS:
        section = node.get(keyword)
        if isinstance(section, dict) and any(_declares_format(v, _depth + 1) for v in section.values()):
            return True
    for keyword in _SCHEMA_LIST_KEYWORDS:
        section = node.get(keyword)
        if isinstance(section, list) and any(_declares_format(v, _depth + 1) for v in section):
            return True
    return False


def _map_error_code(errors: list[JsonschemaError]) -> str:
    """Map jsonschema validation errors to apcore error codes."""
    for error in errors:
        if error.validator == "oneOf":
            if "is valid under each of" in error.message:
                return "SCHEMA_UNION_AMBIGUOUS"
            return "SCHEMA_UNION_NO_MATCH"
        if error.validator == "anyOf":
            return "SCHEMA_UNION_NO_MATCH"
    return "SCHEMA_VALIDATION_ERROR"


def _error_to_detail(error: JsonschemaError) -> SchemaValidationErrorDetail:
    path_parts = list(error.absolute_path)
    path = "".join("/" + str(part).replace("~", "~0").replace("/", "~1") for part in path_parts)
    raw_validator = error.validator
    constraint: str | None = str(raw_validator) if raw_validator is not None else None
    return SchemaValidationErrorDetail(
        path=path,
        message=error.message,
        constraint=constraint,
        expected=None,
        actual=error.instance,
    )


def _check_formats_and_warn(
    data: Any, schema: Any, _path: str = "", _seen: set[Any] | None = None, _depth: int = 0
) -> None:
    """Walk the data/schema tree and log warnings for format violations.

    Sync finding A-D-032: previously this function only iterated
    ``schema['properties']`` at the top level — nested invalid date-time /
    uuid / etc. formats at e.g. ``/user/created_at`` emitted no warning
    in Python while apcore-typescript and apcore-rust did warn. The
    walker now recurses into nested ``properties`` AND ``items`` so
    cross-language conformance fixtures see the same warning set.

    Each node is checked against its own ``format`` before the walk descends,
    which also covers combinator nodes: ``anyOf`` / ``oneOf`` branches (only the
    ones the data actually satisfies, so a sibling branch cannot report a format
    the value never carried) and every ``allOf`` member. An annotation reached
    through more than one branch is reported once.
    """
    if not isinstance(schema, dict) or _depth > _MAX_WALK_DEPTH:
        return
    if _seen is None:
        _seen = set()

    _warn_if_format_violated(data, schema, _path, _seen)

    # Object: recurse on each declared property's sub-schema.
    if isinstance(data, dict):
        properties = schema.get("properties", {})
        if isinstance(properties, dict):
            for prop_name, prop_schema in properties.items():
                if not isinstance(prop_schema, dict):
                    continue
                value = data.get(prop_name)
                if value is None:
                    continue
                child_path = f"{_path}/{prop_name}" if _path else f"/{prop_name}"
                _check_formats_and_warn(value, prop_schema, child_path, _seen, _depth + 1)

        additional = schema.get("additionalProperties")
        if isinstance(additional, dict):
            declared = properties if isinstance(properties, dict) else {}
            for key, value in data.items():
                if key in declared or value is None:
                    continue
                child_path = f"{_path}/{key}" if _path else f"/{key}"
                _check_formats_and_warn(value, additional, child_path, _seen, _depth + 1)

    # Array: walk each element against the schema's `items` declaration.
    if isinstance(data, list):
        items_schema = schema.get("items")
        if isinstance(items_schema, dict):
            for idx, value in enumerate(data):
                _check_formats_and_warn(value, items_schema, f"{_path}[{idx}]", _seen, _depth + 1)

    # Combinators: a union branch annotates the data only when the data satisfies it.
    for keyword in ("anyOf", "oneOf"):
        branches = schema.get(keyword)
        if isinstance(branches, list):
            for branch in branches:
                if isinstance(branch, dict) and Draft202012Validator(branch).is_valid(data):
                    _check_formats_and_warn(data, branch, _path, _seen, _depth + 1)

    members = schema.get("allOf")
    if isinstance(members, list):
        for member in members:
            if isinstance(member, dict):
                _check_formats_and_warn(data, member, _path, _seen, _depth + 1)


def _warn_if_format_violated(data: Any, schema: dict[str, Any], path: str, seen: set[Any]) -> None:
    """Log the SHOULD-level warning when *data* does not satisfy this node's ``format``.

    An unrecognised format is skipped: JSON Schema 2020-12 §7.2.1 puts ``format`` in
    the format-annotation vocabulary, where a format the implementation does not
    recognise is collected as an annotation, never treated as a failure.
    """
    fmt = schema.get("format")
    if not fmt or not isinstance(data, str):
        return
    checker = _FORMAT_CHECKERS.get(fmt)
    if checker is None or checker(data):
        return
    key = (path, fmt, data)
    if key in seen:
        return
    seen.add(key)
    logger.warning(
        "Format violation (non-fatal): field %r declared format=%r but value %r is not conformant",
        path,
        fmt,
        data,
    )
