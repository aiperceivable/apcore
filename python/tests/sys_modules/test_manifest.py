"""Tests for system.manifest.module and system.manifest.full sys modules."""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import BaseModel

from apcore.config import Config
from apcore.errors import InvalidInputError, ModuleNotFoundError
from apcore.module import ModuleAnnotations
from apcore.registry.registry import Registry
from apcore.sys_modules.manifest import ManifestFullModule, ManifestModuleModule


# --- Helpers ---


class _InputSchema(BaseModel):
    x: int
    y: int


class _OutputSchema(BaseModel):
    result: int


class _SampleModule:
    description = "A sample module"
    documentation = "Sample module docs"
    input_schema = _InputSchema
    output_schema = _OutputSchema
    tags = ["math", "sample"]
    annotations = ModuleAnnotations(readonly=True, idempotent=True)

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"result": inputs["x"] + inputs["y"]}


class _MinimalModule:
    description = "Minimal module"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


def _make_registry_with_module(
    module_id: str = "math.add",
    module: Any | None = None,
) -> Registry:
    """Create a registry and register a module."""
    registry = Registry(extensions_dir="/tmp/fake_extensions")
    if module is None:
        module = _SampleModule()
    registry.register(module_id, module)
    return registry


# --- Tests ---


class TestManifestModuleRequiresModuleId:
    def test_manifest_module_requires_module_id(self) -> None:
        registry = _make_registry_with_module()
        mod = ManifestModuleModule(registry=registry)
        with pytest.raises(InvalidInputError):
            mod.execute({}, context=None)

    def test_manifest_module_requires_module_id_empty_string(self) -> None:
        registry = _make_registry_with_module()
        mod = ManifestModuleModule(registry=registry)
        with pytest.raises(InvalidInputError):
            mod.execute({"module_id": ""}, context=None)


class TestManifestModuleNotFound:
    def test_manifest_module_not_found_error(self) -> None:
        registry = Registry(extensions_dir="/tmp/fake_extensions")
        mod = ManifestModuleModule(registry=registry)
        with pytest.raises(ModuleNotFoundError):
            mod.execute({"module_id": "nonexistent.module"}, context=None)


class TestManifestModuleBasicFields:
    def test_manifest_module_returns_basic_fields(self) -> None:
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert result["module_id"] == "math.add"
        assert result["description"] == "A sample module"
        assert result["documentation"] == "Sample module docs"


class TestManifestModuleSourcePath:
    def test_manifest_module_returns_source_path(self) -> None:
        config = Config(data={"project": {"source_root": "src/modules"}})
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry, config=config)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert result["source_path"] == "src/modules/math/add.py"

    def test_manifest_module_source_path_null_when_not_configured(self) -> None:
        config = Config(data={"project": {"source_root": ""}})
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry, config=config)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert result["source_path"] is None

    def test_manifest_module_source_path_null_when_no_config(self) -> None:
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert result["source_path"] is None


class TestManifestModuleSchemas:
    def test_manifest_module_returns_input_schema(self) -> None:
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert "input_schema" in result
        assert result["input_schema"]["properties"]["x"]["type"] == "integer"

    def test_manifest_module_returns_output_schema(self) -> None:
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert "output_schema" in result
        assert result["output_schema"]["properties"]["result"]["type"] == "integer"


class TestManifestModuleAnnotations:
    def test_manifest_module_returns_annotations(self) -> None:
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert "annotations" in result
        annotations = result["annotations"]
        assert annotations["readonly"] is True
        assert annotations["idempotent"] is True

    def test_manifest_module_annotations_readonly_idempotent(self) -> None:
        """Verify ManifestModuleModule itself has readonly=True, idempotent=True."""
        registry = Registry(extensions_dir="/tmp/fake_extensions")
        mod = ManifestModuleModule(registry=registry)
        assert mod.annotations.readonly is True
        assert mod.annotations.idempotent is True


class TestManifestModuleTags:
    def test_manifest_module_returns_tags(self) -> None:
        registry = _make_registry_with_module("math.add")
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert "tags" in result
        assert "math" in result["tags"]
        assert "sample" in result["tags"]


class TestManifestModuleDependencies:
    def test_manifest_module_returns_dependencies(self) -> None:
        module = _SampleModule()
        registry = Registry(extensions_dir="/tmp/fake_extensions")
        registry.register("math.add", module)
        # Inject metadata with dependencies via registry internals
        registry._module_meta["math.add"] = {
            "dependencies": [{"module_id": "core.logger", "optional": False}],
            "metadata": {},
        }
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert "dependencies" in result
        assert result["dependencies"] == [{"module_id": "core.logger", "optional": False}]


class TestManifestModuleMetadata:
    def test_manifest_module_returns_metadata(self) -> None:
        module = _SampleModule()
        registry = Registry(extensions_dir="/tmp/fake_extensions")
        registry.register("math.add", module)
        registry._module_meta["math.add"] = {
            "metadata": {"author": "test", "license": "MIT"},
        }
        mod = ManifestModuleModule(registry=registry)
        result = mod.execute({"module_id": "math.add"}, context=None)
        assert "metadata" in result
        assert result["metadata"]["author"] == "test"
        assert result["metadata"]["license"] == "MIT"


# --- Helpers for manifest_full tests ---


class _BillingModule:
    description = "Billing module"
    documentation = "Billing docs"
    input_schema = _InputSchema
    output_schema = _OutputSchema
    tags = ["billing", "payment"]
    annotations = ModuleAnnotations(readonly=True)

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


class _PaymentModule:
    description = "Payment processor"
    documentation = "Payment docs"
    input_schema = _InputSchema
    output_schema = _OutputSchema
    tags = ["payment"]
    annotations = ModuleAnnotations(readonly=False, destructive=True)

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


def _make_full_registry() -> Registry:
    """Create a registry with several modules for manifest_full testing."""
    registry = Registry(extensions_dir="/tmp/fake_extensions")
    registry.register("math.add", _SampleModule())
    registry.register("billing.invoice", _BillingModule())
    registry.register("payment.process", _PaymentModule())
    return registry


def _make_config() -> Config:
    return Config(
        data={
            "project": {
                "name": "test_project",
                "version": "1.0.0",
                "source_root": "src/modules",
            },
        }
    )


# --- Tests for system.manifest.full ---


class TestManifestFullReturnsAllModules:
    def test_manifest_full_returns_all_modules(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({}, context=None)
        module_ids = [m["module_id"] for m in result["modules"]]
        assert "math.add" in module_ids
        assert "billing.invoice" in module_ids
        assert "payment.process" in module_ids


class TestManifestFullProjectInfo:
    def test_manifest_full_project_info(self) -> None:
        registry = _make_full_registry()
        config = _make_config()
        mod = ManifestFullModule(registry=registry, config=config)
        result = mod.execute({}, context=None)
        assert result["project_name"] == "test_project"
        assert "module_count" in result


class TestManifestFullModuleFields:
    def test_manifest_full_module_fields(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({}, context=None)
        expected_fields = {
            "module_id",
            "description",
            "documentation",
            "source_path",
            "input_schema",
            "output_schema",
            "annotations",
            "tags",
            "dependencies",
            "metadata",
        }
        for entry in result["modules"]:
            assert expected_fields.issubset(
                set(entry.keys())
            ), f"Missing fields in {entry['module_id']}: {expected_fields - set(entry.keys())}"


class TestManifestFullIncludeSchemas:
    def test_manifest_full_include_schemas_true(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({"include_schemas": True}, context=None)
        for entry in result["modules"]:
            if entry["module_id"] == "math.add":
                assert entry["input_schema"] is not None
                assert entry["input_schema"] != {}
                assert entry["output_schema"] is not None
                assert entry["output_schema"] != {}

    def test_manifest_full_include_schemas_false(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({"include_schemas": False}, context=None)
        for entry in result["modules"]:
            assert entry["input_schema"] is None
            assert entry["output_schema"] is None


class TestManifestFullIncludeSourcePaths:
    def test_manifest_full_include_source_paths_true(self) -> None:
        registry = _make_full_registry()
        config = _make_config()
        mod = ManifestFullModule(registry=registry, config=config)
        result = mod.execute({"include_source_paths": True}, context=None)
        for entry in result["modules"]:
            assert entry["source_path"] is not None

    def test_manifest_full_include_source_paths_false(self) -> None:
        registry = _make_full_registry()
        config = _make_config()
        mod = ManifestFullModule(registry=registry, config=config)
        result = mod.execute({"include_source_paths": False}, context=None)
        for entry in result["modules"]:
            assert entry["source_path"] is None


class TestManifestFullFilterByPrefix:
    def test_manifest_full_filter_by_prefix(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({"prefix": "payment"}, context=None)
        module_ids = [m["module_id"] for m in result["modules"]]
        assert "payment.process" in module_ids
        assert "math.add" not in module_ids
        assert "billing.invoice" not in module_ids


class TestManifestFullFilterByTags:
    def test_manifest_full_filter_by_tags(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({"tags": ["billing"]}, context=None)
        module_ids = [m["module_id"] for m in result["modules"]]
        assert "billing.invoice" in module_ids
        assert "math.add" not in module_ids


class TestManifestFullFilterByPrefixAndTags:
    def test_manifest_full_filter_by_prefix_and_tags(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        # prefix="payment" AND tags=["payment"] -> only payment.process
        result = mod.execute({"prefix": "payment", "tags": ["payment"]}, context=None)
        module_ids = [m["module_id"] for m in result["modules"]]
        assert "payment.process" in module_ids
        assert "billing.invoice" not in module_ids
        assert "math.add" not in module_ids


class TestManifestFullNoFiltersReturnsAll:
    def test_manifest_full_no_filters_returns_all(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({}, context=None)
        assert len(result["modules"]) == 3


class TestManifestFullSelfReflective:
    def test_manifest_full_self_reflective(self) -> None:
        registry = Registry(extensions_dir="/tmp/fake_extensions")
        manifest_mod = ManifestFullModule(registry=registry)
        # Bypass reserved word check (sys modules are registered internally)
        registry._modules["system.manifest.full"] = manifest_mod
        registry._lowercase_map["system.manifest.full"] = "system.manifest.full"
        result = manifest_mod.execute({}, context=None)
        module_ids = [m["module_id"] for m in result["modules"]]
        assert "system.manifest.full" in module_ids


class TestManifestFullModuleCountMatches:
    def test_manifest_full_module_count_matches(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({}, context=None)
        assert result["module_count"] == len(result["modules"])

    def test_manifest_full_module_count_matches_filtered(self) -> None:
        registry = _make_full_registry()
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({"prefix": "math"}, context=None)
        assert result["module_count"] == len(result["modules"])
        assert result["module_count"] == 1


class TestManifestFullAnnotations:
    def test_manifest_full_annotations(self) -> None:
        mod = ManifestFullModule(
            registry=Registry(extensions_dir="/tmp/fake_extensions"),
        )
        assert mod.annotations.readonly is True
        assert mod.annotations.idempotent is True


class TestManifestFullEmptyRegistry:
    def test_manifest_full_empty_registry(self) -> None:
        registry = Registry(extensions_dir="/tmp/fake_extensions")
        mod = ManifestFullModule(registry=registry)
        result = mod.execute({}, context=None)
        assert result["modules"] == []
        assert result["module_count"] == 0
