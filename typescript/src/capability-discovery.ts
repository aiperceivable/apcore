import type { Context } from './context.js';
import { InvalidInputError } from './errors.js';
import { copyPolicyInputs } from './policy-evaluation.js';
import { uuidV4 } from './utils/uuid.js';

/** A governance hint for discovery. Actual invocation still enforces the pipeline. */
export type CapabilityAccessHint = 'allowed' | 'requires_approval' | 'conditional';

export interface ModuleSummary {
  readonly moduleId: string;
  readonly version: string;
  readonly description: string;
  readonly tags: readonly string[];
  readonly readonly: boolean;
  readonly accessHint: CapabilityAccessHint;
}

export interface CapabilityPage {
  readonly items: readonly ModuleSummary[];
  readonly nextCursor: string | null;
}

export interface CallableDefinition {
  readonly moduleId: string;
  readonly version: string;
  readonly inputSchema: Record<string, unknown>;
  readonly outputSchema: Record<string, unknown>;
}

export interface CapabilitySearchOptions {
  readonly query?: string;
  readonly prefix?: string;
  readonly tags?: readonly string[];
  readonly readonly?: boolean;
  readonly limit?: number;
  readonly cursor?: string | null;
  readonly includeApprovalRequired?: boolean;
}

/** An optional ranker that returns candidate module IDs in rank order. */
export interface CapabilitySearchProvider {
  search(query: string | undefined, candidates: readonly string[]): readonly string[];
}

type SearchFilters = Required<Omit<CapabilitySearchOptions, 'cursor' | 'readonly'>> &
  Pick<CapabilitySearchOptions, 'readonly'>;

/** @internal Share filter normalization between paging, rankers, and matching. */
export function normalizeCapabilitySearch(options: CapabilitySearchOptions): SearchFilters {
  const { query = '', prefix = '', tags = [], limit = 20, includeApprovalRequired = true } = options;
  if (typeof query !== 'string' || typeof prefix !== 'string') {
    throw new InvalidInputError('Capability query and prefix must be strings');
  }
  if (!Array.isArray(tags) || tags.some(tag => typeof tag !== 'string')) {
    throw new InvalidInputError('Capability tags must be a list of strings');
  }
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) {
    throw new InvalidInputError('Capability limit must be an integer from 1 to 100');
  }
  if (typeof includeApprovalRequired !== 'boolean' ||
      (options.readonly !== undefined && typeof options.readonly !== 'boolean')) {
    throw new InvalidInputError('Capability approval and read-only filters must be booleans');
  }
  return {
    query: query.trim().toLowerCase(),
    prefix,
    tags: [...new Set(tags)].sort(),
    readonly: options.readonly,
    limit, includeApprovalRequired,
  };
}

/** @internal Apply all mandatory filters after visibility has been established. */
export function matchesCapability(summary: ModuleSummary, filters: SearchFilters, lexical: boolean): boolean {
  if (!summary.moduleId.startsWith(filters.prefix) ||
      !filters.tags.every(tag => summary.tags.includes(tag)) ||
      (filters.readonly !== undefined && summary.readonly !== filters.readonly) ||
      (!filters.includeApprovalRequired && summary.accessHint === 'requires_approval')) return false;
  if (!lexical || filters.query === '') return true;
  return [summary.moduleId, summary.description, ...summary.tags].some(
    value => value.toLowerCase().includes(filters.query),
  );
}

interface CursorPosition {
  readonly candidates: readonly string[];
  readonly offset: number;
}

interface CursorRecord extends CursorPosition {
  readonly binding: string;
  readonly expiresAt: number;
}

const MAX_CURSORS = 1024;
const CURSOR_TTL_MS = 5 * 60 * 1000;

function invalidCursor(): InvalidInputError {
  return new InvalidInputError('Invalid or expired capability cursor');
}

function buildCursorBinding(context: Context | null, filters: SearchFilters): string {
  const identity = context?.identity;
  try {
    return copyPolicyInputs('capability discovery', {
      principal: identity === null || identity === undefined ? null : {
        id: identity.id, type: identity.type, roles: [...new Set(identity.roles)].sort(), attrs: identity.attrs,
      },
      callerId: context?.callerId ?? null,
      callChain: [...(context?.callChain ?? [])],
      filters: { ...filters, readonly: filters.readonly ?? null },
    }).canonical.join('');
  } catch {
    throw invalidCursor();
  }
}

/** @internal Executor-scoped, bounded pagination state containing no module metadata. */
export class CapabilityCursorStore {
  private readonly _records = new Map<string, CursorRecord>();

  clear(): void {
    this._records.clear();
  }

  read(cursor: string, context: Context | null, filters: SearchFilters): CursorPosition {
    const record = this._records.get(cursor);
    if (record === undefined || record.expiresAt <= Date.now() ||
        record.binding !== buildCursorBinding(context, filters)) throw invalidCursor();
    return record;
  }

  issue(position: CursorPosition, context: Context | null, filters: SearchFilters): string {
    const binding = buildCursorBinding(context, filters);
    const cursor = uuidV4();
    if (this._records.size >= MAX_CURSORS) {
      const oldest = this._records.keys().next().value;
      if (oldest !== undefined) this._records.delete(oldest);
    }
    this._records.set(cursor, { ...position, binding, expiresAt: Date.now() + CURSOR_TTL_MS });
    return cursor;
  }
}
