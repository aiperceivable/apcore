# CLAUDE.md — apcore-rust Development & Code Quality Specification

## Project Overview

`apcore` is a **schema-driven module standard for AI-perceivable interfaces** — a task orchestration and execution specification framework that supports multiple task types via a unified, schema-first API.

---

## Core Principles

- Prioritize **simplicity, readability, and maintainability** above all.
- Avoid premature abstraction, optimization, or over-engineering.
- Code should be understandable in ≤10 seconds; favor straightforward over clever.
- Always follow: **Understand → Plan → Implement minimally → Test/Validate → Commit**.

---

## Rust Code Quality

### Readability

- Use precise, full-word names; standard abbreviations only when idiomatic (`buf`, `cfg`, `ctx`).
- Functions ≤50 lines, single responsibility, verb-named (`parse_request`, `build_schema`).
- Avoid obscure tricks, overly chained iterators, unnecessary macros, or excessive generics.
- Break complex logic into small, well-named helper functions.

### Types (Mandatory)

- Provide explicit types on all public items; do not rely on inference for public API surfaces.
- Prefer `struct` over raw tuples for anything with more than 2 fields.
- Use **`newtype`** wrappers (`struct TaskId(Uuid);`) to encode domain semantics.
- Use **`enum`** for exhaustive variants; avoid stringly-typed logic.
- Implement `serde::Serialize` / `serde::Deserialize` and `schemars::JsonSchema` on all public data types.

### Design

- Favor **composition over inheritance**; use `trait` only for true behavioral interfaces.
- Prefer plain functions + data structs; minimize trait object (`dyn Trait`) indirection.
- No circular module dependencies.
- Use **dependency injection** (constructor arguments) for config, logging, DB connections, etc.
- Keep `pub` surface minimal — default to module-private, expose only what consumers need.

### Errors & Resources

- Define domain errors with **`thiserror`**; no bare `Box<dyn Error>` in library code.
- Propagate errors with `?`; no `unwrap()` / `expect()` in library paths (tests excepted).
- If `unwrap()` / `expect()` is truly unreachable, document with a `// SAFETY:` or `// INVARIANT:` comment.
- Validate and sanitize all public inputs at crate boundaries.
- Use RAII / `Drop` for resource cleanup; avoid manual teardown.

### Async

- Runtime: **Tokio** (`features = ["full"]`).
- Traits with async methods: use **`async-trait`**.
- Never block the async executor — use `tokio::task::spawn_blocking` for CPU-heavy work.
- Use `tokio::time::sleep` / `tokio::time::timeout`, not `std::thread::sleep`.

### Logging

- Use **`tracing`** — no `println!` / `eprintln!` in production code.
- Level guide:
  - `tracing::error!` — unrecoverable failures
  - `tracing::warn!`  — recoverable anomalies
  - `tracing::info!`  — key business events
  - `tracing::debug!` — internal state for debugging
- Always include structured fields: `tracing::info!(task_id = %id, "task started")`.

### Testing

- Run with: `cargo test --all-features`
- **Unit tests**: in the same file under `#[cfg(test)] mod tests { ... }`.
- **Integration tests**: in `tests/` directory.
- **Examples**: in `examples/`; must build cleanly via `cargo build --examples`.
- Test names: `test_<unit>_<behavior>` (e.g., `test_parse_request_returns_error_on_empty_body`).
- Never change production code without adding or updating corresponding tests.
- Use **`tokio-test`** for async test helpers.

### Serialization

- JSON: `serde_json`. YAML: `serde_yaml`.
- All public schemas must derive `schemars::JsonSchema`.
- Avoid manual `Serialize` / `Deserialize` impls unless schema shape requires it.

---

## Mandatory Quality Gates (CI — all must pass)

Run `make check` before every commit (mirrors CI exactly):

| Command | Purpose |
|---------|---------|
| `cargo fmt --all -- --check` | Formatting |
| `cargo clippy --all-targets --all-features -- -D warnings` | Lint (warnings = errors) |
| `apdev-rs check-chars src/` | Character validation |
| `cargo build --all-features` | Full build |
| `cargo test --all-features` | Tests |
| `cargo build --examples` | Example build |

```bash
make setup   # One-time: installs apdev-rs + pre-commit hook
make fmt     # Auto-format
make check   # Run all gates (do this before git commit)
```

### Clippy & Formatting Rules

- `#[allow(...)]` to silence Clippy requires an inline comment explaining why.
- `#[rustfmt::skip]` is forbidden without documented justification.
- `Cargo.lock` **must be committed** (reproducible CI builds).

### CI Security

- All `uses:` actions must be pinned to a **full commit SHA** (not just a tag):
  ```yaml
  uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
  ```
- Workflow default permissions: `contents: read` (minimum privilege).

---

## Dependency Management

- Evaluate necessity before adding a new dependency.
- Specify minimum compatible versions; avoid over-pinning patch versions.
- Dev-only crates go in `[dev-dependencies]`, never `[dependencies]`.

---

## General Guidelines

- **English only** for all code, comments, doc comments, error messages, and commit messages.
- Fully understand surrounding code before making changes.
- Do not generate unnecessary documentation stubs, example files, or boilerplate unless explicitly requested.
- No secrets hardcoded — use environment variables or configuration files.
