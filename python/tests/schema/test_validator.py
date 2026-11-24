"""Tests for SchemaValidator."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal

import pytest
from pydantic import BaseModel, Field

from apcore.errors import SchemaValidationError
from apcore.schema.validator import SchemaValidator

if TYPE_CHECKING:
    from apcore.schema.loader import SchemaLoader


# --- Inline test models ---


class SimpleModel(BaseModel):
    name: str
    age: int
    active: bool = True


class ConstrainedModel(BaseModel):
    name: str = Field(min_length=2, max_length=50)
    count: int = Field(ge=0, le=100)
    code: str = Field(pattern=r"^[A-Z]{3}$")


class NestedModel(BaseModel):
    class Address(BaseModel):
        city: str
        zip_code: str

    address: Address


class ArrayModel(BaseModel):
    class Item(BaseModel):
        quantity: int

    items: list[Item]


class EnumModel(BaseModel):
    status: Literal["active", "inactive", "pending"]


class StrictModel(BaseModel):
    model_config = {"extra": "forbid"}
    name: str
    value: int


class OptionalFieldsModel(BaseModel):
    name: str = "default"
    count: int = 0


@pytest.fixture
def validator() -> SchemaValidator:
    return SchemaValidator(coerce_types=True)


@pytest.fixture
def strict_validator() -> SchemaValidator:
    return SchemaValidator(coerce_types=False)


# === validate() ===


class TestValidate:
    def test_valid_data(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "Alice", "age": 30}, SimpleModel)
        assert result.valid is True
        assert result.errors == []

    def test_missing_required_field(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "Alice"}, SimpleModel)
        assert result.valid is False
        assert any(e.constraint == "required" for e in result.errors)

    def test_failure_populates_error_code(self, validator: SchemaValidator) -> None:
        """A-D-036 / A-D-034: a plain validation failure result must carry the
        canonical error_code SCHEMA_VALIDATION_ERROR (spec §8.2), not None."""
        result = validator.validate({"name": "Alice"}, SimpleModel)
        assert result.valid is False
        assert result.error_code == "SCHEMA_VALIDATION_ERROR"

    def test_success_has_no_error_code(self, validator: SchemaValidator) -> None:
        """A successful validation must not set an error_code."""
        result = validator.validate({"name": "Alice", "age": 30}, SimpleModel)
        assert result.valid is True
        assert result.error_code is None

    def test_wrong_type_strict(self, strict_validator: SchemaValidator) -> None:
        result = strict_validator.validate({"name": "Alice", "age": "not_a_number"}, SimpleModel)
        assert result.valid is False
        assert any(e.constraint == "type" for e in result.errors)

    def test_pattern_mismatch(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "AB", "count": 5, "code": "abc"}, ConstrainedModel)
        assert result.valid is False
        assert any(e.path == "/code" and e.constraint == "pattern" for e in result.errors)

    def test_below_minimum(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "AB", "count": -1, "code": "ABC"}, ConstrainedModel)
        assert result.valid is False
        assert any(e.path == "/count" and e.constraint == "minimum" for e in result.errors)

    def test_above_maximum(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "AB", "count": 101, "code": "ABC"}, ConstrainedModel)
        assert result.valid is False
        assert any(e.path == "/count" and e.constraint == "maximum" for e in result.errors)

    def test_string_too_short(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "A", "count": 5, "code": "ABC"}, ConstrainedModel)
        assert result.valid is False
        assert any(e.path == "/name" and e.constraint == "minLength" for e in result.errors)

    def test_string_too_long(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "A" * 51, "count": 5, "code": "ABC"}, ConstrainedModel)
        assert result.valid is False
        assert any(e.path == "/name" and e.constraint == "maxLength" for e in result.errors)

    def test_enum_invalid(self, validator: SchemaValidator) -> None:
        result = validator.validate({"status": "unknown"}, EnumModel)
        assert result.valid is False
        assert any(e.path == "/status" and e.constraint == "enum" for e in result.errors)

    def test_multiple_errors(self, validator: SchemaValidator) -> None:
        result = validator.validate({}, SimpleModel)
        assert result.valid is False
        assert len(result.errors) >= 2

    def test_nested_error_path(self, validator: SchemaValidator) -> None:
        result = validator.validate({"address": {"zip_code": "12345"}}, NestedModel)
        assert result.valid is False
        assert any(e.path == "/address" and e.constraint == "required" for e in result.errors)

    def test_array_item_error_path(self, strict_validator: SchemaValidator) -> None:
        result = strict_validator.validate({"items": [{"quantity": "bad"}]}, ArrayModel)
        assert result.valid is False
        assert any("/items/0/quantity" in e.path for e in result.errors)

    def test_extra_properties_strict(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "Alice", "value": 1, "extra": "nope"}, StrictModel)
        assert result.valid is False
        assert any(e.constraint == "additionalProperties" for e in result.errors)

    def test_optional_fields(self, validator: SchemaValidator) -> None:
        result = validator.validate({}, OptionalFieldsModel)
        assert result.valid is True


# === union schemas (A-D-08): top-level oneOf/anyOf ===


class TestUnionValidation:
    """A-D-08: SchemaValidator must surface union codes for top-level oneOf/anyOf.

    Peer parity: TS validator.ts _validateOneOf and Rust jsonschema emit
    SCHEMA_UNION_NO_MATCH / SCHEMA_UNION_AMBIGUOUS. The model carries its
    source schema so validate() can route union schemas through the
    jsonschema-backed exhaustive check in hardening.validate_schema_dict.
    """

    def test_oneof_ambiguous(self, validator: SchemaValidator, schema_loader: "SchemaLoader") -> None:
        # Two branches both match an integer -> ambiguous.
        schema = {
            "oneOf": [
                {"type": "integer", "minimum": 0},
                {"type": "integer", "maximum": 100},
            ]
        }
        model = schema_loader.generate_model(schema, "AmbiguousUnion")
        result = validator.validate(5, model)
        assert result.valid is False
        assert result.error_code == "SCHEMA_UNION_AMBIGUOUS"

    def test_oneof_no_match(self, validator: SchemaValidator, schema_loader: "SchemaLoader") -> None:
        schema = {
            "oneOf": [
                {"type": "integer"},
                {"type": "boolean"},
            ]
        }
        model = schema_loader.generate_model(schema, "NoMatchUnion")
        result = validator.validate("a string", model)
        assert result.valid is False
        assert result.error_code == "SCHEMA_UNION_NO_MATCH"

    def test_oneof_single_match_valid(self, validator: SchemaValidator, schema_loader: "SchemaLoader") -> None:
        schema = {
            "oneOf": [
                {"type": "integer"},
                {"type": "string"},
            ]
        }
        model = schema_loader.generate_model(schema, "SingleMatchUnion")
        result = validator.validate(42, model)
        assert result.valid is True
        assert result.error_code is None

    def test_anyof_no_match(self, validator: SchemaValidator, schema_loader: "SchemaLoader") -> None:
        schema = {
            "anyOf": [
                {"type": "integer"},
                {"type": "boolean"},
            ]
        }
        model = schema_loader.generate_model(schema, "AnyOfNoMatch")
        result = validator.validate("nope", model)
        assert result.valid is False
        assert result.error_code == "SCHEMA_UNION_NO_MATCH"

    def test_normal_object_schema_unaffected(self, validator: SchemaValidator) -> None:
        """A normal object schema must still validate via the Pydantic path."""
        result = validator.validate({"name": "Alice", "age": 30}, SimpleModel)
        assert result.valid is True
        assert result.error_code is None
        bad = validator.validate({"name": "Alice"}, SimpleModel)
        assert bad.valid is False
        assert bad.error_code == "SCHEMA_VALIDATION_ERROR"


# === validate_input() ===


class TestValidateInput:
    def test_valid_returns_dict(self, validator: SchemaValidator) -> None:
        result = validator.validate_input({"name": "Alice", "age": 30}, SimpleModel)
        assert isinstance(result, dict)
        assert result["name"] == "Alice"
        assert result["age"] == 30
        assert result["active"] is True

    def test_invalid_raises(self, validator: SchemaValidator) -> None:
        with pytest.raises(SchemaValidationError) as exc_info:
            validator.validate_input({"name": "Alice"}, SimpleModel)
        assert exc_info.value.code == "SCHEMA_VALIDATION_ERROR"
        assert len(exc_info.value.details["errors"]) > 0

    def test_coercion_enabled(self, validator: SchemaValidator) -> None:
        result = validator.validate_input({"name": "Alice", "age": "30"}, SimpleModel)
        assert result["age"] == 30
        assert isinstance(result["age"], int)

    def test_strict_rejects_coercion(self, strict_validator: SchemaValidator) -> None:
        with pytest.raises(SchemaValidationError):
            strict_validator.validate_input({"name": "Alice", "age": "30"}, SimpleModel)


# === validate_output() ===


class TestValidateOutput:
    def test_valid_returns_dict(self, validator: SchemaValidator) -> None:
        result = validator.validate_output({"name": "Alice", "age": 30}, SimpleModel)
        assert isinstance(result, dict)
        assert result["name"] == "Alice"

    def test_invalid_raises(self, validator: SchemaValidator) -> None:
        with pytest.raises(SchemaValidationError):
            validator.validate_output({"name": "Alice"}, SimpleModel)


# === error conversion ===


class TestErrorConversion:
    def test_missing_constraint(self, validator: SchemaValidator) -> None:
        result = validator.validate({}, SimpleModel)
        name_err = next(e for e in result.errors if e.path == "")
        assert name_err.constraint == "required"

    def test_type_constraint(self, strict_validator: SchemaValidator) -> None:
        result = strict_validator.validate({"name": 123, "age": 30}, SimpleModel)
        name_err = next(e for e in result.errors if e.path == "/name")
        assert name_err.constraint == "type"

    def test_min_length_constraint(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "A", "count": 5, "code": "ABC"}, ConstrainedModel)
        name_err = next(e for e in result.errors if e.path == "/name")
        assert name_err.constraint == "minLength"

    def test_pattern_constraint(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "AB", "count": 5, "code": "abc"}, ConstrainedModel)
        code_err = next(e for e in result.errors if e.path == "/code")
        assert code_err.constraint == "pattern"

    def test_minimum_constraint(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "AB", "count": -1, "code": "ABC"}, ConstrainedModel)
        count_err = next(e for e in result.errors if e.path == "/count")
        assert count_err.constraint == "minimum"

    def test_enum_constraint(self, validator: SchemaValidator) -> None:
        result = validator.validate({"status": "unknown"}, EnumModel)
        status_err = next(e for e in result.errors if e.path == "/status")
        assert status_err.constraint == "enum"

    def test_additional_properties_constraint(self, validator: SchemaValidator) -> None:
        result = validator.validate({"name": "Alice", "value": 1, "extra": "x"}, StrictModel)
        assert any(e.constraint == "additionalProperties" for e in result.errors)

    def test_loc_to_path(self, validator: SchemaValidator) -> None:
        result = validator.validate({}, SimpleModel)
        assert any(e.path == "" and e.constraint == "required" for e in result.errors)

    def test_nested_loc_to_path(self, validator: SchemaValidator) -> None:
        result = validator.validate({"address": {"zip_code": "12345"}}, NestedModel)
        assert any(e.path == "/address" and e.constraint == "required" for e in result.errors)


# === TYPE_MAPPING §11 — what the library-level coercion knob coerces ===


class TestCoercionKnobTable:
    """The knob's accepted set, normative as of spec v1.12.0 (apcore#95).

    Offering ``coerce_types`` stays a MAY; an SDK that offers one MUST coerce
    exactly the §11 table and MUST NOT coerce anything else. Every case here
    asserts *both* modes: the failure this pins is the strict path and the knob
    answering the same way for a spelling neither should accept.
    """

    BOOL_SCHEMA = {"type": "object", "properties": {"flag": {"type": "boolean"}}, "required": ["flag"]}
    INT_SCHEMA = {"type": "object", "properties": {"count": {"type": "integer"}}, "required": ["count"]}
    NUM_SCHEMA = {"type": "object", "properties": {"n": {"type": "number"}}, "required": ["n"]}
    STR_SCHEMA = {"type": "object", "properties": {"s": {"type": "string"}}, "required": ["s"]}

    @pytest.mark.parametrize(
        ("schema_name", "data", "coerced"),
        [
            ("BOOL_SCHEMA", {"flag": "true"}, {"flag": True}),
            ("BOOL_SCHEMA", {"flag": "false"}, {"flag": False}),
            ("INT_SCHEMA", {"count": "42"}, {"count": 42}),
            ("INT_SCHEMA", {"count": "-7"}, {"count": -7}),
            ("NUM_SCHEMA", {"n": "1.5"}, {"n": 1.5}),
            ("NUM_SCHEMA", {"n": "-0.5"}, {"n": -0.5}),
        ],
        ids=["str_true", "str_false", "str_int", "str_negative_int", "str_float", "str_negative_float"],
    )
    def test_accepted_set_coerces_to_the_pinned_value(
        self,
        schema_loader: "SchemaLoader",
        schema_name: str,
        data: dict,
        coerced: dict,
    ) -> None:
        """The three §11 rows, asserted on the value produced — not just validity.

        Validity alone cannot tell ``"false"`` -> False from ``"false"`` -> True,
        and an implementation coercing every non-empty string to True passes a
        validity-only check on both boolean rows.
        """
        model = schema_loader.generate_model(getattr(self, schema_name), f"Accept_{schema_name}_{data}")
        assert SchemaValidator(coerce_types=True).validate(data, model).valid is True
        produced = SchemaValidator(coerce_types=True).validate_input(data, model)
        for key, expected in coerced.items():
            assert produced[key] == expected
            assert type(produced[key]) is type(expected)

    @pytest.mark.parametrize(
        ("schema_name", "data"),
        [
            ("BOOL_SCHEMA", {"flag": "yes"}),
            ("BOOL_SCHEMA", {"flag": "no"}),
            ("BOOL_SCHEMA", {"flag": "on"}),
            ("BOOL_SCHEMA", {"flag": "off"}),
            ("BOOL_SCHEMA", {"flag": "y"}),
            ("BOOL_SCHEMA", {"flag": "t"}),
            ("BOOL_SCHEMA", {"flag": "1"}),
            ("BOOL_SCHEMA", {"flag": "0"}),
            ("BOOL_SCHEMA", {"flag": "True"}),
            ("BOOL_SCHEMA", {"flag": "TRUE"}),
            ("BOOL_SCHEMA", {"flag": "False"}),
            ("BOOL_SCHEMA", {"flag": " true"}),
            ("BOOL_SCHEMA", {"flag": ""}),
            ("BOOL_SCHEMA", {"flag": 1}),
            ("BOOL_SCHEMA", {"flag": 0}),
            ("INT_SCHEMA", {"count": "3.14"}),
            ("INT_SCHEMA", {"count": "abc"}),
            ("INT_SCHEMA", {"count": "true"}),
            ("NUM_SCHEMA", {"n": True}),
            ("STR_SCHEMA", {"s": 42}),
            ("STR_SCHEMA", {"s": True}),
        ],
    )
    def test_everything_else_is_rejected_in_both_modes(
        self,
        schema_loader: "SchemaLoader",
        schema_name: str,
        data: dict,
    ) -> None:
        """MUST NOT coerce anything outside the table.

        The eleven boolean spellings here are the twelve-spelling shell/INI
        dialect apcore-rust wrote and apcore-typescript ported; §11 caps the set
        at JSON's own two literals. ``"0"`` is the sharpest: R5 makes the *number*
        ``0`` a MUST-reject for boolean, so accepting the string put two paths of
        one SDK on opposite sides of a single value.
        """
        model = schema_loader.generate_model(getattr(self, schema_name), f"Reject_{schema_name}_{data}")
        assert SchemaValidator(coerce_types=True).validate(data, model).valid is False
        assert SchemaValidator(coerce_types=False).validate(data, model).valid is False

    @pytest.mark.parametrize("spelling", ["true", "false"])
    def test_strict_path_still_rejects_json_boolean_spellings(
        self, schema_loader: "SchemaLoader", spelling: str
    ) -> None:
        """`_require_bool` keeps its job: R5 makes this a MUST at the boundary.

        The knob gaining a boolean row must not reach the strict path — the two
        answering differently for one schema and input is the failure TYPE_MAPPING
        §11 exists to prevent.
        """
        model = schema_loader.generate_model(self.BOOL_SCHEMA, f"Strict_{spelling}")
        assert SchemaValidator(coerce_types=False).validate({"flag": spelling}, model).valid is False

    def test_coercion_reaches_nested_objects_and_arrays(self, schema_loader: "SchemaLoader") -> None:
        """The pre-pass walks `properties` and `items`, matching TS/Rust `coerceValue`."""
        schema = {
            "type": "object",
            "properties": {
                "inner": {"type": "object", "properties": {"flag": {"type": "boolean"}}},
                "flags": {"type": "array", "items": {"type": "boolean"}},
            },
        }
        model = schema_loader.generate_model(schema, "NestedCoerce")
        data = {"inner": {"flag": "true"}, "flags": ["true", "false"]}
        assert SchemaValidator(coerce_types=True).validate(data, model).valid is True
        produced = SchemaValidator(coerce_types=True).validate_input(data, model)
        assert produced["inner"]["flag"] is True
        assert produced["flags"] == [True, False]

    def test_pre_pass_does_not_mutate_caller_input(self, schema_loader: "SchemaLoader") -> None:
        """A validator that rewrites the dict it was handed is a surprise the
        caller never opted into; only the copy passed to Pydantic is coerced."""
        model = schema_loader.generate_model(self.BOOL_SCHEMA, "NoMutate")
        data = {"flag": "true"}
        SchemaValidator(coerce_types=True).validate(data, model)
        assert data == {"flag": "true"}

    def test_default_is_no_coercion(self, schema_loader: "SchemaLoader") -> None:
        """`coerce_types` defaults to False — the boundary's behaviour, and the
        other two SDKs' default (TYPE_MAPPING §11).

        Asserted on behaviour rather than on the private flag: a default that is
        `False` and read by nothing would satisfy the flag check.
        """
        model = schema_loader.generate_model(self.BOOL_SCHEMA, "DefaultKnob")
        assert SchemaValidator().validate({"flag": "true"}, model).valid is False
        int_model = schema_loader.generate_model(self.INT_SCHEMA, "DefaultKnobInt")
        assert SchemaValidator().validate({"count": "42"}, int_model).valid is False
