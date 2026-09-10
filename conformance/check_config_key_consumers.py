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
                    for key in keys:
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
