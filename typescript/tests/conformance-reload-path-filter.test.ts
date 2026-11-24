/**
 * Cross-language conformance driver for `reload_path_filter.json`
 * (Issue #45.4 — docs/features/system-modules.md
 * #14-granular-reload-via-path-filtering).
 *
 * Fixture source: apcore/conformance/fixtures/reload_path_filter.json
 * (canonical). No `driver_contract` block; the `description` is the contract:
 * `system.control.reload_module` accepts a `path_filter` glob, `path_filter`
 * and `module_id` are mutually exclusive (`MODULE_RELOAD_CONFLICT`), and a
 * `path_filter` matching zero modules is a no-op rather than an error.
 *
 * DRIVER SHAPE: each case exercises the real `ReloadModule.execute()` against a
 * real `Registry`. `Registry.discover()` is stubbed to re-register the case's
 * `registered_modules`, which is what a genuine re-discovery of an unchanged
 * source tree does — without it the reload path has nothing to find and the
 * fixture's `reloaded_modules_set` would be untestable. Nothing else about the
 * reload path is stubbed: matching, unregistering, re-lookup, and the audit /
 * event side-effects all run.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import { ModuleError } from '../src/errors.js';
import { EventEmitter } from '../src/events/emitter.js';
import { Registry } from '../src/registry/registry.js';
import { InMemoryAuditStore } from '../src/sys-modules/audit.js';
import { ReloadModule } from '../src/sys-modules/control.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

interface ReloadCase {
  readonly id: string;
  readonly registered_modules: readonly string[];
  readonly input: Record<string, unknown>;
  readonly expected: Record<string, unknown>;
}

function loadFixture(name: string): { description: string; test_cases: readonly ReloadCase[] } {
  return JSON.parse(fs.readFileSync(path.join(findFixturesRoot(), `${name}.json`), 'utf-8'));
}

const fixture = loadFixture('reload_path_filter');

function dummyModule(): Record<string, unknown> {
  return {
    description: 'Conformance stand-in module',
    version: '1.0.0',
    execute: () => ({ result: 'ok' }),
  };
}

describe('Conformance: reload_module path_filter (reload_path_filter.json)', () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    // ReloadModule logs its audit line through console.warn when no AuditStore
    // is wired; silence it without hiding the behaviour under test.
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
    vi.restoreAllMocks();
  });

  fixture.test_cases.forEach((tc) => {
    it(tc.id, async () => {
      const registry = new Registry();
      for (const id of tc.registered_modules) {
        registry.registerInternal(id, dummyModule());
      }

      // Re-discovery of an unchanged tree re-registers what was unregistered.
      vi.spyOn(registry, 'discover').mockImplementation(async () => {
        let n = 0;
        for (const id of tc.registered_modules) {
          if (registry.get(id) === null) {
            registry.registerInternal(id, dummyModule());
            n += 1;
          }
        }
        return n;
      });

      const auditStore = new InMemoryAuditStore();
      const mod = new ReloadModule(registry, new EventEmitter(), auditStore);

      let result: Record<string, unknown> | null = null;
      let error: unknown = null;
      try {
        result = await mod.execute({ ...tc.input }, null);
      } catch (err) {
        error = err;
      }

      const expected = tc.expected;
      const handled = new Set<string>();

      if ('error_code' in expected) {
        handled.add('error_code');
        expect(error, `case '${tc.id}' expected an error, got result ${JSON.stringify(result)}`)
          .toBeInstanceOf(ModuleError);
        expect((error as ModuleError).code).toBe(expected['error_code']);
      } else if (error !== null) {
        throw error;
      }

      if ('success' in expected) {
        handled.add('success');
        expect(result?.['success']).toBe(expected['success']);
      }

      if ('error' in expected) {
        handled.add('error');
        // The fixture states `error: null` — the no-match case must not raise.
        expect(error).toBe(expected['error']);
      }

      if ('reloaded_modules_set' in expected) {
        handled.add('reloaded_modules_set');
        // path_filter mode reports `reloaded_modules`; single-module mode
        // reports the one `module_id` it reloaded. The fixture asks for the set
        // of reloaded modules either way.
        const reloaded = Array.isArray(result?.['reloaded_modules'])
          ? (result?.['reloaded_modules'] as string[])
          : typeof result?.['module_id'] === 'string'
            ? [result['module_id'] as string]
            : [];
        expect([...reloaded].sort()).toEqual([...(expected['reloaded_modules_set'] as string[])].sort());
      }

      if ('audit_target_module_ids_set' in expected) {
        handled.add('audit_target_module_ids_set');
        // D-111. Asserted as a SET of concrete ids rather than a count: an
        // aggregate entry keyed on the glob has count 1, which a count
        // assertion accepts whenever the glob matches one module, and its
        // target is one `query({ moduleId })` can never find.
        const entries = auditStore.query();
        expect(entries.map((e) => e.targetModuleId).sort()).toEqual(
          [...(expected['audit_target_module_ids_set'] as string[])].sort(),
        );
        for (const moduleId of expected['audit_target_module_ids_set'] as string[]) {
          expect(auditStore.query({ moduleId }).map((e) => e.targetModuleId)).toEqual([moduleId]);
        }
      }

      if (expected['audit_correlation_ids_are_equal_and_non_empty'] === true) {
        handled.add('audit_correlation_ids_are_equal_and_non_empty');
        // A property, not a value: the id is generated per call. All equal
        // rejects a fresh id per entry, which groups nothing; non-empty
        // rejects leaving the field unset.
        const ids = new Set(auditStore.query().map((e) => e.correlationId));
        expect(ids.size, `entries from one bulk reload must share one id`).toBe(1);
        expect([...ids][0]).not.toBe('');
      }

      // Fail loudly rather than silently pass if the fixture grows an
      // expectation this driver does not assert.
      const unhandled = Object.keys(expected).filter(
        (k) => !k.startsWith('_') && !handled.has(k),
      );
      expect(
        unhandled,
        `reload_path_filter.json case '${tc.id}' declares expectations this driver does not ` +
          'assert. The fixture is canonical — extend the driver, do not edit the fixture.',
      ).toEqual([]);
    });
  });

  it('drives every fixture case', () => {
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'glob_filter_restricts_rediscovery',
      'no_filter_uses_module_id_for_single_reload',
      'no_match_filter_is_no_op',
      'module_id_and_path_filter_conflict',
      // The pattern DIALECT — PROTOCOL_SPEC 6.7 clause 4 / 9.2.3 (#117).
      'question_mark_is_a_wildcard_in_path_filter',
      'bracket_is_a_literal_and_MUST_NOT_be_rejected',
      'character_class_does_not_expand',
      'double_star_is_two_ordinary_stars',
      // D-121: the deprecated field is accepted and changes nothing.
      'reload_dependents_is_accepted_and_inert',
      // D-111: per-module audit entries sharing one correlation id.
      'bulk_reload_audits_per_module_with_one_correlation_id',
    ]);
  });
});
