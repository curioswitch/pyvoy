# Settings

The following options can be used to configure pyvoy when using the pyvoy CLI. Most parameters have a corresponding
keyword argument when using `PyvoyServer` programmatically. Envoy can also be [invoked directly](#envoy) with
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
  complicated routing, it is recommended to invoke Envoy [directly](#envoy) with a full config.

## Static files

pyvoy can serve static files from disk alongside your application, backed by the
[envoy-files](https://github.com/curioswitch/envoy-files) dynamic module. The common case is serving a single-page
app bundle next to an API - the application acts as the catch-all and static mounts, at longer path prefixes, win.

- `--static-mount` - Can be specified any number of times to serve a directory of static files. Takes a value in
  the format `prefix=root[=opt:val,opt:val,...]`, where `prefix` is the URL path prefix to mount at and `root` is the
  directory to serve (`~` is expanded). By default the `prefix` is stripped before resolving files under `root`, so
  `--static-mount /assets=./dist` serves `./dist/app.js` at `/assets/app.js`. Supported options:
    - `directory:index|listing|deny` - Behavior when a directory has no matching index file. **Default**: _index_.
    - `index:a.html;b.html` - Files (`;`-separated) attempted in order for directory requests. **Default**: _index.html_.
    - `strip_prefix:/foo` - Override the prefix stripped before file resolution. Pass an empty value to disable.
    - `precompressed:br;gzip;zstd` - Precompressed variants (`;`-separated) served when available. **Default**: _br;gzip_.
    - `dotfiles:true|false` - Whether to serve path segments beginning with a dot. **Default**: _false_.

  For `cache_control`, `mime_overrides`, and other envoy-files knobs, use `PyvoyServer` directly with
  [`StaticMount`](./api.md#pyvoy.StaticMount), which also exposes an `options` dictionary as an escape hatch for any
  remaining envoy-files setting.

## Server

- `--address` - The address to listen on. `--address 0.0.0.0` would listen on all addresses, which is commonly
  needed in container deployments like k8s. **Default**: _127.0.0.1_.
- `--port` - The port to listen on. If `0`, an available port will be automatically used. **Default**: _8000_.
- `--tls-key` / `--tls-cert` - Paths to the TLS key and cert files to enable listening with TLS.
- `--tls-ca-cert` - Path to the TLS cert to verify client certificates with.
- `--tls-require-client-certificate` - Whether a valid TLS client certificate must be present to connect, for mTLS.
  Has no effect if `--tls-ca-cert` is not provided.
- `--tls-port` - Must be used with `--tls-key` / `--tls-cert`. If set, `--port` will listen with plaintext
  and this port will in addition listen with TLS.
- `--tls-disable-http3` - By default, if TLS is enabled, the port will also listen on UDP for HTTP/3 connections.
  Set to disable support for HTTP/3 connections.

## HTTP Client

- `--upstream` - Defines an upstream that can be used with the pyvoy [HTTP Client](./http-client.md). Every upstream
  defines a name and its URL, for example `auth-service=http://auth.svc:8080`. To enable TLS, specify `https://`
  instead - the server's CA cert will be used to authenticate backends, and if TLS key / cert are provided, they
  will be sent for mTLS.

## WebSockets

- `--websockets` - Enables support for ASGI WebSockets. Not enabled by default.
- `--websockets-max-message-size` - Sets the maximum WebSocket message size in bytes. **Default**: _64 MiB_.
- `--websockets-compression` - Sets whether to enable WebSocket per-message deflate compression. **Default**: _enabled_.

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

You can see more details in [deployment](./deployment.md).
