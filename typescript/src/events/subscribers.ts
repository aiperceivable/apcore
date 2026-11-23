/**
 * Event subscribers for webhook, A2A, file, stdout, and filter delivery.
 */

import * as fs from 'node:fs';
import type { ApCoreEvent, EventSubscriber } from './emitter.js';
import { DEFAULT_RETRY } from './retry.js';
import type { RetryConfig } from './retry.js';
import { matchGlob } from '../utils/pattern.js';

const SEVERITY_ORDER: Record<string, number> = { info: 0, warn: 1, error: 2, fatal: 3 };

// Per-type monotonic counters for auto-generated subscriber IDs.
// Pattern: `^{type}-[0-9]+$` per spec event_delivery_semantics fixture.
const _subscriberCounters = new Map<string, number>();
function _nextSubscriberId(typeName: string): string {
  const next = (_subscriberCounters.get(typeName) ?? 0);
  _subscriberCounters.set(typeName, next + 1);
  return `${typeName}-${next}`;
}

/**
 * Delivers events via HTTP POST to a webhook URL.
 *
 * Retries on 5xx and connection errors. As of v0.22.0 finding A-D-EVT-001,
 * the retry policy is unified across event subscribers: the subscriber MUST
 * NOT loop internally. Instead, it rethrows on 5xx / network errors and the
 * outer `EventEmitter._deliver` applies the spec retry policy declared via
 * the `retry` field (defaults to `DEFAULT_RETRY` — 3 attempts, 100 ms initial
 * backoff, 2× multiplier, 30 s cap). 4xx responses are treated as
 * non-retryable: a warning is logged and the call returns normally.
 */
export class WebhookSubscriber implements EventSubscriber {
  /** Declared subscriber kind for DLQ payloads (A-D-029). */
  readonly subscriberType = 'webhook';
  readonly subscriberId: string;
  readonly retry: RetryConfig;
  private readonly _url: string;
  private readonly _headers: Record<string, string>;
  private readonly _timeoutMs: number;

  constructor(
    url: string,
    headers?: Record<string, string>,
    timeoutMsOrOpts: number | { timeoutMs?: number; retry?: RetryConfig; id?: string } = 5000,
    id?: string,
  ) {
    if (typeof timeoutMsOrOpts === 'number') {
      this._timeoutMs = timeoutMsOrOpts;
      this.retry = { ...DEFAULT_RETRY };
      this.subscriberId = id ?? _nextSubscriberId('webhook');
    } else {
      this._timeoutMs = timeoutMsOrOpts.timeoutMs ?? 5000;
      this.retry = { ...DEFAULT_RETRY, ...(timeoutMsOrOpts.retry ?? {}) };
      this.subscriberId = timeoutMsOrOpts.id ?? id ?? _nextSubscriberId('webhook');
    }
    this._url = url;
    this._headers = headers ?? {};
  }

  async onEvent(event: ApCoreEvent): Promise<void> {
    const payload: Record<string, unknown> = {
      event_type: event.eventType,
      module_id: event.moduleId,
      timestamp: event.timestamp,
      severity: event.severity,
      data: event.data,
    };
    const mergedHeaders: Record<string, string> = {
      'Content-Type': 'application/json',
      ...this._headers,
    };

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this._timeoutMs);
    try {
      const response = await fetch(this._url, {
        method: 'POST',
        headers: mergedHeaders,
        body: JSON.stringify(payload),
        signal: controller.signal,
      });

      if (response.status >= 500) {
        // Server error — rethrow so the EventEmitter retry policy applies.
        throw new Error(`Webhook ${this._url} returned ${response.status}`);
      }
      if (response.status >= 400) {
        // Client error — non-retryable; log and return.
        console.warn(
          '[apcore:events]',
          `Webhook ${this._url} returned ${response.status} for event ${event.eventType}`,
        );
      }
    } finally {
      clearTimeout(timer);
    }
  }
}

/**
 * Delivers events via the A2A protocol to the platform.
 *
 * Sends a POST with `skillId="apevo.event_receiver"` and the
 * serialized event in the payload. As of v0.22.0 finding A-D-EVT-001 the
 * subscriber no longer silently swallows errors. Mirroring
 * `WebhookSubscriber`, it rethrows on 5xx / network errors so the unified
 * `EventEmitter._deliver` retry policy applies (defaults to `DEFAULT_RETRY`
 * — 3 attempts, 100 ms initial backoff, 2× multiplier, 30 s cap); after
 * exhaustion the EventEmitter routes the failure through the DLQ +
 * `onFailure` path. 4xx responses are treated as non-retryable: a warning
 * is logged and the call returns normally (no retry, no DLQ).
 */
export class A2ASubscriber implements EventSubscriber {
  /** Declared subscriber kind for DLQ payloads (A-D-029). */
  readonly subscriberType = 'a2a';
  readonly subscriberId: string;
  readonly retry: RetryConfig;
  private readonly _platformUrl: string;
  private readonly _auth: string | Record<string, string> | undefined;
  private readonly _timeoutMs: number;
  private readonly _skillId: string;

  constructor(
    platformUrl: string,
    auth?: string | Record<string, string | unknown>,
    timeoutMsOrOpts:
      | number
      | { timeoutMs?: number; retry?: RetryConfig; id?: string; skillId?: string } = 5000,
    id?: string,
    skillId: string = 'apevo.event_receiver',
  ) {
    if (typeof timeoutMsOrOpts === 'number') {
      this._timeoutMs = timeoutMsOrOpts;
      this.retry = { ...DEFAULT_RETRY };
      this.subscriberId = id ?? _nextSubscriberId('a2a');
      this._skillId = skillId;
    } else {
      this._timeoutMs = timeoutMsOrOpts.timeoutMs ?? 5000;
      this.retry = { ...DEFAULT_RETRY, ...(timeoutMsOrOpts.retry ?? {}) };
      this.subscriberId = timeoutMsOrOpts.id ?? id ?? _nextSubscriberId('a2a');
      this._skillId = timeoutMsOrOpts.skillId ?? skillId;
    }
    this._platformUrl = platformUrl;
    this._auth = auth as string | Record<string, string> | undefined;
  }

  async onEvent(event: ApCoreEvent): Promise<void> {
    const payload: Record<string, unknown> = {
      skillId: this._skillId,
      event: {
        event_type: event.eventType,
        module_id: event.moduleId,
        timestamp: event.timestamp,
        severity: event.severity,
        data: event.data,
      },
    };

    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };

    if (this._auth !== undefined) {
      if (typeof this._auth === 'string') {
        // String auth → Bearer token (per spec)
        headers['Authorization'] = `Bearer ${this._auth}`;
      } else {
        // Dict auth → merge as header overrides
        for (const [key, value] of Object.entries(this._auth)) {
          if (typeof value === 'string') {
            headers[key] = value;
          }
        }
      }
    }

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this._timeoutMs);
    try {
      const response = await fetch(this._platformUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify(payload),
        signal: controller.signal,
      });

      if (response.status >= 500) {
        // Server error — rethrow so the EventEmitter retry policy applies.
        throw new Error(
          `A2A delivery to ${this._platformUrl} failed with status ${response.status}`,
        );
      }
      if (response.status >= 400) {
        // Client error — non-retryable; log and return.
        console.warn(
          '[apcore:events]',
          `A2A delivery to ${this._platformUrl} returned ${response.status} for event ${event.eventType}`,
        );
      }
    } finally {
      clearTimeout(timer);
    }
  }
}

/** Writes events to a local file (built-in type: 'file'). */
export class FileSubscriber implements EventSubscriber {
  /** Declared subscriber kind for DLQ payloads (A-D-029). */
  readonly subscriberType = 'file';
  readonly subscriberId: string;
  /**
   * Retry policy applied by EventEmitter._deliver (apcore#85). `onEvent`
   * re-throws on write failure, so this policy governs real re-delivery of
   * transient I/O errors.
   */
  readonly retry: RetryConfig;
  private readonly _path: string;
  private readonly _append: boolean;
  private readonly _format: string;
  private readonly _rotateBytes: number | null;

  constructor(
    path: string,
    append: boolean = true,
    format: string = 'json',
    rotateBytes?: number,
    id?: string,
    opts?: { retry?: RetryConfig },
  ) {
    this.subscriberId = id ?? _nextSubscriberId('file');
    this._path = path;
    this._append = append;
    this._format = format;
    this._rotateBytes = rotateBytes ?? null;
    this.retry = { ...DEFAULT_RETRY, ...(opts?.retry ?? {}) };
  }

  async onEvent(event: ApCoreEvent): Promise<void> {
    try {
      if (this._rotateBytes !== null) {
        try {
          const stat = fs.statSync(this._path);
          if (stat.size >= this._rotateBytes) {
            fs.renameSync(this._path, `${this._path}.1`);
          }
        } catch {
          // file doesn't exist yet — no rotation needed
        }
      }

      const line =
        this._format === 'json'
          ? JSON.stringify({
              event_type: event.eventType,
              module_id: event.moduleId,
              timestamp: event.timestamp,
              severity: event.severity,
              data: event.data,
            }) + '\n'
          : `[${event.timestamp}] [${event.severity.toUpperCase()}] ${event.eventType} module=${event.moduleId} data=${JSON.stringify(event.data)}\n`;

      fs.writeFileSync(this._path, line, { flag: this._append ? 'a' : 'w', encoding: 'utf-8' });
    } catch (err: unknown) {
      console.warn(
        '[apcore:events]',
        `FileSubscriber failed to write event ${event.eventType} to ${this._path}:`,
        err,
      );
      // Surface the failure so it reaches the emitter's retry/DLQ path
      // (mirrors Python/Rust, which re-raise after logging).
      throw err;
    }
  }
}

/** Writes events to stdout (built-in type: 'stdout'). */
export class StdoutSubscriber implements EventSubscriber {
  /** Declared subscriber kind for DLQ payloads (A-D-029). */
  readonly subscriberType = 'stdout';
  readonly subscriberId: string;
  /**
   * Retry policy applied by EventEmitter._deliver (apcore#85). `onEvent`
   * writes straight to process.stdout, whose failures (EPIPE, closed stream)
   * propagate — so this policy governs real re-delivery.
   */
  readonly retry: RetryConfig;
  private readonly _format: string;
  private readonly _levelFilter: string | null;

  constructor(
    format: string = 'text',
    levelFilter?: string,
    id?: string,
    opts?: { retry?: RetryConfig },
  ) {
    this.subscriberId = id ?? _nextSubscriberId('stdout');
    this._format = format;
    this.retry = { ...DEFAULT_RETRY, ...(opts?.retry ?? {}) };
    if (levelFilter !== undefined && !(levelFilter in SEVERITY_ORDER)) {
      console.warn(
        `[apcore:events] StdoutSubscriber: unknown level_filter '${levelFilter}' — valid values: info, warn, error, fatal. All events will pass.`,
      );
    }
    this._levelFilter = levelFilter ?? null;
  }

  async onEvent(event: ApCoreEvent): Promise<void> {
    if (this._levelFilter !== null) {
      const minLevel = SEVERITY_ORDER[this._levelFilter] ?? 0;
      const eventLevel = SEVERITY_ORDER[event.severity] ?? 0;
      if (eventLevel < minLevel) return;
    }

    const line =
      this._format === 'json'
        ? JSON.stringify({
            event_type: event.eventType,
            module_id: event.moduleId,
            timestamp: event.timestamp,
            severity: event.severity,
            data: event.data,
          })
        : `[${event.timestamp}] [${event.severity.toUpperCase()}] ${event.eventType} module=${event.moduleId} data=${JSON.stringify(event.data)}`;

    process.stdout.write(line + '\n');
  }
}

/**
 * Wraps a delegate subscriber with event-name filtering (built-in type: 'filter').
 *
 * Matching rules (evaluated in order):
 *   1. If include_events is set, forward only events matching any pattern.
 *   2. Otherwise if exclude_events is set, discard events matching any pattern.
 *   3. If neither is set, forward all events.
 */
export class FilterSubscriber implements EventSubscriber {
  /** Declared subscriber kind for DLQ payloads (A-D-029). */
  readonly subscriberType = 'filter';
  /** Stable identity for DLQ payloads and dedup (A-D-022). */
  readonly subscriberId: string;
  /** Retry policy applied by EventEmitter._deliver (A-D-022). */
  readonly retry: RetryConfig;
  private readonly _delegate: EventSubscriber;
  private readonly _includeEvents: string[] | null;
  private readonly _excludeEvents: string[] | null;

  constructor(
    delegate: EventSubscriber,
    includeEvents?: string[],
    excludeEvents?: string[],
    opts?: { id?: string; retry?: RetryConfig },
  ) {
    this._delegate = delegate;
    this._includeEvents = includeEvents ?? null;
    this._excludeEvents = excludeEvents ?? null;
    // Mirror the id/retry surface of the other subscriber types (A-D-022;
    // Python subscribers.py FilterSubscriber accepts id + retry too). When
    // retry is omitted, delivery uses DEFAULT_RETRY via EventEmitter._deliver.
    this.subscriberId = opts?.id ?? _nextSubscriberId('filter');
    this.retry = { ...DEFAULT_RETRY, ...(opts?.retry ?? {}) };
  }

  async onEvent(event: ApCoreEvent): Promise<void> {
    if (this._matches(event.eventType)) {
      await this._delegate.onEvent(event);
    }
  }

  private _matches(eventType: string): boolean {
    if (this._includeEvents !== null) {
      return this._includeEvents.some((pattern) => matchGlob(pattern, eventType));
    }
    if (this._excludeEvents !== null) {
      return !this._excludeEvents.some((pattern) => matchGlob(pattern, eventType));
    }
    return true;
  }
}
