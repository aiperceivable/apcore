"""Exercise binding schema reference resolution through the binding loader."""

from pathlib import Path

import pytest
import yaml

from apcore.bindings import BindingLoader
from apcore.config import Config
from apcore.errors import ModuleError
from apcore.registry import Registry

from .canonical_fixtures import load_fixture


@pytest.mark.parametrize("case", load_fixture("binding_schema_ref_scope.json")["test_cases"], ids=lambda c: c["id"])
def test_binding_schema_ref_scope(tmp_path: Path, case: dict) -> None:
    binding_dir = tmp_path / "bindings"
    binding_dir.mkdir()
    reference = case["schema_ref"]
    if "schema_file" in case:
        schema_file = tmp_path / case["schema_file"]
        schema_file.parent.mkdir(parents=True, exist_ok=True)
        schema = {"input_schema": {"type": "object"}, "output_schema": {"type": "object"}}
        schema_file.write_text("[" if case.get("invalid_document") else yaml.safe_dump(schema), encoding="utf-8")
        if case.get("absolute"):
            reference = str(schema_file)

    module_id = "fixture.echo"
    binding_file = binding_dir / "fixture.binding.yaml"
    binding_file.write_text(
        yaml.safe_dump(
            {
                "spec_version": "1.0",
                "bindings": [
                    {"module_id": module_id, "target": "json:dumps", "description": "Echo", "schema_ref": reference}
                ],
            }
        ),
        encoding="utf-8",
    )
    config = None
    if "schema_root" in case:
        config_path = tmp_path / "apcore.yaml"
        config_path.write_text(
            yaml.safe_dump(
                {"version": "2.0.0", "project": {"name": "fixture"}, "schema": {"root": case["schema_root"]}}
            ),
            encoding="utf-8",
        )
        config = Config.load(str(config_path))

    registry = Registry()
    actual_error = None
    try:
        BindingLoader().load_bindings(str(binding_file), registry, config)
    except ModuleError as error:
        actual_error = error.code

    assert actual_error == case["expected"]["error_code"]
    assert (registry.get(module_id) is not None) == case["expected"]["registered"]
