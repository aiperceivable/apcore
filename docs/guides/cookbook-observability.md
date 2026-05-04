# Cookbook — Observability with OTel + Sensitive Data Redaction

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC §10](../../PROTOCOL_SPEC.md#10-observability-specification). Feature reference: [features/observability.md](../features/observability.md).

End-to-end example: emit OpenTelemetry traces and structured logs from your apcore modules **with PII automatically redacted** via `x-sensitive` schema annotations and `obs.redaction.sensitive_keys`.

## When to use this pattern

- You ship apcore modules to production and want every call traceable through OTel-compatible backends (Tempo, Honeycomb, Datadog, Jaeger).
- Your inputs/outputs include PII (emails, tokens, payment data) that must not appear in logs or spans.
- You're building an audit trail for a compliance review.

## When NOT to use this pattern

- For ad-hoc local debugging: use `LoggingMiddleware` with `log_inputs=True`. The redaction settings still apply, but you don't need OTel.
- For high-cardinality custom metrics: this cookbook covers traces and logs. Metrics are framework-internal; use the `UsageExporter` interface ([fixture: `usage_exporter`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/usage_exporter.json)).

---

## 1. Define a schema with `x-sensitive`

`x-sensitive: true` on a property tells the framework's redactor to mask its value before logging or spanning. The contract is verified by the [`redaction_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/redaction_config.json) fixture.

=== "Python"
    ```python
    from apcore import APCore
    client = APCore()

    @client.module(
        id="user.create",
        description="Create a user account",
        input_schema={
            "type": "object",
            "properties": {
                "email":    {"type": "string", "format": "email"},
                "password": {"type": "string", "x-sensitive": True},
                "name":     {"type": "string"},
                "api_key":  {"type": "string", "x-sensitive": True},
            },
            "required": ["email", "password"],
        },
        output_schema={"type": "object", "properties": {"user_id": {"type": "string"}}},
    )
    def create_user(email: str, password: str, name: str | None, api_key: str | None) -> dict:
        return {"user_id": db.insert(email, hash(password), name, api_key)}
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'user.create',
      description: 'Create a user account',
      inputSchema: Type.Object({
        email:    Type.String({ format: 'email' }),
        password: Type.String({ 'x-sensitive': true }),
        name:     Type.Optional(Type.String()),
        api_key:  Type.Optional(Type.String({ 'x-sensitive': true })),
      }),
      outputSchema: Type.Object({ user_id: Type.String() }),
      execute: async (inputs) => ({ user_id: await db.insert(inputs as any) }),
    });
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, Context};
    use serde_json::{json, Value};

    let mut client = APCore::new();
    client.module(
        "user.create",
        "Create a user account",
        json!({
            "type":"object",
            "properties":{
                "email":    {"type":"string","format":"email"},
                "password": {"type":"string","x-sensitive": true},
                "name":     {"type":"string"},
                "api_key":  {"type":"string","x-sensitive": true}
            },
            "required":["email","password"]
        }),
        json!({"type":"object","properties":{"user_id":{"type":"string"}}}),
        None, vec![], None, None, vec![], None,
        |inputs: Value, _ctx: &Context<Value>| {
            Box::pin(async move {
                Ok(json!({"user_id": db::insert(inputs).await?}))
            })
        },
    )?;
    ```

## 2. Configure redaction

`obs.redaction.sensitive_keys` ships with a canonical default list (D-54: `password`, `secret`, `api_key`, `authorization`, `token`, `private_key`, `cookie`, …) shared identically across all 3 SDKs (fixture: `sensitive_keys_default`). You usually **extend** the default rather than replace it.

```yaml
# apcore.yaml — extract
obs:
  redaction:
    # extend the canonical default list
    sensitive_keys:
      - "ssn"
      - "credit_card_number"
    # add regex patterns for free-form values not behind a known key
    regex_patterns:
      - '(?<![0-9])\d{3}-\d{2}-\d{4}(?![0-9])'        # SSN-like
      - '(?<![A-Z0-9])sk-[A-Za-z0-9]{32,}(?![A-Z0-9])' # OpenAI-key-like
```

Two redaction layers apply:

| Layer | What it scrubs | When |
|-------|---------------|------|
| `x-sensitive` schema annotations | Any property marked `x-sensitive: true` | Inputs **and** outputs, before logging / OTel attribute write |
| `obs.redaction.sensitive_keys` | Any **key** anywhere in inputs/outputs/context.data matching the list | Same |
| `obs.redaction.regex_patterns` | Any **value** matching the regex (free-form text scan) | Same; runs after the key-based pass |

Anything not matched by these layers passes through. Critically, **error message strings are NOT scrubbed by default** — see [troubleshooting §1.8](./troubleshooting.md#18-x-sensitive-true-field-appears-in-plain-text-in-my-error-message).

## 3. Wire OpenTelemetry

```yaml
# apcore.yaml — extract
obs:
  otel:
    enabled: true
    service_name: my-service
    exporter: otlp           # or "console" for local debugging
    endpoint: http://otel-collector:4317
    sample_rate: 1.0         # tune in production
```

Per-call spans the executor emits automatically:

| Span name | Attributes attached |
|-----------|--------------------|
| `module.call`           | `apcore.module_id`, `apcore.caller_id`, `apcore.trace_id`, `apcore.identity.id`, `apcore.duration_ms` |
| `module.stream.chunk`   | per chunk during streaming |
| `pipeline.step.<name>`  | each pipeline step (when step-level tracing is enabled) |
| `acl.evaluate`          | the matching rule (or `<default>`) |
| `approval.gate`         | `apcore.approval.status`, `apcore.approval.approved_by` |

Span attribute values pass through the redaction pipeline before being written, so `x-sensitive` properties show as `<redacted>` rather than the raw value.

## 4. Caller code is unchanged

```python
result = client.call("user.create", {
    "email": "alice@example.com",
    "password": "hunter2",      # x-sensitive — won't appear in spans/logs
    "name": "Alice",
    "api_key": "sk-abc123",      # x-sensitive AND in canonical sensitive_keys list
})
```

Trace output (Jaeger UI excerpt):

```
module.call
├─ apcore.module_id     = "user.create"
├─ apcore.caller_id     = "@external"
├─ apcore.duration_ms   = 14
├─ apcore.input.email   = "alice@example.com"
├─ apcore.input.password = "<redacted>"
├─ apcore.input.name    = "Alice"
├─ apcore.input.api_key = "<redacted>"
└─ apcore.output.user_id = "u_42"
```

## 5. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| Setting `sensitive_keys` overwrites the canonical default | Standard PII keys (`password`, `token`) leak | Always **extend**, never replace; or use `sensitive_keys_extra:` if your SDK supports it |
| `x-sensitive` on a nested object only marks the parent key | Nested fields leak | Mark each sensitive leaf property individually |
| Custom middleware writes raw inputs into spans | Redaction bypassed | Use `context.redacted_inputs` (populated by the framework) instead of the original `inputs` dict |
| Regex patterns are too greedy | Performance hit on every log line | Anchor regexes with negative look-behinds (e.g. lookbehinds for non-digit boundaries) and prefer `sensitive_keys` whenever the data is keyed |
| Production left at `sample_rate: 1.0` | OTel backend overload | Drop to 0.01–0.1 in steady state; sample at 1.0 only for incident windows |
| Trace ID propagation broken across HTTP boundary | Calls appear as separate root spans | Adapter (FastAPI/Express/Axum) must extract `traceparent` from headers and pass into `Context` — see [PROTOCOL_SPEC §10.5](../../PROTOCOL_SPEC.md#10-observability-specification) |

## 6. Verifying redaction in tests

```python
def test_password_redacted_in_logs(caplog):
    client.call("user.create", {"email":"a@b.c","password":"hunter2","name":"A"})
    assert "hunter2" not in caplog.text
    assert "<redacted>" in caplog.text  # or check the logging middleware's redacted record
```

The conformance fixtures cover this cross-SDK:

- [`redaction_config`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/redaction_config.json) — 4 cases for the YAML config surface
- [`sensitive_keys_default`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/sensitive_keys_default.json) — 4 cases asserting the canonical default list ships identically in all 3 SDKs

---

## See also

- [features/observability.md](../features/observability.md) — full feature reference (1814 lines, deep dive)
- [spec/security-considerations.md §2.5](../spec/security-considerations.md#25-sensitive-data-in-logs-t6) — threat model
- [PROTOCOL_SPEC §10](../../PROTOCOL_SPEC.md#10-observability-specification) — normative observability spec
- [troubleshooting §1.8](./troubleshooting.md#18-x-sensitive-true-field-appears-in-plain-text-in-my-error-message) — error-message redaction caveat
