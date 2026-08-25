#!/usr/bin/env python3
"""Fail on `sys.` used as a module-ID namespace.

The apcore control plane is `system.*`. There is no `sys.*` namespace, and `sys`
is not a reserved word — PROTOCOL_SPEC 2.5 reserves eight (`system`, `internal`,
`core`, `apcore`, `plugin`, `schema`, `acl`, `ephemeral`), and the three SDKs
reserve the first seven in `RESERVED_WORDS`. So `sys.control.reload_module` is
not a privileged ID that needs a bypass; it is an ordinary ID anyone can
register, naming a module that does not exist.

Why a guard rather than three edits (apcore#98): the same wrong namespace reached
three documents in this repository, a fourth and fifth in
docs/spec/security-considerations.md that the issue did not count, and the three
`acl_builder` sources in the MCP adapters (apcore-mcp#14). Independent
rediscovery at that rate is what a guard is for.

The two failure modes are not equally loud:

  * In prose, a reader is told the wrong name.
  * In an ACL rule, a pattern matching no module is skipped in silence — a deny
    rule an operator believes is protecting the management surface, protecting
    nothing. `docs/spec/security-considerations.md`'s own audit checklist had
    exactly this shape.

Scope, deliberately narrow: only `sys.` immediately followed by a lowercase
module-ID segment. Host-language and config-path spellings are not module IDs
and are excluded outright (Python's `sys.path` / `sys.exit`, the `sys.modules.*`
config key that `APCORE_SYS_MODULES_*` maps to). Everything else needs an
allowlist entry with a reason, reported STALE once it stops matching.

    python3 conformance/check_module_namespace.py [--paths P ...]

Exit 1 on any new occurrence.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ALLOWLIST = REPO / "conformance" / "module_namespace_allowlist.json"

# `sys.` followed by something ID-shaped, matched to the END of the dotted ID.
# The apcore ID grammar is ^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$ (2.1), so a
# capital or a digit right after a dot is not a module ID and never reaches here.
#
# Matching the WHOLE id, not just the first segment after `sys.`, is load-bearing:
# an allowlist entry is keyed on the matched text, so a one-segment match made
# `sys.control` the key for every `sys.control.*` there is — and the exemption
# written for `sys.control.reload_module` silently covered a newly-introduced
# `sys.control.shutdown`. An allowlist that widens itself is worse than none,
# because it reads as a reviewed exception.
_SEG = r"(?:\*|[a-z][a-z0-9_]*)"
PATTERN = re.compile(rf"\bsys\.{_SEG}(?:\.{_SEG})*")

# Not module IDs, in any file, ever. Compared against the first TWO segments of
# the match, so `sys.modules.reload.enabled` is covered by `sys.modules`.
# Listing them here rather than in the allowlist keeps the allowlist to things
# that are actually exceptions.
NOT_A_MODULE_ID = {
    # Python standard library — `sys.path`, `sys.exit`, `sys.stderr`, ...
    "sys.path", "sys.argv", "sys.exit", "sys.stderr", "sys.stdout",
    "sys.version", "sys.executable", "sys.platform", "sys.maxsize",
    # Config key path. `APCORE_SYS_MODULES_RELOAD_ENABLED` resolves to
    # `sys.modules.reload.enabled` under the longest-prefix rule (9.10) —
    # a Config Bus dot-path, which shares the punctuation and nothing else.
    "sys.modules",
}

DEFAULT_PATHS = ("docs", "schemas", "conformance/fixtures", "examples",
                 "README.md", "CONTRIBUTING.md", "llms.txt")

SUFFIXES = {".md", ".json", ".yaml", ".yml", ".txt"}


def load_allowlist() -> list[dict]:
    if not ALLOWLIST.exists():
        return []
    return json.loads(ALLOWLIST.read_text()).get("allow", [])


def iter_files(paths: list[str]):
    for raw in paths:
        target = REPO / raw
        if target.is_file():
            yield target
        elif target.is_dir():
            for path in sorted(target.rglob("*")):
                if path.is_file() and path.suffix in SUFFIXES:
                    yield path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--paths", nargs="*", default=list(DEFAULT_PATHS))
    args = ap.parse_args()

    allow = load_allowlist()
    # (file, match) pairs an entry covers; used for both suppression and STALE.
    used: set[int] = set()
    findings: list[tuple[str, int, str, str]] = []

    for path in iter_files(args.paths):
        rel = path.relative_to(REPO).as_posix()
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue
        for lineno, line in enumerate(lines, 1):
            for match in PATTERN.finditer(line):
                token = match.group(0)
                if ".".join(token.split(".")[:2]) in NOT_A_MODULE_ID:
                    continue
                covered = False
                for idx, entry in enumerate(allow):
                    if entry.get("file") == rel and entry.get("match") == token:
                        used.add(idx)
                        covered = True
                        break
                if not covered:
                    findings.append((rel, lineno, token, line.strip()))

    for rel, lineno, token, text in findings:
        print(f"{rel}:{lineno}: `{token}` is not a module-ID namespace — the "
              f"control plane is `system.*` (PROTOCOL_SPEC 2.5, 6.7)",
              file=sys.stderr)
        print(f"    {text[:160]}", file=sys.stderr)

    stale = [entry for idx, entry in enumerate(allow) if idx not in used]
    if stale:
        print("\n  STALE allowlist entries — the occurrence is gone, remove them:",
              file=sys.stderr)
        for entry in stale:
            print(f"    {entry.get('file')}: {entry.get('match')}", file=sys.stderr)

    if findings:
        print(f"\n{len(findings)} occurrence(s) of a `sys.` module-ID namespace",
              file=sys.stderr)
        return 1
    print(f"no `sys.` module-ID namespace in {len(args.paths)} path(s) "
          f"({len(allow)} allowlisted)")
    return 1 if stale else 0


if __name__ == "__main__":
    sys.exit(main())
