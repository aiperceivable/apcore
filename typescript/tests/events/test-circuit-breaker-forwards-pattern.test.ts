/**
 * EVT-001 — wrapping a filtered subscriber must not widen it to catch-all.
 *
 * `CircuitBreakerWrapper implements EventSubscriber` but declared no
 * `eventPattern`, so `EventEmitter._getMatchingSubscribers` fell back to its
 * `'*'` default and delivered EVERY event to a subscriber the operator had
 * restricted — an event that was excluded from a webhook got POSTed to it.
 * apcore-rust forwards the pattern.
 *
 * `subscriberId` and `subscriberType` are forwarded for the same reason: the
 * wrapper stands in for the subscriber in the DLQ payload's identity fields.
 */

import { describe, it, expect } from 'vitest';
import { EventEmitter, createEvent } from '../../src/events/emitter.js';
import type { ApCoreEvent, EventSubscriber } from '../../src/events/emitter.js';
import { CircuitBreakerWrapper } from '../../src/events/circuit-breaker.js';

class Filtered implements EventSubscriber {
  readonly received: string[] = [];
  readonly eventPattern = 'apcore.module.*';
  readonly subscriberId = 'sub-filtered-1';
  readonly subscriberType = 'webhook';

  onEvent(event: ApCoreEvent): void {
    this.received.push(event.eventType);
  }
}

describe('CircuitBreakerWrapper forwards the wrapped subscriber identity (EVT-001)', () => {
  it('forwards eventPattern', () => {
    const inner = new Filtered();
    const wrapper = new CircuitBreakerWrapper(inner, { emit: () => {} });
    expect(wrapper.eventPattern).toBe('apcore.module.*');
  });

  it('forwards subscriberId and subscriberType', () => {
    const inner = new Filtered();
    const wrapper = new CircuitBreakerWrapper(inner, { emit: () => {} });
    expect(wrapper.subscriberId).toBe('sub-filtered-1');
    expect(wrapper.subscriberType).toBe('webhook');
  });

  it('an excluded event is not delivered through the wrapper', async () => {
    const emitter = new EventEmitter();
    const inner = new Filtered();
    emitter.subscribe(new CircuitBreakerWrapper(inner, emitter));

    emitter.emit(createEvent('apcore.module.toggled', 'm', 'info', {}));
    emitter.emit(createEvent('apcore.config.updated', null, 'info', {}));
    await emitter.flush();

    expect(inner.received).toEqual(['apcore.module.toggled']);
  });

  it('an unfiltered subscriber still wraps as catch-all', () => {
    const plain: EventSubscriber = { onEvent: () => {} };
    const wrapper = new CircuitBreakerWrapper(plain, { emit: () => {} });
    expect(wrapper.eventPattern).toBeUndefined();
  });
});
