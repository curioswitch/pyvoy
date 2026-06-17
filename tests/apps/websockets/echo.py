from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, WebSocketScope


async def app(
    _scope: WebSocketScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    msg = await recv()
    if msg["type"] != "websocket.connect":
        msg = 'msg["type"] != "websocket.connect"'
        raise RuntimeError(msg)
    await send({"type": "websocket.accept", "headers": (), "subprotocol": None})
    while True:
        msg = await recv()
        if msg["type"] == "websocket.receive":
            await send(
                {
                    "type": "websocket.send",
                    "text": msg.get("text"),
                    "bytes": msg.get("bytes"),
                }
            )
        elif msg["type"] == "websocket.disconnect":
            return
        else:
            msg = f"Unexpected message type: {msg['type']}"
            raise RuntimeError(msg)
