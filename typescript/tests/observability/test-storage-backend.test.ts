/**
 * StorageBackend interface and InMemoryStorageBackend default.
 *
 * Issue #43 §1: language-agnostic key/value storage for observability state.
 */

import { describe, it, expect } from 'vitest';
import {
  InMemoryStorageBackend,
  type StorageBackend,
} from '../../src/observability/storage.js';
import { ErrorHistory } from '../../src/observability/error-history.js';
import { UsageCollector } from '../../src/observability/usage.js';
import { MetricsCollector } from '../../src/observability/metrics.js';
import { ModuleError } from '../../src/errors.js';

describe('InMemoryStorageBackend', () => {
  it('save / get round-trip', async () => {
    const backend: StorageBackend = new InMemoryStorageBackend();
    await backend.save('errors', 'fp1', { code: 'X', message: 'boom' });
    const value = await backend.get('errors', 'fp1');
    expect(value).toEqual({ code: 'X', message: 'boom' });
  });

  it('returns null for missing key', async () => {
    const backend = new InMemoryStorageBackend();
    expect(await backend.get('errors', 'nope')).toBeNull();
  });

  it('list returns all key/value pairs in a namespace', async () => {
    const backend = new InMemoryStorageBackend();
    await backend.save('m', 'a', { v: 1 });
    await backend.save('m', 'b', { v: 2 });
    await backend.save('other', 'c', { v: 3 });

    const entries = await backend.list('m');
    expect(entries).toHaveLength(2);
    const keys = entries.map(([k]) => k).sort();
    expect(keys).toEqual(['a', 'b']);
  });

  it('list with prefix filters keys', async () => {
    const backend = new InMemoryStorageBackend();
    await backend.save('m', 'usage:1', { v: 1 });
    await backend.save('m', 'usage:2', { v: 2 });
    await backend.save('m', 'errors:1', { v: 3 });

    const entries = await backend.list('m', 'usage:');
    expect(entries.map(([k]) => k).sort()).toEqual(['usage:1', 'usage:2']);
  });

  it('delete removes a key', async () => {
    const backend = new InMemoryStorageBackend();
    await backend.save('m', 'a', { v: 1 });
    await backend.delete('m', 'a');
    expect(await backend.get('m', 'a')).toBeNull();
  });

  it('namespaces are isolated', async () => {
    const backend = new InMemoryStorageBackend();
    await backend.save('ns1', 'k', { v: 1 });
    await backend.save('ns2', 'k', { v: 2 });
    expect(await backend.get('ns1', 'k')).toEqual({ v: 1 });
    expect(await backend.get('ns2', 'k')).toEqual({ v: 2 });
  });
});

describe('Observability collectors accept StorageBackend', () => {
  it('ErrorHistory accepts a StorageBackend option', () => {
    const backend = new InMemoryStorageBackend();
    const history = new ErrorHistory({ storage: backend });
    history.record('mod.a', new ModuleError('X', 'boom'));
    expect(history.get('mod.a')).toHaveLength(1);
  });

  it('UsageCollector accepts a StorageBackend option', () => {
    const backend = new InMemoryStorageBackend();
    const usage = new UsageCollector({ storage: backend });
    usage.record('mod.a', 'caller', 100, true);
    expect(usage.getSummary().length).toBeGreaterThanOrEqual(1);
  });

  it('MetricsCollector accepts a StorageBackend option', () => {
    const backend = new InMemoryStorageBackend();
    const metrics = new MetricsCollector({ storage: backend });
    metrics.incrementCalls('mod.a', 'success');
    expect(metrics.snapshot()).toBeDefined();
  });
});
