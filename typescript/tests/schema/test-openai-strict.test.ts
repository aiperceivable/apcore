/**
 * Unit + binding-path regression tests for OpenAI strict-mode compatibility
 * detection (protocol-spec §5.12.5).
 *
 * Before this feature existed, `BindingStrictSchemaIncompatibleError` was
 * defined and exported but no code path ever threw it — `auto_schema: strict`
 * silently produced schemas OpenAI structured outputs would reject. The
 * `loadBindings` cases below fail without the detector wired into
 * `src/bindings.ts`.
 *
 * Cross-SDK feature-list parity lives in
 * `tests/conformance-openai-strict-compat.test.ts`; this file covers the
 * TypeScript-specific wiring and the error payload.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  assertOpenAiStrictCompatible,
  detectOpenAiStrictIncompatibilities,
} from '../../src/schema/openai-strict.js';
import { BindingStrictSchemaIncompatibleError } from '../../src/errors.js';
import { BindingLoader } from '../../src/bindings.js';
import { jsonSchemaToTypeBox } from '../../src/schema/loader-pure.js';
import { Registry } from '../../src/registry/registry.js';

describe('detectOpenAiStrictIncompatibilities', () => {
  it('returns an empty list for a schema OpenAI accepts', () => {
    expect(
      detectOpenAiStrictIncompatibilities({
        type: 'object',
        properties: { a: { type: 'string' }, n: { type: 'integer', minimum: 0 } },
        required: ['a', 'n'],
        additionalProperties: false,
      }),
    ).toEqual([]);
  });

  it('does not report a nested anyOf — OpenAI supports it below the root', () => {
    // This is the nullable wrapper all three SDKs emit. Reporting it would make
    // every optional field fail auto_schema: strict.
    expect(
      detectOpenAiStrictIncompatibilities({
        type: 'object',
        properties: { note: { anyOf: [{ type: 'string' }, { type: 'null' }] } },
      }),
    ).toEqual([]);
  });

  it('reports anyOf when used as the root schema', () => {
    expect(detectOpenAiStrictIncompatibilities({ anyOf: [{ type: 'object' }] })).toEqual([
      '$.anyOf',
    ]);
  });

  it('reports an author-written oneOf without rewriting it to anyOf', () => {
    const schema = {
      type: 'object',
      properties: { mode: { oneOf: [{ type: 'string' }, { type: 'integer' }] } },
    };
    const snapshot = JSON.stringify(schema);

    expect(detectOpenAiStrictIncompatibilities(schema)).toEqual(['$.mode.oneOf']);
    // Rewriting oneOf → anyOf would tell the LLM "both branches matching is
    // fine" while apcore's own validator still raises SCHEMA_UNION_AMBIGUOUS.
    expect(JSON.stringify(schema)).toBe(snapshot);
  });

  it('reports unsupported format values but not the nine supported ones', () => {
    expect(
      detectOpenAiStrictIncompatibilities({
        type: 'object',
        properties: {
          ok: { type: 'string', format: 'date-time' },
          bad: { type: 'string', format: 'uri' },
        },
      }),
    ).toEqual(['$.bad.format=uri']);
  });

  it('returns a sorted, de-duplicated list', () => {
    const got = detectOpenAiStrictIncompatibilities({
      type: 'object',
      properties: {
        zeta: { type: 'string', minLength: 1 },
        alpha: { type: 'string', minLength: 1 },
      },
    });
    expect(got).toEqual(['$.alpha.minLength', '$.zeta.minLength']);
  });

  it('reports a TypeBox union that jsonSchemaToTypeBox lowered from oneOf', () => {
    // TS-specific parity guard: the converter turns `oneOf` into a TypeBox
    // Union (serialised `anyOf`) and records the origin in KEYWORD_MARKER.
    // Without honouring the marker, TypeScript would silently accept a schema
    // Python and Rust reject — and would export "either branch is fine" while
    // SchemaValidator still raises SCHEMA_UNION_AMBIGUOUS at call time.
    const converted = jsonSchemaToTypeBox({
      type: 'object',
      properties: { mode: { oneOf: [{ type: 'string' }, { type: 'integer' }] } },
    }) as unknown as Record<string, unknown>;

    expect(detectOpenAiStrictIncompatibilities(converted)).toEqual(['$.mode.oneOf']);
  });

  it('does not report a TypeBox union lowered from a genuine anyOf', () => {
    const converted = jsonSchemaToTypeBox({
      type: 'object',
      properties: { note: { anyOf: [{ type: 'string' }, { type: 'null' }] } },
    }) as unknown as Record<string, unknown>;

    expect(detectOpenAiStrictIncompatibilities(converted)).toEqual([]);
  });

  it('tolerates a non-object schema node', () => {
    expect(detectOpenAiStrictIncompatibilities(true as unknown as Record<string, unknown>)).toEqual(
      [],
    );
  });
});

describe('assertOpenAiStrictCompatible', () => {
  it('is a no-op for a compatible schema', () => {
    expect(() =>
      assertOpenAiStrictCompatible({ type: 'object', properties: {} }, { moduleId: 'm' }),
    ).not.toThrow();
  });

  it('throws BindingStrictSchemaIncompatibleError with side-prefixed features', () => {
    let caught: unknown;
    try {
      assertOpenAiStrictCompatible(
        { type: 'object', properties: { s: { type: 'string', minLength: 2 } } },
        { moduleId: 'demo.mod', side: 'input', filePath: 'b.yaml' },
      );
    } catch (e) {
      caught = e;
    }
    const err = caught as BindingStrictSchemaIncompatibleError;
    expect(err).toBeInstanceOf(BindingStrictSchemaIncompatibleError);
    expect(err.code).toBe('BINDING_STRICT_SCHEMA_INCOMPATIBLE');
    expect(err.details['featuresListed']).toEqual(['input:$.s.minLength']);
    expect(err.message).toContain("binding 'demo.mod' uses auto_schema: strict");
    expect(err.message).toContain('input:$.s.minLength');
    expect(err.message).toContain('See protocol-spec §5.12.5');
  });
});

describe('BindingLoader auto_schema: strict enforcement', () => {
  let dir: string;
  let loader: BindingLoader;
  let registry: Registry;

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'apcore-strict-'));
    loader = new BindingLoader();
    registry = new Registry();
  });

  afterEach(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  /** Write an ESM target module exporting plain-JSON-Schema input/output. */
  function writeTarget(name: string, inputSchema: unknown, outputSchema: unknown): string {
    const file = join(dir, name);
    writeFileSync(
      file,
      `export const inputSchema = ${JSON.stringify(inputSchema)};\n` +
        `export const outputSchema = ${JSON.stringify(outputSchema)};\n` +
        'export function run(inputs) { return { ok: true, ...inputs }; }\n',
    );
    return file;
  }

  function writeBinding(moduleId: string, target: string, autoSchema: string): string {
    const file = join(dir, 'b.binding.yaml');
    writeFileSync(
      file,
      `bindings:\n  - module_id: ${moduleId}\n    target: "${target}:run"\n` +
        `    auto_schema: ${autoSchema}\n`,
    );
    return file;
  }

  const COMPATIBLE = { type: 'object', properties: { a: { type: 'string' } }, required: ['a'] };

  it('rejects an inferred input schema OpenAI cannot accept', async () => {
    const target = writeTarget(
      'bad_input.mjs',
      { type: 'object', properties: { s: { type: 'string', minLength: 3 } } },
      COMPATIBLE,
    );
    const bindingFile = writeBinding('strict.bad.input', target, 'strict');

    const err = await loader.loadBindings(bindingFile, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingStrictSchemaIncompatibleError);
    expect((err as BindingStrictSchemaIncompatibleError).details['featuresListed']).toEqual([
      'input:$.s.minLength',
    ]);
  });

  it('rejects an inferred output schema OpenAI cannot accept', async () => {
    const target = writeTarget('bad_output.mjs', COMPATIBLE, {
      type: 'object',
      properties: { mode: { oneOf: [{ type: 'string' }, { type: 'integer' }] } },
    });
    const bindingFile = writeBinding('strict.bad.output', target, 'strict');

    const err = await loader.loadBindings(bindingFile, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingStrictSchemaIncompatibleError);
    expect((err as BindingStrictSchemaIncompatibleError).details['featuresListed']).toEqual([
      'output:$.mode.oneOf',
    ]);
  });

  it('accepts a compatible schema under auto_schema: strict', async () => {
    const target = writeTarget('good.mjs', COMPATIBLE, COMPATIBLE);
    const bindingFile = writeBinding('strict.good', target, 'strict');

    const modules = await loader.loadBindings(bindingFile, registry);
    expect(modules).toHaveLength(1);
    expect(registry.has('strict.good')).toBe(true);
  });

  it('does not enforce strict compatibility under auto_schema: permissive', async () => {
    const target = writeTarget(
      'permissive.mjs',
      { type: 'object', properties: { s: { type: 'string', minLength: 3 } } },
      COMPATIBLE,
    );
    const bindingFile = writeBinding('strict.off', target, 'permissive');

    const modules = await loader.loadBindings(bindingFile, registry);
    expect(modules).toHaveLength(1);
  });

  it('does not enforce strict compatibility under implicit auto_schema', async () => {
    const target = writeTarget(
      'implicit.mjs',
      { type: 'object', properties: { s: { type: 'string', minLength: 3 } } },
      COMPATIBLE,
    );
    const file = join(dir, 'b.binding.yaml');
    writeFileSync(file, `bindings:\n  - module_id: strict.implicit\n    target: "${target}:run"\n`);

    const modules = await loader.loadBindings(file, registry);
    expect(modules).toHaveLength(1);
  });
});
