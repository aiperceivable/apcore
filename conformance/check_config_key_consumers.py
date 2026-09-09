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


def _register_deprecation_probe() -> None:
    """Registered by name so a manifest entry can point at it."""
    PROBES["deprecated_inert_keys"] = deprecation_probe


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


def _system_ids(**flags: bool) -> set[str]:
    """Register the sys-module section under `flags` and report the system.* IDs."""
    from apcore.client import APCore
    from apcore.config import Config

    cfg = Config.from_defaults()
    cfg.set("sys_modules.enabled", True)
    cfg.set("sys_modules.events.enabled", True)
    for group, value in flags.items():
        cfg.set(f"sys_modules.{group}.enabled", value)
    return {m for m in APCore(config=cfg).registry.module_ids if m.startswith("system.")}


def _group_probe(group: str, expected: set[str]) -> str | None:
    """A per-group flag must remove exactly its own modules and nothing else."""
    on = _system_ids()
    off = _system_ids(**{group: False})
    if not expected <= on:
        return f"with every flag on, {sorted(expected - on)} did not register"
    removed = on - off
    if removed != expected:
        return (
            f"sys_modules.{group}.enabled=False removed {sorted(removed)}, expected "
            f"{sorted(expected)} — the flag selects the wrong module set, or is ignored"
        )
    return None


@probe("sys_modules.health.enabled")
def _health_enabled() -> str | None:
    return _group_probe("health", {"system.health.summary", "system.health.module"})


@probe("sys_modules.manifest.enabled")
def _manifest_enabled() -> str | None:
    return _group_probe("manifest", {"system.manifest.module", "system.manifest.full"})


@probe("sys_modules.usage.enabled")
def _usage_enabled() -> str | None:
    return _group_probe("usage", {"system.usage.summary", "system.usage.module"})


@probe("sys_modules.control.enabled")
def _control_enabled() -> str | None:
    """The Level 2 write plane (6.7). The one an operator turns off deliberately."""
    return _group_probe(
        "control",
        {
            "system.control.update_config",
            "system.control.reload_module",
            "system.control.toggle_feature",
        },
    )


#: PROTOCOL_SPEC 9.2.4 — the ten keys whose removal window is open.
DEPRECATED_INERT_KEYS = (
    "observability.tracing.enabled",
    "observability.tracing.sampling_rate",
    "observability.tracing.exporter",
    "observability.metrics.enabled",
    "observability.metrics.exporter",
    "logging.level",
    "logging.format",
    "acl.audit.enabled",
    "acl.audit.include_denied",
    "acl.audit.log_level",
)


def deprecation_probe() -> str | None:
    """9.2.4: a declared inert key warns, and a clean configuration does not.

    Both halves are the requirement. A probe that only checked "it warns" would
    pass an implementation that warns for EVERY configuration — the blanket
    warning 9.2.2 rejects, and the failure mode requirement 2 exists to prevent,
    since every one of these keys has a default and a merged-view check fires
    unconditionally. apcore-rust reached exactly that state on the first attempt.
    """
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config

    base = {"version": "1.0.0", "project": {"name": "probe", "version": "0.1.0"}}

    def load(doc: dict) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "apcore.yaml"
            path.write_text(yaml.safe_dump(doc))
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always")
                Config.load(str(path))
            return [str(w.message) for w in caught if "9.2.4" in str(w.message)]

    if load(base):
        return "a configuration declaring none of the deprecated keys still warned"

    for key in DEPRECATED_INERT_KEYS:
        doc = dict(base)
        node = doc
        parts = key.split(".")
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        node[parts[-1]] = "probe" if key.endswith(("level", "format", "exporter")) else 1
        hits = load(doc)
        if not hits:
            return f"{key} was declared and produced no deprecation warning"
        if key not in hits[0]:
            return f"{key} warned but the message does not name it: {hits[0][:120]}"
    return None


@probe("stream.max_merge_depth")
def _max_merge_depth() -> str | None:
    """The configured cap must govern the merge, and must not be removable."""
    from apcore.config import Config
    from apcore.executor import _deep_merge, _resolve_merge_depth

    cfg = Config.from_defaults()
    if _resolve_merge_depth(cfg) != 32:
        return f"the canonical default is 32, got {_resolve_merge_depth(cfg)}"
    cfg.set("stream.max_merge_depth", 4)
    if _resolve_merge_depth(cfg) != 4:
        return "a configured stream.max_merge_depth did not reach the resolver"

    # The cap is observable: below it the merge recurses and base-only keys at
    # that depth survive; at it the merge replaces and they do not.
    def nest(depth: int, sibling: str) -> dict:
        node: dict = {"leaf": sibling, sibling: 1}
        for _ in range(depth):
            node = {"k": node}
        return node

    base = nest(6, "only_base")
    _deep_merge(base, nest(6, "only_override"), max_depth=4)
    node = base
    for _ in range(5):
        node = node["k"]
    if "only_base" in node:
        return "a cap of 4 did not apply at depth 5 — the merge recursed past it"

    # A misconfiguration MUST NOT remove the cap.
    for bad in (0, -1, "x", None):
        cfg.set("stream.max_merge_depth", bad)
        if _resolve_merge_depth(cfg) != 32:
            return f"stream.max_merge_depth={bad!r} did not fall back to the canonical 32"
    return None


def _binding_probe(key: str, value: object, entry_patch: dict) -> str | None:
    """A validation.binding.* limit must be off by default and enforced when set.

    Both halves matter and the first is the decision: apcore does not impose
    limits on the content its users author (PROTOCOL_SPEC 9.1.2), so a probe
    that only checked "the limit rejects" would pass an implementation that
    rejects by default too — which is the regression this guards.
    """
    import tempfile
    from pathlib import Path

    import yaml

    from apcore.bindings import BindingLoader
    from apcore.config import Config
    from apcore.errors import BindingFileInvalidError
    from apcore.registry import Registry

    entry = {
        "module_id": "probe.module",
        "target": "probe_targets:noop",
        "input_schema": {"type": "object", "properties": {}},
        "output_schema": {"type": "object", "properties": {}},
        **entry_patch,
    }
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "probe.binding.yaml"
        path.write_text(yaml.safe_dump({"spec_version": "1.0", "bindings": [entry]}))

        def load(configured: bool) -> str | None:
            cfg = Config.from_defaults()
            if configured:
                cfg.set(key, value)
            try:
                BindingLoader().load_bindings(str(path), Registry(), cfg)
            except BindingFileInvalidError as exc:
                return str(exc)
            except Exception:  # noqa: BLE001 — target resolution is not what we assert
                return None
            return None

        if load(configured=False) is not None:
            return f"{key} rejected with NO limit configured — the default must be unconstrained"
        rejected = load(configured=True)
        if rejected is None:
            return f"{key}={value!r} did not reject an entry that violates it"
        if key.rsplit(".", 1)[-1] not in rejected:
            return f"{key} rejected but the message does not name the key: {rejected}"
    return None


@probe("validation.binding.description_max_length")
def _description_max_length() -> str | None:
    return _binding_probe(
        "validation.binding.description_max_length", 200, {"description": "D" * 300}
    )


@probe("validation.binding.documentation_max_length")
def _documentation_max_length() -> str | None:
    return _binding_probe(
        "validation.binding.documentation_max_length", 5000, {"documentation": "M" * 6000}
    )


@probe("validation.binding.tags_pattern")
def _tags_pattern() -> str | None:
    return _binding_probe(
        "validation.binding.tags_pattern", "^[a-z][a-z0-9_]*$", {"tags": ["Email"]}
    )


@probe("validation.binding.version_require_semver")
def _version_require_semver() -> str | None:
    return _binding_probe(
        "validation.binding.version_require_semver", True, {"version": "1.0"}
    )


def _pipeline_probe(key: str, value: object, step_patch: dict) -> str | None:
    """A validation.pipeline.* limit must be off by default and enforced when set."""
    from apcore.config import Config
    from apcore.pipeline_config import ConfigurationError, _validate_pipeline_limits

    section = {
        "steps": [{"name": "probe_step", "type": "probe_noop", "after": "execute", **step_patch}]
    }

    def run(configured: bool) -> str | None:
        cfg = Config.from_defaults()
        if configured:
            cfg.set(key, value)
        try:
            _validate_pipeline_limits(section, cfg)
        except ConfigurationError as exc:
            return str(exc)
        return None

    if run(configured=False) is not None:
        return f"{key} rejected with NO limit configured — the default must be unconstrained"
    rejected = run(configured=True)
    if rejected is None:
        return f"{key}={value!r} did not reject a step that violates it"
    return None


@probe("validation.pipeline.step_name_max_length")
def _step_name_max_length() -> str | None:
    return _pipeline_probe(
        "validation.pipeline.step_name_max_length", 8, {"name": "x" * 40}
    )


@probe("validation.pipeline.timeout_ms_max")
def _timeout_ms_max() -> str | None:
    return _pipeline_probe("validation.pipeline.timeout_ms_max", 1000, {"timeout_ms": 600000})


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
        dep = entry.get("deprecation")
        if dep:
            if not dep.get("since") or not dep.get("removed_no_earlier_than"):
                problems.append(f"{key}: `deprecation` needs `since` and `removed_no_earlier_than`")
            name = dep.get("probe")
            if name and have_python:
                if name not in PROBES:
                    problems.append(f"{key}: deprecation probe {name!r} is not defined in this file")
                else:
                    try:
                        failure = PROBES[name]()
                    except Exception as exc:  # noqa: BLE001
                        failure = f"probe raised {type(exc).__name__}: {exc}"
                    if failure:
                        problems.append(f"{key}: deprecation probe {name!r} failed — {failure}")

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


_register_deprecation_probe()


if __name__ == "__main__":
    raise SystemExit(main())
