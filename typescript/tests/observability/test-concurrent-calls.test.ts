import { Type } from '@sinclair/typebox';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { APCore } from '../../src/full.js';
import { Context } from '../../src/context.js';
import { MetricsCollector, MetricsMiddleware } from '../../src/observability/metrics.js';
import { InMemoryExporter, TracingMiddleware } from '../../src/observability/tracing.js';
import { TraceContext } from '../../src/trace-context.js';

function signal(): { ready: Promise<void>; send: () => void } {
  let send!: () => void;
  const ready = new Promise<void>((resolve) => { send = resolve; });
  return { ready, send };
}

describe('observability of concurrent module calls', () => {
  afterEach(() => { vi.restoreAllMocks(); });

  it.each([['a', 15, 20], ['b', 25, 10]] as const)(
    'attributes durations and spans to the right caller when %s completes first',
    async (first, durationA, durationB) => {
      const epoch = Date.now();
      let clock = 100;
      vi.spyOn(Date, 'now').mockImplementation(() => epoch + clock);
      vi.spyOn(performance, 'now').mockImplementation(() => clock);
      const client = new APCore();
      const metrics = new MetricsCollector();
      const exporter = new InMemoryExporter();
      client.use(new MetricsMiddleware(metrics));
      client.use(new TracingMiddleware(exporter));
      const sibling = { a: { started: signal(), release: signal() }, b: { started: signal(), release: signal() } };
      const injected = new Map<string, string>();
      const schema = Type.Object({});
      for (const name of ['a', 'b'] as const) {
        await client.module({
          id: `demo.${name}`,
          description: `Wait for sibling ${name}`,
          inputSchema: schema,
          outputSchema: schema,
          execute: async (_inputs, context) => {
            sibling[name].started.send();
            await sibling[name].release.ready;
            injected.set(name, TraceContext.inject(context).traceparent.split('-')[2]);
            return {};
          },
        }).registration;
      }
      await client.module({
        id: 'demo.parent',
        description: 'Launch two siblings',
        inputSchema: schema,
        outputSchema: schema,
        execute: async (_inputs, context) => {
          const a = client.call('demo.a', {}, context);
          await sibling.a.started.ready;
          clock = 105;
          const b = client.call('demo.b', {}, context);
          await sibling.b.started.ready;
          const calls = { a, b };
          const second = first === 'a' ? 'b' : 'a';
          clock = 115;
          sibling[first].release.send();
          await calls[first];
          clock = 125;
          sibling[second].release.send();
          await calls[second];
          return {};
        },
      }).registration;

      await client.call('demo.parent', {}, Context.create());

      const rendered = metrics.exportPrometheus();
      expect(rendered).toContain(`apcore_module_duration_seconds_sum{module_id="demo.a"} ${durationA / 1000}\n`);
      expect(rendered).toContain(`apcore_module_duration_seconds_sum{module_id="demo.b"} ${durationB / 1000}\n`);
      const spans = new Map(exporter.getSpans().map((span) => [span.attributes.module_id, span]));
      const parent = spans.get('demo.parent')!;
      for (const name of ['a', 'b']) {
        const child = spans.get(`demo.${name}`)!;
        expect(child.parentSpanId).toBe(parent.spanId);
        expect(injected.get(name)).toBe(child.spanId);
      }
    },
  );
});
