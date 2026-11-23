/**
 * Each ModuleError subclass forwards optional ErrorOptions fields (cause,
 * traceId, retryable, aiGuidance, userFixable, suggestion) to the base
 * constructor via `options?.field` chains. The default-branch (no options)
 * is exercised by other tests; this file exercises the *truthy* branch so
 * branch coverage covers both sides for every subclass.
 */
import { describe, it, expect } from 'vitest';
import {
  ACLDeniedError,
  ACLRuleError,
  ApprovalDeniedError,
  ApprovalPendingError,
  ApprovalTimeoutError,
  BindingCallableNotFoundError,
  BindingFileInvalidError,
  BindingInvalidTargetError,
  BindingModuleNotFoundError,
  BindingNotCallableError,
  BindingSchemaInferenceFailedError,
  BindingSchemaMissingError,
  BindingSchemaModeConflictError,
  CallDepthExceededError,
  CallFrequencyExceededError,
  CircuitBreakerOpenError,
  CircularCallError,
  CircularDependencyError,
  ConfigBindError,
  ConfigEnvMapConflictError,
  ConfigEnvPrefixConflictError,
  ConfigError,
  ConfigMountError,
  ConfigNamespaceDuplicateError,
  ConfigNamespaceReservedError,
  ConfigNotFoundError,
  DependencyNotFoundError,
  DependencyVersionMismatchError,
  ErrorFormatterDuplicateError,
  FuncMissingReturnTypeError,
  FuncMissingTypeHintError,
  IdTooLongError,
  InternalError,
  InvalidInputError,
  InvalidSegmentError,
  ModuleDisabledError,
  ModuleError,
  ModuleExecuteError,
  ModuleIdConflictError,
  ModuleLoadError,
  ModuleNotFoundError,
  ModuleReloadConflictError,
  ModuleTimeoutError,
  ReloadFailedError,
  SchemaCircularRefError,
  SchemaNotFoundError,
  SchemaParseError,
  SchemaValidationError,
  SysModuleRegistrationError,
  TaskLimitExceededError,
  VersionConstraintError,
} from '../src/errors.js';
import { ConfigurationError } from '../src/pipeline-config.js';
import {
  PipelineAbortError,
  PipelineDependencyError,
  PipelineStepError,
  PipelineStepNotFoundError,
  StepNameDuplicateError,
  StepNotFoundError,
  StepNotRemovableError,
  StepNotReplaceableError,
  StrategyNotFoundError,
} from '../src/pipeline.js';

const fullOpts = {
  cause: new Error('underlying'),
  traceId: 'trace-1',
  retryable: true,
  aiGuidance: 'try this',
  userFixable: true,
  suggestion: 'do that',
};

function expectAllFieldsForwarded(err: ModuleError): void {
  expect(err.cause).toBe(fullOpts.cause);
  expect(err.traceId).toBe(fullOpts.traceId);
  expect(err.retryable).toBe(fullOpts.retryable);
  expect(err.aiGuidance).toBe(fullOpts.aiGuidance);
  expect(err.userFixable).toBe(fullOpts.userFixable);
  expect(err.suggestion).toBe(fullOpts.suggestion);
}

describe('ErrorOptions forwarding — exercises the truthy branch on every subclass', () => {
  it('ConfigNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new ConfigNotFoundError('/p', fullOpts));
  });
  it('ConfigError forwards options', () => {
    expectAllFieldsForwarded(new ConfigError('m', fullOpts));
  });
  it('ACLRuleError forwards options', () => {
    expectAllFieldsForwarded(new ACLRuleError('m', fullOpts));
  });
  it('ACLDeniedError forwards options', () => {
    expectAllFieldsForwarded(new ACLDeniedError('caller', 'target', fullOpts));
  });
  it('ModuleNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new ModuleNotFoundError('mod', fullOpts));
  });
  it('ModuleDisabledError forwards options', () => {
    expectAllFieldsForwarded(new ModuleDisabledError('mod', fullOpts));
  });
  it('ModuleTimeoutError forwards options', () => {
    expectAllFieldsForwarded(new ModuleTimeoutError('mod', 100, fullOpts));
  });
  it('SchemaValidationError forwards options', () => {
    expectAllFieldsForwarded(new SchemaValidationError('m', [], fullOpts));
  });
  it('SchemaNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new SchemaNotFoundError('id', fullOpts));
  });
  it('SchemaParseError forwards options', () => {
    expectAllFieldsForwarded(new SchemaParseError('m', fullOpts));
  });
  it('SchemaCircularRefError forwards options', () => {
    expectAllFieldsForwarded(new SchemaCircularRefError('p', fullOpts));
  });
  it('CallDepthExceededError forwards options', () => {
    expectAllFieldsForwarded(new CallDepthExceededError(5, 4, ['a'], fullOpts));
  });
  it('CircularCallError forwards options', () => {
    expectAllFieldsForwarded(new CircularCallError('m', ['a', 'b', 'a'], fullOpts));
  });
  it('CallFrequencyExceededError forwards options', () => {
    expectAllFieldsForwarded(
      new CallFrequencyExceededError('m', 10, 5, ['a'], fullOpts),
    );
  });
  it('InvalidInputError forwards options', () => {
    expectAllFieldsForwarded(new InvalidInputError('m', fullOpts));
  });
  it('BindingInvalidTargetError forwards options', () => {
    expectAllFieldsForwarded(new BindingInvalidTargetError('t', fullOpts));
  });
  it('BindingModuleNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new BindingModuleNotFoundError('p', fullOpts));
  });
  it('BindingCallableNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new BindingCallableNotFoundError('c', 'p', fullOpts));
  });
  it('BindingNotCallableError forwards options', () => {
    expectAllFieldsForwarded(new BindingNotCallableError('t', fullOpts));
  });
  it('BindingSchemaInferenceFailedError forwards options', () => {
    expectAllFieldsForwarded(
      new BindingSchemaInferenceFailedError('t', 'm', 'f', undefined, fullOpts),
    );
  });
  it('BindingSchemaMissingError forwards options', () => {
    expectAllFieldsForwarded(
      new BindingSchemaMissingError('t', 'm', 'f', undefined, fullOpts),
    );
  });
  it('BindingSchemaModeConflictError forwards options', () => {
    expectAllFieldsForwarded(
      new BindingSchemaModeConflictError('m', ['a', 'b'], 'f', fullOpts),
    );
  });
  it('FuncMissingTypeHintError forwards options', () => {
    expectAllFieldsForwarded(new FuncMissingTypeHintError('fn', 'p', fullOpts));
  });
  it('FuncMissingReturnTypeError forwards options', () => {
    expectAllFieldsForwarded(new FuncMissingReturnTypeError('fn', fullOpts));
  });
  it('BindingFileInvalidError forwards options', () => {
    expectAllFieldsForwarded(new BindingFileInvalidError('p', 'r', fullOpts));
  });
  it('CircularDependencyError forwards options', () => {
    expectAllFieldsForwarded(new CircularDependencyError(['a', 'b'], fullOpts));
  });
  it('ModuleLoadError forwards options', () => {
    expectAllFieldsForwarded(new ModuleLoadError('m', 'r', fullOpts));
  });
  it('DependencyNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new DependencyNotFoundError('m', 'd', fullOpts));
  });
  it('DependencyVersionMismatchError forwards options', () => {
    expectAllFieldsForwarded(
      new DependencyVersionMismatchError('m', 'd', '1.x', '2.x', fullOpts),
    );
  });
  it('ReloadFailedError forwards options', () => {
    expectAllFieldsForwarded(new ReloadFailedError('m', 'r', fullOpts));
  });
  it('ModuleExecuteError forwards options', () => {
    expectAllFieldsForwarded(new ModuleExecuteError('m', 'r', fullOpts));
  });
  it('InternalError forwards options', () => {
    expectAllFieldsForwarded(new InternalError('m', fullOpts));
  });
  it('ApprovalDeniedError forwards options', () => {
    expectAllFieldsForwarded(new ApprovalDeniedError({}, 'm', fullOpts));
  });
  it('ApprovalTimeoutError forwards options', () => {
    expectAllFieldsForwarded(new ApprovalTimeoutError({}, 'm', fullOpts));
  });
  it('ApprovalPendingError forwards options', () => {
    expectAllFieldsForwarded(new ApprovalPendingError({}, 'm', fullOpts));
  });
  it('ConfigNamespaceDuplicateError forwards options', () => {
    expectAllFieldsForwarded(new ConfigNamespaceDuplicateError('ns', fullOpts));
  });
  it('ConfigNamespaceReservedError forwards options', () => {
    expectAllFieldsForwarded(new ConfigNamespaceReservedError('ns', fullOpts));
  });
  it('ConfigEnvPrefixConflictError forwards options', () => {
    expectAllFieldsForwarded(new ConfigEnvPrefixConflictError('PFX_', fullOpts));
  });
  it('ConfigEnvMapConflictError forwards options', () => {
    expectAllFieldsForwarded(new ConfigEnvMapConflictError('VAR', 'owner', fullOpts));
  });
  it('ConfigMountError forwards options', () => {
    expectAllFieldsForwarded(new ConfigMountError('m', fullOpts));
  });
  it('ConfigBindError forwards options', () => {
    expectAllFieldsForwarded(new ConfigBindError('m', fullOpts));
  });
  it('ErrorFormatterDuplicateError forwards options', () => {
    expectAllFieldsForwarded(new ErrorFormatterDuplicateError('a', fullOpts));
  });
  it('TaskLimitExceededError forwards options', () => {
    expectAllFieldsForwarded(new TaskLimitExceededError(10, fullOpts));
  });
  it('VersionConstraintError forwards options', () => {
    expectAllFieldsForwarded(new VersionConstraintError('>=', 'reason', fullOpts));
  });
  it('ModuleIdConflictError forwards options', () => {
    expectAllFieldsForwarded(
      new ModuleIdConflictError('p', ['a', 'b'], 'seg', fullOpts),
    );
  });
  it('InvalidSegmentError forwards options', () => {
    expectAllFieldsForwarded(new InvalidSegmentError('p', 'cls', 'seg', fullOpts));
  });
  it('CircuitBreakerOpenError forwards options', () => {
    expectAllFieldsForwarded(new CircuitBreakerOpenError('m', 'caller', fullOpts));
  });
  it('IdTooLongError forwards options', () => {
    expectAllFieldsForwarded(new IdTooLongError('p', 'm', fullOpts));
  });
  it('ModuleReloadConflictError forwards options', () => {
    expectAllFieldsForwarded(new ModuleReloadConflictError(fullOpts));
  });
  it('SysModuleRegistrationError forwards options.cause when no positional cause is given', () => {
    const err = new SysModuleRegistrationError('m', undefined, fullOpts);
    // SysModuleRegistrationError prefers a positional cause but falls back to options.cause.
    expect(err.cause).toBe(fullOpts.cause);
    expect(err.traceId).toBe(fullOpts.traceId);
    expect(err.retryable).toBe(fullOpts.retryable);
    expect(err.aiGuidance).toBe(fullOpts.aiGuidance);
    expect(err.userFixable).toBe(fullOpts.userFixable);
    expect(err.suggestion).toBe(fullOpts.suggestion);
  });

  it('ModuleError.toJSON() includes every optional field when populated', () => {
    const err = new ModuleError(
      'CODE',
      'msg',
      { k: 'v' },
      new Error('cause'),
      'tid',
      false,
      'guidance',
      true,
      'suggest',
    );
    const json = err.toJSON();
    expect(json.code).toBe('CODE');
    expect(json.details).toEqual({ k: 'v' });
    expect(json.cause).toBe('cause');
    // A-D-008: wire keys are snake_case for cross-process interop.
    expect(json.trace_id).toBe('tid');
    expect(json.retryable).toBe(false);
    expect(json.ai_guidance).toBe('guidance');
    expect(json.user_fixable).toBe(true);
    expect(json.suggestion).toBe('suggest');
    expect(typeof json.timestamp).toBe('string');
  });

  it('ModuleError.toJSON() coerces a non-Error cause via String()', () => {
    // Cast around the constructor type to plant a non-Error cause and exercise
    // the `cause instanceof Error ? msg : String(cause)` branch in toJSON().
    const err = new ModuleError('C', 'm');
    (err as unknown as { cause: unknown }).cause = 'not-an-error';
    const json = err.toJSON();
    expect(json.cause).toBe('not-an-error');
  });

  it('ModuleError.toJSON() omits optional fields when they are at default/null', () => {
    const err = new ModuleError('C', 'm');
    const json = err.toJSON();
    expect(json.cause).toBeUndefined();
    expect(json.trace_id).toBeUndefined();
    expect(json.retryable).toBeUndefined();
    expect(json.ai_guidance).toBeUndefined();
    expect(json.user_fixable).toBeUndefined();
    expect(json.suggestion).toBeUndefined();
    expect(json.details).toBeUndefined();
  });

  it('ModuleError.toString() returns "[code] message"', () => {
    const err = new ModuleError('C', 'hello');
    expect(err.toString()).toBe('[C] hello');
  });

  // -------------------------------------------------------------------------
  // pipeline.ts error classes
  // -------------------------------------------------------------------------

  it('ConfigurationError forwards options', () => {
    expectAllFieldsForwarded(new ConfigurationError('m', fullOpts));
  });
  it('PipelineAbortError forwards options', () => {
    expectAllFieldsForwarded(
      new PipelineAbortError('step', 'why', ['alt'], null, fullOpts),
    );
  });
  it('StepNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new StepNotFoundError('m', fullOpts));
  });
  it('StepNotRemovableError forwards options', () => {
    expectAllFieldsForwarded(new StepNotRemovableError('m', fullOpts));
  });
  it('StepNotReplaceableError forwards options', () => {
    expectAllFieldsForwarded(new StepNotReplaceableError('m', fullOpts));
  });
  it('StepNameDuplicateError forwards options', () => {
    expectAllFieldsForwarded(new StepNameDuplicateError('m', fullOpts));
  });
  it('PipelineStepError forwards options (cause is positional)', () => {
    const err = new PipelineStepError('s', new Error('inner'), null, fullOpts);
    expect(err.traceId).toBe(fullOpts.traceId);
    expect(err.retryable).toBe(fullOpts.retryable);
    expect(err.aiGuidance).toBe(fullOpts.aiGuidance);
    expect(err.userFixable).toBe(fullOpts.userFixable);
    expect(err.suggestion).toBe(fullOpts.suggestion);
  });
  it('PipelineDependencyError forwards options', () => {
    expectAllFieldsForwarded(
      new PipelineDependencyError('s', ['a'], fullOpts),
    );
  });
  it('PipelineStepNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new PipelineStepNotFoundError('s', fullOpts));
  });
  it('StrategyNotFoundError forwards options', () => {
    expectAllFieldsForwarded(new StrategyNotFoundError('m', fullOpts));
  });
});
