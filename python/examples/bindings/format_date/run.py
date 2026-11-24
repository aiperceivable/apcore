"""Demonstrate loading a YAML binding and calling it via the Executor."""

import pathlib
import sys

from apcore.bindings import BindingLoader
from apcore.executor import Executor
from apcore.registry import Registry

# Make format_date.py importable by adding this directory to sys.path.
sys.path.insert(0, str(pathlib.Path(__file__).parent))

binding_file = pathlib.Path(__file__).parent / "format_date.binding.yaml"

registry = Registry()
loader = BindingLoader()
loader.load_bindings(str(binding_file), registry)

executor = Executor(registry=registry)
result = executor.call(
    "utils.format_date",
    {"date_string": "2024-01-15", "output_format": "%B %d, %Y"},
)
print(result)  # {'formatted': 'January 15, 2024'}
