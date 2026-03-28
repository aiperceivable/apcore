# Contributing to apcore

Thank you for your interest in contributing to apcore!

## Ways to Contribute

- **Report bugs**: Open an issue describing the problem
- **Propose features**: Open an issue with the `enhancement` label
- **Submit code**: Fork, branch, and open a pull request
- **Improve docs**: Fix typos, add examples, clarify language
- **Review PRs**: Help review open pull requests

## Protocol Specification

The specification lives in [PROTOCOL_SPEC.md](PROTOCOL_SPEC.md). Changes to
the spec require an issue discussing the change before a PR is opened.
Spec changes must be reviewed by at least 2 maintainers.

## Language SDKs

Each SDK has its own repository with language-specific setup instructions:

| SDK | Repository | Setup |
|-----|-----------|-------|
| Python | [apcore-python](https://github.com/aiperceivable/apcore-python) | Python 3.11+, `pip install -e .` |
| TypeScript | [apcore-typescript](https://github.com/aiperceivable/apcore-typescript) | Node 18+, `npm install` |
| Rust | [apcore-rust](https://github.com/aiperceivable/apcore-rust) | Rust 1.75+, `cargo build` |

## Pull Request Process

1. Fork the repository and create a branch from `main`
2. Make your changes with clear commit messages
3. Add or update tests as appropriate
4. Ensure all tests pass
5. Open a PR with a description of the changes and why they are needed
6. Wait for review — maintainers aim to respond within 7 days

## Developer Certificate of Origin (DCO)

All contributions must be signed off with DCO:

```
git commit -s -m "Description of change"
```

This certifies that you have the right to submit the contribution under
the project's license. See [developercertificate.org](https://developercertificate.org/).

## Coding Guidelines

- Follow the existing code style in each repository
- Write tests for new functionality
- Keep PRs focused — one logical change per PR
- Update documentation when changing user-facing behavior

## Cross-Language Consistency

When making changes that affect the protocol or module behavior, consider
the impact across all three language implementations. Ideally, changes to
the spec should be accompanied by PRs to all affected SDK repositories.

## Reporting Security Issues

Please see [SECURITY.md](SECURITY.md) for instructions on reporting
security vulnerabilities. **Do not open public issues for security bugs.**

## License

By contributing, you agree that your contributions will be licensed under
the [Apache License 2.0](LICENSE).
