/**
 * A-D-05 — CircuitBreaker min_samples parity.
 *
 * Python/Rust open the circuit once `window.length >= min_samples` (default 5)
 * and the error rate crosses the threshold, decoupled from the rolling-window
 * capacity. The TypeScript implementation previously gated opening on a FULL
 * window (`length >= windowSize`, default 10), so it never opened on the spec's
 * minimum-sample boundary. These tests guard the corrected behavior.
 *
 * The constructor also validates `open_threshold` in [0,1] and `window_size >= 1`
 * and clamps `min_samples` down to `window_size` (parity with Python/Rust), so a
 * breaker can always eventually open.
 */
import { describe, expect, it } from 'vitest';
import { Context } from '../../src/context.js';
import { CancelToken } from '../../src/cancel.js';
import { Executor } from '../../src/executor.js';
import { Registry } from '../../src/registry/registry.js';
import {
  CircuitBreakerMiddleware,
  CircuitBreakerState,
} from '../../src/middleware/circuit-breaker.js';

function driveErrors(
  cb: CircuitBreakerMiddleware,
  moduleId: string,
  ctx: Context,
  count: number,
): void {
  for (let i = 0; i < count; i++) {
    try {
      cb.before(moduleId, {}, ctx);
    } catch {
      // circuit may already be OPEN on later iterations — ignore
    }
    cb.onError(moduleId, {}, new Error('simulated'), ctx);
  }
}

describe('CircuitBreaker min_samples (A-D-05)', () => {
  it('allows the next HALF_OPEN probe when the cancelled call has settled', async () => {
    const registry = new Registry();
    const moduleId = 'test.circuit';
    let calls = 0;
    let entered!: () => void;
    const enteredProbe = new Promise<void>((resolve) => { entered = resolve; });
    registry.register(moduleId, {
      description: 'A circuit breaker test module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      async execute(_inputs: Record<string, unknown>, context: Context) {
        calls++;
        if (calls === 1) throw new Error('Service failure');
        if (calls === 2) {
          entered();
          await new Promise<void>((resolve) => context.signal.addEventListener('abort', () => resolve(), { once: true }));
        }
        return { settled: true };
      },
    });
    const executor = new Executor({ registry });
    const breaker = new CircuitBreakerMiddleware({ minSamples: 1, recoveryWindowMs: 0 });
    executor.use(breaker);
    await expect(executor.call(moduleId)).rejects.toThrow('Service failure');

    const token = new CancelToken();
    const probe = executor.call(moduleId, {}, Context.create(undefined, undefined, token));
    await enteredProbe;
    token.cancel();
    await expect(probe).rejects.toMatchObject({ code: 'EXECUTION_CANCELLED' });
    await token.drain();

    await expect(executor.call(moduleId)).resolves.toEqual({ settled: true });
    expect(breaker.getState(moduleId)).toBe(CircuitBreakerState.CLOSED);
  });

  it('opens after 5 consecutive failures with default options', () => {
    const cb = new CircuitBreakerMiddleware();
    const ctx = new Context('trace', 'caller', []);
    const moduleId = 'executor.payment.charge';

    driveErrors(cb, moduleId, ctx, 5);

    expect(cb.getState(moduleId, 'caller')).toBe(CircuitBreakerState.OPEN);
  });

  it('stays CLOSED below min_samples even at a high error rate', () => {
    const cb = new CircuitBreakerMiddleware();
    const ctx = new Context('trace', 'caller', []);
    const moduleId = 'executor.payment.charge';

    driveErrors(cb, moduleId, ctx, 4);

    expect(cb.getState(moduleId, 'caller')).toBe(CircuitBreakerState.CLOSED);
  });

  it('clamps min_samples down to window_size so the breaker can still open', () => {
    // min_samples (100) exceeds window_size (default 20): Python/Rust clamp
    // min_samples to window_size, otherwise the breaker could never open.
    const cb = new CircuitBreakerMiddleware({ minSamples: 100 });
    const ctx = new Context('trace', 'caller', []);
    const moduleId = 'executor.payment.charge';

    // After window_size (20) failures the window is full and min_samples is
    // clamped to 20, so the circuit OPENS.
    driveErrors(cb, moduleId, ctx, 20);

    expect(cb.getState(moduleId, 'caller')).toBe(CircuitBreakerState.OPEN);
  });

  it('clamps min_samples to a smaller windowSize and opens after windowSize failures', () => {
    const cb = new CircuitBreakerMiddleware({ windowSize: 3 });
    const ctx = new Context('trace', 'caller', []);
    const moduleId = 'executor.payment.charge';

    // Default minSamples=5 > windowSize=3 → clamped to 3; opens after 3 failures.
    driveErrors(cb, moduleId, ctx, 3);

    expect(cb.getState(moduleId, 'caller')).toBe(CircuitBreakerState.OPEN);
  });

  it('throws on an out-of-range openThreshold', () => {
    expect(() => new CircuitBreakerMiddleware({ openThreshold: 1.5 })).toThrow(
      /open_threshold/,
    );
    expect(() => new CircuitBreakerMiddleware({ openThreshold: -0.1 })).toThrow(
      /open_threshold/,
    );
  });

  it('throws on a window_size below 1', () => {
    expect(() => new CircuitBreakerMiddleware({ windowSize: 0 })).toThrow(/window_size/);
  });
});
