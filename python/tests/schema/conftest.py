"""Shared pytest fixtures for schema system tests."""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from apcore.config import Config
from apcore.schema.loader import SchemaLoader
from apcore.schema.ref_resolver import RefResolver
from apcore.schema.types import SchemaDefinition
from apcore.schema.validator import SchemaValidator


@pytest.fixture
def fixtures_dir() -> Path:
    """Returns the absolute path to the tests/schema/fixtures/ directory."""
    return Path(__file__).parent / "fixtures"


@pytest.fixture
def schemas_dir(tmp_path: Path, fixtures_dir: Path) -> Path:
    """Copies fixture files to a temp directory for test isolation."""
    dest = tmp_path / "schemas"
    shutil.copytree(fixtures_dir, dest, dirs_exist_ok=True)
    return dest


@pytest.fixture
def schema_config(schemas_dir: Path) -> Config:
    """Returns a Config with schema.root pointing to schemas_dir."""
    return Config(data={"schema": {"root": str(schemas_dir), "strategy": "yaml_first"}})


@pytest.fixture
def schema_loader(schema_config: Config, schemas_dir: Path) -> SchemaLoader:
    """Returns a configured SchemaLoader instance."""
    return SchemaLoader(config=schema_config, schemas_dir=schemas_dir)


@pytest.fixture
def ref_resolver(schemas_dir: Path) -> RefResolver:
    """Returns a RefResolver configured with the test schemas directory."""
    return RefResolver(schemas_dir=schemas_dir, max_depth=32)


@pytest.fixture
def simple_schema_def() -> SchemaDefinition:
    """Returns a pre-built SchemaDefinition for common test use."""
    return SchemaDefinition(
        module_id="test.simple",
        description="A simple test schema",
        input_schema={
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "count": {"type": "integer"},
            },
            "required": ["name"],
        },
        output_schema={
            "type": "object",
            "properties": {
                "result": {"type": "string"},
            },
        },
        version="1.0.0",
    )


@pytest.fixture
def validator() -> SchemaValidator:
    """Returns a SchemaValidator with default coerce_types=True."""
    return SchemaValidator(coerce_types=True)


@pytest.fixture
def strict_validator() -> SchemaValidator:
    """Returns a SchemaValidator with coerce_types=False (strict mode)."""
    return SchemaValidator(coerce_types=False)
