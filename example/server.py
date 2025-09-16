async def app(scope, recv, send) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 202,
            "headers": [(b"content-type", b"text/plain"), (b"x-animal", b"bear")],
        }
    )
    await send(
        {"type": "http.response.body", "body": b"Who are you?", "more_body": True}
    )
    msg = await recv()
    await send(
        {
            "type": "http.response.body",
            "body": b"Hi " + msg["body"] + b". What do you want to do?",
            "more_body": True,
        }
    )
    msg = await recv()
    await send(
        {
            "type": "http.response.body",
            "body": b"Let's " + msg["body"] + b"!",
            "more_body": False,
        }
    )
