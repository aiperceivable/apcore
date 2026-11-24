/**
 * PROTOCOL_SPEC §10.1.1 — `observability.tracing.*` reaches the running client.
 *
 * Five declared keys, all inert (apcore#118, decision D-68 C'). Two of them —
 * `strategy` and `otlp_endpoint` — were declared by §9.15.2's namespace
 * registration and absent from `schemas/apcore-config.schema.json`, so
 * `_config.strict` rejected them as unknown while the specification documented
 * their defaults. The other three were deprecated in spec v1.39.0 on the
 * finding that nothing read them.
 *
 * Every case drives a real client built from a real `Config`, and the ones that
 * can be measured are measured rather than asserted on a field. A test that
 * calls `new TracingMiddleware(...)` directly proves the middleware works,
 * which was never in doubt; what was in doubt is whether a `Config` reaches it.
 */

import { describe, it, expect, vi } from 'vitest';
import { Type } from '@sinclair/typebox';

import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { ConfigError } from '../src/errors.js';
import { DEFAULTS } from '../src/config-defaults.js';
import { InMemoryExporter, StdoutExporter, TracingMiddleware } from '../src/observability/tracing.js';
import { DEFAULT_OTLP_ENDPOINT } from '../src/observability/tracing-config.js';

function makeClient(tracing?: Record<string, unknown>, extra: Record<string, unknown> = {}): APCore {
  const doc: Record<string, unknown> = {
    version: '1.0',
    project: { name: 'tracing-probe' },
    ...extra,
  };
  if (tracing !== undefined) doc['observability'] = { tracing };
  const client = new APCore({ config: new Config(doc) });
  client.register('probe.echo', {
    inputSchema: Type.Object({ n: Type.Number() }),
    outputSchema: Type.Object({ n: Type.Number() }),
    description: 'Echoes its input; used to drive real calls through the pipeline.',
    execute: (inputs: Record<string, unknown>) => ({ n: inputs.n as number }),
  });
  return client;
}

function tracingMiddlewares(client: APCore): TracingMiddleware[] {
  return client.executor.middlewares.filter(
    (m): m is TracingMiddleware => m instanceof TracingMiddleware,
  );
}

const KEYS: ReadonlyArray<[string, unknown, unknown]> = [
  ['enabled', false, true],
  ['sampling_rate', 1.0, 0.25],
  ['strategy', 'full', 'error_first'],
  ['exporter', 'stdout', 'otlp'],
  ['otlp_endpoint', null, 'http://collector:4318/v1/traces'],
];

describe('the five keys are readable, writable and validating alike', () => {
  it.each(KEYS)('%s has its canonical default', (leaf, def) => {
    const tracing = (DEFAULTS as Record<string, Record<string, Record<string, unknown>>>)[
      'observability'
    ]?.['tracing'];
    expect(tracing?.[leaf as string]).toEqual(def);
  });

  it.each(KEYS)('%s round-trips from a document', (leaf, _def, written) => {
    const config = new Config({
      version: '1.0', project: { name: 'p' },
      observability: { tracing: { [leaf as string]: written } },
    });
    expect(config.get(`observability.tracing.${leaf}`)).toEqual(written);
  });

  it.each(KEYS)('%s round-trips through set()', (leaf, _def, written) => {
    const config = Config.fromDefaults();
    config.set(`observability.tracing.${leaf}`, written);
    expect(config.get(`observability.tracing.${leaf}`)).toEqual(written);
  });

  it.each(KEYS)('%s is accepted under strict mode', (leaf, _def, written) => {
    // `strategy` and `otlp_endpoint` were REJECTED here — the defect, exactly.
    // `otlp_endpoint` needs its partner: §10.1.1 requirement 3 makes an
    // endpoint against a non-OTLP exporter an error, and `stdout` is the
    // default. That rule has its own case below.
    const tracing: Record<string, unknown> = { [leaf as string]: written };
    if (leaf === 'otlp_endpoint' && written !== null) tracing['exporter'] = 'otlp';
    const config = new Config({
      version: '1.0', project: { name: 'p' },
      _config: { strict: true },
      observability: { tracing },
    });
    expect(() => config.validate()).not.toThrow();
  });

  it.each([
    ['strategy', 'sometimes'],
    // `in_memory` is here on purpose: §9.15.2 used to name it, and §10.1.1
    // requirement 2 forbids it as a configuration value because the in-memory
    // exporter is a test buffer nothing can read back by name.
    ['exporter', 'in_memory'],
    ['sampling_rate', 2.0],
    ['otlp_endpoint', ''],
  ])('%s rejects %s', (leaf, bad) => {
    const config = new Config({
      version: '1.0', project: { name: 'p' },
      observability: { tracing: { [leaf]: bad } },
    });
    expect(() => config.validate()).toThrow();
  });
});

describe('installation', () => {
  it('installs nothing when tracing is not configured', () => {
    // The whole blast radius: a project that does not ask for tracing is untouched.
    expect(tracingMiddlewares(makeClient())).toEqual([]);
    expect(tracingMiddlewares(makeClient({ enabled: false }))).toEqual([]);
    expect(tracingMiddlewares(makeClient({ strategy: 'off', sampling_rate: 0.5 }))).toEqual([]);
  });

  it('installs a middleware when enabled', () => {
    expect(tracingMiddlewares(makeClient({ enabled: true }))).toHaveLength(1);
  });

  it('never installs a second one from configuration', () => {
    // §10.1.1 requirement 6. Configuration installs into an empty chain, so it
    // can never be the thing that adds a second; this pins that repeated
    // construction from one Config does not accumulate.
    const config = new Config({
      version: '1.0', project: { name: 'p' },
      observability: { tracing: { enabled: true } },
    });
    for (let i = 0; i < 3; i++) {
      expect(tracingMiddlewares(new APCore({ config }))).toHaveLength(1);
    }
  });

  it('leaves a caller-supplied Executor alone', () => {
    const config = new Config({
      version: '1.0', project: { name: 'p' },
      observability: { tracing: { enabled: true } },
    });
    const client = new APCore({ config });
    const prebuilt = client.executor;
    const second = new APCore({ config, executor: prebuilt });
    // The prebuilt executor already carries the one this client installed; the
    // second client must not add another to it.
    expect(tracingMiddlewares(second)).toHaveLength(1);
  });
});

describe('the rate is measured, not asserted on a field', () => {
  async function sampledFraction(tracing: Record<string, unknown>, runs = 400): Promise<number> {
    // The exporter is swapped for an in-memory one AFTER construction, through
    // the public `setExporter` — the same door the span_exporter extension
    // uses. Nothing about the sampling configuration is touched, so what this
    // counts is the decision the CONFIG produced.
    const client = makeClient(tracing);
    const mw = tracingMiddlewares(client)[0]!;
    const collected = new InMemoryExporter();
    mw.setExporter(collected);
    for (let i = 0; i < runs; i++) await client.call('probe.echo', { n: i });
    return collected.getSpans().length / runs;
  }

  it('full samples everything', async () => {
    expect(await sampledFraction({ enabled: true, strategy: 'full', sampling_rate: 0.1 })).toBe(1.0);
  });

  it('off samples nothing', async () => {
    expect(await sampledFraction({ enabled: true, strategy: 'off', sampling_rate: 1.0 })).toBe(0.0);
  });

  it('proportional samples at the configured rate', async () => {
    // An operator asking for 10% gets 10%. Before this change they got 100%,
    // because `full` is the default strategy and short-circuits ahead of the
    // rate — which is why wiring `sampling_rate` alone changed nothing.
    const fraction = await sampledFraction(
      { enabled: true, strategy: 'proportional', sampling_rate: 0.1 }, 2000,
    );
    expect(fraction).toBeGreaterThan(0.05);
    expect(fraction).toBeLessThan(0.16);
  });
});

describe('the exporter, by name', () => {
  it('defaults to stdout', () => {
    const mw = tracingMiddlewares(makeClient({ enabled: true }))[0]!;
    expect((mw as unknown as { _exporter: unknown })._exporter).toBeInstanceOf(StdoutExporter);
  });

  it('passes otlp_endpoint to the exporter', () => {
    const mw = tracingMiddlewares(makeClient({
      enabled: true, exporter: 'otlp',
      otlp_endpoint: 'http://collector.internal:4318/v1/traces',
    }))[0]!;
    const exporter = (mw as unknown as { _exporter: { _endpoint: string } })._exporter;
    expect(exporter._endpoint).toContain('collector.internal');
  });

  it('uses the specified default endpoint when otlp_endpoint is null', () => {
    const mw = tracingMiddlewares(makeClient({ enabled: true, exporter: 'otlp' }))[0]!;
    const exporter = (mw as unknown as { _exporter: { _endpoint: string } })._exporter;
    expect(exporter._endpoint).toBe(DEFAULT_OTLP_ENDPOINT);
  });

  it('rejects an endpoint set against a non-OTLP exporter, at load', () => {
    // §10.1.1 requirement 3 — not a silent no-op. An endpoint written down and
    // read by nothing is the shape of every defect apcore#118 found.
    expect(() =>
      makeClient({ enabled: true, exporter: 'stdout', otlp_endpoint: 'http://x:4318' }),
    ).toThrow(ConfigError);
  });

  it('warns for jaeger, installs nothing, and substitutes nothing', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const client = makeClient({ enabled: true, exporter: 'jaeger' });
    const hits = warn.mock.calls.map((c) => String(c[0])).filter((l) => l.includes('jaeger'));
    warn.mockRestore();
    expect(tracingMiddlewares(client)).toEqual([]);
    expect(hits).toHaveLength(1);
    expect(hits[0]).toContain('otlp');
  });
});

describe('§9.2.4 — the withdrawal is cancelled', () => {
  it.each(['enabled', 'sampling_rate', 'exporter', 'strategy', 'otlp_endpoint'])(
    '%s does not warn as deprecated',
    (leaf) => {
      const value = {
        enabled: true, sampling_rate: 0.5, exporter: 'stdout',
        strategy: 'off', otlp_endpoint: null,
      }[leaf];
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      new Config({
        version: '1.0', project: { name: 'p' },
        observability: { tracing: { [leaf]: value } },
      }).validate();
      const hits = warn.mock.calls.map((c) => String(c[0])).filter((l) => l.includes('9.2.4'));
      warn.mockRestore();
      expect(hits).toEqual([]);
    },
  );
});
