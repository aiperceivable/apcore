/**
 * Cross-language conformance driver for toggle_state_isolation.json (Issue #71).
 *
 * Fixture source: apcore/conformance/fixtures/toggle_state_isolation.json
 * (single source of truth). See that fixture's `description` for the contract.
 *
 * Each APCore instance owns one ToggleState, injected into BOTH the toggle
 * module (write path) and the Executor's pipeline lookup (read path). The
 * runner constructs the named instances in a single process, applies each
 * operation through the owning instance's toggle write path, then asserts
 * each instance's disabled-set through its OWN read path — never the
 * process-global DEFAULT_TOGGLE_STATE. The key contract: disabling a module
 * on instance A MUST NOT affect instance B, and a disable survives a reload
 * of its own instance.
 *
 * Read-path assertion strategy: a disabled module is observed by driving a
 * real pipeline call through the instance and checking it raises
 * ModuleDisabledError (the BuiltinModuleLookup step consults the same
 * per-instance ToggleState). This exercises the actual pipeline lookup, not
 * just the ToggleState field.
 */

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Type } from '@sinclair/typebox';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { ModuleDisabledError } from '../src/errors.js';
import { DEFAULT_TOGGLE_STATE } from '../src/sys-modules/toggle.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadFixture(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

// A trivial no-op module: opaque module_id values from the fixture are
// registered with this shape so the toggle write path (which requires the
// module to exist) and the read path (a real call) both have something to
// resolve.
const NoOpInput = Type.Object({}, { additionalProperties: true });
const NoOpOutput = Type.Object({ ok: Type.Boolean() }, { additionalProperties: true });

function registerNoOp(client: APCore, moduleId: string): void {
  client.module({
    id: moduleId,
    inputSchema: NoOpInput,
    outputSchema: NoOpOutput,
    description: 'No-op module for toggle-isolation conformance',
    execute: () => ({ ok: true }),
  });
}

function newInstance(): APCore {
  // Each instance gets its own per-instance ToggleState (Issue #71). Enabling
  // sys_modules + events wires the toggle write path (system.control.toggle_feature)
  // and the per-instance disable()/enable() convenience wrappers.
  const config = new Config({
    sys_modules: { enabled: true, events: { enabled: true } },
  });
  return new APCore({ config });
}

/**
 * Observe an instance's disabled-set through its OWN read path: a disabled
 * module raises ModuleDisabledError on a real call; an enabled (or never
 * disabled) module does not. Returns true if the module is observed disabled.
 */
async function isObservedDisabled(client: APCore, moduleId: string): Promise<boolean> {
  try {
    await client.call(moduleId, {});
    return false;
  } catch (err) {
    if (err instanceof ModuleDisabledError) return true;
    throw err;
  }
}

describe('Conformance: per-instance ToggleState isolation (Issue #71)', () => {
  const fixture = loadFixture('toggle_state_isolation');

  // Keep the process-global fallback clean so a regression that leaks toggles
  // into DEFAULT_TOGGLE_STATE is caught by the cross-instance assertions.
  beforeEach(() => {
    DEFAULT_TOGGLE_STATE.clear();
    // Silence the audit / control warn logs the toggle module emits.
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    DEFAULT_TOGGLE_STATE.clear();
    vi.restoreAllMocks();
  });

  fixture.test_cases.forEach((tc: any) => {
    it(tc.id, async () => {
      // 1. Construct one real APCore per named instance in the same process.
      const instances = new Map<string, APCore>();
      // Track which modules each instance has seen, so `reload` can
      // re-register exactly the same set while preserving ToggleState.
      const known = new Map<string, Set<string>>();
      for (const name of tc.instances as string[]) {
        instances.set(name, newInstance());
        known.set(name, new Set());
      }

      const ensureRegistered = (name: string, moduleId: string): void => {
        const client = instances.get(name)!;
        const seen = known.get(name)!;
        if (!client.registry.has(moduleId)) {
          registerNoOp(client, moduleId);
        }
        seen.add(moduleId);
      };

      // 2. Apply each operation in order through the owning instance.
      for (const op of tc.operations as any[]) {
        const client = instances.get(op.instance)!;
        if (op.action === 'disable') {
          ensureRegistered(op.instance, op.module_id);
          await client.disable(op.module_id, 'conformance: disable');
        } else if (op.action === 'enable') {
          ensureRegistered(op.instance, op.module_id);
          await client.enable(op.module_id, 'conformance: enable');
        } else if (op.action === 'reload') {
          // Re-register this instance's known modules, mirroring the real
          // hot-reload sequence (unregister old, register new). The Registry is
          // re-populated, but the per-instance ToggleState lives OUTSIDE the
          // Registry (Issue #71 / A-D-12) and MUST survive — so we never touch
          // client.toggleState here.
          const client2 = instances.get(op.instance)!;
          for (const moduleId of known.get(op.instance)!) {
            client2.registry.unregister(moduleId);
            await client2.registry.register(moduleId, {
              description: 'No-op module (reloaded) for toggle-isolation conformance',
              inputSchema: { type: 'object' },
              outputSchema: { type: 'object' },
              execute: () => ({ ok: true }),
            });
          }
        } else {
          throw new Error(`Unknown toggle operation action: ${op.action}`);
        }
      }

      // 3. Assert each instance's disabled-set via its OWN read path.
      for (const [name, expectedDisabled] of Object.entries(
        tc.expected_disabled as Record<string, string[]>,
      )) {
        const client = instances.get(name)!;
        const expectedSet = new Set(expectedDisabled);

        // Every module the instance has seen must report the expected state.
        // We union the expected-disabled ids with this instance's known ids so
        // that a module disabled-elsewhere (e.g. on instance B) is verified to
        // be NOT disabled here.
        const allIds = new Set<string>([...known.get(name)!, ...expectedDisabled]);
        for (const moduleId of allIds) {
          // A module that was never registered on this instance cannot be
          // exercised via the read path; assert isolation through the
          // per-instance ToggleState directly (still NOT DEFAULT_TOGGLE_STATE).
          if (!client.registry.has(moduleId)) {
            expect(client.toggleState.isDisabled(moduleId)).toBe(expectedSet.has(moduleId));
            continue;
          }
          const observed = await isObservedDisabled(client, moduleId);
          expect(observed).toBe(expectedSet.has(moduleId));
          // Cross-check the per-instance ToggleState read path agrees and that
          // nothing leaked into the process-global fallback.
          expect(client.toggleState.isDisabled(moduleId)).toBe(expectedSet.has(moduleId));
        }

        // Disabling on one instance MUST NOT leak into the global fallback.
        for (const moduleId of expectedDisabled) {
          expect(DEFAULT_TOGGLE_STATE.isDisabled(moduleId)).toBe(false);
        }
      }
    });
  });
});
