import { describe, it, expect } from 'vitest';
import { UsageCollector } from '../src/observability/usage.js';
import { UsageSummaryModule, UsageModule } from '../src/sys-modules/usage.js';
import { Registry } from '../src/registry/registry.js';
import { InvalidInputError, ModuleNotFoundError } from '../src/errors.js';

function createRegistry(): Registry {
  return new Registry();
}

function createCollectorWithData(): UsageCollector {
  const collector = new UsageCollector();
  collector.record('test.alpha', 'caller1', 50, true);
  collector.record('test.alpha', 'caller1', 120, true);
  collector.record('test.alpha', 'caller2', 200, false);
  collector.record('test.beta', 'caller1', 30, true);
  return collector;
}

describe('computeP99 (via UsageModule)', () => {
  it('returns 0 for empty latencies', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    // Register the module but record no usage data
    registry.registerInternal('test.empty', { description: 'empty module', execute: () => ({}) });
    const mod = new UsageModule(registry, collector);

    const result = mod.execute({ module_id: 'test.empty' }, {});
    expect(result['p99_latency_ms']).toBe(0);
  });

  it('returns correct p99 for single element', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    registry.registerInternal('test.single', { description: 'single', execute: () => ({}) });
    collector.record('test.single', 'c1', 42, true);

    const mod = new UsageModule(registry, collector);
    const result = mod.execute({ module_id: 'test.single' }, {});
    expect(result['p99_latency_ms']).toBe(42);
  });

  it('returns correct p99 for 100 elements', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    registry.registerInternal('test.many', { description: 'many', execute: () => ({}) });

    for (let i = 1; i <= 100; i++) {
      collector.record('test.many', 'c1', i, true);
    }

    const mod = new UsageModule(registry, collector);
    const result = mod.execute({ module_id: 'test.many' }, {});
    // p99 index = ceil(0.99 * 100) - 1 = 98, sorted[98] = 99
    expect(result['p99_latency_ms']).toBe(99);
  });
});

describe('padHourlyDistribution (via UsageModule)', () => {
  it('pads missing hours with zeros', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    registry.registerInternal('test.pad', { description: 'pad test', execute: () => ({}) });
    collector.record('test.pad', 'c1', 10, true);

    const mod = new UsageModule(registry, collector);
    const result = mod.execute({ module_id: 'test.pad' }, {});
    const hourly = result['hourly_distribution'] as Array<Record<string, unknown>>;

    // Should have exactly 24 entries
    expect(hourly).toHaveLength(24);

    // All entries should have hour, call_count, error_count
    for (const entry of hourly) {
      expect(entry).toHaveProperty('hour');
      expect(entry).toHaveProperty('call_count');
      expect(entry).toHaveProperty('error_count');
    }

    // Exactly 23 entries should have zero counts (1 has the recorded call)
    const zeroEntries = hourly.filter((e) => e['call_count'] === 0);
    expect(zeroEntries).toHaveLength(23);
  });

  it('includes existing hours with correct counts', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    registry.registerInternal('test.existing', { description: 'existing', execute: () => ({}) });

    collector.record('test.existing', 'c1', 10, true);
    collector.record('test.existing', 'c1', 20, false);

    const mod = new UsageModule(registry, collector);
    const result = mod.execute({ module_id: 'test.existing' }, {});
    const hourly = result['hourly_distribution'] as Array<Record<string, unknown>>;

    const nonZero = hourly.filter((e) => (e['call_count'] as number) > 0);
    expect(nonZero).toHaveLength(1);
    expect(nonZero[0]['call_count']).toBe(2);
    expect(nonZero[0]['error_count']).toBe(1);
  });
});

describe('UsageSummaryModule', () => {
  it('returns summary with period, total_calls, total_errors, and modules array', () => {
    const collector = createCollectorWithData();
    const mod = new UsageSummaryModule(collector);

    const result = mod.execute({}, {});
    expect(result).toHaveProperty('period', '24h');
    expect(result).toHaveProperty('total_calls');
    expect(result).toHaveProperty('total_errors');
    expect(result).toHaveProperty('modules');
    expect(result['total_calls']).toBe(4);
    expect(result['total_errors']).toBe(1);
    expect(Array.isArray(result['modules'])).toBe(true);
  });

  it('sorts modules by call_count descending', () => {
    const collector = createCollectorWithData();
    const mod = new UsageSummaryModule(collector);

    const result = mod.execute({}, {});
    const modules = result['modules'] as Array<Record<string, unknown>>;

    expect(modules).toHaveLength(2);
    expect(modules[0]['module_id']).toBe('test.alpha');
    expect(modules[0]['call_count']).toBe(3);
    expect(modules[1]['module_id']).toBe('test.beta');
    expect(modules[1]['call_count']).toBe(1);
  });

  it('includes trend field on each module entry', () => {
    const collector = createCollectorWithData();
    const mod = new UsageSummaryModule(collector);

    const result = mod.execute({}, {});
    const modules = result['modules'] as Array<Record<string, unknown>>;

    for (const m of modules) {
      expect(m).toHaveProperty('trend');
      expect(typeof m['trend']).toBe('string');
    }
  });
});

describe('UsageModule', () => {
  it('throws InvalidInputError when module_id is missing', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    const mod = new UsageModule(registry, collector);

    expect(() => mod.execute({}, {})).toThrow(InvalidInputError);
  });

  it('throws InvalidInputError when module_id is empty string', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    const mod = new UsageModule(registry, collector);

    expect(() => mod.execute({ module_id: '' }, {})).toThrow(InvalidInputError);
  });

  it('throws ModuleNotFoundError for unknown module', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    const mod = new UsageModule(registry, collector);

    expect(() => mod.execute({ module_id: 'no.such.module' }, {})).toThrow(ModuleNotFoundError);
  });

  it('returns detail with p99_latency_ms, callers breakdown, and hourly_distribution', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    registry.registerInternal('test.detail', { description: 'detail', execute: () => ({}) });

    collector.record('test.detail', 'caller1', 50, true);
    collector.record('test.detail', 'caller1', 100, true);
    collector.record('test.detail', 'caller2', 200, false);

    const mod = new UsageModule(registry, collector);
    const result = mod.execute({ module_id: 'test.detail' }, {});

    expect(result).toHaveProperty('module_id', 'test.detail');
    expect(result).toHaveProperty('period', '24h');
    expect(result).toHaveProperty('call_count', 3);
    expect(result).toHaveProperty('error_count', 1);
    expect(result).toHaveProperty('p99_latency_ms');
    expect(result['p99_latency_ms']).toBe(200);
    expect(result).toHaveProperty('trend');

    // Callers breakdown
    const callers = result['callers'] as Array<Record<string, unknown>>;
    expect(callers).toHaveLength(2);
    const c1 = callers.find((c) => c['caller_id'] === 'caller1');
    const c2 = callers.find((c) => c['caller_id'] === 'caller2');
    expect(c1).toBeDefined();
    expect(c1!['call_count']).toBe(2);
    expect(c1!['error_count']).toBe(0);
    expect(c2).toBeDefined();
    expect(c2!['call_count']).toBe(1);
    expect(c2!['error_count']).toBe(1);

    // Hourly distribution
    const hourly = result['hourly_distribution'] as Array<Record<string, unknown>>;
    expect(hourly).toHaveLength(24);
  });

  it('uses custom period parameter', () => {
    const registry = createRegistry();
    const collector = new UsageCollector();
    registry.registerInternal('test.period', { description: 'period', execute: () => ({}) });
    collector.record('test.period', 'c1', 10, true);

    const mod = new UsageModule(registry, collector);
    const result = mod.execute({ module_id: 'test.period', period: '1h' }, {});

    expect(result['period']).toBe('1h');
    expect(result['call_count']).toBe(1);
  });
});
