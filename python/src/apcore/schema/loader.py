"""SchemaLoader — primary entry point for the schema system."""

from __future__ import annotations

import json
import logging
from collections.abc import Callable
from pathlib import Path
from typing import Annotated, Any, Literal, Union, cast

import yaml
from jsonschema import Draft202012Validator
from jsonschema.exceptions import SchemaError as JsonSchemaError
from pydantic import BaseModel, ConfigDict, Field, create_model, model_validator
from pydantic.functional_validators import AfterValidator, BeforeValidator
from pydantic_core import PydanticCustomError, PydanticUndefined
from referencing import Registry, Resource
from referencing.jsonschema import DRAFT202012

from apcore.config import Config
from apcore.errors import SchemaNotFoundError, SchemaParseError
from apcore.schema.hardening import content_hash
from apcore.schema.ref_resolver import RefResolver
from apcore.schema.types import ResolvedSchema, SchemaDefinition, SchemaStrategy

__all__ = ["SchemaLoader"]

logger = logging.getLogger(__name__)

_TYPE_MAP: dict[str, type] = {
    "string": str,
    "integer": int,
    "number": float,
    "boolean": bool,
    "null": type(None),
}

# Assertions that are independent of `type` and must hold alongside it
# (JSON Schema 2020-12 §10.2). When one of these sits next to a `type` keyword,
# both have to be enforced — neither may be discarded in favour of the other.
_COMBINATOR_KEYWORDS = ("const", "enum", "oneOf", "anyOf", "allOf", "not")

# Option keywords that constrain a single instance type (§6). When `type` is an
# array each branch keeps only its own, so a numeric bound cannot land on a string.
_STRING_CONSTRAINTS = {"minLength": "min_length", "maxLength": "max_length", "pattern": "pattern"}
_NUMERIC_CONSTRAINTS = {
    "minimum": "ge",
    "maximum": "le",
    "exclusiveMinimum": "gt",
    "exclusiveMaximum": "lt",
    "multipleOf": "multiple_of",
}
_ARRAY_CONSTRAINTS = {"minItems": "min_length", "maxItems": "max_length"}

# Applicator keywords with no Pydantic equivalent (JSON Schema 2020-12 §10.3,
# §11). Dropping them silently let apcore-python accept contracts apcore-rust
# rejected, so each one is delegated to the jsonschema library instead — the same
# engine `_make_schema_assertion` already uses for combinator siblings.
_APPLICATOR_KEYWORDS = (
    "prefixItems",
    "patternProperties",
    "propertyNames",
    "dependentRequired",
    "dependentSchemas",
    "if",
    "unevaluatedItems",
    "unevaluatedProperties",
)

# Keywords an applicator changes the meaning of, and therefore has to take with
# it. `items` applies only past the prefix once `prefixItems` is there (§10.3.1.2),
# `additionalProperties` skips every pattern-matched key (§10.3.2.3), and
# `then`/`else` assert nothing without `if` (§10.2.2.2/§10.2.2.3).
_APPLICATOR_COMPANIONS: dict[str, tuple[str, ...]] = {
    "prefixItems": ("items",),
    "patternProperties": ("properties", "additionalProperties"),
    "if": ("then", "else"),
}

# §11 defines these two against the annotations every *other* keyword in the same
# schema produced, so a keyword slice would leave them nothing to subtract and
# they would reject properties their siblings had already evaluated. The whole
# schema travels with them instead.
_WHOLE_SCHEMA_APPLICATORS = ("unevaluatedItems", "unevaluatedProperties")

# Every remaining §6 assertion and §10.3 applicator — the ones a *typed*
# position expresses natively (a Field constraint, `list[T]`, a generated model)
# and a `type`-less one cannot. `_APPLICATOR_KEYWORDS` and the combinators are
# deliberately absent: those are already delegated by their own paths.
_BARE_ASSERTION_KEYWORDS = (
    # §6.2 numeric
    "multipleOf",
    "maximum",
    "exclusiveMaximum",
    "minimum",
    "exclusiveMinimum",
    # §6.3 string
    "maxLength",
    "minLength",
    "pattern",
    # §6.4 / §10.3.1 array
    "maxItems",
    "minItems",
    "uniqueItems",
    "maxContains",
    "minContains",
    "items",
    "contains",
    # §6.5 / §10.3.2 object
    "maxProperties",
    "minProperties",
    "required",
    "properties",
    "additionalProperties",
)


def _bare_assertion_schema(schema: dict[str, Any], carried: dict[str, Any] | None) -> dict[str, Any] | None:
    """Build the §6/§10.3 sub-schema a `type`-less position has no annotation for.

    At a position that declares a `type`, every keyword below is expressed
    natively — a Pydantic `Field` constraint, a `list[T]`, a generated model. At
    a position that declares none, `_base_annotation` widens to `Any` and there
    is nothing left to carry them, so they were asserted by nothing at all
    (TYPE_MAPPING §17.1 R1, "no silent drop"): `{"required": ["b"]}` is a
    complete schema and the usual shape of an `if` / `then` / `dependentSchemas`
    branch, and apcore-typescript and apcore-rust both enforce it.

    Delegating to jsonschema rather than mapping to a `Field` argument is what
    makes §17.1 R2 ("inertness") hold for free: `{"minimum": 3}` rejects `1` and
    passes `"x"`, `[1]`, `true` and `null`, because that is what the keyword
    means. Attaching `ge=3` to an `Any`-annotated field instead made the bound
    apply to *every* instance type, and pydantic's fallback for a constraint it
    cannot place on an `any` core schema raised a bare `TypeError` that
    `BuiltinInputValidation` does not catch.

    *carried* is whatever `_applicator_assertion_schema` already took for the
    same position; those keywords must not be re-asserted in isolation. The
    `patternProperties` case is the one that actually breaks: it owns
    `additionalProperties`, and a second, lone `additionalProperties: false`
    check would reject exactly the pattern-matched keys §10.3.2.3 exempts.

    Returns None when the position is typed, is a bare `$ref`, or carries
    nothing left to assert.
    """
    if "type" in schema or "$ref" in schema:
        return None
    already = set(carried) if carried else set()
    sub = {key: schema[key] for key in _BARE_ASSERTION_KEYWORDS if key in schema and key not in already}
    return sub or None


def _applicator_assertion_schema(schema: dict[str, Any]) -> dict[str, Any] | None:
    """Build the sub-schema to delegate to jsonschema, or None when none applies.

    Only the applicator keywords travel, not the whole schema: `type` and the §6
    keywords are already enforced by the generated annotation, and re-asserting
    them on the raw instance would check them twice over a differently-shaped
    value (TYPE_MAPPING §17.3, "Keyword slicing"). The `unevaluated*` pair is the
    documented exception — see `_WHOLE_SCHEMA_APPLICATORS`.
    """
    present = [keyword for keyword in _APPLICATOR_KEYWORDS if keyword in schema]
    if not present:
        return None
    if any(keyword in schema for keyword in _WHOLE_SCHEMA_APPLICATORS):
        return dict(schema)

    sub_schema: dict[str, Any] = {}
    for keyword in present:
        sub_schema[keyword] = schema[keyword]
        for companion in _APPLICATOR_COMPANIONS.get(keyword, ()):
            if companion in schema:
                sub_schema[companion] = schema[companion]

    if "properties" in sub_schema:
        # Only the property *names* matter here — they tell `additionalProperties`
        # which keys are already claimed. Keeping the real sub-schemas would
        # re-assert them on raw, un-coerced data. `True` is the always-true schema.
        sub_schema["properties"] = dict.fromkeys(sub_schema["properties"], True)
    return sub_schema


def _make_property_count_check(schema: dict[str, Any]) -> Callable[[Any], Any] | None:
    """Build a check for `minProperties`/`maxProperties`, or None when neither applies.

    These two (§6.5.1/§6.5.2) deliberately have no table of their own alongside
    the three above: those map a keyword to a Pydantic `Field` argument, and no
    Field argument counts an object's members. A generated model has a fixed set
    of fields, while these keywords assert how many keys the *instance* carried —
    undeclared ones included, since every `additionalProperties` form but `false`
    keeps them.

    Used as a `before` validator so the count is taken from the raw mapping: once
    conversion has run, an omitted optional field is indistinguishable from one
    explicitly set to null, and undeclared keys have moved into
    `__pydantic_extra__`. Both would make the count disagree with jsonschema.

    A non-mapping value passes straight through — an object keyword must not
    touch the `null` branch of a `type` array (§6.5 applies to objects only).
    """
    minimum = schema.get("minProperties")
    maximum = schema.get("maxProperties")
    if minimum is None and maximum is None:
        return None

    def check_property_count(value: Any) -> Any:
        if not isinstance(value, dict):
            return value
        count = len(value)
        if minimum is not None and count < minimum:
            raise ValueError(f"Object should have at least {minimum} properties, got {count}")
        if maximum is not None and count > maximum:
            raise ValueError(f"Object should have at most {maximum} properties, got {count}")
        return value

    return check_property_count


def _as_model_before_validator(check: Callable[[Any], Any]) -> Any:
    """Wrap *check* as a model-level `before` validator for `create_model`.

    `before` so the check sees the raw mapping: after conversion an omitted
    optional field is indistinguishable from an explicit null and undeclared keys
    have moved into `__pydantic_extra__`, which no keyword counting or naming
    properties can tolerate.
    """

    def validate_before(cls: type[Any], data: Any) -> Any:
        return check(data)

    return model_validator(mode="before")(validate_before)


def _with_property_count(annotation: Any, schema: dict[str, Any]) -> Any:
    """Attach the object-count check to an annotation that is not a generated model."""
    check = _make_property_count_check(schema)
    return Annotated[annotation, BeforeValidator(check)] if check is not None else annotation


def _canonical_json(value: Any) -> str:
    """Canonical JSON form of *value*, used as an equality key.

    Mirrors the ``sortedKeysStringify`` apcore-typescript uses for the same
    purpose: keys sorted, no insignificant whitespace.
    """
    return json.dumps(value, sort_keys=True, separators=(",", ":"), default=str)


def _check_unique(v: list[Any]) -> list[Any]:
    """Assert `uniqueItems` (§6.4.3) by comparing canonical JSON forms.

    `set(v)` raised a bare ``TypeError`` on an array of objects — ``dict`` is not
    hashable — and Pydantic only converts ``ValueError``/``AssertionError`` into a
    validation error, so the ``TypeError`` escaped the module call and the caller
    saw it instead of ``SCHEMA_VALIDATION_ERROR``. apcore-typescript and
    apcore-rust both reject the duplicate rather than raising.

    Values are rebuilt with `_to_jsonable` first: this runs as an *after*
    validator, so a nested object has already become a Pydantic model by now.
    """
    keys = [_canonical_json(_to_jsonable(item)) for item in v]
    if len(keys) != len(set(keys)):
        raise ValueError("Items must be unique")
    return v


def _reject_bool(value: Any) -> Any:
    """Reject a bool where a number is expected.

    `bool` subclasses `int` in Python, so Pydantic's lax mode happily reads `True`
    as `1`. JSON Schema treats the two as distinct instance types — `{"type":
    "integer"}` rejects `true` — and so do apcore-typescript and apcore-rust.
    Runs before conversion so the numeric branch of a union declines the value and
    a sibling `boolean` branch can still claim it.
    """
    if isinstance(value, bool):
        raise ValueError("Input should be a number, not a boolean")
    return value


def _require_bool(value: Any) -> Any:
    """Reject a non-bool where a boolean is expected.

    The mirror of `_reject_bool`: lax mode reads `1` and `"true"` as `True`, while
    `{"type": "boolean"}` accepts neither.
    """
    if not isinstance(value, bool):
        raise ValueError("Input should be a valid boolean")
    return value


def _json_integer(value: Any) -> Any:
    """Normalise a JSON `integer` instance before Pydantic's strict `int` check.

    JSON Schema 2020-12 §6.1.1 defines `integer` as *any* number with a zero
    fractional part, so `4.0` is an integer — the `jsonschema` reference
    implementation, apcore-typescript and apcore-rust all accept it. Pydantic
    sees a Python `float` and, in the strict mode the module-invocation boundary
    runs in (TYPE_MAPPING §17.3), rejects it. Narrowing the zero-fraction case
    here keeps "no coercion" meaning "no *type* coercion" rather than
    "reject a JSON integer written with a decimal point".

    Note this is not coercion: `4.5` still fails, and so does `"4"` — neither is
    an integer instance. Bool declines for the reason `_reject_bool` documents.
    """
    if isinstance(value, bool):
        raise ValueError("Input should be a number, not a boolean")
    if isinstance(value, float) and value.is_integer():
        return int(value)
    return value


_NO_BOOL = BeforeValidator(_reject_bool)
_ONLY_BOOL = BeforeValidator(_require_bool)
_JSON_INTEGER = BeforeValidator(_json_integer)


def _scalar_annotation(type_name: str) -> Any:
    """Annotation for a scalar JSON Schema type, with the bool/number guards applied."""
    base = _TYPE_MAP.get(type_name, Any)
    if type_name == "integer":
        return Annotated[base, _JSON_INTEGER]
    if type_name == "number":
        return Annotated[base, _NO_BOOL]
    if type_name == "boolean":
        return Annotated[base, _ONLY_BOOL]
    return base


def _to_jsonable(value: Any) -> Any:
    """Convert a validated Pydantic value back to plain JSON data for a jsonschema check."""
    if isinstance(value, BaseModel):
        return value.model_dump(mode="json")
    if isinstance(value, dict):
        return {k: _to_jsonable(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_to_jsonable(v) for v in value]
    return value


def _make_schema_assertion(
    sub_schema: dict[str, Any],
    *,
    plain_data: bool = False,
    registry: Registry | None = None,
) -> Callable[[Any], Any]:
    """Build an AfterValidator asserting *sub_schema* on an already-typed value.

    Pydantic has no way to intersect a combinator keyword onto a type-derived
    annotation, so the sibling assertion is delegated to the jsonschema library —
    the same engine `hardening.validate_schema_dict` uses, which keeps the two
    validation paths in agreement.

    `plain_data` skips the `_to_jsonable` rebuild when the annotation can only
    produce values jsonschema already reads (see `_is_json_native`); the rebuild
    is a full deep copy run on every single validation.

    The sub-schema is checked against the metaschema here, at model-build time, so
    a malformed contract fails when the module is registered rather than raising a
    bare `AttributeError` out of `iter_errors` on the first real call.
    """
    try:
        Draft202012Validator.check_schema(sub_schema)
    except JsonSchemaError as exc:
        raise SchemaParseError(message=f"Invalid schema for keyword(s) {sorted(sub_schema)}: {exc.message}") from exc

    validator = (
        Draft202012Validator(sub_schema) if registry is None else Draft202012Validator(sub_schema, registry=registry)
    )

    def assert_sub_schema(value: Any) -> Any:
        data = value if plain_data else _to_jsonable(value)
        for error in validator.iter_errors(data):
            # `validator` is the failing keyword name (`enum`, `const`, `not`, …).
            # Passing it through as the error type keeps the reported `constraint`
            # specific instead of collapsing every combinator to "value".
            raise PydanticCustomError(
                cast(Any, str(error.validator or "value")),
                error.message,
                {"expected": error.validator_value},
            )
        return value

    return assert_sub_schema


# Base URI the root document is registered under so a lazy `$ref` can be pointed
# at it. `urn:` keeps it distinguishable from anything a schema author could
# write, and nothing ever dereferences it over the network.
_ROOT_BASE_URI = "urn:apcore:schema-root"


def _ref_assertion_schema(ref: str, root: dict[str, Any]) -> tuple[dict[str, Any], Registry] | None:
    """Build the (schema, registry) pair that asserts *ref* against *root*.

    `RefResolver` leaves a self-reference in place instead of inlining it
    (PROTOCOL_SPEC §4.15.2), so a recursive schema still carries `$ref` nodes when
    it reaches model generation. Pydantic has no annotation for "whatever this
    pointer names, recursively", so the sub-tree is delegated to jsonschema — the
    same engine combinator siblings and applicators already use — with the whole
    root document registered under a synthetic base URI. Every nested `#…`
    reference inside the target then resolves against the real document rather
    than against the fragment handed to the validator.

    Returns None for a reference this SDK cannot anchor (an external URI the
    resolver could not inline), leaving the caller to widen to `Any` rather than
    failing the whole conversion.
    """
    if ref in ("#", "#/") or ref == root.get("$id"):
        pointer = "#"
    elif ref.startswith("#/"):
        pointer = ref
    else:
        return None

    resource = Resource.from_contents(root, default_specification=DRAFT202012)
    registry = Registry().with_resource(_ROOT_BASE_URI, resource)
    return {"$ref": f"{_ROOT_BASE_URI}{pointer}"}, registry


def _branch_constraints(prop_schema: dict[str, Any], type_name: str) -> dict[str, Any]:
    """Collect the option keywords that apply to *type_name* only."""
    if type_name == "string":
        mapping = _STRING_CONSTRAINTS
    elif type_name in ("integer", "number"):
        mapping = _NUMERIC_CONSTRAINTS
    elif type_name == "array":
        mapping = _ARRAY_CONSTRAINTS
    else:
        return {}
    return {arg: prop_schema[keyword] for keyword, arg in mapping.items() if keyword in prop_schema}


def _field_constraints(prop_schema: dict[str, Any], *, is_array: bool) -> dict[str, Any]:
    """Collect the field-wide constraint arguments for a Pydantic Field.

    Gated on a single declared scalar/array `type`, because a Field constraint is
    unconditional: it fires on whatever value reaches the field. That is only the
    right reading when the annotation has already narrowed the value to the type
    the keyword describes.

    A `type` array carries its option keywords per union branch instead (see
    `_union_from_types`); repeating them field-wide would apply a numeric bound to
    the string branch and a length bound to the numeric one.

    A *missing* `type` is the case that was wrong. `minLength` was collected and
    attached to an `Any`-annotated field, so it rejected `[1]` and `{"a": 1}` —
    instances §17.1 R2 requires it to pass, and which apcore-typescript and
    apcore-rust do pass. Worse, pydantic cannot place `ge` or `pattern` on an
    `any` core schema and its `apply_known_metadata` fallback raised a bare
    `TypeError`, which `BuiltinInputValidation` does not catch (it catches only
    `pydantic.ValidationError`), so the module call died uncoded. Those keywords
    now travel through `_bare_assertion_schema` instead, where jsonschema makes
    them inert by construction.
    """
    schema_type = prop_schema.get("type")
    if not isinstance(schema_type, str):
        return {}
    if is_array or schema_type == "array":
        return _branch_constraints(prop_schema, "array")
    if schema_type == "string":
        return _branch_constraints(prop_schema, "string")
    if schema_type in ("integer", "number"):
        return _branch_constraints(prop_schema, "integer")
    return {}


def _is_json_native(prop_schema: dict[str, Any]) -> bool:
    """True when the validated value is already something jsonschema can read.

    jsonschema consumes plain scalars, lists and dicts directly; only a value that
    can hold a Pydantic model needs rebuilding first. Getting this right matters
    for more than tidiness — the rebuild is a full deep copy run on every single
    validation, so a large array pays for it per call.
    """
    schema_type = prop_schema.get("type")
    if isinstance(schema_type, str):
        members = [schema_type]
    elif isinstance(schema_type, list):
        members = schema_type
    else:
        return False

    for member in members:
        if member in _TYPE_MAP:
            continue
        if member == "array":
            items = prop_schema.get("items")
            if isinstance(items, dict) and items.get("type") in _TYPE_MAP:
                continue
        return False
    return bool(members)


class SchemaLoader:
    """Primary entry point for loading, resolving, and generating schemas."""

    def __init__(self, config: Config, schemas_dir: str | Path | None = None) -> None:
        self._config = config
        if schemas_dir is not None:
            self._schemas_dir = Path(schemas_dir).resolve()
        else:
            self._schemas_dir = Path(config.get("schema.root", Config.get_default("schema.root"))).resolve()
        max_depth = config.get("schema.max_ref_depth", 32)
        self._resolver = RefResolver(self._schemas_dir, max_depth=max_depth)
        self._schema_cache: dict[str, SchemaDefinition] = {}
        # Two-level content-addressable cache (PROTOCOL_SPEC §4.15 Issue #44):
        # Level 1: path index — maps module_id to SHA-256 hash of resolved schema
        # Level 2: content cache — maps hash to compiled model pair; deduplicates identical schemas
        self._path_index: dict[str, str] = {}
        self._content_cache: dict[str, tuple[ResolvedSchema, ResolvedSchema]] = {}

    def load(self, module_id: str) -> SchemaDefinition:
        """Load a schema definition from a YAML file."""
        if module_id in self._schema_cache:
            return self._schema_cache[module_id]

        file_path = self._schemas_dir / (module_id.replace(".", "/") + ".schema.yaml")
        if not file_path.exists():
            raise SchemaNotFoundError(schema_id=module_id)

        try:
            data = yaml.safe_load(file_path.read_text())
        except yaml.YAMLError as e:
            raise SchemaParseError(message=f"Invalid YAML in schema for '{module_id}': {e}") from e

        if data is None or not isinstance(data, dict):
            raise SchemaParseError(message=f"Schema file for '{module_id}' is empty or not a mapping")

        for field_name in ("input_schema", "output_schema", "description"):
            if field_name not in data:
                raise SchemaParseError(message=f"Missing required field: {field_name} in schema for '{module_id}'")

        definitions = dict(data.get("definitions", {}))
        definitions.update(data.get("$defs", {}))

        description = data["description"]
        if len(description) > 200:
            logger.warning(f"Schema description for '{module_id}' exceeds 200 characters")

        sd = SchemaDefinition(
            module_id=data.get("module_id", module_id),
            description=description,
            input_schema=data["input_schema"],
            output_schema=data["output_schema"],
            error_schema=data.get("error_schema"),
            definitions=definitions,
            version=data.get("version", "1.0.0"),
            documentation=data.get("documentation"),
            schema_url=data.get("$schema"),
            source_path=file_path,
        )
        self._schema_cache[module_id] = sd
        return sd

    def resolve(self, schema_def: SchemaDefinition) -> tuple[ResolvedSchema, ResolvedSchema]:
        """Resolve all $ref references in a SchemaDefinition."""
        # D-104 (PROTOCOL_SPEC Algorithm A05 step 4a): a local `#/…` reference
        # resolves against the FILE ROOT first and falls back to the schema
        # node being resolved. Both layouts are normative, so both must load.
        #
        # This used to pass `current_file=None`, which made the file root
        # unreachable: `#/definitions/User` with `definitions:` as a top-level
        # sibling of `input_schema` — the layout §4.11's own example uses —
        # raised SCHEMA_NOT_FOUND here and loaded only on apcore-rust, while
        # `#/$defs/User` nested inside `input_schema` loaded here and not
        # there. Each SDK rejected what the other accepted, so no schema file
        # containing a local `$ref` loaded in all three.
        #
        # Handing the resolver the source file also makes
        # `SchemaDefinition.definitions` live: it was collected from the file's
        # top level and read by nothing, precisely because the references that
        # would have used it could not resolve.
        #
        # `source_path` is None for a definition built in memory rather than
        # read from a file; the resolver then behaves exactly as before.
        current_file = schema_def.source_path
        resolved_input = self._resolver.resolve(schema_def.input_schema, current_file)
        resolved_output = self._resolver.resolve(schema_def.output_schema, current_file)

        input_model = self.generate_model(resolved_input, f"{schema_def.module_id}_Input")
        output_model = self.generate_model(resolved_output, f"{schema_def.module_id}_Output")

        input_rs = ResolvedSchema(
            json_schema=resolved_input,
            model=input_model,
            module_id=schema_def.module_id,
            direction="input",
        )
        output_rs = ResolvedSchema(
            json_schema=resolved_output,
            model=output_model,
            module_id=schema_def.module_id,
            direction="output",
        )
        return input_rs, output_rs

    def generate_model(
        self, json_schema: dict[str, Any], model_name: str, *, root: dict[str, Any] | None = None
    ) -> type[BaseModel]:
        """Dynamically generate a Pydantic BaseModel from a JSON Schema dict.

        ``root`` is the document that ``#``-anchored ``$ref`` strings resolve
        against. It defaults to *json_schema* on the outermost call and is passed
        down unchanged through every nested model, so a ``$ref`` buried in
        ``properties`` still finds ``$defs`` at the top of the document.
        """
        if root is None:
            root = json_schema
        properties = json_schema.get("properties", {})
        required = set(json_schema.get("required", []))

        # `additionalProperties: false` forbids undeclared keys; every other form
        # permits them, so they are kept rather than silently dropped (Pydantic's
        # default extra="ignore" would drop them, diverging from the other SDKs).
        # A sub-schema form additionally constrains their values, which Pydantic
        # expresses as a typed __pydantic_extra__ annotation. `true` and `{}` are
        # the same always-true assertion and are treated alike.
        # A `patternProperties` sibling redefines which keys are "additional"
        # (§10.3.2.3 exempts every pattern-matched one), which `extra="forbid"`
        # cannot express; the delegated applicator assertion enforces the pair.
        additional = json_schema.get("additionalProperties", True)
        forbid_extra = additional is False and "patternProperties" not in json_schema
        config = ConfigDict(extra="forbid") if forbid_extra else ConfigDict(extra="allow")

        field_definitions: dict[str, Any] = {}
        if isinstance(additional, dict) and additional:
            # Asserted through jsonschema rather than mapped to a native annotation:
            # the sub-schema may be an array, a nested object, a bare constraint set
            # or a combinator, and a shallow type mapping silently widens all of
            # those to Any.
            extra_type = Annotated[Any, AfterValidator(_make_schema_assertion(additional))]
            field_definitions["__pydantic_extra__"] = (dict[str, extra_type], ...)  # type: ignore[valid-type]
        for prop_name, prop_schema in properties.items():
            python_type, field_info = self._schema_to_field_info(prop_schema, prop_name, model_name, root)
            is_required = prop_name in required

            if not is_required:
                # Make type nullable if not already
                python_type = python_type | None  # type: ignore[operator]
                # Set default to None if no explicit default
                if field_info.default is PydanticUndefined:
                    is_arr = isinstance(prop_schema.get("type"), str) and prop_schema.get("type") == "array"
                    field_info = self._clone_field_with_default(prop_schema, None, is_array=is_arr)

            field_definitions[prop_name] = (python_type, field_info)

        # §6.5.1/§6.5.2 on the schema this model was generated from. Attached at
        # model level because the count belongs to the whole instance, not to any
        # one field; `_handle_object` covers the object schemas that never reach
        # `create_model` because they declare no `properties`.
        validators: dict[str, Any] = {}
        count_check = _make_property_count_check(json_schema)
        if count_check is not None:
            validators["_apcore_property_count"] = _as_model_before_validator(count_check)

        # §10.3/§11 applicators declared on the schema this model was generated
        # from. Model level for the same reason as the count above: they assert
        # over the whole instance, not over any one field.
        applicators = _applicator_assertion_schema(json_schema)
        if applicators is not None:
            validators["_apcore_applicators"] = _as_model_before_validator(
                _make_schema_assertion(applicators, plain_data=True)
            )

        # §10.2 combinators declared on the schema this model was generated from.
        # `_schema_to_field_info` already intersects them onto every *property*,
        # but a combinator at the top of the schema had no owner: a root-level
        # `oneOf`/`anyOf` declares no `properties`, so `create_model` produced a
        # field-less `extra="allow"` model that accepted literally anything. That
        # left the union rule enforced only when validation happened to go through
        # `SchemaValidator._validate_top_level_union`, and bypassed entirely by the
        # bare `model_validate()` call in `BuiltinInputValidation` (spec §4.15.1).
        top_level = {key: json_schema[key] for key in _COMBINATOR_KEYWORDS if key in json_schema}
        if top_level:
            validators["_apcore_combinators"] = _as_model_before_validator(
                _make_schema_assertion(top_level, plain_data=True)
            )

        # A lazy `$ref` at the top of the schema — a schema that is nothing but a
        # reference into its own document — has no property to hang off either.
        top_ref = json_schema.get("$ref")
        if isinstance(top_ref, str) and json_schema is not root:
            built = _ref_assertion_schema(top_ref, root)
            if built is not None:
                ref_schema, registry = built
                validators["_apcore_ref"] = _as_model_before_validator(
                    _make_schema_assertion(ref_schema, plain_data=True, registry=registry)
                )

        model = create_model(  # type: ignore[call-overload]
            model_name,
            __config__=config,
            __validators__=validators or None,
            **field_definitions,
        )
        # A-D-08: retain the source JSON Schema on the generated model so
        # SchemaValidator.validate() can detect a top-level oneOf/anyOf and route
        # it through the jsonschema-backed exhaustive union check (parity with the
        # TS/Rust validators, which emit SCHEMA_UNION_NO_MATCH / SCHEMA_UNION_AMBIGUOUS).
        # A top-level union produces a property-less Pydantic model that would
        # otherwise accept any value via the always-true empty-schema path.
        model.__apcore_source_schema__ = json_schema  # type: ignore[attr-defined]
        # Whether the model carries assertions that live outside `model_fields`.
        # `SchemaValidator.validate` short-circuits a field-less model as the
        # always-true empty schema; a root-level combinator, applicator or
        # property-count check produces exactly such a model while asserting
        # plenty, so the shortcut has to know to stand down.
        model.__apcore_has_assertions__ = bool(validators)  # type: ignore[attr-defined]
        return model

    def _schema_to_field_info(
        self, prop_schema: dict[str, Any], prop_name: str, parent_name: str, root: dict[str, Any]
    ) -> tuple[Any, Any]:
        """Convert a JSON Schema property to (python_type, FieldInfo).

        `type` and its combinator siblings (`const`, `enum`, `oneOf`, `anyOf`,
        `allOf`, `not`) are independent assertions that must all hold
        (JSON Schema 2020-12 §10.2). The annotation comes from whichever keyword
        expresses it natively; every sibling that annotation does not already
        cover is intersected onto it as a jsonschema-backed check, rather than
        the first matching keyword winning and the rest being discarded.
        """
        if not prop_schema:
            # The empty schema asserts nothing (Draft 2020-12 always-true), so it
            # must accept every instance type, not just a mapping.
            return Any, Field(default=...)

        python_type, consumed, is_array = self._base_annotation(prop_schema, prop_name, parent_name, root)

        siblings = {key: prop_schema[key] for key in _COMBINATOR_KEYWORDS if key in prop_schema and key not in consumed}
        if siblings:
            assertion = _make_schema_assertion(siblings, plain_data=_is_json_native(prop_schema))
            python_type = Annotated[python_type, AfterValidator(assertion)]

        # `before` so the applicators see the raw instance: they assert over key
        # presence and array positions, both of which conversion erases.
        applicators = _applicator_assertion_schema(prop_schema)
        if applicators is not None:
            python_type = Annotated[python_type, BeforeValidator(_make_schema_assertion(applicators, plain_data=True))]

        # The §6/§10.3 keywords a `type`-less position has no annotation to carry.
        # `before` for the same reason as the applicators, and because the
        # annotation there is `Any` — there is nothing for conversion to hand over.
        bare = _bare_assertion_schema(prop_schema, applicators)
        if bare is not None:
            python_type = Annotated[python_type, BeforeValidator(_make_schema_assertion(bare, plain_data=True))]

        return python_type, self._build_field(prop_schema, is_array=is_array)

    def _base_annotation(
        self, prop_schema: dict[str, Any], prop_name: str, parent_name: str, root: dict[str, Any]
    ) -> tuple[Any, tuple[str, ...], bool]:
        """Derive the annotation from the keyword that can express it natively.

        Returns (annotation, keywords_consumed, is_array). A keyword is consumed
        only when the annotation fully enforces it; anything left over is applied
        by the caller as a sibling assertion.
        """
        # A lazy `$ref` the resolver deliberately left in place: the sub-tree is
        # recursive, so no finite Pydantic annotation describes it. §8.2.3.1 also
        # makes `$ref` override every sibling keyword in 2020-12 only for
        # `$recursiveRef`-free documents — the delegated check honours whichever
        # applies.
        if "$ref" in prop_schema:
            return self._ref_annotation(prop_schema["$ref"], root), ("$ref",), False

        schema_type = prop_schema.get("type")

        if isinstance(schema_type, list):
            return self._union_from_types(prop_schema, schema_type, prop_name, parent_name, root), (), False

        if schema_type == "object":
            return self._handle_object(prop_schema, prop_name, parent_name, root), (), False

        if schema_type == "array":
            return self._handle_array(prop_schema, prop_name, parent_name, root), (), True

        if isinstance(schema_type, str):
            return _scalar_annotation(schema_type), (), False

        # No `type`: let a combinator carry the annotation itself.
        if "const" in prop_schema:
            return Literal[prop_schema["const"]], ("const",), False

        if "enum" in prop_schema:
            return Literal[tuple(prop_schema["enum"])], ("enum",), False

        for keyword in ("oneOf", "anyOf"):
            if keyword in prop_schema:
                types = [
                    self._schema_to_type(sub, f"{prop_name}_{keyword}_{i}", parent_name, root)
                    for i, sub in enumerate(prop_schema[keyword])
                ]
                # The branch annotations are shape-only (`_schema_to_type` widens a
                # branch without `type` to Any), so the keyword stays unconsumed and
                # the exhaustive check is applied as a sibling assertion.
                return Union[tuple(types)], (), False

        if "allOf" in prop_schema:
            # `_handle_all_of` merges the members into one model, and that merge is
            # lossy (a later member's property definition overwrites an earlier one).
            # The keyword stays unconsumed so the sibling assertion enforces what the
            # merge dropped — the same treatment oneOf/anyOf get above.
            return self._handle_all_of(prop_schema["allOf"], prop_name, parent_name, root), (), False

        if "not" in prop_schema:
            return Any, (), False

        # No `type` and no combinator: whatever else the schema carries (a bare
        # §6 constraint, an applicator) applies only to instances of its own type
        # and is inert on every other one, so the annotation must not narrow to a
        # mapping. `Any` leaves the assertions to the delegated checks.
        return Any, (), False

    def _ref_annotation(self, ref: Any, root: dict[str, Any]) -> Any:
        """Annotation for a lazy `$ref`, delegating the whole sub-tree to jsonschema.

        A `before` validator, so the raw instance is what gets checked: the target
        is typically recursive and no finite Pydantic annotation describes it, so
        there is nothing for a coercing conversion to hand over afterwards.
        """
        if not isinstance(ref, str):
            return Any
        built = _ref_assertion_schema(ref, root)
        if built is None:
            # An external reference this SDK cannot anchor asserts nothing rather
            # than failing the whole conversion — the same widening the
            # apcore-typescript converter applies.
            return Any
        ref_schema, registry = built
        assertion = _make_schema_assertion(ref_schema, plain_data=True, registry=registry)
        return Annotated[Any, BeforeValidator(assertion)]

    def _union_from_types(
        self,
        prop_schema: dict[str, Any],
        schema_type: list[str],
        prop_name: str,
        parent_name: str,
        root: dict[str, Any],
    ) -> Any:
        """Convert a `type` array to a union, keeping each type's option keywords on its own branch."""
        branches: list[Any] = []
        for type_name in schema_type:
            if type_name == "object":
                branches.append(self._handle_object(prop_schema, prop_name, parent_name, root))
                continue
            if type_name not in _TYPE_MAP and type_name != "array":
                raise SchemaParseError(message=f"Unknown type '{type_name}' in type array for '{prop_name}'")

            if type_name == "array":
                base_type = self._handle_array(prop_schema, prop_name, parent_name, root)
            else:
                base_type = _scalar_annotation(type_name)
            constraints = _branch_constraints(prop_schema, type_name)
            branches.append(Annotated[base_type, Field(**constraints)] if constraints else base_type)

        if not branches:
            # The metaschema requires `type` arrays to be non-empty; an empty one
            # asserts nothing can validate, which is never what a contract means.
            raise SchemaParseError(message=f"Empty type array for '{prop_name}'")
        return Union[tuple(branches)]

    def _schema_to_type(self, schema: dict[str, Any], name: str, parent_name: str, root: dict[str, Any]) -> Any:
        """Convert a sub-schema to a Python type (for Union branches)."""
        if "$ref" in schema:
            return self._ref_annotation(schema["$ref"], root)
        schema_type = schema.get("type")
        if schema_type == "object" and "properties" in schema:
            return self.generate_model(schema, f"{parent_name}_{name}", root=root)
        if schema_type and isinstance(schema_type, str):
            # `_scalar_annotation`, not a bare `_TYPE_MAP` lookup: array items and
            # union branches need the same bool / JSON-integer guards a top-level
            # scalar property gets, or `{"items": {"type": "integer"}}` diverges
            # from `{"type": "integer"}` for the same instance.
            return _scalar_annotation(schema_type)
        return Any

    def _handle_object(
        self, prop_schema: dict[str, Any], prop_name: str, parent_name: str, root: dict[str, Any]
    ) -> Any:
        """Handle object type schemas.

        A schema with `properties` becomes a generated model, which carries its own
        `minProperties`/`maxProperties` check; the open-mapping forms below get one
        attached here, since they never reach `create_model`.
        """
        if "properties" in prop_schema:
            return self.generate_model(prop_schema, f"{parent_name}_{prop_name}", root=root)
        annotation: Any = dict[str, Any]
        additional = prop_schema.get("additionalProperties")
        if isinstance(additional, dict) and "type" in additional:
            value_type = _scalar_annotation(additional["type"])
            annotation = dict[str, value_type]  # type: ignore[valid-type]
        return _with_property_count(annotation, prop_schema)

    def _handle_array(self, prop_schema: dict[str, Any], prop_name: str, parent_name: str, root: dict[str, Any]) -> Any:
        """Handle array type schemas."""
        # With a `prefixItems` sibling, `items` describes only the positions past
        # the prefix (§10.3.1.2); `list[T]` would apply it to the tuple head too,
        # so both are left to the delegated applicator assertion.
        items = None if "prefixItems" in prop_schema else prop_schema.get("items")
        if items:
            item_type = self._item_annotation(items, f"{prop_name}_item", parent_name, root)
            base_type: Any = list[item_type]  # type: ignore[valid-type]
        else:
            base_type = list[Any]

        if prop_schema.get("uniqueItems"):
            base_type = Annotated[base_type, AfterValidator(_check_unique)]

        contains = prop_schema.get("contains")
        if isinstance(contains, dict):
            # §6.4.4/§6.4.5 assert nothing on their own — they are meaningful only
            # alongside `contains` (§10.3.1.3), so the three travel together. No
            # Pydantic annotation expresses "at least N items match this sub-schema",
            # so the group is delegated to jsonschema, as combinator siblings are.
            # apcore-typescript emits the same trio onto its TypeBox array options.
            group = {"contains": contains}
            for keyword in ("minContains", "maxContains"):
                if keyword in prop_schema:
                    group[keyword] = prop_schema[keyword]
            assertion = _make_schema_assertion(group, plain_data=_is_json_native(prop_schema))
            base_type = Annotated[base_type, AfterValidator(assertion)]

        return base_type

    def _item_annotation(self, schema: dict[str, Any], name: str, parent_name: str, root: dict[str, Any]) -> Any:
        """Annotation for an array element.

        `_schema_to_type` derives the *shape* only. The combinator and applicator
        keywords sitting beside it are independent assertions (§10.2/§10.3) that a
        `list[T]` annotation cannot express, so they are delegated to jsonschema
        exactly as `_schema_to_field_info` does for a property — otherwise
        `{"items": {"oneOf": [...]}}` widened every element to `Any` and the
        exclusivity rule vanished inside arrays.
        """
        annotation = self._schema_to_type(schema, name, parent_name, root)

        siblings = {key: schema[key] for key in _COMBINATOR_KEYWORDS if key in schema}
        if siblings:
            assertion = _make_schema_assertion(siblings, plain_data=_is_json_native(schema))
            annotation = Annotated[annotation, AfterValidator(assertion)]

        applicators = _applicator_assertion_schema(schema)
        if applicators is not None:
            annotation = Annotated[annotation, BeforeValidator(_make_schema_assertion(applicators, plain_data=True))]

        # §17.3: the §6/§10.3 keywords hold "inside `items` … exactly as at the
        # top level", so a type-less element schema gets the same delegation a
        # type-less property does.
        bare = _bare_assertion_schema(schema, applicators)
        if bare is not None:
            annotation = Annotated[annotation, BeforeValidator(_make_schema_assertion(bare, plain_data=True))]
        return annotation

    def _handle_all_of(
        self, sub_schemas: list[dict[str, Any]], prop_name: str, parent_name: str, root: dict[str, Any]
    ) -> Any:
        """Merge allOf sub-schemas into a single model, or widen to `Any`.

        The merge only describes an *object* intersection. When a member is not
        object-shaped — `{"type": "string"}`, a bare `{"minimum": 1}`, an
        `{"enum": [...]}` — there is no model to merge and the annotation widens
        to `Any`, exactly as `not` does.

        Widening is not a silent drop. `allOf` is left unconsumed by
        `_base_annotation`, so `_schema_to_field_info` hands the whole keyword to
        jsonschema as a sibling assertion and every member is still enforced.
        Raising `SchemaParseError` here instead made a contract apcore-typescript
        and apcore-rust both accept *and enforce* impossible to even register in
        Python. §17.1 R1 licenses a load-time rejection only when the keyword
        cannot be enforced, and this one can.
        """
        if any(sub.get("type") != "object" and "properties" not in sub for sub in sub_schemas):
            return Any

        merged_properties: dict[str, Any] = {}
        merged_required: list[str] = []

        for sub in sub_schemas:
            for name, prop in sub.get("properties", {}).items():
                if name in merged_properties:
                    existing_type = merged_properties[name].get("type")
                    new_type = prop.get("type")
                    if existing_type and new_type and existing_type != new_type:
                        raise SchemaParseError(
                            message=f"allOf conflict: property '{name}' has conflicting types in '{prop_name}'"
                        )
                merged_properties[name] = prop
            merged_required.extend(sub.get("required", []))

        merged_schema = {
            "type": "object",
            "properties": merged_properties,
            "required": list(set(merged_required)),
        }
        return self.generate_model(merged_schema, f"{parent_name}_{prop_name}", root=root)

    def _build_field(self, prop_schema: dict[str, Any], is_array: bool = False) -> Any:
        """Build a Pydantic Field from JSON Schema constraints."""
        kwargs: dict[str, Any] = {"default": prop_schema.get("default", ...)}

        if "description" in prop_schema:
            kwargs["description"] = prop_schema["description"]
        if "title" in prop_schema:
            kwargs["title"] = prop_schema["title"]

        kwargs.update(_field_constraints(prop_schema, is_array=is_array))

        # LLM extensions and format as json_schema_extra
        extra: dict[str, Any] = {key: value for key, value in prop_schema.items() if key.startswith("x-")}
        if "format" in prop_schema:
            extra["format"] = prop_schema["format"]
        if extra:
            kwargs["json_schema_extra"] = extra

        return Field(**kwargs)

    def _clone_field_with_default(self, prop_schema: dict[str, Any], default: Any, is_array: bool = False) -> Any:
        """Build a new Field with the given default, preserving all constraints from schema."""
        schema_with_default = dict(prop_schema)
        schema_with_default["default"] = default
        return self._build_field(schema_with_default, is_array=is_array)

    def get_schema(
        self,
        module_id: str,
        native_input_schema: type[BaseModel] | None = None,
        native_output_schema: type[BaseModel] | None = None,
    ) -> tuple[ResolvedSchema, ResolvedSchema]:
        """Get resolved schemas using the configured loading strategy."""
        if module_id in self._path_index:
            return self._content_cache[self._path_index[module_id]]

        strategy = SchemaStrategy(self._config.get("schema.strategy", "yaml_first"))
        result: tuple[ResolvedSchema, ResolvedSchema] | None = None

        if strategy == SchemaStrategy.YAML_FIRST:
            try:
                result = self._load_and_resolve(module_id)
            except SchemaNotFoundError:
                if native_input_schema and native_output_schema:
                    result = self._wrap_native(module_id, native_input_schema, native_output_schema)
                else:
                    raise

        elif strategy == SchemaStrategy.NATIVE_FIRST:
            if native_input_schema and native_output_schema:
                result = self._wrap_native(module_id, native_input_schema, native_output_schema)
            else:
                result = self._load_and_resolve(module_id)

        elif strategy == SchemaStrategy.YAML_ONLY:
            result = self._load_and_resolve(module_id)

        if result is None:
            raise SchemaNotFoundError(schema_id=module_id)

        self._store_in_cache(module_id, result)
        return result

    def _load_and_resolve(self, module_id: str) -> tuple[ResolvedSchema, ResolvedSchema]:
        """Load and resolve a schema, using the two-level content-addressable cache."""
        if module_id in self._path_index:
            return self._content_cache[self._path_index[module_id]]
        sd = self.load(module_id)
        result = self.resolve(sd)
        self._store_in_cache(module_id, result)
        return result

    def _store_in_cache(self, module_id: str, result: tuple[ResolvedSchema, ResolvedSchema]) -> None:
        """Store a resolved schema pair in the two-level content-addressable cache."""
        combined = {"input": result[0].json_schema, "output": result[1].json_schema}
        digest = content_hash(combined)
        if digest not in self._content_cache:
            self._content_cache[digest] = result
        self._path_index[module_id] = digest

    def _wrap_native(
        self,
        module_id: str,
        input_model: type[BaseModel],
        output_model: type[BaseModel],
    ) -> tuple[ResolvedSchema, ResolvedSchema]:
        """Wrap native Pydantic models as ResolvedSchema without re-generating."""
        input_rs = ResolvedSchema(
            json_schema=input_model.model_json_schema(),
            model=input_model,
            module_id=module_id,
            direction="input",
        )
        output_rs = ResolvedSchema(
            json_schema=output_model.model_json_schema(),
            model=output_model,
            module_id=module_id,
            direction="output",
        )
        return input_rs, output_rs

    def clear_cache(self) -> None:
        """Clear all internal caches."""
        self._schema_cache.clear()
        self._path_index.clear()
        self._content_cache.clear()
        self._resolver.clear_cache()
