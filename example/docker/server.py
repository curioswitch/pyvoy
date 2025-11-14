from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import PlainTextResponse

app = Starlette()


@app.route("/")
async def homepage(_request: Request) -> PlainTextResponse:
    return PlainTextResponse("Hello, World!")
