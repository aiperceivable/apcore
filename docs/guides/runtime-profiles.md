---
description: "Choose the in-memory or full APCore runtime and migrate approval, metrics, event subscribers, and tool exports."
---

# Runtime Profiles

The default Core Profile registers and runs modules in memory. It includes
preflight, approval, cancellation, managed lifecycle, and streaming completion.
Choose explicit extensions when an application reads configuration or discovers
modules from files. The full assembly provides those capabilities together.

## Assemble the application

=== "Python"
    Install the base distribution for the in-memory client. Install
    `apcore[all]` for the full assembly or select `config`, `discovery`,
    `observability`, and `events` separately.

    ```python
    from apcore import APCore

    client = APCore()
    ```

    ```python
    from apcore.full import APCore, Config

    client = APCore(config=Config.load("apcore.yaml"))
    client.discover()
    ```

=== "TypeScript"
    Import the in-memory assembly from `apcore-js`. On Node, import the full
    assembly or compose the `config`, `discovery`, `sys-modules`, and
    `events` subpaths.

    ```typescript
    import { APCore } from "apcore-js";

    const client = new APCore();
    ```

    ```typescript
    import { Config } from "apcore-js/config";
    import { APCore } from "apcore-js/full";

    const client = new APCore({ config: Config.load("apcore.yaml") });
    await client.discover();
    ```

=== "Rust"
    Use the crate with its default features for in-memory execution.
    Enable the `full` feature for the configured assembly.

    ```rust
    use apcore::APCore;

    fn main() {
        let _client = APCore::in_memory();
    }
    ```

    ```rust
    use std::path::Path;
    use apcore::{APCore, APCoreOptions, Config};

    fn main() -> Result<(), Box<dyn std::error::Error>> {
        let _client = APCore::try_start(APCoreOptions {
            config: Some(Config::load(Path::new("apcore.yaml"))?),
            fail_on_error: true,
            ..Default::default()
        })?;
        Ok(())
    }
    ```

## Check approval at startup

The default gate rejects an approval-required call with `APPROVAL_DENIED` when
the host has no handler. An effective `destructive` annotation also requires
approval. Use the Executor's governance report on the running default or
injected Executor to identify missing gates and approval coverage. When
selecting a custom strategy for production, confirm that it includes the
built-in approval step.

The configuration file declares its own supported document version. Keep the
document version for the configuration dialect your application uses.

For Node hosts whose lifecycle hooks call back into the client after an
`await`, the TypeScript full assembly provides per-instance async lifecycle
context. Hosts that compose a custom Node assembly can supply
`createNodeLifecyclePlugin()`.

To retain the earlier behavior during migration, set both `strict` and
`gate_destructive` to false in the policy supplied to the Executor. Set
`gate_destructive` alone to false to preserve strict enforcement of approval
requirements from annotations, ACL decisions, and policy rules. See the
[approval system](../features/approval-system.md#execution-policy).

## Move host integrations

| Existing use | Assembly |
| --- | --- |
| Prometheus HTTP server | Pass the Core text renderer to the Toolkit listener or the application's listener. See [Metrics and Usage](../features/metrics-and-usage.md#prometheus-export). |
| A2A event delivery | Supply the A2A adapter's subscriber factory to each host. Keep its existing `type: a2a` or filter delegate declaration. See [Subscriber Factories](../features/event-system.md#subscriber-factories). |
| Provider tool exports | Use the MCP adapter for MCP tools and the provider converters shipped with it for OpenAI Chat, Responses, and Anthropic. OpenAI and Anthropic use hyphens in place of module ID dots. OpenAI uses strict schemas by default. |
| Generic schema export | Use the Registry schema export. Pass the strict option to apply the shared strict conversion. See [Schema System](../features/schema-system.md). |
| Reload dependent modules | Choose a `path_filter` that includes their module IDs. The reload input reports a schema error for the removed `reload_dependents` option. |

Run preflight and a harmless invocation through each adapter with the same
Core distribution used by the application. The
[Golden Path](https://github.com/aiperceivable/apcore/tree/main/examples/python/golden-path)
checks direct and MCP calls on the same governance pipeline.
