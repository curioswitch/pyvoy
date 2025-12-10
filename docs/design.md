# Design

pyvoy takes advantage of Envoy's dynamic modules rust SDK and PyO3's excellent Rust bindings
to Python to integrate the Python interpreter into the Envoy request lifecycle. The actual
execution of Python code withon Envoy "just works" with normal usage of the Rust SDK.

It is not enough though to just execute Python code, and we have to design around concepts
like the GIL and asyncio event loops.

## Envoy request threads and the GIL

The most important point when implementing a Python application server in a non-Python runtime
such as Tokio or Envoy is that the runtime's request handling threads are themselves async
event loops, using kernel mechanisms such as `io_uring`. Their design means it is very important
that logic on these threads does not block for any significant time - I/O cannot continue which
is essential for servers that are almost always I/O bound.

Executing any Python code however means acquiring the global interpreter lock (GIL) - this is
a blocking operation as it can take some several milliseconds for the interpreter itself to release it.
Thus, we cannot execute Python on Envoy request threads and must have at least one other thread
that acquires the GIL. We will see how this looks for ASGI and WSGI.

## ASGI

ASGI applications use `asyncio` to run on a Python event loop. At first glance, this sounds perfect,
and we will just schedule callbacks from Envoy request threads onto the event loop which would run
with the GIL. However, this does not work because event loops can only have callbacks scheduled
from threads that have the GIL, which we cannot acquire on an Envoy request thread. Thus, we have
an additional helper thread, the GIL thread, that has just one purpose - scheduling callbacks on
the Python event loop while taking the GIL. This thread detaches the GIL to listen on a Rust channel
for request information from the Envoy thread, blocking until receiving one, and then attaches the GIL
and uses `call_soon_threadsafe` to schedule actual logic on the event loop. Because acquiring the GIL
is not trivial, while the logic of just scheduling a callback is, we have an optimization to optimistically
read a limited number of events in a non-blocking way while holding the GIL. This increases throughput
while the limit ensures there is no significant impact on latency. We have to make sure we don't
overdo it since the event loop could never run if we never release the GIL from this thread.

Because the Python event loop also cannot block, it needs to be notified of I/O such as request data
or completion of response data, via `asyncio.Future`. For any invocation of `send` and `receive`, we
create a new `asyncio.Future` and return it to the application. It is passed to the Envoy request thread
which completes it when appropriate. One important point is these futures must be completed, or else
the Python side will hang forever and be effectively leaked. We accomplish this by implementing the
Rust trait `Drop` for structs that hold the futures which we send to Envoy. If the Envoy request
filter itself drops because the client disconnected, this is invoked and we complete the future with
a disconnection event. These wrappers do not use `Arc`, etc - thanks to Rust's assurances on ownership,
we are sure they are owned by the Envoy filter when we pass it, and if the filter is destroyed, it
is guaranteed to be dropped, completing the future.

## WSGI

Unlike ASGI, the Python execution under WSGI blocks on I/O. This simplifies the implementation
compared to WSGI - because of blocking, it is not enough to have a single Python thread, and we need
to have at least as much as the expected concurrency of the server - we default to 200 to match
the Java server Tomcat, but this is a user-workload driven parameter. These threads can directly
block on channels to receive events from the Envoy threads, meaning there is no additional GIL
thread. When issuing I/O such as receiving or sending payload, the I/O event is passed to Envoy
and the thread blocks to receive the reply. If the client disconnects, the channel receives will
fail with an error which we use to terminate execution of the WSGI application, closing the
response generator if needed.
