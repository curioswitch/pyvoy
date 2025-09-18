# pyvoy

pyvoy is a Python application server, based on [envoy][]. It is based on [envoy dynamic modules][], embedding a
Python interpreter into a module that can be loaded by a stock envoy binary.

## Features

- ASGI applications
- Full HTTP protocol support, including HTTP/2 trailers
- Any envoy configuration features such as authentication can be integrated as normal

## Limitations

- Platforms limited to those supported by envoy, which generally means glibc-based Linux or MacOS
- WSGI applications (coming soon)
- Multiple worker threads (coming soon)
- Multiple worker processes. It is recommended to scale up with a higher-level orchestrator instead.

[envoy][https://www.envoyproxy.io/]
[envoy dynamic modules][https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/advanced/dynamic_modules]
