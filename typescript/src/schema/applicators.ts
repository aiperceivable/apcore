/**
 * JSON Schema 2020-12 applicator keywords that TypeBox has no node for.
 *
 * TypeBox covers the §6 validation vocabulary and the §10.2 combinators, but
 * nothing in its type algebra expresses `prefixItems`, `patternProperties`,
 * `propertyNames`, `dependentRequired`, `dependentSchemas`, `if`/`then`/`else`
 * or the two `unevaluated*` keywords. The converter used to drop them, so a
 * contract that declared them was silently unenforced in TypeScript while
 * apcore-rust (jsonschema crate) and apcore-python (jsonschema library)
 * rejected the same input — the worst possible divergence for a cross-language
 * contract.
 *
 * The evaluator here closes that gap. It asserts only the applicator keywords;
 * every sub-schema it needs to test is handed back to the caller's converter
 * through {@link SubSchemaCheck}, so nested `type` / §6 / combinator keywords
 * keep going through TypeBox and there is exactly one validation engine.
 *
 * Browser-safe: no `node:*` imports (see tests/browser-entry.test.ts).
 */

/** Validate *value* against a raw JSON Schema object, via the caller's converter. */
export type SubSchemaCheck = (sub: Record<string, unknown>, value: unknown) => boolean;

/**
 * Presence of any of these makes a schema need the applicator evaluator.
 *
 * `then` / `else` are absent on purpose: §10.2.2.2/§10.2.2.3 make them inert
 * without `if`. `items` and `additionalProperties` are absent for the same
 * reason — they are enforced by TypeBox on their own and only get rerouted here
 * when `prefixItems` / `patternProperties` changes which instance locations they
 * apply to.
 */
export const APPLICATOR_KEYWORDS: readonly string[] = [
  'prefixItems',
  'patternProperties',
  'propertyNames',
  'dependentRequired',
  'dependentSchemas',
  'if',
  'unevaluatedItems',
  'unevaluatedProperties',
];

/**
 * True when the `contains` / `minContains` / `maxContains` trio must be
 * enforced here rather than by TypeBox.
 *
 * TypeBox's array check returns early when the match count is `0`, before it
 * consults `minContains` (`typebox/value/check`), so `minContains: 0` — which
 * JSON Schema 2020-12 §6.4.5 defines as making `contains` vacuously satisfiable
 * — would reject every array with no matching element. apcore-python and
 * apcore-rust both accept it. Only this case is rerouted; a schema with the
 * default or an explicit positive `minContains` stays with TypeBox.
 */
export function containsNeedsEvaluator(schema: Record<string, unknown>): boolean {
  return schema['minContains'] === 0 && isPlainObject(schema['contains']);
}

/** True when *schema* carries at least one keyword this module enforces. */
export function hasApplicatorKeyword(schema: Record<string, unknown>): boolean {
  return (
    APPLICATOR_KEYWORDS.some((keyword) => keyword in schema) || containsNeedsEvaluator(schema)
  );
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Compile a `patternProperties` key. JSON Schema 2020-12 §6.5.5 specifies
 * ECMA-262 regular expressions matched anywhere in the string, which is exactly
 * `new RegExp(pattern).test(key)`. An uncompilable pattern yields `null` and the
 * entry is skipped rather than throwing out of a validation call.
 */
function compilePattern(pattern: string): RegExp | null {
  try {
    return new RegExp(pattern);
  } catch {
    return null;
  }
}

/**
 * Check a value against a sub-schema in any of its three legal forms: the
 * boolean schemas `true` (always valid) and `false` (never valid), or an object.
 */
function checkSub(sub: unknown, value: unknown, check: SubSchemaCheck): boolean {
  if (sub === true) return true;
  if (sub === false) return false;
  if (!isPlainObject(sub)) return true;
  return check(sub, value);
}

/**
 * Assert every applicator keyword *schema* carries against *value*.
 *
 * Each keyword is inert on instances of the wrong type (§6, and §10.3 for the
 * applicators): `prefixItems` accepts every non-array, the object keywords
 * accept every non-object. That is what lets a type-less schema carrying one of
 * them be intersected unconditionally.
 */
export function checkApplicators(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  return (
    checkPrefixItems(schema, value, check) &&
    checkContains(schema, value, check) &&
    checkPatternProperties(schema, value, check) &&
    checkPropertyNames(schema, value, check) &&
    checkDependentRequired(schema, value) &&
    checkDependentSchemas(schema, value, check) &&
    checkConditional(schema, value, check) &&
    checkUnevaluatedItems(schema, value, check) &&
    checkUnevaluatedProperties(schema, value, check)
  );
}

/**
 * §10.3.1.1 `prefixItems` plus the §10.3.1.2 `items` it re-aims.
 *
 * In 2020-12 `items` applies only to the positions `prefixItems` did not cover,
 * so the two have to be enforced together — handing `items` to `Type.Array`
 * while `prefixItems` is present would apply the tail schema to the tuple head.
 */
function checkPrefixItems(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  const prefix = schema['prefixItems'];
  if (!Array.isArray(prefix) || !Array.isArray(value)) return true;
  const items = schema['items'];
  for (let index = 0; index < value.length; index += 1) {
    if (index < prefix.length) {
      if (!checkSub(prefix[index], value[index], check)) return false;
    } else if (items !== undefined && !checkSub(items, value[index], check)) {
      return false;
    }
  }
  return true;
}

/**
 * §6.4.4 `maxContains` / §6.4.5 `minContains` for the `minContains: 0` case
 * TypeBox cannot express.
 *
 * With `minContains: 0` the "at least one match" requirement disappears
 * entirely, so an array with zero matching elements is valid; `maxContains`
 * still caps the match count when present. Every other `minContains` value is
 * left to TypeBox (see {@link containsNeedsEvaluator}) so there is exactly one
 * engine asserting the keyword per schema.
 */
function checkContains(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  if (!containsNeedsEvaluator(schema) || !Array.isArray(value)) return true;
  const maxContains = schema['maxContains'];
  if (typeof maxContains !== 'number') return true;
  const contains = schema['contains'];
  let matches = 0;
  for (const element of value) {
    if (checkSub(contains, element, check)) matches += 1;
  }
  return matches <= maxContains;
}

/**
 * §10.3.2.2 `patternProperties`, plus the §10.3.2.3 `additionalProperties` whose
 * subject it narrows: a key matched by a pattern is no longer "additional", so
 * `additionalProperties` is enforced here rather than by `Type.Object` whenever
 * `patternProperties` is present.
 */
function checkPatternProperties(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  const patterns = schema['patternProperties'];
  if (!isPlainObject(patterns) || !isPlainObject(value)) return true;
  const declared = isPlainObject(schema['properties']) ? schema['properties'] : {};
  const additional = schema['additionalProperties'];

  for (const [key, member] of Object.entries(value)) {
    let matched = false;
    for (const [pattern, sub] of Object.entries(patterns)) {
      const regexp = compilePattern(pattern);
      if (regexp === null || !regexp.test(key)) continue;
      matched = true;
      if (!checkSub(sub, member, check)) return false;
    }
    if (matched || key in declared || additional === undefined) continue;
    if (!checkSub(additional, member, check)) return false;
  }
  return true;
}

/** §10.3.2.4 — every property *name* (a string) must satisfy the sub-schema. */
function checkPropertyNames(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  const names = schema['propertyNames'];
  if (names === undefined || !isPlainObject(value)) return true;
  return Object.keys(value).every((key) => checkSub(names, key, check));
}

/** §6.5.4 — presence of a trigger key requires its dependent keys. */
function checkDependentRequired(schema: Record<string, unknown>, value: unknown): boolean {
  const dependencies = schema['dependentRequired'];
  if (!isPlainObject(dependencies) || !isPlainObject(value)) return true;
  for (const [trigger, required] of Object.entries(dependencies)) {
    if (!(trigger in value) || !Array.isArray(required)) continue;
    if (!required.every((name) => typeof name === 'string' && name in value)) return false;
  }
  return true;
}

/** §10.2.2.4 — presence of a trigger key applies a whole sub-schema to the object. */
function checkDependentSchemas(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  const dependencies = schema['dependentSchemas'];
  if (!isPlainObject(dependencies) || !isPlainObject(value)) return true;
  for (const [trigger, sub] of Object.entries(dependencies)) {
    if (!(trigger in value)) continue;
    if (!checkSub(sub, value, check)) return false;
  }
  return true;
}

/**
 * §10.2.2.1–§10.2.2.3 — `if` never asserts on its own; its outcome selects
 * `then` or `else`, and a missing branch asserts nothing.
 */
function checkConditional(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  const condition = schema['if'];
  if (condition === undefined) return true;
  const branch = checkSub(condition, value, check) ? schema['then'] : schema['else'];
  if (branch === undefined) return true;
  return checkSub(branch, value, check);
}

/** §11.2 — every array position no applicator reached must satisfy the sub-schema. */
function checkUnevaluatedItems(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  const unevaluated = schema['unevaluatedItems'];
  if (unevaluated === undefined || !Array.isArray(value)) return true;
  const evaluated = evaluatedItems(schema, value, check);
  for (let index = 0; index < value.length; index += 1) {
    if (evaluated.has(index)) continue;
    if (!checkSub(unevaluated, value[index], check)) return false;
  }
  return true;
}

/** §11.3 — every property no applicator reached must satisfy the sub-schema. */
function checkUnevaluatedProperties(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): boolean {
  const unevaluated = schema['unevaluatedProperties'];
  if (unevaluated === undefined || !isPlainObject(value)) return true;
  const evaluated = evaluatedProperties(schema, value, check);
  for (const [key, member] of Object.entries(value)) {
    if (evaluated.has(key)) continue;
    if (!checkSub(unevaluated, member, check)) return false;
  }
  return true;
}

/**
 * The in-place applicators (§10.2) whose successful sub-schemas contribute
 * annotations to the parent. `if`/`then`/`else` and `dependentSchemas` are
 * handled separately because which of their sub-schemas apply depends on the
 * instance.
 */
const BRANCH_KEYWORDS: readonly string[] = ['allOf', 'anyOf', 'oneOf'];

/**
 * The sub-schemas of *schema* that annotate the same instance location: the
 * successful `allOf` / `anyOf` / `oneOf` branches, the `if`/`then`/`else` arm
 * the instance selected, and the triggered `dependentSchemas` entries.
 *
 * §11 counts an annotation only from a sub-schema that *succeeded*, which is why
 * each candidate is checked before being descended into. A failing branch of an
 * `anyOf` evaluates nothing, so its properties stay unevaluated.
 */
function annotatingSubSchemas(
  schema: Record<string, unknown>,
  value: unknown,
  check: SubSchemaCheck,
): Record<string, unknown>[] {
  const sources: Record<string, unknown>[] = [];

  for (const keyword of BRANCH_KEYWORDS) {
    const branches = schema[keyword];
    if (!Array.isArray(branches)) continue;
    for (const branch of branches) {
      if (isPlainObject(branch) && checkSub(branch, value, check)) sources.push(branch);
    }
  }

  const condition = schema['if'];
  if (condition !== undefined) {
    const passed = checkSub(condition, value, check);
    if (passed && isPlainObject(condition)) sources.push(condition);
    const branch = passed ? schema['then'] : schema['else'];
    if (isPlainObject(branch) && checkSub(branch, value, check)) sources.push(branch);
  }

  const dependencies = schema['dependentSchemas'];
  if (isPlainObject(dependencies) && isPlainObject(value)) {
    for (const [trigger, sub] of Object.entries(dependencies)) {
      if (!(trigger in value)) continue;
      if (isPlainObject(sub) && checkSub(sub, value, check)) sources.push(sub);
    }
  }

  return sources;
}

/** The property names *schema* evaluates on *value*, its sub-schemas included. */
function evaluatedProperties(
  schema: Record<string, unknown>,
  value: Record<string, unknown>,
  check: SubSchemaCheck,
): Set<string> {
  const evaluated = new Set<string>();
  const keys = Object.keys(value);

  const declared = schema['properties'];
  if (isPlainObject(declared)) {
    for (const key of keys) if (key in declared) evaluated.add(key);
  }

  const patterns = schema['patternProperties'];
  if (isPlainObject(patterns)) {
    for (const pattern of Object.keys(patterns)) {
      const regexp = compilePattern(pattern);
      if (regexp === null) continue;
      for (const key of keys) if (regexp.test(key)) evaluated.add(key);
    }
  }

  // `additionalProperties` applies to every key the two above did not claim, so
  // declaring it in any form leaves nothing unevaluated.
  if (schema['additionalProperties'] !== undefined) {
    for (const key of keys) evaluated.add(key);
  }

  for (const sub of annotatingSubSchemas(schema, value, check)) {
    for (const key of evaluatedProperties(sub, value, check)) evaluated.add(key);
  }
  return evaluated;
}

/** The array indexes *schema* evaluates on *value*, its sub-schemas included. */
function evaluatedItems(
  schema: Record<string, unknown>,
  value: readonly unknown[],
  check: SubSchemaCheck,
): Set<number> {
  const evaluated = new Set<number>();

  const prefix = schema['prefixItems'];
  const prefixLength = Array.isArray(prefix) ? prefix.length : 0;
  for (let index = 0; index < Math.min(prefixLength, value.length); index += 1) {
    evaluated.add(index);
  }

  // `items` covers every position after the prefix, so it leaves no tail.
  if (schema['items'] !== undefined) {
    for (let index = prefixLength; index < value.length; index += 1) evaluated.add(index);
  }

  // §10.3.1.3 — `contains` evaluates exactly the positions that matched it.
  const contains = schema['contains'];
  if (contains !== undefined) {
    for (let index = 0; index < value.length; index += 1) {
      if (checkSub(contains, value[index], check)) evaluated.add(index);
    }
  }

  for (const sub of annotatingSubSchemas(schema, value, check)) {
    for (const index of evaluatedItems(sub, value, check)) evaluated.add(index);
  }
  return evaluated;
}
