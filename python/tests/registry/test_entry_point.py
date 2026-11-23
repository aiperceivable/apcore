"""Tests for entry point resolution: resolve_entry_point() and snake_to_pascal()."""

from __future__ import annotations

from pathlib import Path

import pytest

from apcore.errors import ModuleLoadError
from apcore.registry.entry_point import resolve_entry_point, snake_to_pascal


# --- Module file content templates ---

VALID_MODULE = """
from pydantic import BaseModel

class InputModel(BaseModel):
    value: str

class OutputModel(BaseModel):
    result: str

class MyTestModule:
    input_schema = InputModel
    output_schema = OutputModel
    description = "A test module"

    def execute(self, inputs, context=None):
        return {"result": inputs["value"]}
"""

TWO_MODULES = """
from pydantic import BaseModel

class InputA(BaseModel):
    x: str

class OutputA(BaseModel):
    y: str

class ModuleA:
    input_schema = InputA
    output_schema = OutputA
    description = "Module A"
    def execute(self, inputs, context=None):
        return {}

class InputB(BaseModel):
    a: int

class OutputB(BaseModel):
    b: int

class ModuleB:
    input_schema = InputB
    output_schema = OutputB
    description = "Module B"
    def execute(self, inputs, context=None):
        return {}
"""

NO_MODULE = """
class PlainClass:
    pass

class AnotherPlain:
    x = 10
"""

MODULE_PLUS_PLAIN = """
from pydantic import BaseModel

class InputModel(BaseModel):
    value: str

class OutputModel(BaseModel):
    result: str

class MyModule:
    input_schema = InputModel
    output_schema = OutputModel
    description = "A test module"
    def execute(self, inputs, context=None):
        return {}

class PlainHelper:
    pass
"""


# === resolve_entry_point() auto-infer ===


class TestResolveEntryPointAutoInfer:
    def test_single_module_returns_class(self, tmp_path: Path) -> None:
        """Single module class in file returns that class."""
        f = tmp_path / "mymod.py"
        f.write_text(VALID_MODULE)
        cls = resolve_entry_point(f)
        assert cls.__name__ == "MyTestModule"

    def test_zero_modules_raises(self, tmp_path: Path) -> None:
        """File with no module classes raises ModuleLoadError."""
        f = tmp_path / "nomod.py"
        f.write_text(NO_MODULE)
        with pytest.raises(ModuleLoadError, match="No Module subclass"):
            resolve_entry_point(f)

    def test_multiple_modules_raises(self, tmp_path: Path) -> None:
        """File with multiple module classes raises ModuleLoadError."""
        f = tmp_path / "multi.py"
        f.write_text(TWO_MODULES)
        with pytest.raises(ModuleLoadError, match="Ambiguous"):
            resolve_entry_point(f)

    def test_module_plus_plain_returns_module(self, tmp_path: Path) -> None:
        """Module class among plain classes is correctly identified."""
        f = tmp_path / "mixed.py"
        f.write_text(MODULE_PLUS_PLAIN)
        cls = resolve_entry_point(f)
        assert cls.__name__ == "MyModule"

    def test_imported_class_filtered_by_module_check(self, tmp_path: Path) -> None:
        """Classes imported from elsewhere are filtered out by __module__ check."""
        content = """
from pydantic import BaseModel
# BaseModel is imported, has class-level attributes but __module__ != this file
class Utility:
    pass
"""
        f = tmp_path / "noimport.py"
        f.write_text(content)
        with pytest.raises(ModuleLoadError, match="No Module subclass"):
            resolve_entry_point(f)


# === resolve_entry_point() meta override ===


class TestResolveEntryPointMeta:
    def test_meta_entry_point_loads_specific_class(self, tmp_path: Path) -> None:
        """Meta with entry_point loads the specified class."""
        f = tmp_path / "multi.py"
        f.write_text(TWO_MODULES)
        cls = resolve_entry_point(f, meta={"entry_point": "multi:ModuleB"})
        assert cls.__name__ == "ModuleB"

    def test_meta_nonexistent_class_raises(self, tmp_path: Path) -> None:
        """Meta referencing non-existent class raises ModuleLoadError."""
        f = tmp_path / "mymod.py"
        f.write_text(VALID_MODULE)
        with pytest.raises(ModuleLoadError, match="not found"):
            resolve_entry_point(f, meta={"entry_point": "mymod:NonExistentClass"})


# === resolve_entry_point() error handling ===


class TestResolveEntryPointErrors:
    def test_syntax_error_raises(self, tmp_path: Path) -> None:
        """File with syntax error raises ModuleLoadError."""
        f = tmp_path / "broken.py"
        f.write_text("def broken(")
        with pytest.raises(ModuleLoadError):
            resolve_entry_point(f)

    def test_import_error_raises(self, tmp_path: Path) -> None:
        """File with import error raises ModuleLoadError."""
        f = tmp_path / "badimport.py"
        f.write_text("import nonexistent_module_xyz_12345")
        with pytest.raises(ModuleLoadError):
            resolve_entry_point(f)

    def test_runtime_error_raises(self, tmp_path: Path) -> None:
        """File that raises at module level raises ModuleLoadError."""
        f = tmp_path / "boom.py"
        f.write_text('raise RuntimeError("boom")')
        with pytest.raises(ModuleLoadError):
            resolve_entry_point(f)


# === pre-approval hook ===


class TestPreApprovalHook:
    def test_hook_approves_by_returning(self, tmp_path: Path) -> None:
        f = tmp_path / "mymod.py"
        f.write_text(VALID_MODULE)
        called: list[Path] = []

        def approve(p: Path) -> None:
            called.append(p)

        cls = resolve_entry_point(f, pre_approval_hook=approve)
        assert cls.__name__ == "MyTestModule"
        assert called == [f]

    def test_hook_rejection_wraps_as_module_load_error(self, tmp_path: Path) -> None:
        f = tmp_path / "mymod.py"
        f.write_text(VALID_MODULE)

        def reject(p: Path) -> None:
            raise RuntimeError("not signed")

        with pytest.raises(ModuleLoadError, match="Pre-approval hook rejected"):
            resolve_entry_point(f, pre_approval_hook=reject)


# === snake_to_pascal() ===


class TestSnakeToPascal:
    def test_hello_world(self) -> None:
        assert snake_to_pascal("hello_world") == "HelloWorld"

    def test_single(self) -> None:
        assert snake_to_pascal("single") == "Single"

    def test_already_mixed(self) -> None:
        assert snake_to_pascal("already_Pascal") == "AlreadyPascal"

    def test_empty(self) -> None:
        assert snake_to_pascal("") == ""
