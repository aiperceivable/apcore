"""Context-data namespace validation (Issue #42).

Spec reference: ``features/middleware-system.md`` § "Context Namespacing",
pinned by ``conformance/fixtures/middleware_hardening.json`` cases
``context_namespace_apcore_prefix`` / ``context_namespace_ext_prefix`` /
``context_namespace_violation``.

Two reserved prefixes partition ``Context.data``:

``_apcore.*``
    Owned by framework middleware — e.g. ``_apcore.mw.logging.start_time``,
    ``_apcore.mw.tracing.spans``, ``_apcore.mw.circuit.state``. The canonical
    list of framework-owned keys is :mod:`apcore.context_keys`; this module
    validates the *prefix* rules and deliberately keeps no second copy of that
    list.
``ext.*``
    Owned by user-defined middleware — e.g. ``ext.my_company.request_id``.

Keys with neither prefix are tolerated for backward compatibility but SHOULD be
migrated. A ``user`` writer touching an ``_apcore.*`` key, or a ``framework``
writer touching an ``ext.*`` key, is a namespace violation.

Cross-language parity: apcore-typescript ``validateContextKey``
(``src/middleware/context-namespace.ts``) and apcore-rust
``validate_context_key`` (``src/middleware/context_namespace.rs``). All three
return the same ``{valid, warning}`` pair for the same ``(writer, key)``.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Literal

__all__ = [
    "APCORE_KEY_PREFIX",
    "EXT_KEY_PREFIX",
    "ContextWriter",
    "NamespaceCheck",
    "validate_context_key",
    "enforce_context_key",
]

_logger = logging.getLogger(__name__)

#: Reserved prefix for framework-owned ``context.data`` keys.
APCORE_KEY_PREFIX = "_apcore."
#: Reserved prefix for user-extension ``context.data`` keys.
EXT_KEY_PREFIX = "ext."

#: The party performing a ``context.data`` write.
ContextWriter = Literal["framework", "user"]


@dataclass(frozen=True)
class NamespaceCheck:
    """Outcome of a namespace validation check on a ``context.data`` write."""

    #: True when the write conforms to the namespace rules.
    valid: bool
    #: True when the caller should log a warning. Currently set iff ``not valid``.
    warning: bool


def validate_context_key(writer: ContextWriter, key: str) -> NamespaceCheck:
    """Validate a ``context.data`` write against the spec namespace rules.

    Pure: never logs, never raises. Callers (framework middleware or user
    middleware) SHOULD call this before writing and emit a warning when
    ``warning`` is True. :func:`enforce_context_key` wraps that pattern.

    Args:
        writer: ``"framework"`` for built-in/framework-shipped middleware,
            ``"user"`` for user-supplied middleware or extensions.
        key: The ``context.data`` key about to be written.

    Returns:
        A :class:`NamespaceCheck` with ``valid``/``warning`` flags.
    """
    in_apcore = key.startswith(APCORE_KEY_PREFIX)
    in_ext = key.startswith(EXT_KEY_PREFIX)
    valid = not in_ext if writer == "framework" else not in_apcore
    return NamespaceCheck(valid=valid, warning=not valid)


def enforce_context_key(writer: ContextWriter, key: str) -> NamespaceCheck:
    """Validate and, when invalid, log a warning describing the violation.

    Returns the same :class:`NamespaceCheck` as :func:`validate_context_key` so
    callers may decide whether to skip the write. Parity with apcore-rust
    ``enforce_context_key``.
    """
    check = validate_context_key(writer, key)
    if check.warning:
        if writer == "user":
            _logger.warning(
                "User middleware wrote to reserved '%s*' namespace (key=%r); "
                "framework-owned keys must not be set by user code",
                APCORE_KEY_PREFIX,
                key,
            )
        else:
            _logger.warning(
                "Framework middleware wrote to user '%s*' namespace (key=%r); "
                "user-extension keys must not be set by framework code",
                EXT_KEY_PREFIX,
                key,
            )
    return check
