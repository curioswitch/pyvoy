use std::{
    sync::{Arc, atomic::AtomicBool},
    thread::{self, JoinHandle},
};

use crate::{
    asgi::python::{
        awaitable::{EmptyAwaitable, ErrorAwaitable, ValueAwaitable},
        eventloop::EventLoops,
    },
    envoy::SyncScheduler,
    eventbridge::EventBridge,
    headernames::HeaderNameExt as _,
    types::{ClientDisconnectedError, Constants, PyDictExt as _, Scope, SyncReceiver},
};
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyNone, PyString},
};
use std::sync::mpsc;
use std::sync::mpsc::Sender;

mod awaitable;
mod eventloop;
pub(crate) mod lifespan;

fn get_gil_batch_size() -> usize {
    std::env::var("PYVOY_GIL_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40)
}

struct ExecutorInner {
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    loops: EventLoops,
    constants: Arc<Constants>,
    executor: Executor,
}

/// Holds [`JoinHandle`] for threads created by [`Executor`].
pub(crate) struct ExecutorHandles {
    loops: EventLoops,
    gil_handle: JoinHandle<()>,
}

impl ExecutorHandles {
    pub(crate) fn join(self) {
        self.loops.join();
        let _ = self.gil_handle.join();
    }
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
}

impl Executor {
    pub(crate) fn new(
        app_module: &str,
        app_attr: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_lifespan: Option<bool>,
    ) -> PyResult<(Self, ExecutorHandles)> {
        // Import threading on this thread because Python records the first thread
        // that imports threading as the main thread. When running the Python interpreter, this
        // happens to work, but not when embedding. For our purposes, we just need the asyncio
        // thread to know it's not the main thread and can use this hack. In practice, since
        // this is still during filter initialization, it may be the correct main thread anyway.
        Python::attach(|py| {
            py.import("threading")?;
            Ok::<_, PyErr>(())
        })?;

        let (app, asgi, extensions, loops) = Python::attach(|py| {
            let module = py.import(app_module)?;
            let app = module.getattr(app_attr)?;

            let app = ensure_asgi3_app(py, app)?;

            let asgi = PyDict::new(py);
            asgi.set_item("version", "3.0")?;
            asgi.set_item("spec_version", "2.5")?;
            let extensions = PyDict::new(py);
            extensions.set_item("http.response.trailers", PyDict::new(py))?;

            let loops =
                EventLoops::new(py, worker_threads, &app, &asgi, enable_lifespan, &constants)?;

            Ok::<_, PyErr>((app.unbind(), asgi.unbind(), extensions.unbind(), loops))
        })?;

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
        request_closed: bool,
        trailers_accepted: bool,
        response_closed: Arc<AtomicBool>,
        recv_bridge: EventBridge<RecvFuture>,
        send_bridge: EventBridge<SendEvent>,
        scheduler: SyncScheduler,
    ) {
        self.tx
            .send(Event::ExecuteApp(ExecuteAppEvent {
                scope,
                request_closed,
                trailers_accepted,
                response_closed,
                recv_bridge,
                send_bridge,
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
            scheduler,
        } = self.event.take().unwrap();

        let loop_ = self.loop_.bind(py);

        let scope_dict = PyDict::new(py);
        scope_dict.set_item(&self.constants.typ, &self.constants.http)?;
        scope_dict.set_item(&self.constants.asgi, &self.asgi)?;

        let mut extensions = self.extensions.bind(py).clone();
        if let Some(tls_info) = scope.tls_info {
            extensions = extensions.copy()?;
            let tls_dict = PyDict::new(py);
            tls_dict.set_item(&self.constants.tls_version, tls_info.tls_version)?;
            if let Some(client_cert_name) = tls_info.client_cert_name {
                tls_dict.set_item(
                    &self.constants.client_cert_name,
                    PyString::new(py, &client_cert_name),
                )?;
            } else {
                tls_dict.set_item(&self.constants.client_cert_name, py.None())?;
            }
            extensions.set_item(&self.constants.tls, tls_dict)?;
        }
        scope_dict.set_item(&self.constants.extensions, extensions)?;

        if let Some(state) = &self.state {
            scope_dict.set_item(&self.constants.state, state)?;
        }

        scope_dict.set_http_version(&self.constants, &scope.http_version)?;
        scope_dict.set_http_method(&self.constants, &self.constants.method, &scope.method)?;
        scope_dict.set_http_scheme(&self.constants, &self.constants.scheme, &scope.scheme)?;

        let raw_path: &[u8] =
            if let Some(query_idx) = scope.raw_path.iter().position(|&b| b == b'?') {
                scope_dict.set_item(
                    &self.constants.query_string,
                    PyBytes::new(py, &scope.raw_path[query_idx + 1..]),
                )?;
                &scope.raw_path[..query_idx]
            } else {
                scope_dict.set_item(&self.constants.query_string, &self.constants.empty_bytes)?;
                &scope.raw_path
            };

        let decoded_path = urlencoding::decode_binary(raw_path);
        scope_dict.set_item(
            &self.constants.path,
            PyString::from_bytes(py, &decoded_path)?,
        )?;
        scope_dict.set_item(&self.constants.raw_path, PyBytes::new(py, raw_path))?;
        scope_dict.set_item(&self.constants.root_path, &self.constants.root_path_value)?;
        let headers = PyList::new(
            py,
            scope.headers.iter().map(|(k, v)| {
                (
                    k.to_asgi_bytes(py, &self.constants),
                    PyBytes::new(py, v.as_bytes()),
                )
            }),
        )?;
        scope_dict.set_item(&self.constants.headers, headers)?;
        scope_dict.set_item(
            &self.constants.client,
            scope.client.map(|(a, p)| (PyString::new(py, &a[..]), p)),
        )?;
        scope_dict.set_item(
            &self.constants.server,
            scope.server.map(|(a, p)| (PyString::new(py, &a[..]), p)),
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
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.http_disconnect)?;
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
    scheduler: SyncScheduler,
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
    Shutdown,
}

/// The receive callable we pass to the application.
#[pyclass(module = "_pyvoy.asgi")]
struct RecvCallable {
    recv_bridge: EventBridge<RecvFuture>,
    scheduler: Arc<SyncScheduler>,
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
        let event = PyDict::new(py);
        if self
            .response_closed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            event.set_item(&self.constants.typ, &self.constants.http_disconnect)?;
        } else {
            event.set_item(&self.constants.typ, &self.constants.http_request)?;
            event.set_item(&self.constants.body, &self.constants.empty_bytes)?;
            event.set_item(&self.constants.more_body, false)?;
        }
        ValueAwaitable::new_py(py, event.into_any().unbind())
    }
}

/// The done callback to run when the ASGI application completes.
#[pyclass(module = "_pyvoy.asgi")]
struct AppFutureHandler {
    send_bridge: EventBridge<SendEvent>,
    scheduler: Arc<SyncScheduler>,
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
    scheduler: Arc<SyncScheduler>,
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

/// Converts headers from Python to Rust.
fn extract_headers_from_event<'py>(
    _py: Python<'py>,
    constants: &Constants,
    event: &Bound<'py, PyDict>,
) -> PyResult<Vec<(HeaderName, HeaderValue)>> {
    match event.get_item(&constants.headers)? {
        Some(v) => {
            let cap = v.len().unwrap_or(0);
            let mut headers = Vec::with_capacity(cap);
            for item in v.try_iter()? {
                let tuple = item?;
                let key_item = tuple.get_item(0)?;
                let value_item = tuple.get_item(1)?;
                let key_bytes = key_item.cast::<PyBytes>()?;
                let value_bytes = value_item.cast::<PyBytes>()?;
                headers.push((
                    HeaderName::from_bytes(key_bytes.as_bytes())
                        .map_err(|e| PyValueError::new_err(e.to_string()))?,
                    HeaderValue::from_bytes(value_bytes.as_bytes())
                        .map_err(|e| PyValueError::new_err(e.to_string()))?,
                ));
            }
            Ok(headers)
        }
        None => Ok(Vec::default()),
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
    future: Py<PyAny>,
    loop_: Py<PyAny>,
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

/// Converts an ASGI2 app to ASGI3 if needed.
///
/// Reimplements https://github.com/django/asgiref/blob/main/asgiref/compatibility.py#L40
fn ensure_asgi3_app<'py>(py: Python<'py>, app: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if is_asgi2_app(py, &app)? {
        let asgi2_invoker = ASGI2Invoker { app: app.unbind() };
        Ok(asgi2_invoker.into_bound_py_any(py)?)
    } else {
        Ok(app)
    }
}

/// Tests to see if an application is ASGI2.
fn is_asgi2_app<'py>(py: Python<'py>, app: &Bound<'py, PyAny>) -> PyResult<bool> {
    // Look for a hint on the object first. This is usually only needed for frameworks using
    // metaclass magic or a rare case where a normal function returns a coroutine, making it
    // effectively a coroutinefunction.
    if let Some(v) = app.getattr_opt("_asgi_single_callable")?
        && v.is_truthy()?
    {
        return Ok(false);
    }
    if let Some(v) = app.getattr_opt("_asgi_double_callable")?
        && v.is_truthy()?
    {
        return Ok(true);
    }

    let inspect = py.import("inspect")?;
    // Classes are always double-callable as they cannot be a coroutine.
    if inspect.call_method1("isclass", (&app,))?.is_truthy()? {
        return Ok(true);
    }

    // inspect.iscoroutinefunction Python 3.12+
    let iscoroutinefunction = inspect
        .getattr_opt("iscoroutinefunction")?
        .unwrap_or_else(|| {
            py.import("asyncio")
                .and_then(|asyncio| asyncio.getattr("iscoroutinefunction"))
                .unwrap()
        });

    // Handle callable objects.
    if let Some(callable) = app.getattr_opt("__call__")?
        && iscoroutinefunction.call1((callable,))?.is_truthy()?
    {
        return Ok(false);
    }

    Ok(!iscoroutinefunction.call1((app,))?.is_truthy()?)
}

/// An ASGI3 application that invokes an ASGI2 application.
#[pyclass(module = "_pyvoy.asgi")]
struct ASGI2Invoker {
    app: Py<PyAny>,
}

#[pymethods]
impl ASGI2Invoker {
    fn __call__<'py>(
        &self,
        py: Python<'py>,
        scope: Bound<'py, PyDict>,
        receive: Bound<'py, PyAny>,
        send: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let app = self.app.bind(py).call1((scope,))?;
        let coro = app.call1((receive, send))?;
        Ok(coro)
    }
}
