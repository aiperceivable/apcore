/**
 * The §9.2.4 / §9.2.4.1 deprecation notices for configuration that does
 * nothing (apcore#118, PROTOCOL_SPEC v1.39.0).
 *
 * Ten declared configuration keys reach no consumer in any SDK, and an
 * `audit:` block in an ACL file has never been read by one. Both are now
 * announced. Neither announcement changes any behaviour: the keys still parse,
 * still validate, still answer `get()`, still pass `_config.strict`, and the
 * ACL file still loads exactly the rules it always did.
 *
 * **The half of each notice that matters is the SILENT half.** The notice is a
 * property of the *declared document* — `getDeclared`, never the merged view.
 * On a configuration that declares none of them, the merged view still
 * answers for one of them in legacy mode and two in namespace mode, because
 * `DEFAULTS` and the `observability` namespace registration supply values for
 * them. An implementation driven off `get()` would therefore warn for every
 * configuration ever loaded — the blanket warning §9.2.4 requirement 2
 * rejects. A test that only checked "it warns" would
 * pass such an implementation, so every case below pins the quiet side too.
 *
 * The ACL half has a second silent axis: the notice is scoped to `audit` on
 * purpose. It is a deprecation notice, not unknown-key closure for ACL files —
 * every other unrecognised root key must keep being ignored without a word.
 */

import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ACL } from '../src/acl.js';
import { Config } from '../src/config.js';

/**
 * The keys of PROTOCOL_SPEC §9.2.4, in the order the spec lists them and the
 * notice reports them.
 *
 * Ten when the window opened in spec v1.39.0; **seven** since v1.44.0, which
 * gave `observability.tracing.enabled` / `.sampling_rate` / `.exporter`
 * consumers (§10.1.1) and cancelled their withdrawal. The three that left are
 * pinned from the other side by `WIRED_KEYS` below — a table that never shrank
 * would pass every case here and fail those.
 *
 * Spelled out here rather than imported: `DEPRECATED_INERT_KEYS` is private to
 * `src/config.ts`, and a test that read it would agree with the code by
 * construction — including about the reporting order, which exists so that two
 * SDKs name the same keys the same way.
 */
const INERT_KEYS: readonly string[] = [
  'acl.default_effect',
  'observability.metrics.enabled',
  'observability.metrics.exporter',
  'logging.level',
  'logging.format',
  'acl.audit.enabled',
  'acl.audit.include_denied',
  'acl.audit.log_level',
  'middleware.disabled',
  'extensions.auto_discover',
];

/**
 * The three that spec v1.44.0 wired. Declaring one of these MUST NOT produce
 * the notice — §9.2.4 requirement 1: the table is the whole list.
 */
const WIRED_KEYS: Record<string, string> = {
  'observability.tracing.enabled': 'true',
  'observability.tracing.sampling_rate': '0.1',
  'observability.tracing.exporter': '"stdout"',
  'observability.tracing.strategy': '"off"',
};

/** A type-appropriate YAML scalar for each key. */
const INERT_KEY_VALUES: Record<string, string> = {
  'acl.default_effect': '"allow"',
  'observability.metrics.enabled': 'true',
  'observability.metrics.exporter': '"prometheus"',
  'logging.level': '"info"',
  'logging.format': '"json"',
  'acl.audit.enabled': 'true',
  'acl.audit.include_denied': 'true',
  'acl.audit.log_level': '"info"',
  'middleware.disabled': '["logging"]',
  'extensions.auto_discover': 'false',
};

const MINIMAL_YAML = 'version: "1.0.0"\nproject:\n  name: inert-keys-test\n';

/**
 * All ten in one document. Written out rather than assembled from `declare()`
 * per key, which would emit `observability:` twice and produce a
 * duplicate-mapping-key YAML error.
 */
const ALL_INERT_YAML = `acl:
  default_effect: "allow"
  audit:
    enabled: true
    include_denied: true
    log_level: "info"
logging:
  level: "info"
  format: "json"
observability:
  metrics:
    enabled: true
    exporter: "prometheus"
middleware:
  disabled: ["logging"]
extensions:
  auto_discover: false
`;

/** Render a dotted key as the nested YAML block that declares it. */
function declare(key: string, value: string): string {
  const parts = key.split('.');
  return `${parts
    .map((part, i) =>
      i === parts.length - 1
        ? `${'  '.repeat(i)}${part}: ${value}`
        : `${'  '.repeat(i)}${part}:`,
    )
    .join('\n')}\n`;
}

let tmpDir: string;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'apcore-inert-keys-'));
});

afterEach(() => {
  rmSync(tmpDir, { recursive: true, force: true });
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

function write(name: string, body: string): string {
  const filePath = join(tmpDir, name);
  writeFileSync(filePath, body, 'utf-8');
  return filePath;
}

function spyOnWarn() {
  return vi.spyOn(console, 'warn').mockImplementation(() => {});
}

describe('the §9.2.4 inert-configuration-key notice (apcore#118)', () => {
  /** Lines emitted by this notice, distinct from the ACL notice. */
  function noticesFrom(spy: MockInstance<typeof console.warn>): string[] {
    return spy.mock.calls
      .map((call) => String(call[0]))
      .filter((line) => line.startsWith('[apcore:config] DEPRECATION (apcore#118'));
  }

  it.each(Object.keys(WIRED_KEYS))(
    'is SILENT for %s, which spec v1.44.0 wired',
    (key) => {
      // §9.2.4 requirement 1 — the table is the whole list, and a key that has
      // left it MUST NOT warn. This is the half that fails against a table
      // which never shrank; the cases above pass either way.
      const configPath = write(
        'apcore.yaml',
        MINIMAL_YAML + declare(key, WIRED_KEYS[key] as string),
      );
      const warn = spyOnWarn();

      Config.load(configPath);

      expect(noticesFrom(warn)).toEqual([]);
    },
  );

  it.each(INERT_KEYS)('warns for a configuration that declares %s, and names it', (key) => {
    const configPath = write(
      'apcore.yaml',
      MINIMAL_YAML + declare(key, INERT_KEY_VALUES[key] as string),
    );
    const warn = spyOnWarn();

    Config.load(configPath);

    const notices = noticesFrom(warn);
    expect(notices).toHaveLength(1);
    expect(notices[0]).toContain(key);
    expect(notices[0]).toContain('1 key(s)');
    // Only the declared one. A notice that named all ten would be no more
    // actionable than no notice at all.
    for (const other of INERT_KEYS.filter((k) => k !== key)) {
      expect(notices[0]).not.toContain(other);
    }
  });

  it('names all ten, once, in the §9.2.4 order when a configuration declares all ten', () => {
    const configPath = write('apcore.yaml', MINIMAL_YAML + ALL_INERT_YAML);
    const warn = spyOnWarn();

    Config.load(configPath);

    const notices = noticesFrom(warn);
    expect(notices).toHaveLength(1);
    expect(notices[0]).toContain('10 key(s)');
    // The order is part of the contract: two SDKs reporting the same document
    // must produce the same list.
    expect(notices[0]).toContain(INERT_KEYS.join(', '));
  });

  it('is SILENT for a legacy-mode configuration that declares none of them', () => {
    // The requirement, not a nicety. Every assertion below holds on an
    // implementation driven off the merged view too — except the last one.
    const configPath = write('apcore.yaml', MINIMAL_YAML);
    const warn = spyOnWarn();

    const config = Config.load(configPath);

    for (const key of INERT_KEYS) {
      expect(config.getDeclared(key)).toBeUndefined();
    }
    // ...and yet the MERGED view answers for one of them, out of `DEFAULTS`.
    // That is what makes the silence a real assertion: a merged-view check
    // would fire here, on a document that mentions none of these keys, and so
    // would fire on every configuration this SDK has ever loaded.
    const answeredByMergedView = INERT_KEYS.filter((key) => config.get(key) !== undefined);
    expect(answeredByMergedView).toContain('observability.metrics.enabled');

    expect(noticesFrom(warn)).toEqual([]);
  });

  it('is SILENT for a namespace-mode configuration that declares none of them', () => {
    // A separate merge path, and the worse one for this trap: the
    // `observability` namespace registration seeds every `observability.*` key
    // into the merged tree, so a merged-view check has more triggers here than
    // in legacy mode.
    const configPath = write(
      'apcore.yaml',
      'apcore:\n  version: "1.0.0"\n  project:\n    name: inert-keys-ns\n',
    );
    const warn = spyOnWarn();

    const config = Config.load(configPath);

    for (const key of INERT_KEYS) {
      expect(config.getDeclared(key)).toBeUndefined();
    }
    const answeredByMergedView = INERT_KEYS.filter((key) => config.get(key) !== undefined);
    expect(answeredByMergedView).toEqual([
      'observability.metrics.enabled',
      'observability.metrics.exporter',
    ]);

    expect(noticesFrom(warn)).toEqual([]);
  });

  it('is SILENT for live keys that sit beside the inert ones in the same sections', () => {
    // `acl.root` shares the `acl:` parent with `acl.audit.*`;
    // `stream.max_merge_depth` and `validation.binding.version_require_semver`
    // are the two keys apcore#118 made live rather than deprecating. A
    // prefix-matched check would trip on the first.
    //
    // `acl.default_effect` used to stand here too. Spec v1.47.0 moved it INTO
    // the table (§9.1.3's first application): an ACL's default effect is read
    // from the ACL file, and this twin reaches nothing.
    const configPath = write(
      'apcore.yaml',
      `${MINIMAL_YAML}acl:\n  root: "./acl"\nstream:\n  max_merge_depth: 8\nvalidation:\n  binding:\n    version_require_semver: true\n`,
    );
    const warn = spyOnWarn();

    Config.load(configPath);

    expect(noticesFrom(warn)).toEqual([]);
  });

  it('counts the environment tier as a declaration', () => {
    // §9.2 makes `APCORE_*` an override, and `Config.load` runs it over the raw
    // document to build the declared view. An operator who exports
    // `APCORE_LOGGING_LEVEL` has set an inert key just as surely as one who
    // wrote it in the file, and gets told so.
    const configPath = write('apcore.yaml', MINIMAL_YAML);
    vi.stubEnv('APCORE_LOGGING_LEVEL', 'debug');
    const warn = spyOnWarn();

    Config.load(configPath);

    const notices = noticesFrom(warn);
    expect(notices).toHaveLength(1);
    expect(notices[0]).toContain('logging.level');
    expect(notices[0]).not.toContain('logging.format');
  });

  it('fires once per LOAD, not once per process', () => {
    // The notice belongs to the document loaded, regardless of prior loads.
    const configPath = write('apcore.yaml', MINIMAL_YAML + declare('logging.level', '"info"'));
    const warn = spyOnWarn();

    Config.load(configPath);
    Config.load(configPath);
    Config.load(configPath);

    expect(noticesFrom(warn)).toHaveLength(3);
  });

  it('changes no behaviour: all ten still parse, validate under strict, and answer get()', () => {
    // §9.2.4 requirement 3. Withdrawing these keys cannot be a plain deletion
    // precisely because a configuration carrying them is valid TODAY under
    // `_config.strict: true`; deleting one would turn a currently-valid
    // document into a rejected one, which §13.2 puts behind a two-minor floor.
    const configPath = write(
      'apcore.yaml',
      `${MINIMAL_YAML}_config:\n  strict: true\n${ALL_INERT_YAML}`,
    );
    spyOnWarn();

    const config = Config.load(configPath);

    expect(() => config.validate()).not.toThrow();
    expect(config.get('logging.level')).toBe('info');
    expect(config.get('logging.format')).toBe('json');
    expect(config.get('observability.metrics.exporter')).toBe('prometheus');
    expect(config.get('acl.audit.include_denied')).toBe(true);
  });
});

describe("the §9.2.4.1 ACL-file `audit:` notice (apcore#118)", () => {
  const RULES = 'rules:\n  - callers: ["api.*"]\n    targets: ["executor.*"]\n    effect: allow\n';

  function aclNoticesFrom(spy: MockInstance<typeof console.warn>): string[] {
    return spy.mock.calls
      .map((call) => String(call[0]))
      .filter((line) => line.startsWith('[apcore:acl] DEPRECATION (apcore#118'));
  }

  it('no longer produces a deprecation notice for an audit: block', () => {
    // §9.2.4.1's notice is SUPERSEDED by §6.3.2 (spec v1.45.0). The block had
    // never been read, so the notice was the only signal an operator got. It is
    // now read — it has a delivery contract — and a key that has gained a
    // consumer must stop being announced as going away, for the same reason
    // §9.2.4 requirement 1 says the table is the whole list.
    const aclPath = write(
      'global_acl.yaml',
      `default_effect: deny\n${RULES}audit:\n  enabled: true\n  include_denied: true\n  log_level: "info"\n`,
    );
    const warn = spyOnWarn();

    const acl = ACL.load(aclPath);

    expect(aclNoticesFrom(warn)).toEqual([]);
    expect(acl.rules).toHaveLength(1);
  });

  it('is SILENT for an ACL file with no audit: block', () => {
    const aclPath = write('global_acl.yaml', `default_effect: deny\n${RULES}`);
    const warn = spyOnWarn();

    ACL.load(aclPath);

    expect(aclNoticesFrom(warn)).toEqual([]);
  });

  it('is SILENT for an ACL file carrying some OTHER unknown root key', () => {
    // The scope of the notice is the point. This loader casts the parsed
    // document to an open record and takes the fields it wants, so every
    // unrecognised root key is dropped in silence — and must stay that way.
    // This is a deprecation notice for one withdrawn block, NOT unknown-key
    // closure for ACL files, which would be a behaviour change refusing or
    // flagging documents that load cleanly today.
    const aclPath = write(
      'global_acl.yaml',
      `default_effect: deny\n${RULES}version: "1.0"\nmetadata:\n  owner: "platform-team"\nauditing:\n  enabled: true\n`,
    );
    const warn = spyOnWarn();

    ACL.load(aclPath);

    expect(aclNoticesFrom(warn)).toEqual([]);
    // Nor does anything else complain about them.
    expect(warn.mock.calls.map((call) => String(call[0]))).toEqual([]);
  });

  it('treats an empty audit: block as DECLARED, with every setting at its default', () => {
    // `audit:` with nothing under it parses to null, and the operator still
    // wrote the block. §6.3.2 requirement 2 makes declaration the switch, so
    // presence — not truthiness — is what activates the default sink. Read off
    // the behaviour rather than off a notice, because the notice is gone.
    const aclPath = write('global_acl.yaml', `default_effect: deny\n${RULES}audit:\n`);
    const acl = ACL.load(aclPath);

    const info = vi.spyOn(console, 'info').mockImplementation(() => {});
    acl.check('api.x', 'executor.y');
    const emitted = info.mock.calls.filter((c) => c[0] === 'apcore.acl.audit');
    info.mockRestore();

    expect(emitted).toHaveLength(1);
  });

  it('changes no behaviour: the file loads and the block stays ignored', () => {
    const aclPath = write(
      'global_acl.yaml',
      `default_effect: deny\n${RULES}audit:\n  enabled: true\n  log_level: "info"\n`,
    );
    spyOnWarn();

    const acl = ACL.load(aclPath);

    expect(acl.defaultEffect).toBe('deny');
    expect(acl.rules).toHaveLength(1);
    expect(acl.rules[0]?.callers).toEqual(['api.*']);
    expect(acl.rules[0]?.targets).toEqual(['executor.*']);
    // The block reaches nothing — it is not a rule, and it does not become one.
    expect(acl.check('api.orders', 'executor.email.send_email')).toBe(true);
    expect(acl.check('executor.email.send_email', 'api.orders')).toBe(false);
  });
});
