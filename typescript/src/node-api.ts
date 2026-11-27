// Client
export { APCore } from './client.js';
export type { APCoreOptions, APCorePlugin, ModuleOptions } from './client.js';
export type { DrainAbortedContext, DrainAbortReason, DrainContext, LifecycleError, LifecycleEventFlush, LifecycleModuleReport, LifecycleOperation, LifecycleReport, LifecycleStatus, ShutdownOptions, UnregisterOptions, WatchOptions } from './lifecycle.js';

// Core
export { CancelToken, ExecutionCancelledError } from './cancel.js';
export { Context, createIdentity } from './context.js';
export { ContextKey } from './context-key.js';
export {
  TRACING_SPANS,
  TRACING_SAMPLED,
  METRICS_STARTS,
  LOGGING_START,
  REDACTED_OUTPUT,
  RETRY_COUNT_BASE,
} from './context-keys.js';
export type { Identity, ContextFactory } from './context.js';
export {
  Registry,
  REGISTRY_EVENTS,
  MODULE_ID_PATTERN,
  MAX_MODULE_ID_LENGTH,
  RESERVED_WORDS,
  EPHEMERAL_NAMESPACE_PREFIX,
  isEphemeralModuleId,
} from './registry/registry.js';
export type { Discoverer, ModuleValidator, RegistryOptions } from './registry/registry.js';
export {
  Executor,
  redactSensitive,
  REDACTED_VALUE,
  CTX_GLOBAL_DEADLINE,
  CTX_TRACING_SPANS,
} from './executor.js';
export type { StreamCompletion, StreamCompletionReason, StreamExecution, StreamStepOutcome } from './stream-completion.js';

// Module types
export {
  DEFAULT_ANNOTATIONS,
  createAnnotations,
  annotationsToJSON,
  annotationsFromJSON,
  createPreflightResult,
  TChange,
  TPreviewResult,
} from './module.js';
export type {
  ModuleAnnotations,
  ModuleExample,
  ModuleDescription,
  ResourceLimits,
  ValidationResult,
  PreflightCheckResult,
  PreflightResult,
  Module,
  Change,
  PreviewResult,
} from './module.js';

// Config
export { Config, RESERVED_NAMESPACES, discoverConfigFile } from './config.js';

// Error Formatter
export { ErrorFormatterRegistry } from './error-formatter.js';
export type { ErrorFormatter } from './error-formatter.js';

// Approval
export {
  createApprovalRequest,
  createApprovalResult,
  AlwaysDenyHandler,
  AutoApproveHandler,
  CallbackApprovalHandler,
} from './approval.js';
export type { ApprovalRequest, ApprovalResult, ApprovalHandler } from './approval.js';

// Execution-time governance policy (apcore#76 RFC pilot)
export { ExecutionPolicy, PolicyRule } from './policy.js';
export type { PolicyCallSite, PolicyDecision, PolicyRuleOverrides } from './policy.js';
export type { PolicyEvaluationRequest, PolicyEvaluator, PolicyVerdict } from './policy-evaluation.js';

// Errors
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
  PolicyDeniedError,
  PolicyEvaluationError,
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
  TaskStoreError,
  VersionConstraintError,
  ModuleIdConflictError,
  InvalidSegmentError,
  IdTooLongError,
  CircuitBreakerOpenError,
  ModuleReloadConflictError,
  SysModuleRegistrationError,
  SysModulesDisabledError,
  InvalidParentIdError,
  StreamingInterfaceError,
  DuplicateModuleIdError,
  ContextBindingError,
  ErrorCodes,
} from './errors.js';
export type { ErrorCode, ErrorOptions } from './errors.js';

// Streaming
export { STREAMING_MARKER, isStreamingModule } from './streaming.js';
export type { StreamingModule } from './streaming.js';

// ACL
export { ACL } from './acl.js';
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
} from './acl.js';

// Middleware
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
} from './middleware/index.js';
export type {
  RetryConfig,
  CircuitBreakerOptions,
  ContextKeyWriter,
  ContextKeyValidation,
} from './middleware/index.js';

// Decorator
export { module, FunctionModule } from './decorator.js';

// Extensions
export { ExtensionManager } from './extensions.js';
export type { ExtensionPoint } from './extensions.js';

// Events
export {
  EventEmitter,
  createEvent,
  WebhookSubscriber,
  FileSubscriber,
  StdoutSubscriber,
  FilterSubscriber,
  CircuitBreakerWrapper,
  CircuitState,
} from './events/index.js';
export type {
  ApCoreEvent,
  EventSubscriber,
  CircuitBreakerConfig,
  RetryConfig as EventRetryConfig,
} from './events/index.js';

// System Modules
export {
  registerSysModules,
  createSubscriberFromConfig,
  ToggleState,
  DEFAULT_TOGGLE_STATE,
  isModuleDisabled,
  checkModuleDisabled,
  classifyHealthStatus,
  UpdateConfigModule,
  ReloadModule,
  HealthModule,
  HealthSummaryModule,
  ManifestModule,
  ManifestFullModule,
  UsageModule,
  UsageSummaryModule,
  ToggleFeatureModule,
  InMemoryAuditStore,
  FileOverridesStore,
  InMemoryOverridesStore,
} from './sys-modules/index.js';
export type { SysModulesContext, OverridesStore } from './sys-modules/index.js';

// Async tasks
export {
  AsyncTaskManager,
  TaskStatus,
  InMemoryTaskStore,
  RetryConfig as AsyncRetryConfig,
} from './async-task.js';
export type { TaskInfo, TaskStore, ReaperHandle } from './async-task.js';

// Bindings
export {
  BindingLoader,
  type BindingConfigLike,
  type BindingLoaderOptions,
} from './bindings.js';

// Utils
export { matchPattern, calculateSpecificity } from './utils/pattern.js';
export { canonicalizeName, normalizeToCanonicalId } from './utils/normalize.js';
export type { CanonicalNameError, CanonicalNameResult } from './utils/normalize.js';
export {
  guardCallChain,
  DEFAULT_MAX_CALL_DEPTH,
  DEFAULT_MAX_MODULE_REPEAT,
} from './utils/call-chain.js';
export { propagateError } from './utils/error-propagation.js';

// Multi-class discovery
export { classNameToSegment, discoverMultiClass } from './registry/multi-class.js';
export type { ClassDescriptor, MultiClassEntry } from './registry/multi-class.js';

// Error Code Registry
export {
  ErrorCodeRegistry,
  ErrorCodeCollisionError,
  FRAMEWORK_ERROR_CODE_PREFIXES,
} from './error-code-registry.js';

// Version
export { negotiateVersion, VersionIncompatibleError } from './version.js';

// Schema
export {
  SchemaLoader,
  jsonSchemaToTypeBox,
  contentHash,
  contentHashAsync,
} from './schema/loader.js';
export { SchemaValidator } from './schema/validator.js';
export { SchemaExporter } from './schema/exporter.js';
export { SchemaStrategy, ExportProfile } from './schema/types.js';
export type {
  SchemaDefinition,
  ResolvedSchema,
  SchemaValidationErrorDetail,
  SchemaValidationResult,
} from './schema/types.js';
export { RefResolver } from './schema/ref-resolver.js';
export { toStrictSchema } from './schema/strict.js';
export {
  assertOpenAiStrictCompatible,
  detectOpenAiStrictIncompatibilities,
} from './schema/openai-strict.js';
export type { AssertOpenAiStrictOptions } from './schema/openai-strict.js';

// Registry types
export type { ModuleDescriptor, DiscoveredModule, DependencyInfo } from './registry/types.js';

// Registry conflicts
export { detectIdConflicts } from './registry/conflicts.js';
export type { ConflictResult, ConflictType, ConflictSeverity } from './registry/conflicts.js';

// Observability
export {
  TracingMiddleware,
  StdoutExporter,
  InMemoryExporter,
  OTLPExporter,
  createSpan,
} from './observability/tracing.js';
export type { Span, SpanExporter } from './observability/tracing.js';
export { MetricsCollector, MetricsMiddleware } from './observability/metrics.js';
export {
  ContextLogger,
  ObsLoggingMiddleware,
  RedactionConfig,
  DEFAULT_REDACTION_FIELD_PATTERNS,
} from './observability/context-logger.js';
export {
  ErrorHistory,
  computeFingerprint,
  normalizeMessage,
} from './observability/error-history.js';
export type { ErrorEntry } from './observability/error-history.js';
export { UsageCollector, UsageMiddleware } from './observability/usage.js';
export {
  NoopUsageExporter,
  PeriodicUsageExporter,
} from './observability/usage-exporter.js';
export type { UsageExporter } from './observability/usage-exporter.js';
export {
  computeModuleErrorRate,
  estimateP99FromHistogram,
  matchesModuleId,
  METRIC_CALLS_TOTAL,
  METRIC_DURATION_SECONDS,
} from './observability/metrics-utils.js';
export type {
  UsageRecord,
  CallerUsageSummary,
  HourlyBucket,
  ModuleUsageSummary,
  ModuleUsageDetail,
  UsageCollectorOptions,
} from './observability/usage.js';
export { InMemoryStorageBackend } from './observability/storage.js';
export type { StorageBackend } from './observability/storage.js';
export { BatchSpanProcessor, SimpleSpanProcessor } from './observability/batch-span-processor.js';
export type {
  SpanProcessor,
  BatchSpanProcessorOptions,
} from './observability/batch-span-processor.js';
export { renderPrometheusMetrics } from './observability/prometheus.js';

// Trace Context
export { TraceContext } from './trace-context.js';
export type { TraceParent } from './trace-context.js';

// Pipeline
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
} from './pipeline.js';
export type {
  Step,
  StepResult,
  PipelineContext,
  PipelineState,
  StepTrace,
  PipelineTrace,
  GovernanceState,
  StrategyInfo,
  StepMiddleware,
} from './pipeline.js';

// Built-in Steps
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
} from './builtin-steps.js';
export type { StandardStrategyDeps } from './builtin-steps.js';

// Pipeline Configuration
export {
  registerStepType,
  unregisterStepType,
  registeredStepTypes,
  buildStrategyFromConfig,
  ConfigurationError,
} from './pipeline-config.js';

export { VERSION } from './generated/version.js';
