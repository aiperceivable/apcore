"""The §9.2.4 inert-key deprecation notice, and its §9.2.4.1 ACL-file half.

PROTOCOL_SPEC §9.2.4 / §9.2.4.1, tracking issue aiperceivable/apcore#118.

Ten declared configuration keys reach no consumer in any apcore SDK. They are
schema-declared, environment-overridable, accepted under ``_config.strict`` and
documented with defaults — and inert. v1.39.0 opens their removal window with a
notice; §13.2 / §13.4 keep them parsing for the whole 1.x line.

**What makes these tests discriminating.** "It warns" is the easy half and it is
not the requirement. Requirement 2 says the check is driven by the **declared**
document and never by the merged view, and every one of the ten carries a
default in ``schemas/apcore-config.schema.json`` — three of them
(``observability.tracing.enabled``, ``observability.tracing.sampling_rate``,
``observability.metrics.enabled``) survive as far as this SDK's merged default
table, which :class:`TestAConfigDeclaringNoneOfThemIsSilent` reads back to prove
the point. An implementation driven off the merged view therefore warns for
*every* configuration ever loaded — the blanket warning §9.2.2 rejects, which
trains an operator to ignore the one notice that matters. Every silence
assertion below exists to catch that, and a test that only checked "it warns"
would pass an implementation in exactly that broken state.

The ACL half has its own trap. The notice lives in ``ACL.load`` because deleting
the ``audit`` block from ``acl-config.schema.json`` would produce no signal at
all — no implementation validates an ACL file against that schema and the loader
parses into an open mapping — but it is scoped to ``audit`` *deliberately*. It is
a deprecation notice, not unknown-key closure for ACL files, so
:meth:`TestACLAuditBlockNotice.test_another_unknown_root_key_stays_silent` pins
that every other unrecognised root key is still ignored exactly as before.
"""

from __future__ import annotations

import logging
import warnings
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL
from apcore.config import _DEPRECATED_INERT_KEYS, Config

#: The marker every §9.2.4 / §9.2.4.1 notice carries. Both halves are asserted
#: through it rather than through prose, so a reworded message does not turn a
#: silence assertion into a vacuous one.
_NOTICE_MARKER = "apcore#118"

#: A representative value per key, so each case declares the key with something
#: the schema would recognise rather than a placeholder.
_SAMPLE_VALUES: dict[str, Any] = {
    "acl.default_effect": "allow",
    "observability.tracing.enabled": True,
    "observability.tracing.sampling_rate": 0.25,
    "observability.tracing.exporter": "otlp",
    "observability.metrics.enabled": True,
    "observability.metrics.exporter": "prometheus",
    "logging.level": "debug",
    "logging.format": "text",
    "acl.audit.enabled": True,
    "acl.audit.include_denied": False,
    "acl.audit.log_level": "warn",
    "middleware.disabled": [],
    "extensions.auto_discover": True,
}

_ACL_RULES: list[dict[str, Any]] = [
    {
        "callers": ["*"],
        "targets": ["executor.email.send_email"],
        "effect": "allow",
        "description": "Allow everything to reach send_email",
    }
]


@pytest.fixture(autouse=True)
def _no_ambient_config(monkeypatch: pytest.MonkeyPatch) -> None:
    """§9.14 discovery must be decided by each case, not by the environment."""
    monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)


def _nest(dot_path: str, value: Any) -> dict[str, Any]:
    """``"a.b.c", 1`` -> ``{"a": {"b": {"c": 1}}}``."""
    head, _, tail = dot_path.partition(".")
    return {head: _nest(tail, value) if tail else value}


def _merge(into: dict[str, Any], other: dict[str, Any]) -> dict[str, Any]:
    for key, value in other.items():
        if isinstance(value, dict) and isinstance(into.get(key), dict):
            _merge(into[key], value)
        else:
            into[key] = value
    return into


def _write_config(directory: Path, *sections: dict[str, Any], name: str = "apcore.yaml") -> Path:
    """Write a minimal legacy-mode document carrying *sections*."""
    directory.mkdir(parents=True, exist_ok=True)
    document: dict[str, Any] = {"version": "1.0", "project": {"name": "inert-keys"}}
    for section in sections:
        _merge(document, section)
    target = directory / name
    target.write_text(yaml.safe_dump(document, sort_keys=False), encoding="utf-8")
    return target


def _load_capturing_warnings(path: Path, **kwargs: Any) -> tuple[Config, list[str]]:
    """Load *path* and return it alongside every warning message raised.

    ``"always"`` because these cases ask *whether the condition fired*, which
    removes de-duplication from the question entirely; the one case that asks
    about cadence uses ``"default"`` instead, and says so.

    Every warning is returned, not only the §9.2.4 one: the silence assertions
    have to be able to say that the ten keys are named **nowhere**, not merely
    that one particular notice was absent.
    """
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        config = Config.load(str(path), validate=False, **kwargs)
    return config, [str(warning.message) for warning in caught]


def _notices(messages: list[str]) -> list[str]:
    return [message for message in messages if _NOTICE_MARKER in message]


# ---------------------------------------------------------------------------
# §9.2.4 — the configuration half
# ---------------------------------------------------------------------------


class TestEachDeclaredKeyIsNamed:
    @pytest.mark.parametrize("key", _DEPRECATED_INERT_KEYS)
    def test_declaring_one_key_warns_and_names_it(self, key: str, tmp_path: Path) -> None:
        """Requirement 1: an operator who set the key finds out that it does nothing."""
        config_file = _write_config(tmp_path, _nest(key, _SAMPLE_VALUES[key]))

        config, messages = _load_capturing_warnings(config_file)

        notices = _notices(messages)
        assert len(notices) == 1, f"expected exactly one §9.2.4 notice, got {notices}"
        assert key in notices[0]
        # Requirement 3: the notice changes nothing. The key still parses and
        # still answers get() with the value the document declared.
        assert config.get(key) == _SAMPLE_VALUES[key]

    @pytest.mark.parametrize("key", _DEPRECATED_INERT_KEYS)
    def test_the_notice_names_that_key_and_no_other(self, key: str, tmp_path: Path) -> None:
        """The merged-view failure, caught key by key.

        A check driven off the merged view cannot name one key: the default
        table always carries the three ``observability.*`` booleans and the
        sampling rate, so its notice would list them alongside — or instead of —
        the one the document actually declared.
        """
        config_file = _write_config(tmp_path, _nest(key, _SAMPLE_VALUES[key]))

        _, messages = _load_capturing_warnings(config_file)

        notice = _notices(messages)[0]
        assert "declares 1 key(s)" in notice
        for other in _DEPRECATED_INERT_KEYS:
            if other == key or other.startswith(f"{key}."):
                continue
            # `observability.tracing.enabled` is a substring of nothing else in
            # the set, but `logging.level` is not a substring of `acl.audit.
            # log_level` either — the names are checked whole.
            assert other not in notice, f"{other!r} named for a document that declares only {key!r}"

    def test_two_declared_keys_are_both_named_and_counted(self, tmp_path: Path) -> None:
        config_file = _write_config(
            tmp_path,
            _nest("logging.level", "debug"),
            _nest("acl.audit.include_denied", False),
        )

        _, messages = _load_capturing_warnings(config_file)

        notice = _notices(messages)[0]
        assert "declares 2 key(s)" in notice
        assert "logging.level" in notice
        assert "acl.audit.include_denied" in notice
        assert "observability.tracing.enabled" not in notice

    def test_a_key_declared_only_through_the_environment_is_named(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """§9.1 "What 'declared' means": an env override counts as declaration.

        These keys are environment-overridable, which is one of the four
        properties that make them look alive, so configuring one entirely
        through the environment must reach the notice too.
        """
        config_file = _write_config(tmp_path)
        monkeypatch.setenv("APCORE_LOGGING_LEVEL", "debug")

        config, messages = _load_capturing_warnings(config_file)

        notice = _notices(messages)[0]
        assert "logging.level" in notice
        assert config.get("logging.level") == "debug"


class TestAConfigDeclaringNoneOfThemIsSilent:
    """The requirement, not a nicety — see this module's docstring."""

    def test_a_minimal_document_is_silent(self, tmp_path: Path) -> None:
        config_file = _write_config(tmp_path)

        config, messages = _load_capturing_warnings(config_file)

        assert _notices(messages) == []
        for key in _DEPRECATED_INERT_KEYS:
            assert all(key not in message for message in messages), f"{key!r} named for a document that omits it"
        # …and it is silent *despite* the merged view carrying values for four
        # of the ten. This is the state a merged-view implementation reads, and
        # the reason it would warn for every configuration ever loaded.
        assert config.get("observability.tracing.enabled") is False
        assert config.get("observability.tracing.sampling_rate") == 1.0
        assert config.get("observability.metrics.enabled") is False

    def test_a_document_declaring_only_live_keys_is_silent(self, tmp_path: Path) -> None:
        """The notice is keyed on the names, not on the sections holding them.

        ``acl.root`` sits beside ``acl.audit.*`` and ``obs.redaction.*`` beside
        ``observability.*``; a section-level check would sweep all four in.

        ``acl.default_effect`` used to stand here as the live key beside the
        deprecated ones. Spec v1.47.0 moved it INTO the table (§9.1.3's first
        application): an ACL's default effect is read from the ACL file, and
        this twin reaches nothing — measured, `allow` here yields `deny`. Its
        replacement is ``acl.root``, which is genuinely live and shares the same
        section, so the case still tests what it was written to test.
        """
        config_file = _write_config(
            tmp_path,
            {"acl": {"root": str(tmp_path / "acl")}},
            {"executor": {"default_timeout": 1000}},
            {"stream": {"max_merge_depth": 8}},
            {"obs": {"redaction": {"replacement": "***"}}},
        )

        _, messages = _load_capturing_warnings(config_file)

        assert _notices(messages) == []

    def test_a_config_built_from_defaults_alone_is_silent(self) -> None:
        """``from_defaults()`` declares nothing, so nothing can be reported."""
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            Config.from_defaults()

        assert _notices([str(warning.message) for warning in caught]) == []


class TestTheNoticeChangesNothing:
    """§9.2.4 requirement 3 — a deprecation notice, not a behaviour change."""

    def test_all_ten_still_load_and_validate_under_config_strict(self, tmp_path: Path) -> None:
        """The measurement that forced a deprecation window instead of a deletion.

        All ten are accepted today under ``_config.strict: true``, so removing a
        declared key outright would turn a currently-valid configuration into a
        rejected one. §13.2 sets a two-minor floor and §13.4 restates it for
        ``remove_field``, hence: keep parsing, warn, remove no earlier than 2.0.
        """
        sections = [_nest(key, value) for key, value in _SAMPLE_VALUES.items()]
        config_file = _write_config(tmp_path, *sections, {"_config": {"strict": True}})

        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            config = Config.load(str(config_file), validate=True)

        for key, value in _SAMPLE_VALUES.items():
            assert config.get(key) == value

    def test_the_notice_is_a_deprecation_warning(self, tmp_path: Path) -> None:
        """Category matters: it is what lets a host silence or escalate it."""
        config_file = _write_config(tmp_path, _nest("logging.format", "text"))

        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            Config.load(str(config_file), validate=False)

        notices = [warning for warning in caught if _NOTICE_MARKER in str(warning.message)]
        assert len(notices) == 1
        assert issubclass(notices[0].category, DeprecationWarning)

    def test_every_load_of_the_same_document_warns(self, tmp_path: Path) -> None:
        """Cadence is once per load, never once per process (§9.2.2 requirement 2).

        Run under the ``"default"`` filter action precisely because that is the
        one that de-duplicates on ``(message, category, lineno)``; under
        ``"always"`` there would be nothing to detect. The notice goes through
        ``_emit_per_load``, which allocates a throwaway registry, so the second
        load from this same call site still reports.
        """
        config_file = _write_config(tmp_path, _nest("logging.level", "debug"))

        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("default")
            Config.load(str(config_file), validate=False)
            Config.load(str(config_file), validate=False)

        assert len(_notices([str(warning.message) for warning in caught])) == 2


# ---------------------------------------------------------------------------
# §9.2.4.1 — the ACL-file half
# ---------------------------------------------------------------------------


def _write_acl(directory: Path, *, name: str = "policy.yaml", **extra: Any) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    document: dict[str, Any] = {"default_effect": "deny", "rules": _ACL_RULES, **extra}
    target = directory / name
    target.write_text(yaml.safe_dump(document, sort_keys=False), encoding="utf-8")
    return target


def _acl_notices(caplog: pytest.LogCaptureFixture) -> list[str]:
    return [record.getMessage() for record in caplog.records if _NOTICE_MARKER in record.getMessage()]


class TestACLAuditBlockNotice:
    def test_an_audit_block_no_longer_produces_a_deprecation_notice(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        """§9.2.4.1's notice is SUPERSEDED by §6.3.2 (spec v1.45.0).

        The block had never been read, so the notice was the only signal an
        operator got. It is now read — it has a delivery contract — and a key
        that has gained a consumer must stop being announced as going away,
        for the same reason §9.2.4 requirement 1 says the table is the whole
        list. This case is the one that fails against a loader that kept the
        notice.
        """
        acl_file = _write_acl(tmp_path, audit={"enabled": True, "include_denied": True, "log_level": "info"})

        with caplog.at_level(logging.WARNING):
            acl = ACL.load(str(acl_file))

        assert _acl_notices(caplog) == []
        assert len(acl.rules) == 1
        assert acl.default_effect == "deny"

    def test_an_acl_file_without_an_audit_block_is_silent(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        acl_file = _write_acl(tmp_path)

        with caplog.at_level(logging.WARNING):
            acl = ACL.load(str(acl_file))

        assert _acl_notices(caplog) == []
        assert len(acl.rules) == 1

    def test_another_unknown_root_key_stays_silent(self, tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
        """Scoped to ``audit`` deliberately: this is not unknown-key closure.

        An ACL file is parsed into an open mapping and every unrecognised root
        key is dropped in silence. That stays true — turning the deprecation
        notice into a general unknown-key warning would fire on forward-compatible
        and vendor-annotated policies that are perfectly valid today.
        """
        acl_file = _write_acl(tmp_path, telemetry={"enabled": True}, x_vendor_note="kept for the deploy tooling")

        with caplog.at_level(logging.WARNING):
            acl = ACL.load(str(acl_file))

        assert _acl_notices(caplog) == []
        assert [record.getMessage() for record in caplog.records if "telemetry" in record.getMessage()] == []
        assert len(acl.rules) == 1

    def test_an_audit_block_beside_another_unknown_key_still_validates_only_audit(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        """§6.3.2 requirement 8 — wiring one block is not unknown-key closure.

        The `audit` subtree is validated; every other unrecognised root key in
        an ACL file keeps being ignored, exactly as §9.2.4.1 scoped its own
        notice. A `telemetry:` block that loaded yesterday loads today.
        """
        acl_file = _write_acl(tmp_path, audit={"enabled": False}, telemetry={"enabled": True})

        with caplog.at_level(logging.WARNING):
            acl = ACL.load(str(acl_file))

        assert len(acl.rules) == 1
        assert [r.getMessage() for r in caplog.records if "telemetry" in r.getMessage()] == []


def _write_namespace_config(directory: Path, *sections: dict[str, Any]) -> Path:
    """Write the same document in §9.6 NAMESPACE-mode layout.

    The framework sections live under an ``apcore:`` root, which is the whole
    point: ``config.declared`` is then ``{"apcore": {...}}`` and a flat lookup
    finds nothing.
    """
    directory.mkdir(parents=True, exist_ok=True)
    inner: dict[str, Any] = {"version": "1.0", "project": {"name": "inert-keys"}}
    for section in sections:
        _merge(inner, section)
    target = directory / "apcore.yaml"
    target.write_text(yaml.safe_dump({"apcore": inner}, sort_keys=False), encoding="utf-8")
    return target


class TestNamespaceModeIsNotAnExemption:
    """§9.2.4 requirement 1 says "a loaded configuration document" — both layouts.

    §9.6 puts the framework sections under an ``apcore:`` root in namespace
    mode, so ``config.declared`` is ``{"apcore": {...}}`` there. The first
    implementation of this notice looked only at the flat spelling and was
    therefore **silent for every namespace-mode document**, however many
    deprecated keys it declared. apcore-typescript reaches through the layout in
    ``getDeclared`` and apcore-rust merges the ``apcore:`` members up to the top
    of ``user_namespaces``, so both were already correct — this was a one-SDK
    divergence on a requirement written for all three.
    """

    def test_namespace_mode_declaring_a_key_warns(self, tmp_path: Path) -> None:
        path = _write_namespace_config(tmp_path, {"logging": {"level": "debug"}})
        _, messages = _load_capturing_warnings(path)
        notices = _notices(messages)
        assert len(notices) == 1
        assert "logging.level" in notices[0]

    def test_namespace_mode_names_every_declared_key(self, tmp_path: Path) -> None:
        # `observability.tracing.sampling_rate` used to be the first half of
        # this pair. Spec v1.44.0 gave it a consumer (§10.1.1) and took it out
        # of §9.2.4's table, so it must NOT appear here any more — the second
        # assertion is what would catch a table that never shrank.
        path = _write_namespace_config(
            tmp_path,
            {"observability": {"metrics": {"exporter": "stdout"}}},
            {"acl": {"audit": {"enabled": False}}},
        )
        _, messages = _load_capturing_warnings(path)
        notices = _notices(messages)
        assert len(notices) == 1
        assert "observability.metrics.exporter" in notices[0]
        assert "acl.audit.enabled" in notices[0]

    def test_a_wired_tracing_key_is_not_named(self, tmp_path: Path) -> None:
        """The withdrawal cancelled by spec v1.44.0, pinned from the other side."""
        path = _write_namespace_config(
            tmp_path,
            {"observability": {"tracing": {"sampling_rate": 0.1, "enabled": True}}},
        )
        _, messages = _load_capturing_warnings(path)
        assert _notices(messages) == []

    def test_a_clean_namespace_mode_document_is_silent(self, tmp_path: Path) -> None:
        """The half that fails against a merged-view or always-on implementation."""
        path = _write_namespace_config(tmp_path)
        _, messages = _load_capturing_warnings(path)
        assert _notices(messages) == []
