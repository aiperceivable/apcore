/**
 * Drive `tracing_from_config.json` — §10.1.1 (#118 D-68 C').
 *
 * Every case goes through `new APCore({ config })`. A driver that called
 * `buildTracingMiddleware` directly would prove the builder works, which was
 * never in doubt; what was inert for the whole life of these keys is the step
 * before it — no SDK extracted `observability.tracing.*` from a `Config` and
 * installed anything.
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { describe, it, expect, vi } from 'vitest';

import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { ConfigError } from '../src/errors.js';
import {
  OTLPExporter,
  StdoutExporter,
  TracingMiddleware,
} from '../src/observability/tracing.js';
import { findFixturesRoot } from './spec-repo.js';

interface TracingCase {
  readonly id: string;
  readonly description: string;
  readonly input: { readonly config: Record<string, unknown> };
  readonly expected: Record<string, unknown>;
}

const fixture: { test_cases: readonly TracingCase[] } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'tracing_from_config.json'), 'utf-8'),
);

describe('tracing_from_config.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, () => {
      const expected = testCase.expected;
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      let installed: TracingMiddleware[] = [];
      let error: unknown = null;
      let warnings: string[] = [];
      try {
        // Validate, then construct — the order a real deployment uses.
        // `new Config(document)` does not validate; `Config.load` does, and
        // that is where `expected.loads` is decided.
        const config = new Config({ ...testCase.input.config });
        config.validate();
        const client = new APCore({ config });
        installed = client.executor.middlewares.filter(
          (m): m is TracingMiddleware => m instanceof TracingMiddleware,
        );
      } catch (e) {
        error = e;
      } finally {
        warnings = warn.mock.calls.map((c) => String(c[0]));
        warn.mockRestore();
      }

      if (expected['loads'] === false) {
        expect(error).toBeInstanceOf(ConfigError);
        expect((error as ConfigError).code).toBe(expected['error_code']);
        expect(String(error)).toContain(expected['error_message_contains']);
        return;
      }
      expect(error).toBeNull();

      if ('tracing_middleware_count' in expected) {
        expect(installed).toHaveLength(expected['tracing_middleware_count'] as number);
      }

      const kind = expected['exporter_kind'] as string | undefined;
      if (kind !== undefined) {
        if (installed.length === 0 && expected['otlp_may_be_unavailable'] === true) {
          // §10.1.1 requirement 4: refusing to install rather than installing a
          // middleware that discards every span IS the conformant answer.
          return;
        }
        const exporter = (installed[0] as unknown as { _exporter: unknown })._exporter;
        expect(exporter).toBeInstanceOf(kind === 'stdout' ? StdoutExporter : OTLPExporter);
      }

      const endpoint = expected['otlp_endpoint'] as string | undefined;
      if (endpoint !== undefined && installed.length > 0) {
        const exporter = (installed[0] as unknown as { _exporter: { _endpoint: string } })._exporter;
        expect(exporter._endpoint).toBe(endpoint);
      }

      if ('sampling_strategy' in expected) {
        expect((installed[0] as unknown as { _samplingStrategy: string })._samplingStrategy).toBe(
          expected['sampling_strategy'],
        );
      }
      if ('sampling_rate' in expected) {
        expect((installed[0] as unknown as { _samplingRate: number })._samplingRate).toBe(
          expected['sampling_rate'],
        );
      }

      const naming = expected['warns_naming'] as string | undefined;
      if (naming !== undefined && expected['deprecation_warning'] !== true) {
        expect(warnings.filter((l) => l.includes(naming))).toHaveLength(1);
      }
    });
  }
});

describe('tracing_from_config.json — the §9.2.4 half', () => {
  // The notice fires on `Config.load`, not on `new Config(document)`. Split out
  // for that reason: reading it off the constructor would report "no warning"
  // for every case and pass the positive half vacuously.
  const cases = fixture.test_cases.filter((c) => 'deprecation_warning' in c.expected);

  for (const testCase of cases) {
    it(testCase.id, () => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-tracing-'));
      const file = path.join(dir, 'apcore.yaml');
      fs.writeFileSync(file, JSON.stringify(testCase.input.config), 'utf-8');
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      Config.load(file);
      const notices = warn.mock.calls.map((c) => String(c[0])).filter((l) => l.includes('9.2.4'));
      warn.mockRestore();

      if (testCase.expected['deprecation_warning'] === true) {
        expect(notices).toHaveLength(1);
        expect(notices[0]).toContain(testCase.expected['warns_naming']);
      } else {
        expect(notices).toEqual([]);
      }
    });
  }
});
