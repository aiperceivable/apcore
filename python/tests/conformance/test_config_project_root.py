"""Drive project-root discovery and path resolution through public consumers."""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import Any

import pytest

from apcore.acl import ACL
from apcore.config import Config
from apcore.schema import SchemaLoader

from .canonical_fixtures import load_fixture, reject_unknown_expectations

FIXTURE = "config_project_root.json"
_SPEC = load_fixture(FIXTURE)
_CASES = {case["id"]: case for case in _SPEC["test_cases"]}
_LAYOUT = _SPEC["layout"]


def _layout_path(root: Path, relative: str) -> Path:
    """Resolve fixture tokens using the SDK discovery platform's directories."""
    home = root / _LAYOUT["home"]
    tier6 = (
        home / "Library" / "Application Support" / "apcore"
        if sys.platform == "darwin"
        else (home / ".config" / "apcore")
    )
    tier7 = home / ".apcore"
    tokens = {
        "<tier6_config>": tier6 / "config.yaml",
        "<tier7_config>": tier7 / "config.yaml",
        "<tier6_dir>": tier6,
        "<tier7_dir>": tier7,
    }
    head, separator, suffix = relative.partition("/")
    if head in tokens:
        return tokens[head] / suffix if separator else tokens[head]
    return root / relative


def _materialize(root: Path, case: dict[str, Any], monkeypatch: pytest.MonkeyPatch) -> None:
    for relative in _LAYOUT["dirs"]:
        (root / relative).mkdir()
    home = root / _LAYOUT["home"]
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("USERPROFILE", str(home))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(home / ".config"))
    for name in os.environ.copy():
        if name.startswith("APCORE_"):
            monkeypatch.delenv(name)

    for relative, content in case.get("fs", {}).items():
        path = _layout_path(root, relative)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    for link, target in case.get("symlinks", {}).items():
        resolved_target = _layout_path(root, target)
        _layout_path(root, link).symlink_to(resolved_target, target_is_directory=resolved_target.is_dir())
    for name, value in case.get("env", {}).items():
        monkeypatch.setenv(name, str(_layout_path(root, value)) if name == "APCORE_CONFIG_FILE" else value)
    for name, relative in case.get("absolute_env", {}).items():
        monkeypatch.setenv(name, str(_layout_path(root, relative)))
    monkeypatch.chdir(_layout_path(root, case["cwd"]))


def _assert_root(root: Path, config: Config, expected: dict[str, Any]) -> None:
    project_root = Path(config.project_root).resolve()
    assert project_root == _layout_path(root, expected["project_root"]).resolve()
    assert (project_root == Path.cwd().resolve()) is expected["project_root_equals_cwd"]

    source = config.source_path
    source_dir = None if source is None else Path(source).parent.resolve()
    expected_source = expected["config_source_dir"]
    assert source_dir == (None if expected_source is None else _layout_path(root, expected_source).resolve())


def _assert_path(root: Path, config: Config, raw: str, expected: str) -> None:
    assert isinstance(raw, str)
    actual = config.resolve_path(raw)
    expected_path = _layout_path(root, expected)
    if Path(raw).is_absolute():
        assert actual == str(expected_path)
    else:
        assert Path(actual).is_absolute()
        assert Path(actual).resolve() == expected_path.resolve()


def _assert_config_paths(root: Path, config: Config, expected: dict[str, Any]) -> None:
    for key, target in expected["resolved_config_paths"].items():
        assert key in Config.path_typed_keys()
        if key == "extensions.roots[]":
            raw_roots = config.get("extensions.roots")
            assert isinstance(raw_roots, list)
            for entry, expected_entry in zip(raw_roots, target, strict=True):
                _assert_path(root, config, entry["root"] if isinstance(entry, dict) else entry, expected_entry)
        else:
            _assert_path(root, config, config.get(key, Config.get_default(key)), target)


def _assert_consumers(config: Config, expected: dict[str, Any]) -> None:
    if "resolved_acl_root" in expected:
        acl = ACL.discover(config)
        assert acl is not None
        assert acl.rules[0].description == expected["resolved_acl_root"]
    if "resolved_schema_root" in expected:
        assert SchemaLoader(config).load("probe").description == expected["resolved_schema_root"]


@pytest.mark.parametrize("case_id", _CASES)
def test_config_project_root(case_id: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    case = _CASES[case_id]
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    _materialize(tmp_path, case, monkeypatch)
    if "config_from_mapping" in case:
        config = Config(data=case["config_from_mapping"])
    elif "config_path" in case:
        config = Config.load(str(_layout_path(tmp_path, case["config_path"])), validate=False)
    else:
        config = Config.load(validate=False)

    expected = case["expected"]
    known = {
        "project_root",
        "project_root_equals_cwd",
        "config_source_dir",
        "resolved_config_paths",
        "resolved_acl_root",
        "resolved_schema_root",
    }
    assert not set(expected) - known
    _assert_root(tmp_path, config, expected)
    if consume_from := case.get("consume_from"):
        monkeypatch.chdir(_layout_path(tmp_path, consume_from))
    _assert_config_paths(tmp_path, config, expected)
    _assert_consumers(config, expected)
