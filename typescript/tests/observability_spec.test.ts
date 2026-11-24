/**
 * Spec-traced contract tests for the apcore Observability System (TypeScript SDK).
 *
 * Source spec: apcore/docs/features/observability.md
 * Mirrors the canonical Python suite:
 *   apcore-python/tests/test_observability_spec.py
 *
 * Contracts under test (verbatim `## Contract:` blocks from the spec):
 *   1. `Tracer.start_span`         — MISSING SYMBOL in apcore-typescript (no `Tracer`
 *                                    export; the SDK ships `createSpan` / `Span` /
 *                                    `TracingMiddleware` instead). Documented as skips.
 *   2. `MetricsEmitter.record`     — MISSING SYMBOL in apcore-typescript (no
 *                                    `MetricsEmitter` export; the SDK ships
 *                                    `MetricsCollector.increment` / `observe`). Skips.
 *   3. `PrometheusExporter.export` — PRESENT and fully exercised.
 *
 * Each `it(...)` name carries the verbatim clause id formatted
 * `observability.<method>.<kind>.<detail>` so cross-language diffs line up
 * row-for-row with the Python and Rust suites.
 *
 * These tests are READ-ONLY contract verification — they never modify src/.
 */

import { describe, it, expect } from 'vitest';
import { MetricsCollector } from '../src/observability/metrics.js';
import { PrometheusExporter } from '../src/observability/prometheus-exporter.js';
import * as Observability from '../src/observability/index.js';

// ---------------------------------------------------------------------------
// Missing-symbol detection (keeps the whole file importable / runnable).
//
// The spec's first two contracts name classes (`Tracer`, `MetricsEmitter`)
// that this SDK does not ship. We probe the barrel export dynamically so an
// absent symbol degrades to a skip rather than a compile/import failure.
// ---------------------------------------------------------------------------
const Tracer = (Observability as Record<string, unknown>)['Tracer'] as unknown;
const MetricsEmitter = (Observability as Record<string, unknown>)['MetricsEmitter'] as unknown;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function exporterWithOneCounter(): PrometheusExporter {
  const collector = new MetricsCollector();
  collector.incrementCalls('math.add', 'success');
  return new PrometheusExporter({ collector });
}

// ===========================================================================
// Contract 1: Tracer.start_span  (MISSING SYMBOL — contract gap)
// ===========================================================================

describe('Contract: Tracer.start_span', () => {
  const hasTracer = Tracer != null;
  const maybe = hasTracer ? it : it.skip;

  maybe('observability.start_span.input.name.empty: missing symbol Tracer (contract gap)', () => {
    expect(hasTracer).toBe(true);
  });

  maybe('observability.start_span.property.async: missing symbol Tracer (contract gap)', () => {
    expect(hasTracer).toBe(true);
  });

  maybe('observability.start_span.property.thread_safe: missing symbol Tracer (contract gap)', () => {
    expect(hasTracer).toBe(true);
  });

  maybe('observability.start_span.property.pure: missing symbol Tracer (contract gap)', () => {
    expect(hasTracer).toBe(true);
  });
});

// ===========================================================================
// Contract 2: MetricsEmitter.record  (MISSING SYMBOL — contract gap)
// ===========================================================================

describe('Contract: MetricsEmitter.record', () => {
  const hasEmitter = MetricsEmitter != null;
  const maybe = hasEmitter ? it : it.skip;

  maybe('observability.record.input.metric_name.registered: missing symbol MetricsEmitter (contract gap)', () => {
    expect(hasEmitter).toBe(true);
  });

  maybe('observability.record.property.async: missing symbol MetricsEmitter (contract gap)', () => {
    expect(hasEmitter).toBe(true);
  });

  maybe('observability.record.property.thread_safe: missing symbol MetricsEmitter (contract gap)', () => {
    expect(hasEmitter).toBe(true);
  });

  maybe('observability.record.property.pure: missing symbol MetricsEmitter (contract gap)', () => {
    expect(hasEmitter).toBe(true);
  });
});

// ===========================================================================
// Contract 3: PrometheusExporter.export  (PRESENT — fully exercised)
//
// In apcore-typescript the `collector` from the contract's ### Inputs is
// supplied at construction (`new PrometheusExporter({ collector })`), and
// `export()` takes no arguments and returns the Prometheus text string.
// ===========================================================================

describe('Contract: PrometheusExporter.export', () => {
  it('observability.export.input.collector.required: collector is required and its live data is rendered', () => {
    // Negative: the required `collector` arg cannot be omitted. The TS
    // constructor dereferences `options.collector`, so calling with no
    // argument throws a TypeError (mirrors Python's pytest.raises(TypeError)).
    expect(() => new (PrometheusExporter as unknown as new () => PrometheusExporter)()).toThrow(TypeError);

    // Positive: a supplied collector's data is what `export()` renders.
    const collector = new MetricsCollector();
    collector.incrementCalls('math.add', 'success');
    const exporter = new PrometheusExporter({ collector });
    const text = exporter.export();
    expect(text).toContain('apcore_module_calls_total');
    expect(text).toContain('module_id="math.add"');
    expect(text).toContain('status="success"');
  });

  it('observability.export.error.none: export over empty collector returns string without throwing', () => {
    const exporter = new PrometheusExporter({ collector: new MetricsCollector() });
    const text = exporter.export();
    expect(typeof text).toBe('string');
  });

  it('observability.export.returns.prometheus_text: returns UTF-8 string with HELP/TYPE comment lines', () => {
    const exporter = exporterWithOneCounter();
    const text = exporter.export();
    expect(typeof text).toBe('string');
    // Round-trips through UTF-8 without loss.
    expect(Buffer.from(text, 'utf-8').toString('utf-8')).toBe(text);
    // Prometheus exposition comment lines are present for a known metric.
    expect(text).toContain('# HELP apcore_module_calls_total');
    expect(text).toContain('# TYPE apcore_module_calls_total counter');
  });

  it('observability.export.property.async: export() is synchronous, returns a concrete string (not a Promise)', () => {
    const exporter = exporterWithOneCounter();
    const result = exporter.export();
    expect(typeof result).toBe('string');
    expect(result).not.toBeInstanceOf(Promise);
  });

  it('observability.export.property.thread_safe: >=8 concurrent exports yield identical snapshots, no throw', async () => {
    const exporter = exporterWithOneCounter();
    const doExport = async (): Promise<string> => {
      await Promise.resolve();
      return exporter.export();
    };
    const results = await Promise.all(Array.from({ length: 12 }, () => doExport()));
    expect(results.length).toBe(12);
    const first = results[0];
    expect(results.every((r) => r === first)).toBe(true);
    expect(first).toContain('apcore_module_calls_total');
  });

  it('observability.export.property.pure: export() does not mutate collector but reflects live state', () => {
    const collector = new MetricsCollector();
    collector.incrementCalls('math.add', 'success');
    const exporter = new PrometheusExporter({ collector });

    const before = JSON.stringify(collector.snapshot());
    const first = exporter.export();
    const after = JSON.stringify(collector.snapshot());
    // export() is a query: it does not mutate collector state.
    expect(before).toBe(after);
    expect(first).toContain('math.add');

    // Live-state coupling: a new module appears on the next export.
    collector.incrementCalls('math.sub', 'success');
    const second = exporter.export();
    expect(second).toContain('module_id="math.sub"');
    expect(first).not.toContain('module_id="math.sub"');
  });

  it('observability.export.property.idempotent: two successive calls on unchanged state are identical', () => {
    const exporter = exporterWithOneCounter();
    const first = exporter.export();
    const second = exporter.export();
    expect(first).toBe(second);
    expect(first).not.toBe('');
  });
});
