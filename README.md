# pyvoy

[![License](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![CI](https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml/badge.svg)](https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml)
[![codecov](https://codecov.io/github/curioswitch/pyvoy/graph/badge.svg)](https://codecov.io/github/curioswitch/pyvoy)
[![PyPI version](https://img.shields.io/pypi/v/pyvoy)](https://pypi.org/project/pyvoy)

pyvoy is a Python application server implemented in [Envoy][]. It is based on [Envoy dynamic modules][], embedding a
Python interpreter into a module that can be loaded by a stock Envoy binary.

## Features

- ASGI and WSGI applications
- Worker threads for both, can be useful with free-threaded Python for ASGI
- A complete, battle-tested HTTP stack - it's just Envoy
  - Includes full HTTP protocol support, with HTTP/2 trailers and HTTP/3
- Any Envoy configuration features can be integrated as normal
  - It can be more performant to offload features like CORS or content encoding to Envoy
- Auto-restart on file change and IDE debugging for development

## Limitations

- Platforms limited to those supported by Envoy, which generally means glibc-based Linux on amd64/arm64, MacOS on arm64
  and unofficial support for Windows on amd64
- No support for multiple worker processes. It is recommended to scale up with a higher-level orchestrator instead and
  use a health endpoint wired to RSS for automatic restarts if needed
- Certain non-compliant requests are prevented by Envoy itself
  - The full URL path, including query string, must be ASCII percent-encoded

## Installation

pyvoy is published as a wheel that includes both the dynamic module and Envoy itself. You can use it in the same way
as any other app server.

```bash
uv add pyvoy # or pip install
```

## Running

pyvoy includes a CLI which supports standard options for HTTP servers. If just passing a `module:attr` name to point
to an application, it will be served on plaintext on port 8000.

```bash
uv run pyvoy my.module:app
```

(if the application is named exactly `app`, `:app` can be omitted)

To see a full list of options:

```bash
uv run pyvoy -h
```

### Docker

For production deployments to containers, we recommend running Envoy directly without the pyvoy CLI to avoid potential
issues with subprocess spawning. The pyvoy CLI simply spawns Envoy with an appropriate YAML config and environment
variables for loading the dynamic module. You can see the example [Dockerfile](./example/docker/) for how to set up
the config and environment for running Envoy directly.

Note that the pyvoy CLI with `--print-envoy-config` is run within the Dockerfile to easily set up the config. This is
convenient for simple cases and should run well for normal deployments. But for experienced Envoy users that want to
configure other aspects of Envoy, we also recommend managing the Envoy config in your codebase and adding it to the
container - you can then tweak any and all Envoy parameters to meet your needs.

## Development

We use [poe](https://poethepoet.natn.io/) for running development tasks. For a list of tasks, you can run

```bash
uv run poe -h
```

During development, the most common commands will be

```bash
uv run poe test # Run unit tests
uv run poe format # Apply possible formatting
uv run poe check # Run all checks. If this passes, CI should pass
uv run poe build # Only build pyvoy. Needed if running tests from IDE
```

## Benchmarks

We have some [preliminary benchmarks](bench/run_benchmark.py) just to understand how the approach works specifically for
HTTP/2. The main goal is to see if pyvoy runs in the same ballpark as other servers.

A single example from CI for a 10ms service with 10K response size shows:

```

Running benchmark for pyvoy with interface=asgi protocol=h2 sleep=10ms response_size=100

Requests      [total, rate, throughput]         13957, 2790.41, 2784.89
Duration      [total, attack, wait]             5.012s, 5.002s, 9.904ms
Latencies     [min, mean, 50, 90, 95, 99, max]  9.281ms, 10.715ms, 10.679ms, 11.392ms, 11.594ms, 11.944ms, 13.676ms
Bytes In      [total, mean]                     1395700, 100.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:13957
Error Set:


Running benchmark for granian with interface=asgi protocol=h2 sleep=10ms response_size=10000

Requests      [total, rate, throughput]         13753, 2750.32, 2744.50
Duration      [total, attack, wait]             5.011s, 5.001s, 10.595ms
Latencies     [min, mean, 50, 90, 95, 99, max]  9.272ms, 10.894ms, 10.839ms, 11.614ms, 11.891ms, 12.615ms, 16.173ms
Bytes In      [total, mean]                     137530000, 10000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:13753
Error Set:

 Running benchmark for hypercorn with interface=asgi protocol=h2 sleep=10ms response_size=10000

Requests      [total, rate, throughput]         1003, 183.39, 177.51
Duration      [total, attack, wait]             5.481s, 5.469s, 11.985ms
Latencies     [min, mean, 50, 90, 95, 99, max]  10.283ms, 163.568ms, 13.266ms, 17.517ms, 18.66ms, 5.02s, 5.023s
Bytes In      [total, mean]                     9730000, 9700.90
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           97.01%
Status Codes  [code:count]                      0:30  200:973
Error Set:
Get "http://localhost:8000/controlled": http2: server sent GOAWAY and closed the connection; LastStreamID=2003, ErrCode=NO_ERROR, debug=""
```

We see that hypercorn seems to not perform well with HTTP/2, with errors and resulting poor performance numbers. We will
focus comparisons on granian.

Performance seems to be mostly the same between pyvoy and granian within the range of noise for a fast but still useful
in real-world service. Slower services will see even less of a difference.

We can try to isolate more performance of the app server itself with a less realistic service with no delay or response.

```
Running benchmark for pyvoy with interface=asgi protocol=h2 sleep=0ms response_size=0

Requests      [total, rate, throughput]         160777, 32154.15, 32152.39
Duration      [total, attack, wait]             5s, 5s, 272.72µs
Latencies     [min, mean, 50, 90, 95, 99, max]  160.187µs, 847.224µs, 815.162µs, 1.143ms, 1.287ms, 1.601ms, 2.736ms
Bytes In      [total, mean]                     0, 0.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:160777
Error Set:

Running benchmark for granian with interface=asgi protocol=h2 sleep=0ms response_size=0

Requests      [total, rate, throughput]         135538, 27108.58, 27105.86
Duration      [total, attack, wait]             5s, 5s, 501.885µs
Latencies     [min, mean, 50, 90, 95, 99, max]  160.356µs, 1.053ms, 1.042ms, 1.306ms, 1.418ms, 1.782ms, 4.128ms
Bytes In      [total, mean]                     0, 0.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:135538
Error Set:
```

pyvoy may be showing somewhat better performance. In this test, much time will be marshaling the HTTP
protocol itself and we may be benefitting from Envoy's battle-hardened HTTP/2 stack.

More [charts](./bench/charts) are available to see performance under various configurations.

[Envoy]: https://www.envoyproxy.io/
[Envoy dynamic modules]: https://www.envoyproxy.io/docs/Envoy/latest/intro/arch_overview/advanced/dynamic_modules
