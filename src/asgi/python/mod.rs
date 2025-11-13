use std::{
    sync::{Arc, atomic::AtomicBool},
    thread::{self, JoinHandle},
};

use crate::{
    asgi::python::{
        awaitable::{EmptyAwaitable, ErrorAwaitable, ValueAwaitable},
        lifespan::{Lifespan, execute_lifespan},
    },
    envoy::SyncScheduler,
    eventbridge::EventBridge,
    types::{Constants, HeaderNameExt as _, PyDictExt as _, Scope},
};
use http::{HeaderName, HeaderValue};
use pyo3::{
    IntoPyObjectExt, create_exception,
    exceptions::{PyOSError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyNone, PyString},
};
use std::sync::mpsc;
use std::sync::mpsc::Sender;

mod awaitable;
pub(crate) mod lifespan;

create_exception!(pyvoy, ClientDisconnectedError, PyOSError);

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
    state: Option<Py<PyDict>>,
    loop_: Py<PyAny>,
    constants: Arc<Constants>,
    executor: Executor,
}

/// Holds [`JoinHandle`] for threads created by [`Executor`].
pub(crate) struct ExecutorHandles {
    loop_handle: JoinHandle<()>,
    gil_handle: JoinHandle<()>,
}

impl ExecutorHandles {
    pub(crate) fn join(self) {
        let _ = self.loop_handle.join();
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
///
/// In the future when supporting free-threaded Python, we will be able to
/// execute Python code directly from the request threads due to the lack of a GIL,
/// and can allow both modes with the same interface in this executor.
#[derive(Clone)]
pub(crate) struct Executor {
    tx: Sender<Event>,
}

impl Executor {
    pub(crate) fn new(
        app_module: &str,
        app_attr: &str,
        constants: Arc<Constants>,
    ) -> PyResult<(Self, ExecutorHandles, Option<Lifespan>)> {
        // Import threading on this thread because Python records the first thread
        // that imports threading as the main thread. When running the Python interpreter, this
        // happens to work, but not when embedding. For our purposes, we just need the asyncio
        // thread to know it's not the main thread and can use this hack. In practice, since
        // this is still during filter initialization, it may be the correct main thread anyway.
        Python::attach(|py| {
            py.import("threading")?;
            Ok::<_, PyErr>(())
        })?;

        let (loop_tx, loop_rx) = mpsc::sync_channel(0);
        let loop_handle = thread::spawn(move || {
            let res: PyResult<()> = Python::attach(|py| {
                let uvloop = py.import("uvloop")?;
                let asyncio = py.import("asyncio")?;
                let loop_ = uvloop.call_method0("new_event_loop")?;
                loop_tx.send(loop_.clone().unbind()).unwrap();
                asyncio.call_method1("set_event_loop", (&loop_,))?;
                loop_.call_method0("run_forever")?;
                Ok(())
            });
            res.unwrap();
        });

        let loop_ = loop_rx.recv().map_err(|e| {
            PyRuntimeError::new_err(format!(
                "Failed to initialize asyncio event loop for ASGI executor: {}",
                e
            ))
        })?;

        let (app, asgi, extensions, mut lifespan) = Python::attach(|py| {
            let module = py.import(app_module)?;
            let app = module.getattr(app_attr)?;
            let asgi = PyDict::new(py);
            asgi.set_item("version", "3.0")?;
            asgi.set_item("spec_version", "2.5")?;
            let extensions = PyDict::new(py);
            extensions.set_item("http.response.trailers", PyDict::new(py))?;

            let lifespan = execute_lifespan(&app, &asgi, loop_.bind(py), &constants)?;

            Ok::<_, PyErr>((app.unbind(), asgi.unbind(), extensions.unbind(), lifespan))
        })?;

        let lifespan_supported = lifespan.startup()?;

        let (tx, rx) = mpsc::channel::<Event>();
        let executor = Self { tx };

        let state = if lifespan_supported {
            Some(lifespan.state.take().unwrap())
        } else {
            None
        };

        let inner = ExecutorInner {
            app,
            asgi,
            extensions,
            state,
            loop_,
            constants,
            executor: executor.clone(),
        };

        let gil_handle = thread::spawn(move || {
            let gil_batch_size = get_gil_batch_size();
            let mut shutdown = false;
            while let Ok(event) = rx.recv() {
                if let Err(e) = Python::attach(|py| {
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
                    Ok::<_, PyErr>(())
                }) {
                    eprintln!(
                        "Unexpected Python exception in ASGI executor thread. This likely a bug in pyvoy: {}",
                        e
                    );
                }
                if shutdown {
                    return;
                }
            }
        });
        Ok((
            executor,
            ExecutorHandles {
                loop_handle,
                gil_handle,
            },
            if lifespan_supported {
                Some(lifespan)
            } else {
                None
            },
        ))
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
            .send(Event::ExecuteApp {
                scope,
                request_closed,
                trailers_accepted,
                response_closed,
                recv_bridge,
                send_bridge,
                scheduler,
            })
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

    pub(crate) fn handle_dropped_recv_future(&self, future: Py<PyAny>) {
        self.tx
            .send(Event::HandleDroppedRecvFuture(future))
            .unwrap();
    }

    pub(crate) fn handle_send_future(&self, future: SendFuture) {
        self.tx.send(Event::HandleSendFuture(future)).unwrap();
    }

    pub(crate) fn handle_dropped_send_future(&self, future: Py<PyAny>) {
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
            Event::ExecuteApp {
                scope,
                request_closed,
                trailers_accepted,
                response_closed,
                recv_bridge,
                send_bridge,
                scheduler,
            } => {
                self.execute_app(
                    py,
                    scope,
                    request_closed,
                    trailers_accepted,
                    response_closed,
                    recv_bridge,
                    send_bridge,
                    scheduler,
                )?;
            }
            Event::HandleRecvFuture {
                body,
                more_body,
                mut future,
            } => {
                self.handle_recv_future(py, body, more_body, future.future.take().unwrap())?;
            }
            Event::HandleDroppedRecvFuture(future) => {
                self.handle_dropped_recv_future(py, future)?;
            }
            Event::HandleSendFuture(mut send_future) => {
                self.handle_send_future(py, send_future.future.take().unwrap())?;
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

    fn execute_app<'py>(
        &self,
        py: Python<'py>,
        scope: Scope,
        request_closed: bool,
        trailers_accepted: bool,
        response_closed: Arc<AtomicBool>,
        recv_bridge: EventBridge<RecvFuture>,
        send_bridge: EventBridge<SendEvent>,
        scheduler: SyncScheduler,
    ) -> PyResult<()> {
        let scope_dict = PyDict::new(py);
        scope_dict.set_item(&self.constants.typ, &self.constants.http)?;
        scope_dict.set_item(&self.constants.asgi, &self.asgi)?;
        scope_dict.set_item(&self.constants.extensions, &self.extensions)?;
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
        scope_dict.set_item(&self.constants.root_path, &self.constants.empty_string)?;
        let headers = PyList::new(
            py,
            scope.headers.iter().map(|(k, v)| {
                (
                    k.as_py_bytes(py, &self.constants),
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
                loop_: self.loop_.clone_ref(py),
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
                loop_: self.loop_.clone_ref(py),
                executor: self.executor.clone(),
                constants: self.constants.clone(),
            },
        ))?;
        let asyncio = py.import(&self.constants.asyncio)?;
        let future = asyncio.call_method1(
            &self.constants.run_coroutine_threadsafe,
            (coro, &self.loop_),
        )?;
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

    fn handle_recv_future<'py>(
        &self,
        py: Python<'py>,
        body: Box<[u8]>,
        more_body: bool,
        future: Py<PyAny>,
    ) -> PyResult<()> {
        let set_result = future.getattr(py, &self.constants.set_result)?;
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.http_request)?;
        event.set_item(&self.constants.body, PyBytes::new(py, &body))?;
        event.set_item(&self.constants.more_body, more_body)?;
        self.loop_.call_method1(
            py,
            &self.constants.call_soon_threadsafe,
            (set_result, event),
        )?;
        Ok(())
    }

    fn handle_dropped_recv_future<'py>(&self, py: Python<'py>, future: Py<PyAny>) -> PyResult<()> {
        let set_result = future.getattr(py, &self.constants.set_result)?;
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.http_disconnect)?;
        self.loop_.call_method1(
            py,
            &self.constants.call_soon_threadsafe,
            (set_result, event),
        )?;
        Ok(())
    }

    fn handle_send_future<'py>(&self, py: Python<'py>, future: Py<PyAny>) -> PyResult<()> {
        let set_result = future.getattr(py, &self.constants.set_result)?;
        self.loop_.call_method1(
            py,
            &self.constants.call_soon_threadsafe,
            (set_result, PyNone::get(py)),
        )?;
        Ok(())
    }

    fn handle_dropped_send_future<'py>(&self, py: Python<'py>, future: Py<PyAny>) -> PyResult<()> {
        let set_exception = future.getattr(py, &self.constants.set_exception)?;
        self.loop_.call_method1(
            py,
            &self.constants.call_soon_threadsafe,
            (set_exception, ClientDisconnectedError::new_err(())),
        )?;
        Ok(())
    }

    fn shutdown<'py>(&self, py: Python<'py>) -> PyResult<()> {
        let stop = self.loop_.getattr(py, &self.constants.stop)?;
        self.loop_
            .call_method1(py, &self.constants.call_soon_threadsafe, (stop,))?;
        Ok(())
    }
}

enum Event {
    ExecuteApp {
        scope: Scope,
        request_closed: bool,
        trailers_accepted: bool,
        response_closed: Arc<AtomicBool>,
        recv_bridge: EventBridge<RecvFuture>,
        send_bridge: EventBridge<SendEvent>,
        scheduler: SyncScheduler,
    },
    HandleRecvFuture {
        body: Box<[u8]>,
        more_body: bool,
        future: RecvFuture,
    },
    HandleDroppedRecvFuture(Py<PyAny>),
    HandleSendFuture(SendFuture),
    HandleDroppedSendFuture(Py<PyAny>),
    Shutdown,
}

#[pyclass]
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
            future: Some(future.clone().unbind()),
            executor: self.executor.clone(),
        };
        if self.recv_bridge.send(recv_future).is_ok() {
            self.scheduler.commit(EVENT_ID_REQUEST);
        }
        Ok(future)
    }
}

#[pyclass]
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

#[pyclass]
struct AppFutureHandler {
    send_bridge: EventBridge<SendEvent>,
    scheduler: Arc<SyncScheduler>,
    constants: Arc<Constants>,
}

unsafe impl Sync for AppFutureHandler {}

#[pymethods]
impl AppFutureHandler {
    fn __call__<'py>(&mut self, py: Python<'py>, future: Bound<'py, PyAny>) {
        if let Err(e) = future.call_method1(&self.constants.result, (0,)) {
            if e.is_instance_of::<ClientDisconnectedError>(py) {
                return;
            }
            let tb = e.traceback(py).unwrap().format().unwrap_or_default();
            eprintln!("Exception in ASGI application\n{}{}", tb, e);
            if self.send_bridge.send(SendEvent::Exception).is_ok() {
                self.scheduler.commit(EVENT_ID_EXCEPTION);
            }
        }
    }
}

enum NextASGIEvent {
    Start,
    Body { trailers: bool },
    Trailers,
    Done,
}

#[pyclass]
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
                let status: u16 = match event.get_item(&self.constants.status)? {
                    Some(v) => v.extract()?,
                    None => {
                        return Err(PyRuntimeError::new_err(
                            "Unexpected ASGI message, missing 'status' in 'http.response.start'.",
                        ));
                    }
                };
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
                            future: Some(future.unbind()),
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

pub(crate) struct ResponseStartEvent {
    pub status: u16,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub trailers: bool,
}

pub(crate) struct ResponseBodyEvent {
    pub body: Box<[u8]>,
    // Future to notify when the body is completed writing.
    // Not sent for the final piece of body, so None indicates
    // more_body = False.
    pub future: Option<SendFuture>,
}

pub(crate) struct ResponseTrailersEvent {
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub more_trailers: bool,
}

pub(crate) enum SendEvent {
    Start(ResponseStartEvent, ResponseBodyEvent),
    Body(ResponseBodyEvent),
    Trailers(ResponseTrailersEvent),
    Exception,
}

pub(crate) struct RecvFuture {
    future: Option<Py<PyAny>>,
    executor: Executor,
}

impl Drop for RecvFuture {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            self.executor.handle_dropped_recv_future(future);
        }
    }
}

pub(crate) struct SendFuture {
    future: Option<Py<PyAny>>,
    executor: Executor,
}

impl Drop for SendFuture {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            self.executor.handle_dropped_send_future(future);
        }
    }
}

pub(crate) const EVENT_ID_REQUEST: u64 = 1;
pub(crate) const EVENT_ID_RESPONSE: u64 = 2;
pub(crate) const EVENT_ID_EXCEPTION: u64 = 3;
