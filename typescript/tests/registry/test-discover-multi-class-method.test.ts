/**
 * D-15: Registry must expose `discoverMultiClass` as an instance method,
 * mirroring Python's `Registry.discover_multi_class` and the Rust trait.
 *
 * The free function is preserved as `_discoverMultiClass` (internal).
 */

import { describe, it, expect, beforeEach, afterEach, vi, type MockInstance } from 'vitest';
import {
  Registry,
  _resetMultiClassEnabledDeprecationWarned,
} from '../../src/registry/registry.js';
import { _discoverMultiClass, discoverMultiClass, classNameToSegment } from '../../src/registry/multi-class.js';
import { IdTooLongError, InvalidSegmentError, ModuleIdConflictError } from '../../src/errors.js';

describe('Registry.discoverMultiClass', () => {
  it('exists as an instance method', () => {
    const registry = new Registry();
    expect(typeof (registry as unknown as { discoverMultiClass: unknown }).discoverMultiClass).toBe('function');
  });

  it('returns the same result as the free function (per-class multiClass marker)', () => {
    const registry = new Registry();
    const filePath = 'extensions/email/sender.ts';
    const classes = [
      { name: 'EmailSender', implementsModule: true, multiClass: true },
      { name: 'EmailFormatter', implementsModule: true, multiClass: true },
    ];

    const free = _discoverMultiClass(filePath, classes, 'extensions', true);
    // Canonical 3-arg call shape (D-06): no multiClassEnabled.
    const method = registry.discoverMultiClass(filePath, classes, 'extensions');
    expect(method).toEqual(free);
  });

  it('3-arg form: per-class `multiClass: true` enables multi-class derivation', () => {
    const registry = new Registry();
    const result = registry.discoverMultiClass(
      'extensions/math/math_ops.ts',
      [
        { name: 'Addition', implementsModule: true, multiClass: true },
        { name: 'Subtraction', implementsModule: true, multiClass: true },
      ],
      'extensions',
    );
    expect(result).toEqual([
      { moduleId: 'math.math_ops.addition', className: 'Addition' },
      { moduleId: 'math.math_ops.subtraction', className: 'Subtraction' },
    ]);
  });

  it('3-arg form: no class with `multiClass` falls back to whole-file mode', () => {
    const registry = new Registry();
    const result = registry.discoverMultiClass(
      'extensions/email/sender.ts',
      [
        { name: 'A', implementsModule: true },
        { name: 'B', implementsModule: true },
      ],
      'extensions',
    );
    expect(result).toEqual([{ moduleId: 'email.sender', className: 'A' }]);
  });

  it('3-arg form: extensionsRoot is optional and defaults to "extensions"', () => {
    const registry = new Registry();
    const result = registry.discoverMultiClass(
      'extensions/email/sender.ts',
      [{ name: 'Sender', implementsModule: true, multiClass: true }],
    );
    expect(result).toEqual([{ moduleId: 'email.sender', className: 'Sender' }]);
  });

  it('preserves single-class identity guarantee', () => {
    const registry = new Registry();
    const result = registry.discoverMultiClass(
      'extensions/email/sender.ts',
      [{ name: 'Sender', implementsModule: true, multiClass: true }],
      'extensions',
    );
    expect(result).toEqual([{ moduleId: 'email.sender', className: 'Sender' }]);
  });

  it('public free function is still exported for backwards compatibility', () => {
    expect(typeof discoverMultiClass).toBe('function');
  });
});

describe('Registry.discoverMultiClass — deprecated 4-arg overload (D-06)', () => {
  let warnSpy: MockInstance<typeof console.warn>;

  beforeEach(() => {
    _resetMultiClassEnabledDeprecationWarned();
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
    _resetMultiClassEnabledDeprecationWarned();
  });

  it('still accepts the 4-arg call shape and emits a deprecation warning', () => {
    const registry = new Registry();
    const result = registry.discoverMultiClass(
      'extensions/email/sender.ts',
      [{ name: 'Sender', implementsModule: true, multiClass: true }],
      'extensions',
      true,
    );
    expect(result).toEqual([{ moduleId: 'email.sender', className: 'Sender' }]);
    const deprecationWarn = warnSpy.mock.calls.find((call) =>
      typeof call[0] === 'string' && call[0].includes('DEPRECATION') && call[0].includes('multiClassEnabled'),
    );
    expect(deprecationWarn).toBeDefined();
  });

  it('deprecation warning fires only once per process', () => {
    const registry = new Registry();
    const args = [
      'extensions/email/sender.ts',
      [{ name: 'Sender', implementsModule: true, multiClass: true }],
      'extensions',
      true,
    ] as const;
    registry.discoverMultiClass(...args);
    registry.discoverMultiClass(...args);
    registry.discoverMultiClass(...args);

    const deprecationWarnings = warnSpy.mock.calls.filter((call) =>
      typeof call[0] === 'string' && call[0].includes('DEPRECATION') && call[0].includes('multiClassEnabled'),
    );
    expect(deprecationWarnings).toHaveLength(1);
  });

  it('the 4th `multiClassEnabled` argument is functionally inert — the per-class marker is the source of truth', () => {
    const registry = new Registry();

    // multiClassEnabled=false but classes are marked multiClass:true → still multi-class mode.
    const enabledViaMarker = registry.discoverMultiClass(
      'extensions/math/math_ops.ts',
      [
        { name: 'Addition', implementsModule: true, multiClass: true },
        { name: 'Subtraction', implementsModule: true, multiClass: true },
      ],
      'extensions',
      false,
    );
    expect(enabledViaMarker).toEqual([
      { moduleId: 'math.math_ops.addition', className: 'Addition' },
      { moduleId: 'math.math_ops.subtraction', className: 'Subtraction' },
    ]);

    // multiClassEnabled=true but no class is marked → whole-file mode wins.
    _resetMultiClassEnabledDeprecationWarned();
    const disabledByMarker = registry.discoverMultiClass(
      'extensions/email/sender.ts',
      [
        { name: 'A', implementsModule: true },
        { name: 'B', implementsModule: true },
      ],
      'extensions',
      true,
    );
    expect(disabledByMarker).toEqual([{ moduleId: 'email.sender', className: 'A' }]);
  });
});

describe('discoverMultiClass — edge branches', () => {
  it('returns empty array when no classes implement Module', () => {
    const result = discoverMultiClass(
      'extensions/foo/bar.ts',
      [{ name: 'Plain', implementsModule: false }],
      'extensions',
      true,
    );
    expect(result).toEqual([]);
  });

  it('non-multi-class mode returns the bare base_id even with multiple qualifying classes', () => {
    const result = discoverMultiClass(
      'extensions/email/sender.ts',
      [
        { name: 'A', implementsModule: true },
        { name: 'B', implementsModule: true },
      ],
      'extensions',
      false,
    );
    expect(result).toEqual([{ moduleId: 'email.sender', className: 'A' }]);
  });

  it('returns bare base_id for a single class regardless of its name (A-D-20)', () => {
    // Single-class identity guarantee: no class segment is derived or validated,
    // so a class name that would snake_case to an invalid segment is irrelevant.
    // Matches Python multi_class.py:143 / Rust derive_module_ids.
    const result = discoverMultiClass(
      'extensions/email/sender.ts',
      [{ name: '__', implementsModule: true }],
      'extensions',
      true,
    );
    expect(result).toEqual([{ moduleId: 'email.sender', className: '__' }]);
  });

  // Spec v1.50.0 D-107: the per-class `multiClass` marker is the only
  // multi-class opt-in. These three cases used to reach the derivation loop
  // through the free function's `multiClassEnabled` boolean, which is now
  // ignored, so the descriptors carry the marker instead.
  it('throws ModuleIdConflictError when two classes derive the same segment', () => {
    expect(() =>
      discoverMultiClass(
        'extensions/email/sender.ts',
        [
          { name: 'EmailSender', implementsModule: true, multiClass: true },
          { name: 'Email_Sender', implementsModule: true, multiClass: true },
        ],
        'extensions',
      ),
    ).toThrow(ModuleIdConflictError);
  });

  it('throws InvalidSegmentError in the multi-class loop when the file path produces a non-canonical baseId', () => {
    // baseId="123bad.sender" — starts with a digit, so the composed moduleId
    // fails CANONICAL_ID_RE even though the per-class segment passes SEGMENT_RE.
    expect(() =>
      discoverMultiClass(
        'extensions/123bad/sender.ts',
        [
          { name: 'Alpha', implementsModule: true, multiClass: true },
          { name: 'Beta', implementsModule: true, multiClass: true },
        ],
        'extensions',
      ),
    ).toThrow(InvalidSegmentError);
  });

  it('throws IdTooLongError in the multi-class loop when the composed id exceeds 192 chars', () => {
    const longDir = 'a'.repeat(200);
    expect(() =>
      discoverMultiClass(
        `extensions/${longDir}/file.ts`,
        [
          { name: 'Alpha', implementsModule: true, multiClass: true },
          { name: 'Beta', implementsModule: true, multiClass: true },
        ],
        'extensions',
      ),
    ).toThrow(IdTooLongError);
  });

  it('classNameToSegment handles the documented transitions', () => {
    expect(classNameToSegment('HTTPSender')).toBe('http_sender');
    expect(classNameToSegment('MathOps')).toBe('math_ops');
    expect(classNameToSegment('Has-Dash')).toBe('has_dash');
    expect(classNameToSegment('_LeadingUnderscore_')).toBe('leading_underscore');
  });
});
