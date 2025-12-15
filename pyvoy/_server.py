from __future__ import annotations

import asyncio
import base64
import contextlib
import json
import os
import subprocess
import sys
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import IO, TYPE_CHECKING, Literal

import find_libpython
import yaml
from envoy import get_envoy_path

from ._bin import get_pyvoy_dir_path

if TYPE_CHECKING:
    from collections.abc import Iterable
    from types import TracebackType

Interface = Literal["asgi", "wsgi"]

LogLevel = Literal[
    "trace", "debug", "info", "warning", "warn", "error", "critical", "off"
]


@dataclass(frozen=True)
class Mount:
    app: str
    """The application to mount."""

    path: str
    """The path prefix to mount the application at."""

    interface: str = "asgi"
    """The interface type of the application."""


def get_envoy_environ() -> dict[str, str]:
    env = {
        "PYTHONPATH": os.pathsep.join(sys.path),
        "PYTHONHOME": f"{sys.prefix}:{sys.exec_prefix}",
        "ENVOY_DYNAMIC_MODULES_SEARCH_PATH": str(get_pyvoy_dir_path()),
    }
    if len(args := _maybe_patch_args_with_debug([sys.executable, __file__])) > 2:
        env["PYVOY_PYDEVD_ARGS"] = " ".join(args)

    if os.name == "posix":
        # We use candidate_paths() instead of find_python because the latter
        # returns the real path, not a symlink. In macOS framework packages,
        # the real path is called Python, not libpython.
        candidates = [Path(p) for p in find_libpython.candidate_paths()]
        candidates = [
            p for p in candidates if p.exists() and p.name.startswith("libpython")
        ]
        if candidates:
            if sys.platform == "darwin":
                libpython_dir = str(candidates[0].parent)
                env["DYLD_LIBRARY_PATH"] = libpython_dir
            else:
                env["LD_PRELOAD"] = str(candidates[0])
    if sys.platform == "win32":
        libpython = find_libpython.find_libpython()
        if libpython is not None:
            env["PATH"] = f"{Path(libpython).parent};{os.environ.get('PATH', '')}"

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
    _root_path: str
    _log_level: LogLevel
    _tls_require_client_certificate: bool
    _worker_threads: int | None
    _lifespan: bool | None
    _additional_envoy_args: list[str] | None

    _admin_address: str | None
    _started: bool

    def __init__(
        self,
        app: str | Iterable[Mount],
        *,
        address: str = "127.0.0.1",
        port: int = 0,
        tls_port: int | None = None,
        tls_key: bytes | os.PathLike | None = None,
        tls_cert: bytes | os.PathLike | None = None,
        tls_ca_cert: bytes | os.PathLike | None = None,
        tls_enable_http3: bool = True,
        tls_require_client_certificate: bool = True,
        interface: Interface = "asgi",
        root_path: str = "",
        log_level: LogLevel = "error",
        worker_threads: int | None = None,
        lifespan: bool | None = None,
        additional_envoy_args: list[str] | None = None,
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
        self._tls_require_client_certificate = tls_require_client_certificate
        self._interface = interface
        self._root_path = root_path
        self._stdout = stdout
        self._stderr = stderr
        self._print_envoy_config = print_envoy_config
        self._log_level = log_level
        self._worker_threads = worker_threads
        self._lifespan = lifespan
        self._additional_envoy_args = additional_envoy_args

        self._listener_port_tls = None
        self._started = False
        self._admin_address = None

    async def __aenter__(self) -> PyvoyServer:
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
            if sys.platform == "win32":
                admin_address_file.close()
            args = [
                "--config-yaml",
                json.dumps(config),
                "--admin-address-path",
                admin_address_file.name,
                "--use-dynamic-base-id",
                "--log-level",
                self._log_level,
            ]
            if self._additional_envoy_args:
                args.extend(self._additional_envoy_args)
            self._process = await asyncio.create_subprocess_exec(
                get_envoy_path(),
                *args,
                stdout=self._stdout,
                stderr=self._stderr,
                env=env,
            )
            for _ in range(100):
                if self._process.returncode is not None:
                    return
                with contextlib.suppress(Exception):
                    admin_address = Path(admin_address_file.name).read_text()
                    if admin_address:
                        self._admin_address = admin_address
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

        self._started = True

    async def wait(self) -> None:
        await self._process.wait()

    async def stop(self) -> None:
        if not self._started:
            return
        self._started = False
        try:
            if sys.platform == "win32":
                # Easiest way to gracefully shutdown Envoy on Windows is to use admin API
                if self._admin_address is not None:
                    req = urllib.request.Request(
                        f"http://{self._admin_address}/quitquitquit", method="POST"
                    )
                    await asyncio.to_thread(urllib.request.urlopen, req)
                else:
                    # Shouldn't be running even but send terminate just in case
                    self._process.terminate()
            else:
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

    @property
    def stopped(self) -> bool:
        return not self._started

    def get_envoy_config(self) -> dict:
        base_pyvoy_config = {}
        if self._worker_threads is not None:
            base_pyvoy_config["worker_threads"] = self._worker_threads
        if self._lifespan is not None:
            base_pyvoy_config["lifespan"] = self._lifespan
        virtual_host_config = {"name": "local_service", "domains": ["*"]}
        if isinstance(self._app, str):
            pyvoy_config: dict[str, str | int | bool] = {
                "app": self._app,
                "interface": self._interface,
                "root_path": self._root_path,
                **base_pyvoy_config,
            }

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
                            "value": json.dumps(pyvoy_config),
                        },
                    },
                }
            ]
        else:
            matcher_map = {}
            for mount in self._app:
                pyvoy_config = {
                    "app": mount.app,
                    "interface": mount.interface,
                    "root_path": mount.path,
                    **base_pyvoy_config,
                }
                matcher_map[mount.path] = {
                    "action": {
                        "name": "composite_action",
                        "typed_config": {
                            "@type": "type.googleapis.com/envoy.extensions.filters.http.composite.v3.ExecuteFilterAction",
                            "typed_config": {
                                "name": "pyvoy",
                                "typed_config": {
                                    "@type": "type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter",
                                    "dynamic_module_config": {"name": "pyvoy"},
                                    "filter_name": "pyvoy",
                                    "terminal_filter": True,
                                    "filter_config": {
                                        "@type": "type.googleapis.com/google.protobuf.StringValue",
                                        "value": json.dumps(pyvoy_config),
                                    },
                                },
                            },
                        },
                    }
                }
            http_filters = [
                {
                    "name": "envoy.filters.http.composite",
                    "typed_config": {
                        "@type": "type.googleapis.com/envoy.extensions.common.matching.v3.ExtensionWithMatcher",
                        "xds_matcher": {
                            "matcher_tree": {
                                "input": {
                                    "name": "request-headers",
                                    "typed_config": {
                                        "@type": "type.googleapis.com/envoy.type.matcher.v3.HttpRequestHeaderMatchInput",
                                        "header_name": ":path",
                                    },
                                },
                                "prefix_match_map": {"map": matcher_map},
                            }
                        },
                        "extension_config": {
                            "name": "envoy.filters.http.composite",
                            "typed_config": {
                                "@type": "type.googleapis.com/envoy.extensions.filters.http.composite.v3.Composite"
                            },
                        },
                    },
                },
                {
                    "name": "envoy.filters.http.router",
                    "typed_config": {
                        "@type": "type.googleapis.com/envoy.extensions.filters.http.router.v3.Router"
                    },
                },
            ]
            virtual_host_config["routes"] = [
                {
                    "match": {"prefix": "/"},
                    "direct_response": {
                        "status": 404,
                        "body": {"inline_string": "Not Found"},
                    },
                }
            ]
        http_config = {
            "@type": "type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager",
            "stat_prefix": "ingress_http",
            "route_config": {"virtual_hosts": [virtual_host_config]},
            "http_filters": http_filters,
            "generate_request_id": False,
        }

        enable_http3 = self._tls_enable_http3 and (
            self._tls_key or self._tls_cert or self._tls_ca_cert
        )
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
                    "require_client_certificate": bool(self._tls_ca_cert)
                    and self._tls_require_client_certificate,
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


def _maybe_patch_args_with_debug(args: list[str]) -> list[str]:
    """Makes an attempt to patch a Python command with pydevd.

    This will add debugger arguments if we are currently running under a debugger, which
    we use to reconstruct it within pyvoy.
    """
    try:
        import _pydev_bundle  # pyright: ignore[reportMissingImports]  # noqa: PLC0415

        return _pydev_bundle.pydev_monkey.patch_args(args)
    except Exception:
        return args
