import { describe, it, expect } from 'vitest';
import { classifyHealthStatus, HealthSummaryModule, HealthModule } from '../src/sys-modules/health.js';
import { Registry } from '../src/registry/registry.js';
import { MetricsCollector } from '../src/observability/metrics.js';
import { ErrorHistory } from '../src/observability/error-history.js';
import { Config } from '../src/config.js';
import { ModuleError, InvalidInputError, ModuleNotFoundError } from '../src/errors.js';

function makeDummyModule(description: string = 'dummy') {
  return { description, execute: () => ({}) };
}

function makeRegistry(...moduleIds: string[]): Registry {
  const registry = new Registry();
  for (const id of moduleIds) {
    registry.registerInternal(id, makeDummyModule());
  }
  return registry;
}

describe('classifyHealthStatus', () => {
  it('returns unknown when totalCalls is 0', () => {
    expect(classifyHealthStatus(0, 0)).toBe('unknown');
  });

  it('returns healthy when errorRate < healthyThreshold', () => {
    expect(classifyHealthStatus(0.005, 100)).toBe('healthy');
    expect(classifyHealthStatus(0, 1)).toBe('healthy');
  });

  it('returns degraded when errorRate between thresholds', () => {
    expect(classifyHealthStatus(0.05, 100)).toBe('degraded');
    expect(classifyHealthStatus(0.01, 100)).toBe('degraded');
  });

  it('returns error when errorRate >= degradedThreshold', () => {
    expect(classifyHealthStatus(0.10, 100)).toBe('error');
    expect(classifyHealthStatus(0.50, 100)).toBe('error');
  });

  it('supports custom thresholds', () => {
    // With custom thresholds: healthy < 0.05, degraded < 0.20, error >= 0.20
    expect(classifyHealthStatus(0.04, 100, 0.05, 0.20)).toBe('healthy');
    expect(classifyHealthStatus(0.10, 100, 0.05, 0.20)).toBe('degraded');
    expect(classifyHealthStatus(0.25, 100, 0.05, 0.20)).toBe('error');
  });
});

describe('HealthSummaryModule', () => {
  it('returns summary with counts for healthy, degraded, error, and unknown', () => {
    const registry = makeRegistry('mod.healthy', 'mod.error', 'mod.unknown');
    const metrics = new MetricsCollector();
    const errorHistory = new ErrorHistory();

    // mod.healthy: 100 success, 0 errors => errorRate 0 => healthy
    for (let i = 0; i < 100; i++) {
      metrics.incrementCalls('mod.healthy', 'success');
    }

    // mod.error: 5 success, 95 errors => errorRate 0.95 => error
    for (let i = 0; i < 5; i++) {
      metrics.incrementCalls('mod.error', 'success');
    }
    for (let i = 0; i < 95; i++) {
      metrics.incrementCalls('mod.error', 'error');
    }

    // mod.unknown: no calls => unknown

    const mod = new HealthSummaryModule(registry, metrics, errorHistory);
    const result = mod.execute({}, null);

    const summary = result['summary'] as Record<string, number>;
    expect(summary['total_modules']).toBe(3);
    expect(summary['healthy']).toBe(1);
    expect(summary['error']).toBe(1);
    expect(summary['unknown']).toBe(1);
  });

  it('filters out healthy modules when include_healthy is false', () => {
    const registry = makeRegistry('mod.healthy', 'mod.error');
    const metrics = new MetricsCollector();
    const errorHistory = new ErrorHistory();

    metrics.incrementCalls('mod.healthy', 'success');
    for (let i = 0; i < 100; i++) {
      metrics.incrementCalls('mod.error', 'error');
    }

    const mod = new HealthSummaryModule(registry, metrics, errorHistory);
    const result = mod.execute({ include_healthy: false }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    const ids = modules.map((m) => m['module_id']);
    expect(ids).not.toContain('mod.healthy');
    expect(ids).toContain('mod.error');
  });

  it('filters out healthy modules when include_healthy is string "false"', () => {
    const registry = makeRegistry('mod.healthy', 'mod.error');
    const metrics = new MetricsCollector();
    const errorHistory = new ErrorHistory();

    metrics.incrementCalls('mod.healthy', 'success');
    for (let i = 0; i < 100; i++) {
      metrics.incrementCalls('mod.error', 'error');
    }

    const mod = new HealthSummaryModule(registry, metrics, errorHistory);
    const result = mod.execute({ include_healthy: 'false' }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    const ids = modules.map((m) => m['module_id']);
    expect(ids).not.toContain('mod.healthy');
    expect(ids).toContain('mod.error');
  });

  it('includes project name from config', () => {
    const registry = makeRegistry('mod.a');
    const metrics = new MetricsCollector();
    const errorHistory = new ErrorHistory();
    const config = new Config({ project: { name: 'my-project' } });

    const mod = new HealthSummaryModule(registry, metrics, errorHistory, config);
    const result = mod.execute({}, null);

    const project = result['project'] as Record<string, unknown>;
    expect(project['name']).toBe('my-project');
  });

  it('returns all unknown when MetricsCollector is null', () => {
    const registry = makeRegistry('mod.a', 'mod.b');
    const errorHistory = new ErrorHistory();

    const mod = new HealthSummaryModule(registry, null, errorHistory);
    const result = mod.execute({}, null);

    const summary = result['summary'] as Record<string, number>;
    expect(summary['unknown']).toBe(2);
    expect(summary['healthy']).toBe(0);
  });

  it('includes top_error for modules with errors', () => {
    const registry = makeRegistry('mod.a');
    const metrics = new MetricsCollector();
    const errorHistory = new ErrorHistory();

    metrics.incrementCalls('mod.a', 'error');
    errorHistory.record('mod.a', new ModuleError('SOME_ERR', 'something broke', {}, undefined, undefined, undefined, 'try restarting'));
    errorHistory.record('mod.a', new ModuleError('SOME_ERR', 'something broke'));

    const mod = new HealthSummaryModule(registry, metrics, errorHistory);
    const result = mod.execute({}, null);

    const modules = result['modules'] as Record<string, unknown>[];
    const modA = modules.find((m) => m['module_id'] === 'mod.a');
    expect(modA).toBeDefined();

    const topError = modA!['top_error'] as Record<string, unknown>;
    expect(topError).not.toBeNull();
    expect(topError['code']).toBe('SOME_ERR');
    expect(topError['message']).toBe('something broke');
    expect(topError['count']).toBe(2);
    expect(topError['ai_guidance']).toBe('try restarting');
  });
});

describe('HealthModule', () => {
  it('throws InvalidInputError when module_id is missing', () => {
    const registry = makeRegistry('mod.a');
    const errorHistory = new ErrorHistory();
    const mod = new HealthModule(registry, null, errorHistory);

    expect(() => mod.execute({}, null)).toThrow(InvalidInputError);
  });

  it('throws ModuleNotFoundError for unknown module', () => {
    const registry = makeRegistry('mod.a');
    const errorHistory = new ErrorHistory();
    const mod = new HealthModule(registry, null, errorHistory);

    expect(() => mod.execute({ module_id: 'mod.nonexistent' }, null)).toThrow(ModuleNotFoundError);
  });

  it('returns health data with status, total_calls, error_rate, and latency', () => {
    const registry = makeRegistry('mod.a');
    const metrics = new MetricsCollector();
    const errorHistory = new ErrorHistory();

    // 90 success + 10 error = 100 total, errorRate = 0.1 => 'error' status
    for (let i = 0; i < 90; i++) {
      metrics.incrementCalls('mod.a', 'success');
    }
    for (let i = 0; i < 10; i++) {
      metrics.incrementCalls('mod.a', 'error');
    }
    // Record some durations
    metrics.observeDuration('mod.a', 0.05);
    metrics.observeDuration('mod.a', 0.15);

    const mod = new HealthModule(registry, metrics, errorHistory);
    const result = mod.execute({ module_id: 'mod.a' }, null);

    expect(result['module_id']).toBe('mod.a');
    expect(result['status']).toBe('error');
    expect(result['total_calls']).toBe(100);
    expect(result['error_count']).toBe(10);
    expect(result['error_rate']).toBeCloseTo(0.1);
    // avg of 0.05s and 0.15s = 0.1s = 100ms
    expect(result['avg_latency_ms']).toBeCloseTo(100);
    expect(typeof result['p99_latency_ms']).toBe('number');
  });

  it('returns recent_errors from ErrorHistory', () => {
    const registry = makeRegistry('mod.a');
    const metrics = new MetricsCollector();
    const errorHistory = new ErrorHistory();

    metrics.incrementCalls('mod.a', 'success');

    errorHistory.record('mod.a', new ModuleError('ERR_ONE', 'first error', {}, undefined, undefined, undefined, 'guidance one'));
    errorHistory.record('mod.a', new ModuleError('ERR_TWO', 'second error'));

    const mod = new HealthModule(registry, metrics, errorHistory);
    const result = mod.execute({ module_id: 'mod.a' }, null);

    const recentErrors = result['recent_errors'] as Record<string, unknown>[];
    expect(recentErrors).toHaveLength(2);
    // newest first
    expect(recentErrors[0]['code']).toBe('ERR_TWO');
    expect(recentErrors[1]['code']).toBe('ERR_ONE');
    expect(recentErrors[1]['ai_guidance']).toBe('guidance one');
    expect(recentErrors[0]['count']).toBe(1);
  });

  it('returns zeros when MetricsCollector is null', () => {
    const registry = makeRegistry('mod.a');
    const errorHistory = new ErrorHistory();

    const mod = new HealthModule(registry, null, errorHistory);
    const result = mod.execute({ module_id: 'mod.a' }, null);

    expect(result['total_calls']).toBe(0);
    expect(result['error_count']).toBe(0);
    expect(result['error_rate']).toBe(0);
    expect(result['avg_latency_ms']).toBe(0);
    expect(result['p99_latency_ms']).toBe(0);
    expect(result['status']).toBe('unknown');
  });
});
