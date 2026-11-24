/**
 * Tracing system: Span, SpanExporter implementations, and TracingMiddleware.
 */

import type { Context } from '../context.js';
import { Middleware } from '../middleware/base.js';
import { randomHex } from '../utils/index.js';

export interface Span {
  traceId: string;
  name: string;
  startTime: number;
  spanId: string;
  parentSpanId: string | null;
  attributes: Record<string, unknown>;
  endTime: number | null;
  status: string;
  events: Array<Record<string, unknown>>;
}

export function createSpan(options: {
  traceId: string;
  name: string;
  startTime: number;
  spanId?: string;
  parentSpanId?: string | null;
  attributes?: Record<string, unknown>;
}): Span {
  return {
    traceId: options.traceId,
    name: options.name,
    startTime: options.startTime,
    spanId: options.spanId ?? randomHex(8),
    parentSpanId: options.parentSpanId ?? null,
    attributes: options.attributes ?? {},
    endTime: null,
    status: 'ok',
    events: [],
  };
}

export interface SpanExporter {
  export(span: Span): void;
}

export class StdoutExporter implements SpanExporter {
  export(span: Span): void {
    // Use console.info for universal compatibility (Node.js + browser)
    console.info(JSON.stringify(span));
  }
}

export class InMemoryExporter implements SpanExporter {
  private _spans: Span[] = [];
  private _maxSpans: number;

  constructor(maxSpans: number = 10_000) {
    this._maxSpans = maxSpans;
  }

  export(span: Span): void {
    if (this._spans.length >= this._maxSpans) {
      // Drop oldest half to amortize the cost instead of O(n) shift per insert
      this._spans = this._spans.slice(Math.floor(this._maxSpans / 2));
    }
    this._spans.push(span);
  }

  getSpans(): Span[] {
    return [...this._spans];
  }

  clear(): void {
    this._spans = [];
  }
}

export class OTLPExporter implements SpanExporter {
  private _endpoint: string;
  private _serviceName: string;
  private _headers: Record<string, string>;
  private _timeoutMs: number;

  constructor(options?: {
    endpoint?: string;
    serviceName?: string;
    headers?: Record<string, string>;
    timeoutMs?: number;
  }) {
    this._endpoint = options?.endpoint ?? 'http://localhost:4318/v1/traces';
    this._serviceName = options?.serviceName ?? 'apcore';
    this._headers = options?.headers ?? {};
    this._timeoutMs = options?.timeoutMs ?? 5000;
  }

  export(span: Span): void {
    const payload = {
      resourceSpans: [{
        resource: {
          attributes: [
            { key: 'service.name', value: { stringValue: this._serviceName } },
          ],
        },
        scopeSpans: [{
          scope: { name: 'apcore' },
          spans: [{
            traceId: span.traceId,
            spanId: span.spanId,
            parentSpanId: span.parentSpanId ?? undefined,
            name: span.name,
            startTimeUnixNano: String(Math.round(span.startTime * 1_000_000_000)),
            endTimeUnixNano: span.endTime ? String(Math.round(span.endTime * 1_000_000_000)) : undefined,
            status: { code: span.status === 'ok' ? 1 : 2 },
            attributes: Object.entries(span.attributes).map(([key, value]) => ({
              key,
              value: { stringValue: String(value) },
            })),
          }],
        }],
      }],
    };

    // Fire-and-forget POST with timeout so a stuck OTLP backend does not
    // accumulate pending promises and retain span data in memory.
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this._timeoutMs);
    fetch(this._endpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...this._headers,
      },
      body: JSON.stringify(payload),
      signal: controller.signal,
    })
      .catch((err: unknown) => {
        console.warn('[apcore:tracing] OTLP export failed:', err);
      })
      .finally(() => {
        clearTimeout(timer);
      });
  }
}

const VALID_STRATEGIES = new Set(['full', 'proportional', 'error_first', 'off']);

export class TracingMiddleware extends Middleware {
  private _exporter: SpanExporter;
  private _samplingRate: number;
  private _samplingStrategy: string;

  constructor(
    exporter: SpanExporter,
    samplingRate: number = 1.0,
    samplingStrategy: string = 'full',
  ) {
    super();
    if (samplingRate < 0.0 || samplingRate > 1.0) {
      throw new Error(`sampling_rate must be between 0.0 and 1.0, got ${samplingRate}`);
    }
    if (!VALID_STRATEGIES.has(samplingStrategy)) {
      throw new Error(`sampling_strategy must be one of ${[...VALID_STRATEGIES].join(', ')}, got '${samplingStrategy}'`);
    }
    this._exporter = exporter;
    this._samplingRate = samplingRate;
    this._samplingStrategy = samplingStrategy;
  }

  /** Replace the span exporter used by this middleware. */
  setExporter(exporter: SpanExporter): void {
    if (!exporter || typeof exporter.export !== 'function') {
      throw new Error('exporter must implement SpanExporter interface');
    }
    this._exporter = exporter;
  }

  private _shouldSample(context: Context): boolean {
    const existing = context.data['_apcore.mw.tracing.sampled'];
    if (typeof existing === 'boolean') return existing;

    let decision: boolean;
    if (this._samplingStrategy === 'full') {
      decision = true;
    } else if (this._samplingStrategy === 'off') {
      decision = false;
    } else {
      decision = Math.random() < this._samplingRate;
    }

    context.data['_apcore.mw.tracing.sampled'] = decision;
    return decision;
  }

  override before(
    moduleId: string,
    _inputs: Record<string, unknown>,
    context: Context,
  ): null {
    this._shouldSample(context);

    const spansStack = (context.data['_apcore.mw.tracing.spans'] as Span[]) ?? [];
    context.data['_apcore.mw.tracing.spans'] = spansStack;
    const parentSpanId = spansStack.length > 0 ? spansStack[spansStack.length - 1].spanId : null;

    const span = createSpan({
      traceId: context.traceId,
      name: 'apcore.module.execute',
      startTime: Date.now() / 1000,
      parentSpanId,
      // OBS-003: the attribute map is a WIRE payload, not a
      // language-idiomatic object, so the correlation fields are spelled
      // snake_case — as observability.md names them, as apcore-python and
      // apcore-rust write them, and as the `duration_ms` / `success` /
      // `error_code` attributes written later on this very span already were.
      // A single span used to carry both conventions at once.
      attributes: {
        module_id: moduleId,
        method: 'execute',
        caller_id: context.callerId,
      },
    });
    spansStack.push(span);
    return null;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: Context,
  ): null {
    const spansStack = (context.data['_apcore.mw.tracing.spans'] as Span[]) ?? [];
    if (spansStack.length === 0) return null;

    const span = spansStack.pop()!;
    span.endTime = Date.now() / 1000;
    span.status = 'ok';
    span.attributes['duration_ms'] = (span.endTime - span.startTime) * 1000;
    span.attributes['success'] = true;

    if (context.data['_apcore.mw.tracing.sampled']) {
      this._exporter.export(span);
    }
    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    error: Error,
    context: Context,
  ): null {
    const spansStack = (context.data['_apcore.mw.tracing.spans'] as Span[]) ?? [];
    if (spansStack.length === 0) return null;

    const span = spansStack.pop()!;
    span.endTime = Date.now() / 1000;
    span.status = 'error';
    span.attributes['duration_ms'] = (span.endTime - span.startTime) * 1000;
    span.attributes['success'] = false;
    span.attributes['error_code'] = (error as unknown as Record<string, unknown>)['code'] ?? error.constructor.name;

    const shouldExport =
      this._samplingStrategy === 'error_first' || context.data['_apcore.mw.tracing.sampled'];
    if (shouldExport) {
      this._exporter.export(span);
    }
    return null;
  }
}
