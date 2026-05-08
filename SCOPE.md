# apcore — AI-Perceivable Core Scope Definition

> Clarify what we do and what we don't do.

## 1. Project Positioning

**One-line definition**:
> apcore (AI-Perceivable Core) is an AI-Perceivable module standard that makes every interface naturally perceivable and understandable by AI through enforced Schema definitions and behavioral annotations.

**Slogan**:
> Build once, invoke by Code or AI.

**Core problem**:
> How to build modules that can be both invoked by code and perceived and understood by AI/LLMs?

**Answer**:
> Enforce modules to define `input_schema` / `output_schema` / `description`, making modules inherently AI-Perceivable.

**Positioning**:
```
apcore = AI-Perceivable module standard + Enforced Schema definitions

Not: A framework that can only be used in AI scenarios
But: A universal module standard that is naturally AI-Perceivable
```

**Analogy**:
- apcore is similar to "Django/Spring" (defines how to build modules)
- MCP is similar to "HTTP protocol" (defines communication format)
- apflow is "an application built on apcore"

---

## 2. Core Responsibilities

> This section uses RFC 2119 keywords; see [docs/spec/protocol-spec.md §1.5](./docs/spec/protocol-spec.md#15-specification-keywords) for their meanings.

### 2.1 Module Standardization
- [ ] **Definition (MUST)**: All modules **must** define ID, input_schema, output_schema, and description
- [ ] **Discovery (MUST)**: Implementations **must** support automatic module discovery based on directory scanning
- [ ] **Loading (SHOULD)**: Implementations **should** support lazy loading and dependency injection
- [ ] **Invocation (MUST)**: Implementations **must** automatically perform input validation, output validation, and error handling upon invocation

### 2.2 Schema System
- [ ] **Definition (MUST)**: All modules **must** define input_schema / output_schema, based on JSON Schema Draft 2020-12
- [ ] **Validation (MUST)**: Implementations **must** perform Schema validation on input and output at runtime
- [ ] **Conversion (SHOULD)**: Implementations **should** support conversion from YAML Schema to language-native types

### 2.3 Naming and Addressing
- [ ] **Directory as ID (MUST)**: The directory path **must** serve as the single source of truth for the module ID
- [ ] **ID Map (SHOULD)**: Implementations **should** provide automatic cross-language ID conversion, and **may** allow overrides via YAML configuration

### 2.4 Access Control
- [ ] **ACL (MUST)**: Implementations **must** provide an inter-module invocation access control mechanism
- [ ] **Audit (SHOULD)**: Implementations **should** support invocation audit logs

### 2.5 Observability
- [ ] **Tracing (MUST)**: trace_id **must** be propagated throughout the call chain
- [ ] **Logging (SHOULD)**: Implementations **should** provide structured logging support
- [ ] **Metrics (MAY)**: Implementations **may** collect metrics such as invocation count, latency, etc.

### 2.6 Extension Mechanism
- [ ] **Middleware (MUST)**: Implementations **must** provide before/after/on_error middleware hooks
- [ ] **Extension Points (SHOULD)**: Implementations **should** provide replaceable extension points such as loaders, executors, etc.

### 2.7 Existing Application Integration
- [ ] **`module()` Registration (MUST)**: Implementations **must** provide a `module()` mechanism to wrap existing callables (functions or methods) as standard modules; languages that support Decorator syntax should provide it in Decorator form, while languages that don't should provide it as a function call
- [ ] **External Schema Binding (MUST)**: The standard **must** support YAML binding files to map existing functions as modules with zero code modification
- [ ] **Type Inference (MUST)**: Implementations **must** support automatic JSON Schema generation from language-native type information

---

## 3. Appendix Reference (Non-core, for Reference Only)

The following content is **not part of the core protocol** and serves only as **usage scenario mapping references**:

### 3.1 AI Protocol Mapping Reference
- MCP (Model Context Protocol) mapping approach
- A2A (Agent-to-Agent) mapping approach
- OpenAI Function Calling mapping approach

> These are references for "how to expose apcore modules to external AI systems", not core apcore functionality.

---

## 4. Out of Scope (Won't Do)

Things explicitly **not within the project scope**:

| Won't Do | Reason | Owned By |
|------|------|--------|
| **Workflow Engine** | Application-layer orchestration logic, not framework core | apflow and other upstream projects |
| **MCP/A2A/CLI Adapters** | Protocol and surface adaptation, not core standard | [apcore-mcp](https://github.com/aiperceivable/apcore-mcp), [apcore-a2a](https://github.com/aiperceivable/apcore-a2a), [apcore-cli](https://github.com/aiperceivable/apcore-cli) |
| **Rate Limiting / Circuit Breaker** | Runtime resilience middleware, not module definition | apcore-toolkit (middleware implementations) |
| **Secret Injection / Vault Integration** | Runtime infrastructure, not module standard | apcore-toolkit |
| **Testing Framework** | Developer tooling, not protocol | [apcore-testing](https://github.com/aiperceivable/apcore-testing) |
| **SLA Monitoring / Alerting** | Runtime enforcement (SLA _declaration_ is Core metadata) | apcore-toolkit |
| **Token Counting / Context Window Mgmt** | LLM-specific; apcore stays AI-neutral | Agent frameworks |
| **Concrete Business Modules** | We are a standard, not an application | Upstream projects |
| **LLM Invocation Wrappers** | Stay neutral, don't bind to a specific LLM | LangChain/LlamaIndex/pydantic-ai |
| **Agent Strategies** | Too high-level, belongs to application logic | CrewAI/AutoGen/pydantic-ai |
| **Prompt Templates** | AI orchestration belongs to upper layers | Agent frameworks |
| **Distributed Execution** | Advanced runtime feature | Independent extension projects |
| **UI/CLI Applications** | Focus on core protocol | Upstream projects |
| **Specific Cloud Service Integrations** | Stay neutral | Extension packages |
| **Framework Adapters** (Flask/FastAPI/Django) | Keep core pure, belongs to ecosystem projects | Independent repositories (e.g., django-apcore) |

> apcore only defines the adapter interface specification (see [Adapter Development Guide](./docs/guides/adapter-development.md)); it does not include any framework-specific implementations. The community or official team can develop adapters in independent repositories (e.g., `flask-apcore`, `django-apcore`), built on top of the core `module()` and External Binding mechanisms.

---

## 5. User Story Validation

Use these scenarios to validate whether our boundaries are correct:

### Scenario 1: Python Developer Building an AI Application
```
✅ I defined a database query module and it was automatically discovered by the framework
✅ The framework automatically validates input and output
✅ I can invoke other modules from my code
❌ The framework does not provide LLM invocation (I use the openai library myself)
❌ The framework does not provide an Agent loop (I write my own or use LangGraph)
```

### Scenario 2: Go Developer Wants to Use a Python Module
```
✅ The framework defines a cross-language ID specification
✅ The framework defines a Schema specification
⚡ Cross-language invocation requires RPC (provided by extension packages)
```

### Scenario 3: Team Wants to Define Workflows with YAML
```
✅ The core framework provides standardized module invocation capabilities
❌ Workflow engine is not within apcore's scope (implemented by upstream projects like apflow)
✅ The team can use any workflow solution (apflow, custom-built, etc.)
```

### Scenario 4: Wanting to Expose Modules to Claude/ChatGPT
```
✅ Modules have standard Schema, inherently MCP/OpenAI compatible
❌ MCP Server adapter is not within apcore's scope
✅ Refer to the appendix mapping documentation to implement adapters on your own
```

### Scenario 5: Existing Python Application Wants AI-Perceivable Capabilities
```
✅ Add the @module decorator to existing functions to automatically become apcore modules
✅ Use module(service.method, id=...) to register existing class methods without modifying source code
✅ The framework automatically generates Schema from type annotations
✅ Existing code logic remains completely unchanged
✅ Can also use YAML binding files with absolutely no source code modification
❌ The framework does not automatically scan Flask routes (independent repository ecosystem project)
```

---

## 6. Core vs Extension Decision Table

| Feature | Core? | Reason |
|------|-------|------|
| Module definition/discovery/loading | ✅ Core | Most fundamental capability |
| Schema definition/validation | ✅ Core | Required for AI-Perceivable |
| Canonical ID | ✅ Core | Cross-language foundation |
| ACL permissions | ✅ Core | Security foundation |
| Error handling | ✅ Core | Required |
| Tracing/logging | ✅ Core | Observability foundation |
| Middleware mechanism | ✅ Core | Extension foundation |
| Behavior annotations (readonly, cacheable, paginated, etc.) | ✅ Core | AI-Perceivable behavior hints |
| AI metadata conventions (x-preconditions, x-cost-per-call, etc.) | ✅ Core | Recommended metadata keys for AI planning |
| Deprecation & sunset (deprecated, sunset_date, replacement) | ✅ Core | Module lifecycle metadata |
| `module()` registration (Decorator / function call) | ✅ Core | Least intrusive integration for existing applications |
| External Schema Binding | ✅ Core | Zero code modification integration |
| Type inference Schema generation | ✅ Core | Foundational capability for module() and Binding |
| **MCP Adapter** | ⚡ Ecosystem ([apcore-mcp](https://github.com/aiperceivable/apcore-mcp)) | Protocol adaptation |
| **A2A Adapter** | ⚡ Ecosystem ([apcore-a2a](https://github.com/aiperceivable/apcore-a2a)) | Protocol adaptation |
| **CLI Adapter** | ⚡ Ecosystem ([apcore-cli](https://github.com/aiperceivable/apcore-cli)) | Surface adaptation |
| **Rate Limiting / Circuit Breaker** | ⚡ Ecosystem (apcore-toolkit) | Runtime resilience; implemented as middleware |
| **Secret Injection** | ⚡ Ecosystem (apcore-toolkit) | Runtime infrastructure; `x-sensitive` marking is Core |
| **Testing Framework** | ⚡ Ecosystem ([apcore-testing](https://github.com/aiperceivable/apcore-testing)) | Developer tooling: MockModule, ContractTest, fixtures |
| **OpenAPI / AsyncAPI Export** | ⚡ Ecosystem (apcore-toolkit) | Additional export formats beyond built-in SchemaExporter |
| **SLA Monitoring / Alerting** | ⚡ Ecosystem (apcore-toolkit) | Runtime enforcement; `x-sla` declaration is Core metadata |
| **Dev Server / Hot Reload** | ⚡ Ecosystem (apcore-toolkit) | Developer experience tooling |
| **Workflow Engine** | ⚡ Ecosystem (apflow) | Upper-layer orchestration logic |
| **Distributed Execution** | ⚡ Ecosystem | Advanced runtime feature |
| **Framework Adapters** | ❌ Won't Do (independent repository) | Keep core pure, belongs to ecosystem projects |
| **Web UI** | ❌ Won't Do | Out of scope |
| **Token Counting / Context Window** | ❌ Won't Do | LLM-specific; apcore stays AI-neutral |
| **AI-Assisted Migration Tools** | ⚡ Ecosystem | Developer experience tooling |

---

## 7. Final Confirmation

### Core Protocol Includes:
1. Naming conventions (Directory as ID + ID Map)
2. Directory structure (Directory)
3. Schema specification (Schema)
4. Module specification (Module)
5. Access control configuration (ACL)
6. Approval system (Approval)
7. Error handling (Error)
8. Observability (Observability)
9. Extension mechanism (Extension)
10. Configuration specification (Configuration)
11. Event system (Registry events, lifecycle hooks)
12. Existing application integration (module() + External Binding)

### Appendix Reference (Non-core):
- AI protocol mapping references (MCP, A2A, OpenAI Functions)

---

## 8. Confirmed Decisions

- [x] Workflow: **Won't Do** (application-layer logic, implemented by upstream projects like apflow)
- [x] MCP/A2A/CLI adaptation: **Ecosystem** (apcore-mcp, apcore-a2a, apcore-cli as independent adapter projects)
- [x] Distributed execution: **Won't Do** (runtime feature, not protocol core)
- [ ] Schema and Meta files: Keep separated
- [x] Existing application integration: **Core** (`module()` registration and external Schema binding are core standards)
- [x] Framework adapters: **Won't Do** (independent repositories, apcore only provides adapter interface specification)
- [x] API naming: **No prefix** (rely on language namespaces; languages without namespace mechanisms use `apcore_` prefix)
- [x] Behavior annotations (cacheable, paginated): **Core** (AI-Perceivable behavior hints for agent decision-making)
- [x] AI metadata conventions (x-preconditions, x-cost-per-call, etc.): **Core** (recommended metadata keys, not enforced)
- [x] Rate limiting / circuit breaker: **Ecosystem** (runtime middleware, not module definition)
- [x] Secret injection: **Ecosystem** (runtime infrastructure; `x-sensitive` marking remains Core)
- [x] Testing framework: **Ecosystem** (apcore-testing — MockModule, ContractTest, fixtures)
- [x] Token counting / context window: **Won't Do** (LLM-specific, apcore stays AI-neutral)

## 9. Document Structure

```
apcore/
├── docs/spec/protocol-spec.md          # Core protocol specification (14 chapters)
├── SCOPE.md                  # Scope definition
├── README.md                 # Project introduction
└── docs/                     # Detailed documentation
    ├── concepts.md           # Core concepts
    ├── architecture.md       # Internal architecture
    ├── api/                  # API reference
    ├── features/             # Feature specifications
    ├── guides/               # Usage guides
    └── spec/                 # Standard specification
```
