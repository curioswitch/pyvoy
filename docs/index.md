#

<img alt="logo" src="./img/logo.png" width="320" height="320">
/// caption
A Python application server in Envoy
///

<div style="text-align:center">
<a href="https://opensource.org/licenses/MIT"><img alt="license" src="https://img.shields.io/badge/License-MIT-yellow.svg"></a>
<a href="https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml"><img alt="ci" src="https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml/badge.svg"></a>
<a href="https://codecov.io/github/curioswitch/pyvoy"><img alt="codecov" src="https://codecov.io/github/curioswitch/pyvoy/graph/badge.svg"></a>
<a href="https://pypi.org/project/pyvoy"><img alt="pypi version" src="https://img.shields.io/pypi/v/pyvoy"></a>
</div>

pyvoy is a Python application server implemented in [Envoy][]. It is based on [Envoy dynamic modules][], embedding a
Python interpreter into a module that can be loaded by a stock Envoy binary.

Python servers have generally lagged behind on support for modern HTTP. pyvoy utilizies the battle-hardened Envoy
to bring you all features of HTTP/2 and 3, with great performance and stability.

## Features

- ASGI and WSGI applications with worker threads
- A complete, battle-tested HTTP stack - it's just Envoy
- Any Envoy configuration features can be integrated as normal
- Auto-restart on file change and IDE debugging for development

## Limitations

- Platforms limited to those supported by Envoy, which generally means glibc-based Linux on amd64/arm64 or MacOS on arm64
- Multiple worker processes. It is recommended to scale up with a higher-level orchestrator instead and use a health
  endpoint wired to RSS for automatic restarts if needed
- Certain non-compliant requests like non-ascii query strings are prevented by Envoy itself

## Quickstart

pyvoy is available on [PyPI](https://pypi.org/project/pyvoy/) so can be installed using your favorite package manager.

=== "uv"

    ```bash
    uv add pyvoy
    ```

=== "pip"

    ```bash
    pip install pyvoy
    ```

This will also include the Envoy binary itself - there are no other steps to get running.

---

Given a simple ASGI application:

```python title="main.py"
async def app(scope, receive, send):
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [
            (b'content-type', b'text/plain'),
            (b'content-length', b'13'),
        ],
    })
    await send({
        'type': 'http.response.body',
        'body': b'Hello, world!',
        'more_body': False,
    })
```

Pass it to pyvoy to run.

=== "uv"

    ```bash
    uv run pyvoy main:app
    ```

=== "pip"

    ```bash
    pyvoy main:app
    ```

Using cURL, you can try out a request.

```bash
❯ curl http://localhost:8000/
Hello, world!
```

Flask works fine too, just make sure to pass `--interface=wsgi` when running pyvoy.

For a list of all settings

=== "uv"

    ```bash
    uv run pyvoy -h
    ```

=== "pip"

    ```bash
    pyvoy -h
    ```

or see the documentation for [settings](./settings.md).

[Envoy]: https://www.envoyproxy.io/
[Envoy dynamic modules]: https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/advanced/dynamic_modules

## Why pyvoy?

pyvoy was created out of a desire to bring HTTP/2 trailers to Python application servers to allow using the gRPC
protocol with standard applications. While developing it, we have found it to be a very fast, stable server for
any workload - notably, it is the only server known to pass all of the conformance tests for
[connect-python](https://github.com/connectrpc/connect-python).

What pyvoy isn't is a traditional Python application - we execute Envoy itself, which then loads the Python
interpreter to start the application server. This means certain features like listening on sockets in forked
processes cannot be implemented no matter how hard we try as it is in the domain of Envoy, which ensures they
are handled efficiently and stably. It doesn't mean we miss out on features like IDE debugging though, which
we do support. We also make sure to support common platforms, Linux, macOS, and Windows, though we cannot
support more than that due to lack of support in Envoy.

ASGI and WSGI make it easy to switch servers to try - hopefully you can give it a try and see if you like it.
