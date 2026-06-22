from collections.abc import Awaitable

from pyqwest import Request, Response

class HTTPTransport:
    """A pyqwest transport implementation using Envoy."""

    def __init__(self, cluster_name: str, *, timeout: float | None = None) -> None:
        """Creates a new HTTPTransport.

        Args:
            cluster_name: The name of the upstream cluster to send requests to.
                Must match an upstream configured during server startup.
            timeout: Timeout in seconds for requests. Defaults to 60s.
        """

    def execute(self, request: Request) -> Awaitable[Response]:
        """Executes the given request, returning the response.

        The response is only valid as long as the server request that initiates the
        request is alive. Do not use this HTTPTransport if you need to fire-and-forget
        requests outside of the request context.
        """
