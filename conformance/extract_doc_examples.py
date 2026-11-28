"""Extract the executable documentation examples for an installed SDK."""

from __future__ import annotations

import argparse
import re
from pathlib import Path
from textwrap import dedent

REPO = Path(__file__).resolve().parent.parent
EXTENSIONS = {"python": "py", "typescript": "ts", "rust": "rs"}
STREAM_GUIDE = "docs/features/streaming.md"


def example(source: str, label: str, language: str) -> str:
    markdown = (REPO / source).read_text(encoding="utf-8")
    pattern = (
        rf"^(?P<indent>[ \t]*)<!-- executable-example: {re.escape(label)} -->[ \t]*\n"
        rf"(?:[ \t]*\n)*^(?P=indent)```{language}[ \t]*\n"
        r"(?P<code>.*?)^(?P=indent)```[ \t]*$"
    )
    matches = list(re.finditer(pattern, markdown, flags=re.MULTILINE | re.DOTALL))
    if len(matches) != 1 or not matches[0]["code"].strip():
        raise ValueError(f"{source}: expected one nonempty {language} block marked {label!r}")
    return dedent(matches[0]["code"])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language", choices=EXTENSIONS)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    ext = EXTENSIONS[args.language]
    sources = (
        ("docs/getting-started.md", "quick-start", args.language, f"quick-start/example.{ext}"),
        (STREAM_GUIDE, "governed-stream", args.language, f"governed-stream/example.{ext}"),
        (STREAM_GUIDE, "stream-config", "yaml", "governed-stream/apcore.yaml"),
        (STREAM_GUIDE, "stream-acl", "yaml", "governed-stream/acl/global_acl.yaml"),
        ("docs/features/module-interface.md", "typed-module", args.language, f"typed-module/example.{ext}"),
        ("docs/features/context-object.md", "invocation-budget", args.language, f"invocation-budget/example.{ext}"),
    )
    try:
        files = [(args.destination / target, example(source, label, lang)) for source, label, lang, target in sources]
    except ValueError as exc:
        parser.error(str(exc))
    for path, code in files:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(code, encoding="utf-8")


if __name__ == "__main__":
    main()
