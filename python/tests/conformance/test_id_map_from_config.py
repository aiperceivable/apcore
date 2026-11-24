"""Drive `id_map_from_config.json` — §9.1.1 `id_map.overrides` (#118 D-71).

Every case discovers from a real directory with a real ``apcore.yaml`` and reads
the registered module IDs. Calling ``load_id_map`` directly would prove the
loader works, which was never the question: the mechanism was implemented in all
three SDKs and the CONFIG KEY reached none of them.
"""

from __future__ import annotations

import logging
import os
import tempfile
import warnings
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.config import Config
from apcore.registry import Registry

from .canonical_fixtures import case_ids, load_fixture

FIXTURE = load_fixture("id_map_from_config.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}

_MODULE = """from pydantic import BaseModel


class In(BaseModel):
    pass


class Out(BaseModel):
    ok: bool


class Mod:
    input_schema = In
    output_schema = Out
    description = "Discoverable probe module."

    def execute(self, inputs, context):
        return {"ok": True}
"""


def _tree(declare_override: bool, config_map_entries: list[dict[str, str]] | None = None) -> Path:
    root = Path(tempfile.mkdtemp())
    leaf = root / "ext" / "executor" / "orig"
    leaf.mkdir(parents=True)
    (leaf / "mod.py").write_text(_MODULE, encoding="utf-8")
    for name, module_id in (("map.yaml", "executor.renamed.mod"), ("explicit.yaml", "executor.explicit.mod")):
        (root / name).write_text(
            yaml.safe_dump({"mappings": [{"file": "executor/orig/mod.py", "id": module_id}]}),
            encoding="utf-8",
        )
    if config_map_entries is not None:
        # driver_contract.config_map_entries: replaces map.yaml's mappings.
        (root / "map.yaml").write_text(yaml.safe_dump({"mappings": config_map_entries}), encoding="utf-8")
    doc: dict[str, Any] = {
        "version": "1.0",
        "project": {"name": "id-map-probe"},
        "extensions": {"root": "./ext"},
    }
    if declare_override:
        doc["id_map"] = {"overrides": "./map.yaml"}
    (root / "apcore.yaml").write_text(yaml.safe_dump(doc), encoding="utf-8")
    return root


@pytest.mark.parametrize("case_id", list(CASES))
def test_id_map_from_config(case_id: str, monkeypatch: pytest.MonkeyPatch) -> None:
    case = CASES[case_id]
    root = _tree(case["input"]["declare_override"], case["input"].get("config_map_entries"))
    explicit = str(root / "explicit.yaml") if case["input"]["explicit_argument"] else None
    # driver_contract.env: the variable is unset unless the case lists it.
    monkeypatch.delenv("APCORE_ID__MAP_OVERRIDES", raising=False)
    for name, value in case["input"].get("env", {}).items():
        monkeypatch.setenv(name, value)

    cwd = os.getcwd()
    os.chdir(root)
    previous = logging.getLogger().manager.disable
    logging.disable(logging.CRITICAL)
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            config = Config.load(str(root / "apcore.yaml"))
        registry = Registry(config=config, id_map_path=explicit)
        registry.discover()
        assert sorted(registry.module_ids) == case["expected"]["module_ids"]
    finally:
        logging.disable(previous)
        os.chdir(cwd)


def test_the_override_path_uses_the_same_base_as_extensions_root() -> None:
    """`driver_contract.relative_paths`.

    PROTOCOL_SPEC §9.2.1 leaves the resolution base for path-typed keys
    deliberately unspecified (tracked in #113): `acl.root` resolves against the
    config file's directory, `schema.root` against the process CWD. This key
    does not settle that — it follows its SIBLING, because `id_map.overrides`
    and `extensions.root` are two halves of one discovery configuration and a
    split base between them would be worse than either.

    Driven from a different working directory: both halves fail together, which
    is the property being pinned. If `extensions.root` ever moves to the config
    directory, this case fails and takes the map with it — deliberately.
    """
    root = _tree(declare_override=True)
    elsewhere = Path(tempfile.mkdtemp())
    cwd = os.getcwd()
    os.chdir(elsewhere)
    previous = logging.getLogger().manager.disable
    logging.disable(logging.CRITICAL)
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            config = Config.load(str(root / "apcore.yaml"))
        with pytest.raises(Exception):  # noqa: B017,PT011 — `extensions.root` misses first
            Registry(config=config).discover()
    finally:
        logging.disable(previous)
        os.chdir(cwd)


def test_every_fixture_case_is_driven() -> None:
    assert set(case_ids("id_map_from_config.json")) == set(CASES)
