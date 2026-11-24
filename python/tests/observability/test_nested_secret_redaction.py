"""Regression test for OBS-008: bounded depth-32 redaction recursion.

The previous implementation only redacted ``_secret_*`` keys at the top level
of ``extra``.  Secrets buried inside nested dicts/lists were leaked.  This
test pins the recursive behaviour with a hard cap at depth 32 (matching the
schema validation depth limit) so adversarial / cyclic-looking input cannot
cause runaway recursion.
"""

from __future__ import annotations

import io
import json

from apcore.observability.context_logger import ContextLogger


class TestNestedSecretRedaction:
    """Secret keys nested inside dicts/lists must also be redacted."""

    def test_nested_dict_secret_redacted(self) -> None:
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf, redact_sensitive=True)
        logger.info("msg", extra={"user": {"_secret_token": "abc", "name": "alice"}})
        entry = json.loads(buf.getvalue().strip())
        nested = entry["extra"]["user"]
        assert nested["_secret_token"] == "***REDACTED***"
        assert nested["name"] == "alice"

    def test_deeply_nested_secret_redacted(self) -> None:
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf, redact_sensitive=True)
        # 5 levels deep
        payload = {"a": {"b": {"c": {"d": {"_secret_key": "xxx", "ok": "v"}}}}}
        logger.info("msg", extra=payload)
        entry = json.loads(buf.getvalue().strip())
        leaf = entry["extra"]["a"]["b"]["c"]["d"]
        assert leaf["_secret_key"] == "***REDACTED***"
        assert leaf["ok"] == "v"

    def test_secret_inside_list_redacted(self) -> None:
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf, redact_sensitive=True)
        logger.info(
            "msg",
            extra={"items": [{"_secret_x": "1", "v": 2}, {"v": 3}]},
        )
        entry = json.loads(buf.getvalue().strip())
        items = entry["extra"]["items"]
        assert items[0]["_secret_x"] == "***REDACTED***"
        assert items[0]["v"] == 2
        assert items[1] == {"v": 3}

    def test_recursion_stops_beyond_depth_32(self) -> None:
        """Beyond depth 32 the recursion must NOT descend further (defensive cap)."""
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf, redact_sensitive=True)

        # Build extra such that _secret_x sits at depth 33 (beyond the cap).
        leaf = {"_secret_x": "should_remain"}
        node: dict = leaf
        for _ in range(33):
            node = {"n": node}
        logger.info("msg", extra=node)
        entry = json.loads(buf.getvalue().strip())

        cur = entry["extra"]
        for _ in range(33):
            cur = cur["n"]
        assert (
            cur.get("_secret_x") == "should_remain"
        ), "Recursion must stop at depth 32 — secrets deeper than that are out of scope"

    def test_recursion_reaches_depth_32(self) -> None:
        """At exactly depth 32 redaction still applies."""
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf, redact_sensitive=True)

        leaf = {"_secret_x": "redact_me"}
        node: dict = leaf
        for _ in range(32):
            node = {"n": node}
        logger.info("msg", extra=node)
        entry = json.loads(buf.getvalue().strip())

        cur = entry["extra"]
        for _ in range(32):
            cur = cur["n"]
        assert cur.get("_secret_x") == "***REDACTED***"
