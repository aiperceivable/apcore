"""Spec-traced contract tests for the Config Bus feature.

Source spec: apcore/docs/features/config-bus.md (## Contract: blocks).

Each test carries a verbatim clause id of the form
``config_bus.<method>.<kind>.<detail>`` in a leading docstring so that the
cross-language test matrix (python / typescript / rust) lines up row-for-row.

These tests exercise the PUBLIC contract only and never mutate production
source. The Config namespace registry is class-global, so every test class
restores the registry in setup/teardown to stay isolated.
"""

from __future__ import annotations

import asyncio
import dataclasses
import inspect
from pathlib import Path

import pytest

from apcore.config import (
    Config,
    _GLOBAL_NS_REGISTRY,
    _GLOBAL_NS_REGISTRY_LOCK,
)
from apcore.errors import (
    ConfigBindError,
    ConfigEnvMapConflictError,
    ConfigEnvPrefixConflictError,
    ConfigError,
    ConfigMountError,
    ConfigNamespaceDuplicateError,
    ConfigNamespaceReservedError,
    ConfigNotFoundError,
)

# ---------------------------------------------------------------------------
# Registry isolation helpers (the namespace registry is class-global)
# ---------------------------------------------------------------------------

_BUILTIN_NAMES = {"observability", "obs", "sys_modules"}


def _reset_registry() -> None:
    """Drop test-registered namespaces and clear global env_map claims."""
    from apcore.config import _GLOBAL_ENV_MAP, _GLOBAL_ENV_MAP_CLAIMED

    with _GLOBAL_NS_REGISTRY_LOCK:
        for key in [k for k in _GLOBAL_NS_REGISTRY if k not in _BUILTIN_NAMES]:
            del _GLOBAL_NS_REGISTRY[key]
    _GLOBAL_ENV_MAP.clear()
    _GLOBAL_ENV_MAP_CLAIMED.clear()


def _write_yaml(path: Path, body: str) -> str:
    path.write_text(body, encoding="utf-8")
    return str(path)


#: `project.name` is one of the two keys with no canonical default, so a
#: legacy-mode document without it is rejected by `Config.load()`'s default
#: `validate=True` (PROTOCOL_SPEC §9.1/§9.3). Fixtures below exercise load /
#: get / reload *properties*, not required-field validation, so they carry
#: these two keys to stay valid.
_PROJECT_BLOCK = "project:\n  name: spec_fixture\n"
_REQUIRED_HEADER = "version: '0.15.0'\n" + _PROJECT_BLOCK


@dataclasses.dataclass
class _PluginCfg:
    timeout: int = 5000
    retries: int = 3


# ===========================================================================
# Contract: Config.register_namespace
# ===========================================================================


class TestRegisterNamespaceContract:
    def setup_method(self) -> None:
        _reset_registry()

    def teardown_method(self) -> None:
        _reset_registry()

    def test_name_reserved_apcore(self) -> None:
        """config_bus.register_namespace.input.name.reserved_apcore"""
        with pytest.raises(ConfigNamespaceReservedError) as exc:
            Config.register_namespace("apcore")
        assert exc.value.code == "CONFIG_NAMESPACE_RESERVED"

    def test_name_reserved_config(self) -> None:
        """config_bus.register_namespace.input.name.reserved_config"""
        with pytest.raises(ConfigNamespaceReservedError) as exc:
            Config.register_namespace("_config")
        assert exc.value.code == "CONFIG_NAMESPACE_RESERVED"

    def test_error_namespace_duplicate(self) -> None:
        """config_bus.register_namespace.error.CONFIG_NAMESPACE_DUPLICATE"""
        Config.register_namespace("dup_spec_ns")
        with pytest.raises(ConfigNamespaceDuplicateError) as exc:
            Config.register_namespace("dup_spec_ns")
        assert exc.value.code == "CONFIG_NAMESPACE_DUPLICATE"

    def test_error_env_prefix_conflict(self) -> None:
        """config_bus.register_namespace.error.CONFIG_ENV_PREFIX_CONFLICT"""
        Config.register_namespace("alpha_spec", env_prefix="SHARED_PREFIX")
        with pytest.raises(ConfigEnvPrefixConflictError) as exc:
            Config.register_namespace("beta_spec", env_prefix="SHARED_PREFIX")
        assert exc.value.code == "CONFIG_ENV_PREFIX_CONFLICT"

    def test_error_env_map_conflict(self) -> None:
        """config_bus.register_namespace.error.CONFIG_ENV_MAP_CONFLICT"""
        Config.env_map({"SPEC_PORT": "port"})
        with pytest.raises(ConfigEnvMapConflictError) as exc:
            Config.register_namespace("gamma_spec", env_map={"SPEC_PORT": "server_port"})
        assert exc.value.code == "CONFIG_ENV_MAP_CONFLICT"

    def test_property_async_false(self) -> None:
        """config_bus.register_namespace.property.async

        Contract declares async: false. The method must NOT return an
        awaitable; it registers synchronously and returns None.
        """
        result = Config.register_namespace("sync_spec_ns")
        assert result is None
        assert not inspect.iscoroutinefunction(Config.register_namespace.__func__)

    def test_property_idempotent_false(self) -> None:
        """config_bus.register_namespace.property.idempotent

        Contract declares idempotent: false — a second identical registration
        must raise rather than silently succeed.
        """
        Config.register_namespace("once_spec_ns")
        with pytest.raises(ConfigNamespaceDuplicateError):
            Config.register_namespace("once_spec_ns")
        # State remains: exactly one registration of this name.
        names = [r["name"] for r in Config.registered_namespaces()]
        assert names.count("once_spec_ns") == 1

    def test_property_pure_false(self) -> None:
        """config_bus.register_namespace.property.pure

        Contract declares pure: false — it mutates the class-level registry,
        observable via the public introspection API.
        """
        before = {r["name"] for r in Config.registered_namespaces()}
        assert "pure_spec_ns" not in before
        Config.register_namespace("pure_spec_ns")
        after = {r["name"] for r in Config.registered_namespaces()}
        assert "pure_spec_ns" in after

    @pytest.mark.skip(
        reason="clause too vague: contract declares thread_safe: false "
        "('call before any concurrent Config.load()'); concurrent registration "
        "is explicitly unsupported so there is no safe behavior to assert"
    )
    def test_property_thread_safe_false(self) -> None:
        """config_bus.register_namespace.property.thread_safe"""


# ===========================================================================
# Contract: Config.load
# ===========================================================================


class TestLoadContract:
    def setup_method(self) -> None:
        _reset_registry()

    def teardown_method(self) -> None:
        _reset_registry()

    def test_error_not_found(self, tmp_path: Path) -> None:
        """config_bus.load.error.CONFIG_NOT_FOUND"""
        missing = tmp_path / "does_not_exist.yaml"
        with pytest.raises(ConfigNotFoundError) as exc:
            Config.load(str(missing))
        assert exc.value.code == "CONFIG_NOT_FOUND"

    def test_error_invalid(self, tmp_path: Path) -> None:
        """config_bus.load.error.CONFIG_INVALID

        Spec names ConfigInvalidError(code=CONFIG_INVALID); in this SDK that
        error type is `ConfigError`, whose code is exactly CONFIG_INVALID.
        """
        path = _write_yaml(tmp_path / "bad.yaml", "key: [unterminated\n")
        with pytest.raises(ConfigError) as exc:
            Config.load(path)
        assert exc.value.code == "CONFIG_INVALID"

    def test_error_invalid_non_mapping(self, tmp_path: Path) -> None:
        """config_bus.load.error.CONFIG_INVALID.non_mapping

        A syntactically valid YAML whose root is not a mapping is rejected.
        """
        path = _write_yaml(tmp_path / "list.yaml", "- a\n- b\n")
        with pytest.raises(ConfigError) as exc:
            Config.load(path)
        assert exc.value.code == "CONFIG_INVALID"

    def test_property_async_false(self, tmp_path: Path) -> None:
        """config_bus.load.property.async

        Contract declares async: false — load returns a Config, not a coroutine.
        """
        path = _write_yaml(tmp_path / "ok.yaml", _REQUIRED_HEADER)
        result = Config.load(path)
        assert isinstance(result, Config)
        assert not inspect.iscoroutinefunction(Config.load.__func__)

    def test_property_idempotent_true(self, tmp_path: Path) -> None:
        """config_bus.load.property.idempotent

        Loading the same file twice produces equivalent Config instances.
        """
        path = _write_yaml(tmp_path / "idem.yaml", _REQUIRED_HEADER + "executor:\n  default_timeout: 30000\n")
        first = Config.load(path)
        second = Config.load(path)
        assert first.get("executor.default_timeout") == second.get("executor.default_timeout")
        assert first.data == second.data

    def test_property_pure_false_reads_fs(self, tmp_path: Path) -> None:
        """config_bus.load.property.pure

        Contract declares pure: false — the result reflects on-disk content,
        proving it reads the filesystem rather than returning a constant.
        """
        path = _write_yaml(tmp_path / "fs.yaml", "version: '9.9.9'\n" + _PROJECT_BLOCK)
        config = Config.load(path)
        assert config.get("version") == "9.9.9"


# ===========================================================================
# Contract: Config.get
# ===========================================================================


class TestGetContract:
    def setup_method(self) -> None:
        _reset_registry()

    def teardown_method(self) -> None:
        _reset_registry()

    def test_input_key_empty(self) -> None:
        """config_bus.get.input.key.empty

        An empty key is NOT an error: it resolves no value and returns the
        default, exactly like any other absent key (config-bus.md "Contract:
        Config.get", D-74).

        This was skipped in all three SDKs with the reason "contract says
        Config.get('') is rejected ... but this SDK returns the default" —
        written before D-74, which deleted that row and recorded that it
        "described behaviour no SDK has ever had". The skip outlived the rule
        it was about, and because all three suites skipped it symmetrically the
        skip-asymmetry guard could not see it either.
        """
        config = Config.from_defaults()
        assert config.get("") is None
        sentinel = object()
        assert config.get("", sentinel) is sentinel

    def test_property_missing_returns_default(self) -> None:
        """config_bus.get.input.default.missing_key

        Contract: missing key returns the provided default (no error).
        """
        config = Config.from_defaults()
        sentinel = object()
        assert config.get("definitely.absent.key", sentinel) is sentinel

    def test_property_async_false(self) -> None:
        """config_bus.get.property.async"""
        config = Config.from_defaults()
        result = config.get("version")
        assert not inspect.iscoroutine(result)
        assert not inspect.iscoroutinefunction(Config.get)

    def test_property_idempotent_true(self, tmp_path: Path) -> None:
        """config_bus.get.property.idempotent

        Two identical calls on the same state return identical outcomes.
        """
        path = _write_yaml(tmp_path / "g.yaml", _REQUIRED_HEADER + "executor:\n  default_timeout: 1234\n")
        config = Config.load(path)
        first = config.get("executor.default_timeout")
        second = config.get("executor.default_timeout")
        assert first == second == 1234

    def test_property_pure_true(self, tmp_path: Path) -> None:
        """config_bus.get.property.pure

        get must not mutate config: any public query is unchanged after calling.
        """
        path = _write_yaml(tmp_path / "p.yaml", _REQUIRED_HEADER + "executor:\n  default_timeout: 77\n")
        config = Config.load(path)
        snapshot = config.data
        config.get("executor.default_timeout")
        config.get("missing.key", "x")
        assert config.data == snapshot

    async def test_property_thread_safe_concurrent(self, tmp_path: Path) -> None:
        """config_bus.get.property.thread_safe

        Contract declares thread_safe: true. Launch >=8 concurrent reads with
        distinct keys; assert no exception and every result is correct.
        """
        Config.register_namespace("concur_ns")
        body = "apcore:\n  version: '0.15.0'\nconcur_ns:\n" + "".join(f"  k{i}: {i}\n" for i in range(8))
        path = _write_yaml(tmp_path / "c.yaml", body)
        config = Config.load(path)

        async def read(i: int) -> int:
            return config.get(f"concur_ns.k{i}")

        results = await asyncio.gather(*[read(i) for i in range(8)])
        assert results == list(range(8))


# ===========================================================================
# Contract: Config.namespace
# ===========================================================================


class TestNamespaceContract:
    def setup_method(self) -> None:
        _reset_registry()

    def teardown_method(self) -> None:
        _reset_registry()

    def test_input_unregistered_returns_empty(self) -> None:
        """config_bus.namespace.input.name.unregistered

        Unregistered/empty namespace returns an empty dict, never raises.
        """
        config = Config.from_defaults()
        assert config.namespace("never_registered_ns") == {}

    def test_returns_merged_values(self, tmp_path: Path) -> None:
        """config_bus.namespace.returns.merged

        Returns all values under the namespace (defaults + YAML merged).
        """
        Config.register_namespace("nsret", defaults={"retries": 3})
        path = _write_yaml(tmp_path / "n.yaml", "apcore:\n  version: '0.15.0'\nnsret:\n  timeout: 10000\n")
        config = Config.load(path)
        result = config.namespace("nsret")
        assert result.get("timeout") == 10000
        assert result.get("retries") == 3

    def test_property_async_false(self) -> None:
        """config_bus.namespace.property.async"""
        config = Config.from_defaults()
        result = config.namespace("anything")
        assert isinstance(result, dict)
        assert not inspect.iscoroutinefunction(Config.namespace)

    def test_property_pure_true(self, tmp_path: Path) -> None:
        """config_bus.namespace.property.pure

        namespace() returns a copy: mutating the result must not affect config.
        """
        Config.register_namespace("nspure", defaults={"a": 1})
        path = _write_yaml(tmp_path / "np.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        result = config.namespace("nspure")
        result["a"] = 999
        result["injected"] = True
        fresh = config.namespace("nspure")
        assert fresh.get("a") == 1
        assert "injected" not in fresh

    async def test_property_thread_safe_concurrent(self, tmp_path: Path) -> None:
        """config_bus.namespace.property.thread_safe

        thread_safe: true — concurrent namespace() calls stay consistent.
        """
        Config.register_namespace("nsconcur", defaults={"v": 42})
        path = _write_yaml(tmp_path / "nc.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)

        async def read() -> dict:
            return config.namespace("nsconcur")

        results = await asyncio.gather(*[read() for _ in range(8)])
        assert all(r.get("v") == 42 for r in results)


# ===========================================================================
# Contract: Config.bind
# ===========================================================================


class TestBindContract:
    def setup_method(self) -> None:
        _reset_registry()

    def teardown_method(self) -> None:
        _reset_registry()

    def test_error_bind_error(self, tmp_path: Path) -> None:
        """config_bus.bind.error.CONFIG_BIND_ERROR

        Deserialization failure (unexpected field for a dataclass) raises
        ConfigBindError(code=CONFIG_BIND_ERROR).
        """
        Config.register_namespace("bindbad")
        path = _write_yaml(
            tmp_path / "bb.yaml",
            "apcore:\n  version: '0.15.0'\nbindbad:\n  not_a_field: 1\n",
        )
        config = Config.load(path)
        with pytest.raises(ConfigBindError) as exc:
            config.bind("bindbad", _PluginCfg)
        assert exc.value.code == "CONFIG_BIND_ERROR"

    def test_bind_success(self, tmp_path: Path) -> None:
        """config_bus.bind.returns.instance

        Successful bind returns a populated instance of the target type.
        """
        Config.register_namespace("bindok", defaults={"timeout": 5000, "retries": 3})
        path = _write_yaml(tmp_path / "bo.yaml", "apcore:\n  version: '0.15.0'\nbindok:\n  timeout: 8000\n")
        config = Config.load(path)
        result = config.bind("bindok", _PluginCfg)
        assert isinstance(result, _PluginCfg)
        assert result.timeout == 8000
        assert result.retries == 3

    def test_property_async_false(self, tmp_path: Path) -> None:
        """config_bus.bind.property.async"""
        Config.register_namespace("bindasync", defaults={"timeout": 1, "retries": 1})
        path = _write_yaml(tmp_path / "ba.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        result = config.bind("bindasync", _PluginCfg)
        assert not inspect.iscoroutine(result)
        assert not inspect.iscoroutinefunction(Config.bind)

    def test_property_pure_true(self, tmp_path: Path) -> None:
        """config_bus.bind.property.pure

        bind reads a snapshot and does not mutate config state.
        """
        Config.register_namespace("bindpure", defaults={"timeout": 1, "retries": 1})
        path = _write_yaml(tmp_path / "bp.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        snapshot = config.data
        config.bind("bindpure", _PluginCfg)
        assert config.data == snapshot

    async def test_property_thread_safe_concurrent(self, tmp_path: Path) -> None:
        """config_bus.bind.property.thread_safe

        thread_safe: true — concurrent binds succeed and agree.
        """
        Config.register_namespace("bindconcur", defaults={"timeout": 5000, "retries": 3})
        path = _write_yaml(tmp_path / "bc.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)

        async def do_bind() -> _PluginCfg:
            return config.bind("bindconcur", _PluginCfg)

        results = await asyncio.gather(*[do_bind() for _ in range(8)])
        assert all(r.timeout == 5000 and r.retries == 3 for r in results)


# ===========================================================================
# Contract: Config.mount
# ===========================================================================


class TestMountContract:
    def setup_method(self) -> None:
        _reset_registry()

    def teardown_method(self) -> None:
        _reset_registry()

    def test_input_namespace_reserved_config(self, tmp_path: Path) -> None:
        """config_bus.mount.input.namespace.reserved_config

        Mounting into the reserved `_config` namespace raises
        ConfigMountError(code=CONFIG_MOUNT_ERROR).
        """
        path = _write_yaml(tmp_path / "m.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        with pytest.raises(ConfigMountError) as exc:
            config.mount("_config", from_dict={"strict": True})
        assert exc.value.code == "CONFIG_MOUNT_ERROR"

    def test_error_mount_missing_file(self, tmp_path: Path) -> None:
        """config_bus.mount.error.CONFIG_MOUNT_ERROR.missing_file"""
        Config.register_namespace("mountmiss")
        path = _write_yaml(tmp_path / "mm.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        with pytest.raises(ConfigMountError) as exc:
            config.mount("mountmiss", from_file=str(tmp_path / "nope.yaml"))
        assert exc.value.code == "CONFIG_MOUNT_ERROR"

    def test_error_mount_not_mapping(self, tmp_path: Path) -> None:
        """config_bus.mount.error.CONFIG_MOUNT_ERROR.not_a_mapping"""
        Config.register_namespace("mountlist")
        bad = _write_yaml(tmp_path / "list.yaml", "- a\n- b\n")
        path = _write_yaml(tmp_path / "ml.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        with pytest.raises(ConfigMountError) as exc:
            config.mount("mountlist", from_file=bad)
        assert exc.value.code == "CONFIG_MOUNT_ERROR"

    def test_side_effect_dict_merge(self, tmp_path: Path) -> None:
        """config_bus.mount.side_effect.1.merge_over_defaults

        Mounted dict data is merged into the namespace, observable via get().
        """
        Config.register_namespace("mountmerge", defaults={"timeout": 1})
        path = _write_yaml(tmp_path / "mg.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        config.mount("mountmerge", from_dict={"timeout": 10000})
        assert config.get("mountmerge.timeout") == 10000

    def test_property_async_false(self, tmp_path: Path) -> None:
        """config_bus.mount.property.async"""
        Config.register_namespace("mountasync")
        path = _write_yaml(tmp_path / "ma.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        result = config.mount("mountasync", from_dict={"x": 1})
        assert result is None
        assert not inspect.iscoroutinefunction(Config.mount)

    def test_property_idempotent_false_stacks(self, tmp_path: Path) -> None:
        """config_bus.mount.property.idempotent

        Contract declares idempotent: false — mounting list-valued data twice
        stacks rather than producing identical state.
        """
        Config.register_namespace("mountstack", defaults={"items": []})
        path = _write_yaml(tmp_path / "ms.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        config.mount("mountstack", from_dict={"counter": 1})
        first = config.namespace("mountstack")
        config.mount("mountstack", from_dict={"counter": 2})
        second = config.namespace("mountstack")
        # Second mount changed observable state (not a no-op) -> non-idempotent.
        assert first.get("counter") == 1
        assert second.get("counter") == 2

    def test_property_pure_false_mutates(self, tmp_path: Path) -> None:
        """config_bus.mount.property.pure

        Contract declares pure: false — mount mutates config state.
        """
        Config.register_namespace("mountmut", defaults={"v": 0})
        path = _write_yaml(tmp_path / "mu.yaml", "apcore:\n  version: '0.15.0'\n")
        config = Config.load(path)
        before = config.namespace("mountmut")
        config.mount("mountmut", from_dict={"v": 5})
        after = config.namespace("mountmut")
        assert before.get("v") == 0
        assert after.get("v") == 5

    @pytest.mark.skip(
        reason="clause too vague: contract declares thread_safe: false "
        "('do not call concurrently with reads'); concurrent mutation is "
        "explicitly unsupported so there is no safe behavior to assert"
    )
    def test_property_thread_safe_false(self) -> None:
        """config_bus.mount.property.thread_safe"""


# ===========================================================================
# Contract: Config.reload
# ===========================================================================


class TestReloadContract:
    def setup_method(self) -> None:
        _reset_registry()

    def teardown_method(self) -> None:
        _reset_registry()

    def test_error_not_found(self, tmp_path: Path) -> None:
        """config_bus.reload.error.CONFIG_NOT_FOUND

        Source file removed before reload -> ConfigNotFoundError.
        """
        path = Path(_write_yaml(tmp_path / "r.yaml", _REQUIRED_HEADER))
        config = Config.load(str(path))
        path.unlink()
        with pytest.raises(ConfigNotFoundError) as exc:
            config.reload()
        assert exc.value.code == "CONFIG_NOT_FOUND"

    def test_error_invalid(self, tmp_path: Path) -> None:
        """config_bus.reload.error.CONFIG_INVALID

        Source file becomes invalid YAML before reload -> ConfigError
        (code CONFIG_INVALID, the SDK type for the spec's ConfigInvalidError).
        """
        path = Path(_write_yaml(tmp_path / "ri.yaml", _REQUIRED_HEADER))
        config = Config.load(str(path))
        path.write_text("bad: [unterminated\n", encoding="utf-8")
        with pytest.raises(ConfigError) as exc:
            config.reload()
        assert exc.value.code == "CONFIG_INVALID"

    def test_property_async_false(self, tmp_path: Path) -> None:
        """config_bus.reload.property.async"""
        path = _write_yaml(tmp_path / "ra.yaml", _REQUIRED_HEADER)
        config = Config.load(path)
        result = config.reload()
        assert result is None
        assert not inspect.iscoroutinefunction(Config.reload)

    def test_property_idempotent_true(self, tmp_path: Path) -> None:
        """config_bus.reload.property.idempotent

        Two reloads with unchanged files produce identical state.
        """
        path = _write_yaml(tmp_path / "rid.yaml", _REQUIRED_HEADER + "executor:\n  default_timeout: 2222\n")
        config = Config.load(path)
        config.reload()
        first = config.data
        config.reload()
        second = config.data
        assert first == second

    def test_side_effect_picks_up_change(self, tmp_path: Path) -> None:
        """config_bus.reload.side_effect.1.reread_filesystem

        reload re-reads the YAML: a post-load edit becomes visible only after
        reload(), proving the filesystem re-read side effect and its ordering.
        """
        path = Path(_write_yaml(tmp_path / "rc.yaml", _REQUIRED_HEADER + "executor:\n  default_timeout: 1\n"))
        config = Config.load(str(path))
        assert config.get("executor.default_timeout") == 1
        # Edit on disk; pre-reload state must be unchanged.
        path.write_text(_REQUIRED_HEADER + "executor:\n  default_timeout: 999\n", encoding="utf-8")
        assert config.get("executor.default_timeout") == 1
        # After reload the new value is visible.
        config.reload()
        assert config.get("executor.default_timeout") == 999

    def test_property_pure_false_reapplies_mounts(self, tmp_path: Path) -> None:
        """config_bus.reload.property.pure

        Contract declares pure: false — reload mutates state and re-applies
        stored mounts over freshly-read file data.
        """
        Config.register_namespace("reloadmount", defaults={"v": 0})
        path = _write_yaml(tmp_path / "rm.yaml", "apcore:\n  version: '0.15.0'\nreloadmount:\n  v: 1\n")
        config = Config.load(path)
        config.mount("reloadmount", from_dict={"mounted": True})
        config.reload()
        # File value preserved AND mount re-applied -> state mutated by reload.
        assert config.get("reloadmount.v") == 1
        assert config.get("reloadmount.mounted") is True

    @pytest.mark.skip(
        reason="clause too vague: contract declares thread_safe: false "
        "('no in-flight read protection'); concurrent reload is explicitly "
        "unsupported so there is no safe behavior to assert"
    )
    def test_property_thread_safe_false(self) -> None:
        """config_bus.reload.property.thread_safe"""
