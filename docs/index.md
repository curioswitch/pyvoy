---
title: Introduction
---

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

[Envoy]: https://www.envoyproxy.io/
[Envoy dynamic modules]: https://www.Envoyproxy.io/docs/Envoy/latest/intro/arch_overview/advanced/dynamic_modules
