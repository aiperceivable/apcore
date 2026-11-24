"""PROTOCOL_SPEC §10.6.1 requirements 2, 4 and 5 — where `regex_patterns` is
compiled, what it is allowed to be matched against, and how long the diagnostic
for a broken entry may stay suppressed.

Every case here asserts the half that a passing implementation could otherwise
skip, because each of these three requirements has a failure mode that looks
exactly like success from the outside:

* Requirement 2 — a stringifying implementation redacts *more*, so it passes any
  test that only checks a secret was caught. The discriminating assertion is
  that an ordinary number was **left alone**.
* Requirement 4 — an implementation that swallows `re.error` and carries on
  produces a redaction rule that redacts nothing, which is indistinguishable
  from one that compiles and matches nothing. The discriminating assertion is
  that a diagnostic was emitted at all.
* Requirement 5 — a process-global "already reported" set makes the FIRST
  configuration warn, so a single-configuration test passes. The discriminating
  assertion needs a **second** configuration.
"""

from __future__ import annotations

import logging
import re

import pytest

from apcore.observability.context_logger import RedactionConfig, _apply_redaction_config
from apcore.utils.redaction import compile_value_regexes, redact_sensitive

_BAD = "[invalid"


class TestStringValuesOnly:
    """§10.6.1 requirement 2 — the value rule never sees a non-string."""

    @pytest.fixture
    def config(self) -> RedactionConfig:
        return RedactionConfig(sensitive_keys=[], regex_patterns=[r"[0-9]+", "true", "a"])

    def test_a_number_is_not_tested_against_the_patterns(self, config: RedactionConfig) -> None:
        # `[0-9]+` matches "42", so an implementation that stringifies redacts
        # this field. The observable damage of that is not a leak but its
        # opposite: ordinary numeric telemetry disappearing from logs.
        assert _apply_redaction_config({"amount": 42}, config) == {"amount": 42}

    def test_a_boolean_is_not_tested_against_the_patterns(self, config: RedactionConfig) -> None:
        # Python renders `True`, TypeScript and Rust render `true`, and the
        # pattern here is `true` — so a stringifying Python matches nothing
        # while a stringifying TypeScript matches. One value, two answers, which
        # is why requirement 2 forbids the conversion rather than defining it.
        assert _apply_redaction_config({"flag": True}, config) == {"flag": True}

    def test_a_container_is_not_tested_against_the_patterns(self, config: RedactionConfig) -> None:
        # `str({"a": 1})` is `"{'a': 1}"` in Python and `"[object Object]"` in
        # TypeScript; `a` matches the first and not the second.
        payload = {"mapping": {"a": 1}, "listing": [1, 2]}
        assert _apply_redaction_config(dict(payload), config) == payload

    def test_a_string_is_still_tested(self, config: RedactionConfig) -> None:
        """The other half: requirement 2 narrows the rule, it does not remove it."""
        assert _apply_redaction_config({"text": "order 42"}, config) == {"text": config.replacement}

    def test_a_string_inside_a_container_is_still_reached(self) -> None:
        """Containers are descended into, so nesting is not an exemption."""
        got = redact_sensitive(
            {"nested": {"key": "sk-secret", "count": 7}, "items": ["sk-secret", 7]},
            {},
            sensitive_keys=[],
            regex_patterns=["sk-"],
        )
        assert got == {
            "nested": {"key": "***REDACTED***", "count": 7},
            "items": ["***REDACTED***", 7],
        }

    def test_both_surfaces_agree_on_the_same_value(self) -> None:
        """§10.6 requires the rule on log emission AND executor capture alike.

        They disagreed: `_apply_redaction_config` stringified and
        `redact_sensitive` did not, so `amount: 42` came back redacted from one
        and untouched from the other **inside one SDK**.
        """
        payload = {"amount": 42, "text": "order 42"}
        via_logging = _apply_redaction_config(
            dict(payload), RedactionConfig(sensitive_keys=[], regex_patterns=[r"[0-9]+"])
        )
        via_capture = redact_sensitive(dict(payload), {}, sensitive_keys=[], regex_patterns=[r"[0-9]+"])
        assert via_logging == via_capture


class TestCompilationHappensOnce:
    """§10.6.1 requirement 5 — compile at the configuration read."""

    def test_the_compiled_list_is_available_after_construction(self) -> None:
        config = RedactionConfig(regex_patterns=[r"^Bearer\s", "sk-"])
        assert [p.pattern for p in config.compiled_regex_patterns] == [r"^Bearer\s", "sk-"]
        assert all(p.flags & re.IGNORECASE for p in config.compiled_regex_patterns)

    def test_an_empty_entry_is_dropped_rather_than_matching_everything(self) -> None:
        assert RedactionConfig(regex_patterns=["", "sk-"]).compiled_regex_patterns == [re.compile("sk-", re.IGNORECASE)]

    def test_an_already_compiled_pattern_passes_through(self) -> None:
        """So a caller holding a config recompiles nothing per record."""
        compiled = re.compile("sk-", re.IGNORECASE)
        assert compile_value_regexes([compiled])[0] == [compiled]


class TestUncompilablePatternIsReported:
    """§10.6.1 requirement 4, and requirement 5's bound on suppressing it."""

    def test_a_broken_pattern_is_named_at_construction(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.utils.redaction"):
            config = RedactionConfig(regex_patterns=[_BAD])
        assert _BAD in caplog.text, (
            "an entry that does not compile MUST produce a diagnostic naming it — a rule that "
            f"redacts nothing looks exactly like one that matches nothing. Captured:\n{caplog.text}"
        )
        assert config.compiled_regex_patterns == []
        assert [p for p, _ in config.invalid_regex_patterns] == [_BAD]

    def test_the_diagnostic_carries_the_engine_message(self) -> None:
        """`validate_config()` reports these, so the reason has to survive."""
        _, invalid = compile_value_regexes([_BAD])
        assert invalid and "character set" in invalid[0][1]

    def test_a_second_configuration_is_told_too(self, caplog: pytest.LogCaptureFixture) -> None:
        """The requirement-5 bound, and the one a single-config test cannot see.

        The suppression used to be a module-level `set` that was never cleared,
        so the second deployment to load the same broken pattern got silence —
        the reload case and the multi-tenant case, which are exactly the two
        where an operator most needs telling.
        """
        with caplog.at_level(logging.WARNING, logger="apcore.utils.redaction"):
            RedactionConfig(regex_patterns=[_BAD])
            caplog.clear()
            RedactionConfig(regex_patterns=[_BAD])
        assert _BAD in caplog.text, (
            "the second configuration carrying the same broken pattern must be told as well; "
            f"suppression MUST NOT outlive the configuration it was raised for. Captured:\n{caplog.text}"
        )

    def test_one_configuration_reports_each_bad_entry_once(self, caplog: pytest.LogCaptureFixture) -> None:
        """The other half: it is still one line per entry, not one per record."""
        with caplog.at_level(logging.WARNING, logger="apcore.utils.redaction"):
            config = RedactionConfig(sensitive_keys=[], regex_patterns=[_BAD])
            for _ in range(5):
                _apply_redaction_config({"a": "x", "b": "y"}, config)
        assert caplog.text.count(_BAD) == 1
