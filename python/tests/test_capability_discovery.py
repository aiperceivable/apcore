"""Discovery isolation and lifecycle cases that need the owning Python host."""

from typing import Any

import pytest

from apcore import APCore, Context, Executor, Identity, ModuleAnnotations, Registry
from apcore import capabilities
from apcore.acl import ACL, ACLRule
from apcore.builtin_steps import build_standard_strategy
from apcore.config import Config
from apcore.errors import ACLDeniedError, InvalidInputError, ModuleError, SchemaValidationError
from apcore.pipeline import BaseStep, ExecutionStrategy, PipelineContext, StepResult


class Module:
    input_schema = {"type": "object", "properties": {"key": {"type": "string"}}}
    output_schema = {"type": "object"}
    description = "Test module"

    def __init__(self, *, discoverable: bool = True, version: str = "1.0.0") -> None:
        self.version = version
        self.annotations = ModuleAnnotations(discoverable=discoverable)

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        pytest.fail("Discovery must not enter a module")


def test_cursor_is_scoped_rechecked_and_input_validation_is_generic() -> None:
    registry = Registry()
    for module_id in ("demo.a", "demo.b", "demo.c"):
        registry.register(module_id, Module())
    acl = ACL([ACLRule(["@external"], ["demo.*"], "allow")])
    with Executor(registry, acl=acl) as executor:
        identity = Identity("reader", roles=("staff",), attrs={"tenant": {"region": "north"}})
        context = Context.create(identity=identity)
        first = executor.list_callable(context, limit=1)
        assert [item.module_id for item in first.items] == ["demo.a"]
        assert first.next_cursor
        for foreign in (
            Context.create(identity=Identity("other", roles=identity.roles, attrs=identity.attrs)),
            Context.create(identity=Identity("reader", roles=("other",), attrs=identity.attrs)),
            Context.create(identity=Identity("reader", roles=identity.roles, attrs={"tenant": {"region": "south"}})),
            context.child("other.module"),
        ):
            with pytest.raises(InvalidInputError, match="Invalid discovery cursor"):
                executor.list_callable(foreign, limit=1, cursor=first.next_cursor)
        with pytest.raises(InvalidInputError, match="Invalid discovery cursor"):
            executor.list_callable(context, limit=1, prefix="demo.", cursor=first.next_cursor)
        with pytest.raises(InvalidInputError, match="Invalid discovery cursor"):
            executor.list_callable(context, limit=1, cursor=f"{first.next_cursor}unexpected")
        acl.add_rule(callers=["@external"], targets=["demo.b"], effect="deny")
        second = executor.list_callable(context, limit=1, cursor=first.next_cursor)
        assert [item.module_id for item in second.items] == ["demo.c"]
        assert second.next_cursor is None


@pytest.mark.parametrize(("original", "other"), [(True, 1), (1, 1.0), (-0.0, 0.0)])
def test_cursor_distinguishes_identity_attribute_types(original: Any, other: Any) -> None:
    registry = Registry()
    for module_id in ("demo.a", "demo.b"):
        registry.register(module_id, Module())
    with Executor(registry) as executor:
        context = Context.create(identity=Identity("reader", attrs={"tenant": [original]}))
        first = executor.list_callable(context, limit=1)
        assert first.next_cursor
        altered = Context.create(identity=Identity("reader", attrs={"tenant": [other]}))
        with pytest.raises(InvalidInputError, match="Invalid discovery cursor"):
            executor.list_callable(altered, limit=1, cursor=first.next_cursor)


def test_cursor_expires_at_its_monotonic_deadline(monkeypatch: pytest.MonkeyPatch) -> None:
    registry = Registry()
    for module_id in ("demo.a", "demo.b"):
        registry.register(module_id, Module())
    now = 1_000.0
    monkeypatch.setattr(capabilities, "monotonic", lambda: now)
    with Executor(registry) as executor:
        cursor = executor.list_callable(limit=1).next_cursor
        assert cursor
        now = 1_299.999
        assert [item.module_id for item in executor.list_callable(limit=1, cursor=cursor).items] == ["demo.b"]
        now = 1_300.0
        with pytest.raises(InvalidInputError, match="Invalid discovery cursor"):
            executor.list_callable(limit=1, cursor=cursor)


@pytest.mark.asyncio
@pytest.mark.parametrize("step_name", ["context_creation", "module_lookup"])
async def test_discovery_omits_targets_skipped_by_essential_pipeline_step(step_name: str) -> None:
    registry = Registry()
    registry.register("demo.visible", Module())
    registry.register("demo.matching", Module())
    acl = ACL([ACLRule(["worker.reader"], ["demo.*"], "allow")])
    config = Config({"pipeline": {"configure": {step_name: {"match_modules": ["demo.match*"]}}}})
    context = Context.create().child("worker.reader")
    with Executor(registry, acl=acl, config=config) as executor:
        with pytest.raises(ModuleError) as actual:
            await executor.call_async("demo.visible", {}, context)
        if step_name == "context_creation":
            assert isinstance(actual.value, ACLDeniedError)
        assert executor.get_callable_descriptor("demo.visible", context) is None
        assert [item.module_id for item in executor.list_callable(context).items] == ["demo.matching"]


@pytest.mark.asyncio
async def test_discovery_does_not_guess_a_custom_context_step() -> None:
    class HostContextStep(BaseStep):
        async def execute(self, ctx: PipelineContext) -> StepResult:
            return StepResult(action="continue")

    registry = Registry()
    registry.register("demo.visible", Module())
    acl = ACL([ACLRule(["worker.reader"], ["demo.*"], "allow")])
    steps = build_standard_strategy(registry=registry).steps
    steps[0] = HostContextStep(name="context_creation", description="Host caller frame", provides=("context",))
    strategy = ExecutionStrategy("host-context", steps)
    context = Context.create().child("worker.reader")
    with Executor(registry, acl=acl, strategy=strategy) as executor:
        with pytest.raises(ACLDeniedError):
            await executor.call_async("demo.visible", {}, context)
        assert executor.get_callable_descriptor("demo.visible", context) is None
        assert not executor.list_callable(context).items


def test_full_definition_uses_live_version_and_detaches_schemas() -> None:
    registry = Registry()
    registry.register("demo.versioned", Module(discoverable=False), version="1.0.0")
    registry.register("demo.versioned", Module(version="2.0.0"), version="2.0.0")
    with Executor(registry) as executor:
        assert executor.get_callable_definition("demo.versioned", version_hint="1.0.0") is None
        definition = executor.get_callable_definition("demo.versioned", version_hint="2.0.0")
        assert definition is not None and definition.version == "2.0.0"
        definition.input_schema["properties"].clear()
        assert executor.get_callable_definitions(["demo.versioned"])[0].input_schema == Module.input_schema
        executor.toggle_state.disable("demo.versioned")
        try:
            assert executor.get_callable_definition("demo.versioned") is None
            assert not executor.list_callable().items
        finally:
            executor.toggle_state.enable("demo.versioned")


def test_summary_readonly_uses_descriptor_while_approval_uses_both_annotations() -> None:
    registry = Registry()
    module = Module()
    module.annotations = ModuleAnnotations(requires_approval=True)
    registry.register(
        "demo.annotated", module, metadata={"annotations": {"readonly": True, "requires_approval": False}}
    )
    with Executor(registry) as executor:
        (summary,) = executor.list_callable(readonly=True).items
        assert summary.readonly and summary.access_hint == "requires_approval"


def test_search_provider_output_cannot_disclose_hidden_or_denied_ids() -> None:
    registry = Registry()
    for module_id in ("demo.allowed", "demo.denied"):
        registry.register(module_id, Module())
    registry.register("demo.hidden", Module(discoverable=False))

    class SearchProvider:
        def search(self, query: str | None, candidates: tuple[str, ...]) -> list[str]:
            assert query == "semantic match"
            assert candidates == ("demo.allowed",)
            return ["demo.hidden", "demo.denied", "demo.unknown", "demo.allowed", "demo.allowed"]

    acl = ACL([ACLRule(["@external"], ["demo.allowed"], "allow")])
    with Executor(registry, acl=acl, capability_search_provider=SearchProvider()) as executor:
        assert [item.module_id for item in executor.list_callable(query="semantic match").items] == ["demo.allowed"]


def test_summary_page_drops_a_replaced_registration_before_emission(monkeypatch: pytest.MonkeyPatch) -> None:
    registry = Registry()
    for module_id in ("demo.a", "demo.b"):
        registry.register(module_id, Module())
    snapshot = registry._discovery_registration

    def replace_prior_registration(module_id: str, version_hint: str | None) -> Any:
        if module_id == "demo.b" and registry.has("demo.a"):
            assert registry.unregister("demo.a")
        return snapshot(module_id, version_hint)

    monkeypatch.setattr(registry, "_discovery_registration", replace_prior_registration)
    with Executor(registry) as executor:
        assert [item.module_id for item in executor.list_callable().items] == ["demo.b"]


def test_effective_custom_condition_is_never_called_and_faults_stay_silent(monkeypatch: pytest.MonkeyPatch) -> None:
    class Replacement:
        async def evaluate(self, value: Any, context: Context) -> bool:
            pytest.fail("Discovery must not invoke a registered replacement")

    monkeypatch.setitem(ACL._async_condition_handlers, "roles", Replacement())
    audits = []
    acl = ACL(
        [
            ACLRule(["@external"], ["demo.*"], "deny", conditions={"roles": ["reader"]}),
            ACLRule(["@external"], ["demo.*"], "allow"),
        ],
        audit_logger=audits.append,
    )
    context = Context.create(identity=Identity("actor", roles=("operator",)))
    assert not acl.evaluate_visibility(None, "demo.allowed", context.child("demo.allowed")).visible
    assert not audits


@pytest.mark.asyncio
@pytest.mark.parametrize("conditions", [{"$or": [{}]}, {"$not": {"roles": ["unknown"]}}])
async def test_discovery_uses_registered_compound_condition_identity(
    monkeypatch: pytest.MonkeyPatch, conditions: dict[str, Any]
) -> None:
    calls: list[str] = []

    class Replacement:
        async def evaluate(self, value: Any, context: Context) -> bool:
            calls.append("evaluated")
            return False

    key = next(iter(conditions))
    monkeypatch.setattr(ACL, "_async_condition_handlers", ACL._async_condition_handlers.copy())
    ACL.register_async_condition(key, Replacement())
    registry = Registry()
    registry.register("demo.visible", Module())
    acl = ACL([ACLRule(["@external"], ["demo.visible"], "allow", conditions=conditions)])
    context = Context.create(identity=Identity("reader", roles=("staff",)))
    with Executor(registry, acl=acl) as executor:
        with pytest.raises(ACLDeniedError):
            await executor.call_async("demo.visible", {}, context)
        assert calls == ["evaluated"]
        assert executor.get_callable_descriptor("demo.visible", context) is None
        assert not executor.list_callable(context).items
        assert calls == ["evaluated"]


def test_definition_discards_schema_when_the_same_version_is_replaced(monkeypatch: pytest.MonkeyPatch) -> None:
    registry = Registry()
    registry.register("demo.versioned", Module(), version="1.0.0")
    describe = registry._describe_registration
    replaced = False

    def replace_registration(*args: Any, **kwargs: Any) -> Any:
        nonlocal replaced
        metadata = describe(*args, **kwargs)
        if not replaced:
            replaced = True
            assert registry.unregister("demo.versioned")
            registry.register("demo.versioned", Module(discoverable=False), version="1.0.0")
        return metadata

    monkeypatch.setattr(registry, "_describe_registration", replace_registration)
    with Executor(registry) as executor:
        assert executor.get_callable_descriptor("demo.versioned", version_hint="1.0.0") is None
        assert replaced


@pytest.mark.asyncio
async def test_manifest_search_does_not_use_its_own_caller_grant() -> None:
    registry = Registry()
    registry.register("demo.visible", Module())
    registry.register("demo.private", Module())
    acl = ACL(
        [
            ACLRule(["@external"], ["system.manifest.search"], "allow"),
            ACLRule(["@external"], ["demo.visible"], "allow", conditions={"max_call_depth": 1}),
            ACLRule(["system.manifest.search"], ["demo.private"], "allow"),
        ]
    )
    config = Config({"sys_modules": {"enabled": True}})
    with Executor(registry, acl=acl) as executor:
        app = APCore(executor=executor, config=config)
        result = await app.call_async("system.manifest.search", {"prefix": "demo."})
        assert [item["module_id"] for item in result["items"]] == ["demo.visible"]


@pytest.mark.asyncio
async def test_manifest_search_rejects_null_wire_options() -> None:
    app = APCore(config=Config({"sys_modules": {"enabled": True}}))
    try:
        for name in ("query", "prefix", "tags", "readonly", "limit", "cursor", "include_approval_required"):
            with pytest.raises(SchemaValidationError):
                await app.call_async("system.manifest.search", {name: None})
        assert (await app.call_async("system.manifest.search", {}))["next_cursor"] is None
    finally:
        await app.shutdown()
