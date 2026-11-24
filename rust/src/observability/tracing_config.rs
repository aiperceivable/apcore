//! PROTOCOL_SPEC §10.1.1 — build a [`TracingMiddleware`] from `observability.tracing.*`.
//!
//! The five keys are one unit, and treating them as five independent keys is
//! what kept all five inert (apcore#118, decision D-68 C'). Wiring
//! `sampling_rate` alone yields a key that reads configuration, sets a field
//! and still samples every span, because the strategy short-circuits ahead of
//! the rate. Adding the strategy yields two keys configuring a middleware
//! nothing installs. Installing one needs an exporter, and an exporter is an
//! object rather than a name.
//!
//! Two of the five were never missing, only declared in the wrong place:
//! §9.15.2's namespace registration has always carried `strategy` and
//! `otlp_endpoint`, while `schemas/apcore-config.schema.json` did not — so
//! `_config.strict` rejected both as unknown keys while the specification
//! documented their defaults.

use crate::config::Config;
use crate::errors::{ErrorCode, ModuleError};
use crate::observability::exporters::{OTLPExporter, StdoutExporter};
use crate::observability::span::SpanExporter;
use crate::observability::tracing_middleware::{SamplingStrategy, TracingMiddleware};

/// §10.1.1 requirement 2. Closed, and `in_memory` is deliberately absent: the
/// in-memory exporter is a test buffer a caller selecting it BY NAME has no
/// standardised way to read, so it would take effect and produce nothing an
/// operator can see — the failure this section exists to remove.
const EXPORTERS: [&str; 3] = ["stdout", "otlp", "jaeger"];

/// The endpoint an OTLP exporter uses when `otlp_endpoint` is null.
///
/// Stated in §10.1.1's table so the three SDKs cannot drift: this crate's
/// [`OTLPExporter::new`] takes a required endpoint and had no default of its
/// own, while apcore-python and apcore-typescript both defaulted to this.
pub const DEFAULT_OTLP_ENDPOINT: &str = "http://localhost:4318/v1/traces";

/// The middleware `observability.tracing.*` asks for, or `None`.
///
/// Returns `None` when the configuration does not ask for tracing, and when the
/// named exporter is one this build cannot construct. Returns `Err` only for a
/// configuration that is self-contradictory — see
/// [`check_endpoint_matches_exporter`].
pub fn build_tracing_middleware(config: &Config) -> Result<Option<TracingMiddleware>, ModuleError> {
    if !config.observability.tracing.enabled {
        // The default, and the whole of the blast radius: a project that does
        // not ask for tracing is untouched by §10.1.1.
        return Ok(None);
    }

    let tracing = &config.observability.tracing;
    check_endpoint_matches_exporter(&tracing.exporter, tracing.otlp_endpoint.as_deref())?;

    let Some(exporter) = build_exporter(&tracing.exporter, tracing.otlp_endpoint.as_deref()) else {
        return Ok(None);
    };

    Ok(Some(TracingMiddleware::with_sampling(
        exporter,
        parse_strategy(&tracing.strategy),
        tracing.sampling_rate,
    )))
}

/// §10.1.1 requirement 3 — an endpoint nothing reads is a rejected config.
///
/// Accepting it would leave an operator with a value they wrote down and no way
/// to discover that it does nothing, which is the shape of every defect
/// apcore#118 found.
fn check_endpoint_matches_exporter(
    exporter_name: &str,
    endpoint: Option<&str>,
) -> Result<(), ModuleError> {
    if endpoint.is_none() || exporter_name == "otlp" {
        return Ok(());
    }
    Err(ModuleError::new(
        ErrorCode::ConfigInvalid,
        format!(
            "observability.tracing.otlp_endpoint is set but \
             observability.tracing.exporter is '{exporter_name}', which does not read it. \
             Set exporter to 'otlp', or remove the endpoint."
        ),
    ))
}

/// §10.7. The four values are validated by `Config`, so an unknown one can only
/// arrive from a caller who bypassed validation; it falls back to the declared
/// default rather than panicking.
fn parse_strategy(name: &str) -> SamplingStrategy {
    match name {
        "proportional" => SamplingStrategy::Probabilistic,
        "error_first" => SamplingStrategy::ErrorFirst,
        "off" => SamplingStrategy::Never,
        _ => SamplingStrategy::Always,
    }
}

/// §10.1.1 requirements 2 and 4.
///
/// A name this build cannot construct returns `None` after saying so. It never
/// substitutes a different exporter: a silent substitution is the failure this
/// section removes, and a middleware whose exporter discards every span is
/// worse than no middleware — the operator would see tracing "enabled" and no
/// traces, with nothing to read.
fn build_exporter(name: &str, endpoint: Option<&str>) -> Option<Box<dyn SpanExporter>> {
    match name {
        "stdout" => Some(Box::new(StdoutExporter)),
        "otlp" => {
            // Without the `events` feature this crate's OTLPExporter is a
            // silent no-op that discards every span, so installing a middleware
            // around it is exactly the "enabled and no traces" state
            // requirement 4 refuses.
            if cfg!(not(feature = "events")) {
                tracing::warn!(
                    "observability.tracing.exporter is 'otlp' but this build of apcore does not \
                     have the `events` feature, so the OTLP exporter would discard every span. \
                     No tracing middleware was installed. Rebuild with `--features events`, or \
                     set exporter to 'stdout'."
                );
                return None;
            }
            Some(Box::new(OTLPExporter::new(
                endpoint.unwrap_or(DEFAULT_OTLP_ENDPOINT),
            )))
        }
        "jaeger" => {
            tracing::warn!(
                "observability.tracing.exporter is 'jaeger', which names no implementation in \
                 any apcore SDK. No tracing middleware was installed and no spans will be \
                 exported — the same as before this key was wired. Use 'otlp' with a Jaeger \
                 collector's OTLP endpoint. The value is accepted for the 1.x line and removed \
                 at v2.0."
            );
            None
        }
        other => {
            // Unreachable through a validated Config: the enum is closed and
            // `validate_key_constraint` rejects anything else. Kept so a caller
            // reaching this helper directly gets the same refusal rather than a
            // None with no reason.
            tracing::warn!(
                exporter = other,
                allowed = EXPORTERS.join(", "),
                "observability.tracing.exporter is not one of the allowed values. No tracing \
                 middleware was installed."
            );
            None
        }
    }
}
