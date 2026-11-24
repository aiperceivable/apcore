"""Fail early when CI checks out a spec snapshot older than its SDK tests."""

from pathlib import Path

import yaml

from .canonical_fixtures import fixtures_dir


def _missing_declared_fixtures(fixtures_path: Path, declaration_path: Path) -> list[str]:
    declaration = yaml.safe_load(declaration_path.read_text())
    names = declaration["conformance"]["fixture_results"]["fixture_names"]
    return sorted(f"{name}.json" for name in names if not (fixtures_path / f"{name}.json").is_file())


def test_missing_declared_fixtures_reports_all_missing_files(tmp_path: Path) -> None:
    declaration = tmp_path / "declaration.yaml"
    declaration.write_text(
        yaml.safe_dump(
            {"conformance": {"fixture_results": {"fixture_names": ["binding_file_validation", "canonicalize_name"]}}}
        )
    )
    assert _missing_declared_fixtures(tmp_path, declaration) == [
        "binding_file_validation.json",
        "canonicalize_name.json",
    ]
    (tmp_path / "binding_file_validation.json").write_text("{}")
    assert _missing_declared_fixtures(tmp_path, declaration) == ["canonicalize_name.json"]
    (tmp_path / "canonicalize_name.json").write_text("{}")
    assert _missing_declared_fixtures(tmp_path, declaration) == []


def test_canonical_checkout_contains_all_declared_fixtures() -> None:
    declaration = Path(__file__).resolve().parents[2] / "apcore-conformance.yaml"
    canonical = fixtures_dir()
    missing = _missing_declared_fixtures(canonical, declaration)
    assert not missing, (
        f"Canonical fixtures missing from {canonical}: {', '.join(missing)}. "
        "Publish the apcore spec changes before the SDK changes, then rerun CI. "
        "Do not skip these fixtures or substitute private copies."
    )
