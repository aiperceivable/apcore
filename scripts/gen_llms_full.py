#!/usr/bin/env python3
"""Generate docs/llms-full.txt: the full apcore documentation concatenated for
LLM ingestion, in mkdocs.yml nav order.

Run from the repo root (CI calls it in the "Prepare docs for MkDocs" step, but it
also works standalone for local preview):

    python scripts/gen_llms_full.py

The concise index lives at the repo-root llms.txt (hand-curated). This file is the
full-content companion and is GENERATED — never hand-edit it.
"""

from __future__ import annotations

import pathlib
import sys

import yaml

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
# Canonical public site (matches llms.txt), not the mkdocs.yml github.io site_url.
CANONICAL = "https://apcore.aiperceivable.com"


def page_url(rel: str) -> str:
    """Map a docs-relative .md path to its MkDocs directory-style URL."""
    p = rel[:-3] if rel.endswith(".md") else rel
    if p.endswith("index"):
        p = p[: -len("index")]
    p = p.strip("/")
    return f"{CANONICAL}/{p}/" if p else f"{CANONICAL}/"


def walk(nav: list, out: list[tuple[str | None, str]]) -> None:
    """Flatten the nested mkdocs nav into ordered (title, docpath) pairs."""
    for item in nav:
        if isinstance(item, str):
            out.append((None, item))
        elif isinstance(item, dict):
            for title, val in item.items():
                if isinstance(val, str):
                    out.append((title, val))
                elif isinstance(val, list):
                    walk(val, out)


def strip_frontmatter(text: str) -> str:
    """Remove a leading YAML frontmatter block if present."""
    if text.startswith("---"):
        end = text.find("\n---", 3)
        if end != -1:
            nl = text.find("\n", end + 1)
            return text[nl + 1 :] if nl != -1 else ""
    return text


def main() -> int:
    mkdocs = yaml.safe_load((ROOT / "mkdocs.yml").read_text(encoding="utf-8"))
    pages: list[tuple[str | None, str]] = []
    walk(mkdocs.get("nav", []), pages)

    site = mkdocs.get("site_name", "apcore")
    desc = mkdocs.get("site_description", "")
    parts = [
        f"# {site}\n\n> {desc}\n\n"
        "This file concatenates the full apcore documentation for LLM ingestion, "
        "in nav order. It is generated from mkdocs.yml by scripts/gen_llms_full.py "
        "— do not edit by hand. The concise index is at /llms.txt.\n"
    ]

    seen: set[str] = set()
    missing = 0
    for title, rel in pages:
        if rel in seen:
            continue
        seen.add(rel)
        fp = DOCS / rel
        if not fp.exists():
            print(f"warning: nav references missing file {rel}", file=sys.stderr)
            missing += 1
            continue
        body = strip_frontmatter(fp.read_text(encoding="utf-8")).strip()
        heading = title or fp.stem
        parts.append(
            f"\n\n---\n\n# {heading}\n\nSource: {page_url(rel)}\n\n{body}\n"
        )

    out = DOCS / "llms-full.txt"
    out.write_text("".join(parts), encoding="utf-8")
    print(
        f"wrote {out.relative_to(ROOT)} "
        f"({len(seen) - missing} pages, {out.stat().st_size} bytes)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
