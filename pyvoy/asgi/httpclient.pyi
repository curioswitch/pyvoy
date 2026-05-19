from collections.abc import Awaitable

from pyqwest import Request, Response

class HTTPTransport:
    def __init__(self, backend: str | None = None) -> None: ...
    def execute(self, request: Request) -> Awaitable[Response]: ...
