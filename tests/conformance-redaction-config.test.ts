/**
 * Cross-language conformance driver for `redaction_config.json`
 * (Issue #43 §5 / #45 §3, D-54 —
 * docs/features/observability.md#redaction-configuration).
 *
 * Fixture source: apcore/conformance/fixtures/redaction_config.json (canonical).
 * The `description` plus the fixture's `driver_contract` are the contract:
 * redaction is configured through `obs.redaction.regex_patterns` /
 * `obs.redaction.sensitive_keys` (+ `obs.redaction.replacement`),
 * `sensitive_keys` matches case-insensitively on a substring of the key,
 * `regex_patterns` matches the whole value, the default `sensitive_keys` list
 * covers the common credential terms, and the correlation fields (`trace_id`,
 * `caller_id`, `target_id`, `module_id`, `span_id`) are NEVER redacted.
 *
 * TWO CASE SHAPES
 * ---------------
 * 1. BEHAVIOUR cases carry `redaction_config` (literal rule values). They are
 *    driven through BOTH public surfaces, because the fixture describes config
 *    keys while the middleware calls the object directly:
 *      a. `RedactionConfig.fromConfig(config)` — the `obs.redaction.*` key path
 *         (this also exercises the `obs` namespace registration in
 *         src/config.ts:1322-1334).
 *      b. `new RedactionConfig({...})` — the literal fixture values.
 *    Both must produce the fixture's `expected` mapping, via `apply()` (used by
 *    ObsLoggingMiddleware) and via `redact()` (used by ContextLogger).
 * 2. CONFIG-KEY cases carry `config`, a map of dot-path -> value. Per the
 *    fixture's `which_key_is_read_is_part_of_the_contract`, these are built
 *    from the exact key path the case names, so reading the wrong path fails
 *    here rather than in an operator's production logs.
 *
 * WHY THE CONFIG-KEY CASES EXIST, AND WHY THEIR PAYLOAD LOOKS ODD
 * --------------------------------------------------------------
 * `username` matches NO shipped default; `password` and `_secret_token` both
 * do. So "username redacted AND password left alone" proves the override was
 * READ and REPLACED the defaults, while "password redacted, username alone"
 * proves the defaults are still in force because the configured key was never
 * consulted. That is the exact defect apcore-rust shipped (apcore-rust#32): it
 * read only the legacy `observability.redaction.*` path, so an operator
 * following the documentation had their config silently discarded. A payload
 * whose keys all match a default anyway stays green under both behaviours,
 * which is why no SDK's tests caught it — see the empty-list note below for
 * the same trap in this file's own existing case.
 *
 * WHY THIS FIXTURE CANNOT PIN THE EMPTY-LIST RULE. Case
 * `regex_pattern_value_match` sets `sensitive_keys: []`, but every key in its
 * input either matches a shipped default anyway (`auth_header` contains
 * `auth`) or matches nothing, so its expectations hold whether `[]` is read as
 * "no key patterns" or as "unset". `fromConfig` used to take the second
 * reading and substitute the default list; it now honours `[]` literally, per
 * D-54 and observability.md ("the override REPLACES the default; it does not
 * merge"). That rule is pinned with DISCRIMINATING keys in
 * tests/observability/test-redaction-default-keys.test.ts, not here — reverting
 * the SDK was verified to leave this file green either way.
 *
 * THE LEGACY SPELLING IS PER-SDK HISTORY, AND THE FIXTURE SAYS SO
 * ---------------------------------------------------------------
 * The fixture's `legacy_spelling_is_per_sdk_history` contract: the CANONICAL
 * path (`obs.redaction.sensitive_keys`) is the cross-language contract every
 * SDK MUST read, and `canonical_config_key_is_read` applies to all three. The
 * LEGACY path is not a contract — it is whatever each SDK shipped before D-53:
 *   - apcore-rust       `observability.redaction.sensitive_keys` / `.regex_patterns`
 *   - apcore-typescript `observability.redaction.field_patterns` / `.value_patterns`
 *     (see docs/features/observability.md's migration table, which maps
 *     `observability.redaction.field_patterns` -> `obs.redaction.sensitive_keys`)
 *   - apcore-python     nothing; it reads canonical only.
 * So this driver reads its own spelling out of the case's `legacy_key_by_sdk`
 * map under SDK_LANGUAGE rather than translating the fixture through a
 * driver-local table. An earlier revision of the fixture hard-coded Rust's
 * spelling and this driver carried such a table as a stopgap; a driver-local
 * translation of a canonical fixture drifts silently the moment the fixture
 * moves, which is exactly what then happened.
 *
 * What the driver still refuses to do is repair a key it does not recognise:
 * an `observability.*` path this SDK never read throws (KNOWN_LEGACY_KEYS
 * below) rather than being set blindly, because a config entry nothing reads
 * looks green for precisely the wrong reason. A `null` entry is different from
 * an unrecognised one — it means the case does not apply to this SDK, and per
 * `skip_when_legacy_key_is_null` it MUST be skipped with that reason rather
 * than xfailed as a deficiency.
 */

import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import { Config } from '../src/config.js';
import { redactSensitive } from '../src/executor.js';
import { RedactionConfig } from '../src/observability/context-logger.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

interface RedactionCase {
  readonly id: string;
  /** Behaviour cases: literal rule values. */
  readonly schema?: Record<string, unknown>;
  readonly redaction_config?: {
    readonly use_defaults?: boolean;
    readonly regex_patterns: readonly string[];
    readonly sensitive_keys: readonly string[] | null;
    readonly replacement: string;
  };
  /** Config-key cases: dot-path -> value, pinning WHICH key path is read. */
  readonly config?: Record<string, unknown>;
  /** Canonical half of a case that also writes the SDK's legacy spelling. */
  readonly config_canonical?: Record<string, unknown>;
  /** Per-SDK legacy spelling; `null` means this SDK never had one. */
  readonly legacy_key_by_sdk?: Record<string, string | null>;
  /** Value to write at `legacy_key_by_sdk[SDK_LANGUAGE]`. */
  readonly legacy_value?: readonly string[];
  readonly skip_when_legacy_key_is_null?: string;
  readonly input: Record<string, unknown>;
  readonly expected: Record<string, unknown>;
}

interface RedactionFixture {
  readonly description: string;
  readonly test_cases: readonly RedactionCase[];
  readonly driver_contract?: Record<string, string>;
}

function loadFixture(name: string): RedactionFixture {
  return JSON.parse(fs.readFileSync(path.join(findFixturesRoot(), `${name}.json`), 'utf-8'));
}

const fixture = loadFixture('redaction_config');

/** The key this driver looks itself up under in `legacy_key_by_sdk`. */
const SDK_LANGUAGE = 'typescript';

/**
 * Non-field entries in an `expected` block.
 *
 * `_note` / `_comment` are fixture annotations; `deprecation_warning_*` are
 * assertions about the WARNING, asserted separately. Matched EXACTLY rather
 * than by a leading-underscore heuristic, which this helper used to do: the
 * config-key cases expect a literal `_secret_token` field, and dropping it as
 * an "annotation" would have silently discarded one of the two keys that make
 * those cases discriminating at all.
 */
const NON_FIELD_EXPECTATIONS = new Set([
  '_note',
  '_comment',
  'deprecation_warning_emitted',
  'deprecation_warning_is_one_shot',
]);

function expectedFields(expected: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(expected).filter(([k]) => !NON_FIELD_EXPECTATIONS.has(k)),
  );
}

/** Build the config-key surface named in the fixture description. */
function configFor(tc: RedactionCase): Config {
  const rc = tc.redaction_config!;
  const config = new Config();
  config.set('obs.redaction.regex_patterns', [...rc.regex_patterns]);
  if (rc.sensitive_keys !== null) {
    config.set('obs.redaction.sensitive_keys', [...rc.sensitive_keys]);
  }
  config.set('obs.redaction.replacement', rc.replacement);
  return config;
}

/** Build the same rules directly, honouring an empty `sensitive_keys` literally. */
function constructedFor(tc: RedactionCase): RedactionConfig {
  const rc = tc.redaction_config!;
  const sensitiveKeys = rc.sensitive_keys;
  if (sensitiveKeys === null) {
    // `sensitive_keys: null` + `use_defaults: true` == the shipped default list.
    return RedactionConfig.default();
  }
  return new RedactionConfig({
    fieldPatterns: [...sensitiveKeys],
    valuePatterns: rc.regex_patterns.map((p) => new RegExp(p, 'i')),
    replacement: rc.replacement,
  });
}

// ---------------------------------------------------------------------------
// Config-key cases
// ---------------------------------------------------------------------------

/**
 * The `observability.*` spellings this SDK actually falls back to, per
 * `RedactionConfig.fromConfig`. NOT a translation table — the fixture names
 * the key now. This exists only so a path the fixture attributes to
 * `typescript` that this SDK has never read fails loudly instead of being set
 * blindly, which would leave the case green while nothing read the key.
 */
/**
 * The replacement the config-key cases expect. None of them sets
 * `obs.redaction.replacement`, so they run on the SDK default — asserted in
 * each case body rather than trusted.
 */
const DEFAULT_REPLACEMENT = '***REDACTED***';

const KNOWN_LEGACY_KEYS: ReadonlySet<string> = new Set([
  'observability.redaction.field_patterns',
  'observability.redaction.value_patterns',
  'observability.redaction.replacement',
]);

/**
 * Guard one fixture-named dot-path before it is written to a Config.
 *
 * Canonical `obs.*` paths pass through untouched — a typo there MUST fail, not
 * be repaired.
 */
function assertKnownConfigKey(dotPath: string, tcId: string): string {
  if (dotPath.startsWith('obs.')) return dotPath;
  if (!KNOWN_LEGACY_KEYS.has(dotPath)) {
    throw new Error(
      `redaction_config.json case '${tcId}' names config key '${dotPath}' for ` +
        `'${SDK_LANGUAGE}', which apcore-typescript has never read. Either the fixture's ` +
        'legacy_key_by_sdk entry is wrong, or this SDK genuinely gained the key and ' +
        'KNOWN_LEGACY_KEYS needs it — do NOT let it through silently.',
    );
  }
  return dotPath;
}

/**
 * This SDK's legacy spelling for a case, straight out of the fixture.
 *
 * `undefined` -> the case names no legacy key at all (canonical-only case).
 * `null`      -> the case does not apply to this SDK and MUST be skipped.
 */
function legacyKeyForThisSdk(tc: RedactionCase): string | null | undefined {
  const bySdk = tc.legacy_key_by_sdk;
  if (bySdk === undefined) return undefined;
  if (!(SDK_LANGUAGE in bySdk)) {
    throw new Error(
      `redaction_config.json case '${tc.id}' has a legacy_key_by_sdk map with no ` +
        `'${SDK_LANGUAGE}' entry. Every SDK with a driver needs an entry (use null to mean ` +
        '"this SDK never had a legacy spelling"), otherwise the case silently stops ' +
        'covering this language.',
    );
  }
  return bySdk[SDK_LANGUAGE];
}

/**
 * The value to write at the legacy key.
 *
 * REPORTED FIXTURE GAP: `canonical_config_key_wins_over_legacy` states its
 * `legacy_value` explicitly, but
 * `legacy_config_key_is_honoured_with_a_deprecation_warning` does not — the
 * restructure into `legacy_key_by_sdk` moved the KEY out of the old `config`
 * block and dropped the VALUE that used to travel with it. Rather than
 * hard-code `["username"]` here (a driver-local invention of canonical fixture
 * content, the very thing this rewrite removes), the value is DERIVED from the
 * case's own `expected` block — the fields the case says must come back
 * redacted are exactly the keys the config has to name — and then cross-checked
 * against the canonical sibling that pins the same outcome through the
 * canonical path. If the fixture later grows an explicit `legacy_value`, it
 * wins; if the derivation and the sibling ever disagree, this goes red.
 */
function legacyValueFor(tc: RedactionCase, replacement: string): readonly string[] {
  if (tc.legacy_value !== undefined) return tc.legacy_value;

  const derived = Object.entries(expectedFields(tc.expected))
    .filter(([, value]) => value === replacement)
    .map(([field]) => field);
  expect(
    derived.length,
    `${tc.id}: cannot derive a legacy_value — no expected field is redacted`,
  ).toBeGreaterThan(0);

  const canonicalSibling = fixture.test_cases.find((c) => c.id === 'canonical_config_key_is_read');
  const canonicalValue = canonicalSibling?.config?.['obs.redaction.sensitive_keys'];
  expect(
    derived,
    `${tc.id}: derived legacy_value must match what canonical_config_key_is_read writes at ` +
      'the canonical key — the two cases pin the same outcome through different key paths',
  ).toEqual(canonicalValue);

  return derived;
}

/** Deprecation notices about the legacy redaction keys, in emission order. */
function legacyDeprecationWarnings(spy: MockInstance<typeof console.warn>): string[] {
  return spy.mock.calls
    .map((call) => String(call[0] ?? ''))
    .filter((message) => message.includes('observability.redaction'));
}

/**
 * A freshly-imported copy of the SDK, so the one-shot deprecation bookkeeping
 * (`_REDACTION_LEGACY_DEPRECATION_EMITTED`, module-level state in
 * context-logger.ts) starts unset for each case. Without this, whichever case
 * ran first would consume the single warning and the others would assert
 * against an already-fired latch.
 */
async function freshSdk(): Promise<{
  Config: typeof Config;
  RedactionConfig: typeof RedactionConfig;
}> {
  vi.resetModules();
  const configMod = await import('../src/config.js');
  const loggerMod = await import('../src/observability/context-logger.js');
  return { Config: configMod.Config, RedactionConfig: loggerMod.RedactionConfig };
}

const behaviourCases = fixture.test_cases.filter((c) => c.redaction_config !== undefined);
const configKeyCases = fixture.test_cases.filter(
  (c) =>
    c.config !== undefined || c.config_canonical !== undefined || c.legacy_key_by_sdk !== undefined,
);

describe('Conformance: redaction configuration (redaction_config.json)', () => {
  let warnSpy: MockInstance<typeof console.warn>;

  beforeEach(() => {
    // A legacy-key deprecation warning is possible; keep suite output clean.
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
  });

  // -------------------------------------------------------------------------
  // Behaviour cases (`redaction_config`)
  // -------------------------------------------------------------------------
  behaviourCases.forEach((tc) => {
    const expected = expectedFields(tc.expected);
    // `apply` is a documented SINGLE-LEVEL helper over a flat field map (the
    // `extra` dict at log emission); `redact` is the recursive one. A case
    // whose expectation depends on descending into a container is therefore a
    // case about `redact` only, and asserting `apply` on it would pin the flat
    // helper to a contract it does not have.
    //
    // The distinction is driven off the case's own DATA rather than an ID
    // allowlist: a future nested case is then covered correctly the day it
    // lands, instead of the day someone remembers to add its id here.
    const flat = Object.values(tc.input).every((v) => v === null || typeof v !== 'object');

    // `schema_marked_fields`: a case carrying `schema` goes through the entry
    // point the executor's input capture uses, which walks the schema's
    // `x-sensitive` markers (D-152) before the configured rules.
    const run = (rc: InstanceType<typeof RedactionConfig>): void => {
      if (tc.schema !== undefined) {
        expect(redactSensitive(structuredClone(tc.input), tc.schema, rc)).toEqual(expected);
        return;
      }
      if (flat) expect(rc.apply({ ...tc.input })).toEqual(expected);
      expect(rc.redact({ ...tc.input })).toEqual(expected);
    };

    it(`${tc.id} — via obs.redaction.* config keys`, () => {
      const rc = RedactionConfig.fromConfig(configFor(tc));
      expect(rc.replacement).toBe(tc.redaction_config!.replacement);
      run(rc);
    });

    it(`${tc.id} — via RedactionConfig constructor`, () => {
      run(constructedFor(tc));
    });
  });

  // -------------------------------------------------------------------------
  // Config-key cases — WHICH key path is read
  // -------------------------------------------------------------------------
  configKeyCases.forEach((tc) => {
    const legacyKey = legacyKeyForThisSdk(tc);

    // `null` means the case does not apply to this SDK. Per the fixture's
    // `skip_when_legacy_key_is_null`, that is a SKIP carrying the fixture's own
    // reason — not an xfail, which would assert the SDK is deficient. This
    // branch is inert for apcore-typescript today (our entry is a real key); it
    // exists so that retiring the legacy path here is a one-line fixture edit
    // rather than a driver rewrite.
    const runner = legacyKey === null ? it.skip : it;
    const skipReason =
      tc.skip_when_legacy_key_is_null ??
      fixture.driver_contract?.['legacy_spelling_is_per_sdk_history'] ??
      'legacy_key_by_sdk names no spelling for this SDK';
    const title =
      legacyKey === null
        ? `${tc.id} — N/A for ${SDK_LANGUAGE}: ${skipReason}`
        : `${tc.id} — built from the exact config key path the case names`;

    runner(title, async () => {
      const sdk = await freshSdk();
      const config = new sdk.Config();

      // Canonical half. `config` and `config_canonical` are the same thing —
      // the fixture renamed it on the cases that also write a legacy key.
      for (const [dotPath, value] of Object.entries({
        ...(tc.config ?? {}),
        ...(tc.config_canonical ?? {}),
      })) {
        config.set(assertKnownConfigKey(dotPath, tc.id), value);
      }

      // Legacy half, at THIS SDK's spelling as named by the fixture.
      if (legacyKey !== undefined && legacyKey !== null) {
        config.set(assertKnownConfigKey(legacyKey, tc.id), [
          ...legacyValueFor(tc, DEFAULT_REPLACEMENT),
        ]);
      }

      const rc = sdk.RedactionConfig.fromConfig(config);
      // No config-key case sets `obs.redaction.replacement`, so they all run on
      // the SDK default. Pinned rather than assumed: `legacyValueFor` uses this
      // string to tell which `expected` fields are redacted, and a drifted
      // default would make that derivation silently return nothing.
      expect(rc.replacement, `${tc.id}: default replacement`).toBe(DEFAULT_REPLACEMENT);

      const applied = rc.apply({ ...tc.input });

      // Redaction rewrites values; it never adds or drops fields. Asserted
      // because the per-field loop below is a SUBSET check (the fixture's
      // later cases name only the discriminating fields).
      expect(Object.keys(applied).sort()).toEqual(Object.keys(tc.input).sort());
      for (const [field, value] of Object.entries(expectedFields(tc.expected))) {
        expect(applied[field], `${tc.id}: ${field}`).toEqual(value);
      }
      // ContextLogger's surface must agree with ObsLoggingMiddleware's.
      expect(rc.redact({ ...tc.input })).toEqual(applied);

      // --- the deprecation warning ---
      if ('deprecation_warning_emitted' in tc.expected) {
        const warnings = legacyDeprecationWarnings(warnSpy);
        expect(
          warnings.length > 0,
          `${tc.id}: deprecation warning emitted; saw ${JSON.stringify(warnings)}`,
        ).toBe(tc.expected['deprecation_warning_emitted']);

        if (tc.expected['deprecation_warning_emitted'] === true) {
          // "naming the legacy key and its canonical replacement".
          expect(warnings[0], `${tc.id}: names the legacy key`).toContain(legacyKey);
          expect(warnings[0], `${tc.id}: names the canonical replacement`).toContain(
            'obs.redaction.sensitive_keys',
          );
        }
      }

      if (tc.expected['deprecation_warning_is_one_shot'] === true) {
        // One-shot per PROCESS, not per call: re-reading the same legacy
        // config must not warn again.
        sdk.RedactionConfig.fromConfig(config);
        sdk.RedactionConfig.fromConfig(config);
        expect(
          legacyDeprecationWarnings(warnSpy).length,
          `${tc.id}: the legacy-key deprecation warning must fire exactly once per process`,
        ).toBe(1);
      }
    });
  });

  it('the obs namespace is registered, so obs.redaction.* is a real config surface', () => {
    // The fixture describes configuration keys, not just an object API. If
    // `obs` were not a registered namespace, these keys would be dead weight in
    // a namespace-mode config even though `fromConfig` reads them.
    expect(Config.registeredNamespaces().map((ns) => ns.name)).toContain('obs');
  });

  it('drives every fixture case', () => {
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'regex_pattern_value_match',
      'sensitive_keys_substring_case_insensitive',
      'default_sensitive_keys_cover_common_terms',
      'correlation_fields_never_redacted',
      'canonical_config_key_is_read',
      'legacy_config_key_is_honoured_with_a_deprecation_warning',
      'canonical_config_key_wins_over_legacy',
      // The pattern DIALECT — PROTOCOL_SPEC 9.2.3 / 10.6.1 (#117).
      'sensitive_keys_glob_case_fold_applies_to_the_pattern_too',
      'sensitive_keys_bracket_is_a_literal_never_a_character_class',
      'sensitive_keys_glob_entry_is_anchored_to_the_whole_name',
      'regex_patterns_are_an_unanchored_search',
      // String values only — PROTOCOL_SPEC 10.6.1 requirement 2 (spec v1.40.0).
      'regex_patterns_do_not_test_non_string_values',
      'regex_patterns_still_reach_a_string_inside_a_container',
      // `x-sensitive` inside schema combinators — PROTOCOL_SPEC 10.6 / A13 (D-152).
      'x_sensitive_inside_an_any_of_branch_is_redacted',
      'x_sensitive_in_any_branch_redacts_the_value_whichever_branch_it_matches',
      'x_sensitive_in_an_all_of_branch_is_redacted',
      'properties_declared_inside_a_branch_are_walked',
      'a_combinator_without_a_marker_stays_visible',
      'array_items_marked_inside_a_branch_are_redacted',
    ]);

    // Every case belongs to exactly one shape. A case carrying neither a
    // `redaction_config` nor any config-key block would otherwise be counted
    // above and then driven by nothing.
    expect(behaviourCases.map((c) => c.id)).toEqual([
      'regex_pattern_value_match',
      'sensitive_keys_substring_case_insensitive',
      'default_sensitive_keys_cover_common_terms',
      'correlation_fields_never_redacted',
      'sensitive_keys_glob_case_fold_applies_to_the_pattern_too',
      'sensitive_keys_bracket_is_a_literal_never_a_character_class',
      'sensitive_keys_glob_entry_is_anchored_to_the_whole_name',
      'regex_patterns_are_an_unanchored_search',
      // String values only — PROTOCOL_SPEC 10.6.1 requirement 2 (spec v1.40.0).
      'regex_patterns_do_not_test_non_string_values',
      'regex_patterns_still_reach_a_string_inside_a_container',
      'x_sensitive_inside_an_any_of_branch_is_redacted',
      'x_sensitive_in_any_branch_redacts_the_value_whichever_branch_it_matches',
      'x_sensitive_in_an_all_of_branch_is_redacted',
      'properties_declared_inside_a_branch_are_walked',
      'a_combinator_without_a_marker_stays_visible',
      'array_items_marked_inside_a_branch_are_redacted',
    ]);
    expect(configKeyCases.map((c) => c.id)).toEqual([
      'canonical_config_key_is_read',
      'legacy_config_key_is_honoured_with_a_deprecation_warning',
      'canonical_config_key_wins_over_legacy',
    ]);
    expect(behaviourCases.filter((c) => configKeyCases.includes(c))).toEqual([]);
  });

  it('every case naming a legacy spelling names one for this SDK', () => {
    // The fixture's `legacy_spelling_is_per_sdk_history` contract, from this
    // driver's side: a `legacy_key_by_sdk` map that grows entries for other
    // languages but loses ours would silently stop covering TypeScript.
    // `legacyKeyForThisSdk` throws in that case; this is where it is exercised
    // for every case rather than only for the ones that run.
    const named = configKeyCases.filter((c) => c.legacy_key_by_sdk !== undefined);
    expect(named.length, 'at least one case must pin the legacy spelling').toBeGreaterThan(0);
    for (const tc of named) {
      // Throws if the `typescript` entry is missing entirely.
      const key = legacyKeyForThisSdk(tc);
      // A real spelling must be one this SDK reads; `null` (N/A) is also valid.
      if (key !== null) expect(KNOWN_LEGACY_KEYS.has(key!), `${tc.id}: ${key}`).toBe(true);
    }
    // This SDK does still have a legacy path, so nothing should be skipping.
    expect(
      named.map((c) => legacyKeyForThisSdk(c)),
      'apcore-typescript still honours its pre-D-53 spelling; a null here means the fixture ' +
        'believes the fallback was removed, which would be a spec change, not a driver change',
    ).toEqual(['observability.redaction.field_patterns', 'observability.redaction.field_patterns']);
  });

  it('the config-key cases use a payload that can tell the two behaviours apart', () => {
    // The fixture's `discriminating_payload` contract, asserted rather than
    // trusted: `username` must match NO shipped default (so redacting it
    // proves the override was read) and `password` must match one (so leaving
    // it alone proves the override REPLACED the defaults). If the shipped
    // default list ever grows a `username`-matching entry, these cases go
    // silently vacuous — this test is what stops that.
    const defaults = RedactionConfig.default();
    const untouched = defaults.apply({ username: 'alice', password: 'hunter2' });
    expect(untouched['username'], 'username must not match a shipped default').toBe('alice');
    expect(untouched['password'], 'password must match a shipped default').toBe('***REDACTED***');

    for (const tc of configKeyCases) {
      expect(Object.keys(tc.input), `${tc.id}: discriminating input`).toEqual(
        expect.arrayContaining(['username', 'password']),
      );
    }
  });
});
