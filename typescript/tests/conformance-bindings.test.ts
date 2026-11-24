/**
 * Cross-language conformance drivers for the two binding fixtures.
 *
 * Fixture sources (single source of truth):
 *   - apcore/conformance/fixtures/binding_errors.json
 *   - apcore/conformance/fixtures/binding_yaml_canonical.yaml
 *
 * Both were previously `it.skip`'d in tests/conformance.test.ts with the
 * reason "BindingLoader requires real file I/O and dynamic imports". That
 * reason was wrong: `binding_errors` asserts error MESSAGE parity by
 * constructing the error objects directly (protocol-spec §5.12.8),
 * and `binding_yaml_canonical` asserts the YAML parses and round-trips. Both
 * apcore-python and apcore-rust drive these; this brings TypeScript to parity.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as yaml from 'js-yaml';
import {
  BindingFileInvalidError,
  BindingInvalidTargetError,
  BindingModuleNotFoundError,
  BindingSchemaInferenceFailedError,
  BindingSchemaModeConflictError,
} from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

interface BindingErrorCase {
  id: string;
  error_code: string;
  input: Record<string, string | string[]>;
  expected_message?: string;
  expected_message_contains?: string[];
}

const bindingErrors = JSON.parse(
  fs.readFileSync(path.join(FIXTURES_ROOT, 'binding_errors.json'), 'utf-8'),
) as { test_cases: BindingErrorCase[] };

describe('Conformance: binding_errors.json', () => {
  for (const tc of bindingErrors.test_cases) {
    it(tc.id, () => {
      const input = tc.input;
      let message: string;

      switch (tc.error_code) {
        case 'BINDING_FILE_INVALID':
          message = new BindingFileInvalidError(
            input['file_path'] as string,
            input['reason'] as string,
          ).message;
          break;
        case 'BINDING_SCHEMA_MODE_CONFLICT':
          message = new BindingSchemaModeConflictError(
            input['module_id'] as string,
            input['modes_listed'] as string[],
            input['file_path'] as string,
          ).message;
          break;
        case 'BINDING_SCHEMA_INFERENCE_FAILED':
          message = new BindingSchemaInferenceFailedError(
            input['target'] as string,
            input['module_id'] as string,
            input['file_path'] as string,
          ).message;
          break;
        case 'BINDING_INVALID_TARGET':
          message = new BindingInvalidTargetError(input['target'] as string).message;
          break;
        case 'BINDING_MODULE_NOT_FOUND':
          message = new BindingModuleNotFoundError(input['module_path'] as string).message;
          break;
        case 'PIPELINE_HANDLER_NOT_SUPPORTED':
          // Rust-only error code — apcore-js does not raise it (apcore-python
          // skips this case for the same reason).
          return;
        default:
          throw new Error(`Unhandled binding error_code '${tc.error_code}' in fixture`);
      }

      if (tc.expected_message !== undefined) {
        expect(message).toBe(tc.expected_message);
      }
      for (const substring of tc.expected_message_contains ?? []) {
        expect(message).toContain(substring);
      }
    });
  }
});

describe('Conformance: binding_yaml_canonical.yaml', () => {
  const doc = yaml.load(
    fs.readFileSync(path.join(FIXTURES_ROOT, 'binding_yaml_canonical.yaml'), 'utf-8'),
  ) as { spec_version?: string; bindings: Array<Record<string, unknown>> };

  it('parses and yields the three canonical binding entries', () => {
    expect(Array.isArray(doc.bindings)).toBe(true);
    expect(doc.bindings).toHaveLength(3);
    const ids = doc.bindings.map((b) => b['module_id']);
    expect(ids).toContain('conformance.auto_permissive');
    expect(ids).toContain('conformance.explicit_schema');
    expect(ids).toContain('conformance.auto_strict');
  });

  it('entry 1 declares auto_schema permissive mode', () => {
    const entry = doc.bindings.find((b) => b['module_id'] === 'conformance.auto_permissive')!;
    expect(entry['target']).toBe('conformance_mod:auto_permissive_fn');
    expect(entry['auto_schema']).toBe(true);
    expect(entry['version']).toBe('1.0.0');
    expect(entry['tags']).toEqual(['conformance', 'auto_schema']);
    // Assert the FULL declared key set, not a two-key subset. The fixture used
    // to carry only `readonly` and `idempotent` — two keys every SDK handles —
    // so a binding parser covering 5 of the 12 schema-declared annotation
    // properties passed this case (DEC-002). `extra` is checked separately
    // below because it is the one key whose §4.4.1 wire rule a parser can get
    // wrong in a way the others cannot: routing unknown keys INTO it re-nests
    // extension data under its own name.
    const annotations = entry['annotations'] as Record<string, unknown>;
    expect(annotations['readonly']).toBe(true);
    expect(annotations['idempotent']).toBe(true);
    expect(annotations['destructive']).toBe(false);
    expect(annotations['requires_approval']).toBe(true);
    expect(annotations['open_world']).toBe(false);
    expect(annotations['cacheable']).toBe(true);
    expect(annotations['cache_ttl']).toBe(300);
    expect(annotations['cache_key_fields']).toEqual(['a', 'b']);
    expect(annotations['paginated']).toBe(true);
    expect(annotations['pagination_style']).toBe('offset');
    expect(annotations['extra']).toEqual({ 'mcp.category': 'email' });
  });

  it('entry 2 declares explicit input/output schemas and display metadata', () => {
    const entry = doc.bindings.find((b) => b['module_id'] === 'conformance.explicit_schema')!;
    expect(entry).toHaveProperty('input_schema');
    expect(entry).toHaveProperty('output_schema');
    expect(entry['version']).toBe('2.0.0');
    expect(entry['display']).toEqual({ alias: 'explicit_test', cli: { alias: 'explicit-test' } });
  });

  it('entry 3 declares auto_schema strict mode', () => {
    const entry = doc.bindings.find((b) => b['module_id'] === 'conformance.auto_strict')!;
    expect(entry['auto_schema']).toBe('strict');
    expect(entry['metadata']).toEqual({ owner_team: 'conformance' });
  });
});
