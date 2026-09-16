#!/usr/bin/env python3
"""Report decisions that no executed conformance case pins.

A decision recorded only in prose is a decision nothing verifies. This repository
has shipped two false statements of exactly that kind. D-96 closed by recording
that its union was "unobservable in implementations whose descriptors are derived
from the module (apcore-python, apcore-typescript)" — it was a fail-open approval
bypass in both, corrected four spec versions later as D-125. The v1.10.0 row
recorded that "all three SDKs accept" a version hint — apcore-rust has no such
parameter, corrected as D-126. Neither could ever have been caught by a test,
because the sentence itself said the test was unnecessary.

The fix is not to re-read the prose. It is to require that a decision about
cross-SDK behaviour names an executed case, and to count the ones that do not.

UNLINKED MEANS UNCOVERED, deliberately. A case that pins a decision without
saying so can be weakened by someone who does not know what it is for — which is
how `approval_gate.json` came to set both governance sources from a single
boolean and passed in all three SDKs while one of them had a bypass. Coverage
that nobody can see is coverage nobody can preserve.

    python3 conformance/check_decision_coverage.py [--strict] [--write-ratchet]

Without --strict this reports and exits 0, so the existing backlog does not block
unrelated work; it still fails on a map that has ROTTED (a decision missing from
the map, or a case reference that no longer resolves), because those are not
backlog, they are the map lying. With --strict it exits 1 while any behavioural
decision is unpinned — the setting to switch on once the ratchet reaches zero.

The ratchet may only go down. `--write-ratchet` records a new, lower value.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "conformance" / "fixtures"
MAP = REPO / "conformance" / "decision_coverage.json"
SOURCES = [
    REPO / "docs" / "spec" / "protocol-spec.md",
    REPO / "docs" / "spec" / "2026-09-deep-chain-decisions.md",
]
# The audit that produced this map covers D-74 onward. Earlier decisions predate
# it and are out of scope until someone backfills them; the bound is stated here
# rather than inferred so widening it is a deliberate edit.
FIRST_TRACKED = 74

SDK_DIRS = {"python": "apcore-python", "typescript": "apcore-typescript", "rust": "apcore-rust"}


def load_case_ids(path: Path) -> set[str]:
    """Every addressable case id in one fixture, for both fixture shapes."""
    if path.suffix in (".yaml", ".yml"):
        try:
            import yaml
        except ImportError:  # pragma: no cover - PyYAML is a dev dependency
            return set()
        doc = yaml.safe_load(path.read_text())
        # A binding fixture is a list of declarations, addressed by module_id.
        return {b["module_id"] for b in (doc or {}).get("bindings", []) if "module_id" in b}
    doc = json.loads(path.read_text())
    cases = doc.get("test_cases") or doc.get("cases") or []
    return {c["id"] for c in cases if isinstance(c, dict) and "id" in c}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sdk-root", type=Path, default=REPO.parent)
    ap.add_argument("--strict", action="store_true",
                    help="exit 1 while any behavioural decision is unpinned")
    ap.add_argument("--write-ratchet", action="store_true",
                    help="record a new, lower ratchet value")
    args = ap.parse_args()

    doc = json.loads(MAP.read_text())
    decisions = doc["decisions"]

    rot: list[str] = []

    # 1. Every decision named in the spec must appear in the map.
    named = {
        f"D-{n}"
        for src in SOURCES
        for n in re.findall(r"\bD-(\d{2,3})\b", src.read_text())
        if int(n) >= FIRST_TRACKED
    }
    for key in sorted(named - set(decisions), key=lambda k: int(k[2:])):
        rot.append(f"{key} appears in the spec and is absent from the map")

    # 2. Every case reference must resolve. A map that points at a case someone
    #    renamed is worse than an empty map: it reads as covered.
    cache: dict[str, set[str]] = {}
    for key, entry in sorted(decisions.items(), key=lambda kv: int(kv[0][2:])):
        for ref in entry.get("cases", []):
            if "#" not in ref:
                rot.append(f"{key}: case reference {ref!r} is not 'fixture#case_id'")
                continue
            fixture, case_id = ref.split("#", 1)
            path = FIXTURES / fixture
            if not path.exists():
                rot.append(f"{key}: fixture {fixture!r} does not exist")
                continue
            if fixture not in cache:
                cache[fixture] = load_case_ids(path)
            if case_id not in cache[fixture]:
                rot.append(f"{key}: {fixture} has no case {case_id!r}")

        # 3. A linked case must say what makes it discriminating. If that cannot
        #    be written, the case is not discriminating and the decision is
        #    still unpinned.
        if entry.get("cases") and not entry.get("discriminates"):
            rot.append(f"{key}: has cases and no `discriminates` line")

        # A named test file that no longer exists reads as coverage and is not.
        for sdk, files in (entry.get("sdk_tests") or {}).items():
            if sdk not in SDK_DIRS:
                rot.append(f"{key}: unknown SDK {sdk!r} in sdk_tests")
                continue
            for rel in files:
                if not (args.sdk_root / SDK_DIRS[sdk] / rel).exists():
                    rot.append(f"{key}: {sdk} test {rel!r} does not exist")
        for sdk in entry.get("binds") or []:
            if sdk not in SDK_DIRS:
                rot.append(f"{key}: `binds` names unknown SDK {sdk!r}")
        if entry["kind"] != "behavioural" and not entry.get("note"):
            rot.append(f"{key}: kind={entry['kind']} must carry a `note` saying why")

    # A decision counts as PINNED when a conformance case names it, or when
    # per-SDK regression tests cover every SDK the decision binds.
    #
    # The two are not equivalent and the map says which it has. A conformance
    # case is ONE document three drivers read, so the three cannot drift apart;
    # three separate test files can, and nothing notices when one is weakened.
    # But three red-first tests do stop a regression today, and reporting them
    # as no coverage would understate the work and overstate the backlog —
    # which is its own way of making a number stop meaning anything.
    def pinned(entry: dict) -> bool:
        if entry.get("cases"):
            return True
        tests = entry.get("sdk_tests") or {}
        binds = entry.get("binds") or list(SDK_DIRS)
        return bool(tests) and all(tests.get(sdk) for sdk in binds)

    gaps = sorted(
        (k for k, v in decisions.items() if v["kind"] == "behavioural" and not pinned(v)),
        key=lambda k: int(k[2:]),
    )
    by_case = sum(1 for v in decisions.values()
                  if v["kind"] == "behavioural" and v.get("cases"))
    by_tests = sum(1 for v in decisions.values()
                   if v["kind"] == "behavioural" and not v.get("cases") and pinned(v))
    kinds: dict[str, int] = {}
    for v in decisions.values():
        kinds[v["kind"]] = kinds.get(v["kind"], 0) + 1

    print(f"decisions tracked: {len(decisions)}  " +
          "  ".join(f"{k}={n}" for k, n in sorted(kinds.items())))
    print(f"behavioural pinned by a conformance case: {by_case}")
    print(f"behavioural pinned by per-SDK tests only : {by_tests}")
    print(f"behavioural unpinned:                     {len(gaps)}  (ratchet {doc['ratchet']})")

    if rot:
        print("\nMAP ROT — these are not backlog, the map is lying:")
        for line in rot:
            print(f"  {line}")

    if gaps:
        print("\nUnpinned behavioural decisions:")
        for key in gaps:
            print(f"  {key}  {decisions[key].get('title', '')[:88]}")

    if args.write_ratchet:
        if len(gaps) > doc["ratchet"]:
            print(f"\nREFUSED: ratchet may only go down ({doc['ratchet']} -> {len(gaps)}).")
            return 1
        doc["ratchet"] = len(gaps)
        MAP.write_text(json.dumps(doc, indent=2) + "\n")
        print(f"\nratchet recorded: {len(gaps)}")
        return 0

    if rot:
        return 1
    if len(gaps) > doc["ratchet"]:
        print(f"\nFAIL: {len(gaps)} unpinned, ratchet is {doc['ratchet']} — a new decision "
              "landed without a case.")
        return 1
    if args.strict and gaps:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
