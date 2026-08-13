# Many error cases like invalid YAML are not possible when using the pyvoy
# CLI or PyvoyServer, but we also support running Envoy directly. We check
# issues that can happen with it here.
from __future__ import annotations

import json
import os
import subprocess
import sys

import pytest
from envoy import get_envoy_path

from pyvoy import PyvoyServer
from pyvoy._server import get_envoy_environ

envoy_path = get_envoy_path()

envoy_env = {**os.environ, **get_envoy_environ()}


def _set_dynamic_filter_config_value(
    config: object, filter_name: str, setting: str, value: int
) -> bool:
    if isinstance(config, dict):
        typed_config = config.get("typed_config")
        if config.get("name") == filter_name and isinstance(typed_config, dict):
            filter_config = typed_config.get("filter_config")
            if isinstance(filter_config, dict):
                pyvoy_config = json.loads(filter_config["value"])
                pyvoy_config[setting] = value
                filter_config["value"] = json.dumps(pyvoy_config)
                return True
        return any(
            _set_dynamic_filter_config_value(item, filter_name, setting, value)
            for item in config.values()
        )
    if isinstance(config, list):
        return any(
            _set_dynamic_filter_config_value(item, filter_name, setting, value)
            for item in config
        )
    return False


@pytest.mark.parametrize(
    ("filter_name", "setting", "value", "minimum"),
    [
        ("pyvoy", "worker_threads", 0, 1),
        ("pyvoy", "worker_threads", sys.maxsize + 1, 1),
        ("pyvoy-ws", "worker_threads", 0, 1),
        ("pyvoy-ws", "websockets_max_message_size", -1, 0),
    ],
)
def test_config_invalid_numeric_setting(
    filter_name: str, setting: str, value: int, minimum: int
) -> None:
    config = PyvoyServer(
        "tests.apps.asgi.kitchensink", websockets=filter_name == "pyvoy-ws"
    ).get_envoy_config()
    assert _set_dynamic_filter_config_value(config, filter_name, setting, value)

    result = subprocess.run(
        [envoy_path, "--config-yaml", json.dumps(config)],
        check=False,
        capture_output=True,
        text=True,
        env=envoy_env,
    )

    assert result.returncode != 0
    assert (
        f"Filter config field '{setting}' must be an integer between {minimum} "
        "and the platform maximum"
    ) in result.stderr


def test_config_invalid_yaml():
    conf = """
static_resources:
  listeners:
  - address:
      socket_address:
        address: 127.0.0.1
        port_value: 0
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          '@type': type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          http_filters:
          - name: pyvoy
            typed_config:
              '@type': type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter
              dynamic_module_config:
                name: pyvoy
              filter_config:
                '@type': type.googleapis.com/google.protobuf.StringValue
                value: 'a: b: c'
              filter_name: pyvoy
              terminal_filter: true
          route_config:
            virtual_hosts:
            - domains:
              - '*'
              name: local_service
          stat_prefix: ingress_http
    name: listener
"""
    result = subprocess.run(
        [envoy_path, "--config-yaml", conf],
        check=False,
        capture_output=True,
        text=True,
        env=envoy_env,
    )
    assert result.returncode != 0
    assert "Failed to parse filter config YAML:" in result.stderr


def test_config_empty_config():
    conf = """
static_resources:
  listeners:
  - address:
      socket_address:
        address: 127.0.0.1
        port_value: 0
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          '@type': type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          http_filters:
          - name: pyvoy
            typed_config:
              '@type': type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter
              dynamic_module_config:
                name: pyvoy
              filter_config:
                '@type': type.googleapis.com/google.protobuf.StringValue
                value: ''
              filter_name: pyvoy
              terminal_filter: true
          route_config:
            virtual_hosts:
            - domains:
              - '*'
              name: local_service
          stat_prefix: ingress_http
    name: listener
"""
    result = subprocess.run(
        [envoy_path, "--config-yaml", conf],
        check=False,
        capture_output=True,
        text=True,
        env=envoy_env,
    )
    assert result.returncode != 0
    assert "Filter config is empty" in result.stderr


def test_config_missing_app():
    conf = """
static_resources:
  listeners:
  - address:
      socket_address:
        address: 127.0.0.1
        port_value: 0
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          '@type': type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          http_filters:
          - name: pyvoy
            typed_config:
              '@type': type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter
              dynamic_module_config:
                name: pyvoy
              filter_config:
                '@type': type.googleapis.com/google.protobuf.StringValue
                value: |
                  interface: asgi
              filter_name: pyvoy
              terminal_filter: true
          route_config:
            virtual_hosts:
            - domains:
              - '*'
              name: local_service
          stat_prefix: ingress_http
    name: listener
"""
    result = subprocess.run(
        [envoy_path, "--config-yaml", conf],
        check=False,
        capture_output=True,
        text=True,
        env=envoy_env,
    )
    assert result.returncode != 0
    assert "Filter config missing required 'app' field" in result.stderr


def test_config_unsupported_interface():
    conf = """
static_resources:
  listeners:
  - address:
      socket_address:
        address: 127.0.0.1
        port_value: 0
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          '@type': type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          http_filters:
          - name: pyvoy
            typed_config:
              '@type': type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter
              dynamic_module_config:
                name: pyvoy
              filter_config:
                '@type': type.googleapis.com/google.protobuf.StringValue
                value: |
                  app: tests.apps.asgi.kitchensink:app
                  interface: cgi
              filter_name: pyvoy
              terminal_filter: true
          route_config:
            virtual_hosts:
            - domains:
              - '*'
              name: local_service
          stat_prefix: ingress_http
    name: listener
"""
    result = subprocess.run(
        [envoy_path, "--config-yaml", conf],
        check=False,
        capture_output=True,
        text=True,
        env=envoy_env,
    )
    assert result.returncode != 0
    assert "Unsupported python interface: cgi" in result.stderr


def test_python_asgi_app_failure():
    conf = """
static_resources:
  listeners:
  - address:
      socket_address:
        address: 127.0.0.1
        port_value: 0
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          '@type': type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          http_filters:
          - name: pyvoy
            typed_config:
              '@type': type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter
              dynamic_module_config:
                name: pyvoy
              filter_config:
                '@type': type.googleapis.com/google.protobuf.StringValue
                value: |
                  app: tests.apps.asgi.kitchensink:notthere
                  interface: asgi
              filter_name: pyvoy
              terminal_filter: true
          route_config:
            virtual_hosts:
            - domains:
              - '*'
              name: local_service
          stat_prefix: ingress_http
    name: listener
"""
    result = subprocess.run(
        [envoy_path, "--config-yaml", conf],
        check=False,
        capture_output=True,
        text=True,
        env=envoy_env,
    )
    assert result.returncode != 0
    assert "Failed to initialize ASGI app" in result.stderr


def test_python_wsgi_app_failure():
    conf = """
static_resources:
  listeners:
  - address:
      socket_address:
        address: 127.0.0.1
        port_value: 0
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          '@type': type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          http_filters:
          - name: pyvoy
            typed_config:
              '@type': type.googleapis.com/envoy.extensions.filters.http.dynamic_modules.v3.DynamicModuleFilter
              dynamic_module_config:
                name: pyvoy
              filter_config:
                '@type': type.googleapis.com/google.protobuf.StringValue
                value: |
                  app: tests.apps.wsgi.kitchensink:notthere
                  interface: wsgi
              filter_name: pyvoy
              terminal_filter: true
          route_config:
            virtual_hosts:
            - domains:
              - '*'
              name: local_service
          stat_prefix: ingress_http
    name: listener
"""
    result = subprocess.run(
        [envoy_path, "--config-yaml", conf],
        check=False,
        capture_output=True,
        text=True,
        env=envoy_env,
    )
    assert result.returncode != 0
    assert "Failed to initialize WSGI app" in result.stderr
