/** Verify CI's canonical spec dependency before loading all conformance drivers. */
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import yaml from 'js-yaml';
import { afterEach, describe, expect, it } from 'vitest';
import { findFixturesRoot } from './spec-repo.js';

interface CiStep {
  name?: string;
  run?: string;
  with?: { repository?: string; ref?: string; path?: string };
}

interface CiJob {
  env?: Record<string, string>;
  steps: CiStep[];
}

function readCiJob(): CiJob {
  const workflow = yaml.load(readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8')) as {
    jobs: { 'build-and-test': CiJob };
  };
  return workflow.jobs['build-and-test'];
}

function verifyDeclaredFixtureCheckout(root: string, names: readonly string[]): void {
  const missing = names.map((name) => `${name}.json`).filter((name) => {
    const file = join(root, name);
    return !existsSync(file) || !statSync(file).isFile();
  }).sort();
  if (missing.length > 0) {
    throw new Error(
      `Canonical fixtures missing from ${root}: ${missing.join(', ')}. ` +
      'Publish the apcore spec changes before the SDK changes, then rerun CI. ' +
      'Do not skip these fixtures or substitute private copies.',
    );
  }
}

const temporaryRoots: string[] = [];
afterEach(() => {
  for (const root of temporaryRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('canonical conformance checkout', () => {
  it('checks out spec main independently of SDK branches', () => {
    const checkouts = readCiJob().steps.filter((step) => step.with?.repository === 'aiperceivable/apcore');
    expect(checkouts).toHaveLength(1);
    expect(checkouts[0].with).toMatchObject({ ref: 'main', path: '.apcore-spec' });
  });

  it('runs the fixture preflight after dependency installation and before full tests', () => {
    const job = readCiJob();
    expect(job.env?.CONFORMANCE_SPEC_REPO).toBe('${{ github.workspace }}/.apcore-spec');
    const probes = job.steps.flatMap((step, index) =>
      step.run === 'pnpm exec vitest run tests/test-conformance-checkout.test.ts' ? [index] : []);
    expect(probes).toHaveLength(1);
    expect(probes[0]).toBeGreaterThan(job.steps.findIndex((step) => step.name === 'Install dependencies'));
    expect(probes[0]).toBeLessThan(job.steps.findIndex((step) => step.name === 'Run tests'));
  });

  it('reports every missing fixture with publication-order guidance', () => {
    const root = mkdtempSync(join(tmpdir(), 'apcore-fixture-checkout-'));
    temporaryRoots.push(root);
    const names = ['canonicalize_name', 'binding_file_validation'];
    expect(() => verifyDeclaredFixtureCheckout(root, names)).toThrow(
      'binding_file_validation.json, canonicalize_name.json. Publish the apcore spec changes before the SDK changes',
    );
    writeFileSync(join(root, 'binding_file_validation.json'), '{}');
    expect(() => verifyDeclaredFixtureCheckout(root, names)).toThrow('canonicalize_name.json');
    writeFileSync(join(root, 'canonicalize_name.json'), '{}');
    expect(() => verifyDeclaredFixtureCheckout(root, names)).not.toThrow();
  });

  it('contains every fixture named in the SDK conformance declaration', () => {
    const declaration = yaml.load(readFileSync(new URL('../apcore-conformance.yaml', import.meta.url), 'utf8')) as {
      conformance: { fixture_results: { fixture_names: string[] } };
    };
    verifyDeclaredFixtureCheckout(findFixturesRoot(), declaration.conformance.fixture_results.fixture_names);
  });
});
