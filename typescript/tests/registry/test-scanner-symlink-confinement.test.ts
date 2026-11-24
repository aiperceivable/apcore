/**
 * A symlink whose target escapes the extensions root is never discovered
 * (spec v1.50.0, D-94).
 *
 * The confinement check has to run BEFORE the directory/file split, not inside
 * the directory branch. apcore-python's lived inside it, so a symlinked `.py`
 * whose target was outside the root was discovered and imported — code
 * execution from outside the tree the operator configured, on the one branch
 * that yields importable files.
 *
 * This SDK is correct today. The test exists because nothing said so: D-94 is
 * a security decision, and the discovery path is the one place where a silent
 * regression means executing a file the operator never put there. A behaviour
 * with no test is a behaviour the next refactor is free to change.
 */

import { mkdirSync, writeFileSync, symlinkSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { scanExtensions } from '../../src/registry/scanner.js';

describe('symlink confinement (D-94)', () => {
  let tmp: string;
  let root: string;
  let outside: string;

  beforeEach(() => {
    tmp = join(tmpdir(), `apcore-d94-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    root = join(tmp, 'extensions');
    outside = join(tmp, 'sibling');
    mkdirSync(root, { recursive: true });
    mkdirSync(outside, { recursive: true });
  });

  afterEach(() => rmSync(tmp, { recursive: true, force: true }));

  function scan(followSymlinks: boolean): string[] {
    const found = (scanExtensions as (r: string, d: number, f: boolean) => unknown[])(
      root,
      8,
      followSymlinks,
    );
    return found.map((f) => String((f as { filePath?: string }).filePath ?? f));
  }

  it('does not discover a symlinked FILE whose target escapes the root', () => {
    // The file branch is the one that matters: a symlinked directory that
    // escapes is a traversal problem, a symlinked module file is an execution
    // problem.
    writeFileSync(join(outside, 'escape.ts'), 'export class Escape {}', 'utf-8');
    symlinkSync(join(outside, 'escape.ts'), join(root, 'innocent.ts'));

    expect(scan(true)).toEqual([]);
    expect(scan(false)).toEqual([]);
  });

  it('does not discover through a symlinked DIRECTORY that escapes the root', () => {
    mkdirSync(join(outside, 'pkg'));
    writeFileSync(join(outside, 'pkg', 'escape.ts'), 'export class Escape {}', 'utf-8');
    symlinkSync(join(outside, 'pkg'), join(root, 'linked'));

    expect(scan(true)).toEqual([]);
    expect(scan(false)).toEqual([]);
  });

  it('still discovers a real file inside the root', () => {
    // The control. Without it a scanner that discovers NOTHING passes both
    // cases above, and the confinement check would be indistinguishable from a
    // broken scan.
    writeFileSync(join(root, 'real.ts'), 'export class Real {}', 'utf-8');

    expect(scan(true).length).toBe(1);
    expect(scan(false).length).toBe(1);
  });

  it('still follows a symlink whose target stays INSIDE the root', () => {
    // The second control: confinement must reject what escapes, not every
    // symlink. `followSymlinks=false` skips it for the ordinary reason.
    mkdirSync(join(root, 'real'));
    writeFileSync(join(root, 'real', 'inside.ts'), 'export class Inside {}', 'utf-8');
    symlinkSync(join(root, 'real'), join(root, 'alias'));

    expect(scan(true).length).toBeGreaterThanOrEqual(1);
  });
});
