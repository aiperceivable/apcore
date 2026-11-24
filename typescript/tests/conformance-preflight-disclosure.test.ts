/**
 * Cross-language conformance driver for preflight_disclosure.json
 * (PROTOCOL_SPEC §12.8.5.1).
 *
 * `Executor.validate()` MUST NOT disclose module-level introspection to a caller
 * the ACL denied. `preflight()` and `preview()` are module-authored code whose
 * output names what the call would do — the resolved binary and argv of a
 * command-wrapping module, the target of a write. Module lookup is Step 3 and the
 * ACL check is Step 4, so gating those hooks on "lookup succeeded" alone runs them
 * for a denied caller and returns what they said.
 *
 * DRIVER CONTRACT: this suite MUST drive the real `Executor.validate()` against a
 * real `Registry` and a real `ACL`. The defect lives in `validate()`'s own gating,
 * so a driver that assembles a `PreflightResult` itself asserts nothing.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { ACL, type ACLRule } from '../src/acl.js';
import { Context, Identity } from '../src/context.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { jsonSchemaToTypeBox } from '../src/schema/loader-pure.js';
import type { Change, PreviewResult } from '../src/module.js';
import { findFixturesRoot } from './spec-repo.js';

interface DisclosureCase {
  readonly id: string;
  readonly description?: string;
  readonly input: {
    readonly acl_rules: readonly { callers: string[]; targets: string[]; effect: string }[];
    readonly default_effect: string;
    readonly caller_id: string;
    readonly inputs: Record<string, unknown>;
  };
  readonly expected: {
    readonly valid: boolean;
    readonly checks_present: readonly string[];
    readonly failed_checks: readonly string[];
    readonly checks_absent: readonly string[];
    readonly predicted_changes_count: number;
    readonly hooks_invoked: readonly string[];
    readonly sentinel_absent: boolean;
  };
}

interface DisclosureFixture {
  readonly module_contract: {
    readonly module_id: string;
    readonly sentinel: string;
    readonly input_schema: Record<string, unknown>;
    readonly output_schema: Record<string, unknown>;
    readonly preflight_returns: string[];
    readonly preview_change: Change;
  };
  readonly test_cases: readonly DisclosureCase[];
}

const fixture: DisclosureFixture = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'preflight_disclosure.json'), 'utf-8'),
);
const contract = fixture.module_contract;

/**
 * The fixture's `module_contract`, with an invocation recorder attached.
 *
 * `hooksInvoked` is observed inside the hook bodies rather than inferred from the
 * absent check entries: an implementation that calls the hooks and then drops
 * their results still ran module code for a denied caller, which is the
 * side-effect half of the requirement.
 */
class DestructiveModule {
  readonly inputSchema = jsonSchemaToTypeBox(contract.input_schema);
  readonly outputSchema = jsonSchemaToTypeBox(contract.output_schema);
  readonly description = 'Conformance module for the preflight disclosure gate';
  readonly hooksInvoked: string[] = [];

  execute(): Record<string, unknown> {
    throw new Error('validate() must never execute the module body');
  }

  preflight(): string[] {
    this.hooksInvoked.push('preflight');
    return [...contract.preflight_returns];
  }

  preview(): PreviewResult {
    this.hooksInvoked.push('preview');
    return { changes: [contract.preview_change] };
  }
}

describe('Conformance: validate() withholds module introspection from a denied caller', () => {
  it('drives every fixture case', () => {
    expect(fixture.test_cases.length).toBe(4);
  });

  fixture.test_cases.forEach((tc) => {
    it(tc.id, async () => {
      const registry = new Registry();
      const module = new DestructiveModule();
      registry.register(contract.module_id, module);

      const rules: ACLRule[] = tc.input.acl_rules.map((r) => ({
        callers: [...r.callers],
        targets: [...r.targets],
        effect: r.effect,
        description: 'conformance rule',
      }));
      const executor = new Executor({
        registry,
        acl: new ACL(rules, tc.input.default_effect),
      });
      const context = Context.create(new Identity(tc.input.caller_id, 'module'));

      const result = await executor.validate(contract.module_id, tc.input.inputs, context);
      const names = result.checks.map((c) => c.check);
      const detail =
        `\n  case: ${tc.id}` +
        `\n  description: ${tc.description ?? '(none)'}` +
        `\n  checks: ${JSON.stringify(result.checks.map((c) => [c.check, c.passed]))}` +
        `\n  hooksInvoked: ${JSON.stringify(module.hooksInvoked)}`;

      expect(result.valid, `valid mismatch${detail}`).toBe(tc.expected.valid);

      for (const name of tc.expected.checks_present) {
        expect(names, `check '${name}' MUST be present${detail}`).toContain(name);
      }

      // Absence is asserted on the check list itself: a present-but-empty
      // `module_preflight` entry is already the disclosure that the module
      // implements the hook.
      for (const name of tc.expected.checks_absent) {
        expect(names, `check '${name}' MUST NOT be present${detail}`).not.toContain(name);
      }

      const failed = result.checks
        .filter((c) => !c.passed)
        .map((c) => c.check)
        .sort();
      expect(failed, `failed-check set mismatch${detail}`).toEqual(
        [...tc.expected.failed_checks].sort(),
      );

      expect(
        result.predictedChanges?.length ?? 0,
        `predicted_changes count mismatch${detail}`,
      ).toBe(tc.expected.predicted_changes_count);

      expect(
        module.hooksInvoked,
        `module hook invocation mismatch — the hooks themselves must not run${detail}`,
      ).toEqual([...tc.expected.hooks_invoked]);

      // The sentinel appears in no value the Executor computes on its own, so
      // finding it anywhere in the serialized result proves introspection
      // reached the caller.
      const serialized = JSON.stringify({
        checks: result.checks,
        predictedChanges: result.predictedChanges ?? [],
      });
      if (tc.expected.sentinel_absent) {
        expect(
          serialized.includes(contract.sentinel),
          `sentinel '${contract.sentinel}' leaked to a denied caller${detail}\n  serialized: ${serialized}`,
        ).toBe(false);
      } else {
        expect(
          serialized.includes(contract.sentinel),
          `control case: the sentinel MUST reach a permitted caller, otherwise the ` +
            `denial cases pass for an implementation that never introspects at all${detail}`,
        ).toBe(true);
      }
    });
  });
});
