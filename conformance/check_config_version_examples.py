#!/usr/bin/env python3
"""The config `version:` in documented examples is the FORMAT version, not an SDK release.

`apcore-config.schema.json` describes the top-level `version` as "Configuration
version" and offers `1.0.0` as its example. Documented examples kept filling it
with whatever the SDK's release number happened to be at the time the page was
written: PROTOCOL_SPEC §9.6 carried `0.14.0` across seven blocks, `config-bus.md`
`0.15.0`, and apcore-typescript's README `0.26.0` — three different values for
one field, none of them a configuration version, and every one of them stale the
moment the next release shipped.

That is not cosmetic. A reader copies the block, and the number they copy looks
like something they are supposed to keep in step with their dependency. It also
seeded `apcore-rust`'s default table, which carried `version: "0.16.0"` as "the
frozen baseline spec version" while its two peers supplied no default at all.

    python3 conformance/check_config_version_examples.py [--sdk-root DIR]

Exit 1 on any example whose config `version:` is not one the schema sanctions.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SCHEMA = REPO / "schemas" / "apcore-config.schema.json"

#: Files scanned in this repository. SDK READMEs are added via --sdk-root.
DOC_GLOBS = ["docs/**/*.md", "README.md"]

#: `version: "X"` at the start of a line, with optional YAML indent. Restricted
#: to the two-space indent of a top-level `apcore:` child, or no indent at all
#: (legacy mode), so `project:` / `implementation:` blocks — which legitimately
#: carry an SDK or project version — are out of scope.
VERSION_RE = re.compile(r'^(?P<indent> {0,2})version: "(?P<value>[^"]+)"', re.MULTILINE)

#: Blocks whose nested `version:` is NOT the configuration version.
FOREIGN_PARENTS = ("project:", "implementation:", "package:", "sdk:")

#: Keys that mark a YAML block as an ACL policy file rather than an apcore
#: config. `acl-config.schema.json` owns its own `version`, and it accepts the
#: two-part `1.0` form every documented ACL example uses.
ACL_MARKERS = ("default_effect:", "rules:")

#: Fenced code blocks, with their language tag.
FENCE_RE = re.compile(r"^```(?P<lang>[a-zA-Z0-9_-]*)\n(?P<body>.*?)^```", re.MULTILINE | re.DOTALL)


def sanctioned_versions() -> set[str]:
    schema = json.loads(SCHEMA.read_text())
    node = schema.get("properties", {}).get("version", {})
    examples = set(node.get("examples") or [])
    if not examples:
        raise SystemExit(
            "apcore-config.schema.json declares no `examples` for `version` — "
            "this guard reads them as the sanctioned set, so it cannot run."
        )
    return examples


def parent_block(text: str, start: int, indent: int) -> str | None:
    """The nearest preceding line at a shallower indent, or None at top level."""
    for line in reversed(text[:start].splitlines()):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        line_indent = len(line) - len(line.lstrip())
        if line_indent < indent:
            return line.strip()
    return None


def scan(path: Path, allowed: set[str]) -> list[str]:
    text = path.read_text(encoding="utf-8")
    problems: list[str] = []
    for fence in FENCE_RE.finditer(text):
        if fence.group("lang") not in ("yaml", "yml", ""):
            continue
        body = fence.group("body")
        if any(marker in body for marker in ACL_MARKERS):
            continue  # an ACL policy file, governed by acl-config.schema.json
        for match in VERSION_RE.finditer(body):
            value = match.group("value")
            if value in allowed:
                continue
            indent = len(match.group("indent"))
            parent = parent_block(body, match.start(), indent) if indent else None
            if parent and parent.startswith(FOREIGN_PARENTS):
                continue
            line_no = text[: fence.start("body") + match.start()].count("\n") + 1
            problems.append(
                f"{path.relative_to(REPO.parent)}:{line_no}: config `version: \"{value}\"` "
                f"is not a configuration version — the schema sanctions {sorted(allowed)}"
            )
    return problems


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sdk-root", type=Path, default=REPO.parent)
    args = parser.parse_args()

    allowed = sanctioned_versions()
    files: list[Path] = []
    for pattern in DOC_GLOBS:
        files.extend(sorted(REPO.glob(pattern)))
    for sdk in ("apcore-python", "apcore-typescript", "apcore-rust"):
        readme = args.sdk_root / sdk / "README.md"
        if readme.is_file():
            files.append(readme)

    problems = [p for path in files for p in scan(path, allowed)]
    for problem in problems:
        print(problem, file=sys.stderr)
    print(f"{len(files)} file(s) scanned; {len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
