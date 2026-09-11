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


def sibling_values(cases: list[dict], key: str) -> list:
    """Every value `key` takes across the fixture's own cases.

    Used to mutate an enum-ish expectation ACROSS its semantic boundary rather
    than past the end of it. `mutate` appends a sentinel to a string, which is
    a value no implementation produces — but a driver that only asks
    `expected_load === "ok"` and takes the else-branch is asserting *class
    membership*, not the value, so `"reject"` -> `"reject__MUTATED__"` leaves
    the same branch running and the case reads as unpinned.
    Measured on `acl_effect_value_closure`, whose TypeScript driver runs all
    eleven of its tests and was nevertheless reported as running none of them.

    Flipping to a value the SAME key takes elsewhere in the fixture crosses the
    boundary without a hardcoded table of verdict words: if the corpus uses
    `ok`/`reject`, the flip is `ok`, and if it uses one value throughout there
    is no boundary to cross and the sentinel remains the best available.
    """
    seen = []
    for case in cases:
        v = case.get(key)
        if isinstance(v, str) and v not in seen:
            seen.append(v)
    return seen


#: The mutation strategies, tried in order. A case is pinned when ANY of them
#: makes a driver go red, because each has a blind spot the other covers and
#: neither alone is sound:
#:
#: * `sibling` flips a string to another value the same key takes elsewhere in
#:   the fixture. Needed because appending to `"reject"` leaves it != `"ok"`, so
#:   a driver branching on class membership never notices.
#: * `append` puts a sentinel on the end. Needed because two sibling values can
#:   be INDISTINGUISHABLE to one SDK — apcore-rust maps `version_negotiation`'s
#:   `PARSE_ERROR` onto `VERSION_INCOMPATIBLE` by documented design, so flipping
#:   between them is a no-op there while appending is not.
#:
#: Both blind spots were measured, one before this list existed and one after
#: `sibling` was added on its own.
STRATEGIES = ("sibling", "append")


def mutate(value, alternatives: list | None = None, strategy: str = "sibling"):
    """Return a value no correct implementation can produce.

    Every leaf changes, so a driver asserting any part of the block fails. Prose
    notes inside `expected` (`error_class_name_is_not_the_contract` and its kind)
    mutate too and are simply not asserted — the case is judged by its real
    assertions, which is the intent.
    """
    if isinstance(value, bool):
        return not value
    if isinstance(value, str):
        # Cross the boundary when the fixture shows where it is (see
        # `sibling_values`); otherwise go past the end of the value space.
        if strategy == "sibling":
            for other in alternatives or ():
                if other != value:
                    return other
        return value + SENTINEL
    if isinstance(value, (int, float)):
        return value + 987654
    if isinstance(value, list):
        return [mutate(v, alternatives, strategy) for v in value] if value else [SENTINEL]
    if isinstance(value, dict):
        # An EMPTY dict must not mutate to itself. The comprehension alone
        # returned `{}` unchanged, so any case expecting `{}` could never go red
        # and was reported unpinned no matter what its driver did — a false
        # positive manufactured by the tool. `schema_strict_conversion`'s
        # `empty_schema_passthrough` was exactly that.
        return ({k: mutate(v, alternatives, strategy) for k, v in value.items()}
                if value else {SENTINEL: SENTINEL})
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


#: Files under `tests/` that are harnesses rather than drivers.
#:
#: apcore-rust's `tests/it.rs` is a list of `#[path = "…"] mod …;` lines and
#: nothing else — every driver it aggregates is a separate file this scan finds
#: on its own. But its module NAMES contain the fixture stems (`mod
#: test_acl_handler_error_conformance;` contains `acl_handler_error`), so a
#: plain substring match claims `it.rs` drives almost every fixture.
#:
#: That was not merely redundant, it dominated the sweep. `it` is a declared
#: `[[test]]` target, so `run_drivers` selected it with `--test it` — no filter
#: — and ran the WHOLE 2100-test binary once per mutated case. Measured:
#: 2.64s for the unfiltered binary against 0.10s for the two real drivers
#: combined, i.e. 2.85s per case where 0.20s was the work. Over a full sweep
#: that is the difference between roughly an hour and a few minutes, and it is
#: why the Rust scope was always the slow one.
_HARNESS_FILES = {"it.rs"}


def driver_files(sdk_root: Path) -> dict[str, dict[str, list[Path]]]:
    """{fixture_stem: {sdk: [test files that mention it]}}."""
    stems = [p.stem for p in FIXTURES.glob("*.json")]
    out: dict[str, dict[str, list[Path]]] = {s: {} for s in stems}
    for sdk, repo, suffix in SDKS:
        base = sdk_root / repo / "tests"
        if not base.is_dir():
            continue
        for path in base.rglob(f"*{suffix}"):
            if path.name in _HARNESS_FILES:
                continue
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
    """True when the given driver files FAIL. See :func:`run_drivers_verbose`."""
    return run_drivers_verbose(sdk, sdk_root, files)[0]


def run_drivers_verbose(sdk: str, sdk_root: Path, files: list[Path]) -> tuple[bool, str]:
    """`(failed, combined output)`. Timeouts and crashes count as red.

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
    # The drivers must exercise the CHECKOUT, not whatever `apcore` happens to be
    # installed. CI gets that from `pip install -e`, but a developer with a stale
    # non-editable wheel in site-packages runs the whole sweep against it: the
    # Python drivers go red before any mutation and 36 of 73 fixtures report
    # INDETERMINATE, which reads as "the suite is broken" rather than "the tool
    # is looking at the wrong code". Measured locally, and it costs the sweep
    # half its coverage silently.
    extra_env = {"CONFORMANCE_SPEC_REPO": str(REPO)}
    if sdk == "python":
        src = repo / "src"
        if src.is_dir():
            existing = os.environ.get("PYTHONPATH", "")
            extra_env["PYTHONPATH"] = f"{src}{os.pathsep}{existing}" if existing else str(src)
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
    transcript = ""
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
                                  env={**os.environ, **extra_env})
        except subprocess.TimeoutExpired:
            return True, transcript + f"\n<timeout: {' '.join(cmd)}>"
        text = proc.stdout + proc.stderr
        transcript += text
        if proc.returncode != 0:
            return True, transcript
        if not ran_nothing(sdk, text):
            ran_something = True
    if not ran_something:
        raise NoTestsRan(" | ".join(" ".join(c) for c in cmds))
    return False, transcript


def ran_nothing(sdk: str, output: str) -> bool:
    """Whether a zero-exit run actually executed no tests."""
    if sdk == "python":
        return "no tests ran" in output or "collected 0 items" in output
    if sdk == "typescript":
        return "No test files found" in output or "no tests" in output.lower()
    counts = [int(n) for n in re.findall(r"test result: ok\. (\d+) passed", output)]
    return bool(counts) and sum(counts) == 0


#: Where each framework starts NAMING what failed. Everything before the marker
#: is progress output, and for vitest and pytest that includes the names of
#: tests that PASSED — which is why attribution has to be scoped rather than run
#: over the whole transcript. Measured: vitest's green run of
#: `acl_handler_error` prints all fifteen case ids as `✓` lines, so an unscoped
#: predicate attributes everything and the batch is always thrown away.
#:
#: A marker that is wrong or missing costs nothing but speed: the green baseline
#: then attributes nothing (there is no failure section in a passing run), the
#: red run attributes nothing either, and `batch_probe` raises `Unattributable`
#: and hands the pair to the per-case path.
_FAILURE_MARKERS = {
    "python": ("short test summary info",),
    "typescript": ("Failed Tests", "FAIL "),
    # cargo prints `test <name> ... ok` for passes, so the `failures:` block is
    # the only safe region for a per-case-test driver. Drivers that AGGREGATE
    # into one assertion put the ids in the panic message instead, which appears
    # under `---- <test> stdout ----`.
    "rust": ("failures:", "---- ", "panicked at"),
}


def failure_region(sdk: str, output: str) -> str:
    """The tail of `output` from the first point at which failures are named."""
    starts = [output.find(m) for m in _FAILURE_MARKERS.get(sdk, ())]
    starts = [i for i in starts if i >= 0]
    return output[min(starts):] if starts else ""


def attribute(output: str, case_ids: list[str]) -> set[str]:
    """Which case ids the driver output names.

    One uniform predicate across three frameworks, because they do not report
    the same way and the drivers do not all name a test per case:

    * apcore-python parametrizes with ``ids=lambda c: c["id"]``, so the id is in
      the test NAME (`test_case[<id>]`).
    * apcore-typescript spells its `it(...)` title with the id first, likewise.
    * apcore-rust has drivers that collect into a `failures` vec and assert
      ONCE, so the whole fixture is a single test and the ids live in the
      assertion MESSAGE (`  [<id>] …`).

    Matching the id anywhere in the failure output covers all three. It is a
    coarse predicate on purpose: a FALSE POSITIVE here would mark a dead case
    "pinned" and skip re-verifying it, so it is never trusted on its own —
    :func:`batch_probe` validates it against the unmutated baseline first, and
    anything unattributed falls through to the per-case path that has always
    been the tool's ground truth.
    """
    return {cid for cid in case_ids if cid in output}


def attribute_failures(sdk: str, output: str, case_ids: list[str]) -> set[str]:
    """:func:`attribute`, scoped to the region where failures are named."""
    return attribute(failure_region(sdk, output), case_ids)


class Unattributable(Exception):
    """The batch run's output cannot be tied back to case ids."""


def batch_probe(
    sdk: str,
    sdk_root: Path,
    files: list[Path],
    path: Path,
    backup: Path,
    cases: list[dict],
    baseline_output: str,
    strategy: str,
) -> set[str]:
    """Mutate EVERY case at once and report which ids the driver names.

    Returns the set of case ids this SDK demonstrably runs. Raises
    :class:`Unattributable` when the result cannot be trusted, in which case the
    caller falls back to one mutation per case — the slow path, which is the
    only one whose verdict is unambiguous.

    Two things make batching sound rather than merely fast:

    1. **The baseline must attribute nothing.** The unmutated run is green, so
       no id should appear in a failure position. If one does, this fixture's
       output is not a reliable index of which cases failed, and the whole
       (fixture, SDK) pair goes to the slow path.
    2. **A red run that names nothing is unattributable.** Red with no ids means
       the driver failed for a reason other than the mutations — a parse error,
       a load-time precondition — and reading that as "every case is pinned"
       would be the exact false green this tool exists to prevent.

    A GREEN batch run needs no fallback and is the strongest result available:
    every case was mutated and nothing went red, so this driver runs none of
    them.
    """
    ids = [c["id"] for c in cases]
    stray = attribute_failures(sdk, baseline_output, ids)
    if stray:
        raise Unattributable(
            f"the unmutated run already names {len(stray)} case id(s), so its "
            f"output does not indicate failure"
        )

    mutated = json.loads(backup.read_text())
    for case in mutated["test_cases"]:
        for target in expectation_keys(case):
            case[target] = mutate(case[target], sibling_values(cases, target), strategy)
    path.write_text(json.dumps(mutated, indent=2, ensure_ascii=False) + "\n")
    try:
        red, output = run_drivers_verbose(sdk, sdk_root, files)
    finally:
        shutil.copy(backup, path)

    if not red:
        return set()
    found = attribute_failures(sdk, output, ids)
    if not found:
        raise Unattributable("the run went red but named no case id")
    return found


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
    ap.add_argument("--per-case", action="store_true",
                    help="mutate one case at a time instead of the two-pass sweep. The "
                         "slow path, and the tool's ground truth: the two-pass sweep "
                         "falls back to it per (fixture, SDK) whenever a batch result "
                         "cannot be attributed, and this flag forces it everywhere.")
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
            baseline: dict[str, str] = {}
            for sdk, *_ in sdks:
                if not by_sdk.get(sdk):
                    continue
                try:
                    red, baseline[sdk] = run_drivers_verbose(sdk, args.sdk_root, by_sdk[sdk])
                    if red:
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

            # PASS 1 — mutate every case at once, once per SDK, and read back
            # which ids the driver named. A case named by ANY sdk in scope is
            # pinned and needs no second run; only what is left over is charged
            # for a per-case mutation. On a healthy corpus that leftover is
            # empty, so a fixture costs one run per SDK instead of one per case.
            #
            # `pass1[sdk]` is None when that SDK's batch could not be attributed
            # — the pair then takes the slow path and its verdict is the old,
            # unambiguous one.
            measurable = [c for c in cases if expectation_keys(c)]
            pass1: dict[str, set[str] | None] = {}
            if not args.per_case:
                for sdk, *_ in sdks:
                    files = by_sdk.get(sdk)
                    if not files:
                        continue
                    found: set[str] | None = set()
                    for strategy in STRATEGIES:
                        remaining = [c for c in measurable if c["id"] not in (found or ())]
                        if not remaining:
                            break
                        try:
                            found = (found or set()) | batch_probe(
                                sdk, args.sdk_root, files, path, backup,
                                remaining, baseline.get(sdk, ""), strategy)
                        except (Unattributable, NoTestsRan) as exc:
                            found = None
                            print(f"  per-case  {path.stem} [{sdk}] — {exc}", flush=True)
                            break
                    pass1[sdk] = found

            #: Attributed by at least one SDK whose batch WAS trustworthy. These
            #: skip pass 2; everything else is re-measured the old way.
            batch_pinned = {cid for got in pass1.values() if got for cid in got}

            for case in cases:
                cid = case["id"]
                targets = expectation_keys(case)
                if not targets:
                    no_expected.setdefault(path.name, []).append(cid)
                    continue
                checked += 1
                if cid in batch_pinned:
                    continue
                pinned_by = None
                for strategy in STRATEGIES:
                    if pinned_by is not None:
                        break
                    mutated = json.loads(backup.read_text())
                    for c in mutated["test_cases"]:
                        if c.get("id") == cid:
                            for key in targets:
                                c[key] = mutate(c[key], sibling_values(cases, key), strategy)
                    path.write_text(json.dumps(mutated, indent=2, ensure_ascii=False) + "\n")
                    for sdk, *_ in sdks:
                        files = by_sdk.get(sdk)
                        if not files:
                            continue
                        # A trustworthy GREEN batch already proved this SDK runs
                        # none of the fixture's cases; re-running it per case can
                        # only reproduce that, one process at a time.
                        if not args.per_case and pass1.get(sdk) == set():
                            continue
                        try:
                            if run_drivers(sdk, args.sdk_root, files):
                                pinned_by = sdk
                                break
                        except NoTestsRan:
                            # Cannot conclude anything; the fixture pre-check
                            # above already rejected this shape, so reaching here
                            # means the invocation degraded mid-sweep.
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
