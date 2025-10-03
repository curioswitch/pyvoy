# pyvoy design

This document summarizes the general design of integrating a Python ASGI server into envoy.

pyvoy takes advantage of envoy's dynamic module rust SDK and PyO3's excellent rust bindings
to Python to integrate the Python interpreter into a envoy request lifecycle. The actual
execution of Python code within envoy "just works" with normal usage of the rust SDK.

To actually implement an ASGI server, HTTP events must be translated to ASGI events that
are passed to the Python program. pyvoy is a terminal filter meaning it has full control
of the response lifecycle for the request. The filter passes HTTP information to python
callbacks which are processed by the ASGI application.

Because acquiring the Python GIL to do anything in Python is a blocking operation, we
have two Python threads, the asyncio event loop itself, and a communication thread for
scheduling callbacks on it. Python code is never executed from envoy request threads.
This unfortunately means copying of the data to allow moving threads - in the future
when supporting free-threading Python, this can be optimized further.
