/**
 * Cross-language conformance driver for acl_handler_error.json
 * (PROTOCOL_SPEC §6.1.1 / §6.1.4 / §6.1.4.1 / §6.3.1; A-D-011 / A-D-012).
 *
 * Fixture source: `<spec repo>/conformance/fixtures/acl_handler_error.json`.
 * See the fixture's own `description` for the driver contract.
 *
 * SECURITY: a condition that cannot be EVALUATED is not a condition that is
 * FALSE, and the rule's `effect` decides what the difference means. An
 * unevaluable condition resolves the rule toward refusing access: a `deny`
 * rule takes effect and the call is DENIED, an `allow` rule does not match and
 * MUST NOT grant. The emitted AuditEntry carries a non-null handlerError in
 * both directions.
 */

import { describe, it, expect, afterEach } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { ACL } from '../src/acl.js';
import type { ACLRule, AuditEntry } from '../src/acl.js';
import { Context, Identity } from '../src/context.js';
import type { ACLConditionHandler } from '../src/acl-handlers.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadHandlerErrorFixture(): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, 'acl_handler_error.json'), 'utf-8'));
}

/**
 * Build the `Identity` a case's `context_identity` names.
 *
 * Several cases turn on whether a `roles` condition is SATISFIED, so the
 * identity is part of the fixture rather than the driver's choice. A bare,
 * identity-less context makes
 * `execution_fault_does_not_gate_when_an_or_sibling_is_satisfied` return
 * `false` where `true` is expected — its `roles` branch is UNSATISFIED rather
 * than SATISFIED, so the `$or` is UNEVALUABLE and the `allow` rule correctly
 * does not grant. That failure reads as over-gating and is not, which is
 * exactly why the field must be honoured rather than guessed at.
 */
function buildIdentity(raw: any): Identity {
  return new Identity(raw.id, raw.type, raw.roles ?? []);
}

/**
 * Every §6.1.4 condition path named by a `handler_error`, in the order the
 * message lists them.
 *
 * Each `"; "`-separated part begins with the path in single quotes — "Unknown
 * ACL condition '$or[0].k'", "ACL rule field 'callers' must be…" — so the
 * first quoted token of each part is the path the fixture asserts on.
 */
function handlerErrorPaths(message: string): string[] {
  return message.split('; ').map((part) => {
    const match = /'([^']*)'/.exec(part);
    return match === null ? part : match[1];
  });
}

describe('Conformance: ACL unevaluable conditions (§6.1.1 / §6.1.4 / A-D-011 / A-D-012)', () => {
  const fixture: any = loadHandlerErrorFixture();
  const throwingKey: string = fixture.throwing_condition_key;
  // Present only from spec v1.25.0 onward.
  const unknownKey: string | undefined = fixture.unknown_condition_key;

  // Register a built-in test condition handler whose evaluate() throws, so the
  // ACL exercises its handler-error path. ACL keeps handlers in a static
  // registry; remove the registered key after each case to avoid leakage.
  const throwingHandler: ACLConditionHandler = {
    evaluate(): boolean {
      throw new Error('intentional throwing condition handler');
    },
  };

  afterEach(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (ACL as any).conditionHandlers?.delete(throwingKey);
  });

  fixture.test_cases.forEach((tc: any) => {
    it(tc.id, () => {
      ACL.registerCondition(throwingKey, throwingHandler);
      // The fixture's unknown key must stay unregistered — that is the whole
      // point of the case. Assert it rather than trusting suite ordering.
      if (unknownKey !== undefined) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        expect((ACL as any).conditionHandlers.has(unknownKey)).toBe(false);
      }

      // `callers_raw` / `targets_raw` carry a deliberately malformed value in
      // place of the pattern list (§6.1.4.1). TypeScript can represent both,
      // so `skip_if_unrepresentable` never applies here — unlike apcore-rust,
      // whose `Vec<String>` makes the value unconstructible.
      const rules: ACLRule[] = (tc.rules as any[]).map((r) => ({
        callers: ('callers_raw' in r ? r.callers_raw : r.callers) as string[],
        targets: ('targets_raw' in r ? r.targets_raw : r.targets) as string[],
        effect: r.effect,
        description: r.description ?? '',
        conditions: r.conditions ?? null,
      }));

      const captured: AuditEntry[] = [];
      const acl = new ACL(rules, tc.default_effect, (entry) => captured.push(entry));

      // §6.1.4: a context is supplied only when the case asks for one. The
      // no-context cases are what pin the precheck's ordering against §6.5.
      const withContext = tc.with_context !== false;
      if (withContext) {
        // Fail loudly rather than substituting a bare context: a case whose
        // identity is missing is a case that is not testing what it claims.
        expect(
          tc.context_identity,
          `case '${tc.id}' has with_context true but no context_identity`,
        ).toBeTruthy();
      }
      const ctx = withContext
        ? new Context('trace-id', tc.caller_id, [], null, buildIdentity(tc.context_identity))
        : null;

      let decision: boolean | undefined;
      // §6.1.1 rule 4 / Contract: ACL.check — nothing may raise out of check().
      expect(() => {
        decision = acl.check(tc.caller_id, tc.target_id, ctx);
      }).not.toThrow();

      expect(decision, `case '${tc.id}'`).toBe(tc.expected);

      expect(captured.length).toBeGreaterThan(0);
      const last = captured[captured.length - 1];

      if (tc.expected_audit_handler_error_present) {
        expect(last.handlerError).not.toBeNull();
      } else {
        // The control case: a well-formed conditional rule skipped for want of
        // a context is NOT unevaluable. A non-null handlerError here means the
        // precheck is over-reaching.
        expect(last.handlerError).toBeNull();
      }

      if (Array.isArray(tc.expected_handler_error_paths)) {
        expect(handlerErrorPaths(last.handlerError as string)).toEqual(
          tc.expected_handler_error_paths,
        );
      }
    });
  });
});
