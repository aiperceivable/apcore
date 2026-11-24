"""CI must check the canonical spec dependency before collecting SDK tests."""

from pathlib import Path

import yaml


def test_ci_uses_fixtures_from_the_same_checkout() -> None:
    workflow = yaml.safe_load((Path(__file__).resolve().parents[2] / ".github/workflows/sdk-python.yml").read_text())
    job = workflow["jobs"]["test"]
    assert job["defaults"]["run"]["working-directory"] == "python"
    checkouts = [step for step in job["steps"] if step.get("uses", "").startswith("actions/checkout@")]
    assert len(checkouts) == 1
    assert not checkouts[0].get("with", {}).get("repository")


def test_ci_verifies_fixture_checkout_before_running_tests() -> None:
    workflow = yaml.safe_load((Path(__file__).resolve().parents[2] / ".github/workflows/sdk-python.yml").read_text())
    job = workflow["jobs"]["test"]
    assert job["env"]["CONFORMANCE_SPEC_REPO"] == "${{ github.workspace }}"
    steps = job["steps"]
    probes = [
        index
        for index, step in enumerate(steps)
        if step.get("run") == ("pytest tests/conformance/test_fixture_checkout.py -q")
    ]
    assert len(probes) == 1
    assert probes[0] < next(index for index, step in enumerate(steps) if step.get("name") == "Run tests")
