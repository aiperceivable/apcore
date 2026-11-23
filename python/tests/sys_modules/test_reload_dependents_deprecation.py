"""D-121 — `reload_dependents` is deprecated for removal.

Declared in all three SDKs' input schemas and read by none: a spec MUST ("also
reload modules that depend on matched modules") that no implementation
satisfied. That is the §9.1.3 "declared surface reaches no mechanism" shape the
spec forbids for configuration keys, here applied to a module INPUT FIELD, and
three independent implementations skipping it is the evidence the maintainer
decision rests on — deprecate now, remove at 2.0, do NOT implement.

The field is deprecated rather than removed today because the input schema sets
``additionalProperties: false``: at 2.0 the same call stops being a silent no-op
and becomes a VALIDATION ERROR, so a caller passing it needs a release in which
they are told.
"""

from __future__ import annotations

import logging

import pytest

from apcore.sys_modules.control import ReloadModule


def _warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
    return [r.message for r in caplog.records if "reload_dependents" in r.getMessage()]


def _module() -> ReloadModule:
    return ReloadModule(None, None)  # type: ignore[arg-type]


class TestReloadDependentsDeprecation:
    def test_passing_it_warns(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING):
            _module()._warn_reload_dependents({"reload_dependents": True, "reason": "x"})
        assert len(_warnings(caplog)) == 1

    def test_the_warning_names_the_replacement_and_the_removal(self, caplog: pytest.LogCaptureFixture) -> None:
        """A deprecation notice that does not say what to do instead, or when the
        field stops being ignored, leaves the caller to discover both."""
        with caplog.at_level(logging.WARNING):
            _module()._warn_reload_dependents({"reload_dependents": True, "reason": "x"})
        message = _warnings(caplog)[0]
        assert "path_filter" in message
        assert "2.0" in message
        assert "validation error" in message

    def test_it_warns_once_per_instance(self, caplog: pytest.LogCaptureFixture) -> None:
        """``reload`` is called by hot-reload loops and watchers, so an advisory
        whose volume is proportional to traffic is one operators learn to filter
        out — the cadence D-89 settled."""
        module = _module()
        with caplog.at_level(logging.WARNING):
            for _ in range(5):
                module._warn_reload_dependents({"reload_dependents": True, "reason": "x"})
        assert len(_warnings(caplog)) == 1

    def test_the_cadence_is_per_instance_not_per_process(self, caplog: pytest.LogCaptureFixture) -> None:
        """A process-wide one-shot tells the FIRST caller and leaves every later
        one to discover it in production — the reason D-90's notice is per token
        and D-89's dedupe is per registry instance."""
        with caplog.at_level(logging.WARNING):
            _module()._warn_reload_dependents({"reload_dependents": True, "reason": "x"})
            _module()._warn_reload_dependents({"reload_dependents": True, "reason": "x"})
        assert len(_warnings(caplog)) == 2

    @pytest.mark.parametrize("inputs", [{"reason": "x"}, {"reload_dependents": False, "reason": "x"}])
    def test_control_omitting_it_or_passing_false_says_nothing(
        self, inputs: dict[str, object], caplog: pytest.LogCaptureFixture
    ) -> None:
        """Without this, "it warns" is also satisfied by warning on every reload,
        which is noise for the ordinary call the method exists for."""
        with caplog.at_level(logging.WARNING):
            _module()._warn_reload_dependents(inputs)
        assert _warnings(caplog) == []

    def test_the_schema_marks_it_deprecated_and_says_what_replaces_it(self) -> None:
        """The notice has to be readable without triggering it: a host reading
        the input schema — which is what `describe`/`get_definition` hand an
        agent — must see the deprecation without having to call with the field.
        """
        field = ReloadModule.input_schema["properties"]["reload_dependents"]
        assert field.get("deprecated") is True
        assert "path_filter" in field["description"]
        assert "2.0" in field["description"]
