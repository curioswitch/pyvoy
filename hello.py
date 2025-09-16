async def app(scope, recv, send):
    print(scope)
    await send({
        "type": "http.response.start",
        "status": 202,
        "headers": [
            (b"content-type", b"text/plain"),
            (b"x-animal", b"bear"),
        ],
    })
    print('s1')
    await send({
        "type": "http.response.body",
        "body": b"Who are you?",
        "more_body": True,
    })
    print('r1')
    msg = await recv()
    await send({
        "type": "http.response.body",
        "body": b"Hi " + msg["body"] + b". What do you want to do?",
        "more_body": True,
    })
    print('r2')
    msg = await recv()
    await send({
        "type": "http.response.body",
        "body": b"Let's " + msg["body"] + b"!",
        "more_body": False,
    })
