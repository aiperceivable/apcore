import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  rmSync,
  chmodSync,
  realpathSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, sep, isAbsolute } from 'node:path';
import { scanExtensions, scanMultiRoot } from '../../src/registry/scanner.js';
import { ConfigNotFoundError, ConfigError } from '../../src/errors.js';

let tempDir: string;

beforeEach(() => {
  tempDir = mkdtempSync(join(tmpdir(), 'scanner-test-'));
});

afterEach(() => {
  rmSync(tempDir, { recursive: true, force: true });
});

function touch(relativePath: string, content = ''): string {
  const full = join(tempDir, relativePath);
  const dir = full.substring(0, full.lastIndexOf(sep));
  mkdirSync(dir, { recursive: true });
  writeFileSync(full, content);
  return full;
}

describe('scanExtensions', () => {
  it('discovers .ts and .js files and returns correct DiscoveredModule shape', () => {
    touch('alpha.ts');
    touch('beta.js');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(2);
    const ids = results.map((r) => r.canonicalId).sort();
    expect(ids).toEqual(['alpha', 'beta']);

    for (const mod of results) {
      expect(mod.filePath).toBeTruthy();
      expect(mod.canonicalId).toBeTruthy();
      expect(mod.metaPath).toBeNull();
      expect(mod.namespace).toBeNull();
    }
  });

  it('builds dot-notation canonicalId from nested paths', () => {
    touch('sub/module.ts');
    touch('deep/nested/handler.js');

    const results = scanExtensions(tempDir);
    const ids = results.map((r) => r.canonicalId).sort();
    expect(ids).toEqual(['deep.nested.handler', 'sub.module']);
  });

  it('skips .d.ts declaration files', () => {
    touch('real.ts');
    touch('types.d.ts');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('real');
  });

  it('skips .test.ts, .test.js, .spec.ts, and .spec.js files', () => {
    touch('handler.ts');
    touch('handler.test.ts');
    touch('handler.test.js');
    touch('handler.spec.ts');
    touch('handler.spec.js');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('handler');
  });

  it('skips dot-prefixed and underscore-prefixed entries', () => {
    touch('.hidden/secret.ts');
    touch('_private/internal.ts');
    touch('.env.ts');
    touch('_helper.ts');
    touch('visible.ts');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('visible');
  });

  it('skips node_modules and __pycache__ directories', () => {
    touch('node_modules/pkg/index.ts');
    touch('__pycache__/cached.ts');
    touch('real.ts');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('real');
  });

  it('ignores non-.ts/.js files', () => {
    touch('readme.md');
    touch('config.yaml');
    touch('data.json');
    touch('valid.ts');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('valid');
  });

  it('detects companion _meta.yaml files', () => {
    touch('handler.ts');
    touch('handler_meta.yaml', 'description: a handler');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].metaPath).toBe(join(tempDir, 'handler_meta.yaml'));
  });

  it('sets metaPath to null when no companion _meta.yaml exists', () => {
    touch('handler.ts');
    touch('handler_meta.json', '{}');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].metaPath).toBeNull();
  });

  it('respects maxDepth parameter', () => {
    touch('level1.ts');
    touch('a/level2.ts');
    touch('a/b/level3.ts');

    const resultsDepth1 = scanExtensions(tempDir, 1);
    expect(resultsDepth1.map((r) => r.canonicalId).sort()).toEqual(['level1']);

    const resultsDepth2 = scanExtensions(tempDir, 2);
    expect(resultsDepth2.map((r) => r.canonicalId).sort()).toEqual(['a.level2', 'level1']);

    const resultsAll = scanExtensions(tempDir, 8);
    expect(resultsAll.map((r) => r.canonicalId).sort()).toEqual(['a.b.level3', 'a.level2', 'level1']);
  });

  it('throws ConfigNotFoundError for non-existent root directory', () => {
    const bogus = join(tempDir, 'does-not-exist');
    expect(() => scanExtensions(bogus)).toThrow(ConfigNotFoundError);
  });

  it('deduplicates by canonicalId (first file wins)', () => {
    touch('handler.ts');
    touch('handler.js');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('handler');
  });

  it('handles deeply nested structures correctly', () => {
    touch('a/b/c/d/e/module.ts');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('a.b.c.d.e.module');
  });

  it('returns empty array for an empty directory', () => {
    const results = scanExtensions(tempDir);
    expect(results).toEqual([]);
  });

  it('filePath is the canonical real path of the discovered file (D-127)', () => {
    // This compared against `join(tempDir, 'mod.ts')` until spec v1.56.0.
    // D-127 keys file identity, the module ID and visited-directory tracking on
    // the canonical REAL path, and the reported path follows: an alias is a
    // name, not the module, and the loader has to open the target. Under a root
    // with a symlinked ancestor (`/var` -> `/private/var` on macOS, which is
    // where this test's tmpdir lives) the two spellings differ, so the
    // expectation is stated in real-path terms rather than reconstructed from
    // the path the caller happened to pass in.
    touch('mod.ts');

    const results = scanExtensions(tempDir);
    expect(results).toHaveLength(1);
    expect(results[0].filePath).toBe(realpathSync(join(tempDir, 'mod.ts')));
    expect(isAbsolute(results[0].filePath)).toBe(true);
  });
});

describe('scanMultiRoot', () => {
  it('prefixes canonicalId with namespace', () => {
    const rootA = join(tempDir, 'rootA');
    mkdirSync(rootA);
    writeFileSync(join(rootA, 'handler.ts'), '');

    const results = scanMultiRoot([{ root: rootA, namespace: 'ns1' }]);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('ns1.handler');
    expect(results[0].namespace).toBe('ns1');
  });

  it('auto-uses directory basename as namespace when not specified', () => {
    const rootDir = join(tempDir, 'myextensions');
    mkdirSync(rootDir);
    writeFileSync(join(rootDir, 'action.ts'), '');

    const results = scanMultiRoot([{ root: rootDir }]);
    expect(results).toHaveLength(1);
    expect(results[0].canonicalId).toBe('myextensions.action');
    expect(results[0].namespace).toBe('myextensions');
  });

  it('throws ConfigError for duplicate namespaces', () => {
    const rootA = join(tempDir, 'a');
    const rootB = join(tempDir, 'b');
    mkdirSync(rootA);
    mkdirSync(rootB);

    expect(() =>
      scanMultiRoot([
        { root: rootA, namespace: 'dup' },
        { root: rootB, namespace: 'dup' },
      ]),
    ).toThrow(ConfigError);
  });

  it('merges results from multiple roots', () => {
    const rootA = join(tempDir, 'rootA');
    const rootB = join(tempDir, 'rootB');
    mkdirSync(rootA);
    mkdirSync(rootB);
    writeFileSync(join(rootA, 'foo.ts'), '');
    writeFileSync(join(rootB, 'bar.ts'), '');

    const results = scanMultiRoot([
      { root: rootA, namespace: 'a' },
      { root: rootB, namespace: 'b' },
    ]);
    expect(results).toHaveLength(2);
    const ids = results.map((r) => r.canonicalId).sort();
    expect(ids).toEqual(['a.foo', 'b.bar']);
  });

  it('propagates ConfigNotFoundError for non-existent root', () => {
    const bogus = join(tempDir, 'nonexistent');
    expect(() => scanMultiRoot([{ root: bogus, namespace: 'ns' }])).toThrow(ConfigNotFoundError);
  });

  it('handles empty roots array', () => {
    const results = scanMultiRoot([]);
    expect(results).toEqual([]);
  });

  it('preserves metaPath through multi-root scan', () => {
    const rootDir = join(tempDir, 'ext');
    mkdirSync(rootDir);
    writeFileSync(join(rootDir, 'mod.ts'), '');
    writeFileSync(join(rootDir, 'mod_meta.yaml'), 'description: test');

    const results = scanMultiRoot([{ root: rootDir, namespace: 'pkg' }]);
    expect(results).toHaveLength(1);
    expect(results[0].metaPath).toBe(join(rootDir, 'mod_meta.yaml'));
    expect(results[0].canonicalId).toBe('pkg.mod');
  });
});
