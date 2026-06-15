use std::{
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard, atomic::AtomicU64},
};

use bytes::{Bytes, BytesMut};
use envoy_proxy_dynamic_modules_rust_sdk::{EnvoyBuffer, EnvoyHttpFilterScheduler};
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::{
    Bound, IntoPyObjectExt, Py, PyAny, PyErr, PyResult, Python,
    exceptions::{
        PyConnectionError, PyException, PyRuntimeError, PyStopAsyncIteration, PyValueError,
    },
    pyclass, pymethods,
    sync::MutexExt,
    types::{
        PyAnyMethods as _, PyBytes, PyDict, PyModule, PyModuleMethods as _, PyString,
        PyStringMethods,
    },
};
use url::Url;

use crate::{
    asgi::{
        python::EVENT_ID_OUTGOING_REQUEST,
        shared::awaitable::{EmptyAwaitable, ErrorAwaitable, ValueAwaitable},
    },
    eventbridge::EventBridge,
    types::Constants,
};

pub(crate) enum RequestBody {
    Buffered(Bytes),
    Iter(RequestContent),
}

pub(crate) struct StartStreamEvent {
    pub method: http::Method,
    pub url: Url,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: RequestBody,
    pub cluster_name: Arc<String>,
    pub timeout_ms: u64,
    pub response_future: Py<PyAny>,
    pub response_content: ResponseContent,
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

#[pyclass(module = "_pyvoy.asgi.httpclient", frozen)]
pub(crate) struct TransportBridge {
    loop_: Py<PyAny>,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
}

impl TransportBridge {
    pub(crate) fn new(
        loop_: Py<PyAny>,
        bridge: EventBridge<TransportEvent>,
        scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    ) -> Self {
        Self {
            loop_,
            bridge,
            scheduler,
        }
    }
}

pub(crate) fn register_py_module(py: Python<'_>) -> PyResult<()> {
    let asgi = PyModule::new(py, "pyvoy.asgi")?;
    let httpclient = PyModule::new(py, "pyvoy.asgi.httpclient")?;
    httpclient.add_class::<HTTPTransport>()?;
    asgi.setattr("httpclient", &httpclient)?;

    let sys_modules = py
        .import("sys")?
        .getattr("modules")?
        .cast_into::<PyDict>()?;
    sys_modules.set_item("pyvoy.asgi", asgi)?;
    sys_modules.set_item("pyvoy.asgi.httpclient", httpclient)?;

    Ok(())
}

/// The default upstream stream timeout when none is configured on the transport.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;

#[pyclass(module = "pyvoy.asgi.httpclient")]
struct HTTPTransport {
    cluster_name: Arc<String>,
    timeout_ms: u64,
    constants: Arc<Constants>,
}

#[pymethods]
impl HTTPTransport {
    #[new]
    #[pyo3(signature = (cluster_name, *, timeout=None))]
    fn py_new(py: Python<'_>, cluster_name: String, timeout: Option<f64>) -> Self {
        let timeout_ms = match timeout {
            Some(timeout) => (timeout * 1000.0) as u64,
            None => DEFAULT_TIMEOUT_MS,
        };
        Self {
            cluster_name: Arc::new(cluster_name),
            timeout_ms,
            constants: Constants::get(py),
        }
    }

    fn execute<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let transport_bridge = self
            .constants
            .transport_bridge_contextvar_get
            .bind(py)
            .call1((py.None(),))?;
        if transport_bridge.is_none() {
            return Err(PyRuntimeError::new_err(
                "TransportBridge not found in contextvars. This likely means the HTTP client was used outside of the context of the request.",
            ));
        }
        let transport_bridge = transport_bridge.cast::<TransportBridge>()?.get();
        let loop_ = transport_bridge.loop_.bind(py);
        let future = loop_.call_method0(&self.constants.create_future)?;

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
        let body = if let Ok(bytes) = request_body.extract::<Bytes>() {
            RequestBody::Buffered(bytes)
        } else {
            RequestBody::Iter(RequestContent::new(
                transport_bridge.bridge.clone(),
                &transport_bridge.scheduler,
                request_body.unbind(),
                loop_.clone().unbind(),
                self.constants.clone(),
            ))
        };

        let response_content = ResponseContent::new(
            loop_.clone().unbind(),
            transport_bridge.bridge.clone(),
            transport_bridge.scheduler.clone(),
            self.constants.clone(),
        );

        if transport_bridge
            .bridge
            .send(TransportEvent::Start(StartStreamEvent {
                method,
                url,
                headers,
                body,
                cluster_name: self.cluster_name.clone(),
                timeout_ms: self.timeout_ms,
                response_future: future.clone().unbind(),
                response_content,
            }))
            .is_ok()
        {
            transport_bridge.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
        }

        Ok(future)
    }
}

#[pyclass(module = "_pyvoy.asgi.httpclient", frozen)]
pub(crate) struct ReceivedResponseHeadersExecutor {
    status: StatusCode,
    headers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) response_future: Py<PyAny>,
    pub(crate) response_content: ResponseContent,
    request_content: Option<RequestContent>,
    end_stream: bool,
    constants: Arc<Constants>,
}

impl ReceivedResponseHeadersExecutor {
    pub(crate) fn new(
        status: StatusCode,
        headers: Vec<(HeaderName, HeaderValue)>,
        response_future: Py<PyAny>,
        response_content: ResponseContent,
        request_content: Option<RequestContent>,
        end_stream: bool,
        constants: Arc<Constants>,
    ) -> Self {
        Self {
            status,
            headers,
            response_future,
            response_content,
            request_content,
            end_stream,
            constants,
        }
    }
}

#[pymethods]
impl ReceivedResponseHeadersExecutor {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let response_future = self.response_future.bind(py);
        if response_future
            .call_method0(&self.constants.cancelled)?
            .extract::<bool>()?
        {
            if let Some(request_content) = &self.request_content {
                let task = request_content.inner.task.lock_py_attached(py).unwrap();
                if let Some(task) = task.as_ref() {
                    task.call_method0(py, &self.constants.cancel)?;
                }
            }
            return Ok(());
        }

        let headers = if !self.headers.is_empty() {
            let py_headers = self.constants.class_pyqwest_headers.bind(py).call0()?;
            for (name, value) in &self.headers {
                py_headers.call_method1(
                    &self.constants.add,
                    (name.as_str(), value.to_str().unwrap_or_default()),
                )?;
            }
            Some(py_headers)
        } else {
            None
        };
        let kwargs = PyDict::new(py);
        kwargs.set_item(&self.constants.status, self.status.as_u16())?;
        if let Some(headers) = headers {
            kwargs.set_item(&self.constants.headers, headers)?;
        }
        if !self.end_stream {
            let trailers = self.constants.class_pyqwest_headers.bind(py).call0()?;
            {
                let mut state = self
                    .response_content
                    .inner
                    .state
                    .lock_py_attached(py)
                    .unwrap();
                state.trailers = Some(trailers.clone().unbind());
            }
            kwargs.set_item(
                &self.constants.content,
                self.response_content.clone().into_bound_py_any(py)?,
            )?;
            kwargs.set_item(&self.constants.trailers, trailers)?;
        }

        let response = self
            .constants
            .class_pyqwest_response
            .bind(py)
            .call((), Some(&kwargs))?;

        if let Some(request_content) = &self.request_content {
            let task = request_content.inner.task.lock_py_attached(py).unwrap();
            if let Some(task) = task.as_ref() {
                response.call_method1(&self.constants.set_request_iter_task, (task,))?;
            }
        }

        response_future.call_method1(&self.constants.set_result, (response,))?;
        Ok(())
    }
}

pub struct TransportState {
    pub(super) request_content: Option<RequestContent>,
    pub(super) response_future: Option<Py<PyAny>>,
    pub(super) response_content: ResponseContent,
}

struct ResponseContentState {
    trailers: Option<Py<PyAny>>,
    pending_future: Option<Py<PyAny>>,
    body: BytesMut,
    http_trailers: Option<Vec<(HeaderName, HeaderValue)>>,
    end_stream: bool,
    reset_reason: Option<ResetReason>,
    exception: Option<Py<PyAny>>,
}

pub(crate) struct ResponseContentInner {
    pub(crate) loop_: Py<PyAny>,
    state: Mutex<ResponseContentState>,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    stream_handle: AtomicU64,
    pub(crate) constants: Arc<Constants>,
}

/// ResponseContent is the async iterator returned to Python for responses with content.
/// Because Envoy does not support buffering responses from HTTP client calls, we need to
/// eagerly create it when the request is started so the buffers are available as soon
/// as any data comes in, even before Python reads it.
#[pyclass(module = "_pyvoy.asgi.httpclient", frozen, skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct ResponseContent {
    pub(crate) inner: Arc<ResponseContentInner>,
}

impl ResponseContent {
    fn new(
        loop_: Py<PyAny>,
        bridge: EventBridge<TransportEvent>,
        scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
        constants: Arc<Constants>,
    ) -> Self {
        Self {
            inner: Arc::new(ResponseContentInner {
                loop_,
                state: Mutex::new(ResponseContentState {
                    trailers: None,
                    pending_future: None,
                    body: BytesMut::new(),
                    http_trailers: None,
                    end_stream: false,
                    reset_reason: None,
                    exception: None,
                }),
                bridge,
                scheduler,
                stream_handle: AtomicU64::new(0),
                constants,
            }),
        }
    }

    pub(crate) fn set_stream_handle(&self, handle: u64) {
        self.inner
            .stream_handle
            .store(handle, std::sync::atomic::Ordering::Relaxed);
    }

    /// Buffers response data to return to Python. Returns true if there is a pending Python future
    /// that needs to be notified via the event loop.
    pub(super) fn feed_response_data(&self, data: Vec<u8>, end_stream: bool) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        state.body.extend_from_slice(&data);
        state.end_stream |= end_stream;
        state.pending_future.is_some()
    }

    /// Buffers response trailers to return to Python. Returns true if there is a pending Python future
    /// that needs to be notified via the event loop.
    pub(super) fn feed_response_trailers(
        &self,
        envoy_trailers: &[(EnvoyBuffer, EnvoyBuffer)],
    ) -> bool {
        let mut trailers = Vec::with_capacity(envoy_trailers.len());
        for (name, value) in envoy_trailers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_slice()),
                HeaderValue::from_bytes(value.as_slice()),
            ) {
                trailers.push((name, value));
            }
        }
        let mut state = self.inner.state.lock().unwrap();
        state.http_trailers = Some(trailers);
        state.end_stream = true;
        state.pending_future.is_some()
    }

    pub(super) fn set_exception(&self, exception: Py<PyAny>) {
        let mut state = self.inner.state.lock().unwrap();
        state.exception = Some(exception);
    }

    pub(crate) fn take_exception(&self) -> Option<Py<PyAny>> {
        let mut state = self.inner.state.lock().unwrap();
        state.exception.take()
    }

    pub(super) fn reset(&self, reason: ResetReason) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        state.end_stream = true;
        state.reset_reason = Some(reason);
        state.pending_future.is_some()
    }

    fn maybe_copy_trailers_to_py(
        py: Python<'_>,
        state: &mut MutexGuard<ResponseContentState>,
        inner: &ResponseContentInner,
    ) -> PyResult<()> {
        if let Some(trailers) = state.http_trailers.take()
            && let Some(py_trailers) = &state.trailers
        {
            let py_trailers = py_trailers.bind(py);
            for (name, value) in &trailers {
                py_trailers.call_method1(
                    &inner.constants.add,
                    (name.as_str(), value.to_str().unwrap_or_default()),
                )?;
            }
        }
        Ok(())
    }
}

#[pymethods]
impl ResponseContent {
    fn __aiter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = &self.inner;
        let mut state = inner.state.lock_py_attached(py).unwrap();
        // There's really no use case for concurrently iterating an async body. Order can't be guaranteed,
        // anyways so we just return empty when there was already a pending future.
        if state.pending_future.is_some() {
            return ValueAwaitable::new_py(py, inner.constants.empty_bytes.bind(py));
        }

        Self::maybe_copy_trailers_to_py(py, &mut state, inner)?;

        if !state.body.is_empty() {
            let chunk = PyBytes::new(py, &state.body).into_any();
            state.body.clear();
            return ValueAwaitable::new_py(py, &chunk);
        }

        if let Some(exception) = &state.exception {
            return ErrorAwaitable::new_py(py, PyErr::from_value(exception.bind(py).clone()));
        }
        if let Some(reason) = state.reset_reason {
            return ErrorAwaitable::new_py(py, map_reset_reason(py, reason, &inner.constants)?);
        }

        if state.end_stream {
            return Err(PyStopAsyncIteration::new_err(()));
        }

        let future = self
            .inner
            .loop_
            .bind(py)
            .call_method0(&inner.constants.create_future)?;

        state.pending_future = Some(future.clone().unbind());

        Ok(future)
    }

    /// Called by the event loop when notifying of events from Envoy.
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let inner = &self.inner;
        let mut state = inner.state.lock_py_attached(py).unwrap();

        Self::maybe_copy_trailers_to_py(py, &mut state, inner)?;

        if state.body.is_empty() && !state.end_stream {
            return Ok(());
        }
        let Some(pending_future) = state.pending_future.take() else {
            return Ok(());
        };

        // Deliver any buffered data before surfacing errors or end of stream, matching the
        // ordering in `__anext__`. Otherwise a reset arriving alongside a final chunk while a
        // read is pending would drop the buffered data.
        if !state.body.is_empty() {
            let chunk = PyBytes::new(py, &state.body).into_any();
            state.body.clear();
            pending_future.call_method1(py, &inner.constants.set_result, (chunk,))?;
        } else if let Some(exception) = &state.exception {
            pending_future.call_method1(
                py,
                &inner.constants.set_exception,
                (PyErr::from_value(exception.bind(py).clone()),),
            )?;
        } else if let Some(reason) = state.reset_reason {
            pending_future.call_method1(
                py,
                &inner.constants.set_exception,
                (map_reset_reason(py, reason, &inner.constants)?,),
            )?;
        } else {
            // state.end_stream
            pending_future.call_method1(
                py,
                &inner.constants.set_exception,
                (PyStopAsyncIteration::new_err(()),),
            )?;
        }
        Ok(())
    }

    #[getter]
    fn _read_pending(&self, py: Python<'_>) -> PyResult<bool> {
        let state = self.inner.state.lock_py_attached(py).unwrap();
        Ok(state.pending_future.is_some())
    }

    /// Called by pyqwest when the response is closed. If the stream has not already finished,
    /// the response was closed before being fully consumed so we reset the upstream stream to
    /// release it rather than leaving it open and buffering data until the timeout.
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = &self.inner;
        let state = inner.state.lock_py_attached(py).unwrap();
        if !state.end_stream {
            let stream_handle = inner
                .stream_handle
                .load(std::sync::atomic::Ordering::Relaxed);
            if inner
                .bridge
                .send(TransportEvent::Reset(SendStreamResetEvent {
                    stream_handle,
                    exception: None,
                }))
                .is_ok()
            {
                inner.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
            }
        }
        EmptyAwaitable::new_py(py)
    }
}

#[pyclass(module = "_pyvoy.asgi.httpclient", frozen)]
struct RequestContentForwarder {
    stream_handle: u64,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
}

#[pymethods]
impl RequestContentForwarder {
    fn __call__<'py>(
        &self,
        py: Python<'py>,
        data: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        // TODO: Backpressure when envoy dynamic modules support it
        if let Ok(status) = data.extract::<u32>() {
            match status {
                1 => {
                    if self
                        .bridge
                        .send(TransportEvent::Reset(SendStreamResetEvent {
                            stream_handle: self.stream_handle,
                            exception: None,
                        }))
                        .is_ok()
                    {
                        self.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
                    }
                }
                _ => unreachable!(),
            }
            return EmptyAwaitable::new_py(py);
        } else if let Ok(exception) = data.cast::<PyException>() {
            if self
                .bridge
                .send(TransportEvent::Reset(SendStreamResetEvent {
                    stream_handle: self.stream_handle,
                    exception: Some(exception.clone().into_any().unbind()),
                }))
                .is_ok()
            {
                self.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
            }
            return EmptyAwaitable::new_py(py);
        }
        let (body, end_stream) = if data.is_none() {
            (Bytes::new(), true)
        } else {
            (data.extract::<Bytes>()?, false)
        };
        if self
            .bridge
            .send(TransportEvent::SendData(SendStreamDataEvent {
                stream_handle: self.stream_handle,
                data: body,
                end_stream,
            }))
            .is_ok()
        {
            self.scheduler.commit(EVENT_ID_OUTGOING_REQUEST);
        }
        EmptyAwaitable::new_py(py)
    }
}

pub(crate) struct RequestContentInner {
    stream_handle: AtomicU64,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    request_iter: Option<Py<PyAny>>,
    pub(crate) loop_: Py<PyAny>,
    task: Mutex<Option<Py<PyAny>>>,
    constants: Arc<Constants>,
}

#[pyclass(module = "_pyvoy.asgi.httpclient", from_py_object, frozen)]
#[derive(Clone)]
pub(crate) struct RequestContent {
    pub(crate) inner: Arc<RequestContentInner>,
}

impl RequestContent {
    pub(crate) fn new(
        bridge: EventBridge<TransportEvent>,
        scheduler: &Arc<Box<dyn EnvoyHttpFilterScheduler>>,
        request_iter: Py<PyAny>,
        loop_: Py<PyAny>,
        constants: Arc<Constants>,
    ) -> Self {
        Self {
            inner: Arc::new(RequestContentInner {
                stream_handle: AtomicU64::new(0),
                bridge,
                scheduler: scheduler.clone(),
                request_iter: Some(request_iter),
                loop_,
                task: Mutex::new(None),
                constants,
            }),
        }
    }

    pub(crate) fn set_stream_handle(&self, handle: u64) {
        self.inner
            .stream_handle
            .store(handle, std::sync::atomic::Ordering::Relaxed);
    }
}

#[pymethods]
impl RequestContent {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        if let Some(request_iter) = &self.inner.request_iter {
            let request_content = RequestContentForwarder {
                stream_handle: self
                    .inner
                    .stream_handle
                    .load(std::sync::atomic::Ordering::Relaxed),
                bridge: self.inner.bridge.clone(),
                scheduler: self.inner.scheduler.clone(),
            }
            .into_bound_py_any(py)?;
            let coro = self
                .inner
                .constants
                .glue_forward_bytes
                .bind(py)
                .call1((&request_iter, request_content))?;
            let task = self
                .inner
                .loop_
                .bind(py)
                .call_method1(&self.inner.constants.create_task, (coro,))?;
            self.inner
                .task
                .lock_py_attached(py)
                .unwrap()
                .replace(task.unbind());
        }
        Ok(())
    }
}

#[pyclass]
pub(crate) struct SetResponseFutureException {
    pub(crate) future: Py<PyAny>,
    pub(crate) reason: ResetReason,
    pub(crate) exception: Option<Py<PyAny>>,
    pub(crate) request_content: Option<RequestContent>,
    pub(crate) constants: Arc<Constants>,
}

#[pymethods]
impl SetResponseFutureException {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let err = if let Some(exception) = &self.exception {
            PyErr::from_value(exception.bind(py).clone())
        } else {
            map_reset_reason(py, self.reason, &self.constants)?
        };
        self.future
            .call_method1(py, &self.constants.set_exception, (err,))?;
        if let Some(request_content) = &self.request_content {
            let task = request_content.inner.task.lock_py_attached(py).unwrap();
            if let Some(task) = task.as_ref() {
                task.call_method0(py, &self.constants.cancel)?;
            }
        }
        Ok(())
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
