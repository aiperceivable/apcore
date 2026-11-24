/**
 * Built-in ACL condition handlers and handler interface.
 *
 * Defines the ACLConditionHandler interface, four basic handlers
 * (identity_types, roles, max_call_depth, arguments), and two compound
 * operators ($or, $not).
 */

import type { Context } from './context.js';

/** Handler interface for evaluating a single ACL condition. */
export interface ACLConditionHandler {
  evaluate(value: unknown, context: Context): boolean | Promise<boolean>;
}

/**
 * Three-valued outcome of evaluating an ACL condition (PROTOCOL_SPEC §6.1.1).
 *
 * `'unsatisfied'` means a registered handler ran to completion and answered
 * "no" — an ordinary non-match. `'unevaluable'` means no answer was obtainable
 * at all, which is a different outcome and resolves the enclosing rule toward
 * refusing access: a `deny` rule takes effect, an `allow` rule does not grant.
 * Collapsing the two is the defect §6.1.1 exists to prevent.
 */
export type ConditionOutcome = 'satisfied' | 'unsatisfied' | 'unevaluable';

/**
 * Type alias for the recursive evaluation function used by compound handlers.
 *
 * `path` is the PROTOCOL_SPEC §6.1.4 condition path of the sub-object being
 * evaluated (`$or[1]`, `$or[1].$not`, `''` at the root), so a fault found
 * inside it can be reported at its own position rather than under the
 * operator's key.
 */
export type EvalFn = (
  conditions: Record<string, unknown>,
  context: Context,
  path: string,
) => ConditionOutcome;

/** Async variant of EvalFn for use under asyncCheck(). */
export type AsyncEvalFn = (
  conditions: Record<string, unknown>,
  context: Context,
  path: string,
) => Promise<ConditionOutcome>;

/**
 * A condition handler able to report UNEVALUABLE separately from UNSATISFIED.
 *
 * The public {@link ACLConditionHandler} contract returns a boolean, which
 * cannot carry PROTOCOL_SPEC §6.1.1's third outcome — so the built-in
 * compound operators (`$or`, `$not`), whose children can themselves be
 * unevaluable, implement this richer interface as well. A handler that only
 * implements `evaluate()` keeps the two-outcome contract: `true` is SATISFIED,
 * `false` is UNSATISFIED, a throw is UNEVALUABLE.
 */
export interface ACLOutcomeConditionHandler extends ACLConditionHandler {
  /**
   * @param path - This condition key's own §6.1.4 path (`$or`, `$or[1].$not`),
   *   from which a compound handler derives its children's paths.
   */
  evaluateOutcome(
    value: unknown,
    context: Context,
    path: string,
  ): ConditionOutcome | Promise<ConditionOutcome>;
}

/** True when a handler can report the three-valued outcome directly. */
export function isOutcomeHandler(
  handler: ACLConditionHandler,
): handler is ACLOutcomeConditionHandler {
  return typeof (handler as Partial<ACLOutcomeConditionHandler>).evaluateOutcome === 'function';
}

/** True only for a plain object — excludes null, arrays and every primitive. */
function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

// ---------------------------------------------------------------------------
// Governance projection (PROTOCOL_SPEC §6.1.8)
// ---------------------------------------------------------------------------

/**
 * The structure-only view of a call's arguments that the `arguments` condition
 * reads (PROTOCOL_SPEC §6.1.8).
 *
 * It carries the argument **key set** and each key's JSON type, and it carries
 * **no value at all**. That is structural, not a convention: a projection that
 * cannot hold a value cannot leak one, whatever a future predicate does with
 * it.
 *
 * It is deliberately NOT `Context.redactedInputs`. That field's contract is
 * safe *logging*; it is a raw copy of the arguments when the module declares no
 * `inputSchema` (redaction is driven by `x-sensitive` markers in that schema),
 * and one field serving both "safe to log" and "input to a security decision"
 * will eventually break one of them in a change made for the other.
 */
export interface GovernanceProjection {
  /** The argument key set, in the order the arguments object presented them. */
  readonly keys: readonly string[];
  /** Each key's JSON type name — never its value. */
  readonly types: Readonly<Record<string, string>>;
}

/**
 * `_approval_token` is a protocol-level key (PROTOCOL_SPEC §7.4), not caller
 * input, so it is excluded from the projection for the same reason §7.9.6
 * rule 5 strips it before policy resolution: a retry carrying the token must
 * present the same argument shape to governance as the original call did.
 */
const APPROVAL_TOKEN_KEY = '_approval_token';

/** JSON type name for a projected argument — mirrors `aclTypeName` in acl.ts. */
function jsonTypeName(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  return typeof value;
}

/**
 * Build the §6.1.8 governance projection of a call's arguments.
 *
 * Computed during module lookup (Step 3) and handed to the ACL check
 * (Step 4); the ordering is normative in §6.1.8 rule 1 rather than an
 * implementation detail that happens to hold.
 */
export function buildGovernanceProjection(
  args: Record<string, unknown> | null | undefined,
): GovernanceProjection {
  const keys: string[] = [];
  const types: Record<string, string> = {};
  for (const [key, value] of Object.entries(args ?? {})) {
    if (key === APPROVAL_TOKEN_KEY) continue;
    keys.push(key);
    types[key] = jsonTypeName(value);
  }
  return Object.freeze({ keys: Object.freeze(keys), types: Object.freeze(types) });
}

// ---------------------------------------------------------------------------
// The `arguments` condition (PROTOCOL_SPEC §6.1.7)
// ---------------------------------------------------------------------------

/**
 * The complete predicate vocabulary of the `arguments` condition (§6.1.7).
 *
 * Fixed, and there is no registration point for it: `registerCondition` writes
 * runtime code into a process-wide registry, and a deployment-registered
 * argument handler is exactly the unauditable host code §7.9.6 rule 2 exists to
 * keep out of a governance verdict. A fixed vocabulary keeps the decision
 * reproducible from the ACL document alone.
 */
export const ARGUMENT_PREDICATES: ReadonlySet<string> = new Set([
  'has_key',
  'has_all_keys',
  'has_none_of',
]);

/** True for a list of strings — the only well-formed predicate value (§6.1.7). */
function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((v) => typeof v === 'string');
}

/**
 * Structural fault in an `arguments` condition value, or `null` when it is
 * well-formed. Context-independent and handler-free by construction, so
 * §6.1.4's precheck can call it and every SDK reports the same fault set.
 *
 * Every fault here is UNEVALUABLE under §6.1.1's principle, never UNSATISFIED:
 * a malformed predicate does not ask a question the implementation declined to
 * answer, it asks no question at all — and recording that as "false" puts a
 * `deny` rule carrying `has_keys:` (for `has_all_keys:`) back into the inert
 * state §6.1.1 exists to end.
 *
 * §6.1.8 fixes the reported PATH: it descends to `arguments.<predicate>` where
 * one predicate is at fault and stops at `arguments` where none can be named,
 * exactly as §6.1.4 descends into `$or[1].k`. `arguments` alone does not say
 * which of several predicates is wrong.
 *
 * **Every** faulty predicate is reported and the walk does not stop at the
 * first (§6.1.8 rule 3). Reporting one would make the *choice* of which one
 * observable, and no reading of §6.1.4 makes "the first" a pure function of the
 * rule: `{ has_key: 'force', zzz: [...] }` carries both a malformed value and
 * an unrecognised name, and checking names before values reports a different
 * path from walking predicate by predicate.
 *
 * @param path - The §6.1.4 condition path of the `arguments` key itself.
 * @returns One entry per fault, empty when well-formed.
 */
export interface ArgumentsFault {
  /** The §6.1.8 condition path — `arguments` or `arguments.<predicate>`. */
  readonly path: string;
  readonly message: string;
}

export function describeArgumentsFaults(value: unknown, path: string): ArgumentsFault[] {
  if (!isPlainObject(value)) {
    const kind = Array.isArray(value) ? 'array' : value === null ? 'null' : typeof value;
    // Terminal: a value that is not a mapping has no predicates to walk.
    return [
      { path, message: `ACL condition '${path}' must be a mapping of argument predicates, got ${kind}` },
    ];
  }
  const entries = Object.entries(value);
  if (entries.length === 0) {
    // `arguments: {}` asks nothing. Reading it as vacuously true would widen
    // an `allow` rule with no warning, which is the §6.1.5 failure class.
    return [
      {
        path,
        message:
          `ACL condition '${path}' carries no predicate; expected at least one of ` +
          `${[...ARGUMENT_PREDICATES].sort().join(', ')}`,
      },
    ];
  }
  // Sorted rather than in insertion order, so the finding ORDER is a pure
  // function of the rule (§6.1.4 determinism) when a block carries two faults.
  const faults: ArgumentsFault[] = [];
  for (const [predicate, names] of [...entries].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    if (!ARGUMENT_PREDICATES.has(predicate)) {
      faults.push({
        path: `${path}.${predicate}`,
        message:
          `Unknown ACL argument predicate '${path}.${predicate}'; the vocabulary is closed ` +
          `(${[...ARGUMENT_PREDICATES].sort().join(', ')})`,
      });
      continue;
    }
    if (!isStringArray(names)) {
      const kind = Array.isArray(names) ? 'array with a non-string element' : typeof names;
      faults.push({
        path: `${path}.${predicate}`,
        message: `ACL argument predicate '${path}.${predicate}' must be a list of strings, got ${kind}`,
      });
    }
  }
  return faults;
}

/** Supplies the projection bound to the ACL evaluation currently in flight. */
export type GovernanceProjectionProvider = () => GovernanceProjection | null;

/**
 * `arguments`: structure-only predicates over the call's argument KEYS
 * (PROTOCOL_SPEC §6.1.7).
 *
 * | Predicate      | Passes when                                      |
 * |----------------|--------------------------------------------------|
 * | `has_key`      | **any** of the named keys is present             |
 * | `has_all_keys` | **every** named key is present                   |
 * | `has_none_of`  | **none** of the named keys is present            |
 *
 * Several predicates in one `arguments` object are AND-ed, matching how a
 * `conditions` object combines its own keys.
 *
 * **No predicate reads a value.** The argument view available at Step 4 is not
 * reliably redacted — redaction is driven by `x-sensitive` markers in the
 * module's `inputSchema`, and a module without one gets none — and the
 * arguments are unvalidated, because the ACL check is Step 4 and input schema
 * validation is Step 7. Key presence is the one question well-defined on what
 * is available.
 */
export class ArgumentsHandler implements ACLOutcomeConditionHandler {
  private readonly _projection: GovernanceProjectionProvider;

  constructor(projection: GovernanceProjectionProvider) {
    this._projection = projection;
  }

  evaluateOutcome(value: unknown, _context: Context, path = 'arguments'): ConditionOutcome {
    // Structure first, so a malformed predicate is UNEVALUABLE whether or not
    // a projection happens to be available. §6.1.4's precheck normally reports
    // this before evaluation begins; the guard stays because
    // `_evaluateConditions` is reachable directly.
    if (describeArgumentsFaults(value, path).length > 0) return 'unevaluable';

    const projection = this._projection();
    if (projection === null) {
      // No projection was supplied for this evaluation — a bare `check()` from
      // tooling, or an ACL consulted outside the pipeline. The condition asks
      // about arguments nobody handed over, so no answer is obtainable and
      // §6.1.1's principle applies: a `deny` rule takes effect, an `allow` rule
      // does not grant. Reading it as an empty argument set would make
      // `has_none_of` GRANT for a call whose arguments were never seen.
      return 'unevaluable';
    }

    const present = new Set(projection.keys);
    for (const [predicate, names] of Object.entries(value as Record<string, string[]>)) {
      let passed: boolean;
      if (predicate === 'has_key') {
        passed = names.some((n) => present.has(n));
      } else if (predicate === 'has_all_keys') {
        passed = names.every((n) => present.has(n));
      } else {
        passed = !names.some((n) => present.has(n));
      }
      if (!passed) return 'unsatisfied';
    }
    return 'satisfied';
  }

  evaluate(value: unknown, context: Context): boolean {
    return this.evaluateOutcome(value, context) === 'satisfied';
  }
}

// ---------------------------------------------------------------------------
// Basic handlers
// ---------------------------------------------------------------------------

/** Check context.identity.type is in the allowed list. */
export class IdentityTypesHandler implements ACLConditionHandler {
  evaluate(value: unknown, context: Context): boolean {
    if (context.identity === null) return false;
    if (!Array.isArray(value)) return false;
    return value.includes(context.identity.type);
  }
}

/** Check at least one role overlaps between identity and required roles. */
export class RolesHandler implements ACLConditionHandler {
  evaluate(value: unknown, context: Context): boolean {
    if (context.identity === null) return false;
    if (!Array.isArray(value)) return false;
    const identityRoles = new Set(context.identity.roles);
    return (value as string[]).some((r: string) => identityRoles.has(r));
  }
}

/** Check call chain length does not exceed threshold. */
export class MaxCallDepthHandler implements ACLConditionHandler {
  evaluate(value: unknown, context: Context): boolean {
    let threshold: number;
    if (typeof value === 'number') {
      threshold = value;
    } else if (
      typeof value === 'object' &&
      value !== null &&
      'lte' in value &&
      typeof (value as { lte: unknown }).lte === 'number'
    ) {
      threshold = (value as { lte: number }).lte;
    } else {
      return false;
    }
    // Fail closed on non-integer thresholds (e.g. 5.5). A fractional depth is
    // meaningless and must not silently ALLOW where Python/Rust reject.
    if (!Number.isInteger(threshold)) {
      return false;
    }
    return context.callChain.length <= threshold;
  }
}

// ---------------------------------------------------------------------------
// Compound handlers
// ---------------------------------------------------------------------------

/**
 * `$or`: list of condition dicts. SATISFIED if ANY sub-set passes.
 *
 * Three-valued per PROTOCOL_SPEC §6.1.1: an outright "yes" wins even when a
 * sibling was unevaluable, so a SATISFIED child short-circuits. An UNEVALUABLE
 * child does NOT short-circuit — a later sibling may still be the decisive
 * SATISFIED — and makes the whole `$or` unevaluable only if no child said yes.
 */
export class OrHandler implements ACLOutcomeConditionHandler {
  private readonly _evaluate: EvalFn;

  constructor(evaluateFn: EvalFn) {
    this._evaluate = evaluateFn;
  }

  evaluateOutcome(value: unknown, context: Context, path = '$or'): ConditionOutcome {
    // §6.1.1 case 4: a value malformed for its key is UNEVALUABLE, not false.
    // A handler handed `$or: "typo"` can return false and look exactly like one
    // that answered "no" — which put a deny rule carrying the typo right back
    // into the inert state §6.1.1 exists to end.
    if (!Array.isArray(value)) return 'unevaluable';
    let sawUnevaluable = false;
    for (let i = 0; i < value.length; i++) {
      const sub = value[i];
      if (!isPlainObject(sub)) {
        sawUnevaluable = true;
        continue;
      }
      const outcome = this._evaluate(sub, context, `${path}[${i}]`);
      if (outcome === 'satisfied') return 'satisfied';
      if (outcome === 'unevaluable') sawUnevaluable = true;
    }
    return sawUnevaluable ? 'unevaluable' : 'unsatisfied';
  }

  evaluate(value: unknown, context: Context): boolean {
    return this.evaluateOutcome(value, context) === 'satisfied';
  }
}

/**
 * `$not`: single condition dict. SATISFIED if the sub-set is UNSATISFIED.
 *
 * §6.1.1: `$not` of an UNEVALUABLE child is UNEVALUABLE, never SATISFIED.
 * Negating "no answer" into "yes" would let a misspelled key inside a `$not`
 * satisfy the very rule it was meant to gate.
 */
export class NotHandler implements ACLOutcomeConditionHandler {
  private readonly _evaluate: EvalFn;

  constructor(evaluateFn: EvalFn) {
    this._evaluate = evaluateFn;
  }

  evaluateOutcome(value: unknown, context: Context, path = '$not'): ConditionOutcome {
    // §6.1.1 case 4 — a non-object `$not` cannot be negated into anything
    // meaningful, so it is UNEVALUABLE rather than false.
    if (!isPlainObject(value)) return 'unevaluable';
    const outcome = this._evaluate(value, context, path);
    if (outcome === 'unevaluable') return 'unevaluable';
    return outcome === 'satisfied' ? 'unsatisfied' : 'satisfied';
  }

  evaluate(value: unknown, context: Context): boolean {
    return this.evaluateOutcome(value, context) === 'satisfied';
  }
}

/** `$or` async: list of condition dicts evaluated via the async evaluator. */
export class OrHandlerAsync implements ACLOutcomeConditionHandler {
  private readonly _evaluate: AsyncEvalFn;

  constructor(evaluateFn: AsyncEvalFn) {
    this._evaluate = evaluateFn;
  }

  async evaluateOutcome(
    value: unknown,
    context: Context,
    path = '$or',
  ): Promise<ConditionOutcome> {
    if (!Array.isArray(value)) return 'unevaluable';
    let sawUnevaluable = false;
    for (let i = 0; i < value.length; i++) {
      const sub = value[i];
      if (!isPlainObject(sub)) {
        sawUnevaluable = true;
        continue;
      }
      const outcome = await this._evaluate(sub, context, `${path}[${i}]`);
      if (outcome === 'satisfied') return 'satisfied';
      if (outcome === 'unevaluable') sawUnevaluable = true;
    }
    return sawUnevaluable ? 'unevaluable' : 'unsatisfied';
  }

  async evaluate(value: unknown, context: Context): Promise<boolean> {
    return (await this.evaluateOutcome(value, context)) === 'satisfied';
  }
}

/** `$not` async: single condition dict evaluated via the async evaluator. */
export class NotHandlerAsync implements ACLOutcomeConditionHandler {
  private readonly _evaluate: AsyncEvalFn;

  constructor(evaluateFn: AsyncEvalFn) {
    this._evaluate = evaluateFn;
  }

  async evaluateOutcome(
    value: unknown,
    context: Context,
    path = '$not',
  ): Promise<ConditionOutcome> {
    if (!isPlainObject(value)) return 'unevaluable';
    const outcome = await this._evaluate(value, context, path);
    if (outcome === 'unevaluable') return 'unevaluable';
    return outcome === 'satisfied' ? 'unsatisfied' : 'satisfied';
  }

  async evaluate(value: unknown, context: Context): Promise<boolean> {
    return (await this.evaluateOutcome(value, context)) === 'satisfied';
  }
}

// ---------------------------------------------------------------------------
// Utility functions for element-wise comparison (used by removeRule fix)
// ---------------------------------------------------------------------------

/** Compare two arrays for element-wise equality. */
export function arraysEqual(a: unknown[], b: unknown[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

/** Deep equality for plain objects (conditions comparison). */
export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null) return a === b;
  if (typeof a !== typeof b) return false;
  if (typeof a !== 'object') return false;

  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
      if (!deepEqual(a[i], b[i])) return false;
    }
    return true;
  }

  const objA = a as Record<string, unknown>;
  const objB = b as Record<string, unknown>;
  const keysA = Object.keys(objA);
  const keysB = Object.keys(objB);
  if (keysA.length !== keysB.length) return false;
  for (const key of keysA) {
    if (!(key in objB)) return false;
    if (!deepEqual(objA[key], objB[key])) return false;
  }
  return true;
}
