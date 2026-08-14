#!/usr/bin/env python3
"""Report fixture cases that no SDK driver actually runs.

Two guards already ship, and both answer a question adjacent to this one:

* ``check_driver_coverage.py`` — *does each SDK load this fixture?*
* ``check_expected_keys_read.py`` — *does any driver read this ``expected`` key?*

Neither answers **does any driver run this case**. A fixture can carry a case
every driver skips, quarantines or simply never reaches, and both stay green:
the fixture is loaded, and its ``expected`` key *names* are read — by some other
case in the same file. The case then reads as covered in the fixture, in review,
and in every count derived from the inventory, while nothing is checked.

Static detection does not work here, and the failed attempts are worth recording
so they are not retried:

* Scanning drivers for skip markers (``QUARANTINED``, ``skip``, ``xfail``,
  ``#[ignore]``) flagged 125 of 180 (fixture, SDK) pairs. The word ``skip``
  appears throughout large shared drivers, including inside case *names* such as
  ``skips_running_tasks``. Unusable.
* "Names some case ids but not all" flagged 25 pairs, most of them false: a
  driver that iterates generically may still mention one id in a comment or an
  extra targeted assertion.
* Adding "does the file iterate ``test_cases``" collapsed it to 0 suspects with
  one regex and 37 with a slightly tighter one, almost all TypeScript — the
  answer moved with the regex rather than with the code. A guard whose result
  depends on how its pattern is spelled measures the pattern.

So this checks the property directly: **mutate the case's ``expected`` block so
no correct implementation can satisfy it, run the drivers that reference the
fixture, and see whether anything goes red.** A case no driver runs cannot go
red. That is the same instrument used to verify every fix in the 0.27 sweep, and
it is the only one that caught a driver reading ``expected.wrapped_in`` and
asserting nothing.

    python3 conformance/check_case_pinning.py [--fixture NAME] [--sdk-root DIR]
                                              [--strict] [--write-baseline]

It runs test processes, so it is slow — minutes, not seconds. It belongs in a
scheduled job or a local sweep, not on the per-PR path. ``--strict`` exits 1 on
any unpinned case; the default reports and exits 0.

MUTATION IS DESTRUCTIVE: the fixture file is rewritten and restored around each
case. It restores on exceptions and on SIGINT, but do not run it on a dirty
fixture tree — check `git status` first.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "conformance" / "fixtures"
BASELINE = REPO / "conformance" / "case_pinning_baseline.json"
ALLOWLIST = REPO / "conformance" / "case_pinning_allowlist.json"

SENTINEL = "__APCORE_MUTATION_CANARY__"

# Ordered cheapest-first: a case pinned by the first SDK needs no further runs.
SDKS: list[tuple[str, str, str]] = [
    ("python", "apcore-python", ".py"),
    ("rust", "apcore-rust", ".rs"),
    ("typescript", "apcore-typescript", ".ts"),
]


def mutate(value):
    """Return a value no correct implementation can produce.

    Every leaf changes, so a driver asserting any part of the block fails. Prose
    notes inside `expected` (`error_class_name_is_not_the_contract` and its kind)
    mutate too and are simply not asserted — the case is judged by its real
    assertions, which is the intent.
    """
    if isinstance(value, bool):
        return not value
    if isinstance(value, str):
        return value + SENTINEL
    if isinstance(value, (int, float)):
        return value + 987654
    if isinstance(value, list):
        return [mutate(v) for v in value] if value else [SENTINEL]
    if isinstance(value, dict):
        # An EMPTY dict must not mutate to itself. The comprehension alone
        # returned `{}` unchanged, so any case expecting `{}` could never go red
        # and was reported unpinned no matter what its driver did — a false
        # positive manufactured by the tool. `schema_strict_conversion`'s
        # `empty_schema_passthrough` was exactly that.
        return {k: mutate(v) for k, v in value.items()} if value else {SENTINEL: SENTINEL}
    return SENTINEL


#: Fixtures are not uniform: 242 of 658 cases carry no `expected` object at all
#: and state it with a prefixed top-level key instead — `expected_valid` (134
#: cases), `expected_features`, `expected_error`, `expected_path`,
#: `expected_score`. The first version measured only `expected` and so silently
#: skipped 37% of the corpus while reporting a confident number for the rest.
#:
#: The prefix is the WHOLE rule, deliberately. An earlier revision also counted
#: `error_code`, which reads like an expectation and is one nowhere: in all 21
#: cases that carry it — `error_codes.json`, `binding_errors.json` — it is the
#: code being REGISTERED, an input. Mutating it changed what the test did rather
#: than what it expected, so a driver reacting to the altered input was scored as
#: asserting the declared expectation. That is a false NEGATIVE: it reports
#: coverage that is not there, which is the one direction this tool must never
#: err in. `error_message_contains` appears at top level in zero cases (it lives
#: inside `expected`, where it is mutated anyway), so the list is now empty and
#: kept only to make its emptiness deliberate rather than accidental.
_EXTRA_EXPECTATION_KEYS: frozenset[str] = frozenset()


def expectation_keys(case: dict) -> list[str]:
    """Top-level keys of `case` that state an expectation.

    Anything spelled `expected` or `expected_*`, plus the small set above. A case
    with none of them cannot be probed by mutation — it is reported rather than
    counted as passing, because "not measurable" and "measured and fine" are the
    two answers this whole exercise exists to keep apart.
    """
    return sorted(k for k in case
                  if k == "expected" or k.startswith("expected_") or k in _EXTRA_EXPECTATION_KEYS)


def driver_files(sdk_root: Path) -> dict[str, dict[str, list[Path]]]:
    """{fixture_stem: {sdk: [test files that mention it]}}."""
    stems = [p.stem for p in FIXTURES.glob("*.json")]
    out: dict[str, dict[str, list[Path]]] = {s: {} for s in stems}
    for sdk, repo, suffix in SDKS:
        base = sdk_root / repo / "tests"
        if not base.is_dir():
            continue
        for path in base.rglob(f"*{suffix}"):
            try:
                text = path.read_text()
            except (UnicodeDecodeError, OSError):
                continue
            for stem in stems:
                if stem in text:
                    out[stem].setdefault(sdk, []).append(path)
    return out


def rust_targets(repo: Path) -> set[str]:
    """The `[[test]]` target names declared in Cargo.toml.

    apcore-rust sets `autotests = false`, so a file under `tests/` is either its
    own declared target or a MODULE of `tests/it.rs`. The two need different
    invocations and getting it wrong is silent — see `run_drivers`.
    """
    try:
        text = (repo / "Cargo.toml").read_text()
    except OSError:
        return set()
    return set(re.findall(r'^\s*name\s*=\s*"([^"]+)"', text, re.M)) - {"apcore"}


class NoTestsRan(RuntimeError):
    """The invocation executed zero tests, so its exit code means nothing."""


def run_drivers(sdk: str, sdk_root: Path, files: list[Path]) -> bool:
    """True when the given driver files FAIL. Timeouts and crashes count as red.

    A crash is a legitimate red: a driver that blows up on a mutated expectation
    was reading it.

    Raises `NoTestsRan` when the invocation executed nothing. That is NOT green:
    a filter matching no test exits 0, and reading that as "nothing went red"
    reports the case as unpinned no matter what its driver does. It happened —
    `cargo test -- conformance_test` filters by test NAME, and no test in that
    target is named after its file, so every Rust verdict in the first per-SDK
    sweep was manufactured by this bug rather than measured.
    """
    repo = sdk_root / dict((s, r) for s, r, _ in SDKS)[sdk]
    if sdk == "python":
        cmds = [[sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider",
                 *[str(f.relative_to(repo)) for f in files]]]
    elif sdk == "typescript":
        cmds = [["npx", "vitest", "run", *[str(f.relative_to(repo)) for f in files]]]
    else:
        # A declared [[test]] target is selected with `--test <name>`; anything
        # else is a module of `it.rs`, reached by filtering the `it` binary on
        # the module name.
        #
        # ONE COMMAND PER TARGET, deliberately. Combining them —
        # `--test conformance_test --test it -- test_errors` — applies the
        # filter to BOTH binaries, so the target that has no test matching
        # `test_errors` runs nothing while the other runs something, the summed
        # count is non-zero, and the zero-tests guard below stays quiet. The
        # real driver is silently filtered out and every case reports unpinned.
        targets = rust_targets(repo)
        cmds = []
        for stem in sorted({f.stem for f in files}):
            if stem in targets:
                cmds.append(["cargo", "test", "--all-features", "--test", stem])
            else:
                cmds.append(["cargo", "test", "--all-features", "--test", "it", "--", stem])
        if not cmds:
            raise NoTestsRan("no Rust target or module resolved")

    ran_something = False
    for cmd in cmds:
        try:
            # Point the drivers at the fixture tree THIS run is mutating. Without
            # it they fall through to their own sibling-directory lookup and
            # validate whichever `../apcore` happens to be there: tests run,
            # `NoTestsRan` stays quiet, and every case reports unpinned because
            # the mutation landed in a file nothing read. Worse than the earlier
            # no-op bug, which at least produced a suspicious zero — this one
            # produces a plausible non-zero against the wrong corpus.
            proc = subprocess.run(cmd, cwd=repo, capture_output=True, text=True,
                                  timeout=600,
                                  env={**os.environ, "CONFORMANCE_SPEC_REPO": str(REPO)})
        except subprocess.TimeoutExpired:
            return True
        if proc.returncode != 0:
            return True
        if not ran_nothing(sdk, proc.stdout + proc.stderr):
            ran_something = True
    if not ran_something:
        raise NoTestsRan(" | ".join(" ".join(c) for c in cmds))
    return False


def ran_nothing(sdk: str, output: str) -> bool:
    """Whether a zero-exit run actually executed no tests."""
    if sdk == "python":
        return "no tests ran" in output or "collected 0 items" in output
    if sdk == "typescript":
        return "No test files found" in output or "no tests" in output.lower()
    counts = [int(n) for n in re.findall(r"test result: ok\. (\d+) passed", output)]
    return bool(counts) and sum(counts) == 0


def load_allowlist() -> dict[tuple[str, str], set[str]]:
    """{(fixture, case_id): {sdks that MUST NOT run it}} — deliberate skips.

    Some cases are per-SDK by construction and a driver is *required* to skip
    them. `redaction_config`'s `legacy_config_key_is_honoured_with_a_deprecation_
    warning` is the worked example: it carries `legacy_key_by_sdk.python: null`
    and a `skip_when_legacy_key_is_null` note stating that apcore-python MUST NOT
    gain the fallback, because no Python deployment ever had that spelling and
    adding it would be a security-relevant regression.

    Without this list `--sdk python` reports that case as a gap, which is the
    asymmetric-skip false positive: a case one SDK is *supposed* to skip looks
    identical to a case it forgot to run. Only `--sdk` mode consults it; the
    default question ("does ANY driver run this case") is unaffected, and that
    is the question a genuinely dead case fails.
    """
    if not ALLOWLIST.is_file():
        return {}
    doc = json.loads(ALLOWLIST.read_text())
    return {(e["fixture"], e["case"]): set(e["sdks"]) for e in doc.get("allow", [])}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sdk-root", type=Path, default=REPO.parent)
    ap.add_argument("--fixture", help="limit to one fixture (stem, no .json)")
    ap.add_argument("--sdk", choices=[s[0] for s in SDKS],
                    help="ask the question of ONE SDK: which cases does it not run? "
                         "The default asks whether ANY driver runs the case, which is "
                         "the weaker question — a case only one SDK drives still "
                         "proves one implementation, not three.")
    ap.add_argument("--strict", action="store_true", help="exit 1 on any unpinned case")
    ap.add_argument("--write-baseline", action="store_true")
    args = ap.parse_args()

    sdks = [t for t in SDKS if not args.sdk or t[0] == args.sdk]
    allow = load_allowlist()
    drivers = driver_files(args.sdk_root)
    paths = sorted(FIXTURES.glob("*.json"))
    if args.fixture:
        paths = [p for p in paths if p.stem == args.fixture]
        if not paths:
            print(f"no fixture named {args.fixture!r}", file=sys.stderr)
            return 2

    scratch = Path(tempfile.mkdtemp(prefix="apcore-pinning-"))
    unpinned: dict[str, list[str]] = {}
    no_expected: dict[str, list[str]] = {}
    indeterminate: dict[str, str] = {}
    checked = 0

    def restore_all(*_):
        for backup in scratch.glob("*.json"):
            shutil.copy(backup, FIXTURES / backup.name)
        print("\nfixtures restored", file=sys.stderr)

    signal.signal(signal.SIGINT, lambda *_: (restore_all(), sys.exit(130)))

    try:
        for path in paths:
            doc = json.loads(path.read_text())
            cases = [c for c in doc.get("test_cases", []) if isinstance(c, dict) and c.get("id")]
            if not cases:
                continue
            backup = scratch / path.name
            shutil.copy(path, backup)
            by_sdk = drivers.get(path.stem, {})

            # The whole method reads "red under mutation" as "the case is run",
            # which is only meaningful if the drivers are GREEN unmutated. A
            # fixture whose tests already fail would report every one of its
            # cases as pinned — the tool would be at its most reassuring exactly
            # where the suite is broken.
            already_red = []
            broken = []
            for sdk, *_ in sdks:
                if not by_sdk.get(sdk):
                    continue
                try:
                    if run_drivers(sdk, args.sdk_root, by_sdk[sdk]):
                        already_red.append(sdk)
                except NoTestsRan as exc:
                    broken.append(f"{sdk} ran no tests ({exc})")
            if broken:
                indeterminate[path.name] = "; ".join(broken)
                print(f"  INDETERMINATE  {path.stem} — {indeterminate[path.name]}", flush=True)
                continue
            if already_red:
                indeterminate[path.name] = ", ".join(already_red)
                print(f"  INDETERMINATE  {path.stem} — red before mutation on "
                      f"{indeterminate[path.name]}", flush=True)
                continue

            for case in cases:
                cid = case["id"]
                targets = expectation_keys(case)
                if not targets:
                    no_expected.setdefault(path.name, []).append(cid)
                    continue
                checked += 1
                mutated = json.loads(backup.read_text())
                for c in mutated["test_cases"]:
                    if c.get("id") == cid:
                        for key in targets:
                            c[key] = mutate(c[key])
                path.write_text(json.dumps(mutated, indent=2, ensure_ascii=False) + "\n")
                pinned_by = None
                for sdk, *_ in sdks:
                    files = by_sdk.get(sdk)
                    if not files:
                        continue
                    try:
                        if run_drivers(sdk, args.sdk_root, files):
                            pinned_by = sdk
                            break
                    except NoTestsRan:
                        # Cannot conclude anything; the fixture pre-check above
                        # already rejected this shape, so reaching here means the
                        # invocation degraded mid-sweep.
                        pinned_by = "indeterminate"
                        break
                shutil.copy(backup, path)
                if pinned_by is not None:
                    continue
                skips = allow.get((path.name, cid), set())
                if args.sdk and args.sdk in skips:
                    # Required to skip, not failing to run — see load_allowlist.
                    continue
                unpinned.setdefault(path.name, []).append(cid)
                print(f"  UNPINNED  {path.stem} :: {cid}", flush=True)
    finally:
        restore_all()
        shutil.rmtree(scratch, ignore_errors=True)

    total_unpinned = sum(len(v) for v in unpinned.values())
    print(f"\n{checked} cases mutated across {len(paths)} fixtures — "
          f"{total_unpinned} pinned by no driver, in {len(unpinned)} fixtures")
    if indeterminate:
        print(f"{len(indeterminate)} fixture(s) were RED before mutation, so nothing "
              f"could be concluded about their cases:")
        for stem, sdks in sorted(indeterminate.items()):
            print(f"  {stem}: failing on {sdks}")
    if no_expected:
        n = sum(len(v) for v in no_expected.values())
        print(f"{n} cases state no expectation this tool can mutate — NOT MEASURED, which is "
              f"not the same as measured and fine:")
        for stem, ids in sorted(no_expected.items()):
            print(f"  {stem}: {', '.join(ids[:6])}{' …' if len(ids) > 6 else ''}")

    if args.write_baseline:
        # Partitioned by scope. "any" is the weak question — does SOME driver run
        # this case — and each SDK name is the real one. Storing only the total
        # would say a number moved without saying whose, and the per-SDK sets are
        # nearly disjoint: 84 gaps over 75 cases, only 9 shared by two SDKs and
        # none by all three, which is exactly why "any" read as zero while every
        # SDK still had its own blind spots (apcore#93).
        scope = args.sdk or "any"
        existing = (json.loads(BASELINE.read_text())
                    if BASELINE.is_file() and "by_scope" in BASELINE.read_text() else {})
        by_scope = existing.get("by_scope", {})
        by_scope[scope] = {"unpinned": {k: sorted(v) for k, v in sorted(unpinned.items())},
                           "not_measurable": {k: sorted(v) for k, v in sorted(no_expected.items())}}
        BASELINE.write_text(json.dumps(
            {"description":
                "Fixture cases that no SDK driver runs, accepted as a known backlog. "
                "check_case_pinning.py --strict fails on any unpinned case; this file records "
                "the ones already known so the backlog can shrink without new ones slipping in. "
                "A case here is NOT covered — it only looks covered, in the fixture and in every "
                "count derived from the inventory. Keyed by SCOPE: `any` is the weak question "
                "(does SOME driver run it) and each SDK name is the real one, because the "
                "per-SDK sets are nearly disjoint and a total hides which one regressed.",
             "by_scope": by_scope},
            indent=2) + "\n")
        print(f"wrote {BASELINE.relative_to(REPO)} (scope: {scope})")
        return 0

    if args.strict and total_unpinned:
        print(f"\n{total_unpinned} fixture case(s) are run by no driver. A case that cannot "
              f"go red is not coverage.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
