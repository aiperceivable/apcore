/**
 * Regression for A-D-006 — when an EventEmitter is wired into the Executor, a
 * stream Phase-3 (post-stream output validation) failure MUST emit an
 * `apcore.stream.post_validation_failed` event (chunks are already delivered, so
 * the error is not re-raised). Mirrors apcore-python executor.py.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Executor } from '../src/executor.js';
import { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';
import { EventEmitter } from '../src/events/emitter.js';
import type { ApCoreEvent } from '../src/events/emitter.js';

async function drain(gen: AsyncGenerator<Record<string, unknown>>): Promise<Record<string, unknown>[]> {
  const chunks: Record<string, unknown>[] = [];
  for await (const chunk of gen) chunks.push(chunk);
  return chunks;
}

describe('stream Phase-3 post-validation failure emits event (A-D-006)', () => {
  it('emits apcore.stream.post_validation_failed when wired with an EventEmitter', async () => {
    const registry = new Registry();
    // Output schema requires an integer `count`, but the stream yields a string,
    // so post-stream output validation (Phase 3) fails after chunks are sent.
    const mod = new FunctionModule({
      execute: () => ({ count: 1 }),
      moduleId: 'bad.stream',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ count: Type.Integer() }),
      description: 'Stream whose accumulated output fails validation',
    }) as FunctionModule & { stream: (i: Record<string, unknown>) => AsyncGenerator<Record<string, unknown>> };
    mod.stream = async function* () {
      yield { count: 'not-an-int' };
    };
    registry.register('bad.stream', mod);

    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const executor = new Executor({ registry, eventEmitter: emitter });

    // Chunks are delivered; Phase-3 failure is swallowed (no throw).
    const chunks = await drain(executor.stream('bad.stream', {}));
    expect(chunks.length).toBeGreaterThan(0);

    await emitter.flush();
    const types = events.map((e) => e.eventType);
    expect(types).toContain('apcore.stream.post_validation_failed');
    const evt = events.find((e) => e.eventType === 'apcore.stream.post_validation_failed')!;
    expect(evt.moduleId).toBe('bad.stream');
    expect(evt.severity).toBe('error');
  });

  it('does not throw and does not require an EventEmitter (optional wiring)', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ count: 1 }),
      moduleId: 'bad.stream2',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ count: Type.Integer() }),
      description: 'Stream whose accumulated output fails validation',
    }) as FunctionModule & { stream: (i: Record<string, unknown>) => AsyncGenerator<Record<string, unknown>> };
    mod.stream = async function* () {
      yield { count: 'not-an-int' };
    };
    registry.register('bad.stream2', mod);

    const executor = new Executor({ registry });
    // No emitter wired — must still complete without throwing.
    const chunks = await drain(executor.stream('bad.stream2', {}));
    expect(chunks.length).toBeGreaterThan(0);
  });
});
