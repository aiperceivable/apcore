/**
 * Drive `multi_root_discovery.json` — §9.1.1 `extensions.roots` (#118 D-70).
 *
 * Every case discovers from a real tree with a real `apcore.yaml`. Calling
 * `scanMultiRoot` directly would prove the scanner works, which was never the
 * question in any SDK: this one never read the key, and apcore-rust read the
 * paths and dropped the namespaces.
 *
 * The two roots derive the SAME unprefixed ID on purpose. Without the prefix
 * they collide, so the namespace is observable rather than cosmetic.
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import yaml from 'js-yaml';

import { Config } from '../src/config.js';
import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

interface MultiRootCase {
  readonly id: string;
  readonly input: { readonly extensions: Record<string, unknown> };
  readonly expected: {
    readonly module_ids?: readonly string[];
    readonly raises?: boolean;
    readonly error_message_contains?: string;
  };
}

const fixture: { test_cases: readonly MultiRootCase[] } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'multi_root_discovery.json'), 'utf-8'),
);

const MODULE = [
  'export const Mod = {',
  "  inputSchema: { type: 'object' },",
  "  outputSchema: { type: 'object' },",
  "  description: 'Discoverable probe module.',",
  '  execute: () => ({ ok: true }),',
  '};',
  '',
].join('\n');

function tree(extensions: Record<string, unknown>): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-multi-root-'));
  for (const name of ['alpha', 'beta']) {
    const leaf = path.join(root, name, 'executor', 'svc');
    fs.mkdirSync(leaf, { recursive: true });
    fs.writeFileSync(path.join(leaf, 'mod.ts'), MODULE, 'utf-8');
  }
  fs.writeFileSync(
    path.join(root, 'apcore.yaml'),
    yaml.dump({ version: '1.0', project: { name: 'multi-root-probe' }, extensions }),
    'utf-8',
  );
  return root;
}

let cwd: string;
beforeEach(() => {
  cwd = process.cwd();
});
afterEach(() => {
  process.chdir(cwd);
});

describe('multi_root_discovery.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, async () => {
      const root = tree(testCase.input.extensions);
      process.chdir(root);
      const config = Config.load(path.join(root, 'apcore.yaml'));
      const registry = new Registry({ config });

      if (testCase.expected.raises === true) {
        await expect(registry.discover()).rejects.toThrow(
          new RegExp(testCase.expected.error_message_contains as string),
        );
        return;
      }
      await registry.discover();
      expect([...registry.moduleIds].sort()).toEqual([...testCase.expected.module_ids!]);
    });
  }
});
