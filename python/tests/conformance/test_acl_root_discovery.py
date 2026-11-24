"""Drive `acl_root_discovery.json` — config-driven ACL discovery (D-64, issue #74).

The fixture's central invariant is the negative one: a missing `acl.root` path
attaches **no** ACL and MUST NOT synthesize an empty default-deny policy, even
when `acl.default_effect` is `deny`. Getting that wrong silently denies every
inter-module call in every project without an `acl/` directory, which is exactly
the kind of defect a fixture exists to catch and a hand-written test tends to
forget.

Each case is replayed end to end: an `apcore.yaml` is written into a tmp dir
with the case's `acl.root`, the case's `fs` block materializes the filesystem
state, then `APCore(config=...)` runs the real discovery path and the attached
ACL (`Executor._acl`) is inspected. Cases carrying `caller_id` / `target_id`
additionally assert the enforcement decision, so "attached" is never confused
with "attached but inert".
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL
from apcore.client import APCore
from apcore.config import Config
from apcore.executor import Executor
from apcore.registry.registry import Registry

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("acl_root_discovery.json")
CASES: list[dict[str, Any]] = FIXTURE["test_cases"]
ACL_POLICY: dict[str, Any] = FIXTURE["acl_policy"]
DEFAULT_ACL_ROOT: str = FIXTURE["default_acl_root"]

_KNOWN_EXPECTATIONS = {
    "resolved_acl_root",
    "config_valid",
    "acl_attached",
    "enforcement",
    "decision",
}


def _materialize_fs(root: Path, spec: dict[str, str]) -> None:
    """Create the filesystem state a case declares under `fs`."""
    for relative, kind in spec.items():
        target = root / relative
        if kind == "directory" or relative.endswith("/"):
            target.mkdir(parents=True, exist_ok=True)
        elif kind == "acl_policy":
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(yaml.safe_dump(ACL_POLICY, sort_keys=False), encoding="utf-8")
        else:  # pragma: no cover - guards against an fs kind this driver cannot express
            pytest.fail(f"unhandled fs entry kind {kind!r} for {relative!r}")


def _write_config(root: Path, case: dict[str, Any]) -> Config:
    """Write the case's apcore.yaml and load it (source_path anchors acl.root)."""
    document: dict[str, Any] = {"version": "1.0", "project": {"name": "conformance"}}
    acl_section: dict[str, Any] = {}
    if not case.get("acl_root_unset"):
        acl_section["root"] = case["acl_root"]
    if "default_effect" in case:
        acl_section["default_effect"] = case["default_effect"]
    if acl_section:
        document["acl"] = acl_section

    config_path = root / "apcore.yaml"
    config_path.write_text(yaml.safe_dump(document, sort_keys=False), encoding="utf-8")
    return Config.load(str(config_path))


def _attached_acl(case: dict[str, Any], root: Path) -> ACL | None:
    """Run the real discovery path and return the ACL attached to the Executor."""
    config = _write_config(root, case)
    _materialize_fs(root, case.get("fs", {}))

    if case.get("caller_supplied_executor"):
        registry = Registry()
        client = APCore(registry=registry, executor=Executor(registry=registry), config=config)
    else:
        client = APCore(config=config)
    return client.executor._acl


@pytest.mark.parametrize("case", CASES, ids=lambda c: c["id"])
def test_acl_root_discovery_case(case: dict[str, Any], tmp_path: Path) -> None:
    cid = case["id"]
    expected: dict[str, Any] = case["expected"]
    unknown = set(expected) - _KNOWN_EXPECTATIONS
    assert not unknown, (
        f"[{cid}] acl_root_discovery.json declares expectations this driver cannot check: "
        f"{sorted(unknown)} — extend _KNOWN_EXPECTATIONS rather than ignoring them"
    )

    if "resolved_acl_root" in expected:
        # An unset acl.root must resolve to the documented default rather than
        # to None (which would disable discovery entirely).
        assert (
            Config.get_default("acl.root") == expected["resolved_acl_root"]
        ), f"[{cid}] the acl.root default must be {expected['resolved_acl_root']!r}"
        assert expected["resolved_acl_root"] == DEFAULT_ACL_ROOT

    if expected.get("config_valid"):
        # Omitting acl.root must not be a validation error (Rust parity, D-64).
        config = _write_config(tmp_path, case)
        assert (
            config.get("acl.root", Config.get_default("acl.root")) == expected["resolved_acl_root"]
        ), f"[{cid}] a config omitting acl.root must fall back to the default"

    if "acl_attached" not in expected:
        return

    acl = _attached_acl(case, tmp_path)
    attached = acl is not None
    assert attached is expected["acl_attached"], (
        f"[{cid}] acl_attached mismatch: got {attached}, expected {expected['acl_attached']} "
        f"(acl.root={case.get('acl_root')!r}, fs={case.get('fs')!r}). "
        f"A missing path MUST attach nothing and MUST NOT synthesize a default-deny ACL."
    )

    if "enforcement" in expected:
        assert attached is expected["enforcement"], (
            f"[{cid}] enforcement is active exactly when an ACL is attached: "
            f"got {attached}, expected {expected['enforcement']}"
        )

    if "decision" in expected:
        assert acl is not None, f"[{cid}] a decision expectation requires an attached ACL"
        decision = acl.check(case["caller_id"], case["target_id"])
        assert decision is expected["decision"], (
            f"[{cid}] ACL decision for caller={case['caller_id']!r} target={case['target_id']!r}: "
            f"got {decision}, expected {expected['decision']}"
        )


def test_missing_path_does_not_synthesize_via_discover_directly(tmp_path: Path) -> None:
    """The same invariant one level down: ACL.discover() itself returns None.

    Asserting only through APCore would leave the synthesize-on-missing bug
    reachable by any other caller of `ACL.discover`.
    """
    case = next(c for c in CASES if c["id"] == "missing_path_with_default_deny_does_not_synthesize")
    config = _write_config(tmp_path, case)
    assert ACL.discover(config) is None, (
        "ACL.discover MUST return None for a missing acl.root even when "
        "acl.default_effect is 'deny' — a synthesized deny-all ACL would silently "
        "deny every inter-module call in a project with no ACL directory"
    )


def test_every_fixture_case_is_parametrized() -> None:
    ids = [c["id"] for c in CASES]
    assert len(ids) == len(set(ids)), f"duplicate case ids: {sorted({i for i in ids if ids.count(i) > 1})}"
    assert len(ids) == 10, f"acl_root_discovery.json changed size: {len(ids)} cases, expected 10"
