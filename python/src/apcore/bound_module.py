"""Typed operations on functions registered with :meth:`APCore.module`."""

from __future__ import annotations

import inspect
from collections.abc import Callable, Coroutine
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, ParamSpec, Protocol, Self, TypeVar, get_origin, get_type_hints, overload

from pydantic import BaseModel, TypeAdapter
from pydantic_core import PydanticSerializationError, to_json

from apcore.context import Context, _APPROVAL_TOKEN_KEY
from apcore.decorator import FunctionModule, _context_param_name, _forward_function
from apcore.module import PreflightResult, PreviewResult

if TYPE_CHECKING:
    from apcore.client import APCore

P = ParamSpec("P")
R = TypeVar("R", covariant=True)
T = TypeVar("T")


class _GovernedModule(Protocol[P, R]):
    @property
    def apcore_module(self) -> FunctionModule:
        """The registered module, including its introspection metadata."""
        ...

    @property
    def __name__(self) -> str: ...

    def invoke(self, *args: P.args, **kwargs: P.kwargs) -> R:
        """Execute a call through the client and return its typed result."""
        ...

    async def invoke_async(self, *args: P.args, **kwargs: P.kwargs) -> R:
        """Execute a governed call on the current event loop."""
        ...

    def validate(self, *args: P.args, **kwargs: P.kwargs) -> PreflightResult:
        """Run the client's preflight checks."""
        ...

    async def validate_async(self, *args: P.args, **kwargs: P.kwargs) -> PreflightResult: ...

    def preview(self, *args: P.args, **kwargs: P.kwargs) -> PreviewResult:
        """Return the changes disclosed by the client's preflight checks."""
        ...

    async def preview_async(self, *args: P.args, **kwargs: P.kwargs) -> PreviewResult: ...

    def with_context(self, context: Context[Any]) -> Self:
        """Bind a context to governed operations without changing the function's signature."""
        ...

    def with_approval_token(self, token: str) -> Self:
        """Pass a resumption token to the governed invocation's approval gate."""
        ...


class BoundModule(_GovernedModule[P, R], Protocol[P, R]):
    """A registered sync function with typed governed operations and raw access."""

    def __call__(self, *args: P.args, **kwargs: P.kwargs) -> R: ...

    @property
    def raw(self) -> Callable[P, R]:
        """The original callable for direct execution."""
        ...


class AsyncBoundModule(_GovernedModule[P, R], Protocol[P, R]):
    """A registered async function whose governed calls return its resolved type."""

    def __call__(self, *args: P.args, **kwargs: P.kwargs) -> Coroutine[Any, Any, R]: ...

    @property
    def raw(self) -> Callable[P, Coroutine[Any, Any, R]]:
        """The original coroutine function for direct execution."""
        ...


class _BoundModuleDecorator(Protocol):
    @overload
    def __call__(self, func: Callable[P, Coroutine[Any, Any, T]], /) -> AsyncBoundModule[P, T]: ...

    @overload
    def __call__(self, func: Callable[P, T], /) -> BoundModule[P, T]: ...


@dataclass(frozen=True)
class _Invocation:
    client: APCore
    module_id: str
    context: Context[Any] | None
    approval_token: str | None
    signature: inspect.Signature
    context_parameter: str | None
    return_type: Any
    return_adapter: TypeAdapter[Any]

    def arguments(self, args: tuple[Any, ...], kwargs: dict[str, Any]) -> tuple[dict[str, Any], Context[Any] | None]:
        bound = self.bind(args, kwargs)
        context = self.context
        inputs: dict[str, Any] = {}
        extras: dict[str, Any] = {}
        for name, value in bound.arguments.items():
            kind = self.signature.parameters[name].kind
            if name == self.context_parameter:
                if not isinstance(value, Context):
                    raise TypeError(f"{name!r} must be an apcore Context")
                if context is not None and context is not value:
                    raise TypeError("The function argument and bound invocation provide different contexts")
                context = value
            elif kind == inspect.Parameter.VAR_POSITIONAL:
                raise TypeError("Variadic positional arguments cannot be represented as module inputs")
            elif kind == inspect.Parameter.VAR_KEYWORD:
                extras = value
            else:
                inputs[name] = value
        if inputs.keys() & extras.keys():
            raise TypeError("A positional argument and a keyword extra cannot use the same module input name")
        inputs.update(extras)
        if self.approval_token is not None:
            if _APPROVAL_TOKEN_KEY in inputs:
                raise TypeError("An approval token is already present in the module inputs")
            inputs[_APPROVAL_TOKEN_KEY] = self.approval_token
        return inputs, context

    def bind(self, args: tuple[Any, ...], kwargs: dict[str, Any]) -> inspect.BoundArguments:
        if self.context_parameter is None:
            return self.signature.bind(*args, **kwargs)
        try:
            bound = self.signature.bind(*args, **kwargs)
        except TypeError:
            if self.context_parameter in kwargs:
                raise
        else:
            if self.context_parameter not in bound.arguments or isinstance(
                bound.arguments[self.context_parameter], Context
            ):
                return bound
            if self.context_parameter in kwargs:
                return bound
        parameters = [p for name, p in self.signature.parameters.items() if name != self.context_parameter]
        return self.signature.replace(parameters=parameters).bind(*args, **kwargs)

    def result(self, output: dict[str, Any]) -> Any:
        if self.return_type is type(None):
            return None
        object_result = self.return_type is dict or get_origin(self.return_type) is dict
        model_result = isinstance(self.return_type, type) and issubclass(self.return_type, BaseModel)
        payload = output if object_result or model_result else output["result"]
        try:
            encoded = to_json(payload)
        except PydanticSerializationError:
            return self.return_adapter.validate_python(payload)
        return self.return_adapter.validate_json(encoded)

    def invoke(self, *args: Any, **kwargs: Any) -> Any:
        inputs, context = self.arguments(args, kwargs)
        return self.result(self.client.call(self.module_id, inputs, context))

    async def invoke_async(self, *args: Any, **kwargs: Any) -> Any:
        inputs, context = self.arguments(args, kwargs)
        return self.result(await self.client.call_async(self.module_id, inputs, context))

    def validate(self, *args: Any, **kwargs: Any) -> PreflightResult:
        inputs, context = self.arguments(args, kwargs)
        return self.client.validate(self.module_id, inputs, context)

    async def validate_async(self, *args: Any, **kwargs: Any) -> PreflightResult:
        inputs, context = self.arguments(args, kwargs)
        return await self.client.validate_async(self.module_id, inputs, context)

    def preview(self, *args: Any, **kwargs: Any) -> PreviewResult:
        return PreviewResult(changes=self.validate(*args, **kwargs).predicted_changes)

    async def preview_async(self, *args: Any, **kwargs: Any) -> PreviewResult:
        result = await self.validate_async(*args, **kwargs)
        return PreviewResult(changes=result.predicted_changes)


def _bind_module(client: APCore, func: Callable[..., Any], module: FunctionModule) -> Callable[..., Any]:
    signature = inspect.signature(func)
    context_parameter = _context_param_name(func)
    return_type = get_type_hints(func, include_extras=True)["return"]
    return_adapter: TypeAdapter[Any] = TypeAdapter(return_type)

    def bind(context: Context[Any] | None, approval_token: str | None) -> Callable[..., Any]:
        invocation = _Invocation(
            client, module.module_id, context, approval_token, signature, context_parameter, return_type, return_adapter
        )

        def with_context(context: Context[Any]) -> Callable[..., Any]:
            return bind(context, approval_token)

        def with_approval_token(token: str) -> Callable[..., Any]:
            return bind(context, token)

        wrapped = _forward_function(func)
        wrapped.__dict__.update(
            apcore_module=module,
            raw=func,
            invoke=invocation.invoke,
            invoke_async=invocation.invoke_async,
            validate=invocation.validate,
            validate_async=invocation.validate_async,
            preview=invocation.preview,
            preview_async=invocation.preview_async,
            with_context=with_context,
            with_approval_token=with_approval_token,
        )
        return wrapped

    return bind(None, None)
