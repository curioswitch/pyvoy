from __future__ import annotations

import os

from starlette.applications import Starlette
from starlette.middleware import Middleware
from starlette.middleware.gzip import GZipMiddleware
from starlette.responses import Response
from starlette.routing import Route

# The response body size, set by run_compression_benchmark before launching.
_BODY_SIZE = int(os.environ.get("BENCH_COMPRESSION_BYTES", str(16 * 1024)))

# Matched to Envoy's gzip default of level 6.
_LEVEL = int(os.environ.get("BENCH_COMPRESSION_LEVEL", "6"))

# JSON-lines shaped text, compressible like a real API response but not a single
# repeated run of bytes.
_RECORD = (
    '{{"id": {i}, "name": "item-{i}", "status": "active", '
    '"description": "the quick brown fox jumps over the lazy dog"}}\n'
)
_BODY = "".join(
    _RECORD.format(i=i) for i in range(_BODY_SIZE // len(_RECORD.format(i=0)) + 1)
)[:_BODY_SIZE].encode()


async def _respond(_request: object) -> Response:
    return Response(_BODY, media_type="application/json")


_routes = [Route("/", _respond)]

# Compression done in Python, the baseline for a typical ASGI deployment.
app = Starlette(
    routes=_routes,
    middleware=[Middleware(GZipMiddleware, minimum_size=500, compresslevel=_LEVEL)],
)

# The same application without the middleware, for servers compressing natively.
plain = Starlette(routes=_routes)
