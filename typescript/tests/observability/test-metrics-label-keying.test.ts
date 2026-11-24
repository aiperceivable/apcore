/**
 * OBS-006 — a metric series is keyed on its labels, not on a flattened string
 * that a label value can forge.
 *
 * `MetricsCollector` keyed counters and histograms on `name|k=v,k=v` and
 * parsed that string back at export time. A label value containing `,`, `=`
 * or `|` therefore:
 *   - fabricated labels on export — `{region: "eu,west"}` came out as
 *     `region="eu", west=""`;
 *   - was truncated rather than limited, because `split('=', 2)` DROPS
 *     everything past the second field in JavaScript;
 *   - collided — `{a: "x,b=y"}` and `{a: "x", b: "y"}` flattened to the same
 *     key, so two distinct series silently aggregated into one.
 *
 * apcore-python keys on a sorted tuple and apcore-rust on a `BTreeMap`.
 */

import { describe, it, expect } from 'vitest';
import { MetricsCollector } from '../../src/observability/metrics.js';

function counters(m: MetricsCollector): Record<string, number> {
  return (m.snapshot()['counters'] as Record<string, number>) ?? {};
}

describe('metric series are keyed on the labels themselves (OBS-006)', () => {
  it('two distinct label sets that flatten alike stay distinct', () => {
    const m = new MetricsCollector();
    m.increment('probe_total', { a: 'x,b=y' });
    m.increment('probe_total', { a: 'x', b: 'y' });

    const values = Object.values(counters(m));
    expect(values).toHaveLength(2);
    expect(values.every((v) => v === 1)).toBe(true);
  });

  it('a comma in a label value does not fabricate a second label', () => {
    const m = new MetricsCollector();
    m.increment('probe_total', { region: 'eu,west' });

    const text = m.exportPrometheus();
    expect(text).toContain('region="eu,west"');
    expect(text).not.toContain('west=""');
  });

  it('an equals sign in a label value is preserved whole, not truncated', () => {
    const m = new MetricsCollector();
    m.increment('probe_total', { token: 'a=b=c' });

    const text = m.exportPrometheus();
    expect(text).toContain('token="a=b=c"');
  });

  it('a pipe in a label value does not corrupt the metric name', () => {
    const m = new MetricsCollector();
    m.increment('probe_total', { path: 'a|b' });

    const text = m.exportPrometheus();
    expect(text).toContain('probe_total{path="a|b"} 1');
  });

  it('histogram labels survive the same round trip', () => {
    const m = new MetricsCollector();
    m.observe('probe_seconds', { region: 'eu,west' }, 0.01);

    const text = m.exportPrometheus();
    expect(text).toContain('probe_seconds_count{region="eu,west"} 1');
    expect(text).toContain('probe_seconds_sum{region="eu,west"} 0.01');
    expect(text).not.toContain('west=""');
  });

  it('two histogram series that flatten alike do not aggregate', () => {
    const m = new MetricsCollector();
    m.observe('probe_seconds', { a: 'x,b=y' }, 0.01);
    m.observe('probe_seconds', { a: 'x', b: 'y' }, 0.02);

    const counts = (m.snapshot()['histograms'] as { counts: Record<string, number> }).counts;
    expect(Object.keys(counts)).toHaveLength(2);
    expect(Object.values(counts).every((v) => v === 1)).toBe(true);
  });

  it('the ordinary key shape is unchanged', () => {
    // metrics-utils.ts matches on `|module_id=<id>` and `status=error`
    // substrings, so the common case must keep its exact spelling.
    const m = new MetricsCollector();
    m.incrementCalls('executor.email.send', 'error');
    expect(
      counters(m)['apcore_module_calls_total|module_id=executor.email.send,status=error'],
    ).toBe(1);
  });
});
