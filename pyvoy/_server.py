import json
import os
import subprocess
import sys
import urllib.request
from types import TracebackType

from ._bin import get_envoy_path, get_pyvoy_dir_path, get_upstream_path


class PyvoyServer:
    _listener_address: str
    _listener_port: int

    def __init__(self, app: str, *, address: str = "127.0.0.1", port: int = 0) -> None:
        self._app = app
        self._address = address
        self._port = port

    def __enter__(self) -> "PyvoyServer":
        self.start()
        return self

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.stop()

    def start(self) -> None:
        self._upstream = subprocess.Popen(  # noqa: S603 - OK
            [get_upstream_path()],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        assert self._upstream.stderr is not None  # noqa: S101
        line = self._upstream.stderr.readline()
        if "Listening on port: " not in line:
            msg = "Upstream server failed to start"
            raise RuntimeError(msg)
        upstream_port = int(line.split("Listening on port: ")[1].strip())

        config = {
            "admin": {
                "address": {"socket_address": {"address": "127.0.0.1", "port_value": 0}}
            },
            "static_resources": {
                "listeners": [
                    {
                        "name": "listener",
                        "address": {
                            "socket_address": {
                                "address": self._address,
                                "port_value": self._port,
                            }
                        },
                        "filter_chains": [
                            {
                                "filters": [
                                    {
                                        "name": "envoy.filters.network.http_connection_manager",
                                        "typed_config": {
                                            "@type": "type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager",
                                            "stat_prefix": "ingress_http",
                                            "route_config": {
                                                "virtual_hosts": [
                                                    {
                                                        "name": "local_proxy_route",
                                                        "domains": ["*"],
                                                        "routes": [
                                                            {
                                                                "match": {
                                                                    "prefix": "/"
                                                                },
                                                                "route": {
                                                                    "cluster": "upstream"
                                                                },
                                                            }
                                                        ],
                                                    }
                                                ]
                                            },
                                            "http_filters": [
                                                {
                                                    "name": "pyvoy",
                                                    "typed_config": {
                                                        "@type": "type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter",
                                                        "dynamic_module_config": {
                                                            "name": "pyvoy"
                                                        },
                                                        "filter_name": "pyvoy",
                                                        "filter_config": {
                                                            "@type": "type.googleapis.com/google.protobuf.StringValue",
                                                            "value": self._app,
                                                        },
                                                    },
                                                },
                                                {
                                                    "name": "envoy.filters.http.router",
                                                    "typed_config": {
                                                        "@type": "type.googleapis.com/envoy.extensions.filters.http.router.v3.Router"
                                                    },
                                                },
                                            ],
                                        },
                                    }
                                ]
                            }
                        ],
                    }
                ],
                "clusters": [
                    {
                        "name": "upstream",
                        "connect_timeout": "0.25s",
                        "type": "STATIC",
                        "lb_policy": "ROUND_ROBIN",
                        "http2_protocol_options": {},
                        "load_assignment": {
                            "cluster_name": "upstream",
                            "endpoints": [
                                {
                                    "lb_endpoints": [
                                        {
                                            "endpoint": {
                                                "address": {
                                                    "socket_address": {
                                                        "address": "127.0.0.1",
                                                        "port_value": upstream_port,
                                                    }
                                                }
                                            }
                                        }
                                    ]
                                }
                            ],
                        },
                    }
                ],
            },
        }

        pythonpath = os.pathsep.join(sys.path)

        self._process = subprocess.Popen(  # noqa: S603 - OK
            [get_envoy_path(), "--config-yaml", json.dumps(config)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env={
                **os.environ,
                "PYTHONPATH": pythonpath,
                "PYTHONHOME": f"{sys.prefix}:{sys.exec_prefix}",
                "ENVOY_DYNAMIC_MODULES_SEARCH_PATH": get_pyvoy_dir_path(),
            },
        )
        admin_address = ""
        assert self._process.stderr is not None  # noqa: S101
        while True:
            line = self._process.stderr.readline()
            print(line, end="")
            if self._process.poll() is not None:
                msg = "Envoy process exited unexpectedly"
                raise RuntimeError(msg)
            if "admin address:" in line:
                admin_address = line.split("admin address:")[1].strip()
            if "starting main dispatch loop" in line:
                break
        response = urllib.request.urlopen(
            f"http://{admin_address}/listeners?format=json"
        )
        response_data = json.loads(response.read())
        socket_address = response_data["listener_statuses"][0]["local_address"][
            "socket_address"
        ]
        self._listener_address = socket_address["address"]
        self._listener_port = socket_address["port_value"]

    def stop(self) -> None:
        self._process.terminate()
        self._upstream.terminate()
        self._process.wait()
        self._upstream.wait()

    @property
    def listener_address(self) -> str:
        return self._listener_address

    @property
    def listener_port(self) -> int:
        return self._listener_port
