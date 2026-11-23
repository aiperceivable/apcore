/**
 * Schema extractor — multi-adapter chain for auto_schema inference.
 *
 * Implements protocol-spec §5.12.5 (TypeScript adapter chain).
 *
 * Built-in adapters:
 *   1. TypeBox (priority 100) — detects TypeBox schema objects by symbol
 *   2. JsonSchema (priority 30) — detects plain JSON Schema dicts
 *
 * Custom adapters (zod, class-validator, typia) can be registered at runtime
 * via `SchemaExtractorRegistry.register()`.
 */

import type { TSchema } from '@sinclair/typebox';
import { jsonSchemaToTypeBox } from './loader.js';

// ---------------------------------------------------------------------------
// Adapter interface
// ---------------------------------------------------------------------------

/**
 * A schema adapter can detect and extract a TSchema from an unknown runtime
 * value. Adapters are tried in priority order (highest priority first).
 */
export interface SchemaAdapter {
  /** Unique adapter name (e.g., 'typebox', 'zod'). */
  readonly name: string;
  /** Priority: higher = tried first. Built-in TypeBox = 100, JsonSchema = 30. */
  readonly priority: number;
  /** Return true if this adapter can handle the given value. */
  detect(value: unknown): boolean;
  /** Extract a TSchema from the value. Called only when detect() returned true. */
  extract(value: unknown): TSchema;
}

// ---------------------------------------------------------------------------
// Built-in adapters
// ---------------------------------------------------------------------------

/** Symbol used by TypeBox to mark schema objects. */
const TYPEBOX_KIND = Symbol.for('TypeBox.Kind');

/** Detects @sinclair/typebox schema objects by their internal symbol. */
const typeBoxAdapter: SchemaAdapter = {
  name: 'typebox',
  priority: 100,
  detect(value: unknown): boolean {
    return (
      value !== null &&
      typeof value === 'object' &&
      TYPEBOX_KIND in (value as Record<symbol, unknown>)
    );
  },
  extract(value: unknown): TSchema {
    return value as TSchema;
  },
};

/**
 * Detects plain JSON Schema objects (have a "type" or "properties" key but
 * no TypeBox symbol). Converts to TSchema via the existing jsonSchemaToTypeBox
 * converter.
 */
const jsonSchemaAdapter: SchemaAdapter = {
  name: 'json-schema',
  priority: 30,
  detect(value: unknown): boolean {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return false;
    const obj = value as Record<string, unknown>;
    // Must have at least "type" or "properties" to look like a JSON Schema.
    if (!('type' in obj) && !('properties' in obj)) return false;
    // Exclude TypeBox objects (they also have "type" but have the symbol).
    if (TYPEBOX_KIND in (value as Record<symbol, unknown>)) return false;
    return true;
  },
  extract(value: unknown): TSchema {
    return jsonSchemaToTypeBox(value as Record<string, unknown>);
  },
};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/** Ordered set of schema adapters (sorted by descending priority). */
const _adapters: SchemaAdapter[] = [typeBoxAdapter, jsonSchemaAdapter];
let _sorted = true;

function ensureSorted(): void {
  if (!_sorted) {
    _adapters.sort((a, b) => b.priority - a.priority);
    _sorted = true;
  }
}

/**
 * Register a custom schema adapter into the global chain.
 *
 * Use this to add zod, class-validator, or typia support:
 * ```ts
 * import { SchemaExtractorRegistry } from 'apcore-js';
 * SchemaExtractorRegistry.register(myZodAdapter);
 * ```
 */
export const SchemaExtractorRegistry = {
  register(adapter: SchemaAdapter): void {
    _adapters.push(adapter);
    _sorted = false;
  },

  /** Remove a registered adapter by name. Returns true if found. */
  unregister(name: string): boolean {
    const idx = _adapters.findIndex((a) => a.name === name);
    if (idx === -1) return false;
    _adapters.splice(idx, 1);
    return true;
  },

  /** List registered adapter names in priority order. */
  names(): string[] {
    ensureSorted();
    return _adapters.map((a) => a.name);
  },
};

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/**
 * Attempt to extract a TSchema from an unknown value by running the adapter
 * chain in priority order.
 *
 * @returns The extracted TSchema, or `null` if no adapter matched.
 */
export function extractSchema(value: unknown): TSchema | null {
  ensureSorted();
  for (const adapter of _adapters) {
    if (adapter.detect(value)) {
      return adapter.extract(value);
    }
  }
  return null;
}

/**
 * Scan a module's exports for `inputSchema` and `outputSchema` declarations
 * that can be auto-extracted.
 *
 * Naming conventions tried:
 *   1. `inputSchema` / `outputSchema` (direct named exports)
 *   2. `<symbolName>InputSchema` / `<symbolName>OutputSchema` (companion naming)
 *
 * Each found value is passed through the adapter chain.
 *
 * @param mod - The imported module object
 * @param symbolName - The callable's export name (from target string after `:`)
 * @returns `{ input, output }` pair, or `null` if inference failed.
 */
export function inferSchemasFromModule(
  mod: Record<string, unknown>,
  symbolName: string,
): { input: TSchema; output: TSchema } | null {
  // Convention 1: direct named exports
  const inputRaw = mod['inputSchema'] ?? mod[`${symbolName}InputSchema`];
  const outputRaw = mod['outputSchema'] ?? mod[`${symbolName}OutputSchema`];

  if (inputRaw == null && outputRaw == null) {
    return null;
  }

  const input = inputRaw != null ? extractSchema(inputRaw) : null;
  const output = outputRaw != null ? extractSchema(outputRaw) : null;

  if (input == null || output == null) {
    return null;
  }

  return { input, output };
}
