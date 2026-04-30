use std::{
    str::FromStr,
    sync::{Arc, Mutex},
};

use bytes::{Bytes, BytesMut};
use envoy_proxy_dynamic_modules_rust_sdk::{EnvoyBuffer, EnvoyHttpFilterScheduler};
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::{
    Bound, IntoPyObjectExt, Py, PyAny, PyResult, Python,
    exceptions::{PyRuntimeError, PyStopAsyncIteration, PyValueError},
    pybacked::PyBackedBytes,
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
        python::{self, EVENT_ID_OUTGOING_REQUEST, LoopFuture},
        shared::awaitable::{EmptyAwaitable, ValueAwaitable},
    },
    eventbridge::EventBridge,
    types::Constants,
};

struct CanceledFuture {
    future: Option<LoopFuture>,
    executor: python::Executor,
}

impl Drop for CanceledFuture {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            self.executor.handle_canceled_future(future);
        }
    }
}

pub(super) enum RequestBody {
    Empty,
    Buffered(PyBackedBytes),
    Iter(Py<PyAny>),
}

pub(crate) struct StartStreamEvent {
    pub method: http::Method,
    pub url: Url,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: RequestBody,
    pub cluster_name: Option<Arc<String>>,
    pub response_future: Py<PyAny>,
    pub response_content: ResponseContent,
}

pub(crate) struct SendStreamDataEvent {
    pub stream_handle: u64,
    pub data: Bytes,
    pub end_stream: bool,
}

pub(super) struct OnHttpStreamheadersEvent {
    pub stream_handle: u64,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub end_stream: bool,
}

pub(super) struct OnHttpStreamDataEvent {
    pub stream_handle: u64,
    pub data: Vec<u8>,
    pub end_stream: bool,
}

pub(crate) enum TransportEvent {
    Start(StartStreamEvent),
    SendData(SendStreamDataEvent),
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

#[pyclass(module = "pyvoy.asgi.httpclient")]
struct HTTPTransport {
    cluster_name: Option<Arc<String>>,
    constants: Arc<Constants>,
}

#[pymethods]
impl HTTPTransport {
    #[new]
    #[pyo3(signature = (*, cluster_name=None))]
    fn py_new(py: Python<'_>, cluster_name: Option<String>) -> Self {
        Self {
            cluster_name: cluster_name.map(Arc::new),
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
                headers.push((name, value));
            }
        }

        let request_body = request.getattr(&self.constants.content)?;
        let body = if request_body.is_none() {
            RequestBody::Empty
        } else if let Ok(bytes) = request_body.extract::<PyBackedBytes>() {
            RequestBody::Buffered(bytes)
        } else {
            RequestBody::Iter(request_body.unbind())
        };

        let response_content = ResponseContent::new(loop_.clone().unbind(), self.constants.clone());

        if transport_bridge
            .bridge
            .send(TransportEvent::Start(StartStreamEvent {
                method,
                url,
                headers,
                body,
                cluster_name: self.cluster_name.clone(),
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
pub(crate) struct StreamStartExecutor {
    stream_handle: u64,
    headers: Vec<(HeaderName, HeaderValue)>,
    request_iter: Option<Py<PyAny>>,
    pub(crate) response_future: Py<PyAny>,
    pub(crate) response_content: ResponseContent,
    end_stream: bool,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
    constants: Arc<Constants>,
}

impl StreamStartExecutor {
    pub(crate) fn new(
        stream_handle: u64,
        headers: Vec<(HeaderName, HeaderValue)>,
        request_iter: Option<Py<PyAny>>,
        response_future: Py<PyAny>,
        response_content: ResponseContent,
        end_stream: bool,
        bridge: EventBridge<TransportEvent>,
        scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        constants: Arc<Constants>,
    ) -> Self {
        Self {
            stream_handle,
            headers,
            request_iter,
            response_future,
            response_content,
            end_stream,
            bridge,
            scheduler: Arc::new(scheduler),
            constants,
        }
    }
}

#[pymethods]
impl StreamStartExecutor {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        println!(
            "Called StreamStartExecutor with headers: {:#?}, end_stream={}",
            self.headers, self.end_stream
        );
        let status = self
            .headers
            .iter()
            .find(|(name, _)| name == ":status")
            .and_then(|(_, value)| value.to_str().ok())
            .and_then(|s| StatusCode::from_bytes(s.as_bytes()).ok())
            .unwrap_or(StatusCode::OK);
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
        kwargs.set_item(&self.constants.status, status.as_u16())?;
        if let Some(headers) = headers {
            kwargs.set_item(&self.constants.headers, headers)?;
        }
        if !self.end_stream {
            if let Some(request_iter) = &self.request_iter {
                let request_content = RequestContent {
                    stream_handle: self.stream_handle,
                    bridge: self.bridge.clone(),
                    scheduler: self.scheduler.clone(),
                }
                .into_bound_py_any(py)?;
                let coro = self
                    .constants
                    .glue_forward_bytes
                    .bind(py)
                    .call1((&request_iter, request_content))?;
                // TODO: Cancel
                println!("Creating task for request body iteration");
                self.response_content
                    .inner
                    .loop_
                    .bind(py)
                    .call_method1(&self.constants.create_task, (coro,))?;
            }

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
        self.response_future
            .call_method1(py, &self.constants.set_result, (response,))?;
        Ok(())
    }
}

pub struct TransportState {
    pub(super) request_iter: Option<Py<PyAny>>,
    pub(super) response_future: Option<Py<PyAny>>,
    pub(super) response_content: ResponseContent,
}

struct ResponseContentState {
    trailers: Option<Py<PyAny>>,
    pending_future: Option<Py<PyAny>>,
    body: BytesMut,
    http_trailers: Option<Vec<(HeaderName, HeaderValue)>>,
    end_stream: bool,
}

pub(crate) struct ResponseContentInner {
    pub(crate) loop_: Py<PyAny>,
    state: Mutex<ResponseContentState>,
    constants: Arc<Constants>,
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
    fn new(loop_: Py<PyAny>, constants: Arc<Constants>) -> Self {
        Self {
            inner: Arc::new(ResponseContentInner {
                loop_,
                state: Mutex::new(ResponseContentState {
                    trailers: None,
                    pending_future: None,
                    body: BytesMut::new(),
                    http_trailers: None,
                    end_stream: false,
                }),
                constants,
            }),
        }
    }

    /// Buffers response data to return to Python. Returns true if there is a pending Python future
    /// that needs to be notified via the event loop.
    pub(super) fn feed_response_data(&self, data: Vec<u8>, end_stream: bool) -> bool {
        println!(
            "Feeding response data, len={}, end_stream={}",
            data.len(),
            end_stream
        );
        let mut state = self.inner.state.lock().unwrap();
        state.body.extend_from_slice(&data);
        state.end_stream |= end_stream;
        println!(
            "Pending future: {}, body len={}",
            state.pending_future.is_some(),
            state.body.len()
        );
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
        state.pending_future.is_some()
    }
}

#[pymethods]
impl ResponseContent {
    fn __aiter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        println!("Called ResponseContent __anext__");
        let inner = &self.inner;
        let mut state = inner.state.lock_py_attached(py).unwrap();
        // There's really no use case for concurrently iterating an async body. Order can't be guaranteed,
        // anyways so we just return empty when there was already a pending future.
        if state.pending_future.is_some() {
            println!("Already a pending future, returning empty");
            return ValueAwaitable::new_py(py, inner.constants.empty_bytes.bind(py));
        }

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

        if !state.body.is_empty() {
            println!("Returning body chunk of len {}", state.body.len());
            let chunk = PyBytes::new(py, &state.body).into_any();
            state.body.clear();
            return ValueAwaitable::new_py(py, &chunk);
        }

        if state.end_stream {
            println!("End of stream reached, returning StopAsyncIteration");
            return Err(PyStopAsyncIteration::new_err(()));
        }

        println!("Creating future for pending response data");

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
        println!("Called ResponseContent __call__");
        let inner = &self.inner;
        let mut state = inner.state.lock_py_attached(py).unwrap();
        if state.body.is_empty() && !state.end_stream {
            return Ok(());
        }
        let Some(pending_future) = state.pending_future.take() else {
            return Ok(());
        };
        if !state.body.is_empty() {
            let chunk = PyBytes::new(py, &state.body).into_any();
            state.body.clear();
            pending_future.call_method1(py, &inner.constants.set_result, (chunk,))?;
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
}

#[pyclass(module = "_pyvoy.asgi.httpclient", frozen)]
struct RequestContent {
    stream_handle: u64,
    bridge: EventBridge<TransportEvent>,
    scheduler: Arc<Box<dyn EnvoyHttpFilterScheduler>>,
}

#[pymethods]
impl RequestContent {
    fn __call__<'py>(
        &self,
        py: Python<'py>,
        data: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        // TODO: Backpressure
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
