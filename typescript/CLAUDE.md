# High-Quality Code Specification – Simplicity, Readability, and Maintainability First

## Project Overview
The core of `apcore` is **task orchestration and execution specifications**. It provides a unified task orchestration framework that supports execution of multiple task types.

## Core Principles
- Prioritize **simplicity, readability, and maintainability** above all.
- Avoid premature abstraction, optimization, or over-engineering.
- Code should be understandable in ≤10 seconds; favor straightforward over clever.
- Always follow: Understand → Plan → Implement minimally → Test/Validate → Commit.

## TypeScript Code Quality

### Readability
- Use precise, full-word names (standard abbreviations only when conventional).
- Functions ≤50 lines, single responsibility, verb-named.
- Avoid obscure tricks, excessive generics, or unnecessary abstraction layers.
- Break complex logic into small, well-named helpers.

### Types (Mandatory)
- Full type annotations on all public APIs and function signatures.
- Avoid `any` except for dynamic/external data; prefer `unknown` with narrowing.
- Prefer `interface` for object shapes, `type` for unions/intersections.
- Use `readonly` for immutable data structures.

### Design
- Favor functional style + plain objects; minimize class inheritance.
- Composition > inheritance; use `interface` only for true contracts.
- No circular imports.
- Dependency injection for config, logging, external services, etc.

### Errors & Resources
- Explicit error handling; no bare `catch {}` that swallows errors silently.
- Use `try/finally` or equivalent cleanup patterns for resources.
- Validate/sanitize all public inputs.

### Logging
- Use `console.warn` for warnings in library code.
- Prefix log messages with `[apcore:<subsystem>]` for traceability.
- No `console.log` in production code paths.

### Testing
- Unit tests in `tests/`, ≥90% coverage on core logic.
- Test files named: `test-<unit>.test.ts`.
- Test cases named descriptively: `it('throws X when Y', ...)`.
- Never change production code without updating tests.
- Use `vitest` as test runner.

### Build & Checks
- After changes, always run:
  - `npx tsc --noEmit` (type checking)
  - `npx vitest run` (tests)
- Zero errors before commit.

### Module System
- ESM only (`"type": "module"` in package.json).
- All imports must use `.js` extension (NodeNext resolution).
- Use `@sinclair/typebox` for runtime schema definitions.

### Security & Performance
- Never hardcode secrets; use env/config.
- Validate/sanitize inputs at system boundaries.
- Avoid unjustified quadratic+ complexity in hot paths.

## General Guidelines
- English ONLY for comments, JSDoc, logs, errors, commit messages.
- Fully understand surrounding code before changes.
- Do not generate unnecessary documentation, examples, or stubs unless explicitly requested.
