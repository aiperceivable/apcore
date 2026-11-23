/**
 * Default configuration values, runtime-neutral.
 *
 * These constants and the `getDefault` helper are pure data and have no
 * dependency on `node:*` builtins. They are intentionally factored out of
 * `config.ts` so that browser-side consumers (e.g. the in-browser apcore-js
 * runtime used by apwebsite) can import the runtime classes (Registry,
 * Executor, APCore) without dragging the filesystem-loading code in
 * `config.ts` into the bundle.
 *
 * The full Node-side `Config` API and `discoverConfigFile` live in
 * `./config.ts` and re-export `DEFAULTS` / `getDefault` from here so
 * existing `import { getDefault } from './config.js'` paths keep working.
 */

/**
 * Default configuration values for legacy mode.
 *
 * This table mirrors `schemas/defaults.schema.json` in the spec repo, which is
 * the single source of truth for canonical defaults.
 *
 * NOTE: `version` and `project.name` are deliberately ABSENT. PROTOCOL_SPEC
 * §9.1 ("What is required, and why so little is") defines them as the only two
 * keys with no canonical default, which is exactly why they are the only two
 * required keys. Inventing defaults for them here (this table used to declare
 * `version: '0.16.0'` and `project: { name: 'apcore' }`) made the required-field
 * check in `Config.validate` unreachable: the merge supplied both keys before
 * the check ran. Do not reintroduce them — a component that needs a project
 * name at runtime MUST pass its own explicit fallback to `config.get()`.
 */
export const DEFAULTS: Record<string, unknown> = {
  extensions: {
    root: './extensions',
    auto_discover: true,
    max_depth: 8,
    follow_symlinks: false,
  },
  schema: {
    root: './schemas',
    strategy: 'yaml_first',
    max_ref_depth: 32,
  },
  acl: {
    root: './acl',
    default_effect: 'deny',
  },
  executor: {
    default_timeout: 30000,
    global_timeout: 60000,
    max_call_depth: 32,
    max_module_repeat: 3,
  },
  observability: {
    tracing: {
      enabled: false,
      sampling_rate: 1.0,
      strategy: 'full',
      exporter: 'stdout',
      otlp_endpoint: null,
    },
    metrics: {
      enabled: false,
    },
  },
  sys_modules: {
    enabled: false,
  },
  stream: {
    max_merge_depth: 32,
  },
  // PROTOCOL_SPEC §9.1.2 (spec v1.38.0, apcore#118). The six `validation.*`
  // keys are UNCONSTRAINED by default — apcore does not impose limits on the
  // content its users author, it offers them. Five of the six are unconstrained
  // AS `null`, and `null` is the absence of a default, so only this one carries
  // a canonical value: `defaults.schema.json` declares exactly this entry and
  // `config_key_governance.json` pins this table to that file.
  validation: {
    binding: {
      version_require_semver: false,
    },
  },
  // PROTOCOL_SPEC §5.12.6 / §9.1.1, via `defaults.schema.json`'s `bindings`
  // section (spec v1.36.0). Until that section existed the `'./bindings'`
  // default was unreachable from the mechanism meant to serve it: every
  // consumer had to carry its own literal, and `config.get('bindings.dir')`
  // answered `undefined` for a key the spec gives a canonical default.
  // `dir` is path-typed (§9.2.1); `pattern` is a glob matched against
  // filenames inside `dir` and is deliberately NOT.
  bindings: {
    dir: './bindings',
    pattern: '*.binding.yaml',
  },
};

/**
 * Single source of truth for default values.
 * Components MUST use this instead of hardcoding defaults.
 */
export function getDefault(key: string, fallback?: unknown): unknown {
  const parts = key.split('.');
  let node: unknown = DEFAULTS;
  for (const part of parts) {
    if (node != null && typeof node === 'object' && part in (node as Record<string, unknown>)) {
      node = (node as Record<string, unknown>)[part];
    } else {
      return fallback;
    }
  }
  return node;
}
