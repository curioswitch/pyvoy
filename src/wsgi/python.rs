use executors::{Executor, crossbeam_channel_pool::ThreadPool};
use http::{HeaderName, HeaderValue};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyString, PyTuple},
};

use super::types::*;
use crate::types::*;
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender},
};

#[derive(Clone)]
pub(crate) struct PyExecutor {
    pool: ThreadPool,
    app_module: Box<str>,
    app_attr: Box<str>,
    constants: Arc<Constants>,
}

impl PyExecutor {
    pub(crate) fn new(
        app_module: &str,
        app_attr: &str,
        num_threads: usize,
        constants: Arc<Constants>,
    ) -> PyResult<Self> {
        let pool = ThreadPool::new(num_threads);

        Ok(Self {
            pool,
            app_module: Box::from(app_module),
            app_attr: Box::from(app_attr),
            constants,
        })
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        request_read_tx: Sender<isize>,
        request_body_rx: Receiver<RequestBody>,
        response_tx: Sender<ResponseEvent>,
        response_written_rx: Receiver<()>,
        request_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        response_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    ) {
        let app_module = self.app_module.clone();
        let app_attr = self.app_attr.clone();
        let request_body_rx = Mutex::new(request_body_rx);
        let response_written_rx = Mutex::new(response_written_rx);
        let constants = self.constants.clone();
        self.pool.execute(move || {
            let result: PyResult<()> = Python::attach(|py| {
                let app_module = py.import(&app_module[..])?;
                let app = app_module.getattr(&app_attr[..])?;

                let environ = PyDict::new(py);
                environ.set_http_method(
                    py,
                    &constants,
                    constants.request_method.bind(py),
                    &scope.method,
                )?;

                // TODO: support root_path etc
                environ.set_item(constants.script_name.bind(py), "")?;
                environ.set_item(
                    constants.path_info.bind(py),
                    PyString::from_bytes(py, &scope.raw_path)?,
                )?;
                if !scope.query_string.is_empty() {
                    environ.set_item(
                        constants.query_string.bind(py),
                        PyString::from_bytes(py, &scope.query_string)?,
                    )?;
                }

                for (key, value) in scope.headers.iter() {
                    let value_str = PyString::from_bytes(py, value)?;
                    match &key[..] {
                        b"content-type" => {
                            environ.set_item(constants.content_type.bind(py), value_str)?
                        }
                        b"content-length" => {
                            environ.set_item(constants.content_length.bind(py), value_str)?
                        }
                        _ => {
                            if key[0] == b':' {
                                continue;
                            }
                            let key_str = String::from_utf8_lossy(key)
                                .to_uppercase()
                                .replace("-", "_");
                            let header_name = format!("HTTP_{}", key_str);
                            let value_str = String::from_utf8_lossy(value);
                            if let Some(existing) = environ.get_item(&header_name)? {
                                let existing = existing.cast::<PyString>()?;
                                let new_value = format!("{},{}", existing.to_str()?, value_str);
                                environ.set_item(header_name, new_value)?;
                            } else {
                                environ.set_item(header_name, value_str)?;
                            }
                        }
                    }
                }

                if let Some((server, port)) = scope.server {
                    environ.set_item(constants.server_name.bind(py), &server[..])?;
                    environ.set_item(constants.server_port.bind(py), port.to_string())?;
                } else {
                    // In practice, should never be exercised.
                    environ.set_item(constants.server_name.bind(py), "localhost")?;
                    environ.set_item(constants.server_port.bind(py), "0")?;
                }

                environ.set_http_version(
                    py,
                    &constants,
                    constants.server_protocol.bind(py),
                    &scope.http_version,
                )?;

                environ.set_item(constants.wsgi_version.bind(py), (1, 0))?;
                environ.set_http_scheme(
                    py,
                    &constants,
                    constants.wsgi_url_scheme.bind(py),
                    &scope.scheme,
                )?;
                environ.set_item(
                    constants.wsgi_input.bind(py),
                    RequestInput {
                        request_read_tx,
                        request_body_rx,
                        scheduler: request_scheduler,
                        closed: false,
                    },
                )?;

                // TODO: Support wsgi.errors

                environ.set_item(constants.wsgi_multithread.bind(py), true)?;
                environ.set_item(constants.wsgi_multiprocess.bind(py), false)?;
                environ.set_item(constants.wsgi_run_once.bind(py), false)?;

                let start_response = Bound::new(
                    py,
                    StartResponseCallable {
                        status: 0,
                        headers: None,
                    },
                )?;
                let response = app.call1((environ, start_response.borrow()))?;

                // We ignore all channel errors here since they only happen if the filter was dropped meaning
                // the request was closed, usually by the client. This is not an application error, and we just need
                // to make sure a close() method for a generator is called before returning.

                match response.len() {
                    Ok(0) => {
                        let mut start_response = start_response.borrow_mut();
                        let _ = response_tx.send(ResponseEvent::Start(
                            ResponseStartEvent {
                                status: start_response.status,
                                headers: start_response.headers.take().unwrap_or_default(),
                            },
                            ResponseBodyEvent {
                                body: Box::from([]),
                                more_body: false,
                            },
                        ));
                        response_scheduler.commit(EVENT_ID_RESPONSE);
                    }
                    Ok(1) => {
                        let item =
                            response.try_iter()?.next().ok_or(PyRuntimeError::new_err(
                                "WSGI app returned empty iterator despite len() == 1",
                            ))??;
                        let body: Box<[u8]> = Box::from(item.cast::<PyBytes>()?.as_bytes());
                        let mut start_response = start_response.borrow_mut();
                        let _ = response_tx.send(ResponseEvent::Start(
                            ResponseStartEvent {
                                status: start_response.status,
                                headers: start_response.headers.take().unwrap_or_default(),
                            },
                            ResponseBodyEvent {
                                body,
                                more_body: false,
                            },
                        ));
                        response_scheduler.commit(EVENT_ID_RESPONSE);
                    }
                    _ => {
                        let mut started = false;
                        for item in response.try_iter()? {
                            let body: Box<[u8]> = Box::from(item?.cast::<PyBytes>()?.as_bytes());
                            if !started {
                                let mut start_response = start_response.borrow_mut();
                                let _ = response_tx.send(ResponseEvent::Start(
                                    ResponseStartEvent {
                                        status: start_response.status,
                                        headers: start_response.headers.take().unwrap_or_default(),
                                    },
                                    ResponseBodyEvent {
                                        body,
                                        more_body: true,
                                    },
                                ));
                                started = true;
                            } else {
                                let _ = response_tx.send(ResponseEvent::Body(ResponseBodyEvent {
                                    body,
                                    more_body: true,
                                }));
                            }
                            response_scheduler.commit(EVENT_ID_RESPONSE);
                            py.detach(|| {
                                let _ = response_written_rx.lock().unwrap().recv();
                            });
                        }
                        let _ = response_tx.send(ResponseEvent::Body(ResponseBodyEvent {
                            body: Box::from([]),
                            more_body: false,
                        }));
                        response_scheduler.commit(EVENT_ID_RESPONSE);
                        py.detach(|| {
                            // RecvError only is request filter was dropped, but since this
                            // is the end always safe to ignore.
                            let _ = response_written_rx.lock().unwrap().recv();
                        });
                    }
                }

                if let Ok(close) = response.getattr(constants.close.bind(py)) {
                    close.call0()?;
                }

                Ok(())
            });
            if let Err(e) = result {
                eprintln!("WSGI application error: {}", e);
                let _ = response_tx.send(ResponseEvent::Exception);
                response_scheduler.commit(EVENT_ID_EXCEPTION);
            }
        });
    }
}

#[pyclass]
struct StartResponseCallable {
    status: u16,
    headers: Option<Vec<(HeaderName, HeaderValue)>>,
}

#[pymethods]
impl StartResponseCallable {
    #[pyo3(signature = (status, response_headers, _exc_info=None))]
    fn __call__<'py>(
        &mut self,
        status: &str,
        response_headers: Bound<'py, PyList>,
        _exc_info: Option<Bound<'py, PyTuple>>,
    ) -> PyResult<()> {
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
        self.status = u16::from_str_radix(status_code, 10)
            .map_err(|e| PyValueError::new_err(format!("invalid status code: {}", e)))?;
        self.headers.replace(headers);
        // TODO: Return a write function.
        Ok(())
    }
}

#[pyclass]
struct RequestInput {
    request_read_tx: Sender<isize>,
    request_body_rx: Mutex<Receiver<RequestBody>>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    closed: bool,
}

unsafe impl Sync for RequestInput {}

#[pymethods]
impl RequestInput {
    #[pyo3(signature = (size=-1))]
    fn read<'py>(&mut self, py: Python<'py>, size: Option<isize>) -> PyResult<Bound<'py, PyBytes>> {
        if self.closed {
            return Ok(PyBytes::new(py, &[]));
        }

        let size = size.unwrap_or(-1);

        if size == 0 {
            Ok(PyBytes::new(py, &[]))
        } else {
            let _ = self.request_read_tx.send(size);
            self.scheduler.commit(EVENT_ID_REQUEST);

            let body = py.detach::<PyResult<RequestBody>, _>(|| {
                self.request_body_rx.lock().unwrap().recv().map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to receive request body: {}", e))
                })
            })?;
            if body.closed {
                self.closed = true;
            }

            Ok(PyBytes::new(py, &body.body))
        }
    }
}
