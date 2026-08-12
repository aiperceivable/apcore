# apcore Roadmap

> **Current baseline (2026-08-12):** protocol `1.9.0` (first non-draft release); core SDKs `0.27.0` for Python, TypeScript, and Rust.

This roadmap prioritizes proof of adoption over expansion of the product matrix. The source of truth for implemented behavior is the [protocol specification](docs/spec/protocol-spec.md) plus released SDK code and conformance tests.

## Path to 1.0

Version 1.0 is gated by evidence, not a calendar date.

- [ ] Normative protocol sections have no unresolved contradictions
- [ ] Python, TypeScript, and Rust pass the shared conformance fixtures
- [ ] The supported public API and compatibility policy are documented per SDK
- [ ] At least three independent adopters are listed in [ADOPTERS.md](ADOPTERS.md)
- [ ] At least one external maintainer participates in specification or SDK decisions
- [ ] Security reporting, release ownership, and support boundaries are exercised in practice

## Now: Truth and Golden Path

### Documentation truth

- [x] Align the protocol frontmatter and roadmap with `1.9.0-draft`
- [x] Publish current core and adapter release lines in the README
- [x] Define apcore as a governed, protocol-neutral capability runtime
- [ ] Audit examples against current SDK APIs on every release
- [ ] Mark experimental and planned features explicitly

### One supported adoption path

- [ ] Publish one end-to-end Python example: existing application function → apcore module → schema validation → ACL → approval → audit evidence
- [ ] Project that same module through `apcore-mcp`
- [ ] Provide copy-paste tests for allowed, denied, approval-required, invalid-input, and successful calls
- [ ] Record setup time, code changes, and failure behavior
- [ ] Mirror the proven path in TypeScript and Rust after the Python path is stable

## Next: Conformance and External Validation

### Cross-language conformance

- [ ] Run all shared fixtures in Python, TypeScript, and Rust CI
- [ ] Publish a machine-readable conformance matrix
- [ ] Block coordinated core releases on behavioral fixture regressions
- [ ] Add compatibility checks for official MCP, A2A, CLI, and toolkit adapters

### Adopter evidence

- [ ] Recruit three external design partners with existing applications
- [ ] Document why each adopter chose apcore instead of a protocol-only integration
- [ ] Publish migration notes, operational limits, and unresolved friction
- [ ] Use adopter findings to decide which APIs are stable enough for 1.0

### Operational readiness

- [ ] Publish support and deprecation policies
- [ ] Publish performance baselines for the execution pipeline
- [ ] Exercise the security-response process
- [ ] Define minimum observability evidence for a supported deployment

## Later: Expansion Gates

Additional languages, adapters, hosted services, marketplaces, orchestration features, and autonomous code-generation systems are not current apcore priorities. A proposal enters the active roadmap only when it has:

1. a named adopter problem;
2. a maintainer who owns delivery and support;
3. a compatibility and conformance plan; and
4. evidence that it strengthens the core adoption path.

Adjacent projects such as `apflow` and `apexe` may experiment independently. They are consumers of apcore, not proof that their feature sets belong in the apcore standard.

## Foundation Readiness

Foundation submission remains an outcome of project health rather than a near-term deliverable.

- [x] Apache 2.0 license and public governance documents
- [x] Security policy and maintainer list
- [x] OpenSSF Best Practices passing badge
- [ ] Independent adopters
- [ ] External maintainers from more than one organization
- [ ] Public conformance results
- [ ] Demonstrated specification review process

## How to Contribute

- **Specification:** open an issue with the `spec` label and cite the affected normative section
- **SDK:** work in the language repository and include conformance coverage
- **Adoption:** contribute a reproducible integration example or adopter report
- **Documentation:** correct the source document; do not patch generated site output

Priorities are set from conformance failures, adopter evidence, security risk, and maintainer capacity—in that order.
