"""Tests for SchemaLoader."""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any

import pytest
import yaml
from pydantic import BaseModel, ValidationError, field_validator

from apcore.config import Config
from apcore.errors import SchemaNotFoundError, SchemaParseError
from apcore.schema.loader import SchemaLoader
from apcore.schema.types import ResolvedSchema


def write_yaml(path: Path, data: dict[str, Any]) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(yaml.dump(data, default_flow_style=False))
    return path


def make_loader(schemas_dir: Path) -> SchemaLoader:
    config = Config(
        {
            "schema": {
                "root": str(schemas_dir),
                "strategy": "yaml_first",
                "max_ref_depth": 32,
            }
        }
    )
    return SchemaLoader(config, schemas_dir=schemas_dir)


def write_simple_schema(schemas_dir: Path, name: str = "simple") -> Path:
    return write_yaml(
        schemas_dir / f"{name}.schema.yaml",
        {
            "module_id": name,
            "description": f"A {name} schema",
            "input_schema": {
                "type": "object",
                "properties": {"table": {"type": "string"}},
                "required": ["table"],
            },
            "output_schema": {
                "type": "object",
                "properties": {"rows": {"type": "array", "items": {"type": "string"}}},
            },
        },
    )


# === load() ===


class TestLoad:
    def test_load_valid_schema(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)
        loader = make_loader(tmp_path)
        sd = loader.load("simple")
        assert sd.module_id == "simple"
        assert sd.description == "A simple schema"
        assert "table" in sd.input_schema["properties"]

    def test_module_id_dots_to_path(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "db" / "query.schema.yaml",
            {
                "module_id": "db.query",
                "description": "Query",
                "input_schema": {"type": "object"},
                "output_schema": {"type": "object"},
            },
        )
        loader = make_loader(tmp_path)
        sd = loader.load("db.query")
        assert sd.module_id == "db.query"

    def test_missing_schema_raises(self, tmp_path: Path) -> None:
        loader = make_loader(tmp_path)
        with pytest.raises(SchemaNotFoundError):
            loader.load("nonexistent.module")

    def test_invalid_yaml_raises(self, tmp_path: Path) -> None:
        bad = tmp_path / "bad.schema.yaml"
        bad.write_text("{{invalid: yaml: ---")
        loader = make_loader(tmp_path)
        with pytest.raises(SchemaParseError):
            loader.load("bad")

    def test_missing_required_field_raises(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "incomplete.schema.yaml",
            {"module_id": "incomplete", "description": "No input"},
        )
        loader = make_loader(tmp_path)
        with pytest.raises(SchemaParseError, match="input_schema"):
            loader.load("incomplete")

    def test_empty_file_raises(self, tmp_path: Path) -> None:
        (tmp_path / "empty.schema.yaml").write_text("")
        loader = make_loader(tmp_path)
        with pytest.raises(SchemaParseError):
            loader.load("empty")

    def test_defs_and_definitions_merge(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "merged.schema.yaml",
            {
                "module_id": "merged",
                "description": "Merged defs",
                "input_schema": {"type": "object"},
                "output_schema": {"type": "object"},
                "definitions": {
                    "Foo": {"type": "object", "properties": {"x": {"type": "string"}}},
                    "Bar": {"type": "integer"},
                },
                "$defs": {"Foo": {"type": "object", "properties": {"z": {"type": "boolean"}}}},
            },
        )
        loader = make_loader(tmp_path)
        sd = loader.load("merged")
        assert "Bar" in sd.definitions
        assert sd.definitions["Foo"]["properties"]["z"]["type"] == "boolean"

    def test_long_description_logs_warning(self, tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
        write_yaml(
            tmp_path / "long.schema.yaml",
            {
                "module_id": "long",
                "description": "x" * 201,
                "input_schema": {"type": "object"},
                "output_schema": {"type": "object"},
            },
        )
        loader = make_loader(tmp_path)
        with caplog.at_level(logging.WARNING, logger="apcore"):
            sd = loader.load("long")
        assert sd is not None
        assert "200 characters" in caplog.text

    def test_caching(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)
        loader = make_loader(tmp_path)
        sd1 = loader.load("simple")
        sd2 = loader.load("simple")
        assert sd1 is sd2


# === resolve() ===


class TestResolve:
    def test_resolve_no_refs(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)
        loader = make_loader(tmp_path)
        sd = loader.load("simple")
        input_rs, output_rs = loader.resolve(sd)
        assert input_rs.json_schema["properties"]["table"]["type"] == "string"
        assert input_rs.direction == "input"
        assert output_rs.direction == "output"

    def test_resolve_local_ref(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "withref.schema.yaml",
            {
                "module_id": "withref",
                "description": "With ref",
                "input_schema": {
                    "type": "object",
                    "properties": {"addr": {"$ref": "#/definitions/Address"}},
                    "definitions": {
                        "Address": {
                            "type": "object",
                            "properties": {"city": {"type": "string"}},
                        }
                    },
                },
                "output_schema": {"type": "object"},
            },
        )
        loader = make_loader(tmp_path)
        sd = loader.load("withref")
        input_rs, _ = loader.resolve(sd)
        assert "city" in input_rs.json_schema["properties"]["addr"]["properties"]

    def test_resolve_returns_tuple(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)
        loader = make_loader(tmp_path)
        sd = loader.load("simple")
        result = loader.resolve(sd)
        assert isinstance(result, tuple)
        assert len(result) == 2
        assert isinstance(result[0], ResolvedSchema)
        assert isinstance(result[1], ResolvedSchema)


# === generate_model() ===


class TestGenerateModel:
    def _make_loader(self, tmp_path: Path) -> SchemaLoader:
        return make_loader(tmp_path)

    def test_string_field(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            },
            "TestStr",
        )
        obj = Model(name="hello")
        assert obj.name == "hello"

    def test_integer_field(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"count": {"type": "integer"}},
                "required": ["count"],
            },
            "TestInt",
        )
        obj = Model(count=42)
        assert obj.count == 42

    def test_number_field(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"ratio": {"type": "number"}},
                "required": ["ratio"],
            },
            "TestNum",
        )
        obj = Model(ratio=3.14)
        assert obj.ratio == 3.14

    def test_boolean_field(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"active": {"type": "boolean"}},
                "required": ["active"],
            },
            "TestBool",
        )
        obj = Model(active=True)
        assert obj.active is True

    def test_nested_object(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "address": {
                        "type": "object",
                        "properties": {"city": {"type": "string"}},
                        "required": ["city"],
                    }
                },
                "required": ["address"],
            },
            "TestNested",
        )
        obj = Model(address={"city": "NY"})
        assert obj.address.city == "NY"

    def test_array_with_items(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"tags": {"type": "array", "items": {"type": "string"}}},
                "required": ["tags"],
            },
            "TestArr",
        )
        obj = Model(tags=["a", "b"])
        assert obj.tags == ["a", "b"]

    def test_array_without_items(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"data": {"type": "array"}},
                "required": ["data"],
            },
            "TestArrAny",
        )
        obj = Model(data=[1, "x", True])
        assert len(obj.data) == 3

    def test_additional_properties(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "meta": {
                        "type": "object",
                        "additionalProperties": {"type": "string"},
                    }
                },
                "required": ["meta"],
            },
            "TestAdditional",
        )
        obj = Model(meta={"key": "value"})
        assert obj.meta == {"key": "value"}

    def test_empty_schema(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {"type": "object", "properties": {"payload": {}}, "required": ["payload"]},
            "TestEmpty",
        )
        obj = Model(payload={"any": "thing"})
        assert obj.payload == {"any": "thing"}

    def test_nullable_type(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"name": {"type": ["string", "null"]}},
                "required": ["name"],
            },
            "TestNull",
        )
        assert Model(name=None).name is None
        assert Model(name="hello").name == "hello"

    def test_enum_type(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"status": {"type": "string", "enum": ["active", "inactive"]}},
                "required": ["status"],
            },
            "TestEnum",
        )
        assert Model(status="active").status == "active"
        with pytest.raises(Exception):
            Model(status="unknown")

    def test_const_type(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"version": {"const": "1.0"}},
                "required": ["version"],
            },
            "TestConst",
        )
        assert Model(version="1.0").version == "1.0"
        with pytest.raises(Exception):
            Model(version="2.0")

    def test_required_field_no_default(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            },
            "TestReq",
        )
        with pytest.raises(Exception):
            Model()

    def test_optional_field_default_none(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {"type": "object", "properties": {"nickname": {"type": "string"}}},
            "TestOpt",
        )
        obj = Model()
        assert obj.nickname is None

    def test_default_value(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"retries": {"type": "integer", "default": 3}},
            },
            "TestDefault",
        )
        obj = Model()
        assert obj.retries == 3

    def test_constraints_min_max(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"age": {"type": "integer", "minimum": 0, "maximum": 150}},
                "required": ["age"],
            },
            "TestMinMax",
        )
        assert Model(age=25).age == 25
        with pytest.raises(Exception):
            Model(age=-1)
        with pytest.raises(Exception):
            Model(age=200)

    def test_constraints_string_length(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"code": {"type": "string", "minLength": 3, "maxLength": 10}},
                "required": ["code"],
            },
            "TestStrLen",
        )
        assert Model(code="abc").code == "abc"
        with pytest.raises(Exception):
            Model(code="ab")
        with pytest.raises(Exception):
            Model(code="a" * 11)

    def test_constraints_pattern(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"id": {"type": "string", "pattern": "^[A-Z]{3}$"}},
                "required": ["id"],
            },
            "TestPattern",
        )
        assert Model(id="ABC").id == "ABC"
        with pytest.raises(Exception):
            Model(id="abc")

    def test_unique_items(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "ids": {
                        "type": "array",
                        "items": {"type": "integer"},
                        "uniqueItems": True,
                    }
                },
                "required": ["ids"],
            },
            "TestUnique",
        )
        assert Model(ids=[1, 2, 3]).ids == [1, 2, 3]
        with pytest.raises(Exception):
            Model(ids=[1, 1, 2])

    def test_multiple_of(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"qty": {"type": "integer", "multipleOf": 5}},
                "required": ["qty"],
            },
            "TestMultiple",
        )
        assert Model(qty=10).qty == 10
        with pytest.raises(Exception):
            Model(qty=7)

    def test_one_of(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"value": {"oneOf": [{"type": "string"}, {"type": "integer"}]}},
                "required": ["value"],
            },
            "TestOneOf",
        )
        assert Model(value="hello").value == "hello"
        assert Model(value=42).value == 42

    def test_any_of(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"value": {"anyOf": [{"type": "string"}, {"type": "integer"}]}},
                "required": ["value"],
            },
            "TestAnyOf",
        )
        assert Model(value="hello").value == "hello"
        assert Model(value=42).value == 42

    def test_all_of(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "person": {
                        "allOf": [
                            {
                                "type": "object",
                                "properties": {"name": {"type": "string"}},
                                "required": ["name"],
                            },
                            {
                                "type": "object",
                                "properties": {"age": {"type": "integer"}},
                                "required": ["age"],
                            },
                        ]
                    }
                },
                "required": ["person"],
            },
            "TestAllOf",
        )
        obj = Model(person={"name": "Alice", "age": 30})
        assert obj.person.name == "Alice"
        assert obj.person.age == 30

    def test_all_of_conflict_raises(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        with pytest.raises(SchemaParseError, match="conflict"):
            loader.generate_model(
                {
                    "type": "object",
                    "properties": {
                        "x": {
                            "allOf": [
                                {
                                    "type": "object",
                                    "properties": {"val": {"type": "string"}},
                                },
                                {
                                    "type": "object",
                                    "properties": {"val": {"type": "integer"}},
                                },
                            ]
                        }
                    },
                    "required": ["x"],
                },
                "TestConflict",
            )

    def test_not_is_enforced(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"x": {"not": {"type": "string"}}},
                "required": ["x"],
            },
            "TestNot",
        )
        assert Model(x=42).x == 42
        with pytest.raises(Exception):
            Model(x="a string is excluded by not")

    def test_if_then_else_is_enforced(self, tmp_path: Path) -> None:
        """`if`/`then`/`else` used to raise SchemaParseError; it is asserted now (§10.2.2)."""
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "x": {
                        "if": {"type": "string"},
                        "then": {"minLength": 1},
                        "else": {"type": "integer"},
                    }
                },
                "required": ["x"],
            },
            "TestIfThen",
        )
        assert Model.model_validate({"x": "ok"}).x == "ok"
        assert Model.model_validate({"x": 42}).x == 42
        with pytest.raises(ValidationError):
            Model.model_validate({"x": ""})  # then: minLength 1
        with pytest.raises(ValidationError):
            Model.model_validate({"x": 1.5})  # else: integer

    def test_prefix_items_is_enforced(self, tmp_path: Path) -> None:
        """`prefixItems` positions are checked, and `items` applies only past the prefix."""
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "v": {
                        "type": "array",
                        "prefixItems": [{"type": "string"}],
                        "items": {"type": "integer"},
                    }
                },
                "required": ["v"],
            },
            "TestPrefixItems",
        )
        assert Model.model_validate({"v": ["a", 3]}).v == ["a", 3]
        with pytest.raises(ValidationError):
            Model.model_validate({"v": [1, 3]})  # the prefix position must be a string
        with pytest.raises(ValidationError):
            Model.model_validate({"v": ["a", "b"]})  # the tail must be an integer

    def test_object_applicators_are_enforced(self, tmp_path: Path) -> None:
        """patternProperties / propertyNames / dependentRequired / dependentSchemas."""
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "v": {
                        "type": "object",
                        "patternProperties": {"^S_": {"type": "string"}},
                        "propertyNames": {"maxLength": 4},
                        "dependentRequired": {"a": ["b"]},
                        "dependentSchemas": {"c": {"required": ["d"]}},
                    }
                },
                "required": ["v"],
            },
            "TestObjectApplicators",
        )
        assert Model.model_validate({"v": {"S_x": "s"}}).v == {"S_x": "s"}
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"S_x": 1}})  # patternProperties
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"toolong": 1}})  # propertyNames
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"a": 1}})  # dependentRequired
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"c": 1}})  # dependentSchemas

    def test_unevaluated_properties_is_enforced(self, tmp_path: Path) -> None:
        """`unevaluatedProperties` subtracts what the sibling applicators evaluated (§11.3)."""
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "v": {
                        "type": "object",
                        "allOf": [{"properties": {"a": {"type": "string"}}}],
                        "unevaluatedProperties": False,
                    }
                },
                "required": ["v"],
            },
            "TestUnevaluatedProperties",
        )
        assert Model.model_validate({"v": {"a": "x"}}).v == {"a": "x"}
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"a": "x", "z": 1}})

    def test_applicators_are_inert_on_other_instance_types(self, tmp_path: Path) -> None:
        """An applicator on a type-less schema must not narrow the accepted type."""
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "arr": {"prefixItems": [{"type": "string"}]},
                    "obj": {"dependentRequired": {"a": ["b"]}},
                },
                "required": ["arr", "obj"],
            },
            "TestApplicatorInert",
        )
        validated = Model.model_validate({"arr": "not-an-array", "obj": 42})
        assert validated.arr == "not-an-array"
        assert validated.obj == 42

    def test_root_level_applicators_are_enforced(self, tmp_path: Path) -> None:
        """An applicator on the root schema reaches the model, not only property schemas."""
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
                "dependentRequired": {"a": ["b"]},
            },
            "TestRootApplicators",
        )
        assert Model.model_validate({"a": 1, "b": 2}).a == 1
        with pytest.raises(ValidationError):
            Model.model_validate({"a": 1})

    def test_x_extensions_preserved(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "x-llm-description": "AI desc",
                        "x-sensitive": True,
                    }
                },
                "required": ["name"],
            },
            "TestExt",
        )
        schema = Model.model_json_schema()
        props = schema["properties"]["name"]
        assert props.get("x-llm-description") == "AI desc"
        assert props.get("x-sensitive") is True

    def test_format_not_enforced(self, tmp_path: Path) -> None:
        loader = self._make_loader(tmp_path)
        Model = loader.generate_model(
            {
                "type": "object",
                "properties": {"email": {"type": "string", "format": "email"}},
                "required": ["email"],
            },
            "TestFormat",
        )
        obj = Model(email="not-an-email")
        assert obj.email == "not-an-email"


# === get_schema() ===


class TestGetSchema:
    def test_yaml_first_uses_yaml(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)
        config = Config({"schema": {"root": str(tmp_path), "strategy": "yaml_first"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        input_rs, output_rs = loader.get_schema("simple")
        assert input_rs.module_id == "simple"

    def test_yaml_first_fallback_to_native(self, tmp_path: Path) -> None:
        class InputModel(BaseModel):
            name: str

        class OutputModel(BaseModel):
            result: str

        config = Config({"schema": {"root": str(tmp_path), "strategy": "yaml_first"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        input_rs, output_rs = loader.get_schema(
            "missing.module",
            native_input_schema=InputModel,
            native_output_schema=OutputModel,
        )
        assert input_rs.model is InputModel
        assert output_rs.model is OutputModel

    def test_yaml_first_neither_raises(self, tmp_path: Path) -> None:
        config = Config({"schema": {"root": str(tmp_path), "strategy": "yaml_first"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        with pytest.raises(SchemaNotFoundError):
            loader.get_schema("nonexistent")

    def test_native_first_uses_native(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)

        class InputModel(BaseModel):
            x: int

        class OutputModel(BaseModel):
            y: str

        config = Config({"schema": {"root": str(tmp_path), "strategy": "native_first"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        input_rs, _ = loader.get_schema("simple", native_input_schema=InputModel, native_output_schema=OutputModel)
        assert input_rs.model is InputModel

    def test_native_first_fallback_to_yaml(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)
        config = Config({"schema": {"root": str(tmp_path), "strategy": "native_first"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        input_rs, _ = loader.get_schema("simple")
        assert input_rs.module_id == "simple"

    def test_yaml_only_ignores_native(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)

        class InputModel(BaseModel):
            x: int

        class OutputModel(BaseModel):
            y: str

        config = Config({"schema": {"root": str(tmp_path), "strategy": "yaml_only"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        input_rs, _ = loader.get_schema("simple", native_input_schema=InputModel, native_output_schema=OutputModel)
        assert input_rs.model is not InputModel

    def test_yaml_only_not_found_raises(self, tmp_path: Path) -> None:
        config = Config({"schema": {"root": str(tmp_path), "strategy": "yaml_only"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        with pytest.raises(SchemaNotFoundError):
            loader.get_schema("nonexistent")

    def test_native_preserves_validators(self, tmp_path: Path) -> None:
        class ValidatedInput(BaseModel):
            name: str

            @field_validator("name")
            @classmethod
            def name_must_not_be_empty(cls, v: str) -> str:
                if not v:
                    raise ValueError("name must not be empty")
                return v

        class OutputModel(BaseModel):
            ok: bool

        config = Config({"schema": {"root": str(tmp_path), "strategy": "native_first"}})
        loader = SchemaLoader(config, schemas_dir=tmp_path)
        input_rs, _ = loader.get_schema("test", native_input_schema=ValidatedInput, native_output_schema=OutputModel)
        assert input_rs.model is ValidatedInput


# === clear_cache() ===


class TestContentAddressableCache:
    """Verify _store_in_cache deduplication: same content → one _content_cache entry."""

    def _make_schema_yaml(self, name: str, schema_body: dict) -> dict:
        return {
            "module_id": name,
            "description": f"Schema {name}",
            "input_schema": schema_body,
            "output_schema": {"type": "object"},
        }

    def test_same_content_shares_cache_entry(self, tmp_path: Path) -> None:
        identical_input = {
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"],
        }
        write_yaml(
            tmp_path / "alpha.schema.yaml",
            self._make_schema_yaml("alpha", identical_input),
        )
        write_yaml(
            tmp_path / "beta.schema.yaml",
            self._make_schema_yaml("beta", identical_input),
        )

        loader = make_loader(tmp_path)
        loader.get_schema("alpha")
        loader.get_schema("beta")

        assert (
            len(loader._content_cache) == 1
        ), "Two modules with identical input+output schemas must share one _content_cache entry"
        assert (
            loader._path_index["alpha"] == loader._path_index["beta"]
        ), "Both module_ids must map to the same SHA-256 digest"

    def test_different_content_produces_separate_entries(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "mod_a.schema.yaml",
            self._make_schema_yaml("mod_a", {"type": "object", "properties": {"x": {"type": "string"}}}),
        )
        write_yaml(
            tmp_path / "mod_b.schema.yaml",
            self._make_schema_yaml("mod_b", {"type": "object", "properties": {"y": {"type": "integer"}}}),
        )

        loader = make_loader(tmp_path)
        loader.get_schema("mod_a")
        loader.get_schema("mod_b")

        assert (
            len(loader._content_cache) == 2
        ), "Two modules with different schemas must produce separate _content_cache entries"
        assert loader._path_index["mod_a"] != loader._path_index["mod_b"]


class TestClearCache:
    def test_clear_cache_invalidates(self, tmp_path: Path) -> None:
        write_simple_schema(tmp_path)
        loader = make_loader(tmp_path)
        sd1 = loader.load("simple")
        write_yaml(
            tmp_path / "simple.schema.yaml",
            {
                "module_id": "simple",
                "description": "Updated",
                "input_schema": {"type": "object"},
                "output_schema": {"type": "object"},
            },
        )
        loader.clear_cache()
        sd2 = loader.load("simple")
        assert sd2.description == "Updated"
        assert sd1 is not sd2


class TestTypeAndCombinatorIntersection:
    """`type` and its combinator siblings are independent assertions (JSON Schema 2020-12 §10.2).

    Both must hold: neither the type nor the sibling may be discarded in favour of
    the other. Matches apcore-typescript and apcore-rust.
    """

    def _model(self, tmp_path: Path, prop_schema: dict[str, Any], name: str) -> type[BaseModel]:
        return make_loader(tmp_path).generate_model(
            {"type": "object", "properties": {"v": prop_schema}, "required": ["v"]},
            name,
        )

    def test_type_array_converts_to_a_union(self, tmp_path: Path) -> None:
        Model = self._model(tmp_path, {"type": ["string", "boolean"]}, "UnionMembers")
        assert Model(v="s").v == "s"
        assert Model(v=True).v is True
        for rejected in (42, None):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_type_array_honours_a_non_leading_member(self, tmp_path: Path) -> None:
        """Only the first non-null member used to survive, so the rest were unenforced."""
        Model = self._model(tmp_path, {"type": ["boolean", "string"]}, "UnionTrailing")
        assert Model(v="auto").v == "auto"
        assert Model(v=True).v is True

    def test_type_array_with_object_member_does_not_widen_to_any(self, tmp_path: Path) -> None:
        """An object/array member used to fall through to Any, accepting every value."""
        Model = self._model(
            tmp_path,
            {"type": ["object", "null"], "properties": {"a": {"type": "string"}}},
            "UnionObject",
        )
        assert Model(v=None).v is None
        for rejected in (42, "str"):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_type_array_keeps_option_keywords_per_branch(self, tmp_path: Path) -> None:
        """A numeric bound must not be applied to the string branch, nor vice versa."""
        Model = self._model(
            tmp_path,
            {"type": ["string", "integer"], "minLength": 3, "minimum": 10},
            "UnionConstraints",
        )
        assert Model(v="abc").v == "abc"
        assert Model(v=42).v == 42
        for rejected in ("ab", 5):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_enum_alongside_a_type_array_is_enforced(self, tmp_path: Path) -> None:
        """What apexe emits for `ls --color[=WHEN]`.

        Note the old implementation enforced the enum here too — it won the
        dispatch outright and `type` was the half being dropped. The direction
        that regressed is covered by `test_the_type_half_of_a_type_array_holds`.
        """
        Model = self._model(
            tmp_path,
            {"type": ["string", "boolean"], "enum": ["always", "auto", "never"]},
            "UnionEnum",
        )
        assert Model(v="auto").v == "auto"
        for rejected in ("bogus-not-in-enum", True):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_the_type_half_of_a_type_array_holds(self, tmp_path: Path) -> None:
        """An enum member outside the declared types is rejected by the type half.

        The old implementation built `Literal["a", 1]` and dropped `type`, so `1`
        was accepted even though the schema declares only string and boolean.
        """
        Model = self._model(tmp_path, {"type": ["string", "boolean"], "enum": ["a", 1]}, "UnionEnumType")
        assert Model(v="a").v == "a"
        with pytest.raises(ValidationError):
            Model(v=1)

    def test_enum_alongside_a_scalar_type_is_enforced(self, tmp_path: Path) -> None:
        Model = self._model(tmp_path, {"type": "string", "enum": ["a", "b"]}, "ScalarEnum")
        assert Model(v="a").v == "a"
        for rejected in ("zzz", 5):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_the_type_half_of_a_scalar_type_holds(self, tmp_path: Path) -> None:
        """Same as above for a scalar `type`: `Literal["a", 1]` used to accept `1`."""
        Model = self._model(tmp_path, {"type": "string", "enum": ["a", 1]}, "ScalarEnumType")
        assert Model(v="a").v == "a"
        with pytest.raises(ValidationError):
            Model(v=1)

    def test_type_is_not_discarded_by_a_combinator_sibling(self, tmp_path: Path) -> None:
        """The mirror defect: the combinator used to win and `type` was dropped."""
        Model = self._model(tmp_path, {"type": "string", "anyOf": [{"minLength": 3}]}, "TypeWithAnyOf")
        assert Model(v="abcd").v == "abcd"
        for rejected in ("ab", 12345, {"a": 1}):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_const_alongside_a_type_is_enforced(self, tmp_path: Path) -> None:
        Model = self._model(tmp_path, {"type": "string", "const": "fixed"}, "TypeWithConst")
        assert Model(v="fixed").v == "fixed"
        with pytest.raises(ValidationError):
            Model(v="other")

    def test_constraints_survive_on_a_required_field(self, tmp_path: Path) -> None:
        """`minLength` was kept for optional fields but dropped for required ones."""
        schema = {"type": "string", "minLength": 5, "enum": ["ab", "abcdef"]}
        Model = self._model(tmp_path, schema, "RequiredConstraints")
        assert Model(v="abcdef").v == "abcdef"
        with pytest.raises(ValidationError):
            Model(v="ab")

    def test_constraints_agree_between_required_and_optional(self, tmp_path: Path) -> None:
        schema = {"type": "string", "minLength": 5, "enum": ["ab", "abcdef"]}
        Optional = make_loader(tmp_path).generate_model(
            {"type": "object", "properties": {"v": schema}}, "OptionalConstraints"
        )
        with pytest.raises(ValidationError):
            Optional(v="ab")


class TestAdditionalPropertiesObjectForm:
    """`additionalProperties` as a sub-schema constrains undeclared keys."""

    def test_object_form_constrains_undeclared_keys(self, tmp_path: Path) -> None:
        Model = make_loader(tmp_path).generate_model(
            {
                "type": "object",
                "properties": {"a": {"type": "string"}},
                "additionalProperties": {"type": "integer"},
                "required": ["a"],
            },
            "AdditionalObjectForm",
        )
        assert Model(a="x", zzz=7).zzz == 7
        with pytest.raises(ValidationError):
            Model(a="x", zzz="not-an-integer")

    def test_false_still_forbids_undeclared_keys(self, tmp_path: Path) -> None:
        Model = make_loader(tmp_path).generate_model(
            {
                "type": "object",
                "properties": {"a": {"type": "string"}},
                "additionalProperties": False,
                "required": ["a"],
            },
            "AdditionalFalse",
        )
        assert Model(a="x").a == "x"
        with pytest.raises(ValidationError):
            Model(a="x", zzz=1)


class TestAnnotationsArePreserved:
    """`description` / `title` reach the generated field instead of being dropped."""

    def test_description_and_title_on_a_scalar_type(self, tmp_path: Path) -> None:
        Model = make_loader(tmp_path).generate_model(
            {
                "type": "object",
                "properties": {"v": {"type": "string", "description": "DESC", "title": "TITLE"}},
                "required": ["v"],
            },
            "ScalarAnnotations",
        )
        assert Model.model_fields["v"].description == "DESC"
        assert Model.model_fields["v"].title == "TITLE"

    def test_description_and_title_on_a_type_array(self, tmp_path: Path) -> None:
        Model = make_loader(tmp_path).generate_model(
            {
                "type": "object",
                "properties": {"v": {"type": ["string", "boolean"], "description": "DESC", "title": "TITLE"}},
                "required": ["v"],
            },
            "UnionAnnotations",
        )
        assert Model.model_fields["v"].description == "DESC"
        assert Model.model_fields["v"].title == "TITLE"


class TestTypeArrayBranchConstraints:
    """A `type` array keeps each member's option keywords on its own branch."""

    def _model(self, tmp_path: Path, prop_schema: dict[str, Any], name: str) -> type[BaseModel]:
        return make_loader(tmp_path).generate_model(
            {"type": "object", "properties": {"v": prop_schema}, "required": ["v"]},
            name,
        )

    def test_min_items_survives_a_nullable_array(self, tmp_path: Path) -> None:
        """`minItems` was dropped entirely for `type: ["array", "null"]`."""
        Model = self._model(
            tmp_path,
            {"type": ["array", "null"], "items": {"type": "integer"}, "minItems": 2},
            "NullableArrayMin",
        )
        assert Model(v=[1, 2]).v == [1, 2]
        assert Model(v=None).v is None
        with pytest.raises(ValidationError):
            Model(v=[1])

    def test_max_items_survives_a_nullable_array(self, tmp_path: Path) -> None:
        Model = self._model(
            tmp_path,
            {"type": ["array", "null"], "items": {"type": "integer"}, "maxItems": 3},
            "NullableArrayMax",
        )
        assert Model(v=[1, 2]).v == [1, 2]
        with pytest.raises(ValidationError):
            Model(v=[1, 2, 3, 4, 5])

    def test_an_unknown_type_name_is_rejected_at_build_time(self, tmp_path: Path) -> None:
        """An unrecognised member used to widen the whole union to Any."""
        with pytest.raises(SchemaParseError, match="Unknown type"):
            self._model(tmp_path, {"type": ["string", "bogus"]}, "UnknownMember")

    def test_an_empty_type_array_is_rejected_at_build_time(self, tmp_path: Path) -> None:
        with pytest.raises(SchemaParseError, match="Empty type array"):
            self._model(tmp_path, {"type": []}, "EmptyTypeArray")


class TestBooleanNumberDistinction:
    """JSON Schema treats bool and number as distinct instance types."""

    def _model(self, tmp_path: Path, prop_schema: dict[str, Any], name: str) -> type[BaseModel]:
        return make_loader(tmp_path).generate_model(
            {"type": "object", "properties": {"v": prop_schema}, "required": ["v"]},
            name,
        )

    @pytest.mark.parametrize("declared", ["integer", "number"])
    def test_a_bool_is_not_a_number(self, tmp_path: Path, declared: str) -> None:
        Model = self._model(tmp_path, {"type": declared}, f"Scalar{declared}")
        for rejected in (True, False):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_a_number_is_not_a_bool(self, tmp_path: Path) -> None:
        Model = self._model(tmp_path, {"type": "boolean"}, "ScalarBool")
        assert Model(v=True).v is True
        for rejected in (1, 0, "true"):
            with pytest.raises(ValidationError):
                Model(v=rejected)

    def test_a_bool_does_not_slip_into_a_numeric_union_branch(self, tmp_path: Path) -> None:
        """Pydantic's lax mode read `True` as `1`, which the other SDKs reject."""
        Model = self._model(tmp_path, {"type": ["string", "integer"]}, "UnionNoBool")
        assert Model(v=42).v == 42
        assert Model(v="s").v == "s"
        with pytest.raises(ValidationError):
            Model(v=True)

    def test_a_boolean_branch_still_claims_a_bool(self, tmp_path: Path) -> None:
        Model = self._model(tmp_path, {"type": ["boolean", "integer"]}, "UnionWithBool")
        assert Model(v=True).v is True
        assert Model(v=42).v == 42

    def test_numeric_coercion_is_otherwise_unchanged(self, tmp_path: Path) -> None:
        Model = self._model(tmp_path, {"type": "integer"}, "CoerceInt")
        assert Model(v="42").v == 42
        assert Model(v=42.0).v == 42


class TestAdditionalPropertiesSubSchema:
    """The `additionalProperties` sub-schema is enforced in every form."""

    def _model(self, tmp_path: Path, additional: Any, name: str) -> type[BaseModel]:
        return make_loader(tmp_path).generate_model(
            {
                "type": "object",
                "properties": {"a": {"type": "string"}},
                "additionalProperties": additional,
                "required": ["a"],
            },
            name,
        )

    @pytest.mark.parametrize(
        ("additional", "accepted", "rejected"),
        [
            ({"type": "array", "items": {"type": "integer"}}, [1], ["not-an-int"]),
            ({"type": ["string", "null"]}, "s", 12345),
            ({"enum": ["a", "b"]}, "a", "zzz"),
            ({"minimum": 3}, 5, 1),
            ({"type": "object", "properties": {"n": {"type": "integer"}}}, {"n": 1}, {"n": "x"}),
        ],
        ids=["array", "type-union", "bare-enum", "bare-constraint", "nested-object"],
    )
    def test_a_non_scalar_sub_schema_is_enforced(
        self, tmp_path: Path, additional: Any, accepted: Any, rejected: Any
    ) -> None:
        """A shallow type mapping used to widen all of these to Any."""
        Model = self._model(tmp_path, additional, f"Extra{abs(hash(str(additional)))}")
        assert Model(a="x", zzz=accepted).zzz == accepted
        with pytest.raises(ValidationError):
            Model(a="x", zzz=rejected)

    @pytest.mark.parametrize("additional", [True, {}], ids=["true", "empty-object"])
    def test_an_always_true_form_keeps_undeclared_keys(self, tmp_path: Path, additional: Any) -> None:
        """`true` and `{}` are the same assertion and must behave alike."""
        Model = self._model(tmp_path, additional, f"Always{additional}")
        assert Model(a="x", zzz=1).model_dump() == {"a": "x", "zzz": 1}


class TestMalformedSubSchemaFailsFast:
    """A malformed combinator fails when the module is built, not on first call."""

    @pytest.mark.parametrize(
        "prop_schema",
        [
            {"type": "string", "not": "garbage"},
            {"type": "string", "oneOf": "garbage"},
            {"type": "string", "enum": "not-a-list"},
            {"type": "string", "anyOf": [{"minLength": "not-a-number"}]},
        ],
    )
    def test_a_malformed_combinator_raises_schema_parse_error(self, tmp_path: Path, prop_schema: Any) -> None:
        with pytest.raises(SchemaParseError):
            make_loader(tmp_path).generate_model(
                {"type": "object", "properties": {"v": prop_schema}, "required": ["v"]},
                f"Malformed{abs(hash(str(prop_schema)))}",
            )


class TestAllOfSiblingFallback:
    """`allOf` merging is lossy, so the keyword is also asserted in full."""

    def test_a_constraint_lost_by_the_merge_is_still_enforced(self, tmp_path: Path) -> None:
        Model = make_loader(tmp_path).generate_model(
            {
                "type": "object",
                "properties": {
                    "v": {
                        "allOf": [
                            {"properties": {"a": {"type": "string", "minLength": 5}}, "required": ["a"]},
                            {"properties": {"a": {"type": "string"}}},
                        ]
                    }
                },
                "required": ["v"],
            },
            "AllOfLossy",
        )
        assert Model(v={"a": "abcdef"}).v.a == "abcdef"
        with pytest.raises(ValidationError):
            Model(v={"a": "ab"})


class TestCombinatorErrorDetail:
    """A combinator failure reports the keyword that failed, not a generic "value"."""

    def test_the_failing_keyword_reaches_the_error_detail(self, tmp_path: Path) -> None:
        from apcore.schema.validator import SchemaValidator

        Model = make_loader(tmp_path).generate_model(
            {"type": "object", "properties": {"v": {"type": "string", "enum": ["a", "b"]}}, "required": ["v"]},
            "EnumDetail",
        )
        result = SchemaValidator().validate({"v": "zzz"}, Model)
        assert result.valid is False
        assert result.errors[0].constraint == "enum"
        assert result.errors[0].expected == ["a", "b"]


class TestObjectPropertyCountConstraints:
    """`minProperties`/`maxProperties` (§6.5.1/§6.5.2) reach the generated model.

    Both keywords were dropped wholesale by the converter: `_STRING_CONSTRAINTS`
    and friends map a keyword to a Pydantic `Field` argument, and no Field
    argument counts an object's members, so there was nowhere for them to land.
    apcore-typescript enforces them via `OBJECT_CONSTRAINTS` and apcore-rust via
    the jsonschema crate, which made Python the odd one out. The counts below all
    match `jsonschema.Draft202012Validator` on the same schema and input.
    """

    def _model(self, tmp_path: Path, schema: dict[str, Any], name: str) -> type[BaseModel]:
        return make_loader(tmp_path).generate_model(schema, name)

    def _prop_model(self, tmp_path: Path, prop_schema: dict[str, Any], name: str) -> type[BaseModel]:
        return self._model(
            tmp_path,
            {"type": "object", "properties": {"v": prop_schema}, "required": ["v"]},
            name,
        )

    def test_min_properties_on_the_top_level_schema(self, tmp_path: Path) -> None:
        Model = self._model(
            tmp_path,
            {"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "string"}}, "minProperties": 2},
            "TopLevelMinProps",
        )
        assert Model.model_validate({"a": "x", "b": "y"}).a == "x"
        with pytest.raises(ValidationError):
            Model.model_validate({"a": "x"})

    def test_max_properties_on_the_top_level_schema(self, tmp_path: Path) -> None:
        Model = self._model(
            tmp_path,
            {"type": "object", "properties": {"a": {"type": "string"}}, "maxProperties": 1},
            "TopLevelMaxProps",
        )
        assert Model.model_validate({"a": "x"}).a == "x"
        with pytest.raises(ValidationError):
            Model.model_validate({"a": "x", "extra": 1})

    def test_min_properties_on_a_nested_object_property(self, tmp_path: Path) -> None:
        """The conformance case: an empty object must fail `minProperties: 1`."""
        Model = self._prop_model(
            tmp_path,
            {"type": "object", "minProperties": 1, "properties": {"a": {"type": "string"}}},
            "NestedMinProps",
        )
        assert Model.model_validate({"v": {"a": "x"}}).v is not None
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {}})

    def test_max_properties_on_a_nested_object_property(self, tmp_path: Path) -> None:
        Model = self._prop_model(
            tmp_path,
            {"type": "object", "maxProperties": 1, "properties": {"a": {"type": "string"}}},
            "NestedMaxProps",
        )
        assert Model.model_validate({"v": {"a": "x"}}).v is not None
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"a": "x", "extra": 1}})

    def test_undeclared_keys_count_towards_the_minimum(self, tmp_path: Path) -> None:
        """`additionalProperties` other than `false` keeps extras, so they count."""
        Model = self._model(
            tmp_path,
            {"type": "object", "properties": {"a": {"type": "string"}}, "minProperties": 2},
            "ExtraCountsTowardsMin",
        )
        assert Model.model_validate({"a": "x", "zzz": 1}).a == "x"
        with pytest.raises(ValidationError):
            Model.model_validate({"a": "x"})

    def test_an_omitted_optional_field_is_not_counted(self, tmp_path: Path) -> None:
        """The count comes from the input, not from the model's field list."""
        Model = self._model(
            tmp_path,
            {
                "type": "object",
                "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
                "minProperties": 2,
                "additionalProperties": False,
            },
            "OmittedOptionalNotCounted",
        )
        with pytest.raises(ValidationError):
            Model.model_validate({"a": "x"})

    def test_min_properties_on_an_object_without_declared_properties(self, tmp_path: Path) -> None:
        """An open map never reaches `create_model`, so it needs its own check."""
        Model = self._prop_model(tmp_path, {"type": "object", "minProperties": 2}, "OpenMapMinProps")
        assert Model.model_validate({"v": {"a": 1, "b": 2}}).v == {"a": 1, "b": 2}
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"a": 1}})

    def test_min_properties_on_an_additional_properties_map(self, tmp_path: Path) -> None:
        Model = self._prop_model(
            tmp_path,
            {"type": "object", "minProperties": 2, "additionalProperties": {"type": "integer"}},
            "TypedMapMinProps",
        )
        assert Model.model_validate({"v": {"a": 1, "b": 2}}).v == {"a": 1, "b": 2}
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {"a": 1}})

    def test_the_object_branch_of_a_type_array_carries_the_constraint(self, tmp_path: Path) -> None:
        """The constraint applies to the object branch only; `null` is untouched."""
        Model = self._prop_model(
            tmp_path,
            {"type": ["object", "null"], "minProperties": 1},
            "NullableOpenMapMinProps",
        )
        assert Model.model_validate({"v": {"a": 1}}).v == {"a": 1}
        assert Model.model_validate({"v": None}).v is None
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {}})

    def test_a_type_array_object_branch_with_properties_keeps_the_constraint(self, tmp_path: Path) -> None:
        Model = self._prop_model(
            tmp_path,
            {"type": ["object", "null"], "minProperties": 1, "properties": {"a": {"type": "string"}}},
            "NullableModelMinProps",
        )
        assert Model.model_validate({"v": {"a": "x"}}).v is not None
        assert Model.model_validate({"v": None}).v is None
        with pytest.raises(ValidationError):
            Model.model_validate({"v": {}})


class TestArrayContainsConstraints:
    """`contains` and its §6.4.4/§6.4.5 counts reach the generated model.

    `minContains`/`maxContains` were dropped by the converter, so an array that
    jsonschema (apcore-rust) and TypeBox (apcore-typescript) both reject was
    accepted here. Bare, without `contains`, the two still assert nothing.
    """

    def _model(self, tmp_path: Path, prop_schema: dict[str, Any], name: str) -> type[BaseModel]:
        return make_loader(tmp_path).generate_model(
            {"type": "object", "properties": {"v": prop_schema}, "required": ["v"]},
            name,
        )

    def test_contains_requires_a_matching_item(self, tmp_path: Path) -> None:
        Model = self._model(
            tmp_path,
            {"type": "array", "items": {"type": "integer"}, "contains": {"const": 1}},
            "ArrayContains",
        )
        assert Model.model_validate({"v": [3, 1]}).v == [3, 1]
        with pytest.raises(ValidationError):
            Model.model_validate({"v": [2, 3]})

    def test_min_contains_counts_matching_items(self, tmp_path: Path) -> None:
        Model = self._model(
            tmp_path,
            {"type": "array", "items": {"type": "integer"}, "contains": {"const": 1}, "minContains": 2},
            "ArrayMinContains",
        )
        assert Model.model_validate({"v": [1, 1]}).v == [1, 1]
        with pytest.raises(ValidationError):
            Model.model_validate({"v": [1, 2]})

    def test_max_contains_counts_matching_items(self, tmp_path: Path) -> None:
        Model = self._model(
            tmp_path,
            {"type": "array", "items": {"type": "integer"}, "contains": {"const": 1}, "maxContains": 1},
            "ArrayMaxContains",
        )
        assert Model.model_validate({"v": [1, 2]}).v == [1, 2]
        with pytest.raises(ValidationError):
            Model.model_validate({"v": [1, 1]})

    def test_min_contains_without_contains_asserts_nothing(self, tmp_path: Path) -> None:
        """Emitting it bare would turn this into a schema that rejects every array."""
        Model = self._model(
            tmp_path,
            {"type": "array", "items": {"type": "integer"}, "minContains": 5},
            "ArrayBareMinContains",
        )
        assert Model.model_validate({"v": [1]}).v == [1]

    def test_contains_applies_to_the_array_branch_of_a_type_array(self, tmp_path: Path) -> None:
        Model = self._model(
            tmp_path,
            {"type": ["array", "null"], "items": {"type": "integer"}, "contains": {"const": 1}},
            "NullableArrayContains",
        )
        assert Model.model_validate({"v": [1]}).v == [1]
        assert Model.model_validate({"v": None}).v is None
        with pytest.raises(ValidationError):
            Model.model_validate({"v": [2]})


# ---------------------------------------------------------------------------
# Recursive schemas, nested unions and non-scalar uniqueItems
# (PROTOCOL_SPEC §4.15, JSON Schema 2020-12 §6.4.3 / §10.2)
# ---------------------------------------------------------------------------


def _module_model(schema: dict[str, Any], name: str = "Probe") -> type[BaseModel]:
    """Compile *schema* the way registering a module does: RefResolver + generate_model."""
    from apcore.schema.types import SchemaDefinition

    loader = SchemaLoader(Config({}))
    definition = SchemaDefinition(
        module_id=name,
        description="probe",
        input_schema=schema,
        output_schema={},
    )
    return loader.resolve(definition)[0].model


def _is_valid(schema: dict[str, Any], data: Any, name: str = "Probe") -> bool:
    try:
        _module_model(schema, name).model_validate(data)
        return True
    except ValidationError:
        return False


class TestRecursiveSchema:
    """A self-referencing schema must survive the whole module-loading path."""

    TREE: dict[str, Any] = {
        "$id": "TreeNode",
        "type": "object",
        "required": ["value"],
        "properties": {
            "value": {"type": "string"},
            "children": {"type": "array", "items": {"$ref": "#"}},
        },
    }

    def test_recursive_schema_registers_and_validates_at_depth(self) -> None:
        # Regression: this raised SchemaCircularRefError out of RefResolver, so a
        # TreeNode module could not be registered at all.
        assert _is_valid(self.TREE, {"value": "root"}, "Tree1")
        assert _is_valid(
            self.TREE,
            {"value": "root", "children": [{"value": "child", "children": [{"value": "leaf"}]}]},
            "Tree2",
        )

    def test_recursive_position_still_asserts(self) -> None:
        # Widening an unresolved `$ref` to `Any` would leave the whole sub-tree
        # unchecked, which is a quieter but worse failure than rejecting it.
        assert not _is_valid(self.TREE, {"value": "root", "children": [{"value": 42}]}, "Tree3")
        assert not _is_valid(self.TREE, {"value": "root", "children": [{}]}, "Tree4")

    def test_recursion_anchored_on_a_defs_entry(self) -> None:
        schema: dict[str, Any] = {
            "type": "object",
            "required": ["root"],
            "properties": {"root": {"$ref": "#/$defs/Node"}},
            "$defs": {
                "Node": {
                    "type": "object",
                    "required": ["value"],
                    "properties": {
                        "value": {"type": "string"},
                        "children": {"type": "array", "items": {"$ref": "#/$defs/Node"}},
                    },
                }
            },
        }
        assert _is_valid(schema, {"root": {"value": "a", "children": [{"value": "b"}]}}, "Defs1")
        assert not _is_valid(schema, {"root": {"value": "a", "children": [{"value": 9}]}}, "Defs2")


class TestCombinatorsOutsideProperties:
    """§10.2 combinators must hold wherever they appear, on every call path."""

    def test_root_level_one_of_exclusivity_is_enforced_by_the_model(self) -> None:
        # Regression: a root-level union produced a field-less `extra="allow"`
        # model that accepted anything, so the exclusivity rule only existed in
        # `SchemaValidator._validate_top_level_union` — and `BuiltinInputValidation`
        # calls `model_validate()` directly, bypassing it.
        schema = {
            "oneOf": [
                {"type": "object", "properties": {"k": {"type": "string"}}},
                {"type": "object", "properties": {"k": {"type": "string"}}},
            ]
        }
        assert not _is_valid(schema, {"k": "x"}, "RootOneOf")

    def test_root_level_any_of_requires_a_matching_branch(self) -> None:
        schema = {"anyOf": [{"required": ["k"]}, {"required": ["j"]}]}
        assert not _is_valid(schema, {"z": 1}, "RootAnyOf1")
        assert _is_valid(schema, {"k": 1}, "RootAnyOf2")

    def test_one_of_inside_array_items_keeps_its_exclusivity(self) -> None:
        # `_schema_to_type` derived the element *shape* only, so a combinator on
        # `items` was dropped and every element widened to `Any`.
        schema = {
            "type": "object",
            "required": ["a"],
            "properties": {"a": {"type": "array", "items": {"oneOf": [{"type": "integer"}, {"type": "number"}]}}},
        }
        assert not _is_valid(schema, {"a": [3]}, "ItemOneOf1")
        assert _is_valid(schema, {"a": [3.5]}, "ItemOneOf2")


class TestUniqueItemsOverNonScalars:
    """`uniqueItems` must fail validation, not raise, on unhashable members."""

    SCHEMA: dict[str, Any] = {
        "type": "object",
        "required": ["a"],
        "properties": {
            "a": {
                "type": "array",
                "uniqueItems": True,
                "items": {
                    "type": "object",
                    "properties": {"k": {"type": "string"}, "j": {"type": "integer"}},
                },
            }
        },
    }

    def test_duplicate_objects_fail_validation_without_raising_type_error(self) -> None:
        # Regression: `len(v) != len(set(v))` raised a bare TypeError on a list of
        # dicts. Pydantic only converts ValueError/AssertionError, so the
        # TypeError escaped the module call instead of becoming
        # SCHEMA_VALIDATION_ERROR.
        model = _module_model(self.SCHEMA, "Unique1")
        with pytest.raises(ValidationError):
            model.model_validate({"a": [{"k": "x"}, {"k": "x"}]})

    def test_distinct_objects_are_accepted(self) -> None:
        assert _is_valid(self.SCHEMA, {"a": [{"k": "x"}, {"k": "y"}]}, "Unique2")

    def test_key_order_does_not_make_two_members_distinct(self) -> None:
        assert not _is_valid(self.SCHEMA, {"a": [{"k": "x", "j": 1}, {"j": 1, "k": "x"}]}, "Unique3")

    def test_nested_arrays_are_compared_by_value(self) -> None:
        schema: dict[str, Any] = {
            "type": "object",
            "required": ["a"],
            "properties": {
                "a": {
                    "type": "array",
                    "uniqueItems": True,
                    "items": {"type": "array", "items": {"type": "integer"}},
                }
            },
        }
        assert not _is_valid(schema, {"a": [[1, 2], [1, 2]]}, "Unique4")
        assert _is_valid(schema, {"a": [[1, 2], [2, 1]]}, "Unique5")


class TestTypelessSubSchemaKeywords:
    """§6 / §10.3 keywords at a position that declares no `type`.

    Two halves of one defect, both cross-language (apcore-typescript and
    apcore-rust already behave as asserted here):

    * The keywords were asserted by *nothing*. `_base_annotation` widened a
      type-less sub-schema to `Any` and only the eight `_APPLICATOR_KEYWORDS`
      were delegated onward, so `required`, `items`, `contains`, `minItems`,
      `uniqueItems`, `minProperties`, `additionalProperties` and friends
      vanished (TYPE_MAPPING §17.1 R1, "no silent drop").
    * The §6 option keywords that *were* collected became field-wide Pydantic
      constraints on an `Any` annotation, which made them apply to every
      instance type instead of only their own (§17.1 R2, "inertness"), and the
      numeric/pattern variants escaped as a bare `TypeError` that
      `BuiltinInputValidation` does not catch.
    """

    @staticmethod
    def _wrap(sub: dict[str, Any]) -> dict[str, Any]:
        return {"type": "object", "properties": {"v": sub}, "required": ["v"]}

    # ── R1: the keyword is enforced ────────────────────────────────────────
    @pytest.mark.parametrize(
        ("case", "sub", "value"),
        [
            ("required", {"required": ["b"]}, {"a": 1}),
            ("min_items", {"minItems": 3}, [1]),
            ("max_items", {"maxItems": 1}, [1, 2]),
            ("unique_items", {"uniqueItems": True}, [1, 1]),
            ("contains", {"contains": {"type": "string"}}, [1, 2]),
            ("min_contains", {"contains": {"type": "string"}, "minContains": 2}, ["a", 1]),
            ("items", {"items": {"type": "string"}}, [1]),
            ("min_properties", {"minProperties": 2}, {"a": 1}),
            ("max_properties", {"maxProperties": 1}, {"a": 1, "b": 2}),
            ("additional_properties", {"additionalProperties": False}, {"a": 1}),
            ("properties", {"properties": {"a": {"type": "string"}}}, {"a": 1}),
            ("min_length", {"minLength": 3}, "ab"),
            ("max_length", {"maxLength": 1}, "ab"),
            ("pattern", {"pattern": "^a"}, "zzz"),
            ("minimum", {"minimum": 3}, 1),
            ("maximum", {"maximum": 3}, 9),
            ("exclusive_minimum", {"exclusiveMinimum": 3}, 3),
            ("multiple_of", {"multipleOf": 3}, 4),
        ],
    )
    def test_bare_keyword_rejects_a_violating_instance(self, case: str, sub: dict[str, Any], value: Any) -> None:
        assert not _is_valid(self._wrap(sub), {"v": value}, f"BareReject_{case}")

    @pytest.mark.parametrize(
        ("case", "sub", "value"),
        [
            ("required", {"required": ["b"]}, {"b": 1}),
            ("min_items", {"minItems": 3}, [1, 2, 3]),
            ("unique_items", {"uniqueItems": True}, [1, 2]),
            ("contains", {"contains": {"type": "string"}}, [1, "s"]),
            ("items", {"items": {"type": "string"}}, ["a"]),
            ("min_properties", {"minProperties": 2}, {"a": 1, "b": 2}),
            ("additional_properties", {"additionalProperties": False}, {}),
            ("properties", {"properties": {"a": {"type": "string"}}}, {"a": "s"}),
            ("min_length", {"minLength": 3}, "abc"),
            ("pattern", {"pattern": "^a"}, "abc"),
            ("minimum", {"minimum": 3}, 5),
            ("multiple_of", {"multipleOf": 3}, 9),
        ],
    )
    def test_bare_keyword_accepts_a_satisfying_instance(self, case: str, sub: dict[str, Any], value: Any) -> None:
        assert _is_valid(self._wrap(sub), {"v": value}, f"BareAccept_{case}")

    # ── R2: the keyword is inert on every other instance type ──────────────
    @pytest.mark.parametrize(
        ("case", "sub", "value"),
        [
            ("min_length_on_array", {"minLength": 3}, [1]),
            ("min_length_on_object", {"minLength": 3}, {"a": 1}),
            ("min_length_on_number", {"minLength": 3}, 1),
            ("max_length_on_array", {"maxLength": 1}, [1, 2, 3]),
            ("pattern_on_integer", {"pattern": "^a"}, 5),
            ("pattern_on_array", {"pattern": "^a"}, [1]),
            ("pattern_on_null", {"pattern": "^a"}, None),
            ("minimum_on_string", {"minimum": 3}, "abc"),
            ("minimum_on_array", {"minimum": 3}, [1]),
            ("maximum_on_string", {"maximum": 3}, "abc"),
            ("multiple_of_on_string", {"multipleOf": 3}, "abc"),
            ("min_items_on_string", {"minItems": 3}, "a"),
            ("min_items_on_object", {"minItems": 3}, {"a": 1}),
            ("unique_items_on_string", {"uniqueItems": True}, "aa"),
            ("min_properties_on_array", {"minProperties": 2}, [1]),
            ("required_on_string", {"required": ["b"]}, "x"),
            ("required_on_null", {"required": ["b"]}, None),
            ("items_on_string", {"items": {"type": "string"}}, "x"),
            ("contains_on_object", {"contains": {"type": "string"}}, {"a": 1}),
            ("additional_properties_on_array", {"additionalProperties": False}, [1, 2]),
            ("properties_on_string", {"properties": {"a": {"type": "string"}}}, "x"),
        ],
    )
    def test_bare_keyword_is_inert_on_another_instance_type(self, case: str, sub: dict[str, Any], value: Any) -> None:
        # Not `_is_valid`: `minimum` and `pattern` used to raise a *bare*
        # TypeError out of pydantic's `apply_known_metadata` fallback, which
        # `BuiltinInputValidation` does not catch (it catches only
        # pydantic.ValidationError), so the module call died with an uncoded
        # TypeError rather than a validation error.
        model = _module_model(self._wrap(sub), f"BareInert_{case}")
        model.model_validate({"v": value})

    # ── The two spellings §17.2 calls out by name ──────────────────────────
    def test_bare_required_sub_schema_is_a_complete_schema(self) -> None:
        # TYPE_MAPPING §17.2, `required`: "A name listed here without a
        # `properties` entry is still required: `{"required": ["b"]}` is a
        # complete schema". apcore-typescript and apcore-rust both reject.
        schema = self._wrap({"required": ["b"]})
        assert not _is_valid(schema, {"v": {"a": 1}}, "BareRequiredSpelling1")
        assert _is_valid(schema, {"v": {"a": 1, "b": 2}}, "BareRequiredSpelling2")

    def test_bare_keywords_survive_beside_a_combinator(self) -> None:
        # `not` also lands on the type-less `Any` branch; the §6 keyword beside
        # it must still be asserted.
        schema = self._wrap({"not": {"type": "boolean"}, "minLength": 3})
        assert not _is_valid(schema, {"v": "ab"}, "BareBesideNot1")
        assert _is_valid(schema, {"v": "abc"}, "BareBesideNot2")

    def test_bare_keywords_apply_inside_array_items(self) -> None:
        # §17.3: the rules hold "inside `items` ... exactly as at the top level".
        schema: dict[str, Any] = {
            "type": "object",
            "required": ["a"],
            "properties": {"a": {"type": "array", "items": {"required": ["b"]}}},
        }
        assert not _is_valid(schema, {"a": [{"x": 1}]}, "BareInItems1")
        assert _is_valid(schema, {"a": [{"b": 1}]}, "BareInItems2")
        # Still inert on a non-object element.
        assert _is_valid(schema, {"a": ["str"]}, "BareInItems3")

    def test_pattern_properties_companions_are_not_asserted_twice(self) -> None:
        # `_applicator_assertion_schema` already carries `properties` and
        # `additionalProperties` alongside `patternProperties` (§10.3.2.3
        # exempts every pattern-matched key). Re-asserting
        # `additionalProperties: false` on its own would reject exactly those
        # keys, so the bare delegation must skip what the applicator took.
        schema = self._wrap(
            {
                "patternProperties": {"^s_": {"type": "string"}},
                "additionalProperties": False,
            }
        )
        assert _is_valid(schema, {"v": {"s_a": "x"}}, "PatternCompanion1")
        assert not _is_valid(schema, {"v": {"other": 1}}, "PatternCompanion2")

    def test_unevaluated_properties_still_sees_its_whole_schema(self) -> None:
        schema = self._wrap(
            {
                "allOf": [{"properties": {"a": {"type": "string"}}}],
                "unevaluatedProperties": False,
            }
        )
        assert _is_valid(schema, {"v": {"a": "x"}}, "Unevaluated1")
        assert not _is_valid(schema, {"v": {"a": "x", "b": 1}}, "Unevaluated2")


class TestAllOfWithNonObjectMembers:
    """§17.1 R1 licenses a load-time rejection only when the keyword *cannot* be
    enforced — and Python can enforce this one.

    ``_handle_all_of`` raised ``SchemaParseError`` at model-build time for an
    ``allOf`` whose members are not object-shaped, so a contract
    apcore-typescript and apcore-rust both accept *and enforce* was uncallable
    in Python: the module could not even be registered. The unconsumed-keyword
    path already delegates ``allOf`` to jsonschema whenever a ``type`` sibling
    is present, which is exactly the machinery needed here; falling through to
    ``Any`` lets it run, the same way ``not`` already does.
    """

    @pytest.mark.parametrize(
        ("case", "sub"),
        [
            ("scalar_members", {"allOf": [{"type": "string"}, {"minLength": 3}]}),
            ("numeric_bounds", {"allOf": [{"minimum": 1}, {"maximum": 10}]}),
            ("array_members", {"allOf": [{"type": "array"}, {"minItems": 2}]}),
            ("enum_member", {"allOf": [{"enum": ["a", "b"]}, {"type": "string"}]}),
            ("const_member", {"allOf": [{"const": 7}]}),
            ("mixed", {"allOf": [{"type": "string"}, {"properties": {"a": {}}}]}),
        ],
    )
    def test_schema_is_accepted_at_build_time(self, case: str, sub: dict[str, Any]) -> None:
        # Registering the module must not raise.
        _module_model(
            {"type": "object", "properties": {"v": sub}, "required": ["v"]},
            f"AllOfBuild_{case}",
        )

    def test_scalar_all_of_is_enforced_not_merely_accepted(self) -> None:
        schema: dict[str, Any] = {
            "type": "object",
            "required": ["v"],
            "properties": {"v": {"allOf": [{"type": "string"}, {"minLength": 3}]}},
        }
        assert _is_valid(schema, {"v": "abc"}, "AllOfScalar1")
        assert not _is_valid(schema, {"v": "ab"}, "AllOfScalar2")
        assert not _is_valid(schema, {"v": 123}, "AllOfScalar3")

    def test_numeric_all_of_is_enforced(self) -> None:
        schema: dict[str, Any] = {
            "type": "object",
            "required": ["v"],
            "properties": {"v": {"allOf": [{"minimum": 1}, {"maximum": 10}]}},
        }
        assert _is_valid(schema, {"v": 5}, "AllOfNum1")
        assert not _is_valid(schema, {"v": 0}, "AllOfNum2")
        assert not _is_valid(schema, {"v": 11}, "AllOfNum3")
        # Inert on a non-number (§17.1 R2).
        assert _is_valid(schema, {"v": "x"}, "AllOfNum4")

    def test_object_shaped_all_of_still_merges_into_a_model(self) -> None:
        schema: dict[str, Any] = {
            "type": "object",
            "required": ["v"],
            "properties": {
                "v": {
                    "allOf": [
                        {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]},
                        {"type": "object", "properties": {"b": {"type": "integer"}}, "required": ["b"]},
                    ]
                }
            },
        }
        assert _is_valid(schema, {"v": {"a": "x", "b": 1}}, "AllOfObj1")
        assert not _is_valid(schema, {"v": {"a": "x"}}, "AllOfObj2")
        assert not _is_valid(schema, {"v": {"a": 1, "b": 1}}, "AllOfObj3")
