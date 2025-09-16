import json
import subprocess
import urllib.request


class PyvoyServer:
    listener_address: str
    listener_port: int

    def __init__(self, app: str, *, port: int = 0) -> None:
        self._app = app
        self._port = port

    def __enter__(self) -> "PyvoyServer":
        self.start()
        return self

    def __exit__(self, exc_type, exc_value, traceback) -> None:
        self.stop()

    def start(self) -> None:
        self._upstream = subprocess.Popen(
            ["go", "run", "server.go"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
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
                            "socket_address": {"address": "0.0.0.0", "port_value": self._port}
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

        self._process = subprocess.Popen(
            [
                "/Users/anuraag/git/envoy/bazel-bin/source/exe/envoy-static",
                "--config-yaml",
                json.dumps(config),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        admin_address = ""
        logs = []
        while True:
            line = self._process.stderr.readline()
            logs.append(line)
            if self._process.poll() is not None:
                print("".join(logs))
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
        socket_address = response_data["listener_statuses"][0]["local_address"]["socket_address"]
        self.listener_address = socket_address["address"]
        self.listener_port = socket_address["port_value"]

    def stop(self) -> None:
        self._process.terminate()
        self._upstream.terminate()
        self._process.wait()
        self._upstream.wait()
