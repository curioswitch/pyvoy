from __future__ import annotations

import asyncio
import json
import os
import secrets
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import TYPE_CHECKING, Any, Self, TypeVar

import pytest
import pytest_asyncio
import trustme
from opentelemetry.proto.collector.metrics.v1.metrics_service_pb2 import (
    ExportMetricsServiceRequest,
)
from opentelemetry.proto.collector.trace.v1.trace_service_pb2 import (
    ExportTraceServiceRequest,
)

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Callable, Iterator
    from pathlib import Path

    from opentelemetry.proto.metrics.v1.metrics_pb2 import Metric
    from opentelemetry.proto.trace.v1.trace_pb2 import Span
    from pyqwest import Client


T = TypeVar("T")


@pytest.fixture
def ca_cert(tmp_path: Path) -> str:
    path = tmp_path / "ca.pem"
    trustme.CA().cert_pem.write_to_path(path)
    return str(path)


class _Collector:
    """An OTLP/HTTP collector recording what it receives."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._requests: list[tuple[str, bytes]] = []
        collector = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:
                body = self._read_body()
                with collector._lock:
                    collector._requests.append((self.path, body))
                self.send_response(200)
                self.send_header("content-length", "0")
                self.end_headers()

            def _read_body(self) -> bytes:
                if self.headers.get("transfer-encoding") != "chunked":
                    return self.rfile.read(int(self.headers.get("content-length", 0)))
                body = bytearray()
                while size := int(self.rfile.readline().strip(), 16):
                    body += self.rfile.read(size)
                    self.rfile.readline()
                self.rfile.readline()
                return bytes(body)

            def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
                pass

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.endpoint = f"http://127.0.0.1:{self._server.server_port}"

    def __enter__(self) -> Self:
        threading.Thread(target=self._server.serve_forever, daemon=True).start()
        return self

    def __exit__(self, *_args: object) -> None:
        self._server.shutdown()
        self._server.server_close()

    def _bodies(self, path: str) -> list[bytes]:
        with self._lock:
            return [body for p, body in self._requests if p == path]

    def spans(self) -> dict[str, Span]:
        """Returns the received spans by trace ID."""
        spans: dict[str, Span] = {}
        for body in self._bodies("/v1/traces"):
            request = ExportTraceServiceRequest.FromString(body)
            for resource_spans in request.resource_spans:
                for scope_spans in resource_spans.scope_spans:
                    for span in scope_spans.spans:
                        spans[span.trace_id.hex()] = span
        return spans

    def metrics(self, service_name: str) -> dict[str, Metric]:
        """Returns the received metrics of the service by name."""
        metrics: dict[str, Metric] = {}
        for body in self._bodies("/v1/metrics"):
            request = ExportMetricsServiceRequest.FromString(body)
            for resource_metrics in request.resource_metrics:
                attributes = {
                    attribute.key: attribute.value.string_value
                    for attribute in resource_metrics.resource.attributes
                }
                if attributes.get("service.name") != service_name:
                    continue
                for scope_metrics in resource_metrics.scope_metrics:
                    for metric in scope_metrics.metrics:
                        metrics[metric.name] = metric
        return metrics


# Servers in this module write Envoy's logs to stderr, which pytest shows for
# failed tests, since OpenTelemetry config errors only appear there.


class _FastFlushServer(PyvoyServer):
    """A server whose tracer exports each span as soon as it finishes."""

    def get_envoy_config(self) -> dict:
        config = super().get_envoy_config()
        config["layered_runtime"] = {
            "layers": [
                {
                    "name": "static",
                    "static_layer": {"tracing.opentelemetry.min_flush_spans": 1},
                }
            ]
        }
        return config


async def _eventually(check: Callable[[], T | None]) -> T:
    for _ in range(100):
        if (result := check()) is not None:
            return result
        await asyncio.sleep(0.1)
    msg = "condition not met"
    raise AssertionError(msg)


def _ratio_sampled(trace_id: str, ratio: float) -> bool:
    """Envoy's trace ID ratio sampling decision."""
    value = int.from_bytes(bytes.fromhex(trace_id[:16]), sys.byteorder)
    return value % 1_000_000 < round(ratio * 1_000_000)


@pytest.fixture(autouse=True)
def clean_otel_env(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in list(os.environ):
        if name.startswith("OTEL_"):
            monkeypatch.delenv(name)


@pytest.fixture(scope="module")
def collector() -> Iterator[_Collector]:
    with _Collector() as collector:
        yield collector


def _exporter_env(collector: _Collector) -> dict[str, str]:
    return {
        "OTEL_EXPORTER_OTLP_ENDPOINT": collector.endpoint,
        "OTEL_EXPORTER_OTLP_PROTOCOL": "http/protobuf",
    }


@pytest_asyncio.fixture(scope="module")
async def traced_server(collector: _Collector) -> AsyncIterator[PyvoyServer]:
    async with _FastFlushServer(
        "tests.apps.asgi.otel", env=_exporter_env(collector), stderr=None
    ) as server:
        yield server


@pytest.mark.asyncio
async def test_new_trace(
    traced_server: PyvoyServer, collector: _Collector, client: Client
) -> None:
    response = await client.get(
        f"http://localhost:{traced_server.listener_port}/",
        headers=(("tracestate", "vendor=orphaned"),),
    )
    assert response.status == 200, response.text()
    result = json.loads(response.content)

    envoy_span = await _eventually(lambda: collector.spans().get(result["trace_id"]))
    assert envoy_span.name == "GET"
    assert envoy_span.parent_span_id == b""
    # The application's instrumentation continues the trace from Envoy's span.
    assert result["parent_span_id"] == envoy_span.span_id.hex()
    assert result["sampled"]
    assert (
        result["traceparent"]
        == f"00-{result['trace_id']}-{envoy_span.span_id.hex()}-01"
    )
    # A tracestate without a traceparent does not belong to the new trace.
    assert result["tracestate"] is None


@pytest.mark.asyncio
async def test_continued_trace(
    traced_server: PyvoyServer, collector: _Collector, client: Client
) -> None:
    trace_id = secrets.token_hex(16)
    response = await client.get(
        f"http://localhost:{traced_server.listener_port}/",
        headers=(
            ("traceparent", f"00-{trace_id}-00f067aa0ba902b7-01"),
            ("tracestate", "vendor=value"),
        ),
    )
    assert response.status == 200, response.text()
    result = json.loads(response.content)
    assert result["trace_id"] == trace_id

    envoy_span = await _eventually(lambda: collector.spans().get(trace_id))
    assert envoy_span.parent_span_id.hex() == "00f067aa0ba902b7"
    assert envoy_span.trace_state == "vendor=value"
    assert result["parent_span_id"] == envoy_span.span_id.hex()
    assert result["sampled"]
    assert result["tracestate"] == "vendor=value"


@pytest.mark.asyncio
async def test_unsampled_parent(
    traced_server: PyvoyServer, collector: _Collector, client: Client
) -> None:
    trace_id = secrets.token_hex(16)
    response = await client.get(
        f"http://localhost:{traced_server.listener_port}/",
        headers=(("traceparent", f"00-{trace_id}-00f067aa0ba902b7-00"),),
    )
    assert response.status == 200, response.text()
    result = json.loads(response.content)
    assert result["trace_id"] == trace_id
    assert not result["sampled"]
    assert result["traceparent"].endswith("-00")
    assert result["traceparent"] != f"00-{trace_id}-00f067aa0ba902b7-00"

    # Spans export in order, so a later sampled span shows whether this one was.
    sampled = await client.get(f"http://localhost:{traced_server.listener_port}/")
    sampled_trace_id = json.loads(sampled.content)["trace_id"]
    await _eventually(lambda: collector.spans().get(sampled_trace_id))
    assert trace_id not in collector.spans()


@pytest.mark.asyncio
async def test_trace_id_ratio_sampler(collector: _Collector, client: Client) -> None:
    ratio = 0.5
    sampled_ids: list[str] = []
    unsampled_ids: list[str] = []
    while len(sampled_ids) < 6 or len(unsampled_ids) < 5:
        trace_id = secrets.token_hex(16)
        if _ratio_sampled(trace_id, ratio):
            sampled_ids.append(trace_id)
        else:
            unsampled_ids.append(trace_id)
    # Spans export in order, so ending with a sampled span shows whether the
    # others were.
    trace_ids = [
        *(
            t
            for pair in zip(sampled_ids[:5], unsampled_ids[:5], strict=True)
            for t in pair
        ),
        sampled_ids[5],
    ]

    async with _FastFlushServer(
        "tests.apps.asgi.otel",
        stderr=None,
        env={
            **_exporter_env(collector),
            "OTEL_TRACES_SAMPLER": "traceidratio",
            "OTEL_TRACES_SAMPLER_ARG": str(ratio),
        },
    ) as server:
        results: dict[str, bool] = {}
        # Without parent-based sampling, the parent's sampled flag is ignored.
        for i, trace_id in enumerate(trace_ids):
            response = await client.get(
                f"http://localhost:{server.listener_port}/",
                headers=(
                    ("traceparent", f"00-{trace_id}-00f067aa0ba902b7-0{i // 2 % 2}"),
                ),
            )
            assert response.status == 200, response.text()
            results[trace_id] = json.loads(response.content)["sampled"]

        await _eventually(lambda: collector.spans().get(trace_ids[-1]))

    exported = collector.spans()
    for trace_id, sampled in results.items():
        # The sampled flag pyvoy propagates must match Envoy's own decision.
        assert sampled == (trace_id in exported), trace_id
        assert sampled == _ratio_sampled(trace_id, ratio), trace_id


@pytest.mark.asyncio
async def test_wsgi(collector: _Collector, client: Client) -> None:
    async with _FastFlushServer(
        "tests.apps.wsgi.otel",
        interface="wsgi",
        env=_exporter_env(collector),
        stderr=None,
    ) as server:
        trace_id = secrets.token_hex(16)
        response = await client.get(
            f"http://localhost:{server.listener_port}/",
            headers=(("traceparent", f"00-{trace_id}-00f067aa0ba902b7-01"),),
        )
        assert response.status == 200, response.text()
        result = json.loads(response.content)

        envoy_span = await _eventually(lambda: collector.spans().get(trace_id))
    assert result["traceparent"] == f"00-{trace_id}-{envoy_span.span_id.hex()}-01"


@pytest.mark.asyncio
async def test_metrics(collector: _Collector, client: Client) -> None:
    async with PyvoyServer(
        "tests.apps.asgi.otel",
        stderr=None,
        env={
            **_exporter_env(collector),
            "OTEL_SERVICE_NAME": "metrics-test",
            "OTEL_METRIC_EXPORT_INTERVAL": "100",
            "OTEL_TRACES_EXPORTER": "none",
        },
    ) as server:
        response = await client.get(f"http://localhost:{server.listener_port}/")
        assert response.status == 200, response.text()
        # Tracing is disabled, so the application receives the request unchanged.
        assert json.loads(response.content)["traceparent"] is None

        def requests_total() -> Metric | None:
            metric = collector.metrics("metrics-test").get(
                "envoy.http.downstream_rq_total"
            )
            if metric is None or not any(
                point.as_int for point in metric.sum.data_points
            ):
                return None
            return metric

        metric = await _eventually(requests_total)
    prefixes = {
        attribute.value.string_value
        for point in metric.sum.data_points
        for attribute in point.attributes
        if attribute.key == "envoy.http_conn_manager_prefix"
    }
    assert prefixes == {"ingress_http"}
    assert not any(
        name.startswith("envoy.listener.admin.")
        for name in collector.metrics("metrics-test")
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("protocol", ["grpc", "http/protobuf"])
async def test_tls_collector_starts(
    protocol: str, ca_cert: str, client: Client
) -> None:
    # Nothing listens on the collector, which only needs to be accepted by Envoy.
    async with PyvoyServer(
        "tests.apps.asgi.otel",
        stderr=None,
        env={
            "OTEL_EXPORTER_OTLP_ENDPOINT": "https://localhost:1",
            "OTEL_EXPORTER_OTLP_PROTOCOL": protocol,
            "OTEL_EXPORTER_OTLP_CERTIFICATE": ca_cert,
            "OTEL_EXPORTER_OTLP_TIMEOUT": "2.5",
        },
    ) as server:
        response = await client.get(f"http://localhost:{server.listener_port}/")
        assert response.status == 200, response.text()
        assert json.loads(response.content)["traceparent"] is not None


def _config(env: dict[str, str], **kwargs: Any) -> dict:
    return PyvoyServer("tests.apps.asgi.otel", env=env, **kwargs).get_envoy_config()


def _hcm(config: dict) -> dict:
    return config["static_resources"]["listeners"][0]["filter_chains"][0]["filters"][0][
        "typed_config"
    ]


def _pyvoy_filter_config(config: dict) -> dict:
    return json.loads(
        _hcm(config)["http_filters"][-1]["typed_config"]["filter_config"]["value"]
    )


def _clusters(config: dict) -> dict[str, dict]:
    return {
        cluster["name"]: cluster
        for cluster in config["static_resources"].get("clusters", [])
    }


def test_not_configured() -> None:
    config = _config({})
    assert "tracing" not in _hcm(config)
    assert "stats_sinks" not in config
    assert "trace_sampler" not in _pyvoy_filter_config(config)
    assert not _clusters(config)


@pytest.mark.parametrize(
    ("env", "enable_otel"),
    [
        pytest.param({}, False, id="enable_otel"),
        pytest.param({"OTEL_SDK_DISABLED": "true"}, True, id="sdk_disabled"),
    ],
)
def test_disabled(env: dict[str, str], enable_otel: bool) -> None:
    config = _config(
        {"OTEL_EXPORTER_OTLP_ENDPOINT": "http://collector:4317", **env},
        enable_otel=enable_otel,
    )
    assert "tracing" not in _hcm(config)
    assert "stats_sinks" not in config
    assert not _clusters(config)


def test_environment_read_from_process(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4317")
    config = _config({})
    assert "tracing" in _hcm(config)
    assert "stats_sinks" in config


def test_grpc_default(ca_cert: str) -> None:
    config = _config(
        {
            "OTEL_EXPORTER_OTLP_ENDPOINT": "https://collector.example:4317",
            "OTEL_EXPORTER_OTLP_CERTIFICATE": ca_cert,
            "OTEL_EXPORTER_OTLP_HEADERS": "Authorization=Bearer%20token,x-team=pyvoy",
            "OTEL_SERVICE_NAME": "orders",
            "OTEL_RESOURCE_ATTRIBUTES": "service.name=ignored,deployment.environment=prod%2C1",
        }
    )
    tracer = _hcm(config)["tracing"]["provider"]["typed_config"]
    assert tracer["grpc_service"] == {
        "envoy_grpc": {
            "cluster_name": "pyvoy_otel_traces",
            "authority": "collector.example:4317",
        },
        "timeout": "10s",
        "initial_metadata": [
            {"key": "authorization", "value": "Bearer token"},
            {"key": "x-team", "value": "pyvoy"},
        ],
    }
    assert tracer["service_name"] == "orders"
    assert tracer["resource_detectors"][0]["typed_config"]["attributes"] == {
        "service.name": "orders",
        "deployment.environment": "prod,1",
    }
    sink = config["stats_sinks"][0]["typed_config"]
    assert sink["grpc_service"]["envoy_grpc"]["cluster_name"] == "pyvoy_otel_metrics"
    assert not sink["report_counters_as_deltas"]
    assert config["stats_flush_interval"] == "60.000s"

    clusters = _clusters(config)
    for name in ("pyvoy_otel_traces", "pyvoy_otel_metrics"):
        cluster = clusters[name]
        assert cluster["typed_extension_protocol_options"][
            "envoy.extensions.upstreams.http.v3.HttpProtocolOptions"
        ]["explicit_http_config"] == {"http2_protocol_options": {}}
        tls = cluster["transport_socket"]["typed_config"]
        assert tls["sni"] == "collector.example"
        assert tls["auto_sni_san_validation"]
        assert tls["common_tls_context"]["validation_context"]["trusted_ca"] == {
            "filename": ca_cert
        }
    assert _pyvoy_filter_config(config)["trace_sampler"] == {"parent_based": True}


def test_tls_without_ca() -> None:
    config = _config({"OTEL_EXPORTER_OTLP_ENDPOINT": "https://collector:4317"})
    tls = _clusters(config)["pyvoy_otel_traces"]["transport_socket"]["typed_config"]
    # Like upstreams, the collector is not verified without a CA certificate.
    assert "validation_context" not in tls["common_tls_context"]
    assert not tls["auto_sni_san_validation"]


def test_grpc_endpoint_without_scheme() -> None:
    config = _config(
        {
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": "collector:4317",
            "OTEL_EXPORTER_OTLP_TRACES_INSECURE": "true",
        }
    )
    assert "transport_socket" not in _clusters(config)["pyvoy_otel_traces"]
    # Only traces have an endpoint configured.
    assert "stats_sinks" not in config


def test_http_endpoints() -> None:
    config = _config(
        {
            "OTEL_EXPORTER_OTLP_PROTOCOL": "http/protobuf",
            "OTEL_EXPORTER_OTLP_ENDPOINT": "http://collector:4318/otlp/",
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": "http://metrics:9090/ingest",
            "OTEL_EXPORTER_OTLP_TRACES_HEADERS": "api-key=100%25",
        }
    )
    tracer = _hcm(config)["tracing"]["provider"]["typed_config"]
    assert tracer["http_service"] == {
        "http_uri": {
            "uri": "http://collector:4318/otlp/v1/traces",
            "cluster": "pyvoy_otel_traces",
            "timeout": "10s",
        },
        # The tracer's headers are substitution formatters.
        "request_headers_to_add": [{"header": {"key": "api-key", "value": "100%%"}}],
    }
    sink = config["stats_sinks"][0]["typed_config"]
    assert sink["http_service"]["http_uri"]["uri"] == "http://metrics:9090/ingest"
    clusters = _clusters(config)
    assert clusters["pyvoy_otel_metrics"]["load_assignment"]["endpoints"][0][
        "lb_endpoints"
    ][0]["endpoint"]["address"]["socket_address"] == {
        "address": "metrics",
        "port_value": 9090,
    }
    assert clusters["pyvoy_otel_traces"]["typed_extension_protocol_options"][
        "envoy.extensions.upstreams.http.v3.HttpProtocolOptions"
    ]["explicit_http_config"] == {"http_protocol_options": {}}


@pytest.mark.parametrize(
    ("env", "traces_timeout", "metrics_timeout"),
    [
        ({}, "10s", "10s"),
        ({"OTEL_EXPORTER_OTLP_TIMEOUT": "2.5"}, "2.500s", "2.500s"),
        (
            {
                "OTEL_EXPORTER_OTLP_TIMEOUT": "30",
                "OTEL_EXPORTER_OTLP_METRICS_TIMEOUT": "0.25",
            },
            "30.000s",
            "0.250s",
        ),
    ],
)
def test_timeout(
    env: dict[str, str], traces_timeout: str, metrics_timeout: str
) -> None:
    config = _config(
        {
            "OTEL_EXPORTER_OTLP_ENDPOINT": "http://collector:4317",
            "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "http/protobuf",
            **env,
        }
    )
    tracer = _hcm(config)["tracing"]["provider"]["typed_config"]
    assert tracer["grpc_service"]["timeout"] == traces_timeout
    sink = config["stats_sinks"][0]["typed_config"]
    assert sink["http_service"]["http_uri"]["timeout"] == metrics_timeout


def test_exporter_names() -> None:
    config = _config(
        {
            "OTEL_EXPORTER_OTLP_PROTOCOL": "grpc",
            "OTEL_TRACES_EXPORTER": "console,otlp_proto_http",
            "OTEL_METRICS_EXPORTER": "none",
        }
    )
    tracer = _hcm(config)["tracing"]["provider"]["typed_config"]
    assert (
        tracer["http_service"]["http_uri"]["uri"] == "http://localhost:4318/v1/traces"
    )
    assert "stats_sinks" not in config


@pytest.mark.parametrize(
    ("sampler", "arg", "envoy_sampler", "trace_sampler"),
    [
        (
            "always_on",
            None,
            {"name": "envoy.tracers.opentelemetry.samplers.always_on"},
            {"parent_based": False},
        ),
        (
            "always_off",
            None,
            {
                "name": "envoy.tracers.opentelemetry.samplers.trace_id_ratio_based",
                "sampling_percentage": {"numerator": 0, "denominator": "MILLION"},
            },
            {"parent_based": False, "ratio": 0},
        ),
        (
            "parentbased_traceidratio",
            "0.25",
            {
                "name": "envoy.tracers.opentelemetry.samplers.parent_based",
                "wrapped_sampler": {
                    "name": "envoy.tracers.opentelemetry.samplers.trace_id_ratio_based",
                    "sampling_percentage": {
                        "numerator": 250_000,
                        "denominator": "MILLION",
                    },
                },
            },
            {"parent_based": True, "ratio": 250_000},
        ),
    ],
)
def test_samplers(
    sampler: str,
    arg: str | None,
    envoy_sampler: dict[str, Any],
    trace_sampler: dict[str, Any],
) -> None:
    env = {"OTEL_EXPORTER_OTLP_ENDPOINT": "http://collector:4317"}
    env["OTEL_TRACES_SAMPLER"] = sampler
    if arg is not None:
        env["OTEL_TRACES_SAMPLER_ARG"] = arg
    config = _config(env)
    actual = _hcm(config)["tracing"]["provider"]["typed_config"]["sampler"]

    def simplify(sampler_config: dict) -> dict:
        simplified = {"name": sampler_config["name"]}
        for key, value in sampler_config["typed_config"].items():
            if key == "wrapped_sampler":
                simplified[key] = simplify(value)
            elif key != "@type":
                simplified[key] = value
        return simplified

    assert simplify(actual) == envoy_sampler
    assert _pyvoy_filter_config(config)["trace_sampler"] == trace_sampler


def test_metrics_settings() -> None:
    config = _config(
        {
            "OTEL_EXPORTER_OTLP_ENDPOINT": "http://collector:4317",
            "OTEL_METRIC_EXPORT_INTERVAL": "1500",
            "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE": "LowMemory",
        }
    )
    sink = config["stats_sinks"][0]["typed_config"]
    assert sink["report_counters_as_deltas"]
    assert sink["report_histograms_as_deltas"]
    assert sink["prefix"] == "envoy"
    assert config["stats_flush_interval"] == "1.500s"


@pytest.mark.parametrize(
    ("env", "message"),
    [
        (
            {"OTEL_TRACES_SAMPLER": "xray"},
            "OTEL_TRACES_SAMPLER 'xray' is not supported by Envoy",
        ),
        (
            {"OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "http/json"},
            "OTLP protocol 'http/json' is not supported by Envoy",
        ),
    ],
)
def test_unsupported_tracing(env: dict[str, str], message: str) -> None:
    with pytest.warns(UserWarning, match=message):
        config = _config(
            {"OTEL_EXPORTER_OTLP_ENDPOINT": "http://collector:4317", **env}
        )
    # The application's own trace context is left unchanged.
    assert "tracing" not in _hcm(config)
    assert "trace_sampler" not in _pyvoy_filter_config(config)
    assert "stats_sinks" in config


@pytest.mark.parametrize(
    ("env", "message"),
    [
        (
            {"OTEL_TRACES_SAMPLER": "traceidratio", "OTEL_TRACES_SAMPLER_ARG": "2"},
            "OTEL_TRACES_SAMPLER_ARG must be a number between 0 and 1",
        ),
        (
            {"OTEL_METRIC_EXPORT_INTERVAL": "300000"},
            "OTEL_METRIC_EXPORT_INTERVAL must be between 1 and 299999",
        ),
        (
            {"OTEL_METRIC_EXPORT_INTERVAL": "soon"},
            "OTEL_METRIC_EXPORT_INTERVAL must be an integer",
        ),
        (
            {"OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE": "sometimes"},
            "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE must be one of",
        ),
        (
            {"OTEL_RESOURCE_ATTRIBUTES": "env"},
            "OTEL_RESOURCE_ATTRIBUTES must be comma-separated key=value pairs",
        ),
        (
            {"OTEL_EXPORTER_OTLP_HEADERS": "token"},
            "OTLP traces headers must be comma-separated key=value pairs",
        ),
        (
            {"OTEL_EXPORTER_OTLP_TIMEOUT": "0"},
            "OTLP traces timeout must be a positive number of seconds",
        ),
        (
            {"OTEL_EXPORTER_OTLP_TIMEOUT": "inf"},
            "OTLP traces timeout must be a positive number of seconds",
        ),
        (
            {"OTEL_EXPORTER_OTLP_ENDPOINT": "ftp://collector"},
            "OTLP traces endpoint must be an http or https URL",
        ),
    ],
)
def test_invalid(env: dict[str, str], message: str) -> None:
    with pytest.raises(ValueError, match=message):
        _config({"OTEL_EXPORTER_OTLP_ENDPOINT": "http://collector:4317", **env})
