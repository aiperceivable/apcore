"""Stub tests to confirm apcore.middleware.base is importable."""


def test_middleware_base_importable():
    from apcore.middleware.base import Middleware

    assert Middleware is not None
