"""Config-driven ACL discovery — the `acl.root` path (D-64, issue #74).

This is the *config-driven* counterpart to `acl_agent_governance.py`.

  - `acl_agent_governance.py` shows the MANUAL path: build an `ACL` in code and
    attach it with `client.executor.set_acl(acl)`.
  - This script shows the CONFIG-DRIVEN path: declare `acl.root` in `apcore.yaml`,
    drop a policy file at `acl/global_acl.yaml`, and let `APCore(config=...)`
    AUTO-WIRE enforcement via `ACL.discover(config)` — no `set_acl` call at all.

`acl.root` is resolved relative to the directory of the config file, so the
config and its `acl/` directory travel together. To keep the demo self-contained
and runnable from anywhere, it materializes a throwaway project dir in a temp
location, writes `apcore.yaml` + `acl/global_acl.yaml` into it, then loads it
with `Config.load(<dir>/apcore.yaml)`.

The policy is a small first-match-wins default-deny ACL:
  - allow `@external` (unauthenticated callers) -> `executor.greeter.greet`
  - deny everything else (the `default_effect: deny` floor)

The script proves enforcement is live without any manual wiring: an allowed call
returns a real result, and a denied inter-module call raises `ACLDeniedError`.

Run (from the apcore-python repo root):
    python examples/acl_config_driven.py
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

from apcore import APCore, ACLDeniedError, Context
from apcore.config import Config

# ---------------------------------------------------------------------------
# 1. Materialize a tiny project on disk: apcore.yaml + acl/global_acl.yaml.
#    Co-located so `acl.root: ./acl` resolves relative to the config file.
# ---------------------------------------------------------------------------
APCORE_YAML = """\
apcore:
  acl:
    root: ./acl
    default_effect: deny
"""

# First-match-wins, default-deny. Allow unauthenticated callers to greet only.
GLOBAL_ACL_YAML = """\
default_effect: deny
rules:
  - description: External callers may greet, nothing else.
    callers: ["@external"]
    targets: ["executor.greeter.greet"]
    effect: allow
"""


def write_project(root: Path) -> Path:
    """Write apcore.yaml + acl/global_acl.yaml under `root`; return the config path."""
    config_path = root / "apcore.yaml"
    config_path.write_text(APCORE_YAML, encoding="utf-8")

    acl_dir = root / "acl"
    acl_dir.mkdir(parents=True, exist_ok=True)
    (acl_dir / "global_acl.yaml").write_text(GLOBAL_ACL_YAML, encoding="utf-8")
    return config_path


def main() -> int:
    print("Config-driven ACL discovery — acl.root (D-64, issue #74)")
    print("=" * 72)

    with tempfile.TemporaryDirectory(prefix="apcore-acl-demo-") as tmp:
        root = Path(tmp)
        config_path = write_project(root)
        print(f"Project dir: {root}")
        print(f"  apcore.yaml          -> acl.root: ./acl, default_effect: deny")
        print(f"  acl/global_acl.yaml  -> allow @external -> executor.greeter.greet")
        print("-" * 72)

        # -------------------------------------------------------------------
        # 2. Load config and construct APCore — ACL is auto-wired by
        #    ACL.discover(config) inside __init__. Note: NO set_acl() here.
        # -------------------------------------------------------------------
        config = Config.load(str(config_path))
        client = APCore(config=config)

        @client.module(id="executor.greeter.greet", description="Return a greeting")
        def greet(name: str) -> dict:
            return {"message": f"Hello, {name}!"}

        @client.module(id="executor.vault.read_secret", description="Read a secret")
        def read_secret(key: str) -> dict:
            return {"key": key, "value": "s3cr3t"}

        print("APCore(config=...) auto-wired the ACL via ACL.discover (no manual set_acl).")
        print("-" * 72)

        failures = 0

        # -------------------------------------------------------------------
        # 3a. Allowed call: @external (no caller_id) -> executor.greeter.greet.
        # -------------------------------------------------------------------
        try:
            result = client.call("executor.greeter.greet", {"name": "apcore"}, context=Context.create())
            print(f"ALLOW  @external -> executor.greeter.greet   => {result}")
        except ACLDeniedError as exc:
            failures += 1
            print(f"FAIL   expected ALLOW for executor.greeter.greet, got deny: {exc}")

        # -------------------------------------------------------------------
        # 3b. Denied call: @external -> executor.vault.read_secret (default deny).
        # -------------------------------------------------------------------
        try:
            client.call("executor.vault.read_secret", {"key": "api_token"}, context=Context.create())
            failures += 1
            print("FAIL   expected DENY for executor.vault.read_secret, but it succeeded")
        except ACLDeniedError:
            print("DENY   @external -> executor.vault.read_secret => ACLDeniedError (default_effect: deny)")

    print("=" * 72)
    if failures:
        print(f"{failures} check(s) failed.")
        return 1
    print("Config-driven ACL enforcement is active: allow + deny both verified.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
