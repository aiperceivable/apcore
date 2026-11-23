/**
 * Drive `pipeline_section_wiring.json` — §5.16 requirements 6 and 7 (#118 D-72).
 *
 * Every case goes through `new APCore({ config: new Config(document) })`. That
 * is the whole point of the fixture, and it is why the three fixtures that
 * already cover the pipeline builder could not have caught this: they hand the
 * section straight to `buildStrategyFromConfig(section, …)`, whose first
 * parameter is an object the CALLER supplies. Nothing extracted that object
 * from a `Config`, so `pipeline: remove: [acl_check]` left all eleven steps in
 * place while every builder fixture stayed green.
 *
 * A driver here that called the builder directly would reproduce the defect and
 * pass.
 */

import fs from 'node:fs';
import path from 'node:path';

import { describe, it, expect, vi } from 'vitest';

import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { registerStepType, unregisterStepType } from '../src/pipeline-config.js';
import type { Step, StepResult, PipelineContext } from '../src/pipeline.js';
import { findFixturesRoot } from './spec-repo.js';

interface WiringCase {
  readonly id: string;
  readonly description: string;
  readonly input: {
    readonly register_step_type?: string;
    readonly config: Record<string, unknown>;
  };
  readonly expected: {
    readonly steps: readonly string[];
    readonly security_step_warning: boolean;
    readonly warning_names_step?: string;
    readonly configured_step?: { name: string; field: string; value: unknown };
  };
}

interface Fixture {
  readonly description: string;
  readonly test_cases: readonly WiringCase[];
}

const fixture: Fixture = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'pipeline_section_wiring.json'), 'utf-8'),
);

function probeStep(name: string): Step {
  return {
    name,
    description: 'No-op step registered for the pipeline_section_wiring fixture.',
    async execute(_ctx: PipelineContext): Promise<StepResult> {
      return { action: 'continue' };
    },
  } as unknown as Step;
}

describe('pipeline_section_wiring.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, () => {
      const typeName = testCase.input.register_step_type;
      if (typeName !== undefined) {
        registerStepType(typeName, () => probeStep(typeName));
      }
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      let steps: Step[];
      let hits: string[];
      try {
        const client = new APCore({
          config: new Config({ ...testCase.input.config }),
        });
        steps = [...client.executor.currentStrategy.steps];
        // Read the calls BEFORE restoring: `mockRestore` clears them, and doing
        // it the other way round reported zero warnings for a case that warned.
        hits = warn.mock.calls
          .map((c) => String(c[0]))
          .filter((line) => line.includes('pipeline.remove'));
      } finally {
        warn.mockRestore();
        if (typeName !== undefined) unregisterStepType(typeName);
      }

      expect(steps.map((s) => s.name)).toEqual([...testCase.expected.steps]);

      const configured = testCase.expected.configured_step;
      if (configured !== undefined) {
        const target = steps.find((s) => s.name === configured.name)!;
        // The fixture's field names are snake_case; this SDK spells them camel.
        const camel = configured.field.replace(/_([a-z])/g, (_m, c: string) => c.toUpperCase());
        expect((target as unknown as Record<string, unknown>)[camel]).toEqual(configured.value);
      }

      if (testCase.expected.security_step_warning) {
        // Once per configuration load, per §9.2.2's cadence — not once per step.
        expect(hits.length).toBe(1);
        expect(hits[0]).toContain(testCase.expected.warning_names_step);
      } else {
        expect(hits).toEqual([]);
      }
    });
  }
});
