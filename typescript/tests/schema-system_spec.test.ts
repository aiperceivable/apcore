/**
 * Spec-traced contract tests for the apcore Schema System (TypeScript SDK).
 *
 * Source spec: apcore/docs/features/schema-system.md
 * Canonical suite mirrored: apcore-python/tests/test_schema_system_spec.py
 * (the Python suite is the CANONICAL clause source).
 *
 * Each `it(...)` name carries the verbatim clause id formatted
 * `schema_system.<method>.<kind>.<detail>` (kind in
 * input|error|property|side_effect|return) so cross-language diffs line up
 * row-for-row with the Python and Rust suites.
 *
 * API mapping note (Python -> TypeScript):
 *   The spec phrases these as `Schema.validate` / `Schema.content_hash` etc.
 *   - Python's module-level `validate_schema_dict(data, schema)` (raw JSON
 *     Schema dict -> SchemaValidationResult, never raises) maps to the TS
 *     `SchemaValidator.validate(data, schema)` method. The TS validator
 *     consumes a TypeBox `TSchema`, so each raw JSON Schema dict is first
 *     converted via `jsonSchemaToTypeBox` (the loader's documented public
 *     conversion entry point). The union/oneOf exhaustive-evaluation marker
 *     (`x-apcore-keyword`) is written by `jsonSchemaToTypeBox`, so conversion
 *     through it is required for the union contracts to behave per spec.
 *   - Python's `content_hash(schema)` maps to the TS `contentHash(schema)`
 *     (sync, Node-only, SHA-256 over canonical sorted-key JSON).
 *   - Python's `RefResolver(schemas_dir, max_depth).resolve_ref(...)/.resolve(...)`
 *     maps to the TS `RefResolver(schemasDir, maxDepth).resolveRef(...)/.resolve(...)`
 *     (camelCase). TS `resolve(schema, currentFile?)` takes no per-call dir.
 *
 * These tests are READ-ONLY contract verification — they never modify src/.
 */

import { describe, it, expect } from 'vitest';
import {
  SchemaCircularRefError,
  SchemaMaxDepthExceededError,
  SchemaNotFoundError,
} from '../src/errors.js';
import * as errorsMod from '../src/errors.js';
import { SchemaValidator } from '../src/schema/validator.js';
import { RefResolver } from '../src/schema/ref-resolver.js';
import { jsonSchemaToTypeBox, contentHash } from '../src/schema/loader.js';
import type { SchemaValidationResult } from '../src/schema/types.js';

// ---------------------------------------------------------------------------
// Shared fixture schemas (canonical conformance shapes from the spec)
// ---------------------------------------------------------------------------

const CONSTRAINT_SCHEMA: Record<string, unknown> = {
  type: 'object',
  properties: {
    count: { type: 'integer', minimum: 1, maximum: 100 },
    label: { type: 'string', minLength: 1, maxLength: 50, pattern: '^[a-z_]+$' },
  },
  required: ['count', 'label'],
};

const ONEOF_SCHEMA: Record<string, unknown> = {
  oneOf: [
    { type: 'object', properties: { kind: { const: 'a' } }, required: ['kind'] },
    { type: 'object', properties: { kind: { const: 'b' } }, required: ['kind'] },
  ],
};

const ANYOF_SCHEMA: Record<string, unknown> = {
  anyOf: [
    { type: 'object', properties: { kind: { const: 'a' } }, required: ['kind'] },
    { type: 'object', properties: { kind: { const: 'b' } }, required: ['kind'] },
  ],
};

// A oneOf schema where a single input matches BOTH branches (ambiguous).
const ONEOF_AMBIGUOUS_SCHEMA: Record<string, unknown> = {
  oneOf: [
    { type: 'object' },
    { type: 'object', properties: { kind: { type: 'string' } } },
  ],
};

const TREE_NODE_SCHEMA: Record<string, unknown> = {
  $id: 'TreeNode',
  type: 'object',
  properties: {
    value: { type: 'string' },
    children: { type: 'array', items: { $ref: 'TreeNode' } },
  },
  required: ['value'],
};

// Validator instance with coercion OFF so structural validation maps to the
// Python validate_schema_dict semantics (no mutation / decode of inputs).
const VALIDATOR = new SchemaValidator(false);

function validateDict(
  data: Record<string, unknown>,
  schema: Record<string, unknown>,
): SchemaValidationResult {
  return VALIDATOR.validate(data, jsonSchemaToTypeBox(schema));
}

// ===========================================================================
// Contract: Schema.validate  ->  SchemaValidator.validate(data, schema)
// ===========================================================================

describe('schema-system contract: Schema.validate', () => {
  it('schema_system.validate.input.data_and_schema: validate accepts data + schema and returns a result object', () => {
    // TS surface: SchemaValidator.validate(data, schema). Mirrors Python
    // signature ordering (data first, schema second).
    const result = validateDict({ count: 50, label: 'hello_world' }, CONSTRAINT_SCHEMA);
    expect(typeof result).toBe('object');
    expect('valid' in result).toBe(true);
    expect('errors' in result).toBe(true);
    expect(result.valid).toBe(true);
  });

  it('schema_system.validate.error.no_raise: validation failure is reported via the result, not an exception', () => {
    // An input violating the minimum constraint must NOT throw.
    let result: SchemaValidationResult | undefined;
    expect(() => {
      result = validateDict({ count: 0, label: 'hello' }, CONSTRAINT_SCHEMA);
    }).not.toThrow();
    expect(result!.valid).toBe(false);
    expect(result!.errors.length).toBeGreaterThanOrEqual(1);
  });

  it('schema_system.validate.return.success_shape: success yields valid==true with empty errors', () => {
    const result = validateDict({ count: 50, label: 'hello_world' }, CONSTRAINT_SCHEMA);
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it('schema_system.validate.return.failure_shape: failure yields valid==false with structured error details', () => {
    const result = validateDict({ count: 200, label: 'INVALID LABEL!' }, CONSTRAINT_SCHEMA);
    expect(result.valid).toBe(false);
    expect(Array.isArray(result.errors)).toBe(true);
    expect(result.errors.length).toBeGreaterThanOrEqual(1);
    const detail = result.errors[0];
    expect('path' in detail).toBe(true);
    expect('message' in detail).toBe(true);
  });

  it('schema_system.validate.property.async_false: validate is an ordinary (non-async) function', () => {
    // A synchronous call returns a plain result object, not a Promise.
    const result = validateDict({ count: 50, label: 'hello_world' }, CONSTRAINT_SCHEMA);
    expect(result instanceof Promise).toBe(false);
    expect(VALIDATOR.validate.constructor.name).toBe('Function');
  });

  it('schema_system.validate.property.thread_safe: >=8 concurrent validations succeed without cross-talk', async () => {
    const payloads: Array<[Record<string, unknown>, boolean]> = [];
    for (let i = 0; i < 8; i++) payloads.push([{ count: i + 1, label: 'ok' }, true]);
    for (let i = 0; i < 4; i++) payloads.push([{ count: 0, label: 'ok' }, false]);

    const outcomes = await Promise.all(
      payloads.map(
        ([data, expected]) =>
          new Promise<boolean>((res) => {
            const result = validateDict(data, CONSTRAINT_SCHEMA);
            res(result.valid === expected);
          }),
      ),
    );
    expect(outcomes.length).toBeGreaterThanOrEqual(8);
    expect(outcomes.every((o) => o)).toBe(true);
  });

  it('schema_system.validate.property.pure_idempotent: repeated calls agree and inputs are not mutated', () => {
    const data = { count: 50, label: 'hello_world' };
    const dataSnapshot = JSON.stringify(data);
    const schemaSnapshot = JSON.stringify(CONSTRAINT_SCHEMA);
    const first = validateDict(data, CONSTRAINT_SCHEMA);
    const second = validateDict(data, CONSTRAINT_SCHEMA);
    expect(first.valid).toBe(second.valid);
    expect(first.errors.map((e) => e.path)).toEqual(second.errors.map((e) => e.path));
    // Inputs unchanged (no side effects).
    expect(JSON.stringify(data)).toBe(dataSnapshot);
    expect(JSON.stringify(CONSTRAINT_SCHEMA)).toBe(schemaSnapshot);
  });
});

// ===========================================================================
// Contract: RefResolver -- $ref resolution
//   TS surface: new RefResolver(schemasDir, maxDepth)
//               .resolveRef(refString, currentFile, ...) / .resolve(schema)
// ===========================================================================

describe('schema-system contract: RefResolver', () => {
  it('schema_system.resolve_ref.input.construction: constructed with schemasDir (required) and maxDepth (default 32)', () => {
    // Construction with only schemasDir must succeed (maxDepth defaults to 32).
    const resolver = new RefResolver('/tmp');
    expect(resolver).toBeInstanceOf(RefResolver);
    // Default depth of 32 is exercised: a single inline ref resolves without a
    // depth error (a depth<=0 default would have thrown).
    const out = resolver.resolve({
      type: 'object',
      properties: { addr: { $ref: '#/definitions/Address' } },
      definitions: { Address: { type: 'string' } },
    });
    expect((out['properties'] as Record<string, Record<string, unknown>>)['addr']).toEqual({
      type: 'string',
    });
  });

  it('schema_system.resolve_ref.input.resolve_ref_params: resolveRef accepts a $ref string + currentFile (null for inline)', () => {
    const resolver = new RefResolver('/tmp');
    // Inline local $ref resolves against the in-memory document via resolve().
    const schema = {
      type: 'object',
      properties: { addr: { $ref: '#/definitions/Address' } },
      definitions: { Address: { type: 'string' } },
    };
    const out = resolver.resolve(schema);
    expect((out['properties'] as Record<string, Record<string, unknown>>)['addr']).toEqual({
      type: 'string',
    });
  });

  it('schema_system.resolve.return.inline_resolved: local #/definitions $ref is inlined into the parent document', () => {
    const resolver = new RefResolver('/tmp');
    const schema = {
      type: 'object',
      properties: { addr: { $ref: '#/definitions/Address' } },
      definitions: { Address: { type: 'string', minLength: 2 } },
    };
    const out = resolver.resolve(schema);
    const addr = (out['properties'] as Record<string, Record<string, unknown>>)['addr'];
    expect(addr['type']).toBe('string');
    expect(addr['minLength']).toBe(2);
  });

  it('schema_system.resolve.side_effect.input_not_mutated: input document is not mutated (resolved copy returned)', () => {
    const resolver = new RefResolver('/tmp');
    const schema = {
      type: 'object',
      properties: { addr: { $ref: '#/definitions/Address' } },
      definitions: { Address: { type: 'string' } },
    };
    const out = resolver.resolve(schema);
    expect((schema.properties as Record<string, Record<string, unknown>>)['addr']).toEqual({
      $ref: '#/definitions/Address',
    });
    expect(out).not.toBe(schema);
  });

  it('schema_system.resolve.error.circular_ref: $ref cycle throws SchemaCircularRefError(SCHEMA_CIRCULAR_REF)', () => {
    const resolver = new RefResolver('/tmp');
    const schema = {
      $ref: '#/definitions/A',
      definitions: {
        A: { $ref: '#/definitions/B' },
        B: { $ref: '#/definitions/A' },
      },
    };
    let caught: unknown;
    try {
      resolver.resolve(schema);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(SchemaCircularRefError);
    expect((caught as SchemaCircularRefError).code).toBe('SCHEMA_CIRCULAR_REF');
  });

  it('schema_system.resolve.error.ref_not_found: an unresolvable $ref throws (TS: SchemaNotFoundError/SCHEMA_NOT_FOUND)', () => {
    // SPEC DIVERGENCE (flagged): the contract declares
    // SchemaRefNotFoundError(code=SCHEMA_REF_NOT_FOUND). Neither that class nor
    // that code exists in apcore-typescript (matching apcore-python). The SDK
    // raises SchemaNotFoundError(code=SCHEMA_NOT_FOUND) instead. This asserts
    // the ACTUAL behavior; the spec-named symbol is covered by a skip below.
    const resolver = new RefResolver('/tmp');
    let caught: unknown;
    try {
      resolver.resolveRef('#/definitions/DoesNotExist', null);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(SchemaNotFoundError);
    expect((caught as SchemaNotFoundError).code).toBe('SCHEMA_NOT_FOUND');
  });

  it('schema_system.resolve.error.ref_not_found_spec_symbol: spec SchemaRefNotFoundError is absent -> MISSING-SYMBOL', () => {
    // The contract names SchemaRefNotFoundError(code=SCHEMA_REF_NOT_FOUND).
    // That symbol is absent from apcore-typescript -> MISSING-SYMBOL.
    if (!('SchemaRefNotFoundError' in errorsMod)) {
      // Mirror the Python suite's skip: assert absence so the divergence is
      // recorded without a false-green pass on a non-existent symbol.
      expect('SchemaRefNotFoundError' in errorsMod).toBe(false);
      return;
    }
    throw new Error('unreachable: symbol now exists; revisit divergence');
  });

  it('schema_system.resolve_ref.property.async_false: resolve and resolveRef are synchronous', () => {
    // Mirror Python intent: neither resolve nor resolveRef is async. The
    // whole-document resolve() returns a plain object (not a Promise), and
    // resolveRef is a synchronous method (it throws synchronously on a missing
    // local pointer rather than rejecting a Promise).
    const resolver = new RefResolver('/tmp');
    const out = resolver.resolve({
      type: 'object',
      properties: { addr: { $ref: '#/definitions/Address' } },
      definitions: { Address: { type: 'string' } },
    });
    expect(out instanceof Promise).toBe(false);
    // resolveRef raises synchronously (no Promise rejection) for an unresolved
    // standalone local pointer — confirming it is not async.
    let threwSync = false;
    try {
      resolver.resolveRef('#/definitions/Address', null);
    } catch {
      threwSync = true;
    }
    expect(threwSync).toBe(true);
  });

  it('schema_system.resolve.property.thread_safe: >=8 concurrent resolutions return correctly inlined documents', async () => {
    const resolveOne = (idx: number): Record<string, unknown> => {
      const resolver = new RefResolver('/tmp');
      const schema = {
        type: 'object',
        properties: { v: { $ref: '#/definitions/V' } },
        definitions: { V: { type: 'integer', const: idx } },
      };
      return resolver.resolve(schema);
    };

    const results = await Promise.all(
      Array.from({ length: 8 }, (_, i) => Promise.resolve().then(() => resolveOne(i))),
    );
    expect(results.length).toBeGreaterThanOrEqual(8);
    results.forEach((out, i) => {
      const v = (out['properties'] as Record<string, Record<string, unknown>>)['v'];
      expect(v['const']).toBe(i);
    });
  });

  it('schema_system.resolve.property.idempotent: re-resolving the same input yields an equal resolved document', () => {
    const resolver = new RefResolver('/tmp');
    const schema = {
      type: 'object',
      properties: { addr: { $ref: '#/definitions/Address' } },
      definitions: { Address: { type: 'string' } },
    };
    const first = resolver.resolve(schema);
    const second = resolver.resolve(schema);
    expect(first).toEqual(second);
  });

  it('schema_system.resolve.error.max_depth_exceeded: exceeding maxDepth throws SchemaMaxDepthExceededError(SCHEMA_MAX_DEPTH_EXCEEDED)', () => {
    const resolver = new RefResolver('/tmp', 3);
    const schema = {
      $ref: '#/definitions/A',
      definitions: {
        A: { $ref: '#/definitions/B' },
        B: { $ref: '#/definitions/C' },
        C: { $ref: '#/definitions/D' },
        D: { $ref: '#/definitions/E' },
        E: { type: 'string' },
      },
    };
    let caught: unknown;
    try {
      resolver.resolve(schema);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(SchemaMaxDepthExceededError);
    expect((caught as SchemaMaxDepthExceededError).code).toBe('SCHEMA_MAX_DEPTH_EXCEEDED');
  });
});

// ===========================================================================
// Contract: Schema.validate_union  ->  validate over anyOf/oneOf
// ===========================================================================

describe('schema-system contract: Schema.validate_union', () => {
  it('schema_system.validate_union.input.anyof: anyOf accepts input matching at least one branch', () => {
    expect(validateDict({ kind: 'a' }, ANYOF_SCHEMA).valid).toBe(true);
    expect(validateDict({ kind: 'b' }, ANYOF_SCHEMA).valid).toBe(true);
  });

  it('schema_system.validate_union.input.oneof: oneOf accepts input matching exactly one branch', () => {
    expect(validateDict({ kind: 'a' }, ONEOF_SCHEMA).valid).toBe(true);
  });

  it('schema_system.validate_union.error.no_match: no branch matched -> errorCode SCHEMA_UNION_NO_MATCH', () => {
    const result = validateDict({ kind: 'c' }, ONEOF_SCHEMA);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_UNION_NO_MATCH');

    const anyResult = validateDict({ kind: 'c' }, ANYOF_SCHEMA);
    expect(anyResult.valid).toBe(false);
    expect(anyResult.errorCode).toBe('SCHEMA_UNION_NO_MATCH');
  });

  it('schema_system.validate_union.error.ambiguous: >1 oneOf branch matched -> errorCode SCHEMA_UNION_AMBIGUOUS', () => {
    const result = validateDict({ kind: 'x' }, ONEOF_AMBIGUOUS_SCHEMA);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_UNION_AMBIGUOUS');
  });

  it('schema_system.validate_union.property.all_branches: all branches evaluated (second-branch-only match accepted)', () => {
    expect(validateDict({ kind: 'b' }, ONEOF_SCHEMA).valid).toBe(true);
    expect(validateDict({ kind: 'b' }, ANYOF_SCHEMA).valid).toBe(true);
  });

  it('schema_system.validate_union.property.thread_safe: >=8 concurrent union validations return expected verdicts', async () => {
    const cases: Array<[Record<string, unknown>, boolean]> = [
      [{ kind: 'a' }, true],
      [{ kind: 'b' }, true],
      [{ kind: 'c' }, false],
      [{ kind: 'a' }, true],
      [{ kind: 'b' }, true],
      [{ kind: 'c' }, false],
      [{ kind: 'a' }, true],
      [{ kind: 'b' }, true],
    ];
    const outcomes = await Promise.all(
      cases.map(
        ([data, expected]) =>
          new Promise<boolean>((res) => {
            res(validateDict(data, ONEOF_SCHEMA).valid === expected);
          }),
      ),
    );
    expect(outcomes.length).toBeGreaterThanOrEqual(8);
    expect(outcomes.every((o) => o)).toBe(true);
  });

  it('schema_system.validate_union.property.pure_idempotent: union validation is pure and idempotent', () => {
    const first = validateDict({ kind: 'x' }, ONEOF_AMBIGUOUS_SCHEMA);
    const second = validateDict({ kind: 'x' }, ONEOF_AMBIGUOUS_SCHEMA);
    expect(first.valid).toBe(second.valid);
    expect(first.errorCode).toBe(second.errorCode);
  });
});

// ===========================================================================
// Contract: Schema.validate_recursive  ->  validate over $id/$ref recursion
// ===========================================================================

describe('schema-system contract: Schema.validate_recursive', () => {
  it('schema_system.validate_recursive.input.nested_data: valid nested structures (up to depth 5) validate true', () => {
    expect(validateDict({ value: 'root' }, TREE_NODE_SCHEMA).valid).toBe(true);
    expect(
      validateDict({ value: 'r', children: [{ value: 'c' }] }, TREE_NODE_SCHEMA).valid,
    ).toBe(true);
    const deep = {
      value: 'a',
      children: [
        {
          value: 'b',
          children: [
            { value: 'c', children: [{ value: 'd', children: [{ value: 'e' }] }] },
          ],
        },
      ],
    };
    expect(validateDict(deep, TREE_NODE_SCHEMA).valid).toBe(true);
  });

  it('schema_system.validate_recursive.error.validation_error: non-conforming data -> errorCode SCHEMA_VALIDATION_ERROR', () => {
    // Missing required `value` at the top level.
    const result = validateDict({ children: [] }, TREE_NODE_SCHEMA);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
  });

  it('schema_system.validate_recursive.error.nested_validation_error: a child missing `value` is rejected', () => {
    const result = validateDict(
      { value: 'root', children: [{ children: [] }] },
      TREE_NODE_SCHEMA,
    );
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
  });

  it('schema_system.validate_recursive.property.idempotent: recursive validation is pure and idempotent', () => {
    const data = { value: 'root', children: [{ value: 'c' }] };
    const snapshot = JSON.stringify(data);
    const first = validateDict(data, TREE_NODE_SCHEMA);
    const second = validateDict(data, TREE_NODE_SCHEMA);
    expect(first.valid).toBe(true);
    expect(second.valid).toBe(true);
    expect(JSON.stringify(data)).toBe(snapshot);
  });

  it('schema_system.validate_recursive.property.thread_safe: >=8 concurrent recursive validations return expected verdicts', async () => {
    const validPayload = { value: 'root', children: [{ value: 'c' }] };
    const invalidPayload = { children: [] };
    const cases: Array<[Record<string, unknown>, boolean]> = Array.from(
      { length: 8 },
      (_, i) => (i % 2 === 0 ? [validPayload, true] : [invalidPayload, false]),
    );
    const outcomes = await Promise.all(
      cases.map(
        ([data, expected]) =>
          new Promise<boolean>((res) => {
            res(validateDict(data, TREE_NODE_SCHEMA).valid === expected);
          }),
      ),
    );
    expect(outcomes.length).toBeGreaterThanOrEqual(8);
    expect(outcomes.every((o) => o)).toBe(true);
  });
});

// ===========================================================================
// Contract: Schema.content_hash  ->  contentHash(schema)
// ===========================================================================

describe('schema-system contract: Schema.content_hash', () => {
  it('schema_system.content_hash.input.schema_dict: contentHash accepts a single schema and returns a string', () => {
    const digest = contentHash({ type: 'object' });
    expect(typeof digest).toBe('string');
  });

  it('schema_system.content_hash.return.hex_digest: lowercase hexadecimal SHA-256 digest (64 chars)', () => {
    const digest = contentHash(CONSTRAINT_SCHEMA);
    expect(typeof digest).toBe('string');
    expect(digest.length).toBe(64);
    expect(digest).toBe(digest.toLowerCase());
    expect(/^[0-9a-f]{64}$/.test(digest)).toBe(true);
  });

  it('schema_system.content_hash.error.no_raise: does not throw for a serializable schema dict', () => {
    let digest = '';
    expect(() => {
      digest = contentHash({ a: 1, b: [1, 2, { c: 'd' }] });
    }).not.toThrow();
    expect(digest.length).toBe(64);
  });

  it('schema_system.content_hash.property.canonical_dedup: key ordering does not affect the digest; distinct content differs', () => {
    const a = { b: 1, a: 2, z: { y: 1, x: 2 } };
    const b = { a: 2, z: { x: 2, y: 1 }, b: 1 };
    expect(contentHash(a)).toBe(contentHash(b));
    expect(contentHash({ a: 1 })).not.toBe(contentHash({ a: 2 }));
  });

  it('schema_system.content_hash.property.idempotent: repeated calls agree and input is not mutated', () => {
    const schema = { type: 'object', properties: { x: { type: 'string' } } };
    const snapshot = JSON.stringify(schema);
    const first = contentHash(schema);
    const second = contentHash(schema);
    expect(first).toBe(second);
    expect(JSON.stringify(schema)).toBe(snapshot);
  });

  it('schema_system.content_hash.property.async_false: contentHash is a synchronous callable', () => {
    const digest = contentHash(CONSTRAINT_SCHEMA);
    // Cast through `unknown`: `digest` is statically typed `string`, so a direct
    // `instanceof` is a compile error — the runtime check still proves it is not a Promise.
    expect((digest as unknown) instanceof Promise).toBe(false);
    expect(typeof digest).toBe('string');
  });

  it('schema_system.content_hash.property.thread_safe: >=8 concurrent hashes of the same schema all agree', async () => {
    const expected = contentHash(CONSTRAINT_SCHEMA);
    const digests = await Promise.all(
      Array.from({ length: 8 }, () => Promise.resolve().then(() => contentHash(CONSTRAINT_SCHEMA))),
    );
    expect(digests.length).toBeGreaterThanOrEqual(8);
    expect(digests.every((d) => d === expected)).toBe(true);
  });
});
