/**
 * ACL (Access Control List) types and implementation for apcore.
 */

import type { Context } from './context.js';
import { ACLRuleError } from './errors.js';
import { matchPattern } from './utils/pattern.js';
import type {
  ACLConditionHandler,
  ConditionOutcome,
  GovernanceProjection,
} from './acl-handlers.js';
import {
  ArgumentsHandler,
  IdentityTypesHandler,
  RolesHandler,
  MaxCallDepthHandler,
  OrHandler,
  NotHandler,
  OrHandlerAsync,
  NotHandlerAsync,
  arraysEqual,
  deepEqual,
  describeArgumentsFaults,
  isOutcomeHandler,
} from './acl-handlers.js';

/**
 * Reader for an ACL YAML file. Installed by the Node-only side-effect
 * module `./acl-file.ts`; remains `null` in browser bundles so that
 * `ACL.load()` throws a clear runtime error instead of silently
 * dragging `node:fs` into the browser closure.
 */
type AclFileLoader = (yamlPath: string, auditLogger?: AuditLogger | null) => ACL;
let _aclFileLoader: AclFileLoader | null = null;

/**
 * @internal — used by `./acl-file.ts` to install the Node-side reader.
 * Pass `null` to uninstall (used by tests that need to assert the
 * browser-side guidance error).
 */
export function _setAclFileLoader(fn: AclFileLoader | null): void {
  _aclFileLoader = fn;
}

/**
 * Minimal structural view of a `Config` used by `ACL.discover()`.
 *
 * Declared structurally (rather than importing the concrete `Config`
 * class) so `acl.ts` stays free of the `node:fs`-bearing `config.ts`
 * module and the browser closure remains clean. The real `Config`
 * satisfies this shape via its `get()` method and `sourcePath` getter.
 */
export interface AclConfigLike {
  get(key: string, defaultValue?: unknown): unknown;
  readonly sourcePath: string | null;
}

/**
 * Node-side filesystem discovery for `acl.root`. Installed by the Node-only
 * side-effect module `./acl-file.ts`; remains `null` in browser bundles so
 * that `ACL.discover()` throws a clear runtime error instead of pulling
 * `node:fs` / `node:path` into the browser closure.
 */
type AclDiscoverer = (config: AclConfigLike) => ACL | null;
let _aclDiscoverer: AclDiscoverer | null = null;

/**
 * @internal — used by `./acl-file.ts` to install the Node-side discoverer.
 * Pass `null` to uninstall (parity with `_setAclFileLoader`).
 */
export function _setAclDiscoverer(fn: AclDiscoverer | null): void {
  _aclDiscoverer = fn;
}

/**
 * @internal — true when the Node-side ACL discoverer has been installed.
 * Lets the `APCore` bootstrap skip `ACL.discover()` in browser bundles
 * (where the discoverer is never wired) instead of triggering its throw.
 */
export function _isAclDiscovererInstalled(): boolean {
  return _aclDiscoverer !== null;
}

/**
 * A rule's approval requirement (PROTOCOL_SPEC §6.1.6).
 *
 * Orthogonal to `effect`, which carries authorization. A rule answers two
 * independent questions — "may this caller reach this target at all?" and
 * "must *this particular call* be put to a human first?" — and folding them
 * into one enumeration would make the meaningless state "denied and needs
 * approval" representable while forcing the real one, "allowed but ask first",
 * to be spelled as a kind of denial.
 */
export type ACLApproval = 'required' | 'not_required';

export interface ACLRule {
  callers: string[];
  targets: string[];
  /**
   * Authorization — `'allow'` or `'deny'`, and nothing else (PROTOCOL_SPEC
   * §6.1, §6.1.5).
   *
   * The value set is **closed at every entry point that accepts a rule** — file
   * loading, direct construction and {@link ACL.addRule} — exactly as
   * `approval` is (§6.1.6 rule 3). Anything else is rejected with
   * {@link ACLRuleError} naming the rule index and the offending value; it is
   * never resolved to a decision, because reading an unrecognised value as
   * `deny` turns a rule the operator wrote to PERMIT into one that denies
   * everything it matches whenever `defaultEffect` is `'allow'`.
   *
   * Declared `string` rather than a `'allow' | 'deny'` union for parity with
   * apcore-python (`effect: str`) and apcore-rust (`pub effect: String`), and
   * because the value arrives from YAML with no type system in the loop: the
   * guarantee has to be the runtime check, not the declaration.
   */
  effect: string;
  description: string;
  conditions?: Record<string, unknown> | null;
  /**
   * Whether a call matching this rule must be put to a human before it runs
   * (PROTOCOL_SPEC §6.1.6). Absent means `'not_required'`, so every rule
   * written before v1.28.0 keeps its meaning exactly.
   *
   * `'required'` on an `effect: deny` rule is rejected with {@link ACLRuleError}
   * at every entry point that accepts rules. The combination has no meaning,
   * and silently ignoring one half of a governance rule is the failure mode
   * §6.1.5 was written to end.
   */
  approval?: ACLApproval;
}

/**
 * The structured result of an ACL check (PROTOCOL_SPEC §6.8.1).
 *
 * `check()` returns a boolean, which can carry authorization but not the
 * second axis of §6.1.6. This is the accessor that carries both.
 */
export interface AccessDecision {
  /** Authorization — unchanged semantics from today's boolean. */
  readonly access: 'allow' | 'deny';
  /** Whether **this call** must be put to a human before it runs (§6.1.6). */
  readonly approvalRequired: boolean;
  /** Index of the rule that decided, or `null` when none matched. */
  readonly matchedRuleIndex: number | null;
  /** Which branch of §6.3 produced the decision. */
  readonly reason: string;
}

/**
 * Per-call inputs to an ACL check beyond caller, target and `Context`.
 *
 * Optional throughout: an ACL consulted outside the pipeline supplies none,
 * and a condition that needs one it did not get is UNEVALUABLE under §6.1.1
 * rather than quietly answered against an empty stand-in.
 */
export interface AccessCheckOptions {
  /**
   * The §6.1.8 governance projection of the call's arguments — key set and
   * JSON types, never a value. Read by the built-in `arguments` condition
   * (§6.1.7). The pipeline computes it at Step 3 and passes it here at Step 4.
   */
  readonly arguments?: GovernanceProjection | null;
}

/** Structured record of an ACL check decision. */
export interface AuditEntry {
  readonly timestamp: string; // ISO 8601
  readonly callerId: string;
  readonly targetId: string;
  readonly decision: string; // "allow" or "deny"
  readonly reason: string; // "rule_match", "default_effect", "no_rules"
  readonly matchedRule: string | null; // Rule description
  readonly matchedRuleIndex: number | null;
  readonly identityType: string | null;
  readonly roles: readonly string[];
  readonly callDepth: number | null;
  readonly traceId: string | null;
  /** Error message from a condition handler that threw during evaluation, if any.
   *  Cross-language parity with apcore-python AuditEntry.handler_error (sync A-D-024). */
  readonly handlerError: string | null;
  /**
   * Whether the matched rule required this call to be put to a human
   * (PROTOCOL_SPEC §6.3.1, §6.1.6). `false` when no rule matched or the
   * matched rule required none.
   *
   * A field **beside** `decision` rather than a third `decision` value:
   * `decision` is a string downstream consumers parse, and widening it would
   * break every existing parser.
   */
  readonly approvalRequired: boolean;
}

export type AuditLogger = (entry: AuditEntry) => void;

export type { ConditionOutcome, GovernanceProjection } from './acl-handlers.js';

/**
 * One structural or registry fault reported by {@link ACL.validateRules}
 * (PROTOCOL_SPEC §6.1.2 rule 3, §6.1.3, §6.1.4).
 *
 * `syncResolvable` and `asyncResolvable` are reported separately and MUST NOT
 * be collapsed into one boolean: `asyncCheck()` consults the async registry
 * and falls back to the sync one, while `check()` consults only the sync
 * registry, so a key registered *only* as an async handler is a working
 * condition on one path and an unevaluable one on the other.
 */
export interface RuleValidationFinding {
  /** Index of the offending rule in definition order. */
  readonly ruleIndex: number;
  /**
   * Where the fault sits (§6.1.4): `roles`, `$or[1].mispelled`, `$` for a
   * `conditions` that is not a mapping, `callers` / `targets` for §6.1.4.1.
   * Findings order by this, not by key — a nested `$or` can carry one key at
   * several positions, which leaves key ordering undefined.
   */
  readonly conditionPath: string;
  /**
   * The condition key itself, for readers who do not need the path. `null`
   * where the fault has no key: a non-mapping `conditions`, a malformed
   * `callers` / `targets`, or a non-mapping `$or` element.
   */
  readonly conditionKey: string | null;
  /** The rule's effect — a finding on a `deny` rule is the consequential one. */
  readonly effect: string;
  /**
   * Whether the condition resolves for `check()`.
   *
   * Both flags mean **resolvable on that evaluation path**, not "present in
   * that registry" (§6.1.3 rule 2) — which is why they are `*Resolvable` and
   * not `*Registered`. A structural fault is resolvable on neither path, so
   * both are `false`; an async-only key is `false` / `true`.
   */
  readonly syncResolvable: boolean;
  /** Whether the condition resolves for `asyncCheck()` — the union of the two registries. */
  readonly asyncResolvable: boolean;
}

/**
 * The outcome of matching one rule against a call (PROTOCOL_SPEC §6.3).
 *
 * `'unevaluable'` is not a third kind of "no": it resolves toward refusing
 * access, so a `deny` rule carrying it MATCHES and denies while an `allow`
 * rule carrying it does not match and MUST NOT grant (§6.1.1).
 */
type RuleOutcome = 'match' | 'no_match' | 'unevaluable';

/**
 * Per-`check()` state private to one evaluation.
 *
 * `errors` is keyed by §6.1.4 **condition path** so the same path reported
 * twice in one evaluation collapses to one message and the set can be ordered
 * lexicographically on the way out — by path and not by key, because a nested
 * `$or` can carry one key at several positions, which leaves key ordering
 * undefined. `pending` records the paths added since the rule loop last drained
 * it, so the loop can name exactly the ones that made the rule it just
 * evaluated unevaluable (§6.1.1 rule 3 requires the warning to name the path,
 * the rule index and the effect).
 *
 * `projection` rides the same frame because it has exactly the same lifetime
 * and the same hazards: a nested `check()` from inside a condition handler, or
 * a concurrent `asyncCheck()` suspended mid-evaluation, must not read another
 * call's arguments.
 */
interface EvaluationFrame {
  /** §6.1.4 condition path → message. Keyed by path so §6.1.1 rule 2 can order by it. */
  readonly errors: Map<string, string>;
  /** Paths recorded since the rule loop last drained this, in discovery order. */
  pending: string[];
  /** §6.1.8 governance projection supplied for this evaluation, if any. */
  readonly projection: GovernanceProjection | null;
}

/**
 * JSONPath-style root token for the `conditions` object itself (§6.1.4). Used
 * when `conditions` is not a mapping at all, so no key exists to name.
 */
const CONDITIONS_ROOT_PATH = '$';

/** Which registry decides "resolvable" — §6.1.4 checks the path in use. */
type EvaluationPath = 'sync' | 'async';

/**
 * One structural or registry fault found by §6.1.4's precheck.
 *
 * The precheck is context-independent and runs no handler, so a fault is a
 * pure function of the rule — which is what lets every SDK report the same
 * set, in the same order, for the same rule.
 */
interface RuleFault {
  readonly path: string;
  readonly key: string | null;
  readonly message: string;
  readonly syncResolvable: boolean;
  readonly asyncResolvable: boolean;
}

/**
 * §6.1.3 — whether a condition key resolves on each evaluation path.
 *
 * Both flags mean **resolvable on that path**, not "present in that registry":
 * `asyncCheck()` falls back to the sync registry, so `asyncResolvable` is the
 * union of the two and every built-in leaf handler is resolvable on both.
 *
 * Installed by the `ACL` class body below, which owns the two private
 * registries; declaring it here keeps the precheck a plain function.
 */
let conditionResolvability: (key: string) => {
  syncResolvable: boolean;
  asyncResolvable: boolean;
} = () => ({ syncResolvable: false, asyncResolvable: false });

/** Order faults by §6.1.4 condition path (§6.1.1 rule 2, §6.1.2 rule 3). */
function byPath(a: RuleFault, b: RuleFault): number {
  return a.path < b.path ? -1 : a.path > b.path ? 1 : 0;
}

/** Join a parent path with a child key, per §6.1.4's path table. */
function childPath(prefix: string, key: string): string {
  return prefix === '' ? key : `${prefix}.${key}`;
}

/**
 * The two compound operators §6.2.1 defines for a pattern array.
 *
 * Reserved by **equality**, never by prefix or substring: `$orders.*` is an
 * ordinary pattern that merely begins with the same character, and an
 * implementation testing `p.startsWith('$')` would refuse a legal policy.
 */
const PATTERN_OPERATORS = new Set<string>(['$or', '$not']);

/**
 * §6.2.1 (v1.31.0, #112) — describe why a pattern array's shape is illegal, or
 * `null` when it is legal.
 *
 * A pattern array is **FLAT**: the operators do not nest, there is no
 * precedence, an operand is always a plain pattern string, and there is exactly
 * one operator position — index 0. `$or` / `$not` therefore have two different
 * grammars in this specification and only the one in `conditions` (§6.1.1)
 * nests. The set of legal shapes is closed:
 *
 *   1. the array MUST NOT be empty;
 *   2. every element MUST be a non-empty string;
 *   3. `$or` at index 0 MUST be followed by at least one pattern;
 *   4. `$not` at index 0 MUST be followed by exactly one pattern;
 *   5. `$or` / `$not` MUST NOT appear at any index other than 0.
 *
 * Everything outside that set can never match, so a rule carrying it is not a
 * narrow rule but no rule at all: through v1.30.0 all three SDKs returned
 * `false` from the matcher for `[]`, `['$or']` and `['$not']`, reading an arity
 * fault as a scope decision. On an `allow` rule that is merely useless; on a
 * `deny` rule under `defaultEffect: 'allow'` it is a **fail-open** — the call
 * the operator wrote the rule to block is permitted, by a rule that loaded
 * without error and a validator that called it clean. `['$not', p1, p2, …]` was
 * worse than inert: every SDK consulted `p1` and dropped the rest, so an
 * `allow` rule excluding two targets GRANTED the second one.
 *
 * The checks run most-basic-first — empty array, empty element, reserved token
 * out of position, operator arity — so the message names the fault an operator
 * has to fix first rather than a consequence of it.
 *
 * @param patterns - Already known to hold only strings. The element TYPE is
 *   §6.1.4.1's question and keeps precedence over this one, because an array
 *   whose element 0 is not a string has no meaningful arity reading.
 */
function describePatternArrayShape(patterns: readonly string[]): string | null {
  if (patterns.length === 0) {
    return (
      'the array is empty, so the rule can never match — write ["*"] if "everything" ' +
      'was meant, or delete the rule if "nothing" was'
    );
  }

  const emptyIndex = patterns.indexOf('');
  if (emptyIndex !== -1) {
    return `element ${emptyIndex} is the empty string, which matches no legal module ID`;
  }

  const strayIndex = patterns.findIndex((p, i) => i > 0 && PATTERN_OPERATORS.has(p));
  if (strayIndex !== -1) {
    return (
      `'${patterns[strayIndex]}' appears at index ${strayIndex}, and a pattern array is FLAT — ` +
      'the operators do not nest, there is no precedence, and index 0 is the only operator ' +
      "position. `['$or', '$not', 'a']` is not or-of-not and `['api.*', '$not', 'cli.*']` is " +
      'not "api.* but not cli.*"; no such form exists'
    );
  }

  const first = patterns[0];
  if (first === '$or' && patterns.length < 2) {
    return "'$or' at index 0 must be followed by at least one pattern — this is an OR over nothing";
  }
  if (first === '$not' && patterns.length !== 2) {
    const operands = patterns.length - 1;
    return operands === 0
      ? "'$not' at index 0 must be followed by exactly one pattern — this negates nothing"
      : `'$not' at index 0 takes exactly one operand and this carries ${operands}. Before ` +
          'v1.31.0 every SDK consulted the first and dropped the rest, so an allow rule ' +
          'GRANTED every operand after it — write two rules (§6.2.1, §6.3)';
  }
  return null;
}

/**
 * §6.1.4.1 — `callers` and `targets` MUST be lists of strings, and (v1.31.0,
 * #112) their **shape** must be one §6.2.1 defines.
 *
 * A bare string is iterable in several host languages, so `callers: "admin.*"`
 * written where `["admin.*"]` was meant is read character by character and its
 * `*` matches everything — an `allow` rule carrying that typo grants access to
 * every caller. TypeScript instead threw a `TypeError` out of `check()`, which
 * fails closed but violates `Contract: ACL.check`'s "MUST NOT raise to
 * indicate a deny". Either way the value must never be read as a pattern set:
 * it is a malformed rule, and §6.1.1's effect table decides what that means.
 *
 * The arity half is not disposed of the same way as the type half. `ACL.load`
 * deliberately permitted an empty `callers` / `targets` — only omission was
 * rejected — so a plain YAML file reached it, and a shape fault is now a
 * precheck fault on exactly the same terms as a type fault: the rule's scope is
 * unreadable, the rule is UNEVALUABLE, and §6.1.1's effect table decides. There
 * is no partially-readable tier: `targets: []` is legible as an empty scope in
 * a way `targets: 3` is not, and acting on that difference is the
 * per-implementation judgement call that produced three different answers in
 * #100. In particular §6.1.1 rule 5's "unknowable scope counts as scope"
 * applies unchanged, so a rule carrying `approval: 'required'` still raises the
 * pending requirement.
 */
function precheckPatternField(
  field: 'callers' | 'targets',
  value: unknown,
  out: RuleFault[],
): void {
  let message: string | null = null;
  if (!Array.isArray(value)) {
    message = `ACL rule field '${field}' must be a list of strings, got ${aclTypeName(value)}`;
  } else {
    const badIndex = value.findIndex((p) => typeof p !== 'string');
    if (badIndex !== -1) {
      message =
        `ACL rule field '${field}' must be a list of strings, got a list whose element ` +
        `${badIndex} is ${aclTypeName(value[badIndex])}`;
    } else {
      // §6.2.1 (v1.31.0, #112) — the SHAPE branch, and deliberately the
      // backstop rather than the primary mechanism: every entry point that
      // accepts a rule now rejects these outright, so the only route left to
      // here is a value assigned onto an already-constructed rule. `ACLRule`
      // is a plain interface with mutable properties, so `rule.targets = []`
      // bypasses every constructor — and unlike an unrecognised `effect`,
      // which is never read again once the doors are closed, a mutated
      // pattern array IS read: the matcher consults it on the next `check()`.
      //
      // Reached only after the element-type scan above, so a single field
      // yields at most one fault and §6.1.4.1's TYPE fault keeps precedence:
      // an array whose element 0 is not a string has no meaningful arity
      // reading.
      const shape = describePatternArrayShape(value as string[]);
      if (shape !== null) {
        message = `ACL rule field '${field}' has an illegal pattern-array shape: ${shape}`;
      }
    }
  }
  if (message === null) return;
  out.push({
    path: field,
    key: null,
    message,
    // A structural fault is resolvable on neither evaluation path.
    syncResolvable: false,
    asyncResolvable: false,
  });
}

/**
 * §6.2.1 tier 2 (v1.31.0, #112) — a pattern array that is well-formed under
 * every tier-1 clause and still matches **no legal module ID**, or `null` when
 * it can match something.
 *
 * Reported by {@link ACL.validateRules} and reachable from nowhere else. It is
 * deliberately NOT a {@link RuleFault} the precheck produces: such an array
 * **MUST NOT** be rejected, **MUST NOT** reach `handler_error`, and **MUST NOT**
 * change any access decision. A `deny` rule with `targets: ['$not', '*']` still
 * lets an unrelated target fall through to `defaultEffect` exactly as it did
 * before v1.31.0 — the finding is the whole of the change for it.
 *
 * The split from tier 1 is not tidiness. Tier 1's predicate is finite and
 * structural, so every SDK reports the same set and rejecting is safe. This one
 * reasons about the **match relation**, which §6.2's algorithm defines but does
 * not close — a future pattern feature changes which arrays are satisfiable. An
 * incomplete predicate is survivable in a validator and not at a door: a
 * rejection whose predicate differed between SDKs would mean the same ACL file
 * loads in one language and fails in another, which is the cross-language split
 * §6.1.5 exists to prevent. Divergence in this finding set is therefore
 * acceptable and expected, and §6.1.3's sentence governs: this is diagnostics,
 * not enforcement.
 *
 * The criterion is normative; the shapes below are a MUST-detect **minimum**
 * and not a closed set — enumerating where a principle belonged is the mistake
 * §6.1.1 corrected in v1.25.0.
 *
 * @param patterns - Already known to satisfy every tier-1 clause.
 */
function describeNeverMatchingPatternField(
  field: 'callers' | 'targets',
  patterns: readonly string[],
): string | null {
  if (patterns[0] === '$not') {
    // `!true` is false for every input, so the rule fires for nothing. The
    // criterion is "matches every module ID", not the single literal `*`.
    const operand = patterns[1];
    if (operand !== undefined && isUniversalPattern(operand)) {
      return (
        `'$not' negates '${operand}', which matches every module ID, so the rule ` +
        'fires for none. "Not everything" is well-formed and protects nothing'
      );
    }
    return null;
  }
  // Flat list and explicit `$or` are both OR-of-operands, so the array matches
  // nothing only when every operand does.
  const operands = patterns[0] === '$or' ? patterns.slice(1) : patterns;

  // PROTOCOL_SPEC §6.2.2 (v1.37.0): A08 has `*` as its ONLY metacharacter, and
  // §2.7 forbids `?` in a module ID, so a pattern carrying one matches nothing
  // and can never match anything. The author meant a wildcard — `?` IS one in
  // every other pattern surface the spec defines (A25, §9.2.3) — and got a
  // rule that is silently inert. Reported, never rejected: promoting `?` in
  // A08 would widen `allow` rules that are inert in every deployed policy
  // today, which is the one direction an authorization matcher must not move.
  const withQuestion = operands.filter((p) => p.includes('?'));
  if (operands.length > 0 && withQuestion.length === operands.length) {
    return (
      `every pattern here contains '?', which is a LITERAL in ACL matching ` +
      `(Algorithm A08 — '*' is the only metacharacter) and which §2.7 forbids in a module ID, ` +
      `so ${JSON.stringify(withQuestion)} matches nothing and can never match anything. ` +
      `'?' is a wildcard in every other pattern surface (A25, §9.2.3), which is where the ` +
      `expectation comes from. The rule loads and changes no decision — it simply never fires`
    );
  }

  if (operands.length > 0 && operands.every((p) => matchesNoModuleId(field, p))) {
    return (
      "every pattern is '@external' — the caller-side sentinel §6.5 substitutes for a null " +
      'caller_id. No module ID is `@external`, so as a TARGET pattern it matches nothing. It ' +
      'stays entirely legal in `callers`, which is what it is for'
    );
  }
  return null;
}

/**
 * True for a pattern that matches **every** module ID — `*`, `**`, or any
 * pattern consisting only of wildcards (§6.2's algorithm treats a run of `*`
 * exactly as one).
 */
function isUniversalPattern(pattern: string): boolean {
  return pattern.length > 0 && /^\*+$/.test(pattern);
}

/**
 * True for a pattern that can match no legal module ID **on this field**.
 *
 * Field-specific on purpose: `@external` is what a rule about top-level entry
 * points is written with in `callers`, and is unmatchable in `targets`. A check
 * that fired on both has read the rule as being about the token rather than
 * about the field.
 */
function matchesNoModuleId(field: 'callers' | 'targets', pattern: string): boolean {
  return field === 'targets' && pattern === '@external';
}

/**
 * §6.2.1 tier 2 — the validator-only findings for one rule, in field order.
 *
 * Skips a field that already carries a tier-1 or §6.1.4.1 fault: it is reported
 * once, by the mechanism that also decides, and "matches nothing" is not a
 * second opinion worth having about an array that cannot be read at all.
 */
function neverMatchingFaults(rule: ACLRule): RuleFault[] {
  const out: RuleFault[] = [];
  for (const field of ['callers', 'targets'] as const) {
    const value = rule[field];
    if (!Array.isArray(value)) continue;
    if (value.some((p) => typeof p !== 'string')) continue;
    const patterns = value as string[];
    if (describePatternArrayShape(patterns) !== null) continue;
    const detail = describeNeverMatchingPatternField(field, patterns);
    if (detail === null) continue;
    out.push({
      path: field,
      key: null,
      message: `ACL rule field '${field}' can match no module ID: ${detail}`,
      // §6.1.3 rule 3's keyless structural fault shape: a null key and both
      // resolvability flags false.
      syncResolvable: false,
      asyncResolvable: false,
    });
  }
  return out;
}

/**
 * §6.1.4 — walk a rule's whole `conditions` tree, every `$or` / `$not` branch
 * included, checking structure and the handler registries only.
 *
 * Supplies no context and invokes no handler, so it can run **before** §6.5's
 * "conditions present but no context provided" check — which is what closes
 * the bypass where `conditions: {mispelled: true}` on a `deny` rule passed
 * traffic simply because the caller carried no identity. It never
 * short-circuits: it has no decisive outcome to short-circuit on, and its
 * completeness is what makes §6.1.1 rule 2's deterministic `handler_error`
 * achievable.
 *
 * @param prefix - The §6.1.4 path of the object being walked (`''` at the root).
 */
function precheckConditions(
  conditions: unknown,
  prefix: string,
  mode: EvaluationPath,
  out: RuleFault[],
): void {
  if (!isConditionsObject(conditions)) {
    // §6.1.1 case 5 at the root; a non-mapping `$or` element deeper in.
    const path = prefix === '' ? CONDITIONS_ROOT_PATH : prefix;
    out.push({
      path,
      key: null,
      message: `ACL conditions '${path}' must be a mapping, got ${aclTypeName(conditions)}`,
      syncResolvable: false,
      asyncResolvable: false,
    });
    return;
  }

  for (const [key, value] of Object.entries(conditions)) {
    const path = childPath(prefix, key);

    if (key === '$or') {
      // §6.1.1 case 4 — a value malformed for its key.
      if (!Array.isArray(value)) {
        out.push({
          path,
          key,
          message: `ACL condition '${path}' must be a list of condition objects, got ${aclTypeName(value)}`,
          syncResolvable: false,
          asyncResolvable: false,
        });
        continue;
      }
      value.forEach((sub, i) => precheckConditions(sub, `${path}[${i}]`, mode, out));
      continue;
    }

    if (key === '$not') {
      if (!isConditionsObject(value)) {
        out.push({
          path,
          key,
          message: `ACL condition '${path}' must be a condition object, got ${aclTypeName(value)}`,
          syncResolvable: false,
          asyncResolvable: false,
        });
        continue;
      }
      precheckConditions(value, path, mode, out);
      continue;
    }

    if (key === 'arguments') {
      // §6.1.7's predicate vocabulary is closed and its values are structural,
      // so the whole condition is checkable here — context-free, handler-free
      // and therefore identical across SDKs. Whether a projection was supplied
      // is a per-call question and deliberately NOT asked here, exactly as
      // `roles` is not faulted for a caller who supplied no identity.
      // §6.1.8: the path descends to the offending predicate and EVERY faulty
      // predicate is reported; the KEY stays `arguments`, for a reader who
      // wants the condition rather than its position in the tree.
      for (const fault of describeArgumentsFaults(value, path)) {
        out.push({
          path: fault.path,
          key,
          message: fault.message,
          syncResolvable: false,
          asyncResolvable: false,
        });
      }
      continue;
    }

    // §6.1.1 case 1 / §6.1.3 — resolvable on the evaluation path in use.
    const { syncResolvable, asyncResolvable } = conditionResolvability(key);
    const resolvable = mode === 'sync' ? syncResolvable : asyncResolvable;
    if (resolvable) continue;
    out.push({
      path,
      key,
      message: `Unknown ACL condition '${path}'`,
      syncResolvable,
      asyncResolvable,
    });
  }
}

/**
 * §6.1.4 — the complete, ordered fault set for one rule, structural pattern
 * fields included. Ordered by path so every SDK reports the same sequence.
 */
function precheckRule(rule: ACLRule, mode: EvaluationPath): RuleFault[] {
  const faults: RuleFault[] = [];
  precheckPatternField('callers', rule.callers, faults);
  precheckPatternField('targets', rule.targets, faults);
  if (rule.conditions != null) precheckConditions(rule.conditions, '', mode, faults);
  faults.sort(byPath);
  return faults;
}

/** @internal — exported so `./acl-file.ts` can reuse the parser. */
/**
 * The complete set of keys an ACL rule may carry (PROTOCOL_SPEC §6.1).
 *
 * Closed on purpose: a key nothing evaluates is otherwise dropped in silence,
 * which widens an `allow` rule with no warning (#107).
 */
const RULE_KEYS = new Set([
  'callers',
  'targets',
  'effect',
  'description',
  'conditions',
  // §6.1.6 (v1.28.0). Adding it was only safe once v1.27.0 closed this set: an
  // SDK that still dropped unknown keys would read a `deny`-with-`approval`
  // rule as a bare rule and act on half of what its author wrote.
  'approval',
]);

/**
 * The two values §6.1 defines for a rule's `effect`.
 *
 * Closed for the reason {@link RULE_KEYS} is, one level down: there an unknown
 * KEY was dropped in silence, here a legal key's VALUE was (#111).
 */
const EFFECT_VALUES = new Set<string>(['allow', 'deny']);

/**
 * §6.1.5 (v1.30.0, #111) — reject an `effect` outside the closed value set.
 *
 * One function behind all three doors that accept a rule — file loading,
 * direct construction and runtime insertion — because §6.1.6 rule 3 requires
 * all three and two copies of a validation rule drift. This check used to live
 * inline in {@link _parseAclRule} and so guarded only the file path: `effect:
 * "Allow"`, the capitalisation an operator writes by hand, was rejected by
 * `ACL.load()` and accepted by `new ACL([...])` and `addRule()` — while
 * `default_effect`, the same two legal values one field up, was already
 * guarded at every door.
 *
 * Rejecting is the whole point: an unrecognised `effect` **MUST NOT** be
 * resolved to a decision. Reading it as `deny` looks safe and is not — under
 * `defaultEffect: 'allow'` a rule the operator wrote to PERMIT denies
 * everything it matches, with no error, no warning and nothing from
 * {@link ACL.validateRules} — and on a `deny` rule it is only accidentally
 * right, which lasts until someone revisits which way the fallback points.
 */
function rejectInvalidEffect(effect: unknown, index: number): void {
  if (typeof effect === 'string' && EFFECT_VALUES.has(effect)) return;
  throw new ACLRuleError(
    `Rule ${index} has invalid effect '${String(effect)}', must be 'allow' or 'deny'`,
  );
}

/**
 * §6.1.5 / §6.2.1 point 2 — `default_effect` is closed too, and it is judged
 * **first, before any rule, at every door** (v1.31.0, #112).
 *
 * It is not a rule and has no index, so §6.2.1's rule ordering never reaches
 * it — yet a file wrong in both `default_effect` and a rule is exactly the
 * one-file-one-error case that ordering exists for. Placing it ahead of every
 * rule is what stops `ACL.load()` naming rule 0 while `new ACL([...], 'Allow')`
 * names `default_effect` for the same content.
 *
 * @internal — exported for `./acl-file.ts`, which must run it *before* parsing
 *   any rule. The constructor runs it too, so the check exists once.
 */
export function _rejectInvalidDefaultEffect(defaultEffect: unknown): void {
  if (typeof defaultEffect === 'string' && EFFECT_VALUES.has(defaultEffect)) return;
  throw new ACLRuleError(
    `Invalid default_effect '${String(defaultEffect)}', must be 'allow' or 'deny'`,
  );
}

/**
 * §6.2.1 (v1.31.0, #112) — reject a `callers` / `targets` whose pattern-array
 * shape is outside the closed set {@link describePatternArrayShape} states.
 *
 * **Closing the doors is the mechanism**, exactly as it is for the `effect`
 * value set (§6.1.5) and the rule key set (§6.1.2): this runs at file loading,
 * at direct construction and at runtime insertion alike, on §6.1.6 rule 3's
 * reasoning. `schemas/acl-config.schema.json` has declared `minItems: 1` on
 * both fields since the file existed and `minLength: 1` on their items, and
 * nothing enforced either, because no implementation validates an ACL file
 * against the schema at load time — the third instance of that shape after
 * #107 and #111.
 *
 * A fail-stop is the right answer here because the affected population is, by
 * construction, deployments carrying a rule that provably does nothing. A
 * boot-time error naming `targets` is immediately actionable; "your deployment
 * silently began permitting the call it was written to block" is not.
 *
 * The field's **type** is deliberately not this function's business. A
 * `callers` that is not a list of strings cannot be produced by the loader
 * (which rejects a non-list) and reaches `check()` only through a cast, where
 * §6.1.4.1's precheck classifies it as unevaluable. Faulting it here as well
 * would change that long-standing classification for a shape this section is
 * not about.
 *
 * @param index - The rule's position, which every TypeScript entry point has:
 *   the loader and the constructor take an ordered list, and `addRule` inserts
 *   at the head. §6.1.5 forbids INVENTING a position, not naming a real one.
 */
function rejectMalformedPatternField(
  field: 'callers' | 'targets',
  value: unknown,
  index: number,
): void {
  if (!Array.isArray(value)) return;
  if (value.some((p) => typeof p !== 'string')) return;
  const detail = describePatternArrayShape(value as string[]);
  if (detail === null) return;
  throw new ACLRuleError(
    `Rule ${index} '${field}' has an illegal pattern-array shape: ${detail}. ` +
      'PROTOCOL_SPEC §6.2.1 closes the set of legal shapes — at least one operand, every ' +
      "element a non-empty string, '$or' at index 0 followed by at least one pattern, '$not' " +
      "by exactly one, and '$or' / '$not' nowhere but index 0 — because a shape that can " +
      'never match makes the rule inert, and an inert deny rule under ' +
      "default_effect: 'allow' permits the call it was written to block.",
  );
}

/**
 * §6.2.1 — both pattern fields, checked in the order a reader reads the rule.
 *
 * Applied to `callers` and `targets` **identically**: §6.2.1 constrains them the
 * same way, and an implementation validating one and inferring the other is the
 * specific defect the conformance fixture's `*_in_callers_is_rejected` mirrors
 * exist to catch.
 */
function rejectMalformedPatternFields(rule: ACLRule, index: number): void {
  rejectMalformedPatternField('callers', rule.callers, index);
  rejectMalformedPatternField('targets', rule.targets, index);
}

/** The two values §6.1.6 defines for a rule's `approval` field. */
const APPROVAL_VALUES = new Set<string>(['required', 'not_required']);

/**
 * §6.1.6 rule 2 — `approval: required` on a `deny` rule is rejected.
 *
 * "Denied **and** needs approval" is not a state that means anything. Loading
 * it and enforcing only the `deny` half would be acting on half of a
 * governance rule, which is the failure mode §6.1.5 was written to end.
 *
 * Applied at every entry point that accepts rules — file loading, direct
 * construction and runtime insertion — because a rule built in code is exactly
 * as meaningless as one parsed from YAML.
 *
 * Takes the two fields it reads rather than a whole {@link ACLRule}, so the
 * load path can run it in §6.2.1 point 2's order — after `effect`, before
 * `callers` / `targets` — while the rule object is still being assembled.
 */
function rejectDenyWithApproval(rule: Pick<ACLRule, 'effect' | 'approval'>, index: number): void {
  if (rule.approval !== 'required') return;
  if (rule.effect !== 'deny') return;
  throw new ACLRuleError(
    `Rule ${index} carries approval: 'required' on an effect: 'deny' rule. ` +
      'Authorization and approval are two independent results (PROTOCOL_SPEC §6.1.6) ' +
      'and "denied and needs approval" is not a state that means anything — a denied ' +
      'call never reaches the approval gate. Drop the approval field, or make the rule ' +
      "effect: 'allow' with approval: 'required' if the intent was \"allowed but ask first\".",
  );
}

/**
 * Reserved in earlier revisions of §6.1 and evaluated by no implementation.
 * Rejected like any other unknown key, but named as reserved in the message: an
 * operator who wrote `actions: ["describe"]` meant to restrict the rule and is
 * better served by "not implemented" than by "unknown key".
 */
const RESERVED_RULE_KEYS = new Set(['id', 'actions', 'priority']);

function rejectUnknownRuleKeys(index: number, ruleObj: Record<string, unknown>): void {
  const unknown = Object.keys(ruleObj)
    .filter((k) => !RULE_KEYS.has(k))
    .sort();
  if (unknown.length === 0) return;
  const reserved = unknown.filter((k) => RESERVED_RULE_KEYS.has(k));
  const other = unknown.filter((k) => !RESERVED_RULE_KEYS.has(k));
  const parts: string[] = [];
  if (reserved.length > 0) {
    parts.push(
      `${reserved.map((k) => `'${k}'`).join(', ')} reserved for a future ` +
        'specification version and evaluated by no implementation',
    );
  }
  if (other.length > 0) {
    parts.push(`${other.map((k) => `'${k}'`).join(', ')} unrecognised`);
  }
  throw new ACLRuleError(
    `Rule ${index} carries ${parts.join('; ')}. The rule key set is closed ` +
      `(${[...RULE_KEYS].sort().join(', ')}); a key nothing evaluates would be ` +
      'dropped silently and leave the rule wider than written.',
  );
}

export function _parseAclRule(rawRule: unknown, index: number): ACLRule {
  if (typeof rawRule !== 'object' || rawRule === null || Array.isArray(rawRule)) {
    throw new ACLRuleError(`Rule ${index} must be a mapping, got ${typeof rawRule}`);
  }

  const ruleObj = rawRule as Record<string, unknown>;
  for (const key of ['callers', 'targets', 'effect']) {
    if (!(key in ruleObj)) {
      throw new ACLRuleError(`Rule ${index} missing required key '${key}'`);
    }
  }

  // A missing key was already rejected above so an omission cannot render a
  // rule inert; an unknown key is the same hazard pointing the other way, and
  // was dropped in silence until #107.
  rejectUnknownRuleKeys(index, ruleObj);

  // §6.2.1 point 2 (v1.31.0) — the three axes are validated in ONE order at
  // every door: `effect`, then `approval`, then `callers` / `targets`. A rule
  // bad on more than one of them is refused for the first it fails, so the same
  // rule produces the same error in every implementation and in every SDK.
  // `effect` leads because `rejectDenyWithApproval` reads it to decide whether
  // the pair is the meaningless one.
  rejectInvalidEffect(ruleObj['effect'], index);
  const effect = ruleObj['effect'] as string;

  // §6.1.6: `approval` is optional and its absence means 'not_required', so
  // every rule written before v1.28.0 keeps its meaning exactly. A value
  // outside the two-member enumeration is rejected rather than coerced — a
  // governance field set by truthiness is a decision made by accident.
  const rawApproval = ruleObj['approval'];
  let approval: ACLApproval = 'not_required';
  if (rawApproval !== undefined && rawApproval !== null) {
    if (typeof rawApproval !== 'string' || !APPROVAL_VALUES.has(rawApproval)) {
      throw new ACLRuleError(
        `Rule ${index} has invalid approval '${String(rawApproval)}', ` +
          "must be 'required' or 'not_required'",
      );
    }
    approval = rawApproval as ACLApproval;
  }
  rejectDenyWithApproval({ effect, approval }, index);

  const callers = ruleObj['callers'];
  if (!Array.isArray(callers)) {
    throw new ACLRuleError(`Rule ${index} 'callers' must be a list, got ${typeof callers}`);
  }
  // §6.2.1 (v1.31.0) — the same check `new ACL([...])` and `addRule()` run, not
  // a second copy of it. Sits beside the type check because a YAML file reaches
  // BOTH: `ACL.load` rejects an OMITTED `callers` / `targets` and, until
  // v1.31.0, permitted an empty one.
  rejectMalformedPatternField('callers', callers, index);

  const targets = ruleObj['targets'];
  if (!Array.isArray(targets)) {
    throw new ACLRuleError(`Rule ${index} 'targets' must be a list, got ${typeof targets}`);
  }
  rejectMalformedPatternField('targets', targets, index);

  // `conditions` was previously taken with a bare `as` cast, which asserts a shape
  // TypeScript cannot check at runtime. A scalar therefore reached the gate, where
  // `Object.entries(5)` is `[]` and the AND-loop is vacuously satisfied — an
  // `effect: allow` rule became unconditional. Validate it here like every sibling key.
  const rawConditions = ruleObj['conditions'];
  if (rawConditions !== undefined && rawConditions !== null && !isConditionsObject(rawConditions)) {
    throw new ACLRuleError(
      `Rule ${index} 'conditions' must be a mapping, got ${aclTypeName(rawConditions)}`,
    );
  }

  return {
    callers: callers as string[],
    targets: targets as string[],
    effect,
    description: (ruleObj['description'] as string) ?? '',
    conditions: (rawConditions as Record<string, unknown>) ?? null,
    approval,
  };
}

/** True only for a plain object — excludes null, arrays and every primitive. */
function isConditionsObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** JSON-ish type name for diagnostics, mirroring executor.ts's `jsonTypeName`. */
function aclTypeName(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  return typeof value;
}

// ---------------------------------------------------------------------------
// Audit delivery (PROTOCOL_SPEC §6.3.2, apcore#118 decision D-66)
// ---------------------------------------------------------------------------

/** The three settings of an ACL file's `audit:` block. */
export const AUDIT_FIELDS = ['enabled', 'include_denied', 'log_level'] as const;

/** The stable name the default sink emits under (§10.3's event table). */
export const AUDIT_EVENT_NAME = 'apcore.acl.audit';

export interface AuditConfig {
  enabled: boolean;
  include_denied: boolean;
  log_level: 'trace' | 'debug' | 'info' | 'warn' | 'error';
}

const AUDIT_LEVEL_METHODS: Record<string, 'debug' | 'info' | 'warn' | 'error'> = {
  trace: 'debug',
  debug: 'debug',
  info: 'info',
  warn: 'warn',
  error: 'error',
};

/**
 * The §6.3.1 record under its **wire** names (§6.3.2 requirement 2).
 *
 * §6.3.1 lets an SDK surface use idiomatic field names — this one uses
 * `callerId` — and that licence stops at the default sink. Emitting the
 * in-memory object would put `callerId` on the wire here and `caller_id` in the
 * other two SDKs, which is exactly the "three different structured records from
 * one specification" the requirement exists to prevent.
 *
 * Written out rather than derived by a camel-to-snake transformer: the thirteen
 * names ARE the contract, and a transformer hides which ones exist behind a
 * regex that would also silently invent names for any field added later.
 */
export function auditEntryToWire(entry: AuditEntry): Record<string, unknown> {
  return {
    timestamp: entry.timestamp,
    caller_id: entry.callerId,
    target_id: entry.targetId,
    decision: entry.decision,
    reason: entry.reason,
    matched_rule: entry.matchedRule,
    matched_rule_index: entry.matchedRuleIndex,
    identity_type: entry.identityType,
    roles: entry.roles,
    call_depth: entry.callDepth,
    trace_id: entry.traceId,
    handler_error: entry.handlerError,
    approval_required: entry.approvalRequired,
  };
}

/**
 * The ONE effective sink for an ACL, per §6.3.2 requirement 1.
 *
 * Never two. A callback supplied to the constructor receives every entry and is
 * not narrowed, levelled or silenced by the `audit:` block: an API argument
 * beats configuration, and the alternative lets a file silently truncate a
 * compliance sink a developer installed deliberately. The block configures the
 * **default sink** and nothing else.
 *
 * A sink also owns its failure state. Requirement 5 suppresses diagnostics
 * after the first, scoped to one ACL instance **and one effective sink
 * configuration** — replacing the sink or reloading a configuration that
 * changes it builds a new `AuditSink`, so a new failure is never hidden behind
 * an old one.
 */
export class AuditSink {
  private _failed = false;
  private _rejectedAwaitable = false;

  constructor(
    private readonly _callback: AuditLogger | null,
    /**
     * `null` means the document declared no `audit:` block. Requirement 2:
     * DECLARATION activates the default sink, never the default value —
     * `enabled` defaults to true, so reading the merged view would switch a log
     * record per check on for every ACL file in existence.
     */
    private readonly _config: AuditConfig | null,
  ) {}

  get isActive(): boolean {
    if (this._callback !== null) return true;
    return this._config !== null && this._config.enabled;
  }

  /** Deliver one entry. NEVER throws (§6.3.2 requirement 3). */
  deliver(entry: AuditEntry): void {
    if (this._callback !== null) {
      this._deliverToCallback(entry);
      return;
    }
    if (this._config === null || !this._config.enabled) return;
    if (entry.decision === 'deny' && !this._config.include_denied) {
      // Requirement 6 — the load-time notice for this is emitted once, at load;
      // withholding here is silent by design.
      return;
    }
    this._emitDefault(entry);
  }

  private _deliverToCallback(entry: AuditEntry): void {
    let result: unknown;
    try {
      result = (this._callback as AuditLogger)(entry);
    } catch (e) {
      this._reportFailure(e);
      return;
    }
    if (typeof (result as { then?: unknown } | null)?.then === 'function') {
      // Requirement 4. An async callback's rejection surfaces after the
      // decision has been returned — outside the containment requirement 3
      // promises, and as an unhandled rejection. Treated as an invalid
      // delivery, reported once. The promise is caught so Node does not also
      // report it, which would be a second diagnostic for one cause.
      void (result as Promise<unknown>).catch(() => {});
      if (!this._rejectedAwaitable) {
        this._rejectedAwaitable = true;
        console.warn(
          `[apcore:acl] The ACL auditLogger returned a Promise. PROTOCOL_SPEC §6.3.2 ` +
            `requirement 4 requires a SYNCHRONOUS callback: an async one fails after the ` +
            `access decision has been returned, where it can no longer be contained. This ` +
            `entry was NOT delivered. Enqueue the entry inside the callback and return.`,
        );
      }
    }
  }

  /**
   * §6.3.2 requirement 2 — the default sink.
   *
   * All thirteen §6.3.1 fields go out as STRUCTURED data under their
   * `snake_case` wire names, not interpolated into the message. Without that,
   * one specification yields three different "structured records" across the
   * SDKs and nothing downstream consumes all three.
   */
  private _emitDefault(entry: AuditEntry): void {
    const method = AUDIT_LEVEL_METHODS[(this._config as AuditConfig).log_level] ?? 'info';
    try {
      console[method](AUDIT_EVENT_NAME, auditEntryToWire(entry));
    } catch (e) {
      this._reportFailure(e);
    }
  }

  private _reportFailure(e: unknown): void {
    if (this._failed) return;
    this._failed = true;
    const name = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
    console.warn(
      `[apcore:acl] ACL audit delivery failed (${name}). The access decision is ` +
        `unaffected (PROTOCOL_SPEC §6.3.2 requirement 3) — auditing is a side channel and ` +
        `does not hold a veto over access. Further failures from this sink are not ` +
        `reported; replacing the sink or reloading the ACL starts a new report.`,
    );
  }
}

/**
 * §6.3.2 requirement 1 — name EVERY field the callback overrides.
 *
 * Not only the most visible one: an operator who set `include_denied` and
 * `log_level` and hears about one of them has been told the smaller half of
 * what happened.
 */
export function warnAuditBlockOverridden(
  callback: AuditLogger | null,
  config: AuditConfig | null,
): void {
  if (callback === null || config === null) return;
  console.warn(
    `[apcore:acl] An auditLogger was supplied to this ACL, so it is the effective sink ` +
      `and the ACL file's 'audit:' block does not apply. The callback receives EVERY ` +
      `entry, allow and deny alike. These declared settings have no effect: ` +
      `${AUDIT_FIELDS.map((f) => `audit.${f}`).join(', ')} ` +
      `(PROTOCOL_SPEC §6.3.2 requirement 1).`,
  );
}


export class ACL {
  private static conditionHandlers = new Map<string, ACLConditionHandler>();
  private static asyncConditionHandlers = new Map<string, ACLConditionHandler>();

  static {
    // Give the module-level precheck read access to the two private
    // registries without widening the class's public surface.
    conditionResolvability = (key: string) => {
      const syncResolvable = ACL.conditionHandlers.has(key);
      return {
        syncResolvable,
        asyncResolvable: syncResolvable || ACL.asyncConditionHandlers.has(key),
      };
    };
  }

  static registerCondition(key: string, handler: ACLConditionHandler): void {
    ACL.conditionHandlers.set(key, handler);
  }

  /** Register an async-aware handler for use specifically under asyncCheck(). Falls back to conditionHandlers. */
  static registerAsyncCondition(key: string, handler: ACLConditionHandler): void {
    ACL.asyncConditionHandlers.set(key, handler);
  }

  /**
   * Per-`check()` / per-`asyncCheck()` capture slot for the handler-error
   * message produced by `_evaluateConditions[Async]`.
   *
   * `_currentFrame` names the frame belonging to the evaluation *currently*
   * on the stack. `_evaluateConditions[Async]` captures the reference at
   * entry and writes to that captured object, never back through the static
   * — so neither a nested `check()` (which pushes and pops its own frame)
   * nor a concurrent `asyncCheck()` suspended mid-evaluation can steal or
   * clobber another call's error. This is the JS equivalent of
   * apcore-python's `_handler_error_var` ContextVar token pair and
   * apcore-rust's depth-tracked thread-local / tokio task-local
   * (sync findings A-D-026, W2).
   */
  private static _currentFrame: EvaluationFrame | null = null;

  /** @internal — open a capture frame; returns [frame, previousFrame]. */
  private static _pushEvaluationFrame(
    projection: GovernanceProjection | null,
  ): [EvaluationFrame, EvaluationFrame | null] {
    const previous = ACL._currentFrame;
    const frame: EvaluationFrame = {
      errors: new Map<string, string>(),
      pending: [],
      projection,
    };
    ACL._currentFrame = frame;
    return [frame, previous];
  }

  /**
   * @internal — the §6.1.8 governance projection bound to the evaluation
   * currently in flight, or `null` when the caller supplied none.
   *
   * Read by the built-in `arguments` handler. It rides the evaluation frame
   * rather than a bare static so a nested `check()` from inside a condition
   * handler, or a concurrent `asyncCheck()` suspended mid-evaluation, cannot
   * read another call's arguments.
   */
  static _currentGovernanceProjection(): GovernanceProjection | null {
    return ACL._currentFrame?.projection ?? null;
  }

  /** @internal — restore the caller's capture frame. */
  private static _popEvaluationFrame(previous: EvaluationFrame | null): void {
    ACL._currentFrame = previous;
  }

  /**
   * @internal — read-and-clear this evaluation's captured handler errors.
   *
   * PROTOCOL_SPEC §6.1.1 rule 2: when more than one condition in a single
   * `check()` is unevaluable, `handler_error` MUST report every one it
   * determined, ordered **lexicographically by condition path** and separated
   * by `"; "`. By path rather than evaluation order because the two are not the
   * same across languages — `serde_json`'s map is ordered while a JavaScript
   * object preserves insertion order — and by path rather than key because a
   * key may occur at several positions in a nested `$or` / `$not` tree.
   */
  private static _takeFrameError(frame: EvaluationFrame): string | null {
    if (frame.errors.size === 0) return null;
    const message = [...frame.errors.keys()]
      .sort()
      .map((key) => frame.errors.get(key) as string)
      .join('; ');
    frame.errors.clear();
    frame.pending.length = 0;
    return message;
  }

  /**
   * @internal — record an unevaluable condition on the evaluation currently in
   * flight and warn about it. Keyed by §6.1.4 condition path so §6.1.1 rule 2's
   * lexicographic ordering has something well-defined to order.
   */
  private static _recordUnevaluable(
    frame: EvaluationFrame | null,
    path: string,
    message: string,
  ): void {
    if (frame !== null) {
      if (!frame.errors.has(path)) frame.errors.set(path, message);
      frame.pending.push(path);
    }
    console.warn(
      `[apcore:acl] ${message} — the condition is UNEVALUABLE, not false ` +
        '(PROTOCOL_SPEC §6.1.1: a deny rule takes effect, an allow rule does not grant)',
    );
  }

  /**
   * @internal — §6.3.1's if-and-only-if, for a handler that reports UNEVALUABLE
   * by *returning* it rather than by throwing.
   *
   * The three producers above all record as they go. An
   * {@link ACLOutcomeConditionHandler} can also just return `'unevaluable'` —
   * the built-in `arguments` condition does, when no governance projection was
   * supplied (§6.1.8 rule 1) — and nothing would then reach `handlerError`,
   * leaving an operator with a denial and no reason for it.
   *
   * `$or` and `$not` return `'unevaluable'` too, but only when *propagating* a
   * child's, and that child has already recorded the precise path; a generic
   * entry at the operator's own path would duplicate it while naming a less
   * useful location. `sizeBefore` tells the two apart: anything recorded during
   * the call means the subtree spoke for itself.
   */
  private static _recordBareUnevaluable(
    frame: EvaluationFrame | null,
    path: string,
    sizeBefore: number,
  ): void {
    if (frame !== null && frame.errors.size > sizeBefore) return;
    ACL._recordUnevaluable(frame, path, `Handler for condition '${path}' could not evaluate it as written`);
  }

  /**
   * Evaluate one condition key on the **sync** path.
   *
   * Three of the four UNEVALUABLE producers live here (§6.1.1): no registered
   * handler, a handler that threw, and a handler that returned a Promise —
   * which JavaScript cannot inspect synchronously, so no answer is obtainable
   * at all on this path. (The fourth is a `conditions` value that is not a
   * mapping, handled by the caller.)
   */
  private static _evaluateConditionSync(
    key: string,
    path: string,
    value: unknown,
    context: Context,
    frame: EvaluationFrame | null,
  ): ConditionOutcome {
    const handler = ACL.conditionHandlers.get(key);
    if (handler === undefined) {
      ACL._recordUnevaluable(frame, path, `Unknown ACL condition '${path}'`);
      return 'unevaluable';
    }
    const sizeBefore = frame?.errors.size ?? 0;
    try {
      const result = isOutcomeHandler(handler)
        ? handler.evaluateOutcome(value, context, path)
        : handler.evaluate(value, context);
      if (result instanceof Promise) {
        // apcore-typescript cannot inspect a Promise synchronously, so an
        // async handler reached from check() yields no answer at all.
        ACL._recordUnevaluable(
          frame,
          path,
          `Async condition '${path}' in sync context — use asyncCheck()`,
        );
        return 'unevaluable';
      }
      if (typeof result === 'boolean') return result ? 'satisfied' : 'unsatisfied';
      if (result === 'unevaluable') ACL._recordBareUnevaluable(frame, path, sizeBefore);
      return result;
    } catch (e) {
      ACL._recordUnevaluable(
        frame,
        path,
        `Handler for condition '${path}' threw: ${e instanceof Error ? e.message : String(e)}`,
      );
      return 'unevaluable';
    }
  }

  /**
   * Evaluate a `conditions` object on the sync path, AND-ing its keys with
   * PROTOCOL_SPEC §6.1.1's three-valued composition:
   *
   * - any child UNSATISFIED  → UNSATISFIED (an outright "no" wins)
   * - else any child UNEVALUABLE → UNEVALUABLE
   * - else → SATISFIED
   *
   * Short-circuits on the first UNSATISFIED child, which §6.1.1 permits: the
   * decision is identical either way and a child skipped that way was never
   * evaluated, so it is not unevaluable and MUST NOT set `handler_error`.
   * It deliberately does NOT short-circuit on UNEVALUABLE — a later sibling
   * may still produce the decisive UNSATISFIED, and every unevaluable
   * condition that was actually reached has to reach the audit entry.
   */
  static _evaluateConditions(
    conditions: Record<string, unknown>,
    context: Context,
    prefix = '',
  ): ConditionOutcome {
    // Bind the frame at entry so writes land on this evaluation's slot even if
    // another evaluation becomes current in the meantime.
    const frame = ACL._currentFrame;
    // Fail closed on a malformed `conditions`. §6.1.4's precheck normally
    // catches this before evaluation begins; the guard stays because a scalar
    // here would make `Object.entries()` return `[]`, satisfying the AND-loop
    // vacuously and turning an `allow` rule unconditional.
    if (!isConditionsObject(conditions)) {
      const path = prefix === '' ? CONDITIONS_ROOT_PATH : prefix;
      ACL._recordUnevaluable(
        frame,
        path,
        `ACL conditions '${path}' must be a mapping, got ${aclTypeName(conditions)}`,
      );
      return 'unevaluable';
    }
    let sawUnevaluable = false;
    for (const [key, value] of Object.entries(conditions)) {
      const outcome = ACL._evaluateConditionSync(
        key,
        childPath(prefix, key),
        value,
        context,
        frame,
      );
      if (outcome === 'unsatisfied') return 'unsatisfied';
      if (outcome === 'unevaluable') sawUnevaluable = true;
    }
    return sawUnevaluable ? 'unevaluable' : 'satisfied';
  }

  /**
   * Async twin of {@link ACL._evaluateConditions}, with the same three-valued
   * composition. Only two UNEVALUABLE producers apply here: no registered
   * handler on either registry, and a handler that threw — a Promise is simply
   * awaited on this path (§6.1.3).
   */
  static async _evaluateConditionsAsync(
    conditions: Record<string, unknown>,
    context: Context,
    prefix = '',
  ): Promise<ConditionOutcome> {
    // Bind the frame at entry — after an `await` the static may point at
    // another in-flight evaluation's frame.
    const frame = ACL._currentFrame;
    // Same fail-closed guard as the sync path — see _evaluateConditions.
    if (!isConditionsObject(conditions)) {
      const rootPath = prefix === '' ? CONDITIONS_ROOT_PATH : prefix;
      ACL._recordUnevaluable(
        frame,
        rootPath,
        `ACL conditions '${rootPath}' must be a mapping, got ${aclTypeName(conditions)}`,
      );
      return 'unevaluable';
    }
    let sawUnevaluable = false;
    for (const [key, value] of Object.entries(conditions)) {
      const path = childPath(prefix, key);
      // §6.1.3: the async registry is consulted first, then the sync one.
      const handler = ACL.asyncConditionHandlers.get(key) ?? ACL.conditionHandlers.get(key);
      if (handler === undefined) {
        ACL._recordUnevaluable(frame, path, `Unknown ACL condition '${path}'`);
        sawUnevaluable = true;
        continue;
      }
      let outcome: ConditionOutcome;
      const sizeBefore = frame?.errors.size ?? 0;
      try {
        const result = isOutcomeHandler(handler)
          ? await handler.evaluateOutcome(value, context, path)
          : await handler.evaluate(value, context);
        outcome = typeof result === 'boolean' ? (result ? 'satisfied' : 'unsatisfied') : result;
        if (outcome === 'unevaluable') ACL._recordBareUnevaluable(frame, path, sizeBefore);
      } catch (e) {
        ACL._recordUnevaluable(
          frame,
          path,
          `Handler for condition '${path}' threw: ${e instanceof Error ? e.message : String(e)}`,
        );
        outcome = 'unevaluable';
      }
      if (outcome === 'unsatisfied') return 'unsatisfied';
      if (outcome === 'unevaluable') sawUnevaluable = true;
    }
    return sawUnevaluable ? 'unevaluable' : 'satisfied';
  }

  private _rules: ACLRule[];
  private _defaultEffect: string;
  private _yamlPath: string | null = null;
  private _auditLogger: AuditLogger | null = null;
  private _auditConfig: AuditConfig | null = null;
  private _auditSink: AuditSink = new AuditSink(null, null);
  /** Rule indices already warned about for want of a context (§6.5). */
  private readonly _warnedMissingContext = new Set<number>();
  debug: boolean = false;

  constructor(
    rules: ACLRule[],
    defaultEffect: string = 'deny',
    auditLogger?: AuditLogger | null,
    /**
     * The ACL file's `audit:` block, as declared. `null`/omitted means the
     * document declared none, which keeps the pre-v1.45.0 behaviour of no audit
     * output without a callback (PROTOCOL_SPEC §6.3.2 requirement 2). Supplied
     * by the file loader; callers constructing an ACL directly normally pass
     * `auditLogger` instead.
     */
    auditConfig?: AuditConfig | null,
  ) {
    // §6.1.5 closes `default_effect` on the same terms as a rule's `effect`,
    // and §6.2.1 point 2 places it: judged FIRST, before any rule, at every
    // door. It is first here by position; `ACL.load()` runs the same function
    // before it parses a single rule, because reaching this constructor would
    // otherwise mean every rule had already been parsed and a file wrong in
    // both would name rule 0 through one door and `default_effect` through the
    // other (#111, #112).
    _rejectInvalidDefaultEffect(defaultEffect);
    this._rules = [...rules];
    this._defaultEffect = defaultEffect;
    this._auditLogger = auditLogger ?? null;
    this._auditConfig = auditConfig ?? null;
    // §6.3.2 requirement 1: one effective sink, never two. Built here and
    // rebuilt by `reload()`, which is what scopes requirement 5's
    // once-per-failure suppression to a sink CONFIGURATION rather than to the
    // ACL object's lifetime.
    this._auditSink = new AuditSink(this._auditLogger, this._auditConfig);
    warnAuditBlockOverridden(this._auditLogger, this._auditConfig);
    // §6.2.1 point 2 (v1.31.0, #112) — RULE INDEX DOMINATES THE AXES, so this
    // is one pass per rule and not three passes over the list. Sweeping an axis
    // across every rule before looking at the next axis is forbidden: it makes
    // a rule set with two bad rules report the higher-indexed one, and this
    // door then names a different fault than `ACL.load()` does for the same
    // file, which validates rule by rule. Within a rule the order is `effect`,
    // then `approval`, then the pattern fields — `effect` first because
    // `rejectDenyWithApproval` reads it to decide whether the pair is the
    // meaningless one, so `effect: 'Deny'` with `approval: 'required'` fails on
    // the effect rather than slipping past a `!== 'deny'` early return.
    //
    // §6.1.5 (v1.30.0) closes `effect` at THIS door, not only at `ACL.load()`;
    // §6.1.6 rule 2 is fatal rather than a warning, because unlike an
    // unregistered condition key (§6.1.2 rule 1, which must not break bootstrap
    // order) a `deny` + `approval: required` rule can never become meaningful
    // later; §6.2.1 closes the pattern-array shape.
    this._rules.forEach((rule, i) => {
      rejectInvalidEffect(rule.effect, i);
      rejectDenyWithApproval(rule, i);
      rejectMalformedPatternFields(rule, i);
    });
    // §6.1.2: direct construction is an entry point that accepts rules, so it
    // is covered by load-time validation too — `ACL.load()` reaches this same
    // constructor, which is why the file path needs no separate hook.
    this._warnRuleFaults();
  }

  /**
   * The effect applied when no rule matches (PROTOCOL_SPEC §6.8).
   *
   * A pure read: it emits no audit event, mutates nothing, and takes no lock.
   * Reflects a {@link ACL.reload}, because it reads the live object rather
   * than a cached parse.
   */
  get defaultEffect(): string {
    return this._defaultEffect;
  }

  /**
   * The current rule list in definition order — the order `check()` evaluates
   * it in (PROTOCOL_SPEC §6.8).
   *
   * Returns a frozen snapshot, never the ACL's own array: §6.8 rule 3 forbids
   * handing out a reference through which a caller could mutate the enforced
   * policy. Use {@link ACL.addRule} / {@link ACL.removeRule} to change it.
   */
  get rules(): readonly ACLRule[] {
    return Object.freeze(this._rules.slice());
  }

  /**
   * Report every rule that fails §6.1.4's precheck on the **sync** path
   * (PROTOCOL_SPEC §6.1.2 rule 3, §6.1.3, §6.1.4).
   *
   * Named `validateRules` and not `validateConditions` because it reports
   * structural faults in `callers` and `targets` as well (§6.1.4.1), not only
   * faults inside `conditions`.
   *
   * It is also the **only** reporter of §6.2.1's tier 2 (v1.31.0, #112): a
   * pattern array that is well-formed under every structural rule and still
   * matches no legal module ID — `['$not', '*']`, or `['@external']` as a
   * `targets` pattern. Such a rule protects nothing and is worth saying so
   * about, but it loads, it is never rejected, and it changes no decision.
   *
   * Condition handlers are registered at runtime into a process-wide registry
   * and `acl.root` discovery commonly runs during bootstrap, ahead of the
   * application code that registers them — so loading warns rather than
   * throwing, and this is the deterministic check to run once registration is
   * complete. A pure read: it mutates nothing and registers nothing.
   *
   * A finding is emitted whenever `syncResolvable` is false, **including**
   * when `asyncResolvable` is true: an application calling `check()` then has
   * a condition it cannot evaluate. A caller that only ever uses
   * `asyncCheck()` may ignore such a finding — that judgement belongs to the
   * caller, not to the validator.
   *
   * Findings are ordered by rule index, then lexicographically by **condition
   * path** — by path and not by key, because a nested `$or` may carry the same
   * key at several positions, which leaves key ordering undefined.
   */
  validateRules(): readonly RuleValidationFinding[] {
    const findings: RuleValidationFinding[] = [];
    const rules = this._rules.slice();
    for (let ruleIndex = 0; ruleIndex < rules.length; ruleIndex++) {
      const rule = rules[ruleIndex];
      // §6.2.1 tier 2 is reachable from HERE and from nowhere else. It must not
      // travel through `precheckRule`, which also feeds `handler_error` and the
      // decision: a well-formed array that happens to match nothing is a
      // diagnostic, never a denial. Re-sorted because the two sources are
      // concatenated; a field carrying a tier-1 fault yields no tier-2 one, so
      // the paths cannot collide.
      const faults = [...precheckRule(rule, 'sync'), ...neverMatchingFaults(rule)].sort(byPath);
      for (const fault of faults) {
        findings.push(
          Object.freeze({
            ruleIndex,
            conditionPath: fault.path,
            conditionKey: fault.key,
            effect: rule.effect,
            syncResolvable: fault.syncResolvable,
            asyncResolvable: fault.asyncResolvable,
          }),
        );
      }
    }
    return Object.freeze(findings);
  }

  /**
   * @internal — §6.1.2 rules 1-2 and 4: every entry point that accepts rules
   * warns, and none of them fails, for a rule that fails §6.1.4's precheck on
   * the sync path. The warning names the rule index, the condition path and
   * the rule's `effect`; the `effect` is in the message because a misconfigured
   * `deny` rule is the consequential case.
   *
   * @param onlyIndex - Validate a single rule index (used by `addRule`).
   */
  private _warnRuleFaults(onlyIndex?: number): void {
    const rules = this._rules;
    for (let i = 0; i < rules.length; i++) {
      if (onlyIndex !== undefined && i !== onlyIndex) continue;
      const rule = rules[i];
      // PROTOCOL_SPEC §6.2.2 clause 1 — the load-time half of the `?`
      // diagnostic, on the §6.1.2 precedent: warn, never fail.
      for (const field of ['callers', 'targets'] as const) {
        const value = rule[field];
        if (!Array.isArray(value)) continue;
        for (const pattern of value) {
          if (typeof pattern !== 'string' || !pattern.includes('?')) continue;
          console.warn(
            `[apcore:acl] Rule ${i} (effect=${rule.effect}) ${field} pattern ` +
              `${JSON.stringify(pattern)} contains '?', which is a LITERAL in ACL matching ` +
              `(Algorithm A08 — '*' is the only metacharacter) and which PROTOCOL_SPEC §2.7 ` +
              `forbids in a module ID. The pattern therefore matches nothing and can never ` +
              `match anything: a 'deny' rule guards nothing, an 'allow' rule never fires. ` +
              `'?' IS a wildcard in every other pattern surface (Algorithm A25, §9.2.3), ` +
              `which is where the expectation comes from. Nothing about this rule's meaning ` +
              `has changed — see §6.2.2.`,
          );
        }
      }
      for (const fault of precheckRule(rule, 'sync')) {
        const detail =
          fault.key !== null && !fault.syncResolvable && fault.asyncResolvable
            ? `${fault.message} — registered only as an async handler, so it resolves under asyncCheck() but is UNEVALUABLE under check()`
            : fault.message;
        console.warn(
          `[apcore:acl] Rule ${i} (effect=${rule.effect}): ${detail}. ` +
            'PROTOCOL_SPEC §6.1.1: an unevaluable condition makes a deny rule DENY and an ' +
            'allow rule not grant. Fix the rule or register a handler, and call ' +
            'acl.validateRules() once bootstrap is complete.',
        );
      }
    }
  }

  static load(yamlPath: string, auditLogger?: AuditLogger | null): ACL {
    if (_aclFileLoader === null) {
      throw new ACLRuleError(
        'ACL.load(yamlPath) requires the Node entry of apcore-js. The browser ' +
          'build does not bundle the file reader. Construct ACL programmatically ' +
          'with `new ACL([...rules])` instead.',
      );
    }
    return _aclFileLoader(yamlPath, auditLogger ?? null);
  }

  /**
   * Config-driven ACL discovery (D-64 / issue #74).
   *
   * Reads `acl.root` from `config` (defaulting to `"./acl"` when unset),
   * resolves it relative to the directory of the config's source file when
   * `config.sourcePath` is known — otherwise relative to the process CWD —
   * and loads the ACL from that directory if it exists.
   *
   * Returns `null` when the resolved path does NOT exist. A missing path
   * MUST NOT synthesize an empty default-deny ACL: an empty ACL would deny
   * every inter-module call in every project lacking an `acl/` directory, a
   * silent and severe break. Missing path = no enforcement, identical to a
   * project that never configured an ACL. `acl.default_effect` only takes
   * effect once an ACL is actually loaded from a file.
   *
   * Node-only: filesystem access lives on `./acl-file.ts`. In browser
   * bundles the discoverer is never installed and this throws a clear
   * runtime error (parity with {@link ACL.load}).
   */
  static discover(config: AclConfigLike): ACL | null {
    if (_aclDiscoverer === null) {
      throw new ACLRuleError(
        'ACL.discover(config) requires the Node entry of apcore-js. The browser ' +
          'build does not bundle the filesystem discoverer. Construct ACL ' +
          'programmatically with `new ACL([...rules])` instead.',
      );
    }
    return _aclDiscoverer(config);
  }

  /** @internal — used by `./acl-file.ts` to set `_yamlPath` after parsing. */
  _setYamlPath(yamlPath: string): void {
    this._yamlPath = yamlPath;
  }

  /**
   * Whether the call is authorized — the legacy boolean entry point.
   *
   * **It fails closed on an approval requirement** (PROTOCOL_SPEC §6.8.1): a
   * rule resolving to `allow` with `approvalRequired: true` makes this return
   * `false`. `check()` is public API consumed by callers that are not the
   * Executor — tooling, preflight helpers, third-party integrations — and such
   * a caller can only read a boolean as "let it through / do not". Returning
   * `true` would let it execute a call the ACL said needed a human. `false` is
   * wrong in the benign direction: the caller sees a refusal where the truth
   * was "ask first". Use {@link ACL.checkAccess} to see both axes.
   */
  check(
    callerId: string | null,
    targetId: string,
    context?: Context | null,
    options?: AccessCheckOptions | null,
  ): boolean {
    const decision = this.checkAccess(callerId, targetId, context, options);
    return decision.access === 'allow' && !decision.approvalRequired;
  }

  /**
   * The structured decision for a call (PROTOCOL_SPEC §6.8.1) — authorization
   * and approval requirement as the two independent results §6.1.6 defines.
   *
   * Emits exactly one audit entry, like {@link ACL.check}, which delegates here.
   */
  checkAccess(
    callerId: string | null,
    targetId: string,
    context?: Context | null,
    options?: AccessCheckOptions | null,
  ): AccessDecision {
    const effectiveCaller = callerId === null ? '@external' : callerId;
    const ctx = context ?? null;
    // Snapshot rules + defaultEffect + auditLogger atomically so concurrent
    // addRule/removeRule/setDefaultEffect calls cannot mutate state mid-evaluation.
    // Mirrors asyncCheck snapshot semantics for sync/async parity.
    const rules = this._rules.slice();
    const defaultEffect = this._defaultEffect;
    // The SINK, not the raw callback: delivery has to be contained (§6.3.2
    // requirement 3) and the failure state that scopes requirement 5's
    // suppression lives on the sink, so a bare callable could honour neither.
    const auditSink = this._auditSink;
    // Open a capture frame private to this evaluation. A nested check()
    // invoked from a condition handler pushes its own frame and restores
    // ours on exit, so it can no longer consume our handler error.
    const [frame, previousFrame] = ACL._pushEvaluationFrame(options?.arguments ?? null);
    // §6.1.1 rule 5 — an unevaluable `allow` rule's approval requirement is
    // PENDING, not discarded. A plain local rather than frame state: it is
    // per-evaluation by construction, so a nested check() cannot see it.
    let pendingApproval = false;

    try {
      for (let idx = 0; idx < rules.length; idx++) {
        const rule = rules[idx];
        frame.pending.length = 0;
        const outcome = this._matchesRule(rule, effectiveCaller, targetId, ctx, idx);
        if (outcome === 'unevaluable') {
          if (ACL._raisesPendingApproval(rule)) pendingApproval = true;
          if (!ACL._resolveUnevaluable(rule, idx, frame)) continue;
        } else if (outcome === 'no_match') {
          continue;
        }
        return this._decideByRule(
          effectiveCaller, targetId, rule, idx, ctx, auditSink, frame, pendingApproval,
        );
      }

      return this._decideByDefault(
        effectiveCaller, targetId, defaultEffect, rules.length, ctx, auditSink, frame,
        pendingApproval,
      );
    } finally {
      ACL._popEvaluationFrame(previousFrame);
    }
  }

  /** Async twin of {@link ACL.check}, with the same §6.8.1 fail-closed rule. */
  async asyncCheck(
    callerId: string | null,
    targetId: string,
    context?: Context | null,
    options?: AccessCheckOptions | null,
  ): Promise<boolean> {
    const decision = await this.asyncCheckAccess(callerId, targetId, context, options);
    return decision.access === 'allow' && !decision.approvalRequired;
  }

  /** Async twin of {@link ACL.checkAccess} (PROTOCOL_SPEC §6.8.1). */
  async asyncCheckAccess(
    callerId: string | null,
    targetId: string,
    context?: Context | null,
    options?: AccessCheckOptions | null,
  ): Promise<AccessDecision> {
    const effectiveCaller = callerId === null ? '@external' : callerId;
    const ctx = context ?? null;
    // Snapshot mutable fields before any await to prevent async-gap races
    // (e.g. a concurrent setDefaultEffect() or addRule() call mid-evaluation).
    const rules = this._rules.slice();
    const defaultEffect = this._defaultEffect;
    // The SINK, not the raw callback: delivery has to be contained (§6.3.2
    // requirement 3) and the failure state that scopes requirement 5's
    // suppression lives on the sink, so a bare callable could honour neither.
    const auditSink = this._auditSink;
    // Open a capture frame private to this evaluation (see checkAccess()).
    const [frame, previousFrame] = ACL._pushEvaluationFrame(options?.arguments ?? null);
    // §6.1.1 rule 5 — see checkAccess(). The two paths MUST NOT drift on this:
    // a requirement that survives an unevaluable rule on one path and is lost
    // on the other is the same fail-open, reachable by choosing an entry point.
    let pendingApproval = false;

    try {
      for (let idx = 0; idx < rules.length; idx++) {
        // Re-arm the current frame after every await gap: a concurrent
        // asyncCheck() may have become "current" while we were suspended.
        ACL._currentFrame = frame;
        const rule = rules[idx];
        frame.pending.length = 0;
        const outcome = await this._matchesRuleAsync(rule, effectiveCaller, targetId, ctx, idx);
        // Re-arm again: the await above may have yielded to another evaluation.
        ACL._currentFrame = frame;
        if (outcome === 'unevaluable') {
          if (ACL._raisesPendingApproval(rule)) pendingApproval = true;
          if (!ACL._resolveUnevaluable(rule, idx, frame)) continue;
        } else if (outcome === 'no_match') {
          continue;
        }
        return this._decideByRule(
          effectiveCaller, targetId, rule, idx, ctx, auditSink, frame, pendingApproval,
        );
      }

      return this._decideByDefault(
        effectiveCaller, targetId, defaultEffect, rules.length, ctx, auditSink, frame,
        pendingApproval,
      );
    } finally {
      ACL._popEvaluationFrame(previousFrame);
    }
  }

  /**
   * @internal — build the {@link AccessDecision} for a matched rule and emit
   * its audit entry. Shared by the sync and async paths so the two cannot
   * drift on §6.1.6's second axis.
   *
   * @param pendingApproval - §6.1.1 rule 5: a requirement raised by an earlier
   *   `allow` rule that was unevaluable and therefore did not grant. It
   *   composes by **disjunction** with this rule's own, so the requirement may
   *   originate in a rule that did not match (§6.9 row 1).
   */
  private _decideByRule(
    callerId: string,
    targetId: string,
    rule: ACLRule,
    ruleIndex: number,
    ctx: Context | null,
    auditSink: AuditSink,
    frame: EvaluationFrame,
    pendingApproval: boolean,
  ): AccessDecision {
    // §6.1.5 (v1.30.0): this ternary is TOTAL over the closed value set, not a
    // fallback — as of #111 an `effect` outside {allow, deny} cannot reach here
    // through any door that accepts a rule, which is what satisfies "an
    // implementation MUST NOT resolve an unrecognised effect to a decision".
    // It stays a ternary rather than becoming a throw because §6.1.1 makes
    // `check()` a call that MUST NOT raise; closing the doors is the mechanism,
    // and a rule object mutated in place after validation is the only residue.
    const access = rule.effect === 'allow' ? 'allow' : 'deny';
    // §6.1.6: the two results are orthogonal, and `approval: required` on a
    // `deny` rule is rejected at every entry point — the `access` guard is
    // belt-and-braces against a rule object mutated in place after load.
    //
    // §6.1.1 rule 5, third bullet: a denial CLEARS a pending requirement, and
    // `matchedRuleIndex` keeps naming the rule that actually decided rather
    // than the unevaluable one that raised it — hence the `access` guard
    // covering both terms rather than only the rule's own.
    const approvalRequired =
      access === 'allow' && (rule.approval === 'required' || pendingApproval);
    if (auditSink.isActive) {
      auditSink.deliver(
        this._buildAuditEntry(
          callerId, targetId, access, 'rule_match', rule, ruleIndex, ctx,
          ACL._takeFrameError(frame), approvalRequired,
        ),
      );
    }
    return Object.freeze({ access, approvalRequired, matchedRuleIndex: ruleIndex, reason: 'rule_match' });
  }

  /**
   * @internal — build the {@link AccessDecision} for the no-rule-matched path.
   *
   * §6.9 row 2: `default_effect` stays `allow` / `deny` only — there is no
   * default approval *source*. But `default_effect: allow` **MUST** carry a
   * pending requirement (§6.1.1 rule 5) through to the result: on the shape
   * where the gate is the only rule, there is no later rule to carry it, so
   * the requirement would be lost with nothing left holding it. That yields
   * `approvalRequired: true` with `matchedRuleIndex: null`, which §6.1.1
   * rule 5 makes a legal combination as of v1.29.0. Absent a pending
   * requirement, no match still means `false`.
   */
  private _decideByDefault(
    callerId: string,
    targetId: string,
    defaultEffect: string,
    ruleCount: number,
    ctx: Context | null,
    auditSink: AuditSink,
    frame: EvaluationFrame,
    pendingApproval: boolean,
  ): AccessDecision {
    // Total over the closed set for the same reason as in `_decideByRule`:
    // §6.1.5 closes `default_effect` on the same terms, and the constructor —
    // which `ACL.load()` also reaches — has always rejected anything else.
    const access = defaultEffect === 'allow' ? 'allow' : 'deny';
    const reason = ruleCount === 0 ? 'no_rules' : 'default_effect';
    // A `deny` default clears the pending requirement exactly as a matched
    // `deny` rule does (§6.1.1 rule 5, third bullet).
    const approvalRequired = access === 'allow' && pendingApproval;
    if (auditSink.isActive) {
      auditSink.deliver(
        this._buildAuditEntry(
          callerId, targetId, access, reason, null, null, ctx,
          ACL._takeFrameError(frame), approvalRequired,
        ),
      );
    }
    return Object.freeze({ access, approvalRequired, matchedRuleIndex: null, reason });
  }

  /**
   * Async twin of {@link ACL._matchPatterns} — a separate code path, so §6.2.1
   * has to be honoured in both.
   *
   * The arity guards below are **defence in depth and nothing more** as of
   * v1.31.0 (#112): every entry point rejects an illegal shape and
   * {@link precheckPatternField} classifies whatever arrives around them as
   * unevaluable *before* the rule reaches this matcher, so no illegal array can
   * get here. They are kept rather than deleted because reading an arity fault
   * as a non-match — which is exactly what they used to do — is the fail-open
   * this change exists to end, and a `false` return is the safe residue if some
   * future path did reach them.
   */
  private _matchPatternsAsync(patterns: string[], value: string, context: Context | null): boolean {
    if (patterns.length === 0) return false;

    // Check for compound operators
    const first = patterns[0];
    if (first === '$or') {
      for (const p of patterns.slice(1)) {
        if (this._matchPattern(p, value, context)) return true;
      }
      return false;
    }
    if (first === '$not') {
      if (patterns.length < 2) return false;
      return !this._matchPattern(patterns[1], value, context);
    }

    // Standard OR behavior for flat list
    return patterns.some((p) => this._matchPattern(p, value, context));
  }

  private async _matchesRuleAsync(
    rule: ACLRule,
    caller: string,
    target: string,
    context: Context | null,
    ruleIndex: number,
  ): Promise<RuleOutcome> {
    const frame = ACL._currentFrame;

    const structural = ACL._precheckPatternFields(rule);
    if (structural.length > 0) {
      ACL._recordFaults(structural, frame);
      return 'unevaluable';
    }

    if (!this._matchPatternsAsync(rule.callers, caller, context)) return 'no_match';
    if (!this._matchPatternsAsync(rule.targets, target, context)) return 'no_match';

    if (rule.conditions != null) {
      // §6.1.4, async path: "resolvable" is decided against the async registry
      // with its fallback to the sync one (§6.1.3), so an async-only key is a
      // live condition here and a precheck fault under check().
      const faults: RuleFault[] = [];
      precheckConditions(rule.conditions, '', 'async', faults);
      if (faults.length > 0) {
        ACL._recordFaults(faults.sort(byPath), frame);
        return 'unevaluable';
      }
      if (context === null) {
        this._warnConditionalRuleWithoutContext(rule, ruleIndex);
        return 'no_match';
      }
      const outcome = await ACL._evaluateConditionsAsync(rule.conditions, context);
      if (outcome === 'unsatisfied') return 'no_match';
      if (outcome === 'unevaluable') return 'unevaluable';
    }

    return 'match';
  }

  private _buildAuditEntry(
    callerId: string,
    targetId: string,
    decision: string,
    reason: string,
    matchedRule: ACLRule | null,
    matchedRuleIndex: number | null,
    context: Context | null,
    handlerError: string | null = null,
    approvalRequired = false,
  ): AuditEntry {
    let identityType: string | null = null;
    let roles: readonly string[] = [];
    let callDepth: number | null = null;
    let traceId: string | null = null;

    if (context !== null) {
      traceId = context.traceId;
      callDepth = context.callChain.length;
      if (context.identity !== null) {
        identityType = context.identity.type;
        roles = context.identity.roles;
      }
    }

    return {
      timestamp: new Date().toISOString(),
      callerId,
      targetId,
      decision,
      reason,
      matchedRule: matchedRule?.description ?? null,
      matchedRuleIndex,
      identityType,
      roles,
      callDepth,
      traceId,
      handlerError,
      approvalRequired,
    };
  }

  private _matchPattern(pattern: string, value: string, context: Context | null): boolean {
    if (pattern === '@external') return value === '@external';
    if (pattern === '@system') {
      return context !== null && context.identity !== null && context.identity.type === 'system';
    }
    return matchPattern(pattern, value);
  }

  /**
   * §6.2.1 — match a pattern array against one value.
   *
   * The array is FLAT: index 0 may carry `$or` or `$not` and every later
   * element is a plain pattern, which is why this reads one operator and never
   * recurses. As in {@link ACL._matchPatternsAsync}, the arity guards are
   * defence in depth only — an illegal shape is rejected at every door and
   * classified unevaluable by the precheck before it can reach here (#112).
   */
  private _matchPatterns(patterns: string[], value: string, context: Context | null): boolean {
    if (patterns.length === 0) return false;

    // Check for compound operators
    const first = patterns[0];
    if (first === '$or') {
      return patterns.slice(1).some((p) => this._matchPattern(p, value, context));
    }
    if (first === '$not') {
      if (patterns.length < 2) return false;
      return !this._matchPattern(patterns[1], value, context);
    }

    // Standard OR behavior for flat list
    return patterns.some((p) => this._matchPattern(p, value, context));
  }

  /**
   * @internal — §6.1.4.1: `callers` / `targets` are checked for structure
   * before they are read as pattern sets, so a malformed one is unevaluable
   * rather than a `TypeError` out of `check()` (or, in a language where a bare
   * string is iterable, a wildcard that grants everything).
   *
   * Both fields are checked even when the first is already malformed, and
   * before the patterns are matched at all: the precheck does not
   * short-circuit, and a rule whose fields cannot be read is not a rule that
   * can be said to miss.
   */
  private static _precheckPatternFields(rule: ACLRule): RuleFault[] {
    const faults: RuleFault[] = [];
    precheckPatternField('callers', rule.callers, faults);
    precheckPatternField('targets', rule.targets, faults);
    return faults;
  }

  /** @internal — record a precheck's faults on the in-flight evaluation. */
  private static _recordFaults(faults: RuleFault[], frame: EvaluationFrame | null): void {
    for (const fault of faults) ACL._recordUnevaluable(frame, fault.path, fault.message);
  }

  private _matchesRule(
    rule: ACLRule,
    caller: string,
    target: string,
    context: Context | null,
    ruleIndex: number,
  ): RuleOutcome {
    const frame = ACL._currentFrame;

    const structural = ACL._precheckPatternFields(rule);
    if (structural.length > 0) {
      ACL._recordFaults(structural, frame);
      return 'unevaluable';
    }

    if (!this._matchPatterns(rule.callers, caller, context)) return 'no_match';
    if (!this._matchPatterns(rule.targets, target, context)) return 'no_match';

    if (rule.conditions != null) {
      // §6.1.4: the structural and registry precheck is context-independent,
      // runs no handler, and MUST run BEFORE §6.5's no-context check — that
      // ordering is what closes the bypass where `conditions: {mispelled: true}`
      // on a deny rule passed traffic simply because the caller carried no
      // identity. It also cannot short-circuit, so every configuration fault in
      // the tree reaches handler_error whatever a sibling would have done.
      const faults: RuleFault[] = [];
      precheckConditions(rule.conditions, '', 'sync', faults);
      if (faults.length > 0) {
        ACL._recordFaults(faults.sort(byPath), frame);
        return 'unevaluable';
      }
      if (context === null) {
        // §6.1.4 rule 2: a rule that PASSES the precheck and then finds no
        // context takes §6.5's path. `roles` is answerable in principle; this
        // caller merely supplied no input for it.
        this._warnConditionalRuleWithoutContext(rule, ruleIndex);
        return 'no_match';
      }
      const outcome = ACL._evaluateConditions(rule.conditions, context);
      if (outcome === 'unsatisfied') return 'no_match';
      if (outcome === 'unevaluable') return 'unevaluable';
    }

    return 'match';
  }

  /**
   * @internal — §6.5: "conditions present but no context provided" is a plain
   * non-match, NOT an unevaluable condition. Calling with no context is a
   * legitimate shape for an external entry point, not a misconfiguration, and
   * treating it as an evaluation failure would flip the decision for every
   * `@external` call meeting a conditional `deny` rule.
   *
   * The consequence still needs saying out loud: a conditional `deny` rule is
   * therefore not a backstop for context-less callers. §6.5 asks for a warning
   * the first time such a rule is skipped, naming the rule index and effect.
   */
  private _warnConditionalRuleWithoutContext(rule: ACLRule, ruleIndex: number): void {
    if (this._warnedMissingContext.has(ruleIndex)) return;
    this._warnedMissingContext.add(ruleIndex);
    console.warn(
      `[apcore:acl] Rule ${ruleIndex} (effect=${rule.effect}) carries conditions but the call ` +
        'supplied no Context, so the rule does not match (PROTOCOL_SPEC §6.5). A conditional ' +
        'deny rule is not a backstop for context-less callers — express a backstop as an ' +
        'unconditional deny rule or as default_effect: deny.',
    );
  }

  /**
   * @internal — §6.1.1 rule 5 (v1.29.0, apcore#109): whether an UNEVALUABLE
   * rule leaves a **pending approval requirement** behind when it steps aside.
   *
   * Rule 1's "MUST NOT grant" was a complete instruction while a rule carried
   * one axis. Since §6.1.6 it carries two, and losing the second one is a
   * silent fail-open: a narrow `approval: required` gate ahead of a broad
   * `allow` used to step aside and let the broad rule grant `git push --force`
   * with `approvalRequired: false` — the exact call the operator gated.
   *
   * Only ever called with a rule the caller already classified `'unevaluable'`,
   * and that classification is what supplies §6.1.1 rule 5's **scope**
   * requirement without a second pattern match here:
   *
   * - A rule whose well-formed `callers` / `targets` do not match this call
   *   returns `'no_match'` from {@link ACL._matchesRule} before its conditions
   *   are read at all (§6.1.4 rule 4c), so it never reaches this and raises
   *   nothing. A rule written about one target MUST NOT attach a human to
   *   calls it was never written about.
   * - A rule that is unevaluable because its own pattern field is *malformed*
   *   (§6.1.4.1) DOES reach this, and MUST raise. Its scope cannot be read, so
   *   it cannot be shown not to apply here — the same posture that field
   *   already produces under `deny`, where an unreadable scope denies every
   *   call.
   *
   * The `effect === 'allow'` guard is belt-and-braces: `approval: required` on
   * a `deny` rule is rejected at every entry point (§6.1.6 rule 2), and an
   * unevaluable `deny` rule takes effect anyway, where a denial would clear
   * the requirement regardless.
   */
  private static _raisesPendingApproval(rule: ACLRule): boolean {
    return rule.effect === 'allow' && rule.approval === 'required';
  }

  /**
   * @internal — apply §6.1.1's effect rule to a rule whose conditions could not
   * be evaluated, warn as rule 3 requires, and say whether the rule takes
   * effect.
   *
   * | effect  | condition unevaluable                     |
   * |---------|-------------------------------------------|
   * | `allow` | does not match → continue (MUST NOT grant)|
   * | `deny`  | rule MUST take effect → the call is denied |
   *
   * The `allow` row is only half the story since v1.29.0: the rule steps
   * aside, but any `approval: required` it carried stays **pending** and is
   * composed into whatever grants next ({@link ACL._raisesPendingApproval}).
   * The warning says so, because the previous wording — "does not match and
   * MUST NOT grant" — was logged on the very call the next rule then granted.
   *
   * @returns true when the rule matches and its effect stands.
   */
  private static _resolveUnevaluable(
    rule: ACLRule,
    ruleIndex: number,
    frame: EvaluationFrame,
  ): boolean {
    const takesEffect = rule.effect === 'deny';
    const keys = [...new Set(frame.pending)];
    const named = keys.length > 0 ? keys.map((k) => `'${k}'`).join(', ') : '(see handler_error)';
    console.warn(
      `[apcore:acl] Rule ${ruleIndex} (effect=${rule.effect}) has unevaluable condition(s) ` +
        `${named}. PROTOCOL_SPEC §6.1.1 resolves this toward refusing access: ` +
        (takesEffect
          ? 'the deny rule takes effect and the call is DENIED.'
          : 'the allow rule does not match and MUST NOT grant.') +
        // §6.1.1 rule 5: naming the survivor here is what tells an operator
        // that the gate they wrote is still in force after its rule stepped
        // aside — and, when it is absent, that it is not.
        (ACL._raisesPendingApproval(rule)
          ? ' Its approval: required is PENDING (§6.1.1 rule 5) and composes into whatever grants next.'
          : ''),
    );
    frame.pending.length = 0;
    return takesEffect;
  }

  /**
   * Insert a rule at the head of the list (highest priority).
   *
   * §6.1.2 rule 4 makes runtime insertion an entry point that must be covered
   * by load-time validation, so the new rule meets exactly the checks
   * construction applies — and the split between them is the same here as
   * there. An unresolvable condition key **warns**: handlers are registered at
   * runtime and a key can still become resolvable later, so failing would break
   * bootstrap order. A malformed rule **throws** {@link ACLRuleError}, because
   * nothing later can make it meaningful — an `effect` outside `allow` / `deny`
   * (§6.1.5), a `callers` / `targets` whose pattern-array shape is outside
   * §6.2.1's closed set, or `approval: 'required'` on a `deny` rule (§6.1.6
   * rule 2). A throw leaves the rule list untouched.
   */
  addRule(rule: ACLRule): void {
    // §6.1.5 and §6.1.6 rule 2 — both rejected before insertion, so neither a
    // rule outside the closed `effect` value set nor a meaningless
    // `deny` + `approval` pair ever enters the list (§6.1.2 rule 4: runtime
    // insertion is an entry point too). `addRule` returns nothing by its own
    // contract, which §6.1.6 rule 3 says is not an exemption — throwing is how
    // TypeScript signals an unconstructable value, the same way the constructor
    // does, so there is no fallible twin to add beside it.
    //
    // §6.2.1 point 2 (v1.31.0) fixes the order of the three axes at every door
    // — `effect`, then `approval`, then `callers` / `targets`.
    rejectInvalidEffect(rule.effect, 0);
    rejectDenyWithApproval(rule, 0);
    // §6.2.1 (v1.31.0, #112). Index 0 is where `unshift` puts it, so the
    // message names a real position rather than an invented one.
    //
    // This runs on the rule as it is HANDED TO US, whatever its history: a rule
    // that was well-formed when constructed and has since had `callers` or
    // `targets` assigned is re-validated here and rejected. §6.2.1 point 1
    // requires exactly that and forbids relying on a construction-time check to
    // cover this door — unlike a closed `effect`, which is never read again, a
    // mutated pattern array IS read by the matcher on the next `check()`.
    rejectMalformedPatternFields(rule, 0);
    this._rules.unshift(rule);
    // Rule indices shifted by one; drop the per-index dedupe so a §6.5 warning
    // is not suppressed for a different rule that inherited an old index.
    this._warnedMissingContext.clear();
    this._warnRuleFaults(0);
  }

  removeRule(callers: string[], targets: string[], conditions?: Record<string, unknown> | null): boolean {
    for (let i = 0; i < this._rules.length; i++) {
      const rule = this._rules[i];
      if (!arraysEqual(rule.callers, callers) || !arraysEqual(rule.targets, targets)) continue;
      // Treat an explicit null the same as an omitted/undefined argument:
      // both mean "ignore conditions when matching". Only a concrete
      // conditions object disambiguates by deep equality. Mirrors
      // apcore-python acl.py:631 (`conditions is not None`) and Rust
      // (sync finding A-D-016).
      if (conditions != null && !deepEqual(rule.conditions ?? null, conditions)) continue;
      this._rules.splice(i, 1);
      // D-88: removing rule `i` shifts every rule after it UP by one, so a
      // marker recorded for the old occupant of an index silences the rule that
      // inherits it. The decision generalises to "any operation that inserts,
      // removes or reorders rules"; this SDK was its authority and had cleared
      // in `addRule` and `reload` only, which is the pair the status quo
      // described. Measured before the fix: two conditional rules, warn for
      // index 0, remove index 0, and the surviving rule's §6.5 warning was
      // suppressed by the removed rule's marker.
      this._warnedMissingContext.clear();
      return true;
    }
    return false;
  }

  reload(): void {
    if (this._yamlPath === null) {
      throw new ACLRuleError('Cannot reload: ACL was not loaded from a YAML file');
    }
    // The callback is NOT passed here: `reload` preserves the one this
    // instance already has (requirement 7), and handing it to the temporary
    // would emit requirement 1's override notice a second time for a
    // configuration the caller has already been told about.
    const reloaded = ACL.load(this._yamlPath);
    this._rules = reloaded._rules;
    this._defaultEffect = reloaded._defaultEffect;
    // §6.3.2 requirement 7: a reload refreshes the `audit:` block and PRESERVES
    // a programmatic callback, which was never read from the file. Before
    // v1.45.0 this method refreshed only the rules and the default effect, so
    // the audit block was the one part of the document a reload did not pick up.
    //
    // Building a NEW sink is also what scopes requirement 5's once-per-failure
    // suppression: a reload that changes the sink's configuration starts a
    // fresh report rather than hiding a new failure behind an old one.
    this._auditConfig = reloaded._auditConfig;
    this._auditSink = new AuditSink(this._auditLogger, this._auditConfig);
    warnAuditBlockOverridden(this._auditLogger, this._auditConfig);
    // §6.8 rule 4: `rules` and `defaultEffect` read the live object, so both
    // accessors reflect the reloaded file with no further work.
    this._warnedMissingContext.clear();
  }
}

// ---------------------------------------------------------------------------
// Auto-register built-in handlers at module load time
// ---------------------------------------------------------------------------

// Spec PROTOCOL_SPEC.md §6.1 defines only the plural forms
// (`identity_types`, `roles`, `max_call_depth`). Singular aliases
// were removed to align with Python (apcore-python commit 2c204fb)
// and Rust (apcore-rust src/acl_handlers.rs).
ACL.registerCondition('identity_types', new IdentityTypesHandler());
ACL.registerCondition('roles', new RolesHandler());
ACL.registerCondition('max_call_depth', new MaxCallDepthHandler());
// §6.1.7 — `arguments` is BUILT IN and there is no registration point for it.
// It is registered here, with the other built-ins, precisely so that §6.1.4's
// precheck covers it for free: `argument:` written for `arguments:` is then an
// unregistered condition key, so the rule is unevaluable rather than silently
// inert. It reads the §6.1.8 projection bound to the evaluation in flight.
ACL.registerCondition('arguments', new ArgumentsHandler(ACL._currentGovernanceProjection));
ACL.registerCondition('$or', new OrHandler(ACL._evaluateConditions.bind(ACL)));
ACL.registerCondition('$not', new NotHandler(ACL._evaluateConditions.bind(ACL)));
// Async-aware variants used by asyncCheck() so Promise-returning conditions
// inside $or/$not are awaited rather than dropped via fail-closed.
ACL.registerAsyncCondition('$or', new OrHandlerAsync(ACL._evaluateConditionsAsync.bind(ACL)));
ACL.registerAsyncCondition('$not', new NotHandlerAsync(ACL._evaluateConditionsAsync.bind(ACL)));
