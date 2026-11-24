"""PROTOCOL_SPEC §10.1.1 — build a `TracingMiddleware` from `observability.tracing.*`.

The five keys are one unit, and treating them as five independent keys is what
kept all five inert (apcore#118, decision D-68 C'). Wiring `sampling_rate`
alone yields a key that reads configuration, sets a field and still samples
every span, because the strategy short-circuits ahead of the rate. Adding the
strategy yields two keys configuring a middleware nothing installs. Installing
one needs an exporter, and an exporter is an object rather than a name.

Two of the five were never missing, only declared in the wrong place:
§9.15.2's namespace registration has always carried `strategy` and
`otlp_endpoint`, while `schemas/apcore-config.schema.json` did not — so
`_config.strict` rejected both as unknown keys while the specification
documented their defaults.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING, Any

from apcore.errors import ConfigError

if TYPE_CHECKING:  # pragma: no cover - typing only
    from apcore.config import Config
    from apcore.observability.tracing import TracingMiddleware

_logger = logging.getLogger("apcore")

#: §10.1.1 requirement 2. Closed, and `in_memory` is deliberately absent: the
#: in-memory exporter is a test buffer a caller selecting it BY NAME has no
#: standardised way to read, so it would take effect and produce nothing an
#: operator can see — the failure this section exists to remove.
_EXPORTERS = ("stdout", "otlp", "jaeger")

#: The endpoint an OTLP exporter uses when `otlp_endpoint` is null. Stated in
#: §10.1.1's table so the three SDKs cannot drift: apcore-rust's OTLPExporter
#: takes a required endpoint and had no default of its own.
DEFAULT_OTLP_ENDPOINT = "http://localhost:4318/v1/traces"


def _get(config: Config, key: str, fallback: Any) -> Any:
    value = config.get(f"observability.tracing.{key}")
    return fallback if value is None else value


def build_tracing_middleware(config: Config | None) -> TracingMiddleware | None:
    """The middleware `observability.tracing.*` asks for, or None.

    Returns None when the configuration does not ask for tracing, and when the
    named exporter is one this installation cannot build. Raises `ConfigError`
    (`CONFIG_INVALID`) only for a configuration that is self-contradictory —
    see `_check_endpoint_matches_exporter`.
    """
    if config is None:
        return None
    if not _get(config, "enabled", False):
        # The default, and the whole of the blast radius: a project that does
        # not ask for tracing is untouched by §10.1.1.
        return None

    exporter_name = _get(config, "exporter", "stdout")
    endpoint = config.get("observability.tracing.otlp_endpoint")
    _check_endpoint_matches_exporter(exporter_name, endpoint)

    exporter = _build_exporter(exporter_name, endpoint)
    if exporter is None:
        return None

    from apcore.observability.tracing import TracingMiddleware

    return TracingMiddleware(
        exporter=exporter,
        sampling_rate=float(_get(config, "sampling_rate", 1.0)),
        sampling_strategy=str(_get(config, "strategy", "full")),
    )


def _check_endpoint_matches_exporter(exporter_name: str, endpoint: Any) -> None:
    """§10.1.1 requirement 3 — an endpoint nothing reads is a rejected config.

    Accepting it would leave an operator with a value they wrote down and no
    way to discover that it does nothing, which is the shape of every defect
    apcore#118 found. The check is on the *declared* pair, so it fires whether
    the exporter came from the file, the environment or `Config.set`.
    """
    if endpoint is None or exporter_name == "otlp":
        return
    raise ConfigError(
        f"observability.tracing.otlp_endpoint is set but "
        f"observability.tracing.exporter is {exporter_name!r}, which does not read it. "
        f"Set exporter to 'otlp', or remove the endpoint."
    )


def _build_exporter(name: str, endpoint: Any) -> Any | None:
    """§10.1.1 requirements 2 and 4.

    A name this installation cannot build returns None after saying so. It
    never substitutes a different exporter: a silent substitution is the
    failure this section removes, and a middleware whose exporter discards
    every span is worse than no middleware — the operator would see tracing
    "enabled" and no traces, with nothing to read.
    """
    from apcore.observability.tracing import StdoutExporter

    if name == "stdout":
        return StdoutExporter()

    if name == "otlp":
        try:
            from apcore.observability.tracing import OTLPExporter

            return OTLPExporter(endpoint=endpoint or DEFAULT_OTLP_ENDPOINT)
        except Exception as exc:  # noqa: BLE001 - any import/init failure is the finding
            _logger.warning(
                "observability.tracing.exporter is 'otlp' but the OTLP exporter could not be "
                "built (%s). No tracing middleware was installed and no spans will be "
                "exported. Install the opentelemetry extra, or set exporter to 'stdout'.",
                exc,
            )
            return None

    if name == "jaeger":
        _logger.warning(
            "observability.tracing.exporter is 'jaeger', which names no implementation in any "
            "apcore SDK. No tracing middleware was installed and no spans will be exported — "
            "the same as before this key was wired. Use 'otlp' with a Jaeger collector's OTLP "
            "endpoint. The value is accepted for the 1.x line and removed at v2.0.",
        )
        return None

    # Unreachable through a validated Config: the enum is closed and
    # `_CONSTRAINTS` rejects anything else. Kept so a caller reaching this
    # helper directly gets the same refusal rather than a None with no reason.
    _logger.warning(
        "observability.tracing.exporter is %r, which is not one of %s. No tracing middleware " "was installed.",
        name,
        ", ".join(_EXPORTERS),
    )
    return None
