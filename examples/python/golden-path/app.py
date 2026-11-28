"""Register an existing application capability for code and MCP callers."""

from dataclasses import dataclass, field

from apcore import ACL, ACLRule, APCore, BoundModule, ExecutionPolicy
from apcore.acl import AuditEntry
from apcore_mcp import APCoreMCP, InMemoryApprovalStore
from pydantic import BaseModel

ARCHIVE = "notes.archive"
LOOKUP = "notes.lookup"
RESTRICTED = "notes.restricted"


class ArchiveResult(BaseModel):
    archived: str


@dataclass
class Notebook:
    archived: list[str] = field(default_factory=list)

    def archive(self, note_id: str) -> ArchiveResult:
        self.archived.append(note_id)
        return ArchiveResult(archived=note_id)


def build_app(notebook: Notebook, audit: list[AuditEntry]) -> tuple[APCore, BoundModule[[str], ArchiveResult]]:
    client = APCore(policy=ExecutionPolicy(strict=True, gate_destructive=True))
    client.executor.set_acl(
        ACL(
            [ACLRule(callers=["@external"], targets=[ARCHIVE, LOOKUP], effect="allow")],
            default_effect="deny",
            audit_logger=audit.append,
        )
    )

    @client.module(ARCHIVE, description="Archive a note", destructive=True, requires_approval=True)
    def archive(note_id: str) -> ArchiveResult:
        return notebook.archive(note_id)

    @client.module(LOOKUP, description="Look up a note", annotations={"readonly": True})
    def lookup(note_id: str) -> dict[str, bool]:
        return {"archived": note_id in notebook.archived}

    @client.module(RESTRICTED, description="Access a restricted note")
    def restricted() -> dict[str, bool]:
        raise AssertionError("The ACL must prevent this function from running")

    return client, archive


def build_mcp(client: APCore, reviews: InMemoryApprovalStore) -> APCoreMCP:
    return APCoreMCP(client.executor, approval_store=reviews, require_auth=False, async_tasks=False)
