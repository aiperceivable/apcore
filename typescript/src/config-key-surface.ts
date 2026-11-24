/**
 * The declared configuration key surface of the `apcore` namespace, and the
 * walker that finds keys it does not declare (PROTOCOL_SPEC §9.10,
 * sub-algorithm `reject_unknown_framework_keys`).
 *
 * Runtime-neutral on purpose: no `node:*` import, so this module can back a
 * browser-side `Config` if one ever ships. `config.ts` is the only caller today.
 *
 * ## Why the key list is a constant and not a schema read
 *
 * Every framework section in `schemas/apcore-config.schema.json` is
 * `additionalProperties: false`, and that closedness is what this module
 * enforces. Reading the schema at run time is not available to us: the schemas
 * live in the spec repo (`apcore/schemas/`), the published npm package ships
 * `dist` and `README.md` only, and a `prepare`-time fetch would work solely for
 * the handful of developers who happen to have the spec repo checked out as a
 * sibling. So the surface is a committed projection of the schemas rather than
 * a parse of them.
 *
 * The projection is not trusted on its word. It is the exact array that
 * `conformance/fixtures/config_key_governance.json` publishes as
 * `allowed_keys`, which the spec repo regenerates from
 * `schemas/apcore-config.schema.json`, `schemas/defaults.schema.json` and
 * `schemas/sys-modules.schema.json` via
 * `conformance/generate_config_key_governance.py`, and
 * `tests/conformance-config-key-governance.test.ts` asserts the two are equal
 * on every run. Add a section to the schema, regenerate the fixture, and this
 * file goes red naming the keys it is missing — the drift the list would
 * otherwise accumulate is a test failure, not a silent divergence.
 *
 * All three canonical schemas feed it, not `apcore-config.schema.json` alone.
 * `apcore-config.schema.json`'s `SysModulesConfig` declares `enabled` and
 * nothing else, while `sys-modules.schema.json` declares the rest of the
 * `sys_modules.*` tree that `features/system-modules.md` documents and that
 * every SDK's constraint table already validates. Enforcing against the first
 * file alone would reject `sys_modules.events.enabled` — documented, canonical
 * configuration — under strict mode. The fixture's `canonical_sources` is the
 * repo's existing answer to "what is the config key surface", so it is the one
 * used here.
 *
 * @see PROTOCOL_SPEC §9.6.3 (`_config.strict`), §9.10 (Algorithm A12-NS).
 */

/**
 * Every configuration key the canonical schemas declare, as dot-paths.
 *
 * Leaves only: `acl.audit.enabled` is present, the container `acl.audit` is
 * not. A path in this list terminates the walk — whatever shape its value has
 * (object, array, arbitrary map) belongs to the schema that declared it, not to
 * this check. That is what keeps `pipeline.steps`, `pipeline.configure` and
 * `id_map.overrides` — all of them user-shaped payloads — from being reported
 * key by key.
 *
 * Mirrors `config_key_governance.json` → `allowed_keys`. Do not hand-edit:
 * regenerate the fixture in the spec repo and copy it across.
 */
export const FRAMEWORK_CONFIG_KEYS: readonly string[] = [
  "$schema",
  // Config Bus meta-configuration (§9.6.3). Declared at the schema root, which
  // is additionalProperties: false — without it a legacy apcore.yaml enabling
  // strict mode would fail its own canonical schema, and `_config.strcit` is
  // exactly the typo one least wants silently ignored.
  "_config.allow_unknown",
  "_config.strict",
  "acl.audit.enabled",
  "acl.audit.include_denied",
  "acl.audit.log_level",
  "acl.default_effect",
  "acl.root",
  "bindings.dir",
  "bindings.pattern",
  "executor.default_timeout",
  "executor.global_timeout",
  "executor.max_call_depth",
  "executor.max_module_repeat",
  "extensions.auto_discover",
  "extensions.follow_symlinks",
  "extensions.ignore_patterns",
  "extensions.lazy_load",
  "extensions.max_depth",
  "extensions.namespace",
  "extensions.root",
  "extensions.roots",
  "id_map.auto_detect",
  "id_map.overrides",
  "logging.format",
  "logging.level",
  "middleware.disabled",
  "obs.redaction.regex_patterns",
  "obs.redaction.replacement",
  "obs.redaction.sensitive_keys",
  "observability.metrics.enabled",
  "observability.metrics.exporter",
  "observability.tracing.enabled",
  "observability.tracing.exporter",
  "observability.tracing.otlp_endpoint",
  "observability.tracing.sampling_rate",
  "observability.tracing.strategy",
  "pipeline.configure",
  "pipeline.remove",
  "pipeline.steps",
  "project.name",
  "project.version",
  "schema.max_ref_depth",
  "schema.root",
  "schema.strategy",
  "stream.max_merge_depth",
  "sys_modules.control.enabled",
  "sys_modules.control.overrides_path",
  "sys_modules.enabled",
  "sys_modules.error_history.max_entries_per_module",
  "sys_modules.error_history.max_total_entries",
  "sys_modules.events.enabled",
  "sys_modules.events.subscribers",
  "sys_modules.events.thresholds.error_rate",
  "sys_modules.events.thresholds.latency_p99_ms",
  "sys_modules.health.enabled",
  "sys_modules.manifest.enabled",
  "sys_modules.usage.bucketing_strategy",
  "sys_modules.usage.enabled",
  "sys_modules.usage.retention_hours",
  "validation.binding.description_max_length",
  "validation.binding.documentation_max_length",
  "validation.binding.tags_pattern",
  "validation.binding.version_require_semver",
  "validation.pipeline.step_name_max_length",
  "validation.pipeline.timeout_ms_max",
  "version",
];

/**
 * A node in the declared-key trie: `null` marks a declared leaf, a map marks a
 * container whose children are themselves declared.
 */
type SurfaceNode = Map<string, SurfaceNode | null> | null;

function buildSurfaceTrie(keys: readonly string[]): Map<string, SurfaceNode> {
  const root = new Map<string, SurfaceNode>();
  for (const key of keys) {
    const parts = key.split('.');
    let node = root;
    for (let i = 0; i < parts.length - 1; i += 1) {
      const existing = node.get(parts[i] as string);
      if (existing instanceof Map) {
        node = existing;
      } else {
        const child = new Map<string, SurfaceNode>();
        node.set(parts[i] as string, child);
        node = child;
      }
    }
    node.set(parts[parts.length - 1] as string, null);
  }
  return root;
}

const SURFACE_TRIE = buildSurfaceTrie(FRAMEWORK_CONFIG_KEYS);

/**
 * The framework sections — the top-level keys of the `apcore` namespace that
 * hold a declared sub-tree.
 *
 * Derived from {@link FRAMEWORK_CONFIG_KEYS} rather than listed, so a section
 * added to the schemas becomes a section here the moment the key list is
 * refreshed. `version` and `$schema` are top-level *leaves*, not sections, and
 * are correctly absent.
 */
export const FRAMEWORK_SECTIONS: readonly string[] = Array.from(SURFACE_TRIE.entries())
  .filter(([, node]) => node instanceof Map)
  .map(([name]) => name);

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function walk(
  data: Record<string, unknown>,
  node: Map<string, SurfaceNode>,
  prefix: string,
  out: string[],
): void {
  for (const [key, value] of Object.entries(data)) {
    const path = `${prefix}.${key}`;
    if (!node.has(key)) {
      out.push(path);
      continue;
    }
    const child = node.get(key);
    if (child instanceof Map && isPlainObject(value)) {
      walk(value, child, path, out);
    }
    // A declared leaf ends the walk whatever its value is, and a declared
    // container holding a non-object is a type error that Algorithm A12's
    // constraint table owns — neither is an undeclared key.
  }
}

/**
 * Collect every key inside a framework section that no canonical schema
 * declares (PROTOCOL_SPEC §9.10, `reject_unknown_framework_keys` step 2).
 *
 * Returns dot-paths, sorted, and **all** of them — the caller reports the whole
 * set in one error so that one restart shows the operator the whole problem
 * rather than the first typo of several.
 *
 * Only declared sections are descended into. An unknown *top-level* key is out
 * of scope here by design: in namespace mode that position holds namespaces,
 * governed by the separate `_config.strict` namespace check, and in legacy mode
 * the sub-algorithm is specified over `apcore_data[section]` only.
 *
 * @param apcoreData The `apcore` namespace tree — the whole document in legacy
 *   mode, `data.apcore` in namespace mode.
 */
/**
 * The closed set of **path-typed** configuration keys — those whose value is a
 * filesystem path (PROTOCOL_SPEC §9.2.1). Declared canonically by
 * `"x-apcore-path": true` in `schemas/apcore-config.schema.json`; this array is
 * that projection, on the same terms as `FRAMEWORK_CONFIG_KEYS` above.
 *
 * It exists for consumers outside this SDK. Anything forwarding apcore
 * configuration across a process boundary — a CLI spawning a worker, a
 * supervisor building a container environment — has to know which `APCORE_*`
 * variables carry paths, because a relative value silently re-roots wherever the
 * working directory differs. Without a published set, each such consumer builds
 * its own and drifts from the others.
 *
 * `bindings.pattern` is deliberately excluded: it is a glob matched against
 * filenames *within* `bindings.dir`, never resolved as a path itself.
 * `id_map.overrides` is the path of an ID map file and
 * `sys_modules.control.overrides_path` the path of the runtime overrides file
 * (D-138); the latter's marker lives in the sys-modules namespace schema.
 *
 * `extensions.roots` is list-valued and every element carries a path, in both the
 * bare-string and the `{ root, namespace }` form — hence the element key
 * `extensions.roots[]`.
 *
 * This set says *which* keys carry paths. It says nothing about what a relative
 * value resolves against; that base is unspecified as of spec v1.34.0.
 */
export const PATH_TYPED_CONFIG_KEYS: readonly string[] = [
  "acl.root",
  "bindings.dir",
  "extensions.root",
  "extensions.roots[]",
  "id_map.overrides",
  "schema.root",
  "sys_modules.control.overrides_path",
];

export function collectUndeclaredFrameworkKeys(apcoreData: Record<string, unknown>): string[] {
  const out: string[] = [];
  for (const [section, node] of SURFACE_TRIE) {
    if (!(node instanceof Map)) continue; // top-level leaf ($schema, version)
    const value = apcoreData[section];
    if (!isPlainObject(value)) continue;
    walk(value, node, section, out);
  }
  return out.sort();
}
