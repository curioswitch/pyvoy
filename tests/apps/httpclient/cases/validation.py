from __future__ import annotations

import pytest
from pyvoy.wsgi.httpclient import HTTPTransport as SyncHTTPTransport

from pyvoy.asgi.httpclient import HTTPTransport as AsyncHTTPTransport


def transport_invalid_option() -> None:
    # pyvoy's HTTPTransport only exposes `timeout`; mirror pyqwest's
    # transport_invalid_option for the option we support.
    for transport in (AsyncHTTPTransport, SyncHTTPTransport):
        with pytest.raises(ValueError, match="non-negative"):
            transport("backend_h1c", timeout=-1)

        with pytest.raises(ValueError, match="non-negative"):
            transport("backend_h1c", timeout=float("inf"))

        with pytest.raises(ValueError, match="non-negative"):
            transport("backend_h1c", timeout=float("nan"))
