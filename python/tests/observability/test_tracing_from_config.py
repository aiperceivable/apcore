"""PROTOCOL_SPEC §10.1.1 — `observability.tracing.*` reaches the running client.

Five declared keys, all inert (apcore#118, decision D-68 C'). Two of them —
`strategy` and `otlp_endpoint` — were declared by §9.15.2's namespace
registration and absent from `schemas/apcore-config.schema.json`, so
`_config.strict` rejected them as unknown while the specification documented
their defaults. The other three were deprecated in spec v1.39.0 on the finding
that nothing read them.

**Every case here drives a real client built from a real `Config`**, and the
ones that can be measured are measured rather than asserted on a field. That is
the point: a test that calls `TracingMiddleware(...)` directly proves the
middleware works, which was never in doubt. What was in doubt — for the whole
life of these keys — is whether a `Config` reaches it.
"""

from __future__ import annotations

import logging
import warnings
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

from apcore import APCore
from apcore.config import Config
from apcore.context import Context
from apcore.errors import ConfigError
from apcore.observability.tracing import InMemoryExporter, TracingMiddleware


class _In(BaseModel):
    n: int


class _Out(BaseModel):
    n: int


class _Echo:
    input_schema = _In
    output_schema = _Out
    description = "Echoes its input; used to drive real calls through the pipeline."

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"n": inputs["n"]}


def _client(tracing: dict[str, Any] | None = None, **top: Any) -> APCore:
    doc: dict[str, Any] = {"version": "1.0", "project": {"name": "tracing-probe"}, **top}
    if tracing is not None:
        doc["observability"] = {"tracing": tracing}
    client = APCore(config=Config(doc))
    client.register("probe.echo", _Echo())
    return client


def _tracing_middlewares(client: APCore) -> list[TracingMiddleware]:
    return [m for m in client.executor.middlewares if isinstance(m, TracingMiddleware)]


# ---------------------------------------------------------------------------
# The five keys are readable, writable and validating alike
# ---------------------------------------------------------------------------

_KEYS = (
    ("enabled", False, True),
    ("sampling_rate", 1.0, 0.25),
    ("strategy", "full", "error_first"),
    ("exporter", "stdout", "otlp"),
    ("otlp_endpoint", None, "http://collector:4318/v1/traces"),
)


@pytest.mark.parametrize(("leaf", "default", "written"), _KEYS)
def test_the_key_has_its_canonical_default(leaf: str, default: Any, written: Any) -> None:
    assert Config.from_defaults().get(f"observability.tracing.{leaf}") == default


@pytest.mark.parametrize(("leaf", "default", "written"), _KEYS)
def test_the_key_round_trips_from_a_document(leaf: str, default: Any, written: Any) -> None:
    config = Config({"version": "1.0", "project": {"name": "p"}, "observability": {"tracing": {leaf: written}}})
    assert config.get(f"observability.tracing.{leaf}") == written


@pytest.mark.parametrize(("leaf", "default", "written"), _KEYS)
def test_the_key_round_trips_through_set(leaf: str, default: Any, written: Any) -> None:
    config = Config.from_defaults()
    config.set(f"observability.tracing.{leaf}", written)
    assert config.get(f"observability.tracing.{leaf}") == written


@pytest.mark.parametrize(("leaf", "default", "written"), _KEYS)
def test_the_key_is_accepted_under_strict(leaf: str, default: Any, written: Any) -> None:
    """`strategy` and `otlp_endpoint` were REJECTED here — the defect, exactly.

    `otlp_endpoint` needs its partner: §10.1.1 requirement 3 makes an endpoint
    against a non-OTLP exporter an error, and `stdout` is the default. That rule
    has its own case below.
    """
    tracing: dict[str, Any] = {leaf: written}
    if leaf == "otlp_endpoint" and written is not None:
        tracing["exporter"] = "otlp"
    config = Config(
        {"version": "1.0", "project": {"name": "p"}, "_config": {"strict": True}, "observability": {"tracing": tracing}}
    )
    config.validate()


@pytest.mark.parametrize(
    ("leaf", "bad"),
    [
        ("strategy", "sometimes"),
        ("exporter", "in_memory"),
        ("sampling_rate", 2.0),
        ("otlp_endpoint", ""),
    ],
)
def test_an_out_of_range_value_is_rejected(leaf: str, bad: Any) -> None:
    """`in_memory` is in the list on purpose: §9.15.2 used to name it, and
    §10.1.1 requirement 2 forbids it as a configuration value because the
    in-memory exporter is a test buffer nothing can read back by name."""
    config = Config({"version": "1.0", "project": {"name": "p"}, "observability": {"tracing": {leaf: bad}}})
    with pytest.raises(ConfigError):
        config.validate()


# ---------------------------------------------------------------------------
# Installation
# ---------------------------------------------------------------------------


def test_no_tracing_configuration_installs_nothing() -> None:
    """The whole blast radius: a project that does not ask for tracing is untouched."""
    assert _tracing_middlewares(_client()) == []
    assert _tracing_middlewares(_client({"enabled": False})) == []
    # ...even with everything else configured.
    assert _tracing_middlewares(_client({"strategy": "off", "sampling_rate": 0.5})) == []


def test_enabled_installs_a_middleware() -> None:
    assert len(_tracing_middlewares(_client({"enabled": True}))) == 1


def test_the_configured_strategy_and_rate_reach_the_middleware() -> None:
    mw = _tracing_middlewares(_client({"enabled": True, "strategy": "proportional", "sampling_rate": 0.1}))[0]
    assert mw._sampling_strategy == "proportional"
    assert mw._sampling_rate == 0.1


# ---------------------------------------------------------------------------
# The rate is measured, not asserted on a field
# ---------------------------------------------------------------------------


def _sampled_fraction(tracing: dict[str, Any], runs: int = 400) -> float:
    """Drive `runs` real calls and report what fraction produced a span.

    The exporter is swapped for an in-memory one AFTER construction, through
    the public `set_exporter` — the same door the `span_exporter` extension
    uses. Nothing about the sampling configuration is touched, so what this
    counts is the decision the CONFIG produced.
    """
    client = _client(tracing)
    mw = _tracing_middlewares(client)[0]
    collected = InMemoryExporter()
    mw.set_exporter(collected)
    for i in range(runs):
        client.call("probe.echo", {"n": i})
    return len(collected.get_spans()) / runs


def test_full_samples_everything() -> None:
    assert _sampled_fraction({"enabled": True, "strategy": "full", "sampling_rate": 0.1}) == 1.0


def test_off_samples_nothing() -> None:
    assert _sampled_fraction({"enabled": True, "strategy": "off", "sampling_rate": 1.0}) == 0.0


def test_proportional_samples_at_the_configured_rate() -> None:
    """The acceptance criterion in D-68 C': an operator asking for 10% gets 10%.

    Before this change they got 100%, because `full` is the default strategy
    and it short-circuits ahead of the rate — which is why wiring
    `sampling_rate` alone would have been a fix that changed nothing.
    """
    fraction = _sampled_fraction({"enabled": True, "strategy": "proportional", "sampling_rate": 0.1}, runs=2000)
    # Wide bounds: this is a real random draw, and the assertion that matters is
    # "roughly a tenth", not "not one" and not "all of them".
    assert 0.05 < fraction < 0.16, fraction


# ---------------------------------------------------------------------------
# The exporter, by name
# ---------------------------------------------------------------------------


def _otlp_is_buildable() -> bool:
    """Can this installation construct an OTLP exporter at all?

    `OTLPExporter` needs the `opentelemetry` extra, which is optional and is not
    installed in CI. §10.1.1 requirement 4 makes that a first-class outcome
    rather than a skip: the middleware is NOT installed and the reason is
    logged, because one whose exporter discards every span would show an
    operator tracing "enabled" and no traces. Both branches are asserted below.
    """
    from apcore.observability.tracing import OTLPExporter

    try:
        OTLPExporter()
    except ImportError:
        return False
    return True


def test_stdout_is_the_default_exporter() -> None:
    from apcore.observability.tracing import StdoutExporter

    mw = _tracing_middlewares(_client({"enabled": True}))[0]
    assert isinstance(mw._exporter, StdoutExporter)


@pytest.mark.skipif(not _otlp_is_buildable(), reason="the opentelemetry extra is not installed")
def test_otlp_endpoint_reaches_the_exporter() -> None:
    mw = _tracing_middlewares(
        _client(
            {
                "enabled": True,
                "exporter": "otlp",
                "otlp_endpoint": "http://collector.internal:4318/v1/traces",
            }
        )
    )[0]
    assert "collector.internal" in str(getattr(mw._exporter, "_endpoint", ""))


@pytest.mark.skipif(not _otlp_is_buildable(), reason="the opentelemetry extra is not installed")
def test_a_null_endpoint_uses_the_specified_default() -> None:
    from apcore.observability.tracing_config import DEFAULT_OTLP_ENDPOINT

    mw = _tracing_middlewares(_client({"enabled": True, "exporter": "otlp"}))[0]
    assert str(getattr(mw._exporter, "_endpoint", "")) == DEFAULT_OTLP_ENDPOINT


def test_otlp_without_its_extra_installs_nothing_and_says_so(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """§10.1.1 requirement 4's second case, asserted rather than skipped.

    The two cases above are conditional on an optional dependency; this one
    runs either way, by simulating its absence. Every submodule the constructor
    imports is blanked, not just the parent package — `from
    opentelemetry.sdk.trace import ...` resolves straight out of `sys.modules`
    when the submodule is cached, so patching only `"opentelemetry"` simulates
    absence exactly as long as nothing else has imported the real thing.
    """
    import sys
    from unittest.mock import patch

    absent = dict.fromkeys(
        [
            "opentelemetry",
            "opentelemetry.exporter.otlp.proto.http.trace_exporter",
            "opentelemetry.sdk.resources",
            "opentelemetry.sdk.trace",
            "opentelemetry.sdk.trace.export",
            "opentelemetry.trace",
        ]
    )
    with patch.dict(sys.modules, absent), caplog.at_level(logging.WARNING, logger="apcore"):
        client = _client({"enabled": True, "exporter": "otlp"})

    assert _tracing_middlewares(client) == []
    hits = [r for r in caplog.records if "OTLP exporter could not be built" in r.getMessage()]
    assert len(hits) == 1, [r.getMessage() for r in caplog.records]
    assert "stdout" in hits[0].getMessage()


def test_an_endpoint_with_a_non_otlp_exporter_is_rejected_at_load() -> None:
    """§10.1.1 requirement 3 — not a silent no-op.

    An endpoint written down and read by nothing is the shape of every defect
    apcore#118 found, so this is the one place the section adds an error.
    """
    with pytest.raises(ConfigError, match="otlp_endpoint"):
        _client({"enabled": True, "exporter": "stdout", "otlp_endpoint": "http://x:4318"})


def test_jaeger_warns_installs_nothing_and_substitutes_nothing(
    caplog: pytest.LogCaptureFixture,
) -> None:
    with caplog.at_level(logging.WARNING, logger="apcore"):
        client = _client({"enabled": True, "exporter": "jaeger"})
    assert _tracing_middlewares(client) == []
    hits = [r for r in caplog.records if "jaeger" in r.getMessage()]
    assert len(hits) == 1, [r.getMessage() for r in caplog.records]
    assert "otlp" in hits[0].getMessage()


# ---------------------------------------------------------------------------
# Precedence (§10.1.1 requirement 6, D-73)
# ---------------------------------------------------------------------------


def test_configuration_never_installs_a_second_tracing_middleware() -> None:
    """§10.1.1 requirement 6, stated where it can actually be violated.

    Configuration installs at client construction, into a chain that is empty,
    so it can never be the thing that adds a second. The case that could go
    wrong is the caller-supplied Executor below; this one pins that repeated
    construction from the same `Config` does not accumulate.
    """
    config = Config({"version": "1.0", "project": {"name": "p"}, "observability": {"tracing": {"enabled": True}}})
    for _ in range(3):
        assert len(_tracing_middlewares(APCore(config=config))) == 1


def test_a_middleware_added_by_the_caller_keeps_its_own_configuration() -> None:
    """A caller who wires their own — with an exporter this surface cannot name,
    a custom endpoint or headers — is adding it deliberately and keeps it,
    configured their way rather than the file's."""
    mine = TracingMiddleware(exporter=InMemoryExporter(), sampling_strategy="off")
    client = _client({"enabled": True, "strategy": "full"})
    client.use(mine)
    assert mine in _tracing_middlewares(client)
    assert mine._sampling_strategy == "off"


def test_a_caller_supplied_executor_is_left_alone() -> None:
    """Parity with config-driven ACL discovery: an Executor the caller built is
    respected as-is, tracing included."""
    from apcore.executor import Executor
    from apcore.registry import Registry

    registry = Registry()
    doc = {"version": "1.0", "project": {"name": "p"}, "observability": {"tracing": {"enabled": True}}}
    config = Config(doc)
    client = APCore(registry=registry, executor=Executor(registry=registry, config=config), config=config)
    assert _tracing_middlewares(client) == []


# ---------------------------------------------------------------------------
# §9.2.4 — the withdrawal is cancelled
# ---------------------------------------------------------------------------


def _load_capturing(observability: dict[str, Any]) -> list[str]:
    """Load a real file and return the deprecation warnings it produced.

    Through `Config.load`, not `Config(document)`: §9.2.4's notice is emitted
    once per configuration LOAD, and `Config(...)` never emits it. A first draft
    of these cases used the constructor, so the negative ones passed while
    naming keys that were still in the table — green for the wrong reason,
    which is worse than red.
    """
    import tempfile

    import yaml

    root = Path(tempfile.mkdtemp())
    path = root / "apcore.yaml"
    path.write_text(
        yaml.safe_dump({"version": "1.0", "project": {"name": "p"}, "observability": observability}),
        encoding="utf-8",
    )
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        Config.load(str(path))
    return [str(w.message) for w in caught if issubclass(w.category, DeprecationWarning)]


@pytest.mark.parametrize(
    ("leaf", "value"),
    [
        ("enabled", True),
        ("sampling_rate", 0.5),
        ("exporter", "stdout"),
        ("strategy", "off"),
        ("otlp_endpoint", None),
    ],
)
def test_the_wired_keys_do_not_warn_as_deprecated(leaf: str, value: Any) -> None:
    """§9.2.4 requirement 1: the table is the whole list, and a key that has
    left it MUST NOT warn. Declaring one of the first three was a deprecation
    notice up to spec v1.43.0."""
    hits = [m for m in _load_capturing({"tracing": {leaf: value}}) if "9.2.4" in m]
    assert hits == [], hits


def test_the_metrics_keys_still_warn() -> None:
    """The other half, and the case that proves the one above can fail.
    `metrics.enabled` and `.exporter` stay in §9.2.4: there is no
    metrics-exporter abstraction in any SDK and `MetricsCollector` arrives only
    as a constructor argument."""
    hits = [m for m in _load_capturing({"metrics": {"enabled": True}}) if "9.2.4" in m]
    assert len(hits) == 1, hits
    assert "observability.metrics.enabled" in hits[0]
