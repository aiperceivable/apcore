/**
 * Cross-language driver for `acl_pattern_arity.json`.
 *
 * PROTOCOL_SPEC §6.2.1 with §6.1.4.1 and §6.1.1 (spec v1.31.0, #112): a
 * `callers` / `targets` pattern array's ARITY is closed. At least one operand,
 * every element a non-empty string, `$or` at index 0 followed by at least one
 * pattern, `$not` by exactly one, and neither token anywhere but index 0 —
 * because a pattern array is FLAT, unlike the same operators in `conditions`.
 *
 * **Two tiers, and they are not the same mechanism.** TIER 1 is structural and
 * is rejected with `ACLRuleError` at every door §6.1.6 rule 3 names — file
 * loading, direct construction and runtime insertion — so `expected_load: "ok"`
 * means accepted at every listed door and `"reject"` means rejected at every
 * one. There is deliberately no per-door expectation, because a shape legal
 * through one door and illegal through another IS the defect. TIER 2 is
 * semantic — an array well-formed under every tier-1 rule that still matches no
 * legal module ID, `["$not", "*"]` being the sharpest example — and it loads,
 * is reported by `validateRules()`, and changes no decision.
 *
 * **The backstop** (`kind: "backstop"`) is the one route no door covers:
 * assigning the field on an already-constructed rule. `ACLRule` is a plain
 * interface with mutable properties, so this SDK has that route and the cases
 * are exercised rather than satisfied by construction. Such a value is a
 * §6.1.4.1 precheck fault — the rule is UNEVALUABLE and §6.1.1's effect table
 * decides.
 *
 * **Two decision surfaces, and they can diverge.** `expected_access` binds to
 * the structured `checkAccess().access` STRING; `expected_legacy_check` binds to
 * the boolean `check()`. They differ on
 * `mutated_empty_targets_on_approval_rule_raises_pending_requirement`, where
 * access is `allow` with `approvalRequired: true` and §6.8.1 makes the boolean
 * fail closed — a driver reading the boolean as if it were `access` fails that
 * case alone and passes every other one, which is why they are asserted apart.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { ACL } from '../src/acl.js';
import type { ACLRule, AuditEntry } from '../src/acl.js';
import { ACLRuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';
// Side-effect import installs the Node-side YAML loader onto ACL.load.
import '../src/acl-file.js';

const FIXTURE = 'acl_pattern_arity.json';
const FIXTURE_PATH = path.join(findFixturesRoot(), FIXTURE);
const PRESENT = fs.existsSync(FIXTURE_PATH);

// A driver lands one push before its fixture so `check_driver_coverage.py
// --strict` has a driver to find. Until the fixture is beside it the suite
// skips and names the unexercised fixture — "not verified", never "passed".
const describeIfPresent = PRESENT ? describe : describe.skip;

/** The three doors §6.1.6 rule 3 names. This SDK exposes all three. */
const DOORS = ['load', 'construct', 'add_rule'] as const;
type Door = (typeof DOORS)[number];

type PatternField = 'callers' | 'targets';

interface Mutation {
  field: PatternField;
  value: string[];
}

interface Case {
  id: string;
  kind: 'closure' | 'backstop';
  note: string;
  /** Exactly one of `rule` / `rules`; see {@link rulesOf}. */
  rule?: Record<string, unknown>;
  /**
   * An ORDERED rule set, for the cross-rule half of §6.2.1 point 2. Offered at
   * the `load` and `construct` doors only — `addRule()` takes one rule at a
   * time and cannot express which of several bad rules is refused.
   */
  rules?: Record<string, unknown>[];
  default_effect: string;
  /** `closure` only — the doors the rule is offered at. */
  entry_points?: string[];
  expected_load?: 'ok' | 'reject';
  /**
   * Which AXIS the refusal names (§6.2.1 point 2) — `expected_load` cannot see
   * it, since every one of these cases is bad on more than one axis and would
   * read as `reject` whichever fault an implementation happened to name.
   *
   * The four values mix two levels deliberately: `effect` and `approval` name
   * axes, `callers` and `targets` name a FIELD within the single pattern axis,
   * so either of the last two also asserts that the pattern axis is the one
   * that fired.
   */
  expected_refused_axis?: string;
  /**
   * Which RULE the refusal names. The index chooses the rule and the axis order
   * then chooses the fault inside it, so a driver asserting only the axis
   * passes a rule set refused for the wrong rule on the right axis.
   */
  expected_refused_rule_index?: number;
  /** `backstop` only — one mutation or a list, applied in order. */
  mutate?: Mutation | Mutation[];
  mutation_route?: string;
  caller_id?: string | null;
  target_id?: string;
  /** The STRING from the structured accessor, never the boolean. */
  expected_access?: 'allow' | 'deny';
  /** The boolean from the legacy `check()`, which fails closed (§6.8.1). */
  expected_legacy_check?: boolean;
  expected_approval_required?: boolean;
  expected_matched_rule_index?: number | null;
  expected_audit_handler_error_present?: boolean;
  expected_handler_error_paths?: string[];
  /** Present on both kinds: tier 2 on a `closure`, §6.1.3 rule 3 on a backstop. */
  expected_validation_finding_paths?: string[];
}

const fixture = PRESENT
  ? (JSON.parse(fs.readFileSync(FIXTURE_PATH, 'utf-8')) as { test_cases: Case[] })
  : { test_cases: [] as Case[] };

/**
 * Build the rule the case names.
 *
 * A fresh object per door: the mutation cases assign onto the rule, and a
 * shared one would carry a previous door's value into the next.
 */
function toRule(spec: Record<string, unknown>): ACLRule {
  return {
    callers: [...(spec['callers'] as string[])],
    targets: [...(spec['targets'] as string[])],
    effect: spec['effect'] as string,
    description: (spec['description'] as string) ?? '',
    conditions: null,
    ...(spec['approval'] === undefined ? {} : { approval: spec['approval'] as never }),
  };
}

/**
 * The case's rules in order, whichever shape it carries.
 *
 * A case declares either `rule` (one) or `rules` (an ordered list); the list
 * form exists for the cross-rule half of §6.2.1 point 2, which one rule cannot
 * express. Everything downstream reads this and never the raw fields, so the
 * two shapes travel the same path through every door.
 */
function rulesOf(tc: Case): Record<string, unknown>[] {
  if (tc.rules !== undefined) return tc.rules;
  if (tc.rule !== undefined) return [tc.rule];
  throw new Error(
    `conformance driver: case '${tc.id}' carries neither 'rule' nor 'rules'. ` +
      'Teach the driver, do not skip it.',
  );
}

/**
 * Write the rules as an ACL file, VERBATIM.
 *
 * YAML is a superset of JSON, so each rule is emitted as a flow mapping — which
 * keeps an empty array an empty array rather than something a hand-written
 * block style might round-trip differently. The spec is written as the fixture
 * states it and never through {@link toRule}: a typed rule silently drops a key
 * outside the closed set, so `lowest_indexed_bad_rule_wins_over_a_loader_only_axis`
 * would lose the `priority` that makes rule 1 bad and pass for the wrong reason.
 * `default_effect` is written as given too, since one case declares an illegal
 * one.
 */
function writeAclFile(specs: readonly Record<string, unknown>[], defaultEffect: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'acl-arity-conf-'));
  const file = path.join(dir, 'acl.yaml');
  fs.writeFileSync(
    file,
    `default_effect: ${JSON.stringify(defaultEffect)}\nrules:\n` +
      specs.map((spec) => `  - ${JSON.stringify(spec)}\n`).join(''),
    'utf-8',
  );
  return file;
}

/**
 * The rule keys a typed `ACLRule` can carry (§6.1's closed set).
 *
 * A spec carrying anything else is representable in a file and not in the type,
 * so offering it at a typed door would drop the key and test something the
 * fixture did not write.
 */
const TYPED_RULE_KEYS: ReadonlySet<string> = new Set([
  'callers',
  'targets',
  'effect',
  'description',
  'conditions',
  'approval',
]);

/**
 * Assert a spec is representable in a typed rule before a typed door takes it.
 *
 * Without this, a case whose rule carries a key outside §6.1's closed set is
 * silently stripped by {@link toRule} and the door accepts a rule the fixture
 * never wrote — a false pass that looks like conformance.
 */
function assertTypedRepresentable(caseId: string, door: Door, spec: Record<string, unknown>): void {
  const outside = Object.keys(spec).filter((k) => !TYPED_RULE_KEYS.has(k));
  expect(
    outside,
    `case '${caseId}' offers a rule carrying ${outside.join(', ')} at the ${door} door, ` +
      'where a typed rule cannot represent it. Teach the driver, do not drop the key.',
  ).toEqual([]);
}

/** Push the case's rules through one door, returning the resulting ACL. */
function openDoor(door: Door, tc: Case): ACL {
  const specs = rulesOf(tc);
  switch (door) {
    case 'load':
      return ACL.load(writeAclFile(specs, tc.default_effect));
    case 'construct':
      for (const spec of specs) assertTypedRepresentable(tc.id, door, spec);
      return new ACL(specs.map(toRule), tc.default_effect);
    case 'add_rule': {
      // `addRule` takes one rule at a time, so the fixture never lists this
      // door on a `rules` case — asserted rather than assumed, since silently
      // inserting only the first rule would make a multi-rule case pass here
      // for the wrong reason.
      expect(
        specs.length,
        `case '${tc.id}' lists the add_rule door with ${specs.length} rules`,
      ).toBe(1);
      assertTypedRepresentable(tc.id, door, specs[0]);
      const acl = new ACL([], tc.default_effect);
      acl.addRule(toRule(specs[0]));
      return acl;
    }
  }
}

/**
 * Every §6.1.4 path named by a `handlerError`, in the order the message lists
 * them — the same reading `tests/conformance-acl-handler-error.test.ts` uses.
 * Each `"; "`-separated part begins with its path in single quotes.
 */
function handlerErrorPaths(message: string | null): string[] {
  if (message === null) return [];
  return message.split('; ').map((part) => {
    const match = /'([^']*)'/.exec(part);
    return match === null ? part : match[1];
  });
}

/**
 * Assert that a refusal names the axis the case declares (§6.2.1 point 2).
 *
 * The axis names are the fixture's vocabulary, not this SDK's message text, so
 * each is matched against the message this implementation raises for that axis.
 * An axis the driver does not recognise is a hard failure naming the case and
 * the unknown value — teach the driver, never skip the assertion, or a fixture
 * that grows a fourth axis reads as covered when nothing checks it.
 */
function assertRefusedAxis(caseId: string, axis: string, message: string): void {
  switch (axis) {
    case 'default_effect':
      // Not a rule and carrying no index, so §6.2.1's rule ordering does not
      // reach it; it is judged first instead. `Rule N` must not appear at all —
      // its presence would mean a rule was judged ahead of the file's own
      // effect, which is the divergence this axis was added to close.
      expect(message, `${caseId}: refusal did not name default_effect: ${message}`).toMatch(
        /Invalid default_effect/,
      );
      // No rule index, in ANY spelling — its presence would mean a rule was
      // judged ahead of the file's own effect. Asserted as an empty set rather
      // than as a failed match, so a wording this driver does not recognise
      // cannot read as "no index present".
      expect(
        ruleIndicesNamed(message),
        `${caseId}: a rule was judged before default_effect: ${message}`,
      ).toEqual([]);
      return;
    case 'effect':
      expect(message, `${caseId}: refusal did not name the effect: ${message}`).toMatch(
        /invalid effect/,
      );
      expect(message, `${caseId}: refusal named a later axis: ${message}`).not.toMatch(
        /pattern-array shape|must be a list|approval: 'required'/,
      );
      return;
    case 'approval':
      expect(message, `${caseId}: refusal did not name the approval pair: ${message}`).toMatch(
        /approval: 'required' on an effect: 'deny' rule/,
      );
      expect(message, `${caseId}: refusal named a later axis: ${message}`).not.toMatch(
        /pattern-array shape|must be a list/,
      );
      return;
    case 'callers':
    case 'targets': {
      const other = axis === 'callers' ? 'targets' : 'callers';
      expect(message, `${caseId}: refusal did not name '${axis}': ${message}`).toMatch(
        new RegExp(`'${axis}' (has an illegal pattern-array shape|must be a list)`),
      );
      expect(message, `${caseId}: refusal named '${other}' instead: ${message}`).not.toContain(
        `'${other}'`,
      );
      return;
    }
    default:
      throw new Error(
        `conformance driver: case '${caseId}' declares expected_refused_axis ` +
          `'${axis}', which the driver does not recognise. Teach the driver, do not skip it.`,
      );
  }
}

/**
 * Every rule index a refusal names, in any spelling a door might use.
 *
 * Deliberately case-insensitive and tolerant of a prefix: the axis families are
 * raised from different code paths and need not word the index identically —
 * this SDK's per-rule validator says `Rule 0 has invalid effect`, while another
 * implementation's loader-only key axis says `ACL rule 1 in '<file>' carries
 * 'priority' unrecognised`. A matcher pinned to one spelling reads the other as
 * naming NO rule, which is a silent pass on exactly the case built to catch a
 * real bug (measured in apcore-rust on
 * `lowest_indexed_bad_rule_wins_over_a_loader_only_axis`).
 *
 * A digit must follow the word, so the prose in this SDK's own pattern-shape
 * message — "makes the rule inert", "an inert deny rule under…" — is not a hit.
 */
function ruleIndicesNamed(message: string): number[] {
  return [...message.matchAll(/\brules?\s+(\d+)\b/gi)].map((m) => Number(m[1]));
}

/**
 * Assert that a refusal names the rule the case declares, and only that rule
 * (§6.2.1 point 2).
 *
 * The index chooses the rule and the axis order then chooses the fault inside
 * it, so a driver asserting only the axis passes a rule set refused for the
 * WRONG rule on the right axis — the shape of the loader/constructor divergence
 * these cases were written for. Asserting the whole set rather than the first
 * match also catches a message naming no rule at all and one naming two.
 */
function assertRefusedRuleIndex(caseId: string, index: number, message: string): void {
  const named = new Set(ruleIndicesNamed(message));
  expect(
    [...named],
    `${caseId}: refusal should name rule ${index} and no other: ${message}`,
  ).toEqual([index]);
}

/** The mutations a backstop case declares, as a list whatever its shape. */
function mutations(tc: Case): Mutation[] {
  if (tc.mutate === undefined) return [];
  return Array.isArray(tc.mutate) ? tc.mutate : [tc.mutate];
}

describeIfPresent("Conformance: a pattern array's arity is closed (§6.2.1, spec v1.31.0)", () => {
  it('names only entry points this driver actually exercises', () => {
    // A fixture door left unimplemented would silently narrow the contract to
    // the doors that happen to be covered — the shape of the defect itself.
    const named = new Set(
      fixture.test_cases.filter((tc) => tc.kind === 'closure').flatMap((tc) => tc.entry_points ?? []),
    );
    for (const door of named) {
      expect(DOORS as readonly string[], `fixture names an unexercised door '${door}'`).toContain(
        door,
      );
    }
  });

  it('reads a rule index in every spelling a door might use', () => {
    // The driver's own guard. The axis families are raised from different code
    // paths and need not word the index the same way; a matcher pinned to one
    // spelling reads the others as naming NO rule, which is a silent pass on
    // the very cases that pin the ordering. This is the shape that bit
    // apcore-rust, whose loader-only key axis says `ACL rule 1 in '<file>'`.
    expect(ruleIndicesNamed("Rule 0 has invalid effect 'Allow'")).toEqual([0]);
    expect(ruleIndicesNamed("ACL rule 1 in '/tmp/acl.yaml' carries 'priority'")).toEqual([1]);
    expect(ruleIndicesNamed('rules 2 and Rule 3')).toEqual([2, 3]);
    expect(ruleIndicesNamed("Invalid default_effect 'Allow', must be 'allow' or 'deny'")).toEqual(
      [],
    );
    // Prose about "the rule" is not an index, or every pattern-shape message in
    // this SDK — "makes the rule inert" — would read as naming one.
    expect(ruleIndicesNamed('a shape that can never match makes the rule inert')).toEqual([]);
  });

  it('exercises every case the fixture declares', () => {
    // "N skipped" and "N satisfied by construction" are different claims. This
    // SDK reaches every door and the installed-rule mutation route, so the
    // count below is the count the fixture carries, with nothing elided.
    expect(fixture.test_cases.length).toBeGreaterThan(0);
    for (const tc of fixture.test_cases) {
      expect(['closure', 'backstop'], `case '${tc.id}' declares an unknown kind`).toContain(tc.kind);
      // Exactly one of the two case shapes, never both and never neither.
      expect(
        [tc.rule, tc.rules].filter((v) => v !== undefined).length,
        `case '${tc.id}' must carry exactly one of 'rule' / 'rules'`,
      ).toBe(1);
      // `addRule` takes one rule at a time, so a rule SET has no per-rule door.
      if (tc.rules !== undefined) {
        expect(
          tc.entry_points ?? [],
          `case '${tc.id}' offers a rule set at the add_rule door`,
        ).not.toContain('add_rule');
      }
      // An order expectation on a case that is not refused would never be
      // asserted — it would read as covered while nothing checked it.
      for (const key of ['expected_refused_axis', 'expected_refused_rule_index'] as const) {
        if (tc[key] !== undefined) {
          expect(
            tc.expected_load,
            `case '${tc.id}' declares ${key} but is not a reject case`,
          ).toBe('reject');
        }
      }
    }
  });

  fixture.test_cases.forEach((tc) => {
    it(tc.id, () => {
      if (tc.kind === 'closure') {
        const doors = (tc.entry_points ?? []) as Door[];
        expect(doors.length, `${tc.id} lists no entry point`).toBeGreaterThan(0);
        expect(
          ['ok', 'reject'],
          `case '${tc.id}' states an expected_load the driver does not recognise ` +
            `(${JSON.stringify(tc.expected_load)}). Teach the driver, do not skip it.`,
        ).toContain(tc.expected_load);

        for (const door of doors) {
          if (tc.expected_load === 'reject') {
            // §6.1.6 rule 3 — rejected at EVERY door, with the typed error.
            expect(() => openDoor(door, tc), `${door}: ${tc.note}`).toThrow(ACLRuleError);
            // §6.2.1 point 2 — and for the same axis at every door. A rule bad
            // on several axes is refused for the first one it fails, so which
            // fault the message names is the assertion; `expected_load` alone
            // reads as covered whichever axis an implementation happened to
            // check first, which is how three SDKs answered this three ways.
            if (tc.expected_refused_axis !== undefined) {
              let message = '';
              try {
                openDoor(door, tc);
              } catch (e) {
                message = e instanceof Error ? e.message : String(e);
              }
              assertRefusedAxis(`${tc.id} (${door})`, tc.expected_refused_axis, message);
              // ...and for the same RULE. Index dominates the axes, so a rule
              // set can be refused on the right axis for the wrong rule — which
              // is precisely what an implementation sweeping one axis across the
              // whole list does, and what the axis assertion alone would miss.
              if (tc.expected_refused_rule_index !== undefined) {
                assertRefusedRuleIndex(
                  `${tc.id} (${door})`,
                  tc.expected_refused_rule_index,
                  message,
                );
              }
            }
            // A throw leaves the rule list untouched, so a caller that
            // swallowed it cannot end up enforcing the rule anyway.
            if (door === 'add_rule') {
              const acl = new ACL([], tc.default_effect);
              expect(() => acl.addRule(toRule(rulesOf(tc)[0]))).toThrow(ACLRuleError);
              expect(acl.rules.length, `${door}: rejected rule was still inserted`).toBe(0);
            }
            continue;
          }

          const acl = openDoor(door, tc);
          const specs = rulesOf(tc);
          expect(acl.rules.length, `${door}: ${tc.note}`).toBe(specs.length);
          specs.forEach((spec, i) => {
            expect(acl.rules[i].callers, `${door}: rule ${i}: ${tc.note}`).toEqual(spec['callers']);
            expect(acl.rules[i].targets, `${door}: rule ${i}: ${tc.note}`).toEqual(spec['targets']);
          });

          // A closure case carrying finding paths is TIER 2: it MUST load, and
          // `validateRules()` MUST then report exactly those paths — reported,
          // never rejected, which is what keeps the two tiers distinct.
          if (tc.expected_validation_finding_paths !== undefined) {
            const findings = acl.validateRules();
            expect(
              findings.map((f) => f.conditionPath),
              `${door}: ${tc.note}`,
            ).toEqual(tc.expected_validation_finding_paths);
            for (const finding of findings) {
              // §6.1.3 rule 3's keyless structural fault.
              expect(finding.conditionKey).toBeNull();
              expect(finding.syncResolvable).toBe(false);
              expect(finding.asyncResolvable).toBe(false);
            }
          }
        }
        return;
      }

      // --- kind: "backstop" -------------------------------------------------
      //
      // Build with the WELL-FORMED value, then assign the illegal one onto the
      // already-constructed rule. Every constructor is bypassed, and unlike an
      // unrecognised `effect` — never read again once the doors are closed —
      // the matcher WILL consult this on the next check().
      expect(
        tc.mutation_route ?? 'installed_rule',
        `case '${tc.id}' names a mutation route this driver does not implement`,
      ).toBe('installed_rule');

      const backstopSpecs = rulesOf(tc);
      expect(
        backstopSpecs.length,
        `case '${tc.id}' is a backstop with ${backstopSpecs.length} rules; the mutation and the ` +
          'decision are both stated about one rule',
      ).toBe(1);
      const rule = toRule(backstopSpecs[0]);
      const entries: AuditEntry[] = [];
      const acl = new ACL([rule], tc.default_effect, (e) => entries.push(e));
      for (const m of mutations(tc)) rule[m.field] = [...m.value];
      // The mutation reached the INSTALLED rule, not a copy of it — without
      // this the backstop cases would pass against an ACL that snapshotted.
      for (const m of mutations(tc)) {
        expect(acl.rules[0][m.field], `${tc.id}: mutation did not reach the installed rule`).toEqual(
          m.value,
        );
      }

      const callerId = tc.caller_id ?? null;
      const targetId = tc.target_id as string;

      // §6.1.1 rule 4 / Contract: ACL.check — nothing may raise out of check().
      let decision: ReturnType<ACL['checkAccess']> | undefined;
      expect(() => {
        decision = acl.checkAccess(callerId, targetId);
      }).not.toThrow();
      expect(decision?.access, `${tc.id}: ${tc.note}`).toBe(tc.expected_access);
      if (tc.expected_approval_required !== undefined) {
        expect(decision?.approvalRequired, `${tc.id}: ${tc.note}`).toBe(
          tc.expected_approval_required,
        );
      }
      if ('expected_matched_rule_index' in tc) {
        expect(decision?.matchedRuleIndex, `${tc.id}: ${tc.note}`).toBe(
          tc.expected_matched_rule_index,
        );
      }

      // The legacy boolean is a SEPARATE surface, not a restatement of
      // `access`: §6.8.1 makes it fail closed on an approval requirement, so
      // `allow` + `approvalRequired` reads `false` here and `"allow"` above.
      expect(acl.check(callerId, targetId), `${tc.id}: legacy check(): ${tc.note}`).toBe(
        tc.expected_legacy_check,
      );

      // Every check() emits exactly one audit entry (§6.3.1), and both calls
      // above describe the same fault, so both entries are asserted.
      expect(entries.length).toBe(2);
      for (const entry of entries) {
        if (tc.expected_audit_handler_error_present) {
          expect(entry.handlerError, `${tc.id}: ${tc.note}`).not.toBeNull();
        } else {
          expect(entry.handlerError, `${tc.id}: ${tc.note}`).toBeNull();
        }
        expect(handlerErrorPaths(entry.handlerError), `${tc.id}: ${tc.note}`).toEqual(
          tc.expected_handler_error_paths ?? [],
        );
      }

      const findings = acl.validateRules();
      expect(
        findings.map((f) => f.conditionPath),
        `${tc.id}: ${tc.note}`,
      ).toEqual(tc.expected_validation_finding_paths ?? []);
      for (const finding of findings) {
        // §6.1.3 rule 3 — a keyless structural fault, both flags false.
        expect(finding.conditionKey).toBeNull();
        expect(finding.syncResolvable).toBe(false);
        expect(finding.asyncResolvable).toBe(false);
      }
    });
  });
});
