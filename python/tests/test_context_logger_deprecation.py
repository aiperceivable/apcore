"""`Context.logger` is deprecated (apcore#121), removed at v2.0.

The decision is D-67's boundary applied to an API surface rather than a
configuration key: apcore does not own the host's logging policy, and this
accessor's output is fixed at stderr / `info` / JSON no matter what the host has
configured. Wiring it instead was rejected on a cost that is not effort — the
logger is built from a `Context`, which carries no `Config` in any SDK, so the
only route avoiding `Context.create`'s pinned signature is a process-global
logger, which would end the multi-instance isolation apcore gets for free.

`ObsLoggingMiddleware` is deliberately NOT named as the migration target here:
it emits apcore's execution events, which is a different facility.
"""

from __future__ import annotations

import warnings

import pytest

from apcore.context import Context
from apcore.observability.context_logger import ContextLogger


def test_accessing_the_logger_warns() -> None:
    context = Context(trace_id="t-1", caller_id="api.probe")
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        context.logger
    notices = [w for w in caught if issubclass(w.category, DeprecationWarning)]
    assert len(notices) == 1
    message = str(notices[0].message)
    assert "removed in version 2.0" in message
    assert "host application's logger" in message
    assert "apcore#121" in message


def test_it_still_works_through_the_1_x_line() -> None:
    """A deprecation is a notice, not a removal — §13.2's two-minor floor.

    The accessor keeps returning a logger carrying this context's correlation
    fields until v2.0, so a project that has not migrated yet still runs.
    """
    context = Context(trace_id="t-1", caller_id="api.probe")
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        logger = context.logger
    assert isinstance(logger, ContextLogger)
    assert logger._trace_id == "t-1"
    assert logger._caller_id == "api.probe"


def test_no_apcore_code_path_reaches_it() -> None:
    """The premise of deprecating rather than wiring: nothing internal uses it.

    Written as a test rather than left as a claim in the issue, because it is
    what makes removal at v2.0 safe for the framework itself. A future caller
    inside `src/` fails here and has to argue for itself.
    """
    import pathlib

    src = pathlib.Path(__file__).resolve().parent.parent / "src" / "apcore"
    hits = [
        f"{path.relative_to(src)}:{n}"
        for path in src.rglob("*.py")
        for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1)
        if ("context.logger" in line or "ctx.logger" in line) and "def logger" not in line
    ]
    assert hits == [], f"apcore's own code now reaches Context.logger: {hits}"


@pytest.mark.filterwarnings("error::DeprecationWarning")
def test_nothing_in_the_import_path_touches_it() -> None:
    """Importing and building a client must not trip the notice itself."""
    from apcore import APCore
    from apcore.config import Config

    APCore(config=Config({"version": "1.0", "project": {"name": "p"}}))
