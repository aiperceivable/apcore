/**
 * Spec-traced contract tests for the apcore TypeScript event system.
 *
 * Source spec: apcore/docs/features/event-system.md
 * Canonical suite mirrored: apcore-python/tests/test_event_system_spec.py
 *
 * Each `it(...)` name carries the verbatim clause id
 * ('event_system.<method>.<kind>.<detail>') so cross-language diffs line up.
 *
 * TESTS ONLY — never modifies production source.
 *
 * Symbol-reality notes (drive the skip / divergence decisions below):
 * - `ApCoreEvent` is a plain interface (camelCase fields), not a class. Events
 *   are constructed as plain objects.
 * - `EventEmitter.emit` takes an `ApCoreEvent` and is fire-and-forget (sync,
 *   returns void). The spec's `input.event_type.not_empty` rule is not enforced.
 * - `EventEmitter.subscribe` performs NO runtime async/type guard — the
 *   `EventSubscriber` contract is enforced statically by TypeScript. So the
 *   Python `subscribe.input.subscriber.async_on_event` TypeError guard has no
 *   TS analogue (skip: language-idiom divergence).
 * - Subscribers deliver via global `fetch` (not aiohttp). There is no
 *   `DeliveryError`/`WEBHOOK_DELIVERY_FAILED` type — webhook/a2a rethrow plain
 *   `Error` on 5xx and the emitter owns the DLQ path (skip: missing symbol).
 *   The A2A `ImportError`-without-aiohttp clause has no TS analogue (fetch is a
 *   global; skip: missing symbol).
 * - The circuit-breaker contract names `SubscriberCircuitBreaker.on_failure`
 *   with signature `(subscriber_id, error) -> CircuitState`. This SDK ships
 *   `CircuitBreakerWrapper` with a private `_onFailure(error)` and a public
 *   `onEvent` driver. The exact contract method is a missing symbol; the
 *   observable state machine is exercised via `onEvent` / `state`.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  EventEmitter,
  WebhookSubscriber,
  A2ASubscriber,
} from '../src/events/index.js';
import { CircuitBreakerWrapper, CircuitState } from '../src/events/circuit-breaker.js';
import type { ApCoreEvent, EventSubscriber } from '../src/events/index.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeEvent(overrides: Partial<ApCoreEvent> = {}): ApCoreEvent {
  return {
    eventType: 'apcore.test.event',
    moduleId: 'mod.a',
    timestamp: '2026-03-08T00:00:00Z',
    severity: 'info',
    data: {},
    ...overrides,
  };
}

class RecordingSubscriber implements EventSubscriber {
  readonly eventPattern: string;
  received: ApCoreEvent[] = [];

  constructor(eventPattern: string = '*') {
    this.eventPattern = eventPattern;
  }

  async onEvent(event: ApCoreEvent): Promise<void> {
    this.received.push(event);
  }
}

/** A subscriber that always throws — mirrors Python's AsyncMock(side_effect=...). */
class ThrowingSubscriber implements EventSubscriber {
  readonly eventPattern = '*';
  calls = 0;

  async onEvent(_event: ApCoreEvent): Promise<void> {
    this.calls += 1;
    throw new Error('boom');
  }
}

/** Stub global fetch to resolve with the given HTTP status. Returns the mock. */
function stubFetch(status: number): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn().mockResolvedValue({ status });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

// ===========================================================================
// Contract: EventEmitter.emit
// ===========================================================================

describe('EventEmitter.emit', () => {
  it.skip('event_system.emit.input.event_type.not_empty: emit() does not validate event_type (fire-and-forget; caller precondition)', () => {
    // Spec (event-system.md, EventEmitter.emit) declares event_type MUST NOT be
    // empty as a CALLER PRECONDITION, not a validated rejection: emit() takes an
    // ApCoreEvent and is fire-and-forget — it enforces no validation and raises
    // no error, consistent with the Contract's Errors/Properties sections and
    // identical to Python and Rust. A real it.skip() (not a green placeholder)
    // so cross-language tester runs do not miscount this as an enforcement pass.
  });

  it('event_system.emit.error.none_raised: a throwing subscriber must not surface to emit()/flush() callers', async () => {
    const emitter = new EventEmitter();
    const boom = new ThrowingSubscriber();
    emitter.subscribe(boom);
    try {
      expect(() => emitter.emit(makeEvent())).not.toThrow();
      await expect(emitter.flush(2000)).resolves.toBeUndefined();
    } finally {
      await emitter.shutdown(1000);
    }
    expect(boom.calls).toBeGreaterThanOrEqual(1);
  });

  it('event_system.emit.property.async: emit() is synchronous fire-and-forget (returns void, not a Promise)', async () => {
    const emitter = new EventEmitter();
    try {
      const result = emitter.emit(makeEvent());
      expect(result).toBeUndefined();
      expect(result).not.toBeInstanceOf(Promise);
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.emit.property.thread_safe: >=8 concurrent emits deliver every event exactly once', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    const n = 16;
    try {
      await Promise.all(
        Array.from({ length: n }, (_, i) =>
          Promise.resolve().then(() => emitter.emit(makeEvent({ data: { i } }))),
        ),
      );
      await emitter.flush(5000);
    } finally {
      await emitter.shutdown(2000);
    }
    const delivered = sub.received.map((e) => e.data.i as number).sort((a, b) => a - b);
    expect(delivered).toEqual(Array.from({ length: n }, (_, i) => i));
  });

  it('event_system.emit.property.pure: emit() is NOT pure — it invokes subscriber callbacks', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    try {
      emitter.emit(makeEvent());
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    expect(sub.received.length).toBe(1);
  });

  it('event_system.emit.property.idempotent: emit() is NOT idempotent — two identical emits deliver twice', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    const event = makeEvent();
    try {
      emitter.emit(event);
      emitter.emit(event);
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    expect(sub.received.length).toBe(2);
  });

  it('event_system.emit.side_effect.1.subscriber_invoked: matching subscribers receive the emitted event', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber('apcore.test.*');
    emitter.subscribe(sub);
    const event = makeEvent({ eventType: 'apcore.test.event', data: { k: 'v' } });
    try {
      emitter.emit(event);
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    expect(sub.received.map((e) => e.data)).toEqual([{ k: 'v' }]);
  });
});

// ===========================================================================
// Contract: EventEmitter.subscribe
// ===========================================================================

describe('EventEmitter.subscribe', () => {
  it.skip('event_system.subscribe.input.subscriber.async_on_event: missing symbol — no runtime async-guard (TS enforces EventSubscriber statically)', async () => {
    // Python subscribe() raises TypeError if on_event is not a coroutine
    // function. TypeScript enforces the EventSubscriber.onEvent contract at
    // compile time, so there is no runtime guard to assert against. This is a
    // language-idiom divergence, recorded as a skip.
  });

  it('event_system.subscribe.error.none_raised: a correctly-typed subscriber subscribes without raising', async () => {
    const emitter = new EventEmitter();
    try {
      expect(() => emitter.subscribe(new RecordingSubscriber())).not.toThrow();
      emitter.emit(makeEvent());
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.subscribe.property.async: subscribe() is synchronous (returns void)', async () => {
    const emitter = new EventEmitter();
    try {
      const result = emitter.subscribe(new RecordingSubscriber());
      expect(result).toBeUndefined();
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.subscribe.property.thread_safe: >=8 concurrent subscribes register every subscriber', async () => {
    const emitter = new EventEmitter();
    const n = 12;
    const subs = Array.from({ length: n }, () => new RecordingSubscriber());
    try {
      await Promise.all(
        subs.map((s) => Promise.resolve().then(() => emitter.subscribe(s))),
      );
      // Observe registration via delivery: one emit reaches every subscriber once.
      emitter.emit(makeEvent());
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    const deliveredCount = subs.filter((s) => s.received.length === 1).length;
    expect(deliveredCount).toBe(n);
  });

  it('event_system.subscribe.property.idempotent: subscribing the same object twice delivers each event twice', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    emitter.subscribe(sub);
    try {
      emitter.emit(makeEvent());
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    expect(sub.received.length).toBe(2);
  });
});

// ===========================================================================
// Contract: WebhookSubscriber.deliver
// ===========================================================================

describe('WebhookSubscriber.deliver', () => {
  it('event_system.deliver.input.event.required: webhook delivery POSTs the serialized event', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new WebhookSubscriber('https://example.com/hook');
      const event = makeEvent({ eventType: 'apcore.health.recovered' });
      await sub.onEvent(event);
      expect(fetchMock).toHaveBeenCalledTimes(1);
      const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
      const posted = JSON.parse(init.body as string);
      expect(posted.event_type).toBe('apcore.health.recovered');
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it.skip('event_system.deliver.error.WEBHOOK_DELIVERY_FAILED: missing symbol DeliveryError/WEBHOOK_DELIVERY_FAILED (contract gap)', async () => {
    // Spec declares DeliveryError(code=WEBHOOK_DELIVERY_FAILED) on retry
    // exhaustion. This SDK has no such type — WebhookSubscriber.onEvent rethrows
    // a plain Error on 5xx and the EventEmitter owns the DLQ path
    // (apcore.event.delivery_failed). Recorded as a missing-symbol skip.
  });

  it('event_system.deliver.property.async: webhook delivery returns a Promise that resolves to undefined', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new WebhookSubscriber('https://example.com/hook');
      const result = sub.onEvent(makeEvent());
      expect(result).toBeInstanceOf(Promise);
      await expect(result).resolves.toBeUndefined();
      expect(fetchMock).toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('event_system.deliver.property.thread_safe: >=8 concurrent webhook deliveries each issue their own POST', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new WebhookSubscriber('https://example.com/hook');
      await Promise.all(
        Array.from({ length: 8 }, (_, i) => sub.onEvent(makeEvent({ data: { i } }))),
      );
      expect(fetchMock).toHaveBeenCalledTimes(8);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('event_system.deliver.property.pure: delivery is NOT pure — it performs an outbound HTTP POST', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new WebhookSubscriber('https://example.com/hook');
      await sub.onEvent(makeEvent());
      expect(fetchMock).toHaveBeenCalledTimes(1);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('event_system.deliver.side_effect.1.retry_on_5xx: 5xx rethrows (emitter retries); 4xx does not throw (no retry)', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    // 5xx -> throws
    let fetchMock = stubFetch(503);
    try {
      const sub = new WebhookSubscriber('https://example.com/hook');
      await expect(sub.onEvent(makeEvent())).rejects.toThrow();
    } finally {
      vi.unstubAllGlobals();
    }
    // 4xx -> permanent failure: logged, not thrown
    fetchMock = stubFetch(404);
    try {
      const sub2 = new WebhookSubscriber('https://example.com/hook');
      await expect(sub2.onEvent(makeEvent())).resolves.toBeUndefined();
      expect(fetchMock).toHaveBeenCalledTimes(1);
    } finally {
      vi.unstubAllGlobals();
      vi.restoreAllMocks();
    }
  });
});

// ===========================================================================
// Contract: SubscriberCircuitBreaker.on_failure
// ===========================================================================

describe('SubscriberCircuitBreaker.on_failure', () => {
  const emitterStub = { emit: () => {} };

  it.skip('event_system.on_failure.input.subscriber_id.required: missing symbol — CircuitBreakerWrapper has no subscriber_id param (contract gap)', () => {
    // Spec contract is SubscriberCircuitBreaker.on_failure(subscriber_id, error)
    // -> CircuitState. This SDK ships CircuitBreakerWrapper with a private
    // _onFailure(error) and no subscriber_id parameter. Recorded as a
    // missing-symbol skip.
  });

  it('event_system.on_failure.error.none_raised: breaker must not raise on a delivery failure; it records state', async () => {
    const failing = new ThrowingSubscriber();
    const wrapper = new CircuitBreakerWrapper(failing, emitterStub, { openThreshold: 5 });
    await expect(wrapper.onEvent(makeEvent())).resolves.toBeUndefined();
    expect(wrapper.consecutiveFailures).toBe(1);
  });

  it('event_system.on_failure.returns.circuit_state: CLOSED->OPEN after open_threshold consecutive failures', async () => {
    const failing = new ThrowingSubscriber();
    const wrapper = new CircuitBreakerWrapper(failing, emitterStub, { openThreshold: 3 });
    for (let i = 0; i < 3; i++) {
      await wrapper.onEvent(makeEvent());
    }
    expect(Object.values(CircuitState)).toContain(wrapper.state);
    expect(wrapper.state).toBe(CircuitState.OPEN);
  });

  it('event_system.on_failure.property.async: the failure-accounting transition is synchronous (not a Promise)', () => {
    const failing = new ThrowingSubscriber();
    const wrapper = new CircuitBreakerWrapper(failing, emitterStub, { openThreshold: 2 });
    // _onFailure is a private synchronous transition; access reflectively to
    // assert it returns a value (event or null), not a Promise.
    const result = (
      wrapper as unknown as { _onFailure(e: unknown): unknown }
    )._onFailure(new Error('x'));
    expect(result).not.toBeInstanceOf(Promise);
  });

  it('event_system.on_failure.property.thread_safe: >=8 concurrent failures update the counter without loss (OPEN after threshold)', async () => {
    const failing = new ThrowingSubscriber();
    const wrapper = new CircuitBreakerWrapper(failing, emitterStub, { openThreshold: 8 });
    await Promise.all(Array.from({ length: 8 }, (_, i) => wrapper.onEvent(makeEvent({ data: { i } }))));
    expect(wrapper.consecutiveFailures).toBe(8);
    expect(wrapper.state).toBe(CircuitState.OPEN);
  });

  it('event_system.on_failure.property.pure: failure handling mutates circuit state (consecutiveFailures increments)', async () => {
    const failing = new ThrowingSubscriber();
    const wrapper = new CircuitBreakerWrapper(failing, emitterStub, { openThreshold: 5 });
    const before = wrapper.consecutiveFailures;
    await wrapper.onEvent(makeEvent());
    const after = wrapper.consecutiveFailures;
    expect(after).toBe(before + 1);
  });

  it('event_system.on_failure.property.idempotent: repeated failures are NOT idempotent — each increments the counter', async () => {
    const failing = new ThrowingSubscriber();
    const wrapper = new CircuitBreakerWrapper(failing, emitterStub, { openThreshold: 10 });
    await wrapper.onEvent(makeEvent());
    const first = wrapper.consecutiveFailures;
    await wrapper.onEvent(makeEvent());
    const second = wrapper.consecutiveFailures;
    expect([first, second]).toEqual([1, 2]);
  });
});

// ===========================================================================
// Contract: EventEmitter.unsubscribe
// ===========================================================================

describe('EventEmitter.unsubscribe', () => {
  it('event_system.unsubscribe.input.subscriber.same_reference: removes the subscriber by identity; no further deliveries', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    emitter.unsubscribe(sub);
    try {
      emitter.emit(makeEvent());
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    expect(sub.received).toEqual([]);
  });

  it('event_system.unsubscribe.error.unregistered_no_raise: unsubscribing an unregistered subscriber is a no-op', async () => {
    const emitter = new EventEmitter();
    try {
      expect(() => emitter.unsubscribe(new RecordingSubscriber())).not.toThrow();
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.unsubscribe.property.async: unsubscribe() is synchronous (returns void)', async () => {
    const emitter = new EventEmitter();
    try {
      const sub = new RecordingSubscriber();
      emitter.subscribe(sub);
      expect(emitter.unsubscribe(sub)).toBeUndefined();
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.unsubscribe.property.thread_safe: >=8 concurrent unsubscribes leave the registry empty', async () => {
    const emitter = new EventEmitter();
    const n = 10;
    const subs = Array.from({ length: n }, () => new RecordingSubscriber());
    for (const s of subs) emitter.subscribe(s);
    try {
      await Promise.all(
        subs.map((s) => Promise.resolve().then(() => emitter.unsubscribe(s))),
      );
      // Observe empty registry: an emit reaches no subscriber.
      emitter.emit(makeEvent());
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    expect(subs.every((s) => s.received.length === 0)).toBe(true);
  });

  it('event_system.unsubscribe.property.pure: unsubscribe mutates the subscriber list (delivery stops)', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    try {
      emitter.emit(makeEvent());
      await emitter.flush(2000);
      const before = sub.received.length;
      emitter.unsubscribe(sub);
      emitter.emit(makeEvent());
      await emitter.flush(2000);
      const after = sub.received.length;
      expect([before, after]).toEqual([1, 1]);
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.unsubscribe.property.idempotent: repeated unsubscribe of the same subscriber is a safe no-op', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    try {
      emitter.unsubscribe(sub);
      expect(() => emitter.unsubscribe(sub)).not.toThrow();
      emitter.emit(makeEvent());
      await emitter.flush(2000);
    } finally {
      await emitter.shutdown(1000);
    }
    expect(sub.received).toEqual([]);
  });
});

// ===========================================================================
// Contract: EventEmitter.flush
// ===========================================================================

describe('EventEmitter.flush', () => {
  it('event_system.flush.input.timeout.positive: a positive timeout lets a fast in-flight delivery complete', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    try {
      emitter.emit(makeEvent());
      await emitter.flush(5000);
      expect(sub.received.length).toBe(1);
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.flush.error.none_raised: subscriber errors during flush are discarded; flush must not reject', async () => {
    const emitter = new EventEmitter();
    const boom = new ThrowingSubscriber();
    emitter.subscribe(boom);
    try {
      emitter.emit(makeEvent());
      await expect(emitter.flush(2000)).resolves.toBeUndefined();
    } finally {
      await emitter.shutdown(1000);
    }
    expect(boom.calls).toBeGreaterThanOrEqual(1);
  });

  it('event_system.flush.property.async: flush() returns a Promise (TS divergence — Python flush is sync/blocking)', async () => {
    const emitter = new EventEmitter();
    try {
      const result = emitter.flush(500);
      expect(result).toBeInstanceOf(Promise);
      await expect(result).resolves.toBeUndefined();
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.flush.property.thread_safe: >=8 concurrent flushes (with emits in flight) all resolve without error', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    for (let i = 0; i < 8; i++) emitter.emit(makeEvent());
    try {
      const results = await Promise.allSettled(
        Array.from({ length: 8 }, () => emitter.flush(5000)),
      );
      expect(results.every((r) => r.status === 'fulfilled')).toBe(true);
      await emitter.flush(5000);
    } finally {
      await emitter.shutdown(2000);
    }
    expect(sub.received.length).toBe(8);
  });

  it('event_system.flush.property.pure: flush mutates shared pending state (pending set empties)', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    try {
      emitter.emit(makeEvent());
      await emitter.flush(2000);
      const pending = (emitter as unknown as { _pending: unknown[] })._pending;
      expect(pending.length).toBe(0);
    } finally {
      await emitter.shutdown(1000);
    }
  });

  it('event_system.flush.property.idempotent: a second flush on an already-drained set is a safe no-op', async () => {
    const emitter = new EventEmitter();
    const sub = new RecordingSubscriber();
    emitter.subscribe(sub);
    try {
      emitter.emit(makeEvent());
      await emitter.flush(2000);
      const countAfterFirst = sub.received.length;
      await emitter.flush(2000);
      const countAfterSecond = sub.received.length;
      expect([countAfterFirst, countAfterSecond]).toEqual([1, 1]);
    } finally {
      await emitter.shutdown(1000);
    }
  });
});

// ===========================================================================
// Contract: A2ASubscriber.deliver
// ===========================================================================

describe('A2ASubscriber.deliver', () => {
  it('event_system.deliver.input.event.required_a2a: A2A delivery POSTs a {skillId, event} wrapper', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new A2ASubscriber('https://platform.example.com');
      const event = makeEvent({ eventType: 'apcore.health.recovered' });
      await sub.onEvent(event);
      expect(fetchMock).toHaveBeenCalledTimes(1);
      const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
      const body = JSON.parse(init.body as string);
      expect(body.skillId).toBe('apevo.event_receiver');
      expect(body.event.event_type).toBe('apcore.health.recovered');
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it.skip('event_system.deliver.error.ImportError_a2a: missing symbol — TS uses global fetch, no ImportError without aiohttp (contract gap)', async () => {
    // Python raises ImportError synchronously if aiohttp is not installed. The
    // TS SDK uses the global `fetch`; there is no optional HTTP-client import
    // and thus no ImportError analogue. Recorded as a missing-symbol skip.
  });

  it('event_system.deliver.property.async_a2a: A2A delivery returns a Promise that resolves to undefined', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new A2ASubscriber('https://platform.example.com');
      const result = sub.onEvent(makeEvent());
      expect(result).toBeInstanceOf(Promise);
      await expect(result).resolves.toBeUndefined();
      expect(fetchMock).toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('event_system.deliver.property.thread_safe_a2a: >=8 concurrent A2A deliveries each issue their own POST', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new A2ASubscriber('https://platform.example.com');
      await Promise.all(
        Array.from({ length: 8 }, (_, i) => sub.onEvent(makeEvent({ data: { i } }))),
      );
      expect(fetchMock).toHaveBeenCalledTimes(8);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('event_system.deliver.property.pure_a2a: delivery is NOT pure — it POSTs to the platform URL', async () => {
    const fetchMock = stubFetch(200);
    try {
      const sub = new A2ASubscriber('https://platform.example.com');
      await sub.onEvent(makeEvent());
      expect(fetchMock.mock.calls[0][0]).toBe('https://platform.example.com');
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('event_system.deliver.side_effect.1.auth_modes_a2a: str->Bearer; dict->merged headers; None->no Authorization', async () => {
    // str auth -> Bearer
    let fetchMock = stubFetch(200);
    try {
      const subStr = new A2ASubscriber('https://p.example.com', 'tok123');
      await subStr.onEvent(makeEvent());
      const headersStr = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
      expect(headersStr['Authorization']).toBe('Bearer tok123');
    } finally {
      vi.unstubAllGlobals();
    }

    // dict auth -> merged
    fetchMock = stubFetch(200);
    try {
      const subDict = new A2ASubscriber('https://p.example.com', { 'X-Api-Key': 'k' });
      await subDict.onEvent(makeEvent());
      const headersDict = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
      expect(headersDict['X-Api-Key']).toBe('k');
    } finally {
      vi.unstubAllGlobals();
    }

    // None auth -> no Authorization
    fetchMock = stubFetch(200);
    try {
      const subNone = new A2ASubscriber('https://p.example.com', undefined);
      await subNone.onEvent(makeEvent());
      const headersNone = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
      expect('Authorization' in headersNone).toBe(false);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('event_system.deliver.side_effect.2.retry_on_5xx_a2a: 5xx rethrows (emitter retries); 4xx does not throw (no retry)', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    // 5xx -> throws
    let fetchMock = stubFetch(502);
    try {
      const sub5xx = new A2ASubscriber('https://p.example.com');
      await expect(sub5xx.onEvent(makeEvent())).rejects.toThrow();
    } finally {
      vi.unstubAllGlobals();
    }
    // 4xx -> not thrown
    fetchMock = stubFetch(403);
    try {
      const sub4xx = new A2ASubscriber('https://p.example.com');
      await expect(sub4xx.onEvent(makeEvent())).resolves.toBeUndefined();
      expect(fetchMock).toHaveBeenCalledTimes(1);
    } finally {
      vi.unstubAllGlobals();
      vi.restoreAllMocks();
    }
  });
});
