"""Shared pytest fixtures for the registry test suite."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest
import yaml
from pydantic import BaseModel

from apcore.registry.registry import Registry


# ---------------------------------------------------------------------------
# Helper Pydantic models
# ---------------------------------------------------------------------------


class _FixtureInput(BaseModel):
    """Input schema for fixture modules."""

    value: str


class _FixtureOutput(BaseModel):
    """Output schema for fixture modules."""

    result: str


# ---------------------------------------------------------------------------
# Module classes for fixtures
# ---------------------------------------------------------------------------


class _SampleModule:
    """A valid duck-typed module with all expected attributes."""

    input_schema = _FixtureInput
    output_schema = _FixtureOutput
    description = "Sample fixture module"
    name = "SampleModule"
    version = "1.0.0"
    tags = ["fixture", "sample"]

    def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
        return {"result": inputs["value"]}


class _InvalidModule:
    """A class that does NOT implement the module interface correctly."""

    description = ""
    # Missing input_schema, output_schema, execute


# ---------------------------------------------------------------------------
# Module file template
# ---------------------------------------------------------------------------

_MODULE_TEMPLATE = """\
from pydantic import BaseModel

class TestInput(BaseModel):
    value: str

class TestOutput(BaseModel):
    result: str

class {class_name}:
    input_schema = TestInput
    output_schema = TestOutput
    description = "{description}"
    tags = {tags}

    def execute(self, inputs, context=None):
        return {{"result": inputs["value"]}}
"""


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def sample_module_class() -> type:
    """Return a valid module class for testing."""
    return _SampleModule


@pytest.fixture
def invalid_module_class() -> type:
    """Return an invalid module class (missing required attributes)."""
    return _InvalidModule


@pytest.fixture
def tmp_extensions_dir(tmp_path: Path) -> Path:
    """Create a temp extensions dir with sample module files."""
    ext = tmp_path / "extensions"
    ext.mkdir()

    (ext / "hello.py").write_text(
        _MODULE_TEMPLATE.format(class_name="HelloModule", description="Hello module", tags='["hello"]')
    )
    (ext / "greet.py").write_text(
        _MODULE_TEMPLATE.format(class_name="GreetModule", description="Greet module", tags='["greet"]')
    )

    sub = ext / "sub"
    sub.mkdir()
    (sub / "nested.py").write_text(
        _MODULE_TEMPLATE.format(class_name="NestedModule", description="Nested module", tags='["nested"]')
    )

    return ext


@pytest.fixture
def registry(tmp_extensions_dir: Path) -> Registry:
    """Create a Registry pointed at tmp_extensions_dir (discover NOT called)."""
    return Registry(extensions_dir=str(tmp_extensions_dir))


@pytest.fixture
def meta_yaml(tmp_path: Path) -> Path:
    """Create a sample _meta.yaml file and return its path."""
    meta = {
        "description": "Overridden description from YAML",
        "tags": ["yaml-tag", "override"],
        "version": "2.0.0",
        "dependencies": [{"module_id": "some.dependency", "optional": True}],
    }
    path = tmp_path / "sample_meta.yaml"
    path.write_text(yaml.dump(meta, default_flow_style=False))
    return path
