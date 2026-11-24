/**
 * Spec-traced contract tests for the apcore ACL system (TypeScript SDK).
 *
 * Mirrors the canonical Python suite
 *   apcore-python/tests/test_acl_system_spec.py
 * Each `it(...)` name begins with the VERBATIM clause id of the form
 *   `acl_system.<method>.<kind>.<detail>`
 * so a cross-language diff can match rows by exact clause id.
 *
 * Tests ONLY — production source is never modified.
 *
 * Derived from /apcore/docs/features/acl-system.md (## Contract: blocks).
 */

import { describe, it, expect } from 'vitest';
import { writeFileSync, mkdtempSync, rmSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { ACL } from '../src/acl.js';
import type { ACLRule, AuditEntry } from '../src/acl.js';
import { ACLRuleError, ConfigNotFoundError } from '../src/errors.js';
// Side-effect import installs the Node-side YAML loader onto ACL.load.
import '../src/acl-file.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeTmpDir(): string {
  return mkdtempSync(join(tmpdir(), 'acl-spec-'));
}

function writeYaml(dir: string, body: string, name = 'acl.yaml'): string {
  const path = join(dir, name);
  writeFileSync(path, body, 'utf-8');
  return path;
}

function rule(
  callers: string[],
  targets: string[],
  effect: string,
  description = '',
  conditions: Record<string, unknown> | null = null,
): ACLRule {
  return { callers, targets, effect, description, conditions };
}

const VALID_YAML = `version: "1.0"
default_effect: deny
rules:
  - callers: ["api.*"]
    targets: ["db.*"]
    effect: allow
    description: "API to DB"
`;

// ===========================================================================
// Contract: ACL.check
// ===========================================================================

describe('Contract: ACL.check', () => {
  // acl_system.check.property.async
  it('acl_system.check.property.async: check() is declared async:false (plain bool, not Promise)', () => {
    const acl = new ACL([rule(['*'], ['*'], 'allow')]);
    const result = acl.check('api.x', 'db.y');
    expect(result).not.toBeInstanceOf(Promise);
    expect(result).toBe(true);
  });

  // acl_system.check.property.thread_safe
  it('acl_system.check.property.thread_safe: N>=8 concurrent checks, no error, consistent state', async () => {
    const acl = new ACL([rule(['api.*'], ['db.*'], 'allow')], 'deny');
    const results = await Promise.all(
      Array.from({ length: 16 }, (_, i) =>
        Promise.resolve().then(() => acl.check(`api.${i}`, 'db.read')),
      ),
    );
    expect(results).toHaveLength(16);
    expect(results.every((r) => r === true)).toBe(true);
    // Final state unchanged: rule list still intact.
    expect(acl.check('api.gateway', 'db.read')).toBe(true);
    expect(acl.check('other', 'db.read')).toBe(false);
  });

  // acl_system.check.property.idempotent
  it('acl_system.check.property.idempotent: identical inputs yield identical decisions', () => {
    const acl = new ACL([rule(['api.*'], ['db.*'], 'allow')], 'deny');
    const first = acl.check('api.gateway', 'db.query');
    const second = acl.check('api.gateway', 'db.query');
    expect(first).toBe(true);
    expect(second).toBe(true);
    expect(acl.check('nope', 'db.query')).toBe(false);
    expect(acl.check('nope', 'db.query')).toBe(false);
  });

  // acl_system.check.property.pure
  it('acl_system.check.property.pure: pure:false but no self-mutation visible via public query', () => {
    const acl = new ACL(
      [rule(['api.*'], ['db.*'], 'allow'), rule(['*'], ['*'], 'deny')],
      'deny',
    );
    const beforeAllow = acl.check('api.gateway', 'db.read');
    const beforeDeny = acl.check('evil', 'secret');
    acl.check('api.gateway', 'db.read');
    acl.check('evil', 'secret');
    expect(acl.check('api.gateway', 'db.read')).toBe(beforeAllow);
    expect(acl.check('evil', 'secret')).toBe(beforeDeny);
  });

  // acl_system.check.side_effect.4.evaluate_first_match_wins
  it('acl_system.check.side_effect.4.evaluate_first_match_wins: first matching rule decides', () => {
    const acl = new ACL(
      [rule(['*'], ['*'], 'allow'), rule(['*'], ['*'], 'deny')],
      'deny',
    );
    expect(acl.check('anyone', 'anything')).toBe(true);
  });

  // acl_system.check.side_effect.5.emit_audit_event
  it('acl_system.check.side_effect.5.emit_audit_event: audit event emitted carrying decision', () => {
    const captured: AuditEntry[] = [];
    const acl = new ACL(
      [rule(['api.*'], ['db.*'], 'allow')],
      'deny',
      (entry) => captured.push(entry),
    );
    acl.check('api.gateway', 'db.read');
    expect(captured).toHaveLength(1);
    const entry = captured[0];
    expect(entry.decision).toBe('allow');
    expect(entry.callerId).toBe('api.gateway');
    expect(entry.targetId).toBe('db.read');
  });

  // acl_system.check.input.caller_id.none_maps_to_external
  it('acl_system.check.input.caller_id.none_maps_to_external: null caller => @external', () => {
    const acl = new ACL([rule(['@external'], ['public.*'], 'allow')], 'deny');
    expect(acl.check(null, 'public.docs')).toBe(true);
    // A real caller_id must NOT match @external.
    expect(acl.check('api.handler', 'public.docs')).toBe(false);
  });

  // acl_system.check.error.no_raise_returns_false
  it('acl_system.check.error.no_raise_returns_false: deny is a false return, never a throw', () => {
    const acl = new ACL([], 'deny');
    let result: boolean | undefined;
    expect(() => {
      result = acl.check('api.x', 'db.y');
    }).not.toThrow();
    expect(result).toBe(false);
  });
});

// ===========================================================================
// Contract: ACL.load
// ===========================================================================

describe('Contract: ACL.load', () => {
  // acl_system.load.input.yaml_path.file_must_exist
  it('acl_system.load.input.yaml_path.file_must_exist: missing file rejects with ConfigNotFoundError', () => {
    const dir = makeTmpDir();
    try {
      const missing = join(dir, 'does_not_exist.yaml');
      let err: unknown;
      try {
        ACL.load(missing);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ConfigNotFoundError);
      expect((err as ConfigNotFoundError).code).toBe('CONFIG_NOT_FOUND');
      // TS stores the path under details.configPath (camelCase), where Python
      // uses details["config_path"]. Assert the camelCase TS surface.
      expect((err as ConfigNotFoundError).details['configPath']).toBe(missing);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.error.CONFIG_NOT_FOUND
  it('acl_system.load.error.CONFIG_NOT_FOUND: nonexistent path => ConfigNotFoundError + code', () => {
    const dir = makeTmpDir();
    try {
      const missing = join(dir, 'nope.yaml');
      let err: unknown;
      try {
        ACL.load(missing);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ConfigNotFoundError);
      expect((err as ConfigNotFoundError).code).toBe('CONFIG_NOT_FOUND');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.error.ACL_RULE_ERROR.not_a_mapping
  it('acl_system.load.error.ACL_RULE_ERROR.not_a_mapping: top-level non-mapping => ACLRuleError', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, '- just\n- a\n- list\n');
      let err: unknown;
      try {
        ACL.load(path);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ACLRuleError);
      expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.error.ACL_RULE_ERROR.rules_key_absent
  it('acl_system.load.error.ACL_RULE_ERROR.rules_key_absent: missing rules => ACLRuleError', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, 'version: "1.0"\ndefault_effect: deny\n');
      let err: unknown;
      try {
        ACL.load(path);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ACLRuleError);
      expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.error.ACL_RULE_ERROR.rules_not_list
  it('acl_system.load.error.ACL_RULE_ERROR.rules_not_list: rules non-list => ACLRuleError', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, 'rules:\n  foo: bar\n');
      let err: unknown;
      try {
        ACL.load(path);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ACLRuleError);
      expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.error.ACL_RULE_ERROR.rule_missing_required_key
  it('acl_system.load.error.ACL_RULE_ERROR.rule_missing_required_key: rule missing effect => ACLRuleError', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, 'rules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n');
      let err: unknown;
      try {
        ACL.load(path);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ACLRuleError);
      expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.error.ACL_RULE_ERROR.invalid_effect
  it('acl_system.load.error.ACL_RULE_ERROR.invalid_effect: effect not allow/deny => ACLRuleError', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(
        dir,
        'rules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n    effect: maybe\n',
      );
      let err: unknown;
      try {
        ACL.load(path);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ACLRuleError);
      expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.error.ACL_RULE_ERROR.callers_not_list
  it('acl_system.load.error.ACL_RULE_ERROR.callers_not_list: callers non-list => ACLRuleError', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(
        dir,
        'rules:\n  - callers: "a.*"\n    targets: ["b.*"]\n    effect: allow\n',
      );
      let err: unknown;
      try {
        ACL.load(path);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ACLRuleError);
      expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.side_effect.4.set_yaml_path
  it('acl_system.load.side_effect.4.set_yaml_path: returned instance has yaml path wired (reload succeeds)', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const acl = ACL.load(path);
      // _yamlPath is private in TS; observe via reload() not throwing
      // "not loaded from YAML".
      expect(() => acl.reload()).not.toThrow();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.postcondition.default_effect_deny
  it('acl_system.load.postcondition.default_effect_deny: absent default_effect => deny', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(
        dir,
        'rules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n    effect: allow\n',
      );
      const acl = ACL.load(path);
      expect(acl.check('x', 'y')).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.postcondition.rules_order_preserved
  it('acl_system.load.postcondition.rules_order_preserved: rules keep YAML order (first-match-wins)', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(
        dir,
        'default_effect: deny\nrules:\n' +
          '  - callers: ["*"]\n    targets: ["*"]\n    effect: allow\n' +
          '  - callers: ["*"]\n    targets: ["*"]\n    effect: deny\n',
      );
      const acl = ACL.load(path);
      expect(acl.check('anyone', 'anything')).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.property.async
  it('acl_system.load.property.async: load() is declared async:false (returns ACL, not Promise)', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const result = ACL.load(path);
      expect(result).not.toBeInstanceOf(Promise);
      expect(result).toBeInstanceOf(ACL);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.property.idempotent
  it('acl_system.load.property.idempotent: same file content => equivalent instances', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const a = ACL.load(path);
      const b = ACL.load(path);
      expect(a.check('api.x', 'db.y')).toBe(true);
      expect(b.check('api.x', 'db.y')).toBe(true);
      expect(a.check('x', 'y')).toBe(false);
      expect(b.check('x', 'y')).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.load.property.thread_safe
  it('acl_system.load.property.thread_safe: N>=8 concurrent loads create independent instances', async () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const instances = await Promise.all(
        Array.from({ length: 8 }, () => Promise.resolve().then(() => ACL.load(path))),
      );
      expect(instances).toHaveLength(8);
      expect(instances.every((i) => i instanceof ACL)).toBe(true);
      // Distinct objects, no shared mutable state.
      expect(new Set(instances).size).toBe(8);
      expect(instances.every((i) => i.check('api.x', 'db.y') === true)).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

// ===========================================================================
// Contract: ACL.add_rule  (TS: addRule)
// ===========================================================================

describe('Contract: ACL.add_rule', () => {
  // acl_system.add_rule.side_effect.2.insert_at_index_0
  it('acl_system.add_rule.side_effect.2.insert_at_index_0: new rule evaluated before existing ones', () => {
    const acl = new ACL([rule(['*'], ['*'], 'deny')], 'deny');
    expect(acl.check('admin.root', 'anything')).toBe(false);
    acl.addRule(rule(['admin.*'], ['*'], 'allow'));
    expect(acl.check('admin.root', 'anything')).toBe(true);
  });

  // acl_system.add_rule.postcondition.shifts_prior_rules
  it('acl_system.add_rule.postcondition.shifts_prior_rules: prior rules shift up; new rule first', () => {
    const acl = new ACL([], 'deny');
    acl.addRule(rule(['*'], ['*'], 'deny'));
    acl.addRule(rule(['*'], ['*'], 'allow'));
    expect(acl.check('x', 'y')).toBe(true);
  });

  // acl_system.add_rule.property.idempotent
  it('acl_system.add_rule.property.idempotent: declared false — two identical calls add two rules', () => {
    const acl = new ACL([], 'deny');
    const r = rule(['a.*'], ['b.*'], 'allow');
    acl.addRule(r);
    acl.addRule(r);
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(true);
    expect(acl.check('a.x', 'b.y')).toBe(true); // second copy still present
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(true);
    expect(acl.check('a.x', 'b.y')).toBe(false); // now gone
  });

  // acl_system.add_rule.property.async
  it('acl_system.add_rule.property.async: addRule() is declared async:false, returns undefined/void', () => {
    const acl = new ACL([], 'deny');
    const result = acl.addRule(rule(['a.*'], ['b.*'], 'allow'));
    expect(result).toBeUndefined();
    expect(acl.check('a.x', 'b.y')).toBe(true);
  });

  // acl_system.add_rule.property.thread_safe
  it('acl_system.add_rule.property.thread_safe: N>=8 concurrent inserts, list not corrupted', async () => {
    const acl = new ACL([], 'deny');
    await Promise.all(
      Array.from({ length: 16 }, (_, i) =>
        Promise.resolve().then(() => acl.addRule(rule([`svc.${i}`], ['t.*'], 'allow'))),
      ),
    );
    for (let i = 0; i < 16; i++) {
      expect(acl.check(`svc.${i}`, 't.x')).toBe(true);
    }
    expect(acl.check('svc.absent', 't.x')).toBe(false);
  });

  // acl_system.add_rule.error.value_error_kwargs_path
  // Python-only kwargs overload `add_rule()` with no rule. TS addRule(rule)
  // requires a prebuilt rule argument — the kwargs path / ValueError does not
  // exist in the TS SDK (D10-006: prebuilt-rule form only). Missing symbol.
  it.skip('acl_system.add_rule.error.value_error_kwargs_path: missing symbol — TS addRule has no kwargs path (contract gap)', () => {
    // Intentionally skipped: no TS API surface to exercise.
  });
});

// ===========================================================================
// Contract: ACL.remove_rule  (TS: removeRule)
// ===========================================================================

describe('Contract: ACL.remove_rule', () => {
  // acl_system.remove_rule.side_effect.2.find_first_match
  it('acl_system.remove_rule.side_effect.2.find_first_match: removes by exact callers/targets equality', () => {
    const acl = new ACL(
      [rule(['a.*'], ['b.*'], 'allow'), rule(['c.*'], ['d.*'], 'allow')],
      'deny',
    );
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(true);
    expect(acl.check('a.x', 'b.y')).toBe(false);
    expect(acl.check('c.x', 'd.y')).toBe(true);
  });

  // acl_system.remove_rule.return.true_when_found
  it('acl_system.remove_rule.return.true_when_found: returns true when a matching rule removed', () => {
    const acl = new ACL([rule(['a.*'], ['b.*'], 'allow')], 'deny');
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(true);
  });

  // acl_system.remove_rule.return.false_when_absent
  it('acl_system.remove_rule.return.false_when_absent: returns false when no rule matches', () => {
    const acl = new ACL([rule(['a.*'], ['b.*'], 'allow')], 'deny');
    expect(acl.removeRule(['nope.*'], ['nope.*'])).toBe(false);
  });

  // acl_system.remove_rule.postcondition.at_most_one_removed
  it('acl_system.remove_rule.postcondition.at_most_one_removed: only first match removed per call', () => {
    const r = rule(['a.*'], ['b.*'], 'allow');
    const acl = new ACL([r, r], 'deny');
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(true);
    expect(acl.check('a.x', 'b.y')).toBe(true); // one duplicate remains
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(true);
    expect(acl.check('a.x', 'b.y')).toBe(false);
  });

  // acl_system.remove_rule.property.idempotent
  it('acl_system.remove_rule.property.idempotent: declared false — first true, second false', () => {
    const acl = new ACL([rule(['a.*'], ['b.*'], 'allow')], 'deny');
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(true);
    expect(acl.removeRule(['a.*'], ['b.*'])).toBe(false);
  });

  // acl_system.remove_rule.property.async
  it('acl_system.remove_rule.property.async: removeRule() is declared async:false (plain bool)', () => {
    const acl = new ACL([rule(['a.*'], ['b.*'], 'allow')], 'deny');
    const result = acl.removeRule(['a.*'], ['b.*']);
    expect(result).not.toBeInstanceOf(Promise);
    expect(result).toBe(true);
  });

  // acl_system.remove_rule.A-D-016.explicit_null_ignores_conditions
  it('acl_system.remove_rule.A-D-016: explicit null conditions ignores conditions when matching', () => {
    // A rule that HAS conditions; passing conditions: null must remove it,
    // treating explicit null the same as omitted (Python acl.py:631 / Rust).
    const acl = new ACL(
      [rule(['a'], ['b'], 'allow', '', { roles: ['admin'] })],
      'deny',
    );
    expect(acl.removeRule(['a'], ['b'], null)).toBe(true);
    expect(acl.removeRule(['a'], ['b'], null)).toBe(false);
  });

  // acl_system.remove_rule.A-D-016.specific_conditions_still_disambiguate
  it('acl_system.remove_rule.A-D-016: a specific conditions object still disambiguates', () => {
    const acl = new ACL(
      [
        rule(['a'], ['b'], 'allow', '', { roles: ['admin'] }),
        rule(['a'], ['b'], 'allow', '', { roles: ['user'] }),
      ],
      'deny',
    );
    // Targeted removal: only the user-role rule is removed.
    expect(acl.removeRule(['a'], ['b'], { roles: ['user'] })).toBe(true);
    expect(acl.removeRule(['a'], ['b'], { roles: ['user'] })).toBe(false);
    // The admin-role rule survives.
    expect(acl.removeRule(['a'], ['b'], { roles: ['admin'] })).toBe(true);
  });

  // acl_system.remove_rule.property.thread_safe
  it('acl_system.remove_rule.property.thread_safe: N>=8 concurrent removals, no corruption', async () => {
    const rules = Array.from({ length: 16 }, (_, i) => rule([`svc.${i}`], ['t.*'], 'allow'));
    const acl = new ACL(rules, 'deny');
    const results = await Promise.all(
      Array.from({ length: 16 }, (_, i) =>
        Promise.resolve().then(() => acl.removeRule([`svc.${i}`], ['t.*'])),
      ),
    );
    expect(results.every((r) => r === true)).toBe(true);
    for (let i = 0; i < 16; i++) {
      expect(acl.check(`svc.${i}`, 't.x')).toBe(false);
    }
  });
});

// ===========================================================================
// Contract: ACL.reload
// ===========================================================================

describe('Contract: ACL.reload', () => {
  // acl_system.reload.precondition.requires_yaml_path
  it('acl_system.reload.precondition.requires_yaml_path: reload on non-loaded ACL => ACLRuleError', () => {
    const acl = new ACL([], 'deny'); // not created via load()
    let err: unknown;
    try {
      acl.reload();
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(ACLRuleError);
    expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
  });

  // acl_system.reload.error.ACL_RULE_ERROR.not_loaded_from_yaml
  it('acl_system.reload.error.ACL_RULE_ERROR.not_loaded_from_yaml: no stored path => ACLRuleError + code', () => {
    const acl = new ACL([rule(['*'], ['*'], 'allow')], 'deny');
    let err: unknown;
    try {
      acl.reload();
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(ACLRuleError);
    expect((err as ACLRuleError).code).toBe('ACL_RULE_ERROR');
  });

  // acl_system.reload.error.CONFIG_NOT_FOUND.file_removed
  it('acl_system.reload.error.CONFIG_NOT_FOUND.file_removed: file deleted after load => ConfigNotFoundError', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const acl = ACL.load(path);
      unlinkSync(path);
      let err: unknown;
      try {
        acl.reload();
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(ConfigNotFoundError);
      expect((err as ConfigNotFoundError).code).toBe('CONFIG_NOT_FOUND');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.reload.postcondition.rules_reflect_file
  it('acl_system.reload.postcondition.rules_reflect_file: reload re-reads YAML and updates rules', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(
        dir,
        'default_effect: deny\nrules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n    effect: allow\n',
      );
      const acl = ACL.load(path);
      expect(acl.check('a.x', 'b.y')).toBe(true);
      // Rewrite the file with a different ruleset.
      writeYaml(
        dir,
        'default_effect: deny\nrules:\n  - callers: ["c.*"]\n    targets: ["d.*"]\n    effect: allow\n',
      );
      acl.reload();
      expect(acl.check('a.x', 'b.y')).toBe(false); // old rule gone
      expect(acl.check('c.x', 'd.y')).toBe(true); // new rule active
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.reload.postcondition.discards_runtime_mutations
  it('acl_system.reload.postcondition.discards_runtime_mutations: addRule before reload is discarded', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const acl = ACL.load(path);
      acl.addRule(rule(['runtime.*'], ['*'], 'allow'));
      expect(acl.check('runtime.x', 'anything')).toBe(true);
      acl.reload();
      expect(acl.check('runtime.x', 'anything')).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.reload.property.async
  it('acl_system.reload.property.async: reload() is declared async:false, returns undefined/void', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const acl = ACL.load(path);
      const result = acl.reload();
      expect(result).toBeUndefined();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.reload.property.idempotent
  it('acl_system.reload.property.idempotent: same file content => same rule list across reloads', () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const acl = ACL.load(path);
      acl.reload();
      const firstAllow = acl.check('api.x', 'db.y');
      const firstDeny = acl.check('x', 'y');
      acl.reload();
      expect(acl.check('api.x', 'db.y')).toBe(firstAllow);
      expect(firstAllow).toBe(true);
      expect(acl.check('x', 'y')).toBe(firstDeny);
      expect(firstDeny).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // acl_system.reload.property.thread_safe
  it('acl_system.reload.property.thread_safe: concurrent reload + check, no corruption', async () => {
    const dir = makeTmpDir();
    try {
      const path = writeYaml(dir, VALID_YAML);
      const acl = ACL.load(path);
      const tasks: Promise<unknown>[] = [];
      for (let i = 0; i < 8; i++) {
        tasks.push(Promise.resolve().then(() => acl.reload()));
        tasks.push(Promise.resolve().then(() => acl.check('api.x', 'db.y')));
      }
      const results = await Promise.all(tasks);
      expect(results).toHaveLength(16);
      // Final state consistent: the file's allow rule still applies.
      expect(acl.check('api.x', 'db.y')).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
