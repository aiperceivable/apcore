import { describe, it, expect, vi, afterEach } from 'vitest';
import { Type, FormatRegistry } from '@sinclair/typebox';
import { Value } from '@sinclair/typebox/value';
import { SchemaValidator } from '../../src/schema/validator.js';
import { SchemaValidationError } from '../../src/errors.js';
import { validationResultToError } from '../../src/schema/types.js';
import { jsonSchemaToTypeBox } from '../../src/schema/loader.js';
import {
  FORMAT_VALIDATORS,
  collectSchemaFormats,
  withFormatsAsAnnotations,
} from '../../src/schema/formats.js';

describe('SchemaValidator', () => {
  it('validates correct data', () => {
    const validator = new SchemaValidator();
    const schema = Type.Object({ name: Type.String() });
    const result = validator.validate({ name: 'Alice' }, schema);
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
  });

  it('rejects invalid data', () => {
    const validator = new SchemaValidator();
    const schema = Type.Object({ name: Type.String() });
    const result = validator.validate({ name: 123 }, schema);
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it('validates without coercion', () => {
    const validator = new SchemaValidator(false);
    const schema = Type.Object({ x: Type.Number() });
    const result = validator.validate({ x: 42 }, schema);
    expect(result.valid).toBe(true);
  });

  it('validateInput returns data on valid input', () => {
    const validator = new SchemaValidator();
    const schema = Type.Object({ x: Type.Number() });
    const data = validator.validateInput({ x: 42 }, schema);
    expect(data['x']).toBe(42);
  });

  it('validateInput throws on invalid input', () => {
    const validator = new SchemaValidator();
    const schema = Type.Object({ x: Type.Number() });
    expect(() => validator.validateInput({ x: 'not-a-number' }, schema)).toThrow(SchemaValidationError);
  });

  it('validateOutput returns data on valid output', () => {
    const validator = new SchemaValidator();
    const schema = Type.Object({ result: Type.String() });
    const data = validator.validateOutput({ result: 'ok' }, schema);
    expect(data['result']).toBe('ok');
  });

  it('validateOutput throws on invalid output', () => {
    const validator = new SchemaValidator();
    const schema = Type.Object({ result: Type.String() });
    expect(() => validator.validateOutput({ result: 123 }, schema)).toThrow(SchemaValidationError);
  });

  it('error details include path and message', () => {
    const validator = new SchemaValidator();
    const schema = Type.Object({ nested: Type.Object({ x: Type.Number() }) });
    const result = validator.validate({ nested: { x: 'bad' } }, schema);
    expect(result.valid).toBe(false);
    expect(result.errors[0].path).toBeDefined();
    expect(result.errors[0].message).toBeDefined();
  });

  it('returns invalid result without coercion when data fails Check', () => {
    const validator = new SchemaValidator(false);
    const schema = Type.Object({ x: Type.Number() });
    const result = validator.validate({ x: 'not-a-number' }, schema);
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it('validateInput without coercion returns data on valid input', () => {
    const validator = new SchemaValidator(false);
    const schema = Type.Object({ x: Type.Number() });
    const data = validator.validateInput({ x: 42 }, schema);
    expect(data['x']).toBe(42);
  });

  it('validateInput without coercion throws on invalid input', () => {
    const validator = new SchemaValidator(false);
    const schema = Type.Object({ x: Type.Number() });
    expect(() => validator.validateInput({ x: 'not-a-number' }, schema)).toThrow(SchemaValidationError);
  });
});

describe('SchemaValidator — oneOf exhaustive validation', () => {
  const validator = new SchemaValidator(false);

  it('accepts input matching exactly one oneOf branch', () => {
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        { type: 'object', properties: { kind: { const: 'circle' }, radius: { type: 'number' } }, required: ['kind', 'radius'] },
        { type: 'object', properties: { kind: { const: 'rect' }, width: { type: 'number' } }, required: ['kind', 'width'] },
      ],
    });
    const result = validator.validate({ kind: 'circle', radius: 5 }, schema);
    expect(result.valid).toBe(true);
  });

  it('rejects input matching zero oneOf branches with SCHEMA_UNION_NO_MATCH', () => {
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        { type: 'object', properties: { kind: { const: 'circle' } }, required: ['kind'] },
        { type: 'object', properties: { kind: { const: 'rect' } }, required: ['kind'] },
      ],
    });
    const result = validator.validate({ kind: 'triangle' }, schema);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_UNION_NO_MATCH');
  });

  it('rejects input matching multiple oneOf branches with SCHEMA_UNION_AMBIGUOUS', () => {
    // Both branches accept this input — regression test for short-circuit bug
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        { type: 'object', properties: { value: { type: 'integer' } }, required: ['value'] },
        { type: 'object', properties: { value: { type: 'number' } }, required: ['value'] },
      ],
    });
    const result = validator.validate({ value: 42 }, schema);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_UNION_AMBIGUOUS');
  });
});

describe('SchemaValidator — anyOf validation', () => {
  const validator = new SchemaValidator(false);

  it('accepts input matching the first anyOf branch', () => {
    const schema = jsonSchemaToTypeBox({
      anyOf: [
        { type: 'object', properties: { kind: { const: 'circle' }, radius: { type: 'number' } }, required: ['kind', 'radius'] },
        { type: 'object', properties: { kind: { const: 'rect' }, width: { type: 'number' } }, required: ['kind', 'width'] },
      ],
    });
    const result = validator.validate({ kind: 'circle', radius: 5 }, schema);
    expect(result.valid).toBe(true);
  });

  it('accepts input matching only the second anyOf branch', () => {
    const schema = jsonSchemaToTypeBox({
      anyOf: [
        { type: 'object', properties: { kind: { const: 'circle' } }, required: ['kind', 'radius'] },
        { type: 'object', properties: { kind: { const: 'rect' }, width: { type: 'number' } }, required: ['kind', 'width'] },
      ],
    });
    const result = validator.validate({ kind: 'rect', width: 10 }, schema);
    expect(result.valid).toBe(true);
  });

  it('rejects input matching no anyOf branches with SCHEMA_UNION_NO_MATCH', () => {
    const schema = jsonSchemaToTypeBox({
      anyOf: [
        { type: 'object', properties: { kind: { const: 'circle' } }, required: ['kind'] },
        { type: 'object', properties: { kind: { const: 'rect' } }, required: ['kind'] },
      ],
    });
    const result = validator.validate({ kind: 'triangle' }, schema);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_UNION_NO_MATCH');
  });
});

describe('SchemaValidator — format warnings (SHOULD-level)', () => {
  it('passes structurally valid data and does not warn for valid formats', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { email: { type: 'string', format: 'email' } },
      required: ['email'],
    });
    const result = validator.validate({ email: 'alice@example.com' }, schema);
    expect(result.valid).toBe(true);
    expect(warnSpy).not.toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('passes structurally but warns for invalid format (warn_logged: true)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { ts: { type: 'string', format: 'date-time' } },
      required: ['ts'],
    });
    const result = validator.validate({ ts: 'not-a-date' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('emits format warning for invalid email inside oneOf branch (regression: union skipped _checkFormats)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        {
          type: 'object',
          properties: { contact: { type: 'string', format: 'email' } },
          required: ['contact'],
        },
      ],
    });
    const result = validator.validate({ contact: 'not-an-email' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('emits format warning for invalid format inside array items (regression: _walkFormats array coverage)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: {
        emails: { type: 'array', items: { type: 'string', format: 'email' } },
      },
      required: ['emails'],
    });
    const result = validator.validate({ emails: ['valid@example.com', 'not-an-email'] }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });
});

describe('SchemaValidator — unrecognised format is an annotation (apexe#32)', () => {
  // JSON Schema 2020-12 format-annotation vocabulary: a format the implementation
  // does not recognise is collected as an annotation and MUST NOT fail validation.
  // apcore-python (pydantic json_schema_extra) and apcore-rust (jsonschema crate,
  // Draft 2020-12 with format assertion off) both ignore it; TS used to reject.

  it('accepts a string carrying an unregistered format', () => {
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { path: { type: 'string', format: 'path' } },
      required: ['path'],
    });
    const result = validator.validate({ path: '/tmp/z' }, schema);
    expect(result.valid).toBe(true);
    expect(result.errorCode).toBeUndefined();
  });

  it('does not warn for an unrecognised format (nothing to assert)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string', format: 'bogus-xyz' } },
      required: ['a'],
    });
    const result = validator.validate({ a: '/tmp/z' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBeUndefined();
    expect(warnSpy).not.toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('accepts an unregistered format inside array items (the apexe path operand case)', () => {
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { path: { type: 'array', items: { type: 'string', format: 'path' } } },
      required: ['path'],
    });
    const result = validator.validate({ path: ['/tmp/z'] }, schema);
    expect(result.valid).toBe(true);
  });

  it('accepts an unregistered format inside a oneOf branch', () => {
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        { type: 'object', properties: { p: { type: 'string', format: 'path' } }, required: ['p'] },
      ],
    });
    const result = validator.validate({ p: '/tmp/z' }, schema);
    expect(result.valid).toBe(true);
  });

  it('validateInput returns the data instead of throwing for an unregistered format', () => {
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { p: { type: 'string', format: 'path' } },
      required: ['p'],
    });
    expect(validator.validateInput({ p: '/tmp/z' }, schema)['p']).toBe('/tmp/z');
  });

  it('accepts an unregistered format on a TypeBox schema built without the converter', () => {
    const validator = new SchemaValidator(false);
    const schema = Type.Object({ p: Type.String({ format: 'unregistered-xyz' }) });
    expect(validator.validate({ p: '/tmp/z' }, schema).valid).toBe(true);
  });

  it('keeps the format value on the converted schema (it is an annotation, not dropped)', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { p: { type: 'string', format: 'path' } },
      required: ['p'],
    }) as unknown as { properties: { p: { format?: string } } };
    expect(schema.properties.p.format).toBe('path');
  });

  it('still rejects a type mismatch on a property carrying an unregistered format', () => {
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { p: { type: 'string', format: 'path' } },
      required: ['p'],
    });
    const result = validator.validate({ p: 42 } as unknown as Record<string, unknown>, schema);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
  });

  it('a recognised format that fails still warns and still passes (SHOULD-level, unchanged)', () => {
    // The other half of apexe#32: this stays a warning. The cross-SDK contract is
    // the conformance fixture schema_hardening_formats.json — valid: true,
    // warn_logged: true — and both apcore-python and apcore-rust behave this way.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string', format: 'uri' } },
      required: ['a'],
    });
    const result = validator.validate({ a: '/tmp/z' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining("Format 'uri' validation failed at /a"));
    warnSpy.mockRestore();
  });
});

describe('SchemaValidator — errorCode in results', () => {
  const validator = new SchemaValidator(false);

  it('sets errorCode SCHEMA_VALIDATION_ERROR on plain type failure', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', properties: { x: { type: 'integer' } }, required: ['x'] });
    const result = validator.validate({ x: 'not-an-int' }, schema);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
  });

  it('throws SchemaValidationError carrying SCHEMA_VALIDATION_ERROR code via validateInput', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', properties: { x: { type: 'integer' } }, required: ['x'] });
    try {
      validator.validateInput({ x: 'not-an-int' }, schema);
      throw new Error('expected validateInput to throw');
    } catch (err) {
      expect(err).toBeInstanceOf(SchemaValidationError);
      expect((err as SchemaValidationError).code).toBe('SCHEMA_VALIDATION_ERROR');
    }
  });

  it('preserves SCHEMA_UNION_AMBIGUOUS code through validateInput (A-D-033)', () => {
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        { type: 'object', properties: { value: { type: 'integer' } }, required: ['value'] },
        { type: 'object', properties: { value: { type: 'number' } }, required: ['value'] },
      ],
    });
    try {
      validator.validateInput({ value: 42 }, schema);
      throw new Error('expected validateInput to throw');
    } catch (err) {
      expect(err).toBeInstanceOf(SchemaValidationError);
      expect((err as SchemaValidationError).code).toBe('SCHEMA_UNION_AMBIGUOUS');
    }
  });
});

describe('SchemaValidator — format warnings reach into unions and intersections', () => {
  it('warns for a format on a branch of a type array', () => {
    // `{type: ["string","null"], format: "email"}` converts to a union, and the
    // format annotation lands on the string branch. The walk used to stop at the
    // union node, so this was the one shape where a recognised format was not
    // enforced at all.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: ['string', 'null'], format: 'email' } },
      required: ['a'],
    });
    const result = new SchemaValidator(false).validate({ a: 'not-an-email' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining("Format 'email' validation failed at /a"));
    warnSpy.mockRestore();
  });

  it('does not warn when the data matches a branch carrying no format', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: ['string', 'null'], format: 'email' } },
      required: ['a'],
    });
    const result = new SchemaValidator(false).validate({ a: null }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBeUndefined();
    expect(warnSpy).not.toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('warns for a format on a schema that also carries a combinator sibling', () => {
    // `type` + `enum` converts to an intersection; the format annotation is on
    // the type-derived member.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: {
        a: { type: 'string', format: 'email', enum: ['not-an-email', 'a@b.com'] },
      },
      required: ['a'],
    });
    const result = new SchemaValidator(false).validate({ a: 'not-an-email' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining("Format 'email' validation failed at /a"));
    warnSpy.mockRestore();
  });
});

describe('SchemaValidator — format handling does not leak into the global registry', () => {
  // `FormatRegistry` is process-global and shared with the host application.
  // apcore's annotation semantics must not depend on who registered first, and
  // apcore must not leave accept-everything checkers behind for the host.
  afterEach(() => {
    FormatRegistry.Delete('email');
    FormatRegistry.Delete('apcore-leaked-fmt');
  });

  it('keeps warn-only semantics when the host registered a strict checker first', () => {
    // The failing scenario: a TypeBox host registers a strict `email` checker at
    // startup. Before the scoped override, apcore deferred to it and turned the
    // conformance fixture case into a hard SCHEMA_VALIDATION_ERROR.
    FormatRegistry.Set('email', (v) => /^[^@\s]+@[^@\s]+\.[a-z]{2,}$/i.test(v));
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const validator = new SchemaValidator(false);
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string', format: 'email' } },
      required: ['a'],
    });
    const result = validator.validate({ a: 'not-an-email' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(result.errorCode).toBeUndefined();
    warnSpy.mockRestore();
  });

  it("restores the host's own checker after validating", () => {
    const strict = (v: string) => /^[^@\s]+@[^@\s]+\.[a-z]{2,}$/i.test(v);
    FormatRegistry.Set('email', strict);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string', format: 'email' } },
      required: ['a'],
    });
    new SchemaValidator(false).validate({ a: 'not-an-email' }, schema);
    warnSpy.mockRestore();
    expect(FormatRegistry.Get('email')).toBe(strict);
    // The host's schema still rejects garbage.
    expect(Value.Check(Type.String({ format: 'email' }), 'not-an-email')).toBe(false);
  });

  it('does not register an unknown format the schema carried', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string', format: 'apcore-leaked-fmt' } },
      required: ['a'],
    });
    expect(new SchemaValidator(false).validate({ a: 'anything' }, schema).valid).toBe(true);
    expect(FormatRegistry.Has('apcore-leaked-fmt')).toBe(false);
  });

  it('does not register the formats apcore recognises', () => {
    // Constructing a validator used to register every FORMAT_VALIDATORS key
    // process-globally, so a host's `Type.String({format:'email'})` silently
    // stopped rejecting garbage.
    new SchemaValidator(false);
    expect(FormatRegistry.Has('email')).toBe(false);
  });
});

describe('SchemaValidator — enum beside a type array (apexe value-optional flag)', () => {
  // `{"type": ["string","boolean"], "enum": [...]}` is what apexe emits for
  // `ls --color[=WHEN]`. The enum must be enforced at the validation boundary,
  // or a bogus value reaches argv. apcore-rust and apcore-python both reject it.
  const schema = jsonSchemaToTypeBox({
    type: 'object',
    properties: {
      color: {
        type: ['string', 'boolean'],
        enum: ['always', 'auto', 'never'],
        description: 'colorize the output',
      },
    },
    required: ['color'],
    additionalProperties: false,
  });

  for (const coerceTypes of [true, false]) {
    it(`accepts an enum member (coerceTypes=${coerceTypes})`, () => {
      const validator = new SchemaValidator(coerceTypes);
      expect(validator.validate({ color: 'always' }, schema).valid).toBe(true);
    });

    it(`rejects a non-member string (coerceTypes=${coerceTypes})`, () => {
      const validator = new SchemaValidator(coerceTypes);
      const result = validator.validate({ color: 'bogus-not-in-enum' }, schema);
      expect(result.valid).toBe(false);
      expect(result.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
    });
  }
});

describe('validationResultToError', () => {
  it('throws when result is valid', () => {
    expect(() => validationResultToError({ valid: true, errors: [] })).toThrow(
      'Cannot convert valid result to error',
    );
  });

  it('converts invalid result to SchemaValidationError', () => {
    const result = {
      valid: false,
      errors: [{ path: '/foo', message: 'required', constraint: 'required', expected: 'string', actual: null }],
    };
    const err = validationResultToError(result);
    expect(err).toBeInstanceOf(SchemaValidationError);
    expect(err.message).toContain('Schema validation failed');
  });

  it('normalizes absent constraint and expected to null', () => {
    const result = {
      valid: false,
      errors: [{ path: '/bar', message: 'missing field' }],
    };
    const err = validationResultToError(result);
    expect(err).toBeInstanceOf(SchemaValidationError);
  });
});


describe('SchemaValidator — oneOf exclusivity survives a type sibling', () => {
  // `{type, oneOf}` converts to an intersection, which used to bury the oneOf
  // marker and silently downgrade the assertion to `anyOf` semantics.
  const branches = [
    { type: 'object', properties: { a: { type: 'string' } } },
    { type: 'object', properties: { b: { type: 'string' } } },
  ];

  for (const coerceTypes of [true, false]) {
    it(`reports SCHEMA_UNION_AMBIGUOUS when two branches match (coerceTypes=${coerceTypes})`, () => {
      const schema = jsonSchemaToTypeBox({ type: 'object', oneOf: branches });
      const result = new SchemaValidator(coerceTypes).validate({}, schema);
      expect(result.valid).toBe(false);
      expect(result.errorCode).toBe('SCHEMA_UNION_AMBIGUOUS');
    });

    it(`reports SCHEMA_UNION_NO_MATCH when no branch matches (coerceTypes=${coerceTypes})`, () => {
      const schema = jsonSchemaToTypeBox({
        type: 'object',
        oneOf: [
          { type: 'object', properties: { a: { type: 'string' } }, required: ['a'] },
          { type: 'object', properties: { b: { type: 'number' } }, required: ['b'] },
        ],
      });
      const result = new SchemaValidator(coerceTypes).validate({ c: 1 }, schema);
      expect(result.valid).toBe(false);
      expect(result.errorCode).toBe('SCHEMA_UNION_NO_MATCH');
    });
  }

  it('matches the error code a bare oneOf produces for the same data', () => {
    const bare = new SchemaValidator(false).validate({}, jsonSchemaToTypeBox({ oneOf: branches }));
    const typed = new SchemaValidator(false).validate(
      {},
      jsonSchemaToTypeBox({ type: 'object', oneOf: branches }),
    );
    expect(typed.errorCode).toBe(bare.errorCode);
  });

  it('accepts data matching exactly one branch', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      oneOf: [
        { type: 'object', properties: { a: { type: 'string' } }, required: ['a'] },
        { type: 'object', properties: { b: { type: 'number' } }, required: ['b'] },
      ],
    });
    expect(new SchemaValidator(false).validate({ a: 'x' }, schema).valid).toBe(true);
  });

  it('still enforces the type half alongside the exclusivity check', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'string',
      oneOf: [{ minLength: 2 }],
    });
    const result = new SchemaValidator(false).validate(42 as unknown as Record<string, unknown>, schema);
    expect(result.valid).toBe(false);
  });
});

describe('SchemaValidator — format warnings reach additionalProperties values', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('warns for a format on the additionalProperties schema', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      additionalProperties: { type: 'string', format: 'email' },
    });
    const result = new SchemaValidator(false).validate({ x: 'not-an-email' }, schema);
    expect(result.valid).toBe(true);
    expect(result.warnLogged).toBe(true);
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("[apcore:schema] Format 'email' validation failed at /x"),
    );
  });

  it('does not warn when the additional value satisfies the format', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      additionalProperties: { type: 'string', format: 'email' },
    });
    expect(new SchemaValidator(false).validate({ x: 'a@b.co' }, schema).valid).toBe(true);
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it('prefers the declared property schema over additionalProperties', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { id: { type: 'string' } },
      additionalProperties: { type: 'string', format: 'email' },
    });
    new SchemaValidator(false).validate({ id: 'not-an-email', extra: 'a@b.co' }, schema);
    expect(warnSpy).not.toHaveBeenCalled();
  });
});

describe('SchemaValidator — additionalProperties: false on the output side', () => {
  const schema = jsonSchemaToTypeBox({
    type: 'object',
    properties: { ok: { type: 'boolean' } },
    required: ['ok'],
    additionalProperties: false,
  });

  it('rejects an undeclared key in an output payload', () => {
    expect(() => new SchemaValidator(false).validateOutput({ ok: true, extra: 1 }, schema)).toThrow(
      SchemaValidationError,
    );
  });

  it('accepts an output payload with only declared keys', () => {
    expect(new SchemaValidator(false).validateOutput({ ok: true }, schema)).toEqual({ ok: true });
  });
});

describe('withFormatsAsAnnotations — registry restoration', () => {
  it('restores a pre-existing checker when the wrapped call throws', () => {
    const strict = (v: string): boolean => v === 'strict-only';
    FormatRegistry.Set('apcore-restore-fmt', strict);
    try {
      expect(() =>
        withFormatsAsAnnotations({ type: 'string', format: 'apcore-restore-fmt' }, () => {
          throw new Error('boom');
        }),
      ).toThrow('boom');
      expect(FormatRegistry.Get('apcore-restore-fmt')).toBe(strict);
    } finally {
      FormatRegistry.Delete('apcore-restore-fmt');
    }
  });

  it('deletes a format it introduced when the wrapped call throws', () => {
    expect(() =>
      withFormatsAsAnnotations({ type: 'string', format: 'apcore-absent-fmt' }, () => {
        throw new Error('boom');
      }),
    ).toThrow('boom');
    expect(FormatRegistry.Has('apcore-absent-fmt')).toBe(false);
  });

  it('restores the registry when a validate() call throws', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string', format: 'apcore-throwing-fmt' } },
      required: ['a'],
    });
    const validator = new SchemaValidator(false);
    // validateInput throws on a missing required property, unwinding through
    // the `finally` that restores the registry.
    expect(() => validator.validateInput({}, schema)).toThrow(SchemaValidationError);
    expect(FormatRegistry.Has('apcore-throwing-fmt')).toBe(false);
  });
});

describe('collectSchemaFormats — only schema positions carry a format', () => {
  it('ignores a format key inside default and examples values', () => {
    const collected = collectSchemaFormats({
      type: 'object',
      properties: {
        a: {
          type: 'string',
          default: { format: 'i-am-data' },
          examples: [{ format: 'also-data' }],
        },
      },
    });
    expect([...collected]).toEqual([]);
  });

  it('ignores a format key inside const and enum values', () => {
    const collected = collectSchemaFormats({
      type: 'object',
      const: { format: 'const-data' },
      enum: [{ format: 'enum-data' }],
    });
    expect([...collected]).toEqual([]);
  });

  it('still collects a format from a real schema position', () => {
    const collected = collectSchemaFormats({
      type: 'object',
      properties: { a: { type: 'string', format: 'email' } },
      additionalProperties: { type: 'string', format: 'uuid' },
      items: { type: 'string', format: 'date' },
    });
    expect([...collected].sort()).toEqual(['date', 'email', 'uuid']);
  });

  it('collects from a property whose name collides with a data keyword', () => {
    const collected = collectSchemaFormats({
      type: 'object',
      properties: { default: { type: 'string', format: 'email' } },
    });
    expect([...collected]).toEqual(['email']);
  });
});

describe('FORMAT_VALIDATORS — immutable', () => {
  it('cannot be overwritten by an importer', () => {
    const target = FORMAT_VALIDATORS as Record<string, (v: string) => boolean>;
    expect(() => {
      target['email'] = () => true;
    }).toThrow(TypeError);
    expect(FORMAT_VALIDATORS['email']?.('not-an-email')).toBe(false);
  });

  it('cannot gain a new entry', () => {
    const target = FORMAT_VALIDATORS as Record<string, (v: string) => boolean>;
    expect(() => {
      target['apcore-injected'] = () => true;
    }).toThrow(TypeError);
    expect(FORMAT_VALIDATORS['apcore-injected']).toBeUndefined();
  });
});

describe('SchemaValidator — anyOf error code survives a type sibling', () => {
  const branches = [
    { type: 'object', properties: { a: { type: 'string' } }, required: ['a'] },
    { type: 'object', properties: { b: { type: 'number' } }, required: ['b'] },
  ];

  for (const coerceTypes of [true, false]) {
    it(`reports SCHEMA_UNION_NO_MATCH when no branch matches (coerceTypes=${coerceTypes})`, () => {
      const schema = jsonSchemaToTypeBox({ type: 'object', anyOf: branches });
      const result = new SchemaValidator(coerceTypes).validate({ c: 1 }, schema);
      expect(result.valid).toBe(false);
      expect(result.errorCode).toBe('SCHEMA_UNION_NO_MATCH');
    });
  }

  it('matches the error code a bare anyOf produces for the same data', () => {
    const bare = new SchemaValidator(false).validate({ c: 1 }, jsonSchemaToTypeBox({ anyOf: branches }));
    const typed = new SchemaValidator(false).validate(
      { c: 1 },
      jsonSchemaToTypeBox({ type: 'object', anyOf: branches }),
    );
    expect(bare.errorCode).toBe('SCHEMA_UNION_NO_MATCH');
    expect(typed.errorCode).toBe(bare.errorCode);
  });

  it('accepts data matching several branches — anyOf has no exclusivity rule', () => {
    // The oneOf counting must not be copied here: two matches is valid.
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      anyOf: [{ type: 'object' }, { type: 'object', properties: { a: { type: 'string' } } }],
    });
    expect(new SchemaValidator(false).validate({}, schema).valid).toBe(true);
    expect(new SchemaValidator(false).validate({ a: 'x' }, schema).valid).toBe(true);
  });

  it('never reports SCHEMA_UNION_AMBIGUOUS for an anyOf', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      anyOf: [{ type: 'object' }, { type: 'object' }],
    });
    expect(new SchemaValidator(false).validate({}, schema).errorCode).toBeUndefined();
  });

  it('still enforces the type half alongside the anyOf check', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'string',
      anyOf: [{ minLength: 1 }],
    });
    const result = new SchemaValidator(false).validate(42 as unknown as Record<string, unknown>, schema);
    expect(result.valid).toBe(false);
  });

  it('leaves a hand-written TypeBox union on the anyOf path', () => {
    // No marker, so the unmarked-union fallback must still apply.
    const schema = Type.Union([Type.String(), Type.Number()]);
    const result = new SchemaValidator(false).validate(
      true as unknown as Record<string, unknown>,
      schema,
    );
    expect(result.errorCode).toBe('SCHEMA_UNION_NO_MATCH');
  });
});

describe('SchemaValidator — combinators without a dedicated union code', () => {
  // allOf / not / enum / const never had a SCHEMA_UNION_* code, bare or beside
  // a `type`, so there is nothing to degrade. Locked so it cannot drift.
  const cases: Array<[string, Record<string, unknown>, Record<string, unknown>]> = [
    ['allOf', { allOf: [{ type: 'object', properties: { a: { type: 'string' } }, required: ['a'] }] }, { c: 1 }],
    ['not', { not: { type: 'object', properties: { a: {} }, required: ['a'] } }, { a: 1 }],
    ['enum', { enum: [{ a: 1 }] }, { a: 2 }],
  ];

  for (const [keyword, combinator, data] of cases) {
    it(`reports SCHEMA_VALIDATION_ERROR for ${keyword} with and without a type sibling`, () => {
      const validator = new SchemaValidator(false);
      const bare = validator.validate(data, jsonSchemaToTypeBox(combinator));
      const typed = validator.validate(data, jsonSchemaToTypeBox({ type: 'object', ...combinator }));
      expect(bare.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
      expect(typed.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
    });
  }
});

// ---------------------------------------------------------------------------
// `oneOf` exclusivity is location-independent (PROTOCOL_SPEC §4.15.1)
// ---------------------------------------------------------------------------

describe('SchemaValidator nested oneOf exclusivity', () => {
  const validator = new SchemaValidator(false);

  it('rejects a nested oneOf whose branches both match', () => {
    // Regression: `oneOf` converted to a plain TypeBox `Type.Union`, which has
    // `anyOf` semantics, and the validator only looked for the union marker at
    // the root and along `allOf`. A `oneOf` inside `properties` therefore
    // accepted 3 even though both `integer` and `number` claim it — apcore-python
    // and apcore-rust both reject it.
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      required: ['a'],
      properties: { a: { oneOf: [{ type: 'integer' }, { type: 'number' }] } },
    });
    expect(validator.validate({ a: 3 }, schema).valid).toBe(false);
    expect(validator.validate({ a: 3.5 }, schema).valid).toBe(true);
    expect(validator.validate({ a: 'x' }, schema).valid).toBe(false);
  });

  it('rejects an ambiguous oneOf inside array items', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      required: ['a'],
      properties: {
        a: { type: 'array', items: { oneOf: [{ type: 'integer' }, { type: 'number' }] } },
      },
    });
    expect(validator.validate({ a: [3] }, schema).valid).toBe(false);
    expect(validator.validate({ a: [3.5] }, schema).valid).toBe(true);
  });

  it('leaves nested anyOf accepting several simultaneous matches', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      required: ['a'],
      properties: { a: { anyOf: [{ type: 'integer' }, { type: 'number' }] } },
    });
    expect(validator.validate({ a: 3 }, schema).valid).toBe(true);
  });

  it('keeps reporting SCHEMA_UNION_AMBIGUOUS for a root-level oneOf', () => {
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        { type: 'object', properties: { k: { type: 'string' } } },
        { type: 'object', properties: { k: { type: 'string' } } },
      ],
    });
    const result = validator.validate({ k: 'x' }, schema);
    expect(result.valid).toBe(false);
    expect(result.errorCode).toBe('SCHEMA_UNION_AMBIGUOUS');
  });
});

describe('SchemaValidator uniqueItems over non-scalar items', () => {
  const validator = new SchemaValidator(false);
  const schema = jsonSchemaToTypeBox({
    type: 'object',
    required: ['a'],
    properties: {
      a: {
        type: 'array',
        uniqueItems: true,
        items: { type: 'object', properties: { k: { type: 'string' }, j: { type: 'integer' } } },
      },
    },
  });

  it('rejects duplicate objects and accepts distinct ones', () => {
    expect(validator.validate({ a: [{ k: 'x' }, { k: 'x' }] }, schema).valid).toBe(false);
    expect(validator.validate({ a: [{ k: 'x' }, { k: 'y' }] }, schema).valid).toBe(true);
  });

  it('ignores key order when comparing members', () => {
    expect(
      validator.validate({ a: [{ k: 'x', j: 1 }, { j: 1, k: 'x' }] }, schema).valid,
    ).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// The library-level coercion knob — TYPE_MAPPING §11 (apcore#95)
// ---------------------------------------------------------------------------
//
// Offering the knob stays a MAY; what it coerces became a MUST at spec v1.12.0.
// Before that this SDK carried a twelve-spelling case-insensitive boolean
// dialect ported from apcore-rust during apcore#93 (`yes`/`no`, `on`/`off`,
// `y`/`n`, `t`/`f`, `1`/`0`) that no document described. §11 caps the set at
// JSON's own two literals — it does not delete the feature, it bounds it.
//
// Everything below goes through the same path the conformance driver uses:
// `jsonSchemaToTypeBox` then `new SchemaValidator(true)`. Asserting against a
// hand-built `Type.Boolean()` would test a schema shape the driver never sees.
describe('SchemaValidator coercion knob — accepted set (TYPE_MAPPING §11)', () => {
  const boolSchema = jsonSchemaToTypeBox({
    type: 'object',
    properties: { flag: { type: 'boolean' } },
    required: ['flag'],
  });
  const intSchema = jsonSchemaToTypeBox({
    type: 'object',
    properties: { count: { type: 'integer' } },
    required: ['count'],
  });
  const numSchema = jsonSchemaToTypeBox({
    type: 'object',
    properties: { amount: { type: 'number' } },
    required: ['amount'],
  });

  const coercing = new SchemaValidator(true);
  const strict = new SchemaValidator(false);

  describe('string → boolean accepts exactly the two JSON literals', () => {
    for (const [input, expected] of [
      ['true', true],
      ['false', false],
    ] as const) {
      it(`coerces ${JSON.stringify(input)} to ${expected}`, () => {
        expect(coercing.validate({ flag: input }, boolSchema).valid).toBe(true);
        // The VALUE, not just the verdict: a validator that accepted the string
        // without converting it would pass the check above.
        expect(coercing.validateInput({ flag: input }, boolSchema)['flag']).toBe(expected);
      });

      it(`still rejects ${JSON.stringify(input)} with the knob off`, () => {
        // R5: the strict path — the one the module-invocation boundary uses —
        // rejects every one of these. The knob is a separate path, and the two
        // must not silently disagree about which one they are on.
        expect(strict.validate({ flag: input }, boolSchema).valid).toBe(false);
      });
    }
  });

  describe('string → boolean rejects every other spelling', () => {
    // The twelve-spelling dialect this SDK shipped through 0.27.0, plus the
    // casings its `toLowerCase()` used to fold away. `'0'` is the sharpest:
    // R5 makes the NUMBER 0 a MUST-reject for boolean, so accepting the string
    // put two paths of this SDK on opposite sides of one value.
    const removed = [
      'yes', 'no', 'on', 'off', 'y', 'n', 't', 'f', '1', '0',
      'True', 'TRUE', 'False', 'FALSE', 'tRuE',
      '', ' true ', 'true\n',
    ];
    for (const input of removed) {
      it(`rejects ${JSON.stringify(input)}`, () => {
        const result = coercing.validate({ flag: input }, boolSchema);
        expect(
          result.valid,
          `${JSON.stringify(input)} coerced to a boolean; §11 caps the set at "true"/"false"`,
        ).toBe(false);
        expect(result.errorCode).toBe('SCHEMA_VALIDATION_ERROR');
      });
    }
  });

  describe('non-strings are never coerced toward boolean', () => {
    for (const input of [1, 0, 1.0, null, [], {}]) {
      it(`rejects ${JSON.stringify(input)} for a declared boolean`, () => {
        expect(coercing.validate({ flag: input }, boolSchema).valid).toBe(false);
      });
    }
  });

  describe('string → integer', () => {
    for (const [input, expected] of [
      ['42', 42],
      ['-7', -7],
      ['0', 0],
      ['42.0', 42],
    ] as const) {
      it(`coerces ${JSON.stringify(input)} to ${expected}`, () => {
        expect(coercing.validate({ count: input }, intSchema).valid).toBe(true);
        expect(coercing.validateInput({ count: input }, intSchema)['count']).toBe(expected);
      });
    }

    for (const input of ['3.14', '-0.5', 'abc', '', 'true', '1e400', 'NaN']) {
      it(`rejects ${JSON.stringify(input)}`, () => {
        // A lossy conversion is refused, not truncated: `Value.Convert` would
        // turn "3.14" into 3 and silently change what the caller meant.
        expect(coercing.validate({ count: input }, intSchema).valid).toBe(false);
      });
    }
  });

  describe('string → number', () => {
    for (const [input, expected] of [
      ['1.5', 1.5],
      ['42', 42],
      ['-0.5', -0.5],
    ] as const) {
      it(`coerces ${JSON.stringify(input)} to ${expected}`, () => {
        expect(coercing.validate({ amount: input }, numSchema).valid).toBe(true);
        expect(coercing.validateInput({ amount: input }, numSchema)['amount']).toBe(expected);
      });
    }

    for (const input of ['abc', '', 'true', 'Infinity']) {
      it(`rejects ${JSON.stringify(input)}`, () => {
        expect(coercing.validate({ amount: input }, numSchema).valid).toBe(false);
      });
    }
  });

  it('never coerces toward string', () => {
    const strSchema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { name: { type: 'string' } },
      required: ['name'],
    });
    expect(coercing.validate({ name: 42 }, strSchema).valid).toBe(false);
    expect(coercing.validate({ name: true }, strSchema).valid).toBe(false);
  });

  it('defaults to no coercion, matching the module-invocation boundary', () => {
    // TYPE_MAPPING §17.3. `new SchemaValidator()` with no argument must be the
    // strict validator; a coercing default is how apcore-rust ended up with a
    // validator that disagreed with its own executor.
    expect(new SchemaValidator().validate({ flag: 'true' }, boolSchema).valid).toBe(false);
    expect(new SchemaValidator().validate({ count: '42' }, intSchema).valid).toBe(false);
  });
});
