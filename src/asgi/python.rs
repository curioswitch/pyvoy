use std::thread;

use super::types::*;
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;
use flume;
use flume::Sender;
use pyo3::{
    exceptions::PyRuntimeError,
    intern,
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyNone},
};

struct ExecutorInner {
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
    loop_: Py<PyAny>,
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
    pub(crate) fn new(app_module: &str, app_attr: &str) -> Result<Self, PyErr> {
        let (loop_tx, loop_rx) = flume::bounded(0);
        thread::spawn(move || {
            let res: PyResult<()> = Python::attach(|py| {
                let asyncio = PyModule::import(py, "asyncio")?;
                let loop_ = asyncio.call_method0("new_event_loop")?;
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
            let module = PyModule::import(py, app_module)?;
            let app = module.getattr(app_attr)?;
            let asgi = PyDict::new(py);
            asgi.set_item("version", "3.0")?;
            asgi.set_item("spec_version", "2.2")?;
            let extensions = PyDict::new(py);
            extensions.set_item("http.response.trailers", PyDict::new(py))?;
            Ok::<_, PyErr>((app.unbind(), asgi.unbind(), extensions.unbind()))
        })?;

        let inner = ExecutorInner {
            app,
            asgi,
            extensions,
            loop_,
        };

        let (tx, rx) = flume::unbounded::<Event>();
        thread::spawn(move || {
            while let Ok(event) = rx.recv() {
                if let Err(e) = Python::attach(|py| {
                    inner.handle_event(py, event)?;
                    for event in rx.drain() {
                        inner.handle_event(py, event)?;
                    }
                    Ok::<_, PyErr>(())
                }) {
                    eprintln!(
                        "Unexpected Python exception in ASGI executor thread. This likely a bug in pyvoy: {}",
                        e
                    );
                }
            }
        });
        Ok(Self { tx })
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        trailers_accepted: bool,
        request_future_tx: Sender<Py<PyAny>>,
        response_tx: Sender<ResponseEvent>,
        recv_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        send_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        end_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    ) {
        self.tx
            .send(Event::ExecuteApp {
                scope,
                trailers_accepted,
                request_future_tx,
                response_tx,
                recv_scheduler,
                send_scheduler,
                end_scheduler,
            })
            .unwrap();
    }

    pub(crate) fn handle_request_future(
        &self,
        body: Box<[u8]>,
        more_body: bool,
        future: Py<PyAny>,
    ) {
        self.tx
            .send(Event::HandleRequestFuture {
                body,
                more_body,
                future,
            })
            .unwrap();
    }

    pub(crate) fn handle_response_future(&self, future: Py<PyAny>) {
        self.tx.send(Event::HandleResponseFuture(future)).unwrap();
    }
}

impl ExecutorInner {
    fn handle_event<'py>(&self, py: Python<'py>, event: Event) -> PyResult<()> {
        match event {
            Event::ExecuteApp {
                scope,
                trailers_accepted,
                request_future_tx,
                response_tx,
                recv_scheduler,
                send_scheduler,
                end_scheduler,
            } => self.execute_app(
                py,
                scope,
                trailers_accepted,
                request_future_tx,
                response_tx,
                recv_scheduler,
                send_scheduler,
                end_scheduler,
            ),
            Event::HandleRequestFuture {
                body,
                more_body,
                future,
            } => self.handle_request_future(py, body, more_body, future),
            Event::HandleResponseFuture(future) => self.handle_response_future(py, future),
        }
    }

    fn execute_app<'py>(
        &self,
        py: Python<'py>,
        scope: Scope,
        trailers_accepted: bool,
        request_future_tx: Sender<Py<PyAny>>,
        response_tx: Sender<ResponseEvent>,
        recv_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        send_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        end_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    ) -> PyResult<()> {
        let scope_dict = PyDict::new(py);
        scope_dict.set_item(intern!(py, "type"), intern!(py, "http"))?;
        scope_dict.set_item(intern!(py, "asgi"), self.asgi.bind(py))?;
        scope_dict.set_item(intern!(py, "extensions"), self.extensions.bind(py))?;
        scope_dict.set_item(
            intern!(py, "http_version"),
            match scope.http_version {
                HttpVersion::Http10 => intern!(py, "1.0"),
                HttpVersion::Http11 => intern!(py, "1.1"),
                HttpVersion::Http2 => intern!(py, "2"),
                HttpVersion::Http3 => intern!(py, "3"),
            },
        )?;
        match scope.method {
            HttpMethod::Get => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "GET"))?;
            }
            HttpMethod::Head => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "HEAD"))?;
            }
            HttpMethod::Post => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "POST"))?;
            }
            HttpMethod::Put => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "PUT"))?;
            }
            HttpMethod::Delete => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "DELETE"))?;
            }
            HttpMethod::Connect => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "CONNECT"))?;
            }
            HttpMethod::Options => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "OPTIONS"))?;
            }
            HttpMethod::Trace => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "TRACE"))?;
            }
            HttpMethod::Patch => {
                scope_dict.set_item(intern!(py, "method"), intern!(py, "PATCH"))?;
            }
            HttpMethod::Custom(m) => {
                let method = String::from_utf8_lossy(&m);
                scope_dict.set_item(intern!(py, "method"), &method)?;
            }
        }
        scope_dict.set_item(
            intern!(py, "scheme"),
            match scope.scheme {
                HttpScheme::Http => intern!(py, "http"),
                HttpScheme::Https => intern!(py, "https"),
            },
        )?;
        let decoded_path = urlencoding::decode_binary(&scope.raw_path);
        scope_dict.set_item(intern!(py, "path"), String::from_utf8_lossy(&decoded_path))?;
        scope_dict.set_item(intern!(py, "raw_path"), PyBytes::new(py, &scope.raw_path))?;
        scope_dict.set_item(
            intern!(py, "query_string"),
            PyBytes::new(py, &scope.query_string),
        )?;
        scope_dict.set_item(intern!(py, "root_path"), "")?;
        let headers = PyList::new(
            py,
            scope
                .headers
                .iter()
                .map(|(k, v)| (PyBytes::new(py, &k), PyBytes::new(py, &v))),
        )?;
        scope_dict.set_item(intern!(py, "headers"), headers)?;
        scope_dict.set_item(intern!(py, "client"), scope.client)?;
        scope_dict.set_item(intern!(py, "server"), scope.server)?;

        let app = self.app.bind(py);
        let coro = app.call1((
            scope_dict,
            ReceiveCallable {
                request_future_tx: request_future_tx,
                scheduler: recv_scheduler,
                loop_: self.loop_.clone_ref(py),
            },
            SendCallable {
                next_event: NextASGIEvent::Start,
                response_start: None,
                trailers_accepted: trailers_accepted,
                response_tx: response_tx.clone(),
                scheduler: send_scheduler,
                loop_: self.loop_.clone_ref(py),
            },
        ))?;
        let asyncio = PyModule::import(py, intern!(py, "asyncio"))?;
        let future = asyncio.call_method1(
            intern!(py, "run_coroutine_threadsafe"),
            (coro, self.loop_.bind(py)),
        )?;
        future.call_method1(
            intern!(py, "add_done_callback"),
            (AppFutureHandler {
                response_tx: response_tx,
                scheduler: end_scheduler,
            },),
        )?;

        Ok(())
    }

    fn handle_request_future<'py>(
        &self,
        py: Python<'py>,
        body: Box<[u8]>,
        more_body: bool,
        future: Py<PyAny>,
    ) -> PyResult<()> {
        let future = future.bind(py);
        let set_result = future.getattr(intern!(py, "set_result"))?;
        let event = PyDict::new(py);
        event.set_item(intern!(py, "type"), intern!(py, "http.request"))?;
        event.set_item(intern!(py, "body"), PyBytes::new(py, &body))?;
        event.set_item(intern!(py, "more_body"), more_body)?;
        self.loop_
            .bind(py)
            .call_method1(intern!(py, "call_soon_threadsafe"), (set_result, event))?;
        Ok(())
    }

    fn handle_response_future<'py>(&self, py: Python<'py>, future: Py<PyAny>) -> PyResult<()> {
        let future = future.bind(py);
        let set_result = future.getattr(intern!(py, "set_result"))?;
        self.loop_.bind(py).call_method1(
            intern!(py, "call_soon_threadsafe"),
            (set_result, PyNone::get(py)),
        )?;
        Ok(())
    }
}

enum Event {
    ExecuteApp {
        scope: Scope,
        trailers_accepted: bool,
        request_future_tx: Sender<Py<PyAny>>,
        response_tx: Sender<ResponseEvent>,
        recv_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        send_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        end_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    },
    HandleRequestFuture {
        body: Box<[u8]>,
        more_body: bool,
        future: Py<PyAny>,
    },
    HandleResponseFuture(Py<PyAny>),
}

#[pyclass]
struct ReceiveCallable {
    request_future_tx: Sender<Py<PyAny>>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    loop_: Py<PyAny>,
}

unsafe impl Sync for ReceiveCallable {}

#[pymethods]
impl ReceiveCallable {
    fn __call__(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let future = self
                .loop_
                .bind(py)
                .call_method0(intern!(py, "create_future"))?;
            match self.request_future_tx.send(future.clone().unbind()) {
                Ok(_) => {
                    self.scheduler.commit(EVENT_ID_REQUEST);
                }
                Err(_) => {
                    // TODO: Better to propagate stream close explicitly than assuming this error
                    // means closed.
                    let event = PyDict::new(py);
                    event.set_item(intern!(py, "type"), intern!(py, "http.disconnect"))?;
                    future.call_method1(intern!(py, "set_result"), (event,))?;
                }
            }
            Ok(future.unbind())
        })
    }
}

#[pyclass]
struct AppFutureHandler {
    response_tx: Sender<ResponseEvent>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
}

unsafe impl Sync for AppFutureHandler {}

#[pymethods]
impl AppFutureHandler {
    fn __call__(&mut self, future: Py<PyAny>) {
        Python::attach(|py| {
            let future = future.bind(py);
            match future.call_method1(intern!(py, "result"), (0,)) {
                Ok(_) => {}
                Err(e) => {
                    let tb = e.traceback(py).unwrap().format().unwrap_or_default();
                    eprintln!("Exception in ASGI application\n{}{}", tb, e);
                    self.response_tx
                        .send(ResponseEvent::Exception)
                        // We printed the exception, if the receiver is already gone
                        // for whatever reason it's fine.
                        .unwrap_or_default();
                    self.scheduler.commit(EVENT_ID_EXCEPTION);
                }
            }
        });
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
    response_tx: Sender<ResponseEvent>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    loop_: Py<PyAny>,
}

unsafe impl Sync for SendCallable {}

#[pymethods]
impl SendCallable {
    fn __call__(&mut self, event: Py<PyDict>) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let future = self
                .loop_
                .bind(py)
                .call_method0(intern!(py, "create_future"))?;

            let event = event.bind(py);
            let event_type: String = match event.get_item("type")? {
                Some(v) => v.extract()?,
                None => {
                    return Err(PyRuntimeError::new_err(
                        "Unexpected ASGI message, missing 'type'.",
                    ));
                }
            };

            match &self.next_event {
                NextASGIEvent::Start => {
                    if event_type != "http.response.start" {
                        return Err(PyRuntimeError::new_err(format!(
                            "Expected ASGI message 'http.response.start', but got '{}'.",
                            event_type
                        )));
                    }
                    let mut headers = extract_headers_from_event(py, &event, 1)?;
                    let status: u16 = match event.get_item(intern!(py, "status"))? {
                        Some(v) => v.extract()?,
                        None => {
                            return Err(PyRuntimeError::new_err(
                                "Unexpected ASGI message, missing 'status' in 'http.response.start'.",
                            ));
                        }
                    };
                    headers.push((
                        String::from(":status"),
                        Box::from(status.to_string().as_bytes()),
                    ));
                    let trailers: bool = match event.get_item(intern!(py, "trailers"))? {
                        Some(v) => v.extract()?,
                        None => false,
                    };
                    match trailers {
                        true => self.next_event = NextASGIEvent::BodyWithTrailers,
                        false => self.next_event = NextASGIEvent::BodyWithoutTrailers,
                    };
                    self.response_start.replace(ResponseStartEvent {
                        headers: headers,
                        trailers: trailers && self.trailers_accepted,
                    });
                    future.call_method1(intern!(py, "set_result"), (PyNone::get(py),))?;
                }
                NextASGIEvent::BodyWithoutTrailers | NextASGIEvent::BodyWithTrailers => {
                    if event_type != "http.response.body" {
                        return Err(PyRuntimeError::new_err(format!(
                            "Expected ASGI message 'http.response.body', but got '{}'.",
                            event_type
                        )));
                    }
                    let more_body: bool = match event.get_item(intern!(py, "more_body"))? {
                        Some(v) => v.extract()?,
                        None => false,
                    };
                    let body: Box<[u8]> = match event.get_item(intern!(py, "body"))? {
                        Some(body) => Box::from(body.downcast::<PyBytes>()?.as_bytes()),
                        _ => Box::new([]),
                    };
                    match (more_body, &self.next_event) {
                        (false, NextASGIEvent::BodyWithTrailers) => {
                            self.next_event = NextASGIEvent::Trailers;
                        }
                        (false, NextASGIEvent::BodyWithoutTrailers) => {
                            self.next_event = NextASGIEvent::Done;
                        }
                        _ => {}
                    }
                    let body_event = ResponseBodyEvent {
                        body,
                        more_body,
                        future: future.clone().unbind(),
                    };
                    if let Some(start_event) = self.response_start.take() {
                        self.response_tx
                            .send(ResponseEvent::Start(start_event, body_event))
                            .unwrap();
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    } else {
                        match self.response_tx.send(ResponseEvent::Body(body_event)) {
                            Ok(_) => {
                                self.scheduler.commit(EVENT_ID_RESPONSE);
                            }
                            Err(_) => {
                                // TODO: Better to propagate stream close explicitly than assuming this error
                                // means closed.
                                future
                                    .call_method1(intern!(py, "set_result"), (PyNone::get(py),))?;
                            }
                        }
                    }
                }
                NextASGIEvent::Trailers => {
                    if event_type != "http.response.trailers" {
                        return Err(PyRuntimeError::new_err(format!(
                            "Expected ASGI message 'http.response.trailers', but got '{}'.",
                            event_type
                        )));
                    }
                    let more_trailers: bool = match event.get_item(intern!(py, "more_trailers"))? {
                        Some(v) => v.extract()?,
                        None => false,
                    };
                    if !more_trailers {
                        self.next_event = NextASGIEvent::Done;
                    }
                    if self.trailers_accepted {
                        let headers = extract_headers_from_event(py, &event, 0)?;
                        self.response_tx
                            .send(ResponseEvent::Trailers(ResponseTrailersEvent {
                                headers,
                                more_trailers,
                            }))
                            .unwrap();
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                        future.call_method1(intern!(py, "set_result"), (PyNone::get(py),))?;
                    }
                }
                NextASGIEvent::Done => {
                    return Err(PyRuntimeError::new_err(format!(
                        "Unexpected ASGI message '{}' sent, after response already completed.",
                        event_type
                    )));
                }
            }
            Ok(future.unbind())
        })
    }
}

fn extract_headers_from_event<'py>(
    py: Python<'py>,
    event: &Bound<'py, PyDict>,
    extra_capacity: usize,
) -> PyResult<Vec<(String, Box<[u8]>)>> {
    match event.get_item(intern!(py, "headers"))? {
        Some(v) => {
            let cap = match v.len() {
                Ok(len) => len + extra_capacity,
                Err(_) => extra_capacity,
            };
            let mut headers = Vec::with_capacity(cap);
            for item in v.try_iter()? {
                let tuple = item?;
                let key_item = tuple.get_item(0)?;
                let value_item = tuple.get_item(1)?;
                let key_bytes = key_item.downcast::<PyBytes>()?;
                let value_bytes = value_item.downcast::<PyBytes>()?;
                headers.push((
                    String::from_utf8_lossy(key_bytes.as_bytes()).to_string(),
                    Box::from(value_bytes.as_bytes()),
                ));
            }
            Ok(headers)
        }
        None => Ok(Vec::with_capacity(extra_capacity)),
    }
}
