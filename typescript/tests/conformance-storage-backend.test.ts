/**
 * Cross-language conformance driver for `storage_backend.json`
 * (Issue #43 §1, D-39 — docs/features/observability.md, "Pluggable
 * Observability Storage").
 *
 * Fixture source: apcore/conformance/fixtures/storage_backend.json (canonical).
 * The fixture has no `driver_contract` block; its `description` is the
 * contract: the four-method surface (`save` / `get` / `list` / `delete`),
 * namespace isolation, and idempotent delete, as implemented by the bundled
 * default backend — `InMemoryStorageBackend` here.
 *
 * TYPE-SURFACE NOTE (not a behavioural divergence): the fixture stores scalar
 * values ("ENT-1", "1", "v1"). apcore-rust types the value as
 * `serde_json::Value` and accepts them directly; apcore-python types it `dict`
 * and apcore-typescript types it `Record<string, unknown>`
 * (src/observability/storage.ts:23-43), so the scalars are cast at the call
 * boundary below. Runtime behaviour — and therefore every assertion — is
 * exactly what the fixture states; only the static type is narrower than the
 * fixture assumes. Reported, not worked around.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import {
  InMemoryStorageBackend,
  STORAGE_NAMESPACE_ERROR_HISTORY,
  type StorageBackend,
} from '../src/observability/storage.js';
import { MetricsCollector } from '../src/observability/metrics.js';
import { UsageCollector } from '../src/observability/usage.js';
import { ErrorHistory } from '../src/observability/error-history.js';
import { ModuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

interface StorageOp {
  readonly op: 'save' | 'get' | 'list' | 'delete';
  readonly namespace: string;
  readonly key?: string;
  readonly value?: unknown;
  readonly prefix?: string;
}

interface StorageCase {
  readonly id: string;
  readonly description: string;
  readonly input?: { readonly operations: readonly StorageOp[] };
  /** D-113: the case drives the bundled COLLECTORS, not the backend directly. */
  readonly collectors?: readonly string[];
  readonly omit_backend?: boolean;
  readonly expected: Record<string, unknown>;
}

function loadFixture(name: string): { description: string; test_cases: readonly StorageCase[] } {
  return JSON.parse(fs.readFileSync(path.join(findFixturesRoot(), `${name}.json`), 'utf-8'));
}

const fixture = loadFixture('storage_backend');

// ---------------------------------------------------------------------------
// Operation interpreter
// ---------------------------------------------------------------------------

interface RunResult {
  /** Values returned by each `get` op, in order. */
  readonly gets: unknown[];
  /** Key/value pairs returned by each `list` op, in order. */
  readonly lists: Array<Array<[string, unknown]>>;
  /** Non-null if any operation threw. */
  readonly error: unknown;
}

async function runOperations(ops: readonly StorageOp[]): Promise<RunResult> {
  const backend = new InMemoryStorageBackend();
  const gets: unknown[] = [];
  const lists: Array<Array<[string, unknown]>> = [];
  let error: unknown = null;

  try {
    for (const op of ops) {
      switch (op.op) {
        case 'save':
          // See the TYPE-SURFACE NOTE in the file header: the fixture's scalar
          // values are stored verbatim; only the static type is narrower.
          await backend.save(op.namespace, op.key!, op.value as Record<string, unknown>);
          break;
        case 'get':
          gets.push(await backend.get(op.namespace, op.key!));
          break;
        case 'list':
          lists.push(await backend.list(op.namespace, op.prefix));
          break;
        case 'delete':
          await backend.delete(op.namespace, op.key!);
          break;
        default:
          throw new Error(
            `storage_backend.json uses operation '${(op as StorageOp).op}' this driver does not ` +
              'implement. The fixture is canonical — extend the driver.',
          );
      }
    }
  } catch (err) {
    error = err;
  }

  return { gets, lists, error };
}

/**
 * Assertions for each expectation key the fixture uses. Keeping them in a table
 * lets the driver fail loudly (rather than silently pass) if the fixture gains
 * an expectation nobody checks.
 */
const EXPECTATION_ASSERTIONS: Record<
  string,
  (result: RunResult, expected: unknown, tc: StorageCase) => void
> = {
  final_get_value: (result, expected) => {
    expect(result.gets.length, 'case must perform at least one get()').toBeGreaterThan(0);
    expect(result.gets[result.gets.length - 1]).toEqual(expected);
  },
  matched_keys_sorted: (result, expected) => {
    expect(result.lists.length, 'case must perform at least one list()').toBeGreaterThan(0);
    const keys = result.lists[result.lists.length - 1].map(([k]) => k).sort();
    expect(keys).toEqual(expected);
  },
  raised_error: (result, expected) => {
    expect(result.error !== null, `unexpected error: ${String(result.error)}`).toBe(expected);
  },
  errors_namespace_value: (result, expected) => {
    // Case `namespace_isolation`: first get() reads namespace "errors".
    expect(result.gets[0]).toEqual(expected);
  },
  metrics_namespace_value: (result, expected) => {
    // ...and the second reads the same key under namespace "metrics".
    expect(result.gets[1]).toEqual(expected);
  },
};

/**
 * D-113: which namespace each bundled collector writes, and the omitted default.
 *
 * Drives all named collectors through ONE backend so the assertion is the SET
 * of namespaces the surface produces, not three independent behaviours — an SDK
 * writing two of three would otherwise read as a partial gap rather than a
 * wrong surface.
 */
async function driveCollectorCase(tc: StorageCase): Promise<void> {
  if (tc.omit_backend === true) {
    // The observable is READABILITY: a record written with no backend supplied
    // is still there to read. Asserting a class name would pin this SDK's
    // spelling, which the decision does not.
    const history = new ErrorHistory();
    history.record('mod.x', new ModuleError('E1', 'boom'));
    await new Promise((r) => setTimeout(r, 20));
    const stored = await history.storage.list(STORAGE_NAMESPACE_ERROR_HISTORY);
    expect(
      stored.length > 0,
      `${tc.id}: an omitted backend must be the in-memory one, not 'no storage'`,
    ).toBe(tc.expected['records_readable']);
    return;
  }

  const seen = new Set<string>();
  const recording: StorageBackend = {
    async save(namespace: string) {
      seen.add(namespace);
    },
    async get() {
      return null;
    },
    async list() {
      return [];
    },
    async delete() {},
  };

  for (const collector of tc.collectors ?? []) {
    if (collector === 'metrics') {
      new MetricsCollector({ storage: recording }).observeDuration('mod.x', 0.01);
    } else if (collector === 'usage') {
      new UsageCollector({ storage: recording }).record('mod.x', 'caller', 12, true);
    } else if (collector === 'error_history') {
      new ErrorHistory({ storage: recording }).record('mod.x', new ModuleError('E1', 'boom'));
    } else {
      throw new Error(`${tc.id}: unknown collector '${collector}'`);
    }
  }
  await new Promise((r) => setTimeout(r, 20));

  expect([...seen].sort()).toEqual([...(tc.expected['namespaces_written'] as string[])].sort());
}

describe('Conformance: StorageBackend four-method contract (storage_backend.json)', () => {
  fixture.test_cases.forEach((tc) => {
    it(tc.id, async () => {
      if (tc.collectors !== undefined) {
        await driveCollectorCase(tc);
        return;
      }
      const result = await runOperations(tc.input!.operations);

      const keys = Object.keys(tc.expected).filter((k) => !k.startsWith('_'));
      const unhandled = keys.filter((k) => !(k in EXPECTATION_ASSERTIONS));
      expect(
        unhandled,
        `storage_backend.json case '${tc.id}' declares expectations this driver does not ` +
          'assert. The fixture is canonical — extend the driver, do not edit the fixture.',
      ).toEqual([]);

      // A case whose only op sequence threw would otherwise report a confusing
      // downstream mismatch; surface the original error first unless the case
      // deliberately asserts on `raised_error`.
      if (result.error !== null && !('raised_error' in tc.expected)) {
        throw result.error;
      }

      for (const key of keys) {
        EXPECTATION_ASSERTIONS[key](result, tc.expected[key], tc);
      }
    });
  });

  it('drives every fixture case', () => {
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'save_and_get_roundtrip',
      'list_with_prefix',
      'delete_idempotent',
      'namespace_isolation',
      'save_overwrites_existing',
      // D-113: the collector surface, not the backend contract.
      'each_collector_writes_its_declared_namespace',
      'an_omitted_backend_is_the_in_memory_one',
    ]);
  });
});
