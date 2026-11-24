/**
 * Cross-language driver for `acl_argument_scoped_approval.json`.
 *
 * PROTOCOL_SPEC §6.1.1 / §6.1.6 / §6.1.7 / §6.1.8 / §6.8.1 / §6.9
 * (spec v1.28.0 apcore#108, extended v1.29.0 apcore#109).
 *
 * An ACL rule answers two independent questions — may this caller reach this
 * target at all, and must *this particular call* be put to a human first. The
 * orthogonal `approval` field carries the second, and the built-in
 * structure-only `arguments` condition decides whether a rule matches this call.
 *
 * **Every case runs twice.** The `arguments` condition can only be answered
 * when a governance projection is available, and §6.1.8 case 1 makes `check()`
 * a public entry point that may be called without one — so the same rules and
 * the same call have two well-defined answers and both are contracts. Run 1
 * supplies a projection derived from `arguments`; run 2 supplies none at all.
 * Unsuffixed expectation keys belong to run 1, `*_no_projection` keys to run 2.
 * This SDK can hand a projection to `check()` as well as to `checkAccess()`, so
 * it asserts all five keys in both columns — the driver-side skip the fixture
 * description retires was hiding #109 in exactly the unasserted column.
 *
 * The two cases worth reading before the rest are
 * `no_projection_must_not_grant_via_an_empty_stand_in` and
 * `no_projection_makes_a_deny_rule_take_effect`: they bracket the same
 * fail-open bug from both directions. Substituting an empty key set for an
 * absent projection makes `has_none_of` vacuously satisfied, so an `allow` rule
 * grants for a call whose arguments were never seen — and leaves `has_key`
 * unsatisfied, so a `deny` rule fails to take effect. Only the UNEVALUABLE
 * reading of §6.1.8 rule 1 refuses in both directions.
 *
 * `an_out_of_scope_approval_rule_raises_nothing` is the containment guard for
 * §6.1.1 rule 5: an implementation that raises the pending requirement before
 * matching the rule's patterns passes every other case here and fails that one.
 *
 * This SDK carries the projection in `AccessCheckOptions` rather than on
 * `Context`. §6.1.8 rule 4 leaves that route idiomatic and unconstrained; what
 * it fixes is that the condition sees the projection and that a caller cannot
 * forge one.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { ACL, type ACLRule, type AuditEntry, type GovernanceProjection } from '../src/acl.js';
import { buildGovernanceProjection } from '../src/acl-handlers.js';
import { ACLRuleError } from '../src/errors.js';
import { Context, Identity } from '../src/context.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURE = 'acl_argument_scoped_approval.json';
const FIXTURE_PATH = path.join(findFixturesRoot(), FIXTURE);
const PRESENT = fs.existsSync(FIXTURE_PATH);

// The fixture lands in the spec repo one push after this driver, so that
// `check_driver_coverage.py --strict` has a driver to find for it. Until then
// the suite skips and names the unexercised fixture — "not verified", never
// "passed".
const describeIfPresent = PRESENT ? describe : describe.skip;

interface Case {
  id: string;
  note: string;
  default_effect: string;
  rules: Array<Record<string, unknown>>;
  caller_id: string;
  target_id: string;
  arguments: Record<string, unknown> | null;
  /**
   * Set where a rule carries `callers_raw` / `targets_raw` — a deliberately malformed pattern
   * field a statically typed SDK may not be able to express. TypeScript CAN
   * express it (a cast reaches the same shape a JS caller reaches by accident),
   * so this driver never skips: §6.1.4.1 exists precisely for the construction
   * paths a YAML load cannot police.
   */
  skip_if_unrepresentable?: boolean;
  expected_access: 'allow' | 'deny';
  expected_approval_required: boolean;
  expected_legacy_check: boolean;
  expected_matched_rule_index: number | null;
  expected_audit_handler_error_present: boolean;
  /** Run 2's column, added by spec v1.29.0; absent in the fixture's 20-case shape. */
  expected_access_no_projection?: 'allow' | 'deny';
  expected_approval_required_no_projection?: boolean;
  expected_legacy_check_no_projection?: boolean;
  expected_matched_rule_index_no_projection?: number | null;
  expected_audit_handler_error_present_no_projection?: boolean;
  expected_validation_finding_path: string | null;
  /** Added by spec v1.28.0's follow-up; absent in the fixture's first shape. */
  expected_validation_finding_paths?: string[] | null;
}

const cases: Case[] = PRESENT
  ? (JSON.parse(fs.readFileSync(FIXTURE_PATH, 'utf-8')) as { test_cases: Case[] }).test_cases
  : [];

/** One run's five expectations — the fixture's two columns share this shape. */
interface Column {
  readonly access: 'allow' | 'deny';
  readonly approvalRequired: boolean;
  readonly matchedRuleIndex: number | null;
  readonly legacyCheck: boolean;
  readonly handlerErrorPresent: boolean;
}

function withProjectionColumn(tc: Case): Column {
  return {
    access: tc.expected_access,
    approvalRequired: tc.expected_approval_required,
    matchedRuleIndex: tc.expected_matched_rule_index,
    legacyCheck: tc.expected_legacy_check,
    handlerErrorPresent: tc.expected_audit_handler_error_present,
  };
}

/**
 * Run 2's column, or `null` on a fixture that predates it.
 *
 * Drivers land one push BEFORE the fixture in this project, so a driver must
 * tolerate the shape that predates the keys it reads — an absent key arrives as
 * `undefined`, which a strict comparison would happily assert against. Once the
 * fixture lands, every case carries the column and every case asserts it.
 */
function noProjectionColumn(tc: Case): Column | null {
  if (tc.expected_access_no_projection === undefined) return null;
  return {
    access: tc.expected_access_no_projection,
    approvalRequired: tc.expected_approval_required_no_projection as boolean,
    matchedRuleIndex: (tc.expected_matched_rule_index_no_projection ?? null) as number | null,
    legacyCheck: tc.expected_legacy_check_no_projection as boolean,
    handlerErrorPresent: tc.expected_audit_handler_error_present_no_projection as boolean,
  };
}

function build(tc: Case): { acl: ACL; entries: AuditEntry[] } {
  const entries: AuditEntry[] = [];
  const rules = tc.rules.map(
    (r) =>
      ({
        // `callers_raw` / `targets_raw` carry a value that is not a list of
        // strings. The cast is the point of those cases: §6.1.4.1 must classify
        // it as unevaluable rather than iterate the string character by
        // character, where a `*` would match every caller. Both fields are
        // honoured because §6.1.1 rule 5's malformed-scope clause names both,
        // and reading only one leaves the other half of the clause unverified.
        callers: (r.callers_raw !== undefined ? r.callers_raw : r.callers) as string[],
        targets: (r.targets_raw !== undefined ? r.targets_raw : r.targets) as string[],
        effect: r.effect as string,
        description: '',
        ...(r.conditions !== undefined ? { conditions: r.conditions as Record<string, unknown> } : {}),
        ...(r.approval !== undefined ? { approval: r.approval } : {}),
      }) as ACLRule,
  );
  return { acl: new ACL(rules, tc.default_effect, (e) => entries.push(e)), entries };
}

/**
 * `arguments: null` means NO PROJECTION AT ALL (§6.1.8 rule 1), which is a
 * different case from an empty one — so run 1 supplies nothing either, and the
 * fixture's two columns then hold the same values by construction.
 */
function projectionFor(tc: Case): GovernanceProjection | null {
  return tc.arguments === null ? null : buildGovernanceProjection(tc.arguments);
}

function context(): Context {
  return Context.create(new Identity('u', 'user', ['dev']));
}

/**
 * Assert one column against both the sync and the async entry points.
 *
 * Both pairs are asserted because §6.1.1 rule 5 has to hold on every entry
 * point or it holds on none: a requirement that survives an unevaluable rule
 * under `checkAccess()` and is lost under `asyncCheckAccess()` is the same
 * fail-open, reachable by choosing a different call.
 */
async function exercise(
  tc: Case,
  projection: GovernanceProjection | null,
  expected: Column,
  label: string,
): Promise<void> {
  const note = `${label}\n  ${tc.note}`;
  const options = projection === null ? undefined : { arguments: projection };

  for (const mode of ['sync', 'async'] as const) {
    const where = `${note}\n  [${mode}]`;
    const { acl, entries } = build(tc);

    const decision =
      mode === 'sync'
        ? acl.checkAccess(tc.caller_id, tc.target_id, context(), options)
        : await acl.asyncCheckAccess(tc.caller_id, tc.target_id, context(), options);
    expect(decision.access, where).toBe(expected.access);
    expect(decision.approvalRequired, where).toBe(expected.approvalRequired);
    expect(decision.matchedRuleIndex, where).toBe(expected.matchedRuleIndex);

    // §6.3.1: handlerError is non-null IF AND ONLY IF a condition was
    // unevaluable — the marker that keeps "the handler said no" and "no answer
    // was obtainable" tellable apart after the fact. Read before the legacy
    // call below, which emits its own entry.
    expect(entries.length, `${where}\n  the check must emit exactly one audit entry`).toBe(1);
    const entry = entries[0];
    expect(entry.handlerError !== null, `${where}\n  handlerError was ${entry.handlerError}`).toBe(
      expected.handlerErrorPresent,
    );
    // §6.1.1 rule 5: the audit entry carries the FINAL requirement, including
    // one that a rule which did not match raised and this one inherited.
    expect(entry.approvalRequired, where).toBe(expected.approvalRequired);

    // §6.8.1: the legacy boolean fails closed on an approval requirement, and
    // does so on a PENDING one identically — it is a property of the decision,
    // not of the matched rule.
    const legacy =
      mode === 'sync'
        ? acl.check(tc.caller_id, tc.target_id, context(), options)
        : await acl.asyncCheck(tc.caller_id, tc.target_id, context(), options);
    expect(legacy, where).toBe(expected.legacyCheck);
  }
}

describeIfPresent(
  'Conformance: argument-scoped approval (§6.1.1/§6.1.6/§6.1.7/§6.1.8, spec v1.29.0)',
  () => {
    for (const tc of cases) {
      it(tc.id, async () => {
        // Run 1 — a projection derived from `arguments` (§6.1.8 rule 2: key
        // set and types, never a value), by the route §6.1.8 rule 4 leaves
        // idiomatic.
        await exercise(tc, projectionFor(tc), withProjectionColumn(tc), 'run 1: with projection');

        // Run 2 — NO PROJECTION AT ALL. `check()` is public API and a caller
        // that is not the Executor may invoke it without one (§6.1.8 case 1).
        const second = noProjectionColumn(tc);
        if (second !== null) {
          await exercise(tc, null, second, 'run 2: no projection');
        }

        // §6.1.8 closing paragraph: the well-formedness cases are decidable
        // with no context and no handler, so validateRules() must surface them
        // at deploy time rather than at the first call that trips them.
        // Validation is context-free, so it has one column, not two.
        const { acl } = build(tc);
        const findings = acl.validateRules();
        // §6.1.8 rule 3: every faulty predicate is reported, so a case may pin
        // the exact finding set rather than the presence of one.
        //
        // `?? null` rather than a bare `!== null`: drivers land one push BEFORE
        // the fixture in this project, so a driver must tolerate the fixture
        // shape that predates the key it reads. An absent key arrives as
        // `undefined`, which is not `null`, and the strict comparison sent every
        // case into this branch to be compared against `undefined`.
        const expectedPaths = tc.expected_validation_finding_paths ?? null;
        if (expectedPaths !== null) {
          expect(
            findings.map((f) => f.conditionPath),
            tc.note,
          ).toEqual(expectedPaths);
          for (const f of findings) {
            expect(f.syncResolvable, tc.note).toBe(false);
            expect(f.asyncResolvable, tc.note).toBe(false);
          }
        } else if (tc.expected_validation_finding_path !== null) {
          const at = findings.filter((f) => f.conditionPath === tc.expected_validation_finding_path);
          expect(
            at.length,
            `${tc.note}\n  no finding at '${tc.expected_validation_finding_path}': ${JSON.stringify(findings)}`,
          ).toBeGreaterThan(0);
          expect(at[0].syncResolvable, tc.note).toBe(false);
          expect(at[0].asyncResolvable, tc.note).toBe(false);
        } else {
          expect(findings, `${tc.note}\n  unexpected findings`).toEqual([]);
        }
      });
    }

    it('deny + approval: required is rejected at every entry point (§6.1.6 rule 3)', () => {
      expect(
        () =>
          new ACL([
            { callers: ['*'], targets: ['x.y'], effect: 'deny', description: '', approval: 'required' },
          ]),
      ).toThrow(ACLRuleError);
    });
  },
);
