/**
 * Cross-language conformance driver for `acl_root_discovery.json`
 * (D-64 Recommendation A / issue #74 — config-driven ACL discovery).
 *
 * Fixture source: apcore/conformance/fixtures/acl_root_discovery.json
 * (canonical). No `driver_contract` block; the `description` is the contract:
 * `acl.root` defaults to `./acl`, resolves relative to the config file's
 * directory when known, may name a directory (loading `<root>/global_acl.yaml`)
 * or a YAML file directly, and — the CRITICAL INVARIANT — a missing path
 * attaches NO ACL and MUST NOT synthesize an empty default-deny one even when
 * `acl.default_effect` is `deny`. Auto-discovery is skipped entirely when the
 * caller supplies their own Executor.
 *
 * `ACL.load` / `ACL.discover` are Node-only: the filesystem discoverer lives on
 * the side-effect module `src/acl-file.ts`, which `tests/setup-node-installers.ts`
 * imports for every suite (see vitest.config.ts `setupFiles`). No extra wiring
 * is needed here.
 *
 * `acl_attached` and `enforcement` are asserted BEHAVIOURALLY rather than by
 * reading a field: `Executor` keeps its ACL private (src/executor.ts:235), and
 * "an ACL is attached" is only meaningful if it actually gates calls. Each case
 * therefore wires the discovery result exactly as the `APCore` bootstrap does
 * (src/client.ts:129-134) and observes whether a call the policy denies is
 * blocked.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import yaml from 'js-yaml';
import { Type } from '@sinclair/typebox';

import { ACL } from '../src/acl.js';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { getDefault } from '../src/config-defaults.js';
import { FunctionModule } from '../src/decorator.js';
import { ACLDeniedError } from '../src/errors.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

interface AclRootCase {
  readonly id: string;
  readonly comment?: string;
  readonly acl_root?: string;
  readonly acl_root_unset?: boolean;
  readonly default_effect?: string;
  readonly caller_id?: string | null;
  readonly target_id?: string;
  readonly caller_supplied_executor?: boolean;
  readonly fs?: Record<string, string>;
  readonly expected: Record<string, unknown>;
}

interface AclRootFixture {
  readonly description: string;
  readonly default_acl_root: string;
  readonly acl_policy: { default_effect: string; rules: unknown[] };
  readonly test_cases: readonly AclRootCase[];
}

const fixture: AclRootFixture = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'acl_root_discovery.json'), 'utf-8'),
);

function caseById(id: string): AclRootCase {
  const tc = fixture.test_cases.find((c) => c.id === id);
  if (tc === undefined) {
    throw new Error(
      `acl_root_discovery.json no longer contains case '${id}'. The fixture is canonical — ` +
        'update this driver to match it, do not edit the fixture.',
    );
  }
  return tc;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** A module the fixture's policy denies (`@external` may only reach `greet`). */
const DENIED_TARGET = 'db.write';

function makeModule(id: string): FunctionModule {
  return new FunctionModule({
    execute: () => ({ value: 'ok' }),
    moduleId: id,
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({ value: Type.String() }),
    description: `Module ${id}`,
  });
}

let tmpDir: string;

/** Materialise a case's `fs` block under the temp root. */
function materialiseFs(tc: AclRootCase): void {
  for (const [entry, kind] of Object.entries(tc.fs ?? {})) {
    const target = path.join(tmpDir, entry);
    if (kind === 'directory' || entry.endsWith('/')) {
      fs.mkdirSync(target, { recursive: true });
      continue;
    }
    if (kind !== 'acl_policy') {
      throw new Error(
        `acl_root_discovery.json fs entry '${entry}' has unknown content '${kind}'. ` +
          'The fixture is canonical — extend the driver.',
      );
    }
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, yaml.dump(fixture.acl_policy), 'utf-8');
  }
}

/**
 * Build the case's Config. `acl.root` is written into a real apcore.yaml inside
 * the temp root so relative-path resolution anchors there (mirroring a real
 * project) rather than at the process CWD.
 */
function configFor(tc: AclRootCase): Config {
  const lines = ['version: "1.0.0"', 'project:', '  name: acl-root-conformance'];
  if (!tc.acl_root_unset) {
    lines.push('acl:', `  root: ${tc.acl_root}`);
    if (tc.default_effect !== undefined) lines.push(`  default_effect: ${tc.default_effect}`);
  }
  const configPath = path.join(tmpDir, 'apcore.yaml');
  fs.writeFileSync(configPath, `${lines.join('\n')}\n`, 'utf-8');
  return Config.load(configPath);
}

/** Wire a discovered ACL exactly as the APCore bootstrap does, then probe it. */
async function callIsBlocked(acl: ACL | null): Promise<boolean> {
  const registry = new Registry();
  registry.register(DENIED_TARGET, makeModule(DENIED_TARGET));
  const executor = new Executor({ registry });
  if (acl !== null) executor.setAcl(acl);
  try {
    await executor.call(DENIED_TARGET, {});
    return false;
  } catch (err) {
    if (err instanceof ACLDeniedError) return true;
    throw err;
  }
}

describe('Conformance: config-driven ACL discovery (acl_root_discovery.json)', () => {
  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-conf-acl-root-'));
  });

  afterEach(() => {
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch {
      /* ignore */
    }
  });

  // -------------------------------------------------------------------------
  // Cases asserting `resolved_acl_root` / `config_valid`.
  // -------------------------------------------------------------------------
  for (const id of ['default_value_when_unset', 'config_omitting_acl_root_is_valid']) {
    it(id, () => {
      const tc = caseById(id);
      expect(tc.acl_root_unset).toBe(true);

      let configError: unknown = null;
      let config: Config | null = null;
      try {
        // Config.load validates by default, so a successful load IS the
        // `config_valid` assertion: omitting acl.root must not be rejected.
        config = configFor(tc);
      } catch (err) {
        configError = err;
      }

      if ('config_valid' in tc.expected) {
        expect(configError === null, `config load failed: ${String(configError)}`).toBe(
          tc.expected['config_valid'],
        );
      } else if (configError !== null) {
        throw configError;
      }

      expect(getDefault('acl.root')).toBe(fixture.default_acl_root);
      expect(config!.get('acl.root', getDefault('acl.root'))).toBe(
        tc.expected['resolved_acl_root'],
      );
    });
  }

  // -------------------------------------------------------------------------
  // Cases asserting attachment / enforcement / a single check() decision.
  // -------------------------------------------------------------------------
  for (const id of [
    'present_dir_with_global_acl_attaches_and_enforces',
    'present_dir_enforcement_allows_permitted_call',
    'present_dir_enforcement_denies_other_call',
    'root_as_file_attaches',
    'missing_path_no_op',
    'dir_without_global_acl_no_op',
    'missing_path_with_default_deny_does_not_synthesize',
  ]) {
    it(id, async () => {
      const tc = caseById(id);
      materialiseFs(tc);
      const config = configFor(tc);

      const acl = ACL.discover(config);
      expect(acl !== null, 'acl_attached').toBe(tc.expected['acl_attached']);

      if ('enforcement' in tc.expected) {
        expect(await callIsBlocked(acl), 'enforcement').toBe(tc.expected['enforcement']);
      }

      if ('decision' in tc.expected) {
        expect(acl, `case '${id}' needs a loaded ACL to make a decision`).not.toBeNull();
        expect(
          acl!.check(tc.caller_id ?? null, tc.target_id!),
          `check(${JSON.stringify(tc.caller_id ?? null)}, '${tc.target_id}')`,
        ).toBe(tc.expected['decision']);
      }

      const handled = new Set(['acl_attached', 'enforcement', 'decision']);
      const unhandled = Object.keys(tc.expected).filter(
        (k) => !k.startsWith('_') && !handled.has(k),
      );
      expect(
        unhandled,
        `acl_root_discovery.json case '${id}' declares expectations this driver does not ` +
          'assert. The fixture is canonical — extend the driver, do not edit the fixture.',
      ).toEqual([]);
    });
  }

  // -------------------------------------------------------------------------
  it('caller_supplied_executor_skips_discovery', async () => {
    const tc = caseById('caller_supplied_executor_skips_discovery');
    materialiseFs(tc);
    const config = configFor(tc);

    // Control: the very same config WOULD attach and enforce if APCore built
    // the Executor itself. Without this, "not blocked" could just mean the ACL
    // file was never found.
    expect(ACL.discover(config)).not.toBeNull();

    const registry = new Registry();
    registry.register(DENIED_TARGET, makeModule(DENIED_TARGET));
    const executor = new Executor({ registry });
    const client = new APCore({ registry, executor, config });
    expect(client.executor).toBe(executor);

    let blocked = false;
    try {
      await client.call(DENIED_TARGET, {});
    } catch (err) {
      if (!(err instanceof ACLDeniedError)) throw err;
      blocked = true;
    }

    expect(blocked, 'acl_attached').toBe(tc.expected['acl_attached']);
    expect(blocked, 'enforcement').toBe(tc.expected['enforcement']);
  });

  // -------------------------------------------------------------------------
  it('drives every fixture case', () => {
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'default_value_when_unset',
      'config_omitting_acl_root_is_valid',
      'present_dir_with_global_acl_attaches_and_enforces',
      'present_dir_enforcement_allows_permitted_call',
      'present_dir_enforcement_denies_other_call',
      'root_as_file_attaches',
      'missing_path_no_op',
      'dir_without_global_acl_no_op',
      'missing_path_with_default_deny_does_not_synthesize',
      'caller_supplied_executor_skips_discovery',
    ]);
  });
});
