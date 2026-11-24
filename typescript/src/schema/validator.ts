/**
 * SchemaValidator — validates runtime data against TypeBox schemas.
 */

import type { TSchema } from '@sinclair/typebox';
import { Value, type ValueError } from '@sinclair/typebox/value';
import { ValueErrorType } from '@sinclair/typebox/errors';
import type { SchemaValidationErrorDetail, SchemaValidationResult } from './types.js';
import { validationResultToError } from './types.js';
import { KEYWORD_MARKER } from './constants.js';
import { FORMAT_VALIDATORS, withFormatsAsAnnotations } from './formats.js';

/**
 * The branch list of a union node, or `null` when the schema is not one.
 *
 * Reads `anyOf` directly rather than going through `TypeGuard.IsUnion`, because
 * the converter emits `oneOf` as a custom kind (exclusivity has to hold at every
 * nesting depth, which a plain `Type.Union` cannot express) while still keeping
 * the branch array under `anyOf`.
 */
function _unionBranches(schema: TSchema): TSchema[] | null {
  const branches = (schema as Record<string, unknown>)['anyOf'];
  return Array.isArray(branches) ? (branches as TSchema[]) : null;
}

/**
 * Find the union node carrying {@link KEYWORD_MARKER}, descending through the
 * `allOf` members the converter produces when `oneOf` / `anyOf` has a `type`
 * sibling. Returns `null` when the schema expresses no union assertion.
 */
function findMarkedUnion(schema: Record<string, unknown>): TSchema | null {
  if (typeof schema[KEYWORD_MARKER] === 'string') return schema as TSchema;
  const allOf = schema['allOf'];
  if (!Array.isArray(allOf)) return null;
  for (const member of allOf as Record<string, unknown>[]) {
    const found = findMarkedUnion(member);
    if (found !== null) return found;
  }
  return null;
}

/** The declared JSON Schema types of a node — `type` may be a string or array. */
function declaredTypes(schema: Record<string, unknown>): string[] {
  const t = schema['type'];
  if (typeof t === 'string') return [t];
  if (Array.isArray(t)) return t.filter((x): x is string => typeof x === 'string');
  return [];
}

/**
 * Coerce a string to an integer iff it represents an exactly integral value.
 *
 * TYPE_MAPPING §11's integer row: a string whose entire content parses as an
 * integer. `"42"` → 42, `"-7"` → -7; the integral-float and surrounding-
 * whitespace tolerances (`"42.0"` → 42, `" 42 "` → 42) are shared with
 * apcore-python (pydantic lax) and `apcore-rust::coerce_str_to_integer`.
 * `"3.14"` is a MUST-reject — the declared type bounds the coercion, not the
 * string's parseability — as are `"abc"` and `""`. A lossy conversion is
 * refused rather than truncated, so no SDK silently changes a value's meaning.
 */
function coerceStrToInteger(s: string): number | null {
  const trimmed = s.trim();
  if (trimmed === '') return null;
  const f = Number(trimmed);
  if (!Number.isFinite(f) || !Number.isInteger(f)) return null;
  // Outside the exactly-representable range a parsed double no longer stands
  // for the digits it was written with, so the conversion is not lossless.
  if (!Number.isSafeInteger(f)) return null;
  return f;
}

/** Coerce a string to a float, rejecting empty and non-finite input. */
function coerceStrToNumber(s: string): number | null {
  const trimmed = s.trim();
  if (trimmed === '') return null;
  const f = Number(trimmed);
  return Number.isFinite(f) ? f : null;
}

/**
 * Coerce a string to a boolean: exactly `"true"` and `"false"`, case-sensitive.
 *
 * TYPE_MAPPING §11 "What the knob coerces, when it exists" (normative as of
 * spec v1.12.0) caps the accepted set at JSON's own two boolean literals, so
 * accepting them is reading the same value written as text.
 *
 * This helper used to carry a twelve-spelling case-insensitive dialect ported
 * from `apcore-rust::coerce_str_to_bool` during apcore#93 — `yes`/`no`,
 * `on`/`off`, `y`/`n`, `t`/`f`, `1`/`0`, in any casing. Those are shell and INI
 * conventions and belong to whatever parses `argv`. `"0"` → `false` was the
 * sharpest of them: R5 makes the *number* `0` a MUST-reject for `boolean`, so
 * accepting the string `"0"` here put two paths of this SDK on opposite sides
 * of one value (apcore#95).
 *
 * Nothing is trimmed and nothing is lowercased: `" true "` and `"True"` are not
 * `"true"`.
 */
function coerceStrToBool(s: string): boolean | null {
  if (s === 'true') return true;
  if (s === 'false') return false;
  return null;
}

/**
 * Return `value` with strings converted to the scalar types `schema` declares.
 *
 * Pure — the input is never mutated, so a caller's object is safe to pass. Only
 * reachable under `coerceTypes: true`; see {@link SchemaValidator}.
 *
 * A union node (`anyOf`/`oneOf`, which is also how the converter renders
 * `type: ["string", "boolean"]`) declares no single scalar target, so it is left
 * untouched: guessing a branch is what makes coercion unpredictable.
 */
export function coerceValue(value: unknown, schema: unknown): unknown {
  if (typeof schema !== 'object' || schema === null || Array.isArray(schema)) return value;
  const s = schema as Record<string, unknown>;
  const types = declaredTypes(s);

  if (types.includes('object')) {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) return value;
    const props = s['properties'];
    const propMap =
      typeof props === 'object' && props !== null && !Array.isArray(props)
        ? (props as Record<string, unknown>)
        : null;
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      const propSchema = propMap === null ? undefined : propMap[k];
      out[k] = propSchema === undefined ? v : coerceValue(v, propSchema);
    }
    return out;
  }

  if (types.includes('array')) {
    const items = s['items'];
    if (!Array.isArray(value) || items === undefined) return value;
    return value.map((item) => coerceValue(item, items));
  }

  if (typeof value !== 'string') return value;
  // Order matters only in that a value satisfying several declared targets takes
  // the first — the same precedence apcore-rust applies.
  if (types.includes('boolean')) {
    const b = coerceStrToBool(value);
    if (b !== null) return b;
  }
  if (types.includes('integer')) {
    const i = coerceStrToInteger(value);
    if (i !== null) return i;
  }
  if (types.includes('number')) {
    const n = coerceStrToNumber(value);
    if (n !== null) return n;
  }
  return value;
}

/**
 * Validates runtime data against a TypeBox schema.
 *
 * `coerceTypes` is a **library-level** knob, for a caller validating its own
 * untyped input (a CLI parsing argv, a form handler). It does **not** reach the
 * module-invocation boundary: `builtin-steps.ts` holds a
 * `new SchemaValidator(false)` and never coerces, under any host configuration
 * (TYPE_MAPPING §17.3). There is no `schema.validation.coerce_types` setting —
 * a module's contract has to mean the same thing regardless of who loaded it.
 *
 * It defaults to `false`, matching the boundary and the other two SDKs. Coercion
 * is opt-in: a validator that silently rewrites its input is the wrong default
 * for the common case of checking data you already believe is well-formed.
 *
 * Passing `true` runs {@link coerceValue} as a pre-pass, then validates the
 * coerced value. What it coerces is fixed by TYPE_MAPPING §11 — offering the
 * knob stays a **MAY**, but an SDK that offers one **MUST** coerce exactly this
 * set and nothing else:
 *
 * | from | to | accepted |
 * |---|---|---|
 * | string | `integer` | entire content parses as an integer — `"42"`, `"-7"` |
 * | string | `number`  | entire content parses as a number — `"1.5"`, `"-0.5"` |
 * | string | `boolean` | exactly `"true"` and `"false"`, case-sensitive |
 *
 * Coercion is FROM a string only, and only toward a type the schema declares:
 * a number is never coerced to a boolean, a boolean never to a number, and
 * nothing is coerced toward `string`. `"3.14"` for `{type: "integer"}` stays a
 * string and therefore fails, because a lossy conversion is refused rather than
 * truncated. That is deliberately NOT `Value.Convert`, which mutates its
 * argument in place and truncates `"4.5"` to `4` — a silent data change no
 * other SDK makes.
 *
 * The module-invocation boundary is unaffected: it never coerces on any SDK
 * (TYPE_MAPPING §17.3).
 */
export class SchemaValidator {
  private _coerceTypes: boolean;

  constructor(coerceTypes: boolean = false) {
    this._coerceTypes = coerceTypes;
  }

  /**
   * A `format` TypeBox does not know would otherwise make the structural check
   * fail. JSON Schema 2020-12 §7.2.1 makes `format` an annotation rather than an
   * assertion by default, so every format the schema carries is neutralised for
   * the duration of the check and restored afterwards (apexe#32). Recognised
   * formats are enforced at SHOULD level by `_checkFormats`, never here.
   */
  validate(data: Record<string, unknown>, schema: TSchema): SchemaValidationResult {
    return withFormatsAsAnnotations(schema, () => this._validate(this._coerce(data, schema), schema));
  }

  /**
   * The coercion pre-pass, or the value unchanged when coercion is off.
   *
   * Applied once at each public entry point so the structural check, the error
   * collection and the returned value all see the SAME value — collecting errors
   * against the raw input would report paths for a value that was never
   * validated.
   */
  private _coerce(data: Record<string, unknown>, schema: TSchema): Record<string, unknown> {
    if (!this._coerceTypes) return data;
    return coerceValue(data, schema) as Record<string, unknown>;
  }

  private _validate(data: Record<string, unknown>, schema: TSchema): SchemaValidationResult {
    const s = schema as Record<string, unknown>;

    // oneOf: exhaustive counting — exactly one branch must match
    if (s[KEYWORD_MARKER] === 'oneOf') {
      return this._validateOneOf(data, schema);
    }

    // anyOf: at least one branch must match; use Value.Check per branch for
    // SCHEMA_UNION_NO_MATCH. An unmarked TypeBox `Type.Union` (a schema the
    // module author wrote by hand) gets the same treatment.
    if (s[KEYWORD_MARKER] === 'anyOf' || ('anyOf' in s && !(KEYWORD_MARKER in s))) {
      return this._validateAnyOf(data, schema);
    }

    // A `type` sibling wraps the marked union in an intersection, burying the
    // marker one or more levels down. The union assertion still applies, so
    // `{type: 'object', oneOf: [...]}` and `{type: 'object', anyOf: [...]}`
    // report SCHEMA_UNION_AMBIGUOUS / SCHEMA_UNION_NO_MATCH exactly like their
    // bare counterparts instead of degrading to SCHEMA_VALIDATION_ERROR. Only
    // failures short-circuit here; a satisfied union falls through so the rest
    // of the intersection (the `type` half) is still checked.
    const nestedUnion = findMarkedUnion(s);
    if (nestedUnion !== null) {
      const failure = this._unionFailure(data, nestedUnion);
      if (failure !== null) return failure;
    }

    return this._validateStructural(data, schema);
  }

  /** The plain TypeBox check, decoding first when type coercion is enabled. */
  private _validateStructural(data: unknown, schema: TSchema): SchemaValidationResult {
    let passed: boolean;
    if (this._coerceTypes) {
      try {
        Value.Decode(schema, data);
        passed = true;
      } catch {
        passed = false;
      }
    } else {
      passed = Value.Check(schema, data);
    }

    if (!passed) {
      // Errors are collected only on the failing path — `Value.Errors` walks
      // the whole schema.
      return {
        valid: false,
        errors: this._collectErrors(schema, data),
        errorCode: 'SCHEMA_VALIDATION_ERROR',
      };
    }
    const warnLogged = this._checkFormats(data, schema);
    return { valid: true, errors: [], ...(warnLogged && { warnLogged: true }) };
  }

  validateInput(data: Record<string, unknown>, schema: TSchema): Record<string, unknown> {
    return this._validateAndReturn(data, schema);
  }

  validateOutput(data: Record<string, unknown>, schema: TSchema): Record<string, unknown> {
    return this._validateAndReturn(data, schema);
  }

  private _validateOneOf(data: unknown, schema: TSchema): SchemaValidationResult {
    const failure = this._oneOfFailure(data, schema);
    if (failure !== null) return failure;
    const warnLogged = this._checkFormats(data, schema);
    return { valid: true, errors: [], ...(warnLogged && { warnLogged: true }) };
  }

  /**
   * `oneOf` exhaustive counting: exactly one branch must match. Returns the
   * failure result, or `null` when the data satisfies the exclusivity rule.
   * Kept free of format reporting so it can also be applied to a marked union
   * nested inside an intersection without warning twice.
   */
  private _oneOfFailure(data: unknown, schema: TSchema): SchemaValidationResult | null {
    const branches = _unionBranches(schema);
    if (branches === null) {
      // TypeBox 0.34 unwraps single-element unions to the branch type itself.
      // A single-branch oneOf always matches exactly one branch if the data is valid.
      if (!Value.Check(schema, data)) {
        return {
          valid: false,
          errors: this._collectErrors(schema, data),
          errorCode: 'SCHEMA_UNION_NO_MATCH',
        };
      }
      return null;
    }
    const matchCount = branches.filter((b) => Value.Check(b, data)).length;
    if (matchCount === 0) {
      return {
        valid: false,
        errors: [{ path: '', message: 'oneOf: no branches matched', keyword: 'oneOf' }],
        errorCode: 'SCHEMA_UNION_NO_MATCH',
      };
    }
    if (matchCount > 1) {
      return {
        valid: false,
        errors: [{ path: '', message: `oneOf: expected exactly 1 match, got ${matchCount}`, keyword: 'oneOf' }],
        errorCode: 'SCHEMA_UNION_AMBIGUOUS',
      };
    }
    return null;
  }

  private _validateAnyOf(data: unknown, schema: TSchema): SchemaValidationResult {
    const failure = this._anyOfFailure(data, schema);
    if (failure !== null) return failure;
    const warnLogged = this._checkFormats(data, schema);
    return { valid: true, errors: [], ...(warnLogged && { warnLogged: true }) };
  }

  /**
   * `anyOf`: at least one branch must match. Unlike `oneOf` there is no
   * exclusivity rule — several branches matching is valid — so the only failure
   * is zero matches. Returns the failure result, or `null` when satisfied.
   */
  private _anyOfFailure(data: unknown, schema: TSchema): SchemaValidationResult | null {
    const branches = _unionBranches(schema);
    if (branches === null) {
      // TypeBox 0.34 unwraps single-element unions; treat the unwrapped schema as a single branch.
      if (!Value.Check(schema, data)) {
        return {
          valid: false,
          errors: this._collectErrors(schema, data),
          errorCode: 'SCHEMA_UNION_NO_MATCH',
        };
      }
      return null;
    }
    if (!branches.some((b) => Value.Check(b, data))) {
      return {
        valid: false,
        errors: [{ path: '', message: 'anyOf: no branches matched', keyword: 'anyOf' }],
        errorCode: 'SCHEMA_UNION_NO_MATCH',
      };
    }
    return null;
  }

  /** Apply the branch semantics the marked union's originating keyword implies. */
  private _unionFailure(data: unknown, union: TSchema): SchemaValidationResult | null {
    const keyword = (union as Record<string, unknown>)[KEYWORD_MARKER];
    return keyword === 'oneOf'
      ? this._oneOfFailure(data, union)
      : this._anyOfFailure(data, union);
  }

  private _validateAndReturn(data: Record<string, unknown>, schema: TSchema): Record<string, unknown> {
    return withFormatsAsAnnotations(schema, () => {
      // Route through _validate() so union schemas surface their specific error
      // code (SCHEMA_UNION_NO_MATCH / SCHEMA_UNION_AMBIGUOUS) and plain failures
      // surface SCHEMA_VALIDATION_ERROR — all preserved through the thrown error.
      const value = this._coerce(data, schema);
      const result = this._validate(value, schema);
      if (!result.valid) {
        throw validationResultToError(result);
      }

      if (this._coerceTypes) {
        return Value.Decode(schema, value) as Record<string, unknown>;
      }
      return value;
    });
  }

  /**
   * Walk the schema and data together to find format-annotated string fields.
   * Emits console.warn for each format violation (SHOULD-level enforcement).
   * Returns true if any warnings were emitted.
   */
  private _checkFormats(data: unknown, schema: TSchema): boolean {
    const warnings: string[] = [];
    this._walkFormats(data, schema as Record<string, unknown>, '/', warnings);
    // The same annotation can be reached through more than one union branch.
    const unique = [...new Set(warnings)];
    for (const w of unique) {
      console.warn(`[apcore:schema] ${w}`);
    }
    return unique.length > 0;
  }

  private _walkFormats(data: unknown, schema: Record<string, unknown>, path: string, warnings: string[]): void {
    if (typeof schema !== 'object' || schema === null) return;

    // Union (a `type` array, `anyOf` or `oneOf`): the format annotation lives on
    // the branches, not on the union node. Only branches the data satisfies
    // contribute — a sibling branch describes a different shape, and warning
    // from it would report a format the value was never meant to carry.
    const anyOf = schema['anyOf'];
    if (Array.isArray(anyOf)) {
      for (const branch of anyOf as Record<string, unknown>[]) {
        if (Value.Check(branch as TSchema, data)) {
          this._walkFormats(data, branch, path, warnings);
        }
      }
      return;
    }

    // Intersection (`type` plus a combinator sibling, or `allOf`): every member
    // applies, so every member's annotations are collected.
    const allOf = schema['allOf'];
    if (Array.isArray(allOf)) {
      for (const member of allOf as Record<string, unknown>[]) {
        this._walkFormats(data, member, path, warnings);
      }
      return;
    }

    // String with format annotation
    if (schema['type'] === 'string' && typeof schema['format'] === 'string' && typeof data === 'string') {
      const format = schema['format'] as string;
      const validator = FORMAT_VALIDATORS[format];
      if (validator && !validator(data)) {
        warnings.push(`Format '${format}' validation failed at ${path}: "${data}"`);
      }
      return;
    }

    // Object: recurse into declared properties and the additionalProperties schema
    if (schema['type'] === 'object' && typeof data === 'object' && data !== null && !Array.isArray(data)) {
      this._walkObjectFormats(data as Record<string, unknown>, schema, path, warnings);
    }

    // Array: recurse into each element using the items schema
    if (schema['type'] === 'array' && schema['items'] && Array.isArray(data)) {
      const itemSchema = schema['items'] as Record<string, unknown>;
      data.forEach((item, i) => {
        this._walkFormats(item, itemSchema, `${path === '/' ? '' : path}/${i}`, warnings);
      });
    }
  }

  /**
   * Recurse into an object's declared `properties` and, for the keys none of
   * them covers, into the `additionalProperties` subschema. Skipping the latter
   * hid every format annotation on an open-ended map — apcore-python reports
   * that shape.
   */
  private _walkObjectFormats(
    data: Record<string, unknown>,
    schema: Record<string, unknown>,
    path: string,
    warnings: string[],
  ): void {
    const child = (key: string): string => `${path}${path === '/' ? '' : '/'}${key}`;
    const props = (schema['properties'] ?? {}) as Record<string, Record<string, unknown>>;
    for (const [key, propSchema] of Object.entries(props)) {
      if (key in data) this._walkFormats(data[key], propSchema, child(key), warnings);
    }

    const additional = schema['additionalProperties'];
    if (additional === null || typeof additional !== 'object' || Array.isArray(additional)) return;
    for (const [key, value] of Object.entries(data)) {
      if (key in props) continue;
      this._walkFormats(value, additional as Record<string, unknown>, child(key), warnings);
    }
  }

  private _collectErrors(schema: TSchema, data: unknown): SchemaValidationErrorDetail[] {
    const raw = [...Value.Errors(schema, data)];
    const missing = new Set(raw.filter((error) => error.type === ValueErrorType.ObjectRequiredProperty).map((error) => error.path));
    const errors: SchemaValidationErrorDetail[] = [];
    for (const error of raw) {
      if (missing.has(error.path) && error.type !== ValueErrorType.ObjectRequiredProperty && error.value === undefined) continue;
      // TypeBox adds aggregate intersection failures beside the precise causes.
      if (error.type === ValueErrorType.Intersect && raw.some((other) => other !== error && other.path.startsWith(error.path))) continue;
      const detail = this._typeboxErrorToDetail(error);
      if (!errors.some((item) => item.path === detail.path && item.keyword === detail.keyword)) errors.push(detail);
    }
    return errors;
  }

  private _typeboxErrorToDetail(error: ValueError): SchemaValidationErrorDetail {
    const type = ValueErrorType[error.type];
    const suffix = type.replace(/^(Array|BigInt|Date|Integer|Number|Object|String|Tuple|Uint8Array)/, '');
    const keywords: Record<string, string> = {
      Contains: 'contains', MaxContains: 'maxContains', MinContains: 'minContains',
      MaxItems: 'maxItems', MinItems: 'minItems', UniqueItems: 'uniqueItems',
      ExclusiveMaximum: 'exclusiveMaximum', ExclusiveMinimum: 'exclusiveMinimum',
      Maximum: 'maximum', Minimum: 'minimum', MultipleOf: 'multipleOf',
      AdditionalProperties: 'additionalProperties', MaxProperties: 'maxProperties',
      MinProperties: 'minProperties', RequiredProperty: 'required',
      Format: 'format', FormatUnknown: 'format', MaxLength: 'maxLength', MinLength: 'minLength', Pattern: 'pattern',
    };
    let keyword = keywords[suffix] ?? 'type';
    if (error.type === ValueErrorType.Union) keyword = 'enum' in error.schema ? 'enum' : 'anyOf';
    if (error.type === ValueErrorType.Literal) keyword = 'enum' in error.schema ? 'enum' : 'const';
    if (error.type === ValueErrorType.Intersect) keyword = 'allOf';
    if (error.type === ValueErrorType.Not) keyword = 'not';
    if (error.type === ValueErrorType.Kind) keyword = String(error.schema[KEYWORD_MARKER] ?? 'type');
    const objectConstraint = keyword === 'required' || keyword === 'additionalProperties';
    return {
      path: objectConstraint ? error.path.replace(/\/[^/]*$/, '') : error.path,
      message: error.message,
      keyword,
    };
  }
}
