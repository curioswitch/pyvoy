# Many error cases like invalid YAML are not possible when using the pyvoy
# CLI or PyvoyServer, but we also support running Envoy directly. We check
# issues that can happen with it here.

import os
import subprocess

from pyvoy._bin import get_envoy_path
from pyvoy._server import get_envoy_environ

envoy_path = get_envoy_path()

envoy_env = {**os.environ, **get_envoy_environ()}


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
