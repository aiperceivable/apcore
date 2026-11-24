/**
 * Cross-language conformance driver for approval_request_fields.json (D-03).
 *
 * Fixture source: apcore/conformance/fixtures/approval_request_fields.json
 * (single source of truth). See that fixture's `description` and
 * `driver_contract` for the contract this drives.
 *
 * Spec decision D-03 (apcore/docs/spec/2026-05-decision-log.md, PROTOCOL_SPEC
 * §7.3.1): `ApprovalRequest` carries `callerId` and `action`, populated by the
 * approval gate at Executor Step 4.5 from the Context and module ID already in
 * scope there — `callerId = context.callerId`, `action = moduleId`.
 *
 * The two cases pin the halves that fail in opposite directions:
 *   - nested call: `action` is the TARGET module, not a handler-supplied label.
 *   - top-level call: `callerId` is `null`, never the `"@external"` sentinel
 *     ACL evaluation substitutes internally (§5.7 — `Context.callerId` is null
 *     until `child()` sets it, and the gate reads it with no substitution).
 *
 * `tests/test-approval-executor.test.ts` asserts both by hand. A hand copy
 * cannot notice when the canonical fixture gains a case, which is why this
 * driver reads the fixture itself.
 *
 * Per the fixture's `driver_contract.no_wire_assertion`, both fields are read
 * off the in-process request object the handler received, never a serialized
 * round-trip.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Type } from '@sinclair/typebox';
import { CallbackApprovalHandler, createApprovalResult } from '../src/approval.js';
import type { ApprovalRequest } from '../src/approval.js';
import { Context } from '../src/context.js';
import { FunctionModule } from '../src/decorator.js';
import { Executor } from '../src/executor.js';
import { createAnnotations } from '../src/module.js';
import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

interface ApprovalRequestFieldsCase {
  readonly id: string;
  readonly description: string;
  readonly caller_id: string | null;
  readonly target_id: string;
  readonly expected_request_caller_id: string | null;
  readonly expected_request_action: string;
}

function loadFixture(name: string): { test_cases: readonly ApprovalRequestFieldsCase[] } {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

const Permissive = Type.Object({}, { additionalProperties: true });

/** The target: requires approval, so Step 4.5 builds an ApprovalRequest. */
function gatedModule(moduleId: string): FunctionModule {
  return new FunctionModule({
    moduleId,
    inputSchema: Permissive,
    outputSchema: Permissive,
    description: 'gated target for the D-03 callerId/action contract',
    annotations: createAnnotations({ requiresApproval: true }),
    execute: () => ({ status: 'executed' }),
  });
}

/**
 * A module that invokes `targetId` through its own Context.
 *
 * The nested call is made the way a real module makes one — through
 * `context.executor`, so the Executor's own `Context.child()` sets `callerId` —
 * rather than by hand-building a Context with the field already set. A driver
 * that pre-set the field would pass against a gate that read anything at all
 * off the context it was handed.
 */
function callerModule(moduleId: string, targetId: string): FunctionModule {
  return new FunctionModule({
    moduleId,
    inputSchema: Permissive,
    outputSchema: Permissive,
    description: 'caller for the D-03 nested-call case',
    execute: async (inputs: Record<string, unknown>, context: Context) => {
      const executor = context.executor as Executor;
      return executor.call(targetId, inputs, context);
    },
  });
}

describe('Conformance: ApprovalRequest.callerId / .action (D-03)', () => {
  const fixture = loadFixture('approval_request_fields');

  fixture.test_cases.forEach((tc) => {
    it(tc.id, async () => {
      const captured: ApprovalRequest[] = [];
      const handler = new CallbackApprovalHandler(async (request: ApprovalRequest) => {
        captured.push(request);
        return createApprovalResult({ status: 'approved', approvedBy: 'recorder' });
      });

      const registry = new Registry();
      registry.register(tc.target_id, gatedModule(tc.target_id));

      const executor = new Executor({ registry, approvalHandler: handler });

      if (tc.caller_id === null) {
        // Top-level: a fresh Context that never passed through child().
        await executor.call(tc.target_id, {});
      } else {
        registry.register(tc.caller_id, callerModule(tc.caller_id, tc.target_id));
        await executor.call(tc.caller_id, {});
      }

      expect(captured, 'the gate must reach the handler exactly once').toHaveLength(1);
      const request = captured[0];
      expect(request.callerId).toBe(tc.expected_request_caller_id);
      expect(request.action).toBe(tc.expected_request_action);
    });
  });
});
