use crate::wsgi::python::PyExecutor;
use envoy_proxy_dynamic_modules_rust_sdk::*;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};

use super::types::*;
use crate::types::*;
pub struct Config {
    executor: PyExecutor,
}

impl Config {
    pub fn new(app: &str) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let executor = match PyExecutor::new(module, attr, 200) {
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
        let (request_read_tx, request_read_rx) = mpsc::channel::<isize>();
        let (request_body_tx, request_body_rx) = mpsc::channel::<RequestBody>();
        let (response_tx, response_rx) = mpsc::channel::<ResponseEvent>();
        let (response_written_tx, response_written_rx) = mpsc::channel::<()>();
        Box::new(Filter {
            executor: self.executor.clone(),
            request_closed: false,
            response_closed: false,
            request_read_rx,
            request_read_tx: Some(request_read_tx),
            request_body_rx: Some(request_body_rx),
            request_body_tx,
            pending_read: 0,
            read_buffer: Vec::new(),
            response_rx,
            response_tx: Some(response_tx),
            response_written_tx,
            response_written_rx: Some(response_written_rx),
        })
    }
}

struct Filter {
    executor: PyExecutor,

    request_closed: bool,
    response_closed: bool,

    request_read_rx: Receiver<isize>,
    request_read_tx: Option<Sender<isize>>,
    request_body_rx: Option<Receiver<RequestBody>>,
    request_body_tx: Sender<RequestBody>,
    pending_read: isize,
    read_buffer: Vec<u8>,

    response_rx: Receiver<ResponseEvent>,
    response_tx: Option<Sender<ResponseEvent>>,
    response_written_tx: Sender<()>,
    response_written_rx: Option<Receiver<()>>,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        let scope = new_scope(envoy_filter);
        self.executor.execute_app(
            scope,
            self.request_read_tx.take().unwrap(),
            self.request_body_rx.take().unwrap(),
            self.response_tx.take().unwrap(),
            self.response_written_rx.take().unwrap(),
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
        self.process_read(envoy_filter);
        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        if event_id == EVENT_ID_REQUEST {
            match self.request_read_rx.recv() {
                Ok(n) => {
                    self.pending_read = n;
                }
                Err(_) => {
                    // TODO: Handle
                }
            }
            self.process_read(envoy_filter);
        }
        for event in self.response_rx.try_iter().collect::<Vec<_>>() {
            match event {
                ResponseEvent::Start(start_event, body_event) => {
                    let headers_ref: Vec<(&str, &[u8])> = start_event
                        .headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), &v[..]))
                        .collect();
                    let end_stream = !body_event.more_body;
                    if !end_stream {
                        send_or_log(&self.response_written_tx, ());
                    }
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
                }
                ResponseEvent::Body(event) => {
                    let end_stream = !event.more_body;
                    if !end_stream {
                        send_or_log(&self.response_written_tx, ());
                    }
                    envoy_filter.send_response_data(&event.body, end_stream);
                }
                ResponseEvent::Exception => {
                    if !self.response_closed {
                        self.response_closed = true;
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
            }
        }
    }
}

impl Filter {
    fn process_read<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &mut EHF) {
        if !has_request_body(envoy_filter) && !self.request_closed {
            return;
        }
        match self.pending_read {
            0 => (),
            n if n > 0 => {
                let mut remaining = n as usize;
                let mut body: Vec<u8> = Vec::with_capacity(remaining);
                if let Some(buffers) = envoy_filter.get_request_body() {
                    for buffer in buffers {
                        let to_copy = std::cmp::min(remaining, buffer.as_slice().len());
                        body.extend_from_slice(&buffer.as_slice()[..to_copy]);
                        remaining -= to_copy;
                        if remaining == 0 {
                            break;
                        }
                    }
                    envoy_filter.drain_request_body(body.len());
                }
                self.pending_read = 0;
                send_or_log(
                    &self.request_body_tx,
                    RequestBody {
                        body: body.into_boxed_slice(),
                        closed: !has_request_body(envoy_filter) && self.request_closed,
                    },
                );
            }
            _ => {
                if let Some(buffers) = envoy_filter.get_request_body() {
                    let mut read = 0;
                    for buffer in buffers {
                        self.read_buffer.extend_from_slice(buffer.as_slice());
                        read += buffer.as_slice().len();
                    }
                    envoy_filter.drain_request_body(read);
                }
                if self.request_closed {
                    self.pending_read = 0;
                    send_or_log(
                        &self.request_body_tx,
                        RequestBody {
                            body: std::mem::take(&mut self.read_buffer).into_boxed_slice(),
                            closed: true,
                        },
                    );
                }
            }
        }
    }
}

fn send_or_log<T>(tx: &Sender<T>, value: T) {
    if let Err(err) = tx.send(value) {
        eprintln!(
            "Failed to send event to Python, this is likely a bug in pyvoy: {}",
            err,
        );
    }
}
