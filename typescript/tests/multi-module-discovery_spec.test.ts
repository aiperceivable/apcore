/**
 * Spec-traced contract tests for the multi-module-discovery feature (TypeScript SDK).
 *
 * MIRRORS the canonical Python suite
 *   apcore-python/tests/test_multi_module_discovery_spec.py
 * Each `it(...)` name carries the SAME clause-id VERBATIM so a cross-language
 * diff can match rows by exact clause-id.
 *
 * Contract under test: `Registry.discoverMultiClass` and the underlying
 * free-function helper `discoverMultiClass` from `../src/registry/multi-class.js`.
 *
 * Cross-language divergence (feature spec D11-004): TypeScript does NOT import
 * files at scan time. The method takes pre-resolved `ClassDescriptor[]` from the
 * caller's scanner — `(filePath, classes, extensionsRoot, multiClassEnabled?)`.
 * Opt-in is per-class via `multiClass: true` (apcore decision-log D-06); the
 * `multiClassEnabled` boolean param is retained but functionally inert.
 *
 * Framework: vitest. Tests-only — src/ is never modified. Several clauses
 * therefore assert TS-actual behaviour that diverges from the Python intent;
 * those divergences are reported by the harness, not patched here.
 */

import { describe, it, expect } from 'vitest';
import {
  IdTooLongError,
  InvalidSegmentError,
  ModuleIdConflictError,
} from '../src/errors.js';
import { Registry } from '../src/registry/registry.js';
import {
  classNameToSegment,
  discoverMultiClass,
  type ClassDescriptor,
  type MultiClassEntry,
} from '../src/registry/multi-class.js';

// ---------------------------------------------------------------------------
// Canonical ID grammar (PROTOCOL_SPEC §2.7), mirrored from the feature spec.
// ---------------------------------------------------------------------------
const CANONICAL_ID_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$/;

// MAX_MODULE_ID_LEN is module-private inside multi-class.ts (not exported); the
// public constant MAX_MODULE_ID_LENGTH carries the same value (192).
const MAX_MODULE_ID_LEN = 192;

// ---------------------------------------------------------------------------
// Descriptor helpers. Unlike Python (which imports a .py file at scan time),
// TS callers pre-resolve ClassDescriptors. `multiClass: true` is the per-class
// opt-in (D-06). We default it on so the multi-class derivation path is taken.
// ---------------------------------------------------------------------------
function descriptors(...names: string[]): ClassDescriptor[] {
  return names.map((name) => ({ name, implementsModule: true, multiClass: true }));
}

describe('multi-module-discovery spec contract', () => {
  // =========================================================================
  // RETURN / single-class identity guarantee
  // =========================================================================
  it('multi_module_discovery.discover_multi_class.return.single_class_identity: single class -> bare base_id (A-D-20)', () => {
    // A single qualifying class ALWAYS yields the bare base_id (no class
    // segment), even when the class name differs from the file stem. This
    // matches Python multi_class.py:143 and Rust derive_module_ids.
    // "Addition" != stem "math_ops", yet the result must be the bare base_id.
    const enabled = discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition'),
      'extensions',
      true,
    );
    expect(enabled).toHaveLength(1);
    expect(enabled[0].moduleId).toBe('math.math_ops');
    expect(enabled[0].className).toBe('Addition');

    // Same guarantee with multi-class mode disabled.
    const disabled = discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition'),
      'extensions',
      false,
    );
    expect(disabled).toHaveLength(1);
    expect(disabled[0].moduleId).toBe('math.math_ops');
    expect(disabled[0].className).toBe('Addition');
  });

  it('multi_module_discovery.discover_multi_class.return.two_class_distinct_ids: two classes -> distinct suffixed IDs', () => {
    const result = discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition', 'Subtraction'),
      'extensions',
      true,
    );
    const ids = result.map((e) => e.moduleId).sort();
    expect(ids).toEqual(['math.math_ops.addition', 'math.math_ops.subtraction']);
  });

  it('multi_module_discovery.discover_multi_class.return.pairs_shape: entries are {moduleId, className}', () => {
    const result = discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition', 'Subtraction'),
      'extensions',
      true,
    );
    for (const entry of result) {
      expect(typeof entry.moduleId).toBe('string');
      expect(typeof entry.className).toBe('string');
      // Python returns (module_id, class_ref) tuples; TS returns MultiClassEntry
      // objects whose className is a string (no live class reference at runtime).
      expect(Object.keys(entry).sort()).toEqual(['className', 'moduleId']);
    }
  });

  // =========================================================================
  // INPUT contracts
  // =========================================================================
  it.skip('multi_module_discovery.discover_multi_class.input.file_path.nonexistent: missing symbol — TS never reads the file (no ModuleLoadError path)', () => {
    // Python imports the file at scan time, so a nonexistent path raises
    // ModuleLoadError. TS takes pre-resolved descriptors and never touches the
    // filesystem; there is no nonexistent-file error path. Contract gap.
    expect(true).toBe(false);
  });

  it('multi_module_discovery.discover_multi_class.input.extensions_root.default: default root keeps dir context', () => {
    // Default extensionsRoot is 'extensions'.
    const result = discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition', 'Subtraction'),
      undefined as unknown as string, // rely on default
      true,
    );
    const ids = result.map((e) => e.moduleId).sort();
    expect(ids).toEqual(['math.math_ops.addition', 'math.math_ops.subtraction']);
  });

  it('multi_module_discovery.discover_multi_class.input.extensions_root.custom: custom root drives base id', () => {
    const result = discoverMultiClass(
      'plugins/math/math_ops.ts',
      descriptors('Addition', 'Subtraction'),
      'plugins',
      true,
    );
    const ids = result.map((e) => e.moduleId).sort();
    expect(ids).toEqual(['math.math_ops.addition', 'math.math_ops.subtraction']);
  });

  it.skip('multi_module_discovery.discover_multi_class.input.pre_approval_hook.reject: missing symbol — pre_approval_hook is Python-only', () => {
    // Feature spec: pre_approval_hook is Python-only (TS parses static AST and
    // never imports code at scan time). No such parameter in the TS surface.
    expect(true).toBe(false);
  });

  it.skip('multi_module_discovery.discover_multi_class.input.pre_approval_hook.allow: missing symbol — pre_approval_hook is Python-only', () => {
    expect(true).toBe(false);
  });

  // =========================================================================
  // ERROR contracts (assert .code exactly)
  // =========================================================================
  it('multi_module_discovery.discover_multi_class.error.MODULE_ID_CONFLICT: duplicate segment raises with code + details', () => {
    // MyModule and My_Module both produce segment "my_module".
    let caught: unknown;
    try {
      discoverMultiClass(
        'extensions/pkg/dup.ts',
        descriptors('MyModule', 'My_Module'),
        'extensions',
        true,
      );
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(ModuleIdConflictError);
    const e = caught as ModuleIdConflictError;
    expect(e.code).toBe('MODULE_ID_CONFLICT');
    expect(e.details.conflictingSegment).toBe('my_module');
    expect(new Set(e.details.classNames as string[])).toEqual(new Set(['MyModule', 'My_Module']));
    expect(e.details.filePath).toBeDefined();
  });

  it('multi_module_discovery.discover_multi_class.error.INVALID_SEGMENT: digit-leading segment raises with code', () => {
    // "_3D" snake_cases to a segment that violates ^[a-z][a-z0-9_]*$.
    let caught: unknown;
    try {
      discoverMultiClass(
        'extensions/pkg/bad.ts',
        descriptors('Addition', '_3D'),
        'extensions',
        true,
      );
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(InvalidSegmentError);
    expect((caught as InvalidSegmentError).code).toBe('INVALID_SEGMENT');
  });

  it('multi_module_discovery.discover_multi_class.error.ID_TOO_LONG: over-192 module_id raises with code', () => {
    // Force the full module_id over MAX_MODULE_ID_LEN (192) via a long name.
    const longName = 'A' + 'b'.repeat(MAX_MODULE_ID_LEN + 10);
    let caught: unknown;
    try {
      discoverMultiClass(
        'extensions/pkg/long.ts',
        descriptors(longName, 'Subtraction'),
        'extensions',
        true,
      );
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(IdTooLongError);
    expect((caught as IdTooLongError).code).toBe('ID_TOO_LONG');
  });

  // =========================================================================
  // PROPERTY: snake_case derivation correctness (pure helper)
  // =========================================================================
  it('multi_module_discovery.discover_multi_class.property.snake_case_conversion: classNameToSegment matches spec table', () => {
    const cases: Array<[string, string]> = [
      ['Addition', 'addition'],
      ['MathOps', 'math_ops'],
      ['HTTPSender', 'http_sender'],
      ['MyModule_V2', 'my_module_v2'],
    ];
    for (const [name, expected] of cases) {
      expect(classNameToSegment(name)).toBe(expected);
    }
  });

  it('multi_module_discovery.discover_multi_class.property.grammar_conformance: all derived IDs match canonical grammar', () => {
    const result = discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition', 'Subtraction', 'Multiplication'),
      'extensions',
      true,
    );
    expect(result).toHaveLength(3);
    for (const entry of result) {
      expect(CANONICAL_ID_RE.test(entry.moduleId)).toBe(true);
    }
  });

  it('multi_module_discovery.discover_multi_class.property.pure: pure=false — but TS reads no FS; determined by descriptor inputs only', () => {
    // Python proves impurity by deleting the file (FS dependency). TS never
    // reads the FS, so the function is effectively pure over its (filePath,
    // descriptors) inputs. We assert the observable TS contract: identical
    // inputs -> identical outputs (the FS-impurity intent does not port).
    const args = ['extensions/math/math_ops.ts', descriptors('Addition', 'Subtraction'), 'extensions', true] as const;
    const a = discoverMultiClass(...args);
    const b = discoverMultiClass('extensions/math/math_ops.ts', descriptors('Addition', 'Subtraction'), 'extensions', true);
    expect(a.map((e) => e.moduleId).sort()).toEqual(b.map((e) => e.moduleId).sort());
  });

  it('multi_module_discovery.discover_multi_class.property.idempotent: repeated calls yield same IDs', () => {
    const call = () =>
      discoverMultiClass(
        'extensions/math/math_ops.ts',
        descriptors('Addition', 'Subtraction'),
        'extensions',
        true,
      )
        .map((e) => e.moduleId)
        .sort();
    const ids1 = call();
    const ids2 = call();
    const ids3 = call();
    expect(ids1).toEqual(ids2);
    expect(ids2).toEqual(ids3);
    expect(ids1).toEqual(['math.math_ops.addition', 'math.math_ops.subtraction']);
  });

  it('multi_module_discovery.discover_multi_class.property.async: synchronous (non-Promise) return', () => {
    // async: false — the method/free-function returns a plain array, not a Promise.
    const registry = new Registry();
    const result = registry.discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition'),
      'extensions',
    );
    expect(result).not.toBeInstanceOf(Promise);
    expect(Array.isArray(result)).toBe(true);
    expect(result).toHaveLength(1);
  });

  it('multi_module_discovery.discover_multi_class.property.thread_safe: >=8 concurrent discoveries agree', async () => {
    // thread_safe: true — >=8 concurrent discoveries must all agree and not
    // corrupt one another (each scans an independent descriptor set).
    const tasks = Array.from({ length: 8 }, (_, i) =>
      Promise.resolve().then(() =>
        discoverMultiClass(
          `extensions/math/ops_${i}.ts`,
          descriptors('Addition', 'Subtraction'),
          'extensions',
          true,
        )
          .map((e) => e.moduleId)
          .sort(),
      ),
    );
    const results = await Promise.all(tasks);
    results.forEach((ids, i) => {
      expect(ids).toEqual([`math.ops_${i}.addition`, `math.ops_${i}.subtraction`]);
    });
  });

  // =========================================================================
  // SIDE EFFECTS — observable via public API
  // =========================================================================
  it('multi_module_discovery.discover_multi_class.side_effect.1.discovery_does_not_register: registry unchanged', () => {
    // discoverMultiClass returns candidate pairs but MUST NOT itself register
    // modules into the registry.
    const registry = new Registry();
    const before = new Set(registry.moduleIds);
    registry.discoverMultiClass(
      'extensions/math/math_ops.ts',
      descriptors('Addition', 'Subtraction'),
      'extensions',
    );
    const after = new Set(registry.moduleIds);
    expect(after).toEqual(before);
  });

  it('multi_module_discovery.discover_multi_class.side_effect.2.conflict_aborts_whole_file: no partial results escape on conflict', () => {
    // On conflict the whole file is aborted: the exception propagates before
    // any results escape.
    const captured: MultiClassEntry[] = [];
    let threw = false;
    try {
      captured.push(
        ...discoverMultiClass(
          'extensions/pkg/dup.ts',
          descriptors('Addition', 'MyModule', 'My_Module'),
          'extensions',
          true,
        ),
      );
    } catch (err) {
      threw = true;
      expect(err).toBeInstanceOf(ModuleIdConflictError);
    }
    expect(threw).toBe(true);
    expect(captured).toEqual([]);
  });
});
