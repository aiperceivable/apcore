---
description: "How apcore, an embeddable microframework, governs application capabilities across agent and code entry points."
---

# apcore Positioning

apcore is an **embeddable microframework** for application capabilities that agents and code can call. It pairs a language-agnostic module standard with a protocol-neutral runtime. Applications can embed an SDK and extend the runtime through interfaces for discovery, middleware, access control, approval, validation, and trace export.

## The Problem apcore Owns

Agents can only use application capabilities reliably when four things remain aligned:

1. the input and output contract;
2. the behavioral meaning of the operation;
3. the rules that decide whether the call may execute; and
4. the evidence produced after execution.

Protocol servers can publish tools, but each server still has to implement those runtime rules. Application teams also need the same capability to behave consistently when it is called from direct code, a CLI, an HTTP endpoint, MCP, or A2A.

apcore owns that execution boundary. It defines a capability once, validates the contract, applies ACL and approval policy, runs middleware, executes the implementation, validates the result, and emits traceable outcome data.

## Where apcore Sits

```text
Application code and framework endpoints
                    │
                    ▼
┌───────────────────────────────────────────────┐
│ apcore capability definition and runtime     │
│                                               │
│ schema · annotations · identity · ACL         │
│ approval · middleware · execution · audit     │
└───────────────────────────────────────────────┘
                    │
                    ▼
┌──────────┬──────────┬──────────┬──────────────┐
│ MCP      │ A2A      │ CLI      │ HTTP / code  │
│ adapter  │ adapter  │ adapter  │ integrations │
└──────────┴──────────┴──────────┴──────────────┘
```

Higher-level projects build on this execution boundary. [apexe](https://github.com/aiperceivable/apexe) uses apcore to govern access to existing CLI tools and HTTP APIs. [apflow](https://github.com/aiperceivable/apflow) coordinates distributed tasks and uses the apcore module standard to make capabilities callable by agents.

## What apcore Enforces

### Required capability contract

Every module has a description plus input and output schemas. The runtime validates calls against those schemas. This makes the contract machine-readable and testable; it does not guarantee that a model will choose the right tool or interpret its purpose correctly.

### Runtime governance

Behavioral annotations describe properties such as `readonly`, `destructive`, `idempotent`, `requires_approval`, and `open_world`. ACL, identity, approval policy, and call-chain guards turn those properties into enforceable execution decisions.

### Consistent execution

Every call runs one fixed pipeline: call-chain safety checks, module lookup, access control, approval, `before` middleware, input validation, execution, output validation, `after` middleware, and result handling.

### Operational evidence

Trace context, structured errors, events, metrics hooks, and usage exporters make calls inspectable. Deployments choose their storage, retention, and compliance policy; apcore supplies the runtime signals rather than claiming compliance on their behalf.

## Relationship to MCP and A2A

MCP and A2A define communication surfaces. apcore defines and governs the capability behind a surface.

MCP already supports tool schemas and annotations. apcore does not claim those features are absent. Its value is that the same schema and governance rules can be enforced before a call reaches business logic and reused outside MCP.

The official adapters are independently versioned projects:

| Surface | Project | Role |
|---|---|---|
| MCP | `apcore-mcp` | Project registered modules as MCP tools |
| A2A | `apcore-a2a` | Project modules as A2A skills and Agent Card metadata |
| CLI | `apcore-cli` | Map module schemas to commands and arguments |
| HTTP frameworks | `fastapi-apcore`, `django-apcore`, `flask-apcore`, `nestjs-apcore`, `axum-apcore` | Bind framework endpoints and apcore modules |
| Direct code | Core SDK | Register and invoke modules without a transport adapter |

Use a protocol SDK directly when a protocol surface is the only requirement. Use apcore when validation, access, approval, and audit semantics must stay consistent across more than one caller or surface.

## Supported Implementations

The normative protocol is implemented in Python, TypeScript, and Rust. Cross-language consistency means shared protocol behavior and conformance expectations, not identical internal code structure.

Language SDKs and adapters have separate release lifecycles. Refer to package metadata and each adapter's compatibility declaration rather than assuming all ecosystem packages share one version.

## Adoption Path

The primary adoption path is deliberately narrow:

1. start with one existing application capability;
2. define its schema and behavioral annotations;
3. invoke it through the native SDK with validation, ACL, approval, and trace context;
4. expose it through `apcore-mcp` or another required surface; and
5. verify that denied, approved, failed, and successful calls produce the expected evidence.

Only add orchestration, discovery services, or additional surfaces after this path creates measurable value.

## Claims Discipline

Project documentation and websites must distinguish among:

- **implemented**: present in released code and covered by the repository's tests;
- **supported**: documented with a maintained compatibility contract;
- **experimental**: available for evaluation without a stability promise; and
- **planned**: a roadmap item, not a current capability.

Do not use maturity claims such as “enterprise-grade” or “production ready” without published adoption evidence and an explicit support policy. Do not describe research documents or adjacent projects as part of the normative apcore contract.

## Source of Truth

- [Protocol specification](spec/protocol-spec.md) — normative behavior
- [Scope](https://github.com/aiperceivable/apcore/blob/main/SCOPE.md) — project boundaries
- [Roadmap](https://github.com/aiperceivable/apcore/blob/main/ROADMAP.md) — current priorities and gates
- [Adopters](https://github.com/aiperceivable/apcore/blob/main/ADOPTERS.md) — public adoption evidence
- Language repository package metadata — released SDK versions
