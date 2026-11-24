/**
 * Tests for Pipeline fail-fast configuration (Issue #33 §1.2 and §2.1).
 *
 * §1.2 — YAML pipeline config that references a non-existent step in
 *        `remove`, `configure`, `after`, or `before` MUST throw a
 *        `ConfigurationError` rather than warn-and-continue.
 *
 * §2.1 — Constructing an `ExecutionStrategy` whose step `requires` are not
 *        satisfied by a preceding step's `provides` MUST throw a
 *        `PipelineDependencyError` rather than warn.
 */

import { describe, expect, it } from 'vitest';
import {
  ConfigurationError,
  buildStrategyFromConfig,
  registerStepType,
  unregisterStepType,
} from '../src/pipeline-config.js';
import { ExecutionStrategy, PipelineDependencyError } from '../src/pipeline.js';
import type { Step, StepResult } from '../src/pipeline.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeStep(name: string, opts: { requires?: string[]; provides?: string[] } = {}): Step {
  return {
    name,
    description: `Step ${name}`,
    removable: true,
    replaceable: true,
    requires: opts.requires,
    provides: opts.provides,
    execute: async (): Promise<StepResult> => ({ action: 'continue' }),
  };
}

function makeFakeDeps() {
  return {
    registry: { get: () => null } as any,
    acl: null as any,
    config: { get: () => undefined } as any,
    middlewareManager: null as any,
    approvalHandler: null as any,
  };
}

// ---------------------------------------------------------------------------
// §2.1 — PipelineDependencyError on construction
// ---------------------------------------------------------------------------

describe('ExecutionStrategy unmet dependency (§2.1)', () => {
  it('throws PipelineDependencyError when requires is not provided by any preceding step', () => {
    expect(() => new ExecutionStrategy('bad', [makeStep('a', { requires: ['output'] })])).toThrow(
      PipelineDependencyError,
    );
  });

  it('error message names the offending step and missing field', () => {
    try {
      new ExecutionStrategy('bad', [
        makeStep('first'),
        makeStep('second', { requires: ['validated_inputs'] }),
      ]);
    } catch (e) {
      expect(e).toBeInstanceOf(PipelineDependencyError);
      const msg = (e as Error).message;
      expect(msg).toContain('second');
      expect(msg).toContain('validated_inputs');
    }
  });

  it('does NOT throw when requires is satisfied by a preceding provides', () => {
    expect(
      () =>
        new ExecutionStrategy('ok', [
          makeStep('a', { provides: ['output'] }),
          makeStep('b', { requires: ['output'] }),
        ]),
    ).not.toThrow();
  });

  it('throws even when the requirement appears later in the strategy', () => {
    // 'b' requires output but 'a' (preceding) doesn't provide it; 'c' provides
    // it but comes after 'b'.
    expect(
      () =>
        new ExecutionStrategy('bad', [
          makeStep('a'),
          makeStep('b', { requires: ['output'] }),
          makeStep('c', { provides: ['output'] }),
        ]),
    ).toThrow(PipelineDependencyError);
  });

  it('PipelineDependencyError carries stepName and missingRequires', () => {
    try {
      new ExecutionStrategy('bad', [makeStep('only', { requires: ['ctx', 'output'] })]);
    } catch (e) {
      expect(e).toBeInstanceOf(PipelineDependencyError);
      const err = e as PipelineDependencyError;
      expect(err.stepName).toBe('only');
      expect(err.missingRequires).toContain('output');
    }
  });
});

// ---------------------------------------------------------------------------
// §1.2 — ConfigurationError on missing step references
// ---------------------------------------------------------------------------

describe('buildStrategyFromConfig fail-fast (§1.2)', () => {
  it('throws ConfigurationError when remove targets a nonexistent step', async () => {
    await expect(
      buildStrategyFromConfig({ remove: ['nonexistent_step_xyz'] }, makeFakeDeps()),
    ).rejects.toThrow(ConfigurationError);
  });

  it('throws ConfigurationError when configure targets a nonexistent step', async () => {
    await expect(
      buildStrategyFromConfig(
        { configure: { nonexistent_step_xyz: { ignoreErrors: true } } },
        makeFakeDeps(),
      ),
    ).rejects.toThrow(ConfigurationError);
  });

  it('throws ConfigurationError when steps[].after points at a nonexistent anchor', async () => {
    const TYPE = '__failfast_after';
    registerStepType(TYPE, () => ({
      name: 'custom',
      description: 'c',
      removable: true,
      replaceable: true,
      execute: async () => ({ action: 'continue' as const }),
    }));
    try {
      await expect(
        buildStrategyFromConfig(
          {
            steps: [{ type: TYPE, name: 'custom', after: 'nonexistent_anchor_xyz' }],
          },
          makeFakeDeps(),
        ),
      ).rejects.toThrow(ConfigurationError);
    } finally {
      unregisterStepType(TYPE);
    }
  });

  it('throws ConfigurationError when steps[].before points at a nonexistent anchor', async () => {
    const TYPE = '__failfast_before';
    registerStepType(TYPE, () => ({
      name: 'custom',
      description: 'c',
      removable: true,
      replaceable: true,
      execute: async () => ({ action: 'continue' as const }),
    }));
    try {
      await expect(
        buildStrategyFromConfig(
          {
            steps: [{ type: TYPE, name: 'custom', before: 'nonexistent_anchor_xyz' }],
          },
          makeFakeDeps(),
        ),
      ).rejects.toThrow(ConfigurationError);
    } finally {
      unregisterStepType(TYPE);
    }
  });

  it('throws ConfigurationError when a steps[] entry has neither after nor before', async () => {
    const TYPE = '__failfast_neither';
    registerStepType(TYPE, () => ({
      name: 'custom',
      description: 'c',
      removable: true,
      replaceable: true,
      execute: async () => ({ action: 'continue' as const }),
    }));
    try {
      await expect(
        buildStrategyFromConfig({ steps: [{ type: TYPE, name: 'custom' }] }, makeFakeDeps()),
      ).rejects.toThrow(ConfigurationError);
    } finally {
      unregisterStepType(TYPE);
    }
  });
});
