use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::thread;

use envoy_proxy_dynamic_modules_rust_sdk::{abi::envoy_dynamic_module_type_attribute_id, *};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyNone},
};

pub struct Config {
    loop_: Py<PyAny>,
    app: Py<PyAny>,
    asgi: Py<PyDict>,
}

impl Config {
    pub fn new(_filter_config: &str) -> Option<Self> {
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

        let loop_ = rx.recv().unwrap();

        match Python::attach(|py| {
            let module = PyModule::import(py, "hello")?;
            let app = module.getattr("app")?;
            let asgi = PyDict::new(py);
            asgi.set_item("version", "3.0")?;
            asgi.set_item("spec_version", "2.2")?;
            Ok::<_, PyErr>((app.unbind(), asgi.unbind()))
        }) {
            Ok((app, asgi)) => Some(Self { loop_, app, asgi }),
            Err(e) => {
                eprintln!("Failed to load Python application: {}", e);
                None
            }
        }
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&mut self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        let (loop_, app, asgi) = Python::attach(|py| {
            (
                self.loop_.clone_ref(py),
                self.app.clone_ref(py),
                self.asgi.clone_ref(py),
            )
        });
        Box::new(Filter {
            loop_,
            app,
            asgi,
            sent_response_headers: false,
            start_response_event: None,
            request_future_rx: None,
            response_rx: None,
            response_buffer: None,
            process_response_buffer: false,
        })
    }
}

const EVENT_ID_REQUEST: u64 = 1;
const EVENT_ID_RESPONSE: u64 = 2;

struct Filter {
    loop_: Py<PyAny>,
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    sent_response_headers: bool,
    start_response_event: Option<ResponseStartEvent>,

    request_future_rx: Option<Receiver<Py<PyAny>>>,

    response_rx: Option<Receiver<ResponseEvent>>,
    response_buffer: Option<Vec<ResponseBodyEvent>>,
    process_response_buffer: bool,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &mut self,
        envoy_filter: &mut EHF,
        _end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        println!("OReqH");
        self.execute_app(envoy_filter);
        envoy_filter.remove_request_header("content-length");
        abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::Continue
    }

    fn on_request_body(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        println!("OReqB");
        println!("{}", end_of_stream);
        if has_request_body(envoy_filter) && let Some(request_future_rx) = self.request_future_rx.as_ref() {
            println!("OReqB2");
            match request_future_rx.try_recv() {
                Ok(future) => {
                    process_request_future(envoy_filter, &self.loop_, future, !end_of_stream).unwrap();
                },
                Err(_) => println!("No future available1"),
            }
        }
        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_response_headers(
        &mut self,
        envoy_filter: &mut EHF,
        _end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_response_headers_status {
        println!("OResH");
        let mut header_keys: Vec<String> = Vec::new();
        for (key, _) in envoy_filter.get_response_headers() {
            header_keys.push(String::from_utf8_lossy(key.as_slice()).as_ref().to_string());
        }
        for key in header_keys {
            envoy_filter.remove_response_header(key.as_str());
        }
        if let Some(start_event) = self.start_response_event.take() {
            envoy_filter.set_response_header(":status", start_event.status.to_string().as_bytes());
            for (k, v) in start_event.headers {
                envoy_filter.set_response_header(k.as_str(), v.as_slice());
            }
            self.process_response_buffer = true;
            envoy_filter.new_scheduler().commit(EVENT_ID_RESPONSE);
        }
        self.sent_response_headers = true;
        abi::envoy_dynamic_module_type_on_http_filter_response_headers_status::Continue
    }

    fn on_response_body(
        &mut self,
        _envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_response_body_status {
        println!("OResB");
        println!("{}", end_of_stream);
        // Ignore upstream's response completely.
        abi::envoy_dynamic_module_type_on_http_filter_response_body_status::StopIterationNoBuffer
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        println!("OS1 {}", event_id);
        if event_id == EVENT_ID_REQUEST {
            println!("OS1.0");
            if has_request_body(envoy_filter) && let Some(request_future_rx) = self.request_future_rx.as_ref() {
                println!("OS1.1");
                match request_future_rx.try_recv() {
                    Ok(future) => {
                        println!("OS1.2");
                        process_request_future(envoy_filter, &self.loop_, future, false).unwrap();
                    },
                    Err(_) => println!("No future available2"),
                }
            }
            return;
        }
        if self.process_response_buffer {
            println!("OS2");
            if let Some(buffer) = self.response_buffer.take() {
                println!("OS3");
                for event in buffer {
                    // TODO: It's better to resolve the future after injecting, but if the stream
                    // ends, the filter immediately becomes invalid including the loop_ reference.
                    // Instead of an independent loop_, it should be possible to borrow the Config's
                    // reference somehow.
                    set_future_to_none(&self.loop_, event.future).unwrap();
                    println!("{}", envoy_filter.inject_response_body(&event.body, !event.more_body));
                }
            }
            self.process_response_buffer = false;
            return;
        }
        let response_rx = self.response_rx.as_ref().unwrap();
        println!("OS4");
        if let Ok(event) = response_rx.recv() {
            println!("OS5");
            match event {
                ResponseEvent::Start(event) => {
                    println!("OS6");
                    self.start_response_event.replace(event);
                }
                ResponseEvent::Body(event) => {
                    println!("OS7");
                    match self.sent_response_headers {
                        true => {
                            println!("OS8");
                            set_future_to_none(&self.loop_, event.future).unwrap();
                            println!("{}", envoy_filter.inject_response_body(&event.body, !event.more_body));
                        },
                        false => match self.response_buffer {
                            Some(ref mut buffer) => {
                                println!("OS9");
                                buffer.push(event);
                            },
                            None => {
                                println!("OS10");
                                let buffer = vec![event];
                                self.response_buffer.replace(buffer);
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Filter {
    fn execute_app<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &EHF) {
        let res: PyResult<()> = Python::attach(|py| {
            let scope = PyDict::new(py);
            scope.set_item("type", "http")?;
            scope.set_item("asgi", self.asgi.bind(py))?;
            if let Some(method) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestMethod)
            {
                scope.set_item("method", String::from_utf8_lossy(method.as_slice()))?;
            }
            if let Some(scheme) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestScheme)
            {
                scope.set_item("scheme", String::from_utf8_lossy(scheme.as_slice()))?;
            }
            if let Some(path) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestUrlPath)
            {
                let decoded_path = urlencoding::decode_binary(path.as_slice());
                scope.set_item("path", String::from_utf8_lossy(&decoded_path))?;
                scope.set_item("raw_path", path.as_slice())?;
            }
            if let Some(query) = envoy_filter
                .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestQuery)
            {
                scope.set_item("query", query.as_slice())?;
            } else {
                scope.set_item("query", b"")?;
            }
            scope.set_item("root_path", "")?;
            let headers = PyList::new(
                py,
                envoy_filter
                    .get_request_headers()
                    .iter()
                    .map(|(k, v)| (k.as_slice(), v.as_slice())),
            )?;
            scope.set_item("headers", headers)?;

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

            let (response_tx, response_rx) = channel::<ResponseEvent>();
            self.response_rx.replace(response_rx);

            let (request_future_tx, request_future_rx) = channel::<Py<PyAny>>();
            self.request_future_rx.replace(request_future_rx);

            let app = self.app.bind(py);
            let coro = app.call1((
                scope,
                ASGIReceiveCallable {
                    request_future_tx,
                    scheduler: envoy_filter.new_scheduler(),
                    loop_: self.loop_.clone_ref(py),
                },
                ASGISendCallable {
                    response_tx,
                    scheduler: envoy_filter.new_scheduler(),
                    loop_: self.loop_.clone_ref(py),
                },
            ))?;
            let asyncio = PyModule::import(py, "asyncio")?;
            asyncio.call_method1("run_coroutine_threadsafe", (coro, &self.loop_.bind(py)))?;
            Ok(())
        });
        res.unwrap()
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
        let set_result = future.getattr("set_result")?;
        loop_.bind(py).call_method1("call_soon_threadsafe", (set_result, PyNone::get(py)))?;
        Ok(())
    })
}

fn has_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> bool {
    if let Some(buffers) = envoy_filter.get_request_body() {
        for buffer in buffers {
            if (buffer.as_slice().len() > 0) {
                return true;
            }
        }
    }
    return false;
}

fn process_request_future<EHF: EnvoyHttpFilter>(
    envoy_filter: &mut EHF, loop_: &Py<PyAny>, future: Py<PyAny>, more_body: bool) -> PyResult<()> {
    println!("prf");
    let mut body = Vec::new();
    if let Some(buffers) = envoy_filter.get_request_body() {
        for buffer in buffers {
            body.extend_from_slice(buffer.as_slice());
        }
    }
    envoy_filter.drain_request_body(body.len());
    Python::attach(|py| {
        let future = future.bind(py);
        let set_result = future.getattr("set_result")?;
        let event = PyDict::new(py);
        event.set_item("type", "http.request")?;
        event.set_item("body", body)?;
        event.set_item("more_body", more_body)?;
        loop_.bind(py).call_method1("call_soon_threadsafe", (set_result, event))?;
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
            let future = self.loop_.bind(py).call_method0("create_future")?;
            self.request_future_tx.send(future.clone().unbind()).unwrap();
            self.scheduler.commit(EVENT_ID_REQUEST);
            Ok(future.unbind())
        })
    }
}

#[pyclass]
struct ASGISendCallable {
    response_tx: Sender<ResponseEvent>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    loop_: Py<PyAny>,
}

unsafe impl Sync for ASGISendCallable {}

#[pymethods]
impl ASGISendCallable {
    fn __call__(&self, event: Py<PyDict>) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let future = self.loop_.bind(py).call_method0("create_future")?;
            
            let event = event.bind(py);
            let event_type: String = match event.get_item("type")? {
                Some(v) => v.extract()?,
                None => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        "Missing 'type' in ASGI event",
                    ));
                }
            };

            match event_type.as_str() {
                "http.response.start" => {
                    let status: u16 = match event.get_item("status")? {
                        Some(v) => v.extract()?,
                        None => {
                            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                                "Missing 'status' in http.response.start event",
                            ));
                        }
                    };
                    let headers: Vec<(String, Vec<u8>)> = match event.get_item("headers")? {
                        Some(v) => v.extract()?,
                        None => Vec::new(),
                    };
                    self.response_tx
                        .send(ResponseEvent::Start(ResponseStartEvent {
                            status,
                            headers,
                        }))
                        .unwrap();
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                    future.call_method1("set_result", (PyNone::get(py),))?;
                }
                "http.response.body" => {
                    let more_body: bool = match event.get_item("more_body")? {
                        Some(v) => v.extract()?,
                        None => false,
                    };
                    let body: Vec<u8> = match event.get_item("body")? {
                        Some(body) => body.extract()?,
                        _ => Vec::new(),
                    };
                    self.response_tx.send(ResponseEvent::Body(ResponseBodyEvent { body, more_body, future: future.clone().unbind() })).unwrap();
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                }
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        "Unsupported ASGI event type",
                    ));
                }
            }
            Ok(future.unbind())
        })
    }
}

struct ResponseStartEvent {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
}

struct ResponseBodyEvent {
    body: Vec<u8>,
    more_body: bool,
    future: Py<PyAny>,
}

enum ResponseEvent {
    Start(ResponseStartEvent),
    Body(ResponseBodyEvent),
}
