/**
 * W3C Trace Context support: TraceParent + tracestate parsing and TraceContext
 * injection/extraction.
 */

import type { Context } from './context.js';
import { InvalidParentIdError } from './errors.js';
import type { Span } from './observability/tracing.js';
import { randomHex } from './utils/index.js';

const TRACEPARENT_RE = /^([0-9a-f]{2})-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/;
const PARENT_ID_RE = /^[0-9a-f]{16}$/;
const TRACESTATE_KEY_RE = /^[a-z0-9][a-z0-9_\-*/@]{0,255}$/;
/** W3C tracestate hard cap (RFC: SHOULD discard beyond 32 entries). */
const MAX_TRACESTATE_ENTRIES = 32;
/**
 * Reserved keys under `context.data` carrying the parsed inbound W3C trace
 * state. Cross-language parity: apcore-python/-rust store the flags and
 * tracestate as two scalar keys rather than the whole TraceParent object.
 */
const TRACE_FLAGS_KEY = '_apcore.trace.flags';
const TRACESTATE_KEY = '_apcore.trace.state';
/** Legacy well-known key carrying the whole inbound TraceParent (back-compat). */
const INBOUND_KEY = '_apcore.trace.inbound';

export interface TraceParent {
  readonly version: string;   // "00"
  readonly traceId: string;   // 32 lowercase hex chars
  readonly parentId: string;  // 16 lowercase hex chars
  readonly traceFlags: string; // "01" (sampled) or "00"
  /** Parsed `tracestate` entries. Empty array when the header is absent. */
  readonly tracestate: ReadonlyArray<readonly [string, string]>;
}

/**
 * Headers container the extractor accepts. Plain dict, fetch `Headers`, or `Map`.
 *
 * For plain objects, lookup folds keys to lowercase. For `Headers`, the platform
 * already performs case-insensitive matching. For `Map`, lookup folds keys.
 */
export type HeadersLike =
  | Record<string, unknown>
  | Headers
  | Map<string, string>;

export class TraceContext {
  /**
   * Build outbound trace headers from an apcore Context.
   *
   * Behavior:
   * - Uses `context.traceId` (already 32-hex) as the traceparent trace_id.
   * - When `parentId` is supplied it is used verbatim and MUST match `^[0-9a-f]{16}$`;
   *   otherwise the last span on the tracing stack provides the id, falling back
   *   to a fresh random 16-hex value.
   * - When the Context carries an inbound TraceParent under
   *   `data['_apcore.trace.inbound']`, its `traceFlags` and `tracestate` are
   *   propagated; otherwise traceFlags defaults to `"01"` (sampled) for new roots.
   *
   * @param context apcore Context
   * @param parentId optional 16-hex parent id override
   * @throws {InvalidParentIdError} if `parentId` is provided but malformed
   */
  static inject(context: Context, parentId?: string): Record<string, string> {
    const traceIdHex = context.traceId; // already 32-hex format

    let chosenParent: string;
    if (parentId !== undefined) {
      if (!PARENT_ID_RE.test(parentId)) {
        throw new InvalidParentIdError(parentId);
      }
      chosenParent = parentId;
    } else {
      const spansStack = context.data['_apcore.mw.tracing.spans'] as Span[] | undefined;
      if (spansStack && spansStack.length > 0) {
        chosenParent = spansStack[spansStack.length - 1].spanId;
      } else {
        chosenParent = randomHex(8);
      }
    }

    // Read the parsed inbound trace state from the two scalar keys, falling
    // back to a legacy whole-TraceParent stash for back-compat.
    const inbound = context.data[INBOUND_KEY] as TraceParent | undefined;
    const flags =
      (context.data[TRACE_FLAGS_KEY] as string | undefined) ?? inbound?.traceFlags ?? '01';

    const headers: Record<string, string> = {
      traceparent: `00-${traceIdHex}-${chosenParent}-${flags}`,
    };

    const stateEntries =
      (context.data[TRACESTATE_KEY] as ReadonlyArray<readonly [string, string]> | undefined) ??
      inbound?.tracestate ??
      [];
    if (stateEntries.length > 0) {
      headers.tracestate = TraceContext.formatTracestate(stateEntries);
    }
    return headers;
  }

  /**
   * Parse the `traceparent` (and optional `tracestate`) header from a header
   * container, performing case-insensitive header name lookup.
   *
   * Returns `null` if the traceparent header is missing or malformed.
   */
  static extract(headers: HeadersLike): TraceParent | null {
    const raw = TraceContext.lookupHeaderCi(headers, 'traceparent');
    if (raw === undefined) {
      return null;
    }
    const match = TRACEPARENT_RE.exec(raw.trim().toLowerCase());
    if (match === null) {
      return null;
    }
    const version = match[1];
    const traceId = match[2];
    const parentId = match[3];
    if (version === 'ff') {
      return null;
    }
    if (traceId === '0'.repeat(32) || parentId === '0'.repeat(16)) {
      return null;
    }
    const tracestateRaw = TraceContext.lookupHeaderCi(headers, 'tracestate');
    const tracestate = tracestateRaw !== undefined
      ? Object.freeze(TraceContext.parseTracestate(tracestateRaw).map((p) => Object.freeze(p) as readonly [string, string]))
      : Object.freeze([] as ReadonlyArray<readonly [string, string]>);
    return Object.freeze({
      version,
      traceId,
      parentId,
      traceFlags: match[4],
      tracestate,
    });
  }

  /**
   * Strictly parse a traceparent string, throwing on invalid format.
   *
   * @throws {Error} If the traceparent does not match the expected
   *   `00-<32 hex>-<16 hex>-<2 hex>` format.
   */
  static fromTraceparent(traceparent: string): TraceParent {
    const match = TRACEPARENT_RE.exec(traceparent.trim().toLowerCase());
    if (match === null) {
      throw new Error(
        `Malformed traceparent: ${JSON.stringify(traceparent.slice(0, 100))}. Expected format: 00-<32 hex>-<16 hex>-<2 hex>`,
      );
    }
    const version = match[1];
    const traceId = match[2];
    const parentId = match[3];
    if (version === 'ff') {
      throw new Error('Invalid traceparent: version ff is not allowed');
    }
    if (traceId === '0'.repeat(32) || parentId === '0'.repeat(16)) {
      throw new Error('Invalid traceparent: all-zero trace_id or parent_id');
    }
    return Object.freeze({
      version,
      traceId,
      parentId,
      traceFlags: match[4],
      tracestate: Object.freeze([] as ReadonlyArray<readonly [string, string]>),
    });
  }

  /**
   * Parse a W3C `tracestate` header value.
   *
   * Splits on commas, trims each member, and accepts only entries shaped as
   * `key=value` (the value may contain `=`). Malformed members are dropped
   * silently. The list is truncated to {@link MAX_TRACESTATE_ENTRIES} (32).
   */
  static parseTracestate(raw: string): Array<[string, string]> {
    if (raw.length === 0) {
      return [];
    }
    const entries: Array<[string, string]> = [];
    for (const member of raw.split(',')) {
      const trimmed = member.trim();
      if (trimmed.length === 0) {
        continue;
      }
      const eq = trimmed.indexOf('=');
      if (eq <= 0) {
        // Missing key (eq === 0) or no '=' (eq === -1). Drop silently.
        continue;
      }
      const key = trimmed.slice(0, eq).trim();
      const value = trimmed.slice(eq + 1).trim();
      if (key.length === 0 || value.length === 0) {
        continue;
      }
      // RFC 9: be lenient. Tolerate keys that fail strict regex but at least
      // require non-empty key/value. Strict validation may be opted into by
      // upstream callers if needed.
      if (!TRACESTATE_KEY_RE.test(key)) {
        // Still drop blatantly malformed keys (e.g., contains '=' or whitespace).
        if (/[\s=,]/.test(key)) {
          continue;
        }
      }
      entries.push([key, value]);
      if (entries.length >= MAX_TRACESTATE_ENTRIES) {
        break;
      }
    }
    return entries;
  }

  /**
   * Format tracestate entries to a comma-separated header value.
   */
  static formatTracestate(entries: ReadonlyArray<readonly [string, string]>): string {
    return entries.map(([k, v]) => `${k}=${v}`).join(',');
  }

  /**
   * Case-insensitive header lookup across plain objects, `Headers`, and `Map`.
   *
   * @returns the header value (or `undefined` if not present).
   */
  static lookupHeaderCi(headers: HeadersLike, name: string): string | undefined {
    const target = name.toLowerCase();
    // Headers (fetch API): already case-insensitive.
    if (typeof Headers !== 'undefined' && headers instanceof Headers) {
      const v = headers.get(name);
      return v === null ? undefined : v;
    }
    // Map<string, string>
    if (headers instanceof Map) {
      for (const [k, v] of headers.entries()) {
        if (typeof k === 'string' && k.toLowerCase() === target) {
          return typeof v === 'string' ? v : String(v);
        }
      }
      return undefined;
    }
    // Plain object
    for (const [k, v] of Object.entries(headers)) {
      if (k.toLowerCase() === target) {
        if (typeof v === 'string') return v;
        if (v === undefined || v === null) return undefined;
        return String(v);
      }
    }
    return undefined;
  }
}
