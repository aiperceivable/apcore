/**
 * EventSubscriber implementations — Webhook, A2A, File, Stdout, Filter.
 *
 * Exercises happy paths plus error/retry/timeout/filter branches that the
 * config-driven factory tests don't cover.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  WebhookSubscriber,
  A2ASubscriber,
  FileSubscriber,
  StdoutSubscriber,
  FilterSubscriber,
} from '../src/events/subscribers.js';
import { EventEmitter, createEvent } from '../src/events/emitter.js';
import type { ApCoreEvent, EventSubscriber } from '../src/events/emitter.js';

function makeEvent(overrides: Partial<ApCoreEvent> = {}): ApCoreEvent {
  return {
    eventType: 'apcore.test',
    moduleId: 'mod.a',
    timestamp: '2026-01-01T00:00:00Z',
    severity: 'info',
    data: { x: 1 },
    ...overrides,
  };
}

describe('WebhookSubscriber', () => {
  beforeEach(() => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('POSTs the event payload with default and custom headers', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 200 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new WebhookSubscriber('https://example.test/hook', { 'X-Token': 'abc' }, 5000);
    await sub.onEvent(makeEvent());

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('https://example.test/hook');
    const headers = (init as { headers: Record<string, string> }).headers;
    expect(headers['Content-Type']).toBe('application/json');
    expect(headers['X-Token']).toBe('abc');
    const body = JSON.parse((init as { body: string }).body);
    expect(body.event_type).toBe('apcore.test');
  });

  it('warns but does not throw on 4xx (non-retryable client error)', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 404 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new WebhookSubscriber('https://x/y', undefined, 5000);
    // v0.22.0 A-D-EVT-001: subscriber-internal retry removed. 4xx returns
    // normally; the unified EventEmitter retry policy handles 5xx via rethrow.
    await sub.onEvent(makeEvent());
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('rethrows on 5xx so EventEmitter retry policy applies', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 503 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new WebhookSubscriber('https://x/y', undefined, 5000);
    await expect(sub.onEvent(makeEvent())).rejects.toThrow(/503/);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('rethrows on fetch rejection (network error) so EventEmitter retry policy applies', async () => {
    const fetchMock = vi.fn().mockRejectedValue(new Error('ECONNREFUSED'));
    vi.stubGlobal('fetch', fetchMock);

    const sub = new WebhookSubscriber('https://x/y', undefined, 5000);
    await expect(sub.onEvent(makeEvent())).rejects.toThrow('ECONNREFUSED');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('uses an empty headers map when none are provided', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 200 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new WebhookSubscriber('https://x/y');
    await sub.onEvent(makeEvent());
    const headers = (fetchMock.mock.calls[0][1] as { headers: Record<string, string> }).headers;
    expect(headers['Content-Type']).toBe('application/json');
  });

  it('exposes the spec-default retry config (3 attempts, 100ms initial)', () => {
    const sub = new WebhookSubscriber('https://x/y');
    expect(sub.retry.maxAttempts).toBe(3);
    expect(sub.retry.initialBackoffMs).toBe(100);
    expect(sub.retry.backoffMultiplier).toBe(2);
    expect(sub.retry.maxBackoffMs).toBe(30_000);
  });

  it('accepts a custom retry config via the opts argument', () => {
    const sub = new WebhookSubscriber(
      'https://x/y',
      undefined,
      { retry: { maxAttempts: 5, initialBackoffMs: 250 } },
    );
    expect(sub.retry.maxAttempts).toBe(5);
    expect(sub.retry.initialBackoffMs).toBe(250);
  });
});

// Regression lock for the 4xx-vs-5xx HTTP retry contract (apcore issue #69).
//
// The shared conformance driver fails attempts via a generic throw, which
// BYPASSES the subscriber's HTTP-status logic — so the 4xx-permanent /
// 5xx-retryable distinction is unguarded in the conformance layer. This block
// drives delivery through the real EventEmitter retry pipeline and asserts the
// contract via HTTP-call count (the load-bearing observable).
//
// Verified contract (LOCKED, not changed here):
//   * HTTP 4xx  → permanent client error → NOT retried (fetch called once),
//                 no apcore.event.delivery_failed (DLQ) event.
//   * HTTP 5xx  → server error → retried to maxAttempts (fetch called N times),
//                 then a apcore.event.delivery_failed (DLQ) event is emitted.
//
// A WebhookSubscriber has no eventPattern (catch-all), but the emitter excludes
// catch-all subscribers from DLQ events (A-D-026), so it never re-fetches on the
// DLQ event. Fast retry config (1ms backoff) keeps the test sub-millisecond.
describe('WebhookSubscriber HTTP retry contract (issue #69)', () => {
  const FAST_RETRY = {
    maxAttempts: 3,
    initialBackoffMs: 1,
    maxBackoffMs: 10,
    backoffMultiplier: 1,
  };

  beforeEach(() => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('HTTP 4xx (400) is permanent: fetch called exactly once, no DLQ event', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 400 });
    vi.stubGlobal('fetch', fetchMock);

    const emitter = new EventEmitter();
    const dlqEvents: ApCoreEvent[] = [];
    emitter.subscribe({
      eventPattern: 'apcore.event.delivery_failed',
      onEvent: (e) => { dlqEvents.push(e); },
    });
    emitter.subscribe(
      new WebhookSubscriber('https://x/y', undefined, { retry: FAST_RETRY }),
    );

    emitter.emit(createEvent('test.fourxx', null, 'info', { x: 1 }));
    await emitter.flush();

    // Core lock: a 4xx must NOT be retried.
    expect(fetchMock).toHaveBeenCalledTimes(1);
    // 4xx returns normally (warn, no throw) → delivery succeeds → no DLQ.
    expect(dlqEvents).toHaveLength(0);
  });

  it('HTTP 5xx (503) is retryable: fetch called maxAttempts times, then DLQ event', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 503 });
    vi.stubGlobal('fetch', fetchMock);

    const emitter = new EventEmitter();
    const dlqEvents: ApCoreEvent[] = [];
    emitter.subscribe({
      eventPattern: 'apcore.event.delivery_failed',
      onEvent: (e) => { dlqEvents.push(e); },
    });
    emitter.subscribe(
      new WebhookSubscriber('https://x/y', undefined, { retry: FAST_RETRY }),
    );

    emitter.emit(createEvent('test.fivexx', null, 'info', { x: 1 }));
    await emitter.flush();

    // Core lock: a 5xx is retried to exhaustion (3 attempts).
    expect(fetchMock).toHaveBeenCalledTimes(3);
    // Retry exhaustion routes through the DLQ.
    expect(dlqEvents).toHaveLength(1);
    expect(dlqEvents[0].data['subscriber_type']).toBe('webhook');
    expect(dlqEvents[0].data['attempt_count']).toBe(3);
  });
});

// Regression lock for the 4xx-vs-5xx HTTP retry contract (apcore issue #69),
// A2A variant. Mirrors the WebhookSubscriber contract block above exactly: the
// A2ASubscriber must apply the same 5xx-retryable / 4xx-permanent distinction.
//
// Verified contract:
//   * HTTP 4xx  → permanent client error → NOT retried (fetch called once),
//                 no apcore.event.delivery_failed (DLQ) event.
//   * HTTP 5xx  → server error → retried to maxAttempts (fetch called N times),
//                 then a apcore.event.delivery_failed (DLQ) event is emitted
//                 with subscriber_type === "a2a".
describe('A2ASubscriber HTTP retry contract (issue #69)', () => {
  const FAST_RETRY = {
    maxAttempts: 3,
    initialBackoffMs: 1,
    maxBackoffMs: 10,
    backoffMultiplier: 1,
  };

  beforeEach(() => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('HTTP 4xx (400) is permanent: fetch called exactly once, no DLQ event', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 400 });
    vi.stubGlobal('fetch', fetchMock);

    const emitter = new EventEmitter();
    const dlqEvents: ApCoreEvent[] = [];
    emitter.subscribe({
      eventPattern: 'apcore.event.delivery_failed',
      onEvent: (e) => { dlqEvents.push(e); },
    });
    emitter.subscribe(
      new A2ASubscriber('https://platform/x', undefined, { retry: FAST_RETRY }),
    );

    emitter.emit(createEvent('test.fourxx', null, 'info', { x: 1 }));
    await emitter.flush();

    // Core lock: a 4xx must NOT be retried.
    expect(fetchMock).toHaveBeenCalledTimes(1);
    // 4xx returns normally (warn, no throw) → delivery succeeds → no DLQ.
    expect(dlqEvents).toHaveLength(0);
  });

  it('HTTP 5xx (503) is retryable: fetch called maxAttempts times, then DLQ event', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 503 });
    vi.stubGlobal('fetch', fetchMock);

    const emitter = new EventEmitter();
    const dlqEvents: ApCoreEvent[] = [];
    emitter.subscribe({
      eventPattern: 'apcore.event.delivery_failed',
      onEvent: (e) => { dlqEvents.push(e); },
    });
    emitter.subscribe(
      new A2ASubscriber('https://platform/x', undefined, { retry: FAST_RETRY }),
    );

    emitter.emit(createEvent('test.fivexx', null, 'info', { x: 1 }));
    await emitter.flush();

    // Core lock: a 5xx is retried to exhaustion (3 attempts).
    expect(fetchMock).toHaveBeenCalledTimes(3);
    // Retry exhaustion routes through the DLQ.
    expect(dlqEvents).toHaveLength(1);
    expect(dlqEvents[0].data['subscriber_type']).toBe('a2a');
    expect(dlqEvents[0].data['attempt_count']).toBe(3);
  });
});

describe('A2ASubscriber', () => {
  beforeEach(() => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('POSTs with skillId and Bearer auth when auth is a string', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 200 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new A2ASubscriber('https://platform/x', 'tok-1', 5000);
    await sub.onEvent(makeEvent());
    const headers = (fetchMock.mock.calls[0][1] as { headers: Record<string, string> }).headers;
    expect(headers['Authorization']).toBe('Bearer tok-1');
    const body = JSON.parse((fetchMock.mock.calls[0][1] as { body: string }).body);
    expect(body.skillId).toBe('apevo.event_receiver');
  });

  it('merges dict auth as header overrides and skips non-string values', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 200 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new A2ASubscriber('https://platform/x', { 'X-API-Key': 'k', 'X-Skip': 42 as unknown as string }, 5000);
    await sub.onEvent(makeEvent());
    const headers = (fetchMock.mock.calls[0][1] as { headers: Record<string, string> }).headers;
    expect(headers['X-API-Key']).toBe('k');
    expect(headers['X-Skip']).toBeUndefined();
  });

  it('omits Authorization when no auth is given', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 200 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new A2ASubscriber('https://platform/x');
    await sub.onEvent(makeEvent());
    const headers = (fetchMock.mock.calls[0][1] as { headers: Record<string, string> }).headers;
    expect(headers['Authorization']).toBeUndefined();
  });

  it('rethrows on 5xx so EventEmitter retry+DLQ policy applies', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 500 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new A2ASubscriber('https://platform/x');
    await expect(sub.onEvent(makeEvent())).rejects.toThrow(/status 500/);
  });

  it('warns but does not throw on 4xx (non-retryable client error)', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ status: 404 });
    vi.stubGlobal('fetch', fetchMock);

    const sub = new A2ASubscriber('https://platform/x');
    await sub.onEvent(makeEvent());
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('rethrows on fetch rejection so EventEmitter retry+DLQ policy applies', async () => {
    const fetchMock = vi.fn().mockRejectedValue(new Error('boom'));
    vi.stubGlobal('fetch', fetchMock);

    const sub = new A2ASubscriber('https://platform/x');
    await expect(sub.onEvent(makeEvent())).rejects.toThrow('boom');
  });

  it('exposes the spec-default retry config (3 attempts, 100ms initial)', () => {
    const sub = new A2ASubscriber('https://platform/x');
    expect(sub.retry.maxAttempts).toBe(3);
    expect(sub.retry.initialBackoffMs).toBe(100);
    expect(sub.retry.backoffMultiplier).toBe(2);
    expect(sub.retry.maxBackoffMs).toBe(30_000);
  });

  it('accepts a custom retry config via the opts argument', () => {
    const sub = new A2ASubscriber('https://platform/x', undefined, {
      retry: { maxAttempts: 5 },
    });
    expect(sub.retry.maxAttempts).toBe(5);
  });
});

describe('FileSubscriber', () => {
  let tmpDir: string;
  let filePath: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-events-'));
    filePath = path.join(tmpDir, 'events.log');
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    vi.restoreAllMocks();
  });

  it('appends a JSON line per event by default', async () => {
    const sub = new FileSubscriber(filePath);
    await sub.onEvent(makeEvent({ eventType: 'a' }));
    await sub.onEvent(makeEvent({ eventType: 'b' }));

    const lines = fs.readFileSync(filePath, 'utf-8').split('\n').filter(Boolean);
    expect(lines).toHaveLength(2);
    const first = JSON.parse(lines[0]);
    expect(first.event_type).toBe('a');
  });

  it('writes plain-text format when format=text', async () => {
    const sub = new FileSubscriber(filePath, true, 'text');
    await sub.onEvent(makeEvent({ severity: 'warn' }));
    const contents = fs.readFileSync(filePath, 'utf-8');
    expect(contents).toContain('[WARN]');
    expect(contents).toContain('apcore.test');
  });

  it('overwrites the file when append=false', async () => {
    fs.writeFileSync(filePath, 'preexisting\n', 'utf-8');
    const sub = new FileSubscriber(filePath, false, 'json');
    await sub.onEvent(makeEvent({ eventType: 'fresh' }));
    const contents = fs.readFileSync(filePath, 'utf-8');
    expect(contents).not.toContain('preexisting');
    expect(contents).toContain('fresh');
  });

  it('rotates the file when rotate_bytes threshold is exceeded', async () => {
    fs.writeFileSync(filePath, 'x'.repeat(2000), 'utf-8');
    const sub = new FileSubscriber(filePath, true, 'json', 1000);
    await sub.onEvent(makeEvent());
    expect(fs.existsSync(`${filePath}.1`)).toBe(true);
    // After rotation, the new file holds only the freshly written event.
    const contents = fs.readFileSync(filePath, 'utf-8');
    expect(contents).not.toContain('x'.repeat(20));
  });

  it('does not rotate when the file is below the threshold', async () => {
    const sub = new FileSubscriber(filePath, true, 'json', 1_000_000);
    await sub.onEvent(makeEvent());
    expect(fs.existsSync(`${filePath}.1`)).toBe(false);
  });

  it('skips rotation silently when the file does not yet exist', async () => {
    const sub = new FileSubscriber(filePath, true, 'json', 100);
    await sub.onEvent(makeEvent());
    expect(fs.existsSync(filePath)).toBe(true);
    expect(fs.existsSync(`${filePath}.1`)).toBe(false);
  });

  it('rethrows write errors so they reach retry/DLQ', async () => {
    const warn = vi.spyOn(console, 'warn');
    // Plant a directory at the target path so writeFileSync fails with EISDIR.
    fs.mkdirSync(filePath, { recursive: true });
    const sub = new FileSubscriber(filePath, true, 'json');
    await expect(sub.onEvent(makeEvent())).rejects.toThrow();
    expect(warn).toHaveBeenCalled();
  });

  it('a failing file write produces a delivery_failed DLQ event', async () => {
    // Plant a directory at the target path so writeFileSync fails with EISDIR.
    fs.mkdirSync(filePath, { recursive: true });
    const emitter = new EventEmitter();
    const dlqEvents: ApCoreEvent[] = [];
    emitter.subscribe({
      eventPattern: 'apcore.event.delivery_failed',
      onEvent: (e) => { dlqEvents.push(e); },
    });
    const sub = new FileSubscriber(filePath, true, 'json', undefined, 'file-sub-1');
    emitter.subscribe({
      subscriberId: 'file-sub-1',
      retry: { maxAttempts: 1, initialBackoffMs: 1, maxBackoffMs: 10, backoffMultiplier: 1 },
      onEvent: (e) => sub.onEvent(e),
    });
    emitter.emit(createEvent('test.event', null, 'info', {}));
    await emitter.flush();
    expect(dlqEvents.length).toBe(1);
    expect(dlqEvents[0].data['subscriber_id']).toBe('file-sub-1');
  });
});

describe('StdoutSubscriber', () => {
  // Vitest's narrow MockInstance generic for overloaded fns is unhelpful here;
  // capture writes ourselves and stub stdout.write with a plain function.
  let writes: string[];
  let originalWrite: typeof process.stdout.write;

  beforeEach(() => {
    writes = [];
    originalWrite = process.stdout.write;
    process.stdout.write = ((chunk: unknown): boolean => {
      writes.push(String(chunk));
      return true;
    }) as typeof process.stdout.write;
  });
  afterEach(() => {
    process.stdout.write = originalWrite;
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('writes a text line for each event by default', async () => {
    const sub = new StdoutSubscriber();
    await sub.onEvent(makeEvent({ severity: 'error' }));
    expect(writes).toHaveLength(1);
    expect(writes[0]).toContain('[ERROR]');
  });

  it('writes JSON when format=json', async () => {
    const sub = new StdoutSubscriber('json');
    await sub.onEvent(makeEvent());
    const parsed = JSON.parse(writes[0].trim());
    expect(parsed.event_type).toBe('apcore.test');
  });

  it('respects the levelFilter — drops events below the threshold', async () => {
    const sub = new StdoutSubscriber('text', 'warn');
    await sub.onEvent(makeEvent({ severity: 'info' }));
    expect(writes).toHaveLength(0);

    await sub.onEvent(makeEvent({ severity: 'error' }));
    expect(writes).toHaveLength(1);
  });

  it('warns about unknown levelFilter values but still passes events through', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const sub = new StdoutSubscriber('text', 'bogus');
    await sub.onEvent(makeEvent());
    expect(warn).toHaveBeenCalled();
    expect(writes).toHaveLength(1);
  });

  it('treats unknown event severity as level 0', async () => {
    const sub = new StdoutSubscriber('text', 'warn');
    await sub.onEvent(makeEvent({ severity: 'mystery' }));
    expect(writes).toHaveLength(0);
  });
});

describe('FilterSubscriber', () => {
  function recordingDelegate(): { delegate: EventSubscriber; received: ApCoreEvent[] } {
    const received: ApCoreEvent[] = [];
    return {
      received,
      delegate: { onEvent(e) { received.push(e); } },
    };
  }

  it('forwards all events when no include/exclude is set', async () => {
    const { delegate, received } = recordingDelegate();
    const sub = new FilterSubscriber(delegate);
    await sub.onEvent(makeEvent({ eventType: 'a' }));
    await sub.onEvent(makeEvent({ eventType: 'b' }));
    expect(received).toHaveLength(2);
  });

  it('forwards only events matching include patterns (with glob)', async () => {
    const { delegate, received } = recordingDelegate();
    const sub = new FilterSubscriber(delegate, ['apcore.module.*']);
    await sub.onEvent(makeEvent({ eventType: 'apcore.module.registered' }));
    await sub.onEvent(makeEvent({ eventType: 'apcore.config.changed' }));
    expect(received.map(e => e.eventType)).toEqual(['apcore.module.registered']);
  });

  it('drops events matching exclude patterns when no include is set', async () => {
    const { delegate, received } = recordingDelegate();
    const sub = new FilterSubscriber(delegate, undefined, ['debug.*']);
    await sub.onEvent(makeEvent({ eventType: 'debug.x' }));
    await sub.onEvent(makeEvent({ eventType: 'apcore.x' }));
    expect(received.map(e => e.eventType)).toEqual(['apcore.x']);
  });

  it('include takes precedence over exclude when both are provided', async () => {
    const { delegate, received } = recordingDelegate();
    const sub = new FilterSubscriber(delegate, ['apcore.*'], ['apcore.*']);
    await sub.onEvent(makeEvent({ eventType: 'apcore.x' }));
    expect(received.map(e => e.eventType)).toEqual(['apcore.x']);
  });

  it('? glob matches a single character', async () => {
    const { delegate, received } = recordingDelegate();
    const sub = new FilterSubscriber(delegate, ['x?z']);
    await sub.onEvent(makeEvent({ eventType: 'xaz' }));
    await sub.onEvent(makeEvent({ eventType: 'xabz' }));
    expect(received.map(e => e.eventType)).toEqual(['xaz']);
  });

  // A-D-022: FilterSubscriber must expose subscriberId/retry like the other
  // subscriber types (Python subscribers.py accepts id + retry too).
  it('exposes a generated subscriberId and DEFAULT_RETRY by default', () => {
    const { delegate } = recordingDelegate();
    const sub = new FilterSubscriber(delegate);
    expect(typeof sub.subscriberId).toBe('string');
    expect(sub.subscriberId.length).toBeGreaterThan(0);
    expect(sub.retry.maxAttempts).toBe(3);
  });

  it('honors id and retry passed via the options object', () => {
    const { delegate } = recordingDelegate();
    const sub = new FilterSubscriber(delegate, undefined, undefined, {
      id: 'filter-1',
      retry: { maxAttempts: 5 },
    });
    expect(sub.subscriberId).toBe('filter-1');
    expect(sub.retry.maxAttempts).toBe(5);
  });
});
