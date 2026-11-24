"""Contextual identity extraction helpers for audit-event payloads (Issue #45.2).

Lives in its own module so both ``apcore.registry.registry`` (ephemeral
audit events) and ``apcore.sys_modules.control`` (system.control.*
events) can share the redaction rules without forming an import cycle.
"""

from __future__ import annotations

from typing import Any

#: Identity ``attrs`` whose names match one of these case-insensitive
#: substrings are replaced with the literal string ``"<redacted>"`` in
#: audit-event payloads (Issue #45.2).  This list is intentionally a
#: superset of the canonical ``obs.redaction.sensitive_keys`` so that
#: bearer tokens, signed cookies, and credentials can never leak through
#: the contextual-audit channel even when redaction is disabled globally.
_IDENTITY_SENSITIVE_SUBSTRINGS: tuple[str, ...] = (
    "token",
    "secret",
    "password",
    "passwd",
    "key",
    "auth",
    "credential",
    "cookie",
    "session",
    "bearer",
)

_IDENTITY_REDACTED_TOKEN: str = "<redacted>"


def _extract_caller_id(context: Any) -> str:
    """Return ``context.caller_id`` if set, else the spec sentinel ``@external``."""
    if context is None:
        return "@external"
    caller_id = getattr(context, "caller_id", None)
    if not caller_id:
        return "@external"
    return str(caller_id)


def _redact_identity_attr(name: str, value: Any) -> Any:
    """Replace ``value`` with ``<redacted>`` when ``name`` looks sensitive."""
    lower = name.lower()
    for substr in _IDENTITY_SENSITIVE_SUBSTRINGS:
        if substr in lower:
            return _IDENTITY_REDACTED_TOKEN
    return value


def _extract_identity_dict(context: Any) -> dict[str, Any] | None:
    """Return a redacted-safe dict view of ``context.identity`` or ``None``.

    The dict always includes ``id`` and ``type`` plus any non-empty
    ``roles`` list.  Free-form ``attrs`` (e.g. ``display_name``,
    ``bearer_token``) are surfaced verbatim *except* when their key name
    matches a sensitive substring (``token``, ``secret``, ``password``,
    ``key``, ``auth``, ``credential``, ``cookie``, ``session``,
    ``bearer``), in which case the value is replaced with the literal
    string ``"<redacted>"``.  Callers that need the unredacted identity
    should consult the ``AuditStore`` entry, not the event payload.
    """
    if context is None:
        return None
    identity = getattr(context, "identity", None)
    if identity is None:
        return None
    snapshot: dict[str, Any] = {
        "id": getattr(identity, "id", None),
        "type": getattr(identity, "type", None),
    }
    roles = list(getattr(identity, "roles", ()) or ())
    if roles:
        snapshot["roles"] = roles
    attrs = getattr(identity, "attrs", None) or {}
    if isinstance(attrs, dict):
        for key, value in attrs.items():
            # Don't shadow the canonical id/type/roles keys.
            if key in snapshot:
                continue
            snapshot[key] = _redact_identity_attr(key, value)
    return snapshot


def _audit_payload_extras(context: Any) -> dict[str, Any]:
    """Build the ``caller_id`` (+ optional ``identity``) fragment for audit events."""
    extras: dict[str, Any] = {"caller_id": _extract_caller_id(context)}
    identity_dict = _extract_identity_dict(context)
    if identity_dict is not None:
        extras["identity"] = identity_dict
    return extras
