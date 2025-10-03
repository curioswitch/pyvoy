import json
import os
import subprocess
import sys
import urllib.request
from types import TracebackType
from typing import IO

import yaml

from ._bin import get_envoy_path, get_pyvoy_dir_path


class PyvoyServer:
    _listener_address: str
    _listener_port: int
    _print_startup_logs: bool
    _print_envoy_config: bool

    _output: IO[str]

    def __init__(
        self,
        app: str,
        *,
        address: str = "127.0.0.1",
        port: int = 0,
        print_startup_logs: bool = False,
        print_envoy_config: bool = False,
    ) -> None:
        self._app = app
        self._address = address
        self._port = port
        self._print_startup_logs = print_startup_logs
        self._print_envoy_config = print_envoy_config

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
                                                        "terminal_filter": True,
                                                        "filter_config": {
                                                            "@type": "type.googleapis.com/google.protobuf.StringValue",
                                                            "value": self._app,
                                                        },
                                                    },
                                                }
                                            ],
                                        },
                                    }
                                ]
                            }
                        ],
                    }
                ]
            },
        }

        if self._print_envoy_config:
            print(yaml.dump(config))  # noqa: T201
            return

        pythonpath = os.pathsep.join(sys.path)

        self._process = subprocess.Popen(  # noqa: S603 - OK
            [get_envoy_path(), "--config-yaml", json.dumps(config)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            env={
                **os.environ,
                "PYTHONPATH": pythonpath,
                "PYTHONHOME": f"{sys.prefix}:{sys.exec_prefix}",
                "ENVOY_DYNAMIC_MODULES_SEARCH_PATH": get_pyvoy_dir_path(),
            },
        )
        admin_address = ""
        assert self._process.stdout is not None  # noqa: S101
        self._output = self._process.stdout
        startup_logs = []
        while True:
            line = self._output.readline()
            if self._print_startup_logs:
                print(line, end="")  # noqa: T201
            else:
                startup_logs.append(line)
            if self._process.poll() is not None:
                if not self._print_startup_logs:
                    print("".join(startup_logs), end="")  # noqa: T201
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
        self._output.close()
        self._process.terminate()
        self._process.wait()

    @property
    def listener_address(self) -> str:
        return self._listener_address

    @property
    def listener_port(self) -> int:
        return self._listener_port

    @property
    def output(self) -> IO[str]:
        return self._output
