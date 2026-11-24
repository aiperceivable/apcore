/**
 * Cross-language conformance driver for `usage_exporter.json`
 * (Issue #45 §3, D-55 — docs/features/observability.md#usageexporter-push-style).
 *
 * Fixture source: apcore/conformance/fixtures/usage_exporter.json (canonical).
 * The fixture carries no `driver_contract` block; its `description` is the
 * contract: every SDK ships a `UsageExporter` interface, a `NoopUsageExporter`
 * that drops summaries, and a `PeriodicUsageExporter` that polls
 * `UsageCollector.getSummary()` on a timer and pushes into `exporter.export()`.
 * `stop()` halts the loop, awaits `exporter.shutdown()`, and is idempotent.
 *
 * RESOLVED DIVERGENCE (case `periodic_usage_exporter_stop_is_idempotent_and_drains`)
 * ---------------------------------------------------------------------------------
 * The fixture requires `shutdown_call_count == 1` after two `stop()` calls.
 * `PeriodicUsageExporter.stop()` used to call `this.exporter.shutdown()`
 * unconditionally, so N stop() calls produced N shutdown() calls; it now guards
 * on the running flag, making shutdown() exactly-once per start(). The case is
 * driven normally below — there is no `it.fails` marker any more.
 *
 * NOT PINNED BY THIS FIXTURE: `stop()` before `start()`. The fixture's
 * operation list is start -> wait_ms -> stop -> stop, so the never-started case
 * is agreed across the SDKs by convention rather than by contract. It is
 * covered as a unit test in tests/observability/test-usage-exporter.test.ts,
 * and a fixture case is worth adding upstream — apcore-rust had the same
 * unconditional-shutdown defect and no fixture case caught it either.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import {
  NoopUsageExporter,
  PeriodicUsageExporter,
  UsageCollector,
  type UsageExporter,
} from '../src/observability/index.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading (same shape as tests/conformance-schema-keyword-parity.test.ts)
// ---------------------------------------------------------------------------

interface UsageExporterCase {
  readonly id: string;
  readonly scenario: string;
  readonly exporter_type: string;
  readonly config?: { readonly interval_seconds: number; readonly ticks?: number };
  readonly usage_records?: ReadonlyArray<{
    readonly module_id: string;
    readonly caller_id: string;
    readonly latency_ms: number;
    readonly success: boolean;
  }>;
  readonly operations?: ReadonlyArray<Record<string, unknown>>;
  readonly expected: Record<string, unknown>;
}

function loadFixture(name: string): { description: string; test_cases: readonly UsageExporterCase[] } {
  return JSON.parse(fs.readFileSync(path.join(findFixturesRoot(), `${name}.json`), 'utf-8'));
}

const fixture = loadFixture('usage_exporter');

function caseById(id: string): UsageExporterCase {
  const tc = fixture.test_cases.find((c) => c.id === id);
  if (tc === undefined) {
    throw new Error(
      `usage_exporter.json no longer contains case '${id}'. The fixture is canonical — ` +
        'update this driver to match it, do not edit the fixture.',
    );
  }
  return tc;
}

/** Exporter that records everything it is handed, for assertion against `expected`. */
class RecordingExporter implements UsageExporter {
  readonly exported: unknown[] = [];
  shutdownCalls = 0;

  export(summary: Record<string, unknown>): void {
    this.exported.push(summary);
  }

  shutdown(): void {
    this.shutdownCalls += 1;
  }
}

describe('Conformance: UsageExporter push interface (usage_exporter.json)', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // -------------------------------------------------------------------------
  it('noop_usage_exporter_drops_summary', () => {
    const tc = caseById('noop_usage_exporter_drops_summary');
    const exportOp = tc.operations!.find((op) => op['op'] === 'export')!;
    const summary = exportOp['summary'] as unknown[];

    // "drops the summary" is asserted structurally: a dropped payload is never
    // read. The Proxy records every property access, so forwarding,
    // serialising, or even inspecting the summary would populate `touched`.
    const touched: (string | symbol)[] = [];
    const observedSummary = new Proxy(summary, {
      get(target, prop, receiver) {
        touched.push(prop);
        return Reflect.get(target, prop, receiver);
      },
    });

    const errors: unknown[] = [];
    const exporter = new NoopUsageExporter();
    let shutdownCompleted = false;
    try {
      // `export` is typed for object payloads; the fixture summary is an array
      // of module records, which is what UsageCollector.getSummary() yields.
      exporter.export(observedSummary as unknown as Record<string, unknown>);
      exporter.shutdown();
      shutdownCompleted = true;
    } catch (err) {
      errors.push(err);
    }

    expect(touched, 'NoopUsageExporter must drop the summary, not read it').toEqual(
      tc.expected['calls_observed'],
    );
    expect(shutdownCompleted).toBe(tc.expected['shutdown_completed']);
    expect(errors).toEqual(tc.expected['errors']);
  });

  // -------------------------------------------------------------------------
  it('periodic_usage_exporter_pushes_at_interval', async () => {
    const tc = caseById('periodic_usage_exporter_pushes_at_interval');
    const intervalMs = tc.config!.interval_seconds * 1000;
    const ticks = tc.config!.ticks!;

    const collector = new UsageCollector();
    for (const rec of tc.usage_records!) {
      collector.record(rec.module_id, rec.caller_id, rec.latency_ms, rec.success);
    }

    const exporter = new RecordingExporter();
    const periodic = new PeriodicUsageExporter(collector, exporter, intervalMs);
    periodic.start();

    // Nothing is exported synchronously on start(): the first push is one full
    // interval later.
    expect(exporter.exported.length).toBe(0);
    await vi.advanceTimersByTimeAsync(intervalMs * ticks);

    expect(exporter.exported.length).toBe(tc.expected['export_call_count']);
    for (const payload of exporter.exported) {
      expect(
        JSON.stringify(payload),
        'every pushed summary must carry the recorded module',
      ).toContain(tc.expected['each_export_summary_includes'] as string);
    }

    await periodic.stop();
    expect(exporter.shutdownCalls >= 1).toBe(tc.expected['shutdown_completed_after_stop']);
  });

  // -------------------------------------------------------------------------
  // The case is split: everything the SDK satisfies is asserted here, and the
  // single divergent expectation (`shutdown_call_count`) is pinned separately
  // below. Both halves assert the fixture's own values — neither weakens it.
  it('periodic_usage_exporter_stop_is_idempotent_and_drains (termination half)', async () => {
    const tc = caseById('periodic_usage_exporter_stop_is_idempotent_and_drains');
    const intervalMs = tc.config!.interval_seconds * 1000;
    const waitMs = tc.operations!.find((op) => op['op'] === 'wait_ms')!['duration_ms'] as number;

    const collector = new UsageCollector();
    const exporter = new RecordingExporter();
    const periodic = new PeriodicUsageExporter(collector, exporter, intervalMs);

    const errors: unknown[] = [];
    periodic.start();
    await vi.advanceTimersByTimeAsync(waitMs);
    const exportsAtStop = exporter.exported.length;

    // Two stop() calls, exactly as the fixture's operation list prescribes.
    try {
      await periodic.stop();
      await periodic.stop();
    } catch (err) {
      errors.push(err);
    }

    expect(errors, 'stop() must not raise, and must be safe to call twice').toEqual(
      tc.expected['errors'],
    );
    expect(errors.length === 0).toBe(tc.expected['stop_idempotent']);

    // background_task_terminated: the timer is gone, so no further ticks fire.
    await vi.advanceTimersByTimeAsync(intervalMs * 10);
    expect(exporter.exported.length === exportsAtStop).toBe(
      tc.expected['background_task_terminated'],
    );
  });

  // Was a KNOWN DIVERGENCE xfail: TS called exporter.shutdown() once per
  // stop() call, so a second stop() drove shutdown_call_count to 2. Fixed in
  // src/observability/usage-exporter.ts by guarding on `_running`, which is
  // what apcore-python and apcore-rust already did.
  it(
    'periodic_usage_exporter_stop_is_idempotent_and_drains (shutdown_call_count)',
    async () => {
      const tc = caseById('periodic_usage_exporter_stop_is_idempotent_and_drains');
      const intervalMs = tc.config!.interval_seconds * 1000;
      const waitMs = tc.operations!.find((op) => op['op'] === 'wait_ms')!['duration_ms'] as number;

      const collector = new UsageCollector();
      const exporter = new RecordingExporter();
      const periodic = new PeriodicUsageExporter(collector, exporter, intervalMs);

      periodic.start();
      await vi.advanceTimersByTimeAsync(waitMs);
      await periodic.stop();
      await periodic.stop();

      expect(exporter.shutdownCalls).toBe(tc.expected['shutdown_call_count']);
    },
  );

  // -------------------------------------------------------------------------
  it('drives every fixture case', () => {
    // Guards against the fixture gaining a case this driver silently ignores.
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'noop_usage_exporter_drops_summary',
      'periodic_usage_exporter_pushes_at_interval',
      'periodic_usage_exporter_stop_is_idempotent_and_drains',
    ]);
  });
});
