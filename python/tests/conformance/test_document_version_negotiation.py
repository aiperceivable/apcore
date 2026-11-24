"""Verify compatibility through the public YAML document loaders."""

from pathlib import Path

import pytest
import yaml

from apcore.config import Config
from apcore.errors import ModuleError
from apcore.schema import SchemaLoader

from .canonical_fixtures import load_fixture


@pytest.mark.parametrize("case", load_fixture("document_version_negotiation.json")["test_cases"], ids=lambda c: c["id"])
def test_document_version_negotiation(tmp_path: Path, case: dict) -> None:
    declaration = {"version": case["version"]} if "version" in case else {}
    path = tmp_path / ("apcore.yaml" if case["format"] == "config" else "example.schema.yaml")
    if case["format"] == "config":
        document = {"apcore": {"project": {"name": "conformance"}, **declaration}}
    else:
        document = {
            "module_id": "example",
            "description": "Conformance example",
            "input_schema": {"type": "object"},
            "output_schema": {"type": "object"},
            **declaration,
        }
    path.write_text(yaml.safe_dump(document), encoding="utf-8")

    def load() -> None:
        if case["format"] == "config":
            Config.load(str(path))
        else:
            SchemaLoader(Config(), tmp_path).load("example")

    expected = case["expected"]["error_code"]
    if expected is None:
        load()
    else:
        with pytest.raises(ModuleError) as error:
            load()
        assert error.value.code == expected
