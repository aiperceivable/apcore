/**
 * Drive `acl_audit_delivery.json` — §6.3.2 (#118 D-66).
 *
 * Every case loads a real ACL file and drives real `check()` calls. Reading the
 * parsed block back off a config object would prove the parser works, which was
 * never the question: what had no contract at all was **delivery**.
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { describe, it, expect, vi } from 'vitest';
import yaml from 'js-yaml';

import { ACL, AUDIT_EVENT_NAME } from '../src/acl.js';
import type { AuditEntry } from '../src/acl.js';
import { ConfigError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';
import '../src/acl-file.js';

interface AuditCase {
  readonly id: string;
  readonly input: {
    readonly acl_file: Record<string, unknown>;
    readonly callback: 'none' | 'collecting' | 'failing';
    readonly checks: ReadonlyArray<{ caller_id: string; target_id: string }>;
  };
  readonly expected: Record<string, unknown>;
}

const fixture: { test_cases: readonly AuditCase[] } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'acl_audit_delivery.json'), 'utf-8'),
);

describe('acl_audit_delivery.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, () => {
      const { input, expected } = testCase;
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-acl-audit-'));
      const file = path.join(dir, 'acl.yaml');
      fs.writeFileSync(file, yaml.dump(input.acl_file), 'utf-8');

      const collected: AuditEntry[] = [];
      const callback =
        input.callback === 'collecting'
          ? (e: AuditEntry) => void collected.push(e)
          : input.callback === 'failing'
            ? (): never => {
                throw new Error('the audit sink is down');
              }
            : null;

      const spies = {
        debug: vi.spyOn(console, 'debug').mockImplementation(() => {}),
        info: vi.spyOn(console, 'info').mockImplementation(() => {}),
        warn: vi.spyOn(console, 'warn').mockImplementation(() => {}),
        error: vi.spyOn(console, 'error').mockImplementation(() => {}),
      };
      const restore = (): void => Object.values(spies).forEach((s) => s.mockRestore());

      if (expected['loads'] === false) {
        let thrown: unknown;
        try {
          ACL.load(file, callback);
        } catch (e) {
          thrown = e;
        }
        restore();
        expect(thrown).toBeInstanceOf(ConfigError);
        expect((thrown as ConfigError).code).toBe(expected['error_code']);
        expect(String(thrown)).toContain(expected['error_message_contains']);
        return;
      }

      const acl = ACL.load(file, callback);
      const decisions = input.checks.map((c) =>
        acl.check(c.caller_id, c.target_id) ? 'allow' : 'deny',
      );
      // Read before restoring: `mockRestore` clears the recorded calls, and
      // doing it the other way round reports zero for a case that emitted.
      const audit = Object.entries(spies).flatMap(([level, spy]) =>
        spy.mock.calls
          .filter((c) => c[0] === AUDIT_EVENT_NAME)
          .map((c) => ({ level, payload: c[1] as Record<string, unknown> })),
      );
      const messages = Object.values(spies).flatMap((s) =>
        s.mock.calls.filter((c) => c[0] !== AUDIT_EVENT_NAME).map((c) => String(c[0])),
      );
      restore();

      if ('decisions' in expected) expect(decisions).toEqual(expected['decisions']);
      if ('callback_decisions' in expected) {
        expect(collected.map((e) => e.decision)).toEqual(expected['callback_decisions']);
      }
      if ('default_sink_records' in expected) {
        expect(audit).toHaveLength(expected['default_sink_records'] as number);
      }
      if ('default_sink_field_names' in expected) {
        expect(Object.keys(audit[0]!.payload).sort()).toEqual(
          [...(expected['default_sink_field_names'] as string[])].sort(),
        );
      }
      if ('default_sink_level' in expected) {
        expect(audit[0]!.level).toBe(expected['default_sink_level']);
      }
      if ('default_sink_decisions' in expected) {
        expect(audit.map((a) => a.payload['decision'])).toEqual(
          expected['default_sink_decisions'],
        );
      }
      if ('load_warning_contains' in expected) {
        expect(messages.some((m) => m.includes(expected['load_warning_contains'] as string)))
          .toBe(true);
      }
      if ('load_warning_absent' in expected) {
        expect(messages.some((m) => m.includes(expected['load_warning_absent'] as string)))
          .toBe(false);
      }
      if ('override_warning_names' in expected) {
        const hits = messages.filter((m) => m.includes('does not apply'));
        expect(hits).toHaveLength(1);
        for (const field of expected['override_warning_names'] as string[]) {
          expect(hits[0]).toContain(field);
        }
      }
      if ('delivery_failure_reports' in expected) {
        const hits = messages.filter((m) => m.includes('audit delivery failed'));
        expect(hits).toHaveLength(expected['delivery_failure_reports'] as number);
      }
    });
  }
});
