"""Integration tests for the registry system and public API validation."""

from __future__ import annotations

from pathlib import Path
from typing import Any
from unittest.mock import MagicMock

import pytest
import yaml
from pydantic import BaseModel

from apcore.errors import CircularDependencyError


# ---------------------------------------------------------------------------
# Module file helpers
# ---------------------------------------------------------------------------


def _write_module_file(
    directory: Path,
    filename: str,
    class_name: str,
    description: str = "A test module",
    tags: list[str] | None = None,
) -> Path:
    """Write a valid module .py file to a directory."""
    tags_str = repr(tags or [])
    content = f"""from pydantic import BaseModel

class TestInput(BaseModel):
    value: str

class TestOutput(BaseModel):
    result: str

class {class_name}:
    input_schema = TestInput
    output_schema = TestOutput
    description = "{description}"
    tags = {tags_str}

    def execute(self, inputs, context=None):
        return {{"result": inputs["value"]}}
"""
    file_path = directory / filename
    file_path.parent.mkdir(parents=True, exist_ok=True)
    file_path.write_text(content)
    return file_path


def _write_meta_yaml(directory: Path, module_stem: str, meta: dict[str, Any]) -> Path:
    """Write a companion _meta.yaml file."""
    meta_path = directory / f"{module_stem}_meta.yaml"
    meta_path.write_text(yaml.dump(meta, default_flow_style=False))
    return meta_path


# =========================================================================
# Public API Import Tests
# =========================================================================


class TestPublicAPI:
    def test_import_registry(self) -> None:
        """from apcore.registry import Registry works."""
        from apcore.registry import Registry

        assert Registry is not None
        assert callable(Registry)

    def test_import_module_descriptor(self) -> None:
        """from apcore.registry import ModuleDescriptor works."""
        from apcore.registry import ModuleDescriptor

        assert ModuleDescriptor is not None

    def test_import_validate_module(self) -> None:
        """from apcore.registry import validate_module works."""
        from apcore.registry import validate_module

        assert callable(validate_module)

    def test_import_scan_extensions(self) -> None:
        """from apcore.registry import scan_extensions works."""
        from apcore.registry import scan_extensions

        assert callable(scan_extensions)

    def test_import_resolve_entry_point(self) -> None:
        """from apcore.registry import resolve_entry_point works."""
        from apcore.registry import resolve_entry_point

        assert callable(resolve_entry_point)

    def test_import_resolve_dependencies(self) -> None:
        """from apcore.registry import resolve_dependencies works."""
        from apcore.registry import resolve_dependencies

        assert callable(resolve_dependencies)

    def test_import_load_metadata_and_id_map(self) -> None:
        """from apcore.registry import load_metadata, load_id_map works."""
        from apcore.registry import load_id_map, load_metadata

        assert callable(load_metadata)
        assert callable(load_id_map)

    def test_import_scan_multi_root(self) -> None:
        """from apcore.registry import scan_multi_root works."""
        from apcore.registry import scan_multi_root

        assert callable(scan_multi_root)

    def test_import_discovered_module_and_dependency_info(self) -> None:
        """from apcore.registry import DiscoveredModule, DependencyInfo works."""
        from apcore.registry import DependencyInfo, DiscoveredModule

        assert DependencyInfo is not None
        assert DiscoveredModule is not None

    def test_all_public_names_in_all(self) -> None:
        """__all__ contains all expected public names."""
        import apcore.registry

        expected = {
            "Registry",
            "ModuleDescriptor",
            "DiscoveredModule",
            "DependencyInfo",
            "validate_module",
            "scan_extensions",
            "scan_multi_root",
            "resolve_entry_point",
            "resolve_dependencies",
            "load_metadata",
            "load_id_map",
        }
        assert expected.issubset(set(apcore.registry.__all__))


# =========================================================================
# Integration Tests: End-to-End Flows
# =========================================================================


class TestEndToEnd:
    def test_register_unregister_callbacks(self) -> None:
        """register() -> on callback fired -> unregister() -> on callback fired."""
        from apcore.registry.registry import Registry

        reg = Registry()

        class TestMod:
            input_schema = type("I", (BaseModel,), {"__annotations__": {"value": str}})
            output_schema = type("O", (BaseModel,), {"__annotations__": {"result": str}})
            description = "test"

            def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
                return {"result": "ok"}

        register_mock = MagicMock()
        unregister_mock = MagicMock()
        reg.on("register", register_mock)
        reg.on("unregister", unregister_mock)

        mod = TestMod()
        reg.register("test.mod", mod)
        register_mock.assert_called_once_with("test.mod", mod)

        assert reg.unregister("test.mod") is True
        unregister_mock.assert_called_once_with("test.mod", mod)

    def test_circular_dependency_detection(self, tmp_path: Path) -> None:
        """Circular dependency across 3 modules raises CircularDependencyError."""
        from apcore.registry.registry import Registry

        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext, "mod_a.py", "ModAModule", "Module A")
        _write_module_file(ext, "mod_b.py", "ModBModule", "Module B")
        _write_module_file(ext, "mod_c.py", "ModCModule", "Module C")
        _write_meta_yaml(ext, "mod_a", {"dependencies": [{"module_id": "mod_b"}]})
        _write_meta_yaml(ext, "mod_b", {"dependencies": [{"module_id": "mod_c"}]})
        _write_meta_yaml(ext, "mod_c", {"dependencies": [{"module_id": "mod_a"}]})

        reg = Registry(extensions_dir=str(ext))
        with pytest.raises(CircularDependencyError) as exc_info:
            reg.discover()
        path = exc_info.value.details["cycle_path"]
        assert "mod_a" in path
        assert "mod_b" in path
        assert "mod_c" in path
        assert reg.count == 0

    def test_multi_root_namespace_isolation(self, tmp_path: Path) -> None:
        """Multi-root discovery with namespace isolation."""
        from apcore.registry.registry import Registry

        root_a = tmp_path / "root_a"
        root_a.mkdir()
        root_b = tmp_path / "root_b"
        root_b.mkdir()
        _write_module_file(root_a, "process.py", "ProcessModuleA", "Process A")
        _write_module_file(root_b, "process.py", "ProcessModuleB", "Process B")

        reg = Registry(
            extensions_dirs=[
                {"root": str(root_a), "namespace": "alpha"},
                {"root": str(root_b), "namespace": "beta"},
            ]
        )
        count = reg.discover()
        assert count == 2
        assert reg.has("alpha.process")
        assert reg.has("beta.process")
        assert reg.get("alpha.process") is not reg.get("beta.process")

    def test_metadata_yaml_overrides(self, tmp_path: Path) -> None:
        """Metadata YAML overrides code-level description and tags."""
        from apcore.registry.registry import Registry

        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext, "mymod.py", "MyModule", "Code description", tags=["code-tag"])
        _write_meta_yaml(
            ext,
            "mymod",
            {
                "description": "YAML description",
                "tags": ["yaml-tag"],
            },
        )

        reg = Registry(extensions_dir=str(ext))
        reg.discover()

        defn = reg.get_definition("mymod")
        assert defn is not None
        assert defn.description == "YAML description"
        assert "yaml-tag" in defn.tags

    def test_mixed_valid_invalid_modules(self, tmp_path: Path) -> None:
        """discover() with mixed valid/invalid modules registers only valid ones."""
        from apcore.registry.registry import Registry

        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext, "valid_mod.py", "ValidModule", "Valid")
        # broken module - class without required attributes
        (ext / "broken_mod.py").write_text("class BrokenModule:\n    pass\n")
        # empty module - no module class at all
        (ext / "empty_mod.py").write_text("x = 42\n")

        reg = Registry(extensions_dir=str(ext))
        count = reg.discover()
        assert count == 1
        assert reg.has("valid_mod")
        assert not reg.has("broken_mod")
        assert not reg.has("empty_mod")

    def test_scalar_dependencies_does_not_abort_the_whole_discover_pass(self, tmp_path: Path) -> None:
        """A malformed ``dependencies:`` degrades to no dependencies (not an abort).

        ``parse_dependencies`` is reached from ``_resolve_load_order`` during
        discovery; a truthy scalar there used to raise a bare ``AttributeError``
        that took down the entire pass, not just the one bad module.
        """
        from apcore.registry.registry import Registry

        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext, "bad_deps.py", "BadDepsModule", "Bad deps")
        _write_module_file(ext, "good_mod.py", "GoodModModule", "Good")
        _write_meta_yaml(ext, "bad_deps", {"dependencies": "good_mod"})

        reg = Registry(extensions_dir=str(ext))
        assert reg.discover() == 2
        assert reg.has("bad_deps")
        assert reg.has("good_mod")

    def test_scalar_dependencies_yields_an_empty_descriptor_dependency_list(self, tmp_path: Path) -> None:
        """The other reachable call site, ``get_definition``, degrades the same way."""
        from apcore.registry.registry import Registry

        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext, "bad_deps.py", "BadDepsModule", "Bad deps")
        _write_meta_yaml(ext, "bad_deps", {"dependencies": "good_mod"})

        reg = Registry(extensions_dir=str(ext))
        reg.discover()

        defn = reg.get_definition("bad_deps")
        assert defn is not None
        assert defn.dependencies == []

    def test_conftest_fixtures_smoke(self, registry: Any, sample_module_class: type) -> None:
        """Smoke test: conftest fixtures work correctly."""
        from apcore.registry.registry import Registry

        assert isinstance(registry, Registry)
        assert hasattr(sample_module_class, "input_schema")
        assert hasattr(sample_module_class, "output_schema")
        assert hasattr(sample_module_class, "description")


class TestDiscoveryVersionedStoreParity:
    """Regression: discover() must populate _versioned_modules so version_hint resolves."""

    def test_discovered_module_resolvable_by_version_hint(self, tmp_path: Path) -> None:
        """After discover(), registry.get(id, version_hint=...) must find the discovered module.

        Prior to the fix, _register_in_order wrote only to _modules, not to
        _versioned_modules, so Registry.get(id, version_hint=...) -> None for
        any discovered module (version-hint queries routed through the
        versioned store first).
        """
        from apcore.registry.registry import Registry

        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext, "mod_versioned.py", "ModVersionedModule", "Versioned mod")
        _write_meta_yaml(ext, "mod_versioned", {"version": "1.2.3"})

        reg = Registry(extensions_dir=str(ext))
        registered = reg.discover()
        assert registered == 1

        # Without hint — latest
        m = reg.get("mod_versioned")
        assert m is not None

        # With explicit version hint matching the declared version
        m_exact = reg.get("mod_versioned", version_hint="1.2.3")
        assert m_exact is not None, "version_hint query failed — _register_in_order did not populate _versioned_modules"

        # With caret hint
        m_caret = reg.get("mod_versioned", version_hint="^1.0.0")
        assert m_caret is not None

    def test_discovered_default_version_is_one_zero_zero(self, tmp_path: Path) -> None:
        """Modules discovered without declared version get DEFAULT_MODULE_VERSION."""
        from apcore.registry.registry import DEFAULT_MODULE_VERSION, Registry

        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext, "mod_noversion.py", "ModNoVersionModule", "No version")

        reg = Registry(extensions_dir=str(ext))
        reg.discover()

        # The versioned store should have registered under DEFAULT_MODULE_VERSION
        assert reg._versioned_modules.has_version("mod_noversion", DEFAULT_MODULE_VERSION)
        assert DEFAULT_MODULE_VERSION == "1.0.0"

    def test_manual_register_default_version_aligned(self) -> None:
        """register() without version= gets DEFAULT_MODULE_VERSION, matching discover()."""
        from apcore.registry.registry import DEFAULT_MODULE_VERSION, Registry

        class PlainMod:
            input_schema = type("I", (BaseModel,), {"__annotations__": {"v": str}})
            output_schema = type("O", (BaseModel,), {"__annotations__": {"r": str}})
            description = "plain"

            def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
                return {"r": "ok"}

        reg = Registry()
        reg.register("mod.plain", PlainMod())
        assert reg._versioned_modules.has_version("mod.plain", DEFAULT_MODULE_VERSION)
