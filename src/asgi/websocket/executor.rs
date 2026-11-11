use std::sync::Arc;

use bytes::Bytes;
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyNetworkFilterScheduler;
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::{
    Bound, IntoPyObjectExt as _, Py, PyAny, PyErr, PyResult, Python,
    exceptions::{PyRuntimeError, PyValueError},
    pyclass, pymethods,
    types::{
        PyAnyMethods, PyBytes, PyDict, PyDictMethods as _, PyList, PyString, PyStringMethods as _,
        PyTracebackMethods as _,
    },
};
use tungstenite::{Utf8Bytes, protocol::frame::coding::CloseCode};

use crate::{
    asgi::shared::{
        ExecutorHandles,
        app::load_app,
        awaitable::{EmptyAwaitable, ErrorAwaitable},
        eventloop::{EventLoops, LoopHandle, LoopKind},
        headers::extract_headers_from_event,
        scope::new_scope_dict,
    },
    eventbridge::EventBridge,
    types::{ClientDisconnectedError, Constants, Scope},
};

struct ExecutorInner {
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    root_path: Py<PyString>,
    loops: EventLoops,
    constants: Arc<Constants>,
}

#[derive(Clone)]
pub(super) struct WebSocketExecutor {
    inner: Arc<ExecutorInner>,
}

impl WebSocketExecutor {
    pub(super) fn new(
        app_module: &str,
        app_attr: &str,
        root_path: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_lifespan: Option<bool>,
        loop_kind: LoopKind,
    ) -> PyResult<(Self, ExecutorHandles)> {
        let (app, asgi, loops) = load_app(
            app_module,
            app_attr,
            &constants,
            worker_threads,
            enable_lifespan,
            loop_kind,
        )?;
        let (extensions, root_path) = Python::attach(|py| {
            let extensions = PyDict::new(py);
            extensions.set_item("websocket.http.response", PyDict::new(py))?;
            Ok::<_, PyErr>((extensions.unbind(), PyString::new(py, root_path).unbind()))
        })?;
        let executor = Self {
            inner: Arc::new(ExecutorInner {
                app,
                asgi,
                extensions,
                root_path,
                loops: loops.clone(),
                constants,
            }),
        };
        Ok((executor, ExecutorHandles { loops }))
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        subprotocols: Vec<String>,
        recv_bridge: EventBridge<RecvFuture>,
        send_bridge: EventBridge<SendEvent>,
        scheduler: Box<dyn EnvoyNetworkFilterScheduler>,
    ) {
        let event = ExecuteAppEvent {
            scope: WebSocketScope {
                scope,
                subprotocols,
            },
            recv_bridge,
            send_bridge,
            scheduler,
        };
        let executor = self.clone();
        self.inner
            .loops
            .next()
            .dispatch(Box::new(move |py, handle| {
                executor.run_app(py, handle, event)
            }));
    }

    pub(crate) fn handle_recv_future_connect(&self, mut future: RecvFuture) {
        let Some(future) = future.future.take() else {
            return;
        };
        let executor = ConnectFutureExecutor {
            future: future.future,
            constants: self.inner.constants.clone(),
        };
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| executor.run(py)));
    }

    pub(crate) fn handle_recv_future_message(&self, body: Body, mut future: RecvFuture) {
        let Some(future) = future.future.take() else {
            return;
        };
        let executor = RecvFutureMessageExecutor {
            body,
            future: future.future,
            constants: self.inner.constants.clone(),
        };
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| executor.run(py)));
    }

    pub(crate) fn handle_recv_future_disconnect(
        &self,
        code: u16,
        reason: Utf8Bytes,
        mut future: RecvFuture,
    ) {
        let Some(future) = future.future.take() else {
            return;
        };
        let executor = RecvFutureDisconnectExecutor {
            code,
            reason,
            future: future.future,
            constants: self.inner.constants.clone(),
        };
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| executor.run(py)));
    }

    pub(crate) fn handle_dropped_recv_future(&self, future: LoopFuture) {
        let executor = DroppedRecvFutureExecutor {
            future: future.future,
            constants: self.inner.constants.clone(),
        };
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| executor.run(py)));
    }

    pub(crate) fn handle_send_future(&self, mut future: SendFuture) {
        let Some(future) = future.future.take() else {
            return;
        };
        let constants = self.inner.constants.clone();
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| {
                future
                    .future
                    .call_method1(py, &constants.set_result, (py.None(),))?;
                Ok(())
            }));
    }

    pub(crate) fn handle_dropped_send_future(&self, future: LoopFuture) {
        let constants = self.inner.constants.clone();
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| {
                future.future.call_method1(
                    py,
                    &constants.set_exception,
                    (&constants.client_disconnected_err,),
                )?;
                Ok(())
            }));
    }

    /// Stops the event loops, executing shutdown lifespan if needed.
    pub(crate) fn shutdown(&self) {
        Python::attach(|py| {
            if let Err(e) = self.inner.loops.stop(py) {
                eprintln!(
                    "Unexpected Python exception stopping ASGI event loops. This likely a bug in pyvoy: {}",
                    e
                );
            }
        });
    }

    /// Executes the ASGI application on the event loop thread.
    fn run_app(&self, py: Python<'_>, handle: &LoopHandle, event: ExecuteAppEvent) -> PyResult<()> {
        let ExecuteAppEvent {
            scope,
            recv_bridge,
            send_bridge,
            scheduler,
        } = event;
        let inner = self.inner.as_ref();
        let constants = &inner.constants;

        let loop_ = handle.loop_.bind(py);

        let scope_dict = new_scope_dict(
            py,
            scope.scope,
            &constants.websocket,
            &inner.asgi,
            &inner.extensions,
            Some(&inner.root_path),
            handle.state(),
            constants,
        )?;
        scope_dict.set_item(
            &constants.subprotocols,
            PyList::new(py, scope.subprotocols)?,
        )?;

        let scheduler = Arc::new(scheduler);

        let recv = RecvCallable {
            recv_bridge,
            scheduler: scheduler.clone(),
            loop_: loop_.clone().unbind(),
            executor: self.clone(),
            constants: constants.clone(),
        }
        .into_bound_py_any(py)?;

        let coro = inner.app.bind(py).call1((
            scope_dict,
            recv,
            SendCallable {
                state: SendState::Started,
                response_start: None,
                send_bridge: send_bridge.clone(),
                scheduler: scheduler.clone(),
                loop_: loop_.clone().unbind(),
                executor: self.clone(),
                constants: constants.clone(),
            },
        ))?;
        let future = loop_.call_method1(&constants.create_task, (coro,))?;
        future.call_method1(
            &constants.add_done_callback,
            (AppFutureHandler {
                send_bridge,
                scheduler,
                constants: constants.clone(),
            },),
        )?;

        Ok(())
    }
}

struct ConnectFutureExecutor {
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

impl ConnectFutureExecutor {
    fn run(&self, py: Python<'_>) -> PyResult<()> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.websocket_connect)?;
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

/// Completes a receive future with a message, on the event loop thread.
struct RecvFutureMessageExecutor {
    body: Body,
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

impl RecvFutureMessageExecutor {
    fn run(&self, py: Python<'_>) -> PyResult<()> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.websocket_receive)?;
        match &self.body {
            Body::Bytes(bytes) => {
                event.set_item(&self.constants.bytes, PyBytes::new(py, bytes))?;
            }
            Body::Text(s) => {
                event.set_item(&self.constants.text, PyString::new(py, s))?;
            }
        }
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

/// Completes a receive future with a disconnect, on the event loop thread.
struct RecvFutureDisconnectExecutor {
    code: u16,
    reason: Utf8Bytes,
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

impl RecvFutureDisconnectExecutor {
    fn run(&self, py: Python<'_>) -> PyResult<()> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.websocket_disconnect)?;
        event.set_item(&self.constants.code, self.code)?;
        event.set_item(&self.constants.reason, PyString::new(py, &self.reason))?;
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

/// Completes a dropped receive future with a disconnect, on the event loop thread.
struct DroppedRecvFutureExecutor {
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

impl DroppedRecvFutureExecutor {
    fn run(&self, py: Python<'_>) -> PyResult<()> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.websocket_disconnect)?;
        event.set_item(&self.constants.code, 1005u16)?;
        event.set_item(&self.constants.reason, &self.constants.empty_string)?;
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

struct ExecuteAppEvent {
    scope: WebSocketScope,
    recv_bridge: EventBridge<RecvFuture>,
    send_bridge: EventBridge<SendEvent>,
    scheduler: Box<dyn EnvoyNetworkFilterScheduler>,
}

struct WebSocketScope {
    scope: Scope,
    subprotocols: Vec<String>,
}

pub(super) enum Body {
    Bytes(Bytes),
    Text(Utf8Bytes),
}

/// The receive callable we pass to the application.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct RecvCallable {
    recv_bridge: EventBridge<RecvFuture>,
    scheduler: Arc<Box<dyn EnvoyNetworkFilterScheduler>>,
    loop_: Py<PyAny>,
    executor: WebSocketExecutor,
    constants: Arc<Constants>,
}

unsafe impl Sync for RecvCallable {}

#[pymethods]
impl RecvCallable {
    fn __call__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let future = self
            .loop_
            .bind(py)
            .call_method0(&self.constants.create_future)?;
        let recv_future = RecvFuture {
            future: Some(LoopFuture {
                future: future.clone().unbind(),
                loop_: self.loop_.clone_ref(py),
            }),
            executor: self.executor.clone(),
        };
        if self.recv_bridge.send(recv_future).is_ok() {
            self.scheduler.commit(EVENT_ID_REQUEST);
        }
        Ok(future)
    }
}

/// The done callback to run when the ASGI application completes.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct AppFutureHandler {
    send_bridge: EventBridge<SendEvent>,
    scheduler: Arc<Box<dyn EnvoyNetworkFilterScheduler>>,
    constants: Arc<Constants>,
}

unsafe impl Sync for AppFutureHandler {}

#[pymethods]
impl AppFutureHandler {
    fn __call__<'py>(&mut self, py: Python<'py>, future: Bound<'py, PyAny>) -> PyResult<()> {
        if let Err(e) = future.call_method0(&self.constants.result) {
            if e.is_instance_of::<ClientDisconnectedError>(py) {
                return Ok(());
            }
            let tb = e
                .traceback(py)
                .map(|tb| tb.format().unwrap_or_default())
                .unwrap_or_default();
            eprintln!("Exception in ASGI application\n{}{}", tb, e);
            if self.send_bridge.send(SendEvent::Exception).is_ok() {
                self.scheduler.commit(EVENT_ID_RESPONSE);
            }
        } else if self.send_bridge.send(SendEvent::Abort).is_ok() {
            // The application returned without closing the connection. Match
            // uvicorn: drop the connection rather than sending a close frame,
            // since a compliant app is expected to close explicitly.
            self.scheduler.commit(EVENT_ID_RESPONSE);
        }
        Ok(())
    }
}

enum SendState {
    Started,
    Accepted,
    /// The application started a denial response with `websocket.http.response.start`.
    Responding,
    Closed,
}

/// The send callable passed to ASGI applications.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct SendCallable {
    state: SendState,
    /// The start of a denial response, held until its first body message so the
    /// filter can set the content length when the body is sent in one message.
    response_start: Option<ResponseStartEvent>,
    send_bridge: EventBridge<SendEvent>,
    scheduler: Arc<Box<dyn EnvoyNetworkFilterScheduler>>,
    loop_: Py<PyAny>,
    executor: WebSocketExecutor,
    constants: Arc<Constants>,
}

unsafe impl Sync for SendCallable {}

#[pymethods]
impl SendCallable {
    fn __call__<'py>(
        &mut self,
        py: Python<'py>,
        event: Bound<'py, PyDict>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if matches!(self.state, SendState::Closed) {
            return ErrorAwaitable::new_py(py, ClientDisconnectedError::new_err(()));
        }

        let event_type_py = event
            .get_item(&self.constants.typ)?
            .ok_or_else(|| PyRuntimeError::new_err("Unexpected ASGI message, missing 'type'."))?;
        let event_type = event_type_py.cast::<PyString>()?.to_str()?;
        match event_type {
            "websocket.accept" => match &self.state {
                SendState::Started => {
                    let headers = extract_headers_from_event(py, &self.constants, &event)?;
                    let subprotocol = match event.get_item(&self.constants.subprotocol)? {
                        Some(value) if !value.is_none() => {
                            Some(value.cast::<PyString>()?.to_str()?.into())
                        }
                        _ => None,
                    };
                    self.state = SendState::Accepted;
                    let ret = self
                        .loop_
                        .bind(py)
                        .call_method0(&self.constants.create_future)?;

                    if self
                        .send_bridge
                        .send(SendEvent::Accept(AcceptEvent {
                            subprotocol,
                            headers,
                            future: SendFuture {
                                future: Some(LoopFuture {
                                    future: ret.clone().unbind(),
                                    loop_: self.loop_.clone_ref(py),
                                }),
                                executor: self.executor.clone(),
                            },
                        }))
                        .is_ok()
                    {
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    }

                    Ok(ret)
                }
                SendState::Accepted => ErrorAwaitable::new_py(
                    py,
                    PyRuntimeError::new_err("WebSocket connection has already been accepted."),
                ),
                SendState::Responding => expected_response_body(py, event_type),
                SendState::Closed => unreachable!(),
            },
            "websocket.send" => {
                match self.state {
                    SendState::Started => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyRuntimeError::new_err(
                                "Expected ASGI message 'websocket.accept', but got 'websocket.send'.",
                            ),
                        );
                    }
                    SendState::Responding => return expected_response_body(py, event_type),
                    SendState::Accepted | SendState::Closed => {}
                }
                let bytes = event
                    .get_item(&self.constants.bytes)?
                    .unwrap_or_else(|| py.None().bind(py).clone());
                let body = if bytes.is_none() {
                    let text = event
                        .get_item(&self.constants.text)?
                        .unwrap_or_else(|| py.None().bind(py).clone());

                    if text.is_none() {
                        return Err(PyRuntimeError::new_err(
                            "websocket.send event must have either 'bytes' or 'text' field.",
                        ));
                    }
                    let text_str = text.cast::<PyString>()?;
                    Body::Text(text_str.to_str()?.into())
                } else {
                    Body::Bytes(bytes.extract::<Bytes>()?)
                };

                let ret = self
                    .loop_
                    .bind(py)
                    .call_method0(&self.constants.create_future)?;
                if self
                    .send_bridge
                    .send(SendEvent::SendMessage(SendMessageEvent {
                        body,
                        future: SendFuture {
                            future: Some(LoopFuture {
                                future: ret.clone().unbind(),
                                loop_: self.loop_.clone_ref(py),
                            }),
                            executor: self.executor.clone(),
                        },
                    }))
                    .is_ok()
                {
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                }
                Ok(ret)
            }
            "websocket.close" => {
                if matches!(self.state, SendState::Responding) {
                    return expected_response_body(py, event_type);
                }
                let code = if let Some(code) = event.get_item(&self.constants.code)? {
                    code.extract::<u16>()?
                } else {
                    1000
                };
                if !CloseCode::from(code).is_allowed() {
                    return ErrorAwaitable::new_py(
                        py,
                        PyValueError::new_err(format!("invalid WebSocket close code: {code}")),
                    );
                }
                self.state = SendState::Closed;
                let reason: Utf8Bytes =
                    if let Some(reason) = event.get_item(&self.constants.reason)? {
                        if reason.is_none() {
                            Utf8Bytes::default()
                        } else {
                            reason.extract::<&str>()?.into()
                        }
                    } else {
                        Utf8Bytes::default()
                    };
                if self
                    .send_bridge
                    .send(SendEvent::Close { code, reason })
                    .is_ok()
                {
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                }
                EmptyAwaitable::new_py(py)
            }
            "websocket.http.response.start" => {
                match self.state {
                    SendState::Started => {}
                    SendState::Accepted => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyRuntimeError::new_err(
                                "WebSocket connection has already been accepted.",
                            ),
                        );
                    }
                    SendState::Responding => return expected_response_body(py, event_type),
                    SendState::Closed => unreachable!(),
                }
                let status_code = match event.get_item(&self.constants.status)? {
                    Some(v) => v.extract::<u16>()?,
                    None => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyRuntimeError::new_err(
                                "Unexpected ASGI message, missing 'status' in 'websocket.http.response.start'.",
                            ),
                        );
                    }
                };
                let status = match StatusCode::from_u16(status_code) {
                    Ok(status) if !status.is_informational() => status,
                    _ => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyValueError::new_err(format!(
                                "Invalid HTTP status code '{status_code}'"
                            )),
                        );
                    }
                };
                let headers = extract_headers_from_event(py, &self.constants, &event)?;
                self.state = SendState::Responding;
                self.response_start = Some(ResponseStartEvent { status, headers });
                EmptyAwaitable::new_py(py)
            }
            "websocket.http.response.body" => {
                match self.state {
                    SendState::Responding => {}
                    SendState::Started => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyRuntimeError::new_err(
                                "Expected ASGI message 'websocket.http.response.start', but got 'websocket.http.response.body'.",
                            ),
                        );
                    }
                    SendState::Accepted => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyRuntimeError::new_err(
                                "Expected ASGI message 'websocket.send' or 'websocket.close', but got 'websocket.http.response.body'.",
                            ),
                        );
                    }
                    SendState::Closed => unreachable!(),
                }
                let body = match event.get_item(&self.constants.body)? {
                    Some(body) => body.extract::<Bytes>()?,
                    None => Bytes::new(),
                };
                let more_body: bool = match event.get_item(&self.constants.more_body)? {
                    Some(v) => v.extract()?,
                    None => false,
                };
                let (ret, future) = if more_body {
                    let ret = self
                        .loop_
                        .bind(py)
                        .call_method0(&self.constants.create_future)?;
                    let future = SendFuture {
                        future: Some(LoopFuture {
                            future: ret.clone().unbind(),
                            loop_: self.loop_.clone_ref(py),
                        }),
                        executor: self.executor.clone(),
                    };
                    (ret, Some(future))
                } else {
                    self.state = SendState::Closed;
                    (EmptyAwaitable::new_py(py)?, None)
                };
                let body = ResponseBodyEvent { body, future };
                let event = match self.response_start.take() {
                    Some(start) => SendEvent::ResponseStart(start, body),
                    None => SendEvent::ResponseBody(body),
                };
                if self.send_bridge.send(event).is_ok() {
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                }
                Ok(ret)
            }
            _ => ErrorAwaitable::new_py(
                py,
                PyRuntimeError::new_err(format!(
                    "Unexpected ASGI message of type '{}'.",
                    event_type
                )),
            ),
        }
    }
}

fn expected_response_body<'py>(py: Python<'py>, event_type: &str) -> PyResult<Bound<'py, PyAny>> {
    ErrorAwaitable::new_py(
        py,
        PyRuntimeError::new_err(format!(
            "Expected ASGI message 'websocket.http.response.body', but got '{event_type}'."
        )),
    )
}

pub(super) struct AcceptEvent {
    pub(super) subprotocol: Option<Box<str>>,
    pub(super) headers: Vec<(HeaderName, HeaderValue)>,
    pub(super) future: SendFuture,
}

pub(super) struct SendMessageEvent {
    pub(super) body: Body,
    pub(super) future: SendFuture,
}

pub(super) struct ResponseStartEvent {
    pub(super) status: StatusCode,
    pub(super) headers: Vec<(HeaderName, HeaderValue)>,
}

pub(super) struct ResponseBodyEvent {
    pub(super) body: Bytes,
    /// Completed once the body is written. Set when more body follows and
    /// unset for the final body.
    pub(super) future: Option<SendFuture>,
}

/// An event to send a response, read by the filter when scheduling [`EVENT_ID_RESPONSE`].
pub(super) enum SendEvent {
    /// Accept the WebSocket connection.
    Accept(AcceptEvent),
    /// Sends a WebSocket message.
    SendMessage(SendMessageEvent),
    /// Closes the WebSocket connection.
    Close { code: u16, reason: Utf8Bytes },
    /// Denies the WebSocket connection with an HTTP response, with its first body.
    ResponseStart(ResponseStartEvent, ResponseBodyEvent),
    /// Sends more body of an HTTP response denying the WebSocket connection.
    ResponseBody(ResponseBodyEvent),
    /// Complete the response with failure due to an application exception.
    Exception,
    /// The application returned without closing; drop the connection.
    Abort,
}

/// A Python future to notify when a receive completes.
pub(super) struct RecvFuture {
    future: Option<LoopFuture>,
    executor: WebSocketExecutor,
}

impl Drop for RecvFuture {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            self.executor.handle_dropped_recv_future(future);
        }
    }
}

/// A Python future with its associated event loop for easy access from Rust.
pub(super) struct LoopFuture {
    future: Py<PyAny>,
    loop_: Py<PyAny>,
}

/// A Python future to notify when a send completes.
pub(super) struct SendFuture {
    future: Option<LoopFuture>,
    executor: WebSocketExecutor,
}

impl Drop for SendFuture {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            self.executor.handle_dropped_send_future(future);
        }
    }
}

/// The event ID to trigger the filter to process request events.
pub(crate) const EVENT_ID_REQUEST: u64 = 1;
/// The event ID to trigger the filter to process response events.
pub(crate) const EVENT_ID_RESPONSE: u64 = 2;
