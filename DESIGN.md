# pyvoy design

This document summarizes the general design of integrating a Python ASGI server into envoy.

pyvoy takes advantage of envoy's dynamic module rust SDK and PyO3's excellent rust bindings
to Python to integrate the Python interpreter into a envoy request lifecycle. The actual
execution of Python code within envoy "just works" with normal usage of the rust SDK.

To actually implement an ASGI server, HTTP events must be translated to ASGI events that
are passed to the Python program. This unfortunately doesn't "just work". As envoy is
originally a proxy server, it is strongly coupled to the concept of an upstream server
that requests are sent to, with envoy logic only acting as a filter on this exchange.
As we need access to every part of the request lifecycle, headers, body, trailers on
both request / response side, we need an upstream that provides this. While envoy
does have certain upstream-less concepts, they don't provide the full lifecycle -
direct replies from a filtercannot send response body chunks or trailers, and the
direct response filter prevents request body processing from happening. The latter
notably means that it's not enough even for envoy to use a second listener in the same
process as an upstream.

As such, we use a simple [Go server](./upstream/main.go) as the upstream target of
a pyvoy filter chain. It is specifically designed to provide full access to the
request lifecycle in a way that can be controlled from the pyvoy filter. The flow
of a request then looks like this

- Filter connects to upstream
- Upstream blocks waiting for a single byte of request body
- Filter waits for the application to send a `http.response.start` event with headers
- Application sends `http.response.start`. Filter stores headers and sends a single byte
  to upstream
- Upstream returns response headers, including a `Trailer: P` header to indicate a `P`
  trailer will be sent. This forces trailer processing to be enabled by envoy.
- Filter replaces these response headers with those from the `http.response.start` event
- Any `http.response.body` events received from the application are injected into the
  response body directly. We cannot append to the response body since upstream never sends
  response body, so the filter stream does not run for it.
- When application sends the final `http.response.body`, the request body to upstream is
  closed which triggers it to send the `P` trailer. If the application indicated sending
  trailers, the filter replaces the trailer with what it sends, or otherwise ignores it
  and finishes the stream.

Notes:

- To trigger response headers from upstream, we send a request body of a single byte.
  Sending with no bytes is ignored by envoy.
- Initial attempts to `StopIteration` for response headers before application headers
  are ready did not provide reliable ordering. Triggering response headers after the
  request trigger solved it.
- All events from the application are forwarded to the filter in a channel and processed
  in on_scheduled.

Independently from the above, the request body is processed

- If the application requests to receive request body
  - If there is request body available, it is returned right away
  - Otherwise, the future is stored
- If the filter receives request body
  - If there is a pending stored future, it is returned with the request body
  - The request body is buffered for future read from a receive event

Notably, unlike response processing, request uses the standard filter stream's request
buffer.

Using an external upstream is complicated but is currently working, providing full ASGI
compatibility including having HTTP/2 bidirectional streams with trailers. We see an
expected ~1ms or so of extra latency because of it. We will see if it's possible to
change anything on the envoy side to improve this situation, ideally removing the
external upstream.
