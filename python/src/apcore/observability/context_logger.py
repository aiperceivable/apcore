"""Structured logging: ContextLogger, RedactionConfig, and ObsLoggingMiddleware."""

from __future__ import annotations

import json
import re
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from collections.abc import Sequence
from typing import Any

from apcore.utils.pattern import match_glob
from apcore.utils.redaction import PROTECTED_LOG_FIELDS as _PROTECTED_LOG_FIELDS

# The §10.6.1 key rule is imported, never re-implemented: §10.6 requires it to
# hold identically on the executor capture-point surface and on this
# log-emission surface, so a second copy here is a second rule waiting to
# drift. It already happened to the sibling value matcher — see
# :func:`_value_matches_regex` below for the leak the two copies produced.
# The local name is kept as an alias because it is the documented spelling on
# this surface; the two separator-folding helpers it used to call are internal
# to the canonical matcher and are no longer duplicated here at all.
from apcore.utils.redaction import (
    REDACTED_VALUE,
    ValuePattern,
    _key_matches as _key_matches_sensitive,
    _value_matches,
    compile_value_regexes,
)

from apcore.context_keys import LOGGING_STARTS
from apcore.middleware.base import Middleware


_LEVELS = {
    "trace": 0,
    "debug": 10,
    "info": 20,
    "warn": 30,
    "error": 40,
    "fatal": 50,
}

# Maximum recursion depth for nested redaction.  Matches the spec's schema
# validation depth limit (32) so the redactor can never run away on
# adversarial / cyclic-looking input.  Deeper structures stop being
# inspected — secrets buried below depth 32 are out of scope.
_MAX_REDACTION_DEPTH = 32

# Correlation-ID fields that must NEVER be redacted, by any rule.  Removing or
# scrambling these would break log/trace correlation.  Mirrors the TS
# PROTECTED_LOG_FIELDS and Rust NEVER_REDACT_FIELDS sets.
#
# Re-exported from :mod:`apcore.utils.redaction`, which owns the canonical set:
# the exemption has to hold on BOTH mandated surfaces ("Redaction MUST apply
# both at log emission and at the executor's input/output capture point"), and
# two independent literals is how it came to hold on neither of the recursive
# ones.  Kept importable from this module — it is the documented public name.
PROTECTED_LOG_FIELDS = _PROTECTED_LOG_FIELDS


@dataclass
class RedactionConfig:
    """Runtime-configurable redaction rules for observability logging.

    Applied in addition to schema-level ``x-sensitive`` annotations.
    The union of all matching fields and values is redacted.

    Fields:
        field_patterns: Glob patterns matched against field names (e.g. ``"*password*"``).
        value_patterns: Regex patterns matched against string field values (e.g. ``r"^Bearer .*"``).
        sensitive_keys: Matched case-insensitively against field **names**
            (Issue #43 §5, PROTOCOL_SPEC §10.6.1), per entry: an entry
            containing ``*`` or ``?`` is a glob-dialect pattern (Algorithm
            A25, anchored to the whole name); an entry containing neither is
            a substring match against the normalized name, so a field is
            redacted whenever its name contains it, at any nesting depth.
            ``[``, ``]``, ``{``, ``}`` and ``\\`` are literals, never a
            character class.
        regex_patterns: Regex patterns matched against **string** field
            **values** (Issue #43 §5, PROTOCOL_SPEC §10.6.1).  Matched values
            are replaced with ``replacement``.  Compiled once here, with
            ``re.IGNORECASE``.  A non-string value is never tested and never
            stringified in order to test it (§10.6.1 requirement 2).
        replacement: Substitution string for redacted values; default ``"***REDACTED***"``.

    Two derived fields are computed at construction and are not constructor
    arguments:

        compiled_regex_patterns: ``regex_patterns`` compiled once, per
            §10.6.1 requirement 5 — "compilation SHOULD happen once, when the
            configuration is read, so the diagnostic fires at load rather than
            per log record".  Use this on any hot path.
        invalid_regex_patterns: ``(pattern, engine message)`` for every entry
            that did not compile, for ``validate_config()`` to report
            (§10.6.1 requirement 4).

    Mutating ``regex_patterns`` after construction does **not** recompile.
    That is the point of compiling here: the list is read once, where the
    configuration is, and the requirement-4 diagnostic is raised once for that
    configuration.  Build a new instance to change the patterns.
    """

    field_patterns: list[str] = field(default_factory=list)
    value_patterns: list[str] = field(default_factory=list)
    sensitive_keys: list[str] = field(default_factory=list)
    regex_patterns: list[str] = field(default_factory=list)
    replacement: str = REDACTED_VALUE

    compiled_regex_patterns: list[re.Pattern[str]] = field(init=False, repr=False, compare=False)
    invalid_regex_patterns: list[tuple[str, str]] = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        """Compile ``regex_patterns`` once, reporting the ones that will not.

        PROTOCOL_SPEC §10.6.1 requirement 5. The de-duplication that keeps the
        requirement-4 diagnostic to one line per bad pattern is scoped to this
        construction and nothing wider: the module-level ``set`` this replaced
        was never cleared, so the *second* configuration to carry the same
        broken pattern was told nothing — the reload case and the multi-tenant
        case, which are the two where an operator most needs telling.
        """
        self.compiled_regex_patterns, self.invalid_regex_patterns = compile_value_regexes(self.regex_patterns)

    @classmethod
    def from_config(cls, config: Any) -> RedactionConfig:
        """Build a :class:`RedactionConfig` from an :class:`apcore.config.Config`.

        Reads (Issue #43 §5):
        - ``obs.redaction.regex_patterns`` (list[str])
        - ``obs.redaction.sensitive_keys`` (list[str])
        - ``obs.redaction.replacement`` (str)

        Falls back to the namespace defaults registered in
        :mod:`apcore.config` when keys are missing.  Empty lists are
        permitted; callers that want NO redaction must explicitly set
        ``sensitive_keys: []``.
        """
        from apcore.config import _DEFAULT_OBS_REDACTION_SENSITIVE_KEYS

        regex_patterns = config.get("obs.redaction.regex_patterns", []) or []
        sensitive_keys = config.get(
            "obs.redaction.sensitive_keys",
            list(_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS),
        )
        # ``Config.get`` may return ``None`` when an explicit ``null`` is
        # written in YAML; coerce it to the default list rather than
        # silently disabling all key-based redaction.
        if sensitive_keys is None:
            sensitive_keys = list(_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS)
        replacement = config.get("obs.redaction.replacement", REDACTED_VALUE) or REDACTED_VALUE
        return cls(
            sensitive_keys=list(sensitive_keys),
            regex_patterns=list(regex_patterns),
            replacement=str(replacement),
        )

    @classmethod
    def default(cls) -> RedactionConfig:
        """Return a config seeded with the spec-default ``sensitive_keys`` list."""
        from apcore.config import _DEFAULT_OBS_REDACTION_SENSITIVE_KEYS

        return cls(sensitive_keys=list(_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS))


def _value_matches_regex(value: Any, regex_patterns: Sequence[ValuePattern]) -> bool:
    """Return True if *value* is a string matching one of ``regex_patterns``.

    Delegates to the canonical matcher in :mod:`apcore.utils.redaction` rather
    than repeating it. The two used to be separate, and the two halves of one
    rule then held on one surface each:

    * This one stringified — ``str(value)`` — so with ``regex_patterns:
      [r"\\d+"]`` the field ``amount: 42`` came back ``***REDACTED***`` at log
      emission and ``42`` from ``redact_sensitive``, one SDK giving two answers
      for one value on a rule §10.6 requires to hold on **both** surfaces.
      §10.6.1 requirement 2 now forbids the conversion outright: it is not
      merely unnecessary but unspecifiable, since the three host languages
      render one non-string value three different ways.
    * That one reported an uncompilable pattern (§10.6.1 requirement 4); this
      one swallowed ``re.error`` and continued, which is the silence the
      requirement exists to forbid — an operator-authored rule that redacts
      nothing, indistinguishable from one that works, on a surface where the
      difference is credentials in plaintext.

    Compiled patterns pass straight through, so the hot path costs nothing:
    :class:`RedactionConfig` compiles once at construction (requirement 5).
    """
    if not regex_patterns:
        return False
    return _value_matches(value, compile_value_regexes(regex_patterns)[0])


def _apply_redaction_config(data: dict[str, Any], config: RedactionConfig) -> dict[str, Any]:
    """Return a new dict with fields/values matching RedactionConfig replaced.

    Fields named in :data:`PROTECTED_LOG_FIELDS` are exempt from field-pattern
    matching so user-supplied glob patterns (e.g. ``*_id``) cannot scramble
    correlation identifiers required for log/trace stitching.

    Issue #43 §5: redaction is the **union** of legacy ``field_patterns`` /
    ``value_patterns`` and the new ``sensitive_keys`` (case-insensitive
    substring) / ``regex_patterns`` (case-insensitive value regex) lists.
    """
    result: dict[str, Any] = {}
    for key, value in data.items():
        if key in PROTECTED_LOG_FIELDS:
            result[key] = value
            continue
        # Legacy programmatic `field_patterns` is the same surface as
        # `sensitive_keys` and takes the same matcher (A25, 9.2.3). It stays
        # case-SENSITIVE, as it has always been; only the dialect changes.
        field_match = any(match_glob(pattern, key) for pattern in config.field_patterns)
        # §10.6.1 requirement 2 binds the legacy value rule as much as
        # `regex_patterns`: string values only, never a stringified number or container.
        value_match = isinstance(value, str) and any(re.search(pattern, value) for pattern in config.value_patterns)
        sensitive_key_match = _key_matches_sensitive(key, config.sensitive_keys)
        regex_value_match = _value_matches(value, config.compiled_regex_patterns)
        if field_match or value_match or sensitive_key_match or regex_value_match:
            result[key] = config.replacement
        else:
            result[key] = value
    return result


def _redact_secrets_recursive(
    value: Any,
    depth: int = 0,
    config: RedactionConfig | None = None,
) -> Any:
    """Recursively redact secret-bearing keys/values up to ``_MAX_REDACTION_DEPTH``.

    Issue #43 §5: matching is driven by a :class:`RedactionConfig` rather
    than the hard-coded ``_secret_`` prefix.  Keys whose names match
    :attr:`RedactionConfig.sensitive_keys` (case-insensitive substring or
    glob) have their values replaced with the configured replacement
    token.  String values matching :attr:`RedactionConfig.regex_patterns`
    (case-insensitive) are also redacted, regardless of key name.

    When *config* is ``None`` the spec-default sensitive-keys list is used
    (which includes ``_secret_*`` so legacy callers stay protected).
    Recursion descends into nested dicts and lists.  Beyond
    :data:`_MAX_REDACTION_DEPTH` the current node is returned as-is —
    secrets deeper than that threshold are not inspected (defensive bound
    matching the schema validation depth limit).

    Keys in :data:`PROTECTED_LOG_FIELDS` are exempt from both the name rule
    and the value regex, at every depth.  Only the protected field's own scalar
    value is immune: a container under a protected key is still descended into,
    and array elements — having no key of their own — are never protected.
    """
    if config is None:
        config = RedactionConfig.default()
    if depth > _MAX_REDACTION_DEPTH:
        return value

    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for k, v in value.items():
            # A correlation-ID key is exempt from BOTH rules below, not just the
            # name one: observability.md states the exemption unconditionally, so
            # a trace_id whose VALUE happens to match a secret regex must survive
            # too. Decided once per entry, as apcore-rust does in `redact_inner`.
            protected = isinstance(k, str) and k in PROTECTED_LOG_FIELDS
            if not protected and isinstance(k, str) and _key_matches_sensitive(k, config.sensitive_keys):
                out[k] = config.replacement
            elif not protected and isinstance(v, str) and _value_matches(v, config.compiled_regex_patterns):
                out[k] = config.replacement
            elif protected and not isinstance(v, (dict, list)):
                # Only the protected field's OWN scalar value is immune; a
                # container under a protected key falls through and is descended
                # into, matching apcore-rust.
                out[k] = v
            else:
                out[k] = _redact_secrets_recursive(v, depth + 1, config)
        return out
    if isinstance(value, list):
        # Array elements have no key of their own, so none of them is protected
        # — including elements directly under a protected key.
        return [_redact_secrets_recursive(item, depth + 1, config) for item in value]
    if isinstance(value, str) and _value_matches(value, config.compiled_regex_patterns):
        return config.replacement
    return value


class ContextLogger:
    """Standalone structured logger with context injection and redaction."""

    def __init__(
        self,
        name: str = "apcore",
        *,
        format: str | None = None,
        output_format: str = "json",
        level: str = "info",
        redact_sensitive: bool = True,
        output: Any = None,
        redaction_config: RedactionConfig | None = None,
    ) -> None:
        self._name = name
        self._output_format = format if format is not None else output_format
        self._level = level
        self._level_value = _LEVELS.get(level, 20)
        self._redact_sensitive = redact_sensitive
        self._output = output if output is not None else sys.stderr
        # Issue #43 §5: redaction is config-driven.  When no explicit config
        # is supplied, fall back to the spec-default sensitive_keys list so
        # legacy callers still get ``_secret_*`` redaction.
        self._redaction_config = redaction_config if redaction_config is not None else RedactionConfig.default()
        self._trace_id: str | None = None
        self._module_id: str | None = None
        self._caller_id: str | None = None

    @classmethod
    def from_context(cls, context: Any, name: str, **kwargs: Any) -> ContextLogger:
        """Create a logger that auto-injects trace_id, module_id, caller_id from context."""
        logger = cls(name=name, **kwargs)
        logger._trace_id = context.trace_id
        logger._module_id = context.call_chain[-1] if context.call_chain else None
        logger._caller_id = context.caller_id
        return logger

    def _emit(self, level_name: str, message: str, extra: dict[str, Any] | None) -> None:
        level_value = _LEVELS.get(level_name, 20)
        if level_value < self._level_value:
            return

        redacted_extra = extra
        if extra is not None and self._redact_sensitive:
            # Recursive defense-in-depth: redact any nested keys that match
            # the configured sensitive_keys / regex_patterns up to
            # _MAX_REDACTION_DEPTH (matches schema validation depth limit).
            redacted_extra = _redact_secrets_recursive(extra, config=self._redaction_config)

        now = datetime.now(timezone.utc)
        entry = {
            "timestamp": now.isoformat(),
            "level": level_name,
            "message": message,
            "trace_id": self._trace_id,
            "module_id": self._module_id,
            "caller_id": self._caller_id,
            "logger": self._name,
            "extra": redacted_extra,
        }

        if self._output_format == "json":
            self._output.write(json.dumps(entry, default=str) + "\n")
        else:
            ts = now.strftime("%Y-%m-%d %H:%M:%S")
            lvl = level_name.upper()
            trace = self._trace_id or "none"
            mod = self._module_id or "none"
            extras_str = ""
            if redacted_extra:
                extras_str = " " + " ".join(f"{k}={v}" for k, v in redacted_extra.items())
            self._output.write(f"{ts} [{lvl}] [trace={trace}] [module={mod}] {message}{extras_str}\n")

    def trace(self, message: str, extra: dict[str, Any] | None = None) -> None:
        self._emit("trace", message, extra)

    def debug(self, message: str, extra: dict[str, Any] | None = None) -> None:
        self._emit("debug", message, extra)

    def info(self, message: str, extra: dict[str, Any] | None = None) -> None:
        self._emit("info", message, extra)

    def warn(self, message: str, extra: dict[str, Any] | None = None) -> None:
        self._emit("warn", message, extra)

    def error(self, message: str, extra: dict[str, Any] | None = None) -> None:
        self._emit("error", message, extra)

    def fatal(self, message: str, extra: dict[str, Any] | None = None) -> None:
        self._emit("fatal", message, extra)


class ObsLoggingMiddleware(Middleware):
    """Structured observability logging middleware using ContextLogger.

    Supports ``RedactionConfig`` for runtime-configurable field/value redaction
    applied in addition to schema-level ``x-sensitive`` annotations.
    Uses stack-based timing in context.data for safe nested call support.
    """

    def __init__(
        self,
        logger: ContextLogger | None = None,
        log_inputs: bool = True,
        log_outputs: bool = True,
        redaction_config: RedactionConfig | None = None,
    ) -> None:
        self._logger = logger if logger is not None else ContextLogger(name="apcore.obs_logging")
        self._log_inputs = log_inputs
        self._log_outputs = log_outputs
        self._redaction_config = redaction_config
        # Issue #43 §5: when an explicit RedactionConfig is supplied, override
        # the logger's default (spec-default sensitive_keys) so the secondary
        # _redact_secrets_recursive pass in ContextLogger._emit honours the
        # *same* rules and does not double-redact with a broader default list.
        if redaction_config is not None:
            self._logger._redaction_config = redaction_config

    def _redact(self, data: dict[str, Any]) -> dict[str, Any]:
        """Apply RedactionConfig rules if configured."""
        if self._redaction_config is None:
            return data
        return _apply_redaction_config(data, self._redaction_config)

    def before(self, module_id: str, inputs: dict[str, Any], context: Any) -> dict[str, Any] | None:
        starts = LOGGING_STARTS.get(context, default=[])
        starts.append(time.time())
        LOGGING_STARTS.set(context, starts)
        extra: dict[str, Any] = {
            "module_id": module_id,
            "caller_id": context.caller_id,
        }
        # PROTOCOL_SPEC §10.6.1 requirement 5 (D-131): log the CAPTURED value,
        # never the raw ``inputs`` handed in — only the capture point applies the
        # ``x-sensitive`` schema rule. Nothing captured means nothing logged.
        if self._log_inputs and context.redacted_inputs is not None:
            extra["inputs"] = self._redact(context.redacted_inputs)
        self._logger.info("Module call started", extra=extra)
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Any,
    ) -> dict[str, Any] | None:
        starts = LOGGING_STARTS.get(context, default=[])
        if not starts:
            return None
        start_time = starts.pop()
        duration_ms = (time.time() - start_time) * 1000
        extra: dict[str, Any] = {
            "module_id": module_id,
            "duration_ms": duration_ms,
        }
        # D-131: ``context.redacted_output``, never the raw ``output`` — the raw
        # value carries every ``x-sensitive`` field in plaintext.
        redacted_output = getattr(context, "redacted_output", None)
        if self._log_outputs and redacted_output is not None:
            extra["output"] = self._redact(redacted_output)
        self._logger.info("Module call completed", extra=extra)
        return None

    def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Any) -> dict[str, Any] | None:
        starts = LOGGING_STARTS.get(context, default=[])
        if not starts:
            return None
        start_time = starts.pop()
        duration_ms = (time.time() - start_time) * 1000
        self._logger.error(
            "Module call failed",
            extra={
                "module_id": module_id,
                "duration_ms": duration_ms,
                "error_type": type(error).__name__,
                "error_message": str(error),
            },
        )
        return None


__all__ = [
    "ContextLogger",
    "ObsLoggingMiddleware",
    "RedactionConfig",
]
