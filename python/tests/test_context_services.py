"""Tests for Context[T] generic services field."""

from __future__ import annotations

from dataclasses import dataclass

from apcore.context import Context


@dataclass
class MockServices:
    db: str = "test_db"
    cache: str = "test_cache"


class TestContextServices:
    def test_create_with_services(self) -> None:
        svc = MockServices()
        ctx = Context.create(services=svc)
        assert ctx.services is svc
        assert ctx.services.db == "test_db"

    def test_create_without_services_defaults_to_none(self) -> None:
        ctx = Context.create()
        assert ctx.services is None

    def test_child_inherits_services(self) -> None:
        svc = MockServices(db="prod_db")
        ctx = Context.create(services=svc)
        child = ctx.child("child.module")
        assert child.services is svc
        assert child.services.db == "prod_db"

    def test_child_chain_preserves_services(self) -> None:
        svc = MockServices()
        ctx = Context.create(services=svc)
        child1 = ctx.child("mod.a")
        child2 = child1.child("mod.b")
        assert child2.services is svc

    def test_services_not_in_serialize(self) -> None:
        svc = MockServices()
        ctx = Context.create(services=svc)
        serialized = ctx.serialize()
        assert "services" not in serialized

    def test_deserialize_services_is_none(self) -> None:
        serialized = {
            "trace_id": "t1",
            "caller_id": None,
            "call_chain": [],
            "identity": None,
            "redacted_inputs": None,
            "data": {},
        }
        restored = Context.deserialize(serialized)
        assert restored.services is None
