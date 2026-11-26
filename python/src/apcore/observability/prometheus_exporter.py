"""Prometheus text-format exporter with /metrics, /healthz, and /readyz HTTP endpoints."""

from __future__ import annotations

import logging
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from io import BufferedReader, RawIOBase
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from socket import socket

    from _typeshed import WriteableBuffer

    from apcore.observability.metrics import MetricsCollector
    from apcore.observability.usage import UsageCollector

    _ServerRequest = socket | tuple[bytes, socket]

_logger = logging.getLogger(__name__)
_MAX_CONCURRENT_CONNECTIONS = 64
_REQUEST_READ_TIMEOUT = 10.0
_RESPONSE_WRITE_TIMEOUT = 10.0


class _RequestReader(RawIOBase):
    """Read the complete HTTP request headers within one deadline."""

    def __init__(self, connection: socket) -> None:
        self._connection = connection
        self._deadline = time.monotonic() + _REQUEST_READ_TIMEOUT

    def readable(self) -> bool:
        return True

    def readinto(self, buffer: WriteableBuffer, /) -> int:
        remaining = self._deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("Prometheus request headers timed out")
        self._connection.settimeout(remaining)
        try:
            return self._connection.recv_into(buffer)
        finally:
            self._connection.settimeout(_RESPONSE_WRITE_TIMEOUT)


class _PrometheusHTTPServer(ThreadingHTTPServer):
    def __init__(self, address: tuple[str, int], handler: type[BaseHTTPRequestHandler]) -> None:
        self._slots = threading.BoundedSemaphore(_MAX_CONCURRENT_CONNECTIONS)
        super().__init__(address, handler)

    def process_request(self, request: _ServerRequest, client_address: tuple[str, int]) -> None:
        if not self._slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self._slots.release()
            raise

    def process_request_thread(self, request: _ServerRequest, client_address: tuple[str, int]) -> None:
        try:
            super().process_request_thread(request, client_address)
        finally:
            self._slots.release()


def render_prometheus_metrics(collector: MetricsCollector, usage_collector: UsageCollector | None = None) -> str:
    """Render live metrics and optional usage as Prometheus text."""
    output = collector.export_prometheus()
    if usage_collector is not None:
        output += usage_collector.export_prometheus()
    return output


class PrometheusExporter:
    """Serves Prometheus text metrics and health endpoints over HTTP.

    Starts a background HTTP server on ``start()``. Serves:
        GET /metrics (or configured path) — Prometheus text exposition format
        GET /healthz  — liveness probe, always 200 OK
        GET /readyz   — readiness probe, 200 OK after ``mark_ready()`` is called

    Args:
        collector: The MetricsCollector to read metrics from.
        usage_collector: Optional UsageCollector. When provided, its
            ``apcore_usage_*`` metric families are appended to the /metrics
            output (parity with the TS and Rust exporters). Defaults to None,
            which preserves the previous MetricsCollector-only behavior.
    """

    def __init__(self, collector: MetricsCollector, usage_collector: UsageCollector | None = None) -> None:
        self._collector = collector
        self._usage_collector = usage_collector
        self._server: _PrometheusHTTPServer | None = None
        self._thread: threading.Thread | None = None
        self._listener_lock = threading.Lock()
        self._ready = False
        self._ready_lock = threading.Lock()

    def export(self) -> str:
        """Return current metrics in Prometheus text format.

        Appends the usage collector's ``apcore_usage_*`` families when a usage
        collector was supplied at construction.
        """
        return render_prometheus_metrics(self._collector, self._usage_collector)

    def mark_ready(self) -> None:
        """Signal that the application is ready to serve traffic."""
        with self._ready_lock:
            self._ready = True

    def start(self, port: int = 9090, path: str = "/metrics", *, host: str = "127.0.0.1") -> None:
        """Start the HTTP server in a background daemon thread, bound to ``host``.

        Raises:
            RuntimeError: If the server is already running. Call ``stop()`` first.
        """
        exporter = self

        class _Handler(BaseHTTPRequestHandler):
            def setup(self) -> None:
                super().setup()
                self.rfile.close()
                self.rfile = BufferedReader(_RequestReader(self.connection))
                self.connection.settimeout(_RESPONSE_WRITE_TIMEOUT)

            def do_GET(self) -> None:  # noqa: N802
                if self.path == path:
                    body = exporter.export().encode("utf-8")
                    self.send_response(200)
                    self.send_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                elif self.path == "/healthz":
                    self.send_response(200)
                    self.end_headers()
                    self.wfile.write(b"OK")
                elif self.path == "/readyz":
                    with exporter._ready_lock:
                        ready = exporter._ready
                    if ready:
                        self.send_response(200)
                        self.end_headers()
                        self.wfile.write(b"OK")
                    else:
                        self.send_response(503)
                        self.end_headers()
                        self.wfile.write(b"Not ready")
                else:
                    self.send_response(404)
                    self.end_headers()

            def log_message(self, fmt: str, *args: object) -> None:  # type: ignore[override]
                pass  # suppress access logs

        with self._listener_lock:
            if self._server is not None:
                raise RuntimeError("PrometheusExporter is already running. Call stop() before start().")
            server = _PrometheusHTTPServer((host, port), _Handler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            try:
                thread.start()
            except RuntimeError:
                server.server_close()
                raise
            self._server = server
            self._thread = thread
        _logger.info("PrometheusExporter listening at %s:%d path=%s", *server.server_address, path)

    def stop(self) -> None:
        """Stop accepting new connections and release the listening socket."""
        with self._listener_lock:
            if self._server is None or self._thread is None:
                return
            self._server.shutdown()
            self._thread.join()
            self._server.server_close()
            self._server = None
            self._thread = None


__all__ = ["PrometheusExporter", "render_prometheus_metrics"]
