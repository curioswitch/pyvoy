import asyncio

async def app(scope, send):
    await send({
        "type": "http.response.start",
        "status": 202,
    })
    await send({
        "type": "http.response.body",
        "body": b"Hello",
        "more_body": True,
    })
    await asyncio.sleep(5)
    await send({
        "type": "http.response.body",
        "body": b" World!",
        "more_body": False,
    })
