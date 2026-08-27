#!/usr/bin/env python3
"""Report `expected` keys that no SDK driver reads.

A fixture is a contract only to the extent that a driver asserts it. A key sitting
in `expected` that every driver ignores is worse than no key: it reads as covered
in the fixture, in review, and in any inventory built from the fixture — while
nothing on any of the three SDKs is being checked. Flipping its value red-lines
nothing.

This is the same defect class as asserting a class name instead of a wire code,
one level up. There the assertion existed and was weak; here it does not exist at
all, but the fixture looks identical either way.

Found in the 0.26 sweep: `pipeline_failfast_config.json` asserted `error_type`, a
class name all three SDKs share, and was green while they emitted three different
wire codes. Behind that, `pipeline_step_middleware.json` declared `wrapped_in` and
no driver read it — removing the `MiddlewareChainError` wrapping from two SDKs left
every test passing.

    python3 conformance/check_expected_keys_read.py [--sdk-root DIR] [--strict]
                                                    [--baseline] [--write-baseline]

Only TOP-LEVEL keys of `expected` are checked. Those are assertion names. Keys
nested inside them are usually payload under test — a field name being redacted, a
schema title, a response body — and flagging them produces noise that trains people
to ignore the check.

`--strict` exits 1 on any unread key. `--baseline` accepts the current set and fails
only on NEW ones, which is the useful setting while the backlog is worked down.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "conformance" / "fixtures"
BASELINE = REPO / "conformance" / "expected_keys_baseline.json"
ALLOWLIST = REPO / "conformance" / "expected_keys_allowlist.json"

SDKS = {
    "python": ("apcore-python", ".py"),
    "typescript": ("apcore-typescript", ".ts"),
    "rust": ("apcore-rust", ".rs"),
}


def _fixture_pattern(fixture_stem: str) -> re.Pattern[str]:
    """A quoted mention of `<stem>` or `<stem>.json` — how a loader names one.

    Same rule as `check_driver_coverage.py`: quotes only, never backticks, so a
    fixture named in a docstring does not read as a load.
    """
    return re.compile(r"""["']{re}(?:\.json)?["']""".format(re=re.escape(fixture_stem)))


def drivers_for(sdk_root: Path, fixture_stem: str) -> dict[str, str]:
    """Source of the test files that load `<fixture_stem>.json`, per SDK.

    Scoping the search to a fixture's OWN drivers is the whole point. This used
    to concatenate every test file in each repo and ask whether the key appeared
    anywhere in it, so a key counted as read if any unrelated test happened to
    contain the same short string: `stream_aggregation`'s `b` matched in 124
    files, `schema_strict_conversion`'s `type` in 108. The check reported "0 read
    by no driver" partly by accident, and reported a live allowlist entry as
    stale on the strength of one unrelated schema-coercion test
    (sync finding B-004).
    """
    out: dict[str, str] = {}
    pattern = _fixture_pattern(fixture_stem)
    for sdk, (repo, suffix) in SDKS.items():
        base = sdk_root / repo / "tests"
        if not base.is_dir():
            continue
        chunks = []
        for path in base.rglob(f"*{suffix}"):
            try:
                text = path.read_text()
            except (UnicodeDecodeError, OSError):
                continue
            if pattern.search(text):
                chunks.append(text)
        out[sdk] = "\n".join(chunks)
    return out


def available_sdks(sdk_root: Path) -> list[str]:
    """Which SDK checkouts are present — the script needs all three to be honest."""
    return [sdk for sdk, (repo, _) in SDKS.items() if (sdk_root / repo / "tests").is_dir()]


#: How a driver asserts EVERY key of `expected` without naming any of them.
#:
#: This is the well-written form — iterate the expected map and compare each
#: entry, or deep-equal the whole thing — and it contains no key literals at
#: all. A literal search therefore reports the best drivers as asserting
#: nothing: `usage_contract`'s Rust driver ends in
#: `for (name, want) in expected { assert_eq!(&result[name], want) }` and was
#: reported as leaving five keys unasserted (sync finding B-004).
WHOLESALE_PATTERNS = [
    # Python
    r"\bexpected(?:\[[^\]]+\])?\.items\(\)",
    r'\bcase\["expected"\]\.items\(\)',
    r"==\s*case\[.expected.\]",
    # TypeScript
    r"toEqual\(\s*\w+\.expected",
    r"toMatchObject\(\s*\w+\.expected",
    r"Object\.entries\(\s*\w+\.expected",
    r"Object\.entries\(\s*expected",
    # Rust
    r"for\s*\(\s*\w+\s*,\s*\w+\s*\)\s*in\s*(?:&)?expected",
    r"for\s*\(\s*\w+\s*,\s*\w+\s*\)\s*in\s*case\[.expected.\]",
]
_WHOLESALE_RE = re.compile("|".join(WHOLESALE_PATTERNS))


def asserts_wholesale(text: dict[str, str]) -> bool:
    """True when any driver compares the whole `expected` map rather than named keys."""
    return any(_WHOLESALE_RE.search(t) for t in text.values())


def is_read(key: str, text: dict[str, str]) -> bool:
    """True when a driver of this fixture names `key` as a literal or subscript.

    Quote/bracket delimited so a key that merely appears in prose does not count
    — the same reason `check_driver_coverage.py` excludes backticks. `text` must
    already be scoped to the fixture's own drivers; see `drivers_for`.
    """
    pattern = re.compile(r"""["'\[]{k}["'\]]""".format(k=re.escape(key)))
    return any(pattern.search(t) for t in text.values())


def top_level_expected_keys(fixture: dict) -> set[str]:
    keys: set[str] = set()
    for case in fixture.get("test_cases", []):
        if not isinstance(case, dict):
            continue
        expected = case.get("expected")
        if isinstance(expected, dict):
            keys.update(expected.keys())
    return keys


def load_allowlist() -> dict[str, set[str]]:
    if not ALLOWLIST.is_file():
        return {}
    doc = json.loads(ALLOWLIST.read_text())
    return {e["fixture"]: set(e["keys"]) for e in doc.get("allow", [])}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sdk-root", type=Path, default=REPO.parent)
    ap.add_argument("--strict", action="store_true", help="exit 1 on any unread key")
    ap.add_argument("--baseline", action="store_true",
                    help="exit 1 only on keys not in expected_keys_baseline.json")
    ap.add_argument("--write-baseline", action="store_true")
    args = ap.parse_args()

    present = available_sdks(args.sdk_root)
    if len(present) < 3:
        print(f"Need all three SDK checkouts under {args.sdk_root} "
              f"(found: {', '.join(present) or 'none'}) — skipping.")
        return 0

    allow = load_allowlist()
    unread: dict[str, list[str]] = {}
    stale_allow: list[str] = []
    total_keys = 0

    for path in sorted(FIXTURES.glob("*.json")):
        stem = path.name
        fixture = json.loads(path.read_text())
        # Scoped per fixture: only the files that load THIS one may vouch for
        # its keys.
        text = drivers_for(args.sdk_root, path.stem)
        keys = top_level_expected_keys(fixture)
        wholesale = asserts_wholesale(text)
        total_keys += len(keys)
        allowed = allow.get(stem, set())
        missing = [] if wholesale else sorted(k for k in keys if not is_read(k, text))
        for k in sorted(allowed & set(keys)):
            if wholesale or is_read(k, text):
                stale_allow.append(f"{stem}: {k}")
        flagged = [k for k in missing if k not in allowed]
        if flagged:
            unread[stem] = flagged

    flagged_total = sum(len(v) for v in unread.values())
    print(f"{total_keys} top-level `expected` keys across "
          f"{len(list(FIXTURES.glob('*.json')))} fixtures — "
          f"{flagged_total} read by no driver, in {len(unread)} fixtures")

    if args.write_baseline:
        BASELINE.write_text(json.dumps(
            {"description":
                "`expected` keys no SDK driver reads, accepted as a known backlog. "
                "check_expected_keys_read.py --baseline fails on any key NOT listed here, so the "
                "backlog can shrink without new vacuous expectations slipping in. Delete an entry "
                "when a driver starts asserting it; the list reaching empty is the signal to "
                "switch CI to --strict. A key here is NOT covered — it only looks covered.",
             "unread": {k: sorted(v) for k, v in sorted(unread.items())}},
            indent=2) + "\n")
        print(f"wrote {BASELINE.relative_to(REPO)} with {flagged_total} keys")
        return 0

    for stem, keys in sorted(unread.items()):
        print(f"  {stem} ({len(keys)})")
        print(f"    {', '.join(keys)}")

    if stale_allow:
        print("\n  STALE allowlist entries — a driver now reads these, remove them:")
        for line in stale_allow:
            print(f"    {line}")

    if args.strict and flagged_total:
        print(f"\n{flagged_total} `expected` keys are asserted by no driver.", file=sys.stderr)
        return 1

    if args.baseline:
        known = json.loads(BASELINE.read_text())["unread"] if BASELINE.is_file() else {}
        new = {k: sorted(set(v) - set(known.get(k, []))) for k, v in unread.items()}
        new = {k: v for k, v in new.items() if v}
        closed = {k: sorted(set(v) - set(unread.get(k, []))) for k, v in known.items()}
        closed = {k: v for k, v in closed.items() if v}
        for stem, keys in sorted(closed.items()):
            print(f"  CLOSED   {stem}: {', '.join(keys)} — now asserted; "
                  f"remove from the baseline")
        if new:
            print(f"\n{sum(len(v) for v in new.values())} NEW unasserted `expected` key(s):",
                  file=sys.stderr)
            for stem, keys in sorted(new.items()):
                print(f"  {stem}: {', '.join(keys)}", file=sys.stderr)
            print("\nA key in `expected` that no driver reads is not a contract. "
                  "Assert it, or do not declare it.", file=sys.stderr)
            return 1
        if closed or stale_allow:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
