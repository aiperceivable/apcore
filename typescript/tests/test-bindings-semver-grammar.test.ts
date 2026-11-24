/**
 * PROTOCOL_SPEC §9.1.2 requirement 6 — the pinned SemVer grammar, both halves.
 *
 * The rejection half alone is not coverage, and this file exists because that
 * is exactly how a defect shipped in apcore-rust: the only case anyone wrote
 * was `version: "1.0"` expecting a rejection, and it passed against a pattern
 * that rejected *everything*, valid SemVer included. A constraint that refuses
 * valid input is as broken as one that accepts invalid input, and only the
 * acceptance half can tell the two apart.
 *
 * TypeScript was never affected — it writes the grammar as a single regex
 * literal with no continuation — but the assertion belongs in all three SDKs,
 * because the grammar is pinned in the specification precisely so three
 * implementations cannot diverge, and an untested implementation cannot show
 * that it has not.
 */

import { describe, expect, it } from 'vitest';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import yaml from 'js-yaml';

import { BindingLoader } from '../src/bindings.js';
import { Config } from '../src/config.js';
import { Registry } from '../src/registry/registry.js';

const ACCEPTED = [
  '0.0.0',
  '1.0.0',
  '0.1.0',
  '10.20.30',
  '1.2.3-rc.1',
  '1.2.3-0.3.7',
  '1.2.3+build.5',
  '1.2.3-beta.1+exp.sha.5114f85',
];

const REJECTED = ['1.0', '1', 'v1.0.0', '1.0.0.0', '01.0.0', '1.0.0 '];

/** Load a one-entry binding file at `version`, with the semver check ON. */
async function loadWithVersion(version: string): Promise<string | null> {
  const dir = mkdtempSync(join(tmpdir(), 'semver-'));
  const file = join(dir, 'probe.binding.yaml');
  writeFileSync(
    file,
    yaml.dump({
      spec_version: '1.0',
      bindings: [
        {
          module_id: 'probe.module',
          target: 'probe_targets:noop',
          version,
          input_schema: { type: 'object', properties: {} },
          output_schema: { type: 'object', properties: {} },
        },
      ],
    }),
  );
  const config = Config.fromDefaults();
  config.set('validation.binding.version_require_semver', true);
  try {
    await new BindingLoader().loadBindings(file, new Registry(), config);
    return null;
  } catch (e) {
    const message = String((e as Error).message);
    // Target resolution failing is not what this file asserts on; only a
    // version rejection counts as a rejection here.
    return message.includes('is not SemVer') ? message : null;
  }
}

describe('§9.1.2 SemVer grammar', () => {
  it.each(ACCEPTED)('accepts the valid SemVer %s', async (version) => {
    expect(await loadWithVersion(version)).toBeNull();
  });

  it.each(REJECTED)('rejects %s', async (version) => {
    expect(await loadWithVersion(version)).toContain('is not SemVer');
  });

  it('uses a pattern containing no whitespace', async () => {
    // There is no `x` flag, so a stray space would be a mandatory character in
    // the subject string. Pinned separately from the behaviour above because it
    // is the *mechanism* of the apcore-rust defect: a reformat put literal
    // spaces into the pattern and every version stopped matching.
    const source = await import('node:fs').then((fs) =>
      fs.readFileSync(new URL('../src/bindings.ts', import.meta.url), 'utf-8'),
    );
    const match = source.match(/const SEMVER_RE =\s*(\/\^.*?\/);/s);
    expect(match, 'SEMVER_RE literal not found in src/bindings.ts').not.toBeNull();
    expect(match![1]).not.toMatch(/\s/);
  });
});
