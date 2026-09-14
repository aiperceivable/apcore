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
import contextlib
import json
import re
import sys
import warnings
from collections.abc import Iterator
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
#: §9.2.4's table. Ten when the window opened in spec v1.39.0; seven after
#: v1.44.0 gave the three `observability.tracing.*` keys consumers (§10.1.1) and
#: cancelled their withdrawal; eight since v1.47.0 added `acl.default_effect` as
#: §9.1.3's first application. `WIRED_KEYS` below pins the other half — a table
#: that never shrank passes every case here and fails those.
DEPRECATED_INERT_KEYS = (
    "acl.default_effect",
    "observability.metrics.enabled",
    "observability.metrics.exporter",
    "logging.level",
    "logging.format",
    "acl.audit.enabled",
    "acl.audit.include_denied",
    "acl.audit.log_level",
)


#: The keys spec v1.44.0 took out of §9.2.4, with a valid value for each.
WIRED_KEYS = {
    "observability.tracing.enabled": True,
    "observability.tracing.sampling_rate": 0.25,
    "observability.tracing.exporter": "stdout",
    "observability.tracing.strategy": "off",
}


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

    # 9.2.4 requirement 1 says "a loaded configuration document", which does not
    # exclude one layout. 9.6's NAMESPACE mode puts the framework sections under
    # an `apcore:` root, so a declared view read flat finds nothing there — and
    # apcore-python was silent for every namespace-mode document until that was
    # fixed, while apcore-typescript and apcore-rust were already correct. One
    # SDK diverging on a requirement written for all three is exactly what this
    # guard is for, so both layouts are pinned.
    ns_clean = {"apcore": dict(base)}
    if load(ns_clean):
        return "a clean NAMESPACE-mode document warned"
    ns_declared = {"apcore": {**base, "logging": {"level": "debug"}}}
    hits = load(ns_declared)
    if not hits:
        return "a namespace-mode document declaring logging.level produced no warning"
    if "logging.level" not in hits[0]:
        return f"the namespace-mode notice does not name the key: {hits[0][:120]}"

    # §9.2.4 requirement 1 — the table is the whole list, and a key that has
    # left it MUST NOT warn. Without this half a table that never shrank would
    # satisfy every assertion above.
    for key in WIRED_KEYS:
        doc = dict(base)
        node = doc
        parts = key.split(".")
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        node[parts[-1]] = WIRED_KEYS[key]
        if load(doc):
            return f"{key} gained a consumer in spec v1.44.0 and still warns as deprecated"

    for key in DEPRECATED_INERT_KEYS:
        doc = dict(base)
        node = doc
        parts = key.split(".")
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        node[parts[-1]] = (
            "allow"
            if key == "acl.default_effect"
            else "probe"
            if key.endswith(("level", "format", "exporter"))
            else 1
        )
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


# ---------------------------------------------------------------------------
# The executor limits (#118 audit, 2026-09-10)
# ---------------------------------------------------------------------------


def _probe_module():
    """A trivial registered module, and the client that owns it."""
    from pydantic import BaseModel

    class _In(BaseModel):
        pass

    class _Out(BaseModel):
        ok: bool

    class _M:
        input_schema = _In
        output_schema = _Out
        description = "Config-key probe module."

        def execute(self, inputs: dict, context: object) -> dict:
            return {"ok": True}

    return _M()


def _client(section: dict):
    """An `APCore` built from a document carrying `section`."""
    from apcore import APCore
    from apcore.config import Config

    doc = {"version": "1.0", "project": {"name": "probe"}, **section}
    client = APCore(config=Config(doc))
    client.register("executor.probe.module", _probe_module())
    return client


def _chain_limit_probe(key: str, low: int, chain_len: int, expected: str) -> str | None:
    """A chain limit must reject past the configured bound and admit under it.

    Both halves, because a limit that rejects everything passes the first.
    """
    from apcore.context import Context

    def run(limit: int) -> str:
        client = _client({"executor": {key.rsplit(".", 1)[-1]: limit}})
        ctx = Context.create()
        for i in range(chain_len):
            ctx = ctx.child(f"executor.probe.n{i}" if "depth" in key else "executor.probe.module")
        try:
            client.call("executor.probe.module", {}, context=ctx)
        except Exception as exc:  # noqa: BLE001 — the error TYPE is the observation
            return type(exc).__name__
        return "ok"

    if run(low) != expected:
        return f"{key}={low} did not reject a chain of {chain_len}: got {run(low)}"
    if run(10_000) != "ok":
        return f"{key}=10000 rejected a chain of {chain_len}, so the value is not being read"
    return None


@probe("executor.max_call_depth")
def _max_call_depth() -> str | None:
    return _chain_limit_probe("executor.max_call_depth", 3, 10, "CallDepthExceededError")


@probe("executor.max_module_repeat")
def _max_module_repeat() -> str | None:
    return _chain_limit_probe("executor.max_module_repeat", 2, 6, "CallFrequencyExceededError")


def _timeout_probe(key: str) -> str | None:
    """A timeout must fire below the module's own duration and not above it."""
    import time

    from pydantic import BaseModel

    class _In(BaseModel):
        pass

    class _Out(BaseModel):
        ok: bool

    class _Slow:
        input_schema = _In
        output_schema = _Out
        description = "Sleeps long enough for a 50ms budget to expire."

        def execute(self, inputs: dict, context: object) -> dict:
            time.sleep(0.30)
            return {"ok": True}

    from apcore import APCore
    from apcore.config import Config

    def run(ms: int) -> str:
        doc = {
            "version": "1.0",
            "project": {"name": "probe"},
            "executor": {key.rsplit(".", 1)[-1]: ms},
        }
        client = APCore(config=Config(doc))
        client.register("executor.probe.slow", _Slow())
        try:
            client.call("executor.probe.slow", {})
        except Exception as exc:  # noqa: BLE001
            return type(exc).__name__
        return "ok"

    if run(50) != "ModuleTimeoutError":
        return f"{key}=50 did not time out a 300ms module: got {run(50)}"
    if run(60_000) != "ok":
        return f"{key}=60000 timed out a 300ms module, so the value is not being read"
    return None


@probe("executor.default_timeout")
def _default_timeout() -> str | None:
    return _timeout_probe("executor.default_timeout")


@probe("executor.global_timeout")
def _global_timeout() -> str | None:
    return _timeout_probe("executor.global_timeout")


@probe("_config.strict")
def _config_strict() -> str | None:
    """§9.14: with strict on, an undeclared key inside a framework section is rejected.

    Goes through `Config.load` from a file rather than `Config(dict)`: the
    strict walk runs at load, and a dict-constructed `Config` skips it —
    measured, and worth knowing, because a probe written the obvious way
    reports this key inert.
    """
    import tempfile
    from pathlib import Path

    import yaml

    from apcore.config import Config

    def load(strict: object) -> str:
        doc: dict = {"version": "1.0", "project": {"name": "probe"}, "executor": {"bogus": 1}}
        if strict is not None:
            doc["_config"] = {"strict": strict}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "apcore.yaml"
            path.write_text(yaml.safe_dump(doc), encoding="utf-8")
            try:
                # The temp dir is not the working directory, so #113's project-root
                # notice fires on every load. It is not what this probe measures.
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", DeprecationWarning)
                    Config.load(str(path))
            except Exception as exc:  # noqa: BLE001
                return type(exc).__name__
        return "ok"

    if load(True) == "ok":
        return "_config.strict=true accepted an undeclared key inside a framework section"
    if load(False) != "ok":
        return "_config.strict=false rejected an undeclared key; the flag inverts or is ignored"
    if load(None) != "ok":
        return "an absent _config.strict rejected an undeclared key; the default is not false"
    return None


# ---------------------------------------------------------------------------
# Extension discovery (#118 audit, 2026-09-10)
# ---------------------------------------------------------------------------

#: A file the default discoverer will accept: a class with pydantic
#: input/output schemas and an `execute`. Anything less is discovered and then
#: dropped at registration, which reads as "the key did nothing".
_DISCOVERABLE = '''from pydantic import BaseModel


class In(BaseModel):
    pass


class Out(BaseModel):
    ok: bool


class Mod:
    input_schema = In
    output_schema = Out
    description = "Config-key probe module."

    def execute(self, inputs, context):
        return {"ok": True}
'''


def _discovered(section: dict, build) -> list[str]:
    """Module IDs the registry discovers from a tree `build` lays out.

    Drives `Registry.discover()` — the library's own discovery path — and
    reports what it registered, rather than reading the key back.
    """
    import logging
    import os
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config
    from apcore.registry import Registry

    root = Path(tempfile.mkdtemp())
    build(root)
    doc = {"version": "1.0", "project": {"name": "probe"}, **section}
    (root / "apcore.yaml").write_text(yaml.safe_dump(doc), encoding="utf-8")

    cwd = os.getcwd()
    os.chdir(root)
    try:
        with warnings.catch_warnings():
            # #113's project-root notice fires because the temp dir is not the
            # working directory. Not what these probes measure.
            warnings.simplefilter("ignore", DeprecationWarning)
            config = Config.load(str(root / "apcore.yaml"))
        previous = logging.getLogger().manager.disable
        logging.disable(logging.CRITICAL)
        try:
            registry = Registry(config=config)
            registry.discover()
            return sorted(registry.module_ids)
        finally:
            logging.disable(previous)
    finally:
        os.chdir(cwd)


def _write(path, text: str = _DISCOVERABLE) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


@probe("extensions.root")
def _extensions_root() -> str | None:
    """The configured root selects WHICH tree is scanned."""

    def build(root):
        _write(root / "myext" / "executor" / "here" / "mod.py")
        _write(root / "other" / "executor" / "elsewhere" / "mod.py")

    here = _discovered({"extensions": {"root": "./myext"}}, build)
    there = _discovered({"extensions": {"root": "./other"}}, build)
    if here != ["executor.here.mod"]:
        return f"extensions.root=./myext discovered {here}, expected only executor.here.mod"
    if there != ["executor.elsewhere.mod"]:
        return f"extensions.root=./other discovered {there}, so the key selects nothing"
    return None


@probe("extensions.max_depth")
def _extensions_max_depth() -> str | None:
    """The depth bound must exclude what lies past it and admit what does not."""

    def build(root):
        _write(root / "ext" / "executor" / "shallow" / "mod.py")
        _write(root / "ext" / "executor" / "a" / "b" / "c" / "deep" / "mod.py")

    shallow = _discovered({"extensions": {"root": "./ext", "max_depth": 3}}, build)
    both = _discovered({"extensions": {"root": "./ext", "max_depth": 8}}, build)
    if shallow != ["executor.shallow.mod"]:
        return f"max_depth=3 discovered {shallow}; the bound did not exclude the deep module"
    if len(both) != 2:
        return f"max_depth=8 discovered {both}; the bound excludes what it should admit"
    return None


@probe("extensions.follow_symlinks")
def _extensions_follow_symlinks() -> str | None:
    """A symlinked directory is traversed only when the key says so.

    The link target must live INSIDE the extension root: the scanner confines
    traversal even with the flag on, so a target outside it is refused either
    way and the probe cannot discriminate. Measured — a first version pointed
    the link outside and reported the key inert.
    """

    def build(root):
        ext = root / "ext"
        _write(ext / "executor" / "plain" / "mod.py")
        _write(ext / "hidden" / "target" / "mod.py")
        (ext / "executor" / "sym").symlink_to(ext / "hidden" / "target", target_is_directory=True)

    off = _discovered({"extensions": {"root": "./ext", "follow_symlinks": False}}, build)
    on = _discovered({"extensions": {"root": "./ext", "follow_symlinks": True}}, build)
    if "executor.sym.mod" in off:
        return "follow_symlinks=false traversed a symlinked directory"
    if "executor.sym.mod" not in on:
        return f"follow_symlinks=true did not traverse a symlinked directory: {on}"
    return None


# ---------------------------------------------------------------------------
# ACL and binding discovery (#118 audit, 2026-09-10)
# ---------------------------------------------------------------------------


def _in_project(section: dict, build):
    """Load a config from a throwaway project root laid out by `build`.

    Yields `(config, root)` with the working directory moved there, because
    every key in this group is path-typed and resolves relative to it.
    """
    import contextlib
    import logging
    import os
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config

    @contextlib.contextmanager
    def ctx():
        root = Path(tempfile.mkdtemp())
        build(root)
        doc = {"version": "1.0", "project": {"name": "probe"}, **section}
        (root / "apcore.yaml").write_text(yaml.safe_dump(doc), encoding="utf-8")
        cwd = os.getcwd()
        os.chdir(root)
        previous = logging.getLogger().manager.disable
        logging.disable(logging.CRITICAL)
        try:
            with warnings.catch_warnings():
                # #113's project-root notice; not what these probes measure.
                warnings.simplefilter("ignore", DeprecationWarning)
                yield Config.load(str(root / "apcore.yaml")), root
        finally:
            logging.disable(previous)
            os.chdir(cwd)

    return ctx()


@probe("acl.root")
def _acl_root() -> str | None:
    """The configured directory decides whether an ACL is discovered at all."""
    import yaml

    from apcore.acl import ACL

    doc = {"version": "1.0", "default_effect": "allow", "rules": []}

    def build_at(where: str):
        def build(root):
            (root / where).mkdir(parents=True, exist_ok=True)
            (root / where / "global_acl.yaml").write_text(yaml.safe_dump(doc), encoding="utf-8")

        return build

    with _in_project({"acl": {"root": "./here"}}, build_at("here")) as (config, _):
        found = ACL.discover(config)
    with _in_project({"acl": {"root": "./here"}}, build_at("elsewhere")) as (config, _):
        missing = ACL.discover(config)

    if found is None:
        return "acl.root pointed at a directory holding global_acl.yaml and nothing was discovered"
    if missing is not None:
        # D-64's own invariant: a missing path attaches nothing rather than
        # synthesising an empty default-deny ACL.
        return "acl.root pointed at a directory with no ACL and something was discovered anyway"
    return None


#: A binding file the loader accepts. `target` resolves to a real callable so
#: the entry survives resolution; a binding that fails to resolve is dropped,
#: which from the outside reads as "the key did nothing".
_BINDING_ENTRY = {
    "spec_version": "1.0",
    "bindings": [
        {
            "module_id": "executor.probe.bound",
            "target": "json:dumps",
            "input_schema": {"type": "object", "properties": {}},
            "output_schema": {"type": "object", "properties": {}},
            "description": "Config-key probe binding.",
        }
    ],
}


def _loaded_bindings(section: dict, filename: str) -> object:
    import yaml

    from apcore.bindings import BindingLoader
    from apcore.errors import BindingFileInvalidError
    from apcore.registry import Registry

    def build(root):
        (root / "bdir").mkdir(parents=True, exist_ok=True)
        (root / "bdir" / filename).write_text(yaml.safe_dump(_BINDING_ENTRY), encoding="utf-8")

    with _in_project(section, build) as (config, _):
        try:
            loaded = BindingLoader().load_binding_dir(registry=Registry(), config=config)
        except BindingFileInvalidError:
            return "missing-dir"
    return [m.module_id for m in loaded]


@probe("bindings.dir")
def _bindings_dir() -> str | None:
    """The configured directory is the one scanned."""
    here = _loaded_bindings({"bindings": {"dir": "./bdir"}}, "x.binding.yaml")
    if here != ["executor.probe.bound"]:
        return f"bindings.dir=./bdir loaded {here}, expected the binding it contains"
    elsewhere = _loaded_bindings({"bindings": {"dir": "./nope"}}, "x.binding.yaml")
    if elsewhere != "missing-dir":
        return f"bindings.dir=./nope loaded {elsewhere}, so the key selects nothing"
    return None


@probe("bindings.pattern")
def _bindings_pattern() -> str | None:
    """The configured glob decides which files in that directory are read."""
    default_hit = _loaded_bindings({"bindings": {"dir": "./bdir"}}, "x.binding.yaml")
    default_miss = _loaded_bindings({"bindings": {"dir": "./bdir"}}, "x.yml")
    configured = _loaded_bindings({"bindings": {"dir": "./bdir", "pattern": "*.yml"}}, "x.yml")
    if default_hit != ["executor.probe.bound"]:
        return f"the default pattern did not match x.binding.yaml: {default_hit}"
    if default_miss != []:
        return f"the default pattern matched x.yml, so it is not being applied: {default_miss}"
    if configured != ["executor.probe.bound"]:
        return f"pattern=*.yml did not match x.yml: {configured}"
    return None


# ---------------------------------------------------------------------------
# Schema loading (#118 audit, 2026-09-10)
# ---------------------------------------------------------------------------


def _schema_loader(section: dict, root):
    from apcore.config import Config
    from apcore.schema.loader import SchemaLoader

    doc = {"version": "1.0", "project": {"name": "probe"}, "schema": {"root": str(root), **section}}
    return SchemaLoader(config=Config(doc))


def _schema_tree(marker: str):
    """A `<root>/executor/t/probe.schema.yaml` carrying `marker` as its description."""
    import tempfile
    from pathlib import Path

    import yaml

    root = Path(tempfile.mkdtemp())
    leaf = root / "executor" / "t"
    leaf.mkdir(parents=True)
    (leaf / "probe.schema.yaml").write_text(
        yaml.safe_dump(
            {
                "description": marker,
                "input_schema": {"type": "object", "properties": {"yaml_marker": {}}},
                "output_schema": {"type": "object", "properties": {}},
            }
        ),
        encoding="utf-8",
    )
    return root


@probe("schema.root")
def _schema_root() -> str | None:
    """The configured root decides WHICH definition is loaded, not merely whether one is."""
    import tempfile
    from pathlib import Path

    from apcore.errors import SchemaNotFoundError

    here, there = _schema_tree("FROM_HERE"), _schema_tree("FROM_THERE")
    if _schema_loader({}, here).load("executor.t.probe").description != "FROM_HERE":
        return "schema.root did not select the tree it names"
    if _schema_loader({}, there).load("executor.t.probe").description != "FROM_THERE":
        return "schema.root returned the same definition for two different roots"
    try:
        _schema_loader({}, Path(tempfile.mkdtemp())).load("executor.t.probe")
    except SchemaNotFoundError:
        return None
    return "a root with no definition still resolved one"


@probe("schema.max_ref_depth")
def _schema_max_ref_depth() -> str | None:
    """A `$ref` chain longer than the bound is refused; the same chain under it resolves."""
    import yaml

    from apcore.errors import SchemaMaxDepthExceededError

    root = _schema_tree("deep")
    (root / "leaf.json").write_text('{"type":"object","properties":{"x":{"type":"string"}}}')
    (root / "c.json").write_text('{"$ref":"leaf.json"}')
    (root / "b.json").write_text('{"$ref":"c.json"}')
    (root / "a.json").write_text('{"$ref":"b.json"}')
    (root / "executor" / "t" / "probe.schema.yaml").write_text(
        yaml.safe_dump(
            {
                "description": "deep",
                "input_schema": {"$ref": "a.json"},
                "output_schema": {"type": "object", "properties": {}},
            }
        ),
        encoding="utf-8",
    )

    def resolve(depth: int) -> str:
        loader = _schema_loader({"max_ref_depth": depth}, root)
        try:
            loader.resolve(loader.load("executor.t.probe"))
        except SchemaMaxDepthExceededError:
            return "refused"
        return "resolved"

    if resolve(1) != "refused":
        return "max_ref_depth=1 resolved a three-hop $ref chain"
    if resolve(32) != "resolved":
        return "max_ref_depth=32 refused a three-hop $ref chain, so the value is not being read"
    return None


@probe("schema.strategy")
def _schema_strategy() -> str | None:
    """The strategy decides whether the YAML file or the native model wins.

    Both halves: `native_first` must prefer the model AND the other two must
    prefer the file, or a probe passes on an implementation that ignores the
    key and always reads one of them.
    """
    from pydantic import BaseModel

    class _NativeIn(BaseModel):
        native_marker: str = "x"

    class _NativeOut(BaseModel):
        ok: bool = True

    root = _schema_tree("strategy")

    def props(strategy: str) -> list[str]:
        loader = _schema_loader({"strategy": strategy}, root)
        resolved, _ = loader.get_schema(
            "executor.t.probe", native_input_schema=_NativeIn, native_output_schema=_NativeOut
        )
        schema = getattr(resolved, "schema", None) or getattr(resolved, "json_schema", {})
        return sorted(schema.get("properties", {}))

    if props("native_first") != ["native_marker"]:
        return f"strategy=native_first did not prefer the native model: {props('native_first')}"
    for strategy in ("yaml_first", "yaml_only"):
        if props(strategy) != ["yaml_marker"]:
            return f"strategy={strategy} did not prefer the YAML file: {props(strategy)}"
    return None


# ---------------------------------------------------------------------------
# The sys_modules payload keys (#118 audit, 2026-09-10)
# ---------------------------------------------------------------------------


def _sys_context(section: dict) -> dict:
    """The objects `APCore` assembles from `sys_modules.*`, keyed by role.

    The observable for this family is WHICH object the client builds and what
    it was built with — the caps on the `ErrorHistory`, the presence of a
    `FileOverridesStore`, the thresholds on the platform-notify middleware, the
    subscribers on the emitter. Each is outside `Config` and each is produced by
    `APCore`'s own construction, which is the path an operator actually gets.
    """
    import logging
    import warnings

    from apcore import APCore
    from apcore.config import Config

    doc = {
        "version": "1.0",
        "project": {"name": "probe"},
        "sys_modules": {"enabled": True, **section},
    }
    previous = logging.getLogger().manager.disable
    logging.disable(logging.CRITICAL)
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            return APCore(config=Config(doc))._sys_modules_context
    finally:
        logging.disable(previous)


@probe("sys_modules.events.enabled")
def _sys_events_enabled() -> str | None:
    """The flag gates the three `system.control.*` modules, ALONGSIDE control.enabled.

    Both must be true — measured as a 2x2. Worth pinning precisely, because the
    coupling is not what the key's name suggests: turning `control.enabled` on
    while leaving `events.enabled` off registers no control surface at all.
    """
    import logging
    import warnings

    from apcore import APCore
    from apcore.config import Config

    def control_modules(events: bool, control: bool) -> list[str]:
        doc = {
            "version": "1.0",
            "project": {"name": "probe"},
            "sys_modules": {
                "enabled": True,
                "events": {"enabled": events},
                "control": {"enabled": control},
            },
        }
        previous = logging.getLogger().manager.disable
        logging.disable(logging.CRITICAL)
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                client = APCore(config=Config(doc))
            return sorted(m for m in client.registry.module_ids if m.startswith("system.control."))
        finally:
            logging.disable(previous)

    if not control_modules(events=True, control=True):
        return "events.enabled=true + control.enabled=true registered no control modules"
    if control_modules(events=False, control=True):
        return "events.enabled=false still registered control modules, so the flag is ignored"
    return None


@probe("sys_modules.events.subscribers")
def _sys_events_subscribers() -> str | None:
    """A declared subscriber is assembled onto the emitter; none means none."""
    empty = _sys_context({"events": {"enabled": True, "subscribers": []}})["event_emitter"]
    if getattr(empty, "_subscribers", None):
        return "an empty subscribers list still produced a subscriber"
    one = _sys_context(
        {
            "events": {
                "enabled": True,
                "subscribers": [
                    {
                        "type": "webhook",
                        "url": "https://example.invalid/hook",
                        "event_pattern": "apcore.*",
                    }
                ],
            }
        }
    )["event_emitter"]
    if len(getattr(one, "_subscribers", []) or []) != 1:
        return "a declared webhook subscriber was not assembled onto the emitter"
    return None


def _threshold_probe(field: str, attribute: str, values: tuple) -> str | None:
    """A platform-notify threshold reaches the middleware, and is not a default."""
    for value in values:
        section = {"events": {"enabled": True, "thresholds": {field: value}}}
        middleware = _sys_context(section)["platform_notify_middleware"]
        actual = getattr(middleware, attribute, None)
        if actual != value:
            return f"sys_modules.events.thresholds.{field}={value} reached the middleware as {actual!r}"
    return None


@probe("sys_modules.events.thresholds.error_rate")
def _sys_threshold_error_rate() -> str | None:
    return _threshold_probe("error_rate", "_error_rate_threshold", (0.9, 0.25))


@probe("sys_modules.events.thresholds.latency_p99_ms")
def _sys_threshold_latency() -> str | None:
    return _threshold_probe("latency_p99_ms", "_latency_p99_threshold_ms", (9999, 1234))


def _error_history_probe(field: str, attribute: str, values: tuple) -> str | None:
    for value in values:
        section = {"error_history": {"enabled": True, field: value}}
        history = _sys_context(section)["error_history"]
        actual = getattr(history, attribute, None)
        if actual != value:
            return f"sys_modules.error_history.{field}={value} reached the store as {actual!r}"
    return None


@probe("sys_modules.error_history.max_entries_per_module")
def _sys_history_per_module() -> str | None:
    return _error_history_probe("max_entries_per_module", "_max_entries_per_module", (5, 77))


@probe("sys_modules.error_history.max_total_entries")
def _sys_history_total() -> str | None:
    return _error_history_probe("max_total_entries", "_max_total_entries", (7, 999))


@probe("sys_modules.control.overrides_path")
def _sys_control_overrides_path() -> str | None:
    """The configured path produces a file-backed overrides store; no path, no store."""
    import tempfile
    from pathlib import Path

    events_on = {"events": {"enabled": True}}
    without = _sys_context({"control": {"enabled": True}, **events_on})["overrides_store"]
    if without is not None:
        return f"no overrides_path still produced a store: {type(without).__name__}"

    path = Path(tempfile.mkdtemp()) / "overrides.json"
    store = _sys_context(
        {"control": {"enabled": True, "overrides_path": str(path)}, **events_on}
    )["overrides_store"]
    if store is None:
        return "a configured overrides_path produced no store"
    actual = getattr(store, "_path", getattr(store, "path", None))
    if Path(str(actual)).name != path.name:
        return f"the store was built at {actual!r}, not at the configured path"
    return None


# ---------------------------------------------------------------------------
# Project identity (#118 audit, 2026-09-10)
# ---------------------------------------------------------------------------


@probe("project.name")
def _project_name() -> str | None:
    """The configured name is what `system.manifest.full` reports."""
    import logging
    import warnings

    from apcore import APCore
    from apcore.config import Config

    def manifest(name: str) -> object:
        doc = {
            "version": "1.0",
            "project": {"name": name},
            "sys_modules": {"enabled": True, "manifest": {"enabled": True}},
        }
        previous = logging.getLogger().manager.disable
        logging.disable(logging.CRITICAL)
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                client = APCore(config=Config(doc))
            return client.call("system.manifest.full", {}).get("project_name")
        finally:
            logging.disable(previous)

    for name in ("PROBE_ONE", "PROBE_TWO"):
        reported = manifest(name)
        if reported != name:
            return f"project.name={name!r} was reported as {reported!r}"
    return None


@probe("version")
def _config_version() -> str | None:
    """§9.3 requiredness: a document without it is rejected, one with it loads.

    The narrowest possible consumer, and a real one — this is the key
    `Config.validate`'s required-field check is written about, so an
    implementation that stopped reading it would start accepting documents the
    schema declares invalid.
    """
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config

    def load(doc: dict) -> str:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "apcore.yaml"
            path.write_text(yaml.safe_dump(doc), encoding="utf-8")
            try:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", DeprecationWarning)
                    Config.load(str(path))
            except Exception as exc:  # noqa: BLE001
                return type(exc).__name__
        return "loaded"

    if load({"version": "1.0", "project": {"name": "probe"}}) != "loaded":
        return "a document carrying `version` was rejected"
    if load({"project": {"name": "probe"}}) == "loaded":
        return "a document with no `version` was accepted, so requiredness does not read it"
    return None


def _capture_point(redaction: dict, module_input: dict) -> dict:
    """Run a real module under `redaction` and return the CAPTURED input.

    Drives the path the LIBRARY itself drives — `APCore(config=…)`, a real
    execution, `context.redacted_inputs` — rather than
    `RedactionConfig.from_config` plus a helper. That distinction is why these
    three keys were recorded `partial` for a day: their probes set the key and
    observed an effect, but through a factory **nothing in any SDK's `src/`
    called**, and driving a helper nothing else drives proves the helper. The
    configured rules reached the capture point in no SDK until #120
    (PROTOCOL_SPEC §10.6.1 "Where the rules apply").

    Read through a middleware rather than the `Context` handed to `call`: the
    pipeline derives a child context and the capture point writes to that.
    """
    from pydantic import BaseModel

    from apcore import APCore
    from apcore.config import Config
    from apcore.middleware.base import Middleware

    class _In(BaseModel):
        probe_field: str
        other_field: str

    class _Out(BaseModel):
        ok: bool

    class _M:
        input_schema = _In
        output_schema = _Out
        description = "Config-key probe module."

        def execute(self, inputs: dict, context: object) -> dict:
            return {"ok": True}

    class _Capture(Middleware):
        def __init__(self) -> None:
            self.inputs: dict | None = None

        def before(self, module_id: str, inputs: dict, context: object) -> None:
            self.inputs = context.redacted_inputs
            return None

    client = APCore(
        config=Config(
            {"version": "1.0", "project": {"name": "probe"}, "obs": {"redaction": redaction}}
        )
    )
    client.register("executor.probe.module", _M())
    seen = _Capture()
    client.use(seen)
    client.call("executor.probe.module", module_input)
    if seen.inputs is None:
        raise AssertionError("the capture point did not run")
    return seen.inputs


@probe("obs.redaction.sensitive_keys")
def _sensitive_keys() -> str | None:
    """A field named by the key is redacted at the capture point; one that is not, is not."""
    out = _capture_point(
        {"sensitive_keys": ["probe_field"]},
        {"probe_field": "secret", "other_field": "kept"},
    )
    if out.get("probe_field") == "secret":
        return "a field named by obs.redaction.sensitive_keys was not redacted"
    if out.get("other_field") != "kept":
        return "a field NOT named by obs.redaction.sensitive_keys was redacted"
    return None


@probe("obs.redaction.regex_patterns")
def _regex_patterns() -> str | None:
    """A VALUE matching the key is redacted at the capture point; one that is not, is not."""
    out = _capture_point(
        {"sensitive_keys": [], "regex_patterns": ["^sk-[A-Za-z0-9]{6,}$"]},
        {"probe_field": "sk-abcdef123456", "other_field": "kept"},
    )
    if out.get("probe_field") != "***REDACTED***":
        return f"a value matching obs.redaction.regex_patterns was not redacted: {out!r}"
    if out.get("other_field") != "kept":
        return "a value NOT matching obs.redaction.regex_patterns was redacted"
    return None


@probe("obs.redaction.replacement")
def _replacement() -> str | None:
    """The substitution token at the capture point is the configured one."""
    out = _capture_point(
        {"sensitive_keys": ["probe_field"], "replacement": "<<PROBE>>"},
        {"probe_field": "secret", "other_field": "kept"},
    )
    if out.get("probe_field") != "<<PROBE>>":
        return f"obs.redaction.replacement ignored: got {out.get('probe_field')!r}"
    return None


# ---------------------------------------------------------------------------


_ISO_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")

#: A rise needs enough prose to be reviewable. Long enough to state WHY the
#: number went up, short enough not to be ceremony.
_MIN_RISE_REASON = 60


def _check_ratchet_ledger(
    ratchet: dict[str, object],
    history: list[dict[str, object]],
    entries: dict[str, dict[str, object]],
) -> list[str]:
    """The ceiling must agree with an append-only ledger of its own changes.

    The rule "`inert` and `unaudited` may fall and may not rise" was written in
    a comment, and a comment is enforced only when someone reads it. The first
    raise under it — `inert` 18 -> 19 on 2026-09-10, correcting a key that had
    been mis-recorded as `live` — was made in the same commit that edited the
    number, with the justification in prose beside it. That justification was
    sound; the arrangement was not, because nothing distinguishes it from an
    unsound one.

    So the ceiling is no longer a number that can be edited on its own. It MUST
    equal the `to` of the last `ratchet_history` entry for its status, and the
    entries MUST chain. Raising a ceiling therefore means appending a record
    that says what moved and why, and the diff shows it.

    This does not make a raise impossible — a determined author can write a
    ledger entry too. It makes a raise impossible to perform *silently*, or by
    changing one digit, which is the failure mode a review actually misses.
    """
    problems: list[str] = []
    by_status: dict[str, list[dict[str, object]]] = {}
    for index, record in enumerate(history):
        status = record.get("status")
        if status not in ("inert", "unaudited"):
            problems.append(
                f"ratchet_history[{index}]: status {status!r} is not a ratcheted status"
            )
            continue
        by_status.setdefault(str(status), []).append(record)

        date = record.get("date")
        if not isinstance(date, str) or not _ISO_DATE.match(date):
            problems.append(f"ratchet_history[{index}]: `date` must be YYYY-MM-DD, got {date!r}")
        if not str(record.get("reason") or "").strip():
            problems.append(f"ratchet_history[{index}]: every entry needs a `reason`")

    for status in ("inert", "unaudited"):
        records = by_status.get(status, [])
        if not records:
            problems.append(
                f"ratchet_history: no entry for {status!r}. The ceiling is only meaningful "
                f"beside the record of how it got there."
            )
            continue

        previous: object = None
        for position, record in enumerate(records):
            came_from = record.get("from")
            goes_to = record.get("to")
            if not isinstance(goes_to, int):
                problems.append(f"ratchet_history[{status}][{position}]: `to` must be an integer")
                continue
            if position == 0:
                if came_from is not None and came_from != previous:
                    problems.append(
                        f"ratchet_history[{status}][0]: the first entry opens the ledger and "
                        f"must have `from: null`, got {came_from!r}"
                    )
            elif came_from != previous:
                problems.append(
                    f"ratchet_history[{status}][{position}]: `from` is {came_from!r} but the "
                    f"previous entry ended at {previous!r}. The ledger must chain — a gap is "
                    f"a ceiling change that was never recorded."
                )
            if isinstance(came_from, int) and goes_to > came_from:
                reason = str(record.get("reason") or "")
                if len(reason.strip()) < _MIN_RISE_REASON:
                    problems.append(
                        f"ratchet_history[{status}][{position}]: a RISE ({came_from} -> "
                        f"{goes_to}) needs a `reason` saying why the number went up, not a "
                        f"label. This is the one direction the ratchet exists to resist."
                    )
                keys = record.get("keys")
                if not isinstance(keys, list) or not keys:
                    problems.append(
                        f"ratchet_history[{status}][{position}]: a RISE needs `keys` naming "
                        f"which keys moved to {status!r}"
                    )
                else:
                    # A later entry naming the same key supersedes this one: the
                    # ledger records the past, and a key that rose to `inert` in
                    # September and was wired in October must not turn its own
                    # September entry red for ever. Without this the ledger
                    # penalises exactly the direction it exists to encourage.
                    superseded = {
                        str(k)
                        for later in records[position + 1 :]
                        for k in (later.get("keys") or [])
                    }
                    for key in keys:
                        if str(key) in superseded:
                            continue
                        actual = entries.get(str(key), {}).get("status")
                        if actual is None:
                            problems.append(
                                f"ratchet_history[{status}][{position}]: `keys` names "
                                f"{key!r}, which is not in this manifest"
                            )
                        elif actual != status:
                            problems.append(
                                f"ratchet_history[{status}][{position}]: `keys` names {key!r} "
                                f"as moving to {status!r}, but it is recorded {actual!r}"
                            )
            previous = goes_to

        ceiling = ratchet.get(status)
        last = records[-1].get("to")
        if isinstance(ceiling, int) and isinstance(last, int) and ceiling != last:
            problems.append(
                f"ratchet: the {status!r} ceiling is {ceiling} but the ledger ends at {last}. "
                f"The ceiling is not a number that may be edited on its own — append a "
                f"`ratchet_history` entry saying what moved and why, and the two will agree."
            )

    return problems


def _self_test_ratchet_ledger() -> list[str]:
    """Prove the ledger check rejects each way around it, FOR THE RIGHT REASON.

    The ledger exists because a rule kept in a comment is enforced only when
    someone reads it. A ledger whose enforcement is untested is that same
    arrangement one level down — so each bypass gets a case, and each case
    asserts the *message*, not merely that something was rejected.

    That distinction is not pedantry here; it is the bug this function had on
    its first draft. Every "rise" case was appended to a one-entry base ending
    at 18 while declaring `from: 19`, so all of them were rejected by the CHAIN
    rule and none of them ever exercised the rule they named. Disabling the
    reason-length check left the self-test green. Asserting on the message is
    what makes a case fail when its own rule is removed.

    The last case must stay GREEN: tightening a ceiling has to remain cheap, or
    the ratchet discourages the direction it exists to encourage.
    """
    keys = {"k.inert": {"status": "inert"}, "k.live": {"status": "live"}}
    opens = {"status": "inert", "from": None, "to": 18, "date": "2026-09-09", "reason": "opens"}
    rise = {
        "status": "inert",
        "from": 18,
        "to": 19,
        "date": "2026-09-10",
        "keys": ["k.inert"],
        "reason": "R" * _MIN_RISE_REASON,
    }
    #: A well-formed `unaudited` half, so `inert` is the only thing under test.
    UNAUDITED = {"status": "unaudited", "from": None, "to": 0, "date": "2026-09-09", "reason": "n/a"}

    def check(ceiling: int, hist: list[dict]) -> list[str]:
        return _check_ratchet_ledger({"inert": ceiling, "unaudited": 0}, hist + [UNAUDITED], keys)

    #: (name, ceiling, inert history, substring the rejection MUST contain).
    #: `None` means the case must be accepted.
    cases: list[tuple[str, int, list[dict], str | None]] = [
        ("a ceiling edited on its own", 25, [opens, rise], "may be edited on its own"),
        ("a rise whose reason is a label", 19, [opens, {**rise, "reason": "cleanup"}], "not a label"),
        ("a rise naming a key that is not inert", 19, [opens, {**rise, "keys": ["k.live"]}], "but it is recorded"),
        ("a rise naming no key at all", 19, [opens, {**rise, "keys": []}], "needs `keys` naming"),
        ("a rise naming a key that does not exist", 19, [opens, {**rise, "keys": ["k.nope"]}], "not in this manifest"),
        ("a deleted entry, leaving a broken chain", 19, [rise], "must have `from: null`"),
        ("a gap between two entries", 26, [opens, rise, {**rise, "from": 25, "to": 26}], "must chain"),
        ("a missing ledger", 19, [], "no entry for 'inert'"),
        ("a malformed date", 19, [opens, {**rise, "date": "10/09/2026"}], "must be YYYY-MM-DD"),
        # A FALL, deliberately: on a RISE the length rule fires too, and its
        # message CONTAINS "needs a `reason`" — the substring would then match
        # the wrong rule and the case would pass with this one disabled. Found
        # by disabling each rule in turn and checking this function goes red.
        (
            "an entry with no reason",
            18,
            [opens, rise, {"status": "inert", "from": 19, "to": 18, "date": "2026-09-10", "reason": ""}],
            "every entry needs a `reason`",
        ),
        ("the ledger as it stands", 19, [opens, rise], None),
        (
            "tightening the ceiling after wiring a key",
            18,
            [opens, rise, {"status": "inert", "from": 19, "to": 18, "date": "2026-09-10", "reason": "wired"}],
            None,
        ),
        # The ledger records the PAST. A key that rose to `inert` and was later
        # wired is named by two entries, and the earlier one must not go red for
        # describing what was true when it was written — otherwise fixing a key
        # breaks the ledger, which is the direction the ratchet encourages.
        (
            "a superseded rise, its key since wired",
            18,
            [
                opens,
                {**rise, "keys": ["k.live"]},
                {
                    "status": "inert",
                    "from": 19,
                    "to": 18,
                    "date": "2026-09-11",
                    "keys": ["k.live"],
                    "reason": "wired",
                },
            ],
            None,
        ),
    ]

    failures: list[str] = []
    for name, ceiling, hist, expected in cases:
        found = check(ceiling, hist)
        if expected is None:
            if found:
                failures.append(f"self-test: the ledger check rejected {name}: {found}")
        elif not found:
            failures.append(f"self-test: the ledger check ACCEPTED {name}")
        elif not any(expected in problem for problem in found):
            # Rejected, but by a different rule — the case proves nothing about
            # the one it is named for. This is what made the first draft green
            # while a rule was disabled.
            failures.append(
                f"self-test: {name} was rejected, but not for its own reason "
                f"({expected!r} absent): {found}"
            )
    return failures


@contextlib.contextmanager
def _recording_config_sets() -> "Iterator[set[str]]":
    """Record every dot path a probe puts into a ``Config``.

    Requirement 2 of this file says a probe must "set the key away from its
    default, observe something **outside** `Config` change". Only the second
    half was ever enforced, and the first is the load-bearing one: a probe that
    reaches the behaviour by some other door proves the behaviour exists, not
    that the CONFIG KEY reaches it.

    That is not hypothetical. `acl.default_effect`'s probe constructed
    `ACL(rules=[], default_effect="allow")` directly — never touching `Config` —
    and passed, while no SDK contains a single `config.get("acl.default_effect")`
    reader: the ACL's effect is read from the ACL FILE (`data.get(
    "default_effect", "deny")`). A key nothing reads was recorded as `live` by
    the very guard written to find keys nothing reads.

    Both ways of putting a key into a configuration count, because both are
    ways an operator does it: `Config.set(key, …)`, and declaring it in the
    document a `Config` is constructed from. Watching only `set` rejected a
    probe that built `Config({"obs": {"redaction": {...}}})` — the shape closest
    to a real `apcore.yaml` — which would have pushed probes toward the less
    representative spelling to satisfy the checker.
    """
    from apcore.config import Config

    touched: set[str] = set()

    def walk(node: object, prefix: str = "") -> None:
        if not isinstance(node, dict):
            return
        for key, value in node.items():
            path = f"{prefix}.{key}" if prefix else str(key)
            touched.add(path)
            walk(value, path)

    original_set = Config.set
    original_init = Config.__init__

    def recording_set(self: object, key: str, value: object = None, **kwargs: object) -> object:
        if isinstance(key, str):
            touched.add(key)
        return original_set(self, key, value, **kwargs)  # type: ignore[arg-type]

    def recording_init(self: object, *args: object, **kwargs: object) -> None:
        for candidate in (*args, *kwargs.values()):
            walk(candidate)
        original_init(self, *args, **kwargs)  # type: ignore[arg-type]

    Config.set = recording_set  # type: ignore[method-assign]
    Config.__init__ = recording_init  # type: ignore[method-assign]
    try:
        yield touched
    finally:
        Config.set = original_set  # type: ignore[method-assign]
        Config.__init__ = original_init  # type: ignore[method-assign]


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


# ---------------------------------------------------------------------------
# Unknown namespaces (D-69, spec §9.6.3)
# ---------------------------------------------------------------------------


@probe("_config.allow_unknown")
def _config_allow_unknown() -> str | None:
    """Both halves of §9.6.3's `strict: false` row, which were BOTH inert.

    `false` must drop the namespace (it was stored), and `true` must warn (it
    said nothing). A probe checking only one leaves the row half true.
    """
    import logging
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config

    class _Collect(logging.Handler):
        def __init__(self) -> None:
            super().__init__(logging.DEBUG)
            self.lines: list[str] = []

        def emit(self, record: logging.LogRecord) -> None:
            self.lines.append(record.getMessage())

    def load(allow: bool) -> tuple[object, list[str]]:
        path = Path(tempfile.mkdtemp()) / "apcore.yaml"
        path.write_text(
            yaml.safe_dump({
                "apcore": {"version": "1.0", "project": {"name": "probe"}},
                "_config": {"strict": False, "allow_unknown": allow},
                "billing": {"x": 1},
            }),
            encoding="utf-8",
        )
        handler = _Collect()
        root = logging.getLogger()
        root.addHandler(handler)
        previous = root.level
        root.setLevel(logging.DEBUG)
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                config = Config.load(str(path))
        finally:
            root.removeHandler(handler)
            root.setLevel(previous)
        return config.get("billing.x"), handler.lines

    stored, warned = load(True)
    if stored != 1:
        return "allow_unknown=true did not store the unregistered namespace"
    if not any("billing" in line and "registered" in line for line in warned):
        return "allow_unknown=true stored the namespace and logged no warning (§9.6.3)"

    dropped, _ = load(False)
    if dropped is not None:
        return f"allow_unknown=false left the namespace readable; get() answered {dropped!r}"
    return None


# ---------------------------------------------------------------------------
# Multi-root discovery (D-70, spec §9.1.1)
# ---------------------------------------------------------------------------


@probe("extensions.roots")
def _extensions_roots() -> str | None:
    """Both element shapes, and the NAMESPACE half the key exists for.

    Two roots deriving the same unprefixed ID: without the prefix they collide,
    so this cannot pass on an implementation that reads the paths and drops the
    namespaces — which is what apcore-rust did.
    """
    import logging
    import os
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config
    from apcore.registry import Registry

    module_src = (
        "from pydantic import BaseModel\n\n\n"
        "class In(BaseModel):\n    pass\n\n\n"
        "class Out(BaseModel):\n    ok: bool\n\n\n"
        "class Mod:\n"
        "    input_schema = In\n"
        "    output_schema = Out\n"
        '    description = "Discoverable probe module."\n\n'
        "    def execute(self, inputs, context):\n        return {'ok': True}\n"
    )

    def discover(extensions: dict) -> set[str]:
        root = Path(tempfile.mkdtemp())
        for name in ("alpha", "beta"):
            leaf = root / name / "executor" / "svc"
            leaf.mkdir(parents=True)
            (leaf / "mod.py").write_text(module_src, encoding="utf-8")
        (root / "apcore.yaml").write_text(
            yaml.safe_dump({"version": "1.0", "project": {"name": "probe"},
                            "extensions": extensions}),
            encoding="utf-8",
        )
        cwd = os.getcwd()
        os.chdir(root)
        previous = logging.getLogger().manager.disable
        logging.disable(logging.CRITICAL)
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                config = Config.load(str(root / "apcore.yaml"))
            registry = Registry(config=config)
            registry.discover()
            return set(registry.module_ids)
        finally:
            logging.disable(previous)
            os.chdir(cwd)

    single = discover({"root": "./alpha"})
    if single != {"executor.svc.mod"}:
        return f"the probe tree did not discover its module; got {sorted(single)}"

    strings = discover({"roots": ["./alpha", "./beta"]})
    if strings != {"alpha.executor.svc.mod", "beta.executor.svc.mod"}:
        return f"string entries did not derive namespaces; got {sorted(strings)}"

    objects = discover(
        {"roots": [{"root": "./alpha", "namespace": "aa"},
                   {"root": "./beta", "namespace": "bb"}]}
    )
    if objects != {"aa.executor.svc.mod", "bb.executor.svc.mod"}:
        return f"object entries did not apply their namespaces; got {sorted(objects)}"
    return None


# ---------------------------------------------------------------------------
# The ID map (D-71, spec §9.1.1)
# ---------------------------------------------------------------------------


@probe("id_map.overrides")
def _id_map_overrides() -> str | None:
    """The key must rename a DISCOVERED module, not merely load a file.

    Driven through `Registry.discover()`, because the mechanism was implemented
    in all three SDKs and the config key reached none of them: a probe that
    called `load_id_map` would have passed throughout.
    """
    import logging
    import os
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config
    from apcore.registry import Registry

    module_src = (
        "from pydantic import BaseModel\n\n\n"
        "class In(BaseModel):\n    pass\n\n\n"
        "class Out(BaseModel):\n    ok: bool\n\n\n"
        "class Mod:\n"
        "    input_schema = In\n"
        "    output_schema = Out\n"
        '    description = "Discoverable probe module."\n\n'
        "    def execute(self, inputs, context):\n        return {'ok': True}\n"
    )

    def discover(declare: bool) -> set[str]:
        root = Path(tempfile.mkdtemp())
        leaf = root / "ext" / "executor" / "orig"
        leaf.mkdir(parents=True)
        (leaf / "mod.py").write_text(module_src, encoding="utf-8")
        (root / "map.yaml").write_text(
            yaml.safe_dump(
                {"mappings": [{"file": "executor/orig/mod.py", "id": "executor.renamed.mod"}]}
            ),
            encoding="utf-8",
        )
        doc: dict = {
            "version": "1.0",
            "project": {"name": "probe"},
            "extensions": {"root": "./ext"},
        }
        if declare:
            doc["id_map"] = {"overrides": "./map.yaml"}
        (root / "apcore.yaml").write_text(yaml.safe_dump(doc), encoding="utf-8")
        cwd = os.getcwd()
        os.chdir(root)
        previous = logging.getLogger().manager.disable
        logging.disable(logging.CRITICAL)
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                config = Config.load(str(root / "apcore.yaml"))
            registry = Registry(config=config)
            registry.discover()
            return set(registry.module_ids)
        finally:
            logging.disable(previous)
            os.chdir(cwd)

    undeclared = discover(False)
    if undeclared != {"executor.orig.mod"}:
        return f"the probe tree did not discover its module; got {sorted(undeclared)}"
    declared = discover(True)
    if declared != {"executor.renamed.mod"}:
        return f"id_map.overrides did not rename the discovered module; got {sorted(declared)}"
    return None


# ---------------------------------------------------------------------------
# Tracing from configuration (D-68 C', spec §10.1.1)
# ---------------------------------------------------------------------------


def _tracing_middlewares(tracing: dict) -> list:
    """The TracingMiddleware instances a client built from `tracing` installs.

    Through `APCore`, not `build_tracing_middleware`: the builder is new and was
    never the broken part. What was inert for the whole life of these keys is
    the step BEFORE it — no SDK extracted `observability.tracing.*` from a
    `Config` and installed anything, so `enabled: true` produced no middleware
    and the other four configured one that did not exist.
    """
    from apcore import APCore
    from apcore.config import Config
    from apcore.observability.tracing import TracingMiddleware

    doc = {"version": "1.0", "project": {"name": "probe"}, "observability": {"tracing": tracing}}
    client = APCore(config=Config(doc))
    return [m for m in client.executor.middlewares if isinstance(m, TracingMiddleware)]


@probe("observability.tracing.enabled")
def _tracing_enabled() -> str | None:
    if _tracing_middlewares({}):
        return "no tracing configuration installed a middleware anyway"
    if _tracing_middlewares({"enabled": False}):
        return "observability.tracing.enabled=false installed a middleware"
    if len(_tracing_middlewares({"enabled": True})) != 1:
        return "observability.tracing.enabled=true installed no tracing middleware"
    return None


@probe("observability.tracing.strategy")
def _tracing_strategy() -> str | None:
    """The key that decides whether `sampling_rate` is consulted at all."""
    for name in ("full", "proportional", "error_first", "off"):
        installed = _tracing_middlewares({"enabled": True, "strategy": name})
        if not installed:
            return f"observability.tracing.strategy={name!r} installed no middleware"
        actual = getattr(installed[0], "_sampling_strategy", None)
        if actual != name:
            return f"observability.tracing.strategy={name!r} reached the middleware as {actual!r}"
    return None


@probe("observability.tracing.sampling_rate")
def _tracing_sampling_rate() -> str | None:
    """Measured, not read back: the rate only means anything through a strategy.

    Reading the field back would pass on the implementation this key had before
    v1.44.0 too, where `full` short-circuited ahead of the rate and an operator
    asking for 10% got 100%.
    """
    from apcore.context import Context
    from apcore.observability.tracing import InMemoryExporter

    from apcore import APCore
    from apcore.config import Config

    try:
        from pydantic import BaseModel
    except ImportError:  # pragma: no cover - pydantic is a hard dependency
        return "pydantic is not importable; the probe cannot drive a real call"

    class _In(BaseModel):
        n: int

    class _Out(BaseModel):
        n: int

    class _Echo:
        input_schema = _In
        output_schema = _Out
        description = "Echo module used to drive real calls through the pipeline."

        def execute(self, inputs: dict, context: Context) -> dict:
            return {"n": inputs["n"]}

    doc = {
        "version": "1.0",
        "project": {"name": "probe"},
        "observability": {
            "tracing": {"enabled": True, "strategy": "proportional", "sampling_rate": 0.1}
        },
    }
    client = APCore(config=Config(doc))
    from apcore.observability.tracing import TracingMiddleware

    installed = [m for m in client.executor.middlewares if isinstance(m, TracingMiddleware)]
    if not installed:
        return "a configured sampling_rate installed no middleware to consult it"
    collected = InMemoryExporter()
    installed[0].set_exporter(collected)
    client.register("probe.echo", _Echo())
    runs = 2000
    for i in range(runs):
        client.call("probe.echo", {"n": i})
    fraction = len(collected.get_spans()) / runs
    if not 0.05 < fraction < 0.16:
        return f"sampling_rate=0.1 with strategy=proportional sampled {fraction:.0%}, not ~10%"
    return None


@probe("observability.tracing.exporter")
def _tracing_exporter() -> str | None:
    from apcore.observability.tracing import StdoutExporter

    installed = _tracing_middlewares({"enabled": True, "exporter": "stdout"})
    if not installed or not isinstance(installed[0]._exporter, StdoutExporter):
        return "observability.tracing.exporter='stdout' did not select the stdout exporter"
    # `jaeger` names no implementation: it must install NOTHING rather than
    # silently substituting another exporter (§10.1.1 requirement 4).
    if _tracing_middlewares({"enabled": True, "exporter": "jaeger"}):
        return "observability.tracing.exporter='jaeger' installed a middleware anyway"
    return None


@probe("observability.tracing.otlp_endpoint")
def _tracing_otlp_endpoint() -> str | None:
    """Proved by the refusal first, because that half needs no optional extra.

    `OTLPExporter` requires the `opentelemetry` extra, which is optional and is
    absent in CI, and §10.1.1 requirement 4 makes that a first-class outcome:
    the middleware is NOT installed and the reason is logged. So the load-time
    rejection below is the part that always runs — and it is the stronger proof
    anyway, since a configuration can only be refused for this reason by an
    implementation that READ both keys.
    """
    from apcore.config import Config
    from apcore.errors import ConfigError
    from apcore.observability.tracing import OTLPExporter

    endpoint = "http://collector.internal:4318/v1/traces"

    # §10.1.1 requirement 3: an endpoint nothing reads is a rejected config,
    # never a silent no-op.
    config = Config({
        "version": "1.0", "project": {"name": "probe"},
        "observability": {"tracing": {"exporter": "stdout", "otlp_endpoint": endpoint}},
    })
    try:
        config.validate()
    except ConfigError:
        pass
    else:
        return "otlp_endpoint against a non-OTLP exporter was accepted; it must be rejected at load"

    try:
        OTLPExporter()
    except ImportError:
        # Requirement 4: no middleware, and it must not have substituted one.
        if _tracing_middlewares({"enabled": True, "exporter": "otlp", "otlp_endpoint": endpoint}):
            return "the OTLP exporter is unbuildable here and a middleware was installed anyway"
        return None

    installed = _tracing_middlewares(
        {"enabled": True, "exporter": "otlp", "otlp_endpoint": endpoint}
    )
    if not installed:
        return "observability.tracing.otlp_endpoint with exporter='otlp' installed no middleware"
    reached = str(getattr(installed[0]._exporter, "_endpoint", ""))
    if "collector.internal" not in reached:
        return f"otlp_endpoint did not reach the exporter; it holds {reached!r}"
    return None


# ---------------------------------------------------------------------------
# The declarative pipeline (D-72, spec §5.16 requirements 6 and 7)
# ---------------------------------------------------------------------------


def _strategy_steps(section: object) -> list[str]:
    """Step names of the pipeline an `APCore` builds under a `pipeline:` section.

    Goes through the client, not `build_strategy_from_config`: the builder was
    never the broken part. Its first parameter was a dict the caller supplied,
    and nothing extracted that dict from a loaded `Config` — so a probe that
    called the builder directly would have passed on all three SDKs while
    `pipeline: remove: [acl_check]` in `apcore.yaml` left every step in place.
    """
    from apcore import APCore
    from apcore.config import Config

    doc: dict = {"version": "1.0", "project": {"name": "probe"}}
    if section is not None:
        doc["pipeline"] = section
    client = APCore(config=Config(doc))
    strategy = getattr(client.executor, "_strategy", None)
    return [step.name for step in getattr(strategy, "steps", [])]


@probe("pipeline.remove")
def _pipeline_remove() -> str | None:
    baseline = _strategy_steps(None)
    if "output_validation" not in baseline:
        return "the default pipeline has no output_validation to remove"
    after = _strategy_steps({"remove": ["output_validation"]})
    if "output_validation" in after:
        return "pipeline.remove: ['output_validation'] left the step in the pipeline"
    if len(after) != len(baseline) - 1:
        return f"pipeline.remove removed {len(baseline) - len(after)} steps, expected 1"
    return None


@probe("pipeline.configure")
def _pipeline_configure() -> str | None:
    from apcore import APCore
    from apcore.config import Config

    doc = {
        "version": "1.0",
        "project": {"name": "probe"},
        "pipeline": {"configure": {"input_validation": {"ignore_errors": True}}},
    }
    strategy = getattr(APCore(config=Config(doc)).executor, "_strategy", None)
    steps = {step.name: step for step in getattr(strategy, "steps", [])}
    target = steps.get("input_validation")
    if target is None:
        return "pipeline.configure dropped the step it was meant to configure"
    if not getattr(target, "ignore_errors", False):
        return "pipeline.configure: {input_validation: {ignore_errors: true}} did not take"
    if list(steps) != _strategy_steps(None):
        return "pipeline.configure reordered the pipeline; §5.16 requirement 3 says it must not"
    return None


@probe("pipeline.steps")
def _pipeline_steps() -> str | None:
    """The direction that is not fail-safe: a declared step that never runs."""
    from apcore.pipeline_config import register_step_type, unregister_step_type

    class _ProbeStep:
        name = "probe_inserted"
        description = "Probe step for the pipeline.steps consumer check."

        def __init__(self, _config: object = None) -> None:
            pass

        async def execute(self, state: object) -> object:  # pragma: no cover - never run
            return state

    # `(config_dict) -> BaseStep`, per `register_step_type`'s contract.
    register_step_type("probe_inserted", _ProbeStep)
    try:
        after = _strategy_steps(
            {"steps": [{"name": "probe_inserted", "type": "probe_inserted", "after": "execute"}]}
        )
    finally:
        unregister_step_type("probe_inserted")
    if "probe_inserted" not in after:
        return "pipeline.steps declared a step and the pipeline does not contain it"
    if after.index("probe_inserted") != after.index("execute") + 1:
        return f"pipeline.steps inserted the step at the wrong position: {after}"
    return None


# ---------------------------------------------------------------------------
# Extension discovery (spec v1.42.0, §3.5 / §3.6 A04 step 3a)
# ---------------------------------------------------------------------------


@probe("extensions.ignore_patterns")
def _extensions_ignore_patterns() -> str | None:
    """A skip rule that fails open loads code the operator asked not to load."""
    import logging
    import os
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    from apcore.config import Config
    from apcore.registry import Registry

    module_src = (
        "from pydantic import BaseModel\n\n\n"
        "class In(BaseModel):\n    pass\n\n\n"
        "class Out(BaseModel):\n    ok: bool\n\n\n"
        "class Mod:\n"
        "    input_schema = In\n"
        "    output_schema = Out\n"
        '    description = "Discoverable probe module."\n\n'
        "    def execute(self, inputs, context):\n        return {'ok': True}\n"
    )

    def discover(patterns: list[str] | None) -> set[str]:
        root = Path(tempfile.mkdtemp())
        for sub in ("kept", "skipped"):
            leaf = root / "ext" / "executor" / sub
            leaf.mkdir(parents=True)
            (leaf / "mod.py").write_text(module_src, encoding="utf-8")
        extensions: dict = {"root": "./ext"}
        if patterns is not None:
            extensions["ignore_patterns"] = patterns
        (root / "apcore.yaml").write_text(
            yaml.safe_dump(
                {"version": "1.0", "project": {"name": "probe"}, "extensions": extensions}
            ),
            encoding="utf-8",
        )
        cwd = os.getcwd()
        os.chdir(root)
        previous = logging.getLogger().manager.disable
        logging.disable(logging.CRITICAL)
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                config = Config.load(str(root / "apcore.yaml"))
            registry = Registry(config=config)
            registry.discover()
            return set(registry.module_ids)
        finally:
            logging.disable(previous)
            os.chdir(cwd)

    unfiltered = discover(None)
    if not {m for m in unfiltered if "skipped" in m}:
        return "the probe tree registered no module under 'skipped'; the probe cannot measure"
    filtered = discover(["skipped"])
    still_there = {m for m in filtered if "skipped" in m}
    if still_there:
        return f"extensions.ignore_patterns: ['skipped'] still registered {sorted(still_there)}"
    if not {m for m in filtered if "kept" in m}:
        return "extensions.ignore_patterns: ['skipped'] also skipped the sibling it must keep"
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--sdk-root", default="..", help="directory holding the sibling SDK checkouts")
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="check that the ratchet-ledger enforcement rejects each way around it",
    )
    args = ap.parse_args()

    if args.self_test:
        failures = _self_test_ratchet_ledger()
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        print(f"ratchet-ledger self-test: {'FAILED' if failures else 'every bypass is rejected'}")
        return 1 if failures else 0

    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    entries: dict[str, dict] = manifest["keys"]
    ratchet: dict[str, int] = manifest["ratchet"]
    history: list[dict] = manifest.get("ratchet_history", [])
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
        # "The directory exists" is not "apcore imports". A checkout without the
        # SDK's runtime dependencies installed satisfies the first and fails the
        # second, and the failure then arrives as an ImportError traceback from
        # whichever probe ran first — or, worse, from the `Config.set` recorder,
        # which is entered OUTSIDE the per-probe try/except and killed the whole
        # script. Measured on this checker's first CI run. Report it as a
        # problem instead, naming the missing dependency, so the cause is legible
        # and the probes are not silently skipped.
        try:
            import apcore.config  # noqa: F401
        except ImportError as exc:
            problems.append(
                f"apcore-python is present at {python_src} but does not import: {exc}. "
                f"Every probe would be skipped, so this check would pass without "
                f"verifying anything. Install the SDK's runtime dependencies "
                f"(pydantic, pyyaml, jsonschema) in whatever job runs this."
            )
            have_python = False

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
            with _recording_config_sets() as touched:
                try:
                    failure = PROBES[name]()
                except Exception as exc:  # noqa: BLE001 — a probe that cannot run IS the finding
                    failure = f"probe raised {type(exc).__name__}: {exc}"
            if failure:
                problems.append(f"{key}: probe {name!r} failed — {failure}")
            elif key not in touched:
                problems.append(
                    f"{key}: probe {name!r} passed WITHOUT ever putting {key!r} into a "
                    f"`Config` — neither through `Config.set` nor in a document it was "
                    f"constructed from — so it cannot show that the key reaches anything. It "
                    f"observed something outside `Config` change, which was the stated "
                    f"criterion and is not sufficient: `acl.default_effect`'s probe built "
                    f"`ACL(default_effect=...)` directly and passed for a config key NO SDK "
                    f"reads. Set the key and drive a path the library itself runs, or record "
                    f"the key as `partial`/`inert` with a reason."
                )

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
    problems.extend(_check_ratchet_ledger(ratchet, history, entries))

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
