# CLAUDE.md — apcore Protocol Specification Repository

## Project Context

**apcore** is a language-agnostic **protocol specification** — not a runtime library.
This repo contains specs, docs, JSON Schemas, and conformance fixtures. The SDKs live in separate repos.

| Repo | Role |
|---|---|
| `apcore` (here) | Protocol spec, docs, schemas, conformance tests |
| `apcore-python` / `apcore-typescript` / `apcore-rust` | Language SDK implementations |

Tech stack: **MkDocs Material** + GitHub Pages + JSON Schema Draft 2020-12.

## Directory Conventions

```
apcore/
├── docs/spec/protocol-spec.md          # Single source of truth (RFC 2119)
├── schemas/                   # Canonical JSON Schema files (*.schema.json)
├── conformance/fixtures/      # Cross-language test fixtures (*.json)
├── docs/
│   ├── features/              # One reference page per runtime subsystem (current behaviour, 3-language examples)
│   ├── guides/                # How-to tutorials and cookbooks for SDK users
│   └── spec/                  # Normative spec, conformance, type mapping, algorithms, decision records
├── planning/                  # Internal only — implementation task tracking
│   └── <feature>/             # index.md + plan.md + state.json + tasks/NN-*.md
└── mkdocs.yml                 # Doc site navigation and extensions
```

**Where to put new files:**
- New feature spec → `docs/features/<feature-name>.md`
- New user guide → `docs/guides/<topic>.md`
- New conformance fixture → `conformance/fixtures/<feature>.json`
- New JSON schema → `schemas/<name>.schema.json`

**Naming:** Markdown files use `kebab-case.md`. JSON fixtures use `snake_case.json`.

## Commands

```bash
mkdocs serve                # Local preview at http://127.0.0.1:8000
mkdocs build                # Build static site to site/
git commit -s               # DCO sign-off (auto-added by .githooks/prepare-commit-msg)
```

Deploy: push to `main` → GitHub Actions builds and deploys to GitHub Pages automatically.

## Checklists

### Adding a new documentation page
1. Create the `.md` file in the correct `docs/` subdirectory
2. Add entry to `mkdocs.yml` `nav:` section
3. Add it to its section index page (`docs/features/index.md`, `docs/guides/index.md` or `docs/spec/index.md`). README links only to the section indexes — do not list individual pages there
4. Verify internal links and anchors resolve: `mkdocs build --strict` (anchor validation is enabled) must produce no warnings

### Adding a conformance fixture
1. Create `conformance/fixtures/<name>.json` with structure: `{ "description": "...", "test_cases": [...] }`
2. Each test case needs: `id`, input fields, `expected` result
3. Use `caller_id` / `target_id` terminology (not `caller` / `target`)
4. Add row to `conformance/README.md` fixtures table

### Modifying a JSON Schema
1. Edit file in `schemas/`
2. Must use JSON Schema Draft 2020-12 (`$schema` field)
3. Every `property` must have a `description`
4. Check all `$ref` references still resolve

### Updating docs/spec/protocol-spec.md
1. **Maintainer approval** — 2 maintainers, **or all maintainers when fewer than 3 exist** (`GOVERNANCE.md` § Decision Making). `MAINTAINERS.md` currently lists one, so one approval satisfies it. This is the only thing GOVERNANCE.md requires for a spec change.
2. **A linked issue, if a PR is being opened.** That requirement is `CONTRIBUTING.md` § Protocol Specification — "changes to the spec require an issue discussing the change **before a PR is opened**" — not GOVERNANCE.md, and it is scoped to the PR. Its purpose is that a spec change is aired before a reviewer meets a fait accompli; an issue opened *after* the change is written serves none of it. Do not treat it as blocking work that is not going through a PR, and do not cite GOVERNANCE.md for it.
3. Do NOT remove/weaken a `MUST`/`MUST NOT` without deprecation notice + version bump
4. Do NOT delete or rename anchor IDs — external SDKs link to them
5. Update `CHANGELOG.md` with the change, **and add the version-history row in `docs/spec/protocol-spec.md`** — the header version, the history table and the CHANGELOG are three places one bump has to reach, and three bumps once landed with only the header updated.

## Writing Rules

### RFC 2119 Keywords
In `docs/spec/protocol-spec.md` and `docs/spec/`: use uppercase `MUST`, `MUST NOT`, `SHOULD`, `SHOULD NOT`, `MAY` for normative statements. Never use lowercase "should"/"must" for normative intent.

### Cross-Language Examples
All feature docs must show Python, TypeScript, and Rust examples using MkDocs tabbed sections (`=== "Python"` / `=== "TypeScript"` / `=== "Rust"`). See `.claude/rules/documentation.md` for the full template.

### Field Limits
| Field | Max | Format |
|---|---|---|
| `description` | 200 chars | Plain text, no Markdown. Always required. |
| `documentation` | 5000 chars | Markdown allowed. Optional. |

### Code Examples
- Must be syntactically correct and complete (imports, initialization, invocation)
- Never truncate for brevity — incomplete examples cause integration errors
- Specify language in fenced code blocks (` ```python `, ` ```yaml `, etc.)

### Terminology
Use standardized terms consistently:
- `caller_id` / `target_id` (not `caller` / `target` alone) **in prose**. Never rename a literal schema field to match: a binding entry's field is `target`, ACL rules use `callers` / `targets` — a `target_id:` key in a binding example fails validation
- `module` (not `extension` when referring to the abstract concept)
- `default_effect: deny` (always — never show `allow` as default without a warning)

## Critical Rules

### Specification Integrity
- Do NOT modify `docs/spec/protocol-spec.md` without maintainer approval (2 maintainers, or all of them when fewer than 3 exist — `GOVERNANCE.md` § Decision Making). A linked issue is a `CONTRIBUTING.md` requirement scoped to opening a PR, not a GOVERNANCE one; see the checklist above.
- Do NOT add normative requirements without bumping the relevant version number
- Do NOT contradict existing normative statements — search for conflicts first

### Cross-Language Consistency
- Do NOT document behavior changes for one SDK without noting cross-language impact
- Do NOT add Python-specific idioms to language-agnostic spec sections
- Do NOT assume SDK behavior matches without verifying the relevant SDK repo

### ACL & Security
- Do NOT change `default_effect` from `deny` to `allow` without a prominent warning
- Do NOT show ACL file examples that skip the `audit:` block — the ACL file is the one audit configuration home (spec §6.3.2); `acl.audit.*` and `acl.default_effect` in `apcore.yaml` are deprecated and do nothing
- Do NOT show a config key as working without checking `conformance/config_key_consumers.json` — 16 declared keys are inert
- Do NOT bypass `requires_approval` enforcement at the Executor level

### Architecture
- Do NOT suggest `api.*` calling `executor.*` directly — must go through `orchestrator.*`
- Do NOT store sensitive data in `context.data` without noting `x-sensitive` redaction
- Do NOT conflate apcore (module standard) with MCP (communication protocol)

### Documentation
- Do NOT add user-facing docs to `planning/` — internal only
- Do NOT create top-level `.md` files without linking them from `README.md`
- Do NOT use Schema drafts other than Draft 2020-12 in examples
- Do NOT write history into current-state docs (guides, features, concepts, README). Describe what the product does now; a rule may cite its decision as `(D-xx)`. Backstory — "previously", "SDK X used to", version-tagged notes, audit narratives — belongs only in `CHANGELOG.md` and the decision records under `docs/spec/`
- Do NOT write versions anywhere but `docs/index.md` (and the spec header / CHANGELOG)

## Changelog Format

Follow existing `CHANGELOG.md` entries. Sections per release: `### Added`, `### Changed`, `### Fixed`, `### Deprecated`, `### Removed`. Semantic versioning: `MAJOR.MINOR.PATCH`. Spec-breaking changes increment MAJOR.

## Commit Messages

```
<type>(<scope>): <short imperative summary>
```
Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`. DCO sign-off is required (automated by git hook).

## Other Key Documents

- [`CONTRIBUTING.md`](./CONTRIBUTING.md) — PR and commit process
- [`SECURITY.md`](./SECURITY.md) — Vulnerability reporting (never open public issues for security bugs)

## Key Architecture Concepts (Quick Reference)

These are the five pillars — read the linked docs before modifying related content:

1. **Module ID = Directory Path** — `executor.email.send_email` derived from file path. See `docs/spec/protocol-spec.md` §"Module ID Specification"
2. **Three-Layer Metadata** — Core (required: `input_schema`/`output_schema`/`description`) → Annotation → Extension (`x-` prefix). See `docs/features/schema-system.md`
3. **Execution Pipeline** — 11 ordered steps (`middleware_before` runs before `input_validation`). See `docs/features/core-executor.md` and `docs/features/execution-pipeline.md`
4. **ACL Default-Deny** — All inter-module calls require explicit `allow`. See `docs/features/acl-system.md`
5. **Layer Hierarchy** — `api.*` → `orchestrator.*` → `executor.*` → `common.*`. Downward only.
