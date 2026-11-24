"""Tests for config-driven ACL discovery (D-64, Recommendation A; issue #74).

Covers ``ACL.discover(config)`` resolution and the executor-level enforcement
that results from wiring the discovered ACL via ``set_acl``.

Critical invariant under test: a missing ``acl.root`` path attaches NO ACL
(no enforcement), it must never synthesize an empty default-deny ACL.
"""

from __future__ import annotations

import textwrap
from pathlib import Path

import pytest

from apcore.acl import ACL
from apcore.config import Config
from apcore.context import Context
from apcore.errors import ACLDeniedError
from apcore.executor import Executor

# An ACL that allows external callers to reach "greet" but denies everything
# else (first-match-wins, explicit default deny).
ACL_FILE_CONTENT = textwrap.dedent(
    """\
    default_effect: deny
    rules:
      - callers: ["@external"]
        targets: ["greet"]
        effect: allow
        description: "Allow external callers to access greet"
      - callers: ["*"]
        targets: ["*"]
        effect: deny
        description: "Default deny all other access"
    """
)


def _write_global_acl(acl_dir: Path) -> Path:
    """Create ``<acl_dir>/global_acl.yaml`` with the shared test policy."""
    acl_dir.mkdir(parents=True, exist_ok=True)
    acl_file = acl_dir / "global_acl.yaml"
    acl_file.write_text(ACL_FILE_CONTENT)
    return acl_file


class TestACLDiscoverDefault:
    """The unified default value of ``acl.root`` is ``./acl`` (D-64)."""

    def test_default_acl_root_value(self) -> None:
        assert Config.get_default("acl.root") == "./acl"


class TestACLDiscoverPresent:
    """A present ACL directory yields an enforcing ACL."""

    def test_discover_returns_acl_when_dir_present(self, tmp_path: Path) -> None:
        _write_global_acl(tmp_path / "acl")
        config = Config(data={"acl": {"root": str(tmp_path / "acl")}})

        acl = ACL.discover(config)

        assert acl is not None
        # The loaded policy enforces the file's rules.
        assert acl.check(caller_id=None, target_id="greet") is True
        assert acl.check(caller_id="some.module", target_id="db.write") is False

    def test_discover_returns_acl_when_root_is_file(self, tmp_path: Path) -> None:
        """When ``acl.root`` points directly at a file, load it as-is."""
        acl_file = tmp_path / "custom_acl.yaml"
        acl_file.write_text(ACL_FILE_CONTENT)
        config = Config(data={"acl": {"root": str(acl_file)}})

        acl = ACL.discover(config)

        assert acl is not None
        assert acl.check(caller_id=None, target_id="greet") is True

    def test_discover_resolves_relative_to_config_source(self, tmp_path: Path) -> None:
        """A relative ``acl.root`` resolves against the config file directory."""
        project_dir = tmp_path / "project"
        project_dir.mkdir()
        _write_global_acl(project_dir / "acl")

        config_file = project_dir / "apcore.yaml"
        config_file.write_text(
            textwrap.dedent(
                """\
                version: "0.16.0"
                extensions:
                  root: "./extensions"
                schema:
                  root: "./schemas"
                acl:
                  root: "./acl"
                  default_effect: deny
                project:
                  name: "discovery-test"
                """
            )
        )
        config = Config.load(str(config_file), validate=False)

        acl = ACL.discover(config)

        assert acl is not None
        assert acl.check(caller_id=None, target_id="greet") is True

    def test_present_dir_enforces_through_executor(self, int_registry, tmp_path: Path) -> None:
        """An inter-module call denied by the discovered ACL is blocked (ACLDeniedError)."""
        _write_global_acl(tmp_path / "acl")
        config = Config(data={"acl": {"root": str(tmp_path / "acl")}})

        acl = ACL.discover(config)
        assert acl is not None
        executor = Executor(registry=int_registry)
        executor.set_acl(acl)

        # @external -> greet is allowed by the policy.
        assert executor.call("greet", {"name": "Alice"}) == {"message": "Hello, Alice!"}

        # A non-external caller is denied by the explicit "* -> * deny" rule.
        ctx = Context.create()
        ctx.caller_id = "unauthorized.caller"
        ctx.call_chain = ["unauthorized.caller"]
        with pytest.raises(ACLDeniedError):
            executor.call("greet", {"name": "Alice"}, context=ctx)


class TestACLDiscoverMissing:
    """A missing ACL path is a no-op: NO ACL attached, NO enforcement."""

    def test_discover_returns_none_for_missing_dir(self, tmp_path: Path) -> None:
        missing = tmp_path / "does_not_exist"
        config = Config(data={"acl": {"root": str(missing)}})

        assert ACL.discover(config) is None

    def test_discover_returns_none_for_empty_dir_without_acl_file(self, tmp_path: Path) -> None:
        """Directory exists but has no global_acl.yaml -> still a no-op (None)."""
        empty_dir = tmp_path / "acl"
        empty_dir.mkdir()
        config = Config(data={"acl": {"root": str(empty_dir)}})

        assert ACL.discover(config) is None

    def test_missing_dir_does_not_block_calls(self, int_registry, tmp_path: Path) -> None:
        """Missing acl.root -> executor has no ACL -> inter-module calls NOT blocked."""
        missing = tmp_path / "does_not_exist"
        config = Config(data={"acl": {"root": str(missing)}})

        acl = ACL.discover(config)
        assert acl is None

        # No ACL is attached; the call must succeed (no ACLDeniedError).
        executor = Executor(registry=int_registry)
        ctx = Context.create()
        ctx.caller_id = "unauthorized.caller"
        ctx.call_chain = ["unauthorized.caller"]
        result = executor.call("greet", {"name": "Alice"}, context=ctx)
        assert result == {"message": "Hello, Alice!"}

    def test_missing_dir_does_not_synthesize_default_deny(self, tmp_path: Path) -> None:
        """Regression guard for the D-64 invariant: missing path != empty deny ACL."""
        missing = tmp_path / "nope"
        config = Config(data={"acl": {"root": str(missing), "default_effect": "deny"}})

        # default_effect=deny in config must NOT produce a deny-everything ACL.
        assert ACL.discover(config) is None


class TestACLDiscoverCallerSuppliedExecutor:
    """Auto-discovery must not clobber a caller-supplied Executor's ACL.

    Cross-SDK parity (#74): the TypeScript and Rust SDKs skip config-driven
    discovery when the caller passes their own Executor, so that an ACL the
    caller wired explicitly is never overwritten. Python matches.
    """

    def test_supplied_executor_acl_not_overwritten(self, tmp_path: Path) -> None:
        from apcore.client import APCore
        from apcore.registry import Registry

        # A present acl dir that WOULD be discovered if APCore built the executor.
        _write_global_acl(tmp_path / "acl")
        config = Config(data={"acl": {"root": str(tmp_path / "acl")}})

        # Caller supplies their own executor with no ACL set.
        caller_executor = Executor(registry=Registry())
        assert caller_executor._acl is None

        APCore(config=config, executor=caller_executor)

        # Discovery must have been skipped: the caller's executor is untouched.
        assert caller_executor._acl is None
