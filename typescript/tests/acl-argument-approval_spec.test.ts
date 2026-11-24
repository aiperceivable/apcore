/**
 * Argument-scoped approval — PROTOCOL_SPEC §6.1.6–§6.1.8, §6.3.1, §6.8.1,
 * §6.9, §7.4 and §7.9.5 (spec v1.28.0, apcore#108).
 *
 * The governance gap this closes: the ACL could REFUSE on arguments and an
 * `ApprovalHandler` could WAVE THROUGH on arguments, but nothing could ASK on
 * arguments — and a refusal is not a question. An operator who needed
 * `git push --force` reviewed had to gate every `git push`.
 *
 * The security-relevant assertions here, in the order they matter:
 *   1. the legacy boolean `check()` fails CLOSED on an approval requirement;
 *   2. an `ExecutionPolicy` override cannot CLEAR a requirement the ACL set;
 *   3. the governance projection carries argument keys and never a value.
 */

import { describe, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { Type } from '@sinclair/typebox';
import { ACL } from '../src/acl.js';
import type { ACLRule, AuditEntry } from '../src/acl.js';
import { ArgumentsHandler, buildGovernanceProjection } from '../src/acl-handlers.js';
import { createApprovalResult } from '../src/approval.js';
import type { ApprovalRequest, ApprovalResult } from '../src/approval.js';
import {
  BuiltinACLCheck,
  BuiltinApprovalGate,
  BuiltinModuleLookup,
} from '../src/builtin-steps.js';
import { Context } from '../src/context.js';
import { ACLRuleError, ApprovalDeniedError } from '../src/errors.js';
import { Executor } from '../src/executor.js';
import { createAnnotations } from '../src/module.js';
import type { ModuleAnnotations } from '../src/module.js';
import type { PipelineContext } from '../src/pipeline.js';
import { ExecutionPolicy, PolicyRule } from '../src/policy.js';
import { Registry } from '../src/registry/registry.js';

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const PermissiveInput = Type.Object({}, { additionalProperties: true });
const PermissiveOutput = Type.Object({}, { additionalProperties: true });

function makeModule(annotations: ModuleAnnotations = createAnnotations()): Record<string, unknown> {
  return {
    inputSchema: PermissiveInput,
    outputSchema: PermissiveOutput,
    annotations,
    description: 'test module',
    execute: (_inputs: Record<string, unknown>) => ({ status: 'executed' }),
  };
}

function rule(over: Partial<ACLRule> & Pick<ACLRule, 'callers' | 'targets' | 'effect'>): ACLRule {
  return { description: '', conditions: null, ...over };
}

/** Records every request it is handed and returns a fixed decision. */
class RecordingHandler {
  requests: ApprovalRequest[] = [];

  constructor(private readonly _status: ApprovalResult['status'] = 'approved') {}

  async requestApproval(request: ApprovalRequest): Promise<ApprovalResult> {
    this.requests.push(request);
    return createApprovalResult({ status: this._status });
  }

  async checkApproval(_id: string): Promise<ApprovalResult> {
    return createApprovalResult({ status: 'rejected', reason: 'not supported' });
  }
}

function writeAcl(body: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'acl-approval-'));
  const file = path.join(dir, 'acl.yaml');
  fs.writeFileSync(file, body);
  return file;
}

/**
 * A context that reaches the ACL as caller `agent.planner`, the caller the
 * rules below name.
 *
 * `Context.create()` deliberately gives a top-level context
 * `callerId === null` — which the ACL reads as `@external` — and Step 1 derives
 * the caller from the tail of the parent's `callChain`, so the chain is seeded
 * here to stand in for an inter-module call.
 */
function callerContext(): Context {
  return new Context('0'.repeat(31) + '1', null, ['agent.planner']);
}

// ---------------------------------------------------------------------------
// §6.1.6 — `approval` is a rule field, and `deny` + `required` is rejected
// ---------------------------------------------------------------------------

describe('§6.1.6 the approval rule field', () => {
  it('defaults to not_required so every pre-v1.28.0 rule keeps its meaning', () => {
    const file = writeAcl(
      'default_effect: deny\nrules:\n  - callers: ["*"]\n    targets: ["cli.git_push"]\n    effect: allow\n',
    );
    const acl = ACL.load(file);
    expect(acl.rules[0].approval).toBe('not_required');
    expect(acl.check('agent.planner', 'cli.git_push')).toBe(true);
  });

  it('loads approval: required on an allow rule', () => {
    const file = writeAcl(
      'default_effect: deny\nrules:\n  - callers: ["*"]\n    targets: ["cli.git_push"]\n' +
        '    effect: allow\n    approval: required\n',
    );
    expect(ACL.load(file).rules[0].approval).toBe('required');
  });

  it('rejects approval: required on a deny rule at load', () => {
    const file = writeAcl(
      'default_effect: allow\nrules:\n  - callers: ["*"]\n    targets: ["cli.git_push"]\n' +
        '    effect: deny\n    approval: required\n',
    );
    expect(() => ACL.load(file)).toThrow(ACLRuleError);
    // The message has to explain WHY, because the operator wrote two things
    // that each make sense on their own.
    expect(() => ACL.load(file)).toThrow(/deny/);
  });

  it('rejects deny + approval: required on direct construction, not only on load', () => {
    // §6.1.2 rule 4: every entry point that accepts rules is covered. A rule
    // built in code is exactly as meaningless as one parsed from YAML.
    expect(
      () =>
        new ACL([
          rule({ callers: ['*'], targets: ['cli.*'], effect: 'deny', approval: 'required' }),
        ]),
    ).toThrow(ACLRuleError);
  });

  it('rejects deny + approval: required on runtime insertion', () => {
    const acl = new ACL([]);
    expect(() =>
      acl.addRule(
        rule({ callers: ['*'], targets: ['cli.*'], effect: 'deny', approval: 'required' }),
      ),
    ).toThrow(ACLRuleError);
    expect(acl.rules.length).toBe(0);
  });

  it('rejects a value outside the two-member enumeration rather than coercing it', () => {
    const file = writeAcl(
      'default_effect: deny\nrules:\n  - callers: ["*"]\n    targets: ["cli.*"]\n' +
        '    effect: allow\n    approval: yes_please\n',
    );
    expect(() => ACL.load(file)).toThrow(ACLRuleError);
  });

  it('is a member of the closed rule key set, so a rule carrying it loads', () => {
    // v1.27.0 closed the key set; adding `approval` there is what makes this
    // whole feature safe. Were it absent, the rule below would load as a bare
    // `allow` and the approval half would vanish in silence.
    const file = writeAcl(
      'default_effect: deny\nrules:\n  - callers: ["agent.*"]\n    targets: ["orders.*"]\n' +
        '    effect: allow\n    approval: required\n    description: "ask first"\n',
    );
    expect(ACL.load(file).rules.length).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// §6.8.1 — AccessDecision, and the legacy boolean failing closed
// ---------------------------------------------------------------------------

describe('§6.8.1 AccessDecision', () => {
  const approvalAcl = (): ACL =>
    new ACL(
      [
        rule({
          callers: ['agent.*'],
          targets: ['cli.git_push'],
          effect: 'allow',
          approval: 'required',
          description: 'ask a human',
        }),
      ],
      'deny',
    );

  it('reports both axes of §6.1.6', () => {
    const decision = approvalAcl().checkAccess('agent.planner', 'cli.git_push');
    expect(decision.access).toBe('allow');
    expect(decision.approvalRequired).toBe(true);
    expect(decision.matchedRuleIndex).toBe(0);
    expect(decision.reason).toBe('rule_match');
  });

  it('asyncCheckAccess agrees with the sync accessor', async () => {
    const decision = await approvalAcl().asyncCheckAccess('agent.planner', 'cli.git_push');
    expect(decision).toEqual({
      access: 'allow',
      approvalRequired: true,
      matchedRuleIndex: 0,
      reason: 'rule_match',
    });
  });

  it('SECURITY: the legacy check() returns false when approval is required', () => {
    // A non-Executor caller can only read a boolean as "let it through / do
    // not". Returning true would let it run a call the ACL said needed a
    // human. False is wrong in the benign direction.
    expect(approvalAcl().check('agent.planner', 'cli.git_push')).toBe(false);
  });

  it('SECURITY: the legacy asyncCheck() fails closed identically', async () => {
    expect(await approvalAcl().asyncCheck('agent.planner', 'cli.git_push')).toBe(false);
  });

  it('leaves the boolean untouched for a plain allow', () => {
    const acl = new ACL(
      [rule({ callers: ['agent.*'], targets: ['cli.git_push'], effect: 'allow' })],
      'deny',
    );
    expect(acl.check('agent.planner', 'cli.git_push')).toBe(true);
    expect(acl.checkAccess('agent.planner', 'cli.git_push').approvalRequired).toBe(false);
  });

  it('§6.9 row 2: no match means no approval requirement', () => {
    const decision = new ACL([], 'allow').checkAccess('agent.planner', 'cli.git_push');
    expect(decision).toEqual({
      access: 'allow',
      approvalRequired: false,
      matchedRuleIndex: null,
      reason: 'no_rules',
    });
  });

  it('§6.3.1: approvalRequired rides beside decision on the audit entry', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL(
      [
        rule({
          callers: ['agent.*'],
          targets: ['cli.git_push'],
          effect: 'allow',
          approval: 'required',
        }),
      ],
      'deny',
      (e) => entries.push(e),
    );
    acl.checkAccess('agent.planner', 'cli.git_push');
    expect(entries).toHaveLength(1);
    // `decision` stays a two-value string — a third value would break every
    // downstream parser.
    expect(entries[0].decision).toBe('allow');
    expect(entries[0].approvalRequired).toBe(true);
  });

  it('§6.3.1: exactly one audit entry per check(), even though it delegates', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL(
      [rule({ callers: ['*'], targets: ['cli.*'], effect: 'allow', approval: 'required' })],
      'deny',
      (e) => entries.push(e),
    );
    acl.check('agent.planner', 'cli.git_push');
    expect(entries).toHaveLength(1);
  });

  it('records approvalRequired false on the default-effect branch', () => {
    const entries: AuditEntry[] = [];
    new ACL([], 'deny', (e) => entries.push(e)).check('agent.planner', 'cli.git_push');
    expect(entries[0].approvalRequired).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// §6.1.8 — the governance projection
// ---------------------------------------------------------------------------

describe('§6.1.8 the governance projection', () => {
  it('carries the key set and each key JSON type, and NO value', () => {
    const projection = buildGovernanceProjection({
      force: true,
      remote: 'origin',
      retries: 3,
      refs: ['main'],
      opts: { a: 1 },
      note: null,
      token: 'sk-live-do-not-leak-me',
    });
    expect([...projection.keys].sort()).toEqual([
      'force', 'note', 'opts', 'refs', 'remote', 'retries', 'token',
    ]);
    expect(projection.types).toEqual({
      force: 'boolean',
      remote: 'string',
      retries: 'number',
      refs: 'array',
      opts: 'object',
      note: 'null',
      token: 'string',
    });
    // Structural, not a convention: a projection that cannot hold a value
    // cannot leak one, whatever a future predicate does with it.
    expect(JSON.stringify(projection)).not.toContain('sk-live-do-not-leak-me');
    expect(JSON.stringify(projection)).not.toContain('origin');
  });

  it('excludes the protocol-level _approval_token key', () => {
    // §7.9.6 rule 5's reasoning: the token is not caller input, so a retry
    // carrying it must present the same argument shape to governance as the
    // original call did.
    const projection = buildGovernanceProjection({ force: true, _approval_token: 'tok-1' });
    expect(projection.keys).toEqual(['force']);
  });

  it('is populated at Step 3, BEFORE Step 4 reads it', async () => {
    // §6.1.8 rule 1 makes the ordering normative rather than an implementation
    // detail that happens to hold.
    const registry = new Registry();
    registry.register('cli.git_push', makeModule());
    const ctx: PipelineContext = {
      moduleId: 'cli.git_push',
      inputs: { force: true },
      context: callerContext(),
      module: null,
    };
    expect(ctx.governanceProjection).toBeUndefined();

    await new BuiltinModuleLookup(registry).execute(ctx);
    expect(ctx.governanceProjection?.keys).toEqual(['force']);

    // …and the ACL check declares that dependency rather than trusting order.
    expect(new BuiltinModuleLookup(registry).provides).toContain('governanceProjection');
    expect(new BuiltinACLCheck(null).requires).toContain('governanceProjection');
  });

  it('is NOT context.redactedInputs — that field is the logging copy', async () => {
    const registry = new Registry();
    // No inputSchema, so redaction has no `x-sensitive` markers to work from;
    // `redactedInputs` carries the values the key rules leave alone, and is
    // redacted for logging, not shaped for a decision. Reusing it as the
    // governance input would put values into the decision path.
    registry.register('cli.git_push', {
      outputSchema: PermissiveOutput,
      annotations: createAnnotations(),
      description: 'schema-less',
      execute: () => ({}),
    });
    const ctx: PipelineContext = {
      moduleId: 'cli.git_push',
      inputs: { token: 'sk-live-do-not-leak-me' },
      context: callerContext(),
      module: null,
    };
    await new BuiltinModuleLookup(registry).execute(ctx);
    // PROTOCOL_SPEC 10.6.1 requirement 3: no schema still means the default
    // rules, never no redaction.
    expect(ctx.context.redactedInputs).toEqual({ token: '***REDACTED***' });
    expect(ctx.governanceProjection).toEqual({ keys: ['token'], types: { token: 'string' } });
  });
});

// ---------------------------------------------------------------------------
// §6.1.7 — the `arguments` condition
// ---------------------------------------------------------------------------

describe('§6.1.7 the arguments condition', () => {
  const projectionOf = (args: Record<string, unknown>) => ({
    arguments: buildGovernanceProjection(args),
  });

  function conditionalAcl(conditions: Record<string, unknown>, effect = 'allow'): ACL {
    return new ACL(
      [rule({ callers: ['agent.*'], targets: ['cli.git_push'], effect, conditions })],
      effect === 'allow' ? 'deny' : 'allow',
    );
  }

  it('has_key passes when ANY named key is present', () => {
    const acl = conditionalAcl({ arguments: { has_key: ['force', 'mirror'] } });
    const ctx = callerContext();
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ force: true }))).toBe(true);
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ mirror: 1 }))).toBe(true);
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ remote: 'o' }))).toBe(false);
  });

  it('has_all_keys passes only when EVERY named key is present', () => {
    const acl = conditionalAcl({ arguments: { has_all_keys: ['force', 'remote'] } });
    const ctx = callerContext();
    expect(
      acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ force: true, remote: 'o' })),
    ).toBe(true);
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ force: true }))).toBe(false);
  });

  it('has_none_of passes only when NONE of the named keys is present', () => {
    const acl = conditionalAcl({ arguments: { has_none_of: ['force'] } });
    const ctx = callerContext();
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ remote: 'o' }))).toBe(true);
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ force: true }))).toBe(false);
  });

  it('ANDs several predicates in one arguments object', () => {
    const acl = conditionalAcl({
      arguments: { has_key: ['force'], has_none_of: ['dry_run'] },
    });
    const ctx = callerContext();
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ force: true }))).toBe(true);
    expect(
      acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ force: true, dry_run: true })),
    ).toBe(false);
  });

  it('reads no value: a matching key with any value at all satisfies has_key', () => {
    const acl = conditionalAcl({ arguments: { has_key: ['force'] } });
    const ctx = callerContext();
    for (const value of [true, false, null, 0, '', [], {}]) {
      expect(
        acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ force: value })),
        `force: ${JSON.stringify(value)}`,
      ).toBe(true);
    }
  });

  it('SECURITY: a malformed predicate value is UNEVALUABLE, so a deny rule DENIES', () => {
    // §6.1.1: a handler handed a malformed value can run to completion and
    // look exactly like one that answered "no". Recording that as UNSATISFIED
    // is what left `$or: "typo"` rules inert.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const acl = conditionalAcl({ arguments: { has_key: 'force' } }, 'deny');
      expect(
        acl.check('agent.planner', 'cli.git_push', callerContext(), projectionOf({ force: true })),
      ).toBe(false);
    } finally {
      warn.mockRestore();
    }
  });

  it('SECURITY: a malformed predicate value on an allow rule does NOT grant', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const acl = conditionalAcl({ arguments: { has_key: [3] } });
      expect(
        acl.check('agent.planner', 'cli.git_push', callerContext(), projectionOf({ force: true })),
      ).toBe(false);
    } finally {
      warn.mockRestore();
    }
  });

  it('SECURITY: a typo in the predicate name is UNEVALUABLE, not silently dropped', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      // `has_keys` for `has_all_keys`. Dropping it would leave an
      // unconditional allow rule.
      const acl = conditionalAcl({ arguments: { has_keys: ['force'] } });
      expect(
        acl.check('agent.planner', 'cli.git_push', callerContext(), projectionOf({ force: true })),
      ).toBe(false);
    } finally {
      warn.mockRestore();
    }
  });

  it('SECURITY: an empty arguments object asks nothing and does not grant', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const acl = conditionalAcl({ arguments: {} });
      expect(
        acl.check('agent.planner', 'cli.git_push', callerContext(), projectionOf({ force: true })),
      ).toBe(false);
    } finally {
      warn.mockRestore();
    }
  });

  it('SECURITY: no projection supplied is UNEVALUABLE, so has_none_of does not grant', () => {
    // Reading a missing projection as "an empty argument set" would make
    // `has_none_of` GRANT for a call whose arguments were never seen.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const acl = conditionalAcl({ arguments: { has_none_of: ['force'] } });
      expect(acl.check('agent.planner', 'cli.git_push', callerContext())).toBe(false);
    } finally {
      warn.mockRestore();
    }
  });

  it('§6.1.4: malformed predicates are reported by validateRules(), deterministically', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    let findings;
    try {
      const acl = conditionalAcl({ arguments: { has_key: 'force' } }, 'deny');
      findings = acl.validateRules();
    } finally {
      warn.mockRestore();
    }
    expect(findings).toHaveLength(1);
    // §6.1.8: the path descends to the offending predicate, exactly as §6.1.4
    // descends into `$or[1].k`. The KEY stays `arguments`, for a reader who
    // wants the condition rather than its position in the tree.
    expect(findings[0].conditionPath).toBe('arguments.has_key');
    expect(findings[0].conditionKey).toBe('arguments');
    expect(findings[0].effect).toBe('deny');
  });

  it("§6.1.7: a misspelled `argument:` key is an unregistered condition, not a no-op", () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    let findings;
    try {
      const acl = conditionalAcl({ argument: { has_key: ['force'] } }, 'deny');
      findings = acl.validateRules();
      expect(
        acl.check('agent.planner', 'cli.git_push', callerContext(), projectionOf({ force: true })),
      ).toBe(false);
    } finally {
      warn.mockRestore();
    }
    expect(findings.map((f) => f.conditionPath)).toEqual(['argument']);
  });

  it('nests inside $or and $not like any other condition', () => {
    const acl = conditionalAcl({
      $or: [{ arguments: { has_key: ['force'] } }, { arguments: { has_key: ['mirror'] } }],
    });
    const ctx = callerContext();
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ mirror: 1 }))).toBe(true);
    expect(acl.check('agent.planner', 'cli.git_push', ctx, projectionOf({ remote: 'o' }))).toBe(false);
  });

  it('resolves on the async path too', async () => {
    const acl = conditionalAcl({ arguments: { has_key: ['force'] } });
    const decision = await acl.asyncCheckAccess(
      'agent.planner', 'cli.git_push', callerContext(), projectionOf({ force: true }),
    );
    expect(decision.access).toBe('allow');
  });

  it('has no registration point of its own — it is registered as a built-in', () => {
    // §6.1.7: a deployment-registered argument handler is exactly the
    // unauditable host code §7.9.6 rule 2 keeps out of a governance verdict.
    // Being built-in is also what makes §6.1.4's precheck cover it for free.
    expect(new ACL([]).validateRules()).toHaveLength(0);
    const acl = new ACL([
      rule({
        callers: ['*'],
        targets: ['*'],
        effect: 'allow',
        conditions: { arguments: { has_key: ['x'] } },
      }),
    ]);
    // No fault: `arguments` resolves on both evaluation paths with nothing
    // registered by the application.
    expect(acl.validateRules()).toHaveLength(0);
  });

  it('ArgumentsHandler reports UNEVALUABLE through the outcome interface', () => {
    const handler = new ArgumentsHandler(() => null);
    expect(handler.evaluateOutcome({ has_key: ['a'] }, callerContext())).toBe('unevaluable');
    expect(
      handler.evaluateOutcome('nonsense', callerContext()),
    ).toBe('unevaluable');
  });
});

// ---------------------------------------------------------------------------
// §7.4 / §6.9 — the Step 5 union
// ---------------------------------------------------------------------------

describe('§7.4 the approval gate consults the ACL decision', () => {
  function executorWith(
    acl: ACL | null,
    handler: RecordingHandler | null,
    policy: ExecutionPolicy | null = null,
  ): Executor {
    const registry = new Registry();
    registry.register('cli.git_push', makeModule());
    return new Executor({
      registry,
      acl,
      approvalHandler: handler as never,
      policy,
    });
  }

  const forceRule = (): ACL =>
    new ACL(
      [
        rule({
          callers: ['agent.*'],
          targets: ['cli.git_push'],
          effect: 'allow',
          approval: 'required',
          conditions: { arguments: { has_key: ['force'] } },
        }),
        rule({ callers: ['agent.*'], targets: ['cli.git_push'], effect: 'allow' }),
      ],
      'deny',
    );

  it('asks a human for the call that carried --force, and only that one', async () => {
    const handler = new RecordingHandler('approved');
    const executor = executorWith(forceRule(), handler);

    await executor.call('cli.git_push', { remote: 'origin' }, callerContext());
    expect(handler.requests).toHaveLength(0);

    await executor.call('cli.git_push', { remote: 'origin', force: true }, callerContext());
    expect(handler.requests).toHaveLength(1);
    expect(handler.requests[0].moduleId).toBe('cli.git_push');
  });

  it('§7.4 rule 3: the ApprovalRequest carries requiresApproval effectively true', async () => {
    const handler = new RecordingHandler('approved');
    await executorWith(forceRule(), handler).call(
      'cli.git_push', { force: true }, callerContext(),
    );
    // The module declares no annotation at all — the requirement came from the
    // ACL, and the handler must still see the §7 contract honoured.
    expect(handler.requests[0].annotations.requiresApproval).toBe(true);
  });

  it('a rejected ACL-sourced approval stops the call', async () => {
    const handler = new RecordingHandler('rejected');
    await expect(
      executorWith(forceRule(), handler).call('cli.git_push', { force: true }, callerContext()),
    ).rejects.toBeInstanceOf(ApprovalDeniedError);
  });

  it('the Executor does not read the fail-closed boolean: allow-with-approval still runs', async () => {
    // §6.8.1: the boolean fails closed, but the Executor uses the structured
    // API. Were it reading `check()`, this call would surface as ACL_DENIED
    // instead of going through the gate.
    const handler = new RecordingHandler('approved');
    const result = await executorWith(forceRule(), handler).call(
      'cli.git_push', { force: true }, callerContext(),
    );
    expect(result).toEqual({ status: 'executed' });
  });

  it('§6.9 row 3: the module annotation and the ACL requirement are a union', async () => {
    const registry = new Registry();
    registry.register('cli.git_push', makeModule(createAnnotations({ requiresApproval: true })));
    const handler = new RecordingHandler('approved');
    const executor = new Executor({
      registry,
      acl: new ACL(
        [rule({ callers: ['agent.*'], targets: ['cli.git_push'], effect: 'allow' })],
        'deny',
      ),
      approvalHandler: handler as never,
    });
    await executor.call('cli.git_push', {}, callerContext());
    // The ACL required nothing; the annotation still gates.
    expect(handler.requests).toHaveLength(1);
  });
});

// ---------------------------------------------------------------------------
// §6.1.1 rule 5 — an unevaluable allow rule's approval requirement is PENDING
// ---------------------------------------------------------------------------

/**
 * The regression suite for apcore#109, held here and not only in the
 * conformance driver: that driver reads a fixture from the sibling spec repo
 * and skips itself when it is absent, so without this block the fix would be
 * unverified on any checkout that has no spec repo beside it.
 *
 * The defect: §6.1.1 was written in spec v1.22.0, when a rule carried one axis,
 * and "an `allow` rule MUST NOT grant" was a complete instruction then — the
 * rule steps aside, and stepping aside was harmless because whatever granted
 * next also said `allow`. v1.28.0 gave rules a second axis and did not revisit
 * it, so a narrow `approval: required` gate whose condition could not be
 * evaluated stepped aside and the broad rule behind it granted `git push
 * --force` with `approvalRequired: false`.
 */
describe('§6.1.1 rule 5: an unevaluable allow rule leaves its approval requirement pending', () => {
  /** The deployment shape §6.1.7 exists for: a narrow gate ahead of a broad allow. */
  const gateThenBroadAllow = (conditions: Record<string, unknown>): ACLRule[] => [
    rule({
      callers: ['*'],
      targets: ['cli.git_push'],
      effect: 'allow',
      approval: 'required',
      conditions,
    }),
    rule({ callers: ['*'], targets: ['cli.git_push'], effect: 'allow' }),
  ];

  const HAS_FORCE = { arguments: { has_key: ['force'] } };

  it('THE DEFECT: no projection, so the gate is unevaluable and the broad rule granted unapproved', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL(gateThenBroadAllow(HAS_FORCE), 'deny', (e) => entries.push(e));

    // No `arguments` option at all — §6.1.8 case 1. `check()` is public API and
    // a caller that is not the Executor may invoke it without a projection.
    const decision = acl.checkAccess('agent.planner', 'cli.git_push', callerContext());

    expect(decision.access).toBe('allow');
    // Before the fix this was `false` and rule 1 had quietly replaced rule 0.
    expect(decision.approvalRequired).toBe(true);
    // The index still names the rule that actually DECIDED, not the one that
    // raised the requirement — the requirement may outlive its rule, the
    // diagnostic may not lie about which rule granted.
    expect(decision.matchedRuleIndex).toBe(1);
    // §6.1.1 rule 2 is untouched: a pending requirement neither suppresses nor
    // substitutes for the handler error, and the entry carries the FINAL value.
    expect(entries[0].handlerError).not.toBeNull();
    expect(entries[0].approvalRequired).toBe(true);
  });

  it('a misspelled predicate reaches it WITH a projection, on the ordinary Executor path', () => {
    // `has_keys` for `has_all_keys` — §6.1.8 case 3, an unevaluable rule that
    // has nothing to do with a missing projection. This is the shape that
    // proves the defect is not confined to callers who omit one.
    const acl = new ACL(gateThenBroadAllow({ arguments: { has_keys: ['force'] } }), 'deny');
    const decision = acl.checkAccess('agent.planner', 'cli.git_push', callerContext(), {
      arguments: buildGovernanceProjection({ remote: 'origin', force: true }),
    });

    expect(decision.access).toBe('allow');
    expect(decision.approvalRequired).toBe(true);
    expect(decision.matchedRuleIndex).toBe(1);
  });

  it('rides through default_effect: allow, where there is no later rule to carry it', () => {
    // The boundary a "a later rule grants" reading misses: there IS no later
    // rule. §6.9 row 2 — the default carries the pending requirement, which
    // makes approvalRequired: true with matchedRuleIndex: null legal.
    const acl = new ACL([gateThenBroadAllow(HAS_FORCE)[0]], 'allow');
    const decision = acl.checkAccess('agent.planner', 'cli.git_push', callerContext());

    expect(decision.access).toBe('allow');
    expect(decision.approvalRequired).toBe(true);
    expect(decision.matchedRuleIndex).toBeNull();
    expect(decision.reason).toBe('default_effect');
  });

  it('a malformed pattern field raises it, because the rule\'s scope cannot be read', () => {
    // §6.1.4.1 — unevaluable BEFORE any pattern is matched. The one point at
    // which a requirement attaches without a demonstrated pattern match, and
    // consistent with what the same rule does under `deny`, where an unreadable
    // scope denies every call. The cast is the point: a JS caller reaches this
    // shape by writing `callers: '*'` where a list belongs.
    const acl = new ACL(
      [
        rule({
          callers: '*' as unknown as string[],
          targets: ['cli.git_push'],
          effect: 'allow',
          approval: 'required',
        }),
        rule({ callers: ['*'], targets: ['cli.git_push'], effect: 'allow' }),
      ],
      'deny',
    );
    const decision = acl.checkAccess('agent.planner', 'cli.git_push', callerContext());

    expect(decision.access).toBe('allow');
    expect(decision.approvalRequired).toBe(true);
    expect(decision.matchedRuleIndex).toBe(1);
  });

  it('CONTAINMENT: a rule whose patterns do not match this call raises nothing', () => {
    // The case that keeps the fix from over-reaching. Rule 0 targets
    // `cli.deploy`, so §6.1.4 rule 4c means its conditions are never consulted
    // at all — an implementation that sets `pending` before matching patterns
    // passes every other test in this block and fails this one.
    const acl = new ACL(
      [
        rule({
          callers: ['*'],
          targets: ['cli.deploy'],
          effect: 'allow',
          approval: 'required',
          conditions: HAS_FORCE,
        }),
        rule({ callers: ['*'], targets: ['cli.git_push'], effect: 'allow' }),
      ],
      'deny',
    );
    const decision = acl.checkAccess('agent.planner', 'cli.git_push', callerContext());

    expect(decision.access).toBe('allow');
    expect(decision.approvalRequired).toBe(false);
    expect(decision.matchedRuleIndex).toBe(1);
  });

  it('a denial clears it, and matchedRuleIndex names the rule that decided', () => {
    const acl = new ACL(
      [
        rule({
          callers: ['*'],
          targets: ['cli.git_push'],
          effect: 'allow',
          approval: 'required',
          conditions: HAS_FORCE,
        }),
        rule({ callers: ['*'], targets: ['cli.git_push'], effect: 'deny' }),
      ],
      'allow',
    );
    const decision = acl.checkAccess('agent.planner', 'cli.git_push', callerContext());

    expect(decision.access).toBe('deny');
    // "Denied and needs approval" is not a state that means anything (§6.1.6).
    expect(decision.approvalRequired).toBe(false);
    expect(decision.matchedRuleIndex).toBe(1);
  });

  it('a deny default clears it exactly as a matched deny rule does', () => {
    const acl = new ACL([gateThenBroadAllow(HAS_FORCE)[0]], 'deny');
    const decision = acl.checkAccess('agent.planner', 'cli.git_push', callerContext());

    expect(decision.access).toBe('deny');
    expect(decision.approvalRequired).toBe(false);
  });

  it('the legacy boolean fails closed on a PENDING requirement too (§6.8.1)', () => {
    // §6.8.1 makes fail-closed a property of the DECISION, not of the matched
    // rule: the requirement here was raised by a rule that did not match.
    const acl = new ACL(gateThenBroadAllow(HAS_FORCE), 'deny');
    expect(acl.check('agent.planner', 'cli.git_push', callerContext())).toBe(false);
  });

  it('holds identically on the async entry points', async () => {
    // A requirement that survives an unevaluable rule on one entry point and is
    // lost on another is the same fail-open, reachable by choosing a call.
    const acl = new ACL(gateThenBroadAllow(HAS_FORCE), 'deny');
    const decision = await acl.asyncCheckAccess('agent.planner', 'cli.git_push', callerContext());

    expect(decision.access).toBe('allow');
    expect(decision.approvalRequired).toBe(true);
    expect(decision.matchedRuleIndex).toBe(1);
    expect(await acl.asyncCheck('agent.planner', 'cli.git_push', callerContext())).toBe(false);
  });

  it('reaches the Step 5 gate: a misspelled predicate still asks a human', async () => {
    // End to end, on the pipeline. `BuiltinACLCheck` stores the structured
    // decision's `approvalRequired` and `BuiltinApprovalGate` unions it with the
    // module annotation, so the pending requirement gates the call without the
    // gate knowing anything about §6.1.1.
    const registry = new Registry();
    registry.register('cli.git_push', makeModule());
    const handler = new RecordingHandler('approved');
    const executor = new Executor({
      registry,
      acl: new ACL(gateThenBroadAllow({ arguments: { has_keys: ['force'] } }), 'deny'),
      approvalHandler: handler as never,
    });

    await executor.call('cli.git_push', { remote: 'origin', force: true }, callerContext());

    // Before the fix the gate never saw this call: the typo turned "ask a
    // human" into "do not ask" and `git push --force` ran unapproved.
    expect(handler.requests).toHaveLength(1);
    expect(handler.requests[0].moduleId).toBe('cli.git_push');
  });
});

describe('§6.9 row 4: a policy may ADD a requirement and MUST NOT remove the ACL one', () => {
  function buildGate(policy: ExecutionPolicy | null, handler: RecordingHandler) {
    return new BuiltinApprovalGate(handler as never, policy, null);
  }

  function gateCtx(
    aclApprovalRequired: boolean,
    annotations: ModuleAnnotations = createAnnotations(),
  ): PipelineContext {
    return {
      moduleId: 'orders.ship',
      inputs: {},
      context: callerContext(),
      module: makeModule(annotations),
      aclApprovalRequired,
    };
  }

  it('SECURITY: requires_approval: false cannot strip a requirement the ACL set', async () => {
    // The ACL is CALLER-scoped and an ExecutionPolicy is MODULE-scoped.
    // Letting a policy written for `orders.*` cancel a requirement an ACL
    // author attached to one untrusted caller is a privilege escalation.
    const handler = new RecordingHandler('approved');
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.*', { requiresApproval: false, reason: 'bulk exemption' }),
    ]);
    await buildGate(policy, handler).execute(gateCtx(true));
    expect(handler.requests).toHaveLength(1);
  });

  it('a policy MAY add a requirement the ACL did not set', async () => {
    const handler = new RecordingHandler('approved');
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.*', { requiresApproval: true, reason: 'ops sign-off' }),
    ]);
    await buildGate(policy, handler).execute(gateCtx(false));
    expect(handler.requests).toHaveLength(1);
  });

  it('a policy exemption still clears the module ANNOTATION when the ACL asked nothing', async () => {
    // Row 4's second sentence: `requires_approval: false` overrides the
    // module's annotation, never the ACL's decision. Without this the guard
    // above would have been implemented as "ignore the policy", which is a
    // different and wrong behaviour.
    const handler = new RecordingHandler('approved');
    const policy = new ExecutionPolicy([
      new PolicyRule('orders.*', { requiresApproval: false, reason: 'bulk exemption' }),
    ]);
    await buildGate(policy, handler).execute(
      gateCtx(false, createAnnotations({ requiresApproval: true })),
    );
    expect(handler.requests).toHaveLength(0);
  });

  it('no gate at all when neither source asks', async () => {
    const handler = new RecordingHandler('approved');
    await buildGate(null, handler).execute(gateCtx(false));
    expect(handler.requests).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// §7.9.5 — preflight reports the governance-effective requirement
// ---------------------------------------------------------------------------

describe('§7.9.5 preflight reports the union, not the policy-effective value', () => {
  function preflightExecutor(policy: ExecutionPolicy | null = null): Executor {
    const registry = new Registry();
    registry.register('cli.git_push', makeModule());
    return new Executor({
      registry,
      acl: new ACL(
        [
          rule({
            callers: ['agent.*'],
            targets: ['cli.git_push'],
            effect: 'allow',
            approval: 'required',
            conditions: { arguments: { has_key: ['force'] } },
          }),
          rule({ callers: ['agent.*'], targets: ['cli.git_push'], effect: 'allow' }),
        ],
        'deny',
      ),
      policy,
    });
  }

  it('reports requiresApproval for the call the gate will stop', async () => {
    const result = await preflightExecutor().validate(
      'cli.git_push', { force: true }, callerContext(),
    );
    expect(result.valid).toBe(true);
    expect(result.requiresApproval).toBe(true);
  });

  it('reports no requirement for the call the gate will let through', async () => {
    const result = await preflightExecutor().validate(
      'cli.git_push', { remote: 'origin' }, callerContext(),
    );
    expect(result.requiresApproval).toBe(false);
  });

  it('SECURITY: a policy exemption does not hide the ACL requirement from preflight', async () => {
    const policy = new ExecutionPolicy([
      new PolicyRule('cli.*', { requiresApproval: false, reason: 'bulk exemption' }),
    ]);
    const result = await preflightExecutor(policy).validate(
      'cli.git_push', { force: true }, callerContext(),
    );
    // Reporting only the policy-effective value would tell the caller no
    // approval is needed for a call the gate will stop.
    expect(result.requiresApproval).toBe(true);
  });
});
