"""Full-featured example module: send an email."""

from pydantic import BaseModel, Field

from apcore.module import ModuleAnnotations, ModuleExample
from apcore.observability import ContextLogger


class SendEmailInput(BaseModel):
    """Input schema for send_email module."""

    to: str
    subject: str
    body: str
    api_key: str = Field(..., json_schema_extra={"x-sensitive": True})


class SendEmailOutput(BaseModel):
    """Output schema for send_email module."""

    status: str
    message_id: str


class SendEmailModule:
    """Send an email via an external API.

    Demonstrates:
    - ModuleAnnotations (destructive, not idempotent)
    - ModuleExample instances
    - Sensitive field (api_key with x-sensitive)
    - Tags, version, metadata
    - ContextLogger usage inside execute()
    """

    input_schema = SendEmailInput
    output_schema = SendEmailOutput
    description = "Send an email message"
    tags = ["email", "communication", "external"]
    version = "1.2.0"
    metadata = {"provider": "example-smtp", "max_retries": 3}
    annotations = ModuleAnnotations(
        destructive=True,
        idempotent=False,
        open_world=True,
    )
    examples = [
        ModuleExample(
            title="Send a welcome email",
            inputs={
                "to": "user@example.com",
                "subject": "Welcome!",
                "body": "Welcome to the platform.",
                "api_key": "sk-xxx",
            },
            output={
                "status": "sent",
                "message_id": "msg-12345",
            },
            description="Sends a welcome email to a new user.",
        ),
    ]

    def execute(self, inputs: dict, context) -> dict:
        """Simulate sending an email."""
        logger = ContextLogger.from_context(context, name="send_email")
        logger.info(
            "Sending email",
            extra={"to": inputs["to"], "subject": inputs["subject"]},
        )

        message_id = f"msg-{hash(inputs['to']) % 100000:05d}"

        logger.info("Email sent successfully", extra={"message_id": message_id})
        return {"status": "sent", "message_id": message_id}
