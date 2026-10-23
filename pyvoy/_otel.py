"""Translation of OpenTelemetry SDK environment variables into Envoy config."""

from __future__ import annotations

import math
import sys
import warnings
from dataclasses import dataclass
from typing import TYPE_CHECKING, Literal
from urllib.parse import unquote, urlsplit

if TYPE_CHECKING:
    from collections.abc import Mapping

Signal = Literal["traces", "metrics"]

# The denominator of trace ID ratios. pyvoy's filter recomputes Envoy's sampling
# decision with the same arithmetic, so it is fixed for both.
_RATIO_DENOMINATOR = 1_000_000

# Matches the OpenTelemetry Python SDK so Envoy's telemetry is grouped with the
# application's when no service name is configured.
_DEFAULT_SERVICE_NAME = "unknown_service"

# Envoy rejects a stats flush interval of 300 seconds or more.
_MAX_METRIC_EXPORT_INTERVAL_MS = 299_999


# pyvoy queries the admin listener itself, so its stats would add requests the
# application did not serve to the HTTP stats.
_DROP_ADMIN_STATS = {
    "matcher_tree": {
        "input": {
            "name": "stat_full_name_match_input",
            "typed_config": {
                "@type": "type.googleapis.com/envoy.extensions.matching.common_inputs.stats.v3.StatFullNameMatchInput"
            },
        },
        "prefix_match_map": {
            "map": {
                prefix: {
                    "action": {
                        "name": "otlp_metric_drop_action",
                        "typed_config": {
                            "@type": "type.googleapis.com/envoy.extensions.stat_sinks.open_telemetry.v3.SinkConfig.DropAction"
                        },
                    }
                }
                for prefix in ("http.admin.", "listener.admin.")
            }
        },
    }
}


@dataclass(frozen=True, slots=True)
class OTLPCollector:
    """An upstream cluster to export OTLP to."""

    name: str
    address: str
    grpc: bool
    tls: bool
    ca_cert: str | None
    client_cert: str | None
    client_key: str | None


@dataclass(frozen=True, slots=True)
class OTelConfig:
    """Envoy config for exporting telemetry."""

    tracing: dict | None
    """The HTTP connection manager's tracing config."""

    trace_sampler: dict | None
    """The sampler for pyvoy's filter to propagate Envoy's span with."""

    stats_sink: dict | None
    """The bootstrap stats sink."""

    stats_flush_interval: str | None
    """The bootstrap stats flush interval."""

    collectors: list[OTLPCollector]
    """The clusters the exporters reference."""


@dataclass(frozen=True, slots=True)
class _Exporter:
    url: str
    grpc: bool
    authority: str
    headers: list[tuple[str, str]]
    timeout: str
    collector: OTLPCollector


def otel_config(env: Mapping[str, str]) -> OTelConfig:
    """Returns the Envoy config for the OpenTelemetry variables in env.

    Raises:
        ValueError: If a variable has an invalid value.
    """
    collectors: list[OTLPCollector] = []
    tracing: dict | None = None
    trace_sampler = None
    stats_sink = None
    stats_flush_interval = None
    if _get(env, "OTEL_SDK_DISABLED").lower() == "true":
        return OTelConfig(None, None, None, None, collectors)

    resource_attributes = _resource_attributes(env)
    service_name = resource_attributes["service.name"]
    resource_detectors = [
        {
            "name": "envoy.tracers.opentelemetry.resource_detectors.static_config",
            "typed_config": {
                "@type": "type.googleapis.com/envoy.extensions.tracers.opentelemetry.resource_detectors.v3.StaticConfigResourceDetectorConfig",
                "attributes": resource_attributes,
            },
        }
    ]

    if (exporter := _exporter(env, "traces")) is not None and (
        sampler := _sampler(env)
    ) is not None:
        envoy_sampler, trace_sampler = sampler
        collectors.append(exporter.collector)
        tracing = {
            "provider": {
                "name": "envoy.tracers.opentelemetry",
                "typed_config": {
                    "@type": "type.googleapis.com/envoy.config.trace.v3.OpenTelemetryConfig",
                    # The HTTP exporter's headers support substitution formatters.
                    **_exporter_service(exporter, escape_percent=True),
                    "service_name": service_name,
                    "resource_detectors": resource_detectors,
                    "sampler": envoy_sampler,
                },
            }
        }
        # Envoy on Windows is built without CEL, which makes every substitution
        # formatter fail to parse.
        if sys.platform != "win32":
            # Follow OTel semconv for HTTP server spans with no known route by using HTTP method.
            tracing["operation"] = "%REQ(:METHOD)%"

    if (exporter := _exporter(env, "metrics")) is not None:
        delta = _metrics_delta_temporality(env)
        collectors.append(exporter.collector)
        stats_sink = {
            "name": "envoy.stat_sinks.open_telemetry",
            "typed_config": {
                "@type": "type.googleapis.com/envoy.extensions.stat_sinks.open_telemetry.v3.SinkConfig",
                **_exporter_service(exporter, escape_percent=False),
                "resource_detectors": resource_detectors,
                "prefix": "envoy",
                "custom_metric_conversions": _DROP_ADMIN_STATS,
                "report_counters_as_deltas": delta,
                "report_histograms_as_deltas": delta,
            },
        }
        interval_ms = _int(env, "OTEL_METRIC_EXPORT_INTERVAL", 60_000)
        if not 1 <= interval_ms <= _MAX_METRIC_EXPORT_INTERVAL_MS:
            msg = f"OTEL_METRIC_EXPORT_INTERVAL must be between 1 and {_MAX_METRIC_EXPORT_INTERVAL_MS} milliseconds for Envoy, got {interval_ms}"
            raise ValueError(msg)
        stats_flush_interval = f"{interval_ms // 1000}.{interval_ms % 1000:03d}s"

    return OTelConfig(
        tracing, trace_sampler, stats_sink, stats_flush_interval, collectors
    )


def _get(env: Mapping[str, str], name: str) -> str:
    return env.get(name, "").strip()


def _get_signal(env: Mapping[str, str], signal: Signal, suffix: str) -> str:
    """Returns the signal-specific OTLP exporter variable, falling back to the general one."""
    return _get(env, f"OTEL_EXPORTER_OTLP_{signal.upper()}_{suffix}") or _get(
        env, f"OTEL_EXPORTER_OTLP_{suffix}"
    )


def _int(env: Mapping[str, str], name: str, default: int) -> int:
    value = _get(env, name)
    if not value:
        return default
    try:
        return int(value)
    except ValueError:
        msg = f"{name} must be an integer, got '{value}'"
        raise ValueError(msg) from None


def _resource_attributes(env: Mapping[str, str]) -> dict[str, str]:
    attributes: dict[str, str] = {}
    for item in _get(env, "OTEL_RESOURCE_ATTRIBUTES").split(","):
        if not item.strip():
            continue
        key, sep, value = item.partition("=")
        if not sep or not key.strip():
            msg = f"OTEL_RESOURCE_ATTRIBUTES must be comma-separated key=value pairs, got '{item}'"
            raise ValueError(msg)
        attributes[key.strip()] = unquote(value.strip())
    if service_name := _get(env, "OTEL_SERVICE_NAME"):
        attributes["service.name"] = service_name
    attributes.setdefault("service.name", _DEFAULT_SERVICE_NAME)
    return attributes


def _exporter(env: Mapping[str, str], signal: Signal) -> _Exporter | None:
    """Returns the OTLP exporter for the signal, or None if it is not exported.

    A signal is exported when its exporter variable includes an OTLP exporter or,
    when the exporter variable is unset, an OTLP endpoint is configured. This
    avoids exporting to the default endpoint when nothing is configured.
    """
    upper = signal.upper()
    protocol = _get_signal(env, signal, "PROTOCOL")
    signal_endpoint = _get(env, f"OTEL_EXPORTER_OTLP_{upper}_ENDPOINT")
    endpoint = _get(env, "OTEL_EXPORTER_OTLP_ENDPOINT")
    if names := _get(env, f"OTEL_{upper}_EXPORTER"):
        exporters = {name.strip() for name in names.split(",")}
        # Like the OpenTelemetry Python SDK, the protocol-specific exporter names
        # take precedence over the protocol variable.
        if "otlp_proto_grpc" in exporters:
            protocol = "grpc"
        elif "otlp_proto_http" in exporters:
            protocol = "http/protobuf"
        elif "otlp" not in exporters:
            return None
    elif not signal_endpoint and not endpoint:
        return None

    # The OpenTelemetry Python SDK defaults to gRPC rather than the
    # specification's HTTP, and the application shares these variables.
    protocol = protocol or "grpc"
    if protocol not in ("grpc", "http/protobuf"):
        warnings.warn(
            f"OTLP protocol '{protocol}' is not supported by Envoy, Envoy {signal} will not be exported",
            stacklevel=4,
        )
        return None
    grpc = protocol == "grpc"

    if grpc:
        url = signal_endpoint or endpoint or "http://localhost:4317"
        if "://" not in url:
            # gRPC endpoints may omit the scheme, using the insecure variable instead.
            insecure = _get_signal(env, signal, "INSECURE").lower() == "true"
            url = f"{'http' if insecure else 'https'}://{url}"
    elif signal_endpoint:
        url = signal_endpoint
    elif endpoint:
        url = f"{endpoint.rstrip('/')}/v1/{signal}"
    else:
        url = f"http://localhost:4318/v1/{signal}"

    parsed = urlsplit(url)
    if parsed.scheme not in ("http", "https") or not parsed.hostname:
        msg = f"OTLP {signal} endpoint must be an http or https URL, got '{url}'"
        raise ValueError(msg)
    tls = parsed.scheme == "https"
    try:
        port = parsed.port or (443 if tls else 80)
    except ValueError:
        msg = f"OTLP {signal} endpoint has an invalid port, got '{url}'"
        raise ValueError(msg) from None
    host = f"[{parsed.hostname}]" if ":" in parsed.hostname else parsed.hostname

    return _Exporter(
        url=url,
        grpc=grpc,
        authority=parsed.netloc.rpartition("@")[2],
        headers=_headers(env, signal),
        timeout=_timeout(env, signal),
        collector=OTLPCollector(
            name=f"pyvoy_otel_{signal}",
            address=f"{host}:{port}",
            grpc=grpc,
            tls=tls,
            ca_cert=(_get_signal(env, signal, "CERTIFICATE") or None) if tls else None,
            client_cert=(_get_signal(env, signal, "CLIENT_CERTIFICATE") or None)
            if tls
            else None,
            client_key=(_get_signal(env, signal, "CLIENT_KEY") or None)
            if tls
            else None,
        ),
    )


def _headers(env: Mapping[str, str], signal: Signal) -> list[tuple[str, str]]:
    headers: list[tuple[str, str]] = []
    for item in _get_signal(env, signal, "HEADERS").split(","):
        if not item.strip():
            continue
        key, sep, value = item.partition("=")
        if not sep or not key.strip():
            msg = f"OTLP {signal} headers must be comma-separated key=value pairs, got '{item}'"
            raise ValueError(msg)
        headers.append((key.strip().lower(), unquote(value.strip())))
    return headers


def _timeout(env: Mapping[str, str], signal: Signal) -> str:
    # Seconds, like the OpenTelemetry Python SDK, rather than the
    # specification's milliseconds, since the application shares these variables.
    value = _get_signal(env, signal, "TIMEOUT")
    if not value:
        return "10s"
    try:
        seconds = float(value)
    except ValueError:
        seconds = -1.0
    if not 0 < seconds < math.inf:
        msg = (
            f"OTLP {signal} timeout must be a positive number of seconds, got '{value}'"
        )
        raise ValueError(msg)
    return f"{seconds:.3f}s"


def _exporter_service(exporter: _Exporter, *, escape_percent: bool) -> dict:
    if exporter.grpc:
        grpc_service: dict = {
            "envoy_grpc": {
                "cluster_name": exporter.collector.name,
                "authority": exporter.authority,
            },
            "timeout": exporter.timeout,
        }
        if exporter.headers:
            grpc_service["initial_metadata"] = [
                {"key": key, "value": value} for key, value in exporter.headers
            ]
        return {"grpc_service": grpc_service}
    http_service: dict = {
        "http_uri": {
            "uri": exporter.url,
            "cluster": exporter.collector.name,
            "timeout": exporter.timeout,
        }
    }
    if exporter.headers:
        http_service["request_headers_to_add"] = [
            {
                "header": {
                    "key": key,
                    "value": value.replace("%", "%%") if escape_percent else value,
                }
            }
            for key, value in exporter.headers
        ]
    return {"http_service": http_service}


def _sampler(env: Mapping[str, str]) -> tuple[dict, dict] | None:
    """Returns the Envoy sampler and the matching pyvoy filter sampler.

    Returns None, so that the application's own trace context is left unchanged,
    if Envoy does not support the sampler.
    """
    name = _get(env, "OTEL_TRACES_SAMPLER") or "parentbased_always_on"
    parent_based = name.startswith("parentbased_")
    match name.removeprefix("parentbased_"):
        case "always_on":
            ratio = None
        case "always_off":
            ratio = 0
        case "traceidratio":
            ratio = _ratio(env)
        case _:
            warnings.warn(
                f"OTEL_TRACES_SAMPLER '{name}' is not supported by Envoy, Envoy traces will not be exported",
                stacklevel=4,
            )
            return None

    if ratio is None:
        envoy_sampler = {
            "name": "envoy.tracers.opentelemetry.samplers.always_on",
            "typed_config": {
                "@type": "type.googleapis.com/envoy.extensions.tracers.opentelemetry.samplers.v3.AlwaysOnSamplerConfig"
            },
        }
    else:
        envoy_sampler = {
            "name": "envoy.tracers.opentelemetry.samplers.trace_id_ratio_based",
            "typed_config": {
                "@type": "type.googleapis.com/envoy.extensions.tracers.opentelemetry.samplers.v3.TraceIdRatioBasedSamplerConfig",
                "sampling_percentage": {"numerator": ratio, "denominator": "MILLION"},
            },
        }
    if parent_based:
        envoy_sampler = {
            "name": "envoy.tracers.opentelemetry.samplers.parent_based",
            "typed_config": {
                "@type": "type.googleapis.com/envoy.extensions.tracers.opentelemetry.samplers.v3.ParentBasedSamplerConfig",
                "wrapped_sampler": envoy_sampler,
            },
        }

    trace_sampler: dict = {"parent_based": parent_based}
    if ratio is not None:
        trace_sampler["ratio"] = ratio
    return envoy_sampler, trace_sampler


def _ratio(env: Mapping[str, str]) -> int:
    value = _get(env, "OTEL_TRACES_SAMPLER_ARG")
    if not value:
        return _RATIO_DENOMINATOR
    try:
        ratio = float(value)
    except ValueError:
        ratio = -1.0
    if not 0.0 <= ratio <= 1.0:
        msg = f"OTEL_TRACES_SAMPLER_ARG must be a number between 0 and 1, got '{value}'"
        raise ValueError(msg)
    return round(ratio * _RATIO_DENOMINATOR)


def _metrics_delta_temporality(env: Mapping[str, str]) -> bool:
    value = _get(env, "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE").lower()
    match value:
        case "" | "cumulative":
            return False
        # Envoy's counters and histograms are all synchronous, which low memory
        # temporality reports as deltas.
        case "delta" | "lowmemory":
            return True
        case _:
            msg = f"OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE must be one of 'cumulative', 'delta', or 'lowmemory', got '{value}'"
            raise ValueError(msg)
