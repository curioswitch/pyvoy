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

[envoy]: https://www.envoyproxy.io/
[envoy dynamic modules]: https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/advanced/dynamic_modules

## Benchmarks

We have some [preliminary benchmarks](bench/run_benchmark.py) just to understand how the approach works specifically for
HTTP/2. The main goal is to see if pyvoy runs in the same ballpark as other servers.

A single example from the [full set of results](bench/example_result.txt) from a Mac laptop for a 10ms service shows:

```
Running benchmark for pyvoy with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         3254, 650.43, 647.50
Duration      [total, attack, wait]             5.025s, 5.003s, 22.667ms
Latencies     [min, mean, 50, 90, 95, 99, max]  11.082ms, 15.193ms, 14.717ms, 17.759ms, 19.189ms, 25.022ms, 27.323ms
Bytes In      [total, mean]                     3254000, 1000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:3254
Error Set:

Running benchmark for granian with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         3851, 769.24, 767.32
Duration      [total, attack, wait]             5.019s, 5.006s, 12.523ms
Latencies     [min, mean, 50, 90, 95, 99, max]  10.121ms, 12.877ms, 12.466ms, 14.382ms, 15.295ms, 19.708ms, 22.028ms
Bytes In      [total, mean]                     3851000, 1000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:3851
Error Set:

Running benchmark for hypercorn with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         1011, 150.98, 149.20
Duration      [total, attack, wait]             6.709s, 6.696s, 13.094ms
Latencies     [min, mean, 50, 90, 95, 99, max]  11.983ms, 66.209ms, 16.884ms, 19.179ms, 20.055ms, 2.801s, 5.019s
Bytes In      [total, mean]                     1001000, 990.11
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           99.01%
Status Codes  [code:count]                      0:10  200:1001
Error Set:
Get "http://localhost:8000/controlled": http2: server sent GOAWAY and closed the connection; LastStreamID=2019, ErrCode=NO_ERROR, debug=""
```

We see that hypercorn seems to not perform well with HTTP/2, with errors and resulting poor performance numbers. We will
focus comparisons on granian.

We see a seemingly consistent ~1ms slowdown in pyvoy vs granian. This matches the expected latency hit from using an
external upstream as described in the [design](./DESIGN.md). Notably, this means for the `sleep=0ms` case, the performance
is significantly worse with pyvoy, but for meaningful applications, it should be competitive. We will see if it's possible
to work on the limitation in envoy going forward.
