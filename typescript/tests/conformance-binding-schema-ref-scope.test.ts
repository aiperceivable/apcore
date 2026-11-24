import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { expect, test } from 'vitest';
import { BindingLoader } from '../src/bindings.js';
import { Config } from '../src/config.js';
import { ModuleError } from '../src/errors.js';
import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

interface BindingCase {
  id: string;
  schema_file?: string;
  schema_ref: string;
  schema_root?: string;
  absolute?: boolean;
  invalid_document?: boolean;
  expected: { registered: boolean; error_code: string | null };
}

const fixture: { test_cases: BindingCase[] } = JSON.parse(
  readFileSync(join(findFixturesRoot(), 'binding_schema_ref_scope.json'), 'utf-8'),
);

test.each(fixture.test_cases)('binding schema reference scope: $id', async (testCase) => {
  const root = mkdtempSync(join(tmpdir(), 'apcore-binding-schema-ref-'));
  try {
    const bindingDir = join(root, 'bindings');
    mkdirSync(bindingDir);
    let reference = testCase.schema_ref;
    if (testCase.schema_file !== undefined) {
      const schemaPath = join(root, testCase.schema_file);
      mkdirSync(dirname(schemaPath), { recursive: true });
      const schema = { input_schema: { type: 'object' }, output_schema: { type: 'object' } };
      writeFileSync(schemaPath, testCase.invalid_document ? '[' : JSON.stringify(schema));
      if (testCase.absolute) reference = schemaPath;
    }

    const moduleId = 'fixture.echo';
    const bindingPath = join(bindingDir, 'fixture.binding.yaml');
    writeFileSync(bindingPath, JSON.stringify({
      spec_version: '1.0',
      bindings: [{ module_id: moduleId, target: 'js-yaml:load', description: 'Echo', schema_ref: reference }],
    }));
    let config: Config | undefined;
    if (testCase.schema_root !== undefined) {
      const configPath = join(root, 'apcore.yaml');
      writeFileSync(configPath, JSON.stringify({
        version: '2.0.0', project: { name: 'fixture' }, schema: { root: testCase.schema_root },
      }));
      config = Config.load(configPath);
    }

    const registry = new Registry();
    let actualError: string | null = null;
    try {
      await new BindingLoader().loadBindings(bindingPath, registry, config);
    } catch (error) {
      if (!(error instanceof ModuleError)) throw error;
      actualError = error.code;
    }

    expect(actualError).toBe(testCase.expected.error_code);
    expect(registry.has(moduleId)).toBe(testCase.expected.registered);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
