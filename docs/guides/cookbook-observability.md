---
description: "Cookbook: trace apcore calls from configuration and write structured logs with secrets redacted by x-sensitive schema markers and obs.redaction rules."
---

# Cookbook — Tracing and Redacted Logs

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC §10](../spec/protocol-spec.md#10-observability-specification). Feature reference: [Observability](../features/observability.md), [Redaction](../features/redaction.md).

End-to-end recipe: turn on tracing from `apcore.yaml`, log every module call as a structured record, and keep secrets out of those records with `x-sensitive` schema markers plus the `obs.redaction.*` rules.

## When to use this pattern

- You run apcore modules in production and want every call traceable in an OTLP backend (Tempo, Honeycomb, Datadog, Jaeger's OTLP endpoint).
- Your inputs include credentials or personal data that must not reach logs.

## When NOT to use this pattern

- Metrics (call counts, latency histograms) use `MetricsCollector` + `MetricsMiddleware`, with `PrometheusExporter` to serve them — see [Metrics and Usage](../features/metrics-and-usage.md). The `observability.metrics.*` config keys do nothing; wire metrics in code.
- Per-module usage statistics come from `UsageCollector` and the `system.usage.*` modules ([Metrics and Usage § Usage collector](../features/metrics-and-usage.md#usage-collector)).

---

## 1. Configure redaction and tracing

```yaml
# apcore.yaml
version: "1.0.0"
project:
  name: user-service

obs:
  redaction:
    # Setting this key REPLACES the built-in list — it does not add to it.
    # Keep all 16 default entries, then append your own.
    sensitive_keys:
      - "_secret_*"
      - password
      - passwd
      - secret
      - token
      - api_key
      - apikey
      - apiKey
      - access_key
      - private_key
      - authorization
      - auth
      - credential
      - cookie
      - session
      - bearer
      # additions for this service
      - ssn
      - card_number
    # Matched against string VALUES anywhere in a record (unanchored search).
    # A match replaces the whole value.
    regex_patterns:
      - '[0-9]{3}-[0-9]{2}-[0-9]{4}'    # US SSN shape
      - 'sk-[A-Za-z0-9]{32,}'           # provider API-key shape

observability:
  tracing:
    enabled: true
    exporter: otlp                                  # or stdout while developing
    otlp_endpoint: http://otel-collector:4318/v1/traces
    strategy: error_first                           # every error, plus a sample of successes
    sampling_rate: 0.1
```

What the redaction settings mean:

- **`sensitive_keys` replaces the default list.** The 16 entries above are the SDK default (identical in all three SDKs). If you list only `ssn`, then `password`, `token` and the rest stop being redacted. Leave the key out entirely to keep the defaults. The default list and the matching rules are in [Redaction § Default `sensitive_keys`](../features/redaction.md#default-sensitive_keys).
- Entries without `*` or `?` match as case-insensitive substrings of the field name (`token` also catches `access_token`); entries with `*` or `?` are globs matched against the whole name.
- **Keep `regex_patterns` portable:** no lookahead/lookbehind, no backreferences, no inline flags such as `(?i)` (matching is already case-insensitive). The Rust `regex` crate rejects lookaround, and an entry that fails to compile is reported when the config loads and redacts nothing.
- The replacement marker is `***REDACTED***` (change it with `obs.redaction.replacement`).

What the tracing settings mean:

- `enabled: true` makes `APCore(config=…)` install a `TracingMiddleware`; without it nothing is installed.
- `exporter` is `stdout` or `otlp`. The OTLP exporter needs extra packages in Python (`opentelemetry-sdk`, `opentelemetry-exporter-otlp-proto-http`) and the `events` feature in Rust; without them the SDK logs a warning and installs no tracing middleware. `otlp_endpoint` is only valid with `exporter: otlp`; when omitted the exporter sends to `http://localhost:4318/v1/traces`.
- `sampling_rate` is only consulted by the `proportional` and `error_first` strategies. With the default `strategy: full` every call is recorded whatever the rate says.

Full key reference: [PROTOCOL_SPEC §10.1.1](../spec/protocol-spec.md#1011-tracing-from-configuration-observabilitytracing) and [§10.7](../spec/protocol-spec.md#107-sampling-strategy).

## 2. Mark sensitive fields and wire the logger

`x-sensitive: true` on a schema property redacts that field whatever its name. Tracing is installed from the config; the structured call log is `ObsLoggingMiddleware`, which you add yourself. Give it a `RedactionConfig` built from the same config so the log record follows your `obs.redaction.*` rules.

=== "Python"
    ```python
    from typing import Annotated

    from pydantic import Field

    from apcore import APCore, Config
    from apcore.observability import ContextLogger, ObsLoggingMiddleware, RedactionConfig

    config = Config.load("apcore.yaml")
    client = APCore(config=config)  # installs TracingMiddleware from observability.tracing.*
    client.use(
        ObsLoggingMiddleware(
            logger=ContextLogger(name="user-service"),
            redaction_config=RedactionConfig.from_config(config),
        )
    )

    # x-sensitive travels in the inferred schema via Field(json_schema_extra=...).
    Secret = Annotated[str, Field(json_schema_extra={"x-sensitive": True})]


    @client.module(id="user.create", description="Create a user account")
    def create_user(email: str, password: Secret, pin: Secret, note: str = "") -> dict:
        return {"user_id": "u_42"}


    client.call(
        "user.create",
        {"email": "alice@example.com", "password": "hunter2", "pin": "4711", "note": "ssn 123-45-6789"},
    )
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Config, ContextLogger, ObsLoggingMiddleware, RedactionConfig } from 'apcore-js';

    const config = Config.load('apcore.yaml');
    const client = new APCore({ config }); // installs TracingMiddleware from observability.tracing.*
    client.use(
      new ObsLoggingMiddleware({
        logger: new ContextLogger({ name: 'user-service' }),
        redactionConfig: RedactionConfig.fromConfig(config),
      }),
    );

    client.module({
      id: 'user.create',
      description: 'Create a user account',
      inputSchema: Type.Object({
        email: Type.String(),
        password: Type.String({ 'x-sensitive': true }),
        pin: Type.String({ 'x-sensitive': true }),
        note: Type.Optional(Type.String()),
      }),
      outputSchema: Type.Object({ user_id: Type.String() }),
      execute: async () => ({ user_id: 'u_42' }),
    });

    await client.call('user.create', {
      email: 'alice@example.com',
      password: 'hunter2',
      pin: '4711',
      note: 'ssn 123-45-6789',
    });
    ```

=== "Rust"
    ```rust
    use apcore::{
        APCore, Config, Context, ContextLogger, ModuleError, ObsLoggingMiddleware, RedactionConfig,
    };
    use serde_json::{json, Value};
    use std::path::Path;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let config = Config::load(Path::new("apcore.yaml"))?;
        // Installs TracingMiddleware from observability.tracing.*
        let mut client = APCore::with_config(config.clone());
        client.use_middleware(Box::new(
            ObsLoggingMiddleware::new(ContextLogger::new("user-service"))
                .with_redaction_config(RedactionConfig::from_config(&config)),
        ))?;

        client.module(
            "user.create",
            "Create a user account",
            json!({
                "type": "object",
                "properties": {
                    "email":    {"type": "string"},
                    "password": {"type": "string", "x-sensitive": true},
                    "pin":      {"type": "string", "x-sensitive": true},
                    "note":     {"type": "string"}
                },
                "required": ["email", "password", "pin"]
            }),
            json!({"type": "object", "properties": {"user_id": {"type": "string"}}}),
            None,   // documentation
            vec![], // tags
            None,   // version
            None,   // metadata
            vec![], // examples
            None,   // display
            |_inputs: Value, _ctx: &Context<Value>| {
                Box::pin(async move { Ok(json!({"user_id": "u_42"})) })
            },
        )?;

        client
            .call(
                "user.create",
                json!({
                    "email": "alice@example.com",
                    "password": "hunter2",
                    "pin": "4711",
                    "note": "ssn 123-45-6789"
                }),
                None,
                None,
            )
            .await?;
        Ok(())
    }
    ```

## 3. What the log shows

The `inputs` of the start record from the Python run above:

```text
{"email": "alice@example.com", "password": "***REDACTED***", "pin": "***REDACTED***", "note": "***REDACTED***"}
```

- `password` — matched by `sensitive_keys` and by `x-sensitive`.
- `pin` — `x-sensitive` only.
- `note` — its value contains an SSN-shaped string, so `regex_patterns` replaces the whole value.

Tracing records one span per module call, named `apcore.module.execute`, with the attributes `module_id`, `method`, `caller_id`, `duration_ms`, `success`, and `error_code` on failure. Spans carry no inputs or outputs, so redaction does not apply to them. Nested calls produce child spans under the same `trace_id`.

## 4. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| `sensitive_keys` lists only your additions | `password`, `token`, `authorization` appear in plain text | Copy the 16 defaults into the list (section 1), or leave the key unset |
| Lookaround or `(?i)` in `regex_patterns` | Config load reports the pattern (Rust), or the rule behaves differently per SDK | Stay within the portable subset; matching is already case-insensitive |
| Custom middleware logs the `inputs` argument | Secrets bypass redaction | Log `context.redacted_inputs` (`redactedInputs` in TypeScript) — it has `x-sensitive` and the `obs.redaction.*` rules applied |
| Sensitive value inside an error message | Secret appears in the error returned to the caller and in error logs | Redaction works on fields, not on message text — keep secrets out of messages ([troubleshooting §1.8](./troubleshooting.md#18-x-sensitive-true-field-appears-in-plain-text-in-my-error-message)) |
| `sampling_rate: 0.1` with the default `strategy: full` | Every call is still traced | Set `strategy: proportional` or `error_first` |
| `exporter: otlp` without the OTLP dependencies | Warning at startup, no spans exported | Install the OpenTelemetry packages (Python) or enable the `events` feature (Rust) |
| Calls show up as separate root traces | The inbound `traceparent` header was never read | Build the `Context` from the request headers — see [Integrating Existing Projects](./integrating-existing-projects.md) |

## 5. Testing redaction

Point the logger at a buffer and assert on what it wrote:

```python
import io
from typing import Annotated

from pydantic import Field

from apcore import APCore
from apcore.observability import ContextLogger, ObsLoggingMiddleware

Secret = Annotated[str, Field(json_schema_extra={"x-sensitive": True})]


def test_password_is_redacted_in_call_log() -> None:
    buffer = io.StringIO()
    client = APCore()
    client.use(ObsLoggingMiddleware(logger=ContextLogger(name="test", output=buffer)))

    @client.module(id="user.create", description="Create a user account")
    def create_user(email: str, password: Secret) -> dict:
        return {"user_id": "u_42"}

    client.call("user.create", {"email": "a@example.com", "password": "hunter2"})

    assert "hunter2" not in buffer.getvalue()
    assert "***REDACTED***" in buffer.getvalue()
```

---

## See also

- [features/observability.md](../features/observability.md) — tracing and logging reference
- [features/redaction.md](../features/redaction.md) — `x-sensitive`, `obs.redaction.*` and where redaction applies
- [PROTOCOL_SPEC §10.6](../spec/protocol-spec.md#106-sensitive-data-redaction) — redaction rules
- [spec/security-considerations.md §2.5](../spec/security-considerations.md#25-sensitive-data-in-logs-t6) — threat model for secrets in logs
