# TypeScript Golden Path

`verify.ts` registers a typed module with `APCore.define`, then calls it
through its typed handle and through an MCP client connected in-process.
The same executor normalizes the arguments, denies private lookups, and
requires approval for public lookups. The example's approval handler is
a test double that records the arguments and immediately returns approval.

Run this example in an ES module project with compatible `apcore-js` and
`apcore-mcp` packages, `@modelcontextprotocol/sdk`, `zod`, and TypeScript.

```sh
npx tsc --strict --module NodeNext --target ES2022 --outDir dist verify.ts
node dist/verify.js
```

The optional `agent-tool.ts` example requires the `ai` package and
`@types/json-schema` for strict TypeScript builds. Pass it the
same handle and the input schema from `core.getCallableDefinition(lookup.id)`.
Use the returned tool in the `tools` option of your AI SDK call. It invokes the
same executor as the MCP route. [AI SDK tool reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/tool)
