"""WebSockets over HTTP/2 extended CONNECT (RFC 8441)."""

from __future__ import annotations

import asyncio
import os
import socket
import struct
import subprocess
import time
from typing import TYPE_CHECKING

import h2.config
import h2.connection
import h2.errors
import h2.events
import pytest
import pytest_asyncio

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyvoy import Loop

OPCODE_TEXT = 0x1
OPCODE_BINARY = 0x2
OPCODE_CLOSE = 0x8
OPCODE_PING = 0x9


@pytest_asyncio.fixture(scope="module")
async def server(loop: Loop | None) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.websockets.kitchensink",
        loop=loop,
        websockets=True,
        lifespan=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    ) as server:
        yield server


def _client_frame(opcode: int, payload: bytes) -> bytes:
    length = len(payload)
    if length < 126:
        header = bytes([0x80 | opcode, 0x80 | length])
    elif length < 1 << 16:
        header = bytes([0x80 | opcode, 0x80 | 126]) + struct.pack("!H", length)
    else:
        header = bytes([0x80 | opcode, 0x80 | 127]) + struct.pack("!Q", length)
    mask = os.urandom(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return header + mask + masked


def _text(value: str | bytes) -> str:
    return value.decode() if isinstance(value, bytes) else value


def _close_payload(code: int) -> bytes:
    return struct.pack("!H", code)


class _Http2WebSocket:
    """A minimal blocking WebSocket client over HTTP/2 prior knowledge, with
    the default 64 KiB stream flow control window.
    """

    def __init__(self, port: int, path: str) -> None:
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=10)
        self.conn = h2.connection.H2Connection(
            h2.config.H2Configuration(client_side=True, header_encoding="utf-8")
        )
        self.stream_id: int | None = None
        self.response_headers: dict[str, str] | None = None
        self.settings_received = False
        self.ended = False
        self.end_stream_received = False
        self.reset_code: int | None = None
        self.buffer = bytearray()
        self.conn.initiate_connection()
        self._flush()
        while not self.settings_received:
            self._pump()
        assert self.conn.remote_settings.enable_connect_protocol == 1
        self.stream_id = self.conn.get_next_available_stream_id()
        self.conn.send_headers(
            self.stream_id,
            [
                (":method", "CONNECT"),
                (":protocol", "websocket"),
                (":scheme", "http"),
                (":path", path),
                (":authority", f"127.0.0.1:{port}"),
                ("sec-websocket-version", "13"),
            ],
        )
        self._flush()
        while self.response_headers is None and not self.ended:
            self._pump()

    def close(self) -> None:
        self.sock.close()

    def _flush(self) -> None:
        if data := self.conn.data_to_send():
            self.sock.sendall(data)

    def _pump(self) -> None:
        data = self.sock.recv(65536)
        if not data:
            msg = "connection closed"
            raise EOFError(msg)
        for event in self.conn.receive_data(data):
            if isinstance(event, h2.events.RemoteSettingsChanged):
                self.settings_received = True
            if getattr(event, "stream_id", None) != self.stream_id:
                continue
            if isinstance(event, h2.events.ResponseReceived):
                self.response_headers = {
                    _text(name): _text(value) for name, value in event.headers or ()
                }
            elif isinstance(event, h2.events.DataReceived):
                self.conn.acknowledge_received_data(
                    event.flow_controlled_length, event.stream_id
                )
                self.buffer += event.data
            elif isinstance(event, h2.events.StreamEnded):
                self.end_stream_received = True
                self.ended = True
            elif isinstance(event, h2.events.StreamReset):
                self.reset_code = event.error_code
                self.ended = True
        self._flush()

    def send(self, data: bytes) -> None:
        assert self.stream_id is not None
        while data:
            window = min(
                self.conn.local_flow_control_window(self.stream_id),
                self.conn.max_outbound_frame_size,
            )
            if window == 0:
                self._pump()
                continue
            self.conn.send_data(self.stream_id, data[:window])
            self._flush()
            data = data[window:]

    def end_stream(self) -> None:
        assert self.stream_id is not None
        self.conn.end_stream(self.stream_id)
        self._flush()

    def recv_frame(self) -> tuple[int, bytes]:
        while (frame := self._parse_frame()) is None:
            if self.ended:
                msg = "stream ended"
                raise EOFError(msg)
            self._pump()
        return frame

    def _parse_frame(self) -> tuple[int, bytes] | None:
        if len(self.buffer) < 2:
            return None
        opcode = self.buffer[0] & 0x0F
        length = self.buffer[1] & 0x7F
        offset = 2
        if length == 126:
            if len(self.buffer) < 4:
                return None
            (length,) = struct.unpack_from("!H", self.buffer, 2)
            offset = 4
        elif length == 127:
            if len(self.buffer) < 10:
                return None
            (length,) = struct.unpack_from("!Q", self.buffer, 2)
            offset = 10
        if len(self.buffer) < offset + length:
            return None
        payload = bytes(self.buffer[offset : offset + length])
        del self.buffer[: offset + length]
        return opcode, payload

    def wait_ended(self) -> None:
        while not self.ended:
            self._pump()


def _echo_and_close(port: int) -> None:
    ws = _Http2WebSocket(port, "/echo")
    try:
        assert ws.response_headers is not None
        assert ws.response_headers[":status"] == "200"
        assert "sec-websocket-accept" not in ws.response_headers
        ws.send(_client_frame(OPCODE_TEXT, b"hello"))
        assert ws.recv_frame() == (OPCODE_TEXT, b"hello")
        ws.send(_client_frame(OPCODE_CLOSE, _close_payload(1000)))
        assert ws.recv_frame() == (OPCODE_CLOSE, _close_payload(1000))
        ws.end_stream()
        ws.wait_ended()
        assert ws.reset_code is None
    finally:
        ws.close()


@pytest.mark.asyncio
async def test_echo_and_close(server: PyvoyServer) -> None:
    await asyncio.to_thread(_echo_and_close, server.listener_port)


def _http_version(port: int) -> bytes:
    ws = _Http2WebSocket(port, "/http-version")
    try:
        return ws.recv_frame()[1]
    finally:
        ws.close()


@pytest.mark.asyncio
async def test_http_version(server: PyvoyServer) -> None:
    assert await asyncio.to_thread(_http_version, server.listener_port) == b"2"


def _close_with_echo_in_flight(port: int) -> None:
    ws = _Http2WebSocket(port, "/echo")
    try:
        message = os.urandom(1 << 20)
        # The echo far exceeds the client's flow control window, so the server
        # finishes the close handshake while the echo is still being sent.
        ws.send(
            _client_frame(OPCODE_BINARY, message)
            + _client_frame(OPCODE_CLOSE, _close_payload(1000))
        )
        assert ws.recv_frame() == (OPCODE_BINARY, message)
        assert ws.recv_frame() == (OPCODE_CLOSE, _close_payload(1000))
        ws.end_stream()
        ws.wait_ended()
        assert ws.reset_code is None
    finally:
        ws.close()


@pytest.mark.asyncio
async def test_close_with_echo_in_flight(server: PyvoyServer) -> None:
    await asyncio.to_thread(_close_with_echo_in_flight, server.listener_port)


def _server_close(port: int) -> None:
    ws = _Http2WebSocket(port, "/close")
    try:
        opcode, payload = ws.recv_frame()
        assert opcode == OPCODE_CLOSE
        assert payload[:2] == _close_payload(1001)
        ws.send(_client_frame(OPCODE_CLOSE, _close_payload(1001)))
        ws.end_stream()
        ws.wait_ended()
        assert ws.reset_code is None
    finally:
        ws.close()


@pytest.mark.asyncio
async def test_server_close(server: PyvoyServer) -> None:
    await asyncio.to_thread(_server_close, server.listener_port)


def _close_without_half_close(port: int) -> float:
    ws = _Http2WebSocket(port, "/echo")
    try:
        ws.send(_client_frame(OPCODE_CLOSE, _close_payload(1000)))
        assert ws.recv_frame() == (OPCODE_CLOSE, _close_payload(1000))
        start = time.monotonic()
        ws.wait_ended()
        assert ws.end_stream_received
        # The response completes before the request, which HTTP/2 follows with
        # a reset without an error.
        assert ws.reset_code in (None, h2.errors.ErrorCodes.NO_ERROR)
        return time.monotonic() - start
    finally:
        ws.close()


@pytest.mark.asyncio
async def test_close_without_half_close(server: PyvoyServer) -> None:
    # The server ends its stream after a timeout when the client never does.
    elapsed = await asyncio.to_thread(_close_without_half_close, server.listener_port)
    assert 3 < elapsed < 10


def _end_without_close(port: int) -> None:
    ws = _Http2WebSocket(port, "/echo")
    try:
        ws.send(_client_frame(OPCODE_TEXT, b"hello"))
        assert ws.recv_frame() == (OPCODE_TEXT, b"hello")
        ws.end_stream()
        ws.wait_ended()
        assert ws.reset_code is None
    finally:
        ws.close()


@pytest.mark.asyncio
async def test_end_without_close(server: PyvoyServer) -> None:
    await asyncio.to_thread(_end_without_close, server.listener_port)


def _ping_timeout(port: int) -> tuple[list[tuple[int, bytes]], int | None]:
    ws = _Http2WebSocket(port, "/echo")
    frames: list[tuple[int, bytes]] = []
    try:
        while True:
            try:
                frames.append(ws.recv_frame())
            except EOFError:
                return frames, ws.reset_code
    finally:
        ws.close()


@pytest.mark.asyncio
async def test_ping_timeout(loop: Loop | None) -> None:
    async with PyvoyServer(
        "tests.apps.websockets.kitchensink",
        loop=loop,
        websockets=True,
        lifespan=False,
        websockets_ping_interval=0.2,
        request_idle_timeout=0.5,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    ) as srv:
        # Our client's WINDOW_UPDATEs aren't data on the stream, so they don't
        # answer pings.
        frames, reset_code = await asyncio.to_thread(_ping_timeout, srv.listener_port)
    assert len(frames) >= 2
    assert set(frames[:-1]) == {(OPCODE_PING, b"")}
    assert frames[-1] == (
        OPCODE_CLOSE,
        _close_payload(1011) + b"keepalive ping timeout",
    )
    # Ending the response before the request resets the stream.
    assert reset_code is not None
