#!/usr/bin/env python3
"""Report spec clauses one SDK has disabled while another exercises them.

A conformance suite may legitimately disable a clause: the SDK genuinely has no
such symbol, or names it differently, and a disabled test with a written reason
is more honest than a coarse import failure. The failure mode is what happens
NEXT. The reason is a claim about the SDK, written once; the SDK then changes,
and nothing re-reads it. A disabled test explaining why something cannot be
tested keeps explaining it after it can, and the clause reads as a documented
gap while going untested.

This has already happened twice, both times to work done in this audit:

  * `async_tasks.save.error.TASK_STORE_UNAVAILABLE` stayed `it.skip` /
    `#[ignore]` in apcore-typescript and apcore-rust, reason "missing symbol
    TaskStoreError/TASK_STORE_UNAVAILABLE (contract gap)", after D-92 required
    all three SDKs to define and export exactly that type.
  * `cancellation.raise_if_cancelled.*` stayed `it.skip` in apcore-typescript,
    reason "missing symbol CancelToken.raiseIfCancelled", after the v1.49.0 -
    v1.54.0 implementation added `raiseIfCancelled()` to `src/cancel.ts`.

Neither turned anything red, because a skip is green.

THE ORACLE IS THE PEER SUITES. Resolving "does symbol X exist today" across
three languages is guesswork — a first attempt at it produced more false
positives than findings. But the three suites share clause ids, so a clause
LIVE in one SDK and DISABLED in another is a precise, language-independent
signal, and it is the signal that catches this class: Python exercised
`raise_if_cancelled` throughout.

An asymmetry is not automatically a defect. Some are real API differences
(apcore-python has no `Middleware.detect_async`). The baseline records the ones
already looked at, WITH the state that was true when they were looked at, so
that:

  * a NEW asymmetry is reported, and
  * an asymmetry that has CHANGED shape is reported — including one that has
    resolved itself, because a baseline entry describing a gap that is gone is
    the same stale artifact this guard exists to find.

A baseline entry is a record that someone looked, not a judgement that the skip
is correct.

    python3 conformance/check_skip_asymmetry.py [--strict] [--write-baseline]

THE DETECTOR IS THE GUARD. Deciding whether a clause is live or disabled means
reading three suites' own conventions, and a window that gets one wrong reports
ZERO gaps for that SDK while the run stays green — strictly worse than no guard.
It got two wrong at once for its whole first life: apcore-rust writes the id on
a `// clause:` line ABOVE the attribute block, so a lookbehind-only window read
EVERY `#[ignore]`d clause in that SDK as live (the false negative is in exactly
the direction this guard exists to catch), and apcore-python writes a six-line
`@pytest.mark.skip(reason=...)` above the `def` whose docstring carries the id,
further than three lines reach. `--self-test` pins one sample per convention per
state and runs before every count.

    python3 conformance/check_skip_asymmetry.py --self-test

Without --strict this reports and exits 0 except on baseline ROT or a wrong
detector. The ratchet may only go down.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BASELINE = REPO / "conformance" / "skip_asymmetry_baseline.json"

SDK_DIRS = {"python": "apcore-python", "typescript": "apcore-typescript", "rust": "apcore-rust"}
SDK_EXT = {"python": ".py", "typescript": ".ts", "rust": ".rs"}

# A clause id as the three suites spell it: a dotted path whose second-to-last
# segment names the kind of assertion.
CLAUSE = re.compile(
    r"\b([a-z_0-9]+(?:\.[a-z_0-9]+)*\."
    r"(?:error|property|input|output|return|side_effect)\.[A-Za-z_0-9.]+)"
)
# Anchored to line start. A disabling marker is a statement, never prose: the
# unanchored form matched comments ABOUT a skip, and the item-scoped window
# below reaches far enough to pick those up.
DISABLED = re.compile(
    r"^[ \t]*(?:it\.skip\(|describe\.skip\(|#\[ignore|@pytest\.mark\.skip|pytest\.skip\()",
    re.M,
)

# A disabling marker governs a clause id when both belong to the same test
# ITEM. A fixed line window got this wrong in both directions, and each error
# was silent because a skip is green:
#
#   * apcore-rust writes the id on a `// clause:` line ABOVE the attribute
#     block, so `#[ignore]` sits BELOW the id. A lookbehind-only window read
#     the id as live, and "an id seen both ways counts as live" then refused to
#     downgrade it. EVERY `#[ignore]`d clause in apcore-rust was reported live
#     — a false negative in exactly the direction the guard exists to catch.
#
#   * apcore-python writes `@pytest.mark.skip(reason="...")` across six lines,
#     then `def`, then the docstring that carries the id. A 3-line lookbehind
#     stopped inside the decorator call and never saw its first line, so those
#     clauses were reported live too.
#
# The window is therefore item-scoped: back to the enclosing declaration and
# over the contiguous decorator/attribute block above it, forward to the next
# declaration. Blank lines bound the decorator block, which is what separates
# one test's markers from the previous test's body.
# Both scans are bounded by a BLANK LINE, which is what separates one test's
# block from its neighbour's in all three languages; the caps are only a
# backstop against a file with no blank lines at all. A fixed 4-line forward
# window was too small for apcore-rust's ordinary shape — a `// clause:` line,
# three lines of comment, `#[test]`, `#[ignore]`, `fn` — so the scan gave up,
# fell through to the backward branch, and picked up the PREVIOUS test's
# marker.
BACK_SCAN = 24
FWD_SCAN = 24

# A declaration line: `fn`, `def`, `it(`/`test(`/`describe(` and their `.skip`
# variants.
ITEM_DECL = re.compile(
    r"^\s*(?:pub\s+)?(?:async\s+)?fn\s|"
    r"^\s*(?:async\s+)?def\s|"
    r"^\s*(?:it|test|describe)(?:\.\w+)?\s*\("
)


def marker_window(lines: list[str], i: int) -> list[str]:
    """Lines that may carry a disabling marker for the clause id on line `i`.

    Three conventions, and a window that gets all three wrong if it is a fixed
    number of lines in one direction:

      * apcore-typescript puts the id IN the declaration: `it.skip('<id>', ...)`.
      * apcore-rust puts it ABOVE the attribute block: `// clause: <id>` then
        `#[test]` / `#[ignore]` then `fn`. The marker is BELOW the id.
      * apcore-python puts it INSIDE the body, in the docstring, under a
        `@pytest.mark.skip(reason=...)` that can run six lines. The marker is
        above the id, further than a small lookbehind reaches.

    So: attach the id to its own item first, then take that item's
    decorator/attribute block. Never reach past a neighbouring declaration —
    doing that picked up the PREVIOUS test's `#[ignore]` and called this one
    skipped.
    """
    if ITEM_DECL.search(lines[i]):
        start = i
    else:
        # The id may sit above its declaration; a blank line ends the block.
        for k in range(i + 1, min(len(lines), i + 1 + FWD_SCAN)):
            if not lines[k].strip():
                break
            if ITEM_DECL.search(lines[k]):
                return lines[i : k + 1]
        start = i
        found = False
        for j in range(i, max(-1, i - BACK_SCAN), -1):
            if ITEM_DECL.search(lines[j]):
                start = j
                found = True
                break
        if not found:
            return lines[max(0, i - BACK_SCAN) : i + 1]
    j = start - 1
    while j >= 0 and lines[j].strip() and not ITEM_DECL.search(lines[j]):
        start = j
        j -= 1
    return lines[start : i + 1]


def scan(tests_dir: Path, ext: str) -> dict[str, str]:
    """Map every clause id this suite mentions to "live" or "skipped".

    An id that appears both ways counts as live: one suite may skip a variant
    of a clause it exercises elsewhere, and reporting that as a gap would be
    noise the reader cannot act on.
    """
    out: dict[str, str] = {}
    if not tests_dir.exists():
        return out
    for path in sorted(tests_dir.rglob("*" + ext)):
        lines = path.read_text(errors="replace").splitlines()
        for i, line in enumerate(lines):
            for match in CLAUSE.finditer(line):
                cid = match.group(1).rstrip(".")
                window = marker_window(lines, i)
                state = "skipped" if DISABLED.search("\n".join(window)) else "live"
                if out.get(cid) != "live":
                    out[cid] = state
    return out


# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------
#
# The detector is the whole guard: if it misreads a suite's convention, every
# asymmetry in that SDK silently disappears and the report stays green — which
# is what happened, in two directions at once, for as long as the window was a
# fixed 3-line lookbehind. So the conventions are pinned here, one sample per
# SDK per state, and CI runs this before it trusts a count.
SELF_TEST = [
    # apcore-rust: the id sits ABOVE the attribute block, so the marker is BELOW it.
    (
        ".rs",
        "// clause: a.b.error.X\n#[tokio::test]\n#[ignore = \"a.b.error.X: gone\"]\nasync fn t() {\n}\n",
        {"a.b.error.X": "skipped"},
    ),
    (
        ".rs",
        "// clause: a.b.error.X\n// three\n// lines\n// of comment\n#[test]\nfn t() {\n}\n",
        {"a.b.error.X": "live"},
    ),
    # ...and the marker on a NEIGHBOUR must not reach across.
    (
        ".rs",
        "#[test]\n#[ignore = \"unrelated\"]\nfn prev() {\n}\n\n// clause: a.b.error.X\n#[test]\nfn t() {\n}\n",
        {"a.b.error.X": "live"},
    ),
    # apcore-python: a multi-line decorator above `def`, id in the docstring.
    (
        ".py",
        '    @pytest.mark.skip(\n        reason="one"\n        "two"\n        "three"\n    )\n'
        '    def test_x(self) -> None:\n        """a.b.input.Y"""\n',
        {"a.b.input.Y": "skipped"},
    ),
    (
        ".py",
        '    def test_x(self) -> None:\n        """a.b.input.Y"""\n',
        {"a.b.input.Y": "live"},
    ),
    # A comment ABOUT a skip is not a skip.
    (
        ".rs",
        '// clause: a.b.error.X\n// This was #[ignore]d until the symbol landed.\n#[test]\nfn t() {\n}\n',
        {"a.b.error.X": "live"},
    ),
    # apcore-typescript: the id is inside the declaration call.
    (".ts", "  it.skip('a.b.property.Z: reason', () => {\n  });\n", {"a.b.property.Z": "skipped"}),
    (".ts", "  it('a.b.property.Z: reason', () => {\n  });\n", {"a.b.property.Z": "live"}),
]


def self_test() -> list[str]:
    """Return a failure line per sample the detector reads wrongly."""
    import tempfile

    bad: list[str] = []
    for ext, source, expected in SELF_TEST:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / ("sample" + ext)).write_text(source)
            got = scan(d, ext)
        for cid, want in expected.items():
            if got.get(cid) != want:
                bad.append(
                    f"{ext} sample: {cid} read as {got.get(cid)!r}, expected {want!r}\n"
                    f"    {source!r}"
                )
    return bad


def asymmetries(sdk_root: Path) -> dict[str, dict[str, str]]:
    seen = {sdk: scan(sdk_root / d / "tests", SDK_EXT[sdk]) for sdk, d in SDK_DIRS.items()}
    every_id = set().union(*(s.keys() for s in seen.values()))
    out: dict[str, dict[str, str]] = {}
    for cid in sorted(every_id):
        states = {sdk: seen[sdk].get(cid, "absent") for sdk in SDK_DIRS}
        values = set(states.values())
        if "skipped" in values and "live" in values:
            out[cid] = states
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sdk-root", type=Path, default=REPO.parent)
    ap.add_argument("--strict", action="store_true", help="exit 1 on any asymmetry")
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="check the detector against each suite's convention and exit",
    )
    ap.add_argument("--write-baseline", action="store_true", help="record the current set")
    args = ap.parse_args()

    if args.self_test:
        bad = self_test()
        for line in bad:
            print(f"DETECTOR WRONG: {line}")
        print(f"detector self-test: {len(SELF_TEST)} samples, {len(bad)} wrong")
        return 1 if bad else 0

    # The detector is checked on every run: a count produced by a detector that
    # cannot read one of the suites is worse than no count, because it reports
    # zero gaps for that SDK and the report looks clean.
    detector_bad = self_test()
    if detector_bad:
        print("DETECTOR IS WRONG — the counts below cannot be trusted:")
        for line in detector_bad:
            print(f"  {line}")
        return 1

    found = asymmetries(args.sdk_root)
    doc = json.loads(BASELINE.read_text()) if BASELINE.exists() else {"ratchet": 10**6, "accepted": {}}
    accepted: dict[str, dict] = doc.get("accepted", {})

    rot: list[str] = []
    for cid, entry in sorted(accepted.items()):
        recorded = {k: v for k, v in entry.items() if k in SDK_DIRS}
        if cid not in found:
            rot.append(
                f"{cid}: recorded as an accepted asymmetry and is no longer one — "
                f"the entry describes a gap that is gone"
            )
        elif found[cid] != recorded:
            rot.append(
                f"{cid}: state changed {recorded} -> {found[cid]}; the recorded "
                f"reason was written about the old shape"
            )

    fresh = {cid: st for cid, st in found.items() if cid not in accepted}

    print(f"clauses disabled in one SDK and live in another: {len(found)}  (ratchet {doc['ratchet']})")
    print(f"  recorded: {len(accepted)}   NEW: {len(fresh)}")

    if rot:
        print("\nBASELINE ROT — these are not backlog, the record is lying:")
        for line in rot:
            print(f"  {line}")

    if fresh:
        print("\nNEW asymmetries — each is a claim about an SDK that nobody has checked:")
        for cid, st in sorted(fresh.items()):
            marks = "  ".join(f"{k[:2]}={v}" for k, v in st.items())
            print(f"  {cid:<62} {marks}")

    if args.write_baseline:
        if len(found) > doc["ratchet"]:
            print(f"\nREFUSED: ratchet may only go down ({doc['ratchet']} -> {len(found)}).")
            return 1
        doc["accepted"] = {
            cid: {**st, "note": accepted.get(cid, {}).get("note", "recorded, not yet examined")}
            for cid, st in found.items()
        }
        doc["ratchet"] = len(found)
        BASELINE.write_text(json.dumps(doc, indent=2, ensure_ascii=False) + "\n")
        print(f"\nbaseline recorded: {len(found)}")
        return 0

    if rot:
        return 1
    if len(found) > doc["ratchet"]:
        print(f"\nFAIL: {len(found)} asymmetries, ratchet is {doc['ratchet']}.")
        return 1
    if args.strict and found:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
