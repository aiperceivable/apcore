"""Keep policy evaluation and approval bound to the dispatch snapshot."""

from __future__ import annotations

import asyncio
import hashlib
import json
import math
import re
import struct
import time
from copy import copy, deepcopy
from typing import TYPE_CHECKING, Any, Annotated, get_args, get_origin
from uuid import UUID, SafeUUID

from pydantic import AliasPath, BaseModel, RootModel
from pydantic.fields import FieldInfo

from apcore.errors import PolicyEvaluationError
from apcore.cancel import _AwaitDeadlineExpired, _await_with_cancellation
from apcore.context import _APPROVAL_TOKEN_KEY
from apcore.decorator import FunctionModule
from apcore.policy import PolicyEvaluationRequest, PolicyEvaluator, PolicyVerdict
from apcore.utils.module_output import _validation_property_name

if TYPE_CHECKING:
    from apcore.cancel import CancelToken
    from apcore.context import Context
    from apcore.pipeline import PipelineContext

_SAFE_INT = (1 << 53) - 1
_RULE_ID = re.compile(r"[A-Za-z0-9_.-]{1,64}\Z")
_EVALUATOR_TIMEOUT_SECONDS = 1.0


def _scope_text(value: Any) -> str:
    """Encode supported native values without conflating distinct numbers."""
    if value is None:
        return "null"
    if type(value) is bool:
        return "true" if value else "false"
    if type(value) is int and abs(value) <= _SAFE_INT:
        return f"i{value};"
    if type(value) is float and math.isfinite(value) and (not value.is_integer() or abs(value) <= _SAFE_INT):
        return f"f{struct.pack('>d', value).hex()};"
    if type(value) is str:
        return json.dumps(value, ensure_ascii=False)
    if type(value) is list:
        return "[" + ",".join(_scope_text(item) for item in value) + "]"
    if type(value) is dict and all(type(key) is str for key in value):
        keys = sorted(value, key=lambda key: key.encode("utf-16-be"))
        return "{" + ",".join(f"{_scope_text(key)}:{_scope_text(value[key])}" for key in keys) + "}"
    raise ValueError("Unsupported value in policy scope")


def _argument_digest(arguments: dict[str, Any]) -> str:
    try:
        encoded = _scope_text(arguments).encode("utf-8")
    except (ValueError, RecursionError, KeyError, RuntimeError):
        raise PolicyEvaluationError() from None
    return hashlib.sha256(encoded).hexdigest()


def _project_function_model(value: BaseModel) -> Any:
    fields = type(value).model_fields
    extras = value.model_extra or {}
    stored = vars(value)
    # A detached JSON object cannot preserve private state or whether a field
    # equal to its default was actually supplied.
    if (
        value.__pydantic_private__
        or stored.keys() != fields.keys()
        or value.model_fields_set != fields.keys() | extras.keys()
    ):
        raise PolicyEvaluationError()
    if isinstance(value, RootModel):
        return _project_function_value(stored["root"], fields["root"].annotation)
    projected = {key: _project_function_value(item, Any) for key, item in extras.items()}
    for name, field in fields.items():
        if isinstance(field.validation_alias, AliasPath):
            raise PolicyEvaluationError()
        wire_name = _validation_property_name(name, field.validation_alias, name)
        if wire_name in projected:
            raise PolicyEvaluationError()
        projected[wire_name] = _project_function_value(stored[name], field.annotation)
    return projected


def _project_function_value(value: Any, annotation: Any) -> Any:
    """Project native values only at paths where the declaration fixes their type."""
    if get_origin(annotation) is Annotated:
        annotation = get_args(annotation)[0]
    if annotation is UUID or type(value) is UUID:
        if annotation is not UUID or type(value) is not UUID or value.is_safe is not SafeUUID.unknown:
            raise PolicyEvaluationError()
        return str(value)
    model_type = isinstance(annotation, type) and issubclass(annotation, BaseModel)
    if model_type or isinstance(value, BaseModel):
        if not model_type or type(value) is not annotation:
            raise PolicyEvaluationError()
        return _project_function_model(value)
    parameters = get_args(annotation)
    if type(value) is dict:
        item_type = parameters[1] if get_origin(annotation) is dict and parameters else Any
        return {key: _project_function_value(item, item_type) for key, item in value.items()}
    if type(value) is list:
        item_type = parameters[0] if get_origin(annotation) is list and parameters else Any
        return [_project_function_value(item, item_type) for item in value]
    return value


def _project_function_arguments(arguments: dict[str, Any], fields: dict[str, FieldInfo]) -> dict[str, Any]:
    return {
        name: _project_function_value(value, fields[name].annotation if name in fields else Any)
        for name, value in arguments.items()
    }


class _PolicyInvocation:
    """Record the selected invocation before middleware or provider callbacks."""

    def __init__(self, ctx: PipelineContext, version: str | None) -> None:
        self.module_id = ctx.module_id
        self.module = ctx.module
        self.version = version
        self.caller_id = ctx.context.caller_id
        try:
            identity = ctx.context.identity
            self.authority_key = identity.authority_key() if identity is not None else None
            self.identity = deepcopy(identity)
        except Exception:
            raise PolicyEvaluationError() from None
        self.arguments: dict[str, Any] | None = None
        self._native_arguments: dict[str, Any] | None = None
        self._function_fields: dict[str, FieldInfo] | None = None
        self.digest: str | None = None
        self.authorized = False

    def capture(self, ctx: PipelineContext) -> None:
        arguments = ctx.validated_inputs
        if arguments is None or _APPROVAL_TOKEN_KEY in arguments:
            raise PolicyEvaluationError()
        try:
            if isinstance(self.module, FunctionModule):
                self._function_fields = dict(getattr(self.module.input_schema, "model_fields", {}))
                projected = _project_function_arguments(arguments, self._function_fields)
                self.digest = _argument_digest(projected)
                self._native_arguments = deepcopy(arguments)
                if (
                    _argument_digest(_project_function_arguments(self._native_arguments, self._function_fields))
                    != self.digest
                ):
                    raise PolicyEvaluationError()
                self.arguments = projected
            else:
                self.digest = _argument_digest(arguments)
                self.arguments = deepcopy(arguments)
        except Exception:
            raise PolicyEvaluationError() from None

    def _check_binding(self, ctx: PipelineContext) -> None:
        identity = ctx.context.identity
        if (
            ctx.module_id != self.module_id
            or ctx.module is not self.module
            or ctx.context.caller_id != self.caller_id
            or identity != self.identity
            or (self.authority_key is not None and (identity is None or identity.authority_key() != self.authority_key))
        ):
            raise PolicyEvaluationError()

    def checked_arguments(self, ctx: PipelineContext, *, dispatch: bool = False) -> dict[str, Any]:
        self._check_binding(ctx)
        capture = deepcopy(self.arguments)
        current = ctx.validated_inputs
        if self._function_fields is not None:
            try:
                if current is None or current != self._native_arguments:
                    raise PolicyEvaluationError()
                current = _project_function_arguments(current, self._function_fields)
            except Exception:
                raise PolicyEvaluationError() from None
        if (
            current is None
            or capture is None
            or self.digest is None
            or _argument_digest(current) != self.digest
            or _argument_digest(capture) != self.digest
        ):
            raise PolicyEvaluationError()
        if dispatch and self._native_arguments is not None:
            return self._native_arguments
        return capture

    def preflight_snapshot(self, ctx: PipelineContext) -> tuple[dict[str, Any], Context[Any]]:
        self._check_binding(ctx)
        if self.arguments is None or self.digest is None or _argument_digest(self.arguments) != self.digest:
            raise PolicyEvaluationError()
        context = copy(ctx.context)
        context.identity = deepcopy(self.identity)
        context.call_chain = list(ctx.context.call_chain)
        return self.arguments, context


async def _evaluate_policy(
    evaluator: PolicyEvaluator,
    request: PolicyEvaluationRequest,
    token: CancelToken | None,
    deadline: float | None,
) -> PolicyVerdict:
    """Bound a trusted provider independently of its cancellation cooperation."""
    remaining = _EVALUATOR_TIMEOUT_SECONDS
    if deadline is not None:
        remaining = min(remaining, deadline - time.time())
    try:
        return await _await_with_cancellation(lambda: _read_verdict(evaluator, request), token, remaining)
    except _AwaitDeadlineExpired:
        raise PolicyEvaluationError() from None


async def _read_verdict(evaluator: PolicyEvaluator, request: PolicyEvaluationRequest) -> PolicyVerdict:
    try:
        verdict = await evaluator.evaluate(request)
        if isinstance(verdict, PolicyVerdict) and verdict.decision in ("allow", "deny", "require_approval"):
            return verdict
    except (Exception, asyncio.CancelledError):
        pass
    raise PolicyEvaluationError()


def _safe_rule_id(rule_id: str | None) -> str | None:
    return rule_id if isinstance(rule_id, str) and _RULE_ID.fullmatch(rule_id) else None
