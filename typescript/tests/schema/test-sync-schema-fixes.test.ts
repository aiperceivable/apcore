/**
 * Cross-language sync regressions for the schema layer.
 *
 * W1 — `minContains: 0` must make `contains` vacuously satisfiable
 *      (JSON Schema 2020-12 §6.4.5). TypeBox short-circuits on a zero match
 *      count before consulting `minContains`, so the trio has to be routed
 *      through the applicator evaluator instead.
 * W6 — `$ref` parsing must use maxsplit-1 semantics: a malformed ref carrying
 *      two `#` must keep the tail and fail, not be silently truncated.
 * W7 — the schemas-directory containment check must run on a realpath, so a
 *      symlink pointing outside the directory cannot be read.
 */

import { mkdirSync, rmSync, symlinkSync, writeFileSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { jsonSchemaToTypeBox } from '../../src/schema/loader.js';
import { SchemaValidator } from '../../src/schema/validator.js';
import { RefResolver } from '../../src/schema/ref-resolver.js';
import { SchemaNotFoundError } from '../../src/errors.js';

function isValid(schema: Record<string, unknown>, value: unknown): boolean {
  const validator = new SchemaValidator(false);
  return validator.validate(value as Record<string, unknown>, jsonSchemaToTypeBox(schema)).valid;
}

// ---------------------------------------------------------------------------
// W1 — minContains: 0
// ---------------------------------------------------------------------------

describe('W1: minContains: 0 makes `contains` vacuously satisfiable', () => {
  const schema = { type: 'array', contains: { type: 'number' }, minContains: 0 };

  it('accepts an array with zero matching elements', () => {
    expect(isValid(schema, ['a', 'b'])).toBe(true);
  });

  it('accepts an empty array', () => {
    expect(isValid(schema, [])).toBe(true);
  });

  it('still accepts an array with matching elements', () => {
    expect(isValid(schema, [1, 'a'])).toBe(true);
  });

  it('still enforces maxContains alongside minContains: 0', () => {
    const capped = {
      type: 'array',
      contains: { type: 'number' },
      minContains: 0,
      maxContains: 2,
    };
    expect(isValid(capped, ['a'])).toBe(true);
    expect(isValid(capped, [1, 2])).toBe(true);
    expect(isValid(capped, [1, 2, 3])).toBe(false);
  });

  it('leaves the default minContains: 1 behaviour unchanged', () => {
    const required = { type: 'array', contains: { type: 'number' } };
    expect(isValid(required, ['a'])).toBe(false);
    expect(isValid(required, ['a', 1])).toBe(true);
  });

  it('leaves an explicit minContains > 0 unchanged', () => {
    const twoPlus = { type: 'array', contains: { type: 'number' }, minContains: 2 };
    expect(isValid(twoPlus, [1])).toBe(false);
    expect(isValid(twoPlus, [1, 2])).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// W6 / W7 — RefResolver
// ---------------------------------------------------------------------------

describe('RefResolver path handling', () => {
  let root: string;
  let schemasDir: string;
  let outsideDir: string;

  beforeEach(() => {
    root = realpathSync(
      (() => {
        const d = join(tmpdir(), `apcore-refres-${Date.now()}-${Math.random().toString(36).slice(2)}`);
        mkdirSync(d, { recursive: true });
        return d;
      })(),
    );
    schemasDir = join(root, 'schemas');
    outsideDir = join(root, 'outside');
    mkdirSync(schemasDir, { recursive: true });
    mkdirSync(outsideDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(root, { recursive: true, force: true });
  });

  // W6 — split('#', 2) truncation
  it('rejects a malformed $ref carrying two "#" instead of truncating it', () => {
    writeFileSync(
      join(schemasDir, 'target.schema.yaml'),
      'definitions:\n  Ok:\n    type: string\n',
      'utf-8',
    );
    const resolver = new RefResolver(schemasDir);
    // JS `split('#', 2)` drops "/bogus" and resolves the ref successfully.
    // Python's `split("#", 1)` keeps it, and the pointer "/definitions/Ok#/bogus"
    // fails to resolve.
    expect(() =>
      resolver.resolve({ $ref: 'target.schema.yaml#/definitions/Ok#/bogus' }),
    ).toThrow(SchemaNotFoundError);
  });

  it('still resolves a well-formed cross-file $ref', () => {
    writeFileSync(
      join(schemasDir, 'target.schema.yaml'),
      'definitions:\n  Ok:\n    type: string\n',
      'utf-8',
    );
    const resolver = new RefResolver(schemasDir);
    const resolved = resolver.resolve({ $ref: 'target.schema.yaml#/definitions/Ok' });
    expect(resolved).toEqual({ type: 'string' });
  });

  // W7 — symlink escape
  it('rejects a $ref that reaches outside the schemas dir through a symlink', () => {
    writeFileSync(join(outsideDir, 'secret.schema.yaml'), 'type: string\n', 'utf-8');
    symlinkSync(join(outsideDir, 'secret.schema.yaml'), join(schemasDir, 'link.schema.yaml'));

    const resolver = new RefResolver(schemasDir);
    expect(() => resolver.resolve({ $ref: 'link.schema.yaml' })).toThrow(SchemaNotFoundError);
    expect(() => resolver.resolve({ $ref: 'link.schema.yaml' })).toThrow(/outside schemas directory/);
  });

  it('allows a symlink that stays inside the schemas dir', () => {
    const inner = join(schemasDir, 'inner');
    mkdirSync(inner, { recursive: true });
    writeFileSync(join(inner, 'real.schema.yaml'), 'type: integer\n', 'utf-8');
    symlinkSync(join(inner, 'real.schema.yaml'), join(schemasDir, 'alias.schema.yaml'));

    const resolver = new RefResolver(schemasDir);
    expect(resolver.resolve({ $ref: 'alias.schema.yaml' })).toEqual({ type: 'integer' });
  });

  it('still rejects a plain ../ traversal', () => {
    const resolver = new RefResolver(schemasDir);
    expect(() => resolver.resolve({ $ref: '../outside/secret.schema.yaml' })).toThrow(
      SchemaNotFoundError,
    );
  });
});
