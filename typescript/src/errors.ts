/**
 * Error hierarchy for apcore.
 */

/**
 * Declarative `user_fixable` policy, keyed by error code (single source of
 * truth; kept in lock-step with
 * `apcore/conformance/fixtures/error_recovery_metadata.json` so the language
 * SDKs agree). `true` = the caller can resolve it by changing the input or
 * configuration they sent; `false` = governance / system / structural /
 * transient, not resolvable by changing input. Codes absent here leave
 * `userFixable` unset (e.g. `MODULE_EXECUTE_ERROR` — the module author supplies
 * the recovery guidance).
 *
 * @internal Not part of the public package API — absent from the `index.ts` barrel
 * (and stripped from the published `.d.ts`); exported only so the conformance test
 * can assert fixture↔source parity. Mirrors Python's private `_USER_FIXABLE_BY_CODE`
 * and Rust's `#[doc(hidden)]` `user_fixable_for_code`.
 */
export const USER_FIXABLE_BY_CODE: Readonly<Record<string, boolean>> = Object.freeze({
  // Caller can fix by changing input/config:
  SCHEMA_VALIDATION_ERROR: true,
  GENERAL_INVALID_INPUT: true,
  MODULE_NOT_FOUND: true,
  VERSION_CONSTRAINT_INVALID: true,
  BINDING_SCHEMA_INFERENCE_FAILED: true,
  BINDING_SCHEMA_MODE_CONFLICT: true,
  BINDING_STRICT_SCHEMA_INCOMPATIBLE: true,
  DEPENDENCY_NOT_FOUND: true,
  DEPENDENCY_VERSION_MISMATCH: true,
  // Governance / system / structural / transient — not caller-fixable by input:
  ACL_DENIED: false,
  APPROVAL_DENIED: false,
  APPROVAL_TIMEOUT: false,
  MODULE_TIMEOUT: false,
  MODULE_DISABLED: false,
  CALL_DEPTH_EXCEEDED: false,
  CIRCULAR_CALL: false,
  CALL_FREQUENCY_EXCEEDED: false,
  GENERAL_INTERNAL_ERROR: false,
});

export interface ErrorOptions {
  cause?: Error;
  traceId?: string;
  retryable?: boolean | null;
  aiGuidance?: string | null;
  userFixable?: boolean | null;
  suggestion?: string | null;
  /**
   * Optional structured details payload. Most subclasses build their own
   * `details` from positional constructor args and ignore this field;
   * subclasses that take no detail-bearing positional args (e.g.
   * InvalidInputError) read it so callers can attach a machine-readable
   * payload (e.g. the STREAM_CHUNK_NOT_OBJECT contract) without changing
   * the existing positional parameter order.
   */
  details?: Record<string, unknown>;
}

export class ModuleError extends Error {
  static readonly DEFAULT_RETRYABLE: boolean | null = null;

  readonly code: string;
  readonly details: Record<string, unknown>;
  override readonly cause?: Error;
  readonly traceId?: string;
  readonly timestamp: string;
  readonly retryable: boolean | null;
  readonly aiGuidance: string | null;
  readonly userFixable: boolean | null;
  readonly suggestion: string | null;

  constructor(
    code: string,
    message: string,
    details?: Record<string, unknown>,
    cause?: Error,
    traceId?: string,
    retryable?: boolean | null,
    aiGuidance?: string | null,
    userFixable?: boolean | null,
    suggestion?: string | null,
  ) {
    super(message, cause ? { cause } : undefined);
    this.name = 'ModuleError';
    this.code = code;
    this.details = details ?? {};
    this.cause = cause;
    this.traceId = traceId;
    this.timestamp = new Date().toISOString();
    this.retryable =
      retryable !== undefined
        ? retryable
        : (this.constructor as typeof ModuleError).DEFAULT_RETRYABLE;
    this.aiGuidance = aiGuidance ?? null;
    // Resolve user_fixable from the error code when not explicitly provided
    // (mirrors Python's `_UNSET` sentinel). An explicit value — including an
    // explicit `null` — wins; codes absent from the map leave it null/unset so
    // serialization omits it sparsely.
    this.userFixable =
      userFixable !== undefined ? userFixable : (USER_FIXABLE_BY_CODE[code] ?? null);
    this.suggestion = suggestion ?? null;
  }

  override toString(): string {
    return `[${this.code}] ${this.message}`;
  }

  /**
   * Serialize to the canonical cross-process wire form (A-D-008).
   *
   * Keys are emitted in snake_case — `trace_id`, `ai_guidance`,
   * `user_fixable` — and the `details` object's keys are snake_cased too,
   * so the JSON matches the Python (`errors.py` `to_dict`), Rust (serde),
   * and the spec serialization example. This is the form MCP/A2A bridges
   * put on the wire. Internal TS consumers read the typed properties
   * (`traceId`, `details['callerId']`, ...), not this serialized output.
   */
  toJSON(): Record<string, unknown> {
    const obj: Record<string, unknown> = {
      code: this.code,
      message: this.message,
    };
    if (Object.keys(this.details).length > 0) {
      obj.details = snakeCaseKeys(this.details);
    }
    if (this.cause !== undefined) {
      obj.cause = this.cause instanceof Error ? this.cause.message : String(this.cause);
    }
    if (this.traceId !== undefined) {
      obj.trace_id = this.traceId;
    }
    obj.timestamp = this.timestamp;
    if (this.retryable !== null) {
      obj.retryable = this.retryable;
    }
    if (this.aiGuidance !== null) {
      obj.ai_guidance = this.aiGuidance;
    }
    if (this.userFixable !== null) {
      obj.user_fixable = this.userFixable;
    }
    if (this.suggestion !== null) {
      obj.suggestion = this.suggestion;
    }
    return obj;
  }
}

/** Convert a camelCase identifier to snake_case. */
function camelToSnake(key: string): string {
  return key.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();
}

/**
 * Return a shallow copy of `details` with top-level keys snake_cased, for
 * cross-process wire output (A-D-008). Values are passed through unchanged —
 * detail values are scalars or arrays of scalars, never nested objects whose
 * keys need conversion.
 */
function snakeCaseKeys(details: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(details)) {
    out[camelToSnake(k)] = v;
  }
  return out;
}

export class ConfigNotFoundError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(configPath: string, options?: ErrorOptions) {
    super(
      'CONFIG_NOT_FOUND',
      `Configuration file not found: ${configPath}`,
      { configPath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigNotFoundError';
  }
}

export class ConfigError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(message: string, options?: ErrorOptions) {
    super(
      'CONFIG_INVALID',
      message,
      options?.details ?? {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigError';
  }
}

export class ACLRuleError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(message: string, options?: ErrorOptions) {
    super(
      'ACL_RULE_ERROR',
      message,
      {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ACLRuleError';
  }
}

export class ACLDeniedError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(callerId: string | null, targetId: string, options?: ErrorOptions) {
    super(
      'ACL_DENIED',
      `Access denied: ${callerId} -> ${targetId}`,
      { callerId, targetId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Access denied for '${callerId}' calling '${targetId}'. Verify the caller has the required role or permission, or try an alternative module with similar functionality.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ACLDeniedError';
  }

  get callerId(): string | null {
    return this.details['callerId'] as string | null;
  }

  get targetId(): string {
    return this.details['targetId'] as string;
  }
}

export class ModuleNotFoundError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, options?: ErrorOptions) {
    super(
      'MODULE_NOT_FOUND',
      `Module not found: ${moduleId}`,
      { moduleId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Module '${moduleId}' does not exist in the registry. Verify the module ID spelling. Use system.manifest.full to list available modules.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ModuleNotFoundError';
  }
}

export class ModuleDisabledError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, options?: ErrorOptions) {
    super(
      'MODULE_DISABLED',
      `Module is disabled: ${moduleId}`,
      { moduleId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Module '${moduleId}' is currently disabled. Use system.control.toggle_feature to re-enable it, or find an alternative module.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ModuleDisabledError';
  }
}

export class ModuleTimeoutError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(moduleId: string, timeoutMs: number, options?: ErrorOptions) {
    super(
      'MODULE_TIMEOUT',
      `Module ${moduleId} timed out after ${timeoutMs}ms`,
      { moduleId, timeoutMs },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Module '${moduleId}' timed out after ${timeoutMs}ms. Consider: 1) Breaking the operation into smaller steps. 2) Reducing the input data size. 3) Asking the user if a longer timeout is acceptable.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ModuleTimeoutError';
  }

  get moduleId(): string {
    return this.details['moduleId'] as string;
  }

  get timeoutMs(): number {
    return this.details['timeoutMs'] as number;
  }
}

export class SchemaValidationError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(
    message: string = 'Schema validation failed',
    errors?: Array<Record<string, unknown>>,
    options?: ErrorOptions,
    errorCode: string = 'SCHEMA_VALIDATION_ERROR',
  ) {
    super(
      errorCode,
      message,
      { errors: errors ?? [] },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        'Input validation failed. Review the error details to identify which fields have invalid values, then correct them or ask the user for valid input.',
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'SchemaValidationError';
  }
}

export class SchemaNotFoundError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(schemaId: string, options?: ErrorOptions) {
    super(
      'SCHEMA_NOT_FOUND',
      `Schema not found: ${schemaId}`,
      { schemaId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'SchemaNotFoundError';
  }
}

export class SchemaParseError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(message: string, options?: ErrorOptions) {
    super(
      'SCHEMA_PARSE_ERROR',
      message,
      {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'SchemaParseError';
  }
}

export class SchemaCircularRefError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(refPath: string, options?: ErrorOptions) {
    super(
      'SCHEMA_CIRCULAR_REF',
      `Circular reference detected: ${refPath}`,
      { refPath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'SchemaCircularRefError';
  }
}

export class SchemaMaxDepthExceededError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(refPath: string, options?: ErrorOptions) {
    super(
      'SCHEMA_MAX_DEPTH_EXCEEDED',
      `Maximum reference depth exceeded resolving: ${refPath}`,
      { refPath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'SchemaMaxDepthExceededError';
  }
}

export class CallDepthExceededError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(depth: number, maxDepth: number, callChain: string[], options?: ErrorOptions) {
    super(
      'CALL_DEPTH_EXCEEDED',
      `Call depth ${depth} exceeds maximum ${maxDepth}`,
      // snake_case detail keys for cross-language parity with the spec,
      // apcore-python, and apcore-rust (sync finding A-D-019).
      { depth, max_depth: maxDepth, call_chain: callChain },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Call depth ${depth} exceeds maximum ${maxDepth}. Simplify the module call chain or restructure to reduce nesting depth.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'CallDepthExceededError';
  }

  get currentDepth(): number {
    return this.details['depth'] as number;
  }

  get maxDepth(): number {
    return this.details['max_depth'] as number;
  }
}

export class CircularCallError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, callChain: string[], options?: ErrorOptions) {
    super(
      'CIRCULAR_CALL',
      `Circular call detected for module ${moduleId}`,
      // snake_case detail keys (sync finding A-D-019).
      { module_id: moduleId, call_chain: callChain },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        'A circular call was detected in the module call chain. Review the call_chain in error details and restructure to eliminate the cycle.',
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'CircularCallError';
  }

  get moduleId(): string {
    return this.details['module_id'] as string;
  }
}

export class CallFrequencyExceededError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(
    moduleId: string,
    count: number,
    maxRepeat: number,
    callChain: string[],
    options?: ErrorOptions,
  ) {
    super(
      'CALL_FREQUENCY_EXCEEDED',
      `Module ${moduleId} called ${count} times, max is ${maxRepeat}`,
      // snake_case detail keys (sync finding A-D-019).
      { module_id: moduleId, count, max_repeat: maxRepeat, call_chain: callChain },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Module '${moduleId}' was called ${count} times in this chain (limit ${maxRepeat}), tripping the frequency guard. Reduce repeated calls or batch the work before retrying.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'CallFrequencyExceededError';
  }

  get moduleId(): string {
    return this.details['module_id'] as string;
  }

  get count(): number {
    return this.details['count'] as number;
  }

  get maxRepeat(): number {
    return this.details['max_repeat'] as number;
  }
}

export class InvalidInputError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  /**
   * @param code Error-code override. Defaults to `GENERAL_INVALID_INPUT`;
   *   module-id format validation passes `INVALID_MODULE_ID` (mirrors Python's
   *   `InvalidInputError(code=...)`).
   */
  constructor(
    message: string = 'Invalid input',
    options?: ErrorOptions,
    code: string = 'GENERAL_INVALID_INPUT',
  ) {
    super(
      code,
      message,
      options?.details ?? {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        "The input was malformed or missing required fields. Check the values against the module's input_schema and retry with corrected input.",
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'InvalidInputError';
  }
}

export class BindingInvalidTargetError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(target: string, options?: ErrorOptions) {
    super(
      'BINDING_INVALID_TARGET',
      `Invalid binding target '${target}'. Expected format: 'module.path:callable_name'.`,
      { target },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingInvalidTargetError';
  }
}

export class BindingModuleNotFoundError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(modulePath: string, options?: ErrorOptions) {
    super(
      'BINDING_MODULE_NOT_FOUND',
      `Cannot import module '${modulePath}'.`,
      { modulePath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingModuleNotFoundError';
  }
}

export class BindingCallableNotFoundError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(callableName: string, modulePath: string, options?: ErrorOptions) {
    super(
      'BINDING_CALLABLE_NOT_FOUND',
      `Cannot find callable '${callableName}' in module '${modulePath}'.`,
      { callableName, modulePath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingCallableNotFoundError';
  }
}

export class BindingNotCallableError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(target: string, options?: ErrorOptions) {
    super(
      'BINDING_NOT_CALLABLE',
      `Resolved target '${target}' is not callable.`,
      { target },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingNotCallableError';
  }
}

export class BindingSchemaInferenceFailedError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(
    target: string,
    moduleId?: string,
    filePath?: string,
    remediation?: string,
    options?: ErrorOptions,
  ) {
    const loc = filePath ? `${filePath}: ` : '';
    const modPart = moduleId ? `binding '${moduleId}' ` : '';
    const rem =
      remediation ??
      'no schema source detected. Provide one of: target input/output as TypeBox schemas, zod schemas, ' +
        'class-validator-decorated DTO, or use the typia adapter. ' +
        'Alternatively specify input_schema/output_schema explicitly.';
    super(
      'BINDING_SCHEMA_INFERENCE_FAILED',
      `${loc}${modPart}auto schema inference failed for target '${target}'. ${rem} See protocol-spec §5.12.5`,
      { target, moduleId, filePath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingSchemaInferenceFailedError';
  }
}

/**
 * Deprecated alias for {@link BindingSchemaInferenceFailedError}.
 * Per protocol-spec §5.12.8, the canonical name is
 * BindingSchemaInferenceFailedError. Emits a one-shot deprecation warning on
 * first construction.
 *
 * The removal target is **1.0.0**. An earlier revision of this comment also
 * named 0.20.0 in its prose while the `@deprecated` tag said 1.0.0; 0.20.0
 * shipped in May 2026 with the alias still present, so the prose deadline was
 * both contradictory and already lapsed. 1.0.0 is the operative one.
 *
 * @deprecated Will be removed in 1.0.0. Use {@link BindingSchemaInferenceFailedError} instead.
 */
let _bindingSchemaMissingWarned = false;
export class BindingSchemaMissingError extends BindingSchemaInferenceFailedError {
  constructor(
    target: string,
    moduleId?: string,
    filePath?: string,
    remediation?: string,
    options?: ErrorOptions,
  ) {
    super(target, moduleId, filePath, remediation, options);
    if (!_bindingSchemaMissingWarned) {
      _bindingSchemaMissingWarned = true;
      console.warn(
        '[apcore:errors] BindingSchemaMissingError is deprecated and scheduled for removal in 0.20.0. ' +
          'Use BindingSchemaInferenceFailedError instead. See protocol-spec §5.12.8',
      );
    }
  }
}

export class BindingSchemaModeConflictError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, modesListed: string[], filePath?: string, options?: ErrorOptions) {
    const loc = filePath ? `${filePath}: ` : '';
    super(
      'BINDING_SCHEMA_MODE_CONFLICT',
      `${loc}binding '${moduleId}' specifies multiple schema modes (${modesListed.join(', ')}). ` +
        'Choose one. See protocol-spec §5.12.2',
      { moduleId, modesListed, filePath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingSchemaModeConflictError';
  }
}

/**
 * Raised when auto_schema: strict is requested but the inferred schema contains
 * features incompatible with strict mode (e.g., `anyOf`, `oneOf`, recursive `$ref`).
 * See protocol-spec §5.12.5.
 */
export class BindingStrictSchemaIncompatibleError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(
    moduleId: string,
    featuresListed: string[],
    filePath?: string,
    line?: number,
    options?: ErrorOptions,
  ) {
    const loc = filePath ? (line !== undefined ? `${filePath}:${line}: ` : `${filePath}: `) : '';
    super(
      'BINDING_STRICT_SCHEMA_INCOMPATIBLE',
      `${loc}binding '${moduleId}' uses auto_schema: strict but inferred schema ` +
        `contains incompatible features: ${featuresListed.join(', ')}. ` +
        'See protocol-spec §5.12.5',
      { moduleId, featuresListed, filePath, line },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingStrictSchemaIncompatibleError';
  }
}

/**
 * Raised when a function parameter has no type annotation or a forward
 * reference cannot be resolved during auto-schema inference.
 */
export class FuncMissingTypeHintError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(functionName: string, parameterName: string, options?: ErrorOptions) {
    super(
      'FUNC_MISSING_TYPE_HINT',
      `Parameter '${parameterName}' in function '${functionName}' has no type annotation. ` +
        `Add a type hint like '${parameterName}: string'.`,
      { functionName, parameterName },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'FuncMissingTypeHintError';
  }
}

/**
 * Raised when a function has no return type annotation during auto-schema
 * inference.
 */
export class FuncMissingReturnTypeError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(functionName: string, options?: ErrorOptions) {
    super(
      'FUNC_MISSING_RETURN_TYPE',
      `Function '${functionName}' has no return type annotation. Add a return type like ': Promise<Record<string, unknown>>'.`,
      { functionName },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'FuncMissingReturnTypeError';
  }
}

export class BindingFileInvalidError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(filePath: string, reason: string, options?: ErrorOptions) {
    super(
      'BINDING_FILE_INVALID',
      `Invalid binding file '${filePath}': ${reason}`,
      { filePath, reason },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'BindingFileInvalidError';
  }
}

export class CircularDependencyError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(cyclePath: string[], options?: ErrorOptions) {
    super(
      'CIRCULAR_DEPENDENCY',
      `Circular dependency detected: ${cyclePath.join(' -> ')}`,
      { cyclePath },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'CircularDependencyError';
  }
}

export class ModuleLoadError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, reason: string, options?: ErrorOptions) {
    super(
      'MODULE_LOAD_ERROR',
      `Failed to load module '${moduleId}': ${reason}`,
      { moduleId, reason },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ModuleLoadError';
  }
}

export class DependencyNotFoundError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, dependencyId: string, options?: ErrorOptions) {
    super(
      'DEPENDENCY_NOT_FOUND',
      `Module '${moduleId}' has unsatisfied required dependency '${dependencyId}'`,
      { moduleId, dependencyId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Module '${moduleId}' declares a required dependency on '${dependencyId}', but no such module is registered. Either register '${dependencyId}' before loading '${moduleId}', mark the dependency as optional, or remove it.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'DependencyNotFoundError';
  }
}

export class DependencyVersionMismatchError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(
    moduleId: string,
    dependencyId: string,
    required: string,
    actual: string,
    options?: ErrorOptions,
  ) {
    super(
      'DEPENDENCY_VERSION_MISMATCH',
      `Module '${moduleId}' requires dependency '${dependencyId}' version '${required}', but registered version is '${actual}'`,
      { moduleId, dependencyId, required, actual },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Module '${moduleId}' declares dependency '${dependencyId}' with version constraint '${required}', but the registered version is '${actual}'. Either upgrade the dependency, relax the constraint, or register a compatible version.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'DependencyVersionMismatchError';
  }
}

export class ReloadFailedError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(moduleId: string, reason: string, options?: ErrorOptions) {
    super(
      'RELOAD_FAILED',
      `Failed to reload module '${moduleId}': ${reason}`,
      { moduleId, reason },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ReloadFailedError';
  }
}

export class ModuleExecuteError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = null;

  constructor(moduleId: string, reason: string, options?: ErrorOptions) {
    super(
      'MODULE_EXECUTE_ERROR',
      `Failed to execute module '${moduleId}': ${reason}`,
      { moduleId, reason },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ModuleExecuteError';
  }
}

export class InternalError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(message: string = 'Internal error', options?: ErrorOptions) {
    super(
      'GENERAL_INTERNAL_ERROR',
      message,
      {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'InternalError';
  }
}

/**
 * Base error for all approval-related errors.
 * Carries the full ApprovalResult for inspection by callers.
 */
export class ApprovalError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  readonly result: unknown;

  constructor(
    code: string,
    message: string,
    result: unknown,
    moduleId?: string,
    options?: ErrorOptions,
  ) {
    super(
      code,
      message,
      { moduleId: moduleId ?? null },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ApprovalError';
    this.result = result;
  }

  get moduleId(): string | null {
    return this.details['moduleId'] as string | null;
  }

  get reason(): string | null {
    const r = this.result as Record<string, unknown> | null;
    return (r?.['reason'] as string) ?? null;
  }
}

export class ApprovalDeniedError extends ApprovalError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(result: unknown, moduleId: string = '', options?: ErrorOptions) {
    const reason = (result as Record<string, unknown>)?.['reason'] as string | undefined;
    let msg = `Approval denied for module '${moduleId}'`;
    if (reason) {
      msg += `: ${reason}`;
    }
    super('APPROVAL_DENIED', msg, result, moduleId, options);
    this.name = 'ApprovalDeniedError';
  }
}

export class ApprovalTimeoutError extends ApprovalError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(result: unknown, moduleId: string = '', options?: ErrorOptions) {
    super(
      'APPROVAL_TIMEOUT',
      `Approval timed out for module '${moduleId}'`,
      result,
      moduleId,
      options,
    );
    this.name = 'ApprovalTimeoutError';
  }
}

export class ApprovalPendingError extends ApprovalError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(result: unknown, moduleId: string = '', options?: ErrorOptions) {
    const approvalId = (result as Record<string, unknown>)?.['approvalId'] as string | undefined;
    super(
      'APPROVAL_PENDING',
      `Approval pending for module '${moduleId}'`,
      result,
      moduleId,
      options,
    );
    this.name = 'ApprovalPendingError';
    this.details['approvalId'] = approvalId ?? null;
  }

  get approvalId(): string | null {
    return this.details['approvalId'] as string | null;
  }
}

export class ConfigNamespaceDuplicateError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(namespace: string, options?: ErrorOptions) {
    super(
      'CONFIG_NAMESPACE_DUPLICATE',
      `Namespace already registered: '${namespace}'`,
      { namespace },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigNamespaceDuplicateError';
  }
}

export class ConfigNamespaceReservedError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(namespace: string, options?: ErrorOptions) {
    super(
      'CONFIG_NAMESPACE_RESERVED',
      `Namespace is reserved and cannot be registered: '${namespace}'`,
      { namespace },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigNamespaceReservedError';
  }
}

export class ConfigEnvPrefixConflictError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(envPrefix: string, options?: ErrorOptions) {
    super(
      'CONFIG_ENV_PREFIX_CONFLICT',
      `Environment variable prefix already in use or reserved: '${envPrefix}'`,
      { envPrefix },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigEnvPrefixConflictError';
  }
}

export class ConfigEnvMapConflictError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(envVar: string, owner: string, options?: ErrorOptions) {
    super(
      'CONFIG_ENV_MAP_CONFLICT',
      `Environment variable '${envVar}' is already mapped by '${owner}'`,
      { envVar, owner },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigEnvMapConflictError';
  }
}

export class ConfigMountError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(message: string, options?: ErrorOptions) {
    super(
      'CONFIG_MOUNT_ERROR',
      message,
      {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigMountError';
  }
}

export class ConfigBindError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(message: string, options?: ErrorOptions) {
    super(
      'CONFIG_BIND_ERROR',
      message,
      {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ConfigBindError';
  }
}

export class ErrorFormatterDuplicateError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(adapterName: string, options?: ErrorOptions) {
    super(
      'ERROR_FORMATTER_DUPLICATE',
      `Error formatter already registered for adapter: '${adapterName}'`,
      { adapterName },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ErrorFormatterDuplicateError';
  }
}

export class TaskLimitExceededError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(maxTasks: number, options?: ErrorOptions) {
    super(
      'TASK_LIMIT_EXCEEDED',
      `Task limit reached (${maxTasks})`,
      { maxTasks },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'TaskLimitExceededError';
  }
}

/**
 * Raised when a `TaskStore` backend is unreachable or refuses an operation.
 *
 * Spec v1.50.0, D-92. `async-tasks.md` declares
 * `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` on eight surfaces — every
 * `TaskStore` method and every `AsyncTaskManager` method that touches the
 * store — and no SDK defined it, so no caller could ever catch it. A declared
 * error type that no implementation can raise is the "declared surface reaches
 * no mechanism" shape PROTOCOL_SPEC §9.1.3 forbids for configuration keys, here
 * applied to an error contract.
 *
 * The bundled {@link InMemoryTaskStore} cannot fail and therefore never raises
 * this; that is exactly why the type is **exported** rather than merely raised
 * internally. The hosts who need it are the ones writing the network-backed
 * stores the contract was written for: they raise it, and `AsyncTaskManager`
 * propagates it (D-81) rather than mapping a store outage onto "task not found"
 * / an empty list / a `cancel` that returns true for a save that never landed.
 *
 * Landed here late: D-92 was implemented in apcore-python in the v1.50.0 wave
 * and missed in this SDK and apcore-rust. Nothing noticed until
 * `error_codes.json` gained a case asserting that the code is framework-reserved
 * — an SDK that has never defined it does not collide, so the case is red for
 * exactly the state the decision corrects.
 */
export class TaskStoreError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(operation = '', reason = '', options?: ErrorOptions) {
    let detail = operation
      ? `TaskStore operation '${operation}' failed`
      : 'TaskStore is unavailable';
    if (reason) detail = `${detail}: ${reason}`;
    const details: Record<string, unknown> = {};
    if (operation) details['operation'] = operation;
    if (reason) details['reason'] = reason;
    super(
      'TASK_STORE_UNAVAILABLE',
      detail,
      Object.keys(details).length > 0 ? details : undefined,
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'TaskStoreError';
  }
}

export class VersionConstraintError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(constraint: string, reason: string, options?: ErrorOptions) {
    super(
      'VERSION_CONSTRAINT_INVALID',
      `Invalid version constraint '${constraint}': ${reason}`,
      { constraint, reason },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `Constraint '${constraint}' is not a valid semver expression. ` +
          `Use forms like '1.2.3', '>=1.2.0,<2.0.0', '^1.2.3', or '~1.2'. ${reason}`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'VersionConstraintError';
  }
}

export class ModuleIdConflictError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(
    filePath: string,
    classNames: [string, string],
    conflictingSegment: string,
    options?: ErrorOptions,
  ) {
    super(
      'MODULE_ID_CONFLICT',
      `Module ID conflict in '${filePath}': classes '${classNames[0]}' and '${classNames[1]}' both produce segment '${conflictingSegment}'`,
      { filePath, classNames, conflictingSegment },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ModuleIdConflictError';
  }
}

export class InvalidSegmentError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(
    filePath: string,
    className: string,
    segment: string,
    options?: ErrorOptions,
  ) {
    super(
      'INVALID_SEGMENT',
      `Class '${className}' in '${filePath}' produces invalid segment '${segment}' — does not match ^[a-z][a-z0-9_]*$`,
      { filePath, className, segment },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'InvalidSegmentError';
  }
}

export class CircuitBreakerOpenError extends ModuleError {
  // The circuit closes again after its recovery window, so the same call
  // later may succeed (protocol-spec §8.6, D-135).
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(moduleId: string, callerId: string | null = null, options?: ErrorOptions) {
    super(
      'CIRCUIT_BREAKER_OPEN',
      `Circuit breaker OPEN for module '${moduleId}' (caller: ${callerId ?? 'unknown'}). Wait for the recovery window.`,
      { moduleId, callerId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        `The circuit breaker for '${moduleId}' is open due to high error rates. The module call was short-circuited. Wait for the recovery window to elapse before retrying.`,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'CircuitBreakerOpenError';
  }

  get moduleId(): string {
    return this.details['moduleId'] as string;
  }

  get callerId(): string | null {
    return this.details['callerId'] as string | null;
  }
}

export class IdTooLongError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(filePath: string, moduleId: string, options?: ErrorOptions) {
    super(
      'ID_TOO_LONG',
      `Derived module ID exceeds 192 characters in '${filePath}': '${moduleId}' (${moduleId.length} chars)`,
      { filePath, moduleId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'IdTooLongError';
  }
}

export class ModuleReloadConflictError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(options?: ErrorOptions) {
    super(
      'MODULE_RELOAD_CONFLICT',
      "'module_id' and 'path_filter' are mutually exclusive; provide exactly one",
      {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ModuleReloadConflictError';
  }
}

export class SysModuleRegistrationError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, cause?: Error, options?: ErrorOptions) {
    super(
      'SYS_MODULE_REGISTRATION_FAILED',
      `System module registration failed: ${moduleId}`,
      { moduleId },
      cause ?? options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'SysModuleRegistrationError';
  }
}

/**
 * Raised when an APCore client method requires sys_modules to be enabled.
 *
 * Cross-language equivalent of Rust's `ModuleError(code=SysModulesDisabled)`
 * and Python's `SysModulesDisabledError`. Replaces bare `Error` in
 * `APCore.on/off/disable/enable` so callers can dispatch on
 * `error.code === 'SYS_MODULES_DISABLED'` instead of catching generic
 * `Error`.
 */
export class SysModulesDisabledError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(message: string, options?: ErrorOptions) {
    super(
      'SYS_MODULES_DISABLED',
      message,
      undefined,
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'SysModulesDisabledError';
  }
}

/**
 * Raised when `TraceContext.inject` gets a `parentId` override that is not 16
 * lowercase hex characters (`^[0-9a-f]{16}$`).
 *
 * `userFixable` defaults to `true` on the class rather than through
 * `USER_FIXABLE_BY_CODE`, because that map is asserted equal to
 * `error_recovery_metadata.json`, which does not enumerate this code. The value
 * matches apcore-python `InvalidParentIdError` and apcore-rust
 * `user_fixable_for_code`: the caller fixes it by passing a valid span id.
 */
export class InvalidParentIdError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(parentId: string, options?: ErrorOptions) {
    super(
      'INVALID_PARENT_ID',
      `Malformed parentId override: ${JSON.stringify(parentId)}. Expected 16 lowercase hex chars matching /^[0-9a-f]{16}$/.`,
      { parentId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        'The parentId override passed to TraceContext.inject() must be 16 lowercase hex ' +
          'characters (^[0-9a-f]{16}$). Pass a valid span id or omit the argument to let ' +
          'apcore derive one.',
      options?.userFixable !== undefined ? options.userFixable : true,
      options?.suggestion,
    );
    this.name = 'InvalidParentIdError';
  }
}

export class DuplicateModuleIdError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(moduleId: string, options?: ErrorOptions) {
    super(
      'DUPLICATE_MODULE_ID',
      `Module ID '${moduleId}' is already registered`,
      { moduleId },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'DuplicateModuleIdError';
  }
}

export class StreamingInterfaceError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  readonly moduleId: string;
  readonly annotationValue: boolean;
  readonly hasStreamMethod: boolean;
  readonly hasMarker: boolean;

  constructor(
    moduleId: string,
    annotationValue: boolean,
    hasStreamMethod: boolean,
    hasMarker: boolean,
    options?: ErrorOptions,
  ) {
    super(
      'STREAMING_INTERFACE_MISMATCH',
      `Module '${moduleId}' declares annotations.streaming=true but does not implement the StreamingModule interface (hasStreamMethod=${hasStreamMethod}, hasMarker=${hasMarker})`,
      { moduleId, annotationValue, hasStreamMethod, hasMarker },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'StreamingInterfaceError';
    this.moduleId = moduleId;
    this.annotationValue = annotationValue;
    this.hasStreamMethod = hasStreamMethod;
    this.hasMarker = hasMarker;
  }
}

/**
 * Raised when a Context that is already bound to one Executor (or CancelToken)
 * is offered to a different Executor (or CancelToken). Per apcore Issue #66 /
 * v0.22.0 spec "Contract: Executor binding to Context", binding is a one-shot
 * operation: idempotent for the same instance, error for a different one.
 */
export class ContextBindingError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(message: string, options?: ErrorOptions) {
    super(
      'CONTEXT_BINDING_ERROR',
      message,
      {},
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance ??
        'A Context was offered to an Executor (or attached to a CancelToken) ' +
          'that does not match the one it is already bound to. Create a fresh ' +
          'Context via Context.create() for the new Executor / CancelToken, or ' +
          'reuse the existing bound Context across calls on the same Executor.',
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ContextBindingError';
  }
}

/**
 * All framework error codes as constants.
 * Use these instead of hardcoding error code strings.
 */
export const ErrorCodes = Object.freeze({
  CONFIG_NOT_FOUND: 'CONFIG_NOT_FOUND',
  CONFIG_INVALID: 'CONFIG_INVALID',
  CONFIG_KEY_RESTRICTED: 'CONFIG_KEY_RESTRICTED',
  CONFIG_NAMESPACE_DUPLICATE: 'CONFIG_NAMESPACE_DUPLICATE',
  CONFIG_NAMESPACE_RESERVED: 'CONFIG_NAMESPACE_RESERVED',
  CONFIG_ENV_PREFIX_CONFLICT: 'CONFIG_ENV_PREFIX_CONFLICT',
  CONFIG_MOUNT_ERROR: 'CONFIG_MOUNT_ERROR',
  CONFIG_BIND_ERROR: 'CONFIG_BIND_ERROR',
  CONFIG_ENV_MAP_CONFLICT: 'CONFIG_ENV_MAP_CONFLICT' as const,
  ERROR_FORMATTER_DUPLICATE: 'ERROR_FORMATTER_DUPLICATE',
  ACL_RULE_ERROR: 'ACL_RULE_ERROR',
  ACL_DENIED: 'ACL_DENIED',
  MODULE_NOT_FOUND: 'MODULE_NOT_FOUND',
  MODULE_DISABLED: 'MODULE_DISABLED',
  MODULE_TIMEOUT: 'MODULE_TIMEOUT',
  MODULE_LOAD_ERROR: 'MODULE_LOAD_ERROR',
  RELOAD_FAILED: 'RELOAD_FAILED',
  EXECUTION_CANCELLED: 'EXECUTION_CANCELLED',
  MODULE_EXECUTE_ERROR: 'MODULE_EXECUTE_ERROR',
  SCHEMA_VALIDATION_ERROR: 'SCHEMA_VALIDATION_ERROR',
  SCHEMA_NOT_FOUND: 'SCHEMA_NOT_FOUND',
  SCHEMA_PARSE_ERROR: 'SCHEMA_PARSE_ERROR',
  SCHEMA_CIRCULAR_REF: 'SCHEMA_CIRCULAR_REF',
  SCHEMA_MAX_DEPTH_EXCEEDED: 'SCHEMA_MAX_DEPTH_EXCEEDED',
  CALL_DEPTH_EXCEEDED: 'CALL_DEPTH_EXCEEDED',
  CIRCULAR_CALL: 'CIRCULAR_CALL',
  CALL_FREQUENCY_EXCEEDED: 'CALL_FREQUENCY_EXCEEDED',
  GENERAL_INVALID_INPUT: 'GENERAL_INVALID_INPUT',
  GENERAL_INTERNAL_ERROR: 'GENERAL_INTERNAL_ERROR',
  INVALID_MODULE_ID: 'INVALID_MODULE_ID',
  FUNC_MISSING_TYPE_HINT: 'FUNC_MISSING_TYPE_HINT',
  FUNC_MISSING_RETURN_TYPE: 'FUNC_MISSING_RETURN_TYPE',
  BINDING_INVALID_TARGET: 'BINDING_INVALID_TARGET',
  BINDING_MODULE_NOT_FOUND: 'BINDING_MODULE_NOT_FOUND',
  BINDING_CALLABLE_NOT_FOUND: 'BINDING_CALLABLE_NOT_FOUND',
  BINDING_NOT_CALLABLE: 'BINDING_NOT_CALLABLE',
  BINDING_SCHEMA_MISSING: 'BINDING_SCHEMA_MISSING',
  BINDING_SCHEMA_INFERENCE_FAILED: 'BINDING_SCHEMA_INFERENCE_FAILED',
  BINDING_SCHEMA_MODE_CONFLICT: 'BINDING_SCHEMA_MODE_CONFLICT',
  BINDING_STRICT_SCHEMA_INCOMPATIBLE: 'BINDING_STRICT_SCHEMA_INCOMPATIBLE',
  BINDING_FILE_INVALID: 'BINDING_FILE_INVALID',
  CIRCULAR_DEPENDENCY: 'CIRCULAR_DEPENDENCY',
  MIDDLEWARE_CHAIN_ERROR: 'MIDDLEWARE_CHAIN_ERROR',
  APPROVAL_DENIED: 'APPROVAL_DENIED',
  APPROVAL_TIMEOUT: 'APPROVAL_TIMEOUT',
  APPROVAL_PENDING: 'APPROVAL_PENDING',
  VERSION_INCOMPATIBLE: 'VERSION_INCOMPATIBLE',
  ERROR_CODE_COLLISION: 'ERROR_CODE_COLLISION',
  GENERAL_NOT_IMPLEMENTED: 'GENERAL_NOT_IMPLEMENTED',
  DEPENDENCY_NOT_FOUND: 'DEPENDENCY_NOT_FOUND',
  DEPENDENCY_VERSION_MISMATCH: 'DEPENDENCY_VERSION_MISMATCH',
  TASK_LIMIT_EXCEEDED: 'TASK_LIMIT_EXCEEDED',
  TASK_STORE_UNAVAILABLE: 'TASK_STORE_UNAVAILABLE',
  REAPER_ALREADY_RUNNING: 'REAPER_ALREADY_RUNNING',
  VERSION_CONSTRAINT_INVALID: 'VERSION_CONSTRAINT_INVALID',
  MODULE_ID_CONFLICT: 'MODULE_ID_CONFLICT',
  INVALID_SEGMENT: 'INVALID_SEGMENT',
  ID_TOO_LONG: 'ID_TOO_LONG',
  CIRCUIT_BREAKER_OPEN: 'CIRCUIT_BREAKER_OPEN',
  MODULE_RELOAD_CONFLICT: 'MODULE_RELOAD_CONFLICT',
  SYS_MODULE_REGISTRATION_FAILED: 'SYS_MODULE_REGISTRATION_FAILED',
  SYS_MODULES_DISABLED: 'SYS_MODULES_DISABLED',
  DUPLICATE_MODULE_ID: 'DUPLICATE_MODULE_ID',
  STREAMING_INTERFACE_MISMATCH: 'STREAMING_INTERFACE_MISMATCH',
  CONTEXT_BINDING_ERROR: 'CONTEXT_BINDING_ERROR',
  PIPELINE_STEP_ERROR: 'PIPELINE_STEP_ERROR',
  // Raised from `src/pipeline.ts`, not from this module, which is why they were
  // absent here. `ErrorCodes` is the sole input to `collectFrameworkCodes()` —
  // the exact-code half of the A17 collision guard — so a user module could
  // register them as its own. apcore-rust derives its set from the exhaustive
  // `ErrorCode` enum and rejects all eight; this map must be equally complete.
  PIPELINE_ABORT: 'PIPELINE_ABORT',
  PIPELINE_CONFIGURATION_ERROR: 'PIPELINE_CONFIGURATION_ERROR',
  PIPELINE_DEPENDENCY_ERROR: 'PIPELINE_DEPENDENCY_ERROR',
  STEP_NAME_DUPLICATE: 'STEP_NAME_DUPLICATE',
  STEP_NOT_FOUND: 'STEP_NOT_FOUND',
  STEP_NOT_REMOVABLE: 'STEP_NOT_REMOVABLE',
  STEP_NOT_REPLACEABLE: 'STEP_NOT_REPLACEABLE',
  STRATEGY_NOT_FOUND: 'STRATEGY_NOT_FOUND',
  PIPELINE_STEP_NOT_FOUND: 'PIPELINE_STEP_NOT_FOUND',
  // ERR-002: the same failure mode as the PIPELINE_* block above, on three
  // more codes the framework throws from outside this module.
  //
  // `TraceContext.inject` (src/trace-context.ts) stamps INVALID_PARENT_ID on a
  // malformed `parentId`, and no reserved prefix covers `INVALID_`, so
  // `registry.register('executor.my_mod', ['INVALID_PARENT_ID'])` was ACCEPTED
  // here while apcore-python and apcore-rust raised ERROR_CODE_COLLISION —
  // letting a module's code shadow the framework's.
  //
  // The two SCHEMA_UNION_* codes (src/schema/validator.ts) were reachable by
  // the guard only through the `SCHEMA_` reserved PREFIX. That covers them in
  // practice, but the prefix is a policy about a namespace while this map is
  // the inventory of codes the framework actually throws, and the exact-code
  // half must be able to stand on its own.
  INVALID_PARENT_ID: 'INVALID_PARENT_ID',
  SCHEMA_UNION_NO_MATCH: 'SCHEMA_UNION_NO_MATCH',
  SCHEMA_UNION_AMBIGUOUS: 'SCHEMA_UNION_AMBIGUOUS',
} as const);

export type ErrorCode = (typeof ErrorCodes)[keyof typeof ErrorCodes];
