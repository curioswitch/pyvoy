import asyncio
import base64
import contextlib
import json
import os
import subprocess
import sys
import urllib.request
from pathlib import Path
from tempfile import NamedTemporaryFile
from types import TracebackType
from typing import IO, Literal

import yaml
from find_libpython import find_libpython

from ._bin import get_envoy_path, get_pyvoy_dir_path

Interface = Literal["asgi", "wsgi"]


def get_envoy_environ() -> dict[str, str]:
    env = {
        "PYTHONPATH": os.pathsep.join(sys.path),
        "PYTHONHOME": f"{sys.prefix}:{sys.exec_prefix}",
        "ENVOY_DYNAMIC_MODULES_SEARCH_PATH": str(get_pyvoy_dir_path()),
    }

    if os.name == "posix":
        libpython_path = find_libpython()
        if libpython_path:
            if sys.platform == "darwin":
                libpython_dir = str(Path(libpython_path).parent)
                env["DYLD_LIBRARY_PATH"] = libpython_dir
            else:
                env["LD_PRELOAD"] = libpython_path

    return env


class PyvoyServer:
    _listener_address: str
    _listener_port: int
    _listener_port_tls: int | None
    _stdout: int | IO[bytes] | None
    _stderr: int | IO[bytes] | None
    _print_startup_logs: bool
    _print_envoy_config: bool
    _interface: Interface

    _stopped: bool

    def __init__(
        self,
        app: str,
        *,
        address: str = "127.0.0.1",
        port: int = 0,
        tls_port: int | None = None,
        tls_key: bytes | os.PathLike | None = None,
        tls_cert: bytes | os.PathLike | None = None,
        tls_ca_cert: bytes | os.PathLike | None = None,
        tls_enable_http3: bool = True,
        interface: Interface = "asgi",
        stdout: int | IO[bytes] | None = subprocess.DEVNULL,
        stderr: int | IO[bytes] | None = subprocess.DEVNULL,
        print_envoy_config: bool = False,
    ) -> None:
        self._app = app
        self._address = address
        self._port = port
        self._tls_port = tls_port
        self._tls_key = tls_key
        self._tls_cert = tls_cert
        self._tls_ca_cert = tls_ca_cert
        self._tls_enable_http3 = tls_enable_http3
        self._interface = interface
        self._stdout = stdout
        self._stderr = stderr
        self._print_envoy_config = print_envoy_config

        self._listener_port_tls = None
        self._stopped = False

    async def __aenter__(self) -> "PyvoyServer":
        await self.start()
        return self

    async def __aexit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        await self.stop()

    async def start(self) -> None:
        config = self.get_envoy_config()

        if self._print_envoy_config:
            print(yaml.dump(config))  # noqa: T201
            return

        env = {**os.environ, **get_envoy_environ()}

        with NamedTemporaryFile("r") as admin_address_file:
            self._process = await asyncio.create_subprocess_exec(
                get_envoy_path(),
                "--config-yaml",
                json.dumps(config),
                "--admin-address-path",
                admin_address_file.name,
                "--use-dynamic-base-id",
                stdout=self._stdout,
                stderr=self._stderr,
                env=env,
            )
            for _ in range(100):
                with contextlib.suppress(Exception):
                    admin_address = Path(admin_address_file.name).read_text()
                    if admin_address:
                        response = await asyncio.to_thread(
                            urllib.request.urlopen,
                            f"http://{admin_address}/listeners?format=json",
                        )
                        response_data = json.loads(response.read())
                        socket_address = response_data["listener_statuses"][0][
                            "local_address"
                        ]["socket_address"]
                        self._listener_address = socket_address["address"]
                        self._listener_port = socket_address["port_value"]
                        if self._tls_port is not None:
                            socket_address_tls = response_data["listener_statuses"][1][
                                "local_address"
                            ]["socket_address"]
                            self._listener_port_tls = socket_address_tls["port_value"]
                        break
                await asyncio.sleep(0.1)

    async def wait(self) -> None:
        await self._process.wait()

    async def stop(self) -> None:
        if self._stopped:
            return
        self._stopped = True
        try:
            self._process.terminate()
            await self._process.wait()
        except ProcessLookupError:
            # Envoy likely crashed, no need to look like multiple errors.
            pass

    @property
    def listener_address(self) -> str:
        return self._listener_address

    @property
    def listener_port(self) -> int:
        return self._listener_port

    @property
    def listener_port_tls(self) -> int | None:
        return self._listener_port_tls

    @property
    def stdout(self) -> asyncio.StreamReader | None:
        return self._process.stdout

    @property
    def stderr(self) -> asyncio.StreamReader | None:
        return self._process.stderr

    def get_envoy_config(self) -> dict:
        enable_http3 = self._tls_enable_http3 and (
            self._tls_key or self._tls_cert or self._tls_ca_cert
        )
        http_filters = [
            {
                "name": "pyvoy",
                "typed_config": {
                    "@type": "type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter",
                    "dynamic_module_config": {"name": "pyvoy"},
                    "filter_name": "pyvoy",
                    "terminal_filter": True,
                    "filter_config": {
                        "@type": "type.googleapis.com/google.protobuf.StringValue",
                        "value": json.dumps(
                            {"app": self._app, "interface": self._interface}
                        ),
                    },
                },
            }
        ]
        virtual_host_config = {"name": "local_service", "domains": ["*"]}
        http_config = {
            "@type": "type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager",
            "stat_prefix": "ingress_http",
            "route_config": {"virtual_hosts": [virtual_host_config]},
            "http_filters": http_filters,
            "generate_request_id": False,
        }
        if enable_http3:
            http_config["http3_protocol_options"] = {}
        filter_chain: dict = {
            "filters": [
                {
                    "name": "envoy.filters.network.http_connection_manager",
                    "typed_config": http_config,
                }
            ]
        }

        common_tls_context = {}
        tls_filter_chain = None
        if self._tls_key or self._tls_cert or self._tls_ca_cert:
            tls_certificate = {}
            if self._tls_key:
                tls_certificate["private_key"] = _to_datasource(self._tls_key)
            if self._tls_cert:
                tls_certificate["certificate_chain"] = _to_datasource(self._tls_cert)
            if tls_certificate:
                common_tls_context["tls_certificates"] = [tls_certificate]
            if self._tls_ca_cert:
                common_tls_context["validation_context"] = {
                    "trusted_ca": _to_datasource(self._tls_ca_cert)
                }
            transport_socket = {
                "name": "envoy.transport_sockets.tls",
                "typed_config": {
                    "@type": "type.googleapis.com/envoy.extensions.transport_sockets.tls.v3.DownstreamTlsContext",
                    "common_tls_context": {
                        **common_tls_context,
                        "alpn_protocols": ["h2", "http/1.1"],
                    },
                    "require_client_certificate": bool(self._tls_ca_cert),
                },
            }
            if self._tls_port is not None:
                tls_filter_chain = {
                    **filter_chain,
                    "transport_socket": transport_socket,
                }
            else:
                filter_chain["transport_socket"] = transport_socket

        listeners = [
            {
                "name": "listener",
                "address": {
                    "socket_address": {
                        "address": self._address,
                        "port_value": self._port,
                    }
                },
                "filter_chains": [filter_chain],
            }
        ]
        if tls_filter_chain is not None:
            listeners.append(
                {
                    "name": "listener_tls",
                    "address": {
                        "socket_address": {
                            "address": self._address,
                            "port_value": self._tls_port,
                        }
                    },
                    "filter_chains": [tls_filter_chain],
                }
            )
        if enable_http3:
            listeners.append(
                {
                    "name": "listener_udp",
                    "address": {
                        "socket_address": {
                            "address": self._address,
                            "port_value": self._tls_port
                            if self._tls_port is not None
                            else self._port,
                            "protocol": "UDP",
                        }
                    },
                    "udp_listener_config": {
                        "quic_options": {},
                        "downstream_socket_config": {"prefer_gro": True},
                    },
                    "filter_chains": [
                        {
                            "filters": [
                                {
                                    "name": "envoy.filters.network.http_connection_manager",
                                    "typed_config": {
                                        "@type": "type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager",
                                        "codec_type": "HTTP3",
                                        "stat_prefix": "ingress_http",
                                        "route_config": {
                                            "virtual_hosts": [
                                                {
                                                    "name": "local_service",
                                                    "domains": ["*"],
                                                }
                                            ]
                                        },
                                        "http_filters": http_filters,
                                    },
                                }
                            ],
                            "transport_socket": {
                                "name": "envoy.transport_sockets.quic",
                                "typed_config": {
                                    "@type": "type.googleapis.com/envoy.extensions.transport_sockets.quic.v3.QuicDownstreamTransport",
                                    "downstream_tls_context": {
                                        "common_tls_context": common_tls_context
                                    },
                                },
                            },
                        }
                    ],
                }
            )

        return {
            "admin": {
                "address": {"socket_address": {"address": "127.0.0.1", "port_value": 0}}
            },
            "static_resources": {"listeners": listeners},
        }


def _to_datasource(value: bytes | os.PathLike) -> dict:
    if isinstance(value, os.PathLike):
        return {"filename": os.fspath(value)}
    return {"inline_bytes": base64.b64encode(value).decode()}
