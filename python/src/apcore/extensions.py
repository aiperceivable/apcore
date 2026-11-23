"""Extension point system for apcore.

Provides a centralized mechanism to register, query, and wire custom
extensions (discoverers, middleware, ACL providers, span exporters, and
module validators) into the apcore runtime.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

from apcore.acl import ACL
from apcore.approval import ApprovalHandler
from apcore.errors import InvalidInputError
from apcore.middleware import Middleware
from apcore.observability.tracing import Span, SpanExporter, TracingMiddleware
from apcore.registry.registry import Discoverer, ModuleValidator

if TYPE_CHECKING:
    from apcore.executor import Executor
    from apcore.registry import Registry

logger = logging.getLogger(__name__)


class _CompositeExporter:
    """Delegates span export to multiple underlying exporters."""

    def __init__(self, exporters: list[SpanExporter]) -> None:
        self._exporters = exporters

    def export(self, span: Span) -> None:
        for exp in self._exporters:
            try:
                exp.export(span)
            except Exception:
                logger.warning("Span exporter %s failed", type(exp).__name__, exc_info=True)


@dataclass
class ExtensionPoint:
    """Describes a named slot where extensions can be registered."""

    name: str
    extension_type: type
    description: str
    multiple: bool


def _built_in_points() -> dict[str, ExtensionPoint]:
    """Return the pre-registered extension points."""
    return {
        "discoverer": ExtensionPoint(
            name="discoverer",
            extension_type=Discoverer,
            description="Custom module discovery strategy",
            multiple=False,
        ),
        "middleware": ExtensionPoint(
            name="middleware",
            extension_type=Middleware,
            description="Execution middleware",
            multiple=True,
        ),
        "acl": ExtensionPoint(
            name="acl",
            extension_type=ACL,
            description="Access control provider",
            multiple=False,
        ),
        "span_exporter": ExtensionPoint(
            name="span_exporter",
            extension_type=SpanExporter,
            description="Tracing span exporter",
            multiple=True,
        ),
        "module_validator": ExtensionPoint(
            name="module_validator",
            extension_type=ModuleValidator,
            description="Custom module validation",
            multiple=False,
        ),
        "approval_handler": ExtensionPoint(
            name="approval_handler",
            extension_type=ApprovalHandler,
            description="Approval handler for Step 4.5 gate",
            multiple=False,
        ),
    }


class ExtensionManager:
    """Manages extension points and their registered implementations.

    Pre-registers built-in extension points: discoverer, middleware,
    acl, span_exporter, module_validator, and approval_handler.
    """

    def __init__(self) -> None:
        self._points: dict[str, ExtensionPoint] = _built_in_points()
        self._extensions: dict[str, list[Any]] = {name: [] for name in self._points}

    def _require_point(self, point_name: str) -> ExtensionPoint:
        """Return the named extension point, or raise if it is not registered.

        D-108: an UNKNOWN point name is an error; an EMPTY point is not. The
        distinction used to be blurred by the contract's ``### Errors: No
        errors raised`` row, which was written about the empty case — one SDK
        read it as covering the unknown case too and answered a misspelled
        name with a silent ``None``/``[]``/``False``, so ``get("middlewares")``
        wired nothing and first surfaced as a bug at ``apply()``.
        """
        point = self._points.get(point_name)
        if point is None:
            raise InvalidInputError(f"Unknown extension point: {point_name!r}. Available: {sorted(self._points)}")
        return point

    def register(self, point_name: str, extension: Any) -> None:
        """Register an extension for the given extension point.

        Args:
            point_name: Name of the extension point (e.g. "middleware").
            extension: The extension instance to register.

        Raises:
            InvalidInputError: If point_name is not a known extension point.
            TypeError: If extension does not satisfy the extension point's type.
        """
        point = self._require_point(point_name)
        if not isinstance(extension, point.extension_type):
            raise TypeError(
                f"Extension for '{point_name}' must be an instance of "
                f"{point.extension_type.__name__}, got {type(extension).__name__}"
            )

        if point.multiple:
            self._extensions[point_name].append(extension)
        else:
            self._extensions[point_name] = [extension]

    def get(self, point_name: str) -> Any | None:
        """Return the single extension for a non-multiple point, or None.

        Args:
            point_name: Name of the extension point.

        Raises:
            InvalidInputError: If point_name is not a known extension point.
                A point that exists but holds nothing returns None (D-108).
        """
        self._require_point(point_name)
        exts = self._extensions[point_name]
        return exts[0] if exts else None

    def get_all(self, point_name: str) -> list[Any]:
        """Return all extensions for a multiple-type point.

        Args:
            point_name: Name of the extension point.

        Raises:
            InvalidInputError: If point_name is not a known extension point.
                A point that exists but holds nothing returns [] (D-108).
        """
        self._require_point(point_name)
        return list(self._extensions[point_name])

    def unregister(self, point_name: str, extension: Any) -> bool:
        """Remove a specific extension from an extension point.

        Removal is by IDENTITY (``is``), not equality (D-128). ``list.remove``
        compares with ``==``, so an extension type that defines ``__eq__`` —
        any dataclass middleware, for one — made this remove the FIRST equal
        registration rather than the object the caller handed back: a host
        removing the second of two identically-configured middlewares kept the
        one it wanted gone and lost the one it wanted kept, silently.
        apcore-typescript compares with ``===`` and apcore-rust by pointer
        address; this contract's own Inputs row says "identity comparison".

        Args:
            point_name: Name of the extension point.
            extension: The extension instance to remove.

        Returns:
            True if the extension was found and removed, False otherwise.

        Raises:
            InvalidInputError: If point_name is not a known extension point.
                An extension the point does not hold returns False (D-108).
        """
        self._require_point(point_name)
        exts = self._extensions[point_name]
        for index, candidate in enumerate(exts):
            if candidate is extension:
                del exts[index]
                return True
        return False

    def list_points(self) -> list[ExtensionPoint]:
        """Return all registered extension points."""
        return list(self._points.values())

    def apply(self, registry: Registry, executor: Executor) -> None:
        """Wire all registered extensions into the given registry and executor.

        Connections:
            - discoverer -> registry.set_discoverer()
            - module_validator -> registry.set_validator()
            - acl -> executor.set_acl()
            - approval_handler -> executor.set_approval_handler()
            - middleware -> executor.use() for each
            - span_exporter -> find TracingMiddleware, set exporter (composite if multiple)

        Args:
            registry: The apcore Registry instance.
            executor: The apcore Executor instance.
        """
        # Discoverer
        discoverer = self.get("discoverer")
        if discoverer is not None:
            registry.set_discoverer(discoverer)

        # Module validator
        validator = self.get("module_validator")
        if validator is not None:
            registry.set_validator(validator)

        # ACL
        acl = self.get("acl")
        if acl is not None:
            executor.set_acl(acl)

        # Approval handler
        approval = self.get("approval_handler")
        if approval is not None:
            executor.set_approval_handler(approval)

        # Middleware
        for mw in self.get_all("middleware"):
            executor.use(mw)

        # Span exporters: find existing TracingMiddleware and set exporter(s)
        exporters = self.get_all("span_exporter")
        if exporters:
            tracing_mw = self._find_tracing_middleware(executor)
            if tracing_mw is not None:
                if len(exporters) == 1:
                    tracing_mw.set_exporter(exporters[0])
                else:
                    tracing_mw.set_exporter(_CompositeExporter(exporters))
            else:
                logger.warning(
                    "span_exporter extensions registered but no TracingMiddleware found in executor middleware chain"
                )

    def _find_tracing_middleware(self, executor: Executor) -> TracingMiddleware | None:
        """Find the first TracingMiddleware in the executor's middleware list."""
        for mw in executor.middlewares:
            if isinstance(mw, TracingMiddleware):
                return mw
        return None


__all__ = ["ExtensionPoint", "ExtensionManager"]
