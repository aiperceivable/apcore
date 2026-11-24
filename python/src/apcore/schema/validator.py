"""SchemaValidator — validates runtime data against Pydantic models."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ValidationError as PydanticValidationError

from apcore.schema.types import SchemaValidationErrorDetail, SchemaValidationResult

__all__ = ["SchemaValidator", "coerce_value"]

_PYDANTIC_TO_CONSTRAINT: dict[str, str] = {
    "missing": "required",
    "string_type": "type",
    "int_type": "type",
    "float_type": "type",
    "bool_type": "type",
    "string_too_short": "minLength",
    "string_too_long": "maxLength",
    "string_pattern_mismatch": "pattern",
    "greater_than_equal": "minimum",
    "less_than_equal": "maximum",
    "greater_than": "exclusiveMinimum",
    "less_than": "exclusiveMaximum",
    "literal_error": "enum",
    "value_error": "value",
    "extra_forbidden": "additionalProperties",
    "too_short": "minLength",
    "too_long": "maxLength",
}

_EXPECTED_KEYS = (
    "expected",
    "ge",
    "le",
    "gt",
    "lt",
    "min_length",
    "max_length",
    "pattern",
)


# The only two strings the coercing knob turns into a boolean, and the only two
# JSON itself uses to spell one (TYPE_MAPPING §11 "What the knob coerces, when it
# exists", normative as of spec v1.12.0). Case-sensitive: JSON's boolean literals
# are lowercase, so `"True"` is not one of them.
_STR_TO_BOOL: dict[str, bool] = {"true": True, "false": False}


def coerce_value(value: Any, schema: Any) -> Any:
    """Rewrite strings toward the type *schema* declares, per TYPE_MAPPING §11.

    This is the coercing knob's pre-pass, and it runs **only** when
    ``SchemaValidator(coerce_types=True)`` — never on the module-invocation
    boundary, which calls ``model_validate(strict=True)`` directly and has no
    ``SchemaValidator`` in its path at all (TYPE_MAPPING §17.3).

    Why here and not in the generated model. The `boolean` guard that rejects
    `"true"` is `loader._ONLY_BOOL`, a `BeforeValidator` baked into the Pydantic
    model by `SchemaLoader.generate_model()` — which is built without knowing the
    knob's value, so the guard cannot consult it. Threading a `coerce` flag
    through `generate_model` would have worked, at the cost of a second cache key
    and a second compiled model per schema. Doing it as a validator pre-pass
    instead puts all three SDKs' coercion at the same layer — this is the twin of
    `apcore-typescript::coerceValue` and `apcore-rust::coerce_value` — and leaves
    `_ONLY_BOOL` to keep rejecting `"true"`, `1` and `0` on the strict path, which
    R5 makes a MUST (apcore#95).

    String → `integer` and string → `number` are deliberately *not* handled here:
    Pydantic's own lax mode already implements exactly the rows §11 specifies
    (`"42"` → 42, `"1.5"` → 1.5, `"3.14"` for `integer` rejected), and
    re-implementing them would give one behaviour two sources of truth.

    Coercion is from a string only, and only toward a type the schema declares:
    a number is never coerced to a boolean, a boolean never to a number, and
    nothing is coerced toward `string`. The input is never mutated — containers
    are rebuilt.
    """
    if not isinstance(schema, dict):
        return value

    if isinstance(value, dict):
        properties = schema.get("properties")
        if isinstance(properties, dict):
            return {k: coerce_value(v, properties[k]) if k in properties else v for k, v in value.items()}
        return value

    if isinstance(value, list):
        items = schema.get("items")
        if isinstance(items, dict):
            return [coerce_value(item, items) for item in value]
        return value

    # `bool` is a subclass of `int`, not of `str`, so this cannot catch one.
    if isinstance(value, str) and "boolean" in _declared_types(schema):
        return _STR_TO_BOOL.get(value, value)

    return value


def _declared_types(schema: dict[str, Any]) -> tuple[str, ...]:
    """The `type` keyword as a tuple, whether written as a string or an array."""
    declared = schema.get("type")
    if isinstance(declared, str):
        return (declared,)
    if isinstance(declared, list):
        return tuple(t for t in declared if isinstance(t, str))
    return ()


class SchemaValidator:
    """Validates runtime data against Pydantic models and produces apcore-standard error output.

    ``coerce_types`` is a **library-level** knob, for a caller validating its own
    untyped input (a CLI parsing argv, a form handler). It does **not** reach the
    module-invocation boundary: ``BuiltinInputValidation`` /
    ``BuiltinOutputValidation`` call ``model_validate(strict=True)`` directly and
    never coerce, under any host configuration (TYPE_MAPPING §17.3). There is no
    ``schema.validation.coerce_types`` setting and never was one an SDK read — a
    module's contract has to mean the same thing regardless of who loaded it.

    It defaults to ``False``, matching the boundary and the other two SDKs.
    Coercion is opt-in: a validator that silently rewrites its input is the wrong
    default for the common case of checking data you already believe is well-formed.

    **What it coerces when enabled** is fixed by TYPE_MAPPING §11 (normative as of
    spec v1.12.0): offering the knob stays a **MAY**, but an SDK that offers one
    **MUST** coerce exactly

    ==============  ============  ==============================================
    from            to            accepted
    ==============  ============  ==============================================
    ``string``      ``integer``   entire content parses as an integer — ``"42"``,
                                  ``"-7"``. ``"3.14"`` MUST NOT be accepted.
    ``string``      ``number``    entire content parses as a number — ``"1.5"``.
    ``string``      ``boolean``   exactly ``"true"`` and ``"false"``,
                                  **case-sensitive**.
    ==============  ============  ==============================================

    and **MUST NOT** coerce anything else. The boolean row is served by
    :func:`coerce_value` as a pre-pass; the numeric rows by Pydantic's own lax
    mode, which already implements them. ``"yes"``, ``"on"``, ``"1"``, ``"0"``,
    ``"True"`` and the number ``1`` are all rejected for a ``boolean`` in both
    modes (apcore#95).
    """

    def __init__(self, coerce_types: bool = False) -> None:
        self._coerce_types = coerce_types

    def validate(self, data: Any, model: type[BaseModel]) -> SchemaValidationResult:
        """Validate data against a Pydantic model, returning a result object.

        For models generated from an empty JSON Schema (``{}``), any input
        value is accepted per Draft 2020-12 (the always-true schema).

        A-D-08: when the model's source JSON Schema declares a top-level
        ``oneOf`` / ``anyOf``, validation is routed through the
        jsonschema-backed exhaustive check (``hardening.validate_schema_dict``)
        so union semantics — including ``SCHEMA_UNION_NO_MATCH`` (zero matches)
        and ``SCHEMA_UNION_AMBIGUOUS`` (multiple ``oneOf`` matches) — are
        surfaced, matching the TS and Rust validators. Pydantic's
        ``model_validate`` accepts a union on first match and cannot detect
        ``oneOf`` ambiguity, so it is not used for union schemas.
        """
        data = self._coerce(data, model)

        union_result = self._validate_top_level_union(data, model)
        if union_result is not None:
            return union_result

        # Empty schema (always-true) — accept any value, including non-dict.
        # A field-less model is not necessarily an empty schema: a root-level
        # combinator (`{"allOf": [...]}`), applicator or property-count keyword
        # declares no `properties` either, and `generate_model` expresses those as
        # model-level validators. Taking the shortcut there accepted every input.
        if not model.model_fields and not getattr(model, "__apcore_has_assertions__", False):
            extra_cfg = model.model_config.get("extra", "ignore")
            if extra_cfg != "forbid":
                return SchemaValidationResult(valid=True, errors=[])

        try:
            model.model_validate(data, strict=not self._coerce_types)
            return SchemaValidationResult(valid=True, errors=[])
        except PydanticValidationError as e:
            # A-D-036 / A-D-034: populate the canonical error_code on the public
            # validation path so consumers (and cross-SDK parity) see
            # SCHEMA_VALIDATION_ERROR (spec §8.2) rather than None.
            return SchemaValidationResult(
                valid=False,
                errors=self._pydantic_error_to_details(e),
                error_code="SCHEMA_VALIDATION_ERROR",
            )

    def _coerce(self, data: Any, model: type[BaseModel]) -> Any:
        """Run the §11 coercion pre-pass, or return *data* untouched.

        A no-op unless ``coerce_types=True``, so the strict path — and therefore
        the module-invocation boundary's answer for the same schema and input —
        is bit-for-bit what it was before the knob grew a boolean row.

        ``__apcore_source_schema__`` is attached by ``generate_model``; a model
        built some other way (a hand-written ``BaseModel``) carries no source
        schema, and coercion is skipped rather than guessed at.
        """
        if not self._coerce_types:
            return data
        source_schema = getattr(model, "__apcore_source_schema__", None)
        if not isinstance(source_schema, dict):
            return data
        return coerce_value(data, source_schema)

    @staticmethod
    def _validate_top_level_union(data: Any, model: type[BaseModel]) -> SchemaValidationResult | None:
        """Run the jsonschema union check if the model's source schema is a top-level union.

        Returns the union validation result when the source JSON Schema declares a
        top-level ``oneOf`` or ``anyOf``; otherwise returns None so the caller falls
        through to the standard Pydantic path. Importing ``hardening`` lazily avoids
        a module-level import cycle and keeps the jsonschema dependency off the hot
        path for non-union schemas.
        """
        source_schema = getattr(model, "__apcore_source_schema__", None)
        if not isinstance(source_schema, dict):
            return None
        if "oneOf" not in source_schema and "anyOf" not in source_schema:
            return None

        from apcore.schema.hardening import validate_schema_dict

        return validate_schema_dict(data, source_schema)

    def validate_input(self, data: dict[str, Any], model: type[BaseModel]) -> dict[str, Any]:
        """Validate input data and return the validated dict. Raises SchemaValidationError on failure."""
        return self._validate_and_dump(data, model)

    def validate_output(self, data: dict[str, Any], model: type[BaseModel]) -> dict[str, Any]:
        """Validate output data and return the validated dict. Raises SchemaValidationError on failure."""
        return self._validate_and_dump(data, model)

    def _validate_and_dump(self, data: dict[str, Any], model: type[BaseModel]) -> dict[str, Any]:
        """Validate data and return model_dump(). Raises SchemaValidationError on failure."""
        try:
            instance = model.model_validate(self._coerce(data, model), strict=not self._coerce_types)
            return instance.model_dump()
        except PydanticValidationError as e:
            result = SchemaValidationResult(valid=False, errors=self._pydantic_error_to_details(e))
            raise result.to_error() from e

    def _pydantic_error_to_details(self, error: PydanticValidationError) -> list[SchemaValidationErrorDetail]:
        """Convert Pydantic v2 ValidationError to apcore error details."""
        details: list[SchemaValidationErrorDetail] = []
        for err in error.errors():
            loc = err.get("loc", ())
            pydantic_type = err.get("type", "")
            if pydantic_type in {"missing", "extra_forbidden"}:
                loc = loc[:-1]
            path = "".join("/" + str(segment).replace("~", "~0").replace("/", "~1") for segment in loc)
            constraint = _PYDANTIC_TO_CONSTRAINT.get(pydantic_type, pydantic_type)

            message = err.get("msg", "")

            ctx = err.get("ctx", {})
            expected: Any = None
            for key in _EXPECTED_KEYS:
                val = ctx.get(key)
                if val is not None:
                    expected = val
                    break

            actual = ctx.get("actual")
            if actual is None:
                actual = err.get("input")

            details.append(
                SchemaValidationErrorDetail(
                    path=path,
                    message=message,
                    constraint=constraint,
                    expected=expected,
                    actual=actual,
                )
            )
        return details
