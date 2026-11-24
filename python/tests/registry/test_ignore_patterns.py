"""PROTOCOL_SPEC §3.5 / §3.6 A04 step 3a — `extensions.ignore_patterns`.

A MUST with no supplier until spec v1.42.0: the key was registered in all three
SDKs' configuration key surfaces and read by none of them, so a project that
excluded a directory from discovery had it scanned and its modules registered
anyway. The failure direction is what makes it worth a file of its own — a skip
rule that fails **open** loads code the operator asked not to load.

Every case drives `Registry.discover()` and reads the registered module IDs,
because the key's whole contract is which modules exist afterwards.
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


def _discover(patterns: list[str] | None, subdirs: tuple[str, ...]) -> list[str]:
    """Module IDs discovered from a tree of `subdirs` under `ignore_patterns`."""
    root = Path(tempfile.mkdtemp())
    for sub in subdirs:
        leaf = root / "ext" / "executor" / sub
        leaf.mkdir(parents=True)
        (leaf / "mod.py").write_text(_MODULE, encoding="utf-8")

    extensions: dict[str, Any] = {"root": "./ext"}
    if patterns is not None:
        extensions["ignore_patterns"] = patterns
    (root / "apcore.yaml").write_text(
        yaml.safe_dump({"version": "1.0", "project": {"name": "probe"}, "extensions": extensions}),
        encoding="utf-8",
    )

    cwd = os.getcwd()
    os.chdir(root)
    previous = logging.getLogger().manager.disable
    logging.disable(logging.CRITICAL)
    try:
        with warnings.catch_warnings():
            # #113's project-root notice; not what these cases measure.
            warnings.simplefilter("ignore", DeprecationWarning)
            config = Config.load(str(root / "apcore.yaml"))
        registry = Registry(config=config)
        registry.discover()
        return sorted(registry.module_ids)
    finally:
        logging.disable(previous)
        os.chdir(cwd)


TREE = ("keep", "fixtures", "vendor")
ALL = ["executor.fixtures.mod", "executor.keep.mod", "executor.vendor.mod"]


def test_nothing_configured_discovers_everything() -> None:
    """The half that keeps this additive: an absent key changes nothing."""
    assert _discover(None, TREE) == ALL
    assert _discover([], TREE) == ALL


@pytest.mark.parametrize(
    ("patterns", "expected"),
    [
        (["fixtures"], ["executor.keep.mod", "executor.vendor.mod"]),
        (["ven*"], ["executor.fixtures.mod", "executor.keep.mod"]),
        (["?endor"], ["executor.fixtures.mod", "executor.keep.mod"]),
        (["fixtures", "vendor"], ["executor.keep.mod"]),
    ],
    ids=["literal", "star", "question_mark", "two_entries"],
)
def test_a_configured_pattern_excludes_the_entry(patterns: list[str], expected: list[str]) -> None:
    assert _discover(patterns, TREE) == expected


def test_matching_is_case_sensitive() -> None:
    """§9.2.3 declares this surface case-SENSITIVE, unlike `sensitive_keys`.

    These are filenames. Folding them would make one configuration behave
    differently on a case-insensitive filesystem than on the case-sensitive one
    it was written against.
    """
    assert _discover(["FIXTURES"], TREE) == ALL


def test_the_pattern_matches_a_segment_not_a_path() -> None:
    """A04 step 3a says ENTRY NAME, so `*` cannot cross a directory boundary.

    `executor/*` names no entry: the segments are `executor`, `fixtures`, `mod.py`.
    An implementation matching against the path would exclude everything here.
    """
    assert _discover(["executor/fixtures"], TREE) == ALL


def test_a_configured_pattern_cannot_switch_off_a_builtin_row() -> None:
    """§3.5: the two lists are a UNION, so `.hidden` stays excluded regardless.

    Asserted because "extend the ignore list" could be read as "replace it", and
    a configuration that re-enabled `.git/` or `__pycache__/` would be a
    discovery surface nobody expects.
    """
    root = Path(tempfile.mkdtemp())
    for sub in ("keep", ".hidden"):
        leaf = root / "ext" / "executor" / sub
        leaf.mkdir(parents=True)
        (leaf / "mod.py").write_text(_MODULE, encoding="utf-8")
    (root / "apcore.yaml").write_text(
        yaml.safe_dump(
            {
                "version": "1.0",
                "project": {"name": "probe"},
                # A pattern that matches nothing here: the built-in rows must
                # still apply on their own.
                "extensions": {"root": "./ext", "ignore_patterns": ["nothing_matches_this"]},
            }
        ),
        encoding="utf-8",
    )
    cwd = os.getcwd()
    os.chdir(root)
    previous = logging.getLogger().manager.disable
    logging.disable(logging.CRITICAL)
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            registry = Registry(config=Config.load(str(root / "apcore.yaml")))
        registry.discover()
        assert sorted(registry.module_ids) == ["executor.keep.mod"]
    finally:
        logging.disable(previous)
        os.chdir(cwd)


def test_an_empty_entry_is_dropped() -> None:
    """A25 anchors, so `""` would match only the empty name — an operator who
    leaves a blank line in a YAML list means nothing by it."""
    assert _discover(["", "fixtures"], TREE) == ["executor.keep.mod", "executor.vendor.mod"]
