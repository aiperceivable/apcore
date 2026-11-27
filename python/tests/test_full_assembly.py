"""Check the full assembly from the same public entry point as application code."""

import subprocess
import sys
from pathlib import Path
from textwrap import dedent

from apcore import APCore as CoreClient
from apcore.discovery import Registry as DiscoveryRegistry
from apcore.full import APCore, Config, Registry


def test_root_runs_in_memory_without_importing_optional_dependencies() -> None:
    subprocess.run(
        [
            sys.executable,
            "-c",
            dedent(
                """
                import asyncio
                import sys
                from importlib.abc import MetaPathFinder

                class BlockOptionalImports(MetaPathFinder):
                    def find_spec(self, fullname, path=None, target=None):
                        optional_modules = {'http.server', 'apcore.acl_loader', 'apcore.registry.watcher'}
                        optional_packages = {'yaml', 'watchdog', 'aiohttp', 'opentelemetry'}
                        if fullname in optional_modules or fullname.split('.')[0] in optional_packages:
                            raise AssertionError(f'Kernel imported optional dependency {fullname}')

                sys.meta_path.insert(0, BlockOptionalImports())
                from apcore import APCore
                from apcore.acl import ACL, ACLRule

                acl = ACL([ACLRule(callers=['app.*'], targets=['example.add'], effect='allow')])
                assert acl.check('app.worker', 'example.add')

                app = APCore()

                @app.module(id='example.add')
                def add(left: int, right: int) -> int:
                    return left + right

                async def exercise():
                    assert (await app.executor.validate_async('example.add', {'left': 2, 'right': 3})).valid
                    assert await app.call_async('example.add', {'left': 2, 'right': 3}) == {'result': 5}

                asyncio.run(exercise())
                app.close()
                """
            ),
        ],
        check=True,
    )


def test_full_entry_assembles_configured_discovery(tmp_path: Path) -> None:
    extensions = tmp_path / "extensions"
    extensions.mkdir()
    config_file = tmp_path / "apcore.yaml"
    config_file.write_text(
        "version: 1.0.0\nproject: {name: full-assembly}\nextensions: {root: ./extensions}\n",
        encoding="utf-8",
    )
    config = Config.load(str(config_file))
    client = APCore(config=config)
    try:
        assert isinstance(client, CoreClient)
        assert isinstance(client.registry, Registry)
        assert isinstance(client.registry, DiscoveryRegistry)
        assert client.discover() == 0
        assert client.config is config
        assert client.executor.governance_state().policy_strict
    finally:
        client.close()


def test_clients_resolve_filtered_subscribers_from_their_own_factories() -> None:
    from apcore.events import ApCoreEvent, SubscriberRegistry

    received: list[list[str]] = [[], []]

    class Sink:
        subscriber_id = "test-sink"
        event_pattern = "*"

        def __init__(self, destination: list[str]) -> None:
            self.destination = destination

        async def on_event(self, event: ApCoreEvent) -> None:
            self.destination.append(event.event_type)

    def build_client(destination: list[str]) -> CoreClient:
        subscribers = SubscriberRegistry()
        subscribers.register("local_sink", lambda _: Sink(destination))
        config = Config(
            {
                "sys_modules": {
                    "enabled": True,
                    "control": {"enabled": False},
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {"type": "filter", "delegate_type": "local_sink", "include_events": ["test.*"]}
                        ],
                    },
                },
            }
        )
        return APCore(config=config, subscriber_registry=subscribers)

    clients = [build_client(destination) for destination in received]
    try:
        for index, client in enumerate(clients):
            assert client.events is not None
            client.events.emit(ApCoreEvent(f"test.event_{index}", None, "2026-01-01T00:00:00Z", "info", {}))
            client.events.flush(timeout=5)
        assert received == [["test.event_0"], ["test.event_1"]]
    finally:
        for client in clients:
            client.close()
