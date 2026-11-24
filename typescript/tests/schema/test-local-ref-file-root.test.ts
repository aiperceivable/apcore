/**
 * D-104 (spec v1.50.0) — the base document for a local `#/…` reference.
 *
 * protocol-spec.md Algorithm A05 step 4a: a `#`-prefixed reference resolves
 * its pointer against the FILE ROOT first, and falls back to the schema node
 * being resolved. BOTH layouts are normative and an implementation MUST
 * support both.
 *
 * `SchemaLoader.resolve` called `this._resolver.resolve(schemaDef.inputSchema)`
 * with no `currentFile`, so `#/` could only ever address inside the
 * `input_schema` / `output_schema` node. §4.11's own example — `definitions:`
 * as a top-level sibling of `input_schema` in the FILE — therefore failed to
 * load here and loaded on apcore-rust alone.
 */

import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { Config } from '../../src/config.js';
import { SchemaLoader } from '../../src/schema/loader.js';

describe('a local #/ reference resolves against the file root first (D-104)', () => {
  let tmpDir: string;
  let schemasDir: string;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `apcore-d104-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    schemasDir = join(tmpDir, 'schemas');
    mkdirSync(schemasDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  function writeSchema(relPath: string, content: string): void {
    writeFileSync(join(schemasDir, relPath), content, 'utf-8');
  }

  function makeLoader(): SchemaLoader {
    return new SchemaLoader(new Config({ schema: { root: schemasDir, strategy: 'yaml_first' } }), schemasDir);
  }

  // --- Layout A: `definitions:` at the FILE root (the spec's §4.11 example) ---
  it('Layout A: definitions beside input_schema at the file top level', () => {
    writeSchema(
      'layout_a.schema.yaml',
      `
description: Layout A
definitions:
  User:
    type: object
    properties:
      email:
        type: string
    required:
      - email
input_schema:
  type: object
  properties:
    user:
      $ref: '#/definitions/User'
output_schema:
  type: object
  properties:
    ok:
      type: boolean
`,
    );
    const loader = makeLoader();
    const [input] = loader.resolve(loader.load('layout_a'));
    const props = input.jsonSchema['properties'] as Record<string, Record<string, unknown>>;
    expect(props['user']['type']).toBe('object');
    expect((props['user']['properties'] as Record<string, unknown>)['email']).toEqual({
      type: 'string',
    });
  });

  it('Layout A with $defs at the file top level', () => {
    writeSchema(
      'layout_a_defs.schema.yaml',
      `
description: Layout A with $defs
$defs:
  Tag:
    type: string
    minLength: 1
input_schema:
  type: object
  properties:
    tag:
      $ref: '#/$defs/Tag'
output_schema:
  type: object
  properties:
    ok:
      type: boolean
`,
    );
    const loader = makeLoader();
    const [input] = loader.resolve(loader.load('layout_a_defs'));
    const props = input.jsonSchema['properties'] as Record<string, Record<string, unknown>>;
    expect(props['tag']).toEqual({ type: 'string', minLength: 1 });
  });

  // --- Layout B: `$defs:` nested INSIDE input_schema (the fallback) ---
  it('Layout B: $defs nested inside input_schema still resolves', () => {
    writeSchema(
      'layout_b.schema.yaml',
      `
description: Layout B
input_schema:
  type: object
  $defs:
    User:
      type: object
      properties:
        email:
          type: string
  properties:
    user:
      $ref: '#/$defs/User'
output_schema:
  type: object
  properties:
    ok:
      type: boolean
`,
    );
    const loader = makeLoader();
    const [input] = loader.resolve(loader.load('layout_b'));
    const props = input.jsonSchema['properties'] as Record<string, Record<string, unknown>>;
    expect((props['user']['properties'] as Record<string, unknown>)['email']).toEqual({
      type: 'string',
    });
  });

  it('output_schema resolves against the file root too', () => {
    writeSchema(
      'layout_a_out.schema.yaml',
      `
description: Layout A on the output side
definitions:
  Receipt:
    type: object
    properties:
      id:
        type: string
input_schema:
  type: object
  properties: {}
output_schema:
  type: object
  properties:
    receipt:
      $ref: '#/definitions/Receipt'
`,
    );
    const loader = makeLoader();
    const [, output] = loader.resolve(loader.load('layout_a_out'));
    const props = output.jsonSchema['properties'] as Record<string, Record<string, unknown>>;
    expect((props['receipt']['properties'] as Record<string, unknown>)['id']).toEqual({
      type: 'string',
    });
  });

  it('SchemaDefinition.definitions is populated from the file top level', () => {
    writeSchema(
      'defs_collected.schema.yaml',
      `
description: collected definitions
definitions:
  User:
    type: object
input_schema:
  type: object
  properties: {}
output_schema:
  type: object
  properties: {}
`,
    );
    const sd = makeLoader().load('defs_collected');
    expect(Object.keys(sd.definitions)).toContain('User');
  });

  /* -----------------------------------------------------------------------
   * The fallback is scoped to the document it was declared for.
   *
   * D-104 settled WHICH two bases a local pointer tries; it did not say how
   * far the second one travels, and the answer was "everywhere". The fallback
   * lived on the resolver and was consulted for every local pointer,
   * including ones resolved after following a reference into another file. So
   * an external schema's `#/$defs/X`, for a definition that document does not
   * have, fell back to the CALLING module's schema node and bound to whatever
   * happened to share the name.
   *
   * Three things go wrong, in increasing order of cost: an invalid reference
   * reports success where it owes `SCHEMA_NOT_FOUND`; the resolved schema then
   * validates against a contract the external author never wrote; and §10.6
   * reads `x-sensitive` off the RESOLVED schema, so a field the external
   * document marks sensitive can be replaced by a local definition that does
   * not and be logged in plaintext — the same class of leak as dropping `$ref`
   * sibling keys (SCH-001).
   *
   * All three SDKs had it. apcore-rust was found first, in review; the peers
   * were confirmed by direct reproduction, not by reading the code.
   * --------------------------------------------------------------------- */

  const CALLER_REACHING_OUT = `
description: Caller with a local $defs the external document must not reach
input_schema:
  type: object
  $defs:
    Shared:
      type: object
      properties:
        local_marker:
          type: string
  properties:
    thing:
      $ref: './ext.schema.yaml#/$defs/Thing'
output_schema:
  type: object
  properties:
    ok:
      type: boolean
`;

  it('an external document\'s dangling local ref does not fall back to the caller', () => {
    writeSchema('caller.schema.yaml', CALLER_REACHING_OUT);
    // `#/$defs/Shared` does NOT exist here; this document has never heard of
    // the caller's $defs.
    writeSchema(
      'ext.schema.yaml',
      `
$defs:
  Thing:
    type: object
    properties:
      inner:
        $ref: '#/$defs/Shared'
`,
    );
    const loader = makeLoader();
    expect(() => loader.resolve(loader.load('caller'))).toThrow(
      /Schema not found|SCHEMA_NOT_FOUND/,
    );
  });

  it("an external document's own local ref still resolves in that document", () => {
    // The control: the fix must scope the fallback, not disable it. The two
    // markers are what tell the two same-named definitions apart.
    writeSchema('caller.schema.yaml', CALLER_REACHING_OUT);
    writeSchema(
      'ext.schema.yaml',
      `
$defs:
  Shared:
    type: object
    properties:
      external_marker:
        type: string
  Thing:
    type: object
    properties:
      inner:
        $ref: '#/$defs/Shared'
`,
    );
    const loader = makeLoader();
    const [input] = loader.resolve(loader.load('caller'));
    const props = input.jsonSchema['properties'] as Record<string, Record<string, unknown>>;
    const inner = (props['thing']['properties'] as Record<string, Record<string, unknown>>)['inner'];
    expect(inner['properties']).toEqual({ external_marker: { type: 'string' } });
  });
});
