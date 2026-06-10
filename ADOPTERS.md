# apcore Adopters

This page lists projects and organizations that build on the apcore standard. It exists to
help newcomers gauge real-world usage and to recognize the people relying on the project.

If you use apcore, **please add yourself** — open a pull request editing this file (see the
[template](#add-yourself) at the bottom). Listing is voluntary and can be removed at any time.

> Honesty policy: this list distinguishes **first-party** projects (built by the apcore
> maintainers) from **independent** adopters. We do not list organizations without their
> consent, and we do not inflate the independent section with first-party work.

---

## First-party ecosystem projects

These are maintained by the apcore team and depend on the standard. They demonstrate the
protocol end-to-end but are **not** independent adoption.

| Project | What it is | Uses apcore for |
|---------|------------|-----------------|
| [apexe](https://github.com/aiperceivable/apexe) | CLI-to-agent bridge (Rust) | Scans CLI tools into governed apcore modules and serves them with ACL + audit. |
| [apflow](https://github.com/aiperceivable/apflow) | Distributed task orchestration | Runs orchestrated steps as apcore modules with schema validation and tracing. |
| [aphub](https://github.com/aiperceivable/aphub) | Module registry & governance platform | Indexes and governs apcore modules across an organization. |
| Surface adapters | `apcore-mcp`, `apcore-a2a`, `apcore-cli`, and the framework adapters (`flask`/`fastapi`/`django`/`nestjs`/`axum`) | Project a single module definition onto each delivery protocol. |

---

## Organizations using apcore

> This section is intentionally empty until independent adopters opt in. If your team uses
> apcore in a product, internal tool, or research project, you are the first — and we would
> love to feature you.

_No independent adopters listed yet. Be the first by opening a PR._

---

## Add yourself

Copy this row into the appropriate table and open a pull request:

```markdown
| [Your Org / Project](https://example.com) | <one line: what you build> | <how you use apcore — e.g. "MCP tools with ACL + approval gates"> |
```

Optional fields you may include: the apcore SDK(s) you use (Python / TypeScript / Rust),
which surface adapters you rely on, and whether you are in development or production.

For guidance on contributing, see [CONTRIBUTING.md](CONTRIBUTING.md). For the project's
direction, see [ROADMAP.md](ROADMAP.md).
