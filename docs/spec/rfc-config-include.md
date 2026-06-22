---
description: "Proposed RFC adding a top-level include: key for cross-file apcore.yaml composition — relative-path resolution, deep-merge with local-wins precedence, recursive includes, and cycle detection. Design-first; no implementation yet (issue #75, decision D-65)."
---

# RFC — `include:` Cross-File Configuration Composition

## Status

**Proposed** (2026-06-22) — awaiting maintainer decision. Tracked by [#75](https://github.com/aiperceivable/apcore/issues/75) and decision **D-65** in the [decision log](./2026-05-decision-log.md). No SDK implementation exists yet; this document defines the contract to be ratified before any code lands. Independent of, and unblocked by, the `acl.root` activation in [D-64](./2026-05-decision-log.md) (#74).

## Motivation

A project's `apcore.yaml` grows monolithic: ACL policy, pipeline config, observability, and per-environment overrides all live in one file. Teams want to split shared configuration into reusable fragments — a base file plus environment overlays, or a shared ACL/observability block included by several services. Today the only option is to duplicate YAML or assemble it with an external templating step before apcore ever sees it.

`include:` lets one `apcore.yaml` pull in other YAML fragments and merge them, declaratively, inside the SDK — with deterministic, identical semantics across Python, TypeScript, and Rust.

## Non-goals

YAGNI — explicitly out of scope for v1 (each would need its own RFC if ever justified):

- **Glob / wildcard includes** (`include: ["conf.d/*.yaml"]`). Explicit file lists only.
- **Remote includes** (http(s) URLs, package refs). Local filesystem paths only.
- **Conditional / environment-gated includes**. Use `include:` for structure; keep env overrides in the existing env-var override mechanism.
- **List-element merging** (append/dedup). Lists are replaced wholesale (see §Merge semantics).
- **Reserved namespaces / spec-version fields on the include block.** No forward-compat scaffolding; the key is a plain list of strings.

## Proposed syntax

A top-level `include:` key whose value is a list of file-path strings:

```yaml
# apcore.yaml
include:
  - ./base/observability.yaml
  - ./base/acl.yaml
  - ./env/production.yaml      # later entries override earlier ones

# Local keys below override anything pulled in by include:
acl:
  default_effect: deny
```

- `include` MUST be a list of strings. A scalar, mapping, or non-string element is a config error.
- Paths are resolved **relative to the directory of the file that declares the `include`** (not the root file, not CWD).
- An included file MAY itself declare `include:` — processed recursively, depth-first.

## Merge semantics

Resolution is a pre-processing phase that runs **before** validation, env-var overrides, and binding — it produces a single merged mapping that the rest of the pipeline consumes unchanged.

1. **Order.** For a file with `include: [A, B]` and its own local keys L, the effective precedence is `A < B < L` — i.e. process includes in listed order, then overlay the declaring file's own keys last. **Local keys always win; later includes win over earlier ones.** Rationale: `include:` reads as "pull in these bases, then my file tweaks them," matching developer intuition (cf. CSS `@import`, shell `source`-then-override).
2. **Deep merge for mappings.** When both sides hold a mapping at the same key, merge recursively.
3. **Replace for scalars and lists.** A scalar or list value replaces the corresponding value wholesale — no element-level list merging (YAGNI; predictable).
4. **The `include` key itself is consumed** during expansion and does not appear in the merged result.

### Worked example

```yaml
# base/acl.yaml
acl: { root: ./acl, default_effect: deny }
obs: { redaction: { sensitive_keys: ["password"] } }
```
```yaml
# apcore.yaml
include: [./base/acl.yaml]
acl: { default_effect: allow }          # overrides base's default_effect only
obs: { redaction: { sensitive_keys: ["password", "token"] } }   # list replaced
```
Merged result:
```yaml
acl: { root: ./acl, default_effect: allow }     # root from base, default_effect local-wins
obs: { redaction: { sensitive_keys: ["password", "token"] } }
```

## Relative-path resolution of merged values

After expansion, the merged config's source path is the **root** (including) file. Path-valued config keys (`acl.root`, extension/module roots, etc.) resolve relative to the **root file's directory**, consistent with [D-64 ACL discovery](../features/acl-system.md#contract-acldiscover). A path declared inside an included fragment that needs to point near *that fragment* is an **open question** (§Open questions) — v1 resolves all such paths root-relative for simplicity and to keep one resolution base.

## Errors

| Condition | Error code | Notes |
|---|---|---|
| Included file does not exist | `CONFIG_NOT_FOUND` | reuse existing code; `config_path` = the resolved include path |
| `include` value is not a list of strings | `CONFIG_INVALID` | reuse existing code |
| Cyclic include (a file includes itself directly or transitively) | `CONFIG_INCLUDE_CYCLE` | **new code**; message names the cycle path |

Cycle detection tracks the absolute-path include stack; revisiting a path already on the stack raises `CONFIG_INCLUDE_CYCLE`. (A diamond — the same file included via two distinct branches — is **not** a cycle; it is merged each time it is reached, which is idempotent for identical content.)

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

- single include merged under local keys (local-wins);
- multi-include ordering (`A < B`, later wins);
- deep-merge of nested mappings; list-replace (not append);
- recursive include (a base that itself includes);
- diamond include (same file via two branches → merged, not a cycle error);
- cycle (self-include and A→B→A) → `CONFIG_INCLUDE_CYCLE`;
- missing include → `CONFIG_NOT_FOUND`;
- non-list `include` → `CONFIG_INVALID`;
- relative path resolution against the declaring file's directory.

## Open questions

1. **Fragment-relative path values.** Should a path value (e.g. `acl.root`) declared *inside* an included fragment resolve relative to that fragment's directory rather than the root file? v1 proposes root-relative (one base, simplest); revisit if real use demands fragment-relative.
2. **`CONFIG_INCLUDE_CYCLE` registration.** Confirm the new code fits the canonical config error-code set and the reserved-prefix rules (cf. `error_codes.json`).
3. **Depth cap.** Do we need a max include depth as a configurable policy limit (cf. DECLARATIVE_CONFIG_SPEC §2.5) to bound pathological nesting, or is cycle detection sufficient? Proposed: cycle detection only for v1.

## Cross-refs

- [Declarative Config Spec](./DECLARATIVE_CONFIG_SPEC.md) — config file formats this composes over.
- [ACL System §Contract: ACL.discover](../features/acl-system.md#contract-acldiscover) — relative-path resolution precedent (D-64).
- [Decision Log — D-65](./2026-05-decision-log.md) — the ratification record.
