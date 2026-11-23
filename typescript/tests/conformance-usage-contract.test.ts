/**
 * Cross-language conformance driver for `usage_contract.json`
 * (PROTOCOL_SPEC 6.7.1 — the value semantics no JSON Schema can assert).
 *
 * Fixture source: apcore/conformance/fixtures/usage_contract.json (canonical).
 *
 * Drives the real `system.usage.*` modules against a real `UsageCollector`,
 * per `driver_contract.path`: every divergence this fixture pins lived in the
 * sys-module layer's choice of accessor, so a driver that calls the collector's
 * own period-aware methods asserts the layer that was never wrong.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import { APCore, Registry } from '../src/index.js';
import { Config } from '../src/config.js';
import { UsageCollector, UsageMiddleware } from '../src/observability/usage.js';
import { UsageModule, UsageSummaryModule } from '../src/sys-modules/usage.js';
import { jsonSchemaToTypeBox } from '../src/schema/loader-pure.js';
import { SchemaValidator } from '../src/schema/validator.js';
import { findFixturesRoot, findSchemasRoot } from './spec-repo.js';

interface RecordSpec {
  at_offset: string;
  caller_id: string | null;
  latency_ms?: number;
  success: boolean;
}

interface Case {
  id: string;
  note: string;
  module: string;
  module_id?: string;
  latencies_ms?: number[];
  records?: RecordSpec[];
  inputs?: Record<string, unknown>;
  expected: Record<string, unknown>;
}

const fixture: { test_cases: Case[] } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'usage_contract.json'), 'utf-8'),
);

const OFFSET = /^-(\d+)([hd])$/;

// driver_contract.output_validates_against_the_canonical_schema: the same files
// the spec repo ships, not a copy. `additionalProperties: false` is the point —
// a field one SDK emits and the others do not fails here.
const SCHEMAS_DIR = findSchemasRoot();
const SCHEMA_FOR: Record<string, string> = {
  'system.usage.summary': 'sys-usage-summary.schema.json',
  'system.usage.module': 'sys-usage-module.schema.json',
};

function validateAgainstCanonicalSchema(module: string, output: Record<string, unknown>): void {
  const schema = JSON.parse(
    fs.readFileSync(path.join(SCHEMAS_DIR, SCHEMA_FOR[module]), 'utf-8'),
  );
  // Through the SDK's own conversion, which is the path a real module call
  // takes (`resolveSchema` in builtin-steps): SchemaValidator is TypeBox-based
  // and does not accept raw JSON Schema, so validating any other way would
  // exercise machinery the module path never uses.
  const result = new SchemaValidator(false).validate(output, jsonSchemaToTypeBox(schema));
  expect(result.valid, `${module} output does not satisfy the canonical schema: ${JSON.stringify(result.errors)}`).toBe(true);
}

function at(offset: string): string {
  const match = OFFSET.exec(offset);
  if (!match) throw new Error(`unsupported at_offset ${offset}`);
  const amount = Number(match[1]);
  const ms = match[2] === 'h' ? amount * 3_600_000 : amount * 86_400_000;
  return new Date(Date.now() - ms).toISOString();
}

function registryWith(moduleId: string): Registry {
  const registry = new Registry();
  registry.register(moduleId, {
    description: 'conformance target',
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: () => ({}),
  });
  return registry;
}

/** A Context carrying no caller identity. */
const noCallerContext = { callerId: null, data: {} as Record<string, unknown> };

function collectorFor(testCase: Case, moduleId: string): UsageCollector {
  const collector = new UsageCollector();
  for (const latency of testCase.latencies_ms ?? []) {
    collector.record(moduleId, 'caller-a', latency, true);
  }
  for (const record of testCase.records ?? []) {
    if (record.caller_id === null) {
      // driver_contract.unattributed_caller: a call with NO caller identity
      // must go through this SDK's usage-recording path. apcore-typescript
      // substitutes 'unknown' in UsageMiddleware; apcore-rust does it in the
      // breakdown, because its record() takes Option<&str>.
      const middleware = new UsageMiddleware(collector);
      middleware.before(moduleId, {}, noCallerContext as never);
      if (record.success) {
        middleware.after(moduleId, {}, {}, noCallerContext as never);
      } else {
        middleware.onError(moduleId, {}, new Error('conformance'), noCallerContext as never);
      }
      continue;
    }
    collector.record(
      moduleId,
      record.caller_id,
      record.latency_ms ?? 0,
      record.success,
      at(record.at_offset),
    );
  }
  return collector;
}

function run(testCase: Case): Record<string, unknown> {
  const moduleId = testCase.module_id ?? 'math.add';
  const collector = collectorFor(testCase, moduleId);
  const inputs: Record<string, unknown> = { ...(testCase.inputs ?? {}) };

  if (testCase.module === 'system.usage.summary') {
    return new UsageSummaryModule(collector).execute(inputs, null);
  }
  inputs['module_id'] ??= moduleId;
  return new UsageModule(registryWith(moduleId), collector).execute(inputs, null);
}

describe('conformance: usage_contract.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, async () => {
      const expected = testCase.expected;

      // Rejection cases assert the declared grammar. It lives in inputSchema
      // (6.7.1.1) so rejection happens at input validation with
      // SCHEMA_VALIDATION_ERROR, not inside a private parser.
      if ('error_code' in expected) {
        const pattern = new UsageSummaryModule(new UsageCollector()).inputSchema.properties.period
          .pattern;
        expect(pattern).toBe('^[1-9][0-9]*[hd]$');
        expect(
          new RegExp(pattern).test(testCase.inputs?.['period'] as string),
          `${testCase.id}: fixture expects this period to be rejected`,
        ).toBe(false);

        // ...and then assert what the case actually SAYS. Both checks above are
        // about the schema; neither reads `expected.error_code`, so this branch
        // passed whatever wire code the fixture declared — the case ran and its
        // expectation asserted nothing. Found by `check_case_pinning.py`:
        // mutating the code left all three SDKs green, which is how they came
        // to share the same proxy.
        //
        // Rejection happens at input validation (§6.7.1.1), the pipeline's job
        // rather than `execute`'s, so this has to go through a real client.
        const client = new APCore({
          config: new Config({
            version: '1.0',
            project: { name: 'usage-contract' },
            sys_modules: { enabled: true, usage: { enabled: true } },
          }),
        });
        await expect(
          client.call(testCase.module, { ...(testCase.inputs ?? {}) }),
        ).rejects.toMatchObject({ code: expected['error_code'] });
        return;
      }

      const result = run(testCase);
      validateAgainstCanonicalSchema(testCase.module, result);

      if ('caller_ids' in expected) {
        const ids = (result['callers'] as Array<{ caller_id: string }>).map((c) => c.caller_id);
        expect(ids, `${testCase.id} — ${testCase.note}`).toEqual(expected['caller_ids']);
      }

      if ('hourly_distribution_length' in expected) {
        const hourly = result['hourly_distribution'] as Array<Record<string, unknown>>;
        expect(hourly.length).toBe(expected['hourly_distribution_length']);
        const keyRe = new RegExp(expected['hourly_distribution_key_format'] as string);
        for (const entry of hourly) {
          expect(keyRe.test(entry['hour'] as string), `hour key ${entry['hour']}`).toBe(true);
        }
        const total = hourly.reduce((n, e) => n + (e['call_count'] as number), 0);
        expect(total).toBe(expected['hourly_distribution_total_calls']);
        if (expected['hourly_distribution_sorted_ascending']) {
          const hours = hourly.map((e) => e['hour'] as string);
          expect(hours).toEqual([...hours].sort());
        }
      }

      for (const [field, want] of Object.entries(expected)) {
        if (field.startsWith('hourly_distribution_') || field === 'caller_ids') continue;
        const matcher = expect(result[field], `${testCase.id}: ${field} — ${testCase.note}`);
        if (want !== null && typeof want === 'object') {
          matcher.toEqual(want);
        } else {
          matcher.toBe(want);
        }
      }
    });
  }
});
