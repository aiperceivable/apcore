"""Tests for YAML binding loader."""

from __future__ import annotations

import sys

import pytest

from apcore.bindings import BindingLoader
from apcore.decorator import FunctionModule
from apcore.errors import (
    BindingCallableNotFoundError,
    BindingFileInvalidError,
    BindingInvalidTargetError,
    BindingModuleNotFoundError,
    BindingNotCallableError,
    BindingSchemaMissingError,
)
from apcore.executor import Executor
from apcore.registry import Registry


@pytest.fixture
def loader() -> BindingLoader:
    return BindingLoader()


@pytest.fixture
def registry() -> Registry:
    return Registry()


# ---------------------------------------------------------------------------
# YAML Parsing Tests
# ---------------------------------------------------------------------------


class TestYamlParsing:
    """Tests for load_bindings YAML file handling."""

    def test_single_binding_parsed(self, loader, registry, tmp_path):
        """Valid YAML with single auto_schema binding parsed correctly."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n  - module_id: email.send\n    target: binding_helpers:typed_function\n    auto_schema: true\n"
        )
        result = loader.load_bindings(str(f), registry)
        assert len(result) == 1
        assert isinstance(result[0], FunctionModule)

    def test_resources_budget_is_carried_onto_the_module(self, loader, registry, tmp_path):
        """A binding can declare a per-module execution budget (D-11)."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: email.send\n"
            "    target: binding_helpers:typed_function\n"
            "    auto_schema: true\n"
            "    resources:\n"
            "      timeout: 1234\n"
        )
        result = loader.load_bindings(str(f), registry)
        assert len(result) == 1
        assert result[0].resources == {"timeout": 1234}

    def test_multiple_bindings_parsed(self, loader, registry, tmp_path):
        """Valid YAML with two binding entries."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: func.one\n"
            "    target: binding_helpers:typed_function\n"
            "    auto_schema: true\n"
            "  - module_id: func.two\n"
            "    target: binding_helpers:typed_function\n"
            "    auto_schema: true\n"
        )
        result = loader.load_bindings(str(f), registry)
        assert len(result) == 2

    def test_empty_file_raises(self, loader, registry, tmp_path):
        """Empty file raises BindingFileInvalidError."""
        f = tmp_path / "empty.yaml"
        f.write_text("")
        with pytest.raises(BindingFileInvalidError):
            loader.load_bindings(str(f), registry)

    def test_missing_bindings_key_raises(self, loader, registry, tmp_path):
        """Missing 'bindings' key raises BindingFileInvalidError."""
        f = tmp_path / "bad.yaml"
        f.write_text("modules:\n  - id: test\n")
        with pytest.raises(BindingFileInvalidError):
            loader.load_bindings(str(f), registry)

    def test_bindings_not_list_raises(self, loader, registry, tmp_path):
        """'bindings' not a list raises BindingFileInvalidError."""
        f = tmp_path / "bad.yaml"
        f.write_text("bindings: not_a_list\n")
        with pytest.raises(BindingFileInvalidError):
            loader.load_bindings(str(f), registry)

    def test_missing_module_id_raises(self, loader, registry, tmp_path):
        """Missing module_id in binding entry raises BindingFileInvalidError."""
        f = tmp_path / "bad.yaml"
        f.write_text("bindings:\n  - target: binding_helpers:typed_function\n    auto_schema: true\n")
        with pytest.raises(BindingFileInvalidError):
            loader.load_bindings(str(f), registry)

    def test_missing_target_raises(self, loader, registry, tmp_path):
        """Missing target in binding entry raises BindingFileInvalidError."""
        f = tmp_path / "bad.yaml"
        f.write_text("bindings:\n  - module_id: test.func\n    auto_schema: true\n")
        with pytest.raises(BindingFileInvalidError):
            loader.load_bindings(str(f), registry)

    def test_yaml_syntax_error_raises(self, loader, registry, tmp_path):
        """YAML syntax error raises BindingFileInvalidError."""
        f = tmp_path / "bad.yaml"
        f.write_text("bindings:\n  - module_id: [\n")
        with pytest.raises(BindingFileInvalidError):
            loader.load_bindings(str(f), registry)


# ---------------------------------------------------------------------------
# Target Resolution Tests
# ---------------------------------------------------------------------------


class TestTargetResolution:
    """Tests for resolve_target method."""

    def test_function_resolves(self, loader):
        """'module.path:function_name' resolves to function."""
        result = loader.resolve_target("os.path:join")
        import os.path

        assert result is os.path.join

    def test_class_method_resolves(self, loader):
        """'module:ClassName.method' resolves to bound method."""
        result = loader.resolve_target("binding_helpers:SimpleService.greet")
        assert callable(result)
        assert result("World") == "Hello, World"

    def test_class_auto_instantiated(self, loader):
        """Class with no-arg constructor is auto-instantiated."""
        result = loader.resolve_target("binding_helpers:SimpleService.greet")
        # Result should be a bound method (not unbound)
        assert callable(result)

    def test_class_requiring_args_raises(self, loader):
        """Class requiring constructor args raises BindingCallableNotFoundError."""
        with pytest.raises(BindingCallableNotFoundError):
            loader.resolve_target("binding_helpers:ComplexService.call")

    def test_missing_colon_raises(self, loader):
        """Missing ':' separator raises BindingInvalidTargetError."""
        with pytest.raises(BindingInvalidTargetError):
            loader.resolve_target("some.module.no_colon")

    def test_nonexistent_module_raises(self, loader):
        """Non-existent module raises BindingModuleNotFoundError."""
        with pytest.raises(BindingModuleNotFoundError):
            loader.resolve_target("nonexistent.module.xyz:func")

    def test_nonexistent_callable_raises(self, loader):
        """Non-existent callable raises BindingCallableNotFoundError."""
        with pytest.raises(BindingCallableNotFoundError):
            loader.resolve_target("os.path:nonexistent_function_xyz")

    def test_non_callable_raises(self, loader):
        """Non-callable object raises BindingNotCallableError."""
        with pytest.raises(BindingNotCallableError):
            loader.resolve_target("os.path:sep")

    def test_trusted_prefixes_reject_disallowed(self):
        """trusted_package_prefixes raises BindingInvalidTargetError for modules outside the allowlist."""
        from apcore.bindings import BindingLoader

        restricted = BindingLoader(trusted_package_prefixes={"binding_helpers"})
        with pytest.raises(BindingInvalidTargetError):
            restricted.resolve_target("os.path:join")

    def test_trusted_prefixes_allow_matching(self):
        """trusted_package_prefixes permits modules on the allowlist."""
        from apcore.bindings import BindingLoader

        restricted = BindingLoader(trusted_package_prefixes={"binding_helpers"})
        result = restricted.resolve_target("binding_helpers:SimpleService.greet")
        assert callable(result)

    def test_trusted_prefixes_match_prefix_only(self):
        """A prefix 'binding_helpers' matches 'binding_helpers' AND 'binding_helpers.sub'."""
        from apcore.bindings import BindingLoader

        restricted = BindingLoader(trusted_package_prefixes={"binding_helpers"})
        # Exact match: should work.
        assert callable(restricted.resolve_target("binding_helpers:SimpleService.greet"))
        # Unrelated module starting with a different prefix: blocked.
        with pytest.raises(BindingInvalidTargetError):
            restricted.resolve_target("os:getcwd")


# ---------------------------------------------------------------------------
# Schema Mode Tests
# ---------------------------------------------------------------------------


class TestSchemaMode:
    """Tests for schema resolution modes."""

    def test_auto_schema_uses_type_inference(self, loader, registry, tmp_path):
        """auto_schema=true uses type inference on resolved callable."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n  - module_id: test.typed\n    target: binding_helpers:typed_function\n    auto_schema: true\n"
        )
        result = loader.load_bindings(str(f), registry)
        fm = result[0]
        assert "name" in fm.input_schema.model_fields
        assert "count" in fm.input_schema.model_fields

    def test_auto_schema_untyped_raises(self, loader, registry, tmp_path):
        """auto_schema=true with untyped callable raises BindingSchemaMissingError."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: test.untyped\n"
            "    target: binding_helpers:untyped_function\n"
            "    auto_schema: true\n"
        )
        with pytest.raises(BindingSchemaMissingError):
            loader.load_bindings(str(f), registry)

    def test_inline_schema_creates_model(self, loader, registry, tmp_path):
        """Inline input_schema/output_schema creates Pydantic models."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: test.inline\n"
            "    target: binding_helpers:typed_function\n"
            "    input_schema:\n"
            "      properties:\n"
            "        name:\n"
            "          type: string\n"
            "      required:\n"
            "        - name\n"
            "    output_schema:\n"
            "      properties:\n"
            "        result:\n"
            "          type: string\n"
        )
        result = loader.load_bindings(str(f), registry)
        fm = result[0]
        assert "name" in fm.input_schema.model_fields

    def test_inline_schema_with_untyped_callable(self, loader, registry, tmp_path):
        """Inline schema with untyped callable does not crash."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: test.untyped_inline\n"
            "    target: binding_helpers:untyped_function\n"
            "    input_schema:\n"
            "      properties:\n"
            "        name:\n"
            "          type: string\n"
            "      required:\n"
            "        - name\n"
            "    output_schema:\n"
            "      properties:\n"
            "        result:\n"
            "          type: string\n"
        )
        result = loader.load_bindings(str(f), registry)
        fm = result[0]
        assert "name" in fm.input_schema.model_fields

    def test_inline_schema_basic_types(self, loader, registry, tmp_path):
        """Inline schema basic types mapped correctly."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: test.types\n"
            "    target: binding_helpers:typed_function\n"
            "    input_schema:\n"
            "      properties:\n"
            "        name:\n"
            "          type: string\n"
            "        age:\n"
            "          type: integer\n"
            "        score:\n"
            "          type: number\n"
            "        active:\n"
            "          type: boolean\n"
            "      required:\n"
            "        - name\n"
            "    output_schema:\n"
            "      properties:\n"
            "        result:\n"
            "          type: string\n"
        )
        result = loader.load_bindings(str(f), registry)
        fm = result[0]
        fields = fm.input_schema.model_fields
        assert "name" in fields
        assert "age" in fields
        assert "score" in fields
        assert "active" in fields
        # Verify actual Python types via model validation
        inst = fm.input_schema(name="Alice", age=30, score=9.5, active=True)
        assert inst.name == "Alice"
        assert inst.age == 30
        assert inst.score == 9.5
        assert inst.active is True

    def test_inline_schema_required_array(self, loader, registry, tmp_path):
        """Required array marks fields as required."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: test.req\n"
            "    target: binding_helpers:typed_function\n"
            "    input_schema:\n"
            "      properties:\n"
            "        name:\n"
            "          type: string\n"
            "        age:\n"
            "          type: integer\n"
            "      required:\n"
            "        - name\n"
            "    output_schema:\n"
            "      properties:\n"
            "        result:\n"
            "          type: string\n"
        )
        result = loader.load_bindings(str(f), registry)
        fm = result[0]
        assert fm.input_schema.model_fields["name"].is_required()
        assert not fm.input_schema.model_fields["age"].is_required()

    def test_inline_schema_unsupported_features_permissive(self, loader, registry, tmp_path):
        """Unsupported features (oneOf) create permissive model."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: test.unsupported\n"
            "    target: binding_helpers:typed_function\n"
            "    input_schema:\n"
            "      oneOf:\n"
            "        - type: string\n"
            "        - type: integer\n"
            "    output_schema:\n"
            "      properties:\n"
            "        result:\n"
            "          type: string\n"
        )
        result = loader.load_bindings(str(f), registry)
        fm = result[0]
        assert fm.input_schema.model_config.get("extra") == "allow"

    def test_schema_ref_loads_external_file(self, loader, registry, tmp_path):
        """schema_ref loads external schema file with relative path."""
        schema_file = tmp_path / "schemas.yaml"
        schema_file.write_text(
            "input_schema:\n"
            "  properties:\n"
            "    name:\n"
            "      type: string\n"
            "  required:\n"
            "    - name\n"
            "output_schema:\n"
            "  properties:\n"
            "    result:\n"
            "      type: string\n"
        )
        binding_file = tmp_path / "test.binding.yaml"
        binding_file.write_text(
            "bindings:\n"
            "  - module_id: test.ref\n"
            "    target: binding_helpers:typed_function\n"
            "    schema_ref: schemas.yaml\n"
        )
        result = loader.load_bindings(str(binding_file), registry)
        fm = result[0]
        assert "name" in fm.input_schema.model_fields

    def test_schema_ref_not_found_raises(self, loader, registry, tmp_path):
        """schema_ref file not found raises BindingFileInvalidError."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: test.ref\n"
            "    target: binding_helpers:typed_function\n"
            "    schema_ref: nonexistent_schema.yaml\n"
        )
        with pytest.raises(BindingFileInvalidError):
            loader.load_bindings(str(f), registry)


# ---------------------------------------------------------------------------
# Registration and Integration Tests
# ---------------------------------------------------------------------------


class TestRegistrationAndIntegration:
    """Tests for registry integration and directory loading."""

    def test_load_bindings_registers(self, loader, registry, tmp_path):
        """load_bindings registers all modules with provided registry."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: func.one\n"
            "    target: binding_helpers:typed_function\n"
            "    auto_schema: true\n"
            "  - module_id: func.two\n"
            "    target: binding_helpers:typed_function\n"
            "    auto_schema: true\n"
        )
        loader.load_bindings(str(f), registry)
        assert isinstance(registry.get("func.one"), FunctionModule)
        assert isinstance(registry.get("func.two"), FunctionModule)

    def test_load_bindings_returns_function_modules(self, loader, registry, tmp_path):
        """load_bindings returns list of FunctionModule instances."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n  - module_id: test.func\n    target: binding_helpers:typed_function\n    auto_schema: true\n"
        )
        result = loader.load_bindings(str(f), registry)
        assert all(isinstance(fm, FunctionModule) for fm in result)

    def test_load_binding_dir(self, loader, registry, tmp_path):
        """load_binding_dir processes all matching files."""
        f1 = tmp_path / "a.binding.yaml"
        f1.write_text(
            "bindings:\n  - module_id: file1.func\n    target: binding_helpers:typed_function\n    auto_schema: true\n"
        )
        f2 = tmp_path / "b.binding.yaml"
        f2.write_text(
            "bindings:\n  - module_id: file2.func\n    target: binding_helpers:typed_function\n    auto_schema: true\n"
        )
        result = loader.load_binding_dir(str(tmp_path), registry)
        assert len(result) == 2

    def test_load_binding_dir_nonexistent_raises(self, loader, registry):
        """Non-existent directory raises BindingFileInvalidError."""
        with pytest.raises(BindingFileInvalidError):
            loader.load_binding_dir("/nonexistent/dir/xyz", registry)

    def test_load_binding_dir_no_files_empty(self, loader, registry, tmp_path):
        """Empty directory returns empty list."""
        result = loader.load_binding_dir(str(tmp_path), registry)
        assert result == []

    def test_fail_fast_on_first_error(self, loader, registry, tmp_path):
        """First invalid binding raises error, stops processing."""
        f = tmp_path / "test.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: bad.func\n"
            "    target: nonexistent.module.xyz:func\n"
            "    auto_schema: true\n"
            "  - module_id: good.func\n"
            "    target: binding_helpers:typed_function\n"
            "    auto_schema: true\n"
        )
        with pytest.raises(BindingModuleNotFoundError):
            loader.load_bindings(str(f), registry)
        assert registry.get("good.func") is None


# ---------------------------------------------------------------------------
# Section 06: Integration and Exports Tests
# ---------------------------------------------------------------------------


class TestPublicAPIExportsBindings:
    """Verify that binding-related symbols are importable from apcore."""

    def test_binding_loader_importable(self):
        """'from apcore import BindingLoader' works."""
        from apcore import BindingLoader as BL

        assert BL is not None


class TestBindingLoaderExecutorIntegration:
    """Full pipeline: BindingLoader -> Registry -> Executor.call() -> output."""

    def test_binding_loader_through_executor(self, tmp_path):
        """BindingLoader loads a binding, registers it, Executor.call() returns correct output."""
        # Create a Python module in tmp_path
        mod_file = tmp_path / "sample_mod.py"
        mod_file.write_text("def greet(name: str) -> dict:\n    return {'greeting': f'Hello, {name}!'}\n")

        # Create a YAML binding file
        binding_file = tmp_path / "sample.binding.yaml"
        binding_file.write_text(
            "bindings:\n  - module_id: sample.greet\n    target: sample_mod:greet\n    auto_schema: true\n"
        )

        # Temporarily add tmp_path to sys.path
        sys.path.insert(0, str(tmp_path))
        try:
            reg = Registry()
            loader = BindingLoader()
            loader.load_bindings(str(binding_file), reg)

            executor = Executor(registry=reg)
            result = executor.call("sample.greet", {"name": "World"})
            assert result == {"greeting": "Hello, World!"}
        finally:
            sys.path.remove(str(tmp_path))
            # Clean up imported module
            sys.modules.pop("sample_mod", None)
