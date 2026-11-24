"""Wildcard pattern matching.

``match_pattern`` (Algorithm A08) matches **module IDs** — ACL rule patterns and
pipeline ``match_modules``.  ``match_glob`` (Algorithm A25) matches every other
pattern-valued value in the specification: binding filenames, redaction field
names, event types and ``system.control.reload_module``'s ``path_filter``.

The two are deliberately separate and PROTOCOL_SPEC §9.2.3 says why: A08 has
``*`` alone, and promoting ``?`` there would widen ACL ``allow`` rules that are
inert today (§2.7 forbids ``?`` in a module ID), which is the one direction an
authorization matcher must not move silently.
"""

from __future__ import annotations

__all__ = ["match_pattern", "match_glob", "calculate_specificity"]


def match_pattern(pattern: str, module_id: str) -> bool:
    """Match a module ID against a wildcard pattern (Algorithm A08).

    Supports '*' as a wildcard that matches any sequence of characters
    including dots.

    Args:
        pattern: The pattern to match against. May contain '*' wildcards.
        module_id: The module Canonical ID to test.

    Returns:
        True if the module_id matches the pattern, False otherwise.
    """
    if pattern == "*":
        return True
    if "*" not in pattern:
        return pattern == module_id

    segments = pattern.split("*")
    pos = 0

    if not pattern.startswith("*"):
        if not module_id.startswith(segments[0]):
            return False
        pos = len(segments[0])

    for segment in segments[1:]:
        if not segment:
            continue
        idx = module_id.find(segment, pos)
        if idx == -1:
            return False
        pos = idx + len(segment)

    if not pattern.endswith("*"):
        if not module_id.endswith(segments[-1]):
            return False

    return True


def match_glob(pattern: str, value: str) -> bool:
    """Match *value* against a glob-dialect *pattern* (Algorithm A25).

    PROTOCOL_SPEC §9.2.3.  This is the matcher for every glob-dialect
    pattern-valued value in the specification: ``bindings.pattern``,
    ``obs.redaction.sensitive_keys`` glob entries, event ``event_pattern`` /
    ``include_events`` / ``exclude_events``, and ``path_filter``.

    Exactly two metacharacters:

    - ``*`` — zero or more characters, crossing ``.`` and ``/``
    - ``?`` — exactly one character

    **Every other character is a literal**, ``[``, ``]``, ``{``, ``}``, ``\\``,
    ``!``, ``^`` and ``-`` included.  There is no escape character, and the
    match is anchored to the whole value.

    **Do not replace this with :mod:`fnmatch`.**  ``fnmatch`` supports
    character classes (``[ab]``) and their negation (``[!a]``); the ``glob``
    crate and a translated ``RegExp`` each read those differently again, and
    that is precisely the divergence this function exists to remove (#116,
    #117).  Every string is a valid pattern here — there is no parse phase and
    this function never raises.

    Args:
        pattern: The pattern.  Any string is accepted.
        value: The name to test — a filename, field name, event type or
            module ID, depending on the surface.

    Returns:
        True when the pattern matches the entire value.
    """
    segments = pattern.split("*")
    if len(segments) == 1:
        return _match_exact(segments[0], value)

    if not _match_prefix(segments[0], value):
        return False
    pos = len(segments[0])

    for segment in segments[1:-1]:
        if not segment:
            continue
        end = len(value) - len(segment)
        idx = -1
        i = pos
        while i <= end:
            if _match_exact(segment, value[i : i + len(segment)]):
                idx = i
                break
            i += 1
        if idx == -1:
            return False
        pos = idx + len(segment)

    last = segments[-1]
    if not last:
        return True
    if len(value) - pos < len(last):
        return False
    return _match_exact(last, value[len(value) - len(last) :])


def _match_prefix(segment: str, text: str) -> bool:
    """True when *text* starts with *segment*, treating ``?`` as any character."""
    if len(text) < len(segment):
        return False
    return all(sc == "?" or sc == tc for sc, tc in zip(segment, text))


def _match_exact(segment: str, text: str) -> bool:
    """True when *segment* covers *text* exactly, treating ``?`` as any character."""
    return len(segment) == len(text) and _match_prefix(segment, text)


def calculate_specificity(pattern: str) -> int:
    """Calculate the specificity score of an ACL pattern (Algorithm A10).

    Higher scores indicate more specific patterns.  Scoring per segment:

    - ``"*"`` (pure wildcard) → 0
    - Segment containing ``"*"`` (partial wildcard) → +1
    - Exact segment (no wildcard) → +2

    Examples::

        "*"                       → 0
        "api.*"                   → 2  (exact "api" + wildcard "*")
        "api.handler.*"           → 4
        "api.handler.task_submit" → 6

    Args:
        pattern: An ACL pattern string.

    Returns:
        Non-negative integer specificity score.
    """
    if pattern == "*":
        return 0

    score = 0
    for segment in pattern.split("."):
        if segment == "*":
            pass  # +0
        elif "*" in segment:
            score += 1
        else:
            score += 2
    return score
