/**
 * OpenAI structured-outputs strict-mode compatibility detection.
 *
 * This module is deliberately **separate** from `strict.ts`. `toStrictSchema()`
 * is a general-purpose registry/export transform (see `registry/registry.ts`
 * and `registry/schema-export.ts`); baking an OpenAI-specific dialect check
 * into it would leak one vendor's constraints into every other consumer.
 * Detection lives here and is invoked only on the `auto_schema: strict`
 * binding path (protocol-spec §5.12.5).
 *
 * Detection **never rewrites** the schema. In particular an author-written
 * `oneOf` is reported, not silently downgraded to `anyOf`: the two differ in
 * exclusivity, and apcore's own validator relies on the distinction (it counts
 * matching branches and raises `SCHEMA_UNION_AMBIGUOUS`). Rewriting at export
 * time would trade an explicit export-time rejection for a silent runtime
 * mismatch.
 *
 * ---------------------------------------------------------------------------
 * Source of truth for the keyword set below:
 *   OpenAI — "Structured model outputs", section "Supported schemas / Supported
 *   properties" and "Some type-specific keywords are not yet supported".
 *   https://developers.openai.com/api/docs/guides/structured-outputs
 *   Verified 2026-08-03.
 *
 * What that page states, verbatim in substance:
 *   - Supported types: String, Number, Boolean, Integer, Object, Array, Enum,
 *     anyOf.
 *   - Supported `string` properties: `pattern`, `format` (limited to the nine
 *     values in SUPPORTED_FORMATS below).
 *   - Supported `number` properties: `multipleOf`, `maximum`,
 *     `exclusiveMaximum`, `minimum`, `exclusiveMinimum`.
 *   - Supported `array` properties: `minItems`, `maxItems`.
 *   - Not supported — composition: `allOf`, `not`, `dependentRequired`,
 *     `dependentSchemas`, `if`, `then`, `else`.
 *   - `anyOf` is supported but MUST NOT appear at the root (the root must be an
 *     object).
 *   - `$ref` / `$defs` (including recursive `#` self-references) are supported.
 *   - `additionalProperties: false` and "every property listed in `required`"
 *     are requirements, not incompatibilities — `toStrictSchema()` already
 *     produces both, so they are transformed rather than reported here.
 *
 * Anything that appears in neither the supported nor the explicitly-unsupported
 * list (`oneOf`, `minLength`/`maxLength`, `patternProperties`, `uniqueItems`,
 * …) is treated as unsupported: OpenAI rejects unknown/unlisted keywords when
 * `strict: true`.
 *
 * NOTE: this set tracks a moving target. When OpenAI expands structured-output
 * support, update the three SDK copies (`schema/openai-strict.ts`,
 * `schema/openai_strict.py`, `schema/openai_strict.rs`) together and re-run the
 * `openai_strict_compat.json` conformance fixture, which pins the cross-SDK
 * answer.
 * ---------------------------------------------------------------------------
 */

import { BindingStrictSchemaIncompatibleError } from '../errors.js';
import { KEYWORD_MARKER } from './constants.js';

/**
 * JSON Schema keywords OpenAI structured outputs does not accept under
 * `strict: true`. Kept sorted for reviewability; emission order is normalised
 * by sorting the result, so cross-SDK output does not depend on this order.
 */
const UNSUPPORTED_KEYWORDS: readonly string[] = [
  // Composition — explicitly listed as unsupported by OpenAI.
  'allOf',
  'dependentRequired',
  'dependentSchemas',
  'else',
  'if',
  'not',
  'then',
  // `oneOf` is absent from the supported list (only `anyOf` is named).
  'oneOf',
  // String constraints beyond `pattern` / `format`.
  'maxLength',
  'minLength',
  // Object keywords beyond `properties` / `required` / `additionalProperties`.
  'maxProperties',
  'minProperties',
  'patternProperties',
  'propertyNames',
  'unevaluatedProperties',
  // Array keywords beyond `items` / `minItems` / `maxItems`.
  'additionalItems',
  'contains',
  'maxContains',
  'minContains',
  'prefixItems',
  'unevaluatedItems',
  'uniqueItems',
];

/** The nine `format` values OpenAI accepts for strings. */
const SUPPORTED_FORMATS: ReadonlySet<string> = new Set([
  'date-time',
  'time',
  'date',
  'duration',
  'email',
  'hostname',
  'ipv4',
  'ipv6',
  'uuid',
]);

function isSchemaObject(node: unknown): node is Record<string, unknown> {
  return typeof node === 'object' && node !== null && !Array.isArray(node);
}

/**
 * Walk `schema` and return every feature OpenAI structured outputs rejects
 * under `strict: true`.
 *
 * Returned entries are JSON-path-ish descriptors, sorted and de-duplicated so
 * that Python, TypeScript and Rust produce byte-identical lists:
 *   - `$.tags.uniqueItems`   — unsupported keyword at that location
 *   - `$.created.format=uri` — unsupported `format` value
 *   - `$.anyOf`              — `anyOf` used at the root (root must be an object)
 *
 * An empty array means the schema is compatible.
 */
export function detectOpenAiStrictIncompatibilities(schema: Record<string, unknown>): string[] {
  const found: string[] = [];

  const walk = (node: unknown, path: string, isRoot: boolean): void => {
    if (!isSchemaObject(node)) return;

    for (const keyword of UNSUPPORTED_KEYWORDS) {
      if (keyword in node) found.push(`${path}.${keyword}`);
    }

    // TypeScript-specific: `jsonSchemaToTypeBox()` lowers an authored `oneOf`
    // to a TypeBox `Union` — which serialises as `anyOf` — and records the
    // original keyword in `KEYWORD_MARKER`. Report it as `oneOf`, matching what
    // the Python and Rust detectors see in the source JSON Schema.
    //
    // The marker is not cosmetic: `SchemaValidator` reads it and enforces
    // exactly-one-branch at call time, raising SCHEMA_UNION_AMBIGUOUS. Letting
    // the `anyOf` spelling pass would export "either branch is fine" to the LLM
    // while the module still rejects ambiguous input at runtime — precisely the
    // silent export/runtime mismatch this check exists to prevent.
    const markedAs = node[KEYWORD_MARKER];
    if (markedAs === 'oneOf' && !('oneOf' in node)) found.push(`${path}.oneOf`);

    // `anyOf` is supported, but not as the root schema.
    if (isRoot && 'anyOf' in node && markedAs !== 'oneOf') found.push(`${path}.anyOf`);

    const format = node['format'];
    if (typeof format === 'string' && !SUPPORTED_FORMATS.has(format)) {
      found.push(`${path}.format=${format}`);
    }

    const properties = node['properties'];
    if (isSchemaObject(properties)) {
      for (const key of Object.keys(properties).sort()) {
        walk(properties[key], `${path}.${key}`, false);
      }
    }

    const items = node['items'];
    if (isSchemaObject(items)) walk(items, `${path}[]`, false);

    const additionalProperties = node['additionalProperties'];
    if (isSchemaObject(additionalProperties)) {
      walk(additionalProperties, `${path}.additionalProperties`, false);
    }

    // Recurse into union branches only. `allOf`/`not`/`if`/`then`/`else` are
    // already reported by keyword; descending into them would add noise
    // beneath a finding the caller must fix by removing the branch anyway.
    for (const combinator of ['anyOf', 'oneOf']) {
      const branches = node[combinator];
      if (Array.isArray(branches)) {
        // A marked union stores its branches under `anyOf`; label the path with
        // the authored keyword so the reported paths match Python and Rust.
        const label = combinator === 'anyOf' && markedAs === 'oneOf' ? 'oneOf' : combinator;
        branches.forEach((branch, index) => {
          walk(branch, `${path}.${label}[${index}]`, false);
        });
      }
    }

    for (const defsKey of ['$defs', 'definitions']) {
      const defs = node[defsKey];
      if (isSchemaObject(defs)) {
        for (const key of Object.keys(defs).sort()) {
          walk(defs[key], `${path}.${defsKey}.${key}`, false);
        }
      }
    }
  };

  walk(schema, '$', true);
  return [...new Set(found)].sort();
}

/** Options accepted by {@link assertOpenAiStrictCompatible}. */
export interface AssertOpenAiStrictOptions {
  /** Binding `module_id`, used in the error message. */
  readonly moduleId: string;
  /** Prefix applied to every reported feature, e.g. `input` / `output`. */
  readonly side?: string;
  /** Binding file path, for the `{file_path}:{line}:` message prefix. */
  readonly filePath?: string;
  /** Binding file line, for the `{file_path}:{line}:` message prefix. */
  readonly line?: number;
}

/**
 * Throw {@link BindingStrictSchemaIncompatibleError} when `schema` contains any
 * feature OpenAI structured outputs rejects under `strict: true`.
 *
 * Invoked on the `auto_schema: strict` binding path only
 * (protocol-spec §5.12.5). No-op for compatible schemas.
 */
export function assertOpenAiStrictCompatible(
  schema: Record<string, unknown>,
  options: AssertOpenAiStrictOptions,
): void {
  const features = detectOpenAiStrictIncompatibilities(schema);
  if (features.length === 0) return;

  const { moduleId, side, filePath, line } = options;
  const labelled = side === undefined ? features : features.map((f) => `${side}:${f}`);
  throw new BindingStrictSchemaIncompatibleError(moduleId, labelled, filePath, line);
}
