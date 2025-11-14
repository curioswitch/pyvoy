# pyvoy

[![License](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![CI](https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml/badge.svg)](https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml)
[![codecov](https://codecov.io/github/curioswitch/pyvoy/graph/badge.svg)](https://codecov.io/github/curioswitch/pyvoy)
[![PyPI version](https://img.shields.io/pypi/v/pyvoy)](https://pypi.org/project/pyvoy)

pyvoy is a Python application server implemented in [Envoy][]. It is based on [Envoy dynamic modules][], embedding a
Python interpreter into a module that can be loaded by a stock Envoy binary.

## Features

- ASGI applications
- WSGI applications with worker threads
- A complete, battle-tested HTTP stack - it's just Envoy
  - Includes full HTTP protocol support, with HTTP/2 trailers and HTTP/3
- Any Envoy configuration features such as load shedding can be integrated as normal

## Limitations

- Platforms limited to those supported by Envoy, which generally means glibc-based Linux on amd64/arm64 or MacOS on arm64
- Multiple worker processes. It is recommended to scale up with a higher-level orchestrator instead and use a health
  endpoint wired to RSS for automatic restarts if needed
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

## Benchmarks

We have some [preliminary benchmarks](bench/run_benchmark.py) just to understand how the approach works specifically for
HTTP/2. The main goal is to see if pyvoy runs in the same ballpark as other servers.

A single example from CI for a 10ms service with 10K response size shows:

```
Running benchmark for pyvoy with protocol=h2 sleep=10ms response_size=10000

Requests      [total, rate, throughput]         13460, 2691.78, 2686.15
Duration      [total, attack, wait]             5.011s, 5s, 10.489ms
Latencies     [min, mean, 50, 90, 95, 99, max]  9.546ms, 11.141ms, 11.066ms, 11.943ms, 12.246ms, 12.997ms, 16.798ms
Bytes In      [total, mean]                     134600000, 10000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:13460
Error Set:


Running benchmark for granian with protocol=h2 sleep=10ms response_size=10000

Requests      [total, rate, throughput]         13489, 2697.08, 2691.47
Duration      [total, attack, wait]             5.012s, 5.001s, 10.423ms
Latencies     [min, mean, 50, 90, 95, 99, max]  9.253ms, 11.111ms, 11.038ms, 11.883ms, 12.172ms, 12.946ms, 16.373ms
Bytes In      [total, mean]                     134890000, 10000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:13489
Error Set:

Running benchmark for hypercorn with protocol=h2 sleep=10ms response_size=10000

Requests      [total, rate, throughput]         1002, 183.30, 177.42
Duration      [total, attack, wait]             5.479s, 5.466s, 12.183ms
Latencies     [min, mean, 50, 90, 95, 99, max]  11.43ms, 163.65ms, 13.497ms, 16.099ms, 17.629ms, 5.019s, 5.023s
Bytes In      [total, mean]                     9720000, 9700.60
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           97.01%
Status Codes  [code:count]                      0:30  200:972
Error Set:
Get "http://localhost:8000/controlled": http2: server sent GOAWAY and closed the connection; LastStreamID=2001, ErrCode=NO_ERROR, debug=""
```

We see that hypercorn seems to not perform well with HTTP/2, with errors and resulting poor performance numbers. We will
focus comparisons on granian.

Performance seems to be mostly the same between pyvoy and granian within the range of noise for a fast but still useful
in real-world service.

We can try to isolate more performance of the app server itself with a less realistic service with no delay or response.

```
 Running benchmark for pyvoy with protocol=h2 sleep=0ms response_size=0

Requests      [total, rate, throughput]         104043, 20808.94, 20805.37
Duration      [total, attack, wait]             5.001s, 5s, 856.185µs
Latencies     [min, mean, 50, 90, 95, 99, max]  344.742µs, 1.327ms, 1.294ms, 1.736ms, 1.905ms, 2.271ms, 3.965ms
Bytes In      [total, mean]                     0, 0.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:104043
Error Set:

Running benchmark for granian with protocol=h2 sleep=0ms response_size=0

Requests      [total, rate, throughput]         96513, 19302.87, 19298.03
Duration      [total, attack, wait]             5.001s, 5s, 1.254ms
Latencies     [min, mean, 50, 90, 95, 99, max]  304.289µs, 1.501ms, 1.506ms, 1.827ms, 1.931ms, 2.2ms, 3.776ms
Bytes In      [total, mean]                     0, 0.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:96513
Error Set:
```

We again see very similar performance likely within the range of noise.

[Envoy]: https://www.Envoyproxy.io/
[Envoy dynamic modules]: https://www.Envoyproxy.io/docs/Envoy/latest/intro/arch_overview/advanced/dynamic_modules
