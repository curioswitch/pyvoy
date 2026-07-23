# HTTP Client

While pyvoy is primarily a Python application server, it also provides an HTTP client
implementation using Envoy, which is commonly required by apps. For ASGI, it is an implementation of
pyqwest's [Transport](https://pyqwest.dev/reference/async/#pyqwest.Transport), and for WSGI, of
pyqwest's [SyncTransport](https://pyqwest.dev/reference/sync/#pyqwest.SyncTransport), which
means it can be slotted into a pyqwest [Client](https://pyqwest.dev/reference/async/#pyqwest.Client)
or [SyncClient](https://pyqwest.dev/reference/sync/#pyqwest.SyncClient) as-is. Envoy's HTTP client
implementation provides many advanced features such as DNS load balancing and circuit breakers
needed for advanced production applications.

Envoy HTTP client depends on specifying upstream addresses in configuration; this means
the pyvoy HTTP client is not appropriate for accessing arbitrary servers. In practice,
most applications access a fixed set of backend servers which can be configured as
upstreams. pyvoy provides the [`--upstream`](./settings.md#http-client)
flag to specify a backend host.

```bash
uv run pyvoy package.app --upstream auth-svc=http://localhost:8081 --upstream user-svc=http://localhost:8082
```

Your code initializes a pyvoy `HTTPTransport` with the name passed to upstream.
It is stateless so can be defined anywhere including module constants. Then, just
use it as normal - if you use [Connect-Python](https://connectrpc.com/docs/python/getting-started/),
you can pass in the client and benefit from Envoy's features. Notably, when using gRPC protocol,
having DNS load balancing can improve connectivity significantly.

```python
from pyqwest import Client
from pyvoy.asgi.httpclient import HTTPTransport

auth_client = Client(transport=HTTPTransport("auth-svc"))
user_client = Client(transport=HTTPTransport("user-svc"))

async def app(scope, recv, send):
    res = await auth_client.post("/fetch-token")
    user = await user_client.post("/get-user", headers={"Authorization": f"Bearer {res.text()}"})
```

For WSGI, use `pyvoy.wsgi.httpclient.HTTPTransport` with a pyqwest `SyncClient` instead.

```python
from pyqwest import SyncClient
from pyvoy.wsgi.httpclient import HTTPTransport

auth_client = SyncClient(transport=HTTPTransport("auth-svc"))
user_client = SyncClient(transport=HTTPTransport("user-svc"))

def app(environ, start_response):
    res = auth_client.post("/fetch-token")
    user = user_client.post("/get-user", headers={"Authorization": f"Bearer {res.text()}"})
```

## Limitations

- The transport is implemented using callbacks provided by pyvoy's server request handler. This means client
  requests cannot outlive the server request, notably fire-and-forget type of requests that create a new
  `asyncio.Task` without waiting for it in the server request flow will not work. Use a normal pyqwest
  transport for this use case.

- Envoy currently does not expose functionality to implement backpressure for HTTP clients so pyvoy's
  client also cannot offer this. If you have large streaming payloads and require backpressure, use a normal
  pyqwest transport. We hope to work with the Envoy team to provide this feature in the future.
