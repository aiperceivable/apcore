"""Module decorator, FunctionModule wrapper, and type inference helpers."""

from __future__ import annotations

import dataclasses
import functools
import inspect
import logging
import re
import typing
from typing import Annotated, Any, Callable, get_args, get_origin

from pydantic import BaseModel, ConfigDict, Field, create_model
from pydantic.fields import FieldInfo

from apcore._docstrings import parse_docstring
from apcore.context import Context
from apcore.errors import ErrorCodes, FuncMissingReturnTypeError, FuncMissingTypeHintError, InvalidInputError
from apcore.module import ModuleAnnotations, ModuleExample
from apcore.utils.module_output import normalize_module_output

_logger = logging.getLogger(__name__)


def _has_explicit_field_description(annotation: Any) -> bool:
    """Check if an annotation already includes a Pydantic Field with description."""
    if get_origin(annotation) is Annotated:
        for arg in get_args(annotation)[1:]:
            if isinstance(arg, FieldInfo) and arg.description is not None:
                return True
    return False


def generate_input_model(
    func: Any,
    param_descs: dict[str, str] | None = None,
) -> type[BaseModel]:
    """Convert a function's parameter signature into a dynamic Pydantic BaseModel.

    Skips self/cls, *args, **kwargs, and Context-typed parameters.
    If **kwargs is present, the model is created with extra="allow".
    """
    # Resolve type hints safely (handles from __future__ import annotations)
    try:
        hints = typing.get_type_hints(func, include_extras=True)
    except NameError as exc:
        # Forward reference that can't be resolved
        missing_name = str(exc).split("'")[1] if "'" in str(exc) else "<forward_ref>"
        raise FuncMissingTypeHintError(
            function_name=func.__name__,
            parameter_name=missing_name,
        ) from exc

    if param_descs is None:
        _, _, param_descs = parse_docstring(func)

    sig = inspect.signature(func)
    field_dict: dict[str, Any] = {}
    has_kwargs = False

    for param_name, param in sig.parameters.items():
        # Skip self/cls
        if param_name in ("self", "cls"):
            continue
        # Skip *args
        if param.kind == inspect.Parameter.VAR_POSITIONAL:
            continue
        # Skip **kwargs but record their presence
        if param.kind == inspect.Parameter.VAR_KEYWORD:
            has_kwargs = True
            continue

        # Check for type hint
        if param_name not in hints:
            raise FuncMissingTypeHintError(
                function_name=func.__name__,
                parameter_name=param_name,
            )

        annotation = hints[param_name]

        # Skip Context-typed parameters
        if annotation is Context:
            continue

        # Inject docstring description if no explicit Field(description=...) exists
        desc = param_descs.get(param_name)
        if desc and not _has_explicit_field_description(annotation):
            annotation = Annotated[annotation, Field(description=desc)]

        # Build (type, default) tuple
        if param.default is not inspect.Parameter.empty:
            field_dict[param_name] = (annotation, param.default)
        else:
            field_dict[param_name] = (annotation, ...)

    # Create the model, with extra="allow" if **kwargs was present
    if has_kwargs:
        return create_model("InputModel", __config__=ConfigDict(extra="allow"), **field_dict)
    return create_model("InputModel", **field_dict)


def generate_output_model(func: Any) -> type[BaseModel]:
    """Convert a function's return type annotation into a Pydantic BaseModel.

    - dict / dict[str, T] -> permissive model (extra="allow")
    - BaseModel subclass -> returned directly
    - None -> empty permissive model
    - Other types -> model with single "result" field
    """
    try:
        hints = typing.get_type_hints(func, include_extras=True)
    except NameError:
        hints = {}

    return_type = hints.get("return")
    if return_type is None and "return" not in hints:
        raise FuncMissingReturnTypeError(function_name=func.__name__)

    # Handle None return type
    if return_type is None or return_type is type(None):
        return create_model("OutputModel", __config__=ConfigDict(extra="allow"))

    # Handle BaseModel subclass
    if isinstance(return_type, type) and issubclass(return_type, BaseModel):
        return return_type

    # Handle dict (bare or parameterized)
    if return_type is dict or get_origin(return_type) is dict:
        return create_model("OutputModel", __config__=ConfigDict(extra="allow"))

    # All other types: wrap in a model with a "result" field
    return create_model("OutputModel", result=(return_type, ...))


def _context_param_name(func: Any) -> str | None:
    """Return the name of the parameter typed as ``Context``, or None.

    Detection is type-based, not name-based. Returns the parameter name when
    a single ``Context``-typed parameter is found; returns ``None`` when the
    function has no Context parameter or its annotations cannot be resolved.
    """
    try:
        hints = typing.get_type_hints(func, include_extras=True)
    except NameError:
        return None

    for param_name, hint in hints.items():
        if param_name == "return":
            continue
        if hint is Context:
            return param_name

    return None


def _normalize_result(result: Any) -> dict[str, Any]:
    """Normalize a function's return value to a dict for the executor pipeline."""
    if result is None:
        return {}
    result = normalize_module_output(result)
    if isinstance(result, dict):
        return result
    return {"result": result}


class FunctionModule:
    """Wrapper that adapts a Python function to the apcore module interface.

    Provides input_schema, output_schema, and execute() so the wrapped
    function can participate in the full executor pipeline (ACL, middleware,
    timeout, validation, async support).
    """

    def __init__(
        self,
        func: Callable,
        module_id: str,
        description: str | None = None,
        documentation: str | None = None,
        tags: list[str] | None = None,
        version: str = "1.0.0",
        annotations: dict[str, Any] | None = None,
        metadata: dict[str, Any] | None = None,
        display: dict[str, Any] | None = None,
        examples: list[ModuleExample] | None = None,
        input_schema: type[BaseModel] | None = None,
        output_schema: type[BaseModel] | None = None,
        resources: dict[str, Any] | None = None,
    ) -> None:
        self._func = func
        self.module_id = module_id

        # Parse docstring once for reuse by input model and documentation
        doc_desc, doc_body, param_descs = parse_docstring(func)

        self.input_schema = input_schema if input_schema is not None else generate_input_model(func, param_descs)
        self.output_schema = output_schema if output_schema is not None else generate_output_model(func)

        context_param_name = _context_param_name(func)
        positional_only = [
            parameter
            for parameter in inspect.signature(func).parameters.values()
            if parameter.kind == inspect.Parameter.POSITIONAL_ONLY
        ]

        # Description priority chain
        if description is not None:
            self.description = description
        elif doc_desc is not None:
            self.description = doc_desc
        else:
            self.description = f"Module {func.__name__}"

        self.documentation = documentation if documentation is not None else doc_body
        self.tags = tags
        self.version = version
        if annotations is not None and isinstance(annotations, dict):
            known_fields = {f.name for f in dataclasses.fields(ModuleAnnotations)}
            unknown = [k for k in annotations if k not in known_fields]
            if unknown:
                _logger.warning("Unknown annotation keys ignored for '%s': %s", module_id, unknown)
            self.annotations = ModuleAnnotations(**{k: v for k, v in annotations.items() if k in known_fields})
        else:
            self.annotations = annotations
        self.metadata = metadata
        self.display = display
        self.examples = examples
        # Declared execution budget (PROTOCOL_SPEC §5.2 / D-11). The executor
        # reads ``resources["timeout"]`` in milliseconds and uses it instead of
        # ``executor.default_timeout`` for this module.
        self.resources = resources

        def call_arguments(inputs: dict[str, Any], context: Context) -> tuple[list[Any], dict[str, Any]]:
            kwargs = dict(inputs)
            if context_param_name is not None:
                kwargs[context_param_name] = context
            # Use defaults only for gaps before the last supplied positional
            # argument; the original callable handles trailing defaults.
            count = next(
                (index for index, param in reversed(list(enumerate(positional_only, 1))) if param.name in kwargs),
                0,
            )
            args = []
            for param in positional_only[:count]:
                value = kwargs.pop(param.name, param.default)
                if value is inspect.Parameter.empty:
                    raise TypeError(f"Missing positional-only argument: {param.name!r}")
                args.append(value)
            return args, kwargs

        # Create execute closures — two separate defs required so that
        # inspect.iscoroutinefunction returns the correct value.
        if inspect.iscoroutinefunction(func):

            async def _async_execute(inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                args, kwargs = call_arguments(inputs, context)
                result = await func(*args, **kwargs)
                return _normalize_result(result)

            self.execute = _async_execute
        else:

            def _sync_execute(inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                args, kwargs = call_arguments(inputs, context)
                result = func(*args, **kwargs)
                return _normalize_result(result)

            self.execute = _sync_execute


def _attribute_host(func: Callable) -> Callable:
    """Return the callable that carries ``apcore_module`` and is handed back to the caller.

    A plain function is its own host. A bound method rejects attribute
    assignment, so it gets a transparent forwarder instead.
    """
    if not inspect.ismethod(func):
        return func
    if inspect.iscoroutinefunction(func):

        @functools.wraps(func)
        async def _async_forward(*args: Any, **kwargs: Any) -> Any:
            return await func(*args, **kwargs)

        return _async_forward

    @functools.wraps(func)
    def _forward(*args: Any, **kwargs: Any) -> Any:
        return func(*args, **kwargs)

    return _forward


def _make_auto_id(func: Callable) -> str:
    """Generate a module ID from a function's module path and qualified name."""
    if func.__module__ == "__main__":
        # `__main__` names the entry script, not a location: it is not a legal ID
        # segment, and the same function imported elsewhere would get another ID.
        raise InvalidInputError(
            f"Cannot derive a module ID for '{func.__qualname__}' defined in __main__; "
            'pass an explicit id, e.g. @module(id="my_app.my_function").',
            code=ErrorCodes.INVALID_MODULE_ID,
        )
    raw = f"{func.__module__}.{func.__qualname__}"
    raw = raw.replace("<locals>.", ".")
    raw = raw.lower()
    raw = re.sub(r"[^a-z0-9_.]", "_", raw)
    segments = raw.split(".")
    segments = [f"_{s}" if s and s[0].isdigit() else s for s in segments]
    return ".".join(segments)


def module(
    func_or_none: Callable | None = None,
    /,
    *,
    id: str | None = None,  # noqa: A002
    description: str | None = None,
    documentation: str | None = None,
    annotations: dict[str, Any] | None = None,
    tags: list[str] | None = None,
    version: str = "1.0.0",
    metadata: dict[str, Any] | None = None,
    display: dict[str, Any] | None = None,
    examples: list[ModuleExample] | None = None,
    resources: dict[str, Any] | None = None,
    registry: Any = None,
) -> Any:
    """Wrap a Python function as an apcore module.

    Dual-purpose: works as a decorator and as a function call.
    """

    def _wrap(func: Callable, *, return_module: bool = False) -> Any:
        module_id = id if id is not None else _make_auto_id(func)
        fm = FunctionModule(
            func=func,
            module_id=module_id,
            description=description,
            documentation=documentation,
            annotations=annotations,
            tags=tags,
            version=version,
            metadata=metadata,
            display=display,
            examples=examples,
            resources=resources,
        )
        # Resolved before registering: a bound method refuses attribute
        # assignment, and failing after `register` would leave the module
        # registered while the caller sees an exception.
        host = None if return_module else _attribute_host(func)
        if registry is not None:
            registry.register(fm.module_id, fm)
        if host is None:
            return fm
        host.apcore_module = fm  # type: ignore[attr-defined]
        return host

    if func_or_none is not None and callable(func_or_none):
        # Bare @module or module(func, id=...) call
        if id is not None or registry is not None:
            # Function call form: module(func, id="x")
            return _wrap(func_or_none, return_module=True)
        # Bare decorator: @module
        return _wrap(func_or_none, return_module=False)

    # Decorator with arguments: @module(id="x", ...)
    def decorator(func: Callable) -> Callable:
        return _wrap(func, return_module=False)

    return decorator
