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
            response_rx: None,
        })
    }
}

const EVENT_ID_RESPONSE: u64 = 2;

struct Filter {
    loop_: Py<PyAny>,
    app: Py<PyAny>,
    asgi: Py<PyDict>,
    response_rx: Option<Receiver<ResponseEvent>>,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &mut self,
        envoy_filter: &mut EHF,
        _end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        self.execute_app(envoy_filter);
        abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::Continue
    }

    fn on_request_body(
        &mut self,
        envoy_filter: &mut EHF,
        _end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        println!("RAG");
        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::Continue
    }

    fn on_response_headers(
        &mut self,
        envoy_filter: &mut EHF,
        _end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_response_headers_status {
        println!("BAG");
        envoy_filter.remove_response_header("content-length");
        abi::envoy_dynamic_module_type_on_http_filter_response_headers_status::Continue
    }

    fn on_response_body(
        &mut self,
        envoy_filter: &mut EHF,
        _end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_response_body_status {
        envoy_filter.drain_response_body(1);
        abi::envoy_dynamic_module_type_on_http_filter_response_body_status::StopIterationNoBuffer
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, _event_id: u64) {
        let response_rx = self.response_rx.as_ref().unwrap();
        match response_rx.recv().unwrap() {
            ResponseEvent::Start(status) => {
                println!("CAT");
                envoy_filter.set_response_header(":status", status.to_string().as_bytes());
                envoy_filter.continue_encoding();
                println!("DOG");
            }
            ResponseEvent::Body(body) => {
                envoy_filter.inject_response_body(&body, false);
            }
            ResponseEvent::Finish(body) => {
                envoy_filter.inject_response_body(&body, true);
            }
            ResponseEvent::FinishNoBody() => {
                envoy_filter.inject_response_body(&[], true);
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

            let scheduler = envoy_filter.new_scheduler();
            let (response_tx, response_rx) = channel::<ResponseEvent>();
            self.response_rx.replace(response_rx);

            let app = self.app.bind(py);
            let coro = app.call1((
                scope,
                ASGISendCallable {
                    response_tx,
                    scheduler,
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

#[pyclass]
struct ASGISendCallable {
    response_tx: Sender<ResponseEvent>,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
}

unsafe impl Sync for ASGISendCallable {}

#[pymethods]
impl ASGISendCallable {
    async fn __call__(&self, event: Py<PyDict>) -> PyResult<()> {
        Python::attach(|py| {
            let event = event.bind(py);
            let event_type: String = match event.get_item("type")? {
                Some(v) => v.extract()?,
                None => return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Missing 'type' in ASGI event",
                )),
            };

            match event_type.as_str() {
                "http.response.start" => {
                    let status: u16 = match event.get_item("status")? {
                        Some(v) => v.extract()?,
                        None => {
                            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                                "Missing 'status' in http.response.start event",
                            ))
                        }
                    };
                    self.response_tx.send(ResponseEvent::Start(status)).unwrap();
                    self.scheduler.commit(EVENT_ID_RESPONSE);
                }
                "http.response.body" => {
                    let more_body: bool = match event.get_item("more_body")? {
                        Some(v) => v.extract()?,
                        None => false,
                    };
                    if let Some(body) = event.get_item("body")? {
                        let body: Vec<u8> = body.extract()?;
                        self.response_tx
                            .send(match more_body {
                                true => ResponseEvent::Body(body),
                                false => ResponseEvent::Finish(body),
                            })
                            .unwrap();
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    } else if !more_body {
                        self.response_tx
                            .send(ResponseEvent::FinishNoBody())
                            .unwrap();
                        self.scheduler.commit(EVENT_ID_RESPONSE);
                    }
                }
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        "Unsupported ASGI event type",
                    ))
                }
            }
            Ok(())
        })
    }
}

enum ResponseEvent {
    Start(u16),
    Body(Vec<u8>),
    Finish(Vec<u8>),
    FinishNoBody(),
}
