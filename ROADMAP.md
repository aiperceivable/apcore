# apcore Roadmap

> **Last Updated**: June 2026
> **Status**: Living document — updated quarterly

This roadmap outlines the planned development for the apcore protocol specification and its ecosystem. Community input is welcome — open an [issue](https://github.com/aiperceivable/apcore/issues) to discuss priorities.

---

## Path to 1.0

apcore versions two artifacts independently, and "1.0" means something specific for each:

- **Protocol specification** — currently **v1.8.0-draft**. Reaches **1.0 (stable)** when its
  normative surface is frozen, a conformance suite covers every normative requirement, and
  multiple independently-verified implementations pass it.
- **Language SDKs** (`apcore-python` / `apcore-typescript` / `apcore-rust`) — currently
  **v0.23.0**. Each reaches **1.0** when its public API is stable, it passes the full
  cross-language conformance suite, and it has shipped at least one minor cycle with no
  breaking changes.

**Gates for spec 1.0**

- [ ] Normative surface frozen — no open `MUST` / `MUST NOT` changes
- [ ] Conformance suite covers every normative requirement, by [conformance level](docs/spec/conformance.md)
- [ ] At least three conforming implementations, with independent verification
- [ ] Threat model documented (what ACL/approval do and do **not** guarantee)

**Gates for SDK 1.0**

- [ ] Public API frozen and documented
- [ ] 100% of conformance fixtures passing in CI
- [ ] One breaking-change-free minor cycle
- [ ] Type-export parity across languages (e.g. `py.typed`, published `.d.ts`, `docs.rs`)

> 1.0 is gated on readiness, not a date. The quarterly themes below track the work that
> unblocks these gates.

---

## Current Status (v1.8.0-draft · SDKs v0.23.0)

### Completed

**Protocol Specification**
- [x] Complete PROTOCOL_SPEC.md (RFC 2119 conformant)
- [x] Module lifecycle (11-step pipeline)
- [x] Schema system (three-layer metadata: module, action, LLM extension)
- [x] Behavioral annotations (`readonly`, `destructive`, `requires_approval`, `idempotent`, `open_world`)
- [x] ACL access control (pattern-based, role-aware)
- [x] Context object and trace propagation
- [x] Middleware system
- [x] Configuration management
- [x] Observability (tracing, metrics, structured logging)
- [x] Error handling with `ai_guidance` field

**Language SDKs**
- [x] [apcore-python](https://github.com/aiperceivable/apcore-python) — Python 3.11+
- [x] [apcore-typescript](https://github.com/aiperceivable/apcore-typescript) — Node 18+
- [x] [apcore-rust](https://github.com/aiperceivable/apcore-rust) — Rust 1.75+

**Toolkit (Schema Transformation)**
- [x] [apcore-toolkit-python](https://github.com/aiperceivable/apcore-toolkit-python)
- [x] [apcore-toolkit-typescript](https://github.com/aiperceivable/apcore-toolkit-typescript)
- [x] [apcore-toolkit-rust](https://github.com/aiperceivable/apcore-toolkit-rust)

**MCP Bridge**
- [x] [apcore-mcp-python](https://github.com/aiperceivable/apcore-mcp-python)
- [x] [apcore-mcp-typescript](https://github.com/aiperceivable/apcore-mcp-typescript)
- [x] [apcore-mcp-rust](https://github.com/aiperceivable/apcore-mcp-rust)

**A2A Bridge**
- [x] [apcore-a2a-python](https://github.com/aiperceivable/apcore-a2a-python)
- [x] [apcore-a2a-typescript](https://github.com/aiperceivable/apcore-a2a-typescript)

**CLI Adapter**
- [x] [apcore-cli-python](https://github.com/aiperceivable/apcore-cli-python)
- [x] [apcore-cli-typescript](https://github.com/aiperceivable/apcore-cli-typescript)
- [x] [apcore-cli-rust](https://github.com/aiperceivable/apcore-cli-rust)

**Framework Adapters**
- [x] [django-apcore](https://github.com/aiperceivable/django-apcore)
- [x] [flask-apcore](https://github.com/aiperceivable/flask-apcore)
- [x] [fastapi-apcore](https://github.com/aiperceivable/fastapi-apcore)
- [x] [nestjs-apcore](https://github.com/aiperceivable/nestjs-apcore)
- [x] [axum-apcore](https://github.com/aiperceivable/axum-apcore)
- [x] [tiptap-apcore](https://github.com/aiperceivable/tiptap-apcore)

**Core Product: apexe (CLI-to-Agent Bridge)**
- [x] [apexe](https://github.com/aiperceivable/apexe) — v0.2.0, Rust, 9.1K LOC
  - Scan any CLI tool into governed apcore modules (`apexe scan <tool>`)
  - Serve as MCP tools with ACL, audit logging, and shell injection prevention (`apexe serve`)
  - 3-tier scanning engine (--help → man pages → shell completions)
  - Auto-inferred behavioral annotations (readonly/destructive/requires_approval)
  - Explorer UI for interactive debugging
  - Single-binary distribution, zero dependencies

**Governance & Compliance**
- [x] Apache 2.0 License
- [x] GOVERNANCE.md, CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md, MAINTAINERS.md
- [x] OpenSSF Best Practices Badge (Passing)
- [x] DCO enforcement via GitHub App

---

## 2026 Q2: Specification Finalization & Foundation Readiness

### Specification
- [ ] Stabilize the v1.8.0-draft specification toward the spec 1.0 gates above
- [ ] Cross-language conformance test suite (Python, TypeScript, Rust)
- [ ] Formal specification review process

### Foundation readiness (AAIF primary, CNCF secondary)

> Strategy: build credibility first, then submit. A premature submission risks a hard-to-reverse
> first impression with the AAIF Technical Committee, so submission is gated on the readiness
> signals below — not on a fixed date.

- [x] Positioning document ([docs/POSITIONING.md](docs/POSITIONING.md))
- [ ] Governance in place: GOVERNANCE.md (role ladder), MAINTAINERS.md, CODE_OF_CONDUCT.md, SECURITY.md, ADOPTERS.md
- [ ] Engage the AAIF community as a contributor (showcase demos, conference CFPs, upstream MCP input)
- [ ] Reach the incubation signals below (independent adopters + external maintainers)
- [ ] Submit the project proposal once those signals are met ([aaif/project-proposals](https://github.com/aaif/project-proposals))

### Community Building
- [ ] "Good first issue" labeling for contributor onboarding
- [ ] Community discussion channels
- [ ] Document internal dogfooding results as GitHub issues
- [ ] External early adopter outreach

---

## 2026 Q3: Hardening & Ecosystem Validation

> *Scope adjusts based on team recruitment and foundation progress.*

### SDK Quality & Testing
- [ ] Expand test coverage across Python, TypeScript, and Rust SDKs
- [ ] Cross-language behavioral conformance test suite
- [ ] CI/CD pipeline hardening (automated release, regression tests)
- [ ] Performance benchmarks across languages

### Ecosystem Validation
- [ ] End-to-end integration tests: apcore → MCP → A2A → CLI full chain
- [ ] Framework adapter conformance verification (all 6 adapters)
- [ ] apexe integration validation with real-world CLI tools
- [ ] Continued pilot feedback integration

### Developer Experience
- [ ] Getting-started tutorials per language
- [ ] Interactive module playground
- [ ] Video walkthroughs and demos

### apexe Evolution
- [ ] A2A protocol support (expose scanned tools as agent skills)
- [ ] Package registry publishing (crates.io, Homebrew)

---

## 2026 Q4+: Expansion

> *Scope and pace depend on community growth and contributor recruitment.*

### Ecosystem Growth
- [ ] 3+ independent adopters (Incubation requirement)
- [ ] External maintainers from different organizations
- [ ] OpenSSF Badge Silver/Gold progression
- [ ] Conference talks and workshops

### Additional Language SDKs
- [ ] **Go SDK** — apcore-go + toolkit + MCP/A2A/CLI bridges + Gin/Echo adapters
- [ ] **Java/Kotlin SDK** — apcore-java + toolkit + MCP/A2A/CLI bridges + Spring Boot adapter
- [ ] **Swift SDK** — apcore-swift + toolkit + MCP/A2A/CLI bridges + Vapor adapter
- [ ] **C# SDK** — apcore-dotnet + toolkit + MCP/A2A/CLI bridges + ASP.NET Core adapter

### Advanced Features
- [ ] Module versioning and compatibility system
- [ ] Advanced observability (OpenTelemetry deep integration)
- [ ] Distributed module discovery
- [ ] Enhanced security scanning for modules
- [ ] Plugin system for custom extensions
- [ ] Event engine for audit trails
- [ ] **apdev** — Architecture guardian with multi-language dependency analysis

### Broader Ecosystem (Built on apcore)
- [ ] **aphub** — AI Agent registry and governance platform
- [ ] **apflow** — AI Agent production middleware (durable execution, cost governance)
- [ ] **apevo** — Autonomous error detection, diagnosis, and repair

---

## How to Contribute

We welcome contributions at every level:

- **Spec discussion**: Open an [issue](https://github.com/aiperceivable/apcore/issues) with the `spec` label
- **SDK improvements**: Pick up issues in language-specific repos
- **Framework adapters**: Port existing adapters or create new ones
- **Documentation**: Improve guides, add examples, fix typos
- **Testing**: Expand conformance tests across languages

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

---

## Prioritization

Roadmap priorities are set by maintainers based on:
1. Foundation submission requirements
2. Community demand (GitHub issues, discussions)
3. Cross-language consistency needs
4. Security and stability improvements

To influence priorities, open an issue or join the discussion on existing ones.
