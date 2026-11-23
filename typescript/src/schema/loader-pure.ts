/**
 * Browser-safe portion of the schema loader.
 *
 * Contains the runtime-neutral pieces — JSON-Schema-to-TypeBox conversion and
 * canonical-form async hashing — used by both the Node-side `SchemaLoader` and
 * the browser entry point.
 *
 * No `node:*` imports may be added here. The transitive import graph is
 * audited by `tests/browser-entry.test.ts`.
 */

import { Kind, Type, TypeRegistry, type TSchema } from '@sinclair/typebox';
import { Value } from '@sinclair/typebox/value';
import { checkApplicators, containsNeedsEvaluator, hasApplicatorKeyword } from './applicators.js';
import { KEYWORD_MARKER } from './constants.js';

// ---------------------------------------------------------------------------
// Canonical-form serialization (used for content hashing)
// ---------------------------------------------------------------------------

export function sortedKeysStringify(obj: unknown): string {
  if (obj === null || typeof obj !== 'object') return JSON.stringify(obj);
  if (Array.isArray(obj)) return `[${obj.map(sortedKeysStringify).join(',')}]`;
  const sorted = Object.keys(obj as object).sort();
  const pairs = sorted.map(
    (k) => `${JSON.stringify(k)}:${sortedKeysStringify((obj as Record<string, unknown>)[k])}`,
  );
  return `{${pairs.join(',')}}`;
}

/**
 * Compute the SHA-256 hex digest of the canonical JSON serialization of a
 * schema using the WebCrypto SubtleCrypto API. Output matches `contentHash`
 * (Node-only sync variant) and the Python/Rust SDKs (sync finding A-D-033).
 */
export async function contentHashAsync(schema: unknown): Promise<string> {
  const canonical = sortedKeysStringify(schema);
  type WebCryptoSubtle = {
    digest(algo: string, data: ArrayBuffer | ArrayBufferView): Promise<ArrayBuffer>;
  };
  const subtle: WebCryptoSubtle | undefined =
    typeof globalThis !== 'undefined'
      ? (globalThis as { crypto?: { subtle?: WebCryptoSubtle } }).crypto?.subtle
      : undefined;
  if (!subtle) {
    throw new Error(
      'contentHashAsync(): no WebCrypto SubtleCrypto available in this runtime.',
    );
  }
  const data = new TextEncoder().encode(canonical);
  const digest = await subtle.digest('SHA-256', data);
  const bytes = new Uint8Array(digest);
  let hex = '';
  for (const b of bytes) {
    hex += b.toString(16).padStart(2, '0');
  }
  return hex;
}

// ---------------------------------------------------------------------------
// JSON Schema → TypeBox conversion
// ---------------------------------------------------------------------------

/**
 * Convert a JSON Schema dict to a TypeBox TSchema.
 *
 * Supports recursive schemas through the lazy `$ref` nodes `RefResolver` leaves
 * in place (`#`, `#/$defs/…`, the root `$id`), plus oneOf/anyOf/allOf/not.
 *
 * `root` is the document `#`-anchored references resolve against; it defaults to
 * `schema` itself and is threaded through every nested conversion so a `$ref`
 * buried in `properties` still finds `$defs` at the top of the document.
 */
export function jsonSchemaToTypeBox(
  schema: Record<string, unknown>,
  root?: Record<string, unknown>,
): TSchema {
  return _convert(schema, root ?? schema);
}

function _convert(schema: Record<string, unknown>, root: Record<string, unknown>): TSchema {
  if ('$ref' in schema) {
    return _convertRef(schema['$ref'] as string, schema, root);
  }

  const rawType = schema['type'];
  const schemaType = rawType as string | undefined;

  // A `type` array converts each branch through `_convert`, so every branch
  // already carries the applicator keywords; adding them once more on the union
  // node would only duplicate the work.
  const isTypeUnion = Array.isArray(rawType);

  let result: TSchema;
  if (isTypeUnion) result = _convertTypeUnion(schema, rawType, root);
  else if (schemaType === 'object') result = _convertObject(schema, root);
  else if (schemaType === 'array') result = _convertArray(schema, root);
  else if (schemaType === 'string') result = _convertString(schema);
  else if (schemaType === 'integer') result = _convertNumeric(schema, Type.Integer);
  else if (schemaType === 'number') result = _convertNumeric(schema, Type.Number);
  else if (schemaType === 'boolean') result = Type.Boolean();
  else if (schemaType === 'null') result = Type.Null();
  else {
    // No `type` at all — the combinator and bare constraint keywords are the
    // whole schema.
    return _annotate(_withApplicators(_convertTypeless(schema, root), schema, root), schema);
  }

  // `type` and a combinator keyword are independent assertions that must BOTH
  // hold (JSON Schema 2020-12 §10.2). Converting only the `type` half silently
  // dropped `enum` / `const` / `anyOf` / `allOf` / `not`, so
  // `{type: ["string","boolean"], enum: ["always","auto","never"]}` accepted any
  // string at all — apcore-rust and apcore-python both reject a non-member.
  result = _intersectWithCombinator(result, schema, root);

  return _annotate(isTypeUnion ? result : _withApplicators(result, schema, root), schema);
}

/**
 * Copy the annotation keywords — and the `x-` extension keywords — onto the
 * converted node.
 *
 * The extensions carry behaviour the executor reads off the CONVERTED schema:
 * `resolveSchema` replaces a module's JSON Schema with this node, and the
 * capture point redacts against it. Dropping `x-sensitive` here made every
 * field a plain-JSON-Schema module marked sensitive reach
 * `context.redactedInputs` / `redactedOutput` — and so every built-in logging
 * middleware — in clear text (PROTOCOL_SPEC 10.6, 10.6.1 requirement 5, D-131).
 * An `x-` keyword is not a validation assertion, so the validator ignores it.
 */
function _annotate(result: TSchema, schema: Record<string, unknown>): TSchema {
  const node = result as Record<string, unknown>;
  if (typeof schema['description'] === 'string') node['description'] = schema['description'];
  if (typeof schema['title'] === 'string') node['title'] = schema['title'];
  for (const [key, value] of Object.entries(schema)) {
    if (key.startsWith('x-')) node[key] = value;
  }
  return result;
}

/** Assertion keywords that constrain a value independently of its `type`. */
const COMBINATOR_KEYWORDS: readonly string[] = [
  'enum',
  'const',
  'oneOf',
  'anyOf',
  'allOf',
  'not',
];

/**
 * Intersect a type-derived schema with the constraint its combinator siblings
 * express, so both halves are enforced. Returns `typed` unchanged when the
 * schema carries no combinator keyword.
 */
function _intersectWithCombinator(
  typed: TSchema,
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema {
  if (!COMBINATOR_KEYWORDS.some((keyword) => keyword in schema)) return typed;
  return Type.Intersect([typed, _convertCombinator(schema, root)]);
}

/**
 * A `type` array (`["string", "boolean"]`, `["string", "null"]`) means "any one
 * of these types", so it converts to a union of the same schema narrowed to each
 * member. Converting it to `unknown` would accept values of every other type as
 * well — apcore-rust rejects them, and apexe emits `["string", "boolean"]` for
 * value-optional flags.
 *
 * The branches carry only the type-specific option keywords; `description` /
 * `title` belong on the union node and the combinator keywords are intersected
 * with the whole union by the caller, so both are stripped here.
 */
function _convertTypeUnion(
  schema: Record<string, unknown>,
  types: unknown[],
  root: Record<string, unknown>,
): TSchema {
  const branchSchema = _typeBranchSchema(schema);
  const branches = types
    .filter((t): t is string => typeof t === 'string')
    .map((t) => _convert({ ...branchSchema, type: t }, root));
  return branches.length > 0 ? Type.Union(branches) : Type.Unknown();
}

/**
 * Strip the keywords that belong on the enclosing node rather than on a
 * per-type branch: `description` / `title` are annotations of the whole schema,
 * and the combinator keywords are intersected with the whole node by the caller,
 * so leaving them in would enforce them once per branch.
 */
function _typeBranchSchema(schema: Record<string, unknown>): Record<string, unknown> {
  const branchSchema: Record<string, unknown> = { ...schema };
  for (const keyword of ['description', 'title', ...COMBINATOR_KEYWORDS]) {
    delete branchSchema[keyword];
  }
  return branchSchema;
}

/**
 * The JSON Schema 2020-12 §6 validation keywords TypeBox enforces, one table per
 * type. A keyword absent from its table is dropped by the conversion, so every
 * addition belongs here rather than inline in a converter — a `type` array
 * reaches these through the per-branch conversion in {@link _convertTypeUnion}.
 *
 * Deliberately absent: `required` (§6.5.3) is expressed through
 * `Type.Optional`, and the keywords TypeBox has no node for —
 * `dependentRequired` (§6.5.4), `propertyNames`, `prefixItems` and the rest of
 * `APPLICATOR_KEYWORDS` — are asserted by `./applicators.js` through the custom
 * kind {@link APPLICATOR_KIND}.
 */
const STRING_CONSTRAINTS: readonly string[] = ['minLength', 'maxLength', 'pattern', 'format'];
const NUMERIC_CONSTRAINTS: readonly string[] = [
  'minimum',
  'maximum',
  'exclusiveMinimum',
  'exclusiveMaximum',
  'multipleOf',
];
const ARRAY_CONSTRAINTS: readonly string[] = ['minItems', 'maxItems', 'uniqueItems'];
const OBJECT_CONSTRAINTS: readonly string[] = ['minProperties', 'maxProperties'];

/** Copy the constraint keywords a type supports into a TypeBox options object. */
function _constraintOptions(
  schema: Record<string, unknown>,
  keywords: readonly string[],
): Record<string, unknown> {
  const opts: Record<string, unknown> = {};
  for (const keyword of keywords) {
    if (keyword in schema) opts[keyword] = schema[keyword];
  }
  return opts;
}

function _convertObject(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema {
  const properties = schema['properties'] as Record<string, Record<string, unknown>> | undefined;
  const required = new Set((schema['required'] as string[]) ?? []);
  const additional = _additionalPropertiesOption(schema, root);
  const opts = { ..._constraintOptions(schema, OBJECT_CONSTRAINTS), ...additional };

  if (properties || required.size > 0) {
    const typeboxProps: Record<string, TSchema> = {};
    for (const [name, propSchema] of Object.entries(properties ?? {})) {
      const propType = _convert(propSchema, root);
      typeboxProps[name] = required.has(name) ? propType : Type.Optional(propType);
    }
    // §6.5.3 constrains presence, not shape, so a name listed in `required`
    // without a `properties` entry still has to be there. `{required: ["b"]}` is
    // a whole schema on its own and is exactly what an `if` / `then` /
    // `dependentSchemas` sub-schema usually looks like.
    for (const name of required) {
      if (!(name in typeboxProps)) typeboxProps[name] = Type.Unknown();
    }
    return Type.Object(typeboxProps, opts);
  }
  if (additional) return Type.Object({}, opts);
  // No declared properties and no additionalProperties constraint: an open map.
  return Type.Record(Type.String(), Type.Unknown(), opts);
}

/**
 * Translate `additionalProperties` into the TypeBox object option. Ignoring it
 * let unknown properties through, while apcore-python (`extra="forbid"`) and
 * apcore-rust both reject them. `true` (the default) needs no option.
 *
 * A `patternProperties` sibling changes which keys count as additional
 * (§10.3.2.3 excludes every pattern-matched key), which the TypeBox option
 * cannot express — `_checkApplicatorNode` enforces both together instead.
 */
function _additionalPropertiesOption(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): { additionalProperties: false | TSchema } | undefined {
  if ('patternProperties' in schema) return undefined;
  const additional = schema['additionalProperties'];
  if (additional === false) return { additionalProperties: false };
  if (additional !== null && typeof additional === 'object' && !Array.isArray(additional)) {
    return {
      additionalProperties: _convert(additional as Record<string, unknown>, root),
    };
  }
  return undefined;
}

function _convertArray(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema {
  const opts = _constraintOptions(schema, ARRAY_CONSTRAINTS);
  const contains = schema['contains'];
  if (
    contains !== null &&
    typeof contains === 'object' &&
    !Array.isArray(contains) &&
    // `minContains: 0` is handled by the applicator evaluator instead — see
    // `containsNeedsEvaluator`. TypeBox's array check short-circuits on a zero
    // match count before it ever consults `minContains`, so emitting the trio
    // here would reject every array with no match even though §6.4.5 makes
    // that valid. apcore-python and apcore-rust accept it.
    !containsNeedsEvaluator(schema)
  ) {
    opts['contains'] = _convert(contains as Record<string, unknown>, root);
    // §6.4.4 / §6.4.5 make these meaningful only alongside `contains`. TypeBox
    // applies them regardless and then rejects every array, so they are emitted
    // only when `contains` is there to give them a subject.
    Object.assign(opts, _constraintOptions(schema, ['minContains', 'maxContains']));
  }
  // With a `prefixItems` sibling, `items` no longer describes every position —
  // it applies only past the prefix (§10.3.1.2). `Type.Array` has one element
  // schema for the whole array, so both are left to `_checkApplicatorNode`.
  const items = 'prefixItems' in schema ? undefined : (schema['items'] as Record<string, unknown>);
  return Type.Array(items ? _convert(items, root) : Type.Unknown(), opts);
}

function _convertString(schema: Record<string, unknown>): TSchema {
  const opts = _constraintOptions(schema, STRING_CONSTRAINTS);
  // The `format` value is carried through verbatim: JSON Schema 2020-12 §7.2.1
  // makes it an annotation, so it belongs on the schema but must not become an
  // assertion (apexe#32). Neutralising it for TypeBox's structural check is the
  // validator's job — `withFormatsAsAnnotations` scopes that to one check
  // instead of mutating the process-global `FormatRegistry` permanently.
  return Type.String(opts);
}

function _convertNumeric(
  schema: Record<string, unknown>,
  factory: (opts?: Record<string, unknown>) => TSchema,
): TSchema {
  return factory(_constraintOptions(schema, NUMERIC_CONSTRAINTS));
}

/**
 * The instance type each §6 keyword table constrains, plus the TypeBox node that
 * recognises that type carrying no constraints at all.
 *
 * A validation keyword applies only to instances of its own type and is inert on
 * every other type (JSON Schema 2020-12 §6): `{minimum: 3}` rejects `1` but
 * accepts `"x"`, `[1]`, `true` and `null`. `integer` needs no row — it shares
 * `number`'s keywords and its own JS representation.
 *
 * `keywords` lists what makes the group *apply*; the type converter decides what
 * it does with them. The object and array rows therefore reach past their
 * option tables to the shape keywords `_convertObject` / `_convertArray` also
 * read, because `{required: [...]}` and `{items: {...}}` are complete schemas on
 * their own — `if` / `then` / `dependentSchemas` sub-schemas are written that
 * way constantly, and dropping them made every such condition vacuously true.
 */
const BARE_CONSTRAINT_GROUPS: readonly {
  readonly type: string;
  readonly keywords: readonly string[];
  readonly node: () => TSchema;
}[] = [
  { type: 'string', keywords: STRING_CONSTRAINTS, node: () => Type.String() },
  { type: 'number', keywords: NUMERIC_CONSTRAINTS, node: () => Type.Number() },
  {
    type: 'array',
    keywords: [...ARRAY_CONSTRAINTS, 'items', 'contains'],
    node: () => Type.Array(Type.Unknown()),
  },
  {
    type: 'object',
    keywords: [...OBJECT_CONSTRAINTS, 'properties', 'required', 'additionalProperties'],
    node: () => Type.Object({}),
  },
];

/**
 * Convert a schema that declares no `type`, as `anyOf` / `oneOf` / `allOf`
 * branches, `additionalProperties`, `items`, `contains` and `not` subschemas
 * routinely do. Everything it asserts must hold at once, so the combinator
 * keywords and the bare constraint keywords are intersected.
 *
 * Both halves used to be handled by `_convertCombinator` alone, which returned
 * `Type.Unknown()` when no combinator keyword was present — so
 * `{minLength: 3}` and `{minimum: 3}` became accept-anything and their
 * constraint vanished. The constraint tables were only ever consulted from the
 * `type`-specific converters, leaving type-less positions unreachable.
 */
function _convertTypeless(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema {
  const members = [
    ..._bareConstraintMembers(schema, root),
    ..._combinatorMembers(schema, root),
  ];
  if (members.length === 0) return Type.Unknown();
  if (members.length === 1) return members[0];
  return Type.Intersect(members);
}

/**
 * One TypeBox node per constraint group the type-less schema carries, each
 * shaped "is this type -> the constraints apply; is not this type -> pass".
 *
 * Narrowing to the constrained type instead would be stricter than JSON Schema:
 * `{minimum: 3}` would become "must be a number and >= 3" and start rejecting
 * the strings, arrays and nulls the spec accepts — a worse error than the
 * accept-anything it replaces. Groups are independent, so a schema carrying
 * both `minimum` and `minLength` yields two members that the caller intersects.
 */
function _bareConstraintMembers(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema[] {
  const branchSchema = _typeBranchSchema(schema);
  const members: TSchema[] = [];
  for (const group of BARE_CONSTRAINT_GROUPS) {
    if (!group.keywords.some((keyword) => keyword in branchSchema)) continue;
    const constrained = _convert({ ...branchSchema, type: group.type }, root);
    members.push(Type.Union([constrained, Type.Not(group.node())]));
  }
  return members;
}

/**
 * Convert every combinator keyword the schema carries, not just the first one.
 * `enum` / `const` / `oneOf` / `anyOf` / `allOf` / `not` are independent
 * assertions that must all hold (JSON Schema 2020-12 §10.2), so returning at
 * the first match silently dropped the rest — `{enum: [...], not: {...}}`
 * ignored the `not`, `{anyOf: [...], allOf: [...]}` ignored the `allOf`.
 * apcore-python intersects the whole sibling set the same way.
 */
function _convertCombinator(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema {
  const members = _combinatorMembers(schema, root);
  if (members.length === 0) return Type.Unknown();
  if (members.length === 1) return members[0];
  return Type.Intersect(members);
}

/** One TypeBox node per combinator keyword present, in JSON Schema order. */
function _combinatorMembers(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema[] {
  const branchesOf = (keyword: string): TSchema[] =>
    (schema[keyword] as Record<string, unknown>[]).map((s) => _convert(s, root));

  const members: TSchema[] = [];
  if ('enum' in schema) {
    const enumeration = Type.Union((schema['enum'] as unknown[]).map(_convertConstValue));
    // Preserve its source keyword for canonical validation diagnostics.
    enumeration['enum'] = schema['enum'];
    members.push(enumeration);
  }
  if ('const' in schema) members.push(_convertConstValue(schema['const']));
  // Both unions are marked with the keyword they came from, so the validator
  // still recognises them after a `type` sibling wraps them in an intersection.
  if ('oneOf' in schema) members.push(_markedUnion(branchesOf('oneOf'), 'oneOf'));
  if ('anyOf' in schema) members.push(_markedUnion(branchesOf('anyOf'), 'anyOf'));
  if ('allOf' in schema) members.push(Type.Intersect(branchesOf('allOf')));
  if ('not' in schema) {
    members.push(Type.Not(_convert(schema['not'] as Record<string, unknown>, root)));
  }
  return members;
}

/**
 * TypeBox kind implementing `oneOf` exclusivity: exactly one branch may match
 * (JSON Schema 2020-12 §10.2.1.3, spec §4.15.1).
 *
 * A plain `Type.Union` is `anyOf` — it accepts as soon as one branch matches —
 * so a `oneOf` nested inside `properties` silently lost its exclusivity rule and
 * `{"a": {"oneOf": [{"type": "integer"}, {"type": "number"}]}}` accepted `3`,
 * which both apcore-python and apcore-rust reject. Enforcing it in the node
 * rather than in `SchemaValidator` makes it hold at every depth, since it is
 * `Value.Check` that walks nested positions.
 */
const ONE_OF_KIND = 'apcore:OneOf';

if (!TypeRegistry.Has(ONE_OF_KIND)) {
  TypeRegistry.Set(
    ONE_OF_KIND,
    (schema, value): boolean =>
      (schema as { anyOf: TSchema[] }).anyOf.filter((branch) => Value.Check(branch, value))
        .length === 1,
  );
}

/**
 * Build a union tagged with the JSON Schema keyword it came from. The tag is
 * what lets `SchemaValidator` tell `oneOf` (exactly one branch) from `anyOf`
 * (at least one branch) and report SCHEMA_UNION_* error codes, including when
 * a `type` sibling nests the union inside an intersection.
 *
 * `anyOf` keeps TypeBox's own union node — its semantics already match. `oneOf`
 * gets {@link ONE_OF_KIND} instead, but keeps the `anyOf` branch array on the
 * node so the validator can count matches and report which of
 * SCHEMA_UNION_NO_MATCH / SCHEMA_UNION_AMBIGUOUS applies.
 */
function _markedUnion(branches: TSchema[], keyword: 'oneOf' | 'anyOf'): TSchema {
  if (keyword === 'oneOf') {
    return Type.Unsafe({ [Kind]: ONE_OF_KIND, anyOf: branches, [KEYWORD_MARKER]: 'oneOf' });
  }
  const union = Type.Union(branches) as Record<string, unknown>;
  union[KEYWORD_MARKER] = keyword;
  return union as TSchema;
}

// ---------------------------------------------------------------------------
// Lazy `$ref` binding (recursive schemas)
// ---------------------------------------------------------------------------

/**
 * TypeBox kind that defers a `$ref` to check time.
 *
 * `RefResolver` leaves a self-reference in place rather than inlining it
 * (spec §4.15.2), so the converter must terminate on a schema that contains
 * itself. Converting the target eagerly would not: the node stores the *raw*
 * target sub-schema and converts it on first check, memoised by target identity,
 * so recursion is driven by the depth of the data rather than by the schema.
 */
const REF_KIND = 'apcore:Ref';

/** Converted `$ref` targets, keyed by target sub-schema then by root document. */
const _refTargetCache = new WeakMap<object, WeakMap<object, TSchema>>();

function _checkRef(
  target: Record<string, unknown>,
  root: Record<string, unknown>,
  value: unknown,
): boolean {
  let byRoot = _refTargetCache.get(target);
  if (byRoot === undefined) {
    byRoot = new WeakMap<object, TSchema>();
    _refTargetCache.set(target, byRoot);
  }
  let converted = byRoot.get(root);
  if (converted === undefined) {
    converted = jsonSchemaToTypeBox(target, root);
    byRoot.set(root, converted);
  }
  return Value.Check(converted, value);
}

if (!TypeRegistry.Has(REF_KIND)) {
  TypeRegistry.Set(REF_KIND, (schema, value): boolean => {
    const node = schema as {
      apcoreRefTarget: Record<string, unknown>;
      apcoreRefRoot: Record<string, unknown>;
    };
    return _checkRef(node.apcoreRefTarget, node.apcoreRefRoot, value);
  });
}

/**
 * Locate what a `$ref` points at within `root`. Recognises the three forms a
 * resolved schema can still carry: the whole-document anchors `#` and `#/`, the
 * root `$id`, and an in-document JSON pointer (`#/$defs/Node`).
 *
 * Returns `undefined` for anything else — an external reference the resolver
 * could not inline — which the caller degrades to `unknown` rather than failing
 * the whole conversion.
 */
function _resolveRefTarget(
  ref: string,
  root: Record<string, unknown>,
): Record<string, unknown> | undefined {
  if (ref === '#' || ref === '#/') return root;
  if (typeof root['$id'] === 'string' && ref === root['$id']) return root;
  if (!ref.startsWith('#/')) return undefined;

  let current: unknown = root;
  for (const rawSegment of ref.slice(2).split('/')) {
    const segment = rawSegment.replace(/~1/g, '/').replace(/~0/g, '~');
    if (current === null || typeof current !== 'object' || Array.isArray(current)) return undefined;
    current = (current as Record<string, unknown>)[segment];
  }
  if (current === null || typeof current !== 'object' || Array.isArray(current)) return undefined;
  return current as Record<string, unknown>;
}

function _convertRef(
  ref: string,
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema {
  const target = _resolveRefTarget(ref, root);
  // A `$ref` whose target is the node holding it asserts nothing and would spin
  // forever; an unresolvable one has nothing to assert either.
  if (target === undefined || target === schema) return Type.Unknown();
  return Type.Unsafe({ [Kind]: REF_KIND, apcoreRefTarget: target, apcoreRefRoot: root });
}

/**
 * TypeBox kind routing `Value.Check` to a JSON-equality comparison, used for the
 * `enum` / `const` members `Type.Literal` cannot express.
 */
const JSON_CONST_KIND = 'apcore:JsonConst';

if (!TypeRegistry.Has(JSON_CONST_KIND)) {
  TypeRegistry.Set(
    JSON_CONST_KIND,
    (schema, value): boolean =>
      sortedKeysStringify((schema as { const: unknown }).const) === sortedKeysStringify(value),
  );
}

/**
 * Convert a single `enum` member or a `const` value.
 *
 * JSON Schema allows any JSON value there, but `Type.Literal` accepts only
 * scalars and compares with `===`, so an object or array member produced a node
 * that rejected even the exactly-equal value. Non-scalars therefore get a
 * `const` node whose checker compares canonical JSON. Scalars keep
 * `Type.Literal` for its better error messages.
 */
function _convertConstValue(value: unknown): TSchema {
  if (value === null) return Type.Null();
  const valueType = typeof value;
  if (valueType === 'string' || valueType === 'number' || valueType === 'boolean') {
    return Type.Literal(value as string | number | boolean);
  }
  return Type.Unsafe({ [Kind]: JSON_CONST_KIND, const: value });
}

// ---------------------------------------------------------------------------
// Applicator keywords (prefixItems, patternProperties, propertyNames,
// dependentRequired, dependentSchemas, if/then/else, unevaluated*)
// ---------------------------------------------------------------------------

/**
 * TypeBox kind routing `Value.Check` to the applicator evaluator. Same
 * mechanism as {@link JSON_CONST_KIND}: TypeBox has no node for these keywords,
 * so a custom kind carries the raw sub-schema and decides for itself.
 */
const APPLICATOR_KIND = 'apcore:Applicators';

/**
 * Validate a value against a raw applicator sub-schema by routing it back
 * through the converter, so nested `type` / §6 / combinator keywords are decided
 * by TypeBox and there is only ever one validation engine. Conversion is cached
 * per raw schema object — keyed by root document as well, since the same
 * sub-schema resolves its `$ref`s against whichever document it came from —
 * because the evaluator re-checks the same sub-schema for every array element or
 * object property. {@link _checkRef} already provides exactly that cache.
 */
function _subSchemaChecker(
  root: Record<string, unknown>,
): (sub: Record<string, unknown>, value: unknown) => boolean {
  return (sub, value) => _checkRef(sub, root, value);
}

if (!TypeRegistry.Has(APPLICATOR_KIND)) {
  TypeRegistry.Set(APPLICATOR_KIND, (schema, value): boolean => {
    const node = schema as {
      apcoreSchema: Record<string, unknown>;
      apcoreRoot: Record<string, unknown>;
    };
    return checkApplicators(node.apcoreSchema, value, _subSchemaChecker(node.apcoreRoot));
  });
}

/**
 * Intersect the converted node with the applicator assertions the schema
 * carries, or return it unchanged when it carries none.
 *
 * The applicator node holds the whole raw schema rather than the matched
 * keywords alone: `unevaluatedItems` / `unevaluatedProperties` are defined
 * against the annotations *every* sibling applicator produced (§11), so slicing
 * the schema down would make them reject values the siblings had already
 * evaluated.
 */
function _withApplicators(
  node: TSchema,
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): TSchema {
  if (!hasApplicatorKeyword(schema)) return node;
  return Type.Intersect([
    node,
    Type.Unsafe({ [Kind]: APPLICATOR_KIND, apcoreSchema: schema, apcoreRoot: root }),
  ]);
}
