import { describe, expect, it } from 'vitest';
import { redactSensitive } from '../src/executor.js';

const replacement = '***REDACTED***';

describe('local JSON Schema references preserve sensitive redaction', () => {
  it('redacts a referenced scalar without mutating source data or schema', () => {
    const schema = {
      type: 'object',
      $defs: { code: { type: 'string', 'x-sensitive': true } },
      properties: { value: { $ref: '#/$defs/code' }, public: { type: 'string' } },
    };
    const input = { value: 'private-value', public: 'public-value' };
    const snapshot = structuredClone(schema);
    expect(redactSensitive(input, schema)).toEqual({ value: replacement, public: 'public-value' });
    expect(input.value).toBe('private-value');
    expect(schema).toEqual(snapshot);
  });

  it('redacts referenced sensitive objects and arrays as whole values', () => {
    const schema = {
      $defs: { secret: { 'x-sensitive': true } },
      properties: {
        object: { $ref: '#/$defs/secret' },
        array: { $ref: '#/$defs/secret' },
        scalar: { $ref: '#/$defs/secret' },
      },
    };
    expect(
      redactSensitive({ object: { value: 'private' }, array: ['private'], scalar: 42 }, schema),
    ).toEqual({ object: replacement, array: replacement, scalar: replacement });
  });

  it('resolves escaped and percent-encoded JSON pointer tokens', () => {
    const schema = {
      $defs: { 'a/b~c': { 'x-sensitive': true } },
      properties: { value: { $ref: '#/%24defs/a~1b~0c' } },
    };
    expect(redactSensitive({ value: 'private' }, schema).value).toBe(replacement);
  });

  it('resolves local anchors and root-id references without retrieving schemas', () => {
    const schema = {
      $id: 'https://example.test/root.json',
      $defs: {
        token: { $anchor: 'token', 'x-sensitive': true },
      },
      properties: {
        anchored: { $ref: '#token' },
        identified: { $ref: 'https://example.test/root.json#/$defs/token' },
      },
    };
    expect(redactSensitive({ anchored: 'private', identified: 'private' }, schema)).toEqual({
      anchored: replacement,
      identified: replacement,
    });
  });

  it('resolves schema pointers through array-valued combinator positions', () => {
    const schema = {
      allOf: [{ properties: { value: { $ref: '#/anyOf/0' } } }],
      anyOf: [{ 'x-sensitive': true }],
    };
    expect(redactSensitive({ value: 'private' }, schema).value).toBe(replacement);
  });

  it('resolves self references when the root identifier has an empty fragment', () => {
    const schema = {
      $id: 'https://example.test/root.json#',
      $defs: { token: { 'x-sensitive': true } },
      properties: { value: { $ref: 'https://example.test/root.json#/$defs/token' } },
    };
    expect(redactSensitive({ value: 'private' }, schema).value).toBe(replacement);
  });

  it('breaks cyclic in-memory schema graphs without skipping sensitive branches', () => {
    const recursive: Record<string, unknown> = { properties: { value: { 'x-sensitive': true } } };
    recursive.anyOf = [recursive];
    expect(redactSensitive({ value: 'private' }, recursive).value).toBe(replacement);
  });

  it('resolves in-document embedded resources with relative identifiers and scoped anchors', () => {
    const schema = {
      $id: 'https://example.test/root',
      $defs: {
        child: {
          $id: 'child',
          $anchor: 'secret',
          type: 'object',
          properties: { value: { type: 'string', 'x-sensitive': true } },
        },
      },
      properties: { payload: { $ref: 'child#secret' } },
    };
    expect(redactSensitive({ payload: { value: 'embedded-private' } }, schema)).toEqual({
      payload: { value: replacement },
    });
  });

  it('resolves embedded-resource pointers from each schema node base URI', () => {
    const schema = {
      $id: 'https://example.test/root',
      $defs: {
        child: {
          $id: 'child',
          $defs: { token: { 'x-sensitive': true } },
          properties: { value: { $ref: '#/$defs/token' } },
        },
      },
      properties: { payload: { $ref: 'child' } },
    };
    expect(redactSensitive({ payload: { value: 'embedded-private' } }, schema)).toEqual({
      payload: { value: replacement },
    });
  });

  it('ignores unknown-vocabulary anchor data instead of shadowing real schema anchors', () => {
    const schema = {
      definitions: { opaque: { $anchor: 'private', 'x-sensitive': false } },
      properties: {
        schemaField: { $anchor: 'private', 'x-sensitive': true },
        value: { $ref: '#private' },
      },
    };
    expect(redactSensitive({ value: 'private-value' }, schema).value).toBe(replacement);
  });

  it('preserves declared draft-07 definitions and embedded identifier scopes', () => {
    const schema = {
      $schema: 'http://json-schema.org/draft-07/schema#',
      $id: 'https://example.test/root',
      definitions: {
        child: { $id: 'child', properties: { value: { 'x-sensitive': true } } },
      },
      properties: { payload: { $ref: 'child' } },
    };
    expect(redactSensitive({ payload: { value: 'private-value' } }, schema)).toEqual({
      payload: { value: replacement },
    });
  });

  it('redacts referenced prefix items without applying rest-item schemas to them', () => {
    const schema = {
      $defs: { private: { 'x-sensitive': true } },
      properties: {
        values: {
          type: 'array',
          prefixItems: [{ type: 'string' }, { $ref: '#/$defs/private' }],
          items: { $ref: '#/$defs/private' },
        },
      },
    };
    expect(redactSensitive({ values: ['public', 'private', 'private-rest'] }, schema)).toEqual({
      values: ['public', replacement, replacement],
    });
  });

  it('redacts declared draft-07 tuple and additional-item references', () => {
    const schema = {
      $schema: 'http://json-schema.org/draft-07/schema#',
      definitions: { private: { 'x-sensitive': true } },
      properties: {
        values: {
          type: 'array',
          items: [{ type: 'string' }, { $ref: '#/definitions/private' }],
          additionalItems: { $ref: '#/definitions/private' },
        },
      },
    };
    expect(redactSensitive({ values: ['public', 'private', 'private-rest'] }, schema)).toEqual({
      values: ['public', replacement, replacement],
    });
  });

  it('retains sibling sensitive markers next to references', () => {
    const schema = {
      $defs: { plain: { type: 'string' } },
      properties: {
        value: { $ref: '#/$defs/plain', 'x-sensitive': true },
      },
    };
    expect(redactSensitive({ value: 'private' }, schema).value).toBe(replacement);
  });

  it.each(['anyOf', 'oneOf', 'allOf'])('expands sensitive references in %s branches', (keyword) => {
    const schema = {
      $defs: { token: { 'x-sensitive': true } },
      properties: {
        value: { [keyword]: [{ type: 'string' }, { $ref: '#/$defs/token' }] },
      },
    };
    expect(redactSensitive({ value: 'private' }, schema).value).toBe(replacement);
  });

  it('follows reference chains through object properties and array elements', () => {
    const schema = {
      $defs: {
        token: { 'x-sensitive': true },
        alias: { $ref: '#/$defs/token' },
        record: { type: 'object', properties: { value: { $ref: '#/$defs/alias' } } },
      },
      properties: { records: { type: 'array', items: { $ref: '#/$defs/record' } } },
    };
    expect(redactSensitive({ records: [{ value: 'private', visible: 3 }] }, schema)).toEqual({
      records: [{ value: replacement, visible: 3 }],
    });
  });

  it('traverses recursive data while breaking reference-only cycles', () => {
    const schema = {
      $defs: {
        node: {
          type: 'object',
          properties: { value: { 'x-sensitive': true }, next: { $ref: '#/$defs/node' } },
        },
        loopA: { $ref: '#/$defs/loopB' },
        loopB: { $ref: '#/$defs/loopA' },
      },
      properties: { node: { $ref: '#/$defs/node' }, loop: { $ref: '#/$defs/loopA' } },
    };
    expect(
      redactSensitive(
        { node: { value: 'one', next: { value: 'two', next: null } }, loop: 'visible' },
        schema,
      ),
    ).toEqual({
      node: { value: replacement, next: { value: replacement, next: null } },
      loop: 'visible',
    });
  });

  it('does not interpret anchors in opaque default/example data', () => {
    const schema = {
      $defs: { secret: { $anchor: 'value', 'x-sensitive': true } },
      default: { $anchor: 'value' },
      properties: { value: { $ref: '#value' } },
    };
    expect(redactSensitive({ value: 'private' }, schema).value).toBe(replacement);
  });

  it('never treats external, file, malformed or inherited pointer references as local schemas', () => {
    for (const reference of [
      'https://example.test/external.json',
      'file:///secret.json',
      '#/%broken',
      '#/__proto__',
    ]) {
      const schema = { properties: { value: { $ref: reference } } };
      expect(redactSensitive({ value: 'visible' }, schema)).toEqual({ value: 'visible' });
    }
  });
});
