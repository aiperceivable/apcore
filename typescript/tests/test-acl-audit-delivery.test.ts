/**
 * PROTOCOL_SPEC §6.3.2 — ACL audit delivery (apcore#118, decision D-66).
 *
 * §6.3.1 has always specified the *record*. Nothing specified **delivery**:
 * the ACL constructor's `auditLogger` was the whole surface, with no default
 * sink, no statement of what happens when delivery fails, and no meaning for
 * the `audit:` block's three settings — which were declared in two places and
 * read in neither.
 *
 * The sharpest consequence was measured, not inferred: a throwing audit
 * callback propagated out of `check()` and turned an **allowed** call into an
 * error. That is the one behaviour change here, and it is a fix.
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import yaml from 'js-yaml';

import { ACL, AUDIT_EVENT_NAME } from '../src/acl.js';
import type { ACLRule, AuditEntry } from '../src/acl.js';
import { ConfigError } from '../src/errors.js';
import '../src/acl-file.js';

const RULES = [{ callers: ['api.*'], targets: ['executor.*'], effect: 'allow' }];

let dir: string;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-audit-'));
});
afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

function write(audit?: unknown, extra: Record<string, unknown> = {}): string {
  const doc: Record<string, unknown> = { version: '1.0.0', rules: RULES, ...extra };
  if (audit !== undefined) doc['audit'] = audit;
  const file = path.join(dir, 'acl.yaml');
  fs.writeFileSync(file, yaml.dump(doc), 'utf-8');
  return file;
}

function allowRule(): ACLRule {
  return { callers: ['api.*'], targets: ['executor.*'], effect: 'allow', description: '' };
}

/** Capture every console method the sink and its diagnostics can use. */
function captureConsole() {
  const spies = {
    debug: vi.spyOn(console, 'debug').mockImplementation(() => {}),
    info: vi.spyOn(console, 'info').mockImplementation(() => {}),
    warn: vi.spyOn(console, 'warn').mockImplementation(() => {}),
    error: vi.spyOn(console, 'error').mockImplementation(() => {}),
  };
  return {
    audit: () =>
      Object.entries(spies).flatMap(([level, spy]) =>
        spy.mock.calls
          .filter((c) => c[0] === AUDIT_EVENT_NAME)
          .map((c) => ({ level, payload: c[1] as Record<string, unknown> })),
      ),
    warnings: () => spies.warn.mock.calls.map((c) => String(c[0])),
    restore: () => Object.values(spies).forEach((s) => s.mockRestore()),
  };
}

describe('requirement 2 — declaration activates the default sink', () => {
  it('produces no audit output when no block is declared', () => {
    // The compatibility boundary. `enabled` defaults to true, so a merged-view
    // reading would switch a log record per ACL check on for every ACL file in
    // existence — a behaviour change measured in volume.
    const acl = ACL.load(write());
    const c = captureConsole();
    acl.check('api.x', 'executor.y');
    const records = c.audit();
    c.restore();
    expect(records).toEqual([]);
  });

  it('activates the default sink when the block is declared', () => {
    const acl = ACL.load(write({ enabled: true }));
    const c = captureConsole();
    acl.check('api.x', 'executor.y');
    const records = c.audit();
    c.restore();
    expect(records).toHaveLength(1);
  });

  it('is silent for a declared block with enabled: false', () => {
    const acl = ACL.load(write({ enabled: false }));
    const c = captureConsole();
    acl.check('api.x', 'executor.y');
    const records = c.audit();
    c.restore();
    expect(records).toEqual([]);
  });

  it('carries all thirteen fields as structured data', () => {
    const acl = ACL.load(write({ enabled: true }));
    const c = captureConsole();
    acl.check('api.x', 'executor.y');
    const records = c.audit();
    c.restore();
    expect(Object.keys(records[0]!.payload).sort()).toEqual([
      'approval_required', 'call_depth', 'caller_id', 'decision', 'handler_error',
      'identity_type', 'matched_rule', 'matched_rule_index', 'reason', 'roles',
      'target_id', 'timestamp', 'trace_id',
    ]);
    expect(records[0]!.payload['caller_id']).toBe('api.x');
  });

  it.each([
    ['trace', 'debug'], ['debug', 'debug'], ['info', 'info'],
    ['warn', 'warn'], ['error', 'error'],
  ])('log_level %s emits at %s', (level, method) => {
    const acl = ACL.load(write({ enabled: true, log_level: level }));
    const c = captureConsole();
    acl.check('api.x', 'executor.y');
    const records = c.audit();
    c.restore();
    expect(records[0]!.level).toBe(method);
  });
});

describe('requirement 1 — one effective sink, never two', () => {
  it('a callback receives every entry and the block does not apply', () => {
    // The API-beats-configuration rule in the direction that matters:
    // `include_denied: false` must not silently truncate a compliance sink.
    const seen: AuditEntry[] = [];
    const acl = ACL.load(write({ enabled: false, include_denied: false }), (e) => seen.push(e));
    const c = captureConsole();
    acl.check('api.x', 'executor.y');
    acl.check('worker.x', 'executor.y');
    const records = c.audit();
    c.restore();
    expect(seen.map((e) => e.decision)).toEqual(['allow', 'deny']);
    expect(records).toEqual([]);
  });

  it('names every overridden field, not only the most visible one', () => {
    const c = captureConsole();
    new ACL([allowRule()], 'deny', () => {}, {
      enabled: true, include_denied: false, log_level: 'error',
    });
    const hits = c.warnings().filter((l) => l.includes('does not apply'));
    c.restore();
    expect(hits).toHaveLength(1);
    for (const field of ['audit.enabled', 'audit.include_denied', 'audit.log_level']) {
      expect(hits[0]).toContain(field);
    }
  });

  it('says nothing when there is no block to override', () => {
    const c = captureConsole();
    new ACL([allowRule()], 'deny', () => {});
    const hits = c.warnings().filter((l) => l.includes('does not apply'));
    c.restore();
    expect(hits).toEqual([]);
  });
});

describe('requirement 3 — delivery never changes the access decision', () => {
  const throwing = (): never => {
    throw new Error('the audit sink is down');
  };

  it.each([['api.x', true], ['worker.x', false]] as const)(
    'a throwing callback leaves the %s decision at %s',
    (caller, expected) => {
      // Measured before spec v1.45.0: this threw, and an ALLOWED call became an
      // error. Both decisions are driven — deny is a different branch.
      const acl = new ACL([allowRule()], 'deny', throwing);
      const c = captureConsole();
      const decision = acl.check(caller, 'executor.y');
      c.restore();
      expect(decision).toBe(expected);
    },
  );

  it.each([['api.x', true], ['worker.x', false]] as const)(
    'the async check is unaffected too (%s)',
    async (caller, expected) => {
      const acl = new ACL([allowRule()], 'deny', throwing);
      const c = captureConsole();
      const decision = await acl.asyncCheck(caller, 'executor.y');
      c.restore();
      expect(decision).toBe(expected);
    },
  );

  it('reports a failing sink once, not once per check', () => {
    const acl = new ACL([allowRule()], 'deny', throwing);
    const c = captureConsole();
    for (let i = 0; i < 5; i++) acl.check('api.x', 'executor.y');
    const hits = c.warnings().filter((l) => l.includes('audit delivery failed'));
    c.restore();
    expect(hits).toHaveLength(1);
  });
});

describe('requirement 4 — the callback must be synchronous', () => {
  it('treats a Promise-returning callback as an invalid delivery', async () => {
    // Its rejection would surface after the decision has been returned, outside
    // the containment requirement 3 promises, and as an unhandled rejection.
    const acl = new ACL([allowRule()], 'deny', (() =>
      Promise.reject(new Error('too late to matter'))) as never);
    const c = captureConsole();
    expect(acl.check('api.x', 'executor.y')).toBe(true);
    acl.check('api.x', 'executor.y');
    const hits = c.warnings().filter((l) => l.includes('returned a Promise'));
    c.restore();
    expect(hits).toHaveLength(1);
    expect(hits[0]).toContain('§6.3.2');
    await new Promise((r) => setTimeout(r, 0)); // let the caught rejection settle
  });
});

describe('requirement 6 — include_denied', () => {
  it('withholds denials from the default sink', () => {
    const acl = ACL.load(write({ enabled: true, include_denied: false }));
    const c = captureConsole();
    acl.check('api.x', 'executor.y');
    acl.check('worker.x', 'executor.y');
    const records = c.audit();
    c.restore();
    expect(records.map((r) => r.payload['decision'])).toEqual(['allow']);
  });

  it('warns once per load', () => {
    const c = captureConsole();
    ACL.load(write({ include_denied: false }));
    const hits = c.warnings().filter((l) => l.includes('include_denied'));
    c.restore();
    expect(hits).toHaveLength(1);
    expect(hits[0]).toContain('DENIED');
  });

  it('is silent when include_denied is true', () => {
    const c = captureConsole();
    ACL.load(write({ include_denied: true }));
    const hits = c.warnings().filter((l) => l.includes('include_denied'));
    c.restore();
    expect(hits).toEqual([]);
  });
});

describe('requirement 7 — reload', () => {
  it('refreshes the block and preserves the callback', () => {
    const file = write({ enabled: true, log_level: 'info' });
    const acl = ACL.load(file);
    let c = captureConsole();
    acl.check('api.x', 'executor.y');
    let records = c.audit();
    c.restore();
    expect(records[0]!.level).toBe('info');

    fs.writeFileSync(
      file,
      yaml.dump({ version: '1.0.0', rules: RULES, audit: { enabled: true, log_level: 'error' } }),
      'utf-8',
    );
    acl.reload();
    c = captureConsole();
    acl.check('api.x', 'executor.y');
    records = c.audit();
    c.restore();
    expect(records[0]!.level).toBe('error');
  });

  it('starts a new failure-report scope', () => {
    const throwing = (): never => {
      throw new Error('down');
    };
    const file = write({ enabled: true });
    const acl = ACL.load(file, throwing);

    let c = captureConsole();
    acl.check('api.x', 'executor.y');
    acl.check('api.x', 'executor.y');
    let hits = c.warnings().filter((l) => l.includes('audit delivery failed'));
    c.restore();
    expect(hits).toHaveLength(1);

    acl.reload();
    c = captureConsole();
    acl.check('api.x', 'executor.y');
    hits = c.warnings().filter((l) => l.includes('audit delivery failed'));
    c.restore();
    expect(hits).toHaveLength(1);
  });
});

describe('requirement 8 — the block is validated, nothing else gets stricter', () => {
  it.each([
    { enabled: 'yes' },
    { log_level: 'verbose' },
    { include_denied: 1 },
    { enabled: true, unknown_key: true },
    'not-a-mapping',
  ])('rejects %j at load', (bad) => {
    const c = captureConsole();
    expect(() => ACL.load(write(bad))).toThrow(ConfigError);
    c.restore();
  });

  it('still ignores every other unknown root key', () => {
    const c = captureConsole();
    const acl = ACL.load(
      write({ enabled: true }, { telemetry: { enabled: true }, x_vendor_note: 'kept' }),
    );
    const hits = c.warnings().filter((l) => l.includes('telemetry'));
    c.restore();
    expect(acl.rules).toHaveLength(1);
    expect(hits).toEqual([]);
  });
});
