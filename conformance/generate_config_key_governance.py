#!/usr/bin/env python3
"""Regenerate (or verify) the derived halves of `config_key_governance.json`.

`schemas/` is the single source of truth for the configuration key surface.
This script projects it into the two derived arrays the fixture pins —
`allowed_keys` and `canonical_defaults` — so the fixture can never become a
second, drifting source of truth.

    python3 conformance/generate_config_key_governance.py           # verify (CI)
    python3 conformance/generate_config_key_governance.py --write   # regenerate

Exit status is 1 when the fixture no longer matches the schemas, and the diff
names the offending keys.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FIXTURE = REPO / "conformance" / "fixtures" / "config_key_governance.json"
SENTINEL = "__NO_DEFAULT__"

# (schema file, dot-path prefix). A namespace schema describes the subtree under
# its own name, so its keys are prefixed on the way in.
SOURCES: list[tuple[str, str]] = [
    ("schemas/apcore-config.schema.json", ""),
    ("schemas/defaults.schema.json", ""),
    ("schemas/sys-modules.schema.json", "sys_modules"),
]

# The default table an SDK must reproduce verbatim. defaults.schema.json exists
# to declare it; apcore-config.schema.json's `default:` annotations document the
# user-facing config file and are not the SDK's table.
DEFAULT_TABLE_SOURCE = "schemas/defaults.schema.json"


def schema_store() -> dict:
    """Every schema in `schemas/`, keyed by basename and by `$id`.

    `$id` is a canonical identifier, not a fetch instruction. Without a local
    store a cross-file `$ref` resolves to nothing, and `flatten` used to treat
    that nothing as a scalar — `sys_modules` entered `allowed_keys` as a leaf key
    the moment `$defs/SysModulesConfig` began delegating to the sibling schema,
    while its seven real subkeys came in only because SOURCES happens to list
    that file separately. Silent degradation that still exits 0.
    """
    store: dict = {}
    for path in sorted((REPO / "schemas").glob("*.schema.json")):
        doc = json.loads(path.read_text())
        store[path.name] = doc
        if "$id" in doc:
            store[doc["$id"]] = doc
    return store


def resolve(node: dict, defs: dict, store: dict) -> tuple[dict, dict]:
    """Follow a `$ref` chain to a schema body, with the `$defs` in scope for it.

    Node and `$defs` travel together because a cross-file `$ref` changes which
    document's definitions apply. An unresolvable `$ref` raises rather than
    yielding `{}`: a typo would otherwise collapse a namespace into one scalar
    key and the generator would still report success.
    """
    for _ in range(32):
        ref = node.get("$ref")
        if ref is None:
            return node, defs
        file_part, _sep, frag = ref.partition("#")
        if file_part:
            doc = store.get(file_part)
            if doc is None:
                raise KeyError(f"unresolvable cross-file $ref {ref!r}; "
                               f"known: {', '.join(sorted(k for k in store if '://' not in k))}")
            node, defs = doc, doc.get("$defs", {})
            if not frag:
                continue
        target = defs.get(frag.split("/")[-1])
        if target is None:
            raise KeyError(f"unresolvable $ref {ref!r}")
        node = target
    raise RecursionError("$ref chain exceeded 32 hops")


def flatten(schema: dict, defs: dict, prefix: str = "", out: dict | None = None,
            seen: frozenset[str] | None = None, store: dict | None = None) -> dict:
    """Walk a config JSON Schema, yielding {dot_path: default}."""
    if out is None:
        out = {}
    if seen is None:
        seen = frozenset()
    if store is None:
        store = {}
    ref = schema.get("$ref")
    if ref is not None:
        if ref in seen:
            return out
        seen = seen | {ref}
    node, defs = resolve(schema, defs, store)
    # oneOf/anyOf branches contribute keys too — `extensions` declares `root`
    # and `roots` in exclusive branches, and both are legal config.
    for branch in [node, *node.get("oneOf", []), *node.get("anyOf", [])]:
        for key, val in branch.get("properties", {}).items():
            path = f"{prefix}.{key}" if prefix else key
            target, _ = resolve(val, defs, store)
            if target.get("type") == "object" and ("properties" in target or "oneOf" in target):
                flatten(val, defs, path, out, seen, store)
            else:
                out[path] = val.get("default", SENTINEL)
    return out


def load(rel: str, prefix: str = "", store: dict | None = None) -> dict:
    doc = json.loads((REPO / rel).read_text())
    return flatten(doc, doc.get("$defs", {}), prefix, store=store)


def derive() -> dict:
    store = schema_store()
    allowed: dict = {}
    for rel, prefix in SOURCES:
        allowed.update(load(rel, prefix, store))
    # sys-modules.schema.json is reached twice — once through apcore-config's
    # $defs/SysModulesConfig delegation, once as its own SOURCES entry. Both
    # yield the same prefixed paths, so the merge is idempotent; the explicit
    # entry stays because §9.10 names it a canonical source in its own right.
    canon = {k: v for k, v in load(DEFAULT_TABLE_SOURCE, "", store).items() if v != SENTINEL}
    return {
        "allowed_keys": sorted(allowed),
        "canonical_defaults": dict(sorted(canon.items())),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--write", action="store_true", help="regenerate instead of verifying")
    args = ap.parse_args()

    derived = derive()
    fixture = json.loads(FIXTURE.read_text())

    if args.write:
        fixture["allowed_keys"] = derived["allowed_keys"]
        fixture["canonical_defaults"] = derived["canonical_defaults"]
        FIXTURE.write_text(json.dumps(fixture, indent=2, ensure_ascii=False) + "\n")
        print(f"regenerated {FIXTURE.relative_to(REPO)}: "
              f"{len(derived['allowed_keys'])} allowed keys, "
              f"{len(derived['canonical_defaults'])} canonical defaults")
        return 0

    failed = False

    stale = set(fixture["allowed_keys"]) - set(derived["allowed_keys"])
    fresh = set(derived["allowed_keys"]) - set(fixture["allowed_keys"])
    if stale or fresh:
        failed = True
        print("allowed_keys is out of date:", file=sys.stderr)
        for k in sorted(fresh):
            print(f"  + {k}  (declared by a schema, missing from the fixture)", file=sys.stderr)
        for k in sorted(stale):
            print(f"  - {k}  (in the fixture, declared by no schema)", file=sys.stderr)

    fx, dv = fixture["canonical_defaults"], derived["canonical_defaults"]
    for key in sorted(set(fx) | set(dv)):
        if key not in fx:
            failed = True
            print(f"canonical_defaults missing {key} = {dv[key]!r}", file=sys.stderr)
        elif key not in dv:
            failed = True
            print(f"canonical_defaults has {key}, no schema declares a default", file=sys.stderr)
        elif fx[key] != dv[key]:
            failed = True
            print(f"canonical_defaults {key}: fixture={fx[key]!r} schema={dv[key]!r}", file=sys.stderr)

    if failed:
        print("\nRun: python3 conformance/generate_config_key_governance.py --write",
              file=sys.stderr)
        return 1

    print(f"config_key_governance.json is in sync with schemas/ "
          f"({len(derived['allowed_keys'])} allowed keys, "
          f"{len(derived['canonical_defaults'])} canonical defaults)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
