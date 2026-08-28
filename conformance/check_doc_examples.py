#!/usr/bin/env python3
"""Verify that every symbol the docs import actually exists in the SDK.

Documentation examples ARE the onboarding API: a user copying a block that
imports a symbol which does not exist gets an error before they have written a
line of their own code. This checker exists because ~120 such defects shipped
undetected, and the reason they went unnoticed is structural — the same
nonexistent API was written into all three language tabs in lockstep, so
cross-language review saw three mutually consistent examples and moved on. Only
a machine that resolves symbols against the real SDK can catch that.

Scope, deliberately narrow so the result is trustworthy:

  * Python      `from apcore[.x] import A, B`  → module exists, symbol defined
  * TypeScript  `from 'apcore-js'`             → symbol re-exported from src/index.ts
                `from 'apcore-js/sub'`         → always an error; the package
                                                 exports only "." and "./context-keys"
                `from 'apcore'`                → always an error; the package is apcore-js
  * Rust        `use apcore::{A, B}`           → symbol re-exported from src/lib.rs
                `ErrorCode::X`                 → X is a real enum variant

It does NOT typecheck, compile, or check arity — those need the real toolchains
and are the natural next layer. Every check here is one a wrong answer cannot
survive: the symbol resolves or it does not.

    python3 conformance/check_doc_examples.py [--sdk-root DIR]

Exit 1 on any unresolved symbol. Exits 0 with a notice when the sibling SDK
checkouts are absent, so the doc repo stays cloneable on its own.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# ---------------------------------------------------------------- extraction

FENCE = re.compile(r"```(python|typescript|ts|rust)\n(.*?)```", re.S)


def code_blocks(root: Path) -> list[tuple[Path, int, str, str]]:
    """Yield (file, line_of_fence, language, body) for every fenced block."""
    out: list[tuple[Path, int, str, str]] = []
    targets = sorted(root.glob("docs/**/*.md")) + [root / "README.md"]
    for path in targets:
        if not path.is_file():
            continue
        text = path.read_text()
        for m in FENCE.finditer(text):
            lang = {"ts": "typescript"}.get(m.group(1), m.group(1))
            line = text[: m.start()].count("\n") + 1
            out.append((path, line, lang, m.group(2)))
    return out


# ------------------------------------------------------------- SDK surfaces


def python_surface(sdk: Path) -> dict[str, set[str]]:
    """{module_dotpath: {top-level names}} for the apcore package."""
    pkg = sdk / "src" / "apcore"
    surface: dict[str, set[str]] = {}
    if not pkg.is_dir():
        return surface
    for py in pkg.rglob("*.py"):
        rel = py.relative_to(pkg)
        parts = list(rel.parts)
        if parts[-1] == "__init__.py":
            parts = parts[:-1]
        else:
            parts[-1] = parts[-1][:-3]
        dotted = ".".join(["apcore", *parts])
        src = py.read_text()
        names = set(re.findall(r"^(?:class|def|async def)\s+(\w+)", src, re.M))
        names |= set(re.findall(r"^(\w+)(?:\s*:[^=\n]+)?\s*=", src, re.M))
        # Re-exports: `from x import A, B` and __all__ entries.
        for blk in re.findall(r"^from\s+[\w.]+\s+import\s+\(([^)]*)\)", src, re.M):
            names |= {n.strip().split(" as ")[-1].strip() for n in blk.split(",") if n.strip()}
        for blk in re.findall(r"^from\s+[\w.]+\s+import\s+([^\n(]+)$", src, re.M):
            names |= {n.strip().split(" as ")[-1].strip() for n in blk.split(",") if n.strip()}
        all_m = re.search(r"__all__\s*=\s*\[(.*?)\]", src, re.S)
        if all_m:
            names |= set(re.findall(r'"(\w+)"', all_m.group(1)))
        surface[dotted] = names
    return surface


def typescript_surface(sdk: Path) -> set[str]:
    """Names re-exported from src/index.ts."""
    index = sdk / "src" / "index.ts"
    if not index.is_file():
        return set()
    src = index.read_text()
    names: set[str] = set()
    for blk in re.findall(r"export\s+(?:type\s+)?\{([^}]*)\}\s*from", src, re.S):
        for item in blk.split(","):
            item = item.strip()
            if not item:
                continue
            m = re.match(r"(?:type\s+)?[\w$]+\s+as\s+([\w$]+)$", item) or re.match(
                r"(?:type\s+)?([\w$]+)$", item
            )
            if m:
                names.add(m.group(1))
    for star in re.findall(r"export\s+\*\s+from\s+['\"]\./([^'\"]+)['\"]", src):
        p = sdk / "src" / star.replace(".js", ".ts")
        if p.is_file():
            names |= set(
                re.findall(
                    r"export\s+(?:declare\s+)?(?:abstract\s+)?"
                    r"(?:class|function|const|let|var|interface|type|enum)\s+([\w$]+)",
                    p.read_text(),
                )
            )
    return names


def rust_modules(sdk: Path) -> dict[str, set[str] | None]:
    """Public symbol map for every module under `src/`.

    `src/foo.rs` and `src/foo/mod.rs` both map to `foo`; `src/foo/bar.rs` to
    `foo::bar`. A module that glob-re-exports (`pub use x::*`) maps to `None`,
    meaning "exists, contents not enumerable" — its symbols are not checked,
    because a glob can legitimately supply anything and guessing would produce
    false positives in the one direction this checker must never produce them.
    """
    src_root = sdk / "src"
    mods: dict[str, set[str] | None] = {}
    if not src_root.is_dir():
        return mods
    for f in sorted(src_root.rglob("*.rs")):
        rel = f.relative_to(src_root)
        if rel.name == "lib.rs":
            continue
        parts = list(rel.parts[:-1]) + ([] if rel.name == "mod.rs" else [rel.stem])
        if not parts:
            continue
        key = "::".join(parts)
        text = f.read_text()
        if re.search(r"^\s*pub use [\w:]+::\*\s*;", text, re.M):
            mods[key] = None
            continue
        names: set[str] = set(mods.get(key) or set())
        names |= set(re.findall(r"pub (?:struct|enum|trait|type|const|static|fn|mod) (\w+)", text))
        names |= set(re.findall(r"pub async fn (\w+)", text))
        for blk in re.findall(r"pub use [\w:]+::\{([^}]*)\};", text, re.S):
            for item in blk.split(","):
                item = item.strip().split(" as ")[-1].strip()
                if re.fullmatch(r"[A-Za-z_]\w*", item or ""):
                    names.add(item)
        names |= set(re.findall(r"pub use [\w:]+::(\w+)\s*;", text))
        mods[key] = names
    return mods


def rust_surface(sdk: Path) -> tuple[set[str], set[str]]:
    """(crate-root re-exports, ErrorCode variants)."""
    lib = sdk / "src" / "lib.rs"
    exports: set[str] = set()
    if lib.is_file():
        src = lib.read_text()
        for blk in re.findall(r"pub use [\w:]+::\{([^}]*)\};", src, re.S):
            for item in blk.split(","):
                item = item.strip().split(" as ")[-1].strip()
                if item and re.fullmatch(r"[A-Za-z_]\w*", item):
                    exports.add(item)
        for item in re.findall(r"pub use [\w:]+::(\w+);", src):
            exports.add(item)
        exports |= set(re.findall(r"pub mod (\w+);", src))
        exports |= set(re.findall(r"pub const (\w+)", src))
    variants: set[str] = set()
    errs = sdk / "src" / "errors.rs"
    if errs.is_file():
        m = re.search(r"pub enum ErrorCode\s*\{(.*?)\n\}", errs.read_text(), re.S)
        if m:
            variants = set(re.findall(r"^\s*([A-Z]\w*)\s*,", m.group(1), re.M))
    return exports, variants


# ----------------------------------------------------------------- checkers

Violation = tuple[str, int, str]  # (file:line, line_in_block, message)

ALLOWLIST = REPO / "conformance" / "doc_examples_allowlist.json"


def load_allowlist() -> list[dict]:
    if not ALLOWLIST.is_file():
        return []
    return json.loads(ALLOWLIST.read_text())["allow"]


def allowed(entry: dict, loc: str, msg: str) -> bool:
    """An entry excuses a violation only in the file it names, for the symbol it
    names. A blanket symbol exemption would silently cover a real defect
    elsewhere in the docs."""
    return entry["file"] in loc and f"`{entry['symbol']}`" in msg


def check_python(blocks, surface, out: list[Violation]) -> None:
    if not surface:
        return
    for path, fence_line, lang, body in blocks:
        if lang != "python":
            continue
        for m in re.finditer(r"^\s*from\s+(apcore(?:\.[\w.]+)?)\s+import\s+([^\n#]+)", body, re.M):
            mod, names = m.group(1), m.group(2)
            line = fence_line + body[: m.start()].count("\n") + 1
            loc = f"{path.relative_to(REPO)}:{line}"
            if mod not in surface:
                out.append((loc, line, f"no such module `{mod}`"))
                continue
            for name in (n.strip().split(" as ")[0].strip()
                         for n in names.replace("(", "").replace(")", "").split(",")):
                if name and name not in surface[mod]:
                    out.append((loc, line, f"`{mod}` does not export `{name}`"))


def check_typescript(blocks, exports: set[str], out: list[Violation]) -> None:
    if not exports:
        return
    for path, fence_line, lang, body in blocks:
        if lang != "typescript":
            continue
        for m in re.finditer(r"""from\s+['"](apcore(?:-js)?(?:/[\w./-]+)?)['"]""", body):
            spec = m.group(1)
            line = fence_line + body[: m.start()].count("\n") + 1
            loc = f"{path.relative_to(REPO)}:{line}"
            if spec == "apcore" or spec.startswith("apcore/"):
                out.append((loc, line, f"package is `apcore-js`, not `{spec}`"))
            elif spec.startswith("apcore-js/") and spec != "apcore-js/context-keys":
                out.append((loc, line, f"`{spec}` is not an exported subpath "
                                       "(package.json exports only '.' and './context-keys')"))
        for m in re.finditer(
            r"""import\s+(?:type\s+)?\{([^}]*)\}\s*from\s*['"]apcore-js['"]""", body, re.S
        ):
            line = fence_line + body[: m.start()].count("\n") + 1
            loc = f"{path.relative_to(REPO)}:{line}"
            for item in m.group(1).split(","):
                name = item.strip().split(" as ")[0].strip()
                # Skip inline-comment noise inside multi-line import braces.
                if not name or name.startswith("//") or " " in name:
                    continue
                if name not in exports:
                    out.append((loc, line, f"`apcore-js` does not export `{name}`"))


def check_rust_nested(blocks, mods: dict, out: list[Violation]) -> None:
    """Resolve `use apcore::a::b::{X}` and `use apcore::a::B;` against the SDK's
    module tree.

    `check_rust` below only sees the crate-root brace form `use apcore::{...}`.
    In the current docs that is half the apcore imports in Rust examples; the
    other half are nested and were never examined, which is how
    `apcore::errors::PipelineStepError` — a type that does not exist — survived
    a green CI (#105).
    """
    if not mods:
        return
    brace = re.compile(r"use\s+apcore::([\w:]+)::\{([^}]*)\}\s*;", re.S)
    single = re.compile(r"use\s+apcore::([\w:]+)::(\w+)\s*;")
    for path, fence_line, lang, body in blocks:
        if lang != "rust":
            continue
        seen: set[tuple[int, str]] = set()
        for m in list(brace.finditer(body)) + list(single.finditer(body)):
            modpath = m.group(1)
            names = ([n.strip().split(" as ")[-1].strip() for n in m.group(2).split(",")]
                     if m.re is brace else [m.group(2)])
            line = fence_line + body[: m.start()].count("\n") + 1
            loc = f"{path.relative_to(REPO)}:{line}"
            if modpath not in mods:
                # A trailing segment may be the item itself: `use apcore::acl::ACL;`
                # is caught by `single` with modpath="acl"; but `use apcore::a::b::C`
                # where `a::b` is unknown is a genuine miss.
                key = (line, modpath)
                if key not in seen:
                    seen.add(key)
                    out.append((loc, line, f"`apcore` has no module `{modpath}`"))
                continue
            known = mods[modpath]
            if known is None:
                continue
            for name in names:
                if not name or name == "self" or not re.fullmatch(r"[A-Za-z_]\w*", name):
                    continue
                if name not in known:
                    out.append((loc, line, f"`apcore::{modpath}` does not export `{name}`"))


def check_rust(blocks, exports: set[str], variants: set[str], out: list[Violation]) -> None:
    if not variants:
        return
    for path, fence_line, lang, body in blocks:
        if lang != "rust":
            continue
        for m in re.finditer(r"ErrorCode::(\w+)", body):
            name = m.group(1)
            if name not in variants:
                line = fence_line + body[: m.start()].count("\n") + 1
                out.append((f"{path.relative_to(REPO)}:{line}", line,
                            f"`ErrorCode` has no variant `{name}`"))
        if not exports:
            continue
        for m in re.finditer(r"use\s+apcore::\{([^}]*)\}\s*;", body, re.S):
            line = fence_line + body[: m.start()].count("\n") + 1
            loc = f"{path.relative_to(REPO)}:{line}"
            for item in m.group(1).split(","):
                name = item.strip().split(" as ")[-1].strip()
                if not name or not re.fullmatch(r"[A-Za-z_]\w*", name):
                    continue
                if name not in exports:
                    out.append((loc, line, f"crate root does not re-export `{name}`"))


# --------------------------------------------------------------------- main


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sdk-root", type=Path, default=REPO.parent,
                    help="directory holding apcore-python / apcore-typescript / apcore-rust")
    args = ap.parse_args()

    py_sdk = args.sdk_root / "apcore-python"
    ts_sdk = args.sdk_root / "apcore-typescript"
    rs_sdk = args.sdk_root / "apcore-rust"
    present = [p.name for p in (py_sdk, ts_sdk, rs_sdk) if p.is_dir()]
    if not present:
        print(f"No SDK checkouts under {args.sdk_root} — skipping. "
              "Clone apcore-python / apcore-typescript / apcore-rust as siblings, "
              "or pass --sdk-root.")
        return 0

    blocks = code_blocks(REPO)
    out: list[Violation] = []
    check_python(blocks, python_surface(py_sdk), out)
    check_typescript(blocks, typescript_surface(ts_sdk), out)
    exports, variants = rust_surface(rs_sdk)
    check_rust(blocks, exports, variants, out)
    check_rust_nested(blocks, rust_modules(rs_sdk), out)

    langs = {b[2] for b in blocks}
    print(f"checked {len(blocks)} code blocks ({', '.join(sorted(langs))}) "
          f"against {', '.join(present)}")

    allow = load_allowlist()
    unique = sorted(set(out))
    excused, real = [], []
    for loc, line, msg in unique:
        hit = next((e for e in allow if allowed(e, loc, msg)), None)
        (excused if hit else real).append((loc, line, msg, hit))

    # An allowlist entry that no longer excuses anything has served its purpose:
    # say so, so the exemption does not outlive the reason for it.
    stale = [e for e in allow if not any(h is e for *_, h in excused)]

    for loc, _, msg, hit in excused:
        print(f"  allowed  {loc}: {msg}\n           ({hit['removable_when']})")
    for e in stale:
        print(f"  STALE    allowlist entry `{e['symbol']}` no longer excuses anything — "
              f"remove it ({e['removable_when']})", file=sys.stderr)

    if real:
        print(f"\n{len(real)} unresolved symbol(s):", file=sys.stderr)
        for loc, _, msg, _ in real:
            print(f"  {loc}: {msg}", file=sys.stderr)
        print("\nDocumentation examples are the onboarding API — a reader copying one "
              "of these gets an error before writing any code of their own.", file=sys.stderr)
        return 1
    if stale:
        return 1

    print("every imported symbol resolves against the real SDK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
