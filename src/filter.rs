use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::{mem, thread};

use envoy_proxy_dynamic_modules_rust_sdk::{abi::envoy_dynamic_module_type_attribute_id, *};
use pyo3::{
    exceptions::PyRuntimeError,
    intern,
    prelude::*,
    types::{PyDict, PyList, PyNone},
};

pub struct Config {
    loop_: Py<PyAny>,
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,
}

impl Config {
    pub fn new(filter_config: &str) -> Option<Self> {
        let (tx, rx) = sync_channel(0);
        thread::spawn(move || {
            let res: PyResult<()> = Python::attach(|py| {
                let asyncio = PyModule::import(py, "asyncio")?;
                let loop_ = asyncio.call_method0("new_event_loop")?;
                tx.send(loop_.clone().unbind()).unwrap();
                asyncio.call_method1("set_event_loop", (&loop_,))?;
                loop_.call_method0("run_forever")?;
                Ok(())
            });
            res.unwrap();
        });

        let loop_ = match rx.recv() {
            Ok(loop_) => loop_,
            Err(e) => {
                eprintln!("Failed to create Python event loop thread: {}", e);
                return None;
            }
        };

        match Python::attach(|py| {
            let (module, attr) = filter_config
                .split_once(":")
                .unwrap_or((filter_config, "app"));
            let module = PyModule::import(py, module)?;
            let app = module.getattr(attr)?;
            let asgi = PyDict::new(py);
            asgi.set_item("version", "3.0")?;
            asgi.set_item("spec_version", "2.2")?;
            let extensions = PyDict::new(py);
            extensions.set_item("http.response.trailers", PyDict::new(py))?;
            Ok::<_, PyErr>((app.unbind(), asgi.unbind(), extensions.unbind()))
        }) {
            Ok((app, asgi, extensions)) => Some(Self {
                loop_,
                app,
                asgi,
                extensions,
            }),
            Err(e) => {
                eprintln!("Failed to load Python application: {}", e);
                None
            }
        }
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&mut self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        let (loop_, app, asgi, extensions) = Python::attach(|py| {
            (
                self.loop_.clone_ref(py),
                self.app.clone_ref(py),
                self.asgi.clone_ref(py),
                self.extensions.clone_ref(py),
            )
        });
        let (request_future_tx, request_future_rx) = channel::<Py<PyAny>>();
        let (response_tx, response_rx) = channel::<ResponseEvent>();
        Box::new(Filter {
            loop_,
            app,
            asgi,
            extensions,
            request_closed: false,
            process_response: true,
            response_headers: Vec::new(),
            response_trailers: None,
            request_future_rx: request_future_rx,
            request_future_tx: Some(request_future_tx),
            response_rx: response_rx,
            response_tx: Some(response_tx),
        })
    }
}

const EVENT_ID_REQUEST: u64 = 1;
const EVENT_ID_RESPONSE: u64 = 2;

struct Filter {
    loop_: Py<PyAny>,
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    extensions: Py<PyDict>,

    request_closed: bool,
    process_response: bool,

    response_headers: Vec<(String, Vec<u8>)>,
    response_trailers: Option<Vec<(String, Vec<u8>)>>,

    request_future_rx: Receiver<Py<PyAny>>,
    request_future_tx: Option<Sender<Py<PyAny>>>,
    response_rx: Receiver<ResponseEvent>,
    response_tx: Option<Sender<ResponseEvent>>,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        let trailers_accepted = match envoy_filter.get_request_header_value("te") {
            Some(te) => te.as_slice() == b"trailers",
            None => false,
        };
        self.execute_app(envoy_filter, trailers_accepted);
        if end_of_stream {
            self.request_closed = true;
            abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::ContinueAndDontEndStream
        } else {
            abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::Continue
        }
    }

    fn on_request_body(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        if end_of_stream {
            self.request_closed = true;
        }
        if has_request_body(envoy_filter) {
            match self.request_future_rx.try_recv() {
                Ok(future) => {
                    process_request_future(envoy_filter, &self.loop_, future, !end_of_stream)
                        .unwrap();
                }
                Err(_) => {}
            }
        }
        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_response_headers(
        &mut self,
        envoy_filter: &mut EHF,
        _end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_response_headers_status {
        envoy_filter.remove_response_header("trailer");
        for (k, v) in mem::take(&mut self.response_headers) {
            envoy_filter.set_response_header(k.as_str(), v.as_slice());
        }
        self.process_response = true;
        envoy_filter.new_scheduler().commit(EVENT_ID_RESPONSE);
        abi::envoy_dynamic_module_type_on_http_filter_response_headers_status::Continue
    }

    fn on_response_trailers(
        &mut self,
        envoy_filter: &mut EHF,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_response_trailers_status {
        envoy_filter.remove_response_trailer("P");
        for (k, v) in self.response_trailers.take().unwrap_or_default() {
            envoy_filter.set_response_trailer(k.as_str(), v.as_slice());
        }
        abi::envoy_dynamic_module_type_on_http_filter_response_trailers_status::Continue
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        if event_id == EVENT_ID_REQUEST {
            if self.request_closed || has_request_body(envoy_filter) {
                match self.request_future_rx.try_recv() {
                    Ok(future) => {
                        process_request_future(
                            envoy_filter,
                            &self.loop_,
                            future,
                            !self.request_closed,
                        )
                        .unwrap();
                    }
                    Err(_) => {}
                }
            }
            return;
        }
        if !self.process_response {
            // Headers not received from upstream, wait for them before processing events.
            return;
        }
        // We delay closing the response stream when needed until the end of the function since
        // an end stream can cause the filter to be deallocated right away, invalidating further code.
        let mut close_response = false;
        while let Ok(event) = self.response_rx.try_recv() {
            match event {
                ResponseEvent::Start(event) => {
                    if event.trailers {
                        self.response_trailers.replace(Vec::new());
                    }
                    self.response_headers.push((
                        String::from(":status"),
                        event.status.to_string().into_bytes(),
                    ));
                    for (k, v) in event.headers {
                        if k.eq_ignore_ascii_case("trailer") && !event.trailers {
                            // Skip trailers header if we won't be sending any.
                            continue;
                        }
                        self.response_headers.push((k, v));
                    }
                    // Triggers upstream to send headers. After we encode them, we will be
                    // ready to continue processing messages to completion.
                    envoy_filter.inject_request_body(b" ", false);
                    self.process_response = false;
                    return;
                }
                ResponseEvent::Body(event) => {
                    envoy_filter.inject_response_body(&event.body, false);
                    let end_stream = !event.more_body && self.response_trailers.is_none();
                    if end_stream {
                        close_response = true;
                    }
                    set_future_to_none(&self.loop_, event.future).unwrap();
                }
                ResponseEvent::Trailers(mut event) => {
                    if let Some(trailers) = &mut self.response_trailers {
                        trailers.append(&mut event.headers);
                    }
                    if !event.more_trailers {
                        close_response = true;
                    }
                }
            }
        }
        if close_response {
            // Closing the request stream to upstream causes it to finish the response,
            // which will also allow processing any trailers.
            envoy_filter.inject_request_body(b"", true);
        }
    }
}

impl Filter {
    fn execute_app<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &EHF, trailers_accepted: bool) {
        let res: PyResult<()> = Python::attach(|py| {
            let scope = PyDict::new(py);
            scope.set_item(intern!(py, "type"), intern!(py, "http"))?;
            scope.set_item(intern!(py, "asgi"), self.asgi.bind(py))?;
            scope.set_item(intern!(py, "extensions"), self.extensions.bind(py))?;
            if let Some(method) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestMethod)
            {
                scope.set_item(
                    intern!(py, "method"),
                    String::from_utf8_lossy(method.as_slice()),
                )?;
            }
            if let Some(scheme) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestScheme)
            {
                scope.set_item(
                    intern!(py, "scheme"),
                    String::from_utf8_lossy(scheme.as_slice()),
                )?;
            }
            if let Some(path) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestUrlPath)
            {
                let decoded_path = urlencoding::decode_binary(path.as_slice());
                scope.set_item(intern!(py, "path"), String::from_utf8_lossy(&decoded_path))?;
                scope.set_item(intern!(py, "raw_path"), path.as_slice())?;
            }
            if let Some(query) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestQuery)
            {
                scope.set_item(intern!(py, "query"), query.as_slice())?;
            } else {
                scope.set_item(intern!(py, "query"), b"")?;
            }
            scope.set_item(intern!(py, "root_path"), "")?;
            let headers = PyList::new(
                py,
                envoy_filter
                    .get_request_headers()
                    .iter()
                    .map(|(k, v)| (k.as_slice(), v.as_slice())),
            )?;
            scope.set_item(intern!(py, "headers"), headers)?;

            set_scope_address(
                py,
                &scope,
                envoy_filter,
                "client",
                envoy_dynamic_module_type_attribute_id::SourceAddress,
                envoy_dynamic_module_type_attribute_id::SourcePort,
            )?;

            set_scope_address(
                py,
                &scope,
                envoy_filter,
                "server",
                envoy_dynamic_module_type_attribute_id::DestinationAddress,
                envoy_dynamic_module_type_attribute_id::DestinationPort,
            )?;

            let app = self.app.bind(py);
            let coro = app.call1((
                scope,
                ASGIReceiveCallable {
                    request_future_tx: self.request_future_tx.take().unwrap(),
                    scheduler: envoy_filter.new_scheduler(),
                    loop_: self.loop_.clone_ref(py),
                },
                ASGISendCallable {
                    next_event: NextASGIEvent::Start,
                    trailers_accepted,
                    response_tx: self.response_tx.take().unwrap(),
                    scheduler: envoy_filter.new_scheduler(),
                    loop_: self.loop_.clone_ref(py),
                },
            ))?;
            let asyncio = PyModule::import(py, intern!(py, "asyncio"))?;
            asyncio.call_method1(
                intern!(py, "run_coroutine_threadsafe"),
                (coro, &self.loop_.bind(py)),
            )?;
            Ok(())
        });
        res.unwrap()
    }

    fn send_response_headers<EHF: EnvoyHttpFilter>(
        &mut self,
        envoy_filter: &mut EHF,
        start_event: ResponseStartEvent,
    ) {
        envoy_filter.set_response_header(":status", start_event.status.to_string().as_bytes());
        for (k, v) in start_event.headers {
            if k.eq_ignore_ascii_case("trailer") && self.response_trailers.is_none() {
                // Skip trailers header if we won't be sending any.
                continue;
            }
            envoy_filter.set_response_header(k.as_str(), v.as_slice());
        }
        self.process_response = false;
        envoy_filter.new_scheduler().commit(EVENT_ID_RESPONSE);
    }
}

fn set_scope_address<EHF: EnvoyHttpFilter>(
    py: Python,
    scope: &Bound<PyDict>,
    envoy_filter: &EHF,
    key: &str,
    address_attr_id: envoy_dynamic_module_type_attribute_id,
    port_attr_id: envoy_dynamic_module_type_attribute_id,
) -> PyResult<()> {
    match (
        envoy_filter.get_attribute_string(address_attr_id),
        envoy_filter.get_attribute_int(port_attr_id),
    ) {
        (Some(host), Some(port)) => {
            let host_str = String::from_utf8_lossy(host.as_slice()).to_string();
            let mut host = host_str.as_str();
            if let Some(colon_idx) = host.find(":") {
                host = &host[..colon_idx];
            }
            scope.set_item(key, (host, port))
        }
        _ => scope.set_item(key, PyNone::get(py)),
    }
}

fn set_future_to_none(loop_: &Py<PyAny>, future: Py<PyAny>) -> PyResult<()> {
    Python::attach(|py| {
        let future = future.bind(py);
        let set_result = future.getattr(intern!(py, "set_result"))?;
        loop_.bind(py).call_method1(
            intern!(py, "call_soon_threadsafe"),
            (set_result, PyNone::get(py)),
        )?;
        Ok(())
    })
}

fn has_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> bool {
    if let Some(buffers) = envoy_filter.get_request_body() {
        for buffer in buffers {
            if !buffer.as_slice().is_empty() {
                return true;
            }
        }
    }
    return false;
}

fn process_request_future<EHF: EnvoyHttpFilter>(
    envoy_filter: &mut EHF,
    loop_: &Py<PyAny>,
    future: Py<PyAny>,
    more_body: bool,
) -> PyResult<()> {
    let mut body = Vec::new();
    if let Some(buffers) = envoy_filter.get_request_body() {
        for buffer in buffers {
            body.extend_from_slice(buffer.as_slice());
        }
    }
    envoy_filter.drain_request_body(body.len());
    Python::attach(|py| {
        let future = future.bind(py);
        let set_result = future.getattr(intern!(py, "set_result"))?;
        let event = PyDict::new(py);
        event.set_item(intern!(py, "type"), intern!(py, "http.request"))?;
        event.set_item(intern!(py, "body"), body)?;
        event.set_item(intern!(py, "more_body"), more_body)?;
        loop_
            .bind(py)
            .call_method1(intern!(py, "call_soon_threadsafe"), (set_result, event))?;
        Ok(())
    })
}

#[pyclass]
struct ASGIReceiveCallable {
    request_future_tx: Sender<Py<PyAny>>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    loop_: Py<PyAny>,
}

unsafe impl Sync for ASGIReceiveCallable {}

#[pymethods]
impl ASGIReceiveCallable {
    fn __call__(&self) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let future = self
                .loop_
                .bind(py)
                .call_method0(intern!(py, "create_future"))?;
            self.request_future_tx
                .send(future.clone().unbind())
                .unwrap();
            self.scheduler.commit(EVENT_ID_REQUEST);
            Ok(future.unbind())
        })
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
struct ASGISendCallable {
    next_event: NextASGIEvent,
    trailers_accepted: bool,
    response_tx: Sender<ResponseEvent>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    loop_: Py<PyAny>,
}

unsafe impl Sync for ASGISendCallable {}

#[pymethods]
impl ASGISendCallable {
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
                    let status: u16 = match event.get_item(intern!(py, "status"))? {
                        Some(v) => v.extract()?,
                        None => {
                            return Err(PyRuntimeError::new_err(
                                "Unexpected ASGI message, missing 'status' in 'http.response.start'.",
                            ));
                        }
                    };
                    let headers = extract_headers_from_event(py, &event)?;
                    let trailers: bool = match event.get_item(intern!(py, "trailers"))? {
                        Some(v) => v.extract()?,
                        None => false,
                    };
                    match trailers {
                        true => self.next_event = NextASGIEvent::BodyWithTrailers,
                        false => self.next_event = NextASGIEvent::BodyWithoutTrailers,
                    };
                    self.response_tx
                        .send(ResponseEvent::Start(ResponseStartEvent {
                            status,
                            headers: headers,
                            trailers: trailers && self.trailers_accepted,
                        }))
                        .unwrap();
                    self.scheduler.commit(EVENT_ID_RESPONSE);
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
                    let body: Vec<u8> = match event.get_item(intern!(py, "body"))? {
                        Some(body) => body.extract()?,
                        _ => Vec::new(),
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
                    self.response_tx
                        .send(ResponseEvent::Body(ResponseBodyEvent {
                            body: body.clone(),
                            more_body,
                            future: future.clone().unbind(),
                        }))
                        .unwrap();
                    self.scheduler.commit(EVENT_ID_RESPONSE);
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
                        let headers = extract_headers_from_event(py, &event)?;
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

struct ResponseStartEvent {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    trailers: bool,
}

struct ResponseBodyEvent {
    body: Vec<u8>,
    more_body: bool,
    future: Py<PyAny>,
}

struct ResponseTrailersEvent {
    headers: Vec<(String, Vec<u8>)>,
    more_trailers: bool,
}

enum ResponseEvent {
    Start(ResponseStartEvent),
    Body(ResponseBodyEvent),
    Trailers(ResponseTrailersEvent),
}

fn extract_headers_from_event(
    py: Python,
    event: &Bound<PyDict>,
) -> PyResult<Vec<(String, Vec<u8>)>> {
    match event.get_item(intern!(py, "headers"))? {
        Some(v) => {
            let mut headers = Vec::new();
            for item in v.try_iter()? {
                let tuple = item?;
                let key_item = tuple.get_item(0)?;
                let value_item = tuple.get_item(1)?;
                let key_bytes = key_item.downcast::<pyo3::types::PyBytes>()?;
                let value_bytes = value_item.downcast::<pyo3::types::PyBytes>()?;
                headers.push((
                    String::from_utf8_lossy(key_bytes.as_bytes()).to_string(),
                    value_bytes.as_bytes().to_vec(),
                ));
            }
            Ok(headers)
        }
        None => Ok(Vec::new()),
    }
}
