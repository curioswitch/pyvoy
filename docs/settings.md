# Settings

The following options can be used to configure pyvoy when using the pyvoy CLI. Most parameters have a corresponding
keyword argument when using `PyvoyServer` programmatically. Envoy can also be [invoked directoy](#envoy) with
pyvoy for even further customization.

## Application

- `app` - A single positional argument that is the Python application to run, in the format `<module>:<attribute>`,
  e.g., `myapp.server:app`. If `attribute` matches `app`, it can be omitted, e.g., `myapp.server`.
- `--interface` - One of `asgi` or `wsgi` to indicate the type of application being served. **Default**: _asgi_.

## Server

- `--address` - The address to listen on. `--address 0.0.0.0` would listen on all addresses, which is commonly
  needed in container deployments like k8s. **Default**: _127.0.0.1_.
- `--port` - The port to listen on. If `0`, an available port will be automatically used. **Default**: _8000_.
- `--tls-key` / `--tls-cert` - Paths to the TLS key and cert files to enable listening with TLS.
- `--tls-port` - Must be used with `--tls-key` / `--tls-cert`. If set, `--port` will listen with plaintext
  and this port will in addition listen with TLS.
- `--tls-disable-http3` - By default, if TLS is enabled, the port will also listen on UDP for HTTP/3 connections.
  Set to disable support for HTTP/3 connections.
- `--tls-require-client-certificate` - Whether a valid TLS client certificate must be present to connect, for mTLS.

## Envoy

pyvoy can be used by directly invoking Envoy with an appropriate YAML config. This can be useful for production
deployments by unlocking _all_ the features of Envoy itself. The pyvoy CLI captures common configuration options,
and more may be added with popular demand, but it is impossible to expose every single feature via a CLI option.
