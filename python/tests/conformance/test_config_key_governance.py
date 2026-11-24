"""Drive `config_key_governance.json` — the configuration key-surface guard.

The fixture derives its `allowed_keys` / `canonical_defaults` from the canonical
schemas, so this test is really asking: does `apcore.config`'s idea of the config
surface still match `schemas/`?

It exists because four separate instances of the same defect shipped undetected:
`schema.validation.*` validated by every SDK and declared by no schema, a frozen
`version`/`project` default pair that made the required-field check unreachable,
`middleware.circuit_breaker.*` forbidden by `apcore-config.schema.json` yet
validated everywhere and read nowhere, and a missing Rust default table that
resolved 15 documented keys to null. None was findable by any existing test.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.config import (
    _GLOBAL_NS_REGISTRY,
    Config,
    _collect_unknown_framework_keys,
    _CONSTRAINTS,
    _DEFAULTS,
    _FRAMEWORK_CONFIG_KEYS,
    _FRAMEWORK_SECTION_KEYS,
)
from apcore.errors import ConfigError

from .canonical_fixtures import fixtures_dir, schemas_dir, load_fixture

FIXTURE = load_fixture("config_key_governance.json")
ALLOWED: set[str] = set(FIXTURE["allowed_keys"])
CANONICAL: dict[str, Any] = FIXTURE["canonical_defaults"]


def _flatten(tree: dict[str, Any], prefix: str = "") -> dict[str, Any]:
    """Flatten a nested default table to {dot_path: value}.

    An empty dict is a leaf: it is a declared value, not a subtree to descend.
    """
    out: dict[str, Any] = {}
    for key, val in tree.items():
        path = f"{prefix}.{key}" if prefix else key
        if isinstance(val, dict) and val:
            out.update(_flatten(val, path))
        else:
            out[path] = val
    return out


DEFAULT_KEYS = _flatten(_DEFAULTS)
CONSTRAINT_KEYS = set(_CONSTRAINTS)

#: canonical case id -> the case body. Each test below takes its verdict from
#: the case's own ``expected`` block ("violations": [], "missing": [],
#: "mismatched": []) instead of hard-coding ``== []``: an expectation no driver
#: reads is not a contract, it only looks like one.
CASES: dict[str, Any] = {case["id"]: case for case in FIXTURE["test_cases"]}


def _values_equal(sdk_value: Any, canonical_value: Any) -> bool:
    """Fixture driver_contract: compare numerically when both sides are numbers."""
    if isinstance(sdk_value, bool) or isinstance(canonical_value, bool):
        return sdk_value is canonical_value
    if isinstance(sdk_value, (int, float)) and isinstance(canonical_value, (int, float)):
        return float(sdk_value) == float(canonical_value)
    return bool(sdk_value == canonical_value)


class TestConfigKeySurfaceGovernance:
    def test_default_table_declares_no_undeclared_key(self) -> None:
        """A default for a key no schema declares is an SDK-invented default.

        `apcore-config.schema.json` is `additionalProperties: false`, so a user
        config carrying such a key fails the canonical schema while the SDK
        quietly supplies a value for it.
        """
        expected = CASES["sdk_default_table_declares_no_undeclared_key"]["expected"]
        violations = sorted(set(DEFAULT_KEYS) - ALLOWED)
        assert violations == expected["violations"], (
            "_DEFAULTS declares keys no canonical schema allows:\n  "
            + "\n  ".join(f"{k} = {DEFAULT_KEYS[k]!r}" for k in violations)
            + "\nEither add them to the appropriate schema in apcore/schemas/ "
            "(and regenerate the fixture) or remove them from _DEFAULTS."
        )

    def test_constraint_table_declares_no_undeclared_key(self) -> None:
        """Validating a key the canonical schema forbids is worse than not
        validating it: it tells the operator the key is understood."""
        expected = CASES["sdk_constraint_table_declares_no_undeclared_key"]["expected"]
        violations = sorted(CONSTRAINT_KEYS - ALLOWED)
        assert violations == expected["violations"], (
            "_CONSTRAINTS validates keys no canonical schema allows:\n  " + "\n  ".join(violations)
        )

    def test_reproduces_every_canonical_default(self) -> None:
        """A missing entry means the key resolves to None here while its peers
        return the documented value."""
        expected = CASES["sdk_reproduces_every_canonical_default"]["expected"]
        missing = sorted(set(CANONICAL) - set(DEFAULT_KEYS))
        assert missing == expected["missing"], (
            "defaults.schema.json declares defaults _DEFAULTS does not carry:\n  "
            + "\n  ".join(f"{k} = {CANONICAL[k]!r}" for k in missing)
        )

    def test_default_values_match_canonical_defaults(self) -> None:
        """Compare the RESOLVED default view, per the fixture's driver_contract.

        Reading through ``Config.from_defaults().get()`` rather than indexing
        ``_DEFAULTS`` directly is what the caller actually experiences, and it
        keeps this check comparable with apcore-rust, whose serde struct
        defaults never appear in its default table at all.
        """
        expected = CASES["sdk_default_values_match_canonical_defaults"]["expected"]
        resolved = Config.from_defaults()
        mismatched = sorted(
            key for key, canonical_value in CANONICAL.items() if not _values_equal(resolved.get(key), canonical_value)
        )
        assert mismatched == expected["mismatched"], (
            "Config.from_defaults() resolves values defaults.schema.json does not declare:\n  "
            + "\n  ".join(f"{k}: SDK {resolved.get(k)!r} != canonical {CANONICAL[k]!r}" for k in mismatched)
        )

    @pytest.mark.parametrize("key", sorted(CANONICAL))
    def test_default_table_entry_matches_canonical(self, key: str) -> None:
        """Per-key form of the check above, against the table itself.

        Kept alongside the resolved-view test so a table/resolution disagreement
        (a default that only exists in the ``get()`` path, or vice versa) is
        reported against the specific key.
        """
        assert _values_equal(DEFAULT_KEYS[key], CANONICAL[key]), (
            f"{key}: _DEFAULTS has {DEFAULT_KEYS[key]!r}, " f"defaults.schema.json declares {CANONICAL[key]!r}"
        )

    def test_every_canonical_case_has_a_driver(self) -> None:
        """A case added on the spec side must fail here, not pass unnoticed."""
        covered = {
            "sdk_default_table_declares_no_undeclared_key",
            "sdk_constraint_table_declares_no_undeclared_key",
            "sdk_reproduces_every_canonical_default",
            "sdk_default_values_match_canonical_defaults",
            "unknown_framework_key_is_retained_by_default",
            "unknown_framework_key_is_rejected_under_strict",
            "observability_namespace_declares_what_the_schema_declares",
            "the_three_builtin_namespaces_are_registered",
        }
        assert set(CASES) == covered, (
            f"config_key_governance.json cases without a driver: {sorted(set(CASES) - covered)}; "
            f"drivers claiming cases the fixture no longer defines: {sorted(covered - set(CASES))}"
        )

    def test_fixture_is_derived_not_authored(self) -> None:
        """Guard the guard: if the fixture ever stops naming its generator, the
        next person to hand-edit it will make it a second source of truth."""
        contract = FIXTURE["driver_contract"]["sources"]
        assert "regenerated" in contract and "do NOT hand-edit" in contract
        # Check the declared sources against the SPEC REPO, not against a
        # literal transcription of themselves: a list compared to its own copy
        # cannot fail when a schema is renamed or deleted. (Same tautology
        # shape as apcore-python#32 / aiperceivable/apcore#81.)
        spec_repo = schemas_dir().parent
        declared = FIXTURE["canonical_sources"]
        assert declared, "the fixture must name the schemas it was generated from"
        missing = [src for src in declared if not (spec_repo / src).is_file()]
        assert missing == [], f"canonical_sources names schema files that do not exist under {spec_repo}: {missing}"


# ---------------------------------------------------------------------------
# §9.14 reject_unknown_framework_keys
# ---------------------------------------------------------------------------


def _nest(dotted: dict[str, Any]) -> dict[str, Any]:
    """Expand the fixture's flat ``{"executor.max_call_depth": 7}`` into a tree."""
    tree: dict[str, Any] = {}
    for path, value in dotted.items():
        cursor = tree
        parts = path.split(".")
        for part in parts[:-1]:
            cursor = cursor.setdefault(part, {})
        cursor[parts[-1]] = value
    return tree


#: Keys every document needs to satisfy `_REQUIRED_FIELDS`. The fixture omits
#: them because they are not what these two cases are about; without them a
#: legacy-mode load fails on the required-field check and the strict assertion
#: below would pass for the wrong reason.
_REQUIRED_SCAFFOLD = {"version": "1.0.0", "project": {"name": "governance-test"}}


def _write_legacy(tmp_path: Path, case: dict[str, Any]) -> str:
    """Legacy mode: the whole document *is* the apcore namespace (§9.14 step 1)."""
    tree = _nest(case["config"])
    meta = tree.pop("_config", None)
    document: dict[str, Any] = {**_REQUIRED_SCAFFOLD, **tree}
    if meta is not None:
        document["_config"] = meta
    path = tmp_path / "legacy.yaml"
    path.write_text(yaml.safe_dump(document, sort_keys=False))
    return str(path)


def _write_namespace(tmp_path: Path, case: dict[str, Any]) -> str:
    """Namespace mode: framework sections live under ``apcore:`` (§9.14 step 2)."""
    tree = _nest(case["config"])
    meta = tree.pop("_config", None)
    document: dict[str, Any] = {"apcore": {**_REQUIRED_SCAFFOLD, **tree}}
    if meta is not None:
        document["_config"] = meta
    path = tmp_path / "namespace.yaml"
    path.write_text(yaml.safe_dump(document, sort_keys=False))
    return str(path)


#: Both modes, because §9.6.3 clause (b) and §9.14 step 1 say the rule holds in
#: legacy mode too — a fix applied only to `_validate_namespace_mode` would leave
#: the older and more common file shape unenforced.
_MODES = {"legacy": _write_legacy, "namespace": _write_namespace}


def _partition_config_keys(case: dict[str, Any]) -> tuple[list[str], list[str]]:
    """Split the case's config keys into (declared, undeclared) by `allowed_keys`.

    Asking the fixture's own canonical projection which key is undeclared beats
    recognising the ``zz_`` prefix: rename the key in the fixture and this still
    tests what the case is named after.
    """
    keys = [key for key in case["config"] if not key.startswith("_config.")]
    return [k for k in keys if k in ALLOWED], [k for k in keys if k not in ALLOWED]


class TestUnknownFrameworkKeys:
    """`_config.strict` governs keys *inside* the framework sections (§9.14).

    Non-strict retains them; strict rejects them and names every one.
    """

    @pytest.mark.parametrize("mode", sorted(_MODES))
    def test_unknown_framework_key_is_retained_by_default(self, tmp_path: Path, mode: str) -> None:
        """Default tier: the key survives the load and reads back through get().

        Asserted by READING IT BACK, per the fixture's
        `default_tier_must_be_asserted_by_reading_it_back` contract. A load that
        merely does not raise is equally true of an implementation that dropped
        the key at parse time, which is the defect this case exists to catch.
        """
        case = CASES["unknown_framework_key_is_retained_by_default"]
        expected = case["expected"]
        declared, undeclared = _partition_config_keys(case)
        assert declared and undeclared, "the fixture case must carry one of each kind of key"

        path = _MODES[mode](tmp_path, case)
        config = Config.load(path, validate=True)  # no `_config.strict` in this case
        assert expected["load_succeeds"] is True and expected["error_raised"] is False

        assert (
            config.get(declared[0]) == expected["get_declared_key"]
        ), f"[{mode}] declared key {declared[0]!r} did not survive the load"
        assert config.get(undeclared[0]) == expected["get_undeclared_key"], (
            f"[{mode}] undeclared key {undeclared[0]!r} was discarded at parse time. "
            "§9.14: with `_config.strict` absent or false the key MUST be retained "
            "and readable — 'the operator wrote it and it vanished' is "
            "indistinguishable from 'the operator never wrote it'."
        )

    @pytest.mark.parametrize("mode", sorted(_MODES))
    def test_unknown_framework_key_is_rejected_under_strict(self, tmp_path: Path, mode: str) -> None:
        """Strict tier: CONFIG_INVALID naming EVERY offending key.

        The case declares two undeclared keys in two different sections on
        purpose. Asserting only that a load raised would be satisfied by an
        implementation that stops at the first one and makes the operator
        restart once per typo.
        """
        case = CASES["unknown_framework_key_is_rejected_under_strict"]
        expected = case["expected"]
        assert expected["load_succeeds"] is False

        path = _MODES[mode](tmp_path, case)
        with pytest.raises(ConfigError) as exc:
            Config.load(path, validate=True)

        assert exc.value.code == expected["error_code"]
        message = str(exc.value)
        missing = [key for key in expected["error_names_all_offending_keys"] if key not in message]
        assert missing == [], (
            f"[{mode}] the error named only some offending keys; missing {missing}.\n"
            f"Error was:\n{message}\n"
            "§9.14 step 3: collect every unknown key and raise once, so one "
            "restart shows the operator the whole problem."
        )

    def test_allow_unknown_does_not_relax_strict(self, tmp_path: Path) -> None:
        """`allow_unknown` is about top-level NAMESPACES, not section keys.

        §9.14: stretching one field across two granularities would make its
        meaning depend on where it is read, so `allow_unknown: true` must not
        buy back a key `strict` rejects.
        """
        case = CASES["unknown_framework_key_is_rejected_under_strict"]
        tree = _nest(case["config"])
        meta = tree.pop("_config")
        meta["allow_unknown"] = True
        document = {**_REQUIRED_SCAFFOLD, **tree, "_config": meta}
        path = tmp_path / "allow_unknown.yaml"
        path.write_text(yaml.safe_dump(document, sort_keys=False))

        with pytest.raises(ConfigError) as exc:
            Config.load(str(path), validate=True)
        assert exc.value.code == "CONFIG_INVALID"

    def test_strict_accepts_a_fully_declared_document(self, tmp_path: Path) -> None:
        """The counterweight: strict must not reject keys the schema declares.

        Without this, `_FRAMEWORK_SECTION_KEYS` could be emptied and every test
        above would still pass.
        """
        document = {
            **_REQUIRED_SCAFFOLD,
            "_config": {"strict": True},
            "executor": {"max_call_depth": 7, "default_timeout": 1000},
            "acl": {"root": "./acl", "default_effect": "deny", "audit": {"enabled": True}},
            "observability": {"tracing": {"enabled": False}},
            # sys_modules subsections are declared by sys-modules.schema.json,
            # not by SysModulesConfig; taking only the narrower of the two would
            # reject this document.
            "sys_modules": {"enabled": True, "usage": {"enabled": True}},
        }
        path = tmp_path / "strict_ok.yaml"
        path.write_text(yaml.safe_dump(document, sort_keys=False))

        config = Config.load(str(path), validate=True)
        assert config.get("executor.max_call_depth") == 7
        assert config.get("sys_modules.usage.enabled") is True


def _resolve_ref(node: Any, doc: dict[str, Any], schemas_dir: Path | None = None) -> Any:
    """Follow ``$ref`` pointers within *doc*, and across files when *schemas_dir* is given.

    Local-only resolution is no longer sufficient: `apcore-config.schema.json`
    delegates `sys_modules` to the sibling `sys-modules.schema.json` — which
    owns that namespace under protocol-spec §9.15.3 — instead of restating its
    keys. A resolver that stops at a non-`#/` `$ref` returns the ref node
    itself, a node carrying neither `properties` nor `additionalProperties`,
    which reads as "declares nothing, and is open". Both readings are wrong:
    the target declares seven keys and is `additionalProperties: false`.

    *schemas_dir* is opt-in so callers that only ever see local refs keep the
    previous behaviour unchanged.
    """
    seen = 0
    current = doc
    while isinstance(node, dict) and isinstance(node.get("$ref"), str):
        ref: str = node["$ref"]
        file_part, _, pointer = ref.partition("#")
        if file_part:
            if schemas_dir is None:
                break
            current = json.loads((schemas_dir / file_part).read_text())
            node = current
        target: Any = current
        for part in pointer.lstrip("/").split("/"):
            if part:
                target = target[part]
        node = target
        seen += 1
        assert seen <= 16, f"$ref cycle at {ref}"
    return node


def _declared_keys(node: Any, doc: dict[str, Any]) -> set[str]:
    """Every property name a schema node declares, including combinator branches.

    `ExtensionsConfig` splits `root` and `roots` across a `oneOf`, so reading
    `properties` alone would report both as undeclared.
    """
    node = _resolve_ref(node, doc)
    if not isinstance(node, dict):
        return set()
    keys = set(node.get("properties") or {})
    for combinator in ("allOf", "anyOf", "oneOf"):
        for branch in node.get(combinator) or []:
            keys |= _declared_keys(branch, doc)
    return keys


def test_framework_key_surface_is_derived_from_the_canonical_schema() -> None:
    """`_FRAMEWORK_CONFIG_KEYS` must equal what the canonical schemas declare.

    This is the drift guard the runtime enforcement depends on: the schema files
    ship with the spec repo, not with this package, so
    `_collect_unknown_framework_keys` reads a mirror. A key added to
    `apcore-config.schema.json` must fail here rather than go unenforced.

    Compared as full dot-paths at every depth. It used to compare a
    `section -> direct child names` map, which could not express — and therefore
    could not guard — the nested closedness those schemas actually declare
    (sync finding A-D-020).
    """
    fixture = json.loads((fixtures_dir() / "config_key_governance.json").read_text())

    def _find_allowed(node: object) -> list[str] | None:
        if isinstance(node, dict):
            for key, value in node.items():
                if key == "allowed_keys" and isinstance(value, list):
                    return value
                found = _find_allowed(value)
                if found is not None:
                    return found
        elif isinstance(node, list):
            for item in node:
                found = _find_allowed(item)
                if found is not None:
                    return found
        return None

    declared = _find_allowed(fixture)
    assert declared, "config_key_governance.json must carry an allowed_keys list"

    expected = set(declared)
    actual = set(_FRAMEWORK_CONFIG_KEYS)
    assert actual == expected, (
        "_FRAMEWORK_CONFIG_KEYS has drifted from the canonical schemas.\n"
        f"  declared by the schemas but not enforced here: {sorted(expected - actual)}\n"
        f"  enforced here but not declared by the schemas: {sorted(actual - expected)}"
    )


def test_nested_typo_is_rejected_under_strict() -> None:
    """A misspelling one level below a section MUST be rejected.

    The canonical schemas are `additionalProperties: false` at every level, so
    `observability.tracing.sampling_rat` is invalid — but a one-level check
    passed it, because its parent `tracing` IS declared. The misspelled sampling
    rate then fell back to its default silently, which is the failure strict
    mode exists to prevent.
    """
    errors = _collect_unknown_framework_keys({"observability": {"tracing": {"enabled": True, "sampling_rat": 1.0}}})
    assert any("observability.tracing.sampling_rat" in e for e in errors), errors


def test_declared_nested_keys_are_accepted() -> None:
    """The recursion must not over-reach into rejecting declared nested keys."""
    errors = _collect_unknown_framework_keys(
        {
            "observability": {"tracing": {"enabled": True, "sampling_rate": 1.0}},
            "acl": {"audit": {"enabled": True, "log_level": "info"}},
        }
    )
    assert errors == [], errors


def test_undeclared_subtree_reports_once_not_per_leaf() -> None:
    """An unknown container is one error, not one per key beneath it."""
    errors = _collect_unknown_framework_keys({"observability": {"tracin": {"enabled": True, "sampling_rate": 1.0}}})
    assert len(errors) == 1, errors
    assert "observability.tracin" in errors[0]


def test_every_section_the_schema_closes_is_enforced() -> None:
    """Guard the guard: every section is `additionalProperties: false` upstream.

    If a section ever stops being closed, enforcing closedness against it under
    strict becomes a rejection the canonical schema does not back.
    """
    schemas = schemas_dir()
    config_schema = json.loads((schemas / "apcore-config.schema.json").read_text())
    open_sections = []
    for section in _FRAMEWORK_SECTION_KEYS:
        node = _resolve_ref(config_schema["properties"][section], config_schema, schemas_dir=schemas)
        closed = node.get("additionalProperties") is False or node.get("unevaluatedProperties") is False
        if not closed:
            open_sections.append(section)
    assert (
        open_sections == []
    ), f"these sections are enforced as closed but the canonical schema leaves them open: {open_sections}"


def _schema_defaults(node: Any, prefix: str = "") -> dict[str, Any]:
    """Every `default:` the schema declares, as full dot-paths."""
    out: dict[str, Any] = {}
    for key, value in (node.get("properties") or {}).items():
        path = f"{prefix}.{key}" if prefix else key
        if "default" in value:
            out[path] = value["default"]
        if value.get("properties"):
            out.update(_schema_defaults(value, path))
    return out


def test_legacy_default_table_mirrors_defaults_schema_exactly() -> None:
    """`_DEFAULTS` must be `defaults.schema.json`, key for key and value for value.

    Not "a subset of what some schema allows" — that is
    `test_default_table_declares_no_undeclared_key`, and it passes on a table
    with extra keys because `sys-modules.schema.json` allows them too. The
    contract this pins is narrower: `defaults.schema.json` IS the legacy default
    table, and it is `additionalProperties: false`.

    apcore-typescript's `DEFAULTS` and apcore-rust's `CONFIG_DEFAULTS` mirror it
    exactly. apcore-python carried six extra `sys_modules` leaves, so
    `get("sys_modules.error_history.max_entries_per_module")` answered 50 here
    and undefined/None in both peers over the same call (sync finding A-D-021).
    """
    schemas = schemas_dir()
    canonical = _schema_defaults(json.loads((schemas / "defaults.schema.json").read_text()))

    def _leaves(node: Any, prefix: str = "") -> dict[str, Any]:
        out: dict[str, Any] = {}
        for key, value in node.items():
            path = f"{prefix}.{key}" if prefix else key
            if isinstance(value, dict):
                out.update(_leaves(value, path))
            else:
                out[path] = value
        return out

    actual = _leaves(_DEFAULTS)
    assert actual.keys() == canonical.keys(), (
        "_DEFAULTS has drifted from defaults.schema.json.\n"
        f"  declared by the schema but missing here: {sorted(canonical.keys() - actual.keys())}\n"
        f"  present here but not declared: {sorted(actual.keys() - canonical.keys())}"
    )
    mismatched = {k: (actual[k], canonical[k]) for k in canonical if not _values_equal(actual[k], canonical[k])}
    assert mismatched == {}, f"_DEFAULTS values disagree with the schema: {mismatched}"


def test_sys_modules_namespace_supplies_every_declared_default() -> None:
    """The other half: what the legacy table does not carry, the namespace must.

    §9.15.3 gives `sys-modules.schema.json` ownership of this namespace, and it
    declares fourteen defaults. Trimming `_DEFAULTS` to the single key
    `defaults.schema.json` declares is only correct because the namespace
    registration answers for the rest — it did not, for `error_history` and
    `events.subscribers`, so removing them from the legacy table alone would
    have made them unreachable in BOTH modes.

    `control.overrides_path` is excluded: its declared default is null, which a
    namespace default cannot express distinctly from absence.
    """
    schemas = schemas_dir()
    declared = _schema_defaults(json.loads((schemas / "sys-modules.schema.json").read_text()))
    expected = {k: v for k, v in declared.items() if v is not None}
    assert len(expected) >= 13, f"the schema's default set looks wrong: {expected}"

    registration = _GLOBAL_NS_REGISTRY["sys_modules"]

    def _leaves(node: Any, prefix: str = "") -> dict[str, Any]:
        out: dict[str, Any] = {}
        for key, value in node.items():
            path = f"{prefix}.{key}" if prefix else key
            if isinstance(value, dict):
                out.update(_leaves(value, path))
            else:
                out[path] = value
        return out

    supplied = _leaves(registration.defaults or {})
    missing = sorted(expected.keys() - supplied.keys())
    assert missing == [], "the sys_modules namespace does not supply defaults its own schema " f"declares: {missing}"
    mismatched = {k: (supplied[k], expected[k]) for k in expected if not _values_equal(supplied[k], expected[k])}
    assert mismatched == {}, f"namespace defaults disagree with the schema: {mismatched}"


# ---------------------------------------------------------------------------
# §9.15 built-in namespace registrations (D-144)
# ---------------------------------------------------------------------------


def _load_bare_namespace_document(tmp_path: Path) -> Config:
    """driver_contract.builtin_namespace: a namespace-mode document declaring nothing else."""
    path = tmp_path / "bare.yaml"
    path.write_text(yaml.safe_dump({"apcore": dict(_REQUIRED_SCAFFOLD)}, sort_keys=False))
    return Config.load(str(path), validate=True)


class TestBuiltinNamespaces:
    def test_observability_namespace_declares_what_the_schema_declares(self, tmp_path: Path) -> None:
        case = CASES["observability_namespace_declares_what_the_schema_declares"]
        name = case["builtin_namespace"]
        config = _load_bare_namespace_document(tmp_path)
        leaves = set(_flatten(config.namespace(name), prefix=name))

        violations = sorted(leaves - ALLOWED)
        assert (
            violations == case["expected"]["violations"]
        ), f"the built-in {name!r} registration declares keys no schema declares: {violations}"
        missing = sorted(set(case["expected"]["required_keys"]) - leaves)
        assert missing == [], f"the built-in {name!r} registration lacks {missing}"

    def test_the_three_builtin_namespaces_are_registered(self, tmp_path: Path) -> None:
        case = CASES["the_three_builtin_namespaces_are_registered"]
        _load_bare_namespace_document(tmp_path)
        registered = {ns["name"]: ns["env_prefix"] for ns in Config.registered_namespaces()}
        missing = sorted(
            f"{name} ({prefix})"
            for name, prefix in case["builtin_namespaces_registered"].items()
            if registered.get(name) != prefix
        )
        assert missing == case["expected"]["missing"], f"built-in namespaces not registered as declared: {missing}"
