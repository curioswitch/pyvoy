from collections.abc import Iterator

import pytest

from pyvoy import PyvoyServer


@pytest.fixture(scope="module")
def kitchensink_url() -> Iterator[str]:
    with PyvoyServer("tests.apps.kitchensink") as server:
        yield f"http://{server.listener_address}:{server.listener_port}"
