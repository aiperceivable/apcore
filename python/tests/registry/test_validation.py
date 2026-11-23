"""Tests for validate_module() function."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel

from apcore.registry.validation import validate_module


# --- Test helpers ---


class SimpleInput(BaseModel):
    name: str


class SimpleOutput(BaseModel):
    result: str


class ValidModule:
    """A minimal valid module class."""

    input_schema = SimpleInput
    output_schema = SimpleOutput
    description = "A valid module"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"result": "ok"}


# === Tests ===


class TestValidateModule:
    def test_valid_class_returns_empty(self) -> None:
        """Valid Module class returns empty error list."""
        assert validate_module(ValidModule) == []

    def test_valid_instance_returns_empty(self) -> None:
        """Valid Module instance returns empty error list."""
        instance = ValidModule()
        assert validate_module(instance) == []

    def test_plain_class_returns_error(self) -> None:
        """Plain class with no module interface returns errors."""

        class NotAModule:
            pass

        errors = validate_module(NotAModule)
        assert len(errors) > 0

    def test_missing_input_schema(self) -> None:
        """Missing input_schema produces error."""

        class NoInput:
            output_schema = SimpleOutput
            description = "Test"

            def execute(self, inputs: dict, context: Any) -> dict:
                return {}

        errors = validate_module(NoInput)
        assert len(errors) == 1
        assert "input_schema" in errors[0]

    def test_missing_output_schema(self) -> None:
        """Missing output_schema produces error."""

        class NoOutput:
            input_schema = SimpleInput
            description = "Test"

            def execute(self, inputs: dict, context: Any) -> dict:
                return {}

        errors = validate_module(NoOutput)
        assert len(errors) == 1
        assert "output_schema" in errors[0]

    def test_empty_description(self) -> None:
        """Empty description produces error."""

        class EmptyDesc:
            input_schema = SimpleInput
            output_schema = SimpleOutput
            description = ""

            def execute(self, inputs: dict, context: Any) -> dict:
                return {}

        errors = validate_module(EmptyDesc)
        assert len(errors) == 1
        assert "description" in errors[0]

    def test_missing_execute(self) -> None:
        """Missing execute method produces error."""

        class NoExecute:
            input_schema = SimpleInput
            output_schema = SimpleOutput
            description = "Test"

        errors = validate_module(NoExecute)
        assert len(errors) == 1
        assert "execute" in errors[0]

    def test_input_schema_not_basemodel(self) -> None:
        """input_schema set to non-BaseModel class produces error."""

        class BadInput:
            input_schema = dict
            output_schema = SimpleOutput
            description = "Test"

            def execute(self, inputs: dict, context: Any) -> dict:
                return {}

        errors = validate_module(BadInput)
        assert len(errors) == 1
        assert "input_schema" in errors[0]
        assert "BaseModel" in errors[0]

    def test_multiple_errors_accumulated(self) -> None:
        """Class missing all required attributes returns multiple errors."""

        class BrokenModule:
            pass

        errors = validate_module(BrokenModule)
        assert len(errors) >= 3  # input_schema, output_schema, description, execute

    def test_partial_valid_only_failing_reported(self) -> None:
        """Class with some valid and some invalid returns only failing checks."""

        class PartialModule:
            input_schema = SimpleInput
            description = ""

            def execute(self, inputs: dict, context: Any) -> dict:
                return {}

        errors = validate_module(PartialModule)
        assert len(errors) == 2  # output_schema missing, description empty
        error_text = " ".join(errors)
        assert "output_schema" in error_text
        assert "description" in error_text
        assert "input_schema" not in error_text
        assert "execute" not in error_text
