"""Sensitive field redaction utility (Algorithm A13)."""

from __future__ import annotations

import copy
import logging
import re
from collections.abc import Sequence
from typing import Any, Union

from referencing import Registry, Resource
from referencing.exceptions import Unresolvable
from referencing.jsonschema import DRAFT202012, specification_with

from apcore.utils.pattern import match_glob

logger = logging.getLogger(__name__)

#: One ``regex_patterns`` entry, before or after compilation. A configuration
#: supplies strings; a caller that already compiled them (§10.6.1 requirement 5)
#: supplies patterns, and both reach the same matcher.
ValuePattern = Union[str, "re.Pattern[str]"]

REDACTED_VALUE: str = "***REDACTED***"

# Correlation identifiers that MUST NEVER be redacted, whatever the configured
# ``sensitive_keys`` or ``regex_patterns`` say.
#
#   observability.md § Redaction configuration: "Implementations MUST NOT redact
#   `trace_id`, `caller_id`, `module_id`, or `span_id`; these correlation fields
#   MUST appear unmodified in every log entry."
#
# The rule is unconditional, so the exemption covers BOTH the field-name rule and
# the value-regex rule — a ``trace_id`` whose VALUE happens to match a secret
# regex must still survive, which is exactly where correlation matters most.
# Canonical home: this leaf module, so the executor capture path
# (``redact_sensitive``) and the log-emission path
# (``apcore.observability.context_logger``) share one definition instead of
# guarding at one call site and not the other. Mirrors apcore-rust
# ``NEVER_REDACT_FIELDS`` (src/observability/redaction.rs:18) and
# apcore-typescript ``PROTECTED_LOG_FIELDS``.
PROTECTED_LOG_FIELDS: frozenset[str] = frozenset({"trace_id", "span_id", "caller_id", "module_id", "target_id"})


def _default_sensitive_keys() -> list[str]:
    """Return the spec-default ``obs.redaction.sensitive_keys`` list.

    Imported lazily so this module does not pull in the full Config bus
    on import.  See :mod:`apcore.config` (Issue #43 §5) for the canonical
    list and the matching ``obs.redaction`` namespace registration.
    """
    from apcore.config import _DEFAULT_OBS_REDACTION_SENSITIVE_KEYS

    return list(_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS)


def redact_sensitive(
    data: dict[str, Any],
    schema_dict: dict[str, Any],
    *,
    sensitive_keys: list[str] | None = None,
    regex_patterns: Sequence[ValuePattern] | None = None,
    replacement: str | None = None,
) -> dict[str, Any]:
    """Redact fields marked with x-sensitive in the schema.

    Implements Algorithm A13 from PROTOCOL_SPEC section 9.5.
    Returns a deep copy of data with sensitive values replaced by
    ``replacement`` (default ``"***REDACTED***"``).

    Issue #43 §5: in addition to schema-level ``x-sensitive`` annotations
    and the legacy ``_secret_*`` prefix, fields whose names match any
    pattern in ``sensitive_keys`` (case-insensitive substring or
    a glob-dialect pattern, Algorithm A25) are redacted, and string values matching one of
    ``regex_patterns`` (compiled with :data:`re.IGNORECASE`) are
    redacted.  When ``sensitive_keys`` is ``None`` the spec-default list
    (which still includes ``_secret_*``) is used.

    Args:
        data: The data dict to redact.
        schema_dict: A JSON Schema dict that may contain ``x-sensitive: true``
            on individual properties, including inside ``anyOf`` / ``oneOf`` /
            ``allOf`` branches (D-152) and local references/anchors. Reference
            cycles terminate without expanding the schema or retrieving URLs.
        sensitive_keys: Optional override for the field-name match list.
        regex_patterns: Optional list of regex patterns matched against
            **string** values (case-insensitive, unanchored search). Already
            compiled patterns are accepted and pass through uncompiled, so a
            caller holding a ``RedactionConfig`` avoids recompiling per call
            (PROTOCOL_SPEC 10.6.1 requirement 5). Non-string values are never
            tested and never stringified (requirement 2).
        replacement: Optional replacement token; defaults to
            :data:`REDACTED_VALUE`.

    Returns:
        A new dict with sensitive values replaced. Original data is not modified.
    """
    keys = sensitive_keys if sensitive_keys is not None else _default_sensitive_keys()
    # Compiled ONCE here rather than at every node of the walk below
    # (PROTOCOL_SPEC 10.6.1 requirement 5), which is also what keeps the
    # requirement-4 diagnostic to one line per call instead of one per field.
    value_regexes = compile_value_regexes(regex_patterns)[0] if regex_patterns else []
    token = replacement if replacement is not None else REDACTED_VALUE

    redacted = copy.deepcopy(data)
    _redact_fields(redacted, schema_dict, token=token)
    _redact_by_keys_and_regex(redacted, keys, value_regexes, token)
    return redacted


_COMBINATORS: tuple[str, ...] = ("anyOf", "oneOf", "allOf")


class _SchemaRedactor:
    """Walk data and local schema references together, without schema expansion."""

    def __init__(self, schema: dict[str, Any], token: str) -> None:
        self._token = token
        declared_dialect = schema.get("$schema")
        self._root_dialect: str = (
            declared_dialect if isinstance(declared_dialect, str) else "https://json-schema.org/draft/2020-12/schema"
        )
        self._root_specification = specification_with(self._root_dialect, default=DRAFT202012)
        resource = Resource.from_contents(schema, default_specification=self._root_specification)
        self._resources: dict[int, Resource[Any]] = {}
        self._dialects: dict[int, str] = {}
        self._index_resources(resource, self._root_dialect)
        self._resolver = Registry().resolver_with_root(resource)

    def _index_resources(self, root: Resource[Any], inherited_dialect: str) -> None:
        pending = [(root, inherited_dialect)]
        while pending:
            resource, parent_dialect = pending.pop()
            contents = resource.contents
            if not isinstance(contents, dict) or id(contents) in self._resources:
                continue
            declared_dialect = contents.get("$schema")
            dialect = declared_dialect if isinstance(declared_dialect, str) else parent_dialect
            self._resources[id(contents)] = resource
            self._dialects[id(contents)] = dialect
            pending.extend((child, dialect) for child in resource.subresources())

    def _resource(self, schema: dict[str, Any]) -> Resource[Any]:
        resource = self._resources.get(id(schema))
        if resource is not None:
            return resource
        return Resource.from_contents(schema, default_specification=self._root_specification)

    def _views(self, schema: dict[str, Any], resolver: Any) -> list[tuple[dict[str, Any], Any]]:
        # Resolver is a dialect-dependent third-party type; validate its contents.
        pending = [(schema, resolver)]
        views: list[tuple[dict[str, Any], Any]] = []
        seen: set[int] = set()
        while pending:
            current, context = pending.pop()
            if id(current) in seen:
                continue
            seen.add(id(current))
            views.append((current, context))
            reference = current.get("$ref")
            if isinstance(reference, str):
                try:
                    resolved = context.lookup(reference)
                    if isinstance(resolved.contents, dict):
                        pending.append((resolved.contents, resolved.resolver))
                except Unresolvable:
                    pass  # Unknown references never trigger network or file retrieval.
            for keyword in _COMBINATORS:
                branches = current.get(keyword)
                if isinstance(branches, list):
                    for branch in branches:
                        if isinstance(branch, dict):
                            pending.append((branch, context.in_subresource(self._resource(branch))))
        return views

    def _redact_value(self, value: Any, schema: dict[str, Any], resolver: Any) -> Any:
        views = self._views(schema, resolver)
        if value is not None and any(view.get("x-sensitive") is True for view, _ in views):
            return self._token
        if isinstance(value, dict):
            self._redact_object(value, views)
        elif isinstance(value, list):
            for view, context in views:
                self._redact_array(value, view, context)
        return value

    def _redact_array(self, values: list[Any], schema: dict[str, Any], resolver: Any) -> None:
        items = schema.get("items")
        dialect = self._dialects.get(id(schema), self._root_dialect)
        legacy = "/draft-07/" in dialect or "/2019-09/" in dialect
        tuple_items = items if legacy and isinstance(items, list) else None
        prefix = tuple_items if tuple_items is not None else (schema.get("prefixItems") if not legacy else None)
        prefix = prefix if isinstance(prefix, list) else []
        for index, positional in enumerate(prefix[: len(values)]):
            if isinstance(positional, dict):
                context = resolver.in_subresource(self._resource(positional))
                values[index] = self._redact_value(values[index], positional, context)
        tail = schema.get("additionalItems") if tuple_items is not None else items
        if isinstance(tail, dict):
            context = resolver.in_subresource(self._resource(tail))
            for index in range(len(prefix), len(values)):
                values[index] = self._redact_value(values[index], tail, context)

    def _redact_object(self, data: dict[str, Any], views: list[tuple[dict[str, Any], Any]]) -> None:
        for schema, resolver in views:
            properties = schema.get("properties")
            if not isinstance(properties, dict):
                continue
            for name, field_schema in properties.items():
                if name in data and isinstance(field_schema, dict):
                    child = resolver.in_subresource(self._resource(field_schema))
                    data[name] = self._redact_value(data[name], field_schema, child)

    def redact_fields(self, data: dict[str, Any], schema: dict[str, Any]) -> None:
        self._redact_object(data, self._views(schema, self._resolver))


def _redact_fields(
    data: dict[str, Any],
    schema_dict: dict[str, Any],
    *,
    token: str = REDACTED_VALUE,
) -> None:
    """Redact markers across composition and cycle-safe local references."""
    _SchemaRedactor(schema_dict, token).redact_fields(data, schema_dict)


def _normalize_for_match(s: str) -> str:
    """Lower-case with ``-`` / ``_`` / spaces collapsed to ``_``.

    Lets ``"X-API-Key"`` match the ``"api_key"`` substring as required
    by the Issue #43 §5 spec example.  Glob patterns are NOT normalized —
    they go through :func:`apcore.utils.pattern.match_glob` instead, with the
    case fold applied to the pattern and the key alike (PROTOCOL_SPEC
    §10.6.1).
    """
    return s.lower().replace("-", "_").replace(" ", "_")


def _compact_for_match(s: str) -> str:
    """Lower-case with ``-`` / ``_`` / space stripped entirely.

    Allows camelCase keys like ``"AccessKey"`` to match the ``"access_key"``
    substring (D-54 canonical default list expects this).
    """
    return s.lower().replace("-", "").replace("_", "").replace(" ", "")


def _key_matches(key: str, sensitive_keys: list[str]) -> bool:
    """Return True if *key* matches any entry in ``sensitive_keys``.

    Each pattern is interpreted as either:

    - a glob-dialect pattern (Algorithm A25) when it contains ``*`` or ``?``,
      matched case-insensitively and anchored to the whole name,
    - or a plain case-insensitive substring match otherwise.  Hyphen,
      underscore, and space are treated as equivalent on both sides so
      ``"X-API-Key"`` matches ``"api_key"`` (Issue #43 §5).  The match also
      collapses separators to allow camelCase keys (``AccessKey``) to match
      snake_case patterns (``access_key``).

    Canonical home for the §10.6.1 key rule, which MUST hold identically on
    **both** mandated surfaces — the executor's input/output capture point
    (:func:`redact_sensitive`) and log emission
    (:mod:`apcore.observability.context_logger`, which imports this).  The two
    surfaces used to carry separate copies; the sibling value matcher did too,
    and the copies drifted into giving one SDK two answers for one value
    (see :func:`_value_matches`).
    """
    if not sensitive_keys:
        return False
    norm_key = _normalize_for_match(key)
    compact_key = _compact_for_match(key)
    lower_key = key.lower()
    for pat in sensitive_keys:
        if not pat:
            continue
        lower_pat = pat.lower()
        # PROTOCOL_SPEC 10.6.1: an entry containing `*` or `?` is a glob-dialect
        # pattern (A25, anchored); anything else is a substring. `[` is NOT a
        # trigger — brackets are literals under A25 (9.2.3 requirement 4), and
        # reading them as a class is what let apcore-typescript invert
        # `[!p]assword` into "redact password" (#117). The fold is applied to
        # BOTH sides: folding the key alone left `"*Token*"` matching nothing
        # in apcore-rust, silently.
        if "*" in lower_pat or "?" in lower_pat:
            if match_glob(lower_pat, lower_key):
                return True
        else:
            if _normalize_for_match(pat) in norm_key:
                return True
            # Also match the compact (separator-stripped) form so that
            # camelCase keys like "AccessKey" match the "access_key" pattern.
            if _compact_for_match(pat) in compact_key:
                return True
    return False


def _value_matches(value: Any, value_regexes: Sequence[re.Pattern[str]]) -> bool:
    """Case-insensitive regex search over *value*, when *value* is a string.

    PROTOCOL_SPEC §10.6.1 requirement 2: the value rule applies to **string
    values only**, and a non-string value **MUST NOT** be converted to a string
    in order to test it.  The conversion is not merely unnecessary, it is
    unspecifiable — ``{"a": 1}`` renders as ``{'a': 1}`` in Python, ``[object
    Object]`` in TypeScript and ``{"a":1}`` in Rust, so a rule defined over the
    rendering is three rules.  Containers are descended into by the callers
    instead, so a string *inside* one is still reached at its own position.

    Takes patterns already compiled by :func:`compile_value_regexes`, because
    this runs at every node of the walk and requirement 5 puts compilation at
    the configuration read.
    """
    if not isinstance(value, str) or not value_regexes:
        return False
    for pat in value_regexes:
        if pat.search(value) is not None:
            return True
    return False


def compile_value_regexes(
    patterns: Sequence[ValuePattern],
) -> tuple[list[re.Pattern[str]], list[tuple[str, str]]]:
    """Compile ``obs.redaction.regex_patterns`` once, reporting what will not.

    PROTOCOL_SPEC §10.6.1 requirement 4: a pattern the engine cannot compile
    **MUST NOT** be discarded in silence.  It was — a bare ``except re.error:
    continue`` that re-failed on every log line and said nothing, leaving an
    operator-authored redaction rule that redacts nothing and looks, from the
    outside, exactly like one that works.  On this surface the difference is
    credentials in plaintext (#117 §2).

    Requirement 5 is why this is a batch function rather than a per-pattern
    one, and why it holds no module-level state.  Compilation belongs at the
    point the *configuration* is read, so the diagnostic fires at load rather
    than per log record — and the de-duplication that keeps it to one line
    **MUST NOT outlive that configuration**.  The set this replaced was a
    process-global, never cleared, so the *second* deployment to load the same
    broken pattern was told nothing: precisely the reload case and the
    multi-tenant case, the two where an operator most needs telling.

    Already-compiled patterns pass through, so a caller holding a
    :class:`~apcore.observability.context_logger.RedactionConfig` hands its
    compiled list straight in and nothing is recompiled per record.

    Returns ``(compiled, invalid)`` where ``invalid`` pairs each rejected
    pattern with the engine's message, for ``validate_config()`` to report.
    """
    compiled: list[re.Pattern[str]] = []
    invalid: list[tuple[str, str]] = []
    for pat in patterns:
        if isinstance(pat, re.Pattern):
            compiled.append(pat)
            continue
        if not pat:
            continue
        try:
            compiled.append(re.compile(pat, flags=re.IGNORECASE))
        except re.error as exc:
            invalid.append((pat, str(exc)))
            logger.warning(
                "obs.redaction.regex_patterns entry %r does not compile and will "
                "redact nothing: %s. Patterns should stay inside the portable "
                "subset (no lookaround, no backreferences, no inline (?i) flags) "
                "— see PROTOCOL_SPEC 9.2.3 requirement 6.",
                pat,
                exc,
            )
    return compiled, invalid


def _redact_by_keys_and_regex(
    data: dict[str, Any],
    sensitive_keys: list[str],
    value_regexes: Sequence[re.Pattern[str]],
    token: str,
) -> None:
    """In-place redaction by name (substring/glob) or value (regex), at any depth.

    Replaces the legacy ``_redact_secret_prefix`` walk (Issue #43 §5).
    The default ``sensitive_keys`` list still contains ``_secret_*`` so
    every existing call site retains its prior behaviour.

    Keys in :data:`PROTECTED_LOG_FIELDS` are exempt from both rules at every
    depth, matching apcore-rust's per-entry ``NEVER_REDACT_FIELDS`` check.
    """
    for key in data:
        value = data[key]
        if value is None:
            continue
        # A correlation identifier is exempt from BOTH rules below. Note the
        # guard does not stop the walk: a container under a protected key is
        # still descended into, only the protected field's own scalar value is
        # immune (apcore-rust `redact_inner`: "Containers below a protected key
        # are still descended into ... only the protected field's own scalar
        # value is immune").
        protected = key in PROTECTED_LOG_FIELDS
        if not protected and _key_matches(key, sensitive_keys):
            data[key] = token
            continue
        if not protected and _value_matches(value, value_regexes):
            data[key] = token
            continue
        if isinstance(value, dict):
            _redact_by_keys_and_regex(value, sensitive_keys, value_regexes, token)
        elif isinstance(value, list):
            _redact_in_list(value, sensitive_keys, value_regexes, token)


def _redact_in_list(
    items: list[Any],
    sensitive_keys: list[str],
    value_regexes: Sequence[re.Pattern[str]],
    token: str,
) -> None:
    """Traverse a list, redacting dict children and recursing into nested lists."""
    for index, item in enumerate(items):
        if item is None:
            continue
        if isinstance(item, dict):
            _redact_by_keys_and_regex(item, sensitive_keys, value_regexes, token)
        elif isinstance(item, list):
            _redact_in_list(item, sensitive_keys, value_regexes, token)
        elif _value_matches(item, value_regexes):
            items[index] = token
