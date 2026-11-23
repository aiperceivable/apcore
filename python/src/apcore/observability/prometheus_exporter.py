"""Prometheus text-format exporter with /metrics, /healthz, and /readyz HTTP endpoints."""

from __future__ import annotations

import logging
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from apcore.observability.metrics import MetricsCollector
    from apcore.observability.usage import UsageCollector

_logger = logging.getLogger(__name__)


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
        self._server: HTTPServer | None = None
        self._thread: threading.Thread | None = None
        self._ready = False
        self._ready_lock = threading.Lock()

    def export(self) -> str:
        """Return current metrics in Prometheus text format.

        Appends the usage collector's ``apcore_usage_*`` families when a usage
        collector was supplied at construction.
        """
        output = self._collector.export_prometheus()
        if self._usage_collector is not None:
            usage_output = self._usage_collector.export_prometheus()
            if usage_output:
                output = f"{output}\n{usage_output}" if output else usage_output
        return output

    def mark_ready(self) -> None:
        """Signal that the application is ready to serve traffic."""
        with self._ready_lock:
            self._ready = True

    def start(self, port: int = 9090, path: str = "/metrics") -> None:
        """Start the HTTP server in a background daemon thread.

        Raises:
            RuntimeError: If the server is already running. Call ``stop()`` first.
        """
        if self._server is not None:
            raise RuntimeError("PrometheusExporter is already running. Call stop() before start().")
        exporter = self

        class _Handler(BaseHTTPRequestHandler):
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

        self._server = HTTPServer(("", port), _Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        _logger.info("PrometheusExporter listening on port %d path=%s", port, path)

    def stop(self) -> None:
        """Shut down the HTTP server."""
        if self._server is not None:
            self._server.shutdown()
            self._server = None
        if self._thread is not None:
            self._thread.join(timeout=5.0)
            self._thread = None


__all__ = ["PrometheusExporter"]
