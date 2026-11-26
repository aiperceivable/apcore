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
import math
import re
from copy import deepcopy
from datetime import date, datetime, time
from fractions import Fraction
from typing import TYPE_CHECKING, Any
from uuid import UUID

if TYPE_CHECKING:
    from collections.abc import Callable, Iterator
    from referencing._core import Resolver
    from referencing.jsonschema import Schema

from jsonschema import Draft202012Validator
from jsonschema.exceptions import ValidationError as JsonschemaError
from jsonschema.validators import extend
from pydantic import BaseModel
from referencing import Registry
from referencing.exceptions import Unresolvable
from referencing.jsonschema import DRAFT202012

from apcore.schema.types import SchemaValidationErrorDetail, SchemaValidationResult, _validation_failure_message

__all__ = ["content_hash", "validate_schema_dict", "warn_format_violations"]

logger = logging.getLogger(__name__)

_EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")
_URI_SCHEME_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9+\-.]*://")

# Depth cap for the format walks. `generate_model` stores the caller's dict by
# reference, so a self-referencing schema reaches these functions directly and
# would otherwise recurse until the interpreter's own limit. A `format` deeper
# than this goes unreported, which is acceptable for a SHOULD-level annotation.
_MAX_WALK_DEPTH = 64


def _validate_multiple_of(validator: Any, divisor: Any, instance: Any, schema: Any) -> Iterator[JsonschemaError]:
    """Compare each JSON number's decimal representation as an exact fraction."""
    if not validator.is_type(instance, "number"):
        return
    values = (instance, divisor)
    if (
        all(isinstance(value, int) or (isinstance(value, float) and math.isfinite(value)) for value in values)
        and divisor > 0
    ):
        numerator, denominator = (
            Fraction(str(value)) if isinstance(value, float) else Fraction(value) for value in values
        )
        if numerator % denominator != 0:
            yield JsonschemaError("Value is not a multiple of the declared divisor")
    else:
        yield from Draft202012Validator.VALIDATORS["multipleOf"](validator, divisor, instance, schema)


_Draft202012DecimalValidator = extend(Draft202012Validator, validators={"multipleOf": _validate_multiple_of})
_DRAFT202012_URI = Draft202012Validator.META_SCHEMA["$id"]


def _inherit_decimal_dialect(schema: dict[str, Any]) -> dict[str, Any]:
    """Let explicit Draft 2020-12 subschemas inherit the extended validator."""
    pending: list[Schema] = [schema]
    seen: set[int] = set()
    declarations: list[int] = []
    while pending:
        node = pending.pop()
        if not isinstance(node, dict) or id(node) in seen:
            continue
        seen.add(id(node))
        dialect = node.get("$schema")
        if isinstance(dialect, str):
            if dialect.rstrip("#") != _DRAFT202012_URI:
                continue
            declarations.append(id(node))
        pending.extend(DRAFT202012.subresources_of(node))

    if not declarations:
        return schema
    # jsonschema.evolve switches to the stock class at an explicit URI.
    # Drop redundant declarations from the validation copy; the original
    # schema remains available for references, export, and introspection.
    copies: dict[int, Any] = {}
    normalized = deepcopy(schema, copies)
    for node_id in declarations:
        del copies[node_id]["$schema"]
    return normalized


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
    validation_schema = _inherit_decimal_dialect(schema)
    validator = _Draft202012DecimalValidator(validation_schema, registry=Registry())
    raw_errors = list(validator.iter_errors(data))

    if raw_errors:
        error_code = _map_error_code(raw_errors)
        details = [_error_to_detail(e) for e in raw_errors]
        return SchemaValidationResult(valid=False, errors=details, error_code=error_code)

    _check_formats_and_warn(data, validation_schema)
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
    _check_formats_and_warn(data, _inherit_decimal_dialect(source_schema))


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
        message=_validation_failure_message(constraint or "type"),
        constraint=constraint,
        expected=None,
        actual=error.instance,
    )


def _check_formats_and_warn(
    data: Any,
    schema: Any,
    _path: str = "",
    _seen: set[Any] | None = None,
    _depth: int = 0,
    _resolver: Resolver[Any] | None = None,
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
    resolver = (
        _resolver if _resolver is not None else Registry().resolver_with_root(DRAFT202012.create_resource(schema))
    )

    def visit(value: Any, child: dict[str, Any], path: str) -> None:
        resource = DRAFT202012.create_resource(child)
        _check_formats_and_warn(value, child, path, _seen, _depth + 1, resolver.in_subresource(resource))

    _warn_if_format_violated(data, schema, _path, _seen)
    reference = schema.get("$ref")
    if isinstance(reference, str):
        try:
            resolved = resolver.lookup(reference)
        except Unresolvable:
            # Format diagnostics must never attempt additional retrieval.
            pass
        else:
            _check_formats_and_warn(data, resolved.contents, _path, _seen, _depth + 1, resolved.resolver)

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
                visit(value, prop_schema, child_path)

        additional = schema.get("additionalProperties")
        if isinstance(additional, dict):
            declared = properties if isinstance(properties, dict) else {}
            for key, value in data.items():
                if key in declared or value is None:
                    continue
                child_path = f"{_path}/{key}" if _path else f"/{key}"
                visit(value, additional, child_path)

    # Array: walk each element against the schema's `items` declaration.
    if isinstance(data, list):
        items_schema = schema.get("items")
        if isinstance(items_schema, dict):
            for idx, value in enumerate(data):
                visit(value, items_schema, f"{_path}[{idx}]")

    # Combinators: a union branch annotates the data only when the data satisfies it.
    for keyword in ("anyOf", "oneOf"):
        branches = schema.get(keyword)
        if isinstance(branches, list):
            for branch in branches:
                if isinstance(branch, dict):
                    branch_scope = resolver.in_subresource(DRAFT202012.create_resource(branch))
                    # jsonschema accepts the active resolver, but its typing
                    # omits the keyword. Keep the branch's document scope.
                    predicate = _Draft202012DecimalValidator(branch, _resolver=branch_scope)
                    try:
                        valid = predicate.is_valid(data)
                    except Unresolvable:
                        continue
                    if valid:
                        visit(data, branch, _path)

    members = schema.get("allOf")
    if isinstance(members, list):
        for member in members:
            if isinstance(member, dict):
                visit(data, member, _path)


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
        "Format violation (non-fatal): field %r does not conform to format=%r",
        path,
        fmt,
    )
