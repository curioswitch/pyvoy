# pyvoy

[![License](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![CI](https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml/badge.svg)](https://github.com/curioswitch/pyvoy/actions/workflows/ci.yaml)
[![codecov](https://codecov.io/github/curioswitch/pyvoy/graph/badge.svg)](https://codecov.io/github/curioswitch/pyvoy)
[![PyPI version](https://img.shields.io/pypi/v/pyvoy)](https://pypi.org/project/pyvoy)

pyvoy is a Python application server, based on [envoy][]. It is based on [envoy dynamic modules][], embedding a
Python interpreter into a module that can be loaded by a stock envoy binary.

## Features

- ASGI applications
- WSGI applications with worker threads (WIP, basic applications should work)
- Full HTTP protocol support, including HTTP/2 trailers and HTTP/3
- Any envoy configuration features such as authentication can be integrated as normal

## Limitations

- Platforms limited to those supported by envoy, which generally means glibc-based Linux on amd64/arm64 or MacOS on arm64
- Multiple worker processes. It is recommended to scale up with a higher-level orchestrator instead

## Benchmarks

We have some [preliminary benchmarks](bench/run_benchmark.py) just to understand how the approach works specifically for
HTTP/2. The main goal is to see if pyvoy runs in the same ballpark as other servers.

A single example from the [full set of results](bench/example_result.txt) from a Mac laptop for a 10ms service shows:

```
Running benchmark for pyvoy with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         3311, 661.50, 659.51
Duration      [total, attack, wait]             5.02s, 5.005s, 15.116ms
Latencies     [min, mean, 50, 90, 95, 99, max]  10.605ms, 14.779ms, 14.401ms, 17.044ms, 18.505ms, 22.736ms, 27.81ms
Bytes In      [total, mean]                     3311000, 1000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:3311
Error Set:

Running benchmark for granian with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         3472, 693.31, 690.92
Duration      [total, attack, wait]             5.025s, 5.008s, 17.367ms
Latencies     [min, mean, 50, 90, 95, 99, max]  10.647ms, 14.215ms, 13.515ms, 17.359ms, 19.724ms, 23.372ms, 27.866ms
Bytes In      [total, mean]                     3472000, 1000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:3472
Error Set:

Running benchmark for hypercorn with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         1011, 150.66, 148.82
Duration      [total, attack, wait]             6.726s, 6.71s, 15.608ms
Latencies     [min, mean, 50, 90, 95, 99, max]  11.883ms, 66.338ms, 16.483ms, 19.045ms, 21.296ms, 2.229s, 5.022s
Bytes In      [total, mean]                     1001000, 990.11
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           99.01%
Status Codes  [code:count]                      0:10  200:1001
Error Set:
Get "http://localhost:8000/controlled": http2: server sent GOAWAY and closed the connection; LastStreamID=2019, ErrCode=NO_ERROR, debug=""
```

We see that hypercorn seems to not perform well with HTTP/2, with errors and resulting poor performance numbers. We will
focus comparisons on granian.

Performance seems to be mostly the same between pyvoy and granian within the range of noise.

[envoy]: https://www.envoyproxy.io/
[envoy dynamic modules]: https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/advanced/dynamic_modules
