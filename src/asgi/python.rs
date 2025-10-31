use std::{sync::Arc, thread};

use crate::envoy::SyncScheduler;
use crate::types::*;
use http::{HeaderName, HeaderValue};
use pyo3::{
    IntoPyObjectExt, create_exception,
    exceptions::{PyOSError, PyRuntimeError, PyStopIteration, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyNone, PyString},
};
use std::sync::mpsc;
use std::sync::mpsc::Sender;

create_exception!(pyvoy, ClientDisconnectedError, PyOSError);

struct ExecutorInner {
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    loop_: Py<PyAny>,
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
    ) -> PyResult<Self> {
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
        thread::spawn(move || {
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

        let (app, asgi, extensions) = Python::attach(|py| {
            let module = py.import(app_module)?;
            let app = module.getattr(app_attr)?;
            let asgi = PyDict::new(py);
            asgi.set_item("version", "3.0")?;
            asgi.set_item("spec_version", "2.2")?;
            let extensions = PyDict::new(py);
            extensions.set_item("http.response.trailers", PyDict::new(py))?;
            Ok::<_, PyErr>((app.unbind(), asgi.unbind(), extensions.unbind()))
        })?;

        let (tx, rx) = mpsc::channel::<Event>();
        let executor = Self { tx };

        let inner = ExecutorInner {
            app,
            asgi,
            extensions,
            loop_,
            constants,
            executor: executor.clone(),
        };

        thread::spawn(move || {
            while let Ok(event) = rx.recv() {
                if let Err(e) = Python::attach(|py| {
                    inner.handle_event(py, event)?;
                    Ok::<_, PyErr>(())
                }) {
                    eprintln!(
                        "Unexpected Python exception in ASGI executor thread. This likely a bug in pyvoy: {}",
                        e
                    );
                }
            }
        });
        Ok(executor)
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        request_closed: bool,
        trailers_accepted: bool,
        recv_future_tx: Sender<RecvFuture>,
        response_tx: Sender<ResponseEvent>,
        scheduler: Arc<SyncScheduler>,
    ) {
        self.tx
            .send(Event::ExecuteApp {
                scope,
                request_closed,
                trailers_accepted,
                recv_future_tx,
                response_tx,
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
}

impl ExecutorInner {
    fn handle_event<'py>(&self, py: Python<'py>, event: Event) -> PyResult<()> {
        match event {
            Event::ExecuteApp {
                scope,
                request_closed,
                trailers_accepted,
                recv_future_tx,
                response_tx,
                scheduler,
            } => self.execute_app(
                py,
                scope,
                request_closed,
                trailers_accepted,
                recv_future_tx,
                response_tx,
                scheduler,
            ),
            Event::HandleRecvFuture {
                body,
                more_body,
                mut future,
            } => self.handle_recv_future(py, body, more_body, future.future.take().unwrap()),
            Event::HandleDroppedRecvFuture(future) => self.handle_dropped_recv_future(py, future),
            Event::HandleSendFuture(mut send_future) => {
                self.handle_send_future(py, send_future.future.take().unwrap())
            }
            Event::HandleDroppedSendFuture(future) => self.handle_dropped_send_future(py, future),
        }
    }

    fn execute_app<'py>(
        &self,
        py: Python<'py>,
        scope: Scope,
        request_closed: bool,
        trailers_accepted: bool,
        recv_future_tx: Sender<RecvFuture>,
        response_tx: Sender<ResponseEvent>,
        scheduler: Arc<SyncScheduler>,
    ) -> PyResult<()> {
        let scope_dict = PyDict::new(py);
        scope_dict.set_item(&self.constants.typ, &self.constants.http)?;
        scope_dict.set_item(&self.constants.asgi, &self.asgi)?;
        scope_dict.set_item(&self.constants.extensions, &self.extensions)?;
        scope_dict.set_http_version(
            &self.constants,
            &self.constants.http_version,
            &scope.http_version,
        )?;
        scope_dict.set_http_method(&self.constants, &self.constants.method, &scope.method)?;
        scope_dict.set_http_scheme(&self.constants, &self.constants.scheme, &scope.scheme)?;
        let decoded_path = urlencoding::decode_binary(&scope.raw_path);
        scope_dict.set_item(
            &self.constants.path,
            PyString::from_bytes(py, &decoded_path)?,
        )?;
        scope_dict.set_item(&self.constants.raw_path, PyBytes::new(py, &scope.raw_path))?;
        scope_dict.set_item(
            &self.constants.query_string,
            PyBytes::new(py, &scope.query_string),
        )?;
        scope_dict.set_item(&self.constants.root_path, "")?;
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

        let recv = if request_closed {
            EmptyRecvCallable {
                constants: self.constants.clone(),
            }
            .into_bound_py_any(py)?
        } else {
            RecvCallable {
                recv_future_tx,
                scheduler: scheduler.clone(),
                loop_: self.loop_.clone_ref(py),
                executor: self.executor.clone(),
                constants: self.constants.clone(),
            }
            .into_bound_py_any(py)?
        };

        let app = self.app.bind(py);
        let coro = app.call1((
            scope_dict,
            recv,
            SendCallable {
                next_event: NextASGIEvent::Start,
                response_start: None,
                trailers_accepted,
                closed: false,
                response_tx: response_tx.clone(),
                scheduler: scheduler.clone(),
                loop_: self.loop_.clone_ref(py),
                executor: self.executor.clone(),
                constants: self.constants.clone(),
            },
        ))?;
        let asyncio = py.import(&self.constants.asyncio)?;
        let future = asyncio.call_method1(
            &self.constants.run_coroutine_threadsafe,
            (coro, self.loop_.bind(py)),
        )?;
        future.call_method1(
            &self.constants.add_done_callback,
            (AppFutureHandler {
                response_tx,
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
        let future = future.bind(py);
        let set_result = future.getattr(&self.constants.set_result)?;
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.http_request)?;
        event.set_item(&self.constants.body, PyBytes::new(py, &body))?;
        event.set_item(&self.constants.more_body, more_body)?;
        self.loop_
            .bind(py)
            .call_method1(&self.constants.call_soon_threadsafe, (set_result, event))?;
        Ok(())
    }

    fn handle_dropped_recv_future<'py>(&self, py: Python<'py>, future: Py<PyAny>) -> PyResult<()> {
        let future = future.bind(py);
        let set_result = future.getattr(&self.constants.set_result)?;
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.http_disconnect)?;
        self.loop_
            .bind(py)
            .call_method1(&self.constants.call_soon_threadsafe, (set_result, event))?;
        Ok(())
    }

    fn handle_send_future<'py>(&self, py: Python<'py>, future: Py<PyAny>) -> PyResult<()> {
        let future = future.bind(py);
        let set_result = future.getattr(&self.constants.set_result)?;
        self.loop_.bind(py).call_method1(
            &self.constants.call_soon_threadsafe,
            (set_result, PyNone::get(py)),
        )?;
        Ok(())
    }

    fn handle_dropped_send_future<'py>(&self, py: Python<'py>, future: Py<PyAny>) -> PyResult<()> {
        let future = future.bind(py);
        let set_exception = future.getattr(&self.constants.set_exception)?;
        self.loop_.bind(py).call_method1(
            &self.constants.call_soon_threadsafe,
            (set_exception, ClientDisconnectedError::new_err(())),
        )?;
        Ok(())
    }
}

enum Event {
    ExecuteApp {
        scope: Scope,
        request_closed: bool,
        trailers_accepted: bool,
        recv_future_tx: Sender<RecvFuture>,
        response_tx: Sender<ResponseEvent>,
        scheduler: Arc<SyncScheduler>,
    },
    HandleRecvFuture {
        body: Box<[u8]>,
        more_body: bool,
        future: RecvFuture,
    },
    HandleDroppedRecvFuture(Py<PyAny>),
    HandleSendFuture(SendFuture),
    HandleDroppedSendFuture(Py<PyAny>),
}

#[pyclass]
struct RecvCallable {
    recv_future_tx: Sender<RecvFuture>,
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
        if self.recv_future_tx.send(recv_future).is_ok() {
            self.scheduler.commit(EVENT_ID_REQUEST);
        }
        Ok(future)
    }
}

#[pyclass]
struct EmptyRecvCallable {
    constants: Arc<Constants>,
}

#[pymethods]
impl EmptyRecvCallable {
    fn __call__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.http_request)?;
        event.set_item(&self.constants.body, &self.constants.empty_bytes)?;
        event.set_item(&self.constants.more_body, false)?;
        ValueAwaitable {
            value: Some(event.into_any().unbind()),
        }
        .into_bound_py_any(py)
    }
}

#[pyclass]
struct AppFutureHandler {
    response_tx: Sender<ResponseEvent>,
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
            if self.response_tx.send(ResponseEvent::Exception).is_ok() {
                self.scheduler.commit(EVENT_ID_EXCEPTION);
            }
        }
    }
}

enum NextASGIEvent {
    Start,
    BodyWithoutTrailers,
    BodyWithTrailers,
    Trailers,
    Done,
}

#[pyclass]
struct SendCallable {
    next_event: NextASGIEvent,
    response_start: Option<ResponseStartEvent>,
    trailers_accepted: bool,
    closed: bool,
    response_tx: Sender<ResponseEvent>,
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
            return Err(ClientDisconnectedError::new_err(()));
        }

        let event_type_py = event
            .get_item(&self.constants.typ)?
            .ok_or_else(|| PyRuntimeError::new_err("Unexpected ASGI message, missing 'type'."))?;
        let event_type = event_type_py.cast::<PyString>()?.to_str()?;

        match &self.next_event {
            NextASGIEvent::Start => {
                if event_type != "http.response.start" {
                    return Err(PyRuntimeError::new_err(format!(
                        "Expected ASGI message 'http.response.start', but got '{}'.",
                        event_type
                    )));
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
                if trailers {
                    self.next_event = NextASGIEvent::BodyWithTrailers;
                } else {
                    self.next_event = NextASGIEvent::BodyWithoutTrailers;
                };
                self.response_start.replace(ResponseStartEvent {
                    status,
                    headers,
                    trailers: trailers && self.trailers_accepted,
                });
                EmptyAwaitable.into_bound_py_any(py)
            }
            NextASGIEvent::BodyWithoutTrailers | NextASGIEvent::BodyWithTrailers => {
                if event_type != "http.response.body" {
                    return Err(PyRuntimeError::new_err(format!(
                        "Expected ASGI message 'http.response.body', but got '{}'.",
                        event_type
                    )));
                }
                let more_body: bool = match event.get_item(&self.constants.more_body)? {
                    Some(v) => v.extract()?,
                    None => false,
                };
                let body: Box<[u8]> = match event.get_item(&self.constants.body)? {
                    Some(body) => Box::from(body.cast::<PyBytes>()?.as_bytes()),
                    _ => Box::new([]),
                };
                if !more_body {
                    match &self.next_event {
                        NextASGIEvent::BodyWithTrailers => {
                            self.next_event = NextASGIEvent::Trailers;
                        }
                        NextASGIEvent::BodyWithoutTrailers => {
                            self.next_event = NextASGIEvent::Done;
                        }
                        _ => {}
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
                    (EmptyAwaitable.into_bound_py_any(py)?, None)
                };
                let body_event = ResponseBodyEvent {
                    body,
                    future: send_future,
                };
                if let Some(start_event) = self.response_start.take() {
                    if self
                        .response_tx
                        .send(ResponseEvent::Start(start_event, body_event))
                        .is_ok()
                    {
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    } else {
                        self.closed = true;
                    }
                } else if self
                    .response_tx
                    .send(ResponseEvent::Body(body_event))
                    .is_ok()
                {
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                } else {
                    self.closed = true;
                }
                Ok(ret)
            }
            NextASGIEvent::Trailers => {
                if event_type != "http.response.trailers" {
                    return Err(PyRuntimeError::new_err(format!(
                        "Expected ASGI message 'http.response.trailers', but got '{}'.",
                        event_type
                    )));
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
                        .response_tx
                        .send(ResponseEvent::Trailers(ResponseTrailersEvent {
                            headers,
                            more_trailers,
                        }))
                        .is_ok()
                    {
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    };
                }
                EmptyAwaitable.into_bound_py_any(py)
            }
            NextASGIEvent::Done => Err(PyRuntimeError::new_err(format!(
                "Unexpected ASGI message '{}' sent, after response already completed.",
                event_type
            ))),
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
                    HeaderName::from_bytes(key_bytes.as_bytes()).map_err(|e| {
                        PyValueError::new_err(format!("invalid header name: {}", e))
                    })?,
                    HeaderValue::from_bytes(value_bytes.as_bytes()).map_err(|e| {
                        PyValueError::new_err(format!("invalid header value: {}", e))
                    })?,
                ));
            }
            Ok(headers)
        }
        None => Ok(Vec::new()),
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

pub(crate) enum ResponseEvent {
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

#[pyclass]
struct EmptyAwaitable;

#[pymethods]
impl EmptyAwaitable {
    fn __await__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __iter__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __next__(&self) -> Option<()> {
        None
    }
}

#[pyclass]
struct ValueAwaitable {
    value: Option<Py<PyAny>>,
}

#[pymethods]
impl ValueAwaitable {
    fn __await__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __iter__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __next__<'py>(&mut self) -> PyResult<Py<PyAny>> {
        if let Some(value) = self.value.take() {
            Err(PyStopIteration::new_err(value))
        } else {
            Err(PyStopIteration::new_err(()))
        }
    }
}
