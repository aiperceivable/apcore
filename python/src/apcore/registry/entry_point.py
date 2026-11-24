"""Entry point resolution for discovered module files."""

from __future__ import annotations

import importlib.util
import inspect
from pathlib import Path
from typing import Any, Callable

from pydantic import BaseModel

from apcore.errors import ModuleLoadError

__all__ = ["resolve_entry_point", "PreApprovalHook"]


PreApprovalHook = Callable[[Path], None]
"""Signature for a caller-supplied hook invoked before ``exec_module``.

The hook receives the absolute path of a Python file that the loader is
about to execute. It may:

- ``return`` to approve the load; or
- ``raise`` any exception to reject the load (the exception is wrapped in
  ``ModuleLoadError`` with the original as ``__cause__``).

Typical uses: signature verification, path allowlists, content hashing,
audit logging. The hook runs synchronously inside discovery, so keep it
fast; expensive checks should be cached.
"""


def snake_to_pascal(name: str) -> str:
    """Convert a snake_case string to PascalCase."""
    if not name:
        return ""
    return "".join(part.capitalize() for part in name.split("_"))


def _is_module_class(cls: type, loaded_module_name: str) -> bool:
    """Check if a class looks like an apcore module (duck-type detection)."""
    if cls.__module__ != loaded_module_name:
        return False
    input_schema = getattr(cls, "input_schema", None)
    output_schema = getattr(cls, "output_schema", None)
    if input_schema is None or output_schema is None:
        return False
    if not (inspect.isclass(input_schema) and issubclass(input_schema, BaseModel)):
        return False
    if not (inspect.isclass(output_schema) and issubclass(output_schema, BaseModel)):
        return False
    if not callable(getattr(cls, "execute", None)):
        return False
    return True


def _import_module_from_file(
    file_path: Path,
    pre_approval_hook: PreApprovalHook | None = None,
) -> Any:
    """Dynamically import a Python file and return the loaded module object.

    .. warning::
        **Trust boundary.** This function executes arbitrary Python code from
        ``file_path`` via ``spec.loader.exec_module``. Operators must treat
        ``extensions_root`` (the directory the scanner walks) as an execution
        privilege — anything placed there runs with full Python permissions.
        Recommended hardening:

        - Keep ``follow_symlinks=False`` (the default) unless extension paths
          are fully under team control.
        - Restrict write access to the extensions directory to the same
          principals that can deploy code.
        - If extensions come from untrusted sources, run the framework in a
          sandbox (container, separate UID, seccomp).
        - Pass a ``pre_approval_hook`` that verifies signatures / hashes /
          allowlists before each file is imported.
    """
    if pre_approval_hook is not None:
        try:
            pre_approval_hook(file_path)
        except Exception as exc:
            raise ModuleLoadError(
                module_id=str(file_path),
                reason=f"Pre-approval hook rejected {file_path}: {exc}",
            ) from exc

    module_name = f"apcore_ext_{file_path.stem}"
    spec = importlib.util.spec_from_file_location(module_name, str(file_path))
    if spec is None or spec.loader is None:
        raise ModuleLoadError(
            module_id=str(file_path),
            reason=f"Cannot create import spec for {file_path}",
        )

    mod = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(mod)
    except Exception as exc:
        raise ModuleLoadError(module_id=str(file_path), reason=f"Failed to import module: {exc}") from exc
    return mod


def resolve_entry_point(
    file_path: Path,
    meta: dict[str, Any] | None = None,
    pre_approval_hook: PreApprovalHook | None = None,
) -> type:
    """Resolve the module class from a discovered Python file.

    If meta contains an 'entry_point' key in format 'filename:ClassName',
    loads that specific class. Otherwise auto-infers the single module class.

    When ``pre_approval_hook`` is supplied, it is invoked with ``file_path``
    before ``spec.loader.exec_module`` runs; raising from the hook rejects
    the load (wrapped as ``ModuleLoadError``).
    """
    loaded = _import_module_from_file(file_path, pre_approval_hook=pre_approval_hook)

    # Meta override mode
    if meta and "entry_point" in meta:
        class_name = meta["entry_point"].split(":")[-1]
        cls = getattr(loaded, class_name, None)
        if cls is None:
            raise ModuleLoadError(
                module_id=str(file_path),
                reason=f"Entry point class '{class_name}' not found in {file_path.name}",
            )
        return cls

    # Auto-infer mode
    candidates = [
        cls for _, cls in inspect.getmembers(loaded, inspect.isclass) if _is_module_class(cls, loaded.__name__)
    ]

    if len(candidates) == 1:
        return candidates[0]
    elif len(candidates) == 0:
        raise ModuleLoadError(module_id=str(file_path), reason="No Module subclass found in file")
    else:
        raise ModuleLoadError(
            module_id=str(file_path),
            reason="Ambiguous entry point: multiple Module subclasses found",
        )
