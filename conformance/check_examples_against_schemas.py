"""Validate the repository's config and module document examples.

`schemas/` is described as canonical, but nothing checked that the files this
repository ships actually satisfy it. They did not:
`examples/acl/agent-tool-governance.yaml` failed `acl-config.schema.json` in
three ways at once — `conditions.roles` rejected twice because the schema
declared `identity_roles` (a name in no doc, example or fixture), and
`version: "1.0"` rejected because the schema demanded full semver while every
documented ACL example in the repo uses the two-part form.

A schema nobody validates against is a schema that drifts, and it drifts in the
direction that makes tooling generated from it wrong. `check_doc_examples.py`
resolves imported symbols against the SDKs; this checks document shapes.

    python3 conformance/check_examples_against_schemas.py
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

try:
    import yaml
    from jsonschema import Draft202012Validator
    from jsonschema.exceptions import SchemaError
except ImportError as exc:  # pragma: no cover - CI installs both
    print(f"missing dependency: {exc}", file=sys.stderr)
    raise SystemExit(1) from exc

REPO = Path(__file__).resolve().parent.parent

# Each entry maps a schema to the example files it governs. Add a row when a new
# schema gains shipped examples — an unmapped example is not checked, which is
# the state this script exists to end.
BINDINGS: list[tuple[str, list[str]]] = [
    ("schemas/acl-config.schema.json", ["examples/acl/*.yaml"]),
]

MODULE_SCHEMA_EXAMPLES = (
    "docs/spec/protocol-spec.md",
    "docs/guides/schema-definition.md",
    "docs/guides/multi-language.md",
)
YAML_FENCE = re.compile(r"^```yaml\n(.*?)^```$", re.MULTILINE | re.DOTALL)
MODULE_FIELDS = re.compile(r"^(module_id|input_schema|output_schema):", re.MULTILINE)


def validate_module_schema_examples() -> tuple[int, int]:
    """Validate the complete module Schema file examples from the docs."""
    schema = json.loads((REPO / "schemas/module-schema.schema.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    failures = 0
    checked = 0
    for rel in MODULE_SCHEMA_EXAMPLES:
        markdown = (REPO / rel).read_text()
        examples = [
            match
            for match in YAML_FENCE.finditer(markdown)
            if len(set(MODULE_FIELDS.findall(match[1]))) == 3
        ]
        if not examples:
            print(f"{rel}: no complete module Schema example found", file=sys.stderr)
            failures += 1
        for match in examples:
            line = markdown.count("\n", 0, match.start()) + 1
            location = f"{rel}:{line}"
            try:
                doc = yaml.safe_load(match[1])
            except yaml.YAMLError as exc:
                print(f"{location}: YAML parse error: {exc}", file=sys.stderr)
                failures += 1
                continue
            checked += 1
            if not isinstance(doc, dict) or doc.get("$schema") != schema["$id"]:
                print(f"{location}: expected $schema {schema['$id']}", file=sys.stderr)
                failures += 1
            for error in validator.iter_errors(doc):
                print(f"{location}: {error.json_path}: {error.message}", file=sys.stderr)
                failures += 1
    return checked, failures


def main() -> int:
    checked, failures = validate_module_schema_examples()
    for schema_rel, patterns in BINDINGS:
        schema_path = REPO / schema_rel
        if not schema_path.is_file():
            print(f"{schema_rel}: schema not found", file=sys.stderr)
            failures += 1
            continue
        schema = json.loads(schema_path.read_text())
        try:
            Draft202012Validator.check_schema(schema)
        except SchemaError as exc:
            print(f"{schema_rel}: not a valid Draft 2020-12 schema: {exc}", file=sys.stderr)
            failures += 1
            continue
        validator = Draft202012Validator(schema)

        matched: list[Path] = []
        for pattern in patterns:
            matched.extend(sorted(REPO.glob(pattern)))
        if not matched:
            print(f"{schema_rel}: no example files matched {patterns}", file=sys.stderr)
            failures += 1
            continue

        for path in matched:
            rel = path.relative_to(REPO)
            try:
                doc = yaml.safe_load(path.read_text())
            except (OSError, yaml.YAMLError) as exc:
                print(f"{rel}: YAML parse error: {exc}", file=sys.stderr)
                failures += 1
                continue
            checked += 1
            errors = sorted(validator.iter_errors(doc), key=lambda e: list(e.path))
            for err in errors:
                location = ".".join(str(p) for p in err.path) or "(root)"
                print(f"{rel}: {location}: {err.message}", file=sys.stderr)
                failures += 1

    print(f"{checked} examples validated against {len(BINDINGS) + 1} schemas")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
