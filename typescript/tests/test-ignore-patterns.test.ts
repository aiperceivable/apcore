/**
 * PROTOCOL_SPEC §3.5 / §3.6 A04 step 3a — `extensions.ignore_patterns`.
 *
 * A MUST with no supplier until spec v1.42.0: the key was registered in all
 * three SDKs' configuration key surfaces and read by none of them, so a project
 * that excluded a directory from discovery had it scanned and its modules
 * registered anyway. The failure direction is what earns it a file: a skip rule
 * that fails **open** loads code the operator asked not to load.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';

import { scanExtensions } from '../src/registry/scanner.js';

const SUBDIRS = ['keep', 'fixtures', 'vendor'] as const;
const ALL = ['executor.fixtures.mod', 'executor.keep.mod', 'executor.vendor.mod'];

function tree(subdirs: readonly string[] = SUBDIRS): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-ignore-'));
  for (const sub of subdirs) {
    const dir = path.join(root, 'executor', sub);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'mod.ts'), 'export class Mod {}\n');
  }
  return root;
}

const discover = (patterns: string[], subdirs?: readonly string[]): string[] =>
  scanExtensions(tree(subdirs), 8, false, patterns)
    .map((m) => m.canonicalId)
    .sort();

describe('extensions.ignore_patterns', () => {
  it('discovers everything when nothing is configured', () => {
    // The half that keeps this additive: an absent key changes nothing.
    expect(discover([])).toEqual(ALL);
  });

  it.each([
    ['literal', ['fixtures'], ['executor.keep.mod', 'executor.vendor.mod']],
    ['star', ['ven*'], ['executor.fixtures.mod', 'executor.keep.mod']],
    ['question mark', ['?endor'], ['executor.fixtures.mod', 'executor.keep.mod']],
    ['two entries', ['fixtures', 'vendor'], ['executor.keep.mod']],
  ])('excludes the entry a %s pattern names', (_label, patterns, expected) => {
    expect(discover(patterns as string[])).toEqual(expected);
  });

  it('matches case-sensitively', () => {
    // §9.2.3 declares this surface sensitive, unlike `sensitive_keys`: these are
    // filenames, and folding them would make one configuration behave
    // differently on a case-insensitive filesystem than on the case-sensitive
    // one it was written against.
    expect(discover(['FIXTURES'])).toEqual(ALL);
  });

  it('matches a segment, not a path', () => {
    // A04 step 3a says ENTRY NAME, so `*` cannot cross a directory boundary:
    // the segments here are `executor`, `fixtures`, `mod.ts`. An implementation
    // matching against the path would exclude everything.
    expect(discover(['executor/fixtures'])).toEqual(ALL);
  });

  it('cannot switch off a built-in row', () => {
    // §3.5: the two lists are a UNION. "Extend the ignore list" could be read
    // as "replace it", and a configuration that re-enabled `.git/` would be a
    // discovery surface nobody expects.
    expect(discover(['nothing_matches_this'], ['keep', '.hidden'])).toEqual([
      'executor.keep.mod',
    ]);
  });

  it('drops an empty entry', () => {
    // A25 anchors, so `''` would match only the empty name — an operator who
    // leaves a blank line in a YAML list means nothing by it.
    expect(discover(['', 'fixtures'])).toEqual(['executor.keep.mod', 'executor.vendor.mod']);
  });
});
