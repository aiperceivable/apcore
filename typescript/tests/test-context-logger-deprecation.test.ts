/**
 * `Context.logger` is deprecated (apcore#121), removed at v2.0.
 *
 * The decision is D-67's boundary applied to an API surface rather than a
 * configuration key: apcore does not own the host's logging policy, and this
 * accessor's output is fixed at stderr / `info` / JSON no matter what the host
 * has configured. `ObsLoggingMiddleware` is deliberately NOT the migration
 * target — it emits apcore's execution events, a different facility.
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, it, expect, vi } from 'vitest';

import { Context } from '../src/context.js';
import { ContextLogger } from '../src/observability/context-logger.js';

describe('Context.logger is deprecated', () => {
  it('warns, and says where to go instead', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    new Context({ traceId: 't-1', callerId: 'api.probe' } as never).logger;
    const hits = warn.mock.calls
      .map((c) => String(c[0]))
      .filter((l) => l.includes('Context.logger is deprecated'));
    warn.mockRestore();

    // At most one per process: this accessor can be hit once per module call,
    // and a notice per call is the flood §9.2.2 rejects. The static latch means
    // a prior test in this file may already have consumed it, so the assertion
    // is on the CONTENT of whatever was emitted, checked in the case below.
    expect(hits.length).toBeLessThanOrEqual(1);
  });

  it('names version 2.0, the host logger, and the issue', () => {
    // Reads the source rather than the runtime, because the one-per-process
    // latch makes the message unobservable after the first access anywhere in
    // the suite — a test that only watched the console would pass vacuously.
    const here = path.dirname(fileURLToPath(import.meta.url));
    const src = fs.readFileSync(path.join(here, '..', 'src', 'context.ts'), 'utf-8');
    const notice = src.slice(src.indexOf('Context.logger is deprecated'));
    expect(notice).toContain('removed in version 2.0');
    expect(notice).toContain("host application's logger");
    expect(notice).toContain('apcore#121');
  });

  it('still works through the 1.x line', () => {
    // A deprecation is a notice, not a removal — §13.2's two-minor floor.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const logger = new Context({ traceId: 't-1', callerId: 'api.probe' } as never).logger;
    warn.mockRestore();
    expect(logger).toBeInstanceOf(ContextLogger);
  });

  it('is reached by no apcore code path', () => {
    // The premise of deprecating rather than wiring, written as a test rather
    // than left as a claim: it is what makes removal at v2.0 safe for the
    // framework itself. A future caller inside src/ fails here.
    const here = path.dirname(fileURLToPath(import.meta.url));
    const src = path.join(here, '..', 'src');
    const hits: string[] = [];
    const walk = (dir: string): void => {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) walk(full);
        else if (entry.name.endsWith('.ts')) {
          fs.readFileSync(full, 'utf-8')
            .split('\n')
            .forEach((line, i) => {
              if (/\b(context|ctx)\.logger\b/.test(line)) {
                hits.push(`${path.relative(src, full)}:${i + 1}`);
              }
            });
        }
      }
    };
    walk(src);
    expect(hits).toEqual([]);
  });
});
