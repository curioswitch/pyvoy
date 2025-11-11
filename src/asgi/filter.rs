use envoy_proxy_dynamic_modules_rust_sdk::*;
use http::{HeaderName, HeaderValue};
use pyo3::Python;
use pyo3::types::PyTracebackMethods as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::asgi::python;
use crate::asgi::python::*;
use crate::envoy::*;
use crate::eventbridge::EventBridge;
use crate::types::*;

pub struct Config {
    executor: python::Executor,
    handles: Option<python::ExecutorHandles>,
}

impl Config {
    pub fn new(app: &str, constants: Arc<Constants>) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let (executor, handles) = match python::Executor::new(module, attr, constants) {
            Ok(executor) => executor,
            Err(e) => {
                Python::attach(|py| {
                    let tb = e
                        .traceback(py)
                        .and_then(|tb| tb.format().ok())
                        .unwrap_or_default();
                    envoy_log_error!("Failed to initialize ASGI app\n{}{}", tb, e);
                });
                return None;
            }
        };
        Some(Self {
            executor,
            handles: Some(handles),
        })
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        self.executor.shutdown();
        self.handles.take().unwrap().join();
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&mut self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        Box::new(Filter {
            executor: self.executor.clone(),
            request_closed: false,
            response_closed: Arc::new(AtomicBool::new(false)),
            response_trailers: None,
            recv_bridge: EventBridge::new(),
            send_bridge: EventBridge::new(),
        })
    }
}

struct Filter {
    executor: python::Executor,

    request_closed: bool,
    response_closed: Arc<AtomicBool>,

    response_trailers: Option<Vec<(HeaderName, HeaderValue)>>,

    recv_bridge: EventBridge<RecvFuture>,
    send_bridge: EventBridge<SendEvent>,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        if end_of_stream {
            self.request_closed = true;
        }

        let trailers_accepted = envoy_filter
            .get_request_header_value("te")
            .map(|v| v.as_slice() == b"trailers")
            .unwrap_or(false);
        let scope = new_scope(envoy_filter);

        self.executor.execute_app(
            scope,
            end_of_stream,
            trailers_accepted,
            self.response_closed.clone(),
            self.recv_bridge.clone(),
            self.send_bridge.clone(),
            SyncScheduler::new(envoy_filter.new_scheduler()),
        );
        abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::Continue
    }

    fn on_request_body(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        if end_of_stream {
            self.request_closed = true;
        }

        self.recv_bridge.process(|future| {
            self.executor.handle_recv_future(
                read_request_body(envoy_filter),
                !self.request_closed,
                future,
            );
        });

        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_stream_complete(&mut self, _envoy_filter: &mut EHF) {
        self.response_closed.store(true, Ordering::Relaxed);
        self.recv_bridge.close();
        self.send_bridge.close();
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        if event_id == EVENT_ID_REQUEST {
            if self.request_closed || has_request_body(envoy_filter) {
                self.recv_bridge.process(|future| {
                    self.executor.handle_recv_future(
                        read_request_body(envoy_filter),
                        !self.request_closed,
                        future,
                    );
                });
            }
            return;
        }
        self.send_bridge.process(|event| {
            match event {
                SendEvent::Start(start_event, mut body_event) => {
                    if start_event.trailers {
                        self.response_trailers.replace(Vec::new());
                    }
                    let mut status_buf = itoa::Buffer::new();
                    let mut headers: Vec<(&str, &[u8])> =
                        Vec::with_capacity(start_event.headers.len() + 1);
                    headers.push((":status", status_buf.format(start_event.status).as_bytes()));
                    for (k, v) in start_event.headers.iter() {
                        headers.push((k.as_str(), v.as_bytes()));
                    }
                    let end_stream =
                        body_event.future.is_none() && self.response_trailers.is_none();
                    if let Some(future) = body_event.future.take() {
                        self.executor.handle_send_future(future);
                    }
                    if end_stream {
                        if body_event.body.is_empty() {
                            envoy_filter.send_response_headers(headers, true);
                        } else {
                            envoy_filter.send_response_headers(headers, false);
                            envoy_filter.send_response_data(&body_event.body, true);
                        }
                    } else {
                        envoy_filter.send_response_headers(headers, false);
                        envoy_filter.send_response_data(&body_event.body, false);
                    }
                }
                SendEvent::Body(mut event) => {
                    let end_stream = event.future.is_none() && self.response_trailers.is_none();
                    if let Some(future) = event.future.take() {
                        self.executor.handle_send_future(future);
                    }
                    envoy_filter.send_response_data(&event.body, end_stream);
                    if end_stream {}
                }
                SendEvent::Trailers(mut event) => {
                    if let Some(trailers) = &mut self.response_trailers {
                        trailers.append(&mut event.headers);
                        if !event.more_trailers {
                            let trailers_ref: Vec<(&str, &[u8])> = trailers
                                .iter()
                                .map(|(k, v)| (k.as_str(), v.as_bytes()))
                                .collect();
                            envoy_filter.send_response_trailers(trailers_ref);
                        }
                    }
                }
                SendEvent::Exception => {
                    // Will reset the stream if headers have already been sent.
                    envoy_filter.send_response(
                        500,
                        vec![
                            ("content-type", b"text/plain; charset=utf-8"),
                            ("connection", b"close"),
                        ],
                        Some(b"Internal Server Error"),
                    );
                }
            }
        });
    }
}
