# Settings

The following options can be used to configure pyvoy when using the pyvoy CLI. Most parameters have a corresponding
keyword argument when using `PyvoyServer` programmatically. Envoy can also be [invoked directoy](#envoy) with
pyvoy for even further customization.

## Application

- `app` - A single positional argument that is the Python application to run, in the format `<module>:<attribute>`,
  e.g., `myapp.server:app`. If `attribute` matches `app`, it can be omitted, e.g., `myapp.server`.
- `--interface` - One of `asgi` or `wsgi` to indicate the type of application being served. **Default**: _asgi_.
- `--root-path` - The root path the application is mounted on if behind a reverse proxy.
- `--worker-threads` - The number of Python threads to serve requests with. When using ASGI, this should generally
  be kept at 1 unless using free-threaded Python. **Default**: 1 for ASGI, 200 for WSGI.
- `--[no-]lifespan` - Whether to require or disable ASGI lifespan support. By default, we try to run the application
  with lifespan and ignore any exception that is raised. If enabled explicitly, an exception will cause the server to
  fail to start. If disabled, lifespan will not be run at all.
- `--additional-mount` - Can be specified any number of times to mount additional applications in the server. Takes
  a value in the format `app=path=interface`, with the path being where the application is mounted. For more
  complicated routing, it is recommended to invoke Envoy [directly](#envoy) wth a full config.

## Server

- `--address` - The address to listen on. `--address 0.0.0.0` would listen on all addresses, which is commonly
  needed in container deployments like k8s. **Default**: _127.0.0.1_.
- `--port` - The port to listen on. If `0`, an available port will be automatically used. **Default**: _8000_.
- `--tls-key` / `--tls-cert` - Paths to the TLS key and cert files to enable listening with TLS.
- `--tls-ca-cert` - Path to the TLS cert to verify client certificates with.
- `--tls-require-client-certificate` - Whether a valid TLS client certificate must be present to connect, for mTLS.
  has no effect if `--tls-ca-cert` is not provided.
- `--tls-port` - Must be used with `--tls-key` / `--tls-cert`. If set, `--port` will listen with plaintext
  and this port will in addition listen with TLS.
- `--tls-disable-http3` - By default, if TLS is enabled, the port will also listen on UDP for HTTP/3 connections.
  Set to disable support for HTTP/3 connections.

## Development

- `--reload` - Whether to automatically restart the server on code changes.
- `--reload-dirs` - Directories to watch for code changes. **Default**: current directory
- `--reload-includes` - File patterns to include for reload. **Default**: `*.py`
- `--reload-excludes` - File patterns to exclude from reload. **Default**: `.*,*.py[cod],*.sw.*,~*`

## Envoy

pyvoy can be used by directly invoking Envoy with an appropriate YAML config. This can be useful for production
deployments by unlocking _all_ the features of Envoy itself. The pyvoy CLI captures common configuration options,
and more may be added with popular demand, but it is impossible to expose every single feature via a CLI option.

These flags cause the CLI to print output rather than start a server to help setup for running Envoy directly.

- `--print-envoy-config` - Prints the Envoy config YAML that would be used if actually starting the server.
- `--print-envoy-entrypoint` - Prints a shell script with environment variables included needed for setting up pyvoy.

You can see more details in [./deployment.md](deployment).
