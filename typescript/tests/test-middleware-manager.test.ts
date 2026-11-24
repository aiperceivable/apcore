import { describe, it, expect, vi } from 'vitest';
import { Middleware } from '../src/middleware/base.js';
import { MiddlewareManager, MiddlewareChainError } from '../src/middleware/manager.js';
import { Context, createIdentity } from '../src/context.js';

function makeContext(): Context {
  return Context.create(createIdentity('test-user'));
}

class TaggingMiddleware extends Middleware {
  readonly tag: string;

  constructor(tag: string, priority: number = 100) {
    super(priority);
    this.tag = tag;
  }

  override before(
    _moduleId: string,
    inputs: Record<string, unknown>,
  ): Record<string, unknown> | null {
    const trail = ((inputs['trail'] as string) ?? '') + this.tag;
    return { ...inputs, trail };
  }

  override after(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    output: Record<string, unknown>,
  ): Record<string, unknown> | null {
    const trail = ((output['trail'] as string) ?? '') + this.tag;
    return { ...output, trail };
  }
}

class RecoveringMiddleware extends Middleware {
  readonly recovery: Record<string, unknown>;

  constructor(recovery: Record<string, unknown>) {
    super();
    this.recovery = recovery;
  }

  override onError(): Record<string, unknown> | null {
    return this.recovery;
  }
}

describe('MiddlewareManager', () => {
  it('starts empty', () => {
    const mgr = new MiddlewareManager();
    expect(mgr.snapshot()).toEqual([]);
  });

  it('add and snapshot', () => {
    const mgr = new MiddlewareManager();
    const mw1 = new Middleware();
    const mw2 = new Middleware();
    mgr.add(mw1);
    mgr.add(mw2);
    expect(mgr.snapshot()).toHaveLength(2);
  });

  it('snapshot returns a copy', () => {
    const mgr = new MiddlewareManager();
    mgr.add(new Middleware());
    const snap = mgr.snapshot();
    snap.pop();
    expect(mgr.snapshot()).toHaveLength(1);
  });

  it('remove by identity', () => {
    const mgr = new MiddlewareManager();
    const mw1 = new Middleware();
    const mw2 = new Middleware();
    mgr.add(mw1);
    mgr.add(mw2);
    expect(mgr.remove(mw1)).toBe(true);
    expect(mgr.snapshot()).toEqual([mw2]);
  });

  it('remove returns false when not found', () => {
    const mgr = new MiddlewareManager();
    expect(mgr.remove(new Middleware())).toBe(false);
  });

  it('executeBefore runs in forward order', async () => {
    const mgr = new MiddlewareManager();
    mgr.add(new TaggingMiddleware('A'));
    mgr.add(new TaggingMiddleware('B'));
    mgr.add(new TaggingMiddleware('C'));
    const ctx = makeContext();
    const [result, executed] = await mgr.executeBefore('mod.test', { trail: '' }, ctx);
    expect(result['trail']).toBe('ABC');
    expect(executed).toHaveLength(3);
  });

  it('executeBefore passes original inputs when all return null', async () => {
    const mgr = new MiddlewareManager();
    mgr.add(new Middleware());
    mgr.add(new Middleware());
    const ctx = makeContext();
    const [result] = await mgr.executeBefore('mod.test', { x: 42 }, ctx);
    expect(result).toEqual({ x: 42 });
  });

  it('executeAfter runs in reverse order', async () => {
    const mgr = new MiddlewareManager();
    mgr.add(new TaggingMiddleware('A'));
    mgr.add(new TaggingMiddleware('B'));
    mgr.add(new TaggingMiddleware('C'));
    const ctx = makeContext();
    const result = await mgr.executeAfter('mod.test', {}, { trail: '' }, ctx);
    expect(result['trail']).toBe('CBA');
  });

  it('executeAfter passes original output when all return null', async () => {
    const mgr = new MiddlewareManager();
    mgr.add(new Middleware());
    const ctx = makeContext();
    const result = await mgr.executeAfter('mod.test', {}, { y: 99 }, ctx);
    expect(result).toEqual({ y: 99 });
  });

  it('executeOnError returns first non-null recovery (reverse order)', async () => {
    const mgr = new MiddlewareManager();
    const mwA = new RecoveringMiddleware({ recovered: 'A' });
    const mwB = new RecoveringMiddleware({ recovered: 'B' });
    mgr.add(mwA);
    mgr.add(mwB);
    const ctx = makeContext();
    const result = await mgr.executeOnError('mod.test', {}, new Error('oops'), ctx, [mwA, mwB]);
    expect(result).toEqual({ recovered: 'B' });
  });

  it('executeOnError returns null when no recovery', async () => {
    const mgr = new MiddlewareManager();
    const mw = new Middleware();
    mgr.add(mw);
    const ctx = makeContext();
    const result = await mgr.executeOnError('mod.test', {}, new Error('oops'), ctx, [mw]);
    expect(result).toBeNull();
  });

  it('executeBefore awaits async middleware result (sync A-D-403)', async () => {
    class AsyncBeforeMw extends Middleware {
      override before(
        _moduleId: string,
        inputs: Record<string, unknown>,
      ): Record<string, unknown> | null {
        // Real Promise — manager must await, not pass the Promise itself
        // through as currentInputs.
        return (Promise.resolve({ ...inputs, asyncTouched: true }) as unknown) as
          Record<string, unknown> | null;
      }
    }
    const mgr = new MiddlewareManager();
    mgr.add(new AsyncBeforeMw());
    const ctx = makeContext();
    const [result] = await mgr.executeBefore('mod.test', { x: 1 }, ctx);
    // If manager forgot to await, result['asyncTouched'] would be undefined and
    // result itself would be the unresolved Promise.
    expect(result['asyncTouched']).toBe(true);
    expect(result['x']).toBe(1);
  });

  it('executeAfter awaits async middleware result (sync A-D-403)', async () => {
    class AsyncAfterMw extends Middleware {
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
      ): Record<string, unknown> | null {
        return (Promise.resolve({ ...output, asyncMutated: true }) as unknown) as
          Record<string, unknown> | null;
      }
    }
    const mgr = new MiddlewareManager();
    mgr.add(new AsyncAfterMw());
    const ctx = makeContext();
    const result = await mgr.executeAfter('mod.test', {}, { y: 2 }, ctx);
    expect(result['asyncMutated']).toBe(true);
    expect(result['y']).toBe(2);
  });

  it('executeOnError awaits async recovery (sync A-D-403)', async () => {
    class AsyncRecoverMw extends Middleware {
      override onError(): Record<string, unknown> | null {
        return (Promise.resolve({ recovered: 'async-yes' }) as unknown) as
          Record<string, unknown> | null;
      }
    }
    const mgr = new MiddlewareManager();
    const mw = new AsyncRecoverMw();
    mgr.add(mw);
    const ctx = makeContext();
    const result = await mgr.executeOnError('mod.test', {}, new Error('x'), ctx, [mw]);
    // If we forgot to await, result would be a Promise instance, not the dict.
    expect(result).toEqual({ recovered: 'async-yes' });
  });

  it('executeOnError treats undefined as no-recovery (sync A-D-404)', async () => {
    // Arrow function returning undefined must NOT trigger the recovery path.
    class UndefinedReturnMw extends Middleware {
      override onError(): Record<string, unknown> | null {
        return undefined as unknown as null;
      }
    }
    const mgr = new MiddlewareManager();
    const mw = new UndefinedReturnMw();
    mgr.add(mw);
    const ctx = makeContext();
    const result = await mgr.executeOnError('mod.test', {}, new Error('x'), ctx, [mw]);
    // Strict spec: only a real object or RetrySignal counts as recovery.
    expect(result).toBeNull();
  });

  it('executeOnError swallows errors in onError handlers', async () => {
    class ThrowingOnError extends Middleware {
      override onError(): Record<string, unknown> | null {
        throw new Error('onError also failed');
      }
    }
    const mgr = new MiddlewareManager();
    const mwRecover = new RecoveringMiddleware({ safe: true });
    const mwThrow = new ThrowingOnError();
    mgr.add(mwRecover);
    mgr.add(mwThrow);
    const ctx = makeContext();
    const result = await mgr.executeOnError('mod.test', {}, new Error('original'), ctx, [mwRecover, mwThrow]);
    expect(result).toEqual({ safe: true });
  });

  it('MiddlewareChainError wraps before() failure', async () => {
    class FailingBefore extends Middleware {
      override before(): Record<string, unknown> | null {
        throw new Error('before exploded');
      }
    }
    const mgr = new MiddlewareManager();
    const ok = new TaggingMiddleware('A');
    const fail = new FailingBefore();
    mgr.add(ok);
    mgr.add(fail);
    const ctx = makeContext();

    let caught: MiddlewareChainError | undefined;
    try {
      await mgr.executeBefore('mod.test', { trail: '' }, ctx);
    } catch (e) {
      caught = e as MiddlewareChainError;
    }

    expect(caught).toBeInstanceOf(MiddlewareChainError);
    expect(caught!.original.message).toBe('before exploded');
    expect(caught!.executedMiddlewares).toHaveLength(2);
  });

  describe('priority ordering', () => {
    it('higher priority middleware executes first in before()', async () => {
      const mgr = new MiddlewareManager();
      mgr.add(new TaggingMiddleware('Low', 100));
      mgr.add(new TaggingMiddleware('High', 500));
      mgr.add(new TaggingMiddleware('Mid', 300));
      const ctx = makeContext();
      const [result] = await mgr.executeBefore('mod.test', { trail: '' }, ctx);
      // Sorted by priority descending: High(500), Mid(300), Low(100)
      expect(result['trail']).toBe('HighMidLow');
    });

    it('equal priority preserves registration order', async () => {
      const mgr = new MiddlewareManager();
      mgr.add(new TaggingMiddleware('First', 100));
      mgr.add(new TaggingMiddleware('Second', 100));
      mgr.add(new TaggingMiddleware('Third', 100));
      const ctx = makeContext();
      const [result] = await mgr.executeBefore('mod.test', { trail: '' }, ctx);
      expect(result['trail']).toBe('FirstSecondThird');
    });

    it('lower priority middleware is ordered after higher priority', async () => {
      const mgr = new MiddlewareManager();
      mgr.add(new TaggingMiddleware('Default', 100));
      mgr.add(new TaggingMiddleware('Prioritized', 101));
      const ctx = makeContext();
      const [result] = await mgr.executeBefore('mod.test', { trail: '' }, ctx);
      expect(result['trail']).toBe('PrioritizedDefault');
    });

    it('executeAfter runs in reverse priority order (lowest priority first)', async () => {
      const mgr = new MiddlewareManager();
      mgr.add(new TaggingMiddleware('Low', 100));
      mgr.add(new TaggingMiddleware('High', 500));
      mgr.add(new TaggingMiddleware('Mid', 300));
      const ctx = makeContext();
      const result = await mgr.executeAfter('mod.test', {}, { trail: '' }, ctx);
      // Internal order is [High, Mid, Low]; after() reverses: Low, Mid, High
      expect(result['trail']).toBe('LowMidHigh');
    });

    it('Middleware base class defaults to priority 100', () => {
      const mw = new Middleware();
      expect(mw.priority).toBe(100);
    });

    it('throws RangeError for priority below 0', () => {
      expect(() => new Middleware(-1)).toThrow(RangeError);
      expect(() => new Middleware(-1)).toThrow('priority must be between 0 and 1000');
    });

    it('throws RangeError for priority above 1000', () => {
      expect(() => new Middleware(1001)).toThrow(RangeError);
      expect(() => new Middleware(1001)).toThrow('priority must be between 0 and 1000');
    });

    it('snapshot reflects priority-sorted order', () => {
      const mgr = new MiddlewareManager();
      const low = new TaggingMiddleware('L', 10);
      const high = new TaggingMiddleware('H', 900);
      const mid = new TaggingMiddleware('M', 500);
      mgr.add(low);
      mgr.add(high);
      mgr.add(mid);
      const tags = mgr.snapshot().map((mw) => (mw as TaggingMiddleware).tag);
      expect(tags).toEqual(['H', 'M', 'L']);
    });
  });

  describe('add() guards and identity bookkeeping (A-D-017..022)', () => {
    it('rejects priority > 1000 at the manager level (A-D-017)', () => {
      const mgr = new MiddlewareManager();
      // A plain object middleware bypasses the base-class constructor guard,
      // so the manager-level guard must still reject it.
      const bad = { priority: 1001, before: () => null, after: () => null, onError: () => null } as unknown as Middleware;
      expect(() => mgr.add(bad)).toThrow(RangeError);
    });

    it('warns on duplicate identity without allowDuplicate', () => {
      const mgr = new MiddlewareManager();
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      mgr.add(new TaggingMiddleware('A'));
      mgr.add(new TaggingMiddleware('B')); // same class → same identity
      expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Duplicate middleware registration'));
      warnSpy.mockRestore();
    });

    it('records identity even when allowDuplicate=true; only the warning is suppressed (A-D-021/022 d)', () => {
      const mgr = new MiddlewareManager();
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      // First add with allowDuplicate suppresses the warning AND records identity.
      mgr.add(new TaggingMiddleware('A'), { allowDuplicate: true });
      expect(warnSpy).not.toHaveBeenCalled();
      // A subsequent plain add of the same identity must now warn, proving the
      // identity was recorded under allowDuplicate.
      mgr.add(new TaggingMiddleware('B'));
      expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Duplicate middleware registration'));
      warnSpy.mockRestore();
    });

    it('remove() cleans the identity registry so re-add does not warn (A-D-020 c)', () => {
      const mgr = new MiddlewareManager();
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      const first = new TaggingMiddleware('A');
      mgr.add(first);
      expect(mgr.remove(first)).toBe(true);
      // Identity registry cleaned → re-adding the same class is a fresh
      // registration, no duplicate warning.
      mgr.add(new TaggingMiddleware('B'));
      expect(warnSpy).not.toHaveBeenCalled();
      warnSpy.mockRestore();
    });
  });
});
