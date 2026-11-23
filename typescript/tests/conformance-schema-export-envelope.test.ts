/**
 * Drive `schema_export_envelope.json` — the `Registry.exportSchema` envelope.
 *
 * Four keys, no more. Until this was pinned, TypeScript added `name`,
 * `version`, `tags`, `annotations` and `examples`, which made its export a
 * partial, non-conforming duplicate of `system.manifest.module`
 * (sys-manifest-module.schema.json) rather than a schema export.
 */
import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

interface Case {
  id: string;
  module?: Record<string, unknown>;
  module_id?: string;
  strict: boolean;
  expected: Record<string, unknown> | null;
}

const fixture = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'schema_export_envelope.json'), 'utf-8'),
) as { envelope_keys: string[]; test_cases: Case[] };

/** Build a duck-typed module carrying whatever the fixture declares, so the
 * test proves the exporter DROPS descriptor metadata rather than proving it was
 * never present. */
function makeModule(spec: Record<string, unknown>): Record<string, unknown> {
  const mod: Record<string, unknown> = {
    inputSchema: spec['input_schema'],
    outputSchema: spec['output_schema'],
    description: spec['description'],
    execute: () => ({}),
  };
  for (const attr of ['version', 'tags', 'annotations', 'examples', 'name']) {
    if (attr in spec) mod[attr] = spec[attr];
  }
  return mod;
}

async function exportFor(tc: Case): Promise<Record<string, unknown> | null> {
  const registry = new Registry();
  let moduleId: string;
  if (tc.module) {
    moduleId = tc.module['module_id'] as string;
    await registry.register(moduleId, makeModule(tc.module));
  } else {
    moduleId = tc.module_id as string;
  }
  return registry.exportSchema(moduleId, tc.strict);
}

describe('Conformance: Registry.exportSchema envelope', () => {
  fixture.test_cases.forEach((tc) => {
    it(tc.id, async () => {
      const result = await exportFor(tc);

      if (tc.expected === null) {
        expect(result, 'an unregistered module must export null').toBeNull();
        return;
      }

      // EXACT key set — a subset check would not catch the extra keys this pins.
      expect(Object.keys(result!).sort()).toEqual([...fixture.envelope_keys].sort());
      expect(result).toEqual(tc.expected);
    });
  });

  it('carries no sibling definitions key', async () => {
    // `$defs` live inside input_schema where JSON Schema puts them.
    const tc = fixture.test_cases.find(
      (c) => c.id === 'defs_stay_inside_input_schema_no_sibling_definitions_key',
    )!;
    const result = (await exportFor(tc))!;
    expect(result).not.toHaveProperty('definitions');
    expect(result['input_schema']).toHaveProperty('$defs');
  });
});
