/**
 * Cross-language conformance driver for error_recovery_metadata.json.
 *
 * Fixture source: apcore/conformance/fixtures/error_recovery_metadata.json
 * (single source of truth). See that fixture's `description` for the contract.
 *
 * Mirrors apcore-python tests/test_conformance.py::test_error_recovery_user_fixable
 * and ::test_error_recovery_fixture_matches_source.
 *
 * Per the fixture's `driver_contract.construction`, each code is constructed
 * the way the SDK constructs it when no override is given — its typed error
 * class — and BOTH `retryable` and `userFixable` are asserted (`null` = unset).
 * `retryable` is class-based in this SDK, so a base `ModuleError` would leave
 * the D-135 defaults (EXECUTION_CANCELLED, CIRCUIT_BREAKER_OPEN,
 * PIPELINE_CONFIGURATION_ERROR) unchecked. `ai_guidance` is human-readable
 * and intentionally not pinned.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { ExecutionCancelledError } from '../src/cancel.js';
import {
  ACLDeniedError,
  ApprovalDeniedError,
  ApprovalTimeoutError,
  BindingSchemaInferenceFailedError,
  BindingSchemaModeConflictError,
  BindingStrictSchemaIncompatibleError,
  CallDepthExceededError,
  CallFrequencyExceededError,
  CircuitBreakerOpenError,
  CircularCallError,
  DependencyNotFoundError,
  DependencyVersionMismatchError,
  InternalError,
  InvalidInputError,
  ModuleDisabledError,
  ModuleError,
  ModuleExecuteError,
  ModuleNotFoundError,
  ModuleTimeoutError,
  SchemaValidationError,
  USER_FIXABLE_BY_CODE,
  VersionConstraintError,
} from '../src/errors.js';
import { ConfigurationError } from '../src/pipeline.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadFixture(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

/** How the SDK constructs each code's error when no override is given. */
const CONSTRUCT: Record<string, () => ModuleError> = {
  SCHEMA_VALIDATION_ERROR: () => new SchemaValidationError(),
  GENERAL_INVALID_INPUT: () => new InvalidInputError(),
  MODULE_NOT_FOUND: () => new ModuleNotFoundError('m.x'),
  VERSION_CONSTRAINT_INVALID: () => new VersionConstraintError('>>1', 'bad'),
  BINDING_SCHEMA_INFERENCE_FAILED: () => new BindingSchemaInferenceFailedError('t:f'),
  BINDING_SCHEMA_MODE_CONFLICT: () => new BindingSchemaModeConflictError('m.x', ['a', 'b']),
  BINDING_STRICT_SCHEMA_INCOMPATIBLE: () => new BindingStrictSchemaIncompatibleError('m.x', ['oneOf']),
  DEPENDENCY_NOT_FOUND: () => new DependencyNotFoundError('m.x', 'm.y'),
  DEPENDENCY_VERSION_MISMATCH: () => new DependencyVersionMismatchError('m.x', 'm.y', '>=2', '1.0.0'),
  ACL_DENIED: () => new ACLDeniedError('a.b', 'c.d'),
  APPROVAL_DENIED: () => new ApprovalDeniedError(null, 'm.x'),
  APPROVAL_TIMEOUT: () => new ApprovalTimeoutError(null, 'm.x'),
  MODULE_TIMEOUT: () => new ModuleTimeoutError('m.x', 10),
  MODULE_DISABLED: () => new ModuleDisabledError('m.x'),
  CALL_DEPTH_EXCEEDED: () => new CallDepthExceededError(33, 32, []),
  CIRCULAR_CALL: () => new CircularCallError('m.x', ['m.x', 'm.x']),
  CALL_FREQUENCY_EXCEEDED: () => new CallFrequencyExceededError('m.x', 4, 3, []),
  GENERAL_INTERNAL_ERROR: () => new InternalError(),
  MODULE_EXECUTE_ERROR: () => new ModuleExecuteError('m.x', 'boom'),
  EXECUTION_CANCELLED: () => new ExecutionCancelledError(),
  CIRCUIT_BREAKER_OPEN: () => new CircuitBreakerOpenError('m.x'),
  PIPELINE_CONFIGURATION_ERROR: () => new ConfigurationError('bad pipeline'),
};

describe('Conformance: error recovery metadata (retryable + user_fixable)', () => {
  const fixture = loadFixture('error_recovery_metadata');

  fixture.test_cases.forEach((tc: any) => {
    it(tc.id, () => {
      const make = CONSTRUCT[tc.code];
      expect(make, `no construction for ${tc.code}`).toBeDefined();
      const err = make();
      expect(err.code).toBe(tc.code);
      expect(err.retryable).toBe(tc.expected.retryable ?? null);
      expect(err.userFixable).toBe(tc.expected.user_fixable ?? null);
    });
  });

  it('honours every driver_contract rule', () => {
    expect(Object.keys(fixture.driver_contract).sort()).toEqual([
      'construction',
      'retryable_is_part_of_the_contract',
    ]);
  });

  it('fixture map matches USER_FIXABLE_BY_CODE source of truth', () => {
    // The fixture's code->user_fixable map (excluding the intentionally-unset
    // null entries) must equal the single source of truth in errors.ts.
    const fixtureMap: Record<string, boolean> = {};
    for (const tc of fixture.test_cases as any[]) {
      if (tc.expected.user_fixable !== null) {
        fixtureMap[tc.code] = tc.expected.user_fixable;
      }
    }
    expect(fixtureMap).toEqual({ ...USER_FIXABLE_BY_CODE });
  });
});
