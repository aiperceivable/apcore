import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { Config } from '../src/config.js';
import { SchemaLoader } from '../src/schema/loader.js';
import { findFixturesRoot } from './spec-repo.js';

interface VersionCase {
  id: string;
  format: 'config' | 'schema';
  version?: string;
  expected: { error_code: string | null };
}

const fixture: { test_cases: VersionCase[] } = JSON.parse(
  readFileSync(join(findFixturesRoot(), 'document_version_negotiation.json'), 'utf-8'),
);

test.each(fixture.test_cases)('document version negotiation: $id', ({ format, version, expected }) => {
  const root = mkdtempSync(join(tmpdir(), 'apcore-document-version-'));
  try {
    const path = join(root, format === 'config' ? 'apcore.yaml' : 'example.schema.yaml');
    const document = format === 'config'
      ? { apcore: { version, project: { name: 'conformance' } } }
      : {
        module_id: 'example',
        version,
        description: 'Conformance example',
        input_schema: { type: 'object' },
        output_schema: { type: 'object' },
      };
    // JSON is a subset of YAML and retains the string type of short versions.
    writeFileSync(path, JSON.stringify(document));
    const load = () => format === 'config'
      ? Config.load(path)
      : new SchemaLoader(new Config({}), root).load('example');

    if (expected.error_code === null) {
      expect(load).not.toThrow();
    } else {
      expect(load).toThrow(expect.objectContaining({ code: expected.error_code }));
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
