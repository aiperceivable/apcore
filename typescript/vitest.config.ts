import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    globals: true,
    // Explicit, rather than relying on vitest's 5000ms default. The C7 cases in
    // tests/test-sync-examples.test.ts dynamically `import()` example modules,
    // which pays a one-off TS transform cost on a cold cache — ~2.3s warm, and
    // the first such test is the one that pays it. That left under 2x headroom
    // against the implicit default, close enough that a loaded CI runner could
    // trip it and report a transform cost as a test failure.
    testTimeout: 20000,
    // Side-effect setup: wire Node-only file readers onto the
    // browser-safe runtime modules (ACL.load, registerSysModules, the
    // sys-modules overrides loader). Tests that import individual source
    // files directly (e.g. `from '../src/acl.js'`)
    // bypass the package's Node entry, so we install them here instead.
    setupFiles: ['./tests/setup-node-installers.ts'],
    coverage: {
      provider: 'v8',
      include: ['src/**/*.ts'],
      exclude: ['src/**/index.ts'],
      thresholds: {
        lines: 90,
        functions: 90,
        branches: 90,
        statements: 90,
      },
    },
  },
});
