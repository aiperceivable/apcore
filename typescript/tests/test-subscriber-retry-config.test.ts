/**
 * The nested `retry:` block in subscriber config is read by every factory (apcore#85).
 *
 * `docs/features/event-system.md` documents a per-subscriber `retry:` block and
 * shows it on multiple subscriber types. Before apcore#85 no factory parsed it:
 * only `webhook` built a policy, and only from the legacy flat `retry_count`
 * shorthand. An operator copying the documented example got no retry policy at
 * all, silently.
 *
 * Every assertion below deliberately uses values that differ from DEFAULT_RETRY
 * (maxAttempts=3, initialBackoffMs=100, maxBackoffMs=30000, backoffMultiplier=2.0)
 * — otherwise the test would pass whether or not the config was ever read.
 */
import { describe, expect, it } from 'vitest';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  createSubscriberFromConfig,
} from '../src/sys-modules/registration.js';
import {
  WebhookSubscriber,
  FileSubscriber,
  StdoutSubscriber,
  FilterSubscriber,
} from '../src/events/subscribers.js';
import { EventEmitter } from '../src/events/emitter.js';
import type { ApCoreEvent, EventSubscriber } from '../src/events/emitter.js';
import { DEFAULT_RETRY, resolveRetry } from '../src/events/retry.js';
import type { RetryConfig } from '../src/events/retry.js';

/** A policy in which every single field differs from DEFAULT_RETRY. */
const NON_DEFAULT_RETRY = {
  max_attempts: 7,
  initial_backoff_ms: 250,
  max_backoff_ms: 10_000,
  backoff_multiplier: 3.0,
};

function expectNonDefaultPolicy(retry: RetryConfig | undefined): void {
  const resolved = resolveRetry(retry);
  expect(resolved.maxAttempts).toBe(7);
  expect(resolved.maxAttempts).not.toBe(DEFAULT_RETRY.maxAttempts);
  expect(resolved.initialBackoffMs).toBe(250);
  expect(resolved.initialBackoffMs).not.toBe(DEFAULT_RETRY.initialBackoffMs);
  expect(resolved.maxBackoffMs).toBe(10_000);
  expect(resolved.maxBackoffMs).not.toBe(DEFAULT_RETRY.maxBackoffMs);
  expect(resolved.backoffMultiplier).toBe(3.0);
  expect(resolved.backoffMultiplier).not.toBe(DEFAULT_RETRY.backoffMultiplier);
}

const tmpPath = (name: string): string => path.join(os.tmpdir(), `apcore-retry-${name}.jsonl`);

function makeEvent(): ApCoreEvent {
  return {
    eventType: 'apcore.test',
    moduleId: 'mod.a',
    timestamp: '2026-01-01T00:00:00Z',
    severity: 'info',
    data: {},
  };
}

describe('nested retry: block — one case per built-in subscriber type', () => {
  it('webhook reads the nested retry block', () => {
    const sub = createSubscriberFromConfig({
      type: 'webhook',
      url: 'https://example.com/hook',
      retry: NON_DEFAULT_RETRY,
    });
    expect(sub).toBeInstanceOf(WebhookSubscriber);
    expectNonDefaultPolicy(sub.retry);
  });


  it('file reads the nested retry block', () => {
    const sub = createSubscriberFromConfig({
      type: 'file',
      path: tmpPath('file'),
      retry: NON_DEFAULT_RETRY,
    });
    expect(sub).toBeInstanceOf(FileSubscriber);
    expectNonDefaultPolicy(sub.retry);
  });

  it('stdout reads the nested retry block', () => {
    const sub = createSubscriberFromConfig({
      type: 'stdout',
      format: 'json',
      retry: NON_DEFAULT_RETRY,
    });
    expect(sub).toBeInstanceOf(StdoutSubscriber);
    expectNonDefaultPolicy(sub.retry);
  });

  it('filter reads the nested retry block', () => {
    const sub = createSubscriberFromConfig({
      type: 'filter',
      delegate_type: 'stdout',
      delegate_config: { format: 'json' },
      include_events: ['apcore.error.*'],
      retry: NON_DEFAULT_RETRY,
    });
    expect(sub).toBeInstanceOf(FilterSubscriber);
    expectNonDefaultPolicy(sub.retry);
  });
});

describe('nested retry: block — parsing semantics', () => {
  it('merges a partial block over the spec defaults', () => {
    // The documented `file` example declares only two of the four keys.
    const sub = createSubscriberFromConfig({
      type: 'file',
      path: tmpPath('partial'),
      retry: { max_attempts: 2, initial_backoff_ms: 50 },
    });
    const resolved = resolveRetry(sub.retry);
    expect(resolved.maxAttempts).toBe(2);
    expect(resolved.maxAttempts).not.toBe(DEFAULT_RETRY.maxAttempts);
    expect(resolved.initialBackoffMs).toBe(50);
    expect(resolved.initialBackoffMs).not.toBe(DEFAULT_RETRY.initialBackoffMs);
    // Unspecified keys keep the spec defaults.
    expect(resolved.maxBackoffMs).toBe(DEFAULT_RETRY.maxBackoffMs);
    expect(resolved.backoffMultiplier).toBe(DEFAULT_RETRY.backoffMultiplier);
  });

  it('keeps the spec defaults when the block is absent', () => {
    const sub = createSubscriberFromConfig({ type: 'stdout' });
    expect(resolveRetry(sub.retry)).toEqual(DEFAULT_RETRY);
  });

  it('ignores a non-object retry value without throwing', () => {
    const sub = createSubscriberFromConfig({ type: 'stdout', retry: 'aggressive' });
    expect(resolveRetry(sub.retry)).toEqual(DEFAULT_RETRY);
  });

  it('still honours the deprecated flat retry_count on webhook', () => {
    // retry_count counted retries AFTER the first attempt.
    const sub = createSubscriberFromConfig({
      type: 'webhook',
      url: 'https://example.com/hook',
      retry_count: 5,
    });
    expect(resolveRetry(sub.retry).maxAttempts).toBe(6);
  });

  it('lets the nested block win over flat retry_count', () => {
    const sub = createSubscriberFromConfig({
      type: 'webhook',
      url: 'https://example.com/hook',
      retry_count: 5,
      retry: NON_DEFAULT_RETRY,
    });
    // retry_count=5 would have produced maxAttempts=6; the nested block wins.
    expectNonDefaultPolicy(sub.retry);
  });

  it('applies a retry block inside delegate_config to the delegate, not the filter', () => {
    const sub = createSubscriberFromConfig({
      type: 'filter',
      delegate_type: 'file',
      delegate_config: { path: tmpPath('delegate'), retry: NON_DEFAULT_RETRY },
    }) as FilterSubscriber;
    expect(resolveRetry(sub.retry)).toEqual(DEFAULT_RETRY);
    const delegate = (sub as unknown as { _delegate: EventSubscriber })._delegate;
    expectNonDefaultPolicy(delegate.retry);
  });
});

describe('subscriber factory defaults and validation', () => {
  it.each([
    { type: 'file', path: tmpPath('defaults') },
    { type: 'filter', delegate_type: 'stdout' },
  ])('keeps the default retry policy for $type', (config) => {
    expect(resolveRetry(createSubscriberFromConfig(config).retry)).toEqual(DEFAULT_RETRY);
  });

  it.each([
    { config: {}, message: /missing "type"/ },
    { config: { type: 'unknown' }, message: /Unknown subscriber type 'unknown'/ },
    { config: { type: 'file', path: '' }, message: /non-empty "path"/ },
    { config: { type: 'filter' }, message: /requires a "delegate_type"/ },
    {
      config: { type: 'filter', delegate_type: 'unknown' },
      message: /Unknown delegate subscriber type 'unknown'/,
    },
  ])('rejects invalid subscriber config $config', ({ config, message }) => {
    expect(() => createSubscriberFromConfig(config)).toThrow(message);
  });
});

describe('the declared policy actually governs delivery', () => {
  it('emitter honours a config-declared maxAttempts', async () => {
    let attempts = 0;
    const sub = createSubscriberFromConfig({
      type: 'stdout',
      // 5 differs from the default 3, so a factory that ignored the block
      // would deliver 3 times and fail this assertion.
      retry: { max_attempts: 5, initial_backoff_ms: 0 },
    });
    // Replace only the sink; `retry` still comes from the parsed config.
    (sub as { onEvent: (e: ApCoreEvent) => Promise<void> }).onEvent = async () => {
      attempts += 1;
      throw new Error('transient sink failure');
    };

    const emitter = new EventEmitter();
    emitter.subscribe(sub);
    emitter.emit(makeEvent());
    await emitter.flush(5000);
    await emitter.shutdown();

    expect(attempts).toBe(5);
  });

  it('lets a custom registered type reuse the same nested block shape', () => {
    class CustomSink implements EventSubscriber {
      readonly subscriberType = 'custom_sink';
      constructor(readonly retry?: RetryConfig) {}
      async onEvent(_event: ApCoreEvent): Promise<void> {}
    }
    const factory = (config: Record<string, unknown>) => {
      const raw = config['retry'] as Record<string, number>;
      return new CustomSink({
        maxAttempts: raw['max_attempts'],
        initialBackoffMs: raw['initial_backoff_ms'],
        maxBackoffMs: raw['max_backoff_ms'],
        backoffMultiplier: raw['backoff_multiplier'],
      });
    };
    const sub = createSubscriberFromConfig(
      { type: 'custom_sink', retry: NON_DEFAULT_RETRY },
      { custom_sink: factory },
    );
    expectNonDefaultPolicy(sub.retry);
  });
});
