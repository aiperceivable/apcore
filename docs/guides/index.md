# User Guides

> Practical guides for apcore, from beginner to advanced.

This directory contains 8 practical guides covering the complete module development workflow. Documents are arranged by learning path; it is recommended to read the first four in order, then selectively read the rest as needed.

## Document List

| Document | Description |
|----------|-------------|
| [creating-modules.md](./creating-modules.md) | Create modules from scratch, introducing multiple module definition approaches |
| [schema-definition.md](./schema-definition.md) | Detailed Schema definition, mastering how to declare module input/output structures |
| [middleware.md](./middleware.md) | Middleware development, using middleware to extend pre/post module execution logic |
| [acl-configuration.md](./acl-configuration.md) | ACL permission configuration, setting up access control rules between modules |
| [testing-modules.md](./testing-modules.md) | Module testing strategies, covering unit tests, Schema tests, integration tests, and Mock techniques |
| [adapter-development.md](./adapter-development.md) | Adapter development, developing apcore adapters for third-party Web frameworks |
| [multi-language.md](./multi-language.md) | Cross-language development, using YAML Schema to develop modules in multiple languages |
| [integrating-existing-projects.md](./integrating-existing-projects.md) | Adopt apcore in applications that already have their own request-ID / correlation-ID system |

## Learning Path

**Beginner (Required Reading):**

1. [Creating Modules](./creating-modules.md) -- Quick start
2. [Schema Definition](./schema-definition.md) -- Understand the core mechanism

**Intermediate:**

3. [Middleware](./middleware.md) -- Extend the execution flow
4. [ACL Configuration](./acl-configuration.md) -- Security control
5. [Testing Modules](./testing-modules.md) -- Quality assurance

**Specialized:**

6. [Adapter Development](./adapter-development.md) -- Framework integration
7. [Multi-Language Development](./multi-language.md) -- Cross-language collaboration
8. [Integrating into Existing Projects](./integrating-existing-projects.md) -- Adopt apcore alongside an existing request-ID / correlation-ID system

Before reading these guides, it is recommended to first understand the [Core Concepts](../concepts.md); for interface details, see the [Feature Specifications](../features/).
