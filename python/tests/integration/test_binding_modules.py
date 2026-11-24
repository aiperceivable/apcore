"""Integration tests for YAML binding-loaded modules through the Executor."""

from __future__ import annotations

import sys
import textwrap

import pytest
import yaml

from apcore.bindings import BindingLoader
from apcore.executor import Executor
from apcore.registry import Registry


@pytest.fixture
def binding_setup(tmp_path):
    """Create a binding YAML file and target Python module for testing."""
    target_dir = tmp_path / "targets"
    target_dir.mkdir()
    (target_dir / "__init__.py").write_text("")
    (target_dir / "math_ops.py").write_text(
        textwrap.dedent(
            """\
        def multiply(a: int, b: int) -> dict:
            \"\"\"Multiply two numbers.\"\"\"
            return {"product": a * b}
    """
        )
    )

    binding_data = {
        "bindings": [
            {
                "module_id": "test.multiply",
                "target": "targets.math_ops:multiply",
                "description": "Multiply two integers",
                "auto_schema": True,
            }
        ]
    }
    binding_path = tmp_path / "test.binding.yaml"
    binding_path.write_text(yaml.dump(binding_data))

    sys.path.insert(0, str(tmp_path))

    registry = Registry(extensions_dir=str(tmp_path))

    yield str(binding_path), registry, target_dir

    sys.path.remove(str(tmp_path))
    sys.modules.pop("targets", None)
    sys.modules.pop("targets.math_ops", None)


class TestBindingModules:
    """Binding module integration tests."""

    def test_yaml_binding_loaded_and_registered(self, binding_setup):
        binding_path, registry, _ = binding_setup
        loader = BindingLoader()
        modules = loader.load_bindings(binding_path, registry)
        assert len(modules) == 1
        assert registry.has("test.multiply")

    def test_bound_module_callable_via_executor(self, binding_setup):
        binding_path, registry, _ = binding_setup
        loader = BindingLoader()
        loader.load_bindings(binding_path, registry)

        executor = Executor(registry=registry)
        result = executor.call("test.multiply", {"a": 6, "b": 7})
        assert result == {"product": 42}

    def test_binding_schemas_applied_correctly(self, binding_setup):
        binding_path, registry, _ = binding_setup
        loader = BindingLoader()
        loader.load_bindings(binding_path, registry)

        mod = registry.get("test.multiply")
        assert mod is not None
        assert mod.input_schema is not None
        assert mod.output_schema is not None
        schema_json = mod.input_schema.model_json_schema()
        assert "a" in schema_json.get("properties", {})
        assert "b" in schema_json.get("properties", {})
