from __future__ import annotations

import asyncio
import gzip
import subprocess
import urllib.request
from typing import TYPE_CHECKING, Any

import pytest
import pytest_asyncio

from pyvoy import PyvoyServer, StaticMount

if TYPE_CHECKING:
    from collections.abc import AsyncIterator
    from pathlib import Path

PRECOMPRESSED_MARKER = b"served precompressed from disk, padded padded"


@pytest_asyncio.fixture(scope="module")
async def server() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        content_encodings=["zstd", "br", "gzip"],
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


async def _get(
    server: PyvoyServer, path: str, accept_encoding: str
) -> tuple[str | None, bytes]:
    """Fetches a path without decoding the response body.

    urllib does not negotiate content encodings on its own, so the raw
    Content-Encoding header and body are visible here.
    """
    url = f"http://{server.listener_address}:{server.listener_port}{path}"

    def fetch() -> tuple[str | None, bytes]:
        request = urllib.request.Request(
            url, headers={"Accept-Encoding": accept_encoding}
        )
        with urllib.request.urlopen(request) as response:  # noqa: S310
            return response.headers.get("content-encoding"), response.read()

    return await asyncio.to_thread(fetch)


@pytest.mark.parametrize(
    ("accept_encoding", "expected"),
    [
        ("gzip", "gzip"),
        ("br", "br"),
        ("zstd", "zstd"),
        # The most preferred encoding wins when the client does not prefer one.
        ("gzip, zstd", "zstd"),
        ("zstd, gzip", "zstd"),
        ("*", "zstd"),
        # An explicit client preference still wins.
        ("gzip;q=1.0, zstd;q=0.5", "gzip"),
        # Neither is configured, so the response is sent uncompressed.
        ("identity", None),
        ("deflate", None),
    ],
)
@pytest.mark.asyncio
async def test_content_encoding_negotiated(
    server: PyvoyServer, accept_encoding: str, expected: str | None
) -> None:
    encoding, _ = await _get(server, "/response-body", accept_encoding)

    assert encoding == expected


@pytest.mark.asyncio
async def test_compressed_body_round_trips(server: PyvoyServer) -> None:
    encoding, body = await _get(server, "/response-body", "gzip")

    assert encoding == "gzip"
    assert gzip.decompress(body) == b"Hello world!"


@pytest.mark.asyncio
async def test_compression_disabled_by_default() -> None:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink", stderr=subprocess.STDOUT, stdout=subprocess.PIPE
    ) as server:
        encoding, body = await _get(server, "/response-body", "gzip, br, zstd")

    assert encoding is None
    assert body == b"Hello world!"


@pytest_asyncio.fixture(scope="module")
async def echo_server() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.echo_accept_encoding",
        content_encodings=["gzip"],
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def precompressed_root(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """A static root whose gzip variant holds different content than the original.

    The marker makes it unambiguous whether a gzip response was read off disk or
    compressed on the fly by Envoy.
    """
    root = tmp_path_factory.mktemp("precompressed")
    (root / "index.html").write_bytes(b"<html>plain original, padded padded</html>")
    (root / "index.html.gz").write_bytes(gzip.compress(PRECOMPRESSED_MARKER))
    return root


@pytest_asyncio.fixture(scope="module")
async def app_and_static_server(precompressed_root: Path) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.echo_accept_encoding",
        content_encodings=["gzip"],
        static_mounts=[
            StaticMount(path="/static", root=precompressed_root, precompressed=["gzip"])
        ],
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest.mark.asyncio
async def test_application_does_not_see_accept_encoding(
    echo_server: PyvoyServer,
) -> None:
    encoding, body = await _get(echo_server, "/", "gzip")

    # The response is still compressed, just by Envoy rather than the application.
    assert encoding == "gzip"
    assert b"accept-encoding=<absent>" in gzip.decompress(body)


@pytest.mark.asyncio
async def test_application_sees_accept_encoding_without_compression() -> None:
    async with PyvoyServer(
        "tests.apps.asgi.echo_accept_encoding",
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        encoding, body = await _get(server, "/", "gzip")

    assert encoding is None
    assert b"accept-encoding=gzip" in body


@pytest.mark.asyncio
async def test_static_mount_still_serves_precompressed(
    app_and_static_server: PyvoyServer,
) -> None:
    encoding, body = await _get(app_and_static_server, "/static/index.html", "gzip")

    assert encoding == "gzip"
    assert gzip.decompress(body) == PRECOMPRESSED_MARKER


@pytest.mark.asyncio
async def test_application_mount_alongside_static_does_not_see_accept_encoding(
    app_and_static_server: PyvoyServer,
) -> None:
    encoding, body = await _get(app_and_static_server, "/", "gzip")

    assert encoding == "gzip"
    assert b"accept-encoding=<absent>" in gzip.decompress(body)


def test_accept_encoding_stripped_only_for_application_mounts(tmp_path: Path) -> None:
    config = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        content_encodings=["gzip"],
        static_mounts=[StaticMount(path="/static", root=tmp_path)],
    ).get_envoy_config()

    actions = _composite_actions(config)
    # The application mount runs the strip filter before its terminal filter.
    app_chain = actions["/"]["filter_chain"]["typed_config"]
    assert [f["name"] for f in app_chain] == [
        "envoy.filters.http.header_mutation",
        "pyvoy",
    ]
    assert app_chain[0]["typed_config"]["mutations"]["request_mutations"] == [
        {"remove": "accept-encoding"}
    ]
    # The static mount is left alone so precompressed variants keep working.
    assert "filter_chain" not in actions["/static"]
    assert actions["/static"]["typed_config"]["name"] == "envoy_files"


def test_accept_encoding_kept_without_compression(tmp_path: Path) -> None:
    config = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        static_mounts=[StaticMount(path="/static", root=tmp_path)],
    ).get_envoy_config()

    actions = _composite_actions(config)
    assert "filter_chain" not in actions["/"]
    assert actions["/"]["typed_config"]["name"] == "pyvoy"


def _composite_actions(config: dict[str, Any]) -> dict[str, dict[str, Any]]:
    filters = config["static_resources"]["listeners"][0]["filter_chains"][0]["filters"][
        0
    ]["typed_config"]["http_filters"]
    composite = next(f for f in filters if f["name"] == "envoy.filters.http.composite")
    matcher = composite["typed_config"]["xds_matcher"]["matcher_tree"][
        "prefix_match_map"
    ]["map"]
    return {
        prefix: entry["action"]["typed_config"] for prefix, entry in matcher.items()
    }


def test_compressor_filters_are_chained_in_preference_order() -> None:
    config = PyvoyServer(
        "tests.apps.asgi.kitchensink", content_encodings=["br", "gzip"]
    ).get_envoy_config()

    filters: list[dict[str, Any]] = config["static_resources"]["listeners"][0][
        "filter_chains"
    ][0]["filters"][0]["typed_config"]["http_filters"]
    assert [f["name"] for f in filters] == [
        "envoy.filters.http.compressor.br",
        "envoy.filters.http.compressor.gzip",
        # Without static mounts the strip filter is a plain chain entry.
        "envoy.filters.http.header_mutation",
        "pyvoy",
    ]
    assert filters[0]["typed_config"]["compressor_library"]["name"] == "brotli"
    # Only the most preferred encoding breaks ties in Accept-Encoding.
    assert filters[0]["typed_config"]["choose_first"] is True
    assert "choose_first" not in filters[1]["typed_config"]


@pytest.mark.parametrize("content_encodings", [["gzip", "deflate"], ["snappy"]])
def test_content_encodings_must_be_supported(content_encodings: list[str]) -> None:
    with pytest.raises(ValueError, match="content_encodings must each be one of"):
        PyvoyServer(
            "tests.apps.asgi.kitchensink",
            content_encodings=content_encodings,  # pyright: ignore[reportArgumentType]
        )


def test_content_encodings_must_not_be_duplicated() -> None:
    with pytest.raises(ValueError, match="content_encodings must not contain"):
        PyvoyServer(
            "tests.apps.asgi.kitchensink", content_encodings=["gzip", "br", "gzip"]
        )
