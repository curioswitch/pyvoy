use envoy_proxy_dynamic_modules_rust_sdk::envoy_log_error;
use executors::{Executor as _, crossbeam_channel_pool::ThreadPool};
use http::{HeaderName, HeaderValue, header};
use pyo3::{
    IntoPyObjectExt,
    exceptions::{PyRuntimeError, PyStopIteration, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyString, PyTuple},
};

use super::types::*;
use crate::types::*;
use crate::{envoy::SyncScheduler, eventbridge::EventBridge};
use std::sync::{Arc, Mutex, mpsc::Receiver};

#[derive(Clone)]
pub(crate) struct Executor {
    pool: ThreadPool,
    app: Arc<Py<PyAny>>,
    constants: Arc<Constants>,
}

impl Executor {
    pub(crate) fn new(
        app_module: &str,
        app_attr: &str,
        num_threads: usize,
        constants: Arc<Constants>,
    ) -> PyResult<Self> {
        let app = Python::attach(|py| {
            let module = py.import(app_module)?;
            let app = module.getattr(app_attr)?;
            Ok::<_, PyErr>(app.unbind())
        })?;

        let pool = ThreadPool::new(num_threads);

        Ok(Self {
            pool,
            app: Arc::new(app),
            constants,
        })
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        request_read_bridge: EventBridge<RequestReadEvent>,
        request_body_rx: Receiver<RequestBody>,
        response_bridge: EventBridge<ResponseEvent>,
        response_written_rx: Receiver<()>,
        scheduler: SyncScheduler,
    ) {
        let response_written_rx = SyncReceiver(response_written_rx);
        let constants = self.constants.clone();
        let app = self.app.clone();
        self.pool.execute(move || {
            let scheduler = Arc::new(scheduler);
            let result: PyResult<()> = Python::attach(|py| {
                let app = app.bind(py);

                let environ = PyDict::new(py);
                environ.set_http_method(&constants, &constants.request_method, &scope.method)?;

                // TODO: support root_path etc
                environ.set_item(&constants.script_name, &constants.empty_string)?;

                let raw_path: &[u8] =
                    if let Some(query_idx) = scope.raw_path.iter().position(|&b| b == b'?') {
                        environ.set_item(
                            &constants.wsgi_query_string,
                            PyString::from_bytes(py, &scope.raw_path[query_idx + 1..])?,
                        )?;
                        &scope.raw_path[..query_idx]
                    } else {
                        &scope.raw_path
                    };

                let decoded_path = urlencoding::decode_binary(raw_path);
                environ.set_item(
                    &constants.path_info,
                    PyString::from_bytes(py, &decoded_path)?,
                )?;

                for (key, value) in scope.headers.iter() {
                    match *key {
                        header::CONTENT_TYPE => environ.set_item(
                            &constants.content_type,
                            PyString::from_bytes(py, value.as_bytes())?,
                        )?,
                        header::CONTENT_LENGTH => environ.set_item(
                            &constants.content_length,
                            PyString::from_bytes(py, value.as_bytes())?,
                        )?,
                        _ => {
                            let key_str = key.as_str().to_uppercase().replace("-", "_");
                            let header_name = format!("HTTP_{}", key_str);
                            if let Some(existing) = environ.get_item(&header_name)? {
                                let value_str = String::from_utf8_lossy(value.as_bytes());
                                let existing = existing.cast::<PyString>()?;
                                let new_value = format!("{},{}", existing.to_str()?, value_str);
                                environ.set_item(header_name, new_value)?;
                            } else {
                                environ.set_item(
                                    header_name,
                                    PyString::from_bytes(py, value.as_bytes())?,
                                )?;
                            }
                        }
                    }
                }

                if let Some((server, port)) = scope.server {
                    environ.set_item(&constants.server_name, &server[..])?;
                    environ.set_item(&constants.server_port, port.to_string())?;
                } else {
                    // In practice, should never be exercised.
                    environ.set_item(&constants.server_name, "localhost")?;
                    environ.set_item(&constants.server_port, "0")?;
                }

                environ.set_http_version_wsgi(&constants, &scope.http_version)?;

                environ.set_item(&constants.wsgi_version, (1, 0))?;
                environ.set_http_scheme(&constants, &constants.wsgi_url_scheme, &scope.scheme)?;
                environ.set_item(
                    &constants.wsgi_input,
                    RequestInput {
                        request_read_bridge,
                        request_body_rx: SyncReceiver(request_body_rx),
                        scheduler: scheduler.clone(),
                        closed: false,
                        constants: constants.clone(),
                        lock: Mutex::new(()),
                    },
                )?;
                environ.set_item(
                    &constants.wsgi_errors,
                    ErrorsOutput {
                        buffer: Mutex::new(String::new()),
                    },
                )?;

                environ.set_item(&constants.wsgi_multithread, true)?;
                environ.set_item(&constants.wsgi_multiprocess, false)?;
                environ.set_item(&constants.wsgi_run_once, false)?;

                let response = app.call1((
                    environ,
                    StartResponseCallable {
                        called: false,
                        response_bridge: response_bridge.clone(),
                        scheduler: scheduler.clone(),
                    },
                ))?;

                // We ignore all send errors here since they only happen if the filter was dropped meaning
                // the request was closed, usually by the client. This is not an application error, and we just need
                // to make sure a close() method for a generator is called before returning.

                match response.len() {
                    Ok(0) => {
                        if response_bridge
                            .send(ResponseEvent::Body(ResponseBodyEvent {
                                body: Box::default(),
                                more_body: false,
                            }))
                            .is_ok()
                        {
                            scheduler.commit(EVENT_ID_RESPONSE);
                        }
                    }
                    Ok(1) => {
                        let item =
                            response.try_iter()?.next().ok_or(PyRuntimeError::new_err(
                                "WSGI app returned empty iterator despite len() == 1",
                            ))??;
                        let body: Box<[u8]> = Box::from(item.cast::<PyBytes>()?.as_bytes());
                        if response_bridge
                            .send(ResponseEvent::Body(ResponseBodyEvent {
                                body,
                                more_body: false,
                            }))
                            .is_ok()
                        {
                            scheduler.commit(EVENT_ID_RESPONSE);
                        }
                    }
                    _ => {
                        for item in response.try_iter()? {
                            let body: Box<[u8]> = Box::from(item?.cast::<PyBytes>()?.as_bytes());
                            if response_bridge
                                .send(ResponseEvent::Body(ResponseBodyEvent {
                                    body,
                                    more_body: true,
                                }))
                                .is_ok()
                            {
                                scheduler.commit(EVENT_ID_RESPONSE);
                                py.detach(|| {
                                    let _ = response_written_rx.recv();
                                });
                            }
                        }
                        if response_bridge
                            .send(ResponseEvent::Body(ResponseBodyEvent {
                                body: Box::from([]),
                                more_body: false,
                            }))
                            .is_ok()
                        {
                            scheduler.commit(EVENT_ID_RESPONSE);
                            py.detach(|| {
                                // RecvError only is request filter was dropped, but since this
                                // is the end always safe to ignore.
                                let _ = response_written_rx.recv();
                            });
                        }
                    }
                }

                if let Ok(close) = response.getattr(&constants.close) {
                    close.call0()?;
                }

                Ok(())
            });
            if let Err(e) = result {
                Python::attach(|py| {
                    let tb = e.traceback(py).unwrap().format().unwrap_or_default();
                    eprintln!("Exception in WSGI application\n{}{}", tb, e);
                });
                let _ = response_bridge.send(ResponseEvent::Exception);
                scheduler.commit(EVENT_ID_EXCEPTION);
            }
        });
    }
}

/// The start_response callable passed to the WSGI application.
///
/// It is used to provide response header information, but we cannot immediately flush them to the client.
/// So the callable's job is to record the information provided to then use when iterating the app's
/// response.
///
/// https://peps.python.org/pep-3333/#the-start-response-callable
#[pyclass]
struct StartResponseCallable {
    response_bridge: EventBridge<ResponseEvent>,
    scheduler: Arc<SyncScheduler>,
    called: bool,
}

#[pymethods]
impl StartResponseCallable {
    #[pyo3(signature = (status, response_headers, _exc_info=None))]
    fn __call__<'py>(
        &mut self,
        py: Python<'py>,
        status: &str,
        response_headers: Bound<'py, PyList>,
        _exc_info: Option<Bound<'py, PyTuple>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if self.called {
            // TODO: Allow when exc_info is provided.
            return Err(PyRuntimeError::new_err(
                "start_response called multiple times",
            ));
        }

        self.called = true;
        let mut headers = Vec::with_capacity(response_headers.len());
        for item in response_headers.iter() {
            let key_item = item.get_item(0)?;
            let key = key_item.cast::<PyString>()?;
            let value_item = item.get_item(1)?;
            let value = value_item.cast::<PyString>()?;
            headers.push((
                HeaderName::from_bytes(key.to_str()?.as_bytes())
                    .map_err(|e| PyValueError::new_err(format!("invalid header name: {}", e)))?,
                HeaderValue::from_bytes(value.to_str()?.as_bytes())
                    .map_err(|e| PyValueError::new_err(format!("invalid header value: {}", e)))?,
            ));
        }

        let status_code = match status.split_once(' ') {
            Some((code_str, _)) => code_str,
            None => status,
        };
        let status = status_code
            .parse::<u16>()
            .map_err(|e| PyValueError::new_err(format!("invalid status code: {}", e)))?;

        if self
            .response_bridge
            .send(ResponseEvent::Start(ResponseStartEvent { status, headers }))
            .is_ok()
        {
            self.scheduler.commit(EVENT_ID_RESPONSE);
        }

        WriteCallable {
            response_bridge: self.response_bridge.clone(),
            scheduler: self.scheduler.clone(),
        }
        .into_bound_py_any(py)
    }
}

/// The WSGI input stream to read the request body.
///
/// We send requests to read body to the filter which returns the requested amount, buffering as needed.
/// Buffering in the filter allows us to drain only as much request body as was requested to allow for
/// proper backpressure. Because in WSGI we block on reads, we use channels for the bodies themselves.
///
/// https://peps.python.org/pep-3333/#input-and-error-streams
#[pyclass]
struct RequestInput {
    /// The event bridge to send read requests.
    request_read_bridge: EventBridge<RequestReadEvent>,
    /// The channel receiver to receive body chunks.
    request_body_rx: SyncReceiver<RequestBody>,
    /// The scheduler to wake up the filter to process read events.
    scheduler: Arc<SyncScheduler>,
    /// Whether the request body is closed.
    closed: bool,
    /// Memoized constants.
    constants: Arc<Constants>,
    /// Lock to ensure only one read is executed at a time. No well behaved app should
    /// call read concurrently since the order cannot be determined, but it's not hard
    /// to protect against it either.
    lock: Mutex<()>,
}

unsafe impl Sync for RequestInput {}

#[pymethods]
impl RequestInput {
    #[pyo3(signature = (size=-1))]
    fn read<'py>(&mut self, py: Python<'py>, size: Option<isize>) -> PyResult<Bound<'py, PyBytes>> {
        let size = size.unwrap_or(-1);
        if size == 0 {
            Ok(self.constants.empty_bytes.bind(py).clone())
        } else {
            self.do_read(py, RequestReadEvent::Raw(size))
        }
    }

    #[pyo3(signature = (size=-1))]
    fn readline<'py>(
        &mut self,
        py: Python<'py>,
        size: Option<isize>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let size = size.unwrap_or(-1);
        if size == 0 {
            Ok(self.constants.empty_bytes.bind(py).clone())
        } else {
            self.do_read(py, RequestReadEvent::Line(size))
        }
    }

    fn __iter__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let line = self.do_read(py, RequestReadEvent::Line(-1))?;
        if line.as_bytes().is_empty() {
            Err(PyStopIteration::new_err(()))
        } else {
            Ok(line)
        }
    }

    #[pyo3(signature = (hint=-1))]
    fn readlines<'py>(
        &mut self,
        py: Python<'py>,
        hint: Option<isize>,
    ) -> PyResult<Bound<'py, PyList>> {
        // We ignore hint as is common but want to keep the parameter name matching Python.
        let _ = hint;

        // Follow gunicorn's example of reading the entire request body and splitting it into lines to reduce I/O.
        // This makes sense since it's trivial to use list(iter(input)) to stream lines instead if desired.
        let body = self.do_read(py, RequestReadEvent::Raw(-1))?;
        let res = PyList::empty(py);
        for line in body.as_bytes().split_inclusive(|&b| b == b'\n') {
            res.append(PyBytes::new(py, line))?;
        }
        Ok(res)
    }
}

impl RequestInput {
    fn do_read<'py>(
        &mut self,
        py: Python<'py>,
        event: RequestReadEvent,
    ) -> PyResult<Bound<'py, PyBytes>> {
        if self.closed {
            return Ok(self.constants.empty_bytes.bind(py).clone());
        }

        let _lock = self.lock.lock().unwrap();

        let _ = self.request_read_bridge.send(event);
        self.scheduler.commit(EVENT_ID_REQUEST);

        let body = py.detach::<PyResult<RequestBody>, _>(|| {
            self.request_body_rx.recv().map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to receive request body: {}", e))
            })
        })?;
        if body.closed {
            self.closed = true;
        }

        Ok(PyBytes::new(py, &body.body))
    }
}

/// The write callable returned by start_response which can be used to write response body imperatively.
///
/// It should not be commonly used anymore per the WSGI guidance but is luckily easy to implement.
///
/// https://peps.python.org/pep-3333/#the-write-callable
#[pyclass(module = "_pyvoy.wsgi")]
struct WriteCallable {
    response_bridge: EventBridge<ResponseEvent>,
    scheduler: Arc<SyncScheduler>,
}

#[pymethods]
impl WriteCallable {
    fn __call__<'py>(&mut self, data: Bound<'py, PyBytes>) -> PyResult<()> {
        let body: Box<[u8]> = Box::from(data.as_bytes());
        if self
            .response_bridge
            .send(ResponseEvent::Body(ResponseBodyEvent {
                body,
                more_body: true,
            }))
            .is_ok()
        {
            self.scheduler.commit(EVENT_ID_RESPONSE);
        }
        Ok(())
    }
}

/// The wsgi.errors output stream.
///
/// We simply write lines to the Envoy log as messages at error level.
/// It is expected that write is almost always called with full lines, but we
/// keep a buffer as well in case. We go ahead and flush the buffer when flush
/// is called even if it's not a full line in the end because it seems better
/// to have the output broken up over log lines than to be buffered.
///
/// Envoy seems to automatically trim trailing whitespace, but we'll assume
/// no one will notice that.
#[pyclass(module = "_pyvoy.wsgi")]
struct ErrorsOutput {
    /// A buffer to hold partial lines.
    buffer: Mutex<String>,
}

#[pymethods]
impl ErrorsOutput {
    fn write<'py>(&self, data: Bound<'py, PyString>) -> PyResult<usize> {
        let str = data.to_str()?;

        if str.is_empty() {
            return Ok(0);
        }

        let mut buffer = self.buffer.lock().unwrap();

        // Easiest case, we can output all the lines as is without touching the buffer.
        if str.ends_with('\n') && buffer.is_empty() {
            for line in str.split('\n') {
                if !line.is_empty() {
                    envoy_log_error!("{}", line);
                }
            }
            return Ok(str.len());
        }

        buffer.extend(str.chars());

        let to_write = if buffer.ends_with('\n') {
            std::mem::take(&mut *buffer)
        } else {
            match buffer.rfind('\n') {
                Some(idx) => {
                    let full_buffer = std::mem::take(&mut *buffer);
                    let (to_write, remaining) = full_buffer.split_at(idx + 1);
                    *buffer = remaining.to_string();
                    to_write.to_string()
                }
                None => {
                    // No full lines yet.
                    return Ok(str.len());
                }
            }
        };

        for line in to_write.split('\n') {
            if !line.is_empty() {
                envoy_log_error!("{}", line);
            }
        }

        Ok(str.len())
    }

    fn writelines<'py>(&self, lines: Bound<'py, PyAny>) -> PyResult<()> {
        self.flush()?;

        for item in lines.try_iter()? {
            let item = item?;
            let mut str = item.cast::<PyString>()?.to_str()?;

            if str.is_empty() {
                continue;
            }

            if str.ends_with('\n') {
                str = &str[..str.len() - 1];
            }

            envoy_log_error!("{}", str);
        }
        Ok(())
    }

    fn flush(&self) -> PyResult<()> {
        let buffer = &mut *self.buffer.lock().unwrap();
        if !buffer.is_empty() {
            for line in std::mem::take(&mut *buffer).split('\n') {
                if !line.is_empty() {
                    envoy_log_error!("{}", line);
                }
            }
        }
        Ok(())
    }
}

/// Wrapper to mark Receiver as Sync. PyO3 hackily uses Sync as a signal for whether
/// a type is safe to be used with the GIL detached, even though the thread doesn't change.
/// We know it is fine for our usage.
struct SyncReceiver<T>(Receiver<T>);

impl<T> std::ops::Deref for SyncReceiver<T> {
    type Target = Receiver<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

unsafe impl<T> Sync for SyncReceiver<T> {}
