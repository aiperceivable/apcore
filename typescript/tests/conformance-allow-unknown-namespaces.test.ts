/**
 * Drive `allow_unknown_namespaces.json` — §9.6.3 `_config.allow_unknown`
 * (#118 D-69).
 *
 * Both halves of the `strict: false` row were inert. `allow_unknown: false` is
 * documented as "silently ignored (not stored)" and the namespace was stored
 * anyway; `allow_unknown: true` is documented as "stored, accessible, **WARN
 * logged**" and nothing logged. Fixing one without the other leaves the row
 * half true, so the fixture drives both — and the legacy-mode boundary
 * besides, so the namespace-only scoping is a decision rather than an omission.
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { describe, it, expect, vi } from 'vitest';
import yaml from 'js-yaml';

import { Config } from '../src/config.js';
import { ConfigError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

interface AllowUnknownCase {
  readonly id: string;
  readonly input: {
    readonly mode: 'namespace' | 'legacy';
    readonly config?: Record<string, unknown> | null;
    readonly namespace?: string | null;
    /** D-117: the case declares a REGISTERED namespace rather than a document one. */
    readonly registered_namespace?: { readonly name: string; readonly defaults: Record<string, unknown> };
    readonly key?: string;
  };
  readonly expected: Record<string, unknown>;
}

const fixture: { test_cases: readonly AllowUnknownCase[] } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'allow_unknown_namespaces.json'), 'utf-8'),
);

const BASE = { version: '1.0', project: { name: 'allow-unknown-probe' } };

/**
 * D-117: a registered namespace's defaults answer only in NAMESPACE mode.
 *
 * A legacy document has no namespaces, so a declaration ABOUT a namespace has
 * nothing to say about one. The key is absent from the file by construction —
 * if it were present the document would be answering, not the registration.
 */
function driveRegisteredNamespaceDefault(testCase: AllowUnknownCase): void {
  const { input, expected } = testCase;
  const registration = input.registered_namespace!;
  // Namespace registration is process-wide and permanent (§9.6.3 point 5), so
  // each case registers under its own name rather than racing the other.
  const name = `${registration.name}_${testCase.id.slice(0, 12)}`;
  try {
    Config.registerNamespace({ name, defaults: { ...registration.defaults } });
  } catch {
    // Already registered by a previous run in this process.
  }

  const doc: Record<string, unknown> =
    input.mode === 'namespace' ? { apcore: { ...BASE } } : { ...BASE };
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-d117-'));
  const file = path.join(dir, 'apcore.yaml');
  fs.writeFileSync(file, yaml.dump(doc), 'utf-8');

  const config = Config.load(file);
  const key = input.key!.replace(registration.name, name);
  const value = config.get(key);

  expect(
    value !== undefined && value !== null,
    `${testCase.id}: get(${key}) -> ${JSON.stringify(value)}; the registration ` +
      'must answer in namespace mode and stay silent for a legacy document',
  ).toBe(expected['value_readable']);
  if ('value' in expected) {
    expect(value).toBe(expected['value']);
  }
}

describe('allow_unknown_namespaces.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, () => {
      const { input, expected } = testCase;

      if (input.registered_namespace !== undefined) {
        driveRegisteredNamespaceDefault(testCase);
        return;
      }

      const doc: Record<string, unknown> =
        input.mode === 'namespace' ? { apcore: { ...BASE } } : { ...BASE };
      if (input.config != null) doc['_config'] = { ...input.config };
      if (input.namespace != null) doc[input.namespace] = { x: 1 };

      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-allow-unknown-'));
      const file = path.join(dir, 'apcore.yaml');
      fs.writeFileSync(file, yaml.dump(doc), 'utf-8');

      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      const info = vi.spyOn(console, 'info').mockImplementation(() => {});

      if (expected['loads'] === false) {
        let thrown: unknown;
        try {
          Config.load(file);
        } catch (e) {
          thrown = e;
        }
        warn.mockRestore();
        info.mockRestore();
        expect(thrown).toBeInstanceOf(ConfigError);
        expect((thrown as ConfigError).code).toBe(expected['error_code']);
        expect(String(thrown)).toContain(expected['error_message_contains']);
        return;
      }

      const config = Config.load(file);
      const messages = [...warn.mock.calls, ...info.mock.calls].map((c) => String(c[0]));
      warn.mockRestore();
      info.mockRestore();

      if ('value_readable' in expected) {
        const value = config.get(`${input.namespace}.x`);
        expect(value !== undefined && value !== null).toBe(expected['value_readable']);
      }
      if ('warns_naming' in expected) {
        const needle = expected['warns_naming'] as string;
        const hits = messages.filter((m) => m.includes(needle) && m.includes('registered'));
        expect(hits).toHaveLength(1);
      }
      if ('warns_absent' in expected) {
        const needle = expected['warns_absent'] as string;
        expect(messages.some((m) => m.includes(needle))).toBe(false);
      }
    });
  }
});
