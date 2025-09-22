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

Requests      [total, rate, throughput]         3172, 634.13, 632.44
Duration      [total, attack, wait]             5.015s, 5.002s, 13.346ms
Latencies     [min, mean, 50, 90, 95, 99, max]  11.082ms, 15.677ms, 15.377ms, 18.017ms, 19.703ms, 29.42ms, 35.404ms
Bytes In      [total, mean]                     3172000, 1000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:3172
Error Set:

Running benchmark for granian with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         3321, 663.73, 661.97
Duration      [total, attack, wait]             5.017s, 5.004s, 13.263ms
Latencies     [min, mean, 50, 90, 95, 99, max]  10.618ms, 14.843ms, 14.368ms, 17.228ms, 19.564ms, 26.045ms, 30.606ms
Bytes In      [total, mean]                     3321000, 1000.00
Bytes Out     [total, mean]                     0, 0.00
Success       [ratio]                           100.00%
Status Codes  [code:count]                      200:3321
Error Set:

Running benchmark for hypercorn with sleep=10ms response_size=1000

Requests      [total, rate, throughput]         1011, 148.78, 146.94
Duration      [total, attack, wait]             6.812s, 6.795s, 17.312ms
Latencies     [min, mean, 50, 90, 95, 99, max]  12.401ms, 67.199ms, 17.644ms, 21.632ms, 23.505ms, 2.213s, 5.019s
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
