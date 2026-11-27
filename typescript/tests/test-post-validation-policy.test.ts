import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import {
  ACL,
  CancelToken,
  Context,
  createAnnotations,
  createApprovalResult,
  Executor,
  ModuleError,
  Registry,
  STREAMING_MARKER,
  type ApprovalHandler,
  type ApprovalRequest,
  type PolicyEvaluationRequest,
} from '../src/index.js';
import { ExtensionManager } from '../src/full.js';
import { Middleware } from '../src/middleware/index.js';
import { findFixturesRoot } from './spec-repo.js';

interface Case {
  id: string;
  entry: 'call' | 'stream' | 'preflight' | 'resume';
  verdict: 'allow' | 'deny' | 'require_approval' | 'throw';
  requires_approval?: boolean;
  rewrite_amount?: number;
  inputs?: Record<string, unknown>;
  retry_inputs?: Record<string, unknown>;
  expected: {
    module_starts: number;
    evaluated_amount: number | null;
    approval_requests: number;
    approved_amount?: number;
    requires_approval?: boolean;
    resume_checks?: number;
    error_code?: string;
  };
}

const fixture = JSON.parse(readFileSync(join(findFixturesRoot(), 'post_validation_policy.json'), 'utf8')) as {
  driver_contract: {
    module_id: string;
    module_version: string;
    input_schema: Record<string, unknown>;
    business_input: Record<string, unknown>;
  };
  test_cases: Case[];
};

const { module_id: moduleId, module_version: version, input_schema: inputSchema, business_input: input } = fixture.driver_contract;

async function makeRegistry(requiresApproval = false) {
  const registry = new Registry();
  const execute = vi.fn(async (args: Record<string, unknown>) => args);
  const stream = vi.fn(async function* (args: Record<string, unknown>) { yield args; });
  const preview = vi.fn(() => null);
  await registry.register(moduleId, {
    [STREAMING_MARKER]: true,
    description: 'Return validated arguments',
    inputSchema,
    outputSchema: inputSchema,
    annotations: createAnnotations({ streaming: true, requiresApproval }),
    execute,
    stream,
    preview,
  }, version);
  return { registry, execute, stream, preview };
}

function sameApprovalScope(left: ApprovalRequest, right: ApprovalRequest): boolean {
  return left.moduleId === right.moduleId && left.moduleVersion === right.moduleVersion &&
    left.callerId === right.callerId && left.context.identity?.id === right.context.identity?.id &&
    left.context.identity?.type === right.context.identity?.type &&
    left.canonicalArgsDigest === right.canonicalArgsDigest;
}

describe('post_validation_policy.json', () => {
  it.each(fixture.test_cases)('$id', async (testCase) => {
    const { registry, execute, stream } = await makeRegistry(testCase.requires_approval);
    const evaluated: PolicyEvaluationRequest[] = [];
    const approved: ApprovalRequest[] = [];
    let credentials = 0;
    const approvalHandler: ApprovalHandler = {
      supportsFinalScope: true,
      async requestApproval(request) {
        approved.push(request);
        return testCase.entry === 'resume'
          ? createApprovalResult({ status: 'pending', approvalId: 'scoped-resume' })
          : createApprovalResult({ status: 'approved' });
      },
      async checkApproval() { throw new Error('Status-only checks cannot authorize'); },
      async checkApprovalForCall(credential, request) {
        const matches = credentials++ === 0 && credential === 'scoped-resume' &&
          sameApprovalScope(approved[0], request);
        return createApprovalResult({ status: matches ? 'approved' : 'rejected' });
      },
    };
    const executor = new Executor({ registry, approvalHandler, policyEvaluator: {
      async evaluate(request) {
        evaluated.push(request);
        if (testCase.verdict === 'throw') throw new Error('backend-canary');
        return { decision: testCase.verdict };
      },
    } });
    if (testCase.rewrite_amount !== undefined) {
      executor.useBefore((_id, args) => ({ ...args, amount: testCase.rewrite_amount }));
    }

    let observedError: unknown;
    let requiresApproval = false;
    const args = { ...input, ...testCase.inputs };
    try {
      if (testCase.entry === 'preflight') {
        requiresApproval = (await executor.validate(moduleId, args)).requiresApproval;
      } else if (testCase.entry === 'stream') {
        for await (const _chunk of executor.stream(moduleId, args)) { /* drain */ }
      } else if (testCase.entry === 'resume') {
        await expect(executor.call(moduleId, args)).rejects.toMatchObject({ code: 'APPROVAL_PENDING' });
        await executor.call(moduleId, { ...args, ...testCase.retry_inputs, _approval_token: 'scoped-resume' });
      } else {
        await executor.call(moduleId, args);
      }
    } catch (error) {
      observedError = error;
    }

    const expected = testCase.expected;
    expect(execute.mock.calls.length + stream.mock.calls.length).toBe(expected.module_starts);
    expect(evaluated.at(-1)?.arguments.amount ?? null).toBe(expected.evaluated_amount);
    expect(approved).toHaveLength(expected.approval_requests);
    if ('approved_amount' in expected) expect(approved.at(-1)?.arguments.amount).toBe(expected.approved_amount);
    if ('requires_approval' in expected) expect(requiresApproval).toBe(expected.requires_approval);
    if ('resume_checks' in expected) expect(credentials).toBe(expected.resume_checks);
    if ('error_code' in expected) {
      expect(observedError).toMatchObject({ code: expected.error_code });
      expect(JSON.stringify(observedError)).not.toContain('backend-canary');
    } else if (expected.evaluated_amount === null) {
      expect(observedError).toBeInstanceOf(ModuleError);
    } else {
      expect(observedError).toBeUndefined();
    }
    for (const request of evaluated) {
      expect(request.moduleVersion).toBe(version);
      expect(request.arguments).not.toHaveProperty('_approval_token');
      expect(request.sensitiveFields).toContain('/secret');
    }
    for (const request of approved) {
      expect(request.moduleVersion).toBe(version);
      expect(request.canonicalArgsDigest).toMatch(/^[0-9a-f]{64}$/);
      expect(request.arguments).not.toHaveProperty('_approval_token');
    }
  });
});

describe('post-validation policy executor integration', () => {
  it('does not expose an ACL-rejected preflight to the evaluator or preview', async () => {
    const { registry, preview } = await makeRegistry();
    const evaluate = vi.fn(async () => ({ decision: 'allow' as const }));
    const executor = new Executor({ registry, acl: new ACL([], 'deny'), policyEvaluator: { evaluate } });
    const report = await executor.validate(moduleId, input);
    expect(report.valid).toBe(false);
    expect(evaluate).not.toHaveBeenCalled();
    expect(preview).not.toHaveBeenCalled();
  });

  it('prevents error middleware from recovering a rejected policy', async () => {
    const { registry, execute, stream } = await makeRegistry();
    const onError = vi.fn(() => input);
    class Recovery extends Middleware { override onError = onError; }
    const executor = new Executor({ registry, middlewares: [new Recovery()], policyEvaluator: {
      async evaluate() { return { decision: 'deny' }; },
    } });
    executor.addStepMiddleware({ onStepError: () => input });
    await expect(executor.call(moduleId, input)).rejects.toMatchObject({ code: 'POLICY_DENIED' });
    await expect(executor.stream(moduleId, input).next()).rejects.toMatchObject({ code: 'POLICY_DENIED' });
    expect(onError).not.toHaveBeenCalled();
    expect(execute).not.toHaveBeenCalled();
    expect(stream).not.toHaveBeenCalled();
  });

  it('does not substitute unvalidated inputs or an injected producer from step middleware', async () => {
    const { registry, stream } = await makeRegistry();
    const executor = new Executor({ registry, policyEvaluator: {
      async evaluate(request) {
        Reflect.set(request.arguments, 'amount', 50);
        return { decision: 'allow' };
      },
    } });
    const injected = vi.fn(async function* () { yield { amount: 999 }; });
    executor.addStepMiddleware({
      beforeStep(name, state) {
        if (name === 'input_validation') state.context.outputStream = injected();
        if (name === 'execute') state.context.inputs = { ...input, amount: 999 };
      },
    });
    const iterator = executor.stream(moduleId, input);
    expect((await iterator.next()).value).toEqual(input);
    expect((await iterator.next()).done).toBe(true);
    expect(stream.mock.lastCall?.[0]).toEqual(input);
  });

  it('rejects a strategy missing the approval and validation boundaries', async () => {
    const { registry, execute } = await makeRegistry();
    const evaluator = { async evaluate() { return { decision: 'allow' as const }; } };
    expect(() => new Executor({ registry, strategy: 'minimal', policyEvaluator: evaluator }))
      .toThrow(expect.objectContaining({ code: 'PIPELINE_CONFIGURATION_ERROR' }));
    expect(execute).not.toHaveBeenCalled();
  });

  it('does not turn caller cancellation into an evaluator fault', async () => {
    const { registry, execute } = await makeRegistry();
    const token = new CancelToken();
    const executor = new Executor({ registry, policyEvaluator: {
      async evaluate() {
        token.cancel();
        return { decision: 'allow' };
      },
    } });
    await expect(executor.call(moduleId, input, Context.create(null, null, token)))
      .rejects.toMatchObject({ code: 'EXECUTION_CANCELLED' });
    expect(execute).not.toHaveBeenCalled();
  });

  it('registers a code-only evaluator on just one executor', async () => {
    const { registry } = await makeRegistry();
    const enabled = new Executor({ registry });
    const normal = new Executor({ registry });
    const manager = new ExtensionManager();
    manager.register('policy_evaluator', { async evaluate() { return { decision: 'deny' }; } });
    manager.apply(registry, enabled);
    await expect(enabled.call(moduleId, input)).rejects.toMatchObject({ code: 'POLICY_DENIED' });
    await expect(normal.call(moduleId, input)).resolves.toEqual(input);
  });

  it('requires final-scope support for pending approval and does not fall back on resume', async () => {
    const { registry, execute } = await makeRegistry();
    const pending = vi.fn(async () => createApprovalResult({ status: 'pending', approvalId: 'legacy-token' }));
    const resume = vi.fn(async () => createApprovalResult({ status: 'approved' }));
    const executor = new Executor({
      registry,
      approvalHandler: { requestApproval: pending, checkApproval: resume, checkApprovalForCall: resume },
      policyEvaluator: { async evaluate() { return { decision: 'require_approval' }; } },
    });
    await expect(executor.call(moduleId, input)).rejects.toMatchObject({ code: 'APPROVAL_DENIED' });
    executor.setPolicyEvaluator({ async evaluate() { return { decision: 'allow' }; } });
    await expect(executor.call(moduleId, { ...input, _approval_token: 'legacy-token' }))
      .rejects.toMatchObject({ code: 'APPROVAL_DENIED' });
    expect(pending).toHaveBeenCalledOnce();
    expect(resume).not.toHaveBeenCalled();
    expect(execute).not.toHaveBeenCalled();
  });

  it('hashes the approved JSON-shaped values with UTF-16 ordering and signed-zero preservation', async () => {
    const registry = new Registry();
    const echo = (args: Record<string, unknown>) => args;
    await registry.register(moduleId, {
      description: 'Return the supplied JSON values', inputSchema: { type: 'object' }, outputSchema: { type: 'object' }, execute: echo,
    });
    const approval = vi.fn(async (_request: ApprovalRequest) => createApprovalResult({ status: 'approved' }));
    const executor = new Executor({
      registry,
      approvalHandler: { requestApproval: approval, checkApproval: async () => createApprovalResult({ status: 'rejected' }) },
      policyEvaluator: { async evaluate() { return { decision: 'require_approval' }; } },
    });
    const result = await executor.call(moduleId, { '2': 7, '10': -0, '\u{10400}': 0.25, '\uE000': true });
    const expected = createHash('sha256')
      .update('{"10":f8000000000000000;,"2":i7;,"\u{10400}":f3fd0000000000000;,"\uE000":true}')
      .digest('hex');
    expect(approval.mock.lastCall?.[0].canonicalArgsDigest).toBe(expected);
    expect(Object.is(result['10'], -0)).toBe(true);
  });

  it.each(['call', 'streamWithCompletion'] as const)('bounds evaluator work in %s by the invocation deadline', async (entry) => {
    vi.useFakeTimers();
    try {
      const { registry, execute } = await makeRegistry();
      let signal: AbortSignal | undefined;
      const executor = new Executor({ registry, policyEvaluator: {
        async evaluate(request) {
          signal = request.signal;
          await new Promise<void>(resolve => signal?.addEventListener('abort', () => resolve(), { once: true }));
          return { decision: 'allow' };
        },
      } });
      const context = Context.create(null, null, null, {}, null, (Date.now() + 25) / 1000);
      const stream = entry === 'streamWithCompletion' ? executor.streamWithCompletion(moduleId, input, context) : null;
      const result = expect(stream ? stream.chunks.next() : executor.call(moduleId, input, context)).rejects.toMatchObject({
        code: 'POLICY_EVALUATION_ERROR', retryable: true, userFixable: false,
      });
      await vi.advanceTimersByTimeAsync(25);
      await result;
      if (stream) await expect(stream.completion).resolves.toMatchObject({ reason: 'failed', errorCode: 'POLICY_EVALUATION_ERROR' });
      expect(signal?.aborted).toBe(true);
      expect(execute).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
