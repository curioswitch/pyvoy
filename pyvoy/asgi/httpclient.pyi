from collections.abc import Awaitable

from pyqwest import Request, Response

class HTTPTransport:
    def __init__(self, cluster_name: str, *, timeout: float | None = None) -> None: ...
    def execute(self, request: Request) -> Awaitable[Response]: ...
