"""Publish the canonical module document Schema at its declared URI."""

from pathlib import Path

from mkdocs.structure.files import File


def on_files(files, config):
    schema_uri = "schemas/module-schema.schema.json"
    source = Path(__file__).resolve().parent.parent / schema_uri
    files.append(File.generated(config, schema_uri, abs_src_path=str(source)))
    return files
