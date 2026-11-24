"""Drive `pipeline_failfast_config.json` — pipeline config fail-fast (Issue #33).

middleware-system.md § Configuration safety makes two MUSTs:

* ``PIPELINE_CONFIGURATION_ERROR`` at parse time when a key of the
  ``pipeline.configure`` MAP references an unknown step,
* ``PIPELINE_DEPENDENCY_ERROR`` at strategy construction when a ``requires:``
  is unsatisfied — never deferred to the first ``call()``.

Two things this file used to get wrong, both now fixed in the canonical
fixture rather than worked around here:

* ``pipeline.configure`` was driven as a LIST of ``{name, ...}`` entries. It is
  an object map keyed by step name — what ``$defs/PipelineConfig`` declares and
  what all three SDKs parse.
* A third case drove ``pipeline.step_middleware:``, a config section no SDK has
  ever parsed. It was removed from the fixture, not xfailed here.

Assert the WIRE CODE, not the class name: all three SDKs name the class
``ConfigurationError`` while they emitted three different codes, so a
class-name assertion passed everywhere and proved nothing.

Since aiperceivable/apcore#89 the fixture also pins the SIZE of the
configurable set: ``pipeline.configure`` accepts exactly ``match_modules``,
``ignore_errors``, ``pure``, ``timeout_ms``
(``$defs/ConfigurableStepFields`` / protocol-spec §5.16.1).
``requires`` / ``provides`` are NOT among them — they are the step's own
capability contract, and configuration able to rewrite it disables the
``PIPELINE_DEPENDENCY_ERROR`` MUST this same fixture pins two cases above.
The accept case reads all four fields back OFF THE BUILT STEP: ``raises:
false`` alone is also satisfied by an implementation that takes the keys and
applies none of them.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.pipeline import (
    BaseStep,
    ConfigurationError,
    ExecutionStrategy,
    PipelineContext,
    PipelineDependencyError,
    StepResult,
)
from apcore.pipeline_config import (
    _step_type_registry,
    build_strategy_from_config,
    register_step_type,
)

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("pipeline_failfast_config.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}


class _Step(BaseStep):
    """Inert step used to assemble the fixture's `strategy.steps` declarations."""

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


class _StubRegistry:
    """`build_standard_strategy` only needs an object with these two methods."""

    def get(self, *args: Any, **kwargs: Any) -> None:
        return None

    def discover(self, *args: Any, **kwargs: Any) -> list[Any]:
        return []


def _build_from_yaml(case: dict[str, Any]) -> Any:
    pipeline_section = case["input"]["yaml"]["pipeline"]
    return build_strategy_from_config(pipeline_section, registry=_StubRegistry())


def _strategy_from_fixture(case: dict[str, Any]) -> ExecutionStrategy:
    spec = case["input"]["strategy"]
    steps = [
        _Step(
            step["name"],
            requires=tuple(step.get("requires", ())),
            provides=tuple(step.get("provides", ())),
        )
        for step in spec["steps"]
    ]
    return ExecutionStrategy(spec["name"], steps)


def _observe_parse_failure(case: dict[str, Any]) -> tuple[Any, str | None, ConfigurationError | None]:
    """Build the fixture's YAML and OBSERVE which phase raised.

    ``driver_contract.parse_time``: a driver that only asserts "it raised
    eventually" cannot tell parse time from first ``call()``. Building the
    strategy — and nothing else — is what separates them: if the build returns a
    strategy, the failure was deferred.
    """
    strategy: Any = None
    raised_at: str | None = None
    error: ConfigurationError | None = None
    try:
        strategy = _build_from_yaml(case)
    except ConfigurationError as exc:
        raised_at, error = "parse_time", exc
    return strategy, raised_at, error


def _assert_parse_time_rejection(case: dict[str, Any]) -> ConfigurationError:
    """Assert every key of a rejection case's ``expected`` block."""
    strategy, raised_at, error = _observe_parse_failure(case)

    assert raised_at == case["expected"]["raised_at"], (
        f"[{case['id']}] must be rejected at {case['expected']['raised_at']!r}, " f"observed {raised_at!r}"
    )
    assert (error is not None) is case["expected"][
        "raises"
    ], f"[{case['id']}] expected raises={case['expected']['raises']}, got {error!r}"
    # `deferred_to_first_call`: a build that RETURNS a strategy has deferred the
    # failure to whenever that strategy is first called.
    assert (strategy is not None) is case["expected"]["deferred_to_first_call"], (
        f"[{case['id']}] build_strategy_from_config returned a strategy — the "
        f"misconfiguration would only surface at the first call()"
    )
    assert error is not None
    _assert_message_contains(case, str(error))
    expected_code = case["expected"]["error_code"]
    assert error.code == expected_code, (
        f"[{case['id']}] the WIRE CODE is the contract, not the class name: "
        f"got {error.code!r}, expected {expected_code!r}"
    )
    return error


def _assert_message_contains(case: dict[str, Any], message: str) -> None:
    needles = case["expected"]["error_message_contains"]
    if isinstance(needles, str):
        needles = [needles]
    for needle in needles:
        assert needle in message, f"[{case['id']}] error message must name {needle!r}; got: {message}"


class _NoopStep(BaseStep):
    """Minimal registrable step, so the steps-entry case reaches the insertion path."""

    def __init__(self, config: dict[str, Any] | None = None) -> None:
        super().__init__(name="noop")

    async def execute(self, ctx: Any) -> StepResult:
        return StepResult(action="continue")


class TestConfigParseFailFast:
    """`pipeline.configure` parse-time errors."""

    def test_missing_step_in_configure_raises_configuration_error(self) -> None:
        case = CASES["missing_step_in_configure_raises_configuration_error"]
        with pytest.raises(ConfigurationError) as excinfo:
            _build_from_yaml(case)
        _assert_message_contains(case, str(excinfo.value))
        expected_code = case["expected"]["error_code"]
        assert excinfo.value.code == expected_code, (
            f"[{case['id']}] the WIRE CODE is the contract, not the class name: "
            f"got {excinfo.value.code!r}, expected {expected_code!r}. All three SDKs name this "
            f"class ConfigurationError, so asserting the class name passes while the emitted "
            f"codes diverge."
        )

    def test_unknown_configure_field_raises_configuration_error(self) -> None:
        """A key outside the four configurable fields is a start-up error.

        This is the row on which the three SDKs diverged most sharply and which
        no case exercised: a plain typo. `ignore_error` is one character from
        `ignore_errors`, and the gate that used to gate on `hasattr(step, key)`
        happened to reject it only because no step declares that attribute —
        not because the field set was closed.
        """
        case = CASES["unknown_configure_field_raises_configuration_error"]
        _assert_parse_time_rejection(case)

    def test_configure_must_not_rewrite_a_step_capability_contract(self) -> None:
        """`requires` / `provides` under `configure:` MUST be rejected.

        Both ARE attributes of every BaseStep, so `hasattr(step, key)` accepted
        them and `setattr` applied them. Fed this exact YAML — which shipped as
        the canonical example in features/middleware-system.md — apcore-python
        moved `input_validation` from requires=('module',) to
        requires=('context',), deleting the dependency `module_lookup`
        satisfies. Construction then validates cleanly, so the
        PIPELINE_DEPENDENCY_ERROR MUST pinned by
        `unmet_requires_raises_pipeline_dependency_error` can never fire for
        that step. The documented way to exercise the dependency contract was
        the way to switch it off.
        """
        case = CASES["configure_must_not_rewrite_a_step_capability_contract"]
        error = _assert_parse_time_rejection(case)
        # Rejecting is half of it; the message has to send the operator to the
        # step class, since `requires` did not disappear — it moved.
        assert "capability contract" in str(error), (
            f"[{case['id']}] the message must say WHERE requires/provides live now, "
            f"not merely that the key was rejected; got: {error}"
        )


class TestConfigurableFieldSetIsFour:
    """`driver_contract.configurable_set_is_four` — the accept half.

    Without this, every rejection case above is equally satisfied by an
    implementation that rejects `configure` outright.
    """

    def test_all_four_configurable_fields_are_accepted(self) -> None:
        case = CASES["all_four_configurable_fields_are_accepted"]
        expected = case["expected"]["configured_step_fields"]

        strategy: Any = None
        raised: Exception | None = None
        try:
            # `driver_contract.snake_case_is_the_wire_spelling`: the fixture's
            # keys go in verbatim. No translation table — a driver that spells
            # them its own way tests its translation table, not the contract.
            strategy = _build_from_yaml(case)
        except Exception as exc:  # noqa: BLE001 - the fixture decides whether this may raise
            raised = exc

        assert (raised is not None) is case["expected"][
            "raises"
        ], f"[{case['id']}] all four declared fields must be accepted; got {raised!r}"
        assert strategy is not None
        strategy_callable = all(callable(getattr(step, "execute", None)) for step in strategy.steps)
        assert (
            strategy_callable is case["expected"]["strategy_callable"]
        ), f"[{case['id']}] strategy_callable: every step must expose a callable execute()"

        # `driver_contract.read_the_field_back_off_the_step`: assert the four
        # values ON THE STEP. `raises: false` alone also passes against an
        # implementation that accepts the keys and applies none of them — the
        # pre-fix apcore-rust behaviour for requires/provides: warn, drop,
        # continue. Every one of these four differs from the built-in default
        # (match_modules=None, ignore_errors=False, pure=True, timeout_ms=0),
        # so each assertion can only pass if the override really landed.
        step = next((s for s in strategy.steps if s.name == expected["step_name"]), None)
        assert step is not None, (
            f"[{case['id']}] the configured step {expected['step_name']!r} is missing from the "
            f"built strategy: {[s.name for s in strategy.steps]}"
        )
        assert (
            list(step.match_modules or ()) == expected["match_modules"]
        ), f"[{case['id']}] match_modules was accepted but not applied: {step.match_modules!r}"
        assert (
            step.ignore_errors == expected["ignore_errors"]
        ), f"[{case['id']}] ignore_errors was accepted but not applied: {step.ignore_errors!r}"
        assert step.pure == expected["pure"], (
            f"[{case['id']}] pure was accepted but not applied: {step.pure!r} "
            f"(the built-in default is True, so this only passes if the override landed)"
        )
        assert (
            step.timeout_ms == expected["timeout_ms"]
        ), f"[{case['id']}] timeout_ms was accepted but not applied: {step.timeout_ms!r}"

    def test_the_configurable_set_is_exactly_four(self) -> None:
        """`driver_contract.configurable_set_is_four` — pin the SET, not just its members.

        The accept case proves the four work and the reject cases prove two
        specific keys do not. Neither pins the size: an SDK could add a fifth
        and stay green on both. This reads the declared set off the
        implementation and compares it to the fixture's accept case.
        """
        from apcore.pipeline_config import _CONFIGURABLE_STEP_FIELDS

        case = CASES["all_four_configurable_fields_are_accepted"]
        declared = set(_CONFIGURABLE_STEP_FIELDS)
        assert declared == {"match_modules", "ignore_errors", "pure", "timeout_ms"}, (
            f"$defs/ConfigurableStepFields declares exactly four fields; " f"this SDK declares {sorted(declared)}"
        )
        assert declared == set(
            case["input"]["yaml"]["pipeline"]["configure"]["input_validation"]
        ), "the accept case must exercise every configurable field and no others"
        assert not declared & {
            "requires",
            "provides",
        }, "a step's capability contract is declared by its implementation, never by config"


class TestStrategyConstructionFailFast:
    """`requires:` / `provides:` validation happens at construction, not at call()."""

    def test_unmet_requires_raises_pipeline_dependency_error(self) -> None:
        case = CASES["unmet_requires_raises_pipeline_dependency_error"]
        # OBSERVE the phase that raises. This used to read
        # `assert case["expected"]["raised_at"] == "strategy_construction"`,
        # which compares the fixture to a transcription of itself and passes
        # even if the SDK deferred validation to the first call()
        # (apcore-python#32 / aiperceivable/apcore#81).
        raised_at: str | None = None
        error: PipelineDependencyError | None = None
        strategy: ExecutionStrategy | None = None
        try:
            strategy = _strategy_from_fixture(case)
        except PipelineDependencyError as exc:
            raised_at, error = "strategy_construction", exc

        assert strategy is None, (
            f"[{case['id']}] construction returned a strategy — an unmet `requires:` was NOT "
            f"rejected at strategy construction, so it could only surface later at call()"
        )
        assert raised_at == case["expected"]["raised_at"], (
            f"[{case['id']}] the dependency failure must surface at "
            f"{case['expected']['raised_at']!r}, observed {raised_at!r}"
        )
        assert error is not None
        _assert_message_contains(case, str(error))
        assert (
            error.step_name == "execute"
        ), f"[{case['id']}] the error must name the dependent step, got {error.step_name!r}"

    def test_satisfied_requires_constructs_successfully(self) -> None:
        case = CASES["satisfied_requires_constructs_successfully"]
        # Both `raises` and `strategy_callable` are now bound to observations;
        # they used to be asserted against literals, which cannot fail on SDK
        # behaviour (apcore-python#32 / aiperceivable/apcore#81).
        strategy: ExecutionStrategy | None = None
        raised: Exception | None = None
        try:
            strategy = _strategy_from_fixture(case)
        except Exception as exc:  # noqa: BLE001 - the fixture decides whether this may raise
            raised = exc

        assert (raised is not None) is case["expected"][
            "raises"
        ], f"[{case['id']}] a satisfied `requires:` must construct cleanly; got {raised!r}"
        assert strategy is not None
        assert [s.name for s in strategy.steps] == [
            s["name"] for s in case["input"]["strategy"]["steps"]
        ], f"[{case['id']}] construction must preserve the declared step order"
        # `strategy_callable`: every step is runnable, so the strategy can be
        # executed — construction validated cleanly and left nothing deferred.
        strategy_callable = all(callable(getattr(step, "execute", None)) for step in strategy.steps)
        assert (
            strategy_callable is case["expected"]["strategy_callable"]
        ), f"[{case['id']}] strategy_callable: every step must expose a callable execute()"


class TestStepsEntryIsClosed:
    """`pipeline.steps` entries reject keys `$defs/PipelineStep` does not declare."""

    def test_unknown_key_on_a_steps_entry_raises_configuration_error(self) -> None:
        """A typo on a `steps:` entry is a startup error, not a silently absent field.

        `driver_contract.steps_entries_are_closed_too`: reaching the insertion
        path at all needs a step TYPE registered under the case's `type` value.
        Rewriting the case to use a built-in step name would exercise the
        `configure`/lookup path instead and pass without testing anything.

        Before the fix, this exact entry built successfully with
        `timeout_ms == 0` — the operator's 5000 ms silently absent, because
        `_resolve_step` read the ten fields it knew and ignored the rest while
        `$defs/PipelineStep` had been `additionalProperties: false` all along.
        """
        case = CASES["unknown_key_on_a_steps_entry_raises_configuration_error"]
        step_type = case["input"]["yaml"]["pipeline"]["steps"][0]["type"]
        register_step_type(step_type, _NoopStep)
        try:
            _assert_parse_time_rejection(case)
        finally:
            _step_type_registry.pop(step_type, None)


def test_every_fixture_case_has_a_driver() -> None:
    driven = {
        "missing_step_in_configure_raises_configuration_error",
        "unmet_requires_raises_pipeline_dependency_error",
        "satisfied_requires_constructs_successfully",
        "unknown_configure_field_raises_configuration_error",
        "configure_must_not_rewrite_a_step_capability_contract",
        "all_four_configurable_fields_are_accepted",
        "unknown_key_on_a_steps_entry_raises_configuration_error",
    }
    assert set(CASES) == driven, (
        f"pipeline_failfast_config.json cases without a driver: {sorted(set(CASES) - driven)}; "
        f"drivers with no matching case: {sorted(driven - set(CASES))}"
    )
