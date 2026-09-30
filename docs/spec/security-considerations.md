---
description: "RFC 3552-style security considerations for apcore: threat model, in-scope mitigations (ACL, approval, call-chain guard, validation, redaction, discovery confinement), residual risks, and production audit guidance."
---

# Security Considerations

> **Type:** Informative (RFC 3552 §4 style). [protocol-spec.md](./protocol-spec.md) is normative and wins on any conflict. **Normative cross-references:** §3 Directory, §4.11 `$ref`, §6 ACL, §7 Approval, §9 Configuration, §10 Observability.

This document lists the threats apcore mitigates, the threats it does not address (so hosts know where defense-in-depth is required), and the checks to run before a production deployment. It follows the IETF [RFC 3552](https://www.rfc-editor.org/rfc/rfc3552) pattern: threat model first, mitigations second, residual risks and operational guidance last.

apcore is a governed runtime, not a sandbox. The host process trusts every loaded module's `execute()` body. This document marks the boundary between "apcore enforces this" and "your host must enforce this."

---

## 1. Threat Model

### 1.1 In-scope threats (apcore mitigates)

| ID  | Threat                                  | Where mitigated                            |
|-----|-----------------------------------------|--------------------------------------------|
| T1  | Unauthorised inter-module invocation    | ACL, pipeline Step 4 — enforced only when an ACL is attached; with no ACL file there is no gate ([§6.6.3.1](./protocol-spec.md#6631-layers-2-and-3-are-inactive-by-absence)) |
| T2  | Sensitive operation without sign-off    | Approval gate, pipeline Step 5 (§2.3, §2.9) |
| T3  | Runaway / stack-blowing recursion       | Call-chain guard, pipeline Step 2 (`executor.max_call_depth`, `executor.max_module_repeat`) |
| T4  | Malformed inputs reaching `execute()`   | Input validation, pipeline Step 7 (JSON Schema) |
| T5  | Output schema drift (data exfiltration) | Output validation, pipeline Step 9         |
| T6  | Sensitive data in logs and audit events | `x-sensitive` + `obs.redaction.*` (§2.5)   |
| T7  | Trace ID forgery / log poisoning        | Trace ID validation ([§10.5](./protocol-spec.md#105-trace-id-format)) |
| T8  | Module-ID spoofing of a privileged namespace | First-segment reserved-word check ([§2.6](./protocol-spec.md#26-id-conflict-detection)) |
| T9  | An `annotations.extra` key shadowing a governance annotation | Canonical field wins; the extension key is discarded with a warning ([§4.4.1](./protocol-spec.md#441-annotations-extension-field-extra-wire-format) rule 12) |
| T10 | Unregistering a module while calls are in flight | Safe unregister: started calls complete, new calls get `MODULE_NOT_FOUND` ([§12.7.4](./protocol-spec.md#1274-hot-reload-race-conditions)) |
| T11 | A symlink in the extensions root pointing outside it | Containment on the canonical real path (§2.7) |
| T12 | Approval bypass through a governance source the gate does not read | Gate fires on the union of all sources (§2.9) |
| T13 | A sensitive marker lost or replaced during `$ref` resolution | Sibling keys preserved; local-pointer fallback scoped to its own document (§2.10) |
| T14 | Prototype pollution through a configuration dot-path | Object-graph segments refused (§2.11) |

### 1.2 Out-of-scope threats (host responsibility)

| ID   | Threat                                  | Required compensating control          |
|------|-----------------------------------------|----------------------------------------|
| OT1  | Untrusted module code execution         | Run untrusted modules in a separate process / container / WASM sandbox; apcore does not isolate globals or memory |
| OT2  | Network egress / SSRF from a module     | Egress firewall, library-level allowlists; apcore does not intercept outbound HTTP |
| OT3  | Filesystem access outside the extensions root | OS-level sandboxing (chroot, AppArmor, seccomp); a module can read any path the process can read |
| OT4  | Denial of service via resource exhaustion (CPU, memory) | OS / cgroup limits; apcore enforces per-call and call-chain timeouts but no memory cap |
| OT5  | Side-channel data leakage (timing, error messages) | See §3.4 |
| OT6  | Supply-chain compromise of an SDK or apcore itself | Signature verification, pinned dependencies, SBOM review |
| OT7  | Oversized or adversarial inputs that amplify validation cost | apcore has no input-size cap: limit request size at the transport boundary and rate-limit callers. `schema.max_ref_depth` bounds `$ref` hops and `stream.max_merge_depth` bounds stream-chunk merging |
| OT8  | Authenticating the calling identity itself | apcore consumes `Identity`; populating it from a verified principal is the host's job |
| OT9  | Replacing an extension file or symlink between discovery and load | Keep the extensions root read-only at runtime, or at the same trust level as the host process (§2.8) |

---

## 2. Detailed Threats and Mitigations

### 2.1 Module-ID Spoofing (T8)

**Threat.** A module placed at a crafted path, or registered programmatically, claims a Canonical ID in a privileged namespace (e.g., `system.control.reload_module`).

**Mitigation.**

1. **The reserved first segment is enforced.** [§2.6 `detect_id_conflicts`](./protocol-spec.md#26-id-conflict-detection) step 2 rejects an ID whose **first segment** is a reserved word from [§2.5](./protocol-spec.md#25-reserved-words) (`system`, `internal`, `core`, `apcore`, `plugin`, `schema`, `acl`; `ephemeral` has its own namespace rule). Later segments are unrestricted: a reserved word claims a namespace, and only the first segment can assert one. `system.*` is registrable only through the privileged `register_internal()` path ([§6.6.1](./protocol-spec.md#661-registration-restriction)). Pinned by fixture `id_conflict_reserved_words`.

    > **`sys` is not reserved.** `sys.control.reload_module` is an ordinary module ID that any user module may claim, and it names no privileged module — the control-plane namespace is `system.*`. An ACL rule or audit check written against `sys.*` matches nothing and silently protects nothing.

2. **Conflict detection.** If two modules resolve to the same Canonical ID, the second is rejected with `MODULE_ID_CONFLICT` rather than silently overriding (fixture: `multi_module_discovery`).
3. **IDs derive from paths inside the root.** A discovered module's ID is derived from its canonical real path relative to the extensions root, and a symlink whose target leaves the root is skipped (§2.7).

**Residual risk.** A user who can write to the extensions root can register modules under any non-reserved ID. Treat write access to the extensions root as a privileged capability.

### 2.2 ACL Bypass (T1)

**Threat.** Crafting an invocation that evades intended access rules.

**No ACL, no gate.** ACL enforcement is opt-in. `APCore` loads `acl/global_acl.yaml` under `acl.root` when that file exists; when it does not, **no ACL is attached** and every inter-module call is allowed. A missing ACL file never synthesizes a default-deny ACL ([§6.6.3.1](./protocol-spec.md#6631-layers-2-and-3-are-inactive-by-absence), D-64). An attached ACL is also not enforced under a strategy that omits the `acl_check` step (`internal`, `testing`, `minimal` — [§6.6.3.2](./protocol-spec.md#6632-a-configured-layer-is-not-necessarily-an-enforced-one)).

**Known evasion patterns and mitigations:**

- **Empty `caller_id`.** A null caller is evaluated as `@external`. It reaches `default_effect` only if no rule matches it.
- **Pattern injection.** ACL patterns are not regular expressions. `*` is the only metacharacter; every other character, `?` included, is a literal ([§6.2](./protocol-spec.md#62-rule-matching)). A pattern containing `?` can never match and is reported as a load warning ([§6.2.2](./protocol-spec.md#622-in-an-acl-pattern-is-dead-and-must-be-reported)).
- **Compound operator abuse.** `["$not", p]` takes exactly one operand. `["$not"]`, `["$not", p1, p2]`, `$or` with no operand, and a reserved token anywhere but index 0 are rejected with `ACLRuleError` at every entry point ([§6.2.1](./protocol-spec.md#621-compound-operators-in-pattern-arrays), [§6.5](./protocol-spec.md#65-edge-case-handling)). There is no silently-widened reading.
- **Misspelled or unevaluable conditions.** A condition with no registered handler, a malformed value, or a handler that throws is **unevaluable**, and the decision resolves toward refusing access rather than skipping the rule ([§6.1.1](./protocol-spec.md#611-unevaluable-conditions-v1220-100)).
- **Missing context with conditional rules.** When `conditions` are present but the call carries no context, the rule does **not** match and evaluation continues. A conditional `deny` is therefore not a backstop for context-less calls; express the backstop as an unconditional `deny` rule or `default_effect: deny` ([§6.5](./protocol-spec.md#65-edge-case-handling)).

**`default_effect: deny` is the only safe production setting.** It is set in the ACL file itself; `acl.default_effect` in `apcore.yaml` is inert. `allow` is for narrow opt-in scenarios and needs explicit `deny` rules for sensitive targets (see the warning in [features/acl-system.md](../features/acl-system.md)).

### 2.3 Approval Gate Replay (T2)

**Threat.** A `_approval_token` reused after the underlying decision should have expired, or used by a different caller than the original requester.

**Mitigation.** The protocol does not specify token format — that is delegated to `ApprovalHandler` implementations. Handlers **SHOULD**:

1. Make tokens **single-use** (consume on first `check_approval` success).
2. **Bind tokens to `(caller_id, target_id, input_hash)`** so they cannot be replayed against a different invocation.
3. Set **expiry timestamps** and reject expired tokens with `status: timeout`.
4. Record approver identity (`approved_by`) for audit.

**No handler, no gate.** With no `ApprovalHandler` configured, Step 5 is skipped with a warning and a module that requires approval executes anyway. `ExecutionPolicy(strict = true)` makes the gate fail closed instead ([§7.9.4](./protocol-spec.md#794-fail-loud-not-silent-security-principle)).

**Residual risk.** A handler that issues unbound, long-lived tokens is vulnerable to replay. This is an implementation defect, not a protocol flaw.

### 2.4 `context.data` Injection (T1, OT5)

**Threat.** A module writing untrusted data to `context.data` that downstream middleware or modules trust as authoritative.

**Mitigation.**

1. `context.data` is a shared scratchpad with namespaced keys; `_apcore.` is the framework's prefix ([features/context-object.md](../features/context-object.md)). Modules **SHOULD** namespace their own writes (`<module-id>.<key>`) and **SHOULD NOT** read another module's keys.
2. Identity is carried in `context.identity`, **not** `context.data`. The `Identity` sub-schema is structurally fixed (`id`, `type`, `roles`, `attrs`) and **SHOULD** be populated only at trust-boundary entry points.

**Residual risk.** A module that writes a key like `_apcore.identity.roles=["admin"]` to `context.data` cannot escalate apcore-level privileges (ACL reads `context.identity`, not `context.data`), but **could** mislead custom middleware that reads from `context.data`. Custom middleware **MUST NOT** consult `context.data` for authorization decisions.

### 2.5 Sensitive Data in Logs (T6)

**Threat.** PII, credentials, or session tokens leaking through log streams, captured inputs, or audit events.

**Mitigation layers:**

1. **`x-sensitive: true`** in input/output schemas marks fields ([§10.6](./protocol-spec.md#106-sensitive-data-redaction)). The executor fills `context.redacted_inputs` / `redacted_output`, and logging middleware reads those rather than raw values.
2. **`obs.redaction.sensitive_keys`** matches field names (substring, or anchored glob when the entry contains `*` or `?`). The default is a 16-entry list shared by all three SDKs (fixture: `sensitive_keys_default`). **Setting the key replaces the default list**; to extend it, copy the defaults into your list.
3. **`obs.redaction.regex_patterns`** matches string values by unanchored, case-insensitive search. A pattern that fails to compile is reported by `validate_config()`, never dropped silently ([§10.6.1](./protocol-spec.md#1061-configured-redaction-rules-obsredaction)).
4. Rules 1–3 apply as a union at both log emission and the executor's input/output capture point. The five correlation fields `trace_id`, `span_id`, `caller_id`, `module_id` and `target_id` are never redacted.
5. **Audit-event identity.** Identity `attrs` copied into governance audit events are redacted to `"<redacted>"` when the attribute name contains any of `token`, `secret`, `password`, `passwd`, `key`, `auth`, `credential`, `cookie`, `session` or `bearer`. The list is bare substrings on purpose, so `signing_key`, `auth_header` and `session_id` are all caught, and it applies even when `obs.redaction` is not configured (D-93; [features/system-modules.md](../features/system-modules.md)).

**What is NOT redacted automatically:**

- The contents of an `error.message` string — see §3.3.
- Data leaving the process via custom middleware that bypasses `context.redacted_inputs`.
- Data written to traces / spans by user code.

### 2.6 Trace ID Forgery (T7)

**Threat.** An external caller supplying an attacker-chosen `trace_id` to poison logs, correlate unrelated traffic, or hide their activity.

**Mitigation.** [§10.5](./protocol-spec.md#105-trace-id-format) accepts an inbound trace parent only if it matches `^[0-9a-f]{32}$` and is not the W3C-invalid all-zero or all-`f` value; anything else is replaced with a fresh trace ID and the request continues. There is no path that accepts unvalidated input. Verified by fixture `context_trace_parent`.

### 2.7 Symlink Escape During Discovery (T11)

**Threat.** A symlink inside the extensions root whose target is outside it causes code outside the root to be discovered and executed.

**Mitigation** ([§3.4](./protocol-spec.md#34-symbolic-link-handling), [§3.6](./protocol-spec.md#36-scanning-algorithm)):

1. `extensions.follow_symlinks` defaults to `false`; symlinks are then skipped entirely.
2. With `follow_symlinks: true`, every symlink — file or directory — is resolved to its canonical real path **before** the directory/file split. A target outside the canonical extensions root is **skipped with a warning** (D-94).
3. Each real file is recorded once, and its module ID is derived from its canonical real path, so an alias cannot register a second ID for the same code; a directory whose real path was already visited is skipped, which also terminates cycles (D-127).

**Residual risk.** Containment is checked when the scanner looks, not when the loader opens the file. See §2.8.

### 2.8 Discovery TOCTOU (OT9)

**What is guaranteed.** At the moment the scanner looked, every path it recorded resolved inside the extensions root, and each real file was recorded once (§2.7).

**What is not.** The scanner resolves and records a path, and the loader opens the file afterwards. A party who can write into the extensions root can replace a symlink in that interval, and the loader then opens a target that was never checked. Checking again does not close the window: `realpath` followed by `open` is two operations on a mutable namespace, and a third check only moves it.

**When it matters:**

| Deployment | Reachable? |
|---|---|
| Extensions root is part of the deployed artifact, read-only at runtime | **No.** Nothing can replace the link. |
| Root is operator-managed, at the same trust level as the host process | **No** in any useful sense — a party who can write there can replace the module file itself. |
| Root is writable by a less-trusted party (shared volume, upload directory, multi-tenant plugin drop) | **Yes.** |

Only the third row is an exposure, and there the symlink window is the lesser problem: the same writer can drop a module that passes every check and does anything inside `execute()`. Confinement is not the control that protects that deployment.

**Current position.** apcore assumes the extensions root is at least as trusted as the host process, and a root writable by a less-trusted party is outside the threat model (see [SECURITY.md](https://github.com/aiperceivable/apcore/blob/main/SECURITY.md) § *Trust boundary: the extensions root*). The SDKs do not add a re-check before load: it would cost a syscall, move the window, and suggest the case is handled.

**Options if that assumption changes:**

1. **Open-then-verify** — open the file, `fstat` the descriptor, confirm it is the inode the scan recorded, and load from the descriptor. Closes the file half; Python and TypeScript import by path, so the loader would have to change.
2. **Directory-descriptor traversal** (`openat` / `O_NOFOLLOW`). Closes the directory half too; needs per-platform work, and Node's API for it is thin.
3. **Snapshot the root** — copy or hardlink the tree to a private location and load from there. Removes the window at the cost of a copy per discovery, and breaks hot reload, which watches the original tree.

Any of these becomes worth its cost if apcore supports an extensions root writable by a less-trusted party, a hosted or multi-tenant plugin model, or a loader that already accepts a descriptor.

### 2.9 Approval-Source Bypass (T12)

**Threat.** An operator declares `requires_approval` (or `destructive` under `gate_destructive`) in one place, and the gate reads a different one, so the module runs ungated.

**Mitigation.** The approval requirement is the **union** of every governance source, and no source can cancel another ([§7.4](./protocol-spec.md#74-executor-integration-step-5), [§6.9](./protocol-spec.md#69-governance-precedence-v1280-108)):

- the module's own annotations;
- annotations declared for the module in the registry — a binding file, `*_meta.yaml`, `metadata`, or a supplied descriptor (D-96, D-97, D-125);
- an ACL rule with `approval: required`;
- `ExecutionPolicy.gate_destructive` for a `destructive` module ([§7.9.2](./protocol-spec.md#792-destructive-approval-resolution)).

A policy may add a requirement but never removes one the ACL set. The union is `OR` on `requires_approval` and `destructive` only, and it binds every reader: the gate, the `validate()` preflight, the governance-posture accessor, and the `system.manifest.*` projection all report the same value. The metadata merge precedence of [§4.13](./protocol-spec.md#413-annotation-conflict-rules) (YAML over code) is not used for this: under it, a metadata `false` would cancel a module that declares `true`.

### 2.10 Redaction Bypass Through `$ref` (T13)

**Threat.** Because [§10.6](./protocol-spec.md#106-sensitive-data-redaction) reads `x-sensitive` from the **resolved** schema, a resolver that drops or swaps schema nodes can turn a sensitive field into a plaintext one.

**Mitigation** ([§4.11](./protocol-spec.md#411-schema-references-ref)):

1. **Sibling keys are preserved.** Keys beside a `$ref` — `x-sensitive` included — survive resolution and apply to the resolved node (D-98).
2. **The local-pointer fallback stays in its own document.** A `#/…` pointer resolves against the file root, then falls back to the schema node, but only while resolution is still inside the document that node belongs to. Inside an external document, a pointer that does not resolve there throws `SCHEMA_NOT_FOUND`; it never binds to a same-named definition in the calling module's schema (D-124).

### 2.11 Prototype Pollution Through Configuration Paths (T14)

**Threat.** A configuration dot-path such as `__proto__.polluted` walks out of the configuration object into the host object graph. The `APCORE_*` environment loader maps `APCORE_____PROTO_____POLLUTED` to exactly that path, so the attack needs no module call, ACL decision, or approval.

**Mitigation.** A configuration dot-path addresses data, never the host object graph (D-95). The TypeScript SDK refuses the whole write, with a warning, when any segment is `__proto__`, `constructor` or `prototype`, and uses own-property checks on both read and write paths. Python dictionaries and Rust maps have no prototype chain, so they store such a segment as an ordinary key.

---

## 3. Operational Guidance

### 3.1 Security-relevant configuration to verify before production

`apcore.yaml` — every key below is live; see [§9.1.1](./protocol-spec.md#911-default-values-summary) for defaults:

```yaml
# apcore.yaml (required keys plus the security-relevant ones)
version: "1.0.0"
project:
  name: my-service

extensions:
  root: ./extensions
  follow_symlinks: false          # default; see §2.7 before enabling

acl:
  root: ./acl                     # acl/global_acl.yaml is loaded when present

schema:
  root: ./schemas
  max_ref_depth: 32               # bound $ref hops

executor:
  default_timeout: 30000          # per-module wallclock, ms (0 = no limit)
  global_timeout: 60000           # whole call chain, ms (0 = no limit)
  max_call_depth: 32
  max_module_repeat: 3

stream:
  max_merge_depth: 32

observability:
  tracing:
    enabled: true
    exporter: otlp
    otlp_endpoint: http://otel-collector:4318

obs:
  redaction:
    # Setting sensitive_keys REPLACES the 16-entry default: keep the defaults
    # and append application-specific keys.
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
      - ssn                       # application-specific
    regex_patterns:
      - "\\b\\d{4}[- ]?\\d{4}[- ]?\\d{4}[- ]?\\d{4}\\b"   # card-number shape
```

`acl/global_acl.yaml` — the ACL file owns `default_effect` and the `audit:` block ([§6.3.2](./protocol-spec.md#632-audit-delivery-audit-in-the-acl-file)):

```yaml
# acl/global_acl.yaml
rules:
  - callers: ["@external"]
    targets: ["system.*"]
    effect: deny
    description: "Block external access to system modules"
  - callers: ["api.*"]
    targets: ["orchestrator.*"]
    effect: allow
    description: "API layer calls orchestration layer"

default_effect: deny              # the only safe production setting

audit:
  enabled: true
  log_level: info
  include_denied: true
```

The approval handler and `ExecutionPolicy` are code, not configuration. Wire them on the executor; `strict` makes a missing handler fail closed, and `gate_destructive` gates every `destructive` module:

=== "Python"

    ```python
    from apcore import (
        APCore,
        ApprovalRequest,
        ApprovalResult,
        CallbackApprovalHandler,
        Config,
        ExecutionPolicy,
    )


    async def ask_human(request: ApprovalRequest) -> ApprovalResult:
        # Replace with a real round-trip (ticket, chat, console prompt).
        approved = False
        if approved:
            return ApprovalResult(status="approved", approved_by="oncall@example.com")
        return ApprovalResult(status="rejected", reason=f"{request.module_id} not approved")


    config = Config.load("apcore.yaml")
    client = APCore(
        config=config,
        policy=ExecutionPolicy(gate_destructive=True, strict=True),
    )
    client.executor.set_approval_handler(CallbackApprovalHandler(ask_human))
    print(client.list_modules())
    ```

=== "TypeScript"

    ```typescript
    import {
      APCore,
      CallbackApprovalHandler,
      Config,
      ExecutionPolicy,
      createApprovalResult,
    } from "apcore-js";
    import type { ApprovalRequest, ApprovalResult } from "apcore-js";

    async function askHuman(request: ApprovalRequest): Promise<ApprovalResult> {
      // Replace with a real round-trip (ticket, chat, console prompt).
      const approved = false;
      return approved
        ? createApprovalResult({ status: "approved", approvedBy: "oncall@example.com" })
        : createApprovalResult({ status: "rejected", reason: `${request.moduleId} not approved` });
    }

    const config = Config.load("apcore.yaml");
    const client = new APCore({
      config,
      policy: new ExecutionPolicy(null, { gateDestructive: true, strict: true }),
    });
    client.executor.setApprovalHandler(new CallbackApprovalHandler(askHuman));
    console.log(client.listModules());
    ```

=== "Rust"

    ```rust
    use std::path::Path;
    use std::sync::Arc;

    use apcore::{
        APCore, ApprovalRequest, ApprovalResult, CallbackApprovalHandler, Config,
        ExecutionPolicy, Executor, ModuleError, Registry, ACL,
    };

    async fn ask_human(request: ApprovalRequest) -> Result<ApprovalResult, ModuleError> {
        // Replace with a real round-trip (ticket, chat, console prompt).
        let approved = false;
        if approved {
            Ok(ApprovalResult::approved("oncall@example.com"))
        } else {
            Ok(ApprovalResult::rejected(format!("{} not approved", request.module_id)))
        }
    }

    fn build_client() -> Result<APCore, ModuleError> {
        let config = Config::load(Path::new("apcore.yaml"))?;
        let registry = Arc::new(Registry::default());

        // `set_approval_handler` needs `&mut Executor`, so build the executor
        // first. APCore skips config-driven ACL discovery for a supplied
        // executor, so attach the ACL here.
        let mut executor = Executor::new(Arc::clone(&registry), config.clone());
        if let Some(acl) = ACL::discover(&config)? {
            executor.set_acl(acl);
        }
        executor.set_policy(Some(
            ExecutionPolicy::new(vec![])
                .with_gate_destructive(true)
                .with_strict(true),
        ));
        executor.set_approval_handler(Box::new(CallbackApprovalHandler::new(ask_human)));

        Ok(APCore::with_options(None, Some(executor), Some(config), None))
    }

    fn main() -> Result<(), ModuleError> {
        let client = build_client()?;
        println!("{:?}", client.list_modules(None, None));
        Ok(())
    }
    ```

### 3.2 Audit checklist

For each production deployment:

- [ ] An ACL file exists under `acl.root` and is loaded (a missing file means **no** ACL, not default-deny), and the running strategy contains the `acl_check` step.
- [ ] The ACL file sets `default_effect: deny`.
- [ ] No ACL rule grants `allow` from `*` to a `system.control.*` target without an `identity_types: [system]` or equivalent condition. (Check the spelling: a rule targeting `sys.control.*` matches no module and enforces nothing.)
- [ ] An `ApprovalHandler` is configured, its tokens are bound and single-use, and `ExecutionPolicy(strict = true)` is set so a missing handler fails closed.
- [ ] `obs.redaction.sensitive_keys`, if set, still contains the 16 defaults plus all application-specific PII keys.
- [ ] `validate_config()` reports no uncompilable `obs.redaction.regex_patterns` entry.
- [ ] No module reads `context.data` for authorization decisions (grep your codebase).
- [ ] Untrusted modules run in a separate process or sandbox (see OT1).
- [ ] The extensions root is not writable by unprivileged users, and `extensions.follow_symlinks` is `false` unless you need it (§2.7, §2.8).
- [ ] `executor.default_timeout` and `executor.global_timeout` are non-zero (`0` disables the limit), and `executor.max_call_depth` fits your call graph.
- [ ] Trace IDs from external entry points are validated, not propagated raw.
- [ ] CI runs the conformance fixtures against the SDK version you ship (see [conformance.md §8](./conformance.md#8-conformance-test-fixtures)).

### 3.3 Error message hygiene

Errors raised by apcore framework code do not include user-supplied secret values in `error.message`. **User module code is responsible for the same hygiene in errors it raises:**

```python
# BAD — leaks the secret in error.message
raise ValueError(f"invalid token {user_token!r}")

# GOOD — leaks only the structure
raise ValueError(f"invalid token (length={len(user_token)})")
```

Even with `obs.redaction.regex_patterns` configured, the redactor only scrubs **known formats**. A credential format unique to your system will not match unless you add a pattern for it.

### 3.4 Side-channel observations

apcore does not provide constant-time primitives for token comparison or pattern matching. Modules that compare credentials **MUST** use `hmac.compare_digest` / `crypto.timingSafeEqual` / `subtle::ConstantTimeEq` (Python / TypeScript / Rust respectively).

The executor's per-step timing is observable through tracing spans. Rules of thumb:

- ACL evaluation time is bounded but **not constant** across rule sets — do not infer the secrecy of a rule list from timing.
- Approval handlers are user code; their timing is whatever the handler chooses.

---

## 4. Reporting Vulnerabilities

Do **not** open public GitHub issues for security bugs. See [SECURITY.md](https://github.com/aiperceivable/apcore/blob/main/SECURITY.md) at the repository root for the disclosure process.

---

## 5. References

- [RFC 3552 — Guidelines for Writing RFC Text on Security Considerations](https://www.rfc-editor.org/rfc/rfc3552)
- [Protocol spec §3 Directory](./protocol-spec.md#3-directory-specification)
- [Protocol spec §6 ACL](./protocol-spec.md#6-acl-specification)
- [Protocol spec §7 Approval](./protocol-spec.md#7-approval-system)
- [Protocol spec §8 Errors](./protocol-spec.md#8-error-handling-specification)
- [Protocol spec §10 Observability](./protocol-spec.md#10-observability-specification)
- [features/acl-system.md](../features/acl-system.md) — ACL implementation guide
- [features/approval-system.md](../features/approval-system.md) — approval state machine
- [guides/troubleshooting.md](../guides/troubleshooting.md) — error code reference
- [Conformance fixtures](https://github.com/aiperceivable/apcore/tree/main/conformance/fixtures) — behavioural cross-reference
