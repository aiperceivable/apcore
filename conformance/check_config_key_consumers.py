#!/usr/bin/env python3
"""Does every declared configuration key reach a consumer?

`config_key_governance.json` pins that every declared key is **accepted** — the
three SDKs agree on the key set and on the default table. Nothing pinned that
any declared key is **read**, and 29 of 61 were not (apcore#118): registered in
three key surfaces, schema-declared, environment-overridable, documented with a
default, and inert.

This guard closes the gap the governance fixture structurally cannot see. That
fixture is *generated from* `schemas/`, so a key that is wrong in the schema and
in all three SDKs identically is invisible to it — the same blind spot the
pattern corpus had in apcore#116/#117, where every fixture value sat in the
region where the implementations coincide.

What it enforces
----------------
1. **Coverage.** Every key in `config_key_governance.json` has an entry in
   `config_key_consumers.json`, and every entry names a key that still exists.
   A key added to the surface without declaring what reads it fails here, which
   is the whole point: "declared but inert" must be a written decision rather
   than an accident.
2. **Probes pass.** An entry marked `live` names a probe below, and the probe is
   executed against apcore-python: set the key away from its default, observe
   something **outside `Config`** change.
3. **A ratchet.** `inert` and `unaudited` counts may fall and may not rise.

Why the probes are behavioural and not `grep`
---------------------------------------------
Static detection fails in both directions here, measured rather than assumed:

- **False negatives.** `apcore-python`'s `config.py` holds both the key registry
  and `Config.validate()`, so excluding the registry file hides real readers.
- **False positives, and worse.** Four `observability.*` keys are *plumbed*
  through `Config` in apcore-rust — `set()` writes a typed struct field and
  `get()` reads it back — and consumed by nothing. They **round-trip
  perfectly**, so the obvious set-then-get test passes while
  `sampling_rate: 0.1` yields 100% sampling. A guard that asserts "the key is
  readable" would pass on all four.

Hence: a probe must observe an effect *outside* `Config`.

Usage
-----
    python3 conformance/check_config_key_consumers.py --sdk-root ..
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Callable

HERE = Path(__file__).resolve().parent
GOVERNANCE = HERE / "fixtures" / "config_key_governance.json"
MANIFEST = HERE / "config_key_consumers.json"

VALID_STATUSES = {"live", "partial", "inert", "unaudited"}


# ---------------------------------------------------------------------------
# Probes — each returns None on success or a failure string.
#
# A probe MUST observe an effect outside `Config`. Reading the value back from
# `Config` proves plumbing, not consumption, and plumbing is exactly what the
# four observability keys have.
# ---------------------------------------------------------------------------

PROBES: dict[str, Callable[[], str | None]] = {}


def probe(key: str) -> Callable[[Callable[[], str | None]], Callable[[], str | None]]:
    def register(fn: Callable[[], str | None]) -> Callable[[], str | None]:
        PROBES[key] = fn
        return fn

    return register


@probe("sys_modules.enabled")
def _sys_modules_enabled() -> str | None:
    """The master switch registers, or does not register, the system.* modules."""
    from apcore.client import APCore
    from apcore.config import Config

    def system_ids(enabled: bool) -> set[str]:
        cfg = Config.from_defaults()
        cfg.set("sys_modules.enabled", enabled)
        return {m for m in APCore(config=cfg).registry.module_ids if m.startswith("system.")}

    on, off = system_ids(True), system_ids(False)
    if not on:
        return "sys_modules.enabled=True registered no system.* module"
    if off:
        return f"sys_modules.enabled=False still registered {sorted(off)}"
    return None


@probe("obs.redaction.sensitive_keys")
def _sensitive_keys() -> str | None:
    """A field named by the key is redacted; one that is not, is not."""
    from apcore.config import Config
    from apcore.observability.context_logger import RedactionConfig, _apply_redaction_config

    cfg = Config.from_defaults()
    cfg.set("obs.redaction.sensitive_keys", ["probe_field"])
    rc = RedactionConfig.from_config(cfg)
    out = _apply_redaction_config({"probe_field": "secret", "other_field": "kept"}, rc)
    if out.get("probe_field") == "secret":
        return "a field named by obs.redaction.sensitive_keys was not redacted"
    if out.get("other_field") != "kept":
        return "a field NOT named by obs.redaction.sensitive_keys was redacted"
    return None


@probe("obs.redaction.replacement")
def _replacement() -> str | None:
    """The substitution token is the configured one."""
    from apcore.config import Config
    from apcore.observability.context_logger import RedactionConfig, _apply_redaction_config

    cfg = Config.from_defaults()
    cfg.set("obs.redaction.sensitive_keys", ["probe_field"])
    cfg.set("obs.redaction.replacement", "<<PROBE>>")
    out = _apply_redaction_config({"probe_field": "secret"}, RedactionConfig.from_config(cfg))
    if out.get("probe_field") != "<<PROBE>>":
        return f"obs.redaction.replacement ignored: got {out.get('probe_field')!r}"
    return None


@probe("acl.default_effect")
def _default_effect() -> str | None:
    """With no rule matching, the decision follows the configured default."""
    from apcore.acl import ACL

    allow = ACL(rules=[], default_effect="allow")
    deny = ACL(rules=[], default_effect="deny")
    if not allow.check("api.a", "executor.b"):
        return "default_effect='allow' denied a call no rule matched"
    if deny.check("api.a", "executor.b"):
        return "default_effect='deny' allowed a call no rule matched"
    return None


# ---------------------------------------------------------------------------


def governance_keys() -> set[str]:
    doc = json.loads(GOVERNANCE.read_text(encoding="utf-8"))

    def walk(node: object) -> list[str]:
        found: list[str] = []
        if isinstance(node, dict):
            for k, v in node.items():
                if k in ("allowed_keys", "keys", "allowed") and isinstance(v, list):
                    found.extend(x for x in v if isinstance(x, str))
                else:
                    found.extend(walk(v))
        elif isinstance(node, list):
            for item in node:
                found.extend(walk(item))
        return found

    return set(walk(doc))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sdk-root", default="..", help="directory holding the sibling SDK checkouts")
    args = ap.parse_args()

    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    entries: dict[str, dict] = manifest["keys"]
    ratchet: dict[str, int] = manifest["ratchet"]
    declared = governance_keys()
    problems: list[str] = []

    # 1. Coverage, both directions.
    for key in sorted(declared - set(entries)):
        problems.append(
            f"{key}: declared in config_key_governance.json and absent from "
            f"config_key_consumers.json. A new configuration key must say what reads it, "
            f"or be recorded as inert with a reason — see apcore#118."
        )
    for key in sorted(set(entries) - declared):
        problems.append(f"{key}: listed here but no longer a declared configuration key")

    # 2. Shape, and probes.
    python_src = Path(args.sdk_root).resolve() / "apcore-python" / "src"
    have_python = python_src.is_dir()
    if have_python:
        sys.path.insert(0, str(python_src))

    for key, entry in sorted(entries.items()):
        status = entry.get("status")
        if status not in VALID_STATUSES:
            problems.append(f"{key}: status {status!r} is not one of {sorted(VALID_STATUSES)}")
            continue
        if status in ("inert", "partial") and not entry.get("reason"):
            problems.append(f"{key}: status {status!r} requires a `reason`")
        if status == "live":
            name = entry.get("probe")
            if not name:
                problems.append(f"{key}: status 'live' requires a `probe`")
                continue
            if name not in PROBES:
                problems.append(f"{key}: probe {name!r} is not defined in this file")
                continue
            if not have_python:
                continue
            try:
                failure = PROBES[name]()
            except Exception as exc:  # noqa: BLE001 — a probe that cannot run IS the finding
                failure = f"probe raised {type(exc).__name__}: {exc}"
            if failure:
                problems.append(f"{key}: probe {name!r} failed — {failure}")

    # 3. The ratchet.
    counts = {s: sum(1 for e in entries.values() if e.get("status") == s) for s in VALID_STATUSES}
    for status in ("inert", "unaudited"):
        ceiling = ratchet.get(status)
        if ceiling is None:
            problems.append(f"ratchet: no ceiling recorded for {status!r}")
        elif counts[status] > ceiling:
            problems.append(
                f"ratchet: {counts[status]} keys are {status!r}, above the recorded ceiling of "
                f"{ceiling}. This number may fall and may not rise — lower the ceiling in "
                f"config_key_consumers.json when you fix one."
            )

    for p in problems:
        print(f"  {p}", file=sys.stderr)
    summary = ", ".join(f"{counts[s]} {s}" for s in ("live", "partial", "inert", "unaudited"))
    print(f"{len(declared)} declared configuration keys — {summary}")
    if not have_python:
        print("  (apcore-python not found under --sdk-root; probes were not executed)")
    if problems:
        print(f"{len(problems)} problem(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
