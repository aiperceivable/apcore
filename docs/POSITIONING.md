---
description: "Positions apcore as a protocol-agnostic module standard complementary to MCP, A2A, CLI, and REST, where surface adapters bridge one module definition onto each delivery protocol."
---

# apcore: The Module Standard Every AI Interface Builds On

> **TL;DR**: apcore is **complementary to MCP, A2A, and other protocols — not a replacement for them.** It is a foundational module standard, not tied to any single protocol: you define a module once with enforced schemas, behavioral annotations, and access control, and surface adapters bridge that one definition onto each protocol (MCP via `apcore-mcp`, CLI via `apcore-cli`, REST via framework adapters, …). apcore solves **how to build AI-perceivable modules**; protocols solve **how to deliver** them — the two layers work together.

---

## The Core Idea

```
"Build once, invoke by Code or AI — through any protocol."
```

Traditional software has **UI** for humans and **API** for programs. apcore adds a third interface: the **Cognitive Interface** for AI Agents — modules that AI can perceive, understand, and invoke correctly.

---

## Where apcore Sits in the Stack

```
┌──────────────────────────────────────────────────────────┐
│                   AI Clients / Callers                   │
│     Claude   Cursor   GPT   Other Agents   Human CLI     │
└────────┬──────────┬──────────┬──────────┬──────────┬─────┘
         │          │          │          │          │
         ▼          ▼          ▼          ▼          ▼
┌──────────────────────────────────────────────────────────┐
│                     Surface Adapters                     │
│           MCP    A2A    CLI    REST    Future…           │
└────────┬──────────┬──────────┬──────────┬──────────┬─────┘
         │          │          │          │          │
         ▼          ▼          ▼          ▼          ▼
┌──────────────────────────────────────────────────────────┐
│                  apcore Module Standard                  │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │ Execution Engine (11-step pipeline)                │  │
│  │ ACL → Approval → Validation → Middleware →         │  │
│  │ Execute → Validation → Middleware → Return         │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │ Registry (auto-discovery, ID mapping, caching)     │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │ Modules (user-written business logic)              │  │
│  │ Enforced: input_schema + output_schema +           │  │
│  │           description + annotations + ACL          │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│          Python SDK · TypeScript SDK · Rust SDK          │
└──────────────────────────────────────────────────────────┘
```

**apcore is not at the same layer as MCP, A2A, or CLI — it is complementary to them.** It is the definition layer each of these protocols projects from: you write the module once, and every protocol builds its own surface on top of that single definition.

---

## One Module, Every Surface

Define a module once:

```python
@module(
    id="email.send",
    description="Send an email to a recipient",
    annotations={"destructive": False, "idempotent": True}
)
def send_email(to: str, subject: str, body: str) -> dict:
    return {"success": True, "message_id": "..."}
```

Expose through any surface — zero duplication:

| Surface | Adapter | What It Does |
|---------|---------|-------------|
| **MCP Server** | `apcore-mcp` | Auto-projects modules as MCP tools for Claude, Cursor |
| **A2A Agent** | `apcore-a2a` | Auto-generates Agent Cards, exposes modules as agent skills |
| **CLI Tool** | `apcore-cli` | Auto-maps modules to terminal commands with argument parsing |
| **REST API** | `flask-apcore`, `fastapi-apcore`, `django-apcore`, `nestjs-apcore`, `axum-apcore` | Auto-generates HTTP endpoints |
| **Direct Code** | Native SDK | Import and call in Python, TypeScript, or Rust |
| **Future** | Community adapters | gRPC, GraphQL, OpenAI Tools, or any new protocol |

Each adapter reads the same module definition and translates it to the target protocol. The module itself never changes.

### Surface-Specific Display Overlays

Each surface can customize how a module is presented — without changing the canonical definition:

```yaml
bindings:
  - module_id: "executor.delete_user"
    display:
      cli:
        alias: "remove-user"
        tags: ["dangerous", "admin-only"]
      mcp:
        alias: "delete_user"
      a2a:
        alias: "user.delete"
```

Same module, optimized presentation for each protocol.

---

## What apcore Enforces

### 1. Schema Enforcement

Every module **must** declare `input_schema`, `output_schema`, and `description`. This is not optional — AI agents receive guaranteed-accurate type information regardless of which surface they use.

### 2. Three-Layer Metadata

```
┌──────────────────────────────────────────────────────┐
│ CORE (Required)                                      │
│ input_schema / output_schema / description           │
│ → AI perceives: "what this module does"              │
├──────────────────────────────────────────────────────┤
│ ANNOTATIONS (Recommended)                            │
│ readonly / destructive / requires_approval /         │
│ idempotent / open_world / cacheable / paginated      │
│ → AI understands: "how to use it safely"             │
├──────────────────────────────────────────────────────┤
│ EXTENSIONS (Optional)                                │
│ x-when-to-use / x-common-mistakes / x-cost-per-call  │
│ x-preconditions / x-rate-limit                       │
│ → AI gains wisdom: "when and why to use it"          │
└──────────────────────────────────────────────────────┘
```

### 3. Behavioral Annotations as Cognitive Signals

| Annotation | Signal to AI | Effect |
|------------|-------------|--------|
| `readonly=True` | "Safe to call autonomously" | No confirmation needed |
| `destructive=True` | "Irreversible operation" | AI requests human confirmation |
| `requires_approval=True` | "Must have explicit consent" | Execution pauses for approval |
| `idempotent=True` | "Safe to retry" | AI can retry on failure |
| `open_world=True` | "Connects to external system" | AI informs user |
| `cacheable=True` | "Output can be reused" | AI avoids redundant calls |
| `paginated=True` | "Returns partial results" | AI handles cursor/offset |

### 4. Access Control (ACL)

Pattern-based, role-aware governance — enforced at the module level, applied consistently across all surfaces:

```yaml
rules:
  - callers: ["api.*"]
    targets: ["orchestrator.*"]
    effect: allow
  - callers: ["api.*"]
    targets: ["executor.*"]
    effect: deny
  - callers: ["@external"]
    targets: ["executor.payment.*"]
    conditions:
      roles: ["admin", "finance"]
    effect: allow
default_effect: deny
```

Whether a module is invoked via MCP, CLI, REST, or direct code — the same ACL applies.

### 5. AI-Guided Error Recovery

Every error includes guidance for autonomous AI recovery:

```json
{
  "code": "SCHEMA_VALIDATION_ERROR",
  "message": "Invalid input",
  "ai_guidance": "The user_id must be UUID format. Check the user record and retry.",
  "retryable": false,
  "user_fixable": true
}
```

### 6. 11-Step Execution Pipeline

Every module invocation, regardless of surface, passes through:

1. **Context processing** — trace_id, caller_id, call_chain
2. **Safety checks** — depth limit, circular call detection
3. **Module lookup** — from Registry
4. **ACL enforcement** — permission verification
5. **Approval gate** — human approval if required
6. **Input validation** — against input_schema
7. **Middleware before** — auth, logging, transformation
8. **Module execution** — business logic with timeout
9. **Output validation** — against output_schema
10. **Middleware after** — post-processing
11. **Result return** — with trace metadata

This pipeline guarantees that every call is validated, governed, observable, and auditable — no matter how it arrives.

---

## The Cognitive Interface Lifecycle

Traditional software provides UI for humans and API for programs. apcore provides the **Cognitive Interface** for AI:

```
1. DISCOVERY     →  description, schema
   "I see a tool that handles payments"

2. STRATEGY      →  x-when-to-use, x-common-mistakes
   "I should use this for refunds, not for new charges"

3. GOVERNANCE    →  requires_approval, destructive, ACL
   "This is destructive — I need human approval first"

4. RECOVERY      →  ai_guidance, retryable
   "Input was wrong — I'll fix the format and retry"
```

This lifecycle works identically whether the AI reaches the module through MCP, A2A, CLI, REST, or any future protocol.

---

## How apcore Relates to Each Protocol

### MCP (Model Context Protocol)

| | MCP | apcore |
|---|-----|--------|
| **Layer** | Transport — client-server communication | Foundation — module definition and governance |
| **Solves** | How to send messages between Claude and a tool server | How to build modules that are perceivable, governed, and validated |
| **Bridge** | `apcore-mcp` auto-projects modules as MCP tools | |
| **Annotation mapping** | `readOnlyHint`, `destructiveHint` | `readonly`, `destructive`, `requires_approval`, `idempotent`, `cacheable`, `paginated` |

**apcore-mcp** auto-discovers modules from the Registry and exposes them as MCP tools — zero code changes. Annotations, schemas, and ACL are preserved through the bridge.

### A2A (Agent-to-Agent Protocol)

| | A2A | apcore |
|---|-----|--------|
| **Layer** | Communication — agent-to-agent messaging | Foundation — module/skill definition |
| **Solves** | How agents discover and communicate with each other | How to define the skills that agents expose |
| **Bridge** | `apcore-a2a` auto-generates Agent Cards and skill endpoints | |
| **Discovery** | `/.well-known/agent.json` | Auto-generated from module registry |

**apcore-a2a** transforms apcore modules into A2A-compatible agent skills with auto-generated Agent Cards — including identity, capabilities, and governance metadata.

### CLI (Command-Line Interface)

| | Traditional CLI | apcore |
|---|-----|--------|
| **Layer** | Presentation — terminal commands | Foundation — module definition |
| **Solves** | How to parse arguments and format output | How to define the operations behind commands |
| **Bridge** | `apcore-cli` auto-maps modules to commands | |
| **Parameters** | Manual argparse/click | Auto-inferred from input_schema |

**apcore-cli** generates CLI commands from module definitions — argument parsing, help text, and input validation come from the schema automatically.

### REST / Web Frameworks

| | Traditional REST | apcore |
|---|-----|--------|
| **Layer** | Presentation — HTTP endpoints | Foundation — module definition |
| **Solves** | How to route HTTP requests | How to define the logic behind endpoints |
| **Adapters** | `flask-apcore`, `fastapi-apcore`, `django-apcore`, `nestjs-apcore`, `axum-apcore` | |
| **Routes** | Manual endpoint definition | Auto-mapped from module ID |

Framework adapters scan the module registry and generate HTTP endpoints — with request/response validation, ACL enforcement, and observability built in.

### Future Protocols

apcore's adapter architecture is extensible. Any new protocol can be supported by writing an adapter that reads from the module registry. Potential future surfaces include:

- **gRPC** — auto-generate .proto from module schemas
- **GraphQL** — auto-generate queries/mutations from modules
- **OpenAI Tools** — project modules as OpenAI function calls
- **WebSocket** — streaming module invocation
- **Any protocol not yet invented** — the module standard remains stable

---

## Cross-Language Consistency

The same module specification works across Python, TypeScript, and Rust:

```
apcore Protocol Specification (language-agnostic)
        ↓
┌───────────────┬────────────────┬──────────────┐
│ apcore-python │ apcore-ts      │ apcore-rust  │
│ pip install   │ npm install    │ cargo add    │
└───────┬───────┴────────┬───────┴──────┬───────┘
        ↓                ↓              ↓
   Same schemas, same annotations, same ACL,
   same 11-step pipeline, same behavior
```

A module defined in Python behaves identically when ported to TypeScript or Rust. Cross-language conformance tests verify this.

---

## Four Integration Paths

apcore supports progressive adoption for existing codebases:

| Path | Intrusion Level | Approach |
|------|----------------|----------|
| **Class-based Module** | Full adoption | Implement `Module` interface with explicit schemas |
| **@module Decorator** | Low intrusion | Decorate existing functions, schemas auto-inferred |
| **module() Function** | Very low intrusion | Wrap existing methods at registration time |
| **External YAML Binding** | Zero code modification | Map existing functions via external config files |

---

## Summary

| Question | Answer |
|----------|--------|
| **What is apcore?** | A module standard for AI-perceivable tools — enforced schemas, behavioral annotations, access control |
| **What does it solve?** | How to build modules that any AI can perceive, understand, and invoke correctly |
| **What doesn't it solve?** | Transport protocols, workflow orchestration, LLM invocation — those are separate concerns |
| **Relationship to MCP?** | MCP is one surface. apcore modules auto-expose as MCP tools via `apcore-mcp` |
| **Relationship to A2A?** | A2A is one surface. apcore modules auto-expose as agent skills via `apcore-a2a` |
| **Relationship to CLI?** | CLI is one surface. apcore modules auto-expose as commands via `apcore-cli` |
| **Relationship to REST?** | REST is one surface. Framework adapters auto-generate endpoints |
| **Cross-language?** | Python, TypeScript, Rust — identical behavior guaranteed |
| **Extensible?** | Any future protocol can be supported by writing an adapter |

**Protocols deliver; the module standard defines — apcore is the definition layer every protocol projects from.**

---

## Links

* **Official Website:** [aiperceivable.com](https://aiperceivable.com) (Vision & Concept)
* **Protocol Core & Documentation:** [apcore.aiperceivable.com/POSITIONING/](https://apcore.aiperceivable.com/POSITIONING/) (The Architecture Standard)
* **GitHub Organization:** [github.com/aiperceivable](https://github.com/aiperceivable) (Open Source Ecosystem & SDKs)
