from starlette.applications import Starlette
from starlette.responses import PlainTextResponse

app = Starlette()


@app.route("/")
async def homepage(request):
    return PlainTextResponse("Hello, World!")
