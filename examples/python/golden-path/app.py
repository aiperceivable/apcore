"""Register an existing application capability for code and MCP callers."""

from dataclasses import dataclass, field
from uuid import uuid4

from apcore import ACL, APCore, ACLRule, ApprovalRequest, ApprovalResult, ExecutionPolicy
from apcore.acl import AuditEntry
from apcore_mcp import APCoreMCP
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


@dataclass
class PendingReview:
    request: ApprovalRequest
    approved: bool = False


class DemoReviewQueue:
    """Review requests in one local process during this example."""

    def __init__(self) -> None:
        self.pending: dict[str, PendingReview] = {}

    async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
        approval_id = uuid4().hex
        self.pending[approval_id] = PendingReview(request)
        return ApprovalResult(status="pending", approval_id=approval_id)

    async def check_approval(self, approval_id: str) -> ApprovalResult:
        review = self.pending.get(approval_id)
        if review is None:
            return ApprovalResult(status="rejected")
        if review.approved:
            return ApprovalResult(status="approved", approved_by="demo-reviewer")
        return ApprovalResult(status="pending", approval_id=approval_id)

    async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
        review = self.pending.get(approval_id)
        if review is None or self.scope(review.request) != self.scope(request):
            return ApprovalResult(status="rejected")
        decision = await self.check_approval(approval_id)
        if decision.status == "approved":
            del self.pending[approval_id]
        return decision

    @staticmethod
    def scope(request: ApprovalRequest) -> tuple[object, ...]:
        return (request.caller_id, request.context.identity, request.module_id, request.arguments)

    def approve(self, approval_id: str) -> None:
        self.pending[approval_id].approved = True


def build_app(notebook: Notebook, reviews: DemoReviewQueue, audit: list[AuditEntry]) -> APCore:
    client = APCore(policy=ExecutionPolicy(strict=True, gate_destructive=True))
    client.executor.set_acl(
        ACL(
            [ACLRule(callers=["@external"], targets=[ARCHIVE, LOOKUP], effect="allow")],
            default_effect="deny",
            audit_logger=audit.append,
        )
    )
    client.executor.set_approval_handler(reviews)

    @client.module(ARCHIVE, description="Archive a note", annotations={"destructive": True})
    def archive(note_id: str) -> ArchiveResult:
        return notebook.archive(note_id)

    @client.module(LOOKUP, description="Look up a note", annotations={"readonly": True})
    def lookup(note_id: str) -> dict[str, bool]:
        return {"archived": note_id in notebook.archived}

    @client.module(RESTRICTED, description="Access a restricted note")
    def restricted() -> dict[str, bool]:
        raise AssertionError("The ACL must prevent this function from running")

    return client


def build_mcp(client: APCore, reviews: DemoReviewQueue) -> APCoreMCP:
    return APCoreMCP(client.executor, approval_handler=reviews, require_auth=False, async_tasks=False)
