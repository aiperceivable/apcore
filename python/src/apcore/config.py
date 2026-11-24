"""Configuration loading, validation, and environment variable overrides (Algorithm A12).

0.15.0 additions: Config Bus (§9.4–§9.15) — namespace registry, namespace mode,
mount(), namespace(), get_typed(), bind(), per-namespace env overrides, hot-reload.
"""

from __future__ import annotations

import copy
import dataclasses
import logging
import os

import sys
import threading
import warnings
from pathlib import Path
from collections.abc import Sequence
from typing import Any, Type, TypeVar

import yaml

from apcore.errors import (
    ConfigBindError,
    ConfigEnvMapConflictError,
    ConfigEnvPrefixConflictError,
    ConfigError,
    ConfigMountError,
    ConfigNamespaceDuplicateError,
    ConfigNamespaceReservedError,
    ConfigNotFoundError,
)

from apcore.version import _negotiate_document_version

__all__ = ["Config", "discover_config_file"]

_logger = logging.getLogger(__name__)

#: Environment variable prefix for overrides.
_ENV_PREFIX = "APCORE_"
#: The `apcore` namespace's env prefix; no other namespace may register it (§9.8.2 rule 3).
_APCORE_ENV_PREFIX = "APCORE"

#: Environment variable naming the configuration file to load (§9.14 discovery).
#:
#: apcore#88: this variable is an *argument to* ``Config.load()`` — it selects
#: which document is read — and only happens to share the ``APCORE_`` prefix
#: that §9.2 turns into configuration overrides. Left in the override map its
#: suffix becomes the dot-path ``config.file``, a key no schema declares
#: (checked against ``conformance/fixtures/config_key_governance.json``), which
#: then sits inside the *declared* document the §9.1 required-field check runs
#: against. ``discover_config_file()`` consumes it; the override pass drops it.
_ENV_CONFIG_FILE = "APCORE_CONFIG_FILE"

#: Required configuration fields (dot-paths).
#:
#: PROTOCOL_SPEC §9.1: a key is required *only* when it has no canonical
#: default — that is, only when omitting it leaves a value the framework cannot
#: supply. Exactly two qualify: ``version`` and ``project.name``. Every other
#: key in §9.1 carries a default in ``schemas/defaults.schema.json`` (mirrored
#: by ``_DEFAULTS`` below), so requiring it would reject a document the
#: framework resolves perfectly well. That is why ``extensions.root``,
#: ``schema.root``, ``acl.root`` and ``acl.default_effect`` are no longer listed
#: here, matching ``schemas/apcore-config.schema.json``'s
#: ``required: ["version", "project"]``.
#:
#: These are checked against the DECLARED document (§9.3 step 1), never the
#: merged one — see :meth:`Config.validate`.
_REQUIRED_FIELDS: tuple[str, ...] = (
    "version",
    "project.name",
)

#: Field constraints: field → (validator_fn, error_message).
#:
#: Every numeric check carries ``not isinstance(v, bool)``. ``bool`` subclasses
#: ``int`` in Python, so ``isinstance(True, int)`` is True and a plain
#: ``isinstance(v, int) and v >= 1`` read ``max_call_depth: true`` as a depth
#: limit of 1 rather than rejecting it. ``docs/features/config-bus.md`` states
#: "Booleans are rejected for all numeric fields", and apcore-typescript and
#: apcore-rust — whose native booleans are not integers — reject them for free.
_CONSTRAINTS: dict[str, tuple[Any, str]] = {
    "acl.default_effect": (
        lambda v: v in ("allow", "deny"),
        "must be 'allow' or 'deny'",
    ),
    "observability.tracing.sampling_rate": (
        lambda v: isinstance(v, (int, float)) and not isinstance(v, bool) and 0.0 <= v <= 1.0,
        "must be a number in [0.0, 1.0]",
    ),
    "observability.tracing.strategy": (
        lambda v: v in ("full", "proportional", "error_first", "off"),
        "must be 'full', 'proportional', 'error_first' or 'off'",
    ),
    "observability.tracing.exporter": (
        lambda v: v in ("stdout", "otlp", "jaeger"),
        "must be 'stdout', 'otlp' or 'jaeger'",
    ),
    "observability.tracing.otlp_endpoint": (
        lambda v: v is None or (isinstance(v, str) and bool(v.strip())),
        "must be a non-empty URL string, or null",
    ),
    "extensions.max_depth": (
        lambda v: isinstance(v, int) and not isinstance(v, bool) and 1 <= v <= 16,
        "must be an integer in [1, 16]",
    ),
    "executor.default_timeout": (
        lambda v: isinstance(v, int) and not isinstance(v, bool) and v >= 0,
        "must be a non-negative integer (milliseconds)",
    ),
    "executor.global_timeout": (
        lambda v: isinstance(v, int) and not isinstance(v, bool) and v >= 0,
        "must be a non-negative integer (milliseconds)",
    ),
    "executor.max_call_depth": (
        lambda v: isinstance(v, int) and not isinstance(v, bool) and v >= 1,
        "must be a positive integer",
    ),
    "executor.max_module_repeat": (
        lambda v: isinstance(v, int) and not isinstance(v, bool) and v >= 1,
        "must be a positive integer",
    ),
    "sys_modules.error_history.max_entries_per_module": (
        lambda v: isinstance(v, int) and not isinstance(v, bool) and v >= 1,
        "must be a positive integer",
    ),
    "sys_modules.error_history.max_total_entries": (
        lambda v: isinstance(v, int) and not isinstance(v, bool) and v >= 1,
        "must be a positive integer",
    ),
    "sys_modules.events.thresholds.error_rate": (
        lambda v: isinstance(v, (int, float)) and not isinstance(v, bool) and 0.0 <= v <= 1.0,
        "must be a number in [0.0, 1.0]",
    ),
    "sys_modules.events.thresholds.latency_p99_ms": (
        lambda v: isinstance(v, (int, float)) and not isinstance(v, bool) and v > 0,
        "must be a positive number",
    ),
}

#: Default configuration values.
#:
#: This table mirrors ``schemas/defaults.schema.json``, the canonical source of
#: truth shared by every SDK. It therefore declares no ``version`` and no
#: ``project`` subtree: those two have no canonical default (PROTOCOL_SPEC
#: §9.1), which is precisely what makes them the only required keys. Inventing
#: defaults for them here — a frozen ``version: "0.16.0"`` and
#: ``project.name: "apcore"`` — silently satisfied ``_REQUIRED_FIELDS`` for
#: every document, turning the required-field check into dead code.
#: Consumers that want a fallback for an undeclared ``project.*`` value pass it
#: explicitly at the call site (``config.get("project.source_root", "")``).
_DEFAULTS: dict[str, Any] = {
    "extensions": {
        "root": "./extensions",
        "auto_discover": True,
        "max_depth": 8,
        "follow_symlinks": False,
    },
    "schema": {
        "root": "./schemas",
        "strategy": "yaml_first",
        "max_ref_depth": 32,
    },
    "acl": {
        "root": "./acl",
        "default_effect": "deny",
    },
    "executor": {
        "default_timeout": 30000,
        "global_timeout": 60000,
        "max_call_depth": 32,
        "max_module_repeat": 3,
    },
    "observability": {
        "tracing": {
            "enabled": False,
            "sampling_rate": 1.0,
            "strategy": "full",
            "exporter": "stdout",
            "otlp_endpoint": None,
        },
        "metrics": {
            "enabled": False,
        },
    },
    # Only `enabled`, because that is all `schemas/defaults.schema.json`
    # declares for this section — the same rule `project` is held to below.
    # The other thirteen `sys_modules` defaults live in
    # `schemas/sys-modules.schema.json` and reach a Config through the
    # namespace registration at the bottom of this module, which is the layer
    # §9.15.3 gives that namespace. Carrying them here as well made
    # `get("sys_modules.error_history.max_entries_per_module")` answer 50 in
    # legacy mode while apcore-typescript and apcore-rust answered
    # undefined/None over the same call (sync finding A-D-021). No behaviour
    # depended on the difference: every consumer in `sys_modules/registration.py`
    # passes the schema default as its own fallback.
    "sys_modules": {
        "enabled": False,
    },
    "stream": {
        "max_merge_depth": 32,
    },
    # PROTOCOL_SPEC §9.1.2 (spec v1.38.0, apcore#118). The six `validation.*`
    # keys are UNCONSTRAINED by default — apcore does not impose limits on the
    # content its users author, it offers them. Five of the six are unconstrained
    # AS `None`, and `None` is the absence of a default, so only this one carries
    # a canonical value: `defaults.schema.json` declares exactly this entry and
    # `config_key_governance.json` pins the table to that file.
    "validation": {
        "binding": {
            "version_require_semver": False,
        },
    },
    # Added by spec v1.36.0 (apcore#114). `defaults.schema.json` gained a
    # `bindings` section so that the `./bindings` default §5.12.6 promises is
    # reachable through the mechanism meant to serve it; before that the two
    # values existed only as literals inside each SDK's binding loader, which
    # made `config.get("bindings.dir")` answer None for a fully loaded Config
    # and put the canonical default out of reach of every consumer that asks
    # the configuration system rather than the loader.
    "bindings": {
        "dir": "./bindings",
        "pattern": "*.binding.yaml",
    },
}

#: Framework sections of the ``apcore`` namespace, mapped to the keys each one
#: declares (PROTOCOL_SPEC §9.14 ``reject_unknown_framework_keys``).
#:
#: Every section in ``schemas/apcore-config.schema.json`` is
#: ``additionalProperties: false``; this table is the projection of that
#: closedness the runtime can consult, since the schema files ship with the
#: spec repo rather than with this package. It is *derived*, not authored:
#: ``tests/conformance/test_config_key_governance.py`` rebuilds it from the
#: canonical schema — resolving ``$ref`` and the ``oneOf`` branches
#: ``ExtensionsConfig`` splits ``root``/``roots`` across — and fails on any
#: difference, so a section added to the schema breaks a test here instead of
#: going silently unenforced. Regenerate rather than hand-edit.
#:
#: ``sys_modules`` unions in ``schemas/sys-modules.schema.json``: §9.15.3
#: registers it as a namespace and declares its subsections there, while
#: ``SysModulesConfig`` in apcore-config.schema.json stops at ``enabled``.
#: Taking the narrower of the two would reject ``sys_modules.usage`` — a key
#: the canonical surface declares — for anyone who opts into strict.
#:
#: The check is one level deep, matching §9.14 step 2 ("each key present in
#: ``apcore_data[section]``"), so only each section's direct child names are
#: needed here.
#: The closed set of **path-typed** configuration keys — those whose value is a
#: filesystem path (PROTOCOL_SPEC §9.2.1). Declared canonically by
#: ``"x-apcore-path": true`` in ``schemas/apcore-config.schema.json``; this tuple
#: is that projection.
#:
#: It exists for consumers outside this SDK. Anything that forwards apcore
#: configuration across a process boundary — a CLI spawning a worker, a
#: supervisor building a container environment — has to know which ``APCORE_*``
#: variables carry paths, because a relative value silently re-roots wherever the
#: working directory differs. Without a published set each such consumer builds
#: its own and drifts.
#:
#: ``bindings.pattern`` is deliberately excluded: it is a glob matched against
#: filenames *within* ``bindings.dir``, never resolved as a path itself.
#: ``id_map.overrides`` holds the path of an ID map file (D-138).
#:
#: ``extensions.roots`` is list-valued; every element is path-typed in both the
#: bare-string and the ``{root, namespace}`` form, which is why it is reported
#: under the element key ``extensions.roots[]``.
#:
#: Config.resolve_path applies the project root (§9.2.2) to those values.
_PATH_TYPED_CONFIG_KEYS: tuple[str, ...] = (
    "acl.root",
    "bindings.dir",
    "extensions.root",
    "extensions.roots[]",
    "id_map.overrides",
    "schema.root",
    "sys_modules.control.overrides_path",
)

#: The path-typed keys that resolve through §9.2's *scalar* precedence chain
#: (environment > file > default). ``extensions.roots[]`` is excluded because it
#: is list-valued and §9.2.1 requirement 3 gives it no environment tier at all;
#: its elements are guarded separately by :func:`_discard_empty_path_values`.
_SCALAR_PATH_TYPED_CONFIG_KEYS: frozenset[str] = frozenset(
    key for key in _PATH_TYPED_CONFIG_KEYS if not key.endswith("[]")
)

_FRAMEWORK_CONFIG_KEYS: frozenset[str] = frozenset(
    {
        "$schema",
        "_config.allow_unknown",
        "_config.strict",
        "acl.audit.enabled",
        "acl.audit.include_denied",
        "acl.audit.log_level",
        "acl.default_effect",
        "acl.root",
        "bindings.dir",
        "bindings.pattern",
        "executor.default_timeout",
        "executor.global_timeout",
        "executor.max_call_depth",
        "executor.max_module_repeat",
        "extensions.auto_discover",
        "extensions.follow_symlinks",
        "extensions.ignore_patterns",
        "extensions.lazy_load",
        "extensions.max_depth",
        "extensions.namespace",
        "extensions.root",
        "extensions.roots",
        "id_map.auto_detect",
        "id_map.overrides",
        "logging.format",
        "logging.level",
        "middleware.disabled",
        "obs.redaction.regex_patterns",
        "obs.redaction.replacement",
        "obs.redaction.sensitive_keys",
        "observability.metrics.enabled",
        "observability.metrics.exporter",
        "observability.tracing.enabled",
        "observability.tracing.exporter",
        "observability.tracing.otlp_endpoint",
        "observability.tracing.sampling_rate",
        "observability.tracing.strategy",
        "pipeline.configure",
        "pipeline.remove",
        "pipeline.steps",
        "project.name",
        "project.version",
        "schema.max_ref_depth",
        "schema.root",
        "schema.strategy",
        "stream.max_merge_depth",
        "sys_modules.control.enabled",
        "sys_modules.control.overrides_path",
        "sys_modules.enabled",
        "sys_modules.error_history.max_entries_per_module",
        "sys_modules.error_history.max_total_entries",
        "sys_modules.events.enabled",
        "sys_modules.events.subscribers",
        "sys_modules.events.thresholds.error_rate",
        "sys_modules.events.thresholds.latency_p99_ms",
        "sys_modules.health.enabled",
        "sys_modules.manifest.enabled",
        "sys_modules.usage.bucketing_strategy",
        "sys_modules.usage.enabled",
        "sys_modules.usage.retention_hours",
        "validation.binding.description_max_length",
        "validation.binding.documentation_max_length",
        "validation.binding.tags_pattern",
        "validation.binding.version_require_semver",
        "validation.pipeline.step_name_max_length",
        "validation.pipeline.timeout_ms_max",
        "version",
    }
)
"""Every key the canonical schemas declare, as full dot-paths.

Generated from ``schemas/*.schema.json`` and pinned by
``conformance/fixtures/config_key_governance.json``; the
``generate_config_key_governance.py`` CI step fails when the two drift.

This replaced a ``section -> direct child names`` mapping. Those schemas are
``additionalProperties: false`` at EVERY level, not only at the section root —
``observability.tracing``, ``acl.audit``, ``validation.binding`` and
``obs.redaction`` are each closed in their own right — so a one-level check left
strict mode blind exactly where a typo is hardest to spot:
``observability.tracing.sampling_rat`` passed it (its parent ``tracing`` IS
declared) while the canonical schema rejects it, and the misspelled sampling
rate silently fell back to its default (sync finding A-D-020).
"""


def _build_surface_trie(dot_paths: frozenset[str]) -> dict[str, object]:
    """Compile dot-paths into a nested dict; leaves are ``None``."""
    trie: dict[str, object] = {}
    for path in dot_paths:
        node = trie
        parts = path.split(".")
        for part in parts[:-1]:
            child = node.get(part)
            if not isinstance(child, dict):
                child = {}
                node[part] = child
            node = child
        node.setdefault(parts[-1], None)
    return trie


_SURFACE_TRIE: dict[str, object] = _build_surface_trie(_FRAMEWORK_CONFIG_KEYS)

#: Framework section names — the trie's container entries. Derived rather than
#: listed so a section added to the schemas appears here automatically.
_FRAMEWORK_SECTION_KEYS: frozenset[str] = frozenset(
    name for name, node in _SURFACE_TRIE.items() if isinstance(node, dict)
)

# =============================================================================
# Namespace registry (global, class-level, thread-safe)
# =============================================================================

_RESERVED_NAMESPACES: frozenset[str] = frozenset({"apcore", "_config"})

# Public alias of the reserved-namespace set (PROTOCOL_SPEC §9.9.5).
# Re-exported from ``apcore`` for ergonomic top-level access:
# ``from apcore import RESERVED_NAMESPACES``.
RESERVED_NAMESPACES: frozenset[str] = _RESERVED_NAMESPACES


_DEFAULT_MAX_DEPTH: int = 5
_VALID_ENV_STYLES: frozenset[str] = frozenset({"nested", "flat", "auto"})


@dataclasses.dataclass
class _NamespaceRegistration:
    name: str
    schema: dict[str, Any] | str | None
    env_prefix: str  # auto-derived or explicit (never None after registration)
    defaults: dict[str, Any] | None
    env_style: str  # "auto" (default), "nested", or "flat"
    max_depth: int  # max nesting depth for env conversion (default 5)
    env_map: dict[str, str] | None  # bare env var → config key mapping


_GLOBAL_NS_REGISTRY: dict[str, _NamespaceRegistration] = {}
_GLOBAL_NS_REGISTRY_LOCK: threading.Lock = threading.Lock()
_GLOBAL_ENV_MAP: dict[str, str] = {}  # bare env var → top-level config key
_GLOBAL_ENV_MAP_CLAIMED: dict[str, str] = {}  # env var → owner (for conflict detection)

T = TypeVar("T")


# =============================================================================
# Private helpers
# =============================================================================


def _deep_merge_dicts(base: dict[str, Any], override: dict[str, Any]) -> dict[str, Any]:
    """Recursively merge *override* into a copy of *base*."""
    result = dict(base)
    for key, value in override.items():
        if key in result and isinstance(result[key], dict) and isinstance(value, dict):
            result[key] = _deep_merge_dicts(result[key], value)
        else:
            result[key] = value
    return result


def _get_nested(data: dict[str, Any], dot_path: str, default: Any = None) -> Any:
    """Retrieve a value from a nested dict using a dot-separated path."""
    parts = dot_path.split(".")
    current: Any = data
    for part in parts:
        if isinstance(current, dict) and part in current:
            current = current[part]
        else:
            return default
    return current


def _set_nested(data: dict[str, Any], dot_path: str, value: Any) -> None:
    """Set a value in a nested dict, creating intermediate dicts as needed."""
    parts = dot_path.split(".")
    current = data
    for part in parts[:-1]:
        if part not in current or not isinstance(current[part], dict):
            current[part] = {}
        current = current[part]
    current[parts[-1]] = value


def _is_empty_path_value(value: Any) -> bool:
    """PROTOCOL_SPEC §9.2.1 requirement 5 — "an empty string is not a path".

    Exactly the empty string, not whitespace and not a falsy value of some other
    type. Requirement 5 names ``""`` and nothing else, and a guard that widened
    the test would start discarding values §9.2.1 says nothing about.
    """
    return isinstance(value, str) and value == ""


def _discard_empty_path_values(data: dict[str, Any], *, tier: str) -> dict[str, Any]:
    """Drop path-typed keys (§9.2.1) whose value in *data* is the empty string.

    PROTOCOL_SPEC §9.2.1 requirement 5. §9.2 treats a *set but empty*
    ``APCORE_*`` variable as an override like any other, so ``export
    APCORE_ACL_ROOT=`` silently blanks a directory the configuration file
    correctly declared — and ``""`` is a legal relative path to the filesystem
    API, so it then resolves to the working directory rather than failing. The
    empty value is discarded here and resolution falls through to the next tier
    exactly as if nothing had been set.

    *data* is the document for ONE tier, so removing the key is what "fall
    through" means: the tier below it — the configuration file, then the
    ``_DEFAULTS`` table — supplies the value instead. Applying this to an
    already-merged document would leave the key absent altogether, which is a
    different outcome and not the one requirement 5 asks for.

    ``extensions.roots`` is list-valued, so an element cannot fall through to
    anything; an empty element is dropped from the list, which is the same
    "never use it as a directory" outcome available for a list.
    """
    for key in _SCALAR_PATH_TYPED_CONFIG_KEYS:
        sentinel = object()
        if not _is_empty_path_value(_get_nested(data, key, sentinel)):
            continue
        parts = key.split(".")
        node: Any = data
        for part in parts[:-1]:
            node = node[part]
        del node[parts[-1]]
        _logger.warning(
            "%s set path-typed config key %r to the empty string, which is not a path "
            "(PROTOCOL_SPEC §9.2.1 requirement 5). Discarded; resolution falls through to "
            "the next tier as if it had not been set.",
            tier,
            key,
        )

    roots = _get_nested(data, "extensions.roots", None)
    if isinstance(roots, list):
        kept = [
            element
            for element in roots
            if not _is_empty_path_value(element.get("root") if isinstance(element, dict) else element)
        ]
        if len(kept) != len(roots):
            _set_nested(data, "extensions.roots", kept)
            _logger.warning(
                "%s carried %d empty element(s) in path-typed config key 'extensions.roots[]', "
                "which are not paths (PROTOCOL_SPEC §9.2.1 requirement 5). Discarded.",
                tier,
                len(roots) - len(kept),
            )
    return data


def _apply_env_overrides(
    data: dict[str, Any],
    *,
    namespace_prefixes: Sequence[str] | None = None,
) -> dict[str, Any]:
    """Apply APCORE_* environment variable overrides and global mappings.

    A path-typed key (§9.2.1) whose variable is *set but empty* is skipped
    rather than written: requirement 5 makes such a value no override at all, so
    whatever *data* already carries — the configuration file's value, or the
    default — stands. See :func:`_discard_empty_path_values` for why the guard
    belongs at the tier rather than on the merged document.

    ``namespace_prefixes`` is given in namespace mode, where *data* is the
    ``apcore`` namespace alone: a variable matching one of those registered
    prefixes (each ending in ``_``) belongs to that namespace, and a global
    ``env_map`` variable to its top-level key, so neither is written here too
    (§9.8.2 rule 4, D-146).
    """
    result = copy.deepcopy(data)  # deep copy to protect shared defaults
    for env_key, env_value in os.environ.items():
        # 1. Global env_map (bare env var → top-level key).
        if env_key in _GLOBAL_ENV_MAP:
            if namespace_prefixes is None:
                _set_env_override(result, _GLOBAL_ENV_MAP[env_key], env_key, env_value)
            continue
        if namespace_prefixes is not None and any(env_key.startswith(p) for p in namespace_prefixes):
            continue

        # 2. Standard APCORE_ prefix.
        if not env_key.startswith(_ENV_PREFIX):
            continue
        # apcore#88: the file selector is consumed by discover_config_file();
        # it is an argument to load(), not a value the document declares.
        # Kept here it would inject the phantom key ``config.file``.
        if env_key == _ENV_CONFIG_FILE:
            continue
        suffix = env_key[len(_ENV_PREFIX) :]
        if not suffix:
            continue
        # Convert: single _ → . (separator), double __ → literal _
        dot_path = suffix.lower().replace("__", "\x00").replace("_", ".").replace("\x00", "_")
        _set_env_override(result, dot_path, env_key, env_value)
    return result


def _set_env_override(
    data: dict[str, Any],
    key: str,
    env_key: str,
    raw_value: str,
    *,
    namespace: str | None = None,
    nested: bool = True,
) -> None:
    path_key = f"{namespace}.{key}" if namespace and namespace != "apcore" else key.removeprefix("apcore.")
    if path_key in _SCALAR_PATH_TYPED_CONFIG_KEYS:
        if _is_empty_path_value(raw_value):
            _logger.warning(
                "%s is set but empty; %r is path-typed and an empty string is not a path "
                "(PROTOCOL_SPEC §9.2.1 requirement 5). Ignoring the override.",
                env_key,
                path_key,
            )
            return
        value: Any = raw_value
    else:
        value = _coerce_env_value(raw_value)
    if nested:
        _set_nested(data, key, value)
    else:
        data[key] = value


def _coerce_env_value(value: str) -> Any:
    """Coerce string env value to appropriate Python type."""
    if value.lower() == "true":
        return True
    if value.lower() == "false":
        return False
    try:
        return int(value)
    except ValueError:
        pass
    try:
        return float(value)
    except ValueError:
        pass
    return value


def _env_suffix_to_dot_path_with_depth(suffix: str, max_depth: int) -> str:
    """Convert env var suffix to dot-path, stopping at *max_depth* segments.

    After producing ``max_depth - 1`` dots (i.e. *max_depth* segments),
    remaining ``_`` characters are preserved as literal underscores.
    Double ``__`` always means literal ``_`` regardless of depth.
    """
    lower = suffix.lower()
    result: list[str] = []
    dot_count = 0
    i = 0
    while i < len(lower):
        ch = lower[i]
        if ch == "_":
            if i + 1 < len(lower) and lower[i + 1] == "_":
                result.append("_")  # double __ → literal _
                i += 2
            elif dot_count < max_depth - 1:
                result.append(".")
                dot_count += 1
                i += 1
            else:
                result.append("_")  # depth limit reached
                i += 1
        else:
            result.append(ch)
            i += 1
    return "".join(result).strip(".")


def _auto_resolve_suffix(
    suffix: str,
    defaults: dict[str, Any] | None,
    max_depth: int,
) -> str:
    """Resolve env var suffix using the *defaults* tree structure.

    Tries the full suffix as a flat key first, then recursively splits at
    underscore positions to match nested dict keys.  Falls back to
    ``_env_suffix_to_dot_path_with_depth`` when no match is found.
    """
    lower = suffix.lower()
    if defaults is None:
        return _env_suffix_to_dot_path_with_depth(lower, max_depth)

    result = _match_suffix_to_tree(lower, defaults, 0, max_depth)
    if result is not None:
        return result
    # Fallback: nested conversion with depth limit.
    return _env_suffix_to_dot_path_with_depth(lower, max_depth)


def _match_suffix_to_tree(
    suffix: str,
    tree: dict[str, Any],
    depth: int,
    max_depth: int,
) -> str | None:
    """Try to match *suffix* against keys in *tree* (recursive)."""
    # 1. Try full suffix as a flat key.
    if suffix in tree:
        return suffix

    # 2. Depth limit reached — cannot split further.
    if depth >= max_depth - 1:
        return None

    # 3. Try splitting at each underscore position (left to right).
    for i, ch in enumerate(suffix):
        if ch != "_" or i == 0 or i == len(suffix) - 1:
            continue
        prefix_part = suffix[:i]
        remainder = suffix[i + 1 :]
        subtree = tree.get(prefix_part)
        if isinstance(subtree, dict):
            sub = _match_suffix_to_tree(remainder, subtree, depth + 1, max_depth)
            if sub is not None:
                return prefix_part + "." + sub

    return None


def _resolve_env_suffix(
    suffix: str,
    registration: _NamespaceRegistration,
) -> tuple[str, bool]:
    """Resolve an env var suffix to a config key path.

    Returns ``(key, is_nested)`` where *is_nested* indicates whether the key
    contains dots (i.e. should be stored via ``_set_nested``).
    """
    if registration.env_style == "flat":
        key = suffix.lower()
        return key, False
    if registration.env_style == "auto":
        key = _auto_resolve_suffix(suffix, registration.defaults, registration.max_depth)
        return key, "." in key
    # "nested" (default)
    key = _env_suffix_to_dot_path_with_depth(suffix, registration.max_depth)
    return key, "." in key


def _apply_allow_unknown(
    merged: dict[str, Any],
    registrations: list[_NamespaceRegistration],
) -> dict[str, Any]:
    """PROTOCOL_SPEC §9.6.3's `allow_unknown` row (apcore#118, decision D-69).

    Both halves of that row were inert. ``allow_unknown: false`` is documented
    as "silently ignored (not stored)" and the namespace was stored anyway, so
    `get()` answered for it; ``allow_unknown: true`` is documented as "stored,
    accessible, **WARN logged**" and no implementation logged anything. Fixing
    one without the other would leave the row half true, and both live here.

    **Namespace mode only, by construction.** §9.6.3 is about *namespaces*, and
    a legacy document has none — its root IS the `apcore` namespace, so an
    unrecognised top-level key there is a framework key governed by §9.14's
    walk under `strict`, not by this field. `strict`'s own clause (b) says it
    "applies in legacy mode too", which is the specification saying clause (a)
    does not.

    Only a deployment that explicitly writes ``allow_unknown: false`` changes
    behaviour, and what changes is that it finally gets the published contract
    instead of a no-op — but the change is real: a `get()` that returned a value
    now returns ``None``.
    """
    # An ABSENT `_config` is the default pair `strict: false, allow_unknown:
    # true`, not an exemption: §9.6.3's matrix describes the defaults, so a
    # document that declares an unregistered namespace and no `_config` at all
    # is the row that warns. This is not the blanket warning §9.2.2 rejects —
    # it fires on a condition specific to the document (there IS an
    # unregistered namespace), never on every configuration ever loaded.
    meta = merged.get("_config")
    if not isinstance(meta, dict):
        meta = {}
    if meta.get("strict", False):
        # `strict` already rejects an unknown namespace outright (clause a), so
        # this field is "only relevant when strict: false" per §9.6.3's own
        # comment. Nothing to decide.
        return merged

    known = {r.name for r in registrations} | {"apcore", "_config"}
    unknown = sorted(k for k in merged if k not in known)
    if not unknown:
        return merged

    if meta.get("allow_unknown", True):
        _logger.warning(
            "Configuration declares %d namespace(s) that no package has registered: %s. "
            "They are stored and readable through get(), and NOT validated against any "
            "schema (PROTOCOL_SPEC §9.6.3). Set _config.allow_unknown: false to have them "
            "dropped instead, or _config.strict: true to reject them.",
            len(unknown),
            ", ".join(unknown),
        )
        return merged

    _logger.info(
        "Dropping %d unregistered namespace(s) per _config.allow_unknown: false: %s "
        "(PROTOCOL_SPEC §9.6.3). Their keys are not stored and get() will not answer for "
        "them.",
        len(unknown),
        ", ".join(unknown),
    )
    return {k: v for k, v in merged.items() if k not in set(unknown)}


def _apply_namespace_env_overrides(
    data: dict[str, Any],
    registrations: list[_NamespaceRegistration],
) -> dict[str, Any]:
    """Apply per-namespace env overrides using longest-prefix-match dispatch (§9.8.3).

    Handles three sources in order:
      1. Global env_map (bare env var → top-level key)
      2. Namespace env_map (bare env var → namespace key)
      3. Prefix-based dispatch (MYAPP_FOO → myapp.foo)
    """
    result = copy.deepcopy(data)

    # Build namespace env_map lookup.
    ns_env_maps: dict[str, tuple[str, str]] = {}  # env_var → (ns_name, config_key)
    for reg in registrations:
        if reg.env_map:
            for env_var, config_key in reg.env_map.items():
                ns_env_maps[env_var] = (reg.name, config_key)

    # Prefix table: sorted by length descending for longest-prefix-match.
    prefixed = sorted(
        [r for r in registrations if r.env_prefix],
        key=lambda r: len(r.env_prefix),
        reverse=True,
    )

    for env_key, env_value in os.environ.items():
        # 1. Global env_map (bare env var → top-level dot-path).
        # The mapped target may be a dot-path (e.g. "server.port"); set it
        # nested at the top level so `get()` can traverse it.
        if env_key in _GLOBAL_ENV_MAP:
            _set_env_override(result, _GLOBAL_ENV_MAP[env_key], env_key, env_value)
            continue

        # 2. Namespace env_map (bare env var → namespace key).
        # The mapped target may also be a dot-path within the namespace.
        if env_key in ns_env_maps:
            ns_name, config_key = ns_env_maps[env_key]
            ns_data = result.setdefault(ns_name, {})
            if not isinstance(ns_data, dict):
                ns_data = {}
                result[ns_name] = ns_data
            _set_env_override(ns_data, config_key, env_key, env_value, namespace=ns_name)
            continue

        # 3. Prefix-based dispatch.
        if not prefixed:
            continue
        matched = _find_matching_ns_registration(env_key, prefixed)
        if matched is None:
            continue
        prefix = matched.env_prefix
        suffix = env_key[len(prefix) :]
        if not suffix:
            continue
        if suffix.startswith("_"):
            suffix = suffix[1:]
        if not suffix:
            continue
        key, is_nested = _resolve_env_suffix(suffix, matched)
        ns_data = result.setdefault(matched.name, {})
        _set_env_override(ns_data, key, env_key, env_value, namespace=matched.name, nested=is_nested)

    return result


def _find_matching_ns_registration(
    env_key: str,
    sorted_registrations: list[_NamespaceRegistration],
) -> _NamespaceRegistration | None:
    """Return the first (longest) registration whose ``env_prefix + "_"`` starts env_key.

    The separator is part of the match (§9.8.2 dispatch algorithm): ``APCORE_OBS``
    does not claim ``APCORE_OBSERVABILITY_*`` by sharing its leading letters.
    """
    for reg in sorted_registrations:
        prefix = reg.env_prefix or ""
        if prefix and env_key.startswith(prefix + "_"):
            return reg
    return None


def _registered_env_prefixes(registrations: list[_NamespaceRegistration]) -> list[str]:
    """Each registered namespace's env prefix with its ``_`` separator."""
    return [reg.env_prefix + "_" for reg in registrations if reg.env_prefix]


def _apply_namespace_defaults(
    data: dict[str, Any],
    registrations: list[_NamespaceRegistration],
) -> dict[str, Any]:
    """Merge namespace defaults under their namespace keys (defaults lose to file data)."""
    result = copy.deepcopy(data)
    for reg in registrations:
        if reg.defaults:
            existing = result.get(reg.name, {})
            if not isinstance(existing, dict):
                existing = {}
            result[reg.name] = _deep_merge_dicts(reg.defaults, existing)
    return result


def _validate_namespace_schema(
    namespace: str,
    ns_data: Any,
    schema: dict[str, Any] | str,
) -> None:
    """Validate ns_data against schema."""
    import jsonschema  # type: ignore[import-untyped]

    if isinstance(schema, str):
        schema_path = Path(schema)
        if not schema_path.is_file():
            _logger.warning("Schema file for namespace %r not found: %s", namespace, schema)
            return
        with open(schema_path, encoding="utf-8") as f:
            import json

            loaded_schema = json.load(f)
    else:
        loaded_schema = schema

    try:
        jsonschema.validate(ns_data, loaded_schema)
    except jsonschema.ValidationError as exc:
        raise ConfigError(
            message=f"Namespace {namespace!r} failed schema validation: {exc.message}",
            details={"namespace": namespace, "path": list(exc.absolute_path)},
        ) from exc


def _meta_strict(config_data: dict[str, Any]) -> bool:
    """Read ``_config.strict`` (PROTOCOL_SPEC §9.6.3), defaulting to False.

    ``_config`` sits at the top level of the document in both modes: in legacy
    mode the document *is* the ``apcore`` namespace, so the meta section is its
    sibling either way.
    """
    meta = config_data.get("_config")
    if not isinstance(meta, dict):
        return False
    return bool(meta.get("strict", False))


def _collect_unknown_framework_keys(apcore_data: dict[str, Any], prefix: str = "") -> list[str]:
    """PROTOCOL_SPEC §9.14 ``reject_unknown_framework_keys`` steps 2–3.

    Returns one message per key that a framework section carries and
    ``apcore-config.schema.json`` does not declare — **every** offending key,
    never just the first. An operator who opted into strict deserves to see the
    whole problem in one restart rather than one typo per restart.

    The walk is RECURSIVE: the canonical schemas are ``additionalProperties:
    false`` at every level, so a nested typo such as
    ``observability.tracing.sampling_rat`` is rejected too. It used to check one
    level only, which let exactly that case through (sync finding A-D-020).

    Callers run this only when ``_config.strict`` is true. Without it the keys
    are retained and readable through ``get()``: this SDK stores the parsed
    document as an untyped dict, so nothing is dropped at parse time, and
    §9.14 requires exactly that of every implementation.

    ``allow_unknown`` deliberately plays no part here — §9.6.3 defines it for
    unknown top-level *namespaces*, and stretching one field across two
    granularities would make its meaning depend on where it is read.
    """
    errors: list[str] = []

    def walk(data: dict[str, Any], node: dict[str, Any], path_prefix: str) -> None:
        for key in sorted(data):
            path = f"{path_prefix}.{key}"
            if key not in node:
                errors.append(f"Unknown key '{prefix}{path}' (strict mode enabled)")
                continue  # do not descend into an undeclared subtree
            child = node[key]
            value = data[key]
            if isinstance(child, dict) and isinstance(value, dict):
                walk(value, child, path)
            # A declared leaf ends the walk whatever its value is; a declared
            # container holding a non-object is a type error the A12 constraint
            # table owns, not an undeclared key.

    for section in sorted(_FRAMEWORK_SECTION_KEYS):
        section_data = apcore_data.get(section)
        if not isinstance(section_data, dict):
            continue
        node = _SURFACE_TRIE[section]
        if isinstance(node, dict):
            walk(section_data, node, section)
    return errors


#: §9.14 discovery tiers, numbered exactly as the spec numbers them.
#:
#: The tier is not bookkeeping: PROTOCOL_SPEC §9.2.2 makes it the thing that
#: *selects* the project root, so :func:`_discover_config_file_with_tier` has to
#: report which candidate won and not merely which path.
_TIER_ENV_CONFIG_FILE = 1
_TIER_PROJECT_YAML = 2
_TIER_PROJECT_YML = 3
_TIER_APCORE_YAML = 4
_TIER_APCORE_YML = 5
_TIER_USER_XDG = 6
_TIER_USER_LEGACY = 7

#: Tiers whose config file is *project-scoped* — pointed at explicitly, or found
#: next to the working directory. For these the file's own directory is the
#: project root (§9.2.2). Tiers 6-7 are deliberately absent: a user-level config
#: is shared by every project its owner runs, so ``./extensions`` written there
#: cannot mean ``~/.config/apcore/extensions``.
_PROJECT_SCOPED_TIERS: frozenset[int] = frozenset(
    {
        _TIER_ENV_CONFIG_FILE,
        _TIER_PROJECT_YAML,
        _TIER_PROJECT_YML,
        _TIER_APCORE_YAML,
        _TIER_APCORE_YML,
    }
)

#: CWD-relative candidates, in §9.14 order, paired with their tier number.
_CWD_CANDIDATE_TIERS: tuple[tuple[str, int], ...] = (
    ("project.yaml", _TIER_PROJECT_YAML),
    ("project.yml", _TIER_PROJECT_YML),
    ("apcore.yaml", _TIER_APCORE_YAML),
    ("apcore.yml", _TIER_APCORE_YML),
)


def _discover_config_file_with_tier() -> tuple[str | None, int | None]:
    """:func:`discover_config_file`, plus the §9.14 tier that produced the hit.

    Returns ``(None, None)`` when no candidate exists.
    """
    env_path = os.environ.get(_ENV_CONFIG_FILE)
    if env_path:
        return env_path, _TIER_ENV_CONFIG_FILE

    for candidate_name, tier in _CWD_CANDIDATE_TIERS:
        candidate = Path(candidate_name)
        if candidate.is_file():
            return str(candidate), tier

    if sys.platform == "darwin":
        xdg_config = Path.home() / "Library" / "Application Support" / "apcore" / "config.yaml"
    else:
        xdg_config = Path.home() / ".config" / "apcore" / "config.yaml"

    if xdg_config.is_file():
        return str(xdg_config), _TIER_USER_XDG

    legacy = Path.home() / ".apcore" / "config.yaml"
    if legacy.is_file():
        return str(legacy), _TIER_USER_LEGACY

    return None, None


def discover_config_file() -> str | None:
    """Search for a config file in the standard discovery order (§9.14).

    ``$APCORE_CONFIG_FILE`` is *consumed* here: ``_apply_env_overrides`` skips
    it so it never becomes the ``config.file`` override (apcore#88).
    """
    path, _tier = _discover_config_file_with_tier()
    return path


def _cwd() -> str:
    """The process working directory, resolved, as a string."""
    return str(Path.cwd().resolve())


def _project_root_for(config_file: str | None, tier: int | None) -> str:
    """PROTOCOL_SPEC §9.2.2 ``project_root(config)``.

    The config file's directory for §9.14 tiers 1-5, the process CWD for tiers
    6-7 and for a ``Config`` with no backing file.
    """
    if config_file is not None and tier in _PROJECT_SCOPED_TIERS:
        return str(Path(config_file).parent.resolve())
    return _cwd()


def _emit_per_load(message: str, category: type[Warning], *, stacklevel: int) -> None:
    """Emit a warning for each affected load using the host's warning filters.

    A fresh warning registry keeps one load from suppressing another.

    *stacklevel* is counted from this function's caller exactly as
    ``warnings.warn``'s is, so the reported filename and line are unchanged.
    ``module_globals`` is deliberately not forwarded: ``warn_explicit`` uses it
    only to pull a source line out of an exotic module loader, and a loader
    without ``get_source`` — ``__main__`` under ``python -`` is one — raises
    ``ImportError`` from inside the warning machinery when it is supplied.
    """
    try:
        frame = sys._getframe(stacklevel)
    except ValueError:  # pragma: no cover - shallower stack than expected
        frame = sys._getframe(0)
    warnings.warn_explicit(
        message,
        category,
        frame.f_code.co_filename,
        frame.f_lineno,
        module=frame.f_globals.get("__name__", "<string>"),
        registry=None,
    )


#: PROTOCOL_SPEC §9.2.4 — the declared configuration keys that reach no
#: consumer in any implementation (apcore#118). Order is the order they are
#: reported in, so two SDKs name them the same way.
#:
#: Ten when the window opened in spec v1.39.0; seven since v1.44.0, which gave
#: `observability.tracing.enabled` / `.sampling_rate` / `.exporter` consumers
#: (§10.1.1) and cancelled their withdrawal. A key that has left the table MUST
#: NOT warn — §9.2.4 requirement 1.
#:
#: `acl.default_effect` joined in v1.47.0 as §9.1.3's first application: it is
#: read from the ACL FILE, and the `apcore.yaml` twin reaches nothing. Ordered
#: first because §9.2.4's table lists it first.
#:
#: `middleware.disabled` (D-137) names built-in middleware that is not installed;
#: `extensions.auto_discover` (D-150) is read only by `validate()`'s semantic
#: warning — discovery runs when `discover()` is called.
_DEPRECATED_INERT_KEYS: tuple[str, ...] = (
    "acl.default_effect",
    "observability.metrics.enabled",
    "observability.metrics.exporter",
    "logging.level",
    "logging.format",
    "acl.audit.enabled",
    "acl.audit.include_denied",
    "acl.audit.log_level",
    "middleware.disabled",
    "extensions.auto_discover",
)


def _is_declared_either_mode(config: Config, key: str) -> bool:
    """Was *key* written by the operator, in legacy OR namespace mode?

    §9.6 puts the framework sections under an ``apcore:`` root in namespace
    mode, so ``config.declared`` is ``{"apcore": {...}}`` there and a flat
    lookup finds nothing. §9.2.4 requirement 1 says "a loaded configuration
    document", which does not exclude one layout — a namespace-mode document
    that declares `logging.level` has declared it, and the first implementation
    of this warning was silent for every such document.

    The fallback shape mirrors ``get_declared``'s own: try the flat spelling,
    then the ``apcore.``-prefixed one. Both are tried unconditionally rather
    than branching on the mode, because a key present under either spelling was
    written by somebody either way.
    """
    if _get_nested(config.declared, key) is not None:
        return True
    return _get_nested(config.declared, f"apcore.{key}") is not None


def _warn_deprecated_inert_keys(config: Config) -> None:
    """PROTOCOL_SPEC §9.2.4 — warn for declared keys that reach no consumer.

    Driven by the **declared** document, never the merged view (requirement 2).
    Every one of these keys has a default, so a merged-view check would fire for
    every configuration ever loaded — the blanket warning §9.2.2 rejects, which
    trains operators to ignore the one that matters.

    Behaviour is unchanged (requirement 3): the keys still parse, still validate,
    still answer ``get()``, and are still accepted under ``_config.strict``. This
    adds the one thing they have never had — a way for an operator to find out
    that setting them does nothing.
    """
    declared = [key for key in _DEPRECATED_INERT_KEYS if _is_declared_either_mode(config, key)]
    if not declared:
        return
    _emit_per_load(
        f"apcore#118 (PROTOCOL_SPEC §9.2.4): this configuration declares "
        f"{len(declared)} key(s) that reach no consumer in any apcore SDK and have no "
        f"effect: {', '.join(declared)}. They keep parsing and validating for the whole "
        "1.x line and are removed no earlier than v2.0 (§13.2 / §13.4). Nothing has "
        "changed in this release — the keys did nothing before this warning existed.",
        DeprecationWarning,
        stacklevel=3,
    )


def _collect_uncompilable_regex_patterns(data: dict[str, Any]) -> list[str]:
    """Report every ``obs.redaction.regex_patterns`` entry that does not compile.

    PROTOCOL_SPEC §9.2.3 requirement 6d. The three SDKs previously handled an
    unusable pattern the same way — skip it and carry on — so the divergence was
    never in the failure *policy* but in what counts as a failure: the Rust
    ``regex`` crate refuses lookaround and backreferences by design, JavaScript
    rejects an inline ``(?i)``. Reporting at validation time makes an
    engine-specific rejection visible at deploy rather than never.
    """
    import re as _re

    section = _get_nested(data, "obs.redaction.regex_patterns")
    if not isinstance(section, list):
        return []
    out: list[str] = []
    for index, pattern in enumerate(section):
        if not isinstance(pattern, str) or not pattern:
            continue
        try:
            _re.compile(pattern)
        except _re.error as exc:
            out.append(
                f"obs.redaction.regex_patterns[{index}] does not compile and would redact "
                f"nothing: {pattern!r} ({exc}). Patterns should stay inside the portable subset "
                "— no lookaround, no backreferences, no inline (?i) flags (PROTOCOL_SPEC §9.2.3 "
                "requirement 6)."
            )
    return out


class Config:
    """Configuration system with YAML loading, env overrides, and validation.

    Merge priority (highest wins): environment variables > config file > defaults.

    Backward compatible: ``Config(data={...})`` still works for in-memory
    configuration without file loading or validation.

    0.15.0: Namespace mode is detected when the top-level key ``"apcore"`` is
    present. In namespace mode, ``get()`` resolves the first path segment as a
    namespace name and looks up nested keys within it.
    """

    def __init__(
        self,
        data: dict[str, Any] | None = None,
        env_style: str = "auto",
    ) -> None:
        """Initialize configuration system.

        Args:
            data: Optional in-memory configuration data.
            env_style: Default env var conversion strategy ('auto', 'nested', 'flat').
        """
        self._data: dict[str, Any] = data or {}
        # The document as authored, before ``_DEFAULTS`` was merged in
        # (PROTOCOL_SPEC §9.3 step 1). For an in-memory ``Config(data=...)``
        # the caller-supplied dict *is* the declared document; the loaders
        # overwrite this with the parsed file data.
        self._declared: dict[str, Any] = copy.deepcopy(self._data)
        self._yaml_path: str | None = None
        # PROTOCOL_SPEC §9.2.2: one project root per Config, determined once.
        # A Config built from an in-memory mapping has no backing file, so the
        # root is CWD; ``load()`` replaces it before validating against the
        # file’s project root. Captured eagerly because a later chdir()
        # must not send consumers of one Config to different directories.
        self._reload_path: str | None = None
        self._project_root: str = _cwd()
        self._lock = threading.Lock()
        self._mode: str = "legacy"
        self._mounts: dict[str, dict[str, Any]] = {}
        self._env_style: str = env_style
        # Whether the originating ``load()`` validated; ``reload()`` carries the
        # same choice forward (PROTOCOL_SPEC §9.11 step 5). An in-memory Config
        # was never loaded, so reload() on it raises before this is consulted.
        self._validate_on_load: bool = True

    # ------------------------------------------------------------------
    # Default value resolution (single source of truth)
    # ------------------------------------------------------------------

    @classmethod
    def get_default(cls, key: str, fallback: Any = None) -> Any:
        """Resolve a default value from _DEFAULTS by dot-path.

        This is the single source of truth for all default values.
        Components MUST use this instead of hardcoding defaults.
        """
        parts = key.split(".")
        node: Any = _DEFAULTS
        for part in parts:
            if isinstance(node, dict) and part in node:
                node = node[part]
            else:
                return fallback
        return node

    @classmethod
    def path_typed_keys(cls) -> tuple[str, ...]:
        """Return the closed set of path-typed configuration keys (§9.2.1).

        A path-typed key is one whose value is a filesystem path. The set is
        declared canonically by ``"x-apcore-path": true`` in
        ``schemas/apcore-config.schema.json``; this returns that projection,
        sorted, as a tuple.

        ``extensions.roots`` is reported as ``extensions.roots[]`` because it is
        list-valued and every element carries a path.

        Relative values resolve against :attr:`project_root` (§9.2.2).
        """
        return _PATH_TYPED_CONFIG_KEYS

    # ------------------------------------------------------------------
    # Namespace registry (class-level)
    # ------------------------------------------------------------------

    @classmethod
    def register_namespace(
        cls,
        name: str,
        schema: dict[str, Any] | str | None = None,
        env_prefix: str | None = None,
        defaults: dict[str, Any] | None = None,
        env_style: str | None = None,
        max_depth: int | None = None,
        env_map: dict[str, str] | None = None,
    ) -> None:
        """Register a namespace globally.

        Args:
            name: Namespace name (must not be reserved or already registered).
            schema: Optional JSON Schema dict or path to a JSON Schema file.
            env_prefix: Env var prefix. When ``None``, auto-derived from
                ``name`` via ``name.upper().replace("-", "_")``.  When an
                explicit string, used as-is.
            defaults: Optional default values for this namespace.
            env_style: Env var key conversion strategy (default ``"auto"``).
            max_depth: Max nesting depth for env key conversion (default 5).
            env_map: Explicit mapping of bare env var names to config keys
                within this namespace (e.g. ``{"REDIS_URL": "cache_url"}``).

        Raises:
            ConfigNamespaceReservedError: If ``name`` is a reserved namespace.
            ConfigNamespaceDuplicateError: If ``name`` is already registered.
            ConfigEnvPrefixConflictError: If ``env_prefix`` conflicts.
            ConfigEnvMapConflictError: If an ``env_map`` key is already claimed.
        """
        resolved_style = env_style or "auto"
        if resolved_style not in _VALID_ENV_STYLES:
            msg = f"env_style must be one of {sorted(_VALID_ENV_STYLES)}, got {resolved_style!r}"
            raise ValueError(msg)
        resolved_depth = max_depth if max_depth is not None else _DEFAULT_MAX_DEPTH
        resolved_prefix = env_prefix if env_prefix is not None else name.upper().replace("-", "_")

        if name in _RESERVED_NAMESPACES:
            raise ConfigNamespaceReservedError(name=name)
        # The exact prefix `APCORE` is the `apcore` namespace's own; `APCORE_MCP`
        # and the like stay legal (§9.8.2 rules 2-3, D-146).
        if resolved_prefix == _APCORE_ENV_PREFIX:
            raise ConfigNamespaceReservedError(
                name=name,
                message=f"Env prefix {resolved_prefix!r} is reserved for the 'apcore' namespace",
            )

        with _GLOBAL_NS_REGISTRY_LOCK:
            if name in _GLOBAL_NS_REGISTRY:
                raise ConfigNamespaceDuplicateError(name=name)

            cls._validate_env_prefix(resolved_prefix)

            if env_map:
                cls._validate_env_map(env_map, owner=name)

            _GLOBAL_NS_REGISTRY[name] = _NamespaceRegistration(
                name=name,
                schema=schema,
                env_prefix=resolved_prefix,
                defaults=defaults,
                env_style=resolved_style,
                max_depth=resolved_depth,
                env_map=env_map,
            )

    @classmethod
    def env_map(cls, mapping: dict[str, str]) -> None:
        """Register global bare env var → top-level config key mappings.

        Args:
            mapping: Dict of env var names to config keys
                (e.g. ``{"PORT": "port", "DATABASE_URL": "db_url"}``).

        Raises:
            ConfigEnvMapConflictError: If an env var is already claimed.
        """
        with _GLOBAL_NS_REGISTRY_LOCK:
            for env_var in mapping:
                if env_var in _GLOBAL_ENV_MAP_CLAIMED:
                    owner = _GLOBAL_ENV_MAP_CLAIMED[env_var]
                    raise ConfigEnvMapConflictError(env_var=env_var, owner=owner)
            # All clean — register.
            for env_var, config_key in mapping.items():
                _GLOBAL_ENV_MAP[env_var] = config_key
                _GLOBAL_ENV_MAP_CLAIMED[env_var] = "__global__"

    @classmethod
    def _validate_env_prefix(cls, env_prefix: str) -> None:
        """Raise ConfigEnvPrefixConflictError if env_prefix is already in use."""
        for reg in _GLOBAL_NS_REGISTRY.values():
            if reg.env_prefix == env_prefix:
                raise ConfigEnvPrefixConflictError(env_prefix=env_prefix)

    @classmethod
    def _validate_env_map(cls, env_map: dict[str, str], owner: str) -> None:
        """Raise ConfigEnvMapConflictError if any env var is already claimed."""
        for env_var in env_map:
            if env_var in _GLOBAL_ENV_MAP_CLAIMED:
                existing_owner = _GLOBAL_ENV_MAP_CLAIMED[env_var]
                raise ConfigEnvMapConflictError(env_var=env_var, owner=existing_owner)
        # All clean — claim them.
        for env_var in env_map:
            _GLOBAL_ENV_MAP_CLAIMED[env_var] = owner

    @classmethod
    def registered_namespaces(cls) -> list[dict[str, Any]]:
        """Return a list of dicts describing all registered namespaces."""
        with _GLOBAL_NS_REGISTRY_LOCK:
            return [
                {
                    "name": reg.name,
                    "env_prefix": reg.env_prefix,
                    "has_schema": reg.schema is not None,
                }
                for reg in _GLOBAL_NS_REGISTRY.values()
            ]

    @classmethod
    def reserved_namespaces(cls) -> frozenset[str]:
        """Return the set of top-level namespace names reserved by apcore.

        Implements the public query API required by PROTOCOL_SPEC §9.9.5.

        The returned ``frozenset`` is the single source of truth referenced
        by :meth:`register_namespace` to enforce ``CONFIG_NAMESPACE_RESERVED``
        (§9.5.1 rules 3 and 4). It is callable without instantiating
        ``Config`` so third-party consumers (custom CLIs, framework
        integrations) can fail-fast on user-supplied namespace names before
        invoking :meth:`register_namespace`.

        Returns:
            ``frozenset[str]`` containing at minimum ``"apcore"`` and
            ``"_config"``. The returned object is immutable.
        """
        return _RESERVED_NAMESPACES

    # ------------------------------------------------------------------
    # Factory methods
    # ------------------------------------------------------------------

    @classmethod
    def load(cls, yaml_path: str | None = None, *, validate: bool = True) -> Config:
        """Load configuration from a YAML file with env overrides.

        Detects namespace mode when the top-level key ``"apcore"`` is present.

        Args:
            yaml_path: Path to the YAML configuration file.
            validate: If True, run full validation after loading.

        Returns:
            A new Config instance with merged data (defaults < file < env).

        Raises:
            ConfigNotFoundError: If the file does not exist.
            ConfigError: If the YAML is invalid or validation fails.
        """
        tier: int | None
        if yaml_path is None:
            yaml_path, tier = _discover_config_file_with_tier()
            if yaml_path is None:
                # No configuration file at all -> project root is CWD (§9.2.2).
                # ``from_defaults()`` already set that in ``__init__``.
                return cls.from_defaults()
        else:
            # A path handed in by the caller is tier-1 shaped: the config file
            # was pointed at explicitly, from an arbitrary location, exactly as
            # ``$APCORE_CONFIG_FILE`` does it. Its directory is the project root.
            tier = _TIER_ENV_CONFIG_FILE

        project_root = _project_root_for(yaml_path, tier)
        config = cls._load_document(yaml_path, project_root=project_root, validate=validate)
        _warn_deprecated_inert_keys(config)
        return config

    @classmethod
    def _load_document(cls, yaml_path: str, *, project_root: str, validate: bool) -> Config:
        """Parse a document and validate it using its established project root."""
        path = Path(yaml_path)
        absolute_path = path.absolute()
        if not absolute_path.is_file():
            raise ConfigNotFoundError(config_path=str(path))

        with open(absolute_path, encoding="utf-8") as f:
            try:
                file_data = yaml.safe_load(f)
            except yaml.YAMLError as e:
                raise ConfigError(message=f"Invalid YAML in {yaml_path}: {e}") from e

        if file_data is None:
            file_data = {}
        if not isinstance(file_data, dict):
            raise ConfigError(message=f"Config file must be a mapping, got {type(file_data).__name__}")

        # Namespace mode requires "apcore" key to be a dict, not null/scalar/list.
        apcore_val = file_data.get("apcore")
        if isinstance(apcore_val, dict):
            config = cls._load_namespace_mode(file_data)
        else:
            config = cls._load_legacy_mode(file_data)

        config._yaml_path = str(path)
        config._reload_path = str(absolute_path)
        config._project_root = project_root
        if validate:
            config.validate()
        # PROTOCOL_SPEC §9.11 step 5: reload() re-validates only if the
        # originating load() did. Silently re-imposing validation on a config
        # the caller deliberately loaded with validate=False is a behaviour
        # change disguised as a refresh.
        config._validate_on_load = validate
        return config

    @classmethod
    def _load_legacy_mode(cls, file_data: dict[str, Any]) -> Config:
        """Load in legacy (flat) mode: defaults < file < env."""
        # Snapshot the declared document — everything supplied by *someone*
        # (the file, then env overrides), excluding only the default table.
        # PROTOCOL_SPEC §9.1 "What 'declared' means": an env override counts as
        # declaration, so APCORE_PROJECT_NAME satisfies ``project.name`` for a
        # file that omits it — configuring entirely through the environment is a
        # first-class deployment shape. §9.3 step 1 evaluates requiredness
        # against this, not against ``merged``, where every key is present by
        # construction.
        # §9.2.1 requirement 5, applied to the FILE tier: a path-typed key
        # written as "" in the document is discarded so the default table below
        # it supplies the value, exactly as the env tier's guard in
        # ``_apply_env_overrides`` does for a set-but-empty variable.
        file_data = _discard_empty_path_values(copy.deepcopy(file_data), tier="the configuration file")
        declared = _apply_env_overrides(copy.deepcopy(file_data))
        merged = _deep_merge_dicts(_DEFAULTS, file_data)
        merged = _apply_env_overrides(merged)
        config = cls(data=merged)
        config._declared = declared
        config._mode = "legacy"
        return config

    @classmethod
    def _load_namespace_mode(cls, file_data: dict[str, Any]) -> Config:
        """Load in namespace mode: namespace defaults < file < env overrides."""
        with _GLOBAL_NS_REGISTRY_LOCK:
            registrations = list(_GLOBAL_NS_REGISTRY.values())

        # §9.2.1 requirement 5 at the file tier — see ``_load_legacy_mode``.
        # The framework's own path-typed keys live under the ``apcore``
        # namespace here, which is where ``_apply_env_overrides`` also reaches
        # them below.
        file_data = copy.deepcopy(file_data)
        if isinstance(file_data.get("apcore"), dict):
            file_data["apcore"] = _discard_empty_path_values(file_data["apcore"], tier="the configuration file")

        # Apply namespace defaults (lower priority than file data)
        merged = _apply_namespace_defaults(file_data, registrations)

        # An APCORE_ variable no registered prefix claims belongs to `apcore`;
        # one a registered prefix claims goes to that namespace only (§9.8.2 rule 4).
        claimed_prefixes = _registered_env_prefixes(registrations)
        apcore_ns = merged.get("apcore", {})
        apcore_ns = _apply_env_overrides(apcore_ns, namespace_prefixes=claimed_prefixes)
        merged["apcore"] = apcore_ns

        # Apply per-namespace env overrides for registered namespaces
        merged = _apply_namespace_env_overrides(merged, registrations)

        # §9.6.3's `allow_unknown` row, both halves of which were inert
        # (apcore#118, decision D-69). Applied AFTER the env overrides so a
        # namespace that exists only because of an `APCORE_*` variable is
        # treated the same as one written in the file.
        merged = _apply_allow_unknown(merged, registrations)

        config = cls(data=merged)
        # Same rule as legacy mode (§9.1): file + env overrides, never defaults.
        # The namespace-defaults merge is skipped for exactly that reason.
        declared_ns = copy.deepcopy(file_data)
        declared_apcore = _apply_env_overrides(declared_ns.get("apcore", {}), namespace_prefixes=claimed_prefixes)
        if declared_apcore:
            declared_ns["apcore"] = declared_apcore
        config._declared = _apply_namespace_env_overrides(declared_ns, registrations)
        config._mode = "namespace"
        return config

    @classmethod
    def from_defaults(cls) -> Config:
        """Create a Config from default values with env overrides applied."""
        data = _apply_env_overrides(dict(_DEFAULTS))
        config = cls(data=data)
        # Nothing was declared: every value here comes from the default table
        # or the environment.
        config._declared = {}
        return config

    # ------------------------------------------------------------------
    # Access
    # ------------------------------------------------------------------

    def get(self, key: str, default: Any = None) -> Any:
        """Get a configuration value by dot-path key.

        In namespace mode, the first path segment is the namespace name.
        Namespace names may contain hyphens; the longest matching registered
        namespace prefix is resolved first.
        """
        with self._lock:
            if self._mode == "namespace":
                return self._get_namespace_mode(key, default)
            return _get_nested(self._data, key, default)

    def _get_namespace_mode(self, key: str, default: Any) -> Any:
        """Resolve a dot-path key in namespace mode."""
        namespace, remainder = self._split_namespace_key(key)
        if namespace is None:
            # §9.9.1: Fallback to the implicit "apcore" namespace.
            return self._get_namespace_mode(f"apcore.{key}", default)

        ns_data = self._data.get(namespace)
        if ns_data is None:
            # §9.9.1: Fallback to implicit "apcore" namespace when the resolved
            # top-level segment does not exist in data. Avoid infinite recursion
            # once we are already probing under "apcore".
            if namespace != "apcore":
                return self._get_namespace_mode(f"apcore.{key}", default)
            return default
        if remainder is None:
            if isinstance(ns_data, dict):
                return copy.deepcopy(ns_data)
            # Bare top-level value (e.g. set via global env_map with a single-
            # segment target like ``port``). Return the value directly.
            return ns_data
        if not isinstance(ns_data, dict):
            # A-D-049: remainder requested but the resolved top-level value is a
            # non-dict scalar. Spec §9.9.1 does NOT define an implicit-apcore
            # fallback here (Rust is spec-correct), so return the default rather
            # than recursing into ``apcore.{key}``.
            return default
        return _get_nested(ns_data, remainder, default)

    def _split_namespace_key(self, key: str) -> tuple[str | None, str | None]:
        """Split key into (namespace, remainder) using longest-prefix matching.

        Resolution order (matches TypeScript ``resolveNamespacePath``):
          1. Longest registered namespace prefix.
          2. Built-in ``apcore`` namespace.
          3. Naive first-segment split — allows top-level keys injected via
             global ``env_map`` (e.g. ``server.port``) or unmatched ``APCORE_*``
             fallback paths (e.g. ``executor.timeout``) to resolve against the
             raw data root. Matches cross-language behavior per §9.8 / §9.9.1.

        Returns (namespace, None) if the key is exactly the namespace.
        Returns (namespace, remainder) otherwise.
        Returns (None, None) only for a completely empty key.
        """
        with _GLOBAL_NS_REGISTRY_LOCK:
            known_names = sorted(_GLOBAL_NS_REGISTRY.keys(), key=len, reverse=True)

        for name in known_names:
            if key == name:
                return name, None
            if key.startswith(name + "."):
                return name, key[len(name) + 1 :]

        # Also handle the built-in "apcore" namespace in namespace mode
        if key == "apcore":
            return "apcore", None
        if key.startswith("apcore."):
            return "apcore", key[len("apcore") + 1 :]

        # Naive first-segment fallback for unregistered top-level keys.
        if not key:
            return None, None
        dot_index = key.find(".")
        if dot_index == -1:
            return key, None
        return key[:dot_index], key[dot_index + 1 :]

    def set(self, key: str, value: Any) -> None:
        """Set a configuration value by dot-path key.

        A-D-050: in namespace mode, set() is symmetric with get() — it resolves
        the registered-namespace longest-prefix first, then writes the remainder
        within that namespace. This matters when a namespace name itself
        contains dots (e.g. ``my.app``): a naive dot-path write would nest under
        ``my`` -> ``app`` where get() reads the whole ``my.app`` key.
        """
        with self._lock:
            path_key = key.removeprefix("apcore.") if self._mode == "namespace" else key
            if path_key in _SCALAR_PATH_TYPED_CONFIG_KEYS and _is_empty_path_value(value):
                return
            if self._mode == "namespace":
                namespace, remainder = self._split_namespace_key(key)
                if namespace is not None:
                    if remainder is None:
                        self._data[namespace] = value
                        return
                    ns_data = self._data.get(namespace)
                    if not isinstance(ns_data, dict):
                        ns_data = {}
                        self._data[namespace] = ns_data
                    _set_nested(ns_data, remainder, value)
                    return
            _set_nested(self._data, key, value)

    @property
    def data(self) -> dict[str, Any]:
        """Return a deep copy of the raw config data."""
        with self._lock:
            return copy.deepcopy(self._data)

    @property
    def declared(self) -> dict[str, Any]:
        """Return a deep copy of the document as authored, before defaults merged.

        PROTOCOL_SPEC §9.3 step 1 evaluates required fields against the
        *declared* document: a key that carries a canonical default is never
        required, and checking any key after the default table has been merged
        is a no-op. :meth:`validate` reads this, not :attr:`data`.

        Empty for :meth:`from_defaults`; equal to the caller's dict for an
        in-memory ``Config(data=...)``; the parsed YAML for :meth:`load`.
        Environment overrides are *not* reflected here — they are applied on
        top of the merged document, not on the authored one.

        The apcore-rust SDK exposes the same view as ``Config::get_declared()``.
        """
        with self._lock:
            return copy.deepcopy(self._declared)

    @property
    def source_path(self) -> str | None:
        """Return the path passed to or discovered by :meth:`load`, if any."""
        with self._lock:
            return self._yaml_path

    @property
    def project_root(self) -> str:
        """Return the fixed absolute base for paths in this configuration (§9.2.2)."""
        return self._project_root

    def resolve_path(self, value: str | os.PathLike[str]) -> str:
        """Return an absolute path using the fixed project root (§9.2.2).

        Raises ``ValueError`` for an empty path. The caller supplies the effective
        value after applying configuration precedence (§9.2.1).
        """
        raw_path = os.fspath(value)
        if not raw_path:
            raise ValueError("An empty configuration value is not a path")
        if Path(raw_path).is_absolute():
            return raw_path
        # A terminal slash or dot requires the preceding component to be a
        # directory. pathlib strips these markers, potentially turning an
        # invalid filename into one that can be opened.
        separators = (os.sep, os.altsep) if os.altsep else (os.sep,)
        if raw_path.endswith((*separators, *(f"{sep}." for sep in separators))):
            return os.path.join(self._project_root, raw_path)
        return str(Path(self._project_root) / raw_path)

    # ------------------------------------------------------------------
    # Namespace-mode instance methods
    # ------------------------------------------------------------------

    def mount(
        self,
        namespace: str,
        *,
        from_file: str | None = None,
        from_dict: dict[str, Any] | None = None,
    ) -> None:
        """Attach external configuration to a namespace.

        Exactly one of ``from_file`` or ``from_dict`` must be provided.

        Args:
            namespace: The namespace to mount into (must not be ``"_config"``).
            from_file: Path to a YAML file to load.
            from_dict: A dict to merge into the namespace.

        Raises:
            ConfigMountError: If both or neither source is provided, if the
                namespace is ``"_config"``, or if ``from_file`` does not exist.
        """
        if namespace == "_config":
            raise ConfigMountError(message="Cannot mount into the reserved namespace '_config'")
        if from_file is None and from_dict is None:
            raise ConfigMountError(message="Exactly one of 'from_file' or 'from_dict' must be provided")
        if from_file is not None and from_dict is not None:
            raise ConfigMountError(message="Exactly one of 'from_file' or 'from_dict' must be provided")

        if from_file is not None:
            path = Path(from_file)
            if not path.is_file():
                raise ConfigMountError(message=f"Mount file not found: {from_file}")
            with open(path, encoding="utf-8") as f:
                try:
                    loaded = yaml.safe_load(f)
                except yaml.YAMLError as exc:
                    raise ConfigMountError(message=f"Invalid YAML in mount file {from_file}: {exc}") from exc
            if loaded is None:
                loaded = {}
            if not isinstance(loaded, dict):
                raise ConfigMountError(message=f"Mount file must be a mapping: {from_file}")
            mount_data: dict[str, Any] = loaded
        else:
            mount_data = dict(from_dict or {})

        with self._lock:
            existing = self._mounts.get(namespace, {})
            self._mounts[namespace] = _deep_merge_dicts(existing, mount_data)
            ns_existing = self._data.get(namespace, {})
            if not isinstance(ns_existing, dict):
                ns_existing = {}
            self._data[namespace] = _deep_merge_dicts(ns_existing, mount_data)

    def namespace(self, name: str) -> dict[str, Any]:
        """Return a deep copy of the namespace subtree dict.

        Args:
            name: Namespace name.

        Returns:
            Deep copy of the namespace data dict (empty dict if not present).
        """
        with self._lock:
            ns_data = self._data.get(name, {})
            if not isinstance(ns_data, dict):
                return {}
            return copy.deepcopy(ns_data)

    def get_typed(self, path: str, type_: Type[T]) -> T:
        """Get a configuration value coerced to ``type_``.

        Args:
            path: Dot-path to the value.
            type_: Target Python type to coerce to.

        Returns:
            The value coerced to ``type_``.

        Raises:
            ConfigBindError: If the value is missing or cannot be coerced.
        """
        value = self.get(path)
        if value is None:
            raise ConfigBindError(message=f"No value at path {path!r}")
        try:
            return type_(value)  # type: ignore[call-arg]
        except (TypeError, ValueError) as exc:
            raise ConfigBindError(message=f"Cannot coerce value at {path!r} to {type_.__name__}: {exc}") from exc

    def bind(self, namespace: str, model_class: type) -> Any:
        """Deserialize the namespace into a Pydantic model or dataclass.

        Args:
            namespace: The namespace whose data to bind.
            model_class: A Pydantic model class or Python dataclass.

        Returns:
            An instance of ``model_class`` populated with namespace data.

        Raises:
            ConfigBindError: If deserialization fails.
        """
        ns_data = self.namespace(namespace)
        return _instantiate_model(model_class, ns_data, namespace)

    # ------------------------------------------------------------------
    # Validation (Algorithm A12)
    # ------------------------------------------------------------------

    def validate(self) -> None:
        """Validate the configuration per Algorithm A12.

        Checks required fields, type constraints, and semantic rules.
        Collects all errors before raising.

        Raises:
            ConfigError: With all validation errors in the message
                (``code="CONFIG_INVALID"``).
        """
        namespace_mode = self._mode == "namespace"
        data = self._data.get("apcore", {}) if namespace_mode else self._data
        declared = self._declared.get("apcore", {}) if namespace_mode else self._declared
        if "version" in declared:
            _negotiate_document_version(str(declared["version"]))

        errors: list[str] = []
        if not namespace_mode:
            # Defaults do not satisfy a required declaration in legacy mode.
            for field in _REQUIRED_FIELDS:
                if _get_nested(declared, field) is None:
                    errors.append(f"Missing required field: '{field}'")

        strict = _meta_strict(self._data)
        prefix = "apcore." if namespace_mode else ""
        for field, (check_fn, err_msg) in _CONSTRAINTS.items():
            value = _get_nested(data, field)
            if value is not None and not check_fn(value):
                errors.append(f"Invalid value for '{prefix}{field}': {err_msg} (got {value!r})")

        # §10.1.1: endpoints require an exporter that consumes them.
        otlp_endpoint = _get_nested(data, "observability.tracing.otlp_endpoint")
        if otlp_endpoint is not None:
            exporter = _get_nested(data, "observability.tracing.exporter", "stdout")
            if exporter != "otlp":
                errors.append(
                    f"{prefix}observability.tracing.otlp_endpoint is set but "
                    f"{prefix}observability.tracing.exporter is {exporter!r}, which does not read it. "
                    f"Set exporter to 'otlp', or remove the endpoint."
                )

        self._validate_path_availability(data, errors)
        if strict:
            errors.extend(_collect_unknown_framework_keys(data, prefix=prefix))
        errors.extend(_collect_uncompilable_regex_patterns(data))
        if errors:
            raise ConfigError(
                message=f"Configuration validation failed ({len(errors)} error(s)):\n"
                + "\n".join(f"  - {e}" for e in errors),
                details={"errors": errors},
            )
        if namespace_mode:
            self._validate_registered_namespaces(strict)

    def _validate_path_availability(self, data: dict[str, Any], errors: list[str]) -> None:
        ext_root = _get_nested(data, "extensions.root")
        auto_discover = _get_nested(data, "extensions.auto_discover", False)
        if auto_discover and ext_root and not os.path.exists(self.resolve_path(str(ext_root))):
            _logger.warning(
                "extensions.auto_discover=true but extensions.root '%s' does not exist",
                ext_root,
            )

        schema_strategy = _get_nested(data, "schema.strategy")
        schema_root = _get_nested(data, "schema.root")
        if schema_strategy == "yaml_only" and schema_root and not os.path.isdir(self.resolve_path(str(schema_root))):
            errors.append(f"schema.strategy='yaml_only' but schema.root '{schema_root}' does not exist")

    def _validate_registered_namespaces(self, strict: bool) -> None:
        """Validate the configuration supplied by registered namespaces."""
        with _GLOBAL_NS_REGISTRY_LOCK:
            registry_snapshot = dict(_GLOBAL_NS_REGISTRY)

        for key, value in self._data.items():
            if key in ("apcore", "_config"):
                continue
            reg = registry_snapshot.get(key)
            if reg is None:
                if strict:
                    raise ConfigError(
                        message=f"Unknown namespace {key!r} in strict mode",
                        details={"namespace": key},
                    )
                continue
            if reg.schema is not None:
                _validate_namespace_schema(key, value, reg.schema)

    # ------------------------------------------------------------------
    # Hot-reload
    # ------------------------------------------------------------------

    def reload(self) -> None:
        """Re-read configuration from the original YAML file.

        Re-applies namespace defaults, env overrides, and stored mounts.

        Only works if the Config was created via ``Config.load()``.

        Raises:
            ConfigError: If no YAML path was stored or reload fails.
        """
        with self._lock:
            yaml_path = self._reload_path
            if yaml_path is None:
                raise ConfigError(message="Cannot reload: Config was not loaded from a YAML file")
            stored_mounts = copy.deepcopy(self._mounts)

        reloaded = Config._load_document(yaml_path, project_root=self._project_root, validate=self._validate_on_load)

        with self._lock:
            self._data = reloaded._data
            self._declared = reloaded._declared
            self._mode = reloaded._mode
            # Re-apply mounts
            for namespace, mount_data in stored_mounts.items():
                ns_existing = self._data.get(namespace, {})
                if not isinstance(ns_existing, dict):
                    ns_existing = {}
                self._data[namespace] = _deep_merge_dicts(ns_existing, mount_data)
            self._mounts = stored_mounts

    # ------------------------------------------------------------------
    # Dunder
    # ------------------------------------------------------------------

    def __repr__(self) -> str:
        if self._yaml_path:
            return f"Config(yaml_path={self._yaml_path!r})"
        return f"Config(data=<{len(self._data)} keys>)"


# =============================================================================
# Private: model instantiation helper
# =============================================================================


def _instantiate_model(model_class: type, data: dict[str, Any], namespace: str) -> Any:
    """Bind a config namespace dict to a Pydantic v2 model, dataclass, or plain class.

    Pydantic v2 models are instantiated via ``model_validate`` (which runs
    field validation); dataclasses and any other class accepting a keyword
    init are instantiated via ``model_class(**data)``. Pydantic v1's
    ``parse_obj`` form is no longer attempted because ``pyproject.toml``
    requires ``pydantic>=2.0``.
    """
    try:
        if hasattr(model_class, "model_validate"):
            return model_class.model_validate(data)
        return model_class(**data)
    except Exception as exc:
        raise ConfigBindError(
            message=f"Failed to bind namespace {namespace!r} to {model_class.__name__}: {exc}"
        ) from exc


# =============================================================================
# Bootstrap: register apcore built-in namespaces (§9.15)
# =============================================================================

# Only the two blocks $defs/ObservabilityConfig declares (D-144, §9.15.2). The
# live homes of error history, platform notification and redaction are
# `sys_modules.error_history.*`, `sys_modules.events.thresholds.*` and `obs.redaction.*`.
Config.register_namespace(
    "observability",
    env_prefix="APCORE_OBSERVABILITY",
    defaults={
        "tracing": {
            "enabled": False,
            "strategy": "full",
            "sampling_rate": 1.0,
            "exporter": "stdout",
            "otlp_endpoint": None,
        },
        "metrics": {"enabled": False, "exporter": "stdout"},
    },
)

#: Default ``obs.redaction.sensitive_keys`` list (Issue #43 §5).  Any field
#: whose name contains one of these substrings (case-insensitive) is
#: redacted at log emission and at module input/output capture.  The
#: ``_secret_*`` glob is preserved for backward compatibility with the
#: legacy prefix convention.
_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS: list[str] = [
    "_secret_*",
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "apiKey",
    "access_key",
    "private_key",
    "authorization",
    "auth",
    "credential",
    "cookie",
    "session",
    "bearer",
]


Config.register_namespace(
    "obs",
    env_prefix="APCORE_OBS",
    defaults={
        "redaction": {
            "regex_patterns": [],
            "sensitive_keys": list(_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS),
            "replacement": "***REDACTED***",
        },
    },
)

Config.register_namespace(
    "sys_modules",
    env_prefix="APCORE_SYS",
    defaults={
        # Activation is off by default: PROTOCOL_SPEC 6.6.3 states
        # `sys_modules.enabled = false (default)` -> 0 modules registered, and
        # schemas/sys-modules.schema.json declares `default: false`. Registering
        # `True` here made namespace-mode projects stand up the six read modules
        # without asking — an information-disclosure surface 6.6.3 calls out by
        # name. The per-module sub-flags stay True: they select WHICH modules
        # register once activation has happened.
        "enabled": False,
        "health": {"enabled": True},
        "manifest": {"enabled": True},
        "usage": {
            "enabled": True,
            "retention_hours": 168,
            "bucketing_strategy": "hourly",
        },
        "control": {"enabled": True},
        # `error_history` and `events.subscribers` are declared with defaults by
        # schemas/sys-modules.schema.json and were missing here, so this
        # namespace answered for eleven of its own schema's fourteen keys.
        # `control.overrides_path` is the one deliberate omission: its declared
        # default is null, which a namespace default cannot express distinctly
        # from absence (sync finding A-D-021).
        "error_history": {
            "max_entries_per_module": 50,
            "max_total_entries": 1000,
        },
        "events": {
            "enabled": False,
            "subscribers": [],
            "thresholds": {"error_rate": 0.1, "latency_p99_ms": 5000.0},
        },
    },
)

# Python's synchronous executor bridge is an SDK setting. Keep it in a
# Config Bus namespace so shared protocol configuration stays portable.
Config.register_namespace(
    "apcore-python",
    env_prefix="APCORE_PYTHON",
    schema={
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": False,
        "properties": {
            "executor": {
                "description": "Python executor settings.",
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "sync_bridge_timeout": {
                        "description": "Worker thread wait in milliseconds. Null derives the wait and zero disables the bound.",
                        "type": ["integer", "null"],
                        "minimum": 0,
                    },
                },
            },
        },
    },
)
