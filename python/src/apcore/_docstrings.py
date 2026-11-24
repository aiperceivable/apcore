"""Docstring parser for extracting descriptions and parameter documentation.

Supports Google, NumPy, and Sphinx docstring styles with automatic detection.
No external dependencies -- uses regex parsing only.
"""

from __future__ import annotations

import inspect
import re
from typing import Any, Callable

_SECTION_HEADERS = re.compile(
    r"^\s*(Args|Arguments|Parameters|Returns|Raises|Yields"
    r"|Examples?|Notes?|References?|Attributes|See Also|Warnings?)\s*:",
    re.IGNORECASE,
)
_NUMPY_DIVIDER = re.compile(r"^\s*-{3,}\s*$")


def parse_docstring(
    func: Callable[..., Any],
) -> tuple[str | None, str | None, dict[str, str]]:
    """Parse a function's docstring to extract description and parameter docs.

    Returns:
        Tuple of (description, documentation, param_descriptions):
        - description: First-line summary.
        - documentation: Body text after summary, excluding param sections.
        - param_descriptions: Mapping of parameter names to descriptions.
    """
    doc = inspect.getdoc(func)
    if not doc:
        return (None, None, {})

    lines = doc.strip().split("\n")
    if not lines:
        return (None, None, {})

    description = lines[0].strip()
    style = _detect_style(doc)
    param_descriptions = _PARSERS[style](doc)
    documentation = _extract_documentation(lines)

    return (description, documentation, param_descriptions)


def _detect_style(doc: str) -> str:
    """Auto-detect docstring style: 'google', 'numpy', or 'sphinx'."""
    if re.search(r"^\s*:param\s+(?:\w+\s+)?\w+:", doc, re.MULTILINE):
        return "sphinx"
    if re.search(r"^\s*Parameters\s*\n\s*-{3,}", doc, re.MULTILINE):
        return "numpy"
    if re.search(r"^\s*(Args|Arguments|Parameters)\s*:", doc, re.MULTILINE):
        return "google"
    return "google"


def _parse_google(doc: str) -> dict[str, str]:
    """Parse Google-style docstring parameters."""
    match = re.search(
        r"^\s*(Args|Arguments|Parameters)\s*:\s*\n" r"(.*?)(?=\n\s*\n|\n\s*[A-Z]\w*\s*:|\Z)",
        doc,
        re.MULTILINE | re.DOTALL,
    )
    if not match:
        return {}

    block = match.group(2)
    params: dict[str, str] = {}
    for m in re.finditer(
        r"^\s+(\w+)\s*(?:\([^)]*\))?\s*:\s*(.+?)(?=\n\s+\w+\s*(?:\([^)]*\))?\s*:|\Z)",
        block,
        re.MULTILINE | re.DOTALL,
    ):
        params[m.group(1)] = _clean_multiline(m.group(2))
    return params


def _parse_numpy(doc: str) -> dict[str, str]:
    """Parse NumPy-style docstring parameters."""
    match = re.search(
        r"^\s*Parameters\s*\n\s*-{3,}\s*\n" r"(.*?)(?=\n\s*[A-Z]\w*\s*\n\s*-{3,}|\Z)",
        doc,
        re.MULTILINE | re.DOTALL,
    )
    if not match:
        return {}

    params: dict[str, str] = {}
    for m in re.finditer(
        r"^\s*(\w+)\s*:\s*[^\n]*\n((?:\s{4,}.+\n?)*)",
        match.group(1),
        re.MULTILINE,
    ):
        params[m.group(1)] = _clean_multiline(m.group(2))
    return params


def _parse_sphinx(doc: str) -> dict[str, str]:
    """Parse Sphinx-style docstring parameters."""
    params: dict[str, str] = {}
    for m in re.finditer(
        r":param\s+(?:\w+\s+)?(\w+)\s*:\s*(.+?)(?=\n\s*:|$)",
        doc,
        re.MULTILINE | re.DOTALL,
    ):
        params[m.group(1)] = _clean_multiline(m.group(2))
    return params


def _clean_multiline(text: str) -> str:
    """Clean multiline description text into a single line."""
    return " ".join(line.strip() for line in text.strip().split("\n") if line.strip())


def _extract_documentation(lines: list[str]) -> str | None:
    """Extract body text after the summary, stopping at known section headers."""
    if len(lines) <= 1:
        return None

    body_lines: list[str] = []
    started = False

    for line in lines[1:]:
        if not started and not line.strip():
            continue
        started = True
        if _SECTION_HEADERS.match(line):
            break
        if _NUMPY_DIVIDER.match(line) and body_lines and body_lines[-1].strip():
            body_lines.pop()
            break
        body_lines.append(line)

    text = "\n".join(body_lines).strip()
    return text if text else None


_PARSERS = {
    "google": _parse_google,
    "numpy": _parse_numpy,
    "sphinx": _parse_sphinx,
}
