from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, HTTPScope


async def app(
    _scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 202,
            "headers": [(b"content-type", b"text/plain"), (b"x-animal", b"bear")],
            "trailers": False,
        }
    )
    await send(
        {"type": "http.response.body", "body": b"Who are you?", "more_body": True}
    )
    msg = await recv()
    if msg["type"] == "http.request":
        await send(
            {
                "type": "http.response.body",
                "body": b"Hi " + msg["body"] + b". What do you want to do?",
                "more_body": True,
            }
        )
    msg = await recv()
    if msg["type"] == "http.request":
        await send(
            {
                "type": "http.response.body",
                "body": b"Let's " + msg["body"] + b"!",
                "more_body": False,
            }
        )
