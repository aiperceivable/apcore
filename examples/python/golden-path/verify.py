"""Exercise one application through direct calls and a real MCP session."""

import asyncio
import json
from collections.abc import AsyncIterator
from contextlib import AsyncExitStack, asynccontextmanager
from socket import socket

import uvicorn
from apcore import APCore, BoundModule
from apcore.acl import AuditEntry
from apcore.errors import ACLDeniedError, ApprovalPendingError, SchemaValidationError
from apcore_mcp import APCoreMCP, InMemoryApprovalStore, StorageBackedApprovalHandler
from app import ARCHIVE, LOOKUP, RESTRICTED, ArchiveResult, Notebook, build_app, build_mcp
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client
from mcp.types import CallToolResult, TextContent


async def verify_direct(
    client: APCore,
    archive: BoundModule[[str], ArchiveResult],
    notebook: Notebook,
    reviews: InMemoryApprovalStore,
) -> None:
    assert await client.call_async(LOOKUP, {"note_id": "from-code"}) == {"archived": False}

    try:
        await client.call_async(RESTRICTED, {})
    except ACLDeniedError:
        pass
    else:
        raise AssertionError("Expected an ACL denial")

    try:
        await client.call_async(LOOKUP, {"note_id": ["invalid"]})
    except SchemaValidationError:
        pass
    else:
        raise AssertionError("Expected input validation to fail")

    try:
        await archive.invoke_async("from-code")
    except ApprovalPendingError as pending:
        approval_id = pending.result.audit_id
        resume_token = pending.approval_id
        assert approval_id and resume_token and approval_id != resume_token
    else:
        raise AssertionError("Expected approval to be pending")
    assert notebook.archived == []
    assert await reviews.resolve(approval_id, approved=True)
    resumed = await archive.with_approval_token(resume_token).invoke_async("from-code")
    assert isinstance(resumed, ArchiveResult) and resumed.archived == "from-code"
    assert notebook.archived == ["from-code"]


@asynccontextmanager
async def local_session(mcp: APCoreMCP) -> AsyncIterator[ClientSession]:
    async with mcp.async_serve() as application:
        with socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
            config = uvicorn.Config(application, lifespan="off", log_level="warning")
            server = uvicorn.Server(config)
            task = asyncio.create_task(server.serve(sockets=[listener]))
            try:
                async with asyncio.timeout(10):
                    while not server.started:
                        if task.done():
                            task.result()
                            raise RuntimeError("The MCP server stopped before startup")
                        await asyncio.sleep(0.01)
                url = f"http://127.0.0.1:{port}/mcp"
                async with (
                    streamable_http_client(url, terminate_on_close=False) as (read, write, _),
                    ClientSession(read, write) as session,
                ):
                    await session.initialize()
                    yield session
            finally:
                server.should_exit = True
                await task


def tool_text(result: CallToolResult) -> str:
    (content,) = result.content
    assert isinstance(content, TextContent)
    return content.text


async def verify_mcp(mcp: APCoreMCP, notebook: Notebook, reviews: InMemoryApprovalStore) -> None:
    async with local_session(mcp) as session:
        tools = {tool.name: tool for tool in (await session.list_tools()).tools}
        assert tools[ARCHIVE].inputSchema["properties"]["note_id"]["type"] == "string"
        assert "__apcore_approval_check" in tools
        lookup = await session.call_tool(LOOKUP, {"note_id": "from-code"})
        assert not lookup.isError and json.loads(tool_text(lookup)) == {"archived": True}
        denied = await session.call_tool(RESTRICTED, {})
        assert denied.isError and "Access denied" in tool_text(denied)
        invalid = await session.call_tool(LOOKUP, {"note_id": ["invalid"]})
        assert invalid.isError and "validation" in tool_text(invalid).lower()

        arguments = {"note_id": "from-mcp"}
        pending = await session.call_tool(ARCHIVE, arguments)
        assert pending.isError and "Approval pending" in tool_text(pending)
        assert notebook.archived == ["from-code"]
        assert pending.meta is not None
        approval = pending.meta["apcore"]["approval"]
        approval_id, resume_token = approval["approvalId"], approval["resumeToken"]
        assert approval_id != resume_token
        assert resume_token not in tool_text(pending)

        poll_arguments = {"approval_id": approval_id}
        status = await session.call_tool("__apcore_approval_check", poll_arguments)
        assert not status.isError
        assert json.loads(tool_text(status)) == {**poll_arguments, "status": "pending"}
        assert await reviews.resolve(approval_id, approved=True)
        status = await session.call_tool("__apcore_approval_check", poll_arguments)
        assert not status.isError
        assert json.loads(tool_text(status)) == {**poll_arguments, "status": "approved"}

        approved = await session.call_tool(ARCHIVE, arguments, meta={"approvalId": resume_token})
        assert not approved.isError, tool_text(approved)
        assert json.loads(tool_text(approved)) == {"archived": "from-mcp"}
        assert notebook.archived == ["from-code", "from-mcp"]


async def main() -> None:
    notebook = Notebook()
    audit: list[AuditEntry] = []
    async with AsyncExitStack() as cleanup:
        direct, archive = build_app(notebook, audit)
        cleanup.push_async_callback(direct.shutdown)
        direct_reviews = InMemoryApprovalStore()
        direct.executor.set_approval_handler(StorageBackedApprovalHandler(direct_reviews))
        await verify_direct(direct, archive, notebook, direct_reviews)

        mcp_client, _ = build_app(notebook, audit)
        cleanup.push_async_callback(mcp_client.shutdown)
        mcp_reviews = InMemoryApprovalStore()
        await verify_mcp(build_mcp(mcp_client, mcp_reviews), notebook, mcp_reviews)

    assert sum(entry.decision == "deny" and entry.target_id == RESTRICTED for entry in audit) == 2
    assert sum(entry.decision == "allow" and entry.target_id == ARCHIVE for entry in audit) == 4
    assert all(entry.trace_id for entry in audit)
    for entry in audit:
        print(entry.trace_id, entry.caller_id, entry.target_id, entry.decision)
    print("Direct calls and MCP both passed the governance checks.")


if __name__ == "__main__":
    asyncio.run(main())
