/**
 * Cross-language conformance driver for extension_point_lookup.json (D-108).
 *
 * Fixture source: apcore/conformance/fixtures/extension_point_lookup.json
 * (single source of truth). See that fixture's `description` for the contract.
 *
 * An UNKNOWN extension point is an error; an EMPTY one is not. The two halves
 * are asserted together because each is the other's control: an SDK that
 * answers null/[]/false for a misspelled name turns a typo into a wiring bug
 * that first surfaces at `apply()`, and an SDK that throws for a registered
 * point holding nothing breaks every host that probes before registering.
 *
 * The assertion is the CODE, not the class. Every throw in JavaScript is an
 * `Error`, so `toThrow()` alone would have stayed green against the bare
 * `throw new Error(...)` this decision replaced.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { ExtensionManager } from '../src/extensions.js';
import { ACL } from '../src/acl.js';
import { Middleware } from '../src/middleware/index.js';
import type { ModuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadFixture(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

/** A distinct object per construction — `unregister` compares by identity. */
class LookupMiddleware extends Middleware {}

/** One SDK-native extension satisfying `pointName`'s declared type. */
function makeExtension(pointName: string): unknown {
  if (pointName === 'middleware') return new LookupMiddleware();
  if (pointName === 'acl') return new ACL([], 'deny');
  throw new Error(`extension_point_lookup: no factory for point '${pointName}'`);
}

const fixture = loadFixture('extension_point_lookup');

describe('conformance: extension_point_lookup (D-108)', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, () => {
      const mgr = new ExtensionManager();
      const registered: unknown[] = [];
      for (const pointName of testCase.setup as string[]) {
        const ext = makeExtension(pointName);
        mgr.register(pointName, ext);
        registered.push(ext);
      }

      const pointName: string = testCase.point_name;
      const operation: string = testCase.operation;
      const expected = testCase.expected;

      const invoke = (): unknown => {
        if (operation === 'get') return mgr.get(pointName);
        if (operation === 'get_all') return mgr.getAll(pointName);
        if (operation === 'unregister') {
          // A stranger of the SAME type, never registered, so the call is a
          // genuine identity miss rather than a type mismatch.
          const target =
            testCase.unregister_target === 'setup'
              ? registered[0]
              : makeExtension(pointName === 'acl' ? 'acl' : 'middleware');
          return mgr.unregister(pointName, target);
        }
        throw new Error(`[${testCase.id}] unknown operation '${operation}'`);
      };

      const errorCode: string | null = expected.error_code;
      if (errorCode !== null) {
        let caught: unknown;
        let answered: unknown;
        let threw = false;
        try {
          answered = invoke();
        } catch (e) {
          threw = true;
          caught = e;
        }
        expect(
          threw,
          `[${testCase.id}] ${operation}('${pointName}') must reject an unregistered ` +
            `extension point with ${errorCode}, but answered ${JSON.stringify(answered)}`,
        ).toBe(true);
        expect((caught as ModuleError).code).toBe(errorCode);
        return;
      }

      // No error expected: the call must answer, and answer this.
      const result = invoke();
      if ('value' in expected) {
        expect(result !== null && result !== undefined).toBe(expected.value === 'present');
      } else if ('count' in expected) {
        expect((result as unknown[]).length).toBe(expected.count);
      } else if ('removed' in expected) {
        expect(result).toBe(expected.removed);
      } else {
        throw new Error(`[${testCase.id}] expected block names no assertion`);
      }
    });
  }
});
