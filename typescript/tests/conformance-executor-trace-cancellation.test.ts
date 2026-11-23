/**
 * Cross-language conformance driver for executor_trace_cancellation.json
 * (A-D-001 trace-variant cancellation short-circuit).
 *
 * Fixture source: apcore/conformance/fixtures/executor_trace_cancellation.json
 * (single source of truth). See that fixture's `description` and per-case
 * `notes` for the driver contract.
 *
 * When the pipeline raises ExecutionCancelledError mid-execution, the trace
 * variant (callWithTrace) MUST propagate it directly (code EXECUTION_CANCELLED)
 * and MUST NOT route it through the on_error middleware chain — an on_error
 * middleware that would otherwise recover MUST NOT be able to suppress the
 * cancellation.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Type } from '@sinclair/typebox';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { Middleware } from '../src/middleware/base.js';
import { ExecutionCancelledError } from '../src/cancel.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadFixture(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

/** on_error middleware that records invocation and would otherwise recover. */
class RecordingRecoverMiddleware extends Middleware {
  onErrorInvoked = false;
  override onError(): Record<string, unknown> | null {
    this.onErrorInvoked = true;
    return { recovered: true };
  }
}

describe('Conformance: callWithTrace cancellation bypasses on_error (A-D-001)', () => {
  const fixture = loadFixture('executor_trace_cancellation');

  fixture.test_cases.forEach((tc: any) => {
    it(tc.id, async () => {
      const registry = new Registry();
      registry.register('test.cancel.trace', {
        id: 'test.cancel.trace',
        description: 'Module whose execute raises ExecutionCancelledError',
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        execute: () => {
          throw new ExecutionCancelledError('cancelled mid-execution');
        },
      });

      const mw = new RecordingRecoverMiddleware();
      const executor = new Executor({ registry });
      executor.use(mw);

      let thrown: unknown = null;
      try {
        await executor.callWithTrace('test.cancel.trace', {});
      } catch (e) {
        thrown = e;
      }

      // (1) cancellation propagates with code EXECUTION_CANCELLED, not recovered.
      expect(thrown).toBeInstanceOf(ExecutionCancelledError);
      expect((thrown as ExecutionCancelledError).code).toBe(tc.expected_error);

      // (2) the recording on_error middleware was NOT invoked (bypassed).
      expect(mw.onErrorInvoked).toBe(tc.expected_on_error_invoked);
    });
  });
});
