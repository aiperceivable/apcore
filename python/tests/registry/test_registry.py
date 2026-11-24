"""Tests for the Registry class."""

from __future__ import annotations

import logging
import threading
import time
from pathlib import Path
from typing import Any
from unittest.mock import MagicMock

import pytest
import yaml
from pydantic import BaseModel

from apcore.errors import (
    CircularDependencyError,
    ErrorCodes,
    InvalidInputError,
    ModuleNotFoundError,
)
from apcore.registry.registry import (
    MAX_MODULE_ID_LENGTH,
    RESERVED_WORDS,
    Registry,
)
from apcore.registry.types import ModuleDescriptor


# ---------------------------------------------------------------------------
# Helper schemas and modules for tests
# ---------------------------------------------------------------------------


class _TestInput(BaseModel):
    value: str


class _TestOutput(BaseModel):
    result: str


class _ValidModule:
    """A valid duck-typed test module."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "A valid test module"
    tags = ["test", "sample"]
    version = "2.0.0"

    def execute(self, inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": inputs["value"]}


class _ValidModuleB:
    """Another valid test module."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "Another valid test module"
    tags = ["test"]

    def execute(self, _inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": "b"}


class _ModuleWithOnLoad:
    """Module with on_load lifecycle hook."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "Module with on_load"

    def __init__(self) -> None:
        self.load_called = False

    def execute(self, _inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": "ok"}

    def on_load(self) -> None:
        self.load_called = True


class _ModuleWithFailingOnLoad:
    """Module whose on_load raises."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "Module with failing on_load"

    def execute(self, _inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": "fail"}

    def on_load(self) -> None:
        raise RuntimeError("on_load failed")


class _ModuleWithOnUnload:
    """Module with on_unload lifecycle hook."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "Module with on_unload"

    def __init__(self) -> None:
        self.unload_called = False

    def execute(self, _inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": "ok"}

    def on_unload(self) -> None:
        self.unload_called = True


class _ModuleWithFailingOnUnload:
    """Module whose on_unload raises."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "Module with failing on_unload"

    def execute(self, _inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": "fail"}

    def on_unload(self) -> None:
        raise RuntimeError("on_unload failed")


# ===== Constructor =====


class TestConstructor:
    def test_no_args_defaults(self) -> None:
        """Registry() with no args defaults extension_roots to ['./extensions']."""
        reg = Registry()
        assert len(reg._extension_roots) == 1
        assert reg._extension_roots[0]["root"] == "./extensions"

    def test_extensions_dir_param(self) -> None:
        """Registry(extensions_dir='custom') uses that as single root."""
        reg = Registry(extensions_dir="custom")
        assert reg._extension_roots == [{"root": "custom"}]

    def test_extensions_dirs_param(self) -> None:
        """Registry(extensions_dirs=[...]) uses multi-root config."""
        reg = Registry(extensions_dirs=["dir_a", {"root": "dir_b", "namespace": "ns"}])
        assert len(reg._extension_roots) == 2
        assert reg._extension_roots[0] == {"root": "dir_a"}
        assert reg._extension_roots[1] == {"root": "dir_b", "namespace": "ns"}

    def test_both_extensions_dir_and_dirs_raises(self) -> None:
        """Cannot specify both extensions_dir and extensions_dirs."""
        with pytest.raises(InvalidInputError, match="Cannot specify both"):
            Registry(extensions_dir="x", extensions_dirs=["y"])

    def test_config_extensions_root(self) -> None:
        """Config with extensions.root sets the extension root."""
        from apcore.config import Config

        config = Config(data={"extensions": {"root": "/custom/path"}})
        reg = Registry(config=config)
        assert reg._extension_roots == [{"root": "/custom/path"}]

    def test_individual_param_overrides_config(self) -> None:
        """extensions_dir overrides config.extensions.root."""
        from apcore.config import Config

        config = Config(data={"extensions": {"root": "/from_config"}})
        reg = Registry(config=config, extensions_dir="/override")
        assert reg._extension_roots == [{"root": "/override"}]


# ===== register() =====


class TestRegister:
    def test_register_stores_module(self) -> None:
        """register() stores module, retrievable via get()."""
        reg = Registry()
        mod = _ValidModule()
        reg.register("test.module", mod)
        assert reg.get("test.module") is mod

    def test_register_duplicate_raises(self) -> None:
        """Registering same ID twice raises InvalidInputError."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        with pytest.raises(InvalidInputError, match="already registered"):
            reg.register("test.module", _ValidModuleB())

    def test_register_calls_on_load(self) -> None:
        """register() calls on_load() if the method exists."""
        reg = Registry()
        mod = _ModuleWithOnLoad()
        reg.register("test.module", mod)
        assert mod.load_called is True

    def test_register_on_load_failure_removes_module(self) -> None:
        """on_load() failure removes module and propagates exception."""
        reg = Registry()
        with pytest.raises(RuntimeError, match="on_load failed"):
            reg.register("test.module", _ModuleWithFailingOnLoad())
        assert reg.get("test.module") is None

    def test_register_triggers_callback(self) -> None:
        """register() triggers 'register' event callbacks."""
        reg = Registry()
        calls: list[tuple[str, Any]] = []
        reg.on("register", lambda mid, mod: calls.append((mid, mod)))
        mod = _ValidModule()
        reg.register("test.module", mod)
        assert len(calls) == 1
        assert calls[0] == ("test.module", mod)

    def test_register_without_on_load(self) -> None:
        """Module without on_load() method is registered fine."""
        reg = Registry()
        mod = _ValidModule()
        assert not hasattr(mod, "on_load")
        reg.register("test.module", mod)
        assert reg.get("test.module") is mod

    def test_register_empty_id_raises(self) -> None:
        """register('') raises InvalidInputError."""
        reg = Registry()
        with pytest.raises(InvalidInputError, match="non-empty"):
            reg.register("", _ValidModule())


# ===== unregister() =====


class TestUnregister:
    def test_unregister_existing(self) -> None:
        """Unregister existing module returns True, module gone."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        assert reg.unregister("test.module") is True
        assert reg.get("test.module") is None

    def test_unregister_nonexistent(self) -> None:
        """Unregister non-existent ID returns False."""
        reg = Registry()
        assert reg.unregister("nonexistent") is False

    def test_unregister_calls_on_unload(self) -> None:
        """Unregister calls on_unload() if the method exists."""
        reg = Registry()
        mod = _ModuleWithOnUnload()
        reg.register("test.module", mod)
        reg.unregister("test.module")
        assert mod.unload_called is True

    def test_unregister_on_unload_failure_still_unregisters(self, caplog: pytest.LogCaptureFixture) -> None:
        """on_unload() failure is logged, but module is still removed."""
        reg = Registry()
        mod = _ModuleWithFailingOnUnload()
        reg.register("test.module", mod)
        with caplog.at_level(logging.ERROR):
            result = reg.unregister("test.module")
        assert result is True
        assert reg.get("test.module") is None
        assert "on_unload" in caplog.text

    def test_unregister_triggers_callback(self) -> None:
        """Unregister triggers 'unregister' event callbacks."""
        reg = Registry()
        calls: list[tuple[str, Any]] = []
        reg.on("unregister", lambda mid, mod: calls.append((mid, mod)))
        mod = _ValidModule()
        reg.register("test.module", mod)
        reg.unregister("test.module")
        assert len(calls) == 1
        assert calls[0][0] == "test.module"

    def test_unregister_clears_schema_cache(self) -> None:
        """Unregister clears schema cache for the module."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        reg._schema_cache["test.module"] = {"cached": True}
        reg.unregister("test.module")
        assert "test.module" not in reg._schema_cache


# ===== Query Methods =====


class TestQueryMethods:
    def test_get_empty_id_raises(self) -> None:
        """get('') raises ModuleNotFoundError."""
        reg = Registry()
        with pytest.raises(ModuleNotFoundError):
            reg.get("")

    def test_get_nonexistent_returns_none(self) -> None:
        """get() for missing ID returns None."""
        reg = Registry()
        assert reg.get("nonexistent") is None

    def test_get_existing(self) -> None:
        """get() returns the registered module."""
        reg = Registry()
        mod = _ValidModule()
        reg.register("test.module", mod)
        assert reg.get("test.module") is mod

    def test_has(self) -> None:
        """has() returns True/False based on registration."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        assert reg.has("test.module") is True
        assert reg.has("missing") is False

    def test_list_no_filters(self) -> None:
        """list() with no filters returns sorted list of all IDs."""
        reg = Registry()
        reg.register("c.module", _ValidModule())
        reg.register("a.module", _ValidModuleB())
        reg.register("b.module", _ValidModule())
        assert reg.list() == ["a.module", "b.module", "c.module"]

    def test_list_prefix_filter(self) -> None:
        """list(prefix='foo') returns only IDs starting with 'foo'."""
        reg = Registry()
        reg.register("foo.bar", _ValidModule())
        reg.register("foo.baz", _ValidModuleB())
        reg.register("other.thing", _ValidModule())
        assert reg.list(prefix="foo") == ["foo.bar", "foo.baz"]

    def test_list_tags_filter(self) -> None:
        """list(tags=[...]) returns modules with ALL specified tags."""
        reg = Registry()
        reg.register("mod.a", _ValidModule())  # tags=["test", "sample"]
        reg.register("mod.b", _ValidModuleB())  # tags=["test"]
        assert reg.list(tags=["test", "sample"]) == ["mod.a"]
        assert reg.list(tags=["test"]) == ["mod.a", "mod.b"]

    def test_list_prefix_and_tags(self) -> None:
        """list() with both prefix and tags applies both filters."""
        reg = Registry()
        reg.register("foo.a", _ValidModule())  # tags=["test", "sample"]
        reg.register("foo.b", _ValidModuleB())  # tags=["test"]
        reg.register("bar.a", _ValidModule())  # tags=["test", "sample"]
        result = reg.list(prefix="foo", tags=["test", "sample"])
        assert result == ["foo.a"]

    def test_iter(self) -> None:
        """iter() returns (id, module) tuples."""
        reg = Registry()
        mod_a = _ValidModule()
        mod_b = _ValidModuleB()
        reg.register("a.mod", mod_a)
        reg.register("b.mod", mod_b)
        items = list(reg.iter())
        assert len(items) == 2
        ids = {i[0] for i in items}
        assert ids == {"a.mod", "b.mod"}

    def test_count(self) -> None:
        """count property returns number of registered modules."""
        reg = Registry()
        assert reg.count == 0
        reg.register("a", _ValidModule())
        reg.register("b", _ValidModuleB())
        assert reg.count == 2

    def test_module_ids(self) -> None:
        """module_ids returns sorted list of IDs."""
        reg = Registry()
        reg.register("z.mod", _ValidModule())
        reg.register("a.mod", _ValidModuleB())
        assert reg.module_ids == ["a.mod", "z.mod"]


# ===== get_definition() =====


class TestGetDefinition:
    def test_existing_module(self) -> None:
        """get_definition() returns ModuleDescriptor for registered module."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        defn = reg.get_definition("test.module")
        assert defn is not None
        assert isinstance(defn, ModuleDescriptor)
        assert defn.module_id == "test.module"
        assert defn.description == "A valid test module"
        assert defn.version == "2.0.0"
        assert defn.tags == ["test", "sample"]
        assert "properties" in defn.input_schema
        assert "properties" in defn.output_schema

    def test_nonexistent_returns_none(self) -> None:
        """get_definition() for missing module returns None."""
        reg = Registry()
        assert reg.get_definition("missing") is None

    def test_dict_schema(self) -> None:
        """get_definition() works when input_schema / output_schema are plain dicts."""

        class DictSchemaModule:
            description = "Module with dict schemas"
            input_schema = {"type": "object", "properties": {"name": {"type": "string"}}}
            output_schema = {"type": "object", "properties": {"ok": {"type": "boolean"}}}
            version = "1.0.0"
            tags: list[str] = []

            def execute(self, inputs: dict, context: object = None) -> dict:
                return {"ok": True}

        reg = Registry()
        reg.register("dict.mod", DictSchemaModule())
        defn = reg.get_definition("dict.mod")
        assert defn is not None
        assert defn.input_schema == {"type": "object", "properties": {"name": {"type": "string"}}}
        assert defn.output_schema == {"type": "object", "properties": {"ok": {"type": "boolean"}}}

    def test_no_schema(self) -> None:
        """get_definition() returns empty dict when module has no schema."""

        class NoSchemaModule:
            description = "Module without schemas"
            version = "1.0.0"
            tags: list[str] = []

            def execute(self, inputs: dict, context: object = None) -> dict:
                return {}

        reg = Registry()
        reg.register("no.typed", NoSchemaModule())
        defn = reg.get_definition("no.typed")
        assert defn is not None
        assert defn.input_schema == {}
        assert defn.output_schema == {}


# ===== Event Callbacks =====


class TestEventCallbacks:
    def test_on_register_callback(self) -> None:
        """on('register', cb) calls cb when module is registered."""
        reg = Registry()
        mock_cb = MagicMock()
        reg.on("register", mock_cb)
        mod = _ValidModule()
        reg.register("test.module", mod)
        mock_cb.assert_called_once_with("test.module", mod)

    def test_on_unregister_callback(self) -> None:
        """on('unregister', cb) calls cb when module is unregistered."""
        reg = Registry()
        mock_cb = MagicMock()
        reg.on("unregister", mock_cb)
        mod = _ValidModule()
        reg.register("test.module", mod)
        reg.unregister("test.module")
        mock_cb.assert_called_once_with("test.module", mod)

    def test_invalid_event_raises(self) -> None:
        """on('invalid', cb) raises InvalidInputError."""
        reg = Registry()
        with pytest.raises(InvalidInputError, match="Invalid event"):
            reg.on("invalid", lambda mid, mod: None)

    def test_callback_error_logged(self, caplog: pytest.LogCaptureFixture) -> None:
        """Callback that raises -> error logged, registration completes."""
        reg = Registry()

        def bad_cb(mid: str, mod: Any) -> None:
            raise ValueError("callback boom")

        reg.on("register", bad_cb)
        with caplog.at_level(logging.ERROR):
            reg.register("test.module", _ValidModule())
        assert reg.has("test.module")
        assert "callback boom" in caplog.text

    def test_multiple_callbacks(self) -> None:
        """Multiple callbacks all called in order."""
        reg = Registry()
        order: list[int] = []
        reg.on("register", lambda mid, mod: order.append(1))
        reg.on("register", lambda mid, mod: order.append(2))
        reg.register("test.module", _ValidModule())
        assert order == [1, 2]

    def test_callback_error_increments_metrics_when_collector_wired(self) -> None:
        """When a MetricsCollector is passed to Registry, callback errors
        increment apcore_registry_callback_errors_total with event/module_id/error_type labels."""
        from apcore.observability.metrics import MetricsCollector

        metrics = MetricsCollector()
        reg = Registry(metrics_collector=metrics)

        def bad_cb(mid: str, mod: Any) -> None:
            raise ValueError("callback boom")

        reg.on("register", bad_cb)
        reg.register("test.module", _ValidModule())

        snap = metrics.snapshot()
        # Find the counter key matching our labels (label tuple is sorted).
        matched = [
            (name, dict(labels), count)
            for (name, labels), count in snap["counters"].items()
            if name == "apcore_registry_callback_errors_total"
        ]
        assert len(matched) == 1
        _, labels, count = matched[0]
        assert count == 1
        assert labels == {
            "event": "register",
            "module_id": "test.module",
            "error_type": "ValueError",
        }


# ===== discover() =====


def _write_module_file(path: Path, class_name: str, description: str) -> None:
    """Helper: write a valid module .py file."""
    content = f"""from pydantic import BaseModel

class InputModel(BaseModel):
    value: str

class OutputModel(BaseModel):
    result: str

class {class_name}:
    input_schema = InputModel
    output_schema = OutputModel
    description = "{description}"
    tags = ["auto"]

    def execute(self, inputs, context=None):
        return {{"result": inputs["value"]}}
"""
    path.write_text(content)


class TestDiscover:
    def test_discover_valid_modules(self, tmp_path: Path) -> None:
        """discover() finds valid modules and returns count."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "hello.py", "HelloModule", "Hello module")
        _write_module_file(ext / "world.py", "WorldModule", "World module")
        reg = Registry(extensions_dir=str(ext))
        count = reg.discover()
        assert count == 2
        assert reg.has("hello")
        assert reg.has("world")

    def test_discover_empty_dir(self, tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
        """discover() with empty dir returns 0."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        reg = Registry(extensions_dir=str(ext))
        with caplog.at_level(logging.WARNING):
            count = reg.discover()
        assert count == 0

    def test_discover_skips_invalid_modules(self, tmp_path: Path) -> None:
        """discover() skips modules that fail validation."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "valid.py", "ValidModule", "Valid module")
        # Write an invalid module (no execute method, no schemas)
        (ext / "invalid.py").write_text("class InvalidModule:\n    pass\n")
        reg = Registry(extensions_dir=str(ext))
        count = reg.discover()
        assert count == 1
        assert reg.has("valid")
        assert not reg.has("invalid")

    def test_discover_with_metadata(self, tmp_path: Path) -> None:
        """discover() merges companion _meta.yaml with code attributes."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "mymod.py", "MyModModule", "Code description")
        meta = ext / "mymod_meta.yaml"
        meta.write_text(yaml.dump({"description": "YAML description", "tags": ["yaml_tag"]}))
        reg = Registry(extensions_dir=str(ext))
        reg.discover()
        # Check that the merged metadata reflects YAML overrides
        meta_info = reg._module_meta.get("mymod", {})
        assert meta_info.get("description") == "YAML description"
        assert meta_info.get("tags") == ["yaml_tag"]

    def test_discover_on_load_called(self, tmp_path: Path) -> None:
        """discover() calls on_load() for each module."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        (ext / "loadable.py").write_text(
            """
from pydantic import BaseModel

class Input(BaseModel):
    value: str

class Output(BaseModel):
    result: str

_load_called = False

class LoadableModule:
    input_schema = Input
    output_schema = Output
    description = "Loadable"

    def execute(self, inputs, context=None):
        return {"result": "ok"}

    def on_load(self):
        global _load_called
        _load_called = True
"""
        )
        reg = Registry(extensions_dir=str(ext))
        reg.discover()
        assert reg.has("loadable")

    def test_discover_on_load_failure_skips(self, tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
        """discover() skips module whose on_load() fails."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "good.py", "GoodModule", "Good")
        (ext / "bad_load.py").write_text(
            """
from pydantic import BaseModel

class Input(BaseModel):
    value: str

class Output(BaseModel):
    result: str

class BadLoadModule:
    input_schema = Input
    output_schema = Output
    description = "Bad loader"

    def execute(self, inputs, context=None):
        return {"result": "ok"}

    def on_load(self):
        raise RuntimeError("on_load exploded")
"""
        )
        reg = Registry(extensions_dir=str(ext))
        with caplog.at_level(logging.ERROR):
            count = reg.discover()
        # good.py loaded, bad_load.py skipped
        assert count == 1
        assert reg.has("good")
        assert not reg.has("bad_load")
        assert "on_load" in caplog.text

    def test_discover_with_dependencies(self, tmp_path: Path) -> None:
        """discover() loads modules respecting dependency order."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        # Module A depends on B
        _write_module_file(ext / "mod_a.py", "ModAModule", "Module A")
        (ext / "mod_a_meta.yaml").write_text(
            yaml.dump(
                {
                    "dependencies": [{"module_id": "mod_b"}],
                }
            )
        )
        _write_module_file(ext / "mod_b.py", "ModBModule", "Module B")
        reg = Registry(extensions_dir=str(ext))
        count = reg.discover()
        assert count == 2
        assert reg.has("mod_a")
        assert reg.has("mod_b")

    def test_discover_circular_deps_raises(self, tmp_path: Path) -> None:
        """discover() with circular deps raises CircularDependencyError."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "cycle_a.py", "CycleAModule", "Cycle A")
        (ext / "cycle_a_meta.yaml").write_text(
            yaml.dump(
                {
                    "dependencies": [{"module_id": "cycle_b"}],
                }
            )
        )
        _write_module_file(ext / "cycle_b.py", "CycleBModule", "Cycle B")
        (ext / "cycle_b_meta.yaml").write_text(
            yaml.dump(
                {
                    "dependencies": [{"module_id": "cycle_a"}],
                }
            )
        )
        reg = Registry(extensions_dir=str(ext))
        with pytest.raises(CircularDependencyError):
            reg.discover()
        # No modules registered when circular deps detected
        assert reg.count == 0

    def test_discover_nonexistent_root_raises(self, tmp_path: Path) -> None:
        """discover() with nonexistent extension root raises ConfigNotFoundError."""
        from apcore.errors import ConfigNotFoundError

        reg = Registry(extensions_dir=str(tmp_path / "nonexistent"))
        with pytest.raises(ConfigNotFoundError):
            reg.discover()

    def test_discover_multi_root(self, tmp_path: Path) -> None:
        """discover() with multiple extension roots registers from all roots."""
        root_a = tmp_path / "ext_a"
        root_a.mkdir()
        root_b = tmp_path / "ext_b"
        root_b.mkdir()
        _write_module_file(root_a / "mod_a.py", "ModAModule", "Module A")
        _write_module_file(root_b / "mod_b.py", "ModBModule", "Module B")
        reg = Registry(
            extensions_dirs=[
                {"root": str(root_a), "namespace": "ns_a"},
                {"root": str(root_b), "namespace": "ns_b"},
            ]
        )
        count = reg.discover()
        assert count == 2
        assert reg.has("ns_a.mod_a")
        assert reg.has("ns_b.mod_b")

    def test_discover_get_definition_with_metadata(self, tmp_path: Path) -> None:
        """get_definition() after discover reflects merged metadata."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "defmod.py", "DefModModule", "Code desc")
        (ext / "defmod_meta.yaml").write_text(
            yaml.dump(
                {
                    "description": "YAML desc",
                    "version": "3.0.0",
                    "tags": ["yaml_tag"],
                }
            )
        )
        reg = Registry(extensions_dir=str(ext))
        reg.discover()
        defn = reg.get_definition("defmod")
        assert defn is not None
        assert defn.description == "YAML desc"
        assert defn.version == "3.0.0"
        assert defn.tags == ["yaml_tag"]

    def test_discover_yaml_annotations_field_level_merge(self, tmp_path: Path) -> None:
        """Spec §4.13: YAML annotations merge field-by-field over code annotations.

        Regression: prior to wiring schema.merge_annotations into the registry,
        ``descriptor.annotations`` was read directly from the class attribute,
        so YAML annotations were silently dropped.
        """
        from apcore.module import ModuleAnnotations

        ext = tmp_path / "extensions"
        ext.mkdir()
        # Module class defines readonly=True; YAML adds destructive=True.
        # The merge must preserve readonly while applying destructive.
        (ext / "annotated.py").write_text(
            "from pydantic import BaseModel\n"
            "from apcore.module import ModuleAnnotations\n"
            "\n"
            "class InputModel(BaseModel):\n"
            "    value: str\n"
            "\n"
            "class OutputModel(BaseModel):\n"
            "    result: str\n"
            "\n"
            "class AnnotatedModule:\n"
            "    input_schema = InputModel\n"
            "    output_schema = OutputModel\n"
            "    description = 'mod'\n"
            "    annotations = ModuleAnnotations(readonly=True, idempotent=True)\n"
            "\n"
            "    def execute(self, inputs, context=None):\n"
            "        return {'result': inputs['value']}\n"
        )
        (ext / "annotated_meta.yaml").write_text(yaml.dump({"annotations": {"destructive": True}}))

        reg = Registry(extensions_dir=str(ext))
        reg.discover()
        defn = reg.get_definition("annotated")
        assert defn is not None
        assert isinstance(defn.annotations, ModuleAnnotations)
        # YAML-supplied flag is honored…
        assert defn.annotations.destructive is True
        # …and code-set flags are preserved (field-level merge, not whole-replace).
        assert defn.annotations.readonly is True
        assert defn.annotations.idempotent is True


# ===== clear_cache() =====


class TestClearCache:
    def test_clear_cache(self) -> None:
        """clear_cache() empties _schema_cache."""
        reg = Registry()
        reg._schema_cache["a"] = {"cached": True}
        reg._schema_cache["b"] = {"cached": True}
        reg.clear_cache()
        assert len(reg._schema_cache) == 0


# ===== Thread Safety =====


class TestThreadSafety:
    def test_concurrent_get(self) -> None:
        """Concurrent get() calls don't raise."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        errors: list[Exception] = []

        def reader() -> None:
            try:
                for _ in range(100):
                    reg.get("test.module")
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=reader) for _ in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        assert errors == []

    def test_concurrent_register_no_corruption(self) -> None:
        """Concurrent register() of same ID: one succeeds, others raise."""
        reg = Registry()
        successes: list[str] = []
        failures: list[str] = []

        def registerer(idx: int) -> None:
            try:
                reg.register("shared.module", _ValidModule())
                successes.append(f"thread-{idx}")
            except InvalidInputError:
                failures.append(f"thread-{idx}")

        threads = [threading.Thread(target=registerer, args=(i,)) for i in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        assert len(successes) == 1
        assert len(failures) == 9

    def test_list_returns_snapshot(self) -> None:
        """list() returns snapshot not affected by concurrent mutations."""
        reg = Registry()
        for i in range(10):
            reg.register(f"mod.n{i:02d}", _ValidModule())
        snapshot = reg.list()
        assert len(snapshot) == 10
        # Unregister in another thread
        reg.unregister("mod.n05")
        # Original snapshot is still 10 items
        assert len(snapshot) == 10

    def test_concurrent_has_and_register(self) -> None:
        """Concurrent has() and register() should not raise."""
        reg = Registry()
        errors: list[Exception] = []

        def registerer() -> None:
            try:
                for i in range(50):
                    mid = f"thread.mod.w{threading.current_thread().name}.n{i}"
                    reg.register(mid, _ValidModule())
            except Exception as e:
                errors.append(e)

        def reader() -> None:
            try:
                for _ in range(200):
                    reg.has("thread.mod.n0")
                    reg.count  # noqa: B018
                    reg.module_ids
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=registerer, name=f"w{i}") for i in range(3)]
        threads += [threading.Thread(target=reader) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []

    def test_concurrent_iter_and_unregister(self) -> None:
        """iter() snapshot is safe during concurrent unregister()."""
        reg = Registry()
        for i in range(20):
            reg.register(f"mod.n{i:02d}", _ValidModule())
        errors: list[Exception] = []

        def iterator() -> None:
            try:
                for _ in range(50):
                    list(reg.iter())
            except Exception as e:
                errors.append(e)

        def unregisterer() -> None:
            try:
                for i in range(20):
                    reg.unregister(f"mod.n{i:02d}")
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=iterator) for _ in range(5)]
        threads.append(threading.Thread(target=unregisterer))
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []


# ===== describe() =====


class _ModuleWithCustomDescribe:
    """Module with a custom describe() method."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "Module with custom describe"

    def execute(self, _inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": "ok"}

    def describe(self) -> str:
        return "Custom description from the module itself."


class _ModuleWithStructuredDescribe:
    """Module whose describe() returns the §5.6 introspection MAPPING.

    This is the declared shape of the optional ``Module.describe()``; per D-77
    the registry must not pass it through a string-typed interface, nor
    stringify it.
    """

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "Module with a structured describe"
    tags = ["structured"]

    def execute(self, _inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": "ok"}

    def describe(self) -> dict[str, Any]:
        return {
            "description": self.description,
            "input_schema": {"type": "object"},
            "output_schema": {"type": "object"},
            "annotations": {},
        }


class _ModuleWithDocumentation:
    """Module with documentation for auto-generated describe."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "A documented module"
    tags = ["util", "math"]
    documentation = "This module does interesting things.\nSee docs for details."

    def execute(self, inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": inputs["value"]}


class TestDescribe:
    def test_describe_custom_method(self) -> None:
        """describe() calls module.describe() when it exists."""
        reg = Registry()
        mod = _ModuleWithCustomDescribe()
        reg.register("test.custom", mod)
        result = reg.describe("test.custom")
        assert result == "Custom description from the module itself."

    def test_describe_auto_generated(self) -> None:
        """describe() auto-generates markdown from descriptor when no custom method."""
        reg = Registry()
        mod = _ValidModule()
        reg.register("test.auto", mod)
        result = reg.describe("test.auto")
        assert "# test.auto" in result
        assert "A valid test module" in result
        assert "**Tags:** test, sample" in result
        assert "**Parameters:**" in result
        assert "`value`" in result

    def test_describe_includes_documentation(self) -> None:
        """describe() includes documentation section when available."""
        reg = Registry()
        mod = _ModuleWithDocumentation()
        reg.register("test.documented", mod)
        result = reg.describe("test.documented")
        assert "**Documentation:**" in result
        assert "This module does interesting things." in result

    def test_describe_not_found_raises(self) -> None:
        """describe() raises ModuleNotFoundError for unregistered module."""
        reg = Registry()
        with pytest.raises(ModuleNotFoundError):
            reg.describe("nonexistent.module")

    def test_describe_string_override_returned_verbatim(self) -> None:
        """D-77 rule 1: a string describe() override is the author's own text."""
        reg = Registry()
        reg.register("test.string_override", _ModuleWithCustomDescribe())
        assert reg.describe("test.string_override") == "Custom description from the module itself."

    def test_describe_structured_override_falls_through_to_envelope(self) -> None:
        """D-77 rule 2: a mapping return is NOT the description.

        Previously this returned ``str(dict)`` — a Python repr, which is
        neither a description nor portable. The registry must fall through to
        the generated envelope instead.
        """
        reg = Registry()
        reg.register("test.structured_override", _ModuleWithStructuredDescribe())
        result = reg.describe("test.structured_override")

        assert isinstance(result, str)
        # The generated envelope, not the override.
        assert result.startswith("# test.structured_override")
        assert "Module with a structured describe" in result
        assert "**Tags:** structured" in result
        # No Python repr of the mapping leaked into the description.
        assert "{" not in result
        assert "'input_schema'" not in result

    def test_describe_without_override_uses_envelope(self) -> None:
        """D-77 rule 2: no describe() at all also yields the generated envelope."""
        mod = _ValidModule()
        assert not hasattr(mod, "describe"), "this case is about a module that has no override"

        reg = Registry()
        reg.register("test.no_override", mod)
        result = reg.describe("test.no_override")
        assert result.startswith("# test.no_override")
        assert "A valid test module" in result


# ===== export_schema =====


class TestExportSchema:
    def test_export_schema_returns_dict_for_registered_module(self) -> None:
        """export_schema returns a dict with expected keys for a registered module."""
        reg = Registry()
        mod = _ValidModule()
        reg.register("test.export", mod)
        result = reg.export_schema("test.export")
        assert result is not None
        assert result["module_id"] == "test.export"
        assert result["description"] == "A valid test module"
        assert "input_schema" in result
        assert "output_schema" in result

    def test_export_schema_returns_none_for_unregistered_module(self) -> None:
        """export_schema returns None for a module that is not registered."""
        reg = Registry()
        result = reg.export_schema("not.registered")
        assert result is None

    def test_export_schema_strict_mode(self) -> None:
        """export_schema with strict=True applies strict schema constraints."""
        reg = Registry()
        mod = _ValidModule()
        reg.register("test.strict", mod)
        result = reg.export_schema("test.strict", strict=True)
        assert result is not None
        assert result["module_id"] == "test.strict"
        input_schema = result["input_schema"]
        # strict schema sets additionalProperties to False
        assert input_schema.get("additionalProperties") is False

    def test_export_schema_input_schema_properties(self) -> None:
        """export_schema includes correct input schema properties."""
        reg = Registry()
        mod = _ValidModule()
        reg.register("test.props", mod)
        result = reg.export_schema("test.props")
        assert result is not None
        assert "properties" in result["input_schema"]
        assert "value" in result["input_schema"]["properties"]


# ===== Hot Reload (watch/unwatch) =====


class TestHotReload:
    def test_watch_raises_import_error_without_watchdog(self) -> None:
        """watch() raises ImportError when watchdog is not installed."""
        import sys
        from unittest.mock import patch

        reg = Registry()
        # Temporarily make watchdog unimportable
        with patch.dict(sys.modules, {"watchdog": None, "watchdog.observers": None, "watchdog.events": None}):
            with pytest.raises(ImportError, match="watchdog is required"):
                reg.watch()

    def test_unwatch_safe_when_not_watching(self) -> None:
        """unwatch() is safe to call when not watching."""
        reg = Registry()
        # Should not raise
        reg.unwatch()
        # Call again to verify idempotent
        reg.unwatch()

    def test_path_to_module_id_maps_basename(self) -> None:
        """_path_to_module_id() maps a file path to a module ID correctly."""
        reg = Registry()
        reg.register("my_module", _ValidModule())
        result = reg._path_to_module_id("/some/path/my_module.py")
        assert result == "my_module"

    def test_path_to_module_id_maps_with_namespace(self) -> None:
        """_path_to_module_id() maps a namespaced module correctly."""
        reg = Registry()
        reg.register("ns.my_module", _ValidModule())
        result = reg._path_to_module_id("/some/path/my_module.py")
        assert result == "ns.my_module"

    def test_path_to_module_id_returns_none_for_unknown(self) -> None:
        """_path_to_module_id() returns None for an unknown file."""
        reg = Registry()
        reg.register("my_module", _ValidModule())
        result = reg._path_to_module_id("/some/path/unknown_file.py")
        assert result is None

    def test_module_change_handler_is_module_level(self) -> None:
        """_ModuleChangeHandler is hoisted out of watch() so it is testable."""
        from apcore.registry.registry import _ModuleChangeHandler

        reg = Registry()
        handler = _ModuleChangeHandler(reg)
        # weakref holds the registry; direct debounce dict available for tests.
        assert handler._registry() is reg
        assert isinstance(handler._debounce_timer, dict)

    def test_module_change_handler_debounce_bounds_memory(self) -> None:
        """Debounce dict is pruned when it exceeds the bound."""
        from apcore.registry.registry import _ModuleChangeHandler

        reg = Registry()
        handler = _ModuleChangeHandler(reg)
        # Force enough entries past the bound by bypassing the time window:
        # _should_process returns False for repeats under the 0.3s window, so
        # for the memory-bound test we populate the dict directly.
        for i in range(_ModuleChangeHandler._MAX_DEBOUNCE_ENTRIES + 50):
            handler._debounce_timer[f"/path/{i}.py"] = float(i)
        # Now trigger a single _should_process call to exercise pruning.
        ok = handler._should_process(f"/path/new_{time.time()}.py")
        assert ok is True
        assert len(handler._debounce_timer) <= _ModuleChangeHandler._DEBOUNCE_PRUNE_KEEP + 1

    def test_handle_file_deletion_clears_versioned_stores(self) -> None:
        """_handle_file_deletion removes from both _modules and _versioned_modules."""
        reg = Registry()
        reg.register("my_module", _ValidModule())
        # register() already populated _versioned_modules.
        assert reg._versioned_modules.has("my_module")

        reg._handle_file_deletion("/some/path/my_module.py")

        assert "my_module" not in reg._modules
        assert not reg._versioned_modules.has("my_module")

    def test_handle_file_deletion_unregisters_module(self) -> None:
        """_handle_file_deletion() unregisters a known module."""
        reg = Registry()
        mod = _ModuleWithOnUnload()
        reg.register("deletable", mod)
        assert reg.has("deletable")

        reg._handle_file_deletion("/extensions/deletable.py")

        assert not reg.has("deletable")
        assert mod.unload_called is True

    def test_handle_file_deletion_no_op_for_unknown(self) -> None:
        """_handle_file_deletion() does nothing for an unknown file."""
        reg = Registry()
        reg.register("existing", _ValidModule())
        # Should not raise, should not affect existing modules
        reg._handle_file_deletion("/some/path/unknown.py")
        assert reg.has("existing")


# ===== Custom Discoverer =====


class _MockDiscoverer:
    """A mock discoverer for testing."""

    def __init__(self, modules: list[dict[str, Any]]) -> None:
        self.modules = modules
        self.called_with: list[str] | None = None

    def discover(self, roots: list[str]) -> list[dict[str, Any]]:
        self.called_with = roots
        return self.modules


class TestCustomDiscoverer:
    def test_custom_discoverer_used(self) -> None:
        """When a custom discoverer is set, discover() uses it."""
        mod_a = _ValidModule()
        mod_b = _ValidModuleB()
        discoverer = _MockDiscoverer(
            [
                {"module_id": "custom.a", "module": mod_a},
                {"module_id": "custom.b", "module": mod_b},
            ]
        )

        reg = Registry()
        reg.set_discoverer(discoverer)
        count = reg.discover()

        assert count == 2
        assert reg.has("custom.a")
        assert reg.has("custom.b")
        assert reg.get("custom.a") is mod_a
        assert reg.get("custom.b") is mod_b
        assert discoverer.called_with == ["./extensions"]

    def test_default_discoverer_used_when_none_set(self, tmp_path: Path) -> None:
        """When no custom discoverer is set, default scanning is used."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "default_mod.py", "DefaultModModule", "Default module")

        reg = Registry(extensions_dir=str(ext))
        count = reg.discover()

        assert count == 1
        assert reg.has("default_mod")

    def test_malformed_entries_skipped(self, caplog: pytest.LogCaptureFixture) -> None:
        """Malformed entries from custom discoverer are skipped, valid ones still register."""
        mod_ok = _ValidModule()
        discoverer = _MockDiscoverer(
            [
                {"module": mod_ok},  # missing module_id
                {"module_id": "only_id"},  # missing module
                "not_a_dict",  # wrong type
                None,  # None entry
                {"module_id": "good.mod", "module": mod_ok},
            ]
        )

        reg = Registry()
        reg.set_discoverer(discoverer)
        with caplog.at_level(logging.WARNING):
            count = reg.discover()

        assert count == 1
        assert reg.has("good.mod")
        # All four malformed entries should have logged a warning
        malformed_warnings = [r for r in caplog.records if "Malformed entry" in r.message]
        assert len(malformed_warnings) == 4

    def test_ephemeral_custom_discovered_module_skipped(self, caplog: pytest.LogCaptureFixture) -> None:
        """A-D-015: a custom discoverer returning an ephemeral.* module must be
        skipped (not registered) with a warning, mirroring the filesystem path
        and apcore-typescript. Valid non-ephemeral entries still register."""
        mod_ok = _ValidModule()
        mod_ephemeral = _ValidModuleB()
        discoverer = _MockDiscoverer(
            [
                {"module_id": "ephemeral.session.tmp", "module": mod_ephemeral},
                {"module_id": "good.mod", "module": mod_ok},
            ]
        )

        reg = Registry()
        reg.set_discoverer(discoverer)
        with caplog.at_level(logging.WARNING):
            count = reg.discover()

        assert count == 1
        assert reg.has("good.mod")
        assert not reg.has("ephemeral.session.tmp")
        ephemeral_warnings = [
            r for r in caplog.records if "ephemeral.session.tmp" in r.message and "ephemeral.*" in r.message
        ]
        assert len(ephemeral_warnings) == 1

    def test_non_iterable_discover_result_returns_zero(self, caplog: pytest.LogCaptureFixture) -> None:
        """A-D-014: a custom discoverer returning a non-list (e.g. ``None``)
        must yield ``discover() == 0`` and a warning, not an uncaught
        ``TypeError``, matching apcore-typescript's ``!Array.isArray`` guard.
        """

        class _NoneDiscoverer:
            def discover(self, roots: list[str]) -> Any:  # noqa: ARG002
                return None

        reg = Registry()
        reg.set_discoverer(_NoneDiscoverer())
        with caplog.at_level(logging.WARNING):
            count = reg.discover()

        assert count == 0
        non_array_warnings = [r for r in caplog.records if "non-list" in r.message or "non-array" in r.message]
        assert len(non_array_warnings) == 1

    def test_scalar_discover_result_returns_zero(self, caplog: pytest.LogCaptureFixture) -> None:
        """A-D-014: a custom discoverer returning a scalar must also fail
        gracefully (warn + return 0) rather than iterating a non-iterable.
        """

        class _ScalarDiscoverer:
            def discover(self, roots: list[str]) -> Any:  # noqa: ARG002
                return 42

        reg = Registry()
        reg.set_discoverer(_ScalarDiscoverer())
        with caplog.at_level(logging.WARNING):
            count = reg.discover()

        assert count == 0


# ===== Custom Validator =====


class _RejectAllValidator:
    """Validator that rejects all modules."""

    def validate(self, module: Any) -> list[str]:
        return ["rejected by custom validator"]


class _AcceptAllValidator:
    """Validator that accepts all modules."""

    def validate(self, module: Any) -> list[str]:
        return []


class TestCustomValidator:
    def test_custom_validator_rejects_module(self) -> None:
        """When custom validator returns errors, modules are rejected."""
        mod = _ValidModule()
        discoverer = _MockDiscoverer(
            [
                {"module_id": "rejected.mod", "module": mod},
            ]
        )

        reg = Registry()
        reg.set_discoverer(discoverer)
        reg.set_validator(_RejectAllValidator())
        count = reg.discover()

        assert count == 0
        assert not reg.has("rejected.mod")

    def test_custom_validator_accepts_module(self) -> None:
        """When custom validator returns empty list, modules are accepted."""
        mod = _ValidModule()
        discoverer = _MockDiscoverer(
            [
                {"module_id": "accepted.mod", "module": mod},
            ]
        )

        reg = Registry()
        reg.set_discoverer(discoverer)
        reg.set_validator(_AcceptAllValidator())
        count = reg.discover()

        assert count == 1
        assert reg.has("accepted.mod")
        assert reg.get("accepted.mod") is mod

    def test_custom_validator_with_default_discover(self, tmp_path: Path) -> None:
        """Custom validator works with default file-system discovery too."""
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "val_mod.py", "ValModModule", "Validated module")

        reg = Registry(extensions_dir=str(ext))
        reg.set_validator(_RejectAllValidator())
        count = reg.discover()

        # The custom validator rejects all, so nothing should be registered
        assert count == 0
        assert not reg.has("val_mod")

    def test_custom_validator_called_exactly_once_per_module_in_custom_discovery(self) -> None:
        """D11-006: Custom validator must run exactly ONCE per module in the
        custom-discovery path. Previously _discover_custom validated the
        module inline (line 442) AND the inner register() call ran the
        validator again (line 884), so stateful validators (e.g. one that
        revokes after N approvals) yielded different acceptance counts in
        Python vs TypeScript (TS validates once via _registerImpl which
        skips the validator).
        """

        class _CountingValidator:
            def __init__(self) -> None:
                self.calls = 0

            def validate(self, module: Any) -> list[str]:
                self.calls += 1
                return []

        mod_a = _ValidModule()
        mod_b = _ValidModuleB()
        discoverer = _MockDiscoverer(
            [
                {"module_id": "counted.a", "module": mod_a},
                {"module_id": "counted.b", "module": mod_b},
            ]
        )
        validator = _CountingValidator()

        reg = Registry()
        reg.set_discoverer(discoverer)
        reg.set_validator(validator)
        count = reg.discover()

        assert count == 2, "both modules should register"
        assert validator.calls == 2, (
            f"validator.validate must be called exactly once per module " f"(got {validator.calls} calls for 2 modules)"
        )


# ===== Registry Constants =====


class TestRegistryConstants:
    def test_max_module_id_length_matches_spec(self) -> None:
        """MAX_MODULE_ID_LENGTH matches PROTOCOL_SPEC §2.7 EBNF constraint #1 (192).

        Filesystem-safe: 192 + len('.binding.yaml')=13 = 205 < 255-byte filename limit.
        Bumped from 128 to accommodate Java/.NET deep-namespace FQN-derived IDs.
        """
        assert MAX_MODULE_ID_LENGTH == 192

    def test_max_module_id_length_is_int(self) -> None:
        """MAX_MODULE_ID_LENGTH is an integer."""
        assert isinstance(MAX_MODULE_ID_LENGTH, int)

    def test_reserved_words_is_frozenset(self) -> None:
        """RESERVED_WORDS is a frozenset."""
        assert isinstance(RESERVED_WORDS, frozenset)

    def test_reserved_words_contains_expected_values(self) -> None:
        """RESERVED_WORDS contains all expected reserved words."""
        expected = {"system", "internal", "core", "apcore", "plugin", "schema", "acl"}
        assert RESERVED_WORDS == expected

    def test_reserved_words_rejects_reserved_module_id(self) -> None:
        """Registry.register() rejects module IDs containing reserved words."""
        reg = Registry()
        with pytest.raises(InvalidInputError, match="reserved word"):
            reg.register("system", _ValidModule())

    def test_reserved_words_rejects_reserved_segment(self) -> None:
        """Registry.register() rejects module IDs with a reserved word as the first segment."""
        reg = Registry()
        with pytest.raises(InvalidInputError, match="reserved word"):
            reg.register("internal.my.module", _ValidModule())

    def test_reserved_words_allows_middle_segment(self) -> None:
        """Registry.register() allows module IDs with a reserved word in middle segments."""
        reg = Registry()
        reg.register("my.internal.module", _ValidModule())
        assert reg.has("my.internal.module")

    def test_max_module_id_length_enforced(self) -> None:
        """Registry.register() rejects module IDs exceeding MAX_MODULE_ID_LENGTH."""
        reg = Registry()
        long_id = "a" * (MAX_MODULE_ID_LENGTH + 1)
        with pytest.raises(InvalidInputError, match="maximum length"):
            reg.register(long_id, _ValidModule())

    def test_module_id_at_max_length_accepted(self) -> None:
        """Registry.register() accepts module IDs exactly at MAX_MODULE_ID_LENGTH."""
        reg = Registry()
        # Build a valid ID of exactly MAX_MODULE_ID_LENGTH characters
        # Pattern requires: starts with [a-z], then [a-z0-9_]
        exact_id = "a" * MAX_MODULE_ID_LENGTH
        reg.register(exact_id, _ValidModule())
        assert reg.has(exact_id)


# ===== register_internal — bypasses ONLY reserved word check =====
# (parity with apcore-typescript and apcore-rust)


class TestRegisterInternalValidation:
    def test_register_internal_accepts_reserved_first_segment(self) -> None:
        reg = Registry()
        reg.register_internal("system.health", _ValidModule())
        assert reg.has("system.health")

    def test_register_internal_accepts_reserved_any_segment(self) -> None:
        reg = Registry()
        reg.register_internal("myapp.system.config", _ValidModule())
        assert reg.has("myapp.system.config")

    def test_register_internal_still_rejects_empty(self) -> None:
        reg = Registry()
        with pytest.raises(InvalidInputError, match="non-empty"):
            reg.register_internal("", _ValidModule())

    def test_register_internal_still_rejects_invalid_pattern(self) -> None:
        reg = Registry()
        for bad_id in ["INVALID-ID", "1abc", "Module", "a..b", ".leading", "trailing.", "has space"]:
            with pytest.raises(InvalidInputError, match="Invalid module ID|Must match pattern"):
                reg.register_internal(bad_id, _ValidModule())

    def test_register_internal_still_rejects_over_length(self) -> None:
        reg = Registry()
        overlong = "a" * (MAX_MODULE_ID_LENGTH + 1)
        with pytest.raises(InvalidInputError, match="maximum length"):
            reg.register_internal(overlong, _ValidModule())

    def test_register_internal_rejects_duplicate(self) -> None:
        reg = Registry()
        reg.register_internal("system.dup", _ValidModule())
        with pytest.raises(InvalidInputError, match="already registered"):
            reg.register_internal("system.dup", _ValidModule())

    # D11-001: register_internal intentionally skips _versioned_modules /
    # _versioned_meta inserts because sys/internal modules are not
    # version-tracked. Document the asymmetry: get(sys_id) without a hint
    # works (falls through to _modules), but get(sys_id, version_hint=...)
    # must raise a clear error rather than silently returning None — the
    # caller asked for version resolution on a module that explicitly
    # opted out of version tracking.

    def test_register_internal_get_without_hint_returns_module(self) -> None:
        reg = Registry()
        mod = _ValidModule()
        reg.register_internal("system.unversioned", mod)
        # No hint: falls through to _modules.
        assert reg.get("system.unversioned") is mod

    def test_register_internal_get_with_version_hint_raises_clear_error(self) -> None:
        reg = Registry()
        reg.register_internal("system.unversioned", _ValidModule())
        with pytest.raises(ModuleNotFoundError, match="not version-tracked"):
            reg.get("system.unversioned", version_hint="1.0.0")


class TestDiscoveryValidatesModuleIds:
    """PROTOCOL_SPEC §2.7 module-ID grammar applies to the discovery path too.

    ``_validate_module_id`` was reachable from ``register()`` and
    ``register_internal()`` only. The filesystem-discovery chain ran A03
    conflict detection (duplicate / reserved / case) and nothing else, so no
    grammar or length check stood between a scanned or ID-map-overridden name
    and ``_modules``. ``_apply_id_map_overrides`` is the concrete vector: it
    rewrites ``canonical_id`` from the YAML map *after* scanning, with no
    re-validation.

    Hyphens are banned so MCP / OpenAI tool-name normalisation stays bijective.
    apcore-typescript validates post-override in ``_filterIdConflicts``;
    apcore-rust validates per entry. Both skip the offender with a warning
    rather than failing the whole discovery run.
    """

    @staticmethod
    def _discover_with_id_map(tmp_path: Path, override_id: str, caplog: pytest.LogCaptureFixture) -> Registry:
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "hello.py", "HelloModule", "Hello module")
        id_map = tmp_path / "id_map.yaml"
        id_map.write_text(yaml.dump({"mappings": [{"file": "hello.py", "id": override_id}]}))
        reg = Registry(extensions_dir=str(ext), id_map_path=str(id_map))
        with caplog.at_level(logging.WARNING):
            reg.discover()
        return reg

    @pytest.mark.parametrize(
        ("case", "bad_id"),
        [
            ("hyphen", "foo-bar"),
            ("uppercase", "Foo.Bar"),
            ("leading_digit_segment", "1foo"),
            ("space", "foo bar"),
            ("slash", "foo/bar"),
            ("trailing_dot", "foo."),
            ("too_long", "a" * 201),
        ],
    )
    def test_id_map_override_with_an_invalid_id_is_skipped(
        self, case: str, bad_id: str, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        reg = self._discover_with_id_map(tmp_path, bad_id, caplog)
        assert not reg.has(bad_id), f"invalid ID {bad_id!r} reached the registry"
        assert bad_id not in reg._modules
        assert any(bad_id in r.getMessage() for r in caplog.records)

    def test_id_map_override_with_a_valid_id_still_registers(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        reg = self._discover_with_id_map(tmp_path, "custom.hello", caplog)
        assert reg.has("custom.hello")

    def test_one_invalid_id_does_not_abort_the_whole_discovery_run(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        ext = tmp_path / "extensions"
        ext.mkdir()
        _write_module_file(ext / "good.py", "GoodModule", "Good module")
        _write_module_file(ext / "bad.py", "BadModule", "Bad module")
        id_map = tmp_path / "id_map.yaml"
        id_map.write_text(yaml.dump({"mappings": [{"file": "bad.py", "id": "not-valid"}]}))
        reg = Registry(extensions_dir=str(ext), id_map_path=str(id_map))
        with caplog.at_level(logging.WARNING):
            count = reg.discover()
        assert count == 1
        assert reg.has("good")
        assert not reg.has("not-valid")

    def test_reserved_first_segment_from_the_id_map_is_skipped(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        # `register()` rejects a reserved first segment; discovery must too.
        reg = self._discover_with_id_map(tmp_path, "system.hello", caplog)
        assert not reg.has("system.hello")


class TestRegisterInternalParity:
    """``register_internal`` bypasses **only** the reserved-word check.

    Its docstring has always said so; two checks disagreed.

    * The ``ephemeral.*`` rejection raised a bare builtin ``ValueError`` while
      every other rejection in the same method raises ``InvalidInputError``.
      With no error code attached, no conformance fixture could cover the path.
      apcore-typescript throws ``InvalidInputError``; apcore-rust returns
      ``ModuleError(GeneralInvalidInput)``.
    * The streaming-annotation ⇔ streaming-interface check (Issue #62) ran on
      ``register()`` only. apcore-rust keeps it on both paths — its
      ``register_internal`` delegates to the shared ``register_core``, which
      performs the check.
    """

    class _StreamingLiar:
        """Declares streaming=True and provides no stream() at all."""

        input_schema = None
        output_schema = None
        description = "Claims to stream"

        def __init__(self) -> None:
            from apcore.module import ModuleAnnotations

            self.annotations = ModuleAnnotations(streaming=True)

        def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
            return {}

    class _SyncStreamLiar(_StreamingLiar):
        """Declares streaming=True with a non-async stream()."""

        def stream(self, inputs: dict[str, Any], context: Any = None) -> Any:
            return iter(())

    def test_ephemeral_rejection_is_a_typed_invalid_input_error(self) -> None:
        reg = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            reg.register_internal("ephemeral.test_v1", _ValidModule())
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID
        assert "ephemeral" in str(exc_info.value)
        assert "Registry.register()" in str(exc_info.value)
        # The rejection still short-circuits before any state mutation.
        assert reg.get("ephemeral.test_v1") is None

    def test_bare_ephemeral_id_is_rejected_the_same_way(self) -> None:
        reg = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            reg.register_internal("ephemeral", _ValidModule())
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    @pytest.mark.parametrize("liar", ["missing_stream", "sync_stream"])
    def test_streaming_annotation_is_checked_on_the_internal_path(self, liar: str) -> None:
        from apcore.errors import StreamingInterfaceError

        module = self._StreamingLiar() if liar == "missing_stream" else self._SyncStreamLiar()
        reg = Registry()
        with pytest.raises(StreamingInterfaceError):
            reg.register_internal("system.streaming_liar", module)
        assert reg.get("system.streaming_liar") is None

    def test_register_raises_the_same_streaming_error(self) -> None:
        from apcore.errors import StreamingInterfaceError

        reg = Registry()
        with pytest.raises(StreamingInterfaceError):
            reg.register("plain.streaming_liar", self._StreamingLiar())

    def test_a_real_streaming_module_still_registers_internally(self) -> None:
        from apcore.module import ModuleAnnotations

        class _RealStreamer:
            input_schema = None
            output_schema = None
            description = "Streams for real"
            annotations = ModuleAnnotations(streaming=True)

            def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
                return {}

            async def stream(self, inputs: dict[str, Any], context: Any = None) -> Any:
                yield {}

        reg = Registry()
        reg.register_internal("system.real_streamer", _RealStreamer())
        assert reg.has("system.real_streamer")

    def test_reserved_first_segment_is_still_permitted(self) -> None:
        reg = Registry()
        reg.register_internal("system.health", _ValidModule())
        assert reg.has("system.health")


class TestRegisterVersionArgumentReachesDescriptor:
    """`register(id, module, version=...)` must reach `get_definition().version`.

    Sync finding A-D-002. `merge_module_metadata` was called with the caller's
    `metadata` mapping alone, so the `version` ARGUMENT never entered the merged
    view and `get_definition()` reported the module attribute (or "1.0.0")
    instead. apcore-typescript and apcore-rust both surfaced "2.0.0" for the
    same call.

    A second-order effect rode along: `get_definition` looked the versioned
    metadata up under `getattr(module, "version")` rather than the version the
    module was registered with, so when `version=` named a version the module
    object does not carry, the caller's metadata was dropped from the descriptor
    too.
    """

    class _Mod:
        description = "d"
        input_schema = {"type": "object"}
        output_schema = {"type": "object"}

        async def execute(self, inputs, ctx):  # noqa: ANN001, ANN201
            return {}

    def test_version_argument_reaches_get_definition(self) -> None:
        from apcore import Registry

        registry = Registry()
        registry.register("math.add", self._Mod(), version="2.0.0")
        assert registry.get_definition("math.add").version == "2.0.0"

    def test_metadata_survives_alongside_an_explicit_version(self) -> None:
        from apcore import Registry

        registry = Registry()
        registry.register("math.add", self._Mod(), version="2.0.0", metadata={"foo": 1})
        descriptor = registry.get_definition("math.add")
        assert descriptor.version == "2.0.0"
        assert descriptor.metadata.get("foo") == 1, "the caller's metadata must not vanish when `version=` is supplied"

    def test_module_attribute_still_wins_when_no_version_argument(self) -> None:
        from apcore import Registry

        class Versioned(self._Mod):
            version = "3.1.0"

        registry = Registry()
        registry.register("math.sub", Versioned())
        assert registry.get_definition("math.sub").version == "3.1.0"

    def test_default_when_neither_is_supplied(self) -> None:
        from apcore import Registry

        registry = Registry()
        registry.register("math.mul", self._Mod())
        assert registry.get_definition("math.mul").version == "1.0.0"


class TestDescriptorCarriesTypedDependencies:
    """`get_definition().dependencies` is a typed field on all three SDKs.

    Sync finding A-D-004. PROTOCOL_SPEC §12.2 requires a `dependencies` entry
    in `metadata` to reach the registered module's descriptor. apcore-rust
    carried a typed `Vec<DependencyInfo>`; this SDK surfaced only the unparsed
    list nested under `descriptor.metadata`, and apcore-typescript surfaced it
    on neither.

    Promoting it matters beyond the accessor name: `metadata` is specified as
    "arbitrary extension metadata" — the `x-` layer of the three-layer model —
    while dependencies are structural data the framework consumes for load and
    reload ordering. Left in the extension bag, the
    `{module_id, version?, optional?}` parse falls on every caller;
    `sys_modules/control.py` imported `parse_dependencies` inside its reload
    function to do exactly that.
    """

    class _Mod:
        description = "d"
        input_schema = {"type": "object"}
        output_schema = {"type": "object"}

        async def execute(self, inputs, ctx):  # noqa: ANN001, ANN201
            return {}

    def test_dependencies_are_parsed_into_typed_objects(self) -> None:
        from apcore import Registry
        from apcore.registry.types import DependencyInfo

        registry = Registry()
        registry.register(
            "a.b",
            self._Mod(),
            metadata={"dependencies": [{"module_id": "a.dep", "version": "^1.0", "optional": True}]},
        )
        deps = registry.get_definition("a.b").dependencies
        assert deps == [DependencyInfo(module_id="a.dep", version="^1.0", optional=True)]
        assert isinstance(deps[0], DependencyInfo), "callers must not have to parse raw JSON"

    def test_absent_dependencies_is_an_empty_list_not_none(self) -> None:
        from apcore import Registry

        registry = Registry()
        registry.register("a.c", self._Mod())
        assert registry.get_definition("a.c").dependencies == []

    def test_reload_ordering_accessor_still_agrees(self) -> None:
        # The descriptor field and get_module_metadata() must not disagree —
        # reload ordering reads the latter, and a split between the two is the
        # hazard §12.2 describes.
        from apcore import Registry

        registry = Registry()
        registry.register("a.d", self._Mod(), metadata={"dependencies": [{"module_id": "a.dep"}]})
        from_descriptor = [d.module_id for d in registry.get_definition("a.d").dependencies]
        raw = registry.get_module_metadata("a.d").get("dependencies") or []
        from_metadata = [d["module_id"] for d in raw]
        assert from_descriptor == from_metadata == ["a.dep"]


# ===== D-86: register() validation order =====


class TestRegisterValidationOrder:
    """`register` reports validation failures in a fixed order (D-86, v1.49.0).

    registry-system.md "Side Effects (ordered)" pins it as
    ``module_id`` → structure/streaming → custom validator → duplicate:
    intrinsic-then-extrinsic, because what is wrong with the module itself
    must be fixed either way, whereas a duplicate ID may just mean the author
    picked the wrong name.

    Python previously ran the custom validator BEFORE the streaming check, so
    a module that was both streaming-malformed and validator-rejected reported
    a different error here than apcore-rust and apcore-typescript did.
    """

    class _StreamingLiar:
        """Declares streaming=True and provides no stream()."""

        input_schema = None
        output_schema = None
        description = "Claims to stream"

        def __init__(self) -> None:
            from apcore.module import ModuleAnnotations

            self.annotations = ModuleAnnotations(streaming=True)

        def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
            return {}

    def test_streaming_failure_is_reported_before_the_custom_validator(self) -> None:
        from apcore.errors import StreamingInterfaceError

        reg = Registry()
        reg.set_validator(_RejectAllValidator())

        with pytest.raises(StreamingInterfaceError):
            reg.register("order.streaming_and_validator", self._StreamingLiar())

    def test_streaming_failure_is_reported_before_the_duplicate_check(self) -> None:
        """Malformed, validator-rejected AND duplicate at once: structure wins."""
        from apcore.errors import StreamingInterfaceError

        reg = Registry()
        reg.register("order.all_three", _ValidModule())
        reg.set_validator(_RejectAllValidator())

        with pytest.raises(StreamingInterfaceError):
            reg.register("order.all_three", self._StreamingLiar())

        # The incumbent registration is untouched.
        assert isinstance(reg.get("order.all_three"), _ValidModule)

    def test_custom_validator_is_reported_before_the_duplicate_check(self) -> None:
        reg = Registry()
        reg.register("order.validator_and_dup", _ValidModule())
        reg.set_validator(_RejectAllValidator())

        with pytest.raises(InvalidInputError) as exc_info:
            reg.register("order.validator_and_dup", _ValidModule())

        assert "Custom validator rejected" in str(exc_info.value)
        assert exc_info.value.code != ErrorCodes.DUPLICATE_MODULE_ID

    def test_module_id_failure_is_reported_before_everything_else(self) -> None:
        reg = Registry()
        reg.set_validator(_RejectAllValidator())

        with pytest.raises(InvalidInputError) as exc_info:
            reg.register("Invalid-ID", self._StreamingLiar())

        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_a_stateful_validator_still_runs_for_a_later_duplicate_rejection(self) -> None:
        """Stated side effect of the order: the validator runs, then the
        duplicate check rejects. A stateful validator therefore sees the
        registration that never landed."""

        class _CountingValidator:
            def __init__(self) -> None:
                self.calls = 0

            def validate(self, module: Any) -> list[str]:
                self.calls += 1
                return []

        validator = _CountingValidator()
        reg = Registry()
        reg.register("order.counting", _ValidModule())
        reg.set_validator(validator)

        with pytest.raises(InvalidInputError) as exc_info:
            reg.register("order.counting", _ValidModule())

        assert exc_info.value.code == ErrorCodes.DUPLICATE_MODULE_ID
        assert validator.calls == 1
