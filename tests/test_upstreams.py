from __future__ import annotations

import pytest

from pyvoy import HTTPVersion, PyvoyServer, TLSConfig, Upstream


def test_http3_upstream_requires_tls() -> None:
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        upstreams=[
            Upstream(
                name="backend_h3",
                address="localhost:443",
                http_version=HTTPVersion.HTTP3,
            )
        ],
    )

    with pytest.raises(ValueError, match="HTTP/3 upstream 'backend_h3' requires TLS"):
        server.get_envoy_config()


def test_http3_upstream_uses_quic_transport() -> None:
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        upstreams=[
            Upstream(
                name="backend_h3",
                address="localhost:443",
                http_version=HTTPVersion.HTTP3,
                tls=TLSConfig(ca_cert=b"test CA"),
            )
        ],
    )

    config = server.get_envoy_config()
    cluster = config["static_resources"]["clusters"][0]
    protocol_options = cluster["typed_extension_protocol_options"][
        "envoy.extensions.upstreams.http.v3.HttpProtocolOptions"
    ]
    assert protocol_options["explicit_http_config"] == {"http3_protocol_options": {}}
    transport_socket = cluster["transport_socket"]
    assert transport_socket["name"] == "envoy.transport_sockets.quic"
    typed_config = transport_socket["typed_config"]
    assert typed_config["@type"].endswith(".QuicUpstreamTransport")
    assert typed_config["upstream_tls_context"]["sni"] == "localhost"
    assert typed_config["upstream_tls_context"]["auto_sni_san_validation"]


@pytest.mark.parametrize(
    ("tls", "expected"),
    [
        (None, {"explicit_http_config": {"http_protocol_options": {}}}),
        (TLSConfig(ca_cert=b"test CA"), {"auto_config": {}}),
    ],
)
def test_unspecified_http_version(
    tls: TLSConfig | None, expected: dict[str, object]
) -> None:
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        upstreams=[Upstream(name="backend", address="localhost:8080", tls=tls)],
    )

    config = server.get_envoy_config()
    cluster = config["static_resources"]["clusters"][0]
    protocol_options = cluster["typed_extension_protocol_options"][
        "envoy.extensions.upstreams.http.v3.HttpProtocolOptions"
    ]
    # Envoy refuses to start with automatic HTTP versions on plaintext.
    assert {k: v for k, v in protocol_options.items() if k != "@type"} == expected
