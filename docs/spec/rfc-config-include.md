---
description: "Proposed RFC for a top-level include: key that composes apcore.yaml from YAML fragments: declaring-file-relative paths, deep merge with local-wins precedence, recursive includes, cycle detection. Not implemented."
---

# RFC — `include:` Cross-File Configuration Composition

## Status

**Proposed — awaiting maintainer decision.** Tracked by [#75](https://github.com/aiperceivable/apcore/issues/75) and decision **D-65** in the [2026-05 decision log](./2026-05-decision-log.md). No SDK implements `include:`, and `schemas/apcore-config.schema.json` does not declare it. This document is the contract to ratify before any code lands.

## Motivation

A project's `apcore.yaml` grows monolithic: ACL policy, pipeline config, observability, and per-environment overrides all live in one file. Teams want to split shared configuration into reusable fragments — a base file plus environment overlays, or a shared ACL/observability block included by several services. Today the only option is to duplicate YAML or assemble it with an external templating step before apcore sees it.

`include:` lets one `apcore.yaml` pull in other YAML fragments and merge them, declaratively, inside the SDK — with identical semantics across Python, TypeScript, and Rust.

## Non-goals

Out of scope (each would need its own RFC):

- **Glob / wildcard includes** (`include: ["conf.d/*.yaml"]`). Explicit file lists only.
- **Remote includes** (http(s) URLs, package refs). Local filesystem paths only.
- **Conditional / environment-gated includes.** Environment differences stay in the existing `APCORE_*` override mechanism.
- **List-element merging** (append/dedup). Lists are replaced wholesale.
- **Reserved namespaces or version fields on the include block.** The key is a plain list of strings.

## Proposed syntax

A top-level `include:` key whose value is a list of file-path strings:

```yaml
# apcore.yaml
include:
  - ./base/observability.yaml
  - ./base/acl.yaml
  - ./env/production.yaml      # later entries override earlier ones

# Local keys below override anything pulled in by include:
executor:
  default_timeout: 30000
```

- `include` MUST be a list of strings. A scalar, mapping, or non-string element is a config error.
- Each entry is resolved **relative to the directory of the file that declares it** (not the root file, not the working directory).
- An included file MAY itself declare `include:` — processed recursively, depth-first.

## Merge semantics

Expansion is a pre-processing phase that runs **before** validation, environment overrides, and binding. It produces one merged mapping that the rest of the loader consumes unchanged.

1. **Order.** For a file with `include: [A, B]` and its own keys L, precedence is `A < B < L`: includes in listed order, then the declaring file's own keys. Local keys always win; later includes win over earlier ones.
2. **Deep merge for mappings.** When both sides hold a mapping at the same key, merge recursively.
3. **Replace for scalars and lists.** A scalar or list replaces the corresponding value wholesale.
4. **The `include` key is consumed** during expansion and does not appear in the merged result.

### Worked example

```yaml
# base/observability.yaml
observability:
  tracing: { enabled: true, strategy: full }
obs:
  redaction: { sensitive_keys: ["password"] }
```

```yaml
# apcore.yaml
include: [./base/observability.yaml]
observability:
  tracing: { strategy: error_first }                 # overrides strategy only
obs:
  redaction: { sensitive_keys: ["password", "token"] }  # list replaced, not appended
```

Merged result:

```yaml
observability:
  tracing: { enabled: true, strategy: error_first }  # enabled from base, strategy local-wins
obs:
  redaction: { sensitive_keys: ["password", "token"] }
```

## Path-typed values in the merged document

`include` entries resolve against their declaring file's directory (above). Path-typed configuration values ([§9.2.1](./protocol-spec.md#921-path-typed-configuration-keys) — `acl.root`, `extensions.root`, `schema.root`, `bindings.dir`) are a different matter: the merged mapping becomes one `Config`, and [§9.2.2](./protocol-spec.md#922-path-resolution-base) gives a `Config` exactly one resolution base. `include:` therefore defines no base of its own. The original draft said such values resolve against the root file's directory, "consistent with D-64 ACL discovery"; that matches §9.2.2 only in part — see open question 1.

## Errors

| Condition | Error code | Notes |
|---|---|---|
| Included file does not exist | `CONFIG_NOT_FOUND` | existing code; `config_path` = the resolved include path |
| `include` value is not a list of strings | `CONFIG_INVALID` | existing code |
| Cyclic include (a file includes itself directly or transitively) | `CONFIG_INCLUDE_CYCLE` | **new code**; the message names the cycle path |

Cycle detection tracks the canonical-path include stack; reaching a path already on the stack raises `CONFIG_INCLUDE_CYCLE`. A diamond — the same file included through two distinct branches — is **not** a cycle; it is merged each time it is reached, which is idempotent for identical content.

## Cross-language sketch

Conceptual expansion algorithm (identical semantics; per-SDK idioms differ):

=== "Python"
    ```python
    def expand_includes(path: str, _stack: tuple[str, ...] = ()) -> dict:
        abspath = os.path.realpath(path)
        if abspath in _stack:
            raise ConfigError(ErrorCodes.CONFIG_INCLUDE_CYCLE,
                              f"include cycle: {' -> '.join((*_stack, abspath))}")
        raw = _load_yaml(abspath)            # CONFIG_NOT_FOUND if missing
        includes = raw.pop("include", [])
        if not isinstance(includes, list) or not all(isinstance(i, str) for i in includes):
            raise ConfigError(ErrorCodes.CONFIG_INVALID, "include must be a list of strings")
        base: dict = {}
        for inc in includes:                 # A < B order
            inc_path = os.path.join(os.path.dirname(abspath), inc)
            base = deep_merge(base, expand_includes(inc_path, (*_stack, abspath)))
        return deep_merge(base, raw)         # local (raw) wins
    ```
=== "TypeScript"
    ```typescript
    function expandIncludes(path: string, stack: string[] = []): Record<string, unknown> {
      const abs = fs.realpathSync(path);
      if (stack.includes(abs)) {
        throw new ConfigError('CONFIG_INCLUDE_CYCLE', `include cycle: ${[...stack, abs].join(' -> ')}`);
      }
      const raw = loadYaml(abs);             // CONFIG_NOT_FOUND if missing
      const includes = (raw.include ?? []) as unknown;
      delete (raw as Record<string, unknown>).include;
      if (!Array.isArray(includes) || !includes.every(i => typeof i === 'string')) {
        throw new ConfigError('CONFIG_INVALID', 'include must be a list of strings');
      }
      let base: Record<string, unknown> = {};
      for (const inc of includes as string[]) {
        const incPath = nodePath.join(nodePath.dirname(abs), inc);
        base = deepMerge(base, expandIncludes(incPath, [...stack, abs]));
      }
      return deepMerge(base, raw as Record<string, unknown>);  // local wins
    }
    ```
=== "Rust"
    ```rust
    fn expand_includes(path: &Path, stack: &mut Vec<PathBuf>) -> Result<Value, ConfigError> {
        let abs = fs::canonicalize(path)?;                  // CONFIG_NOT_FOUND if missing
        if stack.contains(&abs) {
            return Err(ConfigError::new(ErrorCode::ConfigIncludeCycle,
                format!("include cycle: {:?} -> {:?}", stack, abs)));
        }
        let mut raw = load_yaml(&abs)?;
        let includes = take_include_list(&mut raw)?;        // CONFIG_INVALID if not [String]
        stack.push(abs.clone());
        let mut base = Value::Mapping(Default::default());
        for inc in includes {                               // A < B order
            let inc_path = abs.parent().unwrap().join(inc);
            base = deep_merge(base, expand_includes(&inc_path, stack)?);
        }
        stack.pop();
        Ok(deep_merge(base, raw))                           // local wins
    }
    ```

## Conformance plan

A `conformance/fixtures/config_include.json` decision table covering:

- single include merged under local keys (local wins);
- multi-include ordering (`A < B`, later wins);
- deep merge of nested mappings; list replace (not append);
- recursive include (a base that itself includes);
- diamond include (same file through two branches → merged, not a cycle error);
- cycle (self-include and A→B→A) → `CONFIG_INCLUDE_CYCLE`;
- missing include → `CONFIG_NOT_FOUND`;
- non-list `include` → `CONFIG_INVALID`;
- `include` entries resolved against the declaring file's directory.

## Open questions

1. **Base for path-typed values.** The draft's "root file's directory" conflicts with §9.2.2. Through v1.x, `extensions.root`, `schema.root` and `bindings.dir` resolve against the process working directory and only `acl.root` resolves against the configuration file's directory. From v2.0 every path-typed value resolves against the project root, which is the working directory for a user-level configuration file (§9.14 tiers 6–7). Ratifying this RFC should mean adopting §9.2.2's base unchanged, and the conformance plan needs a case pinning it.
2. **`CONFIG_INCLUDE_CYCLE` registration.** Confirm the new code fits the canonical `CONFIG_` error-code set and the reserved-prefix rules (`conformance/fixtures/error_codes.json`).
3. **Depth cap.** Is a maximum include depth needed as a configurable policy limit, or is cycle detection enough? Proposed: cycle detection only.

**Settled:** whether a path value declared *inside* a fragment resolves against that fragment's directory. Spec v1.35.0 (§9.2.2) answered it: one base per `Config` leaves no room for a fragment-relative reading.

## Cross-refs

- [Protocol spec §9 — Configuration](./protocol-spec.md#9-configuration-specification) — the configuration surface this composes over.
- [ACL System — Contract: ACL.discover](../features/acl-system.md#contract-acldiscover) — how `acl.root` is resolved today (D-64).
- [Decision log — D-65](./2026-05-decision-log.md) — the decision record.
