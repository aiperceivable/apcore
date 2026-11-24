import { describe, it, expect, vi } from 'vitest';
import { STREAMING_MARKER, isStreamingModule } from '../src/streaming.js';
import type { Module } from '../src/module.js';
import { Registry } from '../src/registry/registry.js';
import { StreamingInterfaceError } from '../src/errors.js';
import { createAnnotations } from '../src/module.js';

class ProperStreamingModule {
  readonly [STREAMING_MARKER] = true as const;
  readonly id = 'test.stream';
  readonly name = 'Test';
  readonly description = 'test';
  readonly inputSchema = {};
  readonly outputSchema = {};
  async execute() { return {}; }
  async *stream() { yield {}; }
}

class LegacyStreamingModule {
  readonly id = 'test.legacy';
  readonly name = 'Legacy';
  readonly description = 'test';
  readonly inputSchema = {};
  readonly outputSchema = {};
  async execute() { return {}; }
  async *stream() { yield {}; }
}

class NonStreamingModule {
  readonly id = 'test.nonstream';
  readonly name = 'NonStream';
  readonly description = 'test';
  readonly inputSchema = {};
  readonly outputSchema = {};
  async execute() { return {}; }
}

describe('StreamingModule interface (#62)', () => {
  it('module with marker + stream() passes isStreamingModule', () => {
    const m = new ProperStreamingModule();
    expect(isStreamingModule(m as unknown as Module)).toBe(true);
  });

  it('module without marker but with stream() passes (transitional) AND warns once', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const m = new LegacyStreamingModule();
    expect(isStreamingModule(m as unknown as Module)).toBe(true);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(warnSpy.mock.calls[0][0]).toContain('STREAMING_MARKER');
    // Second call should NOT warn again for the same instance
    isStreamingModule(m as unknown as Module);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    warnSpy.mockRestore();
  });

  it('module without stream() fails isStreamingModule', () => {
    const m = new NonStreamingModule();
    expect(isStreamingModule(m as unknown as Module)).toBe(false);
  });

  it('module with streaming annotation but no stream() method throws StreamingInterfaceError at registration', () => {
    const registry = new Registry();
    const mod = {
      id: 'test.badstream',
      name: 'BadStream',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      annotations: createAnnotations({ streaming: true }),
      async execute() { return {}; },
    };
    expect(() => registry.register('test.badstream', mod)).toThrow(StreamingInterfaceError);
  });
});
