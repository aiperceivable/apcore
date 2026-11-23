import { describe, it, expect, vi } from 'vitest';
import { EventEmitter, createEvent } from '../src/events/emitter.js';
import type { ApCoreEvent, EventSubscriber } from '../src/events/emitter.js';
import { StdoutSubscriber } from '../src/events/subscribers.js';

describe('Event delivery semantics (#61)', () => {
  describe('eventPattern filtering', () => {
    it('subscriber with eventPattern receives only matching events', async () => {
      const emitter = new EventEmitter();
      const received: string[] = [];
      const sub: EventSubscriber = {
        eventPattern: 'apcore.module.*',
        onEvent: (e) => { received.push(e.eventType); },
      };
      emitter.subscribe(sub);
      emitter.emit(createEvent('apcore.module.executed', null, 'info', {}));
      emitter.emit(createEvent('apcore.registry.registered', null, 'info', {}));
      emitter.emit(createEvent('apcore.module.failed', null, 'info', {}));
      await emitter.flush();
      expect(received).toEqual(['apcore.module.executed', 'apcore.module.failed']);
    });

    it('subscriber without eventPattern receives all events', async () => {
      const emitter = new EventEmitter();
      let count = 0;
      emitter.subscribe({ onEvent: () => { count++; } });
      emitter.emit(createEvent('a', null, 'info', {}));
      emitter.emit(createEvent('b', null, 'info', {}));
      await emitter.flush();
      expect(count).toBe(2);
    });

    it('eventPattern with ? wildcard', async () => {
      const emitter = new EventEmitter();
      const received: string[] = [];
      emitter.subscribe({
        eventPattern: 'apcore.?.done',
        onEvent: (e) => { received.push(e.eventType); },
      });
      emitter.emit(createEvent('apcore.x.done', null, 'info', {}));
      emitter.emit(createEvent('apcore.xy.done', null, 'info', {})); // no match: ? = 1 char
      await emitter.flush();
      expect(received).toEqual(['apcore.x.done']);
    });
  });

  describe('per-subscriber retry', () => {
    it('subscriber with retry config retries on failure', async () => {
      const emitter = new EventEmitter();
      let attempts = 0;
      const sub: EventSubscriber = {
        subscriberId: 'retry-sub',
        retry: { maxAttempts: 3, initialBackoffMs: 1, maxBackoffMs: 10, backoffMultiplier: 1 },
        async onEvent() {
          attempts++;
          if (attempts < 3) throw new Error('transient');
        },
      };
      emitter.subscribe(sub);
      emitter.emit(createEvent('test', null, 'info', {}));
      await emitter.flush();
      expect(attempts).toBe(3);
    });

    it('DLQ event emitted after all retries exhausted', async () => {
      const emitter = new EventEmitter();
      const dlqEvents: ApCoreEvent[] = [];
      emitter.subscribe({
        eventPattern: 'apcore.event.delivery_failed',
        onEvent: (e) => { dlqEvents.push(e); },
      });
      emitter.subscribe({
        subscriberId: 'failing-sub',
        retry: { maxAttempts: 2, initialBackoffMs: 1, maxBackoffMs: 10, backoffMultiplier: 1 },
        async onEvent() { throw new Error('permanent fail'); },
      });
      emitter.emit(createEvent('test.event', null, 'info', {}));
      await emitter.flush();
      expect(dlqEvents.length).toBe(1);
      const dlq = dlqEvents[0];
      expect(dlq.data['subscriber_id']).toBe('failing-sub');
      expect(dlq.data['attempt_count']).toBe(2);
      expect((dlq.data['original_event'] as Record<string,unknown>)['name']).toBe('test.event');
      expect((dlq.data['error'] as Record<string,unknown>)['message']).toBe('permanent fail');
    });

    it('onFailure callback invoked after retry exhaustion', async () => {
      const emitter = new EventEmitter();
      let failureCalled = false;
      let failureError: Error | undefined = undefined;
      let failureAttempts = 0;
      emitter.subscribe({
        subscriberId: 'onfailure-sub',
        retry: { maxAttempts: 2, initialBackoffMs: 1, maxBackoffMs: 10, backoffMultiplier: 1 },
        async onEvent() { throw new Error('boom'); },
        onFailure(_ev, err, count) {
          failureCalled = true;
          failureError = err;
          failureAttempts = count;
        },
      });
      emitter.emit(createEvent('test', null, 'info', {}));
      await emitter.flush();
      expect(failureCalled).toBe(true);
      expect((failureError as Error | undefined)?.message).toBe('boom');
      expect(failureAttempts).toBe(2);
    });
  });

  describe('default retry policy for subscribers without explicit retry config (A-D-005)', () => {
    it('custom subscriber with NO retry field is retried up to spec default (max_attempts=3)', async () => {
      const emitter = new EventEmitter();
      let attempts = 0;
      // No `retry` field at all — must inherit the spec default (max_attempts=3),
      // NOT single-attempt fire-and-forget.
      const sub: EventSubscriber = {
        subscriberId: 'no-retry-config',
        eventPattern: 'test.transient', // scope to target event, ignore any DLQ event
        async onEvent() {
          attempts++;
          if (attempts < 3) throw new Error('transient');
        },
      };
      emitter.subscribe(sub);
      emitter.emit(createEvent('test.transient', null, 'info', {}));
      await emitter.flush();
      // Initial try + 2 retries == 3 attempts; delivery succeeds on the 3rd.
      expect(attempts).toBe(3);
    });

    it('subscriber without retry field that always fails emits a DLQ event after 3 attempts', async () => {
      const emitter = new EventEmitter();
      const dlqEvents: ApCoreEvent[] = [];
      emitter.subscribe({
        eventPattern: 'apcore.event.delivery_failed',
        onEvent: (e) => { dlqEvents.push(e); },
      });
      emitter.subscribe({
        subscriberId: 'always-fails-no-config',
        eventPattern: 'test.permanent', // scope to target event, do not self-consume DLQ
        async onEvent() { throw new Error('permanent'); },
      });
      emitter.emit(createEvent('test.permanent', null, 'info', {}));
      await emitter.flush();
      expect(dlqEvents.length).toBe(1);
      expect(dlqEvents[0].data['subscriber_id']).toBe('always-fails-no-config');
      expect(dlqEvents[0].data['attempt_count']).toBe(3);
    });

    it('explicit retry: { maxAttempts: 1 } disables retry — onEvent called exactly once', async () => {
      const emitter = new EventEmitter();
      let attempts = 0;
      const sub: EventSubscriber = {
        subscriberId: 'explicit-single',
        eventPattern: 'test.single', // scope to the target event, ignore the resulting DLQ event
        retry: { maxAttempts: 1 },
        async onEvent() {
          attempts++;
          throw new Error('boom');
        },
      };
      emitter.subscribe(sub);
      emitter.emit(createEvent('test.single', null, 'info', {}));
      await emitter.flush();
      expect(attempts).toBe(1);
    });
  });

  describe('subscriberId', () => {
    it('subscriberId is used in DLQ payload', async () => {
      const emitter = new EventEmitter();
      const dlqEvents: ApCoreEvent[] = [];
      emitter.subscribe({
        eventPattern: 'apcore.event.delivery_failed',
        onEvent: (e) => { dlqEvents.push(e); },
      });
      emitter.subscribe({
        subscriberId: 'my-custom-id',
        retry: { maxAttempts: 1, initialBackoffMs: 1, maxBackoffMs: 10, backoffMultiplier: 1 },
        async onEvent() { throw new Error('x'); },
      });
      emitter.emit(createEvent('test', null, 'info', {}));
      await emitter.flush();
      expect(dlqEvents[0].data['subscriber_id']).toBe('my-custom-id');
    });
  });

  describe('fan-out isolation', () => {
    it('second subscriber delivers even when first subscriber fails after retries', async () => {
      const emitter = new EventEmitter();
      let secondCalled = false;
      emitter.subscribe({
        retry: { maxAttempts: 2, initialBackoffMs: 1, maxBackoffMs: 10, backoffMultiplier: 1 },
        async onEvent() { throw new Error('fail'); },
      });
      emitter.subscribe({ onEvent: () => { secondCalled = true; } });
      emitter.emit(createEvent('test', null, 'info', {}));
      await emitter.flush();
      expect(secondCalled).toBe(true);
    });
  });

  describe('DLQ not retried (spec fixture: dlq_event_subscriber_failure_is_not_retried)', () => {
    it('DLQ subscriber error is logged once and not retried even with retry config', async () => {
      const emitter = new EventEmitter();
      const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

      // Primary subscriber always fails — triggers DLQ after 2 attempts
      emitter.subscribe({
        subscriberId: 'broken-primary',
        retry: { maxAttempts: 2, initialBackoffMs: 1, maxBackoffMs: 10, backoffMultiplier: 1 },
        async onEvent() { throw new Error('primary fail'); },
      });

      // DLQ subscriber: has retry config with max_attempts=5, but should only be called once
      let dlqAttempts = 0;
      emitter.subscribe({
        eventPattern: 'apcore.event.delivery_failed',
        subscriberId: 'broken-dlq',
        retry: { maxAttempts: 5, initialBackoffMs: 1 },
        async onEvent() { dlqAttempts++; throw new Error('dlq fail'); },
      });

      emitter.emit(createEvent('apcore.test.broken', null, 'info', {}));
      await emitter.flush();

      // DLQ subscriber was called exactly once — no retry on DLQ delivery
      expect(dlqAttempts).toBe(1);
      // Error was logged
      expect(errorSpy).toHaveBeenCalledWith(
        expect.stringContaining('[apcore:events] DLQ subscriber raised:'),
        expect.any(Error),
      );
      // No second-order DLQ event (would cause infinite loop)
      const allEvents: string[] = [];
      emitter.subscribe({ onEvent: (e) => { allEvents.push(e.eventType); } });
      expect(allEvents.filter((t) => t === 'apcore.event.delivery_failed')).toHaveLength(0);

      errorSpy.mockRestore();
    });
  });

  describe('auto-generated subscriber IDs (spec fixture: subscriber_id_sdk_generated_when_omitted)', () => {
    it('StdoutSubscriber generates IDs matching ^stdout-[0-9]+$', () => {
      const s1 = new StdoutSubscriber();
      const s2 = new StdoutSubscriber();
      expect(s1.subscriberId).toMatch(/^stdout-\d+$/);
      expect(s2.subscriberId).toMatch(/^stdout-\d+$/);
      expect(s1.subscriberId).not.toBe(s2.subscriberId);
    });

    it('DLQ events from two unnamed subscribers carry distinct subscriber_ids', async () => {
      const emitter = new EventEmitter();
      const dlqIds: string[] = [];

      const dlqSub: EventSubscriber = {
        eventPattern: 'apcore.event.delivery_failed',
        onEvent: (e) => { dlqIds.push(e.data['subscriber_id'] as string); },
      };
      emitter.subscribe(dlqSub);

      // Use the auto-generated IDs from two StdoutSubscribers in plain subscriber wrappers
      const s1 = new StdoutSubscriber();
      const s2 = new StdoutSubscriber();
      const sub1: EventSubscriber = {
        subscriberId: s1.subscriberId,
        retry: { maxAttempts: 1 },
        async onEvent() { throw new Error('fail1'); },
      };
      const sub2: EventSubscriber = {
        subscriberId: s2.subscriberId,
        retry: { maxAttempts: 1 },
        async onEvent() { throw new Error('fail2'); },
      };
      emitter.subscribe(sub1);
      emitter.subscribe(sub2);

      emitter.emit(createEvent('apcore.test.unidentified', null, 'info', {}));
      await emitter.flush();

      expect(dlqIds).toHaveLength(2);
      expect(dlqIds[0]).toMatch(/^stdout-\d+$/);
      expect(dlqIds[1]).toMatch(/^stdout-\d+$/);
      expect(dlqIds[0]).not.toBe(dlqIds[1]);
    });
  });
});
