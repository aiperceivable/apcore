# High-Quality Code Specification – Simplicity, Readability, and Maintainability First

## Project Overview
The core of `apcore` is **task orchestration and execution specifications**. It provides a unified task orchestration framework that supports execution of multiple task types.

## Core Principles
- Prioritize **simplicity, readability, and maintainability** above all.
- Avoid premature abstraction, optimization, or over-engineering.
- Code should be understandable in ≤10 seconds; favor straightforward over clever.
- Always follow: Understand → Plan → Implement minimally → Test/Validate → Commit.

## Python Code Quality

### Readability
- Use precise, full-word names (standard abbreviations only when conventional).
- Functions ≤50 lines, single responsibility, verb-named.
- Avoid obscure tricks, heavy comprehensions, excessive *args/**kwargs, unnecessary decorators.
- Break complex logic into small, well-named helpers.

### Types (Mandatory)
- Full type annotations everywhere.
- Avoid `Any` except for dynamic/external data.
- Prefer `dataclass`, `TypedDict`, `Protocol`, `NewType`.

### Design
- Favor functional style + data classes; minimize inheritance.
- Composition > inheritance; use Protocols/ABCs only for true interfaces.
- No circular imports.
- Dependency injection for config, logging, DB, etc.

### Errors & Resources
- Explicit exception handling; no bare `except:`.
- Context managers for files/DB/connections.
- Validate/sanitize all public inputs.

### Logging
- Use `logging.info` for key paths.
- `logging.error` + context for exceptions.
- No `print()` in production/debug code.

### Testing
- Unit tests in `tests/`, ≥90% coverage on core logic.
- Name: `test_<unit>_<behavior>`.
- Never change prod code without/updating tests.

### Formatting & Linting
- After changes, always run:
  - ruff check --fix .
  - black .
  - pyright .
- Zero errors/warnings before commit.

### Security & Performance
- Never hardcode secrets; use env/config.
- Validate/sanitize inputs.
- Avoid unjustified quadratic+ complexity in hot paths.

## General Guidelines
- English ONLY for comments, docstrings, logs, errors, commit messages.
- Fully understand surrounding code before changes.
- Do not generate unnecessary documentation, examples, stubs, or bloated `__init__.py` files unless explicitly requested.