"""CI must check the canonical spec dependency before collecting SDK tests."""

from pathlib import Path

import yaml


def test_ci_checks_out_spec_main_independently_of_sdk_branch() -> None:
    workflow = yaml.safe_load((Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml").read_text())
    checkouts = [
        step
        for step in workflow["jobs"]["test"]["steps"]
        if step.get("with", {}).get("repository") == "aiperceivable/apcore"
    ]
    assert len(checkouts) == 1
    assert checkouts[0]["with"]["ref"] == "main"
    assert checkouts[0]["with"]["path"] == ".apcore-spec"


def test_ci_verifies_fixture_checkout_before_running_tests() -> None:
    workflow = yaml.safe_load((Path(__file__).resolve().parents[1] / ".github/workflows/ci.yml").read_text())
    job = workflow["jobs"]["test"]
    assert job["env"]["CONFORMANCE_SPEC_REPO"] == "${{ github.workspace }}/.apcore-spec"
    steps = job["steps"]
    probes = [
        index
        for index, step in enumerate(steps)
        if step.get("run") == ("pytest tests/conformance/test_fixture_checkout.py -q")
    ]
    assert len(probes) == 1
    assert probes[0] < next(index for index, step in enumerate(steps) if step.get("name") == "Run tests")
