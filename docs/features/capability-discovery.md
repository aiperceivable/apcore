# ACL-aware Capability Discovery

Use capability discovery to find public modules for the current caller before requesting their schemas. The executor evaluates the active ACL with the prospective caller frame, including the trusted identity and call depth. It returns only active, discoverable registrations that the ACL can prove visible without invocation arguments. A deny dependent on an unknown argument or custom condition can hide a module until the caller makes an actual invocation.

`list_callable` returns lightweight summaries. Use the query, prefix, tags, and read-only filters to narrow the set. Follow `next_cursor` under the same trusted identity and filters. A cursor belongs to the issuing executor. Role changes, invalid cursors, and expiry can require starting again. Each page is checked against current policy and module availability.

The `access_hint` field reports `requires_approval` for a definite human requirement, `conditional` when the approval requirement depends on the invocation, and `allowed` otherwise. Discovery does not invoke a module, the policy evaluator, or the approval handler. Every real call still runs the normal gates.

Use `get_callable_definitions` for selected module IDs to obtain complete input and output schemas. Each requested module is checked again. Missing and hidden IDs are omitted.

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    page = client.list_callable(limit=10, readonly=True)
    definitions = client.get_callable_definitions([item.module_id for item in page.items])
    for definition in definitions:
        print(definition.module_id, definition.input_schema)
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from "apcore-js";

    const client = new APCore();
    const page = client.listCallable(undefined, { limit: 10, readonly: true });
    const definitions = client.getCallableDefinitions(page.items.map(item => item.moduleId));
    for (const definition of definitions) {
      console.log(definition.moduleId, definition.inputSchema);
    }
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, CapabilityQuery, ModuleError};

    fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        let page = client.list_callable(None, CapabilityQuery {
            limit: 10,
            readonly: Some(true),
            ..CapabilityQuery::default()
        })?;
        let ids: Vec<&str> = page.items.iter().map(|item| item.module_id.as_str()).collect();
        for definition in client.get_callable_definitions(&ids, None) {
            println!("{} {}", definition.module_id, definition.input_schema);
        }
        Ok(())
    }
    ```

Hosts can supply a `CapabilitySearchProvider` to rank eligible IDs. The executor checks any ID returned by the provider against visibility and the current registration.

When system manifests are enabled, [`system.manifest.search`](./system-modules.md#systemmanifestsearch) exposes the summary query to authorized callers. MCP lists eligible tools using their full `inputSchema` and applies the same visibility to module documentation resources. Existing administrative manifest endpoints have their own ACLs.
