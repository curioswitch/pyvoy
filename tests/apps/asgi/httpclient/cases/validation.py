from __future__ import annotations

import pytest

from pyvoy.asgi.httpclient import HTTPTransport


def transport_invalid_option() -> None:
    # pyvoy's HTTPTransport only exposes `timeout`; mirror pyqwest's
    # transport_invalid_option for the option we support.
    with pytest.raises(ValueError, match="non-negative"):
        HTTPTransport("backend_h1c", timeout=-1)

    with pytest.raises(ValueError, match="non-negative"):
        HTTPTransport("backend_h1c", timeout=float("inf"))

    with pytest.raises(ValueError, match="non-negative"):
        HTTPTransport("backend_h1c", timeout=float("nan"))
