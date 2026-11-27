"""The reload module validates the supported input schema."""

import jsonschema
import pytest

from apcore.sys_modules.control import ReloadModule


def test_reload_dependents_is_not_a_supported_property() -> None:
    jsonschema.validate({"module_id": "example.module", "reason": "reload"}, ReloadModule.input_schema)
    with pytest.raises(jsonschema.ValidationError, match="reload_dependents"):
        jsonschema.validate(
            {"module_id": "example.module", "reason": "reload", "reload_dependents": True},
            ReloadModule.input_schema,
        )
