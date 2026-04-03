---
paths:
  - "docs/**/*.md"
  - "README.md"
---

# Documentation Writing Rules

## Structure
- `#` for document title (one per file)
- `##` for major sections
- `###` for subsections
- Keep heading hierarchy sequential — never skip from `##` to `####`

## MkDocs features available
- Admonitions: `!!! note`, `!!! warning`, `!!! danger`, `!!! tip`, `!!! info`
- Collapsible: `??? note "Title"` (collapsed by default)
- Tabbed content: `=== "Tab Name"` (use for cross-language examples)
- Code blocks: specify language, use ```` ```python ````, never bare ```` ``` ````
- Code copy button is enabled globally

## Cross-language examples
When documenting a feature, provide examples for all three languages using tabs:
```
=== "Python"
    ```python
    # Complete, runnable example with imports
    ```
=== "TypeScript"
    ```typescript
    // Complete, runnable example with imports
    ```
=== "Rust"
    ```rust
    // Complete, runnable example with use statements
    ```
```

## Links
- Use relative paths: `[text](./path.md)` or `[text](../other/path.md)`
- Anchor links: `[text](./file.md#section-name)`
- Never use absolute URLs for internal docs
- Run `mkdocs build` to verify links before committing

## When adding a new page
1. Create the file in the correct subdirectory
2. Add to `mkdocs.yml` nav section in the right position
3. Add to `README.md` Documentation Index if the page is user-facing
4. Verify build: `mkdocs build` with no warnings

## Content rules
- `description` field (≤ 200 chars, plain text) is always required in module examples
- `documentation` field (≤ 5000 chars, Markdown allowed) is optional but recommended
- Code examples must be complete and runnable — never truncate
- Do not add user-facing content to `planning/` directory
