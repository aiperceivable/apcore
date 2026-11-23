/**
 * Execution-time governance policy: external overrides for governance annotations.
 *
 * RFC pilot for apcore#76 ("first-class external governance / policy API").
 * An {@link ExecutionPolicy} lets a platform operator override the governance
 * annotations (`requiresApproval`, `destructive`) of already-registered modules
 * at execution time, independent of how the modules were registered. It attaches
 * to the {@link Executor} (`policy` option) and is consulted by the approval
 * gate (Step 5).
 *
 * Precedence: a matched policy rule overrides the module's own declared or
 * scanned annotations — external governance is the platform's word against the
 * module author's. Matching uses the same wildcard semantics as the ACL system
 * (Algorithm A08) and the same specificity scoring (Algorithm A10); on a
 * specificity tie the more restrictive rule wins.
 *
 * Governance principle (apcore#76): a misconfigured or unreachable governance
 * control must warn or error, never silently allow. {@link ExecutionPolicy.fromObject}
 * therefore rejects unknown keys (a typo in a governance file must not silently
 * no-op), and `strict=true` makes the approval gate fail closed when approval is
 * required but no ApprovalHandler is configured.
 */

import type { Context } from './context.js';
import type { ModuleAnnotations } from './module.js';
import { calculateSpecificity, matchPattern } from './utils/pattern.js';

const POLICY_KEYS = new Set(['rules', 'gate_destructive', 'strict']);
const RULE_KEYS = new Set(['pattern', 'requires_approval', 'destructive', 'reason']);

/**
 * Read a governance switch from a parsed policy document, rejecting anything
 * that is not a real boolean.
 *
 * `Boolean(value)` would apply JS truthiness, which disagrees with every other
 * SDK: `[]` is `true` in JS but `False` in Python, and `"false"` is `true` in
 * both — while apcore-rust's serde-typed `bool` field refuses both. These
 * switches decide whether a `destructive` annotation becomes an approval gate
 * and whether the gate fails closed, so a silent coercion is a governance
 * decision made by accident. `fromObject` already fails loud on unknown keys
 * for the same reason; this extends that discipline to the values.
 *
 * `undefined` / `null` mean "unset" and take the documented default of `false`.
 */
function _requireBoolean(value: unknown, key: string): boolean {
  if (value === undefined || value === null) return false;
  if (typeof value !== 'boolean') {
    const kind = Array.isArray(value) ? 'array' : typeof value;
    throw new Error(
      `Policy key '${key}' must be a boolean, got ${kind} (${JSON.stringify(value)}); ` +
        'a governance switch must not be set by type coercion',
    );
  }
  return value;
}

/**
 * The call site a policy decision is being made for (PROTOCOL_SPEC §7.9.6,
 * spec v1.24.0, apcore#102).
 *
 * Through spec v1.23.0 a policy could see only *which* module was being
 * called, never *what it was being called with*, so an operator who needed to
 * gate some calls to a module had to gate all of them. The pipeline already
 * holds this data at the point of the decision — the approval gate is Step 5
 * and the invocation's arguments and `Context` are in scope there — so
 * {@link ExecutionPolicy.resolve} now receives it.
 *
 * Two constraints are normative:
 *
 * 1. The built-in pattern rules of §7.9.1 **MUST NOT** consult it. A rule
 *    set's verdict stays a function of the module ID and the annotations
 *    alone, so it remains statically auditable and reproducible from the
 *    policy document. It exists for host-supplied policies (subclass
 *    `ExecutionPolicy` and override `resolve`) and for carrying the call site
 *    into the audit trail.
 * 2. `arguments` has **NOT** been schema-validated. The approval gate is
 *    Step 5 and input validation is Step 7 (§12.8), so a host-supplied policy
 *    MUST NOT assume its inputs are well-formed, present, or of the declared
 *    type.
 */
export interface PolicyCallSite {
  /**
   * The raw invocation arguments, exactly as handed to the approval gate.
   * NOT schema-validated — see the note on {@link PolicyCallSite}.
   */
  readonly arguments: Record<string, unknown> | null;
  /** The execution {@link Context} the call is running under, when present. */
  readonly context: Context | null;
}

/** Overrides accepted by the {@link PolicyRule} constructor. */
export interface PolicyRuleOverrides {
  /**
   * Override for the module's `requiresApproval` annotation. `null`/`undefined`
   * leaves the module's own value in effect.
   */
  requiresApproval?: boolean | null;
  /**
   * Override for the module's `destructive` annotation. `null`/`undefined`
   * leaves the module's own value in effect.
   */
  destructive?: boolean | null;
  /** Human-readable rationale, carried into the audit trail. */
  reason?: string | null;
}

/**
 * A single pattern-based governance override (apcore#76 RFC pilot).
 *
 * Immutable; validates its fields eagerly so a malformed governance rule fails
 * loud at construction rather than silently no-op'ing at execution time.
 */
export class PolicyRule {
  /**
   * Module ID wildcard pattern (Algorithm A08 semantics, same as ACL rules).
   * `*` matches any character sequence including dots.
   */
  readonly pattern: string;
  readonly requiresApproval: boolean | null;
  readonly destructive: boolean | null;
  readonly reason: string | null;

  constructor(pattern: string, overrides: PolicyRuleOverrides = {}) {
    if (typeof pattern !== 'string' || pattern.length === 0) {
      throw new Error('PolicyRule.pattern must be a non-empty string');
    }
    const requiresApproval = overrides.requiresApproval ?? null;
    const destructive = overrides.destructive ?? null;
    const reason = overrides.reason ?? null;
    for (const [name, value] of [
      ['requiresApproval', requiresApproval],
      ['destructive', destructive],
    ] as const) {
      if (value !== null && typeof value !== 'boolean') {
        throw new Error(`PolicyRule.${name} must be a boolean or null, got ${typeof value}`);
      }
    }
    if (reason !== null && typeof reason !== 'string') {
      throw new Error(`PolicyRule.reason must be a string or null, got ${typeof reason}`);
    }
    this.pattern = pattern;
    this.requiresApproval = requiresApproval;
    this.destructive = destructive;
    this.reason = reason;
    Object.freeze(this);
  }
}

/**
 * The effective governance verdict for one module under a policy.
 *
 * - `requiresApproval` / `destructive`: effective values after overrides.
 * - `needsApproval`: final approval-gate verdict — true when the effective
 *   `requiresApproval` is true, or when the policy gates destructive modules
 *   and the effective `destructive` is true.
 * - `rule`: the matched {@link PolicyRule}, or `null` when no rule matched.
 * - `overridden`: true when the matched rule changed a base annotation value.
 */
export interface PolicyDecision {
  readonly moduleId: string;
  readonly requiresApproval: boolean;
  readonly destructive: boolean;
  readonly needsApproval: boolean;
  readonly rule: PolicyRule | null;
  readonly overridden: boolean;
}

/** Read a boolean annotation from a ModuleAnnotations object or a dict. */
function readAnnotationBool(annotations: unknown, camelKey: string, snakeKey: string): boolean {
  if (annotations == null || typeof annotations !== 'object') return false;
  const obj = annotations as Record<string, unknown>;
  if (camelKey in obj) return Boolean(obj[camelKey]);
  if (snakeKey in obj) return Boolean(obj[snakeKey]);
  return false;
}

/** Tie-break key: rules that force gating rank above rules that relax it. */
function restrictiveness(rule: PolicyRule): number {
  // (requiresApproval === true) outranks (destructive === true); encode as a
  // 2-bit score so a lexicographic tuple comparison collapses to `>`.
  return (rule.requiresApproval === true ? 2 : 0) + (rule.destructive === true ? 1 : 0);
}

/**
 * Declarative execution-time governance overrides (apcore#76 RFC pilot).
 *
 * @example
 * ```ts
 * const policy = new ExecutionPolicy(
 *   [new PolicyRule('orders.delete_*', { requiresApproval: true, reason: 'human sign-off' })],
 *   { gateDestructive: true, strict: true },
 * );
 * const executor = new Executor({ registry, approvalHandler: handler, policy });
 * ```
 */
export class ExecutionPolicy {
  private readonly _rules: readonly PolicyRule[];
  /**
   * When true, modules whose effective `destructive` annotation is true require
   * approval even if `requiresApproval` is false — the opt-in resolution of the
   * destructive↔approval gap described in apcore#76.
   */
  readonly gateDestructive: boolean;
  /**
   * When true, the approval gate fails closed (denies) when a module needs
   * approval but no ApprovalHandler is configured. When false (default), the
   * gate keeps the PROTOCOL_SPEC §7.4 skip behavior but logs a warning.
   */
  readonly strict: boolean;

  constructor(
    rules: PolicyRule[] | null = null,
    options: { gateDestructive?: boolean; strict?: boolean } = {},
  ) {
    const frozen = Object.freeze([...(rules ?? [])]);
    for (const rule of frozen) {
      if (!(rule instanceof PolicyRule)) {
        throw new Error(`ExecutionPolicy rules must be PolicyRule instances, got ${typeof rule}`);
      }
    }
    this._rules = frozen;
    this.gateDestructive = Boolean(options.gateDestructive);
    this.strict = Boolean(options.strict);
  }

  /** The policy's rules as an immutable array. */
  get rules(): readonly PolicyRule[] {
    return this._rules;
  }

  /**
   * Compute the effective governance decision for a module.
   *
   * @param moduleId - Canonical module ID being invoked.
   * @param annotations - The module's annotations (ModuleAnnotations, dict, or
   *   null). Only `requiresApproval`/`requires_approval` and `destructive` are
   *   consulted.
   * @param callSite - The invocation's arguments and `Context`
   *   (PROTOCOL_SPEC §7.9.6). Optional, so every existing caller keeps
   *   compiling and behaving identically. The built-in pattern rules below
   *   deliberately ignore it — §7.9.6(2) requires the verdict to be a function
   *   of the module ID and annotations alone — and it is accepted here so a
   *   host-supplied policy can override this method and decide on arguments,
   *   and so an implementation can carry the call site into the audit trail.
   */
  resolve(
    moduleId: string,
    annotations: unknown = null,
    callSite: PolicyCallSite | null = null,
  ): PolicyDecision {
    // §7.9.6(2) and (5): the built-in rules MUST NOT consult the call site,
    // and adding it MUST NOT change the verdict any existing policy produces.
    // Referenced explicitly so the parameter reads as deliberate rather than
    // forgotten.
    void callSite;
    const baseRequiresApproval = readAnnotationBool(
      annotations,
      'requiresApproval',
      'requires_approval',
    );
    const baseDestructive = readAnnotationBool(annotations, 'destructive', 'destructive');

    const rule = this._match(moduleId);
    const effectiveRequiresApproval =
      rule === null || rule.requiresApproval === null
        ? baseRequiresApproval
        : rule.requiresApproval;
    const effectiveDestructive =
      rule === null || rule.destructive === null ? baseDestructive : rule.destructive;

    return {
      moduleId,
      requiresApproval: effectiveRequiresApproval,
      destructive: effectiveDestructive,
      needsApproval: effectiveRequiresApproval || (this.gateDestructive && effectiveDestructive),
      rule,
      overridden:
        effectiveRequiresApproval !== baseRequiresApproval ||
        effectiveDestructive !== baseDestructive,
    };
  }

  /** Return the winning rule for a module ID, or null. */
  private _match(moduleId: string): PolicyRule | null {
    let best: PolicyRule | null = null;
    let bestScore = -1;
    for (const rule of this._rules) {
      if (!matchPattern(rule.pattern, moduleId)) continue;
      const score = calculateSpecificity(rule.pattern);
      if (score > bestScore) {
        best = rule;
        bestScore = score;
      } else if (
        score === bestScore &&
        best !== null &&
        restrictiveness(rule) > restrictiveness(best)
      ) {
        best = rule;
      }
    }
    return best;
  }

  /**
   * Build a policy from a plain object (parsed YAML/JSON governance file).
   *
   * Parsing is strict: unknown keys throw so a typo in a governance file fails
   * loud instead of silently disabling a control.
   *
   * Expected shape:
   * ```yaml
   * gate_destructive: true
   * strict: true
   * rules:
   *   - pattern: "orders.delete_*"
   *     requires_approval: true
   *     reason: "destructive order operations need human sign-off"
   * ```
   *
   * @throws {Error} On unknown keys, missing `pattern`, or wrong types.
   */
  static fromObject(data: unknown): ExecutionPolicy {
    if (data == null || typeof data !== 'object' || Array.isArray(data)) {
      const kind = Array.isArray(data) ? 'array' : typeof data;
      throw new Error(`Policy document must be a mapping, got ${kind}`);
    }
    const obj = data as Record<string, unknown>;
    const unknown = Object.keys(obj).filter((k) => !POLICY_KEYS.has(k));
    if (unknown.length > 0) {
      throw new Error(
        `Unknown policy keys ${JSON.stringify(unknown.sort())}; a governance file must not silently no-op`,
      );
    }

    const rulesRaw = obj['rules'] ?? [];
    if (!Array.isArray(rulesRaw)) {
      throw new Error(`Policy 'rules' must be a list, got ${typeof rulesRaw}`);
    }

    const rules: PolicyRule[] = [];
    rulesRaw.forEach((item, index) => {
      if (item == null || typeof item !== 'object' || Array.isArray(item)) {
        const kind = Array.isArray(item) ? 'array' : typeof item;
        throw new Error(`Policy rule #${index} must be a mapping, got ${kind}`);
      }
      const ruleObj = item as Record<string, unknown>;
      const unknownRule = Object.keys(ruleObj).filter((k) => !RULE_KEYS.has(k));
      if (unknownRule.length > 0) {
        throw new Error(
          `Policy rule #${index} has unknown keys ${JSON.stringify(unknownRule.sort())}; ` +
            'a governance rule must not silently no-op',
        );
      }
      if (!('pattern' in ruleObj)) {
        throw new Error(`Policy rule #${index} is missing required key 'pattern'`);
      }
      rules.push(
        new PolicyRule(ruleObj['pattern'] as string, {
          requiresApproval: (ruleObj['requires_approval'] ?? null) as boolean | null,
          destructive: (ruleObj['destructive'] ?? null) as boolean | null,
          reason: (ruleObj['reason'] ?? null) as string | null,
        }),
      );
    });

    return new ExecutionPolicy(rules, {
      gateDestructive: _requireBoolean(obj['gate_destructive'], 'gate_destructive'),
      strict: _requireBoolean(obj['strict'], 'strict'),
    });
  }
}

/**
 * Build effective ModuleAnnotations for an ApprovalRequest under a policy
 * decision, preserving the PROTOCOL_SPEC §7 "requiresApproval is guaranteed
 * true" contract: the handler sees the effective governance values, not the
 * module's raw declaration.
 */
export function applyDecisionToAnnotations(
  annotations: ModuleAnnotations,
  decision: PolicyDecision,
): ModuleAnnotations {
  return Object.freeze({
    ...annotations,
    requiresApproval: true,
    destructive: decision.destructive,
  });
}
