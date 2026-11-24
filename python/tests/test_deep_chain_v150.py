"""Regression tests for the spec v1.50.0 deep-chain decisions (D-92 – D-107).

Each test here pins a behaviour that diverged between apcore-python,
apcore-typescript and apcore-rust and was settled by a maintainer decision in
``docs/spec/2026-09-deep-chain-decisions.md`` or the v1.50.0 feature-doc notes.
"""

from __future__ import annotations

import asyncio
import time

import pytest

from apcore import APCore, Config
from apcore.executor import Executor
from apcore.registry import Registry


class _OkModule:
    """Minimal conforming module used by the client tests below."""

    input_schema = {"type": "object"}
    output_schema = {"type": "object"}
    description = "Test module"

    def execute(self, inputs, context):
        return {"ok": True}


def _make_module(module_id: str = "a.b"):
    return _OkModule()


# ---------------------------------------------------------------------------
# CLI-1 — a supplied Executor's registry is the client's registry
# ---------------------------------------------------------------------------


class TestClientAdoptsExecutorRegistry:
    """The client MUST NOT build a second Registry beside a supplied Executor."""

    def test_register_then_call_uses_the_executor_registry(self):
        my_registry = Registry()
        client = APCore(executor=Executor(registry=my_registry))

        client.register("a.b", _make_module())

        # The module must land where the executor will look for it.
        assert my_registry.get("a.b") is not None
        assert client.call("a.b", {}) == {"ok": True}

    def test_list_modules_reads_the_executor_registry(self):
        my_registry = Registry()
        my_registry.register("a.b", _make_module())
        client = APCore(executor=Executor(registry=my_registry))

        # The client's read path must see what the executor can call.
        assert client.list_modules() == ["a.b"]

    def test_registry_attribute_is_the_executor_registry(self):
        my_registry = Registry()
        client = APCore(executor=Executor(registry=my_registry))

        assert client.registry is my_registry

    def test_supplied_registry_is_ignored_when_executor_is_given(self):
        # Contract: APCore.with_options — `registry` is "Ignored when executor
        # is also provided (the executor's own registry is used instead)".
        executor_registry = Registry()
        ignored_registry = Registry()
        client = APCore(
            registry=ignored_registry,
            executor=Executor(registry=executor_registry),
        )

        assert client.registry is executor_registry

    def test_sys_modules_are_registered_where_the_executor_looks(self):
        config = Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}})
        my_registry = Registry()
        client = APCore(executor=Executor(registry=my_registry), config=config)

        client.register("x.y", _make_module("x.y"))
        result = client.disable("x.y")

        assert result["success"] is True


# ---------------------------------------------------------------------------
# CLI-3 — the auto-created Registry receives the client's Config
# ---------------------------------------------------------------------------


class TestClientPassesConfigToRegistry:
    """`extensions.*` and `id_map.overrides` must not be inert via the client."""

    def test_discover_scans_the_configured_root(self, tmp_path):
        ext_root = tmp_path / "my_extensions"
        pkg = ext_root / "executor" / "demo"
        pkg.mkdir(parents=True)
        (pkg / "run.py").write_text(
            "from pydantic import BaseModel\n"
            "\n"
            "\n"
            "class RunInput(BaseModel):\n"
            "    pass\n"
            "\n"
            "\n"
            "class RunOutput(BaseModel):\n"
            "    pass\n"
            "\n"
            "\n"
            "class RunModule:\n"
            "    input_schema = RunInput\n"
            "    output_schema = RunOutput\n"
            "    description = 'Demo module'\n"
            "\n"
            "    def execute(self, inputs, context):\n"
            "        return {}\n"
        )

        config = Config({"extensions": {"root": str(ext_root)}})
        client = APCore(config=config)

        assert client.discover() == 1
        assert "executor.demo.run" in client.list_modules()

    def test_registry_receives_the_config_object(self):
        config = Config({"extensions": {"max_depth": 3, "follow_symlinks": True}})
        client = APCore(config=config)

        max_depth, follow_symlinks, _ = client.registry._scan_params()
        assert max_depth == 3
        assert follow_symlinks is True


# ---------------------------------------------------------------------------
# D-99 / D-100 / D-101 / D-102 — `global_deadline` clock, storage, lifetime
# and recomputation
# ---------------------------------------------------------------------------


class _SlowModule:
    input_schema = {"type": "object"}
    output_schema = {"type": "object"}
    description = "Sleeps briefly"

    async def execute(self, inputs, context):
        await asyncio.sleep(0.5)
        return {"ok": True}


class TestGlobalDeadline:
    def test_caller_supplied_epoch_deadline_fires(self):
        """D-99 — the clock is epoch seconds, so a `time.time()`-based value works."""
        from apcore.context import Context
        from apcore.errors import ModuleTimeoutError

        registry = Registry()
        registry.register("slow.op", _SlowModule())
        executor = Executor(registry=registry)

        # A caller following the spec writes `time.time() + budget`.
        ctx = Context.create(global_deadline=time.time() + 0.05)

        with pytest.raises(ModuleTimeoutError):
            asyncio.run(executor.call_async("slow.op", {}, ctx))

    def test_deadline_is_not_written_onto_the_callers_context(self):
        """D-101 — the deadline belongs to the call tree, not to the Context."""
        from apcore.context import Context

        registry = Registry()
        registry.register("a.b", _make_module())
        executor = Executor(registry=registry)

        ctx = Context.create()
        asyncio.run(executor.call_async("a.b", {}, ctx))

        assert ctx.global_deadline is None

    def test_reused_context_does_not_inherit_the_first_calls_deadline(self):
        """D-101 — a Context reused across top-level calls gets a fresh budget."""
        from apcore.context import Context

        registry = Registry()
        registry.register("a.b", _make_module())
        registry.register("slow.op", _SlowModule())
        # 300 ms global budget.
        executor = Executor(registry=registry, config=Config({"executor": {"global_timeout": 300}}))

        ctx = Context.create()
        asyncio.run(executor.call_async("a.b", {}, ctx))
        time.sleep(0.35)
        # The second call must get its own 300 ms, not the expired remainder.
        assert asyncio.run(executor.call_async("a.b", {}, ctx)) == {"ok": True}

    def test_deserialized_context_recomputes_the_deadline(self):
        """D-102 — recomputation is unconditional; a non-empty call_chain does not gate it."""
        from apcore.context import Context
        from apcore.errors import ModuleTimeoutError

        registry = Registry()
        registry.register("slow.op", _SlowModule())
        executor = Executor(registry=registry, config=Config({"executor": {"global_timeout": 50}}))

        # A Context arriving from another process carries a non-empty
        # call_chain by definition and no global_deadline (it does not
        # serialize).
        wire = Context.create().child("upstream.caller").serialize()
        ctx = Context.deserialize(wire)
        assert ctx.call_chain  # precondition: not a root call
        assert ctx.global_deadline is None

        with pytest.raises(ModuleTimeoutError):
            asyncio.run(executor.call_async("slow.op", {}, ctx))

    def test_caller_supplied_deadline_wins_over_the_config_default(self):
        """D-100 — the first-class ``global_deadline`` field IS the storage.

        Goes RED if the set site stops guarding on ``global_deadline is None``
        (so the config default overwrites the caller's value), or if the
        budget is read from anywhere other than the field the caller wrote.
        """
        from apcore.context import Context
        from apcore.errors import ModuleTimeoutError

        registry = Registry()
        registry.register("slow.op", _SlowModule())
        # A 10 s config budget would let the 0.5 s module finish easily.
        executor = Executor(registry=registry, config=Config({"executor": {"global_timeout": 10000}}))

        ctx = Context.create(global_deadline=time.time() + 0.05)

        with pytest.raises(ModuleTimeoutError):
            asyncio.run(executor.call_async("slow.op", {}, ctx))

    def test_control_the_same_module_completes_under_the_config_default(self):
        """Control for D-100 — without a caller deadline the 10 s budget applies.

        Without this, a blanket "everything times out" regression would pass
        the assertion above for the wrong reason.
        """
        registry = Registry()
        registry.register("slow.op", _SlowModule())
        executor = Executor(registry=registry, config=Config({"executor": {"global_timeout": 10000}}))

        assert asyncio.run(executor.call_async("slow.op", {})) == {"ok": True}


# ---------------------------------------------------------------------------
# APR-2 — `_approval_token` is stripped before policy resolution (§7.9.6 rule 5)
# ---------------------------------------------------------------------------


def _recording_policy():
    """An ExecutionPolicy subclass that records the `arguments` it receives.

    `ExecutionPolicy.resolve`'s own docstring names overriding `resolve` as the
    way "a host-supplied policy" consults the call site, so this is the surface
    §7.9.6 rule 5 protects.
    """
    from apcore.policy import ExecutionPolicy

    class Recorder(ExecutionPolicy):
        def __init__(self):
            super().__init__()
            self.seen: list[dict] = []

        def resolve(self, module_id, annotations, *, arguments=None, context=None):
            self.seen.append(dict(arguments or {}))
            return super().resolve(module_id, annotations, arguments=arguments, context=context)

    return Recorder()


class TestApprovalTokenStrippedBeforePolicy:
    def test_validate_does_not_leak_the_token_to_policy_resolution(self):
        registry = Registry()
        registry.register("a.b", _make_module())
        policy = _recording_policy()
        executor = Executor(registry=registry, policy=policy)

        executor.validate("a.b", {"_approval_token": "tok-123", "amount": 5})

        assert policy.seen, "policy resolution was not reached"
        for seen in policy.seen:
            assert "_approval_token" not in seen
            assert seen == {"amount": 5}

    def test_the_callers_input_mapping_is_not_mutated(self):
        registry = Registry()
        registry.register("a.b", _make_module())
        executor = Executor(registry=registry, policy=_recording_policy())

        inputs = {"_approval_token": "tok-123", "amount": 5}
        executor.validate("a.b", inputs)

        assert inputs == {"_approval_token": "tok-123", "amount": 5}

    def test_execution_policy_resolve_strips_at_the_policy_door(self):
        """The guard also sits inside `ExecutionPolicy.resolve`, covering every door."""
        from apcore.policy import ExecutionPolicy, strip_approval_token

        args = {"_approval_token": "tok-123", "amount": 5}
        assert strip_approval_token(args) == {"amount": 5}
        assert args == {"_approval_token": "tok-123", "amount": 5}
        assert strip_approval_token(None) is None

        # Rule 6: adding/removing the call site MUST NOT change the verdict.
        policy = ExecutionPolicy()
        with_token = policy.resolve("a.b", None, arguments=args)
        without = policy.resolve("a.b", None, arguments={"amount": 5})
        assert with_token.needs_approval == without.needs_approval


# ---------------------------------------------------------------------------
# D-104 — a local `#/…` ref resolves against the file root, then the schema node
# ---------------------------------------------------------------------------


_LAYOUT_A = """\
module_id: demo.a
description: Layout A — definitions at the top level of the FILE
input_schema:
  type: object
  properties:
    user:
      $ref: "#/definitions/User"
output_schema:
  type: object
definitions:
  User:
    type: object
    properties:
      name:
        type: string
"""

_LAYOUT_B = """\
module_id: demo.b
description: Layout B — $defs nested inside input_schema
input_schema:
  type: object
  properties:
    user:
      $ref: "#/$defs/User"
  $defs:
    User:
      type: object
      properties:
        name:
          type: string
output_schema:
  type: object
"""


def _loader(tmp_path):
    from apcore.schema.loader import SchemaLoader

    return SchemaLoader(Config({}), schemas_dir=tmp_path)


class TestLocalRefResolvesAgainstFileRootThenSchemaNode:
    def test_layout_a_file_root_definitions(self, tmp_path):
        """The spec's own §4.11 example: `definitions:` beside `input_schema`."""
        (tmp_path / "demo").mkdir()
        (tmp_path / "demo" / "a.schema.yaml").write_text(_LAYOUT_A)

        loader = _loader(tmp_path)
        resolved_input, _ = loader.resolve(loader.load("demo.a"))

        assert resolved_input.json_schema["properties"]["user"]["properties"]["name"] == {"type": "string"}

    def test_layout_b_schema_node_defs_still_resolves(self, tmp_path):
        """The fallback keeps the layout two of three SDKs already accept."""
        (tmp_path / "demo").mkdir()
        (tmp_path / "demo" / "b.schema.yaml").write_text(_LAYOUT_B)

        loader = _loader(tmp_path)
        resolved_input, _ = loader.resolve(loader.load("demo.b"))

        assert resolved_input.json_schema["properties"]["user"]["properties"]["name"] == {"type": "string"}

    def test_schema_definition_carries_its_source_file(self, tmp_path):
        (tmp_path / "demo").mkdir()
        (tmp_path / "demo" / "a.schema.yaml").write_text(_LAYOUT_A)

        sd = _loader(tmp_path).load("demo.a")

        assert sd.source_path == (tmp_path / "demo" / "a.schema.yaml")

        # `definitions` is collected from the file's top level and is now live.
        assert "User" in sd.definitions

    def test_an_unresolvable_local_ref_still_raises(self, tmp_path):
        from apcore.errors import SchemaNotFoundError

        (tmp_path / "demo").mkdir()
        (tmp_path / "demo" / "c.schema.yaml").write_text(
            _LAYOUT_A.replace("#/definitions/User", "#/definitions/Missing")
        )

        loader = _loader(tmp_path)
        with pytest.raises(SchemaNotFoundError):
            loader.resolve(loader.load("demo.c"))


_CALLER_REACHING_OUT = """\
module_id: demo.caller
description: Caller with a local $defs the external document must not reach
input_schema:
  type: object
  $defs:
    Shared:
      type: object
      properties:
        local_marker:
          type: string
  properties:
    thing:
      $ref: "./ext.schema.yaml#/$defs/Thing"
output_schema:
  type: object
"""

# The external document's `#/$defs/Shared` does NOT exist here. This document
# has never heard of the caller's `$defs`.
_EXT_DANGLING = """\
$defs:
  Thing:
    type: object
    properties:
      inner:
        $ref: "#/$defs/Shared"
"""

# The control: `Shared` defined locally, with a marker that tells the two
# documents apart in the resolved output.
_EXT_RESOLVABLE = """\
$defs:
  Shared:
    type: object
    properties:
      external_marker:
        type: string
  Thing:
    type: object
    properties:
      inner:
        $ref: "#/$defs/Shared"
"""


class TestNodeFallbackIsScopedToItsOwnDocument:
    """D-104 settled WHICH two bases a local pointer tries, not how far the
    second one travels — and the answer was "everywhere".

    The node fallback lived on the resolver and was consulted for every local
    pointer, including ones resolved after following a reference into another
    file. So an external schema's `#/$defs/X`, for a definition that document
    does not have, fell back to the CALLING module's schema node and bound to
    whatever happened to share the name.

    Three things go wrong, in increasing order of cost: an invalid reference
    reports success where it owes ``SCHEMA_NOT_FOUND``; the resolved schema
    then validates against a contract the external author never wrote; and
    §10.6 reads ``x-sensitive`` off the RESOLVED schema, so a field the
    external document marks sensitive can be replaced by a local definition
    that does not and be logged in plaintext — the same class of leak as
    dropping ``$ref`` sibling keys (SCH-001).

    All three SDKs had it. apcore-rust was found first, in review; the peers
    were checked by direct reproduction rather than by reading the code.
    """

    def _write(self, tmp_path, ext_source):
        (tmp_path / "demo").mkdir()
        (tmp_path / "demo" / "caller.schema.yaml").write_text(_CALLER_REACHING_OUT)
        (tmp_path / "demo" / "ext.schema.yaml").write_text(ext_source)

    def test_external_dangling_local_ref_does_not_fall_back_to_the_caller(self, tmp_path):
        from apcore.errors import SchemaNotFoundError

        self._write(tmp_path, _EXT_DANGLING)
        loader = _loader(tmp_path)

        with pytest.raises(SchemaNotFoundError):
            loader.resolve(loader.load("demo.caller"))

    def test_external_documents_own_local_ref_still_resolves_in_that_document(self, tmp_path):
        """The control — the fix must scope the fallback, not disable it."""
        self._write(tmp_path, _EXT_RESOLVABLE)
        loader = _loader(tmp_path)

        resolved_input, _ = loader.resolve(loader.load("demo.caller"))
        inner = resolved_input.json_schema["properties"]["thing"]["properties"]["inner"]

        assert inner["properties"] == {"external_marker": {"type": "string"}}, (
            "the external document's own $defs/Shared must be the one inlined, "
            "and the caller's same-named definition must not be"
        )

    def test_the_callers_own_nested_defs_still_resolve(self, tmp_path):
        """Layout B is unaffected: a document always falls back to its own node."""
        self._write(tmp_path, _EXT_RESOLVABLE)
        loader = _loader(tmp_path)

        resolved_input, _ = loader.resolve(loader.load("demo.caller"))

        assert "Shared" in resolved_input.json_schema["$defs"]


# ---------------------------------------------------------------------------
# EXE-006 — a per-step timeout takes the ordinary step-error path
# ---------------------------------------------------------------------------


class _HangingStep:
    """A step that always outruns its own `timeout_ms`."""

    def __init__(self, *, ignore_errors: bool = False):
        self.name = "hanging_probe"
        self.description = "Sleeps past its per-step timeout"
        self.removable = True
        self.replaceable = True
        self.match_modules = None
        self.ignore_errors = ignore_errors
        self.pure = False
        self.timeout_ms = 20
        self.requires = ()
        self.provides = ()

    async def execute(self, ctx):
        await asyncio.sleep(1.0)
        from apcore.pipeline import StepResult

        return StepResult(action="continue")


class _ErrorRecordingStepMiddleware:
    def __init__(self, recovery=None):
        self.errors: list = []
        self._recovery = recovery

    async def on_step_error(self, step_name, state, error):
        self.errors.append((step_name, error))
        return self._recovery


def _executor_with_hanging_step(*, ignore_errors=False, step_mw=None):
    registry = Registry()
    registry.register("a.b", _make_module())
    executor = Executor(registry=registry)
    strategy = executor._strategy
    strategy.insert_after("module_lookup", _HangingStep(ignore_errors=ignore_errors))
    if step_mw is not None:
        strategy.add_step_middleware(step_mw)
    return executor


class TestStepTimeoutTakesTheStepErrorPath:
    def test_on_step_error_is_invoked(self):
        mw = _ErrorRecordingStepMiddleware()
        executor = _executor_with_hanging_step(step_mw=mw)

        with pytest.raises(Exception):
            asyncio.run(executor.call_async("a.b", {}))

        assert [name for name, _ in mw.errors] == ["hanging_probe"]
        assert isinstance(mw.errors[0][1], asyncio.TimeoutError)

    def test_ignore_errors_applies_to_a_timeout(self):
        executor = _executor_with_hanging_step(ignore_errors=True)

        assert asyncio.run(executor.call_async("a.b", {})) == {"ok": True}

    def test_a_recovery_value_resumes_the_pipeline(self):
        from apcore.pipeline import StepResult

        mw = _ErrorRecordingStepMiddleware(recovery=StepResult(action="continue"))
        executor = _executor_with_hanging_step(step_mw=mw)

        assert asyncio.run(executor.call_async("a.b", {})) == {"ok": True}

    def test_middleware_on_error_sees_the_timeout(self):
        """The Executor must reach `_recover_from_call_error`, not re-raise an abort."""
        from apcore.errors import ModuleTimeoutError
        from apcore.middleware import Middleware

        seen: list = []

        class _RecoveringMiddleware(Middleware):
            def on_error(self, module_id, inputs, error, context):
                seen.append(error)
                return {"recovered": True}

        registry = Registry()
        registry.register("a.b", _make_module())
        executor = Executor(registry=registry)
        executor.use(_RecoveringMiddleware())
        # After `middleware_before`, so a middleware has entered the call.
        executor._strategy.insert_after("middleware_before", _HangingStep())

        assert asyncio.run(executor.call_async("a.b", {})) == {"recovered": True}
        assert len(seen) == 1
        assert isinstance(seen[0], ModuleTimeoutError)
        assert seen[0].code == "MODULE_TIMEOUT"

    def test_the_public_error_is_still_module_timeout(self):
        from apcore.errors import ModuleTimeoutError

        executor = _executor_with_hanging_step()

        with pytest.raises(ModuleTimeoutError) as excinfo:
            asyncio.run(executor.call_async("a.b", {}))
        assert excinfo.value.code == "MODULE_TIMEOUT"


# ---------------------------------------------------------------------------
# EXE-007 — `replace()` / `configure_step()` reject a duplicate step name
# ---------------------------------------------------------------------------


def _named_step(name: str):
    class _Step:
        def __init__(self):
            self.name = name
            self.description = ""
            self.removable = True
            self.replaceable = True
            self.match_modules = None
            self.ignore_errors = False
            self.pure = True
            self.timeout_ms = 0
            self.requires = ()
            self.provides = ()

        async def execute(self, ctx):
            from apcore.pipeline import StepResult

            return StepResult(action="continue")

    return _Step()


def _strategy():
    from apcore.pipeline import ExecutionStrategy

    return ExecutionStrategy("test", [_named_step("first"), _named_step("second"), _named_step("third")])


class TestStepNameUniquenessOnReplacement:
    """docs/features/execution-pipeline.md: step names are unique within a strategy."""

    def test_replace_rejects_an_existing_name(self):
        from apcore.pipeline import StepNameDuplicateError

        strategy = _strategy()
        with pytest.raises(StepNameDuplicateError):
            strategy.replace("first", _named_step("third"))
        assert strategy.step_names() == ["first", "second", "third"]

    def test_configure_step_rejects_an_existing_name(self):
        from apcore.pipeline import StepNameDuplicateError

        strategy = _strategy()
        with pytest.raises(StepNameDuplicateError):
            strategy.configure_step("first", _named_step("second"))
        assert strategy.step_names() == ["first", "second", "third"]

    def test_the_error_carries_the_registry_code(self):
        from apcore.pipeline import StepNameDuplicateError

        strategy = _strategy()
        with pytest.raises(StepNameDuplicateError) as excinfo:
            strategy.replace("first", _named_step("third"))
        assert excinfo.value.code == "STEP_NAME_DUPLICATE"

    def test_replacing_a_step_with_its_own_name_is_allowed(self):
        strategy = _strategy()
        strategy.replace("second", _named_step("second"))
        assert strategy.step_names() == ["first", "second", "third"]

    def test_renaming_to_a_free_name_is_allowed(self):
        strategy = _strategy()
        strategy.configure_step("second", _named_step("renamed"))
        assert strategy.step_names() == ["first", "renamed", "third"]

    def test_the_name_index_never_shadows_a_real_step(self):
        """The defect's consequence: a stale index anchors later edits on an impostor."""
        from apcore.pipeline import StepNameDuplicateError

        strategy = _strategy()
        with pytest.raises(StepNameDuplicateError):
            strategy.replace("third", _named_step("first"))
        # "first" still resolves to index 0, so `insert_after` anchors correctly.
        strategy.insert_after("first", _named_step("inserted"))
        assert strategy.step_names() == ["first", "inserted", "second", "third"]


# ---------------------------------------------------------------------------
# D-92 — `TaskStoreError` must exist before it can be raised
# ---------------------------------------------------------------------------


class TestTaskStoreErrorIsDefinedAndExported:
    """async-tasks.md declares it on eight surfaces; no SDK defined it."""

    def test_it_is_exported_from_the_package_root(self):
        import apcore

        assert hasattr(apcore, "TaskStoreError")
        assert "TaskStoreError" in apcore.__all__

    def test_it_carries_the_registry_code(self):
        from apcore.errors import ErrorCodes, TaskStoreError

        assert ErrorCodes.TASK_STORE_UNAVAILABLE == "TASK_STORE_UNAVAILABLE"
        err = TaskStoreError(operation="save")
        assert err.code == "TASK_STORE_UNAVAILABLE"
        assert err.details["operation"] == "save"

    def test_it_is_a_module_error_and_catchable_as_one(self):
        from apcore.errors import ModuleError, TaskStoreError

        assert issubclass(TaskStoreError, ModuleError)
        with pytest.raises(ModuleError):
            raise TaskStoreError(operation="get")

    def test_the_code_is_framework_reserved(self):
        """A module MUST NOT be able to claim the code as its own (A17)."""
        from apcore.errors import ErrorCodeCollisionError, ErrorCodeRegistry

        registry = ErrorCodeRegistry()
        with pytest.raises(ErrorCodeCollisionError):
            registry.register("my.module", {"TASK_STORE_UNAVAILABLE"})

    def test_a_host_store_can_raise_it_through_the_manager(self):
        """The reason it must be exported: a network-backed store is host code."""
        from apcore.async_task import AsyncTaskManager
        from apcore.errors import TaskStoreError

        class _UnreachableStore:
            async def save(self, info):
                raise TaskStoreError(operation="save")

            async def get(self, task_id):
                raise TaskStoreError(operation="get")

            async def list(self, **kwargs):
                raise TaskStoreError(operation="list")

            async def delete(self, task_id):
                raise TaskStoreError(operation="delete")

        manager = AsyncTaskManager(executor=None, store=_UnreachableStore())
        with pytest.raises(TaskStoreError):
            asyncio.run(manager.get_status("nope"))

    def test_the_bundled_in_memory_store_never_raises_it(self):
        from apcore.async_task import InMemoryTaskStore, TaskInfo, TaskStatus

        store = InMemoryTaskStore()
        info = TaskInfo("t1", "test.echo", TaskStatus.PENDING, submitted_at=1.0)
        asyncio.run(store.save(info))
        assert asyncio.run(store.get("t1")) is not None
        assert asyncio.run(store.get("missing")) is None


# ---------------------------------------------------------------------------
# D-81 — store errors reach the caller (ALL manager methods)
# ---------------------------------------------------------------------------


class _UnavailableStore:
    """A network-backed store in an outage: every method raises the canonical type.

    Each flag is independent so a test can fail exactly the store call the
    method under test makes, and no other.
    """

    def __init__(self):
        from apcore.async_task import InMemoryTaskStore

        self._inner = InMemoryTaskStore()
        self.fail_save = False
        self.fail_get = False
        self.fail_list = False
        self.fail_delete = False
        self.fail_list_expired = False

    @staticmethod
    def _down(operation):
        from apcore.errors import TaskStoreError

        raise TaskStoreError(operation=operation, reason="backing store is unreachable")

    async def save(self, info):
        if self.fail_save:
            self._down("save")
        return await self._inner.save(info)

    async def get(self, task_id):
        if self.fail_get:
            self._down("get")
        return await self._inner.get(task_id)

    async def list(self, status=None):
        if self.fail_list:
            self._down("list")
        return await self._inner.list(status)

    async def delete(self, task_id):
        if self.fail_delete:
            self._down("delete")
        return await self._inner.delete(task_id)

    async def list_expired(self, before_timestamp):
        if self.fail_list_expired:
            self._down("list_expired")
        return await self._inner.list_expired(before_timestamp)


class _EchoExecutor:
    async def call_async(self, module_id, inputs=None, context=None, **kwargs):
        return {"ok": True}

    def call(self, module_id, inputs=None, context=None, **kwargs):
        return {"ok": True}


class TestStoreErrorsReachTheCaller:
    """D-81 — a manager that absorbs a store outage reports "no such task".

    apcore-python had one assertion for this, on `get_status`, filed under
    D-92; six of the seven manager methods the decision names were unpinned.
    The store raises the CANONICAL ``TaskStoreError`` D-92 defines rather than
    a stand-in, which is the part that matters: against a plain ``Exception``
    every assertion here stays green for a manager that swallows
    ``TaskStoreError`` specifically and re-raises everything else — the exact
    failure the decision forbids.
    """

    @staticmethod
    def _manager(store):
        from apcore.async_task import AsyncTaskManager

        return AsyncTaskManager(executor=_EchoExecutor(), store=store)

    @staticmethod
    def _assert_unavailable(what, call):
        from apcore.errors import TaskStoreError

        try:
            result = call()
        except TaskStoreError as err:
            assert err.code == "TASK_STORE_UNAVAILABLE", f"{what}: {err.code}"
            return
        raise AssertionError(f"{what} absorbed a store outage into {result!r}")

    def test_submit_does_not_report_a_task_id_that_never_persisted(self):
        store = _UnavailableStore()
        store.fail_save = True
        manager = self._manager(store)
        self._assert_unavailable("submit", lambda: asyncio.run(manager.submit("test.echo", {})))

    def test_get_status_does_not_report_not_found(self):
        store = _UnavailableStore()
        store.fail_get = True
        manager = self._manager(store)
        self._assert_unavailable("get_status", lambda: manager.get_status("any"))
        self._assert_unavailable("get_status_async", lambda: asyncio.run(manager.get_status_async("any")))

    def test_get_result_does_not_report_not_found(self):
        store = _UnavailableStore()
        store.fail_get = True
        manager = self._manager(store)
        self._assert_unavailable("get_result", lambda: manager.get_result("any"))

    def test_cancel_does_not_report_success_for_a_save_that_never_landed(self):
        store = _UnavailableStore()
        store.fail_get = True
        manager = self._manager(store)
        self._assert_unavailable("cancel", lambda: asyncio.run(manager.cancel("any")))

    def test_list_tasks_does_not_report_no_tasks(self):
        store = _UnavailableStore()
        store.fail_list = True
        manager = self._manager(store)
        self._assert_unavailable("list_tasks", lambda: manager.list_tasks())
        self._assert_unavailable("list_tasks_async", lambda: asyncio.run(manager.list_tasks_async()))

    def test_cleanup_does_not_report_zero_removals(self):
        store = _UnavailableStore()
        store.fail_list = True
        store.fail_list_expired = True
        manager = self._manager(store)
        self._assert_unavailable("cleanup", lambda: asyncio.run(manager.cleanup(0.0)))

    def test_shutdown_does_not_resolve_on_a_cancellation_write_that_failed(self):
        """The worst of the set: shutdown() asserts its own postcondition.

        Every PENDING/RUNNING task is now CANCELLED — for a record that never
        landed.
        """
        from apcore.async_task import TaskInfo, TaskStatus

        store = _UnavailableStore()
        manager = self._manager(store)
        asyncio.run(store.save(TaskInfo("t1", "test.echo", TaskStatus.PENDING, submitted_at=1.0)))
        # `list` still answers, so shutdown() gets past its own store read and
        # fails on the CANCELLED write.
        store.fail_save = True
        self._assert_unavailable("shutdown", lambda: asyncio.run(manager.shutdown()))

    def test_control_every_method_succeeds_against_a_healthy_store(self):
        """Without this, "they all raised" is also satisfied by a broken manager."""
        store = _UnavailableStore()
        manager = self._manager(store)
        task_id = asyncio.run(manager.submit("test.echo", {}))

        assert manager.get_status(task_id) is not None
        assert asyncio.run(manager.get_status_async(task_id)) is not None
        assert isinstance(manager.list_tasks(), list)
        assert isinstance(asyncio.run(manager.list_tasks_async()), list)
        assert isinstance(asyncio.run(manager.cleanup(0.0)), int)
        assert isinstance(asyncio.run(manager.cancel(task_id)), bool)
        asyncio.run(manager.shutdown())


# ---------------------------------------------------------------------------
# IDN-2 — `Identity.roles` is an immutable sequence
# ---------------------------------------------------------------------------


class TestIdentityRolesAreImmutable:
    def test_roles_are_copied_and_frozen(self):
        from apcore.context import Identity

        roles = ["admin"]
        ident = Identity(id="u", roles=roles)  # type: ignore[arg-type]
        roles.append("root")

        assert ident.roles == ("admin",)
        assert isinstance(ident.roles, tuple)

    def test_an_acl_role_check_cannot_be_widened_after_construction(self):
        from apcore.context import Identity

        roles = ["viewer"]
        ident = Identity(id="u", roles=roles)  # type: ignore[arg-type]
        roles.append("root")

        assert "root" not in ident.roles

    def test_a_tuple_stays_a_tuple(self):
        from apcore.context import Identity

        ident = Identity(id="u", roles=("a", "b"))
        assert ident.roles == ("a", "b")

    def test_none_roles_become_an_empty_tuple(self):
        from apcore.context import Identity

        ident = Identity(id="u", roles=None)  # type: ignore[arg-type]
        assert ident.roles == ()


# ---------------------------------------------------------------------------
# CTX-1 — `Context.deserialize` raises a typed error on a malformed identity
# ---------------------------------------------------------------------------


class TestDeserializeMalformedIdentity:
    def test_a_missing_id_raises_a_typed_error(self):
        from apcore.context import Context
        from apcore.errors import ModuleError

        with pytest.raises(ModuleError) as excinfo:
            Context.deserialize({"identity": {"type": "user"}})
        assert excinfo.value.code == "GENERAL_INVALID_INPUT"

    def test_a_non_object_identity_raises_a_typed_error(self):
        from apcore.context import Context
        from apcore.errors import ModuleError

        with pytest.raises(ModuleError) as excinfo:
            Context.deserialize({"identity": "root"})
        assert excinfo.value.code == "GENERAL_INVALID_INPUT"

    def test_no_bare_builtin_escapes(self):
        from apcore.context import Context
        from apcore.errors import ModuleError

        for payload in ({"type": "user"}, "root", 7, ["root"]):
            with pytest.raises(ModuleError):
                Context.deserialize({"identity": payload})

    def test_a_well_formed_identity_still_round_trips(self):
        from apcore.context import Context, Identity

        wire = Context.create(identity=Identity(id="u", roles=("admin",))).serialize()
        revived = Context.deserialize(wire)
        assert revived.identity is not None
        assert revived.identity.id == "u"
        assert revived.identity.roles == ("admin",)


# ---------------------------------------------------------------------------
# CTX-2 — the governance projection is scoped to one ACL evaluation
# ---------------------------------------------------------------------------


class TestGovernanceProjectionIsCallScoped:
    def test_the_projection_is_not_left_on_the_context_after_a_call(self):
        from apcore.context import Context

        registry = Registry()
        registry.register("a.b", _make_module())
        executor = Executor(registry=registry)

        captured: list[Context] = []

        class _CapturingModule(_OkModule):
            def execute(self, inputs, context):
                captured.append(context)
                return {"ok": True}

        registry.register("a.c", _CapturingModule())
        asyncio.run(executor.call_async("a.c", {"amount": 1}))

        assert captured
        assert captured[0].governance_projection is None

    def test_check_access_takes_an_explicit_projection(self):
        from apcore.acl import ACL, ACLRule
        from apcore.context import Context, GovernanceProjection

        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["billing.*"],
                    effect="allow",
                    conditions={"arguments": {"has_key": ["amount"]}},
                )
            ],
            default_effect="deny",
        )
        ctx = Context.create()

        granted = acl.check_access("api.x", "billing.charge", ctx, projection=GovernanceProjection.of({"amount": 1}))
        assert granted.access == "allow"

        # A different call, same Context: the previous projection must not decide it.
        denied = acl.check_access("api.x", "billing.charge", ctx, projection=GovernanceProjection.of({"note": "x"}))
        assert denied.access == "deny"

    def test_the_caller_context_is_not_mutated_by_check_access(self):
        from apcore.acl import ACL
        from apcore.context import Context, GovernanceProjection

        acl = ACL(rules=[], default_effect="deny")
        ctx = Context.create()
        acl.check_access("api.x", "billing.charge", ctx, projection=GovernanceProjection.of({"amount": 1}))

        assert ctx.governance_projection is None

    def test_without_a_projection_an_arguments_condition_stays_unevaluable(self):
        from apcore.acl import ACL, ACLRule
        from apcore.context import Context

        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["billing.*"],
                    effect="allow",
                    conditions={"arguments": {"has_none_of": ["amount"]}},
                )
            ],
            default_effect="deny",
        )
        # No projection: `has_none_of` MUST NOT be vacuously satisfied (§6.1.8).
        assert acl.check_access("api.x", "billing.charge", Context.create()).access == "deny"


# ---------------------------------------------------------------------------
# EVT-001 — the circuit breaker forwards the wrapped subscriber's identity
# ---------------------------------------------------------------------------


class _FilteredSubscriber:
    event_pattern = "apcore.module.*"
    subscriber_id = "webhook-prod"
    subscriber_type = "webhook"

    def __init__(self):
        self.received: list = []

    async def on_event(self, event):
        self.received.append(event)


class TestCircuitBreakerForwardsSubscriberIdentity:
    def test_it_does_not_widen_the_event_pattern(self):
        from apcore.events.circuit_breaker import CircuitBreakerWrapper
        from apcore.events.emitter import EventEmitter, _get_event_pattern

        emitter = EventEmitter()
        wrapped = CircuitBreakerWrapper(_FilteredSubscriber(), emitter)

        assert _get_event_pattern(wrapped) == "apcore.module.*"

    def test_it_forwards_the_subscriber_id_and_type(self):
        from apcore.events.circuit_breaker import CircuitBreakerWrapper
        from apcore.events.emitter import EventEmitter, _get_subscriber_id, _get_subscriber_type

        emitter = EventEmitter()
        wrapped = CircuitBreakerWrapper(_FilteredSubscriber(), emitter)

        assert _get_subscriber_id(wrapped) == "webhook-prod"
        assert _get_subscriber_type(wrapped) == "webhook"

    def test_an_excluded_event_is_not_delivered_through_the_wrapper(self):
        from apcore.events.circuit_breaker import CircuitBreakerWrapper
        from apcore.events.emitter import ApCoreEvent, EventEmitter

        inner = _FilteredSubscriber()
        emitter = EventEmitter()
        emitter.subscribe(CircuitBreakerWrapper(inner, emitter))

        emitter.emit(
            ApCoreEvent(
                event_type="apcore.acl.denied",
                module_id="a.b",
                timestamp="2026-09-15T00:00:00Z",
                severity="warning",
                data={},
            )
        )
        emitter.flush()

        assert inner.received == [], "an event the operator excluded reached the subscriber"

    def test_a_matching_event_still_reaches_it(self):
        from apcore.events.circuit_breaker import CircuitBreakerWrapper
        from apcore.events.emitter import ApCoreEvent, EventEmitter

        inner = _FilteredSubscriber()
        emitter = EventEmitter()
        emitter.subscribe(CircuitBreakerWrapper(inner, emitter))

        emitter.emit(
            ApCoreEvent(
                event_type="apcore.module.executed",
                module_id="a.b",
                timestamp="2026-09-15T00:00:00Z",
                severity="info",
                data={},
            )
        )
        emitter.flush()

        assert len(inner.received) == 1


# ---------------------------------------------------------------------------
# EVT-004 — every subscriber gets a stable identifier at subscribe() time
# ---------------------------------------------------------------------------


class _AnonymousSubscriber:
    async def on_event(self, event):
        raise RuntimeError("always fails")


class TestSubscriberIdIsStable:
    def test_subscribe_assigns_a_generated_id(self):
        from apcore.events.emitter import EventEmitter, _get_subscriber_id

        sub = _AnonymousSubscriber()
        EventEmitter().subscribe(sub)

        sid = _get_subscriber_id(sub)
        assert isinstance(sid, str)
        assert "0x" not in sid, f"the id embeds a heap address: {sid!r}"
        assert sid.startswith("anonymous-"), sid

    def test_two_subscribers_get_distinct_ids(self):
        from apcore.events.emitter import EventEmitter, _get_subscriber_id

        emitter = EventEmitter()
        a, b = _AnonymousSubscriber(), _AnonymousSubscriber()
        emitter.subscribe(a)
        emitter.subscribe(b)

        assert _get_subscriber_id(a) != _get_subscriber_id(b)

    def test_a_declared_id_is_never_overwritten(self):
        from apcore.events.emitter import EventEmitter, _get_subscriber_id

        sub = _FilteredSubscriber()
        EventEmitter().subscribe(sub)

        assert _get_subscriber_id(sub) == "webhook-prod"

    def test_the_id_is_stable_across_reads(self):
        from apcore.events.emitter import EventEmitter, _get_subscriber_id

        sub = _AnonymousSubscriber()
        EventEmitter().subscribe(sub)

        assert _get_subscriber_id(sub) == _get_subscriber_id(sub)


# ---------------------------------------------------------------------------
# SYS-19 — the three `system.control.*` modules declare `idempotent` per module
# ---------------------------------------------------------------------------


class TestSysControlIdempotentAnnotations:
    """system-modules.md states a different value per module; all three took the default."""

    def test_update_config_is_not_idempotent(self):
        from apcore.sys_modules.control import UpdateConfigModule

        assert UpdateConfigModule.annotations.idempotent is False

    def test_reload_module_is_not_idempotent(self):
        from apcore.sys_modules.control import ReloadModuleModule

        assert ReloadModuleModule.annotations.idempotent is False

    def test_toggle_feature_is_idempotent(self):
        from apcore.sys_modules.control import ToggleFeatureModule

        # "toggling to the current state produces the same outcome"
        assert ToggleFeatureModule.annotations.idempotent is True


# ---------------------------------------------------------------------------
# SYS-17 — a bulk reload failure is fatal, not a success response
# ---------------------------------------------------------------------------


class TestBulkReloadFailureIsFatal:
    def _module(self):
        return _make_module()

    def test_a_rediscovery_failure_raises(self):
        from apcore.errors import ReloadFailedError
        from apcore.events.emitter import EventEmitter
        from apcore.sys_modules.control import ReloadModuleModule

        registry = Registry()
        registry.register("demo.one", self._module())
        module = ReloadModuleModule(registry=registry, event_emitter=EventEmitter())

        # `discover()` finds nothing, so the module is unregistered and never
        # restored. That MUST NOT report success.
        with pytest.raises(ReloadFailedError):
            module.execute({"path_filter": "demo.*", "reason": "test"}, None)

    def test_it_does_not_report_success_with_an_empty_reload_list(self):
        from apcore.events.emitter import EventEmitter
        from apcore.sys_modules.control import ReloadModuleModule

        registry = Registry()
        registry.register("demo.one", self._module())
        module = ReloadModuleModule(registry=registry, event_emitter=EventEmitter())

        try:
            result = module.execute({"path_filter": "demo.*", "reason": "test"}, None)
        except Exception:
            return  # fatal, which is the contract
        pytest.fail(f"reported success for a reload that restored nothing: {result}")


# ---------------------------------------------------------------------------
# OBS-007 — the registry callback-error metric has a Prometheus-legal name
# ---------------------------------------------------------------------------


class TestCallbackErrorMetricName:
    def _collector_after_a_callback_error(self):
        from apcore.observability.metrics import MetricsCollector

        collector = MetricsCollector()
        registry = Registry(metrics_collector=collector)

        def _boom(*args, **kwargs):
            raise RuntimeError("callback exploded")

        registry.on("register", _boom)
        registry.register("a.b", _make_module())
        return collector

    def test_the_metric_name_is_prometheus_legal(self):
        import re

        collector = self._collector_after_a_callback_error()
        exposition = collector.export_prometheus()

        for line in exposition.splitlines():
            if not line or line.startswith("#"):
                continue
            name = re.split(r"[{ ]", line, maxsplit=1)[0]
            assert re.fullmatch(r"[a-zA-Z_:][a-zA-Z0-9_:]*", name), f"invalid metric name: {name!r}"

    def test_the_metric_is_named_with_underscores_and_a_total_suffix(self):
        collector = self._collector_after_a_callback_error()
        exposition = collector.export_prometheus()

        assert "apcore_registry_callback_errors_total" in exposition
        assert "apcore.registry.callback_errors" not in exposition

    def test_the_scrape_still_carries_the_other_metrics(self):
        """A dotted name made the WHOLE exposition unparseable, not just its own line."""
        collector = self._collector_after_a_callback_error()
        collector.increment("apcore_module_calls_total", {"module_id": "a.b"})
        exposition = collector.export_prometheus()

        assert "apcore_module_calls_total" in exposition


# ---------------------------------------------------------------------------
# D-122 — shutdown() attempts every cancellation before it reports
# ---------------------------------------------------------------------------


class TestShutdownAttemptsEveryCancellation:
    """A per-task store failure must not strand the remaining tasks.

    The three SDKs split on this while implementing D-81 — two stopped at the
    first failure, one attempted all. The asymmetry decides it: an uncancelled
    task in a shared store holds a ``max_tasks`` slot for every manager sharing
    that store and outlives this process, while a slower shutdown is transient.
    """

    @pytest.mark.asyncio
    async def test_one_failing_cancel_does_not_strand_the_others(self):
        from apcore.async_task import AsyncTaskManager, InMemoryTaskStore, TaskStatus
        from apcore.errors import TaskStoreError

        class OneBadRecordStore(InMemoryTaskStore):
            """Fails the CANCELLED write for exactly one task id."""

            def __init__(self, bad_id: str) -> None:
                super().__init__()
                self._bad_id = bad_id

            async def save(self, info):
                if info.task_id == self._bad_id and info.status is TaskStatus.CANCELLED:
                    raise TaskStoreError(operation="save", reason="conditional write conflict")
                return await super().save(info)

        store = OneBadRecordStore(bad_id="t2")
        mgr = AsyncTaskManager(executor=None, store=store)
        for tid in ("t1", "t2", "t3"):
            await store.save(_pending_task(tid))

        with pytest.raises(TaskStoreError):
            await mgr.shutdown()

        # The failure is reported, AND the tasks it did not concern are cancelled.
        statuses = {i.task_id: i.status for i in await store.list()}
        assert statuses["t1"] is TaskStatus.CANCELLED
        assert statuses["t3"] is TaskStatus.CANCELLED, (
            "a per-task store failure must not strand the tasks after it — "
            "stopping at the first failure leaves them holding max_tasks slots "
            "that outlive this process (D-122)"
        )
        # `t2` is deliberately NOT asserted. `InMemoryTaskStore` hands out the
        # live `TaskInfo`, and `cancel` mutates `info.status` before calling
        # `save`, so the in-memory record reads CANCELLED even though the write
        # raised. That is an aliasing property of this test double, not a D-122
        # behaviour — asserting it would pin the double rather than the contract.
        assert "t2" in statuses

    @pytest.mark.asyncio
    async def test_a_clean_shutdown_still_returns_none(self):
        from apcore.async_task import AsyncTaskManager, InMemoryTaskStore, TaskStatus

        store = InMemoryTaskStore()
        mgr = AsyncTaskManager(executor=None, store=store)
        for tid in ("a", "b"):
            await store.save(_pending_task(tid))

        assert await mgr.shutdown() is None
        assert all(i.status is TaskStatus.CANCELLED for i in await store.list())


def _pending_task(task_id: str):
    """A minimal PENDING TaskInfo the in-memory store accepts."""
    import time as _t

    from apcore.async_task import TaskInfo, TaskStatus

    return TaskInfo(
        task_id=task_id,
        module_id="m.probe",
        status=TaskStatus.PENDING,
        submitted_at=_t.time(),
    )
