use std::{
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use bytes::Bytes;
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::{
    Bound, IntoPyObjectExt, Py, PyAny, PyErr, PyResult, Python,
    exceptions::{
        PyConnectionError, PyRuntimeError, PyStopIteration, PyTimeoutError, PyValueError,
    },
    pyclass, pymethods,
    sync::MutexExt as _,
    types::{
        PyAnyMethods as _, PyBytes, PyDict, PyModule, PyModuleMethods as _, PyString,
        PyStringMethods as _,
    },
};
use url::Url;

use crate::{
    eventbridge::EventBridge,
    types::{Constants, SyncReceiver},
    wsgi::{pool::PoolHandle, types::EVENT_ID_OUTGOING_REQUEST},
};

pub(crate) struct StartStreamEvent {
    pub method: http::Method,
    pub url: Url,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub buffered_body: Option<Bytes>,
    pub cluster_name: Arc<String>,
    pub timeout_ms: u64,
    pub response_tx: Sender<TransportResponse>,
}

pub(crate) struct SendStreamDataEvent {
    pub stream_handle: u64,
    pub data: Bytes,
    pub end_stream: bool,
}

pub(crate) struct SendStreamResetEvent {
    pub stream_handle: u64,
    pub exception: Option<Py<PyAny>>,
}

pub(crate) enum TransportEvent {
    Start(StartStreamEvent),
    SendData(SendStreamDataEvent),
    Reset(SendStreamResetEvent),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ResetReason {
    Local,
    Remote,
    Protocol,
    Connection,
    Overflow,
    ServerRequestDone,
    InvalidUpstream,
}

/// An event delivered from the filter to the blocked application thread over a
/// per-request channel. `Started` is always sent first, followed by `Headers`,
/// body chunks, and finally an `End`, `Trailers`, or `Reset`.
pub(crate) enum TransportResponse {
    Started(u64),
    Headers {
        status: StatusCode,
        headers: Vec<(HeaderName, HeaderValue)>,
        end_stream: bool,
    },
    Body(Box<[u8]>),
    Trailers(Vec<(HeaderName, HeaderValue)>),
    End,
    Reset {
        reason: ResetReason,
        exception: Option<Py<PyAny>>,
    },
}

pub(crate) struct TransportState {
    pub(crate) response_tx: Sender<TransportResponse>,
    pub(crate) exception: Option<Py<PyAny>>,
}

/// Iterates a streaming request body on a pool thread, sending each chunk to the
/// upstream stream.
struct ForwardRequest {
    request_iter: Py<PyAny>,
    stream_handle: u64,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    constants: Arc<Constants>,
}

impl ForwardRequest {
    fn run(self, py: Python<'_>) {
        let result = (|| -> PyResult<()> {
            for item in self.request_iter.bind(py).try_iter()? {
                let data = item?.extract::<Bytes>()?;
                self.send_data(data, false);
            }
            Ok(())
        })();
        match result {
            Ok(()) => self.send_data(Bytes::new(), true),
            Err(e) => {
                let exception = Some(write_error(py, &self.constants, e));
                if self
                    .bridge
                    .send(TransportEvent::Reset(SendStreamResetEvent {
                        stream_handle: self.stream_handle,
                        exception,
                    }))
                    .is_ok()
                {
                    self.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
                }
            }
        }
    }

    fn send_data(&self, data: Bytes, end_stream: bool) {
        if self
            .bridge
            .send(TransportEvent::SendData(SendStreamDataEvent {
                stream_handle: self.stream_handle,
                data,
                end_stream,
            }))
            .is_ok()
        {
            self.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
        }
    }
}

#[pyclass(module = "_pyvoy.wsgi.httpclient", frozen)]
pub(crate) struct TransportBridge {
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    pool: PoolHandle,
}

impl TransportBridge {
    pub(crate) fn new(
        bridge: EventBridge<TransportEvent>,
        scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
        pool: PoolHandle,
    ) -> Self {
        Self {
            bridge,
            scheduler,
            pool,
        }
    }
}

pub(crate) fn register_py_module(py: Python<'_>) -> PyResult<()> {
    let wsgi = PyModule::new(py, "pyvoy.wsgi")?;
    let httpclient = PyModule::new(py, "pyvoy.wsgi.httpclient")?;
    httpclient.add_class::<HTTPTransport>()?;
    wsgi.setattr("httpclient", &httpclient)?;

    let sys_modules = py
        .import("sys")?
        .getattr("modules")?
        .cast_into::<PyDict>()?;
    sys_modules.set_item("pyvoy.wsgi", wsgi)?;
    sys_modules.set_item("pyvoy.wsgi.httpclient", httpclient)?;

    Ok(())
}

/// The default upstream stream timeout when none is configured on the transport.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;

#[pyclass(module = "pyvoy.wsgi.httpclient")]
struct HTTPTransport {
    cluster_name: Arc<String>,
    timeout_ms: u64,
    constants: Arc<Constants>,
}

#[pymethods]
impl HTTPTransport {
    #[new]
    #[pyo3(signature = (cluster_name, *, timeout=None))]
    fn py_new(py: Python<'_>, cluster_name: String, timeout: Option<f64>) -> PyResult<Self> {
        let timeout_ms = match timeout {
            Some(timeout) => {
                if !timeout.is_finite() || timeout < 0.0 {
                    return Err(PyValueError::new_err("timeout must be non-negative"));
                }
                (timeout * 1000.0) as u64
            }
            None => DEFAULT_TIMEOUT_MS,
        };
        Ok(Self {
            cluster_name: Arc::new(cluster_name),
            timeout_ms,
            constants: Constants::get(py),
        })
    }

    fn execute_sync<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let transport_bridge = self
            .constants
            .wsgi_transport_bridge_contextvar_get
            .bind(py)
            .call1((py.None(),))?;
        if transport_bridge.is_none() {
            return Err(PyRuntimeError::new_err(
                "TransportBridge not found in contextvars. This likely means the HTTP client was used outside of the context of the request.",
            ));
        }
        let transport_bridge = transport_bridge.cast::<TransportBridge>()?.get();

        let req_headers = request.getattr(&self.constants.headers)?;

        let mut headers: Vec<(HeaderName, HeaderValue)> = Vec::with_capacity(req_headers.len()?);
        let method = http::Method::from_str(
            request
                .getattr(&self.constants.method)?
                .cast::<PyString>()?
                .to_str()?,
        )
        .map_err(|_| PyValueError::new_err("invalid HTTP method"))?;

        let request_url = request.getattr(&self.constants.url)?;
        let url = url::Url::parse(request_url.cast::<PyString>()?.to_str()?)
            .map_err(|_| PyValueError::new_err("invalid request URL"))?;

        let mut has_content_type = false;

        for item in req_headers
            .call_method0(&self.constants.items)?
            .try_iter()?
        {
            let item = item?;
            let name_py = item.get_item(0)?;
            let name_py_str = name_py.cast::<PyString>()?.to_str()?;
            let value_py = item.get_item(1)?;
            let value_py_str = value_py.cast::<PyString>()?.to_str()?;
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_str(name_py_str),
                HeaderValue::from_str(value_py_str),
            ) {
                if name == http::header::CONTENT_TYPE {
                    has_content_type = true;
                }
                headers.push((name, value));
            }
        }

        if !has_content_type && request.getattr(&self.constants.json)?.is_truthy()? {
            headers.push((
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            ));
        }

        let request_body = request.getattr(&self.constants.content)?;
        let buffered_body = request_body.extract::<Bytes>().ok();
        let streaming = buffered_body.is_none();
        // A per-request timeout set by the synchronous client takes precedence over the
        // transport's default. We enforce it ourselves for the right error type, and also
        // pass it to Envoy so the upstream stream is released.
        let timeout = self.sync_timeout(py)?;
        let deadline = timeout.map(|d| Instant::now() + d);
        let timeout_ms = timeout
            .map(|d| (d.as_millis() as u64).max(1))
            .unwrap_or(self.timeout_ms);
        // Owns the request iterator and closes it whenever this closer is dropped, which
        // guarantees the iteration pool worker is unblocked however the request ends.
        // Moves into the response below on success, otherwise drops on the paths here.
        let request_iter = RequestIterCloser {
            iter: streaming.then(|| request_body.clone().unbind()),
            constants: self.constants.clone(),
        };

        let (response_tx, response_rx) = std::sync::mpsc::channel::<TransportResponse>();
        if transport_bridge
            .bridge
            .send(TransportEvent::Start(StartStreamEvent {
                method,
                url,
                headers,
                buffered_body,
                cluster_name: self.cluster_name.clone(),
                timeout_ms,
                response_tx,
            }))
            .is_ok()
        {
            transport_bridge.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
        }
        let response_rx = SyncReceiver::new(response_rx);

        let stream_handle = match recv_deadline(py, &response_rx, deadline) {
            Received::Value(TransportResponse::Started(handle)) => handle,
            Received::Value(TransportResponse::Reset { reason, exception }) => {
                return Err(self.reset_error(py, reason, exception)?);
            }
            Received::TimedOut => {
                // Without a handle there is no stream to reset; if one was started, Envoy's
                // stream timeout, set to this same timeout, releases it.
                return Err(PyTimeoutError::new_err("request timed out"));
            }
            _ => return Err(PyRuntimeError::new_err("transport closed")),
        };

        if let Some(request_iter) = &request_iter.iter {
            let forward = ForwardRequest {
                request_iter: request_iter.clone_ref(py),
                stream_handle,
                bridge: transport_bridge.bridge.clone(),
                scheduler: transport_bridge.scheduler.clone(),
                constants: self.constants.clone(),
            };
            transport_bridge
                .pool
                .execute(move || Python::attach(|py| forward.run(py)));
        }

        match recv_deadline(py, &response_rx, deadline) {
            Received::Value(TransportResponse::Headers {
                status,
                headers,
                end_stream,
            }) => self.build_response(
                py,
                status,
                headers,
                end_stream,
                response_rx,
                stream_handle,
                deadline,
                request_iter,
                transport_bridge,
            ),
            Received::Value(TransportResponse::Reset { reason, exception }) => {
                Err(self.reset_error(py, reason, exception)?)
            }
            Received::TimedOut => {
                reset_stream(
                    &transport_bridge.bridge,
                    &transport_bridge.scheduler,
                    stream_handle,
                );
                Err(PyTimeoutError::new_err("request timed out"))
            }
            _ => Err(PyRuntimeError::new_err("transport closed")),
        }
    }
}

impl HTTPTransport {
    /// Reads the per-request timeout set by the synchronous client, if any.
    fn sync_timeout(&self, py: Python<'_>) -> PyResult<Option<Duration>> {
        let timeout = self.constants.get_sync_timeout.bind(py).call0()?;
        if timeout.is_none() {
            return Ok(None);
        }
        let secs: f64 = timeout
            .call_method0(&self.constants.total_seconds)?
            .extract()?;
        Ok(Some(if secs > 0.0 {
            Duration::from_secs_f64(secs)
        } else {
            Duration::ZERO
        }))
    }

    fn reset_error(
        &self,
        py: Python<'_>,
        reason: ResetReason,
        exception: Option<Py<PyAny>>,
    ) -> PyResult<PyErr> {
        match exception {
            Some(exception) => Ok(PyErr::from_value(exception.bind(py).clone())),
            None => map_reset_reason(py, reason, &self.constants),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_response<'py>(
        &self,
        py: Python<'py>,
        status: StatusCode,
        headers: Vec<(HeaderName, HeaderValue)>,
        end_stream: bool,
        response_rx: SyncReceiver<TransportResponse>,
        stream_handle: u64,
        deadline: Option<Instant>,
        request_iter: RequestIterCloser,
        transport_bridge: &TransportBridge,
    ) -> PyResult<Bound<'py, PyAny>> {
        let kwargs = PyDict::new(py);
        kwargs.set_item(&self.constants.status, status.as_u16())?;
        if !headers.is_empty() {
            let py_headers = self.constants.class_pyqwest_headers.bind(py).call0()?;
            for (name, value) in &headers {
                py_headers.call_method1(
                    &self.constants.add,
                    (name.as_str(), value.to_str().unwrap_or_default()),
                )?;
            }
            kwargs.set_item(&self.constants.headers, py_headers)?;
        }
        if !end_stream {
            let trailers = self.constants.class_pyqwest_headers.bind(py).call0()?;
            let content = ResponseContent {
                rx: response_rx,
                stream_handle,
                bridge: transport_bridge.bridge.clone(),
                scheduler: transport_bridge.scheduler.clone(),
                trailers: trailers.clone().unbind(),
                deadline,
                request_iter: Mutex::new(request_iter.take()),
                ended: AtomicBool::new(false),
                constants: self.constants.clone(),
            };
            kwargs.set_item(&self.constants.content, content.into_bound_py_any(py)?)?;
            kwargs.set_item(&self.constants.trailers, trailers)?;
        }
        self.constants
            .class_pyqwest_sync_response
            .bind(py)
            .call((), Some(&kwargs))
    }
}

/// ResponseContent is the iterator returned to Python for responses with content.
/// It blocks on the per-request channel until data, trailers, an end of stream,
/// or a reset arrives.
#[pyclass(module = "_pyvoy.wsgi.httpclient", frozen)]
struct ResponseContent {
    rx: SyncReceiver<TransportResponse>,
    stream_handle: u64,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    trailers: Py<PyAny>,
    deadline: Option<Instant>,
    // The streaming request body iterator, closed when this response is closed.
    request_iter: Mutex<Option<Py<PyAny>>>,
    ended: AtomicBool,
    constants: Arc<Constants>,
}

#[pymethods]
impl ResponseContent {
    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __next__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        if self.ended.load(Ordering::Relaxed) {
            return Err(PyStopIteration::new_err(()));
        }
        loop {
            match recv_deadline(py, &self.rx, self.deadline) {
                Received::Value(TransportResponse::Body(chunk)) => {
                    return Ok(PyBytes::new(py, &chunk));
                }
                Received::Value(TransportResponse::Trailers(trailers)) => {
                    let py_trailers = self.trailers.bind(py);
                    for (name, value) in &trailers {
                        py_trailers.call_method1(
                            &self.constants.add,
                            (name.as_str(), value.to_str().unwrap_or_default()),
                        )?;
                    }
                    self.ended.store(true, Ordering::Relaxed);
                    return Err(PyStopIteration::new_err(()));
                }
                Received::Value(TransportResponse::End) => {
                    self.ended.store(true, Ordering::Relaxed);
                    return Err(PyStopIteration::new_err(()));
                }
                Received::Value(TransportResponse::Reset { reason, exception }) => {
                    self.ended.store(true, Ordering::Relaxed);
                    return Err(match exception {
                        Some(exception) => PyErr::from_value(exception.bind(py).clone()),
                        None => map_reset_reason(py, reason, &self.constants)?,
                    });
                }
                Received::Value(_) => continue,
                Received::TimedOut => {
                    self.close(py);
                    return Err(PyTimeoutError::new_err("response read timed out"));
                }
                Received::Closed => {
                    self.ended.store(true, Ordering::Relaxed);
                    return Err(map_reset_reason(py, ResetReason::Local, &self.constants)?);
                }
            }
        }
    }

    /// Called by pyqwest when the response is closed. If the stream has not already finished,
    /// the response was closed before being fully consumed so we reset the upstream stream to
    /// release it rather than leaving it open and buffering data until the timeout, and close
    /// the request iterator to unblock the iteration pool worker if it is still reading.
    fn close(&self, py: Python<'_>) {
        self.reset();
        let iter = self.request_iter.lock_py_attached(py).unwrap().take();
        close_request_iter(py, &self.constants, iter);
    }
}

impl ResponseContent {
    /// Resets the upstream stream if it has not already finished.
    fn reset(&self) {
        if !self.ended.swap(true, Ordering::Relaxed) {
            reset_stream(&self.bridge, &self.scheduler, self.stream_handle);
        }
    }
}

impl Drop for ResponseContent {
    fn drop(&mut self) {
        // Backstop for a response destroyed without being closed or fully consumed, e.g.
        // garbage collected after an error. close() handles the deterministic paths.
        self.reset();
        if let Some(iter) = self.request_iter.get_mut().unwrap().take() {
            Python::attach(|py| close_request_iter(py, &self.constants, Some(iter)));
        }
    }
}

/// The outcome of a deadline-aware receive on the response channel.
enum Received {
    Value(TransportResponse),
    TimedOut,
    Closed,
}

/// Receives the next response event, blocking with the GIL released. If `deadline` is set
/// and passes first, returns `TimedOut`.
fn recv_deadline(
    py: Python<'_>,
    rx: &SyncReceiver<TransportResponse>,
    deadline: Option<Instant>,
) -> Received {
    py.detach(|| match deadline {
        None => match rx.recv() {
            Ok(value) => Received::Value(value),
            Err(_) => Received::Closed,
        },
        Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
            None => Received::TimedOut,
            Some(remaining) => match rx.recv_timeout(remaining) {
                Ok(value) => Received::Value(value),
                Err(RecvTimeoutError::Timeout) => Received::TimedOut,
                Err(RecvTimeoutError::Disconnected) => Received::Closed,
            },
        },
    })
}

fn reset_stream(
    bridge: &EventBridge<TransportEvent>,
    scheduler: &Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    stream_handle: u64,
) {
    if bridge
        .send(TransportEvent::Reset(SendStreamResetEvent {
            stream_handle,
            exception: None,
        }))
        .is_ok()
    {
        scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
    }
}

/// Closes a streaming request body iterator, unblocking the iteration pool worker if it is
/// still reading. Closing is idempotent, so calling it redundantly is harmless. `None` (a
/// non-streaming request) is a no-op.
fn close_request_iter(py: Python<'_>, constants: &Constants, iter: Option<Py<PyAny>>) {
    if let Some(iter) = iter {
        let _ = constants
            .glue_close_request_iterator
            .bind(py)
            .call1((iter.bind(py),));
    }
}

/// Owns a streaming request body iterator and closes it when dropped. Being a drop guard
/// guarantees the iteration pool worker is unblocked however the request ends — client
/// close, timeout, error, or filter teardown — rather than only on paths that remember to
/// do it explicitly. On success ownership of the iterator moves to the response via `take`.
struct RequestIterCloser {
    iter: Option<Py<PyAny>>,
    constants: Arc<Constants>,
}

impl RequestIterCloser {
    fn take(mut self) -> Option<Py<PyAny>> {
        self.iter.take()
    }
}

impl Drop for RequestIterCloser {
    fn drop(&mut self) {
        if self.iter.is_some() {
            let iter = self.iter.take();
            Python::attach(|py| close_request_iter(py, &self.constants, iter));
        }
    }
}

fn write_error(py: Python<'_>, constants: &Constants, cause: PyErr) -> Py<PyAny> {
    let value = cause.value(py);
    let message = value.str().map(|s| s.to_string()).unwrap_or_default();
    match constants
        .class_pyqwest_write_error
        .bind(py)
        .call1((message,))
    {
        Ok(err) => {
            let _ = err.setattr("__cause__", value);
            err.unbind()
        }
        Err(_) => value.clone().into_any().unbind(),
    }
}

fn map_reset_reason(py: Python<'_>, reason: ResetReason, constants: &Constants) -> PyResult<PyErr> {
    let res = match reason {
        ResetReason::Local => PyErr::from_value(
            constants
                .class_pyqwest_read_error
                .bind(py)
                .call1(("local stream reset",))?,
        ),
        ResetReason::Remote => PyErr::from_value(
            constants
                .class_pyqwest_read_error
                .bind(py)
                .call1(("remote stream reset",))?,
        ),
        ResetReason::Protocol => PyErr::from_value(
            constants
                .class_pyqwest_read_error
                .bind(py)
                .call1(("protocol error",))?,
        ),
        ResetReason::Connection => PyConnectionError::new_err("connection error"),
        ResetReason::Overflow => PyErr::from_value(
            constants
                .class_pyqwest_read_error
                .bind(py)
                .call1(("stream overflow",))?,
        ),
        ResetReason::ServerRequestDone => PyErr::from_value(
            constants
                .class_pyqwest_read_error
                .bind(py)
                .call1(("server request completed before client response",))?,
        ),
        ResetReason::InvalidUpstream => PyErr::from_value(
            constants
                .class_pyqwest_read_error
                .bind(py)
                .call1(("invalid upstream config",))?,
        ),
    };
    Ok(res)
}
