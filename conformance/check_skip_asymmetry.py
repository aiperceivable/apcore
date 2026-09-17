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

Without --strict this reports and exits 0 except on baseline ROT. The ratchet
may only go down.
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
DISABLED = re.compile(r"it\.skip\(|describe\.skip\(|#\[ignore|pytest\.mark\.skip|pytest\.skip\(")

# How far above a clause id a disabling marker may sit and still govern it:
# `#[ignore = "..."]` and `@pytest.mark.skip(...)` precede the function they
# disable, and the reason itself often carries the id.
MARKER_LOOKBEHIND = 3


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
                window = "\n".join(lines[max(0, i - MARKER_LOOKBEHIND) : i + 1])
                state = "skipped" if DISABLED.search(window) else "live"
                if out.get(cid) != "live":
                    out[cid] = state
    return out


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
    ap.add_argument("--write-baseline", action="store_true", help="record the current set")
    args = ap.parse_args()

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
