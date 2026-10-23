from __future__ import annotations

import json
from typing import TYPE_CHECKING

from opentelemetry import trace
from opentelemetry.instrumentation.asgi import OpenTelemetryMiddleware
from opentelemetry.sdk.trace import ReadableSpan, TracerProvider
from opentelemetry.sdk.trace.sampling import ALWAYS_ON, ParentBased

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope

# The SDK would otherwise read OTEL_TRACES_SAMPLER, which configures Envoy in
# the tests, while the application should follow the propagated sampled flag.
trace.set_tracer_provider(TracerProvider(sampler=ParentBased(ALWAYS_ON)))


# Reports the trace context the application was given and the span its
# instrumentation started from it.
async def _app(
    scope: Scope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] != "http":
        return
    headers = dict(scope["headers"])
    span = trace.get_current_span()
    span_context = span.get_span_context()
    parent = span.parent if isinstance(span, ReadableSpan) else None
    body = json.dumps(
        {
            "traceparent": headers.get(b"traceparent", b"").decode() or None,
            "tracestate": headers.get(b"tracestate", b"").decode() or None,
            "trace_id": format(span_context.trace_id, "032x"),
            "parent_span_id": format(parent.span_id, "016x") if parent else None,
            "sampled": span_context.trace_flags.sampled,
        }
    ).encode()
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [
                (b"content-type", b"application/json"),
                (b"content-length", str(len(body)).encode()),
            ],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": body, "more_body": False})


app = OpenTelemetryMiddleware(_app)
