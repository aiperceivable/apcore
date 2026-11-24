/**
 * Tests for config-driven ACL discovery (D-64, Recommendation A — issue #74).
 *
 * Covers `ACL.discover(config)` resolution semantics and the end-to-end
 * activation wiring in the `APCore` bootstrap:
 *   - present acl file  => discover returns an ACL AND an inter-module call
 *                          denied by that ACL is actually blocked.
 *   - missing acl path  => discover returns null AND inter-module calls are
 *                          NOT blocked (no silent empty default-deny).
 *   - default `acl.root` resolves to "./acl".
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { writeFileSync, mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import { ACL } from '../src/acl.js';
import { Config } from '../src/config.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { FunctionModule } from '../src/decorator.js';
import { getDefault } from '../src/config-defaults.js';
import { ACLDeniedError } from '../src/errors.js';
import { APCore } from '../src/client.js';

function makeModule(id: string): FunctionModule {
  return new FunctionModule({
    execute: () => ({ value: 'ok' }),
    moduleId: id,
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({ value: Type.String() }),
    description: `Module ${id}`,
  });
}

// A deny-all ACL file: with default_effect deny and no allow rules, every
// inter-module call is blocked when this ACL is attached.
const DENY_ALL_ACL = 'default_effect: deny\nrules: []\n';

describe('ACL.discover (D-64 / #74)', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'apcore-acl-discover-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('returns an ACL when acl.root points at an existing file', () => {
    const aclPath = join(tmpDir, 'acl.yaml');
    writeFileSync(aclPath, DENY_ALL_ACL);

    const config = new Config({ acl: { root: aclPath } });
    const acl = ACL.discover(config);

    expect(acl).toBeInstanceOf(ACL);
    // A real caller -> real target is denied by the deny-all ACL.
    expect(acl!.check('a.mod', 'b.mod')).toBe(false);
  });

  it('loads <root>/global_acl.yaml when acl.root is a directory', () => {
    // acl.root is a directory by convention (the default "./acl"); discovery
    // must load the conventional global_acl.yaml inside it (PROTOCOL_SPEC §3.1).
    // Parity with apcore-python and apcore-rust. Regression: TS previously
    // passed the directory straight to readFileSync and threw EISDIR.
    const aclDir = join(tmpDir, 'acl');
    mkdirSync(aclDir);
    writeFileSync(join(aclDir, 'global_acl.yaml'), DENY_ALL_ACL);

    const config = new Config({ acl: { root: aclDir } });
    const acl = ACL.discover(config);

    expect(acl).toBeInstanceOf(ACL);
    expect(acl!.check('a.mod', 'b.mod')).toBe(false);
  });

  it('returns null when acl.root is a directory without global_acl.yaml', () => {
    // Directory exists but has no global_acl.yaml -> no-op (null), not EISDIR
    // and not a synthesized default-deny.
    const aclDir = join(tmpDir, 'acl');
    mkdirSync(aclDir);

    const config = new Config({ acl: { root: aclDir } });

    expect(ACL.discover(config)).toBeNull();
  });

  it('returns null when acl.root does not exist (no silent default-deny)', () => {
    const missing = join(tmpDir, 'nonexistent', 'acl.yaml');
    const config = new Config({ acl: { root: missing } });

    const acl = ACL.discover(config);

    // CRITICAL INVARIANT: a missing path must attach NOTHING — never an
    // empty default-deny ACL that would block every inter-module call.
    expect(acl).toBeNull();
  });

  it('resolves a relative acl.root against the config source file directory', () => {
    // Lay out: <tmpDir>/conf/project.yaml and <tmpDir>/conf/acl.yaml
    const confDir = join(tmpDir, 'conf');
    mkdirSync(confDir);
    const configPath = join(confDir, 'project.yaml');
    writeFileSync(
      configPath,
      'version: "1.0.0"\nproject:\n  name: acl-src-dir\nacl:\n  root: ./acl.yaml\n',
    );
    writeFileSync(join(confDir, 'acl.yaml'), DENY_ALL_ACL);

    // Load from file so config.sourcePath is populated; resolution must
    // anchor at confDir (the config's directory), NOT the process CWD.
    const config = Config.load(configPath);
    expect(config.sourcePath).toBe(configPath);

    const acl = ACL.discover(config);
    expect(acl).toBeInstanceOf(ACL);
    expect(acl!.check('a.mod', 'b.mod')).toBe(false);
  });

  it('resolves a relative acl.root against CWD when source path is unknown', () => {
    // new Config({...}) has no source file -> sourcePath is null.
    const config = new Config({ acl: { root: './acl-does-not-exist-here' } });
    expect(config.sourcePath).toBeNull();
    // Path resolved against CWD; it does not exist -> null (no enforcement).
    expect(ACL.discover(config)).toBeNull();
  });

  it('default acl.root resolves to "./acl"', () => {
    // Contract: the default value of acl.root is "./acl".
    expect(getDefault('acl.root')).toBe('./acl');

    // A config that omits acl.root falls back to the default. Resolved
    // against tmpDir (which has no ./acl), discovery yields null.
    const config = Config.load(
      (() => {
        const p = join(tmpDir, 'project.yaml');
        writeFileSync(p, 'version: "1.0.0"\nproject:\n  name: defaults-test\n');
        return p;
      })(),
    );
    expect(config.get('acl.root', getDefault('acl.root'))).toBe('./acl');
    // <tmpDir>/acl does not exist -> null, no enforcement.
    expect(ACL.discover(config)).toBeNull();
  });
});

describe('APCore bootstrap ACL activation (D-64 / #74)', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'apcore-acl-bootstrap-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('present acl file => inter-module call is blocked (enforcement active)', async () => {
    const aclPath = join(tmpDir, 'acl.yaml');
    writeFileSync(aclPath, DENY_ALL_ACL);

    const config = new Config({ acl: { root: aclPath } });
    const acl = ACL.discover(config);
    expect(acl).not.toBeNull();

    const registry = new Registry();
    registry.register('test.mod', makeModule('test.mod'));
    const executor = new Executor({ registry });
    executor.setAcl(acl!);

    // External caller, deny-all ACL -> blocked.
    await expect(executor.call('test.mod', {})).rejects.toThrow(ACLDeniedError);
  });

  it('missing acl path => inter-module call is NOT blocked (no enforcement)', async () => {
    const config = new Config({ acl: { root: join(tmpDir, 'no-such-acl.yaml') } });
    const acl = ACL.discover(config);
    expect(acl).toBeNull();

    const registry = new Registry();
    registry.register('test.mod', makeModule('test.mod'));
    const executor = new Executor({ registry });
    // Mirror the bootstrap: only setAcl when discover returned non-null.
    if (acl !== null) executor.setAcl(acl);

    // No ACL attached -> call succeeds, NO ACLDeniedError.
    const result = await executor.call('test.mod', {});
    expect(result['value']).toBe('ok');
  });

  it('relative acl.root anchors at config dir, not CWD', () => {
    const confDir = join(tmpDir, 'nested');
    mkdirSync(confDir);
    const configPath = join(confDir, 'project.yaml');
    writeFileSync(
      configPath,
      'version: "1.0.0"\nproject:\n  name: acl-anchor\nacl:\n  root: ./acl.yaml\n',
    );
    writeFileSync(join(confDir, 'acl.yaml'), DENY_ALL_ACL);

    const config = Config.load(configPath);
    const acl = ACL.discover(config);
    expect(acl).toBeInstanceOf(ACL);
    // Sanity: it loaded from the config dir, not from CWD.
    expect(resolve(confDir, 'acl.yaml')).toContain('nested');
  });

  it('new APCore(config) auto-attaches the discovered ACL (enforcement active)', async () => {
    const aclPath = join(tmpDir, 'acl.yaml');
    writeFileSync(aclPath, DENY_ALL_ACL);

    const client = new APCore({ config: new Config({ acl: { root: aclPath } }) });
    client.registry.register('test.mod', makeModule('test.mod'));

    // The bootstrap discovered and attached the deny-all ACL -> blocked.
    await expect(client.executor.call('test.mod', {})).rejects.toThrow(ACLDeniedError);
  });

  it('new APCore(config) with missing acl path leaves enforcement off', async () => {
    const client = new APCore({
      config: new Config({ acl: { root: join(tmpDir, 'absent.yaml') } }),
    });
    client.registry.register('test.mod', makeModule('test.mod'));

    // No ACL attached (missing path -> null) -> call succeeds.
    const result = await client.executor.call('test.mod', {});
    expect(result['value']).toBe('ok');
  });
});
