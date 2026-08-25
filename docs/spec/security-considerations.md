---
description: "RFC 3552-style security considerations for apcore: threat model, in-scope mitigations (ACL, approval, call-chain guard, input validation), residual risks, and production audit guidance."
---

# Security Considerations

> **Type:** Informative specification (RFC 3552 §4 style). **Normative cross-references:** [PROTOCOL_SPEC](./protocol-spec.md) §6 ACL, §7 Approval, §8 Errors, §10 Observability.

This document enumerates the threats apcore is designed to mitigate, the threats it does not address (so callers know where defense-in-depth is required), and the audit recommendations for production deployments. It is organised in the IETF [RFC 3552](https://www.rfc-editor.org/rfc/rfc3552) "Guidelines for Writing RFC Text on Security Considerations" pattern: threat model first, mitigations second, residual risks and operational guidance last.

apcore is a **module standard**, not a sandbox. The host process trusts every loaded module's `execute()` body to behave well. This document highlights the boundary between "apcore enforces this" and "your host must enforce this."

---

## 1. Threat Model

### 1.1 In-scope threats (apcore mitigates)

| ID  | Threat                                  | Where mitigated                            |
|-----|-----------------------------------------|--------------------------------------------|
| T1  | Unauthorised inter-module invocation    | ACL Step 4 (default-deny)                  |
| T2  | Sensitive operation without sign-off    | Approval Gate Step 5                       |
| T3  | Runaway / stack-blowing recursion       | Call Chain Guard Step 2                    |
| T4  | Malformed inputs reaching `execute()`   | Input Validation Step 7 (JSON Schema)      |
| T5  | Output schema drift (data exfiltration) | Output Validation Step 9                   |
| T6  | Sensitive data in logs                  | `x-sensitive` redaction + `obs.redaction.sensitive_keys` |
| T7  | Trace ID forgery / log poisoning        | Trace ID validation rule (§10.5)           |
| T8  | Module-ID spoofing via path traversal   | Canonical ID derivation (Algorithm A01)    |
| T9  | Annotation tampering at the wire        | Schema-validated `ModuleAnnotations.extra` round-trip (fixture: `annotations_extra_round_trip`) |
| T10 | Concurrent reload corrupting in-flight calls | `MODULE_RELOAD_CONFLICT` retry path     |

### 1.2 Out-of-scope threats (host responsibility)

| ID   | Threat                                  | Required compensating control          |
|------|-----------------------------------------|----------------------------------------|
| OT1  | Untrusted module code execution         | Run untrusted modules in a separate process / container / WASM sandbox; apcore does not isolate Python globals or memory |
| OT2  | Network egress / SSRF from a module     | Egress firewall, library-level allowlists; apcore does not intercept outbound HTTP |
| OT3  | Filesystem access outside `extensions/` | OS-level sandboxing (chroot, AppArmor, seccomp); apcore reads any path the process can read |
| OT4  | Denial of service via resource exhaustion (CPU, memory) | OS / cgroup limits; apcore enforces a per-call timeout but no memory cap |
| OT5  | Side-channel data leakage (timing, error messages) | See §3.4 below |
| OT6  | Supply-chain compromise of an SDK or apcore itself | Signature verification, pinned dependencies, SBOM review |
| OT7  | Adversarial inputs designed to amplify schema validation cost | Set `policy.max_schema_depth`, `policy.max_inputs_size`; consider input rate limiting |
| OT8  | Authenticating the calling identity itself | Apcore consumes `Identity`; populating it from a verified principal is the host's job |

---

## 2. Detailed Threats and Mitigations

### 2.1 Module-ID Spoofing (T8)

**Threat.** A module file placed at a crafted path could attempt to claim a Canonical ID belonging to a privileged module (e.g., `system.control.reload_module`).

**Mitigation.**

1. **Reserved first segments are enforced.** Reserved-word rejection is **not** part of Algorithm A01 (`directory_to_canonical_id`, [§2.1](./protocol-spec.md#21-directory-as-id-core-rule)) — A01 validates segment pattern and length only. It is [§2.6 `detect_id_conflicts`](./protocol-spec.md#26-id-conflict-detection) step 2, which rejects an ID any of whose dot-separated segments is a reserved word, and the equivalent check in each SDK's public `register()`. The framework reserved words are the eight in [§2.5](./protocol-spec.md#25-reserved-words): `system`, `internal`, `core`, `apcore`, `plugin`, `schema`, `acl`, `ephemeral`. `system.*` is therefore registrable only through the privileged `register_internal()` path ([§6.6.1](./protocol-spec.md#661-registration-restriction)).

   > **`sys` is not reserved.** `sys.control.reload_module` is an ordinary module ID that any user module may claim, and it names no privileged module — the control-plane namespace is `system.*`. An ACL rule or audit check written against `sys.*` matches nothing and silently protects nothing.

   **Coverage gap.** No conformance fixture exercises reserved-word rejection today; `normalize_id` covers Algorithm A02 (ID normalization) and carries no reserved-word case. Treat this mitigation as implementation-verified, not fixture-verified.
2. **Conflict detection.** If two modules resolve to the same Canonical ID, the second is rejected with `MODULE_ID_CONFLICT` rather than silently overriding (fixture: `multi_module_discovery`).
3. **Path traversal is impossible.** Canonical IDs derive from the path *relative to* the configured `extension_dir`. Symlinks pointing outside the dir are followed at filesystem level — operators **MUST** ensure the extension directory does not contain attacker-controlled symlinks.

**Residual risk.** A privileged user who can write to `extension_dir` can register modules under any non-reserved ID. Treat write access to `extension_dir` as a privileged capability.

### 2.2 ACL Bypass (T1)

**Threat.** Crafting an invocation that evades intended access rules.

**Known evasion patterns and mitigations:**

- **Empty `caller_id`.** Treated as `@external` and matched accordingly. No fall-through to "no caller, no rule, allow" unless `default_effect: allow` is set.
- **Pattern injection.** Patterns are not regex; the `*` wildcard is the only special character. Untrusted input cannot inject regex metachars.
- **Compound operator abuse.** `["$not", p1, p2, ...]` consults only `patterns[1]`; extras are ignored ([features/acl-system.md](../features/acl-system.md)). Authors **SHOULD** treat additional patterns as undefined behaviour and never rely on them as a hidden allow path.
- **Missing context with conditional rules.** When `conditions` are present but no Context is provided, the rule does **not** match — it neither allows nor denies via that rule. Evaluation continues to the next rule (and eventually to `default_effect`). This is a denial vector, not a bypass.

**`default_effect: deny` is the only safe production setting.** `allow` is provided for narrow opt-in scenarios and **MUST** be accompanied by explicit `deny` rules for sensitive targets (see the warning in [features/acl-system.md](../features/acl-system.md)).

### 2.3 Approval Gate Replay (T2)

**Threat.** A `_approval_token` reused after the underlying decision should have expired, or used by a different caller than the original requester.

**Mitigation.** The protocol does not specify token format — that is delegated to `ApprovalHandler` implementations. Handlers **SHOULD**:

1. Make tokens **single-use** (consume on first `check_approval` success).
2. **Bind tokens to `(caller_id, target_id, input_hash)`** so they cannot be replayed against a different invocation.
3. Set **expiry timestamps** and reject expired tokens with `status: timeout`.
4. Record approver identity (`approved_by`) for audit.

**Residual risk.** A handler that issues unbound, long-lived tokens is vulnerable to replay. This is an implementation defect, not a protocol flaw.

### 2.4 `context.data` Injection (T1, OT5)

**Threat.** A module writing untrusted data to `context.data` that downstream middleware or modules trust as authoritative.

**Mitigation.**

1. `context.data` is documented as a **free-form scratchpad** with **namespaced keys** (`_apcore.<feature>.<key>` for framework-internal use). Modules **SHOULD** namespace their own writes (`<module-id>.<key>`) and **MUST NOT** read another module's keys.
2. Sensitive identity data is carried in `context.identity`, **not** `context.data`. The `Identity` sub-schema is structurally fixed (`id`, `type`, `roles`, `attrs`) and SHOULD be populated only at trust-boundary entry points.

**Residual risk.** A module that writes a key like `_apcore.identity.roles=["admin"]` to `context.data` (squatting on a framework-style key) cannot escalate apcore-level privileges (ACL reads `context.identity`, not `context.data`), but **could** mislead a custom middleware that mistakenly reads from `context.data`. Custom middleware **MUST NOT** consult `context.data` for authorization decisions.

### 2.5 Sensitive Data in Logs (T6)

**Threat.** PII, credentials, or session tokens leaking through log streams.

**Mitigation layers:**

1. **`x-sensitive: true`** in input/output schemas marks fields. The reference logging middleware uses `context.redacted_inputs` rather than raw inputs.
2. **`obs.redaction.sensitive_keys`** ships with a canonical default list (D-54) shared across all 3 SDKs (fixture: `sensitive_keys_default`). Adds keys like `password`, `secret`, `api_key`, `authorization`, etc.
3. **`obs.redaction.regex_patterns`** lets you add custom regex for free-form values that aren't behind a known key (e.g., credit-card patterns in error messages).

**What is NOT redacted automatically:**

- The contents of an `error.message` string — see §3.3 below.
- Data leaving the process via custom middleware that bypasses `context.redacted_inputs`.
- Data written to traces / spans by user code.

### 2.6 Trace ID Forgery (T7)

**Threat.** An external caller supplying an attacker-chosen `trace_id` to poison logs, correlate unrelated traffic, or hide their activity.

**Mitigation.** [PROTOCOL_SPEC §10.5](./protocol-spec.md#10-observability-specification) defines a strict validation/normalisation pipeline: every input is either accepted verbatim **after** validation against the 32-char lowercase hex regex, or replaced with a fresh trace_id. There is no path that accepts unvalidated input. Verified by fixture `context_trace_parent`.

---

## 3. Operational Guidance

### 3.1 Security-relevant configuration to verify before production

```yaml
# apcore.yaml (extract — security-critical fields only)
acl:
  default_effect: deny           # MUST be deny in production
  rules:
    - { ... }                    # explicit allows only

approval:
  handler: my.approval.HumanInLoop  # set; do NOT leave None in prod
  timeout_ms: 60000              # cap blocking handlers

policy:
  max_call_depth: 32             # tune to your call graph
  timeout_ms: 30000              # per-module wallclock
  max_inputs_size: 1048576       # cap deserialised input size
  max_schema_depth: 32           # cap recursive schema cost

obs:
  redaction:
    sensitive_keys: [...]        # extend the canonical list with app-specific keys
    regex_patterns: [...]        # add credit-card / SSN / token patterns
```

### 3.2 Audit checklist

For each production deployment:

- [ ] `default_effect: deny` is set.
- [ ] No ACL rule grants `allow` from `*` to a `system.control.*` target without an `identity_types: [system]` or equivalent condition. (Check the spelling: a rule targeting `sys.control.*` matches no module and enforces nothing.)
- [ ] An `ApprovalHandler` is configured and its tokens are bound + single-use.
- [ ] `obs.redaction.sensitive_keys` extends the canonical default with all app-specific PII keys.
- [ ] No module reads `context.data` for authorization decisions (grep your codebase).
- [ ] Untrusted modules run in a separate process or sandbox (see OT1).
- [ ] The extension directory is not writable by unprivileged users.
- [ ] `policy.timeout_ms` and `policy.max_call_depth` are set to bounded values (no `0` / `unlimited`).
- [ ] Trace IDs from external entry points are validated, not propagated raw.
- [ ] CI runs the conformance test suite against the SDK version you ship (see [spec/conformance.md §8](./conformance.md#8-conformance-test-fixtures)).

### 3.3 Error message hygiene

Errors raised by apcore framework code do not include user-supplied secret values in `error.message`. **User module code is responsible for the same hygiene in errors it raises.** Two anti-patterns to avoid:

```python
# BAD — leaks the secret in error.message
raise ValueError(f"invalid token {user_token!r}")

# GOOD — leaks only the structure
raise ValueError(f"invalid token (length={len(user_token)})")
```

Even with `obs.redaction.regex_patterns` configured, the redactor only scrubs **known formats**. A unique-to-your-system credential pattern won't match the default regexes.

### 3.4 Side-channel observations

apcore does not provide constant-time primitives for token comparison or pattern matching. Modules that compare credentials **MUST** use `hmac.compare_digest` / `crypto.timingSafeEqual` / `subtle::ConstantTimeEq` (Python / TS / Rust respectively).

The Executor's per-step timing is observable through the OTel spans. Rules of thumb:

- ACL evaluation time is bounded but **not constant** across rule sets — do not infer the secrecy of a rule list from timing.
- Approval handlers are user code; their timing is whatever the handler chooses.

---

## 4. Reporting Vulnerabilities

Do **not** open public GitHub issues for security bugs. See [SECURITY.md](https://github.com/aiperceivable/apcore/blob/main/SECURITY.md) at the repository root for the disclosure process.

---

## 5. References

- [RFC 3552 — Guidelines for Writing RFC Text on Security Considerations](https://www.rfc-editor.org/rfc/rfc3552)
- [PROTOCOL_SPEC §6 ACL](./protocol-spec.md#6-acl-specification)
- [PROTOCOL_SPEC §7 Approval](./protocol-spec.md#7-approval-system)
- [PROTOCOL_SPEC §8 Errors](./protocol-spec.md#8-error-handling-specification)
- [PROTOCOL_SPEC §10 Observability](./protocol-spec.md#10-observability-specification)
- [features/acl-system.md](../features/acl-system.md) — ACL implementation guide
- [features/approval-system.md](../features/approval-system.md) — approval state machine
- [guides/troubleshooting.md](../guides/troubleshooting.md) — error code reference
- [Conformance fixtures](https://github.com/aiperceivable/apcore/tree/main/conformance/fixtures) — behavioural cross-reference
