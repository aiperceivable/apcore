/**
 * Canonical conformance-fixture locator, shared by every conformance driver.
 *
 * The fixtures are the single source of truth in the apcore spec repo; this
 * SDK reads them in place rather than vendoring a copy, so a spec-side edit
 * reaches TypeScript on the next test run.
 *
 * Search order:
 *   1. `$CONFORMANCE_SPEC_REPO` — the spec repo root (set by CI).
 *   2. `../apcore/` beside this repo — the standard workspace layout.
 */
import * as fs from 'node:fs';
import * as path from 'node:path';

const SPEC_REPO_ENV = 'CONFORMANCE_SPEC_REPO';

/**
 * Transitional fallback (apcore#86). The locator used to be
 * `APCORE_SPEC_REPO`, but PROTOCOL_SPEC §9.2 makes *every* `APCORE_*` variable
 * a config override: the suffix is lowercased and split into a dot path, so
 * `APCORE_SPEC_REPO=/path` injected `spec.repo` into the declared config
 * document that §9.1's required-field check runs against. The locator is test
 * infrastructure, not configuration, so it moved out of the claimed prefix.
 * Reading the old name keeps a developer who still exports it working.
 * REMOVE once all three SDK CI workflows are on CONFORMANCE_SPEC_REPO.
 */
const LEGACY_SPEC_REPO_ENV = 'APCORE_SPEC_REPO';

/**
 * A `conformance/fixtures` DIRECTORY, taking precedence over the repo-root
 * form (conformance.md §8.2.1 rule 1).
 *
 * It names a directory with no repository around it, which is what makes it
 * useful: a driver can run against a SYNTHESISED fixture set — an older shape,
 * a single edited case — without producing a whole spec repo to hold it.
 * Drivers land one push before fixtures here, so a driver must tolerate the
 * fixture shape that predates the keys it reads, and that cannot be verified
 * from a working tree which already holds the newer fixture.
 */
const FIXTURES_ENV = 'CONFORMANCE_FIXTURES';

/**
 * Transitional fallback for {@link FIXTURES_ENV} (apcore#88), the exact twin of
 * {@link LEGACY_SPEC_REPO_ENV} below and for the same reason: PROTOCOL_SPEC
 * §9.2 lowers every `APCORE_*` variable to a config key, so `APCORE_FIXTURES`
 * declared `fixtures` in a document no schema knows about.
 * REMOVE once all three SDK CI workflows are on CONFORMANCE_FIXTURES.
 */
const LEGACY_FIXTURES_ENV = 'APCORE_FIXTURES';

/**
 * Return the fixtures-directory override as `[variableName, value]`.
 *
 * Same shape and same reason as {@link specRepoEnv}: the name travels with the
 * value so a failure message names the variable the developer actually set.
 */
export function fixturesEnv(): readonly [string, string] | undefined {
  for (const name of [FIXTURES_ENV, LEGACY_FIXTURES_ENV]) {
    const value = process.env[name];
    if (value) return [name, value] as const;
  }
  return undefined;
}

/**
 * Return the spec-repo override as `[variableName, value]`, or `undefined`.
 *
 * The name travels with the value so a failure message can name the variable
 * the developer actually set rather than the one they did not.
 */
export function specRepoEnv(): readonly [string, string] | undefined {
  for (const name of [SPEC_REPO_ENV, LEGACY_SPEC_REPO_ENV]) {
    const value = process.env[name];
    if (value) return [name, value] as const;
  }
  return undefined;
}

/**
 * Resolve the canonical `schemas` directory of the apcore spec repo, or throw.
 *
 * Deliberately does **not** consult `CONFORMANCE_FIXTURES` (conformance.md
 * §8.2.1 rule 4): that variable names one directory rather than a repo, so
 * there is nothing to append `schemas/` to. The call site used to reach here as
 * `path.resolve(findFixturesRoot(), '..', '..', 'schemas')`, which is correct
 * only when the fixtures were resolved through a repo root — set
 * `CONFORMANCE_FIXTURES` to a bare directory and it reads somewhere else.
 */
export function findSchemasRoot(): string {
  const env = specRepoEnv();
  if (env) {
    const [name, value] = env;
    const schemas = path.join(value, 'schemas');
    if (fs.existsSync(schemas)) return schemas;
    throw new Error(`${name}=${value} does not contain schemas/`);
  }

  const repoRoot = path.resolve(__dirname, '..');
  const sibling = path.resolve(repoRoot, '..', 'apcore', 'schemas');
  if (fs.existsSync(sibling)) return sibling;

  throw new Error(
    'Cannot find the apcore schemas.\n\n' +
      'Fix one of:\n' +
      `  1. Set ${SPEC_REPO_ENV} to the apcore spec repo path\n` +
      `  2. Clone apcore as a sibling at ${path.resolve(repoRoot, '..', 'apcore')}\n\n` +
      `Note: ${FIXTURES_ENV} does not help here — it names a fixtures directory, ` +
      'not a repo (conformance.md §8.2.1 rule 4).',
  );
}

/**
 * Resolve the canonical `conformance/fixtures` directory, or throw.
 *
 * conformance.md §8.2.1: `CONFORMANCE_FIXTURES` (a directory), then
 * `CONFORMANCE_SPEC_REPO` (a repo root), then a sibling checkout. A variable
 * that is set but does not resolve throws rather than falling through to the
 * next source — silently testing against different fixtures than the operator
 * named is worse than not running.
 */
export function findFixturesRoot(): string {
  const fixtures = fixturesEnv();
  if (fixtures) {
    const [name, value] = fixtures;
    if (fs.existsSync(value) && fs.statSync(value).isDirectory()) return value;
    throw new Error(`${name}=${value} is not a directory`);
  }

  const env = specRepoEnv();
  if (env) {
    const [name, value] = env;
    const fixtures = path.join(value, 'conformance', 'fixtures');
    if (fs.existsSync(fixtures)) return fixtures;
    throw new Error(`${name}=${value} does not contain conformance/fixtures/`);
  }

  const repoRoot = path.resolve(__dirname, '..'); // apcore-typescript/
  const sibling = path.resolve(repoRoot, '..', 'apcore', 'conformance', 'fixtures');
  if (fs.existsSync(sibling)) return sibling;

  throw new Error(
    'Cannot find apcore conformance fixtures.\n\n' +
      'Fix one of:\n' +
      `  1. Set ${SPEC_REPO_ENV} to the apcore spec repo path\n` +
      `  2. Set ${FIXTURES_ENV} to a conformance/fixtures directory\n` +
      `  3. Clone apcore as a sibling: git clone <apcore-url> ${path.resolve(repoRoot, '..', 'apcore')}\n`,
  );
}
