/**
 * PROTOCOL_SPEC §5.16 requirements 6 and 7 — a configured `pipeline:` section.
 *
 * The section was accepted, validated and then ignored, in all three SDKs
 * (apcore#118, decision D-72). `buildStrategyFromConfig` existed everywhere and
 * its first parameter was an object the CALLER supplied; nothing extracted that
 * object from a loaded `Config`, so `pipeline: remove: [acl_check]` left all
 * eleven steps in place and a declared custom step silently never ran.
 *
 * The asymmetry is why these cases exist. Failing to *remove* a step is
 * fail-safe; failing to *insert* one is not — a declared audit, rate-limit or
 * authorization step that never runs is invisible from inside the running
 * system, and the pipeline an operator reads in configuration is not the
 * pipeline that executes.
 */

import { describe, it, expect, vi } from 'vitest';

import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import { buildStandardStrategy } from '../src/builtin-steps.js';

const DEFAULT = [
  'context_creation',
  'call_chain_guard',
  'module_lookup',
  'acl_check',
  'approval_gate',
  'middleware_before',
  'input_validation',
  'execute',
  'output_validation',
  'middleware_after',
  'return_result',
];

function steps(section?: Record<string, unknown>): string[] {
  const doc: Record<string, unknown> = { version: '1.0', project: { name: 'pipeline-probe' } };
  if (section !== undefined) doc['pipeline'] = section;
  return new APCore({ config: new Config(doc) }).executor.currentStrategy.steps.map((s) => s.name);
}

describe('a configured pipeline section is applied', () => {
  it('leaves the default pipeline when nothing is configured', () => {
    // The half that keeps this additive for everyone who configures nothing.
    expect(steps()).toEqual(DEFAULT);
    expect(steps({})).toEqual(DEFAULT);
  });

  it('removes the named step', () => {
    expect(steps({ remove: ['output_validation'] })).not.toContain('output_validation');
  });

  it('configures a field of an existing step without reordering', () => {
    const doc = {
      version: '1.0',
      project: { name: 'pipeline-probe' },
      pipeline: { configure: { input_validation: { ignore_errors: true } } },
    };
    const strategy = new APCore({ config: new Config(doc) }).executor.currentStrategy;
    const step = strategy.steps.find((s) => s.name === 'input_validation')!;
    expect((step as unknown as { ignoreErrors: boolean }).ignoreErrors).toBe(true);
    expect(strategy.steps.map((s) => s.name)).toEqual(DEFAULT);
  });

  it('an explicit strategy still wins over the configured section', () => {
    // D-73's precedence: an API argument beats `Config`. Without this the
    // wiring would take a caller's hand-built strategy away from them whenever
    // a `pipeline:` section happened to be present.
    const registry = new Registry();
    const executor = new Executor({
      registry,
      config: new Config({
        version: '1.0',
        project: { name: 'pipeline-probe' },
        pipeline: { remove: ['acl_check'] },
      }),
      strategy: buildStandardStrategy({
        config: null,
        registry,
        acl: null,
        approvalHandler: null,
        middlewareManager: new MiddlewareManager(),
      }),
    });
    expect(executor.currentStrategy.steps.map((s) => s.name)).toContain('acl_check');
  });
});

describe('removing a security step warns', () => {
  it.each(['acl_check', 'approval_gate'])('warns for %s', (step) => {
    // §5.16 requirement 7 — the transition, not the steady state. Requirement 6
    // makes a previously ignored section take effect, so a config that has been
    // carrying `remove: [acl_check]` while ACL was enforced anyway starts
    // having ACL genuinely removed.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const names = steps({ remove: [step] });
    const hits = warn.mock.calls.filter((c) => String(c[0]).includes('pipeline.remove'));
    warn.mockRestore();
    expect(names).not.toContain(step);
    expect(hits.length).toBe(1);
    expect(String(hits[0]?.[0])).toContain(step);
  });

  it('is silent for an ordinary step', () => {
    // The other half: the notice is about protections, not every removal.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    steps({ remove: ['output_validation'] });
    const hits = warn.mock.calls.filter((c) => String(c[0]).includes('pipeline.remove'));
    warn.mockRestore();
    expect(hits.length).toBe(0);
  });

  it('reports a handler step it cannot insert synchronously', () => {
    // This SDK alone needs `await import()` for a `handler:` target, and the
    // Executor constructor is synchronous. Reported rather than dropped:
    // omitting it silently is the defect requirement 6 removes, and throwing
    // would stop a project from starting over a step that has never once run.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const names = steps({
      steps: [{ name: 'late', handler: 'some-module:makeStep', after: 'acl_check' }],
    });
    const hits = warn.mock.calls.filter((c) => String(c[0]).includes("'handler:'"));
    warn.mockRestore();
    expect(names).not.toContain('late');
    expect(hits.length).toBe(1);
    expect(String(hits[0]?.[0])).toContain('late');
  });
});
