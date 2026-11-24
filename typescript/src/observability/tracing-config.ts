/**
 * PROTOCOL_SPEC §10.1.1 — build a `TracingMiddleware` from `observability.tracing.*`.
 *
 * The five keys are one unit, and treating them as five independent keys is
 * what kept all five inert (apcore#118, decision D-68 C'). Wiring
 * `sampling_rate` alone yields a key that reads configuration, sets a field and
 * still samples every span, because the strategy short-circuits ahead of the
 * rate. Adding the strategy yields two keys configuring a middleware nothing
 * installs. Installing one needs an exporter, and an exporter is an object
 * rather than a name.
 *
 * Two of the five were never missing, only declared in the wrong place:
 * §9.15.2's namespace registration has always carried `strategy` and
 * `otlp_endpoint`, while `schemas/apcore-config.schema.json` did not — so
 * `_config.strict` rejected both as unknown keys while the specification
 * documented their defaults.
 */

import { ConfigError } from '../errors.js';
import type { Config } from '../config.js';
import {
  OTLPExporter,
  StdoutExporter,
  TracingMiddleware,
  type SpanExporter,
} from './tracing.js';

/**
 * §10.1.1 requirement 2. Closed, and `in_memory` is deliberately absent: the
 * in-memory exporter is a test buffer a caller selecting it BY NAME has no
 * standardised way to read, so it would take effect and produce nothing an
 * operator can see — the failure this section exists to remove.
 */
const EXPORTERS = ['stdout', 'otlp', 'jaeger'] as const;

/**
 * The endpoint an OTLP exporter uses when `otlp_endpoint` is null. Stated in
 * §10.1.1's table so the three SDKs cannot drift: apcore-rust's OTLPExporter
 * takes a required endpoint and had no default of its own.
 */
export const DEFAULT_OTLP_ENDPOINT = 'http://localhost:4318/v1/traces';

function leaf<T>(config: Config, key: string, fallback: T): T {
  const value = config.get(`observability.tracing.${key}`) as T | null | undefined;
  return value === null || value === undefined ? fallback : value;
}

/**
 * The middleware `observability.tracing.*` asks for, or `null`.
 *
 * Returns `null` when the configuration does not ask for tracing, and when the
 * named exporter is one this installation cannot build. Throws `ConfigError`
 * only for a configuration that is self-contradictory — see
 * `checkEndpointMatchesExporter`.
 */
export function buildTracingMiddleware(config: Config | null | undefined): TracingMiddleware | null {
  if (config === null || config === undefined) return null;
  if (leaf<boolean>(config, 'enabled', false) !== true) {
    // The default, and the whole of the blast radius: a project that does not
    // ask for tracing is untouched by §10.1.1.
    return null;
  }

  const exporterName = leaf<string>(config, 'exporter', 'stdout');
  const endpoint = config.get('observability.tracing.otlp_endpoint') as string | null | undefined;
  checkEndpointMatchesExporter(exporterName, endpoint ?? null);

  const exporter = buildExporter(exporterName, endpoint ?? null);
  if (exporter === null) return null;

  return new TracingMiddleware(
    exporter,
    Number(leaf<number>(config, 'sampling_rate', 1.0)),
    String(leaf<string>(config, 'strategy', 'full')),
  );
}

/**
 * §10.1.1 requirement 3 — an endpoint nothing reads is a rejected config.
 *
 * Accepting it would leave an operator with a value they wrote down and no way
 * to discover that it does nothing, which is the shape of every defect
 * apcore#118 found.
 */
function checkEndpointMatchesExporter(exporterName: string, endpoint: string | null): void {
  if (endpoint === null || exporterName === 'otlp') return;
  throw new ConfigError(
    `observability.tracing.otlp_endpoint is set but observability.tracing.exporter is ` +
      `'${exporterName}', which does not read it. Set exporter to 'otlp', or remove the endpoint.`,
  );
}

/**
 * §10.1.1 requirements 2 and 4.
 *
 * A name this installation cannot build returns `null` after saying so. It
 * never substitutes a different exporter: a silent substitution is the failure
 * this section removes, and a middleware whose exporter discards every span is
 * worse than no middleware — the operator would see tracing "enabled" and no
 * traces, with nothing to read.
 */
function buildExporter(name: string, endpoint: string | null): SpanExporter | null {
  if (name === 'stdout') return new StdoutExporter();

  if (name === 'otlp') {
    return new OTLPExporter({ endpoint: endpoint ?? DEFAULT_OTLP_ENDPOINT });
  }

  if (name === 'jaeger') {
    console.warn(
      `[apcore] observability.tracing.exporter is 'jaeger', which names no implementation in ` +
        `any apcore SDK. No tracing middleware was installed and no spans will be exported — ` +
        `the same as before this key was wired. Use 'otlp' with a Jaeger collector's OTLP ` +
        `endpoint. The value is accepted for the 1.x line and removed at v2.0.`,
    );
    return null;
  }

  // Unreachable through a validated Config: the enum is closed and CONSTRAINTS
  // rejects anything else. Kept so a caller reaching this helper directly gets
  // the same refusal rather than a null with no reason.
  console.warn(
    `[apcore] observability.tracing.exporter is '${name}', which is not one of ` +
      `${EXPORTERS.join(', ')}. No tracing middleware was installed.`,
  );
  return null;
}
