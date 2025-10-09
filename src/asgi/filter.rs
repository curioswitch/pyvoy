use crossbeam_channel;
use crossbeam_channel::{Receiver, Sender};
use envoy_proxy_dynamic_modules_rust_sdk::{abi::envoy_dynamic_module_type_attribute_id, *};
use pyo3::{Py, PyAny};

use super::types::*;
use crate::asgi::python;
use crate::types::*;

pub struct Config {
    executor: python::Executor,
}

impl Config {
    pub fn new(filter_config: &str) -> Option<Self> {
        let (module, attr) = filter_config
            .split_once(":")
            .unwrap_or((filter_config, "app"));
        let executor = match python::Executor::new(module, attr) {
            Ok(executor) => executor,
            Err(err) => {
                eprintln!("Failed to initialize ASGI app: {err}");
                return None;
            }
        };
        Some(Self { executor })
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&mut self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        let (request_future_tx, request_future_rx) = crossbeam_channel::unbounded::<Py<PyAny>>();
        let (response_tx, response_rx) = crossbeam_channel::unbounded::<ResponseEvent>();
        Box::new(Filter {
            executor: self.executor.clone(),
            request_closed: false,
            response_state: ResponseState::Started,
            response_trailers: None,
            request_future_rx: request_future_rx,
            request_future_tx: Some(request_future_tx),
            response_rx: response_rx,
            response_tx: Some(response_tx),
        })
    }
}

enum ResponseState {
    Started,
    SentHeaders,
    Complete,
}

struct Filter {
    executor: python::Executor,

    request_closed: bool,
    response_state: ResponseState,

    response_trailers: Option<Vec<(String, Box<[u8]>)>>,

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
        let mut trailers_accepted = false;
        for (name, value) in envoy_filter.get_request_headers() {
            if name.as_slice() == b"te" && value.as_slice() == b"trailers" {
                // Allow te header to upstream
                trailers_accepted = true;
            }
        }
        println!("Trailers accepted: {}", trailers_accepted);
        let scope = self.new_scope(envoy_filter);
        self.executor.execute_app(
            scope,
            trailers_accepted,
            self.request_future_tx.take().unwrap(),
            self.response_tx.take().unwrap(),
            envoy_filter.new_scheduler(),
            envoy_filter.new_scheduler(),
            envoy_filter.new_scheduler(),
        );
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
                    self.executor.handle_request_future(
                        read_request_body(envoy_filter),
                        !end_of_stream,
                        future,
                    );
                }
                Err(_) => {}
            }
        }
        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_stream_complete(&mut self, _envoy_filter: &mut EHF) {}

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        if event_id == EVENT_ID_REQUEST {
            if self.request_closed || has_request_body(envoy_filter) {
                match self.request_future_rx.try_recv() {
                    Ok(future) => {
                        self.executor.handle_request_future(
                            read_request_body(envoy_filter),
                            !self.request_closed,
                            future,
                        );
                    }
                    Err(_) => {}
                }
            }
            return;
        }
        for event in self.response_rx.try_iter().collect::<Vec<_>>() {
            match event {
                ResponseEvent::Start(start_event, body_event) => {
                    if start_event.trailers {
                        self.response_trailers.replace(Vec::new());
                    }
                    let headers_ref: Vec<(&str, &[u8])> = start_event
                        .headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), &v[..]))
                        .collect();
                    let end_stream = !body_event.more_body && self.response_trailers.is_none();
                    self.executor.handle_response_future(body_event.future);
                    if end_stream {
                        if body_event.body.is_empty() {
                            envoy_filter.send_response_headers(headers_ref, true);
                        } else {
                            envoy_filter.send_response_headers(headers_ref, false);
                            envoy_filter.send_response_data(&body_event.body, true);
                        }
                    } else {
                        envoy_filter.send_response_headers(headers_ref, false);
                        envoy_filter.send_response_data(&body_event.body, false);
                    }
                    self.response_state = ResponseState::SentHeaders;
                }
                ResponseEvent::Body(event) => {
                    let end_stream = !event.more_body && self.response_trailers.is_none();
                    self.executor.handle_response_future(event.future);
                    envoy_filter.send_response_data(&event.body, end_stream);
                }
                ResponseEvent::Trailers(mut event) => {
                    if let Some(trailers) = &mut self.response_trailers {
                        trailers.append(&mut event.headers);
                        if !event.more_trailers {
                            let trailers_ref: Vec<(&str, &[u8])> =
                                trailers.iter().map(|(k, v)| (k.as_str(), &v[..])).collect();
                            envoy_filter.send_response_trailers(trailers_ref);
                        }
                    }
                }
                ResponseEvent::Exception => match self.response_state {
                    // While technically it's possible we have a chance to interrupt the response to
                    // trigger a client failure, not always. It is more consistent to always allow it
                    // to be completed successfully on the client side rather than having inconsistent
                    // behavior. The exception itself was logged from Python.
                    ResponseState::Complete => {}
                    _ => {
                        self.response_state = ResponseState::Complete;
                        envoy_filter.send_response(
                            500,
                            vec![
                                ("content-type", b"text/plain; charset=utf-8"),
                                ("connection", b"close"),
                            ],
                            Some(b"Internal Server Error"),
                        );
                    }
                },
            }
        }
    }
}

impl Filter {
    fn new_scope<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &EHF) -> Scope {
        let http_version = match envoy_filter
            .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestProtocol)
        {
            Some(v) => match v.as_slice() {
                b"HTTP/1.0" => HttpVersion::Http10,
                b"HTTP/1.1" => HttpVersion::Http11,
                b"HTTP/2" => HttpVersion::Http2,
                b"HTTP/3" => HttpVersion::Http3,
                _ => HttpVersion::Http11,
            },
            None => HttpVersion::Http11,
        };
        let method = match envoy_filter
            .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestMethod)
        {
            Some(v) => match v.as_slice() {
                b"GET" => HttpMethod::Get,
                b"HEAD" => HttpMethod::Head,
                b"POST" => HttpMethod::Post,
                b"PUT" => HttpMethod::Put,
                b"DELETE" => HttpMethod::Delete,
                b"CONNECT" => HttpMethod::Connect,
                b"OPTIONS" => HttpMethod::Options,
                b"TRACE" => HttpMethod::Trace,
                b"PATCH" => HttpMethod::Patch,
                other => HttpMethod::Custom(Box::from(other)),
            },
            None => HttpMethod::Get,
        };

        let scheme = match envoy_filter
            .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestScheme)
        {
            Some(v) => match v.as_slice() {
                b"http" => HttpScheme::Http,
                b"https" => HttpScheme::Https,
                _ => HttpScheme::Http,
            },
            None => HttpScheme::Http,
        };

        let raw_path = match envoy_filter
            .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestUrlPath)
        {
            Some(v) => Box::from(v.as_slice()),
            None => b"/".to_vec().into_boxed_slice(),
        };

        let query_string = match envoy_filter
            .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestQuery)
        {
            Some(v) => Box::from(v.as_slice()),
            None => b"".to_vec().into_boxed_slice(),
        };

        let headers = envoy_filter
            .get_request_headers()
            .iter()
            .map(|(k, v)| (Box::from(k.as_slice()), Box::from(v.as_slice())))
            .collect();

        let client = get_address(
            envoy_filter,
            envoy_dynamic_module_type_attribute_id::SourceAddress,
            envoy_dynamic_module_type_attribute_id::SourcePort,
        );
        let server = get_address(
            envoy_filter,
            envoy_dynamic_module_type_attribute_id::DestinationAddress,
            envoy_dynamic_module_type_attribute_id::DestinationPort,
        );

        return Scope {
            http_version,
            method,
            scheme,
            raw_path,
            query_string,
            headers,
            client,
            server,
        };
    }
}

fn get_address<EHF: EnvoyHttpFilter>(
    envoy_filter: &EHF,
    address_attr_id: envoy_dynamic_module_type_attribute_id,
    port_attr_id: envoy_dynamic_module_type_attribute_id,
) -> Option<(String, i64)> {
    match (
        envoy_filter.get_attribute_string(address_attr_id),
        envoy_filter.get_attribute_int(port_attr_id),
    ) {
        (Some(host), Some(port)) => {
            let mut host = host.as_slice();
            if let Some(colon_idx) = host.iter().position(|&c| c == b':') {
                host = &host[..colon_idx];
            }
            Some((String::from_utf8_lossy(host).to_string(), port))
        }
        _ => None,
    }
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

fn read_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> Box<[u8]> {
    if let Some(buffers) = envoy_filter.get_request_body() {
        match buffers.len() {
            0 => Box::new([]),
            1 => {
                let body = buffers[0].as_slice().to_vec().into_boxed_slice();
                envoy_filter.drain_request_body(body.len());
                body
            }
            _ => {
                let mut body_len = 0;
                for buffer in &buffers {
                    body_len += buffer.as_slice().len();
                }
                let mut body = Vec::with_capacity(body_len);
                for buffer in buffers {
                    body.extend_from_slice(buffer.as_slice());
                }
                envoy_filter.drain_request_body(body.len());
                body.into_boxed_slice()
            }
        }
    } else {
        Box::new([])
    }
}
