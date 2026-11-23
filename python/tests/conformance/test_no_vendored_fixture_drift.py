"""Guard: no vendored fixture copies exist, and no driver reads one.

apcore-typescript and apcore-rust read the canonical fixture in the spec repo
(via ``$CONFORMANCE_SPEC_REPO`` or a sibling checkout). apcore-python used to keep
private copies under ``tests/conformance/fixtures/`` and resolve them with
``Path(__file__).parent / "fixtures"``, so a fixture that gained a case on the
spec side updated the other two SDKs silently and left Python asserting the old
snapshot. Every driver now goes through ``conformance.canonical_fixtures``.

The vendored directory has since been deleted — every copy in it was verified
byte-identical to its canonical counterpart first. These two tests keep it
deleted: a reintroduced copy is a snapshot that will drift, and a driver that
points back at this package is how the drift becomes invisible.
"""

from __future__ import annotations

from pathlib import Path

_DRIVER_DIR = Path(__file__).resolve().parent
_VENDORED_DIR = _DRIVER_DIR / "fixtures"


def test_no_driver_resolves_fixtures_from_this_package() -> None:
    offenders = []
    for driver in sorted(_DRIVER_DIR.glob("test_*.py")):
        if driver.name == Path(__file__).name:
            continue
        source = driver.read_text()
        if 'Path(__file__).parent / "fixtures"' in source:
            offenders.append(driver.name)
    assert offenders == [], (
        "these drivers resolve fixtures from a vendored copy instead of the "
        f"canonical spec repo: {offenders}. Use conformance.canonical_fixtures."
    )


def test_no_vendored_fixture_directory_exists() -> None:
    stragglers = (
        sorted(p.name for p in _VENDORED_DIR.iterdir() if p.suffix in (".json", ".yaml"))
        if _VENDORED_DIR.is_dir()
        else []
    )
    assert not _VENDORED_DIR.exists(), (
        f"{_VENDORED_DIR} was reintroduced (fixture files: {stragglers or 'none'}). "
        "Conformance fixtures live in the apcore spec repo only; load them with "
        "conformance.canonical_fixtures so a spec-side edit reaches this SDK on "
        "the next run instead of leaving it on a stale snapshot."
    )
