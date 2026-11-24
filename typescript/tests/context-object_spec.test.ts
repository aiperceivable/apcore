/**
 * Spec-traced contract tests for the Context Object feature (TypeScript SDK).
 *
 * Mirrors the canonical Python suite
 * apcore-python/tests/test_context_object_spec.py clause-for-clause.
 * Source spec: apcore/docs/features/context-object.md
 * Contract under test: `ContextKey<T>` (the only `## Contract:` block in the spec).
 *
 * Each `it(...)` name carries the verbatim clause id formatted as
 * `context_object.<method>.<kind>.<detail>` so cross-language diffs line up.
 *
 * framework: vitest. Property tests that the spec marks `async: false` stay
 * synchronous; the thread_safe clause exercises concurrency via `Promise.all`
 * per the shared contract-spec rules.
 *
 * TESTS ONLY — no production source is modified here.
 */

import { describe, it, expect } from 'vitest';
import { ContextKey } from '../src/context-key.js';
import { Context } from '../src/context.js';

function makeCtx(): Context {
  return Context.create();
}

describe('context-object contract: ContextKey<T>', () => {
  // -------------------------------------------------------------------------
  // Inputs — the contract declares NO reject_with rules and NO errors. An
  // "invalid"-style input must NOT throw; instead assert the graceful fallback.
  // -------------------------------------------------------------------------

  it('context_object.get.input.default.absent_key_returns_default_not_raise: get on absent key returns default, never throws', () => {
    const key = new ContextKey<number>('ext.spec.absent');
    const ctx = makeCtx();
    // Default supplied -> returns the default exactly.
    expect(key.get(ctx, 7)).toBe(7);
    // No default supplied -> returns undefined (not a throw).
    expect(key.get(ctx)).toBeUndefined();
  });

  it('context_object.delete.input.absent.delete_absent_is_noop_no_raise: delete on absent key is a no-op, never throws', () => {
    const key = new ContextKey<number>('ext.spec.never_set');
    const ctx = makeCtx();
    expect(() => key.delete(ctx)).not.toThrow();
    expect(key.exists(ctx)).toBe(false);
  });

  it('context_object.scoped.input.suffix.appends_dotted_segment: scoped(suffix) appends a dotted segment', () => {
    const base = new ContextKey<number>('ext.spec.retry');
    const scoped = base.scoped('mod.a');
    expect(scoped.name).toBe('ext.spec.retry.mod.a');
    expect(scoped).not.toBe(base);
  });

  // -------------------------------------------------------------------------
  // Errors — the contract declares NONE. Assert the absence of throws across
  // the full surface so a future regression that adds a throw is caught.
  // -------------------------------------------------------------------------

  it('context_object.contextkey.error.none.no_method_raises: set/get/exists/delete/scoped never throw on normal use', () => {
    const key = new ContextKey<string>('ext.spec.noerr');
    const ctx = makeCtx();
    expect(() => {
      key.set(ctx, 'v');
      expect(key.get(ctx)).toBe('v');
      expect(key.exists(ctx)).toBe(true);
      expect(key.scoped('s').name).toBe('ext.spec.noerr.s');
      key.delete(ctx);
      expect(key.exists(ctx)).toBe(false);
      // Second delete (now-absent) still must not throw.
      key.delete(ctx);
    }).not.toThrow();
  });

  // -------------------------------------------------------------------------
  // Returns — assert the exact declared return shapes.
  // -------------------------------------------------------------------------

  it('context_object.set.returns.none.set_yields_no_value: set returns void/undefined', () => {
    const key = new ContextKey<number>('ext.spec.ret_set');
    const ctx = makeCtx();
    expect(key.set(ctx, 1)).toBeUndefined();
  });

  it('context_object.get.returns.value.present_returns_stored: get returns the stored value when present', () => {
    const key = new ContextKey<number>('ext.spec.ret_get');
    const ctx = makeCtx();
    key.set(ctx, 123);
    expect(key.get(ctx)).toBe(123);
  });

  it('context_object.get.returns.default.absent_returns_default: get narrows to default when absent', () => {
    const key = new ContextKey<number>('ext.spec.ret_get_def');
    const ctx = makeCtx();
    expect(key.get(ctx, 0)).toBe(0);
  });

  it('context_object.exists.returns.bool.true_iff_present: exists returns a boolean true iff present', () => {
    const key = new ContextKey<number>('ext.spec.ret_exists');
    const ctx = makeCtx();
    expect(key.exists(ctx)).toBe(false);
    key.set(ctx, 9);
    expect(key.exists(ctx)).toBe(true);
  });

  it('context_object.delete.returns.none.removes_name_from_data: delete removes the name and returns void/undefined', () => {
    const key = new ContextKey<number>('ext.spec.ret_delete');
    const ctx = makeCtx();
    key.set(ctx, 5);
    expect(key.delete(ctx)).toBeUndefined();
    expect('ext.spec.ret_delete' in ctx.data).toBe(false);
  });

  it('context_object.scoped.returns.key.new_contextkey_named_name_dot_suffix: scoped returns a new ContextKey named name.suffix', () => {
    const base = new ContextKey<number>('ext.spec.ret_scoped');
    const child = base.scoped('x');
    expect(child).toBeInstanceOf(ContextKey);
    expect(child.name).toBe('ext.spec.ret_scoped.x');
  });

  // -------------------------------------------------------------------------
  // Properties.
  // -------------------------------------------------------------------------

  it('context_object.contextkey.property.async.all_methods_synchronous: all methods are synchronous (no Promise/awaitable)', () => {
    const key = new ContextKey<number>('ext.spec.async');
    const ctx = makeCtx();
    // Spec Properties: "async: false — all methods are synchronous."
    // Results are plain values, never Promises.
    expect(key.set(ctx, 1)).not.toBeInstanceOf(Promise);
    const got = key.get(ctx);
    expect(got).not.toBeInstanceOf(Promise);
    expect(got).toBe(1);
    expect(key.exists(ctx)).not.toBeInstanceOf(Promise);
    expect(key.scoped('s')).not.toBeInstanceOf(Promise);
  });

  it('context_object.contextkey.property.thread_safe.concurrent_distinct_keys: >=8 concurrent writers each land their own value', async () => {
    const ctx = makeCtx();
    const n = 16;

    const writer = async (i: number): Promise<number> => {
      const key = new ContextKey<number>(`ext.spec.concurrent.${i}`);
      key.set(ctx, i);
      return key.get(ctx, -1) as number;
    };

    const results = await Promise.all(Array.from({ length: n }, (_, i) => writer(i)));
    expect(results).toEqual(Array.from({ length: n }, (_, i) => i));
    for (let i = 0; i < n; i++) {
      const key = new ContextKey<number>(`ext.spec.concurrent.${i}`);
      expect(key.get(ctx)).toBe(i);
    }
  });

  it('context_object.set.property.idempotent.repeat_same_value_same_state: repeating set with same value yields same state', () => {
    const key = new ContextKey<number>('ext.spec.idem_set');
    const ctx = makeCtx();
    key.set(ctx, 42);
    const first = { ...ctx.data };
    key.set(ctx, 42);
    const second = { ...ctx.data };
    expect(first).toEqual(second);
    expect(key.get(ctx)).toBe(42);
  });

  it('context_object.delete.property.idempotent.repeat_delete_same_state: repeating delete yields same state', () => {
    const key = new ContextKey<number>('ext.spec.idem_delete');
    const ctx = makeCtx();
    key.set(ctx, 1);
    key.delete(ctx);
    const afterFirst = { ...ctx.data };
    key.delete(ctx);
    const afterSecond = { ...ctx.data };
    expect(afterFirst).toEqual(afterSecond);
    expect(key.exists(ctx)).toBe(false);
  });

  it('context_object.exists.property.idempotent.repeat_query_same_state: exists/get are read-only across repeats', () => {
    const key = new ContextKey<number>('ext.spec.idem_query');
    const ctx = makeCtx();
    key.set(ctx, 5);
    const snapshot = { ...ctx.data };
    expect(key.exists(ctx)).toBe(true);
    expect(key.exists(ctx)).toBe(true);
    expect(key.get(ctx)).toBe(5);
    expect(key.get(ctx)).toBe(5);
    expect({ ...ctx.data }).toEqual(snapshot);
  });

  it('context_object.get.property.pure.read_only_no_self_mutation: get does not mutate context.data', () => {
    const key = new ContextKey<number>('ext.spec.pure_get');
    const ctx = makeCtx();
    key.set(ctx, 3);
    const snapshot = { ...ctx.data };
    void key.get(ctx);
    void key.get(ctx, 99);
    expect({ ...ctx.data }).toEqual(snapshot);
  });

  it('context_object.scoped.property.pure.allocates_new_key_no_mutation: scoped allocates a new key and never mutates the receiver', () => {
    const base = new ContextKey<number>('ext.spec.pure_scoped');
    const baseNameBefore = base.name;
    const child = base.scoped('leaf');
    expect(base.name).toBe(baseNameBefore);
    expect(child.name).toBe('ext.spec.pure_scoped.leaf');
    expect(child).not.toBe(base);
  });

  it('context_object.contextkey.property.immutable_key.name_is_readonly: the key name is read-only', () => {
    const key = new ContextKey<number>('ext.spec.frozen');
    // Spec: "the TypeScript `name` is `readonly`." This is a compile-time
    // guarantee only; TypeScript does NOT freeze the instance at runtime, so an
    // unchecked assignment does not throw (DIVERGENCE from Python's frozen
    // dataclass which raises AttributeError). Assert the actual TS behavior:
    // the readonly type prevents reassignment in well-typed code, and the value
    // is stable as observed through the public surface.
    expect(key.name).toBe('ext.spec.frozen');
    // A typed reassignment `key.name = ...` is a compile error; we assert the
    // declared name remains the constructor-supplied value.
    const descriptor = Object.getOwnPropertyDescriptor(key, 'name');
    expect(descriptor?.value).toBe('ext.spec.frozen');
  });

  // -------------------------------------------------------------------------
  // Side Effects — set/delete mutate context.data in place; observe ordering
  // via the public data map.
  // -------------------------------------------------------------------------

  it('context_object.contextkey.side_effect.1.set_then_delete_ordered: set then delete observed in order', () => {
    const key = new ContextKey<string>('ext.spec.effect');
    const ctx = makeCtx();
    const observed: boolean[] = [];
    observed.push(key.exists(ctx)); // absent before set
    key.set(ctx, 'v');
    observed.push(key.exists(ctx)); // present after set
    key.delete(ctx);
    observed.push(key.exists(ctx)); // absent after delete
    expect(observed).toEqual([false, true, false]);
  });

  // -------------------------------------------------------------------------
  // Namespace Convention (Normative) — identifier strings round-trip verbatim
  // into context.data (one shared namespace with raw string keys).
  // -------------------------------------------------------------------------

  it('context_object.contextkey.namespace.ext.shared_with_raw_string_keys: ext key shares one namespace with raw string keys', () => {
    const key = new ContextKey<number>('ext.my_company.retry.count');
    const ctx = makeCtx();
    key.set(ctx, 4);
    // Raw access sees the ContextKey-written value.
    expect(ctx.data['ext.my_company.retry.count']).toBe(4);
    // And a raw write is visible through the typed key.
    ctx.data['ext.my_company.retry.count'] = 9;
    expect(key.get(ctx)).toBe(9);
  });

  it('context_object.contextkey.namespace.apcore_prefix.collides_with_raw: _apcore-prefixed key collides with raw access (two views, one dict)', () => {
    const key = new ContextKey<number>('_apcore.foo');
    const ctx = makeCtx();
    ctx.data['_apcore.foo'] = 1;
    expect(key.exists(ctx)).toBe(true);
    expect(key.get(ctx)).toBe(1);
  });
});
