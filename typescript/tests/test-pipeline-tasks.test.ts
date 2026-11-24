/**
 * Tests for pipeline tasks: executor-refactor (strategy option),
 * preset-strategies, call-with-trace, and introspection.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Context } from '../src/context.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { FunctionModule } from '../src/decorator.js';
import { ExecutionStrategy, StrategyNotFoundError } from '../src/pipeline.js';
import { InvalidInputError } from '../src/errors.js';
import {
  buildStandardStrategy,
  buildInternalStrategy,
  buildTestingStrategy,
  buildPerformanceStrategy,
  BuiltinContextCreation,
  BuiltinModuleLookup,
  BuiltinExecute,
  BuiltinReturnResult,
} from '../src/builtin-steps.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import type { StandardStrategyDeps } from '../src/builtin-steps.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeRegistry(): Registry {
  const reg = new Registry();
  const mod = new FunctionModule({
    execute: (inputs) => ({ greeting: `Hello, ${inputs['name'] ?? 'world'}!` }),
    moduleId: 'test.greet',
    inputSchema: Type.Object({ name: Type.Optional(Type.String()) }),
    outputSchema: Type.Object({ greeting: Type.String() }),
    description: 'Greet module',
  });
  reg.register('test.greet', mod);
  return reg;
}

function makeDeps(registry: Registry): StandardStrategyDeps {
  return {
    config: null,
    registry,
    acl: null,
    approvalHandler: null,
    middlewareManager: new MiddlewareManager(),
  };
}

// ---------------------------------------------------------------------------
// Task 1: executor-refactor -- strategy option in constructor
// ---------------------------------------------------------------------------

describe('Executor strategy option', () => {
  let registry: Registry;

  beforeEach(() => {
    registry = makeRegistry();
  });

  it('accepts null strategy and defaults to standard', async () => {
    const executor = new Executor({ registry, strategy: null });
    expect(executor.currentStrategy).not.toBeNull();
    expect(executor.currentStrategy.name).toBe('standard');
    const result = await executor.call('test.greet', { name: 'Alice' });
    expect(result['greeting']).toBe('Hello, Alice!');
  });

  it('accepts undefined strategy (default) and uses standard', async () => {
    const executor = new Executor({ registry });
    expect(executor.currentStrategy).not.toBeNull();
    expect(executor.currentStrategy.name).toBe('standard');
    const result = await executor.call('test.greet', { name: 'Bob' });
    expect(result['greeting']).toBe('Hello, Bob!');
  });

  it('accepts an ExecutionStrategy instance', () => {
    const deps = makeDeps(registry);
    const strategy = buildStandardStrategy(deps);
    const executor = new Executor({ registry, strategy });
    expect(executor.currentStrategy).toBe(strategy);
    expect(executor.currentStrategy!.name).toBe('standard');
  });

  it('resolves strategy by builtin name string "standard"', () => {
    const executor = new Executor({ registry, strategy: 'standard' });
    expect(executor.currentStrategy).not.toBeNull();
    expect(executor.currentStrategy!.name).toBe('standard');
  });

  it('resolves strategy by builtin name string "testing"', () => {
    const executor = new Executor({ registry, strategy: 'testing' });
    expect(executor.currentStrategy!.name).toBe('testing');
  });

  it('resolves strategy by builtin name string "internal"', () => {
    const executor = new Executor({ registry, strategy: 'internal' });
    expect(executor.currentStrategy!.name).toBe('internal');
  });

  it('resolves strategy by builtin name string "performance"', () => {
    const executor = new Executor({ registry, strategy: 'performance' });
    expect(executor.currentStrategy!.name).toBe('performance');
  });

  it('resolves strategy from static registry', () => {
    const deps = makeDeps(registry);
    const custom = buildStandardStrategy(deps);
    // Give it a custom name by creating a new strategy
    const customStrategy = new ExecutionStrategy('my-custom', [
      new BuiltinContextCreation(null),
      new BuiltinModuleLookup(registry),
      new BuiltinExecute(null),
      new BuiltinReturnResult(),
    ]);
    Executor.registerStrategy('my-custom', customStrategy);
    try {
      const executor = new Executor({ registry, strategy: 'my-custom' });
      expect(executor.currentStrategy).toBe(customStrategy);
    } finally {
      // Clean up to not affect other tests -- use listStrategies to confirm
      // (no unregister API, but static map is shared)
    }
  });

  it('throws StrategyNotFoundError for unknown string', () => {
    expect(() => new Executor({ registry, strategy: 'nonexistent' }))
      .toThrow(StrategyNotFoundError);
  });
});

// ---------------------------------------------------------------------------
// Task 2: preset-strategies
// ---------------------------------------------------------------------------

describe('Preset strategy factories', () => {
  let deps: StandardStrategyDeps;

  beforeEach(() => {
    deps = makeDeps(makeRegistry());
  });

  it('buildStandardStrategy creates 11-step strategy with toggle inlined into module_lookup', () => {
    const strategy = buildStandardStrategy(deps);
    expect(strategy.name).toBe('standard');
    // PROTOCOL_SPEC §5 11-step pipeline; toggle check is inlined into
    // BuiltinModuleLookup rather than a separate step (sync A-D-011).
    expect(strategy.steps.length).toBe(11);
    expect(strategy.stepNames()).toContain('context_creation');
    expect(strategy.stepNames()).not.toContain('toggle_gate');
    expect(strategy.stepNames()).toContain('module_lookup');
    expect(strategy.stepNames()).toContain('acl_check');
    expect(strategy.stepNames()).toContain('approval_gate');
    expect(strategy.stepNames()).toContain('output_validation');
  });

  it('buildInternalStrategy skips ACL and approval', () => {
    const strategy = buildInternalStrategy(deps);
    expect(strategy.name).toBe('internal');
    expect(strategy.stepNames()).not.toContain('acl_check');
    expect(strategy.stepNames()).not.toContain('approval_gate');
    expect(strategy.stepNames()).toContain('context_creation');
    expect(strategy.stepNames()).toContain('module_lookup');
    expect(strategy.stepNames()).toContain('execute');
    expect(strategy.stepNames()).toContain('return_result');
  });

  it('buildTestingStrategy removes acl, approval, and call chain guard (8 steps)', () => {
    const strategy = buildTestingStrategy(deps);
    expect(strategy.name).toBe('testing');
    expect(strategy.steps.length).toBe(8);
    expect(strategy.stepNames()).toEqual([
      'context_creation',
      'module_lookup',
      'middleware_before',
      'input_validation',
      'execute',
      'output_validation',
      'middleware_after',
      'return_result',
    ]);
  });

  it('buildPerformanceStrategy skips middleware before and after', () => {
    const strategy = buildPerformanceStrategy(deps);
    expect(strategy.name).toBe('performance');
    expect(strategy.stepNames()).not.toContain('middleware_before');
    expect(strategy.stepNames()).not.toContain('middleware_after');
    expect(strategy.stepNames()).toContain('acl_check');
    expect(strategy.stepNames()).toContain('input_validation');
    expect(strategy.stepNames()).toContain('execute');
  });
});

// ---------------------------------------------------------------------------
// Task 3: call-with-trace
// ---------------------------------------------------------------------------

describe('Executor.callWithTrace', () => {
  let registry: Registry;

  beforeEach(() => {
    registry = makeRegistry();
  });

  it('returns output and trace with the testing strategy', async () => {
    const executor = new Executor({ registry, strategy: 'testing' });
    const [output, trace] = await executor.callWithTrace('test.greet', { name: 'Trace' });
    expect(output['greeting']).toBe('Hello, Trace!');
    expect(trace.moduleId).toBe('test.greet');
    expect(trace.strategyName).toBe('testing');
    expect(trace.success).toBe(true);
    expect(trace.totalDurationMs).toBeGreaterThanOrEqual(0);
    expect(trace.steps.length).toBeGreaterThan(0);
  });

  it('returns trace with step details', async () => {
    const executor = new Executor({ registry, strategy: 'testing' });
    const [, trace] = await executor.callWithTrace('test.greet', {});
    const stepNames = trace.steps.map((s) => s.name);
    expect(stepNames).toContain('context_creation');
    expect(stepNames).toContain('module_lookup');
    expect(stepNames).toContain('execute');
    expect(stepNames).toContain('return_result');
    for (const step of trace.steps) {
      expect(step.durationMs).toBeGreaterThanOrEqual(0);
      expect(step.skipped).toBe(false);
    }
  });

  it('accepts strategy override via options', async () => {
    const executor = new Executor({ registry, strategy: 'testing' });
    const deps = makeDeps(registry);
    const stdStrategy = buildStandardStrategy(deps);
    const [output, trace] = await executor.callWithTrace('test.greet', { name: 'Override' }, null, { strategy: stdStrategy });
    expect(output['greeting']).toBe('Hello, Override!');
    expect(trace.strategyName).toBe('standard');
    expect(trace.steps.length).toBe(11);
  });

  it('uses default standard strategy when none explicitly set', async () => {
    const executor = new Executor({ registry });
    // Now always has a strategy (defaults to standard)
    const [output, trace] = await executor.callWithTrace('test.greet', {});
    expect(output['greeting']).toBe('Hello, world!');
    expect(trace.strategyName).toBe('standard');
  });

  it('works with null inputs', async () => {
    const executor = new Executor({ registry, strategy: 'testing' });
    const [output, trace] = await executor.callWithTrace('test.greet', null);
    expect(output['greeting']).toBe('Hello, world!');
    expect(trace.success).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Task 4: introspection
// ---------------------------------------------------------------------------

describe('Executor introspection', () => {
  let registry: Registry;

  afterEach(() => {
    // Clean up static registry between tests
    // We cannot directly clear, but we can overwrite with empty values
  });

  beforeEach(() => {
    registry = makeRegistry();
  });

  it('registerStrategy and listStrategies work together', () => {
    const deps = makeDeps(registry);
    const strategy = buildTestingStrategy(deps);
    Executor.registerStrategy('test-intro', strategy);
    const infos = Executor.listStrategies();
    expect(infos.every(s => typeof s.name === 'string')).toBe(true);
    expect(infos.length).toBeGreaterThan(0);
  });

  // core-executor.md "Contract: Executor.list_strategies": one entry for the
  // executor's current strategy plus one per registered strategy. The static
  // overload cannot satisfy it — no built-in is seeded into the static
  // registry, so a default-constructed Executor reported [] where
  // apcore-python reported one entry, and docs/features/execution-pipeline.md
  // (Introspection) documents this call as returning the current strategy.
  it('instance listStrategies() includes the executor current strategy', () => {
    const executor = new Executor({ registry });
    const names = executor.listStrategies().map((s) => s.name);
    expect(names[0]).toBe('standard');
    expect(names).toContain('standard');
  });

  it('instance listStrategies() appends registered strategies without duplicating the current one', () => {
    const deps = makeDeps(registry);
    Executor.registerStrategy('intro-extra', buildTestingStrategy(deps));
    // Registering under the current strategy's own name must not produce a
    // second entry: apcore-python dedupes by name and so does this.
    Executor.registerStrategy('standard', buildStandardStrategy(deps));

    const executor = new Executor({ registry });
    const names = executor.listStrategies().map((s) => s.name);

    expect(names[0]).toBe('standard');
    expect(names).toContain('testing');
    expect(names.filter((n) => n === 'standard')).toHaveLength(1);
  });

  it('describePipeline returns StrategyInfo for current strategy', () => {
    const executor = new Executor({ registry, strategy: 'testing' });
    const info = executor.describePipeline();
    expect(info).not.toBeNull();
    expect(info!.name).toBe('testing');
    expect(info!.stepCount).toBe(8);
    expect(info!.stepNames).toEqual([
      'context_creation',
      'module_lookup',
      'middleware_before',
      'input_validation',
      'execute',
      'output_validation',
      'middleware_after',
      'return_result',
    ]);
    expect(info!.description).toContain('context_creation');
  });

  it('describePipeline returns strategy info for default strategy', () => {
    const executor = new Executor({ registry });
    const info = executor.describePipeline();
    expect(info).not.toBeNull();
    expect(info!.name).toBe('standard');
  });

  it('describePipeline returns info for the current strategy', () => {
    const deps = makeDeps(registry);
    const strategy = buildInternalStrategy(deps);
    const executor = new Executor({ registry, strategy });
    const info = executor.describePipeline();
    expect(info).not.toBeNull();
    expect(info.name).toBe('internal');
  });

  it('currentStrategy getter returns the configured strategy', () => {
    const executor = new Executor({ registry, strategy: 'standard' });
    expect(executor.currentStrategy).not.toBeNull();
    expect(executor.currentStrategy!.name).toBe('standard');
  });

  it('currentStrategy getter returns standard for default mode', () => {
    const executor = new Executor({ registry });
    expect(executor.currentStrategy).not.toBeNull();
    expect(executor.currentStrategy.name).toBe('standard');
  });
});
