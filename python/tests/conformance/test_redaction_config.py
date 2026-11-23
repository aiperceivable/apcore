"""Drive `redaction_config.json` — `obs.redaction.*` matching rules (#43 §5, D-54).

observability.md § Redaction configuration says redaction "MUST apply both at
log emission (in `ContextLogger`) and at the executor's input/output capture
point", so this driver replays every fixture case through **both** public entry
points:

* :class:`~apcore.observability.ContextLogger` — the log-emission path; the
  fixture input becomes the ``extra`` dict of one JSON log record.
* :func:`apcore.utils.redaction.redact_sensitive` — Algorithm A13, the
  input/output capture path used by ``builtin_steps``.

Driving only one of them would leave half the mandated surface unverified —
and that is exactly how the correlation-field MUST came to be violated on both:
the exemption lived in a flat helper neither path called.

Two shapes of case
------------------

*Rule cases* carry a ``redaction_config`` block and pin behaviour GIVEN a set of
rules. *Config-key cases* name their dot-paths directly (``config``, or
``config_canonical`` plus a per-SDK ``legacy_key_by_sdk``) and pin WHICH key path
an implementation consults to obtain those rules — the fixture's
``driver_contract.which_key_is_read_is_part_of_the_contract``. The second shape
exists because behaviour-given-a-config cannot catch an implementation that
reads the wrong key: apcore-rust read only the legacy ``observability.redaction.*``
path, so an operator writing the documented ``obs.redaction.sensitive_keys`` had
their configuration silently discarded (apcore-rust#32). Config-key cases are
therefore built by writing the exact dot-path the case names into a real
:class:`~apcore.config.Config` and reading it back through
:meth:`RedactionConfig.from_config` — never by constructing the RedactionConfig
directly, which would skip the very step under test.

The discriminating payload matters as much as the key path: ``username`` matches
NO entry of the canonical default list while ``password`` and ``_secret_token``
both do, so the expected map can distinguish "the override was read and replaced
the defaults" from "the defaults are in force". A payload whose keys all match a
default anyway stays green under both behaviours, which is why this defect
survived in every SDK.
"""

from __future__ import annotations

import io
import json
import warnings
from typing import Any

import pytest

from apcore.config import _DEFAULT_OBS_REDACTION_SENSITIVE_KEYS, Config
from apcore.observability import ContextLogger, RedactionConfig
from apcore.utils.redaction import redact_sensitive

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("redaction_config.json")
CASES: list[dict[str, Any]] = FIXTURE["test_cases"]

#: This SDK's key in the fixture's ``legacy_key_by_sdk`` maps.
SDK = "python"

#: Keys that mark a case as pinning WHICH dot-path is read rather than what the
#: rules do. ``config`` is a complete canonical map; ``config_canonical`` is the
#: canonical half of a case that also has a legacy half; ``legacy_key_by_sdk``
#: marks a case whose legacy half is per-SDK history.
_CONFIG_KEY_MARKERS = ("config", "config_canonical", "legacy_key_by_sdk")

#: Cases that pin behaviour given a rule set (``redaction_config`` block).
RULE_CASES: list[dict[str, Any]] = [c for c in CASES if "redaction_config" in c]

#: Cases that pin which Config dot-path the rules are read from.
CONFIG_KEY_CASES: list[dict[str, Any]] = [c for c in CASES if any(marker in c for marker in _CONFIG_KEY_MARKERS)]

#: The replacement token the config-key cases expect. None of them sets
#: ``obs.redaction.replacement``, so ``RedactionConfig.from_config`` uses the
#: spec default and every redacted field comes back as exactly this string.
_REPLACEMENT = "***REDACTED***"

#: ``expected`` entries that describe the deprecation warning rather than a
#: redacted field, and so must not be compared against the payload.
_WARNING_EXPECTATIONS = frozenset({"deprecation_warning_emitted", "deprecation_warning_is_one_shot"})

# observability.md: "Implementations MUST NOT redact trace_id, caller_id,
# module_id, or span_id" (the fixture adds target_id). The exemption used to
# live only in _apply_redaction_config, a flat helper that neither mandated path
# calls, so both recursive engines violated the MUST — including on the NAME
# rule. PROTECTED_LOG_FIELDS now sits in apcore.utils.redaction and both
# _redact_secrets_recursive and _redact_by_keys_and_regex consult it, ahead of
# the name rule AND the value regex. No longer an xfail; it is parametrized with
# the rest.
_CORRELATION_CASE = "correlation_fields_never_redacted"


def _build_config(case: dict[str, Any]) -> RedactionConfig:
    """Turn a fixture ``redaction_config`` block into a RedactionConfig."""
    spec = case["redaction_config"]
    keys = spec.get("sensitive_keys")
    if spec.get("use_defaults") or keys is None:
        # `use_defaults: true` / `sensitive_keys: null` means "the canonical
        # default list", not "no key matching".
        keys = _DEFAULT_OBS_REDACTION_SENSITIVE_KEYS
    # `regex_patterns` is a CONSTRUCTOR argument, not an attribute to assign
    # afterwards: RedactionConfig compiles it once in `__post_init__`
    # (PROTOCOL_SPEC 10.6.1 requirement 5), so a later assignment leaves the
    # compiled list behind and the value rule silently matches nothing. This
    # driver did assign it, and every regex case here went green anyway while
    # the old code recompiled per record — which is the same shape as the
    # defect the fixture exists to catch, in the harness.
    return RedactionConfig(
        sensitive_keys=list(keys),
        regex_patterns=list(spec.get("regex_patterns") or []),
        replacement=spec["replacement"],
    )


def _expected_fields(case: dict[str, Any]) -> dict[str, Any]:
    """Fixture expectations minus annotations and warning expectations.

    Drops ``_``-prefixed annotations (e.g. ``_note``) and the
    ``deprecation_warning_*`` keys, which describe the warning rather than a
    field of the redacted payload and are asserted by
    :func:`test_deprecation_warning_expectations`.
    """
    return {k: v for k, v in case["expected"].items() if not k.startswith("_") and k not in _WARNING_EXPECTATIONS}


def _assert_matches(case: dict[str, Any], got: dict[str, Any], via: str) -> None:
    expected = _expected_fields(case)
    mismatches = {k: (got.get(k), v) for k, v in expected.items() if got.get(k) != v}
    assert not mismatches, (
        f"[{case['id']}] via {via}: redaction result differs from the canonical fixture "
        f"(field: got != expected):\n  "
        + "\n  ".join(f"{k}: {g!r} != {e!r}" for k, (g, e) in sorted(mismatches.items()))
    )
    # A field the fixture does not mention must still survive the round trip;
    # dropping keys would make the comparison above vacuous for that field.
    missing = sorted(set(case["input"]) - set(got))
    assert not missing, f"[{case['id']}] via {via}: redaction dropped input fields {missing}"


def _emit_through_context_logger(config: RedactionConfig, payload: dict[str, Any]) -> dict[str, Any]:
    """Log *payload* as `extra` and return the redacted `extra` from the record."""
    sink = io.StringIO()
    logger = ContextLogger(
        name="conformance.redaction",
        output_format="json",
        level="trace",
        output=sink,
        redaction_config=config,
    )
    logger.info("redaction conformance", extra=dict(payload))
    lines = [line for line in sink.getvalue().splitlines() if line.strip()]
    assert len(lines) == 1, f"expected exactly one log record, got {len(lines)}"
    return json.loads(lines[0])["extra"]  # type: ignore[no-any-return]


def _run_redact_sensitive(
    config: RedactionConfig, payload: dict[str, Any], schema: dict[str, Any] | None = None
) -> dict[str, Any]:
    """Executor capture path: Algorithm A13 `redact_sensitive`.

    ``schema`` is the case's ``schema`` (driver_contract.schema_marked_fields):
    the ``x-sensitive`` rule walks it alongside the configured rules. Cases
    without one pass an empty schema and exercise the configured rules only.
    """
    return redact_sensitive(
        dict(payload),
        schema or {},
        sensitive_keys=config.sensitive_keys,
        regex_patterns=config.regex_patterns,
        replacement=config.replacement,
    )


class TestRedactionViaContextLogger:
    """Log-emission path: ContextLogger applies the configured rules to `extra`."""

    @pytest.mark.parametrize("case", RULE_CASES, ids=lambda c: c["id"])
    def test_case(self, case: dict[str, Any]) -> None:
        if "schema" in case:
            # driver_contract.schema_marked_fields names the capture-path entry
            # point: a log record's `extra` carries no schema, so the
            # `x-sensitive` rule is asserted through redact_sensitive only.
            pytest.skip(f"[{case['id']}] schema-marked case; driven through redact_sensitive")
        got = _emit_through_context_logger(_build_config(case), case["input"])
        _assert_matches(case, got, via="ContextLogger._emit")


class TestRedactionViaRedactSensitive:
    """Executor capture path: Algorithm A13 `redact_sensitive`."""

    @pytest.mark.parametrize("case", RULE_CASES, ids=lambda c: c["id"])
    def test_case(self, case: dict[str, Any]) -> None:
        got = _run_redact_sensitive(_build_config(case), case["input"], case.get("schema"))
        _assert_matches(case, got, via="redact_sensitive")


# ---------------------------------------------------------------------------
# Config-key cases: WHICH dot-path the rules are read from
# ---------------------------------------------------------------------------
#
# driver_contract.legacy_spelling_is_per_sdk_history: the CANONICAL key
# `obs.redaction.*` is the cross-language contract and every SDK MUST read it.
# The LEGACY spelling is not — each SDK carries whatever it shipped before D-53
# (apcore-rust `observability.redaction.sensitive_keys`, apcore-typescript
# `observability.redaction.field_patterns`) and apcore-python carries nothing,
# having never had an `observability.*` namespace at all.
#
# A null `legacy_key_by_sdk.python` therefore means the case DOES NOT APPLY, and
# the driver skips it with the fixture's own reason. It is deliberately not an
# xfail: xfail asserts "Python is deficient and should one day pass", which is
# the opposite of the conclusion. Python MUST NOT gain the fallback — it would
# create a deprecated surface for zero users, and a stale
# `observability.redaction.sensitive_keys: []` would flip from "ignored,
# defaults apply" to "key-based redaction disabled", a security regression.
#
# Keying the skip off the fixture rather than off a case id makes it
# self-correcting: give Python a legacy spelling in `legacy_key_by_sdk` and
# these tests start running against it on the next pass, with no driver edit.


def _legacy_key(case: dict[str, Any]) -> str | None:
    """This SDK's pre-D-53 spelling for *case*, or None if it never had one."""
    return (case.get("legacy_key_by_sdk") or {}).get(SDK)


def _canonical_entries(case: dict[str, Any]) -> dict[str, Any]:
    """The canonical dot-path -> value map the case declares, if any.

    ``config`` is a case that is canonical-only; ``config_canonical`` is the
    canonical half of a case that also carries a legacy half.
    """
    return dict(case.get("config") or case.get("config_canonical") or {})


def _legacy_value(case: dict[str, Any]) -> list[str]:
    """The value written at the legacy spelling.

    ``canonical_config_key_wins_over_legacy`` states this explicitly as
    ``legacy_value`` — a DIFFERENT list from the canonical one, which is what
    lets the case prove which of the two won. The legacy-only case does not
    state it, because there the legacy key carries the case's whole rule set:
    derive it from the case's own expectations, i.e. every field the case
    expects to come back redacted. Deriving beats hard-coding ``["username"]``
    here — the payload's discriminating field is the fixture's choice to make.
    """
    if "legacy_value" in case:
        return list(case["legacy_value"])
    return sorted(k for k, v in _expected_fields(case).items() if v == _REPLACEMENT)


def _skip_if_case_does_not_apply(case: dict[str, Any]) -> None:
    """Skip a legacy-only case when this SDK has no legacy spelling.

    A case that also declares a canonical half is NOT skipped: the canonical
    assertion is the cross-language contract and stays meaningful on its own,
    so it is asserted even when the legacy half cannot be set up.
    """
    if _legacy_key(case) is not None or _canonical_entries(case):
        return
    reason = case.get("skip_when_legacy_key_is_null") or ("the fixture states no legacy spelling for this SDK")
    pytest.skip(f"[{case['id']}] legacy_key_by_sdk.{SDK} is null — {reason}")


def _config_from_case(case: dict[str, Any]) -> Config:
    """Write the case's dot-paths verbatim into a real Config.

    Deliberately writes the exact strings the fixture names rather than
    normalising them: reading the wrong key path must fail here, which it
    cannot do if the driver silently maps both spellings onto one setter.

    The legacy half is written only when this SDK has a legacy spelling, so for
    apcore-python a case with both halves degrades to its canonical half.
    """
    entries = _canonical_entries(case)
    legacy_key = _legacy_key(case)
    if legacy_key is not None:
        entries[legacy_key] = _legacy_value(case)
    assert entries, (
        f"[{case['id']}] nothing to configure: the case declares no canonical "
        f"map and no legacy spelling for {SDK}. _skip_if_case_does_not_apply "
        f"should have skipped it before reaching here."
    )
    config = Config()
    for dot_path, value in entries.items():
        config.set(dot_path, value)
    return config


def _redaction_from_case(case: dict[str, Any]) -> RedactionConfig:
    return RedactionConfig.from_config(_config_from_case(case))


def _via(case: dict[str, Any], sink: str) -> str:
    """Describe the path under test, flagging a canonical-only degradation."""
    scope = "canonical half only" if _legacy_key(case) is None and case.get("legacy_key_by_sdk") else "full"
    return f"Config -> RedactionConfig.from_config -> {sink} [{scope}]"


@pytest.mark.parametrize("case", CONFIG_KEY_CASES, ids=lambda c: c["id"])
def test_config_key_via_context_logger(case: dict[str, Any]) -> None:
    """The rules reached the log-emission path from the dot-path the case names."""
    _skip_if_case_does_not_apply(case)
    got = _emit_through_context_logger(_redaction_from_case(case), case["input"])
    _assert_matches(case, got, via=_via(case, "ContextLogger"))


@pytest.mark.parametrize("case", CONFIG_KEY_CASES, ids=lambda c: c["id"])
def test_config_key_via_redact_sensitive(case: dict[str, Any]) -> None:
    """The rules reached the executor capture path from the same dot-path."""
    _skip_if_case_does_not_apply(case)
    got = _run_redact_sensitive(_redaction_from_case(case), case["input"])
    _assert_matches(case, got, via=_via(case, "redact_sensitive"))


@pytest.mark.parametrize("case", CONFIG_KEY_CASES, ids=lambda c: c["id"])
def test_deprecation_warning_expectations(case: dict[str, Any]) -> None:
    """A legacy key read MUST warn once; a canonical key read MUST NOT warn."""
    _skip_if_case_does_not_apply(case)
    expected = case["expected"]
    if not (_WARNING_EXPECTATIONS & set(expected)):
        pytest.skip(f"[{case['id']}] states no deprecation-warning expectation")
    if _legacy_key(case) is None and expected.get("deprecation_warning_emitted"):
        # The case expects a warning that only a legacy-key read can produce,
        # and this SDK has no legacy key to read. Asserting it against the
        # canonical half would demand a warning the spec does not want here.
        pytest.skip(
            f"[{case['id']}] expects a legacy-key deprecation warning, but "
            f"legacy_key_by_sdk.{SDK} is null so no legacy key is read"
        )

    with warnings.catch_warnings(record=True) as first:
        warnings.simplefilter("always")
        _redaction_from_case(case)
    with warnings.catch_warnings(record=True) as second:
        warnings.simplefilter("always")
        _redaction_from_case(case)

    def _deprecations(caught: Any) -> list[str]:
        return [str(w.message) for w in caught if issubclass(w.category, DeprecationWarning)]

    first_warnings = _deprecations(first)
    if "deprecation_warning_emitted" in expected:
        emitted = bool(first_warnings)
        assert emitted is expected["deprecation_warning_emitted"], (
            f"[{case['id']}] deprecation_warning_emitted: got {emitted}, "
            f"expected {expected['deprecation_warning_emitted']} "
            f"(DeprecationWarnings seen: {first_warnings})"
        )

    if expected.get("deprecation_warning_is_one_shot"):
        repeat = _deprecations(second)
        assert not repeat, (
            f"[{case['id']}] the deprecation warning MUST be one-shot, but a second "
            f"RedactionConfig.from_config re-emitted: {repeat}"
        )


def test_python_has_no_legacy_redaction_spelling() -> None:
    """Pin the decision the skips rest on, so it cannot drift silently.

    The skips above are only correct while apcore-python genuinely reads no
    `observability.redaction.*` key. If someone adds the fallback, the skip
    would quietly keep firing and the new code would go untested — so assert
    the SDK's behaviour directly rather than inferring it from the fixture.
    """
    for case in CONFIG_KEY_CASES:
        spelling = (case.get("legacy_key_by_sdk") or {}).get(SDK)
        assert spelling is None, (
            f"[{case['id']}] the fixture now gives {SDK} the legacy spelling "
            f"{spelling!r}. Revisit the skip rationale in this module before "
            f"acting on it — the decision recorded there was that Python MUST "
            f"NOT gain the fallback."
        )

    config = Config()
    config.set("observability.redaction.sensitive_keys", ["username"])
    config.set("observability.redaction.field_patterns", ["username"])
    resolved = RedactionConfig.from_config(config)
    assert "username" not in resolved.sensitive_keys, (
        "apcore-python read an `observability.redaction.*` key. That fallback was "
        "deliberately NOT adopted (it would create a deprecated surface for zero "
        "users, and a stale `sensitive_keys: []` would silently disable key-based "
        "redaction). If this is now intended, update `legacy_key_by_sdk.python` in "
        "redaction_config.json and the rationale in this module."
    )


def test_fixture_case_ids_are_covered() -> None:
    """A new fixture case must fail here rather than land undriven."""
    driven = {
        "regex_pattern_value_match",
        "sensitive_keys_substring_case_insensitive",
        "default_sensitive_keys_cover_common_terms",
        _CORRELATION_CASE,
        "canonical_config_key_is_read",
        "legacy_config_key_is_honoured_with_a_deprecation_warning",
        "canonical_config_key_wins_over_legacy",
        # PROTOCOL_SPEC 9.2.3 / 10.6.1 — the pattern dialect (#117).
        "sensitive_keys_glob_case_fold_applies_to_the_pattern_too",
        "sensitive_keys_bracket_is_a_literal_never_a_character_class",
        "sensitive_keys_glob_entry_is_anchored_to_the_whole_name",
        "regex_patterns_are_an_unanchored_search",
        # PROTOCOL_SPEC 10.6.1 requirement 2 — string values only (v1.40.0).
        "regex_patterns_do_not_test_non_string_values",
        "regex_patterns_still_reach_a_string_inside_a_container",
        # PROTOCOL_SPEC 10.6 / A13 — x-sensitive inside combinator branches (D-152).
        "x_sensitive_inside_an_any_of_branch_is_redacted",
        "x_sensitive_in_any_branch_redacts_the_value_whichever_branch_it_matches",
        "x_sensitive_in_an_all_of_branch_is_redacted",
        "properties_declared_inside_a_branch_are_walked",
        "a_combinator_without_a_marker_stays_visible",
        "array_items_marked_inside_a_branch_are_redacted",
    }
    ids = {c["id"] for c in CASES}
    assert ids == driven, (
        f"redaction_config.json cases without a driver: {sorted(ids - driven)}; "
        f"drivers with no matching case: {sorted(driven - ids)}"
    )
    # Every case must land in exactly one of the two shapes; a third shape would
    # otherwise be counted as covered above while running through no driver.
    unclassified = sorted(ids - {c["id"] for c in RULE_CASES} - {c["id"] for c in CONFIG_KEY_CASES})
    assert not unclassified, (
        f"redaction_config.json cases carrying neither a `redaction_config` block nor "
        f"any of {_CONFIG_KEY_MARKERS}, so no driver replays them: {unclassified}"
    )
