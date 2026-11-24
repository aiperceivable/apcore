"""Drive `tracing_from_config.json` — §10.1.1 (#118 D-68 C').

Every case goes through ``APCore(config=Config(document))``. A driver that
called ``build_tracing_middleware`` directly would prove the builder works,
which was never in doubt; what was inert for the whole life of these keys is
the step before it — no SDK extracted ``observability.tracing.*`` from a
``Config`` and installed anything.
"""

from __future__ import annotations

import logging
from typing import Any

import pytest

from apcore import APCore
from apcore.config import Config
from apcore.errors import ConfigError
from apcore.observability.tracing import OTLPExporter, StdoutExporter, TracingMiddleware

from .canonical_fixtures import case_ids, load_fixture

FIXTURE = load_fixture("tracing_from_config.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}

_EXPORTER_KINDS = {"stdout": StdoutExporter, "otlp": OTLPExporter}


def _build(case: dict[str, Any]) -> tuple[list[TracingMiddleware], ConfigError | None]:
    """Validate, then construct — the order a real deployment uses.

    `Config(document)` does not validate; `Config.load` does. A driver that
    skipped the validate step would report "accepted" for a configuration the
    loader rejects, which is where `expected.loads` is decided.
    """
    config = Config(dict(case["input"]["config"]))
    try:
        config.validate()
        client = APCore(config=config)
    except ConfigError as exc:
        return [], exc
    installed = [m for m in client.executor.middlewares if isinstance(m, TracingMiddleware)]
    return installed, None


@pytest.mark.parametrize("case_id", list(CASES))
def test_tracing_from_config(case_id: str, caplog: pytest.LogCaptureFixture) -> None:
    case = CASES[case_id]
    expected = case["expected"]
    caplog.clear()

    with caplog.at_level(logging.WARNING, logger="apcore"):
        installed, error = _build(case)

    if expected.get("loads") is False:
        assert error is not None, "the configuration must be rejected"
        assert error.code == expected["error_code"]
        assert expected["error_message_contains"] in str(error)
        return

    assert error is None, f"unexpected rejection: {error}"

    if "tracing_middleware_count" in expected:
        assert len(installed) == expected["tracing_middleware_count"]

    kind = expected.get("exporter_kind")
    if kind is not None:
        if not installed and expected.get("otlp_may_be_unavailable"):
            # §10.1.1 requirement 4: this SDK's OTLP support is an optional
            # dependency, and refusing to install rather than installing a
            # middleware that discards every span IS the conformant answer.
            return
        assert installed, f"expected an {kind} exporter and no middleware was installed"
        assert isinstance(installed[0]._exporter, _EXPORTER_KINDS[kind])

    endpoint = expected.get("otlp_endpoint")
    if endpoint is not None and installed:
        assert str(getattr(installed[0]._exporter, "_endpoint", "")) == endpoint

    if "sampling_strategy" in expected:
        assert installed[0]._sampling_strategy == expected["sampling_strategy"]
    if "sampling_rate" in expected:
        assert installed[0]._sampling_rate == expected["sampling_rate"]

    warns_naming = expected.get("warns_naming")
    if warns_naming is not None and expected.get("deprecation_warning") is not True:
        hits = [r for r in caplog.records if warns_naming in r.getMessage()]
        assert len(hits) == 1, [r.getMessage() for r in caplog.records]


@pytest.mark.parametrize(
    "case_id",
    [i for i, c in CASES.items() if "deprecation_warning" in c["expected"]],
)
def test_the_deprecation_half(case_id: str) -> None:
    """§9.2.4's notice fires on `Config.load`, not on `Config(document)`.

    Split out for that reason: reading it off the constructor would report "no
    warning" for every case and pass the positive half vacuously.
    """
    import tempfile
    import warnings
    from pathlib import Path

    import yaml

    case = CASES[case_id]
    root = Path(tempfile.mkdtemp())
    path = root / "apcore.yaml"
    path.write_text(yaml.safe_dump(dict(case["input"]["config"])), encoding="utf-8")
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        Config.load(str(path))
    notices = [str(w.message) for w in caught if "9.2.4" in str(w.message)]

    if case["expected"]["deprecation_warning"]:
        assert len(notices) == 1, notices
        assert case["expected"]["warns_naming"] in notices[0]
    else:
        assert notices == [], notices


def test_every_fixture_case_is_driven() -> None:
    assert set(case_ids("tracing_from_config.json")) == set(CASES)
