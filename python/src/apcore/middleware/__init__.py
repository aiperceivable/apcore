"""Middleware base class and adapters for apcore."""

from apcore.middleware.adapters import AfterMiddleware, BeforeMiddleware
from apcore.middleware.base import Middleware, RetrySignal
from apcore.middleware.circuit_breaker import CircuitBreakerMiddleware, CircuitBreakerState
from apcore.middleware.context_namespace import (
    APCORE_KEY_PREFIX,
    EXT_KEY_PREFIX,
    ContextWriter,
    NamespaceCheck,
    enforce_context_key,
    validate_context_key,
)
from apcore.middleware.error_history_middleware import ErrorHistoryMiddleware
from apcore.middleware.logging import LoggingMiddleware
from apcore.middleware.manager import MiddlewareChainError, MiddlewareManager
from apcore.middleware.platform_notify import PlatformNotifyMiddleware
from apcore.middleware.retry import RetryConfig, RetryMiddleware

__all__ = [
    "Middleware",
    "RetrySignal",
    "BeforeMiddleware",
    "AfterMiddleware",
    "MiddlewareManager",
    "MiddlewareChainError",
    "LoggingMiddleware",
    "RetryConfig",
    "RetryMiddleware",
    "ErrorHistoryMiddleware",
    "PlatformNotifyMiddleware",
    "CircuitBreakerMiddleware",
    "CircuitBreakerState",
    "APCORE_KEY_PREFIX",
    "EXT_KEY_PREFIX",
    "ContextWriter",
    "NamespaceCheck",
    "validate_context_key",
    "enforce_context_key",
]
