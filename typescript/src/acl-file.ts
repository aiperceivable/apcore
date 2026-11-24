/**
 * Side-effect module: installs the Node-side YAML loader on `ACL.load`.
 *
 * Imported by the package's Node entry (`src/index.ts`). The browser
 * entry intentionally does NOT import this file — `ACL.load(...)` then
 * throws a clear runtime error directing the caller to construct ACL
 * programmatically.
 *
 * `node:fs` lives only on this leaf so the browser closure stays clean.
 */

import { existsSync, readFileSync, statSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import yaml from 'js-yaml';
import {
  ACL,
  AUDIT_FIELDS,
  _parseAclRule,
  _rejectInvalidDefaultEffect,
  _setAclFileLoader,
  _setAclDiscoverer,
} from './acl.js';
import type { AclConfigLike, AuditConfig, AuditLogger } from './acl.js';
import { getDefault } from './config-defaults.js';
import { ACLRuleError, ConfigError, ConfigNotFoundError } from './errors.js';

/**
 * Validate an ACL file's `audit:` block (PROTOCOL_SPEC §6.3.2 requirement 8).
 *
 * Returns `null` only when the document declares no `audit` key at all — the
 * distinction requirement 2 turns on, because `enabled` defaults to `true` and
 * reading the merged view would switch a log record per check on for every ACL
 * file in existence.
 *
 * Validates the SUBTREE only: types and unknown keys inside the block. Every
 * other unrecognised root key in an ACL file keeps being ignored.
 */
function parseAuditBlock(data: Record<string, unknown>, yamlPath: string): AuditConfig | null {
  if (!('audit' in data)) return null;
  const raw = data['audit'];
  // Presence, not truthiness. `audit:` with nothing under it parses to null,
  // and the operator still wrote the block: a declaration with every setting
  // at its default, not an absence.
  if (raw === null || raw === undefined) {
    return { enabled: true, include_denied: true, log_level: 'info' };
  }
  if (typeof raw !== 'object' || Array.isArray(raw)) {
    throw new ConfigError(
      `${yamlPath}: 'audit' must be a mapping (PROTOCOL_SPEC §6.3.2), got ${typeof raw}`,
    );
  }
  const block = raw as Record<string, unknown>;

  const unknown = Object.keys(block)
    .filter((k) => !(AUDIT_FIELDS as readonly string[]).includes(k))
    .sort();
  if (unknown.length > 0) {
    throw new ConfigError(
      `${yamlPath}: unknown key(s) in the 'audit' block: ${unknown.join(', ')}. The block ` +
        `accepts exactly ${AUDIT_FIELDS.join(', ')} ($defs/AuditConfig in ` +
        `schemas/acl-config.schema.json).`,
    );
  }

  const config: AuditConfig = { enabled: true, include_denied: true, log_level: 'info' };
  for (const key of ['enabled', 'include_denied'] as const) {
    if (key in block) {
      if (typeof block[key] !== 'boolean') {
        throw new ConfigError(
          `${yamlPath}: 'audit.${key}' must be a boolean, got ${JSON.stringify(block[key])}`,
        );
      }
      config[key] = block[key] as boolean;
    }
  }
  if ('log_level' in block) {
    const levels = ['trace', 'debug', 'info', 'warn', 'error'];
    if (typeof block['log_level'] !== 'string' || !levels.includes(block['log_level'])) {
      throw new ConfigError(
        `${yamlPath}: 'audit.log_level' must be one of ${levels.join(', ')}, got ` +
          `${JSON.stringify(block['log_level'])}`,
      );
    }
    config.log_level = block['log_level'] as AuditConfig['log_level'];
  }

  if (!config.include_denied) {
    // §6.3.2 requirement 6 — a notice, not a refusal. It withholds the
    // security-relevant half of the record, so an operator who wrote it
    // deliberately gets told once per load rather than stopped.
    console.warn(
      `[apcore:acl] ${yamlPath} sets audit.include_denied: false, so DENIED access ` +
        `attempts will not be recorded by the default audit sink (PROTOCOL_SPEC §6.3.2 ` +
        `requirement 6). Allowed calls are still recorded. Remove the entry to restore ` +
        `denials.`,
    );
  }
  return config;
}


_setAclFileLoader((yamlPath: string, auditLogger?: AuditLogger | null): ACL => {
  if (!existsSync(yamlPath)) {
    throw new ConfigNotFoundError(yamlPath);
  }

  let data: unknown;
  try {
    const content = readFileSync(yamlPath, 'utf-8');
    data = yaml.load(content);
  } catch (e) {
    if (e instanceof ConfigNotFoundError) throw e;
    throw new ACLRuleError(`Invalid YAML in ${yamlPath}: ${e}`);
  }

  if (typeof data !== 'object' || data === null || Array.isArray(data)) {
    throw new ACLRuleError(`ACL config must be a mapping, got ${typeof data}`);
  }

  const dataObj = data as Record<string, unknown>;

  // PROTOCOL_SPEC §9.2.4.1 (apcore#118): an `audit:` block in an ACL file has
  // never been read. Deleting it from `acl-config.schema.json` would produce no
  // signal at all — no implementation validates an ACL file against that
  // schema, and this loader casts to an open record and takes the fields it
  // wants, so any unknown root key is dropped in silence. The diagnostic
  // therefore has to live here.
  //
  // Scoped to `audit` deliberately: §6.3.2 requirement 8 validates this SUBTREE
  // and nothing else, so every other unrecognised root key in an ACL file keeps
  // being ignored exactly as before. This was never unknown-key closure for ACL
  // files, and wiring the block does not make it one.
  //
  // The §9.2.4.1 deprecation notice that used to stand here is gone: spec
  // v1.45.0 gave the block a delivery contract, and a key that has gained a
  // consumer must stop being announced as going away.
  const auditConfig = parseAuditBlock(dataObj, yamlPath);

  // §6.2.1 point 2 (v1.31.0, #112) — `default_effect` is judged FIRST, before
  // any rule. It is not a rule and has no index, so the rule ordering never
  // reaches it, and a file wrong in both was refused for its rule here and for
  // `default_effect` at the constructor door, because the rules were all parsed
  // on the way to it. The same function runs at both doors, so there is one
  // check and one message.
  //
  // `?? 'deny'` alone would coerce an ABSENT key and an explicit
  // `default_effect: null` to the same fallback — `??` treats a `null` read
  // off the object identically to an `undefined` one, so it cannot tell
  // "key absent" from "key present with value null" apart. Only the absent
  // case is a real default; an explicit `null` is a value the operator
  // wrote, and `schemas/acl-config.schema.json` declares `default_effect` as a
  // plain string enum, not nullable, so it is exactly as invalid as `"block"`
  // and must reach `_rejectInvalidDefaultEffect` unchanged rather than being
  // silently normalized away before that check ever sees it.
  const hasDefaultEffectKey = 'default_effect' in dataObj;
  const defaultEffect = hasDefaultEffectKey ? (dataObj['default_effect'] as string) : 'deny';
  _rejectInvalidDefaultEffect(defaultEffect);

  if (!('rules' in dataObj)) {
    throw new ACLRuleError("ACL config missing required 'rules' key");
  }

  const rawRules = dataObj['rules'];
  if (!Array.isArray(rawRules)) {
    throw new ACLRuleError(`'rules' must be a list, got ${typeof rawRules}`);
  }

  // One pass, in file order, and every per-rule check lives inside
  // `_parseAclRule` — the rule-key closure (#107) included, which is a
  // loader-only axis and therefore exactly the kind §6.2.1 point 2's sweep
  // prohibition binds. Sweeping any one of them across the file first would
  // refuse a later rule for a fault a lower-indexed rule already had.
  const rules = rawRules.map((raw, i) => _parseAclRule(raw, i));

  const acl = new ACL(rules, defaultEffect, auditLogger ?? null, auditConfig);
  acl._setYamlPath(yamlPath);
  return acl;
});

// ---------------------------------------------------------------------------
// Config-driven ACL discovery (D-64, Recommendation A — issue #74)
// ---------------------------------------------------------------------------

_setAclDiscoverer((config: AclConfigLike): ACL | null => {
  // Read `acl.root`, falling back to the canonical default ("./acl").
  const rawRoot = config.get('acl.root', getDefault('acl.root'));
  if (rawRoot === null || rawRoot === undefined) {
    return null;
  }

  let rootPath = String(rawRoot);
  if (!isAbsolute(rootPath)) {
    // Anchor a relative root at the config file's directory when known,
    // otherwise at the process CWD. Parity with apcore-python
    // (Config.source_path) and apcore-rust (D-64).
    const sourcePath = config.sourcePath;
    const base = sourcePath !== null ? dirname(resolve(sourcePath)) : process.cwd();
    rootPath = resolve(base, rootPath);
  }

  // Missing path => no enforcement. CRITICAL: do NOT synthesize an empty
  // default-deny ACL — that would silently deny every inter-module call in
  // every project lacking an acl file. `acl.default_effect` only applies once
  // a real ACL file is loaded (read by ACL.load from the file itself).
  if (!existsSync(rootPath)) {
    return null;
  }

  // acl.root is a directory by convention (the default "./acl"): load the
  // conventional `<root>/global_acl.yaml` (PROTOCOL_SPEC §3.1 `acl/{scope}_acl.yaml`).
  // A directory without that file is a no-op. acl.root MAY also point directly
  // at a YAML file. Parity with apcore-python and apcore-rust.
  if (statSync(rootPath).isDirectory()) {
    const globalAcl = join(rootPath, 'global_acl.yaml');
    if (!existsSync(globalAcl)) {
      return null;
    }
    return ACL.load(globalAcl);
  }

  return ACL.load(rootPath);
});
