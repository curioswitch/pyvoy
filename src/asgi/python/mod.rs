use std::sync::{Arc, atomic::AtomicBool};

use crate::{
    asgi::{
        shared::{
            ExecutorHandles,
            app::load_app,
            awaitable::{EmptyAwaitable, ErrorAwaitable, ValueAwaitable},
            eventloop::{EventLoops, LoopHandle, LoopKind},
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
    types::{ClientDisconnectedError, Constants, Scope},
};
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyNone, PyString},
};

/// An executor of Python code.
///
/// We separate execution of Python code from Rust code because the Python GIL
/// must be held when executing Python code, and we don't want to potentially
/// block on it from envoy request threads.
///
/// Envoy threads hand work to the event loop threads without the GIL, and the
/// event loops run it while attached to the interpreter.
#[derive(Clone)]
pub(crate) struct Executor {
    inner: Arc<ExecutorInner>,
}

struct ExecutorInner {
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    root_path: Py<PyString>,
    /// Holds the asyncio.Task class, which starts applications as eager tasks.
    /// An eager task runs the application until it first waits, so a response
    /// that needs no I/O completes without another pass through the event loop.
    /// It is set only for asyncio event loops on Python 3.12 or later.
    eager_task: Option<Py<PyAny>>,
    loops: EventLoops,
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

        let (extensions, root_path, eager_task) = Python::attach(|py| {
            let extensions = PyDict::new(py);
            extensions.set_item("http.response.trailers", PyDict::new(py))?;
            let root_path = PyString::new(py, root_path);
            let eager_task = if loop_kind != LoopKind::Trio && py.version_info() >= (3, 12) {
                Some(py.import("asyncio")?.getattr("Task")?.unbind())
            } else {
                None
            };
            Ok::<_, PyErr>((extensions.unbind(), root_path.unbind(), eager_task))
        })?;

        let executor = Self {
            inner: Arc::new(ExecutorInner {
                app,
                asgi,
                extensions,
                root_path,
                eager_task,
                loops: loops.clone(),
                constants,
            }),
        };
        Ok((executor, ExecutorHandles { loops }))
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        request_closed: bool,
        trailers_accepted: bool,
        response_closed: Arc<AtomicBool>,
        recv_bridge: EventBridge<RecvFuture>,
        send_bridge: EventBridge<SendEvent>,
        transport_bridge: Option<EventBridge<TransportEvent>>,
        scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    ) {
        let event = ExecuteAppEvent {
            scope,
            request_closed,
            trailers_accepted,
            response_closed,
            recv_bridge,
            send_bridge,
            transport_bridge,
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

    pub(crate) fn handle_recv_future(
        &self,
        body: Box<[u8]>,
        more_body: bool,
        mut future: RecvFuture,
    ) {
        let Some(future) = future.future.take() else {
            return;
        };
        let constants = self.inner.constants.clone();
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| {
                let event = PyDict::new(py);
                event.set_item(&constants.typ, &constants.http_request)?;
                event.set_item(&constants.body, PyBytes::new(py, &body))?;
                event.set_item(&constants.more_body, more_body)?;
                future
                    .future
                    .call_method1(py, &constants.set_result, (event,))?;
                Ok(())
            }));
    }

    pub(crate) fn handle_dropped_recv_future(&self, future: LoopFuture) {
        let constants = self.inner.constants.clone();
        self.inner
            .loops
            .find(&future.loop_)
            .dispatch(Box::new(move |py, _| {
                let event = constants.asgi_empty_recv_disconnect.bind(py).copy()?;
                future
                    .future
                    .call_method1(py, &constants.set_result, (event,))?;
                Ok(())
            }));
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
                    .call_method1(py, &constants.set_result, (PyNone::get(py),))?;
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

    pub(crate) fn handle_transport_received_response_headers(
        &self,
        status: StatusCode,
        response_headers: Vec<(HeaderName, HeaderValue)>,
        response_content: ResponseContent,
        response_future: Py<PyAny>,
        request_content: Option<RequestContent>,
        end_stream: bool,
    ) {
        let handle = self.inner.loops.find(&response_content.inner.loop_);
        let executor = ReceivedResponseHeadersExecutor::new(
            status,
            response_headers,
            response_future,
            response_content,
            request_content,
            end_stream,
            self.inner.constants.clone(),
        );
        handle.dispatch(Box::new(move |py, _| executor.run(py)));
    }

    pub(crate) fn set_response_future_exception(
        &self,
        future: Py<PyAny>,
        reason: ResetReason,
        response_content: ResponseContent,
        request_content: Option<RequestContent>,
    ) {
        let constants = self.inner.constants.clone();
        self.inner
            .loops
            .find(&response_content.inner.loop_)
            .dispatch(Box::new(move |py, _| {
                SetResponseFutureException {
                    future,
                    reason,
                    exception: response_content.take_exception(),
                    request_content,
                    constants,
                }
                .run(py)
            }));
    }

    pub(crate) fn notify_request(&self, request_content: RequestContent) {
        self.inner
            .loops
            .find(&request_content.inner.loop_)
            .dispatch(Box::new(move |py, _| request_content.start_forwarding(py)));
    }

    pub(crate) fn notify_response(&self, response_content: ResponseContent) {
        self.inner
            .loops
            .find(&response_content.inner.loop_)
            .dispatch(Box::new(move |py, _| response_content.notify(py)));
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
            request_closed,
            trailers_accepted,
            response_closed,
            recv_bridge,
            send_bridge,
            transport_bridge,
            scheduler,
        } = event;
        let inner = self.inner.as_ref();
        let constants = &inner.constants;

        let loop_ = handle.loop_.bind(py);

        let scope_dict = new_scope_dict(
            py,
            scope,
            &constants.http,
            &inner.asgi,
            &inner.extensions,
            Some(&inner.root_path),
            handle.state(),
            constants,
        )?;

        let scheduler = Arc::new(scheduler);

        let recv = if request_closed {
            EmptyRecvCallable {
                response_closed,
                constants: constants.clone(),
            }
            .into_bound_py_any(py)?
        } else {
            RecvCallable {
                recv_bridge,
                scheduler: scheduler.clone(),
                loop_: loop_.clone().unbind(),
                executor: self.clone(),
                constants: constants.clone(),
            }
            .into_bound_py_any(py)?
        };

        let coro = inner.app.bind(py).call1((
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
                executor: self.clone(),
                constants: constants.clone(),
            },
        ))?;

        // A task factory set by the application decides how its tasks are
        // created, so tasks only start eagerly without one.
        let eager = match &inner.eager_task {
            Some(eager_task) if loop_.call_method0(&constants.get_task_factory)?.is_none() => {
                Some(eager_task)
            }
            _ => None,
        };
        let task = {
            // The task copies the current context, so the bridge only needs to be
            // set while creating it. Without an HTTP client, we skip touching the
            // context entirely.
            let _reset = match transport_bridge {
                Some(transport_bridge) => {
                    let transport_bridge = TransportBridge::new(
                        loop_.clone().unbind(),
                        transport_bridge,
                        scheduler.clone(),
                    )
                    .into_py_any(py)?;
                    let token = constants
                        .transport_bridge_contextvar_set
                        .call1(py, (transport_bridge,))?;
                    Some(RunOnDrop(
                        constants.transport_bridge_contextvar_reset.bind(py).clone(),
                        Some((token,)),
                    ))
                }
                None => None,
            };
            match eager {
                Some(eager_task) => eager_task
                    .bind(py)
                    .call((coro,), Some(handle.eager_task_kwargs(py)?))?,
                None => loop_.call_method1(&constants.create_task, (coro,))?,
            }
        };
        let mut handler = AppFutureHandler {
            send_bridge,
            scheduler,
            constants: constants.clone(),
        };
        // An eager task that already completed would only run the callback on
        // the next pass through the event loop.
        if eager.is_some() && task.call_method0(&constants.done)?.is_truthy()? {
            handler.__call__(py, task)?;
        } else {
            task.call_method1(&constants.add_done_callback, (handler,))?;
        }

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
    transport_bridge: Option<EventBridge<TransportEvent>>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
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
