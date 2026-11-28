import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Type } from '@sinclair/typebox';
import { describe, expect, it } from 'vitest';
import { CallBudget, Context, Executor, ModuleError, Registry } from '../src/index.js';
import { findFixturesRoot } from './spec-repo.js';

interface Case {
  id: string;
  module_id: string;
  max_calls: number | null;
  max_cost: number | null;
  estimated_cost: number;
  calls: Array<{ actual_cost: number | null }>;
  expected: Array<{ calls_used: number; calls_reserved: number; cost_used: number; error_code: string | null }>;
}

const fixture = JSON.parse(readFileSync(join(findFixturesRoot(), 'call_budget.json'), 'utf8')) as {
  test_cases: Case[];
};

describe('call_budget.json', () => {
  it.each(fixture.test_cases)('$id', async testCase => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    await registry.register(testCase.module_id, {
      description: 'Settle the actual cost when supplied',
      inputSchema: Type.Object({ actual_cost: Type.Optional(Type.Integer({ minimum: 0 })) }),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      execute(inputs: Record<string, unknown>, context: Context) {
        const actual = inputs.actual_cost;
        if (typeof actual === 'number') context.recordCost(actual);
        return { ok: true };
      },
    });
    const budget = new CallBudget({
      maxCalls: testCase.max_calls ?? undefined,
      maxCost: testCase.max_cost ?? undefined,
      estimatedCosts: { [testCase.module_id]: testCase.estimated_cost },
    });
    const context = Context.create().withBudget(budget);

    expect(testCase.expected).toHaveLength(testCase.calls.length);
    for (const [index, { actual_cost }] of testCase.calls.entries()) {
      const expected = testCase.expected[index];
      const inputs = actual_cost === null ? {} : { actual_cost };
      const invocation = executor.call(testCase.module_id, inputs, context);
      if (expected.error_code === null) {
        await expect(invocation).resolves.toEqual({ ok: true });
      } else {
        await expect(invocation).rejects.toMatchObject({ code: expected.error_code });
        await expect(invocation).rejects.toBeInstanceOf(ModuleError);
      }
      expect(budget.snapshot()).toEqual({
        callsUsed: expected.calls_used,
        callsReserved: expected.calls_reserved,
        costUsed: expected.cost_used,
      });
    }
  });
});
