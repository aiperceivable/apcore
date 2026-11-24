/**
 * Browser-safe subset of apcore-js.
 *
 * This entry exposes only the runtime symbols whose module-load and
 * runtime behaviour do not depend on any Node.js built-in (`node:fs`,
 * `node:path`, `node:os`, `node:crypto`, `node:process`). It is selected
 * automatically by bundlers via `package.json` `exports.browser`, so
 * consumer code keeps writing `import { ... } from 'apcore-js'` regardless
 * of target.
 *
 * The default Node entry (`src/index.ts`) re-exports a strict superset.
 * Adding any Node-only symbol here defeats the purpose of this file —
 * `tests/browser-entry.test.ts` walks the transitive import graph from
 * this file and fails CI if any `node:*` reference leaks in.
 *
 * Excluded from this entry (call them from the Node entry only):
 *   - Config / discoverConfigFile / DEFAULTS-aware Config.load (filesystem)
 *   - BindingLoader (loads YAML from disk)
 *   - SchemaLoader / RefResolver (load schemas from disk)
 *   - Registry directory scanning (`Registry.discover` is exposed but
 *     dynamically loads `node:fs` at call time and will throw cleanly in
 *     a browser; in-memory `register/get/has/list` are fully usable)
 *   - sys-modules `registerSysModules` overridesPath file loader
 *
 * If you reach for one of those in browser code, expose the equivalent
 * data programmatically (e.g. `new Registry()` + `register(...)`,
 * `new ACL([...])`, `jsonSchemaToTypeBox(yourSchema)`) — that is the
 * supported path for in-browser apcore-js use.
 */

// ---- Client / runtime core -----------------------------------------------
export { APCore } from '../client.js';
export type { APCoreOptions, ModuleOptions } from '../client.js';

export { Executor, REDACTED_VALUE } from '../executor.js';

export {
  Registry,
  REGISTRY_EVENTS,
  MODULE_ID_PATTERN,
  MAX_MODULE_ID_LENGTH,
  RESERVED_WORDS,
  EPHEMERAL_NAMESPACE_PREFIX,
  isEphemeralModuleId,
} from '../registry/registry.js';
export type { Discoverer, ModuleValidator } from '../registry/registry.js';

// ---- Context / identity / cancel -----------------------------------------
export { CancelToken, ExecutionCancelledError } from '../cancel.js';
export { Context, createIdentity } from '../context.js';
export { ContextKey } from '../context-key.js';
export {
  TRACING_SPANS,
  TRACING_SAMPLED,
  METRICS_STARTS,
  LOGGING_START,
  REDACTED_OUTPUT,
  RETRY_COUNT_BASE,
} from '../context-keys.js';
export type { Identity, ContextFactory } from '../context.js';
export { TraceContext } from '../trace-context.js';
export type { TraceParent } from '../trace-context.js';

// ---- Module types & annotations ------------------------------------------
export {
  DEFAULT_ANNOTATIONS,
  createAnnotations,
  annotationsToJSON,
  annotationsFromJSON,
  createPreflightResult,
  TChange,
  TPreviewResult,
} from '../module.js';
export type {
  ModuleAnnotations,
  ModuleExample,
  ModuleDescription,
  ValidationResult,
  PreflightCheckResult,
  PreflightResult,
  Module,
  Change,
  PreviewResult,
} from '../module.js';

// ---- Decorator / FunctionModule ------------------------------------------
export { module, FunctionModule } from '../decorator.js';

// ---- Approval -------------------------------------------------------------
export {
  createApprovalRequest,
  createApprovalResult,
  AlwaysDenyHandler,
  AutoApproveHandler,
  CallbackApprovalHandler,
} from '../approval.js';
export type { ApprovalRequest, ApprovalResult, ApprovalHandler } from '../approval.js';

// ---- ACL (in-memory; ACL.load is Node-only and throws in browser) --------
export { ACL } from '../acl.js';
export type {
  AccessCheckOptions,
  AccessDecision,
  ACLApproval,
  ACLRule,
  AuditEntry,
  AuditLogger,
  ConditionOutcome,
  GovernanceProjection,
  RuleValidationFinding,
} from '../acl.js';

// ---- Errors --------------------------------------------------------------
export {
  ModuleError,
  ConfigNotFoundError,
  ConfigError,
  ACLRuleError,
  ACLDeniedError,
  ApprovalError,
  ApprovalDeniedError,
  ApprovalTimeoutError,
  ApprovalPendingError,
  ModuleNotFoundError,
  ModuleDisabledError,
  ModuleTimeoutError,
  SchemaValidationError,
  SchemaNotFoundError,
  SchemaParseError,
  SchemaCircularRefError,
  SchemaMaxDepthExceededError,
  CallDepthExceededError,
  CircularCallError,
  CallFrequencyExceededError,
  InvalidInputError,
  BindingInvalidTargetError,
  BindingModuleNotFoundError,
  BindingCallableNotFoundError,
  BindingNotCallableError,
  BindingSchemaMissingError,
  BindingSchemaInferenceFailedError,
  BindingSchemaModeConflictError,
  BindingStrictSchemaIncompatibleError,
  BindingFileInvalidError,
  FuncMissingTypeHintError,
  FuncMissingReturnTypeError,
  CircularDependencyError,
  DependencyNotFoundError,
  DependencyVersionMismatchError,
  ModuleLoadError,
  ReloadFailedError,
  ModuleExecuteError,
  InternalError,
  ConfigNamespaceDuplicateError,
  ConfigNamespaceReservedError,
  ConfigEnvPrefixConflictError,
  ConfigEnvMapConflictError,
  ConfigMountError,
  ConfigBindError,
  ErrorFormatterDuplicateError,
  TaskLimitExceededError,
  VersionConstraintError,
  ModuleIdConflictError,
  InvalidSegmentError,
  IdTooLongError,
  CircuitBreakerOpenError,
  ModuleReloadConflictError,
  SysModuleRegistrationError,
  SysModulesDisabledError,
  InvalidParentIdError,
  ContextBindingError,
  ErrorCodes,
} from '../errors.js';
export type { ErrorCode, ErrorOptions } from '../errors.js';

// ---- Error formatter / propagation ---------------------------------------
export { ErrorFormatterRegistry } from '../error-formatter.js';
export type { ErrorFormatter } from '../error-formatter.js';
export { propagateError } from '../utils/error-propagation.js';

// ---- Schema (pure helpers only) ------------------------------------------
export {
  jsonSchemaToTypeBox,
  contentHashAsync,
} from '../schema/loader-pure.js';
export { SchemaValidator } from '../schema/validator.js';
export { SchemaStrategy, ExportProfile } from '../schema/types.js';
export type {
  SchemaDefinition,
  ResolvedSchema,
  SchemaValidationErrorDetail,
  SchemaValidationResult,
} from '../schema/types.js';
export { toStrictSchema } from '../schema/strict.js';
export {
  assertOpenAiStrictCompatible,
  detectOpenAiStrictIncompatibilities,
} from '../schema/openai-strict.js';
export type { AssertOpenAiStrictOptions } from '../schema/openai-strict.js';

// ---- Utility helpers (pure) ----------------------------------------------
export { matchPattern, calculateSpecificity } from '../utils/pattern.js';
export { canonicalizeName, normalizeToCanonicalId } from '../utils/normalize.js';
export type { CanonicalNameError, CanonicalNameResult } from '../utils/normalize.js';
export {
  guardCallChain,
  DEFAULT_MAX_CALL_DEPTH,
  DEFAULT_MAX_MODULE_REPEAT,
} from '../utils/call-chain.js';

// ---- Registry helpers (pure) ---------------------------------------------
export { detectIdConflicts } from '../registry/conflicts.js';
export type { ConflictResult, ConflictType, ConflictSeverity } from '../registry/conflicts.js';
export type { ModuleDescriptor, DiscoveredModule, DependencyInfo } from '../registry/types.js';
export { classNameToSegment, discoverMultiClass } from '../registry/multi-class.js';
export type { ClassDescriptor, MultiClassEntry } from '../registry/multi-class.js';

// ---- Error-code registry / version negotiation (pure) --------------------
export {
  ErrorCodeRegistry,
  ErrorCodeCollisionError,
  FRAMEWORK_ERROR_CODE_PREFIXES,
} from '../error-code-registry.js';
export { negotiateVersion, VersionIncompatibleError } from '../version.js';

// ---- Middleware (in-memory primitives) -----------------------------------
export {
  Middleware,
  RetrySignal,
  MiddlewareManager,
  MiddlewareChainError,
  BeforeMiddleware,
  AfterMiddleware,
  LoggingMiddleware,
  RetryMiddleware,
  RetryHintMiddleware,
  ErrorHistoryMiddleware,
  PlatformNotifyMiddleware,
  CircuitBreakerMiddleware,
  CircuitBreakerState,
  CTX_CIRCUIT_STATE,
  validateContextKey,
  isAsyncHandler,
} from '../middleware/index.js';
export type {
  RetryConfig,
  CircuitBreakerOptions,
  ContextKeyWriter,
  ContextKeyValidation,
} from '../middleware/index.js';

// ---- Pipeline ------------------------------------------------------------
export {
  ExecutionStrategy,
  PipelineEngine,
  PipelineAbortError,
  StepNotFoundError,
  StepNotRemovableError,
  StepNotReplaceableError,
  StepNameDuplicateError,
  StrategyNotFoundError,
  PipelineStepError,
  PipelineStepNotFoundError,
  PipelineDependencyError,
} from '../pipeline.js';
export type {
  Step,
  StepResult,
  PipelineContext,
  PipelineState,
  StepTrace,
  PipelineTrace,
  StrategyInfo,
  StepMiddleware,
} from '../pipeline.js';

// ---- Built-in pipeline steps ---------------------------------------------
export {
  BuiltinContextCreation,
  BuiltinCallChainGuard,
  BuiltinModuleLookup,
  BuiltinACLCheck,
  BuiltinApprovalGate,
  BuiltinInputValidation,
  BuiltinMiddlewareBefore,
  BuiltinExecute,
  BuiltinOutputValidation,
  BuiltinMiddlewareAfter,
  BuiltinReturnResult,
  buildStandardStrategy,
  buildInternalStrategy,
  buildTestingStrategy,
  buildPerformanceStrategy,
  buildMinimalStrategy,
} from '../builtin-steps.js';
export type { StandardStrategyDeps } from '../builtin-steps.js';

// ---- Pipeline configuration ----------------------------------------------
export {
  registerStepType,
  unregisterStepType,
  registeredStepTypes,
  buildStrategyFromConfig,
  ConfigurationError,
} from '../pipeline-config.js';

// ---- VERSION -------------------------------------------------------------
export { VERSION } from '../generated/version.js';
