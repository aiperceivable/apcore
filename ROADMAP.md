# apcore Roadmap

> **Last Updated**: March 2026
> **Status**: Living document — updated quarterly

This roadmap outlines the planned development for the apcore protocol specification and its ecosystem. Community input is welcome — open an [issue](https://github.com/aiperceivable/apcore/issues) to discuss priorities.

---

## Current Status (v1.6.0-draft)

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

**Governance & Compliance**
- [x] Apache 2.0 License
- [x] GOVERNANCE.md, CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md, MAINTAINERS.md
- [x] OpenSSF Best Practices Badge (Passing)
- [x] DCO enforcement via GitHub App

---

## 2026 Q2: Ecosystem Maturity

### Specification
- [ ] Finalize v1.6.0 specification
- [ ] Cross-language conformance test suite
- [ ] Formal specification review process

### Tooling
- [ ] **apexe** — CLI-to-Agent bridge for infrastructure tools (kubectl, docker, gh)
- [ ] **apdev** — Architecture guardian with multi-language dependency analysis
  - Tree-sitter based code analysis
  - MCP server integration for IDE support

### Community & Standards
- [ ] Foundation submission preparation (AAIF / CNCF)
- [ ] Positioning document ([docs/POSITIONING.md](docs/POSITIONING.md))
- [ ] "Good first issue" labeling for contributor onboarding
- [ ] Community discussion channels

---

## 2026 Q3: Foundation & Adoption

### Standards Body Engagement
- [ ] Formal foundation submission (AAIF primary, CNCF secondary)
- [ ] CNCF TAG App Delivery engagement
- [ ] Industry partnership outreach

### Technical Expansion
- [ ] Additional framework adapters (Spring Boot, Express, Actix)
- [ ] Enhanced security scanning for modules
- [ ] Plugin system for custom extensions
- [ ] Event engine for audit trails

### Developer Experience
- [ ] Interactive module playground
- [ ] Getting-started tutorials per language
- [ ] Video walkthroughs and demos

---

## 2026 Q4+: Industry Standard

### Ecosystem Growth
- [ ] 3+ independent adopters (Incubation requirement)
- [ ] External maintainers from different organizations
- [ ] OpenSSF Badge Silver/Gold progression
- [ ] Conference talks and workshops

### Advanced Features
- [ ] Module versioning and compatibility system
- [ ] Distributed module discovery
- [ ] Advanced observability (OpenTelemetry deep integration)
- [ ] Performance benchmarks across languages

### Broader Ecosystem
- [ ] **aphub** — AI Agent registry and governance platform
- [ ] **apflow** — Distributed orchestration engine
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
