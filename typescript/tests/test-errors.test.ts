import { describe, expect, it } from 'vitest';
import {
  ACLDeniedError,
  ACLRuleError,
  ApprovalDeniedError,
  ApprovalError,
  ApprovalPendingError,
  ApprovalTimeoutError,
  BindingCallableNotFoundError,
  BindingFileInvalidError,
  BindingInvalidTargetError,
  BindingModuleNotFoundError,
  BindingNotCallableError,
  BindingSchemaMissingError,
  CallDepthExceededError,
  CallFrequencyExceededError,
  CircularCallError,
  CircularDependencyError,
  ConfigBindError,
  ConfigEnvMapConflictError,
  ConfigError,
  ConfigNotFoundError,
  ErrorCodes,
  ErrorFormatterDuplicateError,
  InternalError,
  InvalidInputError,
  ModuleError,
  ModuleExecuteError,
  ModuleLoadError,
  ModuleNotFoundError,
  ModuleTimeoutError,
  SchemaCircularRefError,
  SchemaMaxDepthExceededError,
  SchemaNotFoundError,
  SchemaParseError,
  SchemaValidationError,
} from '../src/errors.js';

describe('ModuleError', () => {
  it('creates with code and message', () => {
    const err = new ModuleError('TEST_CODE', 'test message');
    expect(err.code).toBe('TEST_CODE');
    expect(err.message).toBe('test message');
    expect(err.name).toBe('ModuleError');
    expect(err.details).toEqual({});
    expect(err.timestamp).toBeDefined();
  });

  it('toString includes code and message', () => {
    const err = new ModuleError('ERR', 'something failed');
    expect(err.toString()).toBe('[ERR] something failed');
  });

  it('accepts details, cause, and traceId', () => {
    const cause = new Error('root cause');
    const err = new ModuleError('X', 'msg', { key: 'val' }, cause, 'trace-123');
    expect(err.details).toEqual({ key: 'val' });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe('trace-123');
  });

  it('is an instance of Error', () => {
    const err = new ModuleError('X', 'msg');
    expect(err).toBeInstanceOf(Error);
    expect(err).toBeInstanceOf(ModuleError);
  });
});

describe('Error subclasses', () => {
  it('ModuleNotFoundError', () => {
    const err = new ModuleNotFoundError('mod.x');
    expect(err.name).toBe('ModuleNotFoundError');
    expect(err.code).toBe('MODULE_NOT_FOUND');
    expect(err.message).toContain('mod.x');
    expect(err.details['moduleId']).toBe('mod.x');
  });

  it('ModuleTimeoutError', () => {
    const err = new ModuleTimeoutError('mod.x', 5000);
    expect(err.name).toBe('ModuleTimeoutError');
    expect(err.code).toBe('MODULE_TIMEOUT');
    expect(err.moduleId).toBe('mod.x');
    expect(err.timeoutMs).toBe(5000);
  });

  it('SchemaValidationError', () => {
    const err = new SchemaValidationError('bad data', [{ path: '/x' }]);
    expect(err.name).toBe('SchemaValidationError');
    expect(err.code).toBe('SCHEMA_VALIDATION_ERROR');
    expect(err.details['errors']).toHaveLength(1);
  });

  it('ACLDeniedError', () => {
    const err = new ACLDeniedError('caller.a', 'target.b');
    expect(err.name).toBe('ACLDeniedError');
    expect(err.code).toBe('ACL_DENIED');
    expect(err.callerId).toBe('caller.a');
    expect(err.targetId).toBe('target.b');
  });

  it('CallDepthExceededError', () => {
    const err = new CallDepthExceededError(33, 32, ['a', 'b']);
    expect(err.name).toBe('CallDepthExceededError');
    expect(err.code).toBe('CALL_DEPTH_EXCEEDED');
    expect(err.currentDepth).toBe(33);
    expect(err.maxDepth).toBe(32);
  });

  it('CircularCallError', () => {
    const err = new CircularCallError('mod.a', ['mod.a', 'mod.b', 'mod.a']);
    expect(err.name).toBe('CircularCallError');
    expect(err.code).toBe('CIRCULAR_CALL');
    expect(err.moduleId).toBe('mod.a');
  });

  it('CallFrequencyExceededError', () => {
    const err = new CallFrequencyExceededError('mod.a', 4, 3, ['mod.a', 'mod.a', 'mod.a', 'mod.a']);
    expect(err.name).toBe('CallFrequencyExceededError');
    expect(err.code).toBe('CALL_FREQUENCY_EXCEEDED');
    expect(err.moduleId).toBe('mod.a');
    expect(err.count).toBe(4);
    expect(err.maxRepeat).toBe(3);
  });

  it('ConfigNotFoundError', () => {
    const err = new ConfigNotFoundError('/path/to/config');
    expect(err.name).toBe('ConfigNotFoundError');
    expect(err.code).toBe('CONFIG_NOT_FOUND');
  });

  it('ConfigError', () => {
    const err = new ConfigError('bad config');
    expect(err.name).toBe('ConfigError');
    expect(err.code).toBe('CONFIG_INVALID');
  });

  it('InvalidInputError', () => {
    const err = new InvalidInputError('bad input');
    expect(err.name).toBe('InvalidInputError');
    expect(err.code).toBe('GENERAL_INVALID_INPUT');
  });

  it('BindingInvalidTargetError', () => {
    const err = new BindingInvalidTargetError('bad:target:format');
    expect(err.name).toBe('BindingInvalidTargetError');
    expect(err.code).toBe('BINDING_INVALID_TARGET');
  });

  it('BindingModuleNotFoundError', () => {
    const err = new BindingModuleNotFoundError('some.module');
    expect(err.name).toBe('BindingModuleNotFoundError');
    expect(err.code).toBe('BINDING_MODULE_NOT_FOUND');
  });

  it('BindingCallableNotFoundError', () => {
    const err = new BindingCallableNotFoundError('fn', 'some.module');
    expect(err.name).toBe('BindingCallableNotFoundError');
    expect(err.code).toBe('BINDING_CALLABLE_NOT_FOUND');
  });

  it('BindingNotCallableError', () => {
    const err = new BindingNotCallableError('some:target');
    expect(err.name).toBe('BindingNotCallableError');
    expect(err.code).toBe('BINDING_NOT_CALLABLE');
  });

  it('BindingSchemaInferenceFailedError (canonical name in spec 1.0)', () => {
    const err = new BindingSchemaMissingError('some:target');
    expect(err.name).toBe('BindingSchemaInferenceFailedError');
    expect(err.code).toBe('BINDING_SCHEMA_INFERENCE_FAILED');
    expect(err.message).toContain('some:target');
    expect(err.message).toContain('See protocol-spec §5.12.5');
  });

  it('BindingFileInvalidError', () => {
    const err = new BindingFileInvalidError('/path/file.yaml', 'parse error');
    expect(err.name).toBe('BindingFileInvalidError');
    expect(err.code).toBe('BINDING_FILE_INVALID');
  });

  it('CircularDependencyError', () => {
    const err = new CircularDependencyError(['a', 'b', 'a']);
    expect(err.name).toBe('CircularDependencyError');
    expect(err.code).toBe('CIRCULAR_DEPENDENCY');
    expect(err.message).toContain('a -> b -> a');
  });

  it('ModuleLoadError', () => {
    const err = new ModuleLoadError('mod.a', 'file not found');
    expect(err.name).toBe('ModuleLoadError');
    expect(err.code).toBe('MODULE_LOAD_ERROR');
  });

  it('SchemaNotFoundError', () => {
    const err = new SchemaNotFoundError('schema.x');
    expect(err.name).toBe('SchemaNotFoundError');
    expect(err.code).toBe('SCHEMA_NOT_FOUND');
  });

  it('SchemaParseError', () => {
    const err = new SchemaParseError('invalid yaml');
    expect(err.name).toBe('SchemaParseError');
    expect(err.code).toBe('SCHEMA_PARSE_ERROR');
  });

  it('SchemaCircularRefError', () => {
    const err = new SchemaCircularRefError('#/definitions/A');
    expect(err.name).toBe('SchemaCircularRefError');
    expect(err.code).toBe('SCHEMA_CIRCULAR_REF');
  });

  it('SchemaMaxDepthExceededError', () => {
    const err = new SchemaMaxDepthExceededError('#/definitions/A');
    expect(err.name).toBe('SchemaMaxDepthExceededError');
    expect(err.code).toBe('SCHEMA_MAX_DEPTH_EXCEEDED');
  });

  it('ACLRuleError', () => {
    const err = new ACLRuleError('bad rule');
    expect(err.name).toBe('ACLRuleError');
    expect(err.code).toBe('ACL_RULE_ERROR');
  });
});

describe('ModuleError optional parameters', () => {
  it('defaults details to empty object when not provided', () => {
    const err = new ModuleError('X', 'msg');
    expect(err.details).toEqual({});
  });

  it('leaves cause and traceId undefined when not provided', () => {
    const err = new ModuleError('X', 'msg');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('passes cause to Error super constructor', () => {
    const cause = new Error('root');
    const err = new ModuleError('X', 'msg', {}, cause);
    expect(err.cause).toBe(cause);
  });

  it('sets traceId when provided', () => {
    const err = new ModuleError('X', 'msg', {}, undefined, 'trace-abc');
    expect(err.traceId).toBe('trace-abc');
    expect(err.cause).toBeUndefined();
  });

  it('toString returns formatted code and message', () => {
    const err = new ModuleError('MY_CODE', 'my message');
    expect(err.toString()).toBe('[MY_CODE] my message');
  });
});

describe('Error subclasses with options (cause and traceId branches)', () => {
  const cause = new Error('underlying cause');
  const traceId = 'trace-999';

  it('ConfigNotFoundError with cause and traceId', () => {
    const err = new ConfigNotFoundError('/cfg', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
    expect(err.details['configPath']).toBe('/cfg');
  });

  it('ConfigNotFoundError without options', () => {
    const err = new ConfigNotFoundError('/cfg');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('ConfigError with cause and traceId', () => {
    const err = new ConfigError('bad config', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('ConfigError without options', () => {
    const err = new ConfigError('bad config');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('ACLRuleError with cause and traceId', () => {
    const err = new ACLRuleError('bad rule', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('ACLRuleError without options', () => {
    const err = new ACLRuleError('bad rule');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('ACLDeniedError with cause and traceId', () => {
    const err = new ACLDeniedError('caller.a', 'target.b', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
    expect(err.callerId).toBe('caller.a');
    expect(err.targetId).toBe('target.b');
  });

  it('ACLDeniedError without options', () => {
    const err = new ACLDeniedError('caller.a', 'target.b');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('ACLDeniedError with null callerId', () => {
    const err = new ACLDeniedError(null, 'target.b');
    expect(err.callerId).toBeNull();
    expect(err.targetId).toBe('target.b');
    expect(err.message).toContain('null -> target.b');
  });

  it('ModuleNotFoundError with cause and traceId', () => {
    const err = new ModuleNotFoundError('mod.x', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('ModuleNotFoundError without options', () => {
    const err = new ModuleNotFoundError('mod.x');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('ModuleTimeoutError with cause and traceId', () => {
    const err = new ModuleTimeoutError('mod.x', 3000, { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
    expect(err.moduleId).toBe('mod.x');
    expect(err.timeoutMs).toBe(3000);
  });

  it('ModuleTimeoutError without options', () => {
    const err = new ModuleTimeoutError('mod.x', 3000);
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('SchemaValidationError with cause and traceId', () => {
    const err = new SchemaValidationError('invalid', [{ path: '/a' }], { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('SchemaValidationError without options', () => {
    const err = new SchemaValidationError('invalid', []);
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('SchemaValidationError uses default message when no arguments provided', () => {
    const err = new SchemaValidationError();
    expect(err.message).toBe('Schema validation failed');
    expect(err.details['errors']).toEqual([]);
  });

  it('SchemaValidationError defaults errors to empty array when errors not provided', () => {
    const err = new SchemaValidationError('custom message');
    expect(err.details['errors']).toEqual([]);
  });

  it('SchemaNotFoundError with cause and traceId', () => {
    const err = new SchemaNotFoundError('schema.x', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('SchemaNotFoundError without options', () => {
    const err = new SchemaNotFoundError('schema.x');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('SchemaParseError with cause and traceId', () => {
    const err = new SchemaParseError('bad yaml', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('SchemaParseError without options', () => {
    const err = new SchemaParseError('bad yaml');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('SchemaCircularRefError with cause and traceId', () => {
    const err = new SchemaCircularRefError('#/defs/A', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('SchemaCircularRefError without options', () => {
    const err = new SchemaCircularRefError('#/defs/A');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('CallDepthExceededError with cause and traceId', () => {
    const err = new CallDepthExceededError(5, 4, ['a', 'b'], { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
    expect(err.currentDepth).toBe(5);
    expect(err.maxDepth).toBe(4);
  });

  it('CallDepthExceededError without options', () => {
    const err = new CallDepthExceededError(5, 4, ['a', 'b']);
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('CircularCallError with cause and traceId', () => {
    const err = new CircularCallError('mod.a', ['mod.a', 'mod.b'], { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
    expect(err.moduleId).toBe('mod.a');
  });

  it('CircularCallError without options', () => {
    const err = new CircularCallError('mod.a', ['mod.a', 'mod.b']);
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('CallFrequencyExceededError with cause and traceId', () => {
    const err = new CallFrequencyExceededError('mod.a', 4, 3, ['mod.a'], { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
    expect(err.moduleId).toBe('mod.a');
    expect(err.count).toBe(4);
    expect(err.maxRepeat).toBe(3);
  });

  it('CallFrequencyExceededError without options', () => {
    const err = new CallFrequencyExceededError('mod.a', 4, 3, ['mod.a']);
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('InvalidInputError with cause and traceId', () => {
    const err = new InvalidInputError('bad data', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('InvalidInputError without options', () => {
    const err = new InvalidInputError('bad data');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('InvalidInputError uses default message when no arguments provided', () => {
    const err = new InvalidInputError();
    expect(err.message).toBe('Invalid input');
  });

  it('BindingInvalidTargetError with cause and traceId', () => {
    const err = new BindingInvalidTargetError('bad:target', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('BindingInvalidTargetError without options', () => {
    const err = new BindingInvalidTargetError('bad:target');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('BindingModuleNotFoundError with cause and traceId', () => {
    const err = new BindingModuleNotFoundError('some.module', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('BindingModuleNotFoundError without options', () => {
    const err = new BindingModuleNotFoundError('some.module');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('BindingCallableNotFoundError with cause and traceId', () => {
    const err = new BindingCallableNotFoundError('fn', 'some.module', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('BindingCallableNotFoundError without options', () => {
    const err = new BindingCallableNotFoundError('fn', 'some.module');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('BindingNotCallableError with cause and traceId', () => {
    const err = new BindingNotCallableError('some:target', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('BindingNotCallableError without options', () => {
    const err = new BindingNotCallableError('some:target');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('BindingSchemaInferenceFailedError with options', () => {
    const err = new BindingSchemaMissingError('some:target', 'mod.id', 'b.yaml', undefined, {
      cause,
      traceId,
    });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('BindingSchemaInferenceFailedError without options', () => {
    const err = new BindingSchemaMissingError('some:target');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('BindingFileInvalidError with cause and traceId', () => {
    const err = new BindingFileInvalidError('/file.yaml', 'parse error', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('BindingFileInvalidError without options', () => {
    const err = new BindingFileInvalidError('/file.yaml', 'parse error');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('CircularDependencyError with cause and traceId', () => {
    const err = new CircularDependencyError(['a', 'b', 'a'], { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('CircularDependencyError without options', () => {
    const err = new CircularDependencyError(['a', 'b', 'a']);
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('ModuleLoadError with cause and traceId', () => {
    const err = new ModuleLoadError('mod.a', 'file not found', { cause, traceId });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('ModuleLoadError without options', () => {
    const err = new ModuleLoadError('mod.a', 'file not found');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('ModuleExecuteError with cause and traceId', () => {
    const err = new ModuleExecuteError('mod.a', 'runtime error', { cause, traceId });
    expect(err.name).toBe('ModuleExecuteError');
    expect(err.code).toBe('MODULE_EXECUTE_ERROR');
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
    expect(err.message).toContain('mod.a');
    expect(err.message).toContain('runtime error');
  });

  it('ModuleExecuteError without options', () => {
    const err = new ModuleExecuteError('mod.a', 'runtime error');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('InternalError with cause and traceId', () => {
    const err = new InternalError('something broke', { cause, traceId });
    expect(err.name).toBe('InternalError');
    expect(err.code).toBe('GENERAL_INTERNAL_ERROR');
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe(traceId);
  });

  it('InternalError without options', () => {
    const err = new InternalError('something broke');
    expect(err.cause).toBeUndefined();
    expect(err.traceId).toBeUndefined();
  });

  it('InternalError uses default message when no arguments provided', () => {
    const err = new InternalError();
    expect(err.message).toBe('Internal error');
  });
});

describe('ErrorCodes', () => {
  it('contains MIDDLEWARE_CHAIN_ERROR', () => {
    expect(ErrorCodes.MIDDLEWARE_CHAIN_ERROR).toBe('MIDDLEWARE_CHAIN_ERROR');
  });

  it('contains all expected error codes', () => {
    expect(ErrorCodes.CONFIG_NOT_FOUND).toBe('CONFIG_NOT_FOUND');
    expect(ErrorCodes.CONFIG_INVALID).toBe('CONFIG_INVALID');
    expect(ErrorCodes.ACL_RULE_ERROR).toBe('ACL_RULE_ERROR');
    expect(ErrorCodes.ACL_DENIED).toBe('ACL_DENIED');
    expect(ErrorCodes.MODULE_NOT_FOUND).toBe('MODULE_NOT_FOUND');
    expect(ErrorCodes.MODULE_TIMEOUT).toBe('MODULE_TIMEOUT');
    expect(ErrorCodes.MODULE_LOAD_ERROR).toBe('MODULE_LOAD_ERROR');
    expect(ErrorCodes.MODULE_EXECUTE_ERROR).toBe('MODULE_EXECUTE_ERROR');
    expect(ErrorCodes.SCHEMA_VALIDATION_ERROR).toBe('SCHEMA_VALIDATION_ERROR');
    expect(ErrorCodes.SCHEMA_NOT_FOUND).toBe('SCHEMA_NOT_FOUND');
    expect(ErrorCodes.SCHEMA_PARSE_ERROR).toBe('SCHEMA_PARSE_ERROR');
    expect(ErrorCodes.SCHEMA_CIRCULAR_REF).toBe('SCHEMA_CIRCULAR_REF');
    expect(ErrorCodes.SCHEMA_MAX_DEPTH_EXCEEDED).toBe('SCHEMA_MAX_DEPTH_EXCEEDED');
    expect(ErrorCodes.CALL_DEPTH_EXCEEDED).toBe('CALL_DEPTH_EXCEEDED');
    expect(ErrorCodes.CIRCULAR_CALL).toBe('CIRCULAR_CALL');
    expect(ErrorCodes.CALL_FREQUENCY_EXCEEDED).toBe('CALL_FREQUENCY_EXCEEDED');
    expect(ErrorCodes.GENERAL_INVALID_INPUT).toBe('GENERAL_INVALID_INPUT');
    expect(ErrorCodes.GENERAL_INTERNAL_ERROR).toBe('GENERAL_INTERNAL_ERROR');
    expect(ErrorCodes.FUNC_MISSING_TYPE_HINT).toBe('FUNC_MISSING_TYPE_HINT');
    expect(ErrorCodes.FUNC_MISSING_RETURN_TYPE).toBe('FUNC_MISSING_RETURN_TYPE');
    expect(ErrorCodes.BINDING_INVALID_TARGET).toBe('BINDING_INVALID_TARGET');
    expect(ErrorCodes.BINDING_MODULE_NOT_FOUND).toBe('BINDING_MODULE_NOT_FOUND');
    expect(ErrorCodes.BINDING_CALLABLE_NOT_FOUND).toBe('BINDING_CALLABLE_NOT_FOUND');
    expect(ErrorCodes.BINDING_NOT_CALLABLE).toBe('BINDING_NOT_CALLABLE');
    expect(ErrorCodes.BINDING_SCHEMA_MISSING).toBe('BINDING_SCHEMA_MISSING');
    expect(ErrorCodes.BINDING_FILE_INVALID).toBe('BINDING_FILE_INVALID');
    expect(ErrorCodes.CIRCULAR_DEPENDENCY).toBe('CIRCULAR_DEPENDENCY');
  });

  it('is frozen and cannot be mutated', () => {
    expect(Object.isFrozen(ErrorCodes)).toBe(true);
  });
});

describe('AI Error Guidance Fields', () => {
  describe('ModuleError base defaults', () => {
    it('all AI fields default to null', () => {
      const err = new ModuleError('TEST', 'test');
      expect(err.retryable).toBeNull();
      expect(err.aiGuidance).toBeNull();
      expect(err.userFixable).toBeNull();
      expect(err.suggestion).toBeNull();
    });

    it('accepts explicit AI fields', () => {
      const err = new ModuleError(
        'TEST',
        'test',
        {},
        undefined,
        undefined,
        true,
        'retry after delay',
        false,
        'Wait and try again',
      );
      expect(err.retryable).toBe(true);
      expect(err.aiGuidance).toBe('retry after delay');
      expect(err.userFixable).toBe(false);
      expect(err.suggestion).toBe('Wait and try again');
    });

    it('retryable can be set to false', () => {
      const err = new ModuleError('TEST', 'test', {}, undefined, undefined, false);
      expect(err.retryable).toBe(false);
    });

    it('retryable can be set to null', () => {
      const err = new ModuleError('TEST', 'test', {}, undefined, undefined, null);
      expect(err.retryable).toBeNull();
    });
  });

  describe('toJSON sparse serialization', () => {
    it('omits null AI fields', () => {
      const err = new ModuleError('TEST', 'test');
      const json = err.toJSON();
      expect(json.code).toBe('TEST');
      expect(json.message).toBe('test');
      expect(json).toHaveProperty('timestamp');
      expect(json).not.toHaveProperty('retryable');
      expect(json).not.toHaveProperty('ai_guidance');
      expect(json).not.toHaveProperty('user_fixable');
      expect(json).not.toHaveProperty('suggestion');
      expect(json).not.toHaveProperty('details');
      expect(json).not.toHaveProperty('cause');
      expect(json).not.toHaveProperty('trace_id');
    });

    it('includes non-null AI fields', () => {
      const err = new ModuleError(
        'TEST',
        'test',
        {},
        undefined,
        undefined,
        false,
        'do not retry',
        true,
        'Fix input',
      );
      const json = err.toJSON();
      expect(json.retryable).toBe(false);
      // A-D-008: wire keys are snake_case for cross-process interop.
      expect(json.ai_guidance).toBe('do not retry');
      expect(json.user_fixable).toBe(true);
      expect(json.suggestion).toBe('Fix input');
      expect(json).not.toHaveProperty('aiGuidance');
      expect(json).not.toHaveProperty('userFixable');
    });

    it('includes details when non-empty', () => {
      const err = new ModuleError('TEST', 'test', { key: 'val' });
      const json = err.toJSON();
      expect(json.details).toEqual({ key: 'val' });
    });

    it('includes cause as the Error message only (no type prefix or stack)', () => {
      const cause = new Error('root cause message');
      const err = new ModuleError('TEST', 'test', {}, cause);
      const json = err.toJSON();
      // Only the message is included — no "Error: " prefix or stack frames
      expect(json.cause).toBe('root cause message');
    });

    it('includes trace_id (snake_case) when present', () => {
      const err = new ModuleError('TEST', 'test', {}, undefined, 'trace-abc');
      const json = err.toJSON();
      expect(json.trace_id).toBe('trace-abc');
      expect(json).not.toHaveProperty('traceId');
    });

    // A-D-008: detail object keys are snake_cased on the wire too, matching
    // the spec serialization example (caller_id/target_id) and Python/Rust.
    it('snake_cases detail keys', () => {
      const err = new ACLDeniedError('api.user', 'executor.admin.reset');
      const json = err.toJSON();
      expect(json.details).toEqual({
        caller_id: 'api.user',
        target_id: 'executor.admin.reset',
      });
    });
  });

  describe('DEFAULT_RETRYABLE per subclass', () => {
    it('ModuleTimeoutError defaults to true', () => {
      const err = new ModuleTimeoutError('m', 1000);
      expect(err.retryable).toBe(true);
    });

    it('InternalError defaults to true', () => {
      const err = new InternalError();
      expect(err.retryable).toBe(true);
    });

    it('ApprovalTimeoutError defaults to true', () => {
      const err = new ApprovalTimeoutError({}, 'm');
      expect(err.retryable).toBe(true);
    });

    it('ModuleExecuteError defaults to null', () => {
      const err = new ModuleExecuteError('m', 'fail');
      expect(err.retryable).toBeNull();
    });

    it.each([
      ['ConfigNotFoundError', () => new ConfigNotFoundError('/cfg')],
      ['ConfigError', () => new ConfigError('bad')],
      ['ACLRuleError', () => new ACLRuleError('bad')],
      ['ACLDeniedError', () => new ACLDeniedError('a', 'b')],
      ['ApprovalDeniedError', () => new ApprovalDeniedError({}, 'm')],
      ['ApprovalPendingError', () => new ApprovalPendingError({}, 'm')],
      ['ModuleNotFoundError', () => new ModuleNotFoundError('m')],
      ['SchemaValidationError', () => new SchemaValidationError()],
      ['SchemaNotFoundError', () => new SchemaNotFoundError('s')],
      ['SchemaParseError', () => new SchemaParseError('bad')],
      ['SchemaCircularRefError', () => new SchemaCircularRefError('#/a')],
      ['SchemaMaxDepthExceededError', () => new SchemaMaxDepthExceededError('#/a')],
      ['CallDepthExceededError', () => new CallDepthExceededError(5, 4, ['a'])],
      ['CircularCallError', () => new CircularCallError('m', ['m'])],
      ['CallFrequencyExceededError', () => new CallFrequencyExceededError('m', 4, 3, ['m'])],
      ['InvalidInputError', () => new InvalidInputError()],
      ['BindingInvalidTargetError', () => new BindingInvalidTargetError('t')],
      ['BindingModuleNotFoundError', () => new BindingModuleNotFoundError('m')],
      ['BindingCallableNotFoundError', () => new BindingCallableNotFoundError('c', 'm')],
      ['BindingNotCallableError', () => new BindingNotCallableError('t')],
      ['BindingSchemaInferenceFailedError', () => new BindingSchemaMissingError('t')],
      ['BindingFileInvalidError', () => new BindingFileInvalidError('/f', 'bad')],
      ['CircularDependencyError', () => new CircularDependencyError(['a', 'b'])],
      ['ModuleLoadError', () => new ModuleLoadError('m', 'fail')],
    ] as const)('%s defaults to false', (_name, factory) => {
      const err = factory();
      expect(err.retryable).toBe(false);
    });
  });

  describe('subclass override via options', () => {
    it('overrides retryable on non-retryable subclass', () => {
      const err = new ConfigNotFoundError('/cfg', { retryable: true });
      expect(err.retryable).toBe(true);
    });

    it('overrides retryable to false on retryable subclass', () => {
      const err = new ModuleTimeoutError('m', 1000, { retryable: false });
      expect(err.retryable).toBe(false);
    });

    it('passes aiGuidance and suggestion via options', () => {
      const err = new SchemaValidationError('bad', [], {
        aiGuidance: 'check schema',
        suggestion: 'Fix input',
      });
      expect(err.aiGuidance).toBe('check schema');
      expect(err.suggestion).toBe('Fix input');
    });

    it('passes userFixable via options', () => {
      const err = new ACLDeniedError('a', 'b', { userFixable: true });
      expect(err.userFixable).toBe(true);
    });

    it('explicit null userFixable overrides the per-code default', () => {
      // ACL_DENIED resolves to userFixable=false via USER_FIXABLE_BY_CODE; an explicit
      // null must win (mirrors Python's _UNSET-sentinel override path).
      expect(new ACLDeniedError('a', 'b').userFixable).toBe(false);
      expect(new ACLDeniedError('a', 'b', { userFixable: null }).userFixable).toBeNull();
    });

    it('ApprovalDeniedError with AI fields', () => {
      const err = new ApprovalDeniedError({}, 'm', {
        retryable: true,
        aiGuidance: 'request with different user',
        userFixable: true,
        suggestion: 'Ask admin',
      });
      expect(err.retryable).toBe(true);
      expect(err.aiGuidance).toBe('request with different user');
      expect(err.userFixable).toBe(true);
      expect(err.suggestion).toBe('Ask admin');
    });
  });

  describe('backward compatibility', () => {
    it('existing ModuleError positional args still work', () => {
      const err = new ModuleError('CODE', 'msg', { k: 'v' }, new Error('c'), 'trace-1');
      expect(err.code).toBe('CODE');
      expect(err.details).toEqual({ k: 'v' });
      expect(err.traceId).toBe('trace-1');
      expect(err.retryable).toBeNull();
    });

    it('existing subclass calls without options still work', () => {
      const err = new ConfigNotFoundError('/path');
      expect(err.code).toBe('CONFIG_NOT_FOUND');
      expect(err.retryable).toBe(false);
      expect(err.aiGuidance).toBeNull();
    });

    it('subclass with cause/traceId options still work', () => {
      const cause = new Error('root');
      const err = new ModuleTimeoutError('m', 5000, { cause, traceId: 'trace-99' });
      expect(err.cause).toBe(cause);
      expect(err.traceId).toBe('trace-99');
      expect(err.retryable).toBe(true);
    });
  });
});

describe('Approval error subclasses', () => {
  it('ApprovalError base', () => {
    const err = new ApprovalError('APPROVAL_DENIED', 'denied', {}, 'mod.x');
    expect(err.name).toBe('ApprovalError');
    expect(err.code).toBe('APPROVAL_DENIED');
    expect(err.moduleId).toBe('mod.x');
    expect(err.retryable).toBe(false);
  });

  it('ApprovalDeniedError', () => {
    const result = { reason: 'not allowed' };
    const err = new ApprovalDeniedError(result, 'mod.x');
    expect(err.name).toBe('ApprovalDeniedError');
    expect(err.code).toBe('APPROVAL_DENIED');
    expect(err.message).toContain('not allowed');
    expect(err.retryable).toBe(false);
  });

  it('ApprovalTimeoutError', () => {
    const err = new ApprovalTimeoutError({}, 'mod.x');
    expect(err.name).toBe('ApprovalTimeoutError');
    expect(err.code).toBe('APPROVAL_TIMEOUT');
    expect(err.retryable).toBe(true);
  });

  it('ApprovalPendingError', () => {
    const result = { approvalId: 'abc-123' };
    const err = new ApprovalPendingError(result, 'mod.x');
    expect(err.name).toBe('ApprovalPendingError');
    expect(err.code).toBe('APPROVAL_PENDING');
    expect(err.approvalId).toBe('abc-123');
    expect(err.retryable).toBe(false);
  });

  it('ApprovalDeniedError with options', () => {
    const cause = new Error('root');
    const err = new ApprovalDeniedError({}, 'mod.x', { cause, traceId: 'trace-1' });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe('trace-1');
  });

  it('ApprovalTimeoutError with options', () => {
    const err = new ApprovalTimeoutError({}, 'mod.x', { traceId: 'trace-2' });
    expect(err.traceId).toBe('trace-2');
  });

  it('ApprovalTimeoutError with AI field overrides', () => {
    const err = new ApprovalTimeoutError({}, 'mod.x', {
      retryable: false,
      aiGuidance: 'do not retry automatically',
      userFixable: true,
      suggestion: 'Contact the approver directly',
    });
    expect(err.retryable).toBe(false);
    expect(err.aiGuidance).toBe('do not retry automatically');
    expect(err.userFixable).toBe(true);
    expect(err.suggestion).toBe('Contact the approver directly');
  });

  it('ApprovalPendingError with options', () => {
    const err = new ApprovalPendingError({}, 'mod.x', { traceId: 'trace-3' });
    expect(err.traceId).toBe('trace-3');
  });

  it('ApprovalPendingError with AI field overrides', () => {
    const err = new ApprovalPendingError({}, 'mod.x', {
      aiGuidance: 'poll for approval status',
      suggestion: 'Wait for approval or contact approver',
    });
    expect(err.retryable).toBe(false);
    expect(err.aiGuidance).toBe('poll for approval status');
    expect(err.suggestion).toBe('Wait for approval or contact approver');
  });
});

describe('ConfigEnvMapConflictError', () => {
  it('creates error with correct code and message', () => {
    const err = new ConfigEnvMapConflictError('MY_VAR', 'config.owner');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('CONFIG_ENV_MAP_CONFLICT');
    expect(err.message).toContain('MY_VAR');
    expect(err.message).toContain('config.owner');
    expect(err.name).toBe('ConfigEnvMapConflictError');
  });

  it('passes options to parent constructor', () => {
    const cause = new Error('root');
    const err = new ConfigEnvMapConflictError('MY_VAR', 'config.owner', {
      cause,
      traceId: 'trace-abc',
      retryable: false,
      aiGuidance: 'check config',
      userFixable: true,
      suggestion: 'remove duplicate mapping',
    });
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe('trace-abc');
    expect(err.retryable).toBe(false);
  });
});

describe('ConfigBindError', () => {
  it('passes options to parent constructor', () => {
    const cause = new Error('root');
    const err = new ConfigBindError('bind failed', {
      cause,
      traceId: 'trace-xyz',
      retryable: false,
      aiGuidance: 'check config',
      userFixable: true,
      suggestion: 'fix binding',
    });
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('CONFIG_BIND_ERROR');
    expect(err.cause).toBe(cause);
    expect(err.traceId).toBe('trace-xyz');
  });
});

describe('ErrorFormatterDuplicateError', () => {
  it('passes options to parent constructor', () => {
    const cause = new Error('dup');
    const err = new ErrorFormatterDuplicateError('json', {
      cause,
      traceId: 'trace-fmt',
      retryable: false,
      aiGuidance: 'use unique names',
      userFixable: false,
      suggestion: 'rename adapter',
    });
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('ERROR_FORMATTER_DUPLICATE');
    expect(err.cause).toBe(cause);
    expect(err.message).toContain('json');
  });
});
