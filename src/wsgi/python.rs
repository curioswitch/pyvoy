use executors::{Executor, crossbeam_channel_pool::ThreadPool};
use pyo3::{
    exceptions::PyRuntimeError,
    intern,
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyString, PyTuple},
};

use super::types::*;
use crate::types::*;
use crossbeam_channel::{Receiver, Sender};
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;

#[derive(Clone)]
pub(crate) struct PyExecutor {
    pool: ThreadPool,
    app_module: String,
    app_attr: String,
}

impl PyExecutor {
    pub(crate) fn new(app_module: &str, app_attr: &str, num_threads: usize) -> PyResult<Self> {
        let pool = ThreadPool::new(num_threads);

        Ok(Self {
            pool,
            app_module: app_module.to_string(),
            app_attr: app_attr.to_string(),
        })
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        request_read_tx: Sender<isize>,
        request_body_rx: Receiver<RequestBody>,
        response_tx: Sender<ResponseEvent>,
        request_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        response_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    ) {
        let app_module = self.app_module.clone();
        let app_attr = self.app_attr.clone();
        self.pool.execute(move || {
            let result: PyResult<()> = Python::attach(|py| {
                let app_module = py.import(app_module)?;
                let app = app_module.getattr(app_attr)?;

                let environ = PyDict::new(py);
                scope
                    .method
                    .set_in_dict(py, &environ, intern!(py, "REQUEST_METHOD"))?;

                // TODO: support root_path etc
                environ.set_item(intern!(py, "SCRIPT_NAME"), "")?;
                environ.set_item(
                    intern!(py, "PATH_INFO"),
                    String::from_utf8_lossy(&scope.raw_path),
                )?;
                if !scope.query_string.is_empty() {
                    environ.set_item(
                        intern!(py, "QUERY_STRING"),
                        String::from_utf8_lossy(&scope.query_string),
                    )?;
                }

                for (key, value) in scope.headers.iter() {
                    let value_str = String::from_utf8_lossy(value);
                    match &key[..] {
                        b"content-type" => {
                            environ.set_item(intern!(py, "CONTENT_TYPE"), value_str)?
                        }
                        b"content-length" => {
                            environ.set_item(intern!(py, "CONTENT_LENGTH"), value_str)?
                        }
                        _ => {
                            if key[0] == b':' {
                                continue;
                            }
                            let key_str = String::from_utf8_lossy(&key)
                                .to_uppercase()
                                .replace("-", "_");
                            let header_name = format!("HTTP_{}", key_str);
                            let value_str = String::from_utf8_lossy(&value);
                            match environ.get_item(&header_name)? {
                                Some(existing) => {
                                    let existing = existing.downcast::<PyString>()?;
                                    let new_value = format!("{},{}", existing.to_str()?, value_str);
                                    environ.set_item(header_name, new_value)?;
                                }
                                None => environ.set_item(header_name, value_str)?,
                            }
                        }
                    }
                }

                if let Some((server, port)) = scope.server {
                    environ.set_item(intern!(py, "SERVER_NAME"), server)?;
                    environ.set_item(intern!(py, "SERVER_PORT"), port.to_string())?;
                } else {
                    // In practice, should never be exercised.
                    environ.set_item(intern!(py, "SERVER_NAME"), "localhost")?;
                    environ.set_item(intern!(py, "SERVER_PORT"), "0")?;
                }

                environ.set_item(
                    intern!(py, "SERVER_PROTOCOL"),
                    match scope.http_version {
                        HttpVersion::Http10 => "HTTP/1.0",
                        HttpVersion::Http11 => "HTTP/1.1",
                        HttpVersion::Http2 => "HTTP/2",
                        HttpVersion::Http3 => "HTTP/3",
                    },
                )?;

                environ.set_item(intern!(py, "wsgi.version"), (1, 0))?;
                environ.set_item(
                    intern!(py, "wsgi.url_scheme"),
                    match scope.scheme {
                        HttpScheme::Http => "http",
                        HttpScheme::Https => "https",
                    },
                )?;
                environ.set_item(
                    intern!(py, "wsgi.input"),
                    RequestInput {
                        request_read_tx,
                        request_body_rx,
                        scheduler: request_scheduler,
                        closed: false,
                    },
                )?;

                // TODO: Support wsgi.errors

                environ.set_item(intern!(py, "wsgi.multithread"), true)?;
                environ.set_item(intern!(py, "wsgi.multiprocess"), false)?;
                environ.set_item(intern!(py, "wsgi.run_once"), false)?;

                let start_response = Bound::new(py, StartResponseCallable { headers: None })?;
                let response = app.call1((environ, start_response.borrow()))?;
                match response.len() {
                    Ok(0) => {
                        response_tx.send(ResponseEvent::Start(
                            ResponseStartEvent {
                                headers: start_response
                                    .borrow_mut()
                                    .headers
                                    .take()
                                    .unwrap_or_default(),
                            },
                            ResponseBodyEvent {
                                body: Box::from([]),
                                more_body: false,
                                notify: None,
                            },
                        ));
                        response_scheduler.commit(EVENT_ID_RESPONSE);
                    }
                    Ok(1) => {
                        let item =
                            response.try_iter()?.next().ok_or(PyRuntimeError::new_err(
                                "WSGI app returned empty iterator despite len() == 1",
                            ))??;
                        let body: Box<[u8]> = Box::from(item.downcast::<PyBytes>()?.as_bytes());
                        response_tx.send(ResponseEvent::Start(
                            ResponseStartEvent {
                                headers: start_response
                                    .borrow_mut()
                                    .headers
                                    .take()
                                    .unwrap_or_default(),
                            },
                            ResponseBodyEvent {
                                body,
                                more_body: false,
                                notify: None,
                            },
                        ));
                        response_scheduler.commit(EVENT_ID_RESPONSE);
                    }
                    _ => {
                        let mut started = false;
                        for item in response.try_iter()? {
                            let body: Box<[u8]> =
                                Box::from(item?.downcast::<PyBytes>()?.as_bytes());
                            let (tx, rx) = oneshot::channel::<()>();
                            match started {
                                false => {
                                    response_tx.send(ResponseEvent::Start(
                                        ResponseStartEvent {
                                            headers: start_response
                                                .borrow_mut()
                                                .headers
                                                .take()
                                                .unwrap_or_default(),
                                        },
                                        ResponseBodyEvent {
                                            body,
                                            more_body: true,
                                            notify: Some(tx),
                                        },
                                    ));
                                    started = true;
                                }
                                true => {
                                    response_tx.send(ResponseEvent::Body(ResponseBodyEvent {
                                        body,
                                        more_body: true,
                                        notify: Some(tx),
                                    }));
                                }
                            }
                            response_scheduler.commit(EVENT_ID_RESPONSE);
                            py.detach::<PyResult<()>, _>(|| {
                                rx.recv().map_err(|e| {
                                    PyRuntimeError::new_err(format!(
                                        "Failed to send response: {}",
                                        e
                                    ))
                                })?;
                                Ok(())
                            })?;
                        }
                        let (tx, rx) = oneshot::channel::<()>();
                        response_tx.send(ResponseEvent::Body(ResponseBodyEvent {
                            body: Box::from([]),
                            more_body: false,
                            notify: Some(tx),
                        }));
                        response_scheduler.commit(EVENT_ID_RESPONSE);
                        py.detach::<PyResult<()>, _>(|| {
                            rx.recv().map_err(|e| {
                                PyRuntimeError::new_err(format!("Failed to send response: {}", e))
                            })?;
                            Ok(())
                        })?;
                    }
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
    headers: Option<Vec<(String, Box<[u8]>)>>,
}

#[pymethods]
impl StartResponseCallable {
    #[pyo3(signature = (status, response_headers, exc_info=None))]
    fn __call__<'py>(
        &mut self,
        status: &str,
        response_headers: Bound<'py, PyList>,
        exc_info: Option<Bound<'py, PyTuple>>,
    ) -> PyResult<()> {
        let mut headers = Vec::with_capacity(response_headers.len() + 1);
        for item in response_headers.iter() {
            let key_item = item.get_item(0)?;
            let key = key_item.downcast::<PyString>()?;
            let value_item = item.get_item(1)?;
            let value = value_item.downcast::<PyString>()?;
            headers.push((key.to_string(), Box::from(value.to_str()?.as_bytes())));
        }

        let status_code = match status.split_once(' ') {
            Some((code_str, _)) => code_str,
            None => status,
        };
        headers.push((String::from(":status"), Box::from(status_code.as_bytes())));
        self.headers.replace(headers);
        // TODO: Return a write function.
        Ok(())
    }
}

#[pyclass]
struct RequestInput {
    request_read_tx: Sender<isize>,
    request_body_rx: Receiver<RequestBody>,
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

        match size {
            0 => Ok(PyBytes::new(py, &[])),
            _ => {
                self.request_read_tx.send(size);
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
    }
}
