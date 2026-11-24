"""Drive `multi_root_discovery.json` — §9.1.1 `extensions.roots` (#118 D-70).

Every case discovers from a real tree with a real ``apcore.yaml``. Calling
``scan_multi_root`` directly would prove the scanner works, which was never the
question in any SDK: this one never read the key, and apcore-rust read the paths
and dropped the namespaces.

The two roots derive the SAME unprefixed ID on purpose. Without the prefix they
collide, so the namespace is observable rather than cosmetic.
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

FIXTURE = load_fixture("multi_root_discovery.json")
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


def _tree(extensions: dict[str, Any]) -> Path:
    root = Path(tempfile.mkdtemp())
    for name in ("alpha", "beta"):
        leaf = root / name / "executor" / "svc"
        leaf.mkdir(parents=True)
        (leaf / "mod.py").write_text(_MODULE, encoding="utf-8")
    (root / "apcore.yaml").write_text(
        yaml.safe_dump({"version": "1.0", "project": {"name": "multi-root-probe"}, "extensions": extensions}),
        encoding="utf-8",
    )
    return root


@pytest.mark.parametrize("case_id", list(CASES))
def test_multi_root_discovery(case_id: str) -> None:
    case = CASES[case_id]
    root = _tree(dict(case["input"]["extensions"]))
    expected = case["expected"]

    cwd = os.getcwd()
    os.chdir(root)
    previous = logging.getLogger().manager.disable
    logging.disable(logging.CRITICAL)
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            config = Config.load(str(root / "apcore.yaml"))
        registry = Registry(config=config)
        if expected.get("raises"):
            with pytest.raises(Exception) as excinfo:  # noqa: PT011 — the class is not the contract
                registry.discover()
            assert expected["error_message_contains"] in str(excinfo.value)
            return
        registry.discover()
        assert sorted(registry.module_ids) == expected["module_ids"]
    finally:
        logging.disable(previous)
        os.chdir(cwd)


def test_every_fixture_case_is_driven() -> None:
    assert set(case_ids("multi_root_discovery.json")) == set(CASES)
