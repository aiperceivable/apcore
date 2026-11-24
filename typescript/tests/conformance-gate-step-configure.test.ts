/**
 * Cross-language conformance driver for `gate_step_configure.json`
 * (PROTOCOL_SPEC 5.16.1, decision D-130).
 *
 * Fixture source: apcore/conformance/fixtures/gate_step_configure.json
 * (canonical). Its `driver_contract` block is the contract:
 *
 *  - `path`: the fixture's YAML goes through the same public config path
 *    `pipeline_failfast_config.json` uses — `buildStrategyFromConfig` — with
 *    the `pipeline` section passed through verbatim (snake_case, no key
 *    translation). Nothing is executed; a throw seen here can only have come
 *    from turning the configuration into a strategy.
 *  - `assert_the_wire_code`: `error_code` is asserted, not the class name.
 *  - `message`: every `error_message_contains` fragment must appear.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import { ACL } from '../src/acl.js';
import { ModuleError } from '../src/errors.js';
import { buildStrategyFromConfig } from '../src/pipeline-config.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

interface Case {
  id: string;
  note?: string;
  input: { yaml: { pipeline: Record<string, unknown> } };
  expected: { raises: boolean; error_code?: string; error_message_contains?: string[] };
}

const fixture: { test_cases: Case[]; driver_contract: Record<string, string> } = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'gate_step_configure.json'), 'utf-8'),
);

function deps(): Parameters<typeof buildStrategyFromConfig>[1] {
  return {
    config: null,
    registry: new Registry(),
    acl: new ACL([], 'deny'),
    approvalHandler: null,
    middlewareManager: new MiddlewareManager(),
  };
}

describe('conformance: gate_step_configure.json', () => {
  for (const testCase of fixture.test_cases) {
    it(testCase.id, async () => {
      let error: unknown = null;
      try {
        await buildStrategyFromConfig(testCase.input.yaml.pipeline, deps());
      } catch (err) {
        error = err;
      }
      expect(error !== null, `${testCase.id}: raised ${String(error)}`).toBe(
        testCase.expected.raises,
      );
      if (!testCase.expected.raises) return;
      expect(error).toBeInstanceOf(ModuleError);
      expect((error as ModuleError).code).toBe(testCase.expected.error_code);
      for (const fragment of testCase.expected.error_message_contains ?? []) {
        expect((error as Error).message).toContain(fragment);
      }
    });
  }

  it('honours every driver_contract rule', () => {
    expect(Object.keys(fixture.driver_contract).sort()).toEqual([
      'assert_the_wire_code',
      'message',
      'path',
    ]);
  });
});
