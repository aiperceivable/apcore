#!/usr/bin/env python3
"""Report which conformance fixtures are not driven by all three SDKs.

A conformance fixture exists to prove three implementations agree. One that only
two SDKs load is not doing that job — it is proving two implementations agree,
which is a different and much weaker claim, and it reads as covered in every
inventory and dashboard.

A fixture counts as DRIVEN by an SDK only when a test file in that SDK actually
loads it by name. Merely mentioning the fixture in a comment, or having a
hand-transcribed test that duplicates its cases, does not count: a hand copy
cannot notice when the canonical fixture gains a case.

    python3 conformance/check_driver_coverage.py [--sdk-root DIR] [--strict]

Without --strict this reports and exits 0, so the existing backlog does not
block unrelated work. With --strict it exits 1 on any gap — the setting to
switch on once the backlog is closed, so new fixtures cannot land undriven.
`--baseline` accepts the current gap set and fails only on NEW gaps, which is the
useful middle setting while the backlog is being worked down.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "conformance" / "fixtures"
BASELINE = REPO / "conformance" / "driver_coverage_baseline.json"

SDKS = {
    "python": ("apcore-python", ("tests",), (".py",)),
    "typescript": ("apcore-typescript", ("tests",), (".ts",)),
    "rust": ("apcore-rust", ("tests",), (".rs",)),
}

# `tests/it.rs` is apcore-rust's aggregator — it only declares `mod` lines, so a
# name appearing there says nothing about whether the fixture is loaded.
IGNORED_FILES = {"it.rs"}

# Counting a driver by "does a test file name this fixture" has a hole both SDK
# agents independently walked into and refused: a file whose every test is
# xfail/ignore still names it, so an all-quarantined driver would flip the
# inventory to "covered" while proving nothing. These patterns let the report
# say so out loud. It stays a warning rather than a verdict because the counting
# is a heuristic — a wrong verdict would be worse than a loud caveat.
TEST_DECL = {
    ".py": re.compile(r"^\s*(?:async )?def test_", re.M),
    ".ts": re.compile(r"^\s*(?:it|test)(?:\.each\([^)]*\))?\s*\(", re.M),
    ".rs": re.compile(r"^\s*#\[test\]", re.M),
}
# Anchored to line start: these markers are discussed in prose far more often
# than they are used, and a comment explaining why a case is quarantined must not
# itself count as a quarantine.
QUARANTINE = {
    ".py": re.compile(r"^\s*@pytest\.mark\.(?:xfail|skip)", re.M),
    ".ts": re.compile(r"^\s*(?:it|test|describe)\.(?:fails|skip|todo)\s*\(", re.M),
    ".rs": re.compile(r"^\s*#\[ignore", re.M),
}


def quarantine_ratio(path: Path) -> tuple[int, int]:
    """(quarantined, declared) for one driver file."""
    try:
        text = path.read_text()
    except (UnicodeDecodeError, OSError):
        return (0, 0)
    decl = TEST_DECL.get(path.suffix)
    quar = QUARANTINE.get(path.suffix)
    if not decl or not quar:
        return (0, 0)
    return (len(quar.findall(text)), len(decl.findall(text)))


# Naming a fixture is not loading it. A file that mentions the name in a string
# but never reads JSON is a hand-transcribed copy at best — and a hand copy cannot
# notice when the canonical fixture gains a case, which is the entire reason this
# check exists. So a hit must also sit in a file that reads a fixture, either
# directly or through a tests-local helper that does.
#
# This is still a STATIC approximation: it cannot prove the loaded fixture is the
# one named, only that the file is in the business of loading fixtures. The strong
# proof lives in the drivers themselves, as the `drives_every_fixture_case` guards
# that fail when the fixture gains a case the driver does not handle.
LOAD_PRIMITIVE = {
    ".py": re.compile(r"load_fixture|json\.load|open\("),
    ".ts": re.compile(r"loadFixture|readFileSync|JSON\.parse"),
    ".rs": re.compile(r"read_to_string|include_str!|serde_json::from_str"),
}
# A driver may delegate the read to a shared helper — apcore-python's
# tests/conformance/canonical_fixtures.py is the case that forced this. An import
# of a tests-local module is accepted as the delegation.
LOCAL_IMPORT = {
    ".py": re.compile(r"^\s*from\s+conformance[\w.]*\s+import|^\s*import\s+conformance", re.M),
    ".ts": re.compile(r"""^\s*import\s.*from\s+['"]\.{1,2}/""", re.M),
    ".rs": re.compile(r"^\s*(?:use\s+(?:crate|super)::|mod\s+)", re.M),
}


def loads_fixtures(text: str, suffix: str) -> bool:
    """Whether this file reads a fixture, directly or via a tests-local helper."""
    direct = LOAD_PRIMITIVE.get(suffix)
    delegated = LOCAL_IMPORT.get(suffix)
    if direct and direct.search(text):
        return True
    return bool(delegated and delegated.search(text))


def drivers_for(sdk_root: Path, sdk: str, fixture_stem: str) -> list[str]:
    """Test files in `sdk` that load `<fixture_stem>.json` by name."""
    repo, test_dirs, suffixes = SDKS[sdk]
    base = sdk_root / repo
    if not base.is_dir():
        return []
    # Loaders name a fixture either with its extension (Python's
    # load_fixture("x.json"), Rust's join("x.json")) or as a bare stem the helper
    # completes (TypeScript's loadFixture('x')). Both are loads; a bare mention in
    # prose is not, hence the quote requirement.
    # Only ' and " delimit a string literal in these three languages. A backtick
    # is Markdown emphasis inside a docstring or doc-comment — counting it made
    # `overrides_store=` in an apcore-python docstring read as a load, which
    # inflated the coverage number this script exists to report honestly.
    pattern = re.compile(
        r"""["']{re}(?:\.json)?["']""".format(re=re.escape(fixture_stem))
    )
    hits: list[str] = []
    for test_dir in test_dirs:
        root = base / test_dir
        if not root.is_dir():
            continue
        for path in root.rglob("*"):
            if path.name in IGNORED_FILES or path.suffix not in suffixes:
                continue
            try:
                text = path.read_text()
            except (UnicodeDecodeError, OSError):
                continue
            if pattern.search(text) and loads_fixtures(text, path.suffix):
                hits.append(str(path.relative_to(base)))
    return sorted(hits)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sdk-root", type=Path, default=REPO.parent)
    ap.add_argument("--strict", action="store_true", help="exit 1 on any gap")
    ap.add_argument("--baseline", action="store_true",
                    help="exit 1 only on gaps not in driver_coverage_baseline.json")
    ap.add_argument("--write-baseline", action="store_true")
    args = ap.parse_args()

    present = [s for s, (repo, *_) in SDKS.items() if (args.sdk_root / repo).is_dir()]
    if len(present) < 3:
        print(f"Need all three SDK checkouts under {args.sdk_root} "
              f"(found: {', '.join(present) or 'none'}) — skipping.")
        return 0

    gaps: dict[str, list[str]] = {}
    full = 0
    for path in sorted(FIXTURES.glob("*.json")):
        stem = path.stem
        missing = [s for s in SDKS if not drivers_for(args.sdk_root, s, stem)]
        if missing:
            gaps[stem] = missing
        else:
            full += 1

    total = len(list(FIXTURES.glob("*.json")))
    print(f"{total} fixtures — {full} driven by all three SDKs, {len(gaps)} with gaps")

    vacuous: list[str] = []
    for path in sorted(FIXTURES.glob("*.json")):
        for sdk, (repo, *_rest) in SDKS.items():
            for rel in drivers_for(args.sdk_root, sdk, path.stem):
                q, d = quarantine_ratio(args.sdk_root / repo / rel)
                if d and q >= d:
                    vacuous.append(f"{path.stem} / {sdk}: {rel} — {q} quarantine marker(s) "
                                   f"for {d} test(s); this may be counted as driven while "
                                   f"asserting nothing")
    if vacuous:
        print("\n  possibly-vacuous drivers (every test quarantined):")
        for line in sorted(set(vacuous)):
            print(f"    {line}")

    if args.write_baseline:
        BASELINE.write_text(json.dumps(
            {"description":
                "Fixtures not yet driven by all three SDKs, accepted as a known backlog. "
                "check_driver_coverage.py --baseline fails on any gap NOT listed here, so the "
                "backlog can shrink without new gaps slipping in. Delete an entry when its "
                "driver lands; the list reaching empty is the signal to switch CI to --strict.",
             "gaps": {k: sorted(v) for k, v in sorted(gaps.items())}},
            indent=2) + "\n")
        print(f"wrote {BASELINE.relative_to(REPO)} with {len(gaps)} entries")
        return 0

    for stem, missing in sorted(gaps.items()):
        cases = len(json.loads((FIXTURES / f"{stem}.json").read_text()).get("test_cases", []))
        print(f"  {stem} ({cases} cases) — no driver in: {', '.join(missing)}")

    if args.strict and gaps:
        print(f"\n{len(gaps)} fixtures are not driven by all three SDKs.", file=sys.stderr)
        return 1

    if args.baseline:
        known = json.loads(BASELINE.read_text())["gaps"] if BASELINE.is_file() else {}
        new = {k: v for k, v in gaps.items() if sorted(v) != sorted(known.get(k, []))}
        closed = [k for k in known if k not in gaps]
        for stem in closed:
            print(f"  CLOSED   {stem} — now driven by all three; remove it from the baseline")
        if new:
            print(f"\n{len(new)} NEW coverage gap(s):", file=sys.stderr)
            for stem, missing in sorted(new.items()):
                was = known.get(stem)
                detail = f"was missing {', '.join(sorted(was))}" if was else "not in the baseline"
                print(f"  {stem}: missing {', '.join(sorted(missing))} ({detail})", file=sys.stderr)
            print("\nA fixture only two SDKs load proves two implementations agree, "
                  "not three.", file=sys.stderr)
            return 1
        if closed:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
