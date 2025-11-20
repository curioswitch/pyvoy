from __future__ import annotations

from typing import TYPE_CHECKING

from starlette.applications import Starlette
from starlette.responses import PlainTextResponse

if TYPE_CHECKING:
    from starlette.requests import Request

app = Starlette()


@app.route("/")
async def homepage(_request: Request) -> PlainTextResponse:
    return PlainTextResponse("Hello, World!")
