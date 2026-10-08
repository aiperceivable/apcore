---
description: "How apcore keeps secrets out of logs and captured inputs/outputs: x-sensitive schema fields, obs.redaction.* key and value rules, defaults, and where they apply."
---

# Redaction

> **Normative spec:** [protocol-spec §10.6 Sensitive Data Redaction](../spec/protocol-spec.md#106-sensitive-data-redaction) and [§10.6.1 Configured Redaction Rules](../spec/protocol-spec.md#1061-configured-redaction-rules-obsredaction). This page explains how the rules behave and how to configure them.

A value is replaced by the marker `***REDACTED***` when **any** of three independent rules matches it:

| Rule | Matches | Configured by |
|---|---|---|
| `x-sensitive` | a field whose schema marks it `x-sensitive: true` | the module's input/output schema |
| `sensitive_keys` | a field whose **name** matches an entry | `obs.redaction.sensitive_keys` |
| `regex_patterns` | a **string value** that matches a pattern | `obs.redaction.regex_patterns` |

The same rules apply to structured logs and to the inputs and outputs the executor captures for audit — see [Where redaction applies](#where-redaction-applies). Five correlation fields are never redacted (see [Protected fields](#protected-fields)).

## Schema annotation: `x-sensitive`

Mark a field in the module's schema and its value is redacted wherever the executor captures that schema's data. Nested objects are followed, and so are the branches of `anyOf` / `oneOf` / `allOf`: a field is redacted when any branch of its schema marks it, which covers an optional field (`Secret | None`). An array whose `items` are marked has every element redacted. `null` values stay `null`. The algorithm is [A13 `redact_sensitive`](../spec/algorithms.md#a13-redact_sensitive-sensitive-data-redaction).

```json
{
  "type": "object",
  "properties": {
    "username": { "type": "string" },
    "password": { "type": "string", "x-sensitive": true },
    "card": {
      "type": "object",
      "properties": { "number": { "type": "string", "x-sensitive": true } }
    }
  }
}
```

With these inputs, `context.redacted_inputs` holds `{"username": "alice", "password": "***REDACTED***", "card": {"number": "***REDACTED***"}}`. The module itself still receives the real values. See [Schema System](./schema-system.md) for other `x-` annotations.

## Configured rules (`obs.redaction.*`)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `obs.redaction.sensitive_keys` | list of strings | the [default list](#default-sensitive_keys) | Field-name rules |
| `obs.redaction.regex_patterns` | list of strings | `[]` | Regular expressions tested against string values |
| `obs.redaction.replacement` | string | `***REDACTED***` | The marker written in place of a redacted value |

```yaml
# apcore.yaml
apcore:
  version: "1.0.0"

obs:
  redaction:
    sensitive_keys: ["password", "*token*", "api_key", "authorization"]
    regex_patterns:
      - "^Bearer\\s+\\S+$"
      - "^sk-[A-Za-z0-9]{20,}$"
    replacement: "***REDACTED***"
```

The namespace is `obs.redaction`; `observability.redaction.*` is not a declared key.

### Matching `sensitive_keys`

Each entry is interpreted by its own spelling:

| Entry contains | Treated as | Compared with |
|---|---|---|
| `*` or `?` | a glob pattern ([A25](../spec/algorithms.md#a25-match_glob-portable-glob-matching)), anchored to the whole name | the field name, lowercased |
| neither | a substring | the normalized field name |

- Both kinds are **case-insensitive**, and the pattern is folded as well as the name — `*Token*` matches `access_token`.
- Substring comparison normalizes both sides: lowercase, with `-` and whitespace treated as `_`, and camelCase split, so `X-API-Key`, `api-key` and `apiKey` all match the entry `api_key`.
- A glob must match the **entire** name, so `token*` and the bare substring entry `token` behave differently (table below).
- `[`, `]`, `{`, `}` and `\` are literal characters, never a character class. `[!p]assword` has no `*` or `?`, so it is a substring entry that matches only a name containing that literal text.
- An empty entry is ignored.

| Entry | Redacts | Leaves alone |
|---|---|---|
| `token` | `token`, `access_token`, `X-Token-Id` | `tok` |
| `api_key` | `api_key`, `X-API-Key`, `apiKey` | `api` |
| `token*` | `token_id` | `access_token` (anchored) |
| `*secret*` | `client_secret`, `SecretValue` | `secrecy` |
| `_secret_*` | `_secret_token` | `x_secret_token` (anchored) |

### Default `sensitive_keys`

When `obs.redaction.sensitive_keys` is absent or `null`, every SDK uses this list (D-54, pinned by `conformance/fixtures/sensitive_keys_default.json` and `schemas/apcore-config.schema.json`):

```text
_secret_*  password  passwd  secret  token  api_key  apikey  apiKey
access_key  private_key  authorization  auth  credential  cookie  session  bearer
```

- A configured list **replaces** the default; it is not merged. To extend the defaults, write them out alongside your own entries.
- `sensitive_keys: []` turns the name rule off entirely — including `_secret_*`, which is simply the first default entry. `x-sensitive` and `regex_patterns` still apply.
- The SDK constants are `_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS` (Python), `DEFAULT_REDACTION_FIELD_PATTERNS` (TypeScript) and `DEFAULT_SENSITIVE_KEYS` (Rust).

### Matching `regex_patterns`

- Each pattern is searched for anywhere in the value (**unanchored**) and **case-insensitively**; write `^…$` to require a whole-value match.
- Only **string** values are tested. Numbers, booleans, `null`, objects and arrays are never converted to strings to be tested; objects and arrays are walked, so a string inside them is still tested at its own position.
- Keep patterns inside the portable subset of [§9.2.3](../spec/protocol-spec.md#923-pattern-valued-values) — no lookaround, no backreferences, no inline flag groups such as `(?i)` — so they mean the same thing in all three SDKs.
- A pattern that does not compile is reported when the configuration is read (naming the pattern and the engine's error) and redacts nothing. It is never silently dropped.

!!! warning "Numeric secrets need a name rule"
    `^[0-9]{12,19}$` catches `{"pan": "4111111111111111"}` but not `{"pan": 4111111111111111}` — a number is never tested. Match such fields by name (`sensitive_keys: ["pan"]`), which works whatever the value's type, or keep them as strings.

## Protected fields

These correlation fields are never redacted by `sensitive_keys` or `regex_patterns`, at any depth, whatever the rules say:

```text
trace_id   span_id   caller_id   module_id   target_id
```

Without them a log line cannot be correlated with its trace, which is exactly what an incident investigation needs. The exemption covers the field's own value only: an object reached through a protected key is still walked, and secrets inside it are still redacted.

## Where redaction applies

The union of the three rules is applied, with the same configuration, at both surfaces ([§10.6.1](../spec/protocol-spec.md#where-the-rules-apply)):

1. **The executor's capture point.** The executor stores redacted copies on the context for audit and for middleware: `context.redacted_inputs` is set at pipeline step 3 (`module_lookup`) and refreshed at step 7 (`input_validation`), after middleware may have changed the inputs; `context.redacted_output` is set at step 9 (`output_validation`). The rules come from the client's `obs.redaction.*` configuration. Governance events, error histories and any middleware reading `redacted_inputs` see only these copies.
2. **Log emission.** `ContextLogger` redacts the `extra` object of every record, and `ObsLoggingMiddleware` redacts the inputs and outputs it logs, using its `RedactionConfig` ([Observability › Logging](./observability.md#logging)).

For each field the protected-field check comes first, then the name rule, then the value rule for strings; a field that is not redacted and holds an object or array is walked recursively. With no configuration anywhere, the [default list](#default-sensitive_keys) applies — "no configuration" never means "no redaction".

## RedactionConfig API

`RedactionConfig` is the in-code form of the `obs.redaction.*` rules. Build it from a loaded `Config` so logging uses exactly the rules the executor uses.

| Operation | Python | TypeScript | Rust |
|---|---|---|---|
| From configuration | `RedactionConfig.from_config(config)` | `RedactionConfig.fromConfig(config)` | `RedactionConfig::from_config(&config)` |
| Default list | `RedactionConfig.default()` | `RedactionConfig.default()` | `RedactionConfig::defaults()` |
| Programmatic | `RedactionConfig(sensitive_keys=[…], regex_patterns=[…], replacement=…)` | `new RedactionConfig({ fieldPatterns: […], valuePatterns: […], replacement })` | `RedactionConfig::builder().sensitive_keys([…]).value_patterns([…]).replacement(…).try_build()?` |
| Attach to logging | `ObsLoggingMiddleware(redaction_config=…)`, `ContextLogger(redaction_config=…)` | `new ObsLoggingMiddleware({ redactionConfig })`, `new ContextLogger({ redaction })` | `ObsLoggingMiddleware::…with_redaction_config(…)` |

Things to know:

- **An empty constructor has no rules.** Python `RedactionConfig()` and Rust `RedactionConfig::new()` / `RedactionConfig::default()` (the derived `Default`) match nothing; the default list comes from `RedactionConfig.default()` / `RedactionConfig::defaults()`.
- **TypeScript names:** `fieldPatterns` carries the `sensitive_keys` rule and `valuePatterns` the `regex_patterns` rule. String patterns are compiled case-insensitively; a `RegExp` object is used as given, flags included.
- **Rust names:** use the builder's `sensitive_keys`. Its `field_patterns` treats every entry as a glob, and its `value_patterns` compile **case-sensitively** — only `from_config` compiles them case-insensitively. `build()` panics on an invalid pattern; `try_build()` returns `RedactionConfigError`.
- **Python names:** use `sensitive_keys` and `regex_patterns`. The older `field_patterns` / `value_patterns` fields are matched case-sensitively and test stringified non-string values; avoid them.

=== "Python"
    ```python
    from apcore import APCore, Config
    from apcore.observability import ObsLoggingMiddleware, RedactionConfig

    config = Config.load("apcore.yaml")  # the file shown above
    client = APCore(config=config)  # the executor's capture point uses obs.redaction.*

    # Log with exactly the same rules:
    client.use(ObsLoggingMiddleware(redaction_config=RedactionConfig.from_config(config)))

    # Built in code instead:
    custom = RedactionConfig(sensitive_keys=["password", "internal_token"], regex_patterns=[r"^Bearer\s+\S+$"])
    defaults = RedactionConfig.default()
    assert len(defaults.sensitive_keys) == 16
    ```

=== "TypeScript"
    ```typescript
    import { APCore, Config, ObsLoggingMiddleware, RedactionConfig } from "apcore-js";

    const config = Config.load("apcore.yaml"); // the file shown above
    const client = new APCore({ config }); // the executor's capture point uses obs.redaction.*

    // Log with exactly the same rules:
    client.use(new ObsLoggingMiddleware({ redactionConfig: RedactionConfig.fromConfig(config) }));

    // Built in code instead:
    const custom = new RedactionConfig({
      fieldPatterns: ["password", "internal_token"],
      valuePatterns: ["^Bearer\\s+\\S+$"],
    });
    const redacted = custom.redact({ password: "hunter2", note: "Bearer abc", trace_id: "t1" });
    // { password: "***REDACTED***", note: "***REDACTED***", trace_id: "t1" }
    ```

=== "Rust"
    ```rust
    use std::path::Path;

    use apcore::config::Config;
    use apcore::observability::{ContextLogger, ObsLoggingMiddleware, RedactionConfig};
    use apcore::APCore;
    use serde_json::json;

    fn main() -> Result<(), Box<dyn std::error::Error>> {
        let config = Config::load(Path::new("apcore.yaml"))?; // the file shown above
        let rules = RedactionConfig::from_config(&config);
        let client = APCore::with_config(config); // the executor's capture point uses obs.redaction.*

        // Log with exactly the same rules:
        client.use_middleware(Box::new(
            ObsLoggingMiddleware::new(ContextLogger::new("apcore.obs_logging")).with_redaction_config(rules),
        ))?;

        // Built in code instead:
        let custom = RedactionConfig::builder()
            .sensitive_keys(["password", "internal_token"])
            .value_patterns(["^Bearer\\s+\\S+$"])
            .try_build()?;
        let mut value = json!({ "password": "hunter2", "note": "Bearer abc", "trace_id": "t1" });
        custom.redact(&mut value);
        assert_eq!(value["password"], "***REDACTED***");
        assert_eq!(value["note"], "***REDACTED***");
        assert_eq!(value["trace_id"], "t1");
        Ok(())
    }
    ```

## See also

- [Observability](./observability.md) — `ContextLogger` and `ObsLoggingMiddleware`
- [Core Executor](./core-executor.md) — the pipeline steps that capture `redacted_inputs` / `redacted_output`
- [Context Object](./context-object.md) — `redacted_inputs`, `redacted_output`
- [Error History](./error-history.md) — records built from redacted data
