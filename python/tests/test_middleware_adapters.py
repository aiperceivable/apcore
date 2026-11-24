"""Stub tests to confirm apcore.middleware.adapters is importable."""


def test_middleware_adapters_importable():
    from apcore.middleware.adapters import BeforeMiddleware, AfterMiddleware

    assert BeforeMiddleware is not None
    assert AfterMiddleware is not None
