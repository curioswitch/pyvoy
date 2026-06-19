use std::{
    sync::{Arc, atomic::AtomicBool},
    thread::{self},
};

use crate::{
    asgi::{
        shared::{
            ExecutorHandles,
            app::load_app,
            awaitable::{EmptyAwaitable, ErrorAwaitable, ValueAwaitable},
            eventloop::EventLoops,
            get_gil_batch_size,
            headers::extract_headers_from_event,
            scope::new_scope_dict,
        },
        transport::{
            ReceivedResponseHeadersExecutor, RequestContent, ResetReason, ResponseContent,
            SetResponseFutureException, TransportBridge, TransportEvent,
        },
    },
    eventbridge::EventBridge,
    ondrop::RunOnDrop,
    types::{ClientDisconnectedError, Constants, Scope, SyncReceiver},
};
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyNone, PyString},
};
use std::sync::mpsc;
use std::sync::mpsc::Sender;

struct ExecutorInner {
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    root_path: Py<PyString>,
    loops: EventLoops,
    constants: Arc<Constants>,
    executor: Executor,
}

/// An executor of Python code.
///
/// We separate execution of Python code from Rust code because the Python GIL
/// must be held when executing Python code, and we don't want to potentially
/// block on it from envoy request threads.
///
/// We use two threads, one for the asyncio event loop and one to schedule tasks
/// on it. The task scheduler does some marshaling of Python dictionaries while
/// holding the GIL and then schedules execution of the actual tasks on the
/// asyncio event loop thread.
#[derive(Clone)]
pub(crate) struct Executor {
    tx: Sender<Event>,
    constants: Arc<Constants>,
}

impl Executor {
    pub(crate) fn new(
        app_module: &str,
        app_attr: &str,
        root_path: &str,
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

        let (extensions, root_path) = Python::attach(|py| {
            let extensions = PyDict::new(py);
            extensions.set_item("http.response.trailers", PyDict::new(py))?;
            let root_path = PyString::new(py, root_path);
            Ok::<_, PyErr>((extensions.unbind(), root_path.unbind()))
        })?;

        let (tx, rx) = mpsc::channel::<Event>();
        let executor = Self {
            tx,
            constants: constants.clone(),
        };

        let inner = ExecutorInner {
            app,
            asgi,
            extensions,
            root_path,
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
        request_closed: bool,
        trailers_accepted: bool,
        response_closed: Arc<AtomicBool>,
        recv_bridge: EventBridge<RecvFuture>,
        send_bridge: EventBridge<SendEvent>,
        transport_bridge: EventBridge<TransportEvent>,
        scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    ) {
        self.tx
            .send(Event::ExecuteApp(ExecuteAppEvent {
                scope,
                request_closed,
                trailers_accepted,
                response_closed,
                recv_bridge,
                send_bridge,
                transport_bridge,
                scheduler,
            }))
            .unwrap();
    }

    pub(crate) fn handle_recv_future(&self, body: Box<[u8]>, more_body: bool, future: RecvFuture) {
        self.tx
            .send(Event::HandleRecvFuture {
                body,
                more_body,
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

    pub(crate) fn handle_transport_received_response_headers(
        &self,
        status: StatusCode,
        response_headers: Vec<(HeaderName, HeaderValue)>,
        response_content: ResponseContent,
        response_future: Py<PyAny>,
        request_content: Option<RequestContent>,
        end_stream: bool,
    ) {
        let stream_start_executor = ReceivedResponseHeadersExecutor::new(
            status,
            response_headers,
            response_future,
            response_content,
            request_content,
            end_stream,
            self.constants.clone(),
        );
        self.tx
            .send(Event::HandleTransportReceivedResponseHeaders(
                stream_start_executor,
            ))
            .unwrap();
    }

    pub(crate) fn set_response_future_exception(
        &self,
        future: Py<PyAny>,
        reason: ResetReason,
        response_content: ResponseContent,
        request_content: Option<RequestContent>,
    ) {
        self.tx
            .send(Event::SetResponseFutureException(
                future,
                reason,
                response_content,
                request_content,
            ))
            .unwrap();
    }

    pub(crate) fn notify_request(&self, request_content: RequestContent) {
        self.tx.send(Event::NotifyRequest(request_content)).unwrap();
    }

    pub(crate) fn notify_response(&self, response_content: ResponseContent) {
        self.tx
            .send(Event::NotifyResponse(response_content))
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
                    root_path: self.root_path.clone_ref(py),
                    event: Some(event),
                    constants: self.constants.clone(),
                    executor: self.executor.clone(),
                };
                loop_.call_method1(&self.constants.call_soon_threadsafe, (app_executor,))?;
            }
            Event::HandleRecvFuture {
                body,
                more_body,
                mut future,
            } => {
                if let Some(future) = future.future.take() {
                    let recv_future_executor = RecvFutureExecutor {
                        body,
                        more_body,
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
            // No real logic in handling send or canceled futures so we don't bother with
            // running on the event loop.
            Event::HandleSendFuture(future) => {
                self.handle_send_future(py, future)?;
            }
            Event::HandleDroppedSendFuture(future) => {
                self.handle_dropped_send_future(py, future)?;
            }
            Event::HandleTransportReceivedResponseHeaders(executor) => {
                let loop_ = executor.response_content.inner.loop_.bind(py).clone();
                loop_.call_method1(
                    &self.constants.call_soon_threadsafe,
                    (executor.into_py_any(py)?,),
                )?;
            }
            Event::SetResponseFutureException(
                future,
                reason,
                response_content,
                request_content,
            ) => {
                let loop_ = response_content.inner.loop_.bind(py).clone();
                loop_.call_method1(
                    &self.constants.call_soon_threadsafe,
                    (SetResponseFutureException {
                        future,
                        reason,
                        exception: response_content.take_exception(),
                        request_content,
                        constants: self.constants.clone(),
                    },),
                )?;
            }
            Event::NotifyRequest(content) => {
                let loop_ = content.inner.loop_.bind(py).clone();
                loop_.call_method1(
                    &self.constants.call_soon_threadsafe,
                    (content.into_py_any(py)?,),
                )?;
            }
            Event::NotifyResponse(content) => {
                let loop_ = content.inner.loop_.bind(py).clone();
                loop_.call_method1(
                    &self.constants.call_soon_threadsafe,
                    (content.into_py_any(py)?,),
                )?;
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
            (set_result, PyNone::get(py)),
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
#[pyclass(module = "_pyvoy.asgi")]
struct AppExecutor {
    loop_: Py<PyAny>,
    state: Option<Py<PyDict>>,
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    root_path: Py<PyString>,
    event: Option<ExecuteAppEvent>,
    constants: Arc<Constants>,
    executor: Executor,
}

#[pymethods]
impl AppExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let ExecuteAppEvent {
            scope,
            request_closed,
            trailers_accepted,
            response_closed,
            recv_bridge,
            send_bridge,
            transport_bridge,
            scheduler,
        } = self.event.take().unwrap();

        let loop_ = self.loop_.bind(py);

        let scope_dict = new_scope_dict(
            py,
            scope,
            &self.constants.http,
            &self.asgi,
            &self.extensions,
            Some(&self.root_path),
            &self.state,
            &self.constants,
        )?;

        let scheduler = Arc::new(scheduler);

        let recv = if request_closed {
            EmptyRecvCallable {
                response_closed,
                constants: self.constants.clone(),
            }
            .into_bound_py_any(py)?
        } else {
            RecvCallable {
                recv_bridge,
                scheduler: scheduler.clone(),
                loop_: loop_.clone().unbind(),
                executor: self.executor.clone(),
                constants: self.constants.clone(),
            }
            .into_bound_py_any(py)?
        };

        let coro = self.app.bind(py).call1((
            scope_dict,
            recv,
            SendCallable {
                next_event: NextASGIEvent::Start,
                response_start: None,
                trailers_accepted,
                closed: false,
                send_bridge: send_bridge.clone(),
                scheduler: scheduler.clone(),
                loop_: loop_.clone().unbind(),
                executor: self.executor.clone(),
                constants: self.constants.clone(),
            },
        ))?;

        let transport_bridge =
            TransportBridge::new(loop_.clone().unbind(), transport_bridge, scheduler.clone())
                .into_py_any(py)?;
        let token = self
            .constants
            .transport_bridge_contextvar_set
            .call1(py, (transport_bridge,))?;

        let future = {
            let _reset = RunOnDrop(
                self.constants
                    .transport_bridge_contextvar_reset
                    .bind(py)
                    .clone(),
                Some((token,)),
            );
            loop_.call_method1(&self.constants.create_task, (coro,))?
        };
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

/// The callable scheduled on the event loop to handle a receive future.
#[pyclass(module = "_pyvoy.asgi")]
struct RecvFutureExecutor {
    body: Box<[u8]>,
    more_body: bool,
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

#[pymethods]
impl RecvFutureExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.http_request)?;
        event.set_item(&self.constants.body, PyBytes::new(py, &self.body))?;
        event.set_item(&self.constants.more_body, self.more_body)?;
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

/// The callable scheduled on the event loop to handle a dropped receive future.
#[pyclass(module = "_pyvoy.asgi")]
struct DroppedRecvFutureExecutor {
    future: Py<PyAny>,
    constants: Arc<Constants>,
}

#[pymethods]
impl DroppedRecvFutureExecutor {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<()> {
        let event = self.constants.asgi_empty_recv_disconnect.bind(py).copy()?;
        self.future
            .call_method1(py, &self.constants.set_result, (event,))?;
        Ok(())
    }
}

struct ExecuteAppEvent {
    scope: Scope,
    request_closed: bool,
    trailers_accepted: bool,
    response_closed: Arc<AtomicBool>,
    recv_bridge: EventBridge<RecvFuture>,
    send_bridge: EventBridge<SendEvent>,
    transport_bridge: EventBridge<TransportEvent>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
}

enum Event {
    ExecuteApp(ExecuteAppEvent),
    HandleRecvFuture {
        body: Box<[u8]>,
        more_body: bool,
        future: RecvFuture,
    },
    HandleDroppedRecvFuture(LoopFuture),
    HandleSendFuture(SendFuture),
    HandleDroppedSendFuture(LoopFuture),
    HandleTransportReceivedResponseHeaders(ReceivedResponseHeadersExecutor),
    SetResponseFutureException(
        Py<PyAny>,
        ResetReason,
        ResponseContent,
        Option<RequestContent>,
    ),
    NotifyRequest(RequestContent),
    NotifyResponse(ResponseContent),
    Shutdown,
}

/// The receive callable we pass to the application.
#[pyclass(module = "_pyvoy.asgi")]
struct RecvCallable {
    recv_bridge: EventBridge<RecvFuture>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    loop_: Py<PyAny>,
    executor: Executor,
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

/// The receive callable we pass to the application when the request was closed
/// with headers, usually a GET request.
#[pyclass(module = "_pyvoy.asgi")]
struct EmptyRecvCallable {
    response_closed: Arc<AtomicBool>,
    constants: Arc<Constants>,
}

#[pymethods]
impl EmptyRecvCallable {
    fn __call__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let event = if self
            .response_closed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.constants.asgi_empty_recv_disconnect.bind(py).copy()?
        } else {
            self.constants.asgi_empty_recv.bind(py).copy()?
        };
        ValueAwaitable::new_py(py, &event.into_any())
    }
}

/// The done callback to run when the ASGI application completes.
#[pyclass(module = "_pyvoy.asgi")]
struct AppFutureHandler {
    send_bridge: EventBridge<SendEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
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
            let tb = e.traceback(py).unwrap().format().unwrap_or_default();
            eprintln!("Exception in ASGI application\n{}{}", tb, e);
            if self.send_bridge.send(SendEvent::Exception).is_ok() {
                self.scheduler.commit(EVENT_ID_RESPONSE);
            }
        }
        Ok(())
    }
}

enum NextASGIEvent {
    Start,
    Body { trailers: bool },
    Trailers,
    Done,
}

/// The send callable passed to ASGI applications.
#[pyclass(module = "_pyvoy.asgi")]
struct SendCallable {
    next_event: NextASGIEvent,
    response_start: Option<ResponseStartEvent>,
    trailers_accepted: bool,
    closed: bool,
    send_bridge: EventBridge<SendEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    loop_: Py<PyAny>,
    executor: Executor,
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
        if self.closed {
            return ErrorAwaitable::new_py(py, ClientDisconnectedError::new_err(()));
        }

        let event_type_py = event
            .get_item(&self.constants.typ)?
            .ok_or_else(|| PyRuntimeError::new_err("Unexpected ASGI message, missing 'type'."))?;
        let event_type = event_type_py.cast::<PyString>()?.to_str()?;

        match &self.next_event {
            NextASGIEvent::Start => {
                if event_type != "http.response.start" {
                    return ErrorAwaitable::new_py(
                        py,
                        PyRuntimeError::new_err(format!(
                            "Expected ASGI message 'http.response.start', but got '{}'.",
                            event_type
                        )),
                    );
                }
                let headers = extract_headers_from_event(py, &self.constants, &event)?;
                let status_code = match event.get_item(&self.constants.status)? {
                    Some(v) => v.extract::<u16>()?,
                    None => {
                        return Err(PyRuntimeError::new_err(
                            "Unexpected ASGI message, missing 'status' in 'http.response.start'.",
                        ));
                    }
                };
                let status = StatusCode::from_u16(status_code).map_err(|_| {
                    PyValueError::new_err(format!("Invalid HTTP status code '{}'", status_code))
                })?;

                let trailers: bool = match event.get_item(&self.constants.trailers)? {
                    Some(v) => v.extract()?,
                    None => false,
                };
                self.next_event = NextASGIEvent::Body { trailers };
                self.response_start.replace(ResponseStartEvent {
                    status,
                    headers,
                    trailers: trailers && self.trailers_accepted,
                });
                EmptyAwaitable::new_py(py)
            }
            NextASGIEvent::Body { trailers } => {
                if event_type != "http.response.body" {
                    return ErrorAwaitable::new_py(
                        py,
                        PyRuntimeError::new_err(format!(
                            "Expected ASGI message 'http.response.body', but got '{}'.",
                            event_type
                        )),
                    );
                }
                let more_body: bool = match event.get_item(&self.constants.more_body)? {
                    Some(v) => v.extract()?,
                    None => false,
                };
                let body: Box<[u8]> = match event.get_item(&self.constants.body)? {
                    Some(body) => Box::from(body.cast::<PyBytes>()?.as_bytes()),
                    _ => Box::default(),
                };
                if !more_body {
                    self.next_event = if *trailers {
                        NextASGIEvent::Trailers
                    } else {
                        NextASGIEvent::Done
                    }
                }
                let (ret, send_future) = if more_body {
                    let future = self
                        .loop_
                        .bind(py)
                        .call_method0(&self.constants.create_future)?;
                    (
                        future.clone(),
                        Some(SendFuture {
                            future: Some(LoopFuture {
                                future: future.unbind(),
                                loop_: self.loop_.clone_ref(py),
                            }),
                            executor: self.executor.clone(),
                        }),
                    )
                } else {
                    (EmptyAwaitable::new_py(py)?, None)
                };
                let body_event = ResponseBodyEvent {
                    body,
                    future: send_future,
                };
                if let Some(start_event) = self.response_start.take() {
                    if self
                        .send_bridge
                        .send(SendEvent::Start(start_event, body_event))
                        .is_ok()
                    {
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    } else {
                        // send_future will be completed with exception when dropped if needed.
                        self.closed = true;
                    }
                } else if self.send_bridge.send(SendEvent::Body(body_event)).is_ok() {
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                } else {
                    // send_future will be completed with exception when dropped if needed.
                    self.closed = true;
                }
                Ok(ret)
            }
            NextASGIEvent::Trailers => {
                if event_type != "http.response.trailers" {
                    return ErrorAwaitable::new_py(
                        py,
                        PyRuntimeError::new_err(format!(
                            "Expected ASGI message 'http.response.trailers', but got '{}'.",
                            event_type
                        )),
                    );
                }
                let more_trailers: bool = match event.get_item(&self.constants.more_trailers)? {
                    Some(v) => v.extract()?,
                    None => false,
                };
                if !more_trailers {
                    self.next_event = NextASGIEvent::Done;
                }
                if self.trailers_accepted {
                    let headers = extract_headers_from_event(py, &self.constants, &event)?;
                    if self
                        .send_bridge
                        .send(SendEvent::Trailers(ResponseTrailersEvent {
                            headers,
                            more_trailers,
                        }))
                        .is_ok()
                    {
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    } else {
                        self.closed = true;
                        return ErrorAwaitable::new_py(py, ClientDisconnectedError::new_err(()));
                    }
                }
                EmptyAwaitable::new_py(py)
            }
            NextASGIEvent::Done => ErrorAwaitable::new_py(
                py,
                PyRuntimeError::new_err(format!(
                    "Unexpected ASGI message '{}' sent, after response already completed.",
                    event_type
                )),
            ),
        }
    }
}

/// An event to begin a response with headers.
pub(crate) struct ResponseStartEvent {
    /// The status code of the response.
    pub status: StatusCode,
    /// The headers of the response.
    pub headers: Vec<(HeaderName, HeaderValue)>,
    /// Whether trailers will be sent after the body.
    pub trailers: bool,
}

/// An event to send a chunk of the response body.
pub(crate) struct ResponseBodyEvent {
    /// The body bytes to send.
    pub body: Box<[u8]>,
    // Future to notify when the body is completed writing.
    // Not sent for the final piece of body, so None indicates
    // more_body = False.
    pub future: Option<SendFuture>,
}

/// An event to send response trailers.
pub(crate) struct ResponseTrailersEvent {
    /// The trailers.
    pub headers: Vec<(HeaderName, HeaderValue)>,
    /// Whether more trailers will follow.
    pub more_trailers: bool,
}

/// An event to send a response, read by the filter when scheduling [`EVENT_ID_RESPONSE`].
pub(crate) enum SendEvent {
    /// Start the response with headers and the first body chunk.
    Start(ResponseStartEvent, ResponseBodyEvent),
    /// Send a chunk of the response body.
    Body(ResponseBodyEvent),
    /// Send response trailers.
    Trailers(ResponseTrailersEvent),
    /// Complete the response with failure due to an application exception.
    Exception,
}

/// A Python future to notify when a receive completes.
pub(crate) struct RecvFuture {
    future: Option<LoopFuture>,
    executor: Executor,
}

impl Drop for RecvFuture {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            self.executor.handle_dropped_recv_future(future);
        }
    }
}

/// A Python future with its associated event loop for easy access from Rust.
pub(crate) struct LoopFuture {
    pub(crate) future: Py<PyAny>,
    pub(crate) loop_: Py<PyAny>,
}

/// A Python future to notify when a send completes.
pub(crate) struct SendFuture {
    future: Option<LoopFuture>,
    executor: Executor,
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
/// The event ID to trigger an outgoing HTTP request.
pub(crate) const EVENT_ID_OUTGOING_REQUEST: u64 = 3;
