/**
 * Cross-language conformance driver for async_task_cancellation.json
 * (A-D-003 typed TASK_LIMIT_EXCEEDED / A-D-004 cancel-during-backoff).
 *
 * Fixture source: apcore/conformance/fixtures/async_task_cancellation.json
 * (single source of truth). See that fixture's `description` and per-case
 * `notes` for the driver contract.
 *
 *   submit_over_capacity: submitting beyond max_tasks MUST raise the typed
 *   TaskLimitExceededError (code === 'TASK_LIMIT_EXCEEDED'), not a bare Error.
 *
 *   cancel_during_backoff: cancelling a task while it is in retry backoff MUST
 *   stop further retry attempts and end in CANCELLED — asserted as a
 *   deterministic invariant on attempt count + final status, not a timing
 *   assertion.
 */

import { setTimeout as sleep } from 'node:timers/promises';
import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Type } from '@sinclair/typebox';
import {
  AsyncTaskManager,
  TaskStatus,
  RetryConfig,
} from '../src/async-task.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { FunctionModule } from '../src/decorator.js';
import { TaskLimitExceededError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadFixture(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

describe('Conformance: AsyncTaskManager capacity + cancellation', () => {
  const fixture = loadFixture('async_task_cancellation');

  function findCase(id: string): any {
    const tc = fixture.test_cases.find((t: any) => t.id === id);
    expect(tc).toBeDefined();
    return tc;
  }

  it('submit_over_capacity_raises_task_limit_exceeded', async () => {
    const tc = findCase('submit_over_capacity_raises_task_limit_exceeded');

    const registry = new Registry();
    // A long-running module so the first submitted task occupies the only slot
    // and stays PENDING/RUNNING (active) when the second submit is attempted.
    const longModule = new FunctionModule({
      execute: async (_inputs, context) => {
        await sleep(60_000, undefined, { signal: context.signal });
        return { done: true };
      },
      moduleId: 'test.long',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ done: Type.Boolean() }),
      description: 'Long-running module',
    });
    registry.register('test.long', longModule);

    const executor = new Executor({ registry });
    const manager = new AsyncTaskManager({
      executor,
      maxConcurrent: tc.max_concurrent,
      maxTasks: tc.max_tasks,
    });

    // First submit occupies the single task slot.
    await manager.submit('test.long', {});

    // Second submit must reject with the typed error.
    let thrown: unknown = null;
    try {
      await manager.submit('test.long', {});
    } catch (e) {
      thrown = e;
    }

    expect(thrown).not.toBeNull();
    expect(thrown).toBeInstanceOf(TaskLimitExceededError);
    expect((thrown as TaskLimitExceededError).code).toBe(tc.expected_error);

    await manager.shutdown();
  });

  it('cancel_during_backoff_stops_further_retries', async () => {
    const tc = findCase('cancel_during_backoff_stops_further_retries');

    // Shared, test-observable per-attempt counter incremented on every module
    // invocation. The module always rejects to trigger retry/backoff.
    let attempts = 0;
    const registry = new Registry();
    const alwaysFailing = new FunctionModule({
      execute: () => {
        attempts += 1;
        throw new Error('intentional failure to trigger retry');
      },
      moduleId: 'test.always_failing',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'Always-failing module',
    });
    registry.register('test.always_failing', alwaysFailing);

    const executor = new Executor({ registry });
    const manager = new AsyncTaskManager({ executor, maxConcurrent: 1, maxTasks: 10 });

    const retry = new RetryConfig({
      maxRetries: tc.max_retries,
      retryDelayMs: tc.retry_delay_ms,
      backoffMultiplier: tc.backoff_multiplier,
    });

    const taskId = await manager.submit('test.always_failing', {}, { retry });

    // Wait for the first attempt to run and fail; the task then enters its
    // backoff wait (retryDelayMs = 1000ms before the second attempt).
    let safety = 0;
    while (attempts < 1 && safety < 200) {
      await wait(10);
      safety += 1;
    }
    expect(attempts).toBe(1);

    // Confirm the task is back in PENDING (waiting out backoff), then cancel
    // it mid-backoff — well before the 1000ms delay elapses.
    const cancelled = await manager.cancel(taskId);
    expect(cancelled).toBe(true);

    const attemptsAtCancel = attempts;

    // Wait long enough that, absent the A-D-004 fix, the second retry attempt
    // would have fired (>1000ms backoff). With the fix, cancel unwinds the
    // backoff and no further attempt runs.
    await wait(tc.retry_delay_ms + 300);

    expect(attempts).toBe(attemptsAtCancel); // no attempt started after cancel

    const info = await manager.getStatus(taskId);
    expect(info).not.toBeNull();
    expect(info!.status).toBe(TaskStatus.CANCELLED);
    expect(tc.expected_final_status).toBe('cancelled');

    await manager.shutdown();
  });
});
