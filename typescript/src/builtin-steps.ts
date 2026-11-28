/**
 * Built-in pipeline steps extracted from the executor's hardcoded logic.
 *
 * Each class implements the Step interface and wraps one phase of the
 * execution pipeline.  Dependencies are injected via the constructor so
 * that each step is independently testable.
 */

import type { TSchema } from '@sinclair/typebox';
import { Kind } from '@sinclair/typebox';
import type { ACL, AccessDecision } from './acl.js';
import { buildGovernanceProjection } from './acl-handlers.js';
import { governanceUnion } from './schema/annotations.js';
import { RedactionConfig } from './observability/context-logger.js';
import type { ApprovalHandler, ApprovalRequest, ApprovalResult } from './approval.js';
import { createApprovalRequest, createApprovalResult, takeApprovalToken } from './approval.js';
import type { Config } from './config.js';
import { getDefault } from './config-defaults.js';
import { CancelToken, awaitExecution } from './cancel.js';
import { Context } from './context.js';
import { assertDeclarationInvocation, recordDeclarationDispatch } from './declaration-invocation.js';
import { authorityKeyOf, hasScopeCeiling } from './identity.js';
import { assertScopedInvocation, isStockScopeAcl, recordScopedGrant, recordScopedLookup } from './scoped-invocation.js';
import {
  ACLDeniedError,
  ApprovalDeniedError,
  ApprovalPendingError,
  ApprovalTimeoutError,
  InvalidInputError,
  ModuleDisabledError,
  ModuleExecuteError,
  ModuleNotFoundError,
  ModuleTimeoutError,
  PolicyDeniedError,
  PolicyEvaluationError,
  SchemaValidationError,
} from './errors.js';
import { DEFAULT_TOGGLE_STATE, type ToggleState } from './sys-modules/toggle.js';
import { redactSensitive, sensitivePolicyFields } from './executor.js';
import { auditSpan } from './execution-lifetime.js';
import { type EventEmitter, createEvent } from './events/emitter.js';
import { applyDecisionToAnnotations, DEFAULT_EXECUTION_POLICY } from './policy.js';
import type { ExecutionPolicy, PolicyDecision } from './policy.js';
import {
  assertPolicyBinding, copyPolicyInputs, evaluatePolicy, policyApprovalContext, policyArgumentsDigest, policyInvocationOf,
  type PolicyEvaluator,
} from './policy-evaluation.js';
import { MiddlewareChainError, type MiddlewareManager } from './middleware/manager.js';
import type { Module, ModuleAnnotations } from './module.js';
import { DEFAULT_ANNOTATIONS, isModuleIdempotent } from './module.js';
import type { GovernanceGateMarker, PipelineContext, Step, StepResult } from './pipeline.js';
import { ConfigurationError, ExecutionStrategy, GOVERNANCE_GATE } from './pipeline.js';
import { isStreamingModule } from './streaming.js';
import type { Registry } from './registry/registry.js';
import { jsonSchemaToTypeBox } from './schema/loader-pure.js';
import { declarationJsonObject, declarationOf, takeDeclarationInputs, validateDeclarationInputs } from './schema/standard.js';
import { SchemaValidator } from './schema/validator.js';
import { guardCallChain } from './utils/call-chain.js';
import { normalizeModuleObject } from './utils/module-output.js';

// ---------------------------------------------------------------------------
// Helpers (shared with executor, kept minimal)
// ---------------------------------------------------------------------------

function resolveSchema(mod: Record<string, unknown>, key: string): TSchema | null {
  const declaration = declarationOf(mod);
  if (declaration) return key === 'inputSchema' ? declaration.inputSchema : declaration.outputSchema;
  const schema = mod[key] as TSchema | undefined;
  if (schema == null) return null;
  if (Kind in schema) return schema;
  const converted = jsonSchemaToTypeBox(schema as unknown as Record<string, unknown>);
  mod[key] = converted;
  return converted;
}

// `false`: the module-invocation boundary performs NO type coercion, under any
// host configuration (TYPE_MAPPING §17.3). A contract that declares `integer`
// receives an integer — `"42"` is a type error, not an integer spelled
// differently — because a module's contract has to mean the same thing
// regardless of which host loaded it.
const _schemaValidator = new SchemaValidator(false);

function validateSchema(schema: TSchema, data: Record<string, unknown>, direction: string): void {
  const result = _schemaValidator.validate(data, schema);
  if (result.valid) return;
  throw new SchemaValidationError(`${direction} validation failed`, result.errors as unknown as Array<Record<string, unknown>>);
}

// ---------------------------------------------------------------------------
// 1. BuiltinContextCreation
// ---------------------------------------------------------------------------

/** Creates or inherits execution Context and sets the global deadline. */
export class BuiltinContextCreation implements Step {
  readonly name = 'context_creation';
  readonly description = 'Create or inherit execution context and set global deadline';
  readonly removable = false;
  readonly replaceable = false;
  readonly pure = true;
  readonly provides = ['context'] as const;

  private _globalTimeout: number;

  constructor(config: Config | null) {
    if (config !== null) {
      this._globalTimeout =
        (config.get('executor.global_timeout') as number) ??
        (getDefault('executor.global_timeout') as number);
    } else {
      this._globalTimeout = getDefault('executor.global_timeout') as number;
    }
  }

  async execute(ctx: PipelineContext): Promise<StepResult> {
    // Always wrap in a child context for this module call.
    // If no parent context was provided, create a fresh root first.
    const invocation = policyInvocationOf(ctx);
    if (invocation) assertPolicyBinding(ctx, invocation);
    let derived: Context;
    if (ctx.context == null) {
      // Issue #66: no executor arg; auto-binding happens on the next
      // executor entry. For internal child-context creation we just need a
      // fresh root.
      derived = Context.create().child(ctx.moduleId, new CancelToken());
    } else {
      derived = ctx.context.child(ctx.moduleId, new CancelToken(ctx.context.cancelToken));
    }

    // `global_deadline` — spec v1.50.0 D-99 / D-100 / D-101,
    // core-executor.md "`global_deadline` Representation and Lifetime".
    //
    // D-100: the first-class `Context.globalDeadline` field is the storage.
    // This step used to write `context.data[CTX_GLOBAL_DEADLINE]` and every
    // reader used to read it back from there, so the field — a documented
    // `Context.create` parameter — was read nowhere in the pipeline and a
    // caller-supplied deadline was silently replaced by the config default.
    //
    // D-99: the clock is epoch SECONDS, as docs/features/context-object.md
    // has said since the field was introduced, so a caller writing
    // `Date.now() / 1000 + budget` is on the same basis the pipeline is. The
    // previous `Date.now() + timeout` made the public parameter unusable: a
    // spec-shaped value read as milliseconds has always already expired.
    //
    // D-101: computed onto the context derived for THIS call. `data` is
    // shared by reference through `child()`, so the old
    // `!(KEY in ctx.context.data)` guard saw call #1's key on call #2 — a
    // Context reused across successive top-level calls (explicitly blessed by
    // the spec) inherited the first call's remaining budget. The derived
    // context is fresh per call, and `child()` carries the field down, so a
    // NESTED call still inherits the enclosing budget exactly as before.
    if (derived.globalDeadline === null && this._globalTimeout > 0) {
      derived = derived._withGlobalDeadline(
        (Date.now() + this._globalTimeout) / 1000,
      );
    }

    ctx.context = derived;
    if (invocation) {
      invocation.context = derived;
      invocation.callerId = derived.callerId;
      invocation.identity = derived.identity;
      invocation.principalId = derived.identity?.id;
      invocation.principalType = derived.identity?.type;
      invocation.authorityKey = authorityKeyOf(derived.identity);
    }
    ctx.onContextReady?.(derived);
    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// 2. BuiltinCallChainGuard
// ---------------------------------------------------------------------------

/** Validates call chain depth, repeat limits, and cancel token. */
export class BuiltinCallChainGuard implements Step {
  readonly name = 'call_chain_guard';
  readonly description = 'Call chain guard: depth, repeat limits, cancel token';
  readonly removable = true;
  readonly replaceable = true;
  readonly pure = true;
  readonly requires = ['context'] as const;

  private _maxCallDepth: number;
  private _maxModuleRepeat: number;

  constructor(config: Config | null) {
    if (config !== null) {
      this._maxCallDepth =
        (config.get('executor.max_call_depth') as number) ??
        (getDefault('executor.max_call_depth') as number);
      this._maxModuleRepeat =
        (config.get('executor.max_module_repeat') as number) ??
        (getDefault('executor.max_module_repeat') as number);
    } else {
      this._maxCallDepth = getDefault('executor.max_call_depth') as number;
      this._maxModuleRepeat = getDefault('executor.max_module_repeat') as number;
    }
  }

  async execute(ctx: PipelineContext): Promise<StepResult> {
    // Check cancel token first — throw directly for error type preservation
    if (ctx.context.cancelToken !== null) {
      ctx.context.cancelToken.check();
    }

    // Call chain safety guard — throws CallDepthExceededError, CircularCallError, etc.
    guardCallChain(ctx.moduleId, ctx.context.callChain, this._maxCallDepth, this._maxModuleRepeat);

    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// 3. BuiltinModuleLookup
// ---------------------------------------------------------------------------

/** Resolves the module from the registry and sets ctx.module.
 *
 * Also checks the toggle state and raises ModuleDisabledError if the module
 * has been disabled — inlined here to match the 11-step pipeline spec and
 * apcore-python/apcore-rust implementations (sync finding A-D-011).
 */
const moduleExecutionLeases = new WeakMap<PipelineContext, () => () => void>();

export class BuiltinModuleLookup implements Step {
  readonly name = 'module_lookup';
  readonly description = 'Resolve module from registry, enforce toggle state';
  readonly removable = false;
  readonly replaceable = false;
  readonly pure = true;
  readonly provides = ['module', 'governanceProjection'] as const;

  private _registry: Registry;
  private readonly _toggleState: ToggleState;

  constructor(
    registry: Registry,
    toggleState?: ToggleState,
    private readonly _redaction: RedactionConfig | null = null,
  ) {
    this._registry = registry;
    this._toggleState = toggleState ?? DEFAULT_TOGGLE_STATE;
  }

  /** @internal Shared by module lookup and the system toggle module. */
  get toggleState(): ToggleState {
    return this._toggleState;
  }

  /** @internal Discovery must use the same registry as module lookup. */
  usesRegistry(registry: Registry): boolean {
    return this._registry === registry;
  }

  async execute(ctx: PipelineContext): Promise<StepResult> {
    const invocation = policyInvocationOf(ctx);
    if (invocation) assertPolicyBinding(ctx, invocation);
    if (this._registry.isClosed) throw new ModuleNotFoundError(ctx.moduleId);
    const mod = this._registry.get(ctx.moduleId, ctx.versionHint);
    if (mod === null || (!(invocation?.preflight ?? ctx.dryRun) && this._registry.isDraining(ctx.moduleId))) {
      throw new ModuleNotFoundError(ctx.moduleId);
    }
    assertDeclarationInvocation(ctx, mod);

    // Inline toggle check — matches apcore-python builtin_steps.py:231 and
    // apcore-rust builtin_steps.rs:219 (sync finding A-D-011).
    if (this._toggleState.isDisabled(ctx.moduleId)) {
      throw new ModuleDisabledError(ctx.moduleId);
    }

    ctx.module = mod;
    recordScopedLookup(ctx);
    const moduleId = ctx.moduleId;
    moduleExecutionLeases.set(ctx, this._registry.bindExecution(moduleId, mod, ctx.context.cancelToken));
    // PROTOCOL_SPEC §7.4 (D-96): resolve the registry's DECLARED annotations
    // here and hand them to the gate, which has no registry of its own — the
    // same shape as `aclApprovalRequired`, which Step 4 computes and Step 5
    // reads. Without this the gate could only ever see the live instance, and a
    // `*_meta.yaml` / `metadata` source declaring `requires_approval: true`
    // reached `getDefinition()` and the manifest while the module executed
    // UNGATED.
    ctx.declaredAnnotations = this._registry.getDeclaredAnnotations(ctx.moduleId);
    if (invocation) {
      const version = this._registry.getModuleMetadata(moduleId)['version'];
      invocation.selected = {
        module: mod,
        version: typeof version === 'string' ? version : null,
        governance: governanceUnion((mod as Record<string, unknown>)['annotations'], ctx.declaredAnnotations),
      };
    }

    // PROTOCOL_SPEC §6.1.8: the governance projection is computed HERE, during
    // module lookup, so it is available to the ACL check at Step 4. The
    // ordering is normative, not incidental.
    //
    // It is NOT `context.redactedInputs`, set a few lines below: that field's
    // contract is safe *logging*, it is a raw copy when the module declares no
    // input schema, and one field serving both "safe to log" and "input to a
    // security decision" will eventually break one of them in a change made
    // for the other. The projection carries the key set and each key's JSON
    // type, and structurally cannot carry a value.
    ctx.governanceProjection = buildGovernanceProjection(ctx.inputs);

    // Early input redaction: set context.redactedInputs BEFORE middleware
    // runs (step 6). This ensures logging middleware sees redacted data.
    if (ctx.context != null) {
      const { _approval_token: _, ...capturedInputs } = ctx.inputs;
      const inputSchema = resolveSchema(mod as Record<string, unknown>, 'inputSchema');
      ctx.context.redactedInputs = redactSensitive(
        capturedInputs,
        (inputSchema as Record<string, unknown> | null) ?? {},
        this._redaction,
      );
    }

    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// 6. BuiltinACLCheck
// ---------------------------------------------------------------------------

/** Enforces access control via the ACL provider. */
export class BuiltinACLCheck implements Step {
  readonly name = 'acl_check';
  readonly description = 'Access control list enforcement';
  readonly removable = true;
  readonly replaceable = true;
  // `governanceProjection` is declared as a requirement so §6.1.8 rule 1's
  // "computed at Step 3, available at Step 4" is enforced by the strategy's
  // dependency check rather than left to step ordering holding by habit.
  readonly requires = ['context', 'module', 'governanceProjection'] as const;
  readonly provides = ['policy_acl_checked'] as const;
  readonly pure = true;
  /** A governance gate: `configure` cannot weaken it (PROTOCOL_SPEC 5.16.1, D-130). */
  readonly [GOVERNANCE_GATE]: GovernanceGateMarker = { pureIsDefault: true };

  private _acl: ACL | null;
  private _eventEmitter: EventEmitter | null;

  constructor(acl: ACL | null, eventEmitter: EventEmitter | null = null) {
    this._acl = acl;
    this._eventEmitter = eventEmitter;
  }

  /** @internal Copy the gate, including any declared step configuration. */
  copy(): BuiltinACLCheck {
    return Object.create(Object.getPrototypeOf(this), Object.getOwnPropertyDescriptors(this)) as BuiltinACLCheck;
  }

  /** Update the ACL provider at runtime. */
  setAcl(acl: ACL): void {
    this._acl = acl;
  }

  /** Set the event emitter for ACL denials. */
  setEventEmitter(emitter: EventEmitter | null): void {
    this._eventEmitter = emitter;
  }

  /**
   * The ACL this gate enforces, or `null` when it enforces none.
   *
   * Read by `Executor.governanceState()`, which reports what the running gate
   * holds rather than what the executor was given (PROTOCOL_SPEC 6.6.5.5, D-129).
   */
  get acl(): ACL | null {
    return this._acl;
  }

  async execute(ctx: PipelineContext): Promise<StepResult> {
    const invocation = policyInvocationOf(ctx);
    if (invocation) assertPolicyBinding(ctx, invocation);
    assertScopedInvocation(ctx, false);
    if (hasScopeCeiling(ctx.context.identity) && !isStockScopeAcl(this._acl)) {
      throw new ACLDeniedError(ctx.context.callerId, ctx.moduleId);
    }
    if (this._acl === null) {
      if (invocation) invocation.aclApprovalRequired = false;
      return { action: 'continue' };
    }
    const callerId = ctx.context.callerId;
    const decision = await this._decide(ctx, callerId);
    recordScopedGrant(ctx, decision);
    // §6.9 rows 3–4 / §7.4: the ACL's approval requirement is carried to the
    // approval gate at Step 5, which fires on the union of it, the module's
    // annotation and `gateDestructive`. An implementation that reads only the
    // annotation silently ignores every rule carrying `approval` — the rule
    // loads, matches, and does nothing.
    ctx.aclApprovalRequired = decision.approvalRequired;
    ctx.aclAllowed = decision.access === 'allow';
    if (decision.access !== 'allow') {
      // Publish a governance event on denial (canonical name proposed in
      // apcore#77). Guarded by dryRun so a validate() preflight probe never
      // emits a spurious denial event. Fires only on deny — allows are
      // high-volume and already covered by the apcore.acl.check span.
      this._emitDenied(callerId, ctx.moduleId, ctx.context, ctx.dryRun === true);
      throw new ACLDeniedError(callerId, ctx.moduleId);
    }
    if (invocation) invocation.aclApprovalRequired = decision.approvalRequired;
    return { action: 'continue' };
  }

  /**
   * Obtain the {@link AccessDecision} for this call from whichever entry point
   * the attached provider offers.
   *
   * The structured accessors (§6.8.1) come first because they carry both axes
   * of §6.1.6; the boolean ones are the compatibility path for a duck-typed
   * provider, and a boolean can only mean "authorized", so such a provider
   * contributes no approval requirement.
   *
   * Within each pair the async entry point is preferred so conditions
   * registered via `ACL.registerAsyncCondition()` are actually awaited. The
   * sync evaluator fails a Promise-returning condition closed, which makes a
   * `deny` rule guarded by an async condition silently NOT match — letting a
   * later catch-all `allow` win (fail-open).
   *
   * The Executor deliberately does NOT use the boolean `check()` here: §6.8.1
   * makes that fail closed on an approval requirement, which would turn
   * "allowed but ask first" into a denial instead of a trip through the gate.
   */
  private async _decide(ctx: PipelineContext, callerId: string | null): Promise<AccessDecision> {
    const options = { arguments: ctx.governanceProjection ?? null };
    const aclLike = this._acl as unknown as {
      asyncCheckAccess?: unknown;
      checkAccess?: unknown;
      asyncCheck?: unknown;
    };
    if (typeof aclLike.asyncCheckAccess === 'function') {
      return await (this._acl as ACL).asyncCheckAccess(
        callerId, ctx.moduleId, ctx.context, options,
      );
    }
    if (typeof aclLike.checkAccess === 'function') {
      return (this._acl as ACL).checkAccess(callerId, ctx.moduleId, ctx.context, options);
    }
    const allowed =
      typeof aclLike.asyncCheck === 'function'
        ? await (this._acl as ACL).asyncCheck(callerId, ctx.moduleId, ctx.context)
        : (this._acl as ACL).check(callerId, ctx.moduleId, ctx.context);
    return {
      access: allowed ? 'allow' : 'deny',
      approvalRequired: false,
      matchedRuleIndex: null,
      reason: 'rule_match',
    };
  }

  /** Emit `apcore.acl.denied` on the event bus (apcore#77) when live. */
  private _emitDenied(
    callerId: string | null,
    moduleId: string,
    ctx: Context,
    dryRun: boolean,
  ): void {
    if (this._eventEmitter === null || dryRun) return;
    this._eventEmitter.emit(
      createEvent('apcore.acl.denied', moduleId, 'warn', {
        module_id: moduleId,
        caller_id: callerId,
        reason: 'ACL denied',
        trace_id: ctx.traceId,
      }),
    );
  }
}

// ---------------------------------------------------------------------------
// 7. BuiltinApprovalGate
// ---------------------------------------------------------------------------

/** Handles approval flow for modules that require explicit approval.
 *
 * For modules whose annotations declare `requiresApproval=true` (or that an
 * {@link ExecutionPolicy} gates), calls the configured ApprovalHandler, emits an
 * audit event (tracing span event + optional event-bus event), and translates
 * the result into either continued execution or the appropriate ApprovalError.
 *
 * The default policy requires approval for destructive modules and denies
 * approval-required calls that lack a handler. Explicit policy opt-outs
 * emit a warning once per affected module.
 */
export class BuiltinApprovalGate implements Step {
  readonly name = 'approval_gate';
  readonly description = 'Approval handler flow';
  readonly removable = true;
  readonly replaceable = true;
  readonly requires = ['context', 'module'] as const;
  readonly pure = false;
  /** A governance gate: `configure` cannot weaken it (PROTOCOL_SPEC 5.16.1, D-130). */
  readonly [GOVERNANCE_GATE]: GovernanceGateMarker = { pureIsDefault: false };

  private _handler: ApprovalHandler | null;
  private _policy: ExecutionPolicy;
  private _eventEmitter: EventEmitter | null;
  private readonly _warned = new Set<string>();

  constructor(
    handler: ApprovalHandler | null,
    policy: ExecutionPolicy | null = null,
    eventEmitter: EventEmitter | null = null,
  ) {
    this._handler = handler;
    this._policy = policy ?? DEFAULT_EXECUTION_POLICY;
    this._eventEmitter = eventEmitter;
  }

  /** @internal Keep step configuration and give the runtime its own warning state. */
  copy(): BuiltinApprovalGate {
    const properties: PropertyDescriptorMap = {
      ...Object.getOwnPropertyDescriptors(this),
      _warned: { value: new Set<string>() },
    };
    return Object.create(Object.getPrototypeOf(this), properties) as BuiltinApprovalGate;
  }

  /** Update the approval handler at runtime. */
  setApprovalHandler(handler: ApprovalHandler): void {
    this._handler = handler;
  }

  /** Update the execution policy at runtime. */
  setPolicy(policy: ExecutionPolicy | null): void {
    this._policy = policy ?? DEFAULT_EXECUTION_POLICY;
  }

  /** Set the event emitter for approval decisions and policy overrides. */
  setEventEmitter(emitter: EventEmitter | null): void {
    this._eventEmitter = emitter;
  }

  /** The approval handler this gate consults (PROTOCOL_SPEC 6.6.5.5, D-129). */
  get approvalHandler(): ApprovalHandler | null {
    return this._handler;
  }

  /** The execution policy this gate applies (PROTOCOL_SPEC 6.6.5.5, D-129). */
  get policy(): ExecutionPolicy {
    return this._policy;
  }

  async execute(ctx: PipelineContext): Promise<StepResult> {
    const mod = ctx.module as Record<string, unknown> | null;
    const invocation = policyInvocationOf(ctx);
    const approvalToken = invocation ? null : takeApprovalToken(ctx);
    if (invocation) assertPolicyBinding(ctx, invocation);

    // §6.9 row 4 — the ACL is a CALLER-scoped authorization layer and an
    // `ExecutionPolicy` is a MODULE-scoped platform override, so a policy may
    // ADD an approval requirement and MUST NOT remove one the ACL set. Letting
    // a module-scoped override cancel a caller-scoped decision is a privilege
    // escalation: a policy rule written for `orders.*` would silently strip a
    // requirement an ACL author attached to one untrusted caller. Union is the
    // only safe composition, so this term is OR-ed with the policy's resolution.
    const aclApprovalRequired = (invocation ? invocation.aclApprovalRequired : ctx.aclApprovalRequired) === true;

    // PROTOCOL_SPEC §7.4 (D-96): governance is the UNION of the live module
    // instance and the registry's declared annotations. Reading the instance
    // alone ignored what an operator declared in a `*_meta.yaml` / `metadata`
    // source — a `requires_approval: true` that reached `getDefinition()` and
    // the manifest and gated nothing. Reading the declared slot alone would be
    // worse: it carries YAML > code precedence, so a metadata `false` would
    // cancel a module that asks to be gated. Both single-source readings are
    // fail-OPEN.
    const governance = invocation?.selected ? invocation.selected.governance
      : governanceUnion(mod?.['annotations'], ctx.declaredAnnotations);

    // §7.9.6: policy resolution receives arguments before schema validation.
    // The executor isolates the credential before the pipeline starts.
    const decision = this._policy.resolve(ctx.moduleId, governance, {
      arguments: ctx.inputs ?? null,
      context: ctx.context ?? null,
    });
    if (decision.overridden) {
      this._emitPolicyAudit(decision, ctx.context);
    }

    const needsApproval = decision.needsApproval || aclApprovalRequired;
    if (invocation && !(needsApproval && this._handler === null && this._policy.strict)) {
      invocation.earlyApproval = {
        needed: needsApproval,
        annotations: applyDecisionToAnnotations(governance ?? DEFAULT_ANNOTATIONS, decision),
        handler: this._handler,
      };
      return { action: 'continue' };
    }

    if (!needsApproval) {
      if (approvalToken !== null && authorityKeyOf(ctx.context.identity) !== null) {
        throw new ApprovalDeniedError(createApprovalResult({ status: 'rejected' }), ctx.moduleId);
      }
      if (decision.destructive) {
        this._warnOnce(
          `destructive_ungated:${ctx.moduleId}`,
          `[apcore:approval] Module '${ctx.moduleId}' is annotated destructive=true but is not ` +
            'covered by the approval gate (requiresApproval is false and no policy gates ' +
            'destructive modules). Consider requiresApproval=true or ' +
            'ExecutionPolicy(gateDestructive=true). (apcore#76)',
        );
      }
      return { action: 'continue' };
    }

    if (this._handler === null) {
      if (this._policy.strict) {
        const result = createApprovalResult({
          status: 'rejected',
          reason:
            'Approval required but no ApprovalHandler is configured; ' +
            'ExecutionPolicy(strict=true) fails closed',
        });
        this._emitAudit(result, ctx.moduleId, ctx.context);
        throw new ApprovalDeniedError(result, ctx.moduleId);
      }
      this._warnOnce(
        `no_handler:${ctx.moduleId}`,
        `[apcore:approval] Module '${ctx.moduleId}' requires approval but no ApprovalHandler ` +
          'is configured; explicit ExecutionPolicy(strict=false) lets the call continue ' +
          'per PROTOCOL_SPEC §7.4.',
      );
      return { action: 'continue' };
    }

    const annotations = applyDecisionToAnnotations(governance ?? DEFAULT_ANNOTATIONS, decision);
    const request = this._request(ctx, annotations, structuredClone(ctx.inputs));
    await this._authorize(this._handler, approvalToken, request);
    return { action: 'continue' };
  }

  /** Authorize the detached and validated scope, using the handler selected in Step 5. */
  async authorizeFinal(ctx: PipelineContext, needsApproval: boolean): Promise<void> {
    const invocation = policyInvocationOf(ctx);
    const early = invocation?.earlyApproval;
    const snapshot = invocation?.validated;
    if (!invocation || !early || !snapshot) throw new PolicyEvaluationError(ctx.moduleId);
    const token = takeApprovalToken(ctx);
    if (!needsApproval && token === null) return;
    if (early.handler === null) {
      throw new ApprovalDeniedError(createApprovalResult({
        status: 'rejected', reason: 'Final approval requires an ApprovalHandler',
      }), ctx.moduleId);
    }
    const digest = await policyArgumentsDigest(ctx.moduleId, snapshot);
    assertPolicyBinding(ctx, invocation);
    ctx.context.cancelToken?.check();
    const request = this._request(ctx, early.annotations, structuredClone(snapshot.arguments), {
      moduleVersion: invocation.selected?.version ?? null,
      canonicalArgsDigest: digest,
    });
    await this._authorize(early.handler, token, request, ctx.context);
    assertPolicyBinding(ctx, invocation);
  }

  private _request(
    ctx: PipelineContext,
    annotations: ModuleAnnotations,
    args: Record<string, unknown>,
    scope?: { moduleVersion: string | null; canonicalArgsDigest: string },
  ): ApprovalRequest {
    const mod = ctx.module as Record<string, unknown> | null;
    const authorityKey = authorityKeyOf(ctx.context.identity);
    return createApprovalRequest({
      moduleId: ctx.moduleId,
      callerId: ctx.context.callerId,
      action: ctx.moduleId,
      arguments: args,
      context: scope === undefined && authorityKey === null ? ctx.context : policyApprovalContext(ctx.context),
      annotations: annotations.requiresApproval ? annotations : Object.freeze({ ...annotations, requiresApproval: true }),
      description: (mod?.['description'] as string) ?? null,
      tags: (mod?.['tags'] as string[]) ?? [],
      ...scope,
      ...(authorityKey === null ? {} : { authorityKey }),
    });
  }

  private async _authorize(
    handler: ApprovalHandler, token: string | null, request: ApprovalRequest, auditContext = request.context,
  ): Promise<void> {
    const final = request.canonicalArgsDigest !== undefined;
    const authorityScoped = request.authorityKey !== undefined;
    let result: ApprovalResult;
    try {
      const supportsScope = (!final || handler.supportsFinalScope === true) &&
        (!authorityScoped || handler.supportsAuthorityScope === true);
      result = await this._requestApproval(handler, token, request, supportsScope);
      if (final || authorityScoped) result = this._safeFinalResult(result, supportsScope);
    } catch (error) {
      if (!final && !authorityScoped) throw error;
      result = createApprovalResult({ status: 'rejected' });
    }
    if (final || authorityScoped) auditContext.cancelToken?.check();
    this._emitAudit(result, request.moduleId, auditContext, token);

    if (result.status === 'approved') {
      return;
    }

    if (result.status === 'timeout') {
      throw new ApprovalTimeoutError(result, request.moduleId);
    }

    if (result.status === 'pending') {
      throw new ApprovalPendingError(result, request.moduleId);
    }

    // rejected or unknown
    throw new ApprovalDeniedError(result, request.moduleId);
  }

  private async _requestApproval(
    handler: ApprovalHandler, token: string | null, request: ApprovalRequest, resumeAllowed: boolean,
  ): Promise<ApprovalResult> {
    if (token === null) return await handler.requestApproval(request);
    if (resumeAllowed) {
      const result = await handler.checkApprovalForCall?.(token, request);
      if (result) return result;
    }
    return createApprovalResult({ status: 'rejected', reason: 'The approval handler does not support scoped resumption' });
  }

  private _safeFinalResult(result: ApprovalResult, supportsFinalScope: boolean): ApprovalResult {
    const status = result?.status;
    const id = result?.approvalId;
    const auditId = result?.auditId;
    const pending = status === 'pending' && supportsFinalScope && typeof id === 'string' && id.length > 0;
    return createApprovalResult({
      status: pending ? 'pending' : status === 'approved' || status === 'timeout' ? status : 'rejected',
      approvalId: pending ? id : null,
      auditId: typeof auditId === 'string' && auditId !== id ? auditId : null,
    });
  }

  /** Warn once per dedup key (module/kind pair). */
  private _warnOnce(key: string, message: string): void {
    if (this._warned.has(key)) return;
    this._warned.add(key);
    console.warn(message);
  }

  /** Audit a policy-driven override: span event + optional bus event (apcore#77). */
  private _emitPolicyAudit(decision: PolicyDecision, ctx: Context): void {
    const rule = decision.rule;
    const pattern = rule ? rule.pattern : '';
    const reason = (rule ? rule.reason : null) ?? '';
    const span = auditSpan(ctx);
    if (span) {
      span.events.push({
        name: 'policy_override',
        module_id: decision.moduleId,
        pattern,
        requires_approval: decision.requiresApproval,
        destructive: decision.destructive,
        needs_approval: decision.needsApproval,
        reason,
      });
    }
    if (this._eventEmitter !== null) {
      this._eventEmitter.emit(
        createEvent('apcore.policy.override', decision.moduleId, 'info', {
          module_id: decision.moduleId,
          pattern,
          requires_approval: decision.requiresApproval,
          destructive: decision.destructive,
          needs_approval: decision.needsApproval,
          reason,
          trace_id: ctx.traceId,
        }),
      );
    }
  }

  /** Audit an approval decision: span event + optional bus event (apcore#77).
   *
   * Severity mirrors the outcome: approved/pending are `info`; rejected/timeout
   * (governance interventions) are `warn`.
   */
  private _emitAudit(
    result: ApprovalResult,
    moduleId: string,
    ctx: Context,
    resumeCredential?: string | null,
  ): void {
    const auditId = result.auditId !== result.approvalId && result.auditId !== resumeCredential
      ? result.auditId ?? null
      : null;
    const span = auditSpan(ctx);
    if (span) {
      span.events.push({
        name: 'approval_decision',
        module_id: moduleId,
        status: result.status,
        approved_by: result.approvedBy ?? '',
        reason: result.reason ?? '',
        approval_id: auditId ?? '',
      });
    }
    if (this._eventEmitter !== null) {
      const severity =
        result.status === 'approved' || result.status === 'pending' ? 'info' : 'warn';
      this._eventEmitter.emit(
        createEvent('apcore.approval.decision', moduleId, severity, {
          module_id: moduleId,
          status: result.status,
          approved_by: result.approvedBy,
          reason: result.reason,
          approval_id: auditId,
          trace_id: ctx.traceId,
        }),
      );
    }
  }
}

// ---------------------------------------------------------------------------
// 8. BuiltinInputValidation
// ---------------------------------------------------------------------------

/** Validates inputs against module schema and redacts sensitive fields. */
export class BuiltinInputValidation implements Step {
  /** The resolved `obs.redaction.*` rules (§10.6.1 "Where the rules apply"). */
  constructor(private readonly _redaction: RedactionConfig | null = null) {}

  readonly name = 'input_validation';
  readonly description = 'Schema validation and redaction for inputs';
  readonly removable = true;
  readonly replaceable = true;
  readonly pure = true;
  readonly requires = ['module'] as const;
  readonly provides = ['validated_inputs'] as const;

  async execute(ctx: PipelineContext): Promise<StepResult> {
    assertDeclarationInvocation(ctx);
    const invocation = policyInvocationOf(ctx);
    if (invocation) assertPolicyBinding(ctx, invocation);
    let snapshot = invocation ? copyPolicyInputs(ctx.moduleId, ctx.inputs) : null;
    const mod = ctx.module as Record<string, unknown>;
    const declaration = declarationOf(mod);
    const inputSchema = resolveSchema(mod, 'inputSchema');
    let effectiveInputs = snapshot?.arguments ?? ctx.inputs;
    if (declaration && !snapshot) effectiveInputs = declarationJsonObject(effectiveInputs, 'input');
    if (inputSchema !== null) {
      validateSchema(inputSchema, effectiveInputs, 'Input');
      // Native TypeBox validators can change the inputs and retain aliases.
      // Capture both the values and their encoding from the detached result.
      if (snapshot && !declaration) snapshot = copyPolicyInputs(ctx.moduleId, effectiveInputs);
    }
    if (declaration) {
      effectiveInputs = await validateDeclarationInputs(ctx, declaration, effectiveInputs);
      if (snapshot) snapshot = copyPolicyInputs(ctx.moduleId, effectiveInputs);
    }

    const effectiveSchema = inputSchema ?? {};
    effectiveInputs = snapshot?.arguments ?? effectiveInputs;
    ctx.context.redactedInputs = redactSensitive(effectiveInputs, effectiveSchema, this._redaction);
    ctx.validatedInputs = snapshot || declaration ? structuredClone(effectiveInputs) : effectiveInputs;
    if (invocation && snapshot) {
      snapshot.sensitiveFields = sensitivePolicyFields(effectiveInputs, effectiveSchema);
      invocation.validated = snapshot;
    }
    return { action: 'continue' };
  }
}

/** Evaluate and approve the inputs owned by the invocation before dispatch. */
class BuiltinPostValidationPolicy implements Step {
  readonly name = 'post_validation_policy';
  readonly description = 'Evaluate policy and approve the validated invocation';
  readonly removable = false;
  readonly replaceable = false;
  readonly pure = true;
  readonly requires = ['validated_inputs', 'policy_acl_checked'] as const;
  readonly [GOVERNANCE_GATE]: GovernanceGateMarker = { pureIsDefault: true };

  constructor(
    private readonly evaluator: PolicyEvaluator,
    private readonly approvalGate: BuiltinApprovalGate,
    private readonly emitter: EventEmitter | null,
  ) {}

  async execute(ctx: PipelineContext): Promise<StepResult> {
    const invocation = policyInvocationOf(ctx);
    if (!invocation?.validated || invocation.aclApprovalRequired === undefined ||
        (!invocation.preflight && !invocation.earlyApproval)) {
      throw new PolicyEvaluationError(ctx.moduleId);
    }
    assertPolicyBinding(ctx, invocation);
    const verdict = await evaluatePolicy(this.evaluator, ctx, invocation, invocation.validated);
    assertPolicyBinding(ctx, invocation);
    if (!invocation.preflight && this.emitter) {
      this.emitter.emit(createEvent('apcore.policy.decision', ctx.moduleId, verdict.decision === 'deny' ? 'warn' : 'info', {
        decision: verdict.decision,
        ...(verdict.ruleId === undefined ? {} : { rule_id: verdict.ruleId }),
        sensitive_fields: [...invocation.validated.sensitiveFields],
      }));
    }
    if (verdict.decision === 'deny') throw new PolicyDeniedError(ctx.moduleId);

    invocation.requiresApproval = invocation.earlyApproval?.needed === true || verdict.decision === 'require_approval';
    if (!invocation.preflight) {
      await this.approvalGate.authorizeFinal(ctx, invocation.requiresApproval);
    }
    invocation.approvedForDispatch = true;
    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// 9. BuiltinMiddlewareBefore
// ---------------------------------------------------------------------------

/** Executes before-middleware chain via MiddlewareManager. */
export class BuiltinMiddlewareBefore implements Step {
  readonly name = 'middleware_before';
  readonly description = 'Execute before-middleware chain';
  readonly removable = true;
  readonly replaceable = false;
  readonly pure = false;

  private _middlewareManager: MiddlewareManager;

  constructor(middlewares: MiddlewareManager) {
    this._middlewareManager = middlewares;
  }

  async execute(ctx: PipelineContext): Promise<StepResult> {
    try {
      const [effectiveInputs, executedMiddlewares] = await this._middlewareManager.executeBefore(
        ctx.moduleId,
        ctx.inputs,
        ctx.context,
      );
      ctx.inputs = effectiveInputs;
      ctx.executedMiddlewares = executedMiddlewares;
    } catch (e) {
      if (e instanceof MiddlewareChainError) {
        // Store the executed middlewares for the executor's on_error recovery,
        // then re-raise WITHOUT running on_error here. The executor (call()/
        // callWithTrace()/stream paths) is the SOLE on_error site, so on_error
        // fires exactly once per attempt. Running it here as well would
        // double-fire stateful handlers — e.g. RetryMiddleware would
        // double-increment its counter, halving the retry budget. Mirrors
        // apcore-python builtin_steps.py BuiltinMiddlewareBefore's
        // MiddlewareChainError branch (sync finding A-D-011).
        ctx.executedMiddlewares = e.executedMiddlewares;
        throw e.original;
      }
      throw e;
    }
    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// 10. BuiltinExecute
// ---------------------------------------------------------------------------

/** Executes the module with timeout enforcement. Sets ctx.output. */
export class BuiltinExecute implements Step {
  readonly name = 'execute';
  readonly description = 'Execute module with timeout';
  readonly removable = false;
  readonly replaceable = true;
  readonly requires = ['module'] as const;
  readonly provides = ['output'] as const;
  readonly pure = false;

  constructor(private readonly _config: Config | null) {}

  async execute(ctx: PipelineContext): Promise<StepResult> {
    assertScopedInvocation(ctx, true);
    assertDeclarationInvocation(ctx);
    const invocation = policyInvocationOf(ctx);
    const declaration = declarationOf(ctx.module);
    let inputs = declaration ? takeDeclarationInputs(ctx) : ctx.inputs;
    if (invocation) {
      assertPolicyBinding(ctx, invocation);
      if (!invocation.approvedForDispatch || !invocation.validated) throw new PolicyEvaluationError(ctx.moduleId);
      ctx.inputs = structuredClone(invocation.validated.arguments);
      inputs = invocation.validated.arguments;
    }
    const mod = ctx.module as Record<string, unknown>;
    const context = ctx.context;

    // Cancel check before execution — throw directly for error type preservation
    if (ctx.context.cancelToken !== null) {
      ctx.context.cancelToken.check();
    }

    // The stream executor pauses the pipeline once the producer is available.
    // Its next configured step runs after the chunks have been accumulated.
    if (invocation?.stream ?? ctx.stream) {
      const candidate = ctx.module as Module;
      if (isStreamingModule(candidate)) {
        if (invocation) {
          const protectedProducer = this._streamWithLease(ctx, () => {
            assertScopedInvocation(ctx, true);
            assertPolicyBinding(ctx, invocation);
            invocation.dispatched = true;
            return candidate.stream(inputs, context);
          });
          invocation.producer = protectedProducer;
          ctx.outputStream = protectedProducer;
        } else {
          ctx.outputStream = this._streamWithLease(ctx, () => {
            assertScopedInvocation(ctx, true);
            return candidate.stream(ctx.inputs, ctx.context);
          });
        }
        return { action: 'continue' };
      }
      // fallback: execute normally and wrap as single-chunk
    }

    // Regular execution with timeout.
    //
    // Spec D-11 (per-module timeout): each module MAY declare its own
    // `resources.timeout` (milliseconds) via either the top-level `resources`
    // field or `annotations.resources.timeout`. The effective timeout is the
    // module value when present; otherwise the executor default. The global
    // deadline (if any) further clamps the effective timeout so the call
    // cannot outrun an outer budget.
    let timeoutMs = this._readModuleTimeoutMs(mod) ??
      (this._config?.get('executor.default_timeout') as number | undefined) ??
      (getDefault('executor.default_timeout') as number);
    // D-100: read the first-class field, not a private `data` key. D-99: the
    // field is epoch SECONDS, so scale it before differencing against
    // `Date.now()` milliseconds.
    const globalDeadline = ctx.context.globalDeadline;
    if (globalDeadline !== null) {
      const remaining = globalDeadline * 1000 - Date.now();
      if (remaining <= 0) {
        ctx.context.cancelToken?.cancel();
        throw new ModuleTimeoutError(ctx.moduleId, 0, {
          retryable: isModuleIdempotent(mod),
        });
      }
      if (timeoutMs === 0 || remaining < timeoutMs) {
        timeoutMs = remaining;
      }
    }

    const executeFn = mod['execute'];
    if (typeof executeFn !== 'function') {
      throw new InvalidInputError(`Module '${ctx.moduleId}' has no execute method`);
    }

    const release = moduleExecutionLeases.get(ctx)?.();
    let executionPromise: Promise<Record<string, unknown>>;
    try {
      recordDeclarationDispatch(ctx);
      if (invocation) invocation.dispatched = true;
      executionPromise = Promise.resolve(
        (
          executeFn as (
            inputs: Record<string, unknown>,
            context: Context,
          ) => Promise<Record<string, unknown>> | Record<string, unknown>
        ).call(mod, inputs, ctx.context),
      );
    } catch (error) {
      release?.();
      throw error;
    }
    if (release) void executionPromise.then(release, release);

    const output = await awaitExecution(
      executionPromise,
      ctx.context.cancelToken,
      ctx.moduleId,
      timeoutMs,
      isModuleIdempotent(mod),
    );
    if (output == null) {
      throw new ModuleExecuteError(ctx.moduleId, 'Return value cannot be None');
    }
    const normalized = normalizeModuleObject(output);
    if (normalized === null) {
      throw new ModuleExecuteError(ctx.moduleId, 'Return value must be Map');
    }
    ctx.output = normalized;

    return { action: 'continue' };
  }

  private async *_streamWithLease(
    ctx: PipelineContext,
    createStream: () => AsyncGenerator<Record<string, unknown>>,
  ): AsyncGenerator<Record<string, unknown>> {
    const release = moduleExecutionLeases.get(ctx)?.();
    try {
      yield* createStream();
    } finally {
      release?.();
    }
  }

  /**
   * Read the module's declared `resources.timeout` (D-11) in milliseconds.
   * Returns `null` when not declared. Supports two equivalent locations
   * (module-level `resources` or `annotations.resources`) for parity with
   * the other SDKs.
   */
  private _readModuleTimeoutMs(mod: Record<string, unknown>): number | null {
    // A declared numeric timeout is honoured; a declared NEGATIVE timeout is an
    // invalid input and MUST raise GENERAL_INVALID_INPUT (spec Edge Cases;
    // parity with apcore-python builtin_steps.py:620). A non-numeric / absent
    // value is treated as "not declared" → null (caller falls back to default).
    const readNumber = (val: unknown): number | null => {
      if (typeof val !== 'number' || !Number.isFinite(val)) return null;
      if (val < 0) {
        throw new InvalidInputError(`Negative timeout: ${val}`);
      }
      return val;
    };

    const readResources = (value: unknown) =>
      value !== null && typeof value === 'object'
        ? readNumber((value as Record<string, unknown>)['timeout'])
        : null;
    const annotations = mod['annotations'] as Record<string, unknown> | null | undefined;
    const extra = annotations?.['extra'] as Record<string, unknown> | null | undefined;
    return readResources(mod['resources']) ??
      readResources(annotations?.['resources']) ??
      readResources(extra?.['resources']);
  }

}

// ---------------------------------------------------------------------------
// 11. BuiltinOutputValidation
// ---------------------------------------------------------------------------

/** Validates output against module schema and redacts sensitive fields. */
export class BuiltinOutputValidation implements Step {
  /** The resolved `obs.redaction.*` rules (§10.6.1 "Where the rules apply"). */
  constructor(private readonly _redaction: RedactionConfig | null = null) {}

  readonly name = 'output_validation';
  readonly description = 'Schema validation and redaction for output';
  readonly removable = true;
  readonly requires = ['module', 'output'] as const;
  readonly provides = ['validated_output'] as const;
  readonly replaceable = true;
  readonly pure = true;

  async execute(ctx: PipelineContext): Promise<StepResult> {
    // Skip when no output is available: streaming (Phase 3 handles it),
    // dry_run (execute step was skipped), etc.
    if (ctx.output == null) {
      return { action: 'continue' };
    }

    const mod = ctx.module as Record<string, unknown>;
    const output = ctx.output;

    const outputSchema = resolveSchema(mod, 'outputSchema');
    if (outputSchema == null) {
      ctx.validatedOutput = output;
      // Capture the output with the configured rules even without a schema
      // (PROTOCOL_SPEC 10.6.1 requirements 1 and 3), so a logging middleware
      // has a redacted value to log and never needs the raw one (D-131).
      if (ctx.context != null && typeof output === 'object' && !Array.isArray(output)) {
        ctx.context.redactedOutput = redactSensitive(output, {}, this._redaction);
      }
      return { action: 'continue' };
    }

    // Throws SchemaValidationError directly for error type preservation
    validateSchema(outputSchema, output, 'Output');

    ctx.validatedOutput = output;

    // Store redacted output as first-class Context field (symmetric with
    // redactedInputs). Available to middleware.after() and serialize().
    if (ctx.context != null) {
      ctx.context.redactedOutput = redactSensitive(
        output,
        outputSchema as unknown as Record<string, unknown>,
        this._redaction,
      );
    }

    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// 12. BuiltinMiddlewareAfter
// ---------------------------------------------------------------------------

/** Executes after-middleware chain via MiddlewareManager. */
export class BuiltinMiddlewareAfter implements Step {
  readonly name = 'middleware_after';
  readonly description = 'Execute after-middleware chain';
  readonly removable = true;
  readonly replaceable = false;
  readonly pure = false;

  private _middlewareManager: MiddlewareManager;

  constructor(middlewares: MiddlewareManager) {
    this._middlewareManager = middlewares;
  }

  async execute(ctx: PipelineContext): Promise<StepResult> {
    if (ctx.output == null) {
      return { action: 'continue' };
    }

    const transformed = await this._middlewareManager.executeAfter(
      ctx.moduleId,
      ctx.inputs,
      ctx.output,
      ctx.context,
    );
    ctx.output = transformed;
    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// 13. BuiltinReturnResult
// ---------------------------------------------------------------------------

/** Finalizes the pipeline result. Output is already on ctx.output. */
export class BuiltinReturnResult implements Step {
  readonly name = 'return_result';
  readonly description = 'Finalize pipeline result';
  readonly removable = false;
  readonly requires = ['output'] as const;
  readonly replaceable = false;
  readonly pure = true;

  async execute(_ctx: PipelineContext): Promise<StepResult> {
    // No-op: output is already on ctx.output from previous steps
    return { action: 'continue' };
  }
}

// ---------------------------------------------------------------------------
// Factory: buildStandardStrategy
// ---------------------------------------------------------------------------

export interface StandardStrategyDeps {
  config: Config | null;
  registry: Registry;
  acl: ACL | null;
  approvalHandler: ApprovalHandler | null;
  middlewareManager: MiddlewareManager;
  toggleState?: ToggleState | null;
  /** Optional ExecutionPolicy with governance overrides for the Step 5 gate (apcore#76). */
  policy?: ExecutionPolicy | null;
  /**
   * Optional EventEmitter. When provided, the ACL step publishes
   * apcore.acl.denied and the approval gate publishes apcore.approval.decision
   * / apcore.policy.override governance events (apcore#77).
   */
  eventEmitter?: EventEmitter | null;
}

/** Build the standard 11-step execution strategy matching PROTOCOL_SPEC §5 pipeline.
 *
 * Toggle-state check is inlined inside BuiltinModuleLookup (step 3), matching
 * apcore-python and apcore-rust 11-step implementations (sync finding A-D-011).
 */
/**
 * The `obs.redaction.*` rules the capture point applies.
 *
 * PROTOCOL_SPEC §10.6.1 "Where the rules apply" — the union of `x-sensitive`,
 * `sensitive_keys` and `regex_patterns` MUST hold at log emission AND at the
 * executor's input/output capture point, with the same rules at each. Resolved
 * ONCE per strategy, not per execution: requirement 5 puts compilation at the
 * configuration read, and `RedactionConfig` compiles `regex_patterns` in its
 * constructor. Resolving here also means the two `redactedInputs` writers and
 * the `redactedOutput` writer share one object and cannot drift apart.
 *
 * With no config the DEFAULTS apply (requirement 3), which is what the capture
 * point already did — so an unconfigured caller sees no change.
 */
function resolveRedaction(config: Config | null): RedactionConfig {
  return config === null ? RedactionConfig.default() : RedactionConfig.fromConfig(config);
}

export function buildStandardStrategy(deps: StandardStrategyDeps): ExecutionStrategy {
  const redaction = resolveRedaction(deps.config);
  return new ExecutionStrategy('standard', [
    new BuiltinContextCreation(deps.config),
    new BuiltinCallChainGuard(deps.config),
    new BuiltinModuleLookup(deps.registry, deps.toggleState ?? undefined, redaction),
    new BuiltinACLCheck(deps.acl, deps.eventEmitter ?? null),
    new BuiltinApprovalGate(deps.approvalHandler, deps.policy ?? null, deps.eventEmitter ?? null),
    new BuiltinMiddlewareBefore(deps.middlewareManager),
    new BuiltinInputValidation(redaction),
    new BuiltinExecute(deps.config),
    new BuiltinOutputValidation(redaction),
    new BuiltinMiddlewareAfter(deps.middlewareManager),
    new BuiltinReturnResult(),
  ]);
}

/**
 * Build an internal-only strategy: skips ACL and approval gates.
 * Suitable for trusted internal service-to-service calls.
 */
export function buildInternalStrategy(deps: StandardStrategyDeps): ExecutionStrategy {
  const redaction = resolveRedaction(deps.config);
  return new ExecutionStrategy('internal', [
    new BuiltinContextCreation(deps.config),
    new BuiltinCallChainGuard(deps.config),
    new BuiltinModuleLookup(deps.registry, deps.toggleState ?? undefined, redaction),
    new BuiltinMiddlewareBefore(deps.middlewareManager),
    new BuiltinInputValidation(redaction),
    new BuiltinExecute(deps.config),
    new BuiltinOutputValidation(redaction),
    new BuiltinMiddlewareAfter(deps.middlewareManager),
    new BuiltinReturnResult(),
  ]);
}

/**
 * Build a testing strategy: standard minus ACL, approval, and call chain guard.
 * Retains middleware and validation for correctness. Fast and predictable for tests.
 */
export function buildTestingStrategy(deps: StandardStrategyDeps): ExecutionStrategy {
  const redaction = resolveRedaction(deps.config);
  return new ExecutionStrategy('testing', [
    new BuiltinContextCreation(deps.config),
    new BuiltinModuleLookup(deps.registry, deps.toggleState ?? undefined, redaction),
    new BuiltinMiddlewareBefore(deps.middlewareManager),
    new BuiltinInputValidation(redaction),
    new BuiltinExecute(deps.config),
    new BuiltinOutputValidation(redaction),
    new BuiltinMiddlewareAfter(deps.middlewareManager),
    new BuiltinReturnResult(),
  ]);
}

/**
 * Build a performance strategy: skips middleware before/after for reduced overhead.
 * Retains toggle state, ACL, approval, and validation for correctness.
 * Toggle-state check is inlined in BuiltinModuleLookup (sync finding A-D-011).
 */
export function buildPerformanceStrategy(deps: StandardStrategyDeps): ExecutionStrategy {
  const redaction = resolveRedaction(deps.config);
  return new ExecutionStrategy('performance', [
    new BuiltinContextCreation(deps.config),
    new BuiltinCallChainGuard(deps.config),
    new BuiltinModuleLookup(deps.registry, deps.toggleState ?? undefined, redaction),
    new BuiltinACLCheck(deps.acl, deps.eventEmitter ?? null),
    new BuiltinApprovalGate(deps.approvalHandler, deps.policy ?? null, deps.eventEmitter ?? null),
    new BuiltinInputValidation(redaction),
    new BuiltinExecute(deps.config),
    new BuiltinOutputValidation(redaction),
    new BuiltinReturnResult(),
  ]);
}

/** Attach the final gate only to a strategy with enforceable built-in boundaries. */
export function withPostValidationPolicy(
  strategy: ExecutionStrategy,
  evaluator: PolicyEvaluator,
  emitter: EventEmitter | null,
): ExecutionStrategy {
  const withMiddleware = strategy.steps.length === 11;
  const expected = [
    BuiltinContextCreation, BuiltinCallChainGuard, BuiltinModuleLookup, BuiltinACLCheck, BuiltinApprovalGate,
    ...(withMiddleware ? [BuiltinMiddlewareBefore] : []), BuiltinInputValidation,
    BuiltinExecute, BuiltinOutputValidation, ...(withMiddleware ? [BuiltinMiddlewareAfter] : []), BuiltinReturnResult,
  ];
  const pureSteps: ReadonlySet<unknown> = new Set([
    BuiltinContextCreation, BuiltinCallChainGuard, BuiltinModuleLookup, BuiltinACLCheck, BuiltinInputValidation,
    BuiltinOutputValidation, BuiltinReturnResult,
  ]);
  if (strategy.steps.length !== expected.length || strategy.steps.some((step, index) => {
    const expectedType = expected[index];
    return Object.getPrototypeOf(step) !== expectedType.prototype || step.execute !== expectedType.prototype.execute ||
      step.matchModules != null || step.ignoreErrors === true || Boolean(step.pure) !== pureSteps.has(expectedType);
  })) {
    throw new ConfigurationError('A policy evaluator requires the built-in standard or performance pipeline boundaries.');
  }
  const approvedStrategy = strategy.mapSteps(step => step);
  const approvalGate = approvedStrategy.steps[4] as BuiltinApprovalGate;
  approvedStrategy.insertAfter('input_validation', new BuiltinPostValidationPolicy(evaluator, approvalGate, emitter));
  return approvedStrategy;
}

/**
 * Build a minimal strategy: context → lookup → execute → return only.
 * No safety checks, no ACL, no approval, no validation, no middleware.
 * Suitable for pre-validated internal hot paths. Use with caution.
 */
export function buildMinimalStrategy(deps: StandardStrategyDeps): ExecutionStrategy {
  const redaction = resolveRedaction(deps.config);
  return new ExecutionStrategy('minimal', [
    new BuiltinContextCreation(deps.config),
    // The per-instance ToggleState must be threaded through here too — the
    // other four preset builders pass it, and omitting it silently falls back
    // to DEFAULT_TOGGLE_STATE, so `apcore.disable(module)` on one instance
    // leaks into every other instance using the `minimal` preset (issue #71).
    new BuiltinModuleLookup(deps.registry, deps.toggleState ?? undefined, redaction),
    new BuiltinExecute(deps.config),
    new BuiltinReturnResult(),
  ]);
}
