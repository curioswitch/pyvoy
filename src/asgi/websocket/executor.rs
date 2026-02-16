use std::{
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
    thread,
};

use bytes::Bytes;
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyNetworkFilterScheduler;
use http::{HeaderName, HeaderValue};
use pyo3::{
    Bound, IntoPyObjectExt as _, Py, PyAny, PyErr, PyResult, Python,
    exceptions::PyRuntimeError,
    pyclass, pymethods,
    types::{
        PyAnyMethods, PyBytes, PyDict, PyDictMethods as _, PyString, PyStringMethods as _,
        PyTracebackMethods as _,
    },
};
use tungstenite::Utf8Bytes;

use crate::{
    asgi::shared::{
        ExecutorHandles,
        app::load_app,
        awaitable::{EmptyAwaitable, ErrorAwaitable},
        eventloop::EventLoops,
        get_gil_batch_size,
        headers::extract_headers_from_event,
        scope::new_scope_dict,
    },
    eventbridge::EventBridge,
    types::{ClientDisconnectedError, Constants, Scope, SyncReceiver},
};

struct ExecutorInner {
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    loops: EventLoops,
    constants: Arc<Constants>,
    executor: WebSocketExecutor,
}

#[derive(Clone)]
pub(super) struct WebSocketExecutor {
    tx: Sender<Event>,
}

impl WebSocketExecutor {
    pub(super) fn new(
        app_module: &str,
        app_attr: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_lifespan: Option<bool>,
    ) -> PyResult<(Self, ExecutorHandles)> {
        let (app, asgi, loops) = load_app(
            app_module,
            app_attr,
            &constants,
            worker_threads,
            enable_lifespan,
        )?;
        let extensions = Python::attach(|py| PyDict::new(py).unbind());
        let (tx, rx) = mpsc::channel::<Event>();
        let executor = Self { tx };

        let inner = ExecutorInner {
            app,
            asgi,
            extensions,
            loops: loops.clone(),
            constants,
            executor: executor.clone(),
        };

        let gil_handle = thread::spawn(move || {
            let rx = SyncReceiver::new(rx);
            let gil_batch_size = get_gil_batch_size();
            let mut shutdown = false;
            if let Err(e) = Python::attach(|py| {
                while let Ok(event) = py.detach(|| rx.recv()) {
                    if !inner.handle_event(py, event)? {
                        shutdown = true;
                    } else {
                        // We don't want to be too agressive and starve the event loop but do
                        // suffer significantly reduced throughput when acquiring the GIL each
                        // event since the operations are very fast (we just schedule on the
                        // event loop without actually executing logic). We go ahead and
                        // process any more events that may be present up to a cap to improve
                        // throughput while still having a relatively low upper bound on time.
                        for event in rx.try_iter().take(gil_batch_size) {
                            if !inner.handle_event(py, event)? {
                                shutdown = true;
                                break;
                            }
                        }
                    }
                    if shutdown {
                        break;
                    }
                }

                Ok::<_, PyErr>(())
            }) {
                eprintln!(
                    "Unexpected Python exception in ASGI executor thread. This likely a bug in pyvoy: {}",
                    e
                );
            }
        });
        Ok((executor, ExecutorHandles { loops, gil_handle }))
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        recv_bridge: EventBridge<RecvFuture>,
        send_bridge: EventBridge<SendEvent>,
        scheduler: Box<dyn EnvoyNetworkFilterScheduler>,
    ) {
        self.tx
            .send(Event::ExecuteApp(ExecuteAppEvent {
                scope: WebSocketScope {
                    scope,
                    subprotocols: vec![],
                },
                recv_bridge,
                send_bridge,
                scheduler,
            }))
            .unwrap();
    }

    pub(crate) fn handle_recv_future_connect(&self, future: RecvFuture) {
        self.tx
            .send(Event::HandleRecvFutureConnect(future))
            .unwrap();
    }

    pub(crate) fn handle_recv_future_message(&self, body: Body, future: RecvFuture) {
        self.tx
            .send(Event::HandleRecvFutureMessage { body, future })
            .unwrap();
    }

    pub(crate) fn handle_recv_future_disconnect(
        &self,
        code: u16,
        reason: Utf8Bytes,
        future: RecvFuture,
    ) {
        self.tx
            .send(Event::HandleRecvFutureDisconnect {
                code,
                reason,
                future,
            })
            .unwrap();
    }

    pub(crate) fn handle_dropped_recv_future(&self, future: LoopFuture) {
        self.tx
            .send(Event::HandleDroppedRecvFuture(future))
            .unwrap();
    }

    pub(crate) fn handle_send_future(&self, future: SendFuture) {
        self.tx.send(Event::HandleSendFuture(future)).unwrap();
    }

    pub(crate) fn handle_dropped_send_future(&self, future: LoopFuture) {
        self.tx
            .send(Event::HandleDroppedSendFuture(future))
            .unwrap();
    }

    pub(crate) fn shutdown(&self) {
        self.tx.send(Event::Shutdown).unwrap();
    }
}

impl ExecutorInner {
    fn handle_event<'py>(&self, py: Python<'py>, event: Event) -> PyResult<bool> {
        match event {
            Event::ExecuteApp(event) => {
                let (loop_, state) = self.loops.get(py);
                let app_executor = AppExecutor {
                    loop_: loop_.clone().unbind(),
                    state: state.map(|s| s.unbind()),
                    app: self.app.clone_ref(py),
                    asgi: self.asgi.clone_ref(py),
                    extensions: self.extensions.clone_ref(py),
                    event: Some(event),
                    constants: self.constants.clone(),
                    executor: self.executor.clone(),
                };
                loop_.call_method1(&self.constants.call_soon_threadsafe, (app_executor,))?;
            }
            Event::HandleRecvFutureConnect(mut future) => {
                if let Some(future) = future.future.take() {
                    let connect_future_executor = ConnectFutureExecutor {
                        future: future.future,
                        constants: self.constants.clone(),
                    };
                    future.loop_.call_method1(
                        py,
                        &self.constants.call_soon_threadsafe,
                        (connect_future_executor,),
                    )?;
                }
            }
            Event::HandleRecvFutureMessage { body, mut future } => {
                if let Some(future) = future.future.take() {
                    let recv_future_executor = RecvFutureMessageExecutor {
                        body,
                        future: future.future,
                        constants: self.constants.clone(),
                    };
                    future.loop_.call_method1(
                        py,
                        &self.constants.call_soon_threadsafe,
                        (recv_future_executor,),
                    )?;
                }
            }
            Event::HandleRecvFutureDisconnect {
                code,
                reason,
                mut future,
            } => {
                if let Some(future) = future.future.take() {
                    let recv_future_executor = RecvFutureDisconnectExecutor {
                        code,
                        reason,
                        future: future.future,
                        constants: self.constants.clone(),
                    };
                    future.loop_.call_method1(
                        py,
                        &self.constants.call_soon_threadsafe,
                        (recv_future_executor,),
                    )?;
                }
            }
            Event::HandleDroppedRecvFuture(future) => {
                let dropped_recv_future_executor = DroppedRecvFutureExecutor {
                    future: future.future,
                    constants: self.constants.clone(),
                };
                future.loop_.call_method1(
                    py,
                    &self.constants.call_soon_threadsafe,
                    (dropped_recv_future_executor,),
                )?;
            }
            // No real logic in handling send futures so we don't bother with
            // running on the event loop.
            Event::HandleSendFuture(future) => {
                self.handle_send_future(py, future)?;
            }
            Event::HandleDroppedSendFuture(future) => {
                self.handle_dropped_send_future(py, future)?;
            }
            Event::Shutdown => {
                self.shutdown(py)?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn handle_send_future<'py>(&self, py: Python<'py>, mut future: SendFuture) -> PyResult<()> {
        let f = match future.future.take() {
            Some(f) => f,
            None => {
                return Ok(());
            }
        };
        let set_result = f.future.getattr(py, &self.constants.set_result)?;
        f.loop_.call_method1(
            py,
            &self.constants.call_soon_threadsafe,
            (set_result, py.None()),
        )?;
        Ok(())
    }

    fn handle_dropped_send_future<'py>(&self, py: Python<'py>, future: LoopFuture) -> PyResult<()> {
        let set_exception = future.future.getattr(py, &self.constants.set_exception)?;
        future.loop_.call_method1(
            py,
            &self.constants.call_soon_threadsafe,
            (set_exception, &self.constants.client_disconnected_err),
        )?;
        Ok(())
    }

    fn shutdown<'py>(&self, py: Python<'py>) -> PyResult<()> {
        self.loops.stop(py)?;
        Ok(())
    }
}

/// The callable scheduled on the event loop to execute the ASGI application.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct AppExecutor {
    loop_: Py<PyAny>,
    state: Option<Py<PyDict>>,
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    event: Option<ExecuteAppEvent>,
    constants: Arc<Constants>,
    executor: WebSocketExecutor,
}

#[pymethods]
impl AppExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let ExecuteAppEvent {
            scope,
            recv_bridge,
            send_bridge,
            scheduler,
        } = self.event.take().unwrap();

        let loop_ = self.loop_.bind(py);

        let scope_dict = new_scope_dict(
            py,
            scope.scope,
            &self.constants.websocket,
            &self.asgi,
            &self.extensions,
            &self.state,
            &self.constants,
        )?;

        let scheduler = Arc::new(scheduler);

        let recv = RecvCallable {
            recv_bridge,
            scheduler: scheduler.clone(),
            loop_: loop_.clone().unbind(),
            executor: self.executor.clone(),
            constants: self.constants.clone(),
        }
        .into_bound_py_any(py)?;

        let coro = self.app.bind(py).call1((
            scope_dict,
            recv,
            SendCallable {
                state: SendState::Started,
                send_bridge: send_bridge.clone(),
                scheduler: scheduler.clone(),
                loop_: loop_.clone().unbind(),
                executor: self.executor.clone(),
                constants: self.constants.clone(),
            },
        ))?;
        let future = loop_.call_method1(&self.constants.create_task, (coro,))?;
        future.call_method1(
            &self.constants.add_done_callback,
            (AppFutureHandler {
                send_bridge,
                scheduler,
                constants: self.constants.clone(),
            },),
        )?;

        Ok(())
    }
}

#[pyclass(module = "_pyvoy.asgi.websocket")]
struct ConnectFutureExecutor {
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

#[pymethods]
impl ConnectFutureExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.websocket_connect)?;
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

/// The callable scheduled on the event loop to handle a receive future.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct RecvFutureMessageExecutor {
    body: Body,
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

#[pymethods]
impl RecvFutureMessageExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
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

/// The callable scheduled on the event loop to handle a receive future.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct RecvFutureDisconnectExecutor {
    code: u16,
    reason: Utf8Bytes,
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

#[pymethods]
impl RecvFutureDisconnectExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.websocket_disconnect)?;
        event.set_item(&self.constants.code, self.code)?;
        event.set_item(&self.constants.reason, PyString::new(py, &self.reason))?;
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

/// The callable scheduled on the event loop to handle a dropped receive future.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct DroppedRecvFutureExecutor {
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

#[pymethods]
impl DroppedRecvFutureExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
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

enum Event {
    ExecuteApp(ExecuteAppEvent),
    HandleRecvFutureConnect(RecvFuture),
    HandleRecvFutureMessage {
        body: Body,
        future: RecvFuture,
    },
    HandleRecvFutureDisconnect {
        code: u16,
        reason: Utf8Bytes,
        future: RecvFuture,
    },
    HandleDroppedRecvFuture(LoopFuture),
    HandleSendFuture(SendFuture),
    HandleDroppedSendFuture(LoopFuture),
    Shutdown,
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
        }
        Ok(())
    }
}

enum SendState {
    Started,
    Accepted,
    Closed,
}

/// The send callable passed to ASGI applications.
#[pyclass(module = "_pyvoy.asgi.websocket")]
struct SendCallable {
    state: SendState,
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
                    self.state = SendState::Accepted;
                    let ret = self
                        .loop_
                        .bind(py)
                        .call_method0(&self.constants.create_future)?;

                    if self
                        .send_bridge
                        .send(SendEvent::Accept(AcceptEvent {
                            subprotocol: None,
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
                SendState::Closed => unreachable!(),
            },
            "websocket.send" => {
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
                self.state = SendState::Closed;
                let code = if let Some(code) = event.get_item(&self.constants.code)? {
                    code.extract::<u16>()?
                } else {
                    1000
                };
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

pub(super) struct AcceptEvent {
    pub(super) subprotocol: Option<Box<str>>,
    pub(super) headers: Vec<(HeaderName, HeaderValue)>,
    pub(super) future: SendFuture,
}

pub(super) struct SendMessageEvent {
    pub(super) body: Body,
    pub(super) future: SendFuture,
}

/// An event to send a response, read by the filter when scheduling [`EVENT_ID_RESPONSE`].
pub(super) enum SendEvent {
    /// Accept the WebSocket connection.
    Accept(AcceptEvent),
    /// Sends a WebSocket message.
    SendMessage(SendMessageEvent),
    /// Closes the WebSocket connection.
    Close { code: u16, reason: Utf8Bytes },
    /// Complete the response with failure due to an application exception.
    Exception,
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
