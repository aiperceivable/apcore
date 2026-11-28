import assert from 'node:assert/strict';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { APCore, createApprovalResult, Executor, Registry } from 'apcore-js';
import { ExecutionRouter, MCPServerFactory } from 'apcore-mcp';
import { z } from 'zod/v4';

const registry = new Registry();
const approved: unknown[] = [];
const executor = new Executor({
  registry,
  policyEvaluator: {
    async evaluate({ arguments: args }) {
      return { decision: args['noteId'] === 'private' ? 'deny' : 'require_approval' };
    },
  },
  approvalHandler: {
    supportsFinalScope: true,
    async requestApproval({ arguments: args }) {
      approved.push(args['noteId']);
      return createApprovalResult({ status: 'approved' });
    },
    async checkApproval() { return createApprovalResult({ status: 'rejected' }); },
  },
});
const core = new APCore({ registry, executor });
let calls = 0;
const lookup = await core.define({
  id: 'notes.lookup',
  description: 'Look up a note',
  readonly: true,
  input: z.object({ noteId: z.string().trim().min(1) }),
  output: z.object({ noteId: z.string() }),
  execute: ({ noteId }) => {
    calls++;
    return { noteId };
  },
});
const factory = new MCPServerFactory({ richDescription: false });
const server = factory.createServer();
const router = new ExecutionRouter({
  registry,
  call: executor.call.bind(executor),
  getCallableDescriptor: executor.getCallableDescriptor.bind(executor),
});
factory.registerHandlers(server, factory.buildTools(registry), router);
const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
const mcp = new Client({ name: 'typed-module-example', version: '1' });
try {
  assert.deepEqual(await lookup.invoke({ noteId: ' public ' }), { noteId: 'public' });
  await assert.rejects(lookup.invoke({ noteId: ' private ' }), { code: 'POLICY_DENIED' });
  await assert.rejects(lookup.invoke({ noteId: ' ' }), { code: 'SCHEMA_VALIDATION_ERROR' });
  await server.connect(serverTransport);
  await mcp.connect(clientTransport);
  const listed = (await mcp.listTools()).tools.find(tool => tool.name === lookup.id);
  assert.deepEqual(listed?.inputSchema.required, ['noteId']);
  const good = await mcp.callTool({ name: lookup.id, arguments: { noteId: ' public ' } });
  assert.ok(!good.isError);
  assert.deepEqual(good.structuredContent, { noteId: 'public' });
  assert.ok((await mcp.callTool({ name: lookup.id, arguments: { noteId: ' private ' } })).isError);
  assert.ok((await mcp.callTool({ name: lookup.id, arguments: { noteId: ' ' } })).isError);
  assert.deepEqual(approved, ['public', 'public']);
  assert.equal(calls, 2);
} finally {
  await mcp.close();
  await server.close();
  await core.shutdown();
}
